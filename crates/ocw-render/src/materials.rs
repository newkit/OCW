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
use ocw_assets::materials::{BUILDING_IDS, GEAR_IDS, GROUND_IDS, Library, Loaded, ROAD_IDS};

use crate::console::{ConsoleAppExt, ConsoleCommand, Permission};
use crate::ground::{ClassMix, GROUND_LAYERS, TerrainLook};

/// The ground of one region: the nine layer slots filled with the
/// region's materials (`[[region]]` in `assets/materials.toml`; the first
/// set is the default) and its class mix.
#[derive(Clone)]
pub struct GroundSet {
    pub id: String,
    pub time_zones: Vec<String>,
    pub albedo: Handle<Image>,
    pub normal: Handle<Image>,
    /// Look with the layer sizes (textured).
    pub look: TerrainLook,
    /// Mean albedo of the layers (linear RGB).
    pub means: [[f32; 3]; GROUND_LAYERS],
    pub mix: ClassMix,
}

/// The loaded library (absent until loaded; [`MaterialState`] says why not).
#[derive(Resource, Clone)]
pub struct MaterialLibrary {
    /// The default ground set first, then one per region.
    pub sets: Vec<GroundSet>,
    pub road_albedo: Handle<Image>,
    pub road_normal: Handle<Image>,
    /// Repeat sizes of the road layers (m).
    pub road_sizes: [f32; ROAD_IDS.len()],
    /// Mean albedo of the road layers (linear RGB).
    pub road_means: [[f32; 3]; ROAD_IDS.len()],
    /// The building layers (Phase 8 T4; `None`: a library without them,
    /// the houses keep flat colours).
    pub buildings: Option<BuildingLayers>,
    /// The rider and bike layers (Phase 8 T6; `None`: an older library,
    /// riders and bikes keep their procedural detail only).
    pub gear: Option<GearLayers>,
    pub credits: Vec<String>,
    pub dir: String,
}

impl MaterialLibrary {
    /// The ground set of a package with this time zone.
    pub fn set_for(&self, time_zone: Option<&str>) -> &GroundSet {
        time_zone
            .and_then(|tz| {
                self.sets
                    .iter()
                    .find(|s| s.time_zones.iter().any(|z| z == tz))
            })
            .unwrap_or(&self.sets[0])
    }
}

/// The building materials as texture arrays, in `house_style::Layer`
/// order (`ocw_assets::materials::BUILDING_IDS`).
#[derive(Clone)]
pub struct BuildingLayers {
    pub albedo: Handle<Image>,
    pub normal: Handle<Image>,
    /// Repeat sizes (m).
    pub sizes: [f32; crate::house_style::LAYERS],
    /// Mean albedo (linear RGB).
    pub means: [[f32; 3]; crate::house_style::LAYERS],
}

/// The rider and bike materials as texture arrays, in
/// `ocw_assets::materials::GEAR_IDS` order (Phase 8 T6: jersey knit,
/// carbon twill, saddle leather).
#[derive(Clone)]
pub struct GearLayers {
    pub albedo: Handle<Image>,
    pub normal: Handle<Image>,
    pub sizes: [f32; GEAR_IDS.len()],
    pub means: [[f32; 3]; GEAR_IDS.len()],
}

/// The gear layers of a library, if it has them all (an older library
/// folder: none, with a warning).
fn gear_layers(lib: &Library, images: &mut Assets<Image>) -> Option<GearLayers> {
    let l = match layers(lib, &GEAR_IDS) {
        Ok(l) => l,
        Err(e) => {
            warn!("riders and bikes without textures: {e} (run scripts/fetch-materials.sh)");
            return None;
        }
    };
    let albedo: Vec<_> = l.iter().map(|m| &m.albedo).collect();
    let normal: Vec<_> = l.iter().map(|m| &m.normal).collect();
    match (
        array_image(&albedo, "gear albedo"),
        array_image(&normal, "gear normal"),
    ) {
        (Ok(a), Ok(n)) => Some(GearLayers {
            albedo: images.add(a),
            normal: images.add(n),
            sizes: std::array::from_fn(|i| l[i].material.size_m),
            means: std::array::from_fn(|i| l[i].mean),
        }),
        (Err(e), _) | (_, Err(e)) => {
            warn!("riders and bikes without textures: {e}");
            None
        }
    }
}

/// The building layers of a library, if it has them all (an older library
/// folder without the T4 materials: none, with a warning).
fn building_layers(lib: &Library, images: &mut Assets<Image>) -> Option<BuildingLayers> {
    let l = match layers(lib, &BUILDING_IDS) {
        Ok(l) => l,
        Err(e) => {
            warn!("houses without textures: {e} (run scripts/fetch-materials.sh)");
            return None;
        }
    };
    let albedo: Vec<_> = l.iter().map(|m| &m.albedo).collect();
    let normal: Vec<_> = l.iter().map(|m| &m.normal).collect();
    let (a, n) = match (
        array_image(&albedo, "building albedo"),
        array_image(&normal, "building normal"),
    ) {
        (Ok(a), Ok(n)) => (a, n),
        (Err(e), _) | (_, Err(e)) => {
            warn!("houses without textures: {e}");
            return None;
        }
    };
    Some(BuildingLayers {
        albedo: images.add(a),
        normal: images.add(n),
        sizes: std::array::from_fn(|i| l[i].material.size_m),
        means: std::array::from_fn(|i| l[i].mean),
    })
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
    let road = layers(lib, &ROAD_IDS)?;
    fn albedo<'a>(l: &[&'a Loaded]) -> Vec<&'a ocw_assets::ktx2::Texture> {
        l.iter().map(|m| &m.albedo).collect()
    }
    fn normal<'a>(l: &[&'a Loaded]) -> Vec<&'a ocw_assets::ktx2::Texture> {
        l.iter().map(|m| &m.normal).collect()
    }
    let mut sets = Vec::new();
    let mut ground_set = |id: &str,
                          time_zones: Vec<String>,
                          ids: Vec<String>,
                          mix: ClassMix|
     -> Result<GroundSet, String> {
        let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        let ground = layers(lib, &refs)?;
        let mut look = TerrainLook::default();
        look.mode.x = 1.0;
        for (i, m) in ground.iter().enumerate() {
            look.set_size(i, m.material.size_m);
        }
        Ok(GroundSet {
            id: id.to_owned(),
            time_zones,
            albedo: images.add(array_image(&albedo(&ground), "ground albedo")?),
            normal: images.add(array_image(&normal(&ground), "ground normal")?),
            look,
            means: std::array::from_fn(|i| ground[i].mean),
            mix,
        })
    };
    sets.push(ground_set(
        "default",
        Vec::new(),
        GROUND_IDS.iter().map(|s| (*s).to_owned()).collect(),
        crate::ground::default_mix(),
    )?);
    for r in &lib.manifest.regions {
        sets.push(ground_set(
            &r.id,
            r.time_zones.clone(),
            GROUND_IDS.iter().map(|s| r.layer(s)).collect(),
            crate::ground::region_mix(r),
        )?);
    }
    Ok(MaterialLibrary {
        sets,
        road_albedo: images.add(array_image(&albedo(&road), "road albedo")?),
        road_normal: images.add(array_image(&normal(&road), "road normal")?),
        road_sizes: std::array::from_fn(|i| road[i].material.size_m),
        road_means: std::array::from_fn(|i| road[i].mean),
        buildings: building_layers(lib, images),
        gear: gear_layers(lib, images),
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
