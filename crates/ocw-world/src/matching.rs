//! Map matching (spec §2.7, step 3): snaps a GPS track onto the road
//! graph with a hidden Markov model and the Viterbi algorithm (the
//! approach of Newson & Krumm, used by OSRM and Valhalla).
//!
//! - **States** are points on edges near a GPS point, with a riding
//!   direction (one-way roads only one way): up to [`MatchParams::candidates`]
//!   edges within [`MatchParams::search_radius_m`].
//! - **Emission:** Gaussian in the distance to the road (σ =
//!   [`MatchParams::sigma_m`]).
//! - **Transition:** exponential in the difference between the distance
//!   along the roads and the straight distance between the two GPS points
//!   (β = [`MatchParams::beta_m`]); the road distance comes from a bounded
//!   Dijkstra over riding directions. A U-turn on an edge costs
//!   [`MatchParams::uturn_penalty_m`] extra, so it is only chosen where the
//!   track really turns back (out-and-back routes, spec §2.2.2).
//! - **Gaps and outliers:** points without a road nearby, or that no road
//!   connects to the previous point, are skipped and reported; a gap the
//!   graph cannot bridge within [`MatchParams::max_gap_m`] is an error.
//!
//! The result is the ordinary route format ([`RoutePlan`]) plus a report of
//! the stretches more than [`MatchParams::off_road_m`] from the matched
//! road (spec §2.7 step 5: problems are reported, not hidden).

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

use crate::geo::LocalProjection;
use crate::gpx::Gpx;
use crate::graph::VertexId;
use crate::graph_route::{RouteBreak, RoutePlan};
use crate::network::RoadNetwork;
use crate::path::{Segment, TurnReason};

/// Tuning of the matcher.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MatchParams {
    /// Roads further than this from a GPS point are no candidates (m).
    pub search_radius_m: f64,
    /// At most this many nearest edges per point.
    pub candidates: usize,
    /// GPS noise (m).
    pub sigma_m: f64,
    /// Tolerance of road distance vs straight distance (m).
    pub beta_m: f64,
    /// Extra cost of turning back on a road (m).
    pub uturn_penalty_m: f64,
    /// Points closer than this to the previous one are thinned out (m).
    pub resample_m: f64,
    /// Matched points further than this from their road are reported (m).
    pub off_road_m: f64,
    /// Largest stretch of skipped points before matching fails (m).
    pub max_gap_m: f64,
    /// Backwards jitter on one road that still counts as standing (m).
    pub backtrack_m: f64,
}

impl Default for MatchParams {
    fn default() -> Self {
        Self {
            search_radius_m: 50.0,
            candidates: 6,
            sigma_m: 10.0,
            beta_m: 25.0,
            uturn_penalty_m: 60.0,
            resample_m: 10.0,
            off_road_m: 30.0,
            max_gap_m: 1500.0,
            backtrack_m: 8.0,
        }
    }
}

/// A stretch of the GPS track away from the matched roads.
#[derive(Debug, Clone, PartialEq)]
pub struct OffRoad {
    /// First and last GPS point index (in the input).
    pub first: usize,
    pub last: usize,
    /// Largest distance to the matched road (m); infinite where no road was
    /// within the search radius.
    pub max_distance_m: f64,
}

/// Result of matching a track.
#[derive(Debug, Clone)]
pub struct MatchReport {
    pub plan: RoutePlan,
    /// Stretches more than [`MatchParams::off_road_m`] from the roads.
    pub off_road: Vec<OffRoad>,
    /// GPS points given, and used after thinning and skipping.
    pub points: usize,
    pub used: usize,
    /// U-turns in the route.
    pub uturns: usize,
    /// Length of the GPS track (m).
    pub gps_length_m: f64,
}

impl MatchReport {
    /// Human-readable summary (for world-builder and the console).
    pub fn summary(&self) -> String {
        let mut s = format!(
            "{}: {:.2} km on the road graph ({} steps{}{}), GPS track {:.2} km, {} of {} points used",
            self.plan.name,
            self.plan.length_m() / 1000.0,
            self.plan.steps.len(),
            if self.uturns > 0 {
                format!(", {} U-turns", self.uturns)
            } else {
                String::new()
            },
            if self.plan.is_loop { ", a loop" } else { "" },
            self.gps_length_m / 1000.0,
            self.used,
            self.points
        );
        for o in &self.off_road {
            if o.max_distance_m.is_finite() {
                s.push_str(&format!(
                    "\n  off the roads: points {}-{} (up to {:.0} m away)",
                    o.first, o.last, o.max_distance_m
                ));
            } else {
                s.push_str(&format!(
                    "\n  no road nearby: points {}-{} (skipped)",
                    o.first, o.last
                ));
            }
        }
        s
    }
}

/// Why a track could not be matched.
#[derive(Debug, Clone, PartialEq)]
pub enum MatchError {
    /// Fewer than two usable points.
    TooFewPoints,
    /// No point is near any road (another region?).
    NoRoadNearby,
    /// The roads do not connect the track around this point (input index).
    Broken { point: usize },
    /// Internal: the matched steps do not form a valid route.
    Invalid(RouteBreak),
}

impl std::fmt::Display for MatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooFewPoints => write!(f, "the track needs at least two points"),
            Self::NoRoadNearby => write!(
                f,
                "no road of this world package is near the track (another region?)"
            ),
            Self::Broken { point } => write!(
                f,
                "the roads of this package do not connect the track near point {point}"
            ),
            Self::Invalid(b) => write!(f, "matching produced an invalid route: {b}"),
        }
    }
}

impl std::error::Error for MatchError {}

/// Matches a GPX file (projected around `origin`, the package's lat/lon).
pub fn match_gpx(
    net: &RoadNetwork,
    gpx: &Gpx,
    origin: (f64, f64),
    name: &str,
    params: &MatchParams,
) -> Result<MatchReport, MatchError> {
    let proj = LocalProjection::new(origin.0, origin.1);
    let pts: Vec<[f64; 2]> = gpx
        .points
        .iter()
        .map(|p| proj.project(p.lat, p.lon))
        .collect();
    let mut r = match_track(net, &pts, name, params)?;
    r.gps_length_m = gpx.length_m();
    Ok(r)
}

/// How a stored route came through a package rebuild (spec §4.4: "routes
/// are re-matched automatically and the world builder flags any that
/// break").
#[derive(Debug, Clone)]
pub enum CarriedOver {
    /// All its stable edge IDs are still there and join up.
    Kept(RoutePlan),
    /// It broke (`why`) and was re-matched from its old geometry.
    Rematched {
        why: RouteBreak,
        report: Box<MatchReport>,
    },
}

/// Carries `route` from the old package's graph (`old`, local metres
/// around `old_origin`) over to a rebuilt one (`new`, around `new_origin`):
/// by stable edge ID if possible, otherwise by matching its old geometry.
/// Errors when neither works (the route must be redone).
pub fn carry_over(
    route: &crate::graph_route::GraphRoute,
    old: &RoadNetwork,
    old_origin: (f64, f64),
    new: &RoadNetwork,
    new_origin: (f64, f64),
) -> Result<CarriedOver, String> {
    let why = match route.resolve(new) {
        Ok(plan) => return Ok(CarriedOver::Kept(plan)),
        Err(why) => why,
    };
    let old_plan = route
        .resolve(old)
        .map_err(|e| format!("{why}; and it does not fit the old graph either: {e}"))?;
    let (po, pn) = (
        LocalProjection::new(old_origin.0, old_origin.1),
        LocalProjection::new(new_origin.0, new_origin.1),
    );
    let pts: Vec<[f64; 2]> = old_plan
        .points(old, 10.0)
        .into_iter()
        .map(|p| {
            let (lat, lon) = po.unproject(p);
            pn.project(lat, lon)
        })
        .collect();
    let params = MatchParams {
        // The old geometry is exact, not GPS: tighter.
        sigma_m: 4.0,
        ..MatchParams::default()
    };
    let mut report = match_track(new, &pts, &route.name, &params)
        .map_err(|e| format!("{why}; re-matching failed: {e}"))?;
    if route.is_loop && report.plan.closes(new) && !report.plan.is_loop {
        report.plan = RoutePlan::new(new, &route.name, report.plan.steps.clone(), true)
            .map_err(|e| e.to_string())?;
    }
    Ok(CarriedOver::Rematched {
        why,
        report: Box::new(report),
    })
}

/// A state of the HMM: a point on an edge with a riding direction.
#[derive(Debug, Clone, Copy)]
struct State {
    edge: usize,
    s: f64,
    forward: bool,
    dist: f64,
}

/// How the route gets from one state to the next.
#[derive(Debug, Clone)]
enum Hop {
    /// Further along the same edge.
    Along,
    /// Back on the same edge, turning at `at` metres.
    UTurn { at: f64 },
    /// To the end of the edge, then these whole edges, then into the next.
    Via(Vec<(usize, bool)>),
}

struct Layer {
    /// Index of the GPS point (input).
    point: usize,
    states: Vec<State>,
    score: Vec<f64>,
    back: Vec<Option<(usize, Hop)>>,
}

/// Matches a track of local points (x, z) onto `net`.
pub fn match_track(
    net: &RoadNetwork,
    points: &[[f64; 2]],
    name: &str,
    params: &MatchParams,
) -> Result<MatchReport, MatchError> {
    let index = EdgeIndex::new(net, params.search_radius_m);
    // Thin out dense points.
    let mut kept: Vec<usize> = Vec::new();
    for (i, p) in points.iter().enumerate() {
        match kept.last() {
            Some(&k) if dist2(points[k], *p).sqrt() < params.resample_m => {}
            _ => kept.push(i),
        }
    }
    if let (Some(&last), true) = (kept.last(), points.len() > 1)
        && last != points.len() - 1
    {
        kept.push(points.len() - 1);
    }
    if kept.len() < 2 {
        return Err(MatchError::TooFewPoints);
    }
    let mut skipped: Vec<usize> = Vec::new();
    let mut layers: Vec<Layer> = Vec::new();
    // Skipped GPS distance since the last matched point.
    let mut gap_m = 0.0;
    for &i in &kept {
        let states = candidates(net, &index, points[i], params);
        if states.is_empty() {
            skipped.push(i);
            if let Some(prev) = layers.last() {
                gap_m += dist2(points[prev.point], points[i]).sqrt();
            }
            continue;
        }
        let emission = |s: &State| -0.5 * (s.dist / params.sigma_m).powi(2);
        let Some(prev) = layers.last() else {
            layers.push(Layer {
                point: i,
                score: states.iter().map(emission).collect(),
                back: vec![None; states.len()],
                states,
            });
            continue;
        };
        let gc = dist2(points[prev.point], points[i]).sqrt();
        let bound = gc * 3.0 + 300.0;
        let mut score = vec![f64::NEG_INFINITY; states.len()];
        let mut back: Vec<Option<(usize, Hop)>> = vec![None; states.len()];
        for (ai, a) in prev.states.iter().enumerate() {
            if !prev.score[ai].is_finite() {
                continue;
            }
            let reach = Reach::from(net, a, bound);
            for (bi, b) in states.iter().enumerate() {
                let Some((d, hop)) = hop(net, a, b, &reach, params) else {
                    continue;
                };
                let p = prev.score[ai] - (d - gc).abs() / params.beta_m + emission(b);
                if p > score[bi] {
                    score[bi] = p;
                    back[bi] = Some((ai, hop));
                }
            }
        }
        if score.iter().all(|s| !s.is_finite()) {
            // No road connects this point: an outlier, or a gap.
            skipped.push(i);
            gap_m += gc;
            if gap_m > params.max_gap_m {
                return Err(MatchError::Broken { point: i });
            }
            continue;
        }
        gap_m = 0.0;
        layers.push(Layer {
            point: i,
            states,
            score,
            back,
        });
    }
    if layers.is_empty() {
        return Err(MatchError::NoRoadNearby);
    }
    if layers.len() < 2 {
        return Err(MatchError::TooFewPoints);
    }
    // Backtrack the best sequence.
    let last = layers.last().expect("not empty");
    let mut si = (0..last.states.len())
        .max_by(|&a, &b| last.score[a].total_cmp(&last.score[b]))
        .expect("states");
    let mut chosen: Vec<(State, Option<Hop>)> = Vec::with_capacity(layers.len());
    for layer in layers.iter().rev() {
        let back = layer.back[si].clone();
        chosen.push((layer.states[si], back.as_ref().map(|b| b.1.clone())));
        if let Some((prev, _)) = back {
            si = prev;
        }
    }
    chosen.reverse();
    let mut steps = build_steps(net, &chosen);
    // A track that ends where it started: a loop, starting at the vertex.
    let mut is_loop = false;
    let (first_p, last_p) = (points[kept[0]], points[*kept.last().expect("kept")]);
    if dist2(first_p, last_p).sqrt() < 50.0 && steps.len() >= 3 {
        let (
            Segment::Edge {
                edge: e0,
                from_m: f0,
                to_m: t0,
            },
            Segment::Edge {
                edge: el,
                from_m: fl,
                to_m: tl,
            },
        ) = (steps[0], steps[steps.len() - 1])
        else {
            unreachable!("routes start and end on edges");
        };
        let len = net.geometry(e0).length();
        let fwd = t0 >= f0;
        let at_start = if fwd { fl <= 0.5 } else { fl >= len - 0.5 };
        if el == e0 && (tl >= fl) == fwd && at_start {
            steps.pop();
            steps[0] = Segment::Edge {
                edge: e0,
                from_m: if fwd { 0.0 } else { len },
                to_m: t0,
            };
            is_loop = true;
        }
    }
    let plan = RoutePlan::new(net, name, steps, false).map_err(MatchError::Invalid)?;
    let plan = if is_loop && plan.closes(net) {
        RoutePlan::new(net, name, plan.steps, true).map_err(MatchError::Invalid)?
    } else {
        plan
    };
    // Report: points far from their road, and skipped ones.
    let mut far: Vec<(usize, f64)> = chosen
        .iter()
        .zip(&layers)
        .filter(|(c, _)| c.0.dist > params.off_road_m)
        .map(|(c, l)| (l.point, c.0.dist))
        .collect();
    far.extend(skipped.iter().map(|&i| (i, f64::INFINITY)));
    far.sort_by_key(|f| f.0);
    let off_road = group_off_road(&far, &kept);
    let uturns = plan
        .steps
        .iter()
        .filter(|s| matches!(s, Segment::Turnaround(_)))
        .count();
    let gps_length_m = points.windows(2).map(|w| dist2(w[0], w[1]).sqrt()).sum();
    Ok(MatchReport {
        plan,
        off_road,
        points: points.len(),
        used: layers.len(),
        uturns,
        gps_length_m,
    })
}

/// Groups far / skipped points that follow each other (in `kept` order)
/// into stretches.
fn group_off_road(far: &[(usize, f64)], kept: &[usize]) -> Vec<OffRoad> {
    let pos: HashMap<usize, usize> = kept.iter().enumerate().map(|(k, &i)| (i, k)).collect();
    let mut out: Vec<OffRoad> = Vec::new();
    let mut last_k: Option<usize> = None;
    for &(i, d) in far {
        let k = pos[&i];
        match (out.last_mut(), last_k) {
            (Some(o), Some(lk)) if k == lk + 1 => {
                o.last = i;
                o.max_distance_m = o.max_distance_m.max(d);
            }
            _ => out.push(OffRoad {
                first: i,
                last: i,
                max_distance_m: d,
            }),
        }
        last_k = Some(k);
    }
    out
}

/// The route steps through the chosen states.
fn build_steps(net: &RoadNetwork, chosen: &[(State, Option<Hop>)]) -> Vec<Segment> {
    let mut pieces: Vec<Segment> = Vec::new();
    let first = chosen[0].0;
    let (mut edge, mut s, mut forward) = (first.edge, first.s, first.forward);
    let edge_piece = |edge: usize, from_m: f64, to_m: f64| Segment::Edge { edge, from_m, to_m };
    for (b, hop) in &chosen[1..] {
        let len = net.geometry(edge).length();
        match hop.as_ref().expect("every later state has a hop") {
            Hop::Along => {
                let to = if forward { s.max(b.s) } else { s.min(b.s) };
                pieces.push(edge_piece(edge, s, to));
                s = to;
            }
            Hop::UTurn { at } => {
                // Never behind where the route already got to.
                let at = if forward { s.max(*at) } else { s.min(*at) };
                let to = if forward { b.s.min(at) } else { b.s.max(at) };
                pieces.push(edge_piece(edge, s, at));
                pieces.push(Segment::Turnaround(TurnReason::Manual));
                pieces.push(edge_piece(edge, at, to));
                (s, forward) = (to, !forward);
            }
            Hop::Via(path) => {
                pieces.push(edge_piece(edge, s, if forward { len } else { 0.0 }));
                for &(e, f) in path {
                    pieces.push(Segment::whole(net, e, f));
                }
                let lb = net.geometry(b.edge).length();
                pieces.push(edge_piece(b.edge, if b.forward { 0.0 } else { lb }, b.s));
                (edge, s, forward) = (b.edge, b.s, b.forward);
            }
        }
    }
    merge(pieces)
}

/// Joins pieces along the same edge and drops empty ones (keeping each
/// U-turn between its two halves).
fn merge(pieces: Vec<Segment>) -> Vec<Segment> {
    const EMPTY_M: f64 = 0.05;
    let mut out: Vec<Segment> = Vec::new();
    for p in pieces {
        if let (
            Some(Segment::Edge {
                edge: pe,
                from_m: pf,
                to_m: pt,
            }),
            Segment::Edge { edge, from_m, to_m },
        ) = (out.last_mut(), p)
        {
            let (a, b) = (*pt - *pf, to_m - from_m);
            let same_way = a.abs() < EMPTY_M || b.abs() < EMPTY_M || (a > 0.0) == (b > 0.0);
            if *pe == edge && same_way && (from_m - *pt).abs() < 0.5 {
                *pt = to_m;
                continue;
            }
        }
        out.push(p);
    }
    out.retain(
        |s| !matches!(s, Segment::Edge { from_m, to_m, .. } if (to_m - from_m).abs() < EMPTY_M),
    );
    // A U-turn stays only between two pieces of its edge (rare otherwise: a
    // turn exactly at a vertex, where the next edge just starts there).
    let mut cleaned: Vec<Segment> = Vec::new();
    for (i, s) in out.iter().enumerate() {
        if let Segment::Turnaround(_) = s {
            let ok = matches!(
                (cleaned.last(), out.get(i + 1)),
                (Some(Segment::Edge { edge: a, .. }), Some(Segment::Edge { edge: b, .. })) if a == b
            );
            if !ok {
                continue;
            }
        }
        cleaned.push(*s);
    }
    cleaned
}

/// Road distance and hop from state `a` to `b`, if within reach.
fn hop(
    net: &RoadNetwork,
    a: &State,
    b: &State,
    reach: &Reach,
    params: &MatchParams,
) -> Option<(f64, Hop)> {
    let mut best: Option<(f64, Hop)> = None;
    let mut offer = |d: f64, h: Hop| {
        if best.as_ref().is_none_or(|(bd, _)| d < *bd) {
            best = Some((d, h));
        }
    };
    if a.edge == b.edge {
        let sign = if a.forward { 1.0 } else { -1.0 };
        if a.forward == b.forward {
            let progress = (b.s - a.s) * sign;
            if progress >= -params.backtrack_m {
                // Jitter backwards is allowed but not free (or a rider
                // could drift back along a road without turning).
                let d = if progress >= 0.0 {
                    progress
                } else {
                    -3.0 * progress
                };
                offer(d, Hop::Along);
            }
        } else {
            let at = if a.forward {
                a.s.max(b.s)
            } else {
                a.s.min(b.s)
            };
            offer(
                (at - a.s).abs() + (at - b.s).abs() + params.uturn_penalty_m,
                Hop::UTurn { at },
            );
        }
    }
    // Via the graph: off the end of a's edge, into b's edge at its start.
    let la = net.geometry(a.edge).length();
    let lb = net.geometry(b.edge).length();
    let ra = if a.forward { la - a.s } else { a.s };
    let rb = if b.forward { b.s } else { lb - b.s };
    let vb = net.start_vertex(b.edge, b.forward);
    if let Some((d, path)) = reach.path_to(vb) {
        // Not straight back along the edge just arrived on.
        let last = path.last().copied().unwrap_or((a.edge, a.forward));
        if last != (b.edge, !b.forward) {
            offer(ra + d + rb, Hop::Via(path));
        }
    }
    best
}

/// How a vertex was reached: (edge, forward, previous vertex).
type Came = (usize, bool, u32);

/// Bounded Dijkstra from the end of a state's edge (in its direction).
struct Reach {
    start: VertexId,
    dist: HashMap<u32, (f64, Option<Came>)>,
}

impl Reach {
    fn from(net: &RoadNetwork, a: &State, bound: f64) -> Self {
        let start = net.end_vertex(a.edge, a.forward);
        let forbid = (a.edge, !a.forward);
        let mut dist: HashMap<u32, (f64, Option<Came>)> = HashMap::new();
        dist.insert(start.0, (0.0, None));
        let mut heap = BinaryHeap::new();
        heap.push(Reverse((0u64, start.0)));
        while let Some(Reverse((c, v))) = heap.pop() {
            let c = c as f64 / 1000.0;
            if c > bound || dist.get(&v).is_some_and(|d| c > d.0 + 1e-6) {
                continue;
            }
            let vid = VertexId(v);
            for &e in net.edges_at(vid) {
                let edge = net.edge(e);
                for forward in [true, false] {
                    if net.start_vertex(e, forward) != vid || !edge.oneway.allows(forward) {
                        continue;
                    }
                    if v == start.0 && (e, forward) == forbid {
                        continue;
                    }
                    let o = net.end_vertex(e, forward).0;
                    let nc = c + net.geometry(e).length();
                    if nc <= bound && dist.get(&o).is_none_or(|d| nc < d.0) {
                        dist.insert(o, (nc, Some((e, forward, v))));
                        heap.push(Reverse(((nc * 1000.0) as u64, o)));
                    }
                }
            }
        }
        Self { start, dist }
    }

    /// Distance to `v` and the whole edges ridden there.
    fn path_to(&self, v: VertexId) -> Option<(f64, Vec<(usize, bool)>)> {
        let (d, _) = *self.dist.get(&v.0)?;
        let mut path = Vec::new();
        let mut u = v.0;
        while u != self.start.0 {
            let (_, came) = self.dist[&u];
            let Some((e, f, prev)) = came else { break };
            path.push((e, f));
            u = prev;
        }
        path.reverse();
        Some((d, path))
    }
}

/// Candidate states near `p`: the nearest point of each edge within the
/// search radius (the closest edges first), in each allowed direction.
fn candidates(
    net: &RoadNetwork,
    index: &EdgeIndex,
    p: [f64; 2],
    params: &MatchParams,
) -> Vec<State> {
    let mut best: HashMap<usize, (f64, f64)> = HashMap::new();
    for (edge, k) in index.near(p, params.search_radius_m) {
        let g = net.geometry(edge);
        let (a, b) = (g.points[k], g.points[k + 1]);
        let (a2, b2) = ([a[0], a[2]], [b[0], b[2]]);
        let ab = [b2[0] - a2[0], b2[1] - a2[1]];
        let len2 = ab[0] * ab[0] + ab[1] * ab[1];
        let t = if len2 > 0.0 {
            (((p[0] - a2[0]) * ab[0] + (p[1] - a2[1]) * ab[1]) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let c = [a2[0] + ab[0] * t, a2[1] + ab[1] * t];
        let d = dist2(c, p).sqrt();
        if d > params.search_radius_m {
            continue;
        }
        let s = g.cumulative[k] + (g.cumulative[k + 1] - g.cumulative[k]) * t;
        if best.get(&edge).is_none_or(|x| d < x.0) {
            best.insert(edge, (d, s));
        }
    }
    let mut near: Vec<(usize, f64, f64)> = best.into_iter().map(|(e, (d, s))| (e, d, s)).collect();
    near.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
    near.truncate(params.candidates);
    let mut out = Vec::new();
    for (edge, dist, s) in near {
        for forward in [true, false] {
            if net.edge(edge).oneway.allows(forward) {
                out.push(State {
                    edge,
                    s,
                    forward,
                    dist,
                });
            }
        }
    }
    out
}

/// Grid of edge geometry segments for the candidate search.
struct EdgeIndex {
    cell: f64,
    cells: HashMap<(i32, i32), Vec<(usize, usize)>>,
}

impl EdgeIndex {
    fn new(net: &RoadNetwork, cell: f64) -> Self {
        let cell = cell.max(10.0);
        let mut cells: HashMap<(i32, i32), Vec<(usize, usize)>> = HashMap::new();
        for e in 0..net.graph().edges.len() {
            let g = net.geometry(e);
            for (k, w) in g.points.windows(2).enumerate() {
                let (x0, x1) = (w[0][0].min(w[1][0]), w[0][0].max(w[1][0]));
                let (z0, z1) = (w[0][2].min(w[1][2]), w[0][2].max(w[1][2]));
                for cx in (x0 / cell).floor() as i32..=(x1 / cell).floor() as i32 {
                    for cz in (z0 / cell).floor() as i32..=(z1 / cell).floor() as i32 {
                        cells.entry((cx, cz)).or_default().push((e, k));
                    }
                }
            }
        }
        Self { cell, cells }
    }

    /// Segments in the cells within `r` of `p` (may contain duplicates).
    fn near(&self, p: [f64; 2], r: f64) -> impl Iterator<Item = (usize, usize)> + '_ {
        let (x0, x1) = (
            ((p[0] - r) / self.cell).floor() as i32,
            ((p[0] + r) / self.cell).floor() as i32,
        );
        let (z0, z1) = (
            ((p[1] - r) / self.cell).floor() as i32,
            ((p[1] + r) / self.cell).floor() as i32,
        );
        (x0..=x1)
            .flat_map(move |cx| (z0..=z1).map(move |cz| (cx, cz)))
            .filter_map(|c| self.cells.get(&c))
            .flatten()
            .copied()
    }
}

fn dist2(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{EdgeId, RoadGraph};
    use crate::network::tests::graph;

    /// A grid of 4 × 4 vertices 200 m apart (x = 200·col, z = 200·row).
    fn grid() -> RoadNetwork {
        let mut v = Vec::new();
        for row in 0..4 {
            for col in 0..4 {
                v.push([f64::from(col) * 200.0, f64::from(row) * 200.0]);
            }
        }
        let mut e = Vec::new();
        for row in 0..4u32 {
            for col in 0..4u32 {
                let i = row * 4 + col;
                if col < 3 {
                    e.push((i, i + 1));
                }
                if row < 3 {
                    e.push((i, i + 4));
                }
            }
        }
        RoadNetwork::new(graph(&v, &e))
    }

    /// Points every `step` m along a polyline, with deterministic noise of
    /// up to `noise` m sideways.
    fn sample(line: &[[f64; 2]], step: f64, noise: f64) -> Vec<[f64; 2]> {
        let mut out = Vec::new();
        let mut k = 0u64;
        for w in line.windows(2) {
            let len = dist2(w[0], w[1]).sqrt();
            let n = (len / step).ceil() as usize;
            for i in 0..n {
                let t = i as f64 / n as f64;
                k += 1;
                let r = crate::network::splitmix(k);
                let jx = ((r & 0xffff) as f64 / 65535.0 - 0.5) * 2.0 * noise;
                let jz = (((r >> 16) & 0xffff) as f64 / 65535.0 - 0.5) * 2.0 * noise;
                out.push([
                    w[0][0] + (w[1][0] - w[0][0]) * t + jx,
                    w[0][1] + (w[1][1] - w[0][1]) * t + jz,
                ]);
            }
        }
        out.push(*line.last().unwrap());
        out
    }

    /// Edge index between two vertices (either direction) and whether it
    /// runs from `a` to `b`.
    fn edge(net: &RoadNetwork, a: u32, b: u32) -> (usize, bool) {
        (0..net.graph().edges.len())
            .find_map(|i| {
                let e = net.edge(i);
                if e.from.0 == a && e.to.0 == b {
                    Some((i, true))
                } else if e.from.0 == b && e.to.0 == a {
                    Some((i, false))
                } else {
                    None
                }
            })
            .unwrap()
    }

    fn edges_of(plan: &RoutePlan) -> Vec<(usize, bool)> {
        plan.steps
            .iter()
            .filter_map(|s| match *s {
                Segment::Edge { edge, from_m, to_m } => Some((edge, to_m >= from_m)),
                Segment::Turnaround(_) => None,
            })
            .collect()
    }

    #[test]
    fn a_noisy_track_matches_the_roads_it_rode() {
        let net = grid();
        // From 50 m east of vertex 0 east to vertex 2, south to 10, east to
        // 11 and 100 m on towards 15.
        let line = [
            [50.0, 0.0],
            [400.0, 0.0],
            [400.0, 400.0],
            [600.0, 400.0],
            [600.0, 500.0],
        ];
        let r = match_track(
            &net,
            &sample(&line, 8.0, 6.0),
            "zigzag",
            &MatchParams::default(),
        )
        .unwrap();
        let want = vec![
            edge(&net, 0, 1),
            edge(&net, 1, 2),
            edge(&net, 2, 6),
            edge(&net, 6, 10),
            edge(&net, 10, 11),
            edge(&net, 11, 15),
        ];
        assert_eq!(edges_of(&r.plan), want, "{}", r.summary());
        assert!(
            (r.plan.length_m() - 1050.0).abs() < 12.0,
            "{}",
            r.plan.length_m()
        );
        assert!(r.off_road.is_empty(), "{:?}", r.off_road);
        assert_eq!(r.uturns, 0);
        assert!(!r.plan.is_loop);
        // Starts 50 m along the first edge, ends ~100 m along the last.
        let Segment::Edge { from_m, .. } = r.plan.steps[0] else {
            panic!()
        };
        assert!((from_m - 50.0).abs() < 8.0, "{from_m}");
    }

    #[test]
    fn out_and_back_gets_a_u_turn() {
        let net = grid();
        // East along the top row to 300 m (mid-edge 1-2), back to 50 m.
        let line = [[0.0, 0.0], [300.0, 0.0], [50.0, 0.0]];
        let r = match_track(
            &net,
            &sample(&line, 8.0, 4.0),
            "out-back",
            &MatchParams::default(),
        )
        .unwrap();
        assert_eq!(r.uturns, 1, "{}", r.summary());
        let turn = r
            .plan
            .steps
            .iter()
            .position(|s| matches!(s, Segment::Turnaround(_)))
            .unwrap();
        let Segment::Edge { edge: e, to_m, .. } = r.plan.steps[turn - 1] else {
            panic!()
        };
        assert_eq!(e, edge(&net, 1, 2).0);
        assert!((to_m - 100.0).abs() < 10.0, "turns at ~300 m: {to_m}");
        assert!(
            (r.plan.length_m() - 550.0).abs() < 25.0,
            "{}",
            r.plan.length_m()
        );
    }

    #[test]
    fn a_loop_is_ridden_in_laps() {
        let net = grid();
        // Round the block 0-1-5-4 clockwise (x east, z south: east, south,
        // west, north), starting and ending 60 m east of vertex 0.
        let line = [
            [60.0, 0.0],
            [200.0, 0.0],
            [200.0, 200.0],
            [0.0, 200.0],
            [0.0, 0.0],
            [60.0, 0.0],
        ];
        let r = match_track(
            &net,
            &sample(&line, 8.0, 5.0),
            "block",
            &MatchParams::default(),
        )
        .unwrap();
        assert!(r.plan.is_loop, "{}", r.summary());
        assert!(r.plan.closes(&net));
        assert_eq!(edges_of(&r.plan).len(), 4);
        assert!(
            (r.plan.length_m() - 800.0).abs() < 1.0,
            "{}",
            r.plan.length_m()
        );
    }

    #[test]
    fn one_way_roads_are_matched_the_right_way_round() {
        // Two parallel roads 30 m apart: the northern one only westwards.
        let mut g: RoadGraph = graph(
            &[[0.0, 0.0], [400.0, 0.0], [0.0, -30.0], [400.0, -30.0]],
            &[(0, 1), (2, 3), (0, 2), (1, 3)],
        );
        g.edges[1].oneway = crate::graph::Oneway::Backward;
        let net = RoadNetwork::new(g);
        // Riding east right between them, slightly nearer the north road:
        // only the south road may be ridden eastwards.
        let line = [[20.0, -17.0], [380.0, -17.0]];
        let r = match_track(
            &net,
            &sample(&line, 10.0, 1.0),
            "oneway",
            &MatchParams::default(),
        )
        .unwrap();
        assert_eq!(edges_of(&r.plan), vec![(0, true)], "{}", r.summary());
    }

    #[test]
    fn gaps_and_off_road_stretches_are_reported() {
        let net = grid();
        // Along the top row; a stretch 40 m north of the road (a parallel
        // path missing in the package), and an outlier 300 m off.
        let mut pts = sample(&[[0.0, 0.0], [600.0, 0.0]], 10.0, 0.0);
        for p in pts.iter_mut().skip(20).take(6) {
            p[1] = -40.0;
        }
        pts[40] = [300.0, -300.0];
        let r = match_track(&net, &pts, "gappy", &MatchParams::default()).unwrap();
        assert_eq!(edges_of(&r.plan).len(), 3, "{}", r.summary());
        assert!(
            r.off_road
                .iter()
                .any(|o| o.first == 20 && o.last == 25 && o.max_distance_m > 35.0),
            "{:?}",
            r.off_road
        );
        assert!(
            r.off_road
                .iter()
                .any(|o| o.first == 40 && !o.max_distance_m.is_finite()),
            "{:?}",
            r.off_road
        );
        assert!(r.summary().contains("no road nearby: points 40-40"));
        // Far from every road: no route.
        let far = sample(&[[5000.0, 5000.0], [5600.0, 5000.0]], 10.0, 0.0);
        assert_eq!(
            match_track(&net, &far, "x", &MatchParams::default()).unwrap_err(),
            MatchError::NoRoadNearby
        );
        assert_eq!(
            match_track(&net, &[[0.0, 0.0]], "x", &MatchParams::default()).unwrap_err(),
            MatchError::TooFewPoints
        );
    }

    #[test]
    fn disconnected_roads_break_the_match() {
        // Two separate roads 1 km apart, the track jumps from one to the other.
        let net = RoadNetwork::new(graph(
            &[[0.0, 0.0], [500.0, 0.0], [3000.0, 0.0], [3500.0, 0.0]],
            &[(0, 1), (2, 3)],
        ));
        let mut pts = sample(&[[0.0, 0.0], [500.0, 0.0]], 10.0, 0.0);
        pts.extend(sample(&[[3000.0, 0.0], [3500.0, 0.0]], 10.0, 0.0));
        let err = match_track(&net, &pts, "x", &MatchParams::default()).unwrap_err();
        assert!(matches!(err, MatchError::Broken { .. }), "{err:?}");
        assert!(err.to_string().contains("do not connect"));
    }

    #[test]
    fn routes_survive_a_rebuild_by_id_or_by_rematching() {
        use crate::graph_route::GraphRoute;
        let old = grid();
        let line = [[20.0, 0.0], [400.0, 0.0], [400.0, 300.0]];
        let r = match_track(
            &old,
            &sample(&line, 8.0, 3.0),
            "kept",
            &MatchParams::default(),
        )
        .unwrap();
        let stored = GraphRoute::from_plan(&r.plan, &old);
        // Same graph rebuilt: the stable IDs still fit.
        let o = (47.1, 9.5);
        let same = grid();
        assert!(matches!(
            carry_over(&stored, &old, o, &same, o).unwrap(),
            CarriedOver::Kept(_)
        ));
        // Rebuilt from newer data: the edge 1-2 got a new ID (e.g. split
        // way) – re-matched from the old geometry, onto the same roads.
        let mut g = old.graph().clone();
        let (e12, _) = edge(&old, 1, 2);
        g.edges[e12].id = EdgeId(777_777);
        let new = RoadNetwork::new(g);
        match carry_over(&stored, &old, o, &new, o).unwrap() {
            CarriedOver::Rematched { why, report } => {
                assert!(matches!(why, RouteBreak::MissingEdge { .. }), "{why:?}");
                assert_eq!(edges_of(&report.plan), edges_of(&r.plan));
                assert!((report.plan.length_m() - r.plan.length_m()).abs() < 3.0);
            }
            other => panic!("{other:?}"),
        }
        // The road is gone entirely: an error to report.
        let gone = RoadNetwork::new(graph(&[[5000.0, 0.0], [5400.0, 0.0]], &[(0, 1)]));
        assert!(carry_over(&stored, &old, o, &gone, o).is_err());
    }

    #[test]
    fn gpx_fixture_matches_after_projection() {
        // The fixture was sampled along the grid's roads around 47.1 N 9.5 E.
        let text = include_str!("../tests/fixtures/grid-ride.gpx");
        let gpx = Gpx::parse(text).unwrap();
        let net = grid();
        let r = match_gpx(&net, &gpx, (47.1, 9.5), "fixture", &MatchParams::default()).unwrap();
        assert!(r.off_road.is_empty(), "{}", r.summary());
        assert_eq!(edges_of(&r.plan)[0], edge(&net, 0, 1));
        assert!(r.gps_length_m > 500.0);
        // The stored route refers to stable edge IDs.
        let stored = crate::graph_route::GraphRoute::from_plan(&r.plan, &net);
        assert!(matches!(
            stored.steps[0],
            crate::graph_route::RouteStep::Edge { id: EdgeId(_), .. }
        ));
        assert_eq!(stored.resolve(&net).unwrap(), r.plan);
    }
}
