//! The two projections of the map: [`MapView`] (the studio's north-up map
//! with pan and zoom) and [`NaviView`] (the HUD's heading-up navigation
//! map: the rider fixed in the lower third, the map turning with the
//! direction of travel).
//!
//! Package metres: x east, z south (so north is −z). Screen points: x to
//! the right, y down.

/// Zoom limits of the studio map (metres per screen point).
pub const MIN_M_PER_PX: f64 = 0.05;
pub const MAX_M_PER_PX: f64 = 500.0;

/// The studio map's pan and zoom (north up, east right).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapView {
    /// The package point at the centre of the map (x, z).
    pub centre: [f64; 2],
    /// Metres per screen point.
    pub m_per_px: f64,
}

impl Default for MapView {
    fn default() -> Self {
        Self {
            centre: [0.0; 2],
            m_per_px: 10.0,
        }
    }
}

impl MapView {
    /// The view that shows `min`…`max` whole in a map of `size` points.
    pub fn fit(min: [f64; 2], max: [f64; 2], size: [f32; 2]) -> Self {
        let w = (max[0] - min[0]).max(1.0);
        let h = (max[1] - min[1]).max(1.0);
        let m = (w / f64::from(size[0].max(1.0))).max(h / f64::from(size[1].max(1.0))) * 1.05;
        Self {
            centre: [(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5],
            m_per_px: m.clamp(MIN_M_PER_PX, MAX_M_PER_PX),
        }
    }

    /// A package point on the screen (`mid`: the map's centre on screen).
    pub fn to_screen(self, p: [f64; 2], mid: [f32; 2]) -> [f32; 2] {
        [
            mid[0] + ((p[0] - self.centre[0]) / self.m_per_px) as f32,
            mid[1] + ((p[1] - self.centre[1]) / self.m_per_px) as f32,
        ]
    }

    /// The package point under a screen point.
    pub fn to_world(self, s: [f32; 2], mid: [f32; 2]) -> [f64; 2] {
        [
            self.centre[0] + f64::from(s[0] - mid[0]) * self.m_per_px,
            self.centre[1] + f64::from(s[1] - mid[1]) * self.m_per_px,
        ]
    }

    /// Zooms by `factor` (> 1: closer) keeping the point under `s` still.
    pub fn zoom_at(&mut self, factor: f64, s: [f32; 2], mid: [f32; 2]) {
        let before = self.to_world(s, mid);
        self.m_per_px = (self.m_per_px / factor).clamp(MIN_M_PER_PX, MAX_M_PER_PX);
        let after = self.to_world(s, mid);
        self.centre[0] += before[0] - after[0];
        self.centre[1] += before[1] - after[1];
    }

    /// Moves the map with a drag of `d` points (the ground follows the
    /// pointer).
    pub fn pan(&mut self, d: [f32; 2]) {
        self.centre[0] -= f64::from(d[0]) * self.m_per_px;
        self.centre[1] -= f64::from(d[1]) * self.m_per_px;
    }

    /// The package area a map of `size` points shows: (min, max).
    pub fn visible(&self, size: [f32; 2]) -> ([f64; 2], [f64; 2]) {
        let hx = f64::from(size[0]) * 0.5 * self.m_per_px;
        let hz = f64::from(size[1]) * 0.5 * self.m_per_px;
        (
            [self.centre[0] - hx, self.centre[1] - hz],
            [self.centre[0] + hx, self.centre[1] + hz],
        )
    }
}

/// Where the rider sits on the navigation map: centred, at this share of
/// the height from the top (the lower third, as on a bike computer).
pub const RIDER_ANCHOR: [f32; 2] = [0.5, 0.7];

/// The compass bearing (rad, clockwise from north) of a direction (x, z).
pub fn bearing(dir: [f64; 2]) -> f64 {
    dir[0].atan2(-dir[1])
}

/// The direction (x, z) of a compass bearing (rad, clockwise from north).
pub fn direction(bearing: f64) -> [f64; 2] {
    [bearing.sin(), -bearing.cos()]
}

/// The heading-up projection of the navigation map.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NaviView {
    /// The rider (package metres, x z).
    pub rider: [f64; 2],
    /// The map's up: compass bearing (rad, clockwise from north).
    pub heading: f64,
    /// Metres per map point.
    pub m_per_px: f64,
    /// The map's size (points).
    pub size: [f32; 2],
}

impl NaviView {
    /// The rider's place on the map (points from the top left).
    pub fn anchor(&self) -> [f32; 2] {
        [
            self.size[0] * RIDER_ANCHOR[0],
            self.size[1] * RIDER_ANCHOR[1],
        ]
    }

    /// Up on the map, in package metres (unit).
    pub fn forward(&self) -> [f64; 2] {
        direction(self.heading)
    }

    /// Right on the map, in package metres (unit).
    pub fn right(&self) -> [f64; 2] {
        let f = self.forward();
        [-f[1], f[0]]
    }

    /// A package point on the map (points from the top left).
    pub fn to_screen(&self, p: [f64; 2]) -> [f32; 2] {
        let d = [p[0] - self.rider[0], p[1] - self.rider[1]];
        let (f, r) = (self.forward(), self.right());
        let a = self.anchor();
        [
            a[0] + ((d[0] * r[0] + d[1] * r[1]) / self.m_per_px) as f32,
            a[1] - ((d[0] * f[0] + d[1] * f[1]) / self.m_per_px) as f32,
        ]
    }

    /// The package point under a map point.
    pub fn to_world(&self, s: [f32; 2]) -> [f64; 2] {
        let a = self.anchor();
        let sx = f64::from(s[0] - a[0]) * self.m_per_px;
        let sy = f64::from(a[1] - s[1]) * self.m_per_px;
        let (f, r) = (self.forward(), self.right());
        [
            self.rider[0] + r[0] * sx + f[0] * sy,
            self.rider[1] + r[1] * sx + f[1] * sy,
        ]
    }

    /// The texel offsets (texture of `m_per_texel`) per whole map width
    /// and per whole map height: a map point at `uv` (0…1 over the map)
    /// lies `ax · (u − anchor u) + ay · (v − anchor v)` texels from the
    /// rider. The map shader turns the cached north-up texture with these.
    pub fn texel_axes(&self, m_per_texel: f64) -> ([f64; 2], [f64; 2]) {
        let (f, r) = (self.forward(), self.right());
        let kx = f64::from(self.size[0]) * self.m_per_px / m_per_texel;
        let ky = f64::from(self.size[1]) * self.m_per_px / m_per_texel;
        ([r[0] * kx, r[1] * kx], [-f[0] * ky, -f[1] * ky])
    }

    /// Where north points on the map: a unit vector in screen points.
    pub fn north(&self) -> [f32; 2] {
        let (f, r) = (self.forward(), self.right());
        // North is (0, −1) in package metres.
        [(-r[1]) as f32, (f[1]) as f32]
    }

    /// The farthest a map corner is from the rider (points).
    pub fn reach_px(&self) -> f32 {
        let a = self.anchor();
        let dx = a[0].max(self.size[0] - a[0]);
        let dy = a[1].max(self.size[1] - a[1]);
        (dx * dx + dy * dy).sqrt()
    }

    /// The package area the map can show (a box around the rider).
    pub fn visible(&self) -> ([f64; 2], [f64; 2]) {
        let r = f64::from(self.reach_px()) * self.m_per_px;
        (
            [self.rider[0] - r, self.rider[1] - r],
            [self.rider[0] + r, self.rider[1] + r],
        )
    }

    /// Whether a map point lies inside the map, `inset` points from its
    /// edge, with corners rounded by `radius`.
    pub fn inside(&self, s: [f32; 2], inset: f32, radius: f32) -> bool {
        inside_rounded(s, self.size, inset, radius)
    }
}

/// Whether `s` lies inside a `size` box shrunk by `inset` whose corners
/// are rounded by `radius` (points).
pub fn inside_rounded(s: [f32; 2], size: [f32; 2], inset: f32, radius: f32) -> bool {
    let (lo, hi) = ([inset, inset], [size[0] - inset, size[1] - inset]);
    if s[0] < lo[0] || s[1] < lo[1] || s[0] > hi[0] || s[1] > hi[1] {
        return false;
    }
    let r = radius.max(0.0);
    let cx = s[0].clamp(lo[0] + r, hi[0] - r);
    let cy = s[1].clamp(lo[1] + r, hi[1] - r);
    let (dx, dy) = (s[0] - cx, s[1] - cy);
    dx * dx + dy * dy <= r * r
}

/// Smooths the map's heading towards `target` (both bearings, rad): the
/// short way round, with time constant `tau` s, so the map turns calmly
/// through bends and jumps at once after a U-turn (more than 120°).
pub fn smooth_heading(current: f64, target: f64, dt: f64, tau: f64) -> f64 {
    let tau_pi = std::f64::consts::TAU;
    let mut d = (target - current) % tau_pi;
    if d > std::f64::consts::PI {
        d -= tau_pi;
    } else if d < -std::f64::consts::PI {
        d += tau_pi;
    }
    if d.abs() > 120f64.to_radians() || tau <= 0.0 {
        return target;
    }
    let k = 1.0 - (-dt.max(0.0) / tau).exp();
    let h = current + d * k;
    h.rem_euclid(tau_pi)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_keeps_the_point_under_the_pointer_and_pan_follows_it() {
        let mut v = MapView::fit([0.0, 0.0], [2000.0, 1000.0], [400.0, 400.0]);
        assert_eq!(v.centre, [1000.0, 500.0]);
        assert!((v.m_per_px - 2000.0 / 400.0 * 1.05).abs() < 1e-9);
        let mid = [200.0, 200.0];
        let s = [300.0, 120.0];
        let under = v.to_world(s, mid);
        v.zoom_at(2.0, s, mid);
        let again = v.to_world(s, mid);
        assert!((under[0] - again[0]).abs() < 1e-6 && (under[1] - again[1]).abs() < 1e-6);
        let p = v.to_world([250.0, 250.0], mid);
        v.pan([10.0, -5.0]);
        let moved = v.to_screen(p, mid);
        assert!((moved[0] - 260.0).abs() < 1e-3 && (moved[1] - 245.0).abs() < 1e-3);
        v.zoom_at(1e9, s, mid);
        assert_eq!(v.m_per_px, MIN_M_PER_PX);
        let (lo, hi) = v.visible([100.0, 50.0]);
        assert!((hi[0] - lo[0] - 100.0 * MIN_M_PER_PX).abs() < 1e-9);
    }

    fn navi(heading_deg: f64) -> NaviView {
        NaviView {
            rider: [1000.0, -500.0],
            heading: heading_deg.to_radians(),
            m_per_px: 2.0,
            size: [200.0, 300.0],
        }
    }

    fn close(a: [f32; 2], b: [f32; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3
    }

    /// Heading-up: the rider is fixed in the lower third, the road ahead
    /// is up whatever the direction, and to_world inverts to_screen.
    #[test]
    fn the_navi_map_turns_with_the_rider_who_stays_in_the_lower_third() {
        for deg in [0.0, 37.0, 90.0, 180.0, 271.0] {
            let v = navi(deg);
            let a = v.anchor();
            assert_eq!(a, [100.0, 210.0]);
            assert!(close(v.to_screen(v.rider), a));
            // 100 m ahead: 50 points straight up.
            let f = v.forward();
            let ahead = [v.rider[0] + 100.0 * f[0], v.rider[1] + 100.0 * f[1]];
            assert!(close(v.to_screen(ahead), [100.0, 160.0]), "{deg}");
            // 20 m to the right: 10 points right.
            let r = v.right();
            let right = [v.rider[0] + 20.0 * r[0], v.rider[1] + 20.0 * r[1]];
            assert!(close(v.to_screen(right), [110.0, 210.0]), "{deg}");
            for s in [[0.0f32, 0.0], [200.0, 300.0], [13.0, 250.0]] {
                assert!(close(v.to_screen(v.to_world(s)), s));
            }
        }
        // Riding east: north is to the left on the map, east is up.
        let v = navi(90.0);
        let east = v.to_screen([v.rider[0] + 10.0, v.rider[1]]);
        assert!(close(east, [100.0, 205.0]));
        let n = v.north();
        assert!((n[0] + 1.0).abs() < 1e-6 && n[1].abs() < 1e-6, "{n:?}");
        // Riding north: north is up.
        let n = navi(0.0).north();
        assert!(n[0].abs() < 1e-6 && (n[1] + 1.0).abs() < 1e-6, "{n:?}");
        assert!((bearing([1.0, 0.0]) - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
        assert!(bearing([0.0, -1.0]).abs() < 1e-12);
    }

    /// The shader's texel axes give the same point as to_world.
    #[test]
    fn texel_axes_match_the_projection() {
        let v = navi(123.0);
        let mt = 0.75;
        let (ax, ay) = v.texel_axes(mt);
        for uv in [[0.0f32, 0.0], [1.0, 1.0], [0.3, 0.9]] {
            let s = [uv[0] * v.size[0], uv[1] * v.size[1]];
            let w = v.to_world(s);
            let (du, dv) = (
                f64::from(uv[0] - RIDER_ANCHOR[0]),
                f64::from(uv[1] - RIDER_ANCHOR[1]),
            );
            let t = [ax[0] * du + ay[0] * dv, ax[1] * du + ay[1] * dv];
            assert!((t[0] * mt - (w[0] - v.rider[0])).abs() < 1e-3);
            assert!((t[1] * mt - (w[1] - v.rider[1])).abs() < 1e-3);
        }
        let (lo, hi) = v.visible();
        let r = f64::from(v.reach_px()) * 2.0;
        assert!((hi[0] - lo[0] - 2.0 * r).abs() < 1e-9);
    }

    #[test]
    fn rounded_insides() {
        let size = [100.0, 80.0];
        assert!(inside_rounded([50.0, 40.0], size, 4.0, 10.0));
        assert!(!inside_rounded([2.0, 40.0], size, 4.0, 10.0));
        // The corner is cut off by the radius.
        assert!(!inside_rounded([5.0, 5.0], size, 4.0, 10.0));
        assert!(inside_rounded([14.0, 14.0], size, 4.0, 10.0));
    }

    #[test]
    fn the_heading_turns_calmly_the_short_way() {
        let d = |a: f64| a.to_radians();
        // From 350° to 10°: through north, not back round.
        let h = smooth_heading(d(350.0), d(10.0), 0.1, 0.5);
        assert!(h > d(350.0) || h < d(10.0), "{}", h.to_degrees());
        let mut h = d(350.0);
        for _ in 0..100 {
            h = smooth_heading(h, d(10.0), 0.05, 0.5);
        }
        assert!((h - d(10.0)).abs() < 1e-3);
        // A U-turn jumps.
        assert_eq!(smooth_heading(d(0.0), d(180.0), 0.016, 0.5), d(180.0));
    }
}
