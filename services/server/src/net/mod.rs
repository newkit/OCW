//! QUIC transport for the game protocol (spec §6.2, Phase 5 groundwork).
//!
//! [`GameServer`](crate::GameServer) stays transport-independent: this
//! module only carries its encoded `ClientMessage` / `ServerMessage` bytes
//! between processes.
//!
//! - [`NetServer`] listens on a UDP port and maps every QUIC connection to a
//!   [`ConnectionId`](crate::ConnectionId); the game loop pumps it once per
//!   tick ([`NetServer::pump`], [`NetServer::broadcast`]).
//! - [`NetClient`] connects to one; it offers the same "bytes in, bytes out"
//!   channel interface as the in-process [`LocalConnection`](crate::local::LocalConnection).
//!
//! **Channels.** Each connection has one bidirectional stream, opened by the
//! client, carrying length-prefixed frames ([`framing`]) of every reliable
//! message: Hello / Welcome / Reject, steering and turn intents, U-turns,
//! dev commands and path updates. The two message kinds that are superseded
//! by the next one go as unreliable QUIC datagrams: 10 Hz snapshots (server
//! → client) and sensor samples (client → server, sequence-numbered; the
//! server ignores older ones). A datagram that is larger than the
//! connection allows (a big field's snapshot) or a peer without datagram
//! support falls back to the stream.
//!
//! **Browsers (spec §14 Phase 10 B3).** [`NetServer::listen_webtransport`]
//! adds a WebTransport endpoint (HTTP/3, [`webtransport`]) on its own UDP
//! port feeding the same `GameServer` with the same messages; desktop,
//! bot and browser riders share one session. [`NetClient::connect_web`]
//! is the same client over WebTransport (tests, `ocw-bot --server https://…`).
//!
//! **Security (dev mode).** TLS 1.3 is always on. Until real certificates
//! and OIDC exist, the server uses a self-signed certificate ([`tls`]) and
//! prints its SHA-256 fingerprint; clients pin it (`--server-cert`), or, for
//! a server on the same machine only, skip the check (`--insecure-dev`).

pub mod client;
pub mod framing;
pub mod server;
pub mod tls;
pub mod webtransport;

pub use client::{ClientOptions, LinkStatus, NetClient};
pub use server::{EndpointCounters, NetServer, Transport};
pub use tls::{ServerIdentity, ServerTrust, WebIdentity};

use std::net::{SocketAddr, ToSocketAddrs};

/// Default UDP port of the game server (registered range, not assigned by
/// IANA as far as we know; `--listen` / `--server` take another).
pub const DEFAULT_PORT: u16 = 47800;

/// ALPN protocol id: both sides refuse a QUIC peer speaking something else.
pub const ALPN: &[u8] = b"ocw/1";

/// Close codes (QUIC application error codes).
pub mod close_code {
    /// Normal close (client quits, server shuts down).
    pub const NORMAL: u32 = 0;
    /// The peer broke the framing or sent no stream in time.
    pub const PROTOCOL: u32 = 1;
    /// The client could not keep up with the server's reliable messages.
    pub const TOO_SLOW: u32 = 2;
}

pub use crate::wire::{client_unreliable, oneof_field, server_unreliable};

/// A server address from the command line: `HOST:PORT`, `HOST` (default
/// port), `IP`, `[IPv6]:PORT`. Returns the address and the TLS server name.
pub fn resolve(spec: &str) -> Result<(SocketAddr, String), String> {
    let spec = spec.trim();
    if let Ok(addr) = spec.parse::<SocketAddr>() {
        return Ok((addr, addr.ip().to_string()));
    }
    if let Ok(ip) = spec.trim_matches(['[', ']']).parse::<std::net::IpAddr>() {
        return Ok((SocketAddr::new(ip, DEFAULT_PORT), ip.to_string()));
    }
    let (host, with_port) = match spec.rsplit_once(':') {
        Some((h, p)) if p.parse::<u16>().is_ok() => (h, spec.to_owned()),
        _ => (spec, format!("{spec}:{DEFAULT_PORT}")),
    };
    if host.is_empty() {
        return Err(format!("{spec:?}: no host"));
    }
    let addr = with_port
        .to_socket_addrs()
        .map_err(|e| format!("{spec}: {e}"))?
        .next()
        .ok_or_else(|| format!("{spec}: no address"))?;
    Ok((addr, host.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_addresses_with_and_without_port() {
        let (a, name) = resolve("127.0.0.1:5000").unwrap();
        assert_eq!((a.port(), name.as_str()), (5000, "127.0.0.1"));
        let (a, _) = resolve("127.0.0.1").unwrap();
        assert_eq!(a.port(), DEFAULT_PORT);
        let (a, name) = resolve("[::1]:6000").unwrap();
        assert!(a.is_ipv6() && a.port() == 6000 && name == "::1");
        let (a, _) = resolve("::1").unwrap();
        assert_eq!(a.port(), DEFAULT_PORT);
        let (a, name) = resolve("localhost:7000").unwrap();
        assert!(a.ip().is_loopback());
        assert_eq!((a.port(), name.as_str()), (7000, "localhost"));
        let (a, _) = resolve("localhost").unwrap();
        assert_eq!(a.port(), DEFAULT_PORT);
        assert!(resolve(":7000").is_err());
    }
}
