//! Recentre (spec §14 Phase 11B, *Camera*): makes the head's current
//! place and heading the rider's seated eye on the bike.
//!
//! The runtime's tracking space has its origin wherever the headset was
//! when the session started. [`Recentre`] is the transform from that
//! space into the **anchor space**: origin at the seated eye, −Z along the
//! bike. A recentre takes the head's pose at that moment and maps it to
//! the origin, turned to look forward – **yaw only**: the rider's own
//! pitch and roll stay (gravity is real), so the horizon stays level.
//!
//! [`RecentreAdvice`] watches the head between recentres: a head that sits
//! far from the anchor for a while (the rider moved on the saddle, or
//! put the headset on standing up) asks for a recentre (*proposal*: done
//! automatically after 0.5 m for 5 s; the console's `vr recentre` and the
//! key do it at once).

use glam::Vec3;

use crate::{Pose, yaw_only};

/// Tracking space → anchor space.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Recentre {
    pub anchor_from_tracking: Pose,
}

impl Recentre {
    /// Recentred on `head` (tracking space).
    pub fn at(head: &Pose) -> Self {
        let q = yaw_only(head.orientation).inverse();
        Self {
            anchor_from_tracking: Pose::new(q, -(q * head.position)),
        }
    }

    /// A pose of the tracking space in the anchor space.
    pub fn apply(&self, tracking: &Pose) -> Pose {
        self.anchor_from_tracking.mul(tracking)
    }
}

/// When to recentre by itself.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RecentreAdvice {
    /// Head this far from the anchor (m) …
    pub max_offset_m: f32,
    /// … for this long (s).
    pub hold_s: f32,
    away_s: f32,
}

impl Default for RecentreAdvice {
    fn default() -> Self {
        Self {
            max_offset_m: 0.5,
            hold_s: 5.0,
            away_s: 0.0,
        }
    }
}

impl RecentreAdvice {
    /// Feeds the head in the anchor space after `dt` seconds; true when it
    /// is time to recentre (then the clock starts again).
    pub fn update(&mut self, head_in_anchor: &Pose, dt: f32) -> bool {
        if head_in_anchor.position.length() > self.max_offset_m {
            self.away_s += dt.max(0.0);
        } else {
            self.away_s = 0.0;
        }
        if self.away_s >= self.hold_s {
            self.away_s = 0.0;
            return true;
        }
        false
    }

    /// Seconds the head has been away.
    pub fn away_s(&self) -> f32 {
        self.away_s
    }
}

/// The head's offset from the anchor (m, anchor space): for `vr status`.
pub fn offset(head_in_anchor: &Pose) -> Vec3 {
    head_in_anchor.position
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Quat;

    #[test]
    fn recentre_puts_the_head_at_the_anchor_facing_forward() {
        let head = Pose::new(
            Quat::from_rotation_y(1.1) * Quat::from_rotation_x(-0.2),
            Vec3::new(0.4, 1.3, -0.8),
        );
        let r = Recentre::at(&head);
        let h = r.apply(&head);
        assert!(h.position.length() < 1e-5);
        // Facing −Z, with the rider's own pitch kept.
        assert!(h.orientation.angle_between(Quat::from_rotation_x(-0.2)) < 1e-4);
        // A step to the head's right after the recentre is +X in the anchor.
        let moved = Pose::new(head.orientation, head.position + head.right() * 0.1);
        let m = r.apply(&moved);
        assert!((m.position - Vec3::new(0.1, 0.0, 0.0)).length() < 1e-4);
        // The default is no change.
        assert_eq!(Recentre::default().apply(&head), head);
    }

    #[test]
    fn a_long_offset_asks_for_a_recentre() {
        let mut a = RecentreAdvice::default();
        let near = Pose::from_position(Vec3::new(0.1, 0.0, 0.0));
        let far = Pose::from_position(Vec3::new(0.0, 0.0, 0.7));
        for _ in 0..1000 {
            assert!(!a.update(&near, 0.1));
        }
        // 4.9 s away: not yet; coming back resets the clock.
        for _ in 0..49 {
            assert!(!a.update(&far, 0.1));
        }
        assert!(!a.update(&near, 0.1));
        assert_eq!(a.away_s(), 0.0);
        let n = (0..100).position(|_| a.update(&far, 0.1)).unwrap();
        assert!((49..=50).contains(&n), "after 5 s: {n}");
        assert_eq!(a.away_s(), 0.0);
        assert_eq!(offset(&far), Vec3::new(0.0, 0.0, 0.7));
    }
}
