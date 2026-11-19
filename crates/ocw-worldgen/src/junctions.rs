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
//! All pure; the profiles' grade limits are not re-applied after the blend
//! (a side road climbing 20 % may climb more steeply just beyond the main
//! road's edge: the height it did not climb inside the junction).

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
pub const ROUNDS: usize = 3;

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
fn through_pair(g: &RoadGraph, js: &JunctionSet, adj: &[Vec<usize>], v: usize) -> Option<[usize; 2]> {
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
    /// Profile intervals near a junction steeper than their class limit
    /// + 1 % after the blend (the side roads' missed height).
    pub steep_intervals: usize,
}

/// The drawn surface at every profile sample within reach of a junction
/// surface: (edge, sample, height).
fn drawn_samples(
    g: &RoadGraph,
    js: &JunctionSet,
    keep: &(dyn Fn(usize, f64) -> bool + Sync),
) -> Vec<(usize, usize, f64)> {
    let verts: Vec<usize> = (0..g.vertices.len()).collect();
    let per = crate::par::map(&verts, crate::par::threads(), |&v| {
        let mut out = Vec::new();
        let Some(p) = js.patch(g, v) else {
            return out;
        };
        // The vertex samples of all arms, set only together (one height).
        let mut at_vertex = Vec::new();
        let mut vertex_ok = true;
        for (ai, a) in p.arms.iter().enumerate() {
            let e = &g.edges[a.edge];
            let len = e.length_m();
            let n = e.profile.len();
            let reach = a.mouth_m + a.blend_m + 1e-6;
            let cum = cumulative(&e.points);
            // A main arm (a through road's, or any of a blended patch)
            // shapes the patch with its height and grade at the mouth:
            // the sample whose interval holds the mouth stays as it is, or
            // the patch made again would differ (no fixed point). A side
            // arm's mouth row lies on the main road; only its blend length
            // follows its height.
            let side = p.through.is_some_and(|t| !t.contains(&ai));
            let dist = |i: usize| {
                let s = sample_m(i, len);
                if a.at_from { s } else { len - s }
            };
            let outward = |i: usize| -> Option<usize> {
                if a.at_from {
                    (i + 1 < n).then_some(i + 1)
                } else {
                    i.checked_sub(1)
                }
            };
            for i in 0..n {
                let s = sample_m(i, len);
                let m = dist(i);
                let vertex = if a.at_from { i == 0 } else { i == n - 1 };
                let allowed = m <= reach
                    && keep(a.edge, s)
                    && (side || outward(i).is_some_and(|o| dist(o) <= a.mouth_m + 1e-6));
                if !allowed {
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
        Some(EndRule::Patch(q, _)) if q.vertex == p.vertex => js.drawn_height_at_end(g, ei, k, s, c),
        _ => None,
    }
}

/// Sets the road profiles near every junction surface to the drawn
/// surface (spec §4.8.8 J2) – inside the patch its height along the
/// centre line, along a side road's blend the blended rows – then makes
/// the patches again on the new profiles and repeats ([`ROUNDS`]).
/// `keep(edge, s)`: whether the sample `s` metres along `edge` may change
/// (not on a planned bridge deck). Mitres are not touched (their corners
/// follow the grades). Bridges and tunnels have no patch.
pub fn reconcile_profiles(
    graph: &mut RoadGraph,
    keep: &(dyn Fn(usize, f64) -> bool + Sync),
) -> ReconcileStats {
    let mut st = ReconcileStats::default();
    for round in 0..ROUNDS {
        let js = JunctionSet::new(graph);
        if round == 0 {
            st.patches = (0..graph.vertices.len())
                .filter(|&v| js.patch(graph, v).is_some())
                .count();
        }
        let drawn = drawn_samples(graph, &js, keep);
        drop(js);
        // A sample both ends reach (the middle of a short edge): the mean.
        let mut by: BTreeMap<(usize, usize), (f64, u32)> = BTreeMap::new();
        for (e, i, h) in drawn {
            let x = by.entry((e, i)).or_insert((0.0, 0));
            x.0 += h;
            x.1 += 1;
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
    // How far the final profiles are from the final patches' surface.
    let js = JunctionSet::new(graph);
    for (e, i, h) in drawn_samples(graph, &js, keep) {
        st.residual_m = st.residual_m.max((h - graph.edges[e].profile[i]).abs());
    }
    // Steep intervals within the junctions' reach.
    for (ei, e) in graph.edges.iter().enumerate() {
        let [r0, r1] = js.reach(graph, ei);
        if (r0 <= 0.0 && r1 <= 0.0) || e.profile.len() < 2 {
            continue;
        }
        let len = e.length_m();
        let limit = limits(e.class).0 + 0.01;
        for i in 1..e.profile.len() {
            let (s0, s1) = (sample_m(i - 1, len), sample_m(i, len));
            if s1 - s0 < 0.5 {
                continue;
            }
            let near = s0 <= r0 || len - s1 <= r1;
            if near && ((e.profile[i] - e.profile[i - 1]) / (s1 - s0)).abs() > limit {
                st.steep_intervals += 1;
            }
        }
    }
    st
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
pub fn seams(
    pkg: &WorldPackage,
    g: &RoadGraph,
) -> (Vec<Seam>, Vec<(usize, &'static str)>) {
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
                if !s.points.iter().any(|q| {
                    (f64::from(q[0]) - c[0]).hypot(f64::from(q[2]) - c[1]) < r + 8.0
                }) {
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
        Ok(measure(c, r, &tris, v))
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
                    out.seam_m = out.seam_m.max((h1 - h0 - slope).abs());
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests;
