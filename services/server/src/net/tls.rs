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
        let mut cfg = rustls::ServerConfig::builder_with_provider(provider())
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|e| e.to_string())?
            .with_no_client_auth()
            .with_single_cert(
                vec![self.cert.clone()],
                PrivateKeyDer::Pkcs8(self.key.clone_key()),
            )
            .map_err(|e| format!("bad server certificate or key: {e}"))?;
        cfg.alpn_protocols = vec![ALPN.to_vec()];
        Ok(cfg)
    }

    pub(crate) fn server_config(&self) -> Result<quinn::ServerConfig, String> {
        let crypto =
            QuicServerConfig::try_from(self.rustls_config()?).map_err(|e| e.to_string())?;
        let mut cfg = quinn::ServerConfig::with_crypto(Arc::new(crypto));
        cfg.transport_config(transport(true));
        Ok(cfg)
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
        cfg.alpn_protocols = vec![ALPN.to_vec()];
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
