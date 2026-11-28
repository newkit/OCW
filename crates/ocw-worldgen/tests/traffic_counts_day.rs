//! Spec 18E T11: the flow factors written from a count reproduce the
//! counted daily traffic in a simulated day. The fixture road is the
//! TF-235 at the Cabildo de Tenerife's station CRUZ CHICA (IMD 2025:
//! 9,788 vehicles a day; our download, CC BY – `src/traffic_counts/
//! fixtures/tenerife.counts.csv`): a straight two-way secondary road of
//! 12 km in three edges, the station on the middle one, a rider's bubble
//! on the middle edge only, so every vehicle the sim sends along the road
//! passes the station.

use std::collections::BTreeMap;

use ocw_sim::traffic::demand::{self, DemandTime, Region, annual_day_factor};
use ocw_sim::traffic::world::{WorldRules, unit_aadt};
use ocw_sim::traffic::{Bubble, TrafficConfig, TrafficLevel, TrafficSim};
use ocw_world::RoadNetwork;
use ocw_world::geo::LocalProjection;
use ocw_world::graph::{Edge, EdgeId, Oneway, RoadClass, RoadGraph, VertexId};
use ocw_world::traffic::{Country, EdgeRules, RoadRules, TrafficData};
use ocw_worldgen::traffic_counts::{self as tc, Params, WayNames};

fn fixture_road() -> (RoadGraph, TrafficData, WayNames) {
    let xs = [-6000.0, -1500.0, 1500.0, 6000.0];
    let vertices: Vec<[f64; 2]> = xs.iter().map(|&x| [x, 0.0]).collect();
    let mut names = WayNames::default();
    let mut edges = Vec::new();
    let mut rules = BTreeMap::new();
    for i in 0..3 {
        edges.push(Edge {
            id: EdgeId(10 + i as u64),
            from: VertexId(i as u32),
            to: VertexId(i as u32 + 1),
            points: vec![vertices[i], vertices[i + 1]],
            class: RoadClass::Secondary,
            name: Some("Carretera de La Esperanza".into()),
            width_m: 6.5,
            bridge: false,
            tunnel: false,
            dead_end_m: None,
            boundary: false,
            oneway: Oneway::No,
            osm_way: 100 + i as i64,
            profile: Vec::new(),
        });
        rules.insert(
            EdgeId(10 + i as u64),
            EdgeRules::implied(RoadClass::Secondary, Country::Es, false),
        );
        names.insert(100 + i as i64, Some("TF-235"), None);
    }
    let graph = RoadGraph {
        vertices,
        edges,
        boundary_vertices: Default::default(),
    };
    let data = TrafficData {
        country: Country::Es,
        edges: rules,
        ..Default::default()
    };
    (graph, data, names)
}

#[test]
fn factors_reproduce_the_counted_day_within_ten_per_cent() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/traffic_counts/fixtures/tenerife.counts.csv");
    let (_, stations) = tc::read_counts(&path).unwrap();
    let cruz: Vec<_> = stations.into_iter().filter(|s| s.id == "511").collect();
    let counted = cruz[0].aadt;
    let (graph, mut data, names) = fixture_road();
    let proj = LocalProjection::new(cruz[0].lat, cruz[0].lon + 0.00002);
    let m = tc::match_stations(
        &graph,
        &data,
        &names,
        &proj,
        &cruz,
        &Params::default(),
        |_| true,
    );
    assert_eq!(m.anchors.len(), 1);
    assert_eq!(m.anchors[0].edge, 1);
    let year = annual_day_factor(Region::Es);
    assert!((0.9..1.0).contains(&year), "{year}");
    let flows = tc::flow_factors(&graph, &data, &names, &m, |e| {
        unit_aadt(&data.edges[&graph.edges[e].id], graph.edges[e].class) * year
    });
    tc::apply_flows(&graph, &mut data, &flows);
    let flow = data.edges[&EdgeId(11)].flow;
    assert!(
        (f64::from(flow) - counted / (4500.0 * year)).abs() < 0.006,
        "{flow}"
    );
    // Through the package's encoding (TRFC), as the sim reads it.
    let mut b = Vec::new();
    ocw_world::traffic::write_traffic(&data, &mut b);
    let data = ocw_world::traffic::read_traffic(&b).unwrap();
    let net = RoadNetwork::new(graph);
    let rules = WorldRules::from_rules(
        &RoadRules {
            data: Some(&data),
            country: Country::Es,
        },
        &net,
    );
    let mut sim = TrafficSim::new(
        &net,
        Box::new(rules),
        TrafficConfig {
            seed: 11,
            step_ms: 250,
            level: TrafficLevel::Normal,
            ..TrafficConfig::default()
        },
    );
    let bubble = Bubble {
        center: [0.0, 0.0],
        radius: 1000.0,
    };
    // A Tuesday in April: the model's average weekday (day factor 1).
    let start = DemandTime {
        weekday: 1,
        seconds: 0,
        month: 4,
        holiday: false,
    };
    assert_eq!(demand::day_factor(&start), 1.0);
    sim.step(&net, &[bubble], &[], &start);
    let filled = sim.stats().spawned;
    for t in 1..(86_400 * 4u32) {
        sim.step(&net, &[bubble], &[], &start.plus_seconds(t / 4));
        sim.drain_events();
    }
    let day = (sim.stats().spawned - filled) as f64;
    // That weekday is the year's average day over the annual day factor.
    let as_counted = day * year;
    let err = as_counted / counted - 1.0;
    println!(
        "counted {counted:.0} a day, flow {flow:.2}: simulated {day:.0} on a Tuesday in April \
         = {as_counted:.0} on the year's average day ({:+.1} %)",
        err * 100.0
    );
    assert!(err.abs() < 0.10, "{err}");
    // Without the counts the road gets the class default.
    assert!((counted / (4500.0 * year) - 1.0).abs() > 0.5);
}
