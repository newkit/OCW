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
//! dir>/ocw/strava.toml`); nothing is built in. With them the Strava
//! recordings (S … S) are uploaded – automatically while the setting
//! `strava.auto_upload` is on (default; it replaces `auto_upload` in
//! `strava.toml`, read once: [`AUTO_SETTING`]), by hand otherwise (the
//! ride page's *Upload now*, `strava.sync`) – except those marked as
//! uploaded by hand. **Whole rides never go to Strava** (spec 2.8.1 Q6,
//! v5.00); whole rides uploaded before keep their state.
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
/// The setting of the automatic upload (spec 2.8.1 Q6).
pub const AUTO_SETTING: &str = "strava.auto_upload";
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
    /// The automatic upload of the Strava recordings ([`AUTO_SETTING`]).
    auto: bool,
    /// The setting has a value of its own (in `settings.toml`, the
    /// command line or `set --temp`); else an old `auto_upload` in
    /// `strava.toml` is read once into it.
    auto_set: bool,
    /// The old `strava.toml` key read now: to be saved as the setting.
    migrated: Option<bool>,
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

    /// The automatic upload of the Strava recordings is on.
    pub fn auto(&self) -> bool {
        self.auto
    }

    /// Reads an old `auto_upload` of `strava.toml` once, while the
    /// setting has no value of its own (spec 2.8.1 Q6).
    fn read_old_key(&mut self, config_file: &Path) {
        if self.auto_set {
            return;
        }
        if let Ok(StravaConfig {
            auto_upload: Some(v),
            ..
        }) = StravaConfig::load(config_file)
        {
            self.auto = v;
            self.auto_set = true;
            self.migrated = Some(v);
        }
    }

    /// Starts the worker if the credentials are there now; whether it
    /// started.
    fn try_start(&mut self) -> bool {
        if self.worker.is_some() {
            return false;
        }
        let Some(file) = self.watch.as_ref().map(|w| w.config_dir.join("strava.toml")) else {
            return false;
        };
        self.read_old_key(&file);
        let auto = self.auto;
        let Some(w) = &mut self.watch else {
            return false;
        };
        w.seen = modified(&file);
        let env = w.env.clone();
        let lookup = move |k: &str| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        match start(&w.config_dir, w.rides_dir.clone(), &lookup, w.fake, auto) {
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

    /// The automatic upload's line of `strava.status`.
    fn auto_line(&self) -> String {
        if self.auto {
            "Only Strava recordings (S … S) go to Strava, never whole rides; automatic upload: on (set strava.auto_upload off: you upload them from the ride's page)".into()
        } else {
            "Only Strava recordings (S … S) go to Strava, never whole rides; automatic upload: off – they wait for you: Navi > Training history > the ride > Upload now, or strava.sync (set strava.auto_upload on)".into()
        }
    }

    pub fn status_text(&self) -> String {
        let Some(w) = &self.worker else {
            return format!("{}\n{}", self.off_reason, self.auto_line());
        };
        let s = w.status();
        let mut out = if s.connected {
            format!(
                "Strava: linked{}",
                s.athlete.map(|a| format!(" as {a}")).unwrap_or_default(),
            )
        } else {
            "Strava: configured, not linked (run strava.connect)".to_string()
        };
        out.push('\n');
        out.push_str(&self.auto_line());
        let c = &s.counts;
        out.push_str(&format!(
            "\nStrava recordings: {} uploaded, {} not uploaded yet, {} waiting, {} failed, {} rejected",
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
        // The automatic upload (spec 2.8.1 Q6): the setting, else (once)
        // an old `auto_upload` in `strava.toml`.
        let (auto, auto_set) = app
            .world()
            .get_resource::<ocw_ui::settings::UserSettings>()
            .and_then(|us| us.settings.get_with_source(AUTO_SETTING).ok())
            .map_or((true, false), |(v, source)| {
                (
                    v.as_bool() != Some(false),
                    source != ocw_client::settings::Source::Default,
                )
            });
        let strava = match (config_dir, rides_dir) {
            (Some(config_dir), Some(rides_dir)) => {
                let mut s = Strava {
                    worker: None,
                    off_reason: String::new(),
                    auto,
                    auto_set,
                    migrated: None,
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
                auto,
                auto_set,
                migrated: None,
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
        let auto = strava.auto;
        app.insert_resource(strava)
            .insert_resource(manual::ManualOpen(!dry_run))
            .add_systems(Update, (watch_credentials, poll_worker).chain());
        recording::build(app, api, auto);
        save_migrated(app.world_mut());
        register_commands(app);
    }
}

/// An old `auto_upload` of `strava.toml` was read: it becomes the setting
/// `strava.auto_upload` (saved, so it is read only once; spec 2.8.1 Q6).
fn save_migrated(world: &mut World) {
    let Some(v) = world.resource_mut::<Strava>().migrated.take() else {
        return;
    };
    let value = if v { "on" } else { "off" };
    ocw_ui::settings::save_setting(world, AUTO_SETTING, value);
    if let Some(mut r) = world.get_resource_mut::<recording::StravaRec>() {
        r.0.auto = v;
    }
    let msg = format!(
        "Strava: auto_upload = {v} of strava.toml is now the setting {AUTO_SETTING} = {value} (Navi > Strava > Automatic upload); the line in strava.toml is no longer read"
    );
    info!("{msg}");
    if let Some(mut c) = world.get_resource_mut::<ConsoleState>() {
        c.print(msg);
    }
}

/// `strava.auto_upload on|off` (the setting's apply command, spec 2.8.1
/// Q6): the worker, the recording's notice and the ride pages follow.
fn set_auto(world: &mut World, on: bool) {
    {
        let mut s = world.resource_mut::<Strava>();
        s.auto = on;
        s.auto_set = true;
        if let Some(w) = &s.worker {
            w.send(Cmd::SetAuto(on));
        }
    }
    if let Some(mut r) = world.get_resource_mut::<recording::StravaRec>() {
        r.0.auto = on;
    }
    // Typed as the command, not as `set`: kept too.
    let saved = world
        .get_resource::<ocw_ui::settings::UserSettings>()
        .is_some_and(|us| us.on(AUTO_SETTING) == on);
    if !saved {
        ocw_ui::settings::save_setting(world, AUTO_SETTING, if on { "on" } else { "off" });
    }
    refresh_history(world);
}

/// Starts the worker with the credentials from `strava.toml` or `env`;
/// `Err`: why Strava stays off.
fn start(
    config_dir: &Path,
    rides_dir: PathBuf,
    env: &dyn Fn(&str) -> Option<String>,
    fake: bool,
    auto_upload: bool,
) -> Result<Worker, String> {
    let config_file = config_dir.join("strava.toml");
    let config = StravaConfig::load(&config_file).map_err(|e| format!("Strava: off, {e}"))?;
    let Some(creds) = config.credentials(env) else {
        return Err(format!(
            "Strava: no API application set up (the advanced way). Upload your Strava recordings (S … S) in your browser instead: \
             Navi > Training history > the ride > Upload to Strava, or strava.upload-manual N. \
             With the automatic upload on they wait and are uploaded once it is set up. \
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
        auto_upload,
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
    let ok = s.try_start();
    save_migrated(world);
    if ok {
        started(world);
    }
}

/// Starts the worker now if the credentials came (the commands look at
/// once, not after [`WATCH_S`]).
fn start_if_set_up(world: &mut World) {
    let ok = world.resource_mut::<Strava>().try_start();
    save_migrated(world);
    if ok {
        started(world);
    }
}

/// The worker started while the app runs: the recording knows, the
/// history shows the API's rows.
fn started(world: &mut World) {
    let msg = if world.resource::<Strava>().auto {
        "Strava set up: the API credentials are there; waiting recordings are uploaded once the account is linked (strava.connect)"
    } else {
        "Strava set up: the API credentials are there; the automatic upload is off, upload your Strava recordings from the ride's page (strava.connect links the account)"
    };
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

/// Each ride's Strava state (its Strava recordings'; a ride without one
/// only when its whole file was uploaded before v5.00 – whole rides never
/// go to Strava now, spec 2.8.1 Q6) and the open question, for the
/// training history's pages.
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
        if recs.is_empty() {
            if let Some(s @ StravaState::OnStrava { .. }) = r
                .fit
                .as_deref()
                .and_then(|f| file_state(&state, f, uploading))
            {
                out.insert(r.id.clone(), s);
            }
            continue;
        }
        let states: Vec<Option<StravaState>> = recs
            .iter()
            .map(|(f, _)| file_state(&state, f, uploading))
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

/// The sync names of ride `n` (1 = the newest) to upload by hand: its
/// Strava recordings not on Strava yet (`waiting`: only the waiting ones).
fn recordings_of_ride(world: &World, n: usize, waiting: bool) -> Result<Vec<String>, String> {
    let ride = ocw_ui::history::nth(world, n)?;
    let rides = world
        .get_resource::<crate::recording::Recording>()
        .and_then(|r| r.dir.clone())
        .ok_or("no rides folder")?;
    let state = SyncState::load(&state_file(&rides))?;
    let marks = sync::ManualMarks::load(&rides);
    let all = manual::recordings_within(&rides, &ride);
    if all.is_empty() {
        return Err(format!(
            "ride {n} has no Strava recording (whole rides never go to Strava: press S while riding to record one)"
        ));
    }
    let files: Vec<String> = all
        .into_iter()
        .map(|(f, _)| f)
        .filter(|f| !marks.files.contains(f))
        .filter(|f| match state.rides.get(f) {
            Some(RideSync::Waiting { .. }) => true,
            Some(RideSync::Uploaded { .. } | RideSync::Rejected { .. }) => false,
            _ => !waiting,
        })
        .collect();
    if files.is_empty() {
        return Err(if waiting {
            format!("ride {n} has no waiting Strava recording")
        } else {
            format!("ride {n} has no Strava recording to upload (on Strava already, or marked as uploaded)")
        });
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
            .about("Upload every Strava recording (S … S) not on Strava yet, in the background; whole rides never go to Strava")
            .arg(
                Arg::new("retry")
                    .long("retry")
                    .action(ArgAction::SetTrue)
                    .help("Also retry failed and rejected recordings now"),
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
                    .help("Only the Strava recordings of this ride (1 = the newest) not on Strava yet – the ride page's Upload now; with --waiting only its waiting ones"),
            ),
        |world, m| {
            let waiting = m.get_flag("waiting");
            if waiting || m.get_one::<usize>("ride").is_some() {
                let files = match m.get_one::<usize>("ride") {
                    Some(&n) => Some(recordings_of_ride(world, n, waiting)?),
                    None => None,
                };
                let what = files.as_ref().map_or_else(
                    || "the waiting recordings".to_string(),
                    |f| format!("{} Strava recording{}", f.len(), if f.len() == 1 { "" } else { "s" }),
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
        Command::new("strava.auto_upload")
            .about("Upload the Strava recordings (S … S) automatically (on, the default) or wait for you to upload them from the ride's page (off); whole rides never go to Strava. The setting strava.auto_upload")
            .arg(Arg::new("state").value_parser(["on", "off"])),
        |world, m| {
            if let Some(v) = m.get_one::<String>("state") {
                set_auto(world, v == "on");
            }
            Ok(world.resource::<Strava>().auto_line())
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
        app_with(dir, None)
    }

    /// … with these user settings (`settings.toml`'s text, not saved).
    fn app_with(dir: &Path, settings: Option<&str>) -> App {
        let mut app = App::new();
        if let Some(text) = settings {
            let (s, _) = ocw_client::settings::Settings::parse(text).unwrap();
            app.insert_resource(ocw_ui::settings::UserSettings {
                settings: s,
                store: None,
            });
        }
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
                // Generous: the workspace's tests run in parallel.
                start.elapsed() < std::time::Duration::from_secs(30),
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
    /// hand; the HUD's notice counts them; a whole ride is never sent
    /// (spec 2.8.1 Q6).
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
        // A whole ride without S: never waiting, never uploaded.
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
            "client_id = \"12345\"\nclient_secret = \"made-up\"\n",
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
            "the whole ride is never uploaded"
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

    /// Linked credentials (made-up) and `strava.toml` with `extra`.
    fn set_up(dir: &Path, extra: &str) {
        let config = dir.join("config");
        TokenStore {
            path: config.join("strava-token.toml"),
        }
        .save(&auth::Token {
            access_token: "a".into(),
            refresh_token: "r".into(),
            expires_at: i64::MAX / 2,
            athlete: Some("Cleo Sprinter".into()),
        })
        .unwrap();
        std::fs::write(
            config.join("strava.toml"),
            format!("client_id = \"12345\"\nclient_secret = \"made-up\"\n{extra}"),
        )
        .unwrap();
    }

    fn worker_status(app: &App) -> worker::Status {
        app.world()
            .resource::<Strava>()
            .worker
            .as_ref()
            .unwrap()
            .status()
    }

    /// The old `auto_upload` of `strava.toml` (spec 2.8.1 Q6): read once
    /// into the setting `strava.auto_upload` while that has no value of
    /// its own (said in the console); then the setting rules – a later
    /// `auto_upload = true` there changes nothing. Off, a recording waits
    /// (not sent); `strava.auto_upload on` sends it.
    #[test]
    fn the_old_strava_toml_key_is_read_once_into_the_setting() {
        let dir = temp("migrate");
        set_up(&dir, "auto_upload = false\n");
        let mut app = app_with(&dir, Some(""));
        let us = app.world().resource::<ocw_ui::settings::UserSettings>();
        assert_eq!(
            us.settings.file_value(AUTO_SETTING),
            Some(ocw_client::settings::Value::Bool(false)),
            "now the setting"
        );
        assert!(!app.world().resource::<Strava>().auto());
        assert!(!app.world().resource::<recording::StravaRec>().0.auto);
        assert!(
            app.world()
                .resource::<ConsoleState>()
                .log()
                .iter()
                .any(|l| l.contains("is now the setting strava.auto_upload = off")),
            "{:?}",
            app.world().resource::<ConsoleState>().log()
        );
        wait_until(&mut app, "linked", |a| worker_status(a).connected);
        assert!(!worker_status(&app).auto_upload);
        // A recording with the automatic upload off: it waits for the rider.
        let said = record(&mut app, 1_794_000_000);
        assert!(said.contains(ocw_client::strava::WAITS_FOR_UPLOAD), "{said}");
        for _ in 0..10 {
            app.update();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(
            !state(&dir)
                .rides
                .values()
                .any(|s| matches!(s, RideSync::Uploaded { .. })),
            "not sent"
        );
        let status = run(&mut app, "strava.status");
        assert!(status.contains("automatic upload: off"), "{status}");
        assert!(status.contains("never whole rides"), "{status}");
        // Switched on in the console: it goes up, the setting follows.
        assert!(run(&mut app, "strava.auto_upload on").contains("automatic upload: on"));
        wait_until(&mut app, "uploaded", |_| {
            state(&dir)
                .rides
                .values()
                .any(|s| matches!(s, RideSync::Uploaded { .. }))
        });
        let text = app
            .world()
            .resource::<ocw_ui::settings::UserSettings>()
            .settings
            .to_toml();
        assert!(app.world().resource::<ocw_ui::settings::UserSettings>().on(AUTO_SETTING));
        drop(app);
        // A restart: the setting rules, the old key is not read again.
        set_up(&dir, "auto_upload = false\n");
        let again = app_with(&dir, Some(&text));
        assert!(again.world().resource::<Strava>().auto());
        assert!(
            !again
                .world()
                .resource::<ConsoleState>()
                .log()
                .iter()
                .any(|l| l.contains("is now the setting")),
            "read once"
        );
        drop(again);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The setting off (spec 2.8.1 Q6) and a ride with a Strava
    /// recording made before the set-up: the credentials come, nothing is
    /// uploaded by itself (no question either); the ride page's *upload
    /// now* (`strava.sync --waiting --ride 1`) sends it. A ride without a
    /// recording has no state, `strava.sync --ride` refuses it.
    #[test]
    fn with_the_setting_off_a_ride_page_uploads_its_recording() {
        let dir = temp("off");
        let settings = "[strava]\nauto_upload = false\n";
        let mut app = app_with(&dir, Some(settings));
        app.init_resource::<ocw_ui::history::TrainingHistory>();
        record(&mut app, 1_794_000_100);
        let rides = dir.join("rides");
        let (rec_name, _) = manual::recordings(&rides)[0].clone();
        let a = ocw_fit::decode(&std::fs::read(rides.join(&rec_name)).unwrap()).unwrap();
        // The ride around it (from the recording's own records), and an
        // older ride without any.
        let mut ride = a.clone();
        ride.start_unix_s -= 1;
        {
            let mut h = app
                .world_mut()
                .resource_mut::<ocw_ui::history::TrainingHistory>();
            for (id, act) in [("old", {
                let mut o = a.clone();
                o.start_unix_s -= 86_400;
                o
            }), ("ride", ride)] {
                h.history.add(ocw_client::history::RideEntry::from_activity(
                    &act,
                    ocw_client::history::RideContext {
                        id: id.into(),
                        fit: Some(format!("{id}.fit")),
                        ..Default::default()
                    },
                ));
            }
        }
        assert_eq!(history_states(app.world()).0.get("ride"), Some(&StravaState::Waiting));
        assert_eq!(history_states(app.world()).0.get("old"), None);
        set_up(&dir, "");
        wait_until(&mut app, "the worker starts", |a| {
            a.world().resource::<Strava>().api()
        });
        wait_until(&mut app, "linked", |a| worker_status(a).connected);
        for _ in 0..10 {
            app.update();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(worker_status(&app).question, None);
        assert!(matches!(state(&dir).rides[&rec_name], RideSync::Waiting { .. }));
        let err = run(&mut app, "strava.sync --ride 2");
        assert!(err.contains("no Strava recording"), "{err}");
        let out = run(&mut app, "strava.sync --waiting --ride 1");
        assert!(out.contains("uploading 1 Strava recording"), "{out}");
        wait_until(&mut app, "uploaded", |a| {
            matches!(
                history_states(a.world()).0.get("ride"),
                Some(StravaState::OnStrava { .. })
            )
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
