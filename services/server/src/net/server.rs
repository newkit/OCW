//! The server side: QUIC endpoints feeding a [`GameServer`] – the native
//! one (ALPN `ocw/1`) and, for the browser app, a WebTransport one (HTTP/3,
//! [`super::webtransport`]). Both carry the same messages the same way:
//! one bidirectional stream of frames, datagrams for snapshots and samples.
//!
//! Networking runs on a small Tokio runtime owned by [`NetServer`]; the game
//! loop stays synchronous (one thread, fixed 50 Hz tick) and talks to the
//! connection tasks only through channels:
//!
//! ```text
//! connection task ──NetEvent──▶ NetServer::pump ──▶ GameServer::handle
//! connection task ◀──outbound── NetServer::broadcast ◀── GameServer::tick
//! ```

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use bytes::Bytes;
use quinn::{Connection, Endpoint, Incoming, VarInt};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc as tmpsc;

use super::framing::{read_frame, write_frame};
use super::tls::WebIdentity;
use super::{ServerIdentity, close_code, server_unreliable, webtransport};
use crate::{ConnectionId, GameServer};

/// Messages waiting for one client. Snapshots beyond this are dropped; a
/// client this far behind on reliable messages is disconnected.
const OUTBOUND_QUEUE: usize = 256;
/// A client must open its stream (and say Hello) within this time.
const STREAM_TIMEOUT: Duration = Duration::from_secs(10);

/// Which endpoint a client came through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Transport {
    /// Native QUIC (`ocw-desktop`, `ocw-bot`).
    Quic,
    /// WebTransport over HTTP/3 (the browser app, `ocw-bot` with https://).
    WebTransport,
}

impl Transport {
    pub fn name(self) -> &'static str {
        match self {
            Self::Quic => "QUIC",
            Self::WebTransport => "WebTransport",
        }
    }
}

/// A client's connection: a QUIC connection or a WebTransport session
/// (which prefixes its datagrams with the session id and closes with a
/// capsule).
#[derive(Clone)]
pub(crate) enum PeerConn {
    Quic(Connection),
    // Boxed: a session is much larger than a connection handle.
    Web(Box<web_transport_quinn::Session>),
}

impl PeerConn {
    pub(crate) fn transport(&self) -> Transport {
        match self {
            Self::Quic(_) => Transport::Quic,
            Self::Web(_) => Transport::WebTransport,
        }
    }

    /// The QUIC connection underneath.
    pub(crate) fn quic(&self) -> &Connection {
        match self {
            Self::Quic(c) => c,
            Self::Web(s) => s,
        }
    }

    pub(crate) fn remote_address(&self) -> SocketAddr {
        self.quic().remote_address()
    }

    pub(crate) fn close(&self, code: u32, reason: &[u8]) {
        match self {
            Self::Quic(c) => c.close(VarInt::from_u32(code), reason),
            Self::Web(s) => s.close(code, reason),
        }
    }

    /// Why the connection ended, if it did.
    pub(crate) fn close_reason(&self) -> Option<String> {
        match self {
            Self::Quic(c) => c.close_reason().map(|e| e.to_string()),
            Self::Web(s) => s.close_reason().map(|e| e.to_string()),
        }
    }

    /// The largest datagram payload the peer takes (`None`: no datagrams).
    pub(crate) fn max_datagram(&self) -> Option<usize> {
        match self {
            Self::Quic(c) => c.max_datagram_size(),
            Self::Web(s) => Some(s.max_datagram_size()).filter(|&m| m > 0),
        }
    }

    /// Sends one unreliable datagram if the peer takes one this size.
    pub(crate) fn try_datagram(&self, bytes: &[u8]) -> bool {
        if !self.max_datagram().is_some_and(|max| bytes.len() <= max) {
            return false;
        }
        let b = Bytes::copy_from_slice(bytes);
        match self {
            Self::Quic(c) => c.send_datagram(b).is_ok(),
            Self::Web(s) => s.send_datagram(b).is_ok(),
        }
    }

    pub(crate) async fn read_datagram(&self) -> Result<Bytes, String> {
        match self {
            Self::Quic(c) => c.read_datagram().await.map_err(|e| e.to_string()),
            Self::Web(s) => s.read_datagram().await.map_err(|e| e.to_string()),
        }
    }
}

/// What the connection tasks tell the game loop.
enum NetEvent {
    Connected {
        token: u64,
        conn: PeerConn,
        outbound: tmpsc::Sender<Vec<u8>>,
    },
    Message {
        token: u64,
        bytes: Vec<u8>,
        datagram: bool,
    },
    Closed {
        token: u64,
        reason: String,
    },
}

struct Peer {
    id: ConnectionId,
    conn: PeerConn,
    outbound: tmpsc::Sender<Vec<u8>>,
}

/// Traffic of one endpoint since the start.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EndpointCounters {
    /// Connections (sessions) that came up.
    pub connections: u64,
    /// Messages received (on the stream and as datagrams).
    pub messages_in: u64,
    /// Of those, datagrams.
    pub datagrams_in: u64,
}

/// The WebTransport endpoint (spec §14 Phase 10 B3).
struct WebEndpoint {
    endpoint: Endpoint,
    local_addr: SocketAddr,
    identity: WebIdentity,
    /// Where its certificate is kept (`--tls-dir`), for the rotation.
    dir: Option<PathBuf>,
}

/// A listening game server endpoint.
pub struct NetServer {
    runtime: Option<tokio::runtime::Runtime>,
    endpoint: Endpoint,
    web: Option<WebEndpoint>,
    events_tx: mpsc::Sender<NetEvent>,
    events: mpsc::Receiver<NetEvent>,
    next_token: Arc<AtomicU64>,
    peers: HashMap<u64, Peer>,
    local_addr: SocketAddr,
    fingerprint: String,
    shutdown: Arc<AtomicBool>,
    quic_counters: EndpointCounters,
    web_counters: EndpointCounters,
}

impl NetServer {
    /// Binds the UDP socket and starts accepting connections.
    pub fn bind(addr: SocketAddr, identity: &ServerIdentity) -> Result<Self, String> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("ocw-net")
            .enable_all()
            .build()
            .map_err(|e| format!("network runtime: {e}"))?;
        let config = identity.server_config()?;
        let endpoint = {
            let _rt = runtime.enter();
            Endpoint::server(config, addr).map_err(|e| format!("cannot listen on {addr}: {e}"))?
        };
        let local_addr = endpoint.local_addr().map_err(|e| e.to_string())?;
        let (events_tx, events) = mpsc::channel();
        let next_token = Arc::new(AtomicU64::new(1));
        runtime.spawn(accept_loop(
            endpoint.clone(),
            events_tx.clone(),
            next_token.clone(),
            Transport::Quic,
        ));
        Ok(Self {
            runtime: Some(runtime),
            endpoint,
            web: None,
            events_tx,
            events,
            next_token,
            peers: HashMap::new(),
            local_addr,
            fingerprint: identity.fingerprint(),
            shutdown: Arc::new(AtomicBool::new(false)),
            quic_counters: EndpointCounters::default(),
            web_counters: EndpointCounters::default(),
        })
    }

    /// Also accepts browsers: a WebTransport endpoint on its own UDP
    /// address with the certificate browsers take by hash. `dir`: where
    /// the certificate is kept (renewed certificates are stored there
    /// too). Returns the bound address.
    pub fn listen_webtransport(
        &mut self,
        addr: SocketAddr,
        identity: WebIdentity,
        dir: Option<PathBuf>,
    ) -> Result<SocketAddr, String> {
        let rt = self.runtime.as_ref().ok_or("the server is shut down")?;
        let config = identity.server_config()?;
        let endpoint = {
            let _rt = rt.enter();
            Endpoint::server(config, addr)
                .map_err(|e| format!("cannot listen on {addr} (WebTransport): {e}"))?
        };
        let local_addr = endpoint.local_addr().map_err(|e| e.to_string())?;
        rt.spawn(accept_loop(
            endpoint.clone(),
            self.events_tx.clone(),
            self.next_token.clone(),
            Transport::WebTransport,
        ));
        self.web = Some(WebEndpoint {
            endpoint,
            local_addr,
            identity,
            dir,
        });
        Ok(local_addr)
    }

    /// The bound address (with the actual port when bound to port 0).
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// The WebTransport endpoint's address, if it listens.
    pub fn web_addr(&self) -> Option<SocketAddr> {
        self.web.as_ref().map(|w| w.local_addr)
    }

    /// SHA-256 of the WebTransport certificate (the page's `cert-hash`).
    pub fn web_cert_hash(&self) -> Option<String> {
        self.web.as_ref().map(|w| w.identity.cert_hash())
    }

    /// When the WebTransport certificate expires (Unix s).
    pub fn web_cert_not_after(&self) -> Option<i64> {
        self.web.as_ref().map(|w| w.identity.not_after_unix_s())
    }

    /// Renews the WebTransport certificate when it has less than two days
    /// left (call now and then, e.g. every minute). New sessions get the
    /// new certificate, open ones go on. Returns the new hash.
    pub fn rotate_web_cert(&mut self, now_unix_s: i64) -> Result<Option<String>, String> {
        let Some(web) = self.web.as_mut() else {
            return Ok(None);
        };
        if !web.identity.needs_rotation(now_unix_s) {
            return Ok(None);
        }
        let identity = WebIdentity::generate(now_unix_s)?;
        web.endpoint
            .set_server_config(Some(identity.server_config()?));
        if let Some(dir) = &web.dir {
            identity.store(dir)?;
        }
        let hash = identity.cert_hash();
        web.identity = identity;
        Ok(Some(hash))
    }

    /// SHA-256 fingerprint of the server certificate, for `--server-cert`.
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    /// Number of open connections.
    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }

    /// Open connections through one endpoint.
    pub fn peers_on(&self, transport: Transport) -> usize {
        self.peers
            .values()
            .filter(|p| p.conn.transport() == transport)
            .count()
    }

    /// Traffic of an endpoint since the start.
    pub fn counters(&self, transport: Transport) -> EndpointCounters {
        match transport {
            Transport::Quic => self.quic_counters,
            Transport::WebTransport => self.web_counters,
        }
    }

    /// A flag set on Ctrl-C / SIGINT (and SIGTERM on Unix), for the game loop
    /// to shut down gracefully.
    pub fn shutdown_on_signal(&self) -> Arc<AtomicBool> {
        let flag = self.shutdown.clone();
        if let Some(rt) = &self.runtime {
            let f = flag.clone();
            rt.spawn(async move {
                wait_for_signal().await;
                f.store(true, Ordering::SeqCst);
            });
        }
        flag
    }

    /// Delivers everything the connections received since the last call to
    /// `game` and sends its replies. Call once per tick, before
    /// [`GameServer::tick`].
    pub fn pump(&mut self, game: &mut GameServer) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                NetEvent::Connected {
                    token,
                    conn,
                    outbound,
                } => {
                    let id = game.connect();
                    let transport = conn.transport();
                    tracing::info!(remote = %conn.remote_address(), ?id, transport = transport.name(), "client connected");
                    match transport {
                        Transport::Quic => self.quic_counters.connections += 1,
                        Transport::WebTransport => self.web_counters.connections += 1,
                    }
                    self.peers.insert(token, Peer { id, conn, outbound });
                }
                NetEvent::Message {
                    token,
                    bytes,
                    datagram,
                } => {
                    let Some(peer) = self.peers.get(&token) else {
                        continue;
                    };
                    let c = match peer.conn.transport() {
                        Transport::Quic => &mut self.quic_counters,
                        Transport::WebTransport => &mut self.web_counters,
                    };
                    c.messages_in += 1;
                    c.datagrams_in += u64::from(datagram);
                    let replies = game.handle(peer.id, &bytes);
                    for reply in replies {
                        deliver(peer, reply);
                    }
                }
                NetEvent::Closed { token, reason } => {
                    if let Some(peer) = self.peers.remove(&token) {
                        tracing::info!(remote = %peer.conn.remote_address(), transport = peer.conn.transport().name(), %reason, "client disconnected");
                        game.disconnect(peer.id);
                    }
                }
            }
        }
    }

    /// Sends the messages of one [`GameServer::tick`] to every client that
    /// has joined.
    pub fn broadcast(&mut self, game: &GameServer, messages: &[Vec<u8>]) {
        if messages.is_empty() {
            return;
        }
        let joined: std::collections::BTreeSet<ConnectionId> = game.connections().collect();
        for peer in self.peers.values().filter(|p| joined.contains(&p.id)) {
            for m in messages {
                deliver(peer, m.clone());
            }
        }
    }

    /// Sends one message to the given connections (a world change goes to
    /// the riders of the ended session, a late answer to its asker).
    pub fn send_to(&mut self, conns: &[ConnectionId], bytes: &[u8]) {
        for peer in self.peers.values().filter(|p| conns.contains(&p.id)) {
            deliver(peer, bytes.to_vec());
        }
    }

    /// Closes all connections ("server shutting down") and waits briefly
    /// for the clients to acknowledge.
    pub fn shutdown(mut self) {
        self.close();
    }

    fn close(&mut self) {
        let Some(rt) = self.runtime.take() else {
            return;
        };
        // WebTransport sessions first: their close capsule needs the runtime.
        {
            let _rt = rt.enter();
            for p in self.peers.values() {
                if let PeerConn::Web(s) = &p.conn {
                    s.close(close_code::NORMAL, b"server shutting down");
                }
            }
        }
        let mut endpoints = vec![self.endpoint.clone()];
        endpoints.extend(self.web.as_ref().map(|w| w.endpoint.clone()));
        rt.block_on(async move {
            // Let the capsules go out before the connections close.
            tokio::time::sleep(Duration::from_millis(20)).await;
            for e in &endpoints {
                e.close(
                    VarInt::from_u32(close_code::NORMAL),
                    b"server shutting down",
                );
            }
            for e in &endpoints {
                let _ = tokio::time::timeout(Duration::from_secs(1), e.wait_idle()).await;
            }
        });
        rt.shutdown_timeout(Duration::from_millis(500));
    }
}

impl Drop for NetServer {
    fn drop(&mut self) {
        self.close();
    }
}

/// Queues one message for a client. A full queue drops snapshots (the next
/// one supersedes them); for a reliable message it means the client cannot
/// keep up, and it is disconnected rather than letting memory grow.
fn deliver(peer: &Peer, bytes: Vec<u8>) {
    let unreliable = server_unreliable(&bytes);
    match peer.outbound.try_send(bytes) {
        Ok(()) => {}
        Err(tmpsc::error::TrySendError::Full(_)) if unreliable => {}
        Err(tmpsc::error::TrySendError::Full(_)) => {
            tracing::warn!(remote = %peer.conn.remote_address(), "client too slow, disconnecting");
            peer.conn.close(close_code::TOO_SLOW, b"too slow");
        }
        // The task ended; its Closed event follows.
        Err(tmpsc::error::TrySendError::Closed(_)) => {}
    }
}

async fn wait_for_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut term = match signal(SignalKind::terminate()) {
            Ok(s) => s,
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
                return;
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

async fn accept_loop(
    endpoint: Endpoint,
    events: mpsc::Sender<NetEvent>,
    next_token: Arc<AtomicU64>,
    transport: Transport,
) {
    while let Some(incoming) = endpoint.accept().await {
        let token = next_token.fetch_add(1, Ordering::Relaxed);
        match transport {
            Transport::Quic => tokio::spawn(serve(incoming, token, events.clone())),
            Transport::WebTransport => tokio::spawn(serve_web(incoming, token, events.clone())),
        };
    }
}

/// One native QUIC connection, from the handshake to its end.
async fn serve(incoming: Incoming, token: u64, events: mpsc::Sender<NetEvent>) {
    let remote = incoming.remote_address();
    let conn = match incoming.await {
        Ok(c) => c,
        Err(e) => {
            tracing::info!(%remote, %e, "handshake failed");
            return;
        }
    };
    let peer = PeerConn::Quic(conn.clone());
    let Some(out_rx) = announce(&peer, token, &events) else {
        return;
    };
    let reason = match tokio::time::timeout(STREAM_TIMEOUT, conn.accept_bi()).await {
        Err(_) => {
            peer.close(close_code::PROTOCOL, b"no stream opened");
            "no stream opened".to_owned()
        }
        Ok(Err(e)) => e.to_string(),
        Ok(Ok((send, recv))) => run_peer(&peer, send, recv, token, &events, out_rx).await,
    };
    // No-op if it is closed already.
    peer.close(close_code::NORMAL, b"");
    let _ = events.send(NetEvent::Closed { token, reason });
}

/// One WebTransport session: the QUIC handshake (ALPN h3), HTTP/3's
/// SETTINGS and the CONNECT request (only for [`webtransport::PATH`]),
/// then the same stream and datagrams as a native connection.
async fn serve_web(incoming: Incoming, token: u64, events: mpsc::Sender<NetEvent>) {
    let remote = incoming.remote_address();
    let conn = match incoming.await {
        Ok(c) => c,
        Err(e) => {
            tracing::info!(%remote, %e, "WebTransport handshake failed");
            return;
        }
    };
    let request = match tokio::time::timeout(
        STREAM_TIMEOUT,
        web_transport_quinn::Request::accept(conn.clone()),
    )
    .await
    {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => {
            tracing::info!(%remote, %e, "no WebTransport session");
            conn.close(
                VarInt::from_u32(close_code::PROTOCOL),
                b"no WebTransport session",
            );
            return;
        }
        Err(_) => {
            conn.close(
                VarInt::from_u32(close_code::PROTOCOL),
                b"no CONNECT in time",
            );
            return;
        }
    };
    let path = request.url.path().to_owned();
    if !webtransport::accepts_path(&path) {
        tracing::info!(%remote, %path, "WebTransport session for an unknown path");
        let _ = request.reject(http_not_found()).await;
        return;
    }
    let session = match request.ok().await {
        Ok(s) => s,
        Err(e) => {
            tracing::info!(%remote, %e, "WebTransport session failed");
            return;
        }
    };
    let peer = PeerConn::Web(Box::new(session.clone()));
    let Some(out_rx) = announce(&peer, token, &events) else {
        return;
    };
    let reason = match tokio::time::timeout(STREAM_TIMEOUT, session.accept_bi()).await {
        Err(_) => {
            peer.close(close_code::PROTOCOL, b"no stream opened");
            "no stream opened".to_owned()
        }
        Ok(Err(e)) => e.to_string(),
        Ok(Ok((send, recv))) => run_peer(&peer, send, recv, token, &events, out_rx).await,
    };
    peer.close(close_code::NORMAL, b"");
    let _ = events.send(NetEvent::Closed { token, reason });
}

fn http_not_found() -> web_transport_quinn::http::StatusCode {
    web_transport_quinn::http::StatusCode::NOT_FOUND
}

/// Tells the game loop about a new connection; its outbound queue.
fn announce(
    peer: &PeerConn,
    token: u64,
    events: &mpsc::Sender<NetEvent>,
) -> Option<tmpsc::Receiver<Vec<u8>>> {
    let (outbound, out_rx) = tmpsc::channel::<Vec<u8>>(OUTBOUND_QUEUE);
    events
        .send(NetEvent::Connected {
            token,
            conn: peer.clone(),
            outbound,
        })
        .ok()?;
    Some(out_rx)
}

/// The stream's frames and the datagrams in, the outbound queue out, until
/// the connection ends; returns why.
async fn run_peer<S, R>(
    conn: &PeerConn,
    mut send: S,
    mut recv: R,
    token: u64,
    events: &mpsc::Sender<NetEvent>,
    mut out_rx: tmpsc::Receiver<Vec<u8>>,
) -> String
where
    S: AsyncWrite + Unpin,
    R: AsyncRead + Unpin,
{
    let reader = async {
        loop {
            match read_frame(&mut recv).await {
                Ok(Some(bytes)) => {
                    let m = NetEvent::Message {
                        token,
                        bytes,
                        datagram: false,
                    };
                    if events.send(m).is_err() {
                        return "server stopped".to_owned();
                    }
                }
                Ok(None) => return "client closed its stream".to_owned(),
                // The connection ended (client quit, timeout): say why.
                Err(_) if conn.close_reason().is_some() => {
                    return conn.close_reason().unwrap_or_default();
                }
                Err(e) => {
                    conn.close(close_code::PROTOCOL, b"bad frame");
                    return format!("bad frame: {e}");
                }
            }
        }
    };
    let datagrams = async {
        loop {
            match conn.read_datagram().await {
                Ok(d) => {
                    let m = NetEvent::Message {
                        token,
                        bytes: d.to_vec(),
                        datagram: true,
                    };
                    if events.send(m).is_err() {
                        return "server stopped".to_owned();
                    }
                }
                Err(e) => return e,
            }
        }
    };
    let writer = async {
        while let Some(bytes) = out_rx.recv().await {
            if server_unreliable(&bytes) && conn.try_datagram(&bytes) {
                continue;
            }
            if let Err(e) = write_frame(&mut send, &bytes).await {
                return e.to_string();
            }
        }
        "server stopped".to_owned()
    };
    tokio::select! {
        r = reader => r,
        r = datagrams => r,
        r = writer => r,
    }
}
