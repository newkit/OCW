//! A tile's plants (spec §4.6.1, §14 Phase 8 T8): the trees of the tree
//! grid, the mapped trees and the plantations' rows of section `VEGE`
//! (`ocw_world::plantings`), each with its species
//! (`ocw_world::flora::pick_species`: the measured leaf type or the
//! seasons' regional mix decides the leaf group, the regional tables the
//! species), drawn with the species models of [`crate::flora`] in chunks of
//! [`CHUNK_M`]: every chunk has a near mesh (detailed models, vines,
//! banana plants, the stands of maize and cereals) drawn up to
//! [`NEAR_FADE_M`] and a far mesh (simple models) drawn beyond it, the two
//! cross-faded (dithered) by their visibility ranges.
//!
//! The near meshes are only built where they can be seen (spec §14 Phase 8
//! T8, the memory fix): a tile keeps its chunks' near plants as recipes
//! ([`NearPlants`], 56 bytes a plant instead of some 5 kB of mesh),
//! and [`lazy_plants`] builds a chunk's near mesh on the task pool when a
//! camera comes within [`NEAR_BUILD_M`] (detail chunks: [`DETAIL_BUILD_M`])
//! and drops it again beyond [`NEAR_DROP_M`] ([`DETAIL_DROP_M`]). Built for
//! every resident tile, the forests' near models alone took up to 100 MB
//! a tile and 2.7 GB of wasm memory in the browser.

use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task, futures::check_ready};
use ocw_sky::season::{Crop, CropStage, LeafType, SeasonDay};
use ocw_world::flora::{FloraRegion, LeafGroup, Setting, Site, Species, pick_species, stature};
use ocw_world::landuse::LandClass;
use ocw_world::plantings;
use ocw_world::trees::hash;
use ocw_world::vegetation::FieldCrop;
use ocw_world::{TileKey, WorldPackage};

use crate::flora::{Detail, PlantAt};
use crate::package::{Keep, MeshArrays, TileGround, TreeSite};

/// Chunks per tile side (200 m chunks for 1 km tiles).
pub const CHUNKS: u32 = 5;
/// Detail chunks per tile side (100 m): the plants drawn only up close –
/// banana plants, vine rows – in their own smaller chunks with a shorter
/// range ([`DETAIL_FADE_M`]); beyond, the colour map shows the plantation.
pub const DETAIL_CHUNKS: u32 = 10;
/// The detail plants fade out between these distances from the camera to
/// a detail chunk's centre (m).
pub const DETAIL_FADE_M: std::ops::Range<f32> = 130.0..180.0;
/// The near models give way to the far ones between these distances
/// from the camera to a chunk's centre (m).
pub const NEAR_FADE_M: std::ops::Range<f32> = 230.0..310.0;

/// A chunk's near mesh is built when a camera comes within this distance
/// of its centre (m): [`NEAR_FADE_M`]'s end and a margin for the ride.
pub const NEAR_BUILD_M: f32 = NEAR_FADE_M.end + 120.0;
/// … and dropped again beyond this one (m).
pub const NEAR_DROP_M: f32 = NEAR_BUILD_M + 100.0;
/// The same for the detail chunks (bananas, vines; m).
pub const DETAIL_BUILD_M: f32 = DETAIL_FADE_M.end + 70.0;
pub const DETAIL_DROP_M: f32 = DETAIL_BUILD_M + 60.0;
/// Near meshes built ahead (within the build distance, not yet in view)
/// per frame; those a camera can already see are built at once.
pub const PREBUILD_PER_FRAME: usize = 2;
/// Near meshes being built at a time (beyond those already in view).
pub const MAX_LAZY_TASKS: usize = 4;
/// Stands lower than this (m: potatoes, beet, cereals) come down to the
/// ground at their edges instead of ending in a side.
pub const TAPER_BELOW_M: f32 = 1.0;
/// … to this height above the ground (m), clear of the terrain's surface.
pub const EDGE_LIFT_M: f32 = 0.04;

/// A plant's near model ([`crate::flora::plant`]'s arguments).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NearPlant {
    pub sp: Species,
    pub at: PlantAt,
    pub look: crate::flora::PlantLook,
}

/// A piece of a vine row ([`crate::flora::vine_row`]'s arguments).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VinePiece {
    pub a: Vec3,
    pub b: Vec3,
    pub height: f32,
    pub look: crate::flora::PlantLook,
    pub colour: [f32; 3],
}

/// A field stand's cell ([`crate::flora::stand_cell`]'s arguments).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StandCell {
    pub centre: Vec3,
    pub size: f32,
    /// The drawn ground at the four corners and the centre.
    pub ys: [f32; 5],
    pub tops: [f32; 5],
    pub shades: [f32; 5],
    pub open: [bool; 4],
    pub colour: [f32; 3],
}

/// A chunk's near plants as recipes (chunk-relative): the mesh is made by
/// [`Self::build`] when a camera comes close ([`lazy_plants`]).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NearPlants {
    pub plants: Vec<NearPlant>,
    pub vines: Vec<VinePiece>,
    pub stands: Vec<StandCell>,
}

impl NearPlants {
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The parts (plants, vine pieces, stand cells).
    pub fn len(&self) -> usize {
        self.plants.len() + self.vines.len() + self.stands.len()
    }

    /// The near mesh (its arrays no longer than needed).
    pub fn build(&self) -> MeshArrays {
        let mut m = MeshArrays::default();
        for p in &self.plants {
            crate::flora::plant(&mut m, p.sp, &p.at, &p.look, Detail::Near);
        }
        for v in &self.vines {
            crate::flora::vine_row(&mut m, v.a, v.b, v.height, &v.look, v.colour);
        }
        for c in &self.stands {
            crate::flora::stand_cell(
                &mut m, c.centre, c.size, c.ys, c.tops, c.shades, c.open, c.colour,
            );
        }
        m.shrink_to_fit();
        m
    }

    /// The bytes the recipes take.
    pub fn bytes(&self) -> usize {
        self.plants.capacity() * std::mem::size_of::<NearPlant>()
            + self.vines.capacity() * std::mem::size_of::<VinePiece>()
            + self.stands.capacity() * std::mem::size_of::<StandCell>()
    }

    pub fn shrink_to_fit(&mut self) {
        self.plants.shrink_to_fit();
        self.vines.shrink_to_fit();
        self.stands.shrink_to_fit();
    }

    /// The centre of the parts' feet (chunk-relative): about the centre of
    /// the built mesh's bounding box, from which the visibility range is
    /// measured (`use_aabb`).
    pub fn centre(&self) -> Vec3 {
        let feet = self
            .plants
            .iter()
            .map(|p| p.at.base)
            .chain(self.vines.iter().map(|v| (v.a + v.b) / 2.0))
            .chain(self.stands.iter().map(|c| c.centre));
        let (lo, hi) = feet.fold(
            (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
            |(lo, hi), q| (lo.min(q), hi.max(q)),
        );
        if lo.x.is_finite() {
            (lo + hi) / 2.0
        } else {
            Vec3::ZERO
        }
    }
}

/// The bytes of a tile's plants as built with the tile: the far meshes,
/// the near recipes and the crowns (the near meshes come and go with the
/// camera, [`lazy_plants`]).
pub fn plant_bytes(p: &TilePlants) -> usize {
    p.chunks
        .iter()
        .chain(p.details.iter())
        .map(|c| c.far.bytes())
        .sum::<usize>()
        + resident_plant_bytes(p)
}

/// The bytes of a tile's plants the CPU keeps while the tile is resident:
/// the near recipes and the crowns (the far meshes go to the GPU).
pub fn resident_plant_bytes(p: &TilePlants) -> usize {
    p.chunks
        .iter()
        .chain(p.details.iter())
        .map(|c| c.near.bytes())
        .sum::<usize>()
        + p.crowns.len() * 16
}

/// A chunk's near plants on its entity: [`lazy_plants`] gives it its mesh
/// when a camera comes within `build_m` of `centre` and takes it away
/// beyond `drop_m`.
#[derive(Component)]
pub struct LazyPlants {
    pub plants: std::sync::Arc<NearPlants>,
    /// [`NearPlants::centre`] (entity-relative).
    pub centre: Vec3,
    /// Visible up to here (its visibility range's end and a margin): built
    /// at once when nearer.
    pub show_m: f32,
    pub build_m: f32,
    pub drop_m: f32,
    pub material: Handle<crate::see_through::SeeThroughMaterial>,
}

/// What [`lazy_plants`] does with a chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LazyStep {
    /// Build the mesh now (in view, or about to be).
    Build,
    /// Build it ahead when the frame's budget allows.
    Prebuild,
    /// Drop the mesh (far away).
    Drop,
    Keep,
}

impl LazyPlants {
    /// A chunk's near plants (drawn up to [`NEAR_FADE_M`]).
    pub fn near(
        plants: NearPlants,
        material: Handle<crate::see_through::SeeThroughMaterial>,
    ) -> Self {
        Self {
            centre: plants.centre(),
            plants: std::sync::Arc::new(plants),
            show_m: NEAR_FADE_M.end + 20.0,
            build_m: NEAR_BUILD_M,
            drop_m: NEAR_DROP_M,
            material,
        }
    }

    /// A detail chunk's plants (drawn up to [`DETAIL_FADE_M`]).
    pub fn detail(
        plants: NearPlants,
        material: Handle<crate::see_through::SeeThroughMaterial>,
    ) -> Self {
        Self {
            centre: plants.centre(),
            plants: std::sync::Arc::new(plants),
            show_m: DETAIL_FADE_M.end + 20.0,
            build_m: DETAIL_BUILD_M,
            drop_m: DETAIL_DROP_M,
            material,
        }
    }

    /// The step for a camera `d` m from the centre, with or without the
    /// mesh `built`.
    pub fn step(&self, d: f32, built: bool) -> LazyStep {
        match built {
            true if d > self.drop_m => LazyStep::Drop,
            true => LazyStep::Keep,
            false if d <= self.show_m => LazyStep::Build,
            false if d <= self.build_m => LazyStep::Prebuild,
            false => LazyStep::Keep,
        }
    }
}

/// A chunk's near mesh being built on the task pool ([`lazy_plants`]).
#[derive(Component)]
pub struct LazyBuild(Task<MeshArrays>);

/// Builds the near meshes of the chunks near a camera – those in view at
/// once (in this frame), the others ahead on the task pool,
/// [`PREBUILD_PER_FRAME`] a frame (nearest first, at most
/// [`MAX_LAZY_TASKS`] at a time) – and drops the meshes (and builds) far
/// away ([`LazyPlants`]).
#[allow(clippy::type_complexity)]
pub fn lazy_plants(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    mut chunks: Query<(
        Entity,
        Ref<GlobalTransform>,
        &LazyPlants,
        Has<Mesh3d>,
        Option<&mut LazyBuild>,
    )>,
) {
    let eyes: Vec<Vec3> = cameras
        .iter()
        .filter(|(c, _)| c.is_active)
        .map(|(_, g)| g.translation())
        .collect();
    let mut ahead: Vec<(f32, Entity)> = Vec::new();
    let mut now = Vec::new();
    let mut busy = 0;
    for (e, g, lazy, built, task) in &mut chunks {
        // Spawned this frame: no global position yet.
        if g.is_added() {
            continue;
        }
        let at = g.transform_point(lazy.centre);
        let d = eyes
            .iter()
            .map(|p| p.distance(at))
            .fold(f32::INFINITY, f32::min);
        if let Some(mut task) = task {
            if d > lazy.drop_m {
                commands.entity(e).remove::<LazyBuild>();
            } else if let Some(m) = check_ready(&mut task.0) {
                commands.entity(e).remove::<LazyBuild>().insert((
                    Mesh3d(meshes.add(m.mesh())),
                    MeshMaterial3d(lazy.material.clone()),
                ));
            } else {
                busy += 1;
            }
            continue;
        }
        match lazy.step(d, built) {
            LazyStep::Build => now.push((e, lazy.plants.clone(), lazy.material.clone())),
            LazyStep::Prebuild => ahead.push((d, e)),
            LazyStep::Drop => {
                commands.entity(e).remove::<(
                    Mesh3d,
                    MeshMaterial3d<crate::see_through::SeeThroughMaterial>,
                )>();
            }
            LazyStep::Keep => {}
        }
    }
    // The chunks in view are built in this frame (a teleport, a new tile:
    // like the urgent tiles), in parallel on the desktop.
    #[cfg(not(target_arch = "wasm32"))]
    let built: Vec<MeshArrays> = {
        let tasks: Vec<Task<MeshArrays>> = now
            .iter()
            .map(|(_, plants, _)| {
                let plants = plants.clone();
                AsyncComputeTaskPool::get().spawn(async move { plants.build() })
            })
            .collect();
        tasks.into_iter().map(bevy::tasks::block_on).collect()
    };
    #[cfg(target_arch = "wasm32")]
    let built: Vec<MeshArrays> = now.iter().map(|(_, plants, _)| plants.build()).collect();
    for ((e, _, material), m) in now.into_iter().zip(built) {
        commands
            .entity(e)
            .insert((Mesh3d(meshes.add(m.mesh())), MeshMaterial3d(material)));
    }
    ahead.sort_by(|a, b| a.0.total_cmp(&b.0));
    let room = MAX_LAZY_TASKS.saturating_sub(busy).min(PREBUILD_PER_FRAME);
    for &(_, e) in ahead.iter().take(room) {
        if let Ok((_, _, lazy, _, _)) = chunks.get(e) {
            start_lazy(&mut commands, e, lazy);
        }
    }
}

fn start_lazy(commands: &mut Commands, e: Entity, lazy: &LazyPlants) {
    let plants = lazy.plants.clone();
    let task = AsyncComputeTaskPool::get().spawn(async move { plants.build() });
    commands.entity(e).insert(LazyBuild(task));
}

/// The plants of one chunk of a tile; positions relative to `origin`.
#[derive(Default)]
pub struct TreeChunk {
    pub origin: [f32; 3],
    /// The near plants (their mesh built when a camera comes close).
    pub near: NearPlants,
    /// The far models (built with the tile).
    pub far: MeshArrays,
    /// Trees (grid, mapped and orchard trees) in the chunk.
    pub trees: usize,
}

/// A tile's plants: its chunks (empty ones left out) and the crowns (x,
/// z, radius, top) for the flight's obstacles.
#[derive(Default)]
pub struct TilePlants {
    pub chunks: Vec<TreeChunk>,
    /// The detail chunks (only their `near` mesh; [`DETAIL_CHUNKS`]).
    pub details: Vec<TreeChunk>,
    pub crowns: Vec<[f32; 4]>,
}

impl TilePlants {
    pub fn trees(&self) -> usize {
        self.chunks.iter().map(|c| c.trees).sum()
    }
}

/// The leaf group of a tree from the seasons' leaf type.
fn group(leaf: LeafType) -> LeafGroup {
    match leaf {
        LeafType::Evergreen => LeafGroup::Evergreen,
        LeafType::Deciduous => LeafGroup::Deciduous,
        LeafType::Larch => LeafGroup::Larch,
    }
}

/// Whether water or wetland lies within 15 m of (x, z) (alders and
/// willows on the banks).
fn near_water(pkg: &WorldPackage, x: f64, z: f64) -> bool {
    let Some(lu) = pkg.landuse.as_ref() else {
        return false;
    };
    let size = f64::from(pkg.tile_size_m);
    [
        (0.0, 0.0),
        (15.0, 0.0),
        (-15.0, 0.0),
        (0.0, 15.0),
        (0.0, -15.0),
    ]
    .iter()
    .any(|(dx, dz)| {
        matches!(
            lu.class_at(size, x + dx, z + dz),
            Some(LandClass::Water | LandClass::Wetland)
        )
    })
}

/// The stand of a field crop on the day: its height (m; 0: nothing
/// standing) and colour (linear).
pub fn stand(crop: FieldCrop, day: Option<&SeasonDay>, alt_m: f64) -> (f32, [f32; 3]) {
    let (cal, tall) = match crop {
        FieldCrop::Maize => (Crop::Maize, 2.5),
        FieldCrop::Sunflower => (Crop::Maize, 1.8),
        FieldCrop::Potato | FieldCrop::Beet => (Crop::Maize, 0.5),
        FieldCrop::Cereal => (Crop::Cereal, 0.9),
        FieldCrop::Rapeseed => (Crop::Cereal, 1.3),
        _ => return (0.0, [0.0; 3]),
    };
    let (stage, gdd, cdd) = match day {
        Some(d) => {
            let s = d.state(alt_m);
            (d.crop(cal, alt_m), s.gdd, s.cdd)
        }
        None => (CropStage::Green, 900.0, 0.0),
    };
    let h = match stage {
        CropStage::Sprouting => 0.35,
        CropStage::Green => 0.85,
        CropStage::Ripening | CropStage::Ripe => 1.0,
        _ => 0.0,
    } * tall;
    let mut c = crate::season::crop_target(cal, gdd, cdd);
    if crop == FieldCrop::Rapeseed {
        // Yellow in flower (late April, May).
        let t = ((gdd - 300.0) / 80.0).clamp(0.0, 1.0) * ((620.0 - gdd) / 80.0).clamp(0.0, 1.0);
        let yellow = Color::srgb(0.90, 0.80, 0.12).to_linear();
        c = std::array::from_fn(|i| c[i] + ([yellow.red, yellow.green, yellow.blue][i] - c[i]) * t);
    }
    if crop == FieldCrop::Sunflower && stage == CropStage::Green && gdd > 1100.0 {
        let y = Color::srgb(0.85, 0.65, 0.10).to_linear();
        c = [y.red, y.green, y.blue];
    }
    (if h < 0.3 { 0.0 } else { h }, c)
}

/// Builds a tile's plants: the grid's `classed` trees (already filtered by
/// `keep`), the mapped trees, the plantations' rows and the fields'
/// stands, on the `season` day; the vine rows and the stands on the
/// `ground` as the tile draws it.
pub fn tile_plants(
    pkg: &WorldPackage,
    key: TileKey,
    classed: &[(TreeSite, Option<LandClass>)],
    keep: Keep,
    season: Option<&SeasonDay>,
    ground: &TileGround,
) -> TilePlants {
    let size = pkg.tile_size_m;
    let o = key.origin(f64::from(size));
    let region = FloraRegion::at(pkg.origin.0, pkg.origin.1);
    let mix = ocw_sky::season::Climate::for_place(pkg.origin.0, pkg.origin.1).trees;
    let proj = ocw_world::geo::LocalProjection::new(pkg.origin.0, pkg.origin.1);
    let (lat, lon) = proj.unproject([o[0] + f64::from(size) / 2.0, o[1] + f64::from(size) / 2.0]);
    let windward = ocw_world::flora::windward(lat, lon);
    let grid = |per: u32| -> Vec<TreeChunk> {
        let cs = size / per as f32;
        let n = per as usize;
        (0..n * n)
            .map(|i| {
                let (c, r) = (i % n, i / n);
                let ox = o[0] as f32 + (c as f32 + 0.5) * cs;
                let oz = o[1] as f32 + (r as f32 + 0.5) * cs;
                let oy = pkg.height(f64::from(ox), f64::from(oz)).unwrap_or(0.0) as f32;
                TreeChunk {
                    origin: [ox, oy, oz],
                    ..Default::default()
                }
            })
            .collect()
    };
    let cell_of = |per: u32, x: f32, z: f32| -> usize {
        let cs = size / per as f32;
        let n = per as usize;
        let c = (((x - o[0] as f32) / cs).floor().max(0.0) as usize).min(n - 1);
        let r = (((z - o[1] as f32) / cs).floor().max(0.0) as usize).min(n - 1);
        r * n + c
    };
    let mut chunks = grid(CHUNKS);
    let mut details = grid(DETAIL_CHUNKS);
    let chunk_of = |x: f32, z: f32| cell_of(CHUNKS, x, z);
    let detail_of = |x: f32, z: f32| cell_of(DETAIL_CHUNKS, x, z);
    let kept = |x: f32, z: f32| keep.is_none_or(|f| f([f64::from(x), f64::from(z)]));
    let mut crowns = Vec::new();
    let site_of = |class: Option<LandClass>, x: f32, z: f32, alt: f64| Site {
        region,
        setting: Setting::of_class(class),
        alt_m: alt,
        measured: ocw_world::trees::measured_leaf(pkg, f64::from(x), f64::from(z)),
        windward,
        near_water: near_water(pkg, f64::from(x), f64::from(z)),
    };
    let leaf_of = |class: Option<LandClass>, x: f32, z: f32, alt: f64, bits: (i64, i64)| {
        let setting = crate::season::tree_setting(class);
        let measured = crate::season::measured(ocw_world::trees::measured_leaf(
            pkg,
            f64::from(x),
            f64::from(z),
        ));
        let mix = season.map_or(&mix, |d| &d.climate.trees);
        mix.leaf_type(setting, alt, measured, hash(bits.0, bits.1, 13))
    };
    let put = |chunks: &mut Vec<TreeChunk>,
               crowns: &mut Vec<[f32; 4]>,
               sp: Species,
               base: Vec3,
               scale: f32,
               bits: (i64, i64),
               near_only: bool| {
        // Plants drawn only up close go to the detail chunks.
        let ch = if near_only {
            &mut chunks[detail_of(base.x, base.z)]
        } else {
            &mut chunks[chunk_of(base.x, base.z)]
        };
        let origin = Vec3::from(ch.origin);
        let alt = f64::from(base.y);
        let jitter = hash(bits.0, bits.1, 14) * 2.0 - 1.0;
        let look = crate::flora::plant_look(season, sp, alt, jitter);
        let at = PlantAt {
            base: base - origin,
            scale,
            turn: hash(bits.0, bits.1, 16) * std::f32::consts::TAU,
            shade: hash(bits.0, bits.1, 11),
            hue: hash(bits.0, bits.1, 15),
            pick: hash(bits.0, bits.1, 18),
        };
        ch.near.plants.push(NearPlant { sp, at, look });
        if !near_only {
            crate::flora::plant(&mut ch.far, sp, &at, &look, Detail::Far);
        }
        ch.trees += 1;
        if sp.solid() {
            let (top, r) = crate::flora::crown_top(sp, scale);
            crowns.push([base.x, base.z, r, base.y + top]);
        }
    };
    // The grid's trees.
    for &((base, s), class) in classed {
        let bits = (i64::from(base.x.to_bits()), i64::from(base.z.to_bits()));
        let alt = f64::from(base.y);
        let leaf = leaf_of(class, base.x, base.z, alt, bits);
        let site = site_of(class, base.x, base.z, alt);
        let sp = pick_species(&site, group(leaf), hash(bits.0, bits.1, 19));
        // The grid's scale by land class (0.35 … 1.5) around its class's
        // mean: the species' own size varies by ±30 %.
        let (_, s0, s1) = ocw_world::trees::tree_density(class.unwrap_or_default());
        let scale = s / ((s0 + s1) / 2.0).max(0.1) * stature(region, sp, alt);
        put(&mut chunks, &mut crowns, sp, base, scale, bits, false);
    }
    // The mapped trees.
    for (foot, t) in plantings::mapped_trees(pkg, key) {
        if !kept(foot[0], foot[2]) {
            continue;
        }
        let base = Vec3::from(foot);
        let bits = (i64::from(base.x.to_bits()), i64::from(base.z.to_bits()));
        let alt = f64::from(base.y);
        let sp = if t.species == Species::Unknown {
            let class = pkg
                .landuse
                .as_ref()
                .and_then(|l| l.class_at(f64::from(size), f64::from(base.x), f64::from(base.z)));
            // Mapped trees stand in towns and along roads: the town mix.
            let class = class.or(Some(LandClass::Residential));
            let leaf = leaf_of(class, base.x, base.z, alt, bits);
            pick_species(
                &site_of(class, base.x, base.z, alt),
                group(leaf),
                hash(bits.0, bits.1, 19),
            )
        } else {
            t.species
        };
        let scale = if t.height_m > 0.0 {
            (t.height_m / sp.height_m()).clamp(0.3, 2.0)
        } else {
            0.75 + 0.35 * hash(bits.0, bits.1, 4)
        };
        put(&mut chunks, &mut crowns, sp, base, scale, bits, false);
    }
    // The plantations' rows.
    let (runs, plants) = plantings::tile_rows(pkg, key);
    for p in plants {
        if !kept(p.pos[0], p.pos[2]) {
            continue;
        }
        let base = Vec3::from(p.pos);
        let bits = (i64::from(base.x.to_bits()), i64::from(base.z.to_bits()));
        let near_only = p.species == Species::Banana;
        put(
            if near_only { &mut details } else { &mut chunks },
            &mut crowns,
            p.species,
            base,
            p.scale * 0.8,
            bits,
            near_only,
        );
    }
    let red_vines = matches!(region, FloraRegion::AlpineRhine);
    for r in runs {
        // Pieces of at most 10 m following the ground.
        let (a, b) = (Vec2::from(r.a), Vec2::from(r.b));
        let len = a.distance(b);
        let pieces = (len / 10.0).ceil().max(1.0) as u32;
        for k in 0..pieces {
            let p0 = a.lerp(b, k as f32 / pieces as f32);
            let p1 = a.lerp(b, (k + 1) as f32 / pieces as f32);
            let mid = (p0 + p1) / 2.0;
            if !kept(mid.x, mid.y) {
                continue;
            }
            let y = |p: Vec2| ground.height(f64::from(p.x), f64::from(p.y)).unwrap_or(0.0) as f32;
            let ch = &mut details[detail_of(mid.x, mid.y)];
            let origin = Vec3::from(ch.origin);
            let (q0, q1) = (
                Vec3::new(p0.x, y(p0), p0.y) - origin,
                Vec3::new(p1.x, y(p1), p1.y) - origin,
            );
            let bits = ((mid.x * 4.0) as i64, (mid.y * 4.0) as i64);
            let jitter = hash(bits.0, bits.1, 14) * 2.0 - 1.0;
            let look = crate::flora::vine_look(season, f64::from(y(mid)), jitter);
            let red = red_vines && hash(bits.0 / 400, bits.1 / 400, 44) < 0.6;
            let colour = crate::flora::vine_colour(&look, red, hash(bits.0, bits.1, 11));
            let height = if region == FloraRegion::Tenerife {
                1.1
            } else {
                1.6
            };
            ch.near.vines.push(VinePiece {
                a: q0,
                b: q1,
                height,
                look,
                colour,
            });
        }
    }
    // The fields' stands.
    stands(pkg, key, &mut chunks, &chunk_of, keep, season, ground);
    chunks.retain(|c| !c.near.is_empty() || !c.far.indices.is_empty());
    details.retain(|c| !c.near.is_empty());
    for c in chunks.iter_mut().chain(details.iter_mut()) {
        c.near.shrink_to_fit();
        c.far.shrink_to_fit();
    }
    TilePlants {
        chunks,
        details,
        crowns,
    }
}

/// The stands of maize, cereals, sunflowers, rapeseed and potatoes: a
/// canopy per cell of section `VEGE` (near chunks only) on the drawn
/// `ground` at its corners and its centre; where the neighbour has not the
/// same stand, a tall stand ends in a side, a low one comes down to the
/// ground ([`TAPER_BELOW_M`]).
fn stands(
    pkg: &WorldPackage,
    key: TileKey,
    chunks: &mut [TreeChunk],
    chunk_of: &dyn Fn(f32, f32) -> usize,
    keep: Keep,
    season: Option<&SeasonDay>,
    ground: &TileGround,
) {
    let Some(veg) = pkg.look.vegetation.as_ref() else {
        return;
    };
    if !veg.tiles.get(&key).is_some_and(|v| !v.crops.is_empty()) {
        return;
    }
    let size = f64::from(pkg.tile_size_m);
    let n = veg.cells as usize;
    let cs = size / n as f64;
    let ni = n as i64;
    // The stand per crop at the cell's altitude band (100 m steps).
    let mut cache: std::collections::HashMap<(u8, i64), (f32, [f32; 3])> = Default::default();
    // Whether a stand is drawn in global cell (gc, gr) – its crop code, its
    // height and colour – also in the neighbouring tiles: a stand's edge is
    // where its neighbour has none, not only where the neighbour's crop
    // differs (before 2026-11-17 the cells kept free along a road counted
    // as the same field, and the stand's edge there had no side: its top
    // hung in the air over the ground).
    type Drawn = Option<(u8, f32, [f32; 3])>;
    let mut drawn: std::collections::HashMap<(i64, i64), Drawn> = Default::default();
    let mut present = |gc: i64, gr: i64| -> Drawn {
        if let Some(d) = drawn.get(&(gc, gr)) {
            return *d;
        }
        let tk = TileKey {
            x: gc.div_euclid(ni) as i32,
            z: gr.div_euclid(ni) as i32,
        };
        let d = (|| {
            let t = veg.tiles.get(&tk).filter(|t| !t.crops.is_empty())?;
            let code = t.crops[(gr.rem_euclid(ni) * ni + gc.rem_euclid(ni)) as usize];
            let crop = FieldCrop::from_code(code).filter(|c| {
                matches!(
                    c,
                    FieldCrop::Maize
                        | FieldCrop::Sunflower
                        | FieldCrop::Cereal
                        | FieldCrop::Rapeseed
                        | FieldCrop::Potato
                        | FieldCrop::Beet
                )
            })?;
            let (x, z) = ((gc as f64 + 0.5) * cs, (gr as f64 + 0.5) * cs);
            if keep.is_some_and(|f| !f([x, z])) || plantings_blocked(pkg, x, z) {
                return None;
            }
            let band = (ground.height(x, z).unwrap_or(0.0) / 100.0).round() as i64;
            let (height, colour) = *cache
                .entry((code, band))
                .or_insert_with(|| stand(crop, season, band as f64 * 100.0));
            (height > 0.0).then_some((code, height, colour))
        })();
        drawn.insert((gc, gr), d);
        d
    };
    for r in 0..ni {
        for c in 0..ni {
            // Global cell indices: neighbouring cells share their corners'
            // heights and shades, so the canopy is one bumpy surface.
            let (gc, gr) = (c + i64::from(key.x) * ni, r + i64::from(key.z) * ni);
            let Some((code, height, colour)) = present(gc, gr) else {
                continue;
            };
            let (x, z) = ((gc as f64 + 0.5) * cs, (gr as f64 + 0.5) * cs);
            let h = |dx: f64, dz: f64| ground.height(x + dx, z + dz).unwrap_or(0.0) as f32;
            let hc = h(0.0, 0.0);
            let hs = cs / 2.0;
            let ys = [h(-hs, -hs), h(hs, -hs), h(-hs, hs), h(hs, hs), hc];
            let mut same = |dc: i64, dr: i64| {
                present(gc + dc, gr + dr).is_some_and(|(other, ..)| other == code)
            };
            let open = [!same(-1, 0), !same(1, 0), !same(0, -1), !same(0, 1)];
            // A low stand (potatoes, beet, cereals) comes down to the ground
            // at its edge instead of ending in a side: at every corner
            // shared with a cell without the same stand its top lies just
            // above the ground, so the field swells gently out of the ground
            // (a 0.4 m box with sides read as a slab floating over it).
            let low = height < TAPER_BELOW_M;
            let inside = |dc: i64, dr: i64, same: &mut dyn FnMut(i64, i64) -> bool| {
                !low || [(dc - 1, dr - 1), (dc, dr - 1), (dc - 1, dr), (dc, dr)]
                    .iter()
                    .all(|&(a, b)| (a, b) == (0, 0) || same(a, b))
            };
            let corner = |dc: i64, dr: i64, seed: u64| hash(gc + dc, gr + dr, seed);
            let bump = |v: f32| height * (0.86 + 0.24 * v);
            let mut top_at = |dc: i64, dr: i64| {
                if inside(dc, dr, &mut same) {
                    bump(corner(dc, dr, 45))
                } else {
                    EDGE_LIFT_M
                }
            };
            let tops = [
                top_at(0, 0),
                top_at(1, 0),
                top_at(0, 1),
                top_at(1, 1),
                height * (0.95 + 0.2 * hash(gc, gr, 47)),
            ];
            let open = if low { [false; 4] } else { open };
            let shade = |v: f32| 0.84 + 0.3 * v;
            let shades = [
                shade(corner(0, 0, 46)),
                shade(corner(1, 0, 46)),
                shade(corner(0, 1, 46)),
                shade(corner(1, 1, 46)),
                shade(hash(gc, gr, 48)),
            ];
            let i = chunk_of(x as f32, z as f32);
            let origin = Vec3::from(chunks[i].origin);
            let centre = Vec3::new(x as f32, hc, z as f32) - origin;
            let ys = ys.map(|y| y - origin.y);
            chunks[i].near.stands.push(StandCell {
                centre,
                size: cs as f32,
                ys,
                tops,
                shades,
                open,
                colour,
            });
        }
    }
}

/// Whether a field cell's centre is on a road (the stands leave the road
/// and its verge free).
fn plantings_blocked(pkg: &WorldPackage, x: f64, z: f64) -> bool {
    let key = TileKey::containing([x, z], f64::from(pkg.tile_size_m));
    pkg.tiles.get(&key).is_some_and(|t| {
        t.roads.iter().any(|r| {
            let reach = f64::from(r.width_m) / 2.0 + 4.0;
            r.points.windows(2).any(|w| {
                ocw_world::obstacles::segment_distance(
                    [x, z],
                    [f64::from(w[0][0]), f64::from(w[0][2])],
                    [f64::from(w[1][0]), f64::from(w[1][2])],
                ) < reach
            })
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stands_follow_the_crop_calendar() {
        let m = ocw_sky::season::SeasonModel::for_place(47.99, 7.85);
        // July: maize tall and green, cereals ripe; late August: cereal
        // stubble (nothing standing), maize still there; March: no maize.
        let jul = m.day(195.0);
        let (mh, mc) = stand(FieldCrop::Maize, Some(&jul), 200.0);
        assert!(mh > 1.8 && mc[1] > mc[0], "{mh} {mc:?}");
        let (ch, cc) = stand(FieldCrop::Cereal, Some(&jul), 200.0);
        assert!(ch > 0.6 && cc[0] > cc[2], "{ch} {cc:?}");
        let aug = m.day(240.0);
        assert_eq!(stand(FieldCrop::Cereal, Some(&aug), 200.0).0, 0.0);
        assert!(stand(FieldCrop::Maize, Some(&aug), 200.0).0 > 1.8);
        let mar = m.day(75.0);
        assert_eq!(stand(FieldCrop::Maize, Some(&mar), 200.0).0, 0.0);
        // Rapeseed in flower in early May: yellow.
        let may = m.day(125.0);
        let (rh, rc) = stand(FieldCrop::Rapeseed, Some(&may), 200.0);
        assert!(rh > 0.5 && rc[0] > rc[2] * 3.0, "{rh} {rc:?}");
        // Grass and vineyards have no stand.
        assert_eq!(stand(FieldCrop::Grass, Some(&jul), 200.0).0, 0.0);
        assert_eq!(stand(FieldCrop::Vineyard, Some(&jul), 200.0).0, 0.0);
        // No season: summer.
        assert!(stand(FieldCrop::Maize, None, 200.0).0 > 1.8);
    }

    // --- The memory budget (spec §14 Phase 8 T8, the fix of v3.54) -----------

    use ocw_world::landuse::{LandUse, LandUseTile};
    use ocw_world::vegetation::{NO_ROWS, Vegetation, VegetationTile, encode_direction};
    use ocw_world::{Tile, WorldPackage};
    use std::collections::BTreeMap;

    use crate::package::MeshArrays;

    /// A gently rising 1 km tile at (`lat`, `lon`), all of `class`, its
    /// fields all `crop` (rows at 20°; `None`: no section `VEGE`).
    fn worst_tile(lat: f64, lon: f64, class: LandClass, crop: Option<FieldCrop>) -> WorldPackage {
        let key = TileKey { x: 0, z: 0 };
        let n = 21u32;
        let mut pkg = WorldPackage {
            name: "Budget".into(),
            origin: (lat, lon),
            tile_size_m: 1000.0,
            samples: n,
            tiles: BTreeMap::from([(
                key,
                Tile {
                    key,
                    heights: (0..n * n).map(|i| 300.0 + (i % n) as f32).collect(),
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
            obstacles: None,
        };
        let cells = 100usize;
        pkg.landuse = Some(LandUse {
            cells: cells as u32,
            tiles: BTreeMap::from([(
                key,
                LandUseTile {
                    classes: vec![class; cells * cells],
                    water: Vec::new(),
                },
            )]),
        });
        if let Some(crop) = crop {
            let rows = if plantings::row_spec(crop, FloraRegion::Other).is_some() {
                encode_direction(20f32.to_radians())
            } else {
                NO_ROWS
            };
            pkg.look.vegetation = Some(Vegetation {
                cells: cells as u32,
                sources: Vec::new(),
                tiles: BTreeMap::from([(
                    key,
                    VegetationTile {
                        crops: vec![crop.code(); cells * cells],
                        rows: vec![rows; cells * cells],
                        trees: Vec::new(),
                    },
                )]),
            });
        }
        pkg
    }

    /// What a tile's plants take (MB): built with the tile, kept on the CPU
    /// while resident, the largest near and detail chunk built; and the
    /// most vertices in one chunk.
    fn budget_of(pkg: &WorldPackage) -> (f64, f64, f64, f64, usize) {
        let m = crate::package::build_tile_meshes(pkg, TileKey { x: 0, z: 0 }).unwrap();
        let mb = |b: usize| b as f64 / 1e6;
        let largest = |cs: &[TreeChunk]| {
            cs.iter()
                .map(|c| c.near.build())
                .map(|a| (a.bytes(), a.positions.len()))
                .max()
                .unwrap_or_default()
        };
        let (near, near_v) = largest(&m.plants.chunks);
        let (detail, detail_v) = largest(&m.plants.details);
        (
            mb(plant_bytes(&m.plants)),
            mb(resident_plant_bytes(&m.plants)),
            mb(near),
            mb(detail),
            near_v.max(detail_v),
        )
    }

    /// Chunks of `size` m whose centres can lie within `r` m of a camera.
    fn chunks_within(r: f32, size: f32) -> f32 {
        let reach = r + size * std::f32::consts::FRAC_1_SQRT_2;
        std::f32::consts::PI * reach * reach / (size * size)
    }

    /// Spec §14 Phase 8 T8 (the memory regression of v3.49): the densest
    /// tiles – a closed forest, a vineyard, a maize field (Vaduz), a banana
    /// plantation and an orchard (Tenerife), each over the whole square
    /// kilometre, denser than any real tile of the Liechtenstein or
    /// Tenerife packages – keep a few MB while resident (far meshes, near
    /// recipes), their largest built chunk stays bounded, and a ride with
    /// the browser's 25 resident tiles of the worst kind and every chunk
    /// within the drop distances built stays well under 1 GB. Before the
    /// fix every near mesh was built with the tile: a Liechtenstein forest
    /// tile took 108 MB, a Tenerife banana tile 624 MB.
    #[test]
    fn the_densest_tiles_keep_within_the_memory_budget() {
        let cases = [
            ("forest", worst_tile(47.14, 9.52, LandClass::Forest, None)),
            (
                "vineyard",
                worst_tile(47.14, 9.52, LandClass::Vineyard, Some(FieldCrop::Vineyard)),
            ),
            (
                "maize",
                worst_tile(47.14, 9.52, LandClass::Farmland, Some(FieldCrop::Maize)),
            ),
            (
                "banana",
                worst_tile(28.39, -16.52, LandClass::Farmland, Some(FieldCrop::Banana)),
            ),
            (
                "orchard",
                worst_tile(28.39, -16.52, LandClass::Orchard, Some(FieldCrop::Orchard)),
            ),
        ];
        for (name, pkg) in cases {
            let (built, kept, near, detail, verts) = budget_of(&pkg);
            let size = pkg.tile_size_m;
            let (near_cs, detail_cs) = (size / CHUNKS as f32, size / DETAIL_CHUNKS as f32);
            // The browser's wasm memory on a ride through 25 such tiles:
            // what the resident tiles keep, the tiles pending upload, and
            // after a jump every chunk in view built in one frame.
            let wasm = 25.0 * kept
                + crate::package::MAX_PENDING_TILES as f64 * built
                + f64::from(chunks_within(NEAR_FADE_M.end + 20.0, near_cs)) * near
                + f64::from(chunks_within(DETAIL_FADE_M.end + 20.0, detail_cs)) * detail;
            // All the plants' meshes at once (GPU and CPU): 25 tiles, the
            // chunks within the drop distances built.
            let all = 25.0 * built
                + f64::from(chunks_within(NEAR_DROP_M, near_cs)) * near
                + f64::from(chunks_within(DETAIL_DROP_M, detail_cs)) * detail;
            println!(
                "{name}: built {built:.1} MB, kept {kept:.1} MB, largest chunk near {near:.1} MB, detail {detail:.1} MB ({verts} vertices); ride: wasm {wasm:.0} MB, all {all:.0} MB"
            );
            assert!(built < 60.0, "{name}: {built:.1} MB built per tile");
            assert!(kept < 10.0, "{name}: {kept:.1} MB kept per tile");
            assert!(near < 8.0, "{name}: near chunk {near:.1} MB");
            assert!(detail < 10.0, "{name}: detail chunk {detail:.1} MB");
            assert!(verts < 200_000, "{name}: {verts} vertices in a chunk");
            assert!(wasm < 500.0, "{name}: wasm memory {wasm:.0} MB");
            assert!(all < 1_700.0, "{name}: all meshes {all:.0} MB");
        }
    }

    // --- Stands on the drawn ground (the floating slabs of 2026-11-17) ------

    /// A potato tile on Tenerife on a 10 m terrain grid (the packages'
    /// spacing, the stands' cells one terrain cell each), the ground
    /// `height(i, k)` at the grid's vertices.
    fn field_tile(height: impl Fn(u32, u32) -> f32) -> WorldPackage {
        let mut pkg = worst_tile(28.39, -16.52, LandClass::Farmland, Some(FieldCrop::Potato));
        let n = 101u32;
        pkg.samples = n;
        let t = pkg.tiles.get_mut(&TileKey { x: 0, z: 0 }).unwrap();
        t.heights = (0..n * n).map(|v| height(v % n, v / n)).collect();
        pkg
    }

    /// How high the stands' tops stand over the drawn ground, as shares of
    /// the stand's height: sampled over every top triangle of every stand
    /// cell of the tile (the drawn ground: its terrain mesh, after the
    /// renderer lowered it under the junctions). The stand's top is 0.86 …
    /// 1.15 of its height above the ground at its five vertices, a low
    /// stand's [`EDGE_LIFT_M`] at its edge.
    fn stand_heights_over_the_ground(pkg: &WorldPackage) -> (Vec<f32>, usize) {
        let key = TileKey { x: 0, z: 0 };
        let m = crate::package::build_tile_meshes(pkg, key).unwrap();
        let ground = TileGround::new(pkg, key, Some(&m.terrain));
        let (height, _) = stand(FieldCrop::Potato, None, 0.0);
        let mut shares = Vec::new();
        let mut cells = 0;
        for ch in &m.plants.chunks {
            let o = Vec3::from(ch.origin);
            for c in &ch.near.stands {
                cells += 1;
                let mut s = MeshArrays::default();
                crate::flora::stand_cell(
                    &mut s, c.centre, c.size, c.ys, c.tops, c.shades, c.open, c.colour,
                );
                // The top: the first four triangles.
                for t in s.indices[..12].chunks(3) {
                    let p: [Vec3; 3] =
                        std::array::from_fn(|k| Vec3::from(s.positions[t[k] as usize]) + o);
                    for (a, b) in [(0.0, 0.0), (0.3, 0.3), (0.1, 0.8), (0.8, 0.1), (0.45, 0.1)] {
                        let q = p[0] + (p[1] - p[0]) * a + (p[2] - p[0]) * b;
                        let g = ground.height(f64::from(q.x), f64::from(q.z)).unwrap() as f32;
                        shares.push((q.y - g) / height);
                    }
                }
            }
        }
        (shares, cells)
    }

    /// Regression (2026-11-17, Tenerife: green slabs floating over the
    /// terraces of the Masca road and above La Orotava): a stand's top
    /// follows the ground as the tile draws it, at every point of the cell,
    /// also where the cell's corners are twisted (terrace edges: one corner
    /// metres above the others). Before, the centre of each cell stood at
    /// its corners' mean (the bilinear ground), which on a terrace edge is
    /// metres above the drawn terrain's diagonal.
    #[test]
    fn stands_lie_on_the_drawn_ground_on_terrace_edges() {
        // Terraces 4 m high every 30 m, every third column of vertices a
        // step up: most cells are twisted.
        let terraces = |i: u32, k: u32| 300.0 + 4.0 * ((i * 7 + k * 13) % 5) as f32;
        let pkg = field_tile(terraces);
        // The bilinear ground differs from the drawn one by metres here.
        let key = TileKey { x: 0, z: 0 };
        let drawn = TileGround::new(&pkg, key, None);
        let twist = (0..99)
            .map(|c| {
                let (x, z) = (c as f64 * 10.0 + 5.0, 505.0);
                (pkg.height(x, z).unwrap() - drawn.height(x, z).unwrap()).abs()
            })
            .fold(0.0, f64::max);
        assert!(twist > 2.0, "twist {twist}");
        let (shares, cells) = stand_heights_over_the_ground(&pkg);
        assert!(cells > 9_000, "{cells} stand cells");
        let (lo, hi) = shares
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), s| (lo.min(*s), hi.max(*s)));
        // Never under the ground, never above its own height (only the
        // edges, tapered, come below 0.8 of it).
        assert!(
            lo > 0.05 && hi < 1.2,
            "stand top {lo} … {hi} of its height over the ground"
        );
        let full = shares.iter().filter(|s| **s > 0.8).count();
        assert!(
            full * 10 > shares.len() * 9,
            "{full} of {} at full height",
            shares.len()
        );
    }

    /// The same next to a junction, where the renderer lowers the terrain
    /// under the side road's blend (`junction_mesh::lower_terrain`): the
    /// stands there follow the lowered ground, not the package's.
    #[test]
    fn stands_follow_the_ground_lowered_at_a_junction() {
        use ocw_world::graph::RoadClass;
        use ocw_world::junction::fixtures::t_junction;
        // A T junction at the tile's corner (0, 0): the main road along
        // z = 0, a 20 % side road up along x = 0; the ground 5 cm under the
        // roads' profiles as the world builder settles it.
        let mut pkg = field_tile(|_, k| {
            let z = k as f32 * 10.0;
            if z > 3.75 { 0.2 * z - 0.05 } else { -0.05 }
        });
        pkg.graph = Some(t_junction(RoadClass::Secondary, 0.0, 0.2));
        let key = TileKey { x: 0, z: 0 };
        let m = crate::package::build_tile_meshes(&pkg, key).unwrap();
        // The renderer lowered the ground beside the side road.
        let tile = pkg.tiles.get(&key).unwrap();
        let lowered = m
            .terrain
            .positions
            .iter()
            .zip(&tile.heights)
            .map(|(p, h)| h - p[1])
            .fold(0.0, f32::max);
        assert!(lowered > 0.3, "lowered by {lowered} m");
        let (shares, cells) = stand_heights_over_the_ground(&pkg);
        assert!(cells > 9_000, "{cells} stand cells");
        let (lo, hi) = shares
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), s| (lo.min(*s), hi.max(*s)));
        // Never under the ground, never above its own height (only the
        // edges, tapered, come below 0.8 of it).
        assert!(
            lo > 0.05 && hi < 1.2,
            "stand top {lo} … {hi} of its height over the ground"
        );
        let full = shares.iter().filter(|s| **s > 0.8).count();
        assert!(
            full * 10 > shares.len() * 9,
            "{full} of {} at full height",
            shares.len()
        );
    }

    /// A flat field of `crop` with a road across it along x = 500 (the
    /// cells within its reach are kept free): its stand cells.
    fn field_by_a_road(crop: FieldCrop) -> Vec<StandCell> {
        let mut pkg = field_tile(|_, _| 300.0);
        let v = pkg.look.vegetation.as_mut().unwrap();
        let t = v.tiles.get_mut(&TileKey { x: 0, z: 0 }).unwrap();
        t.crops.iter_mut().for_each(|c| *c = crop.code());
        let tile = pkg.tiles.get_mut(&TileKey { x: 0, z: 0 }).unwrap();
        tile.roads.push(ocw_world::RoadStrip {
            class: ocw_world::graph::RoadClass::Secondary,
            width_m: 6.0,
            points: vec![[500.0, 300.0, 0.0], [500.0, 300.0, 1000.0]],
            surface: ocw_world::RoadSurface::Asphalt,
            dress: ocw_world::RoadDress::of_class(ocw_world::graph::RoadClass::Secondary, 6.0),
            lit: ocw_world::RoadLit::Unknown,
        });
        let m = crate::package::build_tile_meshes(&pkg, TileKey { x: 0, z: 0 }).unwrap();
        let mut cells = Vec::new();
        for ch in &m.plants.chunks {
            let o = Vec3::from(ch.origin);
            cells.extend(ch.near.stands.iter().map(|c| StandCell {
                centre: c.centre + o,
                ..*c
            }));
        }
        cells
    }

    /// Regression (2026-11-17): the cells kept free along a road counted as
    /// the same field, so the stand's edge beside the road had no side and
    /// its top hung in the air over the verge. A tall stand (maize) now ends
    /// in a side there; a low one (potatoes) comes down to the ground at its
    /// edge, with no side anywhere.
    #[test]
    fn stands_end_at_the_road_in_a_side_or_on_the_ground() {
        // The road (6 m) keeps the cells within 7 m of x = 500 free: the
        // columns from 490 to 510; the field's edges are at x 490 and 510.
        let maize = field_by_a_road(FieldCrop::Maize);
        assert!(!maize.is_empty());
        let by_road: Vec<&StandCell> = maize
            .iter()
            .filter(|c| (c.centre.x - 485.0).abs() < 0.1 && c.centre.z > 20.0 && c.centre.z < 980.0)
            .collect();
        assert!(!by_road.is_empty());
        // The x+ side (towards the road) is open; the rows inside are not.
        assert!(
            by_road
                .iter()
                .all(|c| c.open == [false, true, false, false]),
            "{:?}",
            by_road[0].open
        );
        let (tall, _) = stand(FieldCrop::Maize, None, 300.0);
        assert!(
            by_road
                .iter()
                .all(|c| c.tops.iter().all(|t| *t > 0.8 * tall))
        );
        // Potatoes: no sides; the corners on the road's side lie on the
        // ground, those inside the field at the stand's height.
        let potato = field_by_a_road(FieldCrop::Potato);
        assert!(potato.iter().all(|c| c.open == [false; 4]));
        let by_road: Vec<&StandCell> = potato
            .iter()
            .filter(|c| (c.centre.x - 485.0).abs() < 0.1 && c.centre.z > 20.0 && c.centre.z < 980.0)
            .collect();
        assert!(!by_road.is_empty());
        let (low, _) = stand(FieldCrop::Potato, None, 300.0);
        for c in by_road {
            // Corners x−z−, x+z−, x−z+, x+z+: x+ is the road's side.
            assert_eq!([c.tops[1], c.tops[3]], [EDGE_LIFT_M; 2]);
            assert!(c.tops[0] > 0.8 * low && c.tops[2] > 0.8 * low);
        }
    }

    #[test]
    fn near_meshes_come_and_go_with_the_distance() {
        let l = LazyPlants::near(NearPlants::default(), Handle::default());
        assert_eq!(l.step(100.0, false), LazyStep::Build);
        assert_eq!(l.step(NEAR_FADE_M.end, false), LazyStep::Build);
        assert_eq!(l.step(NEAR_BUILD_M - 1.0, false), LazyStep::Prebuild);
        assert_eq!(l.step(NEAR_BUILD_M + 1.0, false), LazyStep::Keep);
        assert_eq!(l.step(NEAR_DROP_M - 1.0, true), LazyStep::Keep);
        assert_eq!(l.step(NEAR_DROP_M + 1.0, true), LazyStep::Drop);
        let d = LazyPlants::detail(NearPlants::default(), Handle::default());
        assert_eq!(d.step(DETAIL_FADE_M.end, false), LazyStep::Build);
        assert_eq!(d.step(DETAIL_BUILD_M - 1.0, false), LazyStep::Prebuild);
        assert_eq!(d.step(DETAIL_DROP_M + 1.0, true), LazyStep::Drop);
        // Between building and dropping a chunk keeps what it has.
        const { assert!(NEAR_FADE_M.end < NEAR_BUILD_M && NEAR_BUILD_M < NEAR_DROP_M) };
        const { assert!(DETAIL_FADE_M.end < DETAIL_BUILD_M && DETAIL_BUILD_M < DETAIL_DROP_M) };
    }

    /// The chunks' near meshes are built near the camera only (the nearest
    /// ahead first) and dropped again when it goes away; the recipes give
    /// the same mesh as drawing the plants directly.
    #[test]
    fn lazy_plants_build_near_the_camera_and_drop_far_away() {
        let pkg = worst_tile(47.14, 9.52, LandClass::Forest, None);
        let m = crate::package::build_tile_meshes(&pkg, TileKey { x: 0, z: 0 }).unwrap();
        let near = m.plants.chunks[0].near.clone();
        let built = near.build();
        let mut direct = MeshArrays::default();
        for p in &near.plants {
            crate::flora::plant(&mut direct, p.sp, &p.at, &p.look, Detail::Near);
        }
        assert_eq!(built.positions, direct.positions);
        assert!(!built.indices.is_empty());
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<crate::see_through::SeeThroughMaterial>()
            .add_systems(Update, lazy_plants);
        let camera = app
            .world_mut()
            .spawn((
                Camera3d::default(),
                Camera::default(),
                GlobalTransform::from_translation(Vec3::new(0.0, 2.0, 0.0)),
            ))
            .id();
        let centre = near.centre();
        let mut spawn = |x: f32| {
            app.world_mut()
                .spawn((
                    LazyPlants::near(near.clone(), Handle::default()),
                    GlobalTransform::from_translation(Vec3::new(x, 2.0, 0.0) - centre),
                ))
                .id()
        };
        // In view, ahead (one more than a frame's prebuilds), far away.
        let seen = spawn(100.0);
        let ahead: Vec<Entity> = (0..=PREBUILD_PER_FRAME)
            .map(|i| spawn(NEAR_BUILD_M - 10.0 + i as f32))
            .collect();
        let far = spawn(NEAR_BUILD_M + 50.0);
        let has = |app: &App, e: Entity| app.world().get::<Mesh3d>(e).is_some();
        let started =
            |app: &App, e: Entity| has(app, e) || app.world().get::<LazyBuild>(e).is_some();
        // Until every started build is done (on the task pool's threads).
        let settle = |app: &mut App| {
            for _ in 0..500 {
                app.update();
                let w = app.world_mut();
                if w.query::<&LazyBuild>().iter(w).next().is_none() {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            panic!("near meshes not built");
        };
        app.update(); // Just spawned: no position yet.
        assert!(!started(&app, seen));
        app.update();
        // In view: built in this frame; ahead: started.
        assert!(has(&app, seen) && !started(&app, far));
        let first: Vec<bool> = ahead.iter().map(|e| started(&app, *e)).collect();
        assert_eq!(first.iter().filter(|b| **b).count(), PREBUILD_PER_FRAME);
        assert!(!first[PREBUILD_PER_FRAME], "the farthest waits");
        settle(&mut app);
        assert!(has(&app, seen));
        assert!(ahead.iter().all(|e| has(&app, *e)));
        assert!(!started(&app, far));
        // The camera goes away: the meshes are dropped.
        app.world_mut()
            .entity_mut(camera)
            .insert(GlobalTransform::from_translation(Vec3::new(
                5_000.0, 2.0, 0.0,
            )));
        app.update();
        assert!(!has(&app, seen) && ahead.iter().all(|e| !has(&app, *e)));
    }
}
