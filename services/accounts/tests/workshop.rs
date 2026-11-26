//! The workshop, the hearts ledger and the ride credits over HTTP (spec 14
//! Phase 4, 4O; Q18–Q21), end to end on the loopback interface: the apps'
//! requests, the backend, the fake Google for the sign-ins.
//!
//! Every scenario runs twice: on the memory store and on PostgreSQL
//! (`OCW_TEST_DATABASE_URL`, set by `make test-accounts` with a
//! throw-away cluster; without it the PostgreSQL runs are skipped, unless
//! `OCW_REQUIRE_POSTGRES=1` makes them fail).

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU32, Ordering};

use ocw_account_api::START_HEARTS;
use ocw_account_api::offline::{OfflineQueue, UploadError};
use ocw_account_api::rides::{
    Device, DeviceKind, RIDE_CREDIT_CONTEXT, RIDE_CREDIT_KIND, RideAnswer, RideCredit, RideStatus,
    RideSummary,
};
use ocw_account_api::workshop::{Action, ActionAnswer, Reason, STARTER_BIKE, WorkshopState};
use ocw_accounts::AuthService;
use ocw_accounts::config::{Config, StorageKind};
use ocw_accounts::fake::{FakeGoogle, TestBrowser};
use ocw_auth::client::{self, Callback, PendingLogin, Session};
use ocw_auth::{CLIENT_DESKTOP, Provider};
use ocw_world::manifest::{RootKeyPair, SigningKeyPair, TrustStore};

const APP: &str = "http://127.0.0.1:5999/callback";

struct Env {
    _rt: tokio::runtime::Runtime,
    svc: Arc<AuthService>,
    issuer: String,
    google: FakeGoogle,
    clock: Arc<AtomicI64>,
    browser: TestBrowser,
    root: RootKeyPair,
    server: SigningKeyPair,
}

fn real_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn seed() -> [u8; 32] {
    let mut s = [0u8; 32];
    getrandom::fill(&mut s).unwrap();
    s
}

/// The backend on `pg` (a PostgreSQL URL) or in memory.
fn env(pg: Option<&str>) -> Env {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let clock = Arc::new(AtomicI64::new(real_now()));
    let c = clock.clone();
    // The project's root key and a game server's signing key it certified.
    let root = RootKeyPair::from_seed("Test project root", seed());
    let server_seed = seed();
    let now = real_now();
    let cert = root.certify(
        "test game server",
        SigningKeyPair::public_key_of(server_seed),
        now - 86_400,
        now + 30 * 86_400,
    );
    let server = SigningKeyPair::new(server_seed, cert).unwrap();
    let trust = TrustStore::new([root.public()]);
    let pg = pg.map(str::to_owned);
    let (svc, issuer, google) = rt.block_on(async move {
        let google = FakeGoogle::start().await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let issuer = format!("http://{}", listener.local_addr().unwrap());
        let mut cfg = Config::dev(&issuer);
        cfg.google = Some(google.config());
        cfg.rate_limit.per_minute = 10_000;
        if let Some(url) = pg {
            cfg.storage.kind = StorageKind::Postgres;
            cfg.storage.url = Some(url);
        }
        let svc = tokio::task::spawn_blocking(move || {
            AuthService::with_clock(cfg, Arc::new(move || c.load(Ordering::SeqCst)))
                .unwrap()
                .with_hearts_trust(trust)
        })
        .await
        .unwrap();
        let svc = Arc::new(svc);
        let s = svc.clone();
        tokio::spawn(async move { ocw_accounts::serve(s, listener).await });
        (svc, issuer, google)
    });
    Env {
        _rt: rt,
        svc,
        issuer,
        google,
        clock,
        browser: TestBrowser::new(),
        root,
        server,
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into()
}

static SUBJECT: AtomicU32 = AtomicU32::new(1);

impl Env {
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.issuer)
    }

    fn now(&self) -> i64 {
        self.clock.load(Ordering::SeqCst)
    }

    /// A new account (a fresh fake Google user) and its access token.
    fn sign_in(&self, name: &str) -> String {
        let n = SUBJECT.fetch_add(1, Ordering::SeqCst);
        let sub = format!("ws-{}-{n}-{}", std::process::id(), real_now());
        self.google
            .set_user(&sub, name, &format!("rider{n}@example.org"));
        let mut r = [0u8; 96];
        getrandom::fill(&mut r).unwrap();
        let p = PendingLogin::new(&r, APP, Some(Provider::Google));
        let url = p.authorize_url(&self.url("/authorize"), CLIENT_DESKTOP);
        let redirect = self.browser.open_until(&url, APP).unwrap();
        let target = redirect.strip_prefix("http://127.0.0.1:5999").unwrap();
        let Callback::Code(code) =
            client::parse_callback(target, client::LOOPBACK_PATH, &p, &self.issuer)
        else {
            panic!("no code: {redirect}");
        };
        let (s, b) = self.post_form("/token", &p.code_form(CLIENT_DESKTOP, &code));
        let t = client::parse_token_response(s, &b).unwrap();
        Session::from_response(
            &t,
            self.now(),
            &self.issuer,
            CLIENT_DESKTOP,
            Some(&p.nonce),
            None,
        )
        .unwrap()
        .access_token
    }

    fn post_form(&self, path: &str, body: &str) -> (u16, String) {
        let mut r = agent()
            .post(&self.url(path))
            .header("content-type", "application/x-www-form-urlencoded")
            .send(body)
            .unwrap();
        (r.status().as_u16(), r.body_mut().read_to_string().unwrap())
    }

    fn get(&self, path: &str, token: Option<&str>) -> (u16, String) {
        let mut req = agent().get(&self.url(path));
        if let Some(t) = token {
            req = req.header("authorization", &format!("Bearer {t}"));
        }
        let mut r = req.call().unwrap();
        (r.status().as_u16(), r.body_mut().read_to_string().unwrap())
    }

    fn post(
        &self,
        path: &str,
        token: Option<&str>,
        key: Option<&str>,
        body: &str,
    ) -> (u16, String) {
        let mut req = agent()
            .post(&self.url(path))
            .header("content-type", "application/json");
        if let Some(t) = token {
            req = req.header("authorization", &format!("Bearer {t}"));
        }
        if let Some(k) = key {
            req = req.header("idempotency-key", k);
        }
        let mut r = req.send(body).unwrap();
        (r.status().as_u16(), r.body_mut().read_to_string().unwrap())
    }

    fn state(&self, token: &str) -> WorkshopState {
        let (s, b) = self.get("/workshop", Some(token));
        assert_eq!(s, 200, "{b}");
        serde_json::from_str(&b).unwrap()
    }

    fn act(&self, token: &str, key: Option<&str>, a: &Action) -> (u16, String) {
        let path = a.endpoint().path();
        self.post(path, Some(token), key, &serde_json::to_string(a).unwrap())
    }

    fn act_ok(&self, token: &str, key: &str, a: &Action) -> ActionAnswer {
        let (s, b) = self.act(token, Some(key), a);
        assert_eq!(s, 200, "{b}");
        serde_json::from_str(&b).unwrap()
    }

    fn account_of(&self, token: &str) -> String {
        self.state(token).account
    }

    /// The ledger's sum equals the cached balance; never negative.
    fn assert_books(&self, token: &str) {
        let st = self.state(token);
        let sum = self.svc.workshop_store().ledger_sum(&st.account).unwrap();
        assert_eq!(sum, st.hearts, "ledger sum vs balance");
        assert!(st.hearts >= 0);
    }

    fn credit(&self, c: &RideCredit, key: &SigningKeyPair) -> (u16, String) {
        let doc = key.sign_document(RIDE_CREDIT_CONTEXT, c);
        self.post(
            "/rides/credit",
            None,
            None,
            &serde_json::to_string(&doc).unwrap(),
        )
    }

    fn summary(&self, token: &str, s: &RideSummary) -> (u16, String) {
        self.post(
            "/rides/summary",
            Some(token),
            None,
            &serde_json::to_string(s).unwrap(),
        )
    }
}

/// A plausible hour on a trainer.
fn hour_ride(id: &str, start: i64) -> RideSummary {
    let n = 3600;
    let power: Vec<u16> = (0..n)
        .map(|i| (200.0 + 70.0 * ((i as f64) / 90.0).sin()) as u16)
        .collect();
    RideSummary {
        ride_id: id.into(),
        started_unix_s: start,
        duration_s: n as u32,
        distance_m: 30_000.0,
        climb_m: 300.0,
        work_j: power.iter().map(|p| f64::from(*p)).sum(),
        weight_kg: 72.0,
        bike: None,
        devices: vec![Device {
            kind: DeviceKind::Trainer,
            name: "Test trainer".into(),
        }],
        cadence_rpm: (0..n).map(|i| 85 + (i % 11) as u16).collect(),
        heart_bpm: power.iter().map(|p| 90 + p / 4).collect(),
        power_w: power,
    }
}

fn credit_for(account: &str, ride: &str, start: i64, now: i64) -> RideCredit {
    let work_j = 600_000.0;
    let climb_m = 200.0;
    RideCredit {
        kind: RIDE_CREDIT_KIND.into(),
        account: account.into(),
        ride_id: ride.into(),
        started_unix_s: start,
        ended_unix_s: start + 3000,
        distance_m: 25_000.0,
        work_j,
        climb_m,
        hearts: ocw_core::hearts::earned(work_j, climb_m),
        bike: None,
        issued_unix_s: now,
    }
}

// ---- the scenarios --------------------------------------------------------

fn start_balance_and_base_parts(e: &Env) {
    let t = e.sign_in("Ada Rider");
    let st = e.state(&t);
    assert_eq!(st.hearts, START_HEARTS);
    assert_eq!(st.ledger.len(), 1);
    assert_eq!(st.ledger[0].reason, Reason::Start);
    assert_eq!(st.ledger[0].delta, START_HEARTS);
    // Nothing owned: the starter bike with base parts, the base kit.
    assert_eq!(st.garage.bikes.len(), 1);
    assert!(st.garage.bikes[0].starter && st.garage.bikes[0].parts.is_empty());
    assert!(st.garage.items.is_empty() && st.garage.kit.is_empty());
    assert_eq!(st.garage.rider, "Ada Rider");
    // The start is booked once.
    let again = e.state(&t);
    assert_eq!((again.hearts, again.ledger.len()), (START_HEARTS, 1));
    e.assert_books(&t);
    // A frameset comes with its base parts: one item, a complete bike.
    let a = e.act_ok(
        &t,
        "k-bike",
        &Action::BuyBike {
            frame: "frame.road.upper".into(),
            with: vec![],
        },
    );
    let bike = a.bike.unwrap();
    assert_eq!(a.hearts, START_HEARTS - 2839);
    let b = a.garage.bikes.iter().find(|b| b.id == bike).unwrap();
    assert_eq!(b.parts.len(), 1);
    // No bearer, no workshop.
    assert_eq!(e.get("/workshop", None).0, 401);
}

fn buying_selling_and_idempotency(e: &Env) {
    let t = e.sign_in("Bea Climber");
    let buy = Action::BuyBike {
        frame: "frame.road.mid".into(),
        with: vec!["wheels.road-alloy.entry".into()],
    };
    // A purchase needs a key.
    assert_eq!(e.act(&t, None, &buy).0, 400);
    let a = e.act_ok(&t, "k1", &buy);
    assert_eq!(a.paid, 1390 + 299);
    assert_eq!(a.hearts, START_HEARTS - 1689);
    // The same request again: the first answer, nothing booked twice.
    let again = e.act_ok(&t, "k1", &buy);
    assert_eq!(again.hearts, a.hearts);
    assert_eq!(again.bike, a.bike);
    let st = e.state(&t);
    assert_eq!(st.hearts, START_HEARTS - 1689);
    assert_eq!(st.garage.bikes.len(), 2);
    let purchases = st
        .ledger
        .iter()
        .filter(|l| l.reason == Reason::Purchase)
        .count();
    assert_eq!(purchases, 2, "frame and wheels, once");
    // The key with another request is refused.
    let other = Action::BuyPart {
        bike: STARTER_BIKE.into(),
        part: "tyres.road.race".into(),
    };
    assert_eq!(e.act(&t, Some("k1"), &other).0, 422);
    // Wrong endpoint.
    let (s, _) = e.post(
        "/workshop/sell",
        Some(&t),
        Some("k2"),
        &serde_json::to_string(&other).unwrap(),
    );
    assert_eq!(s, 400);
    // Kit: worn at once.
    let k = e.act_ok(
        &t,
        "k3",
        &Action::BuyPart {
            bike: STARTER_BIKE.into(),
            part: "helmet.aero-road.top".into(),
        },
    );
    assert!(k.garage.kit.contains_key("helmet"));
    // Select and paint (no key needed, no hearts).
    let bike = a.bike.clone().unwrap();
    let (s, b) = e.act(&t, None, &Action::Select { bike: bike.clone() });
    assert_eq!(s, 200, "{b}");
    // Sell the new bike, unridden: everything back.
    let sold = e.act_ok(&t, "k4", &Action::SellBike { bike: bike.clone() });
    assert_eq!(sold.refund, 1689);
    assert_eq!(sold.garage.active, STARTER_BIKE);
    assert_eq!(sold.hearts, START_HEARTS - 268);
    // Selling it again: not found, nothing changes.
    assert_eq!(e.act(&t, Some("k5"), &Action::SellBike { bike }).0, 404);
    // The starter bike is not for sale.
    assert_eq!(
        e.act(
            &t,
            Some("k6"),
            &Action::SellBike {
                bike: STARTER_BIKE.into()
            }
        )
        .0,
        422
    );
    e.assert_books(&t);
}

fn balance_never_negative(e: &Env) {
    let t = e.sign_in("Cleo Sprinter");
    let (s, b) = e.act(
        &t,
        Some("too-much"),
        &Action::BuyBike {
            frame: "frame.tt.top".into(),
            with: vec![],
        },
    );
    assert_eq!(s, 409, "{b}");
    assert!(b.contains("insufficient_hearts"));
    assert_eq!(e.state(&t).hearts, START_HEARTS);
    // Eight purchases at once of 1,390 ♥ each: three fit in 5,000 ♥.
    let ok = std::thread::scope(|sc| {
        let hs: Vec<_> = (0..8)
            .map(|i| {
                let t = t.clone();
                sc.spawn(move || {
                    e.act(
                        &t,
                        Some(&format!("race-{i}")),
                        &Action::BuyBike {
                            frame: "frame.road.mid".into(),
                            with: vec![],
                        },
                    )
                    .0
                })
            })
            .collect();
        hs.into_iter()
            .map(|h| h.join().unwrap())
            .filter(|s| *s == 200)
            .count()
    });
    assert_eq!(ok, 3);
    assert_eq!(e.state(&t).hearts, START_HEARTS - 3 * 1390);
    e.assert_books(&t);
}

fn signed_server_credits(e: &Env) {
    let t = e.sign_in("Dana Tempo");
    let account = e.account_of(&t);
    let now = e.now();
    let c = credit_for(&account, "ride-1", now - 3600, now);
    let (s, b) = e.credit(&c, &e.server);
    assert_eq!(s, 200, "{b}");
    let a: RideAnswer = serde_json::from_str(&b).unwrap();
    assert_eq!(a.status, RideStatus::Credited);
    assert_eq!(a.hearts, c.hearts);
    assert_eq!(a.balance, START_HEARTS + i64::from(c.hearts));
    // Sent again: the first answer, credited once.
    let (s, b) = e.credit(&c, &e.server);
    assert_eq!(s, 200, "{b}");
    let r: RideAnswer = serde_json::from_str(&b).unwrap();
    assert!(r.repeated);
    assert_eq!(e.state(&t).hearts, START_HEARTS + i64::from(c.hearts));
    // A server whose key no trusted root certified: refused.
    let rogue_root = RootKeyPair::from_seed("Rogue root", seed());
    let rs = seed();
    let rogue = SigningKeyPair::new(
        rs,
        rogue_root.certify(
            "rogue",
            SigningKeyPair::public_key_of(rs),
            now - 10,
            now + 86_400,
        ),
    )
    .unwrap();
    let (s, b) = e.credit(&credit_for(&account, "ride-2", now - 7200, now), &rogue);
    assert_eq!(s, 403, "{b}");
    assert!(b.contains("untrusted_server"));
    // Unsigned / tampered: refused.
    let doc = e.server.sign_document(
        RIDE_CREDIT_CONTEXT,
        &credit_for(&account, "ride-3", now - 9000, now),
    );
    let tampered = serde_json::to_string(&doc)
        .unwrap()
        .replace("\"ride-3\"", "\"ride-4\"");
    assert_eq!(e.post("/rides/credit", None, None, &tampered).0, 403);
    assert_eq!(e.post("/rides/credit", None, None, "{}").0, 400);
    // Hearts the work does not earn: refused.
    let mut greedy = credit_for(&account, "ride-5", now - 12_000, now);
    greedy.hearts += 100;
    assert_eq!(e.credit(&greedy, &e.server).0, 422);
    // Another root's server and the trusted one: the trusted one counts.
    assert!(e.root.public().key_id != rogue_root.public().key_id);
    // An unknown account.
    assert_eq!(
        e.credit(
            &credit_for("no-such-account", "r", now - 100, now),
            &e.server
        )
        .0,
        404
    );
    assert_eq!(e.state(&t).hearts, START_HEARTS + i64::from(c.hearts));
    e.assert_books(&t);
}

fn summaries_and_plausibility(e: &Env) {
    let t = e.sign_in("Eli Rouleur");
    let now = e.now();
    // A plausible hour: credited.
    let s1 = hour_ride("app-ride-1", now - 4 * 3600);
    let (st, b) = e.summary(&t, &s1);
    assert_eq!(st, 200, "{b}");
    let a: RideAnswer = serde_json::from_str(&b).unwrap();
    assert_eq!(a.status, RideStatus::Credited, "{a:?}");
    let earned = ocw_core::hearts::earned(s1.work_j, s1.climb_m);
    assert_eq!(a.hearts, earned);
    // A power spike: held for review, no hearts.
    let mut s2 = hour_ride("app-ride-2", now - 8 * 3600);
    s2.power_w[100] = 2400;
    s2.work_j = s2.power_w.iter().map(|p| f64::from(*p)).sum();
    let a: RideAnswer = serde_json::from_str(&e.summary(&t, &s2).1).unwrap();
    assert_eq!(a.status, RideStatus::Flagged);
    assert_eq!(a.hearts, 0);
    assert!(!a.reasons.is_empty());
    // Virtual power: refused.
    let mut s3 = hour_ride("app-ride-3", now - 12 * 3600);
    s3.devices[0].kind = DeviceKind::Virtual;
    let a: RideAnswer = serde_json::from_str(&e.summary(&t, &s3).1).unwrap();
    assert_eq!(a.status, RideStatus::Refused);
    // The same hour sent under another id: overlaps, refused.
    let mut s4 = s1.clone();
    s4.ride_id = "app-ride-1-again".into();
    let a: RideAnswer = serde_json::from_str(&e.summary(&t, &s4).1).unwrap();
    assert_eq!(a.status, RideStatus::Refused);
    assert!(a.reasons.iter().any(|r| r.contains("overlaps")), "{a:?}");
    // The first summary again: answered before, not credited twice.
    let a: RideAnswer = serde_json::from_str(&e.summary(&t, &s1).1).unwrap();
    assert!(a.repeated);
    assert_eq!(e.state(&t).hearts, START_HEARTS + i64::from(earned));
    // Signed in only; a malformed body.
    assert_eq!(e.post("/rides/summary", None, None, "{}").0, 401);
    assert_eq!(e.post("/rides/summary", Some(&t), None, "{").0, 400);
    // The ride made the starter's parts ridden: a part bought and ridden
    // sells for half.
    let w = e.act_ok(
        &t,
        "w",
        &Action::BuyPart {
            bike: STARTER_BIKE.into(),
            part: "wheels.road-alloy.entry".into(),
        },
    );
    let mut s5 = hour_ride("app-ride-5", now - 2 * 3600 + 1);
    s5.duration_s = 1200;
    s5.power_w.truncate(1200);
    s5.cadence_rpm.truncate(1200);
    s5.heart_bpm.truncate(1200);
    s5.work_j = s5.power_w.iter().map(|p| f64::from(*p)).sum();
    s5.distance_m = 10_000.0;
    s5.climb_m = 50.0;
    let a: RideAnswer = serde_json::from_str(&e.summary(&t, &s5).1).unwrap();
    assert_eq!(a.status, RideStatus::Credited, "{a:?}");
    let sold = e.act_ok(
        &t,
        "sw",
        &Action::Sell {
            item: w.item.unwrap(),
        },
    );
    assert_eq!(sold.refund, 299 / 2);
    e.assert_books(&t);
}

fn offline_queue_uploads_once(e: &Env) {
    let t = e.sign_in("Fay Descender");
    let now = e.now();
    let mut q = OfflineQueue::default();
    q.push(hour_ride("offline-1", now - 30 * 3600));
    q.push(hour_ride("offline-2", now - 26 * 3600));
    let online = std::sync::atomic::AtomicBool::new(false);
    let send = |s: &RideSummary| -> Result<RideAnswer, UploadError> {
        if !online.load(Ordering::SeqCst) {
            return Err(UploadError::Offline("no network".into()));
        }
        let (st, b) = e.summary(&t, s);
        match st {
            200 => Ok(serde_json::from_str(&b).unwrap()),
            401 => Err(UploadError::SignedOut),
            s if s >= 500 => Err(UploadError::Offline(b)),
            _ => Err(UploadError::Rejected(b)),
        }
    };
    // Offline: nothing leaves the queue, nothing is credited.
    let r = q.upload(send);
    assert_eq!((r.kept, r.answers.len()), (2, 0));
    assert_eq!(e.state(&t).hearts, START_HEARTS);
    // Online: both credited, the queue empty.
    online.store(true, Ordering::SeqCst);
    let r = q.upload(send);
    assert_eq!(r.answers.len(), 2);
    assert!(r.answers.iter().all(|a| a.status == RideStatus::Credited));
    assert!(q.is_empty());
    let after = e.state(&t).hearts;
    let each = ocw_core::hearts::earned(hour_ride("x", 0).work_j, hour_ride("x", 0).climb_m);
    assert_eq!(after, START_HEARTS + 2 * i64::from(each));
    // The answer got lost and the app sends the ride again: no change.
    q.push(hour_ride("offline-1", now - 30 * 3600));
    let r = q.upload(send);
    assert!(r.answers[0].repeated);
    assert_eq!(e.state(&t).hearts, after);
    e.assert_books(&t);
}

fn export_and_deletion(e: &Env) {
    let t = e.sign_in("Gil Puncheur");
    e.act_ok(
        &t,
        "x",
        &Action::BuyPart {
            bike: STARTER_BIKE.into(),
            part: "socks.tall.entry".into(),
        },
    );
    let (s, b) = e.get("/account/export", Some(&t));
    assert_eq!(s, 200, "{b}");
    let v: serde_json::Value = serde_json::from_str(&b).unwrap();
    assert_eq!(v["modules"]["workshop"]["hearts"], START_HEARTS - 20);
    let account = e.account_of(&t);
    let mut r = agent()
        .delete(&e.url("/account"))
        .header("authorization", &format!("Bearer {t}"))
        .call()
        .unwrap();
    assert_eq!(
        r.status().as_u16(),
        204,
        "{}",
        r.body_mut().read_to_string().unwrap()
    );
    assert!(
        e.svc
            .workshop_store()
            .ledger(&account, 10)
            .unwrap()
            .is_empty()
    );
}

impl Env {
    fn put(&self, path: &str, token: &str, body: &str) -> (u16, String) {
        let mut r = agent()
            .put(&self.url(path))
            .header("content-type", "application/json")
            .header("authorization", &format!("Bearer {token}"))
            .send(body)
            .unwrap();
        (r.status().as_u16(), r.body_mut().read_to_string().unwrap())
    }
}

/// The ride's damage kept with the account and repaired over HTTP (4O
/// part 4): `/workshop/garage` keeps it, `/workshop/repair` charges the
/// local rule's price, once per idempotency key.
fn damage_and_repairs(e: &Env) {
    let t = e.sign_in("Hal Headwind");
    let damage = Action::Damage {
        bike: STARTER_BIKE.into(),
        condition: [("rear_wheel".to_string(), 500)].into(),
        injury: Some(ocw_account_api::workshop::Injury {
            condition_pm: 800,
            at_unix: e.now(),
        }),
    };
    e.act_ok(&t, "d1", &damage);
    let st = e.state(&t);
    let bike = st.garage.bike(STARTER_BIKE).unwrap();
    assert_eq!(bike.condition.get("rear_wheel"), Some(&500));
    assert_eq!(st.garage.injury.map(|i| i.condition_pm), Some(800));
    let repair = Action::Repair {
        bike: STARTER_BIKE.into(),
        part: Some("rear_wheel".into()),
    };
    let (s, _) = e.act(&t, None, &repair);
    assert_eq!(s, 400, "a repair needs an idempotency key");
    let a = e.act_ok(&t, "r1", &repair);
    assert!(
        a.paid > 0 && a.hearts == START_HEARTS - i64::from(a.paid),
        "{a:?}"
    );
    // The same key again: the first answer, nothing charged twice.
    let again = e.act_ok(&t, "r1", &repair);
    assert_eq!(again.hearts, a.hearts);
    let st = e.state(&t);
    assert!(st.garage.bike(STARTER_BIKE).unwrap().condition.is_empty());
    assert!(st.ledger.iter().any(|l| l.reason == Reason::Repair));
    let (s, b) = e.act(&t, Some("r2"), &repair);
    assert!(s >= 400 && b.contains("needs no repair"), "{s} {b}");
    e.assert_books(&t);
}

/// The rider of an account (4O W5b): none at first, kept with its name
/// checked and unique on the backend, the account's name follows it (the
/// next token carries it), in the export, gone with the account.
fn rider_of_the_account(e: &Env) {
    let t = e.sign_in("Ines Echelon");
    let (s, _) = e.get("/account/rider", Some(&t));
    assert_eq!(s, 404, "no rider yet");
    let doc = |name: &str| {
        serde_json::json!({
            "name": name,
            "rider": {"name": "old", "weight_kg": 61.5, "height_cm": 168.0, "skin": 4},
        })
        .to_string()
    };
    let name = format!("Ines E{}", SUBJECT.load(Ordering::SeqCst));
    let (s, b) = e.put("/account/rider", &t, &doc(&format!("  {name} ")));
    assert_eq!(s, 200, "{b}");
    let kept: ocw_account_api::rider::RiderDoc = serde_json::from_str(&b).unwrap();
    assert_eq!(kept.name, name, "trimmed");
    assert_eq!(kept.rider["name"], name.as_str());
    assert_eq!(kept.rider["skin"], 4);
    let (s, b) = e.get("/account/rider", Some(&t));
    assert_eq!(s, 200, "{b}");
    assert!(b.contains("61.5"));
    let account = e.account_of(&t);
    assert_eq!(
        e.svc.store().account(&account).unwrap().display_name,
        name,
        "the account's name follows"
    );
    // Another account cannot take it (any case); a bad name is refused.
    let t2 = e.sign_in("Jun Breakaway");
    let (s, b) = e.put("/account/rider", &t2, &doc(&name.to_uppercase()));
    assert_eq!(s, 409, "{b}");
    assert!(b.contains("taken"));
    let (s, _) = e.put("/account/rider", &t2, &doc("x"));
    assert_eq!(s, 400, "too short");
    let (s, _) = e.put("/account/rider", &t2, "[1]");
    assert_eq!(s, 400);
    // In the export; gone with the account (the name free again).
    let (s, b) = e.get("/account/export", Some(&t));
    assert_eq!(s, 200);
    let v: serde_json::Value = serde_json::from_str(&b).unwrap();
    assert_eq!(v["modules"]["rider"]["name"], name.as_str());
    let r = agent()
        .delete(&e.url("/account"))
        .header("authorization", &format!("Bearer {t}"))
        .call()
        .unwrap();
    assert_eq!(r.status().as_u16(), 204);
    let (s, b) = e.put("/account/rider", &t2, &doc(&name));
    assert_eq!(s, 200, "{b}");
}

// ---- memory and PostgreSQL ------------------------------------------------

fn pg_url() -> Option<String> {
    match std::env::var("OCW_TEST_DATABASE_URL") {
        Ok(u) if !u.is_empty() => Some(u),
        _ => {
            assert!(
                std::env::var("OCW_REQUIRE_POSTGRES").as_deref() != Ok("1"),
                "OCW_REQUIRE_POSTGRES=1 but no OCW_TEST_DATABASE_URL"
            );
            eprintln!("skipped: no OCW_TEST_DATABASE_URL (make test-accounts)");
            None
        }
    }
}

macro_rules! both {
    ($($name:ident),* $(,)?) => {
        mod memory {
            $(#[test] fn $name() { let e = super::env(None); super::$name(&e); })*
        }
        mod postgres {
            $(#[test] fn $name() {
                let Some(url) = super::pg_url() else { return };
                let e = super::env(Some(&url));
                super::$name(&e);
            })*
        }
    };
}

both!(
    start_balance_and_base_parts,
    buying_selling_and_idempotency,
    balance_never_negative,
    signed_server_credits,
    summaries_and_plausibility,
    offline_queue_uploads_once,
    export_and_deletion,
    damage_and_repairs,
    rider_of_the_account,
);
