//! A camera that flies with an aircraft (spec 14 Phase 18, 18E T8;
//! close-up flyby videos with the camera path of Phase 13 G5): the
//! `air.follow` commands of `ocw_ui::air`.
//!
//! The simulated aircraft move on paths that are functions of the world
//! clock ([`ocw_sim::air::Flight::state`]), so a camera can stay beside
//! one at 70 m/s however slowly a frame is rendered (`--capture`).
//!
//! - **The aircraft's frame** ([`Frame`]): its position, *right*, *up*
//!   (the world's up) and *ahead* (its heading, level) – offsets are
//!   `[right, up, ahead]` metres.
//! - **Keys** ([`FollowKey`]): at T seconds after `play`, the eye's offset,
//!   the offset it looks at and a **lag** (s): the eye rides on the
//!   aircraft as it was `lag` seconds ago, the look goes to it now. A lag
//!   growing over a few seconds lets the aircraft pull away from the
//!   camera along its own path (towards the runway). The eye, the target
//!   and the lag move between the keys on a cubic Hermite curve over the
//!   keys' times (Catmull-Rom tangents, at rest at the first and last
//!   key), so the camera starts and stops gently and passes the keys
//!   without a jerk.
//! - **The unicorn in formation** ([`Formation`]): an offset in the
//!   aircraft's frame, facing its heading.

use ocw_sim::air::path::PathState;

/// An offset `[right, up, ahead]` (m) in an aircraft's frame.
pub type Offset = [f64; 3];

/// An aircraft's frame: where it is and which way it flies (level).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub pos: [f64; 3],
    /// The unit heading (x, z).
    pub ahead: [f64; 2],
}

impl Frame {
    /// The frame of a path state; `last` keeps the heading while it is
    /// (nearly) standing.
    pub fn of(s: &PathState, last: Option<[f64; 2]>) -> Self {
        let l = s.vel[0].hypot(s.vel[2]);
        let ahead = if l > 0.5 {
            [s.vel[0] / l, s.vel[2] / l]
        } else {
            last.unwrap_or([0.0, -1.0])
        };
        Self { pos: s.pos, ahead }
    }

    /// The right-hand direction (x, z).
    pub fn right(&self) -> [f64; 2] {
        [-self.ahead[1], self.ahead[0]]
    }

    /// The world point of an offset.
    pub fn point(&self, o: Offset) -> [f64; 3] {
        let r = self.right();
        [
            self.pos[0] + r[0] * o[0] + self.ahead[0] * o[2],
            self.pos[1] + o[1],
            self.pos[2] + r[1] * o[0] + self.ahead[1] * o[2],
        ]
    }

    /// The true heading (°) it flies.
    pub fn heading_deg(&self) -> f64 {
        ocw_sim::air::heading_of(self.ahead)
    }
}

/// One key of a follow path.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FollowKey {
    /// Seconds after `play`.
    pub t: f64,
    /// Where the eye is.
    pub eye: Offset,
    /// What it looks at.
    pub target: Offset,
    /// The eye's lag behind the aircraft (s, ≥ 0).
    pub lag: f64,
}

/// What the camera does at one moment: the eye in the frame of the
/// aircraft `lag` seconds ago, looking at `target` in its frame now.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FollowAt {
    pub eye: Offset,
    pub target: Offset,
    pub lag: f64,
}

/// The keys of a follow camera, by time.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FollowPath {
    keys: Vec<FollowKey>,
}

impl FollowPath {
    pub fn keys(&self) -> &[FollowKey] {
        &self.keys
    }

    /// Adds a key (kept in time order; a key at the same time replaces it).
    pub fn add(&mut self, k: FollowKey) {
        let k = FollowKey {
            lag: k.lag.max(0.0),
            ..k
        };
        match self.keys.binary_search_by(|o| o.t.total_cmp(&k.t)) {
            Ok(i) => self.keys[i] = k,
            Err(i) => self.keys.insert(i, k),
        }
    }

    pub fn clear(&mut self) {
        self.keys.clear();
    }

    /// When the last key is (s).
    pub fn duration_s(&self) -> f64 {
        self.keys.last().map_or(0.0, |k| k.t)
    }

    /// The camera at `t` seconds after `play` (before the first key its
    /// pose, after the last one the last); `None` without keys.
    pub fn at(&self, t: f64) -> Option<FollowAt> {
        let k = &self.keys;
        let first = k.first()?;
        let last = k.last()?;
        let of = |k: &FollowKey| FollowAt {
            eye: k.eye,
            target: k.target,
            lag: k.lag,
        };
        if t <= first.t || k.len() == 1 {
            return Some(of(first));
        }
        if t >= last.t {
            return Some(of(last));
        }
        let i = k.partition_point(|x| x.t <= t) - 1;
        let (a, b) = (&k[i], &k[i + 1]);
        let h = b.t - a.t;
        let s = (t - a.t) / h;
        // Catmull-Rom tangents over the keys' times; at rest at the ends.
        let tangent = |j: usize, f: &dyn Fn(&FollowKey) -> f64| -> f64 {
            if j == 0 || j + 1 == k.len() {
                0.0
            } else {
                (f(&k[j + 1]) - f(&k[j - 1])) / (k[j + 1].t - k[j - 1].t)
            }
        };
        let herm = |f: &dyn Fn(&FollowKey) -> f64| -> f64 {
            let (p0, p1) = (f(a), f(b));
            let (m0, m1) = (tangent(i, f) * h, tangent(i + 1, f) * h);
            let s2 = s * s;
            let s3 = s2 * s;
            (2.0 * s3 - 3.0 * s2 + 1.0) * p0
                + (s3 - 2.0 * s2 + s) * m0
                + (-2.0 * s3 + 3.0 * s2) * p1
                + (s3 - s2) * m1
        };
        let v3 = |g: fn(&FollowKey) -> Offset| -> Offset {
            [0, 1, 2].map(|c| herm(&|k: &FollowKey| g(k)[c]))
        };
        // The lag never falls back by more than it should (a camera going
        // backwards): smoothstep between its keys, monotone.
        let ss = s * s * (3.0 - 2.0 * s);
        Some(FollowAt {
            eye: v3(|k| k.eye),
            target: v3(|k| k.target),
            lag: a.lag + (b.lag - a.lag) * ss,
        })
    }

    /// The camera's eye and unit look direction at `t` seconds after
    /// `play`, for an aircraft whose state at world time `w` is
    /// `state(w)`, the world clock now being `clock_s`.
    pub fn camera(
        &self,
        t: f64,
        clock_s: f64,
        state: impl Fn(f64) -> PathState,
    ) -> Option<([f64; 3], [f64; 3])> {
        let f = self.at(t)?;
        let now = Frame::of(&state(clock_s), None);
        let then = Frame::of(&state(clock_s - f.lag), Some(now.ahead));
        let eye = then.point(f.eye);
        let target = now.point(f.target);
        let d = [target[0] - eye[0], target[1] - eye[1], target[2] - eye[2]];
        let n = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        let look = if n < 1e-6 {
            let a = now.ahead;
            [a[0], 0.0, a[1]]
        } else {
            [d[0] / n, d[1] / n, d[2] / n]
        };
        Some((eye, look))
    }
}

/// The unicorn flying beside an aircraft.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Formation {
    pub offset: Offset,
}

impl Formation {
    /// Where the unicorn is and its true heading (°) with the aircraft in
    /// state `s`.
    pub fn place(&self, s: &PathState) -> ([f64; 3], f64) {
        let f = Frame::of(s, None);
        (f.point(self.offset), f.heading_deg())
    }
}

/// The first time (s, world clock) from `from_s` on at which the flight
/// is at most `above_m` over `field_m` in the air, scanning its path in
/// steps of 0.1 s up to `until_s` (an arrival's final approach).
pub fn first_below(
    state: impl Fn(f64) -> PathState,
    from_s: f64,
    until_s: f64,
    field_m: f64,
    above_m: f64,
) -> Option<f64> {
    let mut t = from_s;
    while t <= until_s {
        let s = state(t);
        if !s.ground && s.pos[1] - field_m <= above_m {
            return Some(t);
        }
        t += 0.1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Flying east at 70 m/s, descending 3.7 m/s, from x = 0 at t = 0.
    fn east(t: f64) -> PathState {
        PathState {
            pos: [70.0 * t, 500.0 - 3.7 * t, 0.0],
            vel: [70.0, -3.7, 0.0],
            bank: 0.0,
            ground: false,
        }
    }

    fn key(t: f64, eye: Offset, lag: f64) -> FollowKey {
        FollowKey {
            t,
            eye,
            target: [0.0; 3],
            lag,
        }
    }

    #[test]
    fn offsets_are_right_up_ahead_of_the_heading() {
        let f = Frame::of(&east(0.0), None);
        // Flying east: right is south (+z), ahead is east (+x).
        assert_eq!(f.right(), [0.0, 1.0]);
        let p = f.point([20.0, 5.0, 10.0]);
        assert!((p[0] - 10.0).abs() < 1e-9 && (p[1] - 505.0).abs() < 1e-9);
        assert!((p[2] - 20.0).abs() < 1e-9);
        assert!((f.heading_deg() - 90.0).abs() < 1e-9);
        // Standing: the last heading is kept.
        let stand = PathState {
            vel: [0.0; 3],
            ..east(0.0)
        };
        assert_eq!(Frame::of(&stand, Some([1.0, 0.0])).ahead, [1.0, 0.0]);
    }

    #[test]
    fn the_camera_stays_beside_the_aircraft_at_any_frame_rate() {
        let mut p = FollowPath::default();
        p.add(key(0.0, [30.0, 5.0, 0.0], 0.0));
        p.add(key(10.0, [30.0, 5.0, 0.0], 0.0));
        for t in [0.0, 3.3, 7.77, 10.0, 20.0] {
            let (eye, look) = p.camera(t, 100.0 + t, east).unwrap();
            let plane = east(100.0 + t).pos;
            // 30 m to its right, 5 m up, looking at it.
            assert!((eye[0] - plane[0]).abs() < 1e-6, "{t}: {eye:?} {plane:?}");
            assert!((eye[2] - plane[2] - 30.0).abs() < 1e-6);
            assert!(look[2] < -0.98, "{look:?}");
        }
    }

    #[test]
    fn keys_pass_smoothly_and_rest_at_the_ends() {
        let mut p = FollowPath::default();
        p.add(key(10.0, [0.0, 20.0, -60.0], 0.0));
        p.add(key(0.0, [30.0, 0.0, 0.0], 0.0));
        p.add(key(5.0, [0.0, 0.0, 60.0], 0.0));
        assert_eq!(p.keys()[1].t, 5.0, "kept in time order");
        assert_eq!(p.duration_s(), 10.0);
        // Through every key.
        for k in p.keys().to_vec() {
            assert_eq!(p.at(k.t).unwrap().eye, k.eye);
        }
        // At rest at the ends, moving in the middle, no jump anywhere.
        let x = |t: f64| p.at(t).unwrap().eye;
        let speed = |t: f64| {
            let (a, b) = (x(t - 0.01), x(t + 0.01));
            ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2) + (b[2] - a[2]).powi(2)).sqrt() / 0.02
        };
        assert!(speed(0.01) < 1.0 && speed(9.99) < 1.0);
        // The Catmull-Rom tangent at the middle key: (k2 − k0) / 10 s = 7 m/s.
        assert!((speed(5.0) - 7.0).abs() < 0.05, "{}", speed(5.0));
        let mut prev = x(0.0);
        let mut t = 0.0;
        while t < 10.0 {
            t += 0.04;
            let e = x(t);
            let d = ((e[0] - prev[0]).powi(2) + (e[2] - prev[2]).powi(2)).sqrt();
            assert!(d < 1.5, "a jump at {t}: {d}");
            prev = e;
        }
        // A key at the same time replaces it.
        p.add(key(5.0, [1.0, 1.0, 1.0], 0.0));
        assert_eq!(p.keys().len(), 3);
        assert_eq!(p.at(5.0).unwrap().eye, [1.0, 1.0, 1.0]);
        assert_eq!(FollowPath::default().at(1.0), None);
    }

    #[test]
    fn a_growing_lag_lets_the_aircraft_pull_away() {
        let mut p = FollowPath::default();
        p.add(key(0.0, [25.0, 0.0, -40.0], 0.0));
        p.add(key(4.0, [25.0, 0.0, -40.0], 0.0));
        p.add(key(10.0, [25.0, 0.0, -40.0], 4.0));
        let mut last_eye_x = f64::MIN;
        let mut gap = Vec::new();
        for i in 0..=250 {
            let t = f64::from(i) * 0.04;
            let (eye, look) = p.camera(t, t, east).unwrap();
            // The camera never goes backwards and keeps looking at it.
            assert!(eye[0] >= last_eye_x - 1e-9, "{t}");
            last_eye_x = eye[0];
            assert!(look[0] > 0.0);
            gap.push(east(t).pos[0] - eye[0]);
        }
        assert!((gap[0] - 40.0).abs() < 1e-6);
        // 4 s of lag at 70 m/s: 280 m more.
        assert!((gap[250] - 320.0).abs() < 1e-6, "{}", gap[250]);
        assert!(gap.windows(2).all(|w| w[1] >= w[0] - 1e-9));
    }

    #[test]
    fn the_unicorn_flies_beside_the_wing_and_the_crossing_is_found() {
        let f = Formation {
            offset: [26.0, 2.0, -8.0],
        };
        let (p, h) = f.place(&east(10.0));
        assert!((p[0] - (700.0 - 8.0)).abs() < 1e-9);
        assert!((p[2] - 26.0).abs() < 1e-9);
        assert!((h - 90.0).abs() < 1e-9);
        // 500 m − 3.7 m/s: 300 m above a field at 0 after 54.05 s.
        let t = first_below(east, 0.0, 200.0, 0.0, 300.0).unwrap();
        assert!((t - 54.1).abs() < 0.11, "{t}");
        assert_eq!(first_below(east, 0.0, 30.0, 0.0, 300.0), None);
    }
}
