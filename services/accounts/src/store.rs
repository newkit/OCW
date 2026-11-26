//! Durable account storage behind a trait (spec §9.1, §12): accounts and
//! their linked provider identities, refresh tokens (only their SHA-256),
//! encrypted provider tokens (Strava).
//!
//! [`MemStore`] keeps everything in memory and, opened on a path, writes
//! a JSON snapshot after every change (atomically, mode 0600): enough for
//! development and a single small instance. PostgreSQL (`sqlx` with
//! migrations, spec §9) is the next step for the hosted backend; the trait
//! is what it implements. Short-lived state (pending sign-ins, codes,
//! device codes) is not stored here: it lives in the service's memory for
//! minutes and a restart only means signing in again.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use ocw_auth::Provider;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub display_name: String,
    pub created_at: i64,
    pub identities: Vec<Identity>,
}

/// A provider account linked to an OCW account, keyed by the provider's
/// subject (Google `sub`, Strava athlete id).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub provider: Provider,
    pub subject: String,
    pub name: String,
    #[serde(default)]
    pub email: Option<String>,
    pub linked_at: i64,
    /// Strava: `activity:write` granted.
    #[serde(default)]
    pub upload: bool,
}

/// One refresh token (by hash) of a rotation family (one sign-in).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefreshRecord {
    pub hash: String,
    pub family: String,
    pub account: String,
    pub client_id: String,
    pub scope: String,
    pub auth_time: i64,
    pub family_created: i64,
    pub created_at: i64,
    /// Rotated: presenting it again is a reuse (the family is revoked).
    pub used: bool,
    pub revoked: bool,
}

/// A provider token, encrypted ([`crate::secrets`]).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealedSecret {
    pub nonce: String,
    pub ciphertext: String,
}

impl std::fmt::Debug for SealedSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SealedSecret(***)")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    #[error("no such account")]
    NoAccount,
    #[error("this {0} account is linked to another OCW account")]
    LinkedElsewhere(Provider),
    #[error("{0} is the only sign-in of the account: link another provider first")]
    LastIdentity(Provider),
    #[error("{0} is not linked")]
    NotLinked(Provider),
    #[error("storage: {0}")]
    Io(String),
}

/// The account storage.
pub trait Store: Send + Sync {
    fn account(&self, id: &str) -> Option<Account>;
    /// The account a provider identity belongs to.
    fn find_identity(&self, provider: Provider, subject: &str) -> Option<String>;
    fn create_account(&self, account: Account) -> Result<(), StoreError>;
    /// Links (or refreshes the name of) an identity.
    fn link(&self, account: &str, identity: Identity) -> Result<(), StoreError>;
    /// Unlinks a provider; never the last one (an account always keeps a
    /// way to sign in).
    fn unlink(&self, account: &str, provider: Provider) -> Result<(), StoreError>;
    /// Deletes the account with its identities, tokens and secrets.
    fn delete_account(&self, account: &str) -> Result<(), StoreError>;
    /// The account's display name (its rider's name, 4O W5b): the name
    /// in its next tokens.
    fn rename(&self, account: &str, name: &str) -> Result<(), StoreError>;

    fn put_refresh(&self, record: RefreshRecord) -> Result<(), StoreError>;
    fn refresh(&self, hash: &str) -> Option<RefreshRecord>;
    /// Marks one token rotated and returns it as it was before (atomic:
    /// of two concurrent refreshes with one token only one sees it
    /// unused; the other is a reuse).
    fn use_refresh(&self, hash: &str) -> Result<Option<RefreshRecord>, StoreError>;
    /// Forgets tokens created before `before` (idle-expired anyway).
    fn purge_refresh(&self, before: i64) -> Result<(), StoreError>;
    fn revoke_family(&self, family: &str) -> Result<(), StoreError>;
    fn revoke_account(&self, account: &str) -> Result<(), StoreError>;

    fn put_secret(
        &self,
        account: &str,
        provider: Provider,
        s: SealedSecret,
    ) -> Result<(), StoreError>;
    fn secret(&self, account: &str, provider: Provider) -> Option<SealedSecret>;
    fn delete_secret(&self, account: &str, provider: Provider) -> Result<(), StoreError>;
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Data {
    accounts: BTreeMap<String, Account>,
    /// "provider:subject" → account id.
    identities: BTreeMap<String, String>,
    refresh: BTreeMap<String, RefreshRecord>,
    /// "account:provider" → sealed token.
    secrets: BTreeMap<String, SealedSecret>,
}

fn ident_key(p: Provider, subject: &str) -> String {
    format!("{}:{subject}", p.id())
}

fn secret_key(account: &str, p: Provider) -> String {
    format!("{account}:{}", p.id())
}

/// In memory, optionally saved to a JSON file after every change.
pub struct MemStore {
    data: Mutex<Data>,
    path: Option<PathBuf>,
}

impl MemStore {
    pub fn memory() -> Self {
        Self {
            data: Mutex::new(Data::default()),
            path: None,
        }
    }

    /// Opens (or starts) the file store at `path`.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let path = path.into();
        let data = match std::fs::read_to_string(&path) {
            Ok(s) => serde_json::from_str(&s)
                .map_err(|e| StoreError::Io(format!("{}: {e}", path.display())))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Data::default(),
            Err(e) => return Err(StoreError::Io(format!("{}: {e}", path.display()))),
        };
        Ok(Self {
            data: Mutex::new(data),
            path: Some(path),
        })
    }

    fn with<R>(&self, f: impl FnOnce(&mut Data) -> Result<R, StoreError>) -> Result<R, StoreError> {
        let mut d = self.data.lock().unwrap_or_else(|e| e.into_inner());
        let r = f(&mut d)?;
        if let Some(path) = &self.path {
            save(path, &d)?;
        }
        Ok(r)
    }

    fn read<R>(&self, f: impl FnOnce(&Data) -> R) -> R {
        f(&self.data.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// Number of accounts (tests, statistics).
    pub fn accounts(&self) -> usize {
        self.read(|d| d.accounts.len())
    }

    /// Number of refresh tokens kept (tests).
    pub fn refresh_tokens(&self) -> usize {
        self.read(|d| d.refresh.len())
    }
}

/// Writes through a temporary file, readable only by the owner.
pub(crate) fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("part");
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&tmp)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    f.write_all(bytes)?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, path)
}

fn save(path: &std::path::Path, d: &Data) -> Result<(), StoreError> {
    let json = serde_json::to_vec_pretty(d).map_err(|e| StoreError::Io(e.to_string()))?;
    write_private(path, &json).map_err(|e| StoreError::Io(format!("{}: {e}", path.display())))
}

impl Store for MemStore {
    fn account(&self, id: &str) -> Option<Account> {
        self.read(|d| d.accounts.get(id).cloned())
    }

    fn find_identity(&self, provider: Provider, subject: &str) -> Option<String> {
        self.read(|d| d.identities.get(&ident_key(provider, subject)).cloned())
    }

    fn create_account(&self, account: Account) -> Result<(), StoreError> {
        self.with(|d| {
            for i in &account.identities {
                if d.identities
                    .contains_key(&ident_key(i.provider, &i.subject))
                {
                    return Err(StoreError::LinkedElsewhere(i.provider));
                }
            }
            for i in &account.identities {
                d.identities
                    .insert(ident_key(i.provider, &i.subject), account.id.clone());
            }
            d.accounts.insert(account.id.clone(), account);
            Ok(())
        })
    }

    fn link(&self, account: &str, identity: Identity) -> Result<(), StoreError> {
        self.with(|d| {
            let key = ident_key(identity.provider, &identity.subject);
            if d.identities.get(&key).is_some_and(|a| a != account) {
                return Err(StoreError::LinkedElsewhere(identity.provider));
            }
            let acc = d.accounts.get_mut(account).ok_or(StoreError::NoAccount)?;
            // One identity per provider: linking another Google account
            // replaces the old one.
            if let Some(old) = acc
                .identities
                .iter()
                .find(|i| i.provider == identity.provider && i.subject != identity.subject)
            {
                d.identities.remove(&ident_key(old.provider, &old.subject));
            }
            acc.identities.retain(|i| i.provider != identity.provider);
            acc.identities.push(identity);
            acc.identities.sort_by_key(|i| i.provider);
            d.identities.insert(key, account.to_owned());
            Ok(())
        })
    }

    fn unlink(&self, account: &str, provider: Provider) -> Result<(), StoreError> {
        self.with(|d| {
            let acc = d.accounts.get_mut(account).ok_or(StoreError::NoAccount)?;
            let Some(pos) = acc.identities.iter().position(|i| i.provider == provider) else {
                return Err(StoreError::NotLinked(provider));
            };
            if acc.identities.len() == 1 {
                return Err(StoreError::LastIdentity(provider));
            }
            let old = acc.identities.remove(pos);
            d.identities.remove(&ident_key(old.provider, &old.subject));
            d.secrets.remove(&secret_key(account, provider));
            Ok(())
        })
    }

    fn rename(&self, account: &str, name: &str) -> Result<(), StoreError> {
        self.with(|d| {
            d.accounts
                .get_mut(account)
                .ok_or(StoreError::NoAccount)?
                .display_name = name.to_owned();
            Ok(())
        })
    }

    fn delete_account(&self, account: &str) -> Result<(), StoreError> {
        self.with(|d| {
            let acc = d.accounts.remove(account).ok_or(StoreError::NoAccount)?;
            for i in &acc.identities {
                d.identities.remove(&ident_key(i.provider, &i.subject));
            }
            d.refresh.retain(|_, r| r.account != account);
            d.secrets
                .retain(|k, _| !k.starts_with(&format!("{account}:")));
            Ok(())
        })
    }

    fn put_refresh(&self, record: RefreshRecord) -> Result<(), StoreError> {
        self.with(|d| {
            d.refresh.insert(record.hash.clone(), record);
            Ok(())
        })
    }

    fn refresh(&self, hash: &str) -> Option<RefreshRecord> {
        self.read(|d| d.refresh.get(hash).cloned())
    }

    fn use_refresh(&self, hash: &str) -> Result<Option<RefreshRecord>, StoreError> {
        self.with(|d| {
            Ok(d.refresh.get_mut(hash).map(|r| {
                let before = r.clone();
                r.used = true;
                before
            }))
        })
    }

    fn purge_refresh(&self, before: i64) -> Result<(), StoreError> {
        let stale = self.read(|d| d.refresh.values().any(|r| r.created_at < before));
        if !stale {
            return Ok(());
        }
        self.with(|d| {
            d.refresh.retain(|_, r| r.created_at >= before);
            Ok(())
        })
    }

    fn revoke_family(&self, family: &str) -> Result<(), StoreError> {
        self.with(|d| {
            for r in d.refresh.values_mut().filter(|r| r.family == family) {
                r.revoked = true;
            }
            Ok(())
        })
    }

    fn revoke_account(&self, account: &str) -> Result<(), StoreError> {
        self.with(|d| {
            for r in d.refresh.values_mut().filter(|r| r.account == account) {
                r.revoked = true;
            }
            Ok(())
        })
    }

    fn put_secret(
        &self,
        account: &str,
        provider: Provider,
        s: SealedSecret,
    ) -> Result<(), StoreError> {
        self.with(|d| {
            d.secrets.insert(secret_key(account, provider), s);
            Ok(())
        })
    }

    fn secret(&self, account: &str, provider: Provider) -> Option<SealedSecret> {
        self.read(|d| d.secrets.get(&secret_key(account, provider)).cloned())
    }

    fn delete_secret(&self, account: &str, provider: Provider) -> Result<(), StoreError> {
        self.with(|d| {
            d.secrets.remove(&secret_key(account, provider));
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ident(p: Provider, sub: &str) -> Identity {
        Identity {
            provider: p,
            subject: sub.into(),
            name: format!("{p} {sub}"),
            email: None,
            linked_at: 1,
            upload: false,
        }
    }

    fn account(id: &str, i: Identity) -> Account {
        Account {
            id: id.into(),
            display_name: i.name.clone(),
            created_at: 1,
            identities: vec![i],
        }
    }

    #[test]
    fn linking_rules() {
        let s = MemStore::memory();
        s.create_account(account("a", ident(Provider::Google, "g1")))
            .unwrap();
        s.create_account(account("b", ident(Provider::Strava, "s1")))
            .unwrap();
        assert_eq!(
            s.find_identity(Provider::Google, "g1").as_deref(),
            Some("a")
        );
        // Never the last sign-in.
        assert_eq!(
            s.unlink("a", Provider::Google),
            Err(StoreError::LastIdentity(Provider::Google))
        );
        // Another account's identity cannot be linked.
        assert_eq!(
            s.link("a", ident(Provider::Strava, "s1")),
            Err(StoreError::LinkedElsewhere(Provider::Strava))
        );
        s.link("a", ident(Provider::Strava, "s2")).unwrap();
        assert_eq!(s.account("a").unwrap().identities.len(), 2);
        s.unlink("a", Provider::Google).unwrap();
        assert_eq!(s.find_identity(Provider::Google, "g1"), None);
        assert_eq!(
            s.unlink("a", Provider::Google),
            Err(StoreError::NotLinked(Provider::Google))
        );
        // Another Google account replaces the first.
        s.link("a", ident(Provider::Google, "g1")).unwrap();
        s.link("a", ident(Provider::Google, "g9")).unwrap();
        assert_eq!(s.find_identity(Provider::Google, "g1"), None);
        assert_eq!(
            s.find_identity(Provider::Google, "g9").as_deref(),
            Some("a")
        );
        assert!(
            s.create_account(account("c", ident(Provider::Google, "g9")))
                .is_err()
        );
    }

    #[test]
    fn deletion_cascades_and_the_file_survives_a_restart() {
        let dir = std::env::temp_dir().join(format!("ocw-accounts-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("accounts.json");
        {
            let s = MemStore::open(&path).unwrap();
            s.create_account(account("a", ident(Provider::Strava, "1")))
                .unwrap();
            s.put_secret(
                "a",
                Provider::Strava,
                SealedSecret {
                    nonce: "n".into(),
                    ciphertext: "c".into(),
                },
            )
            .unwrap();
            s.put_refresh(RefreshRecord {
                hash: "h".into(),
                family: "f".into(),
                account: "a".into(),
                client_id: "ocw-desktop".into(),
                scope: String::new(),
                auth_time: 0,
                family_created: 0,
                created_at: 0,
                used: false,
                revoked: false,
            })
            .unwrap();
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let s = MemStore::open(&path).unwrap();
        assert_eq!(s.accounts(), 1);
        assert!(s.secret("a", Provider::Strava).is_some());
        s.delete_account("a").unwrap();
        assert_eq!((s.accounts(), s.refresh_tokens()), (0, 0));
        assert!(s.secret("a", Provider::Strava).is_none());
        assert_eq!(s.find_identity(Provider::Strava, "1"), None);
        let s = MemStore::open(&path).unwrap();
        assert_eq!(s.accounts(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
