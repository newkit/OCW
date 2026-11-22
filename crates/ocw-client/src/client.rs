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
    /// The name the server shows and its note when it replaced the
    /// hello's (spec §14 Phase 4, 4N N1; from the Welcome).
    shown_name: Option<String>,
    name_note: Option<String>,
    rider_id: Option<RiderId>,
    predictor: Predictor,
    /// Predicted state before the last physics step: rendering interpolates
    /// between it and the current one, so motion is smooth at any frame rate.
    prev_state: RiderState,
    /// The own rider's bumps over road damage (spec Phase 8 Q12), from
    /// the prediction.
    road_bumps: crate::road_bumps::RoadBumps,
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
    /// Others flying in god mode in the last snapshot (out of the
    /// collisions, spec §14 Phase 13).
    flying_others: Vec<RiderId>,
    /// The own rider's contacts with the others are predicted (spec §14
    /// Phase 12, 6.1; on – off only for measurements).
    predict_rider_contacts: bool,
    names: BTreeMap<RiderId, (String, f64)>,
    /// Every rider's bike and look from the snapshots (no look: an old
    /// server; the renderer picks colours).
    looks: BTreeMap<RiderId, (ProfileId, Option<RiderLook>)>,
    /// The own status signs, sent with every sample (spec §2.6).
    status: v1::RiderStatus,
    /// The others' status signs from the snapshots (spec §2.6).
    statuses: BTreeMap<RiderId, v1::RiderStatus>,
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
    /// The server accepts dev commands (`Welcome.dev_commands`).
    dev_commands: bool,
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
    /// The "last moment" warning (`ride.hit_warning`, spec §14 Phase 12D).
    hit_warning: bool,
    /// Damage (spec §14 Phase 12E / 12F): every rider's from the server,
    /// the own widget; the condition the hello carries from earlier rides
    /// (Phase 12 Q6; never for anonymous riders).
    damage: crate::damage::DamageClient,
    carried: Option<v1::DamageState>,
    /// The signed-in rider's access token for the hello (spec §9.1).
    access_token: Option<String>,
    /// The other riders' photographer shots (spec §2.11, v4.38).
    photos: crate::photographer::RemoteShots,
    /// Voice blocks (spec §7.1.7 Q9): the server this client rides on
    /// (the scope of name keys, `ocw_voice::riders::server_scope`), the
    /// riders' signed-in accounts from the snapshots, and the riders the
    /// server keeps apart from this one (`VoiceApart`).
    server_scope: String,
    accounts: BTreeMap<RiderId, String>,
    voice_apart: std::collections::BTreeSet<RiderId>,
    /// The riders who cannot hear this one (`NotHearingYou`, spec §7.1.7
    /// Q17: worked out by the server per viewer): the headphones it sees.
    not_hearing_you: std::collections::BTreeSet<RiderId>,
    /// Whose voice this rider may receive and who may receive its own
    /// (`VoiceHearing`, spec §7.1.7 Q6); `None` until the server said.
    voice_hearing: Option<ocw_voice::audience::Hearing>,
    /// Where leaves lie on the roads and their season (spec 4.6.2 Q6:
    /// wet leaves cost a little speed and wobble the bike, predicted as
    /// the server rides them).
    leaf_map: Option<Arc<ocw_world::leaves::LeafMap>>,
    leaf_season: ocw_sim::leaves::LeafSeason,
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
            shown_name: None,
            name_note: None,
            rider_id: None,
            predictor: Predictor::new(profile, RiderState::default()),
            prev_state: RiderState::default(),
            road_bumps: Default::default(),
            lateral_target: None,
            lean: None,
            wild: false,
            autodrive_at_start: true,
            steer_seq: 0,
            steer_sent: None,
            others: SnapshotBuffer::default(),
            others_now: Vec::new(),
            flying_others: Vec::new(),
            predict_rider_contacts: true,
            names: BTreeMap::new(),
            looks: BTreeMap::new(),
            status: v1::RiderStatus::default(),
            statuses: BTreeMap::new(),
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
            dev_commands: false,
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
            hit_warning: true,
            damage: Default::default(),
            carried: None,
            access_token: None,
            photos: Default::default(),
            server_scope: ocw_voice::riders::LOCAL_SCOPE.into(),
            accounts: BTreeMap::new(),
            voice_apart: Default::default(),
            not_hearing_you: Default::default(),
            voice_hearing: None,
            leaf_map: None,
            leaf_season: Default::default(),
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
        let mut core = Self::new(placeholder, name, profile);
        // The own rider's contacts with the world are predicted (spec §14
        // Phase 12D, K3).
        core.predictor.set_world(net.obstacles().cloned());
        core.leaf_map = net.leaves().cloned();
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
            ..core
        }
    }

    /// A route ride on the road graph (spec §2.7): the own path follows
    /// `plan` (the server's session rides the same route).
    pub fn with_route_plan(mut self, plan: Arc<RoutePlan>) -> Self {
        if let Some(fr) = &mut self.free {
            let v = self.route.points()[0];
            let route = Route::from_points(
                plan.name.clone(),
                plan.name.clone(),
                vec![v, [v[0] + 1.0, v[1], v[2]]],
                false,
                5.0,
            )
            .expect("two distinct points");
            // The placeholder keeps the package's geographic origin: the
            // GPS line, the FIT track and the sun need it (it was lost
            // here from fae1547 to spec 3.40: no GPS line in route rides).
            self.route = match self.route.geo_origin() {
                Some(origin) => route.with_geo_origin(origin),
                None => route,
            };
            self.event = EventClient::new(plan.name.clone());
            fr.plan = Some(plan);
        }
        self
    }

    /// The same client with a cosmetic look (spec §2.5), sent in the hello.
    /// The condition the rider brings from earlier rides (spec §14 Phase
    /// 12 Q6), sent in the hello; `None`: fresh.
    pub fn with_damage(mut self, carried: Option<v1::DamageState>) -> Self {
        self.carried = carried;
        self
    }

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
            damage: self.carried.clone(),
            access_token: self.access_token.clone(),
        })))
    }

    /// The access token for the hello (spec §9.1; `None`: anonymous).
    pub fn set_access_token(&mut self, token: Option<String>) {
        self.access_token = token;
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
            DeviceEvent::Connected { .. }
            | DeviceEvent::Disconnected
            | DeviceEvent::Failed { .. } => return None,
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
            status: Some(self.status.clone()),
        })))
    }

    /// The own status signs (spec §2.6 *Rider status signs*): the menu, the
    /// microphone, VR, whom the rider does not hear. Returns a sample to
    /// send at once when they changed (the others see the change within a
    /// snapshot, not only with the next trainer sample).
    pub fn set_status(&mut self, status: v1::RiderStatus) -> Option<Vec<u8>> {
        if status == self.status {
            return None;
        }
        self.status = status;
        self.rider_id?;
        Some(self.sample())
    }

    /// The own status signs as sent.
    pub fn status(&self) -> &v1::RiderStatus {
        &self.status
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
                self.shown_name = Some(w.display_name.clone()).filter(|n| !n.is_empty());
                self.name_note = w.name_note.clone();
                self.voice_offered = w.voice;
                // A new session: its server says again whom to keep apart.
                self.voice_apart.clear();
                self.not_hearing_you.clear();
                self.voice_hearing = None;
                self.accounts.clear();
                self.echo_bot = w.echo_bot;
                self.dev_commands = w.dev_commands;
                self.world_access = Some(ocw_protocol::access_from_wire(w.world_access));
                self.god_access = Some(ocw_protocol::access_from_wire(w.god_access));
                self.collisions.on_welcome(&w);
                self.damage.on_welcome(&w);
                // A ghost to the others again for the first seconds, as
                // the server keeps a joining rider (spec §14 Phase 12A).
                self.predictor.rejoin();
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
            S::VoiceApart(a) => {
                self.voice_apart = a.riders.into_iter().map(RiderId).collect();
                None
            }
            S::NotHearingYou(n) => {
                self.not_hearing_you = n.riders.into_iter().map(RiderId).collect();
                None
            }
            S::VoiceHearing(h) => {
                self.voice_hearing = Some(ocw_voice::audience::Hearing::from_wire(&h));
                None
            }
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
            S::Photo(p) => {
                if let Some(shot) = ocw_protocol::photo_from_wire(&p)
                    && Some(RiderId(shot.rider)) != self.rider_id
                {
                    let t = self.photo_time_s(now_s).unwrap_or(0.0);
                    self.photos.on_message(shot, t);
                }
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

    /// The season of the leaves on the road at client time `now_s` (spec
    /// 4.6.2 Q6), as the server takes it: the world clock's day at the
    /// ride's place. Without a leaf map or a clock: none.
    fn refresh_leaves(&mut self, now_s: f64) {
        let Some(map) = self.leaf_map.clone() else {
            return;
        };
        let Some(ms) = self.world_ms(now_s) else {
            return;
        };
        let place = self.geo_origin().unwrap_or(ocw_sky::zone::DEFAULT_PLACE);
        let day = self.leaf_season.day_at(ms, place);
        self.predictor
            .set_leaves(Some(ocw_sim::leaves::LeafGround { map, day }));
    }

    /// The wet leaves' wobble on the lean of a rider in `state` on `track`
    /// at client time `now_s` (spec 4.6.2 Q6; rad).
    fn leaf_lean(&self, now_s: f64, track: &dyn Track, state: &RiderState) -> f64 {
        let Some(ground) = self.predictor.leaves() else {
            return 0.0;
        };
        let Some(w) = self.weather.physics_now(now_s) else {
            return 0.0;
        };
        if !self.weather.physics_on() {
            return 0.0;
        }
        let s = ocw_sim::leaves::slick(Some(ground), Some(&w), track, state);
        ocw_sim::leaves::wobble_lean(s, state.speed, state.distance)
    }

    /// The slickness of the wet leaves under the own rider now (0 – 1,
    /// spec 4.6.2 Q6; for tests and the console).
    pub fn leaf_slick(&self, now_s: f64) -> f64 {
        let (Some(ground), Some(w)) = (self.predictor.leaves(), self.weather.physics_now(now_s))
        else {
            return 0.0;
        };
        if !self.weather.physics_on() {
            return 0.0;
        }
        ocw_sim::leaves::slick(
            Some(ground),
            Some(&w),
            self.track(),
            &self.predictor.state(),
        )
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

    /// Who sets the weather, for messages: "you", "the host", "the
    /// organizer", "the event's organizer" (spec §14 Phase 9, S5: an
    /// event's weather is locked until it is over).
    pub fn weather_control(&self) -> &'static str {
        let event = self
            .event_info()
            .is_some_and(|e| e.weather.is_some() && e.phase != ocw_sim::EventPhase::Finished);
        match self.weather.info().map(|w| w.access) {
            _ if self.may_change_weather() => "you",
            Some(ocw_protocol::ClockAccess::Nobody) if event => "the event's organizer",
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
        self.refresh_leaves(now_s);
        self.human_riders = s.human_riders;
        self.echo_rider = ocw_protocol::echo_rider(s);
        if let Some(c) = &s.clock {
            self.on_clock(c, now_s);
        }
        if let Some(weather) = &s.weather {
            self.weather.on_message(weather, now_s, self.snapshot_age_s);
        }
        self.collisions.on_snapshot(s, now_s, self.rider_id);
        self.damage.on_snapshot(s, now_s, self.rider_id);
        // The damage's effects: the prediction rides as the server does.
        self.predictor
            .set_damage(self.damage.effects(self.rider_id));
        let mut others = Vec::new();
        let mut gods = Vec::new();
        let mut reconcile_with = None;
        let mut own_seen = false;
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
            // Accounts come once a second (spec §7.1.7 Q9): kept between.
            if !u.account_id.is_empty() {
                self.accounts.insert(id, u.account_id.clone());
            }
            self.looks.insert(id, ocw_protocol::rider_look(u));
            match &u.status {
                Some(st) => {
                    self.statuses.insert(id, st.clone());
                }
                None => {
                    self.statuses.remove(&id);
                }
            }
            self.build_hashes.insert(id, u.build_hash);
            if Some(id) == self.rider_id {
                own_seen = true;
                self.hearts = self.hearts.max(u.hearts);
                self.event.on_own_snapshot(s.server_tick, state.distance);
                if self.lateral_target.is_none() {
                    // Start steering from the lane the server assigned.
                    self.lateral_target = Some(state.lateral_offset);
                    self.predictor.set_lateral_target(state.lateral_offset);
                }
                // Down after a crash (spec §14 Phase 12C): the server's state
                // (a crash is only shown when the server says so).
                // Stopped by the road (a repair, a medical stop, spec §14
                // Phase 12E) counts the same.
                let down = self.collisions.crash(id, now_s).is_some() || self.damage.held(Some(id));
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
                    // After the loop: with the others of this snapshot.
                    reconcile_with = Some(state);
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
        self.accounts
            .retain(|id, _| s.riders.iter().any(|u| u.rider_id == id.0));
        let flying: Vec<RiderId> = gods.iter().map(|(id, _)| *id).collect();
        if let Some(state) = reconcile_with {
            // The own rider's contacts with the others are predicted from
            // where this snapshot shows them (spec §14 Phase 12, 6.1).
            let (frame, bodies) = self.other_bodies(&others, &flying);
            self.predictor.set_others(frame, bodies);
            let power = self.power_w();
            let track: &dyn Track = match self.free.as_ref().and_then(|f| f.own.as_deref()) {
                Some(p) => p,
                None => &self.route,
            };
            self.predictor
                .reconcile(track, state, self.snapshot_age_s, power);
        }
        if own_seen {
            // A snap (teleport, large error) must not be interpolated.
            if self.predictor.last_error().abs() > ocw_sim::prediction::SNAP_THRESHOLD_M {
                self.prev_state = self.predictor.state();
            }
        }
        // The others' last known places now: the snapshot's, moved on by
        // its age (half the round trip).
        let age = self.snapshot_age_s;
        self.others_now = others
            .iter()
            .map(|(id, st)| {
                let mut st = *st;
                if !self.collisions.crashed(*id) {
                    st.distance += st.speed * age;
                }
                (*id, st)
            })
            .collect();
        self.flying_others = flying;
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

    /// The autodrive line (m from the road's middle, positive = right),
    /// once it was set.
    pub fn line_target(&self) -> Option<f64> {
        self.lateral_target
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
            // (Stopped by the road after damage, spec §14 Phase 12E, is
            // `was_down` too: the server's state.)
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
        // The weather's physics as the server rides it (spec §14 Phase 9,
        // S6): the wind, the air, the road; in a crosswind the shelter
        // behind the others drifts downwind.
        let weather = self.weather.physics_tick();
        self.predictor.set_weather(weather);
        let drift = weather.map(|w| {
            let track: &dyn Track = match self.free.as_ref().and_then(|f| f.own.as_deref()) {
                Some(p) => p,
                None => &self.route,
            };
            let here = ocw_sim::weather::at_rider(&w, track, &me);
            ocw_sim::weather::wake_drift(here.wind, me.speed)
        });
        let drag = match drift {
            None => ocw_sim::drafting::drag_factor((me.distance, me.lateral_offset), others),
            Some(drift) => ocw_sim::drafting::drag_factor_in_wind(
                (me.distance, me.lateral_offset),
                others,
                drift,
            ),
        };
        self.predictor.set_drag_factor(drag);
        self.prev_state = self.predictor.state();
        let power = self.power_w();
        self.predictor.set_collision_mode(self.collisions.mode());
        let (frame, bodies) = self.other_bodies(&self.others_now, &self.flying_others);
        self.predictor.set_others(frame, bodies);
        let track: &dyn Track = match self.free.as_ref().and_then(|f| f.own.as_deref()) {
            Some(p) => p,
            None => &self.route,
        };
        self.predictor.step(track, power);
        // Road damage under the wheel (spec Phase 8 Q12): the kick, the
        // shake and the sound of a bump.
        self.road_bumps
            .observe(track, &self.prev_state, &self.predictor.state());
        // The predicted contacts moved the others too (until the next
        // snapshot says where they are).
        for (id, r) in self.predictor.other_responses() {
            if let Some((_, o)) = self.others_now.iter_mut().find(|(o, _)| o == id) {
                o.distance += r.ds;
                o.lateral_offset += r.dd;
                if let Some(v) = r.speed {
                    o.speed = v;
                }
                if let Some(h) = r.heading {
                    o.heading = h;
                }
            }
        }
        for (_, o) in &mut self.others_now {
            o.distance += o.speed * ocw_core::SIM_DT;
        }
    }

    /// The other riders as the prediction's collision bodies (spec §14
    /// Phase 12, 6.1): from their last known road states, in a route's
    /// path frame or – in free ride, on their own paths – the world's
    /// plane. Riders flying in god mode and the Echo rider are not solid
    /// (as on the server), fallen riders are obstacles. Their masses are
    /// not sent: the default profile's (the server's response corrects
    /// the difference).
    fn other_bodies(
        &self,
        states: &[(RiderId, RiderState)],
        flying: &[RiderId],
    ) -> (ocw_sim::collide::Frame, Vec<ocw_sim::collide::Body>) {
        use ocw_sim::collide::{Body, Frame};
        let frame = if self.free.is_some() {
            Frame::World
        } else {
            Frame::Path {
                loop_m: self.route.is_loop().then(|| self.route.length()),
            }
        };
        let own_path = self.free.as_ref().is_none_or(|f| f.own.is_some());
        if !self.predict_rider_contacts || !own_path {
            return (frame, Vec::new());
        }
        let profile = RiderProfile::default();
        let mass = profile.rider_mass_kg + profile.bike_mass_kg;
        let bodies = states
            .iter()
            .filter(|(id, _)| !flying.contains(id) && Some(*id) != self.echo_rider)
            .filter_map(|(id, st)| {
                let (pos, road) = match &self.free {
                    Some(fr) => {
                        let p = fr.others.get(id)?.sample(st.distance);
                        let at = p.offset_position(st.lateral_offset);
                        ([at[0], at[2]], p.direction)
                    }
                    None => ([st.distance, st.lateral_offset], [1.0, 0.0]),
                };
                Some(Body {
                    id: *id,
                    pos,
                    road,
                    heading: st.heading,
                    speed: st.speed,
                    lean: ocw_sim::collide::lean_of(st.speed, st.turn_curvature),
                    mass,
                    fallen: self.collisions.crashed(*id),
                    manual: false,
                    grip: 1.0,
                })
            })
            .collect();
        (frame, bodies)
    }

    /// Whether the own rider's contacts with the others are predicted
    /// (on; off only to measure what the prediction gains, spec §14
    /// Phase 12).
    pub fn set_predict_rider_contacts(&mut self, on: bool) {
        self.predict_rider_contacts = on;
    }

    /// The wind relative to the own rider at client time `now_s` (spec §14
    /// Phase 9, S6: the HUD's arrow; whether it affects the physics or
    /// not; calm in a tunnel). `None` before the welcome.
    pub fn rider_wind(&self, now_s: f64) -> Option<ocw_sim::weather::RelativeWind> {
        self.rider_id?;
        let w = self.weather.physics_now(now_s)?;
        Some(ocw_sim::weather::at_rider(&w, self.track(), &self.predictor.state()).wind)
    }

    /// The weather's load for the trainer in SIM mode (spec §14 Phase 9,
    /// S6, Q5): the headwind along the own rider's travel and the air's
    /// density, while the weather affects the physics.
    pub fn trainer_wind(&self, now_s: f64) -> Option<ocw_sim::trainer::TrainerWind> {
        self.rider_id?;
        if !self.weather.physics_on() {
            return None;
        }
        let w = self.weather.physics_now(now_s)?;
        let here = ocw_sim::weather::at_rider(&w, self.track(), &self.predictor.state());
        Some(ocw_sim::trainer::TrainerWind {
            head_mps: here.wind.head_mps,
            air_density: here.density,
        })
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
        let own_state = lerp_state(self.prev_state, self.predictor.state(), alpha);
        let own_path = self.free.as_ref().and_then(|f| f.own.as_deref());
        let mut riders = vec![RiderView {
            id: own,
            name: self
                .names
                .get(&own)
                .map(|n| n.0.clone())
                .unwrap_or_default(),
            state: own_state,
            power_w: self.power_w(),
            cadence_rpm: self.cadence_rpm,
            heart_rate_bpm: self.heart_rate_bpm,
            is_local: true,
            // The own bike and look are known locally.
            bike: self.profile.bike,
            look: Some(self.look),
            parts: self.rider_parts(own),
            crash: self.collisions.crash(own, now_s),
            wobble: self.collisions.wobble(own, tick_now)
                + self.leaf_lean(now_s, self.track(), &own_state),
            damage: self.damage.visible(own),
            cue: self.anim_cue(own, true, &own_state, own_path, tick_now),
            // The own rider never sees headphones on itself.
            signs: crate::voice::signs_seen(Some(own), &self.status, own, &self.status, false),
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
            let others_tick = tick_now - INTERPOLATION_DELAY_S / ocw_core::SIM_DT;
            let path = self
                .free
                .as_ref()
                .and_then(|f| f.others.get(&id))
                .map(|p| &**p);
            let cue = self.anim_cue(id, false, &state, path, others_tick);
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
                damage: self.damage.visible(id),
                // Others are drawn slightly in the past.
                wobble: self.collisions.wobble(id, others_tick)
                    + match path {
                        Some(p) => self.leaf_lean(now_s, p, &state),
                        None if self.free.is_none() => self.leaf_lean(now_s, &self.route, &state),
                        None => 0.0,
                    },
                cue,
                signs: self.statuses.get(&id).map_or_else(Default::default, |st| {
                    let told = self.not_hearing_you.contains(&id);
                    crate::voice::signs_seen(Some(own), &self.status, id, st, told)
                }),
            });
        }
        riders
    }

    /// What a rider's animation needs beyond its state (spec §2.6): free
    /// ride's U-turn loop on its path, the finish of an event and its
    /// winner, the metres to the line; at server tick `tick`.
    fn anim_cue(
        &self,
        id: RiderId,
        own: bool,
        state: &RiderState,
        path: Option<&RidePath>,
        tick: f64,
    ) -> ocw_render::AnimCue {
        let mut cue = self.event.anim_cue(id, own, state.distance, tick);
        cue.uturn = path
            .and_then(|p| p.uturn_phase(state.distance))
            .map(|(progress, side)| ocw_render::UTurnCue { progress, side });
        cue
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
                grip: ocw_sim::weather::grip(self.predictor.weather(), self.track(), me),
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
        encode(&ocw_protocol::rules_request(collisions, chaotic_bots, None))
    }

    /// `ride.damage` for the server (spec §14 Phase 12A, 12E).
    pub fn damage_rules_message(on: bool) -> Vec<u8> {
        encode(&ocw_protocol::rules_request(None, None, Some(on)))
    }

    /// Mend a puncture now (spec §14 Phase 12E; key R while riding with a
    /// flat, console `ride.repair`).
    pub fn repair_message() -> Vec<u8> {
        encode(&ocw_protocol::damage_request(true))
    }

    /// `dev.damage TARGET [PERCENT]` (local servers only).
    pub fn stage_damage_message(target: &str, condition: f64) -> Vec<u8> {
        encode(&ocw_protocol::stage_damage(target, condition))
    }

    /// The damage state of the client (every rider's, the switch).
    pub fn damage_state(&self) -> &crate::damage::DamageClient {
        &self.damage
    }

    /// Damage in force (the server says).
    pub fn damage_on(&self) -> bool {
        self.damage.on()
    }

    /// The own rider's damage as the server sent it last.
    pub fn own_damage(&self) -> Option<&ocw_sim::damage::Damage> {
        self.damage.damage(self.rider_id?)
    }

    /// Stopped by the road (a repair, a medical stop, a spare bike): SIM
    /// flat, like down after a crash.
    pub fn damage_held(&self) -> bool {
        self.damage.held(self.rider_id)
    }

    /// The HUD's damage widget at client time `now_s` (spec §14 Phase 12F).
    pub fn damage_widget(&self, now_s: f64) -> Option<ocw_render::damage_hud::DamageWidgetView> {
        self.damage.widget(self.rider_id, now_s)
    }

    /// The HUD's damage line ("Medical stop: 42 s", "Puncture: …").
    pub fn damage_notice(&self) -> Option<String> {
        self.damage.notice(self.rider_id)
    }

    /// A staged contact (`dev.bump` / `dev.crash`, local servers only).
    pub fn stage_message(crash: bool) -> Vec<u8> {
        encode(&ocw_protocol::stage_contact(crash))
    }

    /// `dev.tree` for the local server (spec §14 Phase 12 K3).
    pub fn stage_tree_message() -> Vec<u8> {
        encode(&ocw_protocol::stage_tree())
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
        if !self.proximity_cue || !self.collisions.mode().on() || !self.aids_allowed() {
            return None;
        }
        crate::collisions::proximity(
            self.others_near()
                .into_iter()
                .filter(|o| !o.fallen)
                .map(|o| (o.ahead_m, o.right_m)),
        )
    }

    /// The proximity arrow (12D, with the proximity cue): the direction to
    /// a rider close beside, in front or behind (rad: 0 ahead, positive to
    /// the right).
    pub fn proximity_arrow(&self) -> Option<f32> {
        if !self.proximity_cue
            || !self.collisions.mode().on()
            || self.own_down()
            || !self.aids_allowed()
        {
            return None;
        }
        crate::aids::proximity_direction(&self.others_near()).map(|a| a as f32)
    }

    /// The "last moment" warning setting (`ride.hit_warning`, on by
    /// default; spec §14 Phase 12D).
    pub fn set_hit_warning(&mut self, on: bool) {
        self.hit_warning = on;
    }

    pub fn hit_warning_on(&self) -> bool {
        self.hit_warning
    }

    /// The "last moment" warning now (12D): a predicted hit within a
    /// second, or the tyres sliding on a wet road; `None` with the
    /// setting off, collisions off, while down or flying.
    pub fn hit_warning(&self) -> Option<ocw_render::HitWarningView> {
        if !self.hit_warning
            || !self.collisions.mode().on()
            || self.own_down()
            || self.god.active()
            || !self.aids_allowed()
        {
            return None;
        }
        let me = self.predictor.state();
        crate::aids::hit_warning(
            crate::aids::Me {
                speed: me.speed,
                heading: me.heading,
            },
            &self.others_near(),
            self.predictor.slip(),
        )
    }

    /// Whether the riding aids may be shown (spec §14 Phase 12 K6): not
    /// while the own rider rides in an event whose organiser does not allow
    /// them (a race by default) – until the own finish or DNF.
    pub fn aids_allowed(&self) -> bool {
        let Some(info) = self.event.info() else {
            return true;
        };
        if info.aids || info.phase == ocw_sim::EventPhase::Finished {
            return true;
        }
        !self.rider_id.is_some_and(|id| {
            info.standings
                .iter()
                .any(|s| s.id == id && s.finish_s.is_none() && !s.dnf)
        })
    }

    /// Whether the own rider was down in the last snapshot.
    fn own_down(&self) -> bool {
        self.rider_id.is_some_and(|id| self.collisions.crashed(id))
    }

    /// The other riders near the own rider in its path frame (metres ahead
    /// along its path, metres to the right; within 80 m): on a route by
    /// the distance (a loop wraps), in free ride where the own path passes
    /// them. Riders flying in unicorn mode are left out.
    pub fn others_near(&self) -> Vec<crate::aids::Near> {
        let me = self.predictor.state();
        let loop_m = (self.free.is_none() && self.route.is_loop()).then(|| self.route.length());
        let own_path = self
            .free
            .as_ref()
            .and_then(|f| f.own.as_ref().map(|o| (f, o)));
        let mut out = Vec::new();
        for (id, o) in &self.others_now {
            if self.god_others.0.contains_key(id) {
                continue;
            }
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
                None if self.free.is_some() => continue,
                None => o.distance,
            };
            let mut ahead = mine - me.distance;
            if let Some(l) = loop_m {
                ahead -= l * (ahead / l).round();
            }
            if ahead.abs() > 80.0 {
                continue;
            }
            out.push(crate::aids::Near {
                ahead_m: ahead,
                right_m: o.lateral_offset - me.lateral_offset,
                speed: o.speed,
                heading: o.heading,
                fallen: self.collisions.crashed(*id),
            });
        }
        out
    }

    /// The chase camera's reaction (spec §14 Phase 12C) at client time
    /// `now_s`: the shake after the own rider's crash or a hard bump
    /// (`shake` off: motion comfort) and the wider orbit while down.
    pub fn camera_fx(&self, now_s: f64, shake: bool) -> ocw_render::CameraFx {
        let Some(id) = self.rider_id else {
            return ocw_render::CameraFx::default();
        };
        let crash = self.collisions.crash(id, now_s);
        let tick = self.server_tick_now(now_s);
        let bump = self.collisions.bump(id);
        let mut fx = crate::camera_fx::camera_fx(
            crash.as_ref(),
            bump.as_ref()
                .map(|b| (b, (tick - b.tick as f64) * ocw_core::SIM_DT)),
            shake,
        );
        // A bump over road damage (spec Phase 8 Q12): a small shake.
        if let Some(seen) = self.road_bumps.last()
            && shake
            && crash.is_none()
        {
            crate::camera_fx::add_shake(&mut fx, seen.age_s(now_s), seen.shake_strength());
        }
        fx
    }

    /// The own rider's bumps over road damage (spec Phase 8 Q12).
    pub fn road_bumps(&self) -> &crate::road_bumps::RoadBumps {
        &self.road_bumps
    }

    /// The game time now (s): the server's tick time (tick / 50), estimated
    /// from the last snapshot – the time base every client shares (the
    /// ride photographer's shots, spec §2.11); `None` before the first
    /// snapshot (the welcome comes first: a 0 there would start the
    /// photographer's ride at the session's start).
    pub fn server_time_s(&self, now_s: f64) -> Option<f64> {
        self.last_snapshot_at?;
        Some(self.server_tick_now(now_s) * ocw_core::SIM_DT)
    }

    /// The game time the other riders are drawn at (s): slightly in the
    /// past, as their interpolated places; their photographers are drawn
    /// at it too.
    pub fn photo_time_s(&self, now_s: f64) -> Option<f64> {
        Some(self.server_time_s(now_s)? - INTERPOLATION_DELAY_S)
    }

    /// The other riders' photographer shots (spec §2.11).
    pub fn remote_shots(&self) -> &crate::photographer::RemoteShots {
        &self.photos
    }

    /// Forgets the other riders' shots that are long over.
    pub fn prune_shots(&mut self, now_s: f64) {
        if let Some(t) = self.photo_time_s(now_s) {
            self.photos.prune(t);
        }
    }

    /// The message announcing or updating the own photographer's shot.
    pub fn photo_message(shot: &ocw_world::photo_share::SharedShot) -> Vec<u8> {
        ocw_protocol::encode(&ocw_protocol::photo_message(shot))
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
        // Damage dots and crashes in the results (spec §14 Phase 12F).
        if let Some(res) = &mut v.results {
            let conditions = self.event.info().map(|i| &i.conditions);
            for row in &mut res.rows {
                let id = RiderId(row.rider);
                row.damage = self.damage.dot(id);
                row.crashes = conditions
                    .and_then(|c| c.iter().find(|(r, ..)| *r == id).map(|(_, n, ..)| *n))
                    .or_else(|| self.damage.damage(id).map(|d| d.crashes))
                    .unwrap_or(0);
            }
        }
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

    /// Whether the own rider is an entrant of a running event (spec §14
    /// Phase 4, 4N Q15: the name is frozen then).
    pub fn in_running_event(&self) -> bool {
        self.event_info().is_some_and(|i| {
            i.phase == ocw_sim::EventPhase::Running
                && i.standings.iter().any(|s| Some(s.id) == self.rider_id)
        })
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
        Self::event_start_message_weather(spec, bots, physics, None)
    }

    /// An event start with the organiser's weather (spec §14 Phase 9, S5:
    /// `event … --weather PRESET|custom …|auto SEED`).
    pub fn event_start_message_weather(
        spec: &ocw_sim::EventSpec,
        bots: Option<u32>,
        physics: Option<ocw_sim::PhysicsMode>,
        weather: Option<&ocw_weather::machine::EventWeather>,
    ) -> Vec<u8> {
        Self::event_start_message_rules(spec, bots, physics, weather, None, None)
    }

    /// An event start with the organiser's weather, damage rules (spec
    /// §14 Phase 12E, 12G: `event … --damage on|off --dnf-injured
    /// --neutral-service`) and weather-physics rule (spec §14 Phase 9,
    /// S6, Q4: `event … --weather-physics on`; `None`: off, the default).
    pub fn event_start_message_rules(
        spec: &ocw_sim::EventSpec,
        bots: Option<u32>,
        physics: Option<ocw_sim::PhysicsMode>,
        weather: Option<&ocw_weather::machine::EventWeather>,
        damage: Option<ocw_sim::damage::Rules>,
        weather_physics: Option<bool>,
    ) -> Vec<u8> {
        let rules = ocw_sim::event::Rules {
            physics,
            weather_physics,
            damage,
            ..Default::default()
        };
        Self::event_start_message_organiser(spec, bots, weather, &rules)
    }

    /// An event start with all the organiser's rules (spec §14 Phase 12
    /// K6: `event … --collisions --damage --aids --chaotic-bots
    /// --time-limit --neutral-service --service-s`, the Organiser rules
    /// page) and the organiser's weather.
    pub fn event_start_message_organiser(
        spec: &ocw_sim::EventSpec,
        bots: Option<u32>,
        weather: Option<&ocw_weather::machine::EventWeather>,
        rules: &ocw_sim::event::Rules,
    ) -> Vec<u8> {
        let start = ocw_protocol::with_event_rules(ocw_protocol::event_start(spec, bots), rules);
        encode(&client_message(C::Event(v1::EventRequest {
            req: Some(v1::event_request::Req::Start(v1::EventStart {
                weather: weather.map(ocw_protocol::event_weather_to_wire),
                ..start
            })),
        })))
    }

    /// `dev.bot_bike` (spec §14 Phase 4, W3; local servers): a bot's
    /// bike and parts, steady power, lane and name.
    pub fn bot_bike_message(bike: v1::BotBike) -> Vec<u8> {
        encode(&client_message(C::Dev(v1::DevCommand {
            cmd: Some(v1::dev_command::Cmd::BotBike(bike)),
        })))
    }

    /// `dev.bot_status` (spec §2.6; local servers): the status signs of
    /// bot `rider_id` (0: every bot).
    pub fn bot_status_message(rider_id: u32, status: v1::RiderStatus) -> Vec<u8> {
        encode(&client_message(C::Dev(v1::DevCommand {
            cmd: Some(v1::dev_command::Cmd::BotStatus(v1::BotStatus {
                rider_id,
                status: Some(status),
            })),
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

    /// The own voice blocks for the server (spec §7.1.7 Q9): every
    /// blocked account and the names blocked on this server.
    pub fn voice_blocks_message(account_ids: Vec<String>, names: Vec<String>) -> Vec<u8> {
        encode(&client_message(C::VoiceBlocks(v1::VoiceBlocks {
            account_ids,
            names,
        })))
    }

    /// The own voice audience for the server (spec §7.1.7 Q8, §7.1.3):
    /// *Who hears me*, *Voice room* and the selected riders.
    pub fn voice_audience_message(a: v1::VoiceAudience) -> Vec<u8> {
        encode(&client_message(C::VoiceAudience(a)))
    }

    /// The server this client rides on, for the voice choices remembered
    /// by name (`ocw_voice::riders::server_scope` of the address joined;
    /// `local` for the app's own servers).
    pub fn set_server_scope(&mut self, spec: &str) {
        self.server_scope = ocw_voice::riders::server_scope(spec);
    }

    pub fn server_scope(&self) -> &str {
        &self.server_scope
    }

    /// The other riders' names and signed-in accounts (from the
    /// snapshots): the keys of the voice choices.
    pub fn rider_identities(&self) -> (BTreeMap<RiderId, String>, BTreeMap<RiderId, String>) {
        let names = self
            .names
            .iter()
            .map(|(id, (n, _))| (*id, n.clone()))
            .collect();
        (names, self.accounts.clone())
    }

    /// The riders the server keeps apart from this one in voice (a block
    /// in either direction, spec §7.1.7 Q9).
    pub fn voice_apart(&self) -> &std::collections::BTreeSet<RiderId> {
        &self.voice_apart
    }

    /// The other riders' status signs as received (tests: no list of whom
    /// a rider does not hear arrives, spec §7.1.7 Q17).
    #[cfg(test)]
    pub(crate) fn statuses_for_test(
        &self,
    ) -> impl Iterator<Item = (&RiderId, &v1::RiderStatus)> + '_ {
        self.statuses.iter()
    }

    /// The riders the server says cannot hear this one (spec §7.1.7 Q17):
    /// the headphones this rider sees.
    pub fn not_hearing_you(&self) -> &std::collections::BTreeSet<RiderId> {
        &self.not_hearing_you
    }

    /// The server's last `VoiceHearing` of this session (spec §7.1.7 Q6).
    pub fn voice_hearing(&self) -> Option<&ocw_voice::audience::Hearing> {
        self.voice_hearing.as_ref()
    }

    /// The echo test's request (spec §7 *Echo bot*).
    pub fn echo_request_message(on: bool, here: bool) -> Vec<u8> {
        encode(&ocw_protocol::echo_request(on, here))
    }

    /// The server lets riders put an Echo rider on the road.
    pub fn echo_bot_allowed(&self) -> bool {
        self.echo_bot
    }

    /// The server accepts dev commands (`Welcome.dev_commands`: its local
    /// server, or `ocw-server --dev-commands` for tests): the apps then
    /// allow their dev console commands for it.
    pub fn dev_commands_allowed(&self) -> bool {
        self.dev_commands
    }

    /// The session's Echo rider (from the last snapshot).
    pub fn echo_rider(&self) -> Option<RiderId> {
        self.echo_rider
    }

    pub fn rider_id(&self) -> Option<RiderId> {
        self.rider_id
    }

    /// The name others see (spec §14 Phase 4, 4N N1): the server's from
    /// the Welcome, else the hello's.
    pub fn shown_name(&self) -> &str {
        self.shown_name.as_deref().unwrap_or(&self.name)
    }

    /// Why the server replaced the hello's name (`Rider NNNN`), if it did.
    pub fn name_note(&self) -> Option<&str> {
        self.name_note.as_deref()
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

    /// Whether the power device is the virtual trainer (no trainer or
    /// power meter connected; the touch power buttons show, spec 3.2.3).
    pub fn virtual_device(&self) -> bool {
        self.virtual_device
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

    /// [`Self::change_bike`] with the bike's condition from the garage
    /// (spec §14 Phase 12 Q6, Phase 4 W6: a repair or a swap reaches the
    /// ride).
    pub fn change_bike_with_damage(
        &mut self,
        bike_id: &str,
        bike: ProfileId,
        look: RiderLook,
        parts: &[String],
        damage: Option<v1::DamageState>,
    ) -> Vec<u8> {
        let _ = self.change_bike(bike_id, bike, look, parts);
        let mut m = ocw_protocol::bike_change(bike_id, bike, look, parts);
        if let Some(v1::client_message::Msg::BikeChange(b)) = &mut m.msg {
            b.damage = damage;
        }
        encode(&m)
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
        assert!(!h.client.dev_commands_allowed(), "the welcome says so");
        let local = Harness::new(0, true);
        assert!(local.client.dev_commands_allowed());
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
            auth: false,
            name_taken: false,
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
        // The U-turn's animation cue (spec §2.6): the run-up, then the
        // loop, in the own rider's view.
        let mut seen = (false, false);
        for _ in 0..100 {
            h.ride(250.0, 0.25);
            if let Some(u) = h.client.view(h.time_s, 0.0)[0].cue.uturn {
                seen.0 |= u.progress < 0.0;
                seen.1 |= u.progress > 0.3 && u.progress < 0.7;
            }
        }
        assert_eq!(seen, (true, true), "the run-up and the loop");
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

    /// Regression (spec 3.40): `with_route_plan` replaced the route with a
    /// placeholder without the package's geographic origin (since
    /// fae1547), so route rides on world packages – desktop `--world PKG
    /// --route NAME`, the browser's `?world=…&route=…` – had no GPS line.
    /// The view fill sets the GPS position in a route ride.
    #[test]
    fn a_route_ride_keeps_its_geo_origin_for_the_gps_line() {
        let origin = (47.14, 9.52);
        let net = Arc::new(RoadNetwork::new(grid()).with_package_name("Grid"));
        let plan = grid_route(&net);
        let c = ClientCore::free_ride(net.clone(), "Routed", RiderProfile::default())
            .with_geo_origin(origin)
            .with_route_plan(plan.clone());
        assert_eq!(c.geo_origin(), Some(origin));
        let server = GameServer::new(
            ocw_server::Course::RouteRide {
                net: net.clone(),
                plan: plan.clone(),
            },
            ServerConfig {
                bots: 0,
                start_m: 0.0,
                voice: None,
                ..Default::default()
            },
        );
        let mut h = Harness::with(server, c);
        h.ride(250.0, 5.0);
        let mut view = ocw_render::RideView::new(Route::test_loop());
        crate::view::fill(
            &mut view,
            &h.client,
            h.time_s,
            h.time_s,
            0.0,
            crate::view::Notices::default(),
            false,
        );
        let (lat, lon, _) = view.position.expect("a GPS position in a route ride");
        assert!((lat - origin.0).abs() < 0.05 && (lon - origin.1).abs() < 0.05);
        // The same rider without a geographic origin: no GPS line.
        let c = ClientCore::free_ride(net, "Routed", RiderProfile::default()).with_route_plan(plan);
        assert_eq!(c.geo_origin(), None);
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
            dev_commands: false,
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

    /// K6 (spec §14 Phase 12): the organiser's aids rule reaches the
    /// client, which turns the warning and the proximity cue off while it
    /// races (a race by default) and on again after its finish; the
    /// winner's finish cue drives the victory pose (spec §2.6).
    #[test]
    fn the_organisers_aids_rule_and_the_victory_cue() {
        let mut h = Harness::new(0, true);
        h.client.set_proximity_cue(true);
        h.ride(200.0, 1.0);
        assert!(h.client.aids_allowed());
        let race = ocw_sim::EventSpec::Race {
            laps: None,
            length_m: Some(200.0),
        };
        // Allowed by default, races too (Phase 12 Q8); the organiser may
        // forbid them.
        h.send(&ClientCore::event_start_message(&race, Some(0)));
        h.ride(200.0, 1.0);
        assert!(h.client.aids_allowed(), "races too, by default");
        let forbidden = ocw_sim::event::Rules {
            aids: Some(false),
            ..Default::default()
        };
        h.send(&ClientCore::event_start_message_organiser(
            &race,
            Some(0),
            None,
            &forbidden,
        ));
        h.ride(200.0, 1.0);
        assert!(!h.client.aids_allowed(), "forbidden by the organiser");
        assert_eq!(
            (h.client.proximity(), h.client.proximity_arrow()),
            (None, None)
        );
        let allowed = ocw_sim::event::Rules {
            aids: Some(true),
            ..Default::default()
        };
        h.send(&ClientCore::event_start_message_organiser(
            &race,
            Some(0),
            None,
            &allowed,
        ));
        h.ride(200.0, 1.0);
        assert!(h.client.aids_allowed());
        // Forbidden again; ride to the line: the winner raises the arms.
        h.send(&ClientCore::event_start_message_organiser(
            &race,
            Some(0),
            None,
            &forbidden,
        ));
        let mut finished = false;
        for _ in 0..240 {
            h.ride(350.0, 0.5);
            let me = h.client.rider_id().unwrap();
            let info = h.client.event_info().unwrap();
            if info
                .standings
                .iter()
                .any(|s| s.id == me && s.finish_s.is_some())
            {
                finished = true;
                break;
            }
        }
        assert!(finished, "the race ends");
        h.ride(300.0, 1.0);
        assert!(h.client.aids_allowed(), "after the own finish");
        let cue = h.client.view(h.time_s, 0.0)[0].cue;
        let f = cue.finish.expect("a finish cue");
        assert!(f.winner && (0.0..3.0).contains(&f.since_s), "{f:?}");
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

    /// Regression (v4.38): the game time the photographer runs on exists
    /// only from the first snapshot – the welcome comes before it, and a 0
    /// then started the photographer's ride at the session's start (a
    /// photo at once after joining a server that had run for minutes).
    #[test]
    fn the_shared_game_time_starts_with_the_first_snapshot() {
        let server = GameServer::new(route(), ServerConfig::default());
        let mut h = Harness::with(
            server,
            ClientCore::new(route(), "Ada Rider", RiderProfile::default()),
        );
        assert!(h.client.rider_id().is_some(), "welcomed");
        assert_eq!(h.client.server_time_s(h.time_s), None);
        assert_eq!(h.client.photo_time_s(h.time_s), None);
        h.ride(200.0, 1.0);
        let t = h.client.server_time_s(h.time_s).unwrap();
        assert!((t - h.server.game_time_s()).abs() < 0.2, "{t}");
        let p = h.client.photo_time_s(h.time_s).unwrap();
        assert!((t - p - INTERPOLATION_DELAY_S).abs() < 1e-9);
    }

    /// The ride photographer seen by another rider (spec §2.11 Q2, v4.38):
    /// the first rider's client shares its drone shot through the server;
    /// the second rider's client draws the same pilot and the same drone
    /// flight at the same game time, the flash at the photo's moment.
    #[test]
    fn another_rider_sees_the_photographer_at_the_same_place_and_time() {
        use crate::photographer::{Limits, Photographer, RemoteShot, remote_scene};
        use ocw_world::photo::{Conditions, FnScene};
        use ocw_world::photo_share::SharedShot;
        let mut h = Harness::new(0, true);
        h.ride(250.0, 2.0);
        let me = h.client.rider_id().unwrap();
        let conn = h.server.connect();
        let mut guest = ClientCore::new(route(), "Bea Climber", RiderProfile::default());
        for r in h.server.handle(conn, &guest.hello()) {
            guest.on_server_message(&r, h.time_s);
        }
        let scene = FnScene {
            ground: |_, _| Some(-5.0),
            water: |x: f64, _| x > 400.0,
            canopy: |_, _| 0.0,
            solids: Vec::new(),
            landmarks: Vec::new(),
        };
        let mut p = Photographer::new(Limits::scaled(10.0), 5);
        p.force_drone();
        let mut sent: Option<SharedShot> = None;
        let mut first: Option<SharedShot> = None;
        let mut owner = Vec::new();
        let mut live = Vec::new();
        let mut taken = None;
        let mut gone = false;
        let ticks = 450 * ocw_core::SIM_TICK_HZ;
        for _ in 0..ticks {
            if (h.time_s * 4.0).fract() < 1e-9
                && let Some(sample) = h.client.on_device_event(power(250.0, h.time_s))
            {
                h.send(&sample);
            }
            for msg in h.server.tick() {
                h.receive(&msg);
                guest.on_server_message(&msg, h.time_s);
            }
            h.client.predict_step();
            let t = h.client.server_time_s(h.time_s).unwrap();
            let st = h.client.state();
            let (sc, ev) = p.frame(
                t,
                h.client.track(),
                st.distance,
                st.lateral_offset,
                st.speed,
                &scene,
                &Conditions::default(),
                &|_| false,
            );
            if ev.photo.is_some() && taken.is_none() {
                taken = Some(t);
            }
            if let Some(s) = p.shared()
                && sent.as_ref() != Some(s)
            {
                let bytes = ClientCore::photo_message(s);
                h.send(&bytes);
                if s.shot == 1 {
                    // (the next shot is planned after this one)
                    first = Some(s.clone());
                    gone = s.gone_at.is_some();
                }
                sent = Some(s.clone());
            }
            if let Some(sc) = sc
                && first
                    .as_ref()
                    .is_some_and(|f| f.ended().is_none_or(|e| t < e))
            {
                owner.push((t, sc));
            }
            let gt = guest.photo_time_s(h.time_s).unwrap_or(f64::NEG_INFINITY);
            let near = guest.track().sample(guest.state().distance).position;
            if let Some(r) = guest
                .remote_shots()
                .scenes(gt, &scene, near)
                .iter()
                .find(|r| r.shot == 1)
            {
                live.push((gt, r.scene.clone()));
            }
            h.time_s += ocw_core::SIM_DT;
            h.time_s = (h.time_s * 1000.0).round() / 1000.0;
            if gone && taken.is_some() && guest.photo_time_s(h.time_s).is_some_and(|g| g > t + 1.0)
            {
                break;
            }
        }
        let taken = taken.expect("the photo");
        assert!(
            gone,
            "the photographer left: taken {taken}, now {}, {:?}, visit {:?}",
            h.client.server_time_s(h.time_s).unwrap(),
            p.shared()
                .map(|s| (s.shot, s.taken_at, s.gone_at, s.aborted_at)),
            p.visit()
                .map(|v| matches!(v, crate::photographer::Visit::Drone { .. }))
        );
        let shot: &RemoteShot = guest
            .remote_shots()
            .get(me.0, 1)
            .expect("the guest got the shot");
        assert!(shot.shot.flight.is_some(), "a drone's shot");
        assert_eq!(shot.shot.taken_at, Some(taken));
        assert_eq!(shot.flash_at, Some(taken), "the flash at the photo's time");
        let mine = first.unwrap();
        assert_eq!(
            SharedShot {
                rider: me.0,
                ..mine
            },
            shot.shot
        );
        // The guest draws at every moment what the owner drew then.
        let mut drone_frames = 0;
        for (t, o) in &owner {
            let r = remote_scene(shot, *t, &scene).expect("drawn by the guest");
            assert_eq!(r.person, o.person, "t {t}");
            assert_eq!(r.flash, o.flash, "t {t}");
            assert_eq!(r.seed, o.seed);
            match (r.drone, o.drone) {
                (Some(a), Some(b)) => {
                    assert_eq!(
                        (a.pos, a.look, a.phase, a.rotor),
                        (b.pos, b.look, b.phase, b.rotor)
                    );
                    drone_frames += 1;
                }
                (a, b) => assert_eq!(a.is_some(), b.is_some(), "t {t}"),
            }
        }
        assert!(drone_frames > 30 * 50, "the drone flew: {drone_frames}");
        // And the guest's own frames, with what it knew then, agree.
        assert!(!live.is_empty());
        for (t, l) in &live {
            let r = remote_scene(shot, *t, &scene).unwrap();
            assert_eq!(
                l.drone.map(|d| d.pos),
                r.drone.map(|d| d.pos),
                "the guest at {t}"
            );
        }
        assert!(
            live.iter().any(|(_, l)| l.flash.is_some()),
            "the guest saw the flash"
        );
        assert_eq!(h.server.photos_taken(me).first(), Some(&taken));
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

    /// A group bump with 60 ms from the server to the client: the own
    /// rider leans into a bot riding beside (steady 250 W, its lane 0.6 m
    /// to the right). The prediction's lateral error against the server
    /// (the best of a few ticks' offsets, the two clocks count from
    /// different ticks), the largest sideways and along steps, the bumps.
    fn group_bump_lateral_error(predict: bool) -> (f64, f64, f64, u64) {
        let server = GameServer::new(route(), ocw_server::local::local_config(1, 0.0));
        let client = ClientCore::new(route(), "Ada Rider", RiderProfile::default());
        let mut h = Harness::with(server, client);
        h.client.set_predict_rider_contacts(predict);
        h.ride(250.0, 20.0);
        let me = h.client.rider_id().unwrap();
        let bot = h.client.others_now[0].0;
        h.send(&ClientCore::bot_bike_message(v1::BotBike {
            rider_id: bot.0,
            power_w: Some(250.0),
            lane_m: Some(0.6),
            ..Default::default()
        }));
        h.send(&ClientCore::stage_message(false));
        h.ride(250.0, 1.0);
        assert!(h.client.own_crash(h.time_s).is_none());
        let bumps_before = h.server.session().collision_stats().bumps;
        // From here the snapshots take 3 ticks to arrive.
        const DELAY: usize = 3;
        h.client.set_snapshot_age(DELAY as f64 * ocw_core::SIM_DT);
        let mut queue: std::collections::VecDeque<(usize, Vec<u8>)> = Default::default();
        let (mut server_d, mut client_d) = (Vec::new(), Vec::new());
        let mut last = h.client.state();
        let (mut step, mut along) = (0.0f64, 0.0f64);
        for tick in 0..150 {
            if tick == 5 {
                h.client.set_lean(0.08);
            }
            if tick == 30 {
                h.client.set_lean(0.0);
            }
            if let Some(m) = h.client.steer_message(h.time_s) {
                h.send(&m);
            }
            if (h.time_s * 4.0).fract() < 1e-9
                && let Some(sample) = h.client.on_device_event(power(250.0, h.time_s))
            {
                h.send(&sample);
            }
            for msg in h.server.tick() {
                queue.push_back((tick, msg));
            }
            while queue.front().is_some_and(|(t, _)| t + DELAY <= tick) {
                let (_, msg) = queue.pop_front().unwrap();
                h.receive(&msg);
            }
            h.client.predict_step();
            h.time_s = ((h.time_s + ocw_core::SIM_DT) * 1000.0).round() / 1000.0;
            server_d.push(h.server.session().rider_state(me).unwrap().lateral_offset);
            let st = h.client.state();
            client_d.push(st.lateral_offset);
            step = step.max((st.lateral_offset - last.lateral_offset).abs());
            along = along.max((st.distance - last.distance - last.speed * ocw_core::SIM_DT).abs());
            last = st;
        }
        let bumps = h.server.session().collision_stats().bumps - bumps_before;
        let err = (-2i32..=2)
            .map(|off| {
                (0..server_d.len())
                    .filter_map(|i| {
                        let j = usize::try_from(i as i32 + off).ok()?;
                        Some((client_d.get(j)? - server_d[i]).abs())
                    })
                    .fold(0.0f64, f64::max)
            })
            .fold(f64::INFINITY, f64::min);
        assert!(h.client.own_crash(h.time_s).is_none(), "a bump, no crash");
        (err, step, along, bumps)
    }

    /// Spec §14 Phase 12 (6.1): the client predicts its own bumps with
    /// other riders with the server's code; the server stays the truth
    /// (its answer is blended in: no sideways step over 6 cm, no snap),
    /// and the lateral error against it is smaller than when only the
    /// server's bumps are blended in.
    #[test]
    fn bumps_with_other_riders_are_predicted_and_reconciled() {
        let (err, step, along, bumps) = group_bump_lateral_error(true);
        let (blind_err, blind_step, blind_along, blind_bumps) = group_bump_lateral_error(false);
        eprintln!(
            "group bump, 60 ms: lateral error {:.1} cm predicted ({bumps} bumps, step {:.1} cm, along {:.1} cm), {:.1} cm server only ({blind_bumps} bumps, step {:.1} cm, along {:.1} cm)",
            err * 100.0,
            step * 100.0,
            along * 100.0,
            blind_err * 100.0,
            blind_step * 100.0,
            blind_along * 100.0
        );

        assert!(bumps > 0 && blind_bumps > 0);
        assert!(step < 0.06, "no sideways snap: {step}");
        assert!(along < 0.1, "no snap along: {along}");
        assert!(err < blind_err * 0.7, "{err} vs {blind_err}");
    }

    /// What the trainer was sent on a steady climb: the SIM commands of
    /// one trainer control following the client frame by frame, and the
    /// load each puts on an FTMS trainer turning at `speed` (N: gravity on
    /// the gradient, rolling, the air with the headwind sent).
    struct ClimbRecord {
        sent: Vec<(f64, ocw_devices::SimulationParameters)>,
    }

    impl ClimbRecord {
        fn load_n(p: &ocw_devices::SimulationParameters, speed: f64) -> f64 {
            let g = ocw_core::GRAVITY;
            let air = speed + p.wind_speed_mps;
            p.mass_kg * g * p.grade.atan().sin()
                + p.crr * p.mass_kg * g * p.grade.atan().cos()
                + p.cw_kg_per_m * air * air.abs()
        }

        /// (min, max) of `f` over the commands sent.
        fn span(&self, f: impl Fn(&ocw_devices::SimulationParameters) -> f64) -> (f64, f64) {
            self.sent
                .iter()
                .map(|(_, p)| f(p))
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
                    (lo.min(v), hi.max(v))
                })
        }

        /// How often a Wahoo KICKR (its own control point, every write
        /// answered at once) is written each op code for these commands.
        fn wahoo_writes(&self) -> std::collections::BTreeMap<u8, usize> {
            use ocw_devices::ble::protocol::TrainerProtocol;
            use ocw_devices::gatt::uuids;
            let mut proto = TrainerProtocol::new(
                &[
                    uuids::CYCLING_POWER_MEASUREMENT,
                    uuids::WAHOO_TRAINER_CONTROL,
                ],
                None,
            );
            let mut count = std::collections::BTreeMap::new();
            for (t, p) in &self.sent {
                let now = Duration::from_secs_f64(*t);
                let mut out = proto
                    .on_command(ocw_devices::DeviceCommand::SetSimulation(*p), now)
                    .control_writes;
                while let Some(w) = out.pop() {
                    *count.entry(w[0]).or_insert(0) += 1;
                    out.extend(
                        proto
                            .on_notification(uuids::WAHOO_TRAINER_CONTROL, &[0x01, w[0], 0x01], now)
                            .control_writes,
                    );
                }
            }
            count
        }
    }

    /// A straight road climbing at exactly 10 % for 3 km.
    fn ten_percent() -> Route {
        let points = (0..=120)
            .map(|i| {
                let x = f64::from(i) * 25.0;
                [x, 0.1 * x, 0.0]
            })
            .collect();
        Route::from_points("ten", "Ten percent", points, false, 6.0).unwrap()
    }

    /// Rides `seconds` of the 10 % climb at 300 W in `weather`, the
    /// trainer control following the client every physics step.
    fn ride_the_climb(weather: ocw_weather::WeatherState, seconds: f64) -> ClimbRecord {
        let mut h = Harness::with(
            GameServer::new(
                ten_percent(),
                ServerConfig {
                    weather: Some(weather),
                    weather_seed: Some(7),
                    // As in the app's solo ride (spec §14 Phase 9, Q4).
                    weather_physics: true,
                    allow_local_events: true,
                    ..Default::default()
                },
            ),
            ClientCore::new(ten_percent(), "Ada Rider", RiderProfile::default()),
        );
        h.ride(300.0, 20.0); // up to speed on the climb
        let mut t = crate::trainer::TrainerControl::new(RiderProfile::default());
        let mut sent = Vec::new();
        let steps = (seconds * f64::from(ocw_core::SIM_TICK_HZ)) as u32;
        for _ in 0..steps {
            h.ride(300.0, ocw_core::SIM_DT);
            let grade = h.client.track().sample(h.client.state().distance).grade;
            assert!(
                (grade - 0.1).abs() < 1e-6,
                "the road is 10 % everywhere: {grade}"
            );
            if let Some(ocw_devices::DeviceCommand::SetSimulation(p)) =
                t.follow(h.time_s, &h.client)
            {
                sent.push((h.time_s, p));
            }
        }
        ClimbRecord { sent }
    }

    /// Bug report by the user (2026-11-16): "while the road was constantly
    /// at 10 % inclination, the resistance [of the Wahoo trainer] varied a
    /// bit". The road under the rider and its gradient are exactly 10 %
    /// (asserted every step); this test looks at everything else SIM sends
    /// on that climb, per weather. The cause: the weather's gusts reached
    /// the trainer step by step and the thinning air re-sent the KICKR's
    /// SIM mode (fixed in `ocw_sim::trainer`: the wind smoothed over
    /// `TRAINER_WIND_TAU_S` with a hysteresis, the coefficients re-sent
    /// for the air only from `CW_RESEND_SHARE`).
    #[test]
    fn a_steady_climb_gives_the_trainer_a_steady_load() {
        let calm = ocw_weather::WeatherState {
            wind_mps: 0.0,
            gust_mps: 0.0,
            ..Default::default()
        };
        let speed = 3.5; // m/s, about 300 W at 10 %
        // (weather, the load's variation at most, the Wahoo wind writes at
        // most in 300 s). Before the fix: the default weather 2.2 % with 47
        // wind writes (each gust step a jump), windy 24 % with 114, and the
        // thinning air re-sent the KICKR's whole SIM mode on the climb.
        for (name, weather, max_span, max_wind_writes) in [
            ("still air", calm, 0.001, 0),
            (
                "the default weather",
                ocw_weather::WeatherState::default(),
                0.012,
                4,
            ),
            (
                "windy",
                ocw_weather::presets::preset("windy").unwrap(),
                0.10,
                30,
            ),
        ] {
            let r = ride_the_climb(weather, 300.0);
            let (g0, g1) = r.span(|p| p.grade);
            let (w0, w1) = r.span(|p| p.wind_speed_mps);
            let (c0, c1) = r.span(|p| p.crr);
            let (a0, a1) = r.span(|p| p.cw_kg_per_m);
            let (m0, m1) = r.span(|p| p.mass_kg);
            let (l0, l1) = r.span(|p| ClimbRecord::load_n(p, speed));
            eprintln!(
                "{name}: {} commands in 300 s; grade {g0:.4}..{g1:.4}, wind {w0:+.1}..{w1:+.1} m/s, \
                 crr {c0:.5}..{c1:.5}, cw {a0:.4}..{a1:.4} kg/m, mass {m0:.1}..{m1:.1} kg; \
                 FTMS load at {speed} m/s {l0:.1}..{l1:.1} N ({:.1} %); Wahoo writes {:?}",
                r.sent.len(),
                100.0 * (l1 - l0) / l0,
                r.wahoo_writes(),
            );
            assert!((g0, g1) == (0.05, 0.05), "{name}: 10 % at 50 % difficulty");
            assert!(
                (l1 - l0) / l0 <= max_span,
                "{name}: the load varies {l0:.1}..{l1:.1} N"
            );
            let writes = r.wahoo_writes();
            let wind_writes = writes.get(&ocw_devices::gatt::wahoo::op::SET_WIND_SPEED);
            assert!(
                wind_writes.copied().unwrap_or(0) <= max_wind_writes,
                "{name}: {writes:?}"
            );
            assert_eq!(
                writes.get(&ocw_devices::gatt::wahoo::op::SET_SIM_MODE),
                Some(&1),
                "{name}: the KICKR's SIM mode once, not again for the thinning air"
            );
        }
    }

    /// The SIM parameters a fresh trainer control sends for the client
    /// now (its first update goes out at once).
    fn trainer_sim(h: &Harness) -> ocw_devices::SimulationParameters {
        let mut t = crate::trainer::TrainerControl::new(RiderProfile::default());
        match t.follow(h.time_s, &h.client) {
            Some(ocw_devices::DeviceCommand::SetSimulation(p)) => p,
            other => panic!("expected SIM parameters, got {other:?}"),
        }
    }

    /// What a mock trainer is written for `p` through its own protocol
    /// (the pure `ble::protocol` state machine; every control write
    /// answered with success): an FTMS trainer's *Set Indoor Bike
    /// Simulation Parameters* (Crr in 0.0001, cw in 0.01 kg/m), a Wahoo
    /// KICKR's SIM mode (mass in 0.01 kg, Crr in 0.0001, cw in 0.001 kg/m).
    fn mock_trainer_writes(p: ocw_devices::SimulationParameters) -> ((f64, f64), (f64, f64, f64)) {
        use ocw_devices::ble::protocol::TrainerProtocol;
        use ocw_devices::gatt::{ftms, uuids};
        let run = |mut proto: TrainerProtocol, control: u16, ok: u8, want: u8| -> Vec<u8> {
            let mut out = proto
                .on_command(ocw_devices::DeviceCommand::SetSimulation(p), Duration::ZERO)
                .control_writes;
            for i in 1..8 {
                if let Some(w) = out.iter().find(|w| w[0] == want) {
                    return w.clone();
                }
                let op = out.last().expect("a control write")[0];
                let reply = if ok == 0x80 {
                    vec![0x80, op, 0x01]
                } else {
                    vec![0x01, op, 0x01]
                };
                out = proto
                    .on_notification(control, &reply, Duration::from_millis(100 * i))
                    .control_writes;
            }
            panic!("no SIM write");
        };
        let mut features = ((1u32 << 1) | (1 << 14)).to_le_bytes().to_vec();
        features.extend(((1u32 << 3) | (1 << 13)).to_le_bytes());
        let ftms_proto = TrainerProtocol::new(
            &[
                uuids::FITNESS_MACHINE_FEATURE,
                uuids::INDOOR_BIKE_DATA,
                uuids::FITNESS_MACHINE_CONTROL_POINT,
            ],
            Some(ftms::Features::parse(&features).unwrap()),
        );
        let w = run(ftms_proto, uuids::FITNESS_MACHINE_CONTROL_POINT, 0x80, 0x11);
        let ftms_sent = (f64::from(w[5]) * 1e-4, f64::from(w[6]) * 0.01);
        let wahoo_proto = TrainerProtocol::new(
            &[
                uuids::CYCLING_POWER_MEASUREMENT,
                uuids::WAHOO_TRAINER_CONTROL,
            ],
            None,
        );
        let w = run(wahoo_proto, uuids::WAHOO_TRAINER_CONTROL, 0x01, 0x43);
        let u16_at = |i: usize| f64::from(u16::from_le_bytes([w[i], w[i + 1]]));
        let wahoo_sent = (u16_at(1) * 0.01, u16_at(3) * 1e-4, u16_at(5) * 1e-3);
        (ftms_sent, wahoo_sent)
    }

    /// Spec §14 Phase 4 (4G) / §5.3: the trainer's SIM parameters come
    /// from the build in force, not the profile's values – Crr, the wind
    /// coefficient and (Wahoo) the mass of the build in free ride (Open);
    /// the frame's curated profile in a Profile race; the `road` profile
    /// with the rider's weight in an Equal race. Checked down to the bytes
    /// a mock FTMS and a mock Wahoo trainer are written.
    #[test]
    fn the_trainer_gets_the_build_in_force_per_race_mode() {
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
        h.ride(250.0, 2.0);
        let stock = RiderProfile::default();
        let built = h.client.effective_profile();
        assert!(built.crr < stock.crr && built.cda_m2 < stock.cda_m2);
        assert!(built.bike_mass_kg < stock.bike_mass_kg);
        // Open (free ride): the build.
        let p = trainer_sim(&h);
        assert!(
            (p.crr - built.crr).abs() < 1e-12,
            "{} vs {}",
            p.crr,
            built.crr
        );
        // cw = ½ ρ CdA at the air's density (the standard one, or the
        // weather's while it counts, S6).
        let half_rho = p.cw_kg_per_m / built.cda_m2;
        assert!((0.5..0.7).contains(&half_rho), "{half_rho}");
        assert!((p.mass_kg - (built.rider_mass_kg + built.bike_mass_kg)).abs() < 1e-9);
        let ((f_crr, _), (w_mass, w_crr, w_cw)) = mock_trainer_writes(p);
        assert!((f_crr - built.crr).abs() < 0.6e-4, "FTMS Crr {f_crr}");
        assert!((w_mass - p.mass_kg).abs() < 0.006, "Wahoo mass {w_mass}");
        assert!((w_crr - built.crr).abs() < 0.6e-4 && (w_cw - p.cw_kg_per_m).abs() < 0.6e-3);
        let open = mock_trainer_writes(p);
        // A race in Profile mode (the default): the profile's values.
        h.send(&ClientCore::event_start_message(
            &ocw_sim::EventSpec::Race {
                laps: None,
                length_m: Some(2000.0),
            },
            None,
        ));
        h.ride(250.0, 1.0);
        assert_eq!(h.client.physics_mode(), ocw_sim::PhysicsMode::Profile);
        let p = trainer_sim(&h);
        assert_eq!(p.crr, stock.crr);
        assert!((p.cw_kg_per_m - half_rho * stock.cda_m2).abs() < 0.01 * half_rho * stock.cda_m2);
        assert!((p.mass_kg - (stock.rider_mass_kg + stock.bike_mass_kg)).abs() < 1e-9);
        let profile = mock_trainer_writes(p);
        assert!(profile.0.0 > open.0.0, "FTMS: the stock tyres' Crr again");
        assert!(
            profile.1.0 > open.1.0 && profile.1.2 > open.1.2,
            "Wahoo: mass and cw"
        );
        h.send(&ClientCore::event_cancel_message());
        h.ride(250.0, 0.5);
        // Equal: a gravel bike with fast tyres rides as the road profile.
        let gravel = RiderProfile::curated(ProfileId::Gravel, 75.0);
        let client = ClientCore::new(route(), "Gravel", gravel)
            .with_parts(&["tyres.gravel.fast".to_string()]);
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
        h.ride(250.0, 2.0);
        let p = trainer_sim(&h);
        assert!(p.crr < gravel.crr, "Open: the fast tyres");
        h.send(&ClientCore::event_start_message_with(
            &ocw_sim::EventSpec::Race {
                laps: None,
                length_m: Some(2000.0),
            },
            None,
            Some(ocw_sim::PhysicsMode::Equal),
        ));
        h.ride(250.0, 1.0);
        assert_eq!(h.client.physics_mode(), ocw_sim::PhysicsMode::Equal);
        let road = RiderProfile::curated(ProfileId::Road, 75.0);
        let p = trainer_sim(&h);
        assert_eq!(p.crr, road.crr);
        let half_rho = p.cw_kg_per_m / road.cda_m2;
        assert!((0.5..0.7).contains(&half_rho), "{half_rho}");
        let ((f_crr, _), (_, w_crr, _)) = mock_trainer_writes(p);
        assert!((f_crr - road.crr).abs() < 0.6e-4 && (w_crr - road.crr).abs() < 0.6e-4);
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
            shape: ocw_core::BodyShape::Slim,
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

    /// The rider status signs end to end (spec §2.6, §7.1.7 Q11): rider A
    /// muted → everybody sees the X mouth; B does not hear A → only A
    /// sees headphones on B; C in the menu → everybody sees the phone.
    /// The headphones come from the server per viewer (spec §7.1.7 Q17):
    /// no client ever gets B's list of whom B does not hear.
    #[test]
    fn status_signs_reach_the_others_per_viewer() {
        let mut server = GameServer::new(route(), ServerConfig::default());
        let names = ["Ada Rider", "Bea Climber", "Cleo Sprinter"];
        let mut conns = Vec::new();
        let mut clients: Vec<ClientCore> = names
            .iter()
            .map(|n| ClientCore::new(route(), *n, RiderProfile::default()))
            .collect();
        for c in &mut clients {
            let conn = server.connect();
            for reply in server.handle(conn, &c.hello()) {
                c.on_server_message(&reply, 0.0);
            }
            conns.push(conn);
        }
        let mut t = 0.0;
        let conns_run = conns.clone();
        let mut run = |server: &mut GameServer, clients: &mut Vec<ClientCore>, ticks: usize| {
            for _ in 0..ticks {
                t += ocw_core::SIM_DT;
                let out = server.tick();
                let directed = server.take_directed();
                for (c, conn) in clients.iter_mut().zip(&conns_run) {
                    let mine = directed.iter().filter(|(to, _)| to == conn).map(|(_, m)| m);
                    for b in out.iter().chain(mine) {
                        c.on_server_message(b, t);
                    }
                }
                for c in clients.iter() {
                    assert!(
                        c.statuses.values().all(|st| st.not_hearing.is_empty()),
                        "no list reaches a client"
                    );
                }
            }
            t
        };
        run(&mut server, &mut clients, 40);
        let ids: Vec<RiderId> = clients.iter().map(|c| c.rider_id().unwrap()).collect();
        let (a, b, c) = (ids[0], ids[1], ids[2]);
        // Everybody in the call; A muted; B muted A for themselves; C reads.
        let st = |muted: bool, menu: bool, not_hearing: Vec<u32>| v1::RiderStatus {
            voice_muted: muted,
            in_menu: menu,
            in_voice: true,
            not_hearing,
            ..Default::default()
        };
        let wanted = [
            st(true, false, vec![]),
            st(false, false, vec![a.0]),
            st(false, true, vec![]),
        ];
        for ((cl, conn), w) in clients.iter_mut().zip(&conns).zip(wanted) {
            let msg = cl.set_status(w.clone()).expect("a change is sent at once");
            server.handle(*conn, &msg);
            assert!(cl.set_status(w).is_none(), "no change, nothing sent");
        }
        let now = run(&mut server, &mut clients, 40);
        let seen = |viewer: usize, of: RiderId| -> ocw_render::Signs {
            clients[viewer]
                .view(now, 1.0)
                .into_iter()
                .find(|r| r.id == of)
                .expect("rider in view")
                .signs
        };
        for v in 0..3 {
            assert!(seen(v, a).voice_muted, "viewer {v} sees A's X mouth");
            assert!(!seen(v, b).voice_muted && !seen(v, c).voice_muted);
            assert!(seen(v, c).in_menu, "viewer {v} sees C's phone");
        }
        assert!(seen(0, b).cannot_hear_you, "A sees headphones on B");
        assert!(!seen(2, b).cannot_hear_you, "C does not");
        assert!(!seen(1, b).cannot_hear_you, "never on the own rider");
        assert!(!seen(1, a).cannot_hear_you && !seen(0, c).cannot_hear_you);
        // B mutes the speaker: everybody in the call sees the headphones,
        // unless the viewer is not in the call.
        let msg = clients[1]
            .set_status(v1::RiderStatus {
                hears_nobody: true,
                ..st(false, false, vec![])
            })
            .unwrap();
        server.handle(conns[1], &msg);
        let msg = clients[2].set_status(st(false, true, vec![])).map(|_| ());
        assert!(msg.is_none());
        let msg = clients[2]
            .set_status(v1::RiderStatus {
                in_voice: false,
                ..st(false, true, vec![])
            })
            .unwrap();
        server.handle(conns[2], &msg);
        let now = run(&mut server, &mut clients, 40);
        let seen = |viewer: usize, of: RiderId| -> ocw_render::Signs {
            clients[viewer]
                .view(now, 1.0)
                .into_iter()
                .find(|r| r.id == of)
                .unwrap()
                .signs
        };
        assert!(seen(0, b).cannot_hear_you);
        assert!(!seen(2, b).cannot_hear_you, "C is not in the call");
        // A leaves: the status goes with the rider.
        server.disconnect(conns[0]);
        let now = run(&mut server, &mut clients, 40);
        assert!(clients[1].view(now, 1.0).iter().all(|r| r.id != a));
    }

    /// Spec Phase 8 Q12: the server and the prediction feel the same road
    /// damage – the prediction stays smooth (no snap, no sideways step
    /// over 6 cm) while the bumps cost speed on the server, and the client
    /// presents each bump once: the shake (unless off) and the sound cue.
    #[test]
    fn road_damage_is_ridden_alike_by_server_and_prediction() {
        use ocw_world::graph::RoadClass;
        use ocw_world::road_condition::Smoothness;
        use ocw_world::road_feel::fixture;
        let ride = |sm: Option<Smoothness>| {
            let net = fixture::network(RoadClass::Residential, sm, 6.0, 980.0);
            let server = GameServer::new(
                ocw_server::Course::FreeRide {
                    net: net.clone(),
                    start: ocw_sim::FreeRideStart {
                        edge: 0,
                        offset_m: 20.0,
                        forward: true,
                    },
                },
                ServerConfig {
                    bots: 0,
                    start_m: 0.0,
                    ..Default::default()
                },
            );
            let mut h = Harness::with(
                server,
                ClientCore::free_ride(net, "Ada Rider", RiderProfile::default()),
            );
            h.ride(250.0, 2.0);
            let mut last = h.client.state();
            let mut shaken = false;
            for _ in 0..(40 * 50) {
                h.ride(250.0, ocw_core::SIM_DT);
                let s = h.client.state();
                assert!(
                    (s.lateral_offset - last.lateral_offset).abs() <= 0.06,
                    "{last:?} → {s:?}"
                );
                assert!(
                    h.client.prediction_error().abs() < ocw_sim::prediction::SNAP_THRESHOLD_M,
                    "{}",
                    h.client.prediction_error()
                );
                shaken |= h.client.camera_fx(h.time_s, true).shake != [0.0; 3];
                assert_eq!(h.client.camera_fx(h.time_s, false).shake, [0.0; 3]);
                last = s;
            }
            let (c, s) = h.positions();
            assert!(close(c.position, s.position, 0.5), "{c:?} vs {s:?}");
            let own = h.client.rider_id().unwrap();
            let server = h.server.session().rider_state(own).unwrap();
            (server.distance, h.client.road_bumps().count(), shaken)
        };
        let (clean, none, still) = ride(None);
        assert_eq!((none, still), (0, false));
        let (good, good_bumps, _) = ride(Some(Smoothness::Good));
        assert_eq!(good_bumps, 0);
        assert!((good - clean).abs() < 1e-6, "{good} vs {clean}");
        let (bad, bumps, shaken) = ride(Some(Smoothness::VeryBad));
        assert!(bumps > 3 && shaken, "{bumps}");
        assert!(bad < clean - 2.0, "{bad} vs {clean}");
    }

    /// Spec Phase 8 Q12: each bump over road damage plays its sound once.
    #[test]
    fn a_road_bump_plays_its_cue_once() {
        use ocw_world::graph::RoadClass;
        use ocw_world::road_condition::Smoothness;
        use ocw_world::road_feel::fixture;
        let net = fixture::network(
            RoadClass::Residential,
            Some(Smoothness::Horrible),
            6.0,
            980.0,
        );
        let path = ocw_world::RidePath::start(net, 0, 0.0, true, 0.0, 0);
        let mut bumps = crate::road_bumps::RoadBumps::default();
        let mut s = RiderState {
            lateral_offset: -1.5,
            distance: 10.0,
            ..Default::default()
        };
        let mut seen = Vec::new();
        while s.distance < 900.0 {
            let before = s;
            s.distance += 0.2;
            if let Some(b) = bumps.observe(&path, &before, &s) {
                seen.push(b.id);
            }
        }
        assert!(!seen.is_empty());
        let n = bumps.count();
        // Back over the last metre (a reconciliation): nothing new.
        let back = RiderState {
            distance: s.distance - 1.0,
            ..s
        };
        for _ in 0..5 {
            let mut a = back;
            while a.distance < s.distance {
                let b = a;
                a.distance += 0.2;
                bumps.observe(&path, &b, &a);
            }
        }
        assert_eq!(bumps.count(), n);
        let mut dedup = seen.clone();
        dedup.dedup();
        assert_eq!(dedup, seen, "each feature once");
        let cue = bumps.last().unwrap().cue();
        assert!(matches!(
            cue,
            ocw_audio::cues::Cue::Pothole | ocw_audio::cues::Cue::Crack
        ));
    }
}
