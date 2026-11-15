//! Sign-in for Open Cycling World (spec §9.1), the parts every side shares.
//!
//! Our accounts backend (`services/accounts`) is the only token issuer for
//! the apps. It brokers the sign-in with Google (OpenID Connect) and
//! Strava (OAuth 2) and issues its own tokens through endpoints shaped
//! like an OpenID Connect provider, so Keycloak or Authentik can replace
//! it later without changing the apps. This crate holds what the backend,
//! the game server and the apps have in common:
//!
//! - [`jwt`]: compact JWS with EdDSA (Ed25519), the access and ID token
//!   claims, the JSON Web Key Set the backend publishes,
//! - [`verify`]: the game server's check of an access token (signature
//!   against the JWKS, expiry, issuer, audience, token type),
//! - [`pkce`]: PKCE S256 (RFC 7636),
//! - [`oidc`]: the wire types (discovery document, token and error
//!   responses, the device authorization of RFC 8628, userinfo),
//! - [`client`]: the apps' side – the authorization request, the loopback
//!   redirect of RFC 8252, the token forms, the stored token set.
//!
//! Pure and wasm-safe: no clock, randomness or I/O. Callers pass the time
//! (Unix seconds) and random bytes.

pub mod client;
pub mod jwt;
pub mod oidc;
pub mod pkce;
pub mod verify;

pub use jwt::{AccessClaims, IdClaims, Jwk, Jwks};
pub use oidc::Provider;
pub use verify::{Verifier, VerifyError};

/// The audience of our access tokens: the game servers and the backend's
/// own account API (spec §9.1).
pub const DEFAULT_AUDIENCE: &str = "ocw";

/// The apps' client ids (public clients with PKCE, no secret).
pub const CLIENT_DESKTOP: &str = "ocw-desktop";
pub const CLIENT_WEB: &str = "ocw-web";
pub const CLIENT_KIOSK: &str = "ocw-kiosk";

/// Base64url without padding (JWS, PKCE).
pub(crate) fn b64(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub(crate) fn unb64(s: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(s.as_bytes())
        .ok()
}

/// An opaque random token: 32 random bytes as base64url (43 characters).
/// Refresh tokens, authorization codes, `state`, `nonce`.
pub fn random_token(bytes: &[u8; 32]) -> String {
    b64(bytes)
}

/// SHA-256 of a secret token as hex: what the backend stores instead of
/// refresh tokens and codes (hashed at rest).
pub fn token_hash(token: &str) -> String {
    use sha2::Digest;
    let d = sha2::Sha256::digest(token.as_bytes());
    d.iter().map(|b| format!("{b:02x}")).collect()
}

/// Equal strings, compared in time independent of where they differ.
pub fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_and_comparisons() {
        assert_eq!(token_hash("abc").len(), 64);
        assert_ne!(token_hash("abc"), token_hash("abd"));
        assert!(constant_time_eq("same", "same"));
        assert!(!constant_time_eq("same", "sama"));
        assert!(!constant_time_eq("same", "same!"));
        assert_eq!(random_token(&[0; 32]).len(), 43);
        assert_eq!(unb64(&b64(b"\x00\xffhi")).unwrap(), b"\x00\xffhi");
        assert_eq!(unb64("not base64!"), None);
    }
}
