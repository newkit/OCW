//! The client side: one QUIC connection to a game server, behind the same
//! "encoded messages in, encoded messages out" interface as the in-process
//! local server, so `ClientCore` runs unchanged on either.

use std::net::SocketAddr;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use quinn::{Connection, ConnectionError, Endpoint, VarInt};
use tokio::sync::mpsc as tmpsc;

use super::framing::{read_frame, write_frame};
use super::{ServerTrust, client_unreliable, close_code};

/// State of the link, for the HUD and `net.stats`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkStatus {
    Connecting,
    /// `datagrams`: datagrams work both ways (else what they would carry
    /// goes on the stream).
    Connected {
        datagrams: bool,
    },
    /// The connection ended (or never came up); why, for the user.
    Closed(String),
}

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
    conn: Option<Connection>,
}

/// A connection to a remote game server.
pub struct NetClient {
    to_server: tmpsc::UnboundedSender<Vec<u8>>,
    from_server: mpsc::Receiver<Vec<u8>>,
    shared: Arc<Mutex<Shared>>,
    endpoint: Endpoint,
    runtime: Option<tokio::runtime::Runtime>,
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
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("ocw-net-client")
            .enable_all()
            .build()
            .map_err(|e| format!("network runtime: {e}"))?;
        let bind: SocketAddr = if addr.is_ipv6() {
            "[::]:0".parse().expect("valid")
        } else {
            "0.0.0.0:0".parse().expect("valid")
        };
        // Quinn spawns its drivers on the current runtime.
        let _rt = runtime.enter();
        let endpoint =
            Endpoint::client(bind).map_err(|e| format!("cannot open a UDP socket: {e}"))?;
        // rustls wants a DNS name or an IP; anything else falls back to the IP.
        let name = if rustls::pki_types::ServerName::try_from(server_name).is_ok() {
            server_name.to_owned()
        } else {
            addr.ip().to_string()
        };
        let connecting = endpoint
            .connect_with(config, addr, &name)
            .map_err(|e| format!("cannot connect to {addr}: {e}"))?;
        let (to_server, out_rx) = tmpsc::unbounded_channel();
        let (in_tx, from_server) = mpsc::channel();
        let shared = Arc::new(Mutex::new(Shared {
            status: LinkStatus::Connecting,
            conn: None,
        }));
        let s = shared.clone();
        tokio::spawn(async move {
            let reason = match connecting.await {
                Ok(conn) => run(conn, options.datagrams, out_rx, in_tx, &s).await,
                Err(e) => describe(&e, true),
            };
            tracing::warn!(%reason, "disconnected from server");
            s.lock().expect("not poisoned").status = LinkStatus::Closed(reason);
        });
        drop(_rt);
        Ok(Self {
            to_server,
            from_server,
            shared,
            endpoint,
            runtime: Some(runtime),
        })
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

    /// Smoothed round-trip time, once connected.
    pub fn rtt(&self) -> Option<Duration> {
        let s = self.shared.lock().expect("not poisoned");
        s.conn.as_ref().map(Connection::rtt)
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
            c.close(VarInt::from_u32(close_code::NORMAL), b"client quit");
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

async fn run(
    conn: Connection,
    accept_datagrams: bool,
    mut out_rx: tmpsc::UnboundedReceiver<Vec<u8>>,
    in_tx: mpsc::Sender<Vec<u8>>,
    shared: &Mutex<Shared>,
) -> String {
    {
        let mut s = shared.lock().expect("not poisoned");
        s.status = LinkStatus::Connected {
            datagrams: accept_datagrams && conn.max_datagram_size().is_some(),
        };
        s.conn = Some(conn.clone());
    }
    tracing::info!(remote = %conn.remote_address(), "connected to server");
    let (mut send, mut recv) = match conn.open_bi().await {
        Ok(s) => s,
        Err(e) => return describe(&e, false),
    };
    // Until the server has answered on the stream (Welcome or Reject),
    // everything goes on the stream, so no sample overtakes the Hello.
    let answered = std::sync::atomic::AtomicBool::new(false);
    let lost = |conn: &Connection, fallback: String| {
        conn.close_reason()
            .map(|e| describe(&e, false))
            .unwrap_or(fallback)
    };
    let reader = async {
        loop {
            match read_frame(&mut recv).await {
                Ok(Some(bytes)) => {
                    answered.store(true, std::sync::atomic::Ordering::Relaxed);
                    if in_tx.send(bytes).is_err() {
                        return "client stopped".to_owned();
                    }
                }
                Ok(None) => return lost(&conn, "the server closed the stream".into()),
                Err(e) => return lost(&conn, format!("bad frame from the server: {e}")),
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
                Err(e) => return describe(&e, false),
            }
        }
    };
    let writer = async {
        while let Some(bytes) = out_rx.recv().await {
            if answered.load(std::sync::atomic::Ordering::Relaxed)
                && client_unreliable(&bytes)
                && conn
                    .max_datagram_size()
                    .is_some_and(|max| bytes.len() <= max)
                && conn.send_datagram(Bytes::from(bytes.clone())).is_ok()
            {
                continue;
            }
            if let Err(e) = write_frame(&mut send, &bytes).await {
                return lost(&conn, e.to_string());
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
