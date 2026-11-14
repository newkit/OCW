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
    /// The weather (spec §14 Phase 9, S4; [`crate::weather::surface_uniform`]):
    /// x wetness, y puddles, z snow cover, w wind sway.
    pub weather: Vec4,
}

/// Distances (m) over which the textures and the windows fade out.
pub const TEXTURE_FADE_M: [f32; 2] = [90.0, 320.0];
pub const WINDOW_FADE_M: [f32; 2] = [420.0, 900.0];
/// Emissive brightness of the brightest lit window (physical units, scaled
/// by the camera's exposure like the sky; [`window_light`] dims each
/// window below it). 60 with every window the same warm white read as
/// harsh white panes (spec §14 Phase 9, 9I-S2).
pub const LIT_WINDOW: f32 = 26.0;

fn pcg(v: u32) -> u32 {
    let s = v.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    let w = ((s >> ((s >> 28) + 4)) ^ s).wrapping_mul(277_803_737);
    (w >> 22) ^ w
}

fn unit(v: u32) -> f32 {
    (v >> 8) as f32 / 16_777_216.0
}

/// The share of lit windows of the building with light seed `seed` at
/// night (`houses::Paint::light_seed`): about one in eight buildings is
/// dark, the others light 12–52 % of their windows.
pub fn lit_share(seed: u32) -> f32 {
    let b = unit(pcg(seed.wrapping_mul(3).wrapping_add(1)));
    if b < 0.12 {
        0.0
    } else {
        0.12 + 0.4 * (b - 0.12) / 0.88
    }
}

/// The light behind window `(cell, storey)` of the building with light
/// seed `seed` at night (the twin of `building.wesl`'s `window_light`,
/// the same integer hash): linear rgb × relative brightness (0.3–1 of
/// [`LIT_WINDOW`]), `None` when the room is dark. Mostly warm incandescent
/// light, some warm white, a few cool LED / TV rooms – never white.
pub fn window_light(seed: u32, cell: i32, storey: i32) -> Option<[f32; 3]> {
    let share = lit_share(seed);
    let key = pcg(seed
        ^ pcg(
            (cell as u32).wrapping_mul(2_654_435_761) ^ pcg((storey as u32).wrapping_add(40_503))
        ));
    if unit(key) >= share {
        return None;
    }
    let hue = unit(pcg(key ^ 0x9E37_79B9));
    let c = if hue >= 0.86 {
        [0.62, 0.74, 1.0]
    } else if hue >= 0.62 {
        [1.0, 0.74, 0.5]
    } else {
        [1.0, 0.56, 0.26]
    };
    let k = 0.3 + 0.7 * unit(pcg(key ^ 0x85EB_CA6B));
    Some(c.map(|v| v * k))
}

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
            weather: Vec4::ZERO,
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

    /// Regression (spec §14 Phase 9, 9I-S2): lit windows were all the same
    /// harsh warm white at 60; now per building a share of them, warm and
    /// varied, dimmer, some buildings dark – deterministic.
    #[test]
    fn lit_windows_are_warm_varied_and_deterministic() {
        let (mut lit, mut total, mut dark_buildings) = (0, 0, 0);
        let (mut warm, mut cool) = (0, 0);
        let mut bright = (f32::MAX, f32::MIN);
        for seed in 0..800u32 {
            let mut any = false;
            for storey in 0..4 {
                for cell in 0..6 {
                    total += 1;
                    let Some(c) = window_light(seed, cell, storey) else {
                        continue;
                    };
                    assert_eq!(window_light(seed, cell, storey), Some(c), "deterministic");
                    any = true;
                    lit += 1;
                    let max = c[0].max(c[1]).max(c[2]);
                    let min = c[0].min(c[1]).min(c[2]);
                    assert!(min < 0.8 * max, "never white: {c:?}");
                    if c[0] > c[2] {
                        warm += 1;
                    } else {
                        cool += 1;
                    }
                    bright = (bright.0.min(max), bright.1.max(max));
                }
            }
            if lit_share(seed) == 0.0 {
                assert!(!any);
                dark_buildings += 1;
            }
        }
        let share = lit as f32 / total as f32;
        assert!((0.2..0.4).contains(&share), "{share}");
        assert!((60..140).contains(&dark_buildings), "{dark_buildings}");
        assert!(warm > 3 * cool && cool > 0, "{warm} warm, {cool} cool");
        assert!(bright.0 < 0.4 && bright.1 > 0.95, "{bright:?}");
        // Neighbouring windows differ (not whole floors at once).
        let row: Vec<bool> = (0..40).map(|c| window_light(7, c, 1).is_some()).collect();
        assert!(row.iter().any(|l| *l) && row.iter().any(|l| !*l), "{row:?}");
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
