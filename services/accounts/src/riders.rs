//! The rider of an account (spec 14 Phase 4, 4O W5b; 4N): `GET` / `PUT
//! /account/rider`. One rider per account; its name follows the name rules
//! of `ocw_core::name` (2–20 characters of every script, no filter) and is
//! unique on the backend (folded with `name::key`); it becomes the
//! account's display name, the name its next tokens carry and others see.
//! The rest is the apps' rider profile, kept as they send it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ocw_account_api::rider::RiderDoc;

use crate::service::AuthService;
use crate::workshop::ApiFail;

/// Why a rider was not kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PutFail {
    /// Another account's rider has this name.
    NameTaken,
    Io(String),
}

/// Where the riders are kept.
pub trait RiderStore: Send + Sync {
    fn rider(&self, account: &str) -> Result<Option<RiderDoc>, String>;
    /// Keeps `doc` (`key`: its name folded); another account's rider with
    /// the same key refuses it.
    fn put_rider(&self, account: &str, key: &str, doc: &RiderDoc) -> Result<(), PutFail>;
    fn delete_rider(&self, account: &str) -> Result<(), String>;
}

/// The riders in memory (development, tests).
#[derive(Default)]
pub struct MemRiders(Mutex<HashMap<String, (String, RiderDoc)>>);

impl RiderStore for MemRiders {
    fn rider(&self, account: &str) -> Result<Option<RiderDoc>, String> {
        Ok(self
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(account)
            .map(|(_, d)| d.clone()))
    }

    fn put_rider(&self, account: &str, key: &str, doc: &RiderDoc) -> Result<(), PutFail> {
        let mut m = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if m.iter().any(|(a, (k, _))| a != account && k == key) {
            return Err(PutFail::NameTaken);
        }
        m.insert(account.to_owned(), (key.to_owned(), doc.clone()));
        Ok(())
    }

    fn delete_rider(&self, account: &str) -> Result<(), String> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(account);
        Ok(())
    }
}

/// The rider endpoints over an [`AuthService`].
pub struct Riders {
    auth: Arc<AuthService>,
}

impl Riders {
    pub fn new(auth: Arc<AuthService>) -> Self {
        Self { auth }
    }

    /// `GET /account/rider`: 404 when the account has none yet.
    pub fn get(&self, authorization: Option<&str>) -> Result<RiderDoc, ApiFail> {
        let c = self.auth.bearer(authorization)?;
        match self.auth.rider_store().rider(&c.sub) {
            Ok(Some(d)) => Ok(d),
            Ok(None) => Err(ApiFail::new(404, "not_found", "no rider yet")),
            Err(e) => {
                tracing::warn!(error = %e, "rider storage");
                Err(ApiFail::new(500, "server_error", "storage error"))
            }
        }
    }

    /// `PUT /account/rider`: the name checked (refused: 400, taken by
    /// another account: 409), then kept and made the account's name.
    pub fn put(&self, authorization: Option<&str>, body: &[u8]) -> Result<RiderDoc, ApiFail> {
        let c = self.auth.bearer(authorization)?;
        let doc: RiderDoc = serde_json::from_slice(body)
            .map_err(|e| ApiFail::new(400, "invalid_request", e.to_string()))?;
        doc.check_shape()
            .map_err(|e| ApiFail::new(400, "invalid_request", e))?;
        let name = ocw_core::name::check(&doc.name)
            .map_err(|e| ApiFail::new(400, "invalid_name", e.to_string()))?;
        let key = ocw_core::name::key(&name);
        let doc = doc.named(&name, self.auth.now());
        match self.auth.rider_store().put_rider(&c.sub, &key, &doc) {
            Ok(()) => {}
            Err(PutFail::NameTaken) => {
                return Err(ApiFail::new(
                    409,
                    "name_taken",
                    format!("{name} is taken by another rider: choose another name"),
                ));
            }
            Err(PutFail::Io(e)) => {
                tracing::warn!(error = %e, "rider storage");
                return Err(ApiFail::new(500, "server_error", "storage error"));
            }
        }
        if let Err(e) = self.auth.store().rename(&c.sub, &name) {
            tracing::warn!(error = %e, "rider name on the account");
        }
        Ok(doc)
    }
}

/// The rider in the account's export and deletion.
pub struct RiderHooks {
    pub store: Arc<dyn RiderStore>,
}

impl crate::AccountHooks for RiderHooks {
    fn name(&self) -> &str {
        "rider"
    }

    fn export(&self, account: &str) -> serde_json::Value {
        self.store
            .rider(account)
            .ok()
            .flatten()
            .map_or(serde_json::Value::Null, |d| {
                serde_json::to_value(d).unwrap_or_default()
            })
    }

    fn delete(&self, account: &str) {
        if let Err(e) = self.store.delete_rider(account) {
            tracing::warn!(error = %e, "deleting the rider");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(name: &str) -> RiderDoc {
        RiderDoc {
            name: name.into(),
            rider: serde_json::json!({"name": name, "weight_kg": 64.0}),
            updated_at: 0,
        }
    }

    #[test]
    fn names_are_unique_across_accounts() {
        let m = MemRiders::default();
        m.put_rider("a", "ida uphill", &doc("Ida Uphill")).unwrap();
        // The same account renames freely.
        m.put_rider("a", "ida uphill", &doc("Ida Uphill")).unwrap();
        assert_eq!(
            m.put_rider("b", "ida uphill", &doc("IDA UPHILL")),
            Err(PutFail::NameTaken)
        );
        m.put_rider("b", "jo cadence", &doc("Jo Cadence")).unwrap();
        assert_eq!(m.rider("b").unwrap().unwrap().name, "Jo Cadence");
        m.delete_rider("a").unwrap();
        assert!(m.rider("a").unwrap().is_none());
        m.put_rider("b", "ida uphill", &doc("Ida Uphill")).unwrap();
    }
}
