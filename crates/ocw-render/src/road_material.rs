//! Road surfaces (spec §4.4, §4.6 step 6, Phase 8 T2): package road strips
//! drawn with the library's road materials – asphalt, worn asphalt,
//! gravel, paving, dirt – chosen per strip from OSM `surface`
//! (`ocw_world::RoadSurface`, package section `RSRF`) with the category as
//! fallback. The strip meshes carry the layer in their second texture
//! coordinate and (across, along) in metres in the first; the edge faces
//! keep their flat colour. Far away the road fades to its vertex colour,
//! the material's mean. Markings (the centre line) are separate meshes and
//! stay as they are.

use bevy::asset::{AssetPath, embedded_asset, embedded_path};
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

use crate::materials::MaterialLibrary;

pub type RoadMaterial = ExtendedMaterial<StandardMaterial, RoadExtension>;

/// Uniform of `road.wesl`.
#[derive(Debug, Clone, Copy, PartialEq, Reflect, ShaderType)]
pub struct RoadLook {
    /// x: textured; y, z: fade start, end (m); w: normal strength.
    pub mode: Vec4,
    /// Repeat size (m) of the five layers (x of the first vector … x of
    /// the second).
    pub sizes: [Vec4; 2],
}

impl Default for RoadLook {
    fn default() -> Self {
        Self {
            mode: Vec4::new(0.0, 60.0, 250.0, 0.9),
            sizes: [Vec4::splat(2.0); 2],
        }
    }
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct RoadExtension {
    #[uniform(100)]
    pub look: RoadLook,
    #[texture(101, dimension = "2d_array")]
    #[sampler(102)]
    pub albedo: Option<Handle<Image>>,
    #[texture(103, dimension = "2d_array")]
    pub normal: Option<Handle<Image>>,
}

impl RoadExtension {
    pub fn of_library(lib: &MaterialLibrary) -> Self {
        let mut look = RoadLook::default();
        look.mode.x = 1.0;
        for (i, s) in lib.road_sizes.iter().enumerate() {
            look.sizes[i / 4][i % 4] = *s;
        }
        Self {
            look,
            albedo: Some(lib.road_albedo.clone()),
            normal: Some(lib.road_normal.clone()),
        }
    }
}

impl MaterialExtension for RoadExtension {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("road.wesl")).with_source("embedded"),
        )
    }
}

pub struct RoadMaterialPlugin;

impl Plugin for RoadMaterialPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "road.wesl");
        app.add_plugins(MaterialPlugin::<RoadMaterial>::default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn road_look_defaults_and_layer_sizes() {
        let l = RoadLook::default();
        assert_eq!(l.mode.x, 0.0, "untextured without a library");
        assert!(l.mode.y < l.mode.z);
        // Five layers fit the two vectors.
        assert!(ocw_assets::materials::ROAD_IDS.len() <= 8);
    }
}
