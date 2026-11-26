//! A certified game server credits a ride on the accounts backend (spec
//! 14 Phase 4, 4O Q19 (a)), end to end on the loopback interface: the
//! server's credit sender signs and posts, the backend checks the key chain
//! and books the hearts once; a server whose key no trusted root certified
//! credits nothing.
#![cfg(feature = "backend")]

use std::sync::Arc;
use std::time::Duration;

use ocw_accounts::AuthService;
use ocw_accounts::config::Config;
use ocw_accounts::store::{Account, Identity};
use ocw_auth::Provider;
use ocw_server::credits::BackendCredits;
use ocw_server::{CreditSink, EndedRide};
use ocw_world::manifest::{RootKeyPair, SigningKeyPair, TrustStore};

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn signing(root: &RootKeyPair, seed: u8) -> SigningKeyPair {
    let s = [seed; 32];
    let cert = root.certify(
        "test server",
        SigningKeyPair::public_key_of(s),
        now() - 3600,
        now() + 86_400,
    );
    SigningKeyPair::new(s, cert).unwrap()
}

#[test]
fn a_certified_server_credits_once_a_rogue_one_never() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let root = RootKeyPair::from_seed("Test project root", [3; 32]);
    let trust = TrustStore::new([root.public()]);
    let (svc, issuer) = rt.block_on(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let issuer = format!("http://{}", listener.local_addr().unwrap());
        let svc = Arc::new(
            AuthService::new(Config::dev(&issuer))
                .unwrap()
                .with_hearts_trust(trust),
        );
        let s = svc.clone();
        tokio::spawn(async move { ocw_accounts::serve(s, listener).await });
        (svc, issuer)
    });
    svc.store()
        .create_account(Account {
            id: "acc-ida".into(),
            display_name: "Ida Uphill".into(),
            created_at: now(),
            identities: vec![Identity {
                provider: Provider::Google,
                subject: "g-ida".into(),
                name: "Ida Uphill".into(),
                email: None,
                linked_at: now(),
                upload: false,
            }],
        })
        .unwrap();
    let ride = |id: &str| EndedRide {
        account_id: "acc-ida".into(),
        ride_id: id.into(),
        started_unix_s: now() - 3600,
        ended_unix_s: now() - 10,
        distance_m: 20_000.0,
        work_j: 500_000.0,
        climb_m: 100.0,
    };
    let good =
        BackendCredits::start_with(signing(&root, 4), issuer.clone(), Duration::from_millis(50))
            .unwrap();
    good.ride_ended(ride("r1"));
    // The same ride again (a retry after a lost answer): booked once.
    good.ride_ended(ride("r1"));
    let rogue_root = RootKeyPair::from_seed("Rogue", [8; 32]);
    let rogue =
        BackendCredits::start_with(signing(&rogue_root, 9), issuer, Duration::from_millis(50))
            .unwrap();
    let mut r2 = ride("r2");
    r2.started_unix_s -= 7200;
    r2.ended_unix_s -= 7200;
    rogue.ride_ended(r2);
    let ledger = || svc.workshop_store().ledger("acc-ida", 50).unwrap();
    let earned = i64::from(ocw_core::hearts::earned(500_000.0, 100.0));
    for _ in 0..200 {
        if ledger().iter().any(|l| l.reference.contains("r1")) {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(Duration::from_millis(300));
    let sum = svc.workshop_store().ledger_sum("acc-ida").unwrap();
    assert_eq!(
        sum,
        ocw_account_api::START_HEARTS + earned,
        "{:?}",
        ledger()
    );
    assert!(ledger().iter().all(|l| !l.reference.contains("r2")));
}
