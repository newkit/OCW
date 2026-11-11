//! Strava sync: every saved ride that is not on Strava yet gets uploaded.
//! The state (`<rides dir>/strava-sync.toml`) remembers what is uploaded,
//! what failed (and when to retry, with exponential backoff) and what
//! Strava rejected for good.

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
    /// uploads and failed ones whose retry time has come.
    pub fn due<'a>(&self, files: &'a [String], now: i64) -> Vec<&'a String> {
        files
            .iter()
            .filter(|f| match self.rides.get(*f) {
                None | Some(RideSync::Processing { .. }) => true,
                Some(RideSync::Failed { retry_at, .. }) => *retry_at <= now,
                Some(RideSync::Uploaded { .. } | RideSync::Rejected { .. }) => false,
            })
            .collect()
    }

    pub fn counts(&self, files: &[String]) -> SyncCounts {
        let mut c = SyncCounts::default();
        for f in files {
            match self.rides.get(f) {
                None | Some(RideSync::Processing { .. }) => c.pending += 1,
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

/// The `.fit` files in `dir`, sorted (oldest first: names start with the date).
pub fn ride_files(dir: &Path) -> Vec<String> {
    let mut files: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.ends_with(".fit"))
        .collect();
    files.sort();
    files
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

/// Uploads all due rides. `sleep` waits between status polls (no-op in
/// tests). Saves the state after every ride.
pub fn sync_once(
    http: &dyn Http,
    creds: &Credentials,
    paths: &SyncPaths,
    now: &dyn Fn() -> i64,
    sleep: &dyn Fn(Duration),
) -> Result<SyncReport, String> {
    let mut report = SyncReport::default();
    let mut state = SyncState::load(&paths.state_file)?;
    let files = ride_files(&paths.rides_dir);
    let due: Vec<String> = state.due(&files, now()).into_iter().cloned().collect();
    if due.is_empty() {
        return Ok(report);
    }
    for file in due {
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
                    RideSync::Failed { .. } => {}
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
                external_id: file.to_string(),
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
            sync_once(http, &creds, &self.paths, &|| now, &|_| {}).unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    const PROCESSING: &str = r#"{"id":11,"error":null,"activity_id":null}"#;

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
}
