//! The bridge's certificate, derived from the pairing secret (spec 14
//! Phase 10, 10D.2 *Built (local bridge)*).
//!
//! A page served over HTTPS (GitHub Pages) may open WebTransport to an
//! address on the local network only with a certificate it trusts; a
//! self-signed one is accepted **by its SHA-256** (`serverCertificateHashes`)
//! if it is ECDSA and valid for at most 14 days. The browser has only the
//! typed code, so both sides build the **same certificate from the secret**:
//!
//! - the key: an ECDSA P-256 scalar from SHA-256 over the secret,
//! - the validity: the week (Unix time / 7 days) it belongs to, from one day
//!   before the week's start for 13 days (so it is valid all that week and
//!   five days more, within the browsers' 14),
//! - every other field fixed, and the signature deterministic (RFC 6979),
//!
//! so the DER bytes and their hash are equal everywhere. The browser offers
//! the hashes of the previous, this and the next week ([`browser_hashes`])
//! and tolerates a clock a few days off. Anyone who has the code can derive
//! the certificate; the code is the shared secret, shown on the desktop and
//! typed by the rider, and the endpoint takes connections from the local
//! network only.

use p256::ecdsa::signature::Signer;
use p256::ecdsa::{Signature, SigningKey};
use sha2::{Digest, Sha256};

const DAY_S: i64 = 86_400;
/// A certificate's week.
pub const WEEK_S: i64 = 7 * DAY_S;
/// Valid from a day before its week's start …
pub const BACKDATE_S: i64 = DAY_S;
/// … for 13 days (browsers take at most 14 by hash).
pub const VALIDITY_S: i64 = 13 * DAY_S;

/// The week of a Unix time.
pub fn week_of(unix_s: i64) -> i64 {
    unix_s.div_euclid(WEEK_S)
}

/// A derived certificate and its key.
#[derive(Clone)]
pub struct BridgeCert {
    pub week: i64,
    /// The certificate (DER).
    pub der: Vec<u8>,
    /// The private key (PKCS #8 DER) for the TLS server.
    pub key_pkcs8: Vec<u8>,
}

impl std::fmt::Debug for BridgeCert {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BridgeCert")
            .field("week", &self.week)
            .field("hash", &hex(&self.hash()))
            .finish_non_exhaustive()
    }
}

impl BridgeCert {
    /// The certificate of `secret` for `week`.
    pub fn derive(secret: u32, week: i64) -> Self {
        let key = signing_key(secret);
        let point = key.verifying_key().to_sec1_point(false);
        let public = point.as_bytes().to_vec();
        let not_before = week * WEEK_S - BACKDATE_S;
        let not_after = not_before + VALIDITY_S;
        let tbs = tbs_certificate(secret, week, &public, not_before, not_after);
        let sig: Signature = key.sign(&tbs);
        let (r, s) = sig.split_bytes();
        let sig_der = seq(&[der_uint(&r), der_uint(&s)].concat());
        let der = seq(&[tbs, ecdsa_with_sha256(), bit_string(&sig_der)].concat());
        let key_pkcs8 = pkcs8(&key.to_bytes(), &public);
        Self {
            week,
            der,
            key_pkcs8,
        }
    }

    /// The certificate the desktop uses at `unix_s`.
    pub fn current(secret: u32, unix_s: i64) -> Self {
        Self::derive(secret, week_of(unix_s))
    }

    /// SHA-256 of the DER: what `serverCertificateHashes` takes.
    pub fn hash(&self) -> [u8; 32] {
        Sha256::digest(&self.der).into()
    }

    /// Validity (Unix s): not before, not after.
    pub fn validity(&self) -> (i64, i64) {
        let nb = self.week * WEEK_S - BACKDATE_S;
        (nb, nb + VALIDITY_S)
    }
}

/// The hashes a browser offers for a code at its time `unix_s`: the last,
/// this and the next week's certificate.
pub fn browser_hashes(secret: u32, unix_s: i64) -> Vec<[u8; 32]> {
    let w = week_of(unix_s);
    (w - 1..=w + 1)
        .map(|week| BridgeCert::derive(secret, week).hash())
        .collect()
}

/// Lower-case hex.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn signing_key(secret: u32) -> SigningKey {
    // A scalar ≥ the group order (or zero) is rejected: the next counter
    // (practically never needed, 1 in 2^32).
    (0u32..)
        .find_map(|n| {
            let h = Sha256::new()
                .chain_update(b"ocw-bridge-key-v1")
                .chain_update(secret.to_le_bytes())
                .chain_update(n.to_le_bytes())
                .finalize();
            SigningKey::from_slice(&h).ok()
        })
        .expect("a valid scalar")
}

// --- A minimal DER writer: just what this certificate needs. ---

fn len(n: usize) -> Vec<u8> {
    match n {
        0..=127 => vec![n as u8],
        128..=255 => vec![0x81, n as u8],
        _ => vec![0x82, (n >> 8) as u8, n as u8],
    }
}

fn tlv(tag: u8, body: &[u8]) -> Vec<u8> {
    [&[tag][..], &len(body.len()), body].concat()
}

fn seq(body: &[u8]) -> Vec<u8> {
    tlv(0x30, body)
}

fn set(body: &[u8]) -> Vec<u8> {
    tlv(0x31, body)
}

/// A non-negative INTEGER from big-endian bytes.
fn der_uint(bytes: &[u8]) -> Vec<u8> {
    let mut b: &[u8] = bytes;
    while b.len() > 1 && b[0] == 0 {
        b = &b[1..];
    }
    let mut body = Vec::with_capacity(b.len() + 1);
    if b.first().is_some_and(|&x| x & 0x80 != 0) {
        body.push(0);
    }
    body.extend_from_slice(b);
    tlv(0x02, &body)
}

fn bit_string(bytes: &[u8]) -> Vec<u8> {
    tlv(0x03, &[&[0u8][..], bytes].concat())
}

fn octet_string(bytes: &[u8]) -> Vec<u8> {
    tlv(0x04, bytes)
}

/// An OBJECT IDENTIFIER from its encoded body.
fn oid(body: &[u8]) -> Vec<u8> {
    tlv(0x06, body)
}

/// 1.2.840.10045.4.3.2
const OID_ECDSA_SHA256: &[u8] = &[0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x04, 0x03, 0x02];
/// 1.2.840.10045.2.1
const OID_EC_PUBLIC_KEY: &[u8] = &[0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x02, 0x01];
/// 1.2.840.10045.3.1.7 (prime256v1)
const OID_P256: &[u8] = &[0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x03, 0x01, 0x07];
/// 2.5.4.3 (commonName)
const OID_CN: &[u8] = &[0x55, 0x04, 0x03];
/// 2.5.29.17 (subjectAltName)
const OID_SAN: &[u8] = &[0x55, 0x1D, 0x11];

/// The certificate's name in issuer, subject and the DNS name.
pub const NAME: &str = "ocw-bridge.local";

fn ecdsa_with_sha256() -> Vec<u8> {
    seq(&oid(OID_ECDSA_SHA256))
}

fn name() -> Vec<u8> {
    seq(&set(&seq(&[oid(OID_CN), tlv(0x0C, NAME.as_bytes())].concat())))
}

fn tbs_certificate(
    secret: u32,
    week: i64,
    public: &[u8],
    not_before: i64,
    not_after: i64,
) -> Vec<u8> {
    let serial: [u8; 32] = Sha256::new()
        .chain_update(b"ocw-bridge-serial")
        .chain_update(secret.to_le_bytes())
        .chain_update(week.to_le_bytes())
        .finalize()
        .into();
    // A positive serial of at most 20 octets.
    let mut serial = serial[..16].to_vec();
    serial[0] = (serial[0] & 0x7F) | 0x01;
    let version = tlv(0xA0, &der_uint(&[2]));
    let validity = seq(&[utc_time(not_before), utc_time(not_after)].concat());
    let spki = seq(&[
        seq(&[oid(OID_EC_PUBLIC_KEY), oid(OID_P256)].concat()),
        bit_string(public),
    ]
    .concat());
    // subjectAltName: dNSName [2].
    let san = seq(&[
        oid(OID_SAN),
        octet_string(&seq(&tlv(0x82, NAME.as_bytes()))),
    ]
    .concat());
    let extensions = tlv(0xA3, &seq(&san));
    seq(&[
        version,
        der_uint(&serial),
        ecdsa_with_sha256(),
        name(),
        validity,
        name(),
        spki,
        extensions,
    ]
    .concat())
}

/// UTCTime `YYMMDDHHMMSSZ` (years 1950 – 2049).
fn utc_time(unix_s: i64) -> Vec<u8> {
    let (y, m, d) = civil_from_days(unix_s.div_euclid(DAY_S));
    let t = unix_s.rem_euclid(DAY_S);
    let s = format!(
        "{:02}{:02}{:02}{:02}{:02}{:02}Z",
        y.rem_euclid(100),
        m,
        d,
        t / 3600,
        t % 3600 / 60,
        t % 60
    );
    tlv(0x17, s.as_bytes())
}

/// Days since 1970-01-01 → (year, month, day) (H. Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// PKCS #8 v1 around a SEC 1 ECPrivateKey with the public key.
fn pkcs8(secret: &[u8], public: &[u8]) -> Vec<u8> {
    let ec_private = seq(&[
        der_uint(&[1]),
        octet_string(secret),
        tlv(0xA1, &bit_string(public)),
    ]
    .concat());
    seq(&[
        der_uint(&[0]),
        seq(&[oid(OID_EC_PUBLIC_KEY), oid(OID_P256)].concat()),
        octet_string(&ec_private),
    ]
    .concat())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-11-24 12:00:00 UTC.
    const NOW: i64 = 1_795_521_600;

    #[test]
    fn the_certificate_is_the_same_wherever_it_is_derived() {
        let a = BridgeCert::current(0x1234_5678 & 0x3FFF_FFFF, NOW);
        let b = BridgeCert::current(0x1234_5678 & 0x3FFF_FFFF, NOW + 3600);
        assert_eq!(a.der, b.der, "deterministic within the week");
        assert_eq!(a.hash(), b.hash());
        // Another secret, another week: another certificate.
        assert_ne!(BridgeCert::current(7, NOW).hash(), a.hash());
        assert_ne!(BridgeCert::current(0x1234_5678 & 0x3FFF_FFFF, NOW + WEEK_S).hash(), a.hash());
        // The browser offers last, this and next week's hash.
        let hashes = browser_hashes(0x1234_5678 & 0x3FFF_FFFF, NOW);
        assert_eq!(hashes.len(), 3);
        assert_eq!(hashes[1], a.hash());
        // A browser clock three days off still offers the desktop's.
        assert!(browser_hashes(0x1234_5678 & 0x3FFF_FFFF, NOW + 3 * DAY_S).contains(&a.hash()));
    }

    #[test]
    fn validity_covers_the_week_within_fourteen_days() {
        for t in [NOW, 0, WEEK_S * 3000 + 5, NOW - WEEK_S + 1] {
            let c = BridgeCert::current(42, t);
            let (nb, na) = c.validity();
            assert!(nb <= t && t < na, "valid at {t}");
            assert!(na - nb <= 14 * DAY_S);
            // Valid through the end of its week.
            assert!(na >= (c.week + 1) * WEEK_S);
        }
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_781), (2026, 11, 24));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        assert_eq!(utc_time(NOW)[2..], *b"261124120000Z");
    }

    #[test]
    fn the_der_is_well_formed() {
        let c = BridgeCert::current(99, NOW);
        // Certificate ::= SEQUENCE { tbs, alg, sig } and its length covers
        // every byte.
        assert_eq!(c.der[0], 0x30);
        let (hdr, body) = match c.der[1] {
            0x82 => (4, ((c.der[2] as usize) << 8) | c.der[3] as usize),
            0x81 => (3, c.der[2] as usize),
            n => (2, n as usize),
        };
        assert_eq!(hdr + body, c.der.len());
        // The key in the PKCS #8 is the derived one, and its uncompressed
        // public point is in the certificate's SPKI.
        let key = signing_key(99);
        let secret = key.to_bytes();
        assert!(c.key_pkcs8.windows(32).any(|w| w == &secret[..]));
        let public = key.verifying_key().to_sec1_point(false);
        assert_eq!(public.as_bytes().len(), 65);
        assert!(c.der.windows(65).any(|w| w == public.as_bytes()));
        assert!(c.der.windows(OID_P256.len()).any(|w| w == OID_P256));
        assert!(c.der.windows(NAME.len()).any(|w| w == NAME.as_bytes()));
        assert!(c.der.len() < 600);
        // INTEGER encoding: leading zeros dropped, a sign octet added.
        assert_eq!(der_uint(&[0, 0, 5]), [2, 1, 5]);
        assert_eq!(der_uint(&[0x80]), [2, 2, 0, 0x80]);
        assert_eq!(der_uint(&[0]), [2, 1, 0]);
    }
}
