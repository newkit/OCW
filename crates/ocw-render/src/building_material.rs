//! The house material (spec §14 Phase 8 T4): one shared material for all
//! building meshes (`houses`) – the library's building layers as texture
//! arrays (when loaded), the window styles of `assets/building-styles.toml`
//! and the night factor that lights the windows (`building.wesl`).

use bevy::asset::{AssetPath, embedded_asset, embedded_path};
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

use crate::house_style::{BuildingStyles, LAYERS, Layer, MAX_WINDOW_STYLES};
use crate::materials::MaterialLibrary;

pub type BuildingMaterial = ExtendedMaterial<StandardMaterial, BuildingExtension>;

/// Uniform of `building.wesl`.
#[derive(Debug, Clone, Copy, PartialEq, Reflect, ShaderType)]
pub struct BuildingLook {
    /// x: textured; y, z: texture fade start, end (m); w: normal strength.
    pub mode: Vec4,
    /// x, y: window fade start, end (m); z: night (0–1); w: lit window
    /// brightness.
    pub facade: Vec4,
    /// Repeat sizes (m) of the 16 layers.
    pub sizes: [Vec4; 4],
    /// Mean albedo per layer (linear).
    pub means: [Vec4; 16],
    /// Four vectors per window style.
    pub windows: [Vec4; 64],
}

/// Distances (m) over which the textures and the windows fade out.
pub const TEXTURE_FADE_M: [f32; 2] = [90.0, 320.0];
pub const WINDOW_FADE_M: [f32; 2] = [420.0, 900.0];
/// Emissive brightness of a lit window (physical units, scaled by the
/// camera's exposure like the sky).
pub const LIT_WINDOW: f32 = 60.0;

impl BuildingLook {
    /// The look without textures: the layers' fallback colours, the
    /// window styles of `styles`.
    pub fn new(styles: &BuildingStyles) -> Self {
        let mut means = [Vec4::ONE; 16];
        for l in [
            Layer::RoofTiles,
            Layer::Plaster,
            Layer::Timber,
            Layer::RoofSlates,
            Layer::Metal,
            Layer::Brick,
            Layer::Stone,
            Layer::Concrete,
            Layer::Glass,
        ] {
            let c = l.fallback_colour();
            means[l as usize] = Vec4::new(c[0], c[1], c[2], 1.0);
        }
        let mut windows = [Vec4::ZERO; 64];
        for (i, w) in styles.windows.iter().take(MAX_WINDOW_STYLES).enumerate() {
            let flags = f32::from(u8::from(w.shutters.is_some()) | (u8::from(w.shopfront) << 1));
            let sh = w.shutters.unwrap_or(w.frame);
            windows[i * 4] = Vec4::new(w.bay_m, w.width_m, w.height_m, w.sill_m);
            windows[i * 4 + 1] = Vec4::new(w.frame[0], w.frame[1], w.frame[2], flags);
            windows[i * 4 + 2] = Vec4::new(sh[0], sh[1], sh[2], w.door_width_m);
            windows[i * 4 + 3] = Vec4::new(w.door[0], w.door[1], w.door[2], w.door_height_m);
        }
        Self {
            mode: Vec4::new(0.0, TEXTURE_FADE_M[0], TEXTURE_FADE_M[1], 0.8),
            facade: Vec4::new(WINDOW_FADE_M[0], WINDOW_FADE_M[1], 0.0, LIT_WINDOW),
            sizes: [Vec4::splat(2.0); 4],
            means,
            windows,
        }
    }

    /// With the library's building layers: textured, their sizes and means.
    pub fn with_library(mut self, lib: &MaterialLibrary) -> Self {
        if let Some(b) = &lib.buildings {
            self.mode.x = 1.0;
            for i in 0..LAYERS {
                self.sizes[i / 4][i % 4] = b.sizes[i];
                let m = b.means[i];
                self.means[i] = Vec4::new(m[0], m[1], m[2], 1.0);
            }
        }
        self
    }

    pub fn set_night(&mut self, night: f32) {
        self.facade.z = night.clamp(0.0, 1.0);
    }
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct BuildingExtension {
    #[uniform(100)]
    pub look: BuildingLook,
    #[texture(101, dimension = "2d_array")]
    #[sampler(102)]
    pub albedo: Option<Handle<Image>>,
    #[texture(103, dimension = "2d_array")]
    pub normal: Option<Handle<Image>>,
}

impl BuildingExtension {
    pub fn new(styles: &BuildingStyles, lib: Option<&MaterialLibrary>) -> Self {
        let look = BuildingLook::new(styles);
        match lib.and_then(|l| l.buildings.as_ref().map(|b| (l, b))) {
            Some((l, b)) => Self {
                look: look.with_library(l),
                albedo: Some(b.albedo.clone()),
                normal: Some(b.normal.clone()),
            },
            None => Self {
                look,
                albedo: None,
                normal: None,
            },
        }
    }
}

impl MaterialExtension for BuildingExtension {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("building.wesl")).with_source("embedded"),
        )
    }
}

/// The shared house material and whether it has the library's textures.
#[derive(Resource)]
pub struct HouseMaterial {
    pub handle: Handle<BuildingMaterial>,
    pub textured: bool,
    pub night: f32,
}

/// The night factor that lights windows: 0 in daylight, 1 from the end of
/// civil twilight (sun 6° below the horizon), by the sun's elevation.
pub fn night_factor(sun_elevation_deg: f64) -> f32 {
    let t = ((2.0 - sun_elevation_deg) / 8.0).clamp(0.0, 1.0) as f32;
    t * t * (3.0 - 2.0 * t)
}

pub struct BuildingMaterialPlugin;

impl Plugin for BuildingMaterialPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "building.wesl");
        app.add_plugins(MaterialPlugin::<BuildingMaterial>::default())
            .add_systems(Update, sync_house_material);
    }
}

/// Makes the shared material, gives it the library's textures once they
/// are loaded, and lights the windows at night (changed only when the
/// night factor moves by 2 %: a changed material is uploaded again).
fn sync_house_material(
    mut commands: Commands,
    house: Option<ResMut<HouseMaterial>>,
    library: Option<Res<MaterialLibrary>>,
    view: Option<Res<crate::RideView>>,
    mut materials: ResMut<Assets<BuildingMaterial>>,
) {
    let styles = &*crate::house_style::STYLES;
    let night = view
        .as_deref()
        .and_then(|v| v.sky.as_ref())
        .map_or(0.0, |s| night_factor(s.sky.sun.elevation_deg));
    let textured = library.as_deref().is_some_and(|l| l.buildings.is_some());
    let Some(mut house) = house else {
        let mut ext = BuildingExtension::new(styles, library.as_deref());
        ext.look.set_night(night);
        let handle = materials.add(BuildingMaterial {
            base: StandardMaterial {
                base_color: Color::WHITE,
                perceptual_roughness: 0.9,
                ..default()
            },
            extension: ext,
        });
        commands.insert_resource(HouseMaterial {
            handle,
            textured,
            night,
        });
        return;
    };
    if house.textured != textured || (house.night - night).abs() > 0.02 {
        if let Some(mut m) = materials.get_mut(&house.handle) {
            if house.textured != textured {
                m.extension = BuildingExtension::new(styles, library.as_deref());
            }
            m.extension.look.set_night(night);
        }
        house.textured = textured;
        house.night = night;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn look_holds_every_window_style_and_layer() {
        let styles = &*crate::house_style::STYLES;
        let l = BuildingLook::new(styles);
        assert_eq!(l.mode.x, 0.0, "untextured without a library");
        assert!(l.mode.y < l.mode.z && l.facade.x < l.facade.y);
        // The garage style: no window, a 2.5 m door.
        let g = styles
            .windows
            .iter()
            .position(|w| w.id == "garage")
            .unwrap();
        assert_eq!(l.windows[g * 4].y, 0.0);
        assert_eq!(l.windows[g * 4 + 2].w, 2.5);
        // Shutter and shop-front flags.
        let a = styles
            .windows
            .iter()
            .position(|w| w.id == "alpine")
            .unwrap();
        assert_eq!(l.windows[a * 4 + 1].w, 1.0);
        let s = styles.windows.iter().position(|w| w.id == "shop").unwrap();
        assert_eq!(l.windows[s * 4 + 1].w, 2.0);
        // Glass has its colour in the means (no texture).
        assert!(l.means[Layer::Glass as usize].x < 0.1);
    }

    #[test]
    fn windows_light_up_after_sunset() {
        assert_eq!(night_factor(30.0), 0.0);
        assert_eq!(night_factor(2.0), 0.0);
        assert!(night_factor(-2.0) > 0.3 && night_factor(-2.0) < 0.7);
        assert_eq!(night_factor(-6.0), 1.0);
        assert_eq!(night_factor(-40.0), 1.0);
    }
}
