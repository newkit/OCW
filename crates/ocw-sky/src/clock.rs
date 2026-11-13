//! The session's world clock (spec §14 Phase 9, 9A / 9D): owned by the
//! game server (§6.1), sent in `Welcome` and the snapshots, predicted
//! smoothly by the clients ([`ClockPredictor`]).
//!
//! The clock runs on **session time** (the server's simulation ticks), not
//! the wall clock: a frame capture (`--capture`, a fixed game-time step per
//! frame) or a `--fast` server keep sun and simulation together. The real
//! UTC instant is read once when the session starts ([`WorldClock::new`]);
//! at real time speed session time and the wall clock run alike.
//!
//! The world instant is `anchor + (t - anchor_t) · rate` (`rate` = speed,
//! 0 while paused), clamped to 1900–2100. The modes say how the anchor was
//! set:
//! - *Real*: the real instant; speed ×1, not paused.
//! - *Offset*: the real instant plus a fixed offset; speed ×1, not paused.
//! - *Fixed*: a chosen instant, then running at the speed (or paused).
//!
//! Changing the speed or pausing in *Real* / *Offset* turns the clock into
//! *Fixed* from the current world instant (the time-lapse starts where the
//! world is).

use crate::time::{MS_PER_HOUR, UnixMs, clamp_range, in_range};

/// How the world instant was chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockMode {
    Real,
    Fixed,
    Offset,
}

impl ClockMode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Real => "real",
            Self::Fixed => "fixed",
            Self::Offset => "offset",
        }
    }
}

/// The time speeds the time machine offers (world seconds per second).
pub const SPEEDS: [f64; 4] = [1.0, 10.0, 60.0, 600.0];
/// Fastest accepted speed (×3600: an hour per second).
pub const MAX_SPEED: f64 = 3600.0;

/// "x1", "x600", "paused".
pub fn speed_label(speed: f64, paused: bool) -> String {
    if paused {
        "paused".into()
    } else if speed.fract() == 0.0 {
        format!("x{speed:.0}")
    } else {
        format!("x{speed}")
    }
}

/// The session's world clock.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldClock {
    mode: ClockMode,
    speed: f64,
    paused: bool,
    /// Session time (s) and world instant of the anchor.
    anchor_t: f64,
    anchor_ms: UnixMs,
    /// The real UTC instant at session time 0.
    real_epoch_ms: UnixMs,
}

impl WorldClock {
    /// A real-time clock; `real_epoch_ms` is the real UTC instant at
    /// session time 0.
    pub fn new(real_epoch_ms: UnixMs) -> Self {
        Self {
            mode: ClockMode::Real,
            speed: 1.0,
            paused: false,
            anchor_t: 0.0,
            anchor_ms: real_epoch_ms,
            real_epoch_ms,
        }
    }

    pub fn mode(&self) -> ClockMode {
        self.mode
    }

    pub fn speed(&self) -> f64 {
        self.speed
    }

    pub fn paused(&self) -> bool {
        self.paused
    }

    /// World seconds per session second (0 while paused).
    pub fn rate(&self) -> f64 {
        if self.paused { 0.0 } else { self.speed }
    }

    /// The real UTC instant at session time `t`.
    pub fn real(&self, t: f64) -> UnixMs {
        self.real_epoch_ms + (t * 1000.0).round() as UnixMs
    }

    /// The world instant at session time `t`.
    pub fn now(&self, t: f64) -> UnixMs {
        let run = ((t - self.anchor_t) * self.rate() * 1000.0).round();
        let run = run.clamp(-1e15, 1e15) as UnixMs;
        clamp_range(self.anchor_ms.saturating_add(run))
    }

    /// World minus real instant at `t`.
    pub fn offset_ms(&self, t: f64) -> i64 {
        self.now(t) - self.real(t)
    }

    fn anchor(&mut self, t: f64, ms: UnixMs) {
        self.anchor_t = t;
        self.anchor_ms = clamp_range(ms);
    }

    /// Back to the real instant (×1, running).
    pub fn set_real(&mut self, t: f64) {
        self.mode = ClockMode::Real;
        self.speed = 1.0;
        self.paused = false;
        let real = self.real(t);
        self.anchor(t, real);
    }

    /// The real instant plus `offset_ms` (×1, running).
    pub fn set_offset(&mut self, t: f64, offset_ms: i64) -> Result<(), String> {
        let at = self.real(t).saturating_add(offset_ms);
        check(at)?;
        self.mode = if offset_ms == 0 {
            ClockMode::Real
        } else {
            ClockMode::Offset
        };
        self.speed = 1.0;
        self.paused = false;
        self.anchor(t, at);
        Ok(())
    }

    /// A chosen instant; the speed and pause state stay.
    pub fn set_fixed(&mut self, t: f64, ms: UnixMs) -> Result<(), String> {
        check(ms)?;
        self.mode = ClockMode::Fixed;
        self.anchor(t, ms);
        Ok(())
    }

    /// Moves the world instant by `delta_ms`, keeping the mode (*Real*
    /// becomes *Offset*).
    pub fn step(&mut self, t: f64, delta_ms: i64) -> Result<(), String> {
        let at = self.now(t).saturating_add(delta_ms);
        check(at)?;
        match self.mode {
            ClockMode::Real | ClockMode::Offset => {
                let offset = at - self.real(t);
                self.set_offset(t, offset)
            }
            ClockMode::Fixed => self.set_fixed(t, at),
        }
    }

    /// The time speed (world seconds per second, 0 < speed ≤ 3600); not
    /// ×1 outside *Fixed* makes the clock *Fixed* at the current instant.
    pub fn set_speed(&mut self, t: f64, speed: f64) -> Result<(), String> {
        if !(speed > 0.0 && speed <= MAX_SPEED) {
            return Err(format!(
                "time speed {speed} out of range (above 0, at most x{MAX_SPEED:.0})"
            ));
        }
        let now = self.now(t);
        if self.mode != ClockMode::Fixed && speed != 1.0 {
            self.mode = ClockMode::Fixed;
        }
        self.speed = speed;
        self.anchor(t, now);
        Ok(())
    }

    /// Pauses or resumes; pausing outside *Fixed* makes the clock *Fixed*.
    pub fn set_paused(&mut self, t: f64, paused: bool) {
        let now = self.now(t);
        if paused && self.mode != ClockMode::Fixed {
            self.mode = ClockMode::Fixed;
        }
        self.paused = paused;
        self.anchor(t, now);
    }

    /// One-line state: "fixed 2026-06-21 ... x600" is the caller's job;
    /// this gives "fixed, x600" / "real, x1" / "offset +2.0 h, x1".
    pub fn describe(&self, t: f64) -> String {
        let speed = speed_label(self.speed, self.paused);
        match self.mode {
            ClockMode::Offset => format!(
                "offset {:+.1} h, {speed}",
                self.offset_ms(t) as f64 / MS_PER_HOUR as f64
            ),
            m => format!("{}, {speed}", m.name()),
        }
    }
}

fn check(ms: UnixMs) -> Result<(), String> {
    if in_range(ms) {
        Ok(())
    } else {
        Err("the time machine goes from 1900 to 2100".into())
    }
}

/// The world clock on a client: the server's clock from the snapshots,
/// advanced smoothly between them (spec §6.1: only the server's clock
/// counts). Small differences are blended in over about a second and the
/// time never runs backwards while the clock runs; a jump (time machine,
/// first snapshot) is taken at once.
#[derive(Debug, Clone, Copy, Default)]
pub struct ClockPredictor {
    /// The predicted world instant at client time `base_t` and its rate.
    state: Option<(f64, f64, f64)>,
    /// The last instant handed out (monotonic while running).
    last: Option<(f64, f64)>,
}

/// Differences above this many world seconds per unit of rate (at least
/// 2 s) snap instead of blending.
const SNAP_S_PER_RATE: f64 = 2.0;
/// Fraction of the difference corrected per snapshot (10 Hz: ~1 s).
const BLEND: f64 = 0.2;

impl ClockPredictor {
    /// A server clock message: world instant `world_ms` and `rate`,
    /// received at client time `now_s`, `age_s` old (half the round trip).
    pub fn on_server(&mut self, world_ms: UnixMs, rate: f64, now_s: f64, age_s: f64) {
        let server = world_ms as f64 + age_s * rate * 1000.0;
        let snap = |s: &mut Self| {
            s.state = Some((server, now_s, rate));
            s.last = None;
        };
        match self.state {
            Some((ms, t, r)) if r == rate => {
                let predicted = ms + (now_s - t) * r * 1000.0;
                let error = server - predicted;
                if error.abs() > SNAP_S_PER_RATE.max(SNAP_S_PER_RATE * rate) * 1000.0 {
                    snap(self);
                } else {
                    self.state = Some((predicted + error * BLEND, now_s, rate));
                }
            }
            _ => snap(self),
        }
    }

    /// Whether a server clock arrived yet.
    pub fn known(&self) -> bool {
        self.state.is_some()
    }

    /// The world instant at client time `now_s`.
    pub fn now(&mut self, now_s: f64) -> Option<UnixMs> {
        let (ms, t, r) = self.state?;
        let mut v = ms + (now_s - t) * r * 1000.0;
        if let Some((last_v, last_t)) = self.last
            && r > 0.0
            && now_s >= last_t
        {
            v = v.max(last_v);
        }
        self.last = Some((v, now_s));
        Some(clamp_range(v.round() as UnixMs))
    }

    /// The rate of the last server clock.
    pub fn rate(&self) -> f64 {
        self.state.map_or(1.0, |s| s.2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::{MAX_MS, MIN_MS, MS_PER_DAY, utc_midnight};

    const EPOCH: UnixMs = 1_781_000_000_000; // 2026-06-09

    #[test]
    fn real_mode_follows_session_time() {
        let c = WorldClock::new(EPOCH);
        assert_eq!(c.mode(), ClockMode::Real);
        assert_eq!(c.now(0.0), EPOCH);
        assert_eq!(c.now(90.5), EPOCH + 90_500);
        assert_eq!(c.offset_ms(1234.0), 0);
        assert_eq!(c.describe(0.0), "real, x1");
    }

    #[test]
    fn speed_and_pause() {
        let mut c = WorldClock::new(EPOCH);
        c.set_speed(10.0, 600.0).unwrap();
        assert_eq!(c.mode(), ClockMode::Fixed, "a time-lapse starts from now");
        assert_eq!(c.now(10.0), EPOCH + 10_000);
        // One minute of riding at x600 is ten hours.
        assert_eq!(c.now(70.0), EPOCH + 10_000 + 10 * MS_PER_HOUR);
        c.set_paused(70.0, true);
        assert_eq!(c.rate(), 0.0);
        assert_eq!(c.now(1000.0), EPOCH + 10_000 + 10 * MS_PER_HOUR);
        c.set_paused(1000.0, false);
        assert_eq!(c.now(1001.0), EPOCH + 10_000 + 10 * MS_PER_HOUR + 600_000);
        assert_eq!(c.describe(1001.0), "fixed, x600");
        assert!(c.set_speed(0.0, 0.0).is_err());
        assert!(c.set_speed(0.0, 5000.0).is_err());
        // Pausing real time freezes the current instant.
        let mut r = WorldClock::new(EPOCH);
        r.set_paused(5.0, true);
        assert_eq!((r.mode(), r.now(50.0)), (ClockMode::Fixed, EPOCH + 5000));
        assert_eq!(r.describe(50.0), "fixed, paused");
    }

    #[test]
    fn offset_and_steps() {
        let mut c = WorldClock::new(EPOCH);
        c.set_offset(0.0, 2 * MS_PER_HOUR).unwrap();
        assert_eq!(c.mode(), ClockMode::Offset);
        assert_eq!(c.now(60.0), EPOCH + 60_000 + 2 * MS_PER_HOUR);
        assert_eq!(c.describe(60.0), "offset +2.0 h, x1");
        // A step keeps the mode: real becomes an offset, fixed stays fixed.
        let mut r = WorldClock::new(EPOCH);
        r.step(0.0, -MS_PER_HOUR).unwrap();
        assert_eq!(
            (r.mode(), r.offset_ms(100.0)),
            (ClockMode::Offset, -MS_PER_HOUR)
        );
        r.step(100.0, MS_PER_HOUR).unwrap();
        assert_eq!(r.mode(), ClockMode::Real, "offset 0 is real time again");
        let mut f = WorldClock::new(EPOCH);
        f.set_fixed(0.0, utc_midnight(2026, 6, 21)).unwrap();
        f.set_speed(0.0, 60.0).unwrap();
        f.step(1.0, MS_PER_DAY).unwrap();
        assert_eq!(f.mode(), ClockMode::Fixed);
        assert_eq!(f.speed(), 60.0, "a fixed clock keeps its speed");
        assert_eq!(f.now(1.0), utc_midnight(2026, 6, 22) + 60_000);
        f.set_real(2.0);
        assert_eq!(
            (f.mode(), f.speed(), f.now(2.0)),
            (ClockMode::Real, 1.0, EPOCH + 2000)
        );
    }

    #[test]
    fn the_range_is_1900_to_2100() {
        let mut c = WorldClock::new(EPOCH);
        assert!(c.set_fixed(0.0, MIN_MS - 1).is_err());
        assert!(c.set_fixed(0.0, MAX_MS + 1).is_err());
        assert!(c.step(0.0, 200 * 365 * MS_PER_DAY).is_err());
        assert_eq!(c.mode(), ClockMode::Real, "refused changes change nothing");
        // A time-lapse stops at the end of 2100.
        c.set_fixed(0.0, MAX_MS - MS_PER_HOUR).unwrap();
        c.set_speed(0.0, 3600.0).unwrap();
        assert_eq!(c.now(10.0), MAX_MS);
    }

    #[test]
    fn prediction_is_smooth_and_follows_the_server() {
        let mut p = ClockPredictor::default();
        assert_eq!(p.now(0.0), None);
        p.on_server(EPOCH, 1.0, 10.0, 0.0);
        assert_eq!(p.now(10.0), Some(EPOCH));
        assert_eq!(p.now(10.5), Some(EPOCH + 500));
        // A snapshot 40 ms late in arriving (jitter): blended, never back.
        let mut prev = p.now(10.6).unwrap();
        p.on_server(EPOCH + 600, 1.0, 10.64, 0.0);
        for i in 0..40 {
            let v = p.now(10.64 + f64::from(i) * 0.01).unwrap();
            assert!(v >= prev, "time runs backwards at step {i}");
            prev = v;
        }
        // Converges on the server's clock within a few snapshots.
        for k in 1..=20 {
            let t = 11.0 + f64::from(k) * 0.1;
            p.on_server(EPOCH + ((t - 10.0) * 1000.0) as i64 - 40, 1.0, t, 0.0);
        }
        let err = p.now(13.0).unwrap() - (EPOCH + 3000 - 40);
        assert!(err.abs() < 5, "{err}");
        // A jump of the time machine (or a new rate) is taken at once.
        p.on_server(EPOCH + MS_PER_DAY, 600.0, 13.1, 0.0);
        assert_eq!(p.now(13.1), Some(EPOCH + MS_PER_DAY));
        assert_eq!(p.now(14.1), Some(EPOCH + MS_PER_DAY + 600_000));
        assert_eq!(p.rate(), 600.0);
        // The snapshot's age is added (half the round trip).
        let mut q = ClockPredictor::default();
        q.on_server(EPOCH, 600.0, 0.0, 0.05);
        assert_eq!(q.now(0.0), Some(EPOCH + 30_000));
        // Paused: stands still.
        q.on_server(EPOCH, 0.0, 1.0, 0.0);
        assert_eq!(q.now(5.0), Some(EPOCH));
    }

    /// Round trip with the server clock: the predicted client clock stays
    /// within a frame of the server's while both run at x600.
    #[test]
    fn prediction_tracks_a_running_server_clock() {
        let mut server = WorldClock::new(EPOCH);
        server.set_speed(0.0, 600.0).unwrap();
        let mut p = ClockPredictor::default();
        // Snapshots every 0.1 s, the client renders at 60 Hz.
        let mut next_snapshot = 0.0;
        let mut prev = None;
        for frame in 0..600 {
            let t = f64::from(frame) / 60.0;
            while next_snapshot <= t {
                p.on_server(server.now(next_snapshot), server.rate(), next_snapshot, 0.0);
                next_snapshot += 0.1;
            }
            let v = p.now(t).unwrap();
            assert!((v - server.now(t)).abs() <= 10, "frame {frame}");
            if let Some(prev) = prev {
                assert!(v > prev);
            }
            prev = Some(v);
        }
    }

    #[test]
    fn speed_labels() {
        assert_eq!(speed_label(600.0, false), "x600");
        assert_eq!(speed_label(1.0, true), "paused");
        assert_eq!(speed_label(2.5, false), "x2.5");
    }
}
