//! Weather rendering (spec §14 Phase 9, S4): draws the session weather the
//! app puts into [`crate::RideView::weather`] –
//!
//! - the light: overcast is flat (no sun, no hard shadows, a brighter grey
//!   sky light), storms are dark, the sun goes behind passing heaps
//!   (`sky.rs` applies [`ocw_weather::light::cloud_light`] and
//!   [`clouds::sun_shade`]);
//! - clouds: a sky dome around the camera whose shader intersects the view
//!   ray with the cloud layer and draws layered noise per cloud type,
//!   self-shadowed towards the sun, with the lightning lighting them up
//!   ([`clouds`]);
//! - fog: the weather's visibility (fog plus precipitation, Koschmieder)
//!   replaces the distance fog when it is shorter (`far_terrain.rs`), and
//!   the sky behind dense fog is the fog's colour;
//! - rain, drizzle, snow, sleet and hail around the camera: one mesh of
//!   seeded quads whose vertex shader places and wraps them in a box
//!   moving with the camera, streaks along the motion, wind-driven
//!   ([`precipitation`]);
//! - wet roads (darker, glossier, puddles in the hollows), snow on the
//!   ground, roofs and tree crowns, trees swaying in the wind: uniforms of
//!   the existing road, ground, building and tree materials
//!   ([`surface_uniform`], [`sync_surfaces`]);
//! - spray behind the wheels on wet roads ([`spray`]);
//! - lightning: the flash lights the scene and the clouds, near strikes
//!   show their channel ([`bolt`]); the thunder is the app's sound.
//!
//! The weather maths is pure (`ocw_weather`); this module only draws it.
//! Without a weather (`RideView::weather` is `None`) nothing changes.

pub mod bolt;
pub mod clouds;
pub mod precipitation;
pub mod spray;

use bevy::asset::embedded_asset;
use bevy::pbr::MaterialPlugin;
use bevy::prelude::*;
use clap::{Arg, Command};
use ocw_weather::{Ground, Strike, WeatherState};

use crate::RideView;
use crate::console::{ConsoleAppExt, ConsoleCommand, Permission};

/// The weather at the local rider this frame (spec §14 Phase 9, S4).
#[derive(Debug, Clone, PartialEq)]
pub struct WeatherView {
    pub state: WeatherState,
    /// Road wetness, puddles, snow cover.
    pub ground: Ground,
    /// The wind now, with the gust (m/s), and the gust envelope (0–1).
    pub wind_mps: f32,
    pub gust: f32,
    /// Strikes of the last minute.
    pub strikes: Vec<StrikeView>,
    /// For the HUD: "rain 9 °C".
    pub summary: String,
    /// The server's session time now (s).
    pub session_s: f64,
}

/// A lightning strike with its flash in the app's time
/// ([`RideView::elapsed_s`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrikeView {
    pub strike: Strike,
    pub flash_at_s: f64,
}

/// How much of the weather is drawn (console `weather.fx`; for frame-time
/// comparisons and weak GPUs).
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct WeatherFx {
    pub clouds: bool,
    /// Particle budget (0: no precipitation drawn).
    pub particles: u32,
    pub spray: bool,
    pub bolts: bool,
}

impl Default for WeatherFx {
    fn default() -> Self {
        Self {
            clouds: true,
            particles: ocw_weather::particles::BUDGET,
            spray: true,
            bolts: true,
        }
    }
}

/// The light the weather effects are lit with, written by the sky every
/// frame (`sky::apply_sky`).
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct SceneLight {
    /// Towards the key light (sun or moon; scene axes).
    pub towards: Vec3,
    /// The key light above the clouds (lux) and its colour (linear).
    pub key_lux: f32,
    pub key_rgb: Vec3,
    /// The ambient light (brightness and colour) under the clouds.
    pub ambient: f32,
    pub ambient_rgb: Vec3,
    /// The fog colour (linear) as the distance fog uses it.
    pub fog: Vec3,
    /// The lightning flash now (0 – ~1) and where it comes from.
    pub flash: f32,
    pub flash_towards: Vec3,
}

impl Default for SceneLight {
    fn default() -> Self {
        Self {
            towards: Vec3::new(0.3, 0.8, 0.5).normalize(),
            key_lux: ocw_sky::light::DAY_SUN_LUX,
            key_rgb: Vec3::ONE,
            ambient: ocw_sky::light::DAY_AMBIENT,
            ambient_rgb: Vec3::ONE,
            fog: Vec3::new(0.35, 0.55, 0.85),
            flash: 0.0,
            flash_towards: Vec3::Y,
        }
    }
}

/// The ambient light a full flash adds (renderer units; the daylight
/// ambient is 600).
pub const FLASH_AMBIENT: f32 = 1800.0;

/// The brightest flash of the recent strikes at app time `now_s`, and the
/// direction towards it (scene axes).
pub fn flash_now(w: &WeatherView, now_s: f64) -> (f32, Vec3) {
    let mut best = (0.0, Vec3::Y);
    for s in &w.strikes {
        let f = ocw_weather::lightning::flash((now_s - s.flash_at_s) as f32, &s.strike);
        if f > best.0 {
            let o = s.strike.offset();
            let d = Vec3::new(o[0], 1500.0, o[1]).normalize_or(Vec3::Y);
            best = (f, d);
        }
    }
    best
}

/// The weather uniform of the surface materials: x road wetness, y
/// puddles, z snow cover, w how much the trees sway (0–1, from the wind
/// now). Quantized (1/256), so the materials change only when it shows.
pub fn surface_uniform(w: Option<&WeatherView>) -> Vec4 {
    let Some(w) = w else { return Vec4::ZERO };
    let q = |x: f32| (x.clamp(0.0, 1.0) * 256.0).round() / 256.0;
    Vec4::new(
        q(w.ground.wetness()),
        q(w.ground.puddles()),
        q(w.ground.snow_cover()),
        q(sway(w.wind_mps)),
    )
}

/// How much trees sway at `wind_mps` (0 calm – 1 a gale): a breeze moves
/// the crowns a little, a storm bends them.
pub fn sway(wind_mps: f32) -> f32 {
    (wind_mps / 28.0).clamp(0.0, 1.0).powf(1.3)
}

/// The visibility (m) the distance fog should have for the weather: fog
/// and precipitation, and the cloud itself when the rider is inside the
/// layer (low cloud on a climb); `None` without a weather.
pub fn fog_visibility(view: &RideView, clouds: Option<&clouds::CloudState>) -> Option<f32> {
    let w = view.weather.as_ref()?;
    let mut vis = w.state.visibility_m();
    if let (Some(r), Some(c)) = (clouds::rider_position(view), clouds) {
        let base = c.ground_y.unwrap_or(r.y) + clouds::layer_shape(&w.state).0;
        if let Some(v) = clouds::in_cloud_visibility(&w.state, r.y + 1.5 - base) {
            vis = vis.min(v);
        }
    }
    Some(vis)
}

/// The wind (m/s, scene x / z) now.
pub fn wind_vector(w: &WeatherView) -> Vec2 {
    let v = w.state.wind_velocity(w.wind_mps);
    Vec2::new(v[0], v[1])
}

pub struct WeatherPlugin;

impl Plugin for WeatherPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "clouds.wesl");
        embedded_asset!(app, "precipitation.wesl");
        embedded_asset!(app, "spray.wesl");
        app.add_plugins((
            MaterialPlugin::<clouds::CloudMaterial>::default(),
            MaterialPlugin::<precipitation::PrecipMaterial>::default(),
            MaterialPlugin::<spray::SprayMaterial>::default(),
        ))
        .init_resource::<WeatherFx>()
        .init_resource::<SceneLight>()
        .init_resource::<clouds::CloudState>()
        .add_systems(
            Update,
            (
                sync_surfaces,
                clouds::update,
                precipitation::update,
                spray::update,
                bolt::update,
            ),
        );
        register_commands(app);
    }
}

/// Hands the weather uniform to every road, terrain, building and tree
/// material (also to ones created later, e.g. streamed tiles), and the
/// wind to the trees.
#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per asset type
pub fn sync_surfaces(
    view: Option<Res<RideView>>,
    roads: Option<ResMut<Assets<crate::road_material::RoadMaterial>>>,
    terrain: Option<ResMut<Assets<crate::ground::TerrainMaterial>>>,
    buildings: Option<ResMut<Assets<crate::building_material::BuildingMaterial>>>,
    trees: Option<ResMut<Assets<crate::see_through::SeeThroughMaterial>>>,
) {
    let w = view.as_ref().and_then(|v| v.weather.as_ref());
    let u = surface_uniform(w);
    macro_rules! sync {
        ($assets:expr, $field:ident) => {
            if let Some(mut assets) = $assets {
                let stale: Vec<_> = assets
                    .iter()
                    .filter(|(_, m)| m.extension.$field.weather != u)
                    .map(|(id, _)| id)
                    .collect();
                for id in stale {
                    if let Some(mut m) = assets.get_mut(id) {
                        m.extension.$field.weather = u;
                    }
                }
            }
        };
    }
    sync!(roads, look);
    sync!(terrain, look);
    sync!(buildings, look);
    // Trees: the uniform and the wind direction (the sway's phase runs on
    // the shader's clock).
    if let Some(mut trees) = trees {
        let dir = w.map_or(Vec2::X, |w| {
            let v = wind_vector(w);
            if v.length() > 1e-3 {
                v.normalize()
            } else {
                Vec2::X
            }
        });
        let want = Vec4::new(dir.x, u.w, dir.y, u.z);
        let stale: Vec<_> = trees
            .iter()
            .filter(|(_, m)| m.extension.wind != want)
            .map(|(id, _)| id)
            .collect();
        for id in stale {
            if let Some(mut m) = trees.get_mut(id) {
                m.extension.wind = want;
            }
        }
    }
}

fn register_commands(app: &mut App) {
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("weather.fx")
            .about("What of the weather is drawn: weather.fx [clouds|particles|spray|bolts] [on|off|N] (particles: the budget, 0-100000)")
            .arg(Arg::new("what").value_parser(["clouds", "particles", "spray", "bolts", "all"]))
            .arg(Arg::new("value")),
        |world, m| {
            let mut fx = world.resource_mut::<WeatherFx>();
            let what = m.get_one::<String>("what").map(String::as_str);
            let value = m.get_one::<String>("value").map(String::as_str);
            let on = |v: Option<&str>| -> Result<bool, String> {
                match v {
                    Some("on") | None => Ok(true),
                    Some("off") => Ok(false),
                    Some(o) => Err(format!("on or off, not {o}")),
                }
            };
            match what {
                Some("clouds") => fx.clouds = on(value)?,
                Some("spray") => fx.spray = on(value)?,
                Some("bolts") => fx.bolts = on(value)?,
                Some("particles") => {
                    fx.particles = match value {
                        Some("off") => 0,
                        Some("on") | None => ocw_weather::particles::BUDGET,
                        Some(n) => n
                            .parse::<u32>()
                            .map_err(|_| format!("a number, on or off, not {n}"))?
                            .min(precipitation::MAX_PARTICLES),
                    }
                }
                Some(_) => {
                    let all = on(value)?;
                    *fx = if all {
                        WeatherFx::default()
                    } else {
                        WeatherFx {
                            clouds: false,
                            particles: 0,
                            spray: false,
                            bolts: false,
                        }
                    };
                }
                None => {}
            }
            Ok(format!(
                "weather fx: clouds {}, particles {}, spray {}, bolts {}",
                if fx.clouds { "on" } else { "off" },
                fx.particles,
                if fx.spray { "on" } else { "off" },
                if fx.bolts { "on" } else { "off" },
            ))
        },
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_weather::presets::preset;

    pub(crate) fn view_of(name: &str) -> WeatherView {
        let state = preset(name).unwrap();
        WeatherView {
            ground: Ground::settled(&state),
            wind_mps: state.wind_mps,
            gust: 0.0,
            strikes: Vec::new(),
            summary: state.hud(),
            session_s: 0.0,
            state,
        }
    }

    #[test]
    fn the_surface_uniform_carries_wetness_snow_and_sway() {
        assert_eq!(surface_uniform(None), Vec4::ZERO);
        let clear = surface_uniform(Some(&view_of("clear")));
        assert_eq!((clear.x, clear.y, clear.z), (0.0, 0.0, 0.0));
        let rain = surface_uniform(Some(&view_of("heavy-rain")));
        assert_eq!(rain.x, 1.0);
        assert!(rain.y > 0.9);
        let snow = surface_uniform(Some(&view_of("heavy-snow")));
        assert_eq!(snow.z, 1.0);
        let storm = surface_uniform(Some(&view_of("storm")));
        assert!(storm.w > 4.0 * clear.w && storm.w <= 1.0);
        assert!(sway(0.0) == 0.0 && sway(40.0) == 1.0);
    }

    #[test]
    fn the_flash_follows_the_nearest_bright_strike() {
        let mut w = view_of("thunderstorm");
        assert_eq!(flash_now(&w, 10.0).0, 0.0);
        let strike = Strike {
            id: 1,
            at_s: 0.0,
            distance_m: 900.0,
            bearing_deg: 90.0,
            intensity: 1.0,
            seed: 0,
        };
        w.strikes.push(StrikeView {
            strike,
            flash_at_s: 10.0,
        });
        let (f, dir) = flash_now(&w, 10.0);
        assert!(f > 0.5, "{f}");
        assert!(dir.x > 0.4 && dir.y > 0.0, "from the east: {dir}");
        assert_eq!(flash_now(&w, 9.0).0, 0.0, "not yet");
        assert!(flash_now(&w, 12.0).0 < 1e-3, "over");
    }

    /// Headless: the materials get the weather uniform, also ones added
    /// later; the console switches the effects.
    #[test]
    fn materials_follow_the_weather_and_the_console_switches_effects() {
        let mut view = RideView::new(ocw_world::Route::test_loop());
        view.weather = Some(view_of("heavy-snow"));
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Assets<crate::see_through::SeeThroughMaterial>>()
            .init_resource::<Assets<crate::road_material::RoadMaterial>>()
            .insert_resource(view)
            .add_plugins(crate::console::ConsoleCorePlugin)
            .init_resource::<WeatherFx>()
            .add_systems(Update, sync_surfaces);
        register_commands(&mut app);
        let tree = app
            .world_mut()
            .resource_mut::<Assets<crate::see_through::SeeThroughMaterial>>()
            .add(crate::see_through::see_through(StandardMaterial::default()));
        app.update();
        let m = app
            .world()
            .resource::<Assets<crate::see_through::SeeThroughMaterial>>()
            .get(&tree)
            .unwrap()
            .extension
            .wind;
        assert_eq!(m.w, 1.0, "snow on the crowns");
        assert!(m.y > 0.0, "swaying");
        let road = app
            .world_mut()
            .resource_mut::<Assets<crate::road_material::RoadMaterial>>()
            .add(crate::road_material::RoadMaterial {
                base: StandardMaterial::default(),
                extension: crate::road_material::RoadExtension::new(None, &Default::default()),
            });
        app.world_mut().resource_mut::<RideView>().weather = Some(view_of("heavy-rain"));
        app.update();
        let r = app
            .world()
            .resource::<Assets<crate::road_material::RoadMaterial>>()
            .get(&road)
            .unwrap()
            .extension
            .look
            .weather;
        assert_eq!(r.x, 1.0, "a wet road");
        app.world_mut()
            .resource_mut::<crate::console::ConsoleState>()
            .queue("weather.fx particles 5000");
        app.update();
        app.update();
        assert_eq!(app.world().resource::<WeatherFx>().particles, 5000);
        app.world_mut()
            .resource_mut::<crate::console::ConsoleState>()
            .queue("weather.fx all off");
        app.update();
        app.update();
        assert!(!app.world().resource::<WeatherFx>().clouds);
    }
}
