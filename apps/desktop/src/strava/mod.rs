//! Strava upload and sync of saved rides (spec §2.8).
//!
//! **The simple way, for every rider** ([`manual`], spec §2.8.1 *Simple
//! upload*): no API application – the ride's FIT file at hand, Strava's
//! own upload page in the browser (`strava.upload-manual`, the ride's
//! page in the training history).
//!
//! **The advanced way** (the interim path until the backend exists, spec
//! §2.8, §14 Phase 5): the user registers their own Strava API
//! application and configures its client id and secret
//! (`OCW_STRAVA_CLIENT_ID` / `OCW_STRAVA_CLIENT_SECRET` or `<config
//! dir>/ocw/strava.toml`); nothing is built in. With them every saved ride
//! is uploaded automatically (except those marked as uploaded by hand).
//!
//! **Recording without Strava set up, uploaded later** (spec §2.8.1):
//! every S recording made without the credentials is kept as *waiting*
//! in the sync state ([`sync::RideSync::Waiting`]). The credentials may
//! come later – at the next start, or while the app runs: the config
//! file is looked at every [`WATCH_S`] seconds (the environment only at
//! the start), and `strava.connect` / `strava.status` / `strava.sync`
//! look at once. Then the worker starts and, once the account is linked,
//! uploads the waiting recordings oldest first – asking once when more
//! than [`worker::ASK_ABOVE`] wait (the menu's *Upload N waiting
//! recordings to Strava? Yes / Pick / No*, `strava.waiting`,
//! `strava.sync --waiting`); the HUD says how many went up. Each ride's
//! state shows on its page in the training history ([`history_states`]).
//!
//! Everything that touches the network runs on the `ocw-strava` thread
//! ([`worker`]); the Bevy side only sends commands and prints messages.
//! `OCW_STRAVA_FAKE=1` (test runs only) answers every request with a
//! stand-in for Strava ([`api::FakeStrava`]) – no network.

pub mod api;
pub mod auth;
pub mod manual;
pub mod recording;
pub mod sync;
pub mod worker;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use bevy::prelude::*;
use clap::{Arg, ArgAction, Command};
use ocw_client::menu::history::StravaState;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, ConsoleState, Permission};

use auth::{ENV_CLIENT_ID, ENV_CLIENT_SECRET, StravaConfig, TokenStore};
use sync::{RideSync, SyncPaths, SyncState};
use worker::{Command as Cmd, Event, Worker, WorkerConfig};

/// How often the config file is looked at while the credentials are
/// missing (s).
pub const WATCH_S: f32 = 3.0;
/// How long the notice after the catch-up upload stays (s).
const CAUGHT_UP_NOTICE_S: f64 = 8.0;

/// `<config dir>/ocw`; `OCW_CONFIG_DIR` overrides it.
pub fn default_config_dir() -> Option<PathBuf> {
    match std::env::var_os("OCW_CONFIG_DIR") {
        Some(d) => Some(PathBuf::from(d)),
        None => Some(dirs::config_dir()?.join("ocw")),
    }
}

/// The sync state's file in the rides folder.
pub fn state_file(rides: &Path) -> PathBuf {
    rides.join("strava-sync.toml")
}

#[derive(Default)]
pub struct StravaPlugin {
    /// Overrides [`default_config_dir`].
    pub config_dir: Option<PathBuf>,
    /// Ignore the environment variables (tests).
    pub ignore_env: bool,
    /// Talk to the stand-in Strava ([`api::FakeStrava`]; tests, and
    /// `OCW_STRAVA_FAKE=1` for test runs of the app).
    pub fake_strava: bool,
}

/// The Strava link; `worker` is `None` when not configured.
#[derive(Resource)]
pub struct Strava {
    worker: Option<Worker>,
    /// Why it is off.
    off_reason: String,
    /// Where the credentials may still come from (while they are missing).
    watch: Option<Watch>,
}

/// Looking for the credentials while the app runs.
struct Watch {
    config_dir: PathBuf,
    rides_dir: PathBuf,
    /// The environment's credentials as they were at the start (it is not
    /// read again).
    env: Vec<(String, String)>,
    /// The config file's modification time when last looked at.
    seen: Option<SystemTime>,
    /// The stand-in Strava instead of Strava.
    fake: bool,
    /// Seconds to the next look.
    next_s: f32,
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

impl Strava {
    /// The automatic API upload is set up (the advanced way).
    pub fn api(&self) -> bool {
        self.worker.is_some()
    }

    /// Tells the sync about a newly saved ride.
    pub fn ride_saved(&self) {
        if let Some(w) = &self.worker {
            w.send(Cmd::RideSaved);
        }
    }

    /// Starts the worker if the credentials are there now; whether it
    /// started.
    fn try_start(&mut self) -> bool {
        if self.worker.is_some() {
            return false;
        }
        let Some(w) = &mut self.watch else {
            return false;
        };
        w.seen = modified(&w.config_dir.join("strava.toml"));
        let env = w.env.clone();
        let lookup = move |k: &str| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        match start(&w.config_dir, w.rides_dir.clone(), &lookup, w.fake) {
            Ok(worker) => {
                self.worker = Some(worker);
                self.off_reason.clear();
                self.watch = None;
                true
            }
            Err(why) => {
                self.off_reason = why;
                false
            }
        }
    }

    pub fn status_text(&self) -> String {
        let Some(w) = &self.worker else {
            return self.off_reason.clone();
        };
        let s = w.status();
        let mut out = if s.connected {
            format!(
                "Strava: linked{} (automatic upload {})",
                s.athlete.map(|a| format!(" as {a}")).unwrap_or_default(),
                if s.auto_upload { "on" } else { "off" }
            )
        } else {
            "Strava: configured, not linked (run strava.connect)".to_string()
        };
        let c = &s.counts;
        out.push_str(&format!(
            "\nrides: {} uploaded, {} pending, {} waiting, {} failed, {} rejected",
            c.uploaded, c.pending, c.waiting, c.failed, c.rejected
        ));
        if let Some(at) = c.next_retry {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as i64);
            out.push_str(&format!("; next retry in {} s", (at - now).max(0)));
        }
        if let Some(n) = s.question {
            out.push_str(&format!(
                "\n{n} waiting recordings: upload them? (strava.waiting yes | pick | no)"
            ));
        }
        if let Some(b) = s.busy {
            out.push_str(&format!("\nnow: {b}"));
        }
        if let Some(l) = s.last {
            out.push_str(&format!("\nlast: {l}"));
        }
        out
    }
}

impl Plugin for StravaPlugin {
    fn build(&self, app: &mut App) {
        let rides_dir = app
            .world()
            .get_resource::<crate::recording::Recording>()
            .and_then(|r| r.dir.clone())
            .or_else(crate::recording::default_rides_dir);
        let config_dir = self.config_dir.clone().or_else(default_config_dir);
        // The environment is read here only.
        let env: Vec<(String, String)> = if self.ignore_env {
            Vec::new()
        } else {
            [ENV_CLIENT_ID, ENV_CLIENT_SECRET]
                .iter()
                .filter_map(|k| Some((k.to_string(), std::env::var(k).ok()?)))
                .collect()
        };
        let strava = match (config_dir, rides_dir) {
            (Some(config_dir), Some(rides_dir)) => {
                let mut s = Strava {
                    worker: None,
                    off_reason: String::new(),
                    watch: Some(Watch {
                        config_dir,
                        rides_dir: rides_dir.clone(),
                        env,
                        seen: None,
                        fake: self.fake_strava || std::env::var_os(api::ENV_FAKE).is_some(),
                        next_s: WATCH_S,
                    }),
                };
                if !s.try_start() {
                    // Recordings from before (made without the set-up)
                    // wait too.
                    if rides_dir.join("strava").is_dir()
                        && let Err(e) = sync::mark_waiting(
                            &state_file(&rides_dir),
                            &sync::sync_files(&rides_dir),
                            crate::recording::unix_now(),
                        )
                    {
                        warn!("Strava's waiting recordings: {e}");
                    }
                }
                s
            }
            _ => Strava {
                worker: None,
                off_reason: "Strava: off (no config or data directory)".into(),
                watch: None,
            },
        };
        if strava.worker.is_none() {
            info!("{}", strava.off_reason.lines().next().unwrap_or_default());
        }
        // The S recording (spec §2.8.1) records with or without the API
        // set-up; without it the rider uploads in the browser. Test runs
        // (`--strava-dry-run`) open neither the file manager nor the
        // browser.
        let dry_run = app
            .world()
            .get_resource::<crate::Args>()
            .is_some_and(|a| a.strava_dry_run);
        let api = strava.worker.is_some();
        app.insert_resource(strava)
            .insert_resource(manual::ManualOpen(!dry_run))
            .add_systems(Update, (watch_credentials, poll_worker).chain());
        recording::build(app, api);
        register_commands(app);
    }
}

/// Starts the worker with the credentials from `strava.toml` or `env`;
/// `Err`: why Strava stays off.
fn start(
    config_dir: &Path,
    rides_dir: PathBuf,
    env: &dyn Fn(&str) -> Option<String>,
    fake: bool,
) -> Result<Worker, String> {
    let config_file = config_dir.join("strava.toml");
    let config = StravaConfig::load(&config_file).map_err(|e| format!("Strava: off, {e}"))?;
    let Some(creds) = config.credentials(env) else {
        return Err(format!(
            "Strava: no API application set up (the advanced way). Upload rides in your browser instead: \
             Navi > Training history > the ride > Upload to Strava, or strava.upload-manual N. \
             Strava recordings (S) wait and are uploaded once it is set up. \
             For automatic uploads register an API application at https://www.strava.com/settings/api \
             (authorization callback domain: localhost), then set {ENV_CLIENT_ID} and {ENV_CLIENT_SECRET} \
             or put client_id and client_secret into {}",
            config_file.display()
        ));
    };
    let http: Arc<dyn api::Http> = if fake {
        warn!(
            "Strava: {} is set, talking to a stand-in, not to Strava",
            api::ENV_FAKE
        );
        Arc::new(api::FakeStrava::default())
    } else {
        Arc::new(api::UreqHttp::default())
    };
    let cfg = WorkerConfig {
        creds,
        paths: SyncPaths {
            state_file: state_file(&rides_dir),
            rides_dir,
            token: TokenStore {
                path: config_dir.join("strava-token.toml"),
            },
        },
        auto_upload: config.auto_upload.unwrap_or(true),
        http,
        open_url: auth::open_browser,
        sleep: std::thread::sleep,
    };
    Worker::spawn(cfg).map_err(|e| format!("Strava: off, cannot start its thread: {e}"))
}

/// Looks for the credentials while they are missing: the config file
/// changed (or appeared) – start the worker.
fn watch_credentials(world: &mut World) {
    let dt = world
        .get_resource::<Time<Real>>()
        .map_or(0.0, |t| t.delta_secs());
    let mut s = world.resource_mut::<Strava>();
    let Some(w) = s.watch.as_mut() else {
        return;
    };
    w.next_s -= dt;
    if w.next_s > 0.0 {
        return;
    }
    w.next_s = WATCH_S;
    let now = modified(&w.config_dir.join("strava.toml"));
    if now.is_none() || now == w.seen {
        return;
    }
    if s.try_start() {
        started(world);
    }
}

/// Starts the worker now if the credentials came (the commands look at
/// once, not after [`WATCH_S`]).
fn start_if_set_up(world: &mut World) {
    if world.resource_mut::<Strava>().try_start() {
        started(world);
    }
}

/// The worker started while the app runs: the recording knows, the
/// history shows the API's rows.
fn started(world: &mut World) {
    let msg = "Strava set up: the API credentials are there; waiting recordings are uploaded once the account is linked (strava.connect)";
    info!("{msg}");
    world.resource_mut::<ConsoleState>().print(msg);
    if let Some(mut r) = world.get_resource_mut::<recording::StravaRec>() {
        r.0.api = true;
    }
    refresh_history(world);
}

fn refresh_history(world: &mut World) {
    if let Some(refresh) = world
        .get_resource::<ocw_ui::history::HistoryHooks>()
        .map(|h| h.refresh)
    {
        refresh(world);
    }
}

/// What the history last saw of the worker: the file uploading, the
/// counts, the question.
type Progress = (Option<String>, sync::SyncCounts, Option<usize>);

/// The background thread's messages in the console, its events in the
/// HUD and the menu; the history's states follow its progress.
fn poll_worker(world: &mut World, mut last: Local<Option<Progress>>) {
    let Some(w) = world.resource::<Strava>().worker.as_ref() else {
        return;
    };
    let messages: Vec<String> = w
        .messages
        .lock()
        .expect("not poisoned")
        .try_iter()
        .collect();
    let events: Vec<Event> = w.events.lock().expect("not poisoned").try_iter().collect();
    let s = w.status();
    for m in messages {
        world.resource_mut::<ConsoleState>().print(m);
    }
    let seen = (s.uploading.clone(), s.counts.clone(), s.question);
    let changed = last.as_ref() != Some(&seen);
    *last = Some(seen);
    if changed || !events.is_empty() {
        refresh_history(world);
    }
    for e in events {
        match e {
            Event::Ask(_) => {
                if let Some(mut m) = world.get_resource_mut::<ocw_ui::menu::MenuState>() {
                    m.queue(ocw_client::menu::model::Action::OpenSub(
                        ocw_client::menu::pages::Page::StravaWaiting,
                    ));
                }
            }
            Event::CaughtUp { uploaded, failed } => {
                let now = crate::recording::game_now(world);
                if let Some(mut r) = world.get_resource_mut::<recording::StravaRec>() {
                    r.0.say_for(
                        worker::caught_up_text(uploaded, failed),
                        now,
                        CAUGHT_UP_NOTICE_S,
                    );
                }
            }
        }
    }
}

/// A sync file's state for the ride page.
fn file_state(state: &SyncState, file: &str, uploading: Option<&str>) -> Option<StravaState> {
    if uploading == Some(file) {
        return Some(StravaState::Uploading);
    }
    Some(match state.rides.get(file)? {
        RideSync::Waiting { .. } => StravaState::Waiting,
        RideSync::Processing { .. } => StravaState::Uploading,
        RideSync::Uploaded { activity_id, .. } => StravaState::OnStrava {
            activity_id: *activity_id,
        },
        RideSync::Failed { error, .. } | RideSync::Rejected { error } => StravaState::Failed {
            error: error.clone(),
        },
    })
}

/// Each ride's Strava state (its Strava recordings', else its own
/// file's) and the open question, for the training history's pages.
pub fn history_states(world: &World) -> (BTreeMap<String, StravaState>, Option<usize>) {
    let mut out = BTreeMap::new();
    let status = world
        .get_resource::<Strava>()
        .and_then(|s| s.worker.as_ref())
        .map(|w| w.status());
    let question = status.as_ref().and_then(|s| s.question);
    let (Some(rides), Some(h)) = (
        world
            .get_resource::<crate::recording::Recording>()
            .and_then(|r| r.dir.clone()),
        world.get_resource::<ocw_ui::history::TrainingHistory>(),
    ) else {
        return (out, question);
    };
    let state = SyncState::load(&state_file(&rides)).unwrap_or_default();
    let uploading = status.as_ref().and_then(|s| s.uploading.as_deref());
    let all = manual::recordings(&rides);
    for r in &h.history.rides {
        let recs = manual::within(&all, r);
        let files: Vec<String> = if recs.is_empty() {
            r.fit.iter().cloned().collect()
        } else {
            recs.into_iter().map(|(n, _)| n).collect()
        };
        let states: Vec<Option<StravaState>> = files
            .iter()
            .map(|f| file_state(&state, f, uploading))
            .collect();
        if let Some(s) = StravaState::of_files(&states) {
            out.insert(r.id.clone(), s);
        }
    }
    (out, question)
}

fn worker(world: &mut World) -> Result<&Worker, String> {
    start_if_set_up(world);
    let s = world.resource::<Strava>();
    s.worker.as_ref().ok_or_else(|| s.off_reason.clone())
}

/// The sync names of ride `n` (1 = the newest) that wait: its waiting
/// Strava recordings.
fn waiting_of_ride(world: &World, n: usize) -> Result<Vec<String>, String> {
    let ride = ocw_ui::history::nth(world, n)?;
    let rides = world
        .get_resource::<crate::recording::Recording>()
        .and_then(|r| r.dir.clone())
        .ok_or("no rides folder")?;
    let state = SyncState::load(&state_file(&rides))?;
    let files: Vec<String> = manual::recordings_within(&rides, &ride)
        .into_iter()
        .map(|(f, _)| f)
        .filter(|f| matches!(state.rides.get(f), Some(RideSync::Waiting { .. })))
        .collect();
    if files.is_empty() {
        return Err(format!("ride {n} has no waiting Strava recording"));
    }
    Ok(files)
}

fn register_commands(app: &mut App) {
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("strava.connect")
            .about("Link your Strava account (opens the Strava login in the browser)"),
        |world, _| {
            worker(world)?.send(Cmd::Connect);
            Ok("opening the Strava login in the browser…".into())
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("strava.sync")
            .about("Upload every saved ride that is not on Strava yet (in the background)")
            .arg(
                Arg::new("retry")
                    .long("retry")
                    .action(ArgAction::SetTrue)
                    .help("Also retry failed and rejected rides now"),
            )
            .arg(
                Arg::new("waiting")
                    .long("waiting")
                    .action(ArgAction::SetTrue)
                    .help("Upload the Strava recordings made before Strava was set up (oldest first)"),
            )
            .arg(
                Arg::new("ride")
                    .long("ride")
                    .value_parser(clap::value_parser!(usize))
                    .requires("waiting")
                    .help("With --waiting: only the waiting recordings of this ride (1 = the newest)"),
            ),
        |world, m| {
            if m.get_flag("waiting") {
                let files = match m.get_one::<usize>("ride") {
                    Some(&n) => Some(waiting_of_ride(world, n)?),
                    None => None,
                };
                let what = files.as_ref().map_or_else(
                    || "the waiting recordings".to_string(),
                    |f| format!("{} waiting recording{}", f.len(), if f.len() == 1 { "" } else { "s" }),
                );
                worker(world)?.send(Cmd::SyncWaiting { files });
                return Ok(format!("uploading {what} to Strava"));
            }
            let retry = m.get_flag("retry");
            worker(world)?.send(Cmd::Sync { retry });
            Ok("Strava sync started".into())
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("strava.waiting")
            .about("Answer the question about the Strava recordings made before Strava was set up: yes (upload them all), pick (the training history's waiting rides), no (they stay waiting)")
            .arg(
                Arg::new("answer")
                    .value_parser(["yes", "pick", "no"])
                    .required(true),
            ),
        |world, m| {
            let answer = m.get_one::<String>("answer").map_or("no", String::as_str);
            let w = worker(world)?;
            match answer {
                "yes" => {
                    w.send(Cmd::SyncWaiting { files: None });
                    Ok("uploading the waiting recordings to Strava".into())
                }
                "pick" => {
                    // The menu's row opens the list itself; the console
                    // opens it here.
                    w.send(Cmd::KeepWaiting);
                    if let Some(mut s) = world.get_resource_mut::<ocw_ui::menu::MenuState>()
                        && s.menu.page()
                            != Some(ocw_client::menu::model::Page::Sub(
                                ocw_client::menu::pages::Page::HistoryWaiting,
                            ))
                    {
                        s.queue(ocw_client::menu::model::Action::OpenSub(
                            ocw_client::menu::pages::Page::HistoryWaiting,
                        ));
                    }
                    Ok("choose a ride, then Strava upload on its page".into())
                }
                _ => {
                    w.send(Cmd::KeepWaiting);
                    Ok("the waiting recordings stay waiting (strava.sync --waiting uploads them)".into())
                }
            }
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("strava.open")
            .about("Open a ride's Strava activity in the browser (N: its number in `history`, 1 = the newest)")
            .arg(Arg::new("n").value_parser(clap::value_parser!(usize)).default_value("1")),
        |world, m| {
            let n = *m.get_one::<usize>("n").expect("default");
            let ride = ocw_ui::history::nth(world, n)?;
            let (states, _) = history_states(world);
            let Some(StravaState::OnStrava {
                activity_id: Some(id),
            }) = states.get(&ride.id)
            else {
                return Err(format!("ride {n} has no single Strava activity known"));
            };
            let url = ocw_client::menu::history::activity_url(*id);
            if world.get_resource::<manual::ManualOpen>().is_some_and(|o| o.0) {
                manual::spawn(&manual::browser_command(manual::Os::current(), &url))?;
            }
            Ok(url)
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("strava.status").about("Show the Strava link and upload state"),
        |world, _| {
            start_if_set_up(world);
            Ok(world.resource::<Strava>().status_text())
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("strava.disconnect").about("Unlink Strava (deletes the stored tokens)"),
        |world, _| {
            worker(world)?.send(Cmd::Disconnect);
            Ok("disconnecting Strava…".into())
        },
    ));
}

#[cfg(test)]
mod tests {
    use ocw_client::recording::{RideRecorder, Sample};
    use ocw_render::console::ConsoleCorePlugin;
    use ocw_world::Route;

    use super::*;
    use crate::recording::Recording;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ocw-strava-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// An app with the Strava plugin, its rides in `dir/rides`, its
    /// config in `dir/config`.
    fn app(dir: &Path) -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ConsoleCorePlugin))
            .insert_resource(Recording {
                recorder: RideRecorder::new("Loop"),
                dir: Some(dir.join("rides")),
                enabled: true,
                hearts_start: None,
                workout: None,
            })
            .add_plugins(StravaPlugin {
                config_dir: Some(dir.join("config")),
                ignore_env: true,
                // No network: the stand-in answers.
                fake_strava: true,
            });
        app
    }

    /// Runs `cmd`; the console's lines since.
    fn run(app: &mut App, cmd: &str) -> String {
        let from = app.world().resource::<ConsoleState>().log().len();
        app.world_mut().resource_mut::<ConsoleState>().queue(cmd);
        app.update();
        app.world().resource::<ConsoleState>().log()[from..].join("\n")
    }

    /// One S recording of 30 s from `start` (UTC Unix s), stopped.
    fn record(app: &mut App, start: i64) -> String {
        let route = Route::test_loop();
        let out = app
            .world_mut()
            .resource_mut::<recording::StravaRec>()
            .0
            .start(0.0);
        recording::handle(app.world_mut(), out, 0.0).unwrap();
        {
            let mut s = app.world_mut().resource_mut::<recording::StravaRec>();
            for i in 0..30 {
                s.0.observe(
                    &route,
                    None,
                    Sample {
                        game_s: i as f64,
                        distance: i as f64 * 8.0,
                        speed: 8.0,
                        power_w: 200.0,
                        ..Sample::default()
                    },
                    start + i,
                );
            }
        }
        let out = app
            .world_mut()
            .resource_mut::<recording::StravaRec>()
            .0
            .stop(30.0);
        recording::handle(app.world_mut(), out, 30.0).unwrap()
    }

    fn state(dir: &Path) -> SyncState {
        SyncState::load(&state_file(&dir.join("rides"))).unwrap()
    }

    fn wait_until(app: &mut App, what: &str, done: impl Fn(&mut App) -> bool) {
        let start = std::time::Instant::now();
        while !done(app) {
            assert!(
                start.elapsed() < std::time::Duration::from_secs(10),
                "timed out: {what}"
            );
            app.update();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[test]
    fn without_configuration_strava_stays_off() {
        let dir = temp("off");
        let mut app = app(&dir);
        for cmd in ["strava.status", "strava.connect", "strava.sync"] {
            let last = run(&mut app, cmd);
            assert!(
                last.contains("no API application set up") && last.contains("strava.upload-manual"),
                "{cmd}: {last}"
            );
        }
        assert!(!app.world().resource::<Strava>().api());
        // S (`strava.record`) records without the API too (spec §2.8.1
        // *Simple upload*); the rider uploads it in the browser.
        assert!(run(&mut app, "strava.record").contains("Strava recording started"));
        let rec = &app.world().resource::<recording::StravaRec>().0;
        assert!(rec.is_recording() && !rec.api && rec.waits);
        // No configuration was written.
        assert!(!dir.join("config").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Spec §2.8.1 *Recording without Strava set up, uploaded later*:
    /// recordings made without the credentials wait (not failed, not
    /// uploaded; the stop says it); the credentials appear while the app
    /// runs – the worker starts within a few seconds and, the account
    /// linked, uploads them oldest first through the usual upload (the
    /// stand-in Strava, no network), skipping one marked as uploaded by
    /// hand; the HUD's notice counts them; whole rides keep their rule.
    #[test]
    fn credentials_appearing_while_the_app_runs_upload_the_waiting_recordings() {
        let dir = temp("later");
        let mut app = app(&dir);
        // Recorded out of order: the upload goes by the recordings' times.
        for t in [3_000_000, 1_000_000, 2_000_000] {
            let said = record(&mut app, 1_794_000_000 + t);
            assert!(
                said.contains("It will be uploaded once Strava is set up"),
                "{said}"
            );
            let notice = app
                .world()
                .resource::<recording::StravaRec>()
                .0
                .notice(30.0)
                .unwrap()
                .to_string();
            assert!(
                notice.contains(ocw_client::strava::WAITS_FOR_SET_UP),
                "{notice}"
            );
        }
        let st = state(&dir);
        assert_eq!(st.rides.len(), 3);
        assert!(
            st.rides
                .values()
                .all(|s| matches!(s, RideSync::Waiting { .. })),
            "{st:?}"
        );
        let mut waiting: Vec<String> = st.rides.keys().cloned().collect();
        waiting.sort();
        // One marked as uploaded by hand: skipped.
        let rides = dir.join("rides");
        sync::ManualMarks {
            files: [waiting[1].clone()].into_iter().collect(),
        }
        .save(&rides)
        .unwrap();
        // A whole ride without S: not waiting (auto_upload decides).
        std::fs::write(rides.join("2026-11-01_09-00-00.fit"), b"FIT").unwrap();
        // The credentials appear (made-up), the account was linked.
        let config = dir.join("config");
        TokenStore {
            path: config.join("strava-token.toml"),
        }
        .save(&auth::Token {
            access_token: "a".into(),
            refresh_token: "r".into(),
            expires_at: i64::MAX / 2,
            athlete: Some("Ada Rider".into()),
        })
        .unwrap();
        std::fs::write(
            config.join("strava.toml"),
            "client_id = \"12345\"\nclient_secret = \"made-up\"\nauto_upload = false\n",
        )
        .unwrap();
        wait_until(&mut app, "the worker starts", |a| {
            a.world().resource::<Strava>().api()
        });
        assert!(app.world().resource::<recording::StravaRec>().0.api);
        wait_until(&mut app, "the waiting ones go up", |_| {
            state(&dir)
                .rides
                .values()
                .filter(|s| matches!(s, RideSync::Uploaded { .. }))
                .count()
                == 2
        });
        let st = state(&dir);
        // Oldest first: the stand-in numbers its activities in order.
        let id = |f: &str| match &st.rides[f] {
            RideSync::Uploaded { activity_id, .. } => activity_id.unwrap(),
            s => panic!("{f}: {s:?}"),
        };
        assert!(id(&waiting[0]) < id(&waiting[2]));
        assert!(
            matches!(st.rides[&waiting[1]], RideSync::Waiting { .. }),
            "marked by hand: not sent"
        );
        assert!(
            !st.rides.contains_key("2026-11-01_09-00-00.fit"),
            "the whole ride follows auto_upload (off)"
        );
        wait_until(&mut app, "the notice", |a| {
            a.world()
                .resource::<recording::StravaRec>()
                .0
                .notice(crate::recording::game_now(a.world()))
                == Some("2 recordings uploaded to Strava")
        });
        let status = run(&mut app, "strava.status");
        assert!(status.contains("2 uploaded"), "{status}");
        // A restart with the credentials: nothing waits, nothing asked.
        drop(app);
        let mut again = self::app(&dir);
        assert!(again.world().resource::<Strava>().api());
        again.update();
        assert!(
            again
                .world()
                .resource::<Strava>()
                .worker
                .as_ref()
                .unwrap()
                .status()
                .question
                .is_none()
        );
        drop(again);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Restarting with more than three waiting recordings and the
    /// credentials: asked once – the menu opens at the question; *No*
    /// leaves them waiting, `strava.sync --waiting` uploads them; the
    /// ride page shows each state.
    #[test]
    fn a_restart_with_many_waiting_asks_first() {
        let dir = temp("ask");
        let rides = dir.join("rides");
        let config = dir.join("config");
        std::fs::create_dir_all(rides.join("strava")).unwrap();
        for d in 1..=4 {
            std::fs::write(
                rides.join(format!("strava/2026-11-0{d}_10-00-00.fit")),
                b"FIT",
            )
            .unwrap();
        }
        // The first start without the set-up: the older recordings wait.
        drop(app(&dir));
        assert_eq!(state(&dir).counts(&sync::sync_files(&rides)).waiting, 4);
        TokenStore {
            path: config.join("strava-token.toml"),
        }
        .save(&auth::Token {
            access_token: "a".into(),
            refresh_token: "r".into(),
            expires_at: i64::MAX / 2,
            athlete: Some("Bea Climber".into()),
        })
        .unwrap();
        std::fs::write(
            config.join("strava.toml"),
            "client_id = \"12345\"\nclient_secret = \"made-up\"\n",
        )
        .unwrap();
        let mut app = app(&dir);
        app.init_resource::<ocw_ui::history::TrainingHistory>();
        wait_until(&mut app, "the question", |a| {
            history_states(a.world()).1 == Some(4)
        });
        assert_eq!(state(&dir).counts(&sync::sync_files(&rides)).waiting, 4);
        assert!(run(&mut app, "strava.waiting no").contains("stay waiting"));
        wait_until(&mut app, "answered", |a| {
            history_states(a.world()).1.is_none()
        });
        assert_eq!(state(&dir).counts(&sync::sync_files(&rides)).waiting, 4);
        assert!(
            run(&mut app, "strava.sync --waiting").contains("uploading the waiting recordings")
        );
        wait_until(&mut app, "uploaded", |a| {
            let _ = a;
            state(&dir).counts(&sync::sync_files(&rides)).uploaded == 4
        });
        drop(app);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_files_state_on_the_ride_page() {
        let mut s = SyncState::default();
        s.rides.insert("w".into(), RideSync::Waiting { since: 1 });
        s.rides
            .insert("p".into(), RideSync::Processing { upload_id: 1 });
        s.rides.insert(
            "u".into(),
            RideSync::Uploaded {
                activity_id: Some(7),
                at: 1,
            },
        );
        s.rides.insert(
            "r".into(),
            RideSync::Rejected {
                error: "bad".into(),
            },
        );
        assert_eq!(file_state(&s, "w", None), Some(StravaState::Waiting));
        assert_eq!(file_state(&s, "w", Some("w")), Some(StravaState::Uploading));
        assert_eq!(file_state(&s, "p", None), Some(StravaState::Uploading));
        assert_eq!(
            file_state(&s, "u", None),
            Some(StravaState::OnStrava {
                activity_id: Some(7)
            })
        );
        assert_eq!(
            file_state(&s, "r", None),
            Some(StravaState::Failed {
                error: "bad".into()
            })
        );
        assert_eq!(file_state(&s, "new", None), None);
    }
}
