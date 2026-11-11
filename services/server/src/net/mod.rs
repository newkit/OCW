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
//! **Security (dev mode).** TLS 1.3 is always on. Until real certificates
//! and OIDC exist, the server uses a self-signed certificate ([`tls`]) and
//! prints its SHA-256 fingerprint; clients pin it (`--server-cert`), or, for
//! a server on the same machine only, skip the check (`--insecure-dev`).

pub mod client;
pub mod framing;
pub mod server;
pub mod tls;

pub use client::{ClientOptions, LinkStatus, NetClient};
pub use server::NetServer;
pub use tls::{ServerIdentity, ServerTrust};

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

/// The field number of the `oneof` member an encoded protocol message
/// carries (its first protobuf key), without decoding the message.
pub fn oneof_field(bytes: &[u8]) -> Option<u32> {
    let mut key: u64 = 0;
    for (i, b) in bytes.iter().take(10).enumerate() {
        key |= u64::from(b & 0x7f) << (7 * i);
        if b & 0x80 == 0 {
            // Wire type 2 (length-delimited): a message field.
            return (key & 7 == 2).then_some((key >> 3) as u32);
        }
    }
    None
}

/// `ServerMessage.snapshot`: superseded by the next one, sent unreliably.
const SERVER_SNAPSHOT_FIELD: u32 = 2;
/// `ClientMessage.sample`: superseded by the next one, sent unreliably.
const CLIENT_SAMPLE_FIELD: u32 = 2;

/// Whether an encoded `ServerMessage` may go as an unreliable datagram.
pub fn server_unreliable(bytes: &[u8]) -> bool {
    oneof_field(bytes) == Some(SERVER_SNAPSHOT_FIELD)
}

/// Whether an encoded `ClientMessage` may go as an unreliable datagram.
pub fn client_unreliable(bytes: &[u8]) -> bool {
    oneof_field(bytes) == Some(CLIENT_SAMPLE_FIELD)
}

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
    use ocw_protocol::v1::{self, client_message::Msg as C, server_message::Msg as S};
    use ocw_protocol::{client_message, encode, server_message};

    #[test]
    fn only_snapshots_and_samples_are_unreliable() {
        let snapshot = encode(&server_message(S::Snapshot(v1::Snapshot::default())));
        // An empty snapshot still carries its (empty) field.
        assert!(server_unreliable(&snapshot));
        let reliable = [
            S::Welcome(v1::Welcome::default()),
            S::Reject(v1::Reject::default()),
            S::DevResult(v1::DevCommandResult::default()),
            S::Path(v1::PathUpdate::default()),
        ];
        for m in reliable {
            assert!(!server_unreliable(&encode(&server_message(m))));
        }
        let sample = encode(&client_message(C::Sample(v1::SensorSample {
            seq: 7,
            power_w: Some(250.0),
            ..Default::default()
        })));
        assert!(client_unreliable(&sample));
        let reliable = [
            C::Hello(v1::Hello::default()),
            C::Steer(v1::SteerIntent::default()),
            C::Turn(v1::TurnIntent::default()),
            C::UTurn(v1::UTurn::default()),
            C::RouteRejoin(v1::RouteRejoin::default()),
            C::Dev(v1::DevCommand::default()),
        ];
        for m in reliable {
            assert!(!client_unreliable(&encode(&client_message(m))));
        }
    }

    #[test]
    fn oneof_field_reads_long_keys_and_refuses_garbage() {
        // Field 300, wire type 2: key 2402 = 0xe2 0x12.
        assert_eq!(oneof_field(&[0xe2, 0x12, 0x00]), Some(300));
        assert_eq!(oneof_field(&[]), None);
        // Wire type 0 (varint), not a message.
        assert_eq!(oneof_field(&[0x10, 0x01]), None);
        // Unterminated varint.
        assert_eq!(oneof_field(&[0xff; 12]), None);
    }

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
