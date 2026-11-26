//! PostgreSQL storage (spec 14 Phase 4, 4O W5a): the accounts ([`Store`])
//! and the workshop with the hearts ledger ([`WorkshopStore`]) in one
//! database, `sqlx` with the migrations in `services/accounts/migrations/`
//! (applied at connect).
//!
//! The service's flows are synchronous (they run on blocking threads or
//! briefly on the HTTP runtime), so the store owns a small Tokio runtime
//! of its own and every call waits for its query there – safe from any
//! thread, inside another runtime too.
//!
//! A workshop transaction locks the account row (`SELECT … FOR UPDATE`),
//! so the changes of one account are serialized while other accounts run
//! in parallel; the balance has a `CHECK (hearts >= 0)`, the ledger is
//! append-only (a trigger refuses updates) with a unique ride reference.

use std::collections::BTreeMap;
use std::future::Future;

use ocw_account_api::rides::RideStatus;
use ocw_account_api::workshop::{Bike, Garage, Item, LedgerEntry, Reason};
use ocw_auth::Provider;
use sqlx::postgres::{PgPool, PgPoolOptions, PgRow};
use sqlx::types::Json;
use sqlx::{Postgres, Row, Transaction};

use crate::store::{Account, Identity, RefreshRecord, SealedSecret, Store, StoreError};
use crate::workshop::ApiFail;
use crate::workshop::store::{
    Change, Done, RideRecord, StoreFail, Tx, TxError, TxFn, Wallet, WorkshopStore,
};

/// The migrations, embedded in the binary.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

pub struct PgStore {
    pool: PgPool,
    rt: Option<tokio::runtime::Runtime>,
}

impl Drop for PgStore {
    fn drop(&mut self) {
        // Safe inside another runtime too (a plain drop would panic there).
        if let Some(rt) = self.rt.take() {
            rt.shutdown_background();
        }
    }
}

fn io(e: sqlx::Error) -> StoreError {
    StoreError::Io(e.to_string())
}

fn wio(e: sqlx::Error) -> StoreFail {
    StoreFail::Io(e.to_string())
}

fn code(e: &sqlx::Error) -> Option<String> {
    e.as_database_error()
        .and_then(|d| d.code())
        .map(|c| c.into_owned())
}

fn status_id(s: RideStatus) -> &'static str {
    match s {
        RideStatus::Credited => "credited",
        RideStatus::Flagged => "flagged",
        RideStatus::Refused => "refused",
    }
}

fn status_parse(s: &str) -> RideStatus {
    match s {
        "credited" => RideStatus::Credited,
        "flagged" => RideStatus::Flagged,
        _ => RideStatus::Refused,
    }
}

impl PgStore {
    /// Connects to `url`, applies the migrations.
    pub fn connect(url: &str, max_connections: u32) -> Result<Self, String> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("ocw-postgres")
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        let url = url.to_owned();
        let pool = rt
            .block_on(async move {
                let pool = PgPoolOptions::new()
                    .max_connections(max_connections.max(1))
                    .acquire_timeout(std::time::Duration::from_secs(10))
                    .connect(&url)
                    .await?;
                MIGRATOR.run(&pool).await?;
                Ok::<_, sqlx::Error>(pool)
            })
            .map_err(|e| format!("PostgreSQL: {e}"))?;
        Ok(Self { pool, rt: Some(rt) })
    }

    /// Runs `fut` on the store's runtime and waits for it.
    fn run<T: Send + 'static>(&self, fut: impl Future<Output = T> + Send + 'static) -> T {
        let (tx, rx) = std::sync::mpsc::channel();
        self.rt.as_ref().expect("running").spawn(async move {
            let _ = tx.send(fut.await);
        });
        rx.recv().expect("the database task ends")
    }

    pub fn pool(&self) -> PgPool {
        self.pool.clone()
    }

    /// Number of accounts (tests).
    pub fn accounts(&self) -> Result<i64, StoreError> {
        let pool = self.pool.clone();
        self.run(async move {
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM accounts")
                .fetch_one(&pool)
                .await
        })
        .map_err(io)
    }
}

fn identity(r: &PgRow) -> Result<Identity, sqlx::Error> {
    let p: String = r.try_get("provider")?;
    Ok(Identity {
        provider: Provider::parse(&p)
            .ok_or_else(|| sqlx::Error::Decode(format!("provider {p}").into()))?,
        subject: r.try_get("subject")?,
        name: r.try_get("name")?,
        email: r.try_get("email")?,
        linked_at: r.try_get("linked_at")?,
        upload: r.try_get("upload")?,
    })
}

fn refresh(r: &PgRow) -> Result<RefreshRecord, sqlx::Error> {
    Ok(RefreshRecord {
        hash: r.try_get("hash")?,
        family: r.try_get("family")?,
        account: r.try_get("account")?,
        client_id: r.try_get("client_id")?,
        scope: r.try_get("scope")?,
        auth_time: r.try_get("auth_time")?,
        family_created: r.try_get("family_created")?,
        created_at: r.try_get("created_at")?,
        used: r.try_get("used")?,
        revoked: r.try_get("revoked")?,
    })
}

async fn insert_identity(
    t: &mut Transaction<'static, Postgres>,
    account: &str,
    i: &Identity,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO identities (provider, subject, account, name, email, linked_at, upload)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(i.provider.id())
    .bind(&i.subject)
    .bind(account)
    .bind(&i.name)
    .bind(&i.email)
    .bind(i.linked_at)
    .bind(i.upload)
    .execute(&mut **t)
    .await?;
    Ok(())
}

const REFRESH_COLS: &str =
    "hash, family, account, client_id, scope, auth_time, family_created, created_at, used, revoked";

impl Store for PgStore {
    fn account(&self, id: &str) -> Option<Account> {
        let pool = self.pool.clone();
        let id = id.to_owned();
        let r = self.run(async move {
            let Some(a) = sqlx::query("SELECT id, display_name, created_at FROM accounts WHERE id = $1")
                .bind(&id)
                .fetch_optional(&pool)
                .await?
            else {
                return Ok(None);
            };
            let rows = sqlx::query(
                "SELECT provider, subject, name, email, linked_at, upload FROM identities WHERE account = $1",
            )
            .bind(&id)
            .fetch_all(&pool)
            .await?;
            let mut identities = rows.iter().map(identity).collect::<Result<Vec<_>, _>>()?;
            identities.sort_by_key(|i| i.provider);
            Ok::<_, sqlx::Error>(Some(Account {
                id: a.try_get("id")?,
                display_name: a.try_get("display_name")?,
                created_at: a.try_get("created_at")?,
                identities,
            }))
        });
        r.unwrap_or_else(|e| {
            tracing::warn!(error = %e, "account lookup");
            None
        })
    }

    fn find_identity(&self, provider: Provider, subject: &str) -> Option<String> {
        let pool = self.pool.clone();
        let subject = subject.to_owned();
        self.run(async move {
            sqlx::query_scalar::<_, String>(
                "SELECT account FROM identities WHERE provider = $1 AND subject = $2",
            )
            .bind(provider.id())
            .bind(&subject)
            .fetch_optional(&pool)
            .await
        })
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "identity lookup");
            None
        })
    }

    fn create_account(&self, account: Account) -> Result<(), StoreError> {
        let pool = self.pool.clone();
        self.run(async move {
            let mut t = pool.begin().await.map_err(io)?;
            for i in &account.identities {
                let held: Option<String> = sqlx::query_scalar(
                    "SELECT account FROM identities WHERE provider = $1 AND subject = $2",
                )
                .bind(i.provider.id())
                .bind(&i.subject)
                .fetch_optional(&mut *t)
                .await
                .map_err(io)?;
                if held.is_some() {
                    return Err(StoreError::LinkedElsewhere(i.provider));
                }
            }
            sqlx::query("INSERT INTO accounts (id, display_name, created_at) VALUES ($1, $2, $3)")
                .bind(&account.id)
                .bind(&account.display_name)
                .bind(account.created_at)
                .execute(&mut *t)
                .await
                .map_err(io)?;
            for i in &account.identities {
                insert_identity(&mut t, &account.id, i).await.map_err(|e| {
                    if code(&e).as_deref() == Some("23505") {
                        StoreError::LinkedElsewhere(i.provider)
                    } else {
                        io(e)
                    }
                })?;
            }
            t.commit().await.map_err(io)
        })
    }

    fn link(&self, account: &str, identity: Identity) -> Result<(), StoreError> {
        let pool = self.pool.clone();
        let account = account.to_owned();
        self.run(async move {
            let mut t = pool.begin().await.map_err(io)?;
            let exists: Option<String> =
                sqlx::query_scalar("SELECT id FROM accounts WHERE id = $1 FOR UPDATE")
                    .bind(&account)
                    .fetch_optional(&mut *t)
                    .await
                    .map_err(io)?;
            let held: Option<String> = sqlx::query_scalar(
                "SELECT account FROM identities WHERE provider = $1 AND subject = $2",
            )
            .bind(identity.provider.id())
            .bind(&identity.subject)
            .fetch_optional(&mut *t)
            .await
            .map_err(io)?;
            if held.is_some_and(|a| a != account) {
                return Err(StoreError::LinkedElsewhere(identity.provider));
            }
            if exists.is_none() {
                return Err(StoreError::NoAccount);
            }
            sqlx::query("DELETE FROM identities WHERE account = $1 AND provider = $2")
                .bind(&account)
                .bind(identity.provider.id())
                .execute(&mut *t)
                .await
                .map_err(io)?;
            insert_identity(&mut t, &account, &identity)
                .await
                .map_err(io)?;
            t.commit().await.map_err(io)
        })
    }

    fn unlink(&self, account: &str, provider: Provider) -> Result<(), StoreError> {
        let pool = self.pool.clone();
        let account = account.to_owned();
        self.run(async move {
            let mut t = pool.begin().await.map_err(io)?;
            let exists: Option<String> =
                sqlx::query_scalar("SELECT id FROM accounts WHERE id = $1 FOR UPDATE")
                    .bind(&account)
                    .fetch_optional(&mut *t)
                    .await
                    .map_err(io)?;
            if exists.is_none() {
                return Err(StoreError::NoAccount);
            }
            let providers: Vec<String> =
                sqlx::query_scalar("SELECT provider FROM identities WHERE account = $1")
                    .bind(&account)
                    .fetch_all(&mut *t)
                    .await
                    .map_err(io)?;
            if !providers.iter().any(|p| p == provider.id()) {
                return Err(StoreError::NotLinked(provider));
            }
            if providers.len() == 1 {
                return Err(StoreError::LastIdentity(provider));
            }
            for q in [
                "DELETE FROM identities WHERE account = $1 AND provider = $2",
                "DELETE FROM provider_secrets WHERE account = $1 AND provider = $2",
            ] {
                sqlx::query(q)
                    .bind(&account)
                    .bind(provider.id())
                    .execute(&mut *t)
                    .await
                    .map_err(io)?;
            }
            t.commit().await.map_err(io)
        })
    }

    fn delete_account(&self, account: &str) -> Result<(), StoreError> {
        let pool = self.pool.clone();
        let account = account.to_owned();
        self.run(async move {
            let r = sqlx::query("DELETE FROM accounts WHERE id = $1")
                .bind(&account)
                .execute(&pool)
                .await
                .map_err(io)?;
            if r.rows_affected() == 0 {
                return Err(StoreError::NoAccount);
            }
            Ok(())
        })
    }

    fn put_refresh(&self, r: RefreshRecord) -> Result<(), StoreError> {
        let pool = self.pool.clone();
        self.run(async move {
            sqlx::query(&format!(
                "INSERT INTO refresh_tokens ({REFRESH_COLS}) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                 ON CONFLICT (hash) DO UPDATE SET family = EXCLUDED.family, account = EXCLUDED.account,
                   client_id = EXCLUDED.client_id, scope = EXCLUDED.scope, auth_time = EXCLUDED.auth_time,
                   family_created = EXCLUDED.family_created, created_at = EXCLUDED.created_at,
                   used = EXCLUDED.used, revoked = EXCLUDED.revoked"
            ))
            .bind(&r.hash)
            .bind(&r.family)
            .bind(&r.account)
            .bind(&r.client_id)
            .bind(&r.scope)
            .bind(r.auth_time)
            .bind(r.family_created)
            .bind(r.created_at)
            .bind(r.used)
            .bind(r.revoked)
            .execute(&pool)
            .await
            .map(|_| ())
            .map_err(io)
        })
    }

    fn refresh(&self, hash: &str) -> Option<RefreshRecord> {
        let pool = self.pool.clone();
        let hash = hash.to_owned();
        self.run(async move {
            let r = sqlx::query(&format!(
                "SELECT {REFRESH_COLS} FROM refresh_tokens WHERE hash = $1"
            ))
            .bind(&hash)
            .fetch_optional(&pool)
            .await?;
            r.as_ref().map(refresh).transpose()
        })
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "refresh lookup");
            None
        })
    }

    fn use_refresh(&self, hash: &str) -> Result<Option<RefreshRecord>, StoreError> {
        let pool = self.pool.clone();
        let hash = hash.to_owned();
        self.run(async move {
            // The row as it was before (atomic: the row lock makes the
            // second of two concurrent refreshes see it used).
            let cols = REFRESH_COLS
                .split(", ")
                .map(|c| format!("old.{c}"))
                .collect::<Vec<_>>()
                .join(", ");
            let r = sqlx::query(&format!(
                "WITH old AS (SELECT {REFRESH_COLS} FROM refresh_tokens WHERE hash = $1 FOR UPDATE)
                 UPDATE refresh_tokens r SET used = TRUE FROM old WHERE r.hash = old.hash
                 RETURNING {cols}"
            ))
            .bind(&hash)
            .fetch_optional(&pool)
            .await
            .map_err(io)?;
            r.as_ref().map(refresh).transpose().map_err(io)
        })
    }

    fn purge_refresh(&self, before: i64) -> Result<(), StoreError> {
        let pool = self.pool.clone();
        self.run(async move {
            sqlx::query("DELETE FROM refresh_tokens WHERE created_at < $1")
                .bind(before)
                .execute(&pool)
                .await
                .map(|_| ())
                .map_err(io)
        })
    }

    fn revoke_family(&self, family: &str) -> Result<(), StoreError> {
        let pool = self.pool.clone();
        let family = family.to_owned();
        self.run(async move {
            sqlx::query("UPDATE refresh_tokens SET revoked = TRUE WHERE family = $1")
                .bind(&family)
                .execute(&pool)
                .await
                .map(|_| ())
                .map_err(io)
        })
    }

    fn revoke_account(&self, account: &str) -> Result<(), StoreError> {
        let pool = self.pool.clone();
        let account = account.to_owned();
        self.run(async move {
            sqlx::query("UPDATE refresh_tokens SET revoked = TRUE WHERE account = $1")
                .bind(&account)
                .execute(&pool)
                .await
                .map(|_| ())
                .map_err(io)
        })
    }

    fn put_secret(
        &self,
        account: &str,
        provider: Provider,
        s: SealedSecret,
    ) -> Result<(), StoreError> {
        let pool = self.pool.clone();
        let account = account.to_owned();
        self.run(async move {
            sqlx::query(
                "INSERT INTO provider_secrets (account, provider, nonce, ciphertext) VALUES ($1, $2, $3, $4)
                 ON CONFLICT (account, provider) DO UPDATE SET nonce = EXCLUDED.nonce, ciphertext = EXCLUDED.ciphertext",
            )
            .bind(&account)
            .bind(provider.id())
            .bind(&s.nonce)
            .bind(&s.ciphertext)
            .execute(&pool)
            .await
            .map(|_| ())
            .map_err(|e| {
                if code(&e).as_deref() == Some("23503") {
                    StoreError::NoAccount
                } else {
                    io(e)
                }
            })
        })
    }

    fn secret(&self, account: &str, provider: Provider) -> Option<SealedSecret> {
        let pool = self.pool.clone();
        let account = account.to_owned();
        self.run(async move {
            let r = sqlx::query(
                "SELECT nonce, ciphertext FROM provider_secrets WHERE account = $1 AND provider = $2",
            )
            .bind(&account)
            .bind(provider.id())
            .fetch_optional(&pool)
            .await?;
            r.map(|r| {
                Ok::<_, sqlx::Error>(SealedSecret {
                    nonce: r.try_get("nonce")?,
                    ciphertext: r.try_get("ciphertext")?,
                })
            })
            .transpose()
        })
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "secret lookup");
            None
        })
    }

    fn delete_secret(&self, account: &str, provider: Provider) -> Result<(), StoreError> {
        let pool = self.pool.clone();
        let account = account.to_owned();
        self.run(async move {
            sqlx::query("DELETE FROM provider_secrets WHERE account = $1 AND provider = $2")
                .bind(&account)
                .bind(provider.id())
                .execute(&pool)
                .await
                .map(|_| ())
                .map_err(io)
        })
    }
}

/// The first step of a workshop transaction.
enum Loaded {
    NoAccount,
    Seen {
        hash: String,
        answer: serde_json::Value,
        hearts: i64,
    },
    Wallet(Transaction<'static, Postgres>, Wallet),
}

async fn load(
    pool: PgPool,
    account: String,
    key: Option<String>,
    window: Option<(i64, i64)>,
) -> Result<Loaded, sqlx::Error> {
    let mut t = pool.begin().await?;
    let Some(a) = sqlx::query(
        "SELECT display_name, hearts, workshop_started, active_bike, next_item
         FROM accounts WHERE id = $1 FOR UPDATE",
    )
    .bind(&account)
    .fetch_optional(&mut *t)
    .await?
    else {
        t.rollback().await?;
        return Ok(Loaded::NoAccount);
    };
    let hearts: i64 = a.try_get("hearts")?;
    if let Some(k) = &key
        && let Some(r) = sqlx::query(
            "SELECT request_hash, answer FROM idempotency WHERE account = $1 AND key = $2",
        )
        .bind(&account)
        .bind(k)
        .fetch_optional(&mut *t)
        .await?
    {
        let answer: Json<serde_json::Value> = r.try_get("answer")?;
        let hash = r.try_get("request_hash")?;
        t.rollback().await?;
        return Ok(Loaded::Seen {
            hash,
            answer: answer.0,
            hearts,
        });
    }
    let started: bool = a.try_get("workshop_started")?;
    let items = sqlx::query(
        "SELECT id, part, paid, bought, ridden_m, with_item FROM items WHERE account = $1 ORDER BY id",
    )
    .bind(&account)
    .fetch_all(&mut *t)
    .await?
    .iter()
    .map(|r| {
        Ok(Item {
            id: r.try_get::<i32, _>("id")? as u32,
            part: r.try_get("part")?,
            paid: r.try_get::<i32, _>("paid")? as u32,
            bought: r.try_get("bought")?,
            ridden_m: r.try_get("ridden_m")?,
            with: r.try_get::<Option<i32>, _>("with_item")?.map(|w| w as u32),
        })
    })
    .collect::<Result<Vec<_>, sqlx::Error>>()?;
    let bikes = sqlx::query(
        "SELECT id, profile, paint, starter, parts, condition FROM bikes WHERE account = $1 ORDER BY hook",
    )
    .bind(&account)
    .fetch_all(&mut *t)
    .await?
    .iter()
    .map(|r| {
        Ok(Bike {
            id: r.try_get("id")?,
            profile: r.try_get("profile")?,
            paint: r.try_get("paint")?,
            starter: r.try_get("starter")?,
            parts: r.try_get::<Json<BTreeMap<String, u32>>, _>("parts")?.0,
            condition: r.try_get::<Json<BTreeMap<String, u32>>, _>("condition")?.0,
        })
    })
    .collect::<Result<Vec<_>, sqlx::Error>>()?;
    let kit = sqlx::query("SELECT slot, item FROM kit WHERE account = $1")
        .bind(&account)
        .fetch_all(&mut *t)
        .await?
        .iter()
        .map(|r| Ok((r.try_get("slot")?, r.try_get::<i32, _>("item")? as u32)))
        .collect::<Result<BTreeMap<String, u32>, sqlx::Error>>()?;
    let overlapping = match window {
        Some((from, to)) => sqlx::query(
            "SELECT ride_id, started, ended FROM ride_uploads
             WHERE account = $1 AND status <> 'refused' AND started < $3 AND ended > $2
             ORDER BY started",
        )
        .bind(&account)
        .bind(from)
        .bind(to)
        .fetch_all(&mut *t)
        .await?
        .iter()
        .map(|r| {
            Ok((
                r.try_get("ride_id")?,
                r.try_get("started")?,
                r.try_get("ended")?,
            ))
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()?,
        None => Vec::new(),
    };
    let garage = if started {
        Garage {
            rider: a.try_get("display_name")?,
            active: a.try_get("active_bike")?,
            bikes,
            items,
            next_item: a.try_get::<i32, _>("next_item")? as u32,
            kit,
        }
    } else {
        Garage::starter("")
    };
    Ok(Loaded::Wallet(
        t,
        Wallet {
            hearts,
            garage,
            started,
            overlapping,
        },
    ))
}

async fn write(
    mut t: Transaction<'static, Postgres>,
    account: String,
    w: Wallet,
    change: Change,
    key: Option<(String, String)>,
    now: i64,
) -> Result<i64, sqlx::Error> {
    let g = &w.garage;
    for q in [
        "DELETE FROM kit WHERE account = $1",
        "DELETE FROM bikes WHERE account = $1",
        "DELETE FROM items WHERE account = $1",
    ] {
        sqlx::query(q).bind(&account).execute(&mut *t).await?;
    }
    for i in &g.items {
        sqlx::query(
            "INSERT INTO items (account, id, part, paid, bought, ridden_m, with_item)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(&account)
        .bind(i.id as i32)
        .bind(&i.part)
        .bind(i.paid as i32)
        .bind(&i.bought)
        .bind(i.ridden_m)
        .bind(i.with.map(|w| w as i32))
        .execute(&mut *t)
        .await?;
    }
    for (hook, b) in g.bikes.iter().enumerate() {
        sqlx::query(
            "INSERT INTO bikes (account, id, hook, profile, paint, starter, parts, condition)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(&account)
        .bind(&b.id)
        .bind(hook as i32)
        .bind(&b.profile)
        .bind(&b.paint)
        .bind(b.starter)
        .bind(Json(&b.parts))
        .bind(Json(&b.condition))
        .execute(&mut *t)
        .await?;
    }
    for (slot, item) in &g.kit {
        sqlx::query("INSERT INTO kit (account, slot, item) VALUES ($1, $2, $3)")
            .bind(&account)
            .bind(slot)
            .bind(*item as i32)
            .execute(&mut *t)
            .await?;
    }
    let delta: i64 = change.bookings.iter().map(|b| b.delta).sum();
    let hearts: i64 = sqlx::query_scalar(
        "UPDATE accounts SET hearts = hearts + $2, workshop_started = $3, active_bike = $4, next_item = $5
         WHERE id = $1 RETURNING hearts",
    )
    .bind(&account)
    .bind(delta)
    .bind(w.started)
    .bind(&g.active)
    .bind(g.next_item as i32)
    .fetch_one(&mut *t)
    .await?;
    for b in &change.bookings {
        sqlx::query(
            "INSERT INTO hearts_ledger (account, delta, reason, reference, at) VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(&account)
        .bind(b.delta)
        .bind(b.reason.id())
        .bind(&b.reference)
        .bind(now)
        .execute(&mut *t)
        .await?;
    }
    if let Some(r) = &change.ride {
        sqlx::query(
            "INSERT INTO ride_uploads (account, ride_id, source, server_key, status, hearts, started, ended, reasons, body, at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(&account)
        .bind(&r.ride_id)
        .bind(&r.source)
        .bind(&r.server_key)
        .bind(status_id(r.status))
        .bind(r.hearts as i32)
        .bind(r.started)
        .bind(r.ended)
        .bind(Json(&r.reasons))
        .bind(Json(&r.body))
        .bind(now)
        .execute(&mut *t)
        .await?;
    }
    if let Some((k, h)) = &key {
        sqlx::query(
            "INSERT INTO idempotency (account, key, request_hash, answer, at) VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(&account)
        .bind(k)
        .bind(h)
        .bind(Json(&change.answer))
        .bind(now)
        .execute(&mut *t)
        .await?;
    }
    t.commit().await?;
    Ok(hearts)
}

impl WorkshopStore for PgStore {
    fn transact(&self, tx: Tx<'_>, f: &mut TxFn<'_, ApiFail>) -> Result<Done, TxError<ApiFail>> {
        let account = tx.account.to_owned();
        let key = tx.key.map(|(k, h)| (k.to_owned(), h.to_owned()));
        let loaded = self
            .run(load(
                self.pool.clone(),
                account.clone(),
                key.as_ref().map(|(k, _)| k.clone()),
                tx.window,
            ))
            .map_err(|e| TxError::Store(wio(e)))?;
        let (t, mut w) = match loaded {
            Loaded::NoAccount => return Err(TxError::Store(StoreFail::NoAccount)),
            Loaded::Seen {
                hash,
                answer,
                hearts,
            } => {
                if key.as_ref().is_some_and(|(_, h)| *h != hash) {
                    return Err(TxError::Store(StoreFail::KeyReused));
                }
                return Ok(Done {
                    answer,
                    repeated: true,
                    hearts,
                });
            }
            Loaded::Wallet(t, w) => (t, w),
        };
        let change = match f(&mut w) {
            Ok(c) => c,
            Err(e) => {
                let _ = self.run(t.rollback());
                return Err(TxError::Op(e));
            }
        };
        let answer = change.answer.clone();
        let hearts = self
            .run(write(t, account, w, change, key, tx.now))
            .map_err(|e| {
                TxError::Store(match code(&e).as_deref() {
                    Some("23514") => StoreFail::Negative,
                    Some("23505") => StoreFail::RideTwice,
                    _ => wio(e),
                })
            })?;
        Ok(Done {
            answer,
            repeated: false,
            hearts,
        })
    }

    fn ledger(&self, account: &str, limit: usize) -> Result<Vec<LedgerEntry>, StoreFail> {
        let pool = self.pool.clone();
        let account = account.to_owned();
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        self.run(async move {
            sqlx::query(
                "SELECT at, delta, reason, reference FROM hearts_ledger WHERE account = $1
                 ORDER BY id DESC LIMIT $2",
            )
            .bind(&account)
            .bind(limit)
            .fetch_all(&pool)
            .await?
            .iter()
            .map(|r| {
                let reason: String = r.try_get("reason")?;
                Ok(LedgerEntry {
                    at: r.try_get("at")?,
                    delta: r.try_get("delta")?,
                    reason: Reason::parse(&reason)
                        .ok_or_else(|| sqlx::Error::Decode(format!("reason {reason}").into()))?,
                    reference: r.try_get("reference")?,
                })
            })
            .collect::<Result<Vec<_>, sqlx::Error>>()
        })
        .map_err(wio)
    }

    fn ledger_sum(&self, account: &str) -> Result<i64, StoreFail> {
        let pool = self.pool.clone();
        let account = account.to_owned();
        self.run(async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT COALESCE(SUM(delta), 0)::BIGINT FROM hearts_ledger WHERE account = $1",
            )
            .bind(&account)
            .fetch_one(&pool)
            .await
        })
        .map_err(wio)
    }

    fn rides(&self, account: &str) -> Result<Vec<RideRecord>, StoreFail> {
        let pool = self.pool.clone();
        let account = account.to_owned();
        self.run(async move {
            sqlx::query(
                "SELECT ride_id, source, server_key, status, hearts, started, ended, reasons, body
                 FROM ride_uploads WHERE account = $1 ORDER BY at DESC, started DESC",
            )
            .bind(&account)
            .fetch_all(&pool)
            .await?
            .iter()
            .map(|r| {
                let status: String = r.try_get("status")?;
                Ok(RideRecord {
                    ride_id: r.try_get("ride_id")?,
                    source: r.try_get("source")?,
                    server_key: r.try_get("server_key")?,
                    status: status_parse(&status),
                    hearts: r.try_get::<i32, _>("hearts")? as u32,
                    started: r.try_get("started")?,
                    ended: r.try_get("ended")?,
                    reasons: r.try_get::<Json<Vec<String>>, _>("reasons")?.0,
                    body: r.try_get::<Json<serde_json::Value>, _>("body")?.0,
                })
            })
            .collect::<Result<Vec<_>, sqlx::Error>>()
        })
        .map_err(wio)
    }

    fn delete(&self, account: &str) -> Result<(), StoreFail> {
        let pool = self.pool.clone();
        let account = account.to_owned();
        self.run(async move {
            let mut t = pool.begin().await?;
            for q in [
                "DELETE FROM kit WHERE account = $1",
                "DELETE FROM bikes WHERE account = $1",
                "DELETE FROM items WHERE account = $1",
                "DELETE FROM hearts_ledger WHERE account = $1",
                "DELETE FROM idempotency WHERE account = $1",
                "DELETE FROM ride_uploads WHERE account = $1",
                "UPDATE accounts SET hearts = 0, workshop_started = FALSE, active_bike = '', next_item = 1 WHERE id = $1",
            ] {
                sqlx::query(q).bind(&account).execute(&mut *t).await?;
            }
            t.commit().await
        })
        .map_err(wio)
    }
}
