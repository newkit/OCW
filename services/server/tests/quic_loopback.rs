//! The QUIC transport end to end on 127.0.0.1 (ephemeral ports, no other
//! network): a server thread runs the game loop like the `ocw-server`
//! binary, clients connect with [`NetClient`].

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use ocw_core::{RiderId, SIM_DT};
use ocw_protocol::v1::{self, client_message::Msg as C, server_message::Msg as S};
use ocw_protocol::{PROTOCOL_VERSION, client_message, decode_server, encode};
use ocw_server::net::tls::parse_fingerprint;
use ocw_server::net::{
    ClientOptions, LinkStatus, NetClient, NetServer, ServerIdentity, ServerTrust,
};
use ocw_server::{GameServer, ServerConfig};
use ocw_world::Route;

struct TestServer {
    addr: SocketAddr,
    fingerprint: String,
    riders: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl TestServer {
    fn start(bots: u32) -> Self {
        let (tx, rx) = mpsc::channel();
        let riders = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (r, s) = (riders.clone(), stop.clone());
        let thread = thread::spawn(move || {
            let identity = ServerIdentity::self_signed().unwrap();
            let mut net = NetServer::bind("127.0.0.1:0".parse().unwrap(), &identity).unwrap();
            tx.send((net.local_addr(), net.fingerprint().to_owned()))
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
                thread::sleep(Duration::from_secs_f64(SIM_DT));
            }
            net.shutdown();
        });
        let (addr, fingerprint) = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        Self {
            addr,
            fingerprint,
            riders,
            stop,
            thread: Some(thread),
        }
    }

    fn trust(&self) -> ServerTrust {
        ServerTrust::Pinned(parse_fingerprint(&self.fingerprint).unwrap())
    }

    fn client(&self, options: ClientOptions) -> NetClient {
        NetClient::connect(self.addr, "localhost", &self.trust(), options).unwrap()
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
        profile: Some(ocw_core::RiderProfile::default().into()),
        world: None,
        rider_weight_kg: None,
        bike_profile: None,
        look: None,
        parts: Vec::new(),
        damage: None,
        access_token: None,
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
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(10));
    }
}

/// What a test client has seen.
#[derive(Default)]
struct Seen {
    rider: Option<RiderId>,
    snapshots: Vec<v1::Snapshot>,
}

impl Seen {
    fn take(&mut self, client: &NetClient) {
        for bytes in client.receive() {
            match decode_server(&bytes).unwrap() {
                S::Welcome(w) => self.rider = Some(RiderId(w.rider_id)),
                S::Snapshot(s) => self.snapshots.push(s),
                S::Reject(r) => panic!("rejected: {}", r.reason),
                _ => {}
            }
        }
    }

    fn last_riders(&self) -> Vec<u32> {
        self.snapshots
            .last()
            .map(|s| s.riders.iter().map(|r| r.rider_id).collect())
            .unwrap_or_default()
    }
}

#[test]
fn client_joins_rides_and_leaves() {
    let server = TestServer::start(2);
    let client = server.client(ClientOptions::default());
    client.send(hello("Remote"));
    let mut seen = Seen::default();
    wait_until("welcome", || {
        seen.take(&client);
        seen.rider.is_some()
    });
    assert_eq!(client.status(), LinkStatus::Connected { datagrams: true });
    assert!(client.rtt().is_some());
    // Pedal for a while: samples (datagrams) reach the server physics.
    let own = seen.rider.unwrap().0;
    let mut seq = 0;
    wait_until("own rider moving", || {
        seq += 1;
        client.send(sample(seq, 300.0));
        thread::sleep(Duration::from_millis(40));
        seen.take(&client);
        seen.snapshots.last().is_some_and(|s| {
            s.riders
                .iter()
                .any(|r| r.rider_id == own && r.distance_cm > 300)
        })
    });
    assert_eq!(seen.last_riders().len(), 3, "two bots and us");
    // Snapshots arrive in order of server ticks here (one host, no loss).
    assert!(seen.snapshots.len() >= 5);
    assert_eq!(server.riders(), 3);
    client.close();
    wait_until("rider removed", || server.riders() == 2);
}

#[test]
fn two_clients_see_each_other() {
    let server = TestServer::start(0);
    let a = server.client(ClientOptions::default());
    let b = server.client(ClientOptions::default());
    a.send(hello("A"));
    b.send(hello("B"));
    let (mut sa, mut sb) = (Seen::default(), Seen::default());
    wait_until("both see two riders", || {
        sa.take(&a);
        sb.take(&b);
        sa.last_riders().len() == 2 && sb.last_riders().len() == 2
    });
    let (ida, idb) = (sa.rider.unwrap().0, sb.rider.unwrap().0);
    assert_ne!(ida, idb);
    for seen in [&sa, &sb] {
        let mut ids = seen.last_riders();
        ids.sort();
        let mut want = vec![ida, idb];
        want.sort();
        assert_eq!(ids, want);
    }
    // A leaves: B's snapshots drop it.
    drop(a);
    wait_until("B sees only itself", || {
        sb.take(&b);
        sb.last_riders() == vec![idb]
    });
}

#[test]
fn snapshots_fall_back_to_the_stream_without_datagrams() {
    let server = TestServer::start(1);
    let client = server.client(ClientOptions { datagrams: false });
    client.send(hello("NoDatagrams"));
    let mut seen = Seen::default();
    wait_until("snapshots on the stream", || {
        seen.take(&client);
        seen.rider.is_some() && seen.snapshots.len() >= 3
    });
    assert_eq!(client.status(), LinkStatus::Connected { datagrams: false });
    // On the stream they are strictly in order.
    let ticks: Vec<u64> = seen.snapshots.iter().map(|s| s.server_tick).collect();
    assert!(ticks.windows(2).all(|w| w[0] < w[1]), "{ticks:?}");
}

#[test]
fn a_wrong_certificate_pin_is_refused() {
    let server = TestServer::start(0);
    let client = NetClient::connect(
        server.addr,
        "localhost",
        &ServerTrust::Pinned([0; 32]),
        ClientOptions::default(),
    )
    .unwrap();
    client.send(hello("Mallory"));
    wait_until("handshake failure", || {
        matches!(client.status(), LinkStatus::Closed(_))
    });
    let LinkStatus::Closed(reason) = client.status() else {
        unreachable!()
    };
    assert!(reason.contains("does not match"), "{reason}");
    assert!(client.receive().is_empty());
    assert_eq!(server.riders(), 0);
}

#[test]
fn the_dev_trust_mode_connects_on_loopback() {
    let server = TestServer::start(0);
    let trust = ServerTrust::for_server(server.addr, None, true).unwrap();
    let client =
        NetClient::connect(server.addr, "127.0.0.1", &trust, ClientOptions::default()).unwrap();
    client.send(hello("Dev"));
    let mut seen = Seen::default();
    wait_until("welcome", || {
        seen.take(&client);
        seen.rider.is_some()
    });
}

#[test]
fn server_shutdown_reaches_the_client() {
    let mut server = TestServer::start(0);
    let client = server.client(ClientOptions::default());
    client.send(hello("Last"));
    let mut seen = Seen::default();
    wait_until("welcome", || {
        seen.take(&client);
        seen.rider.is_some()
    });
    server.stop();
    wait_until("closed", || {
        matches!(client.status(), LinkStatus::Closed(_))
    });
    let LinkStatus::Closed(reason) = client.status() else {
        unreachable!()
    };
    assert!(reason.contains("server shutting down"), "{reason}");
}

/// Ride → Host a ride (spec §3.2.1): the app's local server with the
/// QUIC endpoint. The host's own connection is the first rider; another
/// rider joins over the network and sees the host's course, clock and
/// weather; stopping the host closes the others' link.
#[test]
fn a_hosted_local_server_takes_riders_over_the_network() {
    use ocw_server::local::LocalServer;
    let identity = ServerIdentity::self_signed().unwrap();
    let net = NetServer::bind("127.0.0.1:0".parse().unwrap(), &identity).unwrap();
    let (addr, fp) = (net.local_addr(), net.fingerprint().to_owned());
    let config = ServerConfig {
        weather: ocw_weather::presets::preset("fog"),
        ..ocw_server::local::local_config(2, 0.0)
    };
    let (host, counts) = LocalServer::spawn_hosting(Route::test_hills(), config, net).unwrap();
    host.to_server.send(hello("Host")).unwrap();
    let host_welcome = loop {
        match host.from_server.recv_timeout(Duration::from_secs(10)) {
            Ok(b) => {
                if let Ok(S::Welcome(w)) = decode_server(&b) {
                    break w;
                }
            }
            Err(e) => panic!("no welcome for the host: {e}"),
        }
    };
    let trust = ServerTrust::Pinned(parse_fingerprint(&fp).unwrap());
    let guest = NetClient::connect(addr, "localhost", &trust, ClientOptions::default()).unwrap();
    guest.send(hello("Guest"));
    let mut welcome = None;
    let mut seen = Seen::default();
    wait_until("the guest's welcome and a snapshot with both", || {
        for bytes in guest.receive() {
            match decode_server(&bytes).unwrap() {
                S::Welcome(w) => welcome = Some(w),
                S::Snapshot(s) => seen.snapshots.push(s),
                _ => {}
            }
        }
        welcome.is_some() && seen.last_riders().len() >= 4
    });
    let w = welcome.unwrap();
    // The host's course, clock and weather.
    assert_eq!(w.route_id, host_welcome.route_id);
    assert_eq!(w.route_id, Route::test_hills().id());
    assert!(w.clock.is_some());
    let fog = ocw_weather::presets::preset("fog").unwrap();
    let weather = w
        .weather
        .and_then(|s| s.now)
        .expect("the session's weather");
    assert!(
        (weather.fog_visibility_m - fog.fog_visibility_m).abs() < 1.0,
        "{weather:?}"
    );
    assert!(seen.last_riders().contains(&host_welcome.rider_id));
    wait_until("counts", || {
        counts.riders.load(Ordering::Relaxed) == 2 && counts.peers.load(Ordering::Relaxed) == 1
    });
    // Stop hosting: the guest hears it.
    drop(host);
    wait_until("closed", || matches!(guest.status(), LinkStatus::Closed(_)));
    let LinkStatus::Closed(reason) = guest.status() else {
        unreachable!()
    };
    assert!(reason.contains("server shutting down"), "{reason}");
}

/// A second server on a port in use fails plainly (the host offers to
/// join the one there instead).
#[test]
fn a_port_in_use_is_reported() {
    let identity = ServerIdentity::self_signed().unwrap();
    let a = NetServer::bind("127.0.0.1:0".parse().unwrap(), &identity).unwrap();
    let e = NetServer::bind(a.local_addr(), &identity)
        .err()
        .expect("in use");
    assert!(e.contains("cannot listen on"), "{e}");
}
