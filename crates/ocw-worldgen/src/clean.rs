//! World cleaning (spec §4.8): passes that remove what OSM and the DEM get
//! physically wrong, deviating a little from the source data where that is
//! needed for a believable world. Each pass is a separate, pure function;
//! the world builder runs them in this order:
//!
//! 1. [`clean_polylines`]: duplicate nodes and spikes out of the road
//!    centrelines (before the profiles; vertices never move).
//! 2. [`snap_near_misses`]: road ends that stop just short of (or just
//!    beyond) another road are moved onto it (≤ [`SNAP_M`]); the graph's
//!    topology is not changed (no new junction, so stable IDs and routes
//!    stay), only the geometry and, after the profiles,
//!    [`tie_snapped_heights`] the height of the snapped end.
//! 3. After the elevation profiles: [`ease_junction_mouths`] (a side road
//!    leaves a wider road level for its first metres, so the two strips
//!    meet without a step) and [`limit_vertical_curvature`] (gradient
//!    changes per metre within the class limit inside each road).
//! 4. On the package: [`settle_terrain_under_roads`] lowers the terrain
//!    grid by the least amount so that no grid triangle rises through a road
//!    or bridge deck, replacing the wide cut zone around every road that
//!    left the higher of two neighbouring roads floating.

use std::collections::HashMap;

use ocw_world::graph::{PROFILE_STEP_M, RoadClass, RoadGraph, VertexId};
use ocw_world::package::{TileKey, WorldPackage};

use crate::lint::{DUPLICATE_M, SPIKE_LEG_M, SPIKE_TURN_DEG};
use crate::profile::{limit_grade_locally, limits};
use crate::spatial::SegmentIndex;

/// A road end within this distance of another road is snapped onto it.
pub const SNAP_M: f64 = 3.0;
/// Spikes are removed only when the point lies at most this far off the
/// line between its neighbours (larger detours are real roads).
pub const MAX_SPIKE_M: f64 = 15.0;

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    (b[0] - a[0]).hypot(b[1] - a[1])
}

fn turn_deg(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    let (u, v) = ([b[0] - a[0], b[1] - a[1]], [c[0] - b[0], c[1] - b[1]]);
    let (lu, lv) = (u[0].hypot(u[1]), v[0].hypot(v[1]));
    if lu < 1e-9 || lv < 1e-9 {
        return 0.0;
    }
    ((u[0] * v[0] + u[1] * v[1]) / (lu * lv))
        .clamp(-1.0, 1.0)
        .acos()
        .to_degrees()
}

/// What the horizontal passes changed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HorizontalStats {
    pub duplicates_removed: usize,
    pub spikes_removed: usize,
    pub ends_snapped: usize,
    /// Largest move of a snapped end (m).
    pub max_snap_m: f64,
}

/// Removes duplicate interior points (closer than [`DUPLICATE_M`] to the
/// previous one) and spikes (the line turns back by more than
/// [`SPIKE_TURN_DEG`] with a leg shorter than [`SPIKE_LEG_M`] and the
/// point at most [`MAX_SPIKE_M`] off its neighbours' chord) from one
/// centreline. The end points (graph vertices) are always kept.
pub fn clean_polyline(points: &[[f64; 2]]) -> (Vec<[f64; 2]>, usize, usize) {
    let n = points.len();
    if n <= 2 {
        return (points.to_vec(), 0, 0);
    }
    let mut out: Vec<[f64; 2]> = Vec::with_capacity(n);
    let mut dups = 0;
    out.push(points[0]);
    for (i, &p) in points.iter().enumerate().skip(1) {
        let last = i == n - 1;
        if dist(*out.last().expect("non-empty"), p) < DUPLICATE_M {
            dups += 1;
            if last {
                // Keep the end point; drop the interior one before it.
                if out.len() > 1 {
                    out.pop();
                } else {
                    dups -= 1;
                }
                out.push(p);
            }
            continue;
        }
        out.push(p);
    }
    let mut spikes = 0;
    loop {
        let mut removed = false;
        let mut i = 1;
        while i + 1 < out.len() {
            let (a, b, c) = (out[i - 1], out[i], out[i + 1]);
            let (la, lc) = (dist(a, b), dist(b, c));
            let off = crate::spatial::foot_on_segment(b, a, c).0;
            if turn_deg(a, b, c) > SPIKE_TURN_DEG && la.min(lc) < SPIKE_LEG_M && off <= MAX_SPIKE_M
            {
                out.remove(i);
                spikes += 1;
                removed = true;
            } else {
                i += 1;
            }
        }
        if !removed {
            break;
        }
    }
    (out, dups, spikes)
}

/// [`clean_polyline`] on every edge of the graph.
pub fn clean_polylines(graph: &mut RoadGraph) -> HorizontalStats {
    let mut s = HorizontalStats::default();
    for e in &mut graph.edges {
        let (pts, d, k) = clean_polyline(&e.points);
        if pts.len() >= 2 {
            e.points = pts;
            s.duplicates_removed += d;
            s.spikes_removed += k;
        }
    }
    s
}

/// A road end moved onto another road by [`snap_near_misses`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Snap {
    pub vertex: VertexId,
    /// The edge whose end it is.
    pub edge: usize,
    /// The road it was moved onto, and where along it.
    pub onto: usize,
    pub along: f64,
    pub moved_m: f64,
}

/// Moves every dead end (a vertex with one road, not at the region border)
/// that lies within [`SNAP_M`] of another road onto that road's
/// centreline. Tunnels and bridges are neither snapped nor snapped onto
/// (they are at another level).
pub fn snap_near_misses(graph: &mut RoadGraph) -> (Vec<Snap>, HorizontalStats) {
    let mut stats = HorizontalStats::default();
    let adj = graph.adjacency();
    let index = SegmentIndex::new(graph, 20.0, |e| {
        !graph.edges[e].tunnel && !graph.edges[e].bridge
    });
    let mut snaps = Vec::new();
    for (v, edges) in adj.iter().enumerate() {
        let vid = VertexId(v as u32);
        if edges.len() != 1 || graph.boundary_vertices.contains(&vid) {
            continue;
        }
        let own = edges[0];
        let e = &graph.edges[own];
        if e.tunnel || e.bridge {
            continue;
        }
        let p = graph.vertices[v];
        let Some(hit) = index.near(graph, p, SNAP_M).into_iter().find(|h| {
            let o = &graph.edges[h.edge];
            h.edge != own && o.from != vid && o.to != vid && h.dist > 1e-3
        }) else {
            continue;
        };
        snaps.push(Snap {
            vertex: vid,
            edge: own,
            onto: hit.edge,
            along: hit.along,
            moved_m: hit.dist,
        });
        stats.max_snap_m = stats.max_snap_m.max(hit.dist);
    }
    // Moved after the search, so later ends see the original geometry.
    for s in &snaps {
        let foot = {
            let o = &graph.edges[s.onto];
            point_at(&o.points, s.along)
        };
        graph.vertices[s.vertex.0 as usize] = foot;
        let e = &mut graph.edges[s.edge];
        if e.from == s.vertex {
            e.points[0] = foot;
        } else {
            *e.points.last_mut().expect("points") = foot;
        }
    }
    stats.ends_snapped = snaps.len();
    (snaps, stats)
}

/// The point at arc length `d` along a polyline.
pub fn point_at(points: &[[f64; 2]], d: f64) -> [f64; 2] {
    let mut walked = 0.0;
    for w in points.windows(2) {
        let len = dist(w[0], w[1]);
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

/// Gives each snapped end the height of the road it was snapped onto and
/// re-limits its road between its ends (spec §4.3: the class gradient,
/// locally). Without it the end hangs above or below the other road by
/// whatever the two DEM profiles differ.
pub fn tie_snapped_heights(graph: &mut RoadGraph, snaps: &[Snap]) -> usize {
    let mut tied = 0;
    for s in snaps {
        let Some(target) = graph.edges[s.onto].height_at(s.along) else {
            continue;
        };
        let e = &mut graph.edges[s.edge];
        let n = e.profile.len();
        if n < 2 {
            continue;
        }
        let len = e.length_m();
        let at_start = e.from == s.vertex;
        let (start, end) = if at_start {
            (target, e.profile[n - 1])
        } else {
            (e.profile[0], target)
        };
        e.profile = limit_grade_locally(&e.profile, len, start, end, limits(e.class).0);
        tied += 1;
    }
    tied
}

/// How a road ranks at a junction: wider roads and higher classes first.
fn rank(class: RoadClass, width: f64) -> (u8, i64) {
    let c = match class {
        RoadClass::Trunk => 9,
        RoadClass::Primary => 8,
        RoadClass::Secondary => 7,
        RoadClass::Tertiary => 6,
        RoadClass::Unclassified => 5,
        RoadClass::Residential => 4,
        RoadClass::Service => 3,
        RoadClass::Cycleway => 2,
        RoadClass::Track => 1,
        RoadClass::Path => 0,
    };
    (c, (width * 100.0) as i64)
}

/// Largest step tolerated where a side road's strip lies on the wider
/// road's surface (m).
pub const MOUTH_STEP_M: f64 = 0.15;

/// A side road leaves a wider road at that road's level: the main road is
/// drawn flat across, so a side road climbing at 20 % from the junction
/// stands 0.75 m above the main road's surface at its edge (3.75 m out) –
/// a visible step. The first profile interval of every side road at a
/// junction with a wider road is limited to `MOUTH_STEP_M / mouth`
/// (mouth = the wider road's half width) – never so far that the far end
/// could no longer be reached within the class gradient – and the rest of
/// the side road is re-limited to its class gradient from there. Bridges and tunnels are
/// left alone. Returns the number of road ends changed.
pub fn ease_junction_mouths(graph: &mut RoadGraph) -> usize {
    let adj = graph.adjacency();
    let mut changed = 0;
    for (v, edges) in adj.iter().enumerate() {
        if edges.len() < 2 {
            continue;
        }
        let major = edges
            .iter()
            .copied()
            .max_by_key(|&i| {
                let e = &graph.edges[i];
                (rank(e.class, e.width_m), std::cmp::Reverse(i))
            })
            .expect("edges");
        let (mrank, mouth) = {
            let m = &graph.edges[major];
            (rank(m.class, m.width_m), m.width_m / 2.0)
        };
        for &ei in edges {
            let e = &mut graph.edges[ei];
            if ei == major || e.bridge || e.tunnel || rank(e.class, e.width_m) >= mrank {
                continue;
            }
            let n = e.profile.len();
            let len = e.length_m();
            if n < 3 || len < 2.0 * PROFILE_STEP_M {
                continue;
            }
            let at_start = e.from.0 as usize == v;
            let mut p = e.profile.clone();
            if !at_start {
                p.reverse();
            }
            // Never so flat that the far end is out of the class gradient.
            let need = (p[n - 1] - p[0]).abs() - limits(e.class).0 * (len - PROFILE_STEP_M);
            let allowed = (MOUTH_STEP_M / mouth.max(0.5) * PROFILE_STEP_M).max(need);
            let dh = p[1] - p[0];
            if dh.abs() <= allowed + 1e-9 {
                continue;
            }
            p[1] = p[0] + allowed * dh.signum();
            // The rest between the eased sample and the far end.
            let rest = limit_grade_locally(
                &p[1..],
                len - PROFILE_STEP_M,
                p[1],
                p[n - 1],
                limits(e.class).0,
            );
            p[1..].copy_from_slice(&rest);
            // Both ends eased on a short steep road: no room left for the
            // class gradient – keep the road as it was.
            let limit = limits(e.class).0 + 1e-3 / PROFILE_STEP_M;
            if crate::profile::steepest(&p, 1) > limit.max(crate::profile::steepest(&e.profile, 1))
            {
                continue;
            }
            if !at_start {
                p.reverse();
            }
            e.profile = p;
            changed += 1;
        }
    }
    changed
}

/// Limits the change of gradient per metre inside every road (not at its
/// ends, which are shared junction heights) to its class limit (spec §4.3
/// step 2): a kink is rounded into a vertical curve by moving its sample
/// towards the line between its neighbours. Never steepens (the new
/// gradients lie between the old ones). Bridges and tunnels are planned
/// separately. Returns the number of samples moved.
pub fn limit_vertical_curvature(graph: &mut RoadGraph) -> usize {
    let mut moved = 0;
    for e in &mut graph.edges {
        if e.bridge || e.tunnel || e.profile.len() < 3 {
            continue;
        }
        let max_change = limits(e.class).1;
        let len = e.length_m();
        moved += round_kinks(&mut e.profile, len, max_change);
    }
    moved
}

/// [`limit_vertical_curvature`] on one profile.
pub fn round_kinks(p: &mut [f64], len: f64, max_change_per_m: f64) -> usize {
    let n = p.len();
    let d: Vec<f64> = (0..n)
        .map(|i| (i as f64 * PROFILE_STEP_M).min(len))
        .collect();
    let mut touched = vec![false; n];
    for _ in 0..400 {
        let mut worst = 0.0f64;
        for i in 1..n - 1 {
            let (d1, d2) = (d[i] - d[i - 1], d[i + 1] - d[i]);
            if d1 < 0.5 || d2 < 0.5 {
                continue;
            }
            let (g1, g2) = ((p[i] - p[i - 1]) / d1, (p[i + 1] - p[i]) / d2);
            let allowed = max_change_per_m * (d1 + d2) / 2.0;
            let excess = (g2 - g1).abs() - allowed;
            if excess <= 1e-6 {
                continue;
            }
            worst = worst.max(excess);
            // Height on the chord between the neighbours.
            let chord = p[i - 1] + (p[i + 1] - p[i - 1]) * d1 / (d1 + d2);
            let frac = (excess / (g2 - g1).abs()).min(1.0);
            p[i] += (chord - p[i]) * frac * 0.5;
            touched[i] = true;
        }
        if worst < 1e-4 {
            break;
        }
    }
    touched.iter().filter(|t| **t).count()
}

/// What [`settle_terrain_under_roads`] changed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SettleStats {
    /// Road samples where the terrain rose through the surface.
    pub violations: usize,
    pub vertices_lowered: usize,
    pub max_lowered_m: f64,
}

/// Lowers the terrain grid by the least amount so that its triangles (split
/// like the client mesh) stay at least `clearance` below every road strip
/// and bridge deck of the package, sampled every metre along and every
/// 0.5 m across. The constraints are linear in the vertex heights, and
/// lowering never breaks one that holds, so a Gauss–Seidel projection over
/// the violated samples converges. Border vertices shared by neighbouring
/// tiles are lowered in every copy.
pub fn settle_terrain_under_roads(pkg: &mut WorldPackage, clearance: f64) -> SettleStats {
    let n = pkg.samples as usize;
    let step = (n - 1) as i64;
    let sp = pkg.spacing_m();
    let height = |pkg: &WorldPackage, gi: i64, gk: i64| -> Option<f64> {
        let key = TileKey {
            x: gi.div_euclid(step) as i32,
            z: gk.div_euclid(step) as i32,
        };
        let t = pkg.tiles.get(&key)?;
        let (i, k) = (gi.rem_euclid(step) as usize, gk.rem_euclid(step) as usize);
        Some(f64::from(t.heights[k * n + i]))
    };
    // Constraints: three vertices with weights, and the highest allowed
    // surface there.
    struct C {
        v: [(i64, i64); 3],
        w: [f64; 3],
        max: f64,
    }
    let mut cons: Vec<C> = Vec::new();
    for tile in pkg.tiles.values() {
        for r in &tile.roads {
            let half = f64::from(r.width_m) / 2.0;
            for w in r.points.windows(2) {
                let (a, b) = (w[0].map(f64::from), w[1].map(f64::from));
                let len = (b[0] - a[0]).hypot(b[2] - a[2]);
                if len < 1e-6 {
                    continue;
                }
                let (dx, dz) = ((b[0] - a[0]) / len, (b[2] - a[2]) / len);
                let steps = len.ceil() as usize;
                for j in 0..=steps {
                    let t = (j as f64 / steps as f64).min(1.0);
                    let c = [
                        a[0] + (b[0] - a[0]) * t,
                        a[1] + (b[1] - a[1]) * t,
                        a[2] + (b[2] - a[2]) * t,
                    ];
                    let mut off = -half;
                    while off <= half + 1e-9 {
                        let (x, z) = (c[0] - dz * off, c[2] + dx * off);
                        let (fx, fz) = (x / sp, z / sp);
                        let (gi, gk) = (fx.floor() as i64, fz.floor() as i64);
                        let (tx, tz) = (fx - gi as f64, fz - gk as f64);
                        let (v, wt) = if tx + tz <= 1.0 {
                            (
                                [(gi, gk), (gi + 1, gk), (gi, gk + 1)],
                                [1.0 - tx - tz, tx, tz],
                            )
                        } else {
                            (
                                [(gi + 1, gk + 1), (gi, gk + 1), (gi + 1, gk)],
                                [tx + tz - 1.0, 1.0 - tx, 1.0 - tz],
                            )
                        };
                        let hs: Option<Vec<f64>> =
                            v.iter().map(|&(i, k)| height(pkg, i, k)).collect();
                        if let Some(hs) = hs {
                            let surf: f64 = hs.iter().zip(&wt).map(|(h, w)| h * w).sum();
                            let max = c[1] - clearance;
                            if surf > max + 1e-4 {
                                cons.push(C { v, w: wt, max });
                            }
                        }
                        off += 0.5;
                    }
                }
            }
        }
    }
    let mut stats = SettleStats {
        violations: cons.len(),
        ..Default::default()
    };
    if cons.is_empty() {
        return stats;
    }
    let mut h: HashMap<(i64, i64), f64> = HashMap::new();
    for c in &cons {
        for &v in &c.v {
            h.entry(v)
                .or_insert_with(|| height(pkg, v.0, v.1).expect("checked"));
        }
    }
    let start = h.clone();
    for _ in 0..200 {
        let mut worst = 0.0f64;
        for c in &cons {
            let surf: f64 = c.v.iter().zip(&c.w).map(|(v, w)| h[v] * w).sum();
            let excess = surf - c.max;
            if excess <= 1e-4 {
                continue;
            }
            worst = worst.max(excess);
            let w2: f64 = c.w.iter().map(|w| w * w).sum::<f64>().max(1e-9);
            for (v, w) in c.v.iter().zip(&c.w) {
                *h.get_mut(v).expect("vertex") -= excess * w / w2;
            }
        }
        if worst < 1e-3 {
            break;
        }
    }
    for (&(gi, gk), &new) in &h {
        let old = start[&(gi, gk)];
        if new >= old - 1e-6 {
            continue;
        }
        stats.vertices_lowered += 1;
        stats.max_lowered_m = stats.max_lowered_m.max(old - new);
        set_vertex(pkg, gi, gk, new as f32);
    }
    stats
}

/// Where the terrain belongs to a road: within its half width plus a margin
/// of a road on the ground (no tunnels, no bridges, no bridge spans).
pub struct RoadZone<'a> {
    graph: &'a RoadGraph,
    index: SegmentIndex,
    spans: HashMap<usize, Vec<(f64, f64)>>,
    max_half: f64,
}

impl<'a> RoadZone<'a> {
    /// `spans`: bridge stretches of untagged edges (edge, from, to m).
    pub fn new(graph: &'a RoadGraph, spans: &[(usize, f64, f64)]) -> Self {
        let mut by_edge: HashMap<usize, Vec<(f64, f64)>> = HashMap::new();
        for &(e, a, b) in spans {
            by_edge.entry(e).or_default().push((a, b));
        }
        Self {
            index: SegmentIndex::new(graph, 20.0, |e| {
                !graph.edges[e].tunnel && !graph.edges[e].bridge
            }),
            max_half: graph
                .edges
                .iter()
                .map(|e| e.width_m / 2.0)
                .fold(0.0, f64::max),
            graph,
            spans: by_edge,
        }
    }

    /// Whether `p` lies within a ground road's half width plus `margin`.
    pub fn covers(&self, p: [f64; 2], margin: f64) -> bool {
        self.index
            .near(self.graph, p, self.max_half + margin)
            .iter()
            .any(|h| {
                h.dist <= self.graph.edges[h.edge].width_m / 2.0 + margin
                    && !self
                        .spans
                        .get(&h.edge)
                        .is_some_and(|v| v.iter().any(|&(a, b)| h.along >= a && h.along <= b))
            })
    }
}

/// What [`level_building_sites`] changed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SiteStats {
    pub sites: usize,
    pub vertices: usize,
    pub max_change_m: f64,
}

/// Most a building site moves the terrain (m): steeper sites keep their
/// slope (a house set into the hillside, a basement on the downhill side).
pub const MAX_SITE_CHANGE_M: f64 = 1.5;

/// Sets a global terrain vertex in every tile that holds it (tiles hold
/// both borders, so up to four copies at a corner). Returns the old height.
pub fn set_vertex(pkg: &mut WorldPackage, gi: i64, gk: i64, h: f32) -> Option<f32> {
    let n = pkg.samples as usize;
    let step = (n - 1) as i64;
    let mut old = None;
    for (dx, dz) in [(0i64, 0i64), (-1, 0), (0, -1), (-1, -1)] {
        let (mut i, mut k) = (gi.rem_euclid(step) as usize, gk.rem_euclid(step) as usize);
        if dx != 0 {
            if i != 0 {
                continue;
            }
            i = n - 1;
        }
        if dz != 0 {
            if k != 0 {
                continue;
            }
            k = n - 1;
        }
        let key = TileKey {
            x: (gi.div_euclid(step) + dx) as i32,
            z: (gk.div_euclid(step) + dz) as i32,
        };
        if let Some(t) = pkg.tiles.get_mut(&key) {
            old = old.or(Some(t.heights[k * n + i]));
            t.heights[k * n + i] = h;
        }
    }
    old
}

/// Levels the ground under each building footprint: the terrain vertices
/// inside the outline (and within 1 m of it) move to the mean ground of the
/// footprint, by at most [`MAX_SITE_CHANGE_M`], except where `keep` says
/// (under roads and water). Run before the buildings are placed, which then
/// stand on a level base instead of a wall foot sunk into the uphill side.
pub fn level_building_sites(
    pkg: &mut WorldPackage,
    outlines: &[Vec<[f64; 2]>],
    keep: &impl Fn([f64; 2]) -> bool,
) -> SiteStats {
    let sp = pkg.spacing_m();
    let mut stats = SiteStats::default();
    let mut new_h: std::collections::BTreeMap<(i64, i64), f64> = Default::default();
    for ring in outlines {
        if ring.len() < 3 {
            continue;
        }
        let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for p in ring {
            lo = [lo[0].min(p[0]), lo[1].min(p[1])];
            hi = [hi[0].max(p[0]), hi[1].max(p[1])];
        }
        let rings = vec![ring.clone()];
        let near = |p: [f64; 2]| {
            crate::areas::inside_rings(p, &rings)
                || (0..ring.len()).any(|i| {
                    crate::spatial::foot_on_segment(p, ring[i], ring[(i + 1) % ring.len()]).0 <= 1.0
                })
        };
        let mut verts = Vec::new();
        for gk in ((lo[1] - 1.0) / sp).ceil() as i64..=((hi[1] + 1.0) / sp).floor() as i64 {
            for gi in ((lo[0] - 1.0) / sp).ceil() as i64..=((hi[0] + 1.0) / sp).floor() as i64 {
                let p = [gi as f64 * sp, gk as f64 * sp];
                if near(p) && !keep(p) {
                    verts.push((gi, gk, p));
                }
            }
        }
        if verts.is_empty() {
            continue;
        }
        // The mean ground of the footprint (its corners and centre).
        let mut hs: Vec<f64> = ring.iter().filter_map(|p| pkg.height(p[0], p[1])).collect();
        hs.extend(pkg.height((lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0));
        if hs.is_empty() {
            continue;
        }
        let mean = hs.iter().sum::<f64>() / hs.len() as f64;
        stats.sites += 1;
        for (gi, gk, p) in verts {
            if let Some(h) = pkg.height(p[0], p[1]) {
                let target = h + (mean - h).clamp(-MAX_SITE_CHANGE_M, MAX_SITE_CHANGE_M);
                new_h.insert((gi, gk), target);
            }
        }
    }
    for (&(gi, gk), &h) in &new_h {
        if let Some(old) = set_vertex(pkg, gi, gk, h as f32) {
            stats.vertices += 1;
            stats.max_change_m = stats.max_change_m.max((h - f64::from(old)).abs());
        }
    }
    stats
}

/// What [`clean_landuse`] changed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LandUseCleanStats {
    pub slivers: usize,
    pub gaps: usize,
    pub cells: usize,
}

/// Removes land-use slivers and gaps from the splat map (spec §4.8.5): a
/// patch of at most [`crate::lint::SLIVER_CELLS`] cells of one class (not
/// water: narrow streams are long, not small) takes the most common class
/// around it, and an unclassified hole of at most [`crate::lint::GAP_CELLS`]
/// cells enclosed by one class (the seam between two OSM polygons that do
/// not quite touch) takes that class. Patches at a tile edge are left
/// alone (they may continue in the next tile).
pub fn clean_landuse(lu: &mut ocw_world::landuse::LandUse) -> LandUseCleanStats {
    use ocw_world::landuse::LandClass;
    let n = lu.cells as usize;
    let mut stats = LandUseCleanStats::default();
    for t in lu.tiles.values_mut() {
        let small = crate::lint::small_components(&t.classes, n, crate::lint::GAP_CELLS);
        for (class, cells, around) in small {
            let fill = if class == LandClass::Default {
                if around.len() != 1 {
                    continue;
                }
                stats.gaps += 1;
                around[0]
            } else if class != LandClass::Water && cells.len() <= crate::lint::SLIVER_CELLS {
                // The most common neighbouring class.
                let mut count: HashMap<LandClass, usize> = HashMap::new();
                for &c in &cells {
                    for d in [c - 1, c + 1, c - n, c + n] {
                        if t.classes[d] != class {
                            *count.entry(t.classes[d]).or_default() += 1;
                        }
                    }
                }
                let Some((&best, _)) = count
                    .iter()
                    .max_by_key(|(c, k)| (**k, std::cmp::Reverse(c.code())))
                else {
                    continue;
                };
                stats.slivers += 1;
                best
            } else {
                continue;
            };
            stats.cells += cells.len();
            for c in cells {
                t.classes[c] = fill;
            }
        }
    }
    stats
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_world::graph::{Edge, EdgeId, Oneway};
    use ocw_world::package::{RoadStrip, Tile};

    fn edge(points: Vec<[f64; 2]>, from: u32, to: u32, class: RoadClass) -> Edge {
        Edge {
            id: EdgeId(u64::from(from) * 1000 + u64::from(to)),
            from: VertexId(from),
            to: VertexId(to),
            points,
            class,
            name: None,
            width_m: class.default_width_m(),
            bridge: false,
            tunnel: false,
            dead_end_m: None,
            boundary: false,
            oneway: Oneway::No,
            osm_way: i64::from(from),
            profile: Vec::new(),
        }
    }

    fn flat_profile(e: &mut Edge, h: f64) {
        let n = crate::profile::resample(&e.points, PROFILE_STEP_M).len();
        e.profile = vec![h; n];
    }

    #[test]
    fn duplicates_and_spikes_leave_the_centreline() {
        let pts = vec![
            [0.0, 0.0],
            [10.0, 0.0],
            [10.1, 0.0],
            [20.0, 0.0],
            // A spike: out 8 m and straight back.
            [28.0, 0.0],
            [21.0, 0.3],
            [30.0, 0.0],
            [40.0, 0.0],
            [40.05, 0.0],
        ];
        let (out, dups, spikes) = clean_polyline(&pts);
        assert_eq!(dups, 2);
        assert!(spikes >= 1, "{out:?}");
        assert_eq!(out.first(), Some(&[0.0, 0.0]));
        assert_eq!(out.last(), Some(&[40.05, 0.0]), "the vertex stays");
        let (d, s, x) = crate::lint::polyline_faults(&out);
        assert!(d.is_empty() && s.is_empty() && x.is_empty(), "{out:?}");
        // A hairpin (long legs) is a real road and stays.
        let hairpin = vec![[0.0, 0.0], [50.0, 0.0], [52.0, 4.0], [0.0, 8.0]];
        assert_eq!(clean_polyline(&hairpin).0, hairpin);
    }

    /// A side road ending 2 m short of a main road is snapped onto it and
    /// takes its height; the topology (edges, vertices) does not change.
    #[test]
    fn near_miss_ends_are_snapped_and_tied() {
        let mut main = edge(vec![[0.0, 0.0], [100.0, 0.0]], 0, 1, RoadClass::Secondary);
        let n = 21;
        main.profile = (0..n).map(|i| 400.0 + i as f64 * 0.25).collect();
        let mut side = edge(
            vec![[50.0, 2.0], [50.0, 60.0]],
            2,
            3,
            RoadClass::Residential,
        );
        flat_profile(&mut side, 395.0);
        let mut g = RoadGraph {
            vertices: vec![[0.0, 0.0], [100.0, 0.0], [50.0, 2.0], [50.0, 60.0]],
            edges: vec![main, side],
            boundary_vertices: Default::default(),
        };
        let ids: Vec<EdgeId> = g.edges.iter().map(|e| e.id).collect();
        let (snaps, stats) = snap_near_misses(&mut g);
        assert_eq!(stats.ends_snapped, 1);
        assert!((stats.max_snap_m - 2.0).abs() < 1e-9);
        assert_eq!(g.vertices[2], [50.0, 0.0]);
        assert_eq!(g.edges[1].points[0], [50.0, 0.0]);
        assert_eq!(g.edges.iter().map(|e| e.id).collect::<Vec<_>>(), ids);
        assert_eq!(tie_snapped_heights(&mut g, &snaps), 1);
        let h = g.edges[0].height_at(50.0).unwrap();
        assert!((g.edges[1].profile[0] - h).abs() < 1e-9);
        // Still within its class gradient.
        assert!(
            // `limit_grade_locally` stops at 1 mm of excess per interval.
            crate::profile::steepest(&g.edges[1].profile, 1) <= 0.20 + 1e-3 / PROFILE_STEP_M,
            "{:?}",
            g.edges[1].profile
        );
        // Far ends (> 3 m) and the border are not snapped.
        let mut far = g.clone();
        far.vertices[3] = [50.0, 60.0];
        far.edges[1].points = vec![[50.0, 60.0], [50.0, 120.0]];
        far.vertices[2] = [50.0, 120.0];
        assert_eq!(snap_near_misses(&mut far).1.ends_snapped, 0);
    }

    #[test]
    fn side_roads_leave_wide_roads_level() {
        // A 7.5 m primary road at 400 m and a residential road climbing
        // at 20 % away from it.
        let mut main = edge(vec![[-100.0, 0.0], [100.0, 0.0]], 0, 1, RoadClass::Primary);
        flat_profile(&mut main, 400.0);
        let mut side = edge(vec![[0.0, 0.0], [0.0, 100.0]], 1, 2, RoadClass::Residential);
        side.from = VertexId(1);
        side.profile = (0..21).map(|i| 400.0 + i as f64 * 0.5).collect();
        let mut g = RoadGraph {
            vertices: vec![[-100.0, 0.0], [0.0, 0.0], [0.0, 100.0]],
            edges: vec![main, side],
            boundary_vertices: Default::default(),
        };
        g.edges[0].to = VertexId(1);
        assert_eq!(ease_junction_mouths(&mut g), 1);
        let p = &g.edges[1].profile;
        let mouth = 7.5 / 2.0;
        // At the main road's edge the side road is within the step limit.
        let at_edge = p[0] + (p[1] - p[0]) * mouth / PROFILE_STEP_M;
        assert!(at_edge - 400.0 <= MOUTH_STEP_M + 1e-6, "{p:?}");
        assert_eq!(p[0], 400.0);
        assert_eq!(*p.last().unwrap(), 410.0, "the far end keeps its height");
        assert!(crate::profile::steepest(p, 1) <= 0.20 + 1e-6);
        // At the class limit all the way there is no room: unchanged.
        let mut steep = g.clone();
        steep.edges[1].profile = (0..21).map(|i| 400.0 + i as f64).collect();
        assert_eq!(ease_junction_mouths(&mut steep), 0);
        // A short track at 22 % between two primaries: easing both ends
        // would break its limit, so it stays within it.
        let mut two = g.clone();
        two.vertices.push([0.0, 40.0]);
        let mut top = two.edges[0].clone();
        top.from = VertexId(3);
        top.to = VertexId(4);
        top.points = vec![[-100.0, 40.0], [100.0, 40.0]];
        top.profile = vec![410.0; 41];
        two.vertices.push([100.0, 40.0]);
        two.edges.push(top);
        let mut track = edge(vec![[0.0, 0.0], [0.0, 40.0]], 1, 3, RoadClass::Track);
        track.profile = (0..9).map(|i| 400.0 + i as f64 * 1.1).collect();
        two.edges[1] = track;
        ease_junction_mouths(&mut two);
        assert!(crate::profile::steepest(&two.edges[1].profile, 1) <= 0.25 + 1e-3 / PROFILE_STEP_M);
        // The main road is not touched.
        assert!(g.edges[0].profile.iter().all(|&h| h == 400.0));
    }

    #[test]
    fn kinks_become_vertical_curves() {
        // Flat, then 15 % up: a kink of 15 points at 50 m.
        let len = 100.0;
        let mut p: Vec<f64> = (0..21)
            .map(|i| {
                let d = i as f64 * 5.0;
                if d <= 50.0 { 0.0 } else { (d - 50.0) * 0.15 }
            })
            .collect();
        let moved = round_kinks(&mut p, len, 0.005);
        assert!(moved > 0);
        let g: Vec<f64> = p.windows(2).map(|w| (w[1] - w[0]) / 5.0).collect();
        for w in g.windows(2) {
            assert!((w[1] - w[0]).abs() <= 0.005 * 5.0 + 1e-3, "{g:?}");
        }
        assert!(
            g.iter().all(|&x| (-1e-9..=0.15 + 1e-9).contains(&x)),
            "never steeper"
        );
        assert_eq!((p[0], p[20]), (0.0, 7.5), "ends fixed");
    }

    fn tile_pkg(heights: f32, roads: Vec<RoadStrip>) -> WorldPackage {
        let n = 101usize;
        let key = TileKey { x: 0, z: 0 };
        let mut tiles = std::collections::BTreeMap::new();
        tiles.insert(
            key,
            Tile {
                key,
                heights: vec![heights; n * n],
                roads,
            },
        );
        WorldPackage {
            name: "t".into(),
            origin: (47.0, 9.0),
            tile_size_m: 1000.0,
            samples: n as u32,
            tiles,
            graph: None,
            landuse: None,
            buildings: None,
            bridges: None,
            time_zone: None,
            routes: vec![],
            region: None,
        }
    }

    /// Spec §4.8.3: no grid triangle rises through a road, and the terrain
    /// is lowered only where needed (not a wide cut around the road).
    #[test]
    fn terrain_settles_under_roads_by_the_least_amount() {
        let road = RoadStrip {
            class: RoadClass::Residential,
            width_m: 5.0,
            points: vec![[300.0, 99.0, 503.0], [700.0, 99.0, 503.0]],
        };
        let mut pkg = tile_pkg(100.0, vec![road]);
        let before = pkg.road_pokes(1.0).2;
        assert!(before > 0);
        let s = settle_terrain_under_roads(&mut pkg, 0.05);
        assert!(s.violations > 0 && s.vertices_lowered > 0);
        let (worst, _, above, _) = pkg.road_pokes(0.5);
        assert_eq!(above, 0, "worst {worst}");
        // Vertices two cells away from the road keep their height.
        assert_eq!(pkg.height(500.0, 530.0), Some(100.0));
        assert_eq!(pkg.height(500.0, 470.0), Some(100.0));
        // Under the road the terrain is just below it.
        let h = pkg.surface_height(500.0, 503.0).unwrap();
        assert!(h <= 99.0 - 0.05 + 1e-3 && h > 97.0, "{h}");
    }

    #[test]
    fn shared_border_vertices_change_in_every_tile() {
        let mut pkg = tile_pkg(100.0, vec![]);
        let key = TileKey { x: 1, z: 0 };
        let mut t = pkg.tiles[&TileKey { x: 0, z: 0 }].clone();
        t.key = key;
        pkg.tiles.insert(key, t);
        assert_eq!(set_vertex(&mut pkg, 100, 7, 50.0), Some(100.0));
        let n = 101;
        assert_eq!(
            pkg.tiles[&TileKey { x: 0, z: 0 }].heights[7 * n + 100],
            50.0
        );
        assert_eq!(pkg.tiles[&key].heights[7 * n], 50.0);
    }

    #[test]
    fn building_sites_are_levelled_except_under_roads() {
        let mut pkg = tile_pkg(100.0, vec![]);
        let n = 101;
        // A slope of 20 % along x.
        for k in 0..n {
            for i in 0..n {
                pkg.tiles.get_mut(&TileKey { x: 0, z: 0 }).unwrap().heights[k * n + i] =
                    100.0 + i as f32 * 2.0;
            }
        }
        let house = vec![
            [495.0, 495.0],
            [515.0, 495.0],
            [515.0, 515.0],
            [495.0, 515.0],
        ];
        let before = (
            pkg.height(500.0, 500.0).unwrap(),
            pkg.height(510.0, 510.0).unwrap(),
        );
        let s = level_building_sites(&mut pkg, std::slice::from_ref(&house), &|_| false);
        assert_eq!(s.sites, 1);
        assert!(s.vertices >= 4);
        let after = (
            pkg.height(500.0, 500.0).unwrap(),
            pkg.height(510.0, 510.0).unwrap(),
        );
        assert!(
            (after.0 - after.1).abs() < (before.0 - before.1).abs(),
            "{before:?} {after:?}"
        );
        assert!(s.max_change_m <= MAX_SITE_CHANGE_M + 1e-6);
        // Kept vertices (under a road) do not move.
        let mut pkg2 = tile_pkg(100.0, vec![]);
        let kept = level_building_sites(&mut pkg2, &[house], &|_| true);
        assert_eq!(kept.vertices, 0);
    }

    #[test]
    fn slivers_and_gaps_are_filled() {
        use ocw_world::landuse::{LandClass, LandUse, LandUseTile};
        let n = 10usize;
        let mut classes = vec![LandClass::Forest; n * n];
        classes[3 * n + 3] = LandClass::Meadow; // a 1-cell sliver
        classes[6 * n + 6] = LandClass::Default; // a gap in the forest
        classes[6 * n + 7] = LandClass::Default;
        classes[0] = LandClass::Meadow; // at the tile edge: kept
        let mut lu = LandUse {
            cells: n as u32,
            tiles: [(
                TileKey { x: 0, z: 0 },
                LandUseTile {
                    classes,
                    water: vec![],
                },
            )]
            .into(),
        };
        let s = clean_landuse(&mut lu);
        assert_eq!((s.slivers, s.gaps, s.cells), (1, 1, 3));
        let t = &lu.tiles[&TileKey { x: 0, z: 0 }];
        assert_eq!(t.classes[3 * n + 3], LandClass::Forest);
        assert_eq!(t.classes[6 * n + 6], LandClass::Forest);
        assert_eq!(t.classes[0], LandClass::Meadow);
    }

    #[test]
    fn road_zones_leave_out_bridges_and_spans() {
        let mut a = edge(vec![[0.0, 0.0], [100.0, 0.0]], 0, 1, RoadClass::Residential);
        flat_profile(&mut a, 1.0);
        let mut b = edge(
            vec![[0.0, 50.0], [100.0, 50.0]],
            2,
            3,
            RoadClass::Residential,
        );
        b.bridge = true;
        let g = RoadGraph {
            vertices: vec![[0.0, 0.0], [100.0, 0.0], [0.0, 50.0], [100.0, 50.0]],
            edges: vec![a, b],
            boundary_vertices: Default::default(),
        };
        let z = RoadZone::new(&g, &[(0, 40.0, 60.0)]);
        assert!(z.covers([10.0, 4.0], 2.0));
        assert!(!z.covers([10.0, 6.0], 2.0));
        assert!(!z.covers([50.0, 0.0], 2.0), "on the detected bridge span");
        assert!(!z.covers([10.0, 50.0], 2.0), "on a bridge");
    }
}
