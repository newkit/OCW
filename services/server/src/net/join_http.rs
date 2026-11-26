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

/// What to do with a request head.
#[derive(Debug, PartialEq, Eq)]
pub enum Route {
    /// Answer it here with these bytes (and close).
    Answer(Vec<u8>),
    /// Pass it on, with this head (maybe with `Connection: close`).
    Pass(Vec<u8>),
}

/// Decides what to do with a complete request head (`head` ends with the
/// blank line); `upstream`: whether there is one to pass on to.
pub fn route(head: &[u8], board: &JoinBoard, upstream: bool) -> Route {
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

/// An HTTP/1.1 response with the CORS headers, closing the connection.
fn response(code: u16, reason: &str, kind: &str, body: &[u8], method: &str) -> Vec<u8> {
    let mut out = format!(
        "HTTP/1.1 {code} {reason}\r\n\
         Content-Type: {kind}; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Cache-Control: no-store\r\n\
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
    let upgrade = text
        .lines()
        .skip(1)
        .any(|l| l.to_ascii_lowercase().starts_with("upgrade:"));
    if upgrade {
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
                        tokio::spawn(async move {
                            if let Err(e) = serve(conn, upstream, &board).await {
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
    match route(&buf[..end], board, upstream.is_some()) {
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
        let Route::Answer(r) = route(b"GET /join.json HTTP/1.1\r\n\r\n", &board, true) else {
            panic!()
        };
        assert!(r.starts_with(b"HTTP/1.1 503"));
        board.set(&info());
        let Route::Answer(r) = route(
            b"GET /join.json?t=1 HTTP/1.1\r\nHost: x\r\n\r\n",
            &board,
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
        let Route::Answer(r) = route(b"HEAD /join.json HTTP/1.1\r\n\r\n", &board, true) else {
            panic!()
        };
        assert!(String::from_utf8(r).unwrap().ends_with("\r\n\r\n"));
        let Route::Answer(r) = route(b"OPTIONS /join.json HTTP/1.1\r\n\r\n", &board, true) else {
            panic!()
        };
        assert!(r.starts_with(b"HTTP/1.1 204"));
        let Route::Answer(r) = route(b"POST /join.json HTTP/1.1\r\n\r\n", &board, true) else {
            panic!()
        };
        assert!(r.starts_with(b"HTTP/1.1 405"));
    }

    #[test]
    fn other_requests_pass_on_upgrades_unchanged() {
        let board = JoinBoard::default();
        let ws = b"GET /rtc?access_token=x HTTP/1.1\r\nHost: a\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n";
        assert_eq!(route(ws, &board, true), Route::Pass(ws.to_vec()));
        // A plain request: Connection: close instead of keep-alive.
        let plain = b"GET /rtc/validate HTTP/1.1\r\nHost: a\r\nConnection: keep-alive\r\n\r\n";
        assert_eq!(
            route(plain, &board, true),
            Route::Pass(
                b"GET /rtc/validate HTTP/1.1\r\nHost: a\r\nConnection: close\r\n\r\n".to_vec()
            )
        );
        // Without an upstream: 404.
        let Route::Answer(r) = route(plain, &board, false) else {
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
}
