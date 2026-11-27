//! Tests of the traffic core (spec 18E T3).

use ocw_world::RoadNetwork;
use ocw_world::graph::{Edge, EdgeId, Oneway, RoadClass, RoadGraph, VertexId};

use super::demand::{self, DemandTime, TrafficLevel};
use super::kin::{self, DriverParams};
use super::plan::{self, PlanFollower, TrafficEvent};
use super::routing;
use super::rules::{self, Access, ClassRules, Control, EdgeRule, EdgeRules, VehicleClass};
use super::*;

/// A graph of straight edges (x, z) with classes.
fn graph(vertices: &[[f64; 2]], edges: &[(u32, u32, RoadClass)]) -> RoadGraph {
    RoadGraph {
        vertices: vertices.to_vec(),
        edges: edges
            .iter()
            .enumerate()
            .map(|(i, &(a, b, class))| Edge {
                id: EdgeId(5000 + i as u64),
                from: VertexId(a),
                to: VertexId(b),
                points: vec![vertices[a as usize], vertices[b as usize]],
                class,
                name: None,
                width_m: class.default_width_m(),
                bridge: false,
                tunnel: false,
                dead_end_m: None,
                boundary: false,
                oneway: Oneway::No,
                osm_way: i as i64,
                profile: Vec::new(),
            })
            .collect(),
        boundary_vertices: Default::default(),
    }
}

/// An n × n grid of `block` m with tertiary roads, every third line a
/// primary; vertex (i, j) is i + j·n.
fn grid(n: u32, block: f64) -> RoadGraph {
    let mut v = Vec::new();
    for j in 0..n {
        for i in 0..n {
            v.push([f64::from(i) * block, f64::from(j) * block]);
        }
    }
    let mut e = Vec::new();
    for j in 0..n {
        for i in 0..n {
            let at = i + j * n;
            let class = |k: u32| {
                if k % 3 == 1 {
                    RoadClass::Primary
                } else if k.is_multiple_of(2) {
                    RoadClass::Residential
                } else {
                    RoadClass::Tertiary
                }
            };
            if i + 1 < n {
                e.push((at, at + 1, class(j)));
            }
            if j + 1 < n {
                e.push((at, at + n, class(i)));
            }
        }
    }
    graph(&v, &e)
}

/// Rules on top of the class defaults: edges closed to motor vehicles,
/// to lorries, one-way for cars only, signals at vertices.
#[derive(Default)]
struct TestRules {
    no_motor: Vec<usize>,
    no_hgv: Vec<usize>,
    oneway: Vec<(usize, Oneway)>,
    signals: Vec<u32>,
    stops: Vec<(u32, usize)>,
}

impl EdgeRules for TestRules {
    fn edge_rule(&self, net: &RoadNetwork, edge: usize) -> EdgeRule {
        let mut r = rules::class_rule(net, edge);
        if self.no_motor.contains(&edge) {
            r.access = Access::NONE;
            r.lanes = [0, 0];
        }
        if self.no_hgv.contains(&edge) {
            r.access = r.access.without(VehicleClass::Lorry);
        }
        if let Some(&(_, o)) = self.oneway.iter().find(|(e, _)| *e == edge) {
            r.oneway = o;
            r.lanes = [u8::from(o.allows(true)), u8::from(o.allows(false))];
        }
        r
    }

    fn node_control(
        &self,
        _net: &RoadNetwork,
        vertex: VertexId,
        edge: usize,
        _forward: bool,
    ) -> Option<Control> {
        if self.signals.contains(&vertex.0) {
            Some(Control::Signals)
        } else if self.stops.contains(&(vertex.0, edge)) {
            Some(Control::Stop)
        } else {
            None
        }
    }
}

fn cfg(seed: u64, step_ms: u32, level: TrafficLevel) -> TrafficConfig {
    TrafficConfig {
        seed,
        step_ms,
        level,
        ..TrafficConfig::default()
    }
}

/// The grid world of the rule tests: 9 × 9 blocks of 150 m, a rider's
/// bubble of 500 m in the middle (the outer ring outside, where traffic
/// comes in), one-way streets, a closed street, a lorry ban.
fn rule_world() -> (RoadNetwork, TestRules, Bubble) {
    let net = RoadNetwork::new(grid(9, 150.0));
    let find = |a: u32, b: u32| {
        net.graph()
            .edges
            .iter()
            .position(|e| (e.from.0, e.to.0) == (a, b))
            .expect("edge")
    };
    let n = 9;
    let at = |i: u32, j: u32| i + j * n;
    let rules = TestRules {
        no_motor: vec![find(at(3, 4), at(4, 4)), find(at(4, 3), at(4, 4))],
        no_hgv: vec![
            find(at(1, 4), at(2, 4)),
            find(at(2, 4), at(3, 4)),
            find(at(4, 1), at(5, 1)),
            find(at(5, 1), at(6, 1)),
        ],
        oneway: vec![
            (find(at(4, 4), at(5, 4)), Oneway::Forward),
            (find(at(5, 4), at(6, 4)), Oneway::Forward),
            (find(at(4, 5), at(4, 6)), Oneway::Backward),
            (find(at(3, 2), at(3, 3)), Oneway::Forward),
            (find(at(5, 6), at(6, 6)), Oneway::Backward),
        ],
        signals: vec![at(4, 7)],
        stops: vec![],
    };
    let bubble = Bubble {
        center: [600.0, 600.0],
        radius: 500.0,
    };
    (net, rules, bubble)
}

/// Checks every vehicle's edge against the rules.
fn check_rules(sim: &TrafficSim, what: &str) {
    for (e, fwd, class) in sim.placements() {
        let r = sim.rule(e);
        assert!(
            r.access.allows(class),
            "{what}: a {} on edge {e} it may not use",
            class.name()
        );
        assert!(
            r.oneway.allows(fwd),
            "{what}: a {} against the one-way edge {e}",
            class.name()
        );
        assert!(
            r.lanes_in(fwd) > 0,
            "{what}: a {} on edge {e} without a lane",
            class.name()
        );
    }
}

#[test]
fn idm_free_road_and_standing_leader() {
    let p = DriverParams {
        accel: 1.5,
        decel: 2.0,
        lateral: 3.0,
        headway: 1.4,
        gap: 2.0,
    };
    assert!((kin::idm(&p, 0.0, 20.0, None) - 1.5).abs() < 1e-9);
    assert!(kin::idm(&p, 20.0, 20.0, None).abs() < 1e-9);
    assert!(kin::idm(&p, 25.0, 20.0, None) < 0.0);
    // Close behind a standing car: hard braking, capped.
    let a = kin::idm(&p, 15.0, 20.0, Some((10.0, 0.0)));
    assert_eq!(a, -kin::MAX_BRAKE);
    // A vehicle behind one at the same speed at a long gap barely reacts.
    let a = kin::idm(&p, 15.0, 20.0, Some((200.0, 15.0)));
    assert!(a > 0.5);
}

#[test]
fn curves_and_turns_lower_the_desired_speed() {
    // A circle of 50 m radius as one edge.
    let pts: Vec<[f64; 2]> = (0..=40)
        .map(|k| {
            let t = f64::from(k) / 40.0 * std::f64::consts::PI;
            [50.0 * t.cos(), 50.0 * t.sin()]
        })
        .collect();
    let mut g = graph(&[pts[0], pts[40]], &[(0, 1, RoadClass::Primary)]);
    g.edges[0].points = pts;
    let net = RoadNetwork::new(g);
    let curves = kin::Curves::new(&net);
    let r = curves.radii(&net, 0);
    let mid = r[r.len() / 2];
    assert!((mid - 50.0).abs() < 3.0, "radius {mid}");
    let p = DriverParams {
        accel: 1.5,
        decel: 2.0,
        lateral: 3.0,
        headway: 1.4,
        gap: 2.0,
    };
    let route = [plan::Step {
        edge: 0,
        forward: true,
        speed: 22.0,
        turn_r: kin::STRAIGHT_R as f32,
    }];
    let pos = RoutePos {
        step: 0,
        s: net.geometry(0).length() / 2.0,
    };
    let v0 = kin::desired_speed(&net, &curves, &route, pos, &p);
    assert!(
        (v0 - (3.0f64 * 50.0).sqrt()).abs() < 1.0,
        "in the curve {v0}"
    );
    // A right-angle turn: 10 m radius.
    assert!((kin::turn_radius([1.0, 0.0], [0.0, 1.0]) - 10.0).abs() < 1e-9);
    assert_eq!(kin::turn_radius([1.0, 0.0], [1.0, 0.0]), kin::STRAIGHT_R);
}

#[test]
fn demand_profiles_follow_the_hour_and_the_day() {
    let day = |wd: u8| DemandTime {
        weekday: wd,
        seconds: 0,
        month: 6,
        holiday: false,
    };
    for wd in [0u8, 5, 6] {
        let total: f64 = (0..24 * 60)
            .map(|m| {
                demand::hour_share(&DemandTime {
                    seconds: m * 60,
                    ..day(wd)
                }) / 60.0
            })
            .sum();
        assert!((total - 1.0).abs() < 1e-3, "day {wd}: {total}");
    }
    let at = |wd: u8, h: f64| {
        demand::hour_share(&DemandTime {
            seconds: (h * 3600.0) as u32,
            ..day(wd)
        })
    };
    // Weekday double peak, quiet night.
    assert!(at(1, 8.0) > 5.0 * at(1, 3.0));
    assert!(at(1, 17.5) > at(1, 12.0));
    assert!(at(1, 8.0) > at(1, 11.0));
    // A quiet Sunday morning.
    assert!(at(6, 8.0) < 0.5 * at(1, 8.0));
    assert!(demand::day_factor(&day(6)) < demand::day_factor(&day(1)));
    // Holidays: Easter 2026 is 5 April; Whit Monday 25 May in DE.
    assert_eq!(demand::easter(2026), (4, 5));
    assert_eq!(demand::easter(2025), (4, 20));
    assert!(demand::is_holiday(Region::De, 2026, 5, 25));
    assert!(demand::is_holiday(Region::Es, 2026, 4, 3));
    assert!(demand::is_holiday(Region::Li, 2026, 8, 15));
    assert!(!demand::is_holiday(Region::De, 2026, 8, 15));
    let lt = ocw_sky::LocalTime {
        year: 2026,
        month: 12,
        day: 25,
        hour: 9,
        minute: 0,
        second: 0,
        offset_s: 3600,
        dst: false,
    };
    let t = DemandTime::from_local(&lt, Region::Ch);
    assert!(t.holiday && t.day_type() == demand::DayType::Sunday);
    // More motorbikes on summer weekends, few lorries on Sundays.
    let summer_sat = DemandTime {
        weekday: 5,
        ..day(5)
    };
    let winter_wed = DemandTime {
        weekday: 2,
        month: 1,
        ..day(2)
    };
    let m_sat = demand::class_mix(RoadClass::Secondary, &summer_sat);
    let m_win = demand::class_mix(RoadClass::Secondary, &winter_wed);
    assert!(m_sat[VehicleClass::Motorbike.index()] > 5.0 * m_win[VehicleClass::Motorbike.index()]);
    let m_sun = demand::class_mix(RoadClass::Trunk, &day(6));
    assert!(m_sun[VehicleClass::Lorry.index()] < 0.02);
    assert_eq!(demand::class_mix(RoadClass::Track, &day(1))[5], 1.0);
    assert_eq!(TrafficLevel::Light.factor(), 0.25);
    assert_eq!(TrafficLevel::Busy.factor(), 1.5);
    assert_eq!(TrafficLevel::parse("lunatic"), Some(TrafficLevel::Lunatic));
    assert_eq!(
        DriverManners::parse("realistic"),
        Some(DriverManners::Realistic)
    );
}

#[test]
fn class_rules_and_derived_controls() {
    // A T junction: primary west–east, residential south.
    let g = graph(
        &[
            [-200.0, 0.0],
            [0.0, 0.0],
            [200.0, 0.0],
            [0.0, 200.0],
            [0.0, -300.0],
        ],
        &[
            (0, 1, RoadClass::Primary),
            (1, 2, RoadClass::Primary),
            (1, 3, RoadClass::Residential),
            (4, 1, RoadClass::Cycleway),
        ],
    );
    let net = RoadNetwork::new(g);
    let r = |e| rules::class_rule(&net, e);
    assert!(r(3).access.is_none(), "a cycleway carries no traffic");
    assert!(!r(3).allows(VehicleClass::Car, true));
    assert!(r(0).allows(VehicleClass::Lorry, true));
    assert!(!r(2).access.is_none());
    let v = VertexId(1);
    assert_eq!(rules::derived_control(&net, v, 0, r), Control::Priority);
    assert_eq!(rules::derived_control(&net, v, 2, r), Control::GiveWay);
    // Tracks: tractors only.
    let t = rules::class_defaults(RoadClass::Track).access;
    assert!(t.allows(VehicleClass::Tractor) && !t.allows(VehicleClass::Car));
    // Equal residential roads: right before left.
    let g = graph(
        &[[-100.0, 0.0], [0.0, 0.0], [100.0, 0.0], [0.0, 100.0]],
        &[
            (0, 1, RoadClass::Residential),
            (1, 2, RoadClass::Residential),
            (1, 3, RoadClass::Residential),
        ],
    );
    let net = RoadNetwork::new(g);
    let r = |e| rules::class_rule(&net, e);
    assert_eq!(
        rules::derived_control(&net, VertexId(1), 0, r),
        Control::RightBeforeLeft
    );
}

#[test]
fn shortest_route_respects_oneway_and_access() {
    let (net, rules, _) = rule_world();
    let r: Vec<EdgeRule> = (0..net.graph().edges.len())
        .map(|e| rules.edge_rule(&net, e))
        .collect();
    let n = 9;
    let at = |i: u32, j: u32| VertexId(i + j * n);
    let edge_from = |a: VertexId, b: VertexId| {
        net.graph()
            .edges
            .iter()
            .position(|e| e.from == a && e.to == b)
            .expect("edge")
    };
    // From (6, 4) west to (4, 4): (4,4)→(5,4)→(6,4) is one-way east, so
    // the car goes round the block.
    let start = (edge_from(at(6, 4), at(7, 4)), false);
    let route =
        routing::shortest_route(&net, &r, VehicleClass::Car, start, at(4, 4)).expect("a route");
    for &(e, f) in &route {
        assert!(r[e].allows(VehicleClass::Car, f), "edge {e} forward {f}");
    }
    assert!(route.len() > 3, "round the block: {route:?}");
    // A lorry never uses the hgv=no edges.
    let start = (edge_from(at(0, 4), at(1, 4)), true);
    let route =
        routing::shortest_route(&net, &r, VehicleClass::Lorry, start, at(3, 4)).expect("a route");
    for &(e, _) in &route {
        assert!(r[e].access.allows(VehicleClass::Lorry), "lorry on {e}");
    }
    let car = routing::shortest_route(&net, &r, VehicleClass::Car, start, at(3, 4)).expect("car");
    assert!(car.len() < route.len(), "the car takes the direct street");
}

#[test]
fn no_vehicle_against_a_oneway_or_on_a_closed_edge_in_a_day() {
    let (net, rules, bubble) = rule_world();
    let mut sim = TrafficSim::new(&net, Box::new(rules), cfg(7, 250, TrafficLevel::Busy));
    let start = DemandTime::weekday_at(0.0);
    let ticks = 86_400 * 4;
    let mut most = 0;
    let mut classes = [0u64; 6];
    for t in 0..ticks {
        let time = start.plus_seconds(t / 4);
        sim.step(&net, &[bubble], &[], &time);
        if t.is_multiple_of(4) {
            check_rules(&sim, &format!("tick {t}"));
            most = most.max(sim.len());
        }
        for ev in sim.drain_events() {
            if let TrafficEvent::Spawn(p) = ev {
                classes[p.class.index()] += 1;
                for st in &p.route {
                    assert!(sim.rule(st.edge as usize).allows(p.class, st.forward));
                }
            }
        }
    }
    let s = sim.stats();
    assert!(s.spawned > 2_000, "a day of traffic: {s:?}");
    assert!(most > 20, "at the peak: {most}");
    assert!(classes[VehicleClass::Lorry.index()] > 10, "{classes:?}");
    assert!(classes[VehicleClass::Car.index()] > classes[VehicleClass::Van.index()]);
}

#[test]
fn queues_at_a_signal_clear() {
    // A straight primary road west–east with lights at x = 0.
    let g = graph(
        &[[-1000.0, 0.0], [0.0, 0.0], [600.0, 0.0]],
        &[(0, 1, RoadClass::Primary), (1, 2, RoadClass::Primary)],
    );
    let net = RoadNetwork::new(g);
    let rules = TestRules {
        signals: vec![1],
        ..TestRules::default()
    };
    let mut sim = TrafficSim::new(&net, Box::new(rules), cfg(3, 20, TrafficLevel::Off));
    let bubble = Bubble {
        center: [0.0, 0.0],
        radius: 2_000.0,
    };
    // Start at a red light for the arrival from the west (group 0).
    let mut t = 0;
    while signals::light(1, 0, sim.time_ms()).0 != signals::Light::Red {
        sim.step(&net, &[bubble], &[], &DemandTime::weekday_at(12.0));
        t += 1;
    }
    let route = [(0usize, true), (1usize, true)];
    // Twelve cars arriving 12 m apart, the first 40 m before the line.
    let mut ids = Vec::new();
    for k in 0..12 {
        let s = 960.0 - 12.0 * f64::from(k);
        ids.push(
            sim.spawn_on(&net, VehicleClass::Car, &route, s, 5.0, 0)
                .expect("spawn"),
        );
    }
    let (cycle, _) = signals::cycle(1);
    let time = DemandTime::weekday_at(12.0);
    let mut crossed = std::collections::BTreeSet::new();
    let mut last_edge: Vec<(u32, usize)> = Vec::new();
    for _ in 0..(3 * cycle / 20) {
        let before = signals::light(1, 0, sim.time_ms()).0;
        sim.step(&net, &[bubble], &[], &time);
        t += 1;
        let now: Vec<(u32, usize)> = sim
            .speeds()
            .iter()
            .zip(sim.placements())
            .map(|(&(id, _), (e, _, _))| (id, e))
            .collect();
        for &(id, e) in &now {
            let was = last_edge.iter().find(|(i, _)| *i == id).map(|x| x.1);
            if e == 1 && was == Some(0) {
                assert_ne!(
                    before,
                    signals::Light::Red,
                    "car {id} crossed on red at tick {t}"
                );
                crossed.insert(id);
            }
        }
        last_edge = now;
    }
    assert_eq!(
        crossed.len(),
        ids.len(),
        "the queue cleared within three cycles"
    );
    // While the light was red, the queue stood: the first car stopped
    // before the line (checked by the crossing test above).
}

/// A ring road of `length` m as `n` edges.
fn ring(n: u32, length: f64) -> RoadNetwork {
    let r = length / (2.0 * std::f64::consts::PI);
    let v: Vec<[f64; 2]> = (0..n)
        .map(|k| {
            let t = f64::from(k) / f64::from(n) * 2.0 * std::f64::consts::PI;
            [r * t.cos(), r * t.sin()]
        })
        .collect();
    let e: Vec<(u32, u32, RoadClass)> = (0..n)
        .map(|k| (k, (k + 1) % n, RoadClass::Secondary))
        .collect();
    let mut g = graph(&v, &e);
    for edge in &mut g.edges {
        edge.oneway = Oneway::Forward;
    }
    RoadNetwork::new(g)
}

fn spread(speeds: &[(u32, f64)]) -> (f64, f64, f64) {
    let n = speeds.len() as f64;
    let mean = speeds.iter().map(|s| s.1).sum::<f64>() / n;
    let var = speeds
        .iter()
        .map(|s| (s.1 - mean) * (s.1 - mean))
        .sum::<f64>()
        / n;
    let min = speeds.iter().map(|s| s.1).fold(f64::INFINITY, f64::min);
    (mean, var.sqrt(), min)
}

#[test]
fn a_jam_wave_forms_and_dissolves_on_a_ring() {
    let length = 2_000.0;
    let net = ring(40, length);
    let mut sim = TrafficSim::new(&net, Box::new(ClassRules), cfg(11, 100, TrafficLevel::Off));
    let bubble = Bubble {
        center: [0.0, 0.0],
        radius: 1_000.0,
    };
    let time = DemandTime::weekday_at(8.0);
    let edge_len = net.geometry(0).length();
    // Lorries above capacity: 35 a km (12 m long, slow to pull away:
    // the IDM's congested flow is string-unstable there), evenly spread.
    let count = 70;
    for k in 0..count {
        let d = length * f64::from(k) / f64::from(count);
        let e = ((d / edge_len) as usize).min(39);
        let route: Vec<(usize, bool)> = (0..3).map(|j| ((e + j) % 40, true)).collect();
        sim.spawn_on(
            &net,
            VehicleClass::Lorry,
            &route,
            d - e as f64 * edge_len,
            8.0,
            0,
        );
    }
    // A driver brakes hard once.
    for t in 0..6_000 {
        if t == 50 {
            let first = sim.speeds()[0].0;
            sim.set_speed(first, 0.0);
        }
        sim.step(&net, &[bubble], &[], &time);
    }
    let (mean, sd, min) = spread(&sim.speeds());
    assert_eq!(sim.len(), count as usize, "nobody left the ring");
    assert!(
        min < 1.5 && sd > 2.0,
        "stop and go: mean {mean:.1}, sd {sd:.1}, min {min:.1}"
    );
    // Below capacity (two thirds leave): the wave dissolves.
    let ids: Vec<u32> = sim.speeds().iter().map(|s| s.0).collect();
    for (k, id) in ids.iter().enumerate() {
        if k % 3 != 0 {
            sim.remove(*id);
        }
    }
    for _ in 0..6_000 {
        sim.step(&net, &[bubble], &[], &time);
    }
    let (mean, sd, min) = spread(&sim.speeds());
    assert!(
        min > 0.6 * mean && sd < 1.5,
        "free flow: mean {mean:.1}, sd {sd:.1}, min {min:.1}"
    );
}

/// A grid scenario for determinism, plans and demand: 12 × 12 blocks of
/// 120 m, a bubble of 500 m.
fn busy_world(seed: u64, step_ms: u32, level: TrafficLevel) -> (RoadNetwork, TrafficSim, Bubble) {
    let net = RoadNetwork::new(grid(12, 120.0));
    let mut c = cfg(seed, step_ms, level);
    c.max_vehicles = 400;
    let sim = TrafficSim::new(&net, Box::new(ClassRules), c);
    let bubble = Bubble {
        center: [660.0, 660.0],
        radius: 500.0,
    };
    (net, sim, bubble)
}

fn run(seed: u64, ticks: u32) -> (u64, Vec<TrafficEvent>) {
    let (net, mut sim, bubble) = busy_world(seed, 20, TrafficLevel::Busy);
    let time = DemandTime::weekday_at(17.0);
    let mut events = Vec::new();
    for _ in 0..ticks {
        sim.step(&net, &[bubble], &[], &time);
        events.extend(sim.drain_events());
    }
    (sim.digest(), events)
}

#[test]
fn deterministic_from_the_seed() {
    let (a, ea) = run(42, 3_000);
    let (b, eb) = run(42, 3_000);
    assert_eq!(a, b, "same seed, same state");
    assert!(ea == eb, "same seed, same plans and changes");
    let (c, _) = run(43, 3_000);
    assert_ne!(a, c, "another seed, other traffic");
}

#[test]
fn clients_follow_the_plans_within_the_tolerance() {
    let (net, mut sim, bubble) = busy_world(5, 20, TrafficLevel::Busy);
    let time = DemandTime::weekday_at(8.0);
    let curves = kin::Curves::new(&net);
    let mut client: Vec<PlanFollower> = Vec::new();
    let mut worst: f64 = 0.0;
    let mut changes = 0u64;
    for t in 0..15_000u32 {
        sim.step(&net, &[bubble], &[], &time);
        for f in &mut client {
            f.step(&net, &curves, sim.dt());
        }
        let ev = sim.drain_events();
        changes += ev
            .iter()
            .filter(|e| matches!(e, TrafficEvent::Change(_)))
            .count() as u64;
        plan::apply_events(&mut client, &ev);
        if t.is_multiple_of(10) {
            let views = sim.views(&net);
            assert_eq!(views.len(), client.len(), "tick {t}");
            for (v, f) in views.iter().zip(&client) {
                assert_eq!(v.id, f.id);
                // Centreline positions (one may have crossed a vertex).
                let p = f.point(&net).expect("on the route");
                let q = net.geometry(v.edge).point_at(v.s);
                let d = ((p[0] - q[0]).powi(2) + (p[1] - q[2]).powi(2)).sqrt();
                assert!(d < 2.0, "vehicle {} at tick {t}: {d:.2} m off", v.id);
                worst = worst.max(d);
                assert_eq!(f.lane, v.lane);
            }
        }
    }
    assert!(worst <= plan::POS_TOLERANCE_M + 0.05, "worst {worst}");
    let st = sim.stats();
    let per_min = changes as f64 / (st.vehicle_ticks as f64 * sim.dt() / 60.0);
    println!(
        "plans: {} spawns, {changes} changes, {per_min:.2} changes per vehicle-minute, worst {worst:.3} m",
        st.spawned
    );
    assert!(
        per_min < 20.0,
        "changes are the exception: {per_min:.2} a minute"
    );
}

#[test]
fn demand_follows_the_hour_profile() {
    let mut counts = Vec::new();
    for hour in [3.5, 8.5, 12.5, 17.5] {
        let (net, mut sim, bubble) = busy_world(9, 250, TrafficLevel::Normal);
        let time = DemandTime::weekday_at(hour);
        sim.step(&net, &[bubble], &[], &time);
        let filled = sim.stats().spawned;
        for _ in 0..(1800 * 4) {
            sim.step(&net, &[bubble], &[], &time);
        }
        counts.push((hour, (sim.stats().spawned - filled) as f64, filled));
    }
    let share = |h: f64| demand::hour_share(&DemandTime::weekday_at(h));
    for &(h, c, filled) in &counts {
        println!(
            "hour {h}: {c} vehicles in 30 min (fill {filled}), share {:.4}",
            share(h)
        );
    }
    let (night, morning, noon, evening) = (counts[0].1, counts[1].1, counts[2].1, counts[3].1);
    let ratio = |a: f64, b: f64| a / b.max(1.0);
    let expect = |a: f64, b: f64| share(a) / share(b);
    assert!(ratio(morning, night) > 0.5 * expect(8.5, 3.5), "{counts:?}");
    for (c, h) in [(noon, 12.5), (evening, 17.5)] {
        let r = ratio(c, morning);
        let e = expect(h, 8.5);
        assert!(
            (r / e - 1.0).abs() < 0.2,
            "hour {h}: {r:.2} against {e:.2} ({counts:?})"
        );
    }
    // The fill follows the hour too.
    assert!(counts[1].2 > 3 * counts[0].2);
}

/// The class rules with overtaking forbidden everywhere (T4: riders are
/// not passed there).
struct NoPassing;

impl EdgeRules for NoPassing {
    fn edge_rule(&self, net: &RoadNetwork, edge: usize) -> EdgeRule {
        rules::class_rule(net, edge)
    }

    fn passing(&self, _net: &RoadNetwork, _edge: usize) -> PassRule {
        PassRule::new(around::Country::De, false, [true, true])
    }
}

#[test]
fn vehicles_queue_behind_a_rider() {
    let g = graph(
        &[[0.0, 0.0], [3000.0, 0.0]],
        &[(0, 1, RoadClass::Secondary)],
    );
    let net = RoadNetwork::new(g);
    // Overtaking forbidden (T4): the car waits behind.
    let mut sim = TrafficSim::new(&net, Box::new(NoPassing), cfg(2, 20, TrafficLevel::Off));
    let bubble = Bubble {
        center: [1500.0, 0.0],
        radius: 2000.0,
    };
    let id = sim
        .spawn_on(&net, VehicleClass::Car, &[(0, true)], 10.0, 18.0, 0)
        .expect("car");
    let time = DemandTime::weekday_at(10.0);
    let mut rider = 300.0;
    let mut closest = f64::INFINITY;
    for _ in 0..(120 * 50) {
        let user = RoadUser {
            edge: 0,
            s: rider,
            forward: true,
            speed: 7.0,
            length: 1.8,
            ..RoadUser::default()
        };
        sim.step(&net, &[bubble], &[user], &time);
        rider += 7.0 * 0.02;
        let car = sim.views(&net)[0];
        assert_eq!(car.id, id);
        closest = closest.min(rider - 1.8 - (car.s));
    }
    let car = sim.views(&net)[0];
    assert!(
        closest > RIDER_GAP_M - 0.5,
        "closest {closest:.2} m (bumper to wheel)"
    );
    assert!(
        (car.speed - 7.0).abs() < 0.5,
        "waits at the rider's speed: {}",
        car.speed
    );
}

#[test]
fn faster_vehicles_change_lane_to_pass_on_two_lanes() {
    let mut g = graph(&[[0.0, 0.0], [5000.0, 0.0]], &[(0, 1, RoadClass::Trunk)]);
    g.edges[0].oneway = Oneway::Forward;
    let net = RoadNetwork::new(g);
    assert_eq!(rules::class_rule(&net, 0).lanes, [2, 0]);
    let bubble = Bubble {
        center: [2500.0, 0.0],
        radius: 3000.0,
    };
    // A seed whose car driver wants to go clearly faster than the lorry's.
    let (mut sim, lorry, car) = (4..40)
        .find_map(|seed| {
            let mut sim =
                TrafficSim::new(&net, Box::new(ClassRules), cfg(seed, 20, TrafficLevel::Off));
            let lorry = sim.spawn_on(&net, VehicleClass::Lorry, &[(0, true)], 300.0, 18.0, 0)?;
            let car = sim.spawn_on(&net, VehicleClass::Car, &[(0, true)], 100.0, 25.0, 0)?;
            let speeds: Vec<f32> = sim
                .drain_events()
                .iter()
                .filter_map(|e| {
                    if let TrafficEvent::Spawn(p) = e {
                        Some(p.route[0].speed)
                    } else {
                        None
                    }
                })
                .collect();
            (speeds[1] > speeds[0] + 3.0).then_some((sim, lorry, car))
        })
        .expect("a fast car");
    let time = DemandTime::weekday_at(10.0);
    let mut used_lane_1 = false;
    for _ in 0..(90 * 50) {
        sim.step(&net, &[bubble], &[], &time);
        let v = sim.views(&net);
        let c = v.iter().find(|x| x.id == car).expect("car");
        used_lane_1 |= c.lane == 1;
        let l = v.iter().find(|x| x.id == lorry).expect("lorry");
        if c.s > l.s + 50.0 {
            assert!(used_lane_1);
            return;
        }
    }
    panic!("the car never passed the lorry");
}

#[test]
fn give_way_and_right_before_left_resolve() {
    // An equal residential crossroads: four cars arrive together.
    let g = graph(
        &[
            [-150.0, 0.0],
            [150.0, 0.0],
            [0.0, -150.0],
            [0.0, 150.0],
            [0.0, 0.0],
        ],
        &[
            (0, 4, RoadClass::Residential),
            (4, 1, RoadClass::Residential),
            (2, 4, RoadClass::Residential),
            (4, 3, RoadClass::Residential),
        ],
    );
    let net = RoadNetwork::new(g);
    let mut sim = TrafficSim::new(&net, Box::new(ClassRules), cfg(8, 20, TrafficLevel::Off));
    let bubble = Bubble {
        center: [0.0, 0.0],
        radius: 1000.0,
    };
    let routes = [
        vec![(0usize, true), (1usize, true)],
        vec![(1, false), (0, false)],
        vec![(2, true), (3, true)],
        vec![(3, false), (2, false)],
    ];
    let mut ids = Vec::new();
    for r in &routes {
        ids.push(
            sim.spawn_on(&net, VehicleClass::Car, r, 100.0, 8.0, 0)
                .expect("car"),
        );
    }
    let time = DemandTime::weekday_at(10.0);
    let mut crossed = std::collections::BTreeSet::new();
    for _ in 0..(60 * 50) {
        sim.step(&net, &[bubble], &[], &time);
        for (id, p) in sim.speeds().iter().map(|s| s.0).zip(sim.placements()) {
            let k = ids.iter().position(|&i| i == id).expect("ours");
            if (p.0, p.1) == routes[k][1] {
                crossed.insert(id);
            }
        }
    }
    assert_eq!(crossed.len(), 4, "the right-before-left deadlock resolves");

    // A minor road gives way to a car on the major road.
    let g = graph(
        &[[-400.0, 0.0], [0.0, 0.0], [400.0, 0.0], [0.0, 300.0]],
        &[
            (0, 1, RoadClass::Primary),
            (1, 2, RoadClass::Primary),
            (3, 1, RoadClass::Residential),
        ],
    );
    let net = RoadNetwork::new(g);
    let mut sim = TrafficSim::new(&net, Box::new(ClassRules), cfg(8, 20, TrafficLevel::Off));
    let major = sim
        .spawn_on(
            &net,
            VehicleClass::Car,
            &[(0, true), (1, true)],
            200.0,
            20.0,
            0,
        )
        .expect("major");
    let minor = sim
        .spawn_on(
            &net,
            VehicleClass::Car,
            &[(2, true), (1, true)],
            240.0,
            8.0,
            0,
        )
        .expect("minor");
    let mut minor_first = None;
    let mut major_first = None;
    for t in 0..(60 * 50) {
        sim.step(&net, &[bubble], &[], &time);
        for (id, (e, _, _)) in sim.speeds().iter().map(|s| s.0).zip(sim.placements()) {
            if e == 1 {
                if id == minor && minor_first.is_none() {
                    minor_first = Some(t);
                }
                if id == major && major_first.is_none() {
                    major_first = Some(t);
                }
            }
        }
    }
    let (m, j) = (
        minor_first.expect("minor crossed"),
        major_first.expect("major crossed"),
    );
    assert!(j < m, "the major road went first ({j} < {m})");
}

#[test]
fn traffic_lives_only_in_the_bubbles() {
    let (net, mut sim, bubble) = busy_world(21, 50, TrafficLevel::Normal);
    let time = DemandTime::weekday_at(17.0);
    let mut seen = 0;
    for t in 0..(20 * 60 * 20u32) {
        sim.step(&net, &[bubble], &[], &time);
        if t.is_multiple_of(20) {
            for v in sim.views(&net) {
                let (dx, dz) = (v.xz[0] - bubble.center[0], v.xz[1] - bubble.center[1]);
                let d = (dx * dx + dz * dz).sqrt();
                assert!(
                    d < bubble.radius + sim.config().margin_m + 120.0,
                    "vehicle {} at {d:.0} m",
                    v.id
                );
            }
            seen = seen.max(sim.len());
        }
    }
    assert!(seen > 30, "{seen}");
    assert!(
        sim.stats().despawned > 100,
        "vehicles leave: {:?}",
        sim.stats()
    );
    // No bubble, no traffic.
    for _ in 0..2_000 {
        sim.step(&net, &[], &[], &time);
    }
    assert!(sim.is_empty());
}

#[test]
fn lunatic_drivers_speed() {
    let (net, mut sim, bubble) = busy_world(13, 50, TrafficLevel::Lunatic);
    let time = DemandTime::weekday_at(12.0);
    sim.step(&net, &[bubble], &[], &time);
    let plans: Vec<plan::Plan> = sim
        .drain_events()
        .into_iter()
        .filter_map(|e| {
            if let TrafficEvent::Spawn(p) = e {
                Some(p)
            } else {
                None
            }
        })
        .collect();
    assert!(plans.len() > 10);
    let limit = |p: &plan::Plan| {
        let st = p.route[0];
        sim.rule(st.edge as usize)
            .speed
            .min(p.class.traits().top_speed)
    };
    let mean = plans
        .iter()
        .map(|p| f64::from(p.route[0].speed) / limit(p))
        .sum::<f64>()
        / plans.len() as f64;
    assert!(mean > 1.05, "lunatics speed: {mean:.2} × the limit");
    let fast = plans.iter().filter(|p| p.driver.headway < 0.9).count();
    assert!(fast * 3 > plans.len(), "short headways: {fast}");
}

#[test]
fn plans_of_a_reconnecting_client_match() {
    let (net, mut sim, bubble) = busy_world(17, 20, TrafficLevel::Normal);
    let time = DemandTime::weekday_at(9.0);
    for _ in 0..2_000 {
        sim.step(&net, &[bubble], &[], &time);
    }
    sim.drain_events();
    let plans = sim.current_plans();
    assert_eq!(plans.len(), sim.len());
    let curves = kin::Curves::new(&net);
    let mut client: Vec<PlanFollower> = plans.iter().map(PlanFollower::new).collect();
    for _ in 0..500 {
        sim.step(&net, &[bubble], &[], &time);
        for f in &mut client {
            f.step(&net, &curves, sim.dt());
        }
        plan::apply_events(&mut client, &sim.drain_events());
    }
    let views = sim.views(&net);
    assert_eq!(views.len(), client.len());
    for (v, f) in views.iter().zip(&client) {
        let (e, _, s) = f.on_edge(&net).expect("on route");
        assert_eq!(e, v.edge);
        assert!((s - v.s).abs() <= plan::POS_TOLERANCE_M + 0.05);
    }
}
