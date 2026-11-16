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

/// This machine's addresses others reach it on (no loopback): the
/// source address the system would use towards the internet, for IPv4
/// and IPv6. Nothing is sent (a UDP socket is only connected).
pub fn lan_addresses() -> Vec<std::net::IpAddr> {
    let probe = |bind: &str, to: &str| -> Option<std::net::IpAddr> {
        let s = std::net::UdpSocket::bind(bind).ok()?;
        s.connect(to).ok()?;
        let ip = s.local_addr().ok()?.ip();
        (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
    };
    // Documentation addresses (RFC 5737, RFC 3849): routed like any other.
    [
        probe("0.0.0.0:0", "192.0.2.1:9"),
        probe("[::]:0", "[2001:db8::1]:9"),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// The join links of a server bound to `local` (spec §6.2 *Join links*),
/// for the console and the kiosk: one per address others reach (the LAN
/// addresses for an unspecified bind), with the world and route.
pub fn join_links(
    local: SocketAddr,
    fingerprint: &str,
    world: Option<&str>,
    route: Option<&str>,
) -> Vec<String> {
    join_links_with(local, fingerprint, world, route, None)
}

/// [`join_links`] with the hosting's public address (the router's, spec
/// §7.1.7 Q4 / Q5) first: the link for riders from the internet.
pub fn join_links_with(
    local: SocketAddr,
    fingerprint: &str,
    world: Option<&str>,
    route: Option<&str>,
    public: Option<std::net::IpAddr>,
) -> Vec<String> {
    let mut hosts: Vec<String> = public.map(|ip| ip.to_string()).into_iter().collect();
    if local.ip().is_unspecified() {
        let lan = lan_addresses();
        if lan.is_empty() && hosts.is_empty() {
            hosts.push("HOST".into());
        }
        hosts.extend(lan.iter().map(ToString::to_string));
    } else {
        hosts.push(local.ip().to_string());
    }
    hosts.dedup();
    hosts
        .into_iter()
        .map(|h| {
            let mut l = ocw_protocol::join::JoinLink::new(h, local.port());
            l.cert = Some(fingerprint.to_owned());
            l.world = world.map(str::to_owned);
            l.route = route.map(str::to_owned);
            l.to_link()
        })
        .collect()
}

/// The printed block: "Join link (…):" and the links, indented; the
/// public address's link first when there is one.
pub fn join_lines(
    local: SocketAddr,
    fingerprint: &str,
    world: Option<&str>,
    route: Option<&str>,
    public: Option<std::net::IpAddr>,
) -> String {
    let links = join_links_with(local, fingerprint, world, route, public);
    let mut out = format!(
        "Join link{} (in the app: Ride -> Join; or ocw-desktop --join LINK):",
        if links.len() > 1 { "s" } else { "" }
    );
    for l in links {
        out.push_str("\n  ");
        out.push_str(&l);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_server_prints_its_join_link() {
        let fp = "cd".repeat(32);
        let local: SocketAddr = "127.0.0.1:47800".parse().unwrap();
        let text = join_lines(
            local,
            &fp,
            Some("europe/liechtenstein@3"),
            Some("Vaduz Loop"),
            None,
        );
        assert_eq!(
            text,
            format!(
                "Join link (in the app: Ride -> Join; or ocw-desktop --join LINK):\n  ocw://127.0.0.1?cert={fp}&world=europe/liechtenstein@3&route=Vaduz%20Loop"
            )
        );
        let link = text.lines().nth(1).unwrap().trim();
        let parsed = ocw_protocol::join::JoinLink::parse(link).unwrap();
        assert_eq!(parsed.route.as_deref(), Some("Vaduz Loop"));
        let v6: SocketAddr = "[::1]:47900".parse().unwrap();
        assert_eq!(
            join_links(v6, &fp, None, Some("hills")),
            [format!("ocw://[::1]:47900?cert={fp}&route=hills")]
        );
        // An unspecified bind: an address of this machine, or HOST.
        let any: SocketAddr = "0.0.0.0:47800".parse().unwrap();
        for l in join_links(any, &fp, None, None) {
            assert!(l.starts_with("ocw://") && !l.contains("0.0.0.0"), "{l}");
        }
        // The router's public address first (spec §7.1.7 Q4, Q5).
        let public = Some("81.2.69.160".parse().unwrap());
        let links = join_links_with(any, &fp, None, Some("hills"), public);
        assert_eq!(links[0], format!("ocw://81.2.69.160?cert={fp}&route=hills"));
        let text = join_lines(local, &fp, None, None, public);
        assert!(text.starts_with("Join links"), "{text}");
        assert_eq!(text.lines().count(), 3);
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
