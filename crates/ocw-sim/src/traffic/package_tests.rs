//! Tests of the package rules adapter (spec 18E T5).

use std::sync::Arc;

use ocw_world::graph::{Edge, EdgeId, Oneway, RoadGraph};
use ocw_world::traffic::{Applies, EdgeRules as WorldRules, NodeControl, TrafficData};

use super::*;
use crate::traffic::{Bubble, DemandTime, TrafficConfig, TrafficEvent, TrafficLevel, TrafficSim};

/// A crossroads at (0, 0) with arms of 400 m to the north (edge 0,
/// vertex 1), east (1, 2), south (2, 3) and west (3, 4), each drawn from
/// the centre outwards, and a cycleway on to the east (4, 2 – 5).
fn cross(classes: [RoadClass; 4]) -> RoadGraph {
    let v = vec![
        [0.0, 0.0],
        [0.0, -400.0],
        [400.0, 0.0],
        [0.0, 400.0],
        [-400.0, 0.0],
        [800.0, 0.0],
    ];
    let mut edges: Vec<Edge> = (0..4)
        .map(|i| Edge {
            id: EdgeId(100 + i as u64),
            from: VertexId(0),
            to: VertexId(i as u32 + 1),
            points: vec![v[0], v[i + 1]],
            class: classes[i],
            name: None,
            width_m: classes[i].default_width_m(),
            bridge: false,
            tunnel: false,
            dead_end_m: None,
            boundary: false,
            oneway: Oneway::No,
            osm_way: i as i64,
            profile: Vec::new(),
        })
        .collect();
    edges.push(Edge {
        id: EdgeId(104),
        from: VertexId(2),
        to: VertexId(5),
        points: vec![v[2], v[5]],
        class: RoadClass::Cycleway,
        width_m: 3.0,
        osm_way: 4,
        ..edges[1].clone()
    });
    RoadGraph {
        vertices: v,
        edges,
        boundary_vertices: Default::default(),
    }
}

fn control(edge: u64, along_m: f32, kind: ControlKind, at_junction: bool) -> NodeControl {
    NodeControl {
        edge: EdgeId(edge),
        along_m,
        kind,
        applies: Applies::Backward,
        at_junction,
        osm_node: edge as i64,
    }
}

/// The crossroads' `TRFC`: the east arm one-way towards the centre (drawn
/// outwards: backward only), the north arm without lorries (`hgv=no`) and
/// a 3.5 t limit, the west arm at 30 km/h with two lanes each way, the
/// south arm a give-way sign 20 m before the junction; lights at a
/// crossing on the west arm.
fn data() -> TrafficData {
    let base = |c| WorldRules::implied(c, Country::Li, false);
    let mut edges = BTreeMap::new();
    let mut north = base(RoadClass::Secondary);
    north.access = north.access.with(Vehicle::Hgv, false);
    north.maxweight_t = Some(3.5);
    edges.insert(EdgeId(100), north);
    let mut east = base(RoadClass::Secondary);
    east.oneway = Oneway::Backward;
    east.lanes = [0, 1];
    edges.insert(EdgeId(101), east);
    edges.insert(EdgeId(102), base(RoadClass::Tertiary));
    let mut west = base(RoadClass::Secondary);
    west.maxspeed = [wt::SpeedLimit::Kmh(30); 2];
    west.lanes = [2, 2];
    edges.insert(EdgeId(103), west);
    edges.insert(EdgeId(104), base(RoadClass::Cycleway));
    let mut t = TrafficData {
        country: Country::Li,
        edges,
        controls: vec![
            control(102, 20.0, ControlKind::GiveWay, false),
            control(103, 200.0, ControlKind::CrossingSignals, false),
        ],
        ..Default::default()
    };
    t.sort();
    t
}

fn net(data: Option<TrafficData>) -> RoadNetwork {
    let mut g = cross([RoadClass::Secondary; 4]);
    g.edges[2].class = RoadClass::Tertiary;
    RoadNetwork::new(g).with_traffic(data.map(Arc::new), Country::Li)
}

#[test]
fn the_packages_rules_map_onto_the_core() {
    let n = net(Some(data()));
    let r = PackageRules::new(&n);
    assert!(r.from_data());
    assert_eq!(r.country(), Country::Li);
    assert_eq!(region_of(r.country()), Region::Li);
    let rule = |e| r.edge_rule(&n, e);
    // North: no lorries (hgv=no, and the weight limit would keep them off
    // anyway), cars, vans and buses yes.
    assert!(!rule(0).access.allows(VehicleClass::Lorry));
    assert!(rule(0).access.allows(VehicleClass::Car));
    assert!(rule(0).access.allows(VehicleClass::Bus));
    // East: one-way towards the centre (backward), no lane outwards.
    assert_eq!(rule(1).oneway, Oneway::Backward);
    assert_eq!(rule(1).lanes, [0, 1]);
    assert!(!rule(1).allows(VehicleClass::Car, true));
    assert!(rule(1).allows(VehicleClass::Car, false));
    // West: its limit and lanes.
    assert!((rule(3).speed - 30.0 / 3.6).abs() < 1e-6);
    assert_eq!(rule(3).lanes, [2, 2]);
    // South: Liechtenstein's implied 80, the class's daily traffic.
    assert!(
        (rule(2).speed - 80.0 / 3.6).abs() < 1e-5,
        "{}",
        rule(2).speed * 3.6
    );
    assert_eq!(rule(2).aadt, class_defaults(RoadClass::Tertiary).aadt);
    // The cycleway carries nothing.
    assert!(rule(4).access.is_none() && rule(4).lanes == [0, 0]);
    // The give-way sign 20 m before the junction is the south arm's; the
    // other arms have right of way; the crossing lights along the west
    // arm are no junction control.
    let c = |e: usize, fwd: bool| r.node_control(&n, VertexId(0), e, fwd);
    assert_eq!(c(2, false), Some(Control::GiveWay));
    assert_eq!(c(0, false), Some(Control::Priority));
    assert_eq!(c(3, false), Some(Control::Priority));
    // Not at the far ends.
    assert_eq!(r.node_control(&n, VertexId(4), 3, true), None);
    assert_eq!(r.signalled(), 0);
    assert_eq!(r.controls(), 1);
}

#[test]
fn signals_on_one_arm_signal_every_arm() {
    let mut d = data();
    d.controls = vec![control(100, 0.0, ControlKind::Signals, true)];
    let n = net(Some(d));
    let r = PackageRules::new(&n);
    for e in 0..4 {
        assert_eq!(
            r.node_control(&n, VertexId(0), e, false),
            Some(Control::Signals),
            "arm {e}"
        );
    }
    assert_eq!(r.signalled(), 1);
    // A sign 100 m before the junction is not at it.
    let mut d = data();
    d.controls = vec![control(102, 100.0, ControlKind::Stop, false)];
    let n = net(Some(d));
    let r = PackageRules::new(&n);
    assert_eq!(r.node_control(&n, VertexId(0), 2, false), None);
}

/// Old packages: the rules T2 implies by class and the country.
#[test]
fn without_trfc_the_implied_rules_apply() {
    let n = net(None);
    let r = PackageRules::new(&n);
    assert!(!r.from_data());
    for (i, e) in n.graph().edges.iter().enumerate() {
        let want = edge_rule(&WorldRules::implied(e.class, Country::Li, false), e.class);
        assert_eq!(r.edge_rule(&n, i), want);
    }
    assert!(r.edge_rule(&n, 4).access.is_none(), "cycleway");
    // No sign anywhere: the core derives the junctions.
    assert_eq!(r.node_control(&n, VertexId(0), 0, false), None);
    // Tracks: tractors only, unless the package opens them.
    let track = edge_rule(
        &WorldRules::implied(RoadClass::Track, Country::De, false),
        RoadClass::Track,
    );
    assert_eq!(track.access, Access::only(&[VehicleClass::Tractor]));
}

/// The core keeps the package's rules: in a busy simulated hour no car
/// drives out along the one-way arm, none on the cycleway and no lorry on
/// the north arm.
#[test]
fn the_core_drives_by_the_packages_rules() {
    let n = net(Some(data()));
    let mut sim = TrafficSim::new(
        &n,
        rules_for(&n),
        TrafficConfig {
            seed: 3,
            step_ms: 100,
            level: TrafficLevel::Busy,
            ..Default::default()
        },
    );
    let bubble = Bubble {
        center: [0.0, 0.0],
        radius: 1500.0,
    };
    let mut spawned = 0;
    for t in 0..36_000u32 {
        let time = DemandTime::weekday_at(17.0).plus_seconds(t / 10);
        sim.step(&n, &[bubble], &[], &time);
        for (e, fwd, class) in sim.placements() {
            assert!(!(e == 1 && fwd), "a vehicle against the one-way arm");
            assert!(e != 4, "a vehicle on the cycleway");
            assert!(
                !(e == 0 && class == VehicleClass::Lorry),
                "a lorry on the hgv=no arm"
            );
        }
        spawned += sim
            .drain_events()
            .iter()
            .filter(|e| matches!(e, TrafficEvent::Spawn(_)))
            .count();
    }
    assert!(spawned > 50, "{spawned} vehicles in an hour");
}
