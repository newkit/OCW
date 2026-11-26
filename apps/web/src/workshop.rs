//! The account's workshop in the browser (spec 14 Phase 4, 4O part 3; B6
//! parity with the desktop): the shared workshop (`ocw_ui::workshop`) with
//! the browser's host. Signed in, the garage and the balance are the
//! account's – fetched with `GET /workshop`, kept in `localStorage` for a
//! page opened offline (Q20 (a): read-only then), every purchase, sale and
//! garage change a `fetch()` with an idempotency key whose answer the
//! frame loop applies a frame later, never waiting. Signed out the
//! workshop, the character and the garage are locked (Phase 10 Q5): the
//! browser has no local wallet.
//!
//! The state lives behind a mutex so the `fetch()` futures can answer into
//! it; [`Inner`] is plain Rust and tested natively.

use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use ocw_account_api::offline::{self, OfflineQueue, Synced};
use ocw_account_api::rides::{RideAnswer, RideSummary};
use ocw_account_api::workshop::{Action, ActionAnswer, WorkshopState};
use ocw_ui::workshop::{ActAnswer, WorkshopHost, WorkshopUpdate};

use crate::account::WebAccount;

/// Offline: how often the page tries the backend again (s).
pub const RETRY_S: f64 = 60.0;

/// The `localStorage` key of an account's workshop as last synced.
pub fn cache_key(account: &str) -> String {
    format!("ocw.account.{account}.workshop")
}

/// The `localStorage` key of an account's queued ride summaries (Q20).
pub fn queue_key(account: &str) -> String {
    format!("ocw.account.{account}.rides")
}

/// The account's workshop as the page knows it.
#[derive(Debug, Default)]
pub struct Inner {
    /// Whose (the signed-in account's id).
    pub account: Option<String>,
    pub synced: Option<Synced>,
    /// The backend answered the last call.
    pub online: bool,
    pub answers: Vec<ActAnswer>,
    /// Bumped on every change for the frame loop.
    pub generation: u64,
    /// A `GET /workshop` is on its way.
    pub syncing: bool,
    /// When the last sync was tried (page time, s).
    pub last_try_s: f64,
    /// Ride summaries waiting for the backend (kept in `localStorage`).
    pub queue: OfflineQueue,
    /// A ride is being uploaded.
    pub uploading: bool,
    /// Lines for the console.
    pub said: Vec<String>,
}

impl Inner {
    /// Signed in as `account` (its cache, if the page kept one).
    pub fn open(&mut self, account: &str, cached: Option<&str>, queued: Option<&str>) {
        let (queue, warnings) = OfflineQueue::from_jsonl(queued.unwrap_or_default());
        *self = Self {
            account: Some(account.into()),
            synced: cached.and_then(|t| Synced::parse(t, account)),
            queue,
            said: warnings,
            last_try_s: f64::NEG_INFINITY,
            generation: self.generation + 1,
            ..Self::default()
        };
    }

    /// Signed out: nothing kept in memory.
    pub fn close(&mut self) {
        *self = Self {
            generation: self.generation + 1,
            ..Self::default()
        };
    }

    /// `GET /workshop` answered: the cache's new text (to keep).
    pub fn synced_ok(&mut self, state: WorkshopState, now_unix_s: i64) -> Option<String> {
        if self.account.as_deref() != Some(state.account.as_str()) {
            return None;
        }
        let s = Synced {
            state,
            synced_at: now_unix_s,
        };
        let json = s.to_json();
        self.synced = Some(s);
        self.online = true;
        self.syncing = false;
        self.generation += 1;
        Some(json)
    }

    /// A call got no answer (`offline`) or was refused.
    pub fn failed(&mut self, offline: bool) {
        self.syncing = false;
        if offline {
            self.online = false;
        }
        self.generation += 1;
    }

    /// An action's answer: the garage and balance it left (the cache's
    /// new text when it changed).
    pub fn answered(
        &mut self,
        id: u64,
        result: Result<ActionAnswer, (bool, String)>,
        now_unix_s: i64,
    ) -> Option<String> {
        let mut keep = None;
        let result = match result {
            Ok(a) => {
                self.online = true;
                if let Some(s) = &mut self.synced {
                    s.state.garage = a.garage.clone();
                    s.state.hearts = a.hearts;
                    s.synced_at = now_unix_s;
                    keep = Some(s.to_json());
                }
                Ok(a)
            }
            Err((offline, m)) => {
                if offline {
                    self.online = false;
                }
                Err(m)
            }
        };
        self.answers.push(ActAnswer { id, result });
        self.generation += 1;
        keep
    }

    /// Queues a ride's summary: the queue's new text (to keep).
    pub fn queue_ride(&mut self, s: RideSummary) -> String {
        self.queue.push(s);
        self.generation += 1;
        self.queue.to_jsonl()
    }

    /// The next ride to upload (online, none on its way).
    pub fn next_ride(&mut self) -> Option<RideSummary> {
        if !self.online || self.uploading {
            return None;
        }
        let s = self.queue.pending.first().cloned()?;
        self.uploading = true;
        Some(s)
    }

    /// A ride's answer (`Err((offline, why))`): answered or refused rides
    /// leave the queue, offline keeps it. The queue's new text.
    pub fn ride_answered(
        &mut self,
        ride_id: &str,
        result: Result<RideAnswer, (bool, String)>,
    ) -> String {
        self.uploading = false;
        match result {
            Ok(a) => {
                self.queue.pending.retain(|p| p.ride_id != ride_id);
                if let Some(s) = &mut self.synced {
                    s.state.hearts = a.balance;
                }
                self.said.push(format!(
                    "account: ride {} {:?}{}",
                    a.ride_id,
                    a.status,
                    if a.hearts > 0 {
                        format!(", {} ♥", a.hearts)
                    } else {
                        String::new()
                    }
                ));
            }
            Err((true, _)) => self.online = false,
            Err((false, why)) => {
                self.queue.pending.retain(|p| p.ride_id != ride_id);
                self.said
                    .push(format!("account: ride {ride_id} not taken: {why}"));
            }
        }
        self.generation += 1;
        self.queue.to_jsonl()
    }

    /// What changed since `seen` (the answers taken).
    pub fn update(&mut self, signed_in: bool, seen: u64) -> WorkshopUpdate {
        WorkshopUpdate {
            signed_in: signed_in && self.synced.is_some(),
            synced: (self.generation != seen)
                .then(|| self.synced.clone())
                .flatten(),
            online: self.online,
            queued: self.queue.len(),
            generation: self.generation,
            answers: std::mem::take(&mut self.answers),
        }
    }

    /// Whether a sync is due: none yet, or offline and a minute passed.
    pub fn sync_due(&self, now_s: f64) -> bool {
        self.account.is_some()
            && !self.syncing
            && (self.synced.is_none() || !self.online)
            && now_s - self.last_try_s >= RETRY_S
    }
}

/// The page's account workshop (shared with the `fetch()` futures).
#[derive(Resource, Clone, Default)]
pub struct WebWorkshop(pub Arc<Mutex<Inner>>);

impl WebWorkshop {
    pub fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The signed-in session: (issuer, access token, account id).
fn session(world: &World) -> Option<(String, String, String)> {
    let a = world.get_resource::<WebAccount>()?;
    if !matches!(a.state, ocw_client::account::AccountState::SignedIn { .. }) {
        return None;
    }
    let s = a.session.as_ref()?;
    Some((
        s.issuer.clone(),
        s.access_token.clone(),
        s.account_id.clone(),
    ))
}

fn signed_in(world: &World) -> bool {
    session(world).is_some()
}

fn now_unix_s() -> i64 {
    ocw_server::unix_now_ms() / 1000
}

/// The browser's host: `local` – the page's own (frame-stepped) server.
pub fn host(local: bool) -> WorkshopHost {
    WorkshopHost {
        local_server: local,
        capture: false,
        no_hearts: false,
        requires_account: true,
        session_start: |_| 0.0,
        signed_in,
        workshop_update: |w, seen| {
            let signed = signed_in(w);
            match w.get_resource::<WebWorkshop>() {
                Some(ws) => ws.lock().update(signed, seen),
                None => WorkshopUpdate::default(),
            }
        },
        send_action,
        sync_workshop: |w| {
            if let (Some(ws), Some(s)) = (w.get_resource::<WebWorkshop>(), session(w)) {
                sync(ws.clone(), s);
            }
        },
        ..WorkshopHost::default()
    }
}

/// Sends an action (answered into [`Inner::answers`]).
fn send_action(world: &World, id: u64, action: Action) -> bool {
    let (Some(ws), Some(s)) = (world.get_resource::<WebWorkshop>(), session(world)) else {
        return false;
    };
    let online = ws.lock().online;
    if let Err(e) = offline::allowed(&action, online) {
        ws.lock().answered(id, Err((false, e)), now_unix_s());
        return true;
    }
    act(ws.clone(), s, id, action);
    true
}

/// Follows the sign-in: the account's cache at once, then the backend's;
/// offline, a new try every minute.
pub fn follow(world: &mut World) {
    let Some(ws) = world.get_resource::<WebWorkshop>().cloned() else {
        return;
    };
    let now = world
        .get_resource::<Time<Real>>()
        .map_or(0.0, |t| t.elapsed_secs_f64());
    let s = session(world);
    let mut inner = ws.lock();
    match &s {
        Some((_, _, account)) if inner.account.as_deref() != Some(account.as_str()) => {
            let cached = load(&cache_key(account));
            let queued = load(&queue_key(account));
            inner.open(account, cached.as_deref(), queued.as_deref());
        }
        None if inner.account.is_some() => inner.close(),
        _ => {}
    }
    let said = std::mem::take(&mut inner.said);
    let mut sync_now = false;
    if s.is_some() && inner.sync_due(now) {
        inner.syncing = true;
        inner.last_try_s = now;
        sync_now = true;
    }
    let ride = s.as_ref().and_then(|_| inner.next_ride());
    drop(inner);
    if let Some(s) = s {
        if sync_now {
            sync(ws.clone(), s.clone());
        }
        if let Some(r) = ride {
            upload(ws, s, r);
        }
    }
    if !said.is_empty()
        && let Some(mut c) = world.get_resource_mut::<ocw_render::console::ConsoleState>()
    {
        for m in said {
            info!("{m}");
            c.print(m);
        }
    }
}

/// Queues a ride's summary for the account (spec 14 Phase 4, 4O Q19 /
/// Q20): signed in, on a server that does not credit the hearts itself;
/// uploaded when the backend answers. `None`: not queued (why).
pub fn queue_ride(
    world: &World,
    activity: &ocw_fit::Activity,
    virtual_s: &[bool],
    credited: bool,
) -> Result<String, String> {
    let ws = world
        .get_resource::<WebWorkshop>()
        .ok_or("no account workshop")?;
    let Some((_, _, account)) = session(world) else {
        return Err("signed out".into());
    };
    if !ocw_client::ride_summary::uploads(true, credited, false, false, false) {
        return Err("this server credits the hearts itself".into());
    }
    let rider = ocw_ui::profiles::local_rider(world);
    let bike = world
        .get_resource::<ocw_ui::workshop::WorkshopState>()
        .and_then(|s| s.wallet.get(&rider.name).map(|g| g.active.clone()));
    let id = format!(
        "{}-web-{}",
        ocw_client::recording::file_stem(activity.start_unix_s),
        ride_suffix()
    );
    let summary =
        ocw_client::ride_summary::summary(activity, virtual_s, &id, rider.weight_kg, bike)
            .ok_or("no real power in this ride: nothing to credit")?;
    let text = ws.lock().queue_ride(summary);
    keep(&queue_key(&account), &text);
    Ok(id)
}

pub struct WebWorkshopPlugin {
    /// The ride runs on the page's own server.
    pub local: bool,
}

impl Plugin for WebWorkshopPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(host(self.local))
            .init_resource::<WebWorkshop>()
            .init_resource::<ocw_ui::profiles::RiderProfiles>()
            .add_plugins(ocw_ui::workshop::WorkshopPlugin)
            .add_systems(PreUpdate, follow);
    }
}

#[cfg(target_arch = "wasm32")]
fn load(key: &str) -> Option<String> {
    crate::account::local_get(key)
}

#[cfg(target_arch = "wasm32")]
fn keep(key: &str, text: &str) {
    crate::account::local_set(key, text);
}

#[cfg(target_arch = "wasm32")]
fn ride_suffix() -> String {
    crate::account::random_key()[..12].to_string()
}

#[cfg(target_arch = "wasm32")]
fn upload(ws: WebWorkshop, (issuer, token, account): (String, String, String), r: RideSummary) {
    wasm_bindgen_futures::spawn_local(async move {
        let body = serde_json::to_string(&r).unwrap_or_default();
        let res = crate::account::call_json(
            "POST",
            &format!("{issuer}/rides/summary"),
            Some(&body),
            &token,
            None,
        )
        .await;
        let result = match res {
            Ok((200, body)) => {
                serde_json::from_str::<RideAnswer>(&body).map_err(|e| (true, e.to_string()))
            }
            Ok((s, body)) => Err((s >= 500 || s == 429 || s == 401, refusal(s, &body))),
            Err(e) => Err((true, e)),
        };
        let text = ws.lock().ride_answered(&r.ride_id, result);
        keep(&queue_key(&account), &text);
    });
}

#[cfg(target_arch = "wasm32")]
fn sync(ws: WebWorkshop, (issuer, token, account): (String, String, String)) {
    wasm_bindgen_futures::spawn_local(async move {
        let r = crate::account::call_json("GET", &format!("{issuer}/workshop"), None, &token, None)
            .await;
        let mut inner = ws.lock();
        match r {
            Ok((200, body)) => match serde_json::from_str::<WorkshopState>(&body) {
                Ok(state) => {
                    if let Some(text) = inner.synced_ok(state, now_unix_s()) {
                        keep(&cache_key(&account), &text);
                    }
                }
                Err(e) => {
                    warn!("workshop: {e}");
                    inner.failed(true);
                }
            },
            Ok((s, _)) => inner.failed(s >= 500 || s == 429),
            Err(_) => inner.failed(true),
        }
    });
}

#[cfg(target_arch = "wasm32")]
fn act(
    ws: WebWorkshop,
    (issuer, token, account): (String, String, String),
    id: u64,
    action: Action,
) {
    wasm_bindgen_futures::spawn_local(async move {
        let key = crate::account::random_key();
        let body = serde_json::to_string(&action).unwrap_or_default();
        let url = format!("{issuer}{}", action.endpoint().path());
        let r = crate::account::call_json("POST", &url, Some(&body), &token, Some(&key)).await;
        let result = match r {
            Ok((200, body)) => {
                serde_json::from_str::<ActionAnswer>(&body).map_err(|e| (true, e.to_string()))
            }
            Ok((s, body)) => Err((s >= 500 || s == 429, refusal(s, &body))),
            Err(_) => Err((
                true,
                "the account server does not answer: the workshop is read-only until it does"
                    .into(),
            )),
        };
        let mut inner = ws.lock();
        if let Some(text) = inner.answered(id, result, now_unix_s()) {
            keep(&cache_key(&account), &text);
        }
    });
}

/// The backend's sentence for a refusal.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
fn refusal(status: u16, body: &str) -> String {
    serde_json::from_str::<ocw_account_api::workshop::ApiError>(body)
        .ok()
        .and_then(|e| e.error_description.or(Some(e.error)))
        .unwrap_or_else(|| format!("refused (HTTP {status})"))
}

// Natively (the tests) there is no page and no backend.
#[cfg(not(target_arch = "wasm32"))]
fn load(_: &str) -> Option<String> {
    None
}

#[cfg(not(target_arch = "wasm32"))]
fn keep(_: &str, _: &str) {}

#[cfg(not(target_arch = "wasm32"))]
fn ride_suffix() -> String {
    "native".into()
}

#[cfg(not(target_arch = "wasm32"))]
fn upload(ws: WebWorkshop, _: (String, String, String), r: RideSummary) {
    ws.lock()
        .ride_answered(&r.ride_id, Err((true, "needs the browser".into())));
}

#[cfg(not(target_arch = "wasm32"))]
fn sync(ws: WebWorkshop, _: (String, String, String)) {
    ws.lock().failed(true);
}

#[cfg(not(target_arch = "wasm32"))]
fn act(ws: WebWorkshop, _: (String, String, String), id: u64, _: Action) {
    ws.lock().answered(
        id,
        Err((true, "the account server needs the browser".into())),
        now_unix_s(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_account_api::workshop::Garage;

    fn state(account: &str, hearts: i64) -> WorkshopState {
        WorkshopState {
            account: account.into(),
            hearts,
            garage: Garage::starter("Cleo Cadence"),
            ledger: vec![],
            at: 5,
        }
    }

    /// The page's side of Q20 (a): the cache first, the backend's answer
    /// replaces it, offline nothing is bought (read-only), an action's
    /// answer leaves its garage and balance, another account sees nothing
    /// of this one's.
    #[test]
    fn the_page_follows_the_accounts_workshop() {
        let mut i = Inner::default();
        assert!(!i.sync_due(0.0), "signed out: nothing to sync");
        let cached = Synced {
            state: state("acc-c", 4200),
            synced_at: 1,
        }
        .to_json();
        i.open("acc-c", Some(&cached), None);
        assert_eq!(i.synced.as_ref().unwrap().state.hearts, 4200);
        assert!(!i.online && i.sync_due(0.0));
        let u = i.update(true, 0);
        assert!(u.signed_in && !u.online, "the cache, read-only");
        assert_eq!(u.synced.unwrap().state.hearts, 4200);
        // The backend answers.
        let text = i.synced_ok(state("acc-c", 5000), 10).unwrap();
        assert!(text.contains("acc-c") && i.online && !i.sync_due(100.0));
        // Another account's answer is not this one's.
        assert!(i.synced_ok(state("acc-x", 1), 11).is_none());
        let seen = i.generation;
        assert!(i.update(true, seen).synced.is_none(), "nothing new");
        // A purchase answered.
        let mut g = Garage::starter("Cleo Cadence");
        g.rider = "changed".into();
        let kept = i.answered(
            7,
            Ok(ActionAnswer {
                hearts: 4400,
                garage: g,
                message: "bought".into(),
                fitted: vec![],
                item: None,
                bike: None,
                paid: 600,
                refund: 0,
            }),
            12,
        );
        assert!(kept.unwrap().contains("4400"));
        let u = i.update(true, seen);
        assert_eq!(u.answers.len(), 1);
        assert_eq!(u.synced.unwrap().state.hearts, 4400);
        // Offline: the next call fails, read-only, a new try after 60 s.
        i.answered(8, Err((true, "down".into())), 13);
        assert!(!i.online && i.sync_due(1_000.0));
        assert!(offline::allowed(&Action::Sell { item: 1 }, i.online).is_err());
        // A ride queued while offline waits; online it goes up once.
        let ride = |id: &str| RideSummary {
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
        };
        let text = i.queue_ride(ride("r1"));
        assert!(text.contains("r1") && i.update(true, 0).queued == 1);
        assert!(i.next_ride().is_none(), "offline: kept");
        i.synced_ok(state("acc-c", 4400), 20);
        let r = i.next_ride().unwrap();
        assert!(i.next_ride().is_none(), "one at a time");
        i.ride_answered(&r.ride_id, Err((true, "down".into())));
        assert!(!i.online && i.queue.len() == 1, "offline again: kept");
        i.synced_ok(state("acc-c", 4400), 21);
        let r = i.next_ride().unwrap();
        let text = i.ride_answered(
            &r.ride_id,
            Ok(RideAnswer {
                ride_id: "r1".into(),
                status: ocw_account_api::rides::RideStatus::Credited,
                hearts: 3,
                reasons: vec![],
                balance: 4403,
                repeated: false,
            }),
        );
        assert!(text.is_empty() && i.queue.is_empty());
        assert_eq!(i.synced.as_ref().unwrap().state.hearts, 4403);
        assert!(i.said.iter().any(|l| l.contains("3 ♥")));
        i.queue_ride(ride("r2"));
        let r = i.next_ride().unwrap();
        i.ride_answered(&r.ride_id, Err((false, "malformed".into())));
        assert!(i.queue.is_empty(), "refused as a request: dropped");
        assert_eq!(queue_key("acc-c"), "ocw.account.acc-c.rides");
        // Signed out: nothing kept.
        i.close();
        assert!(i.account.is_none() && i.synced.is_none());
        assert!(!i.update(false, 0).signed_in);
        assert_eq!(cache_key("acc-c"), "ocw.account.acc-c.workshop");
    }
}
