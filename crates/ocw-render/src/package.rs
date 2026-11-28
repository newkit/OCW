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

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::light::NotShadowCaster;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages,
};
#[cfg(not(target_arch = "wasm32"))]
use bevy::tasks::block_on;
use bevy::tasks::{AsyncComputeTaskPool, Task, futures::check_ready};
use ocw_world::buildings::Building;
use ocw_world::graph::RoadClass;
use ocw_world::landuse::{LandClass, LandUseTile};
use ocw_world::package::{RoadStrip, Tile, TileKey, WorldPackage};
use ocw_world::world_set::Residency;
use ocw_world::{Frame, RoadSurface, WorldSet};

use crate::RideView;
use crate::building_material::{BuildingMaterial, HouseMaterial};
use crate::camera::{CameraInteraction, CameraObstacles, ObstacleLayer, SolidFootprints};
use crate::flight_world::{FlightObstacles, StreamFocus, TileDetail};
use crate::ground::{TerrainMaterial, terrain_material};
use crate::materials::MaterialLibrary;
use crate::road_material::{RoadMaterial, RoadMaterials};
use crate::see_through::{SeeThroughMaterial, see_through};
use crate::tunnel_cut::TunnelCut;

/// Tiles kept around the rider (radius in tiles; 2 = 5 × 5).
pub const STREAM_RADIUS: i32 = 2;
/// The ring of near tiles meshed around a riding rider, set by the render
/// distance (spec §14 Phase 10, 10G: `ocw_render::quality::
/// stream_radius_for`; 1 below 2.5 km). Never more than
/// [`STREAM_RADIUS`]; flight keeps its own wider rings.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamRadius(pub i32);

impl Default for StreamRadius {
    fn default() -> Self {
        Self(STREAM_RADIUS)
    }
}

/// The most the ring of near tiles may be, whatever the render distance
/// (`None`: no cap): a phone's browser meshes 3 × 3 tiles (1) to stay in
/// its memory (spec 14 Phase 10 B2; `ocw_ui::view_distance::apply`).
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StreamRadiusCap(pub Option<i32>);

impl StreamRadiusCap {
    /// `want` within the cap (at least 1).
    pub fn limit(&self, want: i32) -> i32 {
        self.0.map_or(want, |c| want.min(c.max(1)))
    }
}

impl StreamRadius {
    /// The ring for `focus` (riding: at most this one).
    pub fn cap(&self, focus: &mut crate::flight_world::StreamFocus, riding: bool) {
        if riding {
            focus.radius = focus.radius.min(self.0.clamp(1, STREAM_RADIUS));
        }
    }
}

/// How far from a riding rider the near tiles beyond the rider's own may
/// lie (m; `None`: the whole ring of [`StreamRadius`]): a phone in the
/// saver or minimal memory mode meshes only the neighbour tiles whose
/// ground comes within this distance of the rider, or that the rider's
/// road enters within [`REACH_AHEAD`] times it ahead (the route
/// corridor) – 1 to 3 of the 3 × 3 tiles instead of 9 – and the far
/// terrain shows the rest (spec 14 Phase 10, 10G *The first tiles' GPU
/// memory*). The rider's own tile is always meshed; a meshed tile is
/// dropped [`REACH_DROP_MARGIN_M`] beyond both.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Default)]
pub struct NearTileReach(pub Option<f64>);

/// The route corridor of [`NearTileReach`]: the road ahead this many
/// times the reach.
pub const REACH_AHEAD: f64 = 2.0;

/// The road ahead is sampled every this many metres.
pub const REACH_AHEAD_STEP_M: f64 = 25.0;

/// The points (home metres, x z) of the local rider's road from its
/// position to `ahead_m` ahead, every [`REACH_AHEAD_STEP_M`].
pub fn road_ahead(view: &RideView, ahead_m: f64) -> Vec<[f64; 2]> {
    let Some(l) = view.local() else {
        return Vec::new();
    };
    let track = view.track(l.id);
    let n = (ahead_m / REACH_AHEAD_STEP_M).ceil().max(0.0) as usize;
    (0..=n)
        .map(|i| {
            let p = track
                .sample(l.state.distance + (i as f64 * REACH_AHEAD_STEP_M).min(ahead_m))
                .position;
            [p[0], p[2]]
        })
        .collect()
}

/// A tile meshed for [`NearTileReach`] stays until the rider is this much
/// farther from it (m), so riding along a tile's edge does not rebuild it.
pub const REACH_DROP_MARGIN_M: f64 = 150.0;

/// The distance (m, in the package's metres) from `p` (home metres) to
/// the ground of tile `id` (0 on it).
pub fn tile_distance(set: &WorldSet, id: TileId, p: [f64; 2]) -> f64 {
    let m = &set.members()[id.0];
    let Some(pkg) = m.package() else {
        return f64::INFINITY;
    };
    let size = f64::from(pkg.tile_size_m);
    let q = m.frame.invert(p);
    let o = id.1.origin(size);
    let dx = (o[0] - q[0]).max(q[0] - (o[0] + size)).max(0.0);
    let dz = (o[1] - q[1]).max(q[1] - (o[1] + size)).max(0.0);
    dx.hypot(dz)
}

/// Whether tile `id` is within `reach` of `p` or holds one of the points
/// of the road `ahead`.
pub fn in_reach(set: &WorldSet, id: TileId, p: [f64; 2], ahead: &[[f64; 2]], reach: f64) -> bool {
    tile_distance(set, id, p) <= reach || ahead.iter().any(|&q| tile_distance(set, id, q) == 0.0)
}

/// The rings to mesh within `reach` of `p` or on the road `ahead` (the
/// rider's own tiles always; [`NearTileReach`]).
pub fn within_reach(
    set: &WorldSet,
    rings: &mut Vec<(i32, TileId)>,
    p: [f64; 2],
    ahead: &[[f64; 2]],
    reach: Option<f64>,
) {
    if let Some(r) = reach {
        rings.retain(|&(ring, id)| ring == 0 || in_reach(set, id, p, ahead, r));
    }
}

/// The rider moving farther than this between two frames is a jump
/// (teleport, start of a ride): the near tiles are then built and uploaded
/// at once ([`urgent_tiles`]).
pub const JUMP_M: f64 = 200.0;
/// After a jump, the tiles up to this ring around the rider (1 = 3 × 3)
/// are there in the next frame.
pub const URGENT_RING: i32 = 1;

/// How many tiles are built at once: the urgent ring after a jump and the
/// most pending beyond it. Each tile's meshes take 14 – 28 MB of heap until
/// they are uploaded, so the 3 × 3 urgent tiles of the start held ~200 MB
/// at once – and the browser's wasm memory never shrinks. A phone builds
/// only the rider's own tile at once and one more at a time
/// ([`TileBuildPace::PHONE`]; spec 14 Phase 10 B7, the iPhone's memory,
/// v5.42); the ground around comes tile by tile within a second.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileBuildPace {
    /// The ring built at once after a jump (0: the rider's tile only).
    pub urgent_ring: i32,
    /// Tiles being built or waiting for their upload beyond it.
    pub max_pending: usize,
    /// Rings beyond the wanted ones a meshed tile stays before it is
    /// dropped (1: 5 × 5 tiles stay meshed around a 3 × 3 ring; 0: only
    /// the wanted ones – a phone's 9 tiles' meshes instead of up to 25).
    pub keep_margin: i32,
}

impl Default for TileBuildPace {
    fn default() -> Self {
        Self {
            urgent_ring: URGENT_RING,
            max_pending: MAX_PENDING_TILES,
            keep_margin: 1,
        }
    }
}

impl TileBuildPace {
    /// A phone's browser: one tile at a time, no meshed tiles beyond the
    /// wanted ones.
    pub const PHONE: Self = Self {
        urgent_ring: 0,
        max_pending: 1,
        keep_margin: 0,
    };
}
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

    /// The sidewalks of the resident tiles (riders beside the road).
    pub fn pavement(&self) -> crate::Pavement {
        crate::Pavement(Arc::new(self.set.pavement()))
    }
}

pub struct PackagePlugin;

impl Plugin for PackagePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TileStreamer>()
            .init_resource::<NeighbourLoads>()
            .init_resource::<crate::flight_world::FlightObstacles>()
            .init_resource::<crate::far_terrain::FarTerrain>()
            .init_resource::<crate::far_terrain::FarCull>()
            .add_systems(Startup, setup_materials)
            .add_systems(
                Update,
                (
                    load_neighbours,
                    stream_tiles,
                    upload_tiles,
                    crate::far_terrain::update_far_terrain,
                    crate::far_terrain::hide_far_under_near,
                    crate::vegetation::lazy_plants,
                    crate::ground_detail::lazy_ground,
                )
                    .chain()
                    .run_if(resource_exists::<PackageWorld>),
            )
            .add_systems(Update, crate::far_terrain::package_view)
            .add_systems(Update, crate::flight_world::show_details);
        crate::far_terrain::register_commands(app);
        crate::ground_detail::register(app);
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
    /// Appends `other` (its indices shifted).
    pub fn append(&mut self, other: MeshArrays) {
        let base = self.positions.len() as u32;
        self.positions.extend(other.positions);
        self.normals.extend(other.normals);
        self.colors.extend(other.colors);
        self.uvs.extend(other.uvs);
        self.uvs_b.extend(other.uvs_b);
        self.indices
            .extend(other.indices.into_iter().map(|i| i + base));
    }

    /// The material layer of the first vertex (roads; tests).
    pub fn roads_layer(&self) -> Option<f32> {
        self.uvs_b
            .first()
            .map(|u| f32::from(crate::road_mesh::decode_b(u[0]).0))
    }

    /// The bytes of the arrays' data.
    pub fn bytes(&self) -> usize {
        self.positions.len() * 12
            + self.normals.len() * 12
            + self.colors.len() * 16
            + (self.uvs.len() + self.uvs_b.len()) * 8
            + self.indices.len() * 4
    }

    /// [`Self::mesh`] of a road strip or overlay mesh: the colour's alpha
    /// (the road's half width in metres, `road.wesl`) divided by
    /// [`ROAD_HALF_SCALE`] so the colours pack into 16 bits (10G).
    pub(crate) fn road_mesh(mut self) -> Mesh {
        for c in &mut self.colors {
            c[3] = (c[3] / ROAD_HALF_SCALE).clamp(0.0, 1.0);
        }
        self.mesh()
    }

    /// Drops the colours when every one is white: a land-use terrain's
    /// (its colour map colours it), 8 – 16 bytes a vertex less on the GPU
    /// and the same picture (10G).
    pub fn drop_white(&mut self) {
        if self.colors.iter().all(|c| *c == [1.0; 4]) {
            self.colors = Vec::new();
        }
    }

    /// Frees the arrays' spare capacity (meshes kept until uploaded).
    pub fn shrink_to_fit(&mut self) {
        self.positions.shrink_to_fit();
        self.normals.shrink_to_fit();
        self.colors.shrink_to_fit();
        self.uvs.shrink_to_fit();
        self.uvs_b.shrink_to_fit();
        self.indices.shrink_to_fit();
    }

    /// The mesh for the GPU only (`RenderAssetUsages::RENDER_WORLD`: no
    /// copy stays in the main world) in compact vertex formats (spec 14
    /// Phase 10, 10G *The first tiles' GPU memory*): the normals in 16-bit
    /// signed normalised components ([`ATTRIBUTE_NORMAL_PACKED`], 8 bytes
    /// instead of 12), colours within 0 – 1 in 16-bit unsigned normalised
    /// ones ([`ATTRIBUTE_COLOR_PACKED`], 8 instead of 16; others, such as
    /// the houses' material codes in the alpha, stay 32-bit floats), no
    /// colour at all when there is none ([`Self::drop_white`]). The shaders
    /// read them as the same `vec3<f32>` / `vec4<f32>`.
    pub(crate) fn mesh(self) -> Mesh {
        let mut m = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        );
        m.insert_attribute(Mesh::ATTRIBUTE_POSITION, self.positions);
        m.insert_attribute(ATTRIBUTE_NORMAL_PACKED, pack_normals(&self.normals));
        if !self.colors.is_empty() {
            match pack_colors(&self.colors) {
                Some(c) => m.insert_attribute(ATTRIBUTE_COLOR_PACKED, c),
                None => m.insert_attribute(Mesh::ATTRIBUTE_COLOR, self.colors),
            }
        }
        if !self.uvs.is_empty() {
            m.insert_attribute(Mesh::ATTRIBUTE_UV_0, self.uvs);
        }
        if !self.uvs_b.is_empty() {
            match pack_codes(&self.uvs_b) {
                Some(b) => m.insert_attribute(ATTRIBUTE_UV_1_CODES, b),
                None => m.insert_attribute(Mesh::ATTRIBUTE_UV_1, self.uvs_b),
            }
        }
        m.insert_indices(indices(self.indices, m.count_vertices()));
        m
    }
}

/// The vertex normal in 16-bit signed normalised components (the fourth
/// is 0; the shaders read the first three as their `vec3<f32>`): 8 bytes
/// a vertex instead of 12, an angle error under 0.01° (10G).
pub const ATTRIBUTE_NORMAL_PACKED: bevy::mesh::MeshVertexAttribute =
    bevy::mesh::MeshVertexAttribute::new(
        "Vertex_Normal",
        1,
        bevy::render::render_resource::VertexFormat::Snorm16x4,
    );

/// The vertex colour in 16-bit unsigned normalised components: 8 bytes a
/// vertex instead of 16, steps of 1 / 65,535 (10G).
pub const ATTRIBUTE_COLOR_PACKED: bevy::mesh::MeshVertexAttribute =
    bevy::mesh::MeshVertexAttribute::new(
        "Vertex_Color",
        5,
        bevy::render::render_resource::VertexFormat::Unorm16x4,
    );

/// The second texture coordinates as 16-bit floats when they are whole
/// codes (roads: layer, kind, lines; houses: materials and windows): 4
/// bytes a vertex instead of 8, the same numbers (10G).
pub const ATTRIBUTE_UV_1_CODES: bevy::mesh::MeshVertexAttribute =
    bevy::mesh::MeshVertexAttribute::new(
        "Vertex_Uv_1",
        3,
        bevy::render::render_resource::VertexFormat::Float16x2,
    );

/// A road mesh's colour alpha is its half width over this (m; `road.wesl`
/// multiplies it back): up to 64 m in steps of 1 mm.
pub const ROAD_HALF_SCALE: f32 = 64.0;

/// The codes as [`ATTRIBUTE_UV_1_CODES`] takes them (`None`: one is not a
/// whole number up to ±2048, which 16-bit floats hold exactly; they stay
/// 32-bit floats).
pub fn pack_codes(b: &[[f32; 2]]) -> Option<bevy::mesh::VertexAttributeValues> {
    if !b
        .iter()
        .flatten()
        .all(|v| v.fract() == 0.0 && v.abs() <= 2048.0)
    {
        return None;
    }
    Some(bevy::mesh::VertexAttributeValues::Float16x2(
        b.iter().map(|v| v.map(half::f16::from_f32)).collect(),
    ))
}

/// The normals as [`ATTRIBUTE_NORMAL_PACKED`] takes them.
pub fn pack_normals(n: &[[f32; 3]]) -> bevy::mesh::VertexAttributeValues {
    let q = |v: f32| (v.clamp(-1.0, 1.0) * 32767.0).round() as i16;
    bevy::mesh::VertexAttributeValues::Snorm16x4(
        n.iter().map(|v| [q(v[0]), q(v[1]), q(v[2]), 0]).collect(),
    )
}

/// The colours as [`ATTRIBUTE_COLOR_PACKED`] takes them (`None`: one is
/// outside 0 – 1 – a code, not a colour – and they stay floats).
pub fn pack_colors(c: &[[f32; 4]]) -> Option<bevy::mesh::VertexAttributeValues> {
    if !c.iter().flatten().all(|v| (0.0..=1.0).contains(v)) {
        return None;
    }
    let q = |v: f32| (v * 65535.0).round() as u16;
    Some(bevy::mesh::VertexAttributeValues::Unorm16x4(
        c.iter().map(|v| v.map(q)).collect(),
    ))
}

/// 16-bit indices where the vertices allow (spec §14 Phase 10 B2: half
/// the GPU memory of a tile's index buffers; almost every chunk mesh has
/// fewer than 65,536 vertices).
pub fn indices(i: Vec<u32>, vertices: usize) -> Indices {
    if vertices <= usize::from(u16::MAX) + 1 {
        Indices::U16(i.into_iter().map(|v| v as u16).collect())
    } else {
        Indices::U32(i)
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
    /// Crossings, stop and give-way lines over the roads (blended).
    pub overlays: MeshArrays,
    pub water: MeshArrays,
    /// The plants in chunks, near and far models (spec §14 Phase 8 T8,
    /// `vegetation`).
    pub plants: crate::vegetation::TilePlants,
    /// Houses in three levels of detail (`houses`, spec §14 Phase 8 T4).
    pub houses: crate::houses::HouseMeshes,
    /// Bridge structures (section `BRDG`, spec §4.8.6).
    pub bridges: MeshArrays,
    /// The buildings as camera-solid obstacles.
    pub solid: SolidFootprints,
    /// The tree crowns as obstacles of god mode's flight (spec §14 Phase
    /// 13B; not of the camera: trees are see-through).
    pub crowns: crate::flight_world::TreeCrowns,
    /// Near tint of the ground layers ([`TileLook::near_tint`]).
    pub near_tint: Option<[[f32; 3]; crate::ground::GROUND_LAYERS]>,
    /// Street lamps along lit roads (spec §14 Phase 9 S2).
    pub lamps: crate::street_lamps::TileLamps,
    /// Fences, walls, posts and road barriers (section `OBST`, spec §14
    /// Phase 12 K3).
    pub obstacles: crate::obstacles::TileObstacleMeshes,
    /// What the tile keeps to place its near-field ground detail when a
    /// camera comes close (spec §4.6.2, `ground_detail`).
    pub ground_detail: crate::ground_detail::TileGroundDetail,
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
    /// Near tint per ground layer from the package's measured look and the
    /// world date (spec §14 Phase 8 T3; `None`: neutral).
    pub near_tint: Option<[[f32; 3]; crate::ground::GROUND_LAYERS]>,
    /// Blend weights of the package's seasonal tint maps at the world date
    /// (empty: no satellite colour in the far colour map).
    pub season_weights: Vec<(usize, f32)>,
    /// The colours of the building materials (spec §14 Phase 8 T4).
    pub house: crate::house_style::HouseLook,
    /// The season of the world date (spec §14 Phase 9, S8): trees, the
    /// colour maps' meadows and fields, the far terrain's canopy and snow
    /// (`None`: no world clock yet – summer without snow).
    pub season: Option<Arc<ocw_sky::season::SeasonDay>>,
    /// The snow ramp the far terrain bakes in (the shaders' near snow).
    pub snow: ocw_sky::season::Ramp,
    /// Every how many of the grid's trees one is drawn (0 or 1: all). A
    /// phone's browser draws every second ([`PlantThinning`]): the far
    /// models of the trees are half of a forested tile's GPU memory (11 of
    /// 21 MB on the northern Black Forest; spec 14 Phase 10 B7, v5.42).
    pub plant_thinning: u8,
}

/// The renderer's [`TileLook::plant_thinning`] (the app sets it; 0 by
/// default: all trees).
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PlantThinning(pub u8);

/// Whether the tree at `p` is drawn with `thinning` (every n-th by a hash
/// of its position, so a tile and its rebuilds keep the same trees).
pub fn tree_kept(p: Vec3, thinning: u8) -> bool {
    if thinning <= 1 {
        return true;
    }
    let h = (p.x.to_bits().wrapping_mul(0x9E37_79B1) ^ p.z.to_bits().wrapping_mul(0x85EB_CA77))
        .rotate_left(13)
        .wrapping_mul(0xC2B2_AE3D);
    (h >> 16).is_multiple_of(u32::from(thinning))
}

impl TileLook {
    /// With [`TileLook::plant_thinning`] set.
    pub fn with_plant_thinning(mut self, thinning: u8) -> Self {
        self.plant_thinning = thinning;
        self
    }

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
            near_tint: None,
            season_weights: Vec::new(),
            house: crate::house_style::HouseLook {
                means: lib.buildings.as_ref().map(|b| b.means),
            },
            season: None,
            snow: ocw_sky::season::Ramp::default(),
            plant_thinning: 0,
        }
    }

    /// The look with the season of the world date (spec §14 Phase 9, S8).
    pub fn with_season(
        mut self,
        season: Option<Arc<ocw_sky::season::SeasonDay>>,
        snow: ocw_sky::season::Ramp,
    ) -> Self {
        self.season = season;
        self.snow = snow;
        self
    }

    /// The look of a package with its measured region look (spec §14
    /// Phase 8 T3) at `day_of_year`: the near layers and the far class
    /// colours tinted towards the measured palette and the season, the
    /// seasonal satellite maps blended by the date. `set` is the package's
    /// ground set (`None` without the material library: only the satellite
    /// colour applies).
    pub fn with_region(
        mut self,
        region: &ocw_world::look::RegionLook,
        set: Option<&crate::materials::GroundSet>,
        day_of_year: Option<f64>,
    ) -> Self {
        use crate::region_look as rl;
        if region.is_empty() {
            return self;
        }
        let weights = match (region.seasons.as_ref(), day_of_year) {
            (Some(s), Some(d)) => s.weights(d),
            _ => Vec::new(),
        };
        if let Some(set) = set {
            let tints = rl::near_tints(region, &set.means, &weights);
            let class = rl::class_tints(&set.mix, &set.means, &tints);
            let palette = crate::ground::class_palette(&set.means, &set.mix, |c| class[c as usize]);
            self.palette = Some(palette.into_iter().map(|(_, c)| c).collect());
            self.near_tint = Some(tints);
        }
        self.season_weights = weights;
        self
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
    // The alpine ground above the region's tree line (v6.58).
    let tree_line = ocw_world::flora::FloraRegion::at(pkg.origin.0, pkg.origin.1).tree_line_m();
    let size = f64::from(pkg.tile_size_m);
    let (o, cs) = (key.origin(size), size / f64::from(cells));
    let alpine = |c: i64, r: i64| {
        pkg.height(o[0] + c as f64 * cs, o[1] + r as f64 * cs)
            .map_or(0.0, |h| crate::ground::alpine_share(h, tree_line))
    };
    let (a, b) = crate::ground::splat_corners_alpine(cells as usize, mix, class, alpine);
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

/// The ground as a tile draws it: the heights of its terrain mesh
/// ([`terrain_arrays_kept`]'s `n × n` grid, after the renderer changed it –
/// the terrain lowered under the junction surfaces), triangulated like the
/// mesh (cells split along (i+1, k)–(i, k+1)); outside the tile, or
/// without the mesh, the package's terrain triangulated the same way
/// ([`WorldPackage::surface_height`]). What lies on the ground – the
/// fields' stands – takes its heights from here, so it lies on the drawn
/// terrain rather than on the bilinear [`WorldPackage::height`], which
/// differs from it by up to the cell's twist (metres on terrace edges).
pub struct TileGround<'a> {
    pkg: &'a WorldPackage,
    /// The tile's terrain mesh (its grid's vertices) and its origin.
    mesh: Option<(&'a MeshArrays, [f64; 2])>,
}

impl<'a> TileGround<'a> {
    /// The ground of tile `key` drawn as `mesh` (its vertices are the
    /// tile's terrain grid; `None`: the package's terrain).
    pub fn new(pkg: &'a WorldPackage, key: TileKey, mesh: Option<&'a MeshArrays>) -> Self {
        let n = pkg.samples as usize;
        let mesh = mesh
            .filter(|m| n >= 2 && m.positions.len() == n * n)
            .map(|m| (m, key.origin(f64::from(pkg.tile_size_m))));
        Self { pkg, mesh }
    }

    /// The drawn ground's height at local (x, z).
    pub fn height(&self, x: f64, z: f64) -> Option<f64> {
        let Some((m, o)) = self.mesh else {
            return self.pkg.surface_height(x, z);
        };
        let n = self.pkg.samples as usize;
        let s = self.pkg.spacing_m();
        let (fx, fz) = ((x - o[0]) / s, (z - o[1]) / s);
        let last = (n - 1) as f64;
        if !(0.0..=last).contains(&fx) || !(0.0..=last).contains(&fz) {
            return self.pkg.surface_height(x, z);
        }
        let (i, k) = ((fx as usize).min(n - 2), (fz as usize).min(n - 2));
        let (tx, tz) = (fx - i as f64, fz - k as f64);
        let h = |i: usize, k: usize| f64::from(m.positions[k * n + i][1]);
        let (h00, h10, h01, h11) = (h(i, k), h(i + 1, k), h(i, k + 1), h(i + 1, k + 1));
        Some(if tx + tz <= 1.0 {
            h00 + tx * (h10 - h00) + tz * (h01 - h00)
        } else {
            h11 + (1.0 - tx) * (h01 - h11) + (1.0 - tz) * (h10 - h11)
        })
    }

    /// The lowest drawn ground under a foot of radius `r` at local (x, z)
    /// (its centre and eight points round its rim): a trunk or a pole
    /// standing there with its base this low reaches the ground all round,
    /// also on a slope or a terrace edge.
    pub fn foot(&self, x: f64, z: f64, r: f64) -> Option<f64> {
        let mut low = self.height(x, z)?;
        for k in 0..8 {
            let a = f64::from(k) * std::f64::consts::FRAC_PI_4;
            if let Some(h) = self.height(x + r * a.cos(), z + r * a.sin()) {
                low = low.min(h);
            }
        }
        Some(low)
    }
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

/// Road surface strips with kerbs, sidewalks, shoulders and plain skirts
/// (no terrain), in world coordinates (spec §14 Phase 8 T5,
/// [`crate::road_mesh`]).
pub fn road_arrays(strips: &[RoadStrip]) -> MeshArrays {
    let style = ocw_world::road_dress::MarkingStyle::default();
    crate::road_mesh::road_meshes(
        strips,
        &crate::road_mesh::RoadContext {
            colours: None,
            ground: &|_, _| None,
            deck: &|_| false,
            style: &style,
        },
    )
    .roads
}

/// The road meshes of a tile of `pkg`: the terrain beside the roads from
/// the package (the shoulders blend into it), no skirts on bridge decks;
/// with `junctions`, the junction surfaces of spec §4.8.8 (J1,
/// [`crate::junction_mesh`]), else the strips as the package has them.
pub fn tile_road_meshes(
    pkg: &WorldPackage,
    tile: &Tile,
    bridges: &[&ocw_world::bridges::Bridge],
    colours: Option<&[[f32; 3]]>,
    junctions: Option<&crate::junction_mesh::PackageJunctions>,
) -> crate::road_mesh::RoadMeshes {
    let style = crate::road_style::style_for(pkg.time_zone.as_deref());
    let ground = |x: f32, z: f32| {
        pkg.surface_height(f64::from(x), f64::from(z))
            .map(|h| h as f32)
    };
    let deck = |p: [f32; 3]| crate::bridges::on_deck(bridges, p);
    let ctx = crate::road_mesh::RoadContext {
        colours,
        ground: &ground,
        deck: &deck,
        style,
    };
    let mut meshes = match (junctions, pkg.graph.as_ref()) {
        (Some(pj), Some(g)) => {
            let shaped = crate::junction_mesh::shape_tile(
                g,
                pj,
                tile.key,
                f64::from(pkg.tile_size_m),
                &tile.roads,
                style,
            );
            crate::junction_mesh::shaped_meshes(&shaped, &ctx)
        }
        _ => crate::road_mesh::road_meshes(&tile.roads, &ctx),
    };
    // The open and gallery stretches of the tunnels (spec 3.2): road strips
    // like any other, the same meshes and materials as the road they
    // continue (the tunnel drawing adds only the gallery's structure).
    if let (Some(g), Some(t)) = (pkg.graph.as_ref(), pkg.look.tunnels.as_ref()) {
        let open = ocw_world::tunnels::surface_strips(
            g,
            t,
            tile.key,
            f64::from(pkg.tile_size_m),
            &tile.roads,
        );
        if !open.is_empty() {
            let m = crate::road_mesh::road_meshes(&open, &ctx);
            meshes.roads.append(m.roads);
            meshes.overlays.append(m.overlays);
        }
    }
    meshes
}

use ocw_world::trees::hash;

/// Crown of a package tree (scale 1): bottom, tip and radius in metres.
pub(crate) const CROWN: (f32, f32, f32) = (1.8, 9.0, 2.2);

/// Where a tree stands (foot of the trunk) and its scale.
pub type TreeSite = (Vec3, f32);

pub use ocw_world::trees::{TRUNK_RADIUS_M, blocked_cells, tile_landuse, tree_density};

/// Tree meshes (trunk and cone crown, crown shades varying per tree).
pub fn tree_arrays(sites: &[TreeSite]) -> MeshArrays {
    let classed: Vec<(TreeSite, Option<LandClass>)> = sites.iter().map(|&s| (s, None)).collect();
    seasonal_tree_arrays(None, &classed, None)
}

/// A ring-built tree part: rings of (height, radius) around `base`
/// (scale `s`), closed by apexes where the radius is 0; normals from the
/// part's centre at `centre_y` (squashed by `flat`: 1 a sphere, larger
/// a cone); uv.x the height above the foot (the sway).
#[allow(clippy::too_many_arguments)]
fn ring_part(
    m: &mut MeshArrays,
    base: Vec3,
    s: f32,
    rings: &[(f32, f32)],
    seg: u32,
    colour: [f32; 4],
    centre_y: f32,
    flat: f32,
    turn: f32,
) {
    let mut prev: Option<(u32, bool)> = None;
    for &(y, r) in rings {
        let b = m.positions.len() as u32;
        let apex = r <= 1e-4;
        let n = if apex { 1 } else { seg };
        for j in 0..n {
            let a = turn + j as f32 / seg as f32 * std::f32::consts::TAU;
            let (cx, cz) = if apex { (0.0, 0.0) } else { (a.cos(), a.sin()) };
            m.positions
                .push((base + Vec3::new(r * cx, y, r * cz) * s).to_array());
            m.uvs.push([y * s, 0.0]);
            let nrm = if apex {
                Vec3::new(0.0, (y - centre_y).signum(), 0.0)
            } else {
                Vec3::new(cx, (y - centre_y) * flat / r.max(0.3), cz).normalize_or(Vec3::Y)
            };
            m.normals.push(nrm.to_array());
            m.colors.push(colour);
        }
        if let Some((pb, papex)) = prev {
            match (papex, apex) {
                (true, true) => {}
                // The winding of the conifers' cones (ring, apex, next).
                (true, false) => {
                    for j in 0..seg {
                        m.indices.extend([pb, b + j, b + (j + 1) % seg]);
                    }
                }
                (false, true) => {
                    for j in 0..seg {
                        m.indices.extend([pb + j, b, pb + (j + 1) % seg]);
                    }
                }
                (false, false) => {
                    for j in 0..seg {
                        let (a0, a1) = (pb + j, pb + (j + 1) % seg);
                        let (b0, b1) = (b + j, b + (j + 1) % seg);
                        m.indices.extend([a0, b0, a1, a1, b0, b1]);
                    }
                }
            }
        }
        prev = Some((b, apex));
    }
}

/// A thin branch: a cone from `from` to `to` (package metres).
fn branch(m: &mut MeshArrays, from: Vec3, to: Vec3, radius: f32, foot_y: f32, colour: [f32; 4]) {
    let d = (to - from).normalize_or(Vec3::Y);
    let u = d.any_orthonormal_vector();
    let v = u.cross(d);
    let b = m.positions.len() as u32;
    let seg = 3u32;
    for j in 0..seg {
        let a = j as f32 / seg as f32 * std::f32::consts::TAU;
        let off = u * a.cos() + v * a.sin();
        m.positions.push((from + off * radius).to_array());
        m.uvs.push([from.y - foot_y, 0.0]);
        m.normals.push(off.to_array());
        m.colors.push(colour);
    }
    m.positions.push(to.to_array());
    m.uvs.push([to.y - foot_y, 0.0]);
    m.normals.push(d.to_array());
    m.colors.push(colour);
    for j in 0..seg {
        m.indices.extend([b + j, b + seg, b + (j + 1) % seg]);
    }
}

/// Tree meshes with the season (spec §14 Phase 9, S8): each tree's leaf
/// type – measured by the package (`ocw_world::trees::measured_leaf`, the
/// T3 hook) or the region's mix by land class and altitude – and its
/// leaves on the world date: conifers as green cones, deciduous trees with
/// round crowns that leaf out, green, colour and go bare (branches), larches
/// as light cones turning golden and bare. Without a season every tree is
/// a conifer, as before.
pub fn seasonal_tree_arrays(
    pkg: Option<&WorldPackage>,
    sites: &[(TreeSite, Option<LandClass>)],
    season: Option<&ocw_sky::season::SeasonDay>,
) -> MeshArrays {
    use ocw_sky::season::LeafType;
    let mut m = MeshArrays::default();
    let rgba = |c: [f32; 3]| [c[0], c[1], c[2], 1.0];
    let twig = rgba(crate::season::twig());
    let crowns = [
        lin(0.13, 0.33, 0.15),
        lin(0.11, 0.28, 0.13),
        lin(0.17, 0.36, 0.14),
        lin(0.20, 0.38, 0.17),
    ];
    let trunk = lin(0.35, 0.24, 0.15);
    for &((base, s), class) in sites {
        let bits = (i64::from(base.x.to_bits()), i64::from(base.z.to_bits()));
        let pick = hash(bits.0, bits.1, 11);
        let crown = crowns[((pick * crowns.len() as f32) as usize).min(crowns.len() - 1)];
        let alt = f64::from(base.y);
        let leaf = match season {
            Some(day) => day.climate.trees.leaf_type(
                crate::season::tree_setting(class),
                alt,
                crate::season::measured(pkg.and_then(|p| {
                    ocw_world::trees::measured_leaf(p, f64::from(base.x), f64::from(base.z))
                })),
                hash(bits.0, bits.1, 13),
            ),
            None => LeafType::Evergreen,
        };
        // The trunk and, for conifers, the cone crown (as before the
        // seasons).
        let conifer = leaf == LeafType::Evergreen || season.is_none();
        let trunk_top = if conifer { 2.2 } else { 2.6 };
        let mut parts = vec![(0.0, trunk_top, TRUNK_RADIUS_M as f32, trunk, 5u32)];
        if conifer {
            parts.push((CROWN.0, CROWN.1, CROWN.2, crown, 7));
        }
        for (y0, y1, r, colour, seg) in parts {
            let b = m.positions.len() as u32;
            for j in 0..seg {
                let a = j as f32 / seg as f32 * std::f32::consts::TAU;
                m.positions
                    .push((base + Vec3::new(r * a.cos(), y0, r * a.sin()) * s).to_array());
                // The height above the foot: the trees sway with it.
                m.uvs.push([y0 * s, 0.0]);
                m.normals.push(
                    Vec3::new(a.cos(), r / (y1 - y0), a.sin())
                        .normalize()
                        .to_array(),
                );
                m.colors.push(colour);
            }
            m.positions.push((base + Vec3::Y * y1 * s).to_array());
            m.uvs.push([y1 * s, 0.0]);
            m.normals.push([0.0, 1.0, 0.0]);
            m.colors.push(colour);
            for j in 0..seg {
                m.indices.extend([b + j, b + seg, b + (j + 1) % seg]);
            }
        }
        let Some(day) = season.filter(|_| !conifer) else {
            continue;
        };
        let jitter = hash(bits.0, bits.1, 14) * 2.0 - 1.0;
        let f = day.foliage(leaf, alt, jitter);
        let colour = rgba(crate::season::tree_colour(
            leaf,
            &f,
            pick,
            hash(bits.0, bits.1, 15),
        ));
        let turn = pick * std::f32::consts::TAU;
        if leaf == LeafType::Larch {
            // A slim light cone; bare: a thin twiggy one.
            let (k, c) = if f.leaves < 0.12 {
                (0.5, twig)
            } else {
                (0.6 + 0.4 * f.leaves, colour)
            };
            let (y0, y1) = (1.6, 9.6);
            let r = 1.9 * k;
            let b = m.positions.len() as u32;
            let seg = 7u32;
            for j in 0..seg {
                let a = turn + j as f32 / seg as f32 * std::f32::consts::TAU;
                m.positions
                    .push((base + Vec3::new(r * a.cos(), y0, r * a.sin()) * s).to_array());
                m.uvs.push([y0 * s, 0.0]);
                m.normals.push(
                    Vec3::new(a.cos(), r / (y1 - y0), a.sin())
                        .normalize()
                        .to_array(),
                );
                m.colors.push(c);
            }
            m.positions.push((base + Vec3::Y * y1 * s).to_array());
            m.uvs.push([y1 * s, 0.0]);
            m.normals.push([0.0, 1.0, 0.0]);
            m.colors.push(c);
            for j in 0..seg {
                m.indices.extend([b + j, b + seg, b + (j + 1) % seg]);
            }
            continue;
        }
        // A deciduous tree: a round crown that grows with the leaves;
        // bare (and while it is thin) its branches show.
        if f.leaves < 0.5 {
            let from = base + Vec3::Y * 2.3 * s;
            // Branches leaving the trunk at different heights, spreading
            // out and up (not a broom from one point).
            for k in 0..6 {
                let a = turn + k as f32 * 2.4;
                let h = 2.2 + 0.75 * k as f32;
                let reach = 2.2 - 0.22 * k as f32;
                let start = base + Vec3::Y * h * s;
                let to = base + Vec3::new(reach * a.cos(), h + 2.4, reach * a.sin()) * s;
                branch(
                    &mut m,
                    start,
                    to,
                    (0.12 - 0.012 * k as f32) * s,
                    base.y,
                    twig,
                );
            }
            branch(
                &mut m,
                from,
                base + Vec3::Y * 7.8 * s,
                0.13 * s,
                base.y,
                twig,
            );
        }
        if f.leaves >= 0.12 {
            let k = 0.55 + 0.45 * f.leaves;
            let c = DECIDUOUS_CENTRE;
            let rings: Vec<(f32, f32)> = DECIDUOUS_CROWN
                .iter()
                .map(|&(y, r)| (c + (y - c) * k, r * k))
                .collect();
            ring_part(&mut m, base, s, &rings, 7, colour, c, 1.0, turn);
        }
    }
    m
}

/// The round crown of a deciduous tree at scale 1 in full leaf: rings of
/// (height, radius) from the bottom apex to the top one (m).
pub(crate) const DECIDUOUS_CROWN: [(f32, f32); 6] = [
    (2.0, 0.0),
    (2.6, 1.9),
    (4.2, 2.7),
    (5.9, 2.5),
    (7.2, 1.5),
    (7.9, 0.0),
];
/// The crown's centre height (m): it shrinks towards it while the leaves
/// come or go.
pub(crate) const DECIDUOUS_CENTRE: f32 = 4.9;

/// Where the trees of a tile stand: from the land use if the package has
/// it, else provisional patches ([`ocw_world::trees::tree_sites`]).
pub fn tree_sites(pkg: &WorldPackage, tile: &Tile) -> Vec<TreeSite> {
    ocw_world::trees::tree_sites(pkg, tile)
        .into_iter()
        .map(|(p, s)| (Vec3::from_array(p), s))
        .collect()
}

/// Trees by land use ([`ocw_world::trees::landuse_tree_sites`]).
pub fn landuse_tree_sites(
    pkg: &WorldPackage,
    tile: &Tile,
    lu: &LandUseTile,
    cells: u32,
) -> Vec<TreeSite> {
    ocw_world::trees::landuse_tree_sites(pkg, tile, lu, cells)
        .into_iter()
        .map(|(p, s)| (Vec3::from_array(p), s))
        .collect()
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

/// How much of the canopy colour lies over a forest cell of the colour map
/// (the ground between the far trees and beyond their range: forest, not
/// litter; the far terrain takes all of it).
pub const COLOUR_MAP_CANOPY: f32 = 0.6;

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
    colour_map_seasonal(pkg, tile, lu, cells, palette, None, None)
}

/// A tile's seasonal satellite map (spec §14 Phase 8 T3): the decoded
/// maps of the package's `SEAS` section blended by `weights`.
pub struct SatelliteMap {
    /// Cells per side.
    pub cells: u32,
    /// `cells²` sRGB, row-major along +z.
    pub rgb: Vec<[u8; 3]>,
}

impl SatelliteMap {
    /// The tile's map at the season `weights`, if the package has one.
    pub fn of_tile(pkg: &WorldPackage, key: TileKey, weights: &[(usize, f32)]) -> Option<Self> {
        let s = pkg.look.seasons.as_ref()?;
        let maps: Vec<(Vec<[u8; 3]>, f32)> = weights
            .iter()
            .map(|&(i, w)| s.map(key, i).map(|m| (m, w)))
            .collect::<Option<_>>()?;
        Some(Self {
            cells: s.cells,
            rgb: crate::region_look::blend_maps(&maps)?,
        })
    }

    /// Bilinear colour at a fraction (u, v) of the tile (0 … 1).
    pub fn at(&self, u: f64, v: f64) -> [u8; 3] {
        let n = self.cells as usize;
        let fx = (u * n as f64 - 0.5).clamp(0.0, (n - 1) as f64);
        let fy = (v * n as f64 - 0.5).clamp(0.0, (n - 1) as f64);
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(n - 1), (y0 + 1).min(n - 1));
        let (tx, ty) = (fx - x0 as f64, fy - y0 as f64);
        let p = |x: usize, y: usize| self.rgb[y * n + x].map(f64::from);
        let (a, b, c, d) = (p(x0, y0), p(x1, y0), p(x0, y1), p(x1, y1));
        std::array::from_fn(|k| {
            let top = a[k] + (b[k] - a[k]) * tx;
            let bot = c[k] + (d[k] - c[k]) * tx;
            (top + (bot - top) * ty).round() as u8
        })
    }
}

/// [`colour_map_with`] with the tile's seasonal satellite colour laid over
/// the class colours ([`crate::region_look::far_colour`]), and the meadows
/// and fields in the colours of the `season` at each cell's altitude
/// (spec §14 Phase 9, S8; [`crate::season::ground_colour`]). The snow is
/// the terrain shader's (it lies on these tiles near and far).
pub fn colour_map_seasonal(
    pkg: &WorldPackage,
    tile: &Tile,
    lu: &LandUseTile,
    cells: u32,
    palette: Option<&[[f32; 3]]>,
    satellite: Option<&SatelliteMap>,
    season: Option<&ocw_sky::season::SeasonDay>,
) -> ColourMap {
    let n = cells as usize;
    let size = f64::from(pkg.tile_size_m);
    let o = tile.key.origin(size);
    let cs = size / f64::from(cells);
    let mut level0 = Vec::with_capacity(n * n * 4);
    // The alpine ground above the region's tree line and the measured
    // forest's canopy (v6.58), as the far terrain draws them.
    let region = ocw_world::flora::FloraRegion::at(pkg.origin.0, pkg.origin.1);
    let tree_line = region.tree_line_m();
    let forest = ocw_world::trees::tile_forest(pkg, tile.key);
    let conifers = crate::flora::far_conifers(region);
    let mut tones: std::collections::HashMap<i64, crate::season::CanopyTones> =
        std::collections::HashMap::new();
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
                crate::region_look::stone_grey(
                    c,
                    palette
                        .and_then(|p| p.get(c as usize).copied())
                        .unwrap_or_else(|| land_colour(c)),
                )
            };
            let mut colour = flat(class);
            // The field's crop (section VEGE, spec §14 Phase 8 T8): its own
            // calendar instead of the region's mix, its share of summer
            // crops in the alpha for the terrain shader (255: unknown, the
            // shader's noise patches).
            let crop = pkg
                .look
                .vegetation
                .as_ref()
                .map(|v| v.at(size, x, z).0)
                .unwrap_or_default();
            let summer = crop
                .arable()
                .then(|| if crop.summer_crop() { 1.0 } else { 0.0 });
            if let (Some(day), Some(y)) = (season, pkg.height(x, z)) {
                colour = match summer {
                    Some(m) => {
                        crate::season::ground_colour_with(class, colour, y, day, m, tree_line)
                    }
                    None => crate::season::ground_colour(class, colour, y, day, tree_line),
                };
            }
            colour = crate::season::plantation_ground(crop, colour);
            let alt = pkg.height(x, z);
            let alpine = alt.map_or(0.0, |h| crate::ground::alpine_share(h, tree_line));
            if crate::ground::turns_alpine(class) && alpine > 0.0 {
                let stone = crate::region_look::alpine_stone(flat);
                let k = alpine * crate::ground::ALPINE_STONE;
                colour = std::array::from_fn(|j| colour[j] + (stone[j] - colour[j]) * k);
            }
            let alpha = summer.map_or(255u8, |m| (m * 250.0f32).round() as u8);
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
                // The canopy over the forest floor (the ground beyond the
                // trees' range is forest, not litter).
                let measured = forest.and_then(|(f, fc)| f.at(tile.key, size, fc, x, z));
                let cover =
                    crate::far_terrain::measured_tree_cover(class, measured.map(|m| m.density))
                        * (1.0 - t)
                        * COLOUR_MAP_CANOPY;
                if cover > 0.0 {
                    let canopy = match (season, alt) {
                        (Some(day), Some(h)) => {
                            let band = (h / 50.0).round() as i64;
                            tones
                                .entry(band)
                                .or_insert_with(|| {
                                    crate::season::CanopyTones::new(
                                        conifers,
                                        band as f64 * 50.0,
                                        day,
                                    )
                                })
                                .at(measured
                                    .and_then(|m| m.broadleaf)
                                    .map(|b| f32::from(b) / 100.0))
                        }
                        _ => conifers,
                    };
                    colour = std::array::from_fn(|j| colour[j] + (canopy[j] - colour[j]) * cover);
                }
            }
            if let Some(sat) = satellite
                && class != LandClass::Water
            {
                let (u, v) = ((c as f64 + 0.5) / n as f64, (r as f64 + 0.5) / n as f64);
                let russet = match (season, alt) {
                    (Some(day), Some(h)) if crate::ground::turns_alpine(class) => {
                        crate::season::russet_amount(day, h, tree_line)
                    }
                    _ => 0.0,
                };
                colour = crate::region_look::far_colour_alpine(
                    colour,
                    Some(sat.at(u, v)),
                    alpine,
                    russet,
                );
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
            level0.push(alpha);
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
/// corners have a water level (the terrain is carved below it, and fitted
/// to the outline along the shore: the shore is where the drawn terrain
/// rises through the water). Each vertex carries the water's depth there
/// (below 0 where the land is above the water) and its kind, the sea where
/// the level is 0 m (`water::KIND_*`, `uv`).
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
        let depth = tile.heights.get(idx as usize).map_or(1.0, |h| level - h);
        let kind = if level == 0.0 {
            crate::water::KIND_SEA
        } else {
            crate::water::KIND_LAKE
        };
        m.uvs.push([depth, kind]);
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

/// The buildings as camera-solid prisms: the footprint up to the top of
/// each house (ridge, spire or parapet, `houses`).
pub fn building_footprints(
    buildings: &[Building],
    specs: &crate::houses::TileSpecs,
) -> SolidFootprints {
    SolidFootprints::new(
        buildings
            .iter()
            .enumerate()
            .map(|(i, b)| (b.outline.clone(), specs.top(i))),
    )
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

/// [`build_tile_meshes_kept`] for a [`TileLook`] (the junction surfaces
/// made for this call; resident packages keep theirs,
/// [`build_tile_meshes_with`]).
pub fn build_tile_meshes_look(
    pkg: &WorldPackage,
    key: TileKey,
    keep: Keep,
    look: &TileLook,
) -> Option<TileMeshes> {
    let junctions = crate::junction_mesh::PackageJunctions::new(pkg);
    build_tile_meshes_with(pkg, key, keep, look, junctions.as_ref())
}

/// [`build_tile_meshes_look`] with the package's junction surfaces
/// (`None`: none, the strips as the package has them).
pub fn build_tile_meshes_with(
    pkg: &WorldPackage,
    key: TileKey,
    keep: Keep,
    look: &TileLook,
    junctions: Option<&crate::junction_mesh::PackageJunctions>,
) -> Option<TileMeshes> {
    let tile = pkg.tiles.get(&key)?;
    let mut classed: Vec<(TreeSite, Option<LandClass>)> =
        ocw_world::trees::classed_tree_sites(pkg, tile)
            .into_iter()
            .map(|((p, s), c)| ((Vec3::from_array(p), s), c))
            .collect();
    if let Some(f) = keep {
        classed.retain(|((p, _), _)| f([f64::from(p.x), f64::from(p.z)]));
    }
    if look.plant_thinning > 1 {
        classed.retain(|((p, _), _)| tree_kept(*p, look.plant_thinning));
    }
    let lu = tile_landuse(pkg, key);
    // The kept buildings with their measured roof colours (section ROOF,
    // in the order of the tile's buildings).
    let (kept, roofs): (
        Option<Vec<Building>>,
        Vec<Option<ocw_world::look::RoofColour>>,
    ) = match pkg.buildings.as_ref().and_then(|b| b.get(&key)) {
        Some(b) => {
            let (bs, rs) = b
                .iter()
                .enumerate()
                .filter(|(_, b)| keep.is_none_or(|f| f(outline_centroid(&b.outline))))
                .map(|(i, b)| (b.clone(), pkg.look.roof(key, i)))
                .unzip();
            (Some(bs), rs)
        }
        None => (None, Vec::new()),
    };
    // Houses (spec §14 Phase 8 T4): the style of each, then its meshes in
    // three levels of detail.
    let specs = kept
        .as_deref()
        .map(|b| crate::houses::house_specs(pkg, b, &roofs, &look.house))
        .unwrap_or_default();
    let houses = kept.as_deref().map_or_else(Default::default, |b| {
        let o = key.origin(f64::from(pkg.tile_size_m));
        crate::houses::tile_houses(b, &specs, [o[0] as f32, o[1] as f32], pkg.tile_size_m)
    });
    let satellite = SatelliteMap::of_tile(pkg, key, &look.season_weights);
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
        .map_or_else(SolidFootprints::default, |b| building_footprints(b, &specs));
    let roads = tile_road_meshes(pkg, tile, &near, look.road_colours.as_deref(), junctions);
    let mut terrain = terrain_arrays_kept(pkg, tile, keep);
    // J1 packages: the terrain lowered under the junction surfaces at load;
    // a J2 package has it settled (section `JNCT`, spec §4.8.8).
    if let (Some(pj), Some(g)) = (junctions.filter(|pj| !pj.settled), pkg.graph.as_ref()) {
        let patches = crate::junction_mesh::patches_near(g, pj, key);
        let grid = crate::junction_mesh::TerrainGrid {
            origin: key.origin(f64::from(pkg.tile_size_m)),
            spacing: pkg.spacing_m(),
            n: pkg.samples as usize,
        };
        crate::junction_mesh::lower_terrain(&mut terrain, g, &patches, grid);
    }
    // The plants, the fields' stands, the lamps and the obstacles stand
    // on the ground as drawn (spec §4.6.1).
    let ground = TileGround::new(pkg, key, Some(&terrain));
    let mut plants =
        crate::vegetation::tile_plants(pkg, key, &classed, keep, look.season.as_deref(), &ground);
    plants.crowns.shrink_to_fit();
    let lamps = crate::street_lamps::tile_lamps(pkg, key, keep, &ground, junctions);
    let obstacles = crate::obstacles::tile_obstacle_meshes(pkg, key, keep, &ground);
    let ground_detail =
        crate::ground_detail::tile_detail(pkg, key, keep, &terrain, &solid, look.season.clone());
    Some(TileMeshes {
        key,
        terrain,
        colour_map: lu.map(|(lu, cells)| {
            colour_map_seasonal(
                pkg,
                tile,
                lu,
                cells,
                look.palette.as_deref(),
                satellite.as_ref(),
                look.season.as_deref(),
            )
        }),
        splat: lu.filter(|_| look.textured()).map(|(_, cells)| {
            let mix = look
                .mix
                .as_deref()
                .copied()
                .unwrap_or_else(crate::ground::default_mix);
            tile_splat(pkg, key, cells, &mix)
        }),
        roads: roads.roads,
        overlays: roads.overlays,
        water: {
            let mut w = lu.map_or_else(MeshArrays::default, |(lu, _)| {
                water_arrays_kept(pkg, tile, lu, keep)
            });
            // The rivers and streams along their course (section WATR).
            let ribbons =
                crate::water::ribbon_arrays(pkg, key, keep, &|x, z| pkg.surface_height(x, z));
            crate::water::append(&mut w, ribbons);
            w
        },
        crowns: crate::flight_world::TreeCrowns::from_crowns(plants.crowns.clone()),
        plants,
        houses,
        bridges: crate::bridges::bridge_arrays(&own),
        solid,
        near_tint: look.near_tint,
        lamps,
        obstacles,
        ground_detail,
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
    pub(crate) water: Handle<crate::water::WaterMaterial>,
}

fn setup_materials(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut tree_materials: ResMut<Assets<SeeThroughMaterial>>,
    mut water: ResMut<Assets<crate::water::WaterMaterial>>,
) {
    let vertex_coloured = StandardMaterial {
        base_color: Color::WHITE,
        perceptual_roughness: 1.0,
        ..default()
    };
    commands.insert_resource(PackageMaterials {
        trees: tree_materials.add(crate::see_through::with_crown_alpha(
            crate::see_through::swaying(see_through(vertex_coloured.clone()), None),
        )),
        vertex_coloured: materials.add(vertex_coloured),
        water: water.add(crate::water::water_material()),
    });
}

/// A tile of member `.0` of the world set.
pub(crate) type TileId = (usize, TileKey);

#[derive(Resource, Default)]
pub(crate) struct TileStreamer {
    pub(crate) loaded: HashMap<TileId, Entity>,
    /// The terrain material of each loaded tile (the orthophoto corridor
    /// is put on it, [`crate::ortho`]).
    pub(crate) terrain: HashMap<TileId, Handle<TerrainMaterial>>,
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
    /// The season bucket the loaded tiles were built for
    /// ([`crate::region_look::season_bucket`]).
    season: Option<u32>,
    /// Whether they were built with the seasons (spec §14 Phase 9, S8;
    /// console `season.fx`).
    seasons_on: bool,
    /// Loaded tiles to build again (the season or world set changed; the
    /// old meshes stay until the new ones are uploaded).
    stale: HashSet<TileId>,
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

/// The local rider's position in home metres (in god mode the unicorn's).
pub(crate) fn rider_position(view: &RideView) -> Option<[f64; 2]> {
    crate::flight_world::stream_focus(view).map(|f| f.centre)
}

/// Riding: the tiles within [`STREAM_RADIUS`] of `p`.
fn riding_focus(p: [f64; 2]) -> StreamFocus {
    StreamFocus {
        centre: p,
        radius: STREAM_RADIUS,
        ahead: None,
    }
}

/// The tiles to show around `p` (home metres), nearest first: of every
/// package with loaded tiles, those within [`STREAM_RADIUS`].
pub fn wanted_tiles(set: &WorldSet, p: [f64; 2]) -> Vec<TileId> {
    wanted_tiles_for(set, &riding_focus(p))
}

/// The tiles to show for `focus` (god mode: wider with the height and
/// ahead of the flight, [`crate::flight_world::stream_focus`]), nearest to
/// its centre first.
pub fn wanted_tiles_for(set: &WorldSet, focus: &StreamFocus) -> Vec<TileId> {
    wanted_tile_rings(set, focus)
        .into_iter()
        .map(|(_, id)| id)
        .collect()
}

/// The tiles [`urgent_tiles`] builds at once after a jump: of the
/// [`wanted_tiles`], those up to [`URGENT_RING`], nearest first.
pub fn urgent_tiles(set: &WorldSet, p: [f64; 2]) -> Vec<TileId> {
    urgent_tiles_within(set, p, URGENT_RING)
}

/// [`urgent_tiles`] up to `ring`.
pub fn urgent_tiles_within(set: &WorldSet, p: [f64; 2], ring: i32) -> Vec<TileId> {
    wanted_tile_rings(set, &riding_focus(p))
        .into_iter()
        .filter(|&(r, _)| r <= ring)
        .map(|(_, id)| id)
        .collect()
}

/// Whether the rider moved from `last` to `p` by a jump ([`JUMP_M`]); the
/// first position (no `last`) is one too.
pub fn is_jump(last: Option<[f64; 2]>, p: [f64; 2]) -> bool {
    last.is_none_or(|q| (p[0] - q[0]).hypot(p[1] - q[1]) > JUMP_M)
}

/// [`wanted_tiles_for`] with their ring around the focus' centre.
fn wanted_tile_rings(set: &WorldSet, focus: &StreamFocus) -> Vec<(i32, TileId)> {
    let mut out: Vec<(i32, TileId)> = Vec::new();
    for (i, m) in set.members().iter().enumerate() {
        let Some(pkg) = m.package() else {
            continue;
        };
        let size = f64::from(pkg.tile_size_m);
        let centre = TileKey::containing(m.frame.invert(focus.centre), size);
        let ahead = focus
            .ahead
            .map(|a| TileKey::containing(m.frame.invert(a), size));
        let mut add = |around: TileKey, radius: i32| {
            for dz in -radius..=radius {
                for dx in -radius..=radius {
                    let key = TileKey {
                        x: around.x + dx,
                        z: around.z + dz,
                    };
                    if pkg.tiles.contains_key(&key) && !out.iter().any(|(_, t)| *t == (i, key)) {
                        out.push((key.ring(centre), (i, key)));
                    }
                }
            }
        };
        add(centre, focus.radius);
        if let Some(a) = ahead {
            add(a, crate::flight_world::AHEAD_RADIUS);
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
    far_tiles_for(set, &riding_focus(p), loaded, 1)
}

/// [`far_tiles`] for a [`StreamFocus`]: beyond `margin` rings more than
/// wanted around its centre and around the point ahead (1 as before; a
/// phone 0, [`TileBuildPace::keep_margin`]).
pub fn far_tiles_for<'a>(
    set: &WorldSet,
    focus: &StreamFocus,
    loaded: impl IntoIterator<Item = &'a TileId>,
    margin: i32,
) -> Vec<TileId> {
    loaded
        .into_iter()
        .filter(|(i, key)| {
            let m = &set.members()[*i];
            match m.package() {
                None => true,
                Some(pkg) => {
                    let size = f64::from(pkg.tile_size_m);
                    let centre = TileKey::containing(m.frame.invert(focus.centre), size);
                    let near_ahead = focus.ahead.is_some_and(|a| {
                        let a = TileKey::containing(m.frame.invert(a), size);
                        key.ring(a) <= crate::flight_world::AHEAD_RADIUS + margin
                    });
                    key.ring(centre) > focus.radius + margin && !near_ahead
                }
            }
        })
        .copied()
        .collect()
}

fn build_task(set: &Arc<WorldSet>, id: TileId, look: &TileLook) -> Task<Option<TileMeshes>> {
    let set = set.clone();
    let look = look.clone();
    AsyncComputeTaskPool::get().spawn(async move { build_now(&set, id, &look) })
}

/// The meshes of a tile, built on this thread.
fn build_now(set: &WorldSet, (i, key): TileId, look: &TileLook) -> Option<TileMeshes> {
    let pkg = set.members()[i].package()?.clone();
    let junctions = crate::junction_mesh::package_junctions(&pkg);
    if set.has_seams() {
        let keep = |q: [f64; 2]| set.draws(i, q);
        build_tile_meshes_with(&pkg, key, Some(&keep), look, junctions.as_deref())
    } else {
        build_tile_meshes_with(&pkg, key, None, look, junctions.as_deref())
    }
}

/// An urgent tile (after a jump) in this frame: natively the frame waits
/// for its task (they run in parallel); in the browser (spec §14 Phase 10
/// B2) the task pool runs on this one thread, so waiting for it would
/// never end – the tile is built right here instead.
fn finish_now(
    task: Task<Option<TileMeshes>>,
    set: &WorldSet,
    id: TileId,
    look: &TileLook,
) -> Option<TileMeshes> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (set, id, look);
        block_on(task)
    }
    #[cfg(target_arch = "wasm32")]
    {
        drop(task);
        build_now(set, id, look)
    }
}

/// Loads and drops neighbour tiles by the rider's distance (spec §2.2.2).
fn load_neighbours(
    mut commands: Commands,
    mut world: ResMut<PackageWorld>,
    view: Res<RideView>,
    mut loads: ResMut<NeighbourLoads>,
    scene: Option<Res<crate::late::SceneGeneration>>,
) {
    let scene = crate::late::current(scene.as_deref());
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
    let reach = crate::flight_world::neighbour_reach_m(&view);
    for r in world.set.plan_residency_reach(p, reach, &loading) {
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
        // The ground of this world set – not of the next one if a world
        // switch comes before the commands are applied (`late`).
        let (ground, pavement) = (world.ground(), world.pavement());
        crate::late::queue_if_current(&mut commands, scene, "neighbour tiles", move |w| {
            w.insert_resource(ground);
            w.insert_resource(pavement);
        });
    }
}

#[allow(clippy::too_many_arguments)] // a Bevy system
fn stream_tiles(
    mut commands: Commands,
    world: Res<PackageWorld>,
    view: Res<RideView>,
    mut streamer: ResMut<TileStreamer>,
    mut obstacles: ResMut<CameraObstacles>,
    mut flight_obstacles: ResMut<crate::flight_world::FlightObstacles>,
    library: Option<Res<MaterialLibrary>>,
    mut seasons: Option<ResMut<crate::season::SeasonCache>>,
    radius: Option<Res<StreamRadius>>,
    pace: Option<Res<TileBuildPace>>,
    (thinning, reach): (Option<Res<PlantThinning>>, Option<Res<NearTileReach>>),
) {
    let pace = pace.as_deref().copied().unwrap_or_default();
    let thinning = thinning.as_deref().copied().unwrap_or_default().0;
    let set = &world.set;
    let Some(mut focus) = crate::flight_world::stream_focus(&view) else {
        return;
    };
    // The render distance's ring (10G): a short distance meshes fewer tiles.
    let riding = view.flight.is_none() && view.guided_camera.is_none();
    if let Some(r) = radius.as_deref() {
        r.cap(&mut focus, riding);
    }
    // A phone's reach (10G): only the neighbour tiles near the rider.
    let reach = reach.and_then(|r| r.0).filter(|_| riding);
    // The road ahead, a margin farther for the tiles already meshed.
    let ahead = reach.map_or_else(Vec::new, |r| {
        road_ahead(&view, r * REACH_AHEAD + REACH_DROP_MARGIN_M)
    });
    let ahead_n = reach.map_or(0, |r| {
        ((r * REACH_AHEAD / REACH_AHEAD_STEP_M).ceil() as usize + 1).min(ahead.len())
    });
    let p = focus.centre;
    // The world date (the sky's local date at the rider, spec §14 Phase 9),
    // in buckets of a few days: the seasonal look changes with it.
    let season = view.sky.as_ref().map(|s| {
        crate::region_look::season_bucket(ocw_world::look::day_of_year(
            s.sky.local.year,
            s.sky.local.month,
            s.sky.local.day,
        ))
    });
    let day = season.map(crate::region_look::bucket_day);
    // The season of the world date at the rider (spec §14 Phase 9, S8).
    let (seasons_on, season_day, snow) = match seasons.as_deref_mut() {
        Some(c) => (c.enabled, c.day(&view), c.snow(&view)),
        None => (false, None, ocw_sky::season::Ramp::default()),
    };
    // The look of each member package: the ground set of its region, tinted
    // by its measured look (spec §14 Phase 8 T3).
    let looks: Vec<TileLook> = set
        .members()
        .iter()
        .map(|m| {
            let tz = m.package().and_then(|p| p.time_zone.as_deref());
            let ground = library.as_deref().map(|lib| lib.set_for(tz));
            let base = match (library.as_deref(), ground) {
                (Some(lib), Some(g)) => TileLook::of_library(lib, g),
                _ => TileLook::default(),
            };
            match m.package() {
                Some(p) => base.with_region(&p.look, ground, day),
                None => base,
            }
            .with_season(season_day.clone(), snow)
            .with_plant_thinning(thinning)
        })
        .collect();
    let look = move |i: usize| looks.get(i).cloned().unwrap_or_default();
    let textured = library.is_some();
    // Packages came or went, the material library arrived or the season
    // moved on: the tiles are built anew (the old meshes stay until the new
    // ones are uploaded). Tasks for the old set are discarded.
    if streamer.generation != set.generation()
        || streamer.textured != textured
        || streamer.season != season
        || streamer.seasons_on != seasons_on
    {
        streamer.generation = set.generation();
        streamer.textured = textured;
        streamer.season = season;
        streamer.seasons_on = seasons_on;
        streamer.building.clear();
        streamer.ready.clear();
        // Rebuilt like new ones, a few at a time ([`tiles_to_build`]).
        let stale: HashSet<TileId> = streamer
            .loaded
            .keys()
            .copied()
            .filter(|id| set.members()[id.0].is_resident())
            .collect();
        streamer.stale = stale;
    }
    let jumped = is_jump(streamer.last_position, p);
    streamer.last_position = Some(p);
    let mut rings = wanted_tile_rings(set, &focus);
    within_reach(set, &mut rings, p, &ahead[..ahead_n], reach);
    let wanted: Vec<TileId> = rings.iter().map(|&(_, id)| id).collect();
    if jumped {
        // Work for the old place is dropped (its tiles go below); loaded
        // tiles whose rebuild is dropped stay stale.
        let dropped: Vec<TileId> = streamer
            .building
            .keys()
            .copied()
            .chain(streamer.ready.iter().map(|(id, _)| *id))
            .filter(|id| !wanted.contains(id) && streamer.loaded.contains_key(id))
            .collect();
        streamer.stale.extend(dropped);
        streamer.building.retain(|id, _| wanted.contains(id));
        streamer.ready.retain(|(id, _)| wanted.contains(id));
    }
    let pending: HashSet<TileId> = streamer
        .building
        .keys()
        .copied()
        .chain(streamer.ready.iter().map(|(id, _)| *id))
        .collect();
    for id in tiles_to_build(
        &rings,
        |id| streamer.loaded.contains_key(&id),
        &streamer.stale,
        &pending,
        pace,
    ) {
        streamer.stale.remove(&id);
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
        let urgent: Vec<TileId> = urgent_tiles_within(set, p, pace.urgent_ring)
            .into_iter()
            .filter(|id| !streamer.loaded.contains_key(id) && wanted.contains(id))
            .collect();
        let mut done = Vec::new();
        for id in urgent {
            if let Some(task) = streamer.building.remove(&id)
                && let Some(meshes) = finish_now(task, set, id, &look(id.0))
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
    let mut far = far_tiles_for(set, &focus, streamer.loaded.keys(), pace.keep_margin);
    if let Some(r) = reach {
        let beyond: Vec<TileId> = beyond_reach(set, streamer.loaded.keys(), p, &ahead, r)
            .filter(|id| !far.contains(id))
            .collect();
        far.extend(beyond);
    }
    for id in far {
        streamer.terrain.remove(&id);
        if let Some(e) = streamer.loaded.remove(&id) {
            commands.entity(e).try_despawn();
            obstacles.remove_tile(id.0 as u32, (id.1.x, id.1.z));
            flight_obstacles
                .0
                .remove_tile(id.0 as u32, (id.1.x, id.1.z));
        }
        streamer.building.remove(&id);
        streamer.ready.retain(|(r, _)| *r != id);
        streamer.stale.remove(&id);
    }
}

/// The `loaded` tiles beyond `reach` + [`REACH_DROP_MARGIN_M`] of `p`
/// and off the road `ahead` (sampled that margin farther; the rider's own
/// tile is at 0: never; [`NearTileReach`]).
pub fn beyond_reach<'a>(
    set: &'a WorldSet,
    loaded: impl IntoIterator<Item = &'a TileId> + 'a,
    p: [f64; 2],
    ahead: &'a [[f64; 2]],
    reach: f64,
) -> impl Iterator<Item = TileId> + 'a {
    loaded
        .into_iter()
        .copied()
        .filter(move |&id| !in_reach(set, id, p, ahead, reach + REACH_DROP_MARGIN_M))
}

/// Tile builds running or waiting for their upload at a time, beyond the
/// urgent ring after a jump: a built tile keeps all its meshes until it is
/// uploaded (one a frame), and the browser's task pool runs every task at
/// once between two frames (spec §14 Phase 8 T8, the memory fix).
pub const MAX_PENDING_TILES: usize = 3;

/// The tiles to start building in this frame, nearest first: of the
/// `wanted` ones (with their ring), those not loaded (or loaded but
/// `stale`: built for another season or world set) and not `pending`
/// (being built or waiting for their upload) – the urgent ring always,
/// the others while fewer than [`MAX_PENDING_TILES`] are pending; then the
/// stale tiles no longer wanted (still loaded, until they are dropped).
/// `pace`: the urgent ring and the most pending ([`TileBuildPace`]).
pub(crate) fn tiles_to_build(
    wanted: &[(i32, TileId)],
    loaded: impl Fn(TileId) -> bool,
    stale: &HashSet<TileId>,
    pending: &HashSet<TileId>,
    pace: TileBuildPace,
) -> Vec<TileId> {
    let mut out = Vec::new();
    let mut busy = pending.len();
    let rest = {
        let mut r: Vec<TileId> = stale
            .iter()
            .copied()
            .filter(|id| !wanted.iter().any(|(_, w)| w == id))
            .collect();
        r.sort();
        r
    };
    let candidates = wanted
        .iter()
        .copied()
        .chain(rest.into_iter().map(|id| (i32::MAX, id)));
    for (ring, id) in candidates {
        if (loaded(id) && !stale.contains(&id)) || pending.contains(&id) || out.contains(&id) {
            continue;
        }
        if ring > pace.urgent_ring && busy >= pace.max_pending {
            continue;
        }
        busy += 1;
        out.push(id);
    }
    out
}

/// The road materials made so far, by marking style, and whether they
/// were made with the material library.
#[derive(Default)]
pub(crate) struct RoadMaterialCache {
    textured: bool,
    by_style: HashMap<usize, RoadMaterials>,
}

/// The material library and the road materials made from it.
type RoadParams<'w, 's> = (
    Option<Res<'w, MaterialLibrary>>,
    Option<ResMut<'w, Assets<RoadMaterial>>>,
    Local<'s, RoadMaterialCache>,
);

#[allow(clippy::too_many_arguments)] // a Bevy system
pub(crate) fn upload_tiles(
    mut commands: Commands,
    world: Res<PackageWorld>,
    mut streamer: ResMut<TileStreamer>,
    materials: Res<PackageMaterials>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut terrain: ResMut<Assets<TerrainMaterial>>,
    (mut obstacles, mut flight_obstacles): (ResMut<CameraObstacles>, ResMut<FlightObstacles>),
    tunnels: Option<Res<crate::tunnel::Tunnels>>,
    (library, mut road_materials, mut road_cache): RoadParams,
    (house, lamps): (
        Option<Res<HouseMaterial>>,
        Option<Res<crate::street_lamps::LampMaterials>>,
    ),
    scene: Option<Res<crate::late::SceneGeneration>>,
) {
    let scene = crate::late::current(scene.as_deref());
    let library = library.as_deref();
    // The road materials, one pair per marking style (made when first
    // needed, again when the library arrives).
    if road_cache.textured != library.is_some() {
        *road_cache = RoadMaterialCache {
            textured: library.is_some(),
            ..default()
        };
    }
    let no_cut = TunnelCut::default();
    let cut = tunnels.as_deref().map_or(&no_cut, |t| &t.cut);
    // One tile per frame, all urgent ones after a jump.
    let count = std::mem::take(&mut streamer.urgent).max(1);
    for _ in 0..count {
        let Some((id, t)) = streamer.ready.pop_front() else {
            return;
        };
        let style = world
            .set
            .members()
            .get(id.0)
            .and_then(|m| m.package())
            .map_or(0, |p| {
                crate::road_style::style_index(p.time_zone.as_deref())
            });
        let roads = road_materials.as_deref_mut().map(|assets| {
            road_cache
                .by_style
                .entry(style)
                .or_insert_with(|| {
                    RoadMaterials::new(assets, library, &crate::road_style::styles()[style])
                })
                .clone()
        });
        upload_tile(
            &mut commands,
            &world,
            &mut streamer,
            &materials,
            (&mut meshes, &mut images, &mut terrain),
            (&mut obstacles, &mut flight_obstacles),
            cut,
            (library, roads.as_ref(), house.as_deref().map(|h| &h.handle)),
            lamps.as_deref().map(|l| &l.heads),
            id,
            t,
        );
        // Spawned when the commands are applied: gone again if a world
        // switch came in between (`late`).
        if let Some(&e) = streamer.loaded.get(&id) {
            crate::late::despawn_if_stale(&mut commands, scene, "tile", e);
        }
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
    (obstacles, flight_obstacles): (&mut CameraObstacles, &mut FlightObstacles),
    cut: &TunnelCut,
    (library, roads, house): (
        Option<&MaterialLibrary>,
        Option<&RoadMaterials>,
        Option<&Handle<BuildingMaterial>>,
    ),
    lamp_heads: Option<&Handle<StandardMaterial>>,
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
    flight_obstacles.0.remove_tile(package, tile);
    // The tree crowns: only the flight keeps clear of them.
    let crowns = std::mem::take(&mut t.crowns);
    if !crowns.is_empty() {
        flight_obstacles.0.insert(
            ObstacleLayer {
                kind: "trees",
                package,
                tile,
            },
            move |x, z| {
                let q = frame.invert([x, z]);
                crowns.top_at(q[0] as f32, q[1] as f32).map(f64::from)
            },
        );
    }
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
        if let Some(tint) = t.near_tint {
            for (slot, c) in e.look.material_tint.iter_mut().zip(tint) {
                *slot = Vec4::new(c[0], c[1], c[2], 1.0);
            }
        }
        e.look.mode.y = splat.cells as f32;
        e.look.tile.x = member.package().map_or(1000.0, |p| p.tile_size_m);
        let (a, b) = splat.images();
        e.splat_a = Some(images.add(a));
        e.splat_b = Some(images.add(b));
        e.albedo = Some(ground.albedo.clone());
        e.normal = Some(ground.normal.clone());
    }
    let terrain_mat = terrain.add(terrain_mat);
    streamer.terrain.insert(id, terrain_mat.clone());
    let lamps = std::mem::take(&mut t.lamps);
    let obstacles = std::mem::take(&mut t.obstacles);
    let ground_detail = std::mem::take(&mut t.ground_detail);
    let detail_pkg = member.package().cloned();
    let parent = commands
        .spawn((
            frame_transform(&frame),
            Visibility::default(),
            Name::new(format!("tile {} {} of {}", t.key.x, t.key.z, member.id)),
            crate::street_lamps::LampSpots(lamps.spots.iter().map(|p| Vec3::from(*p)).collect()),
        ))
        .with_children(|c| {
            c.spawn((
                Mesh3d(crate::mesh_kind::add(
                    meshes,
                    {
                        t.terrain.drop_white();
                        std::mem::take(&mut t.terrain).mesh()
                    },
                    "terrain",
                )),
                MeshMaterial3d(terrain_mat),
                NotShadowCaster,
                CameraInteraction::Solid,
            ));
            if !t.water.indices.is_empty() {
                c.spawn((
                    Mesh3d(crate::mesh_kind::add(meshes, t.water.mesh(), "water")),
                    MeshMaterial3d(materials.water.clone()),
                    NotShadowCaster,
                ));
            }
            // Houses in three levels of detail (spec §14 Phase 8 T4,
            // `houses`): near chunks, the tile's mid and far meshes.
            let houses = std::mem::take(&mut t.houses);
            let ranges = crate::houses::lod_ranges();
            // (The quality preset scales the ranges: `quality::HouseLod`.)
            let parts = houses
                .near
                .into_iter()
                .map(|p| (p, 0u8, true))
                .chain(houses.mid.map(|p| (p, 1, true)))
                .chain(houses.far.map(|p| (p, 2, false)));
            for (part, lod, shadows) in parts {
                let range = ranges[usize::from(lod)].clone();
                let kind = ["houses near", "houses mid", "houses far"][usize::from(lod)];
                let mesh = Mesh3d(crate::mesh_kind::add(meshes, part.mesh.mesh(), kind));
                let at = Transform::from_translation(Vec3::from(part.origin));
                let mut e = match house {
                    Some(h) => c.spawn((mesh, MeshMaterial3d(h.clone()))),
                    None => c.spawn((mesh, MeshMaterial3d(mat.clone()))),
                };
                e.insert((
                    at,
                    range,
                    CameraInteraction::Solid,
                    TileDetail,
                    crate::quality::HouseLod(lod),
                ));
                if !shadows {
                    e.insert(NotShadowCaster);
                }
            }
            if !t.bridges.indices.is_empty() {
                c.spawn((
                    Mesh3d(crate::mesh_kind::add(meshes, t.bridges.mesh(), "bridges")),
                    MeshMaterial3d(mat.clone()),
                    CameraInteraction::Solid,
                ));
            }
            if !t.roads.indices.is_empty() {
                let mesh = Mesh3d(crate::mesh_kind::add(
                    meshes,
                    std::mem::take(&mut t.roads).road_mesh(),
                    "roads",
                ));
                // Roads and their markings lie on the terrain: no shadows of
                // their own (spec §4.5).
                match roads {
                    Some(road) => c.spawn((
                        mesh,
                        MeshMaterial3d(road.strips.clone()),
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
            // Crossings, stop and give-way lines: blended over the road.
            if let (Some(road), false) = (roads, t.overlays.indices.is_empty()) {
                c.spawn((
                    Mesh3d(crate::mesh_kind::add(
                        meshes,
                        std::mem::take(&mut t.overlays).road_mesh(),
                        "road overlays",
                    )),
                    MeshMaterial3d(road.overlays.clone()),
                    crate::world::FlatOverlay,
                ));
            }
            // The plants in chunks (spec §14 Phase 8 T8): the near models
            // up to `vegetation::NEAR_FADE_M`, the far ones beyond, as far as
            // the quality preset says, cross-faded. The near meshes are
            // built only near a camera (`vegetation::lazy_plants`).
            // Banana plants and vine rows: up close only.
            for chunk in std::mem::take(&mut t.plants.details) {
                c.spawn((
                    crate::vegetation::LazyPlants::detail(chunk.near, materials.trees.clone()),
                    Transform::from_translation(Vec3::from(chunk.origin)),
                    Visibility::default(),
                    CameraInteraction::SeeThrough,
                    TileDetail,
                    crate::quality::detail_plant_range(),
                ));
            }
            for chunk in std::mem::take(&mut t.plants.chunks) {
                let at = Transform::from_translation(Vec3::from(chunk.origin));
                if !chunk.near.is_empty() {
                    c.spawn((
                        crate::vegetation::LazyPlants::near(chunk.near, materials.trees.clone()),
                        at,
                        Visibility::default(),
                        CameraInteraction::SeeThrough,
                        TileDetail,
                        crate::quality::near_tree_range(),
                    ));
                }
                if !chunk.far.indices.is_empty() {
                    c.spawn((
                        Mesh3d(crate::mesh_kind::add(
                            meshes,
                            chunk.far.mesh(),
                            "plants far",
                        )),
                        MeshMaterial3d(materials.trees.clone()),
                        at,
                        CameraInteraction::SeeThrough,
                        TileDetail,
                        crate::quality::far_tree_range(None),
                        crate::quality::TileTrees,
                    ));
                }
            }
            // Street lamps (spec §14 Phase 9 S2): poles in the vertex
            // colours, the heads glowing at night.
            if !lamps.poles.indices.is_empty() {
                c.spawn((
                    Mesh3d(crate::mesh_kind::add(meshes, lamps.poles.mesh(), "lamps")),
                    MeshMaterial3d(materials.vertex_coloured.clone()),
                    TileDetail,
                ));
            }
            if let (Some(heads), false) = (lamp_heads, lamps.heads.indices.is_empty()) {
                c.spawn((
                    Mesh3d(crate::mesh_kind::add(meshes, lamps.heads.mesh(), "lamps")),
                    MeshMaterial3d(heads.clone()),
                    NotShadowCaster,
                    TileDetail,
                ));
            }
            // Lamps with a `lod1` (spec 16H Q24 (a)): near and far by
            // cell, switched at 60 m like every prop.
            for cell in lamps.lod {
                let at = Transform::from_translation(Vec3::from(cell.origin));
                let near = crate::lod::PROP.near_range();
                let far = crate::lod::PROP.far_range(None);
                for (range, [poles, heads]) in [(near, cell.near), (far, cell.far)] {
                    if !poles.indices.is_empty() {
                        c.spawn((
                            Mesh3d(crate::mesh_kind::add(meshes, poles.mesh(), "lamps")),
                            MeshMaterial3d(materials.vertex_coloured.clone()),
                            at,
                            range.clone(),
                            TileDetail,
                        ));
                    }
                    if let (Some(h), false) = (lamp_heads, heads.indices.is_empty()) {
                        c.spawn((
                            Mesh3d(crate::mesh_kind::add(meshes, heads.mesh(), "lamps")),
                            MeshMaterial3d(h.clone()),
                            at,
                            range,
                            NotShadowCaster,
                            TileDetail,
                        ));
                    }
                }
            }
            // World objects (spec §14 Phase 12 K3): per chunk, up to
            // `obstacles::DRAW_M`; the road barriers only with
            // `display.road_barriers` (Q7).
            // Near-field ground detail (spec §4.6.2): chunks near the
            // roads, their meshes built near a camera
            // (`ground_detail::lazy_ground`).
            if let Some(pkg) = &detail_pkg {
                crate::ground_detail::spawn_chunks(c, pkg, id.0, ground_detail);
            }
            for chunk in obstacles.chunks {
                let range = chunk.range();
                let mut e = c.spawn((
                    Mesh3d(crate::mesh_kind::add(
                        meshes,
                        chunk.mesh.mesh(),
                        "obstacles",
                    )),
                    MeshMaterial3d(materials.vertex_coloured.clone()),
                    Transform::from_translation(Vec3::from(chunk.origin)),
                    range,
                ));
                if chunk.road {
                    e.insert((Visibility::Hidden, crate::obstacles::RoadBarrierMesh));
                } else {
                    e.insert(TileDetail);
                }
            }
        })
        .id();
    if let Some(old) = streamer.loaded.insert(id, parent) {
        commands.entity(old).try_despawn();
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use ocw_world::buildings::{BuildingKind, Roof};
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
                dress: ocw_world::RoadDress::of_class(RoadClass::Secondary, 6.0),
                lit: ocw_world::RoadLit::Unknown,
                condition: Default::default(),
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
            look: Default::default(),
            obstacles: None,
            places: None,
            junctions: None,
            traffic: None,
            aero: None,
        }
    }

    /// The memory per resident tile (spec §14 Phase 10 B2): meshes up to
    /// 65,536 vertices get 16-bit indices with the same values, larger
    /// ones keep 32 bits.
    /// The packed vertex formats (10G *The first tiles' GPU memory*): a
    /// terrain vertex of a land-use tile takes 28 bytes instead of 48, a
    /// house's 48 instead of 56 (its colours carry codes), a road's 44.
    #[test]
    fn tile_meshes_upload_in_packed_vertex_formats() {
        use bevy::mesh::VertexAttributeValues as V;
        let stride = |m: &Mesh| m.get_mesh_vertex_buffer_layout(&mut Default::default());
        let arrays = |colour: [f32; 4], uv_sets: usize| MeshArrays {
            positions: vec![[0.0; 3]; 3],
            normals: vec![[0.0, 1.0, 0.0], [0.6, 0.8, 0.0], [-1.0, 0.0, 0.0]],
            colors: vec![colour; 3],
            uvs: if uv_sets > 0 {
                vec![[0.5; 2]; 3]
            } else {
                Vec::new()
            },
            uvs_b: if uv_sets > 1 {
                vec![[3.0; 2]; 3]
            } else {
                Vec::new()
            },
            indices: vec![0, 1, 2],
        };
        // A land-use terrain: white vertices, the colour map colours it.
        let mut terrain = arrays([1.0; 4], 1);
        terrain.drop_white();
        assert!(terrain.colors.is_empty());
        let m = terrain.mesh();
        assert!(m.attribute(Mesh::ATTRIBUTE_COLOR).is_none());
        assert_eq!(stride(&m).0.layout().array_stride, 28);
        // Normals in 16 bits, nearly unchanged.
        let Some(V::Snorm16x4(n)) = m.attribute(ATTRIBUTE_NORMAL_PACKED) else {
            panic!("packed normals");
        };
        assert_eq!(n[0], [0, 32767, 0, 0]);
        assert_eq!(n[1], [19660, 26214, 0, 0]);
        assert_eq!(n[2], [-32767, 0, 0, 0]);
        // Colours within 0 – 1 in 16 bits; a tinted terrain keeps them.
        let mut tinted = arrays([0.2, 0.5, 0.1, 1.0], 1);
        tinted.drop_white();
        let m = tinted.mesh();
        let Some(V::Unorm16x4(c)) = m.attribute(ATTRIBUTE_COLOR_PACKED) else {
            panic!("packed colours");
        };
        assert_eq!(c[0], [13107, 32768, 6554, 65535]);
        assert_eq!(stride(&m).0.layout().array_stride, 36);
        // A house's colours carry codes (alpha 3): floats, as before.
        let m = arrays([0.2, 0.5, 0.1, 3.0], 2).mesh();
        assert!(matches!(
            m.attribute(Mesh::ATTRIBUTE_COLOR),
            Some(V::Float32x4(_))
        ));
        // 12 + 8 + 16 + 8 + 4 (its codes in 16-bit floats).
        assert_eq!(stride(&m).0.layout().array_stride, 48);
        // A road: the half width over ROAD_HALF_SCALE, colours in 16 bits
        // (44 bytes instead of 56); the shader multiplies it back.
        let m = arrays([0.2, 0.5, 0.1, 3.75], 2).road_mesh();
        let Some(V::Unorm16x4(c)) = m.attribute(ATTRIBUTE_COLOR_PACKED) else {
            panic!("packed road colours");
        };
        assert!((f32::from(c[0][3]) / 65535.0 * ROAD_HALF_SCALE - 3.75).abs() < 0.001);
        assert!(
            include_str!("road.wesl").contains(&format!(
                "const ROAD_HALF_SCALE: f32 = {ROAD_HALF_SCALE:.1};"
            )),
            "road.wesl's scale"
        );
        // Whole codes up to 2048 in 16-bit floats (uv_b = 3.0 here): 40.
        assert!(matches!(
            m.attribute(ATTRIBUTE_UV_1_CODES),
            Some(V::Float16x2(b)) if b[0][0].to_f32() == 3.0
        ));
        assert_eq!(stride(&m).0.layout().array_stride, 40);
        // Codes beyond (a road's seed bits) or fractions stay floats.
        assert!(pack_codes(&[[2049.0, 0.0]]).is_none());
        assert!(pack_codes(&[[0.5, 0.0]]).is_none());
        assert!(pack_codes(&[[2048.0, -7.0]]).is_some());
        // The same attribute ids: the shaders' locations stay.
        assert_eq!(ATTRIBUTE_NORMAL_PACKED.id, Mesh::ATTRIBUTE_NORMAL.id);
        assert_eq!(ATTRIBUTE_COLOR_PACKED.id, Mesh::ATTRIBUTE_COLOR.id);
    }

    /// A phone's reach (10G): of the 3 × 3 tiles around the rider only
    /// those whose ground is within the reach are meshed; a meshed tile
    /// goes again only beyond the reach and a margin.
    #[test]
    fn a_phones_reach_meshes_only_the_near_neighbour_tiles() {
        let mut p = package();
        let t = p.tiles[&TileKey { x: 0, z: 0 }].clone();
        p.tiles = (-1..=1)
            .flat_map(|z| (-1..=1).map(move |x| TileKey { x, z }))
            .map(|key| (key, Tile { key, ..t.clone() }))
            .collect();
        let set = WorldSet::new("r", Arc::new(p));
        let at = [20.0, 50.0];
        let focus = riding_focus(at);
        let mut rings = wanted_tile_rings(&set, &focus);
        assert_eq!(rings.len(), 9);
        let id = |x, z| (0, TileKey { x, z });
        assert_eq!(tile_distance(&set, id(0, 0), at), 0.0);
        assert_eq!(tile_distance(&set, id(-1, 0), at), 20.0);
        assert!((tile_distance(&set, id(-1, -1), at) - 50f64.hypot(20.0)).abs() < 1e-9);
        let mut near = rings.clone();
        within_reach(&set, &mut near, at, &[], Some(30.0));
        let sorted = |r: &[(i32, TileId)]| {
            let mut k: Vec<TileId> = r.iter().map(|r| r.1).collect();
            k.sort();
            k
        };
        assert_eq!(sorted(&near), vec![id(-1, 0), id(0, 0)]);
        // The road ahead runs north into tile (0, 1): it is meshed too.
        let ahead = [[20.0, 60.0], [20.0, 90.0], [20.0, 110.0]];
        within_reach(&set, &mut rings, at, &ahead, Some(30.0));
        assert_eq!(sorted(&rings), vec![id(-1, 0), id(0, 0), id(0, 1)]);
        // Without a reach: all nine.
        let mut all = wanted_tile_rings(&set, &focus);
        within_reach(&set, &mut all, at, &[], None);
        assert_eq!(all.len(), 9);
        // Dropped beyond reach + margin and off the road only; the own tile
        // never.
        let loaded = [id(0, 0), id(-1, 0), id(1, 0), id(0, 1)];
        assert!(
            beyond_reach(&set, &loaded, at, &ahead, 30.0)
                .next()
                .is_none()
        );
        let gone: Vec<TileId> =
            beyond_reach(&set, &loaded, at, &ahead, -REACH_DROP_MARGIN_M + 10.0).collect();
        assert_eq!(gone, vec![id(-1, 0), id(1, 0)]);
        let gone: Vec<TileId> =
            beyond_reach(&set, &loaded, at, &[], -REACH_DROP_MARGIN_M + 10.0).collect();
        assert_eq!(gone, vec![id(-1, 0), id(1, 0), id(0, 1)]);
    }

    #[test]
    fn small_meshes_get_16_bit_indices() {
        let small = indices(vec![0, 1, 65_535], 65_536);
        assert!(matches!(&small, Indices::U16(v) if v == &[0, 1, 65_535]));
        let large = indices(vec![0, 65_536], 65_537);
        assert!(matches!(&large, Indices::U32(v) if v == &[0, 65_536]));
        let m = MeshArrays {
            positions: vec![[0.0; 3]; 3],
            normals: vec![[0.0, 1.0, 0.0]; 3],
            colors: vec![[1.0; 4]; 3],
            indices: vec![0, 1, 2],
            ..Default::default()
        }
        .mesh();
        assert!(matches!(m.indices(), Some(Indices::U16(v)) if v == &[0, 1, 2]));
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
        // Surface: 11 points × 2; per side the edge top, the lip, the
        // shoulder (outer and its own inner row) and the skirt bottom.
        assert_eq!(m.positions.len(), 22 + 2 * 5 * 11);
        // Surface is 6 m wide (z 47 … 53), slightly above the centreline.
        assert!((m.positions[0][2] - 47.0).abs() < 1e-4 || (m.positions[0][2] - 53.0).abs() < 1e-4);
        assert!((m.positions[0][1] - 5.02).abs() < 1e-4);
        // Without terrain the skirts reach the plain depth down.
        let min_y = m
            .positions
            .iter()
            .map(|p| p[1])
            .fold(f32::INFINITY, f32::min);
        assert!(min_y < 5.0 - 1.5);
    }

    /// Spec §4.8.6: a bridge of the `BRDG` section is drawn with its tile,
    /// and the road strip on its deck has no skirt or shoulder hanging
    /// below it; off the deck the road stands on a wall where the ground
    /// falls away, and a cutting gets a narrow verge (Phase 8 T5).
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
        // Points 3 … 7 lie on the deck: nothing of the road hangs below it.
        let on_deck: Vec<f32> = t
            .roads
            .positions
            .iter()
            .filter(|q| (29.9..=70.1).contains(&q[0]))
            .map(|q| q[1])
            .collect();
        assert!(!on_deck.is_empty());
        assert!(on_deck.iter().all(|&y| y >= 5.02 - 1e-3), "{on_deck:?}");
        // At x = 0 the road is 5 m above the terrain: a wall reaches down
        // to 0.3 m below the ground there (not a floating slab).
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
            at_end
                .iter()
                .all(|&y| y >= 5.02 - crate::road_mesh::EDGE_DEPTH_M - 1e-3),
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
    fn trees_follow_the_season() {
        use ocw_sky::season::SeasonModel;
        // Orchard trees (all deciduous) and forest trees at 450 m in
        // Liechtenstein.
        let at = |x: f32| (Vec3::new(x, 450.0, 0.0), 1.0);
        let sites: Vec<(TreeSite, Option<LandClass>)> = (0..40)
            .map(|i| (at(i as f32 * 7.3), Some(LandClass::Orchard)))
            .collect();
        let m = SeasonModel::for_place(47.14, 9.52);
        let jan = seasonal_tree_arrays(None, &sites, Some(&m.day(15.0)));
        let jul = seasonal_tree_arrays(None, &sites, Some(&m.day(182.0)));
        let oct = seasonal_tree_arrays(None, &sites, Some(&m.day(300.0)));
        // Bare in January: branches only (twig coloured, fewer vertices
        // than the round crowns of July).
        assert!(jan.positions.len() < jul.positions.len());
        let twig = crate::season::twig();
        assert!(jan.colors.iter().any(|c| (c[0] - twig[0]).abs() < 1e-6));
        // Summer green, autumn red-gold crowns.
        let reddest = |a: &MeshArrays| {
            a.colors
                .iter()
                .map(|c| c[0] - c[1])
                .fold(f32::MIN, f32::max)
        };
        assert!(reddest(&oct) > reddest(&jul) + 0.1);
        assert_eq!(jul.uvs.len(), jul.positions.len());
        assert_eq!(jul.normals.len(), jul.positions.len());
        assert!(
            jul.indices
                .iter()
                .all(|&i| (i as usize) < jul.positions.len())
        );
        // Without a season: the conifers of old, the same as tree_arrays.
        let plain: Vec<TreeSite> = sites.iter().map(|(s, _)| *s).collect();
        assert_eq!(
            seasonal_tree_arrays(None, &sites, None).positions,
            tree_arrays(&plain).positions
        );
        // Deterministic.
        assert_eq!(
            seasonal_tree_arrays(None, &sites, Some(&m.day(300.0))).colors,
            oct.colors
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
    /// Regression (2026-11-26, the Port d'Envalira): the bare high slopes
    /// came out sand-yellow – lowland meadow in dormant straw under the
    /// satellite's beige, the rock texture tinted towards the measured
    /// "rock" colour. Above the tree line the grass is turf between scree
    /// and rock (the splat's stony layers), the far colours are grey-brown,
    /// and the rock layer keeps its grey.
    #[test]
    fn bare_high_ground_is_turf_and_stone_not_sand() {
        let mut p = package();
        p.origin = (42.54, 1.60);
        let key = *p.tiles.keys().next().unwrap();
        p.tiles.get_mut(&key).unwrap().heights.fill(2400.0);
        let cells = 30usize;
        let classes: Vec<LandClass> = (0..cells * cells)
            .map(|j| match (j % cells) / 10 {
                0 => LandClass::Meadow,
                1 => LandClass::Rock,
                _ => LandClass::Scree,
            })
            .collect();
        let lu = LandUseTile {
            classes,
            water: Vec::new(),
        };
        p.landuse = Some(ocw_world::landuse::LandUse {
            cells: cells as u32,
            tiles: BTreeMap::from([(key, lu.clone())]),
        });
        // Andorra's measured palette and the satellite's beige there.
        let mut palette: Vec<[f32; 3]> = LandClass::ALL.iter().map(|c| land_colour(*c)).collect();
        let rgb = |c: [u8; 3]| c.map(|v| f32::from(v) / 255.0);
        palette[LandClass::Default as usize] = rgb([74, 78, 47]);
        palette[LandClass::Meadow as usize] = rgb([72, 76, 44]);
        palette[LandClass::Rock as usize] = rgb([120, 115, 81]);
        palette[LandClass::Scree as usize] = rgb([142, 130, 98]);
        let sat = SatelliteMap {
            cells: 2,
            rgb: vec![[121, 107, 85]; 4],
        };
        for doy in [200.0, 281.0] {
            let day = ocw_sky::season::SeasonModel::for_place(42.54, 1.60).day(doy);
            let map = colour_map_seasonal(
                &p,
                &p.tiles[&key],
                &lu,
                cells as u32,
                Some(&palette),
                Some(&sat),
                Some(&day),
            );
            for t in map.data[..cells * cells * 4].chunks(4) {
                let c = [t[0], t[1], t[2]].map(|v| f32::from(v) / 255.0);
                // Russet dormant turf (v6.72; a hue under 30°, red-brown)
                // is not sand.
                let russet =
                    c[0] > c[1] && c[0] > c[2] && (c[1] - c[2]) / (c[0] - c[2]).max(1e-6) < 0.5;
                assert!(
                    !crate::region_look::is_sand(c) || russet,
                    "day {doy}: {c:?}"
                );
            }
        }
        // The near ground: the meadow's corners have scree and rock.
        let s = tile_splat(&p, key, cells as u32, &crate::ground::default_mix());
        let w = |i: usize| {
            let (a, b) = (s.a[i], s.b[i]);
            [a[0], a[1], a[2], a[3], b[0], b[1], b[2], b[3]].map(|v| f32::from(v) / 255.0)
        };
        let meadow = w(5 * (cells + 1) + 5);
        assert!(
            meadow[crate::ground::SCREE] + meadow[crate::ground::ROCK] > 0.3,
            "{meadow:?}"
        );
        // Down in the valley a meadow stays a meadow.
        p.tiles.get_mut(&key).unwrap().heights.fill(1000.0);
        let s = tile_splat(&p, key, cells as u32, &crate::ground::default_mix());
        assert_eq!(s.a[5 * (cells + 1) + 5][crate::ground::MEADOW], 255);
        // The rock layer's tint keeps it grey.
        let pal = ocw_world::look::Palette {
            sources: Vec::new(),
            entries: vec![ocw_world::look::PaletteEntry {
                key: ocw_world::look::PaletteKey::Land(LandClass::Rock),
                rgb: [120, 115, 81],
                samples: 100,
                source: 0,
            }],
        };
        let means = [[0.2, 0.2, 0.2]; crate::ground::GROUND_LAYERS];
        let t = crate::region_look::layer_tints(&pal, &means)[crate::ground::ROCK];
        let tinted = Color::linear_rgb(0.2 * t[0], 0.2 * t[1], 0.2 * t[2]).to_srgba();
        assert!(!crate::region_look::is_sand([
            tinted.red,
            tinted.green,
            tinted.blue
        ]));
        assert!(t[0] / t[2] < 1.3, "{t:?}");
    }

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
                tags: Default::default(),
                parts: Default::default(),
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
        assert_eq!(half.plants.trees(), west_sites);
        assert_eq!(full.plants.trees(), sites.len());
        // The house (centroid x = 16) stays; the roads are always drawn.
        assert_eq!(half.houses.triangles(), full.houses.triangles());
        assert_eq!(half.roads.indices.len(), full.roads.indices.len());
        let east = |q: [f64; 2]| q[0] >= 50.0;
        let other = build_tile_meshes_kept(&p, key, Some(&east)).unwrap();
        assert!(other.houses.is_empty());
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
        a.graph = Some(graph.clone().into());
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
        b.graph = Some(g.into());
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
        // A phone's margin of 0 (the iPhone's memory, v5.42): a tile two
        // rings out of a 3 × 3 ring goes at once; with the margin of 1 it
        // stays.
        let focus = crate::flight_world::StreamFocus {
            centre: [0.0, 50.0],
            radius: 1,
            ahead: None,
        };
        let two_out = (0usize, TileKey { x: -2, z: 0 });
        assert!(loaded.contains(&two_out), "{loaded:?}");
        assert!(!far_tiles_for(&set, &focus, &loaded, 1).contains(&two_out));
        assert!(far_tiles_for(&set, &focus, &loaded, 0).contains(&two_out));
    }

    /// The render distance's ring (spec §14 Phase 10, 10G) caps the riding
    /// ring, never widens it and leaves flight alone.
    #[test]
    fn the_render_distance_caps_the_riding_ring() {
        let focus = |radius| crate::flight_world::StreamFocus {
            centre: [0.0, 0.0],
            radius,
            ahead: None,
        };
        let mut f = focus(STREAM_RADIUS);
        StreamRadius(1).cap(&mut f, true);
        assert_eq!(f.radius, 1);
        let mut f = focus(STREAM_RADIUS);
        StreamRadius(9).cap(&mut f, true);
        assert_eq!(f.radius, STREAM_RADIUS);
        let mut f = focus(3);
        StreamRadius(1).cap(&mut f, false);
        assert_eq!(f.radius, 3, "flight keeps its ring");
        let mut f = focus(STREAM_RADIUS);
        StreamRadius(0).cap(&mut f, true);
        assert_eq!(f.radius, 1, "at least the 3 × 3 tiles");
        assert_eq!(StreamRadius::default().0, STREAM_RADIUS);
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
            parts: Vec::new(),
            crash: None,
            wobble: 0.0,
            damage: None,
            cue: Default::default(),
            signs: Default::default(),
        });
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<Image>()
            .init_asset::<StandardMaterial>()
            .init_asset::<SeeThroughMaterial>()
            .init_asset::<TerrainMaterial>()
            .init_asset::<crate::water::WaterMaterial>()
            .init_asset::<crate::far_terrain::FarMaterial>()
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
        assert_eq!(m.plants.trees(), sites.len());
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
        assert!(!m.houses.is_empty());
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
            ..Default::default()
        };
        let m = build_tile_meshes_look(&p, key, None, &look).unwrap();
        let splat = m.splat.expect("splat weights");
        assert_eq!(splat.cells, 20);
        assert_eq!(splat.a.len(), 21 * 21);
        // Corner (2, 15) lies in the forest: its mix (forest floor 60 %).
        assert_eq!(splat.a[15 * 21 + 2][crate::ground::FOREST_FLOOR], 153);
        // The colour map takes the material means: the forest is blue now
        // (darkened by the canopy shade), under the green of the canopy
        // (v6.58: `COLOUR_MAP_CANOPY` of it).
        let map = m.colour_map.unwrap();
        let texel = |c: usize, r: usize, ch: usize| map.data[(r * 20 + c) * 4 + ch];
        assert!(texel(2, 15, 2) > 60, "{}", texel(2, 15, 2));
        assert!(texel(2, 15, 0) < 30 && texel(2, 15, 1) > texel(2, 15, 0));
        // Roads: layer and metres in the texture coordinates, colour from
        // the library.
        assert_eq!(m.roads.uvs.len(), m.roads.positions.len());
        assert_eq!(m.roads.uvs_b.len(), m.roads.positions.len());
        // The carriageway (kind 0) in the asphalt layer; the shoulders,
        // lips and skirts in their own layers.
        let surface = |u: &[f32; 2]| crate::road_mesh::decode_b(u[0]).1 == 0;
        let n = m.roads.uvs_b.iter().filter(|u| surface(u)).count();
        assert_eq!(n, 22);
        assert!(
            m.roads
                .uvs_b
                .iter()
                .filter(|u| surface(u))
                .all(|u| crate::road_mesh::decode_b(u[0]).0 == 0),
            "asphalt layer"
        );
        let along = m.roads.uvs.iter().map(|u| u[1]).fold(0.0f32, f32::max);
        assert!((along - 100.0).abs() < 0.5, "{along}");
        // Colour from the library; the alpha is the half width (shader).
        assert_eq!(m.roads.colors[0], [0.1, 0.2, 0.3, 3.0]);
        let c = road_colour(RoadClass::Secondary, RoadSurface::Asphalt);
        assert_eq!(flat.roads.colors[0], [c[0], c[1], c[2], 3.0]);
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
            dress: ocw_world::RoadDress::of_class(RoadClass::Residential, 4.0),
            lit: ocw_world::RoadLit::Unknown,
            condition: Default::default(),
        };
        let m = road_arrays(&[strip]);
        assert_eq!(m.roads_layer(), Some(4.0));
    }

    /// Houses (spec §14 Phase 8 T4): a tile's buildings in three levels
    /// of detail – near chunks with the overhanging roof, the merged mid
    /// mesh inside it, the far mesh without small buildings – and as
    /// camera-solid prisms up to their tops.
    #[test]
    fn tile_houses_come_in_three_levels_of_detail() {
        let key = TileKey { x: 0, z: 0 };
        let mut p = landuse_package();
        let house = p.buildings.as_ref().unwrap()[&key][0].clone();
        // A garden shed (12 m²): near and mid, not far.
        let shed = Building {
            kind: BuildingKind::Small,
            outline: vec![[40.0, 20.0], [44.0, 20.0], [44.0, 23.0], [40.0, 23.0]],
            roof: Roof::Gabled { height_m: 1.0 },
            ..house.clone()
        };
        p.buildings
            .as_mut()
            .unwrap()
            .get_mut(&key)
            .unwrap()
            .push(shed);
        let m = build_tile_meshes(&p, key).unwrap();
        let [near, mid, far] = m.houses.triangles();
        assert!(near > mid && mid > far && far > 0, "{near} {mid} {far}");
        // Both buildings lie in the first 250 m chunk of the 100 m tile.
        assert_eq!(m.houses.near.len(), 1);
        let n = &m.houses.near[0];
        assert_eq!(n.origin[0], 125.0);
        let mid_part = m.houses.mid.as_ref().unwrap();
        assert_eq!([mid_part.origin[0], mid_part.origin[2]], [50.0, 50.0]);
        // Positions are relative to the origins: the house spans x 10 … 22.
        let xs = |p: &crate::houses::HousePart| {
            p.mesh
                .positions
                .iter()
                .map(|q| q[0] + p.origin[0])
                .fold((f32::MAX, f32::MIN), |(a, b), x| (a.min(x), b.max(x)))
        };
        let (lo, _) = xs(n);
        assert!(lo < 10.0 - 0.2, "the near roof overhangs: {lo}");
        let (lo, hi) = xs(mid_part);
        assert!(lo > 10.0 && hi < 44.0, "mid inside the walls: {lo} {hi}");
        // The ranges: near fades before mid, far starts before mid ends.
        let r = crate::houses::lod_ranges();
        assert!(r[0].end_margin.end < r[1].end_margin.start);
        assert!(r[2].start_margin.start < r[1].end_margin.start);
        // Camera-solid up to the ridge (or more: a steeper regional roof).
        let top = m.solid.top_at(16.0, 74.0).expect("solid house");
        assert!(top >= 7.5 + 3.0 * 0.5, "{top}");
    }

    use bevy::ecs::system::System;

    /// A streaming app as in a ride on `pkg` (the rider 50 m along the
    /// road at z = 50 m, a camera there).
    fn stream_app(pkg: WorldPackage) -> App {
        use ocw_core::RiderId;
        use ocw_world::Route;
        let pts = (0..=100)
            .map(|i| [f64::from(i) * 10.0, 5.0, 50.0])
            .collect();
        let route = Route::from_points("r", "R", pts, false, 6.0).unwrap();
        let mut view = RideView::new(route);
        view.riders.push(crate::RiderView {
            id: RiderId(1),
            name: "Ada Rider".into(),
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
            parts: Vec::new(),
            crash: None,
            wobble: 0.0,
            damage: None,
            cue: Default::default(),
            signs: Default::default(),
        });
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<Image>()
            .init_asset::<StandardMaterial>()
            .init_asset::<SeeThroughMaterial>()
            .init_asset::<TerrainMaterial>()
            .init_asset::<crate::water::WaterMaterial>()
            .init_asset::<crate::far_terrain::FarMaterial>()
            .init_resource::<CameraObstacles>()
            .insert_resource(PackageWorld::single(Arc::new(pkg)))
            .insert_resource(view)
            .add_plugins(PackagePlugin);
        app.world_mut()
            .resource_mut::<crate::ground_detail::GroundDetailSettings>()
            .mode = crate::ground_detail::GroundDetailMode::Full;
        app.world_mut().spawn((
            Camera3d::default(),
            Camera::default(),
            GlobalTransform::from_translation(Vec3::new(50.0, 7.0, 50.0)),
        ));
        app
    }

    /// The streaming systems of [`PackagePlugin`], run by hand: a frame like
    /// the app's, or the race of 2026-11-27 – the systems run, then a ride
    /// switch ([`crate::rebuild_scene`], exclusive and unordered with them in
    /// the app) despawns the scene, then their commands are applied.
    struct Racer(Vec<Box<dyn System<In = (), Out = ()>>>);

    impl Racer {
        fn new(world: &mut World) -> Self {
            let mut v: Vec<Box<dyn System<In = (), Out = ()>>> = vec![
                Box::new(IntoSystem::into_system(load_neighbours)),
                Box::new(IntoSystem::into_system(stream_tiles)),
                Box::new(IntoSystem::into_system(upload_tiles)),
                Box::new(IntoSystem::into_system(
                    crate::far_terrain::update_far_terrain,
                )),
                Box::new(IntoSystem::into_system(crate::vegetation::lazy_plants)),
                Box::new(IntoSystem::into_system(crate::ground_detail::lazy_ground)),
            ];
            for s in &mut v {
                s.initialize(world);
            }
            Racer(v)
        }

        /// A frame like the app's (each system's commands applied after it).
        fn frame(&mut self, world: &mut World) {
            for s in &mut self.0 {
                s.run((), world).expect("runs");
            }
        }

        /// The systems, the switch, then their commands.
        fn switch(&mut self, world: &mut World) {
            for s in &mut self.0 {
                s.run_without_applying_deferred((), world).expect("runs");
            }
            crate::rebuild_scene(world);
            for s in &mut self.0 {
                s.apply_deferred(world);
            }
        }
    }

    /// The tile parents in the scene and those the streamer knows.
    fn tiles(app: &mut App) -> (usize, usize) {
        let w = app.world_mut();
        let drawn = w
            .query_filtered::<(), With<crate::street_lamps::LampSpots>>()
            .iter(w)
            .count();
        (drawn, w.resource::<TileStreamer>().loaded.len())
    }

    fn count<C: Component>(app: &mut App) -> usize {
        let w = app.world_mut();
        w.query_filtered::<(), With<C>>().iter(w).count()
    }

    /// Updates until `done` (the builds run on the task pool's threads).
    /// Bounded by wall time, not frames: on a loaded build machine the
    /// task pool's builds took longer than 3000 short frames (a flaky
    /// failure in a merge check, 2026-11-27).
    fn until(app: &mut App, what: &str, mut done: impl FnMut(&mut App) -> bool) {
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_secs(120) {
            if done(app) {
                return;
            }
            app.update();
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let state = (
            count::<crate::ground_detail::LazyGround>(app),
            count::<crate::ground_detail::GroundBuild>(app),
            count::<crate::vegetation::LazyPlants>(app),
            count::<crate::vegetation::LazyBuild>(app),
            tiles(app),
        );
        panic!(
            "{what}: not reached (ground chunks, builds, plant chunks, builds, tiles: {state:?})"
        );
    }

    /// Updates until a build of the task pool is done but not yet taken by
    /// its system (`done`), bounded by wall time like [`until`]. While a
    /// build is running (`running`) it waits without updating. With
    /// [`until`] the moment depended on the scheduling: a build that
    /// finished during an update, before its system polled it (a busy
    /// task pool, a long frame), was taken in that same update and never
    /// seen in flight – with the one far terrain build it never came
    /// again (the flaky "far terrain in flight: not reached" of the merge
    /// checks under load, 2026-11-28).
    fn until_built(
        app: &mut App,
        what: &str,
        running: impl Fn(&mut App) -> bool,
        done: impl Fn(&mut App) -> bool,
    ) {
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_secs(120) {
            if done(app) {
                return;
            }
            if running(app) {
                std::thread::sleep(std::time::Duration::from_millis(1));
            } else {
                app.update();
            }
        }
        panic!("{what}: not reached in 120 s");
    }

    /// Ground-detail and plant builds done on the task pool but not yet
    /// put on their chunks, and the far terrain built but not yet spawned.
    fn in_flight(app: &mut App) -> (bool, bool, bool) {
        let w = app.world_mut();
        let ground = w
            .query::<&crate::ground_detail::GroundBuild>()
            .iter(w)
            .any(crate::ground_detail::GroundBuild::is_finished);
        let plants = w
            .query::<&crate::vegetation::LazyBuild>()
            .iter(w)
            .any(crate::vegetation::LazyBuild::is_finished);
        let far = w
            .resource::<crate::far_terrain::FarTerrain>()
            .built_unspawned();
        (ground, plants, far)
    }

    /// A ride or world switch while tiles, ground detail, plants and the far
    /// terrain are still being built (the panic of 2026-11-27: a ground
    /// detail mesh inserted into a chunk the switch had despawned): no
    /// panic, the old results are dropped – nothing of the old scene is left
    /// – and the new scene's tiles and chunks get their own.
    #[test]
    fn a_world_switch_drops_the_results_still_in_flight() {
        let mut pkg = landuse_package();
        let key0 = TileKey { x: 0, z: 0 };
        let tile = pkg.tiles[&key0].clone();
        let lu = pkg.landuse.as_ref().unwrap().tiles[&key0].clone();
        let keys: Vec<TileKey> = (0..6).map(|x| TileKey { x, z: 0 }).collect();
        pkg.tiles = keys
            .iter()
            .map(|&key| {
                (
                    key,
                    Tile {
                        key,
                        ..tile.clone()
                    },
                )
            })
            .collect();
        pkg.landuse.as_mut().unwrap().tiles = keys.iter().map(|&k| (k, lu.clone())).collect();
        let mut app = stream_app(pkg);
        app.update();
        let mut racer = Racer::new(app.world_mut());
        // Its first frame sees every chunk as new (and skips it).
        racer.frame(app.world_mut());
        // 1. Ground detail (and plants) built, not yet on their chunks.
        until_built(
            &mut app,
            "ground detail in flight",
            |app| count::<crate::ground_detail::GroundBuild>(app) > 0,
            |app| in_flight(app).0,
        );
        racer.switch(app.world_mut());
        assert_eq!(tiles(&mut app), (0, 0), "tiles of the old scene left");
        assert_eq!(count::<crate::ground_detail::LazyGround>(&mut app), 0);
        assert_eq!(count::<crate::ground_detail::GroundPatches>(&mut app), 0);
        assert_eq!(count::<crate::vegetation::LazyPlants>(&mut app), 0);
        // 2. A teleport in the switch's frame: its tiles are built and
        // uploaded in that frame, spawned after the switch – and despawned
        // again.
        until(&mut app, "tiles again", |app| tiles(app).1 >= 2);
        app.world_mut().resource_mut::<RideView>().riders[0]
            .state
            .distance = 450.0;
        racer.switch(app.world_mut());
        assert_eq!(tiles(&mut app), (0, 0), "tiles of the old scene left");
        // 3. The far terrain built, not yet spawned: the same.
        until_built(
            &mut app,
            "far terrain in flight",
            |app| {
                app.world()
                    .resource::<crate::far_terrain::FarTerrain>()
                    .building()
            },
            |app| in_flight(app).2,
        );
        racer.switch(app.world_mut());
        assert_eq!(tiles(&mut app), (0, 0), "tiles of the old scene left");
        assert_eq!(count::<crate::far_terrain::FarTile>(&mut app), 0);
        // 4. The new scene streams its own (back at the start, by the
        // camera): tiles, ground detail, plants and one far terrain (not
        // two).
        app.world_mut().resource_mut::<RideView>().riders[0]
            .state
            .distance = 50.0;
        until(&mut app, "new tiles", |app| tiles(app).1 >= 3);
        until(&mut app, "new ground detail", |app| {
            let w = app.world_mut();
            w.query::<&crate::ground_detail::LazyGround>()
                .iter(w)
                .any(|l| l.built.is_some())
        });
        until(&mut app, "new plants", |app| {
            let w = app.world_mut();
            w.query_filtered::<(), (With<crate::vegetation::LazyPlants>, With<Mesh3d>)>()
                .iter(w)
                .next()
                .is_some()
        });
        until(&mut app, "new far terrain", |app| {
            count::<crate::far_terrain::FarTile>(app) > 0
        });
        let (drawn, known) = tiles(&mut app);
        assert_eq!(drawn, known, "every tile drawn is the streamer's");
        let far = app
            .world()
            .resource::<crate::far_terrain::FarTerrain>()
            .tile_count();
        assert_eq!(count::<crate::far_terrain::FarTile>(&mut app), far);
    }

    /// The late helpers on their own: an insert into a despawned entity, a
    /// spawn and a resource for an older scene are dropped; in the same
    /// scene they are applied.
    #[test]
    fn late_results_of_an_old_scene_are_dropped() {
        use crate::late::{LateCommands, SceneGeneration, despawn_if_stale, queue_if_current};
        #[derive(Component)]
        struct Built;
        #[derive(Resource)]
        struct Applied;
        fn build(mut commands: Commands, scene: Res<SceneGeneration>, target: Res<Target>) {
            let e = target.0;
            commands.entity(e).late_insert("test", Built);
            commands.entity(e).late("test", |mut ew| {
                ew.insert(Name::new("late"));
            });
            let spawned = commands.spawn(Built).id();
            despawn_if_stale(&mut commands, scene.0, "test", spawned);
            queue_if_current(&mut commands, scene.0, "test", |w| {
                w.insert_resource(Applied)
            });
        }
        #[derive(Resource)]
        struct Target(Entity);
        let mut world = World::new();
        world.init_resource::<SceneGeneration>();
        let e = world.spawn_empty().id();
        world.insert_resource(Target(e));
        let mut sys = IntoSystem::into_system(build);
        sys.initialize(&mut world);
        // A switch between the system and its commands.
        sys.run_without_applying_deferred((), &mut world).unwrap();
        world.despawn(e);
        world.resource_mut::<SceneGeneration>().0 += 1;
        sys.apply_deferred(&mut world);
        let mut built = world.query_filtered::<(), With<Built>>();
        assert_eq!(built.iter(&world).count(), 0);
        assert!(!world.contains_resource::<Applied>());
        // No switch: applied.
        let e = world.spawn_empty().id();
        world.insert_resource(Target(e));
        sys.run((), &mut world).unwrap();
        assert!(world.get::<Built>(e).is_some());
        assert!(world.get::<Name>(e).is_some());
        assert_eq!(built.iter(&world).count(), 2);
        assert!(world.contains_resource::<Applied>());
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
            trees += m.plants.trees();
            verts += m
                .plants
                .chunks
                .iter()
                .map(|c| c.near.build().positions.len() + c.far.positions.len())
                .sum::<usize>()
                + m.houses.mid.as_ref().map_or(0, |p| p.mesh.positions.len())
                + m.terrain.positions.len();
        }
        println!(
            "25 busiest tiles: {:.1} ms mean, {:.1} ms worst, {trees} trees, {verts} vertices",
            total / 25.0 * 1e3,
            worst * 1e3
        );
    }

    /// A phone's thinned trees (v5.42): every second about, always the
    /// same ones; 0 and 1 keep all.
    #[test]
    fn thinning_keeps_every_nth_tree_always_the_same() {
        let pts: Vec<Vec3> = (0..2000)
            .map(|i| {
                Vec3::new(
                    (i % 50) as f32 * 3.7 + 0.25,
                    0.0,
                    (i / 50) as f32 * 4.1 - 7.0,
                )
            })
            .collect();
        assert!(pts.iter().all(|&p| tree_kept(p, 0) && tree_kept(p, 1)));
        let half = pts.iter().filter(|&&p| tree_kept(p, 2)).count();
        assert!((800..1200).contains(&half), "{half} of 2000");
        let third = pts.iter().filter(|&&p| tree_kept(p, 3)).count();
        assert!((520..820).contains(&third), "{third} of 2000");
        assert!(pts.iter().all(|&p| tree_kept(p, 2) == tree_kept(p, 2)));
        assert_eq!(TileLook::default().plant_thinning, 0, "desktops: all trees");
    }

    /// Spec §14 Phase 8 T8 (the memory regression of v3.49): a tile
    /// waiting for its upload is not built again in every frame (the queue
    /// grew by all its tiles each frame, in the browser to gigabytes), at
    /// most [`MAX_PENDING_TILES`] are pending beyond the urgent ring, and
    /// stale tiles are rebuilt nearest first, the ones no longer wanted
    /// last.
    #[test]
    fn tile_builds_are_bounded_and_never_repeated() {
        let id = |x: i32, z: i32| (0usize, TileKey { x, z });
        let mut wanted: Vec<(i32, TileId)> = (-2i32..=2)
            .flat_map(|z| (-2i32..=2).map(move |x| (x.abs().max(z.abs()), id(x, z))))
            .collect();
        wanted.sort_by_key(|&(r, t)| (r, t));
        let none = HashSet::new();
        // The start: the urgent ring at once, nothing more while it is
        // pending.
        let pace = TileBuildPace::default();
        let first = tiles_to_build(&wanted, |_| false, &none, &none, pace);
        assert_eq!(first.len(), 9, "{first:?}");
        assert!(
            first
                .iter()
                .take(9)
                .all(|t| t.1.x.abs() <= 1 && t.1.z.abs() <= 1)
        );
        // The urgent ones were uploaded, three ring-2 tiles wait in the
        // queue: none of them is built again, nothing more is started.
        let loaded: HashSet<TileId> = first.iter().take(9).copied().collect();
        let queued: HashSet<TileId> = wanted
            .iter()
            .filter(|(r, _)| *r == 2)
            .take(MAX_PENDING_TILES)
            .map(|&(_, t)| t)
            .collect();
        let next = tiles_to_build(&wanted, |t| loaded.contains(&t), &none, &queued, pace);
        assert!(next.is_empty(), "{next:?}");
        // One uploaded: one more starts, never one of the queued.
        let mut queued = queued;
        let up = *queued.iter().next().unwrap();
        queued.remove(&up);
        let mut loaded = loaded;
        loaded.insert(up);
        let next = tiles_to_build(&wanted, |t| loaded.contains(&t), &none, &queued, pace);
        assert_eq!(next.len(), 1);
        assert!(!queued.contains(&next[0]) && !loaded.contains(&next[0]));
        // Stale (a new season): rebuilt nearest first, a few at a time, the
        // one no longer wanted last.
        let all: HashSet<TileId> = wanted.iter().map(|&(_, t)| t).collect();
        let mut stale = all.clone();
        stale.insert(id(3, 0));
        let next = tiles_to_build(
            &wanted,
            |t| all.contains(&t) || t == id(3, 0),
            &stale,
            &none,
            pace,
        );
        assert_eq!(next.len(), 9, "the urgent ring: {next:?}");
        assert_eq!(next[0], id(0, 0));
        assert!(!next.contains(&id(3, 0)));
        let fresh: HashSet<TileId> = all.clone();
        let only_far: HashSet<TileId> = [id(3, 0)].into();
        let next = tiles_to_build(
            &wanted,
            |t| fresh.contains(&t) || t == id(3, 0),
            &only_far,
            &none,
            pace,
        );
        assert_eq!(next, vec![id(3, 0)]);
        // A phone (the iPhone's memory, v5.42): the rider's tile at once,
        // then one at a time – never 9 tiles' meshes on the heap together.
        let phone = TileBuildPace::PHONE;
        let first = tiles_to_build(&wanted, |_| false, &none, &none, phone);
        assert_eq!(first, vec![id(0, 0)], "{first:?}");
        let pending: HashSet<TileId> = first.iter().copied().collect();
        let next = tiles_to_build(&wanted, |_| false, &none, &pending, phone);
        assert!(next.is_empty(), "one pending at a time: {next:?}");
        let loaded: HashSet<TileId> = first.iter().copied().collect();
        let next = tiles_to_build(&wanted, |t| loaded.contains(&t), &none, &none, phone);
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].1.x.abs().max(next[0].1.z.abs()), 1, "nearest first");
    }

    /// The plants' geometry per tile on a real package (not run by
    /// default; spec §14 Phase 8 T8, the memory budget): what a resident
    /// tile keeps (far meshes, near recipes), what all its near meshes
    /// would take if built at once (as before v3.54) and the largest near
    /// and detail chunk built.
    /// `OCW_PACKAGE=data/pilot/liechtenstein.ocwpkg cargo test --release
    /// -p ocw-render plant_geometry_report -- --ignored --nocapture`
    #[test]
    #[ignore = "needs a world package"]
    fn plant_geometry_report() {
        // The vertices of each species' near and far model.
        for sp in ocw_world::flora::Species::ALL {
            let at = crate::flora::PlantAt {
                base: Vec3::ZERO,
                scale: 1.0,
                turn: 0.3,
                shade: 0.5,
                hue: 0.5,
                pick: 0.5,
            };
            let look = crate::flora::PlantLook::SUMMER;
            let count = |d| {
                let mut m = MeshArrays::default();
                crate::flora::plant(&mut m, sp, &at, &look, d);
                m.positions.len()
            };
            println!(
                "{sp:?}: near {} far {} vertices",
                count(crate::flora::Detail::Near),
                count(crate::flora::Detail::Far)
            );
        }
        let path = std::env::var("OCW_PACKAGE").expect("OCW_PACKAGE");
        let p = WorldPackage::from_bytes(&std::fs::read(path).unwrap()).unwrap();
        let mut rows = Vec::new();
        let t = std::time::Instant::now();
        let limit: usize = std::env::var("OCW_TILES")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(usize::MAX);
        let mb = |b: usize| b as f64 / 1e6;
        for &key in p.tiles.keys().take(limit) {
            let Some(m) = build_tile_meshes(&p, key) else {
                continue;
            };
            let pl = &m.plants;
            let kept = crate::vegetation::plant_bytes(pl);
            let built = |cs: &[crate::vegetation::TreeChunk]| -> (usize, usize, usize) {
                cs.iter()
                    .map(|c| {
                        let a = c.near.build();
                        (a.positions.len(), a.bytes())
                    })
                    .fold((0, 0, 0), |s, x| (s.0 + x.0, s.1 + x.1, s.2.max(x.1)))
            };
            let (nv, nb, nmax) = built(&pl.chunks);
            let (dv, db, dmax) = built(&pl.details);
            let fv: usize = pl.chunks.iter().map(|c| c.far.positions.len()).sum();
            rows.push((kept, key, [nv, fv, dv], nb + db, [nmax, dmax], pl.trees()));
        }
        rows.sort_by_key(|r| std::cmp::Reverse(r.0));
        let total: usize = rows.iter().map(|r| r.0).sum();
        let all_built: usize = rows.iter().map(|r| r.0 + r.3).sum();
        println!(
            "{} tiles in {:.1} s: kept {:.1} MB in all ({:.2} MB mean, {:.1} MB worst); with every near mesh built {:.1} MB ({:.2} MB mean); largest chunk built: near {:.1} MB, detail {:.1} MB",
            rows.len(),
            t.elapsed().as_secs_f64(),
            mb(total),
            mb(total) / rows.len().max(1) as f64,
            mb(rows.first().map_or(0, |r| r.0)),
            mb(all_built),
            mb(all_built) / rows.len().max(1) as f64,
            mb(rows.iter().map(|r| r.4[0]).max().unwrap_or(0)),
            mb(rows.iter().map(|r| r.4[1]).max().unwrap_or(0)),
        );
        let mut by_built: Vec<_> = rows.iter().collect();
        by_built.sort_by_key(|r| std::cmp::Reverse(r.0 + r.3));
        for r in rows.iter().take(8).chain(by_built.into_iter().take(8)) {
            println!(
                "tile {:>4},{:>4}: kept {:6.1} MB, all built {:6.1} MB; verts near {:>8} far {:>8} detail {:>8}; largest chunk near {:5.1} MB detail {:5.1} MB; trees {:>6}",
                r.1.x,
                r.1.z,
                mb(r.0),
                mb(r.0 + r.3),
                r.2[0],
                r.2[1],
                r.2[2],
                mb(r.4[0]),
                mb(r.4[1]),
                r.5,
            );
        }
    }

    /// Spec §14 Phase 8 T3: the measured roof colour reaches the building
    /// mesh, the seasonal satellite map the far colour map (by the date),
    /// and a package without a look is drawn as before.
    #[test]
    fn the_measured_look_colours_roofs_and_the_far_ground() {
        use ocw_world::look::{
            RegionLook, RoofColour, RoofOrigin, Season, SeasonInfo, SeasonalTints, encode_tint,
        };
        let key = TileKey { x: 0, z: 0 };
        let plain = landuse_package();
        let mut p = landuse_package();
        let info = |season, centre_doy| SeasonInfo {
            season,
            centre_doy,
            first_doy: 1,
            last_doy: 2,
            scenes: 3,
        };
        p.look = RegionLook {
            roofs: Some(BTreeMap::from([(
                key,
                vec![Some(RoofColour {
                    rgb: [51, 102, 153],
                    origin: RoofOrigin::Orthophoto,
                })],
            )])),
            seasons: Some(SeasonalTints {
                cells: 2,
                source: 0,
                seasons: vec![info(Season::Summer, 200), info(Season::Winter, 15)],
                class_means: vec![vec![], vec![]],
                tiles: BTreeMap::from([(
                    key,
                    vec![
                        encode_tint(&[[40, 120, 40]; 4], 2),
                        encode_tint(&[[240, 240, 245]; 4], 2),
                    ],
                )]),
            }),
            ..Default::default()
        };
        // Roofs: the sampled colour on the roof faces.
        let m = build_tile_meshes(&p, key).unwrap();
        let roof = lin(0.2, 0.4, 0.6);
        let close = |a: &[f32; 4]| (0..3).all(|k| (a[k] - roof[k]).abs() < 1e-3);
        let colours = |m: &TileMeshes| m.houses.mid.as_ref().unwrap().mesh.colors.clone();
        assert!(colours(&m).iter().any(close));
        let before = build_tile_meshes(&plain, key).unwrap();
        assert!(!colours(&before).iter().any(close));
        // Without a date the far map is as before; in winter it turns white,
        // in summer green.
        let map_at = |day: Option<f64>| {
            let look = TileLook::default().with_region(&p.look, None, day);
            let m = build_tile_meshes_look(&p, key, None, &look).unwrap();
            let map = m.colour_map.unwrap();
            // A meadow texel (east, away from the pond).
            let i = (2 * 20 + 18) * 4;
            [map.data[i], map.data[i + 1], map.data[i + 2]]
        };
        let none = map_at(None);
        let old = before.colour_map.unwrap();
        let i = (2 * 20 + 18) * 4;
        assert_eq!(none, [old.data[i], old.data[i + 1], old.data[i + 2]]);
        let winter = map_at(Some(15.0));
        let summer = map_at(Some(200.0));
        assert!(winter[2] > 160 && winter[0] > 160, "{winter:?}");
        assert!(summer[1] > summer[0] && summer[0] < 120, "{summer:?}");
        // Blended half-way in between (autumn has no composite here).
        let between = map_at(Some(290.0));
        assert!(
            between[0] > summer[0] && between[0] < winter[0],
            "{between:?}"
        );
    }
}
