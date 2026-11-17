//! The bike computer in VR (spec §14 Phase 11, 11B *HUD in VR*; decided
//! 2026-11-17, v4.52): no floating panel in front of the rider – all ride
//! information is on a GPS bike computer on an **out-front mount at the
//! centre of the handlebar**, a little ahead of the bar in line with the
//! stem, its screen tilted up toward the rider's eyes. It is part of the
//! bike (it rocks and turns with it), seen when looking down, like a real
//! one.
//!
//! A real unit's screen (≈ 40 × 58 mm) is a few degrees wide from the
//! saddle: the eye resolves it, the Quest 1 (≈ 14 px per degree) does not.
//! So the unit is drawn **[`DEFAULT_SIZE`] times its real size** (the
//! setting `vr.computer_size`, 100 – 400 %), and its screen faces the eye.
//! The mount grows its reach with the size, so the unit always stays ahead
//! of the bar (nothing under it there but air and the front wheel). On a
//! time-trial bike the extensions fill the bar's centre: the unit sits
//! ahead of the extension tips, on their bridge.
//!
//! Frames: the bike's model space (−Z forward, +Y up, metres; the
//! renderer's avatar frame) and the unit's own frame – origin at the
//! unit's centre, +Y the screen's normal, −Z the unit's front (the top of
//! the screen's picture).

use glam::{Quat, Vec3};

use crate::Pose;

/// The unit's body at real size: width (x), height (y), length (z).
pub const BODY_M: Vec3 = Vec3::new(0.052, 0.020, 0.078);
/// The screen at real size: width, length (along the unit).
pub const SCREEN_M: (f32, f32) = (0.040, 0.058);
/// The screen's centre ahead of the body's centre (m, real size): the
/// buttons and the LED bar sit behind it.
pub const SCREEN_AHEAD_M: f32 = 0.004;
/// The screen texture (px): the screen's aspect, sharp beyond the
/// headset's pixels (see the tests).
pub const TEXTURE_PX: (u32, u32) = (600, 870);
/// The size in VR, times the real one (setting `vr.computer_size`: %).
pub const DEFAULT_SIZE: f32 = 2.75;
pub const MIN_SIZE: f32 = 1.0;
pub const MAX_SIZE: f32 = 4.0;
/// The tilt the rider may add to the aim at the eye (degrees,
/// `vr.computer_tilt`).
pub const MAX_TILT_DEG: f32 = 30.0;
/// The smallest text on the screen (font size, texture px).
pub const MIN_FONT_PX: f32 = 48.0;
/// The smallest text must span this much of the view (degrees): ≈ 9 px
/// of the Quest 1's display, readable.
pub const LEGIBLE_DEG: f32 = 0.6;
/// The Quest 1's display: ≈ 14 px per degree (1440 px over ≈ 100°).
pub const QUEST1_PX_PER_DEGREE: f32 = 14.0;
/// The handlebar's radius at the clamp (31.8 mm and 35 mm bars) and the
/// gap the unit keeps from it.
pub const BAR_RADIUS_M: f32 = 0.0175;
pub const CLEARANCE_M: f32 = 0.012;
/// The shortest reach of the mount ahead of the bar (real out-front
/// mounts: 60 – 90 mm).
pub const MIN_REACH_M: f32 = 0.075;
/// The unit's centre above the bar's axis (the mount's rise).
pub const RISE_M: f32 = 0.028;

/// Where the bike's bar is (bike model space).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BarMount {
    /// The centre of the bar at the stem's clamp.
    pub clamp: Vec3,
    /// A time-trial bike: the midpoint of the extensions' tips (where the
    /// hands are); `None` for drop and flat bars.
    pub aero_tips: Option<Vec3>,
}

/// The rider's choice (`vr.computer_size`, `vr.computer_tilt`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComputerSetup {
    /// Times the real size.
    pub size: f32,
    /// Added to the aim at the eye (degrees, + toward the rider).
    pub tilt_deg: f32,
}

impl Default for ComputerSetup {
    fn default() -> Self {
        Self {
            size: DEFAULT_SIZE,
            tilt_deg: 0.0,
        }
    }
}

impl ComputerSetup {
    /// From the settings' percent and degrees, within their ranges.
    pub fn new(size_percent: f32, tilt_deg: f32) -> Self {
        Self {
            size: (size_percent / 100.0).clamp(MIN_SIZE, MAX_SIZE),
            tilt_deg: tilt_deg.clamp(-MAX_TILT_DEG, MAX_TILT_DEG),
        }
    }
}

/// Where the unit is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// The unit in the bike's frame (its centre; rotation about X only).
    pub bike_from_unit: Pose,
    /// The unit's scale (times the real size).
    pub scale: f32,
    /// The mount's arm: from the bar (or the extensions' bridge) …
    pub arm_from: Vec3,
    /// … to under the unit (bike frame).
    pub arm_to: Vec3,
    /// The screen's tilt from flat (rad, + toward the rider).
    pub tilt: f32,
}

/// The extent of the rotated, scaled body toward the rider (+Z) and down
/// (−Y), from its centre.
fn rear_and_down(scale: f32, tilt: f32) -> (f32, f32) {
    let (h, l) = (0.5 * BODY_M.y * scale, 0.5 * BODY_M.z * scale);
    let (s, c) = tilt.sin_cos();
    // The corners (y, z) = (±h, ±l) turned about X by `tilt`:
    // z' = y sin + z cos, y' = y cos − z sin.
    let rear = h * s.abs() + l * c;
    let down = h * c + l * s.abs();
    (rear, down)
}

/// The unit for a bar, aimed at `eye` (bike frame).
pub fn place(mount: &BarMount, eye: Vec3, setup: &ComputerSetup) -> Placement {
    let scale = setup.size.clamp(MIN_SIZE, MAX_SIZE);
    let extra = setup
        .tilt_deg
        .clamp(-MAX_TILT_DEG, MAX_TILT_DEG)
        .to_radians();
    // First guess of the centre (for the aim), then the reach that keeps
    // the tilted unit ahead of the bar, then the aim again from there.
    let base = match mount.aero_tips {
        Some(t) => t + Vec3::new(0.0, -0.02, 0.0),
        None => mount.clamp + Vec3::new(0.0, RISE_M, 0.0),
    };
    let mut pivot = base - Vec3::Z * MIN_REACH_M;
    let mut tilt = 0.0;
    for _ in 0..3 {
        let to_eye = eye - pivot;
        // Angle of the eye from straight up, toward the rider (+Z).
        tilt = (to_eye.z.atan2(to_eye.y) + extra).clamp(0.0, 80f32.to_radians());
        let (rear, _) = rear_and_down(scale, tilt);
        let gap = match mount.aero_tips {
            // Ahead of the hands on the extensions.
            Some(_) => rear + 0.045,
            None => (rear + BAR_RADIUS_M + CLEARANCE_M).max(MIN_REACH_M),
        };
        pivot = base - Vec3::Z * gap;
    }
    let (_, down) = rear_and_down(scale, tilt);
    let arm_from = match mount.aero_tips {
        Some(t) => t + Vec3::new(0.0, -0.03, 0.02),
        None => mount.clamp,
    };
    Placement {
        bike_from_unit: Pose::new(Quat::from_rotation_x(tilt), pivot),
        scale,
        arm_from,
        arm_to: pivot - Vec3::Y * (down * 0.6),
        tilt,
    }
}

impl Placement {
    /// The screen's centre and its normal (bike frame).
    pub fn screen(&self) -> (Vec3, Vec3) {
        let p = &self.bike_from_unit;
        let local = Vec3::new(0.0, 0.5 * BODY_M.y, -SCREEN_AHEAD_M) * self.scale;
        (p.transform_point(local), p.orientation * Vec3::Y)
    }

    /// The screen's size (m) as drawn.
    pub fn screen_m(&self) -> (f32, f32) {
        (SCREEN_M.0 * self.scale, SCREEN_M.1 * self.scale)
    }

    /// The body's eight corners (bike frame).
    pub fn corners(&self) -> [Vec3; 8] {
        let h = 0.5 * BODY_M * self.scale;
        let mut out = [Vec3::ZERO; 8];
        for (i, c) in out.iter_mut().enumerate() {
            let s = |bit: usize| if i & bit != 0 { 1.0 } else { -1.0 };
            *c = self
                .bike_from_unit
                .transform_point(Vec3::new(s(1) * h.x, s(2) * h.y, s(4) * h.z));
        }
        out
    }

    /// The angle (degrees) a line of `font_px` of the screen texture spans
    /// from `eye`.
    pub fn glyph_deg(&self, eye: Vec3, font_px: f32) -> f32 {
        let (centre, normal) = self.screen();
        let to_eye = eye - centre;
        let d = to_eye.length().max(1e-3);
        // Foreshortened by the angle between the screen's normal and the
        // line of sight.
        let facing = normal.dot(to_eye / d).clamp(0.0, 1.0);
        let len = font_px / TEXTURE_PX.1 as f32 * self.screen_m().1 * facing;
        (len / d).atan().to_degrees()
    }

    /// The screen texture's pixels per degree of view (along the screen).
    pub fn texture_px_per_degree(&self, eye: Vec3) -> f32 {
        TEXTURE_PX.1 as f32 / self.glyph_deg(eye, TEXTURE_PX.1 as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A road bike's bar and a rider's steady eye (≈ the generated
    /// endurance bike, bike frame: −Z forward).
    fn road() -> (BarMount, Vec3) {
        (
            BarMount {
                clamp: Vec3::new(0.0, 1.02, -0.38),
                aero_tips: None,
            },
            Vec3::new(0.0, 1.55, 0.12),
        )
    }

    #[test]
    fn the_unit_sits_ahead_of_the_bar_and_faces_the_eye() {
        let (bar, eye) = road();
        for size in [1.0, DEFAULT_SIZE, MAX_SIZE] {
            for tilt in [-MAX_TILT_DEG, 0.0, MAX_TILT_DEG] {
                let p = place(
                    &bar,
                    eye,
                    &ComputerSetup {
                        size,
                        tilt_deg: tilt,
                    },
                );
                // Every corner ahead of the bar's front (or above it).
                for c in p.corners() {
                    let ahead = c.z < bar.clamp.z - BAR_RADIUS_M;
                    let above = c.y > bar.clamp.y + BAR_RADIUS_M;
                    assert!(ahead || above, "size {size} tilt {tilt}: {c}");
                }
                // Centred on the stem's line, a little above the bar.
                let at = p.bike_from_unit.position;
                assert!(at.x.abs() < 1e-6);
                assert!(at.y > bar.clamp.y);
                // Out-front, but not far out.
                let reach = bar.clamp.z - at.z;
                assert!((MIN_REACH_M - 1e-5..0.25).contains(&reach), "{reach}");
            }
            // Without an added tilt the screen faces the eye.
            let p = place(
                &bar,
                eye,
                &ComputerSetup {
                    size,
                    tilt_deg: 0.0,
                },
            );
            let (c, n) = p.screen();
            let to_eye = (eye - c).normalize();
            assert!(n.dot(to_eye) > 0.995, "size {size}: {}", n.dot(to_eye));
        }
    }

    #[test]
    fn the_tilt_turns_the_screen_toward_the_rider() {
        let (bar, eye) = road();
        let flat = place(&bar, eye, &ComputerSetup::new(275.0, -30.0));
        let steep = place(&bar, eye, &ComputerSetup::new(275.0, 30.0));
        assert!(steep.tilt > flat.tilt + 0.9, "{} {}", steep.tilt, flat.tilt);
        // Clamped to the ranges.
        let s = ComputerSetup::new(900.0, 99.0);
        assert_eq!(s.size, MAX_SIZE);
        assert_eq!(s.tilt_deg, MAX_TILT_DEG);
        assert_eq!(ComputerSetup::new(10.0, 0.0).size, MIN_SIZE);
    }

    /// The smallest text is readable on the Quest 1 at the default size,
    /// and the texture is sharper than the display.
    #[test]
    fn the_default_size_is_readable_on_the_quest_1() {
        let (bar, eye) = road();
        let p = place(&bar, eye, &ComputerSetup::default());
        let deg = p.glyph_deg(eye, MIN_FONT_PX);
        assert!(deg >= LEGIBLE_DEG, "{deg}°");
        let quest1 = QUEST1_PX_PER_DEGREE;
        assert!(deg * quest1 >= 8.0, "{} px", deg * quest1);
        assert!(p.texture_px_per_degree(eye) > 2.0 * quest1);
        // Real size is not readable there: that is why it is larger.
        let real = place(&bar, eye, &ComputerSetup::new(100.0, 0.0));
        assert!(real.glyph_deg(eye, MIN_FONT_PX) < LEGIBLE_DEG);
        // The screen stays a glance below the road, not in the middle of
        // the view: ≥ 30° below the horizon.
        let (c, _) = p.screen();
        let down = ((eye.y - c.y) / (eye - c).length()).asin().to_degrees();
        assert!(down > 30.0, "{down}°");
    }

    #[test]
    fn on_aero_bars_it_sits_ahead_of_the_extensions() {
        let tips = Vec3::new(0.0, 1.0, -0.62);
        let bar = BarMount {
            clamp: Vec3::new(0.0, 0.91, -0.32),
            aero_tips: Some(tips),
        };
        let eye = Vec3::new(0.0, 1.32, -0.28);
        let p = place(&bar, eye, &ComputerSetup::default());
        for c in p.corners() {
            assert!(c.z < tips.z - 0.03, "ahead of the hands: {c}");
        }
        assert!(p.glyph_deg(eye, MIN_FONT_PX) >= LEGIBLE_DEG);
        assert!(p.arm_from.z > p.arm_to.z, "the arm reaches forward");
    }
}
