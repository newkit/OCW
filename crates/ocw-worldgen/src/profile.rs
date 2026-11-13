//! Road elevation profiles (spec §4.3): sampled from the DEM every 5 m,
//! smoothed (median, Gaussian), limited in gradient and gradient change
//! per road class, fixed to the junction heights at both ends; bridges and
//! tunnels interpolated between their ends.

use ocw_world::graph::{PROFILE_STEP_M, RoadClass, RoadGraph};

use crate::dem::Dem;
use crate::projection::LocalProjection;

/// Gradient limits of a road class: (max gradient, max gradient change per metre).
pub fn limits(class: RoadClass) -> (f64, f64) {
    match class {
        RoadClass::Trunk | RoadClass::Primary | RoadClass::Secondary => (0.15, 0.005),
        RoadClass::Tertiary | RoadClass::Unclassified | RoadClass::Residential => (0.20, 0.01),
        RoadClass::Cycleway => (0.20, 0.01),
        RoadClass::Service | RoadClass::Track | RoadClass::Path => (0.25, 0.01),
    }
}

/// Gaussian smoothing radius (sigma, in metres) of a road class: main
/// roads run evenly; SRTM is a surface model (trees, buildings), so short
/// bumps are noise.
pub fn smoothing_sigma_m(class: RoadClass) -> f64 {
    match class {
        RoadClass::Trunk | RoadClass::Primary | RoadClass::Secondary => 50.0,
        RoadClass::Tertiary | RoadClass::Unclassified => 40.0,
        _ => 30.0,
    }
}

/// Positions along a polyline every `step` metres, plus the end point.
pub fn resample(points: &[[f64; 2]], step: f64) -> Vec<[f64; 2]> {
    let mut out = vec![points[0]];
    let (mut walked, mut next) = (0.0, step);
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
        while len > 0.0 && next <= walked + len + 1e-9 {
            let t = ((next - walked) / len).min(1.0);
            out.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
            next += step;
        }
        walked += len;
    }
    let last = *points.last().expect("non-empty");
    let end = out.last().expect("non-empty");
    if (end[0] - last[0]).abs() + (end[1] - last[1]).abs() > 1e-6 {
        out.push(last);
    }
    out
}

/// Points every `step` m along `points` with their distance along the
/// polyline (the true arc length, which matches `RoadEdge::height_at`; the
/// chords between resampled points cut corners and are shorter). With
/// `keep_vertices` the original vertices are kept too, so the samples follow
/// the road exactly through bends.
pub fn sample_along(points: &[[f64; 2]], step: f64, keep_vertices: bool) -> Vec<([f64; 2], f64)> {
    let mut out = vec![(points[0], 0.0)];
    let (mut walked, mut next) = (0.0, step);
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
        while len > 0.0 && next <= walked + len + 1e-9 {
            let t = ((next - walked) / len).min(1.0);
            out.push(([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t], next));
            next += step;
        }
        walked += len;
        let last = out.last().expect("non-empty").1;
        if keep_vertices && len > 0.0 && walked - last > 1e-6 {
            out.push((b, walked));
        }
    }
    let last = *points.last().expect("non-empty");
    let end = out.last().expect("non-empty").0;
    if (end[0] - last[0]).abs() + (end[1] - last[1]).abs() > 1e-6 {
        out.push((last, walked));
    }
    out
}

fn median_filter(v: &[f64], half: usize) -> Vec<f64> {
    (0..v.len())
        .map(|i| {
            let (a, b) = (i.saturating_sub(half), (i + half + 1).min(v.len()));
            let mut w = v[a..b].to_vec();
            w.sort_by(f64::total_cmp);
            w[w.len() / 2]
        })
        .collect()
}

fn gaussian(v: &[f64], sigma_samples: f64) -> Vec<f64> {
    let r = (3.0 * sigma_samples).ceil() as i64;
    (0..v.len() as i64)
        .map(|i| {
            let (mut s, mut w) = (0.0, 0.0);
            for k in -r..=r {
                let j = (i + k).clamp(0, v.len() as i64 - 1) as usize;
                let wk = (-(k as f64).powi(2) / (2.0 * sigma_samples * sigma_samples)).exp();
                s += v[j] * wk;
                w += wk;
            }
            s / w
        })
        .collect()
}

/// Distances of the samples from the start (last interval may be shorter).
fn distances(n: usize, length: f64) -> Vec<f64> {
    (0..n)
        .map(|i| (i as f64 * PROFILE_STEP_M).min(length))
        .collect()
}

/// Limits gradient and gradient change while keeping the profile close to
/// `target`, with fixed heights at both ends.
pub fn constrain(
    target: &[f64],
    length: f64,
    start: f64,
    end: f64,
    max_grade: f64,
    max_change_per_m: f64,
) -> Vec<f64> {
    let n = target.len();
    if n < 2 {
        return target.to_vec();
    }
    let d = distances(n, length);
    // Gradients of the target per interval, clamped.
    let mut g: Vec<f64> = (0..n - 1)
        .map(|i| {
            let ds = (d[i + 1] - d[i]).max(1e-9);
            ((target[i + 1] - target[i]) / ds).clamp(-max_grade, max_grade)
        })
        .collect();
    // Limit the change between neighbouring intervals (forward and back).
    for _ in 0..3 {
        for i in 1..g.len() {
            let ds = (d[i + 1] - d[i - 1]) / 2.0;
            let m = max_change_per_m * ds;
            g[i] = g[i].clamp(g[i - 1] - m, g[i - 1] + m);
        }
        for i in (0..g.len().saturating_sub(1)).rev() {
            let ds = (d[i + 2] - d[i]) / 2.0;
            let m = max_change_per_m * ds;
            g[i] = g[i].clamp(g[i + 1] - m, g[i + 1] + m);
        }
    }
    // Meet the end height: distribute the missing height over the
    // intervals in proportion to their room below the gradient limit (so
    // the limit holds whenever the end is reachable at all), then a linear
    // rest for what is physically impossible within the limit.
    let ds: Vec<f64> = (0..n - 1).map(|i| d[i + 1] - d[i]).collect();
    let reached: f64 = g.iter().zip(&ds).map(|(g, ds)| g * ds).sum();
    let miss = end - start - reached;
    let room: Vec<f64> = g
        .iter()
        .map(|&g| {
            if miss > 0.0 {
                (max_grade - g).max(0.0)
            } else {
                (g + max_grade).max(0.0)
            }
        })
        .collect();
    let total: f64 = room.iter().zip(&ds).map(|(r, ds)| r * ds).sum();
    if total > 1e-12 {
        let frac = (miss.abs() / total).min(1.0) * miss.signum();
        for (gi, r) in g.iter_mut().zip(&room) {
            *gi += r * frac;
        }
    }
    let mut h = vec![start; n];
    for i in 1..n {
        h[i] = h[i - 1] + g[i - 1] * ds[i - 1];
    }
    let rest = end - h[n - 1];
    for i in 0..n {
        h[i] += rest * d[i] / length.max(1e-9);
    }
    h
}

/// Per-edge findings for the build log (spec §4.3, step 5).
#[derive(Debug, Clone, PartialEq)]
pub struct Suspicious {
    pub edge: usize,
    pub name: Option<String>,
    /// Steepest raw DEM gradient over 25 m.
    pub raw_grade: f64,
    /// Steepest gradient after smoothing.
    pub smoothed_grade: f64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProfileStats {
    pub samples: usize,
    pub min_m: f64,
    pub max_m: f64,
    pub max_grade: f64,
    pub bridges_tunnels: usize,
    pub missing_dem: usize,
    pub suspicious: Vec<Suspicious>,
}

/// Steepest gradient over windows of `window` samples.
pub fn steepest(h: &[f64], window: usize) -> f64 {
    if h.len() <= window {
        return 0.0;
    }
    (0..h.len() - window)
        .map(|i| (h[i + window] - h[i]).abs() / (window as f64 * PROFILE_STEP_M))
        .fold(0.0, f64::max)
}

/// Moves junction heights so no (non-bridge) edge needs more than its
/// class gradient between its ends: on short edges a single noisy DEM
/// sample would otherwise force an impossible wall. The excess is split
/// between both ends (weighted towards the end with fewer roads), and
/// repeated passes spread it over the neighbouring roads.
pub fn relax_junction_heights(graph: &RoadGraph, vertex_h: &mut [f64]) {
    let degree: Vec<f64> = graph
        .adjacency()
        .iter()
        .map(|a| a.len().max(1) as f64)
        .collect();
    for _ in 0..50 {
        let mut changed = false;
        for e in &graph.edges {
            if e.bridge || e.tunnel {
                continue;
            }
            let (a, b) = (e.from.0 as usize, e.to.0 as usize);
            let allowed = limits(e.class).0 * e.length_m();
            let dh = vertex_h[b] - vertex_h[a];
            let excess = dh.abs() - allowed;
            if excess <= 1e-6 {
                continue;
            }
            // Busier junctions are better anchored: they move less.
            let (wa, wb) = (1.0 / degree[a], 1.0 / degree[b]);
            let s = dh.signum() * excess;
            vertex_h[a] += s * wa / (wa + wb);
            vertex_h[b] -= s * wb / (wa + wb);
            changed = true;
        }
        if !changed {
            break;
        }
    }
}

/// Junctions touched only by bridges and tunnels (a bridge mapped as
/// several ways) have no road on the ground to take their height from; the
/// DEM there is the valley under the bridge or the mountain above the
/// tunnel. Their height is interpolated by distance along the structure
/// between its anchored ends (Gauss–Seidel on the inverse-length weighted
/// mean, exact for chains). Structures with no anchored end keep the DEM.
pub fn interpolate_along_structures(graph: &RoadGraph, anchored: &[bool], vertex_h: &mut [f64]) {
    let mut links: Vec<Vec<(usize, f64)>> = vec![Vec::new(); vertex_h.len()];
    for e in graph.edges.iter().filter(|e| e.bridge || e.tunnel) {
        let (a, b) = (e.from.0 as usize, e.to.0 as usize);
        let w = 1.0 / e.length_m().max(0.1);
        links[a].push((b, w));
        links[b].push((a, w));
    }
    // Only free vertices connected (through structures) to an anchor move.
    let mut reach = anchored.to_vec();
    let mut stack: Vec<usize> = (0..reach.len()).filter(|&v| reach[v]).collect();
    while let Some(v) = stack.pop() {
        for &(u, _) in &links[v] {
            if !reach[u] {
                reach[u] = true;
                stack.push(u);
            }
        }
    }
    let free: Vec<usize> = (0..vertex_h.len())
        .filter(|&v| !anchored[v] && reach[v] && !links[v].is_empty())
        .collect();
    for _ in 0..10_000 {
        let mut moved = 0.0f64;
        for &v in &free {
            let (num, den) = links[v]
                .iter()
                .fold((0.0, 0.0), |(n, d), &(u, w)| (n + vertex_h[u] * w, d + w));
            let h = num / den;
            moved = moved.max((h - vertex_h[v]).abs());
            vertex_h[v] = h;
        }
        if moved < 1e-6 {
            break;
        }
    }
}

/// Elevation filter along whole roads: the pieces of each through-road
/// ([`crate::chains`]) are joined, sampled from the DEM every 5 m,
/// median- and Gaussian-filtered and limited in gradient and gradient
/// change as one profile, so there are no jumps in gradient where OSM
/// splits a road. Main roads go first and fix the heights of their
/// junctions; a later road is split into runs at junctions already fixed.
/// Bridges and tunnels are part of the runs; the DEM along them (the
/// valley below, the mountain above) is replaced by a straight line
/// between the ground at their ends before filtering. Returns the profile
/// per edge and updates the heights of the junctions inside the runs.
pub fn filter_through_roads(
    graph: &RoadGraph,
    height: &impl Fn([f64; 2]) -> Option<f64>,
    vertex_h: &mut [f64],
) -> Vec<Option<Vec<f64>>> {
    use crate::chains::{chain_vertices, through_chains};
    let mut out: Vec<Option<Vec<f64>>> = vec![None; graph.edges.len()];
    let mut fixed = vec![false; vertex_h.len()];
    for chain in through_chains(graph, |_| true) {
        let vs = chain_vertices(graph, &chain);
        let mut start = 0;
        let mut seen = vec![vs[0]];
        for k in 1..=chain.len() {
            let v = vs[k];
            let split = k == chain.len() || fixed[v as usize] || seen.contains(&v);
            seen.push(v);
            if split {
                filter_run(graph, &chain[start..k], height, vertex_h, &mut out);
                for &v in &vs[start..=k] {
                    fixed[v as usize] = true;
                }
                start = k;
                seen = vec![v];
            }
        }
    }
    out
}

/// One run of a through-road between two fixed junction heights.
fn filter_run(
    graph: &RoadGraph,
    run: &[(usize, bool)],
    height: &impl Fn([f64; 2]) -> Option<f64>,
    vertex_h: &mut [f64],
    out: &mut [Option<Vec<f64>>],
) {
    let mut poly: Vec<[f64; 2]> = Vec::new();
    let mut offsets = Vec::with_capacity(run.len());
    // Stretches (from, to metres along the run) on bridges and tunnels.
    let mut structures: Vec<(f64, f64)> = Vec::new();
    let mut total = 0.0;
    let (mut max_grade, mut sigma) = (f64::INFINITY, f64::INFINITY);
    for &(e, fwd) in run {
        let e = &graph.edges[e];
        let mut pts = e.points.clone();
        if !fwd {
            pts.reverse();
        }
        let skip = usize::from(!poly.is_empty());
        poly.extend_from_slice(&pts[skip..]);
        offsets.push(total);
        if e.bridge || e.tunnel {
            structures.push((total, total + e.length_m()));
        }
        total += e.length_m();
        max_grade = max_grade.min(limits(e.class).0);
        sigma = sigma.min(smoothing_sigma_m(e.class));
    }
    let samples = sample_along(&poly, PROFILE_STEP_M, false);
    let len = samples.last().expect("non-empty").1;
    let raw: Vec<Option<f64>> = samples.iter().map(|&(p, _)| height(p)).collect();
    let fallback = raw.iter().flatten().next().copied().unwrap_or(0.0);
    let mut raw: Vec<f64> = raw.iter().map(|v| v.unwrap_or(fallback)).collect();
    let edge_of = |k: usize| {
        let (e, fwd) = run[k];
        let e = &graph.edges[e];
        if fwd {
            (e.from.0, e.to.0)
        } else {
            (e.to.0, e.from.0)
        }
    };
    let (first, last) = (edge_of(0).0, edge_of(run.len() - 1).1);
    // Straight across structures: from the last ground sample before to
    // the first after (the junction height where the run starts or ends
    // on one).
    let on_structure: Vec<bool> = samples
        .iter()
        .map(|&(_, s)| {
            structures
                .iter()
                .any(|&(a, b)| s > a + 1e-6 && s < b - 1e-6)
        })
        .collect();
    let mut k = 0;
    while k < raw.len() {
        if !on_structure[k] {
            k += 1;
            continue;
        }
        let a = k.checked_sub(1);
        let mut b = k;
        while b < raw.len() && on_structure[b] {
            b += 1;
        }
        let ha = a.map_or(vertex_h[first as usize], |a| raw[a]);
        let hb = if b < raw.len() {
            raw[b]
        } else {
            vertex_h[last as usize]
        };
        let sa = a.map_or(0.0, |a| samples[a].1);
        let sb = if b < raw.len() { samples[b].1 } else { len };
        for j in k..b {
            raw[j] = ha + (hb - ha) * (samples[j].1 - sa) / (sb - sa).max(1e-9);
        }
        k = b;
    }
    let target = gaussian(&median_filter(&raw, 2), sigma / PROFILE_STEP_M);
    let (start_h, end_h) = (vertex_h[first as usize], vertex_h[last as usize]);
    // No clipping and integrating along a whole road (that drifts off the
    // terrain): local grade limit, light smoothing for the gradient changes,
    // local limit again.
    let local = limit_grade_locally(&target, len, start_h, end_h, max_grade);
    let soft = gaussian(&local, CHANGE_SIGMA_M / PROFILE_STEP_M);
    let h = limit_grade_locally(&soft, len, start_h, end_h, max_grade);
    let d = distances(h.len(), len);
    let at = |s: f64| {
        if h.len() < 2 {
            return h[0];
        }
        let s = s.clamp(0.0, len);
        let i = ((s / PROFILE_STEP_M).floor() as usize).min(h.len() - 2);
        let seg = (d[i + 1] - d[i]).max(1e-9);
        h[i] + (h[i + 1] - h[i]) * ((s - d[i]) / seg).clamp(0.0, 1.0)
    };
    for (k, &(ei, fwd)) in run.iter().enumerate() {
        let e = &graph.edges[ei];
        let l = e.length_m();
        let n = sample_along(&e.points, PROFILE_STEP_M, false).len();
        let o = offsets[k];
        out[ei] = Some(
            distances(n, l)
                .iter()
                .map(|&x| at(if fwd { o + x } else { o + l - x }))
                .collect(),
        );
        if k + 1 < run.len() {
            vertex_h[edge_of(k).1 as usize] = at(o + l);
        }
    }
}

/// Smoothing (sigma, metres) that rounds the gradient changes where the
/// grade limit clipped a road.
const CHANGE_SIGMA_M: f64 = 20.0;

/// Brings `target` within `max_grade` without drifting away from it: the
/// ends are moved to `start`/`end` (the correction fading out over 200 m),
/// then every interval that is too steep is flattened by moving both of its
/// samples towards each other, repeated until it holds. Unlike clipping
/// the gradient and integrating, the height a steep stretch loses stays
/// there instead of being spread over the whole road.
pub fn limit_grade_locally(
    target: &[f64],
    length: f64,
    start: f64,
    end: f64,
    max_grade: f64,
) -> Vec<f64> {
    let n = target.len();
    let mut h = target.to_vec();
    if n < 2 {
        return h;
    }
    let d = distances(n, length);
    const FADE_M: f64 = 200.0;
    let (ds, de) = (start - target[0], end - target[n - 1]);
    for i in 0..n {
        let from_start = (1.0 - d[i] / FADE_M).max(0.0);
        let from_end = (1.0 - (length - d[i]) / FADE_M).max(0.0);
        h[i] += ds * from_start + de * from_end;
    }
    h[0] = start;
    h[n - 1] = end;
    for _ in 0..2000 {
        let mut worst = 0.0f64;
        for i in 0..n - 1 {
            let run = (d[i + 1] - d[i]).max(1e-9);
            let dh = h[i + 1] - h[i];
            let excess = dh.abs() - max_grade * run;
            if excess <= 1e-9 {
                continue;
            }
            worst = worst.max(excess);
            let s = dh.signum() * excess;
            match (i == 0, i + 1 == n - 1) {
                (true, true) => {}
                (true, false) => h[i + 1] -= s,
                (false, true) => h[i] += s,
                (false, false) => {
                    h[i] += s / 2.0;
                    h[i + 1] -= s / 2.0;
                }
            }
        }
        if worst < 1e-3 {
            break;
        }
    }
    h
}

/// Brings every edge of `graph` within its class gradient, across the
/// junctions. The through-road filter fixes junction heights road by road,
/// so a short connector between two roads filtered on their own (or a
/// bridge between junctions of two roads) can end up far steeper than its
/// class allows: up to 200 % on the pilot region before this pass.
///
/// 1. Junction heights are relaxed again, now over all edges (bridges and
///    tunnels too): an edge whose ends are too far apart moves both
///    (weighted towards the end with fewer roads), repeated until every
///    edge can hold its limit between its ends.
/// 2. Every road whose end moved, or that is still too steep, is limited
///    locally between its new ends ([`limit_grade_locally`]: the end
///    correction fades out over 200 m); bridges and tunnels stay straight.
/// 3. What is left is flattened sample by sample across the junctions
///    (a junction's height is shared by all its roads).
///
/// Profiles that hold their limit and whose ends did not move are not
/// touched.
pub fn limit_grades_across_junctions(graph: &mut RoadGraph, vertex_h: &mut [f64]) {
    let before = vertex_h.to_vec();
    let degree: Vec<f64> = graph
        .adjacency()
        .iter()
        .map(|a| a.len().max(1) as f64)
        .collect();
    for _ in 0..1000 {
        let mut changed = false;
        for e in &graph.edges {
            let (a, b) = (e.from.0 as usize, e.to.0 as usize);
            let dh = vertex_h[b] - vertex_h[a];
            let excess = dh.abs() - limits(e.class).0 * e.length_m();
            if a == b || excess <= 1e-6 {
                continue;
            }
            let (wa, wb) = (1.0 / degree[a], 1.0 / degree[b]);
            let s = dh.signum() * excess;
            vertex_h[a] += s * wa / (wa + wb);
            vertex_h[b] -= s * wb / (wa + wb);
            changed = true;
        }
        if !changed {
            break;
        }
    }
    for e in &mut graph.edges {
        let n = e.profile.len();
        if n < 2 {
            continue;
        }
        let (a, b) = (e.from.0 as usize, e.to.0 as usize);
        let max_grade = limits(e.class).0;
        let moved =
            (vertex_h[a] - before[a]).abs() > 1e-9 || (vertex_h[b] - before[b]).abs() > 1e-9;
        if !moved && steepest(&e.profile, 1) <= max_grade + 1e-6 {
            continue;
        }
        let len = e.length_m();
        let (ha, hb) = (vertex_h[a], vertex_h[b]);
        e.profile = if e.bridge || e.tunnel {
            distances(n, len)
                .iter()
                .map(|x| ha + (hb - ha) * x / len.max(1e-9))
                .collect()
        } else {
            limit_grade_locally(&e.profile, len, ha, hb, max_grade)
        };
    }
    let runs: Vec<(Vec<f64>, f64)> = graph
        .edges
        .iter()
        .map(|e| {
            let d = distances(e.profile.len(), e.length_m());
            let runs = d.windows(2).map(|w| (w[1] - w[0]).max(0.0)).collect();
            (runs, limits(e.class).0)
        })
        .collect();
    for _ in 0..2000 {
        let mut worst = 0.0f64;
        for (e, (run, max_grade)) in graph.edges.iter_mut().zip(&runs) {
            let p = &mut e.profile;
            let n = p.len();
            if n < 2 {
                continue;
            }
            let (a, b) = (e.from.0 as usize, e.to.0 as usize);
            p[0] = vertex_h[a];
            p[n - 1] = vertex_h[b];
            for (k, &r) in run.iter().enumerate() {
                let dh = p[k + 1] - p[k];
                let excess = dh.abs() - max_grade * r;
                if excess <= 1e-9 {
                    continue;
                }
                worst = worst.max(excess);
                let s = dh.signum() * excess / 2.0;
                p[k] += s;
                p[k + 1] -= s;
            }
            vertex_h[a] = p[0];
            vertex_h[b] = p[n - 1];
        }
        if worst < 1e-4 {
            break;
        }
    }
    // Exactly the junction heights at the ends (continuity).
    for e in &mut graph.edges {
        let n = e.profile.len();
        if n >= 2 {
            e.profile[0] = vertex_h[e.from.0 as usize];
            e.profile[n - 1] = vertex_h[e.to.0 as usize];
        }
    }
}

/// Computes `profile` for every edge of `graph`.
///
/// 1. Sample the DEM every 5 m; median filter; Gaussian smoothing per class.
/// 2. Junction heights: the mean of the smoothed ends of the roads meeting
///    there (bridges and tunnels excluded; DEM at the vertex if none), so
///    noisy DEM samples at a single point cannot force impossible gradients.
/// 3. Filter along whole through-roads ([`filter_through_roads`]): one
///    profile across the OSM way splits, limited in gradient (locally, so
///    it stays on the terrain) and gradient change between fixed junctions.
/// 4. The class gradient across junctions
///    ([`limit_grades_across_junctions`]).
pub fn add_profiles(graph: &mut RoadGraph, dem: &Dem, proj: &LocalProjection) -> ProfileStats {
    let height = |p: [f64; 2]| {
        let (lat, lon) = proj.unproject(p);
        dem.elevation(lat, lon)
    };
    let mut stats = ProfileStats {
        min_m: f64::INFINITY,
        max_m: f64::NEG_INFINITY,
        ..Default::default()
    };
    // Pass 1: raw and smoothed profiles; the DEM sampling of each edge is
    // independent, so on all cores (results in edge order, crate::par).
    crate::stages::start("DEM samples");
    let per_edge = crate::par::map(&graph.edges, crate::par::threads(), |e| {
        let pts = resample(&e.points, PROFILE_STEP_M);
        let raw: Vec<Option<f64>> = pts.iter().map(|&p| height(p)).collect();
        let missing = raw.iter().any(Option::is_none);
        let fallback = raw.iter().flatten().next().copied().unwrap_or(0.0);
        let raw: Vec<f64> = raw.iter().map(|v| v.unwrap_or(fallback)).collect();
        let sigma = smoothing_sigma_m(e.class) / PROFILE_STEP_M;
        let smooth = gaussian(&median_filter(&raw, 2), sigma);
        (raw, smooth, missing)
    });
    crate::stages::end();
    let mut raws = Vec::with_capacity(graph.edges.len());
    let mut smooths = Vec::with_capacity(graph.edges.len());
    for (raw, smooth, missing) in per_edge {
        stats.missing_dem += usize::from(missing);
        smooths.push(smooth);
        raws.push(raw);
    }
    // Pass 2: junction heights by consensus.
    let mut sum = vec![(0.0, 0u32); graph.vertices.len()];
    for (e, s) in graph.edges.iter().zip(&smooths) {
        if e.bridge || e.tunnel {
            continue;
        }
        for (v, h) in [(e.from.0, s[0]), (e.to.0, *s.last().expect("non-empty"))] {
            sum[v as usize].0 += h;
            sum[v as usize].1 += 1;
        }
    }
    let mut vertex_h: Vec<f64> = sum
        .iter()
        .zip(&graph.vertices)
        .map(|(&(s, n), &p)| {
            if n > 0 {
                s / f64::from(n)
            } else {
                height(p).unwrap_or(0.0)
            }
        })
        .collect();
    let anchored: Vec<bool> = sum.iter().map(|&(_, n)| n > 0).collect();
    crate::stages::time("junction heights", || {
        interpolate_along_structures(graph, &anchored, &mut vertex_h);
        relax_junction_heights(graph, &mut vertex_h);
    });
    // Pass 3: filtered along whole roads (through the OSM splits), then
    // bridges and tunnels linear between their ends.
    let mut along_roads = crate::stages::time("through roads", || {
        filter_through_roads(&*graph, &height, &mut vertex_h)
    });
    for (i, e) in graph.edges.iter_mut().enumerate() {
        let len = e.length_m();
        let smooth = &smooths[i];
        let (start, end) = (vertex_h[e.from.0 as usize], vertex_h[e.to.0 as usize]);
        let (max_grade, max_change) = limits(e.class);
        if e.bridge || e.tunnel {
            stats.bridges_tunnels += 1;
        }
        let mut profile: Vec<f64> = along_roads[i].take().unwrap_or_else(|| {
            if e.bridge || e.tunnel {
                distances(smooth.len(), len)
                    .iter()
                    .map(|x| start + (end - start) * x / len.max(1e-9))
                    .collect()
            } else {
                constrain(smooth, len, start, end, max_grade, max_change)
            }
        });
        // Exactly the junction heights at the ends (continuity).
        profile[0] = start;
        *profile.last_mut().expect("non-empty") = end;
        e.profile = profile;
    }
    // Pass 4: the class limit across junctions (short connectors between
    // roads filtered on their own, bridges between junctions of different
    // roads).
    crate::stages::time("grades across junctions", || {
        limit_grades_across_junctions(graph, &mut vertex_h);
    });
    for (i, e) in graph.edges.iter().enumerate() {
        let len = e.length_m();
        let raw = &raws[i];
        let max_grade = limits(e.class).0;
        let profile = &e.profile;
        let raw_grade = steepest(raw, 5);
        let smoothed_grade = steepest(profile, 5);
        if !(e.bridge || e.tunnel) && raw_grade > max_grade + 0.10 && len > 50.0 {
            stats.suspicious.push(Suspicious {
                edge: i,
                name: e.name.clone(),
                raw_grade,
                smoothed_grade,
            });
        }
        stats.samples += profile.len();
        stats.max_grade = stats.max_grade.max(smoothed_grade);
        for &h in profile {
            stats.min_m = stats.min_m.min(h);
            stats.max_m = stats.max_m.max(h);
        }
    }
    stats
        .suspicious
        .sort_by(|a, b| b.raw_grade.total_cmp(&a.raw_grade));
    stats
}

/// The point at arc length `d` along a polyline.
fn point_at(points: &[[f64; 2]], d: f64) -> [f64; 2] {
    let mut walked = 0.0;
    for w in points.windows(2) {
        let len = ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt();
        if walked + len >= d && len > 0.0 {
            let t = ((d - walked) / len).clamp(0.0, 1.0);
            return [
                w[0][0] + (w[1][0] - w[0][0]) * t,
                w[0][1] + (w[1][1] - w[0][1]) * t,
            ];
        }
        walked += len;
    }
    *points.last().expect("non-empty")
}

/// Distance of `p` from a polyline and the arc length of its foot point.
fn project_onto(points: &[[f64; 2]], p: [f64; 2]) -> (f64, f64) {
    let (mut best, mut at, mut walked) = (f64::INFINITY, 0.0, 0.0);
    for w in points.windows(2) {
        let (dx, dz) = (w[1][0] - w[0][0], w[1][1] - w[0][1]);
        let len2 = dx * dx + dz * dz;
        let t = if len2 > 0.0 {
            (((p[0] - w[0][0]) * dx + (p[1] - w[0][1]) * dz) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let q = [w[0][0] + dx * t, w[0][1] + dz * t];
        let d = (p[0] - q[0]).hypot(p[1] - q[1]);
        if d < best {
            best = d;
            at = walked + t * len2.sqrt();
        }
        walked += len2.sqrt();
    }
    (best, at)
}

/// Takes the road profiles of `target` from a larger graph of the same
/// roads (`source`, same projection): every sample is read from the source
/// edge of the same OSM way that passes within `tolerance_m`. For a package
/// cut out of a bigger region (spec §2.2.2): its profiles near the border
/// are those of the whole road, so the neighbouring package (built the same
/// way) has the same heights where the road crosses. Edges not fully
/// covered by the source keep their own profile. Returns the edges taken over.
pub fn transfer_profiles(target: &mut RoadGraph, source: &RoadGraph, tolerance_m: f64) -> usize {
    let mut by_way: std::collections::HashMap<i64, Vec<usize>> = Default::default();
    for (i, e) in source.edges.iter().enumerate() {
        if !e.profile.is_empty() {
            by_way.entry(e.osm_way).or_default().push(i);
        }
    }
    let mut taken = 0;
    for e in &mut target.edges {
        let Some(candidates) = by_way.get(&e.osm_way) else {
            continue;
        };
        let len = e.length_m();
        let n = e.profile.len().max(2);
        let new: Option<Vec<f64>> = (0..n)
            .map(|k| {
                let d = if k == n - 1 {
                    len
                } else {
                    (k as f64 * PROFILE_STEP_M).min(len)
                };
                let p = point_at(&e.points, d);
                candidates
                    .iter()
                    .map(|&i| (i, project_onto(&source.edges[i].points, p)))
                    .filter(|(_, (dist, _))| *dist <= tolerance_m)
                    .min_by(|a, b| a.1.0.total_cmp(&b.1.0))
                    .and_then(|(i, (_, at))| source.edges[i].height_at(at))
            })
            .collect();
        if let Some(profile) = new {
            e.profile = profile;
            taken += 1;
        }
    }
    taken
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_are_taken_over_from_the_whole_road() {
        use ocw_world::graph::{Edge, EdgeId, Oneway, VertexId};
        let edge = |way: i64, pts: Vec<[f64; 2]>, profile: Vec<f64>| Edge {
            id: EdgeId(way as u64),
            from: VertexId(0),
            to: VertexId(1),
            points: pts,
            class: RoadClass::Residential,
            name: None,
            width_m: 5.0,
            bridge: false,
            tunnel: false,
            dead_end_m: None,
            boundary: false,
            oneway: Oneway::No,
            osm_way: way,
            profile,
        };
        // The whole road: 100 m east, rising 1 m per 10 m.
        let whole = RoadGraph {
            vertices: vec![[0.0, 0.0], [100.0, 0.0]],
            edges: vec![edge(
                7,
                vec![[0.0, 0.0], [50.0, 0.0], [100.0, 0.0]],
                (0..=20).map(|k| f64::from(k) * 0.5).collect(),
            )],
            boundary_vertices: Default::default(),
        };
        // A package's piece of it from x = 50 to a border at x = 62.5,
        // with a flat profile of its own; another road is left alone.
        let mut part = RoadGraph {
            vertices: vec![[50.0, 0.0], [62.5, 0.0]],
            edges: vec![
                edge(7, vec![[50.0, 0.0], [62.5, 0.0]], vec![0.0; 4]),
                edge(8, vec![[50.0, 0.0], [62.5, 0.0]], vec![0.0; 4]),
            ],
            boundary_vertices: Default::default(),
        };
        assert_eq!(transfer_profiles(&mut part, &whole, 1.0), 1);
        let p = &part.edges[0].profile;
        assert_eq!(p.len(), 4);
        for (got, want) in p.iter().zip([5.0, 5.5, 6.0, 6.25]) {
            assert!((got - want).abs() < 1e-9, "{p:?}");
        }
        assert_eq!(part.edges[1].profile, vec![0.0; 4]);
    }

    #[test]
    fn resampling_every_five_metres() {
        let pts = resample(&[[0.0, 0.0], [12.0, 0.0]], 5.0);
        assert_eq!(pts, vec![[0.0, 0.0], [5.0, 0.0], [10.0, 0.0], [12.0, 0.0]]);
        // Across a corner: 3 m + 4 m legs, 7 m total.
        let pts = resample(&[[0.0, 0.0], [3.0, 0.0], [3.0, 4.0]], 5.0);
        assert_eq!(pts.len(), 3);
        assert!((pts[1][0] - 3.0).abs() < 1e-9 && (pts[1][1] - 2.0).abs() < 1e-9);
        // Exactly on the step: no duplicate end point.
        assert_eq!(resample(&[[0.0, 0.0], [10.0, 0.0]], 5.0).len(), 3);
    }

    #[test]
    fn samples_carry_the_true_distance_through_bends() {
        // A zigzag: chords between 5 m samples are shorter than the path.
        let zigzag: Vec<[f64; 2]> = (0..=40)
            .map(|i| [i as f64 * 2.0, if i % 2 == 0 { 0.0 } else { 2.0 }])
            .collect();
        let length: f64 = zigzag
            .windows(2)
            .map(|w| ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt())
            .sum();
        let plain = sample_along(&zigzag, 5.0, false);
        let chords: f64 = plain
            .windows(2)
            .map(|w| ((w[1].0[0] - w[0].0[0]).powi(2) + (w[1].0[1] - w[0].0[1]).powi(2)).sqrt())
            .sum();
        assert!(chords < length - 10.0, "chords {chords} vs {length}");
        assert!((plain.last().unwrap().1 - length).abs() < 1e-9);
        assert_eq!(
            plain.iter().map(|p| p.0).collect::<Vec<_>>(),
            resample(&zigzag, 5.0)
        );
        for w in plain.windows(2) {
            assert!(w[1].1 > w[0].1);
        }
        // With the vertices kept, every original point is present.
        let full = sample_along(&zigzag, 5.0, true);
        for p in &zigzag {
            assert!(full.iter().any(|q| q.0 == *p), "{p:?}");
        }
        for w in full.windows(2) {
            let d = ((w[1].0[0] - w[0].0[0]).powi(2) + (w[1].0[1] - w[0].0[1]).powi(2)).sqrt();
            assert!(
                (w[1].1 - w[0].1 - d).abs() < 1e-9,
                "distance must match the polyline"
            );
        }
    }

    #[test]
    fn junctions_inside_a_bridge_follow_the_bridge() {
        // Road (anchored) - bridge 30 m - junction - bridge 60 m - road.
        // Regression: the middle junction took the DEM of the valley floor
        // (Steg, after the Stegertunnel: -24 %).
        use ocw_world::graph::{Edge, EdgeId, VertexId};
        let edge = |id, from, to, a: [f64; 2], b: [f64; 2], bridge| Edge {
            id: EdgeId(id),
            from: VertexId(from),
            to: VertexId(to),
            points: vec![a, b],
            class: RoadClass::Secondary,
            name: None,
            width_m: 6.5,
            bridge,
            tunnel: false,
            dead_end_m: None,
            oneway: ocw_world::graph::Oneway::No,
            boundary: false,
            osm_way: id as i64,
            profile: vec![],
        };
        let v = [
            [-100.0, 0.0],
            [0.0, 0.0],
            [30.0, 0.0],
            [90.0, 0.0],
            [190.0, 0.0],
        ];
        let g = RoadGraph {
            vertices: v.to_vec(),
            edges: vec![
                edge(1, 0, 1, v[0], v[1], false),
                edge(2, 1, 2, v[1], v[2], true),
                edge(3, 2, 3, v[2], v[3], true),
                edge(4, 3, 4, v[3], v[4], false),
            ],
            boundary_vertices: Default::default(),
        };
        let anchored = [true, true, false, true, true];
        let mut h = vec![100.0, 100.0, 40.0, 109.0, 109.0];
        interpolate_along_structures(&g, &anchored, &mut h);
        assert!((h[2] - 103.0).abs() < 1e-4, "{}", h[2]);
        assert_eq!((h[1], h[3]), (100.0, 109.0));
        // A structure with no anchored end keeps the DEM.
        let mut h = vec![100.0, 100.0, 40.0, 109.0, 109.0];
        interpolate_along_structures(&g, &[false; 5], &mut h);
        assert_eq!(h[2], 40.0);
    }

    #[test]
    fn through_roads_are_filtered_across_way_splits() {
        // A 5 % climb mapped as three ways; the consensus height of the
        // middle junctions is off by 6 m (a noisy DEM sample). Filtered per
        // piece, the gradient jumps at the splits; along the whole road it
        // runs smoothly and the junctions follow the road.
        use ocw_world::graph::{Edge, EdgeId, VertexId};
        let v = [[0.0, 0.0], [300.0, 0.0], [330.0, 0.0], [700.0, 0.0]];
        let edge = |id: u32, a: usize, b: usize, fwd: bool| {
            let (a, b) = if fwd { (a, b) } else { (b, a) };
            Edge {
                id: EdgeId(u64::from(id)),
                from: VertexId(a as u32),
                to: VertexId(b as u32),
                points: vec![v[a], v[b]],
                class: RoadClass::Secondary,
                name: Some("Climb".into()),
                width_m: 6.5,
                bridge: false,
                tunnel: false,
                dead_end_m: None,
                oneway: ocw_world::graph::Oneway::No,
                boundary: false,
                osm_way: i64::from(id),
                profile: vec![],
            }
        };
        let g = RoadGraph {
            vertices: v.to_vec(),
            edges: vec![
                edge(1, 0, 1, true),
                edge(2, 1, 2, false),
                edge(3, 2, 3, true),
            ],
            boundary_vertices: Default::default(),
        };
        let dem = |p: [f64; 2]| Some(100.0 + 0.05 * p[0]);
        let mut vertex_h = vec![100.0, 121.0, 110.5, 135.0];
        let out = filter_through_roads(&g, &dem, &mut vertex_h);
        assert!((vertex_h[1] - 115.0).abs() < 0.5, "{}", vertex_h[1]);
        assert!((vertex_h[2] - 116.5).abs() < 0.5, "{}", vertex_h[2]);
        // Reassemble the road west to east and check the gradient change.
        let mut road: Vec<f64> = out[0].clone().unwrap();
        let mut mid = out[1].clone().unwrap();
        mid.reverse();
        road.extend_from_slice(&mid[1..]);
        road.extend_from_slice(&out[2].as_ref().unwrap()[1..]);
        let grades: Vec<f64> = road.windows(2).map(|w| (w[1] - w[0]) / 5.0).collect();
        let jump = grades
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f64::max);
        assert!(jump < 0.01, "gradient jumps by {:.1} %", jump * 100.0);
        // Ends meet the junction heights.
        assert!((out[0].as_ref().unwrap().last().unwrap() - vertex_h[1]).abs() < 1e-9);
        assert!((out[1].as_ref().unwrap()[0] - vertex_h[2]).abs() < 1e-9);
    }

    #[test]
    fn bridges_in_a_through_road_ignore_the_valley_below() {
        // Flat road at 200 m, a 60 m bridge over a 30 m deep gorge in the
        // middle, mapped as road - bridge - road.
        use ocw_world::graph::{Edge, EdgeId, VertexId};
        let v = [[0.0, 0.0], [300.0, 0.0], [360.0, 0.0], [700.0, 0.0]];
        let edge = |id: u32, a: usize, b: usize, bridge| Edge {
            id: EdgeId(u64::from(id)),
            from: VertexId(a as u32),
            to: VertexId(b as u32),
            points: vec![v[a], v[b]],
            class: RoadClass::Secondary,
            name: Some("Gorge".into()),
            width_m: 6.5,
            bridge,
            tunnel: false,
            dead_end_m: None,
            oneway: ocw_world::graph::Oneway::No,
            boundary: false,
            osm_way: i64::from(id),
            profile: vec![],
        };
        let g = RoadGraph {
            vertices: v.to_vec(),
            edges: vec![
                edge(1, 0, 1, false),
                edge(2, 1, 2, true),
                edge(3, 2, 3, false),
            ],
            boundary_vertices: Default::default(),
        };
        let dem = |p: [f64; 2]| {
            Some(if p[0] > 300.5 && p[0] < 359.5 {
                170.0
            } else {
                200.0
            })
        };
        let mut vertex_h = vec![200.0, 200.0, 200.0, 200.0];
        let out = filter_through_roads(&g, &dem, &mut vertex_h);
        for p in out.iter().map(|p| p.as_ref().unwrap()) {
            assert!(p.iter().all(|h| (h - 200.0).abs() < 0.01), "{p:?}");
        }
    }

    #[test]
    fn grade_limit_stays_local_on_long_roads() {
        // 5 km at 8 % with three 50 m walls at 40 % (DEM noise): clipping
        // and integrating would leave the rest of the road metres off; the
        // local limit keeps it on the terrain away from the walls.
        let n = 1001;
        let len = 5000.0;
        let mut t: Vec<f64> = (0..n).map(|i| i as f64 * 5.0 * 0.08).collect();
        for &w in &[100usize, 500, 800] {
            for (k, v) in t.iter_mut().enumerate().skip(w) {
                *v += 20.0 * (((k - w) as f64) / 10.0).min(1.0);
            }
        }
        let (a, b) = (t[0], t[n - 1]);
        let h = limit_grade_locally(&t, len, a, b, 0.15);
        assert!(steepest(&h, 1) <= 0.15 + 1e-3, "{}", steepest(&h, 1));
        assert_eq!((h[0], h[n - 1]), (a, b));
        // Away from the walls (> 400 m), the road is where the terrain is.
        for i in [50usize, 300, 700, 950] {
            assert!((h[i] - t[i]).abs() < 0.5, "{i}: {} vs {}", h[i], t[i]);
        }
    }

    #[test]
    fn spikes_are_removed_and_limits_hold() {
        // 1 km climbing 8 % with a 30 m DEM spike, end heights fixed.
        let n = 201;
        let len = 1000.0;
        let mut raw: Vec<f64> = (0..n).map(|i| i as f64 * 5.0 * 0.08).collect();
        raw[100] += 30.0;
        let smooth = gaussian(
            &median_filter(&raw, 2),
            smoothing_sigma_m(RoadClass::Primary) / 5.0,
        );
        let h = constrain(&smooth, len, 0.0, 80.0, 0.15, 0.005);
        assert_eq!(h[0], 0.0);
        assert!((h[n - 1] - 80.0).abs() < 1e-9);
        assert!((h[100] - 40.0).abs() < 1.0, "spike gone: {}", h[100]);
        for w in h.windows(2) {
            assert!(((w[1] - w[0]) / 5.0).abs() <= 0.15 + 1e-6);
        }
        for w in h.windows(3) {
            let (g1, g2) = ((w[1] - w[0]) / 5.0, (w[2] - w[1]) / 5.0);
            // Plus the small linear end correction.
            assert!((g2 - g1).abs() <= 0.005 * 5.0 + 1e-6, "{g1} -> {g2}");
        }
    }

    #[test]
    fn a_wall_is_limited_to_the_class_gradient() {
        // A 30 % DEM wall on a main road: limited to 15 %.
        let raw: Vec<f64> = (0..41).map(|i| (i as f64 * 5.0 * 0.30).min(30.0)).collect();
        let h = constrain(&raw, 200.0, 0.0, 30.0, 0.15, 0.005);
        assert!(steepest(&h, 1) <= 0.15 + 1e-6, "{}", steepest(&h, 1));
        assert!((h[40] - 30.0).abs() < 1e-9);
        assert!(steepest(&raw, 1) > 0.29);
    }

    #[test]
    fn impossible_ends_fall_back_to_a_linear_rest() {
        // 50 m up in 100 m on a 15 % road cannot be done within the limit.
        let raw = vec![0.0; 21];
        let h = constrain(&raw, 100.0, 0.0, 50.0, 0.15, 0.005);
        assert!((h[20] - 50.0).abs() < 1e-9);
        assert!((steepest(&h, 1) - 0.5).abs() < 1e-6, "everything at 50 %");
    }

    #[test]
    fn junction_heights_are_relaxed_on_short_steep_edges() {
        use ocw_world::graph::{Edge, EdgeId, VertexId};
        // 0 —(100 m)— 1 —(13 m)— 2 —(100 m)— 3, residential (20 %); the DEM
        // puts junction 2 6 m above junction 1 (46 % on 13 m).
        let pts = [[0.0, 0.0], [100.0, 0.0], [113.0, 0.0], [213.0, 0.0]];
        let edge = |i: u64, a: u32, b: u32| Edge {
            id: EdgeId(i),
            from: VertexId(a),
            to: VertexId(b),
            points: vec![pts[a as usize], pts[b as usize]],
            class: RoadClass::Residential,
            name: None,
            width_m: 5.0,
            bridge: false,
            tunnel: false,
            dead_end_m: None,
            oneway: ocw_world::graph::Oneway::No,
            boundary: false,
            osm_way: i as i64,
            profile: vec![],
        };
        let g = RoadGraph {
            vertices: pts.to_vec(),
            edges: vec![edge(1, 0, 1), edge(2, 1, 2), edge(3, 2, 3)],
            boundary_vertices: Default::default(),
        };
        let mut h = vec![500.0, 510.0, 516.0, 526.0];
        relax_junction_heights(&g, &mut h);
        for e in &g.edges {
            let (a, b) = (e.from.0 as usize, e.to.0 as usize);
            assert!((h[b] - h[a]).abs() <= 0.2 * e.length_m() + 1e-6, "{h:?}");
        }
        // The short edge got at most 20 %; the neighbours took over the rest.
        assert!((h[2] - h[1] - 2.6).abs() < 1e-6, "{h:?}");
    }

    /// Regression (pilot region): a short connector between two roads
    /// whose junction heights were fixed by their own filtering kept a
    /// 50–200 % ramp (riders stalled on it in free ride). The limit across
    /// junctions spreads it into the neighbouring roads.
    #[test]
    fn short_connectors_between_filtered_roads_keep_their_class_limit() {
        use ocw_world::graph::{Edge, EdgeId, VertexId};
        // 0 —(100 m, flat at 500 m)— 1 —(10 m)— 2 —(100 m, flat at 505 m)— 3;
        // the connector is residential (20 %), the others primary (15 %).
        let pts = [[0.0, 0.0], [100.0, 0.0], [110.0, 0.0], [210.0, 0.0]];
        let edge = |i: u64, a: u32, b: u32, class, profile: Vec<f64>| Edge {
            id: EdgeId(i),
            from: VertexId(a),
            to: VertexId(b),
            points: vec![pts[a as usize], pts[b as usize]],
            class,
            name: None,
            width_m: 5.0,
            bridge: false,
            tunnel: false,
            dead_end_m: None,
            oneway: ocw_world::graph::Oneway::No,
            boundary: false,
            osm_way: i as i64,
            profile,
        };
        let mut g = RoadGraph {
            vertices: pts.to_vec(),
            edges: vec![
                edge(1, 0, 1, RoadClass::Primary, vec![500.0; 21]),
                edge(2, 1, 2, RoadClass::Residential, vec![500.0, 504.0, 505.0]),
                edge(3, 2, 3, RoadClass::Primary, vec![505.0; 21]),
            ],
            boundary_vertices: Default::default(),
        };
        let mut h = vec![500.0, 500.0, 505.0, 505.0];
        assert!(steepest(&g.edges[1].profile, 1) > 0.7);
        limit_grades_across_junctions(&mut g, &mut h);
        for e in &g.edges {
            let limit = limits(e.class).0;
            assert!(steepest(&e.profile, 1) <= limit + 1e-3, "{:?}", e.profile);
            // Continuous at the junctions.
            assert_eq!(e.profile[0], h[e.from.0 as usize]);
            assert_eq!(*e.profile.last().unwrap(), h[e.to.0 as usize]);
        }
        // The height difference stays (spread out), the far ends barely move.
        assert!((h[3] - h[0] - 5.0).abs() < 1e-6, "{h:?}");
        assert!(h[0] > 499.9 && h[3] < 505.1, "{h:?}");
        // Profiles within their limit are left alone.
        let mut flat = g.clone();
        for e in &mut flat.edges {
            e.profile = vec![500.0; e.profile.len()];
        }
        let before = flat.clone();
        limit_grades_across_junctions(&mut flat, &mut [500.0; 4]);
        assert_eq!(flat, before);
    }

    #[test]
    fn class_limits() {
        assert_eq!(limits(RoadClass::Primary).0, 0.15);
        assert_eq!(limits(RoadClass::Path).0, 0.25);
        assert!(limits(RoadClass::Residential).1 > limits(RoadClass::Primary).1);
    }
}
