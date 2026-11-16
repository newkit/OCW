//! Provider tokens at rest (spec §9.1: Strava tokens are stored
//! encrypted, server-side only): ChaCha20-Poly1305 with a 256-bit key from
//! a private file, a random nonce per value, and the account and provider
//! as associated data (a sealed value cannot be moved to another account).

use std::path::Path;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use ocw_auth::Provider;
use serde::{Deserialize, Serialize};

use crate::random_bytes;
use crate::store::{SealedSecret, write_private};

pub struct SecretBox {
    cipher: ChaCha20Poly1305,
}

/// Strava's tokens of one athlete.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
    pub scope: Vec<String>,
}

impl std::fmt::Debug for ProviderTokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ProviderTokens {{ expires_at: {}, scope: {:?}, tokens: *** }}",
            self.expires_at, self.scope
        )
    }
}

fn b64(b: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(b)
}

fn unb64(s: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(s).ok()
}

impl SecretBox {
    /// A key that lives only in this process.
    pub fn ephemeral() -> Self {
        Self::from_key(&random_bytes::<32>())
    }

    pub fn from_key(key: &[u8; 32]) -> Self {
        Self {
            cipher: ChaCha20Poly1305::new(Key::from_slice(key)),
        }
    }

    /// The key from `path`, created (0600) when missing.
    pub fn open(path: &Path) -> Result<Self, String> {
        match std::fs::read(path) {
            Ok(b) => {
                let key: [u8; 32] = b
                    .try_into()
                    .map_err(|_| format!("{}: not a 32-byte key", path.display()))?;
                Ok(Self::from_key(&key))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let key = random_bytes::<32>();
                write_private(path, &key).map_err(|e| format!("{}: {e}", path.display()))?;
                Ok(Self::from_key(&key))
            }
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    fn aad(account: &str, provider: Provider) -> Vec<u8> {
        format!("ocw-secret:{account}:{}", provider.id()).into_bytes()
    }

    pub fn seal(&self, account: &str, provider: Provider, t: &ProviderTokens) -> SealedSecret {
        let nonce = random_bytes::<12>();
        let msg = serde_json::to_vec(t).expect("tokens serialize");
        let aad = Self::aad(account, provider);
        let ct = self
            .cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &msg,
                    aad: &aad,
                },
            )
            .expect("encryption cannot fail");
        SealedSecret {
            nonce: b64(&nonce),
            ciphertext: b64(&ct),
        }
    }

    pub fn open_sealed(
        &self,
        account: &str,
        provider: Provider,
        s: &SealedSecret,
    ) -> Option<ProviderTokens> {
        let nonce = unb64(&s.nonce).filter(|n| n.len() == 12)?;
        let ct = unb64(&s.ciphertext)?;
        let aad = Self::aad(account, provider);
        let msg = self
            .cipher
            .decrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &ct,
                    aad: &aad,
                },
            )
            .ok()?;
        serde_json::from_slice(&msg).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sealed_tokens_open_only_for_their_account() {
        let b = SecretBox::from_key(&[3; 32]);
        let t = ProviderTokens {
            access_token: "strava-access".into(),
            refresh_token: "strava-refresh".into(),
            expires_at: 99,
            scope: vec!["read".into(), "activity:write".into()],
        };
        let s = b.seal("acc_1", Provider::Strava, &t);
        assert!(!s.ciphertext.contains("strava"));
        assert!(!format!("{t:?}").contains("strava-"));
        assert_eq!(
            b.open_sealed("acc_1", Provider::Strava, &s),
            Some(t.clone())
        );
        assert_eq!(b.open_sealed("acc_2", Provider::Strava, &s), None);
        assert_eq!(
            SecretBox::from_key(&[4; 32]).open_sealed("acc_1", Provider::Strava, &s),
            None
        );
        // Two seals of the same value differ (random nonce).
        assert_ne!(b.seal("acc_1", Provider::Strava, &t), s);
    }
}
