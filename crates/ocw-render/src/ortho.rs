//! The orthophoto corridor on the ground (spec §14 Phase 8 T7, §4.6):
//! the package's `ORTC` section ([`ocw_world::ortho`]) blended into the
//! terrain shader near the roads, so the roadside looks like the real
//! place – fields, gardens, driveways – while the tiling materials keep
//! the detail up close.
//!
//! - **Texture per tile** ([`tile_texture`]): the tile's JPEG chunks
//!   decoded into one sRGB image at the corridor's pixel size (2000² at
//!   0.5 m), the corridor weight ([`ocw_world::ortho::corridor_weights`],
//!   stored at 4 m corners) bilinearly in its alpha (0 in chunks the
//!   package does not have), a mip chain averaged in linear light, BC3
//!   compressed (4 MB + mips per tile instead of 21 MB). Built in the
//!   background only for the tiles within [`ORTHO_RING`] of the rider: the
//!   photo fades out before the next ring ([`OrthoLook`]: 650 – 950 m).
//! - **In the shader** (`ground.wesl`, [`blend`] and [`apply`] are its CPU
//!   twins): the photo gives the colour, the materials the detail – the
//!   photo times the materials' brightness relative to the land-use colour
//!   they stand for ([`apply`]). Up close the colour comes from the blurred photo
//!   (a few metres: no magnified 0.5 m pixels next to the rider, the
//!   materials keep all their detail), from [`OrthoLook::sharp_from`] to
//!   [`OrthoLook::sharp_full`] the sharp photo takes over (fields, hedges,
//!   driveways, parked cars); where the materials have faded to the colour
//!   map (beyond 350 m) the result is the photo itself. All of it times the
//!   corridor weight and the view-distance fade. Shadows and cars are toned
//!   down by the world builder (`ocw_worldgen::corridor::tone`); first
//!   renders with the photo replacing the materials from 70 m looked soft
//!   (0.5 m pixels magnified ten times at a raised camera).
//! - **Setting** `display.orthophoto on|off` (console `ortho on|off`,
//!   default on): off drops the textures.

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};
use bevy::tasks::{AsyncComputeTaskPool, Task, futures::check_ready};
use clap::{Arg, Command};
use ocw_world::TileKey;
use ocw_world::ortho::OrthoCorridor;

use crate::console::{ConsoleAppExt, ConsoleCommand, Permission};
use crate::ground::{TerrainLook, TerrainMaterial};
use crate::materials::MaterialLibrary;
use crate::package::{PackageWorld, TileId, TileStreamer};

/// Tiles up to this ring around the rider's tile get the photo.
pub const ORTHO_RING: i32 = 1;

/// The blend parameters (uniform `ortho` and `ortho_blend` of
/// `ground.wesl`, see [`TerrainLook`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OrthoLook {
    /// The sharp photo starts to replace the blurred one in the tint here (m).
    pub sharp_from: f32,
    /// … and has its full share here (m).
    pub sharp_full: f32,
    /// The sharp photo's share at and beyond `sharp_full`.
    pub sharp_share: f32,
    /// How much the materials are tinted towards the photo.
    pub tint: f32,
    /// The photo fades out with the view distance between these (m).
    pub fade: (f32, f32),
}

impl Default for OrthoLook {
    fn default() -> Self {
        Self {
            sharp_from: 15.0,
            sharp_full: 120.0,
            sharp_share: 1.0,
            tint: 1.0,
            fade: (650.0, 950.0),
        }
    }
}

impl OrthoLook {
    /// The two uniform vectors: `ortho` (x: on, y: pixel size, z, w: the
    /// fade) and `ortho_blend` (x, y: sharp from / full, z: share, w: tint).
    pub fn uniforms(&self, on: bool, pixel_m: f32) -> (Vec4, Vec4) {
        (
            Vec4::new(f32::from(u8::from(on)), pixel_m, self.fade.0, self.fade.1),
            Vec4::new(
                self.sharp_from,
                self.sharp_full,
                self.sharp_share,
                self.tint,
            ),
        )
    }
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How the photo is blended at a point (CPU twin of `ground.wesl`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Blend {
    /// How much of the result is the photo blend at all (corridor weight ×
    /// view fade).
    pub weight: f32,
    /// The share of the sharp photo in the tint (the rest: the blurred).
    pub sharp: f32,
    /// How strongly the materials are tinted.
    pub tint: f32,
}

/// The blend at `dist` metres from the camera where the corridor weight
/// (the texture's alpha) is `corridor`.
pub fn blend(dist: f32, corridor: f32, look: &TerrainLook) -> Blend {
    if look.ortho.x < 0.5 {
        return Blend {
            weight: 0.0,
            sharp: 0.0,
            tint: 0.0,
        };
    }
    let fade = 1.0 - smoothstep(look.ortho.z, look.ortho.w, dist);
    let b = look.ortho_blend;
    Blend {
        weight: corridor.clamp(0.0, 1.0) * fade,
        sharp: smoothstep(b.x, b.y, dist) * b.z,
        tint: b.w,
    }
}

/// The range of the materials' brightness detail laid over the photo.
pub const DETAIL: (f32, f32) = (0.4, 2.2);

/// The ground albedo (linear) with the photo blended in: the photo's
/// colour with the materials' brightness detail (their luminance over the
/// colour map's: a per-channel ratio gave colour casts where a material's
/// hue differs from its class colour – purple lawns in the first renders).
/// `material` is the
/// textured albedo, `colour_map` the land-use colour the materials stand
/// for, `photo` the photo, `photo_low` the blurred photo (CPU twin of
/// `ground.wesl`).
pub fn apply(
    b: Blend,
    material: [f32; 3],
    colour_map: [f32; 3],
    photo: [f32; 3],
    photo_low: [f32; 3],
) -> [f32; 3] {
    let lum = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let detail = (lum(material) / lum(colour_map).max(0.01)).clamp(DETAIL.0, DETAIL.1);
    std::array::from_fn(|i| {
        let p = photo_low[i] + (photo[i] - photo_low[i]) * b.sharp;
        let tinted = p * detail;
        material[i] + (tinted - material[i]) * b.weight * b.tint
    })
}

/// A tile's corridor texture: RGBA8 sRGB levels (alpha = corridor weight).
pub struct OrthoTexture {
    pub size: u32,
    pub levels: Vec<Vec<u8>>,
}

fn lin_lut() -> [f32; 256] {
    std::array::from_fn(|i| {
        let c = i as f32 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    })
}

fn to_srgb8(l: f32) -> u8 {
    let c = if l <= 0.003_130_8 {
        l * 12.92
    } else {
        1.055 * l.powf(1.0 / 2.4) - 0.055
    };
    (c * 255.0).round().clamp(0.0, 255.0) as u8
}

/// The next mip level: 2 × 2 means, colour in linear light (odd sizes
/// round down, the last row and column fold in).
pub fn half_level(src: &[u8], size: u32, lut: &[f32; 256]) -> (Vec<u8>, u32) {
    let n = size as usize;
    let h = (n / 2).max(1);
    let mut out = vec![0u8; h * h * 4];
    for z in 0..h {
        for x in 0..h {
            let mut acc = [0.0f32; 4];
            for (dx, dz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (sx, sz) = ((2 * x + dx).min(n - 1), (2 * z + dz).min(n - 1));
                let p = &src[(sz * n + sx) * 4..(sz * n + sx) * 4 + 4];
                for k in 0..3 {
                    acc[k] += lut[usize::from(p[k])];
                }
                acc[3] += f32::from(p[3]);
            }
            let o = &mut out[(z * h + x) * 4..(z * h + x) * 4 + 4];
            for k in 0..3 {
                o[k] = to_srgb8(acc[k] / 4.0);
            }
            o[3] = (acc[3] / 4.0).round() as u8;
        }
    }
    (out, h as u32)
}

/// Decodes tile `key` of the corridor into its texture (RGBA8 levels down
/// to 8 px): `None` without the tile.
pub fn tile_texture(
    c: &OrthoCorridor,
    key: TileKey,
    tile_size: f32,
) -> Option<Result<OrthoTexture, String>> {
    let tile = c.tiles.get(&key)?;
    Some((|| {
        let weights = c.weights(key).expect("tile exists")?;
        let corners = usize::from(c.weight_cells) + 1;
        let cpx = c.chunk_px(tile_size) as usize;
        let n = c.tile_px(tile_size) as usize;
        let chunks = usize::from(c.chunks);
        let mut rgba = vec![0u8; n * n * 4];
        let mut mean = [0u64; 3];
        let mut count = 0u64;
        let mut have = vec![false; chunks * chunks];
        for (&(cx, cz), bytes) in &tile.chunks {
            let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Jpeg)
                .map_err(|e| format!("orthophoto chunk {cx} {cz}: {e}"))?
                .to_rgb8();
            if img.dimensions() != (cpx as u32, cpx as u32) {
                return Err(format!("orthophoto chunk {cx} {cz}: wrong size"));
            }
            let (cx, cz) = (usize::from(cx), usize::from(cz));
            have[cz * chunks + cx] = true;
            for (i, p) in img.pixels().enumerate() {
                let (x, z) = (cx * cpx + i % cpx, cz * cpx + i / cpx);
                let o = (z * n + x) * 4;
                rgba[o..o + 3].copy_from_slice(&p.0);
                for (m, c) in mean.iter_mut().zip(p.0) {
                    *m += u64::from(c);
                }
                count += 1;
            }
        }
        let fill = mean.map(|m| (m / count.max(1)) as u8);
        // Weight cells per pixel.
        let per = (n - 1) as f32 / (corners - 1) as f32;
        for z in 0..n {
            for x in 0..n {
                let o = (z * n + x) * 4;
                if !have[(z / cpx) * chunks + x / cpx] {
                    rgba[o..o + 3].copy_from_slice(&fill);
                    rgba[o + 3] = 0;
                    continue;
                }
                let fx = ((x as f32 + 0.5) / per).min((corners - 1) as f32);
                let fz = ((z as f32 + 0.5) / per).min((corners - 1) as f32);
                let (x0, z0) = (fx as usize, fz as usize);
                let (x1, z1) = ((x0 + 1).min(corners - 1), (z0 + 1).min(corners - 1));
                let (tx, tz) = (fx - x0 as f32, fz - z0 as f32);
                let w = |c: usize, r: usize| f32::from(weights[r * corners + c]);
                let a = w(x0, z0) * (1.0 - tx) + w(x1, z0) * tx;
                let b = w(x0, z1) * (1.0 - tx) + w(x1, z1) * tx;
                rgba[o + 3] = (a * (1.0 - tz) + b * tz).round() as u8;
            }
        }
        let lut = lin_lut();
        let mut levels = vec![rgba];
        let mut size = n as u32;
        while size / 2 >= 8 {
            let (next, s) = half_level(levels.last().expect("one level"), size, &lut);
            levels.push(next);
            size = s;
        }
        Ok(OrthoTexture {
            size: n as u32,
            levels,
        })
    })())
}

/// BC3 (sRGB colour, linear alpha) of every level.
pub fn compress_bc3(t: &OrthoTexture) -> Vec<Vec<u8>> {
    let mut size = t.size as usize;
    t.levels
        .iter()
        .map(|l| {
            let f = texpresso::Format::Bc3;
            let mut out = vec![0u8; f.compressed_size(size, size)];
            let params = texpresso::Params {
                algorithm: texpresso::Algorithm::RangeFit,
                ..Default::default()
            };
            f.compress(l, size, size, params, &mut out);
            size = (size / 2).max(1);
            out
        })
        .collect()
}

/// The GPU image of a tile's corridor.
pub fn ortho_image(t: &OrthoTexture) -> Image {
    let levels = compress_bc3(t);
    Image {
        data: Some(levels.concat()),
        texture_descriptor: bevy::render::render_resource::TextureDescriptor {
            label: Some("orthophoto corridor"),
            size: Extent3d {
                width: t.size,
                height: t.size,
                depth_or_array_layers: 1,
            },
            mip_level_count: levels.len() as u32,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Bc3RgbaUnormSrgb,
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

/// Whether the photo is drawn (`display.orthophoto`).
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct OrthoSettings {
    pub enabled: bool,
    pub look: OrthoLook,
}

impl Default for OrthoSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            look: OrthoLook::default(),
        }
    }
}

/// The corridor textures of the tiles near the rider.
#[derive(Resource, Default)]
pub struct OrthoStreamer {
    images: HashMap<TileId, (Handle<Image>, f32)>,
    building: HashMap<TileId, Task<Option<(Image, f32)>>>,
    generation: u64,
}

impl OrthoStreamer {
    /// Tiles with a corridor texture.
    pub fn loaded(&self) -> usize {
        self.images.len()
    }
}

pub struct OrthoPlugin;

impl Plugin for OrthoPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<OrthoSettings>()
            .init_resource::<OrthoStreamer>()
            .add_systems(
                Update,
                stream_ortho
                    .after(crate::package::upload_tiles)
                    .run_if(resource_exists::<PackageWorld>),
            )
            .console_command(ConsoleCommand::new(
                Permission::User,
                Command::new("ortho")
                    .about("Orthophoto corridor near the roads: on or off (setting display.orthophoto)")
                    .arg(Arg::new("mode").value_parser(["on", "off"])),
                |world, m| {
                    if let Some(v) = m.get_one::<String>("mode") {
                        world.resource_mut::<OrthoSettings>().enabled = v == "on";
                    }
                    let on = world.resource::<OrthoSettings>().enabled;
                    let tiles = world
                        .get_resource::<PackageWorld>()
                        .and_then(|w| w.home().look.ortho.as_ref().map(|c| c.tiles.len()));
                    let loaded = world.resource::<OrthoStreamer>().loaded();
                    Ok(match tiles {
                        Some(n) => format!(
                            "orthophoto {} ({n} tiles with the corridor, {loaded} drawn)",
                            if on { "on" } else { "off" }
                        ),
                        None => format!(
                            "orthophoto {} (this package has no corridor)",
                            if on { "on" } else { "off" }
                        ),
                    })
                },
            ));
    }
}

/// Builds the corridor textures of the tiles near the rider in the
/// background, drops the others and puts them on the tiles' terrain
/// materials.
#[allow(clippy::too_many_arguments)]
fn stream_ortho(
    world: Res<PackageWorld>,
    view: Res<crate::RideView>,
    tiles: Res<TileStreamer>,
    mut streamer: ResMut<OrthoStreamer>,
    settings: Res<OrthoSettings>,
    library: Option<Res<MaterialLibrary>>,
    mut images: ResMut<Assets<Image>>,
    mut terrain: ResMut<Assets<TerrainMaterial>>,
) {
    let set = &world.set;
    if streamer.generation != set.generation() {
        streamer.generation = set.generation();
        streamer.images.clear();
        streamer.building.clear();
    }
    let on = settings.enabled && library.is_some();
    let focus = crate::flight_world::stream_focus(&view).map(|f| f.centre);
    // The loaded tiles that want the photo.
    let wanted: Vec<TileId> = match (on, focus) {
        (true, Some(p)) => tiles
            .loaded
            .keys()
            .copied()
            .filter(|&(i, key)| {
                let m = &set.members()[i];
                m.package().is_some_and(|pkg| {
                    let centre = TileKey::containing(m.frame.invert(p), f64::from(pkg.tile_size_m));
                    key.ring(centre) <= ORTHO_RING
                        && pkg
                            .look
                            .ortho
                            .as_ref()
                            .is_some_and(|c| c.tiles.contains_key(&key))
                })
            })
            .collect(),
        _ => Vec::new(),
    };
    streamer.images.retain(|id, _| wanted.contains(id));
    streamer.building.retain(|id, _| wanted.contains(id));
    for &id in &wanted {
        if streamer.images.contains_key(&id) || streamer.building.contains_key(&id) {
            continue;
        }
        let set = set.clone();
        let task = AsyncComputeTaskPool::get().spawn(async move {
            let pkg = set.members()[id.0].package()?.clone();
            let c = pkg.look.ortho.as_ref()?;
            match tile_texture(c, id.1, pkg.tile_size_m)? {
                Ok(t) => Some((ortho_image(&t), c.pixel_m)),
                Err(e) => {
                    warn!("orthophoto of tile {} {}: {e}", id.1.x, id.1.z);
                    None
                }
            }
        });
        streamer.building.insert(id, task);
    }
    let ids: Vec<TileId> = streamer.building.keys().copied().collect();
    for id in ids {
        let task = streamer.building.get_mut(&id).expect("key exists");
        if let Some(result) = check_ready(task) {
            streamer.building.remove(&id);
            if let Some((img, px)) = result {
                streamer.images.insert(id, (images.add(img), px));
            }
        }
    }
    // Put them on the materials (and take them off the others).
    for (id, handle) in &tiles.terrain {
        let want = streamer.images.get(id);
        let Some(mat) = terrain.get(handle) else {
            continue;
        };
        if mat.extension.ortho.as_ref() == want.map(|w| &w.0) {
            continue;
        }
        let Some(mut mat) = terrain.get_mut(handle) else {
            continue;
        };
        let e = &mut mat.extension;
        let (a, b) = settings
            .look
            .uniforms(want.is_some(), want.map_or(0.5, |w| w.1));
        e.look.ortho = a;
        e.look.ortho_blend = b;
        e.ortho = want.map(|w| w.0.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_world::ortho::{OrthoEncoding, OrthoTile, encode_weights};
    use std::collections::BTreeMap;

    fn look_on() -> TerrainLook {
        let mut l = TerrainLook::default();
        let (a, b) = OrthoLook::default().uniforms(true, 0.5);
        l.ortho = a;
        l.ortho_blend = b;
        l
    }

    /// Blend weights: the blurred photo up close, the sharp one from mid
    /// distance, nothing beyond the fade or outside the corridor, and off
    /// by default.
    #[test]
    fn blend_weights_follow_the_view_distance_and_the_corridor() {
        let off = blend(50.0, 1.0, &TerrainLook::default());
        assert_eq!(off.weight, 0.0);
        let l = look_on();
        let near = blend(3.0, 1.0, &l);
        assert_eq!(near.weight, 1.0);
        assert_eq!(near.sharp, 0.0);
        assert!(near.tint > 0.5);
        let mid = blend(150.0, 1.0, &l);
        assert_eq!(mid.weight, 1.0);
        assert!((mid.sharp - OrthoLook::default().sharp_share).abs() < 1e-6);
        let half = blend(60.0, 1.0, &l);
        assert!(half.sharp > 0.1 && half.sharp < mid.sharp);
        assert_eq!(blend(2000.0, 1.0, &l).weight, 0.0);
        assert!(blend(800.0, 1.0, &l).weight < 1.0);
        assert_eq!(blend(150.0, 0.0, &l).weight, 0.0);
        assert!((blend(150.0, 0.5, &l).weight - 0.5).abs() < 1e-6);
        // The fade ends before the next ring of tiles (which has no photo).
        assert!(OrthoLook::default().fade.1 < 1000.0 * ORTHO_RING as f32);
    }

    /// Up close the material keeps its detail (a bright and a dark texel
    /// stay apart) but takes the blurred photo's colour; far off, where the
    /// materials have faded to the colour map, it is the sharp photo.
    #[test]
    fn the_photo_tints_the_materials_near_and_is_the_ground_far() {
        let l = look_on();
        let cm = [0.1, 0.2, 0.05];
        // The blurred photo is brown, the sharp one has a grey car in it.
        let low = [0.2, 0.15, 0.1];
        let sharp = [0.2, 0.3, 0.1];
        let near = blend(2.0, 1.0, &l);
        let bright = apply(near, [0.15, 0.3, 0.075], cm, sharp, low);
        let dark = apply(near, [0.05, 0.1, 0.025], cm, sharp, low);
        assert!(bright[0] > 2.0 * dark[0]);
        // Redder than the material (the photo is brown, the meadow green):
        // the blurred photo's colour, the material's detail.
        assert!((bright[0] / bright[1] - low[0] / low[1]).abs() < 1e-4);
        assert!((bright[0] / dark[0] - 3.0).abs() < 1e-3);
        // Far off, where the materials are the colour map: the sharp photo.
        let far = blend(300.0, 1.0, &l);
        let f = apply(far, cm, cm, sharp, low);
        for i in 0..3 {
            assert!((f[i] - sharp[i]).abs() < 1e-4, "{f:?}");
        }
        // Outside the corridor nothing changes.
        let none = apply(blend(300.0, 0.0, &l), [0.1, 0.2, 0.05], cm, sharp, low);
        assert_eq!(none, [0.1, 0.2, 0.05]);
    }

    fn jpeg(rgb: [u8; 3], n: u32) -> Vec<u8> {
        let buf: Vec<u8> = (0..n * n).flat_map(|_| rgb).collect();
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90)
            .encode(&buf, n, n, image::ExtendedColorType::Rgb8)
            .unwrap();
        out
    }

    /// The texture: chunks at their place (rows along +z), the weight in
    /// alpha (bilinear, 0 where no chunk is), mips down to 8 px, BC3 of
    /// the right size.
    #[test]
    fn tile_textures_put_chunks_and_weights_in_place() {
        // A 100 m tile, 2 × 2 chunks of 50 m at 1 m, weights at 25 m.
        let corners = 5u32;
        let mut w = vec![0u8; 25];
        // Full weight on the lower row (z = 0) of corners.
        w[..5].fill(255);
        let mut chunks = BTreeMap::new();
        chunks.insert((0u8, 0u8), jpeg([200, 30, 30], 50));
        chunks.insert((1, 0), jpeg([30, 30, 200], 50));
        let mut tiles = BTreeMap::new();
        tiles.insert(
            TileKey { x: 0, z: 0 },
            OrthoTile {
                weights: encode_weights(&w, corners),
                chunks,
            },
        );
        let c = OrthoCorridor {
            pixel_m: 1.0,
            half_width_m: 50.0,
            chunks: 2,
            weight_cells: 4,
            encoding: OrthoEncoding::Jpeg,
            sources: vec![0],
            tiles,
        };
        assert!(tile_texture(&c, TileKey { x: 1, z: 0 }, 100.0).is_none());
        let t = tile_texture(&c, TileKey { x: 0, z: 0 }, 100.0)
            .unwrap()
            .unwrap();
        assert_eq!(t.size, 100);
        // 100, 50, 25, 12.
        assert_eq!(t.levels.len(), 4);
        let px = |x: usize, z: usize| &t.levels[0][(z * 100 + x) * 4..(z * 100 + x) * 4 + 4];
        assert!(px(10, 2)[0] > 150 && px(10, 2)[2] < 80);
        assert!(px(70, 2)[2] > 150);
        // Weight: full at z = 0, fading up to the next corner row (25 m).
        assert!(px(10, 0)[3] > 245);
        let mid = px(10, 12)[3];
        assert!(mid > 90 && mid < 160, "{mid}");
        assert_eq!(px(10, 40)[3], 0);
        // No chunk in the upper half: weight 0.
        assert_eq!(px(10, 80)[3], 0);
        // Mips average in linear light: red and blue halves → purple.
        let m = &t.levels[3];
        assert_eq!(m.len(), 12 * 12 * 4);
        let bc = compress_bc3(&t);
        assert_eq!(bc[0].len(), 25 * 25 * 16);
        assert_eq!(bc[3].len(), 3 * 3 * 16);
        let img = ortho_image(&t);
        assert_eq!(img.texture_descriptor.mip_level_count, 4);
        assert_eq!(
            img.texture_descriptor.format,
            TextureFormat::Bc3RgbaUnormSrgb
        );
    }

    #[test]
    fn mips_average_in_linear_light() {
        let lut = lin_lut();
        // Black and white texels: the mean is mid grey in linear light
        // (sRGB 188), not 128.
        let src = vec![
            0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 255, 255, 255, 255, 255,
        ];
        let (h, s) = half_level(&src, 2, &lut);
        assert_eq!(s, 1);
        assert!((i32::from(h[0]) - 188).abs() <= 1, "{}", h[0]);
        assert_eq!(h[3], 255);
    }

    /// The setting's command (`display.orthophoto`): on by default, `ortho
    /// off` switches it off and says why nothing is drawn.
    #[test]
    fn the_console_command_switches_the_photo() {
        assert!(OrthoSettings::default().enabled);
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            bevy::asset::AssetPlugin::default(),
            crate::console::ConsoleCorePlugin,
            OrthoPlugin,
        ))
        .init_asset::<Image>()
        .init_asset::<TerrainMaterial>();
        let mut run = |line: &str| {
            app.world_mut()
                .resource_mut::<crate::console::ConsoleState>()
                .queue(line);
            app.update();
            let log = app
                .world()
                .resource::<crate::console::ConsoleState>()
                .log()
                .to_vec();
            log.last().cloned().unwrap_or_default()
        };
        assert_eq!(
            run("ortho off"),
            "orthophoto off (this package has no corridor)"
        );
        assert!(!app.world().resource::<OrthoSettings>().enabled);
        let mut run = |line: &str| {
            app.world_mut()
                .resource_mut::<crate::console::ConsoleState>()
                .queue(line);
            app.update();
            app.world()
                .resource::<crate::console::ConsoleState>()
                .log()
                .last()
                .cloned()
                .unwrap_or_default()
        };
        assert!(run("ortho on").starts_with("orthophoto on"));
        assert!(app.world().resource::<OrthoSettings>().enabled);
    }
}
