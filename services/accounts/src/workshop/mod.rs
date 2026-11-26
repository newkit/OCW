//! The workshop and rides modules of the backend (spec 14 Phase 4, 4O
//! W5c / W5d): an account's bikes, parts and kit, its hearts balance with
//! the append-only ledger, purchases and sales at the catalogue's prices,
//! and the hearts of rides.
//!
//! - **Start** (Q18 (a), Q21 (a)): the first time an account's workshop is
//!   touched it gets 5,000 ♥ (ledger reason `start`) and the free starter
//!   bike with its base parts; nothing is imported from a local wallet.
//! - **Actions** ([`Workshop::act`]): `POST /workshop/purchase`, `/sell`
//!   (an `Idempotency-Key` header is required: a repeated request gets the
//!   first answer and changes nothing) and `/garage` (equip, unequip,
//!   select, paint), checked by [`rules`] in one transaction.
//! - **Rides** (Q19 (a)): `POST /rides/credit` – a credit signed by a game
//!   server whose key a configured root certified, credited directly;
//!   `POST /rides/summary` – the app's summary of any other ride, credited
//!   after the plausibility checks of spec 6.4
//!   ([`ocw_account_api::plausibility`]), flagged ones held. Each ride is
//!   credited once (its id is the transaction's key; the ledger has a
//!   unique ride reference), and a ride overlapping another credited ride
//!   of the account in time is refused (no hearts twice for one ride sent
//!   both ways).

pub mod rules;
pub mod store;

use std::sync::Arc;

use ocw_account_api::rides::{
    RIDE_CREDIT_CONTEXT, RIDE_CREDIT_KIND, RideAnswer, RideCredit, RideStatus, RideSummary,
};
use ocw_account_api::workshop::{
    Action, ActionAnswer, Endpoint, Garage, LedgerEntry, Reason, WorkshopState,
};
use ocw_account_api::{START_HEARTS, plausibility};
use ocw_assets::parts::Catalogue;
use ocw_auth::oidc::ErrorResponse;
use ocw_world::manifest::{SignedDocument, TrustStore};
use sha2::{Digest, Sha256};

use crate::service::{AuthService, OAuthError};
use rules::Booking;
use store::{Change, Done, RideRecord, StoreFail, Tx, TxError, Wallet, WorkshopStore};

/// The most ledger lines `GET /workshop` and `/workshop/ledger` return.
pub const LEDGER_PAGE: usize = 50;
/// A signed credit older than this is refused (s, 7 days).
pub const MAX_CREDIT_AGE_S: i64 = 7 * 86_400;

/// An error answer: HTTP status, code, sentence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiFail {
    pub status: u16,
    pub error: &'static str,
    pub message: String,
}

impl ApiFail {
    pub fn new(status: u16, error: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            error,
            message: message.into(),
        }
    }

    pub fn into_oauth(self) -> OAuthError {
        (self.status, ErrorResponse::new(self.error, self.message))
    }
}

impl From<rules::RuleError> for ApiFail {
    fn from(e: rules::RuleError) -> Self {
        let (status, code) = match &e {
            rules::RuleError::NotFound(_) => (404, "not_found"),
            rules::RuleError::Refused(_) => (422, "refused"),
            rules::RuleError::Insufficient { .. } => (409, "insufficient_hearts"),
        };
        Self::new(status, code, e.to_string())
    }
}

impl From<OAuthError> for ApiFail {
    fn from((status, body): OAuthError) -> Self {
        Self::new(
            status,
            if status == 401 {
                "invalid_token"
            } else {
                "error"
            },
            body.error_description.unwrap_or(body.error),
        )
    }
}

fn store_fail(e: StoreFail) -> ApiFail {
    match e {
        StoreFail::NoAccount => ApiFail::new(404, "not_found", "no such account"),
        StoreFail::KeyReused => ApiFail::new(422, "idempotency_key_reused", e.to_string()),
        StoreFail::Negative => ApiFail::new(409, "insufficient_hearts", e.to_string()),
        StoreFail::RideTwice => ApiFail::new(409, "ride_credited", e.to_string()),
        StoreFail::Io(m) => {
            tracing::warn!(error = %m, "workshop storage");
            ApiFail::new(500, "server_error", "storage error")
        }
    }
}

fn tx_fail(e: TxError<ApiFail>) -> ApiFail {
    match e {
        TxError::Op(f) => f,
        TxError::Store(s) => store_fail(s),
    }
}

fn hash(parts: &[&[u8]]) -> String {
    let mut h = Sha256::new();
    for p in parts {
        h.update((p.len() as u64).to_le_bytes());
        h.update(p);
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// `YYYY-MM-DD` of a Unix time.
pub fn date(unix_s: i64) -> String {
    let (y, m, d, ..) = ocw_core::timestamp::civil(unix_s);
    format!("{y:04}-{m:02}-{d:02}")
}

/// The start of a new account's workshop (Q18 (a), Q21 (a)).
fn begin(w: &mut Wallet, rider: &str) -> Vec<Booking> {
    if w.started {
        if !rider.is_empty() {
            w.garage.rider = rider.into();
        }
        return Vec::new();
    }
    w.started = true;
    w.garage = Garage::starter(rider);
    vec![Booking::new(START_HEARTS, Reason::Start, "")]
}

/// The workshop and rides endpoints over an [`AuthService`] (its bearer
/// tokens, its workshop store, its trust in game servers).
pub struct Workshop {
    auth: Arc<AuthService>,
    catalogue: &'static Catalogue,
}

impl Workshop {
    pub fn new(auth: Arc<AuthService>) -> Self {
        Self {
            auth,
            catalogue: ocw_assets::shipped::catalogue(),
        }
    }

    fn store(&self) -> &Arc<dyn WorkshopStore> {
        self.auth.workshop_store()
    }

    fn trust(&self) -> &TrustStore {
        self.auth.hearts_trust()
    }

    /// The account and its display name of a bearer token.
    fn who(&self, authorization: Option<&str>) -> Result<(String, String), ApiFail> {
        let c = self.auth.bearer(authorization)?;
        let name = self
            .auth
            .store()
            .account(&c.sub)
            .map(|a| a.display_name)
            .unwrap_or_default();
        Ok((c.sub, name))
    }

    /// `GET /workshop`.
    pub fn state(&self, authorization: Option<&str>) -> Result<WorkshopState, ApiFail> {
        let (account, name) = self.who(authorization)?;
        self.state_of(&account, &name)
    }

    /// The workshop of `account` (starting it if new).
    pub fn state_of(&self, account: &str, name: &str) -> Result<WorkshopState, ApiFail> {
        let now = self.auth.now();
        let mut garage = None;
        let done = self
            .store()
            .transact(
                Tx {
                    account,
                    key: None,
                    window: None,
                    now,
                },
                &mut |w| {
                    let bookings = begin(w, name);
                    garage = Some(w.garage.clone());
                    Ok(Change {
                        bookings,
                        ..Change::default()
                    })
                },
            )
            .map_err(tx_fail)?;
        Ok(WorkshopState {
            account: account.into(),
            hearts: done.hearts,
            garage: garage.expect("the transaction ran"),
            ledger: self
                .store()
                .ledger(account, LEDGER_PAGE)
                .map_err(store_fail)?,
            at: now,
        })
    }

    /// `GET /workshop/ledger`.
    pub fn ledger(&self, authorization: Option<&str>) -> Result<Vec<LedgerEntry>, ApiFail> {
        let (account, _) = self.who(authorization)?;
        self.store()
            .ledger(&account, LEDGER_PAGE)
            .map_err(store_fail)
    }

    /// `POST /workshop/purchase|sell|garage` with the action as JSON.
    pub fn act(
        &self,
        authorization: Option<&str>,
        endpoint: Endpoint,
        idempotency_key: Option<&str>,
        body: &[u8],
    ) -> Result<ActionAnswer, ApiFail> {
        let (account, name) = self.who(authorization)?;
        let action: Action = serde_json::from_slice(body)
            .map_err(|e| ApiFail::new(400, "invalid_request", format!("the action: {e}")))?;
        if action.endpoint() != endpoint {
            return Err(ApiFail::new(
                400,
                "invalid_request",
                format!("this action goes to {}", action.endpoint().path()),
            ));
        }
        let key = idempotency_key.map(str::trim).filter(|k| !k.is_empty());
        if let Some(k) = key
            && (k.len() > 128 || !k.is_ascii())
        {
            return Err(ApiFail::new(
                400,
                "invalid_request",
                "the Idempotency-Key is at most 128 ASCII characters",
            ));
        }
        if endpoint != Endpoint::Garage && key.is_none() {
            return Err(ApiFail::new(
                400,
                "invalid_request",
                "purchases and sales need an Idempotency-Key header",
            ));
        }
        let h = hash(&[endpoint.path().as_bytes(), body]);
        let now = self.auth.now();
        let today = date(now);
        let cat = self.catalogue;
        let done = self
            .store()
            .transact(
                Tx {
                    account: &account,
                    key: key.map(|k| (k, h.as_str())),
                    window: None,
                    now,
                },
                &mut |w| {
                    let mut bookings = begin(w, &name);
                    let balance = w.hearts + bookings.iter().map(|b| b.delta).sum::<i64>();
                    let r = rules::apply(cat, &mut w.garage, balance, &today, &action)?;
                    let after = balance + r.bookings.iter().map(|b| b.delta).sum::<i64>();
                    let answer = ActionAnswer {
                        message: r.message,
                        hearts: after,
                        garage: w.garage.clone(),
                        item: r.item,
                        bike: r.bike,
                        paid: r.paid,
                        refund: r.refund,
                        fitted: r.fitted,
                    };
                    bookings.extend(r.bookings);
                    Ok(Change {
                        bookings,
                        ride: None,
                        answer: serde_json::to_value(&answer).expect("serializable"),
                    })
                },
            )
            .map_err(tx_fail)?;
        let mut a: ActionAnswer = serde_json::from_value(done.answer)
            .map_err(|e| ApiFail::new(500, "server_error", e.to_string()))?;
        if done.repeated {
            a.message = format!("{} (answered before)", a.message);
        }
        Ok(a)
    }

    /// Books a ride (both paths): idempotent per ride, refused if it
    /// overlaps a ride credited before.
    #[allow(clippy::too_many_arguments)]
    fn book_ride(
        &self,
        account: &str,
        name: &str,
        record: RideRecord,
        shown_id: &str,
        body_hash: &str,
        bike: Option<String>,
        distance_m: f64,
    ) -> Result<RideAnswer, ApiFail> {
        let now = self.auth.now();
        let key = format!("ride:{}", record.ride_id);
        let window = (record.started, record.ended.max(record.started + 1));
        let shown = shown_id.to_owned();
        let done = self.store().transact(
            Tx {
                account,
                key: Some((&key, body_hash)),
                window: Some(window),
                now,
            },
            &mut |w| {
                let mut bookings = begin(w, name);
                let mut rec = record.clone();
                if let Some((other, ..)) = w.overlapping.first()
                    && rec.status == RideStatus::Credited
                {
                    rec.status = RideStatus::Refused;
                    rec.reasons
                        .push(format!("overlaps the ride {other}, credited before"));
                }
                let hearts = if rec.status == RideStatus::Credited {
                    rec.hearts
                } else {
                    0
                };
                if hearts > 0 {
                    bookings.push(Booking::new(
                        i64::from(hearts),
                        Reason::Ride,
                        rec.ride_id.clone(),
                    ));
                }
                if rec.status != RideStatus::Refused {
                    w.garage.ride(bike.as_deref(), distance_m);
                }
                let balance = w.hearts + bookings.iter().map(|b| b.delta).sum::<i64>();
                let answer = RideAnswer {
                    ride_id: shown.clone(),
                    status: rec.status,
                    hearts,
                    reasons: rec.reasons.clone(),
                    balance,
                    repeated: false,
                };
                Ok(Change {
                    bookings,
                    ride: Some(rec),
                    answer: serde_json::to_value(&answer).expect("serializable"),
                })
            },
        );
        let done: Done = done.map_err(tx_fail)?;
        let mut a: RideAnswer = serde_json::from_value(done.answer)
            .map_err(|e| ApiFail::new(500, "server_error", e.to_string()))?;
        if done.repeated {
            a.repeated = true;
            a.balance = done.hearts;
        }
        if a.status == RideStatus::Flagged {
            tracing::info!(%account, ride = %a.ride_id, reasons = ?a.reasons, "ride held for review");
        }
        Ok(a)
    }

    /// `POST /rides/credit`: a ride credit signed by a game server (Q19
    /// (a)). No bearer token: the signature is the authority.
    pub fn credit(&self, body: &[u8]) -> Result<RideAnswer, ApiFail> {
        let doc: SignedDocument = serde_json::from_slice(body)
            .map_err(|e| ApiFail::new(400, "invalid_request", format!("the credit: {e}")))?;
        let c: RideCredit = serde_json::from_str(doc.signed_json())
            .map_err(|e| ApiFail::new(400, "invalid_request", format!("the credit: {e}")))?;
        let now = self.auth.now();
        let claims = doc
            .verify(self.trust(), RIDE_CREDIT_CONTEXT, c.issued_unix_s, now)
            .map_err(|e| {
                ApiFail::new(
                    403,
                    "untrusted_server",
                    format!(
                        "this server may not credit hearts ({e}); upload the ride summary instead"
                    ),
                )
            })?;
        if c.kind != RIDE_CREDIT_KIND {
            return Err(ApiFail::new(400, "invalid_request", "not a ride credit"));
        }
        if c.issued_unix_s < now - MAX_CREDIT_AGE_S {
            return Err(ApiFail::new(422, "refused", "the credit is too old"));
        }
        let finite = [c.distance_m, c.work_j, c.climb_m]
            .iter()
            .all(|x| x.is_finite() && *x >= 0.0);
        if !finite
            || c.ended_unix_s < c.started_unix_s
            || c.ride_id.is_empty()
            || c.ride_id.len() > 128
        {
            return Err(ApiFail::new(400, "invalid_request", "the ride's numbers"));
        }
        let expected = ocw_core::hearts::earned(c.work_j, c.climb_m);
        if c.hearts != expected {
            return Err(ApiFail::new(
                422,
                "refused",
                format!(
                    "{} ♥ claimed for this work and climbing, {expected} ♥ earned",
                    c.hearts
                ),
            ));
        }
        let Some(acc) = self.auth.store().account(&c.account) else {
            return Err(ApiFail::new(404, "not_found", "no such account"));
        };
        let server = claims.key_id.to_string();
        let record = RideRecord {
            ride_id: format!("server:{server}:{}", c.ride_id),
            source: "server".into(),
            server_key: Some(server),
            status: RideStatus::Credited,
            hearts: expected,
            started: c.started_unix_s,
            ended: c.ended_unix_s,
            reasons: Vec::new(),
            body: serde_json::from_str(doc.signed_json()).unwrap_or_default(),
        };
        let h = hash(&[doc.signed_json().as_bytes()]);
        self.book_ride(
            &acc.id,
            &acc.display_name,
            record,
            &c.ride_id,
            &h,
            c.bike.clone(),
            c.distance_m,
        )
    }

    /// `POST /rides/summary`: the app's summary of a ride (Q19 (a), Q20
    /// (a)).
    pub fn summary(&self, authorization: Option<&str>, body: &[u8]) -> Result<RideAnswer, ApiFail> {
        let (account, name) = self.who(authorization)?;
        let s: RideSummary = serde_json::from_slice(body)
            .map_err(|e| ApiFail::new(400, "invalid_request", format!("the summary: {e}")))?;
        let now = self.auth.now();
        let v = plausibility::check(&s, now);
        let (status, reasons) = if !v.refused.is_empty() {
            (RideStatus::Refused, v.refused)
        } else if !v.flags.is_empty() {
            (RideStatus::Flagged, v.flags)
        } else {
            (RideStatus::Credited, Vec::new())
        };
        let mut body_json = serde_json::to_value(&s).unwrap_or_default();
        // Keep the summary for a review, not every sample twice.
        if status == RideStatus::Credited
            && let Some(o) = body_json.as_object_mut()
        {
            for k in ["power_w", "cadence_rpm", "heart_bpm"] {
                o.remove(k);
            }
        }
        let record = RideRecord {
            ride_id: format!("app:{}", s.ride_id),
            source: "summary".into(),
            server_key: None,
            status,
            hearts: v.hearts,
            started: s.started_unix_s,
            ended: s.started_unix_s + i64::from(s.duration_s),
            reasons,
            body: body_json,
        };
        let h = hash(&[body]);
        self.book_ride(
            &account,
            &name,
            record,
            &s.ride_id,
            &h,
            s.bike.clone(),
            s.distance_m,
        )
    }
}

/// The workshop's part of an account's export and deletion (spec 12).
pub struct WorkshopHooks {
    pub store: Arc<dyn WorkshopStore>,
}

impl crate::AccountHooks for WorkshopHooks {
    fn name(&self) -> &str {
        "workshop"
    }

    fn export(&self, account: &str) -> serde_json::Value {
        let ledger = self.store.ledger(account, usize::MAX).unwrap_or_default();
        let rides: Vec<serde_json::Value> = self
            .store
            .rides(account)
            .unwrap_or_default()
            .into_iter()
            .map(|r| {
                serde_json::json!({
                    "ride_id": r.ride_id,
                    "source": r.source,
                    "status": r.status,
                    "hearts": r.hearts,
                    "started": r.started,
                    "ended": r.ended,
                    "reasons": r.reasons,
                })
            })
            .collect();
        serde_json::json!({
            "hearts": self.store.ledger_sum(account).unwrap_or(0),
            "ledger": ledger,
            "rides": rides,
        })
    }

    fn delete(&self, account: &str) {
        if let Err(e) = self.store.delete(account) {
            tracing::warn!(%account, error = %e, "deleting the workshop");
        }
    }
}
