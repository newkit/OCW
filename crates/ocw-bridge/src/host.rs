//! The desktop's bridge endpoint (native only; spec 14 Phase 10, 10D.2
//! *Built (local bridge)*): WebTransport (HTTP/3 over QUIC, ALPN `h3`) on
//! one of the bridge's UDP ports, with the certificate derived from the
//! pairing secret, on a thread of its own (a Tokio runtime, like the game
//! server's network thread).
//!
//! The app talks to it without async code: [`BridgeHost::relay`] queues a
//! device reading for the paired browser (a channel send, no waiting), and
//! [`BridgeHost::events`] tells it who paired or left and the trainer
//! control the browser sends; [`BridgeHost::applied`] confirms a control
//! once it went to the trainer.
//!
//! Only peers on the local network are taken (loopback, the private IPv4
//! ranges, link-local, IPv6 unique-local); one browser at a time
//! ([`crate::relay::Gate`]).

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ocw_devices::{DeviceCommand, DeviceEvent};
use quinn::crypto::rustls::QuicServerConfig;
use tokio::sync::mpsc as tmpsc;

use crate::cert::BridgeCert;
use crate::code::{PORT_SLOTS, PairingCode, port};
use crate::msg::{DeviceInfo, DeviceState, Frames, ToBrowser, ToHost, VERSION, frame};
use crate::relay::{Admit, Gate, Relay};

/// What the app hears from the bridge.
#[derive(Debug, Clone, PartialEq)]
pub enum HostEvent {
    /// A browser paired (what it says it is).
    Paired { client: String },
    /// The paired browser left (why).
    Unpaired { reason: String },
    /// A browser was turned away (why: a wrong code, another browser, not
    /// on the local network).
    Refused { why: String },
    /// Trainer control from the paired browser: give it to the trainer,
    /// then [`BridgeHost::applied`]`(id)`.
    Control { id: u64, command: DeviceCommand },
    /// A report of the paired browser to keep (14I W4: the headset run's
    /// numbers), at most [`crate::msg::MAX_REPORT`] bytes.
    Report {
        kind: String,
        data: serde_json::Value,
    },
}

/// Whether an address is on the local network.
pub fn is_local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_local(IpAddr::V4(v4));
            }
            let seg = v6.segments()[0];
            v6.is_loopback() || (seg & 0xFE00) == 0xFC00 || (seg & 0xFFC0) == 0xFE80
        }
    }
}

/// The address the code names: this machine's IPv4 address on the local
/// network (the one the system would use towards the internet; nothing is
/// sent), else loopback.
pub fn lan_ipv4() -> Ipv4Addr {
    let probe = || -> Option<Ipv4Addr> {
        let s = UdpSocket::bind("0.0.0.0:0").ok()?;
        // A documentation address (RFC 5737): routed like any other.
        s.connect("192.0.2.1:9").ok()?;
        match s.local_addr().ok()?.ip() {
            IpAddr::V4(v4) if !v4.is_unspecified() && !v4.is_loopback() => Some(v4),
            _ => None,
        }
    };
    probe().unwrap_or(Ipv4Addr::LOCALHOST)
}

/// A new pairing secret (OS randomness).
pub fn new_secret() -> u32 {
    let mut b = [0u8; 4];
    getrandom::fill(&mut b).expect("OS randomness");
    u32::from_le_bytes(b) & ((1 << crate::code::SECRET_BITS) - 1)
}

/// How the bridge starts.
#[derive(Debug, Clone)]
pub struct HostOptions {
    pub secret: u32,
    /// The address the code names ([`lan_ipv4`]).
    pub ip: Ipv4Addr,
    /// The bind address (all interfaces; loopback in tests).
    pub bind: IpAddr,
    /// The desktop's name for the browser.
    pub name: String,
    /// Unix time now (the certificate's week).
    pub now_unix_s: i64,
}

/// The environment variable that sets the address the code names (a
/// computer on several networks; tests and screenshots, which use
/// 127.0.0.1 so no real address shows).
pub const IP_ENV: &str = "OCW_BRIDGE_IP";

impl HostOptions {
    pub fn new(secret: u32, name: &str, now_unix_s: i64) -> Self {
        let ip = std::env::var(IP_ENV)
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or_else(lan_ipv4);
        Self {
            secret,
            ip,
            bind: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            name: name.into(),
            now_unix_s,
        }
    }
}

/// Counters for the app's status (and the tests).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HostStats {
    pub relayed: u64,
    pub controls: u64,
    pub refused: u64,
    /// The paired browser's last round trip (ms), from its pings.
    pub pings: u64,
}

#[derive(Default)]
struct Shared {
    gate: Gate,
    relay: Relay,
    /// The connections' outboxes (only the paired one gets readings).
    outs: HashMap<u64, tmpsc::UnboundedSender<ToBrowser>>,
    sessions: HashMap<u64, web_transport_quinn::Session>,
    /// The desktop's devices as last reported (for the welcome).
    devices: Vec<DeviceInfo>,
    stats: HostStats,
    closing: bool,
    epoch: u64,
}

type SharedRef = Arc<Mutex<Shared>>;

fn lock(s: &SharedRef) -> std::sync::MutexGuard<'_, Shared> {
    s.lock().expect("not poisoned")
}

/// The running bridge.
pub struct BridgeHost {
    pub code: PairingCode,
    pub addr: SocketAddr,
    shared: SharedRef,
    events: Mutex<Receiver<HostEvent>>,
    endpoint: Arc<Mutex<Option<quinn::Endpoint>>>,
    thread: Option<std::thread::JoinHandle<()>>,
    start: Instant,
}

/// The session's path (the protocol version).
pub use crate::PATH;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

fn server_config(cert: &BridgeCert) -> Result<quinn::ServerConfig, String> {
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut tls = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| e.to_string())?
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(cert.der.clone())],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(cert.key_pkcs8.clone())),
        )
        .map_err(|e| format!("the bridge's certificate: {e}"))?;
    tls.alpn_protocols = vec![b"h3".to_vec()];
    tls.max_early_data_size = 0;
    let crypto = QuicServerConfig::try_from(tls).map_err(|e| e.to_string())?;
    let mut cfg = quinn::ServerConfig::with_crypto(Arc::new(crypto));
    let mut t = quinn::TransportConfig::default();
    // A browser that vanished (tab closed, headset off) is noticed in 10 s;
    // its pings come every second anyway.
    t.max_idle_timeout(Some(
        quinn::IdleTimeout::try_from(Duration::from_secs(10)).expect("valid"),
    ));
    t.keep_alive_interval(Some(Duration::from_secs(2)));
    cfg.transport_config(Arc::new(t));
    Ok(cfg)
}

impl BridgeHost {
    /// Binds the first free bridge port and starts the thread.
    pub fn start(opts: HostOptions) -> Result<Self, String> {
        let cert = BridgeCert::current(opts.secret, opts.now_unix_s);
        let config = server_config(&cert)?;
        let (socket, slot) = (0..PORT_SLOTS as u8)
            .find_map(|slot| {
                UdpSocket::bind(SocketAddr::new(opts.bind, port(slot)))
                    .ok()
                    .map(|s| (s, slot))
            })
            .ok_or_else(|| {
                format!(
                    "the bridge's UDP ports {} – {} are all in use",
                    port(0),
                    port(PORT_SLOTS as u8 - 1)
                )
            })?;
        let addr = socket.local_addr().map_err(|e| e.to_string())?;
        let code = PairingCode::new(opts.ip, slot, opts.secret);
        let shared: SharedRef = Arc::new(Mutex::new(Shared {
            gate: Gate::new(opts.secret),
            epoch: epoch_now(),
            ..Default::default()
        }));
        let (events_tx, events) = mpsc::channel();
        let endpoint = Arc::new(Mutex::new(None));
        let start = Instant::now();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let thread = {
            let shared = shared.clone();
            let endpoint = endpoint.clone();
            let name = opts.name.clone();
            std::thread::Builder::new()
                .name("ocw-bridge".into())
                .spawn(move || {
                    let rt = match tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                    {
                        Ok(rt) => rt,
                        Err(e) => {
                            let _ = ready_tx.send(Err(e.to_string()));
                            return;
                        }
                    };
                    rt.block_on(async move {
                        let ep = match quinn::Endpoint::new(
                            quinn::EndpointConfig::default(),
                            Some(config),
                            socket,
                            Arc::new(quinn::TokioRuntime),
                        ) {
                            Ok(ep) => ep,
                            Err(e) => {
                                let _ = ready_tx.send(Err(e.to_string()));
                                return;
                            }
                        };
                        *endpoint.lock().expect("not poisoned") = Some(ep.clone());
                        let _ = ready_tx.send(Ok(()));
                        accept_loop(ep, shared, events_tx, name, start).await;
                    });
                })
                .map_err(|e| e.to_string())?
        };
        ready_rx
            .recv()
            .map_err(|_| "the bridge thread ended".to_owned())??;
        tracing::info!(%code, %addr, "sensor bridge: sharing with a browser");
        Ok(Self {
            code,
            addr,
            shared,
            events: Mutex::new(events),
            endpoint,
            thread: Some(thread),
            start,
        })
    }

    /// The bridge's clock (ms since it started): the readings' times.
    pub fn now_ms(&self) -> f64 {
        self.start.elapsed().as_secs_f64() * 1000.0
    }

    /// A device event of the desktop for the paired browser (readings),
    /// and the device list (connects, drops). `virt`: the virtual trainer.
    pub fn relay(&self, device: &str, event: &DeviceEvent, virt: bool) {
        let h = self.now_ms();
        let mut s = lock(&self.shared);
        let state = match event {
            DeviceEvent::Connected { .. } => Some(DeviceState::Connected),
            DeviceEvent::Disconnected => Some(DeviceState::Reconnecting),
            DeviceEvent::Failed { .. } => Some(DeviceState::Failed),
            _ => None,
        };
        if let Some(state) = state {
            match s.devices.iter_mut().find(|d| d.name == device) {
                Some(d) => d.state = state,
                None => s.devices.push(DeviceInfo {
                    name: device.into(),
                    state,
                }),
            }
        }
        let Some(conn) = s.gate.paired().map(|p| p.conn) else {
            return;
        };
        let Some(msg) = s.relay.message(device, event, h, virt) else {
            return;
        };
        if matches!(msg, ToBrowser::Power { .. } | ToBrowser::HeartRate { .. }) {
            s.stats.relayed += 1;
        }
        if let Some(out) = s.outs.get(&conn) {
            let _ = out.send(msg);
        }
    }

    /// A device the desktop stopped (forgotten): off the list.
    pub fn device_gone(&self, device: &str) {
        lock(&self.shared).devices.retain(|d| d.name != device);
    }

    /// A control went to the trainer.
    pub fn applied(&self, id: u64) {
        let s = lock(&self.shared);
        if let Some(conn) = s.gate.paired().map(|p| p.conn)
            && let Some(out) = s.outs.get(&conn)
        {
            let _ = out.send(ToBrowser::Applied { id });
        }
    }

    /// Events since the last call.
    pub fn events(&self) -> Vec<HostEvent> {
        self.events
            .lock()
            .expect("not poisoned")
            .try_iter()
            .collect()
    }

    /// The paired browser (what it says it is).
    pub fn paired(&self) -> Option<String> {
        lock(&self.shared).gate.paired().map(|p| p.client.clone())
    }

    pub fn stats(&self) -> HostStats {
        lock(&self.shared).stats.clone()
    }

    /// Stops sharing: the browser hears why, the port is freed.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        {
            let mut s = lock(&self.shared);
            s.closing = true;
            for out in s.outs.values() {
                let _ = out.send(ToBrowser::Closed {
                    why: "the desktop app stopped sharing its trainer".into(),
                });
            }
        }
        // Give the writers a moment to send the reason.
        std::thread::sleep(Duration::from_millis(100));
        if let Some(ep) = self.endpoint.lock().expect("not poisoned").take() {
            ep.close(quinn::VarInt::from_u32(0), b"stopped");
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for BridgeHost {
    fn drop(&mut self) {
        if self.thread.is_some() {
            self.shutdown();
        }
    }
}

async fn accept_loop(
    ep: quinn::Endpoint,
    shared: SharedRef,
    events: Sender<HostEvent>,
    name: String,
    start: Instant,
) {
    while let Some(incoming) = ep.accept().await {
        let remote = incoming.remote_address();
        if !is_local(remote.ip()) {
            incoming.refuse();
            lock(&shared).stats.refused += 1;
            let _ = events.send(HostEvent::Refused {
                why: format!(
                    "a connection from outside the local network ({})",
                    remote.ip()
                ),
            });
            continue;
        }
        let shared = shared.clone();
        let events = events.clone();
        let name = name.clone();
        tokio::spawn(async move {
            if let Err(e) = serve(incoming, shared, events, name, start).await {
                tracing::info!(%remote, %e, "sensor bridge: connection ended");
            }
        });
    }
    // Wait for the connections to close.
    ep.wait_idle().await;
}

fn now_ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

/// One browser connection: the WebTransport handshake, the hello, then the
/// readings out and pings / controls in.
async fn serve(
    incoming: quinn::Incoming,
    shared: SharedRef,
    events: Sender<HostEvent>,
    name: String,
    start: Instant,
) -> Result<(), String> {
    let conn = incoming.await.map_err(|e| e.to_string())?;
    let request = tokio::time::timeout(
        HANDSHAKE_TIMEOUT,
        web_transport_quinn::Request::accept(conn.clone()),
    )
    .await
    .map_err(|_| "no WebTransport request in time".to_owned())?
    .map_err(|e| e.to_string())?;
    let path = request.url.path().to_owned();
    if path != PATH {
        let _ = request
            .reject(web_transport_quinn::http::StatusCode::NOT_FOUND)
            .await;
        return Err(format!("unknown path {path}"));
    }
    let session = request.ok().await.map_err(|e| e.to_string())?;
    let (mut send, mut recv) = tokio::time::timeout(HANDSHAKE_TIMEOUT, session.accept_bi())
        .await
        .map_err(|_| "no stream in time".to_owned())?
        .map_err(|e| e.to_string())?;
    let mut frames = Frames::default();
    let mut buf = vec![0u8; 4096];
    // The hello.
    let hello: ToHost = loop {
        if let Some(m) = frames.next_message::<ToHost>() {
            break m.map_err(|e| e.to_string())?;
        }
        let n = tokio::time::timeout(HANDSHAKE_TIMEOUT, recv.read(&mut buf))
            .await
            .map_err(|_| "no hello in time".to_owned())?
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "the stream ended before the hello".to_owned())?;
        frames.push(&buf[..n]);
    };
    let client = match &hello {
        ToHost::Hello { client, .. } => client.clone(),
        _ => String::new(),
    };
    let (tx, mut rx) = tmpsc::unbounded_channel::<ToBrowser>();
    let admitted = {
        let mut s = lock(&shared);
        if s.closing {
            return Err("stopping".into());
        }
        match s.gate.hello(&hello, now_ms(start)) {
            Admit::Refused(why) => {
                s.stats.refused += 1;
                Err(why)
            }
            Admit::Paired { conn, replaces } => {
                if let Some(old) = replaces {
                    if let Some(out) = s.outs.remove(&old) {
                        let _ = out.send(ToBrowser::Closed {
                            why: "a newer connection of this page took over".into(),
                        });
                    }
                    if let Some(sess) = s.sessions.remove(&old) {
                        sess.close(2, b"replaced");
                    }
                }
                s.outs.insert(conn, tx.clone());
                s.sessions.insert(conn, session.clone());
                Ok((
                    conn,
                    ToBrowser::Welcome {
                        v: VERSION,
                        host: name.clone(),
                        epoch: s.epoch,
                        devices: s.devices.clone(),
                    },
                ))
            }
        }
    };
    let (conn_id, welcome) = match admitted {
        Ok(a) => a,
        Err(why) => {
            let _ = send
                .write_all(&frame(&ToBrowser::Refused { why: why.clone() }))
                .await;
            let _ = send.finish();
            let _ = events.send(HostEvent::Refused {
                why: format!("{client}: {why}"),
            });
            tokio::time::sleep(Duration::from_millis(200)).await;
            session.close(1, why.as_bytes());
            return Ok(());
        }
    };
    let _ = events.send(HostEvent::Paired {
        client: client.clone(),
    });
    let _ = tx.send(welcome);
    // The writer: everything for this connection, in order.
    let writer = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let closed = matches!(msg, ToBrowser::Closed { .. });
            if send.write_all(&frame(&msg)).await.is_err() {
                break;
            }
            if closed {
                let _ = send.finish();
                break;
            }
        }
    });
    // The reader: pings and controls.
    let reason = 'conn: loop {
        while let Some(m) = frames.next_message::<ToHost>() {
            let m = match m {
                Ok(m) => m,
                Err(e) => break 'conn e.to_string(),
            };
            let now = now_ms(start);
            let mut s = lock(&shared);
            s.gate.heard(conn_id, now);
            match m {
                ToHost::Ping { id, c } => {
                    s.stats.pings += 1;
                    let _ = tx.send(ToBrowser::Pong { id, c, h: now });
                }
                ToHost::Control { id, control } => {
                    s.stats.controls += 1;
                    if s.gate.is_paired(conn_id) {
                        let _ = events.send(HostEvent::Control {
                            id,
                            command: control.command(),
                        });
                    }
                }
                ToHost::Report { kind, data } => {
                    let small = data.to_string().len() <= crate::msg::MAX_REPORT;
                    if s.gate.is_paired(conn_id) && small {
                        let _ = events.send(HostEvent::Report { kind, data });
                    }
                }
                ToHost::Bye => break 'conn "the browser stopped the bridge".to_owned(),
                ToHost::Hello { .. } => {}
            }
        }
        match recv.read(&mut buf).await {
            Ok(Some(n)) => frames.push(&buf[..n]),
            Ok(None) => break "the browser closed the stream".to_owned(),
            Err(e) => break format!("connection lost: {e}"),
        }
    };
    let was_paired = {
        let mut s = lock(&shared);
        s.outs.remove(&conn_id);
        s.sessions.remove(&conn_id);
        s.gate.ended(conn_id)
    };
    drop(tx);
    let _ = writer.await;
    session.close(0, b"");
    if was_paired {
        let _ = events.send(HostEvent::Unpaired { reason });
    }
    Ok(())
}

/// This run of the bridge (the browser restarts its count of readings when
/// it changes): the start time in ms since the Unix epoch.
fn epoch_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
