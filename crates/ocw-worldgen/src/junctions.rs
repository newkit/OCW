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
    /// Samples set on the longer blends of side roads whose blend the
    /// edge's length cut short (first round).
    pub longer_blend_samples: usize,
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
        let Some(p) = js.patch(g, v) else {
            continue;
        };
        for (ai, a) in p.arms.iter().enumerate() {
            let side = p.through.is_some_and(|t| !t.contains(&ai));
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
        let Some(p) = js.patch(g, v) else {
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
        let Some(p) = js.patch(g, v) else {
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

/// Cubic Hermite on [0, 1].
fn hermite(t: f64, p0: f64, m0: f64, p1: f64, m1: f64) -> f64 {
    let (t2, t3) = (t * t, t * t * t);
    (2.0 * t3 - 3.0 * t2 + 1.0) * p0
        + (t3 - 2.0 * t2 + t) * m0
        + (-2.0 * t3 + 3.0 * t2) * p1
        + (t3 - t2) * m1
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
        Some(EndRule::Patch(q, _)) if q.vertex == p.vertex => {
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
        st.residual_m = st.residual_m.max(d);
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
                .filter(|&v| js.patch(graph, v).is_some())
                .count();
        }
        let drawn = drawn_samples(graph, &js, keep);
        // First round: the side roads whose blend the edge's length cut
        // short get the blend they need in their profile.
        let blends = if first {
            longer_blends(graph, &js, keep)
        } else {
            Vec::new()
        };
        drop(js);
        if first {
            st.longer_blend_samples = blends.iter().map(Vec::len).sum();
        }
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
        };
    }
    let len = e.length_m();
    let mut reach = [0.0f64; 2];
    let mut mouths = Vec::new();
    let mut drawn = Vec::new();
    let mut cum = None;
    for (k, end) in js.ends(g, ei).iter().enumerate() {
        if let Some(EndRule::Patch(p, ai)) = end {
            let a = &p.arms[*ai];
            let main = p.through.is_none_or(|t| t.contains(ai));
            reach[k] = a.mouth_m + if main { PROFILE_STEP_M } else { 0.0 };
            if !main {
                let s = a.s_edge(a.mouth_m);
                if s > 1e-3 && s < len - 1e-3 && keep(ei, s) {
                    mouths.push((s, 0.5 * (a.row[0][1] + a.row[1][1])));
                }
                let cum = cum.get_or_insert_with(|| cumulative(&e.points));
                for i in 1..n - 1 {
                    let si = sample_m(i, len);
                    let m = if k == 0 { si } else { len - si };
                    if m <= a.mouth_m - 1e-6 && keep(ei, si) {
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
    }
}

/// A profile with the mouths of [`Fixed`] inserted as fixed points and
/// its samples inside side arms' patches at the drawn height: (heights,
/// distances, fixed, the original sample of each point).
fn with_mouths(
    p: &[f64],
    d: &[f64],
    f: &Fixed,
) -> (Vec<f64>, Vec<f64>, Vec<bool>, Vec<Option<usize>>) {
    let mut pts: Vec<(f64, f64, bool, Option<usize>)> = (0..p.len())
        .map(|i| (d[i], p[i], f.samples[i], Some(i)))
        .collect();
    for &(i, h) in &f.drawn {
        pts[i].1 = h;
    }
    for &(s, h) in &f.mouths {
        // On a sample: that sample is the mouth (inside the patch, kept).
        if let Some(q) = pts.iter_mut().find(|q| (q.0 - s).abs() < 0.05) {
            q.2 = true;
            continue;
        }
        pts.push((s, h, true, None));
    }
    pts.sort_by(|a, b| a.0.total_cmp(&b.0));
    (
        pts.iter().map(|q| q.1).collect(),
        pts.iter().map(|q| q.0).collect(),
        pts.iter().map(|q| q.2).collect(),
        pts.iter().map(|q| q.3).collect(),
    )
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
    let n = p.len();
    if n < 3 {
        return 0;
    }
    let before = p.to_vec();
    let lim = clamp_grades(p, d, fixed, max_grade);
    clamp_to_envelopes(p, d, fixed, &lim, max_change_per_m);
    round_kinks_fixed(p, d, fixed, &lim, max_change_per_m);
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
fn clamp_to_envelopes(p: &mut [f64], d: &[f64], fixed: &[bool], lim: &[f64], max_change: f64) {
    let n = p.len();
    let mut lo = vec![f64::NEG_INFINITY; n];
    let mut hi = vec![f64::INFINITY; n];
    // Forward from every fixed sample whose interval before is fixed too.
    for a in 1..n {
        if !(fixed[a] && fixed[a - 1]) || a + 1 >= n || fixed[a + 1] {
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
        if !(fixed[b] && fixed[b + 1]) || b == 0 || fixed[b - 1] {
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
fn round_kinks_fixed(p: &mut [f64], d: &[f64], fixed: &[bool], lim: &[f64], max_change: f64) {
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
            if excess <= 1e-6 {
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
        // The cheap test first (no junction lookups for the good roads).
        if !needs_relimit(&e.profile, &d, None, max_grade) {
            return None;
        }
        let f = fixed_samples(g, &js, ei, keep);
        let (mut p, d2, fx, orig) = with_mouths(&e.profile, &d, &f);
        let off = f
            .drawn
            .iter()
            .any(|&(i, h)| (e.profile[i] - h).abs() > 1e-3);
        if !off && !needs_relimit(&p, &d2, Some(&fx), max_grade) {
            return None;
        }
        relimit_profile(&mut p, &d2, &fx, max_grade, max_change);
        let mut out = e.profile.clone();
        for (h, o) in p.iter().zip(&orig) {
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
        let (p, d2, fx, _) = with_mouths(&e.profile, &d, &f);
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
        let Some(p) = js.patch(g, v) else {
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
            if a.blend_m <= 0.0 {
                continue;
            }
            let n = a.blend_m.ceil().max(1.0) as usize;
            for k in 0..=n {
                let s = a.mouth_m + a.blend_m * k as f64 / n as f64;
                let r = JunctionSet::blend_row(g, p, ai, s);
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
            return Err((v, js.fallback_reason(g, v).unwrap_or("no surface")));
        };
        let c = g.vertices[v];
        // The disc of the analysis test, and the whole patch with its
        // mouths (where it meets the strips).
        let mut r = adj[v]
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
        for &ei in &adj[v] {
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
        m.degree = adj[v].len();
        if let VertexRule::Patch(p) = rule {
            m.short_side = p.arms.iter().enumerate().any(|(ai, a)| {
                m.seam_owners.contains(&a.edge)
                    && a.blend_m <= 0.0
                    && p.through.is_some_and(|t| !t.contains(&ai))
            });
        }
        Ok(m)
    });
    let mut out = Vec::new();
    let mut without = Vec::new();
    for x in per {
        match x {
            Ok(s) => out.push(s),
            Err(w) => without.push(w),
        }
    }
    (out, without)
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
