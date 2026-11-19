//! The Strava background thread: linking, uploads and retries run here, so
//! the game never waits for the network. The app sends [`Command`]s and
//! reads the shared [`Status`]; messages for the console come back on a
//! channel, [`Event`]s for the HUD and the menu on another.
//!
//! **Waiting recordings** (spec §2.8.1 *Recording without Strava set up,
//! uploaded later*): when the thread starts with the account linked, or
//! `strava.connect` links it, the Strava recordings made while Strava was
//! not set up are uploaded, oldest first ([`Scope::Waiting`]) – after
//! asking once ([`Event::Ask`]) when there are more than [`ASK_ABOVE`];
//! [`Event::CaughtUp`] says how many went up.
//!
//! **Only Strava recordings, automatically or by hand** (spec 2.8.1 Q6):
//! the sync works on the S recordings only, never on whole rides
//! ([`sync::sync_files`]). With the automatic upload on (setting
//! `strava.auto_upload`, [`Command::SetAuto`]) new recordings go up as
//! they are saved and the waiting ones once the account is linked; off,
//! nothing goes up by itself – the rider uploads (`strava.sync`, a ride
//! page's *Upload now*: [`Command::SyncWaiting`] with its files).

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::api::Http;
use super::auth::{self, Credentials};
use super::sync::{self, Scope, SyncCounts, SyncPaths, SyncState};

/// How long `strava.connect` waits for the browser.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(300);
/// Re-check unfinished uploads this often.
const PROCESSING_RECHECK: Duration = Duration::from_secs(60);
/// More waiting recordings than this: ask before uploading them.
pub const ASK_ABOVE: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Link the Strava account (browser login).
    Connect,
    /// Upload all Strava recordings not on Strava yet; `retry` also
    /// retries failed and rejected ones now.
    Sync { retry: bool },
    /// A ride or a Strava recording was saved (synced if the automatic
    /// upload is on).
    RideSaved,
    /// The automatic upload switched on or off (setting
    /// `strava.auto_upload`); on: what waits goes up now.
    SetAuto(bool),
    /// Forget the tokens.
    Disconnect,
    /// Upload the waiting recordings now (`files`: only these, picked by
    /// the rider – waiting or not yet uploaded; `None`: all waiting) – the
    /// answer *Yes*, `strava.sync --waiting`, a ride page's *Upload now*.
    SyncWaiting { files: Option<Vec<String>> },
    /// The answer *No* (or *Pick*): the waiting recordings stay waiting.
    KeepWaiting,
}

/// What the HUD and the menu show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// More than [`ASK_ABOVE`] recordings are waiting: ask once.
    Ask(usize),
    /// Waiting recordings went up (`uploaded`; `failed` will be retried).
    CaughtUp { uploaded: usize, failed: usize },
}

/// The notice after the catch-up: "3 recordings uploaded to Strava".
pub fn caught_up_text(uploaded: usize, failed: usize) -> String {
    let n = |k: usize| if k == 1 { "recording" } else { "recordings" };
    let mut s = format!("{uploaded} {} uploaded to Strava", n(uploaded));
    if failed > 0 {
        s.push_str(&format!("; {failed} {} failed, retried later", n(failed)));
    }
    s
}

/// What `strava.status` shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Status {
    pub connected: bool,
    pub athlete: Option<String>,
    pub auto_upload: bool,
    /// What the thread is doing right now.
    pub busy: Option<String>,
    /// Result of the last action.
    pub last: Option<String>,
    pub counts: SyncCounts,
    /// The ride file being uploaded now (the ride page's *uploading*).
    pub uploading: Option<String>,
    /// The open question: this many waiting recordings to upload?
    pub question: Option<usize>,
}

pub struct WorkerConfig {
    pub creds: Credentials,
    pub paths: SyncPaths,
    pub auto_upload: bool,
    pub http: Arc<dyn Http>,
    /// Opens the authorization page (the browser; a no-op in tests).
    pub open_url: fn(&str) -> Result<(), String>,
    /// Waits between upload status polls.
    pub sleep: fn(Duration),
}

pub struct Worker {
    tx: Sender<Command>,
    pub status: Arc<Mutex<Status>>,
    /// Lines for the console.
    pub messages: Mutex<Receiver<String>>,
    /// For the HUD and the menu.
    pub events: Mutex<Receiver<Event>>,
}

impl Worker {
    pub fn send(&self, cmd: Command) {
        // A send error means the thread is gone; nothing to do then.
        let _ = self.tx.send(cmd);
    }

    pub fn status(&self) -> Status {
        self.status.lock().expect("not poisoned").clone()
    }

    pub fn spawn(cfg: WorkerConfig) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let (msg_tx, msg_rx) = mpsc::channel();
        let (ev_tx, ev_rx) = mpsc::channel();
        let auto = cfg.auto_upload;
        let status = Arc::new(Mutex::new(Status {
            auto_upload: cfg.auto_upload,
            ..Status::default()
        }));
        let shared = status.clone();
        std::thread::Builder::new()
            .name("ocw-strava".into())
            .spawn(move || {
                Thread {
                    cfg,
                    status: shared,
                    messages: msg_tx,
                    events: ev_tx,
                    asked: std::cell::Cell::new(false),
                    auto: std::cell::Cell::new(auto),
                }
                .run(rx)
            })?;
        Ok(Self {
            tx,
            status,
            messages: Mutex::new(msg_rx),
            events: Mutex::new(ev_rx),
        })
    }
}

struct Thread {
    cfg: WorkerConfig,
    status: Arc<Mutex<Status>>,
    messages: Sender<String>,
    events: Sender<Event>,
    /// The question was asked (once per run of the app).
    asked: std::cell::Cell<bool>,
    /// The automatic upload (setting `strava.auto_upload`).
    auto: std::cell::Cell<bool>,
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

impl Thread {
    fn run(self, rx: Receiver<Command>) {
        self.refresh_status();
        // With the automatic upload: the recordings waiting for the
        // set-up first (the credentials just came, or the app starts with
        // them), then the others. Off: nothing goes by itself.
        let mut next_sync = if self.auto.get() && self.connected() {
            self.catch_up();
            Some(Instant::now())
        } else {
            None
        };
        loop {
            let timeout = next_sync.map_or(Duration::from_secs(3600), |t| {
                t.saturating_duration_since(Instant::now())
            });
            let cmd = match rx.recv_timeout(timeout) {
                Ok(c) => c,
                Err(RecvTimeoutError::Timeout) => {
                    if next_sync.is_some_and(|t| t <= Instant::now()) {
                        next_sync = self.sync(false, &Scope::Rides);
                    }
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => return,
            };
            match cmd {
                Command::Connect => {
                    if self.connect() && self.auto.get() {
                        self.catch_up();
                        next_sync = self.sync(false, &Scope::Rides);
                    }
                }
                Command::Sync { retry } => {
                    if retry {
                        self.retry_all();
                    }
                    next_sync = self.sync(true, &Scope::Rides);
                }
                Command::SetAuto(on) => {
                    let was = self.auto.replace(on);
                    self.status.lock().expect("not poisoned").auto_upload = on;
                    if on && !was && self.connected() {
                        self.catch_up();
                        next_sync = self.sync(false, &Scope::Rides);
                    } else if !on {
                        next_sync = None;
                        self.refresh_status();
                    }
                }
                Command::RideSaved => {
                    if self.auto.get() {
                        next_sync = self.sync(false, &Scope::Rides);
                    } else {
                        self.refresh_status();
                    }
                }
                Command::SyncWaiting { files } => {
                    self.status.lock().expect("not poisoned").question = None;
                    let scope = files.map_or(Scope::Waiting, Scope::Files);
                    next_sync = self.sync(true, &scope);
                }
                Command::KeepWaiting => {
                    let n = {
                        let mut s = self.status.lock().expect("not poisoned");
                        s.question = None;
                        s.counts.waiting
                    };
                    self.asked.set(true);
                    self.say(format!(
                        "{n} recording{} stay waiting (strava.sync --waiting uploads them)",
                        if n == 1 { "" } else { "s" }
                    ));
                }
                Command::Disconnect => {
                    let text = match self.cfg.paths.token.delete() {
                        Ok(()) => "Strava disconnected (tokens deleted)".to_string(),
                        Err(e) => format!("cannot delete the Strava tokens: {e}"),
                    };
                    self.say(text);
                    next_sync = None;
                    self.refresh_status();
                }
            }
        }
    }

    fn say(&self, text: impl Into<String>) {
        let text = text.into();
        tracing::info!("{text}");
        self.status.lock().expect("not poisoned").last = Some(text.clone());
        let _ = self.messages.send(text);
    }

    fn set_busy(&self, busy: Option<&str>) {
        self.status.lock().expect("not poisoned").busy = busy.map(String::from);
    }

    fn connected(&self) -> bool {
        matches!(self.cfg.paths.token.load(), Ok(Some(_)))
    }

    fn refresh_status(&self) {
        let token = self.cfg.paths.token.load().ok().flatten();
        let state = SyncState::load(&self.cfg.paths.state_file).unwrap_or_default();
        let counts = state.counts(&sync::sync_files(&self.cfg.paths.rides_dir));
        let mut s = self.status.lock().expect("not poisoned");
        s.connected = token.is_some();
        s.athlete = token.and_then(|t| t.athlete);
        s.counts = counts;
    }

    /// The recordings waiting for the set-up (the account is linked now):
    /// up to [`ASK_ABOVE`] go at once, more are asked about once.
    fn catch_up(&self) -> Option<Instant> {
        self.refresh_status();
        let n = self.status().counts.waiting;
        if n == 0 {
            return None;
        }
        if n > ASK_ABOVE {
            if !self.asked.replace(true) {
                self.status.lock().expect("not poisoned").question = Some(n);
                let _ = self.events.send(Event::Ask(n));
                self.say(format!(
                    "{n} recordings made before Strava was set up are waiting: upload them? (Yes: strava.sync --waiting)"
                ));
            }
            return None;
        }
        self.sync(true, &Scope::Waiting)
    }

    fn retry_all(&self) {
        let path = &self.cfg.paths.state_file;
        if let Ok(mut state) = SyncState::load(path) {
            state.retry_all();
            if let Err(e) = state.save(path) {
                tracing::warn!("{e}");
            }
        }
    }

    /// The browser login; returns whether the account is linked now.
    fn connect(&self) -> bool {
        self.set_busy(Some("waiting for the Strava login in the browser"));
        let result = (|| {
            let (listener, redirect) = auth::listen().map_err(|e| e.to_string())?;
            let state = auth::random_state();
            let url = auth::authorize_url(&self.cfg.creds.client_id, &redirect, &state);
            if let Err(e) = (self.cfg.open_url)(&url) {
                tracing::warn!("{e}");
            }
            // The URL holds no secret (client id and a one-time state only).
            self.say(format!(
                "Log in to Strava in your browser to link it: {url}"
            ));
            let code = auth::wait_for_callback(&listener, &state, CONNECT_TIMEOUT)?;
            self.set_busy(Some("linking Strava"));
            auth::exchange_code(
                &*self.cfg.http,
                &self.cfg.creds,
                &self.cfg.paths.token,
                &code,
            )
            .map_err(|e| e.to_string())
        })();
        self.set_busy(None);
        let linked = match result {
            Ok(token) => {
                self.say(format!(
                    "Strava linked{}",
                    token
                        .athlete
                        .map(|a| format!(" for {a}"))
                        .unwrap_or_default()
                ));
                true
            }
            Err(e) => {
                self.say(format!("Strava not linked: {e}"));
                false
            }
        };
        self.refresh_status();
        linked
    }

    /// One sync run; returns when to run again (retries, unfinished uploads).
    fn sync(&self, manual: bool, scope: &Scope) -> Option<Instant> {
        self.set_busy(Some("uploading rides to Strava"));
        // The waiting recordings before the run: the notice counts them.
        let waiting: Vec<String> = SyncState::load(&self.cfg.paths.state_file)
            .map(|s| {
                s.waiting(&sync::sync_files(&self.cfg.paths.rides_dir))
                    .into_iter()
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        let result = sync::sync_once(
            &*self.cfg.http,
            &self.cfg.creds,
            &self.cfg.paths,
            scope,
            &unix_now,
            &self.cfg.sleep,
            &|file| self.status.lock().expect("not poisoned").uploading = Some(file.to_string()),
        );
        self.status.lock().expect("not poisoned").uploading = None;
        self.set_busy(None);
        self.refresh_status();
        if let Ok(r) = &result {
            let up = r
                .uploaded
                .iter()
                .filter(|(f, _)| waiting.contains(f))
                .count();
            let failed = r.failed.iter().filter(|(f, _)| waiting.contains(f)).count();
            if up > 0 {
                let _ = self.events.send(Event::CaughtUp {
                    uploaded: up,
                    failed,
                });
                self.say(caught_up_text(up, failed));
            }
        }
        let counts = self.status().counts;
        match result {
            Ok(r) => {
                for (file, id) in &r.uploaded {
                    self.say(match id {
                        Some(id) => format!(
                            "uploaded {file} to Strava: https://www.strava.com/activities/{id}"
                        ),
                        None => format!("{file} is already on Strava"),
                    });
                }
                for (file, e) in &r.rejected {
                    self.say(format!("Strava rejected {file}: {e}"));
                }
                for (file, e) in &r.failed {
                    self.say(format!("uploading {file} failed, will retry: {e}"));
                }
                if r.needs_connect {
                    self.say("Strava is not linked (or access was revoked): run strava.connect");
                    return None;
                }
                if manual && r == sync::SyncReport::default() {
                    self.say(if *scope == Scope::Rides {
                        "Strava: no Strava recording to upload"
                    } else {
                        "Strava: no recording is waiting"
                    });
                }
            }
            Err(e) => self.say(format!("Strava sync failed: {e}")),
        }
        if !self.auto.get() && !manual {
            return None;
        }
        let retry = counts
            .next_retry
            .map(|at| Instant::now() + Duration::from_secs((at - unix_now()).max(1) as u64));
        let processing = (counts.pending > 0).then(|| Instant::now() + PROCESSING_RECHECK);
        match (retry, processing) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    fn status(&self) -> Status {
        self.status.lock().expect("not poisoned").clone()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::super::api::tests::MockHttp;
    use super::super::auth::{Token, TokenStore};
    use super::*;

    fn wait_for(w: &Worker, done: impl Fn(&Status) -> bool) -> Status {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            let s = w.status();
            if done(&s) {
                return s;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("timed out: {:?}", w.status());
    }

    #[test]
    fn saved_recordings_are_uploaded_in_the_background() {
        let dir: PathBuf = std::env::temp_dir().join(format!("ocw-worker-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let rides_dir = dir.join("rides");
        std::fs::create_dir_all(rides_dir.join("strava")).unwrap();
        let paths = SyncPaths {
            state_file: rides_dir.join("strava-sync.toml"),
            rides_dir: rides_dir.clone(),
            token: TokenStore {
                path: dir.join("strava-token.toml"),
            },
        };
        paths
            .token
            .save(&Token {
                access_token: "a".into(),
                refresh_token: "r".into(),
                expires_at: i64::MAX / 2,
                athlete: Some("Rider".into()),
            })
            .unwrap();
        let http = Arc::new(MockHttp::default());
        let w = Worker::spawn(WorkerConfig {
            creds: Credentials {
                client_id: "1".into(),
                client_secret: "s".into(),
            },
            paths,
            auto_upload: true,
            http: http.clone(),
            open_url: |_| Ok(()),
            sleep: |_| {},
        })
        .unwrap();
        let s = wait_for(&w, |s| s.connected);
        assert_eq!(s.athlete.as_deref(), Some("Rider"));
        assert_eq!(s.counts, SyncCounts::default());

        http.reply(201, r#"{"id":1,"error":null,"activity_id":77}"#);
        // A whole ride is never uploaded (spec 2.8.1 Q6), its Strava
        // recording is.
        std::fs::write(rides_dir.join("r1.fit"), b"FIT").unwrap();
        std::fs::write(rides_dir.join("strava/r1.fit"), b"FIT").unwrap();
        w.send(Command::RideSaved);
        let s = wait_for(&w, |s| s.counts.uploaded == 1);
        assert_eq!(s.counts.pending, 0);
        assert_eq!(http.urls().len(), 1);
        assert!(s.busy.is_none());
        let msg = w
            .messages
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(1));
        assert_eq!(
            msg.unwrap(),
            "uploaded strava/r1.fit to Strava: https://www.strava.com/activities/77"
        );

        w.send(Command::Disconnect);
        wait_for(&w, |s| !s.connected);
        drop(w);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn linked_paths(name: &str) -> (PathBuf, SyncPaths) {
        let dir: PathBuf =
            std::env::temp_dir().join(format!("ocw-worker-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let rides_dir = dir.join("rides");
        std::fs::create_dir_all(rides_dir.join("strava")).unwrap();
        let paths = SyncPaths {
            state_file: rides_dir.join("strava-sync.toml"),
            rides_dir,
            token: TokenStore {
                path: dir.join("strava-token.toml"),
            },
        };
        paths
            .token
            .save(&Token {
                access_token: "a".into(),
                refresh_token: "r".into(),
                expires_at: i64::MAX / 2,
                athlete: Some("Ada Rider".into()),
            })
            .unwrap();
        (dir, paths)
    }

    /// `n` waiting recordings (recorded without the set-up), the newest
    /// first on disk so the order is the sync's.
    fn waiting(paths: &SyncPaths, n: usize) -> Vec<String> {
        let names: Vec<String> = (1..=n)
            .rev()
            .map(|d| format!("strava/2026-11-{d:02}_10-00-00.fit"))
            .collect();
        for f in &names {
            std::fs::write(paths.rides_dir.join(f), b"FIT").unwrap();
        }
        sync::mark_waiting(&paths.state_file, &sync::sync_files(&paths.rides_dir), 1).unwrap();
        names
    }

    fn spawn(paths: SyncPaths, http: Arc<MockHttp>, auto_upload: bool) -> Worker {
        Worker::spawn(WorkerConfig {
            creds: Credentials {
                client_id: "1".into(),
                client_secret: "s".into(),
            },
            paths,
            auto_upload,
            http,
            open_url: |_| Ok(()),
            sleep: |_| {},
        })
        .unwrap()
    }

    fn uploaded_names(http: &MockHttp) -> Vec<String> {
        http.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.url == super::super::api::UPLOADS_URL)
            .map(|r| {
                let body = String::from_utf8_lossy(&r.body);
                let at = body.find("strava/").expect("a recording");
                body[at..at + 30].to_string()
            })
            .collect()
    }

    /// The credentials came (the thread starts) with up to three
    /// recordings waiting and the automatic upload on: they go at once,
    /// oldest first, and the notice counts them; the whole ride beside
    /// them is never queued (spec 2.8.1 Q6).
    #[test]
    fn a_few_waiting_recordings_go_up_at_once_oldest_first() {
        let (dir, paths) = linked_paths("few");
        let names = waiting(&paths, 3);
        std::fs::write(paths.rides_dir.join("2026-11-01_09-00-00.fit"), b"FIT").unwrap();
        let http = Arc::new(MockHttp::default());
        for id in 1..=3 {
            http.reply(
                201,
                &format!(r#"{{"id":{id},"error":null,"activity_id":{id}}}"#),
            );
        }
        let w = spawn(paths, http.clone(), true);
        let s = wait_for(&w, |s| s.counts.uploaded == 3);
        assert_eq!((s.counts.waiting, s.question), (0, None));
        let mut oldest_first = names.clone();
        oldest_first.reverse();
        assert_eq!(uploaded_names(&http), oldest_first);
        let ev = w
            .events
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(1));
        assert_eq!(
            ev.unwrap(),
            Event::CaughtUp {
                uploaded: 3,
                failed: 0
            }
        );
        assert_eq!(s.counts.pending, 0, "the whole ride is never queued");
        assert_eq!(uploaded_names(&http).len(), 3);
        drop(w);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// More than three waiting: asked once (no upload before the answer);
    /// *No* leaves them waiting; *Yes* (`strava.sync --waiting`) uploads
    /// them; a picked one goes alone.
    #[test]
    fn many_waiting_recordings_are_asked_about_first() {
        let (dir, paths) = linked_paths("many");
        let names = waiting(&paths, 5);
        let http = Arc::new(MockHttp::default());
        let w = spawn(paths, http.clone(), true);
        let s = wait_for(&w, |s| s.question.is_some());
        assert_eq!(s.question, Some(5));
        let ev = w
            .events
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(1));
        assert_eq!(ev.unwrap(), Event::Ask(5));
        assert!(http.urls().is_empty(), "nothing before the answer");
        // No.
        w.send(Command::KeepWaiting);
        let s = wait_for(&w, |s| s.question.is_none());
        assert_eq!(s.counts.waiting, 5);
        w.send(Command::Sync { retry: false });
        wait_for(&w, |s| {
            s.last
                .as_deref()
                .is_some_and(|l| l.contains("no Strava recording to upload"))
        });
        assert!(http.urls().is_empty(), "the usual sync leaves them");
        // Picked: one.
        http.reply(201, r#"{"id":9,"error":null,"activity_id":90}"#);
        w.send(Command::SyncWaiting {
            files: Some(vec![names[0].clone()]),
        });
        let s = wait_for(&w, |s| s.counts.uploaded == 1);
        assert_eq!(s.counts.waiting, 4);
        assert_eq!(
            w.events
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(1))
                .unwrap(),
            Event::CaughtUp {
                uploaded: 1,
                failed: 0
            }
        );
        // Yes: the rest, oldest first; one fails and is retried later.
        for id in 1..=3 {
            http.reply(
                201,
                &format!(r#"{{"id":{id},"error":null,"activity_id":{id}}}"#),
            );
        }
        http.reply(503, "");
        w.send(Command::SyncWaiting { files: None });
        let s = wait_for(&w, |s| s.counts.uploaded == 4);
        assert_eq!((s.counts.waiting, s.counts.failed), (0, 1));
        assert_eq!(
            w.events
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(1))
                .unwrap(),
            Event::CaughtUp {
                uploaded: 3,
                failed: 1
            }
        );
        let order = uploaded_names(&http);
        assert_eq!(order[0], names[0]);
        assert_eq!(
            order[1..].to_vec(),
            vec![
                names[4].clone(),
                names[3].clone(),
                names[2].clone(),
                names[1].clone()
            ]
        );
        drop(w);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The automatic upload off (setting `strava.auto_upload`, spec 2.8.1
    /// Q6): nothing goes up by itself – not the waiting recordings when
    /// the account is linked, not a new one when it is saved; the rider's
    /// upload of one ride's recordings sends those (waiting or new);
    /// switched on, the rest go up at once.
    #[test]
    fn with_the_automatic_upload_off_recordings_wait_for_the_rider() {
        let (dir, paths) = linked_paths("off");
        let names = waiting(&paths, 2);
        let fresh = "strava/2026-11-20_10-00-00.fit".to_string();
        std::fs::write(paths.rides_dir.join(&fresh), b"FIT").unwrap();
        let http = Arc::new(MockHttp::default());
        let w = spawn(paths, http.clone(), false);
        let s = wait_for(&w, |s| s.connected && s.counts.waiting == 2);
        assert!(!s.auto_upload);
        assert_eq!((s.counts.pending, s.question), (1, None));
        w.send(Command::RideSaved);
        std::thread::sleep(Duration::from_millis(100));
        assert!(http.urls().is_empty(), "nothing by itself");
        // The ride page's Upload now: the oldest waiting one and the new one.
        for id in 1..=2 {
            http.reply(
                201,
                &format!(r#"{{"id":{id},"error":null,"activity_id":{id}}}"#),
            );
        }
        w.send(Command::SyncWaiting {
            files: Some(vec![names[1].clone(), fresh.clone()]),
        });
        let s = wait_for(&w, |s| s.counts.uploaded == 2);
        assert_eq!((s.counts.waiting, s.counts.pending), (1, 0));
        assert_eq!(uploaded_names(&http), vec![names[1].clone(), fresh]);
        // Switched on: the last waiting one goes at once.
        http.reply(201, r#"{"id":3,"error":null,"activity_id":3}"#);
        w.send(Command::SetAuto(true));
        let s = wait_for(&w, |s| s.counts.uploaded == 3);
        assert!(s.auto_upload);
        assert_eq!(s.counts.waiting, 0);
        drop(w);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_catch_up_notice() {
        assert_eq!(caught_up_text(3, 0), "3 recordings uploaded to Strava");
        assert_eq!(caught_up_text(1, 0), "1 recording uploaded to Strava");
        assert_eq!(
            caught_up_text(2, 1),
            "2 recordings uploaded to Strava; 1 recording failed, retried later"
        );
    }
}
