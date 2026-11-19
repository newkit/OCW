//! Junction surfaces in the road meshes (spec §4.8.8, stage J1): the
//! renderer's side of `ocw_world::junction`, on the current packages.
//!
//! Per tile, every road strip is matched to its graph edge (by geometry:
//! its points lie on the edge's polyline) and shaped by the pure junction
//! module: trimmed at the mouths, re-heighted along a side road's blend,
//! mitred at way splits ([`road_mesh::road_meshes_rows`] draws it along
//! its rows). The tile draws the patches of the vertices inside it, whole
//! (also where a junction reaches into the neighbouring tile, whose strips
//! are trimmed by the same function): the surface in the main road's
//! layer, colour and frame, the markings at the mouths (the §4.8.8 Q2
//! fallback: a through road's centre line continues, its edge lines break
//! on the sides where other roads leave, the side roads' lines end at
//! their mouths; no lines across other junctions), and the outline's lip,
//! shoulder, skirt or wall, with kerb and sidewalk where the roads have
//! them (§4.8.8 Q4 fallback). The terrain is lowered under the patches
//! (5 cm under the surface), whose corners reach beyond the strips'
//! settled ground.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

use bevy::prelude::*;
use ocw_world::graph::RoadGraph;
use ocw_world::junction::section::{NO_EDGE, StripEdges};
use ocw_world::junction::{JunctionSet, Patch};
use ocw_world::package::RoadStrip;
use ocw_world::road_dress::{CycleLane, MarkCode};
use ocw_world::{RoadDress, RoadSurface, TileKey, WorldPackage};

use crate::package::MeshArrays;
use crate::road_mesh::{
    DressLook, Kind, RoadContext, RoadMeshes, SideDress, code_b, road_meshes_rows, side_dress,
};

/// The terrain is kept this far under a patch's drawn surface (m).
pub const TERRAIN_UNDER_M: f32 = 0.05;

/// The junctions of a package and which edges reach into each tile.
pub struct PackageJunctions {
    pub set: JunctionSet,
    tile_edges: HashMap<TileKey, Vec<u32>>,
    /// The edge of every strip (section `RDGE`), if the package has it for
    /// its graph: no geometric matching then.
    strip_edges: Option<StripEdges>,
    /// The package's profiles are reconciled with the junction surfaces
    /// and its terrain is settled under them (section `JNCT`, J2): the
    /// renderer leaves the terrain as it is.
    pub settled: bool,
}

impl PackageJunctions {
    /// `None` for a package without a road graph.
    pub fn new(pkg: &WorldPackage) -> Option<Self> {
        let g = pkg.graph.as_ref()?;
        let mut pj = Self::of_graph(g, f64::from(pkg.tile_size_m));
        if let Some(j) = &pkg.junctions {
            let index = j.index_for(g);
            pj.settled = index.as_ref().is_some_and(|i| i.settled());
            pj.set = JunctionSet::with_index(g, index);
            pj.strip_edges = j.strip_edges_for(g).cloned();
        }
        Some(pj)
    }

    /// The junctions of a road graph in tiles of `size` metres.
    pub fn of_graph(g: &RoadGraph, size: f64) -> Self {
        Self {
            set: JunctionSet::new(g),
            tile_edges: ocw_world::junction::section::tile_edges(g, size),
            strip_edges: None,
            settled: false,
        }
    }

    /// Edges that may have strips in the tile `key`.
    pub fn tile_edges(&self, key: TileKey) -> &[u32] {
        self.tile_edges.get(&key).map_or(&[], Vec::as_slice)
    }

    /// The edge of each of the `count` strips of tile `key`: stored
    /// (section `RDGE`), else matched by geometry.
    pub fn strip_edges(
        &self,
        g: &RoadGraph,
        key: TileKey,
        strips: &[RoadStrip],
    ) -> Vec<Option<usize>> {
        match self
            .strip_edges
            .as_ref()
            .and_then(|s| s.tile(key, strips.len()))
        {
            Some(stored) => stored
                .iter()
                .map(|&e| (e != NO_EDGE && (e as usize) < g.edges.len()).then_some(e as usize))
                .collect(),
            None => match_strips(g, self.tile_edges(key), strips),
        }
    }
}
type CacheEntry = (Weak<WorldPackage>, Option<Arc<PackageJunctions>>);
static CACHE: Mutex<Vec<CacheEntry>> = Mutex::new(Vec::new());

/// The junctions of a resident package, made once per package (the tiles
/// of one package are built in parallel tasks).
pub fn package_junctions(pkg: &Arc<WorldPackage>) -> Option<Arc<PackageJunctions>> {
    // `OCW_JUNCTIONS=off`: the strips as the package has them (before /
    // after comparisons, frame-rate A/B on one binary).
    if std::env::var("OCW_JUNCTIONS").is_ok_and(|v| v == "off") {
        return None;
    }
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache.retain(|(w, _)| w.strong_count() > 0);
    if let Some((_, j)) = cache
        .iter()
        .find(|(w, _)| std::ptr::eq(w.as_ptr(), Arc::as_ptr(pkg)))
    {
        return j.clone();
    }
    let j = PackageJunctions::new(pkg).map(Arc::new);
    cache.push((Arc::downgrade(pkg), j.clone()));
    j
}

/// Who a part of the road meshes belongs to (the analysis test).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Owner {
    /// The strips of a graph edge.
    Edge(usize),
    /// The junction surface of a vertex.
    Patch(u32),
    /// A strip matched to no edge.
    Strip(usize),
}

/// A tile's roads, shaped for their junctions.
pub struct ShapedTile<'a> {
    pub strips: Vec<RoadStrip>,
    pub rows: Vec<Option<Vec<[[f32; 3]; 2]>>>,
    pub owners: Vec<Owner>,
    /// The patches drawn in this tile, with the strips their arms take
    /// their look from (per arm).
    pub patches: Vec<(&'a Patch, Vec<ArmLook>)>,
}

/// What an arm of a patch looks like (from its strip).
#[derive(Debug, Clone)]
pub struct ArmLook {
    /// The strip runs against its edge (its left is the edge's right).
    pub reversed: bool,
    pub class: ocw_world::graph::RoadClass,
    pub surface: RoadSurface,
    pub dress: RoadDress,
    /// OSM smoothness, tracktype and surface value (the wear).
    pub condition: ocw_world::RoadCondition,
}

/// The edge of each strip of `tile` (`None`: unmatched), by geometry
/// (`ocw_world::junction::section::match_strips`).
pub use ocw_world::junction::section::match_strips;

/// Moves the strip's crossings with its cut start, and those that now lie
/// in a junction surface to its mouth (§4.8.8: stop and give-way lines at
/// the mouth line).
fn move_crossings(
    s: &mut RoadStrip,
    cut0: f32,
    ends: [bool; 2],
    style: &ocw_world::road_dress::MarkingStyle,
) {
    if s.dress.crossings.is_empty() {
        return;
    }
    let mut len = 0.0f32;
    for w in s.points.windows(2) {
        len += (Vec3::from(w[1]) - Vec3::from(w[0])).length();
    }
    for c in &mut s.dress.crossings {
        c.along_m -= cut0;
        let half = style.crossing_length_m(c.kind) / 2.0 + 0.3;
        if ends[0] && c.along_m < half {
            c.along_m = half;
        }
        if ends[1] && c.along_m > len - half {
            c.along_m = len - half;
        }
    }
    s.dress
        .crossings
        .retain(|c| c.along_m >= 0.0 && c.along_m <= len);
}

/// The tile's strips shaped for their junctions, and the patches it draws.
pub fn shape_tile<'a>(
    g: &RoadGraph,
    pj: &'a PackageJunctions,
    key: TileKey,
    size: f64,
    strips: &[RoadStrip],
    style: &ocw_world::road_dress::MarkingStyle,
) -> ShapedTile<'a> {
    let edges = pj.tile_edges(key);
    let matched = pj.strip_edges(g, key, strips);
    let mut out = ShapedTile {
        strips: Vec::with_capacity(strips.len()),
        rows: Vec::with_capacity(strips.len()),
        owners: Vec::with_capacity(strips.len()),
        patches: Vec::new(),
    };
    // The look of each edge from its strip here (the patches' arms).
    let mut looks: HashMap<usize, ArmLook> = HashMap::new();
    for (i, (s, ei)) in strips.iter().zip(&matched).enumerate() {
        let Some(ei) = *ei else {
            out.strips.push(s.clone());
            out.rows.push(None);
            out.owners.push(Owner::Strip(i));
            continue;
        };
        looks.entry(ei).or_insert_with(|| ArmLook {
            reversed: ocw_world::junction::strip_reversed(&g.edges[ei], &s.points),
            class: s.class,
            surface: s.surface,
            dress: s.dress.clone(),
            condition: s.condition,
        });
        match pj.set.shape_strip(g, ei, &s.points, s.width_m / 2.0) {
            None => {
                out.strips.push(s.clone());
                out.rows.push(None);
            }
            Some(sh) if sh.points.len() < 2 => continue,
            Some(sh) => {
                let mut t = s.clone();
                t.points = sh.points;
                t.dress.start_m += sh.cut_m[0];
                for (k, end) in sh.junction_end.iter().enumerate() {
                    if *end {
                        t.dress.junction_m[k] = 0.0;
                    }
                }
                move_crossings(&mut t, sh.cut_m[0], sh.junction_end, style);
                out.strips.push(t);
                out.rows.push(Some(sh.rows));
            }
        }
        out.owners.push(Owner::Edge(ei));
    }
    // The patches of the vertices in this tile.
    let mut seen = std::collections::HashSet::new();
    for &ei in edges {
        let e = &g.edges[ei as usize];
        for v in [e.from.0 as usize, e.to.0 as usize] {
            if !seen.insert(v) || TileKey::containing(g.vertices[v], size) != key {
                continue;
            }
            if let Some(p) = pj.set.patch(g, v) {
                let arm_looks = p
                    .arms
                    .iter()
                    .map(|a| {
                        looks.get(&a.edge).cloned().unwrap_or_else(|| ArmLook {
                            reversed: false,
                            class: a.class,
                            surface: RoadSurface::of_class(a.class),
                            dress: RoadDress::of_class(a.class, (2.0 * a.half) as f32),
                            condition: ocw_world::RoadCondition::UNKNOWN,
                        })
                    })
                    .collect();
                out.patches.push((p, arm_looks));
            }
        }
    }
    out.patches.sort_by_key(|(p, _)| p.vertex);
    out
}

/// The road meshes of a shaped tile (all strips together, as the renderer
/// draws them, plus the patches).
pub fn shaped_meshes(t: &ShapedTile<'_>, ctx: &RoadContext<'_>) -> RoadMeshes {
    let rows: Vec<Option<&crate::road_mesh::Rows>> = t.rows.iter().map(|r| r.as_deref()).collect();
    let mut out = road_meshes_rows(&t.strips, &rows, ctx);
    for (p, looks) in &t.patches {
        patch_mesh(&mut out.roads, p, looks, ctx);
    }
    out
}

fn v3(p: [f64; 3]) -> Vec3 {
    Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32)
}

/// Pushes one road vertex (the road mesh format, `road_mesh`).
fn vertex(m: &mut MeshArrays, p: Vec3, colour: [f32; 3], half: f32, uv: [f32; 2], b: [f32; 2]) {
    m.positions.push(p.to_array());
    m.normals.push([0.0, 1.0, 0.0]);
    m.colors.push([colour[0], colour[1], colour[2], half]);
    m.uvs.push(uv);
    m.uvs_b.push(b);
}

/// A patch: its surface (two copies of the vertices where a through road's
/// edge line breaks on one side only) and its outline's dress.
pub fn patch_mesh(m: &mut MeshArrays, p: &Patch, looks: &[ArmLook], ctx: &RoadContext<'_>) {
    let lift = Vec3::Y * crate::ROAD_SURFACE_LIFT_M;
    let main = p.main_arm();
    let look = &looks[main];
    let half = p.arms[main].half as f32;
    let layer = look.surface.code();
    let far = ctx.colour(layer, {
        let c = crate::package::road_colour(look.class, look.surface);
        [c[0], c[1], c[2]]
    });
    let code = MarkCode::of(&look.dress, look.class, look.surface, &look.condition);
    // With lines: a through road's (its centre line runs on); without: the
    // side where other roads leave (no edge line, no cycle lane line).
    let plain = code.plain_extra_code();
    let (on, off) = if p.through.is_some() {
        let mut open = code;
        open.edge_lines = false;
        open.cycle = [CycleLane::None; 2];
        (
            (code.lines_code(), code.extra_code()),
            (open.lines_code(), open.extra_code()),
        )
    } else {
        ((0, plain), (0, plain))
    };
    let base = m.positions.len() as u32;
    let uvs: Vec<[f64; 2]> = p.vertices.iter().map(|q| p.uv([q[0], q[2]])).collect();
    for (lines, extra) in [on, off] {
        for (q, uv) in p.vertices.iter().zip(&uvs) {
            vertex(
                m,
                v3(*q) + lift,
                far,
                half,
                [uv[0] as f32, uv[1] as f32],
                [code_b(layer, Kind::Surface, extra), lines as f32],
            );
        }
    }
    let nv = p.vertices.len() as u32;
    let open = p.open_sides();
    let band = f64::from(half) - 1.2;
    for t in &p.triangles {
        let reaches_open = t.iter().any(|&i| {
            let u = uvs[i as usize][0];
            (u < -band && open[0]) || (u > band && open[1])
        });
        let copy = if reaches_open { nv } else { 0 };
        m.indices.extend(t.map(|i| base + copy + i));
    }
    // The outline between the mouths.
    for c in &p.chains {
        let n = c.points.len();
        if n < 2 {
            continue;
        }
        let edge: Vec<Vec3> = c.points.iter().map(|q| v3(*q) + lift).collect();
        let (ai, aj) = (&p.arms[c.from_arm], &p.arms[c.to_arm]);
        let horiz = |a: Vec3, b: Vec3| (b - a).with_y(0.0).normalize_or_zero();
        let mouth_out = |arm: &ocw_world::junction::Arm, q: Vec3| {
            horiz(
                Vec3::new(arm.centre[0] as f32, q.y, arm.centre[1] as f32),
                q,
            )
        };
        let out: Vec<Vec3> = (0..n)
            .map(|k| {
                if k == 0 {
                    return mouth_out(ai, edge[0]);
                }
                if k == n - 1 {
                    return mouth_out(aj, edge[n - 1]);
                }
                let (d0, d1) = (horiz(edge[k - 1], edge[k]), horiz(edge[k], edge[k + 1]));
                let d = (d0 + d1).normalize_or_zero();
                Vec3::new(d.z, 0.0, -d.x)
            })
            .collect();
        let mut along = vec![0.0f32; n];
        for k in 1..n {
            along[k] = along[k - 1] + (edge[k] - edge[k - 1]).length();
        }
        let total = along[n - 1].max(1e-3);
        // Arm i's side facing the corner (its + side), arm j's (its −).
        let side_of = |arm: &ocw_world::junction::Arm, look: &ArmLook, plus: bool| -> usize {
            usize::from((plus == arm.at_from) != look.reversed)
        };
        let (li, lj) = (&looks[c.from_arm], &looks[c.to_arm]);
        let (si, sj) = (side_of(ai, li, true), side_of(aj, lj, false));
        let (wi, wj) = (li.dress.sidewalk_m[si], lj.dress.sidewalk_m[sj]);
        let kerbs: Vec<(f32, f32)> = along
            .iter()
            .map(|a| {
                let f = a / total;
                let w = wi + (wj - wi) * f;
                let ki = if wi > 0.0 { li.dress.kerb_m } else { 0.0 };
                let kj = if wj > 0.0 { lj.dress.kerb_m } else { 0.0 };
                (w, ki + (kj - ki) * f)
            })
            .collect();
        let strip = RoadStrip {
            class: li.class,
            width_m: 2.0 * half,
            points: Vec::new(),
            surface: li.surface,
            dress: RoadDress {
                sidewalk_asphalt: li.dress.sidewalk_asphalt,
                ..li.dress.clone()
            },
            lit: ocw_world::RoadLit::Unknown,
            condition: li.condition,
        };
        let dress_look = DressLook::of(&strip, ctx);
        let flags = vec![false; n];
        side_dress(
            m,
            &SideDress {
                side: -1.0,
                half,
                sidewalk: wi > 0.0 || wj > 0.0,
                edge: &edge,
                out: &out,
                along: &along,
                kerbs: &kerbs,
                decks: &flags,
                narrow: &flags,
            },
            &dress_look,
            ctx,
        );
    }
}

/// A tile's terrain grid: `n × n` vertices from `origin`, `spacing` apart
/// (row-major along +z, as `package::terrain_arrays_kept` lays them out).
#[derive(Debug, Clone, Copy)]
pub struct TerrainGrid {
    pub origin: [f64; 2],
    pub spacing: f64,
    pub n: usize,
}

/// Lowers the terrain under the junction surfaces (J1; J2 settles it in
/// the package): under a patch to [`TERRAIN_UNDER_M`] under its drawn
/// surface (its corners reach beyond the ground the world builder settled
/// under the strips, §4.8.3), and where a patch or a side road's blend
/// lies below the road's own profile, by as much as the road was lowered
/// – fully within a grid spacing of the road's edge, fading out over the
/// next (the builder settled the ground to the old profile; a road lowered
/// by a metre would lie under it).
pub fn lower_terrain(
    terrain: &mut MeshArrays,
    g: &RoadGraph,
    patches: &[&Patch],
    grid: TerrainGrid,
) {
    if patches.is_empty() || terrain.positions.len() != grid.n * grid.n {
        return;
    }
    let orig: Vec<f32> = terrain.positions.iter().map(|q| q[1]).collect();
    let sp = grid.spacing.max(1e-3);
    for patch in patches {
        for (ai, arm) in patch.arms.iter().enumerate() {
            let reach = arm.half + 2.0 * sp;
            for (c, delta) in JunctionSet::sunk(g, patch, ai) {
                let lo = |x: f64, o: f64| (((x - reach - o) / sp).floor().max(0.0)) as usize;
                let hi = |x: f64, o: f64| {
                    ((((x + reach - o) / sp).ceil()).max(0.0) as usize).min(grid.n - 1)
                };
                for k in lo(c[1], grid.origin[1])..=hi(c[1], grid.origin[1]) {
                    for i in lo(c[0], grid.origin[0])..=hi(c[0], grid.origin[0]) {
                        let idx = k * grid.n + i;
                        let q = &mut terrain.positions[idx];
                        let d = (f64::from(q[0]) - c[0]).hypot(f64::from(q[2]) - c[1]);
                        let w = ((arm.half + 2.0 * sp - d) / sp).clamp(0.0, 1.0);
                        if w > 0.0 {
                            q[1] = q[1].min(orig[idx] + (delta * w) as f32);
                        }
                    }
                }
            }
        }
    }
    for q in &mut terrain.positions {
        let p = [f64::from(q[0]), f64::from(q[2])];
        for patch in patches {
            if p[0] < patch.min[0] - 1.0
                || p[1] < patch.min[1] - 1.0
                || p[0] > patch.max[0] + 1.0
                || p[1] > patch.max[1] + 1.0
            {
                continue;
            }
            if let Some(h) = patch.height_at(p) {
                let top = h as f32 + crate::ROAD_SURFACE_LIFT_M - TERRAIN_UNDER_M;
                q[1] = q[1].min(top);
            }
        }
    }
}

/// The patches whose surface may reach into tile `key` (its edges' end
/// vertices).
pub fn patches_near<'a>(g: &RoadGraph, pj: &'a PackageJunctions, key: TileKey) -> Vec<&'a Patch> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for &ei in pj.tile_edges(key) {
        let e = &g.edges[ei as usize];
        for v in [e.from.0 as usize, e.to.0 as usize] {
            if seen.insert(v)
                && let Some(p) = pj.set.patch(g, v)
            {
                out.push(p);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests;
