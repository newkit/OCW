//! Road traffic (spec 14 Phase 18, 18B; milestone T3): vehicles on the
//! road graph, pure and deterministic, the same on the server and in
//! the browser (wasm).
//!
//! - **Vehicles** ([`VehicleClass`]: car, van, lorry, bus, motorbike,
//!   tractor) move in 1D along a route of graph edges ([`plan::Step`]),
//!   in a lane of their direction (0 = kerb side), like riders (5.2).
//! - **Car following** by the Intelligent Driver Model ([`kin::idm`]),
//!   towards a desired speed of the limit × a driver factor, lowered ahead
//!   of curves, turns and lower limits ([`kin::desired_speed`]); lane
//!   changes on roads with more lanes a direction (MOBIL-like: a clear
//!   gain, a safe gap, back to the kerb lane when it costs little).
//! - **Junctions** ([`Control`]): signals with cycles from the tick
//!   ([`signals`]), `stop` (halt, then give way), give way, right before
//!   left on equal minor roads (left where traffic keeps left), priority
//!   of the higher class; vehicles do not enter a junction whose exit has
//!   no room.
//! - **Routes** respect `oneway` and access per class ([`EdgeRules`]; the
//!   default [`ClassRules`] derives everything from the road class, T2's
//!   `TRFC` data implements the trait later).
//! - **Spawning** only around the riders' [`Bubble`]s: vehicles enter at
//!   the bubble's rim (and the region's boundary) at the demand of the
//!   hour ([`demand`]), the edges of a new bubble are filled to their
//!   density, vehicles leave and are despawned outside.
//! - **Plans** (Q3 (a)): every vehicle is a [`Plan`] plus [`PlanChange`]s
//!   ([`TrafficEvent`]); a client's [`PlanFollower`]s reproduce the
//!   server's vehicles within [`plan::POS_TOLERANCE_M`].
//! - **Riders** are given as [`RoadUser`]s each step (T4, [`around`]):
//!   vehicles wait behind a rider or a group (one block) with a 10 m gap
//!   and overtake with the region's legal lateral distance only with a
//!   free oncoming lane and the sight distance for the whole manoeuvre
//!   (bends and crests from the road geometry, no junction in it,
//!   `overtaking=no`); they yield to riders at every junction; the
//!   drivers' manners (Q8) and the lunatics (Q4) change how; a vehicle
//!   that cannot stop in time hits the rider when collisions are on
//!   ([`RiderEvent::Contact`]); passing lorries push riders sideways
//!   ([`Gust`]); a vehicle closing in from behind gives the proximity cue
//!   ([`RiderEvent::Approaching`]).
//! - **Determinism:** the same seed, network, rules and inputs give
//!   byte-identical states ([`TrafficSim::digest`]); randomness is hashed
//!   from (seed, tick, edge, …), never drawn in iteration order.

pub mod around;
pub mod demand;
pub mod kin;
pub mod plan;
pub mod routing;
pub mod rules;
pub mod signals;
pub mod world;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_around;

use ocw_world::RoadNetwork;
use ocw_world::graph::{RoadClass, VertexId};

pub use around::{
    ContactSide, Gust, Manner, PassRecord, PassRule, RiderEvent, VehicleBody, VehicleContact,
};
pub use demand::{DemandTime, DriverManners, Region, TrafficLevel};
pub use kin::{Curves, DriverParams, RoutePos};
pub use plan::{Motion, Plan, PlanChange, PlanFollower, Shift, Step, Tick, TrafficEvent};
pub use rules::{Access, ClassRules, Control, EdgeRule, EdgeRules, VehicleClass};
pub use world::WorldRules;

use routing::{key, unit_f};
use signals::Light;

/// The session's traffic settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrafficConfig {
    pub seed: u64,
    /// The fixed step (ms; 20 = the 50 Hz physics tick, 5.2).
    pub step_ms: u32,
    pub level: TrafficLevel,
    pub manners: DriverManners,
    /// No more vehicles than this at once (the server's budget).
    pub max_vehicles: usize,
    /// Vehicles are despawned this far outside every bubble (m).
    pub margin_m: f64,
    /// `ride.collisions` (12A, Q4): vehicles and riders touch
    /// ([`RiderEvent::Contact`]); off, a vehicle that cannot stop brakes
    /// hard and passes through the rider.
    pub collisions: bool,
}

impl Default for TrafficConfig {
    fn default() -> Self {
        Self {
            seed: 1,
            step_ms: 20,
            level: TrafficLevel::Normal,
            manners: DriverManners::Polite,
            max_vehicles: 500,
            margin_m: 150.0,
            collisions: false,
        }
    }
}

/// The area traffic is simulated in: a circle around a rider (spec 18A:
/// 1.5 km).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bubble {
    pub center: [f64; 2],
    pub radius: f64,
}

/// A rider on the road (T4): vehicles wait behind, pass with the legal
/// distance, yield at junctions and can touch it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoadUser {
    /// The rider's id (in the events).
    pub id: u32,
    pub edge: usize,
    /// Metres of the front wheel from the edge's `from` vertex.
    pub s: f64,
    /// Riding from `from` to `to`.
    pub forward: bool,
    /// Speed along the road (m/s).
    pub speed: f64,
    /// Front wheel to rear wheel (m; 12B's 1.75).
    pub length: f64,
    /// Metres right of the road's centreline, in the riding direction
    /// (the rider's lateral offset `d` of 5.2).
    pub lateral: f64,
}

impl Default for RoadUser {
    fn default() -> Self {
        Self {
            id: 0,
            edge: 0,
            s: 0.0,
            forward: true,
            speed: 0.0,
            length: crate::collide::BODY_LENGTH_M,
            lateral: 1.5,
        }
    }
}

/// Gap vehicles keep behind a rider (m; spec 18B: they wait at the
/// rider's speed with a 10 m gap).
pub const RIDER_GAP_M: f64 = 10.0;
/// The pulling-out target is this much more than needed, and a rider
/// blocks only this much inside the passing distance (m): the shift
/// travels as `f32` in the plans.
const SHIFT_SLACK_M: f64 = 0.01;
/// The gap a vehicle closes to while it pulls out to pass (m).
const OUT_GAP_M: f64 = 3.0;
/// A vehicle yields to a rider arriving at a junction within this many
/// seconds of its own arrival (18B: vehicles always yield to riders).
const RIDER_YIELD_S: f64 = 4.0;
/// Leaders farther than this are not looked for (m).
const LOOK_AHEAD_M: f64 = 200.0;
/// Junction logic starts this far before the vertex (m).
const JUNCTION_M: f64 = 80.0;
/// A vehicle that crossed a vertex is "in the junction" for this many metres.
const BOX_M: f64 = 10.0;
/// A vehicle stuck this long is removed (s; a safety valve).
const STUCK_S: f64 = 600.0;

/// A vehicle as the renderer, the tests and the debug plot see it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VehicleView {
    pub id: u32,
    pub class: VehicleClass,
    pub look: u32,
    pub edge: usize,
    pub forward: bool,
    /// Metres from the edge's `from` vertex.
    pub s: f64,
    pub speed: f64,
    pub lane: u8,
    /// Metres right of the centreline (in the driving direction).
    pub lateral: f64,
    /// Position (x, z) and heading (unit, x, z).
    pub xz: [f64; 2],
    pub heading: [f64; 2],
}

/// Counters for tests and tools.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TrafficStats {
    pub spawned: u64,
    pub despawned: u64,
    pub changes: u64,
    /// Vehicle-ticks simulated.
    pub vehicle_ticks: u64,
}

#[derive(Debug, Clone)]
struct Vehicle {
    /// The plan as the clients extrapolate it (route, driver, ghost state).
    f: PlanFollower,
    pos: RoutePos,
    v: f64,
    a: f64,
    lane: u8,
    /// The last step's acceleration came from a leader or a line.
    interacting: bool,
    /// The speed of the binding constraint.
    lead_v: f64,
    /// The control at the end of the current step, for (step, route length).
    ctrl: Control,
    ctrl_for: (usize, usize),
    arrive: [f64; 2],
    sig_group: u8,
    stop_done: bool,
    committed: bool,
    wait_s: f64,
    stuck_s: f64,
    /// The route cannot be extended (it ends at a terminal or leaves).
    final_route: bool,
    lunatic: bool,
    length: f64,
    /// T4: the body's width, the driver's manners, the lateral shift and
    /// the pass in progress.
    width: f64,
    manner: Manner,
    shift: f64,
    shift_to: f64,
    shift_rate: f64,
    pass: Option<PassState>,
    /// Seconds waited behind a rider (for the honk), and the next honk.
    rider_wait_s: f64,
    honk_at_s: f64,
}

/// The phase of a pass (T4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Pulling out behind the block.
    Out,
    /// Beside the block at the passing distance.
    Along,
    /// Past it: moving back in.
    Back,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct PassState {
    phase: Phase,
    /// The lateral distance kept (m) and the road's legal one.
    pass_m: f64,
    /// Into the oncoming half.
    oncoming: bool,
}

/// A rider as one vehicle sees it along its route (T4).
#[derive(Debug, Clone, Copy)]
struct RiderRel {
    /// Index into the road users.
    k: u32,
    /// Riding the vehicle's way.
    same: bool,
    /// The rider's extent along the vehicle's route, from the vehicle's
    /// front (m; negative = behind it).
    lo: f64,
    hi: f64,
    /// The rider's centre in the vehicle's kerb coordinates (m from the
    /// centreline towards the vehicle's kerb side).
    kr: f64,
    /// The rider's speed along the vehicle's direction (negative: oncoming).
    v: f64,
}

/// A vehicle and a rider near each other (T4): what the passing record,
/// the contact and the proximity cue need between ticks.
#[derive(Debug, Clone, Copy)]
struct Pair {
    seen: Tick,
    alongside: bool,
    min_clear: f64,
    legal: f64,
    oncoming: bool,
    rel_speed: f64,
    cued: bool,
    contact: bool,
    prev_lo: f64,
    prev_clear: f64,
    /// The rider came up beside the vehicle from behind (the rider
    /// passing a queue): no overtake to record until they part.
    rider_passing: bool,
}

/// What the rider logic decided for a vehicle this tick (T4).
#[derive(Debug, Clone, Copy, PartialEq)]
struct RiderMove {
    pass: Option<PassState>,
    shift_to: f64,
    shift_rate: f64,
    /// Waiting behind this rider (for the honk).
    wait: Option<u32>,
}

/// One entry of the lane order: a vehicle (or rider) in a lane.
#[derive(Debug, Clone, Copy)]
struct Slot {
    key: u64,
    s: f64,
    /// Vehicle index, or `u32::MAX - k` for road user `k`.
    who: u32,
}

/// A vehicle near a junction it is about to cross, or just crossed.
#[derive(Debug, Clone, Copy)]
struct Approach {
    vertex: u32,
    who: u32,
    dist: f64,
    v: f64,
    arrive: [f64; 2],
    /// The arrival (edge, forward) as a key.
    from: u64,
    level: u8,
    waiting: bool,
    wait_s: f64,
    in_box: bool,
    id: u32,
    /// A rider (T4: vehicles always yield to riders).
    rider: bool,
}

/// A place where vehicles come in (the rim of a bubble, the region's
/// boundary, a dead end).
#[derive(Debug, Clone, Copy)]
struct Entry {
    edge: u32,
    forward: bool,
    /// Where vehicles appear (m along the direction): just outside the rim.
    s: f64,
    /// A dead end or the boundary inside a bubble (half the flow: an
    /// origin, not a through road).
    origin: bool,
    /// Vehicles per second now.
    rate: f64,
}

/// The road traffic of a session.
pub struct TrafficSim {
    cfg: TrafficConfig,
    source: Box<dyn EdgeRules + Send + Sync>,
    rules: Vec<EdgeRule>,
    /// The bounding box of each edge (min x, min z, max x, max z).
    bounds: Vec<[f64; 4]>,
    curves: Curves,
    fingerprint: u64,
    tick: Tick,
    next_id: u32,
    vehicles: Vec<Vehicle>,
    events: Vec<TrafficEvent>,
    bubbles: Vec<Bubble>,
    active: Vec<u32>,
    filled: Vec<bool>,
    any_filled: bool,
    need_fill: bool,
    keep_left: bool,
    entries: Vec<Entry>,
    rates_at: Option<DemandTime>,
    stats: TrafficStats,
    slots: Vec<Slot>,
    slot_of: Vec<usize>,
    approaches: Vec<Approach>,
    users: Vec<RoadUser>,
    /// T4: the passing rules per edge, the riders by edge (edge, user
    /// index), each vehicle's riders (ranges into `rels`), the pairs, the
    /// events for the riders and this tick's gusts.
    passing: Vec<PassRule>,
    users_by_edge: Vec<(u32, u32)>,
    rels: Vec<RiderRel>,
    rel_at: Vec<(u32, u32)>,
    pairs: std::collections::BTreeMap<(u32, u32), Pair>,
    rider_events: Vec<RiderEvent>,
    gusts: Vec<Gust>,
    /// Each vehicle's lane centre in kerb coordinates at this tick.
    lane_k: Vec<f64>,
}

/// Lane key of (edge, direction, lane).
fn lane_key(edge: u32, forward: bool, lane: u8) -> u64 {
    (u64::from(edge) << 9) | (u64::from(forward) << 8) | u64::from(lane)
}

fn dir_key(edge: u32, forward: bool) -> u64 {
    u64::from(edge) * 2 + u64::from(forward)
}

impl TrafficSim {
    /// A simulation on `net` with the rules of `source`.
    pub fn new(
        net: &RoadNetwork,
        source: Box<dyn EdgeRules + Send + Sync>,
        cfg: TrafficConfig,
    ) -> Self {
        let n = net.graph().edges.len();
        let rules: Vec<EdgeRule> = (0..n).map(|e| source.edge_rule(net, e)).collect();
        let passing: Vec<PassRule> = (0..n).map(|e| source.passing(net, e)).collect();
        let bounds = net
            .graph()
            .edges
            .iter()
            .map(|e| {
                e.points.iter().fold(
                    [
                        f64::INFINITY,
                        f64::INFINITY,
                        f64::NEG_INFINITY,
                        f64::NEG_INFINITY,
                    ],
                    |b, p| {
                        [
                            b[0].min(p[0]),
                            b[1].min(p[1]),
                            b[2].max(p[0]),
                            b[3].max(p[1]),
                        ]
                    },
                )
            })
            .collect();
        Self {
            cfg,
            source,
            rules,
            bounds,
            curves: Curves::new(net),
            fingerprint: net.fingerprint(),
            tick: 0,
            next_id: 1,
            vehicles: Vec::new(),
            events: Vec::new(),
            bubbles: Vec::new(),
            active: Vec::new(),
            filled: vec![false; n],
            any_filled: false,
            need_fill: false,
            keep_left: net.keep_left,
            entries: Vec::new(),
            rates_at: None,
            stats: TrafficStats::default(),
            slots: Vec::new(),
            slot_of: Vec::new(),
            approaches: Vec::new(),
            users: Vec::new(),
            passing,
            users_by_edge: Vec::new(),
            rels: Vec::new(),
            rel_at: Vec::new(),
            pairs: Default::default(),
            rider_events: Vec::new(),
            gusts: Vec::new(),
            lane_k: Vec::new(),
        }
    }

    pub fn config(&self) -> &TrafficConfig {
        &self.cfg
    }

    /// Changes the level or the manners (a live setting, 18A); vehicles
    /// already driving keep their drivers.
    pub fn set_level(&mut self, level: TrafficLevel) {
        self.cfg.level = level;
        self.rates_at = None;
    }

    pub fn set_manners(&mut self, manners: DriverManners) {
        self.cfg.manners = manners;
    }

    /// `ride.collisions` on or off (12A; a live session setting).
    pub fn set_collisions(&mut self, on: bool) {
        self.cfg.collisions = on;
    }

    /// The passing rules of edge `e` (T4).
    pub fn passing(&self, e: usize) -> &PassRule {
        &self.passing[e]
    }

    /// The events between vehicles and riders since the last call
    /// (passes, contacts, honks, the proximity cue; T4).
    pub fn drain_rider_events(&mut self) -> Vec<RiderEvent> {
        std::mem::take(&mut self.rider_events)
    }

    /// The air of passing vehicles on each rider at this tick (T4: a
    /// force for the physics, one entry per rider that feels one).
    pub fn gusts(&self) -> &[Gust] {
        &self.gusts
    }

    /// The vehicles as solid bodies in the world (12B with vehicles: the
    /// client's prediction, a rider steering into a car).
    pub fn bodies(&self, net: &RoadNetwork) -> Vec<VehicleBody> {
        self.vehicles
            .iter()
            .map(|v| {
                let view = self.view(net, v);
                let half = v.length / 2.0;
                VehicleBody {
                    id: v.f.id,
                    class: v.f.class,
                    pos: [
                        view.xz[0] - view.heading[0] * half,
                        view.xz[1] - view.heading[1] * half,
                    ],
                    forward: view.heading,
                    half_length: half,
                    half_width: v.width / 2.0,
                    velocity: [view.heading[0] * v.v, view.heading[1] * v.v],
                }
            })
            .collect()
    }

    /// The vehicles near `user` as obstacles for autodrive and the bots
    /// (12D, 18B *Autodrive and bots*: they see vehicles as obstacles), in
    /// the rider's path frame: on the rider's edge and the ones it meets,
    /// within `range` metres. Oncoming vehicles have a negative speed.
    pub fn avoid_others(
        &self,
        net: &RoadNetwork,
        user: &RoadUser,
        range: f64,
    ) -> Vec<crate::avoid::Other> {
        let mut out = Vec::new();
        if user.edge >= self.rules.len() {
            return out;
        }
        let len = net.geometry(user.edge).length();
        let side = if self.keep_left { -1.0 } else { 1.0 };
        let me_c = user.s - if user.forward { 1.0 } else { -1.0 } * user.length / 2.0;
        for v in &self.vehicles {
            let st = v.f.route[v.pos.step];
            if st.edge as usize != user.edge {
                continue;
            }
            let front = if st.forward { v.pos.s } else { len - v.pos.s };
            let dir = if st.forward { 1.0 } else { -1.0 };
            let centre = front - dir * v.length / 2.0;
            let udir = if user.forward { 1.0 } else { -1.0 };
            let ahead = (centre - me_c) * udir;
            if ahead.abs() > range {
                continue;
            }
            let r = &self.rules[st.edge as usize];
            let k = lateral_offset(r, net.edge(user.edge).width_m, v.lane) - v.shift;
            // The vehicle's lateral in its own right-hand frame, then the rider's.
            let lat = k
                * side
                * if st.forward == user.forward {
                    1.0
                } else {
                    -1.0
                };
            let same = st.forward == user.forward;
            out.push(crate::avoid::Other {
                ahead_m: ahead,
                lateral_m: lat,
                target_m: lat,
                speed: if same { v.v } else { -v.v },
                half_length: v.length / 2.0,
                half_width: v.width / 2.0,
            });
        }
        out
    }

    pub fn tick(&self) -> Tick {
        self.tick
    }

    pub fn dt(&self) -> f64 {
        f64::from(self.cfg.step_ms) / 1000.0
    }

    /// The simulation time (ms since its start).
    pub fn time_ms(&self) -> u64 {
        self.tick * u64::from(self.cfg.step_ms)
    }

    pub fn stats(&self) -> TrafficStats {
        self.stats
    }

    pub fn len(&self) -> usize {
        self.vehicles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.vehicles.is_empty()
    }

    /// The rules of edge `e`.
    pub fn rule(&self, e: usize) -> &EdgeRule {
        &self.rules[e]
    }

    pub fn rules(&self) -> &[EdgeRule] {
        &self.rules
    }

    pub fn curves(&self) -> &Curves {
        &self.curves
    }

    /// The events since the last call (spawns, changes, despawns in order).
    pub fn drain_events(&mut self) -> Vec<TrafficEvent> {
        std::mem::take(&mut self.events)
    }

    /// Every vehicle as a fresh plan at the current tick (a client that
    /// joins or reconnects, T5): the state the other clients extrapolate,
    /// so the new client's vehicles move like theirs.
    pub fn current_plans(&self) -> Vec<Plan> {
        self.vehicles
            .iter()
            .map(|v| Plan {
                id: v.f.id,
                class: v.f.class,
                look: v.f.look,
                tick: self.tick,
                driver: v.f.driver,
                route: v.f.route[v.f.pos.step..].to_vec(),
                base: v.f.pos.step as u32,
                s: v.f.pos.s as f32,
                v: v.f.v as f32,
                lane: v.f.lane,
                motion: v.f.motion,
                shift: Shift {
                    at: v.f.shift as f32,
                    to: v.f.shift_to as f32,
                    rate: v.f.shift_rate as f32,
                },
            })
            .collect()
    }

    /// Sets a vehicle's speed at once (tests: a driver who brakes hard).
    pub fn set_speed(&mut self, id: u32, v: f64) {
        if let Some(x) = self.vehicles.iter_mut().find(|x| x.f.id == id) {
            x.v = v.max(0.0);
        }
    }

    /// A hash of the whole state (tick, every vehicle's id, class, step,
    /// position, speed and lane, bit for bit).
    pub fn digest(&self) -> u64 {
        let mut b: Vec<u8> = Vec::with_capacity(16 + self.vehicles.len() * 48);
        b.extend_from_slice(&self.tick.to_le_bytes());
        b.extend_from_slice(&self.next_id.to_le_bytes());
        for v in &self.vehicles {
            b.extend_from_slice(&v.f.id.to_le_bytes());
            b.push(v.f.class.index() as u8);
            b.extend_from_slice(&(v.pos.step as u64).to_le_bytes());
            b.extend_from_slice(&v.pos.s.to_bits().to_le_bytes());
            b.extend_from_slice(&v.v.to_bits().to_le_bytes());
            b.extend_from_slice(&v.a.to_bits().to_le_bytes());
            b.push(v.lane);
            b.extend_from_slice(&v.shift.to_bits().to_le_bytes());
            b.extend_from_slice(&(v.f.route.len() as u32).to_le_bytes());
        }
        ocw_world::graph::fnv1a(&b)
    }

    /// The vehicles as views (positions in the world).
    pub fn views(&self, net: &RoadNetwork) -> Vec<VehicleView> {
        self.vehicles.iter().map(|v| self.view(net, v)).collect()
    }

    fn view(&self, net: &RoadNetwork, v: &Vehicle) -> VehicleView {
        let st = v.f.route[v.pos.step];
        let e = st.edge as usize;
        let g = net.geometry(e);
        let len = g.length();
        let s = if st.forward { v.pos.s } else { len - v.pos.s };
        let p = g.point_at(s);
        let (a, b) = (
            g.point_at((s - 1.0).max(0.0)),
            g.point_at((s + 1.0).min(len)),
        );
        let mut h = ocw_world::network::unit([b[0] - a[0], b[2] - a[2]]);
        if !st.forward {
            h = [-h[0], -h[1]];
        }
        let lateral = (lateral_offset(&self.rules[e], net.edge(e).width_m, v.lane) - v.shift)
            * if net.keep_left { -1.0 } else { 1.0 };
        // Right of the heading (x east, z south): (−h.z, h.x).
        let right = [-h[1], h[0]];
        VehicleView {
            id: v.f.id,
            class: v.f.class,
            look: v.f.look,
            edge: e,
            forward: st.forward,
            s,
            speed: v.v,
            lane: v.lane,
            lateral,
            xz: [p[0] + right[0] * lateral, p[2] + right[1] * lateral],
            heading: h,
        }
    }

    /// Spawns a vehicle on a given route (tests, tools, buses later):
    /// `route` as (edge, forward), `s` metres along the first. The route
    /// is extended by the walk while it stays in a bubble. `None` when
    /// the class may not drive the route.
    pub fn spawn_on(
        &mut self,
        net: &RoadNetwork,
        class: VehicleClass,
        route: &[(usize, bool)],
        s: f64,
        v: f64,
        lane: u8,
    ) -> Option<u32> {
        if route.is_empty() || route.iter().any(|&(e, f)| !self.rules[e].allows(class, f)) {
            return None;
        }
        let id = self.next_id;
        let (driver, factor, lunatic) = self.driver(class, id);
        let mut steps: Vec<Step> = Vec::new();
        for &(e, f) in route {
            let st = routing::make_step(&self.rules, class, e, f, factor);
            routing::link_turn(net, &mut steps, &st);
            steps.push(st);
        }
        self.push_vehicle(net, class, steps, s, v, lane, driver, lunatic);
        Some(id)
    }

    /// The driver of vehicle `id`: parameters, speed factor, lunatic.
    fn driver(&self, class: VehicleClass, id: u32) -> (DriverParams, f64, bool) {
        let t = class.traits();
        let r = |salt: u64| unit_f(key(self.cfg.seed, u64::from(id), salt, 0xD21));
        let lunatic = self.cfg.level == TrafficLevel::Lunatic && r(1) < 0.6;
        if lunatic {
            (
                DriverParams {
                    accel: t.accel * 1.4,
                    decel: t.decel * 1.4,
                    lateral: t.lateral * 1.6,
                    headway: 0.5 + 0.3 * r(2),
                    gap: 1.0,
                },
                1.15 + 0.25 * r(3),
                true,
            )
        } else {
            (
                DriverParams {
                    accel: t.accel * (0.9 + 0.2 * r(4)),
                    decel: t.decel,
                    lateral: t.lateral,
                    headway: 1.0 + 0.8 * r(5),
                    gap: 2.0,
                },
                // Spec 18B: the limit times 0.85 – 1.05.
                0.85 + 0.2 * r(6),
                false,
            )
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn push_vehicle(
        &mut self,
        net: &RoadNetwork,
        class: VehicleClass,
        route: Vec<Step>,
        s: f64,
        v: f64,
        lane: u8,
        driver: DriverParams,
        lunatic: bool,
    ) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        let first = route[0];
        let lane = lane.min(
            self.rules[first.edge as usize]
                .lanes_in(first.forward)
                .max(1)
                - 1,
        );
        let plan = Plan {
            id,
            class,
            look: key(self.cfg.seed, u64::from(id), 0x100C, 0) as u32,
            tick: self.tick,
            driver,
            route,
            base: 0,
            s: s as f32,
            v: v as f32,
            lane,
            motion: Motion::Free,
            shift: Shift::default(),
        };
        let f = PlanFollower::new(&plan);
        let manner = Manner::of(self.cfg.manners, lunatic, |salt| {
            unit_f(key(self.cfg.seed, u64::from(id), salt, 0xA7))
        });
        let mut veh = Vehicle {
            pos: f.pos,
            v: f.v,
            a: 0.0,
            lane,
            interacting: false,
            lead_v: 0.0,
            ctrl: Control::None,
            ctrl_for: (usize::MAX, 0),
            arrive: [1.0, 0.0],
            sig_group: 0,
            stop_done: false,
            committed: false,
            wait_s: 0.0,
            stuck_s: 0.0,
            final_route: false,
            lunatic,
            length: class.traits().length_m,
            width: around::body_size(class).1,
            manner,
            shift: 0.0,
            shift_to: 0.0,
            shift_rate: 0.0,
            pass: None,
            rider_wait_s: 0.0,
            honk_at_s: manner.patience_s,
            f,
        };
        self.refresh_control(net, &mut veh);
        self.vehicles.push(veh);
        self.events.push(TrafficEvent::Spawn(plan));
        self.stats.spawned += 1;
        id
    }

    /// Recomputes the control at the end of the current step when the
    /// step or the route changed.
    fn refresh_control(&self, net: &RoadNetwork, v: &mut Vehicle) {
        let at = (v.pos.step, v.f.route.len());
        if v.ctrl_for == at {
            return;
        }
        let new_step = v.ctrl_for.0 != at.0;
        v.ctrl_for = at;
        let st = v.f.route[v.pos.step];
        let (e, fwd) = (st.edge as usize, st.forward);
        v.arrive = net.arriving_direction(e, fwd, 20.0);
        if new_step {
            v.stop_done = false;
            v.committed = false;
            v.wait_s = 0.0;
        }
        if v.pos.step + 1 >= v.f.route.len() {
            v.ctrl = Control::None;
            return;
        }
        let vertex = net.end_vertex(e, fwd);
        v.ctrl = self
            .source
            .node_control(net, vertex, e, fwd)
            .unwrap_or_else(|| rules::derived_control(net, vertex, e, |x| self.rules[x]));
        if v.ctrl == Control::Signals {
            let axis_edge = net.edges_at(vertex)[0];
            let axis_fwd = net.edge(axis_edge).from == vertex;
            let axis = net.leaving_direction(axis_edge, axis_fwd, 20.0);
            v.sig_group = signals::group(v.arrive, axis);
        }
    }

    /// One fixed step: `bubbles` are the areas around the riders, `users`
    /// the riders on the road, `time` the local time of the place.
    pub fn step(
        &mut self,
        net: &RoadNetwork,
        bubbles: &[Bubble],
        users: &[RoadUser],
        time: &DemandTime,
    ) {
        debug_assert_eq!(net.fingerprint(), self.fingerprint, "another network");
        let dt = self.dt();
        self.users.clear();
        self.users.extend_from_slice(users);
        self.index_users();
        self.build_slots(net);
        self.build_approaches(net);
        self.build_rels(net);
        // T4: passes, contacts, the proximity cue and the gusts at this tick.
        let caps = self.rider_pairs();
        // Accelerations from the state at this tick.
        let accs: Vec<(f64, bool, f64, bool)> = (0..self.vehicles.len())
            .map(|i| self.accel_of(net, i))
            .collect();
        let lane_moves = self.lane_changes(net, &accs);
        let moves: Vec<Option<RiderMove>> = (0..self.vehicles.len())
            .map(|i| self.rider_move(net, i))
            .collect();
        self.tick += 1;
        let mut gone: Vec<usize> = Vec::new();
        let mut vehicles = std::mem::take(&mut self.vehicles);
        for &(i, cap) in &caps {
            vehicles[i].v = vehicles[i].v.min(cap);
        }
        for (i, veh) in vehicles.iter_mut().enumerate() {
            let (a, interacting, lead_v, stop_line) = accs[i];
            if let Some(l) = lane_moves[i] {
                veh.lane = l;
            }
            if let Some(m) = moves[i] {
                if m.pass.is_some() && veh.pass.is_none() {
                    veh.rider_wait_s = 0.0;
                    veh.honk_at_s = veh.manner.patience_s;
                }
                veh.pass = m.pass;
                veh.shift_to = m.shift_to;
                veh.shift_rate = m.shift_rate;
                if let Some(rider) = m.wait {
                    veh.rider_wait_s += dt;
                    if veh.rider_wait_s >= veh.honk_at_s {
                        veh.honk_at_s = veh.rider_wait_s + 1.5 * veh.manner.patience_s;
                        self.rider_events.push(RiderEvent::Honk {
                            tick: self.tick,
                            vehicle: veh.f.id,
                            rider,
                            class: veh.f.class,
                        });
                    }
                }
            }
            veh.shift = plan::shift_step(veh.shift, veh.shift_to, veh.shift_rate, dt);
            let before = veh.pos.step;
            let (v1, ok) = kin::advance(net, &veh.f.route, &mut veh.pos, veh.v, a, dt);
            if ok && veh.pos.step != before && (veh.pass.is_some() || veh.shift != 0.0) {
                // Keep the place across the road when the lane centre of
                // the next edge lies elsewhere (another width).
                let k = |st: Step| {
                    let e = st.edge as usize;
                    lateral_offset(&self.rules[e], net.edge(e).width_m, veh.lane)
                };
                let delta = k(veh.f.route[veh.pos.step]) - k(veh.f.route[before]);
                veh.shift += delta;
                veh.shift_to = f64::from((veh.shift_to + delta) as f32);
            }
            veh.v = v1;
            veh.a = a;
            veh.interacting = interacting;
            veh.lead_v = lead_v;
            if v1 < 0.3 {
                veh.stuck_s += dt;
                if stop_line {
                    veh.wait_s += dt;
                }
            } else {
                veh.stuck_s = 0.0;
            }
            if !ok || veh.stuck_s > STUCK_S {
                gone.push(i);
                continue;
            }
            let st = veh.f.route[veh.pos.step];
            let len = kin::step_len(net, &st);
            let lanes = self.rules[st.edge as usize].lanes_in(st.forward).max(1);
            veh.lane = veh.lane.min(lanes - 1);
            if veh.ctrl == Control::Stop && v1 < 0.3 && stop_line && len - veh.pos.s < 8.0 {
                veh.stop_done = true;
            }
            let extend = self.extend_route(net, veh, bubbles);
            self.refresh_control(net, veh);
            self.track_plan(net, veh, extend, dt);
        }
        self.vehicles = vehicles;
        self.stats.vehicle_ticks += self.vehicles.len() as u64;
        // Far from every bubble (checked once a second, spread over the ticks).
        let per_s = (1000 / u64::from(self.cfg.step_ms.max(1))).max(1);
        for (i, v) in self.vehicles.iter().enumerate() {
            if u64::from(v.f.id) % per_s == self.tick % per_s && !gone.contains(&i) {
                let p = self.view(net, v).xz;
                if !routing::inside(bubbles, p, self.cfg.margin_m + 50.0) {
                    gone.push(i);
                }
            }
        }
        if !gone.is_empty() {
            gone.sort_unstable();
            for &i in gone.iter().rev() {
                let v = self.vehicles.remove(i);
                self.events.push(TrafficEvent::Despawn {
                    id: v.f.id,
                    tick: self.tick,
                });
                self.stats.despawned += 1;
            }
        }
        self.update_bubbles(net, bubbles);
        self.spawn(net, bubbles, time);
    }

    /// Sorts the vehicles into the lane order (riders are seen through
    /// [`Self::build_rels`], T4).
    fn build_slots(&mut self, net: &RoadNetwork) {
        let _ = net;
        self.slots.clear();
        for (i, v) in self.vehicles.iter().enumerate() {
            let st = v.f.route[v.pos.step];
            self.slots.push(Slot {
                key: lane_key(st.edge, st.forward, v.lane),
                s: v.pos.s,
                who: i as u32,
            });
        }
        self.slots.sort_unstable_by(|a, b| {
            a.key
                .cmp(&b.key)
                .then(a.s.total_cmp(&b.s))
                .then(a.who.cmp(&b.who))
        });
        self.slot_of.clear();
        self.slot_of.resize(self.vehicles.len(), usize::MAX);
        for (k, s) in self.slots.iter().enumerate() {
            if (s.who as usize) < self.vehicles.len() {
                self.slot_of[s.who as usize] = k;
            }
        }
    }

    /// Who is about to cross which junction, or is in it.
    fn build_approaches(&mut self, net: &RoadNetwork) {
        self.approaches.clear();
        for (i, v) in self.vehicles.iter().enumerate() {
            let st = v.f.route[v.pos.step];
            let len = kin::step_len(net, &st);
            let dist = len - v.pos.s;
            if v.ctrl != Control::None && dist < 120.0 && v.pos.step + 1 < v.f.route.len() {
                let waiting = v.v < 0.5 && dist < 15.0;
                self.approaches.push(Approach {
                    vertex: net.end_vertex(st.edge as usize, st.forward).0,
                    who: i as u32,
                    dist,
                    v: v.v,
                    arrive: v.arrive,
                    from: dir_key(st.edge, st.forward),
                    level: v.ctrl.level(),
                    waiting,
                    wait_s: v.wait_s,
                    in_box: false,
                    id: v.f.id,
                    rider: false,
                });
            }
            if v.pos.step > 0 && v.pos.s < BOX_M {
                let prev = v.f.route[v.pos.step - 1];
                self.approaches.push(Approach {
                    vertex: net.start_vertex(st.edge as usize, st.forward).0,
                    who: i as u32,
                    dist: 0.0,
                    v: v.v,
                    arrive: [0.0, 0.0],
                    from: dir_key(prev.edge, prev.forward),
                    level: 2,
                    waiting: false,
                    wait_s: 0.0,
                    in_box: true,
                    id: v.f.id,
                    rider: false,
                });
            }
        }
        // Riders arriving at a junction (T4: vehicles always yield to them).
        for (k, u) in self.users.iter().enumerate() {
            if u.edge >= self.rules.len() {
                continue;
            }
            let len = net.geometry(u.edge).length();
            let dist = if u.forward { len - u.s } else { u.s };
            if dist < 70.0 {
                self.approaches.push(Approach {
                    vertex: net.end_vertex(u.edge, u.forward).0,
                    who: u32::MAX - k as u32,
                    dist,
                    v: u.speed,
                    arrive: net.arriving_direction(u.edge, u.forward, 20.0),
                    from: dir_key(u.edge as u32, u.forward),
                    level: 3,
                    waiting: false,
                    wait_s: 0.0,
                    in_box: false,
                    id: 0,
                    rider: true,
                });
            }
        }
        self.approaches
            .sort_unstable_by(|a, b| a.vertex.cmp(&b.vertex).then(a.who.cmp(&b.who)));
    }

    /// The leader of vehicle `i` in its lane (on this step or the next
    /// ones): (gap bumper to bumper, its speed, a rider).
    fn leader(&self, net: &RoadNetwork, i: usize) -> Option<(f64, f64, bool)> {
        let v = &self.vehicles[i];
        let k = self.slot_of[i];
        let me = self.slots[k];
        let slot_len = |s: &Slot| -> (f64, f64, bool) {
            if (s.who as usize) < self.vehicles.len() {
                let o = &self.vehicles[s.who as usize];
                (o.length, o.v, false)
            } else {
                let u = &self.users[(u32::MAX - s.who) as usize];
                (u.length + RIDER_GAP_M, u.speed, true)
            }
        };
        if let Some(next) = self.slots.get(k + 1)
            && next.key == me.key
        {
            let (l, sp, rider) = slot_len(next);
            return Some((next.s - me.s - l, sp, rider));
        }
        let st = v.f.route[v.pos.step];
        let mut base = kin::step_len(net, &st) - v.pos.s;
        let mut step = v.pos.step + 1;
        while base < LOOK_AHEAD_M {
            let Some(ns) = v.f.route.get(step) else {
                break;
            };
            let lanes = self.rules[ns.edge as usize].lanes_in(ns.forward).max(1);
            let want = lane_key(ns.edge, ns.forward, v.lane.min(lanes - 1));
            let at = self.slots.partition_point(|s| s.key < want);
            if let Some(s) = self.slots.get(at)
                && s.key == want
                && s.who != i as u32
            {
                let (l, sp, rider) = slot_len(s);
                return Some((base + s.s - l, sp, rider));
            }
            base += kin::step_len(net, ns);
            step += 1;
        }
        None
    }

    /// Whether vehicle `i` must wait at the end of its step (signals,
    /// stop, give way, a full exit).
    fn must_stop(&self, net: &RoadNetwork, i: usize, dist: f64) -> bool {
        let v = &self.vehicles[i];
        if v.committed || v.ctrl == Control::None || dist > JUNCTION_M {
            return false;
        }
        // No room behind the junction: wait before it.
        if let Some(next) = v.f.route.get(v.pos.step + 1) {
            let lanes = self.rules[next.edge as usize].lanes_in(next.forward).max(1);
            let want = lane_key(next.edge, next.forward, v.lane.min(lanes - 1));
            let at = self.slots.partition_point(|s| s.key < want);
            if let Some(s) = self.slots.get(at)
                && s.key == want
                && (s.who as usize) < self.vehicles.len()
            {
                let o = &self.vehicles[s.who as usize];
                if s.s - o.length < v.length + 1.0 && o.v < 2.0 && dist < 30.0 {
                    return true;
                }
            }
        }
        let st = v.f.route[v.pos.step];
        let vertex = net.end_vertex(st.edge as usize, st.forward).0;
        if self.rider_conflict(i, vertex, dist) {
            return true;
        }
        match v.ctrl {
            Control::None | Control::Priority => false,
            Control::Signals => {
                let (light, age) = signals::light(vertex, v.sig_group, self.time_ms());
                let can_stop = dist - 2.0 > v.v * v.v / (2.0 * v.f.driver.decel);
                match light {
                    Light::Green => false,
                    Light::Amber => can_stop && !v.lunatic,
                    // Lunatics run a light that just turned red.
                    Light::Red => !(v.lunatic && age < 2_000 && !can_stop),
                }
            }
            Control::Stop if !v.stop_done => true,
            Control::Stop | Control::GiveWay | Control::RightBeforeLeft => {
                self.conflict(i, vertex, dist)
            }
        }
    }

    /// Whether a vehicle with right of way over `i` comes, or one is in
    /// the junction.
    fn conflict(&self, i: usize, vertex: u32, dist: f64) -> bool {
        let v = &self.vehicles[i];
        let st = v.f.route[v.pos.step];
        let from = dir_key(st.edge, st.forward);
        let level = v.ctrl.level();
        let gap = if v.lunatic { 1.5 } else { 4.0 };
        let eta = dist.max(0.0) / v.v.max(3.0);
        let at = self.approaches.partition_point(|a| a.vertex < vertex);
        let keep_left = self.keep_left;
        for b in self.approaches[at..]
            .iter()
            .take_while(|a| a.vertex == vertex)
        {
            if b.who as usize == i || b.from == from || b.rider {
                continue;
            }
            if b.in_box {
                if b.v > 0.5 || b.dist < 1.0 {
                    return true;
                }
                continue;
            }
            // B comes from the right: its heading is A's turned left.
            let cross = v.arrive[0] * b.arrive[1] - v.arrive[1] * b.arrive[0];
            let from_right = if keep_left { cross > 0.5 } else { cross < -0.5 };
            let prior = b.level > level || (b.level == level && level < 2 && from_right);
            if !prior {
                continue;
            }
            // Deadlock breaker: equals all waiting – the one waiting
            // longest (then the lower id) goes.
            if b.waiting
                && b.level == level
                && v.wait_s > 2.0
                && (v.wait_s > b.wait_s + 0.5
                    || ((v.wait_s - b.wait_s).abs() <= 0.5 && v.f.id < b.id))
            {
                continue;
            }
            let eta_b = b.dist / b.v.max(0.5);
            if eta_b < eta + gap && b.dist < 100.0 {
                return true;
            }
        }
        false
    }

    /// The acceleration of vehicle `i`: (a, interacting, the binding
    /// constraint's speed, waiting at a line).
    fn accel_of(&self, net: &RoadNetwork, i: usize) -> (f64, bool, f64, bool) {
        let v = &self.vehicles[i];
        let p = &v.f.driver;
        let v0 = kin::desired_speed(net, &self.curves, &v.f.route, v.pos, p);
        let mut a = kin::idm(p, v.v, v0, None);
        let mut interacting = false;
        let mut lead_v = v0;
        let mut consider = |gap: f64, sp: f64| {
            let dv = v.v - sp;
            let s_star =
                p.gap + (v.v * p.headway + v.v * dv / (2.0 * (p.accel * p.decel).sqrt())).max(0.0);
            // Too far to matter (the interaction term under 5 %).
            if gap > 4.5 * s_star {
                return;
            }
            let ai = kin::idm(p, v.v, v0, Some((gap, sp)));
            if !interacting || ai < a {
                a = ai;
                lead_v = sp;
            }
            interacting = true;
        };
        if let Some((gap, sp, _rider)) = self.leader(net, i) {
            consider(gap, sp);
        }
        for (gap, sp) in self.rider_leaders(i) {
            consider(gap, sp);
        }
        let st = v.f.route[v.pos.step];
        let dist = kin::step_len(net, &st) - v.pos.s;
        let stop = self.must_stop(net, i, dist);
        if stop {
            consider((dist - 1.0).max(0.0), 0.0);
        }
        (a, interacting, lead_v, stop)
    }

    /// Lane changes on roads with more than one lane a direction, checked
    /// once a second per vehicle: (new lane) per vehicle.
    fn lane_changes(&self, net: &RoadNetwork, accs: &[(f64, bool, f64, bool)]) -> Vec<Option<u8>> {
        let per_s = (1000 / u64::from(self.cfg.step_ms.max(1))).max(1);
        let mut out = vec![None; self.vehicles.len()];
        for (i, v) in self.vehicles.iter().enumerate() {
            if u64::from(v.f.id) % per_s != self.tick % per_s {
                continue;
            }
            let st = v.f.route[v.pos.step];
            let lanes = self.rules[st.edge as usize].lanes_in(st.forward);
            let dist = kin::step_len(net, &st) - v.pos.s;
            if lanes < 2 || dist < 40.0 || v.pos.s < 10.0 || v.pass.is_some() || v.shift != 0.0 {
                continue;
            }
            let p = &v.f.driver;
            let v0 = kin::desired_speed(net, &self.curves, &v.f.route, v.pos, p);
            let a_now = accs[i].0;
            let mut best: Option<(f64, u8)> = None;
            for target in [v.lane.wrapping_sub(1), v.lane + 1] {
                if target >= lanes {
                    continue;
                }
                let want = lane_key(st.edge, st.forward, target);
                let lo = self.slots.partition_point(|s| s.key < want);
                let hi = self.slots.partition_point(|s| s.key <= want);
                let lane = &self.slots[lo..hi];
                let ahead = lane.partition_point(|s| s.s <= v.pos.s);
                let len_of = |s: &Slot| {
                    if (s.who as usize) < self.vehicles.len() {
                        let o = &self.vehicles[s.who as usize];
                        (o.length, o.v, Some(&o.f.driver))
                    } else {
                        let u = &self.users[(u32::MAX - s.who) as usize];
                        (u.length + RIDER_GAP_M, u.speed, None)
                    }
                };
                let a_new = match lane.get(ahead) {
                    Some(s) => {
                        let (l, sp, _) = len_of(s);
                        kin::idm(p, v.v, v0, Some((s.s - v.pos.s - l, sp)))
                    }
                    None => kin::idm(p, v.v, v0, None),
                };
                // The new follower must not brake hard.
                if ahead > 0 {
                    let s = &lane[ahead - 1];
                    let (_, sp, drv) = len_of(s);
                    let gap = v.pos.s - v.length - s.s;
                    if gap < 2.0 {
                        continue;
                    }
                    if let Some(d) = drv {
                        let af = kin::idm(d, sp, sp.max(1.0), Some((gap, v.v)));
                        if af < -3.0 {
                            continue;
                        }
                    }
                }
                if let Some(s) = lane.get(ahead) {
                    let (l, _, _) = len_of(s);
                    if s.s - l - v.pos.s < 2.0 {
                        continue;
                    }
                }
                // Out to pass: a clear gain; back to the kerb: little loss.
                let gain = a_new - a_now;
                let ok = if target < v.lane {
                    gain > -0.2
                } else {
                    gain > 0.5
                };
                if ok && best.is_none_or(|(g, _)| gain > g) {
                    best = Some((gain, target));
                }
            }
            out[i] = best.map(|(_, l)| l);
        }
        out
    }

    /// Extends the route by the walk when the vehicle nears its end and
    /// the route could go on; the new steps.
    fn extend_route(&self, net: &RoadNetwork, v: &mut Vehicle, bubbles: &[Bubble]) -> Vec<Step> {
        if v.final_route || v.pos.step + 2 < v.f.route.len() {
            return Vec::new();
        }
        let last = *v.f.route.last().expect("a route");
        let factor = f64::from(last.speed)
            / self.rules[last.edge as usize]
                .speed
                .min(v.f.class.traits().top_speed)
                .max(1.0);
        let lim = routing::WalkLimits {
            bubbles,
            margin: self.cfg.margin_m,
            max_m: 2_000.0,
            max_steps: 40,
        };
        let seed = key(
            self.cfg.seed,
            u64::from(v.f.id),
            v.f.route.len() as u64,
            0xE7,
        );
        let more = routing::walk(
            net,
            &self.rules,
            v.f.class,
            (last.edge as usize, last.forward),
            factor,
            seed,
            &lim,
        );
        if more.is_empty() {
            v.final_route = true;
            return more;
        }
        // The last step's turn now leads into the first new step; the
        // client gets the corrected step with the extension.
        let n = v.f.route.len();
        routing::link_turn(net, &mut v.f.route, &more[0]);
        let mut ext = vec![v.f.route[n - 1]];
        ext.extend_from_slice(&more);
        v.f.route.extend_from_slice(&more);
        ext
    }

    /// Steps the vehicle's plan follower and sends a change when the
    /// vehicle left it (or its lane or route changed).
    fn track_plan(&mut self, net: &RoadNetwork, v: &mut Vehicle, extend: Vec<Step>, dt: f64) {
        v.f.step(net, &self.curves, dt);
        let off = kin::route_gap(net, &v.f.route, v.f.pos, v.pos);
        let changed = !extend.is_empty()
            || v.lane != v.f.lane
            || (v.shift - v.f.shift).abs() > plan::SHIFT_TOLERANCE_M
            || v.shift_to != v.f.shift_to
            || off.abs() > plan::POS_TOLERANCE_M
            || (v.v - v.f.v).abs() > plan::SPEED_TOLERANCE
            || (v.f.ended && v.v > 0.0);
        if !changed {
            return;
        }
        let v0 = kin::desired_speed(net, &self.curves, &v.f.route, v.pos, &v.f.driver);
        let motion = if !v.interacting {
            Motion::Free
        } else if v.v < 0.05 && v.a <= 0.0 {
            Motion::Stopped
        } else if v.a.abs() < 0.05 {
            Motion::Accel {
                a: 0.0,
                target: v.v as f32,
            }
        } else if v.a < 0.0 {
            let t = if v.lead_v < v.v {
                v.lead_v.max(0.0)
            } else {
                (v.v + v.a * 2.0).max(0.0)
            };
            Motion::Accel {
                a: v.a as f32,
                target: t as f32,
            }
        } else {
            Motion::Accel {
                a: v.a as f32,
                target: (v.v + v.a * 3.0).min(v0) as f32,
            }
        };
        let change = PlanChange {
            id: v.f.id,
            tick: self.tick,
            step: v.pos.step as u32,
            s: v.pos.s as f32,
            v: v.v as f32,
            lane: v.lane,
            motion,
            shift: Shift {
                at: v.shift as f32,
                to: v.shift_to as f32,
                rate: v.shift_rate as f32,
            },
            extend,
        };
        // The follower shares the vehicle's route, which already has the
        // extension: apply only the state.
        v.f.apply(&PlanChange {
            extend: Vec::new(),
            ..change.clone()
        });
        self.events.push(TrafficEvent::Change(change));
        self.stats.changes += 1;
    }

    /// Recomputes the active edges and the entries when the bubbles moved.
    fn update_bubbles(&mut self, net: &RoadNetwork, bubbles: &[Bubble]) {
        let moved = bubbles.len() != self.bubbles.len()
            || bubbles.iter().zip(&self.bubbles).any(|(a, b)| {
                let (dx, dz) = (a.center[0] - b.center[0], a.center[1] - b.center[1]);
                dx * dx + dz * dz > 25.0 * 25.0 || (a.radius - b.radius).abs() > 1.0
            });
        if !moved {
            return;
        }
        self.bubbles = bubbles.to_vec();
        self.active.clear();
        for (e, b) in self.bounds.iter().enumerate() {
            if self.rules[e].access.is_none() {
                continue;
            }
            let hit = bubbles.iter().any(|bb| {
                let cx = bb.center[0].clamp(b[0], b[2]);
                let cz = bb.center[1].clamp(b[1], b[3]);
                let (dx, dz) = (cx - bb.center[0], cz - bb.center[1]);
                dx * dx + dz * dz <= bb.radius * bb.radius
            });
            if hit {
                self.active.push(e as u32);
            }
        }
        self.entries.clear();
        let verts = &net.graph().vertices;
        for &e in &self.active {
            let edge = net.edge(e as usize);
            for forward in [true, false] {
                if self.rules[e as usize].lanes_in(forward) == 0
                    || !self.rules[e as usize].oneway.allows(forward)
                {
                    continue;
                }
                let start = if forward { edge.from } else { edge.to };
                let outside = !routing::inside(bubbles, verts[start.0 as usize], 0.0);
                if outside || routing::terminal(net, start) {
                    // Appear where the edge comes within half the margin.
                    let g = net.geometry(e as usize);
                    let len = g.length();
                    let n = g.points.len();
                    let s = (0..n)
                        .map(|j| if forward { j } else { n - 1 - j })
                        .find(|&j| {
                            let p = g.points[j];
                            routing::inside(bubbles, [p[0], p[2]], self.cfg.margin_m * 0.5)
                        })
                        .map_or(0.0, |j| {
                            let c = g.cumulative[j];
                            if forward { c } else { len - c }
                        });
                    self.entries.push(Entry {
                        edge: e,
                        forward,
                        s: s.min((len - 1.0).max(0.0)),
                        origin: !outside,
                        rate: 0.0,
                    });
                }
            }
        }
        self.rates_at = None;
        self.need_fill = true;
    }

    /// Spawns vehicles at the entries (by the demand of the hour) and
    /// fills newly active edges to their density.
    fn spawn(&mut self, net: &RoadNetwork, bubbles: &[Bubble], time: &DemandTime) {
        if self.cfg.level == TrafficLevel::Off || bubbles.is_empty() {
            return;
        }
        // Rates once a simulated minute (they change slowly).
        let refresh = self
            .rates_at
            .is_none_or(|t| t.seconds / 60 != time.seconds / 60 || t.weekday != time.weekday);
        if refresh {
            self.rates_at = Some(*time);
            for k in 0..self.entries.len() {
                let en = self.entries[k];
                let r = &self.rules[en.edge as usize];
                let two_way = r.lanes_in(!en.forward) > 0;
                let q = demand::flow_per_hour(r.aadt, two_way, time, self.cfg.level);
                let origin = if en.origin { 0.5 } else { 1.0 };
                self.entries[k].rate = q / 3600.0 * origin;
            }
        }
        if self.need_fill {
            self.need_fill = false;
            let all = !self.any_filled;
            self.fill(net, bubbles, time, all);
            self.any_filled = true;
        }
        let dt = self.dt();
        for k in 0..self.entries.len() {
            if self.vehicles.len() >= self.cfg.max_vehicles {
                break;
            }
            let en = self.entries[k];
            let p = en.rate * dt;
            let u = unit_f(key(
                self.cfg.seed,
                self.tick,
                u64::from(en.edge),
                u64::from(en.forward),
            ));
            if u >= p {
                continue;
            }
            // Room at the start of the kerb lane.
            let want = lane_key(en.edge, en.forward, 0);
            let lo = self.slots.partition_point(|s| s.key < want);
            let hi = self.slots.partition_point(|s| s.key <= want);
            if self.slots[lo..hi].iter().any(|s| (s.s - en.s).abs() < 25.0) {
                continue;
            }
            if self.user_near(net, en.edge as usize, en.forward, en.s, 40.0) {
                continue;
            }
            let road = net.edge(en.edge as usize).class;
            let u2 = unit_f(key(self.cfg.seed, self.tick, u64::from(en.edge), 0xC1A55));
            self.spawn_walker(
                net,
                bubbles,
                time,
                road,
                en.edge as usize,
                en.forward,
                en.s,
                u2,
                None,
            );
        }
    }

    /// Fills the active edges not filled yet to their density (`all`: the
    /// first fill; later ones only away from the riders).
    fn fill(&mut self, net: &RoadNetwork, bubbles: &[Bubble], time: &DemandTime, all: bool) {
        for k in 0..self.active.len() {
            let e = self.active[k] as usize;
            if self.filled[e] {
                continue;
            }
            self.filled[e] = true;
            let len = net.geometry(e).length();
            for forward in [true, false] {
                let r = self.rules[e];
                if r.lanes_in(forward) == 0 || !r.oneway.allows(forward) {
                    continue;
                }
                let two_way = r.lanes_in(!forward) > 0;
                let q = demand::flow_per_hour(r.aadt, two_way, time, self.cfg.level) / 3600.0;
                let k_per_m = q / r.speed.max(3.0);
                const SLOT: f64 = 25.0;
                let p = (k_per_m * SLOT).min(0.8);
                let mut s = 5.0;
                let mut n = 0u64;
                while s + 5.0 < len {
                    if self.vehicles.len() >= self.cfg.max_vehicles {
                        return;
                    }
                    let h = key(self.cfg.seed ^ 0xF111, e as u64, u64::from(forward), n);
                    n += 1;
                    if unit_f(h) < p {
                        let at = s + unit_f(splitmix_k(h)) * (SLOT - 8.0);
                        let g = net.geometry(e);
                        let pt = g.point_at(if forward { at } else { len - at });
                        let near_rider = bubbles.iter().any(|b| {
                            let (dx, dz) = (pt[0] - b.center[0], pt[2] - b.center[1]);
                            dx * dx + dz * dz < (0.6 * b.radius) * (0.6 * b.radius)
                        });
                        if routing::inside(bubbles, [pt[0], pt[2]], 0.0)
                            && (all || !near_rider)
                            && at < len - 2.0
                            && !self.user_near(net, e, forward, at, 30.0)
                        {
                            let road = net.edge(e).class;
                            self.spawn_walker(
                                net,
                                bubbles,
                                time,
                                road,
                                e,
                                forward,
                                at,
                                unit_f(splitmix_k(h ^ 7)),
                                Some(r.speed * 0.8),
                            );
                        }
                    }
                    s += SLOT;
                }
            }
        }
    }

    /// Spawns a vehicle of a class picked by the mix of `road` on (edge,
    /// forward) at `s`, with a walked route.
    #[allow(clippy::too_many_arguments)]
    fn spawn_walker(
        &mut self,
        net: &RoadNetwork,
        bubbles: &[Bubble],
        time: &DemandTime,
        road: RoadClass,
        edge: usize,
        forward: bool,
        s: f64,
        u: f64,
        v: Option<f64>,
    ) {
        let mut mix = demand::class_mix(road, time);
        for (i, m) in mix.iter_mut().enumerate() {
            if !VehicleClass::from_index(i).is_some_and(|c| self.rules[edge].allows(c, forward)) {
                *m = 0.0;
            }
        }
        let sum: f64 = mix.iter().sum();
        if sum <= 0.0 {
            return;
        }
        for m in &mut mix {
            *m /= sum;
        }
        let Some(class) = demand::pick_class(&mix, u) else {
            return;
        };
        let id = self.next_id;
        let (driver, factor, lunatic) = self.driver(class, id);
        let first = routing::make_step(&self.rules, class, edge, forward, factor);
        let lim = routing::WalkLimits {
            bubbles,
            margin: self.cfg.margin_m,
            max_m: 3_000.0,
            max_steps: 60,
        };
        let more = routing::walk(
            net,
            &self.rules,
            class,
            (edge, forward),
            factor,
            key(self.cfg.seed, u64::from(id), 0xA1, 0),
            &lim,
        );
        let mut route = vec![first];
        if let Some(m) = more.first() {
            routing::link_turn(net, &mut route, m);
        }
        route.extend(more);
        let v = v
            .unwrap_or(f64::from(first.speed) * 0.8)
            .min(f64::from(first.speed));
        self.push_vehicle(net, class, route, s, v, 0, driver, lunatic);
    }

    // --- T4: around riders -------------------------------------------

    /// The riders sorted by edge.
    fn index_users(&mut self) {
        self.users_by_edge.clear();
        for (k, u) in self.users.iter().enumerate() {
            if u.edge < self.rules.len() {
                self.users_by_edge.push((u.edge as u32, k as u32));
            }
        }
        self.users_by_edge.sort_unstable();
    }

    /// Whether a rider is within `within` metres of `s` on (edge, forward).
    fn user_near(
        &self,
        net: &RoadNetwork,
        edge: usize,
        forward: bool,
        s: f64,
        within: f64,
    ) -> bool {
        let at = self.users_by_edge.partition_point(|x| x.0 < edge as u32);
        let len = net.geometry(edge).length();
        self.users_by_edge[at..]
            .iter()
            .take_while(|x| x.0 == edge as u32)
            .any(|&(_, k)| {
                let u = &self.users[k as usize];
                let a = if forward { u.s } else { len - u.s };
                (a - s).abs() < within
            })
    }

    /// Each vehicle's riders along its route: on the step before (its
    /// rear), the current one and those ahead within the look-ahead; and
    /// each vehicle's lane centre in kerb coordinates (m from the
    /// centreline towards its kerb side).
    fn build_rels(&mut self, net: &RoadNetwork) {
        let mut rels = std::mem::take(&mut self.rels);
        rels.clear();
        self.rel_at.clear();
        self.lane_k.clear();
        let side = if self.keep_left { -1.0 } else { 1.0 };
        for v in &self.vehicles {
            let e = v.f.route[v.pos.step].edge as usize;
            self.lane_k
                .push(lateral_offset(&self.rules[e], net.edge(e).width_m, v.lane));
            let start = rels.len() as u32;
            if !self.users_by_edge.is_empty() {
                let mut k = v.pos.step.saturating_sub(1);
                let mut base = -v.pos.s;
                if k < v.pos.step {
                    base -= kin::step_len(net, &v.f.route[k]);
                }
                while let Some(st) = v.f.route.get(k) {
                    if base > LOOK_AHEAD_M {
                        break;
                    }
                    let len = kin::step_len(net, st);
                    let at = self.users_by_edge.partition_point(|x| x.0 < st.edge);
                    for &(e, uk) in &self.users_by_edge[at..] {
                        if e != st.edge {
                            break;
                        }
                        let u = &self.users[uk as usize];
                        let a = if st.forward { u.s } else { len - u.s };
                        let same = u.forward == st.forward;
                        let (lo, hi) = if same {
                            (base + a - u.length, base + a)
                        } else {
                            (base + a, base + a + u.length)
                        };
                        if hi < -v.length - 15.0 || lo > LOOK_AHEAD_M {
                            continue;
                        }
                        let lat = if same { u.lateral } else { -u.lateral };
                        rels.push(RiderRel {
                            k: uk,
                            same,
                            lo,
                            hi,
                            kr: lat * side,
                            v: if same { u.speed } else { -u.speed },
                        });
                    }
                    base += len;
                    k += 1;
                }
            }
            self.rel_at.push((start, rels.len() as u32));
        }
        self.rels = rels;
    }

    fn rels_of(&self, i: usize) -> &[RiderRel] {
        let (a, b) = self.rel_at[i];
        &self.rels[a as usize..b as usize]
    }

    /// The riders vehicle `i` must not drive into this tick: (gap for the
    /// IDM, speed).
    fn rider_leaders(&self, i: usize) -> Vec<(f64, f64)> {
        let mut out = Vec::new();
        let rels = self.rels_of(i);
        if rels.is_empty() {
            return out;
        }
        let v = &self.vehicles[i];
        let e = v.f.route[v.pos.step].edge as usize;
        let w2 = v.width / 2.0;
        let d = v.manner.distance(self.passing[e].distance_m);
        let k_lane = self.lane_k[i];
        let (k_now, k_to) = (k_lane - v.shift, k_lane - v.shift_to);
        let clear = |r: &RiderRel, k: f64| (r.kr - k).abs() - w2 - around::RIDER_HALF_W;
        for r in rels {
            if r.same {
                if r.hi <= -v.length {
                    continue;
                }
                if r.lo > 0.0 {
                    let (blocks, gap) = match v.pass.map(|p| p.phase) {
                        // Not passing: where the vehicle drives back in.
                        None => (clear(r, k_lane) < d - SHIFT_SLACK_M, r.lo - RIDER_GAP_M),
                        Some(Phase::Along) => {
                            (clear(r, k_now) < d - SHIFT_SLACK_M, r.lo - RIDER_GAP_M)
                        }
                        Some(Phase::Out) => (clear(r, k_now) < d - SHIFT_SLACK_M, r.lo - OUT_GAP_M),
                        Some(Phase::Back) => {
                            (clear(r, k_to) < d - SHIFT_SLACK_M, r.lo - RIDER_GAP_M)
                        }
                    };
                    if blocks {
                        out.push((gap, r.v));
                    }
                } else if self.cfg.collisions && clear(r, k_now) < 0.15 {
                    // Beside and touching: brake as hard as it can.
                    out.push((0.1, r.v));
                }
            } else if r.lo > 0.0 && clear(r, k_now) < 0.3 {
                // A rider in the vehicle's lane coming towards it; while
                // passing, the pass logic finishes or gives up first and
                // the brakes come only 3 s before meeting.
                let passing = matches!(v.pass, Some(p) if p.phase != Phase::Back);
                // Whether it is back in its lane before they meet: past the
                // riders beside it, then the move back.
                let (mut front, mut vb) = (f64::NEG_INFINITY, 0.0_f64);
                for q in rels {
                    if q.same && q.hi > -v.length && clear(q, k_lane) < d {
                        front = front.max(q.hi);
                        vb = vb.max(q.v);
                    }
                }
                let back_in = (front + v.length + 0.5).max(0.0) / (v.v - vb).max(0.3)
                    + v.shift.abs() / v.manner.shift_rate.max(0.5);
                let meet = r.lo / (v.v - r.v).max(0.5);
                let makes_it = passing && back_in + 0.5 < meet;
                if !makes_it && (!passing || r.lo < 3.0 * (v.v - r.v)) {
                    out.push(((r.lo - 2.0).max(0.1), r.v));
                }
            }
        }
        out
    }

    /// Whether a rider arrives at `vertex` so that vehicle `i` must wait
    /// (18B: vehicles always yield to riders, wherever they come from).
    /// A vehicle that can no longer stop goes on (Q4).
    fn rider_conflict(&self, i: usize, vertex: u32, dist: f64) -> bool {
        let v = &self.vehicles[i];
        if self.users.is_empty() || v.ctrl == Control::None {
            return false;
        }
        let can_stop = dist - 1.0 > v.v * v.v / (2.0 * kin::MAX_BRAKE * 0.6);
        if !can_stop {
            return false;
        }
        let st = v.f.route[v.pos.step];
        let from = dir_key(st.edge, st.forward);
        let eta = dist.max(0.0) / v.v.max(3.0);
        let at = self.approaches.partition_point(|a| a.vertex < vertex);
        self.approaches[at..]
            .iter()
            .take_while(|a| a.vertex == vertex)
            .filter(|b| b.rider && b.from != from)
            .any(|b| b.dist < 60.0 && b.dist / b.v.max(1.0) < eta + RIDER_YIELD_S)
    }

    /// The route ahead of a vehicle up to `reach` m: (distance from its
    /// front to the step's start, step).
    fn ahead_steps(&self, net: &RoadNetwork, v: &Vehicle, reach: f64) -> Vec<(f64, Step)> {
        let mut out = Vec::new();
        let mut base = -v.pos.s;
        let mut k = v.pos.step;
        while let Some(st) = v.f.route.get(k) {
            if base > reach {
                break;
            }
            out.push((base, *st));
            base += kin::step_len(net, st);
            k += 1;
        }
        out
    }

    /// A junction (three or more roads) within `dist` m ahead.
    fn junction_within(&self, net: &RoadNetwork, v: &Vehicle, dist: f64) -> bool {
        self.ahead_steps(net, v, dist).iter().any(|&(base, st)| {
            let end = base + kin::step_len(net, &st);
            end <= dist
                && net
                    .edges_at(net.end_vertex(st.edge as usize, st.forward))
                    .len()
                    > 2
        })
    }

    /// Oncoming vehicles and riders within `reach` m ahead: (distance
    /// front to front, speed).
    fn oncoming(&self, net: &RoadNetwork, v: &Vehicle, reach: f64) -> Vec<(f64, f64)> {
        let mut out = Vec::new();
        for (base, st) in self.ahead_steps(net, v, reach) {
            let len = kin::step_len(net, &st);
            let back = !st.forward;
            for lane in 0..self.rules[st.edge as usize].lanes_in(back) {
                let want = lane_key(st.edge, back, lane);
                let lo = self.slots.partition_point(|s| s.key < want);
                for s in self.slots[lo..].iter().take_while(|s| s.key == want) {
                    let x = base + (len - s.s);
                    if x > 0.0 && x < reach {
                        out.push((x, self.vehicles[s.who as usize].v));
                    }
                }
            }
            // Riders coming the other way (wherever across the road).
            let at = self.users_by_edge.partition_point(|x| x.0 < st.edge);
            for &(e, k) in &self.users_by_edge[at..] {
                if e != st.edge {
                    break;
                }
                let u = &self.users[k as usize];
                if u.forward != st.forward {
                    let x = base + if st.forward { u.s } else { len - u.s };
                    if x > 0.0 && x < reach {
                        out.push((x, u.speed));
                    }
                }
            }
        }
        out
    }

    /// Whether the oncoming half is free for a pass (vehicles and riders
    /// coming, and vehicles of the same way already out to pass).
    fn oncoming_clear(
        &self,
        net: &RoadNetwork,
        i: usize,
        plan: &around::PassPlan,
        v_pass: f64,
        k_need: f64,
        pass_m: f64,
    ) -> bool {
        let v = &self.vehicles[i];
        let m = &v.manner;
        let need = |vo: f64| plan.travel_m + vo * plan.time_s + m.margin_s * (v_pass + vo);
        if self
            .oncoming(net, v, around::MAX_SIGHT_M)
            .iter()
            .any(|&(x, vo)| x < m.judge * need(vo))
        {
            return false;
        }
        let w2 = v.width / 2.0;
        for r in self.rels_of(i) {
            if !r.same
                && r.lo > 0.0
                && (r.kr - k_need).abs() - w2 - around::RIDER_HALF_W < pass_m
                && r.lo < m.judge * need(-r.v)
            {
                return false;
            }
        }
        // Someone of this direction already out there ahead.
        for (base, st) in self.ahead_steps(net, v, plan.travel_m) {
            let lanes = self.rules[st.edge as usize].lanes_in(st.forward);
            for lane in 0..lanes {
                let want = lane_key(st.edge, st.forward, lane);
                let lo = self.slots.partition_point(|s| s.key < want);
                for s in self.slots[lo..].iter().take_while(|s| s.key == want) {
                    let o = &self.vehicles[s.who as usize];
                    if s.who as usize != i && base + s.s > 0.0 && o.pass.is_some() {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// What vehicle `i` does about the riders this tick: wait, pull out to
    /// pass (the legal distance, a free oncoming lane, the sight distance,
    /// no junction – or less, by its manners), pass, move back in, abort,
    /// or swerve from a rider in its lane.
    fn rider_move(&self, net: &RoadNetwork, i: usize) -> Option<RiderMove> {
        let v = &self.vehicles[i];
        let rels = self.rels_of(i);
        if rels.is_empty() && v.pass.is_none() && v.shift_to == 0.0 {
            return None;
        }
        let st = v.f.route[v.pos.step];
        let e = st.edge as usize;
        let rule = &self.rules[e];
        let pr = self.passing[e];
        let half = net.edge(e).width_m / 2.0;
        let two_way = rule.lanes_in(!st.forward) > 0;
        let k_lane = self.lane_k[i];
        let w2 = v.width / 2.0;
        // The strictest legal distance over the longest pass ahead (a pass
        // from a town into the open road keeps the open road's distance).
        let legal = self
            .ahead_steps(net, v, around::MAX_SIGHT_M)
            .iter()
            .map(|(_, st)| self.passing[st.edge as usize].distance_m)
            .fold(pr.distance_m, f64::max);
        let d = v.manner.distance(legal);
        let rhw = around::RIDER_HALF_W;
        let mut m = RiderMove {
            pass: v.pass,
            shift_to: v.shift_to,
            shift_rate: v.shift_rate,
            wait: None,
        };
        let k_floor = -half + around::EDGE_KEEP_M + w2;
        // A rider in the lane coming towards it: over to the kerb.
        let k_now = k_lane - v.shift;
        let swerve = rels
            .iter()
            .any(|r| !r.same && r.lo > 0.0 && r.lo < 60.0 && (r.kr - k_now).abs() - w2 - rhw < 0.3);
        if swerve && v.pass.is_none() {
            m.shift_to = (k_lane - (half - around::EDGE_KEEP_M - w2)).min(0.0);
            m.shift_rate = 1.5;
            return Some(m);
        }
        let blocks_lane = |r: &RiderRel| (r.kr - k_lane).abs() - w2 - rhw < d;
        let mut ahead: Vec<&RiderRel> = rels
            .iter()
            .filter(|r| r.same && r.hi > -v.length - 30.0)
            .collect();
        ahead.sort_by(|a, b| a.lo.total_cmp(&b.lo).then(a.k.cmp(&b.k)));
        let cut_of = |vb: f64| v.manner.cut_in_m + v.manner.cut_in_s * vb;
        match v.pass {
            None => {
                // Back in once behind the riders it gave up passing.
                let close_ahead = v.shift > 0.0
                    && ahead
                        .iter()
                        .any(|r| r.hi > -v.length && r.lo < 4.0 && blocks_lane(r));
                if v.shift_to != 0.0 && !close_ahead {
                    m.shift_to = 0.0;
                    m.shift_rate = 1.0;
                }
                let Some(fi) = ahead.iter().position(|r| r.lo > 0.0 && blocks_lane(r)) else {
                    return Some(m);
                };
                let first = ahead[fi];
                if first.lo > 80.0_f64.max(v.v * 5.0) {
                    return Some(m);
                }
                if first.lo < RIDER_GAP_M + 15.0 && v.v < first.v + 1.5 {
                    m.wait = Some(self.users[first.k as usize].id);
                }
                // Decisions twice a second (spread over the ticks).
                let per = (500 / u64::from(self.cfg.step_ms.max(1))).max(1);
                if u64::from(v.f.id) % per != self.tick % per {
                    return Some(m);
                }
                let lead = self.leader(net, i);
                // Only the first in the queue.
                if let Some((gap, _, _)) = lead
                    && gap < first.lo
                {
                    return Some(m);
                }
                // The block: riders closer together than the room to cut in.
                let cut = cut_of(first.v);
                let join = v.length + 2.0 * cut + around::BUNCH_EXTRA_M;
                let (mut blk_hi, mut kmin, mut vmax) = (first.hi, first.kr - rhw, first.v);
                for r in &ahead[fi + 1..] {
                    if r.lo - blk_hi < join {
                        blk_hi = blk_hi.max(r.hi);
                        kmin = kmin.min(r.kr - rhw);
                        vmax = vmax.max(r.v);
                    } else {
                        break;
                    }
                }
                // A vehicle ahead of the block must leave room to cut in.
                if let Some((gap, _, _)) = lead
                    && gap < blk_hi + v.length + cut + 5.0
                {
                    return Some(m);
                }
                let mut pass_m = d;
                let mut k_need = kmin - pass_m - w2;
                if k_need < k_floor {
                    let room = kmin - w2 - k_floor;
                    let floor = if v.manner.lunatic {
                        0.2
                    } else if v.manner.pass_m.is_some() {
                        around::PASS_FLOOR_M
                    } else {
                        // Polite: the road is too narrow – wait.
                        return Some(m);
                    };
                    if room < floor {
                        return Some(m);
                    }
                    pass_m = room;
                    k_need = k_floor;
                }
                let shift_need = k_lane - k_need;
                if shift_need <= 0.0 {
                    return Some(m);
                }
                let into = two_way && k_need - w2 < 0.0;
                let blind = v.manner.blind;
                if into && !blind && !pr.may_pass(st.forward) {
                    return Some(m);
                }
                let rate = v.manner.shift_rate;
                // The pass speed: no more than the slowest road of the
                // manoeuvre (a pass into a town is slower).
                let v_pass = self
                    .ahead_steps(net, v, around::MAX_SIGHT_M)
                    .iter()
                    .map(|(_, st)| f64::from(st.speed))
                    .fold(
                        kin::desired_speed(net, &self.curves, &v.f.route, v.pos, &v.f.driver),
                        f64::min,
                    );
                // The time pulling out and moving back in counts too.
                let Some(plan) = around::pass_plan(
                    v.v,
                    v_pass,
                    vmax,
                    v.f.driver.accel,
                    blk_hi + v.length + cut,
                    2.0 * shift_need / rate,
                ) else {
                    return Some(m);
                };
                if !blind && plan.travel_m > around::MAX_SIGHT_M {
                    return Some(m);
                }
                if into {
                    if !self.oncoming_clear(net, i, &plan, v_pass, k_need, pass_m) {
                        return Some(m);
                    }
                    if !blind {
                        if self.junction_within(net, v, plan.travel_m) {
                            return Some(m);
                        }
                        let v_onc = rule.speed;
                        let need = plan.travel_m
                            + v_onc * plan.time_s
                            + v.manner.margin_s * (v_pass + v_onc);
                        let sight = around::sight_distance(
                            net,
                            &v.f.route,
                            v.pos,
                            need,
                            half + around::VERGE_M,
                        );
                        if sight < need {
                            return Some(m);
                        }
                    }
                }
                m.pass = Some(PassState {
                    phase: Phase::Out,
                    pass_m,
                    oncoming: into,
                });
                // A little more than needed (the f32 of the plans).
                m.shift_to = shift_need + 2.0 * SHIFT_SLACK_M;
                m.shift_rate = rate;
                m.wait = None;
            }
            Some(p) if p.phase == Phase::Back => {
                if v.shift.abs() < 0.05 {
                    m.pass = None;
                }
            }
            Some(mut p) => {
                let vb = ahead.iter().map(|r| r.v).fold(0.0, f64::max);
                let cut = cut_of(vb);
                let still: Vec<&&RiderRel> = ahead
                    .iter()
                    .filter(|r| blocks_lane(r) && r.hi > -v.length - cut)
                    .collect();
                if still.is_empty() {
                    p.phase = Phase::Back;
                    m.pass = Some(p);
                    m.shift_to = 0.0;
                    m.shift_rate = v.manner.shift_rate;
                    return Some(m);
                }
                if p.phase == Phase::Out && (v.shift - v.shift_to).abs() < 0.05 {
                    p.phase = Phase::Along;
                }
                // A rider farther out joined: pull out further.
                let kmin = still
                    .iter()
                    .map(|r| r.kr - rhw)
                    .fold(f64::INFINITY, f64::min);
                let keep = p.pass_m.max(v.manner.distance(pr.distance_m));
                let more = k_lane - (kmin - keep - w2).max(k_floor) + 2.0 * SHIFT_SLACK_M;
                if more > m.shift_to {
                    m.shift_to = more;
                }
                m.pass = Some(p);
                if p.oncoming {
                    let mut near: Option<(f64, f64)> = None;
                    for (x, vo) in self.oncoming(net, v, 400.0) {
                        if near.is_none_or(|(nx, _)| x < nx) {
                            near = Some((x, vo));
                        }
                    }
                    for r in rels {
                        if !r.same && r.lo > 0.0 && near.is_none_or(|(x, _)| r.lo < x) {
                            near = Some((r.lo, -r.v));
                        }
                    }
                    if let Some((x, vo)) = near {
                        let ttc = x / (v.v + vo).max(1.0);
                        let behind = still.iter().all(|r| r.lo > 0.0);
                        let rest = still.iter().map(|r| r.hi).fold(0.0, f64::max) + v.length + cut;
                        let t_rest = rest / (v.v - vb).max(0.5);
                        if v.manner.lunatic {
                            if ttc < 2.0 {
                                // Cuts back in, beside the rider or not.
                                m.pass = None;
                                m.shift_to = 0.0;
                                m.shift_rate = 3.0;
                            }
                        } else if !behind
                            && ttc < t_rest + v.manner.margin_s
                            && still.iter().all(|r| r.hi < -v.length - 0.5)
                        {
                            // Past the riders but not by the cut-in gap:
                            // moves back in now, with a short gap.
                            p.phase = Phase::Back;
                            m.pass = Some(p);
                            m.shift_to = 0.0;
                            m.shift_rate = v.manner.shift_rate;
                        } else if behind && ttc < t_rest + v.manner.margin_s {
                            // Gives up only while it can still drop back
                            // behind the riders; else it completes the pass.
                            let lo = still.iter().map(|r| r.lo).fold(f64::INFINITY, f64::min);
                            let dv = (v.v - vb).max(0.0);
                            if dv * dv <= 2.0 * v.f.driver.decel * (lo - 3.0).max(0.0) {
                                m.pass = None;
                            }
                        }
                    }
                }
            }
        }
        Some(m)
    }

    /// Passes, contacts, the proximity cue and the gusts at this tick
    /// (T4); the speed caps of vehicles that hit a rider.
    fn rider_pairs(&mut self) -> Vec<(usize, f64)> {
        self.gusts.clear();
        let mut caps = Vec::new();
        if self.users.is_empty() {
            self.pairs.clear();
            return caps;
        }
        let tick = self.tick;
        let dt = self.dt();
        let side = if self.keep_left { -1.0 } else { 1.0 };
        let rhw = around::RIDER_HALF_W;
        // Per rider: lateral, drag, strongest vehicle, its |lateral|.
        let mut acc: Vec<(f64, f64, u32, f64)> = vec![(0.0, 0.0, 0, 0.0); self.users.len()];
        let mut pairs = std::mem::take(&mut self.pairs);
        let mut events = Vec::new();
        for (i, v) in self.vehicles.iter().enumerate() {
            let (a, b) = self.rel_at[i];
            if a == b {
                continue;
            }
            let st = v.f.route[v.pos.step];
            let e = st.edge as usize;
            let two_way = self.rules[e].lanes_in(!st.forward) > 0;
            let kv = self.lane_k[i] - v.shift;
            let (w2, len) = (v.width / 2.0, v.length);
            let legal = self.passing[e].distance_m;
            for r in &self.rels[a as usize..b as usize] {
                let u = &self.users[r.k as usize];
                let clear = (r.kr - kv).abs() - w2 - rhw;
                let overlap = r.lo < 0.0 && r.hi > -len;
                let pair = pairs.entry((v.f.id, u.id)).or_insert(Pair {
                    seen: tick,
                    alongside: false,
                    min_clear: f64::INFINITY,
                    legal,
                    oncoming: false,
                    rel_speed: 0.0,
                    cued: false,
                    contact: false,
                    prev_lo: r.lo,
                    prev_clear: clear,
                    rider_passing: false,
                });
                pair.seen = tick;
                // Away from the vehicle, in the rider's frame (+ = right).
                let away =
                    if r.kr >= kv { 1.0 } else { -1.0 } * side * if r.same { 1.0 } else { -1.0 };
                if r.same {
                    if overlap && !pair.alongside && !pair.rider_passing && pair.prev_lo < -0.6 {
                        // Beside without coming up from behind it.
                        pair.rider_passing = true;
                    }
                    if !overlap {
                        pair.rider_passing = false;
                    }
                    if overlap && !pair.rider_passing {
                        pair.legal = if pair.alongside {
                            pair.legal.max(legal)
                        } else {
                            legal
                        };
                        pair.alongside = true;
                        pair.min_clear = pair.min_clear.min(clear);
                        pair.oncoming |= two_way && kv - w2 < 0.0;
                        pair.rel_speed = v.v - r.v;
                    } else if pair.alongside && r.hi <= -len {
                        events.push(RiderEvent::Passed(PassRecord {
                            tick,
                            vehicle: v.f.id,
                            rider: u.id,
                            class: v.f.class,
                            clearance_m: pair.min_clear,
                            legal_m: pair.legal,
                            rel_speed: pair.rel_speed,
                            lunatic: v.lunatic,
                            oncoming_half: pair.oncoming,
                        }));
                        pair.alongside = false;
                        pair.min_clear = f64::INFINITY;
                        pair.oncoming = false;
                    } else if pair.alongside && r.lo >= 0.0 {
                        // Dropped back behind the rider: no overtake.
                        pair.alongside = false;
                        pair.min_clear = f64::INFINITY;
                        pair.oncoming = false;
                    }
                    let closing = v.v - r.v;
                    if r.lo > 0.0
                        && !pair.cued
                        && closing > 0.5
                        && r.lo <= around::PROXIMITY_M
                        && r.lo / closing <= around::PROXIMITY_S
                    {
                        pair.cued = true;
                        events.push(RiderEvent::Approaching {
                            tick,
                            vehicle: v.f.id,
                            rider: u.id,
                            class: v.f.class,
                            seconds: r.lo / closing,
                            gap_m: r.lo,
                        });
                    }
                }
                if overlap && clear < 0.0 && self.cfg.collisions {
                    if !pair.contact {
                        pair.contact = true;
                        // Apart sideways a tick ago and beside: from the side;
                        // else the front into the rider (or head-on).
                        let sideways = pair.prev_clear >= 0.0 && pair.prev_lo < 0.0;
                        let entering = pair.prev_lo >= -0.6;
                        let kind = match (r.same, entering && !sideways) {
                            (true, true) => ContactSide::Rear,
                            (false, true) => ContactSide::HeadOn,
                            _ => ContactSide::Side,
                        };
                        let along = if r.same { v.v } else { -v.v } - u.speed;
                        let lateral_rate = ((pair.prev_clear - clear) / dt).max(0.0);
                        let (closing_mps, relative) = match kind {
                            ContactSide::Rear => ((v.v - r.v).max(0.0), [along, 0.0]),
                            ContactSide::HeadOn => (v.v + u.speed, [along, 0.0]),
                            ContactSide::Side => (lateral_rate, [along, away * lateral_rate]),
                        };
                        events.push(RiderEvent::Contact(VehicleContact {
                            tick,
                            vehicle: v.f.id,
                            rider: u.id,
                            class: v.f.class,
                            side: kind,
                            closing_mps,
                            relative,
                            push_side: away,
                        }));
                        match kind {
                            ContactSide::Rear => caps.push((i, (r.v - 0.5).max(0.0))),
                            ContactSide::HeadOn => caps.push((i, 0.0)),
                            ContactSide::Side => {}
                        }
                    }
                } else if clear > 0.3 || !overlap {
                    pair.contact = false;
                }
                let rel = v.v - r.v;
                if clear > 0.0 && rel > 0.5 && r.lo < 10.0 && r.hi > -len - 12.0 {
                    let x = (r.lo + r.hi) / 2.0;
                    let (lat, drag) = around::gust(v.f.class, rel, clear, x, len, !r.same);
                    let g = &mut acc[r.k as usize];
                    g.0 += lat * away;
                    g.1 += drag;
                    if lat.abs() > g.3 {
                        g.2 = v.f.id;
                        g.3 = lat.abs();
                    }
                }
                pair.prev_lo = r.lo;
                pair.prev_clear = clear;
            }
        }
        pairs.retain(|_, p| p.seen == tick);
        self.pairs = pairs;
        self.rider_events.extend(events);
        for (k, g) in acc.iter().enumerate() {
            if g.0.abs() > 0.2 || g.1.abs() > 0.2 {
                self.gusts.push(Gust {
                    rider: self.users[k].id,
                    vehicle: g.2,
                    lateral_n: g.0,
                    drag_n: g.1,
                });
            }
        }
        caps
    }

    /// The vehicles' (edge, forward, class) – for the rule tests.
    pub fn placements(&self) -> Vec<(usize, bool, VehicleClass)> {
        self.vehicles
            .iter()
            .map(|v| {
                let st = v.f.route[v.pos.step];
                (st.edge as usize, st.forward, v.f.class)
            })
            .collect()
    }

    /// The vertex at the end of each vehicle's current step and its speed
    /// and distance there (tests: queues).
    pub fn at_junctions(&self, net: &RoadNetwork) -> Vec<(VertexId, f64, f64)> {
        self.vehicles
            .iter()
            .map(|v| {
                let st = v.f.route[v.pos.step];
                (
                    net.end_vertex(st.edge as usize, st.forward),
                    kin::step_len(net, &st) - v.pos.s,
                    v.v,
                )
            })
            .collect()
    }

    /// Removes vehicle `id` (tests; T13's closures).
    pub fn remove(&mut self, id: u32) -> bool {
        if let Some(i) = self.vehicles.iter().position(|v| v.f.id == id) {
            self.vehicles.remove(i);
            self.events.push(TrafficEvent::Despawn {
                id,
                tick: self.tick,
            });
            self.stats.despawned += 1;
            true
        } else {
            false
        }
    }

    /// The driver's manners of vehicle `id` (tests, tools).
    pub fn manner(&self, id: u32) -> Option<Manner> {
        self.vehicles
            .iter()
            .find(|v| v.f.id == id)
            .map(|v| v.manner)
    }

    /// The ids, speeds and accelerations of the vehicles (tests).
    pub fn speeds(&self) -> Vec<(u32, f64)> {
        self.vehicles.iter().map(|v| (v.f.id, v.v)).collect()
    }
}

fn splitmix_k(h: u64) -> u64 {
    ocw_world::network::splitmix(h)
}

/// Metres right of the centreline of lane `lane` (0 = kerb side) in a
/// direction, for right-hand traffic.
pub fn lateral_offset(r: &EdgeRule, width: f64, lane: u8) -> f64 {
    let total = f64::from(r.lanes[0] + r.lanes[1]).max(1.0);
    let lane_w = width / total;
    width * 0.5 - (f64::from(lane) + 0.5) * lane_w
}
