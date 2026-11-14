//! Road surfaces (spec §4.4, §4.6 step 6, Phase 8 T2 / T5): package road
//! strips drawn with the library's road materials – asphalt, worn asphalt,
//! gravel, paving, dirt, and the sidewalk pavers and kerb stones – chosen
//! per strip from OSM `surface` (`ocw_world::RoadSurface`, package section
//! `RSRF`) with the category as fallback. The strip meshes
//! ([`crate::road_mesh`]) carry the layer, the part (carriageway, kerb,
//! sidewalk, shoulder, wall, crossing overlay) and the marking code in
//! their second texture coordinate and (across, along) in metres in the
//! first. Far away the road fades to its vertex colour, the material's
//! mean.
//!
//! **Markings (T5)** are drawn in the shader from the strip's code and the
//! regional style ([`crate::road_style`]) – centre, lane, edge and cycle
//! lane lines on the carriageway, zebras, stop and give-way lines on the
//! overlays – as coverage filtered over each pixel's footprint, so they
//! stay crisp near and turn into their mean grey far away instead of
//! shimmering. **Wear:** tyre tracks, repaired patches and transverse
//! cracks on asphalt, grass between the wheel ruts of tracks, all
//! deterministic per position. Without a material library the same shader
//! draws the flat colours with the markings.

use bevy::asset::{AssetPath, embedded_asset, embedded_path};
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;
use ocw_world::road_dress::{CycleLane, LineSpec, MarkingStyle};

use crate::materials::MaterialLibrary;

pub type RoadMaterial = ExtendedMaterial<StandardMaterial, RoadExtension>;

/// Uniform of `road.wesl`.
#[derive(Debug, Clone, Copy, PartialEq, Reflect, ShaderType)]
pub struct RoadLook {
    /// x: textured; y, z: fade start, end (m); w: normal strength.
    pub mode: Vec4,
    /// Repeat size (m) of the road layers (x of the first vector … w of
    /// the second).
    pub sizes: [Vec4; 2],
    /// Centre line outside towns: width, dash, gap; w: double-line gap.
    pub centre: Vec4,
    /// Centre line in towns: width, dash, gap; w: edge line inset.
    pub centre_town: Vec4,
    /// Lane lines outside towns: width, dash, gap; w: edge line width.
    pub lane: Vec4,
    /// Lane lines in towns: width, dash, gap; w: cycle lane width.
    pub lane_town: Vec4,
    /// Advisory cycle lane line: width, dash, gap; w: exclusive width.
    pub cycle: Vec4,
    /// Zebra: bar, gap, length; w: stop line width.
    pub zebra: Vec4,
    /// Give-way line: width, dash, gap; w: signals spacing.
    pub give_way: Vec4,
    /// Signal crossing lines: width, dash, gap; w: stop line distance.
    pub signals: Vec4,
    /// Line paint (linear rgb); w: wear strength (0: none).
    pub line_colour: Vec4,
    /// Cycle lane paint; w: 1 = an untyped cycle lane is exclusive.
    pub cycle_colour: Vec4,
    /// Zebra paint; w: width of the kerb stones' top (m).
    pub crossing_colour: Vec4,
    /// x: patch chance on asphalt, y: on worn asphalt, z: crack chance on
    /// worn asphalt, w: on asphalt.
    pub wear: Vec4,
    /// The weather (spec §14 Phase 9, S4; [`crate::weather::surface_uniform`]):
    /// x wetness, y puddles, z snow cover, w wind sway.
    pub weather: Vec4,
}

fn spec(l: LineSpec, w: f32) -> Vec4 {
    Vec4::new(l.width, l.dash, l.gap, w)
}

impl RoadLook {
    /// Untextured, with the markings of `style`.
    pub fn of_style(style: &MarkingStyle) -> Self {
        let c = |rgb: [f32; 3], w: f32| Vec4::new(rgb[0], rgb[1], rgb[2], w);
        Self {
            mode: Vec4::new(0.0, 60.0, 250.0, 0.9),
            sizes: [Vec4::splat(2.0); 2],
            centre: spec(style.centre, style.double_gap_m),
            centre_town: spec(style.centre_town, style.edge_inset_m),
            lane: spec(style.lane, style.edge_width_m),
            lane_town: spec(style.lane_town, style.cycle_lane_m),
            cycle: spec(style.cycle_advisory, style.cycle_exclusive_m),
            zebra: Vec4::new(
                style.zebra_bar_m,
                style.zebra_gap_m,
                style.zebra_length_m,
                style.stop_m,
            ),
            give_way: spec(style.give_way, style.signals_spacing_m),
            signals: spec(style.signals, style.stop_before_m),
            line_colour: c(style.line_colour, 1.0),
            cycle_colour: c(
                style.cycle_colour,
                f32::from(u8::from(style.cycle_default == CycleLane::Exclusive)),
            ),
            crossing_colour: c(style.crossing_colour, 0.15),
            wear: Vec4::new(0.06, 0.22, 0.45, 0.08),
            weather: Vec4::ZERO,
        }
    }
}

impl Default for RoadLook {
    fn default() -> Self {
        Self::of_style(&MarkingStyle::default())
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
    /// The road material of a marking style, textured with the library's
    /// road layers if there is one.
    pub fn new(lib: Option<&MaterialLibrary>, style: &MarkingStyle) -> Self {
        let mut look = RoadLook::of_style(style);
        let Some(lib) = lib else {
            return Self {
                look,
                albedo: None,
                normal: None,
            };
        };
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

    pub fn of_library(lib: &MaterialLibrary) -> Self {
        Self::new(Some(lib), &MarkingStyle::default())
    }
}

impl MaterialExtension for RoadExtension {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("road.wesl")).with_source("embedded"),
        )
    }
}

/// The road materials of one marking style: the opaque strips and the
/// blended overlays (crossings).
#[derive(Debug, Clone)]
pub struct RoadMaterials {
    pub strips: Handle<RoadMaterial>,
    pub overlays: Handle<RoadMaterial>,
}

impl RoadMaterials {
    pub fn new(
        assets: &mut Assets<RoadMaterial>,
        lib: Option<&MaterialLibrary>,
        style: &MarkingStyle,
    ) -> Self {
        let ext = RoadExtension::new(lib, style);
        let base = StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 1.0,
            ..default()
        };
        Self {
            strips: assets.add(RoadMaterial {
                base: base.clone(),
                extension: ext.clone(),
            }),
            overlays: assets.add(RoadMaterial {
                base: StandardMaterial {
                    alpha_mode: AlphaMode::Blend,
                    perceptual_roughness: 0.55,
                    ..base
                },
                extension: ext,
            }),
        }
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
        // The road layers fit the two vectors.
        assert!(ocw_assets::materials::ROAD_IDS.len() <= 8);
        assert_eq!(
            usize::from(ocw_world::road_dress::KERB_LAYER) + 1,
            ocw_assets::materials::ROAD_IDS.len()
        );
        assert_eq!(
            ocw_assets::materials::ROAD_IDS[usize::from(ocw_world::road_dress::SIDEWALK_LAYER)],
            "sidewalk"
        );
    }

    #[test]
    fn the_uniform_carries_the_regional_style() {
        let de = RoadLook::of_style(crate::road_style::style_for(Some("Europe/Berlin")));
        let ch = RoadLook::of_style(crate::road_style::style_for(Some("Europe/Vaduz")));
        assert_eq!(de.centre.truncate(), Vec3::new(0.12, 6.0, 12.0));
        assert_eq!(de.centre_town.truncate(), Vec3::new(0.12, 3.0, 6.0));
        assert_eq!(de.cycle_colour.w, 1.0, "German cycle lanes: exclusive");
        assert_eq!(ch.cycle_colour.w, 0.0, "Swiss: advisory");
        assert!(ch.crossing_colour.z < 0.1, "yellow zebras");
        assert_eq!(de.zebra.x, 0.5);
    }
}
