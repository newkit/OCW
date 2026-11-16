//! What a hosting runs beside its game server (spec §7.1.6 V2): the
//! router's port mappings ([`crate::portmap`], §7.1.7 Q4) and the bundled
//! voice server ([`crate::voice::launcher`], §7.1.1 (a)). One code path for
//! both hosts: the `ocw-server` binary (`--listen`) and the desktop app's
//! *Ride → Host a ride* (`LocalServer::spawn_hosting`).
//!
//! [`HostServices::spawn`] starts a thread of their own that lives exactly
//! as long as the hosting:
//!
//! 1. *Router ports* (when switched on): finds the router, maps the
//!    hosting's ports ([`portmap::host_ports`]) and reads its external
//!    address. Never fatal – the state says what happened.
//! 2. *Voice*: starts `livekit-server` with fresh keys ([`start_voice`]).
//!    Its media address is the public host given (`--voice-public-host`,
//!    the join link's host), else the router's public address from step
//!    1, else STUN (§7.1.7 Q5); loopback for a loopback hosting. A voice
//!    server that does not start leaves the hosting without voice (and its
//!    mapped voice ports are removed again), saying why.
//! 3. *Ready* ([`HostServices::wait_ready`], [`HostServices::is_ready`]):
//!    the game server takes [`HostServices::voice_config`] now.
//! 4. Then once a second: the voice server is watched (restarted after a
//!    crash, [`VoiceServer::poll`]) and the leases renewed.
//! 5. *Stop* ([`HostServices::stop`], also on drop): the voice server is
//!    stopped and the mappings are removed, then the thread ends.
//!
//! **No orphan:** the voice server is started by this thread, which ends
//! only after stopping it; Linux's parent-death signal (bound to the
//! starting thread) and Windows' job object end it when the host process
//! dies (`launcher` module docs).

use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::portmap::{self, Discover, HostPort, MapState, PortMapping};
use crate::voice::VoiceConfig;
use crate::voice::launcher::{LaunchOptions, RtcAddress, VoiceEvent, VoicePorts, VoiceServer};

/// The bundled voice server to start.
pub struct VoicePlan {
    /// The binary, ports, state dir (and the readiness probe); its
    /// `bind` and `rtc` are chosen by [`start_voice`].
    pub launch: LaunchOptions,
    /// The host riders from the internet use (the join link's host,
    /// `--voice-public-host`), if given.
    pub public_host: Option<String>,
}

/// What runs beside a hosting.
pub struct HostingOptions {
    /// The game server's QUIC address (UDP).
    pub listen: SocketAddr,
    /// WebTransport's UDP port for browsers, if open.
    pub web_udp: Option<u16>,
    /// The bundled voice server; `None`: none (`voice_off` says why).
    pub voice: Option<VoicePlan>,
    /// Why there is no bundled voice server (an external LiveKit, `--voice
    /// off`, no binary).
    pub voice_off: String,
    /// The router to map ports on; `None`: switched off.
    pub upnp: Option<Discover>,
    pub lease_s: u32,
}

impl HostingOptions {
    /// Nothing beside the game server on `listen` (no voice, no mapping).
    pub fn new(listen: SocketAddr) -> Self {
        Self {
            listen,
            web_udp: None,
            voice: None,
            voice_off: "off".into(),
            upnp: None,
            lease_s: portmap::LEASE_S,
        }
    }
}

/// The voice server's state, for the hosting's pages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoiceState {
    /// Not started: why (an external LiveKit, switched off, no binary).
    Off(String),
    Starting,
    /// Running on these ports.
    On(VoicePorts),
    /// Did not start: why. The hosting runs without voice.
    Failed(String),
}

impl Default for VoiceState {
    fn default() -> Self {
        Self::Off("off".into())
    }
}

impl VoiceState {
    /// One line for people.
    pub fn summary(&self) -> String {
        match self {
            Self::Off(why) => format!("voice: no voice server of its own ({why})"),
            Self::Starting => "voice: starting the voice server …".into(),
            Self::On(p) => format!("voice: on ({p})"),
            Self::Failed(why) => format!("voice chat off: {why}"),
        }
    }
}

/// What the hosting's services did so far.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostStatus {
    /// Steps 1–2 are done (the game server can start).
    pub ready: bool,
    pub voice: VoiceState,
    pub router: MapState,
    /// The hosting's public address: the voice server's media address
    /// and the join link's host for riders from the internet (the given
    /// public host's, else the router's public one).
    pub public_ip: Option<IpAddr>,
    /// The ports others reach (game, voice): shown with the hint.
    pub ports: Vec<HostPort>,
}

struct Shared {
    status: Mutex<HostStatus>,
    changed: Condvar,
    voice: Mutex<Option<VoiceConfig>>,
}

/// The services of one hosting, on their own thread; stopped by
/// [`HostServices::stop`] or when dropped.
pub struct HostServices {
    shared: Arc<Shared>,
    stop: Option<Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for HostServices {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostServices")
            .field("status", &self.status())
            .finish_non_exhaustive()
    }
}

impl HostServices {
    /// Starts the thread (see the module docs); returns at once.
    pub fn spawn(opts: HostingOptions) -> std::io::Result<Self> {
        let voice_ports = opts.voice.as_ref().map(|v| v.launch.ports);
        let ports = portmap::host_ports(opts.listen.port(), opts.web_udp, voice_ports);
        let shared = Arc::new(Shared {
            status: Mutex::new(HostStatus {
                ready: false,
                voice: if opts.voice.is_some() {
                    VoiceState::Starting
                } else {
                    VoiceState::Off(opts.voice_off.clone())
                },
                router: if opts.upnp.is_some() {
                    MapState::Searching
                } else {
                    MapState::Off
                },
                public_ip: None,
                ports: ports.clone(),
            }),
            changed: Condvar::new(),
            voice: Mutex::new(None),
        });
        let (stop, stop_rx) = mpsc::channel::<()>();
        let s = shared.clone();
        let thread = std::thread::Builder::new()
            .name("ocw-host-services".into())
            .spawn(move || run(opts, ports, &s, &stop_rx))?;
        Ok(Self {
            shared,
            stop: Some(stop),
            thread: Some(thread),
        })
    }

    /// What the services did so far.
    pub fn status(&self) -> HostStatus {
        lock(&self.shared.status).clone()
    }

    /// Router ports and voice server are done (started or failed).
    pub fn is_ready(&self) -> bool {
        lock(&self.shared.status).ready
    }

    /// Waits until [`HostServices::is_ready`], at most `within`.
    pub fn wait_ready(&self, within: Duration) -> bool {
        let end = Instant::now() + within;
        let mut s = lock(&self.shared.status);
        while !s.ready {
            let now = Instant::now();
            if now >= end {
                return false;
            }
            s = self
                .shared
                .changed
                .wait_timeout(s, end - now)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
        true
    }

    /// The game server's voice configuration (the bundled voice server's
    /// keys and reach), once it runs.
    pub fn voice_config(&self) -> Option<VoiceConfig> {
        lock(&self.shared.voice).clone()
    }

    /// Stops the voice server and removes the mappings; waits for it.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        drop(self.stop.take());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for HostServices {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn update(shared: &Shared, f: impl FnOnce(&mut HostStatus)) {
    f(&mut lock(&shared.status));
    shared.changed.notify_all();
}

fn run(opts: HostingOptions, ports: Vec<HostPort>, shared: &Shared, stop: &mpsc::Receiver<()>) {
    // 1. The router's ports.
    let mut mapping = None;
    let mut router_ip = None;
    if let Some(discover) = opts.upnp {
        let (m, state) = PortMapping::open(discover, &ports, opts.lease_s);
        match &state {
            MapState::Mapped { .. } => tracing::info!("{}", state.summary()),
            _ => tracing::warn!("{}", state.summary()),
        }
        router_ip = state.public_ip();
        mapping = m;
        update(shared, |s| s.router = state);
    }
    // 2. The voice server.
    let mut voice = None;
    let mut public_ip = router_ip;
    if let Some(plan) = opts.voice {
        match start_voice(plan, opts.listen.ip(), router_ip) {
            Ok((server, config, ip)) => {
                tracing::info!(
                    url = %config.url,
                    room = %config.session_room(),
                    ports = %server.ports(),
                    pid = server.pid().unwrap_or_default(),
                    "voice chat on (bundled LiveKit {})",
                    crate::voice::launcher::PINNED_VERSION
                );
                public_ip = ip;
                let p = server.ports();
                *lock(&shared.voice) = Some(config);
                voice = Some(server);
                update(shared, |s| s.voice = VoiceState::On(p));
            }
            Err(e) => {
                tracing::warn!("voice chat off: {e}");
                // Its ports need not stay open on the router.
                if let Some(m) = &mut mapping {
                    for (p, why) in m.remove_where(|p| p.what.starts_with("voice")) {
                        tracing::warn!(port = %p, %why, "a router port mapping could not be removed");
                    }
                }
                update(shared, |s| s.voice = VoiceState::Failed(e));
            }
        }
    }
    // 3. Ready.
    update(shared, |s| {
        s.public_ip = public_ip;
        s.ready = true;
    });
    // 4. Watch until stopped (a message or the sender dropped).
    while let Err(RecvTimeoutError::Timeout) = stop.recv_timeout(Duration::from_secs(1)) {
        if let Some(v) = &mut voice {
            log_voice_event(v.poll());
        }
        if let Some(m) = &mut mapping {
            for (p, why) in m.renew_if_due() {
                tracing::warn!(port = %p, %why, "a router port mapping could not be renewed");
            }
        }
    }
    // 5. Stop: the voice server, then the mappings.
    drop(voice);
    *lock(&shared.voice) = None;
    drop(mapping);
    update(shared, |s| {
        if matches!(s.voice, VoiceState::On(_) | VoiceState::Starting) {
            s.voice = VoiceState::Off("stopped".into());
        }
        if matches!(s.router, MapState::Mapped { .. } | MapState::Searching) {
            s.router = MapState::Off;
        }
    });
    tracing::info!("hosting services stopped (voice server, router ports)");
}

/// Logs what [`VoiceServer::poll`] noticed.
pub fn log_voice_event(event: Option<VoiceEvent>) {
    match event {
        Some(VoiceEvent::Crashed { status, retry_in }) => tracing::warn!(
            %status,
            retry_in_s = retry_in.as_secs_f32(),
            "the voice server ended; starting it again"
        ),
        Some(VoiceEvent::Restarted) => tracing::info!("the voice server is back"),
        Some(VoiceEvent::RestartFailed { error, retry_in }) => tracing::warn!(
            %error,
            retry_in_s = retry_in.as_secs_f32(),
            "the voice server could not be started again"
        ),
        None => {}
    }
}

/// The address of `host` (a name or an address).
pub fn resolve_host(host: &str) -> Result<IpAddr, String> {
    Ok((host, 0)
        .to_socket_addrs()
        .map_err(|e| format!("{host}: {e}"))?
        .next()
        .ok_or_else(|| format!("{host}: no address"))?
        .ip())
}

/// Starts the bundled voice server for a game server listening on
/// `listen` (spec §7.1.1, §7.1.7 Q5): media on the given public host's
/// address, else on the router's public address (`router_ip`), else STUN;
/// a loopback game server keeps it on this machine. Returns the server,
/// the game server's voice configuration (each grant names the address
/// that rider reaches) and the public address used.
pub fn start_voice(
    plan: VoicePlan,
    listen: IpAddr,
    router_ip: Option<IpAddr>,
) -> Result<(VoiceServer, VoiceConfig, Option<IpAddr>), String> {
    let given = match &plan.public_host {
        Some(host) => Some(resolve_host(host).map_err(|e| format!("--voice-public-host {e}"))?),
        None => None,
    };
    let public_ip = given.or(router_ip);
    let public_host = plan
        .public_host
        .clone()
        .or_else(|| router_ip.map(|ip| ip.to_string()));
    let opts = LaunchOptions {
        bind: listen.is_loopback().then_some(listen),
        rtc: RtcAddress::choose(listen, public_ip),
        ..plan.launch
    };
    let server = VoiceServer::start(opts)?;
    // Riders whose address is unknown get this host.
    let fallback = public_host
        .clone()
        .or_else(|| (!listen.is_unspecified()).then(|| listen.to_string()))
        .or_else(|| crate::net::lan_addresses().first().map(ToString::to_string))
        .unwrap_or_else(|| "127.0.0.1".into());
    let config = server.voice_config(&fallback, public_host);
    Ok((server, config, public_ip))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portmap::MemoryRouter;
    use crate::portmap::tests::found;
    #[cfg(unix)]
    use crate::voice::launcher::tests::{fake, port_lock};
    #[cfg(unix)]
    use std::fs;

    fn listen() -> SocketAddr {
        "0.0.0.0:47800".parse().unwrap()
    }

    #[test]
    fn nothing_beside_the_game_server_is_ready_at_once() {
        let s = HostServices::spawn(HostingOptions::new(listen())).unwrap();
        assert!(s.wait_ready(Duration::from_secs(2)));
        let st = s.status();
        assert_eq!(st.router, MapState::Off);
        assert_eq!(st.voice, VoiceState::Off("off".into()));
        assert_eq!(st.public_ip, None);
        assert_eq!(st.ports.len(), 1, "the game's UDP port only");
        assert!(s.voice_config().is_none());
        s.stop();
    }

    #[test]
    fn router_ports_on_without_voice_map_the_game_port_only() {
        let igd = MemoryRouter {
            external: Some("81.2.69.160".parse().unwrap()),
            ..MemoryRouter::default()
        };
        let table = igd.table.clone();
        let opts = HostingOptions {
            web_udp: Some(47801),
            upnp: Some(found(igd)),
            voice_off: "an external LiveKit".into(),
            ..HostingOptions::new(listen())
        };
        let s = HostServices::spawn(opts).unwrap();
        assert!(s.wait_ready(Duration::from_secs(2)));
        let st = s.status();
        assert_eq!(st.public_ip, Some("81.2.69.160".parse().unwrap()));
        assert_eq!(
            table.lock().unwrap().keys().cloned().collect::<Vec<_>>(),
            [("UDP".to_owned(), 47800), ("UDP".to_owned(), 47801)]
        );
        assert_eq!(
            st.voice.summary(),
            "voice: no voice server of its own (an external LiveKit)"
        );
        drop(s);
        assert!(table.lock().unwrap().is_empty(), "removed with the hosting");
    }

    /// The whole lifecycle with a fake voice server and a fake router:
    /// mapped, voice on the router's public address, stopped and
    /// unmapped together.
    #[cfg(unix)]
    #[test]
    fn voice_and_router_ports_for_the_hostings_lifetime() {
        let d = tempfile::tempdir().unwrap();
        let _ports = port_lock();
        let bin = fake::script(d.path(), fake::READY);
        let launch = fake::options(d.path(), bin);
        let vp = launch.ports;
        let igd = MemoryRouter {
            external: Some("81.2.69.160".parse().unwrap()),
            ..MemoryRouter::default()
        };
        let table = igd.table.clone();
        let s = HostServices::spawn(HostingOptions {
            voice: Some(VoicePlan {
                launch,
                public_host: None,
            }),
            upnp: Some(found(igd)),
            ..HostingOptions::new(listen())
        })
        .unwrap();
        assert!(s.wait_ready(Duration::from_secs(10)));
        let st = s.status();
        assert_eq!(st.voice, VoiceState::On(vp));
        assert_eq!(st.ports.len(), 4);
        assert_eq!(table.lock().unwrap().len(), 4, "game and voice mapped");
        // The router's public address is the voice server's media address
        // and the grant's host for riders from the internet (Q5).
        let config = fs::read_to_string(d.path().join("config")).unwrap();
        assert!(config.contains("node_ip: \"81.2.69.160\""), "{config}");
        let cfg = s.voice_config().expect("voice on");
        assert_eq!(cfg.url, format!("ws://81.2.69.160:{}", vp.signal));
        assert_eq!(
            cfg.reach.as_ref().unwrap().public_host.as_deref(),
            Some("81.2.69.160")
        );
        let pid = fake::read_pid(d.path());
        assert!(fake::alive(pid));
        s.stop();
        assert!(
            fake::wait_gone(pid, Duration::from_secs(3)),
            "the voice server stops with the hosting"
        );
        assert!(table.lock().unwrap().is_empty(), "and the mappings go");
    }

    /// Without a router (or switched off) the voice server asks STUN; a
    /// voice server that does not start leaves the hosting without voice,
    /// ready all the same, and its router ports are closed again.
    #[cfg(unix)]
    #[test]
    fn a_failing_voice_server_never_fails_the_hosting() {
        let d = tempfile::tempdir().unwrap();
        let _ports = port_lock();
        let bin = fake::script(d.path(), "echo 'cannot start: fake failure' >&2\nexit 3");
        let launch = fake::options(d.path(), bin);
        let igd = MemoryRouter::default();
        let table = igd.table.clone();
        let s = HostServices::spawn(HostingOptions {
            voice: Some(VoicePlan {
                launch,
                public_host: None,
            }),
            upnp: Some(found(igd)),
            ..HostingOptions::new(listen())
        })
        .unwrap();
        assert!(s.wait_ready(Duration::from_secs(10)));
        let st = s.status();
        assert!(
            matches!(&st.voice, VoiceState::Failed(w) if w.contains("fake failure")),
            "{:?}",
            st.voice
        );
        assert!(s.voice_config().is_none());
        assert_eq!(
            table.lock().unwrap().keys().cloned().collect::<Vec<_>>(),
            [("UDP".to_owned(), 47800)],
            "the voice ports are closed again"
        );
        // No router: STUN.
        let config = fs::read_to_string(d.path().join("config")).unwrap();
        assert!(config.contains("use_external_ip: true"), "{config}");
    }

    /// The given public host wins over the router's address.
    #[cfg(unix)]
    #[test]
    fn the_given_public_host_wins() {
        let d = tempfile::tempdir().unwrap();
        let _ports = port_lock();
        let bin = fake::script(d.path(), fake::READY);
        let launch = fake::options(d.path(), bin);
        let (server, cfg, ip) = start_voice(
            VoicePlan {
                launch,
                public_host: Some("198.51.100.4".into()),
            },
            "0.0.0.0".parse().unwrap(),
            Some("81.2.69.160".parse().unwrap()),
        )
        .unwrap();
        assert_eq!(ip, Some("198.51.100.4".parse().unwrap()));
        assert!(cfg.url.starts_with("ws://198.51.100.4:"));
        drop(server);
    }
}
