//! Authoritative session logic: all riders of one ride, stepped at a fixed tick (spec §6.1).
//!
//! Used by the remote game server and by the in-process local server alike.
//! A session rides a fixed route, or is a free ride on the road graph where
//! every rider has a path of their own that grows at each junction
//! (spec §2.2.1, §5.2).

use std::collections::BTreeMap;
use std::sync::Arc;

use ocw_core::{ProfileId, RiderId, RiderLook, RiderProfile, SIM_DT};
use ocw_world::network::splitmix;
use ocw_world::{RidePath, RoadNetwork, Route, RoutePlan, Segment, Track};

use crate::drafting;
use crate::event::{self, Event, EventChange, EventKind, EventSpec, Progress, Standing};
use crate::freeride::{self, TurnChoice, UTURN_INTERVAL_S};
use crate::physics::{self, Environment, RiderState, Steering};
use crate::ride::{self, Controls};

mod god_mode;
pub use god_mode::{GodLeave, GodRider};

/// Latest sensor input of a rider.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RiderInput {
    pub power_w: f64,
    pub cadence_rpm: Option<f64>,
    pub heart_rate_bpm: Option<f64>,
    /// Brake key held (stop the rider).
    pub brake: bool,
}

/// How a rider's power is produced.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Control {
    /// Power comes from a connected client.
    Client,
    /// Server-side test rider with a gently varying power around `base_power_w`.
    Bot { base_power_w: f64 },
}

#[derive(Debug, Clone)]
struct Rider {
    name: String,
    profile: RiderProfile,
    /// Cosmetic only (spec §2.5): passed through to the snapshots.
    look: RiderLook,
    state: RiderState,
    input: RiderInput,
    last_input_seq: u32,
    control: Control,
    /// How the rider steers (autodrive in their lane until a client says
    /// otherwise; bots always) and whether they are in wild mode.
    steering: Steering,
    wild: bool,
    /// Scraped along the road edge in the last tick.
    scraping: bool,
    last_steer_seq: u32,
    /// Free ride: the rider's own path, the exit chosen for the next
    /// junction, and when the last manual U-turn was.
    path: Option<RidePath>,
    turn: Option<TurnChoice>,
    last_turn_seq: u32,
    last_u_turn_s: Option<f64>,
    /// First path step changed since the last [`Session::take_path_changes`],
    /// and whether the path was replaced as a whole.
    path_changed: Option<u64>,
    path_reset: bool,
    /// Place in the start formation ([`formation_slot`]).
    slot: usize,
    /// God mode (spec §14 Phase 13): out of the physics, the road
    /// position frozen, flying as a unicorn.
    god: Option<GodRider>,
}

/// State of one rider as broadcast to clients.
#[derive(Debug, Clone, PartialEq)]
pub struct RiderSnapshot {
    pub id: RiderId,
    pub name: String,
    /// The rider's physics profile (it also picks the bike model).
    pub bike: ProfileId,
    /// The rider's cosmetic look.
    pub look: RiderLook,
    pub state: RiderState,
    pub power_w: f64,
    /// Sequence number of the last input from this rider's client the server applied.
    pub last_input_seq: u32,
    /// In god mode: the unicorn's pose (spec §14 Phase 13E).
    pub god: Option<crate::god::GodPose>,
}

/// Where free-ride riders start: `offset_m` along graph edge `edge` (from
/// its `from` vertex), riding it forwards or backwards.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FreeRideStart {
    pub edge: usize,
    pub offset_m: f64,
    pub forward: bool,
}

/// A change of a rider's free-ride path, for the clients (spec §6.5: the
/// server broadcasts the chosen edges). `reset`: a new path (start,
/// teleport) beginning with step `first_index` at path distance `start_m`;
/// otherwise the steps from `first_index` on replace the known ones.
#[derive(Debug, Clone, PartialEq)]
pub struct PathChange {
    pub rider: RiderId,
    pub reset: bool,
    pub first_index: u64,
    pub start_m: f64,
    pub segments: Vec<Segment>,
}

/// A point on the road graph: (edge, ridden forwards, metres from its start).
type EdgePoint = (usize, bool, f64);

/// Slowest speed a bot climbs at (m/s, 10 km/h): like real riders, easy
/// bots push harder uphill instead of crawling.
pub const BOT_MIN_CLIMB_SPEED: f64 = 2.8;

/// Power a bot needs to hold [`BOT_MIN_CLIMB_SPEED`] on `grade` (0 when
/// flat or downhill).
pub fn bot_climb_power_w(profile: &RiderProfile, grade: f64) -> f64 {
    if grade <= 0.0 {
        return 0.0;
    }
    let env = Environment::with_grade(grade);
    let v = BOT_MIN_CLIMB_SPEED;
    physics::resistance(profile, &env, v).total() * v / profile.drivetrain_efficiency
}

/// Gap between two rows of the start formation along the road (m).
pub const FORMATION_ROW_GAP_M: f64 = 3.0;
/// Sideways gap between riders of one formation row (m).
pub const FORMATION_SPACING_M: f64 = 1.2;
/// Most riders in one formation row.
pub const FORMATION_ROW_MAX: usize = 3;

/// Lateral offsets of one start-formation row on a road `width_m` wide:
/// up to [`FORMATION_ROW_MAX`] riders [`FORMATION_SPACING_M`] apart, centre
/// first, always 0.3 m inside the lateral limit (so nobody starts
/// scraping the road edge); narrow roads get fewer riders per row.
pub fn formation_row(width_m: f64) -> Vec<f64> {
    let limit = (physics::lateral_limit(width_m) - 0.3).max(0.0);
    let n =
        ((2.0 * limit / FORMATION_SPACING_M + 1e-9).floor() as usize + 1).min(FORMATION_ROW_MAX);
    let s = FORMATION_SPACING_M;
    match n {
        1 => vec![0.0],
        2 => vec![-s / 2.0, s / 2.0],
        _ => vec![0.0, -s, s],
    }
}

/// Place `slot` of the start formation on a road `width_m` wide: metres
/// behind the start point and lateral offset. Rows of
/// [`formation_row`] riders, [`FORMATION_ROW_GAP_M`] apart; slot 0 is the
/// front row's centre (kept for the first client rider).
pub fn formation_slot(slot: usize, width_m: f64) -> (f64, f64) {
    let row = formation_row(width_m);
    let back = (slot / row.len()) as f64 * FORMATION_ROW_GAP_M;
    (back, row[slot % row.len()])
}

/// One ride: a route (or the road graph) plus the riders on it.
#[derive(Debug, Clone)]
pub struct Session {
    /// The route; in free ride the start of the first path (for its name).
    route: Route,
    /// Free ride: the road network and where riders start.
    free: Option<(Arc<RoadNetwork>, FreeRideStart)>,
    /// A route on the road graph everyone follows (spec §2.7); riders may
    /// leave it and rejoin.
    plan: Option<Arc<RoutePlan>>,
    riders: BTreeMap<RiderId, Rider>,
    tick: u64,
    next_id: u32,
    /// Where riders start on the route, in metres.
    start_m: f64,
    /// How far the start formation's front is moved forward so its last
    /// row still starts on the road (routes that end, short start edges).
    front_shift_m: f64,
    /// The current event (spec §2.3), started on demand or scheduled.
    event: Option<Event>,
    next_event_id: u32,
    /// Event changes since the last [`Session::take_event_changes`].
    event_changes: Vec<EventChange>,
    /// The area god mode may fly over (the road graph's or the route's
    /// box, grown), for the server's plausibility check.
    god_area: Option<crate::god::Bounds>,
}

impl Session {
    pub fn new(route: Route) -> Self {
        Self {
            free: None,
            plan: None,
            riders: BTreeMap::new(),
            tick: 0,
            next_id: 1,
            start_m: 0.0,
            front_shift_m: 0.0,
            event: None,
            next_event_id: 1,
            event_changes: Vec::new(),
            god_area: god_mode::route_area(&route),
            route,
        }
    }

    /// A free ride on `net`: everyone starts at `start` and chooses their
    /// own way at junctions.
    pub fn free_ride(net: Arc<RoadNetwork>, start: FreeRideStart) -> Self {
        let first = RidePath::start(
            net.clone(),
            start.edge,
            start.offset_m,
            start.forward,
            0.0,
            0,
        );
        Self {
            route: first.route().clone(),
            god_area: god_mode::network_area(&net),
            free: Some((net, start)),
            ..Self::new(first.route().clone())
        }
    }

    /// A ride along `plan` on `net` (spec §2.7): everyone starts at the
    /// route's start and follows it through the junctions (bots always);
    /// riders choosing another exit leave it and may rejoin
    /// ([`Session::rejoin_route`]).
    pub fn route_ride(net: Arc<RoadNetwork>, plan: Arc<RoutePlan>) -> Self {
        let (edge, offset_m, forward) = plan.start();
        let start = FreeRideStart {
            edge,
            offset_m,
            forward,
        };
        let first = RidePath::on_route(net.clone(), plan.clone(), 0.0, 0);
        let route = Route::from_points(
            plan.name.clone(),
            plan.name.clone(),
            first.route().points().to_vec(),
            false,
            net.edge(edge).width_m,
        )
        .unwrap_or_else(|_| first.route().clone());
        Self {
            plan: Some(plan),
            ..Self::free_ride(net, start).with_route(route)
        }
    }

    fn with_route(mut self, route: Route) -> Self {
        self.route = route;
        self
    }

    /// The route everyone follows, if this is a route ride.
    pub fn plan(&self) -> Option<&Arc<RoutePlan>> {
        self.plan.as_ref()
    }

    /// The same session with riders starting `start_m` metres along the route.
    pub fn with_start(mut self, start_m: f64) -> Self {
        self.start_m = start_m.max(0.0);
        self
    }

    pub fn route(&self) -> &Route {
        &self.route
    }

    pub fn is_free_ride(&self) -> bool {
        self.free.is_some()
    }

    /// The road network of a free ride.
    pub fn network(&self) -> Option<&Arc<RoadNetwork>> {
        self.free.as_ref().map(|f| &f.0)
    }

    /// A rider's free-ride path.
    pub fn path(&self, id: RiderId) -> Option<&RidePath> {
        self.riders.get(&id)?.path.as_ref()
    }

    /// What rider `id` rides along: their path, or the route.
    pub fn track(&self, id: RiderId) -> &dyn Track {
        match self.path(id) {
            Some(p) => p,
            None => &self.route,
        }
    }

    /// Number of physics ticks since the session started.
    pub fn tick(&self) -> u64 {
        self.tick
    }

    /// Session time in seconds.
    pub fn time_s(&self) -> f64 {
        self.tick as f64 * SIM_DT
    }

    pub fn rider_count(&self) -> usize {
        self.riders.len()
    }

    /// Adds a rider controlled by a client.
    pub fn add_rider(&mut self, name: impl Into<String>, profile: RiderProfile) -> RiderId {
        self.insert(name.into(), profile, Control::Client)
    }

    /// Adds a server-side test rider ("bot") riding around `base_power_w`.
    pub fn add_bot(&mut self, name: impl Into<String>, base_power_w: f64) -> RiderId {
        self.add_bot_with(
            name,
            base_power_w,
            RiderProfile::default(),
            RiderLook::default(),
        )
    }

    /// Adds a bot riding a given physics profile, with a look.
    pub fn add_bot_with(
        &mut self,
        name: impl Into<String>,
        base_power_w: f64,
        profile: RiderProfile,
        look: RiderLook,
    ) -> RiderId {
        let id = self.insert(name.into(), profile, Control::Bot { base_power_w });
        self.set_look(id, look);
        id
    }

    /// Sets a rider's cosmetic look (spec §2.5); false for unknown riders.
    pub fn set_look(&mut self, id: RiderId, look: RiderLook) -> bool {
        match self.riders.get_mut(&id) {
            Some(r) => {
                r.look = look;
                true
            }
            None => false,
        }
    }

    fn insert(&mut self, name: String, profile: RiderProfile, control: Control) -> RiderId {
        let id = RiderId(self.next_id);
        self.next_id += 1;
        // Start formation (spec §6.1): the first free slot; the front row's
        // centre is kept for a client rider, who joins after the bots.
        let first = match control {
            Control::Client => 0,
            Control::Bot { .. } => 1,
        };
        let slot = (first..)
            .find(|s| !self.riders.values().any(|r| r.slot == *s))
            .expect("a free slot");
        let width = self.start_width();
        let (back, lateral_offset) = formation_slot(slot, width);
        // Before the ride starts, move the whole formation forward if its
        // last row would start before the road begins.
        if self.tick == 0 {
            let needed = (back - self.room_behind()).min(self.room_ahead());
            if needed > self.front_shift_m {
                self.front_shift_m = needed;
                let spots: Vec<(RiderId, usize)> =
                    self.riders.iter().map(|(id, r)| (*id, r.slot)).collect();
                for (other, s) in spots {
                    let (path, distance) = self.start_spot(formation_slot(s, width).0);
                    let r = self.riders.get_mut(&other).expect("listed rider");
                    r.state.distance = distance;
                    if path.is_some() {
                        r.path = path;
                        r.path_reset = true;
                        r.path_changed = None;
                    }
                }
            }
        }
        let (path, distance) = self.start_spot(back);
        self.riders.insert(
            id,
            Rider {
                name,
                profile,
                look: RiderLook::default(),
                state: RiderState {
                    distance,
                    lateral_offset,
                    ..Default::default()
                },
                input: RiderInput::default(),
                last_input_seq: 0,
                control,
                steering: Steering::Auto(lateral_offset),
                wild: false,
                scraping: false,
                last_steer_seq: 0,
                path_reset: path.is_some(),
                path,
                turn: None,
                last_turn_seq: 0,
                last_u_turn_s: None,
                path_changed: None,
                slot,
                god: None,
            },
        );
        id
    }

    /// Road width at the start point.
    fn start_width(&self) -> f64 {
        match &self.free {
            Some((net, s)) => net.edge(s.edge).width_m,
            None => self.route.road_width_m(),
        }
    }

    /// Road behind the start point the formation can use (m): on a route
    /// back to its beginning (loops too, so distances stay positive), in
    /// free ride back to the start edge's end; none in a route ride on the
    /// road graph (the route begins there).
    fn room_behind(&self) -> f64 {
        match &self.free {
            Some(_) if self.plan.is_some() => 0.0,
            Some((net, s)) => {
                let len = net.geometry(s.edge).length();
                if s.forward {
                    s.offset_m.max(0.0)
                } else {
                    (len - s.offset_m).max(0.0)
                }
            }
            None => self.start_m,
        }
    }

    /// Road ahead of the start point the formation may be moved onto (m).
    fn room_ahead(&self) -> f64 {
        match &self.free {
            Some(_) if self.plan.is_some() => self.plan.as_ref().map_or(0.0, |p| p.length_m()),
            Some((net, s)) => {
                let len = net.geometry(s.edge).length();
                if s.forward {
                    (len - 1.0 - s.offset_m).max(0.0)
                } else {
                    (s.offset_m - 1.0).max(0.0)
                }
            }
            None if self.route.is_loop() => f64::INFINITY,
            None => (self.route.length() - self.start_m).max(0.0),
        }
    }

    /// Where a rider `back` metres behind the formation's front starts:
    /// their free-ride path (starting there, on the start edge; in a route
    /// ride at the route's start) and their distance. Never before the road
    /// begins.
    fn start_spot(&self, back: f64) -> (Option<RidePath>, f64) {
        let behind = (back - self.front_shift_m).min(self.room_behind());
        match &self.free {
            Some((net, _)) if self.plan.is_some() => {
                let plan = self.plan.as_ref().expect("checked");
                let d = (-behind).max(0.0);
                let mut p = RidePath::on_route(net.clone(), plan.clone(), 0.0, 0);
                p.extend(d, &mut |j| j.default);
                (Some(p), d)
            }
            Some((net, s)) => {
                let offset = if s.forward {
                    s.offset_m - behind
                } else {
                    s.offset_m + behind
                };
                let mut p = RidePath::start(net.clone(), s.edge, offset, s.forward, 0.0, 0);
                p.extend(0.0, &mut |j| j.default);
                (Some(p), 0.0)
            }
            None => (None, (self.start_m - behind).max(0.0)),
        }
    }

    pub fn remove_rider(&mut self, id: RiderId) -> bool {
        self.riders.remove(&id).is_some()
    }

    /// Applies a client input. Inputs with a sequence number not newer than the
    /// last applied one are ignored (reordered or replayed packets, spec §6.4).
    pub fn apply_input(&mut self, id: RiderId, seq: u32, input: RiderInput) -> bool {
        match self.riders.get_mut(&id) {
            Some(r) if r.control == Control::Client && seq > r.last_input_seq => {
                r.last_input_seq = seq;
                r.input = RiderInput {
                    power_w: input.power_w.max(0.0),
                    ..input
                };
                true
            }
            _ => false,
        }
    }

    /// Steering intent of a client rider: lateral offset to move to, clamped
    /// to the road (spec §6.1: clients send intents, the server moves the
    /// rider). Stale sequence numbers and bots are ignored.
    pub fn set_lateral_target(&mut self, id: RiderId, seq: u32, target_m: f64) -> bool {
        let Some(r) = self.riders.get_mut(&id) else {
            return false;
        };
        let width = match &r.path {
            Some(p) => p.road_width_at(r.state.distance),
            None => self.route.road_width_m(),
        };
        let limit = physics::lateral_limit(width);
        if r.control == Control::Client && seq > r.last_steer_seq {
            r.last_steer_seq = seq;
            r.steering = Steering::Auto(target_m.clamp(-limit, limit));
            true
        } else {
            false
        }
    }

    /// Lean steering intent of a client rider (the arrow keys): rides a
    /// curve with this lean (rad, positive = right) until it is 0 again.
    /// Same sequence numbers as [`Session::set_lateral_target`].
    pub fn set_lean(&mut self, id: RiderId, seq: u32, lean_rad: f64) -> bool {
        match self.riders.get_mut(&id) {
            Some(r) if r.control == Control::Client && seq > r.last_steer_seq => {
                r.last_steer_seq = seq;
                r.steering = Steering::Lean(lean_rad);
                true
            }
            _ => false,
        }
    }

    /// Wild mode of a client rider: not bound to the road.
    pub fn set_wild(&mut self, id: RiderId, wild: bool) -> bool {
        match self.riders.get_mut(&id) {
            Some(r) if r.control == Control::Client => {
                r.wild = wild;
                true
            }
            _ => false,
        }
    }

    /// Exit choice of a client rider for a junction (spec §6.5); `None` =
    /// no choice (the default). Applied at the lock point; stale sequence
    /// numbers are ignored.
    pub fn set_turn(&mut self, id: RiderId, seq: u32, choice: Option<TurnChoice>) -> bool {
        match self.riders.get_mut(&id) {
            Some(r)
                if r.control == Control::Client && r.path.is_some() && seq > r.last_turn_seq =>
            {
                r.last_turn_seq = seq;
                r.turn = choice;
                true
            }
            _ => false,
        }
    }

    /// Manual U-turn of a free-ride rider (spec §2.2.2), at most every
    /// [`UTURN_INTERVAL_S`]. `from` is where the client turned (distance,
    /// speed); used when it is close to the server's state, so both build
    /// the same loop.
    pub fn u_turn(&mut self, id: RiderId, from: Option<(f64, f64)>) -> bool {
        let now = self.time_s();
        let Some(r) = self.riders.get_mut(&id) else {
            return false;
        };
        if r.last_u_turn_s.is_some_and(|t| now - t < UTURN_INTERVAL_S) {
            return false;
        }
        let (d, v) = match from {
            Some((d, v)) if (d - r.state.distance).abs() < 10.0 => (d, v),
            _ => (r.state.distance, r.state.speed),
        };
        let Some(path) = &mut r.path else {
            return false;
        };
        match path.u_turn(d, v) {
            Some(i) => {
                r.path_changed = Some(r.path_changed.map_or(i, |c| c.min(i)));
                r.last_u_turn_s = Some(now);
                true
            }
            None => false,
        }
    }

    /// Back to the route after leaving it (spec §2.2.1): the shortest way
    /// to the route is added to the rider's path. `from` is where the
    /// client asked (distance, speed), used when close to the server's
    /// state so both find the same way.
    pub fn rejoin_route(&mut self, id: RiderId, from: Option<(f64, f64)>) -> Result<(), String> {
        let r = self.riders.get_mut(&id).ok_or("no such rider")?;
        let (d, v) = match from {
            Some((d, v)) if (d - r.state.distance).abs() < 10.0 => (d, v),
            _ => (r.state.distance, r.state.speed),
        };
        let path = r.path.as_mut().ok_or("not riding a route")?;
        let i = path.rejoin_route(d, v)?;
        r.path_changed = Some(r.path_changed.map_or(i, |c| c.min(i)));
        r.turn = None;
        Ok(())
    }

    /// Moves a rider to `distance` metres (dev command, local server
    /// only); the distance moved to, `None` for an unknown rider. In free
    /// ride only forwards along the path (junctions take their default
    /// exits on the way). On a route that ends, a distance past its end is
    /// clamped to the end ([`Track::finish`], where riders stop); a loop
    /// takes any distance (it is that many laps on).
    pub fn teleport(&mut self, id: RiderId, distance: f64) -> Option<f64> {
        let finish = self.route.finish();
        let r = self.riders.get_mut(&id)?;
        match &mut r.path {
            Some(path) => {
                if let Some(i) = path.extend(distance, &mut |j| j.default) {
                    r.path_changed = Some(r.path_changed.map_or(i, |c| c.min(i)));
                }
                r.state.distance = distance.max(path.begin_m());
            }
            None => {
                let d = distance.max(0.0);
                r.state.distance = finish.map_or(d, |end| d.min(end));
                if finish.is_some_and(|end| r.state.distance >= end) {
                    r.state.speed = 0.0;
                }
            }
        }
        Some(r.state.distance)
    }

    /// Moves a free-ride rider to the road nearest the point (x, z): a new
    /// path from there, in the direction `forward` along that edge (dev
    /// command).
    pub fn teleport_to(&mut self, id: RiderId, x: f64, z: f64) -> bool {
        let Some((net, _)) = &self.free else {
            return false;
        };
        let Some((edge, offset)) = net.nearest([x, z]) else {
            return false;
        };
        let net = net.clone();
        let Some(r) = self.riders.get_mut(&id) else {
            return false;
        };
        let forward = net.edge(edge).oneway.allows(true);
        let next = r.path.as_ref().map_or(0, RidePath::next_index);
        let d = r.state.distance;
        let mut path =
            RidePath::start(net, edge, offset, forward, d, next).with_plan(self.plan.clone());
        path.extend(d, &mut |j| j.default);
        r.path = Some(path);
        r.path_reset = true;
        r.path_changed = None;
        r.state.lateral_offset = 0.0;
        r.state.heading = 0.0;
        true
    }

    /// Path changes since the last call (for broadcasting).
    pub fn take_path_changes(&mut self) -> Vec<PathChange> {
        let mut out = Vec::new();
        for (id, r) in &mut self.riders {
            let Some(path) = &r.path else { continue };
            let changed = r.path_changed.take();
            let reset = std::mem::take(&mut r.path_reset)
                || changed.is_some_and(|i| i <= path.first_index());
            let first = if reset {
                path.first_index()
            } else if let Some(i) = changed {
                i
            } else {
                continue;
            };
            out.push(PathChange {
                rider: *id,
                reset,
                first_index: first,
                start_m: path.step_start_m(first).unwrap_or(0.0),
                segments: path.segments_from(first),
            });
        }
        out
    }

    /// Every rider's whole path (for a client that just joined).
    pub fn all_paths(&self) -> Vec<PathChange> {
        self.riders
            .iter()
            .filter_map(|(id, r)| {
                let p = r.path.as_ref()?;
                Some(PathChange {
                    rider: *id,
                    reset: true,
                    first_index: p.first_index(),
                    start_m: p.begin_m(),
                    segments: p.segments_from(p.first_index()),
                })
            })
            .collect()
    }

    /// Advances the session by one physics tick.
    pub fn step(&mut self) {
        self.tick += 1;
        let t = self.time_s();
        let held = self.event.as_ref().is_some_and(Event::holds);
        let drafting = self.event.as_ref().is_none_or(|e| e.kind.drafting());
        let group = match self.event.as_ref().map(|e| (e.kind, e.phase)) {
            Some((EventKind::GroupRide { target_w }, event::Phase::Running)) => {
                let (bots, clients): (Vec<_>, Vec<_>) = self
                    .riders
                    .values()
                    .partition(|r| matches!(r.control, Control::Bot { .. }));
                let d = |v: Vec<&Rider>| v.iter().map(|r| r.state.distance).collect::<Vec<_>>();
                event::group_anchor(&d(bots), &d(clients)).map(|a| (target_w, a))
            }
            _ => None,
        };
        let before: Vec<(RiderId, f64)> = self
            .riders
            .iter()
            .map(|(id, r)| (*id, r.state.distance))
            .collect();
        // Drafting uses everyone's position at the start of the tick (in
        // free ride also where on the road graph they are).
        // Riders in god mode fly: they draft nobody and nobody drafts them.
        let positions: Vec<(RiderId, f64, f64, Option<EdgePoint>)> = self
            .riders
            .iter()
            .filter(|(_, r)| r.god.is_none())
            .map(|(id, r)| {
                let at = r.path.as_ref().and_then(|p| p.locate(r.state.distance));
                (*id, r.state.distance, r.state.lateral_offset, at)
            })
            .collect();
        for (id, rider) in &mut self.riders {
            let d = rider.state.distance;
            if rider.god.is_some() {
                // God mode: out of the physics, the road position frozen.
                rider.state.speed = 0.0;
                continue;
            }
            if held {
                // Countdown: the field waits at the start line.
                rider.state.speed = 0.0;
                continue;
            }
            // Free ride: extend the path, lock the next junction's exit.
            if let Some(path) = &mut rider.path {
                let (changed, used) = match rider.control {
                    Control::Bot { .. } => {
                        let seed = (u64::from(id.0) << 40) ^ (d / 50.0) as u64;
                        let changed = path.extend_at_speed(d, rider.state.speed, &mut |j| {
                            freeride::bot_exit(j, splitmix(seed ^ u64::from(j.vertex.0)))
                        });
                        (changed, false)
                    }
                    Control::Client => {
                        freeride::advance(path, d, rider.state.speed, rider.turn.as_ref())
                    }
                };
                if used {
                    rider.turn = None;
                }
                if let Some(i) = changed {
                    rider.path_changed = Some(rider.path_changed.map_or(i, |c| c.min(i)));
                }
                path.trim_behind(d);
            }
            let others = positions.iter().filter(|(other, ..)| other != id);
            let ahead: Vec<(f64, f64)> = match &rider.path {
                Some(p) => others
                    .filter_map(|&(_, _, l, at)| {
                        let (e, f, o) = at?;
                        Some((p.distance_of(e, f, o, d)?, l))
                    })
                    .collect(),
                None => others.map(|&(_, od, l, _)| (od, l)).collect(),
            };
            let drag = if drafting {
                drafting::drag_factor((d, rider.state.lateral_offset), ahead)
            } else {
                1.0
            };
            let track: &dyn Track = match &rider.path {
                Some(p) => p,
                None => &self.route,
            };
            let grade = track.sample(d).grade;
            if let Control::Bot { base_power_w } = rider.control {
                // Deterministic variation so bots don't ride in perfect lockstep.
                let phase = id.0 as f64 * 1.7;
                let cruising = match group {
                    // Group ride: the target pace, regrouping (spec §2.3).
                    Some((target_w, anchor)) => {
                        let personal = event::GROUP_BAND * (t * 0.05 + phase).sin();
                        event::group_power_w(target_w, personal, d - anchor)
                    }
                    None => base_power_w * (1.0 + 0.1 * (t * 0.2 + phase).sin()),
                };
                rider.input.power_w = cruising.max(bot_climb_power_w(&rider.profile, grade));
            }
            let controls = Controls {
                steering: rider.steering,
                wild: rider.wild,
                braking: rider.input.brake,
            };
            ride::tick(
                &mut rider.state,
                &rider.profile,
                rider.input.power_w,
                controls,
                &mut rider.scraping,
                track,
                drag,
                SIM_DT,
            );
        }
        if let Some(ev) = &mut self.event {
            let progress: Vec<Progress> = before
                .iter()
                .filter_map(|&(id, before_m)| {
                    let r = self.riders.get(&id)?;
                    Some(Progress {
                        id,
                        before_m,
                        after_m: r.state.distance,
                        speed: r.state.speed,
                    })
                })
                .collect();
            let changes = ev.update(self.tick, &progress);
            self.event_changes.extend(changes);
        }
    }

    /// Current state of all riders.
    pub fn snapshot(&self) -> Vec<RiderSnapshot> {
        self.riders
            .iter()
            .map(|(id, r)| RiderSnapshot {
                id: *id,
                name: r.name.clone(),
                bike: r.profile.bike,
                look: r.look,
                state: r.state,
                power_w: r.input.power_w,
                last_input_seq: r.last_input_seq,
                god: r.god.map(|g| g.pose),
            })
            .collect()
    }

    pub fn rider_state(&self, id: RiderId) -> Option<RiderState> {
        self.riders.get(&id).map(|r| r.state)
    }

    /// The server-side test riders.
    pub fn bots(&self) -> Vec<RiderId> {
        self.riders
            .iter()
            .filter(|(_, r)| matches!(r.control, Control::Bot { .. }))
            .map(|(id, _)| *id)
            .collect()
    }

    /// The current event, if any (also a finished one, until the next).
    pub fn event(&self) -> Option<&Event> {
        self.event.as_ref()
    }

    /// Event changes since the last call (for broadcasting).
    pub fn take_event_changes(&mut self) -> Vec<EventChange> {
        std::mem::take(&mut self.event_changes)
    }

    /// The course's length and whether it is a loop: the route, or the
    /// route on the road graph; `None` in free ride (no fixed course).
    pub fn course_length(&self) -> Option<(f64, bool)> {
        match (&self.free, &self.plan) {
            (None, _) => Some((self.route.length(), self.route.is_loop())),
            (Some(_), Some(plan)) => Some((plan.length_m(), plan.is_loop)),
            (Some(_), None) => None,
        }
    }

    /// Starts an event now (spec §2.3; replaces a running one): every rider
    /// in the session is in it, lined up in the start formation behind the
    /// start line - at the start / finish of a loop (one lap in), at the
    /// formation's length on a route that ends - and held there through
    /// the countdown. Needs a fixed course (a route; not free ride).
    pub fn start_event(&mut self, spec: EventSpec) -> Result<&Event, String> {
        let (course_m, is_loop) = self
            .course_length()
            .ok_or("events need a route (free ride has no fixed course)")?;
        if self.riders.is_empty() {
            return Err("no riders".into());
        }
        // Formation slots: client riders first (the front row's centre),
        // then the bots, each in joining order.
        let mut order: Vec<RiderId> = self.riders.keys().copied().collect();
        order.sort_by_key(|id| (matches!(self.riders[id].control, Control::Bot { .. }), *id));
        let width = self.start_width();
        let last_back = formation_slot(order.len() - 1, width).0;
        let start_line = if is_loop {
            course_m
        } else {
            last_back.min(course_m)
        };
        if is_loop && course_m < last_back + 1.0 {
            return Err("the loop is too short for the start formation".into());
        }
        let kind = match spec {
            EventSpec::GroupRide { target_w } => EventKind::GroupRide {
                target_w: target_w
                    .unwrap_or(event::DEFAULT_GROUP_W)
                    .clamp(50.0, 600.0),
            },
            EventSpec::Race { laps, length_m } => {
                let (length_m, laps) =
                    event::resolve_length(laps, length_m, course_m, is_loop, start_line)?;
                EventKind::Race { length_m, laps }
            }
            EventSpec::TimeTrial { laps, length_m } => {
                let (length_m, laps) =
                    event::resolve_length(laps, length_m, course_m, is_loop, start_line)?;
                EventKind::TimeTrial { length_m, laps }
            }
        };
        for (slot, id) in order.iter().enumerate() {
            let (back, lateral) = formation_slot(slot, width);
            self.place(*id, slot, (start_line - back).max(0.0), lateral);
        }
        let entrants = order
            .iter()
            .map(|id| (*id, self.riders[id].name.clone()))
            .collect();
        let id = self.next_event_id;
        self.next_event_id += 1;
        let mut ev = Event::new(id, kind, self.tick, start_line, entrants);
        let changes = ev.update(self.tick, &[]);
        self.event_changes.extend(changes);
        Ok(self.event.insert(ev))
    }

    /// Ends the current event (riders ride on where they are).
    pub fn cancel_event(&mut self) -> Option<Event> {
        self.event.take()
    }

    /// Moves a rider to `distance` in formation place `slot`, standing.
    fn place(&mut self, id: RiderId, slot: usize, distance: f64, lateral: f64) {
        let net_plan = match (&self.free, &self.plan) {
            (Some((net, _)), Some(plan)) => Some((net.clone(), plan.clone())),
            _ => None,
        };
        let Some(r) = self.riders.get_mut(&id) else {
            return;
        };
        if let Some((net, plan)) = net_plan {
            let next = r.path.as_ref().map_or(0, RidePath::next_index);
            let mut p = RidePath::on_route(net, plan, 0.0, next);
            p.extend(distance, &mut |j| j.default);
            p.trim_behind(distance);
            r.path = Some(p);
            r.path_reset = true;
            r.path_changed = None;
            r.turn = None;
        }
        r.state = RiderState {
            distance,
            lateral_offset: lateral,
            ..Default::default()
        };
        r.steering = Steering::Auto(lateral);
        r.wild = false;
        r.scraping = false;
        r.slot = slot;
        // An event's start formation ends god mode.
        r.god = None;
    }

    /// The event's ranking now (empty without an event).
    pub fn event_standings(&self) -> Vec<Standing> {
        let Some(ev) = &self.event else {
            return Vec::new();
        };
        let riders: Vec<(RiderId, f64, f64)> = self
            .riders
            .iter()
            .map(|(id, r)| (*id, r.state.distance, r.state.speed))
            .collect();
        ev.standings(&riders)
    }

    /// The physics profile a rider rides with.
    pub fn rider_profile(&self, id: RiderId) -> Option<RiderProfile> {
        self.riders.get(&id).map(|r| r.profile)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A straight 1 km route that ends (not a loop).
    fn ending_route() -> Route {
        let pts = (0..=100).map(|i| [f64::from(i) * 10.0, 0.0, 0.0]).collect();
        Route::from_points("end", "End", pts, false, 6.0).unwrap()
    }

    /// Regression: `teleport 15` on the 14 km Vaduz-Malbun route put the
    /// rider at 15 km; the HUD counted on to 18 km while the avatar, the
    /// gradient and the profile stayed at the end and the speed kept
    /// rising. Now a teleport past the end goes to the end, and riders stop
    /// there (server and prediction alike); loops take any distance.
    #[test]
    fn teleport_and_riding_stop_at_the_end_of_a_route() {
        let mut s = Session::new(ending_route());
        let id = s.add_rider("a", RiderProfile::default());
        pedal(&mut s, id, 300.0);
        assert_eq!(s.teleport(id, 1500.0), Some(1000.0));
        assert_eq!(s.teleport(id, -20.0), Some(0.0));
        assert_eq!(s.teleport(RiderId(99), 10.0), None);
        // Riding into the end: stopped there, however long one pedals.
        assert_eq!(s.teleport(id, 980.0), Some(980.0));
        for _ in 0..(ocw_core::SIM_TICK_HZ as usize * 20) {
            s.step();
        }
        let st = s.rider_state(id).unwrap();
        assert_eq!(st.distance, 1000.0);
        assert_eq!(st.speed, 0.0);
        // The client's prediction stops there too.
        let mut p = crate::prediction::Predictor::new(RiderProfile::default(), st);
        let route = ending_route();
        for _ in 0..100 {
            p.step(&route, 300.0);
        }
        assert_eq!(p.state().distance, 1000.0);
        assert_eq!(p.state().speed, 0.0);
        // A loop: any distance (laps on).
        let mut s = Session::new(Route::test_loop());
        let id = s.add_rider("a", RiderProfile::default());
        let len = Route::test_loop().length();
        assert_eq!(s.teleport(id, len + 100.0), Some(len + 100.0));
    }

    #[test]
    fn stale_inputs_are_ignored() {
        let mut s = Session::new(Route::test_loop());
        let id = s.add_rider("a", RiderProfile::default());
        let input = |p| RiderInput {
            power_w: p,
            ..Default::default()
        };
        assert!(s.apply_input(id, 2, input(200.0)));
        assert!(!s.apply_input(id, 1, input(900.0)));
        assert!(!s.apply_input(id, 2, input(900.0)));
        assert_eq!(s.snapshot()[0].power_w, 200.0);
    }

    #[test]
    fn slow_bots_do_not_get_stuck_on_climbs() {
        // Regression: an 85 W bot crawled up 12 % at ~3 km/h.
        let climb = Route::from_points(
            "c",
            "C",
            vec![[0.0, 0.0, 0.0], [2000.0, 240.0, 0.0]],
            false,
            7.0,
        )
        .unwrap();
        let mut s = Session::new(climb);
        let bot = s.add_bot("slow", 85.0);
        for _ in 0..(60.0 / SIM_DT) as usize {
            s.step();
        }
        let st = s.rider_state(bot).unwrap();
        assert!(st.speed > BOT_MIN_CLIMB_SPEED - 0.1, "{st:?}");
        assert!(st.distance > 120.0, "{st:?}");
        // On the flat it rides at its own (easy) power.
        assert_eq!(bot_climb_power_w(&RiderProfile::default(), 0.0), 0.0);
    }

    #[test]
    fn bots_ride_and_clients_cannot_drive_bots() {
        let mut s = Session::new(Route::test_loop());
        let bot = s.add_bot("bot", 200.0);
        assert!(!s.apply_input(bot, 1, RiderInput::default()));
        for _ in 0..1500 {
            s.step();
        }
        // 30 s at ~200 W from standstill.
        assert!(s.rider_state(bot).unwrap().distance > 150.0);
        assert_eq!(s.tick(), 1500);
    }

    #[test]
    fn riders_get_separate_lanes() {
        let mut s = Session::new(Route::test_loop()).with_start(100.0);
        let spots: Vec<(f64, f64)> = (0..3)
            .map(|i| {
                let id = s.add_bot(format!("b{i}"), 100.0);
                let st = s.rider_state(id).unwrap();
                (st.distance, st.lateral_offset)
            })
            .collect();
        // Bots leave the front row's centre to the client rider.
        assert_eq!(spots, vec![(100.0, -1.2), (100.0, 1.2), (97.0, 0.0)]);
        let me = s.add_rider("me", RiderProfile::default());
        let st = s.rider_state(me).unwrap();
        assert_eq!((st.distance, st.lateral_offset), (100.0, 0.0));
    }

    /// (distance, lateral offset) of every rider.
    fn spots(s: &Session) -> Vec<(f64, f64)> {
        s.snapshot()
            .iter()
            .map(|r| (r.state.distance, r.state.lateral_offset))
            .collect()
    }

    fn assert_formation(spots: &[(f64, f64)], width: f64) {
        let limit = physics::lateral_limit(width);
        for (i, a) in spots.iter().enumerate() {
            assert!(a.1.abs() < limit, "{a:?} off the road");
            assert!(a.0 >= 0.0, "{a:?}");
            for b in &spots[i + 1..] {
                // Distinct spots, at least a rider's width apart sideways or
                // a row apart along the road.
                assert!(
                    (a.0 - b.0).abs() >= FORMATION_ROW_GAP_M - 1e-9 || (a.1 - b.1).abs() >= 1.0,
                    "{a:?} {b:?}"
                );
            }
        }
    }

    #[test]
    fn start_formation_scales_in_rows_behind_the_start() {
        let mut s = Session::new(Route::test_loop()).with_start(500.0);
        for i in 0..20 {
            s.add_bot(format!("b{i}"), 150.0);
        }
        s.add_rider("me", RiderProfile::default());
        let spots = spots(&s);
        assert_formation(&spots, Route::test_loop().road_width_m());
        // Three a row on the 7 m test road, rows 3 m apart behind the start.
        assert!(spots.iter().all(|&(d, _)| d <= 500.0));
        assert_eq!(spots.iter().filter(|&&(d, _)| d == 500.0).count(), 3);
        let last = spots.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
        assert_eq!(last, 500.0 - 6.0 * FORMATION_ROW_GAP_M);
        // Deterministic.
        let mut again = Session::new(Route::test_loop()).with_start(500.0);
        for i in 0..20 {
            again.add_bot(format!("b{i}"), 150.0);
        }
        again.add_rider("me", RiderProfile::default());
        assert_eq!(super::tests::spots(&again), spots);
    }

    #[test]
    fn start_formation_shifts_forward_at_the_route_start() {
        // Starting at 0: the rows go forward from the beginning instead.
        let mut s = Session::new(Route::test_loop());
        let bots: Vec<RiderId> = (0..10).map(|i| s.add_bot(format!("b{i}"), 150.0)).collect();
        let me = s.add_rider("me", RiderProfile::default());
        let spots = spots(&s);
        assert_formation(&spots, Route::test_loop().road_width_m());
        // 11 riders, slots 0..=10: four rows, the last one at 0.
        let first = s.rider_state(bots[0]).unwrap().distance;
        assert_eq!(first, 3.0 * FORMATION_ROW_GAP_M);
        assert_eq!(s.rider_state(me).unwrap().distance, first);
        assert_eq!(spots.iter().map(|p| p.0).fold(f64::INFINITY, f64::min), 0.0);
    }

    #[test]
    fn narrow_roads_get_fewer_riders_per_row() {
        assert_eq!(formation_row(7.0).len(), 3);
        assert_eq!(formation_row(3.0).len(), 2);
        assert_eq!(formation_row(2.0), vec![0.0]);
        for w in [1.0, 2.0, 2.5, 3.0, 3.5, 4.0, 5.0, 7.0, 12.0] {
            let limit = physics::lateral_limit(w);
            for l in formation_row(w) {
                assert!(l.abs() <= (limit - 0.3).max(0.0) + 1e-9, "{w} {l}");
            }
            let (back, _) = formation_slot(7, w);
            assert_eq!(
                back,
                (7 / formation_row(w).len()) as f64 * FORMATION_ROW_GAP_M
            );
        }
    }

    #[test]
    fn rider_behind_catches_up_by_drafting() {
        let mut s = Session::new(Route::test_loop());
        let leader = s.add_rider("Leader", RiderProfile::default());
        let follower = s.add_rider("Follower", RiderProfile::default());
        s.teleport(leader, 1005.0);
        s.teleport(follower, 1000.0);
        for (seq, id) in [(1, leader), (1, follower)] {
            s.apply_input(
                id,
                seq,
                RiderInput {
                    power_w: 250.0,
                    cadence_rpm: None,
                    heart_rate_bpm: None,
                    brake: false,
                },
            );
        }
        for _ in 0..(60 * 50) {
            s.step();
        }
        let gap =
            s.rider_state(leader).unwrap().distance - s.rider_state(follower).unwrap().distance;
        // Same power: without drafting the gap would stay at 5 m.
        assert!(gap < 4.5, "gap {gap}");
    }

    #[test]
    fn client_riders_steer_across_the_road() {
        let mut s = Session::new(Route::test_loop());
        let me = s.add_rider("Me", RiderProfile::default());
        let bot = s.add_bot("Bot", 200.0);
        let start = s.rider_state(me).unwrap().lateral_offset;
        assert!(s.set_lateral_target(me, 1, 2.0));
        for _ in 0..25 {
            s.step(); // 0.5 s
        }
        // Standing still, a bike cannot move sideways.
        assert_eq!(s.rider_state(me).unwrap().lateral_offset, start);
        let ride = RiderInput {
            power_w: 250.0,
            cadence_rpm: None,
            heart_rate_bpm: None,
            brake: false,
        };
        s.apply_input(me, 1, ride);
        for _ in 0..(15 * 50) {
            s.step();
        }
        // Autodrive leans over to the new line and holds it in the curves.
        let at = s.rider_state(me).unwrap().lateral_offset;
        assert!((at - 2.0).abs() < 0.15, "{at}");
        // Clamped to the road (7 m wide → ±2.9 m).
        assert!(s.set_lateral_target(me, 2, 10.0));
        for _ in 0..(15 * 50) {
            s.step();
        }
        let at = s.rider_state(me).unwrap().lateral_offset;
        assert!(at <= 2.9 + 1e-9 && at > 2.7, "{at}");
        // Stale intents and bots are ignored.
        assert!(!s.set_lateral_target(me, 2, -2.0));
        assert!(!s.set_lateral_target(bot, 3, 0.0));
    }

    #[test]
    fn brake_input_stops_a_client_rider() {
        let mut s = Session::new(Route::test_loop());
        let me = s.add_rider("Me", RiderProfile::default());
        let ride = |brake| RiderInput {
            power_w: 250.0,
            cadence_rpm: None,
            heart_rate_bpm: None,
            brake,
        };
        s.apply_input(me, 1, ride(false));
        for _ in 0..(20 * 50) {
            s.step();
        }
        assert!(s.rider_state(me).unwrap().speed > 5.0);
        s.apply_input(
            me,
            2,
            RiderInput {
                power_w: 0.0,
                ..ride(true)
            },
        );
        for _ in 0..(5 * 50) {
            s.step();
        }
        assert_eq!(s.rider_state(me).unwrap().speed, 0.0);
    }

    /// A 4 × 4 grid of crossroads 300 m apart (vertex = 4·row + col at
    /// x = 300·col, z = 300·row).
    pub(crate) fn grid_network() -> Arc<RoadNetwork> {
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
        let edges = pairs
            .iter()
            .enumerate()
            .map(|(k, &(a, b))| Edge {
                id: EdgeId(500 + k as u64),
                from: VertexId(a),
                to: VertexId(b),
                points: vec![vertices[a as usize], vertices[b as usize]],
                class: RoadClass::Tertiary,
                name: None,
                width_m: 6.0,
                bridge: false,
                tunnel: false,
                dead_end_m: None,
                boundary: false,
                oneway: Oneway::No,
                osm_way: k as i64,
                profile: Vec::new(),
            })
            .collect();
        Arc::new(RoadNetwork::new(RoadGraph {
            vertices,
            edges,
            boundary_vertices: Default::default(),
        }))
    }

    /// Free ride starting on the top row's second edge (vertex 1 → 2),
    /// 50 m in, riding east.
    pub(crate) fn free_session() -> Session {
        let net = grid_network();
        let edge = (0..net.graph().edges.len())
            .find(|&e| net.edge(e).from.0 == 1 && net.edge(e).to.0 == 2)
            .unwrap();
        Session::free_ride(
            net,
            FreeRideStart {
                edge,
                offset_m: 50.0,
                forward: true,
            },
        )
    }

    fn pedal(s: &mut Session, id: RiderId, watts: f64) {
        s.apply_input(
            id,
            1,
            RiderInput {
                power_w: watts,
                ..Default::default()
            },
        );
    }

    #[test]
    fn free_ride_bots_spread_over_the_graph() {
        let mut s = free_session();
        assert!(s.is_free_ride());
        let bots: Vec<RiderId> = (0..4).map(|i| s.add_bot(format!("b{i}"), 200.0)).collect();
        let starts = s.take_path_changes();
        assert_eq!(starts.len(), 4);
        assert!(starts.iter().all(|c| c.reset && c.first_index == 0));
        let (mut changes, mut spread) = (0, 0.0f64);
        for t in 0..(240 * 50) {
            s.step();
            changes += s.take_path_changes().len();
            if t % 250 == 0 {
                // How far apart the bots are (they part at junctions).
                let at: Vec<[f64; 3]> = bots
                    .iter()
                    .map(|&b| {
                        s.track(b)
                            .sample(s.rider_state(b).unwrap().distance)
                            .position
                    })
                    .collect();
                for a in &at {
                    for b in &at {
                        spread = spread.max((a[0] - b[0]).hypot(a[2] - b[2]));
                    }
                }
            }
        }
        // Four minutes: over a kilometre each, through several junctions,
        // and their ways part.
        let ends: Vec<[f64; 3]> = bots
            .iter()
            .map(|&b| {
                let d = s.rider_state(b).unwrap().distance;
                assert!(d > 1200.0, "{d}");
                s.track(b).sample(d).position
            })
            .collect();
        assert!(changes >= 12, "{changes}");
        assert!(spread > 150.0, "{spread}");
        // Positions stay on the grid roads (x or z a multiple of 300 m).
        for p in &ends {
            let on = |v: f64| (v / 300.0 - (v / 300.0).round()).abs() * 300.0 < 4.0;
            assert!(on(p[0]) || on(p[2]), "{p:?}");
        }
    }

    #[test]
    fn free_ride_formation_starts_behind_on_the_start_edge() {
        let mut s = free_session();
        let bots: Vec<RiderId> = (0..10).map(|i| s.add_bot(format!("b{i}"), 200.0)).collect();
        let me = s.add_rider("me", RiderProfile::default());
        let width = s
            .network()
            .unwrap()
            .edge(s.free.as_ref().unwrap().1.edge)
            .width_m;
        // Everyone at distance 0 of their own path; the paths start on
        // the start edge at the formation's places.
        let mut at: Vec<(f64, f64)> = Vec::new();
        for &id in bots.iter().chain([&me]) {
            let st = s.rider_state(id).unwrap();
            assert_eq!(st.distance, 0.0);
            let (e, fwd, o) = s.path(id).unwrap().locate(0.0).unwrap();
            assert_eq!(e, s.free.as_ref().unwrap().1.edge);
            assert!(fwd);
            at.push((o, st.lateral_offset));
        }
        assert_formation(&at, width);
        // Front row at the start point (50 m), the client in its centre.
        assert_eq!(at[10], (50.0, 0.0));
        assert!(at.iter().all(|&(o, _)| (41.0..=50.0).contains(&o)));
        // They ride off without trouble.
        for _ in 0..(30 * 50) {
            s.step();
        }
        for &b in &bots {
            assert!(s.rider_state(b).unwrap().distance > 100.0);
        }
    }

    #[test]
    fn free_ride_formation_moves_forward_near_the_edge_start() {
        let net = grid_network();
        let edge = (0..net.graph().edges.len())
            .find(|&e| net.edge(e).from.0 == 1 && net.edge(e).to.0 == 2)
            .unwrap();
        let mut s = Session::free_ride(
            net,
            FreeRideStart {
                edge,
                offset_m: 2.0,
                forward: true,
            },
        );
        for i in 0..9 {
            s.add_bot(format!("b{i}"), 200.0);
        }
        let at: Vec<(f64, f64)> = s
            .snapshot()
            .iter()
            .map(|r| {
                let (_, _, o) = s.path(r.id).unwrap().locate(0.0).unwrap();
                (o, r.state.lateral_offset)
            })
            .collect();
        assert_formation(&at, s.network().unwrap().edge(edge).width_m);
        // Slots 1..=9: four rows, the last one at the edge's start.
        let min = at.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
        assert!(min.abs() < 1e-9, "{at:?}");
        // The path changes broadcast the final places.
        let changes = s.take_path_changes();
        assert_eq!(changes.len(), 9);
        assert!(changes.iter().all(|c| c.reset && c.start_m == 0.0));
    }

    #[test]
    fn route_ride_bots_stay_on_the_route_riders_may_leave_and_rejoin() {
        let net = grid_network();
        let e = |a: u32, b: u32| {
            (0..net.graph().edges.len())
                .find_map(|i| {
                    let x = net.edge(i);
                    if (x.from.0, x.to.0) == (a, b) {
                        Some(Segment::whole(&net, i, true))
                    } else if (x.from.0, x.to.0) == (b, a) {
                        Some(Segment::whole(&net, i, false))
                    } else {
                        None
                    }
                })
                .unwrap()
        };
        // East along the top row to vertex 2, south to 10, east to 11.
        let plan = RoutePlan::new(
            &net,
            "test route",
            vec![e(0, 1), e(1, 2), e(2, 6), e(6, 10), e(10, 11)],
            false,
        )
        .unwrap();
        let mut s = Session::route_ride(net.clone(), Arc::new(plan));
        assert_eq!(s.route().name(), "test route");
        assert!(s.plan().is_some() && s.is_free_ride());
        let me = s.add_rider("me", RiderProfile::default());
        let bot = s.add_bot("bot", 220.0);
        pedal(&mut s, me, 250.0);
        // At vertex 1 (straight on is the route) the client turns right.
        let (j, _) = s.path(me).unwrap().next_junction().unwrap();
        assert_eq!(j.route, Some(j.default));
        let right = j
            .exits
            .iter()
            .find(|x| x.class == ocw_world::TurnClass::Right)
            .unwrap();
        let choice = TurnChoice {
            junction: j.vertex,
            edge: right.edge,
            forward: right.forward,
        };
        assert!(s.set_turn(me, 1, Some(choice)));
        for _ in 0..(60 * 50) {
            s.step();
        }
        let off = s.path(me).unwrap().route_state().unwrap();
        assert!(
            matches!(off, ocw_world::RouteState::Off { left_at: 1, .. }),
            "{off:?}"
        );
        // The bot is still on it.
        let bot_d = s.rider_state(bot).unwrap().distance;
        assert!(s.path(bot).unwrap().on_route_at(bot_d));
        // Back to the route.
        s.rejoin_route(me, None).unwrap();
        assert!(matches!(
            s.path(me).unwrap().route_state(),
            Some(ocw_world::RouteState::On { .. })
        ));
        assert!(!s.take_path_changes().is_empty());
        assert!(s.rejoin_route(me, None).is_err(), "already on it");
        for _ in 0..(150 * 50) {
            s.step();
        }
        let d = s.rider_state(me).unwrap().distance;
        assert!(
            s.path(me).unwrap().on_route_at(d)
                || s.path(me).unwrap().route_progress(d).unwrap().finished
        );
        // Plain free rides cannot rejoin anything.
        let mut f = free_session();
        let r = f.add_rider("r", RiderProfile::default());
        assert!(f.rejoin_route(r, None).is_err());
    }

    #[test]
    fn route_ride_formation_starts_on_the_route_from_its_start() {
        let net = grid_network();
        let e = |a: u32, b: u32| {
            let i = (0..net.graph().edges.len())
                .find(|&i| (net.edge(i).from.0, net.edge(i).to.0) == (a, b))
                .unwrap();
            Segment::whole(&net, i, true)
        };
        let plan = RoutePlan::new(&net, "row", vec![e(0, 1), e(1, 2), e(2, 3)], false).unwrap();
        let mut s = Session::route_ride(net.clone(), Arc::new(plan));
        let bots: Vec<RiderId> = (0..9).map(|i| s.add_bot(format!("b{i}"), 200.0)).collect();
        let me = s.add_rider("me", RiderProfile::default());
        let spots = spots(&s);
        assert_formation(&spots, 6.0);
        // The route begins at the start point: the rows go forward from
        // it, the client in the front row's centre, everyone on the route.
        assert_eq!(spots.iter().map(|p| p.0).fold(f64::INFINITY, f64::min), 0.0);
        let front = 3.0 * FORMATION_ROW_GAP_M;
        let st = s.rider_state(me).unwrap();
        assert_eq!((st.distance, st.lateral_offset), (front, 0.0));
        for &id in bots.iter().chain([&me]) {
            let d = s.rider_state(id).unwrap().distance;
            assert!(s.path(id).unwrap().on_route_at(d));
        }
    }

    #[test]
    fn free_ride_client_turns_where_it_chose() {
        let mut s = free_session();
        let me = s.add_rider("me", RiderProfile::default());
        pedal(&mut s, me, 250.0);
        // The next junction is vertex 2; choose right (south).
        let (j, _) = s.path(me).unwrap().next_junction().unwrap();
        let right = j
            .exits
            .iter()
            .find(|e| e.class == ocw_world::TurnClass::Right)
            .unwrap();
        let choice = TurnChoice {
            junction: j.vertex,
            edge: right.edge,
            forward: right.forward,
        };
        assert!(s.set_turn(me, 1, Some(choice)));
        assert!(!s.set_turn(me, 1, None), "stale");
        for _ in 0..(60 * 50) {
            s.step();
        }
        let d = s.rider_state(me).unwrap().distance;
        assert!(d > 400.0, "{d}");
        let p = s.track(me).sample(d);
        // South of the top row, on the x = 600 m road.
        assert!(
            (p.position[0] - 600.0).abs() < 4.0 && p.position[2] > 100.0,
            "{p:?}"
        );
        assert!(p.direction[1] > 0.99);
    }

    #[test]
    fn free_ride_u_turns_are_rate_limited() {
        let mut s = free_session();
        let me = s.add_rider("me", RiderProfile::default());
        pedal(&mut s, me, 250.0);
        for _ in 0..(10 * 50) {
            s.step();
        }
        s.take_path_changes();
        assert!(s.u_turn(me, None));
        assert!(!s.u_turn(me, None), "rate limited");
        assert_eq!(s.take_path_changes().len(), 1);
        for _ in 0..(30 * 50) {
            s.step();
        }
        let st = s.rider_state(me).unwrap();
        let after = s.track(me).sample(st.distance);
        // Riding west now, and riding on after the slow loop.
        assert!(after.direction[0] < -0.99, "{after:?}");
        assert!(st.speed > 4.0);
        // Route sessions have no U-turns.
        let mut r = Session::new(Route::test_loop());
        let id = r.add_rider("r", RiderProfile::default());
        assert!(!r.u_turn(id, None));
    }

    #[test]
    fn free_ride_drafting_works_on_shared_roads() {
        let mut s = free_session();
        let a = s.add_rider("a", RiderProfile::default());
        let b = s.add_rider("b", RiderProfile::default());
        pedal(&mut s, a, 250.0);
        pedal(&mut s, b, 250.0);
        // a starts 5 m ahead of b, both in the middle of the road.
        s.teleport(a, 5.0);
        s.set_lateral_target(a, 1, 0.0);
        s.set_lateral_target(b, 1, 0.0);
        for _ in 0..(30 * 50) {
            s.step();
        }
        let gap = s.rider_state(a).unwrap().distance - s.rider_state(b).unwrap().distance;
        assert!(gap < 4.5, "gap {gap}");
    }

    fn steps(s: &mut Session, seconds: f64) {
        for _ in 0..(seconds / SIM_DT).round() as usize {
            s.step();
        }
    }

    #[test]
    fn race_lines_up_holds_through_the_countdown_and_ranks_the_finish() {
        use crate::event::{COUNTDOWN_S, EventChange, Phase};
        let mut s = Session::new(Route::test_loop()).with_start(300.0);
        let slow = s.add_bot("slow", 100.0);
        let fast = s.add_bot("fast", 300.0);
        let me = s.add_rider("me", RiderProfile::default());
        pedal(&mut s, me, 200.0);
        steps(&mut s, 10.0);
        let lap = s.route().length();
        let ev = s
            .start_event(EventSpec::Race {
                laps: None,
                length_m: Some(400.0),
            })
            .unwrap();
        // A loop: the start line is the start / finish, one lap in.
        assert_eq!(ev.start_line_m, lap);
        assert_eq!(ev.phase, Phase::Countdown);
        assert_eq!(ev.entrants.len(), 3);
        assert_eq!(
            s.take_event_changes(),
            vec![EventChange::Phase(Phase::Countdown)]
        );
        // The client in the front row's centre, the bots beside it.
        let st = s.rider_state(me).unwrap();
        assert_eq!((st.distance, st.lateral_offset, st.speed), (lap, 0.0, 0.0));
        assert_eq!(s.rider_state(slow).unwrap().distance, lap);
        assert_eq!(s.rider_state(fast).unwrap().lateral_offset, 1.2);
        // Held during the countdown although everyone pedals.
        steps(&mut s, COUNTDOWN_S - 0.5);
        assert_eq!(s.rider_state(me).unwrap().distance, lap);
        assert_eq!(s.event().unwrap().phase, Phase::Countdown);
        steps(&mut s, 1.0);
        assert_eq!(s.event().unwrap().phase, Phase::Running);
        assert!(s.rider_state(me).unwrap().distance > lap);
        assert!(
            s.take_event_changes()
                .contains(&EventChange::Phase(Phase::Running))
        );
        // Live standings: the strong bot leads.
        steps(&mut s, 20.0);
        let st = s.event_standings();
        assert_eq!(st[0].id, fast, "{st:?}");
        assert_eq!(st.last().unwrap().id, slow);
        // Everyone finishes; times in order, the race is over.
        steps(&mut s, 120.0);
        let ev = s.event().unwrap();
        assert_eq!(ev.phase, Phase::Finished, "{ev:?}");
        let st = s.event_standings();
        assert_eq!(
            st.iter().map(|r| r.id).collect::<Vec<_>>(),
            [fast, me, slow]
        );
        let times: Vec<f64> = st.iter().map(|r| r.finish_s.unwrap()).collect();
        assert!(times.windows(2).all(|w| w[0] < w[1]), "{times:?}");
        // 400 m at 300 W from standing: roughly 35 to 60 s.
        assert!((30.0..70.0).contains(&times[0]), "{times:?}");
        let finishes = s
            .take_event_changes()
            .iter()
            .filter(|c| matches!(c, EventChange::Finished { .. }))
            .count();
        assert_eq!(finishes, 3);
        // After the finish everyone rides on.
        let d = s.rider_state(me).unwrap().distance;
        steps(&mut s, 1.0);
        assert!(s.rider_state(me).unwrap().distance > d);
        assert!(s.cancel_event().is_some());
        assert!(s.event().is_none());
    }

    #[test]
    fn open_routes_start_behind_the_formation_and_check_the_length() {
        let road = Route::from_points(
            "road",
            "Road",
            vec![[0.0, 0.0, 0.0], [2000.0, 0.0, 0.0]],
            false,
            7.0,
        )
        .unwrap();
        let mut s = Session::new(road).with_start(800.0);
        for i in 0..5 {
            s.add_bot(format!("b{i}"), 200.0);
        }
        s.add_rider("me", RiderProfile::default());
        let err = s
            .start_event(EventSpec::Race {
                laps: Some(2),
                length_m: None,
            })
            .unwrap_err();
        assert!(err.contains("loop"), "{err}");
        assert!(
            s.start_event(EventSpec::Race {
                laps: None,
                length_m: Some(2500.0),
            })
            .is_err()
        );
        let ev = s
            .start_event(EventSpec::Race {
                laps: None,
                length_m: None,
            })
            .unwrap();
        // Six riders: two rows, the last at 0 m, the line 3 m ahead.
        assert_eq!(ev.start_line_m, FORMATION_ROW_GAP_M);
        assert_eq!(
            ev.kind.length_m(),
            Some(crate::event::DEFAULT_LENGTH_M.min(2000.0 - FORMATION_ROW_GAP_M))
        );
        let min = spots(&s).iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
        assert_eq!(min, 0.0);
        // Free ride has no course.
        let mut f = free_session();
        f.add_rider("r", RiderProfile::default());
        assert!(
            f.start_event(EventSpec::TimeTrial {
                laps: None,
                length_m: None
            })
            .is_err()
        );
    }

    #[test]
    fn time_trials_have_no_drafting() {
        // Two equal riders, one 2 m straight behind the other: in a race
        // the one behind drafts and closes up; in a time trial the gap
        // stays.
        let run = |spec: EventSpec| {
            let mut s = Session::new(Route::test_loop());
            let a = s.add_rider("a", RiderProfile::default());
            let b = s.add_rider("b", RiderProfile::default());
            pedal(&mut s, a, 200.0);
            pedal(&mut s, b, 200.0);
            s.start_event(spec).unwrap();
            let d = s.rider_state(a).unwrap().distance;
            s.teleport(b, d - 2.0);
            s.set_lateral_target(b, 1, 0.0);
            steps(&mut s, crate::event::COUNTDOWN_S + 60.0);
            s.rider_state(a).unwrap().distance - s.rider_state(b).unwrap().distance
        };
        let race = run(EventSpec::Race {
            laps: None,
            length_m: Some(3000.0),
        });
        let tt = run(EventSpec::TimeTrial {
            laps: None,
            length_m: Some(3000.0),
        });
        assert!((tt - 2.0).abs() < 0.3, "tt gap {tt}");
        assert!(race < tt - 0.5, "race gap {race}, tt gap {tt}");
    }

    #[test]
    fn group_ride_bots_hold_the_pace_and_regroup() {
        let mut s = Session::new(Route::test_loop());
        // A strong and an easy bot would split up on their own.
        let strong = s.add_bot("strong", 280.0);
        let easy = s.add_bot("easy", 85.0);
        s.start_event(EventSpec::GroupRide {
            target_w: Some(160.0),
        })
        .unwrap();
        steps(&mut s, crate::event::COUNTDOWN_S + 300.0);
        let gap = s.rider_state(strong).unwrap().distance - s.rider_state(easy).unwrap().distance;
        assert!(gap.abs() < 30.0, "gap {gap}");
        // Their power stays around the target.
        for r in s.snapshot() {
            assert!((60.0..300.0).contains(&r.power_w), "{r:?}");
        }
        assert!(s.event_standings().iter().all(|r| r.finish_s.is_none()));
        assert_eq!(s.event().unwrap().phase, crate::event::Phase::Running);
    }

    #[test]
    fn route_ride_events_put_everyone_back_on_the_route() {
        let net = grid_network();
        let e = |a: u32, b: u32| {
            let i = (0..net.graph().edges.len())
                .find(|&i| (net.edge(i).from.0, net.edge(i).to.0) == (a, b))
                .unwrap();
            Segment::whole(&net, i, true)
        };
        let plan = RoutePlan::new(&net, "row", vec![e(0, 1), e(1, 2), e(2, 3)], false).unwrap();
        let mut s = Session::route_ride(net.clone(), Arc::new(plan));
        let bot = s.add_bot("b", 200.0);
        let me = s.add_rider("me", RiderProfile::default());
        steps(&mut s, 30.0);
        s.take_path_changes();
        let ev = s
            .start_event(EventSpec::TimeTrial {
                laps: None,
                length_m: Some(600.0),
            })
            .unwrap();
        assert_eq!(ev.start_line_m, 0.0);
        for id in [me, bot] {
            let st = s.rider_state(id).unwrap();
            assert!(st.distance.abs() < 1e-9, "{st:?}");
            assert!(s.path(id).unwrap().on_route_at(st.distance));
        }
        // The clients get the new paths.
        let changes = s.take_path_changes();
        assert_eq!(changes.len(), 2);
        assert!(changes.iter().all(|c| c.reset));
    }
}
