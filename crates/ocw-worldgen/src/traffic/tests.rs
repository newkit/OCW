//! The traffic sections from an OSM fixture (spec 18E T2): edge rules on
//! the graph (oneway against the drawing direction, `hgv=no`,
//! `parking:both`, `maxspeed=walk`, per-direction limits on an edge the
//! graph turned round, the roundabout), the implied limits by the country
//! outline and the time zone, in-town roads by a village, signals at a
//! junction and at a crossing, a give-way sign for the minor road, a bus
//! line with its platform, a car park, an aerodrome with its runway, a
//! hospital helipad and the nearby airports; the package round trip and
//! old packages without the sections.

use std::collections::BTreeMap;

use ocw_world::graph::{Oneway, RoadClass};
use ocw_world::traffic::{
    Access, Applies, ControlKind, Country, ParkingPlace, RoadRules, Side, SpeedLimit, SpeedSource,
    Vehicle,
};

use super::*;
use crate::build::{Region, build_road_graph};
use crate::osm::parse_osm_xml_raw;
use crate::places::{PlaceData, add_places};

/// Made-up roads around (47.10, 9.50) inside a made-up country outline
/// tagged LI: a primary road west – east (1 – 5) into a roundabout
/// (5, 20, 21, 22) with a road on to the east (21 – 24); a residential
/// one-way street drawn against its traffic (3 – 10, `oneway=-1`, no
/// lorries, parking on both sides); a tertiary road (30 – 2) with limits
/// per direction and an unclassified one (30 – 1) closing a loop by the
/// village Bea Dorf, and a one-way service road at walking pace drawn
/// from its dead end towards the junction (31 – 30), which the graph
/// turns round.
const FIXTURE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<osm version="0.6" generator="hand">
  <node id="1" lat="47.1000" lon="9.4900"/>
  <node id="2" lat="47.1000" lon="9.4950"><tag k="highway" v="give_way"/></node>
  <node id="3" lat="47.1000" lon="9.5000"><tag k="highway" v="traffic_signals"/></node>
  <node id="4" lat="47.1000" lon="9.5025">
    <tag k="highway" v="crossing"/><tag k="crossing" v="traffic_signals"/>
  </node>
  <node id="5" lat="47.1000" lon="9.5050"/>
  <node id="10" lat="47.1030" lon="9.5000"/>
  <node id="20" lat="47.1003" lon="9.5054"/>
  <node id="21" lat="47.1000" lon="9.5058"/>
  <node id="22" lat="47.0997" lon="9.5054"/>
  <node id="24" lat="47.1000" lon="9.5150"/>
  <node id="30" lat="47.0970" lon="9.4950"/>
  <node id="31" lat="47.0940" lon="9.4950"/>
  <way id="100">
    <nd ref="1"/><nd ref="2"/><nd ref="3"/><nd ref="4"/><nd ref="5"/>
    <tag k="highway" v="primary"/><tag k="name" v="Adastrasse"/>
  </way>
  <way id="101">
    <nd ref="3"/><nd ref="10"/>
    <tag k="highway" v="residential"/><tag k="oneway" v="-1"/>
    <tag k="hgv" v="no"/><tag k="parking:both" v="lane"/>
  </way>
  <way id="102">
    <nd ref="5"/><nd ref="20"/><nd ref="21"/><nd ref="22"/><nd ref="5"/>
    <tag k="highway" v="primary"/><tag k="junction" v="roundabout"/>
  </way>
  <way id="105">
    <nd ref="21"/><nd ref="24"/><tag k="highway" v="primary"/>
  </way>
  <way id="103">
    <nd ref="30"/><nd ref="2"/>
    <tag k="highway" v="tertiary"/>
    <tag k="maxspeed:forward" v="40"/><tag k="maxspeed:backward" v="60"/>
  </way>
  <way id="106">
    <nd ref="30"/><nd ref="1"/><tag k="highway" v="unclassified"/>
  </way>
  <way id="104">
    <nd ref="31"/><nd ref="30"/>
    <tag k="highway" v="service"/><tag k="maxspeed" v="walk"/><tag k="oneway" v="yes"/>
    <tag k="parking:left" v="lane"/><tag k="parking:right" v="no"/>
  </way>
  <node id="40" lat="47.1001" lon="9.4925">
    <tag k="highway" v="bus_stop"/><tag k="name" v="Ada Platz"/>
  </node>
  <relation id="500">
    <member type="way" ref="101" role=""/>
    <member type="way" ref="100" role=""/>
    <member type="way" ref="102" role=""/>
    <member type="way" ref="105" role=""/>
    <member type="node" ref="40" role="platform"/>
    <tag k="type" v="route"/><tag k="route" v="bus"/><tag k="ref" v="12"/>
    <tag k="name" v="Bus 12: Bea Dorf - Ada Platz"/>
  </relation>
  <node id="50" lat="47.0955" lon="9.4955">
    <tag k="place" v="village"/><tag k="name" v="Bea Dorf"/>
  </node>
  <node id="71" lat="47.1030" lon="9.5003"/>
  <node id="72" lat="47.1030" lon="9.5009"/>
  <node id="73" lat="47.1034" lon="9.5009"/>
  <node id="74" lat="47.1034" lon="9.5003"/>
  <way id="700">
    <nd ref="71"/><nd ref="72"/><nd ref="73"/><nd ref="74"/><nd ref="71"/>
    <tag k="amenity" v="parking"/><tag k="capacity" v="40"/><tag k="parking" v="surface"/>
  </way>
  <node id="81" lat="47.0900" lon="9.4800"/>
  <node id="82" lat="47.0900" lon="9.4900"/>
  <way id="801">
    <nd ref="81"/><nd ref="82"/>
    <tag k="aeroway" v="runway"/><tag k="ref" v="09/27"/><tag k="surface" v="grass"/>
    <tag k="width" v="25"/>
  </way>
  <node id="83" lat="47.0890" lon="9.4790"/>
  <node id="84" lat="47.0890" lon="9.4910"/>
  <node id="85" lat="47.0910" lon="9.4910"/>
  <node id="86" lat="47.0910" lon="9.4790"/>
  <way id="800">
    <nd ref="83"/><nd ref="84"/><nd ref="85"/><nd ref="86"/><nd ref="83"/>
    <tag k="aeroway" v="aerodrome"/><tag k="name" v="Flugfeld Ada"/>
    <tag k="icao" v="LSXA"/><tag k="aerodrome:type" v="gliding"/><tag k="ele" v="455"/>
  </way>
  <node id="87" lat="47.1050" lon="9.4900"><tag k="aeroway" v="helipad"/></node>
  <node id="88" lat="47.1060" lon="9.4910">
    <tag k="amenity" v="hospital"/><tag k="name" v="Spital Cora"/>
  </node>
  <node id="90" lat="47.0800" lon="9.4700"/>
  <node id="91" lat="47.0800" lon="9.5300"/>
  <node id="92" lat="47.1200" lon="9.5300"/>
  <node id="93" lat="47.1200" lon="9.4700"/>
  <way id="601"><nd ref="90"/><nd ref="91"/><nd ref="92"/><nd ref="93"/><nd ref="90"/></way>
  <relation id="600">
    <member type="way" ref="601" role="outer"/>
    <tag k="type" v="boundary"/><tag k="boundary" v="administrative"/>
    <tag k="admin_level" v="2"/><tag k="ISO3166-1" v="LI"/><tag k="name" v="Adaland"/>
  </relation>
</osm>
"#;

fn package(zone: &str) -> WorldPackage {
    let mut tiles = BTreeMap::new();
    for x in -3..3 {
        for z in -3..3 {
            let key = TileKey { x, z };
            tiles.insert(
                key,
                ocw_world::Tile {
                    key,
                    heights: vec![0.0; 4],
                    roads: Vec::new(),
                },
            );
        }
    }
    WorldPackage {
        name: "Fixture".into(),
        origin: (47.10, 9.50),
        tile_size_m: 1000.0,
        samples: 2,
        tiles,
        graph: None,
        landuse: None,
        buildings: None,
        bridges: None,
        routes: Vec::new(),
        region: None,
        time_zone: Some(zone.into()),
        look: Default::default(),
        obstacles: None,
        places: None,
        junctions: None,
        traffic: None,
        aero: None,
    }
}

/// The fixture built into a package; `outline`: with the country outline.
fn built(zone: &str, outline: bool) -> (WorldPackage, TrafficStats) {
    let osm = OsmData::parse_osm_xml(FIXTURE).unwrap();
    let mut raw = parse_osm_xml_raw(FIXTURE).unwrap();
    if !outline {
        raw.relations.retain(|r| r.id != 600);
    }
    let (graph, _) = build_road_graph(&osm, &Region::bbox(47.0, 9.4, 47.2, 9.6));
    let mut pkg = package(zone);
    pkg.graph = Some(graph.into());
    let proj = LocalProjection::new(47.10, 9.50);
    let mut places = PlaceData::from_raw(&raw);
    places.retain_near((47.0, 9.4, 47.2, 9.6));
    add_places(&mut pkg, &places, &proj);
    let data = TrafficOsm::from_raw(&raw);
    let st = add_traffic(&mut pkg, &osm, &data, &proj);
    (pkg, st)
}

/// The edge of way `way` (the `n`-th by its start along the way).
fn edge_of(pkg: &WorldPackage, way: i64, n: usize) -> Edge {
    let g = pkg.graph.as_ref().unwrap();
    let mut es: Vec<&Edge> = g.edges.iter().filter(|e| e.osm_way == way).collect();
    es.sort_by(|a, b| {
        let x = |e: &Edge| e.points[0][0].min(e.points.last().unwrap()[0]);
        x(a).total_cmp(&x(b))
    });
    es[n].clone()
}

fn rules_of(pkg: &WorldPackage, e: &Edge) -> ocw_world::traffic::EdgeRules {
    *pkg.traffic.as_ref().unwrap().get(e.id).expect("rules")
}

#[test]
fn edge_rules_from_the_ways() {
    let (pkg, st) = built("Europe/Zurich", true);
    let t = pkg.traffic.as_ref().unwrap();
    assert_eq!(t.country, Country::Li, "{}", st.summary());
    assert_eq!(t.edges.len(), pkg.graph.as_ref().unwrap().edges.len());
    // The main road outside the village: Liechtenstein's implied 80.
    let main = edge_of(&pkg, 100, 0);
    let r = rules_of(&pkg, &main);
    assert_eq!(r.country, Country::Li);
    assert!(!r.urban);
    assert_eq!(r.limit(true), SpeedLimit::Kmh(80));
    assert_eq!(r.speed_source, SpeedSource::Implied);
    assert_eq!(r.access, Access::ALL);
    // The one-way street drawn against its traffic: cars only from 10 to 3,
    // no lorries, parked cars on both sides, 50 in town.
    let street = edge_of(&pkg, 101, 0);
    let r = rules_of(&pkg, &street);
    let forward_is_way = (street.points[0][1] - street.points.last().unwrap()[1]) > 0.0;
    assert!(forward_is_way, "the graph keeps 3 → 10 (from the junction)");
    assert_eq!(r.oneway, Oneway::Backward);
    assert!(r.allows(Vehicle::Car, false) && !r.allows(Vehicle::Car, true));
    assert!(!r.access.allows(Vehicle::Hgv) && r.access.allows(Vehicle::Bus));
    assert!(r.parking.iter().all(|p| p.place == ParkingPlace::Lane));
    assert_eq!(r.limit(false), SpeedLimit::Kmh(50));
    // The roundabout.
    let g = pkg.graph.as_ref().unwrap();
    let ring: Vec<&Edge> = g.edges.iter().filter(|e| e.osm_way == 102).collect();
    assert!(!ring.is_empty());
    for e in &ring {
        let r = rules_of(&pkg, e);
        assert!(r.roundabout);
        assert_ne!(r.oneway, Oneway::No);
    }
    // The tertiary road: its limits per direction; in town by the village.
    let side = edge_of(&pkg, 103, 0);
    let r = rules_of(&pkg, &side);
    assert_eq!(r.limit(true), SpeedLimit::Kmh(40));
    assert_eq!(r.limit(false), SpeedLimit::Kmh(60));
    assert_eq!(r.speed_source, SpeedSource::Tagged);
    assert!(r.urban, "within the village's radius");
    // The service road, turned round by the graph (from the junction to
    // its dead end): its one-way and its sides turn round with it.
    let walk = edge_of(&pkg, 104, 0);
    assert!(
        walk.points[0][1] < walk.points.last().unwrap()[1],
        "the edge runs 30 → 31 (south)"
    );
    let r = rules_of(&pkg, &walk);
    assert_eq!(r.limit(true), SpeedLimit::Walk);
    assert_eq!(r.oneway, Oneway::Backward);
    assert_eq!(r.parking[0].place, ParkingPlace::No);
    assert_eq!(r.parking[1].place, ParkingPlace::Lane);
    assert_eq!(st.motor_edges, 10, "{}", st.summary());
    assert!(st.oneway >= 4 && st.no_hgv >= 1);
    assert_eq!(st.countries, vec![(Country::Li, st.edges)]);
}

#[test]
fn implied_limits_follow_the_time_zone_without_an_outline() {
    let (pkg, _) = built("Europe/Berlin", false);
    let t = pkg.traffic.as_ref().unwrap();
    assert_eq!(t.country, Country::De);
    let r = rules_of(&pkg, &edge_of(&pkg, 100, 0));
    assert_eq!(r.limit(true), SpeedLimit::Kmh(100));
    let (pkg, _) = built("Europe/Andorra", false);
    assert_eq!(
        rules_of(&pkg, &edge_of(&pkg, 100, 0)).limit(true),
        SpeedLimit::Kmh(90)
    );
    let (pkg, _) = built("Atlantic/Canary", false);
    let r = rules_of(&pkg, &edge_of(&pkg, 100, 0));
    assert_eq!(
        (r.country, r.limit(true)),
        (Country::Es, SpeedLimit::Kmh(90))
    );
    // In town with one lane each way: 30 in Spain.
    assert_eq!(
        rules_of(&pkg, &edge_of(&pkg, 101, 0)).limit(false),
        SpeedLimit::Kmh(30)
    );
}

#[test]
fn signals_and_signs() {
    let (pkg, st) = built("Europe/Vaduz", true);
    let t = pkg.traffic.as_ref().unwrap();
    // Junction signals at node 3: on the three arms, towards the junction.
    let at3: Vec<_> = t.controls.iter().filter(|c| c.osm_node == 3).collect();
    assert_eq!(at3.len(), 3, "{:?}", t.controls);
    assert!(
        at3.iter()
            .all(|c| c.kind == ControlKind::Signals && c.at_junction)
    );
    let west = edge_of(&pkg, 100, 1); // 2 – 3
    let c = t
        .controls_on(west.id)
        .iter()
        .find(|c| c.osm_node == 3)
        .unwrap();
    assert_eq!(c.applies, Applies::Forward);
    assert!((c.along_m - west.length_m() as f32).abs() < 0.5);
    let street = edge_of(&pkg, 101, 0); // 3 – 10
    let c = t
        .controls_on(street.id)
        .iter()
        .find(|c| c.osm_node == 3)
        .unwrap();
    assert_eq!((c.applies, c.along_m), (Applies::Backward, 0.0));
    // Crossing lights along the edge 3 – 5, for both directions.
    let east = edge_of(&pkg, 100, 2);
    let c = t
        .controls_on(east.id)
        .iter()
        .find(|c| c.osm_node == 4)
        .unwrap();
    assert_eq!(c.kind, ControlKind::CrossingSignals);
    assert_eq!(c.applies, Applies::Both);
    assert!(!c.at_junction);
    assert!((c.along_m - east.length_m() as f32 / 2.0).abs() < 30.0);
    // The give-way sign at node 2 is for the minor road only.
    let at2: Vec<_> = t.controls.iter().filter(|c| c.osm_node == 2).collect();
    assert_eq!(at2.len(), 1);
    let side = edge_of(&pkg, 103, 0);
    assert_eq!(at2[0].edge, side.id);
    assert_eq!(at2[0].kind, ControlKind::GiveWay);
    assert_eq!(
        at2[0].applies,
        Applies::Forward,
        "towards its end at node 2"
    );
    assert!((at2[0].along_m - side.length_m() as f32).abs() < 0.5);
    assert_eq!((st.signals, st.give_way, st.stops_signs), (4, 1, 0));
}

#[test]
fn bus_line_stops_and_car_park() {
    let (pkg, st) = built("Europe/Vaduz", true);
    let t = pkg.traffic.as_ref().unwrap();
    assert_eq!(t.stops.len(), 1);
    let s = &t.stops[0];
    assert_eq!(s.name, "Ada Platz");
    assert_eq!(s.edge, Some(edge_of(&pkg, 100, 0).id));
    assert_eq!(s.side, Side::Left, "north of an eastbound road");
    assert_eq!(t.bus_routes.len(), 1, "{}", st.summary());
    let r = &t.bus_routes[0];
    assert_eq!(r.reference, "12");
    // Down the one-way street (against the edge), then east from the
    // junction (not the part of the main road west of it), the roundabout
    // left to the core, then on east.
    let street = edge_of(&pkg, 101, 0);
    let east = edge_of(&pkg, 100, 2);
    let on = edge_of(&pkg, 105, 0);
    assert_eq!(r.legs[0], (street.id, false));
    assert_eq!(r.legs[1], (east.id, true));
    assert_eq!(r.legs.last().copied(), Some((on.id, true)));
    assert_eq!(r.legs.len(), 3, "{:?}", r.legs);
    assert_eq!(r.stops, vec![0]);
    // The car park by the one-way street.
    assert_eq!(t.parking.len(), 1);
    let p = &t.parking[0];
    assert_eq!((p.capacity, p.estimated, p.public), (40, false, true));
    assert_eq!(p.osm_id, -700);
    assert_eq!(p.edge, Some(street.id));
}

#[test]
fn aerodromes_runways_helipads_and_nearby_airports() {
    let (pkg, st) = built("Europe/Vaduz", true);
    let a = pkg.aero.as_ref().unwrap();
    let field = a.by_icao("LSXA").expect("the fixture's airfield");
    assert_eq!(field.kind, ocw_world::aero::AerodromeKind::Gliding);
    assert_eq!(field.name, "Flugfeld Ada");
    assert_eq!(field.ele_m, Some(455.0));
    assert!(!field.outside);
    let rw: Vec<_> = a
        .runways
        .iter()
        .filter(|r| r.reference == "09/27")
        .collect();
    assert_eq!(rw.len(), 1);
    assert!(
        (rw[0].heading_deg - 90.0).abs() < 1.0,
        "{}",
        rw[0].heading_deg
    );
    assert!((rw[0].length_m - 758.0).abs() < 15.0, "{}", rw[0].length_m);
    assert_eq!(rw[0].width_m, 25.0);
    assert_eq!(rw[0].surface, "grass");
    assert_eq!(
        rw[0].aerodrome.map(usize::from),
        a.aerodromes.iter().position(|d| d.icao == "LSXA")
    );
    assert_eq!(a.helipads.len(), 1);
    assert!(a.helipads[0].hospital && !a.helipads[0].heliport);
    // Zurich, Altenrhein and Friedrichshafen are outside but in its sky.
    for icao in ["LSZH", "LSZR", "EDNY"] {
        assert!(a.by_icao(icao).is_some_and(|d| d.outside), "{icao}");
    }
    assert_eq!(st.aerodromes, 1);
    assert!(st.nearby >= 3);
}

#[test]
fn round_trip_and_old_packages() {
    let (pkg, _) = built("Europe/Vaduz", true);
    let bytes = pkg.to_bytes();
    let tags: Vec<[u8; 4]> = ocw_world::package::section_sizes(&bytes)
        .iter()
        .map(|s| s.0)
        .collect();
    assert!(tags.contains(ocw_world::traffic::TRAFFIC_TAG));
    assert!(tags.contains(ocw_world::aero::AERO_TAG));
    let back = WorldPackage::from_bytes(&bytes).unwrap();
    assert_eq!(back.traffic, pkg.traffic);
    assert_eq!(back.aero, pkg.aero);
    assert_eq!(back, pkg);
    // A reader that does not know the sections skips them: renamed tags.
    let mut old = bytes.clone();
    for tag in [ocw_world::traffic::TRAFFIC_TAG, ocw_world::aero::AERO_TAG] {
        let at = old.windows(4).rposition(|w| w == tag).unwrap();
        old[at] = b'x';
    }
    let o = WorldPackage::from_bytes(&old).unwrap();
    assert!(o.traffic.is_none() && o.aero.is_none());
    assert_eq!(o.graph, pkg.graph);
    // An old package (no sections): "no traffic data", the implied rules
    // by class and the country of the time zone.
    let mut plain = pkg.clone();
    plain.traffic = None;
    plain.aero = None;
    let o = WorldPackage::from_bytes(&plain.to_bytes()).unwrap();
    assert!(o.traffic.is_none());
    let rules = RoadRules::of(&o);
    assert_eq!(rules.country, Country::Li);
    let g = o.graph.as_ref().unwrap();
    let all = rules.aligned(g);
    assert_eq!(all.len(), g.edges.len());
    let main = edge_of(&o, 100, 0);
    let r = rules.edge(&main);
    assert_eq!(r.limit(true), SpeedLimit::Kmh(80));
    assert_eq!(r.oneway, Oneway::No);
    assert!(rules.controls_on(main.id).is_empty());
    // The same edge from TRFC.
    let with = RoadRules::of(&back);
    assert_eq!(with.edge(&main), rules_of(&pkg, &main));
    assert_eq!(rules_of(&pkg, &main).speed_source, SpeedSource::Implied);
    assert_eq!(main.class, RoadClass::Primary);
}

#[test]
fn speed_coverage_counts_roads_without_a_limit() {
    let (pkg, _) = built("Europe/Vaduz", true);
    let c = pkg
        .traffic
        .as_ref()
        .unwrap()
        .speed_coverage(pkg.graph.as_ref().unwrap());
    let total = c.total();
    assert_eq!(c.missing_edges, 0);
    assert!(total.edges >= 6);
    // Mapped: the tertiary (per direction) and the service road (walk);
    // the unclassified road and the rest are unmapped.
    assert_eq!(total.edges - total.unmapped_edges, 2);
    assert!(c.by_class[&RoadClass::Primary].unmapped_km > 0.5);
}

/// The traffic core reads the built package's rules (spec 18E T5):
/// the one-way street's direction and its lorry ban, the lights at node 3
/// on every arm, the give-way sign for the tertiary road at node 2 (the
/// main road has right of way there), and a simulated busy hour keeps
/// them.
#[test]
fn the_traffic_core_drives_by_the_built_rules() {
    use ocw_sim::traffic::WorldRules;
    use ocw_sim::traffic::{
        Bubble, Control, DemandTime, EdgeRules as _, TrafficConfig, TrafficLevel, TrafficSim,
        VehicleClass,
    };
    let (pkg, _) = built("Europe/Vaduz", true);
    let net = ocw_world::RoadNetwork::new(pkg.graph.clone().unwrap()).with_traffic_of(&pkg);
    let rules = WorldRules::new(&net);
    assert!(rules.from_data());
    let index = |e: &Edge| net.edge_index(e.id).unwrap();
    let street = index(&edge_of(&pkg, 101, 0));
    let r = rules.edge_rule(&net, street);
    assert!(!r.allows(VehicleClass::Car, true) && r.allows(VehicleClass::Car, false));
    assert!(!r.access.allows(VehicleClass::Lorry));
    // Node 3: the end of the main road's 2 – 3 edge; lights on all arms.
    let west = edge_of(&pkg, 100, 1);
    let v3 = west.to;
    for &e in net.edges_at(v3) {
        let fwd = net.edge(e).to == v3;
        assert_eq!(
            rules.node_control(&net, v3, e, fwd),
            Some(Control::Signals),
            "arm {e}"
        );
    }
    // Node 2: the tertiary road gives way, the main road has priority.
    let side = edge_of(&pkg, 103, 0);
    let v2 = side.to;
    assert_eq!(
        rules.node_control(&net, v2, index(&side), true),
        Some(Control::GiveWay)
    );
    let main = index(&edge_of(&pkg, 100, 0));
    assert_eq!(
        rules.node_control(&net, v2, main, net.edge(main).to == v2),
        Some(Control::Priority)
    );
    // A busy hour on the package's rules.
    let mut sim = TrafficSim::new(
        &net,
        Box::new(rules),
        TrafficConfig {
            seed: 11,
            step_ms: 100,
            level: TrafficLevel::Busy,
            ..Default::default()
        },
    );
    let bubble = Bubble {
        center: [0.0, 0.0],
        radius: 1500.0,
    };
    let mut seen = 0;
    for t in 0..36_000u32 {
        let time = DemandTime::weekday_at(8.0).plus_seconds(t / 10);
        sim.step(&net, &[bubble], &[], &time);
        for (e, fwd, class) in sim.placements() {
            assert!(!(e == street && fwd), "against the one-way street");
            assert!(!(e == street && class == VehicleClass::Lorry), "a lorry");
            seen += 1;
        }
        sim.drain_events();
    }
    assert!(seen > 1000, "{seen} vehicle-ticks");
}
