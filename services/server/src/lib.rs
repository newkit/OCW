//! Game server logic (spec §6.1).
//!
//! [`GameServer`] is transport-independent: it consumes and produces encoded
//! protocol messages. The same type runs
//! - inside the desktop app as the **local server** ([`local::LocalServer`]), and
//! - as the headless **remote server** binary (`ocw-server`), reachable over
//!   QUIC with [`net`] (`--listen`, spec §6.2).

pub mod local;
pub mod net;
pub mod stats;
pub mod voice;

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
    /// The real UTC instant (Unix ms) at the session's start; `None`: the
    /// system clock when the server is created (tests fix it).
    pub real_epoch_ms: Option<i64>,
}

pub use ocw_protocol::ClockAccess;

/// The system's real UTC instant (Unix ms).
pub fn unix_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
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
        let mut session = match course.into() {
            Course::Route(route) => Session::new(route).with_start(config.start_m),
            Course::FreeRide { net, start } => Session::free_ride(net, start),
            Course::RouteRide { net, plan } => Session::route_ride(net, plan),
        };
        for i in 0..config.bots {
            add_numbered_bot(&mut session, i);
        }
        let clock = ocw_sky::WorldClock::new(config.real_epoch_ms.unwrap_or_else(unix_now_ms));
        Self {
            clock,
            config,
            session,
            riders: BTreeMap::new(),
            next_connection: 1,
            event_dirty: false,
            names: BTreeMap::new(),
            voice_issued: BTreeMap::new(),
            joined: Vec::new(),
            left: Vec::new(),
        }
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    /// Session time (s): the simulation ticks so far.
    pub fn session_time_s(&self) -> f64 {
        self.session.tick() as f64 / f64::from(SIM_TICK_HZ)
    }

    /// The world clock (spec §14 Phase 9).
    pub fn clock(&self) -> &ocw_sky::WorldClock {
        &self.clock
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
        let allowed = match self.config.clock_access {
            ClockAccess::Anyone => true,
            ClockAccess::Host => self.host() == Some(rider),
            ClockAccess::Nobody => false,
        };
        if !allowed {
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
                None
            }
            (C::Dev(cmd), Some(rider)) => Some(self.dev_command(rider, cmd)),
            (C::Event(req), Some(_)) => Some(self.event_request(req)),
            (C::Clock(req), Some(rider)) => Some(self.clock_request(rider, &req)),
            (C::Voice(req), Some(rider)) => Some(S::Voice(self.voice_grant(conn, rider, &req))),
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
        })
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
            None => result(false, "empty dev command".into()),
        }
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
                match self.session.start_event(spec) {
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
                                "{what}, {} rider{}: start in {:.0} s",
                                ev.entrants.len(),
                                if ev.entrants.len() == 1 { "" } else { "s" },
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
        ocw_protocol::event_state(
            self.session.event(),
            &self.session.event_standings(),
            self.session.tick(),
        )
    }

    /// Advances one physics tick. Returns the encoded messages to broadcast
    /// to every connection: free-ride path changes (reliable, at once) and
    /// a snapshot when one is due (every 5th tick at 50/10 Hz).
    pub fn tick(&mut self) -> Vec<Vec<u8>> {
        self.session.step();
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
        let snapshot = v1::Snapshot {
            server_tick: self.session.tick(),
            riders: self
                .session
                .snapshot()
                .iter()
                .map(ocw_protocol::quantize)
                .collect(),
            human_riders: self.riders.len() as u32,
            clock: Some(self.clock_message()),
        };
        out.push(encode(&server_message(S::Snapshot(snapshot))));
        out
    }

    pub fn connections(&self) -> impl Iterator<Item = ConnectionId> + '_ {
        self.riders.keys().copied()
    }
}

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
}
