//! The Strava background thread: linking, uploads and retries run here, so
//! the game never waits for the network. The app sends [`Command`]s and
//! reads the shared [`Status`]; messages for the console come back on a
//! channel.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::api::Http;
use super::auth::{self, Credentials};
use super::sync::{self, SyncCounts, SyncPaths, SyncState};

/// How long `strava.connect` waits for the browser.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(300);
/// Re-check unfinished uploads this often.
const PROCESSING_RECHECK: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Link the Strava account (browser login).
    Connect,
    /// Upload all rides not on Strava yet; `retry` also retries failed and
    /// rejected ones now.
    Sync { retry: bool },
    /// A ride was saved (synced if automatic upload is on).
    RideSaved,
    /// Forget the tokens.
    Disconnect,
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
                }
                .run(rx)
            })?;
        Ok(Self {
            tx,
            status,
            messages: Mutex::new(msg_rx),
        })
    }
}

struct Thread {
    cfg: WorkerConfig,
    status: Arc<Mutex<Status>>,
    messages: Sender<String>,
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

impl Thread {
    fn run(self, rx: Receiver<Command>) {
        self.refresh_status();
        let mut next_sync = (self.cfg.auto_upload && self.connected()).then(Instant::now);
        loop {
            let timeout = next_sync.map_or(Duration::from_secs(3600), |t| {
                t.saturating_duration_since(Instant::now())
            });
            let cmd = match rx.recv_timeout(timeout) {
                Ok(c) => c,
                Err(RecvTimeoutError::Timeout) => {
                    if next_sync.is_some_and(|t| t <= Instant::now()) {
                        next_sync = self.sync(false);
                    }
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => return,
            };
            match cmd {
                Command::Connect => {
                    if self.connect() && self.cfg.auto_upload {
                        next_sync = self.sync(false);
                    }
                }
                Command::Sync { retry } => {
                    if retry {
                        self.retry_all();
                    }
                    next_sync = self.sync(true);
                }
                Command::RideSaved => {
                    if self.cfg.auto_upload {
                        next_sync = self.sync(false);
                    } else {
                        self.refresh_status();
                    }
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
        let counts = state.counts(&sync::ride_files(&self.cfg.paths.rides_dir));
        let mut s = self.status.lock().expect("not poisoned");
        s.connected = token.is_some();
        s.athlete = token.and_then(|t| t.athlete);
        s.counts = counts;
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
    fn sync(&self, manual: bool) -> Option<Instant> {
        self.set_busy(Some("uploading rides to Strava"));
        let result = sync::sync_once(
            &*self.cfg.http,
            &self.cfg.creds,
            &self.cfg.paths,
            &unix_now,
            &self.cfg.sleep,
        );
        self.set_busy(None);
        self.refresh_status();
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
                    self.say("Strava: nothing to upload");
                }
            }
            Err(e) => self.say(format!("Strava sync failed: {e}")),
        }
        if !self.cfg.auto_upload && !manual {
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
    fn saved_rides_are_uploaded_in_the_background() {
        let dir: PathBuf = std::env::temp_dir().join(format!("ocw-worker-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let rides_dir = dir.join("rides");
        std::fs::create_dir_all(&rides_dir).unwrap();
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
        std::fs::write(rides_dir.join("r1.fit"), b"FIT").unwrap();
        w.send(Command::RideSaved);
        let s = wait_for(&w, |s| s.counts.uploaded == 1);
        assert!(s.busy.is_none());
        let msg = w
            .messages
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(1));
        assert_eq!(
            msg.unwrap(),
            "uploaded r1.fit to Strava: https://www.strava.com/activities/77"
        );

        w.send(Command::Disconnect);
        wait_for(&w, |s| !s.connected);
        drop(w);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
