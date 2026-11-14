//! Far terrain of world packages (spec §4.5 "Far terrain"): every tile of
//! every resident package as a coarse mesh (100 m grid), drawn where the
//! streamed tiles of [`crate::package`] are not – out to the horizon, so
//! the Teide stands above Las Cañadas from 8 km and above the coast from
//! 40 km. A far tile is hidden while its full tile is loaded (no z-fight,
//! no coarse triangles poking through the 10 m terrain); skirts hide the
//! cracks between the two grids along the border.
//!
//! With the far terrain the view reaches farther: the camera's far plane
//! is [`FAR_VIEW_M`], and the fog thins out with the distance to the end
//! of the package ([`package_fog`]). Packages with sea (islands, spec
//! §4.6) get a sea plane at 0 m out to the horizon, so the open sea
//! without tiles is water, not the sky.

use std::collections::HashMap;

use bevy::light::NotShadowCaster;
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task, futures::check_ready};
use ocw_world::WorldSet;
use ocw_world::landuse::LandClass;
use ocw_world::package::{Tile, TileKey, WorldPackage};

use crate::camera::FollowCamera;
use crate::materials::MaterialLibrary;
use crate::package::{
    Keep, MeshArrays, PackageMaterials, PackageWorld, TileId, TileLook, TileStreamer,
    frame_transform, land_colour, tile_landuse,
};

/// Grid spacing of the far terrain (m): every 10th sample of the 10 m grid.
pub const FAR_SPACING_M: f64 = 100.0;
/// The skirts along a far tile's border reach this far down (m).
pub const FAR_SKIRT_M: f32 = 40.0;
/// Camera far plane with a world package (m; Bevy's default is 1 km).
pub const FAR_VIEW_M: f32 = 120_000.0;
/// Where the fog of a package world starts (m), as on the test routes.
pub const FOG_START_M: f32 = 250.0;
/// Fog end with nothing missing around (islands: the sea plane closes the
/// horizon) (m): the Teide at 8 km is 13 % fogged, the coast at 40 km 66 %.
pub const FOG_END_M: f32 = 60_000.0;
/// Fog end at least, however near the package's end (m): the old fog.
pub const FOG_END_MIN_M: f32 = 1_800.0;
/// The sea plane lies this far below the sea level (m): under the tiles'
/// water (0 m), above their carved sea floor (−2 m).
pub const SEA_PLANE_Y: f32 = -0.05;
/// Half-size of the sea plane beyond the package's tiles (m).
const SEA_MARGIN_M: f32 = 100_000.0;

const ROCK: [f32; 3] = [0.5, 0.48, 0.44];
/// The tree crowns seen from afar (sRGB): the mean crown colour of
/// `package::tree_arrays`, shaded (their sides and shadows).
pub const CANOPY: [f32; 3] = [0.11, 0.25, 0.11];

/// How much of the ground the trees of a land class hide from afar
/// (their chance per cell, `package::tree_density`, seen at a slant).
pub fn tree_cover(class: LandClass) -> f32 {
    (crate::package::tree_density(class).0 * 1.1).min(0.95)
}

/// Samples of the package grid per far grid step: the largest divisor of
/// the cells per side not above [`FAR_SPACING_M`] (so the far vertices are
/// package samples and the borders of neighbouring far tiles agree).
pub fn far_stride(pkg: &WorldPackage) -> usize {
    let cells = pkg.samples as usize - 1;
    let want = ((FAR_SPACING_M / pkg.spacing_m()).round() as usize).clamp(1, cells);
    (1..=want)
        .rev()
        .find(|d| cells.is_multiple_of(*d))
        .unwrap_or(1)
}

fn linear(c: [f32; 3]) -> [f32; 4] {
    let l = Color::srgb(c[0], c[1], c[2]).to_linear();
    [l.red, l.green, l.blue, 1.0]
}

/// Far mesh of a tile, in the package's metres: the heights of every
/// [`far_stride`]-th sample, vertex colours from the land use averaged
/// over the far cell around each vertex (the classes' colours of `look`,
/// turning to rock on steep slopes), and skirts [`FAR_SKIRT_M`] deep along
/// the border. Cells whose centre `keep` rejects are left out.
pub fn far_tile_arrays(pkg: &WorldPackage, tile: &Tile, look: &TileLook, keep: Keep) -> MeshArrays {
    let n = pkg.samples as usize;
    let stride = far_stride(pkg);
    let m = (n - 1) / stride + 1;
    let s = pkg.spacing_m() * stride as f64;
    let size = f64::from(pkg.tile_size_m);
    let o = tile.key.origin(size);
    let h =
        |i: usize, k: usize| tile.heights[(k * stride).min(n - 1) * n + (i * stride).min(n - 1)];
    let landuse = tile_landuse(pkg, tile.key);
    let palette = look.palette.as_deref();
    let flat = |c: LandClass| {
        palette
            .and_then(|p| p.get(c as usize).copied())
            .unwrap_or_else(|| land_colour(c))
    };
    let mut out = MeshArrays::default();
    for k in 0..m {
        for i in 0..m {
            let (il, ir) = (i.saturating_sub(1), (i + 1).min(m - 1));
            let (ku, kd) = (k.saturating_sub(1), (k + 1).min(m - 1));
            let dx = (h(ir, k) - h(il, k)) / ((ir - il) as f64 * s) as f32;
            let dz = (h(i, kd) - h(i, ku)) / ((kd - ku) as f64 * s) as f32;
            out.positions.push([
                (o[0] + i as f64 * s) as f32,
                h(i, k),
                (o[1] + k as f64 * s) as f32,
            ]);
            out.normals
                .push(Vec3::new(-dx, 1.0, -dz).normalize().to_array());
            let slope = (dx * dx + dz * dz).sqrt();
            let rock_t = |class: LandClass| match palette {
                Some(_) => crate::ground::rock_weight_under(
                    slope,
                    crate::ground::class_weights(class)[crate::ground::FOREST_FLOOR],
                ),
                None => ((slope - 0.6) / 0.4).clamp(0.0, 1.0),
            };
            let rock = palette.map_or(ROCK, |_| flat(LandClass::Rock));
            let colour = match landuse {
                Some((lu, cells)) => {
                    // The mean colour of 4 × 4 points over the far cell.
                    let (cells, cs) = (cells as usize, size / f64::from(cells));
                    let mut sum = [0.0f32; 3];
                    for b in 0..4 {
                        for a in 0..4 {
                            let x = (i as f64 + (a as f64 + 0.5) / 4.0 - 0.5) * s;
                            let z = (k as f64 + (b as f64 + 0.5) / 4.0 - 0.5) * s;
                            let c = ((x / cs).floor().max(0.0) as usize).min(cells - 1);
                            let r = ((z / cs).floor().max(0.0) as usize).min(cells - 1);
                            let class = lu.classes[r * cells + c];
                            let mut col = flat(class);
                            if !matches!(class, LandClass::Water | LandClass::Glacier) {
                                let t = rock_t(class);
                                col = std::array::from_fn(|j| col[j] + (rock[j] - col[j]) * t);
                                // The far tiles have no trees: their crowns
                                // tint the ground (not on rock faces).
                                let w = tree_cover(class) * (1.0 - t);
                                col = std::array::from_fn(|j| col[j] + (CANOPY[j] - col[j]) * w);
                            }
                            for (acc, c) in sum.iter_mut().zip(col) {
                                *acc += c / 16.0;
                            }
                        }
                    }
                    sum
                }
                None => {
                    let grass = [0.33, 0.52, 0.25];
                    let t = rock_t(LandClass::Default);
                    std::array::from_fn(|j| grass[j] + (ROCK[j] - grass[j]) * t)
                }
            };
            out.colors.push(linear(colour));
        }
    }
    let v = |i: usize, k: usize| (k * m + i) as u32;
    let kept = |i: usize, k: usize| {
        keep.is_none_or(|f| f([o[0] + (i as f64 + 0.5) * s, o[1] + (k as f64 + 0.5) * s]))
    };
    for k in 0..m - 1 {
        for i in 0..m - 1 {
            if kept(i, k) {
                let (a, b, c, d) = (v(i, k), v(i + 1, k), v(i, k + 1), v(i + 1, k + 1));
                out.indices.extend([a, c, b, b, c, d]);
            }
        }
    }
    // Skirts: the border vertices again, lower, joined to the border by
    // quads (both faces drawn: the far material is double-sided).
    let border: Vec<((usize, usize), (usize, usize))> = (0..m - 1)
        .flat_map(|j| {
            [
                ((j, 0), (j + 1, 0)),
                ((j, m - 1), (j + 1, m - 1)),
                ((0, j), (0, j + 1)),
                ((m - 1, j), (m - 1, j + 1)),
            ]
        })
        .collect();
    for ((i0, k0), (i1, k1)) in border {
        let cell = (i0.min(i1).min(m - 2), k0.min(k1).min(m - 2));
        if !kept(cell.0, cell.1) {
            continue;
        }
        let base = out.positions.len() as u32;
        for (i, k) in [(i0, k0), (i1, k1)] {
            let top = v(i, k) as usize;
            let mut p = out.positions[top];
            p[1] -= FAR_SKIRT_M;
            out.positions.push(p);
            out.normals.push(out.normals[top]);
            out.colors.push(out.colors[top]);
        }
        let (a, b) = (v(i0, k0), v(i1, k1));
        out.indices.extend([a, base, b, b, base, base + 1]);
    }
    out
}

/// Whether a package has sea (spec §4.6): water vertices at exactly 0 m,
/// the level the builder gives the sea (lakes get their ground's level).
pub fn has_sea(pkg: &WorldPackage) -> bool {
    pkg.landuse.as_ref().is_some_and(|l| {
        l.tiles
            .values()
            .any(|t| t.water.iter().any(|&(_, level)| level == 0.0))
    })
}

/// The box of a package's tiles (min x, min z, max x, max z; its metres).
pub fn tile_bounds(pkg: &WorldPackage) -> Option<[f64; 4]> {
    let size = f64::from(pkg.tile_size_m);
    pkg.tiles.keys().fold(None, |b, k| {
        let o = k.origin(size);
        let t = [o[0], o[1], o[0] + size, o[1] + size];
        Some(union(b, t))
    })
}

/// Distance from `p` to the end of the terrain, the border of `extent`
/// (the box of all tiles, home metres); `None` without an end: a world
/// with sea (the sea plane goes on to the horizon) or without tiles.
pub fn edge_distance(extent: Option<[f64; 4]>, sea: bool, p: [f64; 2]) -> Option<f64> {
    if sea {
        return None;
    }
    let b = extent?;
    Some(
        (p[0] - b[0])
            .min(b[2] - p[0])
            .min(p[1] - b[1])
            .min(b[3] - p[1])
            .max(0.0),
    )
}

/// The union of two boxes.
fn union(a: Option<[f64; 4]>, b: [f64; 4]) -> [f64; 4] {
    a.map_or(b, |a| {
        [
            a[0].min(b[0]),
            a[1].min(b[1]),
            a[2].max(b[2]),
            a[3].max(b[3]),
        ]
    })
}

/// Fog of a package world: from [`FOG_START_M`] to the end of the
/// terrain around (`edge`, see [`edge_distance`]; at least
/// [`FOG_END_MIN_M`]), at most [`FOG_END_M`] – without an end (islands)
/// [`FOG_END_M`].
pub fn package_fog(edge: Option<f64>) -> FogFalloff {
    let end = edge.map_or(FOG_END_M, |e| (e as f32).clamp(FOG_END_MIN_M, FOG_END_M));
    FogFalloff::Linear {
        start: FOG_START_M,
        end,
    }
}

/// The far tiles of the world set, built off the main thread.
#[derive(Resource, Default)]
pub(crate) struct FarTerrain {
    /// Tile entities by tile, and their parents (one per package).
    tiles: HashMap<TileId, Entity>,
    parents: Vec<Entity>,
    /// What they were built for: world set generation, streamed tiles
    /// ([`WorldSet::tiles_version`](ocw_world::WorldSet::tiles_version)), material library,
    /// season bucket of the world date (the region look, spec §14 Phase 8
    /// T3).
    built: Option<FarKey>,
    task: Option<(FarKey, Task<Vec<FarPackage>>)>,
    /// The box of all tiles of the built set (home metres) and whether a
    /// package of it has sea ([`edge_distance`]).
    extent: Option<[f64; 4]>,
    sea: bool,
}

impl FarTerrain {
    /// Despawns everything (a ride switch).
    pub(crate) fn clear(&mut self, world: &mut World) {
        for e in std::mem::take(&mut self.parents) {
            if let Ok(e) = world.get_entity_mut(e) {
                e.despawn();
            }
        }
        *self = Self::default();
    }
}

/// What the far terrain is built for (see [`FarTerrain::built`]).
type FarKey = (u64, u64, bool, Option<u32>);

/// The far meshes of one package.
struct FarPackage {
    member: usize,
    tiles: Vec<(TileKey, MeshArrays)>,
    /// The box of its tiles (its metres).
    bounds: Option<[f64; 4]>,
    /// The sea plane's box (its metres), if the package has sea.
    sea: Option<[f64; 4]>,
}

fn build_far(set: &WorldSet, looks: &[TileLook]) -> Vec<FarPackage> {
    let mut out = Vec::new();
    for (i, m) in set.members().iter().enumerate() {
        let Some(pkg) = m.package() else { continue };
        let look = looks.get(i).cloned().unwrap_or_default();
        let keep = |q: [f64; 2]| set.draws(i, q);
        let keep: Keep = if set.has_seams() { Some(&keep) } else { None };
        let tiles = pkg
            .tiles
            .values()
            .filter(|t| !t.heights.is_empty())
            .map(|t| (t.key, far_tile_arrays(pkg, t, &look, keep)))
            .collect();
        let bounds = tile_bounds(pkg);
        let sea = bounds.filter(|_| has_sea(pkg));
        out.push(FarPackage {
            member: i,
            tiles,
            bounds,
            sea,
        });
    }
    out
}

/// Builds the far terrain when the world set (or the look) changes and
/// spawns it when built; the old far terrain stays until then.
#[allow(clippy::too_many_arguments)] // a Bevy system
pub(crate) fn update_far_terrain(
    mut commands: Commands,
    world: Res<PackageWorld>,
    mut far: ResMut<FarTerrain>,
    materials: Option<Res<PackageMaterials>>,
    library: Option<Res<MaterialLibrary>>,
    view: Option<Res<crate::RideView>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut standard: ResMut<Assets<StandardMaterial>>,
    mut material: Local<Option<Handle<StandardMaterial>>>,
) {
    let set = world.set.clone();
    // The world date in buckets, as the streamed tiles take it.
    let season = view.as_deref().and_then(|v| v.sky.as_ref()).map(|s| {
        crate::region_look::season_bucket(ocw_world::look::day_of_year(
            s.sky.local.year,
            s.sky.local.month,
            s.sky.local.day,
        ))
    });
    let want: FarKey = (
        set.generation(),
        set.tiles_version(),
        library.is_some(),
        season,
    );
    let pending = far.task.as_ref().map(|t| t.0);
    if far.built != Some(want) && pending != Some(want) {
        // The class colours of the streamed tiles (the region look tinted
        // by the season), so near and far agree.
        let day = season.map(crate::region_look::bucket_day);
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
            })
            .collect();
        let s = set.clone();
        far.task = Some((
            want,
            AsyncComputeTaskPool::get().spawn(async move { build_far(&s, &looks) }),
        ));
    }
    let Some(materials) = materials else { return };
    let Some((key, task)) = far.task.as_mut() else {
        return;
    };
    let Some(built) = check_ready(task) else {
        return;
    };
    let done = *key;
    far.task = None;
    for e in std::mem::take(&mut far.parents) {
        commands.entity(e).despawn();
    }
    far.tiles.clear();
    far.built = Some(done);
    far.extent = None;
    far.sea = false;
    let mat = material
        .get_or_insert_with(|| {
            standard.add(StandardMaterial {
                base_color: Color::WHITE, // vertex colours
                perceptual_roughness: 1.0,
                cull_mode: None,
                double_sided: true,
                ..default()
            })
        })
        .clone();
    for p in built {
        let Some(member) = set.members().get(p.member) else {
            continue;
        };
        far.sea |= p.sea.is_some();
        if let Some(t) = p.bounds {
            let (lo, hi) = (
                member.frame.apply([t[0], t[1]]),
                member.frame.apply([t[2], t[3]]),
            );
            far.extent = Some(union(far.extent, [lo[0], lo[1], hi[0], hi[1]]));
        }
        let mut ids = Vec::with_capacity(p.tiles.len());
        let parent = commands
            .spawn((
                frame_transform(&member.frame),
                Visibility::default(),
                Name::new(format!("far terrain of {}", member.id)),
            ))
            .with_children(|c| {
                for (key, arrays) in p.tiles {
                    let e = c
                        .spawn((
                            FarTile,
                            Mesh3d(meshes.add(arrays.mesh())),
                            MeshMaterial3d(mat.clone()),
                            NotShadowCaster,
                            Visibility::Visible,
                        ))
                        .id();
                    ids.push(((p.member, key), e));
                }
                if let Some(b) = p.sea {
                    let m = f64::from(SEA_MARGIN_M);
                    let (min, max) = ([b[0] - m, b[1] - m], [b[2] + m, b[3] + m]);
                    c.spawn((
                        SeaPlane,
                        Mesh3d(meshes.add(sea_plane(min, max))),
                        MeshMaterial3d(materials.water.clone()),
                        NotShadowCaster,
                    ));
                }
            })
            .id();
        far.parents.push(parent);
        far.tiles.extend(ids);
    }
}

/// A far tile (tests).
#[derive(Component)]
pub struct FarTile;

/// The sea plane of a package with sea (tests).
#[derive(Component)]
pub struct SeaPlane;

fn sea_plane(min: [f64; 2], max: [f64; 2]) -> Mesh {
    let (x0, z0, x1, z1) = (min[0] as f32, min[1] as f32, max[0] as f32, max[1] as f32);
    let y = SEA_PLANE_Y;
    MeshArrays {
        positions: vec![[x0, y, z0], [x1, y, z0], [x0, y, z1], [x1, y, z1]],
        normals: vec![[0.0, 1.0, 0.0]; 4],
        // The colour of the tiles' water (`package::water_arrays`).
        colors: vec![linear([0.16, 0.30, 0.37]); 4],
        indices: vec![0, 2, 1, 1, 2, 3],
        ..Default::default()
    }
    .mesh()
}

/// A far tile is hidden while its full tile is loaded.
pub(crate) fn hide_far_under_near(
    far: Res<FarTerrain>,
    streamer: Res<TileStreamer>,
    mut tiles: Query<&mut Visibility, With<FarTile>>,
) {
    for (id, &e) in &far.tiles {
        let want = if streamer.loaded.contains_key(id) {
            Visibility::Hidden
        } else {
            Visibility::Visible
        };
        if let Ok(mut v) = tiles.get_mut(e)
            && *v != want
        {
            *v = want;
        }
    }
}

/// With a package the camera sees to [`FAR_VIEW_M`] and the fog ends at
/// the end of the terrain ([`package_fog`]); without one (a route world)
/// the far plane and the fog of [`crate::world::fog`].
pub(crate) fn package_view(
    world: Option<Res<PackageWorld>>,
    far: Res<FarTerrain>,
    view: Option<Res<crate::RideView>>,
    clouds: Option<Res<crate::weather::clouds::CloudState>>,
    mut cameras: Query<(&mut Projection, Option<&mut DistanceFog>), With<FollowCamera>>,
) {
    let p = view.as_deref().and_then(|view| {
        let l = view.local()?;
        let q = view.track(l.id).sample(l.state.distance).position;
        Some([q[0], q[2]])
    });
    let (far_m, falloff) = if world.is_some() {
        // No rider (or no far terrain) yet: the near fog.
        let edge = match p {
            Some(p) if far.built.is_some() => edge_distance(far.extent, far.sea, p),
            _ => Some(0.0),
        };
        (FAR_VIEW_M, package_fog(edge))
    } else {
        (
            PerspectiveProjection::default().far,
            crate::world::fog().falloff,
        )
    };
    // The weather's fog (spec §14 Phase 9, S4): its visibility when it is
    // shorter than the distance fog's reach.
    let falloff = match view
        .as_deref()
        .and_then(|v| crate::weather::fog_visibility(v, clouds.as_deref()))
    {
        Some(vis) if vis < 0.9 * fog_reach(&falloff) => FogFalloff::from_visibility(vis),
        _ => falloff,
    };
    for (mut projection, fog) in &mut cameras {
        if let Projection::Perspective(pp) = &mut *projection
            && pp.far != far_m
        {
            pp.far = far_m;
        }
        if let Some(mut fog) = fog
            && !same_falloff(&fog.falloff, &falloff)
        {
            fog.falloff = falloff.clone();
        }
    }
}

fn same_falloff(a: &FogFalloff, b: &FogFalloff) -> bool {
    match (a, b) {
        (FogFalloff::Linear { start: a, end: b }, FogFalloff::Linear { start: c, end: d }) => {
            a == c && (b - d).abs() < 1.0
        }
        (FogFalloff::Exponential { density: a }, FogFalloff::Exponential { density: b }) => {
            (a - b).abs() <= 0.002 * a.abs().max(b.abs())
        }
        _ => false,
    }
}

/// How far one sees through a fog falloff (m): a linear fog's end, an
/// exponential fog's Koschmieder distance.
pub fn fog_reach(f: &FogFalloff) -> f32 {
    match f {
        FogFalloff::Linear { end, .. } => *end,
        FogFalloff::Exponential { density } | FogFalloff::ExponentialSquared { density } => {
            3.0 / density.max(1e-9)
        }
        _ => f32::INFINITY,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use ocw_world::landuse::{LandUse, LandUseTile};

    use super::*;
    use crate::camera::CameraObstacles;
    use crate::ground::TerrainMaterial;
    use crate::package::PackagePlugin;
    use crate::see_through::SeeThroughMaterial;

    /// `count` tiles of 1 km in a row along x, 10 m grid, a cone of
    /// 3,700 m in tile 5 (like the Teide), 100 m elsewhere.
    fn package(count: i32) -> WorldPackage {
        let n = 101;
        let tiles = (0..count)
            .map(|x| {
                let key = TileKey { x, z: 0 };
                let heights = (0..n * n)
                    .map(|j| {
                        let (i, k) = ((j % n) as f64 * 10.0, (j / n) as f64 * 10.0);
                        let r = (f64::from(x) * 1000.0 + i - 5500.0).hypot(k - 500.0);
                        (100.0 + 3600.0 * (1.0 - r / 500.0).max(0.0)) as f32
                    })
                    .collect();
                (
                    key,
                    Tile {
                        key,
                        heights,
                        roads: Vec::new(),
                    },
                )
            })
            .collect();
        WorldPackage {
            name: "t".into(),
            origin: (28.29, -16.52),
            tile_size_m: 1000.0,
            samples: n as u32,
            tiles,
            graph: None,
            landuse: None,
            buildings: None,
            bridges: None,
            routes: Vec::new(),
            region: None,
            time_zone: None,
            look: Default::default(),
            obstacles: None,
        }
    }

    #[test]
    fn the_far_grid_takes_every_tenth_sample_of_the_10_m_grid() {
        let mut p = package(1);
        assert_eq!(far_stride(&p), 10);
        p.samples = 51; // 20 m
        assert_eq!(far_stride(&p), 5);
        p.samples = 7; // 166 m: every sample
        assert_eq!(far_stride(&p), 1);
        p.samples = 61; // 16.7 m: 6 of the 60 cells
        assert_eq!(far_stride(&p), 6);
    }

    #[test]
    fn a_far_tile_has_the_package_heights_skirts_and_faces_up() {
        let p = package(6);
        let tile = &p.tiles[&TileKey { x: 5, z: 0 }];
        let m = far_tile_arrays(&p, tile, &TileLook::default(), None);
        // 11 × 11 vertices, 4 × 10 border edges with two skirt vertices each.
        assert_eq!(m.positions.len(), 121 + 80);
        assert_eq!(m.indices.len(), 100 * 6 + 40 * 6);
        // The summit (x 5500, z 500) is a vertex at its full height.
        assert_eq!(m.positions[5 * 11 + 5], [5500.0, 3700.0, 500.0]);
        for k in 0..11 {
            for i in 0..11 {
                let q = m.positions[k * 11 + i];
                assert_eq!(q[1], tile.heights[k * 10 * 101 + i * 10]);
            }
        }
        // The grid faces up; the skirts hang 40 m below the border.
        for tri in m.indices[..600].chunks(3) {
            let [a, b, c] = [0, 1, 2].map(|j| Vec3::from(m.positions[tri[j] as usize]));
            assert!((b - a).cross(c - a).y > 0.0);
        }
        let low = m.positions[121..]
            .iter()
            .map(|q| q[1])
            .fold(f32::MAX, f32::min);
        assert!((low - (100.0 - FAR_SKIRT_M)).abs() < 1e-3, "{low}");
        // Kept cells only (a seam): the western half, its border skirts
        // (5 + 5 along z, 10 along x).
        let keep = |q: [f64; 2]| q[0] < 5500.0;
        let half = far_tile_arrays(&p, tile, &TileLook::default(), Some(&keep));
        assert_eq!(half.indices.len(), 50 * 6 + (5 + 5 + 10) * 6);
    }

    #[test]
    fn far_colours_are_the_land_use_means() {
        let mut p = package(1);
        let key = TileKey { x: 0, z: 0 };
        let cells = 200usize;
        let classes = (0..cells * cells)
            .map(|j| {
                if j % cells < 100 {
                    LandClass::Forest
                } else {
                    LandClass::Water
                }
            })
            .collect();
        p.landuse = Some(LandUse {
            cells: cells as u32,
            tiles: BTreeMap::from([(
                key,
                LandUseTile {
                    classes,
                    water: vec![(5, 0.0)],
                },
            )]),
        });
        assert!(has_sea(&p));
        let m = far_tile_arrays(&p, &p.tiles[&key], &TileLook::default(), None);
        let water = linear(land_colour(LandClass::Water));
        let close = |a: [f32; 4], b: [f32; 4]| (0..3).all(|j| (a[j] - b[j]).abs() < 1e-3);
        // Vertex (2, 0) is in the forest, (8, 0) in the water, (5, 0) on the
        // border between them: half and half.
        // The forest far away: ground under the crowns.
        let w = tree_cover(LandClass::Forest);
        let forest = linear(std::array::from_fn(|j| {
            land_colour(LandClass::Forest)[j] * (1.0 - w) + CANOPY[j] * w
        }));
        assert!(w > 0.9);
        assert!(close(m.colors[2], forest));
        assert!(close(m.colors[8], water));
        assert!((0..3).all(|j| (m.colors[5][j] - (forest[j] + water[j]) / 2.0).abs() < 0.02));
        // A lake's level is not the sea.
        p.landuse
            .as_mut()
            .unwrap()
            .tiles
            .get_mut(&key)
            .unwrap()
            .water = vec![(5, 412.0)];
        assert!(!has_sea(&p));
    }

    #[test]
    fn the_fog_ends_at_the_end_of_the_terrain_or_far_out_at_sea() {
        let b = Some([0.0, 0.0, 10_000.0, 4_000.0]);
        assert_eq!(edge_distance(b, false, [3_000.0, 1_000.0]), Some(1_000.0));
        assert_eq!(edge_distance(b, false, [-50.0, 1_000.0]), Some(0.0));
        assert_eq!(edge_distance(b, true, [3_000.0, 1_000.0]), None);
        assert_eq!(edge_distance(None, false, [0.0, 0.0]), None);
        let end = |e| match package_fog(e) {
            FogFalloff::Linear { start, end } => {
                assert_eq!(start, FOG_START_M);
                end
            }
            _ => panic!("linear"),
        };
        assert_eq!(end(None), FOG_END_M);
        assert_eq!(end(Some(0.0)), FOG_END_MIN_M);
        assert_eq!(end(Some(9_000.0)), 9_000.0);
        assert_eq!(end(Some(1e9)), FOG_END_M);
    }

    /// Regression (2026-11-13, Tenerife): only the 5 × 5 tiles around the
    /// rider were drawn and the camera saw 1 km, so the Teide 7 km ahead
    /// was missing even with its tiles in the package. Every tile now has a
    /// far mesh, hidden exactly where the full tile is loaded, and the
    /// camera sees to the horizon.
    #[test]
    fn far_tiles_cover_the_package_except_under_the_loaded_tiles() {
        use ocw_core::RiderId;
        use ocw_world::Route;
        let pts = (0..=100)
            .map(|i| [f64::from(i) * 10.0, 100.0, 500.0])
            .collect();
        let route = Route::from_points("r", "R", pts, false, 6.0).unwrap();
        let mut view = crate::RideView::new(route);
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
        });
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<Image>()
            .init_asset::<StandardMaterial>()
            .init_asset::<SeeThroughMaterial>()
            .init_asset::<TerrainMaterial>()
            .init_resource::<CameraObstacles>()
            .insert_resource(PackageWorld::single(Arc::new(package(10))))
            .insert_resource(view)
            .add_plugins(PackagePlugin);
        let camera = app
            .world_mut()
            .spawn((
                FollowCamera,
                Projection::Perspective(PerspectiveProjection::default()),
                crate::world::fog(),
            ))
            .id();
        for _ in 0..400 {
            app.update();
            if app.world().resource::<FarTerrain>().built.is_some()
                && app.world().resource::<TileStreamer>().loaded.len() == 3
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        app.update();
        let far = app.world().resource::<FarTerrain>();
        assert_eq!(far.tiles.len(), 10);
        let loaded = &app.world().resource::<TileStreamer>().loaded;
        let mut near: Vec<i32> = loaded.keys().map(|(_, k)| k.x).collect();
        near.sort();
        assert_eq!(near, vec![0, 1, 2]);
        for (id, &e) in &far.tiles {
            let v = *app.world().get::<Visibility>(e).unwrap();
            let want = if loaded.contains_key(id) {
                Visibility::Hidden
            } else {
                Visibility::Visible
            };
            assert_eq!(v, want, "{id:?}");
        }
        // The Teide's tile (5) is drawn by its far mesh, at its height.
        let e = far.tiles[&(0, TileKey { x: 5, z: 0 })];
        let mesh = app.world().get::<Mesh3d>(e).unwrap().0.clone();
        let meshes = app.world().resource::<Assets<Mesh>>();
        let top = meshes
            .get(&mesh)
            .unwrap()
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .and_then(|a| a.as_float3())
            .unwrap()
            .iter()
            .map(|p| p[1])
            .fold(f32::MIN, f32::max);
        assert!((top - 3700.0).abs() < 1e-3, "{top}");
        // The camera sees to the horizon; without sea the fog ends at the
        // end of the tiles (the rider 50 m in: the near minimum).
        let w = app.world_mut();
        let Projection::Perspective(pp) = w.get::<Projection>(camera).unwrap() else {
            panic!("perspective");
        };
        assert_eq!(pp.far, FAR_VIEW_M);
        let fog = w.get::<DistanceFog>(camera).unwrap();
        assert!(same_falloff(&fog.falloff, &package_fog(Some(0.0))));
        assert!(w.query::<&SeaPlane>().iter(w).next().is_none());
    }
}
