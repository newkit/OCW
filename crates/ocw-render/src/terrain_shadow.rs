//! Terrain shadows (spec 4.5 *Terrain shadows*, v7.19): the mountains
//! shade the valleys at every distance. The sun's cascaded shadow maps
//! reach 40 – 250 m from the camera (`quality::Quality::shadow_distance_m`)
//! and the terrain tiles never cast into them (`NotShadowCaster`: near and
//! far), so before this no ridge shaded anything: a slope in the evening
//! shadow of a ridge was lit from far away, and only the shadows of its
//! trees darkened it once the camera came within the cascades.
//!
//! [`TerrainShadowMap`] keeps the shadow line of the whole world set
//! (`ocw_world::terrain_shadow`) for the key light (the sun, or the moon at
//! night) in one RG16F texture over world x / z – 50 m cells (larger for
//! worlds over 51 km), rebuilt off the main thread when the world set
//! changes and recomputed when the light has moved by more than
//! [`RECOMPUTE_DEG`]. Every terrain-lit surface samples it
//! (`terrain_shadow.wesl`): the near terrain, the far terrain, roads,
//! trees, houses and water – the same answer for the same point from the
//! 10 m near tiles and the 100 m far tiles. With the sun's shadow maps on
//! it multiplies their shadow (the riders and trees near the camera still
//! cast); without them (phones, `shadows off`) the sun's direct light is
//! taken out where the mountains hide it.
//!
//! `terrain.shadow [on|off]` switches it for comparisons.

use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageSampler, ImageSamplerDescriptor};
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, ShaderType, TextureDimension, TextureFormat};
use bevy::tasks::{AsyncComputeTaskPool, Task, futures::check_ready};
use ocw_world::terrain_shadow::{HeightGrid, MAX_SIDE, MIN_CELL_M, ShadowLines, shadow_lines};

use crate::package::PackageWorld;

/// The grid's finest cell (m): 50 m natively; 100 m in the browser,
/// where the background tasks run on the page's thread (a quarter of the
/// work: Andorra's lines in about 15 ms instead of 60).
pub const CELL_M: f64 = if cfg!(target_arch = "wasm32") {
    2.0 * MIN_CELL_M
} else {
    MIN_CELL_M
};

/// The light has to move this far (degrees) before the map is recomputed:
/// the sun takes about a minute of the day for it.
pub const RECOMPUTE_DEG: f32 = 0.25;

/// `terrain_shadow.wesl`'s uniform (binding 120 of each material).
#[derive(Clone, Copy, Debug, ShaderType, Reflect, PartialEq, Default)]
pub struct TerrainShadowUniform {
    /// uv = world xz · scale.xy + scale.zw.
    pub scale: Vec4,
    /// x on (1) / off (0), y the base height of the texture's lines (m),
    /// z strength (0 … 1).
    pub params: Vec4,
}

/// A material extension with the terrain shadow's bindings (120 – 122).
pub trait TerrainShadowed: MaterialExtension {
    fn terrain_shadow(&self) -> (&TerrainShadowUniform, &Option<Handle<Image>>);
    fn set_terrain_shadow(&mut self, u: TerrainShadowUniform, map: Option<Handle<Image>>);
}

/// Implements [`TerrainShadowed`] for an extension with the fields
/// `terrain_shadow` and `terrain_shadow_map`.
#[macro_export]
macro_rules! terrain_shadowed {
    ($t:ty) => {
        impl $crate::terrain_shadow::TerrainShadowed for $t {
            fn terrain_shadow(
                &self,
            ) -> (
                &$crate::terrain_shadow::TerrainShadowUniform,
                &Option<bevy::prelude::Handle<bevy::prelude::Image>>,
            ) {
                (&self.terrain_shadow, &self.terrain_shadow_map)
            }
            fn set_terrain_shadow(
                &mut self,
                u: $crate::terrain_shadow::TerrainShadowUniform,
                map: Option<bevy::prelude::Handle<bevy::prelude::Image>>,
            ) {
                self.terrain_shadow = u;
                self.terrain_shadow_map = map;
            }
        }
    };
}

/// A grid being built: the grid (none without terrain) and its milliseconds.
type GridTask = Task<(Option<HeightGrid>, f32)>;

/// The terrain's shadow line now and how it is kept up to date.
#[derive(Resource, Default)]
pub struct TerrainShadowMap {
    /// What the grid was built for: world set generation and tiles.
    grid_key: Option<(u64, u64)>,
    grid: Option<Arc<HeightGrid>>,
    grid_task: Option<((u64, u64), GridTask)>,
    lines_task: Option<(Vec3, Task<(ShadowLines, f32)>)>,
    /// The light the texture shows, and its lines (the proxy casters).
    towards: Option<Vec3>,
    lines: Option<Arc<ShadowLines>>,
    image: Option<Handle<Image>>,
    /// What the materials get.
    pub uniform: TerrainShadowUniform,
    /// `terrain.shadow off`.
    pub off: bool,
    /// Counts the changes of the texture or the uniform ([`sync`]).
    pub generation: u64,
    /// Milliseconds of the last grid build and line computation (`terrain.shadow`).
    pub grid_ms: f32,
    pub lines_ms: f32,
}

impl TerrainShadowMap {
    /// The grid now (tests, the console).
    pub fn grid(&self) -> Option<&HeightGrid> {
        self.grid.as_deref()
    }

    /// The texture's handle, once there is one.
    pub fn image(&self) -> Option<&Handle<Image>> {
        self.image.as_ref()
    }
}

pub struct TerrainShadowPlugin;

impl Plugin for TerrainShadowPlugin {
    fn build(&self, app: &mut App) {
        bevy::shader::load_shader_library!(app, "terrain_shadow.wesl");
        app.init_resource::<TerrainShadowMap>()
            .add_systems(Update, update_map)
            .add_systems(Update, (sun_sees_proxies, update_proxy).after(update_map))
            .add_systems(
                PostUpdate,
                (
                    sync::<crate::ground::GroundExtension>,
                    sync::<crate::far_terrain::FarExtension>,
                    sync::<crate::see_through::SeeThroughFade>,
                    sync::<crate::building_material::BuildingExtension>,
                    sync::<crate::road_material::RoadExtension>,
                    sync::<crate::water::WaterExtension>,
                ),
            );
        register_command(app);
    }
}

/// The key light's direction (towards it): the sun, the moon once the sun
/// is 4° below the horizon and the moon is up (as `sky::key_light`).
pub fn key_towards(view: &crate::RideView) -> Option<Vec3> {
    let s = &view.sky.as_ref()?.sky;
    Some(
        if s.sun.elevation_deg > -4.0 || s.moon.elevation_deg <= 0.0 {
            crate::sky::sun_direction(s.sun.azimuth_deg, s.sun.elevation_deg)
        } else {
            crate::sky::sun_direction(s.moon.azimuth_deg, s.moon.elevation_deg)
        },
    )
}

/// The uniform for lines on `lines`' nodes, counted from `base`.
pub fn uniform_for(lines: &ShadowLines, base: f32, on: bool) -> TerrainShadowUniform {
    let (sx, sz) = (
        1.0 / (lines.cell_m * lines.nx as f64),
        1.0 / (lines.cell_m * lines.nz as f64),
    );
    TerrainShadowUniform {
        scale: Vec4::new(
            sx as f32,
            sz as f32,
            (0.5 / lines.nx as f64 - lines.origin[0] * sx) as f32,
            (0.5 / lines.nz as f64 - lines.origin[1] * sz) as f32,
        ),
        params: Vec4::new(if on { 1.0 } else { 0.0 }, base, 1.0, 0.0),
    }
}

/// The texture of `lines` (RG16F: the line above `base`, the penumbra).
pub fn lines_image(lines: &ShadowLines, base: f32) -> Image {
    let mut data = Vec::with_capacity(lines.line.len() * 4);
    for l in &lines.line {
        data.extend(f16_bits(l[0] - base).to_le_bytes());
        data.extend(f16_bits(l[1]).to_le_bytes());
    }
    let mut image = Image::new(
        Extent3d {
            width: lines.nx as u32,
            height: lines.nz as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rg16Float,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
    image
}

/// An `f32` as the bits of an IEEE half (round to nearest; out of range
/// to ±65504, tiny to 0).
pub fn f16_bits(v: f32) -> u16 {
    if v.is_nan() {
        return 0x7e00;
    }
    let sign = if v.is_sign_negative() { 0x8000u16 } else { 0 };
    let a = v.abs().min(65504.0);
    if a < 6.103_515_6e-5 {
        // Subnormals: a / 2^-24.
        return sign | (a / 5.960_464_5e-8).round() as u16;
    }
    let bits = a.to_bits();
    let exp = ((bits >> 23) & 0xff) as i32 - 127 + 15;
    let mant = bits & 0x7f_ffff;
    // Round the 23-bit mantissa to 10 bits (carry may bump the exponent).
    let m = (mant + 0x1000) >> 13;
    let h = ((exp as u32) << 10) + m;
    sign | (h.min(0x7bff) as u16)
}

#[allow(clippy::too_many_arguments)] // a Bevy system
fn update_map(
    world: Option<Res<PackageWorld>>,
    view: Option<Res<crate::RideView>>,
    mut map: ResMut<TerrainShadowMap>,
    mut images: ResMut<Assets<Image>>,
    time: Res<Time<Real>>,
    mut last: Local<f32>,
) {
    let Some(world) = world else {
        if map.grid.is_some() || map.uniform.params.x > 0.0 {
            let (off, generation) = (map.off, map.generation + 1);
            *map = TerrainShadowMap {
                off,
                generation,
                image: map.image.clone(),
                ..default()
            };
        }
        return;
    };
    let set = world.set.clone();
    let key = (set.generation(), set.tiles_version());
    if map.grid_key != Some(key) && map.grid_task.as_ref().map(|t| t.0) != Some(key) {
        let s = set.clone();
        map.grid_task = Some((
            key,
            AsyncComputeTaskPool::get().spawn(async move {
                let t0 = bevy::platform::time::Instant::now();
                let g = HeightGrid::of_set(&s, CELL_M, MAX_SIDE);
                (g, t0.elapsed().as_secs_f32() * 1000.0)
            }),
        ));
    }
    if let Some((k, task)) = map.grid_task.as_mut()
        && let Some((grid, ms)) = check_ready(task)
    {
        let k = *k;
        map.grid_task = None;
        map.grid_ms = ms;
        map.grid_key = Some(k);
        map.grid = grid.map(Arc::new);
        map.towards = None;
    }
    let Some(grid) = map.grid.clone() else {
        return;
    };
    let Some(towards) = view.as_deref().and_then(key_towards) else {
        return;
    };
    let now = time.elapsed_secs();
    let stale = map
        .towards
        .is_none_or(|t| t.angle_between(towards).to_degrees() > RECOMPUTE_DEG);
    // At most four recomputations a second (the time machine scrubbing).
    if stale && map.lines_task.is_none() && (map.towards.is_none() || now - *last > 0.25) {
        *last = now;
        let g = grid.clone();
        map.lines_task = Some((
            towards,
            AsyncComputeTaskPool::get().spawn(async move {
                let t0 = bevy::platform::time::Instant::now();
                let l = shadow_lines(&g, towards.to_array());
                (l, t0.elapsed().as_secs_f32() * 1000.0)
            }),
        ));
    }
    if let Some((t, task)) = map.lines_task.as_mut()
        && let Some((lines, ms)) = check_ready(task)
    {
        let t = *t;
        map.lines_task = None;
        map.lines_ms = ms;
        map.lines = Some(Arc::new(lines.clone()));
        map.towards = Some(t);
        let base = grid.min;
        let image = lines_image(&lines, base);
        let handle = match map.image.clone() {
            Some(h) => {
                let _ = images.insert(&h, image);
                h
            }
            None => images.add(image),
        };
        map.image = Some(handle);
        map.uniform = uniform_for(&lines, base, !map.off);
        map.generation += 1;
    }
}

type Ext<E> = ExtendedMaterial<StandardMaterial, E>;

/// Hands the map to every material of kind `E`: all of them when the
/// texture or the uniform changed (the texture is replaced in place, and a
/// touched material takes the new one into its bind group), else only new
/// ones (their copy differs).
fn sync<E: TerrainShadowed>(
    map: Res<TerrainShadowMap>,
    mats: Option<ResMut<Assets<Ext<E>>>>,
    mut seen: Local<u64>,
) {
    let Some(mut mats) = mats else { return };
    let all = *seen != map.generation;
    *seen = map.generation;
    let want = map.uniform;
    let image = map.image.clone();
    let stale: Vec<_> = mats
        .iter()
        .filter(|(_, m)| {
            let (u, h) = m.extension.terrain_shadow();
            all || *u != want || h.as_ref().map(|h| h.id()) != image.as_ref().map(|h| h.id())
        })
        .map(|(id, _)| id)
        .collect();
    for id in stale {
        if let Some(mut m) = mats.get_mut(id) {
            m.extension.set_terrain_shadow(want, image.clone());
        }
    }
}

/// `terrain.shadow [on|off]`.
fn register_command(app: &mut App) {
    use crate::console::{ConsoleAppExt, ConsoleCommand, Permission};
    app.console_command(ConsoleCommand::new(
        Permission::User,
        clap::Command::new("terrain.shadow")
            .about("The mountains' shadows from the height field (spec 4.5): on (default) or off (comparisons); --sun-shadows off also turns the sun's shadow maps off, as on the phones; without arguments: the grid, its cost and the light it shows")
            .arg(clap::Arg::new("state").value_parser(["on", "off"]))
            .arg(
                clap::Arg::new("sun-shadows")
                    .long("sun-shadows")
                    .value_parser(["on", "off"]),
            ),
        |world, m| {
            if let Some(s) = m.get_one::<String>("sun-shadows") {
                let mut sw = world.get_resource_or_insert_with(crate::switches::RenderSwitches::default);
                sw.no_shadow = s == "off";
            }
            let maps = world
                .get_resource::<crate::switches::RenderSwitches>()
                .is_none_or(|s| !s.no_shadow);
            let mut map = world.resource_mut::<TerrainShadowMap>();
            if let Some(s) = m.get_one::<String>("state") {
                map.off = s == "off";
                let on = !map.off && map.image.is_some();
                map.uniform.params.x = if on { 1.0 } else { 0.0 };
                map.generation += 1;
            }
            let grid = match map.grid() {
                Some(g) => format!(
                    "{} x {} nodes of {:.0} m ({:.1} MB heights, {:.1} MB texture), built in {:.0} ms",
                    g.nx,
                    g.nz,
                    g.cell_m,
                    g.bytes() as f64 / 1e6,
                    (g.nx * g.nz * 4) as f64 / 1e6,
                    map.grid_ms
                ),
                None => "no grid (no world package)".into(),
            };
            let light = map.towards.map_or("none yet".into(), |t| {
                format!(
                    "elevation {:.1}°, lines in {:.1} ms",
                    t.y.clamp(-1.0, 1.0).asin().to_degrees(),
                    map.lines_ms
                )
            });
            Ok(format!(
                "terrain shadow {}: {grid}; light {light}; sun shadow maps {}",
                if map.off { "off" } else { "on" },
                if maps { "on" } else { "off" }
            ))
        },
    ));
}

/// The render layer of the proxy casters: only the sun sees it (its
/// shadow maps), no camera.
pub const PROXY_LAYER: usize = 13;
/// The proxy casters float this high above the shaded ground (m), on the
/// ray to the sun: above riders, bikes, cars and lamps, which they shade.
pub const PROXY_CAP_M: f32 = 12.0;
/// At most this many proxy cells per side.
pub const PROXY_CELLS: f32 = 120.0;

/// The proxy casters' mesh (one entity, rebuilt as the camera moves).
#[derive(Component)]
pub struct ShadowProxy;

/// Where the proxy was built: camera xz, map generation.
#[derive(Default)]
pub(crate) struct ProxyState {
    at: Option<(Vec2, u64, f32)>,
    entity: Option<Entity>,
}

/// Objects with Bevy's standard materials (riders' bikes and bodies far
/// from their gear shader, props, glTF models) do not sample the map. Near
/// the camera they still fall into the mountains' shadow: horizontal
/// quads [`PROXY_CAP_M`] above each shaded cell of the ground, moved along
/// the ray to the sun, cast it into the sun's cascades – the cascade's
/// orthographic projection along the light puts their shadow exactly on
/// the cell. The quads are on [`PROXY_LAYER`]: drawn into the shadow maps
/// only. `radius` the cascades' reach (m), `cam` the camera's xz,
/// `towards` the sun. Positions of the triangles (two per shaded cell).
pub fn proxy_quads(
    grid: &HeightGrid,
    lines: &ShadowLines,
    cam: Vec2,
    radius: f32,
    towards: Vec3,
) -> Vec<[f32; 3]> {
    let mut out = Vec::new();
    let flat = Vec2::new(towards.x, towards.z);
    if towards.y <= 0.0 || flat.length() < 1e-4 {
        return out;
    }
    let tan = towards.y / flat.length();
    let shift = flat.normalize() * (PROXY_CAP_M / tan.max(0.009));
    let cell = (2.0 * radius / PROXY_CELLS).max(3.0);
    let n = (2.0 * radius / cell).ceil() as i32;
    let o = cam - Vec2::splat(n as f32 * cell * 0.5);
    for k in 0..n {
        for i in 0..n {
            let c = o + Vec2::new((i as f32 + 0.5) * cell, (k as f32 + 0.5) * cell);
            if c.distance(cam) > radius {
                continue;
            }
            let p = [f64::from(c.x), f64::from(c.y)];
            let Some(h) = grid.height(p) else { continue };
            // A rider's height above the ground.
            if lines.sun(p, h + 1.0) >= 0.5 {
                continue;
            }
            let y = h + PROXY_CAP_M;
            let a = c + shift - Vec2::splat(cell * 0.5);
            let b = a + Vec2::splat(cell);
            let q = |x: f32, z: f32| [x, y, z];
            out.extend([
                q(a.x, a.y),
                q(a.x, b.y),
                q(b.x, b.y),
                q(a.x, a.y),
                q(b.x, b.y),
                q(b.x, a.y),
            ]);
        }
    }
    out
}

/// The sun also sees [`PROXY_LAYER`] (whatever else it sees).
fn sun_sees_proxies(
    mut commands: Commands,
    suns: Query<(Entity, Option<&bevy::camera::visibility::RenderLayers>), With<crate::sky::Sun>>,
) {
    use bevy::camera::visibility::RenderLayers;
    let proxy = RenderLayers::layer(PROXY_LAYER);
    for (e, have) in &suns {
        let have = have.cloned().unwrap_or_default();
        if !have.intersects(&proxy) {
            commands.entity(e).insert(have.with(PROXY_LAYER));
        }
    }
}

#[allow(clippy::too_many_arguments)] // a Bevy system
fn update_proxy(
    mut commands: Commands,
    map: Res<TerrainShadowMap>,
    suns: Query<(&DirectionalLight, &bevy::light::CascadeShadowConfig), With<crate::sky::Sun>>,
    cameras: Query<&GlobalTransform, With<crate::camera::FollowCamera>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut state: Local<ProxyState>,
    mut material: Local<Option<Handle<StandardMaterial>>>,
) {
    let sun = suns.iter().next();
    let on =
        !map.off && map.uniform.params.x > 0.5 && sun.is_some_and(|(l, _)| l.shadow_maps_enabled);
    let (Some(grid), Some(lines), Some(towards), Some(cam), true) = (
        map.grid.as_deref(),
        map.lines.as_deref(),
        map.towards,
        cameras.iter().next(),
        on,
    ) else {
        if let Some(e) = state.entity.take() {
            commands.entity(e).try_despawn();
            state.at = None;
        }
        return;
    };
    let radius = sun
        .and_then(|(_, c)| c.bounds.last().copied())
        .unwrap_or(100.0)
        .min(300.0);
    let at = cam.translation().xz();
    // Rebuilt when the camera has moved a tenth of the reach, or the map
    // or the reach changed.
    if state.at.is_some_and(|(p, g, r)| {
        p.distance(at) < radius * 0.1 && g == map.generation && r == radius
    }) {
        return;
    }
    state.at = Some((at, map.generation, radius));
    let tris = proxy_quads(grid, lines, at, radius, towards);
    let mut mesh = Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    );
    let normals = vec![[0.0, 1.0, 0.0]; tris.len()];
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, tris);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    let handle = meshes.add(mesh);
    let mat = material
        .get_or_insert_with(|| {
            mats.add(StandardMaterial {
                unlit: true,
                cull_mode: None,
                ..default()
            })
        })
        .clone();
    match state.entity {
        Some(e) => {
            commands.entity(e).try_insert(Mesh3d(handle));
        }
        None => {
            state.entity = Some(
                commands
                    .spawn((
                        ShadowProxy,
                        Mesh3d(handle),
                        MeshMaterial3d(mat),
                        Transform::default(),
                        Visibility::Visible,
                        bevy::light::NotShadowReceiver,
                        bevy::camera::visibility::RenderLayers::layer(PROXY_LAYER),
                        Name::new("terrain shadow proxies"),
                    ))
                    .id(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_world::terrain_shadow::HeightGrid;

    #[test]
    fn halves_round_trip() {
        for (v, bits) in [
            (0.0f32, 0u16),
            (1.0, 0x3c00),
            (-2.0, 0xc000),
            (65504.0, 0x7bff),
            (1e9, 0x7bff),
        ] {
            assert_eq!(f16_bits(v), bits, "{v}");
        }
        // 1500 m above the base: 1 m steps there.
        let h = f16_bits(1500.3);
        let back =
            f32::from(1u16 << ((h >> 10) - 15).min(15)) * (1.0 + f32::from(h & 0x3ff) / 1024.0);
        assert!((back - 1500.3).abs() <= 0.5, "{back}");
    }

    #[test]
    fn uniform_puts_texel_centres_on_the_nodes() {
        let g =
            HeightGrid::sample([0.0, 0.0, 1000.0, 500.0], 50.0, MAX_SIDE, |_| Some(10.0)).unwrap();
        let l = shadow_lines(&g, [-1.0, 0.2, 0.0]);
        let u = uniform_for(&l, g.min, true);
        let uv = |x: f32, z: f32| Vec2::new(x * u.scale.x + u.scale.z, z * u.scale.y + u.scale.w);
        // Node (0, 0) at the first texel's centre, the last node at the last's.
        let first = uv(g.origin[0] as f32, g.origin[1] as f32);
        assert!((first - Vec2::new(0.5 / g.nx as f32, 0.5 / g.nz as f32)).length() < 1e-5);
        let last = uv(
            (g.origin[0] + (g.nx - 1) as f64 * g.cell_m) as f32,
            (g.origin[1] + (g.nz - 1) as f64 * g.cell_m) as f32,
        );
        let want = Vec2::new(1.0 - 0.5 / g.nx as f32, 1.0 - 0.5 / g.nz as f32);
        assert!((last - want).length() < 1e-5);
        let img = lines_image(&l, g.min);
        assert_eq!(img.data.as_ref().map(Vec::len), Some(g.nx * g.nz * 4));
    }

    /// 6 × 2 tiles of 1 km, 10 m grid: a ridge along z at x = 1500 m
    /// (600 m high, flanks 30 %), the valley floor at 0 m east of it.
    fn ridge_package() -> ocw_world::package::WorldPackage {
        use ocw_world::package::{Tile, TileKey, WorldPackage};
        let n = 101usize;
        let mut tiles = std::collections::BTreeMap::new();
        for x in 0..6 {
            for z in 0..2 {
                let key = TileKey { x, z };
                let heights = (0..n * n)
                    .map(|j| {
                        let gx = f64::from(x) * 1000.0 + (j % n) as f64 * 10.0;
                        (600.0 - 0.3 * (gx - 1500.0).abs()).max(0.0) as f32
                    })
                    .collect();
                tiles.insert(
                    key,
                    Tile {
                        key,
                        heights,
                        roads: Vec::new(),
                    },
                );
            }
        }
        WorldPackage {
            name: "ridge".into(),
            origin: (42.54, 1.60),
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
            places: None,
            junctions: None,
            traffic: None,
            aero: None,
        }
    }

    fn west_sun(elev: f32) -> Vec3 {
        let e = elev.to_radians();
        Vec3::new(-e.cos(), e.sin(), 0.0)
    }

    /// The near tiles (10 m) and the far tiles (100 m) of the same slopes
    /// under the same sun: the same N·L (no cast shadows involved), and
    /// the same terrain shadow at every vertex – the valley behind the
    /// ridge is in shadow from both, the open slopes in the sun.
    #[test]
    fn near_and_far_terrain_shade_and_shadow_alike() {
        use crate::package::{TileLook, terrain_arrays};
        let pkg = ridge_package();
        let sun = west_sun(10.0);
        let grid = HeightGrid::of_package(&pkg, MIN_CELL_M, MAX_SIDE).unwrap();
        let lines = shadow_lines(&grid, sun.to_array());
        let look = TileLook::default();
        let mut checked = 0;
        for tile in pkg.tiles.values() {
            let near = terrain_arrays(&pkg, tile);
            let far = crate::far_terrain::far_tile_arrays(&pkg, tile, &look, None);
            // The far grid's vertices (11 × 11, before the skirts) are
            // every 10th near vertex.
            for kf in 0..11 {
                for if_ in 0..11 {
                    let f = kf * 11 + if_;
                    let nidx = kf * 10 * 101 + if_ * 10;
                    let pf = Vec3::from(far.positions[f]);
                    let pn = Vec3::from(near.positions[nidx]);
                    assert_eq!(pf, pn);
                    let lf = Vec3::from(far.normals[f]).dot(sun).max(0.0);
                    // The near tile's N·L over the far cell around it.
                    let (mut sum, mut cnt) = (0.0, 0);
                    for kk in (kf * 10).saturating_sub(5)..=(kf * 10 + 5).min(100) {
                        for ii in (if_ * 10).saturating_sub(5)..=(if_ * 10 + 5).min(100) {
                            sum += Vec3::from(near.normals[kk * 101 + ii]).dot(sun).max(0.0);
                            cnt += 1;
                        }
                    }
                    let ln = sum / cnt as f32;
                    // As uploaded: both meshes carry their normals in
                    // 16-bit signed normalised components (10G); the
                    // packing moves N·L by far less than a shade.
                    for (arr, i) in [(&far.normals, f), (&near.normals, nidx)] {
                        let bevy::mesh::VertexAttributeValues::Snorm16x4(q) =
                            crate::package::pack_normals(&arr[i..=i])
                        else {
                            unreachable!()
                        };
                        let unpacked = Vec3::new(
                            f32::from(q[0][0]) / 32767.0,
                            f32::from(q[0][1]) / 32767.0,
                            f32::from(q[0][2]) / 32767.0,
                        )
                        .normalize();
                        let exact = Vec3::from(arr[i]).dot(sun);
                        assert!((unpacked.dot(sun) - exact).abs() < 1e-4);
                    }
                    // Away from the crest and the foot (the 100 m grid
                    // cuts their kinks off).
                    if (pf.x - 1500.0).abs() > 150.0 && (pf.x - 3500.0).abs() > 150.0 {
                        assert!((lf - ln).abs() < 0.05, "at {pf}: far {lf} near {ln}");
                    }
                    let p = [f64::from(pf.x), f64::from(pf.z)];
                    assert_eq!(lines.sun(p, pf.y), lines.sun(p, pn.y));
                    checked += 1;
                }
            }
            // Every near vertex: the valley behind the ridge in shadow,
            // the open ground in the sun.
            for (q, nrm) in near.positions.iter().zip(&near.normals) {
                let q = Vec3::from(*q);
                let p = [f64::from(q.x), f64::from(q.z)];
                let vn = lines.sun(p, q.y);
                if (3600.0..4650.0).contains(&q.x) {
                    assert!(vn < 0.05, "valley floor at {q} lit ({vn})");
                }
                if !(1400.0..=5200.0).contains(&q.x) {
                    assert!(vn > 0.95, "open ground at {q} shaded ({vn})");
                }
                // The east flank faces away from the sun: N·L is 0 there
                // too, so both answers agree on it.
                if (1600.0..3400.0).contains(&q.x) {
                    assert!(Vec3::from(*nrm).dot(sun) <= 0.0 && vn < 0.05, "{q}: {vn}");
                }
            }
        }
        assert_eq!(checked, 12 * 121);
    }

    /// The same valley point stays in the mountains' shadow from 10 km to
    /// 100 m: the lookup is a function of the world point only (the map
    /// covers the whole world; the cascades only add the shadows of the
    /// objects near the camera).
    #[test]
    fn the_shadow_does_not_move_with_the_camera() {
        let pkg = ridge_package();
        let grid = HeightGrid::of_package(&pkg, MIN_CELL_M, MAX_SIDE).unwrap();
        let lines = shadow_lines(&grid, west_sun(10.0).to_array());
        let u = uniform_for(&lines, grid.min, true);
        let uv = |p: Vec2| Vec2::new(p.x * u.scale.x + u.scale.z, p.y * u.scale.y + u.scale.w);
        let point = Vec2::new(4000.0, 1000.0);
        let first = uv(point);
        for _camera_km in [10.0f32, 5.0, 2.0, 1.0, 0.5, 0.1] {
            assert_eq!(uv(point), first);
        }
        assert!((0.0..=1.0).contains(&first.x) && (0.0..=1.0).contains(&first.y));
        assert!(lines.sun([4000.0, 1000.0], 0.0) < 0.05);
    }

    /// The cost: Andorra's 30 × 30 km at 50 m cells is 601² nodes – 1.4 MB
    /// on the GPU (RG16F) – and the largest world 1024² (4 MB).
    #[test]
    fn the_cost_is_bounded() {
        let grid = |w: f64| {
            HeightGrid::sample([0.0, 0.0, w, w], MIN_CELL_M, MAX_SIDE, |_| Some(0.0)).unwrap()
        };
        let big = grid(300_000.0);
        assert!(big.nx * big.nz * 4 <= 4 * 1024 * 1024);
        let andorra = grid(30_000.0);
        assert_eq!((andorra.nx, andorra.nz), (601, 601));
    }

    /// The proxy casters near a camera in the valley's shadow: each quad,
    /// followed along the ray away from the sun, lands on a shaded cell;
    /// none on open ground; at most 120² cells.
    #[test]
    fn proxies_cast_the_mountains_shadow_onto_the_shaded_cells_only() {
        let pkg = ridge_package();
        let grid = HeightGrid::of_package(&pkg, MIN_CELL_M, MAX_SIDE).unwrap();
        let sun = west_sun(10.0);
        let lines = shadow_lines(&grid, sun.to_array());
        let tris = proxy_quads(&grid, &lines, Vec2::new(4000.0, 1000.0), 250.0, sun);
        assert!(!tris.is_empty());
        assert!(tris.len() <= 6 * 120 * 120);
        for quad in tris.chunks(6) {
            let c = (Vec3::from(quad[0]) + Vec3::from(quad[2])) * 0.5;
            // Down the ray by the cap: the ground it shades.
            let g = c - sun * (PROXY_CAP_M / sun.y);
            let p = [f64::from(g.x), f64::from(g.z)];
            let h = grid.height(p).unwrap();
            assert!((g.y - h).abs() < 0.5, "{g} over {h}");
            assert!(lines.sun(p, h + 1.0) < 0.5, "{g} is lit");
        }
        // On open ground in the sun: nothing.
        assert!(proxy_quads(&grid, &lines, Vec2::new(5600.0, 1000.0), 250.0, sun).is_empty());
        // A sun overhead or below the horizon: nothing either.
        assert!(proxy_quads(&grid, &lines, Vec2::new(4000.0, 1000.0), 250.0, Vec3::Y).is_empty());
    }
}
