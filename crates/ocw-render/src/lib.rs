//! Bevy plugins for the Open Cycling World desktop client.
//!
//! This is one of only two crates allowed to depend on Bevy (spec §10). It
//! renders whatever the app puts into [`RideView`]; it knows nothing about
//! networking or devices.

pub mod camera;
pub mod console;
pub mod event_hud;
pub mod font;
pub mod frame_rate;
pub mod hud;
pub mod junction;
pub mod menu;
pub mod package;
pub mod riders;
pub mod see_through;
pub mod terrain;
pub mod tunnel;
pub mod tunnel_cut;
pub mod voice;
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
