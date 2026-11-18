//! Strava sync: every saved ride that is not on Strava yet gets uploaded.
//! The state (`<rides dir>/strava-sync.toml`) remembers what is uploaded,
//! what failed (and when to retry, with exponential backoff) and what
//! Strava rejected for good, and which Strava recordings wait for the
//! Strava set-up (spec §2.8.1 *Recording without Strava set up, uploaded
//! later*: [`RideSync::Waiting`], uploaded by [`Scope::Waiting`] once the
//! credentials are there).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::api::{
    self, ApiError, Http, UploadMeta, UploadStatus, parse_upload, upload_request,
    upload_status_request,
};
use super::auth::{Credentials, TokenStore, valid_token};

/// First retry delay; doubles per failed attempt up to [`MAX_BACKOFF_S`].
pub const BASE_BACKOFF_S: i64 = 60;
pub const MAX_BACKOFF_S: i64 = 6 * 3600;
/// Status polls per upload before resuming on the next sync.
pub const MAX_POLLS: u32 = 30;

/// Sync state of one ride (keyed by its file name).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RideSync {
    /// A Strava recording made while Strava was not set up (no API
    /// credentials): kept locally, uploaded once they are provided – not
    /// failed, not uploaded. Only [`Scope::Waiting`] and [`Scope::Files`]
    /// upload it.
    Waiting { since: i64 },
    /// Sent; Strava is still processing it.
    Processing { upload_id: u64 },
    Uploaded {
        #[serde(default)]
        activity_id: Option<u64>,
        at: i64,
    },
    /// Failed for a reason that may go away; retried at `retry_at`.
    Failed {
        attempts: u32,
        error: String,
        retry_at: i64,
    },
    /// Strava rejected the ride; not retried.
    Rejected { error: String },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncState {
    #[serde(default)]
    pub rides: BTreeMap<String, RideSync>,
}

/// Counts for `strava.status`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncCounts {
    pub uploaded: usize,
    pub pending: usize,
    /// Strava recordings waiting for the set-up ([`RideSync::Waiting`]).
    pub waiting: usize,
    pub failed: usize,
    pub rejected: usize,
    /// Earliest retry time of a failed ride.
    pub next_retry: Option<i64>,
}

pub fn backoff_s(attempts: u32) -> i64 {
    let exp = attempts.saturating_sub(1).min(20);
    (BASE_BACKOFF_S << exp).min(MAX_BACKOFF_S)
}

impl SyncState {
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(s) => toml::from_str(&s).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        let tmp = path.with_extension("toml.part");
        std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// The rides of `files` to work on now, in order: new ones, unfinished
    /// uploads and failed ones whose retry time has come (not the waiting
    /// recordings: [`Self::due_in`]).
    #[cfg(test)]
    pub fn due<'a>(&self, files: &'a [String], now: i64) -> Vec<&'a String> {
        self.due_in(files, now, &Scope::Rides)
    }

    /// The rides of `files` to work on now within `scope`, oldest first
    /// (by the file name without its `strava/` folder: names start with
    /// the date and time).
    pub fn due_in<'a>(&self, files: &'a [String], now: i64, scope: &Scope) -> Vec<&'a String> {
        let mut out: Vec<&String> = files
            .iter()
            .filter(|f| match (scope, self.rides.get(*f)) {
                (Scope::Files(only), state) => {
                    only.contains(f)
                        && !matches!(
                            state,
                            Some(RideSync::Uploaded { .. } | RideSync::Rejected { .. })
                        )
                }
                (Scope::Waiting, state) => matches!(state, Some(RideSync::Waiting { .. })),
                (Scope::Rides, None | Some(RideSync::Processing { .. })) => true,
                (Scope::Rides, Some(RideSync::Failed { retry_at, .. })) => *retry_at <= now,
                (Scope::Rides, _) => false,
            })
            .collect();
        out.sort_by_key(|f| (base_name(f), f.as_str()));
        out
    }

    /// The waiting recordings among `files`, oldest first.
    pub fn waiting<'a>(&self, files: &'a [String]) -> Vec<&'a String> {
        self.due_in(files, 0, &Scope::Waiting)
    }

    /// Marks `file` as waiting for the Strava set-up (unless the sync
    /// knows it already); whether it was marked.
    pub fn mark_waiting(&mut self, file: &str, now: i64) -> bool {
        if self.rides.contains_key(file) {
            return false;
        }
        self.rides
            .insert(file.to_string(), RideSync::Waiting { since: now });
        true
    }

    pub fn counts(&self, files: &[String]) -> SyncCounts {
        let mut c = SyncCounts::default();
        for f in files {
            match self.rides.get(f) {
                None | Some(RideSync::Processing { .. }) => c.pending += 1,
                Some(RideSync::Waiting { .. }) => c.waiting += 1,
                Some(RideSync::Uploaded { .. }) => c.uploaded += 1,
                Some(RideSync::Failed { retry_at, .. }) => {
                    c.failed += 1;
                    c.next_retry = Some(c.next_retry.map_or(*retry_at, |t| t.min(*retry_at)));
                }
                Some(RideSync::Rejected { .. }) => c.rejected += 1,
            }
        }
        c
    }

    /// Records a failed attempt; returns when it will be retried.
    pub fn mark_failed(&mut self, file: &str, error: String, now: i64) -> i64 {
        let attempts = match self.rides.get(file) {
            Some(RideSync::Failed { attempts, .. }) => attempts + 1,
            _ => 1,
        };
        let retry_at = now + backoff_s(attempts);
        self.rides.insert(
            file.to_string(),
            RideSync::Failed {
                attempts,
                error,
                retry_at,
            },
        );
        retry_at
    }

    /// Makes failed and rejected rides due again (`strava.sync --retry`).
    pub fn retry_all(&mut self) {
        self.rides
            .retain(|_, s| !matches!(s, RideSync::Failed { .. } | RideSync::Rejected { .. }));
    }
}

/// Which rides a sync run works on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// The usual sync: every due ride, without the waiting recordings.
    Rides,
    /// Only the recordings waiting for the Strava set-up (the catch-up
    /// upload once the credentials are there).
    Waiting,
    /// These files (picked by the rider), whatever their state – except
    /// uploaded or rejected ones.
    Files(Vec<String>),
}

/// A sync file's name without its folder (`strava/a.fit` → `a.fit`).
fn base_name(file: &str) -> &str {
    file.rsplit('/').next().unwrap_or(file)
}

/// Marks the Strava recordings (`strava/…`) among `files` (as
/// [`sync_files`] gives them) that the sync does not know yet as waiting
/// for the Strava set-up (spec §2.8.1 *Recording without Strava set up*).
/// Writes the state file only when something changed; returns how many
/// were marked.
pub fn mark_waiting(state_file: &Path, files: &[String], now: i64) -> Result<usize, String> {
    let mut state = SyncState::load(state_file)?;
    let n = files
        .iter()
        .filter(|f| f.starts_with("strava/"))
        .filter(|f| state.mark_waiting(f, now))
        .count();
    if n > 0 {
        state.save(state_file)?;
    }
    Ok(n)
}

/// The `.fit` files in `dir` and the Strava recordings in `dir/strava/`
/// (spec §2.8.1: as `strava/<name>.fit`), sorted (oldest first: names
/// start with the date).
pub fn ride_files(dir: &Path) -> Vec<String> {
    let fits = |d: &Path| -> Vec<String> {
        std::fs::read_dir(d)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| n.ends_with(".fit"))
            .collect()
    };
    let mut files = fits(dir);
    files.extend(
        fits(&dir.join("strava"))
            .into_iter()
            .map(|n| format!("strava/{n}")),
    );
    files.sort();
    files
}

/// The ride files the rider marked as uploaded to Strava by hand (spec
/// §2.8.1 *Simple upload*: the app cannot know, the rider marks the ride
/// in the training history); `<rides dir>/strava-manual.toml`, written by
/// the app's main thread, only read by the sync, which skips them – so a
/// later API set-up uploads no duplicate.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManualMarks {
    /// File names as [`ride_files`] gives them (`strava/<name>.fit` for a
    /// Strava recording).
    #[serde(default)]
    pub files: std::collections::BTreeSet<String>,
}

impl ManualMarks {
    pub fn path(rides_dir: &Path) -> PathBuf {
        rides_dir.join("strava-manual.toml")
    }

    /// The marks (none if there is no file or it is broken).
    pub fn load(rides_dir: &Path) -> Self {
        std::fs::read_to_string(Self::path(rides_dir))
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, rides_dir: &Path) -> Result<(), String> {
        let path = Self::path(rides_dir);
        let text = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        let tmp = path.with_extension("toml.part");
        std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Marks (`on`) or unmarks `files`.
    pub fn set(&mut self, files: &[String], on: bool) {
        for f in files {
            if on {
                self.files.insert(f.clone());
            } else {
                self.files.remove(f);
            }
        }
    }
}

/// The ride files the API sync works on: [`ride_files`] without the ones
/// marked as uploaded by hand ([`ManualMarks`]).
pub fn sync_files(dir: &Path) -> Vec<String> {
    let marks = ManualMarks::load(dir);
    ride_files(dir)
        .into_iter()
        .filter(|f| !marks.files.contains(f))
        .collect()
}

/// Strava's `external_id` of a ride file (no path separators).
pub fn external_id(file: &str) -> String {
    file.replace('/', "-")
}

/// What to do after a status answer while polling an upload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PollAction {
    /// Ask again after this delay.
    Wait(Duration),
    Done(RideSync),
    /// Stop polling for now; the next sync resumes with the upload id.
    Later,
}

/// Status polling of one upload: Strava processes uploads asynchronously.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Poller {
    polls: u32,
}

impl Default for Poller {
    fn default() -> Self {
        Self::new()
    }
}

impl Poller {
    pub fn new() -> Self {
        Self { polls: 0 }
    }

    pub fn on_status(&mut self, status: &UploadStatus, now: i64) -> PollAction {
        match status {
            UploadStatus::Ready { activity_id } => PollAction::Done(RideSync::Uploaded {
                activity_id: Some(*activity_id),
                at: now,
            }),
            UploadStatus::Duplicate { activity_id } => PollAction::Done(RideSync::Uploaded {
                activity_id: *activity_id,
                at: now,
            }),
            UploadStatus::Failed { error } => PollAction::Done(RideSync::Rejected {
                error: error.clone(),
            }),
            UploadStatus::Processing { .. } => {
                self.polls += 1;
                if self.polls > MAX_POLLS {
                    PollAction::Later
                } else {
                    // Strava asks for no more than one poll per second; most
                    // uploads finish within ~10 s.
                    PollAction::Wait(Duration::from_secs(if self.polls < 5 { 2 } else { 5 }))
                }
            }
        }
    }
}

/// Where the sync reads and writes.
#[derive(Debug, Clone)]
pub struct SyncPaths {
    pub rides_dir: PathBuf,
    pub state_file: PathBuf,
    pub token: TokenStore,
}

/// Outcome of one sync run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub uploaded: Vec<(String, Option<u64>)>,
    pub failed: Vec<(String, String)>,
    pub rejected: Vec<(String, String)>,
    /// Uploads still processing on Strava.
    pub processing: usize,
    /// Not connected or the authorization was revoked.
    pub needs_connect: bool,
}

/// Uploads all due rides of `scope`, oldest first. `sleep` waits between
/// status polls (no-op in tests); `started` hears of each file before its
/// upload (the ride page's *uploading*). Saves the state after every
/// ride.
pub fn sync_once(
    http: &dyn Http,
    creds: &Credentials,
    paths: &SyncPaths,
    scope: &Scope,
    now: &dyn Fn() -> i64,
    sleep: &dyn Fn(Duration),
    started: &dyn Fn(&str),
) -> Result<SyncReport, String> {
    let mut report = SyncReport::default();
    let mut state = SyncState::load(&paths.state_file)?;
    let files = sync_files(&paths.rides_dir);
    let due: Vec<String> = state
        .due_in(&files, now(), scope)
        .into_iter()
        .cloned()
        .collect();
    if due.is_empty() {
        return Ok(report);
    }
    for file in due {
        started(&file);
        let token = match valid_token(http, creds, &paths.token, now()) {
            Ok(Some(t)) => t,
            Ok(None) | Err(ApiError::Unauthorized) => {
                report.needs_connect = true;
                break;
            }
            Err(e) => {
                // Token refresh failed (network): retry all later.
                let retry_at = state.mark_failed(&file, e.to_string(), now());
                tracing::warn!(%file, error = %e, retry_at, "Strava sync failed");
                report.failed.push((file, e.to_string()));
                break;
            }
        };
        let result = upload_one(http, &token.access_token, paths, &file, &state, now, sleep);
        match result {
            Ok(Some(done)) => {
                match &done {
                    RideSync::Uploaded { activity_id, .. } => {
                        tracing::info!(%file, ?activity_id, "ride uploaded to Strava");
                        report.uploaded.push((file.clone(), *activity_id));
                    }
                    RideSync::Rejected { error } => {
                        tracing::warn!(%file, %error, "Strava rejected the ride");
                        report.rejected.push((file.clone(), error.clone()));
                    }
                    RideSync::Processing { .. } => report.processing += 1,
                    RideSync::Failed { .. } | RideSync::Waiting { .. } => {}
                }
                state.rides.insert(file, done);
            }
            Ok(None) => {}
            Err(ApiError::Unauthorized) => {
                report.needs_connect = true;
                break;
            }
            Err(ApiError::Permanent(error)) => {
                tracing::warn!(%file, %error, "Strava rejected the upload");
                report.rejected.push((file.clone(), error.clone()));
                state.rides.insert(file, RideSync::Rejected { error });
            }
            Err(e) => {
                let retry_at = state.mark_failed(&file, e.to_string(), now());
                tracing::warn!(%file, error = %e, retry_at, "Strava upload failed, will retry");
                report.failed.push((file, e.to_string()));
                // Rate limit or network trouble: the other rides wait too.
                break;
            }
        }
        state.save(&paths.state_file)?;
    }
    state.save(&paths.state_file)?;
    Ok(report)
}

/// Uploads (or resumes polling) one ride. `Ok(None)`: the file vanished.
fn upload_one(
    http: &dyn Http,
    access_token: &str,
    paths: &SyncPaths,
    file: &str,
    state: &SyncState,
    now: &dyn Fn() -> i64,
    sleep: &dyn Fn(Duration),
) -> Result<Option<RideSync>, ApiError> {
    let upload_id = match state.rides.get(file) {
        Some(RideSync::Processing { upload_id }) => *upload_id,
        _ => {
            let path = paths.rides_dir.join(file);
            let Ok(fit) = std::fs::read(&path) else {
                return Ok(None);
            };
            let meta = crate::recording::load_meta(&path);
            let meta = UploadMeta {
                name: meta
                    .as_ref()
                    .map_or("Open Cycling World virtual ride".into(), |m| m.name.clone()),
                description: meta.map_or(
                    "Virtual ride in Open Cycling World. The GPS track is virtual.".into(),
                    |m| m.description,
                ),
                external_id: external_id(file),
            };
            let boundary = format!("ocw-{}", super::auth::random_state());
            let req = upload_request(access_token, file, &fit, &meta, &boundary);
            let first = parse_upload(&api::call(http, &req)?)?;
            match first {
                UploadStatus::Processing { upload_id } => upload_id,
                status => {
                    return Ok(match Poller::new().on_status(&status, now()) {
                        PollAction::Done(done) => Some(done),
                        _ => None,
                    });
                }
            }
        }
    };
    let mut poller = Poller::new();
    let mut status = UploadStatus::Processing { upload_id };
    loop {
        match poller.on_status(&status, now()) {
            PollAction::Done(done) => return Ok(Some(done)),
            PollAction::Later => return Ok(Some(RideSync::Processing { upload_id })),
            PollAction::Wait(d) => sleep(d),
        }
        let req = upload_status_request(access_token, upload_id);
        status = match api::call(http, &req) {
            Ok(resp) => parse_upload(&resp)?,
            // The upload exists; a network blip just postpones the result.
            Err(e) if e.is_retryable() => return Ok(Some(RideSync::Processing { upload_id })),
            Err(e) => return Err(e),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::super::api::tests::MockHttp;
    use super::super::auth::Token;
    use super::*;

    fn files(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn backoff_doubles_up_to_a_limit() {
        assert_eq!(backoff_s(1), 60);
        assert_eq!(backoff_s(2), 120);
        assert_eq!(backoff_s(3), 240);
        assert_eq!(backoff_s(9), 15_360);
        assert_eq!(backoff_s(10), MAX_BACKOFF_S);
        assert_eq!(backoff_s(1000), MAX_BACKOFF_S);
    }

    #[test]
    fn bookkeeping_of_due_and_finished_rides() {
        let mut s = SyncState::default();
        let all = files(&["a.fit", "b.fit", "c.fit", "d.fit", "e.fit"]);
        s.rides.insert(
            "a.fit".into(),
            RideSync::Uploaded {
                activity_id: Some(1),
                at: 0,
            },
        );
        s.rides
            .insert("b.fit".into(), RideSync::Processing { upload_id: 5 });
        assert_eq!(s.mark_failed("c.fit", "offline".into(), 1000), 1060);
        s.rides.insert(
            "d.fit".into(),
            RideSync::Rejected {
                error: "bad".into(),
            },
        );
        assert_eq!(s.due(&all, 1000), vec!["b.fit", "e.fit"]);
        assert_eq!(s.due(&all, 1060), vec!["b.fit", "c.fit", "e.fit"]);
        // A second failure waits twice as long.
        assert_eq!(s.mark_failed("c.fit", "offline".into(), 1060), 1180);
        assert_eq!(
            s.counts(&all),
            SyncCounts {
                uploaded: 1,
                pending: 2,
                waiting: 0,
                failed: 1,
                rejected: 1,
                next_retry: Some(1180)
            }
        );
        let path = std::env::temp_dir().join(format!("ocw-sync-{}.toml", std::process::id()));
        s.save(&path).unwrap();
        assert_eq!(SyncState::load(&path).unwrap(), s);
        let _ = std::fs::remove_file(&path);
        s.retry_all();
        assert_eq!(s.due(&all, 0), vec!["b.fit", "c.fit", "d.fit", "e.fit"]);
    }

    #[test]
    fn polling_state_machine() {
        let mut p = Poller::new();
        let processing = UploadStatus::Processing { upload_id: 3 };
        assert_eq!(
            p.on_status(&processing, 0),
            PollAction::Wait(Duration::from_secs(2))
        );
        for _ in 0..3 {
            p.on_status(&processing, 0);
        }
        assert_eq!(
            p.on_status(&processing, 0),
            PollAction::Wait(Duration::from_secs(5))
        );
        assert_eq!(
            p.on_status(&UploadStatus::Ready { activity_id: 9 }, 50),
            PollAction::Done(RideSync::Uploaded {
                activity_id: Some(9),
                at: 50
            })
        );
        assert!(matches!(
            p.on_status(&UploadStatus::Failed { error: "x".into() }, 0),
            PollAction::Done(RideSync::Rejected { .. })
        ));
        assert!(matches!(
            p.on_status(&UploadStatus::Duplicate { activity_id: None }, 0),
            PollAction::Done(RideSync::Uploaded {
                activity_id: None,
                ..
            })
        ));
        let mut p = Poller::new();
        let last = (0..=MAX_POLLS).map(|_| p.on_status(&processing, 0)).last();
        assert_eq!(last, Some(PollAction::Later));
    }

    struct Fixture {
        dir: PathBuf,
        paths: SyncPaths,
    }

    impl Fixture {
        fn new(name: &str, rides: &[&str]) -> Self {
            let dir = std::env::temp_dir().join(format!("ocw-sync-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            let rides_dir = dir.join("rides");
            std::fs::create_dir_all(&rides_dir).unwrap();
            for r in rides {
                std::fs::write(rides_dir.join(r), b"FITDATA").unwrap();
            }
            std::fs::write(rides_dir.join("notes.txt"), b"x").unwrap();
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
                    access_token: "acc".into(),
                    refresh_token: "ref".into(),
                    expires_at: 1_000_000,
                    athlete: None,
                })
                .unwrap();
            Self { dir, paths }
        }

        fn state(&self) -> SyncState {
            SyncState::load(&self.paths.state_file).unwrap()
        }

        fn sync(&self, http: &MockHttp, now: i64) -> SyncReport {
            let creds = Credentials {
                client_id: "1".into(),
                client_secret: "s".into(),
            };
            sync_once(
                http,
                &creds,
                &self.paths,
                &Scope::Rides,
                &|| now,
                &|_| {},
                &|_| {},
            )
            .unwrap()
        }

        fn sync_in(&self, http: &MockHttp, now: i64, scope: &Scope) -> SyncReport {
            let creds = Credentials {
                client_id: "1".into(),
                client_secret: "s".into(),
            };
            sync_once(http, &creds, &self.paths, scope, &|| now, &|_| {}, &|_| {}).unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    const PROCESSING: &str = r#"{"id":11,"error":null,"activity_id":null}"#;

    /// Rides marked as uploaded by hand (spec §2.8.1 *Simple upload*) are
    /// skipped: no request, not counted as pending; unmarked they are due
    /// again.
    #[test]
    fn rides_marked_as_uploaded_by_hand_are_skipped() {
        let fx = Fixture::new(
            "manual",
            &["2026-01-01_10-00-00.fit", "2026-01-02_10-00-00.fit"],
        );
        let rides = &fx.paths.rides_dir;
        std::fs::create_dir_all(rides.join("strava")).unwrap();
        std::fs::write(rides.join("strava/2026-01-02_10-10-00.fit"), b"FITDATA").unwrap();
        let mut marks = ManualMarks::load(rides);
        assert!(marks.files.is_empty(), "no file: no marks");
        marks.set(
            &[
                "2026-01-01_10-00-00.fit".into(),
                "strava/2026-01-02_10-10-00.fit".into(),
            ],
            true,
        );
        marks.save(rides).unwrap();
        assert_eq!(ManualMarks::load(rides), marks);
        assert_eq!(
            sync_files(rides),
            vec!["2026-01-02_10-00-00.fit".to_string()]
        );
        assert_eq!(fx.state().counts(&sync_files(rides)).pending, 1);
        let http = MockHttp::default();
        http.reply(201, r#"{"id":12,"error":null,"activity_id":null}"#)
            .reply(200, r#"{"id":12,"error":null,"activity_id":101}"#);
        let report = fx.sync(&http, 1000);
        assert_eq!(
            report.uploaded,
            vec![("2026-01-02_10-00-00.fit".into(), Some(101))]
        );
        assert_eq!(
            http.urls(),
            vec![
                api::UPLOADS_URL.to_string(),
                format!("{}/12", api::UPLOADS_URL)
            ]
        );
        // Unmarked: due again.
        marks.set(&["2026-01-01_10-00-00.fit".into()], false);
        marks.save(rides).unwrap();
        assert_eq!(sync_files(rides).len(), 2);
        // A broken file marks nothing.
        std::fs::write(ManualMarks::path(rides), "files = 3").unwrap();
        assert!(ManualMarks::load(rides).files.is_empty());
    }

    #[test]
    fn uploads_new_rides_and_polls_until_ready() {
        let fx = Fixture::new(
            "upload",
            &["2026-01-01_10-00-00.fit", "2026-01-02_10-00-00.fit"],
        );
        let http = MockHttp::default();
        http.reply(201, PROCESSING)
            .reply(200, PROCESSING)
            .reply(200, r#"{"id":11,"error":null,"activity_id":100}"#)
            .reply(201, r#"{"id":12,"error":null,"activity_id":null}"#)
            .reply(200, r#"{"id":12,"error":null,"activity_id":101}"#);
        let report = fx.sync(&http, 1000);
        assert_eq!(
            report.uploaded,
            vec![
                ("2026-01-01_10-00-00.fit".into(), Some(100)),
                ("2026-01-02_10-00-00.fit".into(), Some(101))
            ]
        );
        assert_eq!(
            http.urls(),
            vec![
                api::UPLOADS_URL.to_string(),
                format!("{}/11", api::UPLOADS_URL),
                format!("{}/11", api::UPLOADS_URL),
                api::UPLOADS_URL.to_string(),
                format!("{}/12", api::UPLOADS_URL),
            ]
        );
        assert_eq!(
            fx.state().counts(&ride_files(&fx.paths.rides_dir)).uploaded,
            2
        );
        // Nothing left to do: no requests on the next sync.
        assert_eq!(fx.sync(&http, 2000), SyncReport::default());
        assert_eq!(http.urls().len(), 5);
    }

    /// The Strava recordings (spec §2.8.1, ST1) are queued with the rides:
    /// `rides/strava/<name>.fit`, uploaded like a ride (mock Strava).
    #[test]
    fn strava_recordings_are_uploaded_too() {
        let fx = Fixture::new("recordings", &["2026-01-01_10-00-00.fit"]);
        let sdir = fx.paths.rides_dir.join("strava");
        std::fs::create_dir_all(&sdir).unwrap();
        std::fs::write(sdir.join("2026-01-01_10-20-00.fit"), b"FITDATA").unwrap();
        std::fs::create_dir_all(fx.paths.rides_dir.join("2026-01-01_10-00-00/photos")).unwrap();
        assert_eq!(
            ride_files(&fx.paths.rides_dir),
            vec![
                "2026-01-01_10-00-00.fit".to_string(),
                "strava/2026-01-01_10-20-00.fit".to_string()
            ]
        );
        let http = MockHttp::default();
        http.reply(201, r#"{"id":1,"error":null,"activity_id":7}"#)
            .reply(201, r#"{"id":2,"error":null,"activity_id":8}"#);
        let report = fx.sync(&http, 1000);
        assert_eq!(
            report.uploaded,
            vec![
                ("2026-01-01_10-00-00.fit".into(), Some(7)),
                ("strava/2026-01-01_10-20-00.fit".into(), Some(8))
            ]
        );
        assert_eq!(external_id("strava/a.fit"), "strava-a.fit");
    }

    #[test]
    fn network_failure_backs_off_and_retries() {
        let fx = Fixture::new("retry", &["a.fit", "b.fit"]);
        let http = MockHttp::default();
        http.fail("connection refused");
        let report = fx.sync(&http, 1000);
        assert_eq!(report.failed.len(), 1);
        assert_eq!(http.urls().len(), 1, "b waits too while offline");
        assert!(matches!(
            fx.state().rides["a.fit"],
            RideSync::Failed {
                attempts: 1,
                retry_at: 1060,
                ..
            }
        ));
        // Before the retry time only the never-tried ride is due; it fails too.
        http.reply(503, "");
        fx.sync(&http, 1030);
        assert_eq!(http.urls().len(), 2);
        // After the backoff both go through.
        http.reply(201, r#"{"id":1,"error":null,"activity_id":7}"#)
            .reply(
                201,
                r#"{"id":2,"error":"b.fit duplicate of 8","activity_id":null}"#,
            );
        let report = fx.sync(&http, 1100);
        assert_eq!(
            report.uploaded,
            vec![("a.fit".into(), Some(7)), ("b.fit".into(), Some(8))]
        );
    }

    #[test]
    fn rejected_rides_are_not_retried() {
        let fx = Fixture::new("rejected", &["a.fit"]);
        let http = MockHttp::default();
        http.reply(201, PROCESSING).reply(
            200,
            r#"{"id":11,"error":"Improperly formatted data.","activity_id":null}"#,
        );
        let report = fx.sync(&http, 0);
        assert_eq!(report.rejected.len(), 1);
        assert_eq!(fx.sync(&http, 1_000_000_000), SyncReport::default());
    }

    #[test]
    fn unfinished_uploads_resume_polling() {
        let fx = Fixture::new("resume", &["a.fit"]);
        let http = MockHttp::default();
        http.reply(201, PROCESSING);
        for _ in 0..MAX_POLLS {
            http.reply(200, PROCESSING);
        }
        let report = fx.sync(&http, 0);
        assert_eq!(report.processing, 1);
        assert_eq!(
            fx.state().rides["a.fit"],
            RideSync::Processing { upload_id: 11 }
        );
        // Next sync: no new upload, just the status.
        http.reply(200, r#"{"id":11,"error":null,"activity_id":5}"#);
        let n = http.urls().len();
        let report = fx.sync(&http, 10);
        assert_eq!(report.uploaded, vec![("a.fit".into(), Some(5))]);
        assert_eq!(http.urls()[n..], [format!("{}/11", api::UPLOADS_URL)]);
    }

    #[test]
    fn revoked_access_needs_a_new_connect() {
        let fx = Fixture::new("revoked", &["a.fit"]);
        let http = MockHttp::default();
        http.reply(401, "{}");
        let report = fx.sync(&http, 0);
        assert!(report.needs_connect);
        // The ride stays pending, not failed.
        assert_eq!(fx.state().due(&files(&["a.fit"]), 0), vec!["a.fit"]);
        // Without a token, nothing is sent.
        fx.paths.token.delete().unwrap();
        assert!(fx.sync(&http, 0).needs_connect);
        assert_eq!(http.urls().len(), 1);
    }

    /// Strava recordings made while Strava was not set up (spec §2.8.1
    /// *Recording without Strava set up, uploaded later*) wait: the usual
    /// sync leaves them (whole rides go as before), the catch-up uploads
    /// them oldest first, skipping the ones marked as uploaded by hand; a
    /// picked one goes alone; a failed one is retried like any ride.
    #[test]
    fn waiting_recordings_wait_for_the_catch_up() {
        let fx = Fixture::new("waiting", &["2026-01-01_09-00-00.fit"]);
        let rides = fx.paths.rides_dir.clone();
        std::fs::create_dir_all(rides.join("strava")).unwrap();
        for n in [
            "2026-01-03_10-00-00.fit",
            "2026-01-01_10-00-00.fit",
            "2026-01-02_10-00-00.fit",
            "2026-01-04_10-00-00.fit",
        ] {
            std::fs::write(rides.join("strava").join(n), b"FITDATA").unwrap();
        }
        let marks = ManualMarks {
            files: ["strava/2026-01-02_10-00-00.fit".to_string()]
                .into_iter()
                .collect(),
        };
        marks.save(&rides).unwrap();
        // Recorded without the set-up: waiting (the marked one is skipped,
        // the whole ride is not a recording).
        assert_eq!(
            mark_waiting(&fx.paths.state_file, &sync_files(&rides), 500).unwrap(),
            3
        );
        assert_eq!(
            mark_waiting(&fx.paths.state_file, &sync_files(&rides), 600).unwrap(),
            0,
            "known already"
        );
        assert_eq!(
            fx.state().rides["strava/2026-01-01_10-00-00.fit"],
            RideSync::Waiting { since: 500 }
        );
        let c = fx.state().counts(&sync_files(&rides));
        assert_eq!((c.waiting, c.pending, c.failed), (3, 1, 0));
        // The usual sync: the whole ride only.
        let http = MockHttp::default();
        http.reply(201, r#"{"id":1,"error":null,"activity_id":7}"#);
        let r = fx.sync(&http, 1000);
        assert_eq!(
            r.uploaded,
            vec![("2026-01-01_09-00-00.fit".into(), Some(7))]
        );
        assert_eq!(fx.sync(&http, 1001), SyncReport::default());
        // A picked one goes alone.
        http.reply(201, r#"{"id":2,"error":null,"activity_id":8}"#);
        let r = fx.sync_in(
            &http,
            1002,
            &Scope::Files(vec!["strava/2026-01-04_10-00-00.fit".into()]),
        );
        assert_eq!(
            r.uploaded,
            vec![("strava/2026-01-04_10-00-00.fit".into(), Some(8))]
        );
        // The catch-up: oldest first; the first fails (offline), the rest
        // wait with it, the failed one is retried by the usual sync.
        http.fail("offline");
        let r = fx.sync_in(&http, 1003, &Scope::Waiting);
        assert_eq!(r.failed.len(), 1);
        assert_eq!(r.failed[0].0, "strava/2026-01-01_10-00-00.fit");
        assert!(matches!(
            fx.state().rides["strava/2026-01-01_10-00-00.fit"],
            RideSync::Failed { attempts: 1, .. }
        ));
        assert_eq!(fx.state().counts(&sync_files(&rides)).waiting, 1);
        http.reply(201, r#"{"id":3,"error":null,"activity_id":9}"#)
            .reply(201, r#"{"id":4,"error":null,"activity_id":10}"#);
        let r = fx.sync_in(&http, 1004, &Scope::Waiting);
        assert_eq!(
            r.uploaded,
            vec![("strava/2026-01-03_10-00-00.fit".into(), Some(9))]
        );
        let r = fx.sync(&http, 1003 + BASE_BACKOFF_S);
        assert_eq!(
            r.uploaded,
            vec![("strava/2026-01-01_10-00-00.fit".into(), Some(10))]
        );
        let c = fx.state().counts(&sync_files(&rides));
        assert_eq!((c.waiting, c.pending, c.uploaded), (0, 0, 4));
        // The marked one was never sent.
        assert!(
            !http
                .requests
                .lock()
                .unwrap()
                .iter()
                .any(|r| String::from_utf8_lossy(&r.body).contains("2026-01-02_10-00-00"))
        );
        // The oldest-first order over both folders.
        let files = vec![
            "strava/2026-01-02_10-00-00.fit".to_string(),
            "2026-01-03_10-00-00.fit".to_string(),
            "strava/2026-01-01_10-00-00.fit".to_string(),
        ];
        assert_eq!(
            SyncState::default().due(&files, 0),
            vec![&files[2], &files[0], &files[1]]
        );
    }
}
