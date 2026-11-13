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
//!
//! **Neighbouring packages** (spec §2.2.2): the resource holds a
//! [`WorldSet`] – the home package and its installed neighbours. Tiles of
//! every package with loaded tiles are streamed around the rider, each
//! placed with its package's [`ocw_world::Frame`] (a shift and an x scale
//! into the home metres). Where packages overlap, a package draws only the
//! terrain cells, water, trees and buildings on ground it owns
//! ([`WorldSet::draws`]), so the border has no double terrain. A
//! neighbour's tiles are loaded when the rider comes near it and dropped
//! again far away ([`WorldSet::plan_residency`]); the loading itself (file
//! access, signature check) is the app's [`PackageLoader`], run on a
//! background task.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::light::NotShadowCaster;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages,
};
use bevy::tasks::{AsyncComputeTaskPool, Task, block_on, futures::check_ready};
use ocw_world::buildings::{Building, BuildingKind, Roof, triangulate};
use ocw_world::graph::RoadClass;
use ocw_world::landuse::{LandClass, LandUseTile};
use ocw_world::package::{RoadStrip, Tile, TileKey, WorldPackage};
use ocw_world::world_set::Residency;
use ocw_world::{Frame, RoadSurface, WorldSet};

use crate::RideView;
use crate::camera::{CameraInteraction, CameraObstacles, ObstacleLayer, SolidFootprints};
use crate::ground::{TerrainMaterial, terrain_material};
use crate::materials::MaterialLibrary;
use crate::road_material::{RoadExtension, RoadMaterial};
use crate::see_through::{SeeThroughMaterial, see_through};
use crate::tunnel_cut::TunnelCut;

/// Tiles kept around the rider (radius in tiles; 2 = 5 × 5).
pub const STREAM_RADIUS: i32 = 2;
/// The rider moving farther than this between two frames is a jump
/// (teleport, start of a ride): the near tiles are then built and uploaded
/// at once ([`urgent_tiles`]).
pub const JUMP_M: f64 = 200.0;
/// After a jump, the tiles up to this ring around the rider (1 = 3 × 3)
/// are there in the next frame.
pub const URGENT_RING: i32 = 1;
/// Road edge faces reach this far down (covers the terrain clearance).
const EDGE_DEPTH_M: f32 = 2.0;

/// Loads the tiles of member `i` of the world set again (verified), for
/// neighbours that come into range. Runs on a background task.
pub type PackageLoader = Arc<dyn Fn(usize) -> Result<WorldPackage, String> + Send + Sync>;

/// The loaded world packages (inserted by the app with `--world`).
#[derive(Resource, Clone)]
pub struct PackageWorld {
    pub set: Arc<WorldSet>,
    /// Without one, neighbours keep the tiles they were added with.
    pub loader: Option<PackageLoader>,
}

impl PackageWorld {
    /// A single package.
    pub fn single(pkg: Arc<WorldPackage>) -> Self {
        Self {
            set: Arc::new(WorldSet::new("", pkg)),
            loader: None,
        }
    }

    pub fn home(&self) -> &Arc<WorldPackage> {
        self.set
            .home()
            .package()
            .expect("the home package stays loaded")
    }

    /// The terrain surface in home metres (camera and riders stand on it).
    pub fn ground(&self) -> crate::Ground {
        let set = self.set.clone();
        crate::Ground(Arc::new(move |x, z| set.surface_height([x, z])))
    }
}

pub struct PackagePlugin;

impl Plugin for PackagePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TileStreamer>()
            .init_resource::<NeighbourLoads>()
            .init_resource::<crate::far_terrain::FarTerrain>()
            .add_systems(Startup, setup_materials)
            .add_systems(
                Update,
                (
                    load_neighbours,
                    stream_tiles,
                    upload_tiles,
                    crate::far_terrain::update_far_terrain,
                    crate::far_terrain::hide_far_under_near,
                )
                    .chain()
                    .run_if(resource_exists::<PackageWorld>),
            )
            .add_systems(Update, crate::far_terrain::package_view);
    }
}

/// Where a tile of a package is drawn: the package's frame as an entity
/// transform (y up stays; x scaled and shifted, z shifted).
pub fn frame_transform(f: &Frame) -> Transform {
    Transform {
        translation: Vec3::new(f.offset[0] as f32, 0.0, f.offset[1] as f32),
        scale: Vec3::new(f.scale_x as f32, 1.0, 1.0),
        ..default()
    }
}

/// Which ground of a tile its package draws, in the package's metres
/// (`None` = all of it).
pub type Keep<'a> = Option<&'a (dyn Fn([f64; 2]) -> bool + Sync)>;

/// Mesh arrays of one tile (built off the main thread).
#[derive(Default)]
pub struct MeshArrays {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub colors: Vec<[f32; 4]>,
    /// Texture coordinates (terrain: across the tile; roads: across and
    /// along in metres; empty otherwise).
    pub uvs: Vec<[f32; 2]>,
    /// Second texture coordinates (roads: material layer, 1 on the edge
    /// faces; empty otherwise).
    pub uvs_b: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

impl MeshArrays {
    /// The material layer of the first vertex (roads; tests).
    pub fn roads_layer(&self) -> Option<f32> {
        self.uvs_b.first().map(|u| u[0])
    }

    pub(crate) fn mesh(self) -> Mesh {
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
        if !self.uvs_b.is_empty() {
            m.insert_attribute(Mesh::ATTRIBUTE_UV_1, self.uvs_b);
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
    /// Splat weights at the cell corners (with a material library).
    pub splat: Option<Splat>,
    pub roads: MeshArrays,
    pub water: MeshArrays,
    pub trees: MeshArrays,
    pub buildings: MeshArrays,
    /// Bridge structures (section `BRDG`, spec §4.8.6).
    pub bridges: MeshArrays,
    /// The buildings as camera-solid obstacles.
    pub solid: SolidFootprints,
}

/// What the tile meshes are built for: with a material library, the far
/// colours are the materials' means and the terrain gets splat weights
/// (spec §4.6, Phase 8 T2); without, the flat land-use colours.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TileLook {
    /// Colour map colour per land class (sRGB), by `LandClass` code.
    pub palette: Option<Vec<[f32; 3]>>,
    /// Road colour per surface (linear), by `RoadSurface` code.
    pub road_colours: Option<Vec<[f32; 3]>>,
    /// The ground-layer mix of each land class (its regional set).
    pub mix: Option<Arc<crate::ground::ClassMix>>,
}

impl TileLook {
    pub fn textured(&self) -> bool {
        self.palette.is_some()
    }

    /// The look for a loaded material library and the ground set of a
    /// package's region (class tint neutral).
    pub fn of_library(lib: &MaterialLibrary, set: &crate::materials::GroundSet) -> Self {
        let palette = crate::ground::class_palette(&set.means, &set.mix, |_| [1.0; 3]);
        Self {
            palette: Some(palette.into_iter().map(|(_, c)| c).collect()),
            road_colours: Some(lib.road_means.to_vec()),
            mix: Some(Arc::new(set.mix)),
        }
    }
}

/// Splat weights of a tile: two RGBA8 planes of `(cells + 1)²` corners.
pub struct Splat {
    pub cells: u32,
    pub a: Vec<[u8; 4]>,
    pub b: Vec<[u8; 4]>,
}

impl Splat {
    fn image(data: Vec<[u8; 4]>, size: u32, label: &'static str) -> Image {
        Image {
            data: Some(data.into_iter().flatten().collect()),
            texture_descriptor: TextureDescriptor {
                label: Some(label),
                size: Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: TextureFormat::Rgba8Unorm,
                usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
                view_formats: &[],
            },
            sampler: ImageSampler::Descriptor(ImageSamplerDescriptor {
                address_mode_u: ImageAddressMode::ClampToEdge,
                address_mode_v: ImageAddressMode::ClampToEdge,
                mag_filter: ImageFilterMode::Linear,
                min_filter: ImageFilterMode::Linear,
                ..default()
            }),
            asset_usage: RenderAssetUsages::RENDER_WORLD,
            ..default()
        }
    }

    /// The two weight images.
    pub fn images(self) -> (Image, Image) {
        let n = self.cells + 1;
        (
            Self::image(self.a, n, "splat weights 0-3"),
            Self::image(self.b, n, "splat weights 4-7"),
        )
    }
}

/// The splat weights of a tile (cell corners, neighbour tiles' cells
/// included so tiles agree on their edges).
pub fn tile_splat(
    pkg: &WorldPackage,
    key: TileKey,
    cells: u32,
    mix: &crate::ground::ClassMix,
) -> Splat {
    let n = i64::from(cells);
    let class = |c: i64, r: i64| {
        let (dx, dz) = (c.div_euclid(n), r.div_euclid(n));
        let k = TileKey {
            x: key.x + dx as i32,
            z: key.z + dz as i32,
        };
        let (lu, kc) = tile_landuse(pkg, k)?;
        (kc == cells).then(|| lu.classes[(r.rem_euclid(n) * n + c.rem_euclid(n)) as usize])
    };
    let (a, b) = crate::ground::splat_corners(cells as usize, mix, class);
    Splat { cells, a, b }
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
    terrain_arrays_kept(pkg, tile, None)
}

/// [`terrain_arrays`] with only the grid cells whose centre `keep` accepts.
pub fn terrain_arrays_kept(pkg: &WorldPackage, tile: &Tile, keep: Keep) -> MeshArrays {
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
            if !cell_kept(keep, o, f64::from(s), i, k) {
                continue;
            }
            let v = |i: usize, k: usize| (k * n + i) as u32;
            let (a, b, c, d) = (v(i, k), v(i + 1, k), v(i, k + 1), v(i + 1, k + 1));
            m.indices.extend([a, c, b, b, c, d]);
        }
    }
    m
}

/// Whether grid cell (i, k) of a tile at `o` with spacing `s` is drawn.
fn cell_kept(keep: Keep, o: [f64; 2], s: f64, i: usize, k: usize) -> bool {
    keep.is_none_or(|f| f([o[0] + (i as f64 + 0.5) * s, o[1] + (k as f64 + 0.5) * s]))
}

/// Flat colour of a road (linear): by category where the surface is the
/// category's default (as before the material library), else by surface.
pub fn road_colour(class: RoadClass, surface: RoadSurface) -> [f32; 4] {
    if surface == RoadSurface::of_class(class) {
        return match class {
            RoadClass::Track => lin(0.55, 0.47, 0.36),
            RoadClass::Path => lin(0.62, 0.56, 0.45),
            RoadClass::Cycleway => lin(0.3, 0.3, 0.33),
            _ => lin(0.22, 0.22, 0.24),
        };
    }
    match surface {
        RoadSurface::Asphalt => lin(0.22, 0.22, 0.24),
        RoadSurface::WornAsphalt => lin(0.3, 0.29, 0.28),
        RoadSurface::Gravel => lin(0.58, 0.53, 0.45),
        RoadSurface::Paving => lin(0.45, 0.42, 0.38),
        RoadSurface::Dirt => lin(0.5, 0.41, 0.31),
    }
}

/// Road surface strips with edge faces, in world coordinates.
pub fn road_arrays(strips: &[RoadStrip]) -> MeshArrays {
    road_arrays_with(strips, None, |_| EDGE_DEPTH_M)
}

/// How far the edge face of a road strip reaches down at `p` (its top edge,
/// spec §4.8.3, §4.8.6): nothing on a bridge deck (the deck has its own
/// sides, a skirt would hang below it); elsewhere at least
/// [`EDGE_DEPTH_M`] and down to 0.3 m below the terrain at the edge, so a
/// road the 10 m terrain grid cannot follow (the upper leg of a hairpin, a
/// road above a lower one) stands on a wall instead of floating.
pub fn skirt_depth(
    pkg: &WorldPackage,
    bridges: &[&ocw_world::bridges::Bridge],
    p: [f32; 3],
) -> f32 {
    if crate::bridges::on_deck(bridges, [p[0], p[1] - crate::ROAD_SURFACE_LIFT_M, p[2]]) {
        return 0.0;
    }
    // No terrain there (beyond the package's tiles): the plain skirt.
    let Some(ground) = pkg.surface_height(f64::from(p[0]), f64::from(p[2])) else {
        return EDGE_DEPTH_M;
    };
    EDGE_DEPTH_M.max(p[1] - ground as f32 + 0.3).min(MAX_WALL_M)
}

/// Deepest retaining wall drawn under a road edge (m).
pub const MAX_WALL_M: f32 = 40.0;

/// [`road_arrays`] with the road colours of a material library (`colours`
/// by surface code, linear; the far colour of the road material) and the
/// depth of the edge faces below each point given by `skirt` (m below the
/// surface).
pub fn road_arrays_with(
    strips: &[RoadStrip],
    colours: Option<&[[f32; 3]]>,
    skirt: impl Fn([f32; 3]) -> f32,
) -> MeshArrays {
    let mut m = MeshArrays::default();
    let edge = lin(0.45, 0.43, 0.38);
    for s in strips {
        let colour = colours
            .and_then(|c| c.get(s.surface.code() as usize))
            .map_or_else(
                || road_colour(s.class, s.surface),
                |c| [c[0], c[1], c[2], 1.0],
            );
        let layer = f32::from(s.surface.code());
        let half = s.width_m / 2.0;
        let p: Vec<Vec3> = s.points.iter().map(|q| Vec3::from(*q)).collect();
        let n = p.len();
        let base = m.positions.len() as u32;
        // Surface: two vertices per point; uv = (across, along) in metres.
        let mut along = 0.0f32;
        for i in 0..n {
            if i > 0 {
                along += p[i].distance(p[i - 1]);
            }
            let dir = (p[(i + 1).min(n - 1)] - p[i.saturating_sub(1)])
                .with_y(0.0)
                .normalize_or_zero();
            let right = Vec3::new(-dir.z, 0.0, dir.x);
            for side in [-half, half] {
                m.positions
                    .push((p[i] + right * side + Vec3::Y * crate::ROAD_SURFACE_LIFT_M).to_array());
                m.normals.push([0.0, 1.0, 0.0]);
                m.colors.push(colour);
                m.uvs.push([side, along]);
                m.uvs_b.push([layer, 0.0]);
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
                let top = p[i] + out * half + Vec3::Y * crate::ROAD_SURFACE_LIFT_M;
                let depth = skirt(top.to_array());
                m.positions.push(top.to_array());
                m.positions.push((top - Vec3::Y * depth).to_array());
                m.normals.extend([out.to_array(); 2]);
                m.colors.extend([edge; 2]);
                m.uvs.extend([[0.0, 0.0]; 2]);
                m.uvs_b.extend([[layer, 1.0]; 2]);
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
    colour_map_with(pkg, tile, lu, cells, None)
}

/// [`colour_map`] with the class colours of `palette` (sRGB by class code;
/// the materials' mean colours, so the far ground matches the near).
pub fn colour_map_with(
    pkg: &WorldPackage,
    tile: &Tile,
    lu: &LandUseTile,
    cells: u32,
    palette: Option<&[[f32; 3]]>,
) -> ColourMap {
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
            let flat = |c: LandClass| {
                palette
                    .and_then(|p| p.get(c as usize).copied())
                    .unwrap_or_else(|| land_colour(c))
            };
            let mut colour = flat(class);
            if !matches!(class, LandClass::Water | LandClass::Glacier) {
                let rock = palette.map_or(ROCK, |_| flat(LandClass::Rock));
                let t = match palette {
                    // As the shader turns steep ground into rock.
                    Some(_) => crate::ground::rock_weight_under(
                        slope,
                        crate::ground::class_weights(class)[crate::ground::FOREST_FLOOR],
                    ),
                    None => ((slope - 0.6) / 0.4).clamp(0.0, 1.0),
                };
                colour = std::array::from_fn(|j| colour[j] + (rock[j] - colour[j]) * t);
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
    water_arrays_kept(pkg, tile, lu, None)
}

/// [`water_arrays`] on the cells `keep` accepts.
pub fn water_arrays_kept(
    pkg: &WorldPackage,
    tile: &Tile,
    lu: &LandUseTile,
    keep: Keep,
) -> MeshArrays {
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
            if [a, b, c, d].contains(&u32::MAX) || !cell_kept(keep, o, f64::from(s), i, k) {
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
    build_tile_meshes_kept(pkg, key, None)
}

/// [`build_tile_meshes`] with only the ground `keep` accepts (package
/// metres): terrain and water cells by their centre, trees by their foot,
/// buildings by the centroid of their outline. Road strips are always
/// drawn (the builder cuts them at the package's border).
pub fn build_tile_meshes_kept(pkg: &WorldPackage, key: TileKey, keep: Keep) -> Option<TileMeshes> {
    build_tile_meshes_look(pkg, key, keep, &TileLook::default())
}

/// [`build_tile_meshes_kept`] for a [`TileLook`].
pub fn build_tile_meshes_look(
    pkg: &WorldPackage,
    key: TileKey,
    keep: Keep,
    look: &TileLook,
) -> Option<TileMeshes> {
    let tile = pkg.tiles.get(&key)?;
    let mut sites = tree_sites(pkg, tile);
    if let Some(f) = keep {
        sites.retain(|(p, _)| f([f64::from(p.x), f64::from(p.z)]));
    }
    let lu = tile_landuse(pkg, key);
    let kept: Option<Vec<Building>> = pkg.buildings.as_ref().and_then(|b| b.get(&key)).map(|b| {
        b.iter()
            .filter(|b| keep.is_none_or(|f| f(outline_centroid(&b.outline))))
            .cloned()
            .collect()
    });
    let buildings = kept
        .as_deref()
        .map_or_else(MeshArrays::default, building_arrays);
    let near = crate::bridges::bridges_near(pkg, key);
    let size = f64::from(pkg.tile_size_m);
    let own: Vec<&ocw_world::bridges::Bridge> = pkg
        .bridges
        .as_ref()
        .and_then(|t| t.get(&key))
        .map(|v| {
            v.iter()
                .filter(|b| {
                    let (mid, _) = b.at(b.length_m() / 2.0);
                    let mid = [f64::from(mid[0]), f64::from(mid[2])];
                    TileKey::containing(mid, size) == key && keep.is_none_or(|f| f(mid))
                })
                .collect()
        })
        .unwrap_or_default();
    let solid = kept
        .as_deref()
        .map_or_else(SolidFootprints::default, building_footprints);
    Some(TileMeshes {
        key,
        terrain: terrain_arrays_kept(pkg, tile, keep),
        colour_map: lu
            .map(|(lu, cells)| colour_map_with(pkg, tile, lu, cells, look.palette.as_deref())),
        splat: lu.filter(|_| look.textured()).map(|(_, cells)| {
            let mix = look
                .mix
                .as_deref()
                .copied()
                .unwrap_or_else(crate::ground::default_mix);
            tile_splat(pkg, key, cells, &mix)
        }),
        roads: road_arrays_with(&tile.roads, look.road_colours.as_deref(), |p| {
            skirt_depth(pkg, &near, p)
        }),
        water: lu.map_or_else(MeshArrays::default, |(lu, _)| {
            water_arrays_kept(pkg, tile, lu, keep)
        }),
        trees: tree_arrays(&sites),
        buildings,
        bridges: crate::bridges::bridge_arrays(&own),
        solid,
    })
}

fn outline_centroid(outline: &[[f32; 2]]) -> [f64; 2] {
    let n = outline.len().max(1) as f64;
    let (x, z) = outline.iter().fold((0.0, 0.0), |(x, z), p| {
        (x + f64::from(p[0]), z + f64::from(p[1]))
    });
    [x / n, z / n]
}

#[derive(Resource)]
pub(crate) struct PackageMaterials {
    /// Roads and buildings (camera-solid; the terrain has its own
    /// [`TerrainMaterial`] per tile).
    vertex_coloured: Handle<StandardMaterial>,
    /// Trees: the same, but see-through for the camera (they fade out where
    /// they block the view instead of moving it).
    trees: Handle<SeeThroughMaterial>,
    pub(crate) water: Handle<StandardMaterial>,
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

/// A tile of member `.0` of the world set.
pub(crate) type TileId = (usize, TileKey);

#[derive(Resource, Default)]
pub(crate) struct TileStreamer {
    pub(crate) loaded: HashMap<TileId, Entity>,
    building: HashMap<TileId, Task<Option<TileMeshes>>>,
    ready: VecDeque<(TileId, TileMeshes)>,
    /// World set generation the loaded tiles were clipped for.
    generation: u64,
    /// The rider's position in the last frame (jump detection).
    last_position: Option<[f64; 2]>,
    /// Tiles at the front of `ready` to upload in this frame (after a jump).
    urgent: usize,
    /// Whether the loaded tiles were built with the material library.
    textured: bool,
}

/// Neighbour tiles being loaded.
#[derive(Resource, Default)]
struct NeighbourLoads {
    tasks: HashMap<usize, Task<Result<WorldPackage, String>>>,
}

/// Drops all streamed tiles and pending loads (a ride switch,
/// [`crate::rebuild_scene`]): the next frames stream the new world set.
pub(crate) fn reset_streaming(world: &mut World) {
    let loaded: Vec<Entity> = world
        .get_resource_mut::<TileStreamer>()
        .map(|mut s| {
            let s = std::mem::take(&mut *s);
            s.loaded.into_values().collect()
        })
        .unwrap_or_default();
    for e in loaded {
        if let Ok(e) = world.get_entity_mut(e) {
            e.despawn();
        }
    }
    if let Some(mut l) = world.get_resource_mut::<NeighbourLoads>() {
        l.tasks.clear();
    }
    if let Some(mut far) = world.remove_resource::<crate::far_terrain::FarTerrain>() {
        far.clear(world);
        world.insert_resource(far);
    }
}

/// The local rider's position in home metres.
fn rider_position(view: &RideView) -> Option<[f64; 2]> {
    let local = view.local()?;
    let p = view.track(local.id).sample(local.state.distance).position;
    Some([p[0], p[2]])
}

/// The tiles to show around `p` (home metres), nearest first: of every
/// package with loaded tiles, those within [`STREAM_RADIUS`].
pub fn wanted_tiles(set: &WorldSet, p: [f64; 2]) -> Vec<TileId> {
    wanted_tile_rings(set, p)
        .into_iter()
        .map(|(_, id)| id)
        .collect()
}

/// The tiles [`urgent_tiles`] builds at once after a jump: of the
/// [`wanted_tiles`], those up to [`URGENT_RING`], nearest first.
pub fn urgent_tiles(set: &WorldSet, p: [f64; 2]) -> Vec<TileId> {
    wanted_tile_rings(set, p)
        .into_iter()
        .filter(|&(ring, _)| ring <= URGENT_RING)
        .map(|(_, id)| id)
        .collect()
}

/// Whether the rider moved from `last` to `p` by a jump ([`JUMP_M`]); the
/// first position (no `last`) is one too.
pub fn is_jump(last: Option<[f64; 2]>, p: [f64; 2]) -> bool {
    last.is_none_or(|q| (p[0] - q[0]).hypot(p[1] - q[1]) > JUMP_M)
}

/// [`wanted_tiles`] with their ring around the rider.
fn wanted_tile_rings(set: &WorldSet, p: [f64; 2]) -> Vec<(i32, TileId)> {
    let mut out: Vec<(i32, TileId)> = Vec::new();
    for (i, m) in set.members().iter().enumerate() {
        let Some(pkg) = m.package() else {
            continue;
        };
        let centre = TileKey::containing(m.frame.invert(p), f64::from(pkg.tile_size_m));
        for dz in -STREAM_RADIUS..=STREAM_RADIUS {
            for dx in -STREAM_RADIUS..=STREAM_RADIUS {
                let key = TileKey {
                    x: centre.x + dx,
                    z: centre.z + dz,
                };
                if pkg.tiles.contains_key(&key) {
                    out.push((key.ring(centre), (i, key)));
                }
            }
        }
    }
    out.sort_by_key(|&(ring, (i, _))| (ring, i));
    out
}

/// Tiles to drop: of packages without loaded tiles, or too far away.
pub fn far_tiles<'a>(
    set: &WorldSet,
    p: [f64; 2],
    loaded: impl IntoIterator<Item = &'a TileId>,
) -> Vec<TileId> {
    loaded
        .into_iter()
        .filter(|(i, key)| {
            let m = &set.members()[*i];
            match m.package() {
                None => true,
                Some(pkg) => {
                    let centre = TileKey::containing(m.frame.invert(p), f64::from(pkg.tile_size_m));
                    key.ring(centre) > STREAM_RADIUS + 1
                }
            }
        })
        .copied()
        .collect()
}

fn build_task(set: &Arc<WorldSet>, (i, key): TileId, look: &TileLook) -> Task<Option<TileMeshes>> {
    let set = set.clone();
    let look = look.clone();
    AsyncComputeTaskPool::get().spawn(async move {
        let pkg = set.members()[i].package()?.clone();
        if set.has_seams() {
            let keep = |q: [f64; 2]| set.draws(i, q);
            build_tile_meshes_look(&pkg, key, Some(&keep), &look)
        } else {
            build_tile_meshes_look(&pkg, key, None, &look)
        }
    })
}

/// Loads and drops neighbour tiles by the rider's distance (spec §2.2.2).
fn load_neighbours(
    mut commands: Commands,
    mut world: ResMut<PackageWorld>,
    view: Res<RideView>,
    mut loads: ResMut<NeighbourLoads>,
) {
    if world.set.len() < 2 {
        return;
    }
    let Some(p) = rider_position(&view) else {
        return;
    };
    let mut changed = false;
    // Finished loads.
    let done: Vec<usize> = loads.tasks.keys().copied().collect();
    for i in done {
        let task = loads.tasks.get_mut(&i).expect("key exists");
        if let Some(result) = check_ready(task) {
            loads.tasks.remove(&i);
            match result {
                Ok(pkg) => {
                    info!("world: tiles of {} loaded", world.set.members()[i].id);
                    Arc::make_mut(&mut world.set).set_resident(i, Some(Arc::new(pkg)));
                    changed = true;
                }
                Err(e) => warn!("world: {e}"),
            }
        }
    }
    let loading: BTreeSet<usize> = loads.tasks.keys().copied().collect();
    for r in world.set.plan_residency(p, &loading) {
        match r {
            Residency::Load(i) => {
                let Some(loader) = world.loader.clone() else {
                    continue;
                };
                info!("world: loading the tiles of {}", world.set.members()[i].id);
                loads.tasks.insert(
                    i,
                    AsyncComputeTaskPool::get().spawn(async move { loader(i) }),
                );
            }
            Residency::Unload(i) => {
                // Without a loader they could not come back.
                if world.loader.is_none() {
                    continue;
                }
                info!("world: dropping the tiles of {}", world.set.members()[i].id);
                Arc::make_mut(&mut world.set).set_resident(i, None);
                changed = true;
            }
        }
    }
    if changed {
        commands.insert_resource(world.ground());
    }
}

fn stream_tiles(
    mut commands: Commands,
    world: Res<PackageWorld>,
    view: Res<RideView>,
    mut streamer: ResMut<TileStreamer>,
    mut obstacles: ResMut<CameraObstacles>,
    library: Option<Res<MaterialLibrary>>,
) {
    let set = &world.set;
    let Some(p) = rider_position(&view) else {
        return;
    };
    // The look of each member package: the ground set of its region.
    let looks: Vec<TileLook> = set
        .members()
        .iter()
        .map(|m| {
            library.as_deref().map_or_else(TileLook::default, |lib| {
                let tz = m.package().and_then(|p| p.time_zone.as_deref());
                TileLook::of_library(lib, lib.set_for(tz))
            })
        })
        .collect();
    let look = move |i: usize| looks.get(i).cloned().unwrap_or_default();
    let textured = library.is_some();
    // Packages came or went, or the material library arrived: the tiles
    // are built anew (the old meshes stay until the new ones are uploaded).
    // Tasks for the old set are discarded.
    if streamer.generation != set.generation() || streamer.textured != textured {
        streamer.generation = set.generation();
        streamer.textured = textured;
        streamer.building.clear();
        let loaded: Vec<TileId> = streamer.loaded.keys().copied().collect();
        for id in loaded {
            if set.members()[id.0].is_resident() {
                streamer
                    .building
                    .insert(id, build_task(set, id, &look(id.0)));
            }
        }
    }
    let jumped = is_jump(streamer.last_position, p);
    streamer.last_position = Some(p);
    let wanted = wanted_tiles(set, p);
    if jumped {
        // Work for the old place is dropped (its tiles go below).
        streamer.building.retain(|id, _| wanted.contains(id));
        streamer.ready.retain(|(id, _)| wanted.contains(id));
    }
    for &id in &wanted {
        if streamer.loaded.contains_key(&id) || streamer.building.contains_key(&id) {
            continue;
        }
        streamer
            .building
            .insert(id, build_task(set, id, &look(id.0)));
    }
    if jumped {
        // A teleport (or the start): the 3 × 3 tiles around the rider are
        // built now – their tasks run in parallel on the task pool, this
        // frame waits for them – and uploaded in this frame, so the ground
        // is there in the first frame at the new place instead of tile by
        // tile over half a second (spec §4.5).
        let urgent: Vec<TileId> = urgent_tiles(set, p)
            .into_iter()
            .filter(|id| !streamer.loaded.contains_key(id))
            .collect();
        let mut done = Vec::new();
        for id in urgent {
            if let Some(task) = streamer.building.remove(&id)
                && let Some(meshes) = block_on(task)
            {
                done.push((id, meshes));
            }
        }
        streamer
            .ready
            .retain(|(id, _)| !done.iter().any(|(d, _)| d == id));
        streamer.urgent = done.len();
        for t in done.into_iter().rev() {
            streamer.ready.push_front(t);
        }
    }
    let ids: Vec<TileId> = streamer.building.keys().copied().collect();
    for id in ids {
        let task = streamer.building.get_mut(&id).expect("key exists");
        if let Some(result) = check_ready(task) {
            streamer.building.remove(&id);
            if let Some(meshes) = result {
                streamer.ready.push_back((id, meshes));
            }
        }
    }
    let far = far_tiles(set, p, streamer.loaded.keys());
    for id in far {
        if let Some(e) = streamer.loaded.remove(&id) {
            commands.entity(e).despawn();
            obstacles.remove_tile(id.0 as u32, (id.1.x, id.1.z));
        }
        streamer.building.remove(&id);
        streamer.ready.retain(|(r, _)| *r != id);
    }
}

/// The material library and the road material made from it.
type RoadParams<'w, 's> = (
    Option<Res<'w, MaterialLibrary>>,
    Option<ResMut<'w, Assets<RoadMaterial>>>,
    Local<'s, Option<Handle<RoadMaterial>>>,
);

#[allow(clippy::too_many_arguments)] // a Bevy system
fn upload_tiles(
    mut commands: Commands,
    world: Res<PackageWorld>,
    mut streamer: ResMut<TileStreamer>,
    materials: Res<PackageMaterials>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut terrain: ResMut<Assets<TerrainMaterial>>,
    mut obstacles: ResMut<CameraObstacles>,
    tunnels: Option<Res<crate::tunnel::Tunnels>>,
    (library, mut road_materials, mut textured_roads): RoadParams,
) {
    // The shared road material of the library (made once).
    if textured_roads.is_none()
        && let Some(lib) = library.as_deref()
        && let Some(rm) = road_materials.as_deref_mut()
    {
        *textured_roads = Some(rm.add(RoadMaterial {
            base: StandardMaterial {
                base_color: Color::WHITE,
                perceptual_roughness: 1.0,
                ..default()
            },
            extension: RoadExtension::of_library(lib),
        }));
    }
    let library = library.as_deref();
    let no_cut = TunnelCut::default();
    let cut = tunnels.as_deref().map_or(&no_cut, |t| &t.cut);
    // One tile per frame, all urgent ones after a jump.
    let count = std::mem::take(&mut streamer.urgent).max(1);
    for _ in 0..count {
        let Some((id, t)) = streamer.ready.pop_front() else {
            return;
        };
        upload_tile(
            &mut commands,
            &world,
            &mut streamer,
            &materials,
            (&mut meshes, &mut images, &mut terrain),
            &mut obstacles,
            cut,
            (library, textured_roads.as_ref()),
            id,
            t,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn upload_tile(
    commands: &mut Commands,
    world: &PackageWorld,
    streamer: &mut TileStreamer,
    materials: &PackageMaterials,
    (meshes, images, terrain): (
        &mut Assets<Mesh>,
        &mut Assets<Image>,
        &mut Assets<TerrainMaterial>,
    ),
    obstacles: &mut CameraObstacles,
    cut: &TunnelCut,
    (library, textured_roads): (Option<&MaterialLibrary>, Option<&Handle<RoadMaterial>>),
    id: TileId,
    mut t: TileMeshes,
) {
    let Some(member) = world.set.members().get(id.0) else {
        return;
    };
    let frame = member.frame;
    // Buildings are camera-solid: the camera stays out of and above them
    // (the camera asks in home metres).
    let tile = (t.key.x, t.key.z);
    let package = id.0 as u32;
    obstacles.remove_tile(package, tile);
    let solid = std::mem::take(&mut t.solid);
    if !solid.is_empty() {
        obstacles.insert(
            ObstacleLayer {
                kind: "buildings",
                package,
                tile,
            },
            move |x, z| {
                let q = frame.invert([x, z]);
                solid.top_at(q[0], q[1])
            },
        );
    }
    let mat = materials.vertex_coloured.clone();
    // The tile's own terrain material with its land-use colour map (freed
    // with the tile's entities), with the tunnel tubes cut out
    // (`tunnel_cut`).
    let mut terrain_mat = terrain_material(
        StandardMaterial {
            base_color: Color::WHITE,
            base_color_texture: t.colour_map.take().map(|map| images.add(map.image())),
            perceptual_roughness: 1.0,
            ..default()
        },
        cut,
    );
    // The ground materials, blended by the tile's splat weights (spec §4.6).
    if let (Some(lib), Some(splat)) = (library, t.splat.take()) {
        let e = &mut terrain_mat.extension;
        let ground = lib.set_for(member.package().and_then(|p| p.time_zone.as_deref()));
        e.look = ground.look;
        e.look.mode.y = splat.cells as f32;
        e.look.tile.x = member.package().map_or(1000.0, |p| p.tile_size_m);
        let (a, b) = splat.images();
        e.splat_a = Some(images.add(a));
        e.splat_b = Some(images.add(b));
        e.albedo = Some(ground.albedo.clone());
        e.normal = Some(ground.normal.clone());
    }
    let terrain_mat = terrain.add(terrain_mat);
    let parent = commands
        .spawn((
            frame_transform(&frame),
            Visibility::default(),
            Name::new(format!("tile {} {} of {}", t.key.x, t.key.z, member.id)),
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
            if !t.bridges.indices.is_empty() {
                c.spawn((
                    Mesh3d(meshes.add(t.bridges.mesh())),
                    MeshMaterial3d(mat.clone()),
                    CameraInteraction::Solid,
                ));
            }
            if !t.roads.indices.is_empty() {
                let mesh = Mesh3d(meshes.add(t.roads.mesh()));
                // Roads and their markings lie on the terrain: no shadows of
                // their own (spec §4.5).
                match textured_roads {
                    Some(road) => c.spawn((
                        mesh,
                        MeshMaterial3d(road.clone()),
                        crate::world::FlatOverlay,
                        CameraInteraction::Solid,
                    )),
                    None => c.spawn((
                        mesh,
                        MeshMaterial3d(mat),
                        crate::world::FlatOverlay,
                        CameraInteraction::Solid,
                    )),
                };
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
    if let Some(old) = streamer.loaded.insert(id, parent) {
        commands.entity(old).despawn();
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use ocw_world::geo::LocalProjection;

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
                surface: ocw_world::RoadSurface::Asphalt,
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
            bridges: None,
            routes: Vec::new(),
            region: None,
            time_zone: None,
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

    /// Spec §4.8.6: a bridge of the `BRDG` section is drawn with its tile,
    /// and the road strip on its deck has no 2 m skirt hanging below it.
    #[test]
    fn bridges_are_drawn_and_their_strips_have_no_skirt() {
        use ocw_world::bridges::{Bridge, BridgeMaterial, BridgeSource, BridgeStructure, Railing};
        let mut p = package();
        let key = TileKey { x: 0, z: 0 };
        p.bridges = Some(BTreeMap::from([(
            key,
            vec![Bridge {
                edge: ocw_world::graph::EdgeId(1),
                osm_way: 1,
                structure: BridgeStructure::Beam,
                material: BridgeMaterial::Concrete,
                source: BridgeSource::Water,
                railing: Railing::Parapet,
                width_m: 6.0,
                deck_m: 0.6,
                points: vec![[30.0, 5.0, 50.0], [70.0, 5.0, 50.0]],
                abutment_foot_y: [3.0, 3.0],
                piers: vec![],
                arches: vec![],
            }],
        )]));
        let t = build_tile_meshes(&p, key).unwrap();
        assert!(!t.bridges.indices.is_empty());
        let plain = road_arrays(&p.tiles[&key].roads);
        let low = |m: &MeshArrays| m.positions.iter().filter(|q| q[1] < 4.0).count();
        // Points 3 … 7 lie on the deck: their 4 skirt-bottom vertices are gone.
        assert_eq!(low(&plain) - low(&t.roads), 5 * 2);
        // At x = 0 the road is 5 m above the terrain: its edge face reaches
        // down to 0.3 m below the ground there (a wall, not a floating slab).
        let lowest = t
            .roads
            .positions
            .iter()
            .map(|q| q[1])
            .fold(f32::INFINITY, f32::min);
        assert!((lowest - (-0.3)).abs() < 0.05, "{lowest}");
        // Beyond the last tile (x = 100 is the next tile's edge here) there
        // is no terrain: the plain 2 m skirt, not a 40 m wall.
        let at_end: Vec<f32> = t
            .roads
            .positions
            .iter()
            .filter(|q| q[0] > 99.0)
            .map(|q| q[1])
            .collect();
        assert!(
            at_end.iter().all(|&y| y >= 5.02 - EDGE_DEPTH_M - 1e-3),
            "{at_end:?}"
        );
        // Without the section the package draws as before.
        assert!(
            build_tile_meshes(&package(), key)
                .unwrap()
                .bridges
                .indices
                .is_empty()
        );
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
    fn a_package_draws_only_the_ground_it_owns() {
        // Keep the west half (x < 50 m) of the land-use tile.
        let p = landuse_package();
        let key = TileKey { x: 0, z: 0 };
        let full = build_tile_meshes(&p, key).unwrap();
        let west = |q: [f64; 2]| q[0] < 50.0;
        let half = build_tile_meshes_kept(&p, key, Some(&west)).unwrap();
        // Half of the 10 × 10 terrain cells, two triangles each.
        assert_eq!(full.terrain.indices.len(), 100 * 6);
        assert_eq!(half.terrain.indices.len(), 50 * 6);
        let used = |m: &MeshArrays| -> Vec<[f32; 3]> {
            m.indices.iter().map(|&i| m.positions[i as usize]).collect()
        };
        assert!(used(&half.terrain).iter().all(|v| v[0] <= 50.0));
        // The water (x 60 … 100) is east: gone. Trees only in the west.
        assert!(!full.water.indices.is_empty());
        assert!(half.water.indices.is_empty());
        // Every tree has the same number of vertices.
        let sites = tree_sites(&p, &p.tiles[&key]);
        let west_sites = sites.iter().filter(|(s, _)| s.x < 50.0).count();
        assert!(west_sites > 0);
        assert_eq!(
            half.trees.positions.len() * sites.len(),
            full.trees.positions.len() * west_sites
        );
        // The house (centroid x = 16) stays; the roads are always drawn.
        assert_eq!(half.buildings.indices.len(), full.buildings.indices.len());
        assert_eq!(half.roads.indices.len(), full.roads.indices.len());
        let east = |q: [f64; 2]| q[0] >= 50.0;
        let other = build_tile_meshes_kept(&p, key, Some(&east)).unwrap();
        assert!(other.buildings.indices.is_empty());
        assert!(other.solid.is_empty());
        assert_eq!(other.terrain.indices.len(), 50 * 6);
    }

    /// Two 100 m packages side by side: home tiles x −2..=0, a neighbour
    /// with its own origin 0.001° east (about 76 m) and tiles 0..=2.
    fn two_packages() -> WorldSet {
        let home = LocalProjection::new(47.0, 9.0);
        let region = |w: f64, e: f64| {
            let (n, s) = (
                home.unproject([0.0, -100.0]).0,
                home.unproject([0.0, 200.0]).0,
            );
            ocw_world::region::PackageRegion::bbox(
                s,
                home.unproject([w, 0.0]).1,
                n,
                home.unproject([e, 0.0]).1,
            )
        };
        let mut graph = ocw_world::graph::RoadGraph {
            vertices: vec![[-150.0, 50.0], [0.0, 50.0]],
            ..Default::default()
        };
        let edge = |id: u64, pts: Vec<[f64; 2]>| ocw_world::graph::Edge {
            id: ocw_world::graph::EdgeId(id),
            from: ocw_world::graph::VertexId(0),
            to: ocw_world::graph::VertexId(1),
            points: pts,
            class: RoadClass::Residential,
            name: None,
            width_m: 5.0,
            bridge: false,
            tunnel: false,
            dead_end_m: None,
            boundary: true,
            oneway: ocw_world::graph::Oneway::No,
            osm_way: 5,
            profile: vec![],
        };
        graph.edges = vec![edge(1, vec![[-150.0, 50.0], [0.0, 50.0]])];
        graph
            .boundary_vertices
            .insert(ocw_world::graph::VertexId(1));
        let mut a = package();
        a.tiles = (-2..=0)
            .map(|x| {
                let key = TileKey { x, z: 0 };
                (
                    key,
                    Tile {
                        key,
                        ..package().tiles[&TileKey { x: 0, z: 0 }].clone()
                    },
                )
            })
            .collect();
        a.graph = Some(graph.clone());
        a.region = Some(region(-300.0, 0.0));
        let mut b = a.clone();
        b.name = "b".into();
        b.origin = (47.0, 9.001);
        let shift = LocalProjection::new(47.0, 9.001).project(47.0, 9.0)[0];
        b.tiles = (0..=2)
            .map(|x| {
                let key = TileKey { x, z: 0 };
                (
                    key,
                    Tile {
                        key,
                        ..package().tiles[&TileKey { x: 0, z: 0 }].clone()
                    },
                )
            })
            .collect();
        let mut g = graph;
        g.vertices = vec![[150.0 + shift, 50.0], [shift, 50.0]];
        g.edges = vec![edge(2, vec![[150.0 + shift, 50.0], [shift, 50.0]])];
        b.graph = Some(g);
        b.region = Some(region(0.0, 300.0));
        let mut set = WorldSet::new("a", Arc::new(a));
        set.add_neighbour("b", Arc::new(b)).unwrap();
        set
    }

    #[test]
    fn tiles_of_both_packages_stream_around_the_rider() {
        let set = two_packages();
        let f = set.members()[1].frame;
        assert!(f.offset[0] > 70.0, "{f:?}");
        // At the border: home tiles −2..=0 and neighbour tiles 0..=2.
        let wanted = wanted_tiles(&set, [0.0, 50.0]);
        let of =
            |i: usize| -> Vec<i32> { wanted.iter().filter(|w| w.0 == i).map(|w| w.1.x).collect() };
        let mut home = of(0);
        home.sort();
        assert_eq!(home, vec![-2, -1, 0]);
        // The neighbour's metres are shifted by about 76 m: its tile 2 is
        // more than two tiles away.
        let mut nb = of(1);
        nb.sort();
        assert_eq!(nb, vec![0, 1]);
        // Nearest first.
        assert_eq!(wanted[0].1.ring(TileKey { x: 0, z: 0 }), 0);
        // Far west: the neighbour's tiles go first.
        let loaded: Vec<TileId> = wanted.clone();
        let far = far_tiles(&set, [-250.0, 50.0], &loaded);
        assert!(far.iter().all(|t| t.0 == 1));
        assert!(far.contains(&(1, TileKey { x: 1, z: 0 })));
        // Without its tiles, all of the neighbour's go.
        let mut set = set;
        set.set_resident(1, None);
        assert!(wanted_tiles(&set, [0.0, 50.0]).iter().all(|w| w.0 == 0));
        assert_eq!(
            far_tiles(&set, [0.0, 50.0], &loaded).len(),
            loaded.iter().filter(|t| t.0 == 1).count()
        );
    }

    #[test]
    fn jumps_and_urgent_tiles() {
        assert!(is_jump(None, [0.0, 0.0]));
        assert!(!is_jump(Some([0.0, 0.0]), [3.0, 4.0]));
        assert!(is_jump(Some([0.0, 0.0]), [0.0, JUMP_M + 1.0]));
        let set = two_packages();
        let urgent = urgent_tiles(&set, [-150.0, 50.0]);
        // Around home tile −2: tiles −2 and −1, nearest first (the
        // neighbour's tiles are farther).
        assert_eq!(
            urgent,
            vec![(0, TileKey { x: -2, z: 0 }), (0, TileKey { x: -1, z: 0 })]
        );
        let wanted = wanted_tiles(&set, [-150.0, 50.0]);
        assert!(urgent.iter().all(|u| wanted.contains(u)));
    }

    /// Regression: for about half a second after a teleport the ground
    /// was missing (only the sky and the route's road): the tiles around
    /// the new place were built on background tasks and uploaded one per
    /// frame. Now the 3 × 3 tiles around the rider are there in the first
    /// frame after the jump.
    #[test]
    fn after_a_teleport_the_near_tiles_are_there_in_the_next_frame() {
        use ocw_core::RiderId;
        use ocw_world::Route;
        let mut pkg = package();
        let tile = pkg.tiles[&TileKey { x: 0, z: 0 }].clone();
        pkg.tiles = (0..10)
            .map(|x| {
                let key = TileKey { x, z: 0 };
                (
                    key,
                    Tile {
                        key,
                        ..tile.clone()
                    },
                )
            })
            .collect();
        let pts = (0..=100)
            .map(|i| [f64::from(i) * 10.0, 5.0, 50.0])
            .collect();
        let route = Route::from_points("r", "R", pts, false, 6.0).unwrap();
        let mut view = RideView::new(route);
        view.riders.push(crate::RiderView {
            id: RiderId(1),
            name: "me".into(),
            state: ocw_sim::RiderState {
                distance: 50.0,
                ..default()
            },
            power_w: 0.0,
            cadence_rpm: None,
            heart_rate_bpm: None,
            is_local: true,
            bike: Default::default(),
            look: None,
        });
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<Image>()
            .init_asset::<StandardMaterial>()
            .init_asset::<SeeThroughMaterial>()
            .init_asset::<TerrainMaterial>()
            .init_resource::<CameraObstacles>()
            .insert_resource(PackageWorld::single(Arc::new(pkg)))
            .insert_resource(view)
            .add_plugins(PackagePlugin);
        let loaded = |app: &App| -> Vec<i32> {
            let mut x: Vec<i32> = app
                .world()
                .resource::<TileStreamer>()
                .loaded
                .keys()
                .map(|(_, k)| k.x)
                .collect();
            x.sort();
            x
        };
        // The start is a jump too: tiles 0 and 1 at once.
        app.update();
        assert_eq!(loaded(&app), vec![0, 1]);
        // Teleport to 850 m (tile 8): its ring is there after one frame.
        app.world_mut().resource_mut::<RideView>().riders[0]
            .state
            .distance = 850.0;
        app.update();
        let now = loaded(&app);
        for x in 7..=9 {
            assert!(now.contains(&x), "{now:?}");
        }
        assert!(!now.contains(&0) && !now.contains(&1), "{now:?}");
        // With their meshes.
        let streamer = app.world().resource::<TileStreamer>();
        let e = streamer.loaded[&(0, TileKey { x: 8, z: 0 })];
        let children = app.world().get::<Children>(e).expect("tile meshes");
        assert!(!children.is_empty());
    }

    #[test]
    fn the_frame_transform_places_tiles_like_the_frame() {
        let set = two_packages();
        let f = set.members()[1].frame;
        let t = frame_transform(&f);
        let q = [123.0, -45.0];
        let p = f.apply(q);
        let v = t.transform_point(Vec3::new(q[0] as f32, 7.0, q[1] as f32));
        assert!((f64::from(v.x) - p[0]).abs() < 1e-3 && (f64::from(v.z) - p[1]).abs() < 1e-3);
        assert_eq!(v.y, 7.0);
        assert_eq!(frame_transform(&Frame::IDENTITY), Transform::default());
        // The ground is the owner's: the home package's west of the
        // border, the neighbour's east of it (both slope 1 m per 10 m in
        // their own metres).
        let world = PackageWorld {
            set: Arc::new(set),
            loader: None,
        };
        let ground = world.ground();
        let west = (ground.0)(-50.0, 50.0).unwrap();
        assert!((west - 5.0).abs() < 1e-6, "{west}");
        let east = (ground.0)(150.0, 50.0).unwrap();
        let local = f.invert([150.0, 50.0])[0].rem_euclid(100.0) / 10.0;
        assert!((east - local).abs() < 1e-6, "{east} vs {local}");
        // In the neighbour's region but outside its tiles: the home
        // package's margin tile draws (no hole).
        let margin = (ground.0)(50.0, 50.0).unwrap();
        assert!((margin - 5.0).abs() < 1e-6, "{margin}");
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

    /// With a material library (spec §4.6, Phase 8 T2): splat weights at
    /// the cell corners, the colour map in the materials' mean colours,
    /// roads with their material layer; without, as before.
    #[test]
    fn a_textured_look_adds_splat_weights_and_material_colours() {
        let p = landuse_package();
        let key = TileKey { x: 0, z: 0 };
        let flat = build_tile_meshes(&p, key).unwrap();
        assert!(flat.splat.is_none());
        let mut means = [[0.5, 0.0, 0.0]; crate::ground::GROUND_LAYERS];
        means[crate::ground::FOREST_FLOOR] = [0.0, 0.0, 0.5];
        means[crate::ground::WETLAND] = [0.0, 0.0, 0.5];
        let mix = crate::ground::default_mix();
        let palette = crate::ground::class_palette(&means, &mix, |_| [1.0; 3]);
        let look = TileLook {
            palette: Some(palette.into_iter().map(|(_, c)| c).collect()),
            road_colours: Some(vec![[0.1, 0.2, 0.3]; 5]),
            mix: Some(std::sync::Arc::new(mix)),
        };
        let m = build_tile_meshes_look(&p, key, None, &look).unwrap();
        let splat = m.splat.expect("splat weights");
        assert_eq!(splat.cells, 20);
        assert_eq!(splat.a.len(), 21 * 21);
        // Corner (2, 15) lies in the forest: its mix (forest floor 60 %).
        assert_eq!(splat.a[15 * 21 + 2][crate::ground::FOREST_FLOOR], 153);
        // The colour map takes the material means: the forest is blue now.
        let map = m.colour_map.unwrap();
        let texel = |c: usize, r: usize, ch: usize| map.data[(r * 20 + c) * 4 + ch];
        // (Blue, darkened by the canopy shade: 0.5 · 0.73 linear ≈ 160 sRGB.)
        assert!(texel(2, 15, 2) > 120, "{}", texel(2, 15, 2));
        assert_eq!(texel(2, 15, 0), 0);
        // Roads: layer and metres in the texture coordinates, colour from
        // the library.
        assert_eq!(m.roads.uvs.len(), m.roads.positions.len());
        assert_eq!(m.roads.uvs_b.len(), m.roads.positions.len());
        assert!(m.roads.uvs_b.iter().all(|u| u[0] == 0.0), "asphalt layer");
        let along = m.roads.uvs.iter().map(|u| u[1]).fold(0.0f32, f32::max);
        assert!((along - 100.0).abs() < 0.5, "{along}");
        assert_eq!(m.roads.colors[0], [0.1, 0.2, 0.3, 1.0]);
        assert_eq!(
            flat.roads.colors[0],
            road_colour(RoadClass::Secondary, RoadSurface::Asphalt)
        );
    }

    #[test]
    fn roads_keep_their_category_colour_unless_osm_says_otherwise() {
        assert_eq!(
            road_colour(RoadClass::Track, RoadSurface::Gravel),
            lin(0.55, 0.47, 0.36)
        );
        assert_ne!(
            road_colour(RoadClass::Residential, RoadSurface::Paving),
            road_colour(RoadClass::Residential, RoadSurface::Asphalt)
        );
        let strip = RoadStrip {
            class: RoadClass::Residential,
            width_m: 4.0,
            points: vec![[0.0, 0.0, 0.0], [10.0, 0.0, 0.0]],
            surface: RoadSurface::Dirt,
        };
        let m = road_arrays(&[strip]);
        assert_eq!(m.roads_layer(), Some(4.0));
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
