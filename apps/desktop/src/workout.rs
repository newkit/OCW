//! Structured workouts in the client (spec §2.8, §5.3): load, start, stop,
//! skip; the workout's target power drives the trainer in ERG mode.
//!
//! Workouts are client-side target control: the server only sees the
//! rider's power, as in any ride (spec §6.1), so nothing here touches the
//! protocol. While a workout runs it owns the trainer mode: ERG with the
//! step's target, the mode from before the workout (normally SIM) in free
//! ride steps, and that mode again when it ends or is stopped.
//!
//! Console: `workout load <file>`, `workout start|stop|skip|pause|resume|status`.
//! Command line: `--workout <file>` (loaded and started), `--ftp <W>`.

use std::sync::Arc;

use bevy::prelude::*;
use clap::{Arg, Command};
use ocw_render::RideView;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, ConsoleState, Permission};
use ocw_sim::workout::{Change, Workout, WorkoutPanel, WorkoutRun, format_duration};

use crate::Args;
use crate::connection::ClientState;
use crate::devices::Trainer;
use crate::trainer::{TrainerControl, TrainerMode};

/// The rider's FTP in watts. The single place it comes from (today
/// `--ftp`; later the rider profile's setting).
pub fn ftp_w(args: &Args) -> f64 {
    args.ftp
}

/// How long the panel stays after the workout is done (s).
const DONE_PANEL_S: f64 = 20.0;

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
}

impl Default for WorkoutSession {
    fn default() -> Self {
        Self {
            loaded: None,
            run: None,
            control: Control::Off,
            done_for_s: 0.0,
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
    #[cfg(test)]
    pub fn loaded(&self) -> Option<&Arc<Workout>> {
        self.loaded.as_ref()
    }

    #[cfg(test)]
    pub fn run(&self) -> Option<&WorkoutRun> {
        self.run.as_ref()
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
            Some(Change::Step(i)) => out.push(step_message(run, i, ftp_w)),
            Some(Change::Finished) => out.push(format!("workout {} done", run.workout().name)),
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

/// Reads and parses a workout file (`.toml`, `.erg`, `.mrc`).
pub fn load_file(path: &std::path::Path) -> Result<Workout, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Workout::parse_file(&path.to_string_lossy(), &text).map_err(|e| e.to_string())
}

#[derive(Resource, Default)]
pub struct Workouts(pub WorkoutSession);

pub struct WorkoutPlugin;

impl Plugin for WorkoutPlugin {
    fn build(&self, app: &mut App) {
        let args = app.world().resource::<Args>().clone();
        let mut session = WorkoutSession::default();
        if let Some(path) = &args.workout {
            let started = load_file(path)
                .and_then(|w| session.load(w))
                .and_then(|_| session.start());
            match started {
                Ok(text) => info!("{text}"),
                Err(e) => {
                    eprintln!("--workout: {e}");
                    std::process::exit(1);
                }
            }
        }
        app.insert_resource(Workouts(session)).add_systems(
            Update,
            drive_workout.before(crate::devices::control_trainer),
        );
        register_commands(app);
    }
}

/// Advances the workout with game time (also during frame capture) and
/// publishes the panel.
fn drive_workout(
    time: Res<Time>,
    args: Res<Args>,
    client: Res<ClientState>,
    mut workouts: ResMut<Workouts>,
    mut trainer: ResMut<Trainer>,
    mut view: ResMut<RideView>,
    mut console: ResMut<ConsoleState>,
) {
    let ftp = ftp_w(&args);
    if client.0.rider_id().is_some() {
        let lines = workouts.0.update(
            time.delta_secs_f64(),
            client.0.power_w(),
            ftp,
            &mut trainer.0,
        );
        for l in lines {
            console.print(l);
        }
    }
    let panel = workouts.0.panel(ftp);
    if view.workout != panel {
        view.workout = panel;
    }
}

fn register_commands(app: &mut App) {
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("workout")
            .about("Structured workouts in ERG mode: load <file> (.toml, .erg, .mrc), start, stop, skip, pause, resume, status")
            .subcommand_required(true)
            .subcommand(
                Command::new("load")
                    .about("Load a workout file (.toml, .erg or .mrc)")
                    .arg(Arg::new("file").required(true)),
            )
            .subcommand(Command::new("start").about("Start the loaded workout (again from the beginning)"))
            .subcommand(Command::new("stop").about("Stop the workout; the trainer returns to its previous mode"))
            .subcommand(Command::new("skip").about("Skip to the next step"))
            .subcommand(Command::new("pause").about("Pause the workout clock"))
            .subcommand(Command::new("resume").about("Resume after workout pause"))
            .subcommand(Command::new("status").about("Show the step, target and time left")),
        |world, m| {
            let ftp = ftp_w(world.resource::<Args>());
            let Some((sub, sm)) = m.subcommand() else {
                return Err("expected a subcommand".into());
            };
            match sub {
                "load" => {
                    let path = sm.get_one::<String>("file").expect("required");
                    let w = load_file(std::path::Path::new(path))?;
                    world.resource_mut::<Workouts>().0.load(w)
                }
                "start" => world.resource_mut::<Workouts>().0.start(),
                "stop" => world.resource_scope(|world, mut w: Mut<Workouts>| {
                    w.0.stop(&mut world.resource_mut::<Trainer>().0)
                }),
                "skip" => world.resource_mut::<Workouts>().0.skip(ftp),
                "pause" => world.resource_mut::<Workouts>().0.pause(),
                "resume" => world.resource_mut::<Workouts>().0.resume(),
                "status" => world.resource::<Workouts>().0.status(ftp),
                other => Err(format!("unknown subcommand {other}")),
            }
        },
    ));
}

#[cfg(test)]
mod tests {
    use ocw_core::RiderProfile;
    use ocw_sim::workout::parse_toml;

    use super::*;

    const DEMO: &str = include_str!("../../../crates/ocw-sim/test-data/workouts/short-demo.toml");

    fn demo() -> Workout {
        parse_toml(DEMO).unwrap()
    }

    /// Runs the session for `seconds` in 1/8 s steps.
    fn ride(
        s: &mut WorkoutSession,
        t: &mut TrainerControl,
        seconds: f64,
        power: f64,
    ) -> Vec<String> {
        let mut out = Vec::new();
        for _ in 0..(seconds * 8.0) as usize {
            out.extend(s.update(0.125, power, 200.0, t));
        }
        out
    }

    #[test]
    fn workout_drives_erg_and_returns_to_sim() {
        let mut t = TrainerControl::new(RiderProfile::default());
        let mut s = WorkoutSession::default();
        assert!(s.start().is_err());
        assert!(s.load(demo()).unwrap().contains("6 steps, 2:00"));
        assert!(s.start().unwrap().contains("when you pedal"));
        // Waiting for the rider: ERG with the first target already.
        ride(&mut s, &mut t, 1.0, 0.0);
        assert_eq!(t.mode(), TrainerMode::Erg { target_w: 120.0 });
        assert_eq!(s.run().unwrap().elapsed_s(), 0.0);
        // The ramp from 60 to 90 % of 200 W.
        ride(&mut s, &mut t, 10.0, 150.0);
        assert_eq!(t.mode(), TrainerMode::Erg { target_w: 150.0 });
        let msgs = ride(&mut s, &mut t, 10.0, 150.0);
        assert_eq!(msgs, ["workout step 2/6 (1/2 on): 240 W for 0:20"]);
        assert_eq!(t.mode(), TrainerMode::Erg { target_w: 240.0 });
        assert!(s.status(200.0).unwrap().contains("step 2/6 (1/2 on)"));
        // A new workout cannot be loaded meanwhile.
        assert!(s.load(demo()).is_err());
        // Skip to the free ride step: SIM again.
        for _ in 0..4 {
            s.skip(200.0).unwrap();
        }
        ride(&mut s, &mut t, 1.0, 150.0);
        assert_eq!(t.mode(), TrainerMode::Sim);
        // Done: SIM stays, and the workout no longer controls the trainer.
        let msgs = ride(&mut s, &mut t, 20.0, 150.0);
        assert_eq!(
            msgs.last().unwrap(),
            "workout Demo: 2-minute intervals done"
        );
        assert_eq!(t.mode(), TrainerMode::Sim);
        t.set_mode(TrainerMode::Off);
        ride(&mut s, &mut t, 1.0, 150.0);
        assert_eq!(t.mode(), TrainerMode::Off);
        assert_eq!(s.panel(200.0).unwrap().target, "Workout done");
        // The panel goes after a while.
        ride(&mut s, &mut t, DONE_PANEL_S, 150.0);
        assert!(s.panel(200.0).is_none());
        assert!(s.status(200.0).unwrap().contains("not started"));
    }

    #[test]
    fn stop_restores_the_previous_mode() {
        let mut t = TrainerControl::new(RiderProfile::default());
        t.set_mode(TrainerMode::Off);
        let mut s = WorkoutSession::default();
        s.load(demo()).unwrap();
        s.start().unwrap();
        ride(&mut s, &mut t, 2.0, 100.0);
        assert!(matches!(t.mode(), TrainerMode::Erg { .. }));
        // A restart keeps the mode to return to.
        s.start().unwrap();
        ride(&mut s, &mut t, 1.0, 100.0);
        let text = s.stop(&mut t).unwrap();
        assert!(
            text.starts_with("stopped Demo: 2-minute intervals at 0:01"),
            "{text}"
        );
        assert_eq!(t.mode(), TrainerMode::Off);
        assert!(s.stop(&mut t).is_err());
        assert!(s.skip(200.0).is_err());
        assert!(s.pause().is_err());
        assert!(s.panel(200.0).is_none());
    }

    fn app(extra: &[&str]) -> App {
        use clap::Parser;
        let mut args = vec!["ocw-desktop", "--bots", "0", "--virtual"];
        args.extend_from_slice(extra);
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ocw_render::console::ConsoleCorePlugin))
            .insert_resource(Args::parse_from(args))
            .insert_resource(RideView::new(ocw_world::Route::test_loop()))
            .add_plugins((
                crate::connection::ConnectionPlugin {
                    route: ocw_world::Route::test_loop(),
                },
                crate::devices::DevicesPlugin,
                WorkoutPlugin,
            ));
        app
    }

    fn run(app: &mut App, line: &str) -> String {
        app.world_mut().resource_mut::<ConsoleState>().queue(line);
        app.update();
        let log = app.world().resource::<ConsoleState>().log();
        // The command's output starts after its echo ("> line").
        let echo = log.iter().rposition(|l| l == &format!("> {line}")).unwrap();
        log[echo + 1..].join("\n")
    }

    fn fixture(name: &str) -> String {
        format!(
            "{}/../../crates/ocw-sim/test-data/workouts/{name}",
            env!("CARGO_MANIFEST_DIR")
        )
    }

    #[test]
    fn console_commands() {
        let mut app = app(&[]);
        assert!(run(&mut app, "workout status").starts_with("error: no workout loaded"));
        assert!(run(&mut app, "workout").starts_with("error"));
        let e = run(&mut app, "workout load /nonexistent/w.toml");
        assert!(e.starts_with("error: /nonexistent/w.toml:"), "{e}");
        let out = run(
            &mut app,
            &format!("workout load \"{}\"", fixture("two-by-five.erg")),
        );
        assert!(
            out.starts_with("loaded 2x5 min at 250 W: 5 steps, 23:00"),
            "{out}"
        );
        assert!(run(&mut app, "workout start").starts_with("started"));
        assert!(run(&mut app, "workout status").contains("step 1/5"));
        assert!(run(&mut app, "workout skip").starts_with("workout step 2/5: 250 W"));
        assert!(app.world().resource::<RideView>().workout.is_some());
        assert!(run(&mut app, "workout pause").contains("paused"));
        assert!(run(&mut app, "workout resume").contains("resumed"));
        assert!(run(&mut app, "workout stop").starts_with("stopped"));
        assert!(run(&mut app, "workout stop").starts_with("error"));
    }

    #[test]
    fn workout_flag_loads_and_starts() {
        let path = fixture("sweet-spot.toml");
        let mut app = app(&["--workout", &path, "--ftp", "300"]);
        app.update();
        let w = &app.world().resource::<Workouts>().0;
        assert!(w.run().is_some());
        assert_eq!(w.loaded().unwrap().name, "Sweet spot 3 × 10");
        let status = run(&mut app, "workout status");
        assert!(status.contains("FTP 300 W"), "{status}");
        // 50 % of 300 W at the start of the ramp.
        assert!(status.contains("target 150 W"), "{status}");
    }

    #[test]
    fn pause_and_resume_by_hand() {
        let mut t = TrainerControl::new(RiderProfile::default());
        let mut s = WorkoutSession::default();
        s.load(demo()).unwrap();
        s.start().unwrap();
        ride(&mut s, &mut t, 2.0, 100.0);
        assert!(s.resume().is_err());
        s.pause().unwrap();
        ride(&mut s, &mut t, 5.0, 100.0);
        assert_eq!(s.run().unwrap().elapsed_s(), 2.0);
        assert!(
            s.panel(200.0)
                .unwrap()
                .lines
                .iter()
                .any(|l| l.contains("paused"))
        );
        s.resume().unwrap();
        ride(&mut s, &mut t, 1.0, 100.0);
        assert_eq!(s.run().unwrap().elapsed_s(), 3.0);
    }
}
