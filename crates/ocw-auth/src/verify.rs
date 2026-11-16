//! The check of an access token (spec §9.1): the game server takes the
//! token from the hello and accepts it only with a valid signature from
//! the backend's JWKS, the right issuer, audience and token type, and
//! inside its lifetime. The key set can be replaced at any time (key
//! rotation: the embedding refreshes it from `jwks_uri`).

use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::jwt::{AccessClaims, Jwks, JwtError, Parsed, TYP_ACCESS};

/// Clock skew allowed on `exp` / `iat` (seconds).
pub const LEEWAY_S: i64 = 30;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VerifyError {
    #[error("not a valid token")]
    Malformed,
    #[error("unsupported token algorithm")]
    Algorithm,
    #[error("not an access token")]
    WrongType,
    #[error("signed with an unknown key")]
    UnknownKey,
    #[error("bad token signature")]
    Signature,
    #[error("token expired")]
    Expired,
    #[error("token issued in the future")]
    NotYetValid,
    #[error("token from another issuer")]
    Issuer,
    #[error("token for another audience")]
    Audience,
}

impl VerifyError {
    /// The key set may be stale (a rotation the verifier has not seen):
    /// worth refreshing the JWKS and trying again.
    pub fn refresh_keys(&self) -> bool {
        matches!(self, Self::UnknownKey)
    }
}

impl From<JwtError> for VerifyError {
    fn from(e: JwtError) -> Self {
        match e {
            JwtError::Malformed => Self::Malformed,
            JwtError::Algorithm => Self::Algorithm,
            JwtError::Signature => Self::Signature,
        }
    }
}

/// Checks access tokens of one issuer for one audience.
#[derive(Debug)]
pub struct Verifier {
    issuer: String,
    audience: String,
    keys: RwLock<Jwks>,
    /// A token named a key the set does not have: the embedding should
    /// fetch the JWKS again ([`Verifier::take_stale`]).
    stale: AtomicBool,
}

impl Verifier {
    pub fn new(issuer: impl Into<String>, audience: impl Into<String>, keys: Jwks) -> Self {
        Self {
            issuer: issuer.into(),
            audience: audience.into(),
            keys: RwLock::new(keys),
            stale: AtomicBool::new(false),
        }
    }

    /// Whether a token named an unknown key since the last call (a key
    /// rotation the set has not seen yet): time to fetch `jwks_uri`.
    pub fn take_stale(&self) -> bool {
        self.stale.swap(false, Ordering::Relaxed)
    }

    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    pub fn audience(&self) -> &str {
        &self.audience
    }

    /// Replaces the key set (after a fetch of `jwks_uri`).
    pub fn set_keys(&self, keys: Jwks) {
        *self.keys.write().unwrap_or_else(|e| e.into_inner()) = keys;
    }

    pub fn keys(&self) -> Jwks {
        self.keys.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// The claims of a valid access token at `now_s` (Unix seconds).
    pub fn verify(&self, token: &str, now_s: i64) -> Result<AccessClaims, VerifyError> {
        let parsed = Parsed::parse(token)?;
        if parsed.header.typ.as_deref() != Some(TYP_ACCESS) {
            return Err(VerifyError::WrongType);
        }
        let kid = parsed
            .header
            .kid
            .as_deref()
            .ok_or(VerifyError::UnknownKey)?;
        let key = self
            .keys
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .find(kid)
            .ok_or_else(|| {
                self.stale.store(true, Ordering::Relaxed);
                VerifyError::UnknownKey
            })?;
        let claims: AccessClaims = parsed.verify(&key)?;
        if !crate::constant_time_eq(&claims.iss, &self.issuer) {
            return Err(VerifyError::Issuer);
        }
        if claims.aud != self.audience {
            return Err(VerifyError::Audience);
        }
        if now_s > claims.exp + LEEWAY_S {
            return Err(VerifyError::Expired);
        }
        if claims.iat > now_s + LEEWAY_S {
            return Err(VerifyError::NotYetValid);
        }
        Ok(claims)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jwt::{Jwk, TYP_ID, key_id, sign};
    use ed25519_dalek::SigningKey;

    struct Fixture {
        key: SigningKey,
        kid: String,
        verifier: Verifier,
    }

    fn fixture() -> Fixture {
        let key = SigningKey::from_bytes(&[9; 32]);
        let kid = key_id(&key.verifying_key());
        let verifier = Verifier::new(
            "https://id.example",
            "ocw",
            Jwks {
                keys: vec![Jwk::ed25519(&kid, &key.verifying_key())],
            },
        );
        Fixture { key, kid, verifier }
    }

    fn claims(exp: i64) -> AccessClaims {
        AccessClaims {
            iss: "https://id.example".into(),
            sub: "acc_7".into(),
            aud: "ocw".into(),
            exp,
            iat: exp - 600,
            jti: "j1".into(),
            client_id: "ocw-desktop".into(),
            scope: String::new(),
            name: "Bea".into(),
            auth_time: exp - 600,
            providers: vec![],
        }
    }

    #[test]
    fn accepts_a_good_token_and_refuses_bad_ones() {
        let f = fixture();
        let good = sign(&claims(10_000), &f.key, &f.kid, TYP_ACCESS);
        assert_eq!(f.verifier.verify(&good, 9_500).unwrap().sub, "acc_7");
        // Expired (beyond the leeway), not yet valid.
        assert_eq!(
            f.verifier.verify(&good, 10_000 + LEEWAY_S + 1),
            Err(VerifyError::Expired)
        );
        assert_eq!(
            f.verifier.verify(&good, 10_000 + LEEWAY_S),
            Ok(claims(10_000))
        );
        assert_eq!(
            f.verifier.verify(&good, 9_400 - LEEWAY_S - 1),
            Err(VerifyError::NotYetValid)
        );
        // Another issuer / audience.
        let mut c = claims(10_000);
        c.iss = "https://evil.example".into();
        let t = sign(&c, &f.key, &f.kid, TYP_ACCESS);
        assert_eq!(f.verifier.verify(&t, 9_500), Err(VerifyError::Issuer));
        let mut c = claims(10_000);
        c.aud = "something-else".into();
        let t = sign(&c, &f.key, &f.kid, TYP_ACCESS);
        assert_eq!(f.verifier.verify(&t, 9_500), Err(VerifyError::Audience));
        // An ID token is not an access token.
        let t = sign(&claims(10_000), &f.key, &f.kid, TYP_ID);
        assert_eq!(f.verifier.verify(&t, 9_500), Err(VerifyError::WrongType));
        // A key the set does not have, a forged signature.
        let other = SigningKey::from_bytes(&[8; 32]);
        let t = sign(&claims(10_000), &other, "unknown", TYP_ACCESS);
        assert_eq!(f.verifier.verify(&t, 9_500), Err(VerifyError::UnknownKey));
        assert!(VerifyError::UnknownKey.refresh_keys());
        let t = sign(&claims(10_000), &other, &f.kid, TYP_ACCESS);
        assert_eq!(f.verifier.verify(&t, 9_500), Err(VerifyError::Signature));
        assert_eq!(
            f.verifier.verify("garbage", 9_500),
            Err(VerifyError::Malformed)
        );
    }

    #[test]
    fn key_rotation_replaces_the_set() {
        let f = fixture();
        let new = SigningKey::from_bytes(&[10; 32]);
        let new_kid = key_id(&new.verifying_key());
        let t = sign(&claims(10_000), &new, &new_kid, TYP_ACCESS);
        assert_eq!(f.verifier.verify(&t, 9_500), Err(VerifyError::UnknownKey));
        assert!(f.verifier.take_stale() && !f.verifier.take_stale());
        // The backend publishes the old and the new key during a rotation.
        f.verifier.set_keys(Jwks {
            keys: vec![
                Jwk::ed25519(&f.kid, &f.key.verifying_key()),
                Jwk::ed25519(&new_kid, &new.verifying_key()),
            ],
        });
        assert!(f.verifier.verify(&t, 9_500).is_ok());
        let old = sign(&claims(10_000), &f.key, &f.kid, TYP_ACCESS);
        assert!(f.verifier.verify(&old, 9_500).is_ok());
        // The old key retired: its tokens fail.
        f.verifier.set_keys(Jwks {
            keys: vec![Jwk::ed25519(&new_kid, &new.verifying_key())],
        });
        assert_eq!(f.verifier.verify(&old, 9_500), Err(VerifyError::UnknownKey));
        assert_eq!(f.verifier.keys().keys.len(), 1);
    }
}
