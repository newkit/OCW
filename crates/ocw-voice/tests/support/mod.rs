//! Helpers shared by the voice backend's integration tests: waiting for
//! backend events without losing any, and a throw-away `livekit-server`.
#![allow(dead_code)]

use std::net::{Ipv4Addr, TcpListener, TcpStream, UdpSocket};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use ocw_voice::backend::{BackendEvent, VoiceBackend};

/// Polls `b` until an event matches `what` (true) or `timeout` passes
/// (false). Every polled event is kept in `seen` – also the ones after the
/// match in the same batch: `poll` drains the backend's queue, so an event
/// dropped here is gone for good (the bug of 2026-11-16: `Connected` and
/// the `ParticipantJoined` of a rider already in the room come in one
/// batch, and the join was lost). A match already in `seen` counts.
pub fn wait_for<B: VoiceBackend + ?Sized>(
    b: &mut B,
    seen: &mut Vec<BackendEvent>,
    timeout: Duration,
    what: impl Fn(&BackendEvent) -> bool,
) -> bool {
    if seen.iter().any(&what) {
        return true;
    }
    let start = Instant::now();
    loop {
        let batch = b.poll();
        let hit = batch.iter().any(&what);
        seen.extend(batch);
        if hit {
            return true;
        }
        if start.elapsed() >= timeout {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A `livekit-server` of its own for one test, on free loopback ports;
/// killed when dropped.
pub struct LiveKitServer {
    child: Child,
    pub url: String,
    pub api_key: String,
    pub api_secret: String,
    pub log: PathBuf,
}

/// The binary: `OCW_LIVEKIT_SERVER` (if set and not empty; it must
/// exist), else `~/.local/bin/livekit-server` (where the build machines
/// have it), else `livekit-server` on `PATH`.
pub fn livekit_server_binary() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("OCW_LIVEKIT_SERVER").filter(|p| !p.is_empty()) {
        let p = PathBuf::from(p);
        assert!(p.is_file(), "OCW_LIVEKIT_SERVER: no file {}", p.display());
        return Some(p);
    }
    let exe = format!("livekit-server{}", std::env::consts::EXE_SUFFIX);
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    let local = home.map(|h| PathBuf::from(h).join(".local").join("bin").join(&exe));
    let on_path = std::env::var_os("PATH")
        .map(|p| {
            std::env::split_paths(&p)
                .map(|d| d.join(&exe))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    local.into_iter().chain(on_path).find(|p| p.is_file())
}

fn free_tcp_port() -> u16 {
    TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .expect("a free TCP port")
}

fn free_udp_port() -> u16 {
    UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .and_then(|s| s.local_addr())
        .map(|a| a.port())
        .expect("a free UDP port")
}

impl LiveKitServer {
    /// Starts the server and waits until it accepts connections. `None`
    /// when there is no binary (the caller skips).
    pub fn start() -> Option<Self> {
        let bin = livekit_server_binary()?;
        let (port, tcp, udp) = (free_tcp_port(), free_tcp_port(), free_udp_port());
        // Made-up test keys (the server wants a secret of 32+ characters).
        let api_key = "ocwtestkey".to_string();
        let api_secret = "ocw-test-secret-not-for-production-use".to_string();
        let config = format!(
            "port: {port}\n\
             rtc:\n  tcp_port: {tcp}\n  udp_port: {udp}\n  use_external_ip: false\n  node_ip: 127.0.0.1\n\
             keys:\n  {api_key}: {api_secret}\n\
             logging:\n  level: info\n"
        );
        let log = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("livekit-{port}.log"));
        let out = std::fs::File::create(&log).expect("the server log");
        let err = out.try_clone().expect("the server log");
        let child = Command::new(&bin)
            .args(["--bind", "127.0.0.1", "--config-body", &config])
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(err)
            .spawn()
            .unwrap_or_else(|e| panic!("starting {}: {e}", bin.display()));
        let mut server = Self {
            child,
            url: format!("ws://127.0.0.1:{port}"),
            api_key,
            api_secret,
            log,
        };
        let start = Instant::now();
        while TcpStream::connect((Ipv4Addr::LOCALHOST, port)).is_err() {
            if let Ok(Some(status)) = server.child.try_wait() {
                panic!(
                    "livekit-server ended ({status}), log: {}",
                    server.log.display()
                );
            }
            assert!(
                start.elapsed() < Duration::from_secs(15),
                "livekit-server did not listen within 15 s, log: {}",
                server.log.display()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        eprintln!(
            "started {} on port {port} (log {})",
            bin.display(),
            server.log.display()
        );
        Some(server)
    }
}

impl Drop for LiveKitServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
