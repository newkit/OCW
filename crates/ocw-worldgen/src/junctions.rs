//! Smooth junctions in the world builder (spec §4.8.8 J2): the junction
//! surfaces of `ocw_world::junction` baked into the package.
//!
//! 1. [`smooth_through_kinks`]: the vertical-curvature limit of 4.8.3
//!    across a junction for its through pair (the two arms of a patch's
//!    through road, the two edges of a way split), so a through road does
//!    not change its grade at a junction (Valünastrasse: −3 % / −25 %).
//! 2. [`reconcile_profiles`]: every road's profile near a junction surface
//!    takes the drawn heights (inside the patch the surface along its
//!    centre line, along a side road's blend the blended rows), so the
//!    package's road-graph heights are the drawn heights; the side-road
//!    blend replaces [`crate::clean::ease_junction_mouths`] at the patched
//!    vertices ([`patched_vertices`]).
//! 3. [`surface_samples`]: the drawn junction surfaces (the patches and
//!    the blended rows, which are not flat across) for the terrain settling
//!    ([`crate::clean::settle_terrain_under_surfaces`]).
//! 4. [`junction_data`]: the sections `JNCT` (the rules and outlines) and
//!    `RDGE` (the edge of every strip) for the final graph.
//! 5. [`seams`]: the lint's `junction.seam` – the drawn carriageways at
//!    every junction with a surface, sampled from above as the renderer's
//!    analysis test does.
//!
//! 6. [`relimit_grades`]: the class grade and vertical-curvature limits
//!    applied again after the blend with the junction surfaces fixed (a
//!    side road's missed height spread over a longer stretch), counted by
//!    cause in [`steep_intervals`] (J2b).
//!
//! All pure.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use ocw_world::graph::{PROFILE_STEP_M, RoadGraph, VertexId};
use ocw_world::junction::section::{
    FLAG_SETTLED, JunctionData, JunctionIndex, NO_EDGE, StripEdges, strip_edges_of,
};
use ocw_world::junction::{EndRule, JunctionSet, Patch, VertexRule, frame_at};
use ocw_world::package::WorldPackage;

use crate::clean::round_kinks_at;
use crate::profile::limits;

/// The through pairs are smoothed within this distance of the vertex (m).
pub const THROUGH_WINDOW_M: f64 = 40.0;
/// Reconciliation rounds (the patches are made again on the reconciled
/// profiles; the second round mostly moves nothing).
pub const ROUNDS: usize = 4;

fn cumulative(points: &[[f64; 2]]) -> Vec<f64> {
    let mut out = Vec::with_capacity(points.len());
    let mut s = 0.0;
    for (i, p) in points.iter().enumerate() {
        if i > 0 {
            let q = points[i - 1];
            s += (p[0] - q[0]).hypot(p[1] - q[1]);
        }
        out.push(s);
    }
    out
}

/// Distance of profile sample `i` from the edge's `from` vertex.
fn sample_m(i: usize, len: f64) -> f64 {
    (i as f64 * PROFILE_STEP_M).min(len)
}

/// The vertices that get a junction surface (a patch), for the easing.
pub fn patched_vertices(graph: &RoadGraph) -> Vec<bool> {
    let js = JunctionSet::new(graph);
    (0..graph.vertices.len())
        .map(|v| js.patch(graph, v).is_some())
        .collect()
}

/// The through pair at vertex `v`: the two edges of a way split (on the
/// ground, not at the region border, not a loop), or the through road's
/// two arms of a patch.
fn through_pair(
    g: &RoadGraph,
    js: &JunctionSet,
    adj: &[Vec<usize>],
    v: usize,
) -> Option<[usize; 2]> {
    let edges = &adj[v];
    let usable = |e: usize| {
        let e = &g.edges[e];
        !e.bridge && !e.tunnel && e.from != e.to && e.profile.len() >= 2
    };
    if g.boundary_vertices.contains(&VertexId(v as u32)) {
        return None;
    }
    let pair = if edges.len() == 2 {
        [edges[0], edges[1]]
    } else {
        let p = js.patch(g, v)?;
        let [a, b] = p.through?;
        // In a cluster only a through road whose two arms leave here.
        let here = g.vertices[v];
        if p.arms[a].origin != here || p.arms[b].origin != here {
            return None;
        }
        [p.arms[a].edge, p.arms[b].edge]
    };
    (pair[0] != pair[1] && usable(pair[0]) && usable(pair[1])).then_some(pair)
}

/// The vertical-curvature limit (4.8.3) across the junctions for the
/// through pairs: the joined profile of the two edges, within
/// [`THROUGH_WINDOW_M`] of the vertex, rounded like a kink inside a road
/// (never steeper; the vertex sample moves too, and every other road at
/// the vertex – a side road of the patch, re-blended by
/// [`reconcile_profiles`] – takes the new height). Returns the samples
/// moved.
pub fn smooth_through_kinks(graph: &mut RoadGraph) -> usize {
    let adj = graph.adjacency();
    let mut moved = 0;
    for _ in 0..3 {
        let js = JunctionSet::new(graph);
        let pairs: Vec<(usize, [usize; 2])> = (0..graph.vertices.len())
            .filter_map(|v| through_pair(graph, &js, &adj, v).map(|p| (v, p)))
            .collect();
        drop(js);
        let mut round = 0;
        for (v, [ea, eb]) in pairs {
            // Joined: edge a from its far end to the vertex, then edge b.
            let (a, b) = (&graph.edges[ea], &graph.edges[eb]);
            let (la, lb) = (a.length_m(), b.length_m());
            let a_from = a.from.0 as usize == v;
            let b_from = b.from.0 as usize == v;
            // (edge, sample index) per joined sample, with its distance.
            let mut idx: Vec<(usize, usize)> = Vec::new();
            let mut d: Vec<f64> = Vec::new();
            let mut h: Vec<f64> = Vec::new();
            let na = a.profile.len();
            for k in 0..na {
                let i = if a_from { na - 1 - k } else { k };
                let from_v = if a_from {
                    sample_m(i, la)
                } else {
                    la - sample_m(i, la)
                };
                idx.push((ea, i));
                d.push(-from_v);
                h.push(a.profile[i]);
            }
            let nb = b.profile.len();
            for k in 1..nb {
                let i = if b_from { k } else { nb - 1 - k };
                let from_v = if b_from {
                    sample_m(i, lb)
                } else {
                    lb - sample_m(i, lb)
                };
                idx.push((eb, i));
                d.push(from_v);
                h.push(b.profile[i]);
            }
            // Distances must grow (two samples at one place: nothing to do).
            if d.windows(2).any(|w| w[1] - w[0] < 1e-6) {
                continue;
            }
            let max_change = limits(a.class).1.max(limits(b.class).1);
            let before = h.clone();
            let window = |i: usize| d[i].abs() <= THROUGH_WINDOW_M;
            round_kinks_at(&mut h, &d, max_change, &window);
            let mut vertex_h = None;
            for (k, (&(e, i), (&new, &old))) in idx.iter().zip(h.iter().zip(&before)).enumerate() {
                if (new - old).abs() < 1e-9 {
                    continue;
                }
                round += 1;
                graph.edges[e].profile[i] = new;
                if k == na - 1 {
                    vertex_h = Some(new);
                }
            }
            // The vertex sample is shared: edge b's and the side roads'.
            if let Some(vh) = vertex_h {
                for &e in &adj[v] {
                    let edge = &mut graph.edges[e];
                    if edge.profile.is_empty() {
                        continue;
                    }
                    if edge.from.0 as usize == v {
                        edge.profile[0] = vh;
                    }
                    if edge.to.0 as usize == v {
                        let n = edge.profile.len();
                        edge.profile[n - 1] = vh;
                    }
                }
            }
        }
        moved += round;
        if round == 0 {
            break;
        }
    }
    moved
}

/// What [`reconcile_profiles`] changed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReconcileStats {
    /// Vertices with a junction surface.
    pub patches: usize,
    /// Profile samples set to the drawn surface (first round).
    pub samples: usize,
    /// Largest change of a sample (first round, m).
    pub max_change_m: f64,
    /// Largest change in the last round (m): how far from a fixed point.
    pub last_round_m: f64,
    pub rounds: usize,
    /// After the rounds: the largest difference between a profile sample
    /// and the drawn surface there, on the patches made from the final
    /// profiles (m).
    pub residual_m: f64,
    /// Samples whose drawn surface still differs by more than 1 cm, of
    /// those set.
    pub residual_over_1cm: usize,
    pub residual_samples: usize,
    /// Where the largest difference lies (edge, sample).
    pub residual_at: (usize, usize),
    /// Samples set on the longer blends of side roads whose blend the
    /// edge's length cut short (first round).
    pub longer_blend_samples: usize,
    /// Blended junctions laid on their plane and the samples set on it and
    /// on the curves from it (first round; spec §4.8.8 Q7 (c)).
    pub planes: usize,
    pub plane_samples: usize,
    /// Profile intervals steeper than their class limit + 1 % after the
    /// blend, before re-limiting (all causes).
    pub steep_before: usize,
    /// What the re-limiting changed ([`relimit_grades`], all cycles) and
    /// how many cycles ran.
    pub relimit: RelimitStats,
    pub relimit_cycles: usize,
    /// Afterwards, by cause ([`steep_intervals`]): near a junction, where
    /// the source data is steeper, where the junction surfaces leave too
    /// little room, elsewhere.
    pub steep_intervals: usize,
    pub steep_source: usize,
    pub steep_room: usize,
    pub steep_other: usize,
}

/// The profile samples a junction surface is made from, which the
/// reconciliation keeps (else the patches made again from the new
/// profiles would differ – no fixed point): the two samples round each
/// main arm's mouth (its height and grade there shape the patch: a through
/// road's arms, every arm of a blended patch) and round the end of each
/// side arm's blend (where the blend meets the road's own profile). A side
/// arm's mouth row lies on the main road; its own height there only sets
/// the blend's length.
fn pinned_samples(g: &RoadGraph, js: &JunctionSet) -> std::collections::HashSet<(usize, usize)> {
    let mut out = std::collections::HashSet::new();
    for v in 0..g.vertices.len() {
        let Some(p) = js.own_patch(g, v) else {
            continue;
        };
        for (ai, a) in p.arms.iter().enumerate() {
            // A side arm with its corner (Q6 (d)) is drawn from its own
            // height and grade at the mouth, as a main arm.
            let side = p.through.is_some_and(|t| !t.contains(&ai)) && !a.has_corner();
            let anchor = match (side, a.blend_m > 0.0) {
                (false, _) => a.mouth_m,
                (true, true) => a.mouth_m + a.blend_m,
                (true, false) => continue,
            };
            let e = &g.edges[a.edge];
            let n = e.profile.len();
            if n < 2 {
                continue;
            }
            let s = a.s_edge(anchor).clamp(0.0, a.len);
            let i0 = ((s / PROFILE_STEP_M).floor() as usize).min(n - 2);
            out.insert((a.edge, i0));
            out.insert((a.edge, i0 + 1));
            // On a sample: its neighbours on both sides too.
            for k in [i0, i0 + 1] {
                if (sample_m(k, a.len) - s).abs() < 1e-6 {
                    if k > 0 {
                        out.insert((a.edge, k - 1));
                    }
                    if k + 1 < n {
                        out.insert((a.edge, k + 1));
                    }
                }
            }
        }
    }
    out
}

/// The drawn surface at every profile sample within reach of a junction
/// surface that may change (not [`pinned_samples`], not where `keep` says
/// no): (edge, sample, height). The vertex samples of a patch's arms only
/// all together (one height at the vertex).
fn drawn_samples(
    g: &RoadGraph,
    js: &JunctionSet,
    keep: &(dyn Fn(usize, f64) -> bool + Sync),
) -> Vec<(usize, usize, f64)> {
    let pinned = pinned_samples(g, js);
    let verts: Vec<usize> = (0..g.vertices.len()).collect();
    let per = crate::par::map(&verts, crate::par::threads(), |&v| {
        let mut out = Vec::new();
        let Some(p) = js.own_patch(g, v) else {
            return out;
        };
        let mut at_vertex = Vec::new();
        let mut vertex_ok = true;
        for a in &p.arms {
            let e = &g.edges[a.edge];
            let len = e.length_m();
            let n = e.profile.len();
            let reach = a.mouth_m + a.blend_m + 1e-6;
            let cum = cumulative(&e.points);
            for i in 0..n {
                let s = sample_m(i, len);
                let m = if a.at_from { s } else { len - s };
                let vertex = if a.at_from { i == 0 } else { i == n - 1 };
                if m > reach {
                    continue;
                }
                if !keep(a.edge, s) || pinned.contains(&(a.edge, i)) {
                    if vertex {
                        vertex_ok = false;
                    }
                    continue;
                }
                let c = frame_at(&e.points, &cum, s).0;
                match drawn_on(g, js, p, a.edge, a.at_from, s, c) {
                    Some(h) if vertex => at_vertex.push((a.edge, i, h)),
                    Some(h) => out.push((a.edge, i, h)),
                    None if vertex => vertex_ok = false,
                    None => {}
                }
            }
        }
        // A cluster's edges inside (J3): all their samples, the vertex
        // samples with the arms'.
        for &ei in &p.inner {
            let e = &g.edges[ei];
            let len = e.length_m();
            let n = e.profile.len();
            let cum = cumulative(&e.points);
            for i in 0..n {
                let s = sample_m(i, len);
                let vertex = i == 0 || i == n - 1;
                if !keep(ei, s) || pinned.contains(&(ei, i)) {
                    if vertex {
                        vertex_ok = false;
                    }
                    continue;
                }
                let c = frame_at(&e.points, &cum, s).0;
                match p.height_at(c) {
                    Some(h) if vertex => at_vertex.push((ei, i, h)),
                    Some(h) => out.push((ei, i, h)),
                    None if vertex => vertex_ok = false,
                    None => {}
                }
            }
        }
        if vertex_ok {
            out.extend(at_vertex);
        }
        out
    });
    per.into_iter().flatten().collect()
}

/// The blends a side road needs but its edge's length cut short (J1 blends
/// over at most half the edge): `L = clamp(|Δh| / 0.01, 5 m, 30 m)` from
/// the main road's surface at the mouth (its height and slope) into the
/// road's own profile (its height and grade there), baked into the
/// profile up to one sample before the other end's junction (spec §4.8.8
/// J2: the side arms' profiles follow the patch). Per side road the
/// samples beyond its mouth on that curve: (edge, sample, height). The
/// junction surface made again then blends over its short stretch between
/// heights that already agree.
fn longer_blends(
    g: &RoadGraph,
    js: &JunctionSet,
    keep: &(dyn Fn(usize, f64) -> bool + Sync),
) -> Vec<Vec<(usize, usize, f64)>> {
    use ocw_world::junction::{BLEND_MAX_M, BLEND_MIN_M, BLEND_RATE};
    let verts: Vec<usize> = (0..g.vertices.len()).collect();
    crate::par::map(&verts, crate::par::threads(), |&v| {
        let mut out = Vec::new();
        let Some(p) = js.own_patch(g, v) else {
            return out;
        };
        let Some(through) = p.through else {
            return out;
        };
        for (ai, a) in p.arms.iter().enumerate() {
            if through.contains(&ai) {
                continue;
            }
            let e = &g.edges[a.edge];
            let n = e.profile.len();
            if n < 3 {
                continue;
            }
            if a.has_corner() {
                out.extend(edge_blend(g, js, a, keep));
                continue;
            }
            let mid = [
                0.5 * (a.row[0][0] + a.row[1][0]),
                0.5 * (a.row[0][1] + a.row[1][1]),
                0.5 * (a.row[0][2] + a.row[1][2]),
            ];
            let dh = (mid[1] - a.h).abs();
            let want = (dh / BLEND_RATE).clamp(BLEND_MIN_M, BLEND_MAX_M);
            if a.blend_m + 0.5 >= want {
                continue;
            }
            // Up to one sample before the other end's junction.
            let k = usize::from(a.at_from);
            let other = js.reach(g, a.edge)[k];
            let room = a.len - a.mouth_m - other - PROFILE_STEP_M;
            let l = want.min(room);
            if l <= a.blend_m + 1.0 {
                continue;
            }
            let d = 0.05;
            let q = [mid[0], mid[2]];
            let f = |x: [f64; 2]| p.main_surface(x);
            let g0 = (f([q[0] + a.t[0] * d, q[1] + a.t[1] * d])
                - f([q[0] - a.t[0] * d, q[1] - a.t[1] * d]))
                / (2.0 * d);
            let end = a.s_edge(a.mouth_m + l);
            let p1 = e.height_at(end).unwrap_or(a.h);
            // The own grade (outward) at the end.
            let dir = if a.at_from { 1.0 } else { -1.0 };
            let e2 = (end + dir * 0.05).clamp(0.0, a.len);
            let m1 = if (e2 - end).abs() > 1e-9 {
                (e.height_at(e2).unwrap_or(p1) - p1) / (e2 - end).abs()
            } else {
                a.grade
            };
            for i in 0..n {
                let s = sample_m(i, a.len);
                let m = if a.at_from { s } else { a.len - s };
                if m <= a.mouth_m + 1e-6 || m >= a.mouth_m + l - 1e-6 || !keep(a.edge, s) {
                    continue;
                }
                let t = (m - a.mouth_m) / l;
                out.push((a.edge, i, hermite(t, mid[1], g0 * l, p1, m1 * l)));
            }
        }
        out
    })
}

/// A side road with its corner (spec §4.8.8 Q6 (d), J3): its profile from
/// the main road's edge – where its centre line leaves the main road's
/// carriageway, at the main road's surface there – starting at its own
/// grade and blended into its own profile over `L = clamp(|Δh| / 0.01,
/// 5 m, 30 m)`, up to one sample before the other end's junction (its
/// edge point, for a side road with a corner there too). The samples on
/// that curve: (edge, sample, height). The corner the junction surface
/// draws then follows this profile.
fn edge_blend(
    g: &RoadGraph,
    js: &JunctionSet,
    a: &ocw_world::junction::Arm,
    keep: &(dyn Fn(usize, f64) -> bool + Sync),
) -> Vec<(usize, usize, f64)> {
    use ocw_world::junction::{BLEND_MAX_M, BLEND_MIN_M, BLEND_RATE};
    let e = &g.edges[a.edge];
    let n = e.profile.len();
    let own = |m: f64| e.height_at(a.s_edge(m.clamp(0.0, a.len))).unwrap_or(a.h);
    let slope = |m: f64| (own(m + 0.05) - own(m)) / 0.05;
    let m0 = a.edge_m;
    let dh = own(m0) - a.edge_h;
    // The other end's fixed stretch.
    let k = usize::from(a.at_from);
    let other = match js.ends(g, a.edge)[k] {
        Some(EndRule::Patch(q, i)) if q.arms[i].has_corner() => q.arms[i].edge_m,
        _ => js.reach(g, a.edge)[k],
    };
    let room = a.len - m0 - other - PROFILE_STEP_M;
    let l = (dh.abs() / BLEND_RATE)
        .clamp(BLEND_MIN_M, BLEND_MAX_M)
        .min(room);
    if l < 1.0 {
        return Vec::new();
    }
    // At its own grade from the edge; the crease there is rounded with the
    // grade limits ([`round_creases`], Q7 (b)).
    let (g0, p1, g1) = (slope(m0), own(m0 + l), slope(m0 + l));
    let mut out = Vec::new();
    for i in 0..n {
        let s = sample_m(i, a.len);
        let m = if a.at_from { s } else { a.len - s };
        if m <= m0 + 1e-6 || m >= m0 + l - 1e-6 || !keep(a.edge, s) {
            continue;
        }
        let t = (m - m0) / l;
        out.push((a.edge, i, hermite(t, a.edge_h, g0 * l, p1, g1 * l)));
    }
    out
}

/// A blended junction's surface is one plane (spec §4.8.8 Q7 (c)) and each
/// road turns from it to its own grade over this many metres past the
/// junction surface at least and at most.
pub const PLANE_BLEND_MIN_M: f64 = 10.0;
pub const PLANE_BLEND_MAX_M: f64 = 20.0;

/// The plane through a blended junction (no through road, Q1): the least
/// squares fit to each arm's own profile at its mouth and 5 m on (height
/// and the arm's line), its gradient limited to the steepest class grade
/// of its roads. Returns (height at the patch's centre, gradient x, z).
fn junction_plane(g: &RoadGraph, p: &Patch) -> Option<(f64, f64, f64)> {
    let c = [p.centre[0], p.centre[2]];
    let mut pts: Vec<([f64; 2], f64)> = Vec::new();
    let mut lim = 0.0f64;
    for a in &p.arms {
        let e = &g.edges[a.edge];
        let cum = cumulative(&e.points);
        for m in [a.mouth_m, (a.mouth_m + PROFILE_STEP_M).min(a.len)] {
            let s = a.s_edge(m);
            let q = frame_at(&e.points, &cum, s).0;
            pts.push((q, e.height_at(s)?));
        }
        lim = lim.max(limits(a.class).0);
    }
    if pts.len() < 3 {
        return None;
    }
    // Normal equations of h = h0 + gx·dx + gz·dz, with a small ridge on
    // the gradient (two arms in a line leave the cross slope free: 0).
    let mut m = [[0.0f64; 3]; 3];
    let mut b = [0.0f64; 3];
    for (q, h) in &pts {
        let r = [1.0, q[0] - c[0], q[1] - c[1]];
        for i in 0..3 {
            for j in 0..3 {
                m[i][j] += r[i] * r[j];
            }
            b[i] += r[i] * h;
        }
    }
    let ridge = 1e-3 * (m[1][1] + m[2][2]).max(1.0);
    m[1][1] += ridge;
    m[2][2] += ridge;
    let det = |m: &[[f64; 3]; 3]| {
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    };
    let d = det(&m);
    if d.abs() < 1e-12 {
        return None;
    }
    let solve = |k: usize| {
        let mut mk = m;
        for i in 0..3 {
            mk[i][k] = b[i];
        }
        det(&mk) / d
    };
    let (h0, mut gx, mut gz) = (solve(0), solve(1), solve(2));
    let gl = gx.hypot(gz);
    if gl > lim && gl > 0.0 {
        gx *= lim / gl;
        gz *= lim / gl;
    }
    // The height at the centre with the limited gradient (the mean of the
    // fit points' residuals).
    let mean = pts
        .iter()
        .map(|(q, h)| h - gx * (q[0] - c[0]) - gz * (q[1] - c[1]))
        .sum::<f64>()
        / pts.len() as f64;
    let _ = h0;
    Some((mean, gx, gz))
}

/// The blended junctions' smoother surface (spec §4.8.8 Q7 (c)): at each
/// junction without a through road (equal crossings, Y junctions, forks)
/// every road's profile from the vertex to the first sample past its
/// mouth lies on the junction's plane ([`junction_plane`]) – the patch
/// made from them is that plane – and from there it turns to its own
/// height and grade along a cubic Hermite curve of 10 – 20 m (by the
/// height and the grade it has to make up, up to one sample before the
/// other end's junction). Clusters keep the surface of J3 (their edges
/// inside are nodes of it). Per junction the samples set: (edge, sample,
/// height).
fn blended_planes(
    g: &RoadGraph,
    js: &JunctionSet,
    keep: &(dyn Fn(usize, f64) -> bool + Sync),
) -> Vec<Vec<(usize, usize, f64)>> {
    use ocw_world::junction::BLEND_RATE;
    let verts: Vec<usize> = (0..g.vertices.len()).collect();
    crate::par::map(&verts, crate::par::threads(), |&v| {
        let mut out = Vec::new();
        let Some(p) = js.own_patch(g, v) else {
            return out;
        };
        // Clusters keep their surface (their edges inside are nodes of it).
        if p.through.is_some() || p.arms.len() < 2 || p.members.len() > 1 {
            return out;
        }
        let Some((h0, gx, gz)) = junction_plane(g, p) else {
            return out;
        };
        let c = [p.centre[0], p.centre[2]];
        let plane = |q: [f64; 2]| h0 + gx * (q[0] - c[0]) + gz * (q[1] - c[1]);
        // Every road on the plane and its curve, or none (a junction whose
        // plane one road cannot reach within its class grade – or the
        // grade it has – keeps the surface of J3: no `junction_room`
        // gained, spec §4.8.8 Q7 follow-up).
        for a in &p.arms {
            let e = &g.edges[a.edge];
            let n = e.profile.len();
            if n < 2 {
                continue;
            }
            let len = a.len;
            let cum = cumulative(&e.points);
            // Samples by distance from this vertex.
            let m_of = |i: usize| {
                let s = sample_m(i, len);
                if a.at_from { s } else { len - s }
            };
            let idx: Vec<usize> = if a.at_from {
                (0..n).collect()
            } else {
                (0..n).rev().collect()
            };
            // The first sample past the mouth: still on the plane.
            let Some(k1) = idx.iter().position(|&i| m_of(i) > a.mouth_m + 1e-6) else {
                return Vec::new();
            };
            let k = usize::from(a.at_from);
            let other = js.reach(g, a.edge)[k];
            let room_end = len - other - PROFILE_STEP_M;
            let on_plane = |i: usize| plane(frame_at(&e.points, &cum, sample_m(i, len)).0);
            if idx[..=k1]
                .iter()
                .any(|&i| m_of(i) > room_end + 1e-6 || !keep(a.edge, sample_m(i, len)))
            {
                return Vec::new();
            }
            // From there the curve to the road's own profile.
            let i1 = idx[k1];
            let m1 = m_of(i1);
            let h1 = on_plane(i1);
            let own = |m: f64| e.height_at(a.s_edge(m.clamp(0.0, len))).unwrap_or(h1);
            let own_slope = |m: f64| (own(m + 0.05) - own(m)) / 0.05;
            let slope1 = {
                let d = 0.05;
                let q0 = frame_at(&e.points, &cum, a.s_edge(m1)).0;
                let q1 = frame_at(&e.points, &cum, a.s_edge((m1 + d).min(len))).0;
                (plane(q1) - plane(q0)) / d
            };
            let want = ((own(m1) - h1).abs() / BLEND_RATE)
                .max((own_slope(m1) - slope1).abs() / crate::junctions::CREASE_RATE)
                .clamp(PLANE_BLEND_MIN_M, PLANE_BLEND_MAX_M);
            // The end: a sample at least `want` on, before the room ends.
            let Some(k2) = idx[k1 + 1..]
                .iter()
                .position(|&i| m_of(i) >= m1 + want - 1e-6)
                .map(|x| x + k1 + 1)
            else {
                return Vec::new();
            };
            let i2 = idx[k2];
            let m2 = m_of(i2);
            if m2 > room_end + 1e-6 {
                return Vec::new();
            }
            let (p2, g2) = (e.profile[i2], own_slope(m2));
            let l = m2 - m1;
            let mut arm: Vec<(usize, f64)> = idx[..=k1].iter().map(|&i| (i, on_plane(i))).collect();
            for &i in &idx[k1 + 1..k2] {
                if !keep(a.edge, sample_m(i, len)) {
                    return Vec::new();
                }
                let t = (m_of(i) - m1) / l;
                arm.push((i, hermite(t, h1, slope1 * l, p2, g2 * l)));
            }
            arm.push((i2, p2));
            // Not steeper than the class grade or what the road has there.
            let new_steep = arm
                .windows(2)
                .filter(|w| (m_of(w[1].0) - m_of(w[0].0)).abs() >= 1.0)
                .map(|w| ((w[1].1 - w[0].1) / (m_of(w[1].0) - m_of(w[0].0))).abs())
                .fold(0.0f64, f64::max);
            let old_steep = arm
                .windows(2)
                .filter(|w| (m_of(w[1].0) - m_of(w[0].0)).abs() >= 1.0)
                .map(|w| {
                    ((e.profile[w[1].0] - e.profile[w[0].0]) / (m_of(w[1].0) - m_of(w[0].0))).abs()
                })
                .fold(0.0f64, f64::max);
            if new_steep > limits(a.class).0.max(old_steep) + 1e-6 {
                return Vec::new();
            }
            arm.pop();
            out.extend(arm.into_iter().map(|(i, h)| (a.edge, i, h)));
        }
        out
    })
}

/// Cubic Hermite on [0, 1].
fn hermite(t: f64, p0: f64, m0: f64, p1: f64, m1: f64) -> f64 {
    let (t2, t3) = (t * t, t * t * t);
    (2.0 * t3 - 3.0 * t2 + 1.0) * p0
        + (t3 - 2.0 * t2 + t) * m0
        + (-2.0 * t3 + 3.0 * t2) * p1
        + (t3 - t2) * m1
}

/// A side road's crease is rounded into a vertical curve this long at
/// least and at most (m; spec §4.8.8 Q7 (b)), changing its grade by at
/// most [`CREASE_RATE`] per metre where it can.
pub const CREASE_MIN_M: f64 = 5.0;
pub const CREASE_MAX_M: f64 = 10.0;
pub const CREASE_RATE: f64 = 0.015;

/// The vertical curve's length for a change of grade `dg` (Q7 (b)).
pub fn crease_curve_m(dg: f64) -> f64 {
    (dg.abs() / CREASE_RATE).clamp(CREASE_MIN_M, CREASE_MAX_M)
}

/// The main road's slope (its own surface, outward along the side road)
/// where side arm `a` leaves its carriageway (Q6 (d)'s edge point);
/// `None` for an arm without a corner or one leaving from a cluster vertex
/// off the main road's carriageway (no crease there).
fn main_slope_out(g: &RoadGraph, p: &Patch, a: &ocw_world::junction::Arm) -> Option<f64> {
    if !a.has_corner() || a.edge_m < 0.01 {
        return None;
    }
    let e = &g.edges[a.edge];
    let cum = cumulative(&e.points);
    let (_, fwd) = frame_at(&e.points, &cum, a.s_edge(a.edge_m));
    let out = if a.at_from { fwd } else { [-fwd[0], -fwd[1]] };
    let d = 0.05;
    let q = a.edge_p;
    let f = |k: f64| p.main_own([q[0] + out[0] * d * k, q[1] + out[1] * d * k]);
    Some((f(1.0)? - f(-1.0)?) / (2.0 * d))
}

/// The drawn height on edge `ei` at `s` (point `c`) from the patch `p`'s
/// end of it (not the other end's, on a short edge).
fn drawn_on(
    g: &RoadGraph,
    js: &JunctionSet,
    p: &Patch,
    ei: usize,
    at_from: bool,
    s: f64,
    c: [f64; 2],
) -> Option<f64> {
    let ends = js.ends(g, ei);
    let k = usize::from(!at_from);
    match ends[k] {
        Some(EndRule::Patch(q, _)) | Some(EndRule::Inside(q)) if q.vertex == p.vertex => {
            js.drawn_height_at_end(g, ei, k, s, c)
        }
        _ => None,
    }
}

/// Sets the road profiles near every junction surface to the drawn
/// surface (spec §4.8.8 J2) – inside the patch its height along the
/// centre line, along a side road's blend the blended rows – then makes
/// the patches again on the new profiles and repeats ([`ROUNDS`]).
/// Afterwards the class grade and vertical-curvature limits of 4.8.3 are
/// applied again with the junction surfaces fixed ([`relimit_grades`]: a
/// side road's missed height spread over a longer stretch) and the
/// profiles reconciled once more, up to [`RELIMIT_CYCLES`] times.
/// `keep(edge, s)`: whether the sample `s` metres along `edge` may change
/// (not on a planned bridge deck). Mitres are not touched (their corners
/// follow the grades). Bridges and tunnels have no patch.
pub fn reconcile_profiles(
    graph: &mut RoadGraph,
    keep: &(dyn Fn(usize, f64) -> bool + Sync),
) -> ReconcileStats {
    let mut st = ReconcileStats::default();
    reconcile_rounds(graph, keep, true, &mut st);
    // The intervals over the limit after the blend (before re-limiting).
    let js = JunctionSet::new(graph);
    st.steep_before = steep_intervals(graph, &js, keep).len();
    drop(js);
    for cycle in 0..RELIMIT_CYCLES {
        let rl = relimit_grades(graph, keep);
        st.relimit.edges += rl.edges;
        st.relimit.samples += rl.samples;
        st.relimit.max_change_m = st.relimit.max_change_m.max(rl.max_change_m);
        st.relimit_cycles = cycle + 1;
        if rl.samples == 0 {
            break;
        }
        let mut again = ReconcileStats::default();
        reconcile_rounds(graph, keep, false, &mut again);
        st.rounds += again.rounds;
        st.last_round_m = again.last_round_m;
    }
    // How far the final profiles are from the final patches' surface.
    let js = JunctionSet::new(graph);
    for (e, i, h) in drawn_samples(graph, &js, keep) {
        let d = (h - graph.edges[e].profile[i]).abs();
        if d > st.residual_m {
            st.residual_m = d;
            st.residual_at = (e, i);
        }
        st.residual_samples += 1;
        if d > 0.01 {
            st.residual_over_1cm += 1;
        }
    }
    for s in steep_intervals(graph, &js, keep) {
        match s.cause {
            SteepCause::SourceSteep => st.steep_source += 1,
            SteepCause::JunctionRoom => st.steep_room += 1,
            SteepCause::Junction => st.steep_intervals += 1,
            SteepCause::Other => st.steep_other += 1,
        }
    }
    st
}

/// The reconciliation rounds of [`reconcile_profiles`]; `longer`: the
/// first round also bakes the longer blends ([`longer_blends`]) and counts
/// the patches.
fn reconcile_rounds(
    graph: &mut RoadGraph,
    keep: &(dyn Fn(usize, f64) -> bool + Sync),
    longer: bool,
    st: &mut ReconcileStats,
) {
    for round in 0..ROUNDS {
        let js = JunctionSet::new(graph);
        let first = round == 0 && longer;
        if first {
            st.patches = (0..graph.vertices.len())
                .filter(|&v| js.own_patch(graph, v).is_some())
                .count();
        }
        let drawn = drawn_samples(graph, &js, keep);
        // First round: the side roads whose blend the edge's length cut
        // short get the blend they need in their profile.
        let mut blends = if first {
            longer_blends(graph, &js, keep)
        } else {
            Vec::new()
        };
        if first {
            st.longer_blend_samples = blends.iter().map(Vec::len).sum();
            // Q7 (c): the blended junctions' plane and the curves from it.
            let planes = blended_planes(graph, &js, keep);
            st.planes = planes.iter().filter(|p| !p.is_empty()).count();
            st.plane_samples = planes.iter().map(Vec::len).sum();
            blends.extend(planes);
        }
        drop(js);
        // A sample both ends reach (the middle of a short edge): the mean.
        let mut by: BTreeMap<(usize, usize), (f64, u32)> = BTreeMap::new();
        for (e, i, h) in drawn {
            let x = by.entry((e, i)).or_insert((0.0, 0));
            x.0 += h;
            x.1 += 1;
        }
        for (e, i, h) in blends.into_iter().flatten() {
            by.insert((e, i), (h, 1));
        }
        let mut worst = 0.0f64;
        let mut changed = 0;
        for ((e, i), (sum, n)) in by {
            let h = sum / f64::from(n);
            let old = &mut graph.edges[e].profile[i];
            let d = (h - *old).abs();
            if d > 1e-6 {
                changed += 1;
                worst = worst.max(d);
                *old = h;
            }
        }
        st.rounds = round + 1;
        if round == 0 {
            st.samples = changed;
            st.max_change_m = worst;
        }
        st.last_round_m = worst;
        if worst < 1e-3 {
            break;
        }
    }
}

// --- grade limits after the blend (J2b) ------------------------------------

/// Re-limiting and reconciling alternate at most this often.
pub const RELIMIT_CYCLES: usize = 3;
/// Grades above the class limit by more than this are findings (4.8.1).
pub const GRADE_TOLERANCE: f64 = 0.01;
/// An interval within this distance beyond a junction surface's reach
/// counts as the junction's (`grade_over_limit.junction`).
pub const JUNCTION_NEAR_M: f64 = 30.0;

/// What the re-limiting keeps on an edge ([`fixed_samples`]).
#[derive(Debug, Clone, PartialEq)]
pub struct Fixed {
    /// Per profile sample: kept.
    pub samples: Vec<bool>,
    /// The mouths of side arms (metres from the edge's `from` vertex and
    /// the drawn height there, the main road's surface): kept too, as
    /// points between the samples.
    pub mouths: Vec<(f64, f64)>,
    /// The samples inside a side arm's patch (not the vertex) with the
    /// drawn height there (the patch's mesh): kept at that height. The
    /// reconciliation may have left them off it (the samples it pins
    /// round the end of a short blend can lie inside the patch).
    pub drawn: Vec<(usize, f64)>,
    /// The mouths (of `mouths`) where a side road leaves the main road's
    /// edge at its own grade (Q6 (d)): a crease, no grade carried on.
    pub creases: Vec<f64>,
    /// Per crease (metres from the `from` vertex) the main road's slope
    /// there along the edge (rising towards its `to` end) and the side the
    /// road goes on to (+1: towards `to`): rounded into a vertical curve
    /// (Q7 (b)). Creases at a cluster vertex off the main road have none.
    pub crease_grades: Vec<(f64, f64, f64)>,
}

/// What the re-limiting keeps on edge `ei`: both vertex samples (the
/// junction heights, shared by every road there); at a junction surface
/// the samples up to its mouth (the patch's mesh: the junction stays level
/// with the main road), for a side arm its mouth on the main road's
/// surface, for a main arm the sample past its mouth (its height and
/// grade there shape the patch); the samples `keep` forbids (bridge
/// decks). Bridges and tunnels: everything.
pub fn fixed_samples(
    g: &RoadGraph,
    js: &JunctionSet,
    ei: usize,
    keep: &(dyn Fn(usize, f64) -> bool + Sync),
) -> Fixed {
    let e = &g.edges[ei];
    let n = e.profile.len();
    if e.bridge || e.tunnel || n < 3 {
        return Fixed {
            samples: vec![true; n],
            mouths: Vec::new(),
            drawn: Vec::new(),
            creases: Vec::new(),
            crease_grades: Vec::new(),
        };
    }
    let len = e.length_m();
    let mut reach = [0.0f64; 2];
    let mut mouths = Vec::new();
    let mut drawn = Vec::new();
    let mut creases = Vec::new();
    let mut crease_grades = Vec::new();
    let mut cum = None;
    if js
        .ends(g, ei)
        .iter()
        .any(|e| matches!(e, Some(EndRule::Inside(_))))
    {
        // Inside a cluster's patch (J3): its surface all along.
        return Fixed {
            samples: vec![true; n],
            mouths: Vec::new(),
            drawn: Vec::new(),
            creases: Vec::new(),
            crease_grades: Vec::new(),
        };
    }
    for (k, end) in js.ends(g, ei).iter().enumerate() {
        if let Some(EndRule::Patch(p, ai)) = end {
            let a = &p.arms[*ai];
            let main = p.through.is_none_or(|t| t.contains(ai));
            // A side road with its corner (Q6 (d)): fixed only on the main
            // road's carriageway, up to where it leaves it (a crease: it
            // goes on at its own grade); its corner and mouth follow it.
            let corner = !main && a.has_corner();
            let fixed_to = if corner { a.edge_m } else { a.mouth_m };
            reach[k] = fixed_to + if main { PROFILE_STEP_M } else { 0.0 };
            if !main {
                let s = a.s_edge(fixed_to);
                let h = if corner {
                    a.edge_h
                } else {
                    0.5 * (a.row[0][1] + a.row[1][1])
                };
                if s > 1e-3 && s < len - 1e-3 && keep(ei, s) {
                    mouths.push((s, h));
                    if corner {
                        creases.push(s);
                        if let Some(gm) = main_slope_out(g, p, a) {
                            // Outward is towards `to` from the `from` end.
                            let dir = if k == 0 { 1.0 } else { -1.0 };
                            crease_grades.push((s, gm * dir, dir));
                        }
                    }
                }
                let cum = cum.get_or_insert_with(|| cumulative(&e.points));
                for i in 1..n - 1 {
                    let si = sample_m(i, len);
                    let m = if k == 0 { si } else { len - si };
                    if m <= fixed_to - 1e-6 && keep(ei, si) {
                        let c = frame_at(&e.points, cum, si).0;
                        if let Some(h) = drawn_on(g, js, p, ei, k == 0, si, c) {
                            drawn.push((i, h));
                        }
                    }
                }
            }
        }
    }
    let samples = (0..n)
        .map(|i| {
            let s = sample_m(i, len);
            i == 0
                || i == n - 1
                || s <= reach[0] + 1e-6
                || len - s <= reach[1] + 1e-6
                || !keep(ei, s)
        })
        .collect();
    Fixed {
        samples,
        mouths,
        drawn,
        creases,
        crease_grades,
    }
}

/// A profile with the mouths of [`Fixed`] inserted as fixed points and
/// its samples inside side arms' patches at the drawn height.
struct Marked {
    heights: Vec<f64>,
    dist: Vec<f64>,
    fixed: Vec<bool>,
    /// The original sample of each point.
    orig: Vec<Option<usize>>,
    /// A crease at the point ([`Fixed::creases`]).
    crease: Vec<bool>,
    /// Its main road's slope (along the edge) and the side it goes on to.
    crease_grade: Vec<Option<(f64, f64)>>,
}

fn with_mouths(p: &[f64], d: &[f64], f: &Fixed) -> Marked {
    let mut pts: Vec<(f64, f64, bool, Option<usize>, bool)> = (0..p.len())
        .map(|i| (d[i], p[i], f.samples[i], Some(i), false))
        .collect();
    for &(i, h) in &f.drawn {
        pts[i].1 = h;
    }
    for &(s, h) in &f.mouths {
        let crease = f.creases.iter().any(|&c| (c - s).abs() < 1e-9);
        // On a sample: that sample is the mouth (kept, at its height).
        if let Some(q) = pts.iter_mut().find(|q| (q.0 - s).abs() < 0.05) {
            q.2 = true;
            if crease {
                q.1 = h;
                q.4 = true;
            }
            continue;
        }
        pts.push((s, h, true, None, crease));
    }
    pts.sort_by(|a, b| a.0.total_cmp(&b.0));
    let crease_grade = pts
        .iter()
        .map(|q| {
            if !q.4 {
                return None;
            }
            f.crease_grades
                .iter()
                .find(|c| (c.0 - q.0).abs() < 0.05)
                .map(|c| (c.1, c.2))
        })
        .collect();
    Marked {
        heights: pts.iter().map(|q| q.1).collect(),
        dist: pts.iter().map(|q| q.0).collect(),
        fixed: pts.iter().map(|q| q.2).collect(),
        orig: pts.iter().map(|q| q.3).collect(),
        crease: pts.iter().map(|q| q.4).collect(),
        crease_grade,
    }
}

/// What [`relimit_grades`] changed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RelimitStats {
    /// Edges re-limited and samples moved.
    pub edges: usize,
    pub samples: usize,
    /// Largest change of a sample (m).
    pub max_change_m: f64,
}

/// The class limits of 4.8.3 on one profile (samples at distances `d`)
/// with the samples `fixed` kept (spec §4.8.8 J2b).
///
/// 1. The grade: between two fixed samples every free sample is clamped
///    to its neighbour's cone of the class grade, forward and back until
///    it holds; where the fixed heights themselves lie farther apart than
///    the limit allows (the source data is steeper), the stretch between
///    them climbs at the least grade possible.
/// 2. Leaving a fixed stretch (a junction's surface): the profile stays
///    between the highest and the lowest a road can climb or fall from
///    there, starting at the fixed stretch's grade and changing it by at
///    most the curvature limit up to the class grade – a stretch too steep
///    beside a junction is pulled towards the junction's height and bends
///    into the climb until it meets the road's own profile (the blend
///    lengthened as far as it needs). The same from the far side.
/// 3. The kinks left at free samples are rounded towards their
///    neighbours' chord within the grade limit (a kink at a fixed sample
///    by its free neighbour); then the grade clamp once more (where both
///    cannot hold, the grade wins).
///
/// Returns the samples moved.
pub fn relimit_profile(
    p: &mut [f64],
    d: &[f64],
    fixed: &[bool],
    max_grade: f64,
    max_change_per_m: f64,
) -> usize {
    relimit_profile_creased(
        p,
        d,
        fixed,
        &vec![false; p.len()],
        max_grade,
        max_change_per_m,
    )
}

/// [`relimit_profile`] with creases: fixed points where the road may
/// change its grade at once (a side road leaving the main road's edge at
/// its own grade, spec §4.8.8 Q6 (d)) – no curvature limit carried on
/// from them, no kink rounded there.
pub fn relimit_profile_creased(
    p: &mut [f64],
    d: &[f64],
    fixed: &[bool],
    crease: &[bool],
    max_grade: f64,
    max_change_per_m: f64,
) -> usize {
    let n = p.len();
    if n < 3 {
        return 0;
    }
    let before = p.to_vec();
    let lim = clamp_grades(p, d, fixed, max_grade);
    clamp_to_envelopes(p, d, fixed, crease, &lim, max_change_per_m);
    round_kinks_fixed(p, d, fixed, crease, &lim, max_change_per_m);
    clamp_grades(p, d, fixed, max_grade);
    p.iter()
        .zip(&before)
        .filter(|(a, b)| (**a - **b).abs() > 1e-6)
        .count()
}

/// Step 1 of [`relimit_profile`]; returns the grade each interval may
/// have (the class limit, or the least possible between fixed heights
/// farther apart).
fn clamp_grades(p: &mut [f64], d: &[f64], fixed: &[bool], max_grade: f64) -> Vec<f64> {
    let n = p.len();
    let mut out = vec![max_grade; n - 1];
    let anchors: Vec<usize> = (0..n).filter(|&i| fixed[i]).collect();
    for w in anchors.windows(2) {
        let (a, b) = (w[0], w[1]);
        let run = (d[b] - d[a]).max(1e-9);
        let mean = (p[b] - p[a]).abs() / run;
        let lim = max_grade.max(mean * (1.0 + 1e-9) + 1e-9);
        out[a..b].fill(lim);
        if b <= a + 1 {
            continue;
        }
        for _ in 0..64 {
            let mut moved = false;
            for i in a + 1..b {
                let ds = d[i] - d[i - 1];
                let c = p[i].clamp(p[i - 1] - lim * ds, p[i - 1] + lim * ds);
                if (c - p[i]).abs() > 1e-9 {
                    p[i] = c;
                    moved = true;
                }
            }
            for i in (a + 1..b).rev() {
                let ds = d[i + 1] - d[i];
                let c = p[i].clamp(p[i + 1] - lim * ds, p[i + 1] + lim * ds);
                if (c - p[i]).abs() > 1e-9 {
                    p[i] = c;
                    moved = true;
                }
            }
            if !moved {
                break;
            }
        }
    }
    out
}

/// Step 2 of [`relimit_profile`]: from each fixed stretch of at least two
/// samples (its grade known), the highest and lowest reachable heights
/// with the grade changing by at most `max_change` per metre up to the
/// interval's limit `lim`; the profile is clamped between them where they
/// do not cross.
fn clamp_to_envelopes(
    p: &mut [f64],
    d: &[f64],
    fixed: &[bool],
    crease: &[bool],
    lim: &[f64],
    max_change: f64,
) {
    let n = p.len();
    let mut lo = vec![f64::NEG_INFINITY; n];
    let mut hi = vec![f64::INFINITY; n];
    // Forward from every fixed sample whose interval before is fixed too.
    for a in 1..n {
        if !(fixed[a] && fixed[a - 1]) || a + 1 >= n || fixed[a + 1] || crease[a] {
            continue;
        }
        let g0 = (p[a] - p[a - 1]) / (d[a] - d[a - 1]).max(1e-9);
        let (mut gu, mut gl, mut u, mut l) = (g0, g0, p[a], p[a]);
        let mut i = a + 1;
        while i < n && !fixed[i] {
            let ds = d[i] - d[i - 1];
            let lm = lim[i - 1];
            gu = (gu + max_change * ds).min(lm);
            gl = (gl - max_change * ds).max(-lm);
            u += gu * ds;
            l += gl * ds;
            hi[i] = hi[i].min(u);
            lo[i] = lo[i].max(l);
            i += 1;
        }
    }
    // Backward from every fixed sample whose interval after is fixed too.
    for b in (0..n - 1).rev() {
        if !(fixed[b] && fixed[b + 1]) || b == 0 || fixed[b - 1] || crease[b] {
            continue;
        }
        let g0 = (p[b + 1] - p[b]) / (d[b + 1] - d[b]).max(1e-9);
        // Walking backwards a climb is a fall: the same envelopes.
        let (mut gu, mut gl, mut u, mut l) = (g0, g0, p[b], p[b]);
        let mut i = b;
        while i > 0 && !fixed[i - 1] {
            let ds = d[i] - d[i - 1];
            let lm = lim[i - 1];
            // Going back by ds: the height before is h - g·ds.
            gu = (gu - max_change * ds).max(-lm);
            gl = (gl + max_change * ds).min(lm);
            u -= gu * ds;
            l -= gl * ds;
            hi[i - 1] = hi[i - 1].min(u);
            lo[i - 1] = lo[i - 1].max(l);
            i -= 1;
        }
    }
    for i in 0..n {
        if !fixed[i] && lo[i] <= hi[i] {
            p[i] = p[i].clamp(lo[i], hi[i]);
        }
    }
}

/// Step 3 of [`relimit_profile`]: like [`round_kinks_at`] for the free
/// samples, and a kink at a fixed sample is taken out by its free
/// neighbour (moved halfway towards the grade the curvature limit allows
/// there); every move stays within the grades `lim` of the sample's two
/// intervals, so the grade limit keeps holding.
fn round_kinks_fixed(
    p: &mut [f64],
    d: &[f64],
    fixed: &[bool],
    crease: &[bool],
    lim: &[f64],
    max_change: f64,
) {
    let n = p.len();
    // Moves sample k halfway to `to`, within its intervals' grades.
    let place = |p: &mut [f64], k: usize, to: f64| {
        let mut lo = f64::NEG_INFINITY;
        let mut hi = f64::INFINITY;
        if k > 0 {
            let ds = d[k] - d[k - 1];
            lo = lo.max(p[k - 1] - lim[k - 1] * ds);
            hi = hi.min(p[k - 1] + lim[k - 1] * ds);
        }
        if k + 1 < n {
            let ds = d[k + 1] - d[k];
            lo = lo.max(p[k + 1] - lim[k] * ds);
            hi = hi.min(p[k + 1] + lim[k] * ds);
        }
        let x = p[k] + (to - p[k]) * 0.5;
        if lo <= hi {
            p[k] = x.clamp(lo, hi);
        }
    };
    for _ in 0..2000 {
        let mut worst = 0.0f64;
        for i in 1..n - 1 {
            let (d1, d2) = (d[i] - d[i - 1], d[i + 1] - d[i]);
            if d1 < 0.5 || d2 < 0.5 {
                continue;
            }
            let (g1, g2) = ((p[i] - p[i - 1]) / d1, (p[i + 1] - p[i]) / d2);
            let allowed = max_change * (d1 + d2) / 2.0;
            let excess = (g2 - g1).abs() - allowed;
            if excess <= 1e-6 || crease[i] {
                continue;
            }
            let old = (p[i - 1], p[i], p[i + 1]);
            if !fixed[i] {
                let chord = p[i - 1] + (p[i + 1] - p[i - 1]) * d1 / (d1 + d2);
                let frac = (excess / (g2 - g1).abs()).min(1.0);
                place(p, i, p[i] + (chord - p[i]) * frac);
            } else if !fixed[i + 1] {
                let g = g1 + (g2 - g1).signum() * allowed;
                place(p, i + 1, p[i] + g * d2);
            } else if !fixed[i - 1] {
                let g = g2 - (g2 - g1).signum() * allowed;
                place(p, i - 1, p[i] - g * d1);
            }
            let shift = (p[i - 1] - old.0).abs() + (p[i] - old.1).abs() + (p[i + 1] - old.2).abs();
            worst = worst.max(shift);
        }
        if worst < 1e-5 {
            break;
        }
    }
}

/// Rounds each crease (spec §4.8.8 Q7 (b)): from the crease point (fixed,
/// on the main road's edge) the profile leaves with the main road's slope
/// `gm` (along the edge) and turns to its own grade (its mean grade over
/// the next 10 m) along a parabola – a vertical curve of
/// [`crease_curve_m`], 5 – 10 m. The curve ends lower or higher than the
/// road did there; the difference is spread over the road beyond as a
/// slightly steeper or gentler stretch (within the class grade
/// `max_grade` where the road has room, else up to the next fixed point),
/// so the road keeps its heights farther on. With a fixed point within
/// the curve, a cubic Hermite curve to it. Only free points move. Returns
/// whether anything moved.
pub fn round_creases(
    p: &mut [f64],
    d: &[f64],
    fixed: &[bool],
    crease: &[Option<(f64, f64)>],
    max_grade: f64,
) -> bool {
    let n = p.len();
    let mut moved = false;
    for c in 0..n {
        let Some((gm, dir)) = crease[c] else {
            continue;
        };
        // The points from the crease on, outward, with their distance.
        let mut seq: Vec<(usize, f64)> = Vec::new();
        let mut i = c;
        loop {
            let nx = if dir > 0.0 {
                (i + 1 < n).then_some(i + 1)
            } else {
                i.checked_sub(1)
            };
            let Some(nx) = nx else { break };
            seq.push((nx, (d[nx] - d[c]) * dir));
            if fixed[nx] {
                break;
            }
            i = nx;
        }
        let Some(&(last, u_last)) = seq.last() else {
            continue;
        };
        if seq.len() < 2 || u_last < 1.0 {
            continue;
        }
        let gm_out = gm * dir;
        let h0 = p[c];
        // The road's height `u` metres out (between its points).
        let orig: Vec<(f64, f64)> = std::iter::once((0.0, h0))
            .chain(seq.iter().map(|&(k, u)| (u, p[k])))
            .collect();
        let at = |u: f64| -> f64 {
            let k = orig.partition_point(|q| q.0 <= u).clamp(1, orig.len() - 1);
            let ((u0, a), (u1, b)) = (orig[k - 1], orig[k]);
            if u1 - u0 < 1e-9 {
                b
            } else {
                a + (b - a) * ((u - u0) / (u1 - u0)).clamp(0.0, 1.0)
            }
        };
        let reach = 10.0f64.min(u_last);
        let g1 = (at(reach) - h0) / reach;
        // The steepest the stretch may get: the class grade, or what it
        // has already (the curve never makes it steeper than that – no
        // `junction_room` gained, spec §4.8.8 Q7 follow-up).
        let steepest = |t: &[f64]| -> f64 {
            let mut prev = (0.0, h0);
            let mut worst = 0.0f64;
            for (&(_, u), &h) in seq.iter().zip(t) {
                if u - prev.0 >= 1.0 {
                    worst = worst.max(((h - prev.1) / (u - prev.0)).abs());
                }
                prev = (u, h);
            }
            worst
        };
        let orig_h: Vec<f64> = seq.iter().map(|&(k, _)| p[k]).collect();
        let allowed = max_grade.max(steepest(&orig_h)) + 1e-6;
        let shape = |lc: f64| -> Vec<f64> {
            let mut target: Vec<f64> = Vec::with_capacity(seq.len());
            if u_last <= lc + 1.0 {
                // No room past the curve: a Hermite curve to the fixed point.
                let ge = if seq.len() >= 2 {
                    let (ua, ub) = (seq[seq.len() - 2].1, u_last);
                    (p[last] - p[seq[seq.len() - 2].0]) / (ub - ua).max(1e-9)
                } else {
                    g1
                };
                for &(_, u) in &seq {
                    let t = u / u_last;
                    target.push(hermite(t, h0, gm_out * u_last, p[last], ge * u_last));
                }
            } else {
                let curve = |u: f64| h0 + gm_out * u + (g1 - gm_out) * u * u / (2.0 * lc);
                let delta = at(lc) - curve(lc);
                // Spread the difference beyond the curve, fading out
                // smoothly (no kink where it starts or ends): at most 1.5
                // times its mean extra grade, within the class grade where
                // there is room.
                let head = (max_grade - g1.abs()).max(0.01);
                let spread = (1.5 * delta.abs() / head).max(3.0 * lc).min(u_last - lc);
                for &(k, u) in &seq {
                    target.push(if u <= lc {
                        curve(u)
                    } else if u < lc + spread {
                        let x = (u - lc) / spread;
                        p[k] - delta * (1.0 - x * x * (3.0 - 2.0 * x))
                    } else {
                        p[k]
                    });
                }
            }
            target
        };
        // The curve for the change of grade; shorter (down to 5 m) where
        // that would leave the road steeper than allowed; none where even
        // that would (the crease of Q6 (d) stays).
        let full = crease_curve_m(g1 - gm_out);
        let Some(target) = [full, CREASE_MIN_M]
            .into_iter()
            .map(&shape)
            .find(|t| steepest(t) <= allowed)
        else {
            continue;
        };
        for (&(k, _), &h) in seq.iter().zip(&target) {
            if !fixed[k] && (h - p[k]).abs() > 1e-6 {
                p[k] = h;
                moved = true;
            }
        }
    }
    moved
}

/// Whether a profile needs re-limiting: an interval steeper than its
/// class limit, or a kink above the lint's 8 points at a free sample.
fn needs_relimit(p: &[f64], d: &[f64], fixed: Option<&[bool]>, max_grade: f64) -> bool {
    let n = p.len();
    let grade = |i: usize| (p[i + 1] - p[i]) / (d[i + 1] - d[i]).max(1e-9);
    let free = |i: usize| fixed.is_none_or(|f| !f[i]);
    (0..n - 1).any(|i| d[i + 1] - d[i] >= 0.5 && grade(i).abs() > max_grade + 1e-3)
        || (1..n - 1).any(|i| {
            free(i)
                && d[i] - d[i - 1] >= 0.5
                && d[i + 1] - d[i] >= 0.5
                && (grade(i) - grade(i - 1)).abs() > 0.08
        })
}

/// The class grade and vertical-curvature limits (4.8.3) applied again
/// after the junction blend (spec §4.8.8 J2b) to every road profile that
/// breaks them ([`relimit_profile`] with [`fixed_samples`]): the
/// junctions stay level with the main road, a side road's missed height
/// is spread over as long a stretch as it needs.
pub fn relimit_grades(
    graph: &mut RoadGraph,
    keep: &(dyn Fn(usize, f64) -> bool + Sync),
) -> RelimitStats {
    let js = JunctionSet::new(graph);
    let ids: Vec<usize> = (0..graph.edges.len()).collect();
    let g: &RoadGraph = graph;
    let new = crate::par::map(&ids, crate::par::threads(), |&ei| {
        let e = &g.edges[ei];
        let n = e.profile.len();
        if e.bridge || e.tunnel || n < 3 {
            return None;
        }
        let len = e.length_m();
        let d: Vec<f64> = (0..n).map(|i| sample_m(i, len)).collect();
        let (max_grade, max_change) = limits(e.class);
        // The cheap test first (no junction lookups for the good roads),
        // except at a side road's crease (rounded always, Q7 (b)).
        let creased = js
            .ends(g, ei)
            .iter()
            .any(|end| matches!(end, Some(EndRule::Patch(p, i)) if p.arms[*i].has_corner()));
        if !creased && !needs_relimit(&e.profile, &d, None, max_grade) {
            return None;
        }
        let f = fixed_samples(g, &js, ei, keep);
        let mk = with_mouths(&e.profile, &d, &f);
        let off = f
            .drawn
            .iter()
            .any(|&(i, h)| (e.profile[i] - h).abs() > 1e-3);
        let (mut p, d2, fx) = (mk.heights, mk.dist, mk.fixed);
        let mut rounded = p.clone();
        let creases = round_creases(&mut rounded, &d2, &fx, &mk.crease_grade, max_grade);
        if !off && !creases && !needs_relimit(&p, &d2, Some(&fx), max_grade) {
            return None;
        }
        relimit_profile_creased(&mut p, &d2, &fx, &mk.crease, max_grade, max_change);
        if round_creases(&mut p, &d2, &fx, &mk.crease_grade, max_grade) {
            clamp_grades(&mut p, &d2, &fx, max_grade);
        }
        let mut out = e.profile.clone();
        for (h, o) in p.iter().zip(&mk.orig) {
            if let Some(i) = o {
                out[*i] = *h;
            }
        }
        (out != e.profile).then_some((ei, out))
    });
    drop(js);
    let mut st = RelimitStats::default();
    for (ei, p) in new.into_iter().flatten() {
        let old = &mut graph.edges[ei].profile;
        for (a, b) in old.iter().zip(&p) {
            let c = (a - b).abs();
            if c > 1e-6 {
                st.samples += 1;
                st.max_change_m = st.max_change_m.max(c);
            }
        }
        st.edges += 1;
        *old = p;
    }
    st
}

/// Why a profile interval is steeper than its class limit + 1 % (the
/// lint's `road.vertical.grade_over_limit.CAUSE`, spec §4.8.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SteepCause {
    /// The road's own fixed heights round it (its two junction heights,
    /// bridge decks) lie farther apart than the limit + 1 % allows: the
    /// source data is steeper.
    SourceSteep,
    /// The source heights would allow the limit, but not with the
    /// junction surfaces level with their main roads ([`fixed_samples`]):
    /// too little room between a side road's mouth (on the main road's
    /// surface across, which can lie metres above or below the side road
    /// where it leaves a steep main road at a narrow angle) and the next
    /// fixed height – another junction surface a few metres on (J3's
    /// clusters), a bridge deck.
    JunctionRoom,
    /// Within [`JUNCTION_NEAR_M`] beyond a junction surface's reach.
    Junction,
    /// Elsewhere.
    Other,
}

impl SteepCause {
    pub fn key(self) -> &'static str {
        match self {
            SteepCause::SourceSteep => "source_steep",
            SteepCause::JunctionRoom => "junction_room",
            SteepCause::Junction => "junction",
            SteepCause::Other => "other",
        }
    }
}

/// An interval steeper than its class limit + [`GRADE_TOLERANCE`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Steep {
    pub edge: usize,
    /// The interval between samples `interval` and `interval + 1`.
    pub interval: usize,
    pub grade: f64,
    pub cause: SteepCause,
}

/// Every profile interval (at least 1 m long, not on a bridge or in a
/// tunnel) steeper than its class limit + 1 %, with its cause.
pub fn steep_intervals(
    g: &RoadGraph,
    js: &JunctionSet,
    keep: &(dyn Fn(usize, f64) -> bool + Sync),
) -> Vec<Steep> {
    let ids: Vec<usize> = (0..g.edges.len()).collect();
    let per = crate::par::map(&ids, crate::par::threads(), |&ei| {
        let e = &g.edges[ei];
        let n = e.profile.len();
        let mut out = Vec::new();
        if e.bridge || e.tunnel || n < 2 {
            return out;
        }
        let len = e.length_m();
        let d: Vec<f64> = (0..n).map(|i| sample_m(i, len)).collect();
        let max_grade = limits(e.class).0;
        let steep: Vec<(usize, f64)> = (0..n - 1)
            .filter(|&i| d[i + 1] - d[i] >= 1.0)
            .map(|i| (i, (e.profile[i + 1] - e.profile[i]) / (d[i + 1] - d[i])))
            .filter(|(_, gr)| gr.abs() > max_grade + GRADE_TOLERANCE)
            .collect();
        if steep.is_empty() {
            return out;
        }
        let f = fixed_samples(g, js, ei, keep);
        let mk = with_mouths(&e.profile, &d, &f);
        let (p, d2, fx) = (mk.heights, mk.dist, mk.fixed);
        let reach = js.reach(g, ei);
        // The road's own fixed heights: its ends and the bridge decks.
        let own: Vec<bool> = (0..n)
            .map(|k| k == 0 || k == n - 1 || !keep(ei, d[k]))
            .collect();
        for (i, grade) in steep {
            let a0 = (0..=i).rev().find(|&k| own[k]).unwrap_or(0);
            let b0 = (i + 1..n).find(|&k| own[k]).unwrap_or(n - 1);
            let source = (e.profile[b0] - e.profile[a0]).abs() / (d[b0] - d[a0]).max(1e-9);
            // The fixed points round the interval (a junction's mouth may
            // lie inside it): the steepest grade any two neighbouring ones
            // at least 1 m apart need.
            let lo = d2.partition_point(|&x| x <= d[i] + 1e-9);
            let hi = d2.partition_point(|&x| x < d[i + 1] - 1e-9);
            let a = (0..lo).rev().find(|&k| fx[k]).unwrap_or(0);
            let b = (hi..d2.len()).find(|&k| fx[k]).unwrap_or(d2.len() - 1);
            let anchors: Vec<usize> = (a..=b).filter(|&k| fx[k]).collect();
            let mut mean = 0.0f64;
            for (j, &k) in anchors.iter().enumerate() {
                if let Some(&l) = anchors[j + 1..].iter().find(|&&l| d2[l] - d2[k] >= 1.0) {
                    mean = mean.max((p[l] - p[k]).abs() / (d2[l] - d2[k]));
                }
            }
            // Nothing free round it: nothing the re-limiting could move.
            if (a..=b).all(|k| fx[k]) {
                mean = f64::INFINITY;
            }
            let near = (reach[0] > 0.0 && d[i] <= reach[0] + JUNCTION_NEAR_M)
                || (reach[1] > 0.0 && len - d[i + 1] <= reach[1] + JUNCTION_NEAR_M);
            let cause = if source > max_grade + GRADE_TOLERANCE {
                SteepCause::SourceSteep
            } else if mean > max_grade + GRADE_TOLERANCE {
                SteepCause::JunctionRoom
            } else if near {
                SteepCause::Junction
            } else {
                SteepCause::Other
            };
            out.push(Steep {
                edge: ei,
                interval: i,
                grade,
                cause,
            });
        }
        out
    });
    per.into_iter().flatten().collect()
}

/// The drawn junction surfaces as points (x, surface height, z) for the
/// terrain settling: each patch every `step` m inside its outline, and
/// every side road's blend every metre along, every 0.5 m across (its
/// rows are not flat across like the package strip).
pub fn surface_samples(g: &RoadGraph, js: &JunctionSet, step: f64) -> Vec<[f64; 3]> {
    let verts: Vec<usize> = (0..g.vertices.len()).collect();
    let per = crate::par::map(&verts, crate::par::threads(), |&v| {
        let mut out = Vec::new();
        let Some(p) = js.own_patch(g, v) else {
            return out;
        };
        let (lo, hi) = (p.min, p.max);
        let nx = ((hi[0] - lo[0]) / step).ceil() as usize;
        let nz = ((hi[1] - lo[1]) / step).ceil() as usize;
        for kz in 0..=nz {
            for kx in 0..=nx {
                let q = [
                    (lo[0] + kx as f64 * step).min(hi[0]),
                    (lo[1] + kz as f64 * step).min(hi[1]),
                ];
                if let Some(h) = p.height_at(q) {
                    out.push([q[0], h, q[1]]);
                }
            }
        }
        // The outline itself (the corners reach beyond the grid points).
        for o in &p.outline {
            out.push(o.p);
        }
        for (ai, a) in p.arms.iter().enumerate() {
            let past = js.past_m(g, p, ai);
            if past <= 0.0 {
                continue;
            }
            let n = past.ceil().max(1.0) as usize;
            for k in 0..=n {
                let s = a.mouth_m + past * k as f64 / n as f64;
                let r = js.past_row(g, p, ai, s);
                let w = ((2.0 * a.half) / 0.5).ceil().max(1.0) as usize;
                for j in 0..=w {
                    let t = j as f64 / w as f64;
                    out.push(std::array::from_fn(|c| r[0][c] + (r[1][c] - r[0][c]) * t));
                }
            }
        }
        out
    });
    per.into_iter().flatten().collect()
}

/// The strips' heights after the profiles changed: each point of a strip
/// of a changed edge moves by the profile's change at its place along the
/// edge (`old` the graph the strips were made from, `new` the reconciled
/// one, the same topology). Returns the points moved.
pub fn reheight_strips(
    pkg: &mut WorldPackage,
    old: &RoadGraph,
    new: &RoadGraph,
    edges: &StripEdges,
) -> usize {
    let changed: Vec<bool> = old
        .edges
        .iter()
        .zip(&new.edges)
        .map(|(a, b)| a.profile != b.profile)
        .collect();
    let cums: HashMap<usize, Vec<f64>> = changed
        .iter()
        .enumerate()
        .filter(|(_, c)| **c)
        .map(|(e, _)| (e, cumulative(&old.edges[e].points)))
        .collect();
    let mut moved = 0;
    for (key, tile) in &mut pkg.tiles {
        let Some(ix) = edges.tile(*key, tile.roads.len()) else {
            continue;
        };
        for (strip, &ei) in tile.roads.iter_mut().zip(ix) {
            if ei == NO_EDGE || !changed.get(ei as usize).copied().unwrap_or(false) {
                continue;
            }
            let (eo, en) = (&old.edges[ei as usize], &new.edges[ei as usize]);
            let cum = &cums[&(ei as usize)];
            for q in &mut strip.points {
                let s = project(&eo.points, cum, [f64::from(q[0]), f64::from(q[2])]);
                let dh = en.height_at(s).unwrap_or(0.0) - eo.height_at(s).unwrap_or(0.0);
                if dh.abs() > 1e-6 {
                    q[1] = (f64::from(q[1]) + dh) as f32;
                    moved += 1;
                }
            }
        }
    }
    moved
}

/// The stretches of the package's bridge decks (section `BRDG`) along
/// their edges: (edge, from m, to m) – the decks planned straight between
/// their abutments, which the reconciliation keeps.
pub fn bridge_spans(pkg: &WorldPackage, g: &RoadGraph) -> Vec<(usize, f64, f64)> {
    let by_id: HashMap<ocw_world::graph::EdgeId, usize> =
        g.edges.iter().enumerate().map(|(i, e)| (e.id, i)).collect();
    let mut out = Vec::new();
    for b in pkg.bridges.iter().flat_map(|t| t.values()).flatten() {
        let Some(&ei) = by_id.get(&b.edge) else {
            continue;
        };
        let e = &g.edges[ei];
        let cum = cumulative(&e.points);
        let ends: Vec<f64> = [b.points.first(), b.points.last()]
            .into_iter()
            .flatten()
            .map(|q| project(&e.points, &cum, [f64::from(q[0]), f64::from(q[2])]))
            .collect();
        if let [a, z] = ends[..] {
            out.push((ei, a.min(z), a.max(z)));
        }
    }
    out
}

/// Distance along the polyline of the point nearest to `p`.
fn project(points: &[[f64; 2]], cum: &[f64], p: [f64; 2]) -> f64 {
    let mut best = (f64::INFINITY, 0.0);
    for (k, w) in points.windows(2).enumerate() {
        let (a, b) = (w[0], w[1]);
        let ab = [b[0] - a[0], b[1] - a[1]];
        let l2 = ab[0] * ab[0] + ab[1] * ab[1];
        if l2 <= 0.0 {
            continue;
        }
        let t = (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / l2).clamp(0.0, 1.0);
        let q = [a[0] + ab[0] * t, a[1] + ab[1] * t];
        let d = (q[0] - p[0]).hypot(q[1] - p[1]);
        if d < best.0 - 1e-9 {
            best = (d, cum[k] + t * l2.sqrt());
        }
    }
    best.1
}

/// The junction sections of `pkg` for its (final) graph: `JNCT` with the
/// rules and outlines, flagged as settled when `settled`, and `RDGE`.
pub fn junction_data(pkg: &WorldPackage, settled: bool) -> Option<JunctionData> {
    let g = pkg.graph.as_ref()?;
    let js = JunctionSet::new(g);
    let index = JunctionIndex::of_set(g, &js, if settled { FLAG_SETTLED } else { 0 });
    Some(JunctionData {
        index: Some(Arc::new(index)),
        strip_edges: Some(strip_edges_of(pkg, g)),
        // The mini-roundabouts (J3) stay as the package has them.
        mini_roundabouts: pkg
            .junctions
            .as_ref()
            .and_then(|j| j.mini_roundabouts_for(g).cloned()),
    })
}

// --- the lint -------------------------------------------------------------

/// A junction's drawn carriageways, measured.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Seam {
    pub vertex: usize,
    /// The largest jump of the top carriageway between neighbouring
    /// samples of different roads, less the slope (m).
    pub seam_m: f64,
    /// Area where carriageways of two roads lie within 1 cm (m²).
    pub zfight_m2: f64,
    /// A patch (else a mitre) and the number of roads meeting.
    pub patch: bool,
    pub degree: usize,
    /// Where the largest seam lies and between which (edge index;
    /// `usize::MAX` the patch).
    pub seam_at: [f64; 2],
    pub seam_owners: [usize; 2],
    /// The seam lies at a side road too short for its blend (a through
    /// road's side arm without one: its strip climbs or falls from the
    /// main road's surface at once).
    pub short_side: bool,
}

/// Seams above this (m) are findings (spec §4.8.8: no step above 2 cm).
pub const SEAM_M: f64 = 0.02;
/// Overlaps above this area (m²) are findings.
pub const ZFIGHT_M2: f64 = 0.5;
const SAMPLE_M: f64 = 0.2;
const MARGIN_M: f64 = 2.5;
const PATCH_OWNER: usize = usize::MAX;

/// The drawn carriageways at every vertex with a junction surface (a
/// patch or a mitre), as the renderer draws them (`shape_strip`'s rows,
/// the patch's triangles), sampled from above every 0.2 m over the disc
/// of the widest half width + 2.5 m. Also the vertices where two or more
/// roads meet on the ground without one, with the reason (J3's work).
pub fn seams(pkg: &WorldPackage, g: &RoadGraph) -> (Vec<Seam>, Vec<(usize, &'static str)>) {
    let js = match pkg.junctions.as_ref() {
        Some(j) => JunctionSet::with_index(g, j.index_for(g)),
        None => JunctionSet::new(g),
    };
    let stored = pkg.junctions.as_ref().and_then(|j| j.strip_edges_for(g));
    let edges_owned;
    let edges = match stored {
        Some(s) => s,
        None => {
            edges_owned = strip_edges_of(pkg, g);
            &edges_owned
        }
    };
    // The strips of each edge: (tile, index).
    let mut by_edge: HashMap<u32, Vec<(ocw_world::TileKey, usize)>> = HashMap::new();
    for (key, tile) in &pkg.tiles {
        if let Some(ix) = edges.tile(*key, tile.roads.len()) {
            for (i, &e) in ix.iter().enumerate() {
                if e != NO_EDGE {
                    by_edge.entry(e).or_default().push((*key, i));
                }
            }
        }
    }
    let adj = g.adjacency();
    let verts: Vec<usize> = (0..g.vertices.len())
        .filter(|&v| {
            let a = &adj[v];
            a.len() >= 2
                && !g.boundary_vertices.contains(&VertexId(v as u32))
                && !a.iter().any(|&e| {
                    let e = &g.edges[e];
                    e.bridge || e.tunnel || e.from == e.to
                })
        })
        .collect();
    let per = crate::par::map(&verts, crate::par::threads(), |&v| {
        let Some(rule) = js.rule(g, v) else {
            return Some(Err((v, js.fallback_reason(g, v).unwrap_or("no surface"))));
        };
        // A cluster (J3) is measured once, round its centroid, with the
        // edges of all its vertices.
        let mut c = g.vertices[v];
        let mut edges = adj[v].clone();
        if let VertexRule::Patch(p) = rule {
            if !p.is_at(v) {
                return None;
            }
            if p.members.len() > 1 {
                c = [p.centre[0], p.centre[2]];
                edges = p
                    .members
                    .iter()
                    .flat_map(|&m| adj[m as usize].clone())
                    .collect();
                edges.sort_unstable();
                edges.dedup();
            }
        }
        // The disc of the analysis test, and the whole patch with its
        // mouths (where it meets the strips).
        let mut r = edges
            .iter()
            .map(|&e| g.edges[e].width_m / 2.0)
            .fold(0.0, f64::max)
            + MARGIN_M;
        if let VertexRule::Patch(p) = rule {
            for o in &p.outline {
                r = r.max((o.p[0] - c[0]).hypot(o.p[2] - c[1]) + 1.0);
            }
        }
        let mut tris: Vec<([[f64; 3]; 3], usize)> = Vec::new();
        if let VertexRule::Patch(p) = rule {
            for t in &p.triangles {
                tris.push((t.map(|i| p.vertices[i as usize]), PATCH_OWNER));
            }
        }
        for &ei in &edges {
            for &(key, si) in by_edge.get(&(ei as u32)).into_iter().flatten() {
                let s = &pkg.tiles[&key].roads[si];
                if !s
                    .points
                    .iter()
                    .any(|q| (f64::from(q[0]) - c[0]).hypot(f64::from(q[2]) - c[1]) < r + 8.0)
                {
                    continue;
                }
                let half = s.width_m / 2.0;
                let (pts, rows) = match js.shape_strip(g, ei, &s.points, half) {
                    Some(sh) => (sh.points, sh.rows),
                    None => {
                        let rows = (0..s.points.len())
                            .map(|i| plain_row(&s.points, i, half))
                            .collect();
                        (s.points.clone(), rows)
                    }
                };
                let _ = pts;
                for w in rows.windows(2) {
                    let [l0, r0] = w[0].map(|q| q.map(f64::from));
                    let [l1, r1] = w[1].map(|q| q.map(f64::from));
                    tris.push(([l0, r0, l1], ei));
                    tris.push(([r0, r1, l1], ei));
                }
            }
        }
        let mut m = measure(c, r, &tris, v);
        m.patch = matches!(rule, VertexRule::Patch(_));
        m.degree = match rule {
            VertexRule::Patch(p) => p.arms.len(),
            _ => adj[v].len(),
        };
        if let VertexRule::Patch(p) = rule {
            m.short_side = p.arms.iter().enumerate().any(|(ai, a)| {
                m.seam_owners.contains(&a.edge)
                    && a.blend_m <= 0.0
                    && !a.has_corner()
                    && p.through.is_some_and(|t| !t.contains(&ai))
            });
        }
        Some(Ok(m))
    });
    let mut out = Vec::new();
    let mut without = Vec::new();
    for x in per.into_iter().flatten() {
        match x {
            Ok(s) => out.push(s),
            Err(w) => without.push(w),
        }
    }
    (out, without)
}

/// A side road's surface standing above the main road's drawn surface
/// (spec §4.8.8 Q7: never above the main road at the mouth).
#[derive(Debug, Clone, PartialEq)]
pub struct MouthAbove {
    pub vertex: usize,
    /// How far above the main road's own surface (m) at its worst point.
    pub above_m: f64,
    pub at: [f64; 2],
    /// `mesh`: the junction surface pulled above the main road's own
    /// surface on its carriageway (a corner's slope reaching onto it);
    /// `strip`: a side road's strip lying over the main road's carriageway.
    pub cause: &'static str,
    /// The side road (edge) at the worst point (`usize::MAX`: the patch).
    pub edge: usize,
}

/// More than this above the main road's surface (m) is a finding.
pub const MOUTH_ABOVE_M: f64 = 0.02;

/// Where a side road's drawn surface stands above (or pokes through) the
/// main road's drawn surface at a junction with a through road (spec
/// §4.8.8 Q7): sampled every 0.2 m over the main road's carriageway at the
/// patch (its two arms from the vertex to 1 m past their mouths) and over
/// the main arms' strips within the disc of the seam lint – the junction
/// surface's mesh against the main road's own surface there (`mesh`), the
/// side roads' strips (as `shape_strip` draws them) against the main
/// road's drawn surface (`strip`). One finding per junction, its worst
/// point.
pub fn mouths_above_main(pkg: &WorldPackage, g: &RoadGraph) -> Vec<MouthAbove> {
    let js = match pkg.junctions.as_ref() {
        Some(j) => JunctionSet::with_index(g, j.index_for(g)),
        None => JunctionSet::new(g),
    };
    let stored = pkg.junctions.as_ref().and_then(|j| j.strip_edges_for(g));
    let edges_owned;
    let edges = match stored {
        Some(s) => s,
        None => {
            edges_owned = strip_edges_of(pkg, g);
            &edges_owned
        }
    };
    let mut by_edge: HashMap<u32, Vec<(ocw_world::TileKey, usize)>> = HashMap::new();
    for (key, tile) in &pkg.tiles {
        if let Some(ix) = edges.tile(*key, tile.roads.len()) {
            for (i, &e) in ix.iter().enumerate() {
                if e != NO_EDGE {
                    by_edge.entry(e).or_default().push((*key, i));
                }
            }
        }
    }
    let verts: Vec<usize> = (0..g.vertices.len()).collect();
    let per = crate::par::map(&verts, crate::par::threads(), |&v| {
        let p = js.own_patch(g, v)?;
        let [ma, mb] = p.through?;
        if p.arms.len() < 3 {
            return None;
        }
        let main_edges = [p.arms[ma].edge, p.arms[mb].edge];
        let c = [p.centre[0], p.centre[2]];
        let mut r = p.arms.iter().map(|a| a.half).fold(0.0, f64::max) + MARGIN_M;
        for o in &p.outline {
            r = r.max((o.p[0] - c[0]).hypot(o.p[2] - c[1]) + 1.0);
        }
        // The strips of the arms (shaped as drawn): (triangle, edge).
        let mut tris: Vec<([[f64; 3]; 3], usize)> = Vec::new();
        for a in &p.arms {
            let ei = a.edge;
            for &(key, si) in by_edge.get(&(ei as u32)).into_iter().flatten() {
                let s = &pkg.tiles[&key].roads[si];
                if !s
                    .points
                    .iter()
                    .any(|q| (f64::from(q[0]) - c[0]).hypot(f64::from(q[2]) - c[1]) < r + 8.0)
                {
                    continue;
                }
                let half = s.width_m / 2.0;
                let rows = match js.shape_strip(g, ei, &s.points, half) {
                    Some(sh) => sh.rows,
                    None => (0..s.points.len())
                        .map(|i| plain_row(&s.points, i, half))
                        .collect(),
                };
                for w in rows.windows(2) {
                    let [l0, r0] = w[0].map(|q| q.map(f64::from));
                    let [l1, r1] = w[1].map(|q| q.map(f64::from));
                    tris.push(([l0, r0, l1], ei));
                    tris.push(([r0, r1, l1], ei));
                }
            }
        }
        let ck = |x: f64| x.floor() as i64;
        let mut cells: HashMap<(i64, i64), Vec<u32>> = HashMap::new();
        for (i, (t, _)) in tris.iter().enumerate() {
            let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
            for q in t {
                lo = [lo[0].min(q[0]), lo[1].min(q[2])];
                hi = [hi[0].max(q[0]), hi[1].max(q[2])];
            }
            if hi[0] < c[0] - r || lo[0] > c[0] + r || hi[1] < c[1] - r || lo[1] > c[1] + r {
                continue;
            }
            for x in ck(lo[0])..=ck(hi[0]) {
                for z in ck(lo[1])..=ck(hi[1]) {
                    cells.entry((x, z)).or_default().push(i as u32);
                }
            }
        }
        // The patch's triangles with each corner's height above the main
        // road's own surface (0 on its carriageway, where the mesh is it).
        let ptris: Vec<([[f64; 3]; 3], [f64; 3])> = p
            .triangles
            .iter()
            .map(|t| {
                let c3 = t.map(|i| p.vertices[i as usize]);
                let ex = c3.map(|q| q[1] - p.main_own([q[0], q[2]]).unwrap_or(q[1]));
                (c3, ex)
            })
            .collect();
        let mut pcells: HashMap<(i64, i64), Vec<u32>> = HashMap::new();
        for (i, (t, _)) in ptris.iter().enumerate() {
            let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
            for q in t {
                lo = [lo[0].min(q[0]), lo[1].min(q[2])];
                hi = [hi[0].max(q[0]), hi[1].max(q[2])];
            }
            for x in ck(lo[0])..=ck(hi[0]) {
                for z in ck(lo[1])..=ck(hi[1]) {
                    pcells.entry((x, z)).or_default().push(i as u32);
                }
            }
        }
        let steps = (2.0 * r / SAMPLE_M).ceil() as usize;
        let mut worst: Option<MouthAbove> = None;
        for iz in 0..=steps {
            for ix in 0..=steps {
                let q = [
                    c[0] - r + ix as f64 * SAMPLE_M,
                    c[1] - r + iz as f64 * SAMPLE_M,
                ];
                if (q[0] - c[0]).hypot(q[1] - c[1]) > r {
                    continue;
                }
                // The main road's drawn surface here, and the highest side
                // road surface.
                let on_main = p.main_excess(q) <= 0.0;
                let mut main_h = None;
                let mut side: Option<(f64, usize)> = None;
                if on_main {
                    main_h = p.height_at(q).or_else(|| p.main_own(q));
                    // The junction surface's triangle here: how far its
                    // corners off the carriageway (a side road's corner)
                    // lift it above the main road's own surface.
                    for &ti in pcells.get(&(ck(q[0]), ck(q[1]))).into_iter().flatten() {
                        let (t, ex) = &ptris[ti as usize];
                        if let Some(l) = bary(t, q) {
                            let leak = l[0] * ex[0] + l[1] * ex[1] + l[2] * ex[2];
                            if leak > MOUTH_ABOVE_M
                                && worst.as_ref().is_none_or(|w| leak > w.above_m)
                            {
                                worst = Some(MouthAbove {
                                    vertex: v,
                                    above_m: leak,
                                    at: q,
                                    cause: "mesh",
                                    edge: usize::MAX,
                                });
                            }
                            break;
                        }
                    }
                }
                for &ti in cells.get(&(ck(q[0]), ck(q[1]))).into_iter().flatten() {
                    let (t, o) = &tris[ti as usize];
                    let Some(h) = tri_h(t, q) else { continue };
                    if main_edges.contains(o) {
                        if !on_main {
                            main_h = Some(main_h.map_or(h, |m: f64| m.max(h)));
                        }
                    } else if side.is_none_or(|s| h > s.0) {
                        side = Some((h, *o));
                    }
                }
                let (Some(m), Some((h, o))) = (main_h, side) else {
                    continue;
                };
                let above = h - m;
                if above > MOUTH_ABOVE_M && worst.as_ref().is_none_or(|w| above > w.above_m) {
                    worst = Some(MouthAbove {
                        vertex: v,
                        above_m: above,
                        at: q,
                        cause: "strip",
                        edge: o,
                    });
                }
            }
        }
        worst
    });
    per.into_iter().flatten().collect()
}

/// Flat across, as `road_mesh` draws an unshaped strip.
fn plain_row(pts: &[[f32; 3]], i: usize, half: f32) -> [[f32; 3]; 2] {
    let n = pts.len();
    let (a, b) = (pts[i.saturating_sub(1)], pts[(i + 1).min(n - 1)]);
    let d = [b[0] - a[0], b[2] - a[2]];
    let l = d[0].hypot(d[1]).max(1e-9);
    let r = [-d[1] / l, d[0] / l];
    let p = pts[i];
    [
        [p[0] - r[0] * half, p[1], p[2] - r[1] * half],
        [p[0] + r[0] * half, p[1], p[2] + r[1] * half],
    ]
}

/// A fault of the bands beside a carriageway at a junction (spec §4.8.8
/// *Rule: the sides of a road are as smooth as the road*).
#[derive(Debug, Clone, PartialEq)]
pub struct SideFault {
    /// `gap`, `step`, `kink` or `corner`.
    pub kind: &'static str,
    pub vertex: usize,
    pub at: [f64; 2],
    /// The gap (m), the step (m), the change of grade (points) or the bend
    /// (degrees).
    pub value: f64,
}

/// A gap wider than this (m) between a strip's edge and the junction
/// surface's outline at a mouth corner is a finding.
pub const SIDE_GAP_M: f64 = 0.01;
/// A step higher than this (m) between neighbouring side points less than
/// 0.6 m apart (beyond what the side's grade explains).
pub const SIDE_STEP_M: f64 = 0.05;
/// A change of grade of more than this along a side band (points, over
/// stretches of at least 1 m) beyond the change of the carriageway edge it
/// belongs to.
pub const SIDE_KINK: f64 = 8.0;
/// A bend sharper than this (degrees) between two pieces of a side band,
/// each at least 0.3 m long.
pub const SIDE_CORNER_DEG: f64 = 25.0;

/// The sides of every junction surface as the renderer draws them: each
/// chain of the outline between two mouths (the corner's side band) and
/// the edges of the arms' strips (their rows' corners) from the mouth on,
/// as far as the junction reaches and 5 m more. Measured: `gap` – a mouth
/// corner of the outline that no row of its arm's strip starts at (a
/// hole); `step` – a jump of the side's height between the outline and the
/// strip at a mouth corner, or between neighbouring points of a side less
/// than 0.6 m apart; `kink` – a change of grade along a chain's band over
/// stretches of at least 1 m above 8 points; `corner` – a bend in plan
/// above 25° along a chain or where a chain turns into its arm's edge.
/// One finding per junction and kind, its worst.
pub fn side_faults(pkg: &WorldPackage, g: &RoadGraph) -> Vec<SideFault> {
    let js = match pkg.junctions.as_ref() {
        Some(j) => JunctionSet::with_index(g, j.index_for(g)),
        None => JunctionSet::new(g),
    };
    let stored = pkg.junctions.as_ref().and_then(|j| j.strip_edges_for(g));
    let edges_owned;
    let edges = match stored {
        Some(s) => s,
        None => {
            edges_owned = strip_edges_of(pkg, g);
            &edges_owned
        }
    };
    let mut by_edge: HashMap<u32, Vec<(ocw_world::TileKey, usize)>> = HashMap::new();
    for (key, tile) in &pkg.tiles {
        if let Some(ix) = edges.tile(*key, tile.roads.len()) {
            for (i, &e) in ix.iter().enumerate() {
                if e != NO_EDGE {
                    by_edge.entry(e).or_default().push((*key, i));
                }
            }
        }
    }
    let verts: Vec<usize> = (0..g.vertices.len()).collect();
    let per = crate::par::map(&verts, crate::par::threads(), |&v| {
        let mut out: Vec<SideFault> = Vec::new();
        let Some(p) = js.own_patch(g, v) else {
            return out;
        };
        let mut worst: HashMap<&'static str, SideFault> = HashMap::new();
        let mut note = |kind: &'static str, at: [f64; 2], value: f64| {
            let e = worst.entry(kind).or_insert(SideFault {
                kind,
                vertex: v,
                at,
                value: 0.0,
            });
            if value > e.value {
                e.at = at;
                e.value = value;
            }
        };
        // The arms' strips' rows (shaped as drawn).
        let mut rows_of: HashMap<usize, Vec<Vec<[[f64; 3]; 2]>>> = HashMap::new();
        for a in &p.arms {
            for &(key, si) in by_edge.get(&(a.edge as u32)).into_iter().flatten() {
                let s = &pkg.tiles[&key].roads[si];
                let half = s.width_m / 2.0;
                if let Some(sh) = js.shape_strip(g, a.edge, &s.points, half)
                    && sh.rows.len() >= 2
                {
                    rows_of.entry(a.edge).or_default().push(
                        sh.rows
                            .iter()
                            .map(|r| r.map(|q| q.map(f64::from)))
                            .collect(),
                    );
                }
            }
        }
        let xz = |q: [f64; 3]| [q[0], q[2]];
        let dist = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).hypot(a[1] - b[1]);
        let bend = |a: [f64; 2], b: [f64; 2], c: [f64; 2]| {
            let (u, w) = ([b[0] - a[0], b[1] - a[1]], [c[0] - b[0], c[1] - b[1]]);
            let (lu, lw) = (u[0].hypot(u[1]), w[0].hypot(w[1]));
            if lu < 0.3 || lw < 0.3 {
                return 0.0;
            }
            ((u[0] * w[0] + u[1] * w[1]) / (lu * lw))
                .clamp(-1.0, 1.0)
                .acos()
                .to_degrees()
        };
        // The point of the arm's strip edge that starts at corner `c` and
        // the next one along it (the strip's first row there).
        // (Also the strip's edge from there on, away from the patch.)
        let strip_start = |edge: usize, c: [f64; 3]| -> Option<([f64; 3], [f64; 3], Vec<[f64; 3]>)> {
            for rows in rows_of.get(&edge).into_iter().flatten() {
                for (i, r) in rows.iter().enumerate() {
                    for k in 0..2 {
                        if dist(xz(r[k]), xz(c)) < SIDE_GAP_M {
                            let j = if i == 0 { 1 } else { i - 1 };
                            let side: Vec<[f64; 3]> = if i == 0 {
                                rows.iter().map(|r| r[k]).collect()
                            } else {
                                rows[..=i].iter().rev().map(|r| r[k]).collect()
                            };
                            return Some((r[k], rows[j][k], side));
                        }
                    }
                }
            }
            None
        };
        for ch in &p.chains {
            let pts: Vec<[f64; 3]> = ch.points.clone();
            let n = pts.len();
            if n < 2 {
                continue;
            }
            // The chain point at least 0.3 m (straight) before / after
            // point `i`: bends are measured over pieces of 0.3 m, however
            // finely the chain is cut.
            let reach = |i: usize, fwd: bool| -> Option<usize> {
                let mut k = i;
                loop {
                    if fwd {
                        k += 1;
                        if k >= n {
                            return None;
                        }
                    } else {
                        if k == 0 {
                            return None;
                        }
                        k -= 1;
                    }
                    if dist(xz(pts[k]), xz(pts[i])) >= 0.3 {
                        return Some(k);
                    }
                }
            };
            // Plan: bends along the chain.
            for i in 1..n - 1 {
                let (Some(a), Some(c)) = (reach(i, false), reach(i, true)) else {
                    continue;
                };
                let b = bend(xz(pts[a]), xz(pts[i]), xz(pts[c]));
                if b > SIDE_CORNER_DEG {
                    note("corner", xz(pts[i]), b);
                }
            }
            // The drawn surface's slope at a point (its field, which the
            // outline points lie on): a side band that turns on a sloping
            // surface changes its grade by the turn alone (on a plane at
            // 20 % round a corner of 1.5 m by 0.13 per metre) – that is the
            // surface's slope, not a kink or step of the side.
            let slope = |q: [f64; 2]| {
                let e = 0.05;
                [
                    (p.surface([q[0] + e, q[1]]) - p.surface([q[0] - e, q[1]])) / (2.0 * e),
                    (p.surface([q[0], q[1] + e]) - p.surface([q[0], q[1] - e])) / (2.0 * e),
                ]
            };
            // A stretch of at least 1 m from `pts[0]` along `pts`: its grade
            // and direction.
            let first_metre = |pts: &[[f64; 3]]| -> Option<(f64, [f64; 2])> {
                let mut walked = 0.0;
                for w in pts.windows(2) {
                    walked += dist(xz(w[0]), xz(w[1]));
                    if walked >= 1.0 {
                        let d = [w[1][0] - pts[0][0], w[1][2] - pts[0][2]];
                        let l = d[0].hypot(d[1]).max(1e-9);
                        return Some(((w[1][1] - pts[0][1]) / walked, [d[0] / l, d[1] / l]));
                    }
                }
                None
            };
            // The change of grade from stretch `a` into `b` at `at`, less
            // what the turn explains on the surface's slope (points).
            let kink = |a: (f64, [f64; 2]), b: (f64, [f64; 2]), at: [f64; 2]| {
                let gr = slope(at);
                let turned = gr[0] * (b.1[0] - a.1[0]) + gr[1] * (b.1[1] - a.1[1]);
                ((b.0 - a.0) - turned).abs() * 100.0
            };
            // Height: grade changes along the chain over stretches ≥ 1 m.
            let mut stretch: Vec<((f64, [f64; 2]), [f64; 2])> = Vec::new();
            let mut start = 0;
            while start + 1 < n {
                let mut walked = 0.0;
                let mut end = None;
                for k in start + 1..n {
                    walked += dist(xz(pts[k - 1]), xz(pts[k]));
                    if walked >= 1.0 {
                        end = Some(k);
                        break;
                    }
                }
                let Some(k) = end else { break };
                if let Some(st) = first_metre(&pts[start..=k]) {
                    stretch.push((st, xz(pts[k])));
                }
                start = k;
            }
            for w in stretch.windows(2) {
                let dg = kink(w[0].0, w[1].0, w[0].1);
                if dg > SIDE_KINK {
                    note("kink", w[0].1, dg);
                }
            }
            // Steps between close points: a jump the surface's slope does
            // not explain.
            for w in pts.windows(2) {
                let d = dist(xz(w[0]), xz(w[1]));
                if d < 0.6 {
                    let m = [(w[0][0] + w[1][0]) / 2.0, (w[0][2] + w[1][2]) / 2.0];
                    let gr = slope(m);
                    let rise = gr[0] * (w[1][0] - w[0][0]) + gr[1] * (w[1][2] - w[0][2]);
                    let jump = (w[1][1] - w[0][1] - rise).abs();
                    if jump > SIDE_STEP_M {
                        note("step", xz(w[1]), jump);
                    }
                }
            }
            // Its two ends meet the arms' strips.
            let (i0, i1) = (reach(0, true).unwrap_or(1), reach(n - 1, false).unwrap_or(n - 2));
            for (end, inner, arm, from_start) in [
                (pts[0], pts[i0], ch.from_arm, true),
                (pts[n - 1], pts[i1], ch.to_arm, false),
            ] {
                let a = &p.arms[arm];
                match strip_start(a.edge, end) {
                    None => {
                        // A strip may not exist (an arm inside a cluster):
                        // only arms with strips count.
                        if rows_of.contains_key(&a.edge) {
                            note("gap", xz(end), 1.0);
                        }
                    }
                    Some((q, next, edge)) => {
                        let dh = (q[1] - end[1]).abs();
                        if dh > 0.02 {
                            note("step", xz(end), dh);
                        }
                        // The grade along the side at the joint: the
                        // chain's last metre into the strip edge's first.
                        let inward: Vec<[f64; 3]> = if from_start {
                            pts.clone()
                        } else {
                            pts.iter().rev().copied().collect()
                        };
                        if let (Some(a), Some(b)) = (first_metre(&inward), first_metre(&edge)) {
                            let a = (-a.0, [-a.1[0], -a.1[1]]);
                            let dg = kink(a, b, xz(end));
                            if dg > SIDE_KINK {
                                note("kink", xz(end), dg);
                            }
                        }
                        let b = bend(xz(inner), xz(end), xz(next));
                        if b > SIDE_CORNER_DEG {
                            note("corner", xz(end), b);
                        }
                    }
                }
            }
        }
        out.extend(worst.into_values());
        out
    });
    per.into_iter().flatten().collect()
}

/// The barycentric coordinates of `p` in the triangle (xz) `t`, if inside.
fn bary(t: &[[f64; 3]; 3], p: [f64; 2]) -> Option<[f64; 3]> {
    let (a, b, c) = ([t[0][0], t[0][2]], [t[1][0], t[1][2]], [t[2][0], t[2][2]]);
    let cr = |u: [f64; 2], v: [f64; 2]| u[0] * v[1] - u[1] * v[0];
    let sub = |u: [f64; 2], v: [f64; 2]| [u[0] - v[0], u[1] - v[1]];
    let d = cr(sub(b, a), sub(c, a));
    if d.abs() < 1e-12 {
        return None;
    }
    let w1 = cr(sub(p, a), sub(c, a)) / d;
    let w2 = cr(sub(b, a), sub(p, a)) / d;
    let w0 = 1.0 - w1 - w2;
    let e = -1e-4;
    (w0 >= e && w1 >= e && w2 >= e).then_some([w0, w1, w2])
}

fn tri_h(t: &[[f64; 3]; 3], p: [f64; 2]) -> Option<f64> {
    let (a, b, c) = ([t[0][0], t[0][2]], [t[1][0], t[1][2]], [t[2][0], t[2][2]]);
    let cr = |u: [f64; 2], v: [f64; 2]| u[0] * v[1] - u[1] * v[0];
    let sub = |u: [f64; 2], v: [f64; 2]| [u[0] - v[0], u[1] - v[1]];
    let d = cr(sub(b, a), sub(c, a));
    if d.abs() < 1e-12 {
        return None;
    }
    let w1 = cr(sub(p, a), sub(c, a)) / d;
    let w2 = cr(sub(b, a), sub(p, a)) / d;
    let w0 = 1.0 - w1 - w2;
    let e = -1e-4;
    (w0 >= e && w1 >= e && w2 >= e).then(|| w0 * t[0][1] + w1 * t[1][1] + w2 * t[2][1])
}

/// The seam and the overlap of the carriageways `tris` (with their owner)
/// over the disc of radius `r` round `c`.
fn measure(c: [f64; 2], r: f64, tris: &[([[f64; 3]; 3], usize)], v: usize) -> Seam {
    let cell = 1.0;
    let ck = |x: f64| (x / cell).floor() as i64;
    let mut cells: HashMap<(i64, i64), Vec<u32>> = HashMap::new();
    for (i, (t, _)) in tris.iter().enumerate() {
        let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        for q in t {
            lo = [lo[0].min(q[0]), lo[1].min(q[2])];
            hi = [hi[0].max(q[0]), hi[1].max(q[2])];
        }
        if hi[0] < c[0] - r || lo[0] > c[0] + r || hi[1] < c[1] - r || lo[1] > c[1] + r {
            continue;
        }
        for x in ck(lo[0])..=ck(hi[0]) {
            for z in ck(lo[1])..=ck(hi[1]) {
                cells.entry((x, z)).or_default().push(i as u32);
            }
        }
    }
    let steps = (2.0 * r / SAMPLE_M).ceil() as usize;
    let w = steps + 1;
    let mut top: Vec<Option<(usize, f64)>> = vec![None; w * w];
    let mut out = Seam {
        vertex: v,
        ..Default::default()
    };
    for iz in 0..=steps {
        for ix in 0..=steps {
            let p = [
                c[0] - r + ix as f64 * SAMPLE_M,
                c[1] - r + iz as f64 * SAMPLE_M,
            ];
            if (p[0] - c[0]).hypot(p[1] - c[1]) > r {
                continue;
            }
            let mut surf: Vec<(usize, f64)> = Vec::new();
            for &ti in cells.get(&(ck(p[0]), ck(p[1]))).into_iter().flatten() {
                let (t, o) = &tris[ti as usize];
                let Some(h) = tri_h(t, p) else { continue };
                match surf.iter_mut().find(|s| s.0 == *o) {
                    Some(s) => s.1 = s.1.max(h),
                    None => surf.push((*o, h)),
                }
            }
            if surf.len() >= 2 {
                let lo = surf.iter().map(|s| s.1).fold(f64::INFINITY, f64::min);
                let hi = surf.iter().map(|s| s.1).fold(f64::NEG_INFINITY, f64::max);
                if hi - lo < 0.01 {
                    out.zfight_m2 += SAMPLE_M * SAMPLE_M;
                }
            }
            top[iz * w + ix] = surf.into_iter().max_by(|a, b| a.1.total_cmp(&b.1));
        }
    }
    for iz in 0..=steps {
        for ix in 0..=steps {
            let Some((e0, h0)) = top[iz * w + ix] else {
                continue;
            };
            for (jx, jz) in [(ix + 1, iz), (ix, iz + 1)] {
                if jx > steps || jz > steps {
                    continue;
                }
                let Some((e1, h1)) = top[jz * w + jx] else {
                    continue;
                };
                if e1 == e0 {
                    continue;
                }
                // Less the slope: the difference to the sample before (same
                // owner) or after.
                let (dx, dz) = (jx - ix, jz - iz);
                let before = (ix >= dx && iz >= dz)
                    .then(|| top[(iz - dz) * w + ix - dx])
                    .flatten()
                    .filter(|q| q.0 == e0)
                    .map(|q| h0 - q.1);
                let after = (jx + dx <= steps && jz + dz <= steps)
                    .then(|| top[(jz + dz) * w + jx + dx])
                    .flatten()
                    .filter(|q| q.0 == e1)
                    .map(|q| q.1 - h1);
                if let Some(slope) = before.or(after) {
                    let s = (h1 - h0 - slope).abs();
                    if s > out.seam_m {
                        out.seam_m = s;
                        out.seam_at = [
                            c[0] - r + ix as f64 * SAMPLE_M,
                            c[1] - r + iz as f64 * SAMPLE_M,
                        ];
                        out.seam_owners = [e0, e1];
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests;
