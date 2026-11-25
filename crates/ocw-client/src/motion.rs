//! How smoothly the riders move as drawn (spec 6.3, v5.40's measure): the
//! change of each rider's drawn velocity from one frame to the next, and
//! the snapshots that moved the own prediction by more than
//! [`CORRECTION_M`].
//!
//! A rider riding steadily changes its velocity by a few mm/s per frame;
//! a snapshot taken at the wrong time pulls the own rider back or makes
//! the others surge, and shows here as a large change. The browser keeps
//! one for a remote ride (`net.stats`, `window.ocw.netStats().motion`), so
//! the multiplayer test measures what the rider sees (spec 14 Phase 10,
//! 10G *B3 worker*); `client.rs`'s motion probe is the same measure.

use std::collections::BTreeMap;

use ocw_core::RiderId;

/// A snapshot that moves the own prediction by more than this (m) is a
/// correction.
pub const CORRECTION_M: f64 = 0.02;

/// Frames at the start that are not counted (s): joining, the first
/// snapshots, the time machine's jump.
pub const WARMUP_S: f64 = 5.0;

#[derive(Debug, Clone, Copy, Default)]
struct Track {
    at_s: f64,
    pos: [f64; 3],
    vel: Option<[f64; 3]>,
}

#[derive(Debug, Clone, Copy, Default)]
struct Sum {
    n: u64,
    sq: f64,
    max: f64,
}

impl Sum {
    fn add(&mut self, x: f64) {
        self.n += 1;
        self.sq += x * x;
        self.max = self.max.max(x);
    }

    fn rms(&self) -> f64 {
        if self.n == 0 {
            0.0
        } else {
            (self.sq / self.n as f64).sqrt()
        }
    }
}

/// The measure (see the module docs).
#[derive(Debug, Clone, Default)]
pub struct MotionMeter {
    first_s: Option<f64>,
    own: Option<Track>,
    others: BTreeMap<RiderId, Track>,
    own_dv: Sum,
    others_dv: Sum,
    snapshots: u64,
    corrections: u64,
    error_sum: f64,
}

/// What a [`MotionMeter`] measured.
#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize)]
pub struct MotionSummary {
    /// Frames counted (after [`WARMUP_S`]).
    pub frames: u64,
    /// The own rider's change of drawn velocity per frame (m/s).
    pub own_dv_rms: f64,
    pub own_dv_max: f64,
    /// Snapshots counted, those that moved the prediction by more than
    /// [`CORRECTION_M`], and the mean prediction error at a snapshot (mm).
    pub snapshots: u64,
    pub corrections: u64,
    pub mean_error_mm: f64,
    /// The same change of velocity for the other riders (m/s).
    pub others_dv_rms: f64,
    pub others_dv_max: f64,
}

fn step(t: &mut Track, at_s: f64, pos: [f64; 3]) -> Option<f64> {
    let dt = at_s - t.at_s;
    if dt <= 1e-6 {
        return None;
    }
    let v = [
        (pos[0] - t.pos[0]) / dt,
        (pos[1] - t.pos[1]) / dt,
        (pos[2] - t.pos[2]) / dt,
    ];
    let dv = t
        .vel
        .map(|p| ((v[0] - p[0]).powi(2) + (v[1] - p[1]).powi(2) + (v[2] - p[2]).powi(2)).sqrt());
    *t = Track {
        at_s,
        pos,
        vel: Some(v),
    };
    dv
}

impl MotionMeter {
    fn counting(&mut self, at_s: f64) -> bool {
        let first = *self.first_s.get_or_insert(at_s);
        at_s - first >= WARMUP_S
    }

    /// One drawn frame at `at_s`: the own rider's place and the others'.
    pub fn frame(&mut self, at_s: f64, own: Option<[f64; 3]>, others: &[(RiderId, [f64; 3])]) {
        let count = self.counting(at_s);
        match (own, &mut self.own) {
            (Some(p), Some(t)) => {
                if let Some(dv) = step(t, at_s, p)
                    && count
                {
                    self.own_dv.add(dv);
                }
            }
            (Some(p), None) => {
                self.own = Some(Track {
                    at_s,
                    pos: p,
                    vel: None,
                })
            }
            (None, _) => self.own = None,
        }
        // Riders who left start again when they come back.
        self.others
            .retain(|id, _| others.iter().any(|(o, _)| o == id));
        for (id, p) in others {
            match self.others.get_mut(id) {
                Some(t) => {
                    if let Some(dv) = step(t, at_s, *p)
                        && count
                    {
                        self.others_dv.add(dv);
                    }
                }
                None => {
                    self.others.insert(
                        *id,
                        Track {
                            at_s,
                            pos: *p,
                            vel: None,
                        },
                    );
                }
            }
        }
    }

    /// A snapshot reconciled the own prediction; `error_m`: by how much it
    /// moved it.
    pub fn snapshot(&mut self, at_s: f64, error_m: f64) {
        if self.first_s.is_none_or(|f| at_s - f < WARMUP_S) {
            return;
        }
        self.snapshots += 1;
        self.error_sum += error_m.abs();
        if error_m.abs() > CORRECTION_M {
            self.corrections += 1;
        }
    }

    pub fn summary(&self) -> MotionSummary {
        MotionSummary {
            frames: self.own_dv.n,
            own_dv_rms: self.own_dv.rms(),
            own_dv_max: self.own_dv.max,
            snapshots: self.snapshots,
            corrections: self.corrections,
            mean_error_mm: if self.snapshots == 0 {
                0.0
            } else {
                self.error_sum / self.snapshots as f64 * 1000.0
            },
            others_dv_rms: self.others_dv.rms(),
            others_dv_max: self.others_dv.max,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Steady riders change nothing; one jump shows in its two frames; the
    /// warm-up is not counted.
    #[test]
    fn a_steady_ride_is_smooth_and_a_jump_is_seen() {
        let mut m = MotionMeter::default();
        // Exact in binary: 5 s are frame 80.
        let dt = 0.0625;
        for i in 0..400 {
            let t = f64::from(i) * dt;
            let mut x = 10.0 * t;
            // A 4 cm jump back at frame 250 (counted), one at 25 (warm-up).
            if i == 250 || i == 25 {
                x -= 0.04;
            }
            m.frame(t, Some([x, 0.0, 0.0]), &[(RiderId(7), [0.0, 0.0, 8.0 * t])]);
            if i % 2 == 0 {
                m.snapshot(t, if i == 250 { 0.04 } else { 0.001 });
            }
        }
        let s = m.summary();
        assert_eq!(s.frames, 320);
        // The jump: v changes by −0.64 m/s, then +1.28, then −0.64.
        let j = 0.04 / dt;
        assert!((s.own_dv_max - 2.0 * j).abs() < 1e-9, "{s:?}");
        let expect = ((j * j * 6.0) / 320.0).sqrt();
        assert!((s.own_dv_rms - expect).abs() < 1e-9, "{s:?}");
        assert!(s.others_dv_max < 1e-9, "{s:?}");
        assert_eq!(s.corrections, 1);
        assert_eq!(s.snapshots, 160);
        assert!((s.mean_error_mm - (0.04 + 159.0 * 0.001) / 160.0 * 1000.0).abs() < 1e-9);
    }

    /// The same measure as `client.rs`'s motion probe: the second
    /// difference over the frame time for even frames; uneven frames use
    /// their own times.
    #[test]
    fn uneven_frames_measure_the_velocity_change() {
        let mut m = MotionMeter::default();
        let mut t = 0.0;
        for i in 0..300 {
            t += if i % 2 == 0 { 0.03 } else { 0.05 };
            m.frame(t, Some([5.0 * t, 0.0, 0.0]), &[]);
        }
        assert!(
            m.summary().own_dv_max < 1e-9,
            "a steady speed at uneven frames is smooth"
        );
        // A rider leaves and comes back: no jump from the gap.
        let mut m = MotionMeter::default();
        for i in 0..300 {
            let t = f64::from(i) * 0.02;
            let others: Vec<_> = if (150..160).contains(&i) {
                vec![]
            } else {
                vec![(
                    RiderId(2),
                    [3.0 * t + if i >= 160 { 50.0 } else { 0.0 }, 0.0, 0.0],
                )]
            };
            m.frame(t, None, &others);
        }
        assert!(m.summary().others_dv_max < 1e-9);
    }
}
