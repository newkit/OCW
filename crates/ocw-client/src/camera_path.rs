//! A guided camera path for flyover videos (spec §14 Phase 13, G5; the art
//! project's build diary): keyframes of the camera's pose (the eye and
//! where it looks), played back along a smooth spline in a given time with
//! an easing.
//!
//! - **Position:** a centripetal Catmull-Rom spline through the eyes (no
//!   loops or overshoots between unevenly spaced keys; the ends repeat
//!   their key).
//! - **Look direction:** yaw and pitch (unwrapped, so the camera turns the
//!   short way) on a Catmull-Rom spline over the same parameter.
//! - **Timing:** by arc length – the camera moves at an even speed, eased
//!   at the ends ([`Ease`]); turning on the spot counts as
//!   [`TURN_M_PER_RAD`] metres per radian, so a pan between two keys at
//!   one place takes its time too.
//!
//! Pure (no Bevy, no files): the app adds the camera, the console and the
//! files. The pose depends only on the time since the start, so a capture
//! at a fixed game-time step (`ocw-desktop --capture`) renders a flyover
//! smoothly and the same every run. The text format ([`CameraPath::to_text`],
//! [`CameraPath::parse`]): one `key x y z look_x look_y look_z` line per
//! keyframe (local metres, y up), `#` comments, optional `seconds S` and
//! `ease NAME` lines with the playback's defaults.

/// A pan on the spot counts as this many metres of path per radian.
pub const TURN_M_PER_RAD: f64 = 15.0;
/// Samples per segment for the arc-length table.
const SAMPLES: usize = 48;
/// The default playback time (s).
pub const DEFAULT_SECONDS: f64 = 20.0;

/// The camera's pose: the eye (local metres, y up) and the unit direction
/// it looks along.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub eye: [f64; 3],
    pub look: [f64; 3],
}

impl Pose {
    /// Yaw (rad, from −z towards +x: 0 = north, π/2 = east) and pitch
    /// (rad, up) of the look direction.
    pub fn yaw_pitch(&self) -> (f64, f64) {
        let [x, y, z] = normalized(self.look);
        (x.atan2(-z), y.clamp(-1.0, 1.0).asin())
    }

    /// The look direction of a yaw and pitch ([`Pose::yaw_pitch`]).
    pub fn look_of(yaw: f64, pitch: f64) -> [f64; 3] {
        let (sy, cy) = yaw.sin_cos();
        let (sp, cp) = pitch.sin_cos();
        [sy * cp, sp, -cy * cp]
    }
}

fn normalized(v: [f64; 3]) -> [f64; 3] {
    let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if n < 1e-12 {
        [0.0, 0.0, -1.0]
    } else {
        [v[0] / n, v[1] / n, v[2] / n]
    }
}

/// How the playback speeds up and slows down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Ease {
    /// An even speed from start to end.
    Linear,
    /// Smoothstep: starts and stops gently.
    #[default]
    Smooth,
    /// Smootherstep: an even gentler start and stop.
    InOut,
}

impl Ease {
    pub const NAMES: [&'static str; 3] = ["linear", "smooth", "in-out"];

    pub fn name(self) -> &'static str {
        match self {
            Ease::Linear => "linear",
            Ease::Smooth => "smooth",
            Ease::InOut => "in-out",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "linear" => Some(Ease::Linear),
            "smooth" => Some(Ease::Smooth),
            "in-out" | "inout" => Some(Ease::InOut),
            _ => None,
        }
    }

    /// The share of the path at the share `t` (0–1) of the time.
    pub fn apply(self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Ease::Linear => t,
            Ease::Smooth => t * t * (3.0 - 2.0 * t),
            Ease::InOut => t * t * t * (t * (6.0 * t - 15.0) + 10.0),
        }
    }
}

/// The keyframes of a path and the playback's defaults.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CameraPath {
    pub keys: Vec<Pose>,
    /// The playback time from the file (`None`: [`DEFAULT_SECONDS`]).
    pub seconds: Option<f64>,
    /// The easing from the file (`None`: [`Ease::Smooth`]).
    pub ease: Option<Ease>,
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn len(a: [f64; 3]) -> f64 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

/// One centripetal Catmull-Rom segment between `p1` and `p2` at `u` (0–1).
fn catmull_rom(p0: [f64; 3], p1: [f64; 3], p2: [f64; 3], p3: [f64; 3], u: f64) -> [f64; 3] {
    // Knot spacing |Δp|^0.5 (centripetal); repeated points get a tiny one.
    let knot = |a: [f64; 3], b: [f64; 3]| len(sub(b, a)).sqrt().max(1e-4);
    let t0 = 0.0;
    let t1 = t0 + knot(p0, p1);
    let t2 = t1 + knot(p1, p2);
    let t3 = t2 + knot(p2, p3);
    let t = t1 + (t2 - t1) * u;
    let lerp = |a: [f64; 3], b: [f64; 3], ta: f64, tb: f64| {
        let w = if (tb - ta).abs() < 1e-12 {
            0.0
        } else {
            (t - ta) / (tb - ta)
        };
        [
            a[0] + (b[0] - a[0]) * w,
            a[1] + (b[1] - a[1]) * w,
            a[2] + (b[2] - a[2]) * w,
        ]
    };
    let a1 = lerp(p0, p1, t0, t1);
    let a2 = lerp(p1, p2, t1, t2);
    let a3 = lerp(p2, p3, t2, t3);
    let b1 = lerp(a1, a2, t0, t2);
    let b2 = lerp(a2, a3, t1, t3);
    lerp(b1, b2, t1, t2)
}

/// A uniform Catmull-Rom segment of scalars (the yaw and the pitch).
fn catmull_rom_1(a: f64, b: f64, c: f64, d: f64, u: f64) -> f64 {
    let (u2, u3) = (u * u, u * u * u);
    0.5 * ((2.0 * b)
        + (-a + c) * u
        + (2.0 * a - 5.0 * b + 4.0 * c - d) * u2
        + (-a + 3.0 * b - 3.0 * c + d) * u3)
}

/// A path ready to play: the keys, their unwrapped angles and the
/// arc-length table.
#[derive(Debug, Clone, PartialEq)]
pub struct Spline {
    keys: Vec<Pose>,
    /// Yaw and pitch per key, the yaw unwrapped (the short way round).
    angles: Vec<(f64, f64)>,
    /// Per segment: the cumulative length at each sample (from the start).
    table: Vec<Vec<f64>>,
    length: f64,
}

impl Spline {
    /// The spline through `keys` (`None` without keys).
    pub fn new(keys: &[Pose]) -> Option<Self> {
        if keys.is_empty() {
            return None;
        }
        let mut angles: Vec<(f64, f64)> = Vec::with_capacity(keys.len());
        for k in keys {
            let (mut yaw, pitch) = k.yaw_pitch();
            if let Some((prev, _)) = angles.last() {
                let tau = std::f64::consts::TAU;
                yaw = prev + (yaw - prev + std::f64::consts::PI).rem_euclid(tau)
                    - std::f64::consts::PI;
            }
            angles.push((yaw, pitch));
        }
        let mut s = Spline {
            keys: keys.to_vec(),
            angles,
            table: Vec::new(),
            length: 0.0,
        };
        let mut total = 0.0;
        for seg in 0..keys.len().saturating_sub(1) {
            let mut row = Vec::with_capacity(SAMPLES + 1);
            row.push(total);
            let mut prev = s.at(seg, 0.0);
            for i in 1..=SAMPLES {
                let p = s.at(seg, i as f64 / SAMPLES as f64);
                total +=
                    len(sub(p.0, prev.0)) + TURN_M_PER_RAD * ((p.1 - prev.1).hypot(p.2 - prev.2));
                row.push(total);
                prev = p;
            }
            s.table.push(row);
        }
        s.length = total;
        Some(s)
    }

    /// The path's length (m, with the turns counted, [`TURN_M_PER_RAD`]).
    pub fn length(&self) -> f64 {
        self.length
    }

    /// Eye, yaw and pitch on segment `seg` at `u`.
    fn at(&self, seg: usize, u: f64) -> ([f64; 3], f64, f64) {
        let n = self.keys.len();
        let k = |i: isize| self.keys[i.clamp(0, n as isize - 1) as usize].eye;
        let a = |i: isize| self.angles[i.clamp(0, n as isize - 1) as usize];
        let i = seg as isize;
        let eye = catmull_rom(k(i - 1), k(i), k(i + 1), k(i + 2), u);
        let (a0, a1, a2, a3) = (a(i - 1), a(i), a(i + 1), a(i + 2));
        let yaw = catmull_rom_1(a0.0, a1.0, a2.0, a3.0, u);
        let pitch = catmull_rom_1(a0.1, a1.1, a2.1, a3.1, u).clamp(-1.55, 1.55);
        (eye, yaw, pitch)
    }

    /// The pose at the share `x` (0–1) of the path's length.
    pub fn at_share(&self, x: f64) -> Pose {
        if self.table.is_empty() {
            return self.keys[0];
        }
        let want = x.clamp(0.0, 1.0) * self.length;
        // The segment, then the sample, then linear within the sample.
        let seg = self
            .table
            .iter()
            .position(|row| *row.last().expect("samples") >= want)
            .unwrap_or(self.table.len() - 1);
        let row = &self.table[seg];
        let i = row
            .windows(2)
            .position(|w| w[1] >= want)
            .unwrap_or(SAMPLES - 1);
        let (a, b) = (row[i], row[i + 1]);
        let f = if b - a > 1e-12 {
            (want - a) / (b - a)
        } else {
            0.0
        };
        let u = (i as f64 + f) / SAMPLES as f64;
        let (eye, yaw, pitch) = self.at(seg, u);
        Pose {
            eye,
            look: Pose::look_of(yaw, pitch),
        }
    }
}

/// A playback: the spline, when it started (s, the app's game time), how
/// long it takes and its easing.
#[derive(Debug, Clone, PartialEq)]
pub struct Playback {
    pub spline: Spline,
    pub start_s: f64,
    pub seconds: f64,
    pub ease: Ease,
}

impl Playback {
    /// The pose at game time `now_s`; `None` once it is over.
    pub fn pose(&self, now_s: f64) -> Option<Pose> {
        let t = (now_s - self.start_s) / self.seconds.max(1e-3);
        (t <= 1.0).then(|| self.spline.at_share(self.ease.apply(t.max(0.0))))
    }

    /// The share played at `now_s` (0–1).
    pub fn progress(&self, now_s: f64) -> f64 {
        ((now_s - self.start_s) / self.seconds.max(1e-3)).clamp(0.0, 1.0)
    }
}

impl CameraPath {
    /// The keyframes as text (see the module docs).
    pub fn to_text(&self) -> String {
        let mut s = String::from(
            "# OCW camera path (spec §14 Phase 13 G5): one keyframe per line,\n\
             # key x y z look_x look_y look_z (local metres, y up)\n",
        );
        if let Some(t) = self.seconds {
            s.push_str(&format!("seconds {t}\n"));
        }
        if let Some(e) = self.ease {
            s.push_str(&format!("ease {}\n", e.name()));
        }
        for k in &self.keys {
            let l = normalized(k.look);
            s.push_str(&format!(
                "key {:.3} {:.3} {:.3} {:.6} {:.6} {:.6}\n",
                k.eye[0], k.eye[1], k.eye[2], l[0], l[1], l[2]
            ));
        }
        s
    }

    /// Reads [`CameraPath::to_text`]'s format; an error names the line.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut path = CameraPath::default();
        for (n, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut words = line.split_whitespace();
            let what = words.next().unwrap_or_default();
            let rest: Vec<&str> = words.collect();
            let bad = |why: &str| format!("line {}: {why}: {line}", n + 1);
            match what {
                "key" => {
                    let v: Vec<f64> = rest
                        .iter()
                        .map(|w| w.parse::<f64>().map_err(|_| bad("not a number")))
                        .collect::<Result<_, _>>()?;
                    if v.len() != 6 || v.iter().any(|x| !x.is_finite()) {
                        return Err(bad("a key needs x y z look_x look_y look_z"));
                    }
                    if len([v[3], v[4], v[5]]) < 1e-9 {
                        return Err(bad("the look direction is zero"));
                    }
                    path.keys.push(Pose {
                        eye: [v[0], v[1], v[2]],
                        look: normalized([v[3], v[4], v[5]]),
                    });
                }
                "seconds" => {
                    let t = rest
                        .first()
                        .and_then(|w| w.parse::<f64>().ok())
                        .filter(|t| *t > 0.0 && t.is_finite())
                        .ok_or_else(|| bad("seconds needs a positive number"))?;
                    path.seconds = Some(t);
                }
                "ease" => {
                    let e = rest
                        .first()
                        .and_then(|w| Ease::parse(w))
                        .ok_or_else(|| bad("ease is linear, smooth or in-out"))?;
                    path.ease = Some(e);
                }
                _ => return Err(bad("unknown line")),
            }
        }
        Ok(path)
    }

    /// One line per key for the console's `camera.path list`.
    pub fn list(&self) -> Vec<String> {
        self.keys
            .iter()
            .enumerate()
            .map(|(i, k)| {
                let (yaw, pitch) = k.yaw_pitch();
                format!(
                    "key {}: x {:.1} y {:.1} z {:.1}, heading {:.0}°, pitch {:.0}°",
                    i + 1,
                    k.eye[0],
                    k.eye[1],
                    k.eye[2],
                    yaw.to_degrees().round().rem_euclid(360.0) + 0.0,
                    pitch.to_degrees()
                )
            })
            .collect()
    }

    /// A playback from `start_s` (the file's time and easing unless given).
    pub fn play(&self, start_s: f64, seconds: Option<f64>, ease: Option<Ease>) -> Option<Playback> {
        Some(Playback {
            spline: Spline::new(&self.keys)?,
            start_s,
            seconds: seconds.or(self.seconds).unwrap_or(DEFAULT_SECONDS),
            ease: ease.or(self.ease).unwrap_or_default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(x: f64, y: f64, z: f64, yaw_deg: f64) -> Pose {
        Pose {
            eye: [x, y, z],
            look: Pose::look_of(yaw_deg.to_radians(), -0.2),
        }
    }

    fn close(a: [f64; 3], b: [f64; 3], tol: f64) -> bool {
        len(sub(a, b)) < tol
    }

    fn path() -> CameraPath {
        CameraPath {
            keys: vec![
                key(0.0, 100.0, 0.0, 0.0),
                key(100.0, 120.0, -50.0, 60.0),
                key(300.0, 110.0, -60.0, 120.0),
                key(400.0, 150.0, 40.0, 170.0),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn the_spline_passes_through_every_key() {
        let p = path();
        let s = Spline::new(&p.keys).unwrap();
        assert!(close(s.at_share(0.0).eye, p.keys[0].eye, 1e-9));
        assert!(close(s.at_share(1.0).eye, p.keys[3].eye, 1e-6));
        for (seg, k) in p.keys.iter().enumerate().take(3) {
            let (eye, yaw, pitch) = s.at(seg, 0.0);
            assert!(close(eye, k.eye, 1e-9), "{seg}");
            let (ky, kp) = k.yaw_pitch();
            assert!((yaw - ky).abs() < 1e-9 && (pitch - kp).abs() < 1e-9);
        }
    }

    #[test]
    fn the_motion_is_smooth_and_even() {
        let s = Spline::new(&path().keys).unwrap();
        // Linear: equal steps of time, about equal steps of path (the
        // arc-length table), never a jump.
        let n = 400;
        let mut prev = s.at_share(0.0);
        let mut steps = Vec::new();
        for i in 1..=n {
            let p = s.at_share(f64::from(i) / f64::from(n));
            steps.push(len(sub(p.eye, prev.eye)));
            // The look direction turns a little per step, never flips.
            let dot: f64 = (0..3).map(|k| p.look[k] * prev.look[k]).sum();
            assert!(dot > 0.99, "{i}: {dot}");
            prev = p;
        }
        let mean = steps.iter().sum::<f64>() / steps.len() as f64;
        assert!(steps.iter().all(|d| *d < mean * 2.0), "even: {mean}");
        // Second differences small: no kink at the keys.
        for w in steps.windows(2) {
            assert!((w[1] - w[0]).abs() < mean * 0.5, "{w:?}");
        }
    }

    #[test]
    fn the_look_turns_the_short_way() {
        // From 170° to −170° (190°): 20° through south, not 340°.
        let keys = [key(0.0, 50.0, 0.0, 170.0), key(100.0, 50.0, 0.0, -170.0)];
        let s = Spline::new(&keys).unwrap();
        let mid = s.at_share(0.5);
        let (yaw, _) = mid.yaw_pitch();
        assert!(
            (yaw.to_degrees().rem_euclid(360.0) - 180.0).abs() < 2.0,
            "{}",
            yaw.to_degrees()
        );
        // A pan on the spot takes time too.
        let pan = [key(0.0, 50.0, 0.0, 0.0), key(0.0, 50.0, 0.0, 90.0)];
        let s = Spline::new(&pan).unwrap();
        assert!((s.length() - TURN_M_PER_RAD * std::f64::consts::FRAC_PI_2).abs() < 0.5);
        let (yaw, _) = s.at_share(0.5).yaw_pitch();
        assert!(
            (yaw.to_degrees() - 45.0).abs() < 3.0,
            "{}",
            yaw.to_degrees()
        );
    }

    #[test]
    fn easing_and_playback() {
        for e in [Ease::Linear, Ease::Smooth, Ease::InOut] {
            assert_eq!(e.apply(0.0), 0.0);
            assert!((e.apply(1.0) - 1.0).abs() < 1e-12);
            assert!((e.apply(0.5) - 0.5).abs() < 1e-12);
            assert_eq!(Ease::parse(e.name()), Some(e));
        }
        // Eased: slow at the ends.
        assert!(
            Ease::Smooth.apply(0.05) < 0.05 && Ease::InOut.apply(0.05) < Ease::Smooth.apply(0.05)
        );
        let p = path();
        let play = p.play(10.0, Some(8.0), Some(Ease::Linear)).unwrap();
        assert!(close(play.pose(10.0).unwrap().eye, p.keys[0].eye, 1e-9));
        assert!(close(play.pose(18.0).unwrap().eye, p.keys[3].eye, 1e-6));
        assert_eq!(play.pose(18.1), None, "over");
        assert!((play.progress(14.0) - 0.5).abs() < 1e-12);
        // Defaults: the file's, else 20 s smooth.
        let d = p.play(0.0, None, None).unwrap();
        assert_eq!((d.seconds, d.ease), (DEFAULT_SECONDS, Ease::Smooth));
        assert!(CameraPath::default().play(0.0, None, None).is_none());
        // One key: it stands there.
        let one = CameraPath {
            keys: vec![p.keys[1]],
            ..Default::default()
        };
        let still = one.play(0.0, Some(5.0), None).unwrap();
        assert_eq!(still.pose(2.5).unwrap(), p.keys[1]);
    }

    #[test]
    fn keyframes_save_and_load() {
        let mut p = path();
        p.seconds = Some(12.5);
        p.ease = Some(Ease::InOut);
        let text = p.to_text();
        let back = CameraPath::parse(&text).unwrap();
        assert_eq!(back.seconds, Some(12.5));
        assert_eq!(back.ease, Some(Ease::InOut));
        assert_eq!(back.keys.len(), 4);
        for (a, b) in p.keys.iter().zip(&back.keys) {
            assert!(close(a.eye, b.eye, 1e-3) && close(a.look, b.look, 1e-5));
        }
        assert_eq!(back.list().len(), 4);
        assert!(back.list()[1].starts_with("key 2: x 100.0 y 120.0 z -50.0, heading 60°"));
        // Errors name the line.
        let e = CameraPath::parse("# x\nkey 1 2 3\n").unwrap_err();
        assert!(e.starts_with("line 2:"), "{e}");
        assert!(CameraPath::parse("key 1 2 3 0 0 0").is_err());
        assert!(CameraPath::parse("ease wobbly").is_err());
        assert!(CameraPath::parse("seconds -3").is_err());
        assert!(CameraPath::parse("fly 1").is_err());
        assert_eq!(CameraPath::parse("").unwrap(), CameraPath::default());
    }

    #[test]
    fn yaw_and_pitch_round_trip() {
        for (yaw, pitch) in [(0.0, 0.0), (1.0, -0.3), (-2.5, 0.6), (3.0, 1.2)] {
            let p = Pose {
                eye: [0.0; 3],
                look: Pose::look_of(yaw, pitch),
            };
            let (y, q) = p.yaw_pitch();
            assert!((y - yaw).abs() < 1e-9 && (q - pitch).abs() < 1e-9);
        }
        // North is −z, east +x.
        let east = Pose::look_of(std::f64::consts::FRAC_PI_2, 0.0);
        assert!(close(east, [1.0, 0.0, 0.0], 1e-12));
    }
}
