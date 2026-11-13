//! The material library on the GPU (spec §4.6, Phase 8 T1/T2): the ground
//! and road materials of `assets/materials/` (`scripts/fetch-materials.sh`)
//! as two texture arrays each (albedo + roughness BC3, normal BC5).
//!
//! The library is looked for beside the binary, in the AppImage / macOS
//! bundle layout, in the checkout and in `$OCW_MATERIALS`
//! (`ocw_assets::materials::library_candidates`); `OCW_MATERIALS=off`
//! turns it off. Without it, or on a GPU without BC texture compression,
//! the world keeps the flat land-use colours. The console command
//! `materials` reports which library is in use and lists the credits.

use bevy::asset::RenderAssetUsages;
use bevy::image::{
    CompressedImageFormatSupport, CompressedImageFormats, ImageAddressMode, ImageFilterMode,
    ImageSampler, ImageSamplerDescriptor,
};
use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, TextureDataOrder, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages,
    TextureViewDescriptor, TextureViewDimension,
};
use clap::Command;
use ocw_assets::ktx2::TexFormat;
use ocw_assets::materials::{GROUND_IDS, Library, Loaded, ROAD_IDS};

use crate::console::{ConsoleAppExt, ConsoleCommand, Permission};
use crate::ground::{GROUND_LAYERS, TerrainLook};

/// The loaded library (absent until loaded; [`MaterialState`] says why not).
#[derive(Resource, Clone)]
pub struct MaterialLibrary {
    pub ground_albedo: Handle<Image>,
    pub ground_normal: Handle<Image>,
    pub road_albedo: Handle<Image>,
    pub road_normal: Handle<Image>,
    /// Ground look with the layer sizes (textured).
    pub ground_look: TerrainLook,
    /// Repeat sizes of the road layers (m).
    pub road_sizes: [f32; ROAD_IDS.len()],
    /// Mean albedo of the ground layers (linear RGB).
    pub ground_means: [[f32; 3]; GROUND_LAYERS],
    /// Mean albedo of the road layers (linear RGB).
    pub road_means: [[f32; 3]; ROAD_IDS.len()],
    pub credits: Vec<String>,
    pub dir: String,
}

/// Whether the library is in use, and why not.
#[derive(Resource, Debug, Clone, PartialEq, Default)]
pub enum MaterialState {
    #[default]
    Pending,
    Loaded(String),
    Unavailable(String),
}

pub struct MaterialsPlugin;

impl Plugin for MaterialsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MaterialState>()
            .add_systems(
                PreUpdate,
                load_library.run_if(|s: Res<MaterialState>| *s == MaterialState::Pending),
            )
            .console_command(ConsoleCommand::new(
                Permission::User,
                Command::new("materials")
                    .about("Material library (spec §4.6): which one is in use, and its credits"),
                materials_command,
            ));
    }
}

fn materials_command(world: &mut World, _: &clap::ArgMatches) -> Result<String, String> {
    let mut out = match world.resource::<MaterialState>().clone() {
        MaterialState::Pending => "material library: not loaded yet".to_owned(),
        MaterialState::Loaded(d) => format!("material library: {d}"),
        MaterialState::Unavailable(why) => format!("material library: off ({why}); flat colours"),
    };
    if let Some(lib) = world.get_resource::<MaterialLibrary>() {
        for c in &lib.credits {
            out.push('\n');
            out.push_str(c);
        }
    }
    Ok(out)
}

/// The folder of the library, if any (`OCW_MATERIALS=off`: none).
pub fn library_dir() -> Option<std::path::PathBuf> {
    let env = std::env::var("OCW_MATERIALS").ok();
    if env.as_deref() == Some("off") {
        return None;
    }
    let exe = std::env::current_exe().ok();
    let checkout = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let candidates =
        ocw_assets::materials::library_candidates(env.as_deref(), exe.as_deref(), Some(&checkout));
    ocw_assets::materials::find_library(&candidates, |p| p.is_file())
}

fn load_library(
    mut commands: Commands,
    mut state: ResMut<MaterialState>,
    support: Option<Res<CompressedImageFormatSupport>>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(support) = support else {
        // No render device (headless tests): stay flat.
        *state = MaterialState::Unavailable("no GPU".into());
        return;
    };
    if !support.0.contains(CompressedImageFormats::BC) {
        *state = MaterialState::Unavailable("the GPU has no BC texture compression".into());
        warn!("material library off: the GPU has no BC texture compression");
        return;
    }
    let Some(dir) = library_dir() else {
        *state = MaterialState::Unavailable(
            "no material library (scripts/fetch-materials.sh, or OCW_MATERIALS=off)".into(),
        );
        info!("no material library found: flat land-use colours (run scripts/fetch-materials.sh)");
        return;
    };
    let t = std::time::Instant::now();
    match ocw_assets::materials::load_library(&dir) {
        Ok(lib) => match library_resource(&lib, &mut images) {
            Ok(res) => {
                info!(
                    "material library {}: {} materials in {:.0} ms",
                    dir.display(),
                    lib.materials.len(),
                    t.elapsed().as_secs_f64() * 1000.0
                );
                *state = MaterialState::Loaded(dir.display().to_string());
                commands.insert_resource(res);
            }
            Err(e) => {
                warn!("material library {}: {e}", dir.display());
                *state = MaterialState::Unavailable(e);
            }
        },
        Err(errors) => {
            let e = errors.join("; ");
            warn!("material library {}: {e}", dir.display());
            *state = MaterialState::Unavailable(e);
        }
    }
}

fn layers<'a>(lib: &'a Library, ids: &[&str]) -> Result<Vec<&'a Loaded>, String> {
    ids.iter()
        .map(|id| {
            lib.get(id)
                .ok_or_else(|| format!("material `{id}` missing"))
        })
        .collect()
}

/// The GPU format of a library texture format.
pub fn gpu_format(f: TexFormat) -> TextureFormat {
    match f {
        TexFormat::Bc3Srgb => TextureFormat::Bc3RgbaUnormSrgb,
        TexFormat::Bc5 => TextureFormat::Bc5RgUnorm,
        TexFormat::Rgba8Srgb => TextureFormat::Rgba8UnormSrgb,
        TexFormat::Rgba8 => TextureFormat::Rgba8Unorm,
    }
}

/// One texture array of `textures` (all the same size, format and level
/// count): layer-major data, repeating, trilinear, anisotropic.
pub fn array_image(
    textures: &[&ocw_assets::ktx2::Texture],
    label: &'static str,
) -> Result<Image, String> {
    let first = textures.first().ok_or("no layers")?;
    for t in textures {
        if (t.format, t.width, t.height, t.levels.len())
            != (first.format, first.width, first.height, first.levels.len())
        {
            return Err(format!("{label}: layers differ in size or format"));
        }
    }
    let data: Vec<u8> = textures
        .iter()
        .flat_map(|t| t.levels.iter().flatten().copied())
        .collect();
    Ok(Image {
        data: Some(data),
        data_order: TextureDataOrder::LayerMajor,
        texture_descriptor: TextureDescriptor {
            label: Some(label),
            size: Extent3d {
                width: first.width,
                height: first.height,
                depth_or_array_layers: textures.len() as u32,
            },
            mip_level_count: first.levels.len() as u32,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: gpu_format(first.format),
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        },
        sampler: ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::Repeat,
            address_mode_v: ImageAddressMode::Repeat,
            mag_filter: ImageFilterMode::Linear,
            min_filter: ImageFilterMode::Linear,
            mipmap_filter: ImageFilterMode::Linear,
            anisotropy_clamp: 8,
            ..default()
        }),
        texture_view_descriptor: Some(TextureViewDescriptor {
            label: Some(label),
            dimension: Some(TextureViewDimension::D2Array),
            ..default()
        }),
        asset_usage: RenderAssetUsages::RENDER_WORLD,
        ..default()
    })
}

/// The resource for a loaded library.
pub fn library_resource(
    lib: &Library,
    images: &mut Assets<Image>,
) -> Result<MaterialLibrary, String> {
    let ground = layers(lib, &GROUND_IDS)?;
    let road = layers(lib, &ROAD_IDS)?;
    fn albedo<'a>(l: &[&'a Loaded]) -> Vec<&'a ocw_assets::ktx2::Texture> {
        l.iter().map(|m| &m.albedo).collect()
    }
    fn normal<'a>(l: &[&'a Loaded]) -> Vec<&'a ocw_assets::ktx2::Texture> {
        l.iter().map(|m| &m.normal).collect()
    }
    let mut look = TerrainLook::default();
    look.mode.x = 1.0;
    for (i, m) in ground.iter().enumerate() {
        look.set_size(i, m.material.size_m);
    }
    Ok(MaterialLibrary {
        ground_albedo: images.add(array_image(&albedo(&ground), "ground albedo")?),
        ground_normal: images.add(array_image(&normal(&ground), "ground normal")?),
        road_albedo: images.add(array_image(&albedo(&road), "road albedo")?),
        road_normal: images.add(array_image(&normal(&road), "road normal")?),
        ground_look: look,
        road_sizes: std::array::from_fn(|i| road[i].material.size_m),
        ground_means: std::array::from_fn(|i| ground[i].mean),
        road_means: std::array::from_fn(|i| road[i].mean),
        credits: ocw_assets::materials::credits(&lib.manifest)
            .iter()
            .map(|c| c.line())
            .collect(),
        dir: lib.dir.display().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_assets::ktx2::{Texture, full_chain};

    fn tex(format: TexFormat, size: u32, fill: u8) -> Texture {
        let n = full_chain(size, size) as usize;
        Texture {
            format,
            width: size,
            height: size,
            levels: (0..n)
                .map(|i| vec![fill; format.level_bytes((size >> i).max(1), (size >> i).max(1))])
                .collect(),
            metadata: vec![],
        }
    }

    #[test]
    fn layers_become_one_array_in_layer_major_order() {
        let (a, b) = (tex(TexFormat::Bc3Srgb, 8, 1), tex(TexFormat::Bc3Srgb, 8, 2));
        let img = array_image(&[&a, &b], "t").unwrap();
        let d = img.texture_descriptor;
        assert_eq!(d.size.depth_or_array_layers, 2);
        assert_eq!(d.mip_level_count, 4);
        assert_eq!(d.format, TextureFormat::Bc3RgbaUnormSrgb);
        let data = img.data.unwrap();
        let per_layer: usize = a.levels.iter().map(Vec::len).sum();
        assert_eq!(data.len(), 2 * per_layer);
        assert!(data[..per_layer].iter().all(|x| *x == 1));
        assert!(data[per_layer..].iter().all(|x| *x == 2));
        assert_eq!(
            img.texture_view_descriptor.unwrap().dimension,
            Some(TextureViewDimension::D2Array)
        );
        // Mismatched layers are refused.
        let small = tex(TexFormat::Bc3Srgb, 4, 0);
        assert!(array_image(&[&a, &small], "t").is_err());
        assert_eq!(gpu_format(TexFormat::Bc5), TextureFormat::Bc5RgUnorm);
    }

    /// The road layers are the road surfaces' materials, in code order.
    #[test]
    fn road_layers_match_the_road_surfaces() {
        for s in ocw_world::RoadSurface::ALL {
            assert_eq!(ROAD_IDS[s.code() as usize], s.material_id());
        }
    }

    /// Without a GPU (headless tests) the library stays off and the world
    /// flat-coloured; `OCW_MATERIALS=off` is honoured.
    #[test]
    fn without_a_gpu_the_library_is_off() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Image>()
            .add_plugins(crate::console::ConsoleCorePlugin)
            .add_plugins(MaterialsPlugin);
        app.update();
        assert!(matches!(
            app.world().resource::<MaterialState>(),
            MaterialState::Unavailable(_)
        ));
        assert!(app.world().get_resource::<MaterialLibrary>().is_none());
    }
}
