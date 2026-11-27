//! Game server logic (spec §6.1).
//!
//! [`GameServer`] is transport-independent: it consumes and produces encoded
//! protocol messages. The same type runs
//! - inside the desktop app as the **local server** ([`local::LocalServer`]), and
//! - as the headless **remote server** binary (`ocw-server`), reachable over
//!   QUIC with [`net`] (`--listen`, spec §6.2), and from the browser
//!   over WebTransport (`--webtransport`, spec §14 Phase 10 B3).

// The headphones per viewer (spec §7.1.7 Q17).
pub mod headphones;
// The signed ride credits of a certified server (spec 14 Phase 4, 4O Q19).
#[cfg(not(target_arch = "wasm32"))]
pub mod credits;
// The parts signed-in riders own, from the accounts backend (4O W5d).
#[cfg(not(target_arch = "wasm32"))]
pub mod ownership;
// What a hosting runs beside its game server: the router's port mappings
// and the bundled voice server (spec §7.1.6 V2; native only).
#[cfg(not(target_arch = "wasm32"))]
pub mod hosting;
pub mod local;
// Real weather (spec §14 Phase 9, S7): the session's feed (pure) and the
// HTTP requests (native only, `realweather::http`).
pub mod realweather;
// QUIC (spec §6.2) and world packages from disk: native only; the browser
// app (spec §14 Phase 10) runs the server in its frame loop.
#[cfg(not(target_arch = "wasm32"))]
pub mod net;
// UPnP-IGD port mapping on the router while hosting (spec §7.1.7 Q4).
#[cfg(not(target_arch = "wasm32"))]
pub mod portmap;
pub mod public_stats;
pub mod stats;
// Snapshot timing at a client (interval, jitter, age; spec §14 Phase 10 B3).
pub mod timing;
pub mod voice;
/// Voice blocks: who exchanges no voice with whom (spec §7.1.7 Q9).
pub mod voice_block;
/// Who receives whose voice: range, *Who hears me*, *Voice room*, blocks
/// (spec §7.1.7 Q6, Q8, Q10).
pub mod voice_hearing;
// The wire rules without I/O (frames, datagram rules, the client's link
// state machine): shared with the browser app's WebTransport client.
pub mod wire;
#[cfg(not(target_arch = "wasm32"))]
pub mod worlds;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use ocw_core::name::{self, NameRegistry};
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
    /// `ride.damage` (spec §14 Phase 12A, 12E; Q1: off by default; only
    /// with collisions on), `ocw-server --damage on|off`. Changed with
    /// the collision rules (`collision_access`).
    pub damage: bool,
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
    /// (lat, lon) of the course's local metres (a world package's
    /// origin; a route file's comes with its route): where real weather is
    /// fetched (spec §14 Phase 9, S7). `None`: the route's, else the
    /// default place (Vaduz, like the sun).
    pub geo_origin: Option<(f64, f64)>,
    /// Start in real weather from this data source (`ocw-server --weather
    /// real`; needs a weather fetcher, [`GameServer::set_weather_fetcher`]).
    pub real_weather: Option<ocw_weather::real::Choice>,
    /// The session's *weather affects physics* switch (spec §14 Phase 9,
    /// S6, Q4): on in solo and free ride (the local servers follow the
    /// rider's `weather.physics`, `ocw-server --weather-physics on|off`,
    /// on by default); in events the organiser's rule applies. Changed with
    /// a [`v1::WeatherRequest`] under [`ServerConfig::weather_access`].
    /// `Default` keeps it off (tests: the physics without weather).
    pub weather_physics: bool,
    /// Sign-in (spec §9.1): access tokens in the hello are checked against
    /// the accounts backend's JWKS (signature, expiry, issuer, audience);
    /// a signed-in rider rides under the account's name. `None`: no
    /// sign-in on this server (the local servers): tokens are ignored and
    /// everyone rides anonymously.
    pub auth: Option<std::sync::Arc<ocw_auth::Verifier>>,
    /// Refuse riders without an account (`ocw-server --auth-required`).
    /// By default anonymous riders are allowed (Phase 10 Q5).
    pub auth_required: bool,
    /// The least time between two ride photographer photos the server
    /// lets through (s; spec §2.11). `None`: the photographer's 20 minutes
    /// (no gap at all on a server with dev commands); test runs with a
    /// faster photographer (`ocw-server --photo-time-scale`) shorten it.
    pub photo_gap_s: Option<f64>,
    /// Where the hearts of signed-in riders go when they leave (spec 14
    /// Phase 4, 4O Q19 (a)): a server whose key the project's root key
    /// certified signs a ride credit and sends it to the accounts backend
    /// (`ocw-server --credit-key`). `None`: the apps upload their rides.
    pub hearts_credits: Option<Credits>,
    /// The parts a signed-in rider owns (spec 14 Phase 4, 4O W5d; 4E): with
    /// it, a `BikeChange` (and the hello's parts) count only the parts the
    /// account owns – a part not owned, or any part of an anonymous rider,
    /// counts as the base part, a bike profile the garage has no bike of
    /// as the starter's (`ocw-server --check-parts`). `None`: every part
    /// counts (a local server, the apps' own wallets).
    pub ownership: Option<Ownership>,
}

/// What an account owns, as its garage last said.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Owned {
    /// Catalogue ids of the parts and kit it owns.
    pub parts: std::collections::BTreeSet<String>,
    /// The bike profiles of its bikes (`road`, `gravel`, …).
    pub profiles: std::collections::BTreeSet<String>,
}

/// Where the server learns what a signed-in rider owns (the accounts
/// backend's `GET /workshop` with the rider's own token; tests: a map).
pub trait PartsOwned: Send + Sync {
    /// Fetch `account`'s garage again (a rider joined or changed bikes).
    fn refresh(&self, account: &str, token: &str);
    /// What `account` owns, as last fetched (`None`: not known yet).
    fn owned(&self, account: &str) -> Option<Owned>;
    /// The accounts whose garage arrived since the last call.
    fn take_updated(&self) -> Vec<String>;
}

/// The server's [`PartsOwned`].
#[derive(Clone)]
pub struct Ownership(pub std::sync::Arc<dyn PartsOwned>);

impl std::fmt::Debug for Ownership {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Ownership")
    }
}

/// The profile riders fall back to without a bike of their own (the
/// accounts' starter bike, 4O).
pub const STARTER_PROFILE: ocw_core::ProfileId = ocw_core::ProfileId::Road;

/// `parts` and `bike` as they count with ownership checked: only owned
/// parts, base parts (free) and nothing unknown; a profile the rider has
/// no bike of: the starter's. `owned`: `None` for an anonymous rider or a
/// garage not known yet (only base parts then).
pub fn owned_build(
    bike: ocw_core::ProfileId,
    parts: &[String],
    owned: Option<&Owned>,
) -> (ocw_core::ProfileId, Vec<String>) {
    let table = ocw_sim::PartTable::shipped();
    let bike = match owned {
        Some(o) if o.profiles.contains(bike.id()) => bike,
        _ => STARTER_PROFILE,
    };
    let parts = parts
        .iter()
        .filter(|id| {
            table.get(id).is_some_and(|p| p.base)
                || owned.is_some_and(|o| o.parts.contains(id.as_str()))
        })
        .cloned()
        .collect();
    (bike, parts)
}

/// One signed-in rider's ride on this server, as it ended.
#[derive(Debug, Clone, PartialEq)]
pub struct EndedRide {
    pub account_id: String,
    /// Unique on this server: the join time and the connection.
    pub ride_id: String,
    pub started_unix_s: i64,
    pub ended_unix_s: i64,
    pub distance_m: f64,
    pub work_j: f64,
    pub climb_m: f64,
}

/// Takes the rides that end (it signs and sends them, off the game loop).
pub trait CreditSink: Send + Sync {
    fn ride_ended(&self, ride: EndedRide);
}

/// The server's [`CreditSink`].
#[derive(Clone)]
pub struct Credits(pub std::sync::Arc<dyn CreditSink>);

impl std::fmt::Debug for Credits {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Credits")
    }
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

/// Ticks between two snapshots that carry every rider's damage (2 Hz,
/// spec §14 Phase 12E; a change goes out in the next snapshot).
pub const DAMAGE_TICKS: u64 = 25;

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
    /// The names held in this session (spec §14 Phase 4, 4N Q22: unique
    /// on every server), by connection.
    taken: name::NameBook<ConnectionId>,
    /// When each connection got its last voice token (unix s).
    voice_issued: BTreeMap<ConnectionId, u64>,
    /// The addresses of each network connection (this server's address the
    /// client reached, if known, and the client's), for the voice URL the
    /// rider can reach (spec §7.1.1, §7.1.7 Q5).
    peer_addrs: BTreeMap<ConnectionId, (Option<std::net::IpAddr>, std::net::SocketAddr)>,
    /// Display names of the clients that joined and left, in order (for
    /// the server's statistics, `ocw-server --stats-file`).
    joined: Vec<String>,
    left: Vec<String>,
    /// The connections of test clients (`Hello.bot`, `ocw-bot`): bots on
    /// the public statistics (spec 11.5 *Status page*).
    bot_conns: BTreeSet<ConnectionId>,
    /// Rides started (a connection's first hello), people and bots.
    ride_starts: (u64, u64),
    /// The session's world clock (spec §14 Phase 9), on session time.
    clock: ocw_sky::WorldClock,
    /// The session's weather (spec §14 Phase 9, S4), on session time.
    weather: ocw_weather::SessionWeather,
    /// Session time (s) before the current session: a world switch starts
    /// a new [`Session`] at tick 0, the clock goes on.
    time_base_s: f64,
    /// An allowed world switch, waiting for the embedding to load it.
    pending_world: Option<WorldSwitch>,
    /// The organiser's weather of the current event (spec §14 Phase 9,
    /// S5): its id and weather; locks the weather until the event is over.
    event_weather: Option<(u32, ocw_weather::machine::EventWeather)>,
    /// Real weather (spec §14 Phase 9, S7), while it is on.
    real: Option<realweather::RealSession>,
    /// Performs real weather's requests (`None`: this server has no
    /// weather service – the browser's local server, most tests).
    /// (In a mutex only so the server stays `Sync`; it is never contended.)
    fetcher: Option<std::sync::Mutex<Box<dyn realweather::WeatherFetcher>>>,
    /// (lat, lon) of the course's local metres.
    geo_origin: Option<(f64, f64)>,
    /// The season the leaves on the road lie in (spec 4.6.2 Q6), from the
    /// world clock at the course's place.
    leaf_season: ocw_sim::leaves::LeafSeason,
    /// The scene height (m) of the place real weather was last applied
    /// for: its pressure holds there (spec §14 Phase 9, S6).
    real_pressure_y_m: Option<f64>,
    /// The accounts of signed-in riders (spec §9.1), by connection.
    accounts: BTreeMap<ConnectionId, v1::Account>,
    /// When each connection's rider joined (Unix s; the ride credit).
    joined_at: BTreeMap<ConnectionId, i64>,
    /// With ownership checked: each signed-in connection's token (to fetch
    /// its garage) and the bike it asked for last (applied again when the
    /// garage arrives).
    tokens: BTreeMap<ConnectionId, String>,
    asked: BTreeMap<ConnectionId, (ocw_core::ProfileId, Vec<String>)>,
    /// The riders' status signs (spec §2.6), from their samples (and
    /// `dev.bot_status` for bots); relayed in the snapshots without the
    /// `not_hearing` lists, looks only.
    statuses: BTreeMap<RiderId, v1::RiderStatus>,
    /// The headphones each connection's rider was last told it sees
    /// (spec §7.1.7 Q17: `NotHearingYou`, worked out per viewer).
    not_hearing_you: BTreeMap<ConnectionId, std::collections::BTreeSet<RiderId>>,
    /// The ride photographer's shots (spec §2.11, v4.38): each rider's
    /// checked shots, and those to relay to everyone with the next tick.
    photos: BTreeMap<RiderId, ocw_world::photo_share::ShotLog>,
    photo_out: Vec<v1::PhotoShot>,
    /// Whom each connection's rider blocked in voice (spec §7.1.7 Q9) and
    /// the set of riders each was last told it is apart from.
    voice_blocks: BTreeMap<ConnectionId, voice_block::Blocks>,
    voice_apart: BTreeMap<ConnectionId, std::collections::BTreeSet<RiderId>>,
    /// Each connection's voice audience (spec §7.1.7 Q8: *Who hears me*,
    /// *Voice room*, the selected riders) and the hearing each was last
    /// told (spec §7.1.7 Q6).
    voice_audience: BTreeMap<ConnectionId, voice_hearing::Audience>,
    voice_hearing: BTreeMap<ConnectionId, voice_hearing::Hearing>,
    /// Messages for one connection only ([`GameServer::take_directed`]).
    directed: Vec<(ConnectionId, Vec<u8>)>,
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
        // The jersey designer's choices (spec §14 Phase 8 T6): a second
        // colour from the palette, every other bot a third, a race number.
        design: ocw_core::KitDesign {
            second: Some(PALETTE[(i * 5 + 7) % n]),
            third: (i % 2 == 1).then(|| PALETTE[(i * 11 + 9) % n]),
            number: (i + 1).min(999) as u16,
            helmet_pattern: i.is_multiple_of(3),
            shorts: (i % 4 == 2).then(|| PALETTE[(i * 3 + 6) % n]),
            shoes: (i % 3 == 1).then(|| PALETTE[11]),
            helmet_finish: if i.is_multiple_of(2) {
                ocw_core::Finish::Gloss
            } else {
                ocw_core::Finish::Matte
            },
            ..Default::default()
        },
        ..Default::default()
    };
    (BIKES[i % BIKES.len()], look)
}

/// Adds the `i`-th server bot ("Bot i+1") with its power, bike and look
/// from [`bot_power_w`] and [`bot_bike`]: the same field for the server's
/// startup bots and an event's bots.
/// The organiser's rules in the event start's answer (spec §14 Phase 12
/// K6): ", riding aids not allowed, neutral service, time limit 300 s".
fn organiser_summary(ev: &ocw_sim::Event) -> String {
    let mut s = String::new();
    if let Some(m) = ev.collisions {
        s.push_str(&format!(", collisions {}", m.label()));
    }
    s.push_str(if ev.aids_allowed() {
        ", riding aids allowed"
    } else {
        ", riding aids not allowed"
    });
    if let Some(c) = ev.chaotic_bots {
        s.push_str(if c {
            ", chaotic bots"
        } else {
            ", no chaotic bots"
        });
    }
    if ev.damage.neutral_service {
        s.push_str(", neutral service");
        if let Some(t) = ev.damage.service_s {
            s.push_str(&format!(" (spares in {t} s)"));
        }
    }
    if ev.damage.dnf_badly_injured {
        s.push_str(", DNF when badly injured");
    }
    if let Some(t) = ev.time_limit_s {
        s.push_str(&format!(", time limit {t:.0} s"));
    }
    s
}

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
        let course = course.into();
        let geo_origin = config.geo_origin.or(match &course {
            Course::Route(r) => r.geo_origin(),
            _ => None,
        });
        let session = new_session(course, &config);
        let epoch = config.real_epoch_ms.unwrap_or_else(unix_now_ms);
        let clock = ocw_sky::WorldClock::new(epoch);
        let seed = config
            .weather_seed
            .unwrap_or_else(|| ocw_weather::lightning::unit(epoch as u64, 1, 2).to_bits());
        let weather = ocw_weather::SessionWeather::with(config.weather.unwrap_or_default(), seed);
        let real = config
            .real_weather
            .map(|c| realweather::RealSession::new(c, 0.0));
        Self {
            clock,
            weather,
            config,
            session,
            riders: BTreeMap::new(),
            statuses: BTreeMap::new(),
            next_connection: 1,
            event_dirty: false,
            names: BTreeMap::new(),
            taken: name::NameBook::new(),
            voice_issued: BTreeMap::new(),
            peer_addrs: BTreeMap::new(),
            joined: Vec::new(),
            left: Vec::new(),
            bot_conns: BTreeSet::new(),
            ride_starts: (0, 0),
            time_base_s: 0.0,
            pending_world: None,
            event_weather: None,
            real,
            fetcher: None,
            geo_origin,
            leaf_season: Default::default(),
            real_pressure_y_m: None,
            accounts: BTreeMap::new(),
            joined_at: BTreeMap::new(),
            tokens: BTreeMap::new(),
            asked: BTreeMap::new(),
            photos: BTreeMap::new(),
            photo_out: Vec::new(),
            voice_blocks: BTreeMap::new(),
            voice_apart: BTreeMap::new(),
            voice_audience: BTreeMap::new(),
            voice_hearing: BTreeMap::new(),
            not_hearing_you: BTreeMap::new(),
            directed: Vec::new(),
        }
    }

    /// Lets this server fetch real weather (spec §14 Phase 9, S7): the
    /// HTTP fetcher on the native servers, recorded answers in tests.
    pub fn set_weather_fetcher(&mut self, fetcher: Box<dyn realweather::WeatherFetcher>) {
        self.fetcher = Some(std::sync::Mutex::new(fetcher));
    }

    /// [`GameServer::set_weather_fetcher`] as a builder.
    pub fn with_weather_fetcher(mut self, fetcher: Box<dyn realweather::WeatherFetcher>) -> Self {
        self.set_weather_fetcher(fetcher);
        self
    }

    /// The course's (lat, lon) origin, after a world switch to a package
    /// (the embedding knows it; spec §14 Phase 9, S7).
    pub fn set_geo_origin(&mut self, origin: Option<(f64, f64)>) {
        self.geo_origin = origin;
    }

    /// The real weather's state, while it is on.
    pub fn real_weather(&self) -> Option<&ocw_weather::real::feed::RealInfo> {
        self.real.as_ref().map(|r| &r.info)
    }

    /// Where the session's real weather is wanted: the host's position on
    /// the globe and its height (spec §14 Phase 9, S7; else the first
    /// rider's, else the origin). Rides without a geographic origin (test
    /// routes) are in Vaduz, as for the sun.
    pub fn weather_place(&self) -> ocw_weather::real::Place {
        let known = self.geo_origin.is_some();
        let origin = self.geo_origin.unwrap_or(ocw_sky::zone::DEFAULT_PLACE);
        let rider = self
            .host()
            .or_else(|| self.session.snapshot().first().map(|r| r.id));
        let (x, y, z) = rider
            .and_then(|id| {
                let s = self.session.rider_state(id)?;
                let p = self.session.track(id).sample(s.distance).position;
                Some((p[0], p[1], p[2]))
            })
            .unwrap_or((0.0, 0.0, 0.0));
        let (lat, lon) = ocw_world::geo::LocalProjection::new(origin.0, origin.1).unproject([x, z]);
        ocw_weather::real::Place {
            lat,
            lon,
            // The test routes' heights are relative: put them at Vaduz.
            alt_m: if known { y as f32 } else { 455.0 + y as f32 },
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
        let course = course.into();
        if let Course::Route(r) = &course {
            self.geo_origin = r.geo_origin().or(self.geo_origin);
        }
        self.session = new_session(course, &self.config);
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
        let mut m = ocw_protocol::session_weather_with(
            &self.weather,
            self.session_time_s(),
            self.weather_access(),
            self.real_weather(),
        );
        m.physics = Some(ocw_protocol::physics_rule_to_wire(&self.physics_rule()));
        m
    }

    /// The weather's physics rule now (spec §14 Phase 9, S6): in force or
    /// not, the session's switch, where the pressure holds.
    pub fn physics_rule(&self) -> ocw_protocol::PhysicsRule {
        ocw_protocol::PhysicsRule {
            on: self.session.weather_physics_on(),
            session: self.session.weather_physics_setting(),
            pressure_y_m: self.pressure_y_m(),
        }
    }

    /// The scene height (m) at which the weather's pressure holds: sea
    /// level for presets and the simulation (scene height 0 in a world
    /// package; the test routes stand at Vaduz's 455 m, as for real
    /// weather), the place of the real weather's last data.
    pub fn pressure_y_m(&self) -> f64 {
        let sea = if self.geo_origin.is_some() {
            0.0
        } else {
            -455.0
        };
        match (self.weather.mode(), self.real_pressure_y_m) {
            (ocw_weather::Mode::Real, Some(y)) => y,
            _ => sea,
        }
    }

    /// The session weather's physics at session time `t` (spec §14 Phase
    /// 9, S6): its state, the gust of the moment, the ground.
    pub fn weather_physics_at(&self, t: f64) -> ocw_weather::WeatherPhysics {
        ocw_weather::WeatherPhysics::at(
            &self.weather.now(t),
            self.weather.seed(),
            t,
            &self.weather.ground(),
            self.pressure_y_m(),
        )
    }

    /// Real weather's step (spec §14 Phase 9, S7): answers, requests, and
    /// the session weather from the data.
    fn real_weather_tick(&mut self, t: f64) {
        if self.real.is_none() || self.fetcher.is_none() {
            return;
        }
        let place = self.weather_place();
        let at_ms = self.clock.now(t);
        let now_ms = self.clock.real(t);
        let (Some(real), Some(Ok(fetcher))) = (
            self.real.as_mut(),
            self.fetcher.as_mut().map(std::sync::Mutex::get_mut),
        ) else {
            return;
        };
        match real.step(t, at_ms, now_ms, Some(place), fetcher.as_mut()) {
            realweather::Apply::Keep => {}
            realweather::Apply::Set {
                state,
                transition_s,
                settle,
            } => {
                self.weather.set_real(t, state, transition_s, settle);
                // The data holds at the place's height (S6: its pressure).
                let offset = if self.geo_origin.is_some() {
                    0.0
                } else {
                    455.0
                };
                self.real_pressure_y_m = Some(f64::from(place.alt_m) - offset);
            }
            realweather::Apply::Simulate => {
                let seed = self.weather.seed().rotate_left(29);
                self.weather.simulate(t, seed);
                tracing::warn!("real weather unavailable: simulated weather meanwhile");
            }
        }
    }

    /// Turns real weather on (spec §14 Phase 9, S7) from `choice`.
    fn start_real_weather(
        &mut self,
        choice: ocw_weather::real::Choice,
        transition_s: f64,
    ) -> String {
        match &mut self.real {
            Some(r) => r.set_choice(choice, transition_s),
            None => self.real = Some(realweather::RealSession::new(choice, transition_s)),
        }
        let p = self.weather_place();
        let cell = ocw_weather::real::Cell::of(&p);
        let (lat, lon) = cell.centre();
        format!(
            "real weather ({}) for {:.2}° {}, {:.2}° {}, {:.0} m: fetching ...",
            choice.name(),
            lat.abs(),
            if lat >= 0.0 { "N" } else { "S" },
            lon.abs(),
            if lon >= 0.0 { "E" } else { "W" },
            cell.alt_m()
        )
    }

    /// The organiser's weather of the running event, while it locks the
    /// weather (spec §14 Phase 9, S5: from the start until the event is
    /// finished or cancelled).
    pub fn event_weather(&self) -> Option<&ocw_weather::machine::EventWeather> {
        let (id, w) = self.event_weather.as_ref()?;
        self.session
            .event()
            .is_some_and(|e| e.id == *id && e.phase != EventPhase::Finished)
            .then_some(w)
    }

    /// Who may change the weather now: nobody while an event's organiser
    /// set it, else the session's rule (`ServerConfig::weather_access`).
    pub fn weather_access(&self) -> ClockAccess {
        if self.event_weather().is_some() {
            ClockAccess::Nobody
        } else {
            self.config.weather_access
        }
    }

    /// A weather request from `rider` (spec §14 Phase 9, S4; the weather
    /// machine of S5 sends the same): checked here, on the server (§6.1,
    /// Q2: only the host of a shared session), like the clock's.
    fn weather_request(&mut self, rider: RiderId, req: &v1::WeatherRequest) -> S {
        let result = |ok: bool, message: String| S::DevResult(v1::DevCommandResult { ok, message });
        if let Some(w) = self.event_weather() {
            return result(
                false,
                format!(
                    "the organizer set the weather of this event ({}); it can be changed when the event is over",
                    w.label()
                ),
            );
        }
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
        // The weather-physics switch (S6, Q4), with or without a weather.
        let physics = c.physics.map(|on| {
            self.session.set_weather_physics(on);
            self.config.weather_physics = on;
            let event = self
                .session
                .event()
                .is_some_and(|e| e.phase != EventPhase::Finished);
            format!(
                "weather physics {}{}",
                if on { "on" } else { "off" },
                if event {
                    " (the event's rule applies until it is over)"
                } else {
                    ""
                }
            )
        });
        if let Some(choice) = c.real {
            if self.fetcher.is_none() {
                return result(
                    false,
                    "real weather needs a game server with a weather service (internet access); this one has none".into(),
                );
            }
            let said = self.start_real_weather(choice, c.transition_s);
            tracing::info!(%rider, weather = %said, "real weather on");
            return result(true, said);
        }
        // Any other weather ends real weather.
        if c.target.is_some() || c.simulate.is_some() {
            self.real = None;
        }
        let mut said: Vec<String> = physics.into_iter().collect();
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

    /// The public statistics' numbers now (spec 11.5 *Status page*):
    /// riders and their speeds, people and bots apart, the rides started
    /// and the riders in voice, and each rider on the road by the name it
    /// rides under (11.5 Q1 (b)) – no accounts, addresses or positions.
    pub fn public_observation(&self) -> public_stats::Observation {
        let client_bots: Vec<RiderId> = self
            .bot_conns
            .iter()
            .filter_map(|c| self.riders.get(c).copied())
            .collect();
        // A client's ride is its connection (it keeps it across a world
        // switch); a server rider's its id, apart from those.
        let conn_of: BTreeMap<RiderId, u64> = self.riders.iter().map(|(c, r)| (*r, c.0)).collect();
        let echo = self.session.echo_rider();
        let mut o = public_stats::Observation {
            rides_humans: self.ride_starts.0,
            rides_bots: self.ride_starts.1,
            voice: self.statuses.values().filter(|s| s.in_voice).count() as u32,
            ..Default::default()
        };
        for (id, speed, server_rider) in self.session.motions() {
            let bot = server_rider || client_bots.contains(&id);
            let key = conn_of
                .get(&id)
                .copied()
                .unwrap_or(1 << 63 | u64::from(id.0));
            let name = self.session.rider_name(id).unwrap_or_default();
            o.add_named(key, name, bot, Some(id) == echo, speed);
        }
        o
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

    /// Tells the server where a network connection came from: `local` is
    /// this server's address the client reached (`None`: unknown), `remote`
    /// the client's. A bundled voice server's grants then name the address
    /// this rider can reach (spec §7.1.1 *Joining needs nothing extra*).
    pub fn set_connection_addresses(
        &mut self,
        conn: ConnectionId,
        local: Option<std::net::IpAddr>,
        remote: std::net::SocketAddr,
    ) {
        self.peer_addrs.insert(conn, (local, remote));
    }

    /// Removes a connection and its rider.
    pub fn disconnect(&mut self, conn: ConnectionId) {
        self.credit_ride(conn);
        self.names.remove(&conn);
        self.taken.release(&conn);
        self.accounts.remove(&conn);
        self.joined_at.remove(&conn);
        self.tokens.remove(&conn);
        self.asked.remove(&conn);
        self.voice_issued.remove(&conn);
        self.peer_addrs.remove(&conn);
        self.bot_conns.remove(&conn);
        self.voice_blocks.remove(&conn);
        self.voice_apart.remove(&conn);
        self.voice_audience.remove(&conn);
        self.voice_hearing.remove(&conn);
        self.not_hearing_you.remove(&conn);
        self.directed.retain(|(c, _)| *c != conn);
        if let Some(rider) = self.riders.remove(&conn) {
            self.statuses.remove(&rider);
            self.photos.remove(&rider);
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
            self.refresh_voice_apart();
        }
    }

    /// Hands a leaving signed-in rider's ride to the credit sink (spec 14
    /// Phase 4, 4O Q19 (a)); a ride that earned nothing is not sent.
    fn credit_ride(&mut self, conn: ConnectionId) {
        let (Some(credits), Some(account), Some(rider)) = (
            &self.config.hearts_credits,
            self.accounts.get(&conn),
            self.riders.get(&conn),
        ) else {
            return;
        };
        let Some((meter, distance_m)) = self.session.ride_totals(*rider) else {
            return;
        };
        if meter.hearts() == 0 {
            return;
        }
        let ended = unix_now_ms() / 1000;
        let started = self.joined_at.get(&conn).copied().unwrap_or(ended);
        credits.0.ride_ended(EndedRide {
            account_id: account.account_id.clone(),
            ride_id: format!("{started}-{}-{}", conn.0, rider.0),
            started_unix_s: started,
            ended_unix_s: ended.max(started),
            distance_m,
            work_j: meter.work_j,
            climb_m: meter.climb_m,
        });
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
                // The status signs (spec §2.6): cosmetic, relayed as sent.
                if let Some(st) = sample.status {
                    self.statuses.insert(rider, sanitize_status(st));
                }
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
            (C::Damage(req), Some(rider)) => {
                let r = if req.repair {
                    self.session.request_repair(rider)
                } else {
                    Err("empty damage request".into())
                };
                let (ok, message) = match r {
                    Ok(m) => (true, m),
                    Err(e) => (false, e),
                };
                Some(S::DevResult(v1::DevCommandResult { ok, message }))
            }
            (C::BikeChange(change), Some(rider)) => Some(self.bike_change(rider, &change)),
            (C::Photo(shot), Some(rider)) => {
                self.photo_shot(rider, &shot);
                None
            }
            (C::VoiceBlocks(b), Some(_)) => {
                let blocks = voice_block::Blocks::from_wire(&b);
                if blocks.is_empty() {
                    self.voice_blocks.remove(&conn);
                } else {
                    self.voice_blocks.insert(conn, blocks);
                }
                self.refresh_voice_apart();
                None
            }
            (C::VoiceAudience(a), Some(_)) => {
                self.voice_audience
                    .insert(conn, voice_hearing::Audience::from_wire(&a));
                self.refresh_voice_hearing();
                None
            }
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
                auth: false,
                name_taken: false,
            })),
        };
        let mut out: Vec<Vec<u8>> = reply
            .map(|m| vec![encode(&server_message(m))])
            .unwrap_or_default();
        // A client that just joined a free ride gets everyone's path.
        let joined = !had_rider && self.riders.contains_key(&conn);
        // A newcomer may be blocked by someone already here (by name or
        // account): both are told (spec §7.1.7 Q9).
        if joined {
            self.refresh_voice_apart();
        }
        if joined && self.session.event().is_some() {
            out.push(encode(&server_message(S::Event(self.event_state()))));
        }
        if joined {
            let builds: Vec<RiderId> = self
                .session
                .snapshot()
                .iter()
                .filter(|r| {
                    r.build_hash != 0
                        || r.look.character != Default::default()
                        || r.look.design != Default::default()
                })
                .map(|r| r.id)
                .collect();
            out.extend(builds.into_iter().map(|id| self.build_message(id)));
        }
        // The photographers at work now (spec §2.11): the newcomer sees them.
        if joined {
            let now = self.game_time_s();
            out.extend(self.photos.values().flat_map(|log| {
                log.active(now)
                    .map(|s| encode(&server_message(S::Photo(ocw_protocol::photo_to_wire(s)))))
            }));
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

    /// The account of a hello (spec §9.1): `Ok(None)` rides anonymously,
    /// `Err` is the reason to refuse it.
    fn account_of(&self, hello: &v1::Hello) -> Result<Option<v1::Account>, String> {
        let Some(verifier) = &self.config.auth else {
            // No sign-in on this server: a token is ignored.
            return Ok(None);
        };
        match hello.access_token.as_deref().filter(|t| !t.is_empty()) {
            None if self.config.auth_required => {
                Err("this server needs a signed-in account (Settings -> Account)".into())
            }
            None => Ok(None),
            Some(token) => match verifier.verify(token, unix_now_ms().div_euclid(1000)) {
                Ok(c) => Ok(Some(v1::Account {
                    display_name: if c.name.trim().is_empty() {
                        hello.display_name.clone()
                    } else {
                        c.name
                    },
                    account_id: c.sub,
                    providers: c.providers,
                })),
                Err(e) => Err(format!("sign-in refused ({e}): sign in again")),
            },
        }
    }

    /// The account of a connection's rider, if signed in (spec §9.1).
    pub fn account(&self, conn: ConnectionId) -> Option<&v1::Account> {
        self.accounts.get(&conn)
    }

    fn hello(&mut self, conn: ConnectionId, hello: v1::Hello) -> S {
        if hello.protocol_version != PROTOCOL_VERSION {
            return S::Reject(v1::Reject {
                reason: format!(
                    "protocol version {} not supported (server speaks {PROTOCOL_VERSION}); please update",
                    hello.protocol_version
                ),
                auth: false,
                name_taken: false,
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
                auth: false,
                name_taken: false,
            });
        }
        // Sign-in (spec §9.1): a signed-in rider rides under the
        // account's name; a bad token is refused, no token is anonymous.
        let account = match self.account_of(&hello) {
            Ok(a) => a,
            Err(reason) => {
                tracing::info!(?conn, %reason, "hello refused");
                return S::Reject(v1::Reject {
                    reason,
                    auth: true,
                    name_taken: false,
                });
            }
        };
        let mut hello = hello;
        if let Some(a) = &account {
            hello.display_name = a.display_name.clone();
        }
        // The name (spec §14 Phase 4, 4N N1, Q8 / Q22): the rules of
        // `ocw_core::name`, unique on this server. A taken name is
        // refused (the rider picks another); an invalid one becomes
        // `Rider NNNN` with a note, so no client can put an invisible or
        // overlong name on everyone's screen.
        let checked = name::check(&hello.display_name);
        if let Ok(n) = &checked
            && self.taken.holder(n).is_some_and(|h| h != conn)
        {
            tracing::info!(?conn, name = %n, "hello refused: name taken");
            return S::Reject(v1::Reject {
                reason: format!(
                    "the name \"{n}\" is taken on this server: choose another name (Settings -> Riders) and join again"
                ),
                auth: false,
                name_taken: true,
            });
        }
        let mut profile = physics_profile(&hello);
        // With ownership checked (4O W5d): the garage is fetched with the
        // rider's token; until it arrives only base parts count.
        let (bike, parts) = self.checked_parts(conn, account.as_ref(), profile.bike, &hello.parts);
        if let (Some(o), Some(a), Some(token)) = (
            &self.config.ownership,
            &account,
            hello.access_token.as_deref().filter(|t| !t.is_empty()),
        ) {
            o.0.refresh(&a.account_id, token);
            self.tokens.insert(conn, token.to_owned());
        }
        if bike != profile.bike {
            profile = ocw_core::RiderProfile::curated(bike, profile.rider_mass_kg);
        }
        hello.parts = parts;
        let rider = self.session.add_rider(hello.display_name.clone(), profile);
        let name_note = match checked {
            Ok(n) => {
                hello.display_name = n;
                None
            }
            Err(e) => {
                let given = name::fallback(rider.0);
                let note = if hello.display_name.trim().is_empty()
                    || name::key(&hello.display_name) == "rider"
                {
                    format!("you ride as {given}: give yourself a name in Settings -> Riders")
                } else {
                    format!("your name is not allowed here ({e}): you ride as {given}")
                };
                tracing::info!(%rider, %given, reason = %e, "hello name replaced");
                hello.display_name = given;
                Some(note)
            }
        };
        self.session.rename(rider, hello.display_name.clone());
        self.taken.store(
            conn,
            name::key(&hello.display_name),
            hello.display_name.clone(),
        );
        // The bike's parts (spec §14 Phase 4, 4G): the server's own values.
        if !hello.parts.is_empty() {
            let build =
                ocw_sim::build::build(ocw_sim::PartTable::shipped(), profile.bike, &hello.parts);
            self.session.set_bike(rider, profile, Some(build));
        }
        // Cosmetic only (§2.5): validated (at most 128 bytes; spec §14
        // Phase 4, 4N), then passed through.
        if let Some(look) = &hello.look {
            let look = ocw_protocol::look_from_client(look);
            self.session.set_look(rider, look);
        }
        // The condition from earlier rides (spec §14 Phase 12 Q6): the
        // client keeps it until the backend does (Phase 5); a rider can
        // only claim to be fresher than they are, as a new rider is.
        if let Some(d) = &hello.damage {
            let (condition, parts) = ocw_protocol::carried_from_wire(d);
            self.session.carry_damage(rider, condition, parts);
        }
        self.riders.insert(conn, rider);
        if let Some(a) = &account {
            self.accounts.insert(conn, a.clone());
        }
        // A new ride unless this connection rode before (a world switch
        // sends everyone through a new hello).
        let bot = hello.bot == Some(true);
        if !self.joined_at.contains_key(&conn) {
            if bot {
                self.ride_starts.1 += 1;
            } else {
                self.ride_starts.0 += 1;
            }
        }
        if bot {
            self.bot_conns.insert(conn);
        }
        self.joined_at.insert(conn, unix_now_ms() / 1000);
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
            dev_commands: self.config.allow_dev_commands,
            collisions: ocw_protocol::collision_mode_to_wire(self.session.collision_mode()),
            collision_access: ocw_protocol::access_to_wire(self.config.collision_access),
            chaotic_bots: self.session.chaotic_bots(),
            weather: Some(self.weather_message()),
            damage: self.session.damage_on(),
            account,
            display_name: hello.display_name.clone(),
            name_note,
            hearts_credited: self.config.hearts_credits.is_some(),
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
        if let Some(on) = req.damage {
            self.session.set_damage(on);
            self.config.damage = on;
            said.push(format!("damage {}", if on { "on" } else { "off" }));
            if on && !self.session.collision_mode().on() {
                said.push("no effect while collisions are off".into());
            }
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
    /// `parts` on `bike` as they count for `conn` (ownership checked when
    /// the server does; the request kept to apply again when the garage
    /// arrives).
    fn checked_parts(
        &mut self,
        conn: ConnectionId,
        account: Option<&v1::Account>,
        bike: ocw_core::ProfileId,
        parts: &[String],
    ) -> (ocw_core::ProfileId, Vec<String>) {
        let Some(o) = &self.config.ownership else {
            return (bike, parts.to_vec());
        };
        let owned = account.and_then(|a| o.0.owned(&a.account_id));
        self.asked.insert(conn, (bike, parts.to_vec()));
        owned_build(bike, parts, owned.as_ref())
    }

    /// Garages that arrived (ownership checked): the bikes their riders
    /// asked for apply again with what they own now.
    fn owned_arrived(&mut self) {
        let Some(o) = self.config.ownership.clone() else {
            return;
        };
        for account in o.0.take_updated() {
            let conns: Vec<ConnectionId> = self
                .accounts
                .iter()
                .filter(|(_, a)| a.account_id == account)
                .map(|(c, _)| *c)
                .collect();
            for conn in conns {
                let (Some((bike, parts)), Some(rider)) = (
                    self.asked.get(&conn).cloned(),
                    self.riders.get(&conn).copied(),
                ) else {
                    continue;
                };
                let (bike, parts) = owned_build(bike, &parts, o.0.owned(&account).as_ref());
                let kg = self
                    .session
                    .base_profile(rider)
                    .map_or(ocw_core::RiderProfile::default().rider_mass_kg, |p| {
                        p.rider_mass_kg
                    });
                let build = ocw_sim::build::build(ocw_sim::PartTable::shipped(), bike, &parts);
                self.session.set_bike(
                    rider,
                    ocw_core::RiderProfile::curated(bike, kg),
                    Some(build),
                );
                tracing::info!(%rider, %account, bike = bike.id(), parts = parts.len(), "garage arrived: bike applied with the parts owned");
            }
        }
    }

    fn bike_change(&mut self, rider: RiderId, change: &v1::BikeChange) -> S {
        let result = |ok: bool, message: String| S::DevResult(v1::DevCommandResult { ok, message });
        let asked = ocw_core::ProfileId::from_wire(Some(&change.bike_profile));
        let conn = self
            .riders
            .iter()
            .find(|(_, r)| **r == rider)
            .map(|(c, _)| *c);
        let (bike, parts) = match conn {
            Some(conn) => {
                let account = self.accounts.get(&conn).cloned();
                let checked = self.checked_parts(conn, account.as_ref(), asked, &change.parts);
                // A part bought since the hello: the garage again.
                if let (Some(o), Some(a), Some(t)) =
                    (&self.config.ownership, &account, self.tokens.get(&conn))
                {
                    o.0.refresh(&a.account_id, t);
                }
                checked
            }
            None => (asked, change.parts.clone()),
        };
        let change = &v1::BikeChange {
            parts,
            bike_profile: bike.id().to_owned(),
            ..change.clone()
        };
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
            self.session
                .set_look(rider, ocw_protocol::look_from_client(look));
        }
        // The bike's condition from the garage (spec §14 Phase 12 Q6, Phase
        // 4 W6): a repair or a swap applies with the bike.
        if applied == Some(ocw_sim::BikeApplied::Now)
            && let Some(d) = &change.damage
        {
            let (condition, parts) = ocw_protocol::carried_from_wire(d);
            self.session
                .set_bike_condition(rider, Some(condition), parts);
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
                        let off_road = self.session.off_road(rider).is_some();
                        v1::GodResult {
                            ok: true,
                            message: match (l.fly_back, off_road) {
                                (true, _) => format!("back on {road}"),
                                // Spec §14 Phase 13A: free riding on the terrain.
                                (false, true) => format!("landed {road}: wild mode on"),
                                (false, false) => format!("landed on {road}"),
                            },
                            active: false,
                            start: None,
                            x_m: at[0],
                            y_m: at[1],
                            z_m: at[2],
                            fly_back: l.fly_back,
                            off_road,
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
                let what = if st.lamp {
                    ocw_sim::session::Stage::Lamp
                } else if st.tree {
                    ocw_sim::session::Stage::Tree
                } else if st.crash {
                    ocw_sim::session::Stage::Crash
                } else {
                    ocw_sim::session::Stage::Bump
                };
                match self.session.stage(rider, what) {
                    Ok(m) => result(true, m),
                    Err(e) => result(false, e),
                }
            }
            Some(v1::dev_command::Cmd::BotStatus(b)) => {
                let st = sanitize_status(b.status.unwrap_or_default());
                let bots: Vec<RiderId> = self
                    .session
                    .bots()
                    .iter()
                    .copied()
                    .filter(|id| b.rider_id == 0 || id.0 == b.rider_id)
                    .collect();
                if bots.is_empty() {
                    return result(false, format!("no bot {}", b.rider_id));
                }
                for id in &bots {
                    self.statuses.insert(*id, st.clone());
                }
                result(true, format!("status of {} bot(s) set", bots.len()))
            }
            Some(v1::dev_command::Cmd::BotBike(b)) => match self.bot_bike(&b) {
                Ok(m) => result(true, m),
                Err(e) => result(false, e),
            },
            Some(v1::dev_command::Cmd::Damage(d)) => match ocw_protocol::damage_stage(&d) {
                Some(what) => match self.session.stage_damage(rider, what) {
                    Ok(m) => result(true, m),
                    Err(e) => result(false, e),
                },
                None => result(false, format!("unknown damage target {:?}", d.target)),
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
        let mut grant = voice::grant(cfg, req, rider, name, self.session.event(), now);
        if grant.error.is_empty()
            && let Some(reach) = &cfg.reach
            && let Some(&(local, remote)) = self.peer_addrs.get(&conn)
            && let Some(url) = voice::reach_url(reach, local, remote)
        {
            grant.url = url;
        }
        if grant.error.is_empty() {
            self.voice_issued.insert(conn, now);
            tracing::info!(%rider, room = %grant.room, "voice token issued");
        } else {
            tracing::debug!(%rider, error = %grant.error, "voice token refused");
        }
        grant
    }

    /// The riders `conn`'s rider exchanges no voice with (spec §7.1.7 Q9):
    /// a block in either direction.
    pub fn voice_apart(&self, conn: ConnectionId) -> std::collections::BTreeSet<RiderId> {
        self.compute_voice_apart().remove(&conn).unwrap_or_default()
    }

    fn compute_voice_apart(&self) -> BTreeMap<ConnectionId, std::collections::BTreeSet<RiderId>> {
        if self.voice_blocks.is_empty() {
            return BTreeMap::new();
        }
        voice_block::apart(&self.voice_block_riders(), &self.voice_blocks)
    }

    /// Who blocked whom among the riders here (spec §7.1.7 Q18, for the
    /// headphones): the blocker's rider → the riders it blocked. Never
    /// sent anywhere.
    fn compute_voice_blocked(&self) -> BTreeMap<RiderId, std::collections::BTreeSet<RiderId>> {
        if self.voice_blocks.is_empty() {
            return BTreeMap::new();
        }
        let riders = self.voice_block_riders();
        voice_block::blocked(&riders, &self.voice_blocks)
            .into_iter()
            .filter_map(|(c, set)| riders.get(&c).map(|w| (w.rider, set)))
            .collect()
    }

    /// The riders here as the block rules see them.
    fn voice_block_riders(&self) -> BTreeMap<ConnectionId, voice_block::Who> {
        self.riders
            .iter()
            .map(|(c, r)| {
                (
                    *c,
                    voice_block::Who {
                        rider: *r,
                        name: self.names.get(c).cloned().unwrap_or_default(),
                        account_id: self.accounts.get(c).map(|a| a.account_id.clone()),
                    },
                )
            })
            .collect()
    }

    /// Tells every rider whose set of riders apart changed (spec §7.1.7
    /// Q9): a [`v1::VoiceApart`] to that rider alone. Riders who never had
    /// one apart get nothing.
    fn refresh_voice_apart(&mut self) {
        let mut now = self.compute_voice_apart();
        let conns: Vec<ConnectionId> = self.riders.keys().copied().collect();
        for conn in conns {
            let set = now.remove(&conn).unwrap_or_default();
            let before = self.voice_apart.get(&conn);
            if before.map_or(set.is_empty(), |b| *b == set) {
                continue;
            }
            let msg = v1::VoiceApart {
                riders: set.iter().map(|r| r.0).collect(),
            };
            self.directed
                .push((conn, encode(&server_message(S::VoiceApart(msg)))));
            self.voice_apart.insert(conn, set);
        }
        self.refresh_voice_hearing();
    }

    /// Whose voice `conn`'s rider may receive and who may receive its own
    /// (spec §7.1.7 Q6), as last told.
    pub fn voice_hearing(&self, conn: ConnectionId) -> voice_hearing::Hearing {
        self.voice_hearing.get(&conn).cloned().unwrap_or_default()
    }

    /// Where a rider is for the hearing range: on its free-ride path, else
    /// along the route.
    fn voice_place(&self, rider: RiderId) -> Option<voice_hearing::Place> {
        let state = self.session.rider_state(rider)?;
        Some(match self.session.path(rider) {
            Some(p) => {
                let at = ocw_world::Track::sample(p, state.distance).position;
                voice_hearing::Place::Free([at[0], at[2]])
            }
            None => voice_hearing::Place::Route(state.distance),
        })
    }

    /// Recomputes who receives whose voice (spec §7.1.7 Q6, Q8): range,
    /// audiences, blocks; tells every rider whose answer changed (a
    /// [`v1::VoiceHearing`] to that rider alone). Only on servers with
    /// voice.
    fn refresh_voice_hearing(&mut self) {
        if self.config.voice.is_none() {
            return;
        }
        let mut members: Vec<voice_hearing::Member<ConnectionId>> = self
            .riders
            .iter()
            .filter_map(|(c, r)| {
                Some(voice_hearing::Member {
                    key: Some(*c),
                    who: voice_block::Who {
                        rider: *r,
                        name: self.names.get(c).cloned().unwrap_or_default(),
                        account_id: self.accounts.get(c).map(|a| a.account_id.clone()),
                    },
                    place: self.voice_place(*r)?,
                    audience: self.voice_audience.get(c).cloned().unwrap_or_default(),
                })
            })
            .collect();
        if let Some(echo) = self.session.echo_rider()
            && let Some(place) = self.voice_place(echo)
        {
            members.push(voice_hearing::Member {
                key: None,
                who: voice_block::Who {
                    rider: echo,
                    name: ECHO_NAME.into(),
                    account_id: None,
                },
                place,
                audience: voice_hearing::Audience::echo(),
            });
        }
        let route = self.session.route();
        let loop_m = (route.is_loop() && !self.session.is_free_ride()).then(|| route.length());
        let apart = self.compute_voice_apart();
        let now = voice_hearing::hearing(&members, &apart, &self.voice_hearing, loop_m);
        for (conn, h) in now {
            let before = self.voice_hearing.get(&conn);
            if before.map_or(h == Default::default(), |b| *b == h) {
                continue;
            }
            self.directed
                .push((conn, encode(&server_message(S::VoiceHearing(h.to_wire())))));
            self.voice_hearing.insert(conn, h);
        }
    }

    /// The riders `conn`'s rider sees headphones on (spec §7.1.7 Q17), as
    /// last told.
    pub fn not_hearing_you(&self, conn: ConnectionId) -> std::collections::BTreeSet<RiderId> {
        self.not_hearing_you.get(&conn).cloned().unwrap_or_default()
    }

    /// Works out for every viewer which riders cannot hear it (spec §7.1.7
    /// Q17, [`headphones::cannot_hear`]) and tells each viewer whose set
    /// changed (a [`v1::NotHearingYou`] to that rider alone). The riders'
    /// lists stay here.
    fn refresh_not_hearing_you(&mut self) {
        let conns: Vec<(ConnectionId, RiderId)> =
            self.riders.iter().map(|(c, r)| (*c, *r)).collect();
        // *I hear: nobody* (spec §7.1.7 Q16), from the riders' audiences.
        let hear_nobody: std::collections::BTreeSet<RiderId> = conns
            .iter()
            .filter(|(c, _)| self.voice_audience.get(c).is_some_and(|a| a.hears_nobody()))
            .map(|(_, r)| *r)
            .collect();
        // Blocks (spec §7.1.7 Q18): the blocker shows headphones to the
        // riders it blocked – only to them, who blocked whom stays here.
        let blocked = self.compute_voice_blocked();
        for (conn, rider) in conns {
            let in_voice = self.statuses.get(&rider).is_some_and(|st| st.in_voice);
            let set =
                headphones::cannot_hear(rider, in_voice, &self.statuses, &hear_nobody, &blocked);
            let before = self.not_hearing_you.get(&conn);
            if before.map_or(set.is_empty(), |b| *b == set) {
                continue;
            }
            let msg = v1::NotHearingYou {
                riders: set.iter().map(|r| r.0).collect(),
            };
            self.directed
                .push((conn, encode(&server_message(S::NotHearingYou(msg)))));
            self.not_hearing_you.insert(conn, set);
        }
    }

    /// Takes the messages for single connections (a voice block's
    /// [`v1::VoiceApart`]): the embedding sends each only to its
    /// connection, after the tick's broadcast.
    pub fn take_directed(&mut self) -> Vec<(ConnectionId, Vec<u8>)> {
        std::mem::take(&mut self.directed)
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
                // The organiser's rules (spec §14 Phase 12 K6): collisions,
                // race mode, weather physics, damage, the riding aids,
                // chaotic bots, the time limit; riders cannot change them
                // until the event is over.
                let rules = ocw_protocol::event_rules(&start);
                let weather = ocw_protocol::event_weather(&start);
                let started = self.session.start_event(spec).map(|e| e.id);
                if let Ok(id) = started {
                    self.session.set_event_rules(&rules);
                    self.set_event_weather(id, weather);
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
                        let mut weather = self
                            .event_weather()
                            .map(|w| format!(", weather: {}", w.label()))
                            .unwrap_or_default();
                        if ev.weather_physics {
                            weather.push_str(", weather physics on");
                        }
                        weather.push_str(&organiser_summary(ev));
                        result(
                            true,
                            format!(
                                "{what}, {} rider{}, {} mode ({}){weather}: start in {:.0} s",
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

    /// Puts the organiser's weather of event `id` in force (spec §14
    /// Phase 9, S5): set at once with its ground, or the simulation with
    /// its seed; `None`: the session's weather stays.
    fn set_event_weather(&mut self, id: u32, w: Option<ocw_weather::machine::EventWeather>) {
        use ocw_weather::machine::EventWeather;
        let t = self.session_time_s();
        match &w {
            Some(EventWeather::Fixed { state, .. }) => {
                self.real = None;
                self.weather.set(t, *state, 0.0, true);
            }
            Some(EventWeather::Simulated { seed }) => {
                self.real = None;
                self.weather.simulate(t, *seed);
            }
            None => {}
        }
        if let Some(w) = &w {
            tracing::info!(event = id, weather = %w.label(), "event weather");
        }
        self.event_weather = w.map(|w| (id, w));
    }

    /// The event state to send now.
    fn event_state(&self) -> v1::EventState {
        let mut e = ocw_protocol::event_state(
            self.session.event(),
            &self.session.event_standings(),
            self.session.tick(),
        );
        if let Some(ev) = self.session.event() {
            e.collisions = ocw_protocol::collision_mode_to_wire(self.session.collision_mode());
            // The rules in force (K6): chaotic bots and damage may come
            // from the session's switches.
            e.chaotic_bots = self.session.chaotic_bots_on();
            if let Some(d) = &mut e.damage {
                d.damage = Some(self.session.damage_on());
            }
            e.weather = self
                .event_weather
                .as_ref()
                .filter(|(id, _)| *id == ev.id)
                .map(|(_, w)| ocw_protocol::event_weather_to_wire(w));
            // Crashes and the condition (spec §14 Phase 12E: the results
            // show them), with damage on.
            let damage = self.session.damage_on();
            for st in &mut e.standings {
                if let Some(d) = self.session.damage(RiderId(st.rider_id)) {
                    st.crashes = d.crashes;
                    if damage {
                        st.rider_pm = Some(ocw_sim::damage::per_mille(d.rider));
                        st.bike_pm = Some(ocw_sim::damage::per_mille(d.bike()));
                    }
                }
            }
        }
        e
    }

    /// Advances one physics tick. Returns the encoded messages to broadcast
    /// to every connection: free-ride path changes (reliable, at once) and
    /// a snapshot when one is due (every 5th tick at 50/10 Hz).
    pub fn tick(&mut self) -> Vec<Vec<u8>> {
        self.owned_arrived();
        // The weather's physics for this tick (spec §14 Phase 9, S6): the
        // session rides it while the switch (or the event's rule) is on.
        let t0 = self.session_time_s();
        let physics = self.weather_physics_at(t0);
        self.session.set_weather(Some(physics));
        // The season of the wet leaves on the road (spec 4.6.2 Q6): only
        // where the world has a leaf map.
        if self.session.has_leaf_map() {
            let place = self.geo_origin.unwrap_or(ocw_sky::zone::DEFAULT_PLACE);
            let day = self.leaf_season.day_at(self.clock.now(t0), place);
            self.session.set_leaf_season(Some(day));
        }
        self.session.step();
        let t = self.session_time_s();
        self.weather.tick(t);
        self.real_weather_tick(t);
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
        // The photographers' shots: reliable, relayed to everyone (spec
        // §2.11; the clients draw those near their rider).
        for shot in std::mem::take(&mut self.photo_out) {
            out.push(encode(&server_message(S::Photo(shot))));
        }
        if self.session.tick().is_multiple_of(u64::from(SIM_TICK_HZ)) {
            let now = self.game_time_s();
            for log in self.photos.values_mut() {
                log.prune(now);
            }
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
        // Voice by range (spec §7.1.7 Q6): with every snapshot, sent only
        // when it changes.
        self.refresh_voice_hearing();
        // The headphones per viewer (spec §7.1.7 Q17).
        self.refresh_not_hearing_you();
        let echo = self.session.echo_rider();
        // The signed-in riders' accounts (spec §9.1), once a second: the
        // stable key of a voice block or volume (§7.1.7 Q9).
        let accounts: BTreeMap<RiderId, &str> =
            if self.session.tick().is_multiple_of(u64::from(SIM_TICK_HZ)) {
                self.riders
                    .iter()
                    .filter_map(|(c, r)| self.accounts.get(c).map(|a| (*r, a.account_id.as_str())))
                    .collect()
            } else {
                BTreeMap::new()
            };
        // Damage (spec §14 Phase 12E): at 2 Hz and on every change.
        let changed = self.session.take_damage_changes();
        let all = self.session.tick().is_multiple_of(DAMAGE_TICKS);
        let snapshot = v1::Snapshot {
            server_tick: self.session.tick(),
            riders: self
                .session
                .snapshot()
                .iter()
                .map(|r| v1::RiderUpdate {
                    echo: Some(r.id) == echo,
                    hearts: self.session.hearts(r.id),
                    damage: (all || changed.contains(&r.id))
                        .then(|| self.session.damage(r.id).map(ocw_protocol::damage_to_wire))
                        .flatten(),
                    // Without the lists of whom a rider does not hear
                    // (spec §7.1.7 Q17).
                    status: self.statuses.get(&r.id).map(headphones::relayed),
                    account_id: accounts
                        .get(&r.id)
                        .map(|a| (*a).to_owned())
                        .unwrap_or_default(),
                    ..ocw_protocol::quantize(r)
                })
                .collect(),
            human_riders: self.riders.len() as u32,
            clock: Some(self.clock_message()),
            collisions: ocw_protocol::collision_mode_to_wire(self.session.collision_mode()),
            chaotic_bots: self.session.chaotic_bots(),
            weather: Some(self.weather_message()),
            damage: self.session.damage_on(),
        };
        out.push(encode(&server_message(S::Snapshot(snapshot))));
        out
    }

    pub fn connections(&self) -> impl Iterator<Item = ConnectionId> + '_ {
        self.riders.keys().copied()
    }

    /// The game time the clients share (s): the session's tick / 50 (the
    /// ride photographer's shots, spec §2.11).
    pub fn game_time_s(&self) -> f64 {
        self.session.tick() as f64 / f64::from(SIM_TICK_HZ)
    }

    /// The photos rider `id`'s photographer took in this session (game
    /// times), as the server accepted them.
    pub fn photos_taken(&self, id: RiderId) -> &[f64] {
        self.photos.get(&id).map_or(&[], |l| l.taken())
    }

    /// A rider's photographer shot (spec §2.11, v4.38): checked (at most 5
    /// photos per ride, 20 minutes apart – not on a server for tests –, the
    /// camera near the rider, plausible times, the place fixed once shown,
    /// not too many updates), then relayed to everyone with the next tick.
    fn photo_shot(&mut self, rider: RiderId, wire: &v1::PhotoShot) {
        if self.session.is_god(rider) {
            return;
        }
        let Some(mut shot) = ocw_protocol::photo_from_wire(wire) else {
            tracing::debug!(%rider, "incomplete photo shot dropped");
            return;
        };
        shot.rider = rider.0;
        let Some(state) = self.session.rider_state(rider) else {
            return;
        };
        let at = self
            .session
            .track(rider)
            .sample(state.distance)
            .offset_position(state.lateral_offset);
        let gap_s = match self.config.photo_gap_s {
            Some(g) => g,
            None if self.config.allow_dev_commands => 0.0,
            None => ocw_world::photo_share::GAP_S,
        };
        let rules = ocw_world::photo_share::Rules {
            gap_s,
            ..Default::default()
        };
        let now = self.game_time_s();
        match self
            .photos
            .entry(rider)
            .or_default()
            .accept(&shot, now, at, &rules)
        {
            Ok(()) => self.photo_out.push(ocw_protocol::photo_to_wire(&shot)),
            Err(e) => tracing::debug!(%rider, shot = shot.shot, error = %e, "photo shot refused"),
        }
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

/// A rider's status as kept (spec §2.6): at most [`MAX_NOT_HEARING`]
/// riders in `not_hearing` and in `silenced` (0 % volume, §7.1.7 Q18),
/// sorted, once each.
pub fn sanitize_status(mut st: v1::RiderStatus) -> v1::RiderStatus {
    for list in [&mut st.not_hearing, &mut st.silenced] {
        list.sort_unstable();
        list.dedup();
        list.truncate(MAX_NOT_HEARING);
    }
    st
}

/// The most riders one rider's `not_hearing` list carries.
pub const MAX_NOT_HEARING: usize = 64;

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
    session.set_weather_physics(config.weather_physics);
    session.set_damage(config.damage);
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
        hello_as(version, world, &tester("Tester"))
    }

    /// `base` for a test's first hello under it, then "base B", "base C",
    /// … (names are unique on a server, spec §14 Phase 4, 4N Q22; every
    /// test runs on its own thread).
    fn tester(base: &str) -> String {
        thread_local! {
            static USED: std::cell::RefCell<BTreeMap<String, u8>> =
                const { std::cell::RefCell::new(BTreeMap::new()) };
        }
        USED.with(|u| {
            let mut u = u.borrow_mut();
            let n = u.entry(base.to_string()).or_insert(0);
            *n += 1;
            if *n == 1 {
                base.to_string()
            } else {
                format!("{base} {}", (b'A' + *n - 1) as char)
            }
        })
    }

    /// A hello under `name`.
    fn hello_as(version: u32, world: Option<v1::WorldPackageId>, name: &str) -> Vec<u8> {
        encode(&client_message(C::Hello(v1::Hello {
            protocol_version: version,
            display_name: name.into(),
            profile: Some(ocw_core::RiderProfile::default().into()),
            world,
            rider_weight_kg: None,
            bike_profile: None,
            look: None,
            parts: Vec::new(),
            damage: None,
            access_token: None,
            bot: None,
        })))
    }

    fn join_with_weight(server: &mut GameServer, weight: Option<f32>) -> RiderId {
        let conn = server.connect();
        let reply = server.handle(
            conn,
            &encode(&client_message(C::Hello(v1::Hello {
                protocol_version: PROTOCOL_VERSION,
                display_name: tester("Weighed"),
                profile: None,
                world: None,
                rider_weight_kg: weight,
                bike_profile: None,
                look: None,
                parts: Vec::new(),
                damage: None,
                access_token: None,
                bot: None,
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

    /// The bundled voice server (spec §7.1.1): each grant names the
    /// address that rider reaches; an external LiveKit keeps its URL.
    #[test]
    fn voice_grant_url_per_connection() {
        let now = Arc::new(std::sync::atomic::AtomicU64::new(1_900_000_000));
        let mut external = voice_server(now.clone());
        let mut bundled = voice_server(now);
        if let Some(v) = &mut bundled.config.voice {
            v.reach = Some(voice::VoiceReach {
                port: 7890,
                public_host: Some("ride.example.org".into()),
                public_url: None,
            });
        }
        let lan: std::net::SocketAddr = "192.168.5.9:40000".parse().unwrap();
        let web: std::net::SocketAddr = "203.0.113.7:40000".parse().unwrap();
        let here: std::net::IpAddr = "192.168.5.2".parse().unwrap();
        for (which, from, url) in [
            (true, lan, "ws://192.168.5.2:7890"),
            (true, web, "ws://ride.example.org:7890"),
            (false, lan, "ws://127.0.0.1:7880"),
        ] {
            let server = if which { &mut bundled } else { &mut external };
            let conn = server.connect();
            server.set_connection_addresses(conn, Some(here), from);
            server.handle(conn, &hello(PROTOCOL_VERSION));
            let g = ask_voice(server, conn, v1::VoiceChannel::Session, "");
            assert_eq!(g.url, url, "{from}");
            server.disconnect(conn);
            assert!(server.peer_addrs.is_empty());
        }
        // No address known (the in-process connection): the configured URL.
        let conn = bundled.connect();
        bundled.handle(conn, &hello(PROTOCOL_VERSION));
        let g = ask_voice(&mut bundled, conn, v1::VoiceChannel::Session, "");
        assert_eq!(g.url, "ws://127.0.0.1:7880");
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
                display_name: tester("Styled"),
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
            status: None,
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

    /// "Tester", or "Tester B", "Tester C", … when taken (names are
    /// unique on a server, spec §14 Phase 4, 4N Q22).
    fn free_tester(server: &GameServer) -> String {
        std::iter::repeat_with(|| tester("Tester"))
            .take(26)
            .find(|n| server.taken.holder(n).is_none())
            .expect("a free name")
    }

    fn join(server: &mut GameServer) -> (ConnectionId, v1::Welcome) {
        let conn = server.connect();
        let name = free_tester(server);
        let reply = server.handle(conn, &hello_as(PROTOCOL_VERSION, None, &name));
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

    fn join_named(server: &mut GameServer, name: &str) -> (ConnectionId, RiderId) {
        let conn = server.connect();
        let reply = server.handle(
            conn,
            &encode(&client_message(C::Hello(v1::Hello {
                protocol_version: PROTOCOL_VERSION,
                display_name: name.into(),
                ..Default::default()
            }))),
        );
        match decode_server(&reply[0]).unwrap() {
            S::Welcome(w) => (conn, RiderId(w.rider_id)),
            other => panic!("expected a welcome, got {other:?}"),
        }
    }

    fn blocks(names: &[&str]) -> Vec<u8> {
        encode(&client_message(C::VoiceBlocks(v1::VoiceBlocks {
            names: names.iter().map(|n| (*n).to_owned()).collect(),
            ..Default::default()
        })))
    }

    /// The directed `VoiceApart` messages by connection (their riders).
    fn aparts(server: &mut GameServer) -> BTreeMap<ConnectionId, Vec<u32>> {
        server
            .take_directed()
            .into_iter()
            .map(|(c, m)| match decode_server(&m).unwrap() {
                S::VoiceApart(a) => (c, a.riders),
                other => panic!("expected a VoiceApart, got {other:?}"),
            })
            .collect()
    }

    /// Voice blocks (spec §7.1.7 Q9) with three riders: A blocks B –
    /// the server tells A and B, and only them, that they exchange no
    /// voice (both directions, without saying who blocked whom); C is
    /// told nothing and stays with both. Unblocking tells both again.
    #[test]
    fn a_block_keeps_both_riders_apart_and_no_one_else() {
        let mut server = GameServer::new(Route::test_loop(), ServerConfig::default());
        let (a, ra) = join_named(&mut server, "Ada Rider");
        let (b, rb) = join_named(&mut server, "Bea Climber");
        let (c, _) = join_named(&mut server, "Cleo Sprinter");
        assert!(server.take_directed().is_empty(), "no blocks, nothing said");
        assert!(server.handle(a, &blocks(&["Bea Climber"])).is_empty());
        let told = aparts(&mut server);
        assert_eq!(told.get(&a), Some(&vec![rb.0]));
        assert_eq!(told.get(&b), Some(&vec![ra.0]), "the blocked rider too");
        assert!(!told.contains_key(&c), "C is not told anything");
        assert_eq!(server.voice_apart(c), Default::default());
        // The same list again: nothing new.
        server.handle(a, &blocks(&["Bea Climber"]));
        assert!(server.take_directed().is_empty());
        // B's message names A, not who blocked whom: the same as when B
        // had blocked A.
        let mut other = GameServer::new(Route::test_loop(), ServerConfig::default());
        let (oa, _) = join_named(&mut other, "Ada Rider");
        let (ob, _) = join_named(&mut other, "Bea Climber");
        other.handle(ob, &blocks(&["Ada Rider"]));
        let o = aparts(&mut other);
        assert_eq!((o[&oa].len(), o[&ob].len()), (1, 1));
        // Unblocked (an empty list): both told, the sets empty.
        server.handle(a, &blocks(&[]));
        let told = aparts(&mut server);
        assert_eq!(told.get(&a), Some(&vec![]));
        assert_eq!(told.get(&b), Some(&vec![]));
        assert_eq!(told.len(), 2);
        // The tick delivers nothing of it to everyone: broadcasts carry no
        // VoiceApart.
        server.handle(a, &blocks(&["Bea Climber"]));
        let broadcast: Vec<Vec<u8>> = (0..10).flat_map(|_| server.tick()).collect();
        assert!(
            broadcast
                .iter()
                .all(|m| !matches!(decode_server(m), Ok(S::VoiceApart(_))))
        );
        assert_eq!(aparts(&mut server).len(), 2);
    }

    /// A rider who joins after the block (by name) is kept apart at once;
    /// when the blocker leaves, the blocked rider is told.
    #[test]
    fn blocks_meet_newcomers_and_end_with_the_blocker() {
        let mut server = GameServer::new(Route::test_loop(), ServerConfig::default());
        let (a, ra) = join_named(&mut server, "Ada Rider");
        let (c, _) = join_named(&mut server, "Cleo Sprinter");
        server.handle(a, &blocks(&["Bea Climber"]));
        assert!(server.take_directed().is_empty(), "Bea is not here");
        let (b, rb) = join_named(&mut server, "Bea Climber");
        let told = aparts(&mut server);
        assert_eq!(told.get(&a), Some(&vec![rb.0]));
        assert_eq!(told.get(&b), Some(&vec![ra.0]));
        assert!(!told.contains_key(&c));
        server.disconnect(a);
        let told = aparts(&mut server);
        assert_eq!(told.get(&b), Some(&vec![]), "nobody apart any more");
        assert_eq!(told.len(), 1);
        // Messages queued for a connection that left are dropped.
        server.handle(b, &blocks(&["Cleo Sprinter"]));
        server.disconnect(c);
        assert!(server.take_directed().iter().all(|(conn, _)| *conn != c));
    }

    /// The protocol's round trip of the two new messages.
    #[test]
    fn voice_block_messages_round_trip() {
        let m = v1::VoiceBlocks {
            account_ids: vec!["acc-1".into()],
            names: vec!["Bea Climber".into()],
        };
        match ocw_protocol::decode_client(&encode(&client_message(C::VoiceBlocks(m.clone()))))
            .unwrap()
        {
            C::VoiceBlocks(back) => assert_eq!(back, m),
            other => panic!("{other:?}"),
        }
        let a = v1::VoiceApart { riders: vec![3, 7] };
        match decode_server(&encode(&server_message(S::VoiceApart(a.clone())))).unwrap() {
            S::VoiceApart(back) => assert_eq!(back, a),
            other => panic!("{other:?}"),
        }
        let u = v1::RiderUpdate {
            rider_id: 2,
            account_id: "acc-2".into(),
            ..Default::default()
        };
        let snap = v1::Snapshot {
            riders: vec![u.clone()],
            ..Default::default()
        };
        match decode_server(&encode(&server_message(S::Snapshot(snap)))).unwrap() {
            S::Snapshot(back) => assert_eq!(back.riders[0], u),
            other => panic!("{other:?}"),
        }
    }

    /// The headphones per viewer (spec §7.1.7 Q17 (b)): the server works
    /// out for each rider which riders cannot hear it and tells only that
    /// rider (`NotHearingYou`); no snapshot to any client carries a list
    /// of whom a rider does not hear. Mutes and *Voice room* (the
    /// clients' `not_hearing`) and *hears nobody* count; *Who hears me*
    /// and the hearing range do not (Q12); a viewer outside the call sees
    /// none; a rider who leaves takes them along.
    #[test]
    fn headphones_are_worked_out_per_viewer_and_no_list_leaves_the_server() {
        let mut server = GameServer::new(
            Route::test_loop(),
            ServerConfig {
                allow_dev_commands: true,
                voice: Some(voice::VoiceConfig::new(
                    "ws://127.0.0.1:7880",
                    "devkey",
                    "devsecret",
                )),
                ..Default::default()
            },
        );
        let (a, ra) = join_named(&mut server, "Ada Rider");
        let (b, rb) = join_named(&mut server, "Bea Climber");
        let (c, rc) = join_named(&mut server, "Cleo Sprinter");
        let send = |server: &mut GameServer, conn, st: v1::RiderStatus| {
            server.handle(
                conn,
                &encode(&client_message(C::Sample(v1::SensorSample {
                    seq: 1,
                    power_w: Some(0.0),
                    status: Some(st),
                    ..Default::default()
                }))),
            );
        };
        let st = |not_hearing: &[RiderId]| v1::RiderStatus {
            in_voice: true,
            not_hearing: not_hearing.iter().map(|r| r.0).collect(),
            ..Default::default()
        };
        // Ticks; every message any client gets is checked: no snapshot
        // carries a list; each NotHearingYou goes to its rider only.
        let run = |server: &mut GameServer| -> BTreeMap<ConnectionId, Vec<u32>> {
            let mut told = BTreeMap::new();
            for _ in 0..TICKS_PER_BROADCAST * 2 {
                let out = server.tick();
                let directed = server.take_directed();
                let all = out
                    .iter()
                    .map(|m| (None, m))
                    .chain(directed.iter().map(|(c, m)| (Some(*c), m)));
                for (to, m) in all {
                    match decode_server(m) {
                        Ok(S::Snapshot(s)) => {
                            for r in &s.riders {
                                let list = r
                                    .status
                                    .as_ref()
                                    .map_or(0, |st| st.not_hearing.len() + st.silenced.len());
                                assert_eq!(list, 0, "a list in a snapshot: {r:?}");
                            }
                        }
                        Ok(S::NotHearingYou(n)) => {
                            let to = to.expect("never broadcast");
                            told.insert(to, n.riders);
                        }
                        _ => {}
                    }
                }
            }
            told
        };
        let seen = |server: &GameServer, conn| -> Vec<u32> {
            server.not_hearing_you(conn).iter().map(|r| r.0).collect()
        };
        for conn in [a, b, c] {
            send(&mut server, conn, st(&[]));
        }
        assert!(
            run(&mut server).is_empty(),
            "nobody sees headphones: nothing said"
        );
        // Mutes: Bea muted Ada for herself – Ada alone sees headphones on Bea.
        send(&mut server, b, st(&[ra]));
        let told = run(&mut server);
        assert_eq!(told.get(&a), Some(&vec![rb.0]));
        assert_eq!(told.len(), 1, "only Ada is told: {told:?}");
        assert!(seen(&server, b).is_empty() && seen(&server, c).is_empty());
        // Voice room: Cleo listens to *Selected riders* (Bea only), so her
        // client does not hear Ada – Ada sees headphones on Bea and Cleo.
        send(&mut server, c, st(&[ra]));
        let told = run(&mut server);
        assert_eq!(told.get(&a), Some(&vec![rb.0, rc.0]));
        assert_eq!(seen(&server, b), Vec::<u32>::new());
        // *Who hears me: No one* (Ada): nobody may hear Ada, but that is not
        // "cannot hear you" – nothing changes.
        server.handle(
            a,
            &encode(&client_message(C::VoiceAudience(v1::VoiceAudience {
                heard_by: v1::VoiceHeardBy::NoOne as i32,
                ..Default::default()
            }))),
        );
        assert!(run(&mut server).is_empty());
        assert!(server.voice_hearing(a).heard_by.is_empty());
        assert_eq!(seen(&server, a), [rb.0, rc.0]);
        // Range (Q20): Cleo rides 1 km ahead – out of hearing for all, yet
        // the headphones still say only what the riders chose.
        server.handle(
            c,
            &encode(&client_message(C::Dev(v1::DevCommand {
                cmd: Some(v1::dev_command::Cmd::TeleportDistanceM(1000.0)),
            }))),
        );
        send(&mut server, c, st(&[]));
        run(&mut server);
        assert!(server.voice_hearing(c).may_hear.is_empty(), "out of range");
        assert_eq!(seen(&server, a), [rb.0], "distance is no reason");
        assert!(seen(&server, c).is_empty());
        // Hears nobody (Cleo's speaker muted): every viewer in the call
        // sees it, at any distance; a viewer outside the call sees none.
        send(
            &mut server,
            c,
            v1::RiderStatus {
                hears_nobody: true,
                ..st(&[])
            },
        );
        send(
            &mut server,
            b,
            v1::RiderStatus {
                in_voice: false,
                ..Default::default()
            },
        );
        let told = run(&mut server);
        assert_eq!(seen(&server, a), [rc.0], "Bea left the call: no reason now");
        assert_eq!(told.get(&b), None, "Bea saw none and sees none");
        assert!(seen(&server, b).is_empty());
        // A rider who leaves takes the headphones along.
        send(&mut server, b, st(&[ra]));
        run(&mut server);
        assert_eq!(seen(&server, a), [rb.0, rc.0]);
        server.disconnect(b);
        let told = run(&mut server);
        assert_eq!(told.get(&a), Some(&vec![rc.0]));
        // *I hear: nobody* (spec §7.1.7 Q16) from Ada's audience alone –
        // her status says nothing: Cleo sees headphones on Ada; Ada gets
        // no voice from the server.
        send(&mut server, c, st(&[]));
        send(&mut server, a, st(&[]));
        server.handle(
            a,
            &encode(&client_message(C::VoiceAudience(v1::VoiceAudience {
                heard_by: v1::VoiceHeardBy::Nearby as i32,
                listen: v1::VoiceListen::Nobody as i32,
                ..Default::default()
            }))),
        );
        let told = run(&mut server);
        assert_eq!(
            told.get(&c),
            Some(&vec![ra.0]),
            "Cleo sees Ada's headphones"
        );
        assert!(seen(&server, a).is_empty(), "Ada sees none on Cleo");
        assert!(
            server.voice_hearing(a).may_hear.is_empty(),
            "Ada hears nobody"
        );
        // Spec §7.1.7 Q18: 0 % volume and a block count too, each for that
        // viewer only. Ada hears everybody again; Dan joins.
        server.handle(
            a,
            &encode(&client_message(C::VoiceAudience(v1::VoiceAudience {
                heard_by: v1::VoiceHeardBy::Nearby as i32,
                ..Default::default()
            }))),
        );
        let (d, rd) = join_named(&mut server, "Dan Tempo");
        send(&mut server, d, st(&[]));
        run(&mut server);
        assert!(seen(&server, c).is_empty() && seen(&server, d).is_empty());
        // Ada sets Cleo to 0 %: Cleo alone sees headphones on Ada.
        send(
            &mut server,
            a,
            v1::RiderStatus {
                silenced: vec![rc.0],
                ..st(&[])
            },
        );
        let told = run(&mut server);
        assert_eq!(told.get(&c), Some(&vec![ra.0]), "0 % volume");
        assert_eq!(told.len(), 1, "only Cleo is told: {told:?}");
        // Dan blocks Cleo by name: Cleo sees headphones on Dan (and still
        // on Ada); Dan sees none on Cleo; Ada none.
        server.handle(
            d,
            &encode(&client_message(C::VoiceBlocks(v1::VoiceBlocks {
                names: vec!["Cleo Sprinter".into()],
                ..Default::default()
            }))),
        );
        let told = run(&mut server);
        assert_eq!(told.get(&c), Some(&vec![ra.0, rd.0]), "blocked by Dan");
        assert!(seen(&server, d).is_empty(), "the blocker sees none");
        assert!(seen(&server, a).is_empty());
        // Unblock and back to 100 %: gone.
        server.handle(
            d,
            &encode(&client_message(C::VoiceBlocks(Default::default()))),
        );
        send(&mut server, a, st(&[]));
        let told = run(&mut server);
        assert_eq!(told.get(&c), Some(&vec![]));
        assert!(seen(&server, c).is_empty());
    }

    /// The status signs (spec §2.6): relayed from the samples into the
    /// snapshots, `dev.bot_status` for bots, gone with the rider.
    #[test]
    fn status_signs_travel_in_the_snapshots() {
        let mut server = GameServer::new(
            Route::test_loop(),
            ServerConfig {
                bots: 2,
                allow_dev_commands: true,
                ..Default::default()
            },
        );
        let (conn, w) = join(&mut server);
        let me = w.rider_id;
        let status = v1::RiderStatus {
            voice_muted: true,
            in_menu: true,
            in_voice: true,
            not_hearing: vec![9, 3, 9],
            ..Default::default()
        };
        server.handle(
            conn,
            &encode(&client_message(C::Sample(v1::SensorSample {
                seq: 1,
                power_w: Some(100.0),
                status: Some(status),
                ..Default::default()
            }))),
        );
        let s = next_snapshot(&mut server);
        let mine = s.riders.iter().find(|r| r.rider_id == me).unwrap();
        let st = mine.status.as_ref().expect("relayed");
        assert!(st.voice_muted && st.in_menu && !st.in_vr);
        // The list of whom the rider does not hear stays here (spec
        // §7.1.7 Q17), kept sorted, once each.
        assert!(st.not_hearing.is_empty(), "never relayed: {st:?}");
        assert_eq!(server.statuses[&RiderId(me)].not_hearing, vec![3, 9]);
        assert!(
            s.riders
                .iter()
                .filter(|r| r.rider_id != me)
                .all(|r| r.status.is_none())
        );
        // A bot in VR (scenes and tests).
        let bot = server.session().bots()[0];
        let reply = server.handle(
            conn,
            &encode(&client_message(C::Dev(v1::DevCommand {
                cmd: Some(v1::dev_command::Cmd::BotStatus(v1::BotStatus {
                    rider_id: bot.0,
                    status: Some(v1::RiderStatus {
                        in_vr: true,
                        ..Default::default()
                    }),
                })),
            }))),
        );
        assert!(matches!(decode_server(&reply[0]), Ok(S::DevResult(r)) if r.ok));
        let s = next_snapshot(&mut server);
        let b = s.riders.iter().find(|r| r.rider_id == bot.0).unwrap();
        assert!(b.status.as_ref().is_some_and(|st| st.in_vr));
        // Not a bot: refused.
        let reply = server.handle(
            conn,
            &encode(&client_message(C::Dev(v1::DevCommand {
                cmd: Some(v1::dev_command::Cmd::BotStatus(v1::BotStatus {
                    rider_id: me,
                    status: None,
                })),
            }))),
        );
        assert!(matches!(decode_server(&reply[0]), Ok(S::DevResult(r)) if !r.ok));
        server.disconnect(conn);
        assert!(!server.statuses.contains_key(&RiderId(me)));
        // At most MAX_NOT_HEARING riders.
        let big = sanitize_status(v1::RiderStatus {
            not_hearing: (0..500).collect(),
            ..Default::default()
        });
        assert_eq!(big.not_hearing.len(), MAX_NOT_HEARING);
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

    /// Who may use the weather machine (spec §14 Phase 9, S5, Q2): solo
    /// the rider, a shared ride its host (or anyone, `--weather-control
    /// anyone`), nobody in an organizer's session.
    #[test]
    fn weather_machine_permissions_solo_host_anyone_organizer() {
        use ocw_weather::presets::preset;
        let fog = ocw_protocol::WeatherChange {
            target: Some(preset("fog").unwrap()),
            transition_s: 60.0,
            ..Default::default()
        };
        let mut solo = GameServer::new(Route::test_loop(), crate::local::local_config(2, 0.0));
        let (me, w) = join(&mut solo);
        let w = ocw_protocol::weather_info(&w.weather.unwrap());
        assert_eq!(w.access, ClockAccess::Anyone);
        assert!(ask_weather(&mut solo, me, fog).0, "solo: the rider");
        for (access, guest_may) in [
            (ClockAccess::Host, false),
            (ClockAccess::Anyone, true),
            (ClockAccess::Nobody, false),
        ] {
            let mut shared = weather_server(access);
            let (host, _) = join(&mut shared);
            let (guest, _) = join(&mut shared);
            assert_eq!(
                ask_weather(&mut shared, guest, fog).0,
                guest_may,
                "{access:?}"
            );
            assert_eq!(
                ask_weather(&mut shared, host, fog).0,
                access != ClockAccess::Nobody,
                "{access:?}"
            );
        }
        // The host leaves: the next rider is the host.
        let mut shared = weather_server(ClockAccess::Host);
        let (host, _) = join(&mut shared);
        let (guest, _) = join(&mut shared);
        shared.disconnect(host);
        assert!(ask_weather(&mut shared, guest, fog).0);
    }

    /// The server clamps the weather machine's values again (spec §14
    /// Phase 9, S5: the client's editor validates, the server decides).
    #[test]
    fn the_server_clamps_the_weather_machine() {
        use ocw_weather::presets::preset;
        let mut server = weather_server(ClockAccess::Anyone);
        let (me, _) = join(&mut server);
        let mut wild = preset("storm").unwrap();
        wild.wind_mps = 1e9;
        wild.gust_mps = 2.0;
        wild.temperature_c = f32::NAN;
        wild.cloud_cover = 7.0;
        wild.fog_visibility_m = -50.0;
        wild.precip_mm_h = 1e6;
        let req = ocw_protocol::WeatherChange {
            target: Some(wild),
            transition_s: 1e12,
            ..Default::default()
        };
        assert!(ask_weather(&mut server, me, req).0);
        let t = *server.weather().target();
        assert_eq!((t.wind_mps, t.gust_mps), (90.0, 90.0));
        assert_eq!(t.cloud_cover, 1.0);
        assert_eq!(t.precip_mm_h, 400.0);
        assert_eq!(t.fog_visibility_m, ocw_weather::state::MIN_VISIBILITY_M);
        assert!(t.temperature_c.is_finite());
        // A day at most.
        assert!(server.weather().transition_left_s(server.session_time_s()) <= 86_400.0);
    }

    /// A transition over time (spec §14 Phase 9, S5): clear to a
    /// thunderstorm in one minute, eased, reached after 60 s of session
    /// time.
    #[test]
    fn a_transition_runs_over_session_time() {
        use ocw_weather::presets::preset;
        let mut server = weather_server(ClockAccess::Anyone);
        let (me, _) = join(&mut server);
        let ts = preset("thunderstorm").unwrap();
        let (ok, msg) = ask_weather(
            &mut server,
            me,
            ocw_protocol::WeatherChange {
                target: Some(ts),
                transition_s: 60.0,
                settle_ground: true,
                ..Default::default()
            },
        );
        assert!(ok && msg.contains("thunderstorm in 60 s"), "{msg}");
        // A snapshot every 5 s of session time.
        let mut seen = Vec::new();
        for _ in 0..13 {
            for _ in 0..49 {
                next_snapshot(&mut server);
            }
            let w = ocw_protocol::weather_info(&next_snapshot(&mut server).weather.unwrap());
            seen.push((w.now.cloud_cover, w.target.map(|t| t.1)));
        }
        // Growing cloud, the time left falling, then reached.
        assert!(seen.windows(2).all(|p| p[1].0 >= p[0].0), "{seen:?}");
        let (mid, left) = seen[5];
        assert!(mid > 0.3 && mid < 0.8, "{seen:?}");
        assert!(left.is_some_and(|l| (25.0..35.0).contains(&l)), "{seen:?}");
        assert_eq!(seen[12], (1.0, None));
        assert_eq!(server.weather().now(server.session_time_s()), ts);
    }

    /// The weather's physics rule (spec §14 Phase 9, S6, Q4): in the
    /// welcome and every snapshot; the host switches it, like the weather;
    /// the local servers start with it on, a server's default config off.
    #[test]
    fn weather_physics_travels_and_only_the_host_switches_it() {
        let mut server = GameServer::new(
            Route::test_loop(),
            ServerConfig {
                weather_physics: true,
                ..weather_server(ClockAccess::Host).config.clone()
            },
        );
        let (host, hw) = join(&mut server);
        let (guest, _) = join(&mut server);
        let w = ocw_protocol::weather_info(&hw.weather.unwrap());
        assert!(w.physics.on && w.physics.session);
        // A test route stands at Vaduz: its sea level is 455 m below.
        assert_eq!(w.physics.pressure_y_m, -455.0);
        let off = ocw_protocol::WeatherChange {
            physics: Some(false),
            ..Default::default()
        };
        let (ok, msg) = ask_weather(&mut server, guest, off);
        assert!(!ok && msg.contains("only the host"), "{msg}");
        assert!(server.session().weather_physics_on());
        let (ok, msg) = ask_weather(&mut server, host, off);
        assert!(ok && msg == "weather physics off", "{msg}");
        let snap = next_snapshot(&mut server);
        let w = ocw_protocol::weather_info(&snap.weather.unwrap());
        assert!(!w.physics.on && !w.physics.session);
        assert!(!server.session().weather_physics_on());
        // The defaults: on for the local servers, off in `Default`.
        let solo = GameServer::new(Route::test_loop(), crate::local::local_config(0, 0.0));
        assert!(solo.physics_rule().on);
        assert!(!weather_server(ClockAccess::Host).physics_rule().on);
    }

    /// Bots riding `seconds` in the storm on a server with the weather's
    /// physics `on`: their states every second.
    fn storm_ride(storm: bool, on: bool) -> Vec<Vec<ocw_sim::RiderSnapshot>> {
        let mut server = GameServer::new(
            Route::test_hills(),
            ServerConfig {
                bots: 4,
                weather: storm.then(|| ocw_weather::presets::preset("storm").unwrap()),
                weather_seed: Some(9),
                real_epoch_ms: Some(EPOCH_MS),
                weather_physics: on,
                ..Default::default()
            },
        );
        (0..20 * SIM_TICK_HZ)
            .filter_map(|i| {
                server.tick();
                (i % SIM_TICK_HZ == 0).then(|| server.session().snapshot())
            })
            .collect()
    }

    /// Spec §14 Phase 9 9G: the switch off is the physics of before, bit
    /// for bit, whatever the weather; on, a storm changes the ride.
    #[test]
    fn weather_physics_off_rides_exactly_as_before() {
        let calm = storm_ride(false, false);
        assert_eq!(storm_ride(true, false), calm);
        assert_ne!(storm_ride(true, true), calm);
        assert_eq!(storm_ride(true, true), storm_ride(true, true));
    }

    /// Q4: in an event the organiser decides (off unless enabled), shown in
    /// the event's state; after it the session's switch counts again.
    #[test]
    fn an_events_weather_physics_is_the_organisers() {
        let mut server = GameServer::new(Route::test_loop(), crate::local::local_config(0, 0.0));
        let (me, _) = join(&mut server);
        assert!(server.session().weather_physics_on(), "solo: on");
        let race = |physics: Option<bool>| {
            event_msg(v1::event_request::Req::Start(v1::EventStart {
                weather_physics: physics,
                ..ocw_protocol::event_start(
                    &EventSpec::Race {
                        laps: None,
                        length_m: Some(1000.0),
                    },
                    Some(0),
                )
            }))
        };
        let reply = server.handle(me, &race(None));
        assert!(!reply.is_empty());
        assert!(
            !server.session().weather_physics_on(),
            "events: off by default"
        );
        assert!(!server.physics_rule().on && server.physics_rule().session);
        server.handle(me, &event_msg(v1::event_request::Req::Cancel(true)));
        assert!(server.session().weather_physics_on());
        let reply = server.handle(me, &race(Some(true)));
        let S::DevResult(r) = decode_server(&reply[0]).unwrap() else {
            panic!("a result")
        };
        assert!(
            r.ok && r.message.contains("weather physics on"),
            "{}",
            r.message
        );
        assert!(server.session().weather_physics_on());
        let state = server.event_state();
        assert!(ocw_protocol::event_info(&state).unwrap().weather_physics);
    }

    /// K6 (spec §14 Phase 12): the organiser's rules are put in force for
    /// the event and travel in its state – the aids (allowed by default,
    /// races too, Phase 12 Q8; forbidden by the organiser), chaotic bots over the session's switch, the time limit,
    /// the neutral service's wait, damage in force – and end with it.
    #[test]
    fn the_organisers_rules_are_in_force_for_the_event() {
        let mut server = GameServer::new(Route::test_loop(), crate::local::local_config(0, 0.0));
        let (me, _) = join(&mut server);
        let spec = EventSpec::Race {
            laps: None,
            length_m: Some(1000.0),
        };
        let start = |rules: &ocw_sim::event::Rules| {
            event_msg(v1::event_request::Req::Start(
                ocw_protocol::with_event_rules(ocw_protocol::event_start(&spec, Some(2)), rules),
            ))
        };
        server.handle(me, &start(&Default::default()));
        let info = ocw_protocol::event_info(&server.event_state()).unwrap();
        assert!(info.aids && !info.chaotic_bots && info.time_limit_s.is_none());
        assert!(server.session().aids_allowed(), "races too, by default");
        server.handle(
            me,
            &start(&ocw_sim::event::Rules {
                aids: Some(false),
                ..Default::default()
            }),
        );
        let info = ocw_protocol::event_info(&server.event_state()).unwrap();
        assert!(!info.aids && !server.session().aids_allowed(), "forbidden");
        let rules = ocw_sim::event::Rules {
            collisions: Some(ocw_sim::collide::Mode::Full),
            aids: Some(true),
            chaotic_bots: Some(true),
            time_limit_s: Some(90.0),
            damage: Some(ocw_sim::damage::Rules {
                on: Some(true),
                dnf_badly_injured: true,
                neutral_service: true,
                service_s: Some(25),
            }),
            ..Default::default()
        };
        let reply = server.handle(me, &start(&rules));
        let S::DevResult(r) = decode_server(&reply[0]).unwrap() else {
            panic!("a result")
        };
        assert!(
            r.ok && r.message.contains("riding aids allowed")
                && r.message.contains("chaotic bots")
                && r.message.contains("spares in 25 s")
                && r.message.contains("time limit 90 s"),
            "{}",
            r.message
        );
        assert!(
            !server.session().chaotic_bots(),
            "the session's switch stays"
        );
        assert!(server.session().chaotic_bots_on() && server.session().aids_allowed());
        assert!(server.session().damage_on());
        let info = ocw_protocol::event_info(&server.event_state()).unwrap();
        assert!(info.aids && info.chaotic_bots);
        assert_eq!(info.time_limit_s, Some(90.0));
        assert_eq!(info.damage.service_s, Some(25));
        assert_eq!(info.damage.on, Some(true), "damage in force");
        assert_eq!(server.session().event().unwrap().time_limit_s(1000.0), 90.0);
        // Over: the session's own switches again.
        server.handle(me, &event_msg(v1::event_request::Req::Cancel(true)));
        assert!(!server.session().chaotic_bots_on() && server.session().aids_allowed());
        // A group ride allows the aids by default.
        let group = EventSpec::GroupRide { target_w: None };
        server.handle(
            me,
            &event_msg(v1::event_request::Req::Start(ocw_protocol::event_start(
                &group,
                Some(1),
            ))),
        );
        assert!(
            ocw_protocol::event_info(&server.event_state())
                .unwrap()
                .aids
        );
    }

    fn race_with_weather(w: Option<ocw_weather::machine::EventWeather>) -> Vec<u8> {
        event_msg(v1::event_request::Req::Start(v1::EventStart {
            weather: w.as_ref().map(ocw_protocol::event_weather_to_wire),
            ..ocw_protocol::event_start(
                &EventSpec::Race {
                    laps: None,
                    length_m: Some(1000.0),
                },
                Some(2),
            )
        }))
    }

    /// The organiser sets an event's weather (spec §14 Phase 9, S5, Q4):
    /// in force from the start, shown in the event's info, and nobody can
    /// change it until the event is over.
    #[test]
    fn an_events_weather_is_fixed_by_its_organiser_until_it_is_over() {
        use ocw_weather::machine::EventWeather;
        use ocw_weather::presets::preset;
        let mut server = GameServer::new(Route::test_loop(), crate::local::local_config(0, 0.0));
        let (me, _) = join(&mut server);
        let ts = EventWeather::Fixed {
            name: "thunderstorm".into(),
            state: preset("thunderstorm").unwrap(),
        };
        let reply = server.handle(me, &race_with_weather(Some(ts.clone())));
        let S::DevResult(r) = decode_server(&reply[0]).unwrap() else {
            panic!()
        };
        assert!(r.ok && r.message.contains("weather: thunderstorm"), "{r:?}");
        let t = server.session_time_s();
        assert_eq!(server.weather().now(t), preset("thunderstorm").unwrap());
        assert!(
            server.weather().ground().wetness() > 0.9,
            "settled: wet roads"
        );
        assert_eq!(server.event_weather(), Some(&ts));
        // Locked for everyone, the single player included.
        let snap = next_snapshot(&mut server);
        let w = ocw_protocol::weather_info(&snap.weather.unwrap());
        assert_eq!(w.access, ClockAccess::Nobody);
        let change = ocw_protocol::WeatherChange {
            target: Some(preset("clear").unwrap()),
            ..Default::default()
        };
        let (ok, msg) = ask_weather(&mut server, me, change);
        assert!(
            !ok && msg.contains("organizer") && msg.contains("thunderstorm"),
            "{msg}"
        );
        // The event's info shows it.
        let info = ocw_protocol::event_info(&server.event_state()).unwrap();
        assert_eq!(info.weather.as_deref(), Some("thunderstorm"));
        // Cancelled: the weather is the session's again.
        server.handle(me, &event_msg(v1::event_request::Req::Cancel(true)));
        assert!(server.event_weather().is_none());
        assert_eq!(server.weather_access(), ClockAccess::Anyone);
        assert!(ask_weather(&mut server, me, change).0);
        // An event with the simulation: seeded, locked too.
        let reply = server.handle(
            me,
            &race_with_weather(Some(EventWeather::Simulated { seed: 7 })),
        );
        let S::DevResult(r) = decode_server(&reply[0]).unwrap() else {
            panic!()
        };
        assert!(r.message.contains("simulated (seed 7)"), "{r:?}");
        assert_eq!(server.weather().mode(), ocw_weather::Mode::Simulated);
        let w = ocw_protocol::weather_info(&next_snapshot(&mut server).weather.unwrap());
        assert_eq!((w.sim_seed, w.access), (7, ClockAccess::Nobody));
        // An event without a weather keeps the session's rule.
        server.handle(me, &event_msg(v1::event_request::Req::Cancel(true)));
        server.handle(me, &race_with_weather(None));
        assert!(server.event_weather().is_none());
        assert!(ask_weather(&mut server, me, change).0);
        assert!(server.event_state().weather.is_none());
    }

    /// The event's weather is unlocked once the event is finished.
    #[test]
    fn a_finished_event_unlocks_the_weather() {
        use ocw_weather::machine::EventWeather;
        use ocw_weather::presets::preset;
        // Everything fixed (the session's start instant and the weather's
        // seed came from the system clock before): the test reads no clock.
        let mut server = GameServer::new(
            Route::test_loop(),
            ServerConfig {
                real_epoch_ms: Some(EPOCH_MS),
                weather_seed: Some(5),
                ..crate::local::local_config(0, 0.0)
            },
        );
        let (me, _) = join(&mut server);
        let w = EventWeather::Fixed {
            name: "snow".into(),
            state: preset("snow").unwrap(),
        };
        let r = event_msg(v1::event_request::Req::Start(v1::EventStart {
            weather: Some(ocw_protocol::event_weather_to_wire(&w)),
            ..ocw_protocol::event_start(
                &EventSpec::TimeTrial {
                    laps: None,
                    length_m: Some(ocw_sim::event::MIN_LENGTH_M),
                },
                None,
            )
        }));
        let reply = server.handle(me, &r);
        assert!(
            server.event_weather().is_some(),
            "{:?}",
            decode_server(&reply[0])
        );
        // Ride the 100 m hard: the countdown (5 s) and about 15 s of
        // riding; a minute of session time at most.
        let mut ticks = 0;
        for i in 0..50 * 90 {
            if i % 5 == 0 {
                server.handle(me, &sample(i / 5, 900.0));
            }
            server.tick();
            ticks += 1;
            if server.event_weather().is_none() {
                break;
            }
        }
        assert_eq!(
            server.session.event().map(|e| e.phase),
            Some(EventPhase::Finished),
            "after {ticks} ticks"
        );
        assert!(ticks < 50 * 60, "{ticks} ticks");
        assert!(server.event_weather().is_none());
        assert_eq!(server.weather_access(), ClockAccess::Anyone);
        // The same run again: the same tick (deterministic).
        let again = {
            let mut s = GameServer::new(
                Route::test_loop(),
                ServerConfig {
                    real_epoch_ms: Some(EPOCH_MS),
                    weather_seed: Some(5),
                    ..crate::local::local_config(0, 0.0)
                },
            );
            let (me, _) = join(&mut s);
            s.handle(me, &r);
            let mut n = 0;
            for i in 0..50 * 90 {
                if i % 5 == 0 {
                    s.handle(me, &sample(i / 5, 900.0));
                }
                s.tick();
                n += 1;
                if s.event_weather().is_none() {
                    break;
                }
            }
            n
        };
        assert_eq!(again, ticks);
    }

    const OM_FORECAST: &str =
        include_str!("../../../crates/ocw-weather/tests/data/openmeteo-forecast-vaduz.json");
    const OM_LOTHAR: &str = include_str!(
        "../../../crates/ocw-weather/tests/data/openmeteo-archive-lothar-feldberg.json"
    );
    const METNO: &str =
        include_str!("../../../crates/ocw-weather/tests/data/metno-complete-vaduz.json");
    const SMN_VAD: &str =
        include_str!("../../../crates/ocw-weather/tests/data/meteoswiss-vad-t-now.csv");

    /// When the samples were recorded (14 Nov 2026 11:40 UTC).
    const RECORDED_MS: i64 = 1_794_656_400_000;

    type Log = std::sync::Arc<std::sync::Mutex<Vec<ocw_weather::real::feed::Fetch>>>;

    /// A server whose real weather comes from the recorded answers (or
    /// fails: offline), and the log of its requests.
    fn real_server(online: bool, access: ClockAccess) -> (GameServer, Log) {
        use ocw_weather::real::Provider;
        use ocw_weather::real::feed::Kind;
        let fetcher = realweather::FnFetcher::new(move |f: &ocw_weather::real::feed::Fetch| {
            if !online {
                return Err(format!("{}: offline", f.provider.name()));
            }
            Ok(match (f.provider, f.kind) {
                (Provider::OpenMeteo, Kind::Historic) => OM_LOTHAR,
                (Provider::OpenMeteo, _) => OM_FORECAST,
                (Provider::MetNorway, _) => METNO,
                (Provider::MeteoSwiss, _) => SMN_VAD,
                (Provider::Dwd, _) => return Err("DWD: no sample".into()),
            }
            .to_owned())
        });
        let log = fetcher.log.clone();
        let server = GameServer::new(
            Route::test_loop(),
            ServerConfig {
                real_epoch_ms: Some(RECORDED_MS),
                weather_seed: Some(5),
                weather_access: access,
                clock_access: access,
                ..Default::default()
            },
        )
        .with_weather_fetcher(Box::new(fetcher));
        (server, log)
    }

    fn real(choice: ocw_weather::real::Choice, transition_s: f64) -> ocw_protocol::WeatherChange {
        ocw_protocol::WeatherChange {
            real: Some(choice),
            transition_s,
            ..Default::default()
        }
    }

    /// Real weather (spec §14 Phase 9, S7): the host turns it on, the
    /// server asks MeteoSwiss (Vaduz's station) and MET Norway once for the
    /// cell, sets the weather at once with its ground, tells everyone the
    /// providers and the attribution, and asks again only when the answers
    /// are old.
    #[test]
    fn real_weather_comes_from_the_server_for_the_place() {
        use ocw_weather::real::feed::Status;
        use ocw_weather::real::{Choice, Provider};
        let (mut server, log) = real_server(true, ClockAccess::Host);
        let (host, _) = join(&mut server);
        let (guest, _) = join(&mut server);
        let (ok, msg) = ask_weather(&mut server, guest, real(Choice::Automatic, 0.0));
        assert!(!ok && msg.contains("only the host"), "{msg}");
        let (ok, msg) = ask_weather(&mut server, host, real(Choice::Automatic, 0.0));
        assert!(ok, "{msg}");
        assert!(
            msg.contains("47.15° N, 9.55° E"),
            "the cell, not the rider: {msg}"
        );
        let snap = next_snapshot(&mut server);
        let info = ocw_protocol::weather_info(&snap.weather.unwrap());
        assert_eq!(info.mode, ocw_weather::Mode::Real);
        let r = info.real.unwrap();
        assert_eq!(r.status, Status::Live);
        assert_eq!(r.providers, [Provider::MeteoSwiss, Provider::MetNorway]);
        assert_eq!(r.attribution, "Weather: MeteoSwiss, MET Norway, CC BY 4.0");
        assert_eq!(r.station.as_deref(), Some("Vaduz (VAD)"));
        // The measured 7.3 °C at Vaduz (the test loop lies at Vaduz' height).
        assert!(
            (info.now.temperature_c - 7.3).abs() < 0.3,
            "{}",
            info.now.temperature_c
        );
        assert!(info.immediate, "set at once");
        let asked = log.lock().unwrap().len();
        assert_eq!(asked, 2);
        // A minute later: re-evaluated, nothing asked again.
        for _ in 0..50 * 60 {
            server.tick();
        }
        assert_eq!(log.lock().unwrap().len(), asked, "cached per cell");
        assert_eq!(server.weather().mode(), ocw_weather::Mode::Real);
        // After 10 minutes the station is asked again.
        for _ in 0..50 * 600 {
            server.tick();
        }
        assert!(log.lock().unwrap().len() > asked);
        // A preset ends real weather.
        let (ok, _) = ask_weather(
            &mut server,
            host,
            ocw_protocol::WeatherChange {
                target: ocw_weather::presets::preset("fog"),
                ..Default::default()
            },
        );
        assert!(ok);
        let info = ocw_protocol::weather_info(&next_snapshot(&mut server).weather.unwrap());
        assert!(info.real.is_none());
        assert_eq!(info.mode, ocw_weather::Mode::Fixed);
    }

    #[test]
    fn real_weather_with_a_fixed_provider_and_without_a_weather_service() {
        use ocw_weather::real::{Choice, Provider};
        let (mut server, log) = real_server(true, ClockAccess::Anyone);
        let (me, _) = join(&mut server);
        let (ok, _) = ask_weather(
            &mut server,
            me,
            real(Choice::Only(Provider::OpenMeteo), 30.0),
        );
        assert!(ok);
        let info = ocw_protocol::weather_info(&next_snapshot(&mut server).weather.unwrap());
        let r = info.real.unwrap();
        assert_eq!(r.attribution, "Weather: Open-Meteo, CC BY 4.0");
        assert_eq!(r.choice, Choice::Only(Provider::OpenMeteo));
        assert!(
            !info.immediate && info.target.is_some(),
            "a 30 s transition"
        );
        assert_eq!(log.lock().unwrap()[0].provider, Provider::OpenMeteo);
        // DWD does not cover Vaduz: Automatic with a note.
        let (ok, _) = ask_weather(&mut server, me, real(Choice::Only(Provider::Dwd), 0.0));
        assert!(ok);
        let info = ocw_protocol::weather_info(&next_snapshot(&mut server).weather.unwrap());
        let note = info.real.unwrap().note.unwrap();
        assert!(note.contains("DWD does not cover this place"), "{note}");
        // A server without a weather service refuses.
        let mut plain = GameServer::new(Route::test_loop(), ServerConfig::default());
        let (me, _) = join(&mut plain);
        let (ok, msg) = ask_weather(&mut plain, me, real(Choice::Automatic, 0.0));
        assert!(!ok && msg.contains("weather service"), "{msg}");
    }

    /// Offline: no data at all – the simulation meanwhile, with a note.
    #[test]
    fn real_weather_offline_falls_back_to_the_simulation() {
        use ocw_weather::real::Choice;
        use ocw_weather::real::feed::Status;
        let (mut server, _) = real_server(false, ClockAccess::Anyone);
        let (me, _) = join(&mut server);
        assert!(ask_weather(&mut server, me, real(Choice::Automatic, 0.0)).0);
        // The regional station and MET Norway fail, then Open-Meteo.
        for _ in 0..50 * 3 {
            server.tick();
        }
        let info = ocw_protocol::weather_info(&next_snapshot(&mut server).weather.unwrap());
        assert_eq!(info.mode, ocw_weather::Mode::Simulated);
        let r = info.real.unwrap();
        assert_eq!(r.status, Status::Offline);
        assert!(r.note.unwrap().contains("simulated weather meanwhile"));
        assert!(r.attribution.is_empty());
    }

    /// Ride a day: the time machine to Storm Lothar (26 Dec 1999), the
    /// weather real – ERA5 history at once, the gale.
    #[test]
    fn real_weather_of_a_past_day_comes_from_era5() {
        use ocw_weather::real::Choice;
        use ocw_weather::real::feed::Kind;
        let (mut server, log) = real_server(true, ClockAccess::Anyone);
        let (me, _) = join(&mut server);
        assert!(ask_weather(&mut server, me, real(Choice::Automatic, 0.0)).0);
        next_snapshot(&mut server);
        let lothar = ocw_weather::real::time::utc_ms(1999, 12, 26, 11, 0, 0).unwrap();
        let (ok, msg) = ask_clock(&mut server, me, ocw_sky::ClockChange::Fixed(lothar));
        assert!(ok, "{msg}");
        let info = ocw_protocol::weather_info(&next_snapshot(&mut server).weather.unwrap());
        assert!(log.lock().unwrap().iter().any(|f| f.kind == Kind::Historic));
        let r = info.real.unwrap();
        assert!(r.historic);
        assert_eq!(
            r.attribution,
            "Weather: Open-Meteo (ERA5, Copernicus), CC BY 4.0"
        );
        assert!(info.now.gust_mps > 30.0, "{}", info.now.gust_mps);
        assert!(info.immediate, "a clock jump applies at once");
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

    /// The public statistics (spec 11.5 *Status page*): server bots and
    /// `ocw-bot` clients count as bots, people as people; joins and
    /// leaves move the count; a world switch is no new ride; the report
    /// names the riders on the road by the names they ride under (11.5
    /// Q1 (b)), bots marked, a refused name as the fallback the others see.
    #[test]
    fn public_statistics_count_people_and_bots_and_name_them() {
        let mut server = GameServer::new(
            Route::test_loop(),
            ServerConfig {
                bots: 2,
                ..Default::default()
            },
        );
        let ada = server.connect();
        server.handle(ada, &hello_as(PROTOCOL_VERSION, None, "Ada Rider"));
        let bea = server.connect();
        server.handle(bea, &hello_as(PROTOCOL_VERSION, None, "Bea Climber"));
        let bot = server.connect();
        server.handle(
            bot,
            &encode(&client_message(C::Hello(v1::Hello {
                protocol_version: PROTOCOL_VERSION,
                display_name: "linux-1".into(),
                bot: Some(true),
                ..Default::default()
            }))),
        );
        for _ in 0..100 {
            server.tick();
        }
        let o = server.public_observation();
        assert_eq!((o.humans, o.bots), (2, 3), "{o:?}");
        assert_eq!((o.rides_humans, o.rides_bots), (2, 1));
        assert!(o.bots_mps > 0.0, "the server bots ride: {o:?}");
        server.disconnect(bea);
        let o = server.public_observation();
        assert_eq!((o.humans, o.bots), (1, 3));
        // A world switch: everyone says hello again, no new ride.
        let (conns, _) = server.switch_world(Route::test_hills(), v1::WorldChange::default());
        assert_eq!(conns.len(), 2);
        server.handle(ada, &hello_as(PROTOCOL_VERSION, None, "Ada Rider B"));
        let o = server.public_observation();
        assert_eq!((o.rides_humans, o.rides_bots, o.humans), (2, 1, 1));
        // A hostile name is refused by the name checks: the list has the
        // fallback the other riders see.
        let eve = server.connect();
        server.handle(
            eve,
            &hello_as(PROTOCOL_VERSION, None, "<script>alert(1)</script>"),
        );
        let o = server.public_observation();
        let mut stats = public_stats::PublicStats::new(1_795_694_400);
        stats.observe(1_795_694_400, ocw_core::SIM_DT, &o);
        let report = stats.report(1_795_694_400, "test", &public_stats::WorldNote::default());
        let named: Vec<(&str, bool)> = report
            .riders
            .online
            .iter()
            .map(|r| (r.name.as_str(), r.bot))
            .collect();
        let eve_name = snapshot_name(&server, server.riders[&eve].0);
        assert!(eve_name.starts_with("Rider "), "{eve_name}");
        assert_eq!(
            named,
            [
                ("Ada Rider B", false),
                (eve_name.as_str(), false),
                ("Bot 1", true),
                ("Bot 2", true),
            ],
            "{report:?}"
        );
        let json = serde_json::to_string(&report).unwrap();
        for gone in ["Bea", "<script>", "127.0.0.1"] {
            assert!(!json.contains(gone), "{json}");
        }
    }

    fn welcome_or_reject(server: &mut GameServer, conn: ConnectionId, name: &str) -> S {
        let reply = server.handle(conn, &hello_as(PROTOCOL_VERSION, None, name));
        decode_server(&reply[0]).unwrap()
    }

    fn snapshot_name(server: &GameServer, rider: u32) -> String {
        server
            .session()
            .snapshot()
            .iter()
            .find(|r| r.id.0 == rider)
            .map(|r| r.name.clone())
            .expect("in the session")
    }

    /// Names are unique on a server (spec §14 Phase 4, 4N Q8 / Q22): a
    /// second "Ada Rider" (in any case) is refused and picks another, no
    /// "(2)" suffix; the name is free again once its rider left.
    #[test]
    fn names_are_unique_on_a_server() {
        let mut server = GameServer::new(Route::test_loop(), ServerConfig::default());
        let ada = server.connect();
        let S::Welcome(w) = welcome_or_reject(&mut server, ada, "  Ada   Rider ") else {
            panic!("welcome")
        };
        assert_eq!(w.display_name, "Ada Rider", "normalized");
        assert_eq!(w.name_note, None);
        assert_eq!(snapshot_name(&server, w.rider_id), "Ada Rider");
        let other = server.connect();
        let S::Reject(r) = welcome_or_reject(&mut server, other, "ada rider") else {
            panic!("refused")
        };
        assert!(r.name_taken && !r.auth, "{r:?}");
        assert!(r.reason.contains("taken"), "{}", r.reason);
        assert_eq!(server.session().rider_count(), 1, "no rider added");
        // Another name joins.
        let S::Welcome(b) = welcome_or_reject(&mut server, other, "Bea Climber") else {
            panic!("welcome")
        };
        assert_eq!(b.display_name, "Bea Climber");
        // Once Ada leaves, her name is free.
        server.disconnect(ada);
        let third = server.connect();
        assert!(matches!(
            welcome_or_reject(&mut server, third, "Ada Rider"),
            S::Welcome(_)
        ));
    }

    /// The server checks names too (N1): an invalid name – invisible,
    /// too long, reserved, the default "Rider" – becomes "Rider NNNN"
    /// with a note; bots and the echo keep their reserved names.
    #[test]
    fn invalid_names_become_rider_numbers() {
        let mut server = GameServer::new(Route::test_loop(), ServerConfig::default());
        let mut first = None;
        for (name, note) in [
            ("Ada\u{202E}Rider", "not allowed"),
            (&"x".repeat(500)[..], "not allowed"),
            ("Echo", "not allowed"),
            ("Rider", "give yourself a name"),
            ("", "give yourself a name"),
        ] {
            let conn = server.connect();
            let S::Welcome(w) = welcome_or_reject(&mut server, conn, name) else {
                panic!("welcome for {name:?}")
            };
            let given = format!("Rider {:04}", w.rider_id);
            assert_eq!(w.display_name, given);
            assert!(
                w.name_note.as_deref().is_some_and(|n| n.contains(note)),
                "{name:?}: {:?}",
                w.name_note
            );
            assert_eq!(snapshot_name(&server, w.rider_id), given);
            first.get_or_insert(w.rider_id);
        }
        // A fallback is unique (the rider id) and nobody can take it.
        let conn = server.connect();
        let taken = snapshot_name(&server, first.expect("joined"));
        let S::Welcome(w) = welcome_or_reject(&mut server, conn, &taken) else {
            panic!("welcome")
        };
        assert_ne!(w.display_name, taken);
    }

    /// Looks never change the physics (spec §2.5, §14 Phase 4, 4N N13): a
    /// property test over many random characters – every base body,
    /// shape, age, hair and facial hair – of riders with the same weight
    /// and bike: the same physics profile on the server, whatever the
    /// look says (an obese shape at 70 kg too, 4N Q23).
    #[test]
    fn looks_never_change_the_physics() {
        use ocw_core::character::{Age, HAIR_STYLES};
        use ocw_core::{BaseBody, BodyShape, Character};
        let mut server = GameServer::new(Route::test_loop(), ServerConfig::default());
        let mut seed: u64 = 0x5EED_4A11;
        let mut next = |n: usize| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            ((seed >> 33) % n as u64) as usize
        };
        let mut profiles = Vec::new();
        for i in 0..40 {
            let character = Character {
                height_cm: 150 + next(56) as u16,
                base: BaseBody::ALL[next(3)],
                shape: BodyShape::ALL[next(7)],
                age: Age::ALL[next(6)],
                skin: next(ocw_core::character::SKIN_TONES.len()) as u8,
                hair: next(HAIR_STYLES.len()) as u8,
                moustache: next(6) as u8,
                beard: next(8) as u8,
                greying: next(2) == 0,
                ..Character::default()
            };
            let look = ocw_core::RiderLook {
                character,
                ..Default::default()
            };
            let hello = v1::Hello {
                protocol_version: PROTOCOL_VERSION,
                display_name: format!("Ada Rider {i}"),
                profile: None,
                world: None,
                rider_weight_kg: Some(70.0),
                bike_profile: Some("road-aero".into()),
                look: Some(look.into()),
                parts: Vec::new(),
                damage: None,
                access_token: None,
                bot: None,
            };
            let conn = server.connect();
            let reply = server.handle(conn, &encode(&client_message(C::Hello(hello))));
            let S::Welcome(w) = decode_server(&reply[0]).unwrap() else {
                panic!("welcome")
            };
            let id = RiderId(w.rider_id);
            profiles.push(server.session().base_profile(id).expect("joined"));
        }
        assert!(profiles.windows(2).all(|w| w[0] == w[1]), "{profiles:?}");
        assert_eq!(profiles[0].rider_mass_kg, 70.0);
    }

    /// No renaming during a ride (4N Q15): nothing a client sends after
    /// the hello changes its name – a bike change keeps it.
    #[test]
    fn the_name_stays_for_the_whole_ride() {
        let mut server = GameServer::new(Route::test_loop(), ServerConfig::default());
        let conn = server.connect();
        let S::Welcome(w) = welcome_or_reject(&mut server, conn, "Cleo Sprinter") else {
            panic!("welcome")
        };
        let change = encode(&client_message(C::BikeChange(v1::BikeChange {
            bike_id: "road-aero".into(),
            bike_profile: "road-aero".into(),
            look: Some(v1::RiderLook::default()),
            parts: Vec::new(),
            damage: None,
        })));
        server.handle(conn, &change);
        assert_eq!(snapshot_name(&server, w.rider_id), "Cleo Sprinter");
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
        let name = free_tester(server);
        let reply = server.handle(conn, &hello_as(PROTOCOL_VERSION, None, &name));
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
                &encode(&ocw_protocol::rules_request(Some(on), Some(on), None)),
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
            &encode(&ocw_protocol::rules_request(None, Some(true), None)),
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
    /// Damage (spec §14 Phase 12E, K4): `ride.damage` from the rules
    /// request, the carried condition of the hello, each rider's damage in
    /// the snapshots, the repair request and a workshop repair with the
    /// bike change.
    #[test]
    fn damage_travels_with_the_rules_the_hello_and_the_snapshots() {
        let mut server = GameServer::new(Route::test_loop(), crate::local::local_config(2, 500.0));
        // A rider who brings a buckled wheel and an injury.
        let conn = server.connect();
        let mut parts = [1.0; ocw_sim::damage::PARTS];
        parts[ocw_sim::damage::Part::RearWheel.index()] = 0.4;
        let reply = server.handle(
            conn,
            &encode(&client_message(C::Hello(v1::Hello {
                protocol_version: PROTOCOL_VERSION,
                display_name: tester("Hurt"),
                damage: Some(ocw_protocol::carried_damage(0.55, &parts)),
                ..Default::default()
            }))),
        );
        let S::Welcome(w) = decode_server(&reply[0]).unwrap() else {
            panic!("a welcome");
        };
        assert!(!w.damage, "Q1: off by default");
        let me = RiderId(w.rider_id);
        assert_eq!(server.session().damage(me).unwrap().rider, 0.55);
        // Damage on (the single player may).
        let reply = server.handle(
            conn,
            &encode(&ocw_protocol::rules_request(None, None, Some(true))),
        );
        match decode_server(&reply[0]).unwrap() {
            S::DevResult(r) => assert!(r.ok && r.message.contains("damage on"), "{r:?}"),
            other => panic!("{other:?}"),
        }
        let snap = next_snapshot(&mut server);
        assert!(snap.damage);
        let mine = snap.riders.iter().find(|u| u.rider_id == me.0).unwrap();
        let d = ocw_protocol::rider_damage(mine).expect("damage after a change");
        assert_eq!(d.part(ocw_sim::damage::Part::RearWheel), 0.4);
        // A staged puncture and the repair request.
        let reply = server.handle(
            conn,
            &encode(&ocw_protocol::stage_damage("puncture_rear", 1.0)),
        );
        match decode_server(&reply[0]).unwrap() {
            S::DevResult(r) => assert!(r.ok, "{r:?}"),
            other => panic!("{other:?}"),
        }
        let reply = server.handle(conn, &encode(&ocw_protocol::damage_request(true)));
        match decode_server(&reply[0]).unwrap() {
            S::DevResult(r) => assert!(r.ok && r.message.contains("puncture"), "{r:?}"),
            other => panic!("{other:?}"),
        }
        let snap = next_snapshot(&mut server);
        let mine = snap.riders.iter().find(|u| u.rider_id == me.0).unwrap();
        let d = ocw_protocol::rider_damage(mine).unwrap();
        assert!(d.flat[1] && d.stop.is_some());
        // Not every snapshot carries it: 2 Hz and on change.
        let without = (0..10)
            .map(|_| next_snapshot(&mut server))
            .filter(|s| {
                s.riders
                    .iter()
                    .find(|u| u.rider_id == me.0)
                    .is_some_and(|u| u.damage.is_none())
            })
            .count();
        assert!(without > 0);
        // A repair in the workshop reaches the ride with the bike.
        let mut change = match ocw_protocol::bike_change(
            "road",
            ocw_core::ProfileId::Road,
            Default::default(),
            &[],
        )
        .msg
        {
            Some(v1::client_message::Msg::BikeChange(b)) => b,
            _ => unreachable!(),
        };
        change.damage = Some(ocw_protocol::carried_damage(
            0.55,
            &[1.0; ocw_sim::damage::PARTS],
        ));
        server.handle(conn, &encode(&client_message(C::BikeChange(change))));
        let d = server.session().damage(me).unwrap();
        assert_eq!(d.bike(), 1.0);
        assert!(!d.flat[1]);
        // The organiser's rules travel with the event start and its state.
        let rules = ocw_sim::damage::Rules {
            on: Some(true),
            dnf_badly_injured: true,
            neutral_service: true,
            service_s: None,
        };
        let start = v1::EventStart {
            kind: v1::EventKind::Race as i32,
            damage: Some(ocw_protocol::damage_rules_to_wire(&rules)),
            ..Default::default()
        };
        let reply = server.handle(
            conn,
            &encode(&client_message(C::Event(v1::EventRequest {
                req: Some(v1::event_request::Req::Start(start)),
            }))),
        );
        match decode_server(&reply[0]).unwrap() {
            S::DevResult(r) => assert!(r.ok, "{r:?}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(server.session().event_damage_rules(), rules);
        let state = ocw_protocol::event_info(&server.event_state()).unwrap();
        assert_eq!(state.damage, rules);
    }

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
            damage: None,
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
            damage: None,
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
                display_name: tester("Fitted"),
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

    /// The ride photographer's shots (spec §2.11, v4.38).
    mod photographer {
        use super::*;
        use ocw_world::photo::{CameraPose, Carrier, PoseKind};
        use ocw_world::photo_share::SharedShot;

        fn rider_at(server: &GameServer, id: RiderId) -> [f64; 3] {
            let s = server.session().rider_state(id).unwrap();
            ocw_world::Track::sample(server.session().track(id), s.distance)
                .offset_position(s.lateral_offset)
        }

        fn shot(n: u32, near: [f64; 3], now: f64) -> SharedShot {
            SharedShot {
                rider: 99,
                shot: n,
                pose: CameraPose {
                    kind: PoseKind::Side,
                    carrier: Carrier::Person { crouch: false },
                    feet: [near[0] + 12.0, near[1], near[2]],
                    eye: [near[0] + 12.0, near[1] + 1.6, near[2]],
                    look: [-1.0, 0.0, 0.0],
                },
                seed: 11,
                appear_at: now,
                planned_at: now + 8.0,
                taken_at: None,
                gone_at: None,
                aborted_at: None,
                flight: None,
            }
        }

        fn send(server: &mut GameServer, conn: ConnectionId, s: &SharedShot) {
            let replies = server.handle(conn, &encode(&ocw_protocol::photo_message(s)));
            assert!(replies.is_empty());
        }

        fn relayed(out: &[Vec<u8>]) -> Vec<SharedShot> {
            out.iter()
                .filter_map(|b| match decode_server(b).ok()? {
                    S::Photo(p) => ocw_protocol::photo_from_wire(&p),
                    _ => None,
                })
                .collect()
        }

        /// One rider's shot reaches everyone with its rider set by the
        /// server, unchanged; a client joining later gets the open one.
        #[test]
        fn a_shot_is_relayed_to_everyone() {
            let mut server = GameServer::new(Route::test_loop(), ServerConfig::default());
            let (host, host_id) = joined(&mut server);
            let (_guest, _) = joined(&mut server);
            for _ in 0..10 {
                server.tick();
            }
            let now = server.game_time_s();
            let s = shot(1, rider_at(&server, host_id), now);
            send(&mut server, host, &s);
            let got = relayed(&server.tick());
            assert_eq!(got.len(), 1, "once, to every connection");
            assert_eq!(got[0].rider, host_id.0, "the server's rider id");
            assert_eq!(
                SharedShot {
                    rider: 99,
                    ..got[0].clone()
                },
                s
            );
            let late = server.connect();
            let replies = server.handle(late, &hello(PROTOCOL_VERSION));
            let open = relayed(&replies);
            assert_eq!(open.len(), 1, "the newcomer sees the photographer");
            assert_eq!(open[0].pose, s.pose);
        }

        /// The server's checks: the camera near the rider, at most one photo
        /// in 20 minutes, the place fixed.
        #[test]
        fn far_too_frequent_or_moved_shots_are_not_relayed() {
            let mut server = GameServer::new(Route::test_loop(), ServerConfig::default());
            let (host, host_id) = joined(&mut server);
            for _ in 0..10 {
                server.tick();
            }
            let now = server.game_time_s();
            let here = rider_at(&server, host_id);
            let far = shot(1, [here[0] + 6000.0, here[1], here[2]], now);
            send(&mut server, host, &far);
            assert!(relayed(&server.tick()).is_empty(), "6 km away");
            let mut s = shot(2, here, now);
            send(&mut server, host, &s);
            assert_eq!(relayed(&server.tick()).len(), 1);
            let mut moved = s.clone();
            moved.pose.eye[0] += 50.0;
            moved.pose.feet[0] += 50.0;
            send(&mut server, host, &moved);
            assert!(relayed(&server.tick()).is_empty(), "moved");
            let t = server.game_time_s();
            s.taken_at = Some(t);
            send(&mut server, host, &s);
            assert_eq!(relayed(&server.tick()).len(), 1, "the photo");
            assert_eq!(server.photos_taken(host_id).len(), 1);
            s.gone_at = Some(server.game_time_s());
            send(&mut server, host, &s);
            assert_eq!(relayed(&server.tick()).len(), 1, "gone");
            // The next photo 1 s later: not before 20 minutes.
            for _ in 0..50 {
                server.tick();
            }
            let now = server.game_time_s();
            let next = shot(3, rider_at(&server, host_id), now);
            send(&mut server, host, &next);
            assert!(relayed(&server.tick()).is_empty(), "20 minutes apart");
            // A server for tests (dev commands) leaves the gap out.
            let mut dev = GameServer::new(
                Route::test_loop(),
                ServerConfig {
                    allow_dev_commands: true,
                    ..Default::default()
                },
            );
            let (c, id) = joined(&mut dev);
            dev.tick();
            for n in 1..=7 {
                let now = dev.game_time_s();
                let mut s = shot(n, rider_at(&dev, id), now);
                send(&mut dev, c, &s);
                dev.tick();
                s.taken_at = Some(dev.game_time_s());
                send(&mut dev, c, &s);
                dev.tick();
            }
            assert_eq!(dev.photos_taken(id).len(), 5, "never more than 5");
        }
    }
}

/// Sign-in in the hello (spec §9.1): the game server checks the access
/// token against the backend's keys; anonymous riders stay allowed.
#[cfg(test)]
mod auth_tests {
    use super::*;
    use ocw_auth::jwt::{AccessClaims, Jwk, Jwks, TYP_ACCESS, TYP_ID, key_id, sign};
    use ocw_protocol::{client_message, decode_server};

    const ISS: &str = "https://accounts.example.org";

    fn key() -> ed25519_dalek::SigningKey {
        ed25519_dalek::SigningKey::from_bytes(&[5; 32])
    }

    fn verifier() -> std::sync::Arc<ocw_auth::Verifier> {
        let k = key().verifying_key();
        std::sync::Arc::new(ocw_auth::Verifier::new(
            ISS,
            ocw_auth::DEFAULT_AUDIENCE,
            Jwks {
                keys: vec![Jwk::ed25519(&key_id(&k), &k)],
            },
        ))
    }

    fn token(exp_in_s: i64, tune: impl FnOnce(&mut AccessClaims)) -> String {
        let now = unix_now_ms() / 1000;
        let mut c = AccessClaims {
            iss: ISS.into(),
            sub: "acc_42".into(),
            aud: ocw_auth::DEFAULT_AUDIENCE.into(),
            exp: now + exp_in_s,
            iat: now - 1,
            jti: "j".into(),
            client_id: ocw_auth::CLIENT_DESKTOP.into(),
            scope: "openid".into(),
            name: "Ada Rider".into(),
            auth_time: now,
            providers: vec!["google".into(), "strava".into()],
        };
        tune(&mut c);
        let k = key();
        sign(&c, &k, &key_id(&k.verifying_key()), TYP_ACCESS)
    }

    fn join(server: &mut GameServer, token: Option<String>) -> (ConnectionId, ServerMessageKind) {
        let conn = server.connect();
        let reply = server.handle(
            conn,
            &encode(&client_message(C::Hello(v1::Hello {
                protocol_version: PROTOCOL_VERSION,
                display_name: "Anonymous rider".into(),
                access_token: token,
                ..Default::default()
            }))),
        );
        let kind = match decode_server(&reply[0]).unwrap() {
            S::Welcome(w) => ServerMessageKind::Welcome(Box::new(w)),
            S::Reject(r) => ServerMessageKind::Reject(r),
            m => panic!("{m:?}"),
        };
        (conn, kind)
    }

    /// Accounts (spec §7.1.7 Q9): the snapshots carry a signed-in rider's
    /// account once a second, and a block by account keeps that rider
    /// apart whatever their name.
    #[test]
    fn accounts_in_snapshots_and_blocks_by_account() {
        let mut s = server(false);
        let (signed, _) = join(&mut s, Some(token(600, |_| {})));
        let (anon, _) = join(&mut s, None);
        let snaps: Vec<v1::Snapshot> = (0..u64::from(SIM_TICK_HZ) * 2)
            .flat_map(|_| s.tick())
            .filter_map(|b| match decode_server(&b) {
                Ok(S::Snapshot(x)) => Some(x),
                _ => None,
            })
            .collect();
        let with: Vec<&v1::Snapshot> = snaps
            .iter()
            .filter(|x| x.riders.iter().any(|r| r.account_id == "acc_42"))
            .collect();
        assert_eq!(with.len(), 2, "once a second, of 20 snapshots");
        assert!(
            with.iter()
                .all(|x| x.server_tick.is_multiple_of(u64::from(SIM_TICK_HZ)))
        );
        assert!(
            snaps
                .iter()
                .flat_map(|x| &x.riders)
                .filter(|r| r.account_id.is_empty())
                .count()
                > 20,
            "the anonymous rider never has one"
        );
        s.take_directed();
        s.handle(
            anon,
            &encode(&client_message(C::VoiceBlocks(v1::VoiceBlocks {
                account_ids: vec!["acc_42".into()],
                ..Default::default()
            }))),
        );
        let told: Vec<ConnectionId> = s.take_directed().into_iter().map(|(c, _)| c).collect();
        assert!(told.contains(&signed) && told.contains(&anon), "{told:?}");
        assert!(!s.voice_apart(signed).is_empty());
    }

    #[derive(Default)]
    struct Recorder(std::sync::Mutex<Vec<EndedRide>>);

    impl CreditSink for Recorder {
        fn ride_ended(&self, ride: EndedRide) {
            self.0.lock().unwrap().push(ride);
        }
    }

    #[derive(Default)]
    struct FakeGarages {
        owned: std::sync::Mutex<BTreeMap<String, Owned>>,
        updated: std::sync::Mutex<Vec<String>>,
        asked: std::sync::Mutex<Vec<(String, String)>>,
    }

    impl PartsOwned for FakeGarages {
        fn refresh(&self, account: &str, token: &str) {
            self.asked
                .lock()
                .unwrap()
                .push((account.into(), token.into()));
        }
        fn owned(&self, account: &str) -> Option<Owned> {
            self.owned.lock().unwrap().get(account).cloned()
        }
        fn take_updated(&self) -> Vec<String> {
            std::mem::take(&mut self.updated.lock().unwrap())
        }
    }

    /// The ownership check (spec 14 Phase 4, 4O W5d; 4E): with it, a
    /// signed-in rider's `BikeChange` counts only the parts the account
    /// owns (a profile it has no bike of: the starter's), an anonymous
    /// rider's only base parts; a garage arriving later applies the bike
    /// asked for again; the rider's token fetches it.
    #[test]
    fn only_owned_parts_count_when_the_server_checks() {
        const OWNED: &str = "wheels.road-carbon.mid";
        const NOT_YET: &str = "tyres.road.race";
        let garages = std::sync::Arc::new(FakeGarages::default());
        garages.owned.lock().unwrap().insert(
            "acc_42".into(),
            Owned {
                parts: [OWNED.to_string()].into(),
                profiles: ["road".to_string()].into(),
            },
        );
        let mut s = GameServer::new(
            Route::test_loop(),
            ServerConfig {
                auth: Some(verifier()),
                ownership: Some(Ownership(garages.clone())),
                ..ServerConfig::default()
            },
        );
        let (signed, _) = join(&mut s, Some(token(600, |_| {})));
        let (anon, _) = join(&mut s, None);
        assert_eq!(
            garages.asked.lock().unwrap()[0].0,
            "acc_42",
            "fetched with the token"
        );
        let change = |profile: &str| {
            encode(&client_message(C::BikeChange(v1::BikeChange {
                bike_id: "b".into(),
                bike_profile: profile.into(),
                look: None,
                parts: vec![OWNED.into(), NOT_YET.into()],
                damage: None,
            })))
        };
        let expect =
            |s: &GameServer, conn: ConnectionId, bike: ocw_core::ProfileId, parts: &[&str]| {
                let rider = s.riders[&conn];
                let kg = s.session().base_profile(rider).unwrap().rider_mass_kg;
                let parts: Vec<String> = parts.iter().map(|p| p.to_string()).collect();
                let want = ocw_sim::build::build(ocw_sim::PartTable::shipped(), bike, &parts)
                    .rider_profile(kg);
                assert_eq!(s.session().rider_profile(rider).unwrap(), want, "{parts:?}");
            };
        s.handle(signed, &change("road"));
        expect(&s, signed, ocw_core::ProfileId::Road, &[OWNED]);
        s.handle(anon, &change("road"));
        expect(&s, anon, ocw_core::ProfileId::Road, &[]);
        // A time-trial bike the garage does not have: the starter's road.
        s.handle(signed, &change("tt"));
        expect(&s, signed, ocw_core::ProfileId::Road, &[OWNED]);
        // The tyres bought meanwhile: the garage arrives, the bike asked
        // for applies again.
        s.handle(signed, &change("road"));
        garages
            .owned
            .lock()
            .unwrap()
            .get_mut("acc_42")
            .unwrap()
            .parts
            .insert(NOT_YET.into());
        garages.updated.lock().unwrap().push("acc_42".into());
        s.tick();
        expect(&s, signed, ocw_core::ProfileId::Road, &[OWNED, NOT_YET]);
        // Without the check every part counts (the app's own garage).
        assert_eq!(
            owned_build(ocw_core::ProfileId::Gravel, &[NOT_YET.into()], None).0,
            STARTER_PROFILE
        );
    }

    /// A server with a credit key (spec 14 Phase 4, 4O Q19): a signed-in
    /// rider's ride goes to the credit sink when they leave – the work of
    /// real power, not that of virtual power, never an anonymous rider's.
    #[test]
    fn a_signed_in_riders_ride_is_credited_when_they_leave() {
        let rec = std::sync::Arc::new(Recorder::default());
        let mut s = GameServer::new(
            Route::test_loop(),
            ServerConfig {
                auth: Some(verifier()),
                hearts_credits: Some(Credits(rec.clone())),
                ..ServerConfig::default()
            },
        );
        let (signed, k) = join(&mut s, Some(token(600, |_| {})));
        let ServerMessageKind::Welcome(w) = k else {
            panic!("{k:?}")
        };
        assert!(w.hearts_credited, "the welcome says the server credits");
        let (anon, _) = join(&mut s, None);
        let (virt, _) = join(&mut s, Some(token(600, |c| c.sub = "acc_43".into())));
        for seq in 0..SIM_TICK_HZ * 120 {
            for (c, virtual_power) in [(signed, false), (anon, false), (virt, true)] {
                s.handle(
                    c,
                    &encode(&client_message(C::Sample(v1::SensorSample {
                        seq,
                        power_w: Some(400.0),
                        cadence_rpm: Some(90.0),
                        virtual_power: Some(virtual_power),
                        ..Default::default()
                    }))),
                );
            }
            s.tick();
        }
        for c in [signed, anon, virt] {
            s.disconnect(c);
        }
        let rides = rec.0.lock().unwrap().clone();
        assert_eq!(rides.len(), 1, "{rides:?}");
        let r = &rides[0];
        assert_eq!(r.account_id, "acc_42");
        assert!(r.work_j > 40_000.0 && r.distance_m > 100.0, "{r:?}");
        assert!(r.ended_unix_s >= r.started_unix_s);
        // Without a sink the welcome says so.
        let (_, k) = join(&mut server(false), Some(token(600, |_| {})));
        let ServerMessageKind::Welcome(w) = k else {
            panic!("{k:?}")
        };
        assert!(!w.hearts_credited);
    }

    #[derive(Debug)]
    enum ServerMessageKind {
        Welcome(Box<v1::Welcome>),
        Reject(v1::Reject),
    }

    fn server(required: bool) -> GameServer {
        GameServer::new(
            Route::test_loop(),
            ServerConfig {
                auth: Some(verifier()),
                auth_required: required,
                ..ServerConfig::default()
            },
        )
    }

    #[test]
    fn a_good_token_names_the_rider() {
        let mut s = server(false);
        let (conn, k) = join(&mut s, Some(token(600, |_| {})));
        let ServerMessageKind::Welcome(w) = k else {
            panic!("{k:?}")
        };
        let a = w.account.expect("the account");
        assert_eq!(
            (a.account_id.as_str(), a.display_name.as_str()),
            ("acc_42", "Ada Rider")
        );
        assert_eq!(a.providers, ["google", "strava"]);
        assert_eq!(
            s.account(conn).map(|a| a.account_id.as_str()),
            Some("acc_42")
        );
        let me = s
            .session
            .snapshot()
            .into_iter()
            .find(|r| r.id.0 == w.rider_id)
            .unwrap();
        assert_eq!(me.name, "Ada Rider", "the account's name, not the hello's");
        s.disconnect(conn);
        assert!(s.account(conn).is_none());
    }

    #[test]
    fn bad_or_expired_tokens_are_refused() {
        let mut s = server(false);
        let other = ed25519_dalek::SigningKey::from_bytes(&[6; 32]);
        let forged = {
            let now = unix_now_ms() / 1000;
            let c = AccessClaims {
                iss: ISS.into(),
                sub: "acc_admin".into(),
                aud: ocw_auth::DEFAULT_AUDIENCE.into(),
                exp: now + 600,
                iat: now,
                jti: "x".into(),
                client_id: "x".into(),
                scope: String::new(),
                name: "Evil".into(),
                auth_time: now,
                providers: vec![],
            };
            // Our key id, another key.
            sign(&c, &other, &key_id(&key().verifying_key()), TYP_ACCESS)
        };
        let id_token = {
            let k = key();
            let t = token(600, |_| {});
            let claims: AccessClaims = ocw_auth::jwt::Parsed::parse(&t)
                .unwrap()
                .claims_unverified()
                .unwrap();
            sign(&claims, &k, &key_id(&k.verifying_key()), TYP_ID)
        };
        for (what, t) in [
            ("expired", token(-120, |_| {})),
            (
                "another issuer",
                token(600, |c| c.iss = "https://evil.example".into()),
            ),
            (
                "another audience",
                token(600, |c| c.aud = "other-game".into()),
            ),
            ("forged", forged),
            ("an ID token", id_token),
            ("garbage", "not.a.token".into()),
        ] {
            let (_, k) = join(&mut s, Some(t));
            let ServerMessageKind::Reject(r) = k else {
                panic!("{what}: {k:?}")
            };
            assert!(r.auth, "{what}");
            assert!(r.reason.contains("sign in again"), "{what}: {}", r.reason);
        }
        assert_eq!(s.session.snapshot().len(), 0, "nobody joined");
    }

    #[test]
    fn anonymous_riders_stay_allowed_unless_required() {
        let mut s = server(false);
        let (_, k) = join(&mut s, None);
        let ServerMessageKind::Welcome(w) = k else {
            panic!("{k:?}")
        };
        assert!(w.account.is_none());
        let mut s = server(true);
        let (_, k) = join(&mut s, None);
        let ServerMessageKind::Reject(r) = k else {
            panic!("{k:?}")
        };
        assert!(r.auth && r.reason.contains("signed-in account"));
        let (_, k) = join(&mut s, Some(token(600, |_| {})));
        assert!(matches!(k, ServerMessageKind::Welcome(_)));
    }

    #[test]
    fn servers_without_sign_in_ignore_tokens() {
        let mut s = GameServer::new(Route::test_loop(), ServerConfig::default());
        let (_, k) = join(&mut s, Some("anything".into()));
        let ServerMessageKind::Welcome(w) = k else {
            panic!("{k:?}")
        };
        assert!(w.account.is_none());
    }
}
