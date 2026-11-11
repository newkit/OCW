//! Bevy plugins for the Open Cycling World desktop client.
//!
//! This is one of only two crates allowed to depend on Bevy (spec §10). It
//! renders whatever the app puts into [`RideView`]; it knows nothing about
//! networking or devices.

pub mod camera;
pub mod console;
pub mod hud;
pub mod package;
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

/// Ground height lookup (terrain or world package) at local (x, z), for
/// keeping the camera above the ground.
#[derive(Resource, Clone)]
pub struct Ground(pub std::sync::Arc<dyn Fn(f64, f64) -> Option<f64> + Send + Sync>);

/// A tree as an obstacle (for the camera): a cone crown over a trunk.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TreeCone {
    /// Foot of the trunk.
    pub base: Vec3,
    /// Height of the crown's bottom and tip above `base`.
    pub crown_bottom: f32,
    pub crown_top: f32,
    /// Crown radius at its bottom.
    pub radius: f32,
}

impl TreeCone {
    /// Height of the crown's surface above the point (x, z) (world), if the
    /// crown covers it: the cone's height at that distance from the trunk.
    pub fn surface_at(&self, x: f32, z: f32) -> Option<f32> {
        let d = ((x - self.base.x).powi(2) + (z - self.base.z).powi(2)).sqrt();
        (d < self.radius).then(|| {
            self.base.y
                + self.crown_bottom
                + (self.crown_top - self.crown_bottom) * (1.0 - d / self.radius)
        })
    }

    /// Crown radius at height `y` (world), `None` outside the crown.
    pub fn radius_at(&self, y: f32) -> Option<f32> {
        let h = y - self.base.y;
        (self.crown_bottom..=self.crown_top)
            .contains(&h)
            .then(|| self.radius * (self.crown_top - h) / (self.crown_top - self.crown_bottom))
    }
}

/// The trees currently in the world, by tile (route mode: one entry).
#[derive(Resource, Debug, Default)]
pub struct Trees(pub std::collections::HashMap<(i32, i32), Vec<TreeCone>>);

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
        app.init_resource::<Trees>().add_plugins((
            world::WorldPlugin,
            package::PackagePlugin,
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
