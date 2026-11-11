//! Camera that follows the local rider.
//!
//! In chase mode the camera can be orbited around the rider: drag with the
//! left mouse button to turn and tilt, mouse wheel to zoom, `R` to reset.

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;

use crate::console::ConsoleState;
use crate::riders::avatar_transform;
use crate::{RideView, TreeCone, world};

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

/// Clearance kept between the line of sight and the terrain.
pub const SIGHT_CLEARANCE_M: f32 = 0.5;
/// Highest lift of the eye to see over terrain (m).
const MAX_SIGHT_LIFT_M: f32 = 40.0;

/// Lifts `eye` until the line of sight to `target` clears `ground` (with
/// [`SIGHT_CLEARANCE_M`]): a hill or embankment between the camera and the
/// rider would otherwise cut through the view. Raising the eye by Δ raises
/// the line at fraction t of the way by Δ·(1 − t), so the lift needed is the
/// largest (ground + clearance − line) / (1 − t) along the line; the last
/// part next to the rider is skipped (the rider stands on that ground).
pub fn clear_line_of_sight(
    eye: Vec3,
    target: Vec3,
    ground: &dyn Fn(f64, f64) -> Option<f64>,
) -> Vec3 {
    const SAMPLES: usize = 24;
    let mut lift = 0.0f32;
    for i in 1..SAMPLES {
        let t = i as f32 / SAMPLES as f32;
        if t > 0.85 {
            break;
        }
        let p = eye.lerp(target, t);
        if let Some(h) = ground(f64::from(p.x), f64::from(p.z)) {
            let need = (h as f32 + SIGHT_CLEARANCE_M - p.y) / (1.0 - t);
            lift = lift.max(need);
        }
    }
    eye + Vec3::Y * lift.clamp(0.0, MAX_SIGHT_LIFT_M)
}

/// Gap kept between the camera and a tree crown (m).
pub const TREE_CLEARANCE_M: f32 = 0.5;

/// Pushes `eye` sideways out of any tree crown it is in (the camera would
/// otherwise look out from inside a tree).
pub fn out_of_trees<'a>(mut eye: Vec3, trees: impl IntoIterator<Item = &'a TreeCone>) -> Vec3 {
    for t in trees {
        let (dx, dz) = (eye.x - t.base.x, eye.z - t.base.z);
        if dx.abs() > t.radius + TREE_CLEARANCE_M || dz.abs() > t.radius + TREE_CLEARANCE_M {
            continue;
        }
        let Some(r) = t.radius_at(eye.y) else {
            continue;
        };
        let r = r + TREE_CLEARANCE_M;
        let d = (dx * dx + dz * dz).sqrt();
        if d < r {
            let dir = if d > 1e-4 {
                Vec2::new(dx, dz) / d
            } else {
                Vec2::X
            };
            eye.x = t.base.x + dir.x * r;
            eye.z = t.base.z + dir.y * r;
        }
    }
    eye
}

#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
fn follow(
    view: Res<RideView>,
    mode: Res<CameraMode>,
    orbit: Res<CameraOrbit>,
    ground: Option<Res<crate::Ground>>,
    trees: Option<Res<crate::Trees>>,
    time: Res<Time>,
    mut camera: Query<&mut Transform, With<FollowCamera>>,
    mut smoothed_target: Local<Option<Vec3>>,
) {
    let Some(local) = view.local() else { return };
    let Ok(mut cam) = camera.single_mut() else {
        return;
    };
    // Follow the avatar itself: on the terrain off the road (wild mode) and
    // along the rider's own heading, not the road's.
    let rider = avatar_transform(&view, &local.state, ground.as_deref());
    let p = view.route.sample(local.state.distance);
    let road = Vec3::new(p.direction[0] as f32, 0.0, p.direction[1] as f32);
    // Positive heading = to the right; Bevy turns left for a positive Y rotation.
    let forward = Quat::from_rotation_y(-local.state.heading as f32) * road;
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
    let mut target = target;
    if let Some(g) = ground.as_deref() {
        // Never below the ground, also while smoothing towards a low eye,
        // and nothing of the terrain between the eye and the rider (the
        // rider, not the look-ahead point: uphill that is in the slope).
        let h = (g.0)(f64::from(cam.translation.x), f64::from(cam.translation.z));
        cam.translation = above_ground(cam.translation, h);
        let rider_head = rider.translation + Vec3::Y * 1.2;
        // Trees between the eye and the rider count like terrain.
        let (lo, hi) = (
            cam.translation.min(rider_head),
            cam.translation.max(rider_head),
        );
        let near_trees: Vec<TreeCone> = trees
            .as_deref()
            .map(|t| {
                t.0.values()
                    .flatten()
                    .filter(|c| {
                        c.base.x + c.radius >= lo.x
                            && c.base.x - c.radius <= hi.x
                            && c.base.z + c.radius >= lo.z
                            && c.base.z - c.radius <= hi.z
                    })
                    .copied()
                    .collect()
            })
            .unwrap_or_default();
        let obstacles = |x: f64, z: f64| {
            let terrain = (g.0)(x, z);
            let tree = near_trees
                .iter()
                .filter_map(|c| c.surface_at(x as f32, z as f32))
                .fold(None, |m: Option<f32>, h| Some(m.map_or(h, |m| m.max(h))));
            match (terrain, tree) {
                (Some(a), Some(b)) => Some(a.max(f64::from(b))),
                (a, b) => a.or(b.map(f64::from)),
            }
        };
        cam.translation = clear_line_of_sight(cam.translation, rider_head, &obstacles);
        // Not inside a tree crown (riding through a forest).
        if let Some(trees) = trees.as_deref() {
            cam.translation = out_of_trees(cam.translation, trees.0.values().flatten());
            let h = (g.0)(f64::from(cam.translation.x), f64::from(cam.translation.z));
            cam.translation = above_ground(cam.translation, h);
        }
        // The look-ahead point stays above the ground too, but not so far
        // above the rider that the rider leaves the picture.
        if let Some(h) = (g.0)(f64::from(target.x), f64::from(target.z)) {
            let rider_y = rider.translation.y;
            target.y = target.y.max(h as f32 + 1.0).min(rider_y + 3.0);
        }
    }
    // Smooth the look-at point like the eye (bumpy ground ahead would make
    // the view jerk); snap when far (teleport).
    let target = match *smoothed_target {
        Some(t) if t.distance(target) < 50.0 => t.lerp(target, k),
        _ => target,
    };
    *smoothed_target = Some(target);
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
    fn camera_sees_over_a_hill_between_it_and_the_rider() {
        // Rider at x = 0 on flat ground (0 m); a 5 m ridge at x 3 … 5 m
        // between the rider and the eye at x = 8, 2 m high.
        let ridge = |x: f64, _z: f64| Some(if (3.0..=5.0).contains(&x) { 5.0 } else { 0.0 });
        let target = Vec3::new(0.0, 1.0, 0.0);
        let eye = clear_line_of_sight(Vec3::new(8.0, 2.0, 0.0), target, &ridge);
        assert!(eye.y > 2.0 && eye.x == 8.0 && eye.z == 0.0, "{eye}");
        // Now the whole line is above the ridge (+ clearance).
        for i in 1..20 {
            let p = eye.lerp(target, i as f32 / 20.0);
            if (3.0..=5.0).contains(&p.x) {
                assert!(p.y >= 5.0 + SIGHT_CLEARANCE_M - 0.3, "{p}");
            }
        }
        // Nothing in the way: unchanged.
        let flat = |_: f64, _: f64| Some(0.0);
        assert_eq!(
            clear_line_of_sight(Vec3::new(8.0, 2.0, 0.0), target, &flat),
            Vec3::new(8.0, 2.0, 0.0)
        );
        // Outside the terrain data: unchanged.
        let none = |_: f64, _: f64| None;
        assert_eq!(
            clear_line_of_sight(Vec3::new(8.0, 2.0, 0.0), target, &none),
            Vec3::new(8.0, 2.0, 0.0)
        );
    }

    #[test]
    fn camera_is_pushed_out_of_tree_crowns() {
        let tree = TreeCone {
            base: Vec3::new(10.0, 0.0, 0.0),
            crown_bottom: 2.0,
            crown_top: 9.0,
            radius: 2.2,
        };
        // In the crown, 1 m from the trunk at 2 m: out to radius + clearance.
        let eye = out_of_trees(Vec3::new(11.0, 2.0, 0.0), [&tree]);
        assert!(
            (eye.x - (10.0 + 2.2 + TREE_CLEARANCE_M)).abs() < 1e-5,
            "{eye}"
        );
        assert_eq!((eye.y, eye.z), (2.0, 0.0));
        // Higher up the crown is narrower.
        let eye = out_of_trees(Vec3::new(10.0, 8.0, 0.5), [&tree]);
        let r = tree.radius_at(8.0).unwrap() + TREE_CLEARANCE_M;
        assert!((eye.z - r).abs() < 1e-5, "{eye}");
        // The crown's surface: highest at the trunk, its bottom at the rim.
        assert_eq!(tree.surface_at(10.0, 0.0), Some(9.0));
        assert!((tree.surface_at(12.2 - 1e-4, 0.0).unwrap() - 2.0).abs() < 1e-3);
        assert_eq!(tree.surface_at(13.0, 0.0), None);
        // Under the crown (trunk height) and above the tip: unchanged.
        let below = Vec3::new(11.0, 1.0, 0.0);
        assert_eq!(out_of_trees(below, [&tree]), below);
        let above = Vec3::new(10.2, 9.5, 0.0);
        assert_eq!(out_of_trees(above, [&tree]), above);
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
