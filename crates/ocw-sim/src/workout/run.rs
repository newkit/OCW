//! Running a workout: workout time → step, target power, time left.
//!
//! **Clock (decided, spec §2.8):** the workout clock is the rider's
//! pedalling time, not wall-clock time, so stopping for a drink does not
//! eat into an interval:
//! - it waits at 0:00 until the rider first pedals (power > 0);
//! - it runs while the rider pedals, and for up to [`AUTO_PAUSE_AFTER_S`]
//!   of zero power (shifting, a short coast);
//! - after that it pauses, and resumes as soon as power returns;
//! - `pause` / `resume` stop and restart it by hand;
//! - the ride itself never pauses (spec §3.2), only the workout clock.
//!
//! The ERG target stays set while the clock waits or pauses, so the
//! trainer holds it when the rider starts again.

use std::sync::Arc;

use super::{Workout, format_duration};

/// Zero power this long pauses the workout clock (s).
pub const AUTO_PAUSE_AFTER_S: f64 = 3.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    /// Started, waiting for the rider to pedal.
    Waiting,
    Running,
    /// Not pedalling for [`AUTO_PAUSE_AFTER_S`]; resumes with power.
    Paused,
    /// Paused by hand (`workout pause`); resumes with `workout resume`.
    PausedByRider,
    Finished,
}

/// What happened during [`WorkoutRun::advance`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// A new step started (index).
    Step(usize),
    Finished,
}

/// A workout being ridden.
#[derive(Debug, Clone)]
pub struct WorkoutRun {
    workout: Arc<Workout>,
    elapsed_s: f64,
    idle_s: f64,
    state: RunState,
}

/// Where the rider is in the workout, for the HUD and `workout status`.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkoutStatus {
    pub state: RunState,
    pub elapsed_s: f64,
    pub total_s: f64,
    /// Current step (0-based) and the number of steps.
    pub index: usize,
    pub count: usize,
    pub step_left_s: f64,
    /// ERG target now (W); `None` in a free ride step or after the end.
    pub target_w: Option<f64>,
    pub cadence_rpm: Option<f64>,
    /// The step's target: "250 W", "150 → 200 W", "free ride".
    pub step: String,
    /// Interval position, e.g. "2/3 on".
    pub label: Option<String>,
    /// The cue showing now, else the step's text.
    pub text: Option<String>,
    /// "250 W for 5:00", or `None` in the last step.
    pub next: Option<String>,
}

impl WorkoutRun {
    pub fn new(workout: Arc<Workout>) -> Self {
        Self {
            workout,
            elapsed_s: 0.0,
            idle_s: 0.0,
            state: RunState::Waiting,
        }
    }

    pub fn workout(&self) -> &Arc<Workout> {
        &self.workout
    }

    pub fn elapsed_s(&self) -> f64 {
        self.elapsed_s
    }

    pub fn state(&self) -> RunState {
        self.state
    }

    pub fn is_finished(&self) -> bool {
        self.state == RunState::Finished
    }

    fn index(&self) -> Option<usize> {
        self.workout.segment_at(self.elapsed_s).map(|(i, _)| i)
    }

    /// Advances by `dt_s` of ride time with the rider's current power.
    pub fn advance(&mut self, dt_s: f64, power_w: f64) -> Option<Change> {
        let dt = if dt_s.is_finite() { dt_s.max(0.0) } else { 0.0 };
        let pedalling = power_w > 0.0;
        match self.state {
            RunState::Finished | RunState::PausedByRider => return None,
            RunState::Waiting | RunState::Paused if !pedalling => return None,
            RunState::Waiting | RunState::Paused => {
                self.state = RunState::Running;
                self.idle_s = 0.0;
            }
            RunState::Running if pedalling => self.idle_s = 0.0,
            RunState::Running => {
                self.idle_s += dt;
                if self.idle_s > AUTO_PAUSE_AFTER_S {
                    self.state = RunState::Paused;
                    return None;
                }
            }
        }
        let before = self.index();
        self.elapsed_s += dt;
        self.changed(before)
    }

    fn changed(&mut self, before: Option<usize>) -> Option<Change> {
        if self.elapsed_s >= self.workout.duration_s() {
            self.elapsed_s = self.workout.duration_s();
            self.state = RunState::Finished;
            return Some(Change::Finished);
        }
        let now = self.index();
        if now != before {
            now.map(Change::Step)
        } else {
            None
        }
    }

    /// Skips to the start of the next step (or finishes).
    pub fn skip(&mut self) -> Option<Change> {
        let i = self.index()?;
        let before = Some(i);
        self.elapsed_s = self.workout.start_of(i + 1);
        if self.state == RunState::Waiting {
            self.state = RunState::Running;
        }
        self.changed(before)
    }

    /// Pauses by hand; returns false when already finished.
    pub fn pause(&mut self) -> bool {
        if self.is_finished() {
            return false;
        }
        self.state = RunState::PausedByRider;
        true
    }

    /// Resumes after [`WorkoutRun::pause`]: running again (it pauses again
    /// by itself without pedalling).
    pub fn resume(&mut self) -> bool {
        if self.state != RunState::PausedByRider {
            return false;
        }
        self.state = RunState::Running;
        self.idle_s = 0.0;
        true
    }

    /// The ERG target now (W); `None` in free ride steps and after the end.
    pub fn target_w(&self, ftp_w: f64) -> Option<f64> {
        if self.is_finished() {
            return None;
        }
        self.workout.target_w_at(self.elapsed_s, ftp_w)
    }

    pub fn status(&self, ftp_w: f64) -> WorkoutStatus {
        let w = &self.workout;
        let count = w.segments().len();
        let (index, offset) = w.segment_at(self.elapsed_s).unwrap_or((count - 1, 0.0));
        let seg = &w.segments()[index];
        let finished = self.is_finished();
        let next = w.segments().get(index + 1).filter(|_| !finished).map(|n| {
            format!(
                "{} for {}",
                n.describe(ftp_w),
                format_duration(n.duration_s)
            )
        });
        WorkoutStatus {
            state: self.state,
            elapsed_s: self.elapsed_s,
            total_s: w.duration_s(),
            index,
            count,
            step_left_s: if finished {
                0.0
            } else {
                seg.duration_s - offset
            },
            target_w: self.target_w(ftp_w),
            cadence_rpm: seg.cadence_rpm,
            step: seg.describe(ftp_w),
            label: seg.label.clone(),
            text: w
                .cue_at(self.elapsed_s)
                .map(|c| c.text.clone())
                .or_else(|| seg.text.clone()),
            next,
        }
    }
}

impl WorkoutStatus {
    /// Multi-line text for `workout status`.
    pub fn describe(&self, name: &str) -> String {
        let state = match self.state {
            RunState::Waiting => " (start pedalling to begin)",
            RunState::Running => "",
            RunState::Paused => " (paused: not pedalling)",
            RunState::PausedByRider => " (paused; workout resume)",
            RunState::Finished => " (finished)",
        };
        let mut s = format!(
            "{name}: {} / {}{state}\nstep {}/{}{}: {}, {} left",
            format_duration(self.elapsed_s),
            format_duration(self.total_s),
            self.index + 1,
            self.count,
            self.label
                .as_ref()
                .map(|l| format!(" ({l})"))
                .unwrap_or_default(),
            self.step,
            format_duration(self.step_left_s),
        );
        if let Some(t) = self.target_w {
            s.push_str(&format!("\ntarget {t:.0} W"));
        }
        if let Some(c) = self.cadence_rpm {
            s.push_str(&format!(" at {c:.0} rpm"));
        }
        if let Some(n) = &self.next {
            s.push_str(&format!("\nnext: {n}"));
        }
        if let Some(t) = &self.text {
            s.push_str(&format!("\n\"{t}\""));
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workout::{Cue, Power, Segment, Target};

    /// 10 s ramp 100 → 200 W, 2 × (5 s at 300 W, 5 s free), 10 s at 50 % FTP.
    fn workout() -> Arc<Workout> {
        let mut on = Segment::new(5.0, Target::steady(Power::Watts(300.0)));
        on.cadence_rpm = Some(100.0);
        let off = Segment::new(5.0, Target::Free);
        let mut segs = vec![Segment::new(
            10.0,
            Target::Power {
                from: Power::Watts(100.0),
                to: Power::Watts(200.0),
            },
        )];
        for k in 1..=2 {
            segs.push(Segment {
                label: Some(format!("{k}/2 on")),
                ..on.clone()
            });
            segs.push(Segment {
                label: Some(format!("{k}/2 off")),
                text: Some("easy".into()),
                ..off.clone()
            });
        }
        segs.push(Segment::new(10.0, Target::steady(Power::Ftp(0.5))));
        Arc::new(
            Workout::new(
                "w",
                "",
                segs,
                vec![Cue {
                    at_s: 12.0,
                    duration_s: 2.0,
                    text: "push".into(),
                }],
            )
            .unwrap(),
        )
    }

    /// Advances in 1/8 s ticks (exact in binary).
    fn ride(r: &mut WorkoutRun, seconds: f64, power: f64) -> Vec<Change> {
        let mut changes = Vec::new();
        for _ in 0..(seconds * 8.0).round() as usize {
            changes.extend(r.advance(0.125, power));
        }
        changes
    }

    #[test]
    fn waits_for_pedalling_then_follows_the_steps() {
        let mut r = WorkoutRun::new(workout());
        assert_eq!(r.state(), RunState::Waiting);
        // The first target is set before the rider pedals.
        assert_eq!(r.target_w(250.0), Some(100.0));
        assert!(ride(&mut r, 5.0, 0.0).is_empty());
        assert_eq!(r.elapsed_s(), 0.0);
        ride(&mut r, 5.0, 150.0);
        assert_eq!(r.state(), RunState::Running);
        assert!((r.target_w(250.0).unwrap() - 150.0).abs() < 1e-6);
        let changes = ride(&mut r, 5.0, 150.0);
        assert_eq!(changes, vec![Change::Step(1)]);
        let s = r.status(250.0);
        assert_eq!((s.index, s.count), (1, 6));
        assert_eq!(s.target_w, Some(300.0));
        assert_eq!(s.cadence_rpm, Some(100.0));
        assert_eq!(s.label.as_deref(), Some("1/2 on"));
        assert!((s.step_left_s - 5.0).abs() < 0.05, "{}", s.step_left_s);
        assert_eq!(s.next.as_deref(), Some("free ride for 0:05"));
        // The cue at 12 s replaces the step text for 2 s.
        ride(&mut r, 2.0, 150.0);
        assert_eq!(r.status(250.0).text.as_deref(), Some("push"));
        // Free step: no ERG target, its text shows.
        ride(&mut r, 3.5, 150.0);
        let s = r.status(250.0);
        assert_eq!(s.target_w, None);
        assert_eq!(s.step, "free ride");
        assert_eq!(s.text.as_deref(), Some("easy"));
        // The FTP-relative last step.
        ride(&mut r, 15.0, 150.0);
        assert_eq!(r.target_w(250.0), Some(125.0));
        assert_eq!(r.status(250.0).next, None);
        let changes = ride(&mut r, 11.0, 150.0);
        assert_eq!(changes.last(), Some(&Change::Finished));
        assert!(r.is_finished());
        assert_eq!(r.elapsed_s(), 40.0);
        assert_eq!(r.target_w(250.0), None);
        let s = r.status(250.0);
        assert_eq!((s.index, s.step_left_s), (5, 0.0));
        assert!(s.describe("w").contains("(finished)"));
    }

    #[test]
    fn auto_pause_after_three_seconds_without_power() {
        let mut r = WorkoutRun::new(workout());
        ride(&mut r, 2.0, 200.0);
        // A short coast counts.
        ride(&mut r, 2.0, 0.0);
        assert!((r.elapsed_s() - 4.0).abs() < 1e-6);
        assert_eq!(r.state(), RunState::Running);
        // A long stop pauses after 3 s without power in all.
        ride(&mut r, 10.0, 0.0);
        assert_eq!(r.state(), RunState::Paused);
        assert!((r.elapsed_s() - 5.0).abs() < 1e-6, "{}", r.elapsed_s());
        assert!(r.status(200.0).describe("w").contains("not pedalling"));
        // The target stays while paused.
        assert!(r.target_w(200.0).is_some());
        ride(&mut r, 1.0, 120.0);
        assert_eq!(r.state(), RunState::Running);
        assert!((r.elapsed_s() - 6.0).abs() < 1e-6);
    }

    #[test]
    fn manual_pause_resume_and_skip() {
        let mut r = WorkoutRun::new(workout());
        // Skipping from the start starts the clock in step 2.
        assert_eq!(r.skip(), Some(Change::Step(1)));
        assert_eq!(r.elapsed_s(), 10.0);
        assert!(r.pause());
        ride(&mut r, 5.0, 300.0);
        assert_eq!(r.elapsed_s(), 10.0);
        assert!(r.status(200.0).describe("w").contains("workout resume"));
        assert!(r.resume());
        assert!(!r.resume());
        ride(&mut r, 1.0, 300.0);
        assert!((r.elapsed_s() - 11.0).abs() < 1e-6);
        for i in 2..=5 {
            assert_eq!(r.skip(), Some(Change::Step(i)));
        }
        assert_eq!(r.skip(), Some(Change::Finished));
        assert_eq!(r.skip(), None);
        assert!(!r.pause());
        // Bad time steps do not move the clock.
        let mut r = WorkoutRun::new(workout());
        r.advance(f64::NAN, 100.0);
        r.advance(-1.0, 100.0);
        assert_eq!(r.elapsed_s(), 0.0);
    }

    #[test]
    fn status_text() {
        let mut r = WorkoutRun::new(workout());
        ride(&mut r, 11.0, 200.0);
        let text = r.status(200.0).describe("Demo");
        assert_eq!(
            text,
            "Demo: 0:11 / 0:40\nstep 2/6 (1/2 on): 300 W, 0:04 left\ntarget 300 W at 100 rpm\nnext: free ride for 0:05"
        );
    }
}
