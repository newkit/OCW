//! Riding signed in without the backend (Q20 (a)): the app keeps the
//! account's workshop as last synced and rides with its bikes and kit,
//! **read-only** – nothing is bought, sold or changed until the backend
//! answers again – and queues the summary of every ride that earns hearts;
//! the queue is uploaded once the backend answers, and each ride is
//! credited once, after the checks of spec 6.4 (the backend keys it by its
//! `ride_id`, so an upload repeated after a lost answer changes nothing).
//!
//! The app stores both as text (the desktop in its data directory, the
//! browser in `localStorage`): [`Synced`] as JSON, [`OfflineQueue`] as
//! JSON lines.

use serde::{Deserialize, Serialize};

use crate::rides::{RideAnswer, RideSummary};
use crate::workshop::{Action, WorkshopState};

/// The account's workshop as last synced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Synced {
    pub state: WorkshopState,
    /// When the app got it (Unix s).
    pub synced_at: i64,
}

impl Synced {
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("serializable") + "\n"
    }

    /// The cache of `account` (another account's cache is not used).
    pub fn parse(text: &str, account: &str) -> Option<Self> {
        serde_json::from_str::<Self>(text)
            .ok()
            .filter(|s| s.state.account == account)
    }
}

/// Whether the workshop takes `action` now: offline it is read-only.
pub fn allowed(action: &Action, online: bool) -> Result<(), String> {
    if online {
        return Ok(());
    }
    Err(if action.moves_hearts() {
        "offline: buying and selling wait until the account backend answers again (you ride with the bikes and kit last synced)".into()
    } else {
        "offline: the workshop is read-only until the account backend answers again".into()
    })
}

/// Why an upload did not get an answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UploadError {
    /// No connection, a timeout, the backend down (5xx): keep the ride
    /// and stop for now.
    Offline(String),
    /// The backend refused the request itself (4xx other than 401/429:
    /// malformed): it never will take it, so it leaves the queue.
    Rejected(String),
    /// Not signed in any more (401): keep it for the next sign-in.
    SignedOut,
}

/// What an upload round did.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct UploadReport {
    pub answers: Vec<RideAnswer>,
    /// Rides the backend refused as requests (ride id, why).
    pub dropped: Vec<(String, String)>,
    /// Still queued (offline again, signed out).
    pub kept: usize,
    /// The round stopped at an error (offline, signed out).
    pub stopped: Option<UploadError>,
}

/// The rides waiting for the backend, oldest first.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct OfflineQueue {
    pub pending: Vec<RideSummary>,
}

impl OfflineQueue {
    /// Queues a ride (a ride id already queued is replaced, not doubled).
    pub fn push(&mut self, s: RideSummary) {
        self.pending.retain(|p| p.ride_id != s.ride_id);
        self.pending.push(s);
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub fn len(&self) -> usize {
        self.pending.len()
    }

    /// One JSON line per ride.
    pub fn to_jsonl(&self) -> String {
        self.pending
            .iter()
            .map(|s| serde_json::to_string(s).expect("serializable") + "\n")
            .collect()
    }

    /// Reads the queue; unreadable lines come back as warnings (a broken
    /// line never loses the others).
    pub fn from_jsonl(text: &str) -> (Self, Vec<String>) {
        let mut q = Self::default();
        let mut warnings = Vec::new();
        for (n, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<RideSummary>(line) {
                Ok(s) => q.push(s),
                Err(e) => warnings.push(format!("queued ride line {}: {e}", n + 1)),
            }
        }
        (q, warnings)
    }

    /// Uploads the queue in order with `send` (`POST /rides/summary`):
    /// every answered ride (credited, flagged, refused, or answered before)
    /// leaves the queue; at the first error that is not the ride's own
    /// fault the round stops and the rest stays queued.
    pub fn upload(
        &mut self,
        mut send: impl FnMut(&RideSummary) -> Result<RideAnswer, UploadError>,
    ) -> UploadReport {
        let mut report = UploadReport::default();
        let mut rest = Vec::new();
        let mut pending = std::mem::take(&mut self.pending).into_iter();
        for s in pending.by_ref() {
            match send(&s) {
                Ok(a) => report.answers.push(a),
                Err(UploadError::Rejected(why)) => report.dropped.push((s.ride_id.clone(), why)),
                Err(e) => {
                    rest.push(s);
                    report.stopped = Some(e);
                    break;
                }
            }
        }
        rest.extend(pending);
        report.kept = rest.len();
        self.pending = rest;
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rides::RideStatus;
    use crate::workshop::Garage;

    fn ride(id: &str) -> RideSummary {
        RideSummary {
            ride_id: id.into(),
            started_unix_s: 1_800_000_000,
            duration_s: 2,
            distance_m: 10.0,
            climb_m: 0.0,
            work_j: 400.0,
            weight_kg: 70.0,
            bike: None,
            devices: vec![],
            power_w: vec![200, 200],
            cadence_rpm: vec![],
            heart_bpm: vec![],
        }
    }

    fn answer(s: &RideSummary) -> RideAnswer {
        RideAnswer {
            ride_id: s.ride_id.clone(),
            status: RideStatus::Credited,
            hearts: 1,
            reasons: vec![],
            balance: 5001,
            repeated: false,
        }
    }

    #[test]
    fn the_queue_survives_text_and_never_doubles_a_ride() {
        let mut q = OfflineQueue::default();
        q.push(ride("a"));
        q.push(ride("b"));
        q.push(ride("a"));
        assert_eq!(q.len(), 2);
        let text = q.to_jsonl() + "not json\n";
        let (back, warnings) = OfflineQueue::from_jsonl(&text);
        assert_eq!(back.len(), 2);
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn upload_stops_offline_and_keeps_the_rest() {
        let mut q = OfflineQueue::default();
        for id in ["a", "b", "c", "d"] {
            q.push(ride(id));
        }
        let mut calls = 0;
        let r = q.upload(|s| {
            calls += 1;
            match s.ride_id.as_str() {
                "a" => Ok(answer(s)),
                "b" => Err(UploadError::Rejected("malformed".into())),
                _ => Err(UploadError::Offline("no network".into())),
            }
        });
        assert_eq!(calls, 3);
        assert_eq!(r.answers.len(), 1);
        assert_eq!(r.dropped.len(), 1);
        assert_eq!(r.kept, 2);
        assert_eq!(
            q.pending
                .iter()
                .map(|s| s.ride_id.as_str())
                .collect::<Vec<_>>(),
            ["c", "d"]
        );
        // Back online: the rest goes once, the queue is empty.
        let r = q.upload(|s| Ok(answer(s)));
        assert_eq!(r.answers.len(), 2);
        assert!(q.is_empty());
        let r = q.upload(|_| panic!("nothing left to send"));
        assert!(r.answers.is_empty());
    }

    #[test]
    fn offline_is_read_only() {
        let buy = Action::BuyPart {
            bike: "starter".into(),
            part: "wheels.road-alloy.entry".into(),
        };
        assert!(allowed(&buy, true).is_ok());
        assert!(allowed(&buy, false).unwrap_err().contains("buying"));
        let sel = Action::Select {
            bike: "starter".into(),
        };
        assert!(allowed(&sel, false).is_err());
    }

    #[test]
    fn synced_cache_is_per_account() {
        let s = Synced {
            state: WorkshopState {
                account: "acc-1".into(),
                hearts: 5000,
                garage: Garage::starter("Ada Rider"),
                ledger: vec![],
                at: 1,
            },
            synced_at: 2,
        };
        let t = s.to_json();
        assert_eq!(Synced::parse(&t, "acc-1"), Some(s));
        assert_eq!(Synced::parse(&t, "acc-2"), None);
    }
}
