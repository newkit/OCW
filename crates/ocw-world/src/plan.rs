//! Route planning (spec §14 Phase 16, 16E, 16H Q3): the built-in planner on
//! the package's road graph and the [`RoutePlanner`] interface the external
//! planners (BRouter first, [`brouter`]) implement.
//!
//! - **The built-in planner** ([`plan_leg`], [`plan_route`]): Dijkstra on
//!   the [`RoadNetwork`] between via-points snapped onto the roads, with a
//!   cycling [`Profile`] as the cost model ([`EdgeCosts`]: a factor per
//!   road class and surface, a climbing cost from the road profile,
//!   one-way roads only their way). Its result is a [`RoutePlan`] on the
//!   graph – no matching needed. Offline, no service, no extra licence.
//! - **External planners** answer with a polyline (and maybe elevation):
//!   a [`PlannedTrack`]; the studio matches it onto the package's graph
//!   with the HMM matcher ([`crate::matching`]), which marks what does not
//!   fit. The planners' HTTP transports are the apps' (native or the
//!   browser's `fetch`); this module only builds requests and reads
//!   answers, so it stays pure and wasm-safe.

pub mod brouter;

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use crate::geo::LocalProjection;
use crate::gpx::GpxPoint;
use crate::graph::RoadClass;
use crate::graph_route::{RouteBreak, RoutePlan};
use crate::network::RoadNetwork;
use crate::path::{Segment, TurnReason};
use crate::road_surface::RoadSurface;

/// A cycling profile of the built-in planner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Profile {
    /// Paved roads; gravel, dirt and paths are avoided where a road exists.
    #[default]
    RoadBike,
    /// Tracks and gravel welcome, big roads avoided.
    Gravel,
    /// Tracks and paths preferred.
    Mtb,
    /// Small roads and cycleways; big roads strongly avoided.
    Quiet,
}

impl Profile {
    pub const ALL: [Profile; 4] = [
        Profile::RoadBike,
        Profile::Gravel,
        Profile::Mtb,
        Profile::Quiet,
    ];

    /// The id in documents, settings and the console.
    pub fn id(self) -> &'static str {
        match self {
            Self::RoadBike => "road",
            Self::Gravel => "gravel",
            Self::Mtb => "mtb",
            Self::Quiet => "quiet",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::RoadBike => "Road bike",
            Self::Gravel => "Gravel",
            Self::Mtb => "MTB",
            Self::Quiet => "Quiet roads",
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.id() == id.trim())
    }

    /// The cost per metre of a road class (1 = the best roads).
    pub fn class_factor(self, class: RoadClass) -> f64 {
        use RoadClass::*;
        match self {
            Self::RoadBike => match class {
                Trunk => 3.0,
                Primary => 1.6,
                Secondary => 1.2,
                Tertiary | Unclassified | Residential | Cycleway => 1.0,
                Service => 1.3,
                Track => 2.5,
                Path => 4.0,
            },
            Self::Gravel => match class {
                Trunk => 5.0,
                Primary => 2.5,
                Secondary => 1.6,
                Tertiary => 1.2,
                Unclassified | Residential | Service | Cycleway => 1.0,
                Track => 0.9,
                Path => 1.5,
            },
            Self::Mtb => match class {
                Trunk => 6.0,
                Primary => 3.0,
                Secondary => 2.0,
                Tertiary => 1.5,
                Unclassified | Residential | Service | Cycleway => 1.2,
                Track => 0.9,
                Path => 0.8,
            },
            Self::Quiet => match class {
                Trunk => 8.0,
                Primary => 4.0,
                Secondary => 2.0,
                Tertiary => 1.3,
                Unclassified | Residential | Service => 1.0,
                Cycleway => 0.9,
                Track => 1.8,
                Path => 3.0,
            },
        }
    }

    /// The cost per metre of a surface.
    pub fn surface_factor(self, s: RoadSurface) -> f64 {
        use RoadSurface::*;
        match (self, s) {
            (_, Asphalt) => 1.0,
            (Self::RoadBike, WornAsphalt) => 1.2,
            (Self::RoadBike, Paving) => 1.5,
            (Self::RoadBike, Gravel) => 3.0,
            (Self::RoadBike, Dirt) => 5.0,
            (Self::Quiet, WornAsphalt) => 1.1,
            (Self::Quiet, Paving) => 1.3,
            (Self::Quiet, Gravel) => 1.6,
            (Self::Quiet, Dirt) => 2.5,
            (Self::Gravel, Gravel | Dirt) => 0.95,
            (Self::Mtb, Gravel | Dirt) => 0.9,
            (_, _) => 1.05,
        }
    }

    /// Extra cost per metre climbed (in metres of flat road).
    pub fn climb_cost(self) -> f64 {
        match self {
            Self::RoadBike | Self::Quiet => 6.0,
            Self::Gravel => 5.0,
            Self::Mtb => 4.0,
        }
    }
}

/// The cost per metre of every edge in both riding directions
/// (`f64::INFINITY`: not allowed – a one-way road the wrong way), and the
/// climbing of each direction.
#[derive(Debug, Clone, PartialEq)]
pub struct EdgeCosts {
    pub profile: Profile,
    /// Per edge: (forward, backward) total cost of the whole edge.
    whole: Vec<[f64; 2]>,
    /// Per edge: cost per metre (forward, backward).
    per_m: Vec<[f64; 2]>,
}

impl EdgeCosts {
    /// The costs of `profile` on `net`; `surface` is the package's surface
    /// of an edge (index), else the class's usual one is assumed.
    pub fn new(
        net: &RoadNetwork,
        profile: Profile,
        surface: Option<&dyn Fn(usize) -> RoadSurface>,
    ) -> Self {
        let n = net.graph().edges.len();
        let mut whole = Vec::with_capacity(n);
        let mut per_m = Vec::with_capacity(n);
        for i in 0..n {
            let e = net.edge(i);
            let len = net.geometry(i).length().max(1e-3);
            let s = surface.map_or_else(|| RoadSurface::of_class(e.class), |f| f(i));
            let base = profile.class_factor(e.class) * profile.surface_factor(s);
            let (up_f, up_b) = climbs(net, i);
            let mut w = [f64::INFINITY; 2];
            let mut p = [f64::INFINITY; 2];
            for (k, forward) in [true, false].into_iter().enumerate() {
                if !e.oneway.allows(forward) {
                    continue;
                }
                let up = if forward { up_f } else { up_b };
                let total = len * base + up * profile.climb_cost();
                w[k] = total;
                p[k] = total / len;
            }
            whole.push(w);
            per_m.push(p);
        }
        Self {
            profile,
            whole,
            per_m,
        }
    }

    /// Cost of riding `edge` from `from_m` to `to_m`.
    pub fn cost(&self, edge: usize, from_m: f64, to_m: f64) -> f64 {
        let k = usize::from(to_m < from_m);
        self.per_m[edge][k] * (to_m - from_m).abs()
    }

    fn whole(&self, edge: usize, forward: bool) -> f64 {
        self.whole[edge][usize::from(!forward)]
    }

    pub fn allows(&self, edge: usize, forward: bool) -> bool {
        self.whole(edge, forward).is_finite()
    }
}

/// Metres climbed along an edge forward and backward (from its profile).
fn climbs(net: &RoadNetwork, edge: usize) -> (f64, f64) {
    let g = net.geometry(edge);
    let (mut up, mut down) = (0.0, 0.0);
    for w in g.points.windows(2) {
        let d = w[1][1] - w[0][1];
        if d > 0.0 {
            up += d;
        } else {
            down -= d;
        }
    }
    (up, down)
}

/// A via-point snapped onto the road graph.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Snap {
    pub edge: usize,
    /// Metres from the edge's first vertex.
    pub s: f64,
    /// How far the clicked point was from the road (m).
    pub distance_m: f64,
}

/// The road point nearest to `p` (x, z).
pub fn snap(net: &RoadNetwork, p: [f64; 2]) -> Option<Snap> {
    let (edge, s) = net.nearest(p)?;
    let q = net.geometry(edge).point_at(s);
    Some(Snap {
        edge,
        s,
        distance_m: (q[0] - p[0]).hypot(q[2] - p[1]),
    })
}

/// Why a route could not be planned.
#[derive(Debug, Clone, PartialEq)]
pub enum PlanError {
    /// Fewer than two via-points.
    TooFewVias,
    /// The package has no roads (or via-point `via` is nowhere near one).
    NoRoad { via: usize },
    /// No ridable way between via-point `leg` and the next.
    Unreachable { leg: usize },
    /// The planned steps are not a valid route (internal).
    Invalid(RouteBreak),
    /// No server is set for an external planner.
    NoServer(String),
    /// The planner's server answered with an error.
    Server(String),
    /// The request did not get an answer (network, timeout).
    Network(String),
    /// The answer could not be read.
    Parse(String),
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooFewVias => f.write_str("a route needs at least two via-points"),
            Self::NoRoad { via } => write!(f, "via-point {} is not near a road", via + 1),
            Self::Unreachable { leg } => write!(
                f,
                "no ridable way from via-point {} to via-point {}",
                leg + 1,
                leg + 2
            ),
            Self::Invalid(b) => write!(f, "the planned route is not valid: {b}"),
            Self::NoServer(m) | Self::Server(m) | Self::Network(m) | Self::Parse(m) => {
                f.write_str(m)
            }
        }
    }
}

impl std::error::Error for PlanError {}

/// The search's nodes: the graph's vertices, plus the target (the last).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Back {
    None,
    /// Reached from the start snap along its edge.
    Start {
        forward: bool,
    },
    /// Reached from vertex `from` over edge `edge`.
    Edge {
        from: usize,
        edge: usize,
        forward: bool,
    },
    /// The target, reached from vertex `from` onto the target's edge.
    Target {
        from: usize,
        forward: bool,
    },
}

/// The cheapest way from `from` to `to` under `costs`: edge steps (no
/// zero-length ones; empty when both are the same point).
pub fn plan_leg(
    net: &RoadNetwork,
    costs: &EdgeCosts,
    from: Snap,
    to: Snap,
) -> Option<Vec<Segment>> {
    const EPS: f64 = 1e-6;
    let nv = net.graph().vertices.len();
    let target = nv;
    let mut best = vec![f64::INFINITY; nv + 1];
    let mut back = vec![Back::None; nv + 1];
    let mut heap: BinaryHeap<Reverse<(Ord64, usize)>> = BinaryHeap::new();
    let len0 = net.geometry(from.edge).length();
    let len1 = net.geometry(to.edge).length();
    // Straight along the shared edge.
    if from.edge == to.edge {
        let forward = to.s >= from.s;
        if costs.allows(from.edge, forward) || (to.s - from.s).abs() < EPS {
            let c = costs.cost(from.edge, from.s, to.s);
            if (to.s - from.s).abs() < EPS {
                return Some(Vec::new());
            }
            best[target] = c;
            back[target] = Back::Start { forward };
            heap.push(Reverse((Ord64(c), target)));
        }
    }
    for forward in [true, false] {
        if !costs.allows(from.edge, forward) {
            continue;
        }
        let end = if forward { len0 } else { 0.0 };
        let v = net.end_vertex(from.edge, forward).0 as usize;
        let c = costs.cost(from.edge, from.s, end);
        if c < best[v] {
            best[v] = c;
            back[v] = Back::Start { forward };
            heap.push(Reverse((Ord64(c), v)));
        }
    }
    while let Some(Reverse((Ord64(c), v))) = heap.pop() {
        if c > best[v] {
            continue;
        }
        if v == target {
            break;
        }
        let vid = crate::graph::VertexId(v as u32);
        // Onto the target's edge from this vertex.
        let e1 = net.edge(to.edge);
        for (forward, at_start) in [(true, e1.from == vid), (false, e1.to == vid)] {
            if !at_start || !costs.allows(to.edge, forward) {
                continue;
            }
            let begin = if forward { 0.0 } else { len1 };
            let nc = c + costs.cost(to.edge, begin, to.s);
            if nc < best[target] {
                best[target] = nc;
                back[target] = Back::Target { from: v, forward };
                heap.push(Reverse((Ord64(nc), target)));
            }
        }
        for &edge in net.edges_at(vid) {
            for forward in [true, false] {
                if net.start_vertex(edge, forward) != vid || !costs.allows(edge, forward) {
                    continue;
                }
                let w = net.end_vertex(edge, forward).0 as usize;
                let nc = c + costs.whole(edge, forward);
                if nc < best[w] {
                    best[w] = nc;
                    back[w] = Back::Edge {
                        from: v,
                        edge,
                        forward,
                    };
                    heap.push(Reverse((Ord64(nc), w)));
                }
            }
        }
    }
    if !best[target].is_finite() {
        return None;
    }
    // Walk back.
    let mut rev: Vec<Segment> = Vec::new();
    let mut at = target;
    loop {
        match back[at] {
            Back::None => return None,
            Back::Start { forward } => {
                let end = if at == target {
                    to.s
                } else if forward {
                    len0
                } else {
                    0.0
                };
                rev.push(Segment::Edge {
                    edge: from.edge,
                    from_m: from.s,
                    to_m: end,
                });
                break;
            }
            Back::Target { from: v, forward } => {
                rev.push(Segment::Edge {
                    edge: to.edge,
                    from_m: if forward { 0.0 } else { len1 },
                    to_m: to.s,
                });
                at = v;
            }
            Back::Edge {
                from: v,
                edge,
                forward,
            } => {
                rev.push(Segment::whole(net, edge, forward));
                at = v;
            }
        }
    }
    rev.reverse();
    rev.retain(|s| match *s {
        Segment::Edge { from_m, to_m, .. } => (to_m - from_m).abs() > EPS,
        Segment::Turnaround(_) => true,
    });
    Some(rev)
}

/// Totally ordered f64 for the heap (costs are never NaN).
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
struct Ord64(f64);
impl Eq for Ord64 {}
impl Ord for Ord64 {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&o.0)
    }
}

/// Joins legs into one list of steps: a U-turn where the route turns back
/// on the same road at a via-point, steps on the same edge in the same
/// direction merged.
pub fn join_legs(legs: &[Vec<Segment>]) -> Vec<Segment> {
    let mut out: Vec<Segment> = Vec::new();
    for s in legs.iter().flatten() {
        let Segment::Edge { edge, from_m, to_m } = *s else {
            out.push(*s);
            continue;
        };
        if let Some(Segment::Edge {
            edge: pe,
            from_m: pf,
            to_m: pt,
        }) = out.last().copied()
            && pe == edge
            && (pt - from_m).abs() < 0.01
        {
            if (pt >= pf) == (to_m >= from_m) {
                *out.last_mut().expect("checked") = Segment::Edge {
                    edge,
                    from_m: pf,
                    to_m,
                };
                continue;
            }
            out.push(Segment::Turnaround(TurnReason::Manual));
        }
        out.push(*s);
    }
    // A merge can leave a step of zero length (there and back again).
    out.retain(|s| match *s {
        Segment::Edge { from_m, to_m, .. } => (to_m - from_m).abs() > 1e-6,
        Segment::Turnaround(_) => true,
    });
    // ... and with it two U-turns in a row, or one at an end.
    let mut clean: Vec<Segment> = Vec::with_capacity(out.len());
    for s in out {
        if matches!(s, Segment::Turnaround(_))
            && (clean.is_empty() || matches!(clean.last(), Some(Segment::Turnaround(_))))
        {
            continue;
        }
        clean.push(s);
    }
    while matches!(clean.last(), Some(Segment::Turnaround(_))) {
        clean.pop();
    }
    clean
}

/// Plans a route through `via` (x, z) on `net`: the snapped via-points and
/// the legs between them.
pub fn plan_legs(
    net: &RoadNetwork,
    costs: &EdgeCosts,
    via: &[[f64; 2]],
) -> Result<(Vec<Snap>, Vec<Vec<Segment>>), PlanError> {
    if via.len() < 2 {
        return Err(PlanError::TooFewVias);
    }
    let snaps = via
        .iter()
        .enumerate()
        .map(|(i, p)| snap(net, *p).ok_or(PlanError::NoRoad { via: i }))
        .collect::<Result<Vec<_>, _>>()?;
    let legs = snaps
        .windows(2)
        .enumerate()
        .map(|(i, w)| plan_leg(net, costs, w[0], w[1]).ok_or(PlanError::Unreachable { leg: i }))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((snaps, legs))
}

/// Plans a route through `via` (x, z) and returns it as a [`RoutePlan`].
pub fn plan_route(
    net: &RoadNetwork,
    costs: &EdgeCosts,
    via: &[[f64; 2]],
    name: &str,
) -> Result<RoutePlan, PlanError> {
    let (_, legs) = plan_legs(net, costs, via)?;
    RoutePlan::new(net, name, join_legs(&legs), false).map_err(PlanError::Invalid)
}

/// Points (x, height, z) along steps every `step_m` metres (and at every
/// step's ends), heights from the road profile.
pub fn steps_points(net: &RoadNetwork, steps: &[Segment], step_m: f64) -> Vec<[f64; 3]> {
    let mut out: Vec<[f64; 3]> = Vec::new();
    for s in steps {
        let Segment::Edge { edge, from_m, to_m } = *s else {
            continue;
        };
        let g = net.geometry(edge);
        let n = ((to_m - from_m).abs() / step_m.max(0.5)).ceil().max(1.0) as usize;
        for k in 0..=n {
            let p = g.point_at(from_m + (to_m - from_m) * k as f64 / n as f64);
            if out
                .last()
                .is_none_or(|l| (l[0] - p[0]).hypot(l[2] - p[2]) > 0.01)
            {
                out.push(p);
            }
        }
    }
    out
}

/// What a planner answers: a polyline with optional elevation.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlannedTrack {
    pub points: Vec<GpxPoint>,
    /// What the planner says about it (m, m, s), if it does.
    pub length_m: Option<f64>,
    pub ascent_m: Option<f64>,
    pub time_s: Option<f64>,
    /// "Routing: BRouter" – shown with the route (16E *Licences*).
    pub credit: String,
}

/// A routing planner: via-points (lat, lon) and a profile to a polyline
/// (16E). The built-in planner implements it too ([`BuiltinPlanner`]),
/// so the studio and the game can offer them side by side.
pub trait RoutePlanner {
    /// "builtin", "brouter".
    fn id(&self) -> &str;
    /// The credit shown with its routes.
    fn credit(&self) -> String;
    /// Whether via-points leave the device (§12: the app says so first).
    fn is_external(&self) -> bool;
    fn plan(&self, via: &[(f64, f64)], profile: &str) -> Result<PlannedTrack, PlanError>;
}

/// The built-in planner as a [`RoutePlanner`] (on the package graph,
/// projected around `origin`).
pub struct BuiltinPlanner<'a> {
    pub net: &'a RoadNetwork,
    pub origin: (f64, f64),
    /// The package's surface of an edge, if known.
    pub surface: Option<&'a dyn Fn(usize) -> RoadSurface>,
}

impl RoutePlanner for BuiltinPlanner<'_> {
    fn id(&self) -> &str {
        "builtin"
    }
    fn credit(&self) -> String {
        "Routing: OCW built-in planner, © OpenStreetMap contributors".into()
    }
    fn is_external(&self) -> bool {
        false
    }
    fn plan(&self, via: &[(f64, f64)], profile: &str) -> Result<PlannedTrack, PlanError> {
        let profile = Profile::parse(profile).unwrap_or_default();
        let costs = EdgeCosts::new(self.net, profile, self.surface);
        let proj = LocalProjection::new(self.origin.0, self.origin.1);
        let pts: Vec<[f64; 2]> = via.iter().map(|&(a, o)| proj.project(a, o)).collect();
        let plan = plan_route(self.net, &costs, &pts, "planned")?;
        let points: Vec<GpxPoint> = steps_points(self.net, &plan.steps, 10.0)
            .into_iter()
            .map(|p| {
                let (lat, lon) = proj.unproject([p[0], p[2]]);
                GpxPoint {
                    lat,
                    lon,
                    ele: Some(p[1]),
                }
            })
            .collect();
        Ok(PlannedTrack {
            points,
            length_m: Some(plan.length_m()),
            ascent_m: Some(plan.elevation_gain_m(self.net)),
            time_s: None,
            credit: self.credit(),
        })
    }
}

/// A made-up test network (tests here, in `ocw-edit` and the studio): a
/// 4 × 4 grid 200 m apart; row 0 (z = 0) is a primary road, the
/// column x = 600 a track; edge ids 1000 + index.
pub fn test_grid() -> RoadNetwork {
    use crate::graph::{Edge, EdgeId, Oneway, RoadGraph, VertexId};
    let mut v = Vec::new();
    for row in 0..4 {
        for col in 0..4 {
            v.push([f64::from(col) * 200.0, f64::from(row) * 200.0]);
        }
    }
    let mut edges = Vec::new();
    let mut add = |a: u32, b: u32, class: RoadClass| {
        let i = edges.len() as u64;
        edges.push(Edge {
            id: EdgeId(1000 + i),
            from: VertexId(a),
            to: VertexId(b),
            points: vec![v[a as usize], v[b as usize]],
            class,
            name: None,
            width_m: 5.0,
            bridge: false,
            tunnel: false,
            dead_end_m: None,
            boundary: false,
            oneway: Oneway::No,
            osm_way: i as i64,
            profile: Vec::new(),
        });
    };
    for row in 0..4u32 {
        for col in 0..4u32 {
            let i = row * 4 + col;
            if col < 3 {
                add(
                    i,
                    i + 1,
                    if row == 0 {
                        RoadClass::Primary
                    } else {
                        RoadClass::Residential
                    },
                );
            }
            if row < 3 {
                add(
                    i,
                    i + 4,
                    if col == 3 {
                        RoadClass::Track
                    } else {
                        RoadClass::Residential
                    },
                );
            }
        }
    }
    RoadNetwork::new(RoadGraph {
        vertices: v.clone(),
        edges,
        boundary_vertices: Default::default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{Oneway, VertexId};

    use super::test_grid as grid;

    fn vertices_of(net: &RoadNetwork, steps: &[Segment]) -> Vec<u32> {
        let mut out = Vec::new();
        for s in steps {
            if let Segment::Edge { edge, from_m, to_m } = *s {
                let fwd = to_m >= from_m;
                let len = net.geometry(edge).length();
                if (fwd && to_m >= len - 1e-6) || (!fwd && to_m <= 1e-6) {
                    out.push(net.end_vertex(edge, fwd).0);
                }
            }
        }
        out
    }

    #[test]
    fn a_leg_between_two_points_takes_the_shortest_way() {
        let net = grid();
        let costs = EdgeCosts::new(&net, Profile::RoadBike, None);
        // From the middle of 5-6 to the middle of 9-10.
        let a = snap(&net, [300.0, 200.0]).unwrap();
        let b = snap(&net, [300.0, 400.0]).unwrap();
        let steps = plan_leg(&net, &costs, a, b).unwrap();
        let plan = RoutePlan::new(&net, "t", steps, false).unwrap();
        assert!(
            (plan.length_m() - 400.0).abs() < 1e-6,
            "{}",
            plan.length_m()
        );
        // The same edge: straight along it.
        let c = snap(&net, [350.0, 200.0]).unwrap();
        let steps = plan_leg(&net, &costs, a, c).unwrap();
        assert_eq!(steps.len(), 1);
        assert!((RoutePlan::new(&net, "t", steps, false).unwrap().length_m() - 50.0).abs() < 1e-6);
        // The same point: nothing to ride.
        assert!(plan_leg(&net, &costs, a, a).unwrap().is_empty());
    }

    #[test]
    fn the_road_bike_avoids_the_track_and_the_quiet_profile_the_main_road() {
        let net = grid();
        // Along x = 600 from z = 0 to z = 600: the track is the direct way.
        let via = [[600.0, 0.0], [600.0, 600.0]];
        let mtb = plan_route(&net, &EdgeCosts::new(&net, Profile::Mtb, None), &via, "m").unwrap();
        assert!((mtb.length_m() - 600.0).abs() < 1e-6);
        let road = plan_route(
            &net,
            &EdgeCosts::new(&net, Profile::RoadBike, None),
            &via,
            "r",
        )
        .unwrap();
        assert!(
            (road.length_m() - 1000.0).abs() < 1e-6,
            "{}",
            road.length_m()
        );
        let track_edges = road
            .steps
            .iter()
            .filter(|s| matches!(s, Segment::Edge { edge, .. } if net.edge(*edge).class == RoadClass::Track))
            .count();
        assert_eq!(track_edges, 0);
        // Along the primary row 0 from x = 0 to x = 600: quiet takes row 1.
        let via = [[0.0, 0.0], [600.0, 0.0]];
        let quiet =
            plan_route(&net, &EdgeCosts::new(&net, Profile::Quiet, None), &via, "q").unwrap();
        assert!(quiet.length_m() > 600.0 + 1.0);
        let road = plan_route(
            &net,
            &EdgeCosts::new(&net, Profile::RoadBike, None),
            &via,
            "r",
        )
        .unwrap();
        assert!((road.length_m() - 600.0).abs() < 1e-6);
    }

    #[test]
    fn one_way_roads_are_ridden_only_their_way() {
        let mut net = grid();
        let mut g = net.graph().clone();
        // 0 → 1 only (edge 0 runs from vertex 0 to 1).
        g.edges[0].oneway = Oneway::Forward;
        net = RoadNetwork::new(g);
        let costs = EdgeCosts::new(&net, Profile::RoadBike, None);
        let there = plan_route(&net, &costs, &[[0.0, 0.0], [200.0, 0.0]], "a").unwrap();
        assert!((there.length_m() - 200.0).abs() < 1e-6);
        let back = plan_route(&net, &costs, &[[200.0, 0.0], [0.0, 0.0]], "b").unwrap();
        assert!(
            (back.length_m() - 600.0).abs() < 1e-6,
            "{}",
            back.length_m()
        );
        assert_eq!(vertices_of(&net, &back.steps), vec![5, 4, 0]);
    }

    #[test]
    fn a_via_point_where_the_route_turns_back_gets_a_u_turn() {
        let net = grid();
        let costs = EdgeCosts::new(&net, Profile::RoadBike, None);
        let via = [[250.0, 400.0], [300.0, 400.0], [250.0, 400.0]];
        let (_, legs) = plan_legs(&net, &costs, &via).unwrap();
        let steps = join_legs(&legs);
        assert!(matches!(steps[1], Segment::Turnaround(_)), "{steps:?}");
        let plan = RoutePlan::new(&net, "t", steps, false).unwrap();
        assert!((plan.length_m() - 100.0).abs() < 1e-6);
        // Legs that go on in the same direction are merged into one step.
        let via = [[220.0, 400.0], [300.0, 400.0], [380.0, 400.0]];
        let (_, legs) = plan_legs(&net, &costs, &via).unwrap();
        assert_eq!(join_legs(&legs).len(), 1);
    }

    #[test]
    fn climbing_costs_count_in_the_riding_direction() {
        let net = grid();
        let mut g = net.graph().clone();
        // Edge 0 (0 → 1) climbs 30 m.
        g.edges[0].profile = (0..=20).map(|k| f64::from(k) * 1.5).collect();
        let net = RoadNetwork::new(g);
        let c = EdgeCosts::new(&net, Profile::RoadBike, None);
        let len = net.geometry(0).length();
        let up = c.cost(0, 0.0, len);
        let down = c.cost(0, len, 0.0);
        assert!(up > down + 100.0, "{up} {down}");
    }

    #[test]
    fn unreachable_and_too_few_via_points_are_errors() {
        let net = grid();
        let costs = EdgeCosts::new(&net, Profile::RoadBike, None);
        assert_eq!(
            plan_route(&net, &costs, &[[0.0, 0.0]], "x").unwrap_err(),
            PlanError::TooFewVias
        );
        let mut g = net.graph().clone();
        // Vertex 15's two edges one-way into it: no way out.
        for e in &mut g.edges {
            if e.to == VertexId(15) {
                e.oneway = Oneway::Forward;
            }
        }
        let net = RoadNetwork::new(g);
        let costs = EdgeCosts::new(&net, Profile::RoadBike, None);
        assert_eq!(
            plan_route(&net, &costs, &[[600.0, 600.0], [0.0, 0.0]], "x").unwrap_err(),
            PlanError::Unreachable { leg: 0 }
        );
    }

    #[test]
    fn the_builtin_planner_answers_like_an_external_one() {
        let net = grid();
        let p = BuiltinPlanner {
            net: &net,
            origin: (47.0, 9.5),
            surface: None,
        };
        let proj = LocalProjection::new(47.0, 9.5);
        let a = proj.unproject([0.0, 0.0]);
        let b = proj.unproject([400.0, 200.0]);
        let t = p.plan(&[a, b], "road").unwrap();
        assert!((t.length_m.unwrap() - 600.0).abs() < 1e-6);
        assert!(!p.is_external());
        assert!(t.credit.contains("OpenStreetMap"));
        let first = t.points[0];
        assert!((first.lat - a.0).abs() < 1e-7 && (first.lon - a.1).abs() < 1e-7);
    }

    #[test]
    fn profiles_have_ids() {
        for p in Profile::ALL {
            assert_eq!(Profile::parse(p.id()), Some(p));
        }
        assert_eq!(Profile::parse("bmx"), None);
    }
}
