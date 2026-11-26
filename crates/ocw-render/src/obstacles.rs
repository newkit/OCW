//! The obstacles of a world package drawn (spec §14 Phase 12 K3, Q7):
//! fences, walls, hedges and guard rails beside the roads, posts, pylons
//! and the point barriers off the road – what riders in wild mode can hit
//! – always; the **road barriers** (gates, bollards, lift gates, blocks,
//! cycle barriers on a road or within 2 m of it, section `OBST` flag
//! `ROAD_BARRIER`) only with the setting `display.road_barriers` (console
//! `road_barriers on|off`, default off): they never block the routed road.
//!
//! The point obstacles are model files (`artwork/3dmodels/props/`, spec
//! §14 Phase 15, 15H MX: [`model_id_of`]), fitted to each one's height and
//! width and set on the ground ([`ocw_assets::props::place`]); the line
//! obstacles stay generated from the world data (15E). Both are merged in
//! the vertex colours per 125 m chunk of a tile, drawn up to [`DRAW_M`]
//! from the camera (`VisibilityRange`, the cheap level of detail). Tunnel walls and bridge railings have their
//! own meshes (`tunnel`, `bridges`); kerbs are part of the road dress.

use std::sync::Arc;

use bevy::camera::visibility::VisibilityRange;
use bevy::prelude::*;
use clap::{Arg, Command};
use ocw_assets::model::Model;
use ocw_assets::props::{Instance, Level, Placed, has_lod1, place_level};
use ocw_world::obstacles::{LineObstacle, ObstacleKind, PointObstacle};
use ocw_world::{TileKey, WorldPackage};

use crate::console::{ConsoleAppExt, ConsoleCommand, Permission};
use crate::package::{Keep, MeshArrays, TileGround};

/// Obstacles are drawn this far from the camera (m).
pub const DRAW_M: f32 = 350.0;
/// The chunks of a tile per side (125 m for 1 km tiles).
const CHUNKS: i32 = 8;

/// `display.road_barriers`: draw the road barriers.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ObstacleSettings {
    pub road_barriers: bool,
}

/// A chunk's mesh of road barriers (shown with `display.road_barriers`).
#[derive(Component)]
pub struct RoadBarrierMesh;

/// One chunk of a tile's obstacles.
#[derive(Default)]
pub struct ObstacleChunk {
    /// Where the chunk's mesh is placed (its first vertex, or the middle
    /// of a [`LodCells`] cell's props; the level of detail is measured
    /// from there).
    pub origin: [f32; 3],
    pub mesh: MeshArrays,
    /// Road barriers (toggled by the setting).
    pub road: bool,
    /// Which level of its props the mesh is (`None`: props without a
    /// `lod1`, drawn the same at any distance).
    pub level: Option<Level>,
}

impl ObstacleChunk {
    /// Its visibility range: up to [`DRAW_M`]; the near level of props
    /// with a `lod1` up to [`crate::lod::PROP`]'s switch (60 m), the far
    /// level from there (spec 16H Q24 (a)).
    pub fn range(&self) -> VisibilityRange {
        match self.level {
            None => draw_range(),
            Some(Level::Near) => crate::lod::PROP.near_range(),
            Some(Level::Lod1) => crate::lod::PROP.far_range(Some(draw_range().end_margin)),
        }
    }
}

/// Props with a far level (`lod1`) are merged per cell of this size (m),
/// a near and a far mesh each, so the switch at 60 m happens within half
/// a cell's diagonal (at most 11 m, about the fade) of each prop: the
/// cell's origin is the middle of its props' feet.
pub const LOD_CELL_M: f32 = 16.0;

/// The props with a `lod1` of a tile, by [`LOD_CELL_M`] cell: two layers
/// of meshes per level (the obstacles: the rest and the road barriers;
/// the street lamps: poles and glowing heads).
#[derive(Default)]
pub struct LodCells {
    cells: std::collections::BTreeMap<(i32, i32), LodCellSum>,
}

#[derive(Default)]
struct LodCellSum {
    feet: Vec3,
    n: f32,
    near: [MeshArrays; 2],
    far: [MeshArrays; 2],
}

/// One cell of [`LodCells`], its meshes relative to `origin`.
#[derive(Default)]
pub struct LodCell {
    pub origin: [f32; 3],
    pub near: [MeshArrays; 2],
    pub far: [MeshArrays; 2],
}

impl LodCells {
    /// The near and far meshes of the cell of a prop standing at `foot`.
    pub fn at(&mut self, foot: Vec3) -> (&mut [MeshArrays; 2], &mut [MeshArrays; 2]) {
        let key = (
            (foot.x / LOD_CELL_M).floor() as i32,
            (foot.z / LOD_CELL_M).floor() as i32,
        );
        let c = self.cells.entry(key).or_default();
        c.feet += foot;
        c.n += 1.0;
        (&mut c.near, &mut c.far)
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// The cells, each moved to the middle of its props' feet.
    pub fn finish(self) -> Vec<LodCell> {
        self.cells
            .into_values()
            .map(|c| {
                let o = c.feet / c.n;
                let shift = |mut m: MeshArrays| {
                    for p in &mut m.positions {
                        *p = [p[0] - o.x, p[1] - o.y, p[2] - o.z];
                    }
                    m
                };
                let [a, b] = c.near;
                let [d, e] = c.far;
                LodCell {
                    origin: o.to_array(),
                    near: [shift(a), shift(b)],
                    far: [shift(d), shift(e)],
                }
            })
            .collect()
    }
}

/// The obstacle meshes of a tile.
#[derive(Default)]
pub struct TileObstacleMeshes {
    pub chunks: Vec<ObstacleChunk>,
}

impl TileObstacleMeshes {
    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    /// Triangles of the road barriers and of the rest.
    pub fn triangles(&self) -> (usize, usize) {
        self.chunks.iter().fold((0, 0), |(r, o), c| {
            let n = c.mesh.indices.len() / 3;
            if c.road { (r + n, o) } else { (r, o + n) }
        })
    }
}

fn lin(r: f32, g: f32, b: f32) -> [f32; 4] {
    let c = Color::srgb(r, g, b).to_linear();
    [c.red, c.green, c.blue, 1.0]
}

/// Colours of the line obstacles (the point obstacles are model files).
struct Palette {
    steel: [f32; 4],
    wood: [f32; 4],
    stone: [f32; 4],
    hedge: [f32; 4],
}

impl Palette {
    fn new() -> Self {
        Self {
            steel: lin(0.66, 0.68, 0.70),
            wood: lin(0.45, 0.33, 0.22),
            stone: lin(0.55, 0.52, 0.47),
            hedge: lin(0.17, 0.32, 0.13),
        }
    }
}

/// Adds a box along `a` → `b` (feet on the ground at both ends): `half`
/// its half thickness, from `lo` to `hi` above the feet.
fn beam(
    m: &mut MeshArrays,
    a: [f32; 3],
    b: [f32; 3],
    half: f32,
    lo: f32,
    hi: f32,
    colour: [f32; 4],
) {
    let (dx, dz) = (b[0] - a[0], b[2] - a[2]);
    let len = dx.hypot(dz);
    if len < 1e-4 {
        return;
    }
    let (fx, fz) = (dx / len, dz / len);
    let (rx, rz) = (-fz * half, fx * half);
    let p = |q: [f32; 3], s: f32, y: f32| [q[0] + rx * s, q[1] + y, q[2] + rz * s];
    let mut quad = |v: [[f32; 3]; 4], n: [f32; 3]| {
        let base = m.positions.len() as u32;
        for p in v {
            m.positions.push(p);
            m.normals.push(n);
            m.colors.push(colour);
        }
        m.indices
            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    };
    let (r, l) = ([-fz, 0.0, fx], [fz, 0.0, -fx]);
    quad(
        [p(a, 1.0, lo), p(b, 1.0, lo), p(b, 1.0, hi), p(a, 1.0, hi)],
        r,
    );
    quad(
        [
            p(b, -1.0, lo),
            p(a, -1.0, lo),
            p(a, -1.0, hi),
            p(b, -1.0, hi),
        ],
        l,
    );
    quad(
        [p(a, 1.0, hi), p(b, 1.0, hi), p(b, -1.0, hi), p(a, -1.0, hi)],
        [0.0, 1.0, 0.0],
    );
    quad(
        [p(a, -1.0, lo), p(a, 1.0, lo), p(a, 1.0, hi), p(a, -1.0, hi)],
        [-fx, 0.0, -fz],
    );
    quad(
        [p(b, 1.0, lo), p(b, -1.0, lo), p(b, -1.0, hi), p(b, 1.0, hi)],
        [fx, 0.0, fz],
    );
}

/// Adds an upright prism of `sides` sides around `c` (radius `r` at the
/// foot, `top` at the top) from `lo` to `hi` above it.
#[allow(clippy::too_many_arguments)]
fn prism(
    m: &mut MeshArrays,
    c: [f32; 3],
    r: f32,
    top: f32,
    lo: f32,
    hi: f32,
    sides: u32,
    colour: [f32; 4],
) {
    for j in 0..sides {
        let (a0, a1) = (
            j as f32 / sides as f32 * std::f32::consts::TAU,
            (j + 1) as f32 / sides as f32 * std::f32::consts::TAU,
        );
        let at = |a: f32, rr: f32, y: f32| [c[0] + rr * a.cos(), c[1] + y, c[2] + rr * a.sin()];
        let mid = (a0 + a1) / 2.0;
        let n = [mid.cos(), (r - top) / (hi - lo).max(0.01), mid.sin()];
        let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        let n = [n[0] / l, n[1] / l, n[2] / l];
        let base = m.positions.len() as u32;
        for p in [
            at(a0, r, lo),
            at(a1, r, lo),
            at(a1, top, hi),
            at(a0, top, hi),
        ] {
            m.positions.push(p);
            m.normals.push(n);
            m.colors.push(colour);
        }
        m.indices
            .extend([base, base + 2, base + 1, base, base + 3, base + 2]);
        // The cap.
        let cap = m.positions.len() as u32;
        for p in [[c[0], c[1] + hi, c[2]], at(a0, top, hi), at(a1, top, hi)] {
            m.positions.push(p);
            m.normals.push([0.0, 1.0, 0.0]);
            m.colors.push(colour);
        }
        m.indices.extend([cap, cap + 2, cap + 1]);
    }
}

/// The ground under a point: the road surface on a road barrier, else the
/// terrain as the tile draws it (`ground`: incl. the junction lowering,
/// spec §4.6.1; the road's height where the terrain is missing).
fn foot(pkg: &WorldPackage, ground: &TileGround, x: f32, z: f32, on_road: bool) -> f32 {
    let (xf, zf) = (f64::from(x), f64::from(z));
    let road = pkg
        .road_heights(xf, zf)
        .into_iter()
        .fold(None::<f64>, |m, h| Some(m.map_or(h, |m| m.max(h))));
    match (on_road, road) {
        (true, Some(r)) => (r + ocw_world::package::ROAD_LIFT_M) as f32,
        _ => ground.height(xf, zf).or(road).map_or(0.0, |h| h as f32),
    }
}

/// The width a gate spans without one in the package (m).
pub const DEFAULT_SPAN_M: f32 = 3.0;

/// The turn (`Quat::from_rotation_y`) of a point obstacle's model: model
/// X across the way, model −Z along it (`angle`: the way's direction).
pub fn yaw_of(angle: f32) -> f32 {
    -(angle + std::f32::consts::FRAC_PI_2)
}

/// Appends a placed model's parts whose slot `keep` accepts.
pub(crate) fn append(m: &mut MeshArrays, placed: &Placed, keep: impl Fn(&str) -> bool) {
    for part in placed.parts.iter().filter(|p| keep(&p.slot)) {
        let base = m.positions.len() as u32;
        m.positions.extend(&part.positions);
        m.normals.extend(&part.normals);
        m.colors.extend(&part.colors);
        m.indices.extend(part.indices.iter().map(|i| i + base));
    }
}

/// A point obstacle's mesh: its kind's model file ([`model_id_of`])
/// fitted to its height and the width it spans, turned with its way and
/// set on `ground` (spec §14 Phase 15, 15H MX; `ocw_assets::props`).
pub fn point_mesh(
    m: &mut MeshArrays,
    p: &PointObstacle,
    model: &Model,
    ground: &dyn Fn(f32, f32) -> f32,
) {
    point_mesh_level(m, p, model, ground, Level::Near);
}

/// [`point_mesh`] of the file's `level`.
pub fn point_mesh_level(
    m: &mut MeshArrays,
    p: &PointObstacle,
    model: &Model,
    ground: &dyn Fn(f32, f32) -> f32,
    level: Level,
) {
    let span = if p.width_m > 0.5 {
        p.width_m
    } else {
        DEFAULT_SPAN_M
    };
    let inst = Instance {
        height_m: Some(p.height_m.max(0.2)),
        span_m: Some(span),
        reach_m: None,
    };
    let foot = Vec3::new(p.pos[0], ground(p.pos[0], p.pos[1]), p.pos[1]);
    append(
        m,
        &place_level(model, &inst, foot, yaw_of(p.angle), ground, level),
        |_| true,
    );
}

/// A flat disc of `sides` around `c` in the plane of `right` and up,
/// its face looking along `normal`.
fn disc(
    m: &mut MeshArrays,
    c: [f32; 3],
    right: [f32; 3],
    normal: [f32; 3],
    r: f32,
    sides: u32,
    colour: [f32; 4],
) {
    let base = m.positions.len() as u32;
    m.positions.push(c);
    m.normals.push(normal);
    m.colors.push(colour);
    for j in 0..sides {
        let a = j as f32 / sides as f32 * std::f32::consts::TAU;
        let (x, y) = (a.cos() * r, a.sin() * r);
        m.positions
            .push([c[0] + right[0] * x, c[1] + y, c[2] + right[2] * x]);
        m.normals.push(normal);
        m.colors.push(colour);
    }
    // Counter-clockwise seen from the front (`normal`).
    for j in 0..sides {
        let (a, b) = (base + 1 + j, base + 1 + (j + 1) % sides);
        if (right[2] * normal[0] - right[0] * normal[2]) > 0.0 {
            m.indices.extend([base, b, a]);
        } else {
            m.indices.extend([base, a, b]);
        }
    }
}

/// The plate of the "no bicycles" sign (m): radius, its red ring.
pub const SIGN_PLATE_R: f32 = 0.32;
const SIGN_RING_R: f32 = 0.26;

/// The warning sign at an end of a road closed for bikes (16H Q6;
/// `ocw_world::bike_signs`): a grey pole and a round plate – a red ring
/// round a white face with a black bicycle, the European "no bicycles"
/// sign – facing the rider who comes onto the road (`p.angle`: the
/// direction its face looks), its back plain grey.
pub fn no_bike_sign(m: &mut MeshArrays, p: &PointObstacle, foot: f32) {
    let palette = (
        lin(0.62, 0.64, 0.66),
        lin(0.80, 0.08, 0.06),
        lin(0.96, 0.96, 0.94),
        lin(0.05, 0.05, 0.05),
    );
    let (grey, red, white, black) = palette;
    let top = p.height_m.max(1.5);
    let c = [p.pos[0], foot, p.pos[1]];
    prism(m, c, 0.035, 0.035, 0.0, top - 0.05, 8, grey);
    let (fx, fz) = (p.angle.cos(), p.angle.sin());
    let normal = [fx, 0.0, fz];
    let back = [-fx, 0.0, -fz];
    // Right as seen from the front.
    let right = [fz, 0.0, -fx];
    let mid = top - SIGN_PLATE_R;
    let at = |d: f32, x: f32, y: f32| {
        [
            c[0] + fx * d + right[0] * x,
            foot + mid + y,
            c[2] + fz * d + right[2] * x,
        ]
    };
    disc(m, at(0.045, 0.0, 0.0), right, normal, SIGN_PLATE_R, 24, red);
    disc(m, at(0.05, 0.0, 0.0), right, normal, SIGN_RING_R, 24, white);
    disc(m, at(0.04, 0.0, 0.0), right, back, SIGN_PLATE_R, 24, grey);
    // The bicycle: two wheels and the frame between them.
    for x in [-0.1_f32, 0.1] {
        disc(m, at(0.055, x, -0.04), right, normal, 0.065, 12, black);
        disc(m, at(0.056, x, -0.04), right, normal, 0.045, 12, white);
    }
    let bar = |m: &mut MeshArrays, a: (f32, f32), b: (f32, f32)| {
        let (pa, pb) = (at(0.057, a.0, a.1), at(0.057, b.0, b.1));
        // A thin quad in the plate's plane.
        let (dx, dy, dz) = (pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]);
        let len = (dx * dx + dy * dy + dz * dz).sqrt().max(1e-4);
        // Perpendicular in the plane: (along right, up) rotated by 90°.
        let along_r = (dx * right[0] + dz * right[2]) / len;
        let along_y = dy / len;
        let w = 0.012;
        let off = |q: [f32; 3], s: f32| {
            [
                q[0] - right[0] * along_y * w * s,
                q[1] + along_r * w * s,
                q[2] - right[2] * along_y * w * s,
            ]
        };
        let base = m.positions.len() as u32;
        for q in [off(pa, 1.0), off(pb, 1.0), off(pb, -1.0), off(pa, -1.0)] {
            m.positions.push(q);
            m.normals.push(normal);
            m.colors.push(black);
        }
        m.indices
            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        m.indices
            .extend([base, base + 2, base + 1, base, base + 3, base + 2]);
    };
    bar(m, (-0.1, -0.04), (-0.02, 0.05));
    bar(m, (-0.02, 0.05), (0.07, 0.05));
    bar(m, (0.07, 0.05), (0.1, -0.04));
    bar(m, (-0.1, -0.04), (0.0, -0.04));
    bar(m, (0.0, -0.04), (0.07, 0.05));
    bar(m, (-0.02, 0.05), (-0.04, 0.09));
    bar(m, (0.07, 0.05), (0.06, 0.1));
}

/// The model file of a point obstacle kind (spec §14 Phase 15, 15H names;
/// kinds without their own file use the generic barrier post).
pub fn model_id_of(kind: ObstacleKind) -> &'static str {
    kind.model_id().unwrap_or("props/barrier-post-hexagonal")
}

/// A line obstacle's mesh (per segment, feet on the terrain).
fn line_mesh(
    m: &mut MeshArrays,
    pkg: &WorldPackage,
    ground: &TileGround,
    l: &LineObstacle,
    c: &Palette,
) {
    let on_ground = |p: [f32; 2]| [p[0], foot(pkg, ground, p[0], p[1], false), p[1]];
    let h = l.height_m.max(0.3);
    for w in l.points.windows(2) {
        let (a, b) = (on_ground(w[0]), on_ground(w[1]));
        let len = (b[0] - a[0]).hypot(b[2] - a[2]);
        match l.kind {
            ObstacleKind::Wall => beam(m, a, b, 0.2, -0.3, h, c.stone),
            ObstacleKind::Hedge => beam(m, a, b, 0.5, -0.2, h, c.hedge),
            ObstacleKind::Fence | ObstacleKind::GateLine => {
                let posts = (len / 2.5).ceil().max(1.0) as usize;
                for k in 0..=posts {
                    let t = k as f32 / posts as f32;
                    let q = on_ground([
                        w[0][0] + (w[1][0] - w[0][0]) * t,
                        w[0][1] + (w[1][1] - w[0][1]) * t,
                    ]);
                    prism(m, q, 0.05, 0.05, -0.2, h, 4, c.wood);
                }
                for k in [0.45, 0.9] {
                    beam(m, a, b, 0.025, h * k - 0.04, h * k + 0.04, c.wood);
                }
            }
            ObstacleKind::GuardRail => {
                let posts = (len / 4.0).ceil().max(1.0) as usize;
                for k in 0..=posts {
                    let t = k as f32 / posts as f32;
                    let q = on_ground([
                        w[0][0] + (w[1][0] - w[0][0]) * t,
                        w[0][1] + (w[1][1] - w[0][1]) * t,
                    ]);
                    prism(m, q, 0.06, 0.06, -0.2, h - 0.05, 4, c.steel);
                }
                beam(m, a, b, 0.05, h - 0.33, h, c.steel);
            }
            // Kerbs are drawn with the road dress.
            _ => {}
        }
    }
}

/// The obstacle meshes of tile `key`: per chunk the road barriers and the
/// rest, only what `keep` accepts (by the first point).
pub fn tile_obstacle_meshes(
    pkg: &WorldPackage,
    key: TileKey,
    keep: Keep,
    ground: &TileGround,
) -> TileObstacleMeshes {
    let shipped = |k: ObstacleKind| ocw_assets::shipped::model(model_id_of(k));
    tile_obstacle_meshes_from(pkg, key, keep, ground, &shipped)
}

/// [`tile_obstacle_meshes`] with each kind's file from `model_of`: the
/// props of a file with a `lod1` are drawn near and far by
/// [`LodCells`] cell, the others in their chunk.
/// Pylons of one power line stand this far apart at most (m).
pub const PYLON_SPAN_M: f32 = 600.0;

/// The direction of the power line through the pylon at `at` (rad, modulo
/// π; the way-angle convention of [`PointObstacle::angle`]): towards its
/// nearest neighbour among the package's pylons within [`PYLON_SPAN_M`],
/// averaged with the one on the other side if there is one; `None`
/// without a neighbour (the road's direction stays).
pub fn pylon_line_angle(pkg: &WorldPackage, at: [f32; 2]) -> Option<f32> {
    let o = pkg.obstacles.as_ref()?;
    let size = pkg.tile_size_m;
    let key = TileKey::containing([f64::from(at[0]), f64::from(at[1])], f64::from(size));
    let mut near: Vec<(f32, [f32; 2])> = Vec::new();
    for dz in -1..=1 {
        for dx in -1..=1 {
            let k = TileKey {
                x: key.x + dx,
                z: key.z + dz,
            };
            let Some(t) = o.tiles.get(&k) else { continue };
            for q in t.points.iter().filter(|q| q.kind == ObstacleKind::Pylon) {
                let d = [q.pos[0] - at[0], q.pos[1] - at[1]];
                let len = d[0].hypot(d[1]);
                if len > 15.0 && len <= PYLON_SPAN_M {
                    near.push((len, [d[0] / len, d[1] / len]));
                }
            }
        }
    }
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    let first = near.first()?.1;
    // The nearest one on the other side (more than 120° away).
    let dir = match near
        .iter()
        .find(|n| n.1[0] * first[0] + n.1[1] * first[1] < -0.5)
    {
        Some(other) => [first[0] - other.1[0], first[1] - other.1[1]],
        None => first,
    };
    Some(dir[1].atan2(dir[0]).rem_euclid(std::f32::consts::PI))
}

pub fn tile_obstacle_meshes_from(
    pkg: &WorldPackage,
    key: TileKey,
    keep: Keep,
    ground: &TileGround,
    model_of: &dyn Fn(ObstacleKind) -> Arc<Model>,
) -> TileObstacleMeshes {
    let Some(t) = pkg.obstacles.as_ref().and_then(|o| o.tiles.get(&key)) else {
        return TileObstacleMeshes::default();
    };
    let size = pkg.tile_size_m;
    let o = key.origin(f64::from(size));
    let chunk_of = |p: [f32; 2]| {
        let c = size / CHUNKS as f32;
        let i = (((p[0] - o[0] as f32) / c).floor() as i32).clamp(0, CHUNKS - 1);
        let k = (((p[1] - o[1] as f32) / c).floor() as i32).clamp(0, CHUNKS - 1);
        (k * CHUNKS + i) as usize
    };
    let kept = |p: [f32; 2]| keep.is_none_or(|f| f([f64::from(p[0]), f64::from(p[1])]));
    let colours = Palette::new();
    let mut cells = LodCells::default();
    // The kinds' files, read once per tile.
    let mut models: Vec<(ObstacleKind, Arc<Model>)> = Vec::new();
    let n = (CHUNKS * CHUNKS) as usize;
    let (mut road, mut rest): (Vec<MeshArrays>, Vec<MeshArrays>) = (
        (0..n).map(|_| MeshArrays::default()).collect(),
        (0..n).map(|_| MeshArrays::default()).collect(),
    );
    for p in t
        .points
        .iter()
        // Planted trees are drawn by the vegetation, the mapped street
        // lamps beside their road by `street_lamps`.
        .filter(|p| {
            kept(p.pos) && !matches!(p.kind, ObstacleKind::PlantedTree | ObstacleKind::StreetLamp)
        })
    {
        let target = if p.road_barrier() {
            &mut road
        } else {
            &mut rest
        };
        let on_road = p.road_barrier();
        let at = |x: f32, z: f32| foot(pkg, ground, x, z, on_road);
        if p.kind == ObstacleKind::NoBikeSign {
            // Made from the world data (16H Q6), no model file.
            let foot = at(p.pos[0], p.pos[1]);
            no_bike_sign(&mut rest[chunk_of(p.pos)], p, foot);
            continue;
        }
        let model = match models.iter().find(|(k, _)| *k == p.kind) {
            Some((_, m)) => m.clone(),
            None => {
                let m = model_of(p.kind);
                models.push((p.kind, m.clone()));
                m
            }
        };
        // A pylon's arms reach across its power line (the direction to
        // the pylons beside it), not across the road it stands near.
        let line = (p.kind == ObstacleKind::Pylon)
            .then(|| pylon_line_angle(pkg, p.pos))
            .flatten();
        let turned;
        let p = match line {
            Some(a) => {
                turned = PointObstacle { angle: a, ..*p };
                &turned
            }
            None => p,
        };
        if has_lod1(&model) {
            // Near and far by cell (spec 16H Q24 (a): props at 60 m).
            let layer = usize::from(on_road);
            let foot = Vec3::new(p.pos[0], at(p.pos[0], p.pos[1]), p.pos[1]);
            let (near, far) = cells.at(foot);
            point_mesh_level(&mut near[layer], p, &model, &at, Level::Near);
            point_mesh_level(&mut far[layer], p, &model, &at, Level::Lod1);
        } else {
            point_mesh(&mut target[chunk_of(p.pos)], p, &model, &at);
        }
    }
    for l in t
        .lines
        .iter()
        .filter(|l| l.points.first().is_some_and(|p| kept(*p)))
    {
        line_mesh(&mut rest[chunk_of(l.points[0])], pkg, ground, l, &colours);
    }
    let mut chunks = Vec::new();
    for (mesh, is_road) in road
        .into_iter()
        .map(|m| (m, true))
        .chain(rest.into_iter().map(|m| (m, false)))
    {
        if !mesh.indices.is_empty() {
            let mut mesh = mesh;
            let origin = mesh.positions[0];
            for p in &mut mesh.positions {
                *p = [p[0] - origin[0], p[1] - origin[1], p[2] - origin[2]];
            }
            chunks.push(ObstacleChunk {
                origin,
                mesh,
                road: is_road,
                level: None,
            });
        }
    }
    for cell in cells.finish() {
        for (level, layers) in [(Level::Near, cell.near), (Level::Lod1, cell.far)] {
            for (layer, mesh) in layers.into_iter().enumerate() {
                if !mesh.indices.is_empty() {
                    chunks.push(ObstacleChunk {
                        origin: cell.origin,
                        mesh,
                        road: layer == 1,
                        level: Some(level),
                    });
                }
            }
        }
    }
    TileObstacleMeshes { chunks }
}

/// The level of detail: drawn up to [`DRAW_M`] from the chunk's origin.
pub fn draw_range() -> VisibilityRange {
    VisibilityRange {
        start_margin: 0.0..0.0,
        end_margin: DRAW_M - 30.0..DRAW_M,
        use_aabb: false,
    }
}

pub struct ObstaclesPlugin;

impl Plugin for ObstaclesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ObstacleSettings>()
            .add_systems(Update, show_road_barriers)
            .console_command(ConsoleCommand::new(
                Permission::User,
                Command::new("road_barriers")
                    .about("Gates, bollards and barriers on the roads: on or off (setting display.road_barriers; never solid)")
                    .arg(Arg::new("mode").value_parser(["on", "off"])),
                |world, m| {
                    if let Some(v) = m.get_one::<String>("mode") {
                        world.resource_mut::<ObstacleSettings>().road_barriers = v == "on";
                    }
                    let on = world.resource::<ObstacleSettings>().road_barriers;
                    let counts = world
                        .get_resource::<crate::package::PackageWorld>()
                        .and_then(|w| w.home().obstacles.as_ref().map(|o| o.counts()));
                    let road = world
                        .get_resource::<crate::package::PackageWorld>()
                        .and_then(|w| w.home().obstacles.as_ref())
                        .map(|o| {
                            o.tiles
                                .values()
                                .flat_map(|t| &t.points)
                                .filter(|p| p.road_barrier())
                                .count()
                        });
                    Ok(match (counts, road) {
                        (Some(_), Some(n)) => format!(
                            "road barriers {} ({n} in this package)",
                            if on { "shown" } else { "hidden" }
                        ),
                        _ => format!(
                            "road barriers {} (this package has no obstacles section)",
                            if on { "shown" } else { "hidden" }
                        ),
                    })
                },
            ));
    }
}

/// Shows or hides the road barriers by the setting.
fn show_road_barriers(
    settings: Res<ObstacleSettings>,
    mut q: Query<&mut Visibility, With<RoadBarrierMesh>>,
    added: Query<(), Added<RoadBarrierMesh>>,
) {
    if !settings.is_changed() && added.is_empty() {
        return;
    }
    let want = if settings.road_barriers {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for mut v in &mut q {
        if *v != want {
            *v = want;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 16H Q6: the warning sign stands on its foot, its plate's front
    /// looks where the sign faces (at the rider coming onto the road).
    #[test]
    fn the_no_bike_sign_faces_the_rider() {
        let mut m = MeshArrays::default();
        let p = PointObstacle {
            kind: ObstacleKind::NoBikeSign,
            flags: 0,
            pos: [10.0, 20.0],
            angle: std::f32::consts::FRAC_PI_2,
            width_m: 0.0,
            height_m: ocw_world::bike_signs::HEIGHT_M,
        };
        no_bike_sign(&mut m, &p, 400.0);
        assert!(!m.indices.is_empty());
        assert_eq!(m.positions.len(), m.normals.len());
        let (lo, hi) = m
            .positions
            .iter()
            .fold((f32::MAX, f32::MIN), |(a, b), q| (a.min(q[1]), b.max(q[1])));
        assert!(
            (lo - 400.0).abs() < 1e-3 && (hi - 402.3).abs() < 0.01,
            "{lo} {hi}"
        );
        // Everything within a plate's reach of the pole.
        assert!(
            m.positions
                .iter()
                .all(|q| (q[0] - 10.0).hypot(q[2] - 20.0) < 0.4)
        );
        // The red ring's face looks along +z (angle π/2), its triangles
        // turn counter-clockwise seen from there.
        let red = m
            .colors
            .iter()
            .position(|c| c[0] > 0.5 && c[1] < 0.1)
            .expect("red");
        assert!((m.normals[red][2] - 1.0).abs() < 1e-5);
        let tri = m
            .indices
            .chunks(3)
            .find(|t| t[0] as usize == red - 1 || t[0] as usize == red)
            .unwrap();
        let v = |i: u32| Vec3::from(m.positions[i as usize]);
        let n = (v(tri[1]) - v(tri[0])).cross(v(tri[2]) - v(tri[0]));
        assert!(n.z > 0.0, "front faces the rider: {n}");
    }
    use ocw_world::obstacles::{Obstacles, ROAD_BARRIER, TileObstacles};
    use std::collections::BTreeMap;

    fn package() -> WorldPackage {
        let key = TileKey { x: 0, z: 0 };
        let mut o = Obstacles::new(1000.0);
        let point = |kind, x: f32, flags| PointObstacle {
            kind,
            flags,
            pos: [x, 500.0],
            angle: 0.0,
            width_m: 6.0,
            height_m: kind.default_height_m() as f32,
        };
        o.tiles.insert(
            key,
            TileObstacles {
                points: vec![
                    point(ObstacleKind::Bollard, 100.0, ROAD_BARRIER),
                    point(ObstacleKind::Gate, 200.0, ROAD_BARRIER),
                    point(ObstacleKind::LiftGate, 300.0, ROAD_BARRIER),
                    point(ObstacleKind::CycleBarrier, 400.0, ROAD_BARRIER),
                    point(ObstacleKind::Block, 450.0, ROAD_BARRIER),
                    point(ObstacleKind::Post, 500.0, 0),
                ],
                lines: [
                    ObstacleKind::Fence,
                    ObstacleKind::Wall,
                    ObstacleKind::Hedge,
                    ObstacleKind::GuardRail,
                    ObstacleKind::Kerb,
                ]
                .into_iter()
                .enumerate()
                .map(|(i, kind)| LineObstacle {
                    kind,
                    flags: 0,
                    height_m: 1.2,
                    points: vec![
                        [600.0, 400.0 + i as f32 * 10.0],
                        [700.0, 400.0 + i as f32 * 10.0],
                    ],
                })
                .collect(),
                trees: Vec::new(),
            },
        );
        WorldPackage {
            name: "t".into(),
            origin: (47.0, 9.0),
            tile_size_m: 1000.0,
            samples: 11,
            tiles: BTreeMap::from([(
                key,
                ocw_world::Tile {
                    key,
                    heights: vec![10.0; 121],
                    roads: Vec::new(),
                },
            )]),
            graph: None,
            landuse: None,
            buildings: None,
            bridges: None,
            routes: Vec::new(),
            region: None,
            time_zone: None,
            look: Default::default(),
            obstacles: Some(o),
            places: None,
            junctions: None,
        }
    }

    #[test]
    fn road_barriers_and_the_rest_are_separate_chunks_on_the_ground() {
        let pkg = package();
        let ground = TileGround::new(&pkg, TileKey { x: 0, z: 0 }, None);
        let mut m = tile_obstacle_meshes(&pkg, TileKey { x: 0, z: 0 }, None, &ground);
        // Back to package metres (the meshes are relative to their origin).
        for c in &mut m.chunks {
            assert_eq!(c.mesh.positions[0], [0.0; 3]);
            let o = c.origin;
            for p in &mut c.mesh.positions {
                *p = [p[0] + o[0], p[1] + o[1], p[2] + o[2]];
            }
        }
        let (road, rest) = m.triangles();
        assert!(road > 100 && rest > 100, "{road} {rest}");
        assert!(m.chunks.iter().any(|c| c.road) && m.chunks.iter().any(|c| !c.road));
        for c in &m.chunks {
            assert_eq!(c.mesh.positions.len(), c.mesh.normals.len());
            assert_eq!(c.mesh.positions.len(), c.mesh.colors.len());
            for p in &c.mesh.positions {
                // On the terrain (10 m), below the tallest (the 7 m post).
                assert!(p[1] >= 9.6 && p[1] <= 17.1, "{p:?}");
            }
        }
        // The gate spans the 6 m road: posts 3 m either side of it.
        let gate = m
            .chunks
            .iter()
            .find(|c| c.road && c.mesh.positions.iter().any(|p| (p[0] - 200.0).abs() < 0.2))
            .unwrap();
        let zs: Vec<f32> = gate
            .mesh
            .positions
            .iter()
            .filter(|p| (p[0] - 200.0).abs() < 0.2)
            .map(|p| p[2])
            .collect();
        assert!(
            zs.iter().any(|z| (z - 503.0).abs() < 0.1)
                && zs.iter().any(|z| (z - 497.0).abs() < 0.1)
        );
        // Kept out: nothing of a tile the keep filter refuses.
        let none: &(dyn Fn([f64; 2]) -> bool + Sync) = &|_| false;
        assert!(tile_obstacle_meshes(&pkg, TileKey { x: 0, z: 0 }, Some(none), &ground).is_empty());
    }

    /// Props whose file has a `lod1` (spec 16H Q24 (a)): near and far
    /// meshes per cell, the near one drawn up to 60 m, the `lod1` from
    /// there to the obstacles' draw distance; props without stay in their
    /// chunk, drawn the same at any distance.
    #[test]
    fn props_with_a_lod1_are_drawn_with_it_beyond_60_m() {
        let pkg = package();
        let key = TileKey { x: 0, z: 0 };
        let ground = TileGround::new(&pkg, key, None);
        let red = |m: &ocw_assets::model::Material| ocw_assets::model::Material {
            name: "paint_red".into(),
            color: [1.0, 0.0, 0.0, 1.0],
            ..m.clone()
        };
        // Bollards with a far level, the other kinds as shipped.
        let model_of = |k: ObstacleKind| {
            let m = ocw_assets::shipped::model(model_id_of(k));
            if k == ObstacleKind::Bollard {
                Arc::new(ocw_assets::props::with_copied_lod1(&m, &red))
            } else {
                m
            }
        };
        let plain = tile_obstacle_meshes(&pkg, key, None, &ground);
        let m = tile_obstacle_meshes_from(&pkg, key, None, &ground, &model_of);
        let levels: Vec<_> = m.chunks.iter().filter_map(|c| c.level).collect();
        assert_eq!(levels, [Level::Near, Level::Lod1], "one bollard, one cell");
        let near = m
            .chunks
            .iter()
            .find(|c| c.level == Some(Level::Near))
            .unwrap();
        let far = m
            .chunks
            .iter()
            .find(|c| c.level == Some(Level::Lod1))
            .unwrap();
        assert!(near.road && far.road, "the bollard is a road barrier");
        assert_eq!(near.mesh.positions.len(), far.mesh.positions.len());
        assert!(
            far.mesh.colors.iter().all(|c| c[1] == 0.0 && c[0] > 0.0),
            "lod1 drawn"
        );
        assert!(
            near.mesh.colors.iter().any(|c| c[1] > 0.0),
            "the near model"
        );
        // Its cell's origin is its foot (one prop).
        assert!((near.origin[0] - 100.0).abs() < 1e-3 && (near.origin[2] - 500.0).abs() < 1e-3);
        // Up to 60 m near, beyond it lod1 (cross-faded over 54 – 66 m),
        // to the obstacles' draw distance.
        assert_eq!(near.range().end_margin, 54.0..66.0);
        assert_eq!(far.range().start_margin, 54.0..66.0);
        assert_eq!(far.range().end_margin, draw_range().end_margin);
        let (a, b) = (near.range(), far.range());
        assert!(a.end_margin.end <= b.start_margin.end && b.start_margin.start < 60.0);
        // The rest unchanged: as many triangles less as the bollard has.
        let tris = |c: &ObstacleChunk| c.mesh.indices.len() / 3;
        let lod_less: usize = m
            .chunks
            .iter()
            .filter(|c| c.level.is_none())
            .map(tris)
            .sum();
        let all: usize = plain.chunks.iter().map(tris).sum();
        assert_eq!(lod_less + tris(near), all);
        assert!(
            plain
                .chunks
                .iter()
                .all(|c| c.level.is_none() && c.range().end_margin == draw_range().end_margin)
        );
    }

    // --- Posts on the drawn ground (spec §4.6.1, 2026-11-17) ---------------

    /// A tile on a 10 m terrain grid with the ground `height(i, k)` and
    /// wooden posts (not on a road) every 3 m over `area` (x0, z0, x1, z1).
    fn posts_tile(height: impl Fn(u32, u32) -> f32, area: [f32; 4]) -> WorldPackage {
        let mut pkg = package();
        let n = 101u32;
        pkg.samples = n;
        let key = TileKey { x: 0, z: 0 };
        pkg.tiles.get_mut(&key).unwrap().heights =
            (0..n * n).map(|v| height(v % n, v / n)).collect();
        let mut points = Vec::new();
        let mut z = area[1];
        while z <= area[3] {
            let mut x = area[0];
            while x <= area[2] {
                points.push(PointObstacle {
                    kind: ObstacleKind::Post,
                    flags: 0,
                    pos: [x, z],
                    angle: 0.0,
                    width_m: 0.0,
                    height_m: 1.2,
                });
                x += 3.0;
            }
            z += 3.0;
        }
        let t = pkg.obstacles.as_mut().unwrap().tiles.get_mut(&key).unwrap();
        t.points = points;
        t.lines.clear();
        pkg
    }

    /// Each post's foot (its lowest vertex) against the drawn ground (the
    /// tile's terrain mesh after the junction lowering) at the post: below
    /// it (the post sinks in), but not deeper than half a metre. Returns
    /// the number of posts checked.
    fn post_feet_on_the_drawn_ground(pkg: &WorldPackage) -> usize {
        let key = TileKey { x: 0, z: 0 };
        let m = crate::package::build_tile_meshes(pkg, key).unwrap();
        let ground = TileGround::new(pkg, key, Some(&m.terrain));
        let verts: Vec<[f32; 3]> = m
            .obstacles
            .chunks
            .iter()
            .flat_map(|c| {
                let o = c.origin;
                c.mesh
                    .positions
                    .iter()
                    .map(move |p| [p[0] + o[0], p[1] + o[1], p[2] + o[2]])
            })
            .collect();
        let posts = &pkg.obstacles.as_ref().unwrap().tiles[&key].points;
        for p in posts {
            let foot = verts
                .iter()
                .filter(|v| (v[0] - p.pos[0]).hypot(v[2] - p.pos[1]) < 0.2)
                .map(|v| v[1])
                .fold(f32::MAX, f32::min);
            let g = ground
                .height(f64::from(p.pos[0]), f64::from(p.pos[1]))
                .unwrap() as f32;
            assert!(
                foot <= g - 0.1 && foot >= g - 0.5,
                "post at {:?}: foot {foot:.2}, drawn ground {g:.2}",
                p.pos
            );
        }
        posts.len()
    }

    /// Posts stand on the drawn terrain on terrace edges (the cells
    /// twisted: one corner metres above the others).
    #[test]
    fn posts_stand_on_the_drawn_ground_on_terrace_edges() {
        let terraces = |i: u32, k: u32| 300.0 + 4.0 * ((i * 7 + k * 13) % 5) as f32;
        let pkg = posts_tile(terraces, [101.0, 101.0, 199.0, 199.0]);
        assert!(post_feet_on_the_drawn_ground(&pkg) > 1_000);
    }

    /// Regression (2026-11-17): next to a junction the renderer lowers the
    /// terrain under the side road's blend (`junction_mesh::lower_terrain`);
    /// the posts there stand on the lowered ground. Before, they stood on
    /// the package's terrain and floated over the lowered one.
    #[test]
    fn posts_follow_the_ground_lowered_at_a_junction() {
        use ocw_world::graph::RoadClass;
        use ocw_world::junction::fixtures::t_junction;
        let mut pkg = posts_tile(
            |_, k| {
                let z = k as f32 * 10.0;
                if z > 3.75 { 0.2 * z - 0.05 } else { -0.05 }
            },
            [1.0, 1.0, 40.0, 130.0],
        );
        pkg.graph = Some(t_junction(RoadClass::Secondary, 0.0, 0.2).into());
        let key = TileKey { x: 0, z: 0 };
        let m = crate::package::build_tile_meshes(&pkg, key).unwrap();
        // The renderer lowered the ground under some of the posts.
        let lowest = pkg.obstacles.as_ref().unwrap().tiles[&key]
            .points
            .iter()
            .map(|p| {
                let (x, z) = (f64::from(p.pos[0]), f64::from(p.pos[1]));
                let drawn = TileGround::new(&pkg, key, Some(&m.terrain))
                    .height(x, z)
                    .unwrap();
                pkg.surface_height(x, z).unwrap() - drawn
            })
            .fold(0.0, f64::max);
        assert!(lowest > 0.3, "lowered by {lowest:.2} m under the posts");
        assert!(post_feet_on_the_drawn_ground(&pkg) > 100);
    }

    /// The sizes of the meshes the generator drew before the switch to the
    /// files (v5.08; at the kind's default height, a 3 m span, angle 0:
    /// model x across = world z): (kind, x, z half extents about the foot,
    /// bottom and top above the ground).
    type Size = (ObstacleKind, [f32; 2], [f32; 2], [f32; 2]);
    const OLD_SIZES: [Size; 7] = [
        (
            ObstacleKind::Bollard,
            [-0.101, 0.101],
            [-0.101, 0.101],
            [-0.1, 0.9],
        ),
        (
            ObstacleKind::Post,
            [-0.13, 0.13],
            [-0.1126, 0.1126],
            [-0.2, 7.0],
        ),
        (ObstacleKind::Block, [-0.3, 0.3], [-0.6, 0.6], [-0.1, 0.8]),
        (
            ObstacleKind::Gate,
            [-0.08, 0.08],
            [-1.5693, 1.5693],
            [-0.2, 1.3],
        ),
        (
            ObstacleKind::LiftGate,
            [-0.15, 0.15],
            [-1.6299, 1.5],
            [-0.2, 1.15],
        ),
        (
            ObstacleKind::CycleBarrier,
            [-0.63, 0.63],
            [-1.026, 1.026],
            [-0.1, 1.0],
        ),
        (
            ObstacleKind::Barrier,
            [-0.25, 0.25],
            [-0.2165, 0.2165],
            [-0.1, 1.0],
        ),
    ];

    /// A pylon's arms reach across its power line: the line's direction
    /// comes from the pylons beside it, not from the road.
    #[test]
    fn a_pylon_faces_its_power_line() {
        let mut pkg = posts_tile(|_, _| 300.0, [1.0, 1.0, 2.0, 2.0]);
        let pylon = |x: f32, z: f32| PointObstacle {
            kind: ObstacleKind::Pylon,
            flags: 0,
            pos: [x, z],
            angle: 0.0,
            width_m: 0.0,
            height_m: 25.5,
        };
        // A line running north-east: pylons 300 m apart on the diagonal.
        let key = TileKey { x: 0, z: 0 };
        let set = |pkg: &mut WorldPackage, points: Vec<PointObstacle>| {
            pkg.obstacles
                .as_mut()
                .unwrap()
                .tiles
                .entry(key)
                .or_default()
                .points = points;
        };
        set(
            &mut pkg,
            vec![
                pylon(100.0, 100.0),
                pylon(312.0, 312.0),
                pylon(524.0, 524.0),
            ],
        );
        let a = pylon_line_angle(&pkg, [312.0, 312.0]).unwrap();
        assert!((a - std::f32::consts::FRAC_PI_4).abs() < 1e-3, "{a}");
        let end = pylon_line_angle(&pkg, [100.0, 100.0]).unwrap();
        assert!((end - std::f32::consts::FRAC_PI_4).abs() < 1e-3, "{end}");
        // A lone pylon keeps the road's direction.
        set(&mut pkg, vec![pylon(100.0, 100.0)]);
        assert_eq!(pylon_line_angle(&pkg, [100.0, 100.0]), None);
    }

    /// Regression (2026-11-26, the Port d'Envalira): the power line
    /// pylons (`power=tower`) stood beside the roads as solid grey
    /// pyramids 25 m high, obelisks. They are drawn from the open lattice
    /// tower now: its cross arms reach out beyond the legs' foot, it
    /// stands at the pylon's height on its 5 m foot, and from the front
    /// most of its box is open (`ocw_assets::pylon` tests the coverage).
    #[test]
    fn pylons_are_lattice_towers_not_obelisks() {
        assert_eq!(model_id_of(ObstacleKind::Pylon), "props/pylon-lattice");
        let model = ocw_assets::shipped::model(model_id_of(ObstacleKind::Pylon));
        let flat = |_: f32, _: f32| 3.0;
        let p = PointObstacle {
            kind: ObstacleKind::Pylon,
            flags: 0,
            pos: [50.0, 60.0],
            angle: 0.0,
            width_m: 0.0,
            height_m: 25.5,
        };
        let mut m = MeshArrays::default();
        point_mesh(&mut m, &p, &model, &flat);
        let lo = m.positions.iter().fold([f32::MAX; 3], |a, v| {
            std::array::from_fn(|k| a[k].min(v[k]))
        });
        let hi = m.positions.iter().fold([f32::MIN; 3], |a, v| {
            std::array::from_fn(|k| a[k].max(v[k]))
        });
        assert!((hi[1] - 3.0 - 25.5).abs() < 0.3, "{lo:?} {hi:?}");
        let (wx, wz) = (hi[0] - lo[0], hi[2] - lo[2]);
        // The arms across the line (11 m), the foot along it (5 m).
        assert!(wx.max(wz) > 10.0 && wx.min(wz) < 6.0, "{wx} x {wz}");
        assert!(m.indices.len() / 3 > 500, "the lattice's members");
    }

    /// Every point obstacle is drawn from its model file (spec §14 Phase
    /// 15, 15H MX), at the size the generator drew it, with the file's
    /// triangles; each file has its collision proxies and its fit.
    #[test]
    fn point_obstacles_are_drawn_from_their_files_at_the_old_sizes() {
        let flat = |_: f32, _: f32| 3.0;
        for (kind, x, z, y) in OLD_SIZES {
            let id = model_id_of(kind);
            let model = ocw_assets::shipped::model(id);
            let e = ocw_assets::library::library()
                .entry(id)
                .expect("in the manifest");
            assert_eq!(e.kind, ocw_assets::kinds::Kind::Prop, "{id}");
            let fit = ocw_assets::props::FileFit::of(&model).expect("a fit");
            assert_eq!(fit.height_m, kind.default_height_m() as f32, "{id}");
            assert!(
                !ocw_assets::props::proxies(&model).is_empty(),
                "{id}: col_*"
            );
            let p = PointObstacle {
                kind,
                flags: 0,
                pos: [50.0, 60.0],
                angle: 0.0,
                width_m: 0.0,
                height_m: kind.default_height_m() as f32,
            };
            let mut m = MeshArrays::default();
            point_mesh(&mut m, &p, &model, &flat);
            let drawn: usize = model
                .nodes
                .iter()
                .enumerate()
                .filter(|(i, _)| !model.under_named(*i, |n| n.starts_with("col_")))
                .flat_map(|(_, n)| &n.primitives)
                .map(|p| p.triangles())
                .sum();
            assert_eq!(m.indices.len() / 3, drawn, "{id}");
            let lo = m.positions.iter().fold([f32::MAX; 3], |a, v| {
                std::array::from_fn(|k| a[k].min(v[k]))
            });
            let hi = m.positions.iter().fold([f32::MIN; 3], |a, v| {
                std::array::from_fn(|k| a[k].max(v[k]))
            });
            let near = |a: f32, b: f32| (a - b).abs() < 2e-3;
            assert!(
                near(lo[0] - 50.0, x[0])
                    && near(hi[0] - 50.0, x[1])
                    && near(lo[2] - 60.0, z[0])
                    && near(hi[2] - 60.0, z[1])
                    && near(lo[1] - 3.0, y[0])
                    && near(hi[1] - 3.0, y[1]),
                "{id}: {lo:?} {hi:?}"
            );
        }
        // Kinds without a file of their own use the generic barrier post.
        assert_eq!(
            model_id_of(ObstacleKind::Kerb),
            "props/barrier-post-hexagonal"
        );
    }

    /// A gate across a 6 m road: posts 3 m either side, the bars between
    /// them; a taller bollard is scaled with its foot as deep.
    #[test]
    fn spans_and_heights_fit_the_files() {
        let flat = |_: f32, _: f32| 0.0;
        let gate = ocw_assets::shipped::model(model_id_of(ObstacleKind::Gate));
        let p = PointObstacle {
            kind: ObstacleKind::Gate,
            flags: 0,
            pos: [0.0, 0.0],
            angle: 0.0,
            width_m: 6.0,
            height_m: 1.2,
        };
        let mut m = MeshArrays::default();
        point_mesh(&mut m, &p, &gate, &flat);
        let zs = m
            .positions
            .iter()
            .fold((f32::MAX, f32::MIN), |(a, b), v| (a.min(v[2]), b.max(v[2])));
        assert!(
            (zs.0 + 3.0693).abs() < 2e-3 && (zs.1 - 3.0693).abs() < 2e-3,
            "{zs:?}"
        );
        let bollard = ocw_assets::shipped::model(model_id_of(ObstacleKind::Bollard));
        let p = PointObstacle {
            kind: ObstacleKind::Bollard,
            height_m: 1.4,
            ..p
        };
        let mut m = MeshArrays::default();
        point_mesh(&mut m, &p, &bollard, &flat);
        let ys = m
            .positions
            .iter()
            .fold((f32::MAX, f32::MIN), |(a, b), v| (a.min(v[1]), b.max(v[1])));
        assert!(
            (ys.0 + 0.1).abs() < 1e-4 && (ys.1 - 1.4).abs() < 1e-4,
            "{ys:?}"
        );
    }

    #[test]
    fn the_setting_shows_and_hides_the_road_barriers() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            crate::console::ConsoleCorePlugin,
            ObstaclesPlugin,
        ));
        let e = app
            .world_mut()
            .spawn((Visibility::default(), RoadBarrierMesh))
            .id();
        app.update();
        assert_eq!(app.world().get::<Visibility>(e), Some(&Visibility::Hidden));
        app.world_mut()
            .resource_mut::<crate::console::ConsoleState>()
            .queue("road_barriers on");
        app.update();
        app.update();
        assert!(app.world().resource::<ObstacleSettings>().road_barriers);
        assert_eq!(
            app.world().get::<Visibility>(e),
            Some(&Visibility::Inherited)
        );
    }
}
