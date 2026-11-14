//! TLS for the QUIC transport, development mode (spec §6.4, §12).
//!
//! The server has a self-signed certificate: generated at every start, or
//! kept in a directory (`--tls-dir`) so its fingerprint stays the same.
//! The client trusts it in one of two ways, never silently:
//!
//! - [`ServerTrust::Pinned`]: the SHA-256 fingerprint of exactly this
//!   certificate (`--server-cert`), printed by the server at start. Works for
//!   any address; a different certificate fails the handshake.
//! - [`ServerTrust::InsecureLoopback`] (`--insecure-dev`): any certificate,
//!   but only for a loopback address (a server on the same machine).
//!
//! Both still verify the TLS 1.3 handshake signature, so the peer must hold
//! the certificate's private key. Certificates from a public CA (WebPKI
//! roots) and OIDC session tokens come with the backend (Phase 5).

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use sha2::{Digest, Sha256};

use super::ALPN;

/// The crypto provider: *ring* (see the workspace `Cargo.toml`).
fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// QUIC transport settings shared by both sides.
pub(crate) fn transport(datagrams: bool) -> Arc<quinn::TransportConfig> {
    let mut t = quinn::TransportConfig::default();
    // A dead peer is noticed within 15 s; keep-alives keep idle (paused)
    // connections and NAT bindings open.
    t.max_idle_timeout(Some(
        quinn::IdleTimeout::try_from(Duration::from_secs(15)).expect("valid timeout"),
    ));
    t.keep_alive_interval(Some(Duration::from_secs(4)));
    if !datagrams {
        // Tells the peer we accept no datagrams: it falls back to the stream.
        t.datagram_receive_buffer_size(None);
    }
    Arc::new(t)
}

/// SHA-256 of a DER certificate as lowercase hex.
pub fn fingerprint(cert: &[u8]) -> String {
    Sha256::digest(cert)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Parses a fingerprint: 64 hex digits, optionally separated by `:` or
/// spaces (as `openssl x509 -fingerprint` prints them), any case.
pub fn parse_fingerprint(s: &str) -> Result<[u8; 32], String> {
    let hex: String = s.chars().filter(|c| !matches!(c, ':' | ' ')).collect();
    if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!(
            "{s:?} is not a SHA-256 fingerprint (64 hex digits, as ocw-server prints it)"
        ));
    }
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).expect("checked hex");
    }
    Ok(out)
}

/// The server's certificate and private key.
pub struct ServerIdentity {
    cert: CertificateDer<'static>,
    key: PrivatePkcs8KeyDer<'static>,
}

impl ServerIdentity {
    /// A fresh self-signed certificate for `localhost` and the loopback
    /// addresses (clients pin the certificate, so the names do not matter
    /// for remote use).
    pub fn self_signed() -> Result<Self, String> {
        let names = ["localhost", "127.0.0.1", "::1"].map(String::from).to_vec();
        let ck = rcgen::generate_simple_self_signed(names)
            .map_err(|e| format!("certificate generation failed: {e}"))?;
        Ok(Self {
            cert: ck.cert.der().clone(),
            key: PrivatePkcs8KeyDer::from(ck.signing_key.serialize_der()),
        })
    }

    /// The certificate kept in `dir` (`cert.der`, `key.der`), created on
    /// first use. The key file is readable only by its owner (Unix).
    pub fn load_or_create(dir: &Path) -> Result<Self, String> {
        let (cert_path, key_path) = (dir.join("cert.der"), dir.join("key.der"));
        let ctx = |p: &Path, e: std::io::Error| format!("{}: {e}", p.display());
        if cert_path.exists() || key_path.exists() {
            let cert = std::fs::read(&cert_path).map_err(|e| ctx(&cert_path, e))?;
            let key = std::fs::read(&key_path).map_err(|e| ctx(&key_path, e))?;
            let id = Self {
                cert: CertificateDer::from(cert),
                key: PrivatePkcs8KeyDer::from(key),
            };
            // Fail now, not at the first client.
            id.rustls_config()?;
            return Ok(id);
        }
        let id = Self::self_signed()?;
        std::fs::create_dir_all(dir).map_err(|e| ctx(dir, e))?;
        std::fs::write(&cert_path, id.cert.as_ref()).map_err(|e| ctx(&cert_path, e))?;
        write_private(&key_path, id.key.secret_pkcs8_der()).map_err(|e| ctx(&key_path, e))?;
        Ok(id)
    }

    /// SHA-256 fingerprint of the certificate (what clients pin).
    pub fn fingerprint(&self) -> String {
        fingerprint(&self.cert)
    }

    fn rustls_config(&self) -> Result<rustls::ServerConfig, String> {
        rustls_server_config(&self.cert, &self.key, ALPN)
    }

    pub(crate) fn server_config(&self) -> Result<quinn::ServerConfig, String> {
        quinn_server_config(self.rustls_config()?)
    }
}

fn rustls_server_config(
    cert: &CertificateDer<'static>,
    key: &PrivatePkcs8KeyDer<'static>,
    alpn: &[u8],
) -> Result<rustls::ServerConfig, String> {
    let mut cfg = rustls::ServerConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| e.to_string())?
        .with_no_client_auth()
        .with_single_cert(vec![cert.clone()], PrivateKeyDer::Pkcs8(key.clone_key()))
        .map_err(|e| format!("bad server certificate or key: {e}"))?;
    cfg.alpn_protocols = vec![alpn.to_vec()];
    Ok(cfg)
}

fn quinn_server_config(tls: rustls::ServerConfig) -> Result<quinn::ServerConfig, String> {
    let crypto = QuicServerConfig::try_from(tls).map_err(|e| e.to_string())?;
    let mut cfg = quinn::ServerConfig::with_crypto(Arc::new(crypto));
    cfg.transport_config(transport(true));
    Ok(cfg)
}

/// Seconds per day.
const DAY_S: i64 = 86_400;
/// Browsers accept a certificate by its hash (`serverCertificateHashes`)
/// only if it is valid for at most 14 days (W3C WebTransport, Chrome).
pub const WEB_CERT_MAX_VALIDITY_S: i64 = 14 * DAY_S;
/// Validity of the WebTransport certificate: 13 days, from an hour back
/// (clock skew), so the total stays within the browsers' 14 days.
pub const WEB_CERT_VALIDITY_S: i64 = 13 * DAY_S;
pub const WEB_CERT_BACKDATE_S: i64 = 3600;
/// A new certificate is made when the current one has less than this left
/// (at start with `--tls-dir`, and while running).
pub const WEB_CERT_RENEW_BEFORE_S: i64 = 2 * DAY_S;

/// The WebTransport endpoint's certificate (spec §14 Phase 10 B3): an
/// ECDSA P-256 self-signed certificate valid for 13 days, which browsers
/// accept by its SHA-256 hash (`serverCertificateHashes`, page option
/// `cert-hash`) without a CA. It is renewed (rotated) before it expires;
/// the page then needs the new hash. Production certificates from a CA
/// come with hosting (B8).
pub struct WebIdentity {
    cert: CertificateDer<'static>,
    key: PrivatePkcs8KeyDer<'static>,
    not_before_unix_s: i64,
    not_after_unix_s: i64,
}

impl WebIdentity {
    /// A new certificate valid from `now − 1 h` for 13 days.
    pub fn generate(now_unix_s: i64) -> Result<Self, String> {
        let not_before_unix_s = now_unix_s - WEB_CERT_BACKDATE_S;
        let not_after_unix_s = not_before_unix_s + WEB_CERT_VALIDITY_S;
        let at = |s: i64| {
            time::OffsetDateTime::from_unix_timestamp(s)
                .map_err(|e| format!("certificate time: {e}"))
        };
        let names = ["localhost", "127.0.0.1", "::1"].map(String::from).to_vec();
        let mut params = rcgen::CertificateParams::new(names)
            .map_err(|e| format!("certificate generation failed: {e}"))?;
        params.not_before = at(not_before_unix_s)?;
        params.not_after = at(not_after_unix_s)?;
        // ECDSA P-256: the only key type browsers take by hash.
        let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
            .map_err(|e| format!("key generation failed: {e}"))?;
        let cert = params
            .self_signed(&key)
            .map_err(|e| format!("certificate generation failed: {e}"))?;
        Ok(Self {
            cert: cert.der().clone(),
            key: PrivatePkcs8KeyDer::from(key.serialize_der()),
            not_before_unix_s,
            not_after_unix_s,
        })
    }

    /// The certificate kept in `dir` (`web-cert.der`, `web-key.der`,
    /// `web-cert.validity`), so the hash stays the same across restarts;
    /// a new one when there is none, it is damaged, or it expires within
    /// two days. Returns it and whether it is new.
    pub fn load_or_create(dir: &Path, now_unix_s: i64) -> Result<(Self, bool), String> {
        let read = || -> Option<Self> {
            let validity = std::fs::read_to_string(dir.join("web-cert.validity")).ok()?;
            let mut it = validity.split_whitespace().map(str::parse::<i64>);
            let (Some(Ok(not_before_unix_s)), Some(Ok(not_after_unix_s))) = (it.next(), it.next())
            else {
                return None;
            };
            let id = Self {
                cert: CertificateDer::from(std::fs::read(dir.join("web-cert.der")).ok()?),
                key: PrivatePkcs8KeyDer::from(std::fs::read(dir.join("web-key.der")).ok()?),
                not_before_unix_s,
                not_after_unix_s,
            };
            id.rustls_config().ok()?;
            Some(id)
        };
        if let Some(id) = read().filter(|id| id.usable(now_unix_s)) {
            return Ok((id, false));
        }
        let id = Self::generate(now_unix_s)?;
        id.store(dir)?;
        Ok((id, true))
    }

    /// Writes the certificate into `dir`, replacing an old one.
    pub fn store(&self, dir: &Path) -> Result<(), String> {
        let ctx = |p: &Path, e: std::io::Error| format!("{}: {e}", p.display());
        std::fs::create_dir_all(dir).map_err(|e| ctx(dir, e))?;
        let (cert_path, key_path, validity_path) = (
            dir.join("web-cert.der"),
            dir.join("web-key.der"),
            dir.join("web-cert.validity"),
        );
        for p in [&cert_path, &key_path, &validity_path] {
            match std::fs::remove_file(p) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(ctx(p, e)),
                _ => {}
            }
        }
        std::fs::write(&cert_path, self.cert.as_ref()).map_err(|e| ctx(&cert_path, e))?;
        write_private(&key_path, self.key.secret_pkcs8_der()).map_err(|e| ctx(&key_path, e))?;
        std::fs::write(
            &validity_path,
            format!("{} {}\n", self.not_before_unix_s, self.not_after_unix_s),
        )
        .map_err(|e| ctx(&validity_path, e))
    }

    /// Valid now, within the browsers' 14 days, and not about to expire.
    pub fn usable(&self, now_unix_s: i64) -> bool {
        self.not_after_unix_s - self.not_before_unix_s <= WEB_CERT_MAX_VALIDITY_S
            && now_unix_s >= self.not_before_unix_s
            && !self.needs_rotation(now_unix_s)
    }

    /// Less than two days left: time for a new certificate.
    pub fn needs_rotation(&self, now_unix_s: i64) -> bool {
        now_unix_s >= self.not_after_unix_s - WEB_CERT_RENEW_BEFORE_S
    }

    /// The SHA-256 of the certificate as lowercase hex: the page's
    /// `cert-hash` (`serverCertificateHashes`).
    pub fn cert_hash(&self) -> String {
        fingerprint(&self.cert)
    }

    pub fn not_before_unix_s(&self) -> i64 {
        self.not_before_unix_s
    }

    pub fn not_after_unix_s(&self) -> i64 {
        self.not_after_unix_s
    }

    /// The DER certificate (tests check its fields).
    pub fn cert_der(&self) -> &[u8] {
        self.cert.as_ref()
    }

    fn rustls_config(&self) -> Result<rustls::ServerConfig, String> {
        rustls_server_config(&self.cert, &self.key, super::webtransport::ALPN)
    }

    pub(crate) fn server_config(&self) -> Result<quinn::ServerConfig, String> {
        quinn_server_config(self.rustls_config()?)
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    opts.open(path)?.write_all(bytes)
}

/// How the client trusts the server's certificate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerTrust {
    /// Only the certificate with this SHA-256 fingerprint.
    Pinned([u8; 32]),
    /// Any certificate, for a loopback address only (development).
    InsecureLoopback,
}

impl ServerTrust {
    /// From the client options: a pin (`--server-cert`) or `--insecure-dev`,
    /// which is refused for an address that is not loopback.
    pub fn for_server(
        addr: SocketAddr,
        pin: Option<&str>,
        insecure_dev: bool,
    ) -> Result<Self, String> {
        match (pin, insecure_dev) {
            (Some(p), _) => Ok(Self::Pinned(parse_fingerprint(p)?)),
            (None, true) if addr.ip().is_loopback() => Ok(Self::InsecureLoopback),
            (None, true) => Err(format!(
                "--insecure-dev only works for a server on this machine (127.0.0.1, ::1); \
                 for {addr} pin its certificate with --server-cert SHA256"
            )),
            (None, false) => Err(
                "the server's certificate must be trusted: pass --server-cert SHA256 \
                 (ocw-server prints it at start) or, for a server on this machine, --insecure-dev"
                    .into(),
            ),
        }
    }

    pub(crate) fn client_config(
        &self,
        addr: SocketAddr,
        datagrams: bool,
    ) -> Result<quinn::ClientConfig, String> {
        self.client_config_for(addr, datagrams, ALPN)
    }

    /// The client configuration for a protocol (`ocw/1`, or `h3` for
    /// WebTransport).
    pub(crate) fn client_config_for(
        &self,
        addr: SocketAddr,
        datagrams: bool,
        alpn: &[u8],
    ) -> Result<quinn::ClientConfig, String> {
        let pin = match self {
            Self::Pinned(p) => Some(*p),
            // Checked again here: no path may skip verification remotely.
            Self::InsecureLoopback if addr.ip().is_loopback() => None,
            Self::InsecureLoopback => {
                return Err(format!("refusing an unverified connection to {addr}"));
            }
        };
        let provider = provider();
        let mut cfg = rustls::ClientConfig::builder_with_provider(provider.clone())
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|e| e.to_string())?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(DevVerifier { pin, provider }))
            .with_no_client_auth();
        cfg.alpn_protocols = vec![alpn.to_vec()];
        let crypto = QuicClientConfig::try_from(cfg).map_err(|e| e.to_string())?;
        let mut cfg = quinn::ClientConfig::new(Arc::new(crypto));
        cfg.transport_config(transport(datagrams));
        Ok(cfg)
    }
}

/// Checks the end-entity certificate against the pin (or accepts any, for
/// loopback); handshake signatures are always verified.
#[derive(Debug)]
struct DevVerifier {
    pin: Option<[u8; 32]>,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for DevVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        match self.pin {
            Some(pin) if Sha256::digest(end_entity.as_ref()).as_slice() != pin => {
                Err(rustls::Error::General(format!(
                    "server certificate {} does not match --server-cert",
                    fingerprint(end_entity)
                )))
            }
            _ => Ok(ServerCertVerified::assertion()),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprints_parse_with_separators_and_any_case() {
        let id = ServerIdentity::self_signed().unwrap();
        let fp = id.fingerprint();
        assert_eq!(fp.len(), 64);
        let bytes = parse_fingerprint(&fp).unwrap();
        let colons: Vec<String> = bytes.iter().map(|b| format!("{b:02X}")).collect();
        assert_eq!(parse_fingerprint(&colons.join(":")).unwrap(), bytes);
        assert!(parse_fingerprint("abc").is_err());
        assert!(parse_fingerprint(&"g".repeat(64)).is_err());
        // Each generated certificate is new.
        assert_ne!(ServerIdentity::self_signed().unwrap().fingerprint(), fp);
    }

    #[test]
    fn insecure_mode_only_for_loopback() {
        let local: SocketAddr = "127.0.0.1:47800".parse().unwrap();
        let local6: SocketAddr = "[::1]:47800".parse().unwrap();
        let remote: SocketAddr = "192.0.2.10:47800".parse().unwrap();
        assert_eq!(
            ServerTrust::for_server(local, None, true),
            Ok(ServerTrust::InsecureLoopback)
        );
        assert_eq!(
            ServerTrust::for_server(local6, None, true),
            Ok(ServerTrust::InsecureLoopback)
        );
        let err = ServerTrust::for_server(remote, None, true).unwrap_err();
        assert!(err.contains("--server-cert"), "{err}");
        // Nothing given: no silent default.
        assert!(ServerTrust::for_server(local, None, false).is_err());
        // A pin works for any address.
        let pin = "ab".repeat(32);
        assert!(matches!(
            ServerTrust::for_server(remote, Some(&pin), false),
            Ok(ServerTrust::Pinned(_))
        ));
        // Even if constructed directly, the config refuses a remote address.
        assert!(
            ServerTrust::InsecureLoopback
                .client_config(remote, true)
                .is_err()
        );
        assert!(
            ServerTrust::InsecureLoopback
                .client_config(local, true)
                .is_ok()
        );
    }

    /// The two validity times (notBefore, notAfter) of a DER certificate:
    /// the first two UTCTime values (tag 0x17, 13 bytes `YYMMDDHHMMSSZ`).
    fn der_times(der: &[u8]) -> Vec<String> {
        let mut out = Vec::new();
        let mut i = 0;
        while i + 15 <= der.len() && out.len() < 2 {
            if der[i] == 0x17 && der[i + 1] == 13 {
                let t = &der[i + 2..i + 15];
                if t[12] == b'Z' && t[..12].iter().all(u8::is_ascii_digit) {
                    out.push(String::from_utf8_lossy(t).into_owned());
                    i += 15;
                    continue;
                }
            }
            i += 1;
        }
        out
    }

    /// 2026-11-14 12:00:00 UTC.
    const NOW: i64 = 1_794_657_600;

    #[test]
    fn the_webtransport_certificate_suits_browsers() {
        let id = WebIdentity::generate(NOW).unwrap();
        assert_eq!(id.not_before_unix_s(), NOW - 3600);
        assert_eq!(id.not_after_unix_s() - id.not_before_unix_s(), 13 * DAY_S);
        assert!(id.usable(NOW));
        // The hash is the SHA-256 of the DER certificate, 64 hex digits.
        assert_eq!(id.cert_hash(), fingerprint(id.cert_der()));
        assert_eq!(parse_fingerprint(&id.cert_hash()).unwrap().len(), 32);
        assert_ne!(
            WebIdentity::generate(NOW).unwrap().cert_hash(),
            id.cert_hash()
        );
        // The certificate itself carries that validity (≤ 14 days).
        assert_eq!(der_times(id.cert_der()), ["261114110000Z", "261127110000Z"]);
        // ECDSA P-256 (id-ecPublicKey 1.2.840.10045.2.1, prime256v1
        // 1.2.840.10045.3.1.7): the only key browsers accept by hash.
        let der = id.cert_der();
        let has = |oid: &[u8]| der.windows(oid.len()).any(|w| w == oid);
        assert!(has(&[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01]));
        assert!(has(&[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07]));
        // Expiry: renewed two days before the end, unusable after it.
        let end = id.not_after_unix_s();
        assert!(!id.needs_rotation(end - 2 * DAY_S - 1));
        assert!(id.needs_rotation(end - 2 * DAY_S));
        assert!(!id.usable(end));
        assert!(!id.usable(id.not_before_unix_s() - 1));
        assert!(id.server_config().is_ok());
    }

    #[test]
    fn the_webtransport_certificate_is_kept_and_rotated() {
        let dir = std::env::temp_dir().join(format!("ocw-webtls-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (first, new) = WebIdentity::load_or_create(&dir, NOW).unwrap();
        assert!(new);
        // A restart a week later: the same certificate (and hash).
        let (again, new) = WebIdentity::load_or_create(&dir, NOW + 7 * DAY_S).unwrap();
        assert!(!new);
        assert_eq!(again.cert_hash(), first.cert_hash());
        assert_eq!(again.not_after_unix_s(), first.not_after_unix_s());
        // Eleven days later it has less than two days left: a new one.
        let later = NOW + 11 * DAY_S;
        let (rotated, new) = WebIdentity::load_or_create(&dir, later).unwrap();
        assert!(new);
        assert_ne!(rotated.cert_hash(), first.cert_hash());
        assert_eq!(rotated.not_before_unix_s(), later - 3600);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join("web-key.der"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o077, 0, "key readable by others: {mode:o}");
        }
        // A damaged file: a new certificate (only a development
        // certificate the page gets by hash, nothing is lost).
        std::fs::write(dir.join("web-key.der"), b"junk").unwrap();
        let (fresh, new) = WebIdentity::load_or_create(&dir, later).unwrap();
        assert!(new && fresh.cert_hash() != rotated.cert_hash());
        // A validity longer than browsers accept is not reused.
        std::fs::write(
            dir.join("web-cert.validity"),
            format!("{} {}", later - 3600, later + 30 * DAY_S),
        )
        .unwrap();
        assert!(WebIdentity::load_or_create(&dir, later).unwrap().1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn identity_is_kept_in_its_directory() {
        let dir = std::env::temp_dir().join(format!("ocw-tls-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let first = ServerIdentity::load_or_create(&dir).unwrap();
        let again = ServerIdentity::load_or_create(&dir).unwrap();
        assert_eq!(first.fingerprint(), again.fingerprint());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join("key.der"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o077, 0, "key readable by others: {mode:o}");
        }
        // A damaged key is reported, not replaced.
        std::fs::write(dir.join("key.der"), b"junk").unwrap();
        assert!(ServerIdentity::load_or_create(&dir).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
