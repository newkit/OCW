//! Authoritative session logic: all riders of one ride, stepped at a fixed tick (spec §6.1).
//!
//! Used by the remote game server and by the in-process local server alike.
//! A session rides a fixed route, or is a free ride on the road graph where
//! every rider has a path of their own that grows at each junction
//! (spec §2.2.1, §5.2).

use std::collections::BTreeMap;
use std::sync::Arc;

use ocw_core::{RiderId, RiderProfile, SIM_DT};
use ocw_world::network::splitmix;
use ocw_world::{RidePath, RoadNetwork, Route, Segment, Track};

use crate::drafting;
use crate::freeride::{self, TurnChoice, UTURN_INTERVAL_S};
use crate::physics::{self, Environment, RiderState, Steering};
use crate::ride::{self, Controls};

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
}

/// State of one rider as broadcast to clients.
#[derive(Debug, Clone, PartialEq)]
pub struct RiderSnapshot {
    pub id: RiderId,
    pub name: String,
    pub state: RiderState,
    pub power_w: f64,
    /// Sequence number of the last input from this rider's client the server applied.
    pub last_input_seq: u32,
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
    physics::resistance(profile, &env, v).total() * v / physics::DRIVETRAIN_EFFICIENCY
}

/// One ride: a route (or the road graph) plus the riders on it.
#[derive(Debug, Clone)]
pub struct Session {
    /// The route; in free ride the start of the first path (for its name).
    route: Route,
    /// Free ride: the road network and where riders start.
    free: Option<(Arc<RoadNetwork>, FreeRideStart)>,
    riders: BTreeMap<RiderId, Rider>,
    tick: u64,
    next_id: u32,
    /// Where riders start on the route, in metres.
    start_m: f64,
}

impl Session {
    pub fn new(route: Route) -> Self {
        Self {
            route,
            free: None,
            riders: BTreeMap::new(),
            tick: 0,
            next_id: 1,
            start_m: 0.0,
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
            free: Some((net, start)),
            ..Self::new(first.route().clone())
        }
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
        self.insert(
            name.into(),
            RiderProfile::default(),
            Control::Bot { base_power_w },
        )
    }

    fn insert(&mut self, name: String, profile: RiderProfile, control: Control) -> RiderId {
        let id = RiderId(self.next_id);
        self.next_id += 1;
        // Spread riders across the lane: 0, +1.2, -1.2, +2.4, -2.4 m ...
        let slot = self.riders.len() as f64;
        let side = if self.riders.len().is_multiple_of(2) {
            1.0
        } else {
            -1.0
        };
        let mut lateral_offset = side * ((slot + 1.0) / 2.0).floor() * 1.2;
        let (path, distance) = match &self.free {
            Some((net, s)) => {
                let mut p = RidePath::start(net.clone(), s.edge, s.offset_m, s.forward, 0.0, 0);
                p.extend(0.0, &mut |j| j.default);
                // Narrow roads: stay on them (off the edge, or it scrapes).
                let limit = (physics::lateral_limit(p.road_width_at(0.0)) - 0.3).max(0.0);
                lateral_offset = lateral_offset.clamp(-limit, limit);
                (Some(p), 0.0)
            }
            None => (None, self.start_m),
        };
        self.riders.insert(
            id,
            Rider {
                name,
                profile,
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
            },
        );
        id
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

    /// Moves a rider to `distance` metres (dev command, local server only).
    /// In free ride only forwards along the path (junctions take their
    /// default exits on the way).
    pub fn teleport(&mut self, id: RiderId, distance: f64) -> bool {
        let Some(r) = self.riders.get_mut(&id) else {
            return false;
        };
        match &mut r.path {
            Some(path) => {
                if let Some(i) = path.extend(distance, &mut |j| j.default) {
                    r.path_changed = Some(r.path_changed.map_or(i, |c| c.min(i)));
                }
                r.state.distance = distance.max(path.begin_m());
            }
            None => r.state.distance = distance.max(0.0),
        }
        true
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
        let mut path = RidePath::start(net, edge, offset, forward, d, next);
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
        // Drafting uses everyone's position at the start of the tick (in
        // free ride also where on the road graph they are).
        let positions: Vec<(RiderId, f64, f64, Option<EdgePoint>)> = self
            .riders
            .iter()
            .map(|(id, r)| {
                let at = r.path.as_ref().and_then(|p| p.locate(r.state.distance));
                (*id, r.state.distance, r.state.lateral_offset, at)
            })
            .collect();
        for (id, rider) in &mut self.riders {
            let d = rider.state.distance;
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
            let drag = drafting::drag_factor((d, rider.state.lateral_offset), ahead);
            let track: &dyn Track = match &rider.path {
                Some(p) => p,
                None => &self.route,
            };
            let grade = track.sample(d).grade;
            if let Control::Bot { base_power_w } = rider.control {
                // Deterministic variation so bots don't ride in perfect lockstep.
                let phase = id.0 as f64 * 1.7;
                let cruising = base_power_w * (1.0 + 0.1 * (t * 0.2 + phase).sin());
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
    }

    /// Current state of all riders.
    pub fn snapshot(&self) -> Vec<RiderSnapshot> {
        self.riders
            .iter()
            .map(|(id, r)| RiderSnapshot {
                id: *id,
                name: r.name.clone(),
                state: r.state,
                power_w: r.input.power_w,
                last_input_seq: r.last_input_seq,
            })
            .collect()
    }

    pub fn rider_state(&self, id: RiderId) -> Option<RiderState> {
        self.riders.get(&id).map(|r| r.state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let mut s = Session::new(Route::test_loop());
        let offsets: Vec<f64> = (0..3)
            .map(|i| {
                let id = s.add_bot(format!("b{i}"), 100.0);
                s.rider_state(id).unwrap().lateral_offset
            })
            .collect();
        assert_eq!(offsets, vec![0.0, -1.2, 1.2]);
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
}
