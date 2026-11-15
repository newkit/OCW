//! An eye's field of view and its projection.
//!
//! Headset eyes see asymmetric frusta: each eye sees more to its own side
//! than towards the nose, and more below than above. OpenXR gives the four
//! half-angles ([`Fov`], its `XrFovf`); the renderer needs the matrix.
//! Bevy draws with an **infinite, reverse-Z** perspective (depth 1 at the
//! near plane, 0 at infinity, right-handed, −Z forward), so
//! [`clip_from_view`] builds the asymmetric version of
//! glam's right-handed `perspective_infinite_reverse` (`bevy_math::proj`),
//! which it equals for a
//! symmetric field of view (tested).

use glam::{Mat4, Vec3, Vec4};

/// The four half-angles of an eye's field of view (rad), OpenXR's signs:
/// `left` and `down` are negative for a view that includes the centre.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fov {
    pub left: f32,
    pub right: f32,
    pub up: f32,
    pub down: f32,
}

impl Fov {
    /// A symmetric field of view: `vertical` (full angle, rad) and the
    /// width / height ratio `aspect`.
    pub fn symmetric(vertical: f32, aspect: f32) -> Self {
        let t = (0.5 * vertical).tan();
        let h = (t * aspect).atan();
        Self {
            left: -h,
            right: h,
            up: 0.5 * vertical,
            down: -0.5 * vertical,
        }
    }

    /// Mirrored left ↔ right (the other eye of a symmetric headset).
    pub fn mirrored(&self) -> Self {
        Self {
            left: -self.right,
            right: -self.left,
            ..*self
        }
    }

    /// The tangents (left, right, up, down).
    pub fn tangents(&self) -> (f32, f32, f32, f32) {
        (
            self.left.tan(),
            self.right.tan(),
            self.up.tan(),
            self.down.tan(),
        )
    }

    /// Full horizontal and vertical angles (rad).
    pub fn extent(&self) -> (f32, f32) {
        (self.right - self.left, self.up - self.down)
    }

    /// Width / height of the image plane.
    pub fn aspect(&self) -> f32 {
        let (l, r, u, d) = self.tangents();
        (r - l) / (u - d)
    }

    /// A usable field of view: finite, each side under 90°, and opening
    /// in both directions.
    pub fn is_valid(&self) -> bool {
        let lim = std::f32::consts::FRAC_PI_2 - 1e-3;
        [self.left, self.right, self.up, self.down]
            .iter()
            .all(|a| a.is_finite() && a.abs() < lim)
            && self.right > self.left
            && self.up > self.down
    }

    /// Whether the view direction `dir` (view space, −Z forward) is inside.
    pub fn contains(&self, dir: Vec3) -> bool {
        if dir.z >= 0.0 {
            return false;
        }
        let (l, r, u, d) = self.tangents();
        let (x, y) = (dir.x / -dir.z, dir.y / -dir.z);
        (l..=r).contains(&x) && (d..=u).contains(&y)
    }
}

/// Bevy's projection for `fov`: infinite far plane, reverse Z (`near` maps
/// to depth 1), right-handed.
pub fn clip_from_view(fov: &Fov, near: f32) -> Mat4 {
    let (l, r, u, d) = fov.tangents();
    let w = r - l;
    let h = u - d;
    Mat4::from_cols(
        Vec4::new(2.0 / w, 0.0, 0.0, 0.0),
        Vec4::new(0.0, 2.0 / h, 0.0, 0.0),
        Vec4::new((r + l) / w, (u + d) / h, 0.0, -1.0),
        Vec4::new(0.0, 0.0, near, 0.0),
    )
}

/// The frustum's corners between the view-space depths `z_near` and
/// `z_far` (both negative: in front), in Bevy's order (near plane first:
/// bottom right, top right, top left, bottom left; then the far plane).
pub fn frustum_corners(fov: &Fov, z_near: f32, z_far: f32) -> [Vec3; 8] {
    let (l, r, u, d) = fov.tangents();
    let plane = |z: f32| {
        let s = z.abs();
        [
            Vec3::new(r * s, d * s, z),
            Vec3::new(r * s, u * s, z),
            Vec3::new(l * s, u * s, z),
            Vec3::new(l * s, d * s, z),
        ]
    };
    let (a, b) = (plane(z_near), plane(z_far));
    [a[0], a[1], a[2], a[3], b[0], b[1], b[2], b[3]]
}

/// The near plane the eye cameras use (m): close enough for the
/// handlebar under the rider's eyes.
pub const NEAR_M: f32 = 0.05;

#[cfg(test)]
mod tests {
    use super::*;

    fn ndc(m: &Mat4, p: Vec3) -> Vec3 {
        let c = *m * p.extend(1.0);
        c.truncate() / c.w
    }

    #[test]
    fn symmetric_fov_is_bevys_own_projection() {
        let (fov_y, aspect, near) = (1.2f32, 0.9f32, 0.1f32);
        let ours = clip_from_view(&Fov::symmetric(fov_y, aspect), near);
        let bevys =
            glam::camera::rh::proj::directx::perspective_infinite_reverse(fov_y, aspect, near);
        assert!(ours.abs_diff_eq(bevys, 1e-5), "{ours} vs {bevys}");
        assert!((Fov::symmetric(fov_y, aspect).aspect() - aspect).abs() < 1e-5);
    }

    /// Reference values: an Oculus-Quest-like left eye (wider to the
    /// left and down), the matrix worked out by hand.
    #[test]
    fn asymmetric_projection_matches_reference_values() {
        let fov = Fov {
            left: -0.9,
            right: 0.75,
            up: 0.85,
            down: -0.95,
        };
        let m = clip_from_view(&fov, 0.05);
        let (l, r, u, d) = (0.9f32.tan(), 0.75f32.tan(), 0.85f32.tan(), 0.95f32.tan());
        // tan: 1.260158, 0.931596, 1.138333, 1.398383.
        assert!((l - 1.260_158).abs() < 1e-5 && (d - 1.398_383).abs() < 1e-5);
        let expect = Mat4::from_cols(
            Vec4::new(2.0 / (r + l), 0.0, 0.0, 0.0),
            Vec4::new(0.0, 2.0 / (u + d), 0.0, 0.0),
            Vec4::new((r - l) / (r + l), (u - d) / (u + d), 0.0, -1.0),
            Vec4::new(0.0, 0.0, 0.05, 0.0),
        );
        assert!(m.abs_diff_eq(expect, 1e-6));
        assert!((m.x_axis.x - 0.912_511).abs() < 1e-4);
        assert!((m.z_axis.x + 0.149_908).abs() < 1e-4);
        assert!((m.z_axis.y + 0.102_514).abs() < 1e-4);
        // The frustum's edges land on the NDC square's edges.
        let z = -3.0;
        let at = |tx: f32, ty: f32| ndc(&m, Vec3::new(tx * 3.0, ty * 3.0, z));
        assert!((at(r, 0.0).x - 1.0).abs() < 1e-5);
        assert!((at(-l, 0.0).x + 1.0).abs() < 1e-5);
        assert!((at(0.0, u).y - 1.0).abs() < 1e-5);
        assert!((at(0.0, -d).y + 1.0).abs() < 1e-5);
        // Reverse Z: the near plane at depth 1, farther ever closer to 0.
        assert!((ndc(&m, Vec3::new(0.0, 0.0, -0.05)).z - 1.0).abs() < 1e-5);
        let far = ndc(&m, Vec3::new(0.0, 0.0, -10_000.0)).z;
        assert!(far > 0.0 && far < 1e-5);
    }

    #[test]
    fn frustum_corners_follow_the_angles() {
        let fov = Fov {
            left: -0.5,
            right: 0.3,
            up: 0.4,
            down: -0.6,
        };
        let c = frustum_corners(&fov, -1.0, -10.0);
        // Bottom right of the near plane, top left of the far plane.
        assert!((c[0] - Vec3::new(0.3f32.tan(), -(0.6f32.tan()), -1.0)).length() < 1e-5);
        assert!(
            (c[6] - Vec3::new(-(0.5f32.tan()) * 10.0, 0.4f32.tan() * 10.0, -10.0)).length() < 1e-4
        );
        // Every corner projects onto the NDC square's corners.
        let m = clip_from_view(&fov, 0.1);
        for p in c {
            let n = ndc(&m, p);
            assert!((n.x.abs() - 1.0).abs() < 1e-4 && (n.y.abs() - 1.0).abs() < 1e-4);
        }
    }

    #[test]
    fn validity_mirroring_and_containment() {
        let fov = Fov {
            left: -0.9,
            right: 0.75,
            up: 0.85,
            down: -0.95,
        };
        assert!(fov.is_valid());
        let m = fov.mirrored();
        assert_eq!((m.left, m.right), (-0.75, 0.9));
        assert!(fov.contains(Vec3::NEG_Z));
        assert!(!fov.contains(Vec3::Z));
        assert!(!fov.contains(Vec3::new(1.0, 0.0, -0.5)));
        assert!(fov.contains(Vec3::new(-1.2, 0.0, -1.0)));
        assert!(!Fov { right: -1.0, ..fov }.is_valid());
        assert!(
            !Fov {
                up: f32::NAN,
                ..fov
            }
            .is_valid()
        );
        assert!(!Fov { left: -1.6, ..fov }.is_valid());
        let (h, v) = fov.extent();
        assert!((h - 1.65).abs() < 1e-6 && (v - 1.8).abs() < 1e-6);
    }
}
