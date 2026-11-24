//! Eye views: where each eye is (in the tracking space) and what it sees.
//!
//! A runtime locates the two eyes itself (`xrLocateViews`), so the IPD is
//! the distance between the two eye positions it reports – the headset's
//! lens setting. Without a runtime (`--vr-sim`, tests) [`sim_views`]
//! builds the eyes from a head pose: half the IPD to each side.

use glam::{Quat, Vec3};

use crate::Pose;
use crate::projection::Fov;

/// One eye: its pose in the tracking space and its field of view.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EyeView {
    pub pose: Pose,
    pub fov: Fov,
}

/// Left eye first.
pub type StereoViews = [EyeView; 2];

/// The default IPD (m) when nothing is known: the adult median.
pub const DEFAULT_IPD_M: f32 = 0.063;

/// The distance between the eyes (m).
pub fn ipd(views: &StereoViews) -> f32 {
    views[0].pose.position.distance(views[1].pose.position)
}

/// The head between the eyes: the midpoint, the left eye's orientation
/// (the runtimes give both eyes the head's orientation, canted displays
/// aside).
pub fn head(views: &StereoViews) -> Pose {
    Pose::new(
        views[0].pose.orientation,
        0.5 * (views[0].pose.position + views[1].pose.position),
    )
}

/// The offset of each eye from the head (head frame): ∓IPD/2 along +X.
pub fn eye_offsets(ipd_m: f32) -> [Vec3; 2] {
    let h = 0.5 * ipd_m.max(0.0);
    [Vec3::new(-h, 0.0, 0.0), Vec3::new(h, 0.0, 0.0)]
}

/// Simulated eyes for a head pose (both eyes look where the head looks).
pub fn sim_views(head: &Pose, ipd_m: f32, fov_left: &Fov) -> StereoViews {
    let off = eye_offsets(ipd_m);
    let eye = |i: usize, fov: Fov| EyeView {
        pose: Pose::new(head.orientation, head.transform_point(off[i])),
        fov,
    };
    [eye(0, *fov_left), eye(1, fov_left.mirrored())]
}

/// An Oculus Quest (first generation)-like left eye: about 94° across and
/// 104° high, more to the outside and down (approximate; the runtime's
/// own values win on a real headset).
pub const QUEST1_LEFT_FOV: Fov = Fov {
    left: -0.907,
    right: 0.733,
    up: 0.873,
    down: -0.942,
};

/// A slow, smooth look around for `--vr-sim` and the screenshots: a
/// seated rider glancing left and right and a little up and down, `t`
/// seconds in (no head bob: the position stays at the anchor).
pub fn sim_head(t: f32) -> Pose {
    let yaw = 0.25 * (0.4 * t).sin();
    let pitch = -0.08 + 0.05 * (0.27 * t + 1.0).sin();
    Pose::new(
        Quat::from_rotation_y(yaw) * Quat::from_rotation_x(pitch),
        Vec3::ZERO,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sim_eyes_sit_half_the_ipd_apart() {
        let head_pose = Pose::new(Quat::from_rotation_y(0.4), Vec3::new(1.0, 1.6, 2.0));
        let v = sim_views(&head_pose, 0.064, &QUEST1_LEFT_FOV);
        assert!((ipd(&v) - 0.064).abs() < 1e-6);
        let h = head(&v);
        assert!((h.position - head_pose.position).length() < 1e-6);
        // The left eye is to the head's left, both look the same way.
        let right = head_pose.right();
        assert!((v[1].pose.position - v[0].pose.position).dot(right) > 0.063);
        assert_eq!(v[0].pose.orientation, v[1].pose.orientation);
        // The right eye's field of view is the left's mirrored.
        assert_eq!(v[1].fov.right, -QUEST1_LEFT_FOV.left);
        assert!(QUEST1_LEFT_FOV.is_valid());
        let (h_deg, v_deg) = {
            let (a, b) = QUEST1_LEFT_FOV.extent();
            (a.to_degrees(), b.to_degrees())
        };
        assert!((90.0..100.0).contains(&h_deg) && (100.0..110.0).contains(&v_deg));
        assert_eq!(eye_offsets(-1.0), [Vec3::ZERO, Vec3::ZERO]);
    }

    #[test]
    fn the_simulated_head_looks_around_smoothly() {
        let mut prev = sim_head(0.0);
        for i in 1..600 {
            let p = sim_head(i as f32 / 60.0);
            // No more than ~1° per frame at 60 Hz, no position change.
            assert!(p.orientation.angle_between(prev.orientation) < 0.02);
            assert_eq!(p.position, Vec3::ZERO);
            prev = p;
        }
    }
}
