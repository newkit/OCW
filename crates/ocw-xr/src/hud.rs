//! The HUD in VR, X1's simple version (spec §14 Phase 11, X1; the bike
//! computer on the stem and the world-space menus come with X3): one
//! fixed panel in front of and below the rider's eyes, carried with the
//! anchor (world-locked to the bike, never head-locked: it does not swim
//! with the head), tilted to face the eye.

use glam::{Quat, Vec3};

use crate::Pose;

/// The panel's size and place relative to the anchor (the seated eye).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HudPanel {
    /// Width and height (m).
    pub width_m: f32,
    pub height_m: f32,
    /// Ahead of the eye (m, horizontal) …
    pub ahead_m: f32,
    /// … and below it (m).
    pub below_m: f32,
    /// The panel texture's pixels (width, height).
    pub texture_px: (u32, u32),
}

impl Default for HudPanel {
    /// About 30° wide and 15–28° below the horizon: over the bars, under
    /// the road ahead, readable at the Quest 1's ~14 px per degree.
    fn default() -> Self {
        Self {
            width_m: 0.52,
            height_m: 0.26,
            ahead_m: 0.95,
            below_m: 0.36,
            texture_px: (1024, 512),
        }
    }
}

impl HudPanel {
    /// The panel's centre and orientation in the anchor space: its normal
    /// (+Z of the quad) points back at the eye.
    pub fn anchor_from_panel(&self) -> Pose {
        let centre = Vec3::new(0.0, -self.below_m, -self.ahead_m);
        // Lying back by the angle under which the eye sees it.
        Pose::new(Quat::from_rotation_x(-self.depression()), centre)
    }

    /// Angle below the horizon of the panel's centre (rad).
    pub fn depression(&self) -> f32 {
        (self.below_m / self.ahead_m).atan()
    }

    /// The angle the panel spans across (rad).
    pub fn angular_width(&self) -> f32 {
        let d = self.ahead_m.hypot(self.below_m);
        2.0 * (0.5 * self.width_m / d).atan()
    }

    /// Texture pixels per degree of view across the panel.
    pub fn px_per_degree(&self) -> f32 {
        self.texture_px.0 as f32 / self.angular_width().to_degrees()
    }

    /// The four corners in the anchor space (bottom left, bottom right,
    /// top right, top left).
    pub fn corners(&self) -> [Vec3; 4] {
        let p = self.anchor_from_panel();
        let (w, h) = (0.5 * self.width_m, 0.5 * self.height_m);
        [
            Vec3::new(-w, -h, 0.0),
            Vec3::new(w, -h, 0.0),
            Vec3::new(w, h, 0.0),
            Vec3::new(-w, h, 0.0),
        ]
        .map(|c| p.transform_point(c))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::QUEST1_LEFT_FOV;

    #[test]
    fn the_panel_faces_the_eye_below_the_road_ahead() {
        let hud = HudPanel::default();
        let p = hud.anchor_from_panel();
        // The quad's normal points back at the eye (the anchor's origin).
        let normal = p.orientation * Vec3::Z;
        let to_eye = (-p.position).normalize();
        assert!(normal.dot(to_eye) > 0.999, "{normal} {to_eye}");
        // Below the horizon, but in a comfortable glance (≤ 30°).
        let dep = hud.depression().to_degrees();
        assert!((15.0..=30.0).contains(&dep), "{dep}");
        // All of it inside the Quest 1's view when looking a little down.
        let look = Quat::from_rotation_x(-hud.depression());
        for c in hud.corners() {
            assert!(QUEST1_LEFT_FOV.contains(look.inverse() * c), "{c}");
        }
        // Sharp enough to read: the texture outresolves the display.
        assert!(hud.px_per_degree() > 25.0, "{}", hud.px_per_degree());
        let w = hud.angular_width().to_degrees();
        assert!((25.0..=35.0).contains(&w), "{w}");
    }
}
