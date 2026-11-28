//! Live road events in the simulated traffic (spec 14 Phase 18, 18D *The*
//! Live traffic data *switch*, 18E T13): closures, roadworks, reported
//! jams and obstructions of a live source (the DGT's DATEX II feed for
//! Tenerife), mapped onto the package's road edges and laid over the
//! edges' rules – never the riders' physics.
//!
//! - [`GeoEvent`] is one record of a source as it lies on the globe (its
//!   from / to points, the road's kilometres, the direction); [`map_events`]
//!   snaps it onto the network and follows the road between the two
//!   points: the [`EdgeEvent`]s (edge, direction, what).
//! - [`apply`] changes an [`EdgeRule`] by an event: a **closure** takes
//!   away the direction's lanes (both closed: no access – nothing spawns
//!   on it or routes over it); **roadworks** and **obstructions** lower
//!   the limit and close a lane where there are two; a **reported jam**
//!   lowers the limit to the queue's speed and raises the demand
//!   ([`JAM_DEMAND`]) so the car-following model forms the queue; a
//!   **speed limit** lowers the limit.
//! - [`super::TrafficSim::set_live`] puts a set of events in force (the
//!   statistics' rules are kept; no events = the statistics again).

use ocw_world::RoadNetwork;
use ocw_world::geo::LocalProjection;
use ocw_world::graph::Oneway;

use super::rules::{Access, EdgeRule};

/// What a live record does to the traffic on its edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LiveKind {
    /// The road (direction) is closed to motor traffic.
    Closed,
    /// Roadworks with a lane closed (or one lane in turns).
    LaneClosed,
    /// Roadworks without a lane closed: slower.
    Roadworks,
    /// A broken-down vehicle, an object, an accident: a lane blocked.
    Obstruction,
    /// A reported jam (slow, queuing or stationary traffic).
    Jam,
    /// A temporary speed limit.
    SpeedLimit,
}

impl LiveKind {
    pub const ALL: [Self; 6] = [
        Self::Closed,
        Self::LaneClosed,
        Self::Roadworks,
        Self::Obstruction,
        Self::Jam,
        Self::SpeedLimit,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Closed => "closed",
            Self::LaneClosed => "lane closed",
            Self::Roadworks => "roadworks",
            Self::Obstruction => "obstruction",
            Self::Jam => "jam",
            Self::SpeedLimit => "speed limit",
        }
    }

    /// The limit it sets when the record gives none (m/s; `None`: no
    /// change of the limit).
    pub fn default_speed(self) -> Option<f32> {
        match self {
            Self::Closed | Self::SpeedLimit => None,
            Self::LaneClosed => Some(ROADWORKS_LANE_KMH / 3.6),
            Self::Roadworks => Some(ROADWORKS_KMH / 3.6),
            Self::Obstruction => Some(OBSTRUCTION_KMH / 3.6),
            Self::Jam => Some(JAM_QUEUING_KMH / 3.6),
        }
    }
}

/// Roadworks with a lane closed: the limit (km/h).
pub const ROADWORKS_LANE_KMH: f32 = 40.0;
/// Roadworks without a lane closed (km/h).
pub const ROADWORKS_KMH: f32 = 60.0;
/// Past a broken-down vehicle or an object (km/h).
pub const OBSTRUCTION_KMH: f32 = 30.0;
/// Reported jams: slow traffic, queuing, stationary (walking pace; km/h).
pub const JAM_SLOW_KMH: f32 = 25.0;
pub const JAM_QUEUING_KMH: f32 = 10.0;
pub const JAM_STATIONARY_KMH: f32 = 4.0;
/// A reported jam multiplies the edge's demand by this (18D: "the demand on
/// those edges … raised until the model forms a queue").
pub const JAM_DEMAND: f64 = 2.5;
/// A temporary limit below this is not believed (km/h; the feed has a few
/// limits of 2 km/h).
pub const MIN_LIMIT_KMH: f32 = 10.0;
/// A record's point is matched to a road within this distance (m).
pub const SNAP_M: f64 = 60.0;

/// The directions of an edge an event holds for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Dir {
    Both,
    /// From the edge's `from` to its `to`.
    Forward,
    Backward,
}

impl Dir {
    pub fn holds(self, forward: bool) -> bool {
        match self {
            Self::Both => true,
            Self::Forward => forward,
            Self::Backward => !forward,
        }
    }

    pub fn of(forward: bool) -> Self {
        if forward {
            Self::Forward
        } else {
            Self::Backward
        }
    }
}

/// One live event on one edge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EdgeEvent {
    pub edge: u32,
    pub dir: Dir,
    pub kind: LiveKind,
    /// The limit it sets (m/s; `None`: the kind's own, or none).
    pub speed_mps: Option<f32>,
}

impl EdgeEvent {
    /// The limit in force by this event (m/s), if it sets one.
    pub fn speed(&self) -> Option<f32> {
        self.speed_mps.or(self.kind.default_speed())
    }
}

/// Applies `ev` to the rule of its edge (`rule` is the statistics' rule or
/// one with other events already applied).
pub fn apply(rule: &mut EdgeRule, ev: &EdgeEvent) {
    let dirs: &[usize] = match ev.dir {
        Dir::Both => &[0, 1],
        Dir::Forward => &[0],
        Dir::Backward => &[1],
    };
    match ev.kind {
        LiveKind::Closed => {
            for &d in dirs {
                rule.lanes[d] = 0;
            }
            if rule.lanes == [0, 0] {
                rule.access = Access::NONE;
            } else {
                rule.oneway = if rule.lanes[0] > 0 {
                    Oneway::Forward
                } else {
                    Oneway::Backward
                };
            }
        }
        LiveKind::LaneClosed | LiveKind::Obstruction => {
            for &d in dirs {
                if rule.lanes[d] > 1 {
                    rule.lanes[d] -= 1;
                }
            }
        }
        LiveKind::Jam => rule.aadt *= JAM_DEMAND,
        LiveKind::Roadworks | LiveKind::SpeedLimit => {}
    }
    if let Some(v) = ev.speed() {
        let v = f64::from(v.max(JAM_STATIONARY_KMH / 3.6));
        if v < rule.speed {
            rule.speed = v;
        }
    }
}

/// A live record as it lies on the globe (the source's parser makes it).
#[derive(Debug, Clone, PartialEq)]
pub struct GeoEvent {
    pub kind: LiveKind,
    /// The limit the record sets (km/h), if it gives one.
    pub speed_kmh: Option<f32>,
    /// The road's name in the record ("TF-5").
    pub road: String,
    /// Where it starts (lat, lon) in its direction of travel …
    pub from: (f64, f64),
    /// … and ends (`None`: a point).
    pub to: Option<(f64, f64)>,
    /// The road's length between the two by its kilometre points (m).
    pub length_m: Option<f64>,
    /// Only the direction of travel from `from` to `to` (else both); for a
    /// point, its compass bearing of travel (degrees, 0 = north).
    pub one_direction: bool,
    pub bearing_deg: Option<f64>,
}

/// Why a record has no edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unmapped {
    /// No motor road within [`SNAP_M`] of its point.
    NoRoad,
    /// No road between its two points as long as the record says.
    NoPath,
}

/// The nearest motor road to `p` (x, z) within `radius`: (edge, metres
/// along its polyline, distance); a bigger road wins over a nearer small
/// one by 4 m a rank (a record names a main road, its point is rounded).
pub fn snap(
    net: &RoadNetwork,
    rules: &[EdgeRule],
    p: [f64; 2],
    radius: f64,
) -> Option<(usize, f64, f64)> {
    let mut best: Option<(f64, usize, f64, f64)> = None;
    for e in net.edges_near(p, radius) {
        if rules.get(e).is_none_or(|r| r.access.is_none()) {
            continue;
        }
        let pts = &net.edge(e).points;
        let mut along = 0.0;
        for w in pts.windows(2) {
            let (a, b) = (w[0], w[1]);
            let ab = [b[0] - a[0], b[1] - a[1]];
            let len2 = ab[0] * ab[0] + ab[1] * ab[1];
            let len = len2.sqrt();
            let t = if len2 > 0.0 {
                (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / len2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let d = ((a[0] + ab[0] * t - p[0]).powi(2) + (a[1] + ab[1] * t - p[1]).powi(2)).sqrt();
            if d <= radius {
                let score = d - 4.0 * f64::from(rules[e].rank);
                if best.is_none_or(|(s, ..)| score < s) {
                    best = Some((score, e, along + len * t, d));
                }
            }
            along += len;
        }
    }
    best.map(|(_, e, s, d)| (e, s, d))
}

fn polyline_len(pts: &[[f64; 2]]) -> f64 {
    pts.windows(2)
        .map(|w| ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt())
        .sum()
}

/// The road from (edge `a`, `sa` m along) to (edge `b`, `sb`): the edges
/// with their direction of travel, by the shortest way over motor roads
/// (one-way roads only their way when `directed`), at most `max_m` long.
fn follow(
    net: &RoadNetwork,
    rules: &[EdgeRule],
    (a, sa): (usize, f64),
    (b, sb): (usize, f64),
    directed: bool,
    max_m: f64,
) -> Option<Vec<(usize, bool)>> {
    use std::cmp::Reverse;
    use std::collections::{BTreeMap, BinaryHeap};
    let len = |e: usize| polyline_len(&net.edge(e).points);
    if a == b {
        let fwd = sb >= sa;
        let ok = !directed || rules[a].oneway.allows(fwd);
        return ((sb - sa).abs() <= max_m && ok).then(|| vec![(a, fwd)]);
    }
    let usable = |e: usize, fwd: bool| {
        let r = &rules[e];
        !r.access.is_none() && (!directed || r.oneway.allows(fwd))
    };
    // Dijkstra over vertices from the start edge's ends (the part of edge
    // `a` to each end), into either end of edge `b`.
    let ea = net.edge(a);
    let la = len(a);
    let mut dist: BTreeMap<u32, f64> = BTreeMap::new();
    let mut prev: BTreeMap<u32, (usize, bool)> = BTreeMap::new();
    let mut heap = BinaryHeap::new();
    let key = |c: f64| Reverse((c * 100.0) as u64);
    for (v, c, fwd) in [(ea.to, la - sa, true), (ea.from, sa, false)] {
        if usable(a, fwd) && dist.get(&v.0).is_none_or(|d| c < *d) {
            dist.insert(v.0, c);
            prev.insert(v.0, (a, fwd));
            heap.push((key(c), v.0));
        }
    }
    let eb = net.edge(b);
    let lb = len(b);
    let mut best: Option<(f64, u32, bool)> = None;
    while let Some((Reverse(k), v)) = heap.pop() {
        let c = k as f64 / 100.0;
        if dist.get(&v).is_some_and(|d| c > *d + 0.01) || c > max_m {
            continue;
        }
        // Into edge b at this vertex?
        for (end, fwd, rest) in [(eb.from.0, true, sb), (eb.to.0, false, lb - sb)] {
            if end == v && usable(b, fwd) && best.is_none_or(|(bc, ..)| c + rest < bc) {
                best = Some((c + rest, v, fwd));
            }
        }
        for &e in net.edges_at(ocw_world::graph::VertexId(v)) {
            if e == a || e == b {
                continue;
            }
            let ed = net.edge(e);
            for fwd in [true, false] {
                let (start, end) = if fwd {
                    (ed.from, ed.to)
                } else {
                    (ed.to, ed.from)
                };
                if start.0 != v || !usable(e, fwd) {
                    continue;
                }
                let nc = c + len(e);
                if nc <= max_m && dist.get(&end.0).is_none_or(|d| nc < *d) {
                    dist.insert(end.0, nc);
                    prev.insert(end.0, (e, fwd));
                    heap.push((key(nc), end.0));
                }
            }
        }
    }
    let (total, mut v, fwd_b) = best?;
    if total > max_m {
        return None;
    }
    let mut out = vec![(b, fwd_b)];
    loop {
        let (e, fwd) = *prev.get(&v)?;
        out.push((e, fwd));
        if e == a {
            break;
        }
        v = if fwd {
            net.edge(e).from.0
        } else {
            net.edge(e).to.0
        };
    }
    out.reverse();
    Some(out)
}

/// The compass bearing (degrees) of an edge's travel direction at `s`.
fn bearing_at(net: &RoadNetwork, e: usize, s: f64, forward: bool) -> f64 {
    let pts = &net.edge(e).points;
    let mut along = 0.0;
    let mut seg = (pts[0], pts[pts.len().saturating_sub(1)]);
    for w in pts.windows(2) {
        let l = ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt();
        seg = (w[0], w[1]);
        if along + l >= s {
            break;
        }
        along += l;
    }
    let (a, b) = if forward { seg } else { (seg.1, seg.0) };
    // x east, z south.
    let (east, north) = (b[0] - a[0], -(b[1] - a[1]));
    east.atan2(north).to_degrees().rem_euclid(360.0)
}

/// Maps `events` onto `net` (local metres of the package's origin
/// `origin`, lat / lon): for each record its edge events, or why none.
/// `rules` are the statistics' rules (only motor roads are matched).
pub fn map_events(
    net: &RoadNetwork,
    rules: &[EdgeRule],
    origin: (f64, f64),
    events: &[GeoEvent],
) -> Vec<Result<Vec<EdgeEvent>, Unmapped>> {
    let proj = LocalProjection::new(origin.0, origin.1);
    events
        .iter()
        .map(|g| {
            let speed_mps = g
                .speed_kmh
                .filter(|v| *v >= MIN_LIMIT_KMH || g.kind == LiveKind::Jam)
                .map(|v| v / 3.6);
            if g.kind == LiveKind::SpeedLimit && speed_mps.is_none() {
                return Ok(Vec::new());
            }
            let pa = proj.project(g.from.0, g.from.1);
            let (a, sa, _) = snap(net, rules, pa, SNAP_M).ok_or(Unmapped::NoRoad)?;
            let ev = |edge: usize, dir: Dir| EdgeEvent {
                edge: edge as u32,
                dir,
                kind: g.kind,
                speed_mps,
            };
            let Some(to) = g.to else {
                let dir = match g.bearing_deg {
                    Some(b) if g.one_direction => {
                        let f = bearing_at(net, a, sa, true);
                        let d = (f - b + 540.0).rem_euclid(360.0) - 180.0;
                        Dir::of(d.abs() <= 90.0)
                    }
                    _ => Dir::Both,
                };
                return Ok(vec![ev(a, dir)]);
            };
            let pb = proj.project(to.0, to.1);
            let (b, sb, _) = snap(net, rules, pb, SNAP_M).ok_or(Unmapped::NoRoad)?;
            let straight = ((pb[0] - pa[0]).powi(2) + (pb[1] - pa[1]).powi(2)).sqrt();
            let max_m = (2.0 * straight).max(g.length_m.map_or(0.0, |l| l * 1.4)) + 300.0;
            let path = follow(net, rules, (a, sa), (b, sb), g.one_direction, max_m)
                .or_else(|| {
                    // A one-way carriageway snapped the wrong way round:
                    // the road between the two, both ways.
                    g.one_direction
                        .then(|| follow(net, rules, (a, sa), (b, sb), false, max_m))
                        .flatten()
                })
                .ok_or(Unmapped::NoPath)?;
            let mut out: Vec<EdgeEvent> = Vec::with_capacity(path.len());
            for (e, fwd) in path {
                let dir = if g.one_direction {
                    Dir::of(fwd)
                } else {
                    Dir::Both
                };
                if !out.iter().any(|x| x.edge == e as u32 && x.dir == dir) {
                    out.push(ev(e, dir));
                }
            }
            Ok(out)
        })
        .collect()
}

/// Whether `events` close (edge, direction).
pub fn closed(events: &[EdgeEvent], edge: usize, forward: bool) -> bool {
    events
        .iter()
        .any(|e| e.kind == LiveKind::Closed && e.edge as usize == edge && e.dir.holds(forward))
}
