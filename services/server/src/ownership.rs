//! What signed-in riders own, from the accounts backend (spec 14 Phase 4,
//! 4O W5d; 4E): when a rider joins or changes bikes, the server fetches
//! their garage – `GET /workshop` with the rider's own access token – on a
//! thread of its own, never in the game loop; until it arrives only base
//! parts count, then the bike the rider asked for applies again with the
//! parts they own (`GameServer::owned_arrived`). `ocw-server
//! --check-parts` turns it on.

use std::collections::{BTreeSet, HashMap};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ocw_account_api::workshop::{Garage, WorkshopState};

use crate::{Owned, PartsOwned};

/// What a garage owns: its items' catalogue ids and its bikes' profiles.
pub fn owned_of(g: &Garage) -> Owned {
    Owned {
        parts: g.items.iter().map(|i| i.part.clone()).collect(),
        profiles: g
            .bikes
            .iter()
            .map(|b| b.profile.clone())
            .collect::<BTreeSet<_>>(),
    }
}

#[derive(Default)]
struct Known {
    owned: HashMap<String, Owned>,
    updated: Vec<String>,
}

/// Fetches garages from the accounts backend on its own thread.
pub struct BackendOwnership {
    tx: Mutex<Sender<(String, String)>>,
    known: Arc<Mutex<Known>>,
}

impl BackendOwnership {
    /// Starts the fetcher for `backend` (the accounts backend's base URL).
    pub fn start(backend: String) -> std::io::Result<Self> {
        let (tx, rx) = channel();
        let known = Arc::new(Mutex::new(Known::default()));
        let k = known.clone();
        std::thread::Builder::new()
            .name("ocw-ownership".into())
            .spawn(move || run(&backend, &rx, &k))?;
        Ok(Self {
            tx: Mutex::new(tx),
            known,
        })
    }
}

impl PartsOwned for BackendOwnership {
    fn refresh(&self, account: &str, token: &str) {
        let _ = self
            .tx
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .send((account.to_owned(), token.to_owned()));
    }

    fn owned(&self, account: &str) -> Option<Owned> {
        self.known
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .owned
            .get(account)
            .cloned()
    }

    fn take_updated(&self) -> Vec<String> {
        std::mem::take(&mut self.known.lock().unwrap_or_else(|e| e.into_inner()).updated)
    }
}

/// `GET /workshop` with the rider's token.
pub fn fetch(backend: &str, token: &str) -> Result<WorkshopState, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(20)))
        .build()
        .into();
    let mut r = agent
        .get(&format!("{}/workshop", backend.trim_end_matches('/')))
        .header("authorization", &format!("Bearer {token}"))
        .call()
        .map_err(|e| e.to_string())?;
    let status = r.status().as_u16();
    let text = r.body_mut().read_to_string().unwrap_or_default();
    if status != 200 {
        return Err(format!("HTTP {status}"));
    }
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

fn run(backend: &str, rx: &Receiver<(String, String)>, known: &Mutex<Known>) {
    while let Ok((account, token)) = rx.recv() {
        // A hosted backend woken from sleep answers after a few seconds:
        // one more try.
        let got = fetch(backend, &token).or_else(|e| {
            tracing::debug!(%account, %e, "garage: second try");
            std::thread::sleep(Duration::from_secs(3));
            fetch(backend, &token)
        });
        match got {
            Ok(w) if w.account == account => {
                let o = owned_of(&w.garage);
                tracing::info!(%account, parts = o.parts.len(), bikes = o.profiles.len(), "garage arrived");
                let mut k = known.lock().unwrap_or_else(|e| e.into_inner());
                k.owned.insert(account.clone(), o);
                k.updated.push(account);
            }
            Ok(_) => tracing::warn!(%account, "garage: another account's answer"),
            Err(e) => tracing::warn!(%account, %e, "garage not fetched: base parts only"),
        }
    }
}
