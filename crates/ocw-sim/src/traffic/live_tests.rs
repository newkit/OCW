//! Tests of the live road events in the simulated traffic (spec 18E T13).

use ocw_world::RoadNetwork;
use ocw_world::geo::LocalProjection;
use ocw_world::graph::{Oneway, RoadClass};

use super::demand::{DemandTime, TrafficLevel};
use super::live::{self, Dir, EdgeEvent, GeoEvent, LiveKind};
use super::rules::{self, ClassRules, EdgeRule, EdgeRules};
use super::tests::{cfg, graph, grid};
use super::*;

fn ev(edge: usize, dir: Dir, kind: LiveKind) -> EdgeEvent {
    EdgeEvent {
        edge: edge as u32,
        dir,
        kind,
        speed_mps: None,
    }
}

fn two_lane_rule() -> EdgeRule {
    EdgeRule {
        oneway: Oneway::No,
        lanes: [2, 2],
        speed: 80.0 / 3.6,
        access: rules::Access::ALL,
        aadt: 9000.0,
        roundabout: false,
        rank: 6,
    }
}

#[test]
fn events_change_the_rules_of_their_edges() {
    let base = two_lane_rule();
    // A closure of one direction: no lanes that way, the other stays.
    let mut r = base;
    live::apply(&mut r, &ev(0, Dir::Forward, LiveKind::Closed));
    assert_eq!(r.lanes, [0, 2]);
    assert!(!r.allows(VehicleClass::Car, true) && r.allows(VehicleClass::Car, false));
    // Both: no motor vehicle at all.
    let mut r = base;
    live::apply(&mut r, &ev(0, Dir::Both, LiveKind::Closed));
    assert!(r.access.is_none() && r.lanes == [0, 0]);
    // Roadworks with a lane closed: one lane, 40 km/h.
    let mut r = base;
    live::apply(&mut r, &ev(0, Dir::Both, LiveKind::LaneClosed));
    assert_eq!(r.lanes, [1, 1]);
    assert!((r.speed * 3.6 - 40.0).abs() < 0.01);
    // … on a road of one lane a direction the lane stays (alternating).
    let mut one = base;
    one.lanes = [1, 1];
    live::apply(&mut one, &ev(0, Dir::Backward, LiveKind::Obstruction));
    assert_eq!(one.lanes, [1, 1]);
    assert!((one.speed * 3.6 - 30.0).abs() < 0.01);
    // A jam: the queue's speed and more demand.
    let mut r = base;
    live::apply(
        &mut r,
        &EdgeEvent {
            speed_mps: Some(live::JAM_STATIONARY_KMH / 3.6),
            ..ev(0, Dir::Forward, LiveKind::Jam)
        },
    );
    assert!(r.speed * 3.6 < 5.0);
    assert!((r.aadt - 9000.0 * live::JAM_DEMAND).abs() < 1e-6);
    // A limit never raises the road's own.
    let mut r = base;
    r.speed = 30.0 / 3.6;
    live::apply(
        &mut r,
        &EdgeEvent {
            speed_mps: Some(60.0 / 3.6),
            ..ev(0, Dir::Both, LiveKind::SpeedLimit)
        },
    );
    assert!((r.speed * 3.6 - 30.0).abs() < 0.01);
}

/// The grid world: 9 × 9 blocks of 150 m; the primary road along j = 4
/// (vertices 36 – 44) closed between i = 3 and i = 6.
fn closed_world() -> (RoadNetwork, Vec<usize>, Bubble) {
    let net = RoadNetwork::new(grid(9, 150.0));
    let find = |a: u32, b: u32| {
        net.graph()
            .edges
            .iter()
            .position(|e| (e.from.0, e.to.0) == (a, b))
            .expect("edge")
    };
    let closed: Vec<usize> = (3..6).map(|i| find(36 + i, 37 + i)).collect();
    let bubble = Bubble {
        center: [600.0, 600.0],
        radius: 550.0,
    };
    (net, closed, bubble)
}

#[test]
fn a_closure_keeps_every_vehicle_off_the_closed_part_for_an_hour() {
    let (net, closed, bubble) = closed_world();
    let mut sim = TrafficSim::new(&net, Box::new(ClassRules), cfg(11, 200, TrafficLevel::Busy));
    let time = DemandTime::weekday_at(17.0);
    let on_it = |sim: &TrafficSim| {
        sim.placements()
            .iter()
            .filter(|(e, _, _)| closed.contains(e))
            .count()
    };
    // Ten minutes of traffic, then the closure: the vehicles on it or
    // routed over it leave.
    let mut before = 0;
    for t in 0..3_000u32 {
        sim.step(&net, &[bubble], &[], &time.plus_seconds(t / 5));
        before = before.max(on_it(&sim));
    }
    assert!(before > 0, "the main road has traffic before");
    let events: Vec<EdgeEvent> = closed
        .iter()
        .map(|&e| ev(e, Dir::Both, LiveKind::Closed))
        .collect();
    sim.set_live(&events);
    sim.drain_events();
    assert_eq!(on_it(&sim), 0, "nothing left on the closed part");
    let mut routed = 0;
    for t in 0..18_000u32 {
        sim.step(&net, &[bubble], &[], &time.plus_seconds(600 + t / 5));
        if t % 25 == 0 {
            assert_eq!(on_it(&sim), 0, "tick {t}: a vehicle on the closed road");
        }
        for e in sim.drain_events() {
            match e {
                TrafficEvent::Spawn(p) => {
                    assert!(p.route.iter().all(|s| !closed.contains(&(s.edge as usize))));
                }
                TrafficEvent::Change(c) => {
                    assert!(
                        c.extend
                            .iter()
                            .all(|s| !closed.contains(&(s.edge as usize)))
                    );
                    routed += usize::from(!c.extend.is_empty());
                }
                TrafficEvent::Despawn { .. } => {}
            }
        }
    }
    assert!(routed > 50 && sim.len() > 20, "the traffic goes on");
    // Lifted: the statistics again, and the road fills.
    sim.set_live(&[]);
    assert_eq!(sim.rules(), sim.base_rules());
    let mut after = 0;
    for t in 0..3_000u32 {
        sim.step(&net, &[bubble], &[], &time.plus_seconds(4_200 + t / 5));
        after = after.max(on_it(&sim));
    }
    assert!(after > 0, "traffic on the main road again");
}

/// A straight primary road 2 km west to east (two edges of 1 km).
fn straight() -> RoadNetwork {
    RoadNetwork::new(graph(
        &[[0.0, 0.0], [1000.0, 0.0], [2000.0, 0.0]],
        &[(0, 1, RoadClass::Primary), (1, 2, RoadClass::Primary)],
    ))
}

/// The mean speed (m/s) and vehicles on edge `e` going forward, over 20
/// simulated minutes of busy traffic, with `events`.
fn measure(events: &[EdgeEvent], e: usize) -> (f64, f64) {
    let net = straight();
    let mut sim = TrafficSim::new(&net, Box::new(ClassRules), cfg(5, 100, TrafficLevel::Busy));
    sim.set_live(events);
    let bubble = Bubble {
        center: [1000.0, 0.0],
        radius: 1500.0,
    };
    let time = DemandTime::weekday_at(17.5);
    let (mut v, mut n, mut samples) = (0.0, 0usize, 0usize);
    for t in 0..12_000u32 {
        sim.step(&net, &[bubble], &[], &time.plus_seconds(t / 10));
        if t >= 3_000 && t % 50 == 0 {
            samples += 1;
            for view in sim.views(&net) {
                if view.edge == e && view.forward {
                    v += view.speed;
                    n += 1;
                }
            }
        }
    }
    (v / n.max(1) as f64, n as f64 / samples as f64)
}

#[test]
fn roadworks_and_jams_lower_the_speeds_on_their_edges() {
    let (free_v, free_n) = measure(&[], 1);
    let works = [ev(1, Dir::Forward, LiveKind::LaneClosed)];
    let (works_v, _) = measure(&works, 1);
    let jam = [EdgeEvent {
        speed_mps: Some(live::JAM_QUEUING_KMH / 3.6),
        ..ev(1, Dir::Forward, LiveKind::Jam)
    }];
    let (jam_v, jam_n) = measure(&jam, 1);
    assert!(free_v > 15.0, "free flow at about the limit: {free_v:.1}");
    assert!(
        works_v < 13.0 && works_v < 0.7 * free_v,
        "roadworks at 40 km/h: {works_v:.1}"
    );
    assert!(jam_v < 3.5, "the reported jam at queuing speed: {jam_v:.1}");
    assert!(
        jam_n > 2.0 * free_n,
        "denser in the jam: {jam_n:.1} against {free_n:.1}"
    );
}

#[test]
fn records_map_onto_the_roads_between_their_points() {
    let net = RoadNetwork::new(grid(9, 150.0));
    let r: Vec<EdgeRule> = (0..net.graph().edges.len())
        .map(|e| ClassRules.edge_rule(&net, e))
        .collect();
    let origin = (28.30, -16.50);
    let p = LocalProjection::new(origin.0, origin.1);
    let at = |x: f64, z: f64| p.unproject([x, z]);
    let find = |a: u32, b: u32| {
        net.graph()
            .edges
            .iter()
            .position(|e| (e.from.0, e.to.0) == (a, b))
            .expect("edge") as u32
    };
    // Along j = 4 from x = 460 to x = 890, eastbound only: the edges
    // 3 → 4 … 5 → 6 forward.
    let g = GeoEvent {
        kind: LiveKind::Closed,
        speed_kmh: None,
        road: "TF-1".into(),
        from: at(460.0, 605.0),
        to: Some(at(890.0, 595.0)),
        length_m: Some(400.0),
        one_direction: true,
        bearing_deg: None,
    };
    let edges = live::map_events(&net, &r, origin, std::slice::from_ref(&g))[0]
        .clone()
        .expect("mapped");
    let want: Vec<u32> = (3..6).map(|i| find(36 + i, 37 + i)).collect();
    assert_eq!(edges.iter().map(|e| e.edge).collect::<Vec<_>>(), want);
    assert!(edges.iter().all(|e| e.dir == Dir::Forward));
    // Westbound: the same edges backward.
    let west = GeoEvent {
        from: g.to.expect("to"),
        to: Some(g.from),
        ..g.clone()
    };
    let w = live::map_events(&net, &r, origin, &[west])[0]
        .clone()
        .expect("west");
    assert!(w.iter().all(|e| e.dir == Dir::Backward) && w.len() == 3);
    // A point with a bearing: its edge in that direction.
    let pt = GeoEvent {
        kind: LiveKind::Obstruction,
        to: None,
        bearing_deg: Some(270.0),
        ..g.clone()
    };
    let one = live::map_events(&net, &r, origin, &[pt])[0]
        .clone()
        .expect("point");
    assert_eq!((one.len(), one[0].dir), (1, Dir::Backward));
    // Far from every road: not mapped.
    let far = GeoEvent {
        from: at(5000.0, 5000.0),
        ..g.clone()
    };
    assert_eq!(
        live::map_events(&net, &r, origin, &[far])[0],
        Err(live::Unmapped::NoRoad)
    );
    // A speed limit below 10 km/h is not believed (nothing to do).
    let slow = GeoEvent {
        kind: LiveKind::SpeedLimit,
        speed_kmh: Some(2.0),
        ..g
    };
    assert_eq!(
        live::map_events(&net, &r, origin, &[slow])[0],
        Ok(Vec::new())
    );
}
