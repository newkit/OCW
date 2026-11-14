//! Sun, sky and night (spec §14 Phase 9, S1 / S2): draws the sky state the
//! app puts into [`crate::RideView::sky`] – the sun's direction, colour and
//! strength, the ambient light, the camera exposure (auto-exposure from
//! the light model, eased like an eye), the fog colour, Bevy's physically
//! based atmosphere (Hillaire 2020; or a plain sky colour,
//! `display.sky simple`), shadows following the sun; at night the same
//! directional light is the moon (bluish, by phase and height, soft
//! shadows when it is bright; S2), and the bike lights of the riders who
//! fitted lights in the workshop (decision Q3). The light model itself is
//! pure (`ocw_sky::light`, `ocw_sky::night`); moon disc, planets and Milky
//! Way are in [`crate::night_sky`], street lamps in
//! [`crate::street_lamps`].
//!
//! Without a sky state (`RideView::sky` is `None`, e.g. headless tests) the
//! scene keeps the fixed daylight it was built with.

use bevy::camera::Exposure;
use bevy::light::atmosphere::ScatteringMedium;
use bevy::light::{Atmosphere, GlobalAmbientLight, NotShadowCaster, SunDisk};
use bevy::pbr::{AtmosphereSettings, DistanceFog};
use bevy::prelude::*;
use clap::{Arg, Command};

use crate::RideView;
use crate::camera::FollowCamera;
use crate::console::{ConsoleAppExt, ConsoleCommand, Permission};
use crate::riders::{Avatar, GHOST_ID};

/// What the app computed for this frame (spec §14 Phase 9).
#[derive(Debug, Clone, PartialEq)]
pub struct SkyView {
    /// Local time, sun position and light at the rider.
    pub sky: ocw_sky::Sky,
    /// The HUD clock: "☼ 18:42", "☼ 18:42 x600", "◐ 21:40 paused".
    pub hud: String,
}

/// How the sky is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkyMode {
    /// Bevy's atmosphere (sky, sun disc, aerial perspective; needs compute
    /// shaders).
    Atmosphere,
    /// One sky colour from the light model (fallback for weak GPUs).
    Simple,
}

/// When the bike lights are on – on bikes with lights fitted in the
/// workshop (slot `lights`, decision Q3; bikes without have none).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LightsMode {
    /// From dusk to dawn (the sun below 1°) and in tunnels.
    Auto,
    On,
    Off,
}

/// Whether a bike with these parts (catalogue ids) has lights: a part of
/// the slot `lights` other than the base "no lights" (spec §14 Phase 4,
/// `lights.set`).
pub fn has_lights(parts: &[String]) -> bool {
    let cat = ocw_assets::shipped::catalogue();
    parts.iter().any(|id| {
        cat.get(id)
            .is_some_and(|p| p.slot == ocw_assets::parts::Slot::Lights && p.hearts > 0)
    })
}

/// Sky settings (Settings → Display, `display.night_brightness`,
/// `display.sky`, `display.bike_lights`).
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct SkySettings {
    /// Least displayed ambient light at night, share of daylight (0–1).
    pub night_floor: f32,
    pub mode: SkyMode,
    pub lights: LightsMode,
}

impl Default for SkySettings {
    fn default() -> Self {
        Self {
            night_floor: ocw_sky::light::DEFAULT_NIGHT_FLOOR,
            mode: SkyMode::Atmosphere,
            lights: LightsMode::Auto,
        }
    }
}

/// Marks the sun (the scene's directional light).
#[derive(Component)]
pub struct Sun;

/// Marks the atmosphere entity.
#[derive(Component)]
struct SkyAtmosphere;

/// The bike lights of a rider (children of the avatar).
#[derive(Component)]
struct BikeLamps;

/// One lamp: shown while the lights are on.
#[derive(Component)]
pub struct BikeLamp;

/// The current exposure (eased between frames).
#[derive(Resource, Default)]
struct ExposureState(Option<f32>);

pub struct SkyPlugin;

impl Plugin for SkyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SkySettings>()
            .init_resource::<ExposureState>()
            .add_systems(
                Update,
                (apply_sky, atmosphere, add_bike_lamps, bike_lamps).chain(),
            );
        register_commands(app);
    }
}

/// Unit vector towards the sun in the scene (x east, y up, z south: the
/// local metres of routes and packages) from azimuth (clockwise from
/// north) and elevation.
pub fn sun_direction(azimuth_deg: f64, elevation_deg: f64) -> Vec3 {
    let (az, el) = (azimuth_deg.to_radians(), elevation_deg.to_radians());
    Vec3::new(
        (el.cos() * az.sin()) as f32,
        el.sin() as f32,
        -(el.cos() * az.cos()) as f32,
    )
}

/// The sun light's transform: shining from `towards_sun` (its -Z axis
/// points away from the sun).
pub fn sun_transform(towards_sun: Vec3) -> Transform {
    let up = if towards_sun.y.abs() > 0.99 {
        Vec3::Z
    } else {
        Vec3::Y
    };
    Transform::default().looking_to(-towards_sun, up)
}

fn srgb(c: [f32; 3]) -> Color {
    Color::srgb(c[0], c[1], c[2])
}

fn linear(c: [f32; 3]) -> Color {
    Color::linear_rgb(c[0], c[1], c[2])
}

type CameraParts = (
    Entity,
    Option<&'static mut DistanceFog>,
    Option<&'static mut Exposure>,
);
/// Lamp mesh, white and red lamp materials.
type LampAssets = (
    Handle<Mesh>,
    Handle<StandardMaterial>,
    Handle<StandardMaterial>,
);

#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
fn apply_sky(
    view: Option<Res<RideView>>,
    settings: Res<SkySettings>,
    time: Res<Time>,
    mut exposure: ResMut<ExposureState>,
    ambient: Option<ResMut<GlobalAmbientLight>>,
    clear: Option<ResMut<ClearColor>>,
    mut suns: Query<
        (
            Entity,
            &mut DirectionalLight,
            &mut Transform,
            Option<&SunDisk>,
        ),
        With<Sun>,
    >,
    mut cameras: Query<CameraParts, With<FollowCamera>>,
    tunnels: Option<Res<crate::tunnel::Tunnels>>,
    mut commands: Commands,
) {
    let Some(sky) = view.as_ref().and_then(|v| v.sky.as_ref()) else {
        return;
    };
    // In a tunnel the hill above shades the rider (the terrain casts no
    // shadows in the renderer, only the thin tube would).
    let daylight = daylight_at_rider(view.as_deref(), tunnels.as_deref());
    let light = ocw_sky::light::light(sky.sky.sun.elevation_deg, settings.night_floor);
    let moon = &sky.sky.moon;
    let moonlight = ocw_sky::night::moonlight(moon.elevation_deg, moon.phase_angle_deg);
    let atmosphere = settings.mode == SkyMode::Atmosphere;
    let key = key_light(&sky.sky, &light, &moonlight, atmosphere);
    for (e, mut sun, mut transform, disk) in &mut suns {
        let t = sun_transform(key.towards);
        if transform.rotation != t.rotation {
            transform.rotation = t.rotation;
        }
        sun.illuminance = key.lux * daylight;
        sun.color = key.colour;
        sun.shadow_maps_enabled = key.shadows && daylight > 0.0;
        // The atmosphere draws a sun disc at the light: not at the moon
        // (it has its own disc, `night_sky`).
        let want = if key.moon {
            SunDisk {
                intensity: 0.0,
                ..SunDisk::EARTH
            }
        } else {
            SunDisk::EARTH
        };
        if disk.is_none_or(|d| d.intensity != want.intensity) {
            commands.entity(e).insert(want);
        }
    }
    if let Some(mut a) = ambient {
        a.brightness = light.ambient + moonlight.ambient;
        a.color = linear(light.ambient_rgb);
    }
    if let Some(mut c) = clear {
        let want = srgb(light.fog_srgb);
        if c.0 != want {
            c.0 = want;
        }
    }
    let ev = ocw_sky::light::adapt_ev(
        exposure.0.unwrap_or(light.ev100),
        light.ev100,
        time.delta_secs(),
    );
    exposure.0 = Some(ev);
    for (e, fog, cam_exposure) in &mut cameras {
        if let Some(mut fog) = fog {
            fog.color = srgb(light.fog_srgb);
        }
        match cam_exposure {
            Some(mut x) => x.ev100 = ev,
            None => {
                commands.entity(e).insert(Exposure { ev100: ev });
            }
        }
    }
}

/// The scene's one directional light: the sun, or at night the moon.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KeyLight {
    /// Unit vector towards the light (scene axes).
    pub towards: Vec3,
    pub lux: f32,
    pub colour: Color,
    pub shadows: bool,
    /// The moon is the light.
    pub moon: bool,
}

/// The key light for a sky: the sun while it is up (or brighter), else
/// the moon. With Bevy's atmosphere the light is given above the
/// atmosphere (it reddens and dims it itself).
pub fn key_light(
    sky: &ocw_sky::Sky,
    light: &ocw_sky::Light,
    moonlight: &ocw_sky::Moonlight,
    atmosphere: bool,
) -> KeyLight {
    let sun_up = sky.sun.elevation_deg > -1.0;
    let use_moon = !sun_up && moonlight.lux > 0.0 && moonlight.lux > light.sun_lux;
    if use_moon {
        let rgb = ocw_sky::night::MOON_RGB;
        let (lux, colour) = if atmosphere {
            (moonlight.lux_space, linear(rgb))
        } else {
            (moonlight.lux, linear(moonlight.rgb))
        };
        return KeyLight {
            towards: sun_direction(sky.moon.azimuth_deg, sky.moon.elevation_deg),
            lux,
            colour,
            shadows: moonlight.shadows,
            moon: true,
        };
    }
    let (lux, colour) = if atmosphere {
        (light.sun_lux_space * sun_up as u8 as f32, Color::WHITE)
    } else {
        (light.sun_lux, linear(light.sun_rgb))
    };
    KeyLight {
        towards: sun_direction(sky.sun.azimuth_deg, sky.sun.elevation_deg),
        lux,
        colour,
        shadows: light.shadows,
        moon: false,
    }
}

/// Spawns or removes Bevy's atmosphere with the sky mode.
fn atmosphere(
    mut commands: Commands,
    view: Option<Res<RideView>>,
    settings: Res<SkySettings>,
    media: Option<ResMut<Assets<ScatteringMedium>>>,
    existing: Query<Entity, With<SkyAtmosphere>>,
    cameras: Query<(Entity, Has<AtmosphereSettings>), With<FollowCamera>>,
) {
    let want =
        settings.mode == SkyMode::Atmosphere && view.as_ref().is_some_and(|v| v.sky.is_some());
    let Some(mut media) = media else { return };
    if want && existing.is_empty() {
        let medium = media.add(ScatteringMedium::default());
        commands.spawn((SkyAtmosphere, Atmosphere::earth(medium)));
    } else if !want {
        for e in &existing {
            commands.entity(e).despawn();
        }
    }
    for (cam, has) in &cameras {
        if want && !has {
            commands.entity(cam).insert(AtmosphereSettings::default());
        } else if !want && has {
            commands.entity(cam).remove::<AtmosphereSettings>();
        }
    }
}

/// Colours of the front and rear lamps (linear, above 1: they glow).
pub const LAMP_WHITE: LinearRgba = LinearRgba::rgb(6.0, 5.7, 5.1);
pub const LAMP_RED: LinearRgba = LinearRgba::rgb(5.0, 0.12, 0.06);

/// Front lamp (white) and rear lamp (red) positions on the bike, in the
/// avatar's frame (it faces -Z): at the handlebar and under the saddle.
pub const FRONT_LAMP: Vec3 = Vec3::new(0.0, 0.93, -0.60);
pub const REAR_LAMP: Vec3 = Vec3::new(0.0, 0.72, 0.42);

/// Gives every rider (not the ghost) a front and a rear lamp; the local
/// rider's front lamp also lights the road (a spot light).
fn add_bike_lamps(
    mut commands: Commands,
    avatars: Query<(Entity, &Avatar), Without<BikeLamps>>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
    mut cache: Local<Option<LampAssets>>,
) {
    if avatars.is_empty() {
        return;
    }
    let (Some(mut meshes), Some(mut materials)) = (meshes, materials) else {
        return;
    };
    let (mesh, white, red) = cache
        .get_or_insert_with(|| {
            // Unlit: the base colour is what shows (an unlit material
            // ignores the emissive – the lamps were black before S2).
            let lamp = |c: LinearRgba| StandardMaterial {
                base_color: c.into(),
                emissive: c,
                unlit: true,
                fog_enabled: false,
                ..default()
            };
            (
                meshes.add(Cuboid::new(0.07, 0.05, 0.05)),
                materials.add(lamp(LAMP_WHITE)),
                materials.add(lamp(LAMP_RED)),
            )
        })
        .clone();
    for (e, avatar) in &avatars {
        commands.entity(e).insert(BikeLamps);
        if avatar.id == GHOST_ID {
            continue;
        }
        for (at, material) in [(FRONT_LAMP, white.clone()), (REAR_LAMP, red.clone())] {
            commands.spawn((
                BikeLamp,
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material),
                Transform::from_translation(at),
                Visibility::Hidden,
                NotShadowCaster,
                ChildOf(e),
            ));
        }
        if avatar.is_local {
            commands.spawn((
                BikeLamp,
                SpotLight {
                    color: Color::srgb(1.0, 0.96, 0.9),
                    intensity: 12_000.0,
                    range: 45.0,
                    outer_angle: 0.42,
                    inner_angle: 0.22,
                    shadow_maps_enabled: false,
                    ..default()
                },
                // Ahead and a little down onto the road.
                Transform::from_translation(FRONT_LAMP)
                    .looking_to(Vec3::new(0.0, -0.16, -1.0), Vec3::Y),
                Visibility::Hidden,
                ChildOf(e),
            ));
        }
    }
}

/// Whether the bike lights are on.
pub fn lamps_on(mode: LightsMode, light: Option<&ocw_sky::Light>) -> bool {
    match mode {
        LightsMode::On => true,
        LightsMode::Off => false,
        LightsMode::Auto => light.is_some_and(|l| l.lights_on),
    }
}

/// Share of direct sunlight at the local rider: 0 deep in a tunnel
/// ([`crate::tunnel::daylight_share`]), else 1.
pub fn daylight_at_rider(view: Option<&RideView>, tunnels: Option<&crate::tunnel::Tunnels>) -> f32 {
    match (view.and_then(RideView::local), tunnels) {
        (Some(local), Some(t)) => crate::tunnel::daylight_share(&t.stretches, local.state.distance),
        _ => 1.0,
    }
}

fn bike_lamps(
    view: Option<Res<RideView>>,
    settings: Res<SkySettings>,
    tunnels: Option<Res<crate::tunnel::Tunnels>>,
    avatars: Query<&Avatar>,
    mut lamps: Query<(&mut Visibility, &ChildOf), With<BikeLamp>>,
) {
    let light = view
        .as_ref()
        .and_then(|v| v.sky.as_ref())
        .map(|s| s.sky.light);
    // In a tunnel the lights go on, as cyclists switch them on there.
    let in_tunnel = view.as_ref().is_some_and(|v| v.sky.is_some())
        && settings.lights == LightsMode::Auto
        && daylight_at_rider(view.as_deref(), tunnels.as_deref()) < 1.0;
    let on = in_tunnel || lamps_on(settings.lights, light.as_ref());
    // Only the bikes with lights fitted (Q3).
    let fitted: std::collections::HashSet<ocw_core::RiderId> = view
        .as_ref()
        .map(|v| {
            v.riders
                .iter()
                .filter(|r| has_lights(&r.parts))
                .map(|r| r.id)
                .collect()
        })
        .unwrap_or_default();
    for (mut v, parent) in &mut lamps {
        let has = avatars
            .get(parent.parent())
            .is_ok_and(|a| fitted.contains(&a.id));
        let want = if on && has {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *v != want {
            *v = want;
        }
    }
}

fn register_commands(app: &mut App) {
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("sky.night")
            .about("Night brightness: the least light at night in % of daylight (0-50; setting display.night_brightness)")
            .arg(Arg::new("percent").value_parser(clap::value_parser!(u32).range(0..=50))),
        |world, m| {
            let mut s = world.resource_mut::<SkySettings>();
            if let Some(p) = m.get_one::<u32>("percent") {
                s.night_floor = *p as f32 / 100.0;
            }
            Ok(format!("night brightness {:.0} %", s.night_floor * 100.0))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("sky.mode")
            .about("How the sky is drawn: atmosphere (physically based) or simple (one colour; setting display.sky)")
            .arg(Arg::new("mode").value_parser(["atmosphere", "simple"])),
        |world, m| {
            let mut s = world.resource_mut::<SkySettings>();
            match m.get_one::<String>("mode").map(String::as_str) {
                Some("atmosphere") => s.mode = SkyMode::Atmosphere,
                Some(_) => s.mode = SkyMode::Simple,
                None => {}
            }
            Ok(format!(
                "sky {}",
                if s.mode == SkyMode::Atmosphere { "atmosphere" } else { "simple" }
            ))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("sky.lights")
            .about("Bike lights: auto (dusk to dawn), on or off (setting display.bike_lights)")
            .arg(Arg::new("mode").value_parser(["auto", "on", "off"])),
        |world, m| {
            let mut s = world.resource_mut::<SkySettings>();
            match m.get_one::<String>("mode").map(String::as_str) {
                Some("auto") => s.lights = LightsMode::Auto,
                Some("on") => s.lights = LightsMode::On,
                Some(_) => s.lights = LightsMode::Off,
                None => {}
            }
            Ok(format!(
                "bike lights {}",
                match s.lights {
                    LightsMode::Auto => "auto",
                    LightsMode::On => "on",
                    LightsMode::Off => "off",
                }
            ))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("sky").about("Show the sun, the moon, the planets and the light at the rider"),
        |world, _| {
            let view = world.resource::<RideView>();
            let Some(s) = &view.sky else {
                return Err("no sky yet (waiting for the server's clock)".into());
            };
            let l = &s.sky.light;
            let planets: Vec<String> = s
                .sky
                .planets
                .iter()
                .filter(|p| p.elevation_deg > 0.0)
                .map(|p| {
                    format!(
                        "{} {:.0}°/{:.0}° mag {:.1}",
                        p.planet.name(),
                        p.azimuth_deg,
                        p.elevation_deg,
                        p.magnitude()
                    )
                })
                .collect();
            let pollution = world
                .get_resource::<crate::night_sky::LightPollution>()
                .map_or(0.0, |p| p.0);
            Ok(format!(
                "{} local, sun azimuth {:.1}° elevation {:.2}°, sun {:.0} lx, ambient {:.2}, EV100 {:.1}, lights {}; {}; planets up: {}; light pollution {:.0} %",
                s.sky.local.hh_mm(),
                s.sky.sun.azimuth_deg,
                s.sky.sun.elevation_deg,
                l.sun_lux,
                l.ambient,
                l.ev100,
                if l.lights_on { "on" } else { "off" },
                s.sky.moon_line(),
                if planets.is_empty() { "none".to_owned() } else { planets.join(", ") },
                pollution * 100.0
            ))
        },
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-4
    }

    #[test]
    fn the_sun_shines_from_its_azimuth() {
        // East at the horizon: +x; south high: -z is north, so +z.
        assert!(close(sun_direction(90.0, 0.0), Vec3::X));
        assert!(close(sun_direction(180.0, 0.0), Vec3::Z));
        assert!(close(sun_direction(0.0, 0.0), -Vec3::Z));
        assert!(close(sun_direction(123.0, 90.0), Vec3::Y));
        // The light travels away from the sun.
        let d = sun_direction(200.0, 35.0);
        let t = sun_transform(d);
        assert!(close(t.forward().as_vec3(), -d));
        let top = sun_transform(Vec3::Y);
        assert!(close(top.forward().as_vec3(), -Vec3::Y));
    }

    #[test]
    fn lamps_follow_the_mode_and_the_dark() {
        let night = ocw_sky::light::light(-10.0, 0.1);
        let day = ocw_sky::light::light(30.0, 0.1);
        assert!(lamps_on(LightsMode::Auto, Some(&night)));
        assert!(!lamps_on(LightsMode::Auto, Some(&day)));
        assert!(!lamps_on(LightsMode::Auto, None));
        assert!(lamps_on(LightsMode::On, Some(&day)));
        assert!(!lamps_on(LightsMode::Off, Some(&night)));
    }

    fn rider_view(id: u32, parts: &[&str]) -> crate::RiderView {
        crate::RiderView {
            id: ocw_core::RiderId(id),
            name: format!("r{id}"),
            state: Default::default(),
            power_w: 200.0,
            cadence_rpm: None,
            heart_rate_bpm: None,
            is_local: id == 1,
            bike: Default::default(),
            look: None,
            parts: parts.iter().map(|p| p.to_string()).collect(),
            crash: None,
            wobble: 0.0,
        }
    }

    /// Q3: only bikes with lights fitted in the workshop have them.
    #[test]
    fn lights_come_with_the_fitted_part() {
        assert!(has_lights(&["lights.set".into()]));
        assert!(!has_lights(&[]));
        assert!(!has_lights(&["lights.none.base".into()]));
        let mut view = RideView::new(ocw_world::Route::test_loop());
        view.sky = Some(sky_at("night"));
        view.riders.push(rider_view(1, &[]));
        view.riders.push(rider_view(2, &["lights.set"]));
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .insert_resource(view)
            .insert_resource(GlobalAmbientLight::default())
            .insert_resource(ClearColor::default())
            .add_plugins(crate::console::ConsoleCorePlugin)
            .add_plugins(SkyPlugin);
        let bare = app
            .world_mut()
            .spawn((
                Avatar {
                    id: ocw_core::RiderId(1),
                    is_local: true,
                },
                Transform::default(),
                Visibility::default(),
            ))
            .id();
        let fitted = app
            .world_mut()
            .spawn((
                Avatar {
                    id: ocw_core::RiderId(2),
                    is_local: false,
                },
                Transform::default(),
                Visibility::default(),
            ))
            .id();
        app.update();
        app.update();
        let w = app.world_mut();
        let lamps: Vec<(Visibility, Entity)> = w
            .query_filtered::<(&Visibility, &ChildOf), With<BikeLamp>>()
            .iter(w)
            .map(|(v, c)| (*v, c.parent()))
            .collect();
        assert!(lamps.iter().any(|(_, p)| *p == bare), "spawned, hidden");
        // Regression: the unlit lamps show their base colour, so it must
        // be the lamp's (it was black).
        let w = app.world_mut();
        let handles: Vec<Handle<StandardMaterial>> = w
            .query_filtered::<&MeshMaterial3d<StandardMaterial>, With<BikeLamp>>()
            .iter(w)
            .map(|m| m.0.clone())
            .collect();
        let mats = app.world().resource::<Assets<StandardMaterial>>();
        assert!(!handles.is_empty());
        for h in handles {
            let m = mats.get(&h).unwrap();
            assert!(
                m.unlit && m.base_color.to_linear().red > 1.0,
                "{:?}",
                m.base_color
            );
        }
        for (v, p) in lamps {
            let want = if p == fitted {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
            assert_eq!(v, want);
        }
    }

    /// At night the one directional light is the moon: bluish, shadows
    /// at full moon, no sun disc drawn at it; by day the sun.
    #[test]
    fn the_moon_is_the_key_light_at_night() {
        let zone = ocw_sky::Zone::get("Europe/Vaduz").unwrap();
        let place = ocw_sky::zone::DEFAULT_PLACE;
        // The night of the full moon of 24 Nov 2026.
        let (ms, _) = zone.instant((2026, 11, 25), (0, 0, 0)).unwrap();
        let sky = ocw_sky::Sky::at(ms, &zone, place, 0.1);
        let k = key_light(&sky, &sky.light, &sky.moonlight, false);
        assert!(k.moon && k.shadows, "{k:?}");
        assert!(k.lux > 0.5 && k.lux < 5.0, "{k:?}");
        let c = k.colour.to_linear();
        assert!(c.blue > c.red, "bluish");
        let up = sun_direction(sky.moon.azimuth_deg, sky.moon.elevation_deg);
        assert!((k.towards - up).length() < 1e-5 && k.towards.y > 0.5);
        // The new moon of 9 Dec: no moon, a dark key light.
        let (ms, _) = zone.instant((2026, 12, 9), (0, 0, 0)).unwrap();
        let dark = ocw_sky::Sky::at(ms, &zone, place, 0.1);
        let k = key_light(&dark, &dark.light, &dark.moonlight, true);
        assert!(!k.moon && k.lux == 0.0 && !k.shadows, "{k:?}");
        // Noon: the sun.
        let noon = sky_at("noon").sky;
        let k = key_light(&noon, &noon.light, &noon.moonlight, true);
        assert!(!k.moon && k.lux > 10_000.0);
    }

    fn sky_at(elevation_target: &str) -> SkyView {
        let zone = ocw_sky::Zone::get("Europe/Vaduz").unwrap();
        let (ms, _) = zone
            .instant(
                (2026, 6, 21),
                match elevation_target {
                    "noon" => (13, 23, 0),
                    _ => (23, 50, 0),
                },
            )
            .unwrap();
        let sky = ocw_sky::Sky::at(ms, &zone, ocw_sky::zone::DEFAULT_PLACE, 0.1);
        SkyView {
            hud: sky.hud_clock(),
            sky,
        }
    }

    /// Regression (user, 2026-11-13): deep in a tunnel the rider gets no
    /// direct sun at noon (the terrain above casts no shadows in the
    /// renderer; light leaked through the tube in squares), and the bike
    /// lamps are on; back outside the sun is full again.
    #[test]
    fn no_direct_sun_on_the_rider_in_a_tunnel() {
        let mut view = RideView::new(ocw_world::Route::test_loop());
        view.sky = Some(sky_at("noon"));
        view.riders.push(crate::RiderView {
            id: ocw_core::RiderId(1),
            name: "me".into(),
            state: ocw_sim::RiderState {
                distance: 1000.0,
                ..Default::default()
            },
            power_w: 200.0,
            cadence_rpm: None,
            heart_rate_bpm: None,
            is_local: true,
            bike: Default::default(),
            look: None,
            parts: vec!["lights.set".into()],
            crash: None,
            wobble: 0.0,
        });
        let mut tunnels = crate::tunnel::Tunnels::default();
        tunnels.stretches = vec![(800.0, 1200.0)];
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .insert_resource(view)
            .insert_resource(tunnels)
            .insert_resource(GlobalAmbientLight::default())
            .insert_resource(ClearColor::default())
            .add_plugins(crate::console::ConsoleCorePlugin)
            .add_plugins(SkyPlugin);
        app.world_mut().resource_mut::<SkySettings>().mode = SkyMode::Simple;
        let sun = app
            .world_mut()
            .spawn((Sun, DirectionalLight::default(), Transform::default()))
            .id();
        app.world_mut().spawn((
            Avatar {
                id: ocw_core::RiderId(1),
                is_local: true,
            },
            Transform::default(),
            Visibility::default(),
        ));
        app.update();
        app.update();
        let light = |app: &App| *app.world().get::<DirectionalLight>(sun).unwrap();
        assert_eq!(light(&app).illuminance, 0.0, "sun in the tunnel");
        assert!(!light(&app).shadow_maps_enabled);
        let w = app.world_mut();
        let mut q = w.query_filtered::<&Visibility, With<BikeLamp>>();
        let lamps: Vec<_> = q.iter(w).copied().collect();
        assert_eq!(lamps.len(), 3);
        assert!(lamps.iter().all(|v| *v == Visibility::Inherited));
        // Out of the tunnel: full noon sun with shadows, lamps off.
        app.world_mut().resource_mut::<RideView>().riders[0]
            .state
            .distance = 1500.0;
        app.update();
        assert!(light(&app).illuminance > 10_000.0);
        assert!(light(&app).shadow_maps_enabled);
        let w = app.world_mut();
        let mut q = w.query_filtered::<&Visibility, With<BikeLamp>>();
        assert!(q.iter(w).all(|v| *v == Visibility::Hidden));
    }

    /// Headless: the sky state drives sun, ambient light, exposure and the
    /// bike lamps of every rider but the ghost.
    #[test]
    fn the_sky_state_drives_light_exposure_and_lamps() {
        let mut view = RideView::new(ocw_world::Route::test_loop());
        view.sky = Some(sky_at("noon"));
        view.riders.push(rider_view(1, &["lights.set"]));
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .insert_resource(view)
            .insert_resource(GlobalAmbientLight::default())
            .insert_resource(ClearColor::default())
            .add_plugins(crate::console::ConsoleCorePlugin)
            .add_plugins(SkyPlugin);
        app.world_mut().resource_mut::<SkySettings>().mode = SkyMode::Simple;
        let sun = app
            .world_mut()
            .spawn((Sun, DirectionalLight::default(), Transform::default()))
            .id();
        let cam = app
            .world_mut()
            .spawn((FollowCamera, Transform::default()))
            .id();
        let rider = app
            .world_mut()
            .spawn((
                Avatar {
                    id: ocw_core::RiderId(1),
                    is_local: true,
                },
                Transform::default(),
                Visibility::default(),
            ))
            .id();
        let ghost = app
            .world_mut()
            .spawn((
                Avatar {
                    id: GHOST_ID,
                    is_local: false,
                },
                Transform::default(),
                Visibility::default(),
            ))
            .id();
        app.update();
        app.update();
        let noon_lux = app
            .world()
            .get::<DirectionalLight>(sun)
            .unwrap()
            .illuminance;
        assert!(noon_lux > 10_000.0, "{noon_lux}");
        let fwd = app
            .world()
            .get::<Transform>(sun)
            .unwrap()
            .forward()
            .as_vec3();
        assert!(fwd.y < -0.8 && fwd.z < 0.0, "from high in the south: {fwd}");
        let day_ev = app.world().get::<Exposure>(cam).unwrap().ev100;
        let lamps = |app: &mut App| {
            let w = app.world_mut();
            let mut q = w.query::<(&BikeLamp, &Visibility, &ChildOf)>();
            q.iter(w)
                .map(|(_, v, c)| (*v, c.parent()))
                .collect::<Vec<_>>()
        };
        let l = lamps(&mut app);
        assert_eq!(
            l.len(),
            3,
            "front, rear and the spot light of the local rider"
        );
        assert!(
            l.iter()
                .all(|(v, p)| *v == Visibility::Hidden && *p == rider)
        );
        assert!(!l.iter().any(|(_, p)| *p == ghost));
        // Night: no sun – only the dim light of the crescent moon (six
        // days old, low in the west) –, darker exposure target (brighter
        // EV) and lamps on.
        app.world_mut().resource_mut::<RideView>().sky = Some(sky_at("night"));
        app.update();
        let night_lux = app
            .world()
            .get::<DirectionalLight>(sun)
            .unwrap()
            .illuminance;
        assert!(night_lux < 1.0, "{night_lux}");
        assert!(
            !app.world()
                .get::<DirectionalLight>(sun)
                .unwrap()
                .shadow_maps_enabled
        );
        let night_ev = app.world().get::<Exposure>(cam).unwrap().ev100;
        assert!(night_ev < day_ev - 5.0, "{night_ev} vs {day_ev}");
        assert!(
            lamps(&mut app)
                .iter()
                .all(|(v, _)| *v == Visibility::Inherited)
        );
        let floor = app.world().resource::<GlobalAmbientLight>().brightness;
        assert!(floor > 0.0);
        // The console switches them off.
        app.world_mut()
            .resource_mut::<crate::console::ConsoleState>()
            .queue("sky.lights off");
        app.update();
        app.update();
        assert!(
            lamps(&mut app)
                .iter()
                .all(|(v, _)| *v == Visibility::Hidden)
        );
    }
}
