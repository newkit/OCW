//! Structured workouts in the client (spec §2.8, §5.3), shared by the
//! desktop and the browser app (spec §14 Phase 10 B4): load, start, stop,
//! skip; the workout's target power drives the trainer in ERG mode.
//!
//! Workouts are client-side target control: the server only sees the
//! rider's power, as in any ride (spec §6.1), so nothing here touches the
//! protocol. While a workout runs it owns the trainer mode: ERG with the
//! step's target, the mode from before the workout (normally SIM) in free
//! ride steps, and that mode again when it ends or is stopped. The apps
//! add the files, menus and console commands.

use std::sync::Arc;

use ocw_sim::workout::{Change, Workout, WorkoutPanel, WorkoutRun, format_duration};

use crate::trainer::{TrainerControl, TrainerMode};

/// FTP when neither `--ftp` nor the rider profile gives one (W).
pub const DEFAULT_FTP_W: f64 = 200.0;

/// The example workouts built into the app (`workout start --example`).
pub const EXAMPLES: &[(&str, &str)] = &[
    (
        "short-demo.toml",
        include_str!("../../ocw-sim/test-data/workouts/short-demo.toml"),
    ),
    (
        "sweet-spot.toml",
        include_str!("../../ocw-sim/test-data/workouts/sweet-spot.toml"),
    ),
    (
        "two-by-five.erg",
        include_str!("../../ocw-sim/test-data/workouts/two-by-five.erg"),
    ),
    (
        "over-unders.mrc",
        include_str!("../../ocw-sim/test-data/workouts/over-unders.mrc"),
    ),
];

/// An example's name: its file name without the extension.
pub fn example_name(file: &str) -> &str {
    file.rsplit_once('.').map_or(file, |(s, _)| s)
}

/// A bundled example by its name with or without extension
/// (`short-demo`, `two-by-five.erg`).
pub fn example(name: &str) -> Result<Workout, String> {
    let (file, text) = EXAMPLES
        .iter()
        .find(|(f, _)| *f == name || example_name(f) == name)
        .ok_or_else(|| {
            let names: Vec<&str> = EXAMPLES.iter().map(|(f, _)| example_name(f)).collect();
            format!("no example {name} (examples: {})", names.join(", "))
        })?;
    Workout::parse_file(file, text).map_err(|e| e.to_string())
}

/// How long the panel stays after the workout is done (s).
pub const DONE_PANEL_S: f64 = 20.0;

/// Whether the workout controls the trainer.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Control {
    Off,
    /// Started; takes over the trainer at the next update.
    Starting,
    /// Controlling; `resume` is the mode to return to.
    Active {
        resume: TrainerMode,
    },
}

/// The client's workout state, without Bevy.
pub struct WorkoutSession {
    loaded: Option<Arc<Workout>>,
    run: Option<WorkoutRun>,
    control: Control,
    done_for_s: f64,
    /// Counts the starts, step changes, ends and stops: each one begins a
    /// new lap in the ride's FIT file (spec §2.8).
    laps: u64,
}

impl Default for WorkoutSession {
    fn default() -> Self {
        Self {
            loaded: None,
            run: None,
            control: Control::Off,
            done_for_s: 0.0,
            laps: 0,
        }
    }
}

fn summary(w: &Workout) -> String {
    format!(
        "{}: {} steps, {}",
        w.name,
        w.segments().len(),
        format_duration(w.duration_s())
    )
}

impl WorkoutSession {
    pub fn loaded(&self) -> Option<&Arc<Workout>> {
        self.loaded.as_ref()
    }

    pub fn run(&self) -> Option<&WorkoutRun> {
        self.run.as_ref()
    }

    /// Grows by one at each workout start, step change, end and stop: the
    /// recorder starts a FIT lap when it changes.
    pub fn lap_count(&self) -> u64 {
        self.laps
    }

    fn running(&self) -> bool {
        self.run.as_ref().is_some_and(|r| !r.is_finished())
    }

    /// Loads a workout (to start with [`WorkoutSession::start`]); not while
    /// one runs.
    pub fn load(&mut self, workout: Workout) -> Result<String, String> {
        if self.running() {
            return Err("a workout is running (workout stop first)".into());
        }
        let text = format!("loaded {} (workout start)", summary(&workout));
        self.loaded = Some(Arc::new(workout));
        Ok(text)
    }

    /// Starts (or restarts) the loaded workout from the beginning.
    pub fn start(&mut self) -> Result<String, String> {
        let w = self
            .loaded
            .clone()
            .ok_or("no workout loaded (workout load <file>)")?;
        let text = format!("started {}; the clock starts when you pedal", summary(&w));
        self.run = Some(WorkoutRun::new(w));
        self.done_for_s = 0.0;
        self.laps += 1;
        // A restart keeps the mode to return to.
        if !matches!(self.control, Control::Active { .. }) {
            self.control = Control::Starting;
        }
        Ok(text)
    }

    /// Stops the workout and gives the trainer back its previous mode.
    pub fn stop(&mut self, trainer: &mut TrainerControl) -> Result<String, String> {
        let run = self.run.take().ok_or("no workout running")?;
        self.release(trainer);
        if !run.is_finished() {
            self.laps += 1;
        }
        Ok(format!(
            "stopped {} at {}; trainer {}",
            run.workout().name,
            format_duration(run.elapsed_s()),
            trainer.mode()
        ))
    }

    fn release(&mut self, trainer: &mut TrainerControl) {
        if let Control::Active { resume } = self.control {
            trainer.set_mode(resume);
        }
        self.control = Control::Off;
    }

    pub fn skip(&mut self, ftp_w: f64) -> Result<String, String> {
        let run = self
            .run
            .as_mut()
            .filter(|r| !r.is_finished())
            .ok_or("no workout running")?;
        self.laps += 1;
        Ok(match run.skip() {
            Some(Change::Step(i)) => step_message(run, i, ftp_w),
            _ => "skipped the last step".into(),
        })
    }

    pub fn pause(&mut self) -> Result<String, String> {
        if self.run.as_mut().is_some_and(|r| r.pause()) {
            Ok("workout paused (workout resume)".into())
        } else {
            Err("no workout running".into())
        }
    }

    pub fn resume(&mut self) -> Result<String, String> {
        match self.run.as_mut().map(|r| r.resume()) {
            Some(true) => Ok("workout resumed".into()),
            Some(false) => Err("the workout is not paused by hand".into()),
            None => Err("no workout running".into()),
        }
    }

    pub fn status(&self, ftp_w: f64) -> Result<String, String> {
        match (&self.run, &self.loaded) {
            (Some(r), _) => Ok(format!(
                "{}\nFTP {ftp_w:.0} W",
                r.status(ftp_w).describe(&r.workout().name)
            )),
            (None, Some(w)) => Ok(format!("loaded {} (not started)", summary(w))),
            (None, None) => Err("no workout loaded (workout load <file>)".into()),
        }
    }

    /// Advances the workout by `dt_s` with the rider's `power_w` and sets
    /// the trainer mode. Returns lines for the console.
    pub fn update(
        &mut self,
        dt_s: f64,
        power_w: f64,
        ftp_w: f64,
        trainer: &mut TrainerControl,
    ) -> Vec<String> {
        let mut out = Vec::new();
        let Some(run) = self.run.as_mut() else {
            return out;
        };
        if run.is_finished() {
            self.done_for_s += dt_s.max(0.0);
            if self.done_for_s > DONE_PANEL_S {
                self.run = None;
            }
            return out;
        }
        if self.control == Control::Starting {
            self.control = Control::Active {
                resume: trainer.mode(),
            };
        }
        match run.advance(dt_s, power_w) {
            Some(Change::Step(i)) => {
                self.laps += 1;
                out.push(step_message(run, i, ftp_w));
            }
            Some(Change::Finished) => {
                self.laps += 1;
                out.push(format!("workout {} done", run.workout().name));
            }
            None => {}
        }
        if let Control::Active { resume } = self.control {
            match run.target_w(ftp_w) {
                Some(target_w) => trainer.set_mode(TrainerMode::Erg { target_w }),
                // Free ride step: the road again.
                None if !run.is_finished() => trainer.set_mode(resume),
                None => {}
            }
        }
        if run.is_finished() {
            self.release(trainer);
        }
        out
    }

    /// The running workout for the menu: (name, "step 2/6").
    pub fn now(&self) -> Option<(String, String)> {
        let r = self.run.as_ref().filter(|r| !r.is_finished())?;
        let s = r.status(DEFAULT_FTP_W);
        Some((
            r.workout().name.clone(),
            format!("step {}/{}", s.index + 1, s.count),
        ))
    }

    pub fn panel(&self, ftp_w: f64) -> Option<WorkoutPanel> {
        self.run.as_ref().map(|r| WorkoutPanel::new(r, ftp_w))
    }
}

fn step_message(run: &WorkoutRun, i: usize, ftp_w: f64) -> String {
    let s = run.status(ftp_w);
    let seg = &run.workout().segments()[i];
    format!(
        "workout step {}/{}{}: {} for {}{}",
        i + 1,
        s.count,
        s.label.map(|l| format!(" ({l})")).unwrap_or_default(),
        seg.describe(ftp_w),
        format_duration(seg.duration_s),
        seg.text
            .as_ref()
            .map(|t| format!(" – {t}"))
            .unwrap_or_default()
    )
}
