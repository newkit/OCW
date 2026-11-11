//! The server side: a QUIC endpoint feeding a [`GameServer`].
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
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use bytes::Bytes;
use quinn::{Connection, Endpoint, Incoming, VarInt};
use tokio::sync::mpsc as tmpsc;

use super::framing::{read_frame, write_frame};
use super::{ServerIdentity, close_code, server_unreliable};
use crate::{ConnectionId, GameServer};

/// Messages waiting for one client. Snapshots beyond this are dropped; a
/// client this far behind on reliable messages is disconnected.
const OUTBOUND_QUEUE: usize = 256;
/// A client must open its stream (and say Hello) within this time.
const STREAM_TIMEOUT: Duration = Duration::from_secs(10);

/// What the connection tasks tell the game loop.
enum NetEvent {
    Connected {
        token: u64,
        conn: Connection,
        outbound: tmpsc::Sender<Vec<u8>>,
    },
    Message {
        token: u64,
        bytes: Vec<u8>,
    },
    Closed {
        token: u64,
        reason: String,
    },
}

struct Peer {
    id: ConnectionId,
    conn: Connection,
    outbound: tmpsc::Sender<Vec<u8>>,
}

/// A listening game server endpoint.
pub struct NetServer {
    runtime: Option<tokio::runtime::Runtime>,
    endpoint: Endpoint,
    events: mpsc::Receiver<NetEvent>,
    peers: HashMap<u64, Peer>,
    local_addr: SocketAddr,
    fingerprint: String,
    shutdown: Arc<AtomicBool>,
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
        let (tx, events) = mpsc::channel();
        runtime.spawn(accept_loop(endpoint.clone(), tx));
        Ok(Self {
            runtime: Some(runtime),
            endpoint,
            events,
            peers: HashMap::new(),
            local_addr,
            fingerprint: identity.fingerprint(),
            shutdown: Arc::new(AtomicBool::new(false)),
        })
    }

    /// The bound address (with the actual port when bound to port 0).
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// SHA-256 fingerprint of the server certificate, for `--server-cert`.
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    /// Number of open connections.
    pub fn peer_count(&self) -> usize {
        self.peers.len()
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
                    tracing::info!(remote = %conn.remote_address(), ?id, "client connected");
                    self.peers.insert(token, Peer { id, conn, outbound });
                }
                NetEvent::Message { token, bytes } => {
                    let Some(peer) = self.peers.get(&token) else {
                        continue;
                    };
                    let replies = game.handle(peer.id, &bytes);
                    for reply in replies {
                        deliver(peer, reply);
                    }
                }
                NetEvent::Closed { token, reason } => {
                    if let Some(peer) = self.peers.remove(&token) {
                        tracing::info!(remote = %peer.conn.remote_address(), %reason, "client disconnected");
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

    /// Closes all connections ("server shutting down") and waits briefly
    /// for the clients to acknowledge.
    pub fn shutdown(mut self) {
        self.close();
    }

    fn close(&mut self) {
        let Some(rt) = self.runtime.take() else {
            return;
        };
        self.endpoint.close(
            VarInt::from_u32(close_code::NORMAL),
            b"server shutting down",
        );
        let endpoint = self.endpoint.clone();
        rt.block_on(async move {
            let _ = tokio::time::timeout(Duration::from_secs(1), endpoint.wait_idle()).await;
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
            peer.conn
                .close(VarInt::from_u32(close_code::TOO_SLOW), b"too slow");
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

async fn accept_loop(endpoint: Endpoint, events: mpsc::Sender<NetEvent>) {
    let mut next_token = 1u64;
    while let Some(incoming) = endpoint.accept().await {
        let token = next_token;
        next_token += 1;
        tokio::spawn(serve(incoming, token, events.clone()));
    }
}

/// One client connection, from the handshake to its end.
async fn serve(incoming: Incoming, token: u64, events: mpsc::Sender<NetEvent>) {
    let remote = incoming.remote_address();
    let conn = match incoming.await {
        Ok(c) => c,
        Err(e) => {
            tracing::info!(%remote, %e, "handshake failed");
            return;
        }
    };
    let (outbound, mut out_rx) = tmpsc::channel::<Vec<u8>>(OUTBOUND_QUEUE);
    if events
        .send(NetEvent::Connected {
            token,
            conn: conn.clone(),
            outbound,
        })
        .is_err()
    {
        return;
    }
    let reason = match tokio::time::timeout(STREAM_TIMEOUT, conn.accept_bi()).await {
        Err(_) => {
            conn.close(VarInt::from_u32(close_code::PROTOCOL), b"no stream opened");
            "no stream opened".to_owned()
        }
        Ok(Err(e)) => e.to_string(),
        Ok(Ok((mut send, mut recv))) => {
            let reader = async {
                loop {
                    match read_frame(&mut recv).await {
                        Ok(Some(bytes)) => {
                            if events.send(NetEvent::Message { token, bytes }).is_err() {
                                return "server stopped".to_owned();
                            }
                        }
                        Ok(None) => return "client closed its stream".to_owned(),
                        Err(e) => {
                            conn.close(VarInt::from_u32(close_code::PROTOCOL), b"bad frame");
                            return format!("bad frame: {e}");
                        }
                    }
                }
            };
            let datagrams = async {
                loop {
                    match conn.read_datagram().await {
                        Ok(d) => {
                            let bytes = d.to_vec();
                            if events.send(NetEvent::Message { token, bytes }).is_err() {
                                return "server stopped".to_owned();
                            }
                        }
                        Err(e) => return e.to_string(),
                    }
                }
            };
            let writer = async {
                while let Some(bytes) = out_rx.recv().await {
                    if server_unreliable(&bytes)
                        && conn
                            .max_datagram_size()
                            .is_some_and(|max| bytes.len() <= max)
                        && conn.send_datagram(Bytes::from(bytes.clone())).is_ok()
                    {
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
    };
    // No-op if it is closed already.
    conn.close(VarInt::from_u32(close_code::NORMAL), b"");
    let _ = events.send(NetEvent::Closed { token, reason });
}
