//! The camera on the ride photographer's drone (spec §2.11, a dev tool for
//! videos, v4.45): the dev console's `camera.drone pov|follow|off`.
//!
//! - **POV** ([`DroneCamera::Pov`]): the window shows what the drone's
//!   camera sees – from just in front of its lens along its view, with the
//!   photo's field of view, so the last frame before the flash is the
//!   photo.
//! - **Follow** ([`DroneCamera::Follow`]): the drone from a little behind
//!   and to the side of its camera's heading, looking past it at what it
//!   films (the rider), smoothed; never under the ground or a building.
//!
//! The own photographer's drone is taken, else the first other rider's.
//! While no drone is out the normal camera stays (the switch waits for the
//! next drone). Runs after the normal camera, so it wins while a drone is
//! out.

use bevy::prelude::*;

use crate::RideView;
use crate::camera::{CameraObstacles, DEFAULT_FOV, FollowCamera, solid_height};
use crate::photographer::{DroneFigure, PHOTO_FOV_Y};

/// Where the window's camera is while a drone flies.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DroneCamera {
    /// The normal camera.
    #[default]
    Off,
    /// From the drone's camera.
    Pov,
    /// Following the drone from behind and aside.
    Follow,
}

impl DroneCamera {
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Pov => "pov",
            Self::Follow => "follow",
        }
    }
}

impl std::str::FromStr for DroneCamera {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "off" => Ok(Self::Off),
            "pov" => Ok(Self::Pov),
            "follow" => Ok(Self::Follow),
            other => Err(format!("unknown drone camera '{other}' (pov, follow, off)")),
        }
    }
}

/// The POV eye is this far in front of the drone's centre along its view
/// (m): in front of the lens and the gimbal, so none of the model is seen.
pub const POV_AHEAD_M: f32 = 0.45;
/// The follow camera: behind the drone's heading, to its right, above it
/// (m), and how far along the drone's view it looks (m).
pub const FOLLOW_BACK_M: f32 = 3.0;
pub const FOLLOW_SIDE_M: f32 = 1.2;
pub const FOLLOW_UP_M: f32 = 0.6;
pub const FOLLOW_LOOK_M: f32 = 2.0;
/// Smoothing of the follow camera's heading and eye (s; the eye lags
/// ~1.5 m at the drone's 13 m/s, its look-at point is not smoothed: the
/// drone stays in the picture).
pub const HEADING_TAU_S: f32 = 1.2;
pub const EYE_TAU_S: f32 = 0.12;
/// The follow camera snaps instead of gliding over a longer way (m).
pub const SNAP_M: f32 = 60.0;
/// At least this far above the ground or a solid object (m).
pub const CLEARANCE_M: f32 = 1.5;

/// The drone to film: the own, else the first other rider's.
pub fn drone_of(view: &RideView) -> Option<DroneFigure> {
    view.photographer
        .and_then(|p| p.drone)
        .or_else(|| view.photographers.iter().find_map(|p| p.drone))
}

/// The POV camera: eye and (unit) view direction.
pub fn pov(d: &DroneFigure) -> (Vec3, Vec3) {
    let look = crate::to_vec3(d.look).normalize_or(Vec3::NEG_Z);
    (crate::to_vec3(d.pos) + look * POV_AHEAD_M, look)
}

/// The horizontal heading of the drone's camera (unit), or `None` when it
/// looks straight down or up.
pub fn heading(look: Vec3) -> Option<Vec3> {
    let h = Vec3::new(look.x, 0.0, look.z);
    (h.length() > 0.05).then(|| h.normalize())
}

/// Turns the smoothed heading `prev` towards `goal` by the frame's `dt`;
/// the first frame takes the goal.
pub fn smooth_heading(prev: Option<Vec3>, goal: Vec3, dt: f32) -> Vec3 {
    let Some(p) = prev else {
        return goal;
    };
    let k = 1.0 - (-dt / HEADING_TAU_S).exp();
    p.lerp(goal, k).normalize_or(goal)
}

/// The follow camera's wanted eye and look-at point for a drone at `pos`
/// looking along `look`, its heading `h` (horizontal, unit).
pub fn follow_goal(pos: Vec3, look: Vec3, h: Vec3) -> (Vec3, Vec3) {
    let right = Vec3::new(-h.z, 0.0, h.x);
    let eye = pos - h * FOLLOW_BACK_M + right * FOLLOW_SIDE_M + Vec3::Y * FOLLOW_UP_M;
    (eye, pos + look.normalize_or(h) * FOLLOW_LOOK_M)
}

/// Glides `prev` to `goal` by the frame's `dt` (snaps when far or first).
pub fn glide(prev: Option<Vec3>, goal: Vec3, dt: f32) -> Vec3 {
    match prev {
        Some(p) if p.distance(goal) < SNAP_M => p.lerp(goal, 1.0 - (-dt / EYE_TAU_S).exp()),
        _ => goal,
    }
}

/// The follow camera's smoothed state.
#[derive(Debug, Default)]
pub(crate) struct FollowState {
    heading: Option<Vec3>,
    eye: Option<Vec3>,
}

#[allow(clippy::too_many_arguments)] // a Bevy system
pub(crate) fn drone_camera(
    mode: Res<DroneCamera>,
    view: Res<RideView>,
    ground: Option<Res<crate::Ground>>,
    obstacles: Res<CameraObstacles>,
    time: Res<Time>,
    mut camera: Query<(&mut Transform, &mut Projection), With<FollowCamera>>,
    mut state: Local<FollowState>,
) {
    let drone = match *mode {
        DroneCamera::Off => None,
        _ => drone_of(&view),
    };
    let Some(d) = drone else {
        *state = FollowState::default();
        return;
    };
    let Ok((mut cam, mut proj)) = camera.single_mut() else {
        return;
    };
    let fov = if *mode == DroneCamera::Pov {
        PHOTO_FOV_Y
    } else {
        DEFAULT_FOV
    };
    if let Projection::Perspective(p) = &mut *proj
        && (p.fov - fov).abs() > 1e-4
    {
        p.fov = fov;
    }
    if *mode == DroneCamera::Pov {
        let (eye, dir) = pov(&d);
        cam.translation = eye;
        cam.look_to(dir, Vec3::Y);
        *state = FollowState::default();
        return;
    }
    let dt = time.delta_secs();
    let pos = crate::to_vec3(d.pos);
    let look = crate::to_vec3(d.look).normalize_or(Vec3::NEG_Z);
    let goal_h = heading(look).or(state.heading).unwrap_or(Vec3::NEG_Z);
    let h = smooth_heading(state.heading, goal_h, dt);
    let (eye, target) = follow_goal(pos, look, h);
    let mut eye = glide(state.eye, eye, dt);
    if let Some(top) = solid_height(
        ground.as_deref(),
        &obstacles,
        f64::from(eye.x),
        f64::from(eye.z),
    ) {
        eye.y = eye.y.max(top as f32 + CLEARANCE_M);
    }
    *state = FollowState {
        heading: Some(h),
        eye: Some(eye),
    };
    cam.translation = eye;
    cam.look_at(target, Vec3::Y);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::photographer::PhotographerView;

    fn drone(pos: [f64; 3], look: [f64; 3]) -> DroneFigure {
        DroneFigure {
            pos,
            look,
            rotor: 1.0,
        }
    }

    #[test]
    fn modes_parse() {
        assert_eq!("pov".parse::<DroneCamera>(), Ok(DroneCamera::Pov));
        assert_eq!("follow".parse::<DroneCamera>(), Ok(DroneCamera::Follow));
        assert_eq!("off".parse::<DroneCamera>(), Ok(DroneCamera::Off));
        assert!("drone".parse::<DroneCamera>().is_err());
        for m in [DroneCamera::Off, DroneCamera::Pov, DroneCamera::Follow] {
            assert_eq!(m.label().parse::<DroneCamera>(), Ok(m));
        }
    }

    #[test]
    fn the_own_drone_first_else_another_riders() {
        let mut v = RideView::new(ocw_world::Route::test_loop());
        assert!(drone_of(&v).is_none());
        let other = drone([5.0, 10.0, 0.0], [1.0, 0.0, 0.0]);
        v.photographers = vec![
            PhotographerView::default(),
            PhotographerView {
                drone: Some(other),
                ..default()
            },
        ];
        assert_eq!(drone_of(&v), Some(other));
        let own = drone([0.0, 8.0, 0.0], [0.0, 0.0, -1.0]);
        v.photographer = Some(PhotographerView {
            drone: Some(own),
            ..default()
        });
        assert_eq!(drone_of(&v), Some(own));
    }

    #[test]
    fn pov_is_in_front_of_the_lens_along_the_view() {
        let d = drone([10.0, 20.0, 30.0], [0.0, -3.0, -4.0]);
        let (eye, dir) = pov(&d);
        assert!((dir - Vec3::new(0.0, -0.6, -0.8)).length() < 1e-5);
        assert!((eye.distance(Vec3::new(10.0, 20.0, 30.0)) - POV_AHEAD_M).abs() < 1e-4);
        assert!((eye - Vec3::new(10.0, 20.0, 30.0)).dot(dir) > 0.0);
    }

    #[test]
    fn heading_is_horizontal_and_none_straight_down() {
        let h = heading(Vec3::new(3.0, -2.0, 4.0)).unwrap();
        assert_eq!(h.y, 0.0);
        assert!((h - Vec3::new(0.6, 0.0, 0.8)).length() < 1e-5);
        assert!(heading(Vec3::NEG_Y).is_none());
    }

    #[test]
    fn follow_is_behind_aside_and_above_looking_past_the_drone() {
        let pos = Vec3::new(0.0, 10.0, 0.0);
        let h = Vec3::NEG_Z;
        let (eye, target) = follow_goal(pos, Vec3::new(0.0, -0.5, -1.0), h);
        // Behind (+z when heading −z), to the right (+x), above.
        assert!((eye.z - FOLLOW_BACK_M).abs() < 1e-5);
        assert!((eye.x - FOLLOW_SIDE_M).abs() < 1e-5);
        assert!((eye.y - (10.0 + FOLLOW_UP_M)).abs() < 1e-5);
        // Looks past the drone, the way it films: the drone is between.
        let to_drone = (pos - eye).normalize();
        let to_target = (target - eye).normalize();
        assert!(to_drone.dot(to_target) > 0.9);
        assert!(eye.distance(target) > eye.distance(pos));
    }

    #[test]
    fn heading_and_eye_glide_and_snap_when_far() {
        let h = smooth_heading(Some(Vec3::NEG_Z), Vec3::X, 1.0 / 30.0);
        assert!((h.length() - 1.0).abs() < 1e-5);
        assert!(h.x > 0.0 && h.x < 0.1, "turns a little per frame: {h}");
        assert_eq!(smooth_heading(None, Vec3::X, 0.1), Vec3::X);
        let e = glide(Some(Vec3::ZERO), Vec3::new(1.0, 0.0, 0.0), 1.0 / 30.0);
        assert!(e.x > 0.0 && e.x < 0.3, "{e}");
        // Following a drone at 13 m/s: the eye lags less than 2 m.
        let mut eye = Some(Vec3::ZERO);
        let mut goal = Vec3::ZERO;
        for _ in 0..90 {
            goal.x += 13.0 / 30.0;
            eye = Some(glide(eye, goal, 1.0 / 30.0));
        }
        let lag = goal.x - eye.unwrap().x;
        assert!(lag > 0.5 && lag < 2.0, "lag {lag}");
        let far = Vec3::new(SNAP_M + 1.0, 0.0, 0.0);
        assert_eq!(glide(Some(Vec3::ZERO), far, 1.0 / 30.0), far);
        assert_eq!(glide(None, far, 0.0), far);
    }
}
