//! The client side: one connection to a game server – native QUIC, or
//! WebTransport like the browser app ([`NetClient::connect_web`]) – behind
//! the same "encoded messages in, encoded messages out" interface as the
//! in-process local server, so `ClientCore` runs unchanged on either.

use std::net::SocketAddr;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use quinn::{ConnectionError, Endpoint};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc as tmpsc;

use super::framing::{read_frame, write_frame};
use super::server::PeerConn;
use super::{ServerTrust, client_unreliable, close_code, webtransport};

pub use crate::wire::LinkStatus;

/// Client options (mostly for tests).
#[derive(Debug, Clone)]
pub struct ClientOptions {
    /// Accept datagrams from the server. Off: the server sends snapshots on
    /// the stream (the fallback for peers without datagram support).
    pub datagrams: bool,
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self { datagrams: true }
    }
}

struct Shared {
    status: LinkStatus,
    conn: Option<PeerConn>,
}

/// A connection to a remote game server.
pub struct NetClient {
    to_server: tmpsc::UnboundedSender<Vec<u8>>,
    from_server: mpsc::Receiver<Vec<u8>>,
    shared: Arc<Mutex<Shared>>,
    endpoint: Endpoint,
    runtime: Option<tokio::runtime::Runtime>,
}

fn runtime() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .thread_name("ocw-net-client")
        .enable_all()
        .build()
        .map_err(|e| format!("network runtime: {e}"))
}

fn client_endpoint(addr: SocketAddr) -> Result<Endpoint, String> {
    let bind: SocketAddr = if addr.is_ipv6() {
        "[::]:0".parse().expect("valid")
    } else {
        "0.0.0.0:0".parse().expect("valid")
    };
    Endpoint::client(bind).map_err(|e| format!("cannot open a UDP socket: {e}"))
}

impl NetClient {
    /// Starts connecting to `addr` in the background; messages sent before
    /// the connection is up are queued. `server_name` is the TLS name (SNI).
    pub fn connect(
        addr: SocketAddr,
        server_name: &str,
        trust: &ServerTrust,
        options: ClientOptions,
    ) -> Result<Self, String> {
        let config = trust.client_config(addr, options.datagrams)?;
        let runtime = runtime()?;
        // Quinn spawns its drivers on the current runtime.
        let _rt = runtime.enter();
        let endpoint = client_endpoint(addr)?;
        // rustls wants a DNS name or an IP; anything else falls back to the IP.
        let name = if rustls::pki_types::ServerName::try_from(server_name).is_ok() {
            server_name.to_owned()
        } else {
            addr.ip().to_string()
        };
        let connecting = endpoint
            .connect_with(config, addr, &name)
            .map_err(|e| format!("cannot connect to {addr}: {e}"))?;
        drop(_rt);
        Ok(Self::start(
            endpoint,
            runtime,
            options.datagrams,
            async move {
                connecting
                    .await
                    .map(PeerConn::Quic)
                    .map_err(|e| describe(&e, true))
            },
        ))
    }

    /// Connects over WebTransport, as the browser app does (spec §14
    /// Phase 10 B3): `url` is `https://HOST[:PORT][/ocw/1]` (default port
    /// [`webtransport::DEFAULT_PORT`]); `trust` pins the certificate by its
    /// SHA-256 – the browsers' `serverCertificateHashes`.
    pub fn connect_web(
        url: &str,
        trust: &ServerTrust,
        options: ClientOptions,
    ) -> Result<Self, String> {
        let (url, addr) = webtransport::resolve_url(url)?;
        let config = trust.client_config_for(addr, options.datagrams, webtransport::ALPN)?;
        let runtime = runtime()?;
        let _rt = runtime.enter();
        let endpoint = client_endpoint(addr)?;
        drop(_rt);
        let wt = web_transport_quinn::Client::new(endpoint.clone(), config);
        Ok(Self::start(
            endpoint,
            runtime,
            options.datagrams,
            async move {
                wt.connect(url)
                    .await
                    .map(|s| PeerConn::Web(Box::new(s)))
                    .map_err(|e| format!("cannot connect: {e}"))
            },
        ))
    }

    /// The link task: waits for `connected`, then runs the connection.
    fn start(
        endpoint: Endpoint,
        runtime: tokio::runtime::Runtime,
        datagrams: bool,
        connected: impl Future<Output = Result<PeerConn, String>> + Send + 'static,
    ) -> Self {
        let (to_server, out_rx) = tmpsc::unbounded_channel();
        let (in_tx, from_server) = mpsc::channel();
        let shared = Arc::new(Mutex::new(Shared {
            status: LinkStatus::Connecting,
            conn: None,
        }));
        let s = shared.clone();
        runtime.spawn(async move {
            let reason = match connected.await {
                Ok(conn) => run(conn, datagrams, out_rx, in_tx, &s).await,
                Err(e) => e,
            };
            tracing::warn!(%reason, "disconnected from server");
            s.lock().expect("not poisoned").status = LinkStatus::Closed(reason);
        });
        Self {
            to_server,
            from_server,
            shared,
            endpoint,
            runtime: Some(runtime),
        }
    }

    /// Queues an encoded `ClientMessage`. Dropped once the link is closed.
    pub fn send(&self, bytes: Vec<u8>) {
        let _ = self.to_server.send(bytes);
    }

    /// The encoded `ServerMessage`s received since the last call.
    pub fn receive(&self) -> Vec<Vec<u8>> {
        self.from_server.try_iter().collect()
    }

    pub fn status(&self) -> LinkStatus {
        self.shared.lock().expect("not poisoned").status.clone()
    }

    /// The transport of the link, once connected.
    pub fn transport(&self) -> Option<super::Transport> {
        let s = self.shared.lock().expect("not poisoned");
        s.conn.as_ref().map(PeerConn::transport)
    }

    /// Smoothed round-trip time, once connected.
    pub fn rtt(&self) -> Option<Duration> {
        let s = self.shared.lock().expect("not poisoned");
        s.conn.as_ref().map(|c| c.quic().rtt())
    }

    /// Closes the connection ("client quit") and waits briefly so the
    /// server hears it at once instead of after the idle timeout.
    pub fn close(mut self) {
        self.shut();
    }

    fn shut(&mut self) {
        let Some(rt) = self.runtime.take() else {
            return;
        };
        if let Some(c) = self.shared.lock().expect("not poisoned").conn.take() {
            // In the runtime: a WebTransport session sends its close capsule.
            let _rt = rt.enter();
            c.close(close_code::NORMAL, b"client quit");
        }
        let endpoint = self.endpoint.clone();
        rt.block_on(async move {
            let _ = tokio::time::timeout(Duration::from_millis(300), endpoint.wait_idle()).await;
        });
        rt.shutdown_timeout(Duration::from_millis(200));
    }
}

impl Drop for NetClient {
    fn drop(&mut self) {
        self.shut();
    }
}

/// A connection error in words for the HUD.
fn describe(e: &ConnectionError, handshake: bool) -> String {
    match e {
        ConnectionError::ApplicationClosed(c) => {
            let reason = String::from_utf8_lossy(&c.reason);
            if reason.is_empty() {
                "the server closed the connection".into()
            } else {
                format!("the server closed the connection: {reason}")
            }
        }
        ConnectionError::TimedOut if handshake => "no answer from the server".into(),
        ConnectionError::TimedOut => "connection timed out".into(),
        ConnectionError::LocallyClosed => "connection closed".into(),
        e if handshake => format!("cannot connect: {e}"),
        e => format!("connection lost: {e}"),
    }
}

/// Why a connection ended, in words.
fn lost(conn: &PeerConn, fallback: String) -> String {
    match conn {
        PeerConn::Quic(c) => c.close_reason().map(|e| describe(&e, false)),
        PeerConn::Web(s) => s.close_reason().map(|e| match e {
            web_transport_quinn::SessionError::ConnectionError(e) => describe(&e, false),
            web_transport_quinn::SessionError::WebTransportError(
                web_transport_quinn::WebTransportError::Closed(_, reason),
            ) if !reason.is_empty() => format!("the server closed the connection: {reason}"),
            e => format!("connection lost: {e}"),
        }),
    }
    .unwrap_or(fallback)
}

async fn run(
    conn: PeerConn,
    accept_datagrams: bool,
    out_rx: tmpsc::UnboundedReceiver<Vec<u8>>,
    in_tx: mpsc::Sender<Vec<u8>>,
    shared: &Mutex<Shared>,
) -> String {
    {
        let mut s = shared.lock().expect("not poisoned");
        s.status = LinkStatus::Connected {
            datagrams: accept_datagrams && conn.max_datagram().is_some(),
        };
        s.conn = Some(conn.clone());
    }
    tracing::info!(remote = %conn.remote_address(), transport = conn.transport().name(), "connected to server");
    match &conn {
        PeerConn::Quic(c) => match c.open_bi().await {
            Ok((send, recv)) => pump(&conn, send, recv, out_rx, in_tx).await,
            Err(e) => describe(&e, false),
        },
        PeerConn::Web(s) => match s.open_bi().await {
            Ok((send, recv)) => pump(&conn, send, recv, out_rx, in_tx).await,
            Err(e) => lost(&conn, format!("cannot open the stream: {e}")),
        },
    }
}

async fn pump<S, R>(
    conn: &PeerConn,
    mut send: S,
    mut recv: R,
    mut out_rx: tmpsc::UnboundedReceiver<Vec<u8>>,
    in_tx: mpsc::Sender<Vec<u8>>,
) -> String
where
    S: AsyncWrite + Unpin,
    R: AsyncRead + Unpin,
{
    // Until the server has answered on the stream (Welcome or Reject),
    // everything goes on the stream, so no sample overtakes the Hello.
    let answered = std::sync::atomic::AtomicBool::new(false);
    let reader = async {
        loop {
            match read_frame(&mut recv).await {
                Ok(Some(bytes)) => {
                    answered.store(true, std::sync::atomic::Ordering::Relaxed);
                    if in_tx.send(bytes).is_err() {
                        return "client stopped".to_owned();
                    }
                }
                Ok(None) => return lost(conn, "the server closed the stream".into()),
                Err(e) => return lost(conn, format!("bad frame from the server: {e}")),
            }
        }
    };
    let datagrams = async {
        loop {
            match conn.read_datagram().await {
                Ok(d) => {
                    if in_tx.send(d.to_vec()).is_err() {
                        return "client stopped".to_owned();
                    }
                }
                Err(e) => return lost(conn, e),
            }
        }
    };
    let writer = async {
        while let Some(bytes) = out_rx.recv().await {
            if answered.load(std::sync::atomic::Ordering::Relaxed)
                && client_unreliable(&bytes)
                && conn.try_datagram(&bytes)
            {
                continue;
            }
            if let Err(e) = write_frame(&mut send, &bytes).await {
                return lost(conn, e.to_string());
            }
        }
        // The NetClient was dropped.
        "connection closed".to_owned()
    };
    tokio::select! {
        r = reader => r,
        r = datagrams => r,
        r = writer => r,
    }
}
