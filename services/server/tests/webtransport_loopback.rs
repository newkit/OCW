//! The WebTransport endpoint end to end on 127.0.0.1 (spec §14 Phase 10
//! B3): a server thread runs the game loop with both endpoints like
//! `ocw-server --listen --webtransport`; clients connect over WebTransport
//! with [`NetClient::connect_web`] (pinning the certificate by its hash,
//! as browsers do with `serverCertificateHashes`) and over native QUIC,
//! and share one session.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use ocw_core::{RiderId, SIM_DT};
use ocw_protocol::v1::{self, client_message::Msg as C, server_message::Msg as S};
use ocw_protocol::{PROTOCOL_VERSION, client_message, decode_server, encode};
use ocw_server::net::tls::parse_fingerprint;
use ocw_server::net::{
    ClientOptions, LinkStatus, NetClient, NetServer, ServerIdentity, ServerTrust, Transport,
    WebIdentity,
};
use ocw_server::{GameServer, ServerConfig};
use ocw_world::Route;

fn unix_now_s() -> i64 {
    ocw_server::unix_now_ms() / 1000
}

struct TestServer {
    quic: SocketAddr,
    web: SocketAddr,
    fingerprint: String,
    web_hash: String,
    riders: Arc<AtomicUsize>,
    web_messages: Arc<AtomicU64>,
    web_datagrams: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl TestServer {
    fn start(bots: u32) -> Self {
        let (tx, rx) = mpsc::channel();
        let riders = Arc::new(AtomicUsize::new(0));
        let web_messages = Arc::new(AtomicU64::new(0));
        let web_datagrams = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (r, s, wm, wd) = (
            riders.clone(),
            stop.clone(),
            web_messages.clone(),
            web_datagrams.clone(),
        );
        let thread = thread::spawn(move || {
            let identity = ServerIdentity::self_signed().unwrap();
            let mut net = NetServer::bind("127.0.0.1:0".parse().unwrap(), &identity).unwrap();
            let web = WebIdentity::generate(unix_now_s()).unwrap();
            let web_addr = net
                .listen_webtransport("127.0.0.1:0".parse().unwrap(), web, None)
                .unwrap();
            tx.send((
                net.local_addr(),
                web_addr,
                net.fingerprint().to_owned(),
                net.web_cert_hash().unwrap(),
            ))
            .unwrap();
            let mut game = GameServer::new(
                Route::test_loop(),
                ServerConfig {
                    bots,
                    ..Default::default()
                },
            );
            while !s.load(Ordering::SeqCst) {
                net.pump(&mut game);
                let out = game.tick();
                net.broadcast(&game, &out);
                r.store(game.session().rider_count(), Ordering::SeqCst);
                let c = net.counters(Transport::WebTransport);
                wm.store(c.messages_in, Ordering::SeqCst);
                wd.store(c.datagrams_in, Ordering::SeqCst);
                thread::sleep(Duration::from_secs_f64(SIM_DT));
            }
            net.shutdown();
        });
        let (quic, web, fingerprint, web_hash) = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        Self {
            quic,
            web,
            fingerprint,
            web_hash,
            riders,
            web_messages,
            web_datagrams,
            stop,
            thread: Some(thread),
        }
    }

    fn web_url(&self) -> String {
        format!("https://127.0.0.1:{}", self.web.port())
    }

    fn web_client(&self, options: ClientOptions) -> NetClient {
        let trust = ServerTrust::Pinned(parse_fingerprint(&self.web_hash).unwrap());
        NetClient::connect_web(&self.web_url(), &trust, options).unwrap()
    }

    fn quic_client(&self) -> NetClient {
        let trust = ServerTrust::Pinned(parse_fingerprint(&self.fingerprint).unwrap());
        NetClient::connect(self.quic, "localhost", &trust, ClientOptions::default()).unwrap()
    }

    fn riders(&self) -> usize {
        self.riders.load(Ordering::SeqCst)
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            t.join().unwrap();
        }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn hello(name: &str) -> Vec<u8> {
    encode(&client_message(C::Hello(v1::Hello {
        protocol_version: PROTOCOL_VERSION,
        display_name: name.into(),
        ..Default::default()
    })))
}

fn sample(seq: u32, power: f32) -> Vec<u8> {
    encode(&client_message(C::Sample(v1::SensorSample {
        seq,
        power_w: Some(power),
        ..Default::default()
    })))
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(10));
    }
}

#[derive(Default)]
struct Seen {
    rider: Option<RiderId>,
    snapshots: Vec<v1::Snapshot>,
    rejected: Option<String>,
}

impl Seen {
    fn take(&mut self, client: &NetClient) {
        for bytes in client.receive() {
            match decode_server(&bytes).unwrap() {
                S::Welcome(w) => self.rider = Some(RiderId(w.rider_id)),
                S::Snapshot(s) => self.snapshots.push(s),
                S::Reject(r) => self.rejected = Some(r.reason),
                _ => {}
            }
        }
    }

    fn names(&self) -> Vec<String> {
        self.snapshots
            .last()
            .map(|s| s.riders.iter().map(|r| r.display_name.clone()).collect())
            .unwrap_or_default()
    }

    fn own(&self) -> Option<&v1::RiderUpdate> {
        let id = self.rider?;
        self.snapshots
            .last()?
            .riders
            .iter()
            .find(|r| r.rider_id == id.0)
    }
}

/// A WebTransport client joins, its samples (datagrams after the Welcome)
/// reach the server, and it rides; snapshots come back as datagrams.
#[test]
fn a_webtransport_client_joins_and_rides() {
    let server = TestServer::start(2);
    let c = server.web_client(ClientOptions::default());
    c.send(hello("Browser"));
    let mut seen = Seen::default();
    wait_until("the welcome", || {
        seen.take(&c);
        seen.rider.is_some()
    });
    assert_eq!(c.status(), LinkStatus::Connected { datagrams: true });
    assert_eq!(c.transport(), Some(Transport::WebTransport));
    assert!(c.rtt().is_some());
    let mut seq = 0;
    wait_until("riding at 300 W", || {
        seq += 1;
        c.send(sample(seq, 300.0));
        thread::sleep(Duration::from_millis(50));
        seen.take(&c);
        seen.own()
            .is_some_and(|r| r.power_w == 300 && r.distance_cm > 100 && r.last_input_seq > 5)
    });
    // The two bots and the browser rider.
    assert_eq!(server.riders(), 3);
    assert!(seen.names().contains(&"Browser".to_owned()));
    assert!(seen.snapshots.len() >= 5);
    // The samples after the Welcome came as datagrams.
    assert!(server.web_datagrams.load(Ordering::SeqCst) > 3);
    assert!(server.web_messages.load(Ordering::SeqCst) > seq as u64 / 2);
    c.close();
    wait_until("the rider to leave", || server.riders() == 2);
}

/// A native QUIC rider and a WebTransport rider in one session see each
/// other (the browser-and-desktop interop of B3).
#[test]
fn quic_and_webtransport_riders_share_a_session() {
    let server = TestServer::start(0);
    let desktop = server.quic_client();
    let browser = server.web_client(ClientOptions::default());
    desktop.send(hello("Desktop"));
    browser.send(hello("Browser"));
    let (mut d, mut b) = (Seen::default(), Seen::default());
    let mut seq = 0;
    wait_until("both riders in both views", || {
        seq += 1;
        desktop.send(sample(seq, 200.0));
        browser.send(sample(seq, 250.0));
        thread::sleep(Duration::from_millis(50));
        d.take(&desktop);
        b.take(&browser);
        let both = |s: &Seen| {
            let n = s.names();
            n.contains(&"Desktop".to_owned()) && n.contains(&"Browser".to_owned())
        };
        both(&d) && both(&b)
    });
    wait_until("both moving", || {
        seq += 1;
        desktop.send(sample(seq, 200.0));
        browser.send(sample(seq, 250.0));
        thread::sleep(Duration::from_millis(50));
        d.take(&desktop);
        b.take(&browser);
        d.own()
            .is_some_and(|r| r.power_w == 200 && r.distance_cm > 50)
            && b.own()
                .is_some_and(|r| r.power_w == 250 && r.distance_cm > 50)
    });
    // The browser sees the desktop rider's power and vice versa.
    let other = |s: &Seen, name: &str| {
        s.snapshots
            .last()
            .and_then(|x| x.riders.iter().find(|r| r.display_name == name))
            .map(|r| r.power_w)
    };
    assert_eq!(other(&b, "Desktop"), Some(200));
    assert_eq!(other(&d, "Browser"), Some(250));
    assert_eq!(server.riders(), 2);
}

/// Without datagrams (the peer accepts none) everything goes on the
/// stream, and the rider still rides.
#[test]
fn webtransport_without_datagrams_falls_back_to_the_stream() {
    let server = TestServer::start(0);
    let c = server.web_client(ClientOptions { datagrams: false });
    c.send(hello("Stream"));
    let mut seen = Seen::default();
    let mut seq = 0;
    wait_until("riding on the stream only", || {
        seq += 1;
        c.send(sample(seq, 220.0));
        thread::sleep(Duration::from_millis(50));
        seen.take(&c);
        seen.own()
            .is_some_and(|r| r.power_w == 220 && r.distance_cm > 50)
    });
    assert_eq!(c.status(), LinkStatus::Connected { datagrams: false });
    assert!(seen.snapshots.len() >= 3);
}

/// A wrong certificate hash fails the handshake: the link closes with a
/// reason, no rider joins.
#[test]
fn a_wrong_certificate_hash_is_refused() {
    let server = TestServer::start(0);
    let other = WebIdentity::generate(unix_now_s()).unwrap().cert_hash();
    let trust = ServerTrust::Pinned(parse_fingerprint(&other).unwrap());
    let c = NetClient::connect_web(&server.web_url(), &trust, ClientOptions::default()).unwrap();
    c.send(hello("Mallory"));
    wait_until("the refusal", || {
        matches!(c.status(), LinkStatus::Closed(_))
    });
    let LinkStatus::Closed(reason) = c.status() else {
        unreachable!()
    };
    assert!(reason.contains("cannot connect"), "{reason}");
    thread::sleep(Duration::from_millis(200));
    assert_eq!(server.riders(), 0);
    // The native certificate is not the WebTransport one either.
    let trust = ServerTrust::Pinned(parse_fingerprint(&server.fingerprint).unwrap());
    let c = NetClient::connect_web(&server.web_url(), &trust, ClientOptions::default()).unwrap();
    wait_until("the refusal", || {
        matches!(c.status(), LinkStatus::Closed(_))
    });
}

/// A server shutdown reaches WebTransport clients with its reason.
#[test]
fn a_shutdown_reaches_webtransport_clients() {
    let mut server = TestServer::start(0);
    let c = server.web_client(ClientOptions::default());
    c.send(hello("Browser"));
    let mut seen = Seen::default();
    wait_until("the welcome", || {
        seen.take(&c);
        seen.rider.is_some()
    });
    server.stop();
    wait_until("the close", || matches!(c.status(), LinkStatus::Closed(_)));
    let LinkStatus::Closed(reason) = c.status() else {
        unreachable!()
    };
    assert!(reason.contains("server shutting down"), "{reason}");
    assert!(seen.rejected.is_none());
}

/// The rotation renews the certificate when it nears its end: new
/// sessions need the new hash, the session already open goes on.
#[test]
fn the_certificate_rotates_while_running() {
    let identity = ServerIdentity::self_signed().unwrap();
    let mut net = NetServer::bind("127.0.0.1:0".parse().unwrap(), &identity).unwrap();
    let now = unix_now_s();
    let web = WebIdentity::generate(now).unwrap();
    let addr = net
        .listen_webtransport("127.0.0.1:0".parse().unwrap(), web, None)
        .unwrap();
    let first = net.web_cert_hash().unwrap();
    assert_eq!(net.rotate_web_cert(now).unwrap(), None, "not due yet");
    let end = net.web_cert_not_after().unwrap();
    let mut game = GameServer::new(Route::test_loop(), ServerConfig::default());
    let url = format!("https://127.0.0.1:{}", addr.port());
    let pin = |h: &str| ServerTrust::Pinned(parse_fingerprint(h).unwrap());
    let old = NetClient::connect_web(&url, &pin(&first), ClientOptions::default()).unwrap();
    old.send(hello("Old"));
    let mut seen = Seen::default();
    let pump = |net: &mut NetServer, game: &mut GameServer| {
        net.pump(game);
        let out = game.tick();
        net.broadcast(game, &out);
        thread::sleep(Duration::from_secs_f64(SIM_DT));
    };
    wait_until("the first welcome", || {
        pump(&mut net, &mut game);
        seen.take(&old);
        seen.rider.is_some()
    });
    // Two days before the end: a new certificate.
    let rotated = net.rotate_web_cert(end - 2 * 86_400).unwrap().unwrap();
    assert_ne!(rotated, first);
    assert_eq!(net.web_cert_hash().unwrap(), rotated);
    let stale = NetClient::connect_web(&url, &pin(&first), ClientOptions::default()).unwrap();
    let fresh = NetClient::connect_web(&url, &pin(&rotated), ClientOptions::default()).unwrap();
    fresh.send(hello("New"));
    let mut seen_new = Seen::default();
    wait_until("the stale hash refused, the new one joined", || {
        pump(&mut net, &mut game);
        seen_new.take(&fresh);
        seen.take(&old);
        matches!(stale.status(), LinkStatus::Closed(_)) && seen_new.rider.is_some()
    });
    // The old session is still there.
    assert!(matches!(old.status(), LinkStatus::Connected { .. }));
    assert_eq!(game.session().rider_count(), 2);
    net.shutdown();
}
