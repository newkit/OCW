//! Road vehicles over QUIC on 127.0.0.1 (spec 14 Phase 18, 18E T5): a
//! server thread runs the game loop with busy road traffic on a made-up
//! town, two clients connect with [`NetClient`], follow the vehicles from
//! their `TrafficUpdate`s with the sim's `PlanFollower` and see the same
//! vehicles at the same tick within 0.2 m; a third client that connects
//! late gets the current plans at once.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use ocw_core::SIM_DT;
use ocw_core::traffic::RoadTraffic;
use ocw_protocol::v1::{self, client_message::Msg as C, server_message::Msg as S};
use ocw_protocol::vehicles::{change_from_wire, spawn_from_wire};
use ocw_protocol::{PROTOCOL_VERSION, client_message, decode_server, encode};
use ocw_server::net::tls::parse_fingerprint;
use ocw_server::net::{ClientOptions, NetClient, NetServer, ServerIdentity, ServerTrust};
use ocw_server::{Course, GameServer, ServerConfig};
use ocw_sim::traffic::{Curves, PlanFollower, Tick};
use ocw_world::RoadNetwork;
use ocw_world::graph::{Edge, EdgeId, Oneway, RoadClass, RoadGraph, VertexId};

/// A made-up town: a 9 × 9 grid of 250 m blocks of secondary roads.
fn town() -> Arc<RoadNetwork> {
    let n = 9u32;
    let mut vertices = Vec::new();
    for row in 0..n {
        for col in 0..n {
            vertices.push([f64::from(col) * 250.0, f64::from(row) * 250.0]);
        }
    }
    let mut edges = Vec::new();
    for row in 0..n {
        for col in 0..n {
            let i = row * n + col;
            for (j, ok) in [(i + 1, col + 1 < n), (i + n, row + 1 < n)] {
                if !ok {
                    continue;
                }
                let k = edges.len();
                edges.push(Edge {
                    id: EdgeId(3000 + k as u64),
                    from: VertexId(i),
                    to: VertexId(j),
                    points: vec![vertices[i as usize], vertices[j as usize]],
                    class: RoadClass::Secondary,
                    name: Some(format!("Bea Road {k}")),
                    width_m: 7.0,
                    bridge: false,
                    tunnel: false,
                    dead_end_m: None,
                    boundary: false,
                    oneway: Oneway::No,
                    osm_way: k as i64,
                    profile: Vec::new(),
                });
            }
        }
    }
    Arc::new(RoadNetwork::new(RoadGraph {
        vertices,
        edges,
        boundary_vertices: Default::default(),
    }))
}

struct TestServer {
    addr: SocketAddr,
    fingerprint: String,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl TestServer {
    fn start(net: Arc<RoadNetwork>) -> Self {
        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let s = stop.clone();
        let thread = thread::spawn(move || {
            let identity = ServerIdentity::self_signed().unwrap();
            let mut net_server =
                NetServer::bind("127.0.0.1:0".parse().unwrap(), &identity).unwrap();
            tx.send((net_server.local_addr(), net_server.fingerprint().to_owned()))
                .unwrap();
            let mut game = GameServer::new(
                Course::FreeRide {
                    net,
                    start: ocw_sim::FreeRideStart {
                        edge: 40,
                        offset_m: 10.0,
                        forward: true,
                    },
                },
                ServerConfig {
                    road_traffic: RoadTraffic::Busy,
                    // Wednesday 10 June 2026, 17:30 in Vaduz.
                    real_epoch_ms: Some(1_781_105_400_000),
                    air_seed: Some(5),
                    ..Default::default()
                },
            );
            while !s.load(Ordering::SeqCst) {
                net_server.pump(&mut game);
                let out = game.tick();
                net_server.broadcast(&game, &out);
                net_server.send_directed(game.take_directed(), None);
                thread::sleep(Duration::from_secs_f64(SIM_DT));
            }
            net_server.shutdown();
        });
        let (addr, fingerprint) = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        Self {
            addr,
            fingerprint,
            stop,
            thread: Some(thread),
        }
    }

    fn client(&self) -> NetClient {
        let trust = ServerTrust::Pinned(parse_fingerprint(&self.fingerprint).unwrap());
        NetClient::connect(self.addr, "localhost", &trust, ClientOptions::default()).unwrap()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            t.join().unwrap();
        }
    }
}

fn hello(name: &str) -> Vec<u8> {
    encode(&client_message(C::Hello(v1::Hello {
        protocol_version: PROTOCOL_VERSION,
        display_name: name.into(),
        profile: Some(ocw_core::RiderProfile::default().into()),
        ..Default::default()
    })))
}

/// A minimal vehicle client: the followers from the updates.
struct Follow {
    net: Arc<RoadNetwork>,
    curves: Curves,
    followers: BTreeMap<u32, PlanFollower>,
    tick: Tick,
    updates: usize,
    resets: usize,
}

impl Follow {
    fn new(net: Arc<RoadNetwork>) -> Self {
        Self {
            curves: Curves::new(&net),
            net,
            followers: BTreeMap::new(),
            tick: 0,
            updates: 0,
            resets: 0,
        }
    }

    fn take(&mut self, client: &NetClient) {
        for bytes in client.receive() {
            let Ok(S::Traffic(u)) = decode_server(&bytes) else {
                continue;
            };
            assert_eq!(u.graph_hash, self.net.fingerprint());
            self.updates += 1;
            if u.reset {
                self.resets += 1;
                self.followers.clear();
            }
            self.tick = u.tick;
            for id in &u.despawns {
                self.followers.remove(id);
            }
            for c in &u.changes {
                let c = change_from_wire(c, u.tick).unwrap();
                if let Some(f) = self.followers.get_mut(&c.id) {
                    f.apply(&c);
                }
            }
            for s in &u.spawns {
                let p = spawn_from_wire(s, u.tick).unwrap();
                self.followers.insert(p.id, PlanFollower::new(&p));
            }
        }
    }

    fn at(&mut self, tick: Tick) -> BTreeMap<u32, [f64; 2]> {
        let (net, curves) = (&*self.net, &self.curves);
        self.followers
            .iter_mut()
            .filter_map(|(id, f)| {
                f.advance_to(net, curves, tick, 0.02);
                Some((*id, f.point(net)?))
            })
            .collect()
    }
}

fn wait_until(what: &str, secs: u64, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn two_clients_over_quic_see_the_same_vehicles() {
    let net = town();
    let server = TestServer::start(net.clone());
    let (ada, bea) = (server.client(), server.client());
    ada.send(hello("Ada Rider"));
    bea.send(hello("Bea Climber"));
    let (mut fa, mut fb) = (Follow::new(net.clone()), Follow::new(net.clone()));
    // Twenty seconds of busy traffic.
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_secs(20) {
        fa.take(&ada);
        fb.take(&bea);
        thread::sleep(Duration::from_millis(20));
    }
    wait_until("both at the same update", 10, || {
        fa.take(&ada);
        fb.take(&bea);
        fa.tick == fb.tick && fa.tick > 0
    });
    let tick = fa.tick + 25;
    let (pa, pb) = (fa.at(tick), fb.at(tick));
    assert!(pa.len() > 20, "{} vehicles", pa.len());
    let mut worst: f64 = 0.0;
    let mut common = 0;
    for (id, a) in &pa {
        if let Some(b) = pb.get(id) {
            common += 1;
            worst = worst.max(((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt());
        }
    }
    assert!(common * 10 >= pa.len() * 9, "{common} of {}", pa.len());
    assert!(worst <= 0.2, "the clients differ by {worst:.3} m");
    assert_eq!(fa.resets, 1);
    // A late client: the current plans with its first update.
    let cleo = server.client();
    cleo.send(hello("Cleo Sprinter"));
    let mut fc = Follow::new(net.clone());
    wait_until("the late client's plans", 10, || {
        fc.take(&cleo);
        !fc.followers.is_empty()
    });
    assert_eq!(fc.resets, 1);
    assert!(fc.followers.len() > 20);
}
