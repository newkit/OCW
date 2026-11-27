//! The beating heart next to the heart rate (spec 3.2 *Heart rate*;
//! the user, 2026-11-27: "make the heart of the HR pounding in pink when
//! HR increases") – the pure part, no Bevy: the trend of the measured
//! heart rate ([`HrTrend`]: rising, steady or falling, with hysteresis),
//! the beat timed by the measured BPM ([`HeartBeat`]: phase-continuous
//! when the BPM changes) and what the icon looks like now ([`HeartLook`]:
//! size, glow, how pink). The HUD draws it ([`crate::heart_hud`]); the
//! `hr.test` console command (`ocw_ui::hr_test`) plays a [`HrProgram`].
//!
//! The trend: an exponential average of the heart rate (time constant
//! [`SMOOTH_S`]) compared with the same average [`LOOKBACK_S`] ago –
//! "rising" from +[`RISE_ON`] bpm per 10 s, back to "steady" below
//! +[`RISE_OFF`] or when the last [`RECENT_S`] rose less than that (the
//! rate levelled off or turned down: the 10 s comparison alone would stay
//! "rising" for a while after a peak); the same downwards for "falling".
//! A state holds at least
//! [`HOLD_S`]. A missing heart rate or a gap of more than [`GAP_S`] between
//! readings starts over (no trend until [`MIN_SPAN_S`] of readings).

use std::collections::VecDeque;

/// The average's time constant (s).
pub const SMOOTH_S: f64 = 4.0;
/// How far back the average is compared (s).
pub const LOOKBACK_S: f64 = 10.0;
/// Readings needed before there is a trend (s).
pub const MIN_SPAN_S: f64 = 6.0;
/// "Rising" from this rise per [`LOOKBACK_S`] (bpm).
pub const RISE_ON: f64 = 2.0;
/// … and back to "steady" below this one (bpm).
pub const RISE_OFF: f64 = 0.5;
/// The recent span that ends a rise or a fall early (s).
pub const RECENT_S: f64 = 5.0;
/// A state holds at least this long (s).
pub const HOLD_S: f64 = 2.0;
/// A longer gap between readings starts over (s).
pub const GAP_S: f64 = 5.0;
/// History spacing (s).
const STEP_S: f64 = 0.5;

/// Where the heart rate goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Trend {
    Rising,
    #[default]
    Steady,
    Falling,
}

/// The heart rate's trend (feed it every frame or every reading).
#[derive(Debug, Clone, Default)]
pub struct HrTrend {
    /// The average and the time of the last update.
    avg: Option<(f64, f64)>,
    /// The average every [`STEP_S`] (time, value), oldest first.
    history: VecDeque<(f64, f64)>,
    /// When the average started.
    since: f64,
    trend: Trend,
    /// When the trend last changed.
    changed: f64,
}

impl HrTrend {
    /// The trend now.
    pub fn trend(&self) -> Trend {
        self.trend
    }

    /// The rise of the average over [`LOOKBACK_S`] (bpm; `None`: not yet
    /// [`MIN_SPAN_S`] of readings).
    pub fn rise(&self) -> Option<f64> {
        self.rise_over(LOOKBACK_S)
    }

    /// The rise of the average over the last `span_s` (≤ [`LOOKBACK_S`]),
    /// per [`LOOKBACK_S`] (bpm).
    fn rise_over(&self, span_s: f64) -> Option<f64> {
        let (t, now) = self.avg?;
        if t - self.since < MIN_SPAN_S {
            return None;
        }
        let target = t - span_s;
        // The oldest entry not older than the lookback (or the oldest).
        let &(t0, then) = self
            .history
            .iter()
            .find(|(ht, _)| *ht >= target)
            .or(self.history.front())?;
        let span = t - t0;
        (span > 0.0).then(|| (now - then) * LOOKBACK_S / span)
    }

    /// Takes the heart rate `bpm` at time `t` (s; `None`: no heart rate)
    /// and returns the trend.
    pub fn update(&mut self, t: f64, bpm: Option<f64>) -> Trend {
        let Some(bpm) = bpm.filter(|b| b.is_finite() && *b > 0.0) else {
            *self = Self::default();
            return self.trend;
        };
        match self.avg {
            Some((last, _)) if t < last || t - last > GAP_S => {
                *self = Self::default();
                self.start(t, bpm);
            }
            None => self.start(t, bpm),
            Some((last, avg)) => {
                let a = 1.0 - (-(t - last) / SMOOTH_S).exp();
                self.avg = Some((t, avg + (bpm - avg) * a));
            }
        }
        let (t, avg) = self.avg.expect("set");
        if self.history.back().is_none_or(|(ht, _)| t - ht >= STEP_S) {
            self.history.push_back((t, avg));
        }
        while self
            .history
            .get(1)
            .is_some_and(|(ht, _)| *ht <= t - LOOKBACK_S)
        {
            self.history.pop_front();
        }
        let recent = self.rise_over(RECENT_S).unwrap_or(0.0);
        let want = match (self.rise(), self.trend) {
            (None, _) => Trend::Steady,
            (Some(r), Trend::Rising) if r >= RISE_OFF && recent >= RISE_OFF => Trend::Rising,
            (Some(r), Trend::Falling) if r <= -RISE_OFF && recent <= -RISE_OFF => Trend::Falling,
            (Some(r), _) if r >= RISE_ON => Trend::Rising,
            (Some(r), _) if r <= -RISE_ON => Trend::Falling,
            _ => Trend::Steady,
        };
        if want != self.trend && t - self.changed >= HOLD_S {
            self.trend = want;
            self.changed = t;
        }
        self.trend
    }

    fn start(&mut self, t: f64, bpm: f64) {
        self.avg = Some((t, bpm));
        self.since = t;
        self.changed = t;
    }
}

/// The beat: a phase in beats (0–1) that runs at the measured BPM.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct HeartBeat {
    /// The phase (beats, 0 ≤ phase < 1); a beat starts at 0.
    pub phase: f64,
}

/// Part of a beat the pulse takes.
pub const PULSE_WIDTH: f64 = 0.35;

impl HeartBeat {
    /// Advances by `dt` s at `bpm`; `true` when a beat started.
    pub fn advance(&mut self, dt: f64, bpm: f64) -> bool {
        let step = (dt.max(0.0) * bpm.clamp(0.0, 260.0) / 60.0).min(0.99);
        let next = self.phase + step;
        self.phase = next.fract();
        next >= 1.0
    }

    /// The pulse now: 0 at rest, 1 at its peak (early in the beat).
    pub fn pulse(&self) -> f64 {
        pulse_at(self.phase)
    }
}

/// The pulse at `phase`: a quick swell and fall in the first
/// [`PULSE_WIDTH`] of the beat (sin²), 0 after.
pub fn pulse_at(phase: f64) -> f64 {
    if (0.0..PULSE_WIDTH).contains(&phase) {
        (std::f64::consts::PI * phase / PULSE_WIDTH).sin().powi(2)
    } else {
        0.0
    }
}

/// The soft beat's size at its peak.
pub const SOFT_SCALE: f64 = 1.15;
/// The pounding beat's size at its peak (while the heart rate rises).
pub const POUND_SCALE: f64 = 1.3;
/// How fast the colour turns (per s; 0.4 s from one to the other).
const PINK_RATE: f64 = 2.5;

/// How the heart icon looks now.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct HeartLook {
    /// Its size (1: at rest).
    pub scale: f64,
    /// The glow around it (0–1).
    pub glow: f64,
    /// How pink (0: the normal colour, 1: pink).
    pub pink: f64,
}

/// The heart: trend, beat and colour together (one per HUD).
#[derive(Debug, Clone, Default)]
pub struct Heart {
    pub trend: HrTrend,
    pub beat: HeartBeat,
    pink: f64,
    last_t: Option<f64>,
}

impl Heart {
    /// Takes the heart rate at `t` (s) and returns the look; `None`
    /// without a heart rate (no heart). `still`: reduced motion – the
    /// colour changes, the heart does not beat.
    pub fn update(&mut self, t: f64, bpm: Option<f64>, still: bool) -> Option<HeartLook> {
        let dt = self.last_t.map_or(0.0, |l| (t - l).clamp(0.0, 0.5));
        self.last_t = Some(t);
        let trend = self.trend.update(t, bpm);
        let bpm = bpm.filter(|b| b.is_finite() && *b > 0.0)?;
        self.beat.advance(dt, bpm);
        let target = if trend == Trend::Rising { 1.0 } else { 0.0 };
        let k = (PINK_RATE * dt).min(1.0);
        self.pink += (target - self.pink).clamp(-k, k);
        let pulse = if still { 0.0 } else { self.beat.pulse() };
        let peak = SOFT_SCALE + (POUND_SCALE - SOFT_SCALE) * self.pink;
        Some(HeartLook {
            scale: 1.0 + (peak - 1.0) * pulse,
            glow: self.pink * (0.45 + 0.55 * pulse),
            pink: self.pink,
        })
    }

    /// The heart rate rises now.
    pub fn rising(&self) -> bool {
        self.trend.trend() == Trend::Rising
    }
}

/// A test heart rate (console `hr.test`): `from` bpm for `hold_s` s, then
/// to `to` bpm over `ramp_s` s, then steady; a reading per second like a
/// strap, with a made-up jitter of ± `noise` bpm.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct HrProgram {
    pub hold_s: f64,
    pub from: f64,
    pub to: f64,
    pub ramp_s: f64,
    pub noise: f64,
}

impl HrProgram {
    /// A steady heart rate.
    pub fn steady(bpm: f64) -> Self {
        Self {
            from: bpm,
            to: bpm,
            ..Self::default()
        }
    }

    /// The reading `t` s after the start (whole seconds: one per second).
    pub fn at(&self, t: f64) -> f64 {
        let s = t.max(0.0).floor();
        let r = (s - self.hold_s).max(0.0);
        let f = if self.ramp_s > 0.0 {
            (r / self.ramp_s).min(1.0)
        } else {
            1.0
        };
        let jitter = if self.noise > 0.0 {
            // A fixed pseudo-random sequence in −1 … 1.
            let mut x = (s as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03;
            x ^= x >> 29;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            x ^= x >> 32;
            (x % 2001) as f64 / 1000.0 - 1.0
        } else {
            0.0
        };
        (self.from + (self.to - self.from) * f + jitter * self.noise).round()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feeds `series(t)` at 1 Hz readings, sampled at 30 fps, for `secs`;
    /// returns the trend each second.
    fn run(secs: usize, series: impl Fn(f64) -> Option<f64>) -> Vec<Trend> {
        let mut tr = HrTrend::default();
        let mut out = Vec::new();
        for frame in 0..secs * 30 {
            let t = frame as f64 / 30.0;
            let v = tr.update(t, series(t.floor()));
            if frame % 30 == 29 {
                out.push(v);
            }
        }
        out
    }

    fn noisy(steady: f64, amp: f64) -> impl Fn(f64) -> Option<f64> {
        let p = HrProgram {
            from: steady,
            to: steady,
            ramp_s: 0.0,
            noise: amp,
            ..HrProgram::default()
        };
        move |t| Some(p.at(t))
    }

    #[test]
    fn a_steady_heart_rate_is_steady() {
        assert!(
            run(120, |_| Some(140.0))
                .iter()
                .all(|t| *t == Trend::Steady)
        );
    }

    #[test]
    fn a_rise_shows_within_a_few_seconds_and_ends_when_it_levels_off() {
        // 120 → 160 bpm over 40 s (+10 bpm per 10 s), then steady.
        let p = HrProgram {
            from: 120.0,
            to: 160.0,
            ramp_s: 40.0,
            noise: 0.0,
            ..HrProgram::default()
        };
        let tr = run(90, |t| Some(p.at(t)));
        let first = tr.iter().position(|t| *t == Trend::Rising).unwrap();
        assert!(first <= 8, "rising after {first} s");
        assert!(tr[first..40].iter().all(|t| *t == Trend::Rising), "{tr:?}");
        let off = tr.iter().rposition(|t| *t == Trend::Rising).unwrap();
        assert!(
            (40..62).contains(&off),
            "steady again after {off} s: {tr:?}"
        );
        assert!(tr[65..].iter().all(|t| *t == Trend::Steady));
    }

    #[test]
    fn a_peak_and_a_drop_end_the_rise_soon() {
        // 118 for 7 s, up to 166 by 14 s, steady to 20 s, down to 146 by
        // 25 s (the video's ride).
        let tr = run(32, |t| {
            Some(if t < 7.0 {
                118.0
            } else if t < 20.0 {
                (118.0 + 48.0 * ((t - 7.0) / 7.0).min(1.0)).round()
            } else {
                (166.0 - 20.0 * ((t - 20.0) / 5.0).min(1.0)).round()
            })
        });
        let on = tr.iter().position(|t| *t == Trend::Rising).unwrap();
        let off = tr.iter().rposition(|t| *t == Trend::Rising).unwrap();
        assert!((8..12).contains(&on), "{tr:?}");
        assert!((22..28).contains(&off), "{tr:?}");
        assert_eq!(tr[31], Trend::Falling, "{tr:?}");
    }

    #[test]
    fn a_slow_drift_is_not_a_rise() {
        // +1 bpm per 10 s: below the threshold.
        let tr = run(120, |t| Some((130.0 + t / 10.0).round()));
        assert!(tr.iter().all(|t| *t != Trend::Rising), "{tr:?}");
    }

    #[test]
    fn a_fall_is_falling_not_rising() {
        let p = HrProgram {
            from: 170.0,
            to: 130.0,
            ramp_s: 40.0,
            noise: 0.0,
            ..HrProgram::default()
        };
        let tr = run(60, |t| Some(p.at(t)));
        assert!(tr.contains(&Trend::Falling));
        assert!(!tr.contains(&Trend::Rising));
    }

    #[test]
    fn noise_does_not_flicker() {
        // ± 1.5 bpm jitter on a steady 142: never rising nor falling.
        for base in [95.0, 142.0, 171.0] {
            let tr = run(300, noisy(base, 1.5));
            assert!(tr.iter().all(|t| *t == Trend::Steady), "{base}: {tr:?}");
        }
        // A rise with the same jitter stays rising without flicker.
        let p = HrProgram {
            from: 120.0,
            to: 160.0,
            ramp_s: 40.0,
            noise: 1.5,
            ..HrProgram::default()
        };
        let tr = run(60, |t| Some(p.at(t)));
        let first = tr.iter().position(|t| *t == Trend::Rising).unwrap();
        let changes = tr.windows(2).filter(|w| w[0] != w[1]).count();
        assert!(first <= 9 && changes <= 2, "{tr:?}");
    }

    #[test]
    fn a_dropout_starts_over() {
        // Rising, then the strap drops out for 3 s, then a steady value:
        // no trend right after it comes back, steady later.
        let tr = run(60, |t| match t as usize {
            0..20 => Some(120.0 + t * 1.5),
            20..23 => None,
            _ => Some(150.0),
        });
        assert_eq!(tr[15], Trend::Rising);
        assert!(tr[20..].iter().all(|t| *t == Trend::Steady), "{tr:?}");
        // A gap in time (the app stalled) starts over too.
        let mut h = HrTrend::default();
        for i in 0..20 {
            h.update(i as f64, Some(120.0 + i as f64 * 2.0));
        }
        assert_eq!(h.trend(), Trend::Rising);
        assert_eq!(h.update(40.0, Some(160.0)), Trend::Steady);
        assert_eq!(h.rise(), None);
    }

    #[test]
    fn the_beat_runs_at_the_heart_rate() {
        // 120 bpm for 10.2 s at 60 fps: 20 beats.
        let mut b = HeartBeat::default();
        let beats = (0..612).filter(|_| b.advance(1.0 / 60.0, 120.0)).count();
        assert_eq!(beats, 20);
        // 60 bpm at 30 fps: 10 beats in 10.2 s.
        let mut b = HeartBeat::default();
        assert_eq!((0..306).filter(|_| b.advance(1.0 / 30.0, 60.0)).count(), 10);
    }

    #[test]
    fn the_phase_runs_on_when_the_bpm_changes() {
        let mut b = HeartBeat::default();
        for _ in 0..13 {
            b.advance(1.0 / 60.0, 90.0);
        }
        let before = b.phase;
        b.advance(1.0 / 60.0, 150.0);
        // One frame at the new rate: no jump back to 0, no skip.
        assert!((b.phase - (before + 150.0 / 3600.0)).abs() < 1e-9);
        // Long frames never skip a whole beat.
        let mut b = HeartBeat { phase: 0.5 };
        assert!(b.advance(5.0, 200.0));
        assert!((b.phase - 0.49).abs() < 1e-9);
        assert!(b.phase < 1.0);
    }

    #[test]
    fn the_pulse_is_short_and_peaks_early() {
        assert_eq!(pulse_at(0.0), 0.0);
        assert!((pulse_at(PULSE_WIDTH / 2.0) - 1.0).abs() < 1e-9);
        assert_eq!(pulse_at(0.5), 0.0);
        assert_eq!(pulse_at(0.9), 0.0);
    }

    #[test]
    fn soft_beat_steady_pink_pounding_when_rising() {
        let mut h = Heart::default();
        let mut max = 0.0f64;
        for f in 0..600 {
            let l = h.update(f as f64 / 30.0, Some(140.0), false).unwrap();
            if f > 300 {
                max = max.max(l.scale);
                assert_eq!(l.pink, 0.0);
                assert_eq!(l.glow, 0.0);
            }
        }
        assert!((max - SOFT_SCALE).abs() < 0.01, "{max}");
        // Rising: pink, a stronger beat, a glow.
        let mut max = 0.0f64;
        let mut look = HeartLook::default();
        for f in 600..1200 {
            let t = f as f64 / 30.0;
            look = h
                .update(t, Some((140.0 + (t - 20.0)).floor()), false)
                .unwrap();
            if f > 900 {
                max = max.max(look.scale);
            }
        }
        assert!(h.rising());
        assert_eq!(look.pink, 1.0);
        assert!(look.glow > 0.4);
        assert!((max - POUND_SCALE).abs() < 0.01, "{max}");
        // Reduced motion: the colour, no beat.
        let l = h.update(40.05, Some(161.0), true).unwrap();
        assert_eq!(l.scale, 1.0);
        assert_eq!(l.pink, 1.0);
        // No heart rate: no heart.
        assert_eq!(h.update(40.1, None, false), None);
        assert!(!h.rising());
    }

    #[test]
    fn the_test_program_ramps_then_holds() {
        let p = HrProgram {
            from: 110.0,
            to: 160.0,
            ramp_s: 25.0,
            noise: 0.0,
            ..HrProgram::default()
        };
        assert_eq!(p.at(0.0), 110.0);
        assert_eq!(p.at(10.7), 130.0);
        assert_eq!(p.at(25.0), 160.0);
        assert_eq!(p.at(90.0), 160.0);
        assert_eq!(HrProgram::steady(142.0).at(3.0), 142.0);
        let held = HrProgram { hold_s: 5.0, ..p };
        assert_eq!(held.at(4.9), 110.0);
        assert_eq!(held.at(15.0), 130.0);
        assert_eq!(held.at(40.0), 160.0);
        let n = HrProgram {
            noise: 2.0,
            ..HrProgram::steady(140.0)
        };
        assert!((0..100).all(|s| (n.at(s as f64) - 140.0).abs() <= 2.0));
        assert!((0..100).any(|s| n.at(s as f64) != 140.0));
    }
}
