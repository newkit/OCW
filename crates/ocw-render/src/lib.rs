//! Bevy plugins for the Open Cycling World desktop client.
//!
//! This is one of only two crates allowed to depend on Bevy (spec §10). It
//! renders whatever the app puts into [`RideView`]; it knows nothing about
//! networking or devices.

pub mod camera;
pub mod console;
pub mod hud;
pub mod riders;
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
}

impl RideView {
    pub fn new(route: Route) -> Self {
        Self {
            route,
            riders: Vec::new(),
            elapsed_s: 0.0,
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
