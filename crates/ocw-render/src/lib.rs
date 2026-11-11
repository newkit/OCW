//! Bevy plugins for the Open Cycling World desktop client.
//!
//! This is one of only two crates allowed to depend on Bevy (spec §10). It
//! renders whatever the app puts into [`RideView`]; it knows nothing about
//! networking or devices.

pub mod camera;
pub mod console;
pub mod hud;
pub mod riders;
pub mod terrain;
pub mod world;

use bevy::prelude::*;
use ocw_core::RiderId;
use ocw_sim::RiderState;
use ocw_world::Route;

/// What the renderer should show this frame. Written by the app.
#[derive(Resource)]
pub struct RideView {
    pub route: Route,
    pub riders: Vec<RiderView>,
    /// Session time in seconds.
    pub elapsed_s: f64,
    /// Device and trainer status for the HUD.
    pub status: String,
    /// Real terrain around the route (from a route file); `None` = terrain
    /// generated from the route's own elevation (test routes).
    pub terrain_base: Option<ocw_world::ElevationGrid>,
}

impl RideView {
    pub fn new(route: Route) -> Self {
        Self {
            route,
            riders: Vec::new(),
            elapsed_s: 0.0,
            status: String::new(),
            terrain_base: None,
        }
    }

    pub fn local(&self) -> Option<&RiderView> {
        self.riders.iter().find(|r| r.is_local)
    }
}

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
}

/// All rendering plugins of the desktop client.
pub struct OcwRenderPlugins;

impl Plugin for OcwRenderPlugins {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            world::WorldPlugin,
            riders::RidersPlugin,
            camera::CameraPlugin,
            hud::HudPlugin,
            console::ConsolePlugin,
        ));
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
