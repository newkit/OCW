//! The package's traffic rules (`TRFC`, T2's [`ocw_world::traffic`]) as
//! the core's [`EdgeRules`] (spec 14 Phase 18, 18E T4 / T5): [`WorldRules`]
//! over T2's `RoadRules` – each edge's motor one-way, lanes, speed limit
//! and access, the signals, stop and give-way signs at the junctions, and
//! (T4) the passing rule: the country and *in town* give the legal
//! passing distance, `overtaking=no` the places where riders may not be
//! passed. Packages without `TRFC` read the rules T2 implies by class and
//! the country of the package's time zone ([`RoadNetwork::road_rules`]).
//!
//! ```ignore
//! let rules = WorldRules::of(&package, &net); // or WorldRules::new(&net)
//! let sim = TrafficSim::new(&net, Box::new(rules), cfg);
//! ```
//!
//! What maps how:
//! - **one-way, lanes, roundabout** as the package has them (a direction
//!   without lanes carries no traffic; an allowed one at least one lane);
//! - **speed**: T2's driving guess at most the limit
//!   (`EdgeRules::target_kmh`) in the edge's main direction (forward if
//!   it may be driven, else backward);
//! - **access** per class: cars and vans `motorcar`, lorries `hgv` (and
//!   not under a `maxweight` below 7.5 t or a `maxheight` below 3.8 m),
//!   buses `bus` / `psv` (not under 3.5 m), motorbikes `motorcycle`,
//!   tractors `agricultural`; on roads (not tracks) also the class's own
//!   limits of [`class_defaults`] (no lorries or buses on service roads,
//!   no tractors on trunk roads);
//! - **daily traffic**: the class's typical value × the edge's `flow`
//!   (T11's counts), a fifth of it where only access is allowed;
//! - **rank** for right of way: the class's;
//! - **controls**: signals and signs at a junction (or along the edge
//!   within [`SIGN_REACH_M`] of it, towards the nearer end) for the
//!   directions they apply to; where any arm of a junction has signals,
//!   every motor arm has them; at a junction with signs the arms without
//!   one have right of way (not at roundabouts). Crossing lights along an
//!   edge are not stopped at (the core knows controls only at junctions);
//! - **passing** ([`PassRule`]): the edge's country, *in town* and
//!   `overtaking=no`.
//!
//! Routes without a road graph (test routes, route files) drive their own
//! road ([`route_network`]).

use std::collections::{BTreeMap, BTreeSet};

use ocw_world::RoadNetwork;
use ocw_world::graph::{RoadClass, VertexId};
use ocw_world::traffic::{self as wt, ControlKind, Country, Vehicle};

use super::around::PassRule;
use super::demand::Region;
use super::rules::{
    Access, ClassRules, Control, EdgeRule, EdgeRules, VehicleClass, class_defaults,
};

/// A sign or a signal this close to a junction counts as at the junction (m).
pub const SIGN_REACH_M: f32 = 30.0;

/// Lorries stay off roads with a `maxweight` below this (t).
const LORRY_MIN_WEIGHT_T: f32 = 7.5;
/// Lorries stay off roads with a `maxheight` below this (m).
const LORRY_HEIGHT_M: f32 = 3.8;
/// Buses stay off roads with a `maxheight` below this (m).
const BUS_HEIGHT_M: f32 = 3.5;

/// The traffic rules of a world package's network, owned (the core keeps
/// them for the session).
#[derive(Debug, Clone)]
pub struct WorldRules {
    /// The package's rules by edge index (the passing rule reads them).
    world: Vec<wt::EdgeRules>,
    rules: Vec<EdgeRule>,
    /// (vertex, edge, arriving forward) → control.
    controls: BTreeMap<(u32, u32, bool), Control>,
    /// Junctions where some arm has a sign or signals.
    controlled: BTreeSet<u32>,
    /// Junctions with signals.
    signalled: BTreeSet<u32>,
    /// The package carried a `TRFC` section.
    from_data: bool,
    country: Country,
}

/// The rules of `net`: its package's ([`WorldRules`]) – `TRFC` or the
/// rules T2 implies by class and country.
pub fn rules_for(net: &RoadNetwork) -> Box<dyn EdgeRules + Send + Sync> {
    Box::new(WorldRules::new(net))
}

/// The rules of `net` by road class alone (T3's default, for comparison).
pub fn class_rules() -> Box<dyn EdgeRules + Send + Sync> {
    Box::new(ClassRules)
}

/// The holiday region of a country (the demand's calendar, 18B).
pub fn region_of(country: Country) -> Region {
    match country {
        Country::De => Region::De,
        Country::Ch => Region::Ch,
        Country::Li => Region::Li,
        Country::Es => Region::Es,
        _ => Region::Other,
    }
}

/// The core's access bits from T2's access and the edge's limits.
fn access_of(r: &wt::EdgeRules, class: RoadClass) -> Access {
    let mut a = Access::NONE;
    let lorry_ok = r.maxweight_t.is_none_or(|t| t >= LORRY_MIN_WEIGHT_T)
        && r.maxheight_m.is_none_or(|h| h >= LORRY_HEIGHT_M);
    let bus_ok = r.maxheight_m.is_none_or(|h| h >= BUS_HEIGHT_M);
    for (c, ok) in [
        (VehicleClass::Car, r.access.allows(Vehicle::Car)),
        (VehicleClass::Van, r.access.allows(Vehicle::Car)),
        (
            VehicleClass::Lorry,
            r.access.allows(Vehicle::Hgv) && lorry_ok,
        ),
        (VehicleClass::Bus, r.access.allows(Vehicle::Bus) && bus_ok),
        (
            VehicleClass::Motorbike,
            r.access.allows(Vehicle::Motorcycle),
        ),
        (VehicleClass::Tractor, r.access.allows(Vehicle::Tractor)),
    ] {
        if ok {
            a = a.with(c);
        }
    }
    // On roads the class's own limits too (tracks: as the package opens them).
    if class != RoadClass::Track {
        a = Access(a.0 & class_defaults(class).access.0);
    }
    a
}

/// The core's rule of one edge from T2's.
pub fn edge_rule(r: &wt::EdgeRules, class: RoadClass) -> EdgeRule {
    let d = class_defaults(class);
    let access = access_of(r, class);
    let lanes = if access.is_none() {
        [0, 0]
    } else {
        [
            if r.oneway.allows(true) {
                r.lanes[0].max(1)
            } else {
                0
            },
            if r.oneway.allows(false) {
                r.lanes[1].max(1)
            } else {
                0
            },
        ]
    };
    let main = lanes[0] > 0 || lanes[1] == 0;
    let kmh = f64::from(r.target_kmh(class, main)).max(5.0);
    let dest = if r.destination_only { 0.2 } else { 1.0 };
    EdgeRule {
        oneway: r.oneway,
        lanes,
        speed: kmh / 3.6,
        access,
        aadt: d.aadt * f64::from(r.flow.max(0.0)) * dest,
        roundabout: r.roundabout,
        rank: d.rank,
    }
}

impl WorldRules {
    /// The rules of `net`'s package ([`RoadNetwork::road_rules`]).
    pub fn new(net: &RoadNetwork) -> Self {
        Self::from_rules(&net.road_rules(), net)
    }

    /// The rules of `package` for its network `net` (the package's `TRFC`,
    /// whether or not `net` carries it).
    pub fn of(package: &ocw_world::WorldPackage, net: &RoadNetwork) -> Self {
        Self::from_rules(&wt::RoadRules::of(package), net)
    }

    /// The rules of `rr` on `net` (by edge index).
    pub fn from_rules(rr: &wt::RoadRules<'_>, net: &RoadNetwork) -> Self {
        let graph = net.graph();
        let world = rr.aligned(graph);
        let rules: Vec<EdgeRule> = graph
            .edges
            .iter()
            .zip(&world)
            .map(|(e, w)| edge_rule(w, e.class))
            .collect();
        let mut controls = BTreeMap::new();
        let mut controlled = BTreeSet::new();
        let mut signalled = BTreeSet::new();
        for (i, e) in graph.edges.iter().enumerate() {
            if rules[i].access.is_none() {
                continue;
            }
            let len = net.geometry(i).length() as f32;
            for c in rr.controls_on(e.id) {
                let ctrl = match c.kind {
                    ControlKind::Signals => Control::Signals,
                    ControlKind::Stop => Control::Stop,
                    ControlKind::GiveWay => Control::GiveWay,
                    // Lights at a crossing along the road: not a junction.
                    ControlKind::CrossingSignals => continue,
                };
                // The nearer end: arriving there forward (at `to`) or
                // backward (at `from`).
                let at_to = c.along_m * 2.0 >= len;
                let dist = if at_to { len - c.along_m } else { c.along_m };
                if !c.at_junction && dist > SIGN_REACH_M {
                    continue;
                }
                if !c.applies.includes(at_to) {
                    continue;
                }
                let vertex = if at_to { e.to } else { e.from };
                if net.edges_at(vertex).len() <= 2 {
                    // A bend or a road end: nothing to give way to.
                    continue;
                }
                controlled.insert(vertex.0);
                if ctrl == Control::Signals {
                    signalled.insert(vertex.0);
                }
                // The strictest of several (signals over a sign).
                let key = (vertex.0, i as u32, at_to);
                let old = controls.get(&key).copied();
                if old.is_none_or(|o| strictness(ctrl) > strictness(o)) {
                    controls.insert(key, ctrl);
                }
            }
        }
        Self {
            world,
            rules,
            controls,
            controlled,
            signalled,
            from_data: rr.data.is_some(),
            country: rr.country,
        }
    }

    /// The package's rules of edge `e`.
    pub fn world_rule(&self, e: usize) -> &wt::EdgeRules {
        &self.world[e]
    }

    /// The package had a `TRFC` section (else: T2's implied rules).
    pub fn from_data(&self) -> bool {
        self.from_data
    }

    /// The country of the package (its holiday region for the demand).
    pub fn country(&self) -> Country {
        self.country
    }

    /// The junctions with signals.
    pub fn signalled(&self) -> usize {
        self.signalled.len()
    }

    /// The (vertex, edge, forward) arrivals with an explicit control.
    pub fn controls(&self) -> usize {
        self.controls.len()
    }
}

fn strictness(c: Control) -> u8 {
    match c {
        Control::Signals => 3,
        Control::Stop => 2,
        Control::GiveWay => 1,
        _ => 0,
    }
}

impl EdgeRules for WorldRules {
    fn edge_rule(&self, _net: &RoadNetwork, edge: usize) -> EdgeRule {
        self.rules[edge]
    }

    fn node_control(
        &self,
        net: &RoadNetwork,
        vertex: VertexId,
        edge: usize,
        forward: bool,
    ) -> Option<Control> {
        if let Some(c) = self.controls.get(&(vertex.0, edge as u32, forward)) {
            return Some(*c);
        }
        if self.signalled.contains(&vertex.0) {
            // Every motor arm of a signalled junction has the lights.
            return Some(Control::Signals);
        }
        if self.controlled.contains(&vertex.0)
            && !net
                .edges_at(vertex)
                .iter()
                .any(|&x| self.rules[x].roundabout)
        {
            // The arms without a sign are the major road.
            return Some(Control::Priority);
        }
        None
    }

    fn passing(&self, _net: &RoadNetwork, edge: usize) -> PassRule {
        let w = &self.world[edge];
        PassRule::new(w.country, w.urban, w.no_overtaking)
    }
}

/// The road of a route without a road graph (the test routes, route
/// files; spec 18E T5) as a network the traffic can drive: a loop is a
/// ring of two edges (split half way), an open route one edge with a dead
/// end at each end; a secondary road of the route's width with its
/// heights. The same route gives the same network (and fingerprint) on
/// the server and every client.
pub fn route_network(route: &ocw_world::Route) -> RoadNetwork {
    use ocw_world::graph::{Edge, EdgeId, Oneway, PROFILE_STEP_M, RoadGraph};
    let pts = route.points();
    let mut cum = vec![0.0];
    for w in pts.windows(2) {
        let d = ((w[1][0] - w[0][0]).powi(2) + (w[1][2] - w[0][2]).powi(2)).sqrt();
        cum.push(cum.last().copied().unwrap_or(0.0) + d);
    }
    let edge = |k: u64, from: u32, to: u32, range: &[usize]| {
        let points: Vec<[f64; 2]> = range.iter().map(|&i| [pts[i][0], pts[i][2]]).collect();
        let start = cum[range[0]];
        let len = cum[*range.last().expect("points")] - start;
        let height = |s: f64| {
            let d = start + s;
            let i = cum.partition_point(|&c| c <= d).clamp(1, pts.len() - 1);
            let seg = cum[i] - cum[i - 1];
            let t = if seg > 0.0 {
                ((d - cum[i - 1]) / seg).clamp(0.0, 1.0)
            } else {
                0.0
            };
            pts[i - 1][1] + (pts[i][1] - pts[i - 1][1]) * t
        };
        let n = (len / PROFILE_STEP_M).floor() as usize;
        let mut profile: Vec<f64> = (0..=n).map(|j| height(j as f64 * PROFILE_STEP_M)).collect();
        if (n as f64) * PROFILE_STEP_M < len - 1e-9 {
            profile.push(height(len));
        }
        Edge {
            id: EdgeId(k),
            from: VertexId(from),
            to: VertexId(to),
            points,
            class: RoadClass::Secondary,
            name: Some(route.name().to_owned()),
            width_m: route.road_width_m(),
            bridge: false,
            tunnel: false,
            dead_end_m: None,
            boundary: false,
            oneway: Oneway::No,
            osm_way: 0,
            profile,
        }
    };
    let last = pts.len() - 1;
    let (vertices, edges) = if route.is_loop() && pts.len() >= 4 {
        let half = route.length() / 2.0;
        let mid = cum.partition_point(|&c| c < half).clamp(1, last - 1);
        let first: Vec<usize> = (0..=mid).collect();
        // The loop's last point repeats the first.
        let second: Vec<usize> = (mid..=last).collect();
        (
            vec![[pts[0][0], pts[0][2]], [pts[mid][0], pts[mid][2]]],
            vec![edge(1, 0, 1, &first), edge(2, 1, 0, &second)],
        )
    } else {
        let all: Vec<usize> = (0..=last).collect();
        (
            vec![[pts[0][0], pts[0][2]], [pts[last][0], pts[last][2]]],
            vec![edge(1, 0, 1, &all)],
        )
    };
    RoadNetwork::new(RoadGraph {
        vertices,
        edges,
        boundary_vertices: Default::default(),
    })
    .with_package_name(route.id())
}

/// Where a rider `distance` metres along `route` is on its
/// [`route_network`]: (edge, metres from the edge's `from`), riding
/// forward.
pub fn on_route_network(
    net: &RoadNetwork,
    route: &ocw_world::Route,
    distance: f64,
) -> (usize, f64) {
    let d = route.normalize_distance(distance);
    let first = net.edge(0).length_m();
    if net.graph().edges.len() > 1 && d > first {
        (1, (d - first).min(net.edge(1).length_m()))
    } else {
        (0, d.min(first))
    }
}

#[cfg(test)]
#[path = "world_tests.rs"]
mod tests;
