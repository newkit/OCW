//! The token signing keys (Ed25519) with rotation and the published JWKS
//! (spec §9.1). The newest key signs; a retired key stays published until
//! every token it signed has expired, so a rotation never logs anyone out.

use std::path::PathBuf;
use std::sync::RwLock;

use ed25519_dalek::SigningKey;
use ocw_auth::jwt::{Jwk, Jwks, key_id};
use serde::{Deserialize, Serialize};

use crate::random_bytes;
use crate::store::write_private;

struct Entry {
    kid: String,
    key: SigningKey,
    created_at: i64,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    kid: String,
    created_at: i64,
    /// The secret key, base64url.
    secret: String,
}

pub struct KeyRing {
    /// Oldest first; the last one signs.
    keys: RwLock<Vec<Entry>>,
    file: Option<PathBuf>,
    rotate_s: i64,
    /// How long a retired key stays published.
    grace_s: i64,
}

impl KeyRing {
    /// Keys in memory (a fresh key now).
    pub fn memory(now: i64, rotate_s: i64, grace_s: i64) -> Self {
        let ring = Self {
            keys: RwLock::new(Vec::new()),
            file: None,
            rotate_s,
            grace_s,
        };
        ring.push(now);
        ring
    }

    /// Keys kept in `dir/signing-keys.json` (0600), created on first use.
    pub fn open(dir: PathBuf, now: i64, rotate_s: i64, grace_s: i64) -> Result<Self, String> {
        let file = dir.join("signing-keys.json");
        let saved: Vec<Saved> = match std::fs::read_to_string(&file) {
            Ok(s) => serde_json::from_str(&s).map_err(|e| format!("{}: {e}", file.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(format!("{}: {e}", file.display())),
        };
        let mut keys = Vec::new();
        for s in saved {
            let bytes: [u8; 32] = decode(&s.secret)
                .and_then(|b| b.try_into().ok())
                .ok_or_else(|| format!("{}: bad key {}", file.display(), s.kid))?;
            keys.push(Entry {
                kid: s.kid,
                key: SigningKey::from_bytes(&bytes),
                created_at: s.created_at,
            });
        }
        let ring = Self {
            keys: RwLock::new(keys),
            file: Some(file),
            rotate_s,
            grace_s,
        };
        if ring
            .keys
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty()
        {
            ring.push(now);
        }
        ring.save()?;
        Ok(ring)
    }

    fn push(&self, now: i64) {
        let key = SigningKey::from_bytes(&random_bytes::<32>());
        let kid = key_id(&key.verifying_key());
        self.keys
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .push(Entry {
                kid,
                key,
                created_at: now,
            });
    }

    fn save(&self) -> Result<(), String> {
        let Some(file) = &self.file else {
            return Ok(());
        };
        let saved: Vec<Saved> = self
            .keys
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|e| Saved {
                kid: e.kid.clone(),
                created_at: e.created_at,
                secret: encode(&e.key.to_bytes()),
            })
            .collect();
        let json = serde_json::to_vec_pretty(&saved).map_err(|e| e.to_string())?;
        write_private(file, &json).map_err(|e| format!("{}: {e}", file.display()))
    }

    /// The signing key and its id.
    pub fn active(&self) -> (String, SigningKey) {
        let keys = self.keys.read().unwrap_or_else(|e| e.into_inner());
        let e = keys.last().expect("a key ring always has a key");
        (e.kid.clone(), e.key.clone())
    }

    /// A new signing key now; the old one stays published for the grace
    /// time.
    pub fn rotate(&self, now: i64) -> Result<(), String> {
        self.push(now);
        self.prune(now);
        self.save()
    }

    /// Rotates when the signing key is older than the rotation period.
    /// Returns whether it rotated.
    pub fn maybe_rotate(&self, now: i64) -> Result<bool, String> {
        let due = {
            let keys = self.keys.read().unwrap_or_else(|e| e.into_inner());
            keys.last()
                .is_none_or(|e| now - e.created_at >= self.rotate_s)
        };
        if due {
            self.rotate(now)?;
        }
        Ok(due)
    }

    /// Drops retired keys past their grace time.
    fn prune(&self, now: i64) {
        let mut keys = self.keys.write().unwrap_or_else(|e| e.into_inner());
        // A key retired when its successor was created.
        let succ: Vec<i64> = keys.iter().skip(1).map(|e| e.created_at).collect();
        let mut idx = 0;
        keys.retain(|_| {
            let keep = succ
                .get(idx)
                .is_none_or(|&retired| now - retired < self.grace_s);
            idx += 1;
            keep
        });
    }

    /// The published key set (the signing key and the retired ones still
    /// in their grace time).
    pub fn jwks(&self) -> Jwks {
        Jwks {
            keys: self
                .keys
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .map(|e| Jwk::ed25519(&e.kid, &e.key.verifying_key()))
                .collect(),
        }
    }
}

fn encode(b: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

fn decode(s: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(s)
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_keeps_the_old_key_for_its_grace_time() {
        let ring = KeyRing::memory(0, 100, 30);
        let (kid0, _) = ring.active();
        assert!(!ring.maybe_rotate(99).unwrap());
        assert!(ring.maybe_rotate(100).unwrap());
        let (kid1, _) = ring.active();
        assert_ne!(kid0, kid1);
        let ids = |r: &KeyRing| -> Vec<String> {
            r.jwks().keys.iter().filter_map(|k| k.kid.clone()).collect()
        };
        assert_eq!(ids(&ring), [kid0.clone(), kid1.clone()]);
        // Past the grace time the next rotation drops it.
        ring.rotate(131).unwrap();
        let (kid2, _) = ring.active();
        assert_eq!(ids(&ring), [kid1, kid2]);
    }

    #[test]
    fn keys_survive_a_restart() {
        let dir = std::env::temp_dir().join(format!("ocw-accounts-keys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let a = KeyRing::open(dir.clone(), 0, 100, 30).unwrap();
        let b = KeyRing::open(dir.clone(), 50, 100, 30).unwrap();
        assert_eq!(a.active().0, b.active().0);
        assert_eq!(a.jwks(), b.jwks());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join("signing-keys.json"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
