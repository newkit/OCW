//! Bevy plugins for the Open Cycling World desktop client.
//!
//! This is one of only two crates allowed to depend on Bevy (spec §10). It
//! renders whatever the app puts into [`RideView`]; it knows nothing about
//! networking or devices.

pub mod bridges;
pub mod building_material;
pub mod camera;
pub mod console;
pub mod event_hud;
pub mod far_terrain;
pub mod flight_world;
pub mod font;
pub mod frame_rate;
pub mod ground;
pub mod house_style;
pub mod houses;
pub mod hud;
pub mod inspect_hud;
pub mod junction;
pub mod materials;
pub mod media;
pub mod menu;
pub mod package;
pub mod region_look;
pub mod riders;
pub mod road_material;
pub mod see_through;
pub mod sky;
pub mod stamp;
pub mod terrain;
pub mod tunnel;
pub mod tunnel_cut;
pub mod unicorn;
pub mod voice;
#[cfg(test)]
mod wheel_contact;
pub mod workout;
pub mod world;

use std::collections::BTreeMap;
use std::sync::Arc;

use bevy::prelude::*;
use ocw_core::RiderId;
use ocw_sim::RiderState;
use ocw_world::{RidePath, Route, Track};

/// What the renderer should show this frame. Written by the app.
#[derive(Resource)]
pub struct RideView {
    /// The route (in free ride: where the ride started; riders ride their
    /// own paths, see [`RideView::paths`]).
    pub route: Route,
    /// Free ride: every rider's path through the road graph.
    pub paths: BTreeMap<RiderId, Arc<RidePath>>,
    /// Free ride on the road graph (no route centre line or start arch).
    pub free_ride: bool,
    /// The junction ahead of the local rider (free ride, spec §2.2.1).
    pub junction: Option<JunctionView>,
    /// A message for the rider (e.g. "Dead end in 800 m", spec §2.2.2).
    pub notice: Option<String>,
    /// Route ride: name and progress for the HUD ("Rheintal 3.2 / 12.4 km").
    pub route_status: Option<String>,
    pub riders: Vec<RiderView>,
    /// Session time in seconds.
    pub elapsed_s: f64,
    /// Device and trainer status for the HUD.
    pub status: String,
    /// Real terrain around the route (from a route file); `None` = terrain
    /// generated from the route's own elevation (test routes).
    pub terrain_base: Option<ocw_world::ElevationGrid>,
    /// A video recording is running: its length so far (s), for the "REC"
    /// indicator.
    pub recording_s: Option<f64>,
    /// A short app message shown under the REC indicator for a few seconds
    /// (e.g. "Video saved: <file>").
    pub app_message: Option<String>,
    /// The local rider's GPS position (lat °, lon °, altitude m), if the
    /// ride has a geographic origin (route files, world packages).
    pub position: Option<(f64, f64, f64)>,
    /// The current event (spec §2.3): countdown, standings, results.
    pub event: Option<EventView>,
    /// A time trial's ghost (the rider's best previous effort) on the
    /// local rider's track, drawn as a translucent rider.
    pub ghost: Option<RiderState>,
    /// The running workout's panel (spec §2.8), if any.
    pub workout: Option<ocw_sim::workout::WorkoutPanel>,
    /// The sky at the local rider (spec §14 Phase 9): local time, sun and
    /// light; `None` until the server's clock is known (fixed daylight).
    pub sky: Option<sky::SkyView>,
    /// God mode (spec §14 Phase 13): the local rider's flight (the flight
    /// camera and the flight HUD follow it); `None` while riding.
    pub flight: Option<FlightView>,
    /// Riders in god mode, drawn as unicorns (the local one too); their
    /// avatars are hidden once the transformation has turned them.
    pub unicorns: Vec<UnicornView>,
    /// God mode's pins (P, spec §14 Phase 13D): kept for the ride, drawn
    /// as glowing beams with their number.
    pub pins: Vec<PinView>,
}

/// A pin put in god mode (spec §14 Phase 13D).
#[derive(Debug, Clone, PartialEq)]
pub struct PinView {
    /// Its number (1, 2, …), shown over the beam.
    pub number: u32,
    /// Where it stands (local metres; the foot, on the ground).
    pub pos: [f64; 3],
}

/// The local rider's flight in god mode (spec §14 Phase 13B, 13D).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FlightView {
    /// The unicorn (local metres; x, y up, z).
    pub pos: [f64; 3],
    /// View yaw (rad, counter-clockwise from north) and pitch (rad, up).
    pub yaw: f64,
    pub pitch: f64,
    pub speed_mps: f64,
    /// Height above the ground below (m).
    pub height_m: f64,
    /// The wheel's base-speed scale (×0.25 … ×8).
    pub scale: f64,
    /// C: the view from the unicorn's head.
    pub first_person: bool,
    /// Where leaving (G) lands: the glowing ring on the road.
    pub landing: Option<[f64; 3]>,
    /// Gliding down / back (G, Shift + G).
    pub leaving: bool,
    /// The keys line of the flight HUD.
    pub keys: String,
    /// Horizontal velocity (m/s; x, z): the tiles stream ahead of it
    /// (spec §14 Phase 13D).
    pub velocity: [f64; 2],
    /// The inspection panel (I): position, latitude / longitude, tile,
    /// local time; empty = hidden.
    pub panel: Vec<String>,
    /// The crosshair (shown with the panel) and what it points at, one
    /// line per thing; `None` = hidden.
    pub crosshair: Option<Vec<String>>,
    /// The touch controls of the flight are shown (the last input was a
    /// touch, spec §3.2.2).
    pub touch: bool,
}

/// A unicorn to draw (spec §14 Phase 13C): a rider in god mode.
#[derive(Debug, Clone, PartialEq)]
pub struct UnicornView {
    pub id: RiderId,
    pub name: String,
    pub is_local: bool,
    /// The unicorn's body (local metres).
    pub pos: [f64; 3],
    /// Where it looks (rad): the body turns with the yaw.
    pub yaw: f64,
    pub pitch: f64,
    pub speed_mps: f64,
    /// Climb rate (m/s), turn rate (rad/s) and acceleration (m/s²): the
    /// wings flap and bank with them.
    pub climb_mps: f64,
    pub turn_rate: f64,
    pub accel: f64,
    /// Seconds since the rider turned into the unicorn (the 0.6 s
    /// transformation, the sparkle burst).
    pub age_s: f64,
    /// Landing / flying back: the wings fold.
    pub landing: bool,
    /// The rider's colours: the saddle cloth and the mane's first band.
    pub colour: ocw_core::Rgb,
}

impl UnicornView {
    /// Whether the transformation has turned the rider into the unicorn
    /// (the rider's avatar is hidden from then on).
    pub fn turned(&self) -> bool {
        self.age_s >= ocw_sim::god::TRANSFORM_S * 0.5
    }
}

impl RideView {
    pub fn new(route: Route) -> Self {
        Self {
            route,
            riders: Vec::new(),
            elapsed_s: 0.0,
            status: String::new(),
            terrain_base: None,
            paths: BTreeMap::new(),
            free_ride: false,
            junction: None,
            notice: None,
            recording_s: None,
            app_message: None,
            position: None,
            route_status: None,
            event: None,
            ghost: None,
            workout: None,
            sky: None,
            flight: None,
            unicorns: Vec::new(),
            pins: Vec::new(),
        }
    }

    pub fn local(&self) -> Option<&RiderView> {
        self.riders.iter().find(|r| r.is_local)
    }

    /// What rider `id` rides along: their free-ride path, or the route.
    pub fn track(&self, id: RiderId) -> &dyn Track {
        match self.paths.get(&id) {
            Some(p) => p.as_ref(),
            None => &self.route,
        }
    }
}

/// What the HUD shows of an event (spec §2.3); the app computes the
/// texts (Bevy-free), the HUD only lays them out.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EventView {
    /// The big centre text: "Get ready", "3", "2", "1", "GO!".
    pub countdown: Option<String>,
    /// "Race - 1 lap (4.0 km)".
    pub title: String,
    /// "P3/11  +4.2 s to Bot 4", "2.31 km to go", "Ghost -1.8 s".
    pub lines: Vec<String>,
    /// The results panel, until dismissed.
    pub results: Option<ResultsView>,
}

/// The results panel of a race or time trial.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResultsView {
    pub title: String,
    pub rows: Vec<ResultRow>,
    /// Under the table: the ghost comparison, how to close the panel.
    pub footer: Vec<String>,
}

/// One row of the results: "3", "Bot 4", "10:12.3", "+0:04.2".
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResultRow {
    pub position: String,
    pub name: String,
    pub time: String,
    pub gap: String,
    pub is_local: bool,
}

/// The junction ahead in free ride, for the overlay (spec §2.2.1).
#[derive(Debug, Clone, PartialEq)]
pub struct JunctionView {
    /// Distance to the junction (m; negative just after it).
    pub distance_m: f64,
    /// Exits from leftmost to rightmost.
    pub exits: Vec<ExitView>,
    /// The exit that will be taken, and the default one.
    pub selected: usize,
    pub default: usize,
    /// The choice is locked (within ~20 m of the junction).
    pub locked: bool,
    /// Route ride: the route's exit (spec §2.2.1: arrows show the route
    /// direction; another exit leaves the route).
    pub route: Option<usize>,
}

/// One exit of a junction in the overlay.
#[derive(Debug, Clone, PartialEq)]
pub struct ExitView {
    /// Turn angle (degrees, positive = right).
    pub angle_deg: f64,
    /// Street name or ref, or the road class.
    pub name: String,
    pub dead_end: bool,
    pub unpaved: bool,
    pub steep: bool,
}

/// Ground height lookup (terrain or world package) at local (x, z), for
/// keeping the camera above the ground (the terrain is camera-solid, see
/// [`camera::CameraInteraction`]).
#[derive(Resource, Clone)]
pub struct Ground(pub std::sync::Arc<dyn Fn(f64, f64) -> Option<f64> + Send + Sync>);

/// The drawn road surface lies this far above the road profile (the
/// track's height, which the conformed terrain meets: the same height would
/// z-fight). Every road surface (route ribbon, package road strips, tunnel
/// floor) is drawn at it, and the riders' tyres stand on it. The same as
/// the package's [`ocw_world::package::ROAD_LIFT_M`].
pub const ROAD_SURFACE_LIFT_M: f32 = ocw_world::package::ROAD_LIFT_M as f32;

/// One rider to draw.
#[derive(Debug, Clone)]
pub struct RiderView {
    pub id: RiderId,
    pub name: String,
    pub state: RiderState,
    pub power_w: f64,
    pub cadence_rpm: Option<f64>,
    pub heart_rate_bpm: Option<f64>,
    pub is_local: bool,
    /// The rider's bike (physics profile; picks the model, spec §2.5).
    pub bike: ocw_core::ProfileId,
    /// The rider's cosmetic look; `None` (an old server): colours by id.
    pub look: Option<ocw_core::RiderLook>,
}

/// All rendering plugins of the desktop client.
pub struct OcwRenderPlugins;

impl Plugin for OcwRenderPlugins {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            font::UiFontPlugin,
            see_through::SeeThroughPlugin,
            world::WorldPlugin,
            package::PackagePlugin,
            riders::RidersPlugin,
            camera::CameraPlugin,
            tunnel::TunnelPlugin,
            tunnel_cut::TunnelCutPlugin,
            hud::HudPlugin,
            event_hud::EventHudPlugin,
            junction::JunctionPlugin,
            menu::MenuPlugin,
            console::ConsolePlugin,
            workout::WorkoutPanelPlugin,
            voice::VoiceHudPlugin,
        ))
        .add_plugins((
            sky::SkyPlugin,
            materials::MaterialsPlugin,
            road_material::RoadMaterialPlugin,
            unicorn::UnicornPlugin,
            building_material::BuildingMaterialPlugin,
            inspect_hud::InspectHudPlugin,
        ));
    }
}

/// A ride switch (spec §3.2.1, M3): despawns the world of the old ride
/// (light, terrain, road, trees, streamed package tiles, avatars), then
/// spawns it again from the current [`RideView`] and
/// [`package::PackageWorld`] (the caller replaced them before). Camera,
/// HUD, menu and console stay. Without the render plugins (headless tests
/// with `MinimalPlugins`) only the despawning runs.
pub fn rebuild_scene(world: &mut World) {
    let old: Vec<Entity> = world
        .query_filtered::<Entity, With<world::SceneEntity>>()
        .iter(world)
        .collect();
    for e in old {
        if let Ok(e) = world.get_entity_mut(e) {
            e.despawn();
        }
    }
    package::reset_streaming(world);
    riders::reset_avatars(world);
    if let Some(mut o) = world.get_resource_mut::<camera::CameraObstacles>() {
        o.clear();
    }
    if let Some(mut o) = world.get_resource_mut::<flight_world::FlightObstacles>() {
        o.0.clear();
    }
    if let Some(mut t) = world.get_resource_mut::<tunnel::Tunnels>() {
        t.reset();
    }
    world.remove_resource::<Ground>();
    let renders = world.contains_resource::<Assets<Mesh>>()
        && world.contains_resource::<Assets<StandardMaterial>>()
        && world.contains_resource::<Assets<see_through::SeeThroughMaterial>>()
        && world.contains_resource::<RideView>();
    if renders {
        use bevy::ecs::system::RunSystemOnce;
        if let Err(e) = world.run_system_once(world::spawn_world) {
            error!("rebuilding the world: {e}");
        }
    }
}

/// Converts route coordinates (f64, metres) to a Bevy vector.
///
/// Phase 1 routes are small enough for f32. Large worlds use tile-local
/// coordinates and a floating origin (spec §4.5).
pub fn to_vec3(p: [f64; 3]) -> Vec3 {
    Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32)
}

/// Horizontal bounding box of a route: `([min_x, min_z], [max_x, max_z])`.
pub fn route_bounds(route: &Route) -> ([f64; 2], [f64; 2]) {
    route.points().iter().fold(
        ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]),
        |(min, max), p| {
            (
                [min[0].min(p[0]), min[1].min(p[2])],
                [max[0].max(p[0]), max[1].max(p[2])],
            )
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene(app: &mut App) -> Vec<Entity> {
        let w = app.world_mut();
        let mut q = w.query_filtered::<Entity, With<world::SceneEntity>>();
        let mut v: Vec<Entity> = q.iter(w).collect();
        v.sort();
        v
    }

    fn avatars(app: &mut App) -> usize {
        let w = app.world_mut();
        w.query::<&riders::Avatar>().iter(w).count()
    }

    /// The shadow-casting lights: (directional lights, those of them
    /// casting shadows, spot and point lights casting shadows).
    fn shadow_lights(app: &mut App) -> (usize, usize, usize) {
        let w = app.world_mut();
        let dirs: Vec<bool> = w
            .query::<&DirectionalLight>()
            .iter(w)
            .map(|d| d.shadow_maps_enabled)
            .collect();
        let spots = w
            .query::<&SpotLight>()
            .iter(w)
            .filter(|s| s.shadow_maps_enabled)
            .count();
        let points = w
            .query::<&PointLight>()
            .iter(w)
            .filter(|p| p.shadow_maps_enabled)
            .count();
        (
            dirs.len(),
            dirs.iter().filter(|s| **s).count(),
            spots + points,
        )
    }

    /// Regression (user report 2026-11-13, "two shadows of the rider"):
    /// one sun, the only light that casts shadows, at every time of day
    /// (with the bike lights on at dusk and night) and after ride
    /// switches; the bike lamps cast none.
    #[test]
    fn one_shadow_casting_light_at_every_time_and_after_switches() {
        let zone = ocw_sky::Zone::get("Europe/Vaduz").unwrap();
        let sky = |hour: u32, minute: u32| {
            let (ms, _) = zone.instant((2026, 11, 13), (hour, minute, 0)).unwrap();
            let sky = ocw_sky::Sky::at(ms, &zone, ocw_sky::zone::DEFAULT_PLACE, 0.1);
            sky::SkyView {
                hud: sky.hud_clock(),
                sky,
            }
        };
        let mut view = RideView::new(Route::test_loop());
        view.riders.push(RiderView {
            id: RiderId(1),
            name: "a".into(),
            state: RiderState::default(),
            power_w: 0.0,
            cadence_rpm: None,
            heart_rate_bpm: None,
            is_local: true,
            bike: ocw_core::ProfileId::Road,
            look: None,
        });
        view.sky = Some(sky(12, 0));
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .init_asset::<see_through::SeeThroughMaterial>()
            .insert_resource(view)
            .add_plugins(console::ConsoleCorePlugin)
            .add_plugins((world::WorldPlugin, riders::RidersPlugin, sky::SkyPlugin));
        app.world_mut().resource_mut::<sky::SkySettings>().mode = sky::SkyMode::Simple;
        app.update();
        app.update();
        assert_eq!(shadow_lights(&mut app), (1, 1, 0), "noon");
        for round in 0..3 {
            for (h, m) in [
                (7, 40),
                (8, 0),
                (10, 0),
                (16, 30),
                (16, 55),
                (17, 15),
                (22, 0),
                (12, 0),
            ] {
                app.world_mut().resource_mut::<RideView>().sky = Some(sky(h, m));
                app.update();
                app.update();
                let (dirs, casting, others) = shadow_lights(&mut app);
                assert_eq!(dirs, 1, "{h}:{m:02} round {round}: one sun");
                assert!(casting <= 1, "{h}:{m:02} round {round}");
                assert_eq!(others, 0, "{h}:{m:02}: no lamp casts shadows");
                let w = app.world_mut();
                let lamp_meshes = w
                    .query_filtered::<Has<bevy::light::NotShadowCaster>, (With<sky::BikeLamp>, With<Mesh3d>)>()
                    .iter(w)
                    .collect::<Vec<_>>();
                assert_eq!(lamp_meshes.len(), 2, "front and rear lamp");
                assert!(lamp_meshes.iter().all(|off| *off), "lamps cast no shadow");
            }
            // A ride switch: the old sun goes with the old world.
            app.world_mut().resource_mut::<RideView>().route = if round % 2 == 0 {
                Route::test_hills()
            } else {
                Route::test_loop()
            };
            rebuild_scene(app.world_mut());
            app.update();
            app.update();
            assert_eq!(shadow_lights(&mut app).0, 1, "after switch {round}");
        }
    }

    #[test]
    fn a_ride_switch_rebuilds_the_world_and_the_riders() {
        let mut view = RideView::new(Route::test_loop());
        view.riders.push(RiderView {
            id: RiderId(1),
            name: "a".into(),
            state: RiderState::default(),
            power_w: 0.0,
            cadence_rpm: None,
            heart_rate_bpm: None,
            is_local: true,
            bike: ocw_core::ProfileId::Road,
            look: None,
        });
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .init_asset::<see_through::SeeThroughMaterial>()
            .insert_resource(view)
            .add_plugins((world::WorldPlugin, riders::RidersPlugin));
        app.update();
        let first = scene(&mut app);
        assert!(first.len() > 100, "light, terrain, road, trees, arch");
        assert!(app.world().contains_resource::<Ground>());
        assert_eq!(avatars(&mut app), 1);
        // The app replaces the view (another route, a new session), then
        // rebuilds: nothing of the old world is left.
        app.world_mut().resource_mut::<RideView>().route = Route::test_hills();
        rebuild_scene(app.world_mut());
        let second = scene(&mut app);
        assert!(second.len() > 100);
        assert!(second.iter().all(|e| !first.contains(e)));
        assert_eq!(avatars(&mut app), 0, "the avatars are spawned afresh");
        app.update();
        assert_eq!(avatars(&mut app), 1);
        assert_eq!(scene(&mut app), second, "built once");
        // Without the render assets (headless app tests) only the despawning runs.
        let mut bare = App::new();
        bare.add_plugins(MinimalPlugins);
        rebuild_scene(bare.world_mut());
    }
}
