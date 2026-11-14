//! How the browser plays the mixer (spec §14 Phase 10 B5): Web Audio has
//! no callback into wasm on the page's thread, so the app renders the mix
//! from its frame loop in chunks and schedules each chunk as an
//! `AudioBuffer` at an exact time on the context's clock
//! (`AudioBufferSourceNode.start(when)`: back to back, sample-exact). The
//! [`Scheduler`] decides which chunks are due: enough to stay [`lead`]
//! seconds ahead of the clock (the latency; long enough to bridge a slow
//! frame), never more than [`MAX_CHUNKS_PER_PUMP`] at once.
//!
//! When the clock overtook the schedule (a frame took longer than the
//! lead: shader compilation, a tab in the background) that is an
//! *underrun*: the schedule restarts [`START_MARGIN_S`] after the clock
//! (the gap is silence; nothing old is played late) and the lead grows by
//! [`LEAD_STEP_S`] up to [`LEAD_MAX_S`]. After [`CALM_S`] without an
//! underrun it shrinks again step by step to [`LEAD_START_S`].
//!
//! Pure (the clock is passed in) and tested natively.
//!
//! [`lead`]: Scheduler::lead_s

/// Frames per chunk: 1024 (21 ms at 48 kHz).
pub const CHUNK_FRAMES: usize = 1024;
/// The lead at the start (s).
pub const LEAD_START_S: f64 = 0.10;
/// The longest lead after underruns (s).
pub const LEAD_MAX_S: f64 = 0.30;
/// The lead grows (and later shrinks) by this much (s).
pub const LEAD_STEP_S: f64 = 0.04;
/// Seconds without an underrun before the lead shrinks one step.
pub const CALM_S: f64 = 30.0;
/// After a start or a gap, the first chunk begins this long after the
/// clock (s): time for the buffer to reach the audio thread.
pub const START_MARGIN_S: f64 = 0.03;
/// At most this many chunks per call (a long stall does not render seconds
/// of audio in one frame).
pub const MAX_CHUNKS_PER_PUMP: usize = 24;

/// The scheduler's counters (for `audio.status` and the page's
/// `window.ocw.audio()`).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ScheduleStats {
    /// Chunks scheduled.
    pub chunks: u64,
    /// The clock overtook the schedule.
    pub underruns: u32,
    /// Seconds of audio scheduled.
    pub scheduled_s: f64,
}

/// See the module docs.
#[derive(Debug, Clone)]
pub struct Scheduler {
    rate: f64,
    lead_s: f64,
    /// When the next chunk starts (the context's clock, s); `None` before
    /// the start or after a reset.
    next_at: Option<f64>,
    /// The last underrun (or the start), for shrinking the lead.
    calm_since: f64,
    stats: ScheduleStats,
}

impl Scheduler {
    /// A scheduler for a context running at `sample_rate`.
    pub fn new(sample_rate: f64) -> Self {
        Self {
            rate: sample_rate.max(8000.0),
            lead_s: LEAD_START_S,
            next_at: None,
            calm_since: 0.0,
            stats: ScheduleStats::default(),
        }
    }

    pub fn sample_rate(&self) -> f64 {
        self.rate
    }

    /// A chunk's length (s).
    pub fn chunk_s(&self) -> f64 {
        CHUNK_FRAMES as f64 / self.rate
    }

    /// How far ahead of the clock the schedule reaches now (s).
    pub fn lead_s(&self) -> f64 {
        self.lead_s
    }

    pub fn stats(&self) -> ScheduleStats {
        self.stats
    }

    /// Where the schedule ends (s on the context's clock), if running.
    pub fn scheduled_until(&self) -> Option<f64> {
        self.next_at
    }

    /// The context stopped (suspended, closed): the next call starts a new
    /// schedule.
    pub fn reset(&mut self) {
        self.next_at = None;
    }

    /// The start times of the chunks to render and schedule now, in order
    /// (`now_s`: the context's `currentTime`).
    pub fn due(&mut self, now_s: f64) -> Vec<f64> {
        let next = match self.next_at {
            None => {
                self.calm_since = now_s;
                now_s + START_MARGIN_S
            }
            Some(t) if t < now_s => {
                self.stats.underruns += 1;
                self.lead_s = (self.lead_s + LEAD_STEP_S).min(LEAD_MAX_S);
                self.calm_since = now_s;
                now_s + START_MARGIN_S
            }
            Some(t) => {
                if now_s - self.calm_since >= CALM_S && self.lead_s > LEAD_START_S {
                    self.lead_s = (self.lead_s - LEAD_STEP_S).max(LEAD_START_S);
                    self.calm_since = now_s;
                }
                t
            }
        };
        let chunk = self.chunk_s();
        let mut out = Vec::new();
        let mut at = next;
        while at < now_s + self.lead_s && out.len() < MAX_CHUNKS_PER_PUMP {
            out.push(at);
            at += chunk;
        }
        self.next_at = Some(at);
        self.stats.chunks += out.len() as u64;
        self.stats.scheduled_s += out.len() as f64 * chunk;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f64 = 48_000.0;

    #[test]
    fn keeps_the_lead_with_back_to_back_chunks() {
        let mut s = Scheduler::new(SR);
        let first = s.due(1.0);
        // From 1.03 s until 1.1 s: 4 chunks of 21.3 ms.
        assert_eq!(first.len(), 4, "{first:?}");
        assert!((first[0] - (1.0 + START_MARGIN_S)).abs() < 1e-12);
        for w in first.windows(2) {
            assert!((w[1] - w[0] - s.chunk_s()).abs() < 1e-12, "back to back");
        }
        // 60 frames per second: one or two chunks a frame, always ahead,
        // never a gap, no underrun.
        let mut end = s.scheduled_until().unwrap();
        for i in 1..=600 {
            let now = 1.0 + f64::from(i) / 60.0;
            let due = s.due(now);
            if let Some(&a) = due.first() {
                assert!((a - end).abs() < 1e-9, "no gap at {now}");
            }
            end = s.scheduled_until().unwrap();
            assert!(end >= now + s.lead_s() - 1e-9, "ahead at {now}");
            assert!(end <= now + s.lead_s() + s.chunk_s() + 1e-9);
        }
        assert_eq!(s.stats().underruns, 0);
        let total = s.stats().scheduled_s;
        assert!(
            (total - (end - 1.0 - START_MARGIN_S)).abs() < 1e-6,
            "{total}"
        );
    }

    /// A long frame: the clock overtakes the schedule; it restarts after
    /// the clock and the lead grows, then shrinks again when calm.
    #[test]
    fn an_underrun_restarts_and_raises_the_lead() {
        let mut s = Scheduler::new(SR);
        s.due(0.0);
        let due = s.due(0.5);
        assert_eq!(s.stats().underruns, 1);
        assert!((due[0] - 0.53).abs() < 1e-12, "{due:?}");
        assert!((s.lead_s() - (LEAD_START_S + LEAD_STEP_S)).abs() < 1e-12);
        for k in 0..10 {
            s.due(1.0 + f64::from(k));
        }
        assert!((s.lead_s() - LEAD_MAX_S).abs() < 1e-12, "capped");
        // Calm: one step down per CALM_S.
        let t0 = 20.0;
        s.due(t0);
        let mut t = t0;
        while t < t0 + CALM_S + 0.1 {
            t += 1.0 / 60.0;
            s.due(t);
        }
        assert!((s.lead_s() - (LEAD_MAX_S - LEAD_STEP_S)).abs() < 1e-9);
    }

    #[test]
    fn a_stall_never_renders_more_than_the_cap() {
        let mut s = Scheduler::new(8000.0);
        // A big lead at a low rate would be many chunks; capped anyway.
        s.lead_s = 100.0;
        assert_eq!(s.due(0.0).len(), MAX_CHUNKS_PER_PUMP);
        s.reset();
        assert_eq!(s.scheduled_until(), None);
        assert!((s.due(5.0)[0] - 5.03).abs() < 1e-12, "a fresh start");
        assert_eq!(s.stats().underruns, 0, "a reset is no underrun");
    }
}
