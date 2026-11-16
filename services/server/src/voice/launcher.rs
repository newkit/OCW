//! The bundled voice server (spec §7.1.1 (a), milestone V1): whoever hosts
//! a game server that others join starts LiveKit's `livekit-server` beside
//! it, as a child process, and stops it with the game server.
//!
//! - **Keys:** a fresh API key and secret at every start (OS randomness),
//!   held in memory only. The child gets them in its environment
//!   (`LIVEKIT_KEYS="KEY: SECRET"`), never on its command line (that shows
//!   in `ps`) and never in a file; the game server signs the riders' tokens
//!   with the same pair ([`VoiceServer::voice_config`]).
//! - **Config:** generated per start ([`config_yaml`]) and passed in the
//!   environment too (`LIVEKIT_CONFIG`): signalling on TCP 7880, ICE over
//!   TCP on 7881, all media on one UDP port 7882 (LiveKit's UDP mux), no
//!   TURN, the address riders' media go to ([`RtcAddress`], §7.1.7 Q5).
//!   Inherited `LIVEKIT_*` variables are removed, so nothing else changes it.
//! - **Ready** when `/rtc/validate` accepts a token signed with *this*
//!   start's keys (so another LiveKit on the same port is never mistaken
//!   for ours), at most [`READY_TIMEOUT`]; else the start fails, the child
//!   is stopped and the session runs without voice, saying why.
//! - **No orphan:** stopped when the [`VoiceServer`] is dropped; after a
//!   crash of the host, Linux kills it through the parent-death signal
//!   (`setpriv --pdeathsig KILL`, util-linux: `PR_SET_PDEATHSIG` without
//!   unsafe code in this workspace), Windows through a job object that
//!   kills its processes when the host's handle closes, and on every Unix
//!   (macOS above all) a pid file is checked at the next start and a
//!   leftover `livekit-server` is ended.
//! - **Crashes** of the child are restarted with back-off (2, 4 … 60 s,
//!   [`VoiceServer::poll`]) with the same keys and ports; clients reconnect
//!   by themselves (§7 retry).
//!
//! The parent-death signal belongs to the *thread* that started the child:
//! start (and restart) it from a thread that lives as long as the game
//! server (the `ocw-server` binary uses its main thread).
//!
//! Later steps (§7.1.6 V3 / V4: who hears whom) need the server API of
//! LiveKit: [`VoiceServer::api_url`] and [`VoiceServer::keys`] are there
//! for that; the per-rider token grants go through
//! [`super::TokenGrants`].

use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::{VoiceConfig, VoiceReach, jwt_hs256, url_host};

/// The voice server's file name.
pub const BINARY: &str = if cfg!(windows) {
    "livekit-server.exe"
} else {
    "livekit-server"
};
/// The LiveKit server release the apps ship (`scripts/fetch-livekit-server.sh`).
pub const PINNED_VERSION: &str = "1.13.10";
/// How long the voice server may take to answer (spec §7.1.1 *Lifecycle*).
pub const READY_TIMEOUT: Duration = Duration::from_secs(10);
/// The child's environment: its API key pair and its config.
pub const ENV_KEYS: &str = "LIVEKIT_KEYS";
pub const ENV_CONFIG: &str = "LIVEKIT_CONFIG";
/// The longest wait before restarting a crashed voice server.
pub const BACKOFF_MAX: Duration = Duration::from_secs(60);
/// A child that ran this long counts as stable: the next crash waits the
/// first back-off again.
const STABLE: Duration = Duration::from_secs(60);
/// Room and identity of the readiness probe's token.
const PROBE_ROOM: &str = "ocw-ready-probe";

/// The voice server's ports (spec §7.1.1 *Network*).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoicePorts {
    /// Signalling (WebSocket) and LiveKit's HTTP API, TCP.
    pub signal: u16,
    /// ICE over TCP, for riders whose networks block UDP.
    pub tcp: u16,
    /// All media, one UDP port (LiveKit's UDP mux).
    pub udp: u16,
}

impl Default for VoicePorts {
    fn default() -> Self {
        Self {
            signal: 7880,
            tcp: 7881,
            udp: 7882,
        }
    }
}

impl VoicePorts {
    /// `SIGNAL,TCP,UDP`, e.g. `7880,7881,7882` (`--voice-ports`).
    pub fn parse(text: &str) -> Result<Self, String> {
        let p: Vec<u16> = text
            .split(',')
            .map(|s| s.trim().parse::<u16>().ok().filter(|&p| p != 0))
            .collect::<Option<_>>()
            .ok_or_else(|| format!("{text:?}: three ports SIGNAL,TCP,UDP (1-65535)"))?;
        let [signal, tcp, udp] = p[..] else {
            return Err(format!("{text:?}: three ports SIGNAL,TCP,UDP"));
        };
        if signal == tcp {
            return Err(format!("{text:?}: the two TCP ports must differ"));
        }
        Ok(Self { signal, tcp, udp })
    }
}

impl std::fmt::Display for VoicePorts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/tcp, {}/tcp, {}/udp", self.signal, self.tcp, self.udp)
    }
}

/// The address the voice server tells riders to send their media to
/// (ICE candidates; spec §7.1.7 Q5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RtcAddress {
    /// Riders on this machine only (a game server on a loopback address):
    /// signalling on 127.0.0.1, media on this machine's own addresses
    /// (no STUN).
    Loopback,
    /// The public address of the join link's host, plus this machine's
    /// own addresses for riders on the LAN.
    Public(IpAddr),
    /// No host given: LiveKit asks a STUN server for the public address,
    /// plus this machine's own addresses.
    Stun,
}

impl RtcAddress {
    /// For a game server listening on `listen`, with the public address of
    /// the join link's host if one is given.
    pub fn choose(listen: IpAddr, public: Option<IpAddr>) -> Self {
        match public {
            Some(ip) => Self::Public(ip),
            None if listen.is_loopback() => Self::Loopback,
            None => Self::Stun,
        }
    }
}

/// The API key pair of one start. `Debug` never shows the secret.
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKeys {
    pub key: String,
    pub secret: String,
}

impl std::fmt::Debug for ApiKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiKeys")
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl ApiKeys {
    /// A fresh pair: key `OCW` + 16 hex digits, secret 32 random bytes as
    /// 64 hex digits (plain YAML scalars, no quoting needed).
    pub fn generate() -> Result<Self, String> {
        let mut key = [0u8; 8];
        let mut secret = [0u8; 32];
        getrandom::fill(&mut key)
            .and_then(|()| getrandom::fill(&mut secret))
            .map_err(|e| format!("no OS randomness for the voice keys: {e}"))?;
        Ok(Self {
            key: format!("OCW{}", hex(&key)),
            secret: hex(&secret),
        })
    }

    /// The value of [`ENV_KEYS`].
    pub fn env_value(&self) -> String {
        format!("{}: {}", self.key, self.secret)
    }
}

/// The voice server's config (YAML, LiveKit 1.13; it refuses unknown
/// fields). `bind`: the one address to listen on, `None`: all.
pub fn config_yaml(ports: VoicePorts, bind: Option<IpAddr>, rtc: RtcAddress) -> String {
    let mut y = String::from("# OCW's voice server (spec §7.1.1), generated at each start\n");
    y += &format!("port: {}\n", ports.signal);
    if let Some(ip) = bind {
        y += &format!("bind_addresses:\n  - \"{ip}\"\n");
    }
    y += "rtc:\n";
    y += &format!("  tcp_port: {}\n  udp_port: {}\n", ports.tcp, ports.udp);
    match rtc {
        RtcAddress::Loopback => {
            // This machine's own interface addresses: WebRTC clients bind
            // their sockets to those and never send to 127.0.0.1.
            y += "  use_external_ip: false\n";
        }
        RtcAddress::Public(ip) => {
            y += &format!(
                "  use_external_ip: false\n  node_ip: \"{ip}\"\n  advertise_internal_ip: true\n"
            );
        }
        RtcAddress::Stun => {
            y += "  use_external_ip: true\n  advertise_internal_ip: true\n";
        }
    }
    y += "turn:\n  enabled: false\n";
    y += "room:\n  empty_timeout: 300\n";
    y += "logging:\n  level: info\n";
    y
}

/// Finds the voice server: `explicit` (`--voice-server`), else next to
/// the executable (`exe_dir`), else on `path` (`$PATH`).
pub fn find_binary(
    explicit: Option<&Path>,
    exe_dir: Option<&Path>,
    path: Option<&OsStr>,
) -> Result<PathBuf, String> {
    if let Some(p) = explicit {
        return if p.is_file() {
            Ok(p.to_owned())
        } else {
            Err(format!("--voice-server {}: no such file", p.display()))
        };
    }
    let beside = exe_dir.map(|d| d.join(BINARY));
    if let Some(p) = beside.as_ref().filter(|p| p.is_file()) {
        return Ok(p.clone());
    }
    if let Some(found) = path
        .into_iter()
        .flat_map(std::env::split_paths)
        .map(|d| d.join(BINARY))
        .find(|p| p.is_file())
    {
        return Ok(found);
    }
    Err(format!(
        "no {BINARY} next to the executable or on PATH (scripts/fetch-livekit-server.sh fetches it)"
    ))
}

/// The default folder of the voice server's log and pid files:
/// `<data dir>/ocw/voice/`.
pub fn default_state_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("ocw/voice")
}

/// What a readiness probe checks.
pub struct ProbeTarget<'a> {
    /// The signalling port's address on this machine.
    pub addr: SocketAddr,
    pub keys: &'a ApiKeys,
}

/// A readiness check: `true` once the voice server answers.
pub type Probe = Arc<dyn Fn(&ProbeTarget<'_>) -> bool + Send + Sync>;

/// The real check: LiveKit's `/rtc/validate` accepts a token signed with
/// this start's keys (200), so a different server on the port fails it.
pub fn http_probe() -> Probe {
    Arc::new(|t: &ProbeTarget<'_>| {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let claims = serde_json::json!({
            "iss": t.keys.key,
            "sub": PROBE_ROOM,
            "nbf": now.saturating_sub(5),
            "exp": now + 60,
            "video": { "room": PROBE_ROOM, "roomJoin": true },
        });
        let token = jwt_hs256(&claims, t.keys.secret.as_bytes());
        http_status(t.addr, &format!("/rtc/validate?access_token={token}")) == Some(200)
    })
}

/// The status code of `GET path` on `addr` (HTTP/1.1, short timeouts).
fn http_status(addr: SocketAddr, path: &str) -> Option<u16> {
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(500)).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    s.set_write_timeout(Some(Duration::from_secs(2))).ok()?;
    write!(
        s,
        "GET {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        url_host(&addr.ip().to_string())
    )
    .ok()?;
    let mut head = [0u8; 32];
    let mut n = 0;
    while n < 12 {
        match s.read(&mut head[n..]) {
            Ok(0) | Err(_) => break,
            Ok(k) => n += k,
        }
    }
    let text = std::str::from_utf8(&head[..n]).ok()?;
    text.strip_prefix("HTTP/1.")?.get(2..5)?.parse().ok()
}

/// Whether the three ports are free on this machine (any address):
/// another voice server must not answer in place of ours.
pub fn check_ports_free(ports: VoicePorts) -> Result<(), String> {
    let any = IpAddr::V4(Ipv4Addr::UNSPECIFIED);
    let busy = |p: u16, what: &str| format!("port {p}/{what} is in use (another voice server?)");
    TcpListener::bind((any, ports.signal)).map_err(|_| busy(ports.signal, "tcp"))?;
    TcpListener::bind((any, ports.tcp)).map_err(|_| busy(ports.tcp, "tcp"))?;
    UdpSocket::bind((any, ports.udp)).map_err(|_| busy(ports.udp, "udp"))?;
    Ok(())
}

/// How long to wait before the `failures`-th restart: 2, 4, 8 … 60 s
/// (with `first` = 2 s).
pub fn backoff(first: Duration, failures: u32) -> Duration {
    let exp = failures.saturating_sub(1).min(16);
    first.saturating_mul(1 << exp).min(BACKOFF_MAX.max(first))
}

/// Inherited environment variables the child must not see: they would
/// change its config (LiveKit reads `LIVEKIT_*`, `NODE_IP`, `UDP_PORT`,
/// `REDIS_*`) or are the game server's own secret.
pub fn inherited_to_remove(names: impl IntoIterator<Item = OsString>) -> Vec<OsString> {
    names
        .into_iter()
        .filter(|n| {
            n.to_str().is_some_and(|n| {
                n.starts_with("LIVEKIT_")
                    || n.starts_with("REDIS_")
                    || matches!(n, "NODE_IP" | "UDP_PORT" | super::ENV_API_SECRET)
            })
        })
        .collect()
}

/// `setpriv` with the parent-death signal (Linux, util-linux ≥ 2.33).
fn pdeathsig_wrapper() -> Option<PathBuf> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    ["/usr/bin/setpriv", "/bin/setpriv"]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.is_file())
}

/// The command that starts the voice server: the binary (behind `setpriv
/// --pdeathsig KILL` when `wrapper` is given) with no options of its own –
/// keys and config only in the environment.
pub fn build_command(
    binary: &Path,
    keys: &ApiKeys,
    config: &str,
    wrapper: Option<&Path>,
) -> Command {
    let mut cmd = match wrapper {
        Some(w) => {
            let mut c = Command::new(w);
            c.args(["--pdeathsig", "KILL", "--"]).arg(binary);
            c
        }
        None => Command::new(binary),
    };
    for name in inherited_to_remove(std::env::vars_os().map(|(k, _)| k)) {
        cmd.env_remove(name);
    }
    cmd.env(ENV_KEYS, keys.env_value())
        .env(ENV_CONFIG, config)
        .stdin(Stdio::null());
    cmd
}

/// How to start the voice server.
#[derive(Clone)]
pub struct LaunchOptions {
    pub binary: PathBuf,
    pub ports: VoicePorts,
    /// The one address to listen on (`None`: all).
    pub bind: Option<IpAddr>,
    pub rtc: RtcAddress,
    pub ready_timeout: Duration,
    /// The first wait before a restart after a crash.
    pub backoff_first: Duration,
    /// Its log (`livekit-PORT.log`) and pid file (`livekit-PORT.pid`).
    pub state_dir: PathBuf,
    pub probe: Probe,
}

impl LaunchOptions {
    /// The defaults: ports 7880 / 7881 / 7882, all addresses, STUN, 10 s,
    /// the real readiness probe.
    pub fn new(binary: impl Into<PathBuf>, state_dir: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
            ports: VoicePorts::default(),
            bind: None,
            rtc: RtcAddress::Stun,
            ready_timeout: READY_TIMEOUT,
            backoff_first: Duration::from_secs(2),
            state_dir: state_dir.into(),
            probe: http_probe(),
        }
    }

    fn log_file(&self) -> PathBuf {
        self.state_dir
            .join(format!("livekit-{}.log", self.ports.signal))
    }

    fn pid_file(&self) -> PathBuf {
        self.state_dir
            .join(format!("livekit-{}.pid", self.ports.signal))
    }

    fn probe_addr(&self) -> SocketAddr {
        let ip = match self.bind {
            Some(ip) if !ip.is_unspecified() => ip,
            _ => IpAddr::V4(Ipv4Addr::LOCALHOST),
        };
        SocketAddr::new(ip, self.ports.signal)
    }
}

/// What [`VoiceServer::poll`] noticed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoiceEvent {
    /// The voice server ended by itself; it is started again after `retry_in`.
    Crashed { status: String, retry_in: Duration },
    /// Started again and ready.
    Restarted,
    /// A restart failed; tried again after `retry_in`.
    RestartFailed { error: String, retry_in: Duration },
}

#[derive(Debug, Clone, Copy)]
enum State {
    Running { since: Instant },
    Starting { since: Instant },
    Waiting { until: Instant },
    Stopped,
}

/// A running voice server, stopped when dropped.
pub struct VoiceServer {
    opts: LaunchOptions,
    keys: ApiKeys,
    config: String,
    child: Option<Child>,
    #[cfg(windows)]
    job: Option<win32job::Job>,
    state: State,
    failures: u32,
}

impl std::fmt::Debug for VoiceServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VoiceServer")
            .field("binary", &self.opts.binary)
            .field("ports", &self.opts.ports)
            .field("keys", &self.keys)
            .field("pid", &self.pid())
            .finish_non_exhaustive()
    }
}

impl VoiceServer {
    /// Starts the voice server with fresh keys and waits until it is ready
    /// (at most [`LaunchOptions::ready_timeout`]). On an error nothing is
    /// left running.
    pub fn start(opts: LaunchOptions) -> Result<Self, String> {
        let keys = ApiKeys::generate()?;
        fs::create_dir_all(&opts.state_dir)
            .map_err(|e| format!("{}: {e}", opts.state_dir.display()))?;
        if let Some(pid) = clean_stale(&opts.pid_file()) {
            tracing::warn!(pid, "ended a voice server left over from an earlier run");
        }
        check_ports_free(opts.ports)?;
        let config = config_yaml(opts.ports, opts.bind, opts.rtc);
        let mut server = Self {
            opts,
            keys,
            config,
            child: None,
            #[cfg(windows)]
            job: None,
            state: State::Stopped,
            failures: 0,
        };
        server.spawn(true)?;
        let started = Instant::now();
        loop {
            if let Some(status) = server.exited() {
                let tail = server.log_tail();
                server.stop();
                return Err(format!(
                    "{} exited at start ({status}){tail}",
                    server.opts.binary.display()
                ));
            }
            if server.ready() {
                server.state = State::Running {
                    since: Instant::now(),
                };
                return Ok(server);
            }
            if started.elapsed() >= server.opts.ready_timeout {
                let tail = server.log_tail();
                server.stop();
                return Err(format!(
                    "the voice server was not ready after {} s{tail}",
                    server.opts.ready_timeout.as_secs_f32()
                ));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// This start's API key pair (the game server signs tokens with it).
    pub fn keys(&self) -> &ApiKeys {
        &self.keys
    }

    pub fn ports(&self) -> VoicePorts {
        self.opts.ports
    }

    /// The generated config (no secrets in it).
    pub fn config(&self) -> &str {
        &self.config
    }

    /// The process id of the voice server, while one runs.
    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().map(Child::id)
    }

    /// Whether it runs and answered.
    pub fn is_running(&self) -> bool {
        matches!(self.state, State::Running { .. })
    }

    /// LiveKit's HTTP server API on this machine (room service: the
    /// per-rider subscription rules of §7.1.7 Q6 / Q8 will go through it).
    pub fn api_url(&self) -> String {
        let a = self.opts.probe_addr();
        format!("http://{}:{}", url_host(&a.ip().to_string()), a.port())
    }

    /// The game server's voice configuration for this voice server: tokens
    /// signed with its keys; each grant names the address the rider
    /// reached (`reach`), `ws://FALLBACK_HOST:PORT` when that is unknown.
    pub fn voice_config(&self, fallback_host: &str, public_host: Option<String>) -> VoiceConfig {
        let port = self.opts.ports.signal;
        VoiceConfig {
            reach: Some(VoiceReach { port, public_host }),
            ..VoiceConfig::new(
                format!("ws://{}:{port}", url_host(fallback_host)),
                self.keys.key.clone(),
                self.keys.secret.clone(),
            )
        }
    }

    /// Watches the child: call it now and then (about once a second) from
    /// the thread that started it. Restarts a crashed voice server after
    /// the back-off, without blocking.
    pub fn poll(&mut self) -> Option<VoiceEvent> {
        let now = Instant::now();
        match self.state {
            State::Stopped => None,
            State::Running { since } | State::Starting { since } => {
                let starting = matches!(self.state, State::Starting { .. });
                if let Some(status) = self.exited() {
                    if !starting && now.duration_since(since) >= STABLE {
                        self.failures = 0;
                    }
                    return Some(self.crashed(status));
                }
                if starting {
                    if self.ready() {
                        self.state = State::Running { since: now };
                        return Some(VoiceEvent::Restarted);
                    }
                    if now.duration_since(since) >= self.opts.ready_timeout {
                        self.kill_child();
                        return Some(self.crashed("not ready after a restart".into()));
                    }
                }
                None
            }
            State::Waiting { until } if now >= until => match self.spawn(false) {
                Ok(()) => {
                    self.state = State::Starting { since: now };
                    None
                }
                Err(error) => {
                    self.failures += 1;
                    let retry_in = backoff(self.opts.backoff_first, self.failures);
                    self.state = State::Waiting {
                        until: now + retry_in,
                    };
                    Some(VoiceEvent::RestartFailed { error, retry_in })
                }
            },
            State::Waiting { .. } => None,
        }
    }

    /// Stops the voice server (also on drop).
    pub fn stop(&mut self) {
        self.kill_child();
        self.state = State::Stopped;
        let _ = fs::remove_file(self.opts.pid_file());
    }

    fn crashed(&mut self, status: String) -> VoiceEvent {
        self.child = None;
        self.failures += 1;
        let retry_in = backoff(self.opts.backoff_first, self.failures);
        self.state = State::Waiting {
            until: Instant::now() + retry_in,
        };
        VoiceEvent::Crashed { status, retry_in }
    }

    fn ready(&self) -> bool {
        (self.opts.probe)(&ProbeTarget {
            addr: self.opts.probe_addr(),
            keys: &self.keys,
        })
    }

    /// The exit status, once the child has ended.
    fn exited(&mut self) -> Option<String> {
        let child = self.child.as_mut()?;
        match child.try_wait() {
            Ok(Some(status)) => Some(status.to_string()),
            Ok(None) => None,
            Err(e) => Some(e.to_string()),
        }
    }

    fn kill_child(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        #[cfg(windows)]
        {
            self.job = None;
        }
    }

    fn spawn(&mut self, first: bool) -> Result<(), String> {
        let log_path = self.opts.log_file();
        let log = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(!first)
            .truncate(first)
            .open(&log_path)
            .map_err(|e| format!("{}: {e}", log_path.display()))?;
        let err = log.try_clone().map_err(|e| e.to_string())?;
        let wrapper = pdeathsig_wrapper();
        if first && wrapper.is_none() && cfg!(target_os = "linux") {
            tracing::warn!(
                "no setpriv (util-linux): a crash of this server may leave the voice server running until the next start"
            );
        }
        let mut cmd = build_command(
            &self.opts.binary,
            &self.keys,
            &self.config,
            wrapper.as_deref(),
        );
        cmd.stdout(log).stderr(err);
        let child = cmd
            .spawn()
            .map_err(|e| format!("{}: {e}", self.opts.binary.display()))?;
        #[cfg(windows)]
        {
            self.job = kill_on_close_job(&child);
        }
        if let Err(e) = fs::write(self.opts.pid_file(), child.id().to_string()) {
            tracing::warn!(%e, "voice server pid file");
        }
        self.child = Some(child);
        Ok(())
    }

    /// The last lines of the voice server's log, for error messages.
    fn log_tail(&self) -> String {
        let text = fs::read_to_string(self.opts.log_file()).unwrap_or_default();
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        if lines.is_empty() {
            return String::new();
        }
        let tail = lines[lines.len().saturating_sub(3)..].join(" | ");
        format!(
            "; log {}: {}",
            self.opts.log_file().display(),
            tail.chars().take(600).collect::<String>()
        )
    }
}

impl Drop for VoiceServer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// A job object that ends the child when this process's handle to it
/// closes – also when the host crashes (Windows).
#[cfg(windows)]
fn kill_on_close_job(child: &Child) -> Option<win32job::Job> {
    use std::os::windows::io::AsRawHandle;
    let mut info = win32job::ExtendedLimitInfo::new();
    info.limit_kill_on_job_close();
    let job = win32job::Job::create_with_limit_info(&info)
        .map_err(|e| tracing::warn!(?e, "voice server job object"))
        .ok()?;
    job.assign_process(child.as_raw_handle() as isize)
        .map_err(|e| tracing::warn!(?e, "voice server job object"))
        .ok()?;
    Some(job)
}

/// Ends a voice server left over from an earlier run (its pid file names
/// it and the process is still a `livekit-server`) and removes the file.
/// Returns the pid it ended.
pub fn clean_stale(pid_file: &Path) -> Option<u32> {
    let pid: u32 = fs::read_to_string(pid_file).ok()?.trim().parse().ok()?;
    let _ = fs::remove_file(pid_file);
    if !cfg!(unix) || !process_is_voice_server(pid) {
        return None;
    }
    let _ = Command::new("kill")
        .args(["-KILL", &pid.to_string()])
        .stderr(Stdio::null())
        .status();
    let deadline = Instant::now() + Duration::from_secs(3);
    while process_is_voice_server(pid) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    Some(pid)
}

/// Whether `pid` is a live (not zombie) process whose name names LiveKit.
fn process_is_voice_server(pid: u32) -> bool {
    let out = Command::new("ps")
        .args(["-o", "stat=,comm=", "-p", &pid.to_string()])
        .stderr(Stdio::null())
        .output();
    let Ok(out) = out else {
        return false;
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut parts = text.split_whitespace();
    let (Some(stat), Some(comm)) = (parts.next(), parts.next()) else {
        return false;
    };
    !stat.starts_with('Z') && comm.contains("livekit")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ports_parse_and_show() {
        assert_eq!(
            VoicePorts::parse("7880,7881,7882").unwrap(),
            VoicePorts::default()
        );
        assert_eq!(
            VoicePorts::parse(" 9000, 9001 ,9002").unwrap(),
            VoicePorts {
                signal: 9000,
                tcp: 9001,
                udp: 9002
            }
        );
        for bad in [
            "",
            "7880",
            "7880,7881",
            "1,2,3,4",
            "0,1,2",
            "a,b,c",
            "7880,7880,7882",
        ] {
            assert!(VoicePorts::parse(bad).is_err(), "{bad:?}");
        }
        assert_eq!(
            VoicePorts::default().to_string(),
            "7880/tcp, 7881/tcp, 7882/udp"
        );
    }

    #[test]
    fn config_has_the_ports_and_the_rtc_address() {
        let ports = VoicePorts::default();
        let lo = config_yaml(
            ports,
            Some("127.0.0.1".parse().unwrap()),
            RtcAddress::Loopback,
        );
        for line in [
            "port: 7880\n",
            "bind_addresses:\n  - \"127.0.0.1\"\n",
            "  tcp_port: 7881\n",
            "  udp_port: 7882\n",
            "  use_external_ip: false\n",
            "turn:\n  enabled: false\n",
        ] {
            assert!(lo.contains(line), "{line:?} in\n{lo}");
        }
        assert!(!lo.contains("node_ip"), "media on the interface addresses");
        let public = config_yaml(
            ports,
            None,
            RtcAddress::Public("203.0.113.7".parse().unwrap()),
        );
        assert!(!public.contains("bind_addresses"));
        assert!(public.contains("  node_ip: \"203.0.113.7\"\n"));
        assert!(public.contains("  advertise_internal_ip: true\n"));
        assert!(public.contains("  use_external_ip: false\n"));
        let stun = config_yaml(ports, None, RtcAddress::Stun);
        assert!(stun.contains("  use_external_ip: true\n"));
        assert!(stun.contains("  advertise_internal_ip: true\n"));
        assert!(!stun.contains("node_ip"));
        // No keys in the config: they go in their own variable.
        assert!(!stun.contains("keys"));
    }

    #[test]
    fn rtc_address_follows_the_join_host() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        assert_eq!(
            RtcAddress::choose(ip("127.0.0.1"), None),
            RtcAddress::Loopback
        );
        assert_eq!(RtcAddress::choose(ip("0.0.0.0"), None), RtcAddress::Stun);
        assert_eq!(
            RtcAddress::choose(ip("0.0.0.0"), Some(ip("203.0.113.7"))),
            RtcAddress::Public(ip("203.0.113.7"))
        );
    }

    #[test]
    fn keys_are_fresh_and_never_shown() {
        let a = ApiKeys::generate().unwrap();
        let b = ApiKeys::generate().unwrap();
        assert_ne!(a.secret, b.secret);
        assert_ne!(a.key, b.key);
        assert_eq!(a.secret.len(), 64);
        assert!(a.key.starts_with("OCW") && a.key.len() == 19);
        assert_eq!(a.env_value(), format!("{}: {}", a.key, a.secret));
        assert!(!format!("{a:?}").contains(&a.secret));
    }

    #[test]
    fn binary_found_explicitly_beside_the_exe_then_on_path() {
        let d = tempfile::tempdir().unwrap();
        let (exe, onpath, other) = (
            d.path().join("exe"),
            d.path().join("bin"),
            d.path().join("x"),
        );
        for p in [&exe, &onpath, &other] {
            fs::create_dir_all(p).unwrap();
        }
        let path = std::env::join_paths([&other, &onpath]).unwrap();
        assert!(find_binary(None, Some(&exe), Some(&path)).is_err());
        fs::write(onpath.join(BINARY), "").unwrap();
        assert_eq!(
            find_binary(None, Some(&exe), Some(&path)).unwrap(),
            onpath.join(BINARY)
        );
        fs::write(exe.join(BINARY), "").unwrap();
        assert_eq!(
            find_binary(None, Some(&exe), Some(&path)).unwrap(),
            exe.join(BINARY)
        );
        let given = other.join("my-livekit");
        assert!(find_binary(Some(&given), Some(&exe), Some(&path)).is_err());
        fs::write(&given, "").unwrap();
        assert_eq!(
            find_binary(Some(&given), Some(&exe), Some(&path)).unwrap(),
            given
        );
    }

    #[test]
    fn backoff_doubles_up_to_a_minute() {
        let s = |n| backoff(Duration::from_secs(2), n).as_secs();
        assert_eq!(
            (1..=8).map(s).collect::<Vec<_>>(),
            [2, 4, 8, 16, 32, 60, 60, 60]
        );
        assert_eq!(s(1000), 60);
    }

    #[test]
    fn the_secret_is_in_the_environment_never_in_argv() {
        let keys = ApiKeys::generate().unwrap();
        let config = config_yaml(VoicePorts::default(), None, RtcAddress::Stun);
        for wrapper in [None, Some(Path::new("/usr/bin/setpriv"))] {
            let cmd = build_command(
                Path::new("/opt/ocw/livekit-server"),
                &keys,
                &config,
                wrapper,
            );
            let argv: Vec<String> = std::iter::once(cmd.get_program())
                .chain(cmd.get_args())
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            let all = argv.join(" ");
            assert!(
                !all.contains(&keys.secret) && !all.contains(&keys.key),
                "{all}"
            );
            assert!(
                !all.contains("port"),
                "no config on the command line: {all}"
            );
            assert!(all.ends_with("livekit-server"), "{all}");
            let env: Vec<(String, Option<String>)> = cmd
                .get_envs()
                .map(|(k, v)| {
                    (
                        k.to_string_lossy().into_owned(),
                        v.map(|v| v.to_string_lossy().into_owned()),
                    )
                })
                .collect();
            assert!(env.contains(&(ENV_KEYS.into(), Some(keys.env_value()))));
            assert!(env.contains(&(ENV_CONFIG.into(), Some(config.clone()))));
        }
    }

    #[test]
    fn inherited_livekit_variables_are_removed() {
        let names = [
            "PATH",
            "LIVEKIT_KEYS",
            "LIVEKIT_CONFIG",
            "NODE_IP",
            "UDP_PORT",
            "REDIS_HOST",
            "OCW_LIVEKIT_API_SECRET",
            "OCW_LIVEKIT_URL",
            "HOME",
        ]
        .map(OsString::from);
        assert_eq!(
            inherited_to_remove(names),
            [
                "LIVEKIT_KEYS",
                "LIVEKIT_CONFIG",
                "NODE_IP",
                "UDP_PORT",
                "REDIS_HOST",
                "OCW_LIVEKIT_API_SECRET"
            ]
            .map(OsString::from)
        );
    }

    /// Tests that pick free ports run one at a time: ports freed by one
    /// are soon handed out again to another.
    fn port_lock() -> std::sync::MutexGuard<'static, ()> {
        static PORTS: std::sync::Mutex<()> = std::sync::Mutex::new(());
        PORTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[test]
    fn busy_ports_are_found() {
        let _ports = port_lock();
        let l = TcpListener::bind("0.0.0.0:0").unwrap();
        let p = l.local_addr().unwrap().port();
        let free = || {
            let t = TcpListener::bind("0.0.0.0:0").unwrap();
            t.local_addr().unwrap().port()
        };
        let e = check_ports_free(VoicePorts {
            signal: p,
            tcp: free(),
            udp: free(),
        })
        .unwrap_err();
        assert!(e.contains(&format!("port {p}/tcp is in use")), "{e}");
    }

    // ---------------------------------------------- with a fake voice server
    #[cfg(unix)]
    mod fake {
        use super::*;
        use std::os::unix::fs::PermissionsExt;

        /// Free TCP + UDP ports for one fake start.
        pub(super) fn free_ports() -> VoicePorts {
            loop {
                let a = TcpListener::bind("0.0.0.0:0").unwrap();
                let b = TcpListener::bind("0.0.0.0:0").unwrap();
                let c = UdpSocket::bind("0.0.0.0:0").unwrap();
                let p = VoicePorts {
                    signal: a.local_addr().unwrap().port(),
                    tcp: b.local_addr().unwrap().port(),
                    udp: c.local_addr().unwrap().port(),
                };
                if p.signal != p.tcp {
                    return p;
                }
            }
        }

        /// A fake `livekit-server` in `dir`: records its argv, command
        /// line and environment, then (if `ready`) marks itself ready and
        /// sleeps as one process; `body` replaces the end.
        pub(super) fn script(dir: &Path, body: &str) -> PathBuf {
            let out = dir.display();
            let text = format!(
                "#!/bin/sh\n\
                 echo $$ > '{out}/pid'\n\
                 printf '%s\\n' \"$0\" \"$@\" > '{out}/argv'\n\
                 if [ -r /proc/$$/cmdline ]; then tr '\\0' ' ' < /proc/$$/cmdline > '{out}/cmdline'; fi\n\
                 printf '%s' \"$LIVEKIT_KEYS\" > '{out}/keys'\n\
                 printf '%s' \"$LIVEKIT_CONFIG\" > '{out}/config'\n\
                 {body}\n"
            );
            let path = dir.join("bin").join(BINARY);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, text).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
            path
        }

        pub(super) const READY: &str = "touch \"$(dirname \"$0\")/../ready\"\nexec sleep 1000";

        /// Ready when the fake marked itself.
        pub(super) fn options(dir: &Path, binary: PathBuf) -> LaunchOptions {
            let marker = dir.join("ready");
            LaunchOptions {
                ports: free_ports(),
                ready_timeout: Duration::from_secs(5),
                backoff_first: Duration::from_millis(50),
                probe: Arc::new(move |_| marker.exists()),
                ..LaunchOptions::new(binary, dir.join("state"))
            }
        }

        /// Whether `pid` is a live process (zombies count as gone).
        pub(super) fn alive(pid: u32) -> bool {
            let out = Command::new("ps")
                .args(["-o", "stat=", "-p", &pid.to_string()])
                .output()
                .unwrap();
            let s = String::from_utf8_lossy(&out.stdout);
            let s = s.trim();
            !s.is_empty() && !s.starts_with('Z')
        }

        pub(super) fn wait_gone(pid: u32, within: Duration) -> bool {
            let end = Instant::now() + within;
            while Instant::now() < end {
                if !alive(pid) {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            !alive(pid)
        }

        pub(super) fn read_pid(dir: &Path) -> u32 {
            fs::read_to_string(dir.join("pid"))
                .unwrap()
                .trim()
                .parse()
                .unwrap()
        }
    }

    #[cfg(unix)]
    #[test]
    fn starts_with_keys_in_the_environment_and_stops() {
        use fake::*;
        let d = tempfile::tempdir().unwrap();
        let _ports = port_lock();
        let bin = script(d.path(), READY);
        let opts = options(d.path(), bin);
        let ports = opts.ports;
        let server = VoiceServer::start(opts).unwrap();
        assert!(server.is_running());
        let pid = server.pid().unwrap();
        let keys = server.keys().clone();
        // The secret reached the child in its environment …
        assert_eq!(
            fs::read_to_string(d.path().join("keys")).unwrap(),
            keys.env_value()
        );
        let config = fs::read_to_string(d.path().join("config")).unwrap();
        assert!(config.contains(&format!("port: {}\n", ports.signal)));
        // … never on its command line (the arguments, and as `ps` shows it).
        let argv = fs::read_to_string(d.path().join("argv")).unwrap();
        assert_eq!(argv.lines().count(), 1, "no arguments: {argv}");
        if let Ok(cmdline) = fs::read_to_string(d.path().join("cmdline")) {
            assert!(!cmdline.contains(&keys.secret), "{cmdline}");
            assert!(!cmdline.contains("--pdeathsig"), "setpriv execs: {cmdline}");
        }
        let pid_file = d.path().join(format!("state/livekit-{}.pid", ports.signal));
        assert_eq!(fs::read_to_string(&pid_file).unwrap(), pid.to_string());
        // The game server's tokens use the same keys.
        let cfg = server.voice_config("192.168.1.20", None);
        assert_eq!(cfg.url, format!("ws://192.168.1.20:{}", ports.signal));
        assert_eq!(
            (cfg.api_key.as_str(), cfg.api_secret.as_str()),
            (keys.key.as_str(), keys.secret.as_str())
        );
        assert_eq!(cfg.reach.as_ref().unwrap().port, ports.signal);
        assert_eq!(
            server.api_url(),
            format!("http://127.0.0.1:{}", ports.signal)
        );
        drop(server);
        assert!(
            wait_gone(pid, Duration::from_secs(3)),
            "stopped with the server"
        );
        assert!(!pid_file.exists());
        // A new start: new keys.
        let _ = fs::remove_file(d.path().join("ready"));
        let again =
            VoiceServer::start(options(d.path(), d.path().join("bin").join(BINARY))).unwrap();
        assert_ne!(again.keys().secret, keys.secret);
    }

    #[cfg(unix)]
    #[test]
    fn not_ready_in_time_fails_and_leaves_nothing() {
        use fake::*;
        let d = tempfile::tempdir().unwrap();
        let _ports = port_lock();
        let bin = script(d.path(), "exec sleep 1000");
        let opts = LaunchOptions {
            ready_timeout: Duration::from_millis(400),
            ..options(d.path(), bin)
        };
        let t = Instant::now();
        let e = VoiceServer::start(opts).unwrap_err();
        assert!(e.contains("not ready after 0.4 s"), "{e}");
        assert!(t.elapsed() < Duration::from_secs(3));
        assert!(wait_gone(read_pid(d.path()), Duration::from_secs(3)));
    }

    #[cfg(unix)]
    #[test]
    fn an_exit_at_start_says_why() {
        use fake::*;
        let d = tempfile::tempdir().unwrap();
        let _ports = port_lock();
        let bin = script(
            d.path(),
            "echo 'could not parse config: Ada test' >&2\nexit 3",
        );
        let e = VoiceServer::start(options(d.path(), bin)).unwrap_err();
        assert!(e.contains("exited at start"), "{e}");
        assert!(e.contains("could not parse config: Ada test"), "{e}");
    }

    #[cfg(unix)]
    #[test]
    fn a_crash_is_restarted_with_the_same_keys() {
        use fake::*;
        let d = tempfile::tempdir().unwrap();
        let _ports = port_lock();
        let bin = script(d.path(), READY);
        let mut server = VoiceServer::start(options(d.path(), bin)).unwrap();
        let (pid, keys) = (server.pid().unwrap(), server.keys().clone());
        assert_eq!(server.poll(), None);
        let _ = fs::remove_file(d.path().join("ready"));
        Command::new("kill")
            .args(["-KILL", &pid.to_string()])
            .status()
            .unwrap();
        let mut events = Vec::new();
        let end = Instant::now() + Duration::from_secs(10);
        while !events.contains(&VoiceEvent::Restarted) && Instant::now() < end {
            events.extend(server.poll());
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            matches!(&events[0], VoiceEvent::Crashed { retry_in, .. } if *retry_in == Duration::from_millis(50)),
            "{events:?}"
        );
        assert!(events.contains(&VoiceEvent::Restarted), "{events:?}");
        assert!(server.is_running());
        let new_pid = server.pid().unwrap();
        assert_ne!(new_pid, pid);
        assert_eq!(server.keys(), &keys);
        assert_eq!(
            fs::read_to_string(d.path().join("keys")).unwrap(),
            keys.env_value()
        );
        drop(server);
        assert!(wait_gone(new_pid, Duration::from_secs(3)));
    }

    #[cfg(unix)]
    #[test]
    fn a_leftover_from_a_crashed_run_is_ended_at_the_next_start() {
        use fake::*;
        let d = tempfile::tempdir().unwrap();
        // A "livekit-server" left running: `sleep` under that name.
        let sleep = ["/bin/sleep", "/usr/bin/sleep"]
            .into_iter()
            .find(|p| Path::new(p).exists())
            .unwrap();
        let fake = d.path().join(BINARY);
        std::os::unix::fs::symlink(sleep, &fake).unwrap();
        let mut left = Command::new(&fake).arg("30").spawn().unwrap();
        let pid = left.id();
        let pid_file = d.path().join("livekit-7880.pid");
        fs::write(&pid_file, pid.to_string()).unwrap();
        // Not ended: an unrelated process under a recycled pid.
        let mut other = Command::new(sleep).arg("30").spawn().unwrap();
        let other_file = d.path().join("other.pid");
        fs::write(&other_file, other.id().to_string()).unwrap();
        assert_eq!(clean_stale(&other_file), None);
        assert!(alive(other.id()));
        let _ = other.kill();
        let _ = other.wait();
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(clean_stale(&pid_file), Some(pid));
        let _ = left.wait();
        assert!(!alive(pid));
        assert!(!pid_file.exists());
        assert_eq!(clean_stale(&pid_file), None);
    }

    /// The parent's side of [`no_orphan_when_the_host_is_killed`]: started
    /// as a child process of the test binary.
    #[cfg(target_os = "linux")]
    #[test]
    fn orphan_parent_role() {
        let Some(dir) = std::env::var_os("OCW_LAUNCHER_TEST_PARENT") else {
            return;
        };
        let dir = PathBuf::from(dir);
        let bin = fake::script(&dir, fake::READY);
        let server = VoiceServer::start(fake::options(&dir, bin)).unwrap();
        fs::write(dir.join("child_pid"), server.pid().unwrap().to_string()).unwrap();
        std::thread::sleep(Duration::from_secs(60));
        drop(server);
    }

    /// Killing the host (SIGKILL: no destructor runs) ends the voice
    /// server too (Linux: the parent-death signal through setpriv).
    #[cfg(target_os = "linux")]
    #[test]
    fn no_orphan_when_the_host_is_killed() {
        if pdeathsig_wrapper().is_none() {
            eprintln!("skipped: no setpriv on this machine");
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let _ports = port_lock();
        let mut parent = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "voice::launcher::tests::orphan_parent_role",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("OCW_LAUNCHER_TEST_PARENT", d.path())
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let pid_file = d.path().join("child_pid");
        let end = Instant::now() + Duration::from_secs(20);
        while !pid_file.exists() && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(20));
        }
        let child: u32 = fs::read_to_string(&pid_file)
            .expect("the parent started the voice server")
            .trim()
            .parse()
            .unwrap();
        assert!(fake::alive(child));
        parent.kill().unwrap();
        parent.wait().unwrap();
        assert!(
            fake::wait_gone(child, Duration::from_secs(5)),
            "the voice server {child} outlived its killed host"
        );
    }
}
