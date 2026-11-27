//! Road vehicles end to end (spec 14 Phase 18, 18E T5): the in-memory
//! server simulates them while the road traffic is on and sends each
//! client the plans of the vehicles near its rider and their changes;
//! two clients see the same vehicles, a client that comes back gets the
//! current plans, only vehicles within the interest radius are sent, no
//! road traffic sends nothing, the browser ignores them, and the data
//! stay under 2 kB/s with about 100 vehicles near a rider.

use std::collections::{BTreeMap, BTreeSet};

use super::*;
use ocw_core::traffic::{DriverManners, RoadTraffic};
use ocw_server::vehicles::{INTEREST_HYSTERESIS_M, INTEREST_M};
use ocw_world::graph::{Edge, EdgeId, Oneway, RoadClass, RoadGraph, VertexId};

/// Wednesday 10 June 2026, 17:30 in Vaduz (the evening peak).
const PEAK_MS: i64 = 1_781_105_400_000;

/// A made-up town: a 13 × 13 grid of 250 m blocks, secondary roads every
/// fourth row and column, tertiary ones between (3 km across).
fn town() -> Arc<RoadNetwork> {
    let n = 13u32;
    let block = 250.0;
    let mut vertices = Vec::new();
    for row in 0..n {
        for col in 0..n {
            vertices.push([f64::from(col) * block, f64::from(row) * block]);
        }
    }
    let mut edges = Vec::new();
    let mut add = |a: u32, b: u32, major: bool| {
        let k = edges.len();
        edges.push(Edge {
            id: EdgeId(7000 + k as u64),
            from: VertexId(a),
            to: VertexId(b),
            points: vec![vertices[a as usize], vertices[b as usize]],
            class: if major {
                RoadClass::Secondary
            } else {
                RoadClass::Tertiary
            },
            name: Some(format!("Ada Street {k}")),
            width_m: if major { 7.5 } else { 6.0 },
            bridge: false,
            tunnel: false,
            dead_end_m: None,
            boundary: false,
            oneway: Oneway::No,
            osm_way: k as i64,
            profile: Vec::new(),
        });
    };
    for row in 0..n {
        for col in 0..n {
            let i = row * n + col;
            if col + 1 < n {
                add(i, i + 1, row % 4 == 0);
            }
            if row + 1 < n {
                add(i, i + n, col % 4 == 0);
            }
        }
    }
    Arc::new(
        RoadNetwork::new(RoadGraph {
            vertices,
            edges,
            boundary_vertices: Default::default(),
        })
        .with_package_name("Ada Town"),
    )
}

/// A server with clients on the town, stepped tick by tick.
struct Ride {
    net: Arc<RoadNetwork>,
    server: GameServer,
    clients: Vec<(ocw_server::ConnectionId, ClientCore)>,
    t: f64,
    /// Traffic messages each client received.
    traffic_msgs: Vec<u64>,
}

impl Ride {
    fn new(level: RoadTraffic) -> Self {
        let net = town();
        // The middle of the town, on a secondary road eastwards.
        let start = net
            .graph()
            .edges
            .iter()
            .position(|e| {
                e.class == RoadClass::Secondary
                    && e.points[0] == [1500.0, 1000.0]
                    && e.points[1][0] > 1500.0
            })
            .unwrap();
        let server = GameServer::new(
            ocw_server::Course::FreeRide {
                net: net.clone(),
                start: ocw_sim::FreeRideStart {
                    edge: start,
                    offset_m: 20.0,
                    forward: true,
                },
            },
            ServerConfig {
                road_traffic: level,
                driver_manners: DriverManners::Polite,
                real_epoch_ms: Some(PEAK_MS),
                air_seed: Some(77),
                ..Default::default()
            },
        );
        Self {
            net,
            server,
            clients: Vec::new(),
            t: 0.0,
            traffic_msgs: Vec::new(),
        }
    }

    fn join(&mut self, name: &str) -> usize {
        let conn = self.server.connect();
        let mut c = ClientCore::free_ride(self.net.clone(), name, RiderProfile::default());
        for reply in self.server.handle(conn, &c.hello()) {
            c.on_server_message(&reply, self.t);
        }
        assert!(c.rider_id().is_some(), "{name} joined");
        self.clients.push((conn, c));
        self.traffic_msgs.push(0);
        self.clients.len() - 1
    }

    fn leave(&mut self, i: usize) {
        let (conn, _) = self.clients.remove(i);
        self.traffic_msgs.remove(i);
        self.server.disconnect(conn);
    }

    /// One 50 Hz tick of everything; the riders ride at `watts`.
    fn tick(&mut self, watts: f64) {
        let quarter = (self.t * 4.0).fract() < 1e-9;
        for (conn, c) in &mut self.clients {
            if quarter && let Some(s) = c.on_device_event(power(watts, self.t)) {
                self.server.handle(*conn, &s);
            }
        }
        let out = self.server.tick();
        let directed = self.server.take_directed();
        for (k, (conn, c)) in self.clients.iter_mut().enumerate() {
            for b in &out {
                c.on_server_message(b, self.t);
            }
            for (to, b) in &directed {
                if to == conn {
                    if matches!(decode_server(b), Ok(S::Traffic(_))) {
                        self.traffic_msgs[k] += 1;
                    }
                    c.on_server_message(b, self.t);
                }
            }
            c.predict_step();
        }
        self.t += ocw_core::SIM_DT;
        self.t = (self.t * 1000.0).round() / 1000.0;
    }

    fn run(&mut self, seconds: f64, watts: f64) {
        for _ in 0..(seconds * 50.0).round() as u32 {
            self.tick(watts);
        }
    }

    /// Runs on to just after a broadcast (every client has every event).
    fn until_broadcast(&mut self, watts: f64) {
        self.tick(watts);
        while !self
            .server
            .session()
            .tick()
            .is_multiple_of(u64::from(ocw_core::TICKS_PER_BROADCAST))
        {
            self.tick(watts);
        }
    }

    /// The server's vehicles now (id → x, z).
    fn server_positions(&self) -> BTreeMap<u32, [f64; 2]> {
        self.server
            .traffic_sim()
            .map(|s| s.views(&self.net).iter().map(|v| (v.id, v.xz)).collect())
            .unwrap_or_default()
    }

    fn sim_tick(&self) -> u64 {
        self.server.traffic_sim().map_or(0, |s| s.tick())
    }

    /// Where client `i`'s rider is on the server (x, z).
    fn rider_xz(&self, i: usize) -> [f64; 2] {
        let id = self.clients[i].1.rider_id().unwrap();
        let s = self.server.session();
        let d = s.rider_state(id).unwrap().distance;
        let p = s.track(id).sample(d).position;
        [p[0], p[2]]
    }
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// Two clients of one session see the same vehicles at the same tick,
/// and each within the server's tolerance of the server's own.
#[test]
fn two_clients_see_the_same_vehicles() {
    let mut r = Ride::new(RoadTraffic::Busy);
    let ada = r.join("Ada Rider");
    let bea = r.join("Bea Climber");
    r.run(60.0, 200.0);
    r.until_broadcast(200.0);
    let tick = r.sim_tick();
    let a = r.clients[ada].1.vehicles().positions_at(tick);
    let b = r.clients[bea].1.vehicles().positions_at(tick);
    assert!(a.len() > 20, "vehicles near the riders: {}", a.len());
    let server = r.server_positions();
    let mut worst_ab: f64 = 0.0;
    let mut worst_server: f64 = 0.0;
    for (id, pa) in &a {
        // The riders ride together: the same vehicles are near both.
        if let Some(pb) = b.get(id) {
            worst_ab = worst_ab.max(dist(*pa, *pb));
        }
        let ps = server.get(id).expect("a vehicle the server has");
        worst_server = worst_server.max(dist(*pa, *ps));
    }
    let common = a.keys().filter(|id| b.contains_key(id)).count();
    assert!(common * 10 >= a.len() * 9, "{common} of {}", a.len());
    assert!(worst_ab <= 0.2, "the clients differ by {worst_ab:.3} m");
    assert!(
        worst_server <= ocw_sim::traffic::plan::POS_TOLERANCE_M + 0.05,
        "a client differs from the server by {worst_server:.3} m"
    );
    // The renderer's view: every vehicle placed, headings unit.
    let view = r.clients[ada].1.vehicles_view(r.t);
    assert_eq!(view.len(), a.len());
    for v in &view {
        assert!(v.pos.iter().all(|x| x.is_finite()));
        let h = (v.heading[0].powi(2) + v.heading[1].powi(2)).sqrt();
        assert!((h - 1.0).abs() < 1e-6);
    }
    // Corrections are blended, and none was a jump.
    let st = r.clients[ada].1.vehicles().stats();
    assert_eq!(st.snapped, 0, "{st:?}");
    assert!(st.changes > 0 && st.spawns > 0, "{st:?}");
}

/// A client that drops and connects again gets a reset with the current
/// plans: at once the same vehicles as the server.
#[test]
fn a_reconnecting_client_gets_the_current_plans() {
    let mut r = Ride::new(RoadTraffic::Normal);
    r.join("Ada Rider");
    r.join("Bea Climber");
    r.run(30.0, 0.0);
    r.leave(1);
    r.run(10.0, 0.0);
    let bea = r.join("Bea Climber");
    r.until_broadcast(0.0);
    let c = &r.clients[bea].1;
    assert_eq!(c.vehicles().stats().resets, 1, "{:?}", c.vehicles().stats());
    let tick = r.sim_tick();
    let mine = c.vehicles().positions_at(tick);
    let server = r.server_positions();
    assert!(!mine.is_empty());
    for (id, p) in &mine {
        let d = dist(*p, server[id]);
        assert!(
            d <= ocw_sim::traffic::plan::POS_TOLERANCE_M + 0.05,
            "{id}: {d:.3} m"
        );
    }
    // Every vehicle in range came with the reset.
    let me = r.rider_xz(bea);
    for (id, p) in &server {
        if dist(*p, me) < INTEREST_M - 50.0 {
            assert!(
                mine.contains_key(id),
                "vehicle {id} near the rider is missing"
            );
        }
    }
}

/// Interest management: a client has exactly the vehicles near its rider
/// (in within 1.5 km, out beyond 1.6 km), however many the server runs.
#[test]
fn a_client_gets_only_the_vehicles_within_its_radius() {
    let mut r = Ride::new(RoadTraffic::Busy);
    let ada = r.join("Ada Rider");
    for _ in 0..6 {
        r.run(10.0, 250.0);
        r.until_broadcast(250.0);
        let me = r.rider_xz(ada);
        let server = r.server_positions();
        let known: BTreeSet<u32> = r
            .server
            .traffic_known(r.clients[ada].0)
            .into_iter()
            .collect();
        for (id, p) in &server {
            let d = dist(*p, me);
            if d <= INTEREST_M {
                assert!(known.contains(id), "{id} at {d:.0} m not sent");
            }
            if known.contains(id) {
                assert!(
                    d <= INTEREST_M + INTEREST_HYSTERESIS_M,
                    "{id} at {d:.0} m kept"
                );
            }
        }
        // The client holds the same set.
        assert_eq!(r.clients[ada].1.vehicles().len(), known.len());
    }
    // The bubble's margin: the server has vehicles the client never gets.
    let me = r.rider_xz(ada);
    assert!(
        r.server_positions()
            .values()
            .any(|p| dist(*p, me) > INTEREST_M + INTEREST_HYSTERESIS_M),
        "vehicles beyond the radius exist on the server"
    );
}

/// The data stay small (18A: 1 – 2 kB/s for 100 vehicles near a rider):
/// measured at the evening peak with normal and busy road traffic.
#[test]
fn about_a_hundred_vehicles_near_a_rider_cost_under_2_kb_a_second() {
    for level in [RoadTraffic::Normal, RoadTraffic::Busy] {
        let mut r = Ride::new(level);
        let ada = r.join("Ada Rider");
        // Fill the bubble, then measure two minutes.
        r.run(60.0, 200.0);
        let conn = r.clients[ada].0;
        let before = r.server.traffic_sent(conn);
        let mut near = 0usize;
        let secs = 120.0;
        for _ in 0..(secs as u32) {
            r.run(1.0, 200.0);
            near += r.server.traffic_known(conn).len();
        }
        let after = r.server.traffic_sent(conn);
        let bytes_s = (after.bytes - before.bytes) as f64 / secs;
        let msgs_s = (after.messages - before.messages) as f64 / secs;
        let near = near as f64 / secs;
        eprintln!(
            "traffic bandwidth ({level:?}): {near:.0} vehicles near the rider, {bytes_s:.0} B/s, \
             {msgs_s:.1} messages/s, {:.2} spawns/s, {:.2} changes/s, {:.2} despawns/s; \
             the first minute with the initial fill {} B",
            (after.spawns - before.spawns) as f64 / secs,
            (after.changes - before.changes) as f64 / secs,
            (after.despawns - before.despawns) as f64 / secs,
            before.bytes
        );
        assert!(near >= 90.0, "{near:.0} vehicles near the rider");
        assert!(bytes_s < 2000.0, "{bytes_s:.0} B/s");
    }
}

/// No road traffic: no vehicle, no message; switched on and off again,
/// the client gets them and loses them.
#[test]
fn no_road_traffic_sends_nothing() {
    let mut r = Ride::new(RoadTraffic::Off);
    let ada = r.join("Ada Rider");
    r.run(20.0, 200.0);
    assert_eq!(r.server.vehicle_count(), 0);
    assert!(r.server.traffic_sim().is_none(), "no simulation, no cost");
    assert_eq!(r.traffic_msgs[ada], 0);
    assert_eq!(r.server.traffic_sent(r.clients[ada].0).messages, 0);
    // The rider switches it on (the local server's single player may).
    let on = ClientCore::traffic_message(Some(RoadTraffic::Normal), None);
    let conn = r.clients[ada].0;
    r.server.handle(conn, &on);
    r.run(20.0, 200.0);
    assert!(r.server.vehicle_count() > 0);
    assert!(!r.clients[ada].1.vehicles().is_empty());
    let off = ClientCore::traffic_message(Some(RoadTraffic::Off), None);
    r.server.handle(conn, &off);
    r.run(1.0, 200.0);
    assert_eq!(r.server.vehicle_count(), 0);
    assert!(
        r.clients[ada].1.vehicles().is_empty(),
        "the reset cleared them"
    );
    let n = r.traffic_msgs[ada];
    r.run(10.0, 200.0);
    assert_eq!(r.traffic_msgs[ada], n, "off: nothing more");
}

/// The browser shows no road traffic (Q9 (b)): a server that sends some
/// anyway changes nothing there.
#[test]
fn the_browser_ignores_road_vehicles() {
    let mut r = Ride::new(RoadTraffic::Busy);
    let ada = r.join("Ada Rider");
    r.clients[ada].1.set_road_vehicles_shown(false);
    r.run(20.0, 200.0);
    assert!(r.traffic_msgs[ada] > 0, "the server sent vehicles");
    assert!(r.clients[ada].1.vehicles().is_empty());
    assert!(r.clients[ada].1.vehicles_view(r.t).is_empty());
}

/// A world without traffic data (an old package, this test's town) gets
/// the rules implied by class; a world switch starts the traffic again.
#[test]
fn old_worlds_have_traffic_and_a_world_switch_resets_it() {
    let mut r = Ride::new(RoadTraffic::Normal);
    let ada = r.join("Ada Rider");
    r.run(20.0, 200.0);
    assert!(r.server.vehicle_count() > 0, "implied rules carry traffic");
    let old_tick = r.sim_tick();
    let net = town();
    let (conns, bytes) = r.server.switch_world(
        ocw_server::Course::FreeRide {
            net: net.clone(),
            start: ocw_sim::FreeRideStart {
                edge: 0,
                offset_m: 10.0,
                forward: true,
            },
        },
        v1::WorldChange {
            package_id: "test/ada-town".into(),
            version: 2,
            ..Default::default()
        },
    );
    assert_eq!(conns.len(), 1);
    r.clients[ada].1.on_server_message(&bytes, r.t);
    assert!(
        r.clients[ada].1.vehicles().is_empty(),
        "the old world's are gone"
    );
    r.run(0.1, 0.0);
    assert!(r.sim_tick() < old_tick, "a new traffic simulation");
}
