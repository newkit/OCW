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
use ocw_assets::props::{Instance, Placed, place};
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
    /// Where the chunk's mesh is placed (its first vertex; the level of
    /// detail is measured from there).
    pub origin: [f32; 3],
    pub mesh: MeshArrays,
    /// Road barriers (toggled by the setting).
    pub road: bool,
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
        &place(model, &inst, foot, yaw_of(p.angle), ground),
        |_| true,
    );
}

/// The model file of a point obstacle kind (spec §14 Phase 15, 15H names;
/// kinds without their own file use the generic barrier post).
pub fn model_id_of(kind: ObstacleKind) -> &'static str {
    match kind {
        ObstacleKind::Bollard => "props/bollard-round",
        ObstacleKind::Post => "props/post-hexagonal",
        ObstacleKind::Pylon => "props/pylon-pyramid",
        ObstacleKind::Block => "props/block-concrete-box",
        ObstacleKind::Gate => "props/gate-swing",
        ObstacleKind::LiftGate => "props/barrier-lift-arm",
        ObstacleKind::CycleBarrier => "props/barrier-cycle-staggered",
        _ => "props/barrier-post-hexagonal",
    }
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
        .filter(|p| kept(p.pos) && p.kind != ObstacleKind::PlantedTree)
    {
        let target = if p.road_barrier() {
            &mut road
        } else {
            &mut rest
        };
        let on_road = p.road_barrier();
        let at = |x: f32, z: f32| foot(pkg, ground, x, z, on_road);
        let model = match models.iter().find(|(k, _)| *k == p.kind) {
            Some((_, m)) => m.clone(),
            None => {
                let m = ocw_assets::shipped::model(model_id_of(p.kind));
                models.push((p.kind, m.clone()));
                m
            }
        };
        point_mesh(&mut target[chunk_of(p.pos)], p, &model, &at);
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
            });
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
        pkg.graph = Some(t_junction(RoadClass::Secondary, 0.0, 0.2));
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
    const OLD_SIZES: [Size; 8] = [
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
        (ObstacleKind::Pylon, [-2.5, 2.5], [-2.5, 2.5], [-0.2, 30.0]),
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
