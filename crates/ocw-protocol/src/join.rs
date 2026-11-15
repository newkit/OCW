//! Join links (spec §6.2 *Join links*, §3.2.1 *Ride → Join*): how a rider
//! names a game server to join, in one line to share.
//!
//! The canonical form is
//! `ocw://HOST[:PORT]?cert=SHA256HEX[&world=ID[@VERSION]][&route=NAME][&name=TEXT]`
//! (port default 47800, the native QUIC endpoint). [`JoinLink::parse`]
//! also takes
//!
//! - `https://HOST[:PORT][/ocw/1]?cert-hash=HEX…` – a WebTransport server,
//!   as the browser joins it (port default 47801; `cert=` works too),
//! - a bare `HOST[:PORT]` (IPv6 as `[::1]:47800`), the fingerprint entered
//!   separately ([`JoinLink::with_fingerprint`]),
//! - a pasted copy of the server's printed line
//!   `ocw-desktop --server HOST:PORT --server-cert HEX` (or
//!   `--insecure-dev`), since that is what people copy.
//!
//! Pure and wasm-safe: no name resolution here (the apps resolve the
//! host when they connect). A link without a fingerprint may only name
//! this machine ([`JoinLink::check_trust`]), like `--insecure-dev`.

use std::fmt::Write as _;

/// The native QUIC port (`ocw-server --listen`, the kiosk).
pub const DEFAULT_PORT: u16 = 47800;
/// The WebTransport port (`ocw-server --webtransport`).
pub const DEFAULT_WEB_PORT: u16 = 47801;

/// How the server is reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkTransport {
    /// Native QUIC (`ocw://`): the desktop app.
    Quic,
    /// WebTransport (`https://`): browsers, and the desktop too.
    WebTransport,
}

/// A parsed join link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinLink {
    pub transport: LinkTransport,
    /// Host name or IP (IPv6 without brackets).
    pub host: String,
    pub port: u16,
    /// The SHA-256 fingerprint of the server's certificate (the QUIC one,
    /// or the WebTransport `cert-hash`), 64 lowercase hex digits.
    pub cert: Option<String>,
    /// The world package the server rides (`id`, `id@version`, or a
    /// manifest URL to fetch it from).
    pub world: Option<String>,
    pub route: Option<String>,
    /// A name for the ride (shown in the recent list).
    pub name: Option<String>,
}

impl JoinLink {
    /// A link to `host:port` (native QUIC).
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            transport: LinkTransport::Quic,
            host: host.into().trim_matches(['[', ']']).to_owned(),
            port,
            cert: None,
            world: None,
            route: None,
            name: None,
        }
    }

    /// Parses any of the accepted forms (see the module docs).
    pub fn parse(input: &str) -> Result<Self, String> {
        let s = input.trim().trim_matches(['"', '\'']).trim();
        if s.is_empty() {
            return Err("enter a join link, e.g. ocw://host:47800?cert=…".into());
        }
        if s.contains("--server") {
            return parse_command_line(s);
        }
        let lower = s.to_ascii_lowercase();
        if let Some(rest) = strip_prefix_ci(s, &lower, "ocw://") {
            return parse_url(rest, LinkTransport::Quic);
        }
        if let Some(rest) = strip_prefix_ci(s, &lower, "https://") {
            return parse_url(rest, LinkTransport::WebTransport);
        }
        if lower.starts_with("http://") {
            return Err(
                "http:// links do not work: game servers use ocw:// (or https:// for WebTransport)"
                    .into(),
            );
        }
        if let Some(i) = s.find("://") {
            return Err(format!(
                "unknown link type {:?}: use ocw://HOST:PORT?cert=… (or https:// for WebTransport)",
                &s[..i]
            ));
        }
        if s.contains(char::is_whitespace) {
            return Err(format!(
                "{s:?} is not a join link: use ocw://HOST:PORT?cert=… or HOST:PORT"
            ));
        }
        parse_url(s, LinkTransport::Quic)
    }

    /// The link with the fingerprint entered separately (a bare
    /// `HOST:PORT`); an empty `hex` keeps it as it is.
    pub fn with_fingerprint(mut self, hex: &str) -> Result<Self, String> {
        if !hex.trim().is_empty() {
            self.cert = Some(parse_fingerprint(hex)?);
        }
        Ok(self)
    }

    /// The host as in an address: IPv6 in brackets.
    pub fn host_for_addr(&self) -> String {
        if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        }
    }

    /// `HOST:PORT` (what `--server` and `ocw_server::net::resolve` take).
    pub fn server_spec(&self) -> String {
        format!("{}:{}", self.host_for_addr(), self.port)
    }

    /// The WebTransport URL of an `https://` link.
    pub fn web_url(&self) -> String {
        format!("https://{}:{}/ocw/1", self.host_for_addr(), self.port)
    }

    /// The host names this machine: `localhost`, 127.x.x.x, `::1`.
    pub fn is_loopback(&self) -> bool {
        let h = self.host.to_ascii_lowercase();
        h == "localhost"
            || h == "::1"
            || h.strip_prefix("127.").is_some_and(|r| {
                r.split('.').count() == 3 && r.split('.').all(|p| p.parse::<u8>().is_ok())
            })
    }

    /// A link without a fingerprint is only accepted for this machine
    /// (like `--insecure-dev`); the error says why.
    pub fn check_trust(&self) -> Result<(), String> {
        if self.cert.is_none() && !self.is_loopback() {
            return Err(format!(
                "{} needs the server's certificate fingerprint (64 hex digits, `cert=` in the link): without it anyone on the way could pretend to be the server. Ask its host for the full link",
                self.server_spec()
            ));
        }
        Ok(())
    }

    /// The canonical link (`ocw://…` or `https://…`).
    pub fn to_link(&self) -> String {
        let mut out = match self.transport {
            LinkTransport::Quic => format!("ocw://{}", self.host_for_addr()),
            LinkTransport::WebTransport => format!("https://{}", self.host_for_addr()),
        };
        let default = match self.transport {
            LinkTransport::Quic => DEFAULT_PORT,
            LinkTransport::WebTransport => DEFAULT_WEB_PORT,
        };
        if self.port != default || self.transport == LinkTransport::WebTransport {
            let _ = write!(out, ":{}", self.port);
        }
        let mut sep = '?';
        let mut add = |key: &str, value: &str| {
            let _ = write!(out, "{sep}{key}={}", encode(value));
            sep = '&';
        };
        if let Some(c) = &self.cert {
            add(
                match self.transport {
                    LinkTransport::Quic => "cert",
                    LinkTransport::WebTransport => "cert-hash",
                },
                c,
            );
        }
        if let Some(w) = &self.world {
            add("world", w);
        }
        if let Some(r) = &self.route {
            add("route", r);
        }
        if let Some(n) = &self.name {
            add("name", n);
        }
        out
    }

    /// A short label: `host:port` and the world / route, for lists.
    pub fn label(&self) -> String {
        let place = match (&self.world, &self.route) {
            (Some(w), Some(r)) => format!(" – {} / {r}", short_world(w)),
            (Some(w), None) => format!(" – {}", short_world(w)),
            (None, Some(r)) => format!(" – {r}"),
            (None, None) => String::new(),
        };
        match &self.name {
            Some(n) => format!("{n} ({}){place}", self.server_spec()),
            None => format!("{}{place}", self.server_spec()),
        }
    }

    /// The world's package id and version, if the link names one that is
    /// not a URL.
    pub fn world_id(&self) -> Option<(&str, Option<u64>)> {
        let w = self.world.as_deref()?;
        if w.contains("://") {
            return None;
        }
        Some(match w.rsplit_once('@') {
            Some((id, v)) if v.parse::<u64>().is_ok() => (id, v.parse().ok()),
            _ => (w, None),
        })
    }

    /// The manifest URL to fetch the world from, if the link has one.
    pub fn world_url(&self) -> Option<&str> {
        self.world
            .as_deref()
            .filter(|w| w.starts_with("https://") || w.starts_with("http://"))
    }
}

/// `europe/liechtenstein@3` → `liechtenstein`, a URL → its last path part.
fn short_world(w: &str) -> &str {
    let w = w.split('@').next().unwrap_or(w);
    let w = w.trim_end_matches("/manifest.json").trim_end_matches('/');
    w.rsplit('/').next().unwrap_or(w)
}

fn strip_prefix_ci<'a>(s: &'a str, lower: &str, prefix: &str) -> Option<&'a str> {
    lower.starts_with(prefix).then(|| &s[prefix.len()..])
}

/// Parses `HOST[:PORT][/path][?query]` of a link.
fn parse_url(rest: &str, transport: LinkTransport) -> Result<JoinLink, String> {
    let (authority, query) = match rest.split_once('?') {
        Some((a, q)) => (a, q),
        None => (rest, ""),
    };
    let (authority, path) = match authority.find('/') {
        Some(i) => (&authority[..i], &authority[i..]),
        None => (authority, ""),
    };
    if transport == LinkTransport::Quic && !matches!(path, "" | "/") {
        return Err(format!(
            "{path:?}: an ocw:// link has no path, only ?cert=…"
        ));
    }
    if authority.contains('@') {
        return Err("a join link has no user name (…@host)".into());
    }
    let default = match transport {
        LinkTransport::Quic => DEFAULT_PORT,
        LinkTransport::WebTransport => DEFAULT_WEB_PORT,
    };
    let (host, port) = split_host_port(authority, default)?;
    let mut link = JoinLink::new(host, port);
    link.transport = transport;
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let v = decode(v)?;
        match k.to_ascii_lowercase().as_str() {
            "cert" | "cert-hash" | "cert_hash" | "server-cert" => {
                link.cert = Some(parse_fingerprint(&v)?);
            }
            "world" if !v.is_empty() => link.world = Some(v),
            "route" if !v.is_empty() => link.route = Some(v),
            "name" if !v.is_empty() => link.name = Some(v),
            // Page options of the browser (server=…) are not join links.
            "server" => {
                return Err(
                    "this is a page link (server=…); paste the server's join link instead".into(),
                );
            }
            // Unknown options are left out (newer servers may add some).
            _ => {}
        }
    }
    Ok(link)
}

/// `HOST`, `HOST:PORT`, `[V6]`, `[V6]:PORT`, a bare IPv6 (default port).
fn split_host_port(s: &str, default: u16) -> Result<(String, u16), String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("the link has no host".into());
    }
    let port = |p: &str| -> Result<u16, String> {
        match p.parse::<u16>() {
            Ok(0) | Err(_) => Err(format!("{p:?} is not a port (1–65535)")),
            Ok(n) => Ok(n),
        }
    };
    if let Some(rest) = s.strip_prefix('[') {
        let (h, after) = rest
            .split_once(']')
            .ok_or_else(|| format!("{s:?}: missing ] after the IPv6 address"))?;
        check_host(h, true)?;
        return match after {
            "" => Ok((h.to_owned(), default)),
            a => match a.strip_prefix(':') {
                Some(p) => Ok((h.to_owned(), port(p)?)),
                None => Err(format!("{s:?}: expected :PORT after ]")),
            },
        };
    }
    if s.matches(':').count() > 1 {
        // A bare IPv6 address: no port (write [::1]:47800 for one).
        check_host(s, true)?;
        return Ok((s.to_owned(), default));
    }
    match s.split_once(':') {
        Some((h, p)) => {
            check_host(h, false)?;
            Ok((h.to_owned(), port(p)?))
        }
        None => {
            check_host(s, false)?;
            Ok((s.to_owned(), default))
        }
    }
}

fn check_host(h: &str, v6: bool) -> Result<(), String> {
    if h.is_empty() {
        return Err("the link has no host".into());
    }
    let ok = if v6 {
        h.chars()
            .all(|c| c.is_ascii_hexdigit() || c == ':' || c == '.')
    } else {
        h.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_'))
    };
    if ok {
        Ok(())
    } else {
        Err(format!("{h:?} is not a host name or address"))
    }
}

/// Parses a pasted `ocw-desktop --server HOST:PORT --server-cert HEX`.
fn parse_command_line(s: &str) -> Result<JoinLink, String> {
    let words: Vec<&str> = s.split_whitespace().collect();
    let value = |flag: &str| -> Option<&str> {
        words.iter().enumerate().find_map(|(i, w)| {
            if *w == flag {
                words.get(i + 1).copied()
            } else {
                w.strip_prefix(flag).and_then(|r| r.strip_prefix('='))
            }
        })
    };
    let server = value("--server").ok_or("the line has no server after --server")?;
    let mut link = parse_url(server, LinkTransport::Quic)?;
    if let Some(c) = value("--server-cert") {
        link.cert = Some(parse_fingerprint(c)?);
    }
    if let Some(w) = value("--world") {
        link.world = Some(w.to_owned());
    }
    if let Some(r) = value("--route") {
        link.route = Some(r.to_owned());
    }
    Ok(link)
}

/// 64 hex digits (`:` and spaces between them allowed), lowercase.
pub fn parse_fingerprint(s: &str) -> Result<String, String> {
    let hex: String = s
        .trim()
        .chars()
        .filter(|c| !matches!(c, ':' | ' '))
        .collect();
    if let Some(bad) = hex.chars().find(|c| !c.is_ascii_hexdigit()) {
        return Err(format!(
            "the certificate fingerprint has only hex digits (0-9, a-f); got {bad:?}"
        ));
    }
    if hex.len() != 64 {
        return Err(format!(
            "the certificate fingerprint has 64 hex digits; got {}",
            hex.len()
        ));
    }
    Ok(hex.to_ascii_lowercase())
}

/// Percent-encodes what a query value must not contain.
fn encode(v: &str) -> String {
    let mut out = String::new();
    for b in v.bytes() {
        match b {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'.'
            | b'_'
            | b'~'
            | b'/'
            | b'@'
            | b':' => {
                out.push(b as char);
            }
            _ => {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}

/// Decodes `%XX` (and `+` as a space) in a query value.
fn decode(v: &str) -> Result<String, String> {
    let bytes = v.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let hex = v
                    .get(i + 1..i + 3)
                    .and_then(|h| u8::from_str_radix(h, 16).ok())
                    .ok_or_else(|| format!("{v:?}: a broken %-escape"))?;
                out.push(hex);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).map_err(|_| format!("{v:?}: not UTF-8"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FP: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn the_canonical_link_round_trips() {
        let l = JoinLink::parse(&format!(
            "ocw://ride.example.org:48000?cert={FP}&world=europe/liechtenstein@3&route=Vaduz%E2%80%93Malbun&name=Tuesday+Ride"
        ))
        .unwrap();
        assert_eq!(l.transport, LinkTransport::Quic);
        assert_eq!((l.host.as_str(), l.port), ("ride.example.org", 48000));
        assert_eq!(l.cert.as_deref(), Some(FP));
        assert_eq!(l.world_id(), Some(("europe/liechtenstein", Some(3))));
        assert_eq!(l.route.as_deref(), Some("Vaduz–Malbun"));
        assert_eq!(l.name.as_deref(), Some("Tuesday Ride"));
        assert_eq!(JoinLink::parse(&l.to_link()).unwrap(), l);
        assert_eq!(l.server_spec(), "ride.example.org:48000");
    }

    #[test]
    fn the_port_defaults_and_is_left_out_of_the_canonical_form() {
        let l = JoinLink::parse(&format!("OCW://10.0.0.7?cert={}", FP.to_uppercase())).unwrap();
        assert_eq!(l.port, DEFAULT_PORT);
        assert_eq!(l.cert.as_deref(), Some(FP), "lowercase");
        assert_eq!(l.to_link(), format!("ocw://10.0.0.7?cert={FP}"));
        let l = JoinLink::parse(&format!("https://10.0.0.7?cert-hash={FP}")).unwrap();
        assert_eq!(
            (l.transport, l.port),
            (LinkTransport::WebTransport, DEFAULT_WEB_PORT)
        );
        assert_eq!(l.web_url(), "https://10.0.0.7:47801/ocw/1");
        assert!(l.to_link().starts_with("https://10.0.0.7:47801?cert-hash="));
    }

    #[test]
    fn ipv6_hosts_with_and_without_port() {
        let l = JoinLink::parse(&format!("ocw://[2001:db8::5]:47900?cert={FP}")).unwrap();
        assert_eq!((l.host.as_str(), l.port), ("2001:db8::5", 47900));
        assert_eq!(l.server_spec(), "[2001:db8::5]:47900");
        assert_eq!(JoinLink::parse(&l.to_link()).unwrap(), l);
        let l = JoinLink::parse("[::1]").unwrap();
        assert_eq!((l.host.as_str(), l.port), ("::1", DEFAULT_PORT));
        assert!(l.is_loopback());
        let l = JoinLink::parse("::1").unwrap();
        assert_eq!(l.port, DEFAULT_PORT);
        let l = JoinLink::parse(&format!("https://[::1]:47801/ocw/1?cert={FP}")).unwrap();
        assert_eq!(l.web_url(), "https://[::1]:47801/ocw/1");
    }

    #[test]
    fn bare_host_and_a_separate_fingerprint() {
        let l = JoinLink::parse("kiosk.local:47800").unwrap();
        assert_eq!(l.cert, None);
        assert!(l.check_trust().unwrap_err().contains("fingerprint"));
        let colons: Vec<&str> = (0..32).map(|i| &FP[2 * i..2 * i + 2]).collect();
        let l = l.with_fingerprint(&colons.join(":")).unwrap();
        assert_eq!(l.cert.as_deref(), Some(FP));
        assert!(l.check_trust().is_ok());
        assert!(
            JoinLink::parse("h")
                .unwrap()
                .with_fingerprint("")
                .unwrap()
                .cert
                .is_none()
        );
    }

    #[test]
    fn the_servers_printed_line_is_accepted() {
        let l = JoinLink::parse(&format!(
            "  ocw-desktop --server 192.168.1.20:47800 --server-cert {FP}\n"
        ))
        .unwrap();
        assert_eq!(
            (l.host.as_str(), l.port, l.cert.as_deref()),
            ("192.168.1.20", 47800, Some(FP))
        );
        let l = JoinLink::parse("ocw-desktop --server 127.0.0.1 --insecure-dev").unwrap();
        assert!(l.cert.is_none() && l.is_loopback() && l.check_trust().is_ok());
        let l = JoinLink::parse(&format!(
            "ocw-desktop --server=[::1]:5000 --server-cert={FP} --world europe/x --route Loop"
        ))
        .unwrap();
        assert_eq!(
            (l.port, l.world.as_deref(), l.route.as_deref()),
            (5000, Some("europe/x"), Some("Loop"))
        );
        assert!(
            JoinLink::parse("ocw-desktop --server")
                .unwrap_err()
                .contains("no server")
        );
    }

    #[test]
    fn errors_are_plain() {
        let e = JoinLink::parse("ocw://h?cert=abc123abc123").unwrap_err();
        assert_eq!(e, "the certificate fingerprint has 64 hex digits; got 12");
        assert!(
            JoinLink::parse(&format!("ocw://h?cert={}", "z".repeat(64)))
                .unwrap_err()
                .contains("only hex digits")
        );
        assert!(
            JoinLink::parse("")
                .unwrap_err()
                .contains("enter a join link")
        );
        assert!(
            JoinLink::parse("ocw://:47800")
                .unwrap_err()
                .contains("no host")
        );
        assert!(
            JoinLink::parse("ocw://h:0")
                .unwrap_err()
                .contains("not a port")
        );
        assert!(
            JoinLink::parse("ocw://h:99999")
                .unwrap_err()
                .contains("not a port")
        );
        assert!(
            JoinLink::parse("ocw://h/x")
                .unwrap_err()
                .contains("no path")
        );
        assert!(
            JoinLink::parse("ocw://[::1")
                .unwrap_err()
                .contains("missing ]")
        );
        assert!(JoinLink::parse("http://h").unwrap_err().contains("http://"));
        assert!(
            JoinLink::parse("ftp://h")
                .unwrap_err()
                .contains("unknown link type")
        );
        assert!(
            JoinLink::parse("two words")
                .unwrap_err()
                .contains("not a join link")
        );
        assert!(JoinLink::parse("ho$t").unwrap_err().contains("not a host"));
        assert!(
            JoinLink::parse("ocw://h?world=%zz")
                .unwrap_err()
                .contains("%-escape")
        );
        assert!(
            JoinLink::parse("https://h?server=x")
                .unwrap_err()
                .contains("page link")
        );
    }

    #[test]
    fn only_this_machine_goes_without_a_fingerprint() {
        for h in [
            "localhost",
            "127.0.0.1",
            "127.1.2.3",
            "[::1]",
            "LOCALHOST:5000",
        ] {
            let l = JoinLink::parse(h).unwrap();
            assert!(l.check_trust().is_ok(), "{h}");
        }
        for h in [
            "127.example.org",
            "10.0.0.1",
            "[2001:db8::1]",
            "localhost.example.org",
        ] {
            let l = JoinLink::parse(h).unwrap();
            assert!(l.check_trust().is_err(), "{h}");
        }
    }

    #[test]
    fn labels_and_world_urls() {
        let mut l = JoinLink::new("10.0.0.2", 47800);
        assert_eq!(l.label(), "10.0.0.2:47800");
        l.world = Some("europe/liechtenstein@3".into());
        l.route = Some("Malbun".into());
        assert_eq!(l.label(), "10.0.0.2:47800 – liechtenstein / Malbun");
        l.world = Some("https://worlds.example.org/tenerife/manifest.json".into());
        assert_eq!(
            l.world_url(),
            Some("https://worlds.example.org/tenerife/manifest.json")
        );
        assert_eq!(l.world_id(), None);
        assert_eq!(l.label(), "10.0.0.2:47800 – tenerife / Malbun");
        let back = JoinLink::parse(&l.to_link()).unwrap();
        assert_eq!(back, l);
    }
}
