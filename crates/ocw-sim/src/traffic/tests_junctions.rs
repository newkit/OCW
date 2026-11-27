//! Tests of the junction logic against queues and gridlocks (spec 18H,
//! 18E T3 / T5): the permanent queue at two junctions 140 m apart on the
//! Liechtenstein main road (vertices 9 and 3710 of the package), where
//! short side-road edges (5 – 7 m) meet the main road.

use std::collections::{BTreeMap, BTreeSet};

use ocw_world::RoadNetwork;
use ocw_world::graph::RoadClass;

use super::demand::{DemandTime, TrafficLevel};
use super::rules::{ClassRules, VehicleClass};
use super::tests::{cfg, graph};
use super::*;

/// A primary road west–east through vertex 1 at (0, 0) and a service road
/// 5 m long north to vertex 3, where it forks (vertices 4 and 5).
/// Edges: 0 = 0 → 1 and 1 = 1 → 2 (primary), 2 = 1 → 3 (5 m), 3 = 3 → 4,
/// 4 = 3 → 5 (service).
fn side_road() -> (RoadNetwork, TrafficSim, Bubble) {
    let g = graph(
        &[
            [-300.0, 0.0],
            [0.0, 0.0],
            [300.0, 0.0],
            [0.0, 5.0],
            [0.0, 205.0],
            [200.0, 5.0],
        ],
        &[
            (0, 1, RoadClass::Primary),
            (1, 2, RoadClass::Primary),
            (1, 3, RoadClass::Service),
            (3, 4, RoadClass::Service),
            (3, 5, RoadClass::Service),
        ],
    );
    let net = RoadNetwork::new(g);
    let sim = TrafficSim::new(&net, Box::new(ClassRules), cfg(5, 20, TrafficLevel::Off));
    let bubble = Bubble {
        center: [0.0, 0.0],
        radius: 1_000.0,
    };
    (net, sim, bubble)
}

/// The edge vehicle `id` is on, if it is still there.
fn edge_of(sim: &TrafficSim, id: u32) -> Option<(usize, bool)> {
    sim.speeds()
        .iter()
        .zip(sim.placements())
        .find(|(s, _)| s.0 == id)
        .map(|(_, p)| (p.0, p.1))
}

fn speed_of(sim: &TrafficSim, id: u32) -> f64 {
    sim.speeds()
        .iter()
        .find(|s| s.0 == id)
        .map_or(f64::NAN, |s| s.1)
}

#[test]
fn a_vehicle_leaving_on_the_arrival_edge_blocks_nobody() {
    // B turns from the main road into the 5 m side road and stops before
    // its fork for a rider: it stands in the main-road junction, but in
    // the other half of the side road. A, coming out of the side road,
    // must not wait for it (the old "anyone in the junction" rule held A
    // as long as B stood: the two-junction queue of spec 18H).
    let (net, mut sim, bubble) = side_road();
    let time = DemandTime::weekday_at(10.0);
    let rider = RoadUser {
        id: 1,
        edge: 4,
        s: 3.0,
        forward: false,
        speed: 0.0,
        length: 1.8,
        ..RoadUser::default()
    };
    let b = sim
        .spawn_on(
            &net,
            VehicleClass::Car,
            &[(0, true), (2, true), (3, true)],
            296.0,
            2.0,
            0,
        )
        .expect("B");
    let mut t = 0;
    while !(edge_of(&sim, b) == Some((2, true)) && speed_of(&sim, b) < 0.3) {
        sim.step(&net, &[bubble], &[rider], &time);
        t += 1;
        assert!(t < 30 * 50, "B stands in the side road");
    }
    let a = sim
        .spawn_on(
            &net,
            VehicleClass::Car,
            &[(2, false), (1, true)],
            0.5,
            0.0,
            0,
        )
        .expect("A");
    let mut out = None;
    for k in 0..(20 * 50) {
        sim.step(&net, &[bubble], &[rider], &time);
        if out.is_none() && edge_of(&sim, a) == Some((1, true)) {
            out = Some(k);
        }
    }
    let k = out.expect("A came out of the side road while B stood");
    assert!(k < 10 * 50, "A came out after {} s", k / 50);
    assert_eq!(
        edge_of(&sim, b),
        Some((2, true)),
        "B still waits for the rider"
    );
}

#[test]
fn no_entry_without_room_over_a_short_exit() {
    // Q stands just past the fork, 4 m into its edge: the 5 m side road
    // has no room for P's 4.5 m car. P waits on the main road before the
    // junction (don't block the box) instead of stopping with its rear
    // across the main road; when Q leaves, P turns in.
    let (net, mut sim, bubble) = side_road();
    let time = DemandTime::weekday_at(10.0);
    let q = sim
        .spawn_on(&net, VehicleClass::Car, &[(3, true)], 4.0, 0.0, 0)
        .expect("Q");
    let p = sim
        .spawn_on(
            &net,
            VehicleClass::Car,
            &[(0, true), (2, true), (3, true)],
            250.0,
            10.0,
            0,
        )
        .expect("P");
    for _ in 0..(30 * 50) {
        sim.set_speed(q, 0.0);
        sim.step(&net, &[bubble], &[], &time);
        assert_eq!(
            edge_of(&sim, p),
            Some((0, true)),
            "P stays on the main road"
        );
    }
    let at = sim.views(&net).into_iter().find(|v| v.id == p).expect("P");
    assert!(at.speed < 0.3, "P waits");
    assert!(
        at.s > 280.0 && at.s < 300.0,
        "P waits at the junction: {:.1} m",
        at.s
    );
    let mut turned = false;
    for _ in 0..(30 * 50) {
        sim.step(&net, &[bubble], &[], &time);
        turned |= matches!(edge_of(&sim, p), Some((2 | 3, true)) | None);
    }
    assert!(turned, "P turned in once Q had left");
}

#[test]
fn a_major_vehicle_held_at_the_line_does_not_hold_the_side_road() {
    // P on the main road waits at the junction to turn into the side road
    // (no room behind it, Q stands at the fork); S comes out of the side
    // road. S gives way to the main road, but P is not coming: S goes. The
    // old rule had S wait for P and P for the room S's queue would make:
    // the gridlock at vertex 3713 of the package (spec 18H).
    let (net, mut sim, bubble) = side_road();
    let time = DemandTime::weekday_at(10.0);
    let q = sim
        .spawn_on(&net, VehicleClass::Car, &[(3, true)], 4.0, 0.0, 0)
        .expect("Q");
    let p = sim
        .spawn_on(
            &net,
            VehicleClass::Car,
            &[(0, true), (2, true), (3, true)],
            250.0,
            10.0,
            0,
        )
        .expect("P");
    for _ in 0..(20 * 50) {
        sim.set_speed(q, 0.0);
        sim.step(&net, &[bubble], &[], &time);
    }
    assert!(speed_of(&sim, p) < 0.3, "P waits at the line");
    let s = sim
        .spawn_on(
            &net,
            VehicleClass::Car,
            &[(2, false), (0, false)],
            0.5,
            0.0,
            0,
        )
        .expect("S");
    let mut out = false;
    for _ in 0..(15 * 50) {
        sim.set_speed(q, 0.0);
        sim.step(&net, &[bubble], &[], &time);
        out |= edge_of(&sim, s) == Some((0, false));
    }
    assert!(out, "S came out past the waiting P");
    let w = sim.waits(&net);
    let held = w.iter().find(|w| w.id == p).expect("P still waits");
    assert_eq!(held.why, Wait::ExitFull);
}

/// The two junctions of the Liechtenstein main road (spec 18H): the motor
/// roads within 450 m of the middle between vertices 9 and 3710 of the
/// package, as `traffic_bench --view -2820,-1218,450 --fixture` writes
/// them (OSM-derived, ODbL); coordinates relative to that middle.
fn two_junctions() -> RoadNetwork {
    use ocw_world::graph::{Edge, EdgeId, Oneway, RoadGraph, VertexId};
    let text = include_str!("fixtures/two_junctions.txt");
    let mut vertices = Vec::new();
    let mut edges = Vec::new();
    let num = |x: &str| x.parse::<f64>().expect("number");
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        let f: Vec<&str> = line.split_whitespace().collect();
        match f[0] {
            "v" => vertices.push([num(f[1]), num(f[2])]),
            "e" => {
                let class = match f[3] {
                    "Trunk" => RoadClass::Trunk,
                    "Primary" => RoadClass::Primary,
                    "Secondary" => RoadClass::Secondary,
                    "Tertiary" => RoadClass::Tertiary,
                    "Unclassified" => RoadClass::Unclassified,
                    "Residential" => RoadClass::Residential,
                    "Service" => RoadClass::Service,
                    "Track" => RoadClass::Track,
                    other => panic!("class {other}"),
                };
                let oneway = match f[4] {
                    "Forward" => Oneway::Forward,
                    "Backward" => Oneway::Backward,
                    _ => Oneway::No,
                };
                let points = f[6..]
                    .iter()
                    .map(|p| {
                        let (x, z) = p.split_once(',').expect("x,z");
                        [num(x), num(z)]
                    })
                    .collect();
                let i = edges.len();
                edges.push(Edge {
                    id: EdgeId(9000 + i as u64),
                    from: VertexId(f[1].parse().expect("from")),
                    to: VertexId(f[2].parse().expect("to")),
                    points,
                    class,
                    name: None,
                    width_m: num(f[5]),
                    bridge: false,
                    tunnel: false,
                    dead_end_m: None,
                    boundary: false,
                    oneway,
                    osm_way: i as i64,
                    profile: Vec::new(),
                });
            }
            other => panic!("line {other}"),
        }
    }
    RoadNetwork::new(RoadGraph {
        vertices,
        edges,
        boundary_vertices: Default::default(),
    })
}

/// Vehicles in a closed cycle of waits: each waits for the one it yields
/// to (or the one in its full exit, or in the junction), else for the
/// one ahead, and the chain comes back to it.
fn gridlocked(sim: &mut TrafficSim, net: &RoadNetwork) -> usize {
    let waits = sim.waits(net);
    let next: BTreeMap<u32, u32> = waits
        .iter()
        .filter_map(|w| w.blocker.or(w.behind).map(|b| (w.id, b)))
        .collect();
    let mut stuck = BTreeSet::new();
    for w in &waits {
        let mut seen = Vec::new();
        let mut cur = w.id;
        while let Some(&n) = next.get(&cur) {
            if let Some(at) = seen.iter().position(|&x| x == cur) {
                stuck.extend(seen[at..].iter().copied());
                break;
            }
            seen.push(cur);
            cur = n;
        }
    }
    stuck.len()
}

/// Runs the two-junction road at busy traffic at 17:30 for `minutes`
/// (steps of `step_ms`): (standing share, vehicles that left per hour, the longest a
/// vehicle stood without moving (s), vehicles in a gridlock at the end).
fn run_two_junctions_with(minutes: u32, seed: u64, step_ms: u32) -> (f64, f64, f64, usize) {
    let hz = 1000 / step_ms;
    let net = two_junctions();
    let mut c = cfg(seed, step_ms, TrafficLevel::Busy);
    c.max_vehicles = 300;
    let mut sim = TrafficSim::new(&net, Box::new(ClassRules), c);
    let bubble = Bubble {
        center: [0.0, 0.0],
        radius: 450.0,
    };
    let start = DemandTime::weekday_at(17.5);
    let warm = 60 * hz;
    let ticks = minutes * 60 * hz;
    let (mut standing, mut sampled) = (0u64, 0u64);
    let mut stood: BTreeMap<u32, f64> = BTreeMap::new();
    let mut longest = 0.0f64;
    let mut gone0 = 0;
    for t in 0..warm + ticks {
        sim.step(&net, &[bubble], &[], &start.plus_seconds(t / hz));
        if t == warm {
            gone0 = sim.stats().despawned;
        }
        if t >= warm && t % hz == 0 {
            let now = sim.speeds();
            let ids: BTreeSet<u32> = now.iter().map(|s| s.0).collect();
            stood.retain(|id, _| ids.contains(id));
            for (id, v) in now {
                sampled += 1;
                let s = stood.entry(id).or_insert(0.0);
                if v < 0.3 {
                    standing += 1;
                    *s += 1.0;
                    longest = longest.max(*s);
                } else {
                    *s = 0.0;
                }
            }
        }
    }
    let left = (sim.stats().despawned - gone0) as f64 * 60.0 / f64::from(minutes);
    let share = standing as f64 / sampled.max(1) as f64;
    (share, left, longest, gridlocked(&mut sim, &net))
}

#[test]
fn the_two_junction_road_flows_for_an_hour_at_busy_traffic() {
    // An hour of the evening peak on the real geometry, class rules (two
    // seeds the old rules locked: vehicles stood 82 % and 33 % of the
    // time, the side roads and the main road behind them for 10 minutes
    // and more); now queues come and go.
    for seed in [1, 4] {
        let (share, left, longest, stuck) = run_two_junctions_with(60, seed, 50);
        eprintln!(
            "two junctions, busy, 60 min, seed {seed}: standing {:.1} %, {left:.0} vehicles/h left, longest stand {longest:.0} s, {stuck} in a gridlock",
            100.0 * share
        );
        assert_eq!(stuck, 0, "seed {seed}: no gridlock at the end");
        assert!(
            share < 0.05,
            "seed {seed}: standing {:.1} % of the time",
            100.0 * share
        );
        assert!(
            left > 1000.0,
            "seed {seed}: {left:.0} vehicles an hour leave"
        );
        assert!(
            longest < 120.0,
            "seed {seed}: a vehicle stood {longest:.0} s"
        );
    }
}
