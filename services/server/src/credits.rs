//! The hearts of a signed server (spec 14 Phase 4, 4O Q19 (a)): when a
//! signed-in rider leaves, the game server signs a ride credit with its
//! signing key – certified by the project's root key, the key hierarchy of
//! world packages (spec 4.7) – and sends it to the accounts backend
//! (`POST /rides/credit`), off the game loop on a thread of its own, with
//! retries while the backend does not answer. The backend credits each ride
//! once, so a retry after a lost answer changes nothing.
//!
//! `ocw-server --credit-key FILE` (a signing key of `world-builder keys
//! new-signing`) turns it on; the welcome then tells the apps that this
//! server credits the hearts (`Welcome.hearts_credited`), so they upload no
//! summary of the ride.

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::Duration;

use ocw_account_api::rides::{RIDE_CREDIT_CONTEXT, RIDE_CREDIT_KIND, RideAnswer, RideCredit};
use ocw_world::manifest::{SignedDocument, SigningKeyPair};

use crate::{CreditSink, EndedRide};

/// Tries per credit before it is given up (with [`RETRY`] between).
pub const TRIES: u32 = 20;
/// The wait between two tries while the backend does not answer.
pub const RETRY: Duration = Duration::from_secs(30);

/// The signed credit of an ended ride at `now_unix_s`.
pub fn credit_of(r: &EndedRide, now_unix_s: i64) -> RideCredit {
    RideCredit {
        kind: RIDE_CREDIT_KIND.into(),
        account: r.account_id.clone(),
        ride_id: r.ride_id.clone(),
        started_unix_s: r.started_unix_s,
        ended_unix_s: r.ended_unix_s,
        distance_m: r.distance_m,
        work_j: r.work_j,
        climb_m: r.climb_m,
        hearts: ocw_core::hearts::earned(r.work_j, r.climb_m),
        bike: None,
        issued_unix_s: now_unix_s,
    }
}

/// Why a credit was not answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendError {
    /// No answer or a 5xx: try again later.
    Later(String),
    /// The backend refused it (4xx): never again.
    Refused(u16, String),
}

/// Posts one signed credit to `backend` (its base URL).
pub fn post(backend: &str, doc: &SignedDocument) -> Result<RideAnswer, SendError> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(15)))
        .build()
        .into();
    let body = serde_json::to_string(doc).map_err(|e| SendError::Refused(0, e.to_string()))?;
    let mut r = agent
        .post(&format!("{}/rides/credit", backend.trim_end_matches('/')))
        .header("content-type", "application/json")
        .send(&body)
        .map_err(|e| SendError::Later(e.to_string()))?;
    let status = r.status().as_u16();
    let text = r.body_mut().read_to_string().unwrap_or_default();
    match status {
        200 => serde_json::from_str(&text).map_err(|e| SendError::Later(e.to_string())),
        s if s >= 500 || s == 429 => Err(SendError::Later(format!("{s} {text}"))),
        s => Err(SendError::Refused(s, text)),
    }
}

fn now_s() -> i64 {
    crate::unix_now_ms() / 1000
}

/// Signs and sends the credits of ended rides on its own thread.
pub struct BackendCredits {
    tx: Sender<EndedRide>,
}

impl BackendCredits {
    /// Starts the sender for `backend` (the accounts backend's base URL).
    pub fn start(key: SigningKeyPair, backend: String) -> std::io::Result<Self> {
        Self::start_with(key, backend, RETRY)
    }

    /// With another retry interval (tests).
    pub fn start_with(
        key: SigningKeyPair,
        backend: String,
        retry: Duration,
    ) -> std::io::Result<Self> {
        let (tx, rx) = channel();
        std::thread::Builder::new()
            .name("ocw-credits".into())
            .spawn(move || run(&key, &backend, &rx, retry))?;
        Ok(Self { tx })
    }
}

impl CreditSink for BackendCredits {
    fn ride_ended(&self, ride: EndedRide) {
        let _ = self.tx.send(ride);
    }
}

fn run(key: &SigningKeyPair, backend: &str, rx: &Receiver<EndedRide>, retry: Duration) {
    // (signed credit, tries left)
    let mut pending: Vec<(SignedDocument, String, u32)> = Vec::new();
    loop {
        let wait = if pending.is_empty() {
            Duration::from_secs(3600)
        } else {
            retry
        };
        match rx.recv_timeout(wait) {
            Ok(r) => {
                let doc = key.sign_document(RIDE_CREDIT_CONTEXT, &credit_of(&r, now_s()));
                pending.push((doc, r.ride_id, TRIES));
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) if pending.is_empty() => return,
            Err(RecvTimeoutError::Disconnected) => std::thread::sleep(retry),
        }
        pending.retain_mut(|(doc, ride, left)| match post(backend, doc) {
            Ok(a) => {
                tracing::info!(%ride, hearts = a.hearts, status = ?a.status, "ride credited");
                false
            }
            Err(SendError::Refused(s, why)) => {
                tracing::warn!(%ride, status = s, %why, "ride credit refused");
                false
            }
            Err(SendError::Later(why)) => {
                *left -= 1;
                if *left == 0 {
                    tracing::warn!(%ride, %why, "ride credit given up");
                }
                *left > 0
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_credit_counts_the_hearts_of_the_work() {
        let r = EndedRide {
            account_id: "acc".into(),
            ride_id: "1-2-3".into(),
            started_unix_s: 100,
            ended_unix_s: 3700,
            distance_m: 30_000.0,
            work_j: 720_000.0,
            climb_m: 300.0,
        };
        let c = credit_of(&r, 3800);
        assert_eq!(c.hearts, 72 + 15);
        assert_eq!(c.kind, RIDE_CREDIT_KIND);
        assert_eq!((c.started_unix_s, c.issued_unix_s), (100, 3800));
    }
}
