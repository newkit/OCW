//! Tests of T4 – around riders (spec 18E T4): passing distances,
//! waiting, groups, junctions, manners, lunatics, contacts, gusts, the
//! proximity cue, determinism and the clients' followers.

use ocw_world::RoadNetwork;
use ocw_world::graph::{Edge, EdgeId, Oneway, RoadClass, RoadGraph, VertexId};

use super::around::{self, Country, PassRule};
use super::demand::{DemandTime, DriverManners, TrafficLevel};
use super::plan::{self, PlanFollower};
use super::rules::{self, ClassRules, EdgeRule, EdgeRules, VehicleClass};
use super::*;
use crate::collide;

const DT: f64 = 0.02;

fn edge(i: usize, a: u32, b: u32, points: Vec<[f64; 2]>, class: RoadClass) -> Edge {
    Edge {
        id: EdgeId(7000 + i as u64),
        from: VertexId(a),
        to: VertexId(b),
        points,
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
    }
}

/// One road along `points` (a single edge), heights every 5 m.
fn one_road(points: Vec<[f64; 2]>, class: RoadClass, profile: Vec<f64>) -> RoadNetwork {
    let a = points[0];
    let b = *points.last().expect("points");
    let mut e = edge(0, 0, 1, points, class);
    e.profile = profile;
    RoadNetwork::new(RoadGraph {
        vertices: vec![a, b],
        edges: vec![e],
        boundary_vertices: Default::default(),
    })
}

fn straight(len: f64) -> Vec<[f64; 2]> {
    vec![[0.0, 0.0], [len, 0.0]]
}

/// A straight of `a` m, a 90° bend of radius `r`, a straight of `b` m.
fn bend(a: f64, r: f64, b: f64) -> Vec<[f64; 2]> {
    let mut p = vec![[0.0, 0.0], [a, 0.0]];
    let n = 24;
    for k in 1..=n {
        let t = f64::from(k) / f64::from(n) * std::f64::consts::FRAC_PI_2;
        p.push([a + r * t.sin(), r - r * t.cos()]);
    }
    p.push([a + r, r + b]);
    p
}

fn sim(
    net: &RoadNetwork,
    rules: Box<dyn EdgeRules + Send + Sync>,
    seed: u64,
    manners: DriverManners,
    level: TrafficLevel,
    collisions: bool,
) -> TrafficSim {
    TrafficSim::new(
        net,
        rules,
        TrafficConfig {
            seed,
            level,
            manners,
            collisions,
            ..TrafficConfig::default()
        },
    )
}

fn everywhere() -> Bubble {
    Bubble {
        center: [1500.0, 0.0],
        radius: 6000.0,
    }
}

/// A rider riding straight along edge 0 at a constant speed.
#[derive(Clone, Copy)]
struct Rider {
    id: u32,
    edge: usize,
    /// Front wheel along the riding direction (m).
    d: f64,
    speed: f64,
    lateral: f64,
    forward: bool,
}

impl Rider {
    fn user(&self, net: &RoadNetwork) -> RoadUser {
        let len = net.geometry(self.edge).length();
        RoadUser {
            id: self.id,
            edge: self.edge,
            s: if self.forward { self.d } else { len - self.d },
            forward: self.forward,
            speed: self.speed,
            length: collide::BODY_LENGTH_M,
            lateral: self.lateral,
        }
    }
}

fn passes(ev: &[RiderEvent]) -> Vec<PassRecord> {
    ev.iter()
        .filter_map(|e| match e {
            RiderEvent::Passed(p) => Some(*p),
            _ => None,
        })
        .collect()
}

fn contacts(ev: &[RiderEvent]) -> Vec<VehicleContact> {
    ev.iter()
        .filter_map(|e| match e {
            RiderEvent::Contact(c) => Some(*c),
            _ => None,
        })
        .collect()
}

// --- The stadium: 10,000 overtakes --------------------------------------

/// A closed loop of two 4 km straights and two bends of 150 m radius,
/// edges of 250 m on the straights; the first 40 % of each straight is
/// unclassified (50 km/h: in town, 1.5 m), the rest secondary (70 km/h:
/// outside, 2 m). Returns the network and the edges' cumulative starts.
pub(super) fn stadium() -> (RoadNetwork, Vec<f64>) {
    let (l, r) = (4000.0, 150.0);
    let mut pts: Vec<([f64; 2], RoadClass)> = Vec::new();
    let seg = 250.0;
    let n = (l / seg) as usize;
    let straight_class = |k: usize| {
        if (k as f64) < 0.4 * n as f64 {
            RoadClass::Unclassified
        } else {
            RoadClass::Secondary
        }
    };
    for k in 0..n {
        pts.push(([k as f64 * seg, 0.0], straight_class(k)));
    }
    let arc = 12;
    for k in 0..arc {
        let t = -std::f64::consts::FRAC_PI_2 + std::f64::consts::PI * k as f64 / arc as f64;
        pts.push(([l + r * t.cos(), r + r * t.sin()], RoadClass::Secondary));
    }
    for k in 0..n {
        pts.push(([l - k as f64 * seg, 2.0 * r], straight_class(k)));
    }
    for k in 0..arc {
        let t = std::f64::consts::FRAC_PI_2 + std::f64::consts::PI * k as f64 / arc as f64;
        pts.push(([r * t.cos(), r + r * t.sin()], RoadClass::Secondary));
    }
    let m = pts.len();
    let vertices: Vec<[f64; 2]> = pts.iter().map(|p| p.0).collect();
    let edges: Vec<Edge> = (0..m)
        .map(|k| {
            let (a, b) = (k, (k + 1) % m);
            edge(
                k,
                a as u32,
                b as u32,
                vec![vertices[a], vertices[b]],
                pts[k].1,
            )
        })
        .collect();
    let net = RoadNetwork::new(RoadGraph {
        vertices,
        edges,
        boundary_vertices: Default::default(),
    });
    let mut cum = Vec::with_capacity(m + 1);
    let mut c = 0.0;
    for e in 0..m {
        cum.push(c);
        c += net.geometry(e).length();
    }
    cum.push(c);
    (net, cum)
}

/// A rider on the stadium: a position along the loop (m, in edge order)
/// and its direction.
#[derive(Clone, Copy)]
pub(super) struct LoopRider {
    pub id: u32,
    pub p: f64,
    pub speed: f64,
    pub lateral: f64,
    pub forward: bool,
}

impl LoopRider {
    pub fn advance(&mut self, total: f64, dt: f64) {
        let dir = if self.forward { 1.0 } else { -1.0 };
        self.p = (self.p + dir * self.speed * dt).rem_euclid(total);
    }

    pub fn user(&self, cum: &[f64]) -> RoadUser {
        let e = cum.partition_point(|&c| c <= self.p).saturating_sub(1);
        let e = e.min(cum.len() - 2);
        RoadUser {
            id: self.id,
            edge: e,
            s: self.p - cum[e],
            forward: self.forward,
            speed: self.speed,
            length: collide::BODY_LENGTH_M,
            lateral: self.lateral,
        }
    }
}

/// 80 riders in groups of 1 – 4 (1 – 2.5 m wheel gaps), both ways (7.5
/// and 6 m/s), straight lines 1.8 – 2.2 m right of the centreline (0.3 – 0.7
/// m from the edge of the narrower road, 1 – 1.5 m from the wider one's).
pub(super) fn stadium_riders(total: f64, seed: u64) -> Vec<LoopRider> {
    let mut h = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut u = || {
        h ^= h << 13;
        h ^= h >> 7;
        h ^= h << 17;
        (h >> 11) as f64 / (1u64 << 53) as f64
    };
    let mut out = Vec::new();
    let mut id = 1;
    let mut g = 0;
    while out.len() < 80 {
        let size = 1 + (u() * 4.0) as usize;
        let p0 = u() * total;
        // One speed a direction: scripted riders do not avoid vehicles,
        // so they must not catch up with a queue (or each other).
        let speed = if g % 2 == 0 { 7.5 } else { 6.0 };
        let lateral = 1.8 + 0.4 * u();
        let forward = g % 2 == 0;
        let mut p = p0;
        for _ in 0..size {
            out.push(LoopRider {
                id,
                p,
                speed,
                lateral,
                forward,
            });
            id += 1;
            let gap = collide::BODY_LENGTH_M + 1.0 + 1.5 * u();
            p = (p - if forward { gap } else { -gap }).rem_euclid(total);
        }
        g += 1;
    }
    out
}

/// Runs the stadium until `want` overtakes (or the time cap): the passes
/// and the contacts.
pub(super) fn run_stadium(
    manners: DriverManners,
    level: TrafficLevel,
    seed: u64,
    want: usize,
    max_s: f64,
) -> (Vec<PassRecord>, Vec<VehicleContact>, u64) {
    let (net, cum) = stadium();
    let total = *cum.last().expect("loop");
    let mut sim = sim(&net, Box::new(ClassRules), seed, manners, level, true);
    let bubble = Bubble {
        center: [2000.0, 150.0],
        radius: 2400.0,
    };
    let mut riders = stadium_riders(total, seed);
    let time = DemandTime::weekday_at(10.0);
    let (mut all, mut hits) = (Vec::new(), Vec::new());
    let mut waits = 0u64;
    let ticks = (max_s / DT) as u64;
    for t in 0..ticks {
        let users: Vec<RoadUser> = riders.iter().map(|r| r.user(&cum)).collect();
        sim.step(&net, &[bubble], &users, &time.plus_seconds((t / 50) as u32));
        let ev = sim.drain_rider_events();
        all.extend(passes(&ev));
        hits.extend(contacts(&ev));
        if t % 50 == 0 {
            waits += sim.speeds().iter().filter(|(_, v)| *v < 11.0).count() as u64;
        }
        for r in &mut riders {
            r.advance(total, DT);
        }
        if all.len() >= want {
            break;
        }
    }
    (all, hits, waits)
}

#[test]
fn ten_thousand_overtakes_keep_the_legal_distance() {
    for manners in [DriverManners::Polite, DriverManners::Honking] {
        let (all, hits, _) = run_stadium(manners, TrafficLevel::Busy, 3, 10_000, 8.0 * 3600.0);
        assert!(
            all.len() >= 10_000,
            "{}: only {} overtakes",
            manners.name(),
            all.len()
        );
        let close: Vec<&PassRecord> = all
            .iter()
            .filter(|p| p.clearance_m < p.legal_m - 1e-6)
            .collect();
        assert!(
            close.is_empty(),
            "{}: {} of {} passes closer than the law, first {:?}",
            manners.name(),
            close.len(),
            all.len(),
            close.first()
        );
        assert!(
            hits.is_empty(),
            "{}: contacts {:?}",
            manners.name(),
            hits.first()
        );
        let min = all
            .iter()
            .map(|p| p.clearance_m)
            .fold(f64::INFINITY, f64::min);
        let out = all.iter().filter(|p| p.oncoming_half).count();
        println!(
            "{}: {} overtakes, closest {min:.3} m, {out} into the oncoming half",
            manners.name(),
            all.len()
        );
        // The passes needed the oncoming half, at both distances.
        assert!(
            out * 2 > all.len(),
            "{}: {out} into the oncoming half",
            manners.name()
        );
        for legal in [1.5, 2.0] {
            assert!(
                all.iter().any(|p| (p.legal_m - legal).abs() < 1e-9),
                "{}: no pass at {legal} m",
                manners.name()
            );
        }
    }
}

#[test]
fn some_realism_passes_closer_but_above_the_floor() {
    let (all, hits, _) = run_stadium(
        DriverManners::Realistic,
        TrafficLevel::Busy,
        3,
        2_000,
        4.0 * 3600.0,
    );
    assert!(all.len() >= 2_000);
    assert!(hits.is_empty(), "contacts {:?}", hits.first());
    let close = all
        .iter()
        .filter(|p| p.clearance_m < p.legal_m - 1e-6)
        .count();
    assert!(close > 0, "no close pass at all");
    assert!(
        close * 2 < all.len(),
        "too many close passes: {close} of {}",
        all.len()
    );
    let min = all
        .iter()
        .map(|p| p.clearance_m)
        .fold(f64::INFINITY, f64::min);
    assert!(
        min >= around::PASS_FLOOR_M - 1e-6,
        "under the floor: {min:.2} m"
    );
}

// --- Waiting ------------------------------------------------------------

/// What [`behind_rider`] gives: the passes, the car's (s, lateral, the
/// rider's d) per tick, its id and every rider event.
type Behind = (Vec<PassRecord>, Vec<(f64, f64, f64)>, u32, Vec<RiderEvent>);

/// Runs one car (and optional oncoming cars) behind a rider on `net`:
/// the passes, the car's laterals (s, lateral) and its id.
#[allow(clippy::too_many_arguments)]
fn behind_rider(
    net: &RoadNetwork,
    rules: Box<dyn EdgeRules + Send + Sync>,
    manners: DriverManners,
    level: TrafficLevel,
    seed: u64,
    rider: Rider,
    car_at: f64,
    oncoming: &[f64],
    seconds: f64,
) -> Behind {
    let mut s = sim(net, rules, seed, manners, level, true);
    let len = net.geometry(0).length();
    let car = s
        .spawn_on(net, VehicleClass::Car, &[(0, true)], car_at, 12.0, 0)
        .expect("car");
    for &at in oncoming {
        s.spawn_on(net, VehicleClass::Car, &[(0, false)], len - at, 16.0, 0)
            .expect("oncoming car");
    }
    let time = DemandTime::weekday_at(10.0);
    let mut r = rider;
    let mut all = Vec::new();
    let mut track = Vec::new();
    let mut events = Vec::new();
    for _ in 0..(seconds / DT) as u64 {
        s.step(net, &[everywhere()], &[r.user(net)], &time);
        let ev = s.drain_rider_events();
        all.extend(passes(&ev));
        events.extend(ev);
        if let Some(v) = s.views(net).iter().find(|v| v.id == car) {
            track.push((v.s, v.lateral, r.d));
        }
        r.d += r.speed * DT;
    }
    (all, track, car, events)
}

fn rider_at(d: f64) -> Rider {
    Rider {
        id: 1,
        edge: 0,
        d,
        speed: 6.0,
        lateral: 1.5,
        forward: true,
    }
}

#[test]
fn a_vehicle_waits_behind_a_rider_on_an_overtaking_no_road() {
    struct NoOvertaking;
    impl EdgeRules for NoOvertaking {
        fn edge_rule(&self, net: &RoadNetwork, edge: usize) -> EdgeRule {
            rules::class_rule(net, edge)
        }
        fn passing(&self, _: &RoadNetwork, _: usize) -> PassRule {
            PassRule::new(Country::De, false, [true, true])
        }
    }
    let net = one_road(straight(3000.0), RoadClass::Secondary, Vec::new());
    let (all, track, _, _) = behind_rider(
        &net,
        Box::new(NoOvertaking),
        DriverManners::Polite,
        TrafficLevel::Off,
        1,
        rider_at(300.0),
        200.0,
        &[],
        120.0,
    );
    assert!(all.is_empty(), "passed on an overtaking=no road");
    let (s, _, d) = *track.last().expect("car");
    assert!(d - collide::BODY_LENGTH_M - s > RIDER_GAP_M - 0.5);
    // In Spain a car may cross the solid line to pass a rider.
    struct Spain;
    impl EdgeRules for Spain {
        fn edge_rule(&self, net: &RoadNetwork, edge: usize) -> EdgeRule {
            rules::class_rule(net, edge)
        }
        fn passing(&self, _: &RoadNetwork, _: usize) -> PassRule {
            PassRule::new(Country::Es, false, [true, true])
        }
    }
    let (all, _, _, _) = behind_rider(
        &net,
        Box::new(Spain),
        DriverManners::Polite,
        TrafficLevel::Off,
        1,
        rider_at(300.0),
        200.0,
        &[],
        120.0,
    );
    assert_eq!(all.len(), 1, "Spain: one pass");
    assert!(all[0].clearance_m >= 1.5 - 1e-9);
}

#[test]
fn a_vehicle_waits_for_oncoming_traffic_then_passes() {
    let net = one_road(straight(4000.0), RoadClass::Secondary, Vec::new());
    // A column of oncoming cars, 120 m apart, coming from 600 m on.
    let column: Vec<f64> = (0..8).map(|k| 600.0 + 120.0 * f64::from(k)).collect();
    let (all, track, _, _) = behind_rider(
        &net,
        Box::new(ClassRules),
        DriverManners::Polite,
        TrafficLevel::Off,
        1,
        rider_at(300.0),
        250.0,
        &column,
        150.0,
    );
    assert_eq!(all.len(), 1, "one pass after the column");
    assert!(all[0].clearance_m >= 2.0 - 1e-9, "{:?}", all[0]);
    // The column (at about 22 m/s closing) is past after some 50 s: the
    // car did not leave its lane before.
    let lane = track[0].1;
    let first_out = track
        .iter()
        .position(|t| (t.1 - lane).abs() > 0.2)
        .expect("pulled out") as f64
        * DT;
    assert!(first_out > 30.0, "pulled out at {first_out:.1} s");
}

#[test]
fn a_narrow_road_is_not_passed_by_polite_drivers() {
    // A service road (3.5 m): no room for 1.5 m beside a rider.
    let net = one_road(straight(2000.0), RoadClass::Service, Vec::new());
    for manners in [DriverManners::Polite, DriverManners::Honking] {
        let (all, _, _, events) = behind_rider(
            &net,
            Box::new(ClassRules),
            manners,
            TrafficLevel::Off,
            2,
            Rider {
                speed: 4.0,
                lateral: 0.8,
                ..rider_at(200.0)
            },
            150.0,
            &[],
            200.0,
        );
        assert!(
            all.is_empty(),
            "{}: passed on a narrow road",
            manners.name()
        );
        let honks = events
            .iter()
            .filter(|e| matches!(e, RiderEvent::Honk { .. }))
            .count();
        // Polite drivers never honk; mostly polite ones after a long wait.
        assert_eq!(
            honks > 0,
            manners == DriverManners::Honking,
            "{}",
            manners.name()
        );
        assert!(honks <= 3, "{} honks", honks);
    }
}

#[test]
fn no_pass_into_a_blind_bend() {
    // 400 m straight, a bend of 40 m radius, then 1.5 km straight.
    let net = one_road(bend(400.0, 40.0, 1500.0), RoadClass::Secondary, Vec::new());
    let bend_start = 400.0;
    let bend_end = 400.0 + 40.0 * std::f64::consts::FRAC_PI_2;
    let (all, track, _, _) = behind_rider(
        &net,
        Box::new(ClassRules),
        DriverManners::Polite,
        TrafficLevel::Off,
        1,
        rider_at(260.0),
        200.0,
        &[],
        150.0,
    );
    let lane = track[0].1;
    for &(s, lat, _) in &track {
        if s > bend_start - 250.0 && s < bend_start + 15.0 {
            assert!(
                (lat - lane).abs() < 0.05,
                "pulled out at {s:.0} m before the bend"
            );
        }
    }
    assert_eq!(all.len(), 1, "passed after the bend");
    // A lunatic does it in the bend.
    let (net2, lunatic_seed) = (
        net,
        (1..200)
            .find(|&seed| {
                let mut s = sim(
                    &one_road(bend(400.0, 40.0, 1500.0), RoadClass::Secondary, Vec::new()),
                    Box::new(ClassRules),
                    seed,
                    DriverManners::Polite,
                    TrafficLevel::Lunatic,
                    false,
                );
                let id = s
                    .spawn_on(
                        &one_road(bend(400.0, 40.0, 1500.0), RoadClass::Secondary, Vec::new()),
                        VehicleClass::Car,
                        &[(0, true)],
                        200.0,
                        12.0,
                        0,
                    )
                    .expect("car");
                s.manner(id).is_some_and(|m| m.lunatic)
            })
            .expect("a lunatic seed"),
    );
    let (all, track, _, _) = behind_rider(
        &net2,
        Box::new(ClassRules),
        DriverManners::Polite,
        TrafficLevel::Lunatic,
        lunatic_seed,
        rider_at(260.0),
        200.0,
        &[],
        60.0,
    );
    let lane = track[0].1;
    let out_before_bend = track
        .iter()
        .any(|&(s, lat, _)| s > bend_start - 250.0 && s < bend_end && (lat - lane).abs() > 0.5);
    assert!(out_before_bend, "the lunatic waited");
    assert!(!all.is_empty() && all[0].lunatic);
}

#[test]
fn no_pass_over_a_blind_crest() {
    // A crest 12 m high over 300 m at 600 – 900 m on a 3 km road.
    let profile: Vec<f64> = (0..=600)
        .map(|k| {
            let s = f64::from(k) * 5.0;
            if (600.0..=900.0).contains(&s) {
                let x = (s - 750.0) / 150.0;
                12.0 * (1.0 - x * x)
            } else {
                0.0
            }
        })
        .collect();
    let net = one_road(straight(3000.0), RoadClass::Secondary, profile);
    // The sight distance itself: short before the top, long after.
    let route = [Step {
        edge: 0,
        forward: true,
        speed: 19.0,
        turn_r: 10_000.0,
    }];
    let at = |s: f64| around::sight_distance(&net, &route, RoutePos { step: 0, s }, 500.0, 5.0);
    assert!(at(560.0) < 250.0, "before the crest {:.0}", at(560.0));
    assert!(at(1000.0) >= 500.0 - 1e-9);
    let (all, track, _, _) = behind_rider(
        &net,
        Box::new(ClassRules),
        DriverManners::Polite,
        TrafficLevel::Off,
        1,
        rider_at(520.0),
        480.0,
        &[],
        150.0,
    );
    let lane = track[0].1;
    for &(s, lat, d) in &track {
        if d < 700.0 {
            assert!(
                (lat - lane).abs() < 0.05,
                "pulled out at {s:.0} m below the crest"
            );
        }
    }
    assert_eq!(all.len(), 1);
}

#[test]
fn a_group_is_passed_as_one_block() {
    let net = one_road(straight(5000.0), RoadClass::Secondary, Vec::new());
    let mut s = sim(
        &net,
        Box::new(ClassRules),
        1,
        DriverManners::Polite,
        TrafficLevel::Off,
        true,
    );
    let car = s
        .spawn_on(&net, VehicleClass::Car, &[(0, true)], 200.0, 12.0, 0)
        .expect("car");
    // Five riders 1.5 m apart (wheel to wheel), then one alone 300 m on.
    let mut riders: Vec<Rider> = (0..5)
        .map(|k| Rider {
            id: 10 + k,
            d: 400.0 - f64::from(k) * (collide::BODY_LENGTH_M + 1.5),
            lateral: 1.6 + 0.2 * f64::from(k % 2),
            ..rider_at(0.0)
        })
        .collect();
    riders.push(Rider {
        id: 20,
        d: 700.0,
        ..rider_at(0.0)
    });
    let time = DemandTime::weekday_at(10.0);
    let mut back_in_between = 0;
    let mut was_out = false;
    let mut all = Vec::new();
    for _ in 0..(120.0 / DT) as u64 {
        let users: Vec<RoadUser> = riders.iter().map(|r| r.user(&net)).collect();
        s.step(&net, &[everywhere()], &users, &time);
        all.extend(passes(&s.drain_rider_events()));
        let v = s
            .views(&net)
            .into_iter()
            .find(|v| v.id == car)
            .expect("car");
        let out = v.lateral < 1.0;
        let group_passed = all.iter().filter(|p| p.rider < 20).count();
        if was_out && !out && group_passed > 0 && group_passed < 5 {
            back_in_between += 1;
        }
        was_out = out;
        for r in &mut riders {
            r.d += r.speed * DT;
        }
    }
    let group: Vec<&PassRecord> = all.iter().filter(|p| p.rider < 20).collect();
    assert_eq!(group.len(), 5, "the whole group passed: {all:?}");
    assert_eq!(back_in_between, 0, "cut in inside the group");
    assert!(group.iter().all(|p| p.clearance_m >= 2.0 - 1e-9));
    assert!(all.iter().any(|p| p.rider == 20), "the lone rider too");
}

// --- Junctions ----------------------------------------------------------

/// A crossroads at (0, 0): a primary road west – east, a residential road
/// south – north; arms of 600 m.
fn crossroads() -> RoadNetwork {
    let v = vec![
        [0.0, 0.0],
        [-600.0, 0.0],
        [600.0, 0.0],
        [0.0, 600.0],
        [0.0, -600.0],
    ];
    let mk =
        |i: usize, a: u32, b: u32, class| edge(i, a, b, vec![v[a as usize], v[b as usize]], class);
    RoadNetwork::new(RoadGraph {
        vertices: v.clone(),
        edges: vec![
            mk(0, 1, 0, RoadClass::Primary),
            mk(1, 0, 2, RoadClass::Primary),
            mk(2, 3, 0, RoadClass::Residential),
            mk(3, 0, 4, RoadClass::Residential),
        ],
        boundary_vertices: Default::default(),
    })
}

#[test]
fn vehicles_yield_to_riders_at_junctions() {
    let net = crossroads();
    let run = |with_rider: bool| {
        let mut s = sim(
            &net,
            Box::new(ClassRules),
            1,
            DriverManners::Polite,
            TrafficLevel::Off,
            true,
        );
        // A car on the major road (right of way) and a rider coming from
        // the side road; both reach the crossing in about 10 s.
        let car = s
            .spawn_on(
                &net,
                VehicleClass::Car,
                &[(0, true), (1, true)],
                400.0,
                20.0,
                0,
            )
            .expect("car");
        let mut rider = Rider {
            id: 5,
            edge: 2,
            d: 540.0,
            speed: 6.0,
            lateral: 1.0,
            forward: true,
        };
        let time = DemandTime::weekday_at(10.0);
        let mut min_v = f64::INFINITY;
        let mut conflict = false;
        let mut rider_when_car_arrives = None;
        for _ in 0..(30.0 / DT) as u64 {
            let users = if with_rider && rider.d < 600.0 {
                vec![rider.user(&net)]
            } else if with_rider {
                // Past the vertex: on the far arm.
                vec![
                    Rider {
                        edge: 3,
                        d: rider.d - 600.0,
                        ..rider
                    }
                    .user(&net),
                ]
            } else {
                Vec::new()
            };
            s.step(&net, &[everywhere()], &users, &time);
            let v = s
                .views(&net)
                .into_iter()
                .find(|v| v.id == car)
                .expect("car");
            // The car's front within 5 m of the crossing while the rider is.
            let car_at = if v.edge == 0 { v.s - 600.0 } else { v.s };
            let rider_at = rider.d - 600.0;
            if with_rider && car_at.abs() < 4.0 && rider_at.abs() < 4.0 {
                conflict = true;
            }
            if v.xz[0] < 0.5 && v.xz[0] > -120.0 {
                min_v = min_v.min(v.speed);
            }
            if car_at >= -1.0 && rider_when_car_arrives.is_none() {
                rider_when_car_arrives = Some(rider_at);
            }
            rider.d += rider.speed * DT;
        }
        (min_v, conflict, rider_when_car_arrives)
    };
    let (alone, _, _) = run(false);
    assert!(alone > 15.0, "slowed without a rider: {alone:.1} m/s");
    let (yielding, conflict, rider_at) = run(true);
    assert!(yielding < alone - 5.0, "did not slow: {yielding:.1} m/s");
    assert!(!conflict, "car and rider in the crossing together");
    // The rider (due at the same moment) crossed first.
    let rider_at = rider_at.expect("the car crossed");
    assert!(
        rider_at > 2.0,
        "the rider was {rider_at:.1} m from the crossing"
    );
}

// --- Lunatics, contacts --------------------------------------------------

#[test]
fn lunatics_pass_dangerously_and_hit_riders_with_collisions_on() {
    let net = one_road(straight(3000.0), RoadClass::Secondary, Vec::new());
    let len = net.geometry(0).length();
    let lunatic_seed = (1..300)
        .find(|&seed| {
            let mut s = sim(
                &net,
                Box::new(ClassRules),
                seed,
                DriverManners::Polite,
                TrafficLevel::Lunatic,
                true,
            );
            let id = s
                .spawn_on(&net, VehicleClass::Car, &[(0, true)], 400.0, 14.0, 0)
                .expect("car");
            s.manner(id).is_some_and(|m| m.lunatic)
        })
        .expect("a lunatic seed");
    // An oncoming car at a distance that makes the lunatic cut back in.
    let run = |onc: f64, collisions: bool| {
        let mut s = sim(
            &net,
            Box::new(ClassRules),
            lunatic_seed,
            DriverManners::Polite,
            TrafficLevel::Lunatic,
            collisions,
        );
        s.spawn_on(&net, VehicleClass::Car, &[(0, true)], 400.0, 14.0, 0)
            .expect("car");
        s.spawn_on(&net, VehicleClass::Car, &[(0, false)], len - onc, 20.0, 0)
            .expect("oncoming");
        let mut r = rider_at(430.0);
        let time = DemandTime::weekday_at(10.0);
        let mut ev = Vec::new();
        for _ in 0..(40.0 / DT) as u64 {
            s.step(&net, &[everywhere()], &[r.user(&net)], &time);
            ev.extend(s.drain_rider_events());
            r.d += r.speed * DT;
        }
        ev
    };
    let hit = (0..60)
        .map(|k| 500.0 + 10.0 * f64::from(k))
        .find(|&onc| !contacts(&run(onc, true)).is_empty())
        .expect("a lunatic who cuts in onto the rider");
    let on = run(hit, true);
    let c = contacts(&on)[0];
    // Cut in beside the rider or onto its back wheel.
    assert!(c.relative[0] > 0.5, "the car was faster: {c:?}");
    // The rider's side of the contact (12B with vehicles): a crash with
    // collisions on, nothing with them off.
    let body = collide::Body::on_path(ocw_core::RiderId(1), 0.0, 1.5, 0.0, 6.0, 80.0);
    let full = collide::vehicle_hit(
        &body,
        c.closing_mps,
        c.relative,
        c.push_side,
        collide::Mode::Full,
    );
    assert!(
        full.crash
            .is_some_and(|h| h.kind == collide::ContactKind::Vehicle)
    );
    let off = collide::vehicle_hit(
        &body,
        c.closing_mps,
        c.relative,
        c.push_side,
        collide::Mode::Off,
    );
    assert!(!off.touched());
    // With collisions off the same situation never touches.
    assert!(contacts(&run(hit, false)).is_empty());
    // Lunatics pass too close on an open road.
    let (all, _, _) = run_stadium(DriverManners::Polite, TrafficLevel::Lunatic, 5, 400, 1800.0);
    let mad: Vec<&PassRecord> = all.iter().filter(|p| p.lunatic).collect();
    assert!(!mad.is_empty());
    let close = mad.iter().filter(|p| p.clearance_m < p.legal_m).count();
    assert!(
        close * 2 > mad.len(),
        "{close} of {} lunatic passes too close",
        mad.len()
    );
    // … while the sane drivers of the same roads keep the law.
    let bad: Vec<&PassRecord> = all
        .iter()
        .filter(|p| !p.lunatic && p.clearance_m < p.legal_m - 1e-6)
        .collect();
    assert!(
        bad.is_empty(),
        "{} sane passes too close: {:?}",
        bad.len(),
        bad.first()
    );
}

#[test]
fn a_car_that_cannot_stop_hits_the_rider_only_with_collisions_on() {
    let net = one_road(straight(2000.0), RoadClass::Secondary, Vec::new());
    for collisions in [true, false] {
        let mut s = sim(
            &net,
            Box::new(ClassRules),
            1,
            DriverManners::Polite,
            TrafficLevel::Off,
            collisions,
        );
        let car = s
            .spawn_on(&net, VehicleClass::Car, &[(0, true)], 300.0, 20.0, 0)
            .expect("car");
        let time = DemandTime::weekday_at(10.0);
        // No rider, then one swerving into the lane 12 m ahead, slow.
        for _ in 0..10 {
            s.step(&net, &[everywhere()], &[], &time);
        }
        let front = s.views(&net)[0].s;
        let mut r = Rider {
            d: front + 12.0 + collide::BODY_LENGTH_M,
            speed: 3.0,
            ..rider_at(0.0)
        };
        let mut ev = Vec::new();
        let mut ahead_after = false;
        for _ in 0..(4.0 / DT) as u64 {
            s.step(&net, &[everywhere()], &[r.user(&net)], &time);
            ev.extend(s.drain_rider_events());
            r.d += r.speed * DT;
            let v = s
                .views(&net)
                .into_iter()
                .find(|v| v.id == car)
                .expect("car");
            ahead_after |= v.s - v_len(VehicleClass::Car) > r.d;
        }
        let hits = contacts(&ev);
        if collisions {
            assert_eq!(hits.len(), 1, "{hits:?}");
            assert_eq!(hits[0].side, ContactSide::Rear);
            assert!(hits[0].closing_mps > 1.0);
            assert!(!ahead_after, "drove through the rider");
        } else {
            assert!(hits.is_empty());
            assert!(ahead_after, "passed through without contact");
        }
    }
}

fn v_len(c: VehicleClass) -> f64 {
    c.traits().length_m
}

#[test]
fn a_rider_steering_into_a_car_touches_it_only_with_collisions_on() {
    // In town (1.5 m), a fast rider: the car is beside it for a while.
    let net = one_road(straight(3000.0), RoadClass::Unclassified, Vec::new());
    for collisions in [true, false] {
        let mut s = sim(
            &net,
            Box::new(ClassRules),
            1,
            DriverManners::Polite,
            TrafficLevel::Off,
            collisions,
        );
        let car = s
            .spawn_on(&net, VehicleClass::Car, &[(0, true)], 250.0, 12.0, 0)
            .expect("car");
        let mut r = Rider {
            speed: 8.0,
            lateral: 2.0,
            ..rider_at(300.0)
        };
        let mut steering = false;
        let time = DemandTime::weekday_at(10.0);
        let mut ev = Vec::new();
        for _ in 0..(60.0 / DT) as u64 {
            s.step(&net, &[everywhere()], &[r.user(&net)], &time);
            ev.extend(s.drain_rider_events());
            let v = s
                .views(&net)
                .into_iter()
                .find(|v| v.id == car)
                .expect("car");
            // The car comes up beside the rider: the rider swerves left.
            steering |= v.lateral < 0.0 && v.s > r.d - 0.9;
            if steering && r.lateral > -2.0 {
                r.lateral -= 3.0 * DT;
            }
            r.d += r.speed * DT;
        }
        let hits = contacts(&ev);
        assert_eq!(!hits.is_empty(), collisions, "{hits:?}");
        if collisions {
            assert_eq!(hits[0].side, ContactSide::Side);
            let body = collide::Body::on_path(ocw_core::RiderId(1), 0.0, 1.0, 0.0, 6.0, 80.0);
            let resp = collide::vehicle_hit(
                &body,
                hits[0].closing_mps,
                hits[0].relative,
                hits[0].push_side,
                collide::Mode::Full,
            );
            assert!(resp.crash.is_some(), "swept down by a car passing at speed");
        }
    }
}

// --- Gusts, proximity cue -------------------------------------------------

#[test]
fn the_gust_grows_with_speed_and_falls_with_distance() {
    let lorry = VehicleClass::Lorry;
    let peak = |v: f64, c: f64| around::gust(lorry, v, c, 0.0, 12.0, false).0;
    // ∝ speed², less farther away, a lorry more than a car.
    assert!((peak(20.0, 1.0) / peak(10.0, 1.0) - 4.0).abs() < 1e-9);
    assert!(peak(20.0, 1.0) > peak(20.0, 2.0) && peak(20.0, 2.0) > peak(20.0, 3.0));
    assert!(peak(20.0, 1.0) > around::gust(VehicleClass::Car, 20.0, 1.0, 0.0, 4.5, false).0 * 2.0);
    // A lorry 14 m/s faster at 1.5 m: a push of some 10 – 40 N.
    let f = peak(14.0, 1.5);
    assert!((3.0..40.0).contains(&f), "{f:.1} N");
    // The bow pushes away, the wake behind the rear pulls back.
    assert!(around::gust(lorry, 14.0, 1.5, -12.0, 12.0, false).0 < 0.0);
    // The drag change sums to about zero over a pass (no drafting).
    let (mut sum, mut abs) = (0.0, 0.0);
    let mut x = 60.0;
    while x > -80.0 {
        let d = around::gust(lorry, 14.0, 1.5, x, 12.0, false).1;
        sum += d * 0.05;
        abs += d.abs() * 0.05;
        x -= 0.05;
    }
    assert!(sum.abs() < 0.1 * abs, "net drag {sum:.2} of {abs:.2}");
    // In the simulation: a lorry passing a rider pushes it.
    let net = one_road(straight(3000.0), RoadClass::Secondary, Vec::new());
    let mut s = sim(
        &net,
        Box::new(ClassRules),
        1,
        DriverManners::Polite,
        TrafficLevel::Off,
        true,
    );
    s.spawn_on(&net, VehicleClass::Lorry, &[(0, true)], 200.0, 15.0, 0)
        .expect("lorry");
    let mut r = Rider {
        lateral: 2.2,
        ..rider_at(300.0)
    };
    let time = DemandTime::weekday_at(10.0);
    let (mut push, mut pull) = (0.0_f64, 0.0_f64);
    for _ in 0..(60.0 / DT) as u64 {
        s.step(&net, &[everywhere()], &[r.user(&net)], &time);
        for g in s.gusts() {
            assert_eq!(g.rider, 1);
            push = push.max(g.lateral_n);
            pull = pull.min(g.lateral_n);
        }
        r.d += r.speed * DT;
    }
    // The lorry passes on the rider's left: pushed right, then pulled left.
    assert!(
        push > 1.0 && pull < -0.5,
        "push {push:.1} N, pull {pull:.1} N"
    );
}

#[test]
fn a_vehicle_from_behind_gives_the_proximity_cue() {
    let net = one_road(straight(3000.0), RoadClass::Secondary, Vec::new());
    let (_, _, car, events) = behind_rider(
        &net,
        Box::new(ClassRules),
        DriverManners::Polite,
        TrafficLevel::Off,
        1,
        rider_at(400.0),
        100.0,
        &[],
        30.0,
    );
    let cues: Vec<&RiderEvent> = events
        .iter()
        .filter(|e| matches!(e, RiderEvent::Approaching { .. }))
        .collect();
    assert_eq!(cues.len(), 1, "one cue per approach");
    if let RiderEvent::Approaching {
        vehicle, seconds, ..
    } = cues[0]
    {
        assert_eq!(*vehicle, car);
        assert!(*seconds <= around::PROXIMITY_S);
    }
}

#[test]
fn autodrive_sees_vehicles_as_obstacles() {
    let net = one_road(straight(2000.0), RoadClass::Secondary, Vec::new());
    let mut s = sim(
        &net,
        Box::new(ClassRules),
        1,
        DriverManners::Polite,
        TrafficLevel::Off,
        true,
    );
    s.spawn_on(&net, VehicleClass::Car, &[(0, true)], 330.0, 0.0, 0)
        .expect("standing car");
    s.spawn_on(
        &net,
        VehicleClass::Car,
        &[(0, false)],
        2000.0 - 360.0,
        10.0,
        0,
    )
    .expect("oncoming car");
    let me = rider_at(300.0).user(&net);
    let others = s.avoid_others(&net, &me, 100.0);
    assert_eq!(others.len(), 2);
    let ahead = others.iter().find(|o| o.speed >= 0.0).expect("ahead");
    assert!(ahead.ahead_m > 20.0 && ahead.ahead_m < 30.0, "{ahead:?}");
    assert!(ahead.lateral_m > 0.5, "in the rider's half");
    let onc = others.iter().find(|o| o.speed < 0.0).expect("oncoming");
    assert!(onc.lateral_m < 0.0, "the other half");
    // Its world body sits where its view is.
    let b = s.bodies(&net);
    assert!((b[0].half_width * 2.0 - around::body_size(VehicleClass::Car).1).abs() < 1e-9);
}

// --- Rules, determinism, followers ---------------------------------------

#[test]
fn legal_distances_by_country() {
    assert_eq!(around::legal_pass_m(Country::De, true), 1.5);
    assert_eq!(around::legal_pass_m(Country::De, false), 2.0);
    assert_eq!(around::legal_pass_m(Country::At, false), 2.0);
    for c in [Country::Es, Country::Ch, Country::Li, Country::Ad] {
        assert_eq!(around::legal_pass_m(c, false), 1.5, "{c:?}");
        assert_eq!(around::legal_pass_m(c, true), 1.5, "{c:?}");
    }
    assert!(PassRule::new(Country::Es, false, [true, true]).may_pass(true));
    assert!(!PassRule::new(Country::De, false, [true, false]).may_pass(true));
    assert!(PassRule::new(Country::De, false, [true, false]).may_pass(false));
    // Without data: in town by class or limit.
    assert!(PassRule::implied(RoadClass::Residential, 30.0 / 3.6).urban);
    assert!(!PassRule::implied(RoadClass::Secondary, 70.0 / 3.6).urban);
    // The vehicles' bodies from their model files.
    let (_, car) = around::body_size(VehicleClass::Car);
    let (_, lorry) = around::body_size(VehicleClass::Lorry);
    assert!((1.6..2.2).contains(&car), "car {car}");
    assert!((2.2..2.7).contains(&lorry), "lorry {lorry}");
}

#[test]
fn world_rules_give_the_country_and_town() {
    use ocw_world::traffic as wt;
    let net = one_road(straight(1000.0), RoadClass::Secondary, Vec::new());
    let id = net.graph().edges[0].id;
    let mut edges = std::collections::BTreeMap::new();
    let mut r = wt::EdgeRules::implied(RoadClass::Secondary, Country::At, true);
    r.no_overtaking = [true, false];
    edges.insert(id, r);
    let data = wt::TrafficData {
        country: Country::At,
        edges,
        ..Default::default()
    };
    let rules = WorldRules::from_rules(
        &wt::RoadRules {
            data: Some(&data),
            country: Country::At,
        },
        &net,
    );
    let p = rules.passing(&net, 0);
    assert_eq!((p.country, p.urban, p.distance_m), (Country::At, true, 1.5));
    assert!(!p.may_pass(true) && p.may_pass(false));
    let e = rules.edge_rule(&net, 0);
    assert!(e.access.allows(VehicleClass::Car) && e.access.allows(VehicleClass::Lorry));
    assert_eq!(e.lanes, [1, 1]);
}

#[test]
fn deterministic_with_riders() {
    let digest = |manners: DriverManners| {
        let (net, cum) = stadium();
        let total = *cum.last().expect("loop");
        let mut s = sim(
            &net,
            Box::new(ClassRules),
            9,
            manners,
            TrafficLevel::Busy,
            true,
        );
        let bubble = Bubble {
            center: [2000.0, 150.0],
            radius: 2400.0,
        };
        let mut riders = stadium_riders(total, 9);
        let time = DemandTime::weekday_at(10.0);
        let mut events = Vec::new();
        for _ in 0..6000 {
            let users: Vec<RoadUser> = riders.iter().map(|r| r.user(&cum)).collect();
            s.step(&net, &[bubble], &users, &time);
            events.extend(s.drain_rider_events());
            for r in &mut riders {
                r.advance(total, DT);
            }
        }
        (s.digest(), format!("{events:?}"))
    };
    let a = digest(DriverManners::Polite);
    assert_eq!(a, digest(DriverManners::Polite));
    assert_ne!(a.0, digest(DriverManners::Realistic).0);
}

#[test]
fn clients_follow_passing_vehicles_sideways() {
    let (net, cum) = stadium();
    let total = *cum.last().expect("loop");
    let mut s = sim(
        &net,
        Box::new(ClassRules),
        4,
        DriverManners::Polite,
        TrafficLevel::Busy,
        true,
    );
    let bubble = Bubble {
        center: [2000.0, 150.0],
        radius: 2400.0,
    };
    let mut riders = stadium_riders(total, 4);
    let time = DemandTime::weekday_at(10.0);
    let mut followers: Vec<PlanFollower> = Vec::new();
    let mut worst = 0.0_f64;
    let mut shifted = 0u64;
    for _ in 0..(300.0 / DT) as u64 {
        let users: Vec<RoadUser> = riders.iter().map(|r| r.user(&cum)).collect();
        s.step(&net, &[bubble], &users, &time);
        for f in &mut followers {
            f.step(&net, s.curves(), DT);
        }
        plan::apply_events(&mut followers, &s.drain_events());
        for v in s.views(&net) {
            let f = followers.iter().find(|f| f.id == v.id).expect("a follower");
            let r = s.rule(v.edge);
            let lat = lateral_offset(r, net.edge(v.edge).width_m, f.lane) - f.shift;
            worst = worst.max((lat - v.lateral).abs());
            if f.shift.abs() > 0.5 {
                shifted += 1;
            }
        }
        for r in &mut riders {
            r.advance(total, DT);
        }
    }
    assert!(shifted > 0, "nobody pulled out");
    assert!(
        worst <= plan::SHIFT_TOLERANCE_M + 1e-3,
        "off sideways by {worst:.3} m"
    );
}
