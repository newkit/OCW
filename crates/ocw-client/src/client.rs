//! Engine-independent client logic: device samples → server, server messages →
//! prediction (own rider) and interpolation (others) → render view.
//!
//! The Bevy systems in `connection.rs` only move bytes in and out of this type,
//! so the whole client loop is unit-testable without a window or GPU.
//!
//! In free ride (spec §2.2.1) the client also predicts its own path through
//! the road graph: it locks the chosen exit at the lock point like the
//! server does, sends the choice as a `TurnIntent`, and takes the server's
//! `PathUpdate`s as corrections; other riders' paths come only from the
//! server.

use std::collections::BTreeMap;
use std::sync::Arc;

use ocw_core::{INTERPOLATION_DELAY_S, ProfileId, RiderId, RiderLook, RiderProfile};
use ocw_devices::DeviceEvent;
use ocw_protocol::v1::{self, client_message::Msg as C, server_message::Msg as S};
use ocw_protocol::{PROTOCOL_VERSION, client_message, decode_server, dequantize, encode};
use ocw_render::{ExitView, JunctionView, RiderView};
use ocw_sim::freeride::{self, TurnChoice, UTURN_INTERVAL_S};
use ocw_sim::{Predictor, RiderState, SnapshotBuffer};
use ocw_world::path::lock_distance;
use ocw_world::{Junction, RidePath, RoadNetwork, Route, RoutePlan, RouteState, Track, TurnReason};

use crate::event::{EventClient, GhostKey, GhostOutcome};

/// Minimum time between steering messages while the target changes.
pub const STEER_SEND_INTERVAL_S: f64 = 0.05;
/// The junction overlay appears within this distance, or within
/// [`JUNCTION_LOOKAHEAD_S`] of riding, whichever is larger (spec §2.2.1).
pub const JUNCTION_LOOKAHEAD_M: f64 = 150.0;
pub const JUNCTION_LOOKAHEAD_S: f64 = 10.0;
/// The ghost rides this far right of the centre line (m), beside the
/// rider (who starts a time trial in the middle of the road).
pub const GHOST_LATERAL_M: f64 = 1.0;
/// Dead-end warnings start this far before the end (spec §2.2.2).
pub const DEAD_END_WARNING_M: f64 = 1000.0;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NetStats {
    pub messages_in: u64,
    pub malformed: u64,
    pub snapshots: u64,
    pub samples_out: u64,
    pub last_snapshot_tick: u64,
    /// Snapshots dropped because a newer one had arrived (datagrams can be
    /// reordered on a network link).
    pub stale_snapshots: u64,
    /// Free ride: path updates received, and own-path corrections among them.
    pub path_updates: u64,
}

/// Something the user should be told about (shown in the console).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    Joined {
        rider_id: RiderId,
        route_id: String,
    },
    Rejected(String),
    CommandResult(String),
    /// The answer to a voice token request (spec §7).
    Voice(v1::VoiceGrant),
    /// The session rides another world now (spec §3.2.1, the host's
    /// World → Change world): load it and join again.
    WorldChange(v1::WorldChange),
    /// God mode (spec §14 Phase 13): the server's answer, and what changed.
    God {
        event: Option<crate::god_state::GodEvent>,
        message: String,
    },
}

/// Free-ride state of the client.
#[derive(Debug, Clone)]
struct FreeRide {
    net: Arc<RoadNetwork>,
    /// Route ride: the route the own path follows (spec §2.7).
    plan: Option<Arc<RoutePlan>>,
    /// The own path, predicted (shared with the render view).
    own: Option<Arc<RidePath>>,
    /// The other riders' paths, from the server.
    others: BTreeMap<RiderId, Arc<RidePath>>,
    /// Exit chosen for the next junction (`None`: the default).
    choice: Option<TurnChoice>,
    turn_seq: u32,
    /// Last choice sent: for which junction, and what.
    turn_sent: Option<(ocw_world::graph::VertexId, Option<TurnChoice>)>,
    last_u_turn_s: Option<f64>,
    /// The junction locked last, until passed: (junction, its path
    /// distance, the exit taken).
    locked: Option<(Junction, f64, usize)>,
}

pub struct ClientCore {
    route: Route,
    free: Option<FreeRide>,
    /// The own weight and the frame's curated profile (spec §2.5).
    profile: RiderProfile,
    /// The own bike's parts and their build (spec §14 Phase 4, 4G; the
    /// same pure code and catalogue as the server's), the server's word on
    /// it (`RiderBuild`), and what the prediction rides with now (the race
    /// mode in force).
    parts: Vec<String>,
    own_build: Option<ocw_sim::Build>,
    own_effect: Option<ocw_protocol::BuildInfo>,
    effective: RiderProfile,
    /// The other riders' builds (`RiderBuild`, once per change) and the
    /// hash each snapshot names (a build is drawn only while they agree).
    builds: BTreeMap<RiderId, ocw_protocol::BuildInfo>,
    build_hashes: BTreeMap<RiderId, u32>,
    /// The own cosmetic look (sent in the hello, spec §2.5).
    look: RiderLook,
    name: String,
    rider_id: Option<RiderId>,
    predictor: Predictor,
    /// Predicted state before the last physics step: rendering interpolates
    /// between it and the current one, so motion is smooth at any frame rate.
    prev_state: RiderState,
    /// Steering target (lateral offset); set from the first own snapshot.
    lateral_target: Option<f64>,
    /// Lean steering (rad, positive = right), the arrow keys; replaces the
    /// target while set.
    lean: Option<f64>,
    /// Wild mode: not bound to the road.
    wild: bool,
    /// Autodrive on when the ride starts (setting `controls.autodrive`).
    autodrive_at_start: bool,
    steer_seq: u32,
    /// Last steering intent sent ((lean mode, mrad or cm, wild)) and when.
    steer_sent: Option<((bool, i32, bool), f64)>,
    others: SnapshotBuffer,
    /// Latest known state of the other riders, advanced every physics tick
    /// (for predicting the local rider's drafting).
    others_now: Vec<(RiderId, RiderState)>,
    names: BTreeMap<RiderId, (String, f64)>,
    /// Every rider's bike and look from the snapshots (no look: an old
    /// server; the renderer picks colours).
    looks: BTreeMap<RiderId, (ProfileId, Option<RiderLook>)>,
    power_w: f64,
    cadence_rpm: Option<f64>,
    heart_rate_bpm: Option<f64>,
    /// Extra power from the keyboard, on top of the device power.
    key_power_w: f64,
    /// The power device is virtual (the virtual trainer): no hearts
    /// (spec §14 Phase 4, 4E).
    virtual_device: bool,
    /// Hearts the server credited the own rider in this session.
    hearts: u32,
    braking: bool,
    last_sample_us: u64,
    seq: u32,
    stats: NetStats,
    /// How old a snapshot is on arrival (half the round trip; 0 for the
    /// in-process server): the prediction extrapolates it by this much.
    snapshot_age_s: f64,
    /// The steering mode saved while the menu holds autodrive.
    menu_hold: Option<HeldSteering>,
    /// The current event (spec §2.3) and the time trial's ghost.
    event: EventClient,
    /// The last snapshot's server tick and client time it arrived.
    last_snapshot_at: Option<(u64, f64)>,
    /// The server offers voice chat (`Welcome.voice`).
    voice_offered: bool,
    /// Human riders in the session (from the last snapshot).
    human_riders: u32,
    /// The server lets riders put an Echo rider on the road
    /// (`Welcome.echo_bot`) and the one on the road (spec §7 *Echo bot*).
    echo_bot: bool,
    echo_rider: Option<RiderId>,
    /// The session's world clock (spec §14 Phase 9): the server's, advanced
    /// smoothly between snapshots, and its last message.
    clock: ocw_sky::ClockPredictor,
    clock_info: Option<ocw_protocol::ClockInfo>,
    /// The session's weather (spec §14 Phase 9, S4): the server's, eased
    /// between snapshots, and the lightning strikes.
    weather: crate::weather::WeatherClient,
    /// Who may switch the world (from the welcome).
    world_access: Option<ocw_protocol::ClockAccess>,
    /// Who may enter god mode (spec §14 Phase 13A; `None`: not yet told).
    god_access: Option<ocw_protocol::ClockAccess>,
    /// Unicorn mode: the own flight and the other riders' unicorns.
    god: crate::god_state::GodCore,
    god_others: crate::god_state::OtherGods,
    /// A god-mode change found in a snapshot (the server ended it).
    god_notice: Option<crate::god_state::GodEvent>,
    /// Collisions (spec §14 Phase 12): the rules, crashes and bumps from
    /// the server, the own autodrive's avoidance (as the server's), the
    /// own rider down in the last snapshot, the proximity cue setting.
    collisions: crate::collisions::CollisionClient,
    avoid: ocw_sim::avoid::AvoidState,
    was_down: bool,
    proximity_cue: bool,
}

/// Steering mode before the menu opened (spec §3.2.1), restored on closing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HeldSteering {
    autodrive: bool,
    wild: bool,
}

impl ClientCore {
    pub fn new(route: Route, name: impl Into<String>, profile: RiderProfile) -> Self {
        Self {
            free: None,
            profile,
            parts: Vec::new(),
            own_build: None,
            own_effect: None,
            effective: profile,
            builds: BTreeMap::new(),
            build_hashes: BTreeMap::new(),
            look: RiderLook::default(),
            name: name.into(),
            rider_id: None,
            predictor: Predictor::new(profile, RiderState::default()),
            prev_state: RiderState::default(),
            lateral_target: None,
            lean: None,
            wild: false,
            autodrive_at_start: true,
            steer_seq: 0,
            steer_sent: None,
            others: SnapshotBuffer::default(),
            others_now: Vec::new(),
            names: BTreeMap::new(),
            looks: BTreeMap::new(),
            power_w: 0.0,
            cadence_rpm: None,
            heart_rate_bpm: None,
            key_power_w: 0.0,
            virtual_device: false,
            hearts: 0,
            braking: false,
            last_sample_us: 0,
            seq: 0,
            stats: NetStats::default(),
            snapshot_age_s: 0.0,
            menu_hold: None,
            event: EventClient::new(route.id()),
            last_snapshot_at: None,
            voice_offered: false,
            human_riders: 0,
            echo_bot: false,
            echo_rider: None,
            clock: ocw_sky::ClockPredictor::default(),
            clock_info: None,
            weather: Default::default(),
            world_access: None,
            god_access: None,
            god: Default::default(),
            god_others: Default::default(),
            god_notice: None,
            collisions: Default::default(),
            avoid: Default::default(),
            was_down: false,
            proximity_cue: false,
            route,
        }
    }

    /// A client for a free ride on `net` (the server sends the paths).
    pub fn free_ride(
        net: Arc<RoadNetwork>,
        name: impl Into<String>,
        profile: RiderProfile,
    ) -> Self {
        let v = net.graph().vertices.first().copied().unwrap_or([0.0, 0.0]);
        let placeholder = Route::from_points(
            "free-ride",
            "Free ride",
            vec![[v[0], 0.0, v[1]], [v[0] + 1.0, 0.0, v[1]]],
            false,
            5.0,
        )
        .expect("two distinct points");
        Self {
            free: Some(FreeRide {
                net,
                plan: None,
                own: None,
                others: BTreeMap::new(),
                choice: None,
                turn_seq: 0,
                turn_sent: None,
                last_u_turn_s: None,
                locked: None,
            }),
            ..Self::new(placeholder, name, profile)
        }
    }

    /// A route ride on the road graph (spec §2.7): the own path follows
    /// `plan` (the server's session rides the same route).
    pub fn with_route_plan(mut self, plan: Arc<RoutePlan>) -> Self {
        if let Some(fr) = &mut self.free {
            let v = self.route.points()[0];
            self.route = Route::from_points(
                plan.name.clone(),
                plan.name.clone(),
                vec![v, [v[0] + 1.0, v[1], v[2]]],
                false,
                5.0,
            )
            .expect("two distinct points");
            self.event = EventClient::new(plan.name.clone());
            fr.plan = Some(plan);
        }
        self
    }

    /// The same client with a cosmetic look (spec §2.5), sent in the hello.
    pub fn with_look(mut self, look: RiderLook) -> Self {
        self.look = look;
        self
    }

    /// The same client with the bike's parts (catalogue ids, spec §14
    /// Phase 4, 4G), sent in the hello; the prediction rides the build.
    pub fn with_parts(mut self, parts: &[String]) -> Self {
        self.set_own_build(self.profile.bike, parts);
        self.refresh_physics();
        self
    }

    fn set_own_build(&mut self, bike: ProfileId, parts: &[String]) {
        self.parts = parts.to_vec();
        let b = ocw_sim::build::build(ocw_sim::PartTable::shipped(), bike, parts);
        self.own_build = (!b.is_stock()).then_some(b);
    }

    /// The race mode the own rider rides under now (spec §14 Phase 4,
    /// 4G): the event's while entered in one that is not over, else open.
    pub fn physics_mode(&self) -> ocw_sim::PhysicsMode {
        match self.event.info() {
            Some(info) if self.in_event(info) => info.physics,
            _ => ocw_sim::PhysicsMode::Open,
        }
    }

    fn in_event(&self, info: &ocw_protocol::EventInfo) -> bool {
        info.phase != ocw_sim::EventPhase::Finished
            && info.standings.iter().any(|s| Some(s.id) == self.rider_id)
    }

    /// The prediction rides what the server rides: the build under the
    /// race mode in force.
    fn refresh_physics(&mut self) {
        let p =
            ocw_sim::build::effective(self.physics_mode(), &self.profile, self.own_build.as_ref());
        if p != self.effective {
            self.effective = p;
            self.predictor.set_profile(p);
        }
    }

    /// The own bike's build as the prediction knows it (`None`: stock).
    pub fn own_build(&self) -> Option<&ocw_sim::Build> {
        self.own_build.as_ref()
    }

    /// The server's word on the own bike's build (its effect after the
    /// caps), once a `RiderBuild` arrived.
    pub fn own_effect(&self) -> Option<&ocw_protocol::BuildInfo> {
        self.own_effect.as_ref()
    }

    /// The physics the own rider rides with now (profile, build, mode).
    pub fn effective_profile(&self) -> RiderProfile {
        self.effective
    }

    /// Another rider's character, from their last `RiderBuild` (`None`:
    /// not told yet, or an older server).
    pub fn rider_character(&self, id: RiderId) -> Option<ocw_core::Character> {
        self.builds
            .get(&id)
            .and_then(|b| b.look)
            .map(|l| l.character)
    }

    /// Another rider's kit design, from their last `RiderBuild` (spec §14
    /// Phase 8 T6; `None`: not told yet, or an older server).
    pub fn rider_design(&self, id: RiderId) -> Option<ocw_core::KitDesign> {
        self.builds.get(&id).and_then(|b| b.look).map(|l| l.design)
    }

    /// A rider's look from the snapshots (colours, pattern) with the
    /// character and the kit design of their last `RiderBuild` (spec §14
    /// Phase 4, 4C; Phase 8 T6: once per change).
    fn with_character(&self, id: RiderId, look: Option<RiderLook>) -> Option<RiderLook> {
        let full = self.builds.get(&id).and_then(|b| b.look);
        match (look, full) {
            (Some(mut l), Some(f)) => {
                l.character = f.character;
                l.design = f.design;
                Some(l)
            }
            (l, _) => l,
        }
    }

    /// The fitted parts of a rider's bike as last told by the server
    /// (empty: the stock bike, or the build not known yet).
    pub fn rider_parts(&self, id: RiderId) -> Vec<String> {
        if Some(id) == self.rider_id {
            return self
                .own_build
                .as_ref()
                .map(|b| b.parts.clone())
                .unwrap_or_default();
        }
        let hash = self.build_hashes.get(&id).copied().unwrap_or(0);
        match self.builds.get(&id) {
            Some(b) if hash != 0 && b.hash == hash => b.parts.clone(),
            _ => Vec::new(),
        }
    }

    /// The same client starting with autodrive off (lean steering) when
    /// `on` is false (setting `controls.autodrive`, spec §3.2.1).
    pub fn with_autodrive_at_start(mut self, on: bool) -> Self {
        self.autodrive_at_start = on;
        self
    }

    /// Name of what is ridden (the route's, or "Free ride").
    pub fn course_name(&self) -> &str {
        self.route.name()
    }

    /// (lat, lon) of the local metres, if known (route files, packages).
    pub fn geo_origin(&self) -> Option<(f64, f64)> {
        self.route.geo_origin()
    }

    /// The same client with the geographic origin of its local metres
    /// (free ride: the world package's), for the GPS track of rides.
    pub fn with_geo_origin(mut self, origin: (f64, f64)) -> Self {
        self.route = self.route.clone().with_geo_origin(origin);
        self
    }

    pub fn is_free_ride(&self) -> bool {
        self.free.is_some()
    }

    /// The first message to send after connecting.
    pub fn hello(&self) -> Vec<u8> {
        encode(&client_message(C::Hello(v1::Hello {
            protocol_version: PROTOCOL_VERSION,
            display_name: self.name.clone(),
            // The server uses its curated profile with our weight (§2.5).
            profile: None,
            world: self.free.as_ref().map(|f| ocw_protocol::world_id(&f.net)),
            rider_weight_kg: Some(self.profile.rider_mass_kg as f32),
            // Only the profile's id: the server has its own table (§2.5).
            bike_profile: Some(self.profile.bike.id().into()),
            look: Some(self.look.into()),
            // Part ids only: the server has its own catalogue (4G).
            parts: self.parts.clone(),
        })))
    }

    /// Handles a device event; returns the encoded sample to send, if any.
    /// Heart rate is stored and sent with the next power sample.
    pub fn on_device_event(&mut self, event: DeviceEvent) -> Option<Vec<u8>> {
        let (watts, cadence_rpm, timestamp) = match event {
            DeviceEvent::Power {
                watts,
                cadence_rpm,
                timestamp,
            } => (watts, cadence_rpm, timestamp),
            DeviceEvent::HeartRate { bpm, .. } => {
                self.heart_rate_bpm = Some(bpm);
                return None;
            }
            DeviceEvent::Connected { .. } | DeviceEvent::Disconnected => return None,
        };
        self.power_w = watts;
        self.cadence_rpm = cadence_rpm;
        self.last_sample_us = timestamp.as_micros() as u64;
        Some(self.sample())
    }

    /// Encodes the current inputs as the next sensor sample.
    fn sample(&mut self) -> Vec<u8> {
        self.seq += 1;
        self.stats.samples_out += 1;
        encode(&client_message(C::Sample(v1::SensorSample {
            seq: self.seq,
            client_time_us: self.last_sample_us,
            power_w: Some(self.power_w() as f32),
            cadence_rpm: self.cadence_rpm.map(|c| c as f32),
            heart_rate_bpm: self.heart_rate_bpm.map(|h| h as f32),
            brake: Some(self.braking),
            virtual_power: Some(self.virtual_power()),
        })))
    }

    /// Brake key pressed or released; returns a sample to send at once when
    /// the state changed (the next trainer sample may be 250 ms away).
    pub fn set_brake(&mut self, braking: bool) -> Option<Vec<u8>> {
        if braking == self.braking {
            return None;
        }
        self.braking = braking;
        self.predictor.set_braking(braking);
        Some(self.sample())
    }

    pub fn braking(&self) -> bool {
        self.braking
    }

    /// Handles one encoded server message received at client time `now_s`.
    pub fn on_server_message(&mut self, bytes: &[u8], now_s: f64) -> Option<Notice> {
        self.stats.messages_in += 1;
        let Ok(msg) = decode_server(bytes) else {
            self.stats.malformed += 1;
            return None;
        };
        match msg {
            S::Welcome(w) => {
                // Junctions are vertex indices: riding on another graph
                // than the server's would go wrong at every junction.
                if let (Some(fr), Some(theirs)) = (&self.free, &w.world)
                    && theirs.graph_hash != fr.net.fingerprint()
                {
                    return Some(Notice::Rejected(format!(
                        "world package mismatch: the server rides {} ({:016x}), this client has {} ({:016x})",
                        theirs.name,
                        theirs.graph_hash,
                        fr.net.package,
                        fr.net.fingerprint()
                    )));
                }
                // Riders are distances along the course: both sides must
                // ride the same one (graph rides are checked above).
                if w.free_ride != self.free.is_some()
                    || (!w.free_ride && w.route_id != self.route.id())
                {
                    let theirs = if w.free_ride {
                        "on a world package's road graph".to_owned()
                    } else {
                        format!("route {:?}", w.route_id)
                    };
                    let ours = if self.free.is_some() {
                        "on a world package's road graph".to_owned()
                    } else {
                        format!("route {:?}", self.route.id())
                    };
                    return Some(Notice::Rejected(format!(
                        "course mismatch: the server rides {theirs}, this client {ours}; start both with the same --route / --world"
                    )));
                }
                let id = RiderId(w.rider_id);
                self.rider_id = Some(id);
                self.voice_offered = w.voice;
                self.echo_bot = w.echo_bot;
                self.world_access = Some(ocw_protocol::access_from_wire(w.world_access));
                self.god_access = Some(ocw_protocol::access_from_wire(w.god_access));
                self.collisions.on_welcome(&w);
                if let Some(c) = &w.clock {
                    self.on_clock(c, now_s);
                }
                if let Some(weather) = &w.weather {
                    self.weather.on_message(weather, now_s, self.snapshot_age_s);
                }
                if !self.autodrive_at_start {
                    self.set_autodrive(false);
                }
                Some(Notice::Joined {
                    rider_id: id,
                    route_id: w.route_id,
                })
            }
            S::Reject(r) => Some(Notice::Rejected(r.reason)),
            S::DevResult(r) => Some(Notice::CommandResult(if r.ok {
                r.message
            } else {
                format!("error: {}", r.message)
            })),
            S::Snapshot(s) => {
                self.on_snapshot(&s, now_s);
                None
            }
            S::Path(p) => {
                self.on_path(&p);
                None
            }
            S::Voice(g) => Some(Notice::Voice(g)),
            S::God(r) => Some(Notice::God {
                event: self.god.on_result(&r, now_s),
                message: if r.ok {
                    r.message
                } else {
                    format!("unicorn mode: {}", r.message)
                },
            }),
            S::WorldChange(c) => {
                // The session this client joined has ended: its rider ids
                // mean other riders now, so nothing is reconciled until
                // the app has loaded the new world and joined again.
                self.rider_id = None;
                Some(Notice::WorldChange(c))
            }
            S::Event(e) => {
                let info = ocw_protocol::event_info(&e);
                if self.event.on_state(info, self.rider_id, now_s) {
                    // Moved to the start formation: steer from the new
                    // place on (autodrive, the lane the server gave).
                    self.lateral_target = None;
                    self.lean = None;
                    self.steer_sent = None;
                }
                self.refresh_physics();
                None
            }
            S::RiderBuild(b) => {
                let info = ocw_protocol::build_info(&b);
                if Some(info.rider) == self.rider_id {
                    // The server applied the own bike (at once, or after
                    // an event): the prediction follows it.
                    let kg = self.profile.rider_mass_kg;
                    self.profile = RiderProfile::curated(info.profile, kg);
                    let parts = info.parts.clone();
                    self.set_own_build(info.profile, &parts);
                    self.own_effect = Some(info);
                    self.refresh_physics();
                } else {
                    self.builds.insert(info.rider, info);
                }
                None
            }
        }
    }

    fn on_clock(&mut self, c: &v1::WorldClock, now_s: f64) {
        let info = ocw_protocol::clock_info(c);
        self.clock
            .on_server(info.world_ms, info.rate, now_s, self.snapshot_age_s);
        self.clock_info = Some(info);
    }

    /// The world instant (Unix ms) at client time `now_s` (spec §14 Phase
    /// 9): the server's clock, predicted; `None` before the first message
    /// (or from a server without a clock).
    pub fn world_ms(&mut self, now_s: f64) -> Option<i64> {
        self.clock.now(now_s)
    }

    /// The server's clock as last received (mode, speed, who may change it).
    pub fn clock_info(&self) -> Option<&ocw_protocol::ClockInfo> {
        self.clock_info.as_ref()
    }

    /// Whether this rider may change the world clock (the server checks it
    /// again): anyone on a local server, the host in a shared ride.
    pub fn may_change_clock(&self) -> bool {
        self.clock_info.is_some_and(|c| c.may_change(self.rider_id))
    }

    /// Whether this rider may switch the world (spec §3.2.1; the server
    /// checks it again): anyone on a local server, the host of a shared ride.
    pub fn may_switch_world(&self) -> bool {
        let host = self.clock_info.and_then(|c| c.host);
        self.world_access
            .is_none_or(|a| a.allows(self.rider_id, host))
    }

    /// Who switches the world, for messages: "you", "the host", "the organizer".
    pub fn world_control(&self) -> &'static str {
        match self.world_access {
            _ if self.may_switch_world() => "you",
            Some(ocw_protocol::ClockAccess::Nobody) => "the organizer",
            _ => "the host",
        }
    }

    /// The session's weather on this client (spec §14 Phase 9, S4).
    pub fn weather(&self) -> &crate::weather::WeatherClient {
        &self.weather
    }

    pub fn weather_mut(&mut self) -> &mut crate::weather::WeatherClient {
        &mut self.weather
    }

    /// Whether this rider may change the weather (the server checks it
    /// again; Q2): anyone on a local server, the host of a shared ride.
    pub fn may_change_weather(&self) -> bool {
        let host = self.clock_info.and_then(|c| c.host);
        self.weather
            .info()
            .is_some_and(|w| w.may_change(self.rider_id, host))
    }

    /// Who sets the weather, for messages: "you", "the host", "the organizer".
    pub fn weather_control(&self) -> &'static str {
        match self.weather.info().map(|w| w.access) {
            _ if self.may_change_weather() => "you",
            Some(ocw_protocol::ClockAccess::Nobody) => "the organizer",
            _ => "the host",
        }
    }

    /// The encoded weather request.
    pub fn weather_message(change: &ocw_protocol::WeatherChange) -> Vec<u8> {
        encode(&ocw_protocol::weather_request(change))
    }

    /// The encoded time-machine request.
    pub fn clock_message(change: ocw_sky::ClockChange) -> Vec<u8> {
        encode(&ocw_protocol::clock_request(change))
    }

    /// A rider's path from the server: a new one, an extension, or a
    /// correction of the own prediction (spec §6.5).
    fn on_path(&mut self, u: &v1::PathUpdate) {
        let own_id = self.rider_id;
        let Some(fr) = &mut self.free else { return };
        let Ok(segments) = ocw_protocol::path_segments(u, &fr.net) else {
            self.stats.malformed += 1;
            return;
        };
        self.stats.path_updates += 1;
        let rider = RiderId(u.rider_id);
        let current = if Some(rider) == own_id {
            fr.own.take()
        } else {
            fr.others.remove(&rider)
        };
        let net = fr.net.clone();
        // The own path follows the route (others' paths only come from the
        // server and need no route).
        let plan = if Some(rider) == own_id {
            fr.plan.clone()
        } else {
            None
        };
        let fresh = || {
            RidePath::from_segments(net.clone(), &segments, u.start_m, u.first_index)
                .ok()
                .map(|p| Arc::new(p.with_plan(plan.clone())))
        };
        let updated = match current {
            Some(mut p) if !u.reset && u.first_index > p.first_index() => {
                match Arc::make_mut(&mut p).replace_from(u.first_index, &segments) {
                    Ok(()) => Some(p),
                    Err(_) => fresh(),
                }
            }
            _ => fresh(),
        };
        if let Some(p) = updated {
            if Some(rider) == own_id {
                fr.own = Some(p);
            } else {
                fr.others.insert(rider, p);
            }
        }
    }

    /// Sets how old snapshots are on arrival (half the network round trip).
    pub fn set_snapshot_age(&mut self, age_s: f64) {
        self.snapshot_age_s = age_s.clamp(0.0, 1.0);
    }

    fn on_snapshot(&mut self, s: &v1::Snapshot, now_s: f64) {
        // Datagrams may arrive out of order: an older snapshot would move
        // everyone back in time.
        if self.stats.snapshots > 0 && s.server_tick <= self.stats.last_snapshot_tick {
            self.stats.stale_snapshots += 1;
            return;
        }
        self.stats.snapshots += 1;
        self.stats.last_snapshot_tick = s.server_tick;
        self.last_snapshot_at = Some((s.server_tick, now_s));
        self.human_riders = s.human_riders;
        self.echo_rider = ocw_protocol::echo_rider(s);
        if let Some(c) = &s.clock {
            self.on_clock(c, now_s);
        }
        if let Some(weather) = &s.weather {
            self.weather.on_message(weather, now_s, self.snapshot_age_s);
        }
        self.collisions.on_snapshot(s, now_s, self.rider_id);
        let mut others = Vec::new();
        let mut gods = Vec::new();
        for u in &s.riders {
            let (id, state) = dequantize(u);
            let god = ocw_protocol::rider_god(u);
            if Some(id) == self.rider_id {
                if let Some(e) = self.god.on_own_snapshot(god.is_some(), now_s) {
                    self.god_notice = Some(e);
                }
            } else if let Some(g) = god {
                gods.push((id, g));
            }
            self.names
                .insert(id, (u.display_name.clone(), f64::from(u.power_w)));
            self.looks.insert(id, ocw_protocol::rider_look(u));
            self.build_hashes.insert(id, u.build_hash);
            if Some(id) == self.rider_id {
                self.hearts = self.hearts.max(u.hearts);
                self.event.on_own_snapshot(s.server_tick, state.distance);
                if self.lateral_target.is_none() {
                    // Start steering from the lane the server assigned.
                    self.lateral_target = Some(state.lateral_offset);
                    self.predictor.set_lateral_target(state.lateral_offset);
                }
                let power = self.power_w();
                let track: &dyn Track = match self.free.as_ref().and_then(|f| f.own.as_deref()) {
                    Some(p) => p,
                    None => &self.route,
                };
                // Down after a crash (spec §14 Phase 12C): the server's state
                // (a crash is only shown when the server says so).
                let down = self.collisions.crash(id, now_s).is_some();
                if self.was_down && !down {
                    // Riding again: autodrive's line where the rider got up.
                    self.lateral_target = Some(state.lateral_offset);
                    if self.lean.is_none() {
                        self.predictor.set_lateral_target(state.lateral_offset);
                    }
                    self.avoid = Default::default();
                }
                self.was_down = down;
                if self.event.holds(self.rider_id) || god.is_some() || self.god.active() || down {
                    // Held at an event's start line, flying in god mode
                    // (the road position frozen, then placed by the
                    // server), or down: exactly the server's.
                    self.predictor.set_state(state);
                    self.prev_state = state;
                } else {
                    self.predictor
                        .reconcile(track, state, self.snapshot_age_s, power);
                }
                // A snap (teleport, large error) must not be interpolated.
                if self.predictor.last_error().abs() > ocw_sim::prediction::SNAP_THRESHOLD_M {
                    self.prev_state = self.predictor.state();
                }
            } else {
                others.push((id, state));
            }
        }
        if let Some(fr) = &mut self.free {
            // Forget riders who left; drop what lies far behind the others.
            fr.others
                .retain(|id, _| others.iter().any(|(o, _)| o == id));
            for (id, st) in &others {
                if let Some(p) = fr.others.get_mut(id)
                    && p.needs_trim(st.distance - 50.0)
                {
                    Arc::make_mut(p).trim_behind(st.distance - 50.0);
                }
            }
        }
        self.others_now = others.clone();
        self.others.push(now_s, others);
        self.god_others.on_snapshot(&gods, now_s);
    }

    /// What the local rider rides along: the own free-ride path (once
    /// known), or the route.
    pub fn track(&self) -> &dyn Track {
        match self.free.as_ref().and_then(|f| f.own.as_deref()) {
            Some(p) => p,
            None => &self.route,
        }
    }

    /// The lateral limit at the rider (road edge, or wild mode's).
    fn lateral_limit(&self) -> f64 {
        if self.wild {
            ocw_sim::physics::WILD_LIMIT_M
        } else {
            let d = self.predictor.state().distance;
            ocw_sim::physics::lateral_limit(self.track().road_width_at(d))
        }
    }

    /// Sets the steering target (lateral offset in metres), within the road.
    pub fn steer_to(&mut self, target_m: f64) {
        if self.rider_id.is_none() {
            return;
        }
        let limit = self.lateral_limit();
        let t = target_m.clamp(-limit, limit);
        self.lateral_target = Some(t);
        self.lean = None;
        self.predictor.set_lateral_target(t);
    }

    /// Lean steering (the arrow keys): the rider leans by `lean_rad`
    /// (positive = right) and rides a curve until the lean is 0 again.
    pub fn set_lean(&mut self, lean_rad: f64) {
        if self.rider_id.is_none() {
            return;
        }
        let max = ocw_sim::physics::MAX_STEER_LEAN;
        let l = lean_rad.clamp(-max, max);
        self.lean = Some(l);
        self.predictor.set_lean(l);
    }

    /// The current lean steering, if the rider steers by leaning.
    pub fn lean(&self) -> Option<f64> {
        self.lean
    }

    /// Autodrive: the rider keeps to its line (the middle of the road
    /// unless moved) by itself; otherwise the rider steers by leaning.
    pub fn autodrive(&self) -> bool {
        self.lean.is_none()
    }

    /// Switches autodrive on (to the middle of the road) or off (upright,
    /// steering by leaning from now on).
    pub fn set_autodrive(&mut self, on: bool) {
        if on {
            self.steer_to(0.0);
        } else {
            self.set_lean(0.0);
        }
    }

    /// Moves the autodrive line sideways by `delta_m` (positive = right).
    pub fn steer_by(&mut self, delta_m: f64) {
        let t = self.lateral_target.unwrap_or(0.0);
        self.steer_to(t + delta_m);
    }

    pub fn wild(&self) -> bool {
        self.wild
    }

    /// Steering mode and surface for the HUD, e.g. "manual | WILD | grass".
    pub fn steering_status(&self) -> String {
        let mut s = String::from(if self.autodrive() {
            "autodrive"
        } else {
            "manual"
        });
        if self.wild {
            s.push_str(" | WILD");
        }
        if self.menu_hold.is_some() {
            s.push_str(" (menu)");
        }
        let surface = ocw_sim::ride::surface(self.track(), &self.predictor.state());
        if surface != ocw_world::Surface::Asphalt {
            s.push_str(" | ");
            s.push_str(surface.name());
        }
        s
    }

    /// Wild mode: not bound to the road, ride anywhere (spec §2.2).
    /// Switching it on also switches autodrive off (upright, lean
    /// steering), so the rider can leave the road; switching it off keeps
    /// the steering mode (spec §5.2.1).
    pub fn set_wild(&mut self, wild: bool) {
        self.wild = wild;
        self.predictor.set_wild(wild);
        if wild {
            self.set_autodrive(false);
        } else {
            // Back within the road for the autodrive line.
            let limit = self.lateral_limit();
            if let Some(t) = self.lateral_target {
                self.lateral_target = Some(t.clamp(-limit, limit));
            }
        }
    }

    /// The menu opened (spec §3.2.1): the ride goes on, so the rider stays
    /// on the road by itself: autodrive on (wild mode off; from manual
    /// steering at the rider's current line, so it does not swerve) and the
    /// next junction back to its default exit. [`Self::release_steering`]
    /// restores the previous mode. Holding twice keeps the first mode.
    pub fn hold_steering(&mut self) {
        if self.menu_hold.is_some() {
            return;
        }
        self.menu_hold = Some(HeldSteering {
            autodrive: self.autodrive(),
            wild: self.wild,
        });
        if self.wild {
            self.set_wild(false);
        }
        if !self.autodrive() {
            let line = self.predictor.state().lateral_offset;
            self.steer_to(line);
        }
        self.clear_exit();
    }

    /// The menu closed: back to the steering mode before
    /// [`Self::hold_steering`]: manual steering upright (no lean carried
    /// over), wild mode again; autodrive keeps the line it rode.
    pub fn release_steering(&mut self) {
        let Some(h) = self.menu_hold.take() else {
            return;
        };
        if h.wild {
            // Directly: `set_wild` would also switch autodrive off.
            self.wild = true;
            self.predictor.set_wild(true);
        }
        if !h.autodrive {
            self.set_lean(0.0);
        }
    }

    /// Whether the menu holds the steering in autodrive.
    pub fn steering_held(&self) -> bool {
        self.menu_hold.is_some()
    }

    pub fn lateral_target(&self) -> Option<f64> {
        self.lateral_target
    }

    /// The steering intent to send now, if the target changed (at most
    /// every `STEER_SEND_INTERVAL_S` while it keeps changing).
    pub fn steer_message(&mut self, now_s: f64) -> Option<Vec<u8>> {
        let key = match self.lean {
            Some(l) => (true, (l * 1000.0).round() as i32, self.wild),
            None => (
                false,
                (self.lateral_target? * 100.0).round() as i32,
                self.wild,
            ),
        };
        if let Some((sent, at)) = self.steer_sent
            && (sent == key || now_s - at < STEER_SEND_INTERVAL_S)
        {
            return None;
        }
        self.steer_seq += 1;
        self.steer_sent = Some((key, now_s));
        let (lean_steering, value, wild) = key;
        Some(encode(&client_message(C::Steer(v1::SteerIntent {
            seq: self.steer_seq,
            lateral_target_cm: if lean_steering { 0 } else { value },
            lean_steering,
            lean_mrad: if lean_steering { value } else { 0 },
            wild,
        }))))
    }

    /// The junction the own path ends at (the next decision) and its path
    /// distance.
    fn next_junction(&self) -> Option<(Junction, f64)> {
        self.free.as_ref()?.own.as_ref()?.next_junction()
    }

    /// Whether a junction is ahead within the overlay's look-ahead and not
    /// locked yet (the arrow keys then choose the exit, spec §2.2.1).
    pub fn junction_active(&self) -> bool {
        let st = self.predictor.state();
        self.next_junction().is_some_and(|(_, at)| {
            let left = at - st.distance;
            left >= lock_distance(st.speed)
                && left <= JUNCTION_LOOKAHEAD_M.max(JUNCTION_LOOKAHEAD_S * st.speed)
        })
    }

    /// Moves the exit selection of the next junction `steps` exits to the
    /// right (negative: left), spec §2.2.1. Back at the default it is "no
    /// choice". Returns the chosen exit's description.
    pub fn turn(&mut self, steps: i32) -> Option<String> {
        let (j, _) = self.next_junction()?;
        let fr = self.free.as_mut()?;
        let current = freeride::exit_index(&j, fr.choice.as_ref()) as i32;
        let i = (current + steps).clamp(0, j.exits.len() as i32 - 1) as usize;
        self.select_exit(&j, i)
    }

    /// Selects exit `i` (leftmost = 0) of the next junction.
    pub fn choose_exit(&mut self, i: usize) -> Option<String> {
        let (j, _) = self.next_junction()?;
        (i < j.exits.len()).then_some(())?;
        self.select_exit(&j, i)
    }

    /// Back to the default exit ("no choice").
    pub fn clear_exit(&mut self) -> Option<String> {
        let (j, _) = self.next_junction()?;
        self.select_exit(&j, j.default)
    }

    /// Selects the leftmost / straightest / rightmost-like exit by name:
    /// "left", "straight" or "right" (the nearest of that class).
    pub fn choose_direction(&mut self, dir: &str) -> Option<String> {
        let (j, _) = self.next_junction()?;
        let i = match dir {
            "left" => j
                .exits
                .iter()
                .rposition(|e| e.angle_deg < -ocw_world::network::STRAIGHT_DEG)?,
            "right" => j
                .exits
                .iter()
                .position(|e| e.angle_deg > ocw_world::network::STRAIGHT_DEG)?,
            "straight" => (0..j.exits.len()).min_by(|&a, &b| {
                j.exits[a]
                    .angle_deg
                    .abs()
                    .total_cmp(&j.exits[b].angle_deg.abs())
            })?,
            _ => return None,
        };
        self.select_exit(&j, i)
    }

    fn select_exit(&mut self, j: &Junction, i: usize) -> Option<String> {
        let fr = self.free.as_mut()?;
        let e = &j.exits[i];
        fr.choice = (i != j.default).then_some(TurnChoice {
            junction: j.vertex,
            edge: e.edge,
            forward: e.forward,
        });
        let note = match j.route {
            Some(r) if r == i => ", the route",
            Some(_) => ", leaves the route (ride.rejoin returns to it)",
            None if i == j.default => ", the default",
            None => "",
        };
        Some(format!(
            "next junction: {} ({}){note}",
            e.class.name(),
            exit_name(&fr.net, e.edge),
        ))
    }

    /// The exit choice to send, when it changed (spec §6.5: reliable,
    /// whenever the choice changes).
    pub fn turn_message(&mut self) -> Option<Vec<u8>> {
        let (j, _) = self.next_junction()?;
        let fr = self.free.as_mut()?;
        let choice = fr.choice.filter(|c| c.junction == j.vertex);
        let key = (j.vertex, choice);
        if fr.turn_sent == Some(key) {
            return None;
        }
        // "No choice" only needs sending to take back a choice.
        let server_has_choice = matches!(fr.turn_sent, Some((_, Some(_))));
        fr.turn_sent = Some(key);
        if choice.is_none() && !server_has_choice {
            return None;
        }
        fr.turn_seq += 1;
        Some(encode(&client_message(C::Turn(ocw_protocol::turn_intent(
            fr.turn_seq,
            j.vertex,
            choice.as_ref(),
            &fr.net,
        )))))
    }

    /// Manual U-turn (spec §2.2.2): predicted at once; returns the message
    /// for the server. At most every [`UTURN_INTERVAL_S`].
    pub fn u_turn(&mut self, now_s: f64) -> Option<Vec<u8>> {
        let st = self.predictor.state();
        let fr = self.free.as_mut()?;
        if fr
            .last_u_turn_s
            .is_some_and(|t| now_s - t < UTURN_INTERVAL_S)
        {
            return None;
        }
        // Quantized like the message, so the server builds the same loop.
        let (d_cm, v_mm) = (
            (st.distance * 100.0).round().max(0.0) as u32,
            (st.speed * 1000.0).round().max(0.0) as u32,
        );
        let own = fr.own.as_mut()?;
        Arc::make_mut(own).u_turn(f64::from(d_cm) / 100.0, f64::from(v_mm) / 1000.0)?;
        fr.last_u_turn_s = Some(now_s);
        fr.choice = None;
        Some(encode(&client_message(C::UTurn(v1::UTurn {
            distance_cm: d_cm,
            speed_mm_s: v_mm,
        }))))
    }

    /// The junction overlay (spec §2.2.1): the next junction within the
    /// look-ahead, or the one just locked until it is passed.
    pub fn junction_view(&self) -> Option<JunctionView> {
        let fr = self.free.as_ref()?;
        let st = self.predictor.state();
        let d = st.distance;
        if let Some((j, at, chosen)) = &fr.locked
            && d < at + 8.0
        {
            return Some(junction_view(&fr.net, j, *chosen, at - d, true));
        }
        let (j, at) = fr.own.as_ref()?.next_junction()?;
        if at - d > JUNCTION_LOOKAHEAD_M.max(JUNCTION_LOOKAHEAD_S * st.speed) {
            return None;
        }
        let i = freeride::exit_index(&j, fr.choice.as_ref());
        Some(junction_view(&fr.net, &j, i, at - d, false))
    }

    /// Route ride: back to the route after leaving it (console
    /// `ride.rejoin`, key J). Predicted at once; returns the message for the
    /// server, or why it is not possible.
    pub fn rejoin_route(&mut self) -> Result<Vec<u8>, String> {
        let st = self.predictor.state();
        let fr = self.free.as_mut().ok_or("not riding a route")?;
        let own = fr.own.as_mut().ok_or("not riding yet")?;
        // Quantized like the message, so the server finds the same way.
        let (d_cm, v_mm) = (
            (st.distance * 100.0).round().max(0.0) as u32,
            (st.speed * 1000.0).round().max(0.0) as u32,
        );
        Arc::make_mut(own).rejoin_route(f64::from(d_cm) / 100.0, f64::from(v_mm) / 1000.0)?;
        fr.choice = None;
        Ok(encode(&client_message(C::RouteRejoin(v1::RouteRejoin {
            distance_cm: d_cm,
            speed_mm_s: v_mm,
        }))))
    }

    /// Route ride: whether the rider is on the route right now.
    pub fn on_route(&self) -> Option<bool> {
        let own = self.free.as_ref()?.own.as_ref()?;
        own.plan()?;
        Some(own.on_route_at(self.predictor.state().distance))
    }

    /// Route ride: the route's name and progress for the HUD, e.g.
    /// "Rheintal 3.2 / 12.4 km", "Rheintal lap 2 0.4 / 3.1 km", or
    /// "Rheintal (off the route)".
    pub fn route_status(&self) -> Option<String> {
        let fr = self.free.as_ref()?;
        let plan = fr.plan.as_ref()?;
        let Some(own) = fr.own.as_ref() else {
            return Some(plan.name.clone());
        };
        let p = own.route_progress(self.predictor.state().distance)?;
        Some(if p.finished {
            format!("{} (finished)", plan.name)
        } else if !p.on_route {
            format!("{} (off the route)", plan.name)
        } else if plan.is_loop {
            format!(
                "{} lap {} {:.1} / {:.1} km",
                plan.name,
                p.lap + 1,
                p.done_m / 1000.0,
                p.length_m / 1000.0
            )
        } else {
            format!(
                "{} {:.1} / {:.1} km",
                plan.name,
                p.done_m / 1000.0,
                p.length_m / 1000.0
            )
        })
    }

    /// A message for the rider: "Dead end in 800 m", "End of region",
    /// "U-turn" (spec §2.2.2); on a route ride also that the rider left it.
    pub fn notice(&self) -> Option<String> {
        let own = self.free.as_ref()?.own.as_ref()?;
        let d = self.predictor.state().distance;
        if own.in_turnaround(d) {
            return Some("U-turn".into());
        }
        if own.plan().is_some() && !own.on_route_at(d) {
            let progress = own.route_progress(d);
            let back = matches!(own.route_state(), Some(RouteState::On { .. }));
            if !progress.is_some_and(|p| p.finished) {
                return Some(if back {
                    "Off the route - heading back to it".into()
                } else {
                    "Off the route - press J (ride.rejoin) to return".into()
                });
            }
        }
        let round = |m: f64| (m / 10.0).round() * 10.0;
        match own.turnaround_ahead(d) {
            Some((at, TurnReason::Boundary)) if at - d < DEAD_END_WARNING_M => Some(format!(
                "End of region in {:.0} m - install the neighbouring package to ride on",
                round(at - d)
            )),
            Some((at, TurnReason::Manual)) => Some(format!("U-turn in {:.0} m", round(at - d))),
            _ => own
                .dead_end_ahead(d)
                .filter(|m| *m < DEAD_END_WARNING_M)
                .map(|m| format!("Dead end in {:.0} m", round(m.max(0.0)))),
        }
    }

    /// Every rider's free-ride path (for rendering).
    pub fn paths(&self) -> BTreeMap<RiderId, Arc<RidePath>> {
        let Some(fr) = &self.free else {
            return BTreeMap::new();
        };
        let mut out = fr.others.clone();
        if let (Some(id), Some(own)) = (self.rider_id, &fr.own) {
            out.insert(id, own.clone());
        }
        out
    }

    /// One fixed physics tick of the local prediction.
    pub fn predict_step(&mut self) {
        if self.rider_id.is_none() {
            return;
        }
        if self.event.holds(self.rider_id) || self.god.active() || self.was_down {
            // The countdown: the server holds the field at the start line;
            // god mode: the rider is out of the physics (spec §14 Phase
            // 13E); down after a crash: the server's state (Phase 12C).
            self.prev_state = self.predictor.state();
            return;
        }
        let me = self.predictor.state();
        // Free ride: extend the own path like the server (lock point).
        if let Some(fr) = &mut self.free
            && let Some(own) = &mut fr.own
        {
            let d = me.distance;
            if own.needs_extend(d, me.speed) {
                let pending = own.next_junction();
                let (changed, used) =
                    freeride::advance(Arc::make_mut(own), d, me.speed, fr.choice.as_ref());
                if let (Some((j, at)), Some(_)) = (pending, changed)
                    && d >= at - lock_distance(me.speed)
                {
                    let chosen = freeride::exit_index(&j, fr.choice.as_ref());
                    fr.locked = Some((j, at, chosen));
                }
                if used {
                    fr.choice = None;
                }
            }
            if own.needs_trim(d) {
                Arc::make_mut(own).trim_behind(d);
            }
        }
        let others: Vec<(f64, f64)> = match self
            .free
            .as_ref()
            .and_then(|f| f.own.as_ref().map(|o| (f, o)))
        {
            Some((fr, own)) => freeride::others_on_path(
                own,
                me.distance,
                self.others_now.iter().filter_map(|(id, s)| {
                    fr.others
                        .get(id)
                        .map(|p| (p.as_ref(), s.distance, s.lateral_offset))
                }),
            ),
            None => self
                .others_now
                .iter()
                .map(|(_, o)| (o.distance, o.lateral_offset))
                .collect(),
        };
        self.avoid_step(&me);
        let drag = ocw_sim::drafting::drag_factor((me.distance, me.lateral_offset), others);
        self.predictor.set_drag_factor(drag);
        self.prev_state = self.predictor.state();
        let power = self.power_w();
        let track: &dyn Track = match self.free.as_ref().and_then(|f| f.own.as_deref()) {
            Some(p) => p,
            None => &self.route,
        };
        self.predictor.step(track, power);
        for (_, o) in &mut self.others_now {
            o.distance += o.speed * ocw_core::SIM_DT;
        }
    }

    /// Riders to render at client time `now_s`: the local rider first
    /// (predicted), then the others (interpolated, slightly in the past).
    /// `alpha` (0–1) is how far the frame lies between the last two physics
    /// steps (Bevy's `Time<Fixed>::overstep_fraction`).
    pub fn view(&self, now_s: f64, alpha: f64) -> Vec<RiderView> {
        let Some(own) = self.rider_id else {
            return Vec::new();
        };
        let tick_now = self.server_tick_now(now_s);
        let mut riders = vec![RiderView {
            id: own,
            name: self
                .names
                .get(&own)
                .map(|n| n.0.clone())
                .unwrap_or_default(),
            state: lerp_state(self.prev_state, self.predictor.state(), alpha),
            power_w: self.power_w(),
            cadence_rpm: self.cadence_rpm,
            heart_rate_bpm: self.heart_rate_bpm,
            is_local: true,
            // The own bike and look are known locally.
            bike: self.profile.bike,
            look: Some(self.look),
            parts: self.rider_parts(own),
            crash: self.collisions.crash(own, now_s),
            wobble: self.collisions.wobble(own, tick_now),
        }];
        for (id, state) in self.others.sample(now_s - INTERPOLATION_DELAY_S) {
            // Free ride: only riders whose path is known can be placed.
            if let Some(fr) = &self.free
                && !fr.others.contains_key(&id)
            {
                continue;
            }
            let (name, power_w) = self.names.get(&id).cloned().unwrap_or_default();
            let (bike, look) = self.looks.get(&id).copied().unwrap_or_default();
            let look = self.with_character(id, look);
            riders.push(RiderView {
                id,
                name,
                state,
                power_w,
                cadence_rpm: None,
                heart_rate_bpm: None,
                is_local: false,
                bike,
                look,
                parts: self.rider_parts(id),
                crash: self.collisions.crash(id, now_s),
                // Others are drawn slightly in the past.
                wobble: self
                    .collisions
                    .wobble(id, tick_now - INTERPOLATION_DELAY_S / ocw_core::SIM_DT),
            });
        }
        riders
    }

    /// Autodrive avoidance of the own rider (spec §14 Phase 12D, the same
    /// code as the server's): its line and virtual brake from the others'
    /// last known positions; manual riders get no help.
    fn avoid_step(&mut self, me: &RiderState) {
        let base = self.lateral_target.unwrap_or(me.lateral_offset);
        if !self.collisions.mode().on() || self.lean.is_some() || self.wild {
            self.predictor.set_speed_cap(None);
            if self.lean.is_none() && self.avoid.pass_m.take().is_some() {
                self.predictor.set_lateral_target(base);
            }
            return;
        }
        let loop_m = (self.free.is_none() && self.route.is_loop()).then(|| self.route.length());
        let own_path = self
            .free
            .as_ref()
            .and_then(|f| f.own.as_ref().map(|o| (f, o)));
        let mut others = Vec::new();
        for (id, o) in &self.others_now {
            let mine = match own_path {
                Some((fr, own)) => {
                    let Some(p) = fr.others.get(id) else { continue };
                    let Some((e, f, off)) = p.locate(o.distance) else {
                        continue;
                    };
                    let Some(m) = own.distance_of(e, f, off, me.distance) else {
                        continue;
                    };
                    m
                }
                None => o.distance,
            };
            let mut ahead = mine - me.distance;
            if let Some(l) = loop_m {
                ahead -= l * (ahead / l).round();
            }
            if ahead.abs() > 80.0 {
                continue;
            }
            others.push(if self.collisions.crashed(*id) {
                ocw_sim::avoid::Other::fallen(ahead, o.lateral_offset)
            } else {
                ocw_sim::avoid::Other::rider(ahead, o.lateral_offset, o.lateral_offset, o.speed)
            });
        }
        let width = self.track().road_width_at(me.distance);
        let left = self.free.as_ref().is_some_and(|f| f.net.keep_left);
        let plan = ocw_sim::avoid::plan(
            &ocw_sim::avoid::Me {
                lateral_m: me.lateral_offset,
                speed: me.speed,
                lane_m: base,
                limit_m: ocw_sim::physics::lateral_limit(width),
                road_width_m: width,
            },
            &others,
            ocw_sim::avoid::overtake_left(left),
            &mut self.avoid,
        );
        self.predictor.set_lateral_target(plan.target_m);
        self.predictor.set_speed_cap(plan.speed_cap);
    }

    /// The collision model in force (spec §14 Phase 12A, from the server).
    pub fn collision_mode(&self) -> ocw_sim::collide::Mode {
        self.collisions.mode()
    }

    /// Chaotic bots on (Phase 12 Q3, from the server).
    pub fn chaotic_bots(&self) -> bool {
        self.collisions.chaotic_bots()
    }

    /// Whether this rider may change the collision rules (the server
    /// checks again).
    pub fn may_change_rules(&self) -> bool {
        self.collisions
            .access()
            .is_none_or(|a| a.allows(self.rider_id, self.clock_info.and_then(|c| c.host)))
    }

    /// The rules request (`ride.collisions`, `ride.chaotic_bots`).
    pub fn rules_message(collisions: Option<bool>, chaotic_bots: Option<bool>) -> Vec<u8> {
        encode(&ocw_protocol::rules_request(collisions, chaotic_bots))
    }

    /// A staged contact (`dev.bump` / `dev.crash`, local servers only).
    pub fn stage_message(crash: bool) -> Vec<u8> {
        encode(&ocw_protocol::stage_contact(crash))
    }

    /// The own rider's crash at client time `now_s`, if down.
    pub fn own_crash(&self, now_s: f64) -> Option<ocw_sim::collide::CrashView> {
        self.collisions.crash(self.rider_id?, now_s)
    }

    /// The HUD's crash line ("Crash! ... back on the bike in 3.2 s").
    pub fn crash_notice(&self, now_s: f64) -> Option<String> {
        self.collisions.notice(self.rider_id, now_s)
    }

    /// The crash feed of the last seconds ("R. Rider crashed at km 12.4").
    pub fn crash_feed(&self, now_s: f64) -> Vec<String> {
        self.collisions.feed(now_s)
    }

    /// The collision state (bumps and crashes of every rider, for sounds).
    pub fn collision_state(&self) -> &crate::collisions::CollisionClient {
        &self.collisions
    }

    /// The proximity cue setting (`ride.proximity_cue`, off by default).
    pub fn set_proximity_cue(&mut self, on: bool) {
        self.proximity_cue = on;
    }

    pub fn proximity_cue_on(&self) -> bool {
        self.proximity_cue
    }

    /// The proximity cue now (12D): the side of a rider close beside (-1
    /// left, +1 right) or 0 in front; `None` when nobody is close, the cue
    /// is off or there are no collisions.
    pub fn proximity(&self) -> Option<i32> {
        if !self.proximity_cue || !self.collisions.mode().on() {
            return None;
        }
        let me = self.predictor.state();
        let loop_m = (self.free.is_none() && self.route.is_loop()).then(|| self.route.length());
        crate::collisions::proximity(self.others_now.iter().filter_map(|(_, o)| {
            if self.free.is_some() {
                return None;
            }
            let mut ahead = o.distance - me.distance;
            if let Some(l) = loop_m {
                ahead -= l * (ahead / l).round();
            }
            Some((ahead, o.lateral_offset - me.lateral_offset))
        }))
    }

    /// The server tick now, estimated from the last snapshot.
    fn server_tick_now(&self, now_s: f64) -> f64 {
        self.last_snapshot_at
            .map_or(0.0, |(t, at)| crate::event::estimate_tick(t, at, now_s))
    }

    /// The event's HUD view at client time `now_s` (spec §2.3).
    pub fn event_view(&self, now_s: f64) -> Option<ocw_render::EventView> {
        let others: Vec<f64> = self.others_now.iter().map(|(_, s)| s.distance).collect();
        let mut v = self.event.view(
            now_s,
            self.server_tick_now(now_s),
            self.rider_id,
            self.predictor.state().distance,
            &others,
        )?;
        // The event's feed (spec §14 Phase 12C): "R. Rider crashed at km 12.4".
        v.lines.extend(self.collisions.feed(now_s));
        Some(v)
    }

    /// The own rider's part of the event, for the FIT laps
    /// ([`EventClient::lap_segment`]).
    pub fn event_lap_segment(&self) -> Option<(u32, u32)> {
        self.event.lap_segment(self.rider_id)
    }

    /// The current event as the server last sent it.
    pub fn event_info(&self) -> Option<&ocw_protocol::EventInfo> {
        self.event.info()
    }

    /// The time trial's ghost at client time `now_s`, beside the rider's
    /// line (it is not on the server, so nobody drafts it).
    pub fn ghost_state(&self, now_s: f64) -> Option<RiderState> {
        let tick = self.server_tick_now(now_s);
        let d = self.event.ghost_distance(tick, self.rider_id)?;
        let half = 0.25 / ocw_core::SIM_DT;
        let speed = match (
            self.event.ghost_distance(tick - half, self.rider_id),
            self.event.ghost_distance(tick + half, self.rider_id),
        ) {
            (Some(a), Some(b)) => (b - a) / 0.5,
            _ => 0.0,
        };
        Some(RiderState {
            distance: d,
            speed,
            lateral_offset: GHOST_LATERAL_M,
            ..Default::default()
        })
    }

    /// The time trial's ghost to load, if not loaded yet.
    pub fn wants_ghost(&self) -> Option<GhostKey> {
        self.event.wants_ghost()
    }

    pub fn set_ghost(&mut self, key: GhostKey, ghost: Option<ocw_sim::GhostTrace>) {
        self.event.set_ghost(key, ghost);
    }

    /// A finished time trial's effort, to keep if it is the best.
    pub fn take_finished_trace(&mut self) -> Option<(GhostKey, ocw_sim::GhostTrace)> {
        self.event.take_finished_trace()
    }

    pub fn set_ghost_outcome(&mut self, outcome: GhostOutcome) {
        self.event.set_outcome(outcome);
    }

    /// Whether the results panel is up.
    pub fn results_showing(&self, now_s: f64) -> bool {
        self.event.results_showing(now_s)
    }

    /// Closes the results panel (Esc / A); false if none was up.
    pub fn dismiss_results(&mut self, now_s: f64) -> bool {
        self.event.dismiss_results(now_s)
    }

    /// Whether the results panel takes the input at `now_s` (up, or closed
    /// in this very frame: the key that closed it does nothing else).
    pub fn results_input(&self, now_s: f64) -> bool {
        self.event.results_input(now_s)
    }

    /// The request to start an event (the local server decides, spec §2.3).
    pub fn event_start_message(spec: &ocw_sim::EventSpec, bots: Option<u32>) -> Vec<u8> {
        encode(&client_message(C::Event(v1::EventRequest {
            req: Some(v1::event_request::Req::Start(ocw_protocol::event_start(
                spec, bots,
            ))),
        })))
    }

    /// The request to start an event with the organiser's race mode
    /// (`None`: by its kind – Profile for races and group rides, Open for
    /// time trials; spec §14 Phase 4, 4G).
    pub fn event_start_message_with(
        spec: &ocw_sim::EventSpec,
        bots: Option<u32>,
        physics: Option<ocw_sim::PhysicsMode>,
    ) -> Vec<u8> {
        encode(&client_message(C::Event(v1::EventRequest {
            req: Some(v1::event_request::Req::Start(
                ocw_protocol::event_start_with(spec, bots, physics),
            )),
        })))
    }

    /// `dev.bot_bike` (spec §14 Phase 4, W3; local servers): a bot's
    /// bike and parts, steady power, lane and name.
    pub fn bot_bike_message(bike: v1::BotBike) -> Vec<u8> {
        encode(&client_message(C::Dev(v1::DevCommand {
            cmd: Some(v1::dev_command::Cmd::BotBike(bike)),
        })))
    }

    /// The request to cancel the current event.
    pub fn event_cancel_message() -> Vec<u8> {
        encode(&client_message(C::Event(v1::EventRequest {
            req: Some(v1::event_request::Req::Cancel(true)),
        })))
    }

    pub fn route(&self) -> &Route {
        &self.route
    }
    /// The server issues voice tokens (spec §7); false before the welcome.
    pub fn voice_offered(&self) -> bool {
        self.voice_offered
    }

    /// Riders of connected clients in the session, the own one included
    /// (0 before the first snapshot or from an old server).
    pub fn human_riders(&self) -> u32 {
        self.human_riders
    }

    /// A voice token request for the server.
    pub fn voice_request_message(req: v1::VoiceTokenRequest) -> Vec<u8> {
        encode(&client_message(C::Voice(req)))
    }

    /// The echo test's request (spec §7 *Echo bot*).
    pub fn echo_request_message(on: bool, here: bool) -> Vec<u8> {
        encode(&ocw_protocol::echo_request(on, here))
    }

    /// The server lets riders put an Echo rider on the road.
    pub fn echo_bot_allowed(&self) -> bool {
        self.echo_bot
    }

    /// The session's Echo rider (from the last snapshot).
    pub fn echo_rider(&self) -> Option<RiderId> {
        self.echo_rider
    }

    pub fn rider_id(&self) -> Option<RiderId> {
        self.rider_id
    }
    pub fn state(&self) -> RiderState {
        self.predictor.state()
    }
    /// The rider's power: the device's plus the keyboard's (↑).
    pub fn power_w(&self) -> f64 {
        self.power_w + self.key_power_w
    }

    /// Extra power from the keyboard (↑ / ↓), added to the device's.
    /// The power device is virtual (the virtual trainer, `sim.power`):
    /// samples say so and earn no hearts (spec §14 Phase 4, 4E).
    pub fn set_virtual_device(&mut self, on: bool) {
        self.virtual_device = on;
    }

    /// Whether the power sent is virtual: the virtual trainer, or keyboard
    /// power on top of a real device.
    pub fn virtual_power(&self) -> bool {
        self.virtual_device || self.key_power_w > 0.0
    }

    /// Hearts the server credited the own rider in this session so far
    /// (spec §14 Phase 4, 4E): a running total from the snapshots.
    pub fn server_hearts(&self) -> u32 {
        self.hearts
    }

    /// The own physics profile (weight and bike).
    pub fn profile(&self) -> RiderProfile {
        self.profile
    }

    /// Whether a bike change now waits for the end of an event (the own
    /// rider is entered in one that is not over: from the next start).
    pub fn bike_change_waits(&self) -> bool {
        self.event.info().is_some_and(|i| self.in_event(i))
    }

    /// The workshop's bike when the rider leaves it (spec §14 Phase 4,
    /// 4A, Q1, 4G): the message to send. Outside events the prediction
    /// switches to the new profile and build at once, as the server does;
    /// in an event that is not over ([`ClientCore::bike_change_waits`])
    /// the server applies it when the event ends and says so with a
    /// `RiderBuild`, which the prediction then follows. The look is
    /// cosmetic and applies at once.
    pub fn change_bike(
        &mut self,
        bike_id: &str,
        bike: ProfileId,
        look: RiderLook,
        parts: &[String],
    ) -> Vec<u8> {
        if !self.bike_change_waits() {
            self.profile = RiderProfile::curated(bike, self.profile.rider_mass_kg);
            self.set_own_build(bike, parts);
            self.refresh_physics();
        }
        self.look = look;
        encode(&ocw_protocol::bike_change(bike_id, bike, look, parts))
    }

    pub fn key_power_w(&self) -> f64 {
        self.key_power_w
    }

    /// Sets the keyboard power; returns a sample to send when it changed
    /// (the next device sample may be 250 ms away).
    pub fn set_key_power(&mut self, watts: f64) -> Option<Vec<u8>> {
        let watts = watts.max(0.0);
        if (watts - self.key_power_w).abs() < 1e-9 {
            return None;
        }
        self.key_power_w = watts;
        Some(self.sample())
    }
    pub fn cadence_rpm(&self) -> Option<f64> {
        self.cadence_rpm
    }
    pub fn heart_rate_bpm(&self) -> Option<f64> {
        self.heart_rate_bpm
    }
    pub fn prediction_error(&self) -> f64 {
        self.predictor.last_error()
    }
    pub fn stats(&self) -> &NetStats {
        &self.stats
    }

    /// God mode (spec §14 Phase 13): the own flight.
    pub fn god(&self) -> &crate::god_state::GodCore {
        &self.god
    }

    pub fn god_mut(&mut self) -> &mut crate::god_state::GodCore {
        &mut self.god
    }

    /// A god-mode change the snapshots brought (taken once).
    pub fn take_god_notice(&mut self) -> Option<crate::god_state::GodEvent> {
        self.god_notice.take()
    }

    /// Whether this rider may enter god mode (the server checks it
    /// again): anyone on a local server, the host of a shared ride by
    /// default, nobody in an organizer's session.
    pub fn may_use_god(&self) -> bool {
        let host = self.clock_info.and_then(|c| c.host);
        self.god_access
            .is_none_or(|a| a.allows(self.rider_id, host))
    }

    /// Where leaving god mode over (x, z) lands (spec §14 Phase 13A): the
    /// nearest road of the graph in free ride, the nearest point of the
    /// route on a route (the road surface, local metres).
    pub fn landing_spot(&self, x: f64, z: f64) -> Option<[f64; 3]> {
        let lift = f64::from(ocw_render::ROAD_SURFACE_LIFT_M);
        let p = match &self.free {
            Some(fr) => match &fr.plan {
                Some(plan) => ocw_sim::god::nearest_on_plan(&fr.net, plan, [x, z])?.1,
                None => {
                    let (edge, offset) = fr.net.nearest([x, z])?;
                    fr.net.geometry(edge).point_at(offset)
                }
            },
            None => ocw_sim::god::nearest_on_polyline(self.route.points(), [x, z])?.1,
        };
        Some([p[0], p[1] + lift, p[2]])
    }

    /// The unicorns to draw at `now_s` (spec §14 Phase 13C): the own
    /// (flying or gliding) and the other riders' in god mode.
    pub fn unicorns(&self, now_s: f64) -> Vec<ocw_render::UnicornView> {
        let colour = |id: RiderId, local: bool| {
            if local {
                self.look.jersey
            } else {
                self.looks.get(&id).and_then(|l| l.1).map_or(
                    ocw_core::bike::PALETTE[id.0 as usize % ocw_core::bike::PALETTE.len()],
                    |l| l.jersey,
                )
            }
        };
        let name = |id: RiderId| self.names.get(&id).map(|n| n.0.clone()).unwrap_or_default();
        let view =
            |id: RiderId, u: crate::god_state::LocalUnicorn, local: bool| ocw_render::UnicornView {
                id,
                name: name(id),
                is_local: local,
                pos: u.pose.pos,
                yaw: u.pose.yaw,
                pitch: u.pose.pitch,
                speed_mps: u.pose.speed,
                climb_mps: u.climb_mps,
                turn_rate: u.turn_rate,
                accel: u.accel,
                age_s: u.age_s,
                landing: u.leaving,
                colour: colour(id, local),
            };
        let mut out = Vec::new();
        if let (Some(me), Some(u)) = (self.rider_id, self.god.unicorn(now_s)) {
            out.push(view(me, u, true));
        }
        for (id, u) in self.god_others.sample(now_s, INTERPOLATION_DELAY_S) {
            out.push(view(id, u, false));
        }
        out
    }
}

/// Street name or ref of an edge, or its road class.
fn exit_name(net: &RoadNetwork, edge: usize) -> String {
    let e = net.edge(edge);
    e.name
        .clone()
        .unwrap_or_else(|| format!("{:?}", e.class).to_lowercase())
}

fn junction_view(
    net: &RoadNetwork,
    j: &Junction,
    selected: usize,
    distance_m: f64,
    locked: bool,
) -> JunctionView {
    JunctionView {
        distance_m,
        exits: j
            .exits
            .iter()
            .map(|e| ExitView {
                angle_deg: e.angle_deg,
                name: exit_name(net, e.edge),
                dead_end: e.dead_end,
                unpaved: e.unpaved,
                steep: e.steep,
            })
            .collect(),
        selected,
        default: j.default,
        locked,
        route: j.route,
    }
}

/// Linear interpolation of a rider state (`t` clamped to 0–1).
fn lerp_state(a: RiderState, b: RiderState, t: f64) -> RiderState {
    let t = t.clamp(0.0, 1.0);
    RiderState {
        distance: a.distance + (b.distance - a.distance) * t,
        speed: a.speed + (b.speed - a.speed) * t,
        lateral_offset: a.lateral_offset + (b.lateral_offset - a.lateral_offset) * t,
        heading: a.heading + (b.heading - a.heading) * t,
        turn_curvature: a.turn_curvature + (b.turn_curvature - a.turn_curvature) * t,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use ocw_server::{GameServer, ServerConfig};

    use super::*;

    fn route() -> Route {
        Route::test_loop()
    }

    fn power(watts: f64, t: f64) -> DeviceEvent {
        DeviceEvent::Power {
            watts,
            cadence_rpm: Some(90.0),
            timestamp: Duration::from_secs_f64(t),
        }
    }

    /// A client connected to an in-memory `GameServer`, driven tick by tick.
    struct Harness {
        server: GameServer,
        conn: ocw_server::ConnectionId,
        client: ClientCore,
        notices: Vec<Notice>,
        time_s: f64,
    }

    /// The host's world switch in a shared ride (spec §3.2.1): only the
    /// host may (a quick check here, the server's is the rule); when the
    /// server switched, the client leaves the old session and is told.
    #[test]
    fn a_world_change_ends_the_joined_session() {
        let config = || ServerConfig {
            world_access: ocw_protocol::ClockAccess::Host,
            ..Default::default()
        };
        let mut host = Harness::with(
            GameServer::new(route(), config()),
            ClientCore::new(route(), "Host", RiderProfile::default()),
        );
        host.ride(100.0, 0.2);
        assert!(host.client.may_switch_world(), "the only rider is the host");
        assert_eq!(host.client.world_control(), "you");
        // A second client on the same server is a guest.
        let conn = host.server.connect();
        let mut guest = ClientCore::new(route(), "Guest", RiderProfile::default());
        for reply in host.server.handle(conn, &guest.hello()) {
            guest.on_server_message(&reply, 0.0);
        }
        for b in (0..5).flat_map(|_| host.server.tick()) {
            guest.on_server_message(&b, 0.1);
        }
        assert!(guest.rider_id().is_some());
        assert!(!guest.may_switch_world());
        assert_eq!(guest.world_control(), "the host");
        let (conns, bytes) = host.server.switch_world(
            route(),
            v1::WorldChange {
                package_id: "europe/blackforest".into(),
                version: 1,
                ..Default::default()
            },
        );
        assert_eq!(conns.len(), 2);
        let n = guest.on_server_message(&bytes, 0.2);
        assert!(
            matches!(&n, Some(Notice::WorldChange(c)) if c.package_id == "europe/blackforest"),
            "{n:?}"
        );
        assert_eq!(guest.rider_id(), None, "left the ended session");
        // Without a welcome yet (a local server before joining): no limit.
        let fresh = ClientCore::new(route(), "New", RiderProfile::default());
        assert!(fresh.may_switch_world());
    }

    impl Harness {
        fn new(bots: u32, allow_dev: bool) -> Self {
            let server = GameServer::new(
                route(),
                ServerConfig {
                    allow_dev_commands: allow_dev,
                    bots,
                    start_m: 0.0,
                    allow_local_events: true,
                    voice: None,
                    ..Default::default()
                },
            );
            let client = ClientCore::new(route(), "Tester", RiderProfile::default());
            Self::with(server, client)
        }

        /// Free ride on the grid (see [`grid`]), starting 50 m east of
        /// vertex 1 on the top row, riding east.
        fn free(bots: u32) -> Self {
            let net = Arc::new(RoadNetwork::new(grid()));
            let edge = (0..net.graph().edges.len())
                .find(|&e| net.edge(e).from.0 == 1 && net.edge(e).to.0 == 2)
                .unwrap();
            let server = GameServer::new(
                ocw_server::Course::FreeRide {
                    net: net.clone(),
                    start: ocw_sim::FreeRideStart {
                        edge,
                        offset_m: 50.0,
                        forward: true,
                    },
                },
                ServerConfig {
                    allow_dev_commands: true,
                    bots,
                    start_m: 0.0,
                    allow_local_events: true,
                    voice: None,
                    ..Default::default()
                },
            );
            Self::with(
                server,
                ClientCore::free_ride(net, "Free", RiderProfile::default()),
            )
        }

        fn with(mut server: GameServer, client: ClientCore) -> Self {
            let conn = server.connect();
            let mut h = Self {
                server,
                conn,
                client,
                notices: Vec::new(),
                time_s: 0.0,
            };
            let hello = h.client.hello();
            h.send(&hello);
            h
        }

        fn send(&mut self, bytes: &[u8]) {
            for reply in self.server.handle(self.conn, bytes) {
                self.receive(&reply);
            }
        }

        fn receive(&mut self, bytes: &[u8]) {
            if let Some(n) = self.client.on_server_message(bytes, self.time_s) {
                self.notices.push(n);
            }
        }

        /// Runs `seconds` of simulated time at the real rates: 50 Hz server and
        /// prediction, 4 Hz trainer samples.
        fn ride(&mut self, watts: f64, seconds: f64) {
            let ticks = (seconds * f64::from(ocw_core::SIM_TICK_HZ)).round() as u32;
            for _ in 0..ticks {
                if (self.time_s * 4.0).fract() < 1e-9
                    && let Some(sample) = self.client.on_device_event(power(watts, self.time_s))
                {
                    self.send(&sample);
                }
                for msg in self.server.tick() {
                    self.receive(&msg);
                }
                self.client.predict_step();
                self.time_s += ocw_core::SIM_DT;
                // Avoid drift so the 4 Hz check stays exact.
                self.time_s = (self.time_s * 1000.0).round() / 1000.0;
            }
        }
    }

    /// The world clock end to end (spec §14 Phase 9): the server's clock
    /// reaches the client with the welcome and the snapshots, the client
    /// predicts it between them, and a time-machine request changes it for
    /// the client within a snapshot.
    #[test]
    fn world_clock_reaches_the_client_and_the_time_machine_changes_it() {
        let server = GameServer::new(
            route(),
            ServerConfig {
                real_epoch_ms: Some(1_781_000_000_000),
                ..Default::default()
            },
        );
        let mut h = Harness::with(
            server,
            ClientCore::new(route(), "Tester", RiderProfile::default()),
        );
        assert_eq!(h.client.world_ms(0.0), Some(1_781_000_000_000));
        assert!(h.client.may_change_clock(), "the only rider is the host");
        h.ride(0.0, 2.0);
        let now = h.client.world_ms(h.time_s).unwrap();
        assert!((now - (1_781_000_000_000 + 2000)).abs() <= 20, "{now}");
        // x600 from a fixed instant.
        let fixed = 1_781_000_000_000 + 86_400_000;
        h.send(&ClientCore::clock_message(ocw_sky::ClockChange::Fixed(
            fixed,
        )));
        h.send(&ClientCore::clock_message(ocw_sky::ClockChange::Speed(
            600.0,
        )));
        assert!(
            matches!(h.notices.last(), Some(Notice::CommandResult(m)) if m == "world clock: fixed, x600"),
            "{:?}",
            h.notices.last()
        );
        h.ride(0.0, 1.0);
        let info = *h.client.clock_info().unwrap();
        assert_eq!((info.speed, info.mode), (600.0, ocw_sky::ClockMode::Fixed));
        let now = h.client.world_ms(h.time_s).unwrap();
        // One second at x600 = 10 minutes (within a snapshot interval).
        assert!((now - (fixed + 600_000)).abs() <= 600 * 110, "{now}");
    }

    /// Voice (spec §7): the welcome says whether the server offers voice,
    /// snapshots count the human riders, and a token request is answered
    /// with a grant for the client's rider.
    #[test]
    fn voice_offer_human_count_and_grant() {
        let mut h = Harness::new(2, false);
        assert!(!h.client.voice_offered(), "this server has no voice");
        h.ride(100.0, 0.3);
        assert_eq!(h.client.human_riders(), 1, "bots are not counted");
        h.send(&ClientCore::voice_request_message(v1::VoiceTokenRequest {
            channel: v1::VoiceChannel::Session as i32,
            group: String::new(),
        }));
        assert!(
            matches!(h.notices.last(), Some(Notice::Voice(g)) if g.token.is_empty() && !g.error.is_empty())
        );

        let server = GameServer::new(
            route(),
            ServerConfig {
                voice: Some(ocw_server::voice::VoiceConfig::new(
                    "ws://127.0.0.1:7880",
                    "k",
                    "s",
                )),
                ..Default::default()
            },
        );
        let mut h = Harness::with(
            server,
            ClientCore::new(route(), "Tester", RiderProfile::default()),
        );
        assert!(h.client.voice_offered());
        h.send(&ClientCore::voice_request_message(
            v1::VoiceTokenRequest::default(),
        ));
        let Some(Notice::Voice(g)) = h.notices.last() else {
            panic!("no grant: {:?}", h.notices)
        };
        assert_eq!(
            g.identity,
            ocw_protocol::voice_identity(h.client.rider_id().unwrap())
        );
        assert!(!g.token.is_empty() && g.error.is_empty());
    }

    /// The echo test (spec §7 *Echo bot*): the client asks, the Echo rider
    /// appears in the view with its name tag and is known as the echo.
    #[test]
    fn echo_rider_appears_on_request() {
        let server = GameServer::new(
            route(),
            ServerConfig {
                bots: 1,
                allow_echo_bot: true,
                ..Default::default()
            },
        );
        let mut h = Harness::with(
            server,
            ClientCore::new(route(), "Tester", RiderProfile::default()),
        );
        assert!(h.client.echo_bot_allowed());
        h.ride(100.0, 0.3);
        assert_eq!(h.client.echo_rider(), None);
        h.send(&ClientCore::echo_request_message(true, false));
        assert!(
            matches!(h.notices.last(), Some(Notice::CommandResult(m)) if m.contains("Echo rider")),
            "{:?}",
            h.notices.last()
        );
        h.ride(100.0, 0.3);
        let echo = h.client.echo_rider().expect("the Echo rider");
        let view = h.client.view(h.time_s, 1.0);
        let e = view.iter().find(|r| r.id == echo).expect("in the view");
        assert_eq!(e.name, "Echo");
        assert_eq!(h.client.human_riders(), 1, "not counted as a human rider");
        h.send(&ClientCore::echo_request_message(false, false));
        h.ride(100.0, 0.3);
        assert_eq!(h.client.echo_rider(), None);
    }

    #[test]
    fn joins_and_shows_nothing_before_welcome() {
        let client = ClientCore::new(route(), "A", RiderProfile::default());
        assert!(client.view(0.0, 1.0).is_empty());
        let mut h = Harness::new(0, false);
        assert!(matches!(h.notices[0], Notice::Joined { .. }));
        assert!(h.client.rider_id().is_some());
        h.ride(200.0, 0.0);
        assert_eq!(h.client.view(0.0, 1.0).len(), 1);
    }

    /// The rider profile's weight goes to the server in the hello (spec
    /// §3.2.1) and both sides simulate with it, so the prediction agrees.
    #[test]
    fn the_rider_weight_reaches_the_server_and_the_prediction_agrees() {
        let server = GameServer::new(
            route(),
            ServerConfig {
                allow_dev_commands: false,
                bots: 0,
                start_m: 0.0,
                allow_local_events: false,
                voice: None,
                ..Default::default()
            },
        );
        let client = ClientCore::new(route(), "Light", RiderProfile::for_rider_weight(58.0));
        let mut h = Harness::with(server, client);
        let id = h.client.rider_id().expect("joined");
        let p = h.server.session().rider_profile(id).unwrap();
        assert_eq!(p, RiderProfile::for_rider_weight(58.0));
        h.ride(250.0, 20.0);
        let server = h.server.session().rider_state(id).unwrap();
        let client = h.client.state();
        assert!(
            (client.speed - server.speed).abs() < 0.3,
            "client {} m/s, server {} m/s",
            client.speed,
            server.speed
        );
    }

    /// The bike profile reaches the server by its id; server and prediction
    /// ride the same curated profile, so they agree (spec §2.5), for every
    /// profile, on asphalt and on gravel.
    #[test]
    fn every_bike_profile_reaches_the_server_and_the_prediction_agrees() {
        for gravel in [false, true] {
            for bike in ProfileId::ALL {
                let course = if gravel {
                    route().with_surfaces(vec![(0.0, 1.0e5, ocw_world::Surface::Gravel)])
                } else {
                    route()
                };
                let server = GameServer::new(course.clone(), ServerConfig::default());
                let profile = RiderProfile::curated(bike, 68.0);
                let client = ClientCore::new(course, "Bike", profile);
                let mut h = Harness::with(server, client);
                let id = h.client.rider_id().expect("joined");
                assert_eq!(h.server.session().rider_profile(id).unwrap(), profile);
                h.ride(250.0, 20.0);
                let server = h.server.session().rider_state(id).unwrap();
                let client = h.client.state();
                assert!(
                    (client.speed - server.speed).abs() < 0.3,
                    "{bike:?} gravel {gravel}: client {} m/s, server {} m/s",
                    client.speed,
                    server.speed
                );
            }
        }
    }

    /// The workshop's bike change (spec §14 Phase 4, 4A, Q1): the server
    /// and the prediction switch together; none in a running event.
    #[test]
    fn a_bike_change_switches_server_and_prediction() {
        let mut h = Harness::new(0, true);
        let id = h.client.rider_id().unwrap();
        let look = RiderLook {
            frame: ocw_core::Rgb(0x1E88E5),
            ..Default::default()
        };
        assert!(!h.client.bike_change_waits());
        let msg = h
            .client
            .change_bike("mtb", ProfileId::Mountain, look, &["tyres.mtb.xc".into()]);
        h.send(&msg);
        let p = h.server.session().rider_profile(id).unwrap();
        assert_eq!(p.bike, ProfileId::Mountain);
        // The XC tyres count (spec 4G): server and prediction alike.
        assert!(p.crr < RiderProfile::curated(ProfileId::Mountain, 75.0).crr);
        assert_eq!(h.client.effective_profile(), p);
        assert_eq!(h.client.profile().bike, ProfileId::Mountain);
        h.ride(250.0, 10.0);
        let server = h.server.session().rider_state(id).unwrap();
        assert!((h.client.state().speed - server.speed).abs() < 0.3);
        let view = h.client.view(h.time_s, 1.0);
        assert_eq!(view[0].bike, ProfileId::Mountain);
        assert_eq!(view[0].look, Some(look));
    }

    /// Hearts (spec §14 Phase 4, 4E): the server's running total reaches
    /// the client; virtual power is flagged and earns none.
    #[test]
    fn hearts_come_from_the_server_and_not_from_virtual_power() {
        let mut h = Harness::new(0, true);
        assert!(!h.client.virtual_power());
        h.ride(400.0, 40.0);
        let real = h.client.server_hearts();
        assert!(real >= 1, "{real}");
        h.client.set_virtual_device(true);
        assert!(h.client.virtual_power());
        h.ride(400.0, 40.0);
        assert_eq!(
            h.client.server_hearts(),
            real,
            "virtual power earns nothing"
        );
        h.client.set_virtual_device(false);
        h.client.set_key_power(100.0);
        assert!(h.client.virtual_power(), "keyboard power is virtual");
    }

    /// The own look goes out in the hello; the others' looks come back in
    /// the snapshots and reach the render view with their bikes.
    #[test]
    fn looks_reach_the_view() {
        let server = GameServer::new(
            route(),
            ServerConfig {
                bots: 3,
                ..Default::default()
            },
        );
        let look = RiderLook {
            frame: ocw_core::Rgb(0x00FF00),
            ..Default::default()
        };
        let client = ClientCore::new(
            route(),
            "Styled",
            RiderProfile::curated(ProfileId::TimeTrial, 75.0),
        )
        .with_look(look);
        let mut h = Harness::with(server, client);
        h.ride(150.0, 2.0);
        let view = h.client.view(h.time_s, 1.0);
        assert_eq!(view[0].look, Some(look));
        assert_eq!(view[0].bike, ProfileId::TimeTrial);
        let others: Vec<_> = view[1..].iter().map(|r| (r.bike, r.look)).collect();
        assert_eq!(others.len(), 3);
        for (i, (bike, look)) in others.iter().enumerate() {
            let (b, l) = ocw_server::bot_bike(i as u32);
            assert_eq!((*bike, *look), (b, Some(l)));
        }
    }

    /// Setting `controls.autodrive off`: the ride starts with lean steering.
    #[test]
    fn autodrive_can_be_off_at_the_start() {
        let server = GameServer::new(route(), ServerConfig::default());
        let client = ClientCore::new(route(), "Manual", RiderProfile::default())
            .with_autodrive_at_start(false);
        let mut h = Harness::with(server, client);
        assert!(!h.client.autodrive());
        h.ride(200.0, 1.0);
        assert!(!h.client.autodrive());
        assert!(Harness::new(0, false).client.autodrive());
    }

    #[test]
    fn device_samples_have_increasing_sequence_numbers() {
        let mut c = ClientCore::new(route(), "A", RiderProfile::default());
        for i in 1..=3u32 {
            let bytes = c.on_device_event(power(150.0, f64::from(i))).unwrap();
            match ocw_protocol::decode_client(&bytes).unwrap() {
                C::Sample(s) => {
                    assert_eq!(s.seq, i);
                    assert_eq!(s.power_w, Some(150.0));
                }
                other => panic!("unexpected {other:?}"),
            }
        }
        assert_eq!(c.stats().samples_out, 3);
        assert!(
            c.on_device_event(DeviceEvent::HeartRate {
                bpm: 120.0,
                timestamp: Duration::ZERO,
            })
            .is_none()
        );
        // Heart rate is sent along with the next power sample.
        let bytes = c.on_device_event(power(150.0, 4.0)).unwrap();
        match ocw_protocol::decode_client(&bytes).unwrap() {
            C::Sample(s) => assert_eq!(s.heart_rate_bpm, Some(120.0)),
            other => panic!("unexpected {other:?}"),
        }
        assert!(c.on_device_event(DeviceEvent::Disconnected).is_none());
    }

    /// Desktop client ↔ server through the real protocol (spec §14, Phase 1).
    #[test]
    fn prediction_tracks_server_over_a_long_ride() {
        let mut h = Harness::new(3, false);
        h.ride(250.0, 60.0);
        let own = h.client.rider_id().unwrap();
        let server = h.server.session().rider_state(own).unwrap();
        let predicted = h.client.state();
        assert!(
            predicted.distance > 300.0,
            "rode {:.0} m",
            predicted.distance
        );
        // Same physics on both sides: only quantization and the sub-snapshot
        // tick offset remain.
        assert!(
            (server.distance - predicted.distance).abs() < 0.5,
            "server {:.2} m, predicted {:.2} m",
            server.distance,
            predicted.distance
        );
        assert!(h.client.prediction_error().abs() < 0.5);
        assert!(h.client.stats().snapshots >= 590);
        assert_eq!(h.client.stats().malformed, 0);
    }

    #[test]
    fn view_contains_local_rider_first_and_interpolated_bots() {
        let mut h = Harness::new(3, false);
        h.ride(200.0, 5.0);
        let view = h.client.view(h.time_s, 1.0);
        assert_eq!(view.len(), 4);
        assert!(view[0].is_local && view[0].name == "Tester");
        assert_eq!(view.iter().filter(|r| r.is_local).count(), 1);
        for bot in &view[1..] {
            assert!(!bot.name.is_empty());
            assert!(bot.state.distance > 0.0);
            assert!(bot.power_w > 0.0);
        }
    }

    #[test]
    fn teleport_snaps_prediction_on_next_snapshot() {
        let mut h = Harness::new(0, true);
        h.ride(200.0, 1.0);
        let cmd = encode(&client_message(C::Dev(v1::DevCommand {
            cmd: Some(v1::dev_command::Cmd::TeleportDistanceM(2000.0)),
        })));
        h.send(&cmd);
        assert!(
            matches!(h.notices.last(), Some(Notice::CommandResult(m)) if !m.starts_with("error"))
        );
        h.ride(200.0, 0.2);
        assert!((h.client.state().distance - 2000.0).abs() < 5.0);
    }

    #[test]
    fn dev_commands_are_refused_by_a_public_server() {
        let mut h = Harness::new(0, false);
        let cmd = encode(&client_message(C::Dev(v1::DevCommand {
            cmd: Some(v1::dev_command::Cmd::TeleportDistanceM(2000.0)),
        })));
        h.send(&cmd);
        assert!(
            matches!(h.notices.last(), Some(Notice::CommandResult(m)) if m.starts_with("error"))
        );
    }

    #[test]
    fn prediction_is_continuous_across_the_loop_seam() {
        let mut h = Harness::new(0, true);
        let len = h.client.route().length();
        let cmd = encode(&client_message(C::Dev(v1::DevCommand {
            cmd: Some(v1::dev_command::Cmd::TeleportDistanceM(len - 20.0)),
        })));
        h.send(&cmd);
        h.ride(300.0, 0.2);
        let mut last = h.client.state().distance;
        for _ in 0..30 {
            h.ride(300.0, 0.5);
            let d = h.client.state().distance;
            // Never jumps backwards or by more than a few metres per step.
            assert!(d >= last && d - last < 10.0, "{last:.2} -> {d:.2}");
            last = d;
        }
        assert!(last > len, "crossed the seam: {last:.1} of {len:.1} m");
        // Rendering wraps the cumulative distance onto the loop.
        let p = h.client.route().sample(last).position;
        let q = h.client.route().sample(last - len).position;
        assert!((p[0] - q[0]).abs() < 1e-6 && (p[2] - q[2]).abs() < 1e-6);
    }

    #[test]
    fn rejection_and_malformed_messages_are_reported() {
        let mut c = ClientCore::new(route(), "A", RiderProfile::default());
        assert_eq!(c.on_server_message(&[0xff, 0x01], 0.0), None);
        assert_eq!(c.stats().malformed, 1);
        let reject = encode(&ocw_protocol::server_message(S::Reject(v1::Reject {
            reason: "full".into(),
        })));
        assert_eq!(
            c.on_server_message(&reject, 0.0),
            Some(Notice::Rejected("full".into()))
        );
        assert!(c.rider_id().is_none());
    }

    /// The real wiring: client core ↔ `LocalServer` thread ↔ virtual trainer thread.
    #[test]
    fn rides_against_threaded_local_server_and_virtual_trainer() {
        use ocw_devices::{DeviceBackend, VirtualTrainer};
        use std::time::Instant;

        let server = ocw_server::local::LocalServer::spawn(route(), 2, 0.0).unwrap();
        let trainer = VirtualTrainer::new(300.0).start().unwrap();
        let mut client = ClientCore::new(route(), "Live", RiderProfile::default());
        server.to_server.send(client.hello()).unwrap();
        let start = Instant::now();
        let mut next_tick = 0.0;
        while start.elapsed() < Duration::from_millis(1500) {
            let now = start.elapsed().as_secs_f64();
            for e in trainer.events.try_iter() {
                if let Some(sample) = client.on_device_event(e) {
                    server.to_server.send(sample).unwrap();
                }
            }
            for msg in server.from_server.try_iter() {
                client.on_server_message(&msg, now);
            }
            while next_tick <= now {
                client.predict_step();
                next_tick += ocw_core::SIM_DT;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let _ = trainer.commands.send(ocw_devices::DeviceCommand::Shutdown);
        assert!(client.rider_id().is_some());
        assert!(client.power_w() > 250.0);
        assert!(client.stats().snapshots >= 10);
        assert!(client.state().distance > 0.5);
        assert_eq!(client.view(start.elapsed().as_secs_f64(), 1.0).len(), 3);
    }

    /// Drafting a bot: the client predicts the server's drafting, so the
    /// prediction stays close (spec §5.2).
    #[test]
    fn prediction_includes_drafting() {
        let mut h = Harness::new(1, true);
        let bot = h
            .server
            .session()
            .snapshot()
            .into_iter()
            .find(|r| Some(r.id) != h.client.rider_id())
            .unwrap();
        // Put the client rider 3 m behind the bot.
        let cmd = encode(&client_message(C::Dev(v1::DevCommand {
            cmd: Some(v1::dev_command::Cmd::TeleportDistanceM(
                bot.state.distance - 3.0,
            )),
        })));
        h.send(&cmd);
        h.ride(220.0, 30.0);
        assert!(
            h.client.prediction_error().abs() < 0.3,
            "error {}",
            h.client.prediction_error()
        );
    }

    /// Renders at 144 Hz with 50 Hz physics steps: the rider must move by
    /// (nearly) the same amount every frame, not in 0-or-1-step jumps
    /// (regression test for the bouncing rider and scenery).
    #[test]
    fn rendered_motion_is_smooth_between_physics_steps() {
        let mut h = Harness::new(0, false);
        h.ride(250.0, 8.0); // get up to speed
        let dt = 1.0 / 144.0;
        let mut acc = 0.0;
        let mut last = None;
        let mut moves = Vec::new();
        for frame in 0..288 {
            acc += dt;
            while acc >= ocw_core::SIM_DT {
                h.client.predict_step();
                acc -= ocw_core::SIM_DT;
            }
            let alpha = acc / ocw_core::SIM_DT;
            let d = h.client.view(h.time_s + f64::from(frame) * dt, alpha)[0]
                .state
                .distance;
            if let Some(prev) = last {
                moves.push(d - prev);
            }
            last = Some(d);
        }
        let mean = moves.iter().sum::<f64>() / moves.len() as f64;
        let (min, max) = moves.iter().fold((f64::INFINITY, 0.0f64), |(lo, hi), m| {
            (lo.min(*m), hi.max(*m))
        });
        assert!(mean > 0.03, "rider should move: {mean}");
        assert!(
            min > mean * 0.8 && max < mean * 1.2,
            "uneven motion per frame: min {min:.4}, mean {mean:.4}, max {max:.4} m"
        );
    }

    /// Steering: the client reacts at once, sends the intent (rate
    /// limited), and the server ends up at the same lateral offset.
    #[test]
    fn steering_is_predicted_and_applied_by_the_server() {
        let mut h = Harness::new(0, false);
        h.ride(200.0, 1.0);
        let start = h.client.state().lateral_offset;
        assert_eq!(h.client.lateral_target(), Some(start));
        // The first intent is sent once; unchanged targets are not resent.
        let first = h.client.steer_message(h.time_s).expect("initial intent");
        h.send(&first);
        assert!(h.client.steer_message(h.time_s + 1.0).is_none());
        h.client.steer_to(start + 1.5);
        let msg = h.client.steer_message(h.time_s + 0.1).expect("changed");
        match ocw_protocol::decode_client(&msg).unwrap() {
            C::Steer(s) => assert_eq!(s.lateral_target_cm, ((start + 1.5) * 100.0).round() as i32),
            other => panic!("unexpected {other:?}"),
        }
        // Rate limited: a further change right away waits.
        h.client.steer_to(start + 1.6);
        assert!(h.client.steer_message(h.time_s + 0.11).is_none());
        let msg = h
            .client
            .steer_message(h.time_s + 0.16)
            .expect("after the interval");
        h.send(&msg);
        h.ride(200.0, 8.0);
        let own = h.client.rider_id().unwrap();
        let server = h.server.session().rider_state(own).unwrap().lateral_offset;
        // Autodrive steers there smoothly (it leans over, it does not slide).
        assert!((server - (start + 1.6)).abs() < 0.1, "server {server}");
        assert!((h.client.state().lateral_offset - server).abs() < 0.05);
        // Never off the road.
        h.client.steer_to(100.0);
        assert_eq!(
            h.client.lateral_target(),
            Some(ocw_sim::physics::lateral_limit(7.0))
        );
    }

    /// Lean steering: the rider turns into a curve and keeps it until
    /// straightened, the same on the client and the server.
    #[test]
    fn lean_steering_rides_a_curve_until_straightened() {
        let mut h = Harness::new(0, false);
        h.ride(250.0, 4.0);
        let start = h.client.state().lateral_offset;
        let own = h.client.rider_id().unwrap();
        h.client.set_lean(-0.15);
        let msg = h.client.steer_message(h.time_s).expect("lean intent");
        match ocw_protocol::decode_client(&msg).unwrap() {
            C::Steer(s) => assert!(s.lean_steering && s.lean_mrad == -150, "{s:?}"),
            other => panic!("unexpected {other:?}"),
        }
        h.send(&msg);
        h.ride(250.0, 0.6);
        // Turning left: heading and offset to the left, on both sides.
        let server = h.server.session().rider_state(own).unwrap();
        let client = h.client.state();
        assert!(
            server.heading < -0.05 && server.turn_curvature < 0.0,
            "{server:?}"
        );
        assert!(server.lateral_offset < start - 0.2, "{server:?}");
        assert!(
            (client.heading - server.heading).abs() < 0.05,
            "{client:?} vs {server:?}"
        );
        assert!((client.lateral_offset - server.lateral_offset).abs() < 0.3);
        // Upright on the curving loop: nobody steers the bends, so the
        // rider ends up at an edge; client and server agree.
        h.client.set_lean(0.0);
        let msg = h.client.steer_message(h.time_s + 1.0).expect("straighten");
        h.send(&msg);
        h.ride(250.0, 20.0);
        let server = h.server.session().rider_state(own).unwrap();
        let limit = ocw_sim::physics::lateral_limit(7.0);
        assert!(
            (server.lateral_offset.abs() - limit).abs() < 1e-6,
            "{server:?}"
        );
        assert!((h.client.state().lateral_offset - server.lateral_offset).abs() < 0.3);
        // Autodrive brings it back to the middle.
        h.client.set_autodrive(true);
        assert_eq!(h.client.lean(), None);
        let msg = h.client.steer_message(h.time_s + 2.0).expect("autodrive");
        h.send(&msg);
        h.ride(250.0, 10.0);
        let server = h.server.session().rider_state(own).unwrap();
        assert!(server.lateral_offset.abs() < 0.3, "{server:?}");
    }

    /// Regression: leaning hard into the road edge, then holding the other
    /// arrow key stopped the lean at upright, and the rider scraped along
    /// the edge until it nearly stopped. Now the lean goes on to the other
    /// side and the rider leaves the edge, on the client and the server.
    #[test]
    fn steered_into_the_edge_the_rider_steers_back_out() {
        let mut h = Harness::new(0, false);
        h.ride(250.0, 8.0);
        let own = h.client.rider_id().unwrap();
        let limit = ocw_sim::physics::lateral_limit(7.0);
        let mut lean = -ocw_sim::physics::MAX_STEER_LEAN;
        h.client.set_lean(lean);
        let msg = h.client.steer_message(h.time_s).expect("lean intent");
        h.send(&msg);
        h.ride(250.0, 3.0);
        let server = h.server.session().rider_state(own).unwrap();
        assert!((server.lateral_offset + limit).abs() < 1e-6, "{server:?}");
        let pushed = server.speed;
        // Hold → (frame = tick), as the input system does.
        let mut pause = 0.0;
        for _ in 0..(4 * ocw_core::SIM_TICK_HZ) {
            lean = crate::input::lean_ramp(lean, 1.0, &mut pause, ocw_core::SIM_DT);
            h.client.set_lean(lean);
            if let Some(msg) = h.client.steer_message(h.time_s) {
                h.send(&msg);
            }
            h.ride(250.0, ocw_core::SIM_DT);
        }
        assert!(lean > 0.2, "{lean}");
        let server = h.server.session().rider_state(own).unwrap();
        let client = h.client.state();
        assert!(server.lateral_offset > -limit + 1.0, "{server:?}");
        // Still rolling (scraping on held it near 2 m/s).
        assert!(server.speed > 0.8 * pushed, "{pushed} {server:?}");
        assert!(
            (client.lateral_offset - server.lateral_offset).abs() < 0.3,
            "{client:?} vs {server:?}"
        );
        assert!((client.speed - server.speed).abs() < 0.2);
    }

    /// Wild mode on switches autodrive off (one intent carries both), so
    /// the rider leans off the road; wild off keeps the steering mode.
    #[test]
    fn wild_mode_switches_autodrive_off_and_leaves_the_road() {
        let mut h = Harness::new(0, false);
        h.ride(250.0, 4.0);
        let own = h.client.rider_id().unwrap();
        let first = h.client.steer_message(h.time_s).expect("initial intent");
        h.send(&first);
        assert!(h.client.autodrive());
        h.client.set_wild(true);
        assert!(!h.client.autodrive());
        assert_eq!(h.client.lean(), Some(0.0));
        assert!(h.client.steering_status().starts_with("manual | WILD"));
        let msg = h.client.steer_message(h.time_s + 1.0).expect("wild intent");
        match ocw_protocol::decode_client(&msg).unwrap() {
            C::Steer(s) => assert!(s.wild && s.lean_steering && s.lean_mrad == 0, "{s:?}"),
            other => panic!("unexpected {other:?}"),
        }
        h.send(&msg);
        assert!(h.client.steer_message(h.time_s + 2.0).is_none());
        h.client.set_lean(-0.15);
        let msg = h.client.steer_message(h.time_s + 2.0).expect("lean intent");
        h.send(&msg);
        h.ride(250.0, 3.0);
        let limit = ocw_sim::physics::lateral_limit(7.0);
        let server = h.server.session().rider_state(own).unwrap();
        assert!(server.lateral_offset < -limit - 1.0, "{server:?}");
        assert!((h.client.state().lateral_offset - server.lateral_offset).abs() < 0.3);
        // Autodrive switched on in wild mode stays on when wild goes off.
        h.client.set_autodrive(true);
        h.client.set_wild(false);
        assert!(h.client.autodrive());
        assert!(!h.client.steering_status().contains("WILD"));
        // Manual steering also stays manual.
        h.client.set_wild(true);
        h.client.set_autodrive(true);
        h.client.set_lean(0.1);
        h.client.set_wild(false);
        assert_eq!(h.client.lean(), Some(0.1));
    }

    /// The menu holds autodrive (spec §3.2.1): manual steering with a lean
    /// would leave the road while the rider looks at the menu.
    #[test]
    fn menu_holds_autodrive_and_restores_manual_steering() {
        let mut h = Harness::new(0, false);
        h.ride(250.0, 2.0);
        let own = h.client.rider_id().unwrap();
        h.client.set_lean(0.2);
        let msg = h.client.steer_message(h.time_s).expect("lean intent");
        h.send(&msg);
        h.ride(250.0, 0.5);
        h.client.hold_steering();
        assert!(h.client.autodrive() && h.client.steering_held());
        assert!(h.client.steering_status().starts_with("autodrive (menu)"));
        let msg = h
            .client
            .steer_message(h.time_s + 1.0)
            .expect("autodrive intent");
        match ocw_protocol::decode_client(&msg).unwrap() {
            C::Steer(s) => assert!(!s.lean_steering && !s.wild, "{s:?}"),
            other => panic!("unexpected {other:?}"),
        }
        h.send(&msg);
        // Held twice (menu reopened from a sub page): the first mode stays.
        h.client.hold_steering();
        // The ride goes on: power samples still reach the server, the rider
        // rides on and stays on the road.
        let d0 = h.server.session().rider_state(own).unwrap().distance;
        assert!(
            h.client
                .on_device_event(power(250.0, h.time_s + 0.1))
                .is_some()
        );
        h.ride(250.0, 10.0);
        let st = h.server.session().rider_state(own).unwrap();
        assert!(st.distance - d0 > 50.0, "{st:?}");
        let limit = ocw_sim::physics::lateral_limit(7.0);
        assert!(st.lateral_offset.abs() <= limit + 0.05, "{st:?}");
        assert!((h.client.state().lateral_offset - st.lateral_offset).abs() < 0.1);
        // Closed: manual again, upright.
        h.client.release_steering();
        assert!(!h.client.steering_held());
        assert_eq!(h.client.lean(), Some(0.0));
        assert!(h.client.steering_status().starts_with("manual"));
        // Released twice: nothing more happens.
        h.client.set_lean(0.1);
        h.client.release_steering();
        assert_eq!(h.client.lean(), Some(0.1));
    }

    #[test]
    fn menu_keeps_the_autodrive_line_and_restores_wild_mode() {
        let mut h = Harness::new(0, false);
        h.ride(250.0, 2.0);
        // Autodrive on its own line: unchanged by the menu.
        h.client.steer_to(1.0);
        h.client.hold_steering();
        assert_eq!(h.client.lateral_target(), Some(1.0));
        h.client.release_steering();
        assert!(h.client.autodrive());
        assert_eq!(h.client.lateral_target(), Some(1.0));
        // Wild mode: off while the menu is open (back within the road).
        h.client.set_wild(true);
        h.client.hold_steering();
        assert!(!h.client.wild() && h.client.autodrive());
        let limit = ocw_sim::physics::lateral_limit(7.0);
        assert!(h.client.lateral_target().unwrap().abs() <= limit);
        h.client.release_steering();
        assert!(h.client.wild() && !h.client.autodrive());
        // Wild with autodrive: both come back.
        h.client.set_autodrive(true);
        h.client.hold_steering();
        h.client.release_steering();
        assert!(h.client.wild() && h.client.autodrive());
    }

    /// While the menu is open the next junction takes its default exit.
    #[test]
    fn menu_resets_the_junction_choice_to_the_default_exit() {
        let mut h = Harness::free(0);
        h.ride(250.0, 1.0);
        h.client.turn(1).expect("a junction ahead");
        let msg = h.client.turn_message().expect("turn intent");
        h.send(&msg);
        h.client.hold_steering();
        // Taking back the choice is sent to the server.
        let msg = h.client.turn_message().expect("choice taken back");
        match ocw_protocol::decode_client(&msg).unwrap() {
            C::Turn(t) => assert!(t.exit_edge_id.is_none(), "{t:?}"),
            other => panic!("unexpected {other:?}"),
        }
        h.send(&msg);
        let mut seen = false;
        for _ in 0..500 {
            h.ride(250.0, 0.1);
            if let Some(j) = h.client.junction_view() {
                assert_eq!(j.selected, j.default, "{j:?}");
                seen = true;
            }
        }
        assert!(seen);
        // Straight on through x = 600 m: still on the top row (z = 0).
        let (c, s) = h.positions();
        assert!(c.position[0] > 620.0 && c.position[2].abs() < 6.0, "{c:?}");
        assert!(close(c.position, s.position, 1.0), "{c:?} vs {s:?}");
    }

    /// Down arrow: the brake reaches the server at once and stops the
    /// rider; the prediction brakes too.
    #[test]
    fn braking_stops_the_rider_on_server_and_client() {
        let mut h = Harness::new(0, false);
        h.ride(250.0, 15.0);
        assert!(h.client.state().speed > 5.0);
        let msg = h.client.set_brake(true).expect("brake sent at once");
        match ocw_protocol::decode_client(&msg).unwrap() {
            C::Sample(s) => assert_eq!(s.brake, Some(true)),
            other => panic!("unexpected {other:?}"),
        }
        assert!(h.client.set_brake(true).is_none(), "unchanged");
        h.send(&msg);
        h.ride(0.0, 5.0);
        let own = h.client.rider_id().unwrap();
        assert_eq!(h.server.session().rider_state(own).unwrap().speed, 0.0);
        assert!(h.client.state().speed < 0.05, "{}", h.client.state().speed);
        // Released: riding again.
        let msg = h.client.set_brake(false).unwrap();
        h.send(&msg);
        h.ride(250.0, 3.0);
        assert!(h.client.state().speed > 1.0);
    }

    /// ↑ adds power on top of the device (e.g. a real trainer nobody pedals).
    #[test]
    fn keyboard_power_adds_to_the_device_power() {
        let mut h = Harness::new(0, false);
        h.ride(0.0, 1.0); // device reports 0 W
        assert_eq!(h.client.power_w(), 0.0);
        let msg = h.client.set_key_power(250.0).expect("sent at once");
        match ocw_protocol::decode_client(&msg).unwrap() {
            C::Sample(s) => assert_eq!(s.power_w, Some(250.0)),
            other => panic!("unexpected {other:?}"),
        }
        assert!(h.client.set_key_power(250.0).is_none(), "unchanged");
        h.send(&msg);
        // Device samples keep 0 W, the keyboard's 250 W stay on top.
        h.ride(0.0, 10.0);
        assert_eq!(h.client.power_w(), 250.0);
        let own = h.client.rider_id().unwrap();
        assert!(h.server.session().rider_state(own).unwrap().speed > 5.0);
        assert!(h.client.set_key_power(-5.0).is_some());
        assert_eq!(h.client.key_power_w(), 0.0);
    }

    /// A 4 × 4 grid of crossroads 300 m apart (vertex 4·row + col at
    /// x = 300·col, z = 300·row; +z is south).
    fn grid() -> ocw_world::graph::RoadGraph {
        use ocw_world::graph::{Edge, EdgeId, Oneway, RoadClass, RoadGraph, VertexId};
        let n = 4u32;
        let mut vertices = Vec::new();
        for row in 0..n {
            for col in 0..n {
                vertices.push([f64::from(col) * 300.0, f64::from(row) * 300.0]);
            }
        }
        let mut pairs = Vec::new();
        for row in 0..n {
            for col in 0..n {
                let i = row * n + col;
                if col + 1 < n {
                    pairs.push((i, i + 1));
                }
                if row + 1 < n {
                    pairs.push((i, i + n));
                }
            }
        }
        RoadGraph {
            edges: pairs
                .iter()
                .enumerate()
                .map(|(k, &(a, b))| Edge {
                    id: EdgeId(9000 + k as u64),
                    from: VertexId(a),
                    to: VertexId(b),
                    points: vec![vertices[a as usize], vertices[b as usize]],
                    class: RoadClass::Tertiary,
                    name: Some(format!("Road {k}")),
                    width_m: 6.0,
                    bridge: false,
                    tunnel: false,
                    dead_end_m: None,
                    boundary: false,
                    oneway: Oneway::No,
                    osm_way: k as i64,
                    profile: Vec::new(),
                })
                .collect(),
            vertices,
            boundary_vertices: Default::default(),
        }
    }

    impl Harness {
        /// Where the local rider is on the client and on the server.
        fn positions(&self) -> (ocw_world::RoutePoint, ocw_world::RoutePoint) {
            let own = self.client.rider_id().unwrap();
            let server = self.server.session().rider_state(own).unwrap();
            (
                self.client.track().sample(self.client.state().distance),
                self.server.session().track(own).sample(server.distance),
            )
        }
    }

    fn close(a: [f64; 3], b: [f64; 3], tol: f64) -> bool {
        (a[0] - b[0]).hypot(a[2] - b[2]) < tol
    }

    /// Free ride through two junctions (spec §14, Phase 3): a chosen exit
    /// (right) and then the default (straight on). The client predicts its
    /// path, the server takes the same exits, and both agree.
    #[test]
    fn free_ride_through_junctions_with_a_chosen_and_a_default_exit() {
        let mut h = Harness::free(2);
        h.ride(250.0, 1.0);
        assert!(h.client.is_free_ride());
        let own = h.client.rider_id().unwrap();
        assert!(h.client.paths().contains_key(&own));
        // The junction at x = 600 m is 250 m ahead: no overlay yet, but a
        // pre-selection is allowed.
        assert!(h.client.junction_view().is_none());
        assert!(!h.client.junction_active());
        let said = h.client.turn(1).expect("a junction ahead");
        assert!(said.contains("right"), "{said}");
        let msg = h.client.turn_message().expect("turn intent");
        match ocw_protocol::decode_client(&msg).unwrap() {
            C::Turn(t) => assert!(t.exit_edge_id.is_some() && t.junction_id == 2, "{t:?}"),
            other => panic!("unexpected {other:?}"),
        }
        h.send(&msg);
        assert!(h.client.turn_message().is_none(), "unchanged");
        let (mut active, mut locked) = (false, false);
        for _ in 0..400 {
            h.ride(250.0, 0.1);
            if let Some(j) = h.client.junction_view() {
                assert!(j.exits[j.selected].angle_deg > 60.0, "{j:?}");
                assert_ne!(j.selected, j.default);
                if j.locked {
                    locked = true;
                    assert!(j.distance_m < lock_distance(12.0));
                } else {
                    active = true;
                    assert!(j.distance_m <= 150.0 && j.distance_m >= ocw_world::path::LOCK_M - 1.0);
                }
            }
        }
        assert!(active && locked);
        // Turned right: heading south on the x = 600 m road.
        let (c, s) = h.positions();
        assert!(
            (c.position[0] - 600.0).abs() < 4.0 && c.position[2] > 30.0,
            "{c:?}"
        );
        assert!(c.direction[1] > 0.99);
        assert!(close(c.position, s.position, 1.0), "{c:?} vs {s:?}");
        // The next junction (x 600, z 300): nothing chosen, straight on.
        h.ride(250.0, 45.0);
        let (c, s) = h.positions();
        assert!(
            (c.position[0] - 600.0).abs() < 4.0 && c.position[2] > 320.0,
            "{c:?}"
        );
        assert!(c.direction[1] > 0.99, "{c:?}");
        assert!(close(c.position, s.position, 1.0), "{c:?} vs {s:?}");
        assert!(h.client.prediction_error().abs() < 0.5);
        // Same path on both sides.
        let d = h.client.state().distance;
        let server_path = h.server.session().path(own).unwrap();
        for k in [-150.0, -60.0, 0.0, 20.0] {
            let a = h.client.track().sample(d + k).position;
            let b = server_path.sample(d + k).position;
            assert!(close(a, b, 1e-6), "{k}: {a:?} vs {b:?}");
        }
        // The bots ride their own ways and are drawn.
        assert_eq!(h.client.view(h.time_s, 1.0).len(), 3);
        assert_eq!(h.client.paths().len(), 3);
        assert_eq!(h.client.stats().malformed, 0);
    }

    /// The choice did not reach the server in time: the server's default
    /// applies and corrects the client's predicted path (spec §6.5).
    #[test]
    fn free_ride_late_choice_is_corrected_by_the_server() {
        let mut h = Harness::free(0);
        h.ride(250.0, 1.0);
        h.client.turn(1).unwrap(); // right, but never sent
        h.ride(250.0, 45.0);
        let (c, s) = h.positions();
        // Straight on (east) along the top row, like the server.
        assert!(c.position[0] > 650.0 && c.position[2].abs() < 4.0, "{c:?}");
        assert!(c.direction[0] > 0.99);
        assert!(close(c.position, s.position, 1.0), "{c:?} vs {s:?}");
    }

    /// U-turn (spec §2.2.2): predicted at once, the server builds the same
    /// loop from the client's position; rate limited.
    #[test]
    fn free_ride_u_turn_on_client_and_server() {
        let mut h = Harness::free(0);
        h.ride(250.0, 8.0);
        let msg = h.client.u_turn(h.time_s).expect("u-turn");
        assert!(h.client.u_turn(h.time_s + 1.0).is_none(), "rate limited");
        assert_eq!(
            h.client
                .notice()
                .as_deref()
                .map(|n| n.starts_with("U-turn")),
            Some(true)
        );
        h.send(&msg);
        h.ride(250.0, 25.0);
        let (c, s) = h.positions();
        assert!(c.direction[0] < -0.99, "{c:?}");
        assert!(close(c.position, s.position, 0.5), "{c:?} vs {s:?}");
        assert!(h.client.prediction_error().abs() < 0.5);
    }

    /// Regression (pilot package, Fürst-Franz-Josef-Strasse in Vaduz): a
    /// U-turn on a one-way street sent the rider back against it; at the
    /// vertex behind there was no exit, so an automatic U-turn followed a
    /// few metres on and the rider turned back and forth in place. Now the
    /// rider rides on against the one-way road to the first legal exit, and
    /// client and server agree.
    #[test]
    fn free_ride_u_turn_on_a_one_way_street_rides_back() {
        use ocw_world::graph::Oneway;
        let mut g = grid();
        // The top row one-way eastwards; vertex 1 only joins it (no road
        // south), so riding back west it has no legal exit.
        g.edges.retain(|e| !(e.from.0 == 1 && e.to.0 == 5));
        for e in &mut g.edges {
            if e.from.0 < 3 && e.to.0 == e.from.0 + 1 {
                e.oneway = Oneway::Forward;
            }
        }
        let net = Arc::new(RoadNetwork::new(g));
        let edge = (0..net.graph().edges.len())
            .find(|&e| net.edge(e).from.0 == 1 && net.edge(e).to.0 == 2)
            .unwrap();
        let server = GameServer::new(
            ocw_server::Course::FreeRide {
                net: net.clone(),
                start: ocw_sim::FreeRideStart {
                    edge,
                    offset_m: 100.0,
                    forward: true,
                },
            },
            ServerConfig {
                allow_dev_commands: true,
                bots: 0,
                start_m: 0.0,
                allow_local_events: true,
                voice: None,
                ..Default::default()
            },
        );
        let mut h = Harness::with(
            server,
            ClientCore::free_ride(net, "Oneway", RiderProfile::default()),
        );
        h.ride(250.0, 3.0);
        let msg = h.client.u_turn(h.time_s).expect("u-turn");
        h.send(&msg);
        // Back west past vertex 1 (x = 300) on the one-way road against it.
        let mut west_of_vertex_1 = false;
        for _ in 0..50 {
            h.ride(250.0, 1.0);
            let (c, s) = h.positions();
            assert!(close(c.position, s.position, 0.5), "{c:?} vs {s:?}");
            west_of_vertex_1 |= c.position[0] < 250.0;
        }
        let (c, _) = h.positions();
        assert!(west_of_vertex_1, "still near the U-turn: {c:?}");
        assert!(h.client.prediction_error().abs() < 0.5);
    }

    /// A route on the grid: from vertex 1 east to 2, south to 6 and 10,
    /// east to 11.
    fn grid_route(net: &RoadNetwork) -> Arc<RoutePlan> {
        let e = |a: u32, b: u32| {
            (0..net.graph().edges.len())
                .find_map(|i| {
                    let x = net.edge(i);
                    if (x.from.0, x.to.0) == (a, b) {
                        Some(ocw_world::Segment::whole(net, i, true))
                    } else if (x.from.0, x.to.0) == (b, a) {
                        Some(ocw_world::Segment::whole(net, i, false))
                    } else {
                        None
                    }
                })
                .unwrap()
        };
        let steps = vec![e(1, 2), e(2, 6), e(6, 10), e(10, 11)];
        Arc::new(RoutePlan::new(net, "Grid route", steps, false).unwrap())
    }

    impl Harness {
        /// A route ride on the grid with `bots` bots.
        fn route_ride(bots: u32) -> Self {
            let net = Arc::new(RoadNetwork::new(grid()).with_package_name("Grid"));
            let plan = grid_route(&net);
            let server = GameServer::new(
                ocw_server::Course::RouteRide {
                    net: net.clone(),
                    plan: plan.clone(),
                },
                ServerConfig {
                    allow_dev_commands: true,
                    bots,
                    start_m: 0.0,
                    allow_local_events: true,
                    voice: None,
                    ..Default::default()
                },
            );
            Self::with(
                server,
                ClientCore::free_ride(net, "Routed", RiderProfile::default()).with_route_plan(plan),
            )
        }
    }

    /// Route ride (spec §2.7, §2.2.1): the overlay shows the route's exit
    /// as the default, the client and the server follow it; another exit
    /// leaves the route, `ride.rejoin` brings the rider back; bots stay on
    /// it.
    #[test]
    fn route_ride_follow_leave_and_rejoin() {
        let mut h = Harness::route_ride(1);
        h.ride(250.0, 1.0);
        assert_eq!(h.client.course_name(), "Grid route");
        assert!(
            matches!(&h.notices[0], Notice::Joined { route_id, .. } if route_id == "Grid route")
        );
        assert_eq!(h.client.on_route(), Some(true));
        assert!(
            h.client
                .route_status()
                .unwrap()
                .starts_with("Grid route 0.0 / 1.2 km"),
            "{:?}",
            h.client.route_status()
        );
        // At vertex 2 the route turns right (south): that is the default.
        let mut saw_route_arrow = false;
        for _ in 0..450 {
            h.ride(250.0, 0.1);
            if let Some(j) = h.client.junction_view() {
                assert_eq!(j.route, Some(j.default), "{j:?}");
                assert_eq!(j.selected, j.default);
                assert!(j.exits[j.default].angle_deg > 60.0, "right: {j:?}");
                saw_route_arrow = true;
            }
        }
        assert!(saw_route_arrow);
        let (c, s) = h.positions();
        assert!(c.direction[1] > 0.99, "south on the route: {c:?}");
        assert!(close(c.position, s.position, 1.0), "{c:?} vs {s:?}");
        assert_eq!(h.client.on_route(), Some(true));
        // Vertex 6: leave the route (straight on is the route; turn left).
        h.ride(250.0, 5.0);
        let said = h.client.choose_direction("left").expect("junction at 6");
        assert!(said.contains("leaves the route"), "{said}");
        let msg = h.client.turn_message().unwrap();
        h.send(&msg);
        h.ride(250.0, 40.0);
        let (c, s) = h.positions();
        assert!(c.direction[0] > 0.99, "left = east, off the route: {c:?}");
        assert!(close(c.position, s.position, 1.0), "{c:?} vs {s:?}");
        assert_eq!(h.client.on_route(), Some(false));
        assert!(h.client.notice().unwrap().contains("ride.rejoin"));
        assert!(h.client.route_status().unwrap().contains("off the route"));
        // Back to the route: predicted, and the server agrees.
        let msg = h.client.rejoin_route().expect("a way back");
        assert!(h.client.rejoin_route().unwrap_err().contains("already"));
        h.send(&msg);
        h.ride(250.0, 150.0);
        let status = h.client.route_status().unwrap();
        assert!(!status.contains("off the route"), "{status}");
        let (c, s) = h.positions();
        assert!(close(c.position, s.position, 1.0), "{c:?} vs {s:?}");
        // The bot never left the route.
        let bot = h
            .server
            .session()
            .snapshot()
            .iter()
            .map(|r| r.id)
            .find(|&id| Some(id) != h.client.rider_id())
            .unwrap();
        let bd = h.server.session().rider_state(bot).unwrap().distance;
        let bp = h.server.session().path(bot).unwrap();
        assert!(bp.on_route_at(bd) || bp.route_progress(bd).unwrap().finished);
        assert_eq!(h.client.stats().malformed, 0);
    }

    /// Package version check (spec §6.5): a client with another build of the
    /// package is refused, by the server or (for servers that do not check)
    /// by the client itself.
    #[test]
    fn another_world_package_is_refused() {
        let net = Arc::new(RoadNetwork::new(grid()).with_package_name("Grid"));
        let mut other = grid();
        other.edges[0].points[1][0] += 1.0;
        let other = Arc::new(RoadNetwork::new(other).with_package_name("Grid"));
        let server = GameServer::new(
            ocw_server::Course::FreeRide {
                net,
                start: ocw_sim::FreeRideStart {
                    edge: 1,
                    offset_m: 10.0,
                    forward: true,
                },
            },
            ServerConfig::default(),
        );
        let mut h = Harness::with(
            server,
            ClientCore::free_ride(other.clone(), "Old", RiderProfile::default()),
        );
        assert!(
            matches!(&h.notices[..], [Notice::Rejected(r)] if r.contains("world package mismatch")),
            "{:?}",
            h.notices
        );
        assert_eq!(h.client.rider_id(), None);
        // A Welcome naming another graph is refused by the client too.
        let welcome = encode(&ocw_protocol::server_message(S::Welcome(v1::Welcome {
            protocol_version: PROTOCOL_VERSION,
            rider_id: 7,
            tick_rate_hz: 50,
            broadcast_rate_hz: 10,
            route_id: String::new(),
            free_ride: true,
            world: Some(v1::WorldPackageId {
                name: "Grid".into(),
                graph_hash: other.fingerprint() ^ 1,
            }),
            graph_route: String::new(),
            voice: false,
            clock: None,
            world_access: 0,
            god_access: 0,
            echo_bot: false,
            ..Default::default()
        })));
        h.receive(&welcome);
        assert!(matches!(h.notices.last(), Some(Notice::Rejected(_))));
        assert_eq!(h.client.rider_id(), None);
    }

    #[test]
    fn a_server_on_another_course_is_refused() {
        // Server on the loop, client on the hills: riding on would place
        // everyone at the right distance on the wrong road.
        let server = GameServer::new(Route::test_loop(), ServerConfig::default());
        let h = Harness::with(
            server,
            ClientCore::new(Route::test_hills(), "Hills", RiderProfile::default()),
        );
        assert!(
            matches!(&h.notices[..], [Notice::Rejected(r)] if r.contains("course mismatch") && r.contains("--route")),
            "{:?}",
            h.notices
        );
        assert_eq!(h.client.rider_id(), None);
        // A free-ride client on a route server, too.
        let server = GameServer::new(Route::test_loop(), ServerConfig::default());
        let net = Arc::new(RoadNetwork::new(grid()));
        let h = Harness::with(
            server,
            ClientCore::free_ride(net, "Free", RiderProfile::default()),
        );
        assert!(matches!(&h.notices[..], [Notice::Rejected(r)] if r.contains("course mismatch")));
    }

    #[test]
    fn reordered_snapshots_are_dropped() {
        let mut h = Harness::new(1, false);
        h.ride(250.0, 2.0);
        let before = h.client.stats().clone();
        assert!(before.snapshots >= 10);
        // A late datagram: an older tick than the last one.
        let late = encode(&ocw_protocol::server_message(S::Snapshot(v1::Snapshot {
            server_tick: before.last_snapshot_tick - 5,
            riders: Vec::new(),
            human_riders: 1,
            clock: None,
            ..Default::default()
        })));
        h.receive(&late);
        let after = h.client.stats();
        assert_eq!(after.stale_snapshots, 1);
        assert_eq!(after.snapshots, before.snapshots);
        assert_eq!(after.last_snapshot_tick, before.last_snapshot_tick);
        // The other rider was not dropped by the empty, stale snapshot.
        assert_eq!(h.client.view(h.time_s, 1.0).len(), 2);
    }

    #[test]
    fn snapshot_age_extrapolates_the_reconciled_state() {
        // Same ride twice; one client believes snapshots are 100 ms old.
        let mut fresh = Harness::new(0, false);
        let mut aged = Harness::new(0, false);
        aged.client.set_snapshot_age(0.1);
        fresh.ride(300.0, 3.0);
        aged.ride(300.0, 3.0);
        let (a, b) = (fresh.client.state(), aged.client.state());
        assert!(b.distance > a.distance, "{} vs {}", b.distance, a.distance);
        // Ages are clamped.
        aged.client.set_snapshot_age(-1.0);
        assert_eq!(aged.client.snapshot_age_s, 0.0);
    }

    #[test]
    fn exit_selection_steps_through_the_exits() {
        let mut h = Harness::free(0);
        h.ride(250.0, 1.0);
        // The top row's junction: straight on (the default) or right.
        // Nothing to the left of straight.
        assert!(h.client.turn(-1).unwrap().contains("default"));
        assert!(h.client.turn(1).unwrap().contains("right"));
        assert!(h.client.turn(1).unwrap().contains("right"));
        // Back to straight: the default, "no choice".
        assert!(h.client.turn(-1).unwrap().contains("default"));
        assert!(h.client.choose_direction("left").is_none());
        assert!(
            h.client
                .choose_direction("right")
                .unwrap()
                .contains("right")
        );
        assert!(h.client.clear_exit().unwrap().contains("default"));
        assert!(h.client.choose_exit(9).is_none());
        // Route rides have no junctions.
        let mut r = Harness::new(0, false);
        r.ride(200.0, 1.0);
        assert!(r.client.turn(1).is_none());
        assert!(r.client.u_turn(1.0).is_none());
        assert!(r.client.junction_view().is_none());
    }

    #[test]
    fn race_countdown_holds_the_prediction_then_ranks() {
        let mut h = Harness::new(5, true);
        h.ride(200.0, 3.0);
        h.client.steer_to(1.5);
        h.ride(200.0, 1.0);
        let race = ocw_sim::EventSpec::Race {
            laps: None,
            length_m: Some(300.0),
        };
        h.send(&ClientCore::event_start_message(&race, Some(3)));
        assert!(
            matches!(&h.notices[..], [.., Notice::CommandResult(m)] if m.contains("4 riders")),
            "{:?}",
            h.notices
        );
        h.ride(200.0, 2.5);
        // Lined up at the start line, in the front row's centre: the
        // prediction stays there although the rider pedals.
        let line = h.client.event_info().unwrap().start_line_m;
        let st = h.client.state();
        assert!(
            (st.distance - line).abs() < 0.01,
            "{} at {line}",
            st.distance
        );
        assert_eq!(
            h.client.lateral_target(),
            Some(0.0),
            "steers from its place"
        );
        let v = h.client.event_view(h.time_s).unwrap();
        assert_eq!(v.countdown.as_deref(), Some("3"));
        assert_eq!(v.title, "Race - 0.3 km");
        h.ride(200.0, 6.0);
        assert!(h.client.state().distance > line + 5.0);
        let v = h.client.event_view(h.time_s).unwrap();
        assert!(
            v.lines[0].starts_with('P') && v.lines[0].contains("/4"),
            "{v:?}"
        );
        assert!(h.client.prediction_error().abs() < 1.0);
        // To the finish: the results panel, until dismissed.
        h.ride(200.0, 60.0);
        assert!(h.client.results_showing(h.time_s));
        let v = h.client.event_view(h.time_s).unwrap();
        assert!(v.lines[0].starts_with("Finished P"), "{v:?}");
        assert_eq!(v.results.unwrap().rows.len(), 4);
        assert!(h.client.dismiss_results(h.time_s));
        assert!(h.client.event_view(h.time_s).unwrap().results.is_none());
        // No ghost for races.
        assert!(h.client.wants_ghost().is_none());
        assert!(h.client.take_finished_trace().is_none());
        // Cancelled: no event any more.
        h.send(&ClientCore::event_cancel_message());
        h.ride(200.0, 0.2);
        assert!(h.client.event_view(h.time_s).is_none());
    }

    #[test]
    fn time_trial_records_the_effort_and_rides_against_it() {
        let mut h = Harness::new(2, true);
        let tt = ocw_sim::EventSpec::TimeTrial {
            laps: None,
            length_m: Some(250.0),
        };
        h.send(&ClientCore::event_start_message(&tt, None));
        h.ride(250.0, 0.5);
        // Alone: the bots are gone.
        assert_eq!(h.server.session().rider_count(), 1);
        let key = h.client.wants_ghost().unwrap();
        assert_eq!(key.length_m, 250);
        h.client.set_ghost(key.clone(), None);
        assert!(h.client.ghost_state(h.time_s).is_none());
        h.ride(250.0, 60.0);
        let (k, first) = h.client.take_finished_trace().unwrap();
        assert_eq!(k, key);
        assert_eq!(first.length_m(), 250.0);
        assert!(first.samples().len() > 20);
        h.client
            .set_ghost_outcome(crate::event::GhostOutcome::First);
        let r = h.client.event_view(h.time_s).unwrap().results.unwrap();
        assert!(r.footer[0].contains("your ghost now"), "{r:?}");
        // Again, slower, against the first effort.
        h.send(&ClientCore::event_start_message(&tt, None));
        h.ride(150.0, 0.5);
        let key = h.client.wants_ghost().unwrap();
        h.client.set_ghost(key, Some(first.clone()));
        h.ride(150.0, 15.0);
        let ghost = h.client.ghost_state(h.time_s).unwrap();
        let me = h.client.state();
        assert!(ghost.distance > me.distance, "{ghost:?} {me:?}");
        assert!(ghost.speed > 1.0);
        let v = h.client.event_view(h.time_s).unwrap();
        assert!(
            v.lines.last().unwrap().contains("(behind)"),
            "{:?}",
            v.lines
        );
        h.ride(150.0, 60.0);
        let (_, second) = h.client.take_finished_trace().unwrap();
        assert!(second.duration_s() > first.duration_s());
        assert!(
            h.client.ghost_state(h.time_s).is_none(),
            "gone at the finish"
        );
    }

    /// God mode end to end (spec §14 Phase 13F): enter, the poses reach the
    /// server and another client sees the unicorn; the road position stays
    /// frozen; leaving in free ride lands on the nearest road, standing,
    /// and the prediction stands there with the server.
    #[test]
    fn god_mode_flies_others_see_the_unicorn_and_it_lands_on_the_nearest_road() {
        use crate::god_state::{GodEvent, Phase};
        use ocw_sim::god::FlightInput;
        let mut h = Harness::free(0);
        h.ride(150.0, 1.0);
        let net = Arc::new(RoadNetwork::new(grid()));
        let conn = h.server.connect();
        let mut guest = ClientCore::free_ride(net, "Guest", RiderProfile::default());
        for reply in h.server.handle(conn, &guest.hello()) {
            guest.on_server_message(&reply, h.time_s);
        }
        let me = h.client.rider_id().unwrap();
        assert!(h.client.may_use_god(), "the host may");
        assert!(!guest.may_use_god(), "a guest may not (the default)");
        let msg = h.client.god_mut().request_enter(h.time_s).unwrap();
        h.send(&msg);
        assert!(
            matches!(
                h.notices.last(),
                Some(Notice::God {
                    event: Some(GodEvent::Entered),
                    ..
                })
            ),
            "{:?}",
            h.notices.last()
        );
        let frozen = h.server.session().rider_state(me).unwrap().distance;
        let took_off = h.client.god().flight().pos;
        // Fly 3 s along the riding direction at the speed law.
        let ahead = FlightInput {
            forward: 1.0,
            ..Default::default()
        };
        let flat = |_: f64, _: f64| Some(0.0);
        for _ in 0..150 {
            let t = h.time_s;
            h.client
                .god_mut()
                .step(&ahead, (0.0, 0.0), 0.0, ocw_core::SIM_DT, t, &flat, None);
            if let Some(m) = h.client.god_mut().pose_message(t) {
                h.send(&m);
            }
            for msg in h.server.tick() {
                h.receive(&msg);
                guest.on_server_message(&msg, t);
            }
            h.client.predict_step();
            h.time_s += ocw_core::SIM_DT;
        }
        let pos = h.client.god().flight().pos;
        let flown = (pos[0] - took_off[0]).hypot(pos[2] - took_off[2]);
        assert!(flown > 40.0, "flew: {pos:?} from {took_off:?}");
        assert_eq!(h.server.session().rider_state(me).unwrap().distance, frozen);
        assert!(
            (h.client.state().distance - frozen).abs() < 0.01,
            "no prediction"
        );
        // The guest sees the unicorn near the flight, with the name.
        let seen = guest.unicorns(h.time_s);
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].name, "Free");
        assert!(!seen[0].is_local);
        let d = (seen[0].pos[0] - pos[0]).hypot(seen[0].pos[2] - pos[2]);
        assert!(d < 15.0, "{d}");
        assert_eq!(h.client.unicorns(h.time_s).len(), 1, "the own unicorn");
        // G over (610, 450) of the grid: lands on x = 600 (nearest road).
        h.client
            .god_mut()
            .teleport([610.0, 60.0, 450.0], None, None);
        let spot = h.client.landing_spot(610.0, 450.0).unwrap();
        assert!((spot[0] - 600.0).abs() < 1e-6 && (spot[2] - 450.0).abs() < 1e-6);
        h.client.god_mut().set_landing(Some(spot));
        let msg = h.client.god_mut().request_leave(false, h.time_s).unwrap();
        h.send(&msg);
        let mut landed = false;
        for _ in 0..200 {
            let t = h.time_s;
            if h.client
                .god_mut()
                .step(&ahead, (0.0, 0.0), 0.0, ocw_core::SIM_DT, t, &flat, None)
                == Some(GodEvent::Landed)
            {
                landed = true;
            }
            for msg in h.server.tick() {
                h.receive(&msg);
            }
            h.client.predict_step();
            h.time_s += ocw_core::SIM_DT;
        }
        assert!(landed && h.client.god().phase() == Phase::Riding);
        let at = h
            .server
            .session()
            .track(me)
            .sample(h.server.session().rider_state(me).unwrap().distance);
        assert!((at.position[0] - 600.0).abs() < 5.0, "{:?}", at.position);
        let mine = h.client.track().sample(h.client.state().distance);
        assert!(
            close(mine.position, at.position, 5.0),
            "{:?} vs {:?}",
            mine.position,
            at.position
        );
    }

    /// Collisions on the client (spec §14 Phase 12D, 12H): a bump the
    /// server resolves is blended into the prediction without a visible
    /// snap; a crash shows only once the server says so, with the HUD line
    /// and the feed.
    #[test]
    fn bumps_reconcile_smoothly_and_crashes_come_only_from_the_server() {
        let server = GameServer::new(route(), ocw_server::local::local_config(4, 0.0));
        let client = ClientCore::new(route(), "Tester", RiderProfile::default());
        let mut h = Harness::with(server, client);
        h.ride(250.0, 10.0);
        assert_eq!(h.client.collision_mode(), ocw_sim::collide::Mode::Full);
        let stage = ClientCore::stage_message(false);
        h.send(&stage);
        let mut last = h.client.state();
        let (mut lateral_jump, mut along_jump) = (0.0f64, 0.0f64);
        for _ in 0..100 {
            h.ride(250.0, 0.02);
            let st = h.client.state();
            lateral_jump = lateral_jump.max((st.lateral_offset - last.lateral_offset).abs());
            along_jump =
                along_jump.max((st.distance - last.distance - last.speed * ocw_core::SIM_DT).abs());
            last = st;
        }
        assert!(
            h.server.session().collision_stats().bumps > 0,
            "{:?}",
            h.server.session().collision_stats()
        );
        assert!(lateral_jump < 0.06, "no sideways snap: {lateral_jump}");
        assert!(along_jump < 0.1, "no snap along: {along_jump}");
        assert!(h.client.prediction_error().abs() < ocw_sim::prediction::SNAP_THRESHOLD_M);
        assert!(h.client.own_crash(h.time_s).is_none());
        // The staged crash: nothing on the client before a snapshot says so.
        h.ride(250.0, 4.0);
        let stage = ClientCore::stage_message(true);
        for reply in h.server.handle(h.conn, &stage) {
            h.receive(&reply);
        }
        assert!(
            h.client.own_crash(h.time_s).is_none(),
            "only from the server"
        );
        h.ride(250.0, 0.4);
        let c = h.client.own_crash(h.time_s).expect("the server's crash");
        assert_eq!(c.kind, ocw_sim::collide::ContactKind::WheelTouch);
        let view = h.client.view(h.time_s, 0.0);
        assert!(view[0].crash.is_some());
        assert!(
            h.client
                .crash_notice(h.time_s)
                .unwrap()
                .starts_with("Crash!")
        );
        assert!(
            h.client
                .crash_feed(h.time_s)
                .iter()
                .any(|l| l.starts_with("You crashed")),
            "{:?}",
            h.client.crash_feed(h.time_s)
        );
        // While down the prediction holds the server's state; afterwards
        // the rider rides on from where the server put them.
        h.ride(0.0, 2.0);
        assert!(h.client.state().speed < 0.5);
        h.ride(250.0, 12.0);
        assert!(h.client.own_crash(h.time_s).is_none());
        assert!(h.client.state().speed > 3.0);
    }

    const UPGRADES: [&str; 4] = [
        "wheels.road-carbon.top",
        "tyres.road.race",
        "groupset.road-electronic.top",
        "cockpit.road.integrated",
    ];

    /// W3 (spec §14 Phase 4, 4G): the own build goes out in the hello and
    /// rides in the prediction as on the server; the server's word on it
    /// (the effect after the caps) comes back; in a race (Profile mode by
    /// default) the prediction drops the parts like the server.
    #[test]
    fn the_own_build_rides_in_the_prediction_and_the_race_mode_counts() {
        let parts: Vec<String> = UPGRADES.map(String::from).to_vec();
        let client = ClientCore::new(route(), "Fitted", RiderProfile::default()).with_parts(&parts);
        let mut h = Harness::with(
            GameServer::new(
                route(),
                ServerConfig {
                    allow_local_events: true,
                    ..Default::default()
                },
            ),
            client,
        );
        let id = h.client.rider_id().unwrap();
        h.ride(250.0, 2.0);
        let server = h.server.session().rider_profile(id).unwrap();
        assert_eq!(h.client.effective_profile(), server);
        assert!(server.cda_m2 < RiderProfile::default().cda_m2);
        let effect = h.client.own_effect().expect("the server's RiderBuild");
        let b = h.client.own_build().unwrap();
        assert!((effect.effect.flat - b.flat_gain).abs() < 1e-4);
        assert_eq!(effect.parts, b.parts);
        h.ride(250.0, 20.0);
        let (c, s) = (
            h.client.state(),
            h.server.session().rider_state(id).unwrap(),
        );
        assert!(
            (c.speed - s.speed).abs() < 0.05,
            "{} vs {}",
            c.speed,
            s.speed
        );
        // A race: Profile mode, the parts are cosmetic on both sides.
        h.send(&ClientCore::event_start_message(
            &ocw_sim::EventSpec::Race {
                laps: None,
                length_m: Some(2000.0),
            },
            None,
        ));
        h.ride(250.0, 1.0);
        assert_eq!(h.client.physics_mode(), ocw_sim::PhysicsMode::Profile);
        assert_eq!(h.client.effective_profile(), RiderProfile::default());
        assert_eq!(
            h.server.session().rider_profile(id).unwrap(),
            RiderProfile::default()
        );
        // A bike change in the race waits for its end, on both sides.
        assert!(h.client.bike_change_waits());
        let msg = h
            .client
            .change_bike("gravel", ProfileId::Gravel, RiderLook::default(), &[]);
        h.send(&msg);
        assert_eq!(h.client.profile().bike, ProfileId::Road);
        assert!(
            h.notices
                .iter()
                .any(|n| matches!(n, Notice::CommandResult(m) if m.contains("next start")))
        );
        h.send(&ClientCore::event_cancel_message());
        h.ride(250.0, 0.5);
        assert_eq!(
            h.client.profile().bike,
            ProfileId::Gravel,
            "the server's RiderBuild"
        );
        assert_eq!(h.client.own_build(), None);
        assert_eq!(
            h.client.effective_profile(),
            h.server.session().rider_profile(id).unwrap()
        );
    }

    /// The others' parts (the W2 open issue): sent once per change, drawn
    /// while the snapshots' hash matches; a joining client gets them too.
    #[test]
    fn other_riders_parts_reach_the_view() {
        let mut h = Harness::new(0, true);
        let conn = h.server.connect();
        let parts: Vec<String> = UPGRADES.map(String::from).to_vec();
        let mut other =
            ClientCore::new(route(), "Other", RiderProfile::default()).with_parts(&parts);
        for reply in h.server.handle(conn, &other.hello()) {
            other.on_server_message(&reply, 0.0);
        }
        let oid = other.rider_id().unwrap();
        h.ride(200.0, 1.0);
        let view = h.client.view(h.time_s, 1.0);
        let o = view.iter().find(|r| r.id == oid).expect("the other rider");
        let expected = other.own_build().unwrap().parts.clone();
        assert_eq!(o.parts, expected);
        assert!(view[0].parts.is_empty(), "the own stock bike");
        // A newcomer learns the build with its welcome.
        let conn2 = h.server.connect();
        let mut late = ClientCore::new(route(), "Late", RiderProfile::default());
        for reply in h.server.handle(conn2, &late.hello()) {
            late.on_server_message(&reply, 0.0);
        }
        for b in (0..5).flat_map(|_| h.server.tick()) {
            late.on_server_message(&b, 0.1);
        }
        assert_eq!(late.rider_parts(oid), expected);
        // Back to the stock bike: hash 0, nothing drawn.
        let back = other.change_bike("road", ProfileId::Road, RiderLook::default(), &[]);
        h.server.handle(conn, &back);
        h.ride(200.0, 1.0);
        let view = h.client.view(h.time_s, 1.0);
        assert!(view.iter().find(|r| r.id == oid).unwrap().parts.is_empty());
    }

    /// Others see your look (spec §14 Phase 4, 4C, W4): the character goes
    /// out once in a RiderBuild (not in the snapshots), the worn kit as
    /// catalogue ids with the parts; a change of the character alone sends
    /// it again; a joining client gets both.
    #[test]
    fn other_riders_character_and_kit_reach_the_view() {
        let mut h = Harness::new(0, true);
        let conn = h.server.connect();
        let character = ocw_core::Character {
            height_cm: 188,
            build: ocw_core::BodyBuild::Slim,
            skin: 7,
            hair: 2,
            ..Default::default()
        };
        let look = RiderLook {
            jersey: ocw_core::Rgb(0x1E88E5),
            character,
            ..Default::default()
        };
        let kit = vec![
            "helmet.tt.top".to_string(),
            "jersey.skinsuit.top".to_string(),
        ];
        let mut other = ClientCore::new(route(), "Other", RiderProfile::default())
            .with_look(look)
            .with_parts(&kit);
        for reply in h.server.handle(conn, &other.hello()) {
            other.on_server_message(&reply, 0.0);
        }
        let oid = other.rider_id().unwrap();
        h.ride(200.0, 1.0);
        let view = h.client.view(h.time_s, 1.0);
        let o = view.iter().find(|r| r.id == oid).expect("the other rider");
        assert_eq!(o.look.unwrap().character, character);
        assert_eq!(o.look.unwrap().jersey, look.jersey);
        assert_eq!(
            o.parts,
            vec!["jersey.skinsuit.top".to_string(), "helmet.tt.top".into()]
        );
        // The snapshots carry only the colours.
        let snap = h
            .server
            .tick()
            .into_iter()
            .chain((0..5).flat_map(|_| h.server.tick()))
            .find_map(|m| match ocw_protocol::decode_server(&m) {
                Ok(v1::server_message::Msg::Snapshot(s)) => Some(s),
                _ => None,
            })
            .expect("a snapshot");
        let u = snap.riders.iter().find(|u| u.rider_id == oid.0).unwrap();
        assert_eq!(u.look.as_ref().unwrap().height_cm, None);
        // A new character alone: once more in a RiderBuild.
        let bald = RiderLook {
            character: ocw_core::Character {
                hair: 5,
                ..character
            },
            ..look
        };
        let m = other.change_bike("road", ProfileId::Road, bald, &kit);
        h.server.handle(conn, &m);
        h.ride(200.0, 1.0);
        let view = h.client.view(h.time_s, 1.0);
        let o = view.iter().find(|r| r.id == oid).unwrap();
        assert_eq!(o.look.unwrap().character.hair, 5);
        // A newcomer gets the look with its welcome.
        let conn2 = h.server.connect();
        let mut late = ClientCore::new(route(), "Late", RiderProfile::default());
        for reply in h.server.handle(conn2, &late.hello()) {
            late.on_server_message(&reply, 0.0);
        }
        for b in (0..5).flat_map(|_| h.server.tick()) {
            late.on_server_message(&b, 0.1);
        }
        assert_eq!(late.rider_character(oid).map(|c| c.hair), Some(5));
        assert_eq!(late.rider_parts(oid).len(), 2);
    }

    /// Others see your kit design (spec §14 Phase 8 T6): the pattern with
    /// the snapshots' colours, the design (second and third colour, race
    /// number, kit colours, finishes) once per change in a RiderBuild, as
    /// the character; a joining client gets it too.
    #[test]
    fn other_riders_kit_design_reaches_the_view() {
        let mut h = Harness::new(0, true);
        let conn = h.server.connect();
        let design = ocw_core::KitDesign {
            second: Some(ocw_core::Rgb(0xFDD835)),
            third: Some(ocw_core::Rgb(0x1E1E22)),
            number: 77,
            helmet_pattern: true,
            shorts: Some(ocw_core::Rgb(0x3949AB)),
            gloves: Some(ocw_core::Rgb(0xF2F2EE)),
            helmet_finish: ocw_core::Finish::Matte,
            ..Default::default()
        };
        let look = RiderLook {
            jersey: ocw_core::Rgb(0x1E88E5),
            pattern: ocw_core::KitPattern::Stripes,
            design,
            ..Default::default()
        };
        let mut other = ClientCore::new(route(), "Other", RiderProfile::default()).with_look(look);
        for reply in h.server.handle(conn, &other.hello()) {
            other.on_server_message(&reply, 0.0);
        }
        let oid = other.rider_id().unwrap();
        h.ride(200.0, 1.0);
        let view = h.client.view(h.time_s, 1.0);
        let o = view.iter().find(|r| r.id == oid).expect("the other rider");
        assert_eq!(o.look, Some(look));
        // A new design alone: once more.
        let numbered = RiderLook {
            design: ocw_core::KitDesign {
                number: 5,
                ..design
            },
            pattern: ocw_core::KitPattern::Gradient,
            ..look
        };
        let m = other.change_bike("road", ProfileId::Road, numbered, &[]);
        h.server.handle(conn, &m);
        h.ride(200.0, 1.0);
        let view = h.client.view(h.time_s, 1.0);
        let o = view.iter().find(|r| r.id == oid).unwrap();
        assert_eq!(o.look, Some(numbered));
        // A newcomer: the design with the welcome (a design alone, no
        // parts and the default character, is reason enough to send it).
        let conn2 = h.server.connect();
        let mut late = ClientCore::new(route(), "Late", RiderProfile::default());
        for reply in h.server.handle(conn2, &late.hello()) {
            late.on_server_message(&reply, 0.0);
        }
        for b in (0..5).flat_map(|_| h.server.tick()) {
            late.on_server_message(&b, 0.1);
        }
        assert_eq!(late.rider_design(oid).map(|d| d.number), Some(5));
    }
}
