//! Camera that follows the local rider.
//!
//! In chase mode the camera can be orbited around the rider: drag with the
//! left mouse button to turn and tilt, mouse wheel to zoom, `R` to reset.

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;

use crate::console::ConsoleState;
use crate::riders::rider_transform;
use crate::{RideView, world};

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CameraMode>()
            .init_resource::<CameraOrbit>()
            .add_systems(Startup, spawn_camera)
            .add_systems(Update, orbit_input)
            .add_systems(PostUpdate, follow.before(TransformSystems::Propagate));
    }
}

/// Camera perspective, switchable with the console command `camera.mode`.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CameraMode {
    /// Behind and above the rider.
    #[default]
    Chase,
    /// From the side of the road.
    Side,
    /// High above, looking down.
    Top,
}

impl std::str::FromStr for CameraMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "chase" => Ok(Self::Chase),
            "side" => Ok(Self::Side),
            "top" => Ok(Self::Top),
            other => Err(format!("unknown camera mode '{other}' (chase, side, top)")),
        }
    }
}

#[derive(Component)]
pub struct FollowCamera;

/// Orbit of the chase camera around the rider, relative to the riding
/// direction. The default reproduces the classic chase view.
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct CameraOrbit {
    /// Rotation around the rider (radians, 0 = behind, π = in front).
    pub yaw: f32,
    /// Elevation above the horizon (radians).
    pub pitch: f32,
    /// Distance from the rider in metres.
    pub distance: f32,
}

impl Default for CameraOrbit {
    fn default() -> Self {
        Self {
            yaw: 0.0,
            pitch: (2.6f32).atan2(6.5),
            distance: (6.5f32 * 6.5 + 2.6 * 2.6).sqrt(),
        }
    }
}

pub const MIN_PITCH: f32 = -0.05;
pub const MAX_PITCH: f32 = 1.45;
pub const MIN_DISTANCE: f32 = 2.5;
pub const MAX_DISTANCE: f32 = 80.0;
/// Radians per pixel of mouse movement.
const DRAG_SENSITIVITY: f32 = 0.005;

impl CameraOrbit {
    /// Applies a mouse drag (pixels) and wheel steps; keeps the limits.
    pub fn apply(&mut self, drag: Vec2, wheel_steps: f32) {
        self.yaw = (self.yaw - drag.x * DRAG_SENSITIVITY).rem_euclid(std::f32::consts::TAU);
        self.pitch = (self.pitch + drag.y * DRAG_SENSITIVITY).clamp(MIN_PITCH, MAX_PITCH);
        self.distance =
            (self.distance * (-wheel_steps * 0.12).exp()).clamp(MIN_DISTANCE, MAX_DISTANCE);
    }

    /// Eye and look-at target for a rider at `rider` riding along `forward`.
    pub fn eye_and_target(&self, rider: Vec3, forward: Vec3) -> (Vec3, Vec3) {
        let back = Quat::from_rotation_y(self.yaw) * -forward;
        let eye = rider + (back * self.pitch.cos() + Vec3::Y * self.pitch.sin()) * self.distance;
        // Behind the rider, look a bit ahead of them; when orbited to the
        // side or front, look at the rider.
        let ahead = self.yaw.cos().max(0.0) * 6.0;
        (eye, rider + Vec3::Y * 1.0 + forward * ahead)
    }
}

/// Mouse orbit (left button drag), zoom (wheel) and reset (`R`), unless the
/// console is open.
fn orbit_input(
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    console: Option<Res<ConsoleState>>,
    mut orbit: ResMut<CameraOrbit>,
) {
    if console.is_some_and(|c| c.is_open()) {
        return;
    }
    if keys.just_pressed(KeyCode::KeyR) {
        *orbit = CameraOrbit::default();
        return;
    }
    let drag = if buttons.pressed(MouseButton::Left) {
        motion.delta
    } else {
        Vec2::ZERO
    };
    let steps = match scroll.unit {
        MouseScrollUnit::Line => scroll.delta.y,
        MouseScrollUnit::Pixel => scroll.delta.y / 40.0,
    };
    if drag != Vec2::ZERO || steps != 0.0 {
        orbit.apply(drag, steps);
    }
}

fn spawn_camera(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 10.0, 20.0).looking_at(Vec3::ZERO, Vec3::Y),
        world::fog(),
        // Soft shadow edges (9-tap filter); less flicker than hardware 2×2.
        bevy::light::ShadowFilteringMethod::Gaussian,
        FollowCamera,
    ));
}

/// Lowest camera height above the ground (no looking through road or terrain).
pub const MIN_CAMERA_CLEARANCE_M: f32 = 0.6;

/// Lifts `eye` so it stays at least `MIN_CAMERA_CLEARANCE_M` above `ground`.
pub fn above_ground(eye: Vec3, ground: Option<f64>) -> Vec3 {
    match ground {
        Some(h) => eye.with_y(eye.y.max(h as f32 + MIN_CAMERA_CLEARANCE_M)),
        None => eye,
    }
}

fn follow(
    view: Res<RideView>,
    mode: Res<CameraMode>,
    orbit: Res<CameraOrbit>,
    ground: Option<Res<crate::Ground>>,
    time: Res<Time>,
    mut camera: Query<&mut Transform, With<FollowCamera>>,
) {
    let Some(local) = view.local() else { return };
    let Ok(mut cam) = camera.single_mut() else {
        return;
    };
    let rider = rider_transform(&view, local.state.distance, local.state.lateral_offset);
    let p = view.route.sample(local.state.distance);
    let forward = Vec3::new(p.direction[0] as f32, 0.0, p.direction[1] as f32);
    let right = Vec3::new(-forward.z, 0.0, forward.x);
    let (eye, target) = match *mode {
        CameraMode::Chase => orbit.eye_and_target(rider.translation, forward),
        CameraMode::Side => (
            rider.translation + right * 7.0 + Vec3::Y * 1.5,
            rider.translation + Vec3::Y * 0.9,
        ),
        CameraMode::Top => (
            rider.translation - forward * 30.0 + Vec3::Y * 60.0,
            rider.translation,
        ),
    };
    // Smooth the eye position; snap if far away (teleport, mode switch at start).
    let k = 1.0 - (-8.0 * time.delta_secs()).exp();
    cam.translation = if cam.translation.distance(eye) > 50.0 {
        eye
    } else {
        cam.translation.lerp(eye, k)
    };
    // Never below the ground, also while smoothing towards a low eye.
    let g = ground.and_then(|g| (g.0)(f64::from(cam.translation.x), f64::from(cam.translation.z)));
    cam.translation = above_ground(cam.translation, g);
    cam.look_at(target, Vec3::Y);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-4
    }

    #[test]
    fn camera_stays_above_the_ground() {
        let eye = Vec3::new(1.0, 100.2, 2.0);
        assert_eq!(
            above_ground(eye, Some(100.0)).y,
            100.0 + MIN_CAMERA_CLEARANCE_M
        );
        assert_eq!(above_ground(eye, Some(90.0)), eye);
        assert_eq!(above_ground(eye, None), eye);
    }

    #[test]
    fn default_orbit_is_the_classic_chase_view() {
        let (eye, target) = CameraOrbit::default().eye_and_target(Vec3::ZERO, Vec3::NEG_Z);
        assert!(close(eye, Vec3::new(0.0, 2.6, 6.5)), "{eye}");
        assert!(close(target, Vec3::new(0.0, 1.0, -6.0)), "{target}");
    }

    #[test]
    fn dragging_orbits_and_limits_apply() {
        let mut o = CameraOrbit::default();
        // Drag far to the left/right: half a turn puts the camera in front.
        o.apply(
            Vec2::new(-std::f32::consts::PI / DRAG_SENSITIVITY, 0.0),
            0.0,
        );
        let (eye, target) = o.eye_and_target(Vec3::ZERO, Vec3::NEG_Z);
        assert!(eye.z < -6.0, "{eye}");
        assert!(close(target, Vec3::Y), "looks at the rider: {target}");
        // Pitch and zoom are clamped.
        o.apply(Vec2::new(0.0, 1e6), 0.0);
        assert_eq!(o.pitch, MAX_PITCH);
        o.apply(Vec2::new(0.0, -1e6), 100.0);
        assert_eq!((o.pitch, o.distance), (MIN_PITCH, MIN_DISTANCE));
        o.apply(Vec2::ZERO, -100.0);
        assert_eq!(o.distance, MAX_DISTANCE);
    }

    #[test]
    fn camera_mode_parses() {
        assert_eq!("chase".parse::<CameraMode>(), Ok(CameraMode::Chase));
        assert_eq!("side".parse::<CameraMode>(), Ok(CameraMode::Side));
        assert_eq!("top".parse::<CameraMode>(), Ok(CameraMode::Top));
        assert!("Top".parse::<CameraMode>().is_err());
        assert!("".parse::<CameraMode>().is_err());
        assert_eq!(CameraMode::default(), CameraMode::Chase);
    }
}
