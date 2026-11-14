//! Game server logic (spec §6.1).
//!
//! [`GameServer`] is transport-independent: it consumes and produces encoded
//! protocol messages. The same type runs
//! - inside the desktop app as the **local server** ([`local::LocalServer`]), and
//! - as the headless **remote server** binary (`ocw-server`), reachable over
//!   QUIC with [`net`] (`--listen`, spec §6.2), and from the browser
//!   over WebTransport (`--webtransport`, spec §14 Phase 10 B3).

pub mod local;
// QUIC (spec §6.2) and world packages from disk: native only; the browser
// app (spec §14 Phase 10) runs the server in its frame loop.
#[cfg(not(target_arch = "wasm32"))]
pub mod net;
pub mod stats;
// Snapshot timing at a client (interval, jitter, age; spec §14 Phase 10 B3).
pub mod timing;
pub mod voice;
// The wire rules without I/O (frames, datagram rules, the client's link
// state machine): shared with the browser app's WebTransport client.
pub mod wire;
#[cfg(not(target_arch = "wasm32"))]
pub mod worlds;

use std::collections::BTreeMap;
use std::sync::Arc;

use ocw_core::{BROADCAST_HZ, RiderId, SIM_TICK_HZ, TICKS_PER_BROADCAST};
use ocw_protocol::v1::{self, client_message::Msg as C, server_message::Msg as S};
use ocw_protocol::{PROTOCOL_VERSION, decode_client, encode, server_message};
use ocw_sim::{EventKind, EventPhase, EventSpec, FreeRideStart, RiderInput, Session};
use ocw_world::{RoadNetwork, Route, RoutePlan};

/// What a session rides: a fixed route, free ride on the road graph, or a
/// route on the road graph (followed with the free-ride machinery, spec
/// §2.7).
#[derive(Debug, Clone)]
pub enum Course {
    Route(Route),
    FreeRide {
        net: Arc<RoadNetwork>,
        start: FreeRideStart,
    },
    RouteRide {
        net: Arc<RoadNetwork>,
        plan: Arc<RoutePlan>,
    },
}

pub use ocw_protocol::world_id;

impl From<Route> for Course {
    fn from(r: Route) -> Self {
        Self::Route(r)
    }
}

/// Identifies one client connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConnectionId(pub u64);

#[derive(Debug, Clone, Default)]
pub struct ServerConfig {
    /// Accept [`v1::DevCommand`]s. Only true for the in-process local server (spec §2.9).
    pub allow_dev_commands: bool,
    /// Number of server-side test riders.
    pub bots: u32,
    /// Where on the route everyone starts, in metres.
    pub start_m: f64,
    /// Riders may start and cancel events ([`v1::EventRequest`]): only the
    /// in-process local server, where the single player is the organizer
    /// (spec §2.3); on a remote server events are scheduled.
    pub allow_local_events: bool,
    /// LiveKit server and API key pair for voice chat (spec §7); `None`:
    /// no voice on this server (token requests are refused).
    pub voice: Option<voice::VoiceConfig>,
    /// Who may change the world clock (spec §14 Phase 9, 9D): the local
    /// servers let their single player (anyone), a shared session only
    /// its host (the default).
    pub clock_access: ClockAccess,
    /// Who may switch the session's world package (spec §3.2.1, World →
    /// Change world; [`v1::WorldRequest`]): the same rule as the clock –
    /// the local servers' single player, the host of a shared session (the
    /// default), nobody when the organizer fixed the world.
    pub world_access: ClockAccess,
    /// Who may enter god mode (spec §14 Phase 13A, [`v1::GodRequest`]):
    /// anyone on the local servers (the single player), the host of a
    /// shared session (the default), nobody in an organizer's session.
    pub god_access: ClockAccess,
    /// The real UTC instant (Unix ms) at the session's start; `None`: the
    /// system clock when the server is created (tests fix it).
    pub real_epoch_ms: Option<i64>,
    /// Riders may put the Echo rider on the road ([`v1::EchoRequest`], the
    /// echo test of spec §7): the local servers, `ocw-server --echo-bot`.
    pub allow_echo_bot: bool,
    /// Every session starts with the Echo rider at the start
    /// (`ocw-server --echo-bot`).
    pub echo_bot_at_start: bool,
    /// `ride.collisions` of the session (spec §14 Phase 12A): the local
    /// servers follow the rider's setting (on by default, Q1), `ocw-server
    /// --collisions on|off`. `Default` keeps it off (tests, embeddings that
    /// decide themselves).
    pub collisions: bool,
    /// `ride.chaotic_bots` (Phase 12 Q3, off by default).
    pub chaotic_bots: bool,
    /// Who may change them ([`v1::RulesRequest`]): the single player on
    /// the local servers, the host of a shared session (the default),
    /// nobody when the organizer fixed them.
    pub collision_access: ClockAccess,
    /// Who may change the session's weather (spec §14 Phase 9, S4 / S5;
    /// [`v1::WeatherRequest`], Q2): the single player on the local servers,
    /// the host of a shared session (the default), nobody when the
    /// organizer fixed it.
    pub weather_access: ClockAccess,
    /// The weather the session starts with (`None`: clear; real weather
    /// is Phase 9 S7); `ocw-server --weather PRESET`.
    pub weather: Option<ocw_weather::WeatherState>,
    /// Seed of the gusts, the lightning and the weather simulation
    /// (`None`: from the session's start instant; tests fix it).
    pub weather_seed: Option<u64>,
}

pub use ocw_protocol::ClockAccess;

/// The system's real UTC instant (Unix ms).
pub fn unix_now_ms() -> i64 {
    // std's clock panics on wasm32-unknown-unknown; web-time asks the
    // browser (`Date.now()`).
    #[cfg(not(target_arch = "wasm32"))]
    use std::time::{SystemTime, UNIX_EPOCH};
    #[cfg(target_arch = "wasm32")]
    use web_time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// Ticks between two event-state broadcasts while a race or time trial
/// runs (2 Hz: the standings and gaps).
pub const EVENT_STATE_TICKS: u64 = 25;

/// Authoritative server for one session.
pub struct GameServer {
    config: ServerConfig,
    session: Session,
    /// Connection → rider, once the client said hello.
    riders: BTreeMap<ConnectionId, RiderId>,
    next_connection: u64,
    /// The event changed outside a tick (cancelled): send its state.
    event_dirty: bool,
    /// Display names from the hellos (voice tokens carry them).
    names: BTreeMap<ConnectionId, String>,
    /// When each connection got its last voice token (unix s).
    voice_issued: BTreeMap<ConnectionId, u64>,
    /// Display names of the clients that joined and left, in order (for
    /// the server's statistics, `ocw-server --stats-file`).
    joined: Vec<String>,
    left: Vec<String>,
    /// The session's world clock (spec §14 Phase 9), on session time.
    clock: ocw_sky::WorldClock,
    /// The session's weather (spec §14 Phase 9, S4), on session time.
    weather: ocw_weather::SessionWeather,
    /// Session time (s) before the current session: a world switch starts
    /// a new [`Session`] at tick 0, the clock goes on.
    time_base_s: f64,
    /// An allowed world switch, waiting for the embedding to load it.
    pending_world: Option<WorldSwitch>,
}

/// A world switch the host asked for (spec §3.2.1), checked and waiting
/// for the embedding (`ocw-server`) to load the package
/// ([`GameServer::take_world_switch`], then [`GameServer::switch_world`] or
/// [`result_message`] to the asker on failure).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldSwitch {
    pub conn: ConnectionId,
    pub rider: RiderId,
    /// The asker's display name.
    pub by: String,
    /// What the request names: an id, `id@version`, a name or a file name
    /// of the server's catalog.
    pub world: String,
    /// A route of the package (`None`: a free ride).
    pub route: Option<String>,
}

/// An encoded `DevCommandResult` (the answer to a request).
pub fn result_message(ok: bool, message: impl Into<String>) -> Vec<u8> {
    encode(&server_message(S::DevResult(v1::DevCommandResult {
        ok,
        message: message.into(),
    })))
}

/// Base power of the bots, from easy riders to strong ones, so they spread
/// out on the road instead of riding as one bunch: twelve distinct
/// strengths from 85 to 280 W, easy and strong ones interleaved so even a
/// small field mixes them.
pub const BOT_POWERS_W: [f64; 12] = [
    210.0, 110.0, 160.0, 85.0, 250.0, 135.0, 185.0, 95.0, 280.0, 120.0, 230.0, 145.0,
];

/// Most bots in an event's field.
pub const MAX_EVENT_BOTS: u32 = 50;

/// "2 laps (8.0 km)" or "5.0 km".
pub fn distance_label(length_m: f64, laps: Option<u32>) -> String {
    match laps {
        Some(n) => format!(
            "{n} lap{} ({:.1} km)",
            if n == 1 { "" } else { "s" },
            length_m / 1000.0
        ),
        None => format!("{:.1} km", length_m / 1000.0),
    }
}

/// Base power of the `i`-th bot.
pub fn bot_power_w(i: u32) -> f64 {
    BOT_POWERS_W[i as usize % BOT_POWERS_W.len()]
}

/// Bike and look of the `i`-th bot: the bike types in turn (road, TT,
/// gravel, MTB, then the other road profiles), colours from the palette,
/// so a small field shows every bike model (spec §2.5).
pub fn bot_bike(i: u32) -> (ocw_core::ProfileId, ocw_core::RiderLook) {
    use ocw_core::ProfileId::*;
    use ocw_core::bike::PALETTE;
    const BIKES: [ocw_core::ProfileId; 6] =
        [Road, TimeTrial, Gravel, Mountain, RoadAero, RoadClimbing];
    let i = i as usize;
    let n = PALETTE.len();
    let look = ocw_core::RiderLook {
        // Not the local rider's #FF1472 (palette 0) for the jersey.
        jersey: PALETTE[1 + (i * 5) % (n - 1)],
        frame: PALETTE[(i * 7 + 3) % n],
        helmet: PALETTE[(i * 3 + 9) % n],
        pattern: ocw_core::KitPattern::ALL[i % ocw_core::KitPattern::ALL.len()],
        ..Default::default()
    };
    (BIKES[i % BIKES.len()], look)
}

/// Adds the `i`-th server bot ("Bot i+1") with its power, bike and look
/// from [`bot_power_w`] and [`bot_bike`]: the same field for the server's
/// startup bots and an event's bots.
fn add_numbered_bot(session: &mut Session, i: u32) -> RiderId {
    let (bike, look) = bot_bike(i);
    session.add_bot_with(
        format!("Bot {}", i + 1),
        bot_power_w(i),
        ocw_core::RiderProfile::curated(bike, 75.0),
        look,
    )
}

impl GameServer {
    /// A session on `course` (a [`Route`] converts into one).
    pub fn new(course: impl Into<Course>, config: ServerConfig) -> Self {
        let session = new_session(course.into(), &config);
        let epoch = config.real_epoch_ms.unwrap_or_else(unix_now_ms);
        let clock = ocw_sky::WorldClock::new(epoch);
        let seed = config
            .weather_seed
            .unwrap_or_else(|| ocw_weather::lightning::unit(epoch as u64, 1, 2).to_bits());
        let weather = ocw_weather::SessionWeather::with(config.weather.unwrap_or_default(), seed);
        Self {
            clock,
            weather,
            config,
            session,
            riders: BTreeMap::new(),
            next_connection: 1,
            event_dirty: false,
            names: BTreeMap::new(),
            voice_issued: BTreeMap::new(),
            joined: Vec::new(),
            left: Vec::new(),
            time_base_s: 0.0,
            pending_world: None,
        }
    }

    /// The world switch waiting to be loaded, if any (taken once).
    pub fn take_world_switch(&mut self) -> Option<WorldSwitch> {
        self.pending_world.take()
    }

    /// Whether a world switch was allowed and not yet taken.
    pub fn world_switch_pending(&self) -> bool {
        self.pending_world.is_some()
    }

    /// Ends the session and starts a new one on `course` (another world,
    /// spec §3.2.1): the bots ride again, the world clock goes on, every
    /// rider leaves the session (connections stay) and joins again with a
    /// new Hello. Returns the connections that rode and the encoded
    /// [`v1::WorldChange`] to send them.
    pub fn switch_world(
        &mut self,
        course: impl Into<Course>,
        change: v1::WorldChange,
    ) -> (Vec<ConnectionId>, Vec<u8>) {
        self.time_base_s = self.session_time_s();
        self.session = new_session(course.into(), &self.config);
        let conns: Vec<ConnectionId> = std::mem::take(&mut self.riders).into_keys().collect();
        self.event_dirty = false;
        self.pending_world = None;
        tracing::info!(world = %change.package_id, route = %change.route, riders = conns.len(), "world switched");
        (conns, encode(&server_message(S::WorldChange(change))))
    }

    /// A world switch request from `rider` (spec §3.2.1): checked here, on
    /// the server (§6.1), as the clock's; the package is loaded by the
    /// embedding.
    fn world_request(&mut self, conn: ConnectionId, rider: RiderId, req: v1::WorldRequest) -> S {
        let result = |ok: bool, message: String| S::DevResult(v1::DevCommandResult { ok, message });
        if !self.config.world_access.allows(Some(rider), self.host()) {
            return result(
                false,
                match self.config.world_access {
                    ClockAccess::Nobody => {
                        "the world of this session is set by the organizer".into()
                    }
                    _ => "only the host of this shared ride can change the world".into(),
                },
            );
        }
        let world = req.world.trim().to_owned();
        if world.is_empty() {
            return result(
                false,
                "which world? (an id or name the server lists)".into(),
            );
        }
        if self.pending_world.is_some() {
            return result(false, "another world is being loaded".into());
        }
        let route = (!req.route.trim().is_empty()).then(|| req.route.trim().to_owned());
        tracing::info!(%rider, %world, ?route, "world switch asked");
        self.pending_world = Some(WorldSwitch {
            conn,
            rider,
            by: self.names.get(&conn).cloned().unwrap_or_default(),
            world: world.clone(),
            route,
        });
        result(true, format!("the server loads {world} ..."))
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    /// Session time (s): the simulation ticks so far.
    pub fn session_time_s(&self) -> f64 {
        self.time_base_s + self.session.tick() as f64 / f64::from(SIM_TICK_HZ)
    }

    /// The world clock (spec §14 Phase 9).
    pub fn clock(&self) -> &ocw_sky::WorldClock {
        &self.clock
    }

    /// The session's weather (spec §14 Phase 9, S4).
    pub fn weather(&self) -> &ocw_weather::SessionWeather {
        &self.weather
    }

    fn weather_message(&self) -> v1::SessionWeather {
        ocw_protocol::session_weather(
            &self.weather,
            self.session_time_s(),
            self.config.weather_access,
        )
    }

    /// A weather request from `rider` (spec §14 Phase 9, S4; the weather
    /// machine of S5 sends the same): checked here, on the server (§6.1,
    /// Q2: only the host of a shared session), like the clock's.
    fn weather_request(&mut self, rider: RiderId, req: &v1::WeatherRequest) -> S {
        let result = |ok: bool, message: String| S::DevResult(v1::DevCommandResult { ok, message });
        if !self.config.weather_access.allows(Some(rider), self.host()) {
            return result(
                false,
                match self.config.weather_access {
                    ClockAccess::Nobody => {
                        "the weather of this session is set by the organizer".into()
                    }
                    _ => "only the host of this shared ride can change the weather".into(),
                },
            );
        }
        let c = ocw_protocol::weather_change(req);
        let t = self.session_time_s();
        let mut said = Vec::new();
        if let Some(seed) = c.simulate {
            let seed = if seed == 0 {
                self.weather.seed().rotate_left(17) ^ t.to_bits()
            } else {
                seed
            };
            self.weather.simulate(t, seed);
            said.push(format!("weather simulation (seed {seed})"));
        }
        if let Some(target) = c.target {
            self.weather.set(t, target, c.transition_s, c.settle_ground);
            said.push(if c.transition_s > 0.0 {
                format!(
                    "weather -> {} in {:.0} s",
                    target.condition().label(),
                    c.transition_s
                )
            } else {
                format!("weather: {}", self.weather.now(t).summary())
            });
        }
        if !c.ground.is_empty() {
            let g = c.ground.apply(self.weather.ground());
            self.weather.set_ground(g);
            said.push(format!(
                "ground: wet {:.0} %, puddles {:.0} %, snow {:.0} %",
                g.wetness() * 100.0,
                g.puddles() * 100.0,
                g.snow_cover() * 100.0
            ));
        }
        if let Some((d, b)) = c.strike {
            let s = self.weather.strike(t, d, b);
            said.push(format!(
                "lightning {:.0} m away, thunder in {:.1} s",
                s.distance_m,
                s.thunder_delay_s()
            ));
        }
        if said.is_empty() {
            return result(false, "empty weather request".into());
        }
        let said = said.join("; ");
        tracing::info!(%rider, weather = %said, "weather changed");
        result(true, said)
    }

    /// The host of a shared session: the rider connected longest (lowest
    /// connection id); `None` while nobody is connected.
    pub fn host(&self) -> Option<RiderId> {
        self.riders.values().next().copied()
    }

    fn clock_message(&self) -> v1::WorldClock {
        ocw_protocol::world_clock(
            &self.clock,
            self.session_time_s(),
            self.config.clock_access,
            self.host(),
        )
    }

    /// A time-machine request from `rider` (spec §14 Phase 9, 9D): checked
    /// here, on the server (§6.1), not only in the client's menu.
    fn clock_request(&mut self, rider: RiderId, req: &v1::ClockRequest) -> S {
        let result = |ok: bool, message: String| S::DevResult(v1::DevCommandResult { ok, message });
        if !self.config.clock_access.allows(Some(rider), self.host()) {
            return result(
                false,
                match self.config.clock_access {
                    ClockAccess::Nobody => {
                        "the time of this session is set by the organizer".into()
                    }
                    _ => "only the host of this shared ride can change the time".into(),
                },
            );
        }
        let Some(change) = ocw_protocol::clock_change(req) else {
            return result(false, "empty time request".into());
        };
        let t = self.session_time_s();
        use ocw_sky::ClockChange as K;
        let done = match change {
            K::Real => {
                self.clock.set_real(t);
                Ok(())
            }
            K::Fixed(ms) => self.clock.set_fixed(t, ms),
            K::Offset(ms) => self.clock.set_offset(t, ms),
            K::Step(ms) => self.clock.step(t, ms),
            K::Speed(s) => self.clock.set_speed(t, s),
            K::Paused(p) => {
                self.clock.set_paused(t, p);
                Ok(())
            }
        };
        match done {
            Ok(()) => {
                // Everyone gets the new time with the next snapshot (at
                // most 100 ms).
                tracing::info!(%rider, clock = %self.clock.describe(t), "world clock changed");
                result(true, format!("world clock: {}", self.clock.describe(t)))
            }
            Err(e) => result(false, e),
        }
    }

    /// Display names of the clients that joined so far, in order.
    pub fn joined(&self) -> &[String] {
        &self.joined
    }

    /// Display names of the clients that left (disconnected) so far.
    pub fn left(&self) -> &[String] {
        &self.left
    }

    /// Registers a new connection.
    pub fn connect(&mut self) -> ConnectionId {
        let id = ConnectionId(self.next_connection);
        self.next_connection += 1;
        id
    }

    /// Removes a connection and its rider.
    pub fn disconnect(&mut self, conn: ConnectionId) {
        self.names.remove(&conn);
        self.voice_issued.remove(&conn);
        if let Some(rider) = self.riders.remove(&conn) {
            let name = self
                .session
                .snapshot()
                .into_iter()
                .find(|r| r.id == rider)
                .map(|r| r.name.clone())
                .unwrap_or_default();
            self.left.push(name);
            self.session.remove_rider(rider);
            tracing::info!(%rider, "rider left");
        }
    }

    /// Handles one encoded client message. Returns encoded replies for this connection.
    pub fn handle(&mut self, conn: ConnectionId, bytes: &[u8]) -> Vec<Vec<u8>> {
        let msg = match decode_client(bytes) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(?conn, %e, "dropping malformed message");
                return Vec::new();
            }
        };
        let had_rider = self.riders.contains_key(&conn);
        let reply = match (msg, self.riders.get(&conn).copied()) {
            (C::Hello(hello), None) => Some(self.hello(conn, hello)),
            (C::Hello(_), Some(_)) => None,
            (C::Sample(sample), Some(rider)) => {
                let input = RiderInput {
                    power_w: sample.power_w.map(f64::from).unwrap_or(0.0),
                    cadence_rpm: sample.cadence_rpm.map(f64::from),
                    heart_rate_bpm: sample.heart_rate_bpm.map(f64::from),
                    brake: sample.brake.unwrap_or(false),
                };
                self.session.apply_input(rider, sample.seq, input);
                // Virtual power earns no hearts (spec §14 Phase 4, 4E).
                self.session
                    .set_virtual_power(rider, sample.virtual_power.unwrap_or(false));
                None
            }
            (C::Dev(cmd), Some(rider)) => Some(self.dev_command(rider, cmd)),
            (C::Event(req), Some(_)) => Some(self.event_request(req)),
            (C::Clock(req), Some(rider)) => Some(self.clock_request(rider, &req)),
            (C::Weather(req), Some(rider)) => Some(self.weather_request(rider, &req)),
            (C::World(req), Some(rider)) => Some(self.world_request(conn, rider, req)),
            (C::God(req), Some(rider)) => Some(S::God(self.god_request(rider, req))),
            (C::GodPose(pose), Some(rider)) => {
                // Implausible poses are dropped: the others keep seeing the
                // last good one (spec §14 Phase 13E).
                let teleport = pose.teleport && self.config.allow_dev_commands;
                let p = ocw_protocol::god_pose_from_wire(&pose);
                if let Err(e) = self.session.set_god_pose(rider, p, teleport)
                    && self.session.is_god(rider)
                {
                    tracing::debug!(%rider, error = e.describe(), "god pose refused");
                }
                None
            }
            (C::Voice(req), Some(rider)) => Some(S::Voice(self.voice_grant(conn, rider, &req))),
            (C::Echo(req), Some(rider)) => Some(self.echo_request(rider, req)),
            (C::Rules(req), Some(rider)) => Some(self.rules_request(rider, req)),
            (C::BikeChange(change), Some(rider)) => Some(self.bike_change(rider, &change)),
            (C::Turn(turn), Some(rider)) => {
                if let Some(net) = self.session.network() {
                    let choice = ocw_protocol::turn_choice(&turn, net);
                    self.session.set_turn(rider, turn.seq, choice);
                }
                None
            }
            (C::UTurn(u), Some(rider)) => {
                let from = (
                    f64::from(u.distance_cm) / 100.0,
                    f64::from(u.speed_mm_s) / 1000.0,
                );
                self.session.u_turn(rider, Some(from));
                None
            }
            (C::RouteRejoin(u), Some(rider)) => {
                let from = (
                    f64::from(u.distance_cm) / 100.0,
                    f64::from(u.speed_mm_s) / 1000.0,
                );
                if let Err(e) = self.session.rejoin_route(rider, Some(from)) {
                    tracing::debug!(%rider, %e, "rejoin refused");
                }
                None
            }
            (C::Steer(steer), Some(rider)) => {
                self.session.set_wild(rider, steer.wild);
                if steer.lean_steering {
                    self.session
                        .set_lean(rider, steer.seq, f64::from(steer.lean_mrad) / 1000.0);
                } else {
                    self.session.set_lateral_target(
                        rider,
                        steer.seq,
                        f64::from(steer.lateral_target_cm) / 100.0,
                    );
                }
                None
            }
            (_, None) => Some(S::Reject(v1::Reject {
                reason: "send Hello first".into(),
            })),
        };
        let mut out: Vec<Vec<u8>> = reply
            .map(|m| vec![encode(&server_message(m))])
            .unwrap_or_default();
        // A client that just joined a free ride gets everyone's path.
        let joined = !had_rider && self.riders.contains_key(&conn);
        if joined && self.session.event().is_some() {
            out.push(encode(&server_message(S::Event(self.event_state()))));
        }
        if joined {
            let builds: Vec<RiderId> = self
                .session
                .snapshot()
                .iter()
                .filter(|r| r.build_hash != 0 || r.look.character != Default::default())
                .map(|r| r.id)
                .collect();
            out.extend(builds.into_iter().map(|id| self.build_message(id)));
        }
        if joined && let Some(net) = self.session.network() {
            out.extend(
                self.session
                    .all_paths()
                    .iter()
                    .map(|c| encode(&server_message(S::Path(ocw_protocol::path_update(c, net))))),
            );
        }
        out
    }

    fn hello(&mut self, conn: ConnectionId, hello: v1::Hello) -> S {
        if hello.protocol_version != PROTOCOL_VERSION {
            return S::Reject(v1::Reject {
                reason: format!(
                    "protocol version {} not supported (server speaks {PROTOCOL_VERSION}); please update",
                    hello.protocol_version
                ),
            });
        }
        // Junctions are vertex indices: both sides need the same graph.
        let ours = self.session.network().map(|n| world_id(n));
        if let (Some(ours), Some(theirs)) = (&ours, &hello.world)
            && ours.graph_hash != theirs.graph_hash
        {
            return S::Reject(v1::Reject {
                reason: format!(
                    "world package mismatch: the server rides {} ({:016x}), you have {} ({:016x}); install the same package version",
                    ours.name, ours.graph_hash, theirs.name, theirs.graph_hash
                ),
            });
        }
        let profile = physics_profile(&hello);
        let rider = self.session.add_rider(hello.display_name.clone(), profile);
        // The bike's parts (spec §14 Phase 4, 4G): the server's own values.
        if !hello.parts.is_empty() {
            let build =
                ocw_sim::build::build(ocw_sim::PartTable::shipped(), profile.bike, &hello.parts);
            self.session.set_bike(rider, profile, Some(build));
        }
        // Cosmetic only (§2.5): validated, then passed through.
        if let Some(look) = &hello.look {
            self.session.set_look(rider, look.into());
        }
        self.riders.insert(conn, rider);
        self.names.insert(conn, hello.display_name.clone());
        self.joined.push(hello.display_name.clone());
        let clock = Some(self.clock_message());
        tracing::info!(%rider, name = %hello.display_name, weight_kg = profile.rider_mass_kg, bike = profile.bike.id(), "rider joined");
        S::Welcome(v1::Welcome {
            protocol_version: PROTOCOL_VERSION,
            rider_id: rider.0,
            tick_rate_hz: SIM_TICK_HZ,
            broadcast_rate_hz: BROADCAST_HZ,
            route_id: self.session.route().id().to_owned(),
            free_ride: self.session.is_free_ride(),
            world: ours,
            graph_route: self
                .session
                .plan()
                .map(|p| p.name.clone())
                .unwrap_or_default(),
            voice: self.config.voice.is_some(),
            clock,
            world_access: ocw_protocol::access_to_wire(self.config.world_access),
            god_access: ocw_protocol::access_to_wire(self.config.god_access),
            echo_bot: self.config.allow_echo_bot,
            collisions: ocw_protocol::collision_mode_to_wire(self.session.collision_mode()),
            collision_access: ocw_protocol::access_to_wire(self.config.collision_access),
            chaotic_bots: self.session.chaotic_bots(),
            weather: Some(self.weather_message()),
        })
    }

    /// The collision rules (spec §14 Phase 12A): who may change them is
    /// checked here (`collision_access`); everyone sees the new rules in
    /// the next snapshot.
    fn rules_request(&mut self, rider: RiderId, req: v1::RulesRequest) -> S {
        let result = |ok: bool, message: String| S::DevResult(v1::DevCommandResult { ok, message });
        if !self
            .config
            .collision_access
            .allows(Some(rider), self.host())
        {
            return result(
                false,
                match self.config.collision_access {
                    ClockAccess::Nobody => "the organizer fixed the collision rules".into(),
                    _ => "only the host may change the collision rules".into(),
                },
            );
        }
        let mut said = Vec::new();
        if let Some(on) = req.collisions {
            self.session.set_collisions(on);
            self.config.collisions = on;
            said.push(format!("collisions {}", if on { "on" } else { "off" }));
        }
        if let Some(on) = req.chaotic_bots {
            self.session.set_chaotic_bots(on);
            self.config.chaotic_bots = on;
            said.push(format!("chaotic bots {}", if on { "on" } else { "off" }));
        }
        if said.is_empty() {
            return result(false, "empty rules request".into());
        }
        let mode = self.session.collision_mode();
        if req.collisions == Some(true) && mode == ocw_sim::collide::Mode::Soft {
            said.push("soft bumps only in this group ride".into());
        }
        result(true, said.join(", "))
    }

    /// The workshop's bike (spec §14 Phase 4, 4A, Q1, 4G): applied at once
    /// in free ride and outside events; an entrant of an event that is not
    /// over keeps the bike it started with and gets this one when the event
    /// ends (from the next start). The profile comes from the server's own
    /// table (unknown id: the default road profile) with the rider's
    /// weight, the parts' physics from the server's catalogue copy
    /// ([`ocw_sim::build`]: real weights, class steps, the build caps;
    /// unknown or unfitting ids count as base parts; ownership is checked
    /// with accounts, Phase 5); the look is cosmetic and applies at once.
    /// Everyone learns the new build from a `RiderBuild` once it applies.
    fn bike_change(&mut self, rider: RiderId, change: &v1::BikeChange) -> S {
        let result = |ok: bool, message: String| S::DevResult(v1::DevCommandResult { ok, message });
        let bike = ocw_core::ProfileId::from_wire(Some(&change.bike_profile));
        let kg = self
            .session
            .base_profile(rider)
            .map_or(ocw_core::RiderProfile::default().rider_mass_kg, |p| {
                p.rider_mass_kg
            });
        let build = ocw_sim::build::build(ocw_sim::PartTable::shipped(), bike, &change.parts);
        let effect = build_line(&build);
        let applied = self.session.set_bike(
            rider,
            ocw_core::RiderProfile::curated(bike, kg),
            Some(build),
        );
        if let Some(look) = &change.look {
            self.session.set_look(rider, look.into());
        }
        tracing::info!(%rider, bike = bike.id(), parts = change.parts.len(), ?applied, "bike changed");
        match applied {
            Some(ocw_sim::BikeApplied::Now) => result(
                true,
                format!("riding the {} ({}): {effect}", bike.name(), change.bike_id),
            ),
            Some(ocw_sim::BikeApplied::AtNextStart) => result(
                true,
                format!(
                    "in an event: the {} ({effect}) applies when it is over, from the next start",
                    bike.name()
                ),
            ),
            None => result(false, "no rider".into()),
        }
    }

    /// The echo test (spec §7 *Echo bot*): the Echo rider on the road
    /// (ahead of the asker) or away.
    fn echo_request(&mut self, rider: RiderId, req: v1::EchoRequest) -> S {
        let result = |ok: bool, message: String| S::DevResult(v1::DevCommandResult { ok, message });
        if !self.config.allow_echo_bot {
            return result(
                false,
                "this server has no Echo rider (ocw-server --echo-bot)".into(),
            );
        }
        if !req.on {
            return if self.session.remove_echo() {
                tracing::info!(%rider, "echo rider removed");
                result(true, "Echo rider gone".into())
            } else {
                result(true, "no Echo rider on the road".into())
            };
        }
        let existed = self.session.echo_rider().is_some();
        let near = (req.here || !existed).then_some(rider);
        let id = self.session.place_echo(ECHO_NAME, near);
        tracing::info!(%rider, echo = %id, here = req.here, "echo rider placed");
        result(
            true,
            if near.is_some() {
                format!(
                    "Echo rider {:.0} m ahead: ride to it and speak",
                    ocw_sim::session::ECHO_AHEAD_M
                )
            } else {
                "Echo rider on the road".into()
            },
        )
    }

    /// A god-mode request from `rider` (spec §14 Phase 13): who may is
    /// checked here (`god_access`, not in a running event); the session
    /// freezes the road position, and on leaving places the rider on the
    /// road (§6.1: the server stays the truth for where they re-enter).
    fn god_request(&mut self, rider: RiderId, req: v1::GodRequest) -> v1::GodResult {
        let refused = |message: String, active: bool| v1::GodResult {
            ok: false,
            message,
            active,
            ..Default::default()
        };
        match req.req {
            Some(v1::god_request::Req::Enter(_)) => {
                if !self.config.god_access.allows(Some(rider), self.host()) {
                    return refused(
                        match self.config.god_access {
                            ClockAccess::Nobody => "god mode is off in this session".into(),
                            _ => "only the host of this shared ride may use god mode".into(),
                        },
                        false,
                    );
                }
                match self.session.enter_god(rider) {
                    Ok(pose) => {
                        tracing::info!(%rider, "god mode on");
                        v1::GodResult {
                            ok: true,
                            message: "god mode: you fly as a unicorn".into(),
                            active: true,
                            start: Some(ocw_protocol::god_pose_to_wire(&pose, 0, false)),
                            ..Default::default()
                        }
                    }
                    Err(e) => refused(e, false),
                }
            }
            Some(v1::god_request::Req::Leave(l)) => {
                let how = if l.fly_back {
                    ocw_sim::GodLeave::FlyBack
                } else {
                    ocw_sim::GodLeave::Land { x: l.x_m, z: l.z_m }
                };
                match self.session.leave_god(rider, how) {
                    Ok(at) => {
                        tracing::info!(%rider, fly_back = l.fly_back, "god mode off");
                        let d = self.session.rider_state(rider).map_or(0.0, |s| s.distance);
                        let road = ocw_world::Track::name_at(self.session.track(rider), d);
                        v1::GodResult {
                            ok: true,
                            message: if l.fly_back {
                                format!("back on {road}")
                            } else {
                                format!("landed on {road}")
                            },
                            active: false,
                            start: None,
                            x_m: at[0],
                            y_m: at[1],
                            z_m: at[2],
                            fly_back: l.fly_back,
                        }
                    }
                    Err(e) => refused(e, self.session.is_god(rider)),
                }
            }
            None => refused("empty god request".into(), self.session.is_god(rider)),
        }
    }

    fn dev_command(&mut self, rider: RiderId, cmd: v1::DevCommand) -> S {
        let result = |ok: bool, message: String| S::DevResult(v1::DevCommandResult { ok, message });
        if !self.config.allow_dev_commands {
            return result(false, "dev commands are disabled on this server".into());
        }
        match cmd.cmd {
            Some(v1::dev_command::Cmd::TeleportDistanceM(d)) => {
                match self.session.teleport(rider, d) {
                    Some(at) if at < d - 0.5 => result(
                        true,
                        format!(
                            "teleported to {:.3} km, the end of the route (it is not a loop; riders stop there)",
                            at / 1000.0
                        ),
                    ),
                    Some(at) if at > d + 0.5 => result(
                        true,
                        format!("teleported to {:.3} km, the start", at / 1000.0),
                    ),
                    Some(at) => result(true, format!("teleported to {:.3} km", at / 1000.0)),
                    None => result(false, "no rider to teleport".into()),
                }
            }
            Some(v1::dev_command::Cmd::TeleportPoint(p)) => {
                if self.session.teleport_to(rider, p.x_m, p.z_m) {
                    result(
                        true,
                        format!(
                            "teleported to the road nearest x {:.0} z {:.0}",
                            p.x_m, p.z_m
                        ),
                    )
                } else {
                    result(false, "only in free ride".into())
                }
            }
            Some(v1::dev_command::Cmd::Stage(st)) => {
                let what = if st.crash {
                    ocw_sim::session::Stage::Crash
                } else {
                    ocw_sim::session::Stage::Bump
                };
                match self.session.stage(rider, what) {
                    Ok(m) => result(true, m),
                    Err(e) => result(false, e),
                }
            }
            Some(v1::dev_command::Cmd::BotBike(b)) => match self.bot_bike(&b) {
                Ok(m) => result(true, m),
                Err(e) => result(false, e),
            },
            None => result(false, "empty dev command".into()),
        }
    }

    /// `dev.bot_bike` (spec §14 Phase 4, W3): a bot's bike, steady power
    /// and lane, to compare a stock and an upgraded bike side by side.
    fn bot_bike(&mut self, b: &v1::BotBike) -> Result<String, String> {
        let id = RiderId(b.rider_id);
        if !self.session.bots().contains(&id) {
            return Err(format!("{id} is no bot of this session"));
        }
        let base = self.session.base_profile(id).ok_or("no such rider")?;
        let bike = if b.bike_profile.trim().is_empty() {
            base.bike
        } else {
            ocw_core::ProfileId::parse(&b.bike_profile)
                .ok_or_else(|| format!("unknown bike profile {:?}", b.bike_profile))?
        };
        let profile = ocw_core::RiderProfile::curated(bike, base.rider_mass_kg);
        let build = ocw_sim::build::build(ocw_sim::PartTable::shipped(), bike, &b.parts);
        let effect = build_line(&build);
        self.session.set_bike(id, profile, Some(build));
        if !b.name.trim().is_empty() {
            self.session.rename(id, b.name.trim());
        }
        if let Some(w) = b.power_w {
            self.session.set_bot_power(id, f64::from(w), true);
        }
        if let Some(l) = b.lane_m {
            self.session.set_bot_lane(id, f64::from(l));
        }
        tracing::info!(%id, bike = bike.id(), parts = b.parts.len(), "bot bike set");
        Ok(format!("{id}: {} – {effect}", bike.name()))
    }

    /// A voice token for the rider of `conn` (spec §7), or why not.
    fn voice_grant(
        &mut self,
        conn: ConnectionId,
        rider: RiderId,
        req: &v1::VoiceTokenRequest,
    ) -> v1::VoiceGrant {
        let Some(cfg) = &self.config.voice else {
            return voice::refused(
                req.channel,
                if self.config.allow_dev_commands {
                    "voice chat needs a multiplayer server (ocw-desktop --server)"
                } else {
                    "voice chat is not configured on this server"
                },
            );
        };
        let now = (cfg.clock)();
        if let Some(&last) = self.voice_issued.get(&conn)
            && now < last + voice::MIN_REQUEST_INTERVAL_S
        {
            return voice::refused(
                req.channel,
                "too many voice token requests; try again in a moment",
            );
        }
        let name = self.names.get(&conn).map_or("", String::as_str);
        let grant = voice::grant(cfg, req, rider, name, self.session.event(), now);
        if grant.error.is_empty() {
            self.voice_issued.insert(conn, now);
            tracing::info!(%rider, room = %grant.room, "voice token issued");
        } else {
            tracing::debug!(%rider, error = %grant.error, "voice token refused");
        }
        grant
    }

    /// Starts or cancels an event (spec §2.3; local server only).
    fn event_request(&mut self, req: v1::EventRequest) -> S {
        let result = |ok: bool, message: String| S::DevResult(v1::DevCommandResult { ok, message });
        if !self.config.allow_local_events {
            return result(
                false,
                "events on this server are scheduled by the organizer".into(),
            );
        }
        match req.req {
            Some(v1::event_request::Req::Cancel(_)) => match self.session.cancel_event() {
                Some(ev) => {
                    self.event_dirty = true;
                    result(true, format!("{} cancelled", ev.kind.name()))
                }
                None => result(false, "no event to cancel".into()),
            },
            Some(v1::event_request::Req::Start(start)) => {
                let Some(spec) = ocw_protocol::event_spec(&start) else {
                    return result(false, "unknown event kind".into());
                };
                if self.session.course_length().is_none() {
                    return result(
                        false,
                        "events need a route (free ride has no fixed course)".into(),
                    );
                }
                // The field: a time trial is ridden alone; races and group
                // rides get the bots asked for (or keep the current ones).
                let bots = match spec {
                    EventSpec::TimeTrial { .. } => Some(0),
                    _ => start.bots.map(|b| b.min(MAX_EVENT_BOTS)),
                };
                if let Some(n) = bots {
                    for id in self.session.bots() {
                        self.session.remove_rider(id);
                    }
                    for i in 0..n {
                        add_numbered_bot(&mut self.session, i);
                    }
                }
                let organiser = start.collisions.map(ocw_protocol::collision_mode_from_wire);
                let physics = ocw_protocol::event_physics(&start);
                let started = self.session.start_event(spec).map(|e| e.id);
                if started.is_ok() {
                    self.session.set_event_collisions(organiser);
                    self.session.set_event_physics(physics);
                }
                match started.map(|_| self.session.event().expect("just started")) {
                    Ok(ev) => {
                        let what = match ev.kind {
                            EventKind::GroupRide { target_w } => {
                                format!("group ride at {target_w:.0} W")
                            }
                            EventKind::Race { length_m, laps } => {
                                format!("race over {}", distance_label(length_m, laps))
                            }
                            EventKind::TimeTrial { length_m, laps } => {
                                format!("time trial over {}", distance_label(length_m, laps))
                            }
                        };
                        result(
                            true,
                            format!(
                                "{what}, {} rider{}, {} mode ({}): start in {:.0} s",
                                ev.entrants.len(),
                                if ev.entrants.len() == 1 { "" } else { "s" },
                                ev.physics.name(),
                                ev.physics.describe(),
                                ocw_sim::event::COUNTDOWN_S
                            ),
                        )
                    }
                    Err(e) => result(false, e),
                }
            }
            None => result(false, "empty event request".into()),
        }
    }

    /// The event state to send now.
    fn event_state(&self) -> v1::EventState {
        let mut e = ocw_protocol::event_state(
            self.session.event(),
            &self.session.event_standings(),
            self.session.tick(),
        );
        if self.session.event().is_some() {
            e.collisions = ocw_protocol::collision_mode_to_wire(self.session.collision_mode());
        }
        e
    }

    /// Advances one physics tick. Returns the encoded messages to broadcast
    /// to every connection: free-ride path changes (reliable, at once) and
    /// a snapshot when one is due (every 5th tick at 50/10 Hz).
    pub fn tick(&mut self) -> Vec<Vec<u8>> {
        self.session.step();
        let t = self.session_time_s();
        self.weather.tick(t);
        let mut out = Vec::new();
        // The event: at once on a change, and at 2 Hz while a race or time
        // trial runs.
        let changed = !self.session.take_event_changes().is_empty();
        let live = self
            .session
            .event()
            .is_some_and(|e| e.phase == EventPhase::Running && e.kind.length_m().is_some())
            && self.session.tick().is_multiple_of(EVENT_STATE_TICKS);
        if changed || live || std::mem::take(&mut self.event_dirty) {
            out.push(encode(&server_message(S::Event(self.event_state()))));
        }
        // Bike builds: reliable, once per change (spec §14 Phase 4, 4G).
        for id in self.session.take_build_changes() {
            out.push(self.build_message(id));
        }
        if let Some(net) = self.session.network().cloned() {
            for c in self.session.take_path_changes() {
                out.push(encode(&server_message(S::Path(ocw_protocol::path_update(
                    &c, &net,
                )))));
            }
        }
        if !self
            .session
            .tick()
            .is_multiple_of(u64::from(TICKS_PER_BROADCAST))
        {
            return out;
        }
        let echo = self.session.echo_rider();
        let snapshot = v1::Snapshot {
            server_tick: self.session.tick(),
            riders: self
                .session
                .snapshot()
                .iter()
                .map(|r| v1::RiderUpdate {
                    echo: Some(r.id) == echo,
                    hearts: self.session.hearts(r.id),
                    ..ocw_protocol::quantize(r)
                })
                .collect(),
            human_riders: self.riders.len() as u32,
            clock: Some(self.clock_message()),
            collisions: ocw_protocol::collision_mode_to_wire(self.session.collision_mode()),
            chaotic_bots: self.session.chaotic_bots(),
            weather: Some(self.weather_message()),
        };
        out.push(encode(&server_message(S::Snapshot(snapshot))));
        out
    }

    pub fn connections(&self) -> impl Iterator<Item = ConnectionId> + '_ {
        self.riders.keys().copied()
    }

    /// The encoded `RiderBuild` of rider `id`.
    fn build_message(&self, id: RiderId) -> Vec<u8> {
        let profile = self
            .session
            .base_profile(id)
            .map_or_else(Default::default, |p| p.bike);
        let look = self.session.look(id);
        encode(&server_message(S::RiderBuild(ocw_protocol::rider_build(
            id,
            profile,
            self.session.rider_build(id),
            look.as_ref(),
        ))))
    }
}

/// "+1.2 % flat, +0.9 % at 6 %, 7.4 kg" – a build's effect against the
/// profile's stock bike (4G).
pub fn build_line(b: &ocw_sim::Build) -> String {
    let e = b.effect_from(&ocw_sim::Build::stock(b.profile));
    format!("{}, {:.1} kg", e.line(), b.params.bike_mass_kg)
}

/// A new session on `course` with the configured bots.
fn new_session(course: Course, config: &ServerConfig) -> Session {
    let mut session = match course {
        Course::Route(route) => Session::new(route).with_start(config.start_m),
        Course::FreeRide { net, start } => Session::free_ride(net, start),
        Course::RouteRide { net, plan } => Session::route_ride(net, plan),
    };
    for i in 0..config.bots {
        add_numbered_bot(&mut session, i);
    }
    session.set_collisions(config.collisions);
    session.set_chaotic_bots(config.chaotic_bots);
    if config.allow_echo_bot && config.echo_bot_at_start {
        session.place_echo(ECHO_NAME, None);
    }
    session
}

/// The Echo rider's name tag (spec §7 *Echo bot*).
pub const ECHO_NAME: &str = "Echo";

/// The physics profile of a joining rider (spec §2.5, §3.2.1): the
/// curated profile the hello names (`bike_profile`; unknown or absent:
/// the default road profile) from the server's own table, with the
/// rider's weight from the hello (clamped to 30–200 kg). Drag, rolling
/// resistance and bike mass sent by the client are ignored: the server is
/// the truth (§6.1).
pub fn physics_profile(hello: &v1::Hello) -> ocw_core::RiderProfile {
    let bike = ocw_core::ProfileId::from_wire(hello.bike_profile.as_deref());
    let kg = hello
        .rider_weight_kg
        .or(hello.profile.as_ref().map(|p| p.rider_mass_kg))
        .map_or(ocw_core::RiderProfile::default().rider_mass_kg, f64::from);
    ocw_core::RiderProfile::curated(bike, kg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_protocol::{client_message, decode_server};

    fn hello(version: u32) -> Vec<u8> {
        hello_with(version, None)
    }

    fn hello_with(version: u32, world: Option<v1::WorldPackageId>) -> Vec<u8> {
        encode(&client_message(C::Hello(v1::Hello {
            protocol_version: version,
            display_name: "Tester".into(),
            profile: Some(ocw_core::RiderProfile::default().into()),
            world,
            rider_weight_kg: None,
            bike_profile: None,
            look: None,
            parts: Vec::new(),
        })))
    }

    fn join_with_weight(server: &mut GameServer, weight: Option<f32>) -> RiderId {
        let conn = server.connect();
        let reply = server.handle(
            conn,
            &encode(&client_message(C::Hello(v1::Hello {
                protocol_version: PROTOCOL_VERSION,
                display_name: "Weighed".into(),
                profile: None,
                world: None,
                rider_weight_kg: weight,
                bike_profile: None,
                look: None,
                parts: Vec::new(),
            }))),
        );
        match decode_server(&reply[0]).unwrap() {
            S::Welcome(w) => RiderId(w.rider_id),
            other => panic!("expected a welcome, got {other:?}"),
        }
    }

    fn voice_server(now: Arc<std::sync::atomic::AtomicU64>) -> GameServer {
        let clock: voice::Clock = Arc::new(move || now.load(std::sync::atomic::Ordering::SeqCst));
        GameServer::new(
            Route::test_loop(),
            ServerConfig {
                voice: Some(voice::VoiceConfig {
                    instance: "test".into(),
                    clock,
                    ..voice::VoiceConfig::new("ws://127.0.0.1:7880", "devkey", "devsecret")
                }),
                ..Default::default()
            },
        )
    }

    fn ask_voice(
        server: &mut GameServer,
        conn: ConnectionId,
        channel: v1::VoiceChannel,
        group: &str,
    ) -> v1::VoiceGrant {
        let reply = server.handle(conn, &encode(&ocw_protocol::voice_request(channel, group)));
        match decode_server(&reply[0]).unwrap() {
            S::Voice(g) => g,
            other => panic!("expected a voice grant, got {other:?}"),
        }
    }

    /// Voice tokens (spec §7): bound to the connection's rider, for the
    /// session room, rate limited, refused before the hello.
    #[test]
    fn voice_tokens_for_joined_riders() {
        use std::sync::atomic::{AtomicU64, Ordering};
        let now = Arc::new(AtomicU64::new(1_900_000_000));
        let mut server = voice_server(now.clone());
        let conn = server.connect();
        // Before the hello: rejected like any other message.
        let reply = server.handle(
            conn,
            &encode(&ocw_protocol::voice_request(v1::VoiceChannel::Session, "")),
        );
        assert!(matches!(decode_server(&reply[0]).unwrap(), S::Reject(_)));
        let welcome = server.handle(conn, &hello(PROTOCOL_VERSION));
        let S::Welcome(w) = decode_server(&welcome[0]).unwrap() else {
            panic!("no welcome")
        };
        assert!(w.voice, "the welcome says voice is offered");
        let g = ask_voice(&mut server, conn, v1::VoiceChannel::Session, "");
        assert_eq!(g.error, "");
        assert_eq!(
            g.identity,
            ocw_protocol::voice_identity(RiderId(w.rider_id))
        );
        assert_eq!(g.room, "ocw-test");
        assert_eq!(g.url, "ws://127.0.0.1:7880");
        let claims = voice::verify_jwt_hs256(&g.token, b"devsecret").unwrap();
        assert_eq!(claims["sub"], g.identity.as_str());
        assert_eq!(claims["name"], "Tester");
        assert_eq!(claims["exp"], 1_900_000_000u64 + voice::DEFAULT_TOKEN_TTL_S);
        // At once again: too many requests; later: fine.
        let again = ask_voice(&mut server, conn, v1::VoiceChannel::Session, "");
        assert!(
            again.token.is_empty() && again.error.contains("too many"),
            "{again:?}"
        );
        now.fetch_add(voice::MIN_REQUEST_INTERVAL_S, Ordering::SeqCst);
        let g = ask_voice(&mut server, conn, v1::VoiceChannel::Group, "team-1");
        assert_eq!(g.room, "ocw-test-group-team-1");
        // A second rider gets their own identity in the same room.
        let other = server.connect();
        server.handle(other, &hello(PROTOCOL_VERSION));
        let g2 = ask_voice(&mut server, other, v1::VoiceChannel::Session, "");
        assert_eq!(g2.room, "ocw-test");
        assert_ne!(g2.identity, g.identity);
        // Snapshots count the connected riders (not bots) for the
        // automatic join.
        let snap = (0..TICKS_PER_BROADCAST)
            .flat_map(|_| server.tick())
            .find_map(|b| match decode_server(&b) {
                Ok(S::Snapshot(s)) => Some(s),
                _ => None,
            })
            .unwrap();
        assert_eq!(snap.human_riders, 2);
    }

    #[test]
    fn voice_refused_without_configuration() {
        for (config, text) in [
            (ServerConfig::default(), "not configured"),
            (
                ServerConfig {
                    allow_dev_commands: true,
                    ..Default::default()
                },
                "multiplayer server",
            ),
        ] {
            let mut server = GameServer::new(Route::test_loop(), config);
            let conn = server.connect();
            let welcome = server.handle(conn, &hello(PROTOCOL_VERSION));
            let S::Welcome(w) = decode_server(&welcome[0]).unwrap() else {
                panic!("no welcome")
            };
            assert!(!w.voice);
            let g = ask_voice(&mut server, conn, v1::VoiceChannel::Session, "");
            assert!(g.token.is_empty() && g.error.contains(text), "{g:?}");
        }
    }

    /// The rider weight from the hello is the rider's mass in the server's
    /// physics (spec §3.2.1), clamped; the rest is the curated profile.
    #[test]
    fn the_hello_weight_reaches_the_physics_profile() {
        let mut server = GameServer::new(Route::test_hills(), ServerConfig::default());
        let light = join_with_weight(&mut server, Some(58.0));
        let p = server.session().rider_profile(light).unwrap();
        assert_eq!(p.rider_mass_kg, 58.0);
        assert_eq!(p.total_mass_kg(), 66.0);
        let heavy = join_with_weight(&mut server, Some(900.0));
        assert_eq!(
            server.session().rider_profile(heavy).unwrap().rider_mass_kg,
            ocw_core::MAX_RIDER_WEIGHT_KG
        );
        let none = join_with_weight(&mut server, None);
        assert_eq!(
            server.session().rider_profile(none).unwrap(),
            ocw_core::RiderProfile::default()
        );
        // The weight changes the ride: on the climb the same power carries
        // the light rider faster than the heavy one.
        for rider in [light, heavy] {
            let conn = server
                .riders
                .iter()
                .find(|(_, r)| **r == rider)
                .map(|(c, _)| *c)
                .unwrap();
            let sample = v1::SensorSample {
                seq: 1,
                power_w: Some(250.0),
                ..Default::default()
            };
            server.handle(conn, &encode(&client_message(C::Sample(sample))));
        }
        for _ in 0..(50 * 60) {
            server.tick();
        }
        let distance = |r| server.session().rider_state(r).unwrap().distance;
        assert!(
            distance(light) > distance(heavy) + 10.0,
            "{} vs {}",
            distance(light),
            distance(heavy)
        );
    }

    /// A client cannot choose its own drag or rolling resistance (§2.5):
    /// from the deprecated profile field only the rider mass counts.
    #[test]
    fn client_physics_parameters_are_ignored() {
        let hello = v1::Hello {
            profile: Some(v1::RiderProfile {
                rider_mass_kg: 70.0,
                bike_mass_kg: 1.0,
                cda_m2: 0.01,
                crr: 0.0,
            }),
            ..Default::default()
        };
        assert_eq!(
            physics_profile(&hello),
            ocw_core::RiderProfile::for_rider_weight(70.0)
        );
        let hello = v1::Hello {
            rider_weight_kg: Some(64.0),
            bike_profile: None,
            look: None,
            ..hello
        };
        assert_eq!(physics_profile(&hello).rider_mass_kg, 64.0);
        let hello = v1::Hello {
            rider_weight_kg: Some(f32::NAN),
            bike_profile: None,
            look: None,
            ..hello
        };
        assert_eq!(physics_profile(&hello).rider_mass_kg, 75.0);
    }

    /// The hello's bike profile id selects the server's curated profile;
    /// unknown ids fall back to the default, client numbers never count.
    #[test]
    fn the_hello_bike_profile_selects_the_curated_profile() {
        for id in ocw_core::ProfileId::ALL {
            let hello = v1::Hello {
                bike_profile: Some(id.id().into()),
                rider_weight_kg: Some(70.0),
                profile: Some(v1::RiderProfile {
                    rider_mass_kg: 70.0,
                    bike_mass_kg: 1.0,
                    cda_m2: 0.01,
                    crr: 0.0,
                }),
                ..Default::default()
            };
            assert_eq!(
                physics_profile(&hello),
                ocw_core::RiderProfile::curated(id, 70.0)
            );
        }
        let unknown = v1::Hello {
            bike_profile: Some("jetpack".into()),
            ..Default::default()
        };
        assert_eq!(physics_profile(&unknown), ocw_core::RiderProfile::default());
        let upper = v1::Hello {
            bike_profile: Some("MTB".into()),
            ..Default::default()
        };
        assert_eq!(physics_profile(&upper).bike, ocw_core::ProfileId::Mountain);
    }

    /// The look from the hello reaches the snapshots (validated), with the
    /// bike profile the server chose; bots ride varied bikes.
    #[test]
    fn looks_pass_through_to_the_snapshots() {
        let mut server = GameServer::new(
            Route::test_hills(),
            ServerConfig {
                bots: 4,
                ..Default::default()
            },
        );
        let conn = server.connect();
        let reply = server.handle(
            conn,
            &encode(&client_message(C::Hello(v1::Hello {
                protocol_version: PROTOCOL_VERSION,
                display_name: "Styled".into(),
                bike_profile: Some("gravel".into()),
                look: Some(v1::RiderLook {
                    frame_rgb: 0xFF12_3456,
                    jersey_rgb: 0x00FF00,
                    helmet_rgb: 0x0000FF,
                    kit_pattern: 2,
                    ..Default::default()
                }),
                ..Default::default()
            }))),
        );
        let S::Welcome(w) = decode_server(&reply[0]).unwrap() else {
            panic!("expected a welcome");
        };
        let mut snapshot = None;
        for _ in 0..TICKS_PER_BROADCAST {
            for bytes in server.tick() {
                if let Ok(S::Snapshot(s)) = decode_server(&bytes) {
                    snapshot = Some(s);
                }
            }
        }
        let snapshot = snapshot.expect("a snapshot");
        let me = snapshot
            .riders
            .iter()
            .find(|r| r.rider_id == w.rider_id)
            .unwrap();
        let (bike, look) = ocw_protocol::rider_look(me);
        assert_eq!(bike, ocw_core::ProfileId::Gravel);
        let look = look.unwrap();
        assert_eq!(look.frame, ocw_core::Rgb(0x123456));
        assert_eq!(look.pattern, ocw_core::KitPattern::Sash);
        let bikes: std::collections::BTreeSet<_> = snapshot
            .riders
            .iter()
            .filter(|r| r.rider_id != w.rider_id)
            .map(|r| ocw_protocol::rider_look(r).0.bike())
            .collect();
        assert_eq!(bikes.len(), 4, "{bikes:?}");
    }

    #[test]
    fn bots_get_distinct_bikes_and_never_the_local_pink() {
        for i in 0..40 {
            let (_, look) = bot_bike(i);
            assert_ne!(look.jersey, ocw_core::RiderLook::default().jersey);
        }
        assert_ne!(bot_bike(0).0.bike(), bot_bike(1).0.bike());
    }

    fn line_network(name: &str, length: f64) -> Arc<RoadNetwork> {
        use ocw_world::graph::{Edge, EdgeId, Oneway, RoadClass, RoadGraph, VertexId};
        let edge = Edge {
            id: EdgeId(9),
            from: VertexId(0),
            to: VertexId(1),
            points: vec![[0.0, 0.0], [length, 0.0]],
            class: RoadClass::Tertiary,
            name: None,
            width_m: 6.0,
            bridge: false,
            tunnel: false,
            dead_end_m: None,
            boundary: false,
            oneway: Oneway::No,
            osm_way: 1,
            profile: Vec::new(),
        };
        Arc::new(
            RoadNetwork::new(RoadGraph {
                vertices: vec![[0.0, 0.0], [length, 0.0]],
                edges: vec![edge],
                boundary_vertices: Default::default(),
            })
            .with_package_name(name),
        )
    }

    #[test]
    fn clients_with_another_world_package_are_rejected() {
        let net = line_network("Testland", 500.0);
        let course = || Course::FreeRide {
            net: net.clone(),
            start: FreeRideStart {
                edge: 0,
                offset_m: 10.0,
                forward: true,
            },
        };
        let mut server = GameServer::new(course(), ServerConfig::default());
        // Same graph: welcome, with the server's package identity.
        let conn = server.connect();
        let S::Welcome(w) = decode_server(
            &server.handle(conn, &hello_with(PROTOCOL_VERSION, Some(world_id(&net))))[0],
        )
        .unwrap() else {
            panic!("expected welcome")
        };
        assert_eq!(w.world, Some(world_id(&net)));
        assert!(w.free_ride);
        assert_eq!(w.graph_route, "");
        // Another build of the package (a longer road): rejected.
        let other = line_network("Testland", 501.0);
        assert_ne!(other.fingerprint(), net.fingerprint());
        let conn = server.connect();
        let S::Reject(r) = decode_server(
            &server.handle(conn, &hello_with(PROTOCOL_VERSION, Some(world_id(&other))))[0],
        )
        .unwrap() else {
            panic!("expected reject")
        };
        assert!(r.reason.contains("world package mismatch"), "{}", r.reason);
        assert_eq!(server.session().rider_count(), 1);
        // A route session (no graph): no package check, no package sent.
        let mut plain = GameServer::new(Route::test_loop(), ServerConfig::default());
        let conn = plain.connect();
        let S::Welcome(w) = decode_server(
            &plain.handle(conn, &hello_with(PROTOCOL_VERSION, Some(world_id(&other))))[0],
        )
        .unwrap() else {
            panic!("expected welcome")
        };
        assert_eq!(w.world, None);
    }

    #[test]
    fn route_rides_name_the_route_and_accept_rejoins() {
        let net = line_network("Testland", 800.0);
        let plan = ocw_world::RoutePlan::new(
            &net,
            "Line",
            vec![ocw_world::Segment::Edge {
                edge: 0,
                from_m: 0.0,
                to_m: 800.0,
            }],
            false,
        )
        .unwrap();
        let mut server = GameServer::new(
            Course::RouteRide {
                net: net.clone(),
                plan: Arc::new(plan),
            },
            ServerConfig::default(),
        );
        let conn = server.connect();
        let replies = server.handle(conn, &hello(PROTOCOL_VERSION));
        let S::Welcome(w) = decode_server(&replies[0]).unwrap() else {
            panic!("expected welcome")
        };
        assert_eq!(w.graph_route, "Line");
        assert_eq!(w.route_id, "Line");
        // The joining client gets the paths.
        assert!(replies.len() > 1);
        // On the route already: a rejoin is refused quietly.
        let rejoin = encode(&client_message(C::RouteRejoin(v1::RouteRejoin {
            distance_cm: 0,
            speed_mm_s: 0,
        })));
        assert!(server.handle(conn, &rejoin).is_empty());
    }

    fn sample(seq: u32, power: f32) -> Vec<u8> {
        encode(&client_message(C::Sample(v1::SensorSample {
            seq,
            client_time_us: 0,
            power_w: Some(power),
            cadence_rpm: None,
            heart_rate_bpm: None,
            brake: None,
            virtual_power: None,
        })))
    }

    #[test]
    fn hello_then_ride() {
        let mut server = GameServer::new(Route::test_loop(), ServerConfig::default());
        let conn = server.connect();
        let reply = server.handle(conn, &hello(PROTOCOL_VERSION));
        let S::Welcome(w) = decode_server(&reply[0]).unwrap() else {
            panic!("expected welcome")
        };
        assert_eq!(w.tick_rate_hz, 50);
        server.handle(conn, &sample(1, 250.0));
        let mut snapshots = 0;
        for _ in 0..250 {
            for bytes in server.tick() {
                snapshots += 1;
                let S::Snapshot(s) = decode_server(&bytes).unwrap() else {
                    panic!()
                };
                assert_eq!(s.riders.len(), 1);
            }
        }
        assert_eq!(snapshots, 50, "10 Hz over 5 s");
        let state = server.session().rider_state(RiderId(w.rider_id)).unwrap();
        // 250 W from standstill for 5 s: power-limited acceleration covers ~15 m.
        assert!(state.distance > 10.0, "distance {}", state.distance);
    }

    const EPOCH_MS: i64 = 1_781_000_000_000;

    fn clock_server(access: ClockAccess) -> GameServer {
        GameServer::new(
            Route::test_loop(),
            ServerConfig {
                clock_access: access,
                real_epoch_ms: Some(EPOCH_MS),
                ..Default::default()
            },
        )
    }

    fn join(server: &mut GameServer) -> (ConnectionId, v1::Welcome) {
        let conn = server.connect();
        let reply = server.handle(conn, &hello(PROTOCOL_VERSION));
        match decode_server(&reply[0]).unwrap() {
            S::Welcome(w) => (conn, w),
            other => panic!("expected a welcome, got {other:?}"),
        }
    }

    fn ask_clock(
        server: &mut GameServer,
        conn: ConnectionId,
        c: ocw_sky::ClockChange,
    ) -> (bool, String) {
        let reply = server.handle(conn, &encode(&ocw_protocol::clock_request(c)));
        match decode_server(&reply[0]).unwrap() {
            S::DevResult(r) => (r.ok, r.message),
            other => panic!("expected a result, got {other:?}"),
        }
    }

    fn next_snapshot(server: &mut GameServer) -> v1::Snapshot {
        (0..TICKS_PER_BROADCAST)
            .flat_map(|_| server.tick())
            .find_map(|b| match decode_server(&b) {
                Ok(S::Snapshot(s)) => Some(s),
                _ => None,
            })
            .expect("a snapshot every 5 ticks")
    }

    /// The world clock is server state (spec §14 Phase 9, §6.1): in the
    /// welcome and in every snapshot, on session time.
    #[test]
    fn the_world_clock_travels_in_welcome_and_snapshots() {
        let mut server = clock_server(ClockAccess::Anyone);
        let (conn, w) = join(&mut server);
        let c = ocw_protocol::clock_info(&w.clock.unwrap());
        assert_eq!(
            (c.world_ms, c.rate, c.mode),
            (EPOCH_MS, 1.0, ocw_sky::ClockMode::Real)
        );
        assert_eq!(c.host, Some(RiderId(w.rider_id)));
        let snap = next_snapshot(&mut server);
        let c = ocw_protocol::clock_info(&snap.clock.unwrap());
        assert_eq!(c.world_ms, EPOCH_MS + 100, "5 ticks of 20 ms");
        // x600 from a fixed instant: a snapshot later the world is a minute on.
        let fixed = ocw_sky::time::utc_midnight(2026, 6, 21) + 19 * ocw_sky::time::MS_PER_HOUR;
        assert!(ask_clock(&mut server, conn, ocw_sky::ClockChange::Fixed(fixed)).0);
        let (ok, msg) = ask_clock(&mut server, conn, ocw_sky::ClockChange::Speed(600.0));
        assert!(ok, "{msg}");
        assert_eq!(msg, "world clock: fixed, x600");
        let snap = next_snapshot(&mut server);
        let c = ocw_protocol::clock_info(&snap.clock.unwrap());
        assert_eq!(c.world_ms, fixed + 60_000);
        assert_eq!((c.rate, c.speed), (600.0, 600.0));
        // Out of range: refused, the clock unchanged.
        let (ok, msg) = ask_clock(
            &mut server,
            conn,
            ocw_sky::ClockChange::Fixed(ocw_sky::time::MAX_MS + 1),
        );
        assert!(!ok && msg.contains("1900"), "{msg}");
        assert!(ask_clock(&mut server, conn, ocw_sky::ClockChange::Real).0);
        assert_eq!(server.clock().mode(), ocw_sky::ClockMode::Real);
        assert_eq!(server.clock().now(server.session_time_s()), EPOCH_MS + 200);
    }

    /// Only the host of a shared session changes the time (spec §14 Phase
    /// 9, 9D, Q2): checked on the server; the host is the rider connected
    /// longest, and the next one takes over when the host leaves.
    #[test]
    fn only_the_host_may_change_the_time() {
        use ocw_sky::ClockChange as K;
        let mut server = clock_server(ClockAccess::Host);
        let (host, hw) = join(&mut server);
        let (guest, gw) = join(&mut server);
        let c = ocw_protocol::clock_info(&gw.clock.unwrap());
        assert_eq!(c.host, Some(RiderId(hw.rider_id)));
        assert!(!c.may_change(Some(RiderId(gw.rider_id))));
        let (ok, msg) = ask_clock(&mut server, guest, K::Step(3_600_000));
        assert!(!ok && msg.contains("only the host"), "{msg}");
        assert_eq!(server.clock().mode(), ocw_sky::ClockMode::Real, "unchanged");
        assert!(ask_clock(&mut server, host, K::Step(3_600_000)).0);
        assert_eq!(server.clock().mode(), ocw_sky::ClockMode::Offset);
        // The host leaves: the guest is the host now.
        server.disconnect(host);
        assert_eq!(server.host(), Some(RiderId(gw.rider_id)));
        assert!(ask_clock(&mut server, guest, K::Paused(true)).0);
        let snap = next_snapshot(&mut server);
        let c = ocw_protocol::clock_info(&snap.clock.unwrap());
        assert!(c.paused && c.may_change(Some(RiderId(gw.rider_id))));
        // Before the hello nothing goes.
        let stranger = server.connect();
        let reply = server.handle(stranger, &encode(&ocw_protocol::clock_request(K::Real)));
        assert!(matches!(decode_server(&reply[0]).unwrap(), S::Reject(_)));
        // An organizer's session: nobody.
        let mut fixed = clock_server(ClockAccess::Nobody);
        let (only, _) = join(&mut fixed);
        let (ok, msg) = ask_clock(&mut fixed, only, K::Real);
        assert!(!ok && msg.contains("organizer"), "{msg}");
        // The local server's single player may (solo = own host).
        let mut solo = clock_server(ClockAccess::Anyone);
        let (_, _) = join(&mut solo);
        let (second, _) = join(&mut solo);
        assert!(ask_clock(&mut solo, second, K::Speed(60.0)).0);
    }

    fn weather_server(access: ClockAccess) -> GameServer {
        GameServer::new(
            Route::test_loop(),
            ServerConfig {
                weather_access: access,
                real_epoch_ms: Some(EPOCH_MS),
                weather_seed: Some(5),
                ..Default::default()
            },
        )
    }

    fn ask_weather(
        server: &mut GameServer,
        conn: ConnectionId,
        c: ocw_protocol::WeatherChange,
    ) -> (bool, String) {
        let reply = server.handle(conn, &encode(&ocw_protocol::weather_request(&c)));
        match decode_server(&reply[0]).unwrap() {
            S::DevResult(r) => (r.ok, r.message),
            other => panic!("expected a result, got {other:?}"),
        }
    }

    /// The weather is server state (spec §14 Phase 9, S4, §6.1): in the
    /// welcome and every snapshot, on session time; transitions, ground
    /// and strikes reach everyone; only the host changes it (Q2).
    #[test]
    fn the_weather_travels_in_snapshots_and_only_the_host_changes_it() {
        use ocw_weather::presets::preset;
        let mut server = weather_server(ClockAccess::Host);
        let (host, hw) = join(&mut server);
        let (guest, _) = join(&mut server);
        let w = ocw_protocol::weather_info(&hw.weather.unwrap());
        assert_eq!(w.now, preset("clear").unwrap(), "clear until S7");
        assert_eq!((w.seed, w.access), (5, ClockAccess::Host));
        // The guest may not; the host may: a thunderstorm over 2 s.
        let storm = ocw_protocol::WeatherChange {
            target: Some(preset("thunderstorm").unwrap()),
            transition_s: 2.0,
            ..Default::default()
        };
        let (ok, msg) = ask_weather(&mut server, guest, storm);
        assert!(!ok && msg.contains("only the host"), "{msg}");
        let (ok, msg) = ask_weather(&mut server, host, storm);
        assert!(ok && msg.contains("thunderstorm in 2 s"), "{msg}");
        let snap = next_snapshot(&mut server);
        let w = ocw_protocol::weather_info(&snap.weather.unwrap());
        assert!(w.target.is_some() && !w.immediate);
        assert!(
            w.now.cloud_cover > 0.05 && w.now.cloud_cover < 1.0,
            "on its way"
        );
        for _ in 0..25 {
            next_snapshot(&mut server);
        }
        let snap = next_snapshot(&mut server);
        let w = ocw_protocol::weather_info(&snap.weather.unwrap());
        assert_eq!(w.now, preset("thunderstorm").unwrap());
        assert!(w.target.is_none());
        assert!(w.ground.wetness() > 0.0, "the rain wets the road");
        // A test strike reaches everyone's snapshots with its time.
        let (ok, msg) = ask_weather(
            &mut server,
            host,
            ocw_protocol::WeatherChange {
                strike: Some((1715.0, 90.0)),
                ..Default::default()
            },
        );
        assert!(ok && msg.contains("thunder in 5.0 s"), "{msg}");
        let snap = next_snapshot(&mut server);
        let w = ocw_protocol::weather_info(&snap.weather.unwrap());
        assert!(w.strikes.iter().any(|s| s.distance_m == 1715.0));
        // Ground overrides and the simulation.
        let (ok, _) = ask_weather(
            &mut server,
            host,
            ocw_protocol::WeatherChange {
                ground: ocw_weather::command::GroundChange {
                    snow_cover: Some(1.0),
                    ..Default::default()
                },
                simulate: Some(3),
                ..Default::default()
            },
        );
        assert!(ok);
        assert_eq!(server.weather().ground().snow_cover(), 1.0);
        assert_eq!(server.weather().mode(), ocw_weather::Mode::Simulated);
        let (ok, msg) = ask_weather(&mut server, host, Default::default());
        assert!(!ok && msg.contains("empty"), "{msg}");
        // An organizer's session: nobody; a local server: the single player.
        let mut fixed = weather_server(ClockAccess::Nobody);
        let (only, _) = join(&mut fixed);
        let (ok, msg) = ask_weather(&mut fixed, only, storm);
        assert!(!ok && msg.contains("organizer"), "{msg}");
        let mut solo = GameServer::new(Route::test_loop(), crate::local::local_config(0, 0.0));
        let (me, _) = join(&mut solo);
        assert!(ask_weather(&mut solo, me, storm).0);
    }

    fn world_server(access: ClockAccess) -> GameServer {
        GameServer::new(
            Route::test_loop(),
            ServerConfig {
                world_access: access,
                bots: 2,
                real_epoch_ms: Some(EPOCH_MS),
                ..Default::default()
            },
        )
    }

    fn ask_world(server: &mut GameServer, conn: ConnectionId, world: &str) -> (bool, String) {
        let reply = server.handle(conn, &encode(&ocw_protocol::world_request(world, None)));
        match decode_server(&reply[0]).unwrap() {
            S::DevResult(r) => (r.ok, r.message),
            other => panic!("expected a result, got {other:?}"),
        }
    }

    /// World → Change world in a shared session (spec §3.2.1): only the
    /// host may, checked on the server like the clock; the server keeps
    /// one request at a time for the embedding to load.
    #[test]
    fn only_the_host_may_switch_the_world() {
        let mut server = world_server(ClockAccess::Host);
        let (host, hw) = join(&mut server);
        let (guest, _) = join(&mut server);
        assert_eq!(
            ocw_protocol::access_from_wire(hw.world_access),
            ClockAccess::Host
        );
        let (ok, msg) = ask_world(&mut server, guest, "europe/blackforest");
        assert!(!ok && msg.contains("only the host"), "{msg}");
        assert!(!server.world_switch_pending());
        let (ok, msg) = ask_world(&mut server, host, "  ");
        assert!(!ok && msg.contains("which world"), "{msg}");
        let (ok, msg) = ask_world(&mut server, host, "europe/blackforest");
        assert!(ok && msg.contains("loads europe/blackforest"), "{msg}");
        // One at a time.
        let (ok, msg) = ask_world(&mut server, host, "europe/liechtenstein");
        assert!(!ok && msg.contains("being loaded"), "{msg}");
        let req = server.take_world_switch().unwrap();
        assert_eq!(
            (req.conn, req.world.as_str(), req.route.as_deref()),
            (host, "europe/blackforest", None)
        );
        assert_eq!(req.by, "Tester");
        assert!(server.take_world_switch().is_none());
        // With a route.
        let reply = server.handle(
            host,
            &encode(&ocw_protocol::world_request("li", Some("Vaduz - Malbun"))),
        );
        assert!(matches!(decode_server(&reply[0]).unwrap(), S::DevResult(r) if r.ok));
        assert_eq!(
            server.take_world_switch().unwrap().route.as_deref(),
            Some("Vaduz - Malbun")
        );
        // Nobody / anyone.
        let mut fixed = world_server(ClockAccess::Nobody);
        let (only, _) = join(&mut fixed);
        let (ok, msg) = ask_world(&mut fixed, only, "x");
        assert!(!ok && msg.contains("organizer"), "{msg}");
        let mut open = world_server(ClockAccess::Anyone);
        join(&mut open);
        let (second, _) = join(&mut open);
        assert!(ask_world(&mut open, second, "x").0);
        // Before the hello: rejected.
        let stranger = server.connect();
        let reply = server.handle(stranger, &encode(&ocw_protocol::world_request("x", None)));
        assert!(matches!(decode_server(&reply[0]).unwrap(), S::Reject(_)));
    }

    /// The switch itself: a new session (bots again, nobody joined), the
    /// clock goes on, every client is told and joins again on its
    /// connection.
    #[test]
    fn a_world_switch_restarts_the_session_and_everyone_joins_again() {
        let mut server = world_server(ClockAccess::Host);
        let (host, hw) = join(&mut server);
        let (guest, _) = join(&mut server);
        for _ in 0..50 {
            server.tick();
        }
        let t = server.session_time_s();
        assert_eq!(server.session().rider_count(), 4);
        assert!(ask_world(&mut server, host, "hills").0);
        let req = server.take_world_switch().unwrap();
        let (conns, bytes) = server.switch_world(
            Route::test_hills(),
            v1::WorldChange {
                package_id: req.world.clone(),
                name: "Hills".into(),
                by: req.by.clone(),
                ..Default::default()
            },
        );
        assert_eq!(conns, [host, guest]);
        let S::WorldChange(c) = decode_server(&bytes).unwrap() else {
            panic!("a world change")
        };
        assert_eq!((c.package_id.as_str(), c.by.as_str()), ("hills", "Tester"));
        // The new session: only the bots, at its start; the clock went on.
        assert_eq!(server.session().rider_count(), 2);
        assert_eq!(server.session().route().name(), Route::test_hills().name());
        assert_eq!(server.session_time_s(), t, "no jump back");
        assert_eq!(server.clock().now(server.session_time_s()), EPOCH_MS + 1000);
        assert_eq!(server.connections().count(), 0);
        assert!(
            server
                .tick()
                .iter()
                .all(|m| !matches!(decode_server(m), Ok(S::WorldChange(_))))
        );
        // A sample before the new hello is refused; the hello joins again.
        let reply = server.handle(guest, &sample(1, 100.0));
        assert!(matches!(decode_server(&reply[0]).unwrap(), S::Reject(_)));
        let reply = server.handle(host, &hello(PROTOCOL_VERSION));
        let S::Welcome(w) = decode_server(&reply[0]).unwrap() else {
            panic!("welcome")
        };
        // A rider of the new session (ids count again from the bots on).
        let _ = hw;
        assert_eq!(server.session().rider_count(), 3);
        assert_eq!(w.route_id, Route::test_hills().id());
        assert_eq!(server.host(), Some(RiderId(w.rider_id)), "still the host");
        assert!(matches!(
            decode_server(&result_message(false, "nope")).unwrap(),
            S::DevResult(r) if !r.ok && r.message == "nope"
        ));
    }

    #[test]
    fn rejects_wrong_version_and_missing_hello() {
        let mut server = GameServer::new(Route::test_loop(), ServerConfig::default());
        let conn = server.connect();
        assert!(matches!(
            decode_server(&server.handle(conn, &sample(1, 100.0))[0]).unwrap(),
            S::Reject(_)
        ));
        assert!(matches!(
            decode_server(&server.handle(conn, &hello(99))[0]).unwrap(),
            S::Reject(_)
        ));
        assert_eq!(server.session().rider_count(), 0);
    }

    #[test]
    fn dev_commands_only_on_local_server() {
        let teleport = encode(&client_message(C::Dev(v1::DevCommand {
            cmd: Some(v1::dev_command::Cmd::TeleportDistanceM(1000.0)),
        })));
        for allow in [false, true] {
            let mut server = GameServer::new(
                Route::test_loop(),
                ServerConfig {
                    allow_dev_commands: allow,
                    bots: 0,
                    start_m: 0.0,
                    allow_local_events: false,
                    voice: None,
                    ..Default::default()
                },
            );
            let conn = server.connect();
            server.handle(conn, &hello(PROTOCOL_VERSION));
            let S::DevResult(r) = decode_server(&server.handle(conn, &teleport)[0]).unwrap() else {
                panic!()
            };
            assert_eq!(r.ok, allow);
        }
    }

    /// Regression: `teleport` past the end of a route that ends was
    /// accepted as it was; now it goes to the end and says so.
    #[test]
    fn teleport_past_the_end_of_a_route_goes_to_its_end() {
        let pts = (0..=100).map(|i| [f64::from(i) * 10.0, 0.0, 0.0]).collect();
        let route = Route::from_points("end", "End", pts, false, 6.0).unwrap();
        let mut server = GameServer::new(
            route,
            ServerConfig {
                allow_dev_commands: true,
                bots: 0,
                ..Default::default()
            },
        );
        let conn = server.connect();
        let S::Welcome(w) =
            decode_server(&server.handle(conn, &hello(PROTOCOL_VERSION))[0]).unwrap()
        else {
            panic!("expected a welcome")
        };
        let teleport = |km: f64| {
            encode(&client_message(C::Dev(v1::DevCommand {
                cmd: Some(v1::dev_command::Cmd::TeleportDistanceM(km * 1000.0)),
            })))
        };
        let S::DevResult(r) = decode_server(&server.handle(conn, &teleport(1.5))[0]).unwrap()
        else {
            panic!()
        };
        assert!(r.ok, "{}", r.message);
        assert!(
            r.message.contains("1.000 km, the end of the route"),
            "{}",
            r.message
        );
        let state = server.session().rider_state(RiderId(w.rider_id)).unwrap();
        assert_eq!(state.distance, 1000.0);
        let S::DevResult(r) = decode_server(&server.handle(conn, &teleport(0.5))[0]).unwrap()
        else {
            panic!()
        };
        assert_eq!(r.message, "teleported to 0.500 km");
    }

    #[test]
    fn bots_ride_at_different_strengths() {
        let n = BOT_POWERS_W.len() as u32;
        assert!(n >= 10);
        let powers: Vec<f64> = (0..n).map(bot_power_w).collect();
        // Some are clearly slower, some faster, and they are all different.
        assert!(
            powers.iter().filter(|&&p| p < 140.0).count() >= 4,
            "{powers:?}"
        );
        assert!(powers.iter().filter(|&&p| p > 200.0).count() >= 3);
        assert!(powers.iter().all(|&p| (85.0..=280.0).contains(&p)));
        let mut sorted = powers.clone();
        sorted.sort_by(f64::total_cmp);
        sorted.dedup();
        assert_eq!(sorted.len(), n as usize);
        // The first bots (a small server) already mix slow and fast.
        assert!(bot_power_w(1) < bot_power_w(0) - 50.0);
        let first: Vec<f64> = (0..4).map(bot_power_w).collect();
        assert!(first.iter().any(|&p| p < 100.0) && first.iter().any(|&p| p > 200.0));
        // Deterministic, and it repeats for big fields.
        assert_eq!(bot_power_w(n + 3), bot_power_w(3));
    }

    #[test]
    fn many_bots_start_in_formation_on_the_road() {
        let route = Route::test_loop();
        let limit = ocw_sim::physics::lateral_limit(route.road_width_m());
        let server = GameServer::new(
            route,
            ServerConfig {
                allow_dev_commands: false,
                bots: 20,
                start_m: 200.0,
                allow_local_events: false,
                voice: None,
                ..Default::default()
            },
        );
        let snap = server.session().snapshot();
        assert_eq!(snap.len(), 20);
        let mut spots: Vec<(i64, i64)> = snap
            .iter()
            .map(|r| {
                assert!(r.state.lateral_offset.abs() < limit, "{r:?}");
                assert!(r.state.distance <= 200.0 && r.state.distance >= 180.0);
                (
                    (r.state.distance * 10.0).round() as i64,
                    (r.state.lateral_offset * 10.0).round() as i64,
                )
            })
            .collect();
        spots.sort();
        spots.dedup();
        assert_eq!(spots.len(), 20);
    }

    #[test]
    fn disconnect_removes_rider_and_bots_stay() {
        let mut server = GameServer::new(
            Route::test_loop(),
            ServerConfig {
                allow_dev_commands: false,
                bots: 3,
                start_m: 0.0,
                allow_local_events: false,
                voice: None,
                ..Default::default()
            },
        );
        let conn = server.connect();
        server.handle(conn, &hello(PROTOCOL_VERSION));
        assert_eq!(server.session().rider_count(), 4);
        assert_eq!(server.joined(), ["Tester"]);
        assert!(server.left().is_empty());
        server.disconnect(conn);
        assert_eq!(server.session().rider_count(), 3);
        assert_eq!(server.left(), ["Tester"]);
        // A connection that never said hello leaves no trace.
        let quiet = server.connect();
        server.disconnect(quiet);
        assert_eq!((server.joined().len(), server.left().len()), (1, 1));
    }

    fn event_msg(req: v1::event_request::Req) -> Vec<u8> {
        encode(&client_message(C::Event(v1::EventRequest {
            req: Some(req),
        })))
    }

    fn start(spec: EventSpec, bots: Option<u32>) -> Vec<u8> {
        event_msg(v1::event_request::Req::Start(ocw_protocol::event_start(
            &spec, bots,
        )))
    }

    /// The echo test's rider (spec §7 *Echo bot*): placed on request
    /// ahead of the asker, marked in the snapshots, not a human rider;
    /// refused where the server does not allow it.
    #[test]
    fn echo_rider_on_request() {
        let snapshot = |server: &mut GameServer| {
            (0..TICKS_PER_BROADCAST)
                .flat_map(|_| server.tick())
                .find_map(|b| match decode_server(&b) {
                    Ok(S::Snapshot(s)) => Some(s),
                    _ => None,
                })
                .unwrap()
        };
        let ask = |server: &mut GameServer, conn, on, here| {
            let r = server.handle(conn, &encode(&ocw_protocol::echo_request(on, here)));
            match decode_server(&r[0]).unwrap() {
                S::DevResult(d) => d,
                other => panic!("{other:?}"),
            }
        };
        let mut server = GameServer::new(
            Route::test_loop(),
            ServerConfig {
                bots: 1,
                allow_echo_bot: true,
                ..Default::default()
            },
        );
        let conn = server.connect();
        let welcome = server.handle(conn, &hello(PROTOCOL_VERSION));
        let S::Welcome(w) = decode_server(&welcome[0]).unwrap() else {
            panic!("no welcome")
        };
        assert!(w.echo_bot);
        assert_eq!(ocw_protocol::echo_rider(&snapshot(&mut server)), None);
        let r = ask(&mut server, conn, true, false);
        assert!(r.ok && r.message.contains("ahead"), "{r:?}");
        let snap = snapshot(&mut server);
        let echo = ocw_protocol::echo_rider(&snap).expect("an Echo rider");
        let me = snap
            .riders
            .iter()
            .find(|u| u.rider_id == w.rider_id)
            .unwrap();
        let e = snap.riders.iter().find(|u| u.rider_id == echo.0).unwrap();
        assert_eq!(e.display_name, ECHO_NAME);
        assert!(e.distance_cm > me.distance_cm, "ahead of the asker");
        assert_eq!(snap.riders.iter().filter(|u| u.echo).count(), 1);
        assert_eq!(snap.human_riders, 1, "not a human rider");
        // Asked again (not "here"): it stays; "here" moves it.
        let at = e.distance_cm;
        ask(&mut server, conn, true, false);
        let snap = snapshot(&mut server);
        let e = snap.riders.iter().find(|u| u.echo).unwrap();
        assert_eq!(e.distance_cm, at);
        assert_eq!(e.rider_id, echo.0, "one Echo rider");
        assert!(ask(&mut server, conn, false, false).ok);
        assert_eq!(ocw_protocol::echo_rider(&snapshot(&mut server)), None);
        // Not allowed: refused.
        let mut server = local(0);
        server.config.allow_echo_bot = false;
        let conn = server.connect();
        server.handle(conn, &hello(PROTOCOL_VERSION));
        let r = ask(&mut server, conn, true, false);
        assert!(!r.ok && r.message.contains("--echo-bot"));
        // `ocw-server --echo-bot`: one at the start of the session.
        let mut server = GameServer::new(
            Route::test_loop(),
            ServerConfig {
                allow_echo_bot: true,
                echo_bot_at_start: true,
                ..Default::default()
            },
        );
        assert!(ocw_protocol::echo_rider(&snapshot(&mut server)).is_some());
    }

    fn local(bots: u32) -> GameServer {
        GameServer::new(
            Route::test_loop(),
            ServerConfig {
                allow_dev_commands: true,
                bots,
                start_m: 500.0,
                allow_local_events: true,
                voice: None,
                ..Default::default()
            },
        )
    }

    /// Ticks `n` times; the event states broadcast meanwhile.
    fn tick_events(server: &mut GameServer, n: usize) -> Vec<ocw_protocol::EventInfo> {
        let mut out = Vec::new();
        for _ in 0..n {
            for bytes in server.tick() {
                if let S::Event(e) = decode_server(&bytes).unwrap()
                    && let Some(info) = ocw_protocol::event_info(&e)
                {
                    out.push(info);
                }
            }
        }
        out
    }

    #[test]
    fn local_race_over_the_wire() {
        let mut server = local(10);
        let conn = server.connect();
        server.handle(conn, &hello(PROTOCOL_VERSION));
        let race = EventSpec::Race {
            laps: None,
            length_m: Some(300.0),
        };
        let S::DevResult(r) =
            decode_server(&server.handle(conn, &start(race, Some(3)))[0]).unwrap()
        else {
            panic!()
        };
        assert!(r.ok, "{}", r.message);
        assert!(
            r.message.contains("race over 0.3 km, 4 riders"),
            "{}",
            r.message
        );
        // The field: three bots and the rider.
        assert_eq!(server.session().rider_count(), 4);
        let states = tick_events(&mut server, 1);
        assert_eq!(states.len(), 1);
        assert_eq!(states[0].phase, EventPhase::Countdown);
        assert_eq!(states[0].standings.len(), 4);
        // GO is broadcast at once, then the standings at 2 Hz.
        let states = tick_events(&mut server, ocw_sim::event::COUNTDOWN_TICKS as usize + 50);
        assert_eq!(states[0].phase, EventPhase::Running);
        assert!((2..=4).contains(&states.len()), "{}", states.len());
        // A rider who joins later gets the event at once.
        let late = server.connect();
        let replies = server.handle(late, &hello(PROTOCOL_VERSION));
        assert!(
            replies
                .iter()
                .any(|b| matches!(decode_server(b), Ok(S::Event(_))))
        );
        server.disconnect(late);
        // Cancel: an empty state.
        let S::DevResult(r) = decode_server(
            &server.handle(conn, &event_msg(v1::event_request::Req::Cancel(true)))[0],
        )
        .unwrap() else {
            panic!()
        };
        assert!(r.ok);
        let mut cancelled = false;
        for bytes in server.tick() {
            if let S::Event(e) = decode_server(&bytes).unwrap() {
                cancelled = ocw_protocol::event_info(&e).is_none();
            }
        }
        assert!(cancelled);
        assert!(server.session().event().is_none());
    }

    #[test]
    fn time_trials_are_ridden_alone_and_remote_servers_refuse() {
        let mut server = local(5);
        let conn = server.connect();
        server.handle(conn, &hello(PROTOCOL_VERSION));
        let tt = EventSpec::TimeTrial {
            laps: None,
            length_m: None,
        };
        let S::DevResult(r) = decode_server(&server.handle(conn, &start(tt, Some(5)))[0]).unwrap()
        else {
            panic!()
        };
        assert!(r.ok, "{}", r.message);
        assert!(r.message.contains("1 lap"), "{}", r.message);
        assert_eq!(server.session().rider_count(), 1);
        // A group ride without a bot count keeps the field (none now).
        let group = EventSpec::GroupRide { target_w: None };
        server.handle(conn, &start(group, None));
        assert_eq!(server.session().rider_count(), 1);
        server.handle(conn, &start(group, Some(4)));
        assert_eq!(server.session().rider_count(), 5);
        // Event bots are as varied as the startup bots (regression: they
        // all were the pink default rider on the standard road bike).
        let bots = server.session().bots();
        let field: Vec<_> = server
            .session()
            .snapshot()
            .into_iter()
            .filter(|r| bots.contains(&r.id))
            .collect();
        assert_eq!(field.len(), 4);
        let bikes: std::collections::BTreeSet<_> = field.iter().map(|r| r.bike.bike()).collect();
        assert_eq!(bikes.len(), 4, "{bikes:?}");
        for r in &field {
            assert_ne!(r.look.jersey, ocw_core::RiderLook::default().jersey);
        }
        // Remote server: not by riders.
        let mut remote = GameServer::new(Route::test_loop(), ServerConfig::default());
        let conn = remote.connect();
        remote.handle(conn, &hello(PROTOCOL_VERSION));
        let S::DevResult(r) = decode_server(&remote.handle(conn, &start(tt, None))[0]).unwrap()
        else {
            panic!()
        };
        assert!(!r.ok && r.message.contains("organizer"), "{}", r.message);
        assert!(remote.session().event().is_none());
    }

    #[test]
    fn free_rides_have_no_events() {
        let net = line_network("Testland", 500.0);
        let mut server = GameServer::new(
            Course::FreeRide {
                net,
                start: FreeRideStart {
                    edge: 0,
                    offset_m: 10.0,
                    forward: true,
                },
            },
            ServerConfig {
                allow_local_events: true,
                voice: None,
                bots: 2,
                ..Default::default()
            },
        );
        let conn = server.connect();
        server.handle(conn, &hello(PROTOCOL_VERSION));
        let race = EventSpec::Race {
            laps: None,
            length_m: None,
        };
        let S::DevResult(r) =
            decode_server(&server.handle(conn, &start(race, Some(1)))[0]).unwrap()
        else {
            panic!()
        };
        assert!(!r.ok && r.message.contains("route"), "{}", r.message);
        // The bots were not touched.
        assert_eq!(server.session().rider_count(), 3);
    }

    fn god_result(
        server: &mut GameServer,
        conn: ConnectionId,
        msg: v1::ClientMessage,
    ) -> v1::GodResult {
        match decode_server(&server.handle(conn, &encode(&msg))[0]).unwrap() {
            S::God(r) => r,
            other => panic!("expected a god result, got {other:?}"),
        }
    }

    fn joined(server: &mut GameServer) -> (ConnectionId, RiderId) {
        let conn = server.connect();
        let reply = server.handle(conn, &hello(PROTOCOL_VERSION));
        match decode_server(&reply[0]).unwrap() {
            S::Welcome(w) => (conn, RiderId(w.rider_id)),
            other => panic!("expected a welcome, got {other:?}"),
        }
    }

    /// God mode on the server (spec §14 Phase 13E, 13F): the host may (by
    /// default), others not; the pose reaches the snapshots, too fast poses
    /// are dropped; leaving lands on the route.
    #[test]
    fn god_mode_access_poses_and_landing() {
        let mut server = GameServer::new(Route::test_loop(), ServerConfig::default());
        let (host, host_id) = joined(&mut server);
        let (guest, _) = joined(&mut server);
        let r = god_result(&mut server, guest, ocw_protocol::god_enter());
        assert!(!r.ok && !r.active && r.message.contains("host"), "{r:?}");
        let r = god_result(&mut server, host, ocw_protocol::god_enter());
        assert!(r.ok && r.active, "{r:?}");
        let start = ocw_protocol::god_pose_from_wire(r.start.as_ref().unwrap());
        // The welcome says who may.
        let mut s2 = GameServer::new(Route::test_loop(), ServerConfig::default());
        let c = s2.connect();
        let S::Welcome(w) = decode_server(&s2.handle(c, &hello(PROTOCOL_VERSION))[0]).unwrap()
        else {
            panic!()
        };
        assert_eq!(
            ocw_protocol::access_from_wire(w.god_access),
            ClockAccess::Host
        );
        // A plausible pose is passed on, one 50 km away is not.
        let pose = |x: f64, teleport: bool| {
            encode(&client_message(C::GodPose(ocw_protocol::god_pose_to_wire(
                &ocw_sim::god::GodPose {
                    pos: [x, start.pos[1] + 50.0, start.pos[2]],
                    ..start
                },
                1,
                teleport,
            ))))
        };
        next_snapshot(&mut server);
        assert!(
            server
                .handle(host, &pose(start.pos[0] + 30.0, false))
                .is_empty()
        );
        let snap = next_snapshot(&mut server);
        let me = snap
            .riders
            .iter()
            .find(|u| u.rider_id == host_id.0)
            .unwrap();
        let god = ocw_protocol::rider_god(me).expect("in god mode");
        assert!((god.pos[0] - start.pos[0] - 30.0).abs() < 1e-6);
        server.handle(host, &pose(start.pos[0] + 50_000.0, false));
        // Not a dev server: a "teleport" is checked like any pose.
        server.handle(host, &pose(start.pos[0] + 50_000.0, true));
        let snap = next_snapshot(&mut server);
        let me = snap
            .riders
            .iter()
            .find(|u| u.rider_id == host_id.0)
            .unwrap();
        assert!((ocw_protocol::rider_god(me).unwrap().pos[0] - start.pos[0] - 30.0).abs() < 1e-6);
        assert_eq!(server.session().god(host_id).unwrap().refused, 2);
        // Out of the physics: the road position does not move.
        let d = server.session().rider_state(host_id).unwrap().distance;
        for _ in 0..100 {
            server.tick();
        }
        assert_eq!(server.session().rider_state(host_id).unwrap().distance, d);
        // Land 300 m along the loop.
        let p = server.session().route().sample(300.0);
        let r = god_result(
            &mut server,
            host,
            ocw_protocol::god_leave(false, p.position[0], p.position[2]),
        );
        assert!(r.ok && !r.active, "{r:?}");
        assert!((r.x_m - p.position[0]).abs() < 1.0 && (r.z_m - p.position[2]).abs() < 1.0);
        let st = server.session().rider_state(host_id).unwrap();
        assert!(
            (st.distance - 300.0).abs() < 1.0 && st.speed == 0.0,
            "{st:?}"
        );
        let snap = next_snapshot(&mut server);
        let me = snap
            .riders
            .iter()
            .find(|u| u.rider_id == host_id.0)
            .unwrap();
        assert_eq!(ocw_protocol::rider_god(me), None);
        // Leaving again: refused.
        let r = god_result(&mut server, host, ocw_protocol::god_leave(true, 0.0, 0.0));
        assert!(!r.ok);
    }

    #[test]
    fn god_mode_is_refused_when_off_and_in_running_events() {
        let mut server = GameServer::new(
            Route::test_loop(),
            ServerConfig {
                god_access: ClockAccess::Nobody,
                ..Default::default()
            },
        );
        let (conn, _) = joined(&mut server);
        let r = god_result(&mut server, conn, ocw_protocol::god_enter());
        assert!(!r.ok && r.message.contains("off"), "{r:?}");
        // The local server: anyone, but not during an event.
        let mut server = local(0);
        let (conn, rider) = joined(&mut server);
        let race = EventSpec::Race {
            laps: None,
            length_m: Some(400.0),
        };
        server.handle(conn, &start(race, Some(0)));
        server.tick();
        let r = god_result(&mut server, conn, ocw_protocol::god_enter());
        assert!(!r.ok && r.message.contains("event"), "{r:?}");
        assert!(!server.session().is_god(rider));
        server.handle(conn, &event_msg(v1::event_request::Req::Cancel(true)));
        let r = god_result(&mut server, conn, ocw_protocol::god_enter());
        assert!(r.ok, "{r:?}");
        // A dev server's teleport skips the speed check.
        let start_pose = ocw_protocol::god_pose_from_wire(r.start.as_ref().unwrap());
        let far = ocw_sim::god::GodPose {
            pos: [start_pose.pos[0] + 3_000.0, 300.0, start_pose.pos[2]],
            ..start_pose
        };
        server.handle(
            conn,
            &encode(&client_message(C::GodPose(ocw_protocol::god_pose_to_wire(
                &far, 1, true,
            )))),
        );
        assert_eq!(server.session().god(rider).unwrap().pose.pos, far.pos);
        let r = god_result(&mut server, conn, ocw_protocol::god_leave(true, 0.0, 0.0));
        assert!(r.ok && r.fly_back, "{r:?}");
    }

    /// Collision rules (spec §14 Phase 12A): in the welcome and the
    /// snapshots; the host (here: anyone on a local server) changes them,
    /// others may not on a shared server.
    #[test]
    fn collision_rules_travel_and_only_the_host_changes_them() {
        let mut server = GameServer::new(
            Route::test_loop(),
            ServerConfig {
                collisions: true,
                ..Default::default()
            },
        );
        let (host, w) = join(&mut server);
        assert_eq!(
            ocw_protocol::collision_mode_from_wire(w.collisions),
            ocw_sim::collide::Mode::Full
        );
        assert!(!w.chaotic_bots);
        assert_eq!(
            ocw_protocol::access_from_wire(w.collision_access),
            ClockAccess::Host
        );
        let (guest, _) = join(&mut server);
        let ask = |server: &mut GameServer, conn, on: bool| {
            let reply = server.handle(
                conn,
                &encode(&ocw_protocol::rules_request(Some(on), Some(on))),
            );
            match decode_server(&reply[0]).unwrap() {
                S::DevResult(r) => (r.ok, r.message),
                other => panic!("{other:?}"),
            }
        };
        let (ok, msg) = ask(&mut server, guest, false);
        assert!(!ok && msg.contains("host"), "{msg}");
        let (ok, msg) = ask(&mut server, host, false);
        assert!(ok && msg.contains("collisions off"), "{msg}");
        let reply = server.handle(
            host,
            &encode(&ocw_protocol::rules_request(None, Some(true))),
        );
        assert!(
            matches!(decode_server(&reply[0]).unwrap(), S::DevResult(r) if r.ok && r.message == "chaotic bots on")
        );
        let snap = next_snapshot(&mut server);
        assert_eq!(
            ocw_protocol::collision_mode_from_wire(snap.collisions),
            ocw_sim::collide::Mode::Off
        );
        assert!(snap.chaotic_bots);
        // A world switch keeps the rules.
        server.switch_world(Route::test_loop(), v1::WorldChange::default());
        assert!(!server.session().collisions_setting());
        assert!(server.session().chaotic_bots());
    }

    /// A staged crash (`dev.crash`, spec §14 Phase 12H): the crash state
    /// reaches the snapshots, the event info names the model.
    #[test]
    fn a_staged_crash_reaches_the_snapshots() {
        let mut server = GameServer::new(Route::test_loop(), crate::local::local_config(3, 500.0));
        let (conn, rider) = joined(&mut server);
        server.handle(conn, &sample(1, 250.0));
        for _ in 0..400 {
            server.tick();
        }
        let reply = server.handle(conn, &encode(&ocw_protocol::stage_contact(true)));
        match decode_server(&reply[0]).unwrap() {
            S::DevResult(r) => assert!(r.ok, "{}", r.message),
            other => panic!("{other:?}"),
        }
        let mut seen = None;
        for _ in 0..4 {
            let snap = next_snapshot(&mut server);
            let me = snap.riders.iter().find(|u| u.rider_id == rider.0).unwrap();
            if let Some(c) = ocw_protocol::rider_crash(me) {
                seen = Some(c);
                break;
            }
        }
        let c = seen.expect("the crash is in a snapshot");
        assert_eq!(c.kind, ocw_sim::collide::ContactKind::WheelTouch);
        assert!(c.by.is_some());
        // Remote servers refuse staging (dev commands are local only).
        let mut remote = GameServer::new(
            Route::test_loop(),
            ServerConfig {
                collisions: true,
                bots: 2,
                ..Default::default()
            },
        );
        let (conn, _) = joined(&mut remote);
        let reply = remote.handle(conn, &encode(&ocw_protocol::stage_contact(false)));
        match decode_server(&reply[0]).unwrap() {
            S::DevResult(r) => assert!(!r.ok),
            other => panic!("{other:?}"),
        }
        // Group rides: soft bumps only, in the event's state.
        let mut server = GameServer::new(Route::test_loop(), crate::local::local_config(3, 500.0));
        let (conn, _) = joined(&mut server);
        let start = encode(&client_message(C::Event(v1::EventRequest {
            req: Some(v1::event_request::Req::Start(v1::EventStart {
                kind: v1::EventKind::GroupRide as i32,
                ..Default::default()
            })),
        })));
        let replies = server.handle(conn, &start);
        let state = replies
            .iter()
            .chain(server.tick().iter())
            .find_map(|b| match decode_server(b) {
                Ok(S::Event(e)) => Some(e),
                _ => None,
            })
            .expect("an event state");
        assert_eq!(
            ocw_protocol::collision_mode_from_wire(state.collisions),
            ocw_sim::collide::Mode::Soft
        );
    }

    fn bike_change_result(
        server: &mut GameServer,
        conn: ConnectionId,
        profile: &str,
    ) -> v1::DevCommandResult {
        let msg = encode(&client_message(C::BikeChange(v1::BikeChange {
            bike_id: profile.into(),
            bike_profile: profile.into(),
            look: Some(v1::RiderLook {
                frame_rgb: 0x43A047,
                ..ocw_core::RiderLook::default().into()
            }),
            parts: vec!["wheels.road-carbon.top".into()],
        })));
        let reply = server.handle(conn, &msg);
        match decode_server(&reply[0]).unwrap() {
            S::DevResult(r) => r,
            other => panic!("{other:?}"),
        }
    }

    /// The workshop's bike applies when the rider leaves it (spec §14
    /// Phase 4, 4A, Q1): at once outside events, from the next start for
    /// an entrant of a running event; the server's own profile table.
    #[test]
    fn bike_changes_apply_at_once_outside_events() {
        let mut server = local(0);
        let (conn, rider) = joined(&mut server);
        let r = bike_change_result(&mut server, conn, "gravel");
        assert!(r.ok, "{r:?}");
        let p = server.session().rider_profile(rider).unwrap();
        assert_eq!(
            p,
            ocw_core::RiderProfile::curated(ocw_core::ProfileId::Gravel, p.rider_mass_kg)
        );
        let snap = (0..10)
            .flat_map(|_| server.tick())
            .filter_map(|b| match decode_server(&b) {
                Ok(S::Snapshot(s)) => Some(s),
                _ => None,
            })
            .last()
            .expect("a snapshot");
        let me = snap.riders.iter().find(|u| u.rider_id == rider.0).unwrap();
        assert_eq!(me.bike_profile, "gravel");
        assert_eq!(me.look.unwrap().frame_rgb, 0x43A047);
        // An unknown profile is the default one, never the client's numbers.
        assert!(bike_change_result(&mut server, conn, "rocket").ok);
        assert_eq!(
            server.session().rider_profile(rider).unwrap().bike,
            ocw_core::ProfileId::Road
        );
        // In a race: from the next start.
        let race = EventSpec::Race {
            laps: None,
            length_m: Some(400.0),
        };
        server.handle(conn, &start(race, Some(0)));
        server.tick();
        let r = bike_change_result(&mut server, conn, "tt");
        assert!(r.ok && r.message.contains("next start"), "{r:?}");
        assert_eq!(
            server.session().rider_profile(rider).unwrap().bike,
            ocw_core::ProfileId::Road
        );
        // The event ends: the waiting bike applies.
        server.handle(conn, &event_msg(v1::event_request::Req::Cancel(true)));
        assert_eq!(
            server.session().rider_profile(rider).unwrap().bike,
            ocw_core::ProfileId::TimeTrial
        );
        assert!(bike_change_result(&mut server, conn, "tt").ok);
        assert_eq!(
            server.session().rider_profile(rider).unwrap().bike,
            ocw_core::ProfileId::TimeTrial
        );
    }

    /// Hearts in the snapshots (spec §14 Phase 4, 4E): a running total of
    /// the server's own count; virtual power earns nothing.
    #[test]
    fn snapshots_carry_the_credited_hearts() {
        let mut server = local(0);
        let (conn, rider) = joined(&mut server);
        let (vconn, vrider) = joined(&mut server);
        let sample = |seq: u32, virtual_power: bool| {
            encode(&client_message(C::Sample(v1::SensorSample {
                seq,
                power_w: Some(400.0),
                virtual_power: Some(virtual_power),
                ..Default::default()
            })))
        };
        server.handle(conn, &sample(1, false));
        server.handle(vconn, &sample(1, true));
        // 30 s at 400 W = 12 kJ: one heart.
        let mut last = None;
        for _ in 0..(ocw_core::SIM_TICK_HZ * 30) {
            for b in server.tick() {
                if let Ok(S::Snapshot(s)) = decode_server(&b) {
                    last = Some(s);
                }
            }
        }
        let snap = last.unwrap();
        let hearts = |id: RiderId| {
            snap.riders
                .iter()
                .find(|u| u.rider_id == id.0)
                .unwrap()
                .hearts
        };
        assert!(hearts(rider) >= 1, "{}", hearts(rider));
        assert_eq!(hearts(vrider), 0);
    }

    fn builds_in(msgs: &[Vec<u8>]) -> Vec<ocw_protocol::BuildInfo> {
        msgs.iter()
            .filter_map(|b| match decode_server(b) {
                Ok(S::RiderBuild(r)) => Some(ocw_protocol::build_info(&r)),
                _ => None,
            })
            .collect()
    }

    const UPGRADES: [&str; 7] = [
        "wheels.road-carbon.top",
        "tyres.road.race",
        "groupset.road-electronic.top",
        "cockpit.road.integrated",
        "saddle.road.top",
        "seatpost.carbon",
        "pedals.road.top",
    ];

    /// W3 (spec §14 Phase 4, 4G): a `BikeChange` applies the parts'
    /// physics from the server's own catalogue (within the caps); everyone
    /// gets the build once (`RiderBuild`, reliable), the snapshots only
    /// its hash; a client joining later gets every build; the Hello's
    /// parts are the start build.
    #[test]
    fn bike_change_applies_the_build_and_tells_the_others_once() {
        let mut server = local(0);
        let (a, ra) = joined(&mut server);
        let (_b, rb) = joined(&mut server);
        server.tick();
        let msg = encode(&client_message(C::BikeChange(v1::BikeChange {
            bike_id: "road".into(),
            bike_profile: "road".into(),
            look: None,
            parts: UPGRADES.map(String::from).to_vec(),
        })));
        let reply = server.handle(a, &msg);
        let S::DevResult(r) = decode_server(&reply[0]).unwrap() else {
            panic!("a result");
        };
        assert!(r.ok && r.message.contains("% flat"), "{r:?}");
        let expected = ocw_sim::build::build(
            ocw_sim::PartTable::shipped(),
            ocw_core::ProfileId::Road,
            &UPGRADES.map(String::from),
        );
        assert_eq!(
            server.session().rider_profile(ra).unwrap(),
            expected.rider_profile(75.0)
        );
        let mut out = Vec::new();
        for _ in 0..10 {
            out.extend(server.tick());
        }
        let builds = builds_in(&out);
        assert_eq!(builds.len(), 1, "once, not with every snapshot");
        assert_eq!(builds[0].rider, ra);
        assert_eq!(builds[0].parts, expected.parts);
        assert_eq!(builds[0].hash, expected.hash());
        assert!((builds[0].effect.flat - expected.flat_gain).abs() < 1e-4);
        let snap = out
            .iter()
            .rev()
            .find_map(|b| match decode_server(b) {
                Ok(S::Snapshot(s)) => Some(s),
                _ => None,
            })
            .unwrap();
        let hash = |id: RiderId| {
            snap.riders
                .iter()
                .find(|u| u.rider_id == id.0)
                .unwrap()
                .build_hash
        };
        assert_eq!((hash(ra), hash(rb)), (expected.hash(), 0));
        // A client joining later gets the builds with its welcome.
        let c = server.connect();
        let replies = server.handle(c, &hello(PROTOCOL_VERSION));
        let builds = builds_in(&replies);
        assert_eq!(builds.len(), 1);
        assert_eq!(builds[0].rider, ra);
        // The Hello's parts: a joining rider starts on its build.
        let d = server.connect();
        let replies = server.handle(
            d,
            &encode(&client_message(C::Hello(v1::Hello {
                protocol_version: PROTOCOL_VERSION,
                display_name: "Fitted".into(),
                bike_profile: Some("tt".into()),
                parts: vec!["wheels.tt.disc".into(), "frame.mtb.upper".into()],
                ..Default::default()
            }))),
        );
        let S::Welcome(w) = decode_server(&replies[0]).unwrap() else {
            panic!("welcome");
        };
        let rd = RiderId(w.rider_id);
        let b = server.session().rider_build(rd).expect("a build");
        assert_eq!(
            b.parts,
            vec!["wheels.tt.disc".to_string()],
            "the MTB frame does not fit"
        );
        let tt = ocw_core::RiderProfile::curated(ocw_core::ProfileId::TimeTrial, 75.0);
        assert!(server.session().rider_profile(rd).unwrap().cda_m2 < tt.cda_m2);
        let builds = builds_in(&server.tick());
        assert!(builds.iter().any(|b| b.rider == rd));
    }

    /// The sprint of a stock and an upgraded bot (spec §14 Phase 4, W3):
    /// in *Open* the upgraded bot wins by the build's few per cent, in
    /// *Profile* (the default for races) both ride exactly the same.
    fn sprint(physics: Option<ocw_sim::PhysicsMode>) -> (f64, f64, String) {
        let mut server = local(2);
        let (conn, _) = joined(&mut server);
        let bots = server.session().bots();
        let bike = |id: RiderId, parts: &[&str], name: &str, lane: f32| {
            encode(&client_message(C::Dev(v1::DevCommand {
                cmd: Some(v1::dev_command::Cmd::BotBike(v1::BotBike {
                    rider_id: id.0,
                    bike_profile: "road".into(),
                    parts: parts.iter().map(|s| s.to_string()).collect(),
                    power_w: Some(300.0),
                    lane_m: Some(lane),
                    name: name.into(),
                })),
            })))
        };
        let mut said = String::new();
        for (id, parts, name, lane) in [
            (bots[0], &[][..], "Stock", -2.4),
            (bots[1], &UPGRADES[..], "Upgraded", 2.4),
        ] {
            let reply = server.handle(conn, &bike(id, parts, name, lane));
            let S::DevResult(r) = decode_server(&reply[0]).unwrap() else {
                panic!("a result");
            };
            assert!(r.ok, "{r:?}");
            said = r.message;
        }
        let start = v1::EventStart {
            physics: physics.map(ocw_protocol::physics_mode_to_wire),
            ..ocw_protocol::event_start(
                &EventSpec::Race {
                    laps: None,
                    length_m: Some(1000.0),
                },
                None,
            )
        };
        let reply = server.handle(
            conn,
            &encode(&client_message(C::Event(v1::EventRequest {
                req: Some(v1::event_request::Req::Start(start)),
            }))),
        );
        let S::DevResult(r) = decode_server(&reply[0]).unwrap() else {
            panic!("a result");
        };
        assert!(r.ok, "{r:?}");
        for _ in 0..(ocw_core::SIM_TICK_HZ * 200) {
            server.tick();
        }
        let ev = server.session().event().unwrap();
        let t = |id| ev.entrant(id).unwrap().finish_s.expect("finished");
        assert_eq!(ev.physics, physics.unwrap_or(ocw_sim::PhysicsMode::Profile));
        (t(bots[0]), t(bots[1]), said + " / " + &r.message)
    }

    #[test]
    fn an_upgraded_bot_wins_in_open_mode_and_ties_in_profile_mode() {
        let (stock, upgraded, said) = sprint(Some(ocw_sim::PhysicsMode::Open));
        assert!(said.contains("Open mode"), "{said}");
        let gain = stock / upgraded - 1.0;
        assert!(
            gain > 0.004 && gain < ocw_sim::build::FLAT_CAP + 0.004,
            "{stock} {upgraded}"
        );
        for physics in [None, Some(ocw_sim::PhysicsMode::Profile)] {
            let (stock, upgraded, said) = sprint(physics);
            assert_eq!(stock, upgraded, "{physics:?}");
            assert!(said.contains("Profile mode"), "{said}");
        }
        // The dev command is for bots on local servers only.
        let mut server = local(1);
        let (conn, me) = joined(&mut server);
        let reply = server.handle(
            conn,
            &encode(&client_message(C::Dev(v1::DevCommand {
                cmd: Some(v1::dev_command::Cmd::BotBike(v1::BotBike {
                    rider_id: me.0,
                    ..Default::default()
                })),
            }))),
        );
        let S::DevResult(r) = decode_server(&reply[0]).unwrap() else {
            panic!("a result");
        };
        assert!(!r.ok, "{r:?}");
    }
}
