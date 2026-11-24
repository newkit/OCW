//! The first-person anchor (spec §14 Phase 11B, *Camera*): the rider's
//! seated eye on the bike, and how the tracked eyes become world cameras.
//!
//! The chain, right to left:
//!
//! `world_from_eye = world_from_anchor ∘ anchor_from_tracking ∘ tracking_from_eye`
//!
//! - `tracking_from_eye`: the runtime's located view (or the simulated
//!   head's eye);
//! - `anchor_from_tracking`: the [`crate::recentre::Recentre`];
//! - `world_from_anchor`: [`world_from_anchor`] – the eye point of the
//!   posed rider (`ocw_assets::pose::eye_point`, bike frame) carried by the
//!   bike's world pose. With the **horizon lock** (the comfort default,
//!   11B) only the bike's heading turns the anchor: the bike's lean in
//!   corners and its pitch on climbs never roll or tip the view; the
//!   rider's own head does.
//!
//! The server stays authoritative (spec §6.1): the head pose only moves
//! the cameras, never the rider.

use glam::Vec3;

use crate::view::{EyeView, StereoViews};
use crate::{Pose, recentre::Recentre, yaw_only};

/// The anchor in the world for a bike at `world_from_bike` with the
/// seated eye at `eye_in_bike` (bike frame: −Z forward, +Y up).
pub fn world_from_anchor(world_from_bike: &Pose, eye_in_bike: Vec3, horizon_lock: bool) -> Pose {
    let position = world_from_bike.transform_point(eye_in_bike);
    let orientation = if horizon_lock {
        yaw_only(world_from_bike.orientation)
    } else {
        world_from_bike.orientation
    };
    Pose::new(orientation, position)
}

/// The eyes in the world.
pub fn world_eyes(anchor: &Pose, recentre: &Recentre, views: &StereoViews) -> StereoViews {
    views.map(|v| EyeView {
        pose: anchor.mul(&recentre.apply(&v.pose)),
        fov: v.fov,
    })
}

/// The head in the world (between the eyes).
pub fn world_head(anchor: &Pose, recentre: &Recentre, head_in_tracking: &Pose) -> Pose {
    anchor.mul(&recentre.apply(head_in_tracking))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::{QUEST1_LEFT_FOV, sim_views};
    use glam::Quat;

    fn bike(yaw: f32, lean: f32, pitch: f32) -> Pose {
        Pose::new(
            Quat::from_rotation_y(yaw) * Quat::from_rotation_x(pitch) * Quat::from_rotation_z(lean),
            Vec3::new(100.0, 20.0, -50.0),
        )
    }

    #[test]
    fn the_horizon_stays_level_in_corners_and_on_climbs() {
        let eye = Vec3::new(0.0, 1.55, 0.1);
        let b = bike(0.6, 0.35, 0.08);
        let a = world_from_anchor(&b, eye, true);
        // Level: the anchor's right axis is horizontal, forward too.
        assert!(a.right().y.abs() < 1e-5 && a.forward().y.abs() < 1e-5);
        // Facing the bike's heading.
        let f = b.forward();
        let flat = Vec3::new(f.x, 0.0, f.z).normalize();
        assert!((a.forward() - flat).length() < 1e-4);
        // The eye rides where the bike carries it (also leaning).
        assert!((a.position - b.transform_point(eye)).length() < 1e-5);
        // Without the lock the view rolls with the bike.
        let free = world_from_anchor(&b, eye, false);
        assert!(free.right().y.abs() > 0.3);
    }

    #[test]
    fn recentred_eyes_sit_on_the_anchor() {
        let anchor = world_from_anchor(&bike(-1.0, 0.0, 0.0), Vec3::new(0.0, 1.5, 0.0), true);
        // The rider's head somewhere in the room, turned away.
        let head = Pose::new(Quat::from_rotation_y(2.0), Vec3::new(0.3, 1.2, 0.5));
        let views = sim_views(&head, 0.064, &QUEST1_LEFT_FOV);
        let r = Recentre::at(&head);
        let eyes = world_eyes(&anchor, &r, &views);
        let mid = 0.5 * (eyes[0].pose.position + eyes[1].pose.position);
        assert!((mid - anchor.position).length() < 1e-4);
        // Looking along the bike, the eyes across it.
        assert!((eyes[0].pose.forward() - anchor.forward()).length() < 1e-4);
        let across = eyes[1].pose.position - eyes[0].pose.position;
        assert!((across.normalize() - anchor.right()).length() < 1e-3);
        assert!((across.length() - 0.064).abs() < 1e-5);
        let h = world_head(&anchor, &r, &head);
        assert!((h.position - anchor.position).length() < 1e-4);
        assert_eq!(eyes[0].fov, QUEST1_LEFT_FOV);
    }
}
