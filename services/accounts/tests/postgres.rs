//! The PostgreSQL store itself (spec 14 Phase 4, 4O W5a): the account
//! rules of the `Store` trait as the memory store keeps them, and the
//! database's own guards – the balance never below zero, the ledger
//! append-only, a ride credited once. Needs `OCW_TEST_DATABASE_URL`
//! (`make test-accounts`); skipped without it unless
//! `OCW_REQUIRE_POSTGRES=1`.

use ocw_accounts::pg::PgStore;
use ocw_accounts::store::{Account, Identity, RefreshRecord, SealedSecret, Store, StoreError};
use ocw_auth::Provider;

fn store() -> Option<PgStore> {
    match std::env::var("OCW_TEST_DATABASE_URL") {
        Ok(u) if !u.is_empty() => Some(PgStore::connect(&u, 4).unwrap()),
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

/// Ids unique per test run (the database is shared by the tests).
fn id(s: &str) -> String {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{s}-{}-{t}", std::process::id())
}

fn ident(p: Provider, sub: &str) -> Identity {
    Identity {
        provider: p,
        subject: sub.into(),
        name: format!("Hana Hill {p}"),
        email: Some("hana@example.org".into()),
        linked_at: 1,
        upload: p == Provider::Strava,
    }
}

fn account(id: &str, i: Identity) -> Account {
    Account {
        id: id.into(),
        display_name: "Hana Hill".into(),
        created_at: 1,
        identities: vec![i],
    }
}

fn sql(s: &PgStore, q: &'static str, account: &str) -> Result<u64, String> {
    let pool = s.pool();
    let account = account.to_owned();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async move {
        sqlx::query(q)
            .bind(&account)
            .execute(&pool)
            .await
            .map(|r| r.rows_affected())
            .map_err(|e| e.to_string())
    })
}

#[test]
fn linking_rules_as_the_memory_store() {
    let Some(s) = store() else { return };
    let (a, b, c) = (id("a"), id("b"), id("c"));
    let (g1, g9, s1, s2) = (id("g1"), id("g9"), id("s1"), id("s2"));
    s.create_account(account(&a, ident(Provider::Google, &g1)))
        .unwrap();
    s.create_account(account(&b, ident(Provider::Strava, &s1)))
        .unwrap();
    assert_eq!(s.find_identity(Provider::Google, &g1), Some(a.clone()));
    assert_eq!(
        s.unlink(&a, Provider::Google),
        Err(StoreError::LastIdentity(Provider::Google))
    );
    assert_eq!(
        s.link(&a, ident(Provider::Strava, &s1)),
        Err(StoreError::LinkedElsewhere(Provider::Strava))
    );
    s.link(&a, ident(Provider::Strava, &s2)).unwrap();
    let acc = s.account(&a).unwrap();
    assert_eq!(acc.identities.len(), 2);
    assert_eq!(acc.identities[0].provider, Provider::Google);
    assert_eq!(acc.identities[0].email.as_deref(), Some("hana@example.org"));
    s.unlink(&a, Provider::Google).unwrap();
    assert_eq!(s.find_identity(Provider::Google, &g1), None);
    assert_eq!(
        s.unlink(&a, Provider::Google),
        Err(StoreError::NotLinked(Provider::Google))
    );
    s.link(&a, ident(Provider::Google, &g1)).unwrap();
    s.link(&a, ident(Provider::Google, &g9)).unwrap();
    assert_eq!(s.find_identity(Provider::Google, &g1), None);
    assert_eq!(s.find_identity(Provider::Google, &g9), Some(a.clone()));
    assert!(
        s.create_account(account(&c, ident(Provider::Google, &g9)))
            .is_err()
    );
    assert_eq!(s.account(&c), None);
    assert_eq!(
        s.link(&id("nobody"), ident(Provider::Google, &id("g"))),
        Err(StoreError::NoAccount)
    );
}

#[test]
fn tokens_secrets_and_deletion() {
    let Some(s) = store() else { return };
    let a = id("acc");
    s.create_account(account(&a, ident(Provider::Strava, &id("st"))))
        .unwrap();
    let rec = RefreshRecord {
        hash: id("h"),
        family: id("f"),
        account: a.clone(),
        client_id: "ocw-desktop".into(),
        scope: "openid".into(),
        auth_time: 5,
        family_created: 5,
        created_at: 10,
        used: false,
        revoked: false,
    };
    s.put_refresh(rec.clone()).unwrap();
    assert_eq!(s.refresh(&rec.hash), Some(rec.clone()));
    // Rotation: the first use sees it unused, the second used.
    let first = s.use_refresh(&rec.hash).unwrap().unwrap();
    assert!(!first.used);
    assert!(s.use_refresh(&rec.hash).unwrap().unwrap().used);
    assert_eq!(s.use_refresh("no-such-hash").unwrap(), None);
    s.revoke_family(&rec.family).unwrap();
    assert!(s.refresh(&rec.hash).unwrap().revoked);
    s.put_secret(
        &a,
        Provider::Strava,
        SealedSecret {
            nonce: "n".into(),
            ciphertext: "c".into(),
        },
    )
    .unwrap();
    assert!(s.secret(&a, Provider::Strava).is_some());
    s.purge_refresh(11).unwrap();
    assert_eq!(s.refresh(&rec.hash), None);
    s.delete_account(&a).unwrap();
    assert_eq!(s.account(&a), None);
    assert!(s.secret(&a, Provider::Strava).is_none());
    assert_eq!(s.delete_account(&a), Err(StoreError::NoAccount));
}

#[test]
fn the_database_guards_the_hearts() {
    let Some(s) = store() else { return };
    let a = id("guard");
    s.create_account(account(&a, ident(Provider::Google, &id("gg"))))
        .unwrap();
    // The balance never below zero, even written by hand.
    let e = sql(&s, "UPDATE accounts SET hearts = -1 WHERE id = $1", &a).unwrap_err();
    assert!(e.contains("hearts_never_negative"), "{e}");
    // The ledger is append-only.
    sql(
        &s,
        "INSERT INTO hearts_ledger (account, delta, reason, reference, at) VALUES ($1, 5, 'ride', 'r1', 0)",
        &a,
    )
    .unwrap();
    let e = sql(
        &s,
        "UPDATE hearts_ledger SET delta = 500 WHERE account = $1",
        &a,
    )
    .unwrap_err();
    assert!(e.contains("append-only"), "{e}");
    // A ride once; a start once; only known reasons.
    assert!(
        sql(
            &s,
            "INSERT INTO hearts_ledger (account, delta, reason, reference, at) VALUES ($1, 5, 'ride', 'r1', 0)",
            &a,
        )
        .is_err()
    );
    sql(
        &s,
        "INSERT INTO hearts_ledger (account, delta, reason, reference, at) VALUES ($1, 5000, 'start', '', 0)",
        &a,
    )
    .unwrap();
    assert!(
        sql(
            &s,
            "INSERT INTO hearts_ledger (account, delta, reason, reference, at) VALUES ($1, 5000, 'start', '', 0)",
            &a,
        )
        .is_err()
    );
    assert!(
        sql(
            &s,
            "INSERT INTO hearts_ledger (account, delta, reason, reference, at) VALUES ($1, 1, 'gift', '', 0)",
            &a,
        )
        .is_err()
    );
    // Deleting the account takes its ledger along.
    s.delete_account(&a).unwrap();
    assert_eq!(
        sql(&s, "DELETE FROM hearts_ledger WHERE account = $1", &a).unwrap(),
        0
    );
}
