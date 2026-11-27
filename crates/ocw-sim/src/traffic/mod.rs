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
//! - **Riders** are given as [`RoadUser`]s each step: vehicles queue
//!   behind them with a 10 m gap (T4 adds overtaking).
//! - **Determinism:** the same seed, network, rules and inputs give
//!   byte-identical states ([`TrafficSim::digest`]); randomness is hashed
//!   from (seed, tick, edge, …), never drawn in iteration order.

pub mod demand;
pub mod kin;
pub mod package;
pub mod plan;
pub mod routing;
pub mod rules;
pub mod signals;

#[cfg(test)]
mod tests;

use ocw_world::RoadNetwork;
use ocw_world::graph::{RoadClass, VertexId};

pub use demand::{DemandTime, DriverManners, Region, TrafficLevel};
pub use kin::{Curves, DriverParams, RoutePos};
pub use plan::{Motion, Plan, PlanChange, PlanFollower, Step, Tick, TrafficEvent};
pub use rules::{Access, ClassRules, Control, EdgeRule, EdgeRules, VehicleClass};

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

/// Something on the road vehicles must not drive through: a rider (T4
/// adds overtaking; until then vehicles queue behind).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoadUser {
    pub edge: usize,
    /// Metres from the edge's `from` vertex.
    pub s: f64,
    /// Riding from `from` to `to`.
    pub forward: bool,
    pub speed: f64,
    pub length: f64,
}

/// Gap vehicles keep behind a rider (m; spec 18B: they wait at the
/// rider's speed with a 10 m gap).
pub const RIDER_GAP_M: f64 = 10.0;
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
        let lateral = lateral_offset(&self.rules[e], net.edge(e).width_m, v.lane)
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
        };
        let f = PlanFollower::new(&plan);
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
        self.build_slots(net);
        self.build_approaches(net);
        // Accelerations from the state at this tick.
        let accs: Vec<(f64, bool, f64, bool)> = (0..self.vehicles.len())
            .map(|i| self.accel_of(net, i))
            .collect();
        let lane_moves = self.lane_changes(net, &accs);
        self.tick += 1;
        let mut gone: Vec<usize> = Vec::new();
        let mut vehicles = std::mem::take(&mut self.vehicles);
        for (i, veh) in vehicles.iter_mut().enumerate() {
            let (a, interacting, lead_v, stop_line) = accs[i];
            if let Some(l) = lane_moves[i] {
                veh.lane = l;
            }
            let (v1, ok) = kin::advance(net, &veh.f.route, &mut veh.pos, veh.v, a, dt);
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

    /// Sorts the vehicles and road users into the lane order.
    fn build_slots(&mut self, net: &RoadNetwork) {
        self.slots.clear();
        for (i, v) in self.vehicles.iter().enumerate() {
            let st = v.f.route[v.pos.step];
            self.slots.push(Slot {
                key: lane_key(st.edge, st.forward, v.lane),
                s: v.pos.s,
                who: i as u32,
            });
        }
        for (k, u) in self.users.iter().enumerate() {
            if u.edge >= self.rules.len() {
                continue;
            }
            let len = net.geometry(u.edge).length();
            let s = if u.forward { u.s } else { len - u.s };
            // A rider keeps to the kerb side: the kerb lane of the direction.
            self.slots.push(Slot {
                key: lane_key(u.edge as u32, u.forward, 0),
                s,
                who: u32::MAX - k as u32,
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
            if b.who as usize == i || b.from == from {
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
            if lanes < 2 || dist < 40.0 || v.pos.s < 10.0 {
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
