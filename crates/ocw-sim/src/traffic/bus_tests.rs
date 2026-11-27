//! Tests of the buses on their lines and the parked cars (spec 18B).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use ocw_world::RoadNetwork;
use ocw_world::graph::RoadClass;
use ocw_world::traffic::{
    BusRoute, BusStop, Country, EdgeRules as WorldEdge, Parking, ParkingArea, ParkingKind,
    ParkingOrientation, ParkingPlace, Side, TrafficData,
};

use super::bus::{self, BusLine};
use super::demand::{DemandTime, TrafficLevel};
use super::parked::{self, ParkedCars};
use super::plan::{self, PlanFollower, TrafficEvent};
use super::tests::{cfg, graph};
use super::*;

/// A main road of 2 km along x (secondary, vertices every 200 m, edges
/// 0 – 9 drawn eastwards) with side streets of 200 m north and south at
/// every inner vertex (residential, edges 10 – 27), in Liechtenstein, in
/// town. Bus line 1 runs east along the main road, line 1's return west;
/// stops at 100 m of edges 1, 3, 5, 7 (east) and 8, 6, 4, 2 (west).
fn street(parking: bool) -> RoadNetwork {
    let mut v: Vec<[f64; 2]> = (0..=10).map(|i| [f64::from(i) * 200.0, 0.0]).collect();
    let mut e: Vec<(u32, u32, RoadClass)> =
        (0..10).map(|i| (i, i + 1, RoadClass::Secondary)).collect();
    for i in 1..10u32 {
        for z in [-200.0, 200.0] {
            v.push([f64::from(i) * 200.0, z]);
            e.push((i, v.len() as u32 - 1, RoadClass::Residential));
        }
    }
    let g = graph(&v, &e);
    let mut edges = BTreeMap::new();
    for (i, ed) in g.edges.iter().enumerate() {
        let mut r = WorldEdge::implied(ed.class, Country::Li, true);
        if parking {
            let lane = Parking {
                place: ParkingPlace::Lane,
                orientation: ParkingOrientation::Parallel,
            };
            if i < 10 {
                // The main road: parking on both sides, but none on edge 9.
                r.parking = if i == 9 {
                    [Parking {
                        place: ParkingPlace::No,
                        orientation: ParkingOrientation::Parallel,
                    }; 2]
                } else {
                    [lane; 2]
                };
            } else if i % 2 == 0 {
                // Every other side street: tagged on both sides.
                r.parking = [lane; 2];
            }
        }
        edges.insert(ed.id, r);
    }
    let id = |i: usize| g.edges[i].id;
    let stop = |i: usize, side: Side| BusStop {
        osm_node: 7000 + i as i64,
        pos: [i as f32 * 200.0 + 100.0, 0.0],
        name: format!("Stop {i}"),
        edge: Some(id(i)),
        along_m: 100.0,
        side,
    };
    let mut stops = Vec::new();
    for i in [1, 3, 5, 7] {
        stops.push(stop(i, Side::Right));
    }
    for i in [8, 6, 4, 2] {
        stops.push(stop(i, Side::Left));
    }
    let east = BusRoute {
        osm_relation: 901,
        reference: "11".into(),
        name: "Bus 11: West => East End".into(),
        legs: (0..10).map(|i| (id(i), true)).collect(),
        stops: vec![0, 1, 2, 3],
    };
    let west = BusRoute {
        osm_relation: 902,
        reference: "11".into(),
        name: "Bus 11".into(),
        legs: (0..10).rev().map(|i| (id(i), false)).collect(),
        stops: vec![4, 5, 6, 7],
    };
    let parking_areas = vec![
        ParkingArea {
            osm_id: -55,
            pos: [1000.0, 120.0],
            capacity: 40,
            estimated: false,
            public: true,
            kind: ParkingKind::Surface,
            edge: Some(id(5)),
            along_m: 0.0,
        },
        ParkingArea {
            osm_id: -56,
            pos: [600.0, -120.0],
            capacity: 200,
            estimated: false,
            public: true,
            kind: ParkingKind::MultiStorey,
            edge: Some(id(3)),
            along_m: 0.0,
        },
    ];
    let mut data = TrafficData {
        country: Country::Li,
        edges,
        stops,
        bus_routes: vec![east, west],
        parking: parking_areas,
        ..Default::default()
    };
    data.sort();
    RoadNetwork::new(g).with_traffic(Some(Arc::new(data)), Country::Li)
}

fn sim(net: &RoadNetwork, seed: u64, level: TrafficLevel) -> TrafficSim {
    TrafficSim::new(
        net,
        Box::new(world::WorldRules::new(net)),
        cfg(seed, 100, level),
    )
}

const BUBBLE: Bubble = Bubble {
    center: [1000.0, 0.0],
    radius: 1500.0,
};

#[test]
fn lines_follow_the_relations_ways_with_their_stops() {
    let net = street(false);
    let s = sim(&net, 1, TrafficLevel::Normal);
    let lines = s.bus_lines();
    assert_eq!(lines.len(), 2, "{:?}", s.bus_line_counts());
    let east = &lines[0];
    assert_eq!(east.steps, (0..10).map(|i| (i, true)).collect::<Vec<_>>());
    assert_eq!(east.reference, "11");
    assert_eq!(east.destination, "East End", "the name after =>");
    assert_eq!(lines[1].destination, "Stop 2", "the last stop");
    assert_eq!(east.sign(), "11 East End");
    assert!((east.length - 2000.0).abs() < 1.0);
    let at: Vec<f64> = east.stops.iter().map(|s| s.at).collect();
    for (k, i) in [1.0, 3.0, 5.0, 7.0].iter().enumerate() {
        assert!(
            (at[k] - (i * 200.0 + 100.0 + bus::FRONT_PAST_M)).abs() < 0.5,
            "{at:?}"
        );
    }
    // The return: the same stops' places from the other end.
    let west = &lines[1];
    assert_eq!(west.steps[0], (9, false));
    for (k, i) in [8.0, 6.0, 4.0, 2.0].iter().enumerate() {
        let x = 2000.0 - (i * 200.0 + 100.0) + bus::FRONT_PAST_M;
        assert!((west.stops[k].at - x).abs() < 0.5, "{:?}", west.stops);
    }
    assert_eq!(bus::route_of(east.id), Some(0));
    assert_eq!(bus::route_of(west.id), Some(1));
    assert_eq!(bus::route_of(0), None);
}

#[test]
fn a_gap_in_the_relation_is_joined_and_platform_pairs_are_one_stop() {
    let mut net = street(false);
    let mut data = (*net.road_rules().data.expect("data")).clone();
    // Way 4 missing from the relation (off the extract), stop 1 mapped
    // twice (platform and stop position 3 m apart).
    data.bus_routes[0].legs.remove(4);
    let mut twin = data.stops[1].clone();
    twin.osm_node += 1;
    twin.along_m += 3.0;
    data.stops.push(twin);
    let n = data.stops.len() as u32 - 1;
    data.bus_routes[0].stops.insert(2, n);
    net = net.with_traffic(Some(Arc::new(data)), Country::Li);
    let s = sim(&net, 1, TrafficLevel::Normal);
    let east = &s.bus_lines()[0];
    assert_eq!(east.steps, (0..10).map(|i| (i, true)).collect::<Vec<_>>());
    assert_eq!(s.bus_line_counts().joined, 1);
    assert_eq!(east.stops.len(), 4, "{:?}", east.stops);
}

#[test]
fn headways_follow_the_hour_and_the_day() {
    let at = |weekday: u8, h: f64| DemandTime {
        weekday,
        seconds: (h * 3600.0) as u32,
        month: 6,
        holiday: false,
    };
    // Weekday: the base by day, two thirds in the peaks, double in the
    // evening, none at night.
    assert_eq!(bus::headway_s(15, &at(2, 10.5)), Some(900));
    assert_eq!(bus::headway_s(15, &at(2, 7.5)), Some(600));
    assert_eq!(bus::headway_s(15, &at(2, 17.0)), Some(600));
    assert_eq!(bus::headway_s(10, &at(2, 17.0)), Some(600));
    assert_eq!(bus::headway_s(15, &at(2, 21.0)), Some(1800));
    assert_eq!(bus::headway_s(15, &at(2, 2.0)), None);
    assert_eq!(bus::headway_s(30, &at(2, 23.5)), None);
    // Saturday a third more, Sunday double, at least 30 minutes.
    assert_eq!(bus::headway_s(15, &at(5, 10.5)), Some(1200));
    assert_eq!(bus::headway_s(15, &at(6, 10.5)), Some(1800));
    assert_eq!(bus::headway_s(20, &at(6, 10.5)), Some(2400));
    assert_eq!(bus::headway_s(15, &at(6, 6.0)), None);
    // A holiday is a Sunday.
    let holiday = DemandTime {
        holiday: true,
        ..at(2, 10.5)
    };
    assert_eq!(bus::headway_s(15, &holiday), Some(1800));
    // The departures of a day: rising, at the hour's headway.
    let line = BusLine {
        id: bus::line_id(0, 0),
        relation: 1,
        reference: "1".into(),
        destination: String::new(),
        steps: vec![(0, true)],
        start: vec![0.0],
        length: 1000.0,
        stops: Vec::new(),
        base_min: 15,
        phase_s: 420,
        offset_m: 0.0,
        night: false,
    };
    let count = |weekday| bus::departures(&line, &at(weekday, 12.0)).len();
    assert!(
        count(2) > count(5) && count(5) > count(6),
        "{} {} {}",
        count(2),
        count(5),
        count(6)
    );
    let d = bus::departures(&line, &at(2, 12.0));
    assert!(d.windows(2).all(|w| w[0] < w[1]));
    assert!(d.iter().all(|&s| s >= 5 * 3600), "none at night: {d:?}");
    let midday: Vec<u32> = d
        .iter()
        .copied()
        .filter(|&s| (36_000..43_200).contains(&s))
        .collect();
    assert!(midday.windows(2).all(|w| w[1] - w[0] == 900), "{midday:?}");
    assert_eq!(midday.len(), 8);
    // About 3 – 4 buses an hour by day on a weekday, one or two on Sunday.
    let per_day = [count(2), count(6)];
    assert!(
        (50..90).contains(&per_day[0]) && (20..40).contains(&per_day[1]),
        "{per_day:?}"
    );
    // A night line runs only in the nights to Saturday and Sunday.
    let n1 = BusLine {
        night: true,
        ..line.clone()
    };
    assert!(bus::departures(&n1, &at(2, 12.0)).is_empty());
    let sat = bus::departures(&n1, &at(5, 12.0));
    assert_eq!(sat.len(), 4);
    assert!(sat.iter().all(|&s| s < 4 * 3600));
}

/// Runs `minutes` from `start_h` on a weekday with a bubble over the
/// whole street; `each` sees the sim after every step.
fn run(
    net: &RoadNetwork,
    s: &mut TrafficSim,
    start_h: f64,
    minutes: u32,
    mut each: impl FnMut(&TrafficSim, &[TrafficEvent]),
) {
    let t0 = DemandTime::weekday_at(start_h);
    let per_s = 1000 / s.cfg.step_ms as u64;
    for k in 0..(u64::from(minutes) * 60 * per_s) {
        let time = t0.plus_seconds((k / per_s) as u32);
        s.step(net, &[BUBBLE], &[], &time);
        let ev = s.drain_events();
        each(s, &ev);
    }
}

#[test]
fn buses_follow_their_lines_and_stop_at_every_stop_with_the_doors_open() {
    let net = street(false);
    let mut s = sim(&net, 7, TrafficLevel::Normal);
    // Per bus: where it was first seen, the stops it stood at with the
    // doors open, and the ticks of each dwell.
    let mut first: BTreeMap<u32, (usize, f64)> = BTreeMap::new();
    let mut doors: BTreeMap<u32, BTreeSet<usize>> = BTreeMap::new();
    let mut dwell: BTreeMap<(u32, usize), (u64, u64)> = BTreeMap::new();
    let mut last: BTreeMap<u32, (usize, f64, usize)> = BTreeMap::new();
    let mut lines_seen = BTreeSet::new();
    let mut spawns = 0;
    run(&net, &mut s, 10.0, 40, |s, ev| {
        for e in ev {
            if let TrafficEvent::Spawn(p) = e
                && p.class == VehicleClass::Bus
            {
                assert_ne!(p.line, 0, "every bus runs on a line where lines exist");
                spawns += 1;
            }
        }
        for b in s.buses() {
            let line = &s.bus_lines()[b.line];
            first.entry(b.id).or_insert((b.line, b.at));
            last.insert(b.id, (b.line, b.at, b.next_stop));
            lines_seen.insert(b.line);
            if b.doors {
                doors.entry(b.id).or_default().insert(b.next_stop);
                let stop = &line.stops[b.next_stop];
                assert!(
                    (b.at - stop.at).abs() < 1.6,
                    "at the platform: {b:?} {stop:?}"
                );
                assert!(b.speed < 0.01);
                let d = dwell.entry((b.id, b.next_stop)).or_insert((s.tick(), 0));
                d.1 = s.tick();
            }
        }
        // Every bus on its line's edges, in the line's direction.
        for v in &s.vehicles {
            if let Some(b) = &v.bus {
                let st = v.f.route[v.pos.step];
                let line = &s.bus_lines()[b.line as usize];
                assert_eq!(
                    line.steps[b.base + v.pos.step],
                    (st.edge as usize, st.forward)
                );
            }
        }
    });
    assert!(spawns >= 4, "{spawns} buses in 40 minutes");
    assert_eq!(lines_seen.len(), 2, "both directions run");
    // Every stop a bus passed after it came in was served.
    let mut served = 0;
    for (id, &(li, from)) in &first {
        let (_, to, _) = last[id];
        let line = &s.bus_lines()[li];
        for (k, st) in line.stops.iter().enumerate() {
            if st.at > from + 60.0 && st.at < to - 1.0 {
                assert!(
                    doors.get(id).is_some_and(|d| d.contains(&k)),
                    "bus {id} passed stop {k} of line {li} without stopping ({from:.0} → {to:.0} m)"
                );
                served += 1;
            }
        }
    }
    assert!(served >= 6, "{served} stops served");
    // Dwells of 10 – 30 s (the doors open all but the last 2 s).
    for (&(id, k), &(a, b)) in &dwell {
        let open_s = (b - a + 1) as f64 * 0.1;
        let (_, _, next) = last[&id];
        if next > k {
            assert!(
                (7.9..=28.1).contains(&open_s),
                "bus {id} stop {k}: {open_s:.1} s"
            );
        }
    }
}

#[test]
fn bus_timetables_are_deterministic_and_reproduced_by_the_clients() {
    let net = street(false);
    let curves = Curves::new(&net);
    let mut digests = Vec::new();
    for _ in 0..2 {
        let mut s = sim(&net, 11, TrafficLevel::Normal);
        let mut followers: Vec<PlanFollower> = Vec::new();
        let mut buses: Vec<(u32, u32)> = Vec::new();
        let mut doors_seen = 0;
        run(&net, &mut s, 8.0, 15, |s, ev| {
            for f in &mut followers {
                f.advance_to(&net, &curves, s.tick(), s.dt());
            }
            plan::apply_events(&mut followers, ev);
            for e in ev {
                if let TrafficEvent::Spawn(p) = e
                    && p.line != 0
                {
                    buses.push((p.id, p.line));
                }
            }
            // A client's followers open the doors at the server's ticks
            // and stand where the server's bus stands.
            for b in s.buses() {
                let f = followers.iter().find(|f| f.id == b.id).expect("followed");
                assert_eq!(f.doors, b.doors, "bus {} at tick {}", b.id, s.tick());
                doors_seen += usize::from(b.doors);
            }
        });
        assert!(doors_seen > 0);
        digests.push((s.digest(), buses));
    }
    assert_eq!(digests[0], digests[1]);
    // Another seed: other phases.
    let other = sim(&net, 12, TrafficLevel::Normal);
    let a = sim(&net, 11, TrafficLevel::Normal);
    assert!(
        other.bus_lines()[0].phase_s != a.bus_lines()[0].phase_s
            || other.bus_lines()[1].phase_s != a.bus_lines()[1].phase_s
    );
}

#[test]
fn traffic_waits_behind_a_bus_at_its_stop() {
    let net = street(false);
    let mut s = sim(&net, 3, TrafficLevel::Busy);
    let mut queued = 0;
    run(&net, &mut s, 17.0, 30, |s, _| {
        for (i, v) in s.vehicles.iter().enumerate() {
            let Some(b) = &v.bus else { continue };
            if b.dwell_end.is_none() {
                continue;
            }
            let st = v.f.route[v.pos.step];
            for (j, o) in s.vehicles.iter().enumerate() {
                if i == j {
                    continue;
                }
                let os = o.f.route[o.pos.step];
                if (os.edge, os.forward, o.lane) != (st.edge, st.forward, v.lane) {
                    continue;
                }
                // Nobody inside the standing bus …
                assert!(
                    o.pos.s <= v.pos.s - v.length + 0.05 || o.pos.s - o.length >= v.pos.s - 0.05,
                    "vehicle {} drove into the bus {} at its stop",
                    o.f.id,
                    v.f.id
                );
                // … and some wait behind it.
                let gap = v.pos.s - v.length - o.pos.s;
                if (0.0..12.0).contains(&gap) && o.v < 0.5 {
                    queued += 1;
                }
            }
        }
    });
    assert!(queued > 0, "no vehicle waited behind a bus at its stop");
}

#[test]
fn parked_cars_stand_outside_the_lanes_and_the_riders_line() {
    let net = Arc::new(street(true));
    let p = ParkedCars::new(net.clone()).expect("traffic data");
    let cars = p.near([1000.0, 0.0], 1600.0);
    assert!(cars.len() > 100, "{} cars", cars.len());
    let data = net.road_rules().data.expect("data");
    for c in &cars {
        let Some(e) = c.edge else { continue };
        let e = e as usize;
        let edge = net.edge(e);
        let rule = world::edge_rule(&net.road_rules().edge(edge), edge.class);
        let lane = lateral_offset(&rule, edge.width_m, 0);
        let g = net.geometry(e);
        for q in c.corners() {
            // Each corner's distance from the centreline.
            let d = g
                .points
                .windows(2)
                .map(|w| {
                    ocw_world::obstacles::segment_distance(
                        q,
                        [w[0][0], w[0][2]],
                        [w[1][0], w[1][2]],
                    )
                })
                .fold(f64::INFINITY, f64::min);
            assert!(
                d >= lane + 1.28 + parked::LANE_CLEAR_M - 0.05,
                "a corner {d:.2} m from the centreline of edge {e} (lane {lane:.2})"
            );
            assert!(d >= parked::RIDERS_CLEAR_M - 0.05);
        }
        // Not near the junctions or the stops.
        let len = g.length();
        let (s, _) = {
            let mut best = (0.0, f64::INFINITY);
            for (k, w) in g.points.windows(2).enumerate() {
                let d = ocw_world::obstacles::segment_distance(
                    [c.pos[0], c.pos[2]],
                    [w[0][0], w[0][2]],
                    [w[1][0], w[1][2]],
                );
                if d < best.1 {
                    best = (g.cumulative[k], d);
                }
            }
            best
        };
        assert!(s > parked::JUNCTION_GAP_M - 3.0 && s < len - parked::JUNCTION_GAP_M + 3.0);
        for st in data.stops.iter().filter(|st| st.edge == Some(edge.id)) {
            assert!((f64::from(st.along_m) - s).abs() > parked::STOP_GAP_M - 4.0);
        }
        // On no other road.
        for o in net.edges_near([c.pos[0], c.pos[2]], 20.0) {
            if o == e {
                continue;
            }
            let og = net.geometry(o);
            for q in c.corners() {
                let d = og
                    .points
                    .windows(2)
                    .map(|w| {
                        ocw_world::obstacles::segment_distance(
                            q,
                            [w[0][0], w[0][2]],
                            [w[1][0], w[1][2]],
                        )
                    })
                    .fold(f64::INFINITY, f64::min);
                assert!(d >= net.edge(o).width_m / 2.0, "on edge {o}");
            }
        }
    }
    // None where parking is forbidden (edge 9 of the main road).
    assert!(cars.iter().all(|c| c.edge != Some(9)));
}

#[test]
fn parked_cars_per_km_by_street_type() {
    let net = Arc::new(street(true));
    let p = ParkedCars::new(net.clone()).expect("traffic data");
    let cars = p.near([1000.0, 0.0], 1600.0);
    let mut per_edge: BTreeMap<u32, usize> = BTreeMap::new();
    for c in &cars {
        if let Some(e) = c.edge {
            *per_edge.entry(e).or_default() += 1;
        }
    }
    let per_km = |edges: &[usize]| {
        let n: usize = edges
            .iter()
            .map(|e| per_edge.get(&(*e as u32)).copied().unwrap_or(0))
            .sum();
        let km: f64 = edges.iter().map(|&e| net.geometry(e).length()).sum::<f64>() / 1000.0;
        n as f64 / km
    };
    let main = per_km(&(0..9).collect::<Vec<_>>());
    let tagged_side: Vec<usize> = (10..28).filter(|i| i % 2 == 0).collect();
    let untagged_side: Vec<usize> = (10..28).filter(|i| i % 2 == 1).collect();
    let (tagged, guessed) = (per_km(&tagged_side), per_km(&untagged_side));
    // Both sides of slots of 6 m: 330 a km at most; residential 65 %,
    // secondary 40 %, untagged town streets half.
    assert!(
        (120.0..260.0).contains(&tagged),
        "residential {tagged:.0}/km"
    );
    assert!((50.0..140.0).contains(&main), "secondary {main:.0}/km");
    assert!(
        guessed > 0.3 * tagged && guessed < 0.75 * tagged,
        "untagged {guessed:.0}/km"
    );
    assert!(tagged > main);
}

#[test]
fn parked_cars_are_the_same_everywhere_and_in_car_parks() {
    let net = Arc::new(street(true));
    let a = ParkedCars::new(net.clone()).expect("data");
    // Another copy of the same world (a client), asked in another order.
    let b = ParkedCars::new(Arc::new(street(true))).expect("data");
    let far = b.near([1800.0, 0.0], 300.0);
    let near_b = b.near([200.0, 0.0], 300.0);
    assert_eq!(a.near([200.0, 0.0], 300.0), near_b);
    assert_eq!(a.near([1800.0, 0.0], 300.0), far);
    let ids: BTreeSet<u32> = a.near([1000.0, 0.0], 1600.0).iter().map(|c| c.id).collect();
    assert_eq!(ids.len(), a.near([1000.0, 0.0], 1600.0).len(), "unique ids");
    assert!(ids.iter().all(|&i| i & 0x8000_0000 != 0));
    // The surface car park of 40 spaces about half full, none in the
    // multi-storey one.
    let lot: Vec<_> = a
        .near([1000.0, 120.0], 40.0)
        .into_iter()
        .filter(|c| c.edge.is_none())
        .collect();
    assert!(
        (10..=32).contains(&lot.len()),
        "{} in the car park",
        lot.len()
    );
    assert!(
        a.near([600.0, -120.0], 25.0)
            .iter()
            .all(|c| c.edge.is_some())
    );
    // A world without traffic data has none.
    let plain = Arc::new(RoadNetwork::new(street(false).graph().clone()));
    assert!(ParkedCars::new(plain).is_none());
    // Solids for the riders.
    let mut out = Vec::new();
    let c = lot[0];
    a.solids(
        [c.pos[0] - 1.0, c.pos[2] - 1.0],
        [c.pos[0] + 1.0, c.pos[2] + 1.0],
        &mut out,
    );
    assert!(
        out.iter()
            .any(|s| s.kind == ocw_world::obstacles::ObstacleKind::ParkedCar)
    );
}

#[test]
fn a_rider_steering_into_a_parked_car_hits_it() {
    let net = Arc::new(street(true));
    let p = Arc::new(ParkedCars::new(net.clone()).expect("data"));
    let car = p
        .near([1000.0, 0.0], 400.0)
        .into_iter()
        .find(|c| c.edge == Some(4))
        .expect("a car on edge 4");
    let mut cache = crate::collide::WorldCache::default();
    cache.parked = Some(p);
    let body = crate::collide::Body {
        pos: [car.pos[0] - 1.6, car.pos[2]],
        ..crate::collide::Body::on_path(ocw_core::RiderId(1), 0.0, 0.0, 0.0, 8.0, 80.0)
    };
    let mut scratch = Vec::new();
    let r = cache.resolve(
        &body,
        0.0,
        1,
        crate::collide::Mode::Soft,
        1.0,
        false,
        &mut scratch,
    );
    assert!(r.touched() || r.speed.is_some(), "{r:?}");
    assert!(
        scratch
            .iter()
            .any(|s| s.kind == ocw_world::obstacles::ObstacleKind::ParkedCar)
    );
}
