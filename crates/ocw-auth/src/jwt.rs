//! Compact JWS with EdDSA (RFC 7515, RFC 8037), our token claims and the
//! JSON Web Key Set (RFC 7517).
//!
//! Only `EdDSA` with Ed25519 keys is accepted for our own tokens: one
//! algorithm, no `none`, no HMAC (no algorithm confusion). Access tokens
//! carry `typ: at+jwt` (RFC 9068) so an ID token is never taken for one.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier as _, VerifyingKey};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::{b64, unb64};

/// The only signature algorithm of our tokens.
pub const ALG: &str = "EdDSA";
/// `typ` of access tokens (RFC 9068).
pub const TYP_ACCESS: &str = "at+jwt";
/// `typ` of ID tokens.
pub const TYP_ID: &str = "JWT";
/// Tokens longer than this are refused before parsing.
pub const MAX_TOKEN_LEN: usize = 8 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header {
    pub alg: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub typ: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kid: Option<String>,
}

/// The claims of our access tokens (spec §9.1): who (the OCW account), for
/// whom (`aud`), how long, which app asked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessClaims {
    pub iss: String,
    /// The OCW account id.
    pub sub: String,
    pub aud: String,
    pub exp: i64,
    pub iat: i64,
    pub jti: String,
    pub client_id: String,
    #[serde(default)]
    pub scope: String,
    /// The account's display name (riders see it).
    #[serde(default)]
    pub name: String,
    /// When the user signed in at a provider (Unix s): sensitive account
    /// actions (deletion) want a recent sign-in.
    #[serde(default)]
    pub auth_time: i64,
    /// The linked providers ("google", "strava").
    #[serde(default)]
    pub providers: Vec<String>,
}

/// The claims of the ID tokens the apps get (OpenID Connect Core §2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdClaims {
    pub iss: String,
    pub sub: String,
    /// The app's client id.
    pub aud: String,
    pub exp: i64,
    pub iat: i64,
    #[serde(default)]
    pub auth_time: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nonce: Option<String>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub providers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JwtError {
    #[error("not a token")]
    Malformed,
    #[error("unsupported algorithm")]
    Algorithm,
    #[error("bad signature")]
    Signature,
}

/// Signs `claims` as a compact JWS with `key` (id `kid`).
pub fn sign<T: Serialize>(claims: &T, key: &SigningKey, kid: &str, typ: &str) -> String {
    let header = Header {
        alg: ALG.into(),
        typ: Some(typ.into()),
        kid: Some(kid.into()),
    };
    let h = b64(&serde_json::to_vec(&header).expect("header serializes"));
    let p = b64(&serde_json::to_vec(claims).expect("claims serialize"));
    let input = format!("{h}.{p}");
    let sig = key.sign(input.as_bytes());
    format!("{input}.{}", b64(&sig.to_bytes()))
}

/// A token split into its parts, signature not yet checked.
pub struct Parsed {
    pub header: Header,
    signing_input: String,
    payload: Vec<u8>,
    signature: Signature,
}

impl Parsed {
    pub fn parse(token: &str) -> Result<Self, JwtError> {
        if token.len() > MAX_TOKEN_LEN {
            return Err(JwtError::Malformed);
        }
        let mut parts = token.split('.');
        let (Some(h), Some(p), Some(s), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(JwtError::Malformed);
        };
        let header: Header = unb64(h)
            .and_then(|b| serde_json::from_slice(&b).ok())
            .ok_or(JwtError::Malformed)?;
        if header.alg != ALG {
            return Err(JwtError::Algorithm);
        }
        let payload = unb64(p).ok_or(JwtError::Malformed)?;
        let sig: [u8; 64] = unb64(s)
            .and_then(|b| b.try_into().ok())
            .ok_or(JwtError::Malformed)?;
        Ok(Self {
            header,
            signing_input: format!("{h}.{p}"),
            payload,
            signature: Signature::from_bytes(&sig),
        })
    }

    /// Checks the signature with `key` and decodes the claims.
    pub fn verify<T: DeserializeOwned>(&self, key: &VerifyingKey) -> Result<T, JwtError> {
        key.verify_strict(self.signing_input.as_bytes(), &self.signature)
            .map_err(|_| JwtError::Signature)?;
        serde_json::from_slice(&self.payload).map_err(|_| JwtError::Malformed)
    }

    /// The claims **without** checking the signature: only for an app
    /// reading the tokens it just got from the token endpoint over TLS
    /// (OpenID Connect Core §3.1.3.7), never for a server's decision.
    pub fn claims_unverified<T: DeserializeOwned>(&self) -> Result<T, JwtError> {
        serde_json::from_slice(&self.payload).map_err(|_| JwtError::Malformed)
    }
}

/// One public key (RFC 7517; Ed25519 as OKP, RFC 8037). Other key types
/// (a provider's RSA keys) parse and are ignored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Jwk {
    pub kty: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crv: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kid: Option<String>,
    #[serde(rename = "use", default, skip_serializing_if = "Option::is_none")]
    pub use_: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alg: Option<String>,
}

impl Jwk {
    pub fn ed25519(kid: &str, key: &VerifyingKey) -> Self {
        Self {
            kty: "OKP".into(),
            crv: Some("Ed25519".into()),
            x: Some(b64(key.as_bytes())),
            kid: Some(kid.into()),
            use_: Some("sig".into()),
            alg: Some(ALG.into()),
        }
    }

    /// The Ed25519 key, if this is one.
    pub fn verifying_key(&self) -> Option<VerifyingKey> {
        if self.kty != "OKP" || self.crv.as_deref() != Some("Ed25519") {
            return None;
        }
        if self.use_.as_deref().is_some_and(|u| u != "sig")
            || self.alg.as_deref().is_some_and(|a| a != ALG)
        {
            return None;
        }
        let bytes: [u8; 32] = unb64(self.x.as_deref()?)?.try_into().ok()?;
        VerifyingKey::from_bytes(&bytes).ok()
    }
}

/// The key set the backend publishes at `jwks_uri`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Jwks {
    pub keys: Vec<Jwk>,
}

impl Jwks {
    /// The key with id `kid`.
    pub fn find(&self, kid: &str) -> Option<VerifyingKey> {
        self.keys
            .iter()
            .filter(|k| k.kid.as_deref() == Some(kid))
            .find_map(Jwk::verifying_key)
    }
}

/// A key id from the public key: the first 16 characters of the base64url
/// SHA-256 of its bytes (stable, no registry needed).
pub fn key_id(key: &VerifyingKey) -> String {
    use sha2::Digest;
    let d = sha2::Sha256::digest(key.as_bytes());
    b64(&d)[..16].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn claims() -> AccessClaims {
        AccessClaims {
            iss: "https://id.example".into(),
            sub: "acc_1".into(),
            aud: "ocw".into(),
            exp: 2000,
            iat: 1000,
            jti: "j".into(),
            client_id: "ocw-desktop".into(),
            scope: "openid".into(),
            name: "Marianne".into(),
            auth_time: 1000,
            providers: vec!["google".into()],
        }
    }

    #[test]
    fn sign_and_verify_round_trip() {
        let k = key(1);
        let kid = key_id(&k.verifying_key());
        let t = sign(&claims(), &k, &kid, TYP_ACCESS);
        let p = Parsed::parse(&t).unwrap();
        assert_eq!(p.header.kid.as_deref(), Some(kid.as_str()));
        assert_eq!(p.header.typ.as_deref(), Some(TYP_ACCESS));
        let c: AccessClaims = p.verify(&k.verifying_key()).unwrap();
        assert_eq!(c, claims());
        // Another key: refused.
        assert_eq!(
            p.verify::<AccessClaims>(&key(2).verifying_key()),
            Err(JwtError::Signature)
        );
    }

    #[test]
    fn tampering_and_other_algorithms_are_refused() {
        let k = key(1);
        let t = sign(&claims(), &k, "k", TYP_ACCESS);
        let mut parts: Vec<String> = t.split('.').map(str::to_owned).collect();
        // Another payload under the same signature.
        let mut evil = claims();
        evil.sub = "acc_admin".into();
        parts[1] = b64(&serde_json::to_vec(&evil).unwrap());
        let forged = parts.join(".");
        assert_eq!(
            Parsed::parse(&forged)
                .unwrap()
                .verify::<AccessClaims>(&k.verifying_key()),
            Err(JwtError::Signature)
        );
        // alg none / HS256: refused when parsing.
        for alg in ["none", "HS256", "RS256"] {
            let h = b64(format!(r#"{{"alg":"{alg}","kid":"k"}}"#).as_bytes());
            let t = format!("{h}.{}.{}", parts[1], parts[2]);
            assert_eq!(Parsed::parse(&t).err(), Some(JwtError::Algorithm), "{alg}");
        }
        for bad in [
            "",
            "a.b",
            "a.b.c.d",
            "!!.!!.!!",
            &"x".repeat(MAX_TOKEN_LEN + 1),
        ] {
            assert!(Parsed::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn jwks_finds_ed25519_keys_and_ignores_others() {
        let k = key(3).verifying_key();
        let rsa: Jwk =
            serde_json::from_str(r#"{"kty":"RSA","n":"AQAB","e":"AQAB","kid":"r","alg":"RS256"}"#)
                .unwrap();
        let set = Jwks {
            keys: vec![rsa, Jwk::ed25519("e", &k)],
        };
        let json = serde_json::to_string(&set).unwrap();
        assert!(json.contains(r#""use":"sig""#), "{json}");
        let back: Jwks = serde_json::from_str(&json).unwrap();
        assert_eq!(back.find("e"), Some(k));
        assert_eq!(back.find("r"), None);
        assert_eq!(back.find("missing"), None);
        let mut enc = Jwk::ed25519("x", &k);
        enc.use_ = Some("enc".into());
        assert_eq!(enc.verifying_key(), None);
    }
}
