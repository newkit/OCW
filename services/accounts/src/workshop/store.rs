//! The workshop's storage (4O): per account the hearts balance, the
//! garage (bikes, items, the worn kit), the append-only hearts ledger, the
//! idempotency keys and the rides credited.
//!
//! Every change runs as one transaction ([`WorkshopStore::transact`]): the
//! account's wallet is locked and loaded, the change computed, then the
//! garage, the ledger lines, the cached balance (never below zero), the
//! idempotency key with its answer and the ride record are written
//! together – or nothing is. [`MemWorkshop`] keeps it in memory
//! (development, the `memory` and `file` storage kinds, unit tests);
//! [`crate::pg::PgStore`] in PostgreSQL.

use std::collections::BTreeMap;
use std::sync::Mutex;

use ocw_account_api::rides::RideStatus;
use ocw_account_api::workshop::{Garage, LedgerEntry};

use super::ApiFail;
use super::rules::Booking;

/// An account's wallet as a transaction sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct Wallet {
    pub hearts: i64,
    pub garage: Garage,
    /// The start grant was booked (a new account: false).
    pub started: bool,
    /// The rides credited (or held) whose time overlaps the window asked
    /// for: (ride id, start, end).
    pub overlapping: Vec<(String, i64, i64)>,
}

/// A ride credited, held or refused (4O `ride_uploads`).
#[derive(Debug, Clone, PartialEq)]
pub struct RideRecord {
    pub ride_id: String,
    /// `server` (a signed credit) or `summary` (uploaded by the app).
    pub source: String,
    /// The key id of the signing server, if any.
    pub server_key: Option<String>,
    pub status: RideStatus,
    pub hearts: u32,
    pub started: i64,
    pub ended: i64,
    pub reasons: Vec<String>,
    /// The uploaded summary or credit (JSON), kept for a review.
    pub body: serde_json::Value,
}

/// What a transaction writes besides the garage.
#[derive(Debug, Clone, Default)]
pub struct Change {
    pub bookings: Vec<Booking>,
    pub ride: Option<RideRecord>,
    /// The answer, stored with the idempotency key.
    pub answer: serde_json::Value,
}

/// A transaction's request.
#[derive(Debug, Clone, Default)]
pub struct Tx<'a> {
    pub account: &'a str,
    /// The idempotency key and a hash of the request it came with.
    pub key: Option<(&'a str, &'a str)>,
    /// Load the rides overlapping this time window ([`Wallet::overlapping`]).
    pub window: Option<(i64, i64)>,
    pub now: i64,
}

/// What a transaction did.
#[derive(Debug, Clone, PartialEq)]
pub struct Done {
    pub answer: serde_json::Value,
    /// The key was seen before: this is the first answer, nothing changed.
    pub repeated: bool,
    pub hearts: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreFail {
    #[error("no such account")]
    NoAccount,
    #[error("the Idempotency-Key was used for another request")]
    KeyReused,
    #[error("the balance would go below zero")]
    Negative,
    #[error("this ride was credited already")]
    RideTwice,
    #[error("storage: {0}")]
    Io(String),
}

/// The change a transaction computes, or why it refuses (its own error
/// type, carried through the store unchanged).
pub type TxFn<'f, E> = dyn FnMut(&mut Wallet) -> Result<Change, E> + 'f;

/// Either the closure's error or the store's.
#[derive(Debug)]
pub enum TxError<E> {
    Op(E),
    Store(StoreFail),
}

/// The workshop storage.
pub trait WorkshopStore: Send + Sync {
    /// Runs `f` in one transaction on the account's wallet (see the
    /// module docs). A key seen before with the same hash returns its
    /// stored answer without calling `f`; with another hash
    /// [`StoreFail::KeyReused`]. An error of `f` changes nothing and
    /// stores no key.
    fn transact(&self, tx: Tx<'_>, f: &mut TxFn<'_, ApiFail>) -> Result<Done, TxError<ApiFail>>;

    /// The newest `limit` ledger lines, newest first.
    fn ledger(&self, account: &str, limit: usize) -> Result<Vec<LedgerEntry>, StoreFail>;

    /// The sum of the account's ledger (equals the cached balance).
    fn ledger_sum(&self, account: &str) -> Result<i64, StoreFail>;

    /// The rides of the account, newest first (export, review).
    fn rides(&self, account: &str) -> Result<Vec<RideRecord>, StoreFail>;

    /// Deletes the account's workshop data (account deletion, spec 12).
    fn delete(&self, account: &str) -> Result<(), StoreFail>;
}

#[derive(Debug, Clone, Default)]
struct MemAccount {
    hearts: i64,
    garage: Option<Garage>,
    ledger: Vec<LedgerEntry>,
    keys: BTreeMap<String, (String, serde_json::Value)>,
    rides: Vec<RideRecord>,
}

/// The workshop in memory.
#[derive(Default)]
pub struct MemWorkshop {
    data: Mutex<BTreeMap<String, MemAccount>>,
}

impl MemWorkshop {
    pub fn new() -> Self {
        Self::default()
    }
}

impl WorkshopStore for MemWorkshop {
    fn transact(&self, tx: Tx<'_>, f: &mut TxFn<'_, ApiFail>) -> Result<Done, TxError<ApiFail>> {
        let mut data = self.data.lock().unwrap_or_else(|e| e.into_inner());
        let acc = data.entry(tx.account.to_owned()).or_default();
        if let Some((key, hash)) = tx.key
            && let Some((h, answer)) = acc.keys.get(key)
        {
            if h != hash {
                return Err(TxError::Store(StoreFail::KeyReused));
            }
            return Ok(Done {
                answer: answer.clone(),
                repeated: true,
                hearts: acc.hearts,
            });
        }
        let overlapping = match tx.window {
            Some((from, to)) => acc
                .rides
                .iter()
                .filter(|r| r.status != RideStatus::Refused && r.started < to && r.ended > from)
                .map(|r| (r.ride_id.clone(), r.started, r.ended))
                .collect(),
            None => Vec::new(),
        };
        let mut w = Wallet {
            hearts: acc.hearts,
            garage: acc.garage.clone().unwrap_or_else(|| Garage::starter("")),
            started: acc.garage.is_some(),
            overlapping,
        };
        let change = f(&mut w).map_err(TxError::Op)?;
        let delta: i64 = change.bookings.iter().map(|b| b.delta).sum();
        if acc.hearts + delta < 0 {
            return Err(TxError::Store(StoreFail::Negative));
        }
        if let Some(r) = &change.ride
            && acc.rides.iter().any(|x| x.ride_id == r.ride_id)
        {
            return Err(TxError::Store(StoreFail::RideTwice));
        }
        acc.hearts += delta;
        acc.garage = Some(w.garage);
        for b in &change.bookings {
            acc.ledger.push(LedgerEntry {
                at: tx.now,
                delta: b.delta,
                reason: b.reason,
                reference: b.reference.clone(),
            });
        }
        if let Some(r) = change.ride {
            acc.rides.push(r);
        }
        if let Some((key, hash)) = tx.key {
            acc.keys
                .insert(key.to_owned(), (hash.to_owned(), change.answer.clone()));
        }
        Ok(Done {
            answer: change.answer,
            repeated: false,
            hearts: acc.hearts,
        })
    }

    fn ledger(&self, account: &str, limit: usize) -> Result<Vec<LedgerEntry>, StoreFail> {
        let data = self.data.lock().unwrap_or_else(|e| e.into_inner());
        Ok(data
            .get(account)
            .map(|a| a.ledger.iter().rev().take(limit).cloned().collect())
            .unwrap_or_default())
    }

    fn ledger_sum(&self, account: &str) -> Result<i64, StoreFail> {
        let data = self.data.lock().unwrap_or_else(|e| e.into_inner());
        Ok(data
            .get(account)
            .map(|a| a.ledger.iter().map(|e| e.delta).sum())
            .unwrap_or(0))
    }

    fn rides(&self, account: &str) -> Result<Vec<RideRecord>, StoreFail> {
        let data = self.data.lock().unwrap_or_else(|e| e.into_inner());
        Ok(data
            .get(account)
            .map(|a| a.rides.iter().rev().cloned().collect())
            .unwrap_or_default())
    }

    fn delete(&self, account: &str) -> Result<(), StoreFail> {
        self.data
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(account);
        Ok(())
    }
}
