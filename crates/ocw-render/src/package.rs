//! Streams a world package (spec §4.5): the 5 × 5 tiles around the local
//! rider, meshed on background threads, uploaded one per frame; far tiles
//! are dropped. Terrain, all road strips, and (provisionally) scattered
//! trees merged per tile (spike decision, spec §3.1.1).

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::light::NotShadowCaster;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task, futures::check_ready};
use ocw_world::graph::RoadClass;
use ocw_world::package::{RoadStrip, Tile, TileKey, WorldPackage};

use crate::RideView;

/// Tiles kept around the rider (radius in tiles; 2 = 5 × 5).
pub const STREAM_RADIUS: i32 = 2;
/// Road edge faces reach this far down (covers the terrain clearance).
const EDGE_DEPTH_M: f32 = 2.0;

/// The loaded world package (inserted by the app with `--world`).
#[derive(Resource, Clone)]
pub struct PackageWorld(pub Arc<WorldPackage>);

pub struct PackagePlugin;

impl Plugin for PackagePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TileStreamer>()
            .add_systems(Startup, setup_materials)
            .add_systems(
                Update,
                (stream_tiles, upload_tiles)
                    .chain()
                    .run_if(resource_exists::<PackageWorld>),
            );
    }
}

/// Mesh arrays of one tile (built off the main thread).
#[derive(Default)]
pub struct MeshArrays {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub colors: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
}

impl MeshArrays {
    fn mesh(self) -> Mesh {
        let mut m = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        );
        m.insert_attribute(Mesh::ATTRIBUTE_POSITION, self.positions);
        m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals);
        m.insert_attribute(Mesh::ATTRIBUTE_COLOR, self.colors);
        m.insert_indices(Indices::U32(self.indices));
        m
    }
}

pub struct TileMeshes {
    pub key: TileKey,
    pub terrain: MeshArrays,
    pub roads: MeshArrays,
    pub trees: MeshArrays,
    /// The same trees as obstacles for the camera.
    pub tree_cones: Vec<crate::TreeCone>,
}

fn lin(r: f32, g: f32, b: f32) -> [f32; 4] {
    let c = Color::srgb(r, g, b).to_linear();
    [c.red, c.green, c.blue, 1.0]
}

/// Terrain mesh of a tile, in world coordinates (local metres).
pub fn terrain_arrays(pkg: &WorldPackage, tile: &Tile) -> MeshArrays {
    let n = pkg.samples as usize;
    let s = pkg.spacing_m() as f32;
    let o = tile.key.origin(f64::from(pkg.tile_size_m));
    let h = |i: usize, k: usize| tile.heights[k.min(n - 1) * n + i.min(n - 1)];
    let mut m = MeshArrays::default();
    let grass = lin(0.33, 0.52, 0.25);
    let rock = lin(0.5, 0.48, 0.44);
    for k in 0..n {
        for i in 0..n {
            let (il, ir) = (i.saturating_sub(1), (i + 1).min(n - 1));
            let (ku, kd) = (k.saturating_sub(1), (k + 1).min(n - 1));
            let dx = (h(ir, k) - h(il, k)) / ((ir - il) as f32 * s);
            let dz = (h(i, kd) - h(i, ku)) / ((kd - ku) as f32 * s);
            m.positions.push([
                o[0] as f32 + i as f32 * s,
                h(i, k),
                o[1] as f32 + k as f32 * s,
            ]);
            m.normals
                .push(Vec3::new(-dx, 1.0, -dz).normalize().to_array());
            let slope = (dx * dx + dz * dz).sqrt();
            let t = ((slope - 0.6) / 0.4).clamp(0.0, 1.0);
            m.colors
                .push(std::array::from_fn(|c| grass[c] + (rock[c] - grass[c]) * t));
        }
    }
    for k in 0..n - 1 {
        for i in 0..n - 1 {
            let v = |i: usize, k: usize| (k * n + i) as u32;
            let (a, b, c, d) = (v(i, k), v(i + 1, k), v(i, k + 1), v(i + 1, k + 1));
            m.indices.extend([a, c, b, b, c, d]);
        }
    }
    m
}

fn road_colour(class: RoadClass) -> [f32; 4] {
    match class {
        RoadClass::Track => lin(0.55, 0.47, 0.36),
        RoadClass::Path => lin(0.62, 0.56, 0.45),
        RoadClass::Cycleway => lin(0.3, 0.3, 0.33),
        _ => lin(0.22, 0.22, 0.24),
    }
}

/// Road surface strips with edge faces, in world coordinates.
pub fn road_arrays(strips: &[RoadStrip]) -> MeshArrays {
    let mut m = MeshArrays::default();
    let edge = lin(0.45, 0.43, 0.38);
    for s in strips {
        let colour = road_colour(s.class);
        let half = s.width_m / 2.0;
        let p: Vec<Vec3> = s.points.iter().map(|q| Vec3::from(*q)).collect();
        let n = p.len();
        let base = m.positions.len() as u32;
        // Surface: two vertices per point.
        for i in 0..n {
            let dir = (p[(i + 1).min(n - 1)] - p[i.saturating_sub(1)])
                .with_y(0.0)
                .normalize_or_zero();
            let right = Vec3::new(-dir.z, 0.0, dir.x);
            for side in [-half, half] {
                m.positions
                    .push((p[i] + right * side + Vec3::Y * 0.02).to_array());
                m.normals.push([0.0, 1.0, 0.0]);
                m.colors.push(colour);
            }
        }
        for i in 0..n as u32 - 1 {
            let (l0, r0, l1, r1) = (
                base + 2 * i,
                base + 2 * i + 1,
                base + 2 * i + 2,
                base + 2 * i + 3,
            );
            m.indices.extend([l0, r0, l1, r0, r1, l1]);
        }
        // Edge faces down into the terrain, both sides, both faces.
        for side in [-1.0f32, 1.0] {
            let eb = m.positions.len() as u32;
            for i in 0..n {
                let dir = (p[(i + 1).min(n - 1)] - p[i.saturating_sub(1)])
                    .with_y(0.0)
                    .normalize_or_zero();
                let out = Vec3::new(-dir.z, 0.0, dir.x) * side;
                let top = p[i] + out * half + Vec3::Y * 0.02;
                m.positions.push(top.to_array());
                m.positions.push((top - Vec3::Y * EDGE_DEPTH_M).to_array());
                m.normals.extend([out.to_array(); 2]);
                m.colors.extend([edge; 2]);
            }
            for i in 0..n as u32 - 1 {
                let (t0, b0, t1, b1) = (eb + 2 * i, eb + 2 * i + 1, eb + 2 * i + 2, eb + 2 * i + 3);
                m.indices
                    .extend([t0, b0, t1, b0, b1, t1, t0, t1, b0, b0, t1, b1]);
            }
        }
    }
    m
}

fn hash(x: i64, z: i64, seed: u64) -> f32 {
    let mut h = (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (z as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ seed;
    h ^= h >> 33;
    h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    h ^= h >> 33;
    (h >> 40) as f32 / (1u64 << 24) as f32
}

/// Crown of a package tree (scale 1): bottom, tip and radius in metres.
const CROWN: (f32, f32, f32) = (1.8, 9.0, 2.2);

/// The trees of a tile as obstacles (see [`tree_arrays`]).
pub fn tree_cones(pkg: &WorldPackage, tile: &Tile) -> Vec<crate::TreeCone> {
    tree_sites(pkg, tile)
        .into_iter()
        .map(|(base, s)| crate::TreeCone {
            base,
            crown_bottom: CROWN.0 * s,
            crown_top: CROWN.1 * s,
            radius: CROWN.2 * s,
        })
        .collect()
}

/// Provisional vegetation: trees on a jittered grid in patches, not on
/// steep rock and never on a road (land use from OSM replaces this).
pub fn tree_arrays(pkg: &WorldPackage, tile: &Tile) -> MeshArrays {
    let mut m = MeshArrays::default();
    let crown = lin(0.13, 0.33, 0.15);
    let trunk = lin(0.35, 0.24, 0.15);
    for (base, s) in tree_sites(pkg, tile) {
        for (y0, y1, r, colour, seg) in [
            (0.0, 2.2, 0.25, trunk, 5u32),
            (CROWN.0, CROWN.1, CROWN.2, crown, 7),
        ] {
            let b = m.positions.len() as u32;
            for j in 0..seg {
                let a = j as f32 / seg as f32 * std::f32::consts::TAU;
                m.positions
                    .push((base + Vec3::new(r * a.cos(), y0, r * a.sin()) * s).to_array());
                m.normals.push(
                    Vec3::new(a.cos(), r / (y1 - y0), a.sin())
                        .normalize()
                        .to_array(),
                );
                m.colors.push(colour);
            }
            m.positions.push((base + Vec3::Y * y1 * s).to_array());
            m.normals.push([0.0, 1.0, 0.0]);
            m.colors.push(colour);
            for j in 0..seg {
                m.indices.extend([b + j, b + seg, b + (j + 1) % seg]);
            }
        }
    }
    m
}

/// Where the trees of a tile stand (foot of the trunk) and their scale.
fn tree_sites(pkg: &WorldPackage, tile: &Tile) -> Vec<(Vec3, f32)> {
    let size = f64::from(pkg.tile_size_m);
    let o = tile.key.origin(size);
    let cell = 18.0;
    let cells = (size / cell) as i64;
    let mut sites = Vec::new();
    let road_pts: Vec<(Vec2, f32)> = tile
        .roads
        .iter()
        .flat_map(|r| {
            r.points
                .iter()
                .map(move |p| (Vec2::new(p[0], p[2]), r.width_m / 2.0 + 5.0))
        })
        .collect();
    for k in 0..cells {
        for i in 0..cells {
            let (gx, gz) = (tile.key.x as i64 * cells + i, tile.key.z as i64 * cells + k);
            // Forest patches: coarse noise from a 6 × 6-cell lattice.
            let patch = hash(gx.div_euclid(6), gz.div_euclid(6), 7);
            if hash(gx, gz, 1) > patch * 0.9 {
                continue;
            }
            let x = o[0] + (i as f64 + f64::from(hash(gx, gz, 2))) * cell;
            let z = o[1] + (k as f64 + f64::from(hash(gx, gz, 3))) * cell;
            let p2 = Vec2::new(x as f32, z as f32);
            if road_pts.iter().any(|(q, r)| q.distance_squared(p2) < r * r) {
                continue;
            }
            let (Some(y), Some(yx), Some(yz)) = (
                pkg.height(x, z),
                pkg.height(x + 2.0, z),
                pkg.height(x, z + 2.0),
            ) else {
                continue;
            };
            if ((yx - y).powi(2) + (yz - y).powi(2)).sqrt() / 2.0 > 0.8 {
                continue; // rock faces
            }
            let s = 0.7 + 0.7 * hash(gx, gz, 4);
            let base = Vec3::new(x as f32, y as f32 - 0.3, z as f32);
            sites.push((base, s));
        }
    }
    sites
}

pub fn build_tile_meshes(pkg: &WorldPackage, key: TileKey) -> Option<TileMeshes> {
    let tile = pkg.tiles.get(&key)?;
    Some(TileMeshes {
        key,
        terrain: terrain_arrays(pkg, tile),
        roads: road_arrays(&tile.roads),
        trees: tree_arrays(pkg, tile),
        tree_cones: tree_cones(pkg, tile),
    })
}

#[derive(Resource)]
struct PackageMaterials {
    vertex_coloured: Handle<StandardMaterial>,
}

fn setup_materials(mut commands: Commands, mut materials: ResMut<Assets<StandardMaterial>>) {
    commands.insert_resource(PackageMaterials {
        vertex_coloured: materials.add(StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 1.0,
            ..default()
        }),
    });
}

#[derive(Resource, Default)]
struct TileStreamer {
    loaded: HashMap<TileKey, Entity>,
    building: HashMap<TileKey, Task<Option<TileMeshes>>>,
    ready: VecDeque<TileMeshes>,
}

fn rider_tile(view: &RideView, pkg: &WorldPackage) -> Option<TileKey> {
    let local = view.local()?;
    let p = view.track(local.id).sample(local.state.distance).position;
    Some(TileKey::containing(
        [p[0], p[2]],
        f64::from(pkg.tile_size_m),
    ))
}

fn stream_tiles(
    mut commands: Commands,
    world: Res<PackageWorld>,
    view: Res<RideView>,
    mut streamer: ResMut<TileStreamer>,
    mut trees: ResMut<crate::Trees>,
) {
    let pkg = &world.0;
    let Some(center) = rider_tile(&view, pkg) else {
        return;
    };
    let pool = AsyncComputeTaskPool::get();
    let mut wanted: Vec<TileKey> = (-STREAM_RADIUS..=STREAM_RADIUS)
        .flat_map(|dz| {
            (-STREAM_RADIUS..=STREAM_RADIUS).map(move |dx| TileKey {
                x: center.x + dx,
                z: center.z + dz,
            })
        })
        .filter(|k| pkg.tiles.contains_key(k))
        .collect();
    wanted.sort_by_key(|k| k.ring(center));
    for key in wanted {
        if streamer.loaded.contains_key(&key) || streamer.building.contains_key(&key) {
            continue;
        }
        let pkg = world.0.clone();
        streamer
            .building
            .insert(key, pool.spawn(async move { build_tile_meshes(&pkg, key) }));
    }
    let keys: Vec<TileKey> = streamer.building.keys().copied().collect();
    for key in keys {
        let task = streamer.building.get_mut(&key).expect("key exists");
        if let Some(result) = check_ready(task) {
            streamer.building.remove(&key);
            if let Some(meshes) = result {
                streamer.ready.push_back(meshes);
            }
        }
    }
    let far: Vec<TileKey> = streamer
        .loaded
        .keys()
        .filter(|k| k.ring(center) > STREAM_RADIUS + 1)
        .copied()
        .collect();
    for key in far {
        if let Some(e) = streamer.loaded.remove(&key) {
            commands.entity(e).despawn();
            trees.0.remove(&(key.x, key.z));
        }
    }
}

fn upload_tiles(
    mut commands: Commands,
    mut streamer: ResMut<TileStreamer>,
    materials: Res<PackageMaterials>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut trees: ResMut<crate::Trees>,
) {
    let Some(mut t) = streamer.ready.pop_front() else {
        return;
    };
    trees
        .0
        .insert((t.key.x, t.key.z), std::mem::take(&mut t.tree_cones));
    let mat = materials.vertex_coloured.clone();
    let parent = commands
        .spawn((
            Transform::default(),
            Visibility::default(),
            Name::new(format!("tile {} {}", t.key.x, t.key.z)),
        ))
        .with_children(|c| {
            c.spawn((
                Mesh3d(meshes.add(t.terrain.mesh())),
                MeshMaterial3d(mat.clone()),
                NotShadowCaster,
            ));
            if !t.roads.indices.is_empty() {
                c.spawn((
                    Mesh3d(meshes.add(t.roads.mesh())),
                    MeshMaterial3d(mat.clone()),
                    NotShadowCaster,
                ));
            }
            if !t.trees.indices.is_empty() {
                c.spawn((Mesh3d(meshes.add(t.trees.mesh())), MeshMaterial3d(mat)));
            }
        })
        .id();
    if let Some(old) = streamer.loaded.insert(t.key, parent) {
        commands.entity(old).despawn();
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn package() -> WorldPackage {
        let key = TileKey { x: 0, z: 0 };
        let n = 11;
        let tile = Tile {
            key,
            heights: (0..n * n).map(|i| (i % n) as f32).collect(), // rising 1 m per 10 m in x
            roads: vec![RoadStrip {
                class: RoadClass::Secondary,
                width_m: 6.0,
                points: (0..11).map(|i| [i as f32 * 10.0, 5.0, 50.0]).collect(),
            }],
        };
        WorldPackage {
            name: "t".into(),
            origin: (47.0, 9.0),
            tile_size_m: 100.0,
            samples: n as u32,
            tiles: BTreeMap::from([(key, tile)]),
            graph: None,
        }
    }

    #[test]
    fn terrain_mesh_matches_the_heights_and_faces_up() {
        let p = package();
        let m = terrain_arrays(&p, &p.tiles[&TileKey { x: 0, z: 0 }]);
        assert_eq!(m.positions.len(), 121);
        assert_eq!(m.indices.len(), 10 * 10 * 6);
        assert_eq!(m.positions[5], [50.0, 5.0, 0.0]);
        for tri in m.indices.chunks(3) {
            let [a, b, c] = [0, 1, 2].map(|j| Vec3::from(m.positions[tri[j] as usize]));
            assert!((b - a).cross(c - a).y > 0.0);
        }
        // 10 % slope in x: normal tilts towards -x.
        assert!(m.normals[60][0] < -0.05);
    }

    #[test]
    fn road_strips_have_surface_and_edges() {
        let p = package();
        let m = road_arrays(&p.tiles[&TileKey { x: 0, z: 0 }].roads);
        // Surface: 11 points × 2; edges: 2 sides × 11 × 2.
        assert_eq!(m.positions.len(), 22 + 44);
        // Surface is 6 m wide (z 47 … 53), slightly above the centreline.
        assert!((m.positions[0][2] - 47.0).abs() < 1e-4 || (m.positions[0][2] - 53.0).abs() < 1e-4);
        assert!((m.positions[0][1] - 5.02).abs() < 1e-4);
        // Edge faces reach down below the terrain clearance.
        let min_y = m
            .positions
            .iter()
            .map(|p| p[1])
            .fold(f32::INFINITY, f32::min);
        assert!(min_y < 5.0 - 1.5);
    }

    #[test]
    fn trees_avoid_roads_and_are_deterministic() {
        let p = package();
        let tile = &p.tiles[&TileKey { x: 0, z: 0 }];
        let a = tree_arrays(&p, tile);
        assert_eq!(a.positions, tree_arrays(&p, tile).positions);
        // Each tree has 14 vertices (trunk 5 + tip, crown 7 + tip); its
        // first vertex lies on the trunk ring at angle 0, i.e. at the base's z.
        assert!(!a.positions.is_empty());
        assert_eq!(a.positions.len() % 14, 0);
        for tree in a.positions.chunks(14) {
            // Corridor: half width 3 m + 5 m clearance.
            assert!(
                (tree[0][2] - 50.0).abs() >= 8.0 - 1e-3,
                "tree at z {}",
                tree[0][2]
            );
        }
        assert!(build_tile_meshes(&p, TileKey { x: 5, z: 5 }).is_none());
    }
}
