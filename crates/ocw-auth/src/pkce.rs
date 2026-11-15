//! PKCE with S256 (RFC 7636): the only method we send and accept (spec
//! §9.1, OAuth 2.0 Security BCP, RFC 9700 §2.1.1).

use sha2::Digest;

use crate::b64;

/// The only accepted `code_challenge_method`.
pub const METHOD: &str = "S256";

/// A code verifier from 32 random bytes (43 base64url characters).
pub fn verifier(random: &[u8; 32]) -> String {
    b64(random)
}

/// `BASE64URL(SHA256(verifier))`.
pub fn challenge(verifier: &str) -> String {
    b64(&sha2::Sha256::digest(verifier.as_bytes()))
}

/// A well-formed verifier: 43–128 characters of `[A-Za-z0-9-._~]`.
pub fn valid_verifier(v: &str) -> bool {
    (43..=128).contains(&v.len())
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~'))
}

/// A well-formed S256 challenge: 43 base64url characters.
pub fn valid_challenge(c: &str) -> bool {
    c.len() == 43
        && c.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

/// Whether `verifier` belongs to `challenge`.
pub fn matches(verifier: &str, challenge_value: &str) -> bool {
    valid_verifier(verifier) && crate::constant_time_eq(&challenge(verifier), challenge_value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc7636_appendix_b() {
        // The example of RFC 7636, Appendix B.
        let v = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(challenge(v), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        assert!(matches(v, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"));
        assert!(!matches(v, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cX"));
        assert!(valid_challenge(&challenge(v)));
    }

    #[test]
    fn verifier_shape() {
        let v = verifier(&[7; 32]);
        assert_eq!(v.len(), 43);
        assert!(valid_verifier(&v));
        assert!(!valid_verifier("short"));
        assert!(!valid_verifier(&"a".repeat(129)));
        assert!(!valid_verifier(&format!("{}+", "a".repeat(43))));
        assert!(!matches("short", &challenge("short")));
        assert!(!valid_challenge("plain-text-challenge"));
    }
}
