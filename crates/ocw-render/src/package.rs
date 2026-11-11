//! Streams a world package (spec §4.5): the 5 × 5 tiles around the local
//! rider, meshed on background threads, uploaded one per frame; far tiles
//! are dropped. Terrain, all road strips, water surfaces, trees and
//! buildings merged per tile (spike decision, spec §3.1.1).
//!
//! With land use (package section `LUSE`, spec §4.6) the terrain gets a
//! colour map per tile (one texel per land-use cell, mipmapped), trees
//! stand where the land use says (dense in forests, scattered on meadows,
//! never on roads, water or buildings) and water is drawn flat at its
//! level. Packages without land use keep plain grass and placeholder tree
//! patches.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::light::NotShadowCaster;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages,
};
use bevy::tasks::{AsyncComputeTaskPool, Task, futures::check_ready};
use ocw_world::buildings::{Building, BuildingKind, Roof, triangulate};
use ocw_world::graph::RoadClass;
use ocw_world::landuse::{LandClass, LandUseTile};
use ocw_world::package::{RoadStrip, Tile, TileKey, WorldPackage};

use crate::RideView;
use crate::camera::{CameraInteraction, CameraObstacles, ObstacleLayer, SolidFootprints};
use crate::see_through::{SeeThroughMaterial, see_through};

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
    /// Texture coordinates (terrain only; empty otherwise).
    pub uvs: Vec<[f32; 2]>,
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
        if !self.uvs.is_empty() {
            m.insert_attribute(Mesh::ATTRIBUTE_UV_0, self.uvs);
        }
        m.insert_indices(Indices::U32(self.indices));
        m
    }
}

pub struct TileMeshes {
    pub key: TileKey,
    pub terrain: MeshArrays,
    /// Terrain colours from the land use (`None`: vertex colours only).
    pub colour_map: Option<ColourMap>,
    pub roads: MeshArrays,
    pub water: MeshArrays,
    pub trees: MeshArrays,
    pub buildings: MeshArrays,
    /// The buildings as camera-solid obstacles.
    pub solid: SolidFootprints,
}

/// An sRGB RGBA colour map with its mip chain (level 0 first).
pub struct ColourMap {
    pub size: u32,
    pub levels: u32,
    pub data: Vec<u8>,
}

impl ColourMap {
    fn image(self) -> Image {
        Image {
            data: Some(self.data),
            texture_descriptor: TextureDescriptor {
                label: Some("land-use colour map"),
                size: Extent3d {
                    width: self.size,
                    height: self.size,
                    depth_or_array_layers: 1,
                },
                mip_level_count: self.levels,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: TextureFormat::Rgba8UnormSrgb,
                usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
                view_formats: &[],
            },
            sampler: ImageSampler::Descriptor(ImageSamplerDescriptor {
                address_mode_u: ImageAddressMode::ClampToEdge,
                address_mode_v: ImageAddressMode::ClampToEdge,
                mag_filter: ImageFilterMode::Linear,
                min_filter: ImageFilterMode::Linear,
                mipmap_filter: ImageFilterMode::Linear,
                anisotropy_clamp: 8,
                ..default()
            }),
            asset_usage: RenderAssetUsages::RENDER_WORLD,
            ..default()
        }
    }
}

fn lin(r: f32, g: f32, b: f32) -> [f32; 4] {
    let c = Color::srgb(r, g, b).to_linear();
    [c.red, c.green, c.blue, 1.0]
}

/// Terrain mesh of a tile, in world coordinates (local metres). With land
/// use the colours come from the colour map and the vertices stay white.
pub fn terrain_arrays(pkg: &WorldPackage, tile: &Tile) -> MeshArrays {
    let textured = tile_landuse(pkg, tile.key).is_some();
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
            m.uvs
                .push([i as f32 / (n - 1) as f32, k as f32 / (n - 1) as f32]);
            if textured {
                m.colors.push([1.0; 4]);
                continue;
            }
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

/// Where a tree stands (foot of the trunk) and its scale.
pub type TreeSite = (Vec3, f32);

/// The land use of a tile, if the package has it.
pub fn tile_landuse(pkg: &WorldPackage, key: TileKey) -> Option<(&LandUseTile, u32)> {
    let l = pkg.landuse.as_ref()?;
    Some((l.tiles.get(&key)?, l.cells))
}

/// Tree meshes (trunk and cone crown, crown shades varying per tree).
pub fn tree_arrays(sites: &[TreeSite]) -> MeshArrays {
    let mut m = MeshArrays::default();
    let crowns = [
        lin(0.13, 0.33, 0.15),
        lin(0.11, 0.28, 0.13),
        lin(0.17, 0.36, 0.14),
        lin(0.20, 0.38, 0.17),
    ];
    let trunk = lin(0.35, 0.24, 0.15);
    for &(base, s) in sites {
        let pick = hash(i64::from(base.x.to_bits()), i64::from(base.z.to_bits()), 11);
        let crown = crowns[((pick * crowns.len() as f32) as usize).min(crowns.len() - 1)];
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

/// Where the trees of a tile stand: from the land use if the package has
/// it, else provisional patches.
pub fn tree_sites(pkg: &WorldPackage, tile: &Tile) -> Vec<TreeSite> {
    match tile_landuse(pkg, tile.key) {
        Some((lu, cells)) => landuse_tree_sites(pkg, tile, lu, cells),
        None => placeholder_tree_sites(pkg, tile),
    }
}

/// Chance of a tree per 8 m × 8 m cell and the scale range, by land class.
pub fn tree_density(class: LandClass) -> (f32, f32, f32) {
    match class {
        LandClass::Forest => (0.85, 0.8, 1.5),
        LandClass::Orchard => (0.45, 0.45, 0.65),
        LandClass::Scrub => (0.3, 0.35, 0.6),
        LandClass::Park => (0.1, 0.7, 1.2),
        LandClass::Residential => (0.06, 0.6, 1.0),
        LandClass::Wetland => (0.05, 0.5, 0.9),
        LandClass::Default | LandClass::Meadow | LandClass::Farmland => (0.012, 0.7, 1.3),
        LandClass::Commercial | LandClass::Industrial => (0.01, 0.6, 1.0),
        LandClass::Vineyard
        | LandClass::Water
        | LandClass::Rock
        | LandClass::Scree
        | LandClass::Glacier
        | LandClass::Bare => (0.0, 1.0, 1.0),
    }
}

/// Land-use cells of a tile where no tree may stand: near the road strips
/// (3 m beyond the edge) and under buildings (plus 1.5 m), also buildings
/// of the neighbouring tiles that reach into this one.
pub fn blocked_cells(pkg: &WorldPackage, tile: &Tile, cells: u32) -> Vec<bool> {
    let size = pkg.tile_size_m;
    let o = tile.key.origin(f64::from(size));
    let (ox, oz) = (o[0] as f32, o[1] as f32);
    let n = cells as usize;
    let cs = size / cells as f32;
    let mut blocked = vec![false; n * n];
    let cell_range = |lo: f32, hi: f32, origin: f32| {
        let a = ((lo - origin) / cs).floor().max(0.0) as usize;
        let b = (((hi - origin) / cs).floor() + 1.0).clamp(0.0, n as f32) as usize;
        a..b
    };
    for r in &tile.roads {
        // A cell is blocked if its centre is within the reach (half a cell
        // diagonal more, so no part of a blocked tree's cell is missed).
        let reach = r.width_m / 2.0 + 3.0 + cs * 0.71;
        for w in r.points.windows(2) {
            let (a, b) = (Vec2::new(w[0][0], w[0][2]), Vec2::new(w[1][0], w[1][2]));
            let ab = b - a;
            for k in cell_range(a.y.min(b.y) - reach, a.y.max(b.y) + reach, oz) {
                for i in cell_range(a.x.min(b.x) - reach, a.x.max(b.x) + reach, ox) {
                    let p = Vec2::new(ox + (i as f32 + 0.5) * cs, oz + (k as f32 + 0.5) * cs);
                    let t = if ab.length_squared() > 0.0 {
                        ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    if p.distance(a + ab * t) <= reach {
                        blocked[k * n + i] = true;
                    }
                }
            }
        }
    }
    if let Some(all) = &pkg.buildings {
        for dz in -1..=1 {
            for dx in -1..=1 {
                let key = TileKey {
                    x: tile.key.x + dx,
                    z: tile.key.z + dz,
                };
                for b in all.get(&key).into_iter().flatten() {
                    let (mut lo, mut hi) =
                        (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY));
                    for p in &b.outline {
                        lo = lo.min(Vec2::from(*p));
                        hi = hi.max(Vec2::from(*p));
                    }
                    for k in cell_range(lo.y - 1.5, hi.y + 1.5, oz) {
                        for i in cell_range(lo.x - 1.5, hi.x + 1.5, ox) {
                            blocked[k * n + i] = true;
                        }
                    }
                }
            }
        }
    }
    blocked
}

/// Terrain height at (x, z) and whether it is a rock face there.
fn ground_and_steep(pkg: &WorldPackage, x: f64, z: f64) -> Option<(f64, bool)> {
    let (y, yx, yz) = (
        pkg.height(x, z)?,
        pkg.height(x + 2.0, z)?,
        pkg.height(x, z + 2.0)?,
    );
    Some((y, ((yx - y).powi(2) + (yz - y).powi(2)).sqrt() / 2.0 > 0.8))
}

/// Trees by land use: a jittered 8 m grid, each candidate kept with the
/// chance of its class ([`tree_density`]), not in blocked cells
/// ([`blocked_cells`]) and not on rock faces.
pub fn landuse_tree_sites(
    pkg: &WorldPackage,
    tile: &Tile,
    lu: &LandUseTile,
    cells: u32,
) -> Vec<TreeSite> {
    let size = f64::from(pkg.tile_size_m);
    let o = tile.key.origin(size);
    let cell = 8.0;
    let per_side = (size / cell) as i64;
    let n = cells as usize;
    let cs = size / f64::from(cells);
    let blocked = blocked_cells(pkg, tile, cells);
    let mut sites = Vec::new();
    for k in 0..per_side {
        for i in 0..per_side {
            let (gx, gz) = (
                i64::from(tile.key.x) * per_side + i,
                i64::from(tile.key.z) * per_side + k,
            );
            let x = o[0] + (i as f64 + f64::from(hash(gx, gz, 2))) * cell;
            let z = o[1] + (k as f64 + f64::from(hash(gx, gz, 3))) * cell;
            let (c, r) = (
                (((x - o[0]) / cs) as usize).min(n - 1),
                (((z - o[1]) / cs) as usize).min(n - 1),
            );
            let (chance, s0, s1) = tree_density(lu.classes[r * n + c]);
            if hash(gx, gz, 1) >= chance || blocked[r * n + c] {
                continue;
            }
            let Some((y, too_steep)) = ground_and_steep(pkg, x, z) else {
                continue;
            };
            if too_steep {
                continue;
            }
            let s = s0 + (s1 - s0) * hash(gx, gz, 4);
            sites.push((Vec3::new(x as f32, y as f32 - 0.3, z as f32), s));
        }
    }
    sites
}

/// Provisional vegetation (packages without land use): trees on a
/// jittered grid in patches, not on steep rock and never on a road.
fn placeholder_tree_sites(pkg: &WorldPackage, tile: &Tile) -> Vec<TreeSite> {
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
            let Some((y, too_steep)) = ground_and_steep(pkg, x, z) else {
                continue;
            };
            if too_steep {
                continue; // rock faces
            }
            let s = 0.7 + 0.7 * hash(gx, gz, 4);
            let base = Vec3::new(x as f32, y as f32 - 0.3, z as f32);
            sites.push((base, s));
        }
    }
    sites
}

/// Ground colour of a land class (sRGB).
pub fn land_colour(class: LandClass) -> [f32; 3] {
    match class {
        LandClass::Default => [0.36, 0.53, 0.26],
        LandClass::Meadow => [0.40, 0.58, 0.27],
        LandClass::Farmland => [0.56, 0.59, 0.31],
        LandClass::Forest => [0.21, 0.33, 0.16],
        LandClass::Orchard => [0.38, 0.52, 0.25],
        LandClass::Vineyard => [0.50, 0.50, 0.30],
        LandClass::Residential => [0.43, 0.52, 0.34],
        LandClass::Commercial => [0.56, 0.55, 0.52],
        LandClass::Industrial => [0.52, 0.51, 0.49],
        LandClass::Park => [0.32, 0.56, 0.24],
        LandClass::Water => [0.20, 0.33, 0.40],
        LandClass::Wetland => [0.36, 0.46, 0.31],
        LandClass::Rock => [0.52, 0.50, 0.47],
        LandClass::Scree => [0.62, 0.60, 0.56],
        LandClass::Glacier => [0.88, 0.92, 0.95],
        LandClass::Bare => [0.66, 0.60, 0.48],
        LandClass::Scrub => [0.31, 0.43, 0.22],
    }
}

const ROCK: [f32; 3] = [0.5, 0.48, 0.44];

/// Colour map of a tile: one texel per land-use cell, the class colour
/// turning to rock on steep slopes, with a little per-texel and per-patch
/// noise; plus its mip chain (box filtered).
pub fn colour_map(pkg: &WorldPackage, tile: &Tile, lu: &LandUseTile, cells: u32) -> ColourMap {
    let n = cells as usize;
    let size = f64::from(pkg.tile_size_m);
    let o = tile.key.origin(size);
    let cs = size / f64::from(cells);
    let mut level0 = Vec::with_capacity(n * n * 4);
    for r in 0..n {
        for c in 0..n {
            let class = lu.classes[r * n + c];
            let (x, z) = (o[0] + (c as f64 + 0.5) * cs, o[1] + (r as f64 + 0.5) * cs);
            let slope = match (
                pkg.height(x - 2.0, z),
                pkg.height(x + 2.0, z),
                pkg.height(x, z - 2.0),
                pkg.height(x, z + 2.0),
            ) {
                (Some(a), Some(b), Some(d), Some(e)) => {
                    (((b - a) / 4.0).powi(2) + ((e - d) / 4.0).powi(2)).sqrt() as f32
                }
                _ => 0.0,
            };
            let mut colour = land_colour(class);
            if !matches!(class, LandClass::Water | LandClass::Glacier) {
                let t = ((slope - 0.6) / 0.4).clamp(0.0, 1.0);
                colour = std::array::from_fn(|j| colour[j] + (ROCK[j] - colour[j]) * t);
            }
            let (gc, gr) = (
                i64::from(tile.key.x) * n as i64 + c as i64,
                i64::from(tile.key.z) * n as i64 + r as i64,
            );
            let shade = 0.93
                + 0.1 * hash(gc, gr, 21)
                + 0.08 * (hash(gc.div_euclid(6), gr.div_euclid(6), 22) - 0.5);
            for ch in colour {
                level0.push(((ch * shade).clamp(0.0, 1.0) * 255.0).round() as u8);
            }
            level0.push(255);
        }
    }
    let mut data = level0.clone();
    let (mut prev, mut w, mut levels) = (level0, n, 1);
    while w > 1 {
        let nw = (w / 2).max(1);
        let mut next = Vec::with_capacity(nw * nw * 4);
        for r in 0..nw {
            for c in 0..nw {
                for ch in 0..4 {
                    let at = |rr: usize, cc: usize| {
                        u32::from(prev[(rr.min(w - 1) * w + cc.min(w - 1)) * 4 + ch])
                    };
                    let sum = at(2 * r, 2 * c)
                        + at(2 * r, 2 * c + 1)
                        + at(2 * r + 1, 2 * c)
                        + at(2 * r + 1, 2 * c + 1);
                    next.push(((sum + 2) / 4) as u8);
                }
            }
        }
        data.extend_from_slice(&next);
        prev = next;
        w = nw;
        levels += 1;
    }
    ColourMap {
        size: cells,
        levels,
        data,
    }
}

/// Flat water surfaces: a quad over every terrain grid cell whose four
/// corners have a water level (the terrain is carved below it).
pub fn water_arrays(pkg: &WorldPackage, tile: &Tile, lu: &LandUseTile) -> MeshArrays {
    let mut m = MeshArrays::default();
    if lu.water.is_empty() {
        return m;
    }
    let n = pkg.samples as usize;
    let s = pkg.spacing_m() as f32;
    let o = tile.key.origin(f64::from(pkg.tile_size_m));
    let colour = lin(0.16, 0.30, 0.37);
    let mut vertex = vec![u32::MAX; n * n];
    for &(idx, level) in &lu.water {
        let (i, k) = (idx as usize % n, idx as usize / n);
        vertex[idx as usize] = m.positions.len() as u32;
        m.positions.push([
            o[0] as f32 + i as f32 * s,
            level,
            o[1] as f32 + k as f32 * s,
        ]);
        m.normals.push([0.0, 1.0, 0.0]);
        m.colors.push(colour);
    }
    for k in 0..n - 1 {
        for i in 0..n - 1 {
            let (a, b, c, d) = (
                vertex[k * n + i],
                vertex[k * n + i + 1],
                vertex[(k + 1) * n + i],
                vertex[(k + 1) * n + i + 1],
            );
            if [a, b, c, d].contains(&u32::MAX) {
                continue;
            }
            m.indices.extend([a, c, b, b, c, d]);
        }
    }
    m
}

/// Pushes a triangle wound so that it faces along `normal`.
fn push_tri(m: &mut MeshArrays, p: [Vec3; 3], normal: Vec3, colour: [f32; 4]) {
    let b = m.positions.len() as u32;
    let facing = (p[1] - p[0]).cross(p[2] - p[0]).dot(normal) >= 0.0;
    for q in p {
        m.positions.push(q.to_array());
        m.normals.push(normal.to_array());
        m.colors.push(colour);
    }
    if facing {
        m.indices.extend([b, b + 1, b + 2]);
    } else {
        m.indices.extend([b, b + 2, b + 1]);
    }
}

fn push_quad(m: &mut MeshArrays, p: [Vec3; 4], normal: Vec3, colour: [f32; 4]) {
    push_tri(m, [p[0], p[1], p[2]], normal, colour);
    push_tri(m, [p[0], p[2], p[3]], normal, colour);
}

/// Wall and roof colours of a building (sRGB), varied per building.
pub fn building_colours(b: &Building) -> ([f32; 3], [f32; 3]) {
    let p = b.outline[0];
    let pick = |seed: u64, n: usize| {
        let h = hash(i64::from(p[0].to_bits()), i64::from(p[1].to_bits()), seed);
        ((h * n as f32) as usize).min(n - 1)
    };
    let plaster = [
        [0.93, 0.91, 0.86],
        [0.95, 0.93, 0.88],
        [0.90, 0.85, 0.76],
        [0.92, 0.89, 0.82],
        [0.86, 0.84, 0.80],
    ];
    let wall = match b.kind {
        BuildingKind::House | BuildingKind::Other => plaster[pick(31, plaster.len())],
        BuildingKind::Apartments => [0.86, 0.84, 0.80],
        BuildingKind::Commercial => [0.78, 0.79, 0.80],
        BuildingKind::Industrial => [0.66, 0.67, 0.68],
        BuildingKind::Farm => [0.52, 0.38, 0.26],
        BuildingKind::Religious => [0.96, 0.95, 0.92],
        BuildingKind::Public => [0.87, 0.83, 0.74],
        BuildingKind::Small => [0.58, 0.48, 0.38],
    };
    let tiles = [
        [0.55, 0.24, 0.17],
        [0.47, 0.22, 0.16],
        [0.34, 0.32, 0.32],
        [0.60, 0.30, 0.20],
    ];
    let roof = match b.roof {
        Roof::Gabled { .. } => tiles[pick(32, tiles.len())],
        Roof::Flat => [0.42, 0.42, 0.41],
    };
    (wall, roof)
}

/// Building meshes of a tile: walls from the base to the eaves around the
/// outline and the courtyards, a flat roof (triangulated) or a gable roof
/// with its ridge along the first edge.
pub fn building_arrays(buildings: &[Building]) -> MeshArrays {
    let mut m = MeshArrays::default();
    for b in buildings {
        let (wall, roof) = building_colours(b);
        let (wall, roof) = (
            lin(wall[0], wall[1], wall[2]),
            lin(roof[0], roof[1], roof[2]),
        );
        let at = |p: [f32; 2], y: f32| Vec3::new(p[0], y, p[1]);
        for ring in std::iter::once(&b.outline).chain(&b.holes) {
            let n = ring.len();
            for i in 0..n {
                let (a, c) = (ring[i], ring[(i + 1) % n]);
                let normal = Vec3::new(c[1] - a[1], 0.0, a[0] - c[0]).normalize_or_zero();
                if normal == Vec3::ZERO {
                    continue;
                }
                push_quad(
                    &mut m,
                    [
                        at(a, b.base_y),
                        at(c, b.base_y),
                        at(c, b.eave_y),
                        at(a, b.eave_y),
                    ],
                    normal,
                    wall,
                );
            }
        }
        match b.roof {
            Roof::Flat => {
                let pts: Vec<[f32; 2]> = b
                    .outline
                    .iter()
                    .chain(b.holes.iter().flatten())
                    .copied()
                    .collect();
                for t in triangulate(&b.outline, &b.holes) {
                    push_tri(
                        &mut m,
                        t.map(|j| at(pts[j as usize], b.eave_y)),
                        Vec3::Y,
                        roof,
                    );
                }
            }
            Roof::Gabled { height_m } => {
                let c: Vec<Vec3> = b.outline.iter().map(|&p| at(p, b.eave_y)).collect();
                let up = Vec3::Y * height_m;
                let (r0, r1) = ((c[3] + c[0]) / 2.0 + up, (c[1] + c[2]) / 2.0 + up);
                // Two roof planes (facing up and out over their long side) …
                for (q, out) in [
                    ([c[0], c[1], r1, r0], c[0] - c[3]),
                    ([c[2], c[3], r0, r1], c[3] - c[0]),
                ] {
                    let normal = (q[1] - q[0]).cross(q[3] - q[0]).normalize_or_zero();
                    let normal = if normal.dot(out.with_y(0.0) + Vec3::Y) < 0.0 {
                        -normal
                    } else {
                        normal
                    };
                    push_quad(&mut m, q, normal, roof);
                }
                // … and the gable triangles above the short walls.
                for (tri, out) in [
                    ([c[1], c[2], r1], c[1] - c[0]),
                    ([c[3], c[0], r0], c[0] - c[1]),
                ] {
                    push_tri(&mut m, tri, out.with_y(0.0).normalize_or_zero(), wall);
                }
            }
        }
    }
    m
}

/// The buildings as camera-solid prisms: the footprint up to the ridge
/// (gable roofs) or the flat roof.
pub fn building_footprints(buildings: &[Building]) -> SolidFootprints {
    SolidFootprints::new(buildings.iter().map(|b| {
        let top = match b.roof {
            Roof::Flat => b.eave_y,
            Roof::Gabled { height_m } => b.eave_y + height_m,
        };
        (b.outline.clone(), top)
    }))
}

pub fn build_tile_meshes(pkg: &WorldPackage, key: TileKey) -> Option<TileMeshes> {
    let tile = pkg.tiles.get(&key)?;
    let sites = tree_sites(pkg, tile);
    let lu = tile_landuse(pkg, key);
    let tile_buildings = pkg.buildings.as_ref().and_then(|b| b.get(&key));
    let buildings = tile_buildings.map_or_else(MeshArrays::default, |b| building_arrays(b));
    let solid = tile_buildings.map_or_else(SolidFootprints::default, |b| building_footprints(b));
    Some(TileMeshes {
        key,
        terrain: terrain_arrays(pkg, tile),
        colour_map: lu.map(|(lu, cells)| colour_map(pkg, tile, lu, cells)),
        roads: road_arrays(&tile.roads),
        water: lu.map_or_else(MeshArrays::default, |(lu, _)| water_arrays(pkg, tile, lu)),
        trees: tree_arrays(&sites),
        buildings,
        solid,
    })
}

#[derive(Resource)]
struct PackageMaterials {
    /// Terrain and roads (camera-solid).
    vertex_coloured: Handle<StandardMaterial>,
    /// Trees: the same, but see-through for the camera (they fade out where
    /// they block the view instead of moving it).
    trees: Handle<SeeThroughMaterial>,
    water: Handle<StandardMaterial>,
}

fn setup_materials(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut tree_materials: ResMut<Assets<SeeThroughMaterial>>,
) {
    let vertex_coloured = StandardMaterial {
        base_color: Color::WHITE,
        perceptual_roughness: 1.0,
        ..default()
    };
    commands.insert_resource(PackageMaterials {
        trees: tree_materials.add(see_through(vertex_coloured.clone())),
        vertex_coloured: materials.add(vertex_coloured),
        water: materials.add(StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.08,
            reflectance: 0.6,
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
    mut obstacles: ResMut<CameraObstacles>,
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
            obstacles.remove_tile((key.x, key.z));
        }
    }
}

fn upload_tiles(
    mut commands: Commands,
    mut streamer: ResMut<TileStreamer>,
    materials: Res<PackageMaterials>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut standard: ResMut<Assets<StandardMaterial>>,
    mut obstacles: ResMut<CameraObstacles>,
) {
    let Some(mut t) = streamer.ready.pop_front() else {
        return;
    };
    // Buildings are camera-solid: the camera stays out of and above them.
    let tile = (t.key.x, t.key.z);
    obstacles.remove_tile(tile);
    let solid = std::mem::take(&mut t.solid);
    if !solid.is_empty() {
        obstacles.insert(
            ObstacleLayer {
                kind: "buildings",
                tile,
            },
            move |x, z| solid.top_at(x, z),
        );
    }
    let mat = materials.vertex_coloured.clone();
    // The tile's own terrain material with its land-use colour map (freed
    // with the tile's entities).
    let terrain_mat = match t.colour_map.take() {
        Some(map) => standard.add(StandardMaterial {
            base_color: Color::WHITE,
            base_color_texture: Some(images.add(map.image())),
            perceptual_roughness: 1.0,
            ..default()
        }),
        None => mat.clone(),
    };
    let parent = commands
        .spawn((
            Transform::default(),
            Visibility::default(),
            Name::new(format!("tile {} {}", t.key.x, t.key.z)),
        ))
        .with_children(|c| {
            c.spawn((
                Mesh3d(meshes.add(t.terrain.mesh())),
                MeshMaterial3d(terrain_mat),
                NotShadowCaster,
                CameraInteraction::Solid,
            ));
            if !t.water.indices.is_empty() {
                c.spawn((
                    Mesh3d(meshes.add(t.water.mesh())),
                    MeshMaterial3d(materials.water.clone()),
                    NotShadowCaster,
                ));
            }
            if !t.buildings.indices.is_empty() {
                c.spawn((
                    Mesh3d(meshes.add(t.buildings.mesh())),
                    MeshMaterial3d(mat.clone()),
                    CameraInteraction::Solid,
                ));
            }
            if !t.roads.indices.is_empty() {
                c.spawn((
                    Mesh3d(meshes.add(t.roads.mesh())),
                    MeshMaterial3d(mat),
                    NotShadowCaster,
                    CameraInteraction::Solid,
                ));
            }
            if !t.trees.indices.is_empty() {
                c.spawn((
                    Mesh3d(meshes.add(t.trees.mesh())),
                    MeshMaterial3d(materials.trees.clone()),
                    CameraInteraction::SeeThrough,
                ));
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
            landuse: None,
            buildings: None,
            routes: Vec::new(),
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
        let a = tree_arrays(&tree_sites(&p, tile));
        assert_eq!(a.positions, tree_arrays(&tree_sites(&p, tile)).positions);
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
        // Without land use: vertex colours, no colour map, no water.
        let m = build_tile_meshes(&p, TileKey { x: 0, z: 0 }).unwrap();
        assert!(m.colour_map.is_none() && m.water.indices.is_empty());
        assert_ne!(m.terrain.colors[0], [1.0; 4]);
    }

    /// The test tile (100 m, 5 m land-use cells): forest for x < 50, a
    /// pond at x 70…90, z 0…40 (water levels at its vertices), meadow
    /// elsewhere; a building in the forest.
    fn landuse_package() -> WorldPackage {
        let mut p = package();
        let key = TileKey { x: 0, z: 0 };
        let cells = 20u32;
        let classes = (0..400)
            .map(|j| {
                let (c, r) = (j % 20, j / 20);
                if (14..18).contains(&c) && r < 8 {
                    LandClass::Water
                } else if c < 10 {
                    LandClass::Forest
                } else {
                    LandClass::Meadow
                }
            })
            .collect();
        let mut water = Vec::new();
        for k in 0..=4u32 {
            for i in 6..=10u32 {
                water.push((k * 11 + i, 3.0));
            }
        }
        p.landuse = Some(ocw_world::landuse::LandUse {
            cells,
            tiles: BTreeMap::from([(key, LandUseTile { classes, water })]),
        });
        p.buildings = Some(BTreeMap::from([(
            key,
            vec![Building {
                kind: BuildingKind::House,
                outline: vec![[10.0, 70.0], [22.0, 70.0], [22.0, 78.0], [10.0, 78.0]],
                holes: vec![],
                base_y: 0.5,
                eave_y: 7.5,
                roof: Roof::Gabled { height_m: 3.0 },
            }],
        )]));
        p
    }

    #[test]
    fn land_use_places_trees_colours_the_terrain_and_draws_water() {
        let p = landuse_package();
        let key = TileKey { x: 0, z: 0 };
        let tile = &p.tiles[&key];
        let sites = tree_sites(&p, tile);
        let class = |x: f32, z: f32| {
            p.landuse
                .as_ref()
                .unwrap()
                .class_at(100.0, f64::from(x), f64::from(z))
                .unwrap()
        };
        let forest = sites
            .iter()
            .filter(|(b, _)| class(b.x, b.z) == LandClass::Forest)
            .count();
        let other = sites.len() - forest;
        assert!(forest > 20, "{forest} forest trees");
        assert!(other <= 3, "{other} trees outside the forest");
        for (b, _) in &sites {
            assert_ne!(class(b.x, b.z), LandClass::Water);
            assert!((b.z - 50.0).abs() >= 6.0, "tree on the road at {b}");
            let in_house = (8.5..23.5).contains(&b.x) && (68.5..79.5).contains(&b.z);
            assert!(!in_house, "tree in the building at {b}");
        }
        let m = build_tile_meshes(&p, key).unwrap();
        assert_eq!(m.trees.positions.len(), sites.len() * 14);
        // The building is camera-solid, the forest is not.
        assert_eq!(m.solid.len(), 1);
        let roof = m.solid.top_at(16.0, 74.0).expect("solid building");
        assert_eq!(roof, 10.5, "ridge height");
        let (tree, _) = sites[0];
        assert_eq!(m.solid.top_at(f64::from(tree.x), f64::from(tree.z)), None);
        // Textured terrain: white vertices with texture coordinates.
        assert!(m.terrain.colors.iter().all(|c| *c == [1.0; 4]));
        assert_eq!(m.terrain.uvs.len(), m.terrain.positions.len());
        assert_eq!(m.terrain.uvs[120], [1.0, 1.0]);
        // Colour map: 20 × 20 with mips 10, 5, 2, 1.
        let map = m.colour_map.unwrap();
        assert_eq!((map.size, map.levels), (20, 5));
        assert_eq!(map.data.len(), (400 + 100 + 25 + 4 + 1) * 4);
        let texel = |c: usize, r: usize| &map.data[(r * 20 + c) * 4..(r * 20 + c) * 4 + 3];
        // Forest is darker than meadow; the pond is blue.
        let sum = |t: &[u8]| t.iter().map(|&v| u32::from(v)).sum::<u32>();
        assert!(sum(texel(2, 15)) < sum(texel(12, 15)));
        let pond = texel(15, 2);
        assert!(pond[2] > pond[0]);
        // Water: 4 × 4 grid cells, flat at 3 m.
        assert_eq!(m.water.indices.len(), 16 * 6);
        assert!(m.water.positions.iter().all(|q| q[1] == 3.0));
        for tri in m.water.indices.chunks(3) {
            let [a, b, c] = [0, 1, 2].map(|j| Vec3::from(m.water.positions[tri[j] as usize]));
            assert!((b - a).cross(c - a).y > 0.0);
        }
        assert!(!m.buildings.indices.is_empty());
    }

    /// Every triangle winds towards its normal, and that normal points
    /// away from the building's centre (a convex building).
    fn assert_outward(m: &MeshArrays, centre: Vec3) {
        for tri in m.indices.chunks(3) {
            let [a, b, c] = [0, 1, 2].map(|j| Vec3::from(m.positions[tri[j] as usize]));
            let n = Vec3::from(m.normals[tri[0] as usize]);
            let geometric = (b - a).cross(c - a);
            assert!(geometric.dot(n) > 0.0, "winding against the normal at {a}");
            assert!(
                n.dot((a + b + c) / 3.0 - centre) > 0.0,
                "inward face at {a}"
            );
        }
    }

    #[test]
    fn buildings_are_closed_boxes_with_flat_or_gable_roofs() {
        let flat = Building {
            kind: BuildingKind::Industrial,
            outline: vec![[0.0, 0.0], [20.0, 0.0], [20.0, 10.0], [0.0, 10.0]],
            holes: vec![],
            base_y: 99.5,
            eave_y: 108.0,
            roof: Roof::Flat,
        };
        let m = building_arrays(std::slice::from_ref(&flat));
        // 4 walls × 2 triangles + 2 roof triangles.
        assert_eq!(m.indices.len(), (8 + 2) * 3);
        assert_outward(&m, Vec3::new(10.0, 104.0, 5.0));
        let top = m.positions.iter().map(|p| p[1]).fold(f32::MIN, f32::max);
        assert_eq!(top, 108.0);
        let gabled = Building {
            roof: Roof::Gabled { height_m: 4.0 },
            kind: BuildingKind::House,
            ..flat.clone()
        };
        let m = building_arrays(std::slice::from_ref(&gabled));
        // Walls 8, roof planes 4, gables 2.
        assert_eq!(m.indices.len(), (8 + 4 + 2) * 3);
        assert_outward(&m, Vec3::new(10.0, 104.0, 5.0));
        // The ridge runs along x (the first edge) at the eaves + 4 m.
        let ridge: Vec<[f32; 3]> = m
            .positions
            .iter()
            .filter(|p| p[1] == 112.0)
            .copied()
            .collect();
        assert!(!ridge.is_empty() && ridge.iter().all(|p| p[2] == 5.0));
        // A courtyard: its walls face into the courtyard, the roof leaves it open.
        let court = Building {
            holes: vec![vec![[5.0, 3.0], [5.0, 7.0], [15.0, 7.0], [15.0, 3.0]]],
            ..flat
        };
        let m = building_arrays(&[court]);
        assert_eq!(m.indices.len(), (8 + 8 + 8) * 3);
        let roof_area: f32 = m
            .indices
            .chunks(3)
            .filter(|t| m.normals[t[0] as usize] == [0.0, 1.0, 0.0])
            .map(|t| {
                let [a, b, c] = [0, 1, 2].map(|j| Vec3::from(m.positions[t[j] as usize]));
                (b - a).cross(c - a).length() / 2.0
            })
            .sum();
        assert!((roof_area - 160.0).abs() < 1e-3, "{roof_area}");
    }

    /// Tile meshing time on a real package (not run by default):
    /// `OCW_PACKAGE=data/pilot/liechtenstein-landuse.ocwpkg cargo test
    /// --release -p ocw-render tile_build_time -- --ignored --nocapture`
    #[test]
    #[ignore = "needs a world package"]
    fn tile_build_time() {
        let path = std::env::var("OCW_PACKAGE").expect("OCW_PACKAGE");
        let p = WorldPackage::from_bytes(&std::fs::read(path).unwrap()).unwrap();
        let mut keys: Vec<(usize, TileKey)> = p
            .tiles
            .keys()
            .map(|k| {
                let n = p
                    .buildings
                    .as_ref()
                    .and_then(|b| b.get(k))
                    .map_or(0, Vec::len);
                (n, *k)
            })
            .collect();
        keys.sort();
        let (mut worst, mut total, mut trees, mut verts) = (0.0f64, 0.0, 0, 0);
        for &(_, key) in keys.iter().rev().take(25) {
            let t = std::time::Instant::now();
            let m = build_tile_meshes(&p, key).unwrap();
            let s = t.elapsed().as_secs_f64();
            worst = worst.max(s);
            total += s;
            trees += m.trees.positions.len() / 14;
            verts +=
                m.trees.positions.len() + m.buildings.positions.len() + m.terrain.positions.len();
        }
        println!(
            "25 busiest tiles: {:.1} ms mean, {:.1} ms worst, {trees} trees, {verts} vertices",
            total / 25.0 * 1e3,
            worst * 1e3
        );
    }
}
