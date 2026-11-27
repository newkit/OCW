//! How browsers from the internet find the server (spec 11.5 *Join info*):
//! a small HTTP endpoint that answers `GET /join.json` with the server's
//! current WebTransport URL and certificate hashes (the current one and,
//! a day before a renewal, the next one), its voice URL and its world – so
//! a page (the public preview on GitHub Pages) learns the certificate at
//! runtime instead of having it baked in when it was built. Everything
//! else on the port is passed on unchanged to an upstream (the bundled
//! voice server's signalling port), so one TLS port of a hosting's proxy
//! (Fly.io: `https://APP.fly.dev`, `wss://APP.fly.dev`) serves both.
//!
//! The answer holds no secret: it is what a join link says. It is allowed
//! for every origin (`Access-Control-Allow-Origin: *`, no credentials) and
//! never cached (the hashes change at renewal and restart).
//!
//! **The status page (spec 11.5 *Status page*):** with a [`StatusBoard`]
//! the endpoint also answers `GET /` from a browser (no `Upgrade` header:
//! a WebSocket to `/` still goes to the voice server) with a small HTML
//! page, `GET /stats.json` with the public statistics
//! ([`crate::public_stats`]: counts and the names riders ride under, no
//! addresses or positions) and the page's files under [`ASSET_PREFIX`]. LiveKit's own paths (`/rtc…`, `/twirp/…`) and
//! everything else are passed on as before.
//!
//! Passing on: the request head is read (up to [`MAX_HEAD`] bytes), then
//! the connection is joined to a new one to the upstream. A request that
//! is not a WebSocket upgrade goes with `Connection: close`, so a proxy
//! that keeps connections open never sends a later `/join.json` there.

use std::net::SocketAddr;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// The path of the join info.
pub const PATH: &str = "/join.json";

/// The public statistics (spec 11.5 *Status page*).
pub const STATS_PATH: &str = "/stats.json";

/// The status page's own files (a prefix LiveKit does not use).
pub const ASSET_PREFIX: &str = "/_ocw/";

/// The status page's template (`{{…}}` filled in per request).
const PAGE: &str = include_str!("status_page.html");

/// The logo (the page's icon).
const ICON_SVG: &[u8] = include_bytes!("../../../../artwork/logos/ocw.svg");

/// The status page's policy: only its own inline style and script, and
/// requests to this origin.
const PAGE_CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; img-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

/// The longest request head read before answering or passing on.
pub const MAX_HEAD: usize = 16 * 1024;

/// How long a client may take to send its request head.
const HEAD_TIMEOUT: Duration = Duration::from_secs(10);

/// What a page needs to join (the body of `/join.json`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct JoinInfo {
    /// The WebTransport URL riders from the internet use
    /// (`https://IPV4:47801`).
    pub webtransport: String,
    /// The SHA-256 hashes (hex) the page passes as
    /// `serverCertificateHashes`: the current certificate first, then the
    /// next one once it is announced (a day before the renewal).
    pub cert_hashes: Vec<String>,
    /// When the current certificate ends (Unix s).
    pub not_after_unix_s: i64,
    /// The voice server's URL for riders from the internet (`wss://…`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voice: Option<String>,
    /// The world package the session rides (its manifest id), with its
    /// version and the SHA-256 of the package file: the page must load the
    /// same package.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub world: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub world_version: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub world_sha256: Option<String>,
    /// The route ridden (none: a free ride); a test route's name without a
    /// world.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route: Option<String>,
    /// The page options for joining (`server=…&cert-hash=…`).
    pub query: String,
    /// The desktop app's join link over native QUIC
    /// (`ocw://IPV4?cert=FINGERPRINT&world=ID@VERSION`; the `--listen`
    /// certificate, whose fingerprint stays the same across restarts):
    /// what a kiosk joining this server uses (`ocw-kiosk.sh`,
    /// `OCW_KIOSK_SERVER=remote`, spec 11.5 *Join info*). None without a
    /// public host.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub join: Option<String>,
}

impl JoinInfo {
    /// Fills [`Self::query`] from the URL and hashes.
    pub fn with_query(mut self) -> Self {
        self.query =
            super::webtransport::page_query(&self.webtransport, &self.cert_hashes.join(","));
        self
    }
}

/// The current join info, shared with the endpoint's thread.
#[derive(Clone, Default)]
pub struct JoinBoard(Arc<RwLock<String>>);

impl JoinBoard {
    /// Publishes `info` (later requests get it).
    pub fn set(&self, info: &JoinInfo) {
        let text = serde_json::to_string_pretty(info).unwrap_or_default() + "\n";
        *self.0.write().unwrap_or_else(|e| e.into_inner()) = text;
    }

    /// The published text (empty before the first [`Self::set`]).
    pub fn text(&self) -> String {
        self.0.read().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

/// What the status page names (set at the start and at a world switch).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PageInfo {
    /// The desktop app's join link (`ocw://IPV4?cert=…&world=…`).
    pub desktop_link: String,
    /// The region's name (`Liechtenstein`).
    pub world_name: String,
    pub world_version: Option<u64>,
    /// The route ridden (none: a free ride).
    pub route: Option<String>,
    /// The server's version.
    pub version: String,
}

#[derive(Default)]
struct Status {
    page: PageInfo,
    stats: String,
}

/// The status page's data and the public statistics, shared with the
/// endpoint's thread (spec 11.5 *Status page*).
#[derive(Clone, Default)]
pub struct StatusBoard(Arc<RwLock<Status>>);

impl StatusBoard {
    /// What the page names.
    pub fn set_page(&self, page: PageInfo) {
        self.0.write().unwrap_or_else(|e| e.into_inner()).page = page;
    }

    /// Publishes the statistics (later `/stats.json` requests get them).
    /// `<`, `>` and `&` (only ever inside strings: riders' names) are
    /// sent as `\u` escapes, so no name reads as markup anywhere.
    pub fn set_stats(&self, report: &crate::public_stats::StatsReport) {
        let text = serde_json::to_string_pretty(report)
            .unwrap_or_default()
            .replace('<', "\\u003c")
            .replace('>', "\\u003e")
            .replace('&', "\\u0026")
            + "\n";
        self.0.write().unwrap_or_else(|e| e.into_inner()).stats = text;
    }

    /// The published statistics (empty before the first
    /// [`Self::set_stats`]).
    pub fn stats_text(&self) -> String {
        self.0
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .stats
            .clone()
    }

    /// The page, filled in.
    pub fn page_html(&self) -> String {
        let page = self
            .0
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .page
            .clone();
        let or = |s: &str, none: &str| html(if s.is_empty() { none } else { s });
        let route = match &page.route {
            Some(r) => format!("the route {}", html(r)),
            None => "free ride on every road of the world".to_owned(),
        };
        let version = page
            .world_version
            .map_or_else(|| "–".to_owned(), |v| v.to_string());
        PAGE.replace("{{LINK}}", &or(&page.desktop_link, "(not ready yet)"))
            .replace("{{WORLD_NAME}}", &or(&page.world_name, "a test route"))
            .replace("{{WORLD_VERSION}}", &version)
            .replace("{{ROUTE}}", &route)
            .replace("{{VERSION}}", &or(&page.version, "–"))
    }
}

/// `text` for HTML (text and attribute values).
fn html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// What to do with a request head.
#[derive(Debug, PartialEq, Eq)]
pub enum Route {
    /// Answer it here with these bytes (and close).
    Answer(Vec<u8>),
    /// Pass it on, with this head (maybe with `Connection: close`).
    Pass(Vec<u8>),
}

/// Decides what to do with a complete request head (`head` ends with the
/// blank line); `status`: the status page's data, when it is served;
/// `upstream`: whether there is one to pass on to.
pub fn route(
    head: &[u8],
    board: &JoinBoard,
    status: Option<&StatusBoard>,
    upstream: bool,
) -> Route {
    let text = String::from_utf8_lossy(head);
    let mut parts = text.lines().next().unwrap_or_default().split_whitespace();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let path = target.split(['?', '#']).next().unwrap_or("");
    if path == PATH {
        return Route::Answer(match method {
            "GET" | "HEAD" => {
                let body = board.text();
                if body.is_empty() {
                    response(
                        503,
                        "Service Unavailable",
                        "text/plain",
                        b"not ready\n",
                        method,
                    )
                } else {
                    response(200, "OK", "application/json", body.as_bytes(), method)
                }
            }
            "OPTIONS" => response(204, "No Content", "text/plain", b"", method),
            _ => response(
                405,
                "Method Not Allowed",
                "text/plain",
                b"GET only\n",
                method,
            ),
        });
    }
    if let Some(status) = status
        && let Some(answer) = status_route(method, path, &text, status)
    {
        return Route::Answer(answer);
    }
    if !upstream {
        return Route::Answer(response(
            404,
            "Not Found",
            "text/plain",
            b"not found\n",
            method,
        ));
    }
    Route::Pass(close_unless_upgrade(head))
}

/// The status page's answers (`None`: not one of its requests).
fn status_route(method: &str, path: &str, head: &str, status: &StatusBoard) -> Option<Vec<u8>> {
    let read = matches!(method, "GET" | "HEAD");
    let refuse = || {
        response(
            405,
            "Method Not Allowed",
            "text/plain",
            b"GET only\n",
            method,
        )
    };
    if path == STATS_PATH {
        return Some(match method {
            "GET" | "HEAD" => {
                let body = status.stats_text();
                if body.is_empty() {
                    response(
                        503,
                        "Service Unavailable",
                        "text/plain",
                        b"not ready\n",
                        method,
                    )
                } else {
                    response(200, "OK", "application/json", body.as_bytes(), method)
                }
            }
            "OPTIONS" => response(204, "No Content", "text/plain", b"", method),
            _ => refuse(),
        });
    }
    if let Some(file) = path.strip_prefix(ASSET_PREFIX) {
        return Some(match (read, file) {
            (true, "icon.svg") => response_with(
                (200, "OK"),
                "image/svg+xml",
                ICON_SVG,
                method,
                "Cache-Control: public, max-age=86400\r\n",
            ),
            (true, _) => response(404, "Not Found", "text/plain", b"not found\n", method),
            (false, _) => refuse(),
        });
    }
    // A browser opening the address: the page. A WebSocket to `/` (an
    // upgrade) and other methods go on to the voice server.
    if path == "/" && read && !has_upgrade(head) {
        return Some(response_with(
            (200, "OK"),
            "text/html",
            status.page_html().as_bytes(),
            method,
            &format!(
                "Cache-Control: no-store\r\nContent-Security-Policy: {PAGE_CSP}\r\nReferrer-Policy: no-referrer\r\n"
            ),
        ));
    }
    None
}

/// Whether a request head asks for an upgrade (a WebSocket).
fn has_upgrade(head: &str) -> bool {
    head.lines()
        .skip(1)
        .any(|l| l.to_ascii_lowercase().starts_with("upgrade:"))
}

/// An HTTP/1.1 response with the CORS headers, never cached, closing the
/// connection.
fn response(code: u16, reason: &str, kind: &str, body: &[u8], method: &str) -> Vec<u8> {
    response_with(
        (code, reason),
        kind,
        body,
        method,
        "Cache-Control: no-store\r\n",
    )
}

/// [`response`] with its own caching and more headers (`extra`, each
/// line ending with `\r\n`).
fn response_with(
    (code, reason): (u16, &str),
    kind: &str,
    body: &[u8],
    method: &str,
    extra: &str,
) -> Vec<u8> {
    let charset = if kind.starts_with("image/") {
        ""
    } else {
        "; charset=utf-8"
    };
    let mut out = format!(
        "HTTP/1.1 {code} {reason}\r\n\
         Content-Type: {kind}{charset}\r\n\
         Content-Length: {}\r\n\
         {extra}\
         Access-Control-Allow-Origin: *\r\n\
         Access-Control-Allow-Methods: GET, HEAD, OPTIONS\r\n\
         Access-Control-Max-Age: 600\r\n\
         X-Content-Type-Options: nosniff\r\n\
         Connection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    if method != "HEAD" {
        out.extend_from_slice(body);
    }
    out
}

/// The head with `Connection: close` unless it asks for an upgrade (a
/// WebSocket).
fn close_unless_upgrade(head: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(head);
    if has_upgrade(&text) {
        return head.to_vec();
    }
    let mut out = String::with_capacity(head.len() + 20);
    let mut lines = text.split("\r\n");
    out.push_str(lines.next().unwrap_or_default());
    out.push_str("\r\n");
    for l in lines.filter(|l| !l.is_empty()) {
        if !l.to_ascii_lowercase().starts_with("connection:") {
            out.push_str(l);
            out.push_str("\r\n");
        }
    }
    out.push_str("Connection: close\r\n\r\n");
    out.into_bytes()
}

/// Where a request head ends (after `\r\n\r\n`).
fn head_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
}

/// The running endpoint (it runs on its own thread until the process
/// ends).
pub struct JoinFront {
    local_addr: SocketAddr,
}

impl JoinFront {
    /// Listens on `addr` (TCP) on a thread of its own; `upstream`: where
    /// other requests go (the voice server's signalling port).
    pub fn spawn(
        addr: SocketAddr,
        upstream: Option<SocketAddr>,
        board: JoinBoard,
    ) -> Result<Self, String> {
        Self::spawn_with(addr, upstream, board, None)
    }

    /// [`Self::spawn`] with the status page (`status`; spec 11.5 *Status
    /// page*).
    pub fn spawn_with(
        addr: SocketAddr,
        upstream: Option<SocketAddr>,
        board: JoinBoard,
        status: Option<StatusBoard>,
    ) -> Result<Self, String> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        let listener = rt
            .block_on(TcpListener::bind(addr))
            .map_err(|e| format!("cannot listen on {addr} (join info, TCP): {e}"))?;
        let local_addr = listener.local_addr().map_err(|e| e.to_string())?;
        std::thread::Builder::new()
            .name("ocw-join-http".into())
            .spawn(move || {
                rt.block_on(async move {
                    loop {
                        let Ok((conn, _)) = listener.accept().await else {
                            tokio::time::sleep(Duration::from_millis(100)).await;
                            continue;
                        };
                        let board = board.clone();
                        let status = status.clone();
                        tokio::spawn(async move {
                            if let Err(e) = serve(conn, upstream, &board, status.as_ref()).await {
                                tracing::debug!(%e, "join info connection");
                            }
                        });
                    }
                });
            })
            .map_err(|e| e.to_string())?;
        Ok(Self { local_addr })
    }

    /// The bound address.
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }
}

async fn serve(
    mut conn: TcpStream,
    upstream: Option<SocketAddr>,
    board: &JoinBoard,
    status: Option<&StatusBoard>,
) -> std::io::Result<()> {
    let mut buf = Vec::with_capacity(2048);
    let end = tokio::time::timeout(HEAD_TIMEOUT, async {
        let mut chunk = [0u8; 2048];
        loop {
            if let Some(end) = head_end(&buf) {
                return Ok::<_, std::io::Error>(Some(end));
            }
            if buf.len() >= MAX_HEAD {
                return Ok(None);
            }
            let n = conn.read(&mut chunk).await?;
            if n == 0 {
                return Ok(None);
            }
            buf.extend_from_slice(&chunk[..n]);
        }
    })
    .await
    .unwrap_or(Ok(None))?;
    let Some(end) = end else {
        return Ok(());
    };
    match route(&buf[..end], board, status, upstream.is_some()) {
        Route::Answer(bytes) => {
            conn.write_all(&bytes).await?;
            conn.shutdown().await
        }
        Route::Pass(head) => {
            let Some(up) = upstream else { return Ok(()) };
            let mut next = match TcpStream::connect(up).await {
                Ok(s) => s,
                Err(_) => {
                    let bytes = response(
                        502,
                        "Bad Gateway",
                        "text/plain",
                        b"the voice server is not running\n",
                        "GET",
                    );
                    conn.write_all(&bytes).await?;
                    return conn.shutdown().await;
                }
            };
            next.write_all(&head).await?;
            next.write_all(&buf[end..]).await?;
            tokio::io::copy_bidirectional(&mut conn, &mut next)
                .await
                .map(|_| ())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> JoinInfo {
        JoinInfo {
            webtransport: "https://203.0.113.7:47801".into(),
            cert_hashes: vec!["aa".repeat(32), "bb".repeat(32)],
            not_after_unix_s: 1_800_000_000,
            voice: Some("wss://ride.example.org".into()),
            world: Some("liechtenstein".into()),
            world_version: Some(10),
            world_sha256: Some("cc".repeat(32)),
            route: None,
            query: String::new(),
            join: Some(format!(
                "ocw://203.0.113.7?cert={}&world=liechtenstein@10",
                "dd".repeat(32)
            )),
        }
        .with_query()
    }

    #[test]
    fn the_join_info_names_both_hashes_and_no_secret() {
        let i = info();
        assert_eq!(
            i.query,
            format!(
                "server=https%3A%2F%2F203.0.113.7%3A47801&cert-hash={},{}",
                "aa".repeat(32),
                "bb".repeat(32)
            )
        );
        let board = JoinBoard::default();
        board.set(&i);
        let v: serde_json::Value = serde_json::from_str(&board.text()).unwrap();
        assert_eq!(v["cert_hashes"].as_array().unwrap().len(), 2);
        assert_eq!(v["world"], "liechtenstein");
        assert!(v.get("route").is_none(), "a free ride has no route key");
        assert!(
            v["join"]
                .as_str()
                .unwrap()
                .starts_with("ocw://203.0.113.7?cert=")
        );
        let keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        for k in keys {
            assert!(
                !k.contains("key") && !k.contains("secret") && !k.contains("token"),
                "{k}"
            );
        }
    }

    #[test]
    fn join_json_is_answered_with_cors_and_without_cache() {
        let board = JoinBoard::default();
        // Not ready before the first set.
        let Route::Answer(r) = route(b"GET /join.json HTTP/1.1\r\n\r\n", &board, None, true) else {
            panic!()
        };
        assert!(r.starts_with(b"HTTP/1.1 503"));
        board.set(&info());
        let Route::Answer(r) = route(
            b"GET /join.json?t=1 HTTP/1.1\r\nHost: x\r\n\r\n",
            &board,
            None,
            true,
        ) else {
            panic!()
        };
        let text = String::from_utf8(r).unwrap();
        assert!(text.starts_with("HTTP/1.1 200 OK\r\n"), "{text}");
        assert!(text.contains("Access-Control-Allow-Origin: *\r\n"));
        assert!(text.contains("Cache-Control: no-store\r\n"));
        assert!(text.contains("Connection: close\r\n"));
        assert!(text.ends_with(&board.text()));
        // HEAD: no body; OPTIONS (a preflight): 204; others refused.
        let Route::Answer(r) = route(b"HEAD /join.json HTTP/1.1\r\n\r\n", &board, None, true)
        else {
            panic!()
        };
        assert!(String::from_utf8(r).unwrap().ends_with("\r\n\r\n"));
        let Route::Answer(r) = route(b"OPTIONS /join.json HTTP/1.1\r\n\r\n", &board, None, true)
        else {
            panic!()
        };
        assert!(r.starts_with(b"HTTP/1.1 204"));
        let Route::Answer(r) = route(b"POST /join.json HTTP/1.1\r\n\r\n", &board, None, true)
        else {
            panic!()
        };
        assert!(r.starts_with(b"HTTP/1.1 405"));
    }

    #[test]
    fn other_requests_pass_on_upgrades_unchanged() {
        let board = JoinBoard::default();
        let ws = b"GET /rtc?access_token=x HTTP/1.1\r\nHost: a\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n";
        assert_eq!(route(ws, &board, None, true), Route::Pass(ws.to_vec()));
        // A plain request: Connection: close instead of keep-alive.
        let plain = b"GET /rtc/validate HTTP/1.1\r\nHost: a\r\nConnection: keep-alive\r\n\r\n";
        assert_eq!(
            route(plain, &board, None, true),
            Route::Pass(
                b"GET /rtc/validate HTTP/1.1\r\nHost: a\r\nConnection: close\r\n\r\n".to_vec()
            )
        );
        // Without an upstream: 404.
        let Route::Answer(r) = route(plain, &board, None, false) else {
            panic!()
        };
        assert!(r.starts_with(b"HTTP/1.1 404"));
    }

    /// Over real sockets: `/join.json` answered, a WebSocket-like request
    /// joined to the upstream both ways.
    #[test]
    fn serves_and_passes_on_over_tcp() {
        use std::io::{Read, Write};
        let up = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let up_addr = up.local_addr().unwrap();
        let echo = std::thread::spawn(move || {
            let (mut s, _) = up.accept().unwrap();
            let mut got = [0u8; 4096];
            let mut n = 0;
            while !got[..n].ends_with(b"ping") {
                n += s.read(&mut got[n..]).unwrap();
            }
            s.write_all(b"HTTP/1.1 101 Switching Protocols\r\n\r\npong")
                .unwrap();
            String::from_utf8_lossy(&got[..n]).into_owned()
        });
        let board = JoinBoard::default();
        board.set(&info());
        let front =
            JoinFront::spawn("127.0.0.1:0".parse().unwrap(), Some(up_addr), board.clone()).unwrap();
        let mut c = std::net::TcpStream::connect(front.local_addr()).unwrap();
        c.write_all(b"GET /join.json HTTP/1.1\r\nHost: x\r\n\r\n")
            .unwrap();
        let mut text = String::new();
        c.read_to_string(&mut text).unwrap();
        assert!(text.starts_with("HTTP/1.1 200") && text.ends_with(&board.text()));
        let mut ws = std::net::TcpStream::connect(front.local_addr()).unwrap();
        ws.write_all(b"GET /rtc HTTP/1.1\r\nUpgrade: websocket\r\n\r\nping")
            .unwrap();
        let mut reply = [0u8; 64];
        let mut n = 0;
        while !reply[..n].ends_with(b"pong") {
            n += ws.read(&mut reply[n..]).unwrap();
        }
        let seen = echo.join().unwrap();
        assert!(
            seen.starts_with("GET /rtc HTTP/1.1\r\nUpgrade: websocket\r\n\r\n"),
            "{seen}"
        );
    }

    fn status() -> StatusBoard {
        let s = StatusBoard::default();
        s.set_page(PageInfo {
            desktop_link: format!(
                "ocw://203.0.113.7?cert={}&world=liechtenstein@10",
                "dd".repeat(32)
            ),
            world_name: "Liechtenstein".into(),
            world_version: Some(10),
            route: None,
            version: "0.2.6".into(),
        });
        s
    }

    fn answer(head: &[u8], status: &StatusBoard) -> Option<String> {
        let board = JoinBoard::default();
        board.set(&info());
        match route(head, &board, Some(status), true) {
            Route::Answer(r) => Some(String::from_utf8_lossy(&r).into_owned()),
            Route::Pass(_) => None,
        }
    }

    /// A browser's `GET /` gets the page; a WebSocket to `/` and every
    /// LiveKit path go on to the voice server (spec 11.5 *Status page*).
    #[test]
    fn the_root_is_the_page_for_browsers_and_livekit_keeps_its_paths() {
        let s = status();
        let page = answer(
            b"GET / HTTP/1.1\r\nHost: ocw-server.fly.dev\r\nAccept: text/html\r\n\r\n",
            &s,
        )
        .expect("the page");
        assert!(page.starts_with("HTTP/1.1 200 OK\r\n"), "{page}");
        assert!(page.contains("Content-Type: text/html; charset=utf-8\r\n"));
        assert!(page.contains("Content-Security-Policy: default-src 'none';"));
        assert!(page.contains("Ride now in your browser"));
        assert!(page.contains("https://newkit.github.io/OCW/?together=1"));
        assert!(page.contains(&format!(
            "ocw://203.0.113.7?cert={}&amp;world=liechtenstein@10",
            "dd".repeat(32)
        )));
        assert!(page.contains("<dd>Liechtenstein</dd>") && page.contains("<dd>10</dd>"));
        assert!(!page.contains("{{"), "every placeholder is filled");
        // With a query too; HEAD: no body.
        assert!(answer(b"GET /?x=1 HTTP/1.1\r\n\r\n", &s).is_some());
        let head = answer(b"HEAD / HTTP/1.1\r\n\r\n", &s).unwrap();
        assert!(head.ends_with("\r\n\r\n"));
        // A WebSocket upgrade to `/` (any case) and LiveKit's paths pass.
        for req in [
            &b"GET / HTTP/1.1\r\nConnection: Upgrade\r\nupgrade: websocket\r\n\r\n"[..],
            b"GET /rtc?access_token=x HTTP/1.1\r\nUpgrade: websocket\r\n\r\n",
            b"GET /rtc/v1?access_token=x HTTP/1.1\r\nUpgrade: websocket\r\n\r\n",
            b"GET /rtc/validate?access_token=x HTTP/1.1\r\n\r\n",
            b"GET /rtc/v1/validate?access_token=x HTTP/1.1\r\n\r\n",
            b"POST /twirp/livekit.RoomService/ListRooms HTTP/1.1\r\n\r\n",
            b"POST / HTTP/1.1\r\n\r\n",
            b"GET /favicon.ico HTTP/1.1\r\n\r\n",
            b"GET /index.html HTTP/1.1\r\n\r\n",
        ] {
            assert_eq!(answer(req, &s), None, "{}", String::from_utf8_lossy(req));
        }
        // Without the status page `/` goes on as before.
        let board = JoinBoard::default();
        assert!(matches!(
            route(b"GET / HTTP/1.1\r\n\r\n", &board, None, true),
            Route::Pass(_)
        ));
    }

    #[test]
    fn join_json_stays_as_it_was_with_the_status_page() {
        let s = status();
        let board = JoinBoard::default();
        board.set(&info());
        let with = route(b"GET /join.json HTTP/1.1\r\n\r\n", &board, Some(&s), true);
        let without = route(b"GET /join.json HTTP/1.1\r\n\r\n", &board, None, true);
        assert_eq!(with, without);
    }

    #[test]
    fn stats_json_is_counts_with_cors_and_without_cache() {
        let s = status();
        let r = answer(b"GET /stats.json HTTP/1.1\r\n\r\n", &s).unwrap();
        assert!(
            r.starts_with("HTTP/1.1 503"),
            "not ready before the first set"
        );
        let mut stats = crate::public_stats::PublicStats::new(1_795_694_400);
        let mut o = crate::public_stats::Observation::default();
        o.add(false, 8.0);
        o.add(true, 0.0);
        stats.observe(1_795_694_400, 1.0, &o);
        s.set_stats(&stats.report(
            1_795_694_410,
            "0.2.6",
            &crate::public_stats::WorldNote::default(),
        ));
        let r = answer(b"GET /stats.json?t=5 HTTP/1.1\r\n\r\n", &s).unwrap();
        assert!(r.starts_with("HTTP/1.1 200 OK\r\n"), "{r}");
        assert!(r.contains("Content-Type: application/json; charset=utf-8\r\n"));
        assert!(r.contains("Access-Control-Allow-Origin: *\r\n"));
        assert!(r.contains("Cache-Control: no-store\r\n"));
        let body = &r[r.find("\r\n\r\n").unwrap() + 4..];
        let v: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(
            (v["riders"]["humans"].as_u64(), v["riders"]["bots"].as_u64()),
            (Some(1), Some(1))
        );
        assert_eq!(v["uptime_s"], 10);
        let r = answer(b"OPTIONS /stats.json HTTP/1.1\r\n\r\n", &s).unwrap();
        assert!(r.starts_with("HTTP/1.1 204"));
        let r = answer(b"PUT /stats.json HTTP/1.1\r\n\r\n", &s).unwrap();
        assert!(r.starts_with("HTTP/1.1 405"));
    }

    /// The riders on the road by name (11.5 Q1 (b)): a hostile name is
    /// sent as text that no browser reads as markup, and the page puts
    /// names in only as text, with a cap and "+N more".
    #[test]
    fn named_riders_are_served_as_text_and_the_page_sets_them_as_text() {
        let s = status();
        let mut stats = crate::public_stats::PublicStats::new(1_795_694_400);
        let mut o = crate::public_stats::Observation::default();
        o.add_named(1, "Ada Rider", false, false, 8.0);
        o.add_named(2, "<script>alert(1)</script> & co", false, false, 8.0);
        o.add_named(1 << 63 | 3, "Echo", true, true, 0.0);
        stats.observe(1_795_694_400, 1.0, &o);
        s.set_stats(&stats.report(
            1_795_694_460,
            "0.2.6",
            &crate::public_stats::WorldNote::default(),
        ));
        let r = answer(b"GET /stats.json HTTP/1.1\r\n\r\n", &s).unwrap();
        assert!(r.contains("Content-Type: application/json; charset=utf-8\r\n"));
        assert!(r.contains("X-Content-Type-Options: nosniff\r\n"));
        let body = &r[r.find("\r\n\r\n").unwrap() + 4..];
        assert!(
            !body.contains('<') && !body.contains('>') && !body.contains('&'),
            "{body}"
        );
        let v: serde_json::Value = serde_json::from_str(body).unwrap();
        let online = v["riders"]["online"].as_array().unwrap();
        let names: Vec<&str> = online.iter().map(|o| o["name"].as_str().unwrap()).collect();
        assert_eq!(
            names,
            ["<script>alert(1)</script> & co", "Ada Rider", "Echo"]
        );
        assert_eq!(
            (online[2]["bot"].as_bool(), online[2]["echo"].as_bool()),
            (Some(true), Some(true))
        );
        assert_eq!(online[1]["riding_s"], 60);
        assert_eq!(v["riders"]["more"], 0);
        // The page: names only through textContent, never as HTML.
        let page = s.page_html();
        for bad in [
            "innerHTML",
            "outerHTML",
            "insertAdjacentHTML",
            "document.write",
        ] {
            assert!(!page.contains(bad), "{bad}");
        }
        assert!(page.contains("String(o.name)") && page.contains("e.textContent = text"));
        assert!(page.contains("var CAP = 50;") && page.contains("\" more\""));
        assert!(page.contains("id=\"names\""));
        // The privacy note says what is shown.
        assert!(page.contains("The names riders ride under are shown here while they ride"));
        assert!(page.contains("no IP addresses") && page.contains("no positions"));
        assert!(!page.contains("counts only"));
    }

    /// The eight tiles fill whole rows: 4 a row on desktop, 2 on a phone
    /// (not 5 + 3 from `auto-fill`).
    #[test]
    fn the_stat_tiles_fill_whole_rows() {
        let page = status().page_html();
        let tiles = page.matches("<div class=\"stat\">").count();
        assert_eq!(tiles, 8);
        assert!(!page.contains("auto-fill"));
        assert!(
            page.contains(
                ".grid { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr));"
            )
        );
        assert!(page.contains(".grid { grid-template-columns: repeat(2, minmax(0, 1fr)); }"));
        assert_eq!(tiles % 4, 0);
    }

    #[test]
    fn the_page_files_are_under_their_own_prefix() {
        let s = status();
        let r = answer(b"GET /_ocw/icon.svg HTTP/1.1\r\n\r\n", &s).unwrap();
        assert!(r.starts_with("HTTP/1.1 200 OK\r\n"), "{r}");
        assert!(r.contains("Content-Type: image/svg+xml\r\n"));
        assert!(r.contains("Cache-Control: public, max-age=86400\r\n"));
        assert!(!r.contains("no-store"));
        assert!(r.contains("<svg"));
        let r = answer(b"GET /_ocw/../join.json HTTP/1.1\r\n\r\n", &s).unwrap();
        assert!(r.starts_with("HTTP/1.1 404"));
        let r = answer(b"POST /_ocw/icon.svg HTTP/1.1\r\n\r\n", &s).unwrap();
        assert!(r.starts_with("HTTP/1.1 405"));
    }

    #[test]
    fn page_values_are_escaped() {
        let s = StatusBoard::default();
        s.set_page(PageInfo {
            world_name: "<script>alert(1)</script>".into(),
            route: Some("A & B".into()),
            ..Default::default()
        });
        let page = s.page_html();
        assert!(!page.contains("<script>alert"));
        assert!(page.contains("&lt;script&gt;") && page.contains("the route A &amp; B"));
        assert!(page.contains("(not ready yet)"));
    }

    /// Over real sockets: the page from the endpoint, a WebSocket to `/`
    /// passed on.
    #[test]
    fn serves_the_page_and_passes_a_websocket_to_the_root_over_tcp() {
        use std::io::{Read, Write};
        let up = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let up_addr = up.local_addr().unwrap();
        let echo = std::thread::spawn(move || {
            let (mut s, _) = up.accept().unwrap();
            let mut got = [0u8; 4096];
            let mut n = 0;
            while !got[..n].ends_with(b"\r\n\r\n") {
                n += s.read(&mut got[n..]).unwrap();
            }
            s.write_all(b"HTTP/1.1 101 Switching Protocols\r\n\r\n")
                .unwrap();
            String::from_utf8_lossy(&got[..n]).into_owned()
        });
        let front = JoinFront::spawn_with(
            "127.0.0.1:0".parse().unwrap(),
            Some(up_addr),
            JoinBoard::default(),
            Some(status()),
        )
        .unwrap();
        let mut c = std::net::TcpStream::connect(front.local_addr()).unwrap();
        c.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
        let mut text = String::new();
        c.read_to_string(&mut text).unwrap();
        assert!(text.starts_with("HTTP/1.1 200") && text.contains("</html>"));
        let mut ws = std::net::TcpStream::connect(front.local_addr()).unwrap();
        ws.write_all(b"GET / HTTP/1.1\r\nUpgrade: websocket\r\n\r\n")
            .unwrap();
        let mut reply = [0u8; 64];
        let mut n = 0;
        while !reply[..n].ends_with(b"\r\n\r\n") {
            n += ws.read(&mut reply[n..]).unwrap();
        }
        assert!(reply.starts_with(b"HTTP/1.1 101"));
        assert!(
            echo.join()
                .unwrap()
                .starts_with("GET / HTTP/1.1\r\nUpgrade: websocket")
        );
    }
}
