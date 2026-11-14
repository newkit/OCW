//! The WebTransport endpoint for the browser app (spec §6.2, §14 Phase 10
//! B3): HTTP/3 over QUIC on its own UDP port (default [`DEFAULT_PORT`]),
//! next to the native endpoint, with the same messages the same way – the
//! client opens one bidirectional stream of length-prefixed frames for the
//! reliable messages, snapshots and samples go as datagrams (WebTransport
//! datagrams: QUIC datagrams with the session id in front).
//!
//! Library: `web-transport-quinn` (MIT OR Apache-2.0) does HTTP/3's
//! SETTINGS and the extended CONNECT on a `quinn::Connection` of our own
//! endpoint (our rustls config, the *ring* provider); its streams are
//! tokio `AsyncRead` / `AsyncWrite`, so [`super::framing`] is reused.
//!
//! Certificates: the browser accepts the server's self-signed certificate
//! by its SHA-256 (`serverCertificateHashes`) if it is ECDSA and valid for
//! at most 14 days ([`super::tls::WebIdentity`]: 13 days, renewed two days
//! before the end). Certificates from a CA come with hosting (B8).

use std::net::{SocketAddr, ToSocketAddrs};

/// HTTP/3 ALPN (WebTransport needs it).
pub const ALPN: &[u8] = b"h3";

/// Default UDP port of the WebTransport endpoint (the native one is
/// [`super::DEFAULT_PORT`]).
pub const DEFAULT_PORT: u16 = 47801;

/// The session's path: the protocol version, like the native ALPN `ocw/1`.
pub const PATH: &str = "/ocw/1";

/// The paths a session may ask for: [`PATH`], or the bare origin.
pub fn accepts_path(path: &str) -> bool {
    matches!(path, "" | "/" | PATH)
}

/// Parses a WebTransport URL (`https://HOST[:PORT][/path]`; the port
/// defaults to [`DEFAULT_PORT`], the path to [`PATH`]) and resolves it.
pub fn resolve_url(spec: &str) -> Result<(url::Url, SocketAddr), String> {
    let mut url = url::Url::parse(spec.trim()).map_err(|e| format!("{spec:?}: {e}"))?;
    if url.scheme() != "https" {
        return Err(format!(
            "{spec:?}: WebTransport needs an https:// URL (e.g. https://127.0.0.1:{DEFAULT_PORT})"
        ));
    }
    let host = url
        .host_str()
        .ok_or_else(|| format!("{spec:?}: no host"))?
        .trim_matches(['[', ']'])
        .to_owned();
    if url.port().is_none() {
        let _ = url.set_port(Some(DEFAULT_PORT));
    }
    if matches!(url.path(), "" | "/") {
        url.set_path(PATH);
    }
    let port = url.port().unwrap_or(DEFAULT_PORT);
    let addr = (host.as_str(), port)
        .to_socket_addrs()
        .map_err(|e| format!("{spec}: {e}"))?
        .next()
        .ok_or_else(|| format!("{spec}: no address"))?;
    Ok((url, addr))
}

/// The URL browsers connect to for a bound address (`HOST` for an
/// unspecified one).
pub fn url_for(addr: SocketAddr) -> String {
    let host = if addr.ip().is_unspecified() {
        "HOST".to_owned()
    } else if addr.is_ipv6() {
        format!("[{}]", addr.ip())
    } else {
        addr.ip().to_string()
    };
    format!("https://{host}:{}", addr.port())
}

/// The page options that join this server (`server`, `cert-hash`; the
/// value escaped for a query string).
pub fn page_query(url: &str, cert_hash: &str) -> String {
    let esc: String = url
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            b => format!("%{b:02X}"),
        })
        .collect();
    format!("server={esc}&cert-hash={cert_hash}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_get_the_default_port_and_path() {
        let (u, a) = resolve_url("https://127.0.0.1").unwrap();
        assert_eq!(u.as_str(), "https://127.0.0.1:47801/ocw/1");
        assert_eq!(a.port(), DEFAULT_PORT);
        let (u, a) = resolve_url("https://127.0.0.1:5000/").unwrap();
        assert_eq!((u.path(), a.port()), (PATH, 5000));
        let (u, a) = resolve_url("https://[::1]:6000/ocw/1").unwrap();
        assert!(a.is_ipv6() && a.port() == 6000);
        assert_eq!(u.path(), PATH);
        let (_, a) = resolve_url("https://localhost:7000").unwrap();
        assert!(a.ip().is_loopback());
        assert!(resolve_url("http://127.0.0.1:47801").is_err());
        assert!(resolve_url("127.0.0.1:47801").is_err());
        assert!(accepts_path("/ocw/1") && accepts_path("/") && !accepts_path("/admin"));
    }

    #[test]
    fn the_page_query_escapes_the_url() {
        let addr: SocketAddr = "127.0.0.1:47801".parse().unwrap();
        assert_eq!(url_for(addr), "https://127.0.0.1:47801");
        assert_eq!(
            url_for("[::1]:9".parse().unwrap()),
            "https://[::1]:9".to_owned()
        );
        assert_eq!(
            url_for("0.0.0.0:47801".parse().unwrap()),
            "https://HOST:47801"
        );
        assert_eq!(
            page_query("https://127.0.0.1:47801", "ab12"),
            "server=https%3A%2F%2F127.0.0.1%3A47801&cert-hash=ab12"
        );
    }
}
