//! Tests of the live road events (spec 18E T13): a recorded excerpt of the
//! DGT's DATEX II feed (our own fetch, CC BY, "Fuente: DGT": the fixture's
//! header) → the records of Tenerife → edge events on an excerpt of the
//! Tenerife package's road graph (the TF-5 and the TF-82 at Icod de los
//! Vinos); the game server lays them over its traffic and tells the riders
//! who want them; the feed down → the statistics with a note; no fetch for
//! worlds without a source; the request budget; no raw record in any
//! message. No network.

use std::sync::{Arc, Mutex};

use ocw_protocol::v1;
use ocw_sim::traffic::live::{Dir, LiveKind, Unmapped, map_events};
use ocw_sim::traffic::{ClassRules, EdgeRule, EdgeRules, VehicleClass};
use ocw_world::RoadNetwork;
use ocw_world::graph::{Edge, EdgeId, Oneway, RoadClass, RoadGraph, VertexId};

use super::*;

const FEED: &str = include_str!("fixtures/dgt-2026-11-28.xml");
const ICOD: &str = include_str!("fixtures/tenerife-icod.txt");

/// 2026-11-28 10:00 UTC: every recorded record holds.
const T0: i64 = 1_795_860_000_000;

fn tenerife() -> GeoBox {
    SOURCES[0].area
}

/// The Icod excerpt of the Tenerife package (OSM-derived, ODbL) and the
/// place of its local origin.
fn icod() -> (RoadNetwork, (f64, f64)) {
    let mut vertices = Vec::new();
    let mut edges = Vec::new();
    let mut origin = None;
    let num = |x: &str| x.parse::<f64>().expect("number");
    for line in ICOD.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        match f.as_slice() {
            ["#", "origin", lat, lon] => origin = Some((num(lat), num(lon))),
            ["v", x, z] => vertices.push([num(x), num(z)]),
            ["e", from, to, class, oneway, width, pts @ ..] => {
                let class = match *class {
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
                let oneway = match *oneway {
                    "Forward" => Oneway::Forward,
                    "Backward" => Oneway::Backward,
                    _ => Oneway::No,
                };
                let points = pts
                    .iter()
                    .map(|p| {
                        let (x, z) = p.split_once(',').expect("x,z");
                        [num(x), num(z)]
                    })
                    .collect();
                let i = edges.len();
                edges.push(Edge {
                    id: EdgeId(13_000 + i as u64),
                    from: VertexId(from.parse().expect("from")),
                    to: VertexId(to.parse().expect("to")),
                    points,
                    class,
                    name: None,
                    width_m: num(width),
                    bridge: false,
                    tunnel: false,
                    dead_end_m: None,
                    boundary: false,
                    oneway,
                    osm_way: i as i64,
                    profile: Vec::new(),
                });
            }
            _ => {}
        }
    }
    let graph = RoadGraph {
        vertices,
        edges,
        boundary_vertices: Default::default(),
    };
    (RoadNetwork::new(graph), origin.expect("an origin line"))
}

fn class_rules(net: &RoadNetwork) -> Vec<EdgeRule> {
    (0..net.graph().edges.len())
        .map(|e| ClassRules.edge_rule(net, e))
        .collect()
}

fn parse(area: &GeoBox) -> Vec<RoadRecord> {
    DgtDatex2.parse(&mut FEED.as_bytes(), area).expect("parsed")
}

fn by_road<'a>(r: &'a [RoadRecord], road: &str) -> Vec<&'a RoadRecord> {
    r.iter().filter(|x| x.event.road == road).collect()
}

#[test]
fn the_fixture_names_its_source_licence_and_changes() {
    let head = &FEED[..FEED.find("<d2:payload").expect("payload")];
    assert!(head.contains("nap.dgt.es/datex2/v3/dgt/SituationPublication/datex2_v37.xml"));
    assert!(head.contains("Creative Commons Attribution"));
    assert!(head.contains("\"Fuente: DGT\""));
    assert!(head.contains("2026-11-28"));
    assert!(head.contains("made for the test"));
    assert!(ICOD.contains("OpenStreetMap contributors, ODbL"));
    assert_eq!(DgtDatex2.attribution(), DGT_ATTRIBUTION);
    assert_eq!(DgtDatex2.request(1).url, DGT_URL);
}

#[test]
fn tenerifes_records_are_kept_and_the_rest_of_spain_left_out() {
    let r = parse(&tenerife());
    assert!(r.iter().all(|x| x.event.road.starts_with("TF-")), "{r:#?}");
    for road in [
        "TF-5", "TF-82", "TF-375", "TF-436", "TF-445", "TF-342", "TF-13",
    ] {
        assert!(!by_road(&r, road).is_empty(), "{road}");
    }
    // The TF-82 at El Tanque: closed both ways, a record per direction.
    let tf82 = by_road(&r, "TF-82");
    assert_eq!(tf82.len(), 2);
    assert!(
        tf82.iter()
            .all(|x| x.event.kind == LiveKind::Closed && x.event.one_direction)
    );
    assert_eq!(tf82[0].event.length_m.map(|l| l.round()), Some(500.0));
    // The TF-445 (Teno) closed after a rock fall, direction unknown.
    let tf445 = by_road(&r, "TF-445");
    assert!(tf445[0].event.kind == LiveKind::Closed && !tf445[0].event.one_direction);
    // The TF-5 at Icod: roadworks (its speed records' limits of 1 km/h
    // are not believed: roadworks too) and the reported jam.
    let tf5 = by_road(&r, "TF-5");
    assert!(
        tf5.iter()
            .filter(|x| x.event.kind == LiveKind::Roadworks)
            .count()
            >= 3
    );
    let jam: Vec<_> = tf5
        .iter()
        .filter(|x| x.event.kind == LiveKind::Jam)
        .collect();
    assert_eq!(jam.len(), 1);
    assert_eq!(
        jam[0].event.speed_kmh,
        Some(ocw_sim::traffic::live::JAM_STATIONARY_KMH)
    );
    assert_eq!(jam[0].event.bearing_deg, Some(270.0));
    // Short closures on the TF-375: one lane in turns.
    assert!(
        by_road(&r, "TF-375")
            .iter()
            .any(|x| x.event.kind == LiveKind::LaneClosed)
    );
    // Broken-down vehicles: points.
    let tf342 = by_road(&r, "TF-342");
    assert!(tf342[0].event.kind == LiveKind::Obstruction && tf342[0].event.to.is_none());
    // Times: the TF-82's until March 2027.
    assert!(tf82[0].holds(T0));
    assert!(!tf82[0].holds(parse_time("2027-03-12T00:00:00Z").unwrap()));
    // Elsewhere in Spain: Asturias' slow traffic in its own box.
    let asturias = GeoBox {
        lat_min: 43.0,
        lat_max: 43.7,
        lon_min: -7.2,
        lon_max: -4.5,
    };
    let a = parse(&asturias);
    assert!(
        a.iter()
            .any(|x| x.event.road == "AS-15" && x.event.kind == LiveKind::Jam)
    );
    assert!(a.iter().all(|x| !x.event.road.starts_with("TF-")));
    // Not DATEX II.
    assert!(
        DgtDatex2
            .parse(&mut "<html></html>".as_bytes(), &tenerife())
            .is_err()
    );
    assert!(
        DgtDatex2
            .parse(&mut "<a><b>".as_bytes(), &tenerife())
            .is_err()
    );
}

#[test]
fn times_parse_with_their_offsets() {
    assert_eq!(parse_time("1970-01-01T00:00:00Z"), Some(0));
    assert_eq!(parse_time("2026-11-28T11:00:00.000+01:00"), Some(T0));
    assert_eq!(parse_time("2026-11-28T10:00:00.5Z"), Some(T0 + 500));
    assert_eq!(parse_time("2026-11-28T09:30:00-00:30"), Some(T0));
    assert_eq!(parse_time("soon"), None);
}

#[test]
fn the_records_map_onto_the_tf5_and_the_tf82_edges() {
    let (net, origin) = icod();
    let rules = class_rules(&net);
    let records = parse(&tenerife());
    let geo: Vec<_> = records.iter().map(|r| r.event.clone()).collect();
    let mapped = map_events(&net, &rules, origin, &geo);
    let len = |e: u32| {
        let p = &net.edge(e as usize).points;
        p.windows(2)
            .map(|w| ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt())
            .sum::<f64>()
    };
    for (r, m) in records.iter().zip(&mapped) {
        match r.event.road.as_str() {
            "TF-5" => {
                let edges = m.as_ref().expect("the TF-5 is in the excerpt");
                let total: f64 = edges.iter().map(|e| len(e.edge)).sum();
                let trunk: f64 = edges
                    .iter()
                    .filter(|e| net.edge(e.edge as usize).class == RoadClass::Trunk)
                    .map(|e| len(e.edge))
                    .sum();
                assert!(
                    trunk > 0.8 * total,
                    "on the TF-5 (trunk): {trunk:.0} of {total:.0} m"
                );
                let want = r.event.length_m.expect("km points");
                assert!(
                    (total - want).abs() < 0.4 * want + 100.0,
                    "{:?}: {total:.0} m for {want:.0} m",
                    r.event.kind
                );
                if r.event.kind == LiveKind::Jam {
                    // Westbound: from the east end of the excerpt's TF-5
                    // towards Icod.
                    assert!(edges.iter().all(|e| e.dir != Dir::Both));
                }
            }
            "TF-82" => {
                let edges = m.as_ref().expect("the TF-82 is in the excerpt");
                assert!(edges.iter().all(|e| {
                    net.edge(e.edge as usize).class == RoadClass::Primary && e.dir != Dir::Both
                }));
                let total: f64 = edges.iter().map(|e| len(e.edge)).sum();
                assert!((300.0..900.0).contains(&total), "{total:.0} m");
            }
            // Outside the excerpt.
            _ => assert_eq!(m, &Err(Unmapped::NoRoad), "{}", r.event.road),
        }
    }
    // The two TF-82 records close its two directions of the same edges.
    let tf82: Vec<_> = records
        .iter()
        .zip(&mapped)
        .filter(|(r, _)| r.event.road == "TF-82")
        .map(|(_, m)| m.clone().expect("mapped"))
        .collect();
    let mut a: Vec<u32> = tf82[0].iter().map(|e| e.edge).collect();
    let mut b: Vec<u32> = tf82[1].iter().map(|e| e.edge).collect();
    a.sort_unstable();
    b.sort_unstable();
    assert_eq!(a, b);
    assert_ne!(
        tf82[0][0].dir,
        tf82[1]
            .iter()
            .find(|e| e.edge == tf82[0][0].edge)
            .unwrap()
            .dir
    );
}

// ------------------------------------------------------------- server

use crate::{ConnectionId, GameServer, ServerConfig};
use ocw_core::traffic::RoadTraffic;
use ocw_protocol::v1::{client_message::Msg as C, server_message::Msg as S};
use ocw_protocol::{PROTOCOL_VERSION, client_message, decode_server, encode};

fn hello(name: &str) -> Vec<u8> {
    encode(&client_message(C::Hello(v1::Hello {
        protocol_version: PROTOCOL_VERSION,
        display_name: name.into(),
        profile: Some(ocw_core::RiderProfile::default().into()),
        ..Default::default()
    })))
}

fn join(s: &mut GameServer, name: &str) -> ConnectionId {
    let c = s.connect();
    let r = s.handle(c, &hello(name));
    assert!(matches!(decode_server(&r[0]), Ok(S::Welcome(_))), "{name}");
    c
}

fn road_to(s: &mut GameServer) -> Vec<(ConnectionId, v1::LiveRoadUpdate)> {
    s.take_directed()
        .into_iter()
        .filter_map(|(c, b)| match decode_server(&b) {
            Ok(S::LiveRoad(u)) => Some((c, u)),
            _ => None,
        })
        .collect()
}

fn wish(on: bool) -> Vec<u8> {
    encode(&ocw_protocol::live_road::wish(on))
}

/// The edge of the excerpt's TF-5 a rider starts on (a trunk edge).
fn tf5_start(net: &RoadNetwork) -> usize {
    (0..net.graph().edges.len())
        .filter(|&e| net.edge(e).class == RoadClass::Trunk && net.edge(e).oneway == Oneway::No)
        .max_by(|a, b| {
            let l = |e: usize| net.geometry(e).length();
            l(*a).total_cmp(&l(*b))
        })
        .expect("a trunk edge")
}

/// A Tenerife server on the Icod excerpt with busy road traffic.
fn icod_server(road: RoadTraffic, config: ServerConfig) -> (GameServer, Arc<RoadNetwork>) {
    let (net, origin) = icod();
    let net = Arc::new(net);
    let start = tf5_start(&net);
    let s = GameServer::new(
        crate::Course::FreeRide {
            net: net.clone(),
            start: ocw_sim::FreeRideStart {
                edge: start,
                offset_m: 20.0,
                forward: true,
            },
        },
        ServerConfig {
            road_traffic: road,
            real_epoch_ms: Some(T0),
            geo_origin: Some(origin),
            air_seed: Some(3),
            ..config
        },
    );
    (s, net)
}

type Log = Arc<Mutex<Vec<RoadRequest>>>;

/// A fetcher answering with the recorded feed while `up` holds.
fn recorded(up: Arc<Mutex<bool>>) -> (Box<dyn RoadFetcher>, Log) {
    let f = FnRoadFetcher::new(move |_r: &RoadRequest| {
        Some(if *up.lock().unwrap() {
            Ok(FEED.to_owned())
        } else {
            Err("connection refused".into())
        })
    });
    let log = f.log.clone();
    (Box::new(f), log)
}

#[test]
fn the_server_lays_the_events_over_its_traffic_and_tells_the_riders() {
    let (mut s, net) = icod_server(RoadTraffic::Busy, ServerConfig::default());
    let up = Arc::new(Mutex::new(true));
    let (fetcher, log) = recorded(up.clone());
    s.set_road_fetcher(fetcher);
    let ada = join(&mut s, "Ada Rider");
    // Off: told the world's source, nothing fetched.
    s.handle(ada, &wish(false));
    let told = road_to(&mut s);
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].1.status, v1::LiveRoadStatus::Available as i32);
    assert_eq!(told[0].1.attribution, "Fuente: DGT");
    assert_eq!(told[0].1.what, "closures, roadworks and reported jams");
    for _ in 0..500 {
        s.tick();
    }
    assert!(log.lock().unwrap().is_empty(), "nobody wants it");
    // On: one fetch, the events on the traffic, the rider told.
    s.handle(ada, &wish(true));
    let mut got = Vec::new();
    for _ in 0..200 {
        s.tick();
        got.extend(road_to(&mut s));
    }
    assert_eq!(log.lock().unwrap().len(), 1);
    let live = got
        .iter()
        .rev()
        .find(|(_, u)| u.status == v1::LiveRoadStatus::Live as i32)
        .expect("a live update")
        .1
        .clone();
    assert_eq!(live.attribution, "Fuente: DGT");
    assert!(live.events.iter().any(|e| e.road == "TF-82"));
    assert!(live.events.iter().any(|e| e.road == "TF-5"));
    assert_eq!(live.graph_hash, net.fingerprint());
    let sim = s.traffic_sim().expect("traffic runs");
    let events = sim.live_events().to_vec();
    assert!(!events.is_empty());
    let closed: Vec<(usize, bool)> = events
        .iter()
        .filter(|e| e.kind == LiveKind::Closed)
        .flat_map(|e| {
            [true, false]
                .into_iter()
                .filter(|f| e.dir.holds(*f))
                .map(|f| (e.edge as usize, f))
                .collect::<Vec<_>>()
        })
        .collect();
    assert!(!closed.is_empty());
    for &(e, f) in &closed {
        assert!(!sim.rule(e).allows(VehicleClass::Car, f), "edge {e} closed");
    }
    // The reported jam: walking pace on its edges.
    let jam: Vec<usize> = events
        .iter()
        .filter(|e| e.kind == LiveKind::Jam)
        .map(|e| e.edge as usize)
        .collect();
    assert!(!jam.is_empty());
    for &e in &jam {
        assert!(sim.rule(e).speed * 3.6 < 5.0, "jam on {e}");
        assert!(sim.base_rule(e).speed * 3.6 > 50.0);
    }
    // Five minutes of traffic: nothing on a closed direction.
    for t in 0..(50 * 300) {
        s.tick();
        if t % 50 == 0 {
            let sim = s.traffic_sim().expect("traffic");
            for (e, f, _) in sim.placements() {
                assert!(!closed.contains(&(e, f)), "a vehicle on closed edge {e}");
            }
        }
    }
    // No raw record in any message: no record ids, references,
    // municipalities, causes or XML.
    let bytes = encode(&ocw_protocol::server_message(S::LiveRoad(live.clone())));
    let text = String::from_utf8_lossy(&bytes);
    for raw in [
        "21185369",
        "TRZ_OB",
        "El Tanque",
        "Icod",
        "Santa Cruz",
        "roadMaintenance",
        "situation",
        "DGT DATEX",
        "<",
    ] {
        assert!(!text.contains(raw), "{raw} in a message");
    }
    // The feed goes down: the statistics again, with the note.
    *up.lock().unwrap() = false;
    let mut down = None;
    for _ in 0..(50 * 200) {
        s.tick();
        if let Some((_, u)) = road_to(&mut s)
            .into_iter()
            .find(|(_, u)| u.status == v1::LiveRoadStatus::Down as i32)
        {
            down = Some(u);
            break;
        }
    }
    let down = down.expect("told it is down");
    assert_eq!(down.note, NOTE_DOWN);
    assert!(down.events.is_empty());
    let sim = s.traffic_sim().expect("traffic");
    assert!(sim.live_events().is_empty());
    assert_eq!(sim.rules(), sim.base_rules(), "the statistics' rules");
    // Up again: live after the back-off.
    *up.lock().unwrap() = true;
    for _ in 0..(50 * 240) {
        s.tick();
    }
    assert_eq!(s.live_road().state(), FeedState::Live);
    assert!(!s.traffic_sim().expect("traffic").live_events().is_empty());
    // Off: the statistics, no more requests.
    s.handle(ada, &wish(false));
    let n = log.lock().unwrap().len();
    for _ in 0..(50 * 600) {
        s.tick();
    }
    assert_eq!(log.lock().unwrap().len(), n);
    assert!(s.traffic_sim().expect("traffic").live_events().is_empty());
}

#[test]
fn no_fetch_for_worlds_without_a_source() {
    for (world, origin) in [
        ("Liechtenstein", (47.14, 9.52)),
        ("Black Forest", (47.92, 7.96)),
        ("Andorra", (42.51, 1.52)),
    ] {
        let (net, _) = icod();
        let net = Arc::new(net);
        let start = tf5_start(&net);
        let mut s = GameServer::new(
            crate::Course::FreeRide {
                net,
                start: ocw_sim::FreeRideStart {
                    edge: start,
                    offset_m: 20.0,
                    forward: true,
                },
            },
            ServerConfig {
                road_traffic: RoadTraffic::Busy,
                real_epoch_ms: Some(T0),
                geo_origin: Some(origin),
                ..Default::default()
            },
        );
        let (fetcher, log) = recorded(Arc::new(Mutex::new(true)));
        s.set_road_fetcher(fetcher);
        let ada = join(&mut s, "Ada Rider");
        s.handle(ada, &wish(true));
        let told = road_to(&mut s);
        assert_eq!(
            told[0].1.status,
            v1::LiveRoadStatus::NoSource as i32,
            "{world}"
        );
        assert!(told[0].1.note.starts_with(NOTE_NO_SOURCE), "{world}");
        assert!(told[0].1.attribution.is_empty());
        for _ in 0..(50 * 300) {
            s.tick();
        }
        assert!(log.lock().unwrap().is_empty(), "{world}: no request");
        assert!(s.live_road_events().is_empty());
    }
    assert!(source_for((28.39, -16.52)).is_some_and(|e| e.world == "Tenerife"));
    assert!(
        source_for((28.10, -15.42)).is_none(),
        "Gran Canaria: no package"
    );
}

#[test]
fn only_the_rider_who_decides_makes_the_server_fetch() {
    // A shared ride: the host decides the road traffic (Q2).
    let (mut s, _) = icod_server(
        RoadTraffic::Busy,
        ServerConfig {
            traffic_access: crate::ClockAccess::Host,
            ..Default::default()
        },
    );
    let (fetcher, log) = recorded(Arc::new(Mutex::new(true)));
    s.set_road_fetcher(fetcher);
    let host = join(&mut s, "Ada Rider");
    let guest = join(&mut s, "Bea Climber");
    s.handle(host, &wish(false));
    s.handle(guest, &wish(true));
    let told = road_to(&mut s);
    let to_guest = told.iter().find(|(c, _)| *c == guest).expect("guest told");
    assert_eq!(to_guest.1.note, "the host decides");
    for _ in 0..(50 * 200) {
        s.tick();
    }
    assert!(log.lock().unwrap().is_empty());
    // The host wants it: fetched, and the guest gets the events too.
    s.handle(host, &wish(true));
    let mut got = Vec::new();
    for _ in 0..200 {
        s.tick();
        got.extend(road_to(&mut s));
    }
    assert_eq!(log.lock().unwrap().len(), 1);
    assert!(
        got.iter()
            .any(|(c, u)| *c == guest && u.status == v1::LiveRoadStatus::Live as i32)
    );
    // Road traffic off: nothing fetched, the row says why.
    let (mut off, _) = icod_server(RoadTraffic::Off, ServerConfig::default());
    let (fetcher, log) = recorded(Arc::new(Mutex::new(true)));
    off.set_road_fetcher(fetcher);
    let c = join(&mut off, "Cy Sprinter");
    off.handle(c, &wish(true));
    let t = road_to(&mut off);
    assert_eq!(t[0].1.note, "turn on road traffic first");
    for _ in 0..(50 * 200) {
        off.tick();
    }
    assert!(log.lock().unwrap().is_empty());
    // A server that fetches nothing (the browser's own).
    let (mut plain, _) = icod_server(RoadTraffic::Busy, ServerConfig::default());
    let d = join(&mut plain, "Dee Descender");
    plain.handle(d, &wish(true));
    assert_eq!(
        road_to(&mut plain)[0].1.status,
        v1::LiveRoadStatus::Unavailable as i32
    );
}

/// The feed stepped every second for `seconds` from `T0`, wanted, with a
/// fetcher answering `answer`: the times of its requests.
fn run_feed(
    seconds: i64,
    answer: impl FnMut(&RoadRequest) -> Option<Result<String, String>> + Send + 'static,
) -> (Vec<i64>, LiveRoadFeed) {
    let mut feed = LiveRoadFeed::new(Some((28.39, -16.52)));
    let mut f = FnRoadFetcher::new(answer);
    let log = f.log.clone();
    let mut times = Vec::new();
    for k in 0..seconds {
        let now = T0 + k * 1000;
        let before = log.lock().unwrap().len();
        feed.step(now, true, Some(&mut f));
        if log.lock().unwrap().len() > before {
            times.push(now);
        }
    }
    (times, feed)
}

#[test]
fn the_request_budget_holds_and_failures_back_off() {
    // Up all day: every two minutes, never more than 720 in 24 hours.
    let (times, feed) = run_feed(86_400 + 3_600, |_r: &RoadRequest| Some(Ok(FEED.to_owned())));
    assert!(times.windows(2).all(|w| w[1] - w[0] >= FETCH_GAP_MS));
    let day = times.iter().filter(|t| **t < T0 + 86_400_000).count();
    assert!((700..=DAY_MAX).contains(&day), "{day}");
    assert_eq!(feed.state(), FeedState::Live);
    assert_eq!(feed.stats.records, parse(&tenerife()).len());
    // Down: 1, 2, 4, … minutes between tries, at most 30.
    let (times, feed) = run_feed(4 * 3_600, |_r: &RoadRequest| Some(Err("HTTP 503".into())));
    let gaps: Vec<i64> = times.windows(2).map(|w| (w[1] - w[0]) / 1000).collect();
    assert_eq!(&gaps[..5], &[60, 120, 240, 480, 960]);
    assert!(gaps.iter().all(|g| *g <= BACKOFF_MAX_MS / 1000));
    assert_eq!(feed.state(), FeedState::Down);
    assert_eq!(feed.note(), NOTE_DOWN);
    assert!(feed.records(T0).is_empty());
    // A source that never answers: down after the timeout.
    let (times, feed) = run_feed(60, |_r: &RoadRequest| None);
    assert_eq!(times.len(), 1);
    assert_eq!(feed.state(), FeedState::Down);
}

#[test]
fn old_events_go_back_to_the_statistics() {
    let mut feed = LiveRoadFeed::new(Some((28.39, -16.52)));
    let mut first = true;
    let mut f = FnRoadFetcher::new(move |_r: &RoadRequest| {
        // The first answer, then a source that hangs every time.
        std::mem::take(&mut first).then(|| Ok(FEED.to_owned()))
    });
    assert!(feed.step(T0, true, Some(&mut f)));
    assert_eq!(feed.state(), FeedState::Live);
    assert!(!feed.records(T0).is_empty());
    // Each later try times out: down at the first failure, the records
    // gone (no stale events in the traffic).
    let mut t = T0;
    while feed.state() == FeedState::Live && t < T0 + STALE_MS + 60_000 {
        t += 1000;
        feed.step(t, true, Some(&mut f));
    }
    assert_eq!(feed.state(), FeedState::Down);
    assert!(t - T0 <= FETCH_GAP_MS + ANSWER_TIMEOUT_MS + 2_000);
    assert!(feed.records(t).is_empty());
}
