//! VR for Open Cycling World (spec §14 Phase 11, milestone X1): the pure
//! parts, shared by the desktop app (OpenXR over Oculus Link / SteamVR /
//! Monado), the standalone headset build (Phase 14, Android OpenXR) and
//! later WebXR.
//!
//! Conventions are OpenXR's, which are Bevy's too: right-handed, +Y up,
//! −Z forward, metres, radians. A [`Pose`] maps its child frame into its
//! parent frame (`parent_from_child`).
//!
//! - [`projection`]: per-eye field of view → the asymmetric, infinite,
//!   reverse-Z projection Bevy renders with, and its frustum corners.
//! - [`view`]: eye views, IPD, the simulated eyes of `--vr-sim`.
//! - [`recentre`]: the yaw-only recentre and the advice after a long
//!   head offset.
//! - [`anchor`]: the rider's first-person anchor on the bike (horizon
//!   lock) and how the tracked views become world cameras.
//! - [`hud`]: the fixed HUD panel in front of the rider.
//! - [`session`]: the session state machine (idle → ready → focused →
//!   stopping → lost, reconnect) over a mockable [`session::XrRuntime`].
//! - [`frame`]: the frame loop's order of calls and its timing statistics
//!   (frame time, latency, missed frames).
//! - [`swapchain`]: the colour format choice and the eye texture size.
//! - [`headset`]: headset profiles (Oculus Quest 1: 1440 × 1600 per eye,
//!   72 Hz) and the 72–90 Hz budget.
//! - [`mock`]: an in-memory runtime for tests (no headset, no OpenXR).

pub mod anchor;
pub mod frame;
pub mod headset;
pub mod hud;
pub mod mock;
pub mod projection;
pub mod recentre;
pub mod session;
pub mod swapchain;
pub mod view;

pub use glam::{Mat4, Quat, Vec3};

/// A rigid transform: `parent_from_child` (rotation, then translation).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub orientation: Quat,
    pub position: Vec3,
}

impl Default for Pose {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Pose {
    pub const IDENTITY: Self = Self {
        orientation: Quat::IDENTITY,
        position: Vec3::ZERO,
    };

    pub fn new(orientation: Quat, position: Vec3) -> Self {
        Self {
            orientation,
            position,
        }
    }

    pub fn from_position(position: Vec3) -> Self {
        Self::new(Quat::IDENTITY, position)
    }

    /// `self ∘ child`: first `child`, then `self`.
    pub fn mul(&self, child: &Pose) -> Pose {
        Pose {
            orientation: (self.orientation * child.orientation).normalize(),
            position: self.position + self.orientation * child.position,
        }
    }

    pub fn inverse(&self) -> Pose {
        let q = self.orientation.inverse();
        Pose {
            orientation: q,
            position: -(q * self.position),
        }
    }

    pub fn transform_point(&self, p: Vec3) -> Vec3 {
        self.position + self.orientation * p
    }

    pub fn forward(&self) -> Vec3 {
        self.orientation * Vec3::NEG_Z
    }

    pub fn right(&self) -> Vec3 {
        self.orientation * Vec3::X
    }

    pub fn to_mat4(&self) -> Mat4 {
        Mat4::from_rotation_translation(self.orientation, self.position)
    }

    /// Finite and with a unit quaternion (runtimes report garbage when a
    /// pose is not valid).
    pub fn is_valid(&self) -> bool {
        self.position.is_finite()
            && self.orientation.is_finite()
            && (self.orientation.length() - 1.0).abs() < 1e-3
    }
}

/// The heading part of `q`: the rotation about +Y that turns −Z to where
/// `q` looks, projected onto the ground (no pitch, no roll). Looking
/// straight up or down keeps `q`'s up vector's heading instead.
pub fn yaw_only(q: Quat) -> Quat {
    Quat::from_rotation_y(yaw_angle(q))
}

/// The yaw angle of [`yaw_only`] (rad in −π…π, + = turned left, about +Y).
pub fn yaw_angle(q: Quat) -> f32 {
    let f = q * Vec3::NEG_Z;
    let flat = Vec3::new(f.x, 0.0, f.z);
    let dir = if flat.length_squared() > 1e-8 {
        flat
    } else {
        // Looking straight down (up): the head's top points forward (back).
        let up = q * Vec3::Y;
        let s = if f.y < 0.0 { 1.0 } else { -1.0 };
        Vec3::new(up.x * s, 0.0, up.z * s)
    };
    if dir.length_squared() < 1e-12 {
        return 0.0;
    }
    (-dir.x).atan2(-dir.z)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_2;

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-5
    }

    #[test]
    fn poses_compose_and_invert() {
        let a = Pose::new(Quat::from_rotation_y(FRAC_PI_2), Vec3::new(1.0, 2.0, 3.0));
        let b = Pose::from_position(Vec3::new(0.0, 0.0, -1.0));
        // b's origin one metre ahead of a, a turned left: ahead is −X.
        assert!(close(a.mul(&b).position, Vec3::new(0.0, 2.0, 3.0)));
        let id = a.mul(&a.inverse());
        assert!(close(id.position, Vec3::ZERO));
        assert!(id.orientation.angle_between(Quat::IDENTITY) < 1e-5);
        let p = Vec3::new(0.3, -0.2, 5.0);
        assert!(close(a.to_mat4().transform_point3(p), a.transform_point(p)));
        assert!(close(a.forward(), Vec3::NEG_X));
        assert!(Pose::IDENTITY.is_valid());
        assert!(!Pose::new(Quat::from_xyzw(0.0, 0.0, 0.0, 0.0), Vec3::ZERO).is_valid());
        assert!(!Pose::from_position(Vec3::NAN).is_valid());
    }

    #[test]
    fn yaw_only_drops_pitch_and_roll() {
        let yaw = 0.7;
        let q =
            Quat::from_rotation_y(yaw) * Quat::from_rotation_x(-0.4) * Quat::from_rotation_z(0.3);
        let y = yaw_only(q);
        assert!(y.angle_between(Quat::from_rotation_y(yaw)) < 1e-4);
        assert!((yaw_angle(q) - yaw).abs() < 1e-4);
        assert!((yaw_angle(Quat::from_rotation_y(-1.2)) + 1.2).abs() < 1e-4);
        // Straight down: the heading of the top of the head.
        let down = Quat::from_rotation_y(0.5) * Quat::from_rotation_x(-FRAC_PI_2);
        assert!(yaw_only(down).angle_between(Quat::from_rotation_y(0.5)) < 1e-3);
        assert_eq!(yaw_only(Quat::IDENTITY), Quat::IDENTITY);
    }
}
