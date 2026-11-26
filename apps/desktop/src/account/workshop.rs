//! The account's workshop on the `ocw-account` thread (spec 14 Phase 4,
//! 4O; Q20 (a)): the garage and balance synced from the backend and kept
//! in a cache file for riding offline, the workshop's actions sent with an
//! idempotency key, and the queue of ride summaries uploaded once the
//! backend answers. Bevy-free; the files live per account in
//! `<data dir>/ocw/accounts/<account id>/` (`workshop.json`,
//! `ride-queue.jsonl`), so another account's rides are never sent with
//! this one's sign-in.

use std::path::{Path, PathBuf};

use ocw_account_api::offline::{self, OfflineQueue, Synced, UploadError, UploadReport};
use ocw_account_api::rides::RideSummary;
use ocw_account_api::workshop::{Action, ActionAnswer, WorkshopState};
use ocw_auth::client::Session;

use super::flow::{ApiError, AuthClient};

pub const CACHE_FILE: &str = "workshop.json";
pub const QUEUE_FILE: &str = "ride-queue.jsonl";
/// Offline: how often the thread tries the backend again (s).
pub const RETRY_S: i64 = 60;

/// An action's answer for the app.
#[derive(Debug, Clone, PartialEq)]
pub struct ActAnswer {
    pub id: u64,
    pub result: Result<ActionAnswer, String>,
}

/// What the app reads of the account's workshop.
#[derive(Debug, Clone, Default)]
pub struct WorkshopShared {
    /// The workshop as last synced (or from the cache file).
    pub synced: Option<Synced>,
    pub online: bool,
    /// Ride summaries waiting in the queue.
    pub queued: usize,
    /// Answers not yet taken by the app.
    pub answers: Vec<ActAnswer>,
    /// Bumped on every change of `synced`.
    pub generation: u64,
}

/// The workshop's files and state of one account.
pub struct WorkshopSync {
    dir: Option<PathBuf>,
    pub account: String,
    pub synced: Option<Synced>,
    pub queue: OfflineQueue,
    pub online: bool,
    pub last_try: i64,
}

fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    let tmp = path.with_extension("part");
    std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

/// The account's folder under the data directory (the id made safe for a
/// file name).
pub fn account_dir(data: &Path, account: &str) -> PathBuf {
    let safe: String = account
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    data.join("accounts").join(safe)
}

impl WorkshopSync {
    /// The account's cache and queue (`data`: `<data dir>/ocw`; `None`:
    /// in memory only).
    pub fn open(data: Option<&Path>, account: &str) -> (Self, Vec<String>) {
        let dir = data.map(|d| account_dir(d, account));
        let mut warnings = Vec::new();
        let synced = dir
            .as_ref()
            .and_then(|d| std::fs::read_to_string(d.join(CACHE_FILE)).ok())
            .and_then(|t| Synced::parse(&t, account));
        let queue = match dir
            .as_ref()
            .and_then(|d| std::fs::read_to_string(d.join(QUEUE_FILE)).ok())
        {
            Some(t) => {
                let (q, w) = OfflineQueue::from_jsonl(&t);
                warnings.extend(w);
                q
            }
            None => OfflineQueue::default(),
        };
        (
            Self {
                dir,
                account: account.into(),
                synced,
                queue,
                online: false,
                last_try: i64::MIN / 2,
            },
            warnings,
        )
    }

    fn save_cache(&self) {
        if let (Some(d), Some(s)) = (&self.dir, &self.synced)
            && let Err(e) = write_atomic(&d.join(CACHE_FILE), &s.to_json())
        {
            tracing::warn!(%e, "account workshop cache");
        }
    }

    fn save_queue(&self) {
        if let Some(d) = &self.dir
            && let Err(e) = write_atomic(&d.join(QUEUE_FILE), &self.queue.to_jsonl())
        {
            tracing::warn!(%e, "ride queue");
        }
    }

    fn keep(&mut self, state: WorkshopState, now: i64) {
        self.synced = Some(Synced {
            state,
            synced_at: now,
        });
        self.save_cache();
    }

    /// Fetches the workshop. `Err` when it got no answer (the cache stays).
    pub fn sync(&mut self, c: &AuthClient, s: &Session, now: i64) -> Result<(), ApiError> {
        self.last_try = now;
        match c.workshop(s) {
            Ok(state) => {
                self.online = true;
                self.keep(state, now);
                Ok(())
            }
            Err(e) => {
                self.online = !matches!(e, ApiError::Offline(_));
                Err(e)
            }
        }
    }

    /// Sends an action (offline: refused, the workshop is read-only).
    pub fn act(
        &mut self,
        c: &AuthClient,
        s: &Session,
        action: &Action,
        key: &str,
        now: i64,
    ) -> Result<ActionAnswer, String> {
        offline::allowed(action, self.online)?;
        match c.workshop_act(s, action, key) {
            Ok(a) => {
                self.online = true;
                if let Some(syn) = &mut self.synced {
                    syn.state.garage = a.garage.clone();
                    syn.state.hearts = a.hearts;
                    syn.synced_at = now;
                    self.save_cache();
                }
                Ok(a)
            }
            Err(ApiError::Offline(e)) => {
                self.online = false;
                Err(format!(
                    "the account server does not answer ({e}): the workshop is read-only until it does"
                ))
            }
            Err(e) => Err(e.to_string()),
        }
    }

    /// Queues a ride's summary (written to the file at once).
    pub fn queue_ride(&mut self, summary: RideSummary) {
        self.queue.push(summary);
        self.save_queue();
    }

    /// Uploads the queue in order; what the backend answered leaves it.
    pub fn upload(&mut self, c: &AuthClient, s: &Session) -> UploadReport {
        if self.queue.is_empty() {
            return UploadReport::default();
        }
        let r = self.queue.upload(|sum| {
            c.ride_summary(s, sum).map_err(|e| match e {
                ApiError::Offline(m) => UploadError::Offline(m),
                ApiError::SignedOut => UploadError::SignedOut,
                ApiError::Refused(m) => UploadError::Rejected(m),
            })
        });
        if matches!(r.stopped, Some(UploadError::Offline(_))) {
            self.online = false;
        } else if !r.answers.is_empty() {
            self.online = true;
        }
        self.save_queue();
        r
    }

    pub fn shared(&self, generation: u64) -> WorkshopShared {
        WorkshopShared {
            synced: self.synced.clone(),
            online: self.online,
            queued: self.queue.len(),
            answers: Vec::new(),
            generation,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_account_api::workshop::Garage;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ocw-ws-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn the_cache_and_queue_survive_a_restart_per_account() {
        let d = temp("files");
        let (mut w, warn) = WorkshopSync::open(Some(&d), "acc/1");
        assert!(warn.is_empty() && w.synced.is_none() && w.queue.is_empty());
        w.keep(
            WorkshopState {
                account: "acc/1".into(),
                hearts: 4321,
                garage: Garage::starter("Jo Cadence"),
                ledger: vec![],
                at: 5,
            },
            6,
        );
        w.queue_ride(RideSummary {
            ride_id: "r".into(),
            started_unix_s: 1,
            duration_s: 1,
            distance_m: 0.0,
            climb_m: 0.0,
            work_j: 0.0,
            weight_kg: 70.0,
            bike: None,
            devices: vec![],
            power_w: vec![1],
            cadence_rpm: vec![],
            heart_bpm: vec![],
        });
        assert!(account_dir(&d, "acc/1").ends_with("accounts/acc_1"));
        let (w, _) = WorkshopSync::open(Some(&d), "acc/1");
        assert_eq!(w.synced.as_ref().map(|s| s.state.hearts), Some(4321));
        assert_eq!(w.queue.len(), 1);
        // Another account sees neither.
        let (o, _) = WorkshopSync::open(Some(&d), "acc-2");
        assert!(o.synced.is_none() && o.queue.is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }
}
