//! The obstacles of a world package drawn (spec §14 Phase 12 K3, Q7):
//! fences, walls, hedges and guard rails beside the roads, posts, pylons
//! and the point barriers off the road – what riders in wild mode can hit
//! – always; the **road barriers** (gates, bollards, lift gates, blocks,
//! cycle barriers on a road or within 2 m of it, section `OBST` flag
//! `ROAD_BARRIER`) only with the setting `display.road_barriers` (console
//! `road_barriers on|off`, default off): they never block the routed road.
//!
//! Simple generated meshes in the vertex colours, per 125 m chunk of a
//! tile, drawn up to [`DRAW_M`] from the camera (`VisibilityRange`, the
//! cheap level of detail). Tunnel walls and bridge railings have their
//! own meshes (`tunnel`, `bridges`); kerbs are part of the road dress.

use bevy::camera::visibility::VisibilityRange;
use bevy::prelude::*;
use clap::{Arg, Command};
use ocw_world::obstacles::{LineObstacle, ObstacleKind, PointObstacle};
use ocw_world::{TileKey, WorldPackage};

use crate::console::{ConsoleAppExt, ConsoleCommand, Permission};
use crate::package::{Keep, MeshArrays};

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

/// Colours of the parts.
struct Palette {
    concrete: [f32; 4],
    steel: [f32; 4],
    wood: [f32; 4],
    stone: [f32; 4],
    hedge: [f32; 4],
    red: [f32; 4],
    white: [f32; 4],
    dark: [f32; 4],
}

impl Palette {
    fn new() -> Self {
        Self {
            concrete: lin(0.62, 0.61, 0.58),
            steel: lin(0.66, 0.68, 0.70),
            wood: lin(0.45, 0.33, 0.22),
            stone: lin(0.55, 0.52, 0.47),
            hedge: lin(0.17, 0.32, 0.13),
            red: lin(0.78, 0.12, 0.10),
            white: lin(0.92, 0.92, 0.90),
            dark: lin(0.22, 0.23, 0.24),
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
/// terrain (or the road's height where the terrain is missing).
fn foot(pkg: &WorldPackage, x: f32, z: f32, on_road: bool) -> f32 {
    let (xf, zf) = (f64::from(x), f64::from(z));
    let road = pkg
        .road_heights(xf, zf)
        .into_iter()
        .fold(None::<f64>, |m, h| Some(m.map_or(h, |m| m.max(h))));
    match (on_road, road) {
        (true, Some(r)) => (r + ocw_world::package::ROAD_LIFT_M) as f32,
        _ => pkg
            .surface_height(xf, zf)
            .or(road)
            .map_or(0.0, |h| h as f32),
    }
}

/// A point obstacle's mesh.
fn point_mesh(m: &mut MeshArrays, pkg: &WorldPackage, p: &PointObstacle, c: &Palette) {
    let on_road = p.road_barrier();
    let y = foot(pkg, p.pos[0], p.pos[1], on_road);
    let at = [p.pos[0], y, p.pos[1]];
    let h = p.height_m.max(0.2);
    // Across the way the barrier stands on.
    let (ax, az) = (-p.angle.sin(), p.angle.cos());
    let across = |w: f32| {
        let q = [at[0] + ax * w, at[2] + az * w];
        [q[0], foot(pkg, q[0], q[1], on_road), q[1]]
    };
    let span = if p.width_m > 0.5 { p.width_m } else { 3.0 };
    match p.kind {
        ObstacleKind::Bollard => {
            prism(m, at, 0.1, 0.09, -0.1, h, 8, c.dark);
            prism(m, at, 0.101, 0.101, h - 0.25, h - 0.15, 8, c.white);
        }
        ObstacleKind::Post => prism(m, at, 0.13, 0.1, -0.2, h, 6, c.wood),
        ObstacleKind::Pylon => prism(m, at, 2.5, 0.4, -0.2, h, 4, c.steel),
        ObstacleKind::Block => {
            let (a, b) = (across(-0.6), across(0.6));
            beam(m, a, b, 0.3, -0.1, h, c.concrete);
        }
        ObstacleKind::Gate => {
            let (a, b) = (across(-span / 2.0), across(span / 2.0));
            prism(m, a, 0.08, 0.08, -0.2, h + 0.1, 6, c.steel);
            prism(m, b, 0.08, 0.08, -0.2, h + 0.1, 6, c.steel);
            for k in [0.35, 0.65, 0.95] {
                beam(m, a, b, 0.03, h * k - 0.03, h * k + 0.03, c.steel);
            }
        }
        ObstacleKind::LiftGate => {
            let (a, b) = (across(-span / 2.0), across(span / 2.0));
            prism(m, a, 0.15, 0.15, -0.2, h + 0.15, 6, c.white);
            let bar = h - 0.05;
            // Red and white stripes of 0.5 m.
            let n = (span / 0.5).ceil().max(1.0) as usize;
            for k in 0..n {
                let (t0, t1) = (k as f32 / n as f32, (k + 1) as f32 / n as f32);
                let p0 = [a[0] + (b[0] - a[0]) * t0, a[1], a[2] + (b[2] - a[2]) * t0];
                let p1 = [a[0] + (b[0] - a[0]) * t1, a[1], a[2] + (b[2] - a[2]) * t1];
                let col = if k % 2 == 0 { c.red } else { c.white };
                beam(m, p0, p1, 0.04, bar - 0.05, bar + 0.05, col);
            }
        }
        ObstacleKind::CycleBarrier => {
            // Two staggered rails, each across most of the way.
            let (ux, uz) = (p.angle.cos(), p.angle.sin());
            for (off, from, to) in [(-0.6f32, -1.0f32, 0.4f32), (0.6, -0.4, 1.0)] {
                let base = [at[0] + ux * off, at[2] + uz * off];
                let q = |w: f32| {
                    let q = [base[0] + ax * w, base[1] + az * w];
                    [q[0], foot(pkg, q[0], q[1], on_road), q[1]]
                };
                let (a, b) = (q(from), q(to));
                beam(m, a, b, 0.03, h - 0.06, h, c.steel);
                prism(m, a, 0.03, 0.03, -0.1, h, 6, c.steel);
                prism(m, b, 0.03, 0.03, -0.1, h, 6, c.steel);
            }
        }
        _ => prism(m, at, 0.25, 0.2, -0.1, h, 6, c.dark),
    }
}

/// A line obstacle's mesh (per segment, feet on the terrain).
fn line_mesh(m: &mut MeshArrays, pkg: &WorldPackage, l: &LineObstacle, c: &Palette) {
    let ground = |p: [f32; 2]| [p[0], foot(pkg, p[0], p[1], false), p[1]];
    let h = l.height_m.max(0.3);
    for w in l.points.windows(2) {
        let (a, b) = (ground(w[0]), ground(w[1]));
        let len = (b[0] - a[0]).hypot(b[2] - a[2]);
        match l.kind {
            ObstacleKind::Wall => beam(m, a, b, 0.2, -0.3, h, c.stone),
            ObstacleKind::Hedge => beam(m, a, b, 0.5, -0.2, h, c.hedge),
            ObstacleKind::Fence | ObstacleKind::GateLine => {
                let posts = (len / 2.5).ceil().max(1.0) as usize;
                for k in 0..=posts {
                    let t = k as f32 / posts as f32;
                    let q = ground([
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
                    let q = ground([
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
pub fn tile_obstacle_meshes(pkg: &WorldPackage, key: TileKey, keep: Keep) -> TileObstacleMeshes {
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
    let n = (CHUNKS * CHUNKS) as usize;
    let (mut road, mut rest): (Vec<MeshArrays>, Vec<MeshArrays>) = (
        (0..n).map(|_| MeshArrays::default()).collect(),
        (0..n).map(|_| MeshArrays::default()).collect(),
    );
    for p in t.points.iter().filter(|p| kept(p.pos)) {
        let target = if p.road_barrier() {
            &mut road
        } else {
            &mut rest
        };
        point_mesh(&mut target[chunk_of(p.pos)], pkg, p, &colours);
    }
    for l in t
        .lines
        .iter()
        .filter(|l| l.points.first().is_some_and(|p| kept(*p)))
    {
        line_mesh(&mut rest[chunk_of(l.points[0])], pkg, l, &colours);
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
        }
    }

    #[test]
    fn road_barriers_and_the_rest_are_separate_chunks_on_the_ground() {
        let pkg = package();
        let mut m = tile_obstacle_meshes(&pkg, TileKey { x: 0, z: 0 }, None);
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
        assert!(tile_obstacle_meshes(&pkg, TileKey { x: 0, z: 0 }, Some(none)).is_empty());
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
