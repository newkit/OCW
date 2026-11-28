//! Tests of the counting stations into the package (spec 18E T11): the
//! counts format, station matching on fixtures made from our downloads
//! (`fixtures/`, each with its source's licence in the header), the flow
//! factors and their spread, determinism.

use super::*;
use ocw_world::graph::{Edge, EdgeId, VertexId};
use ocw_world::traffic::{Country, EdgeRules};

fn fixture(name: &str) -> (CountSource, Vec<Station>) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/traffic_counts/fixtures")
        .join(name);
    read_counts(&path).unwrap()
}

/// A graph being drawn: vertices merged by position.
#[derive(Default)]
struct Draw {
    graph: RoadGraph,
    rules: TrafficData,
    names: WayNames,
}

impl Draw {
    fn vertex(&mut self, p: [f64; 2]) -> VertexId {
        let g = &mut self.graph;
        if let Some(i) = g
            .vertices
            .iter()
            .position(|v| (v[0] - p[0]).abs() < 0.01 && (v[1] - p[1]).abs() < 0.01)
        {
            return VertexId(i as u32);
        }
        g.vertices.push(p);
        VertexId(g.vertices.len() as u32 - 1)
    }

    /// An edge along `pts`; `oneway` forward (drawn direction) or two-way.
    fn road(
        &mut self,
        pts: &[[f64; 2]],
        class: RoadClass,
        oneway: bool,
        reference: Option<&str>,
        name: Option<&str>,
    ) -> usize {
        let from = self.vertex(pts[0]);
        let to = self.vertex(*pts.last().unwrap());
        let i = self.graph.edges.len();
        let way = 1000 + i as i64;
        self.graph.edges.push(Edge {
            id: EdgeId(1 + i as u64),
            from,
            to,
            points: pts.to_vec(),
            class,
            name: name.map(str::to_string),
            width_m: class.default_width_m(),
            bridge: false,
            tunnel: false,
            dead_end_m: None,
            boundary: false,
            oneway: if oneway { Oneway::Forward } else { Oneway::No },
            osm_way: way,
            profile: Vec::new(),
        });
        let mut r = EdgeRules::implied(class, Country::Es, false);
        if oneway {
            r.oneway = Oneway::Forward;
            r.lanes = [2, 0];
        }
        self.rules.edges.insert(EdgeId(1 + i as u64), r);
        self.names.insert(way, reference, name);
        i
    }
}

/// The station's point plus `t` along the unit `d` and `s` across it.
fn at(d: [f64; 2], t: f64, s: f64) -> [f64; 2] {
    [d[0] * t - d[1] * s, d[1] * t + d[0] * s]
}

fn bearing_vec(deg: f64) -> [f64; 2] {
    let r = deg.to_radians();
    [r.sin(), -r.cos()]
}

/// Tenerife around the AÑAZA station (TF-1, 96,707 a day): the two
/// carriageways 12 m either side, each in three edges of 2 km, a TF-28
/// crossing right at the station, an unnumbered service road 28 m off;
/// near SOMOSIERRA (TF-5) only a TF-28 road.
fn tenerife() -> (Draw, LocalProjection, Vec<Station>, [usize; 6]) {
    let (_, st) = fixture("tenerife.counts.csv");
    let anaza = &st[0];
    let proj = LocalProjection::new(anaza.lat, anaza.lon);
    let d = bearing_vec(anaza.dir1_bearing_deg.unwrap());
    let mut w = Draw::default();
    let mut up = [0; 3];
    let mut down = [0; 3];
    for (k, (a, b)) in [(-3000.0, -1000.0), (-1000.0, 1000.0), (1000.0, 3000.0)]
        .into_iter()
        .enumerate()
    {
        // Direction 1 (rising kilometre points) along the bearing …
        up[k] = w.road(
            &[at(d, a, 12.0), at(d, b, 12.0)],
            RoadClass::Trunk,
            true,
            Some("TF-1"),
            Some("Autopista del Sur"),
        );
    }
    for (k, (a, b)) in [(3000.0, 1000.0), (1000.0, -1000.0), (-1000.0, -3000.0)]
        .into_iter()
        .enumerate()
    {
        // … direction 2 back on the other side.
        down[2 - k] = w.road(
            &[at(d, a, -12.0), at(d, b, -12.0)],
            RoadClass::Trunk,
            true,
            Some("TF-1"),
            Some("Autopista del Sur"),
        );
    }
    let cross = w.road(
        &[at(d, 0.0, -400.0), at(d, 0.0, 400.0)],
        RoadClass::Secondary,
        false,
        Some("TF-28"),
        None,
    );
    let service = w.road(
        &[at(d, -500.0, 28.0), at(d, 500.0, 28.0)],
        RoadClass::Service,
        false,
        None,
        None,
    );
    let somo = proj.project(st[3].lat, st[3].lon);
    w.road(
        &[
            [somo[0] - 300.0, somo[1] + 15.0],
            [somo[0] + 300.0, somo[1] + 15.0],
        ],
        RoadClass::Secondary,
        false,
        Some("TF-28"),
        None,
    );
    (
        w,
        proj,
        st,
        [up[1], down[1], up[2], down[0], cross, service],
    )
}

/// The flow-1 volume of the sim (class default × the annual day factor
/// of ocw-sim's demand; here fixed numbers keep the test self-contained).
fn unit(g: &RoadGraph, rules: &TrafficData, e: usize) -> f64 {
    let base = match g.edges[e].class {
        RoadClass::Trunk => 20_000.0,
        RoadClass::Primary => 9_000.0,
        RoadClass::Secondary => 4_500.0,
        RoadClass::Tertiary => 2_200.0,
        RoadClass::Unclassified => 700.0,
        RoadClass::Residential => 250.0,
        RoadClass::Service => 60.0,
        _ => 8.0,
    };
    let _ = rules;
    base * 0.944
}

#[test]
fn counts_files_parse_with_their_attribution() {
    for (f, n, id) in [
        ("tenerife.counts.csv", 4, "cabildo-tenerife-imd"),
        ("liechtenstein.counts.csv", 3, "li-miv"),
        ("blackforest.counts.csv", 2, "bw-svz"),
    ] {
        let (src, st) = fixture(f);
        assert_eq!(st.len(), n, "{f}");
        assert_eq!(src.id, id);
        let a = src.attribution();
        assert!(a.source.starts_with("Traffic counts: "), "{a:?}");
        assert!(a.notice.contains(&src.notice) && a.notice.contains("TRFC"));
        assert!(!a.licence.is_empty() && a.url.is_some());
    }
    let (_, bw) = fixture("blackforest.counts.csv");
    // A quoted cell with a comma-free name and brackets.
    assert_eq!(
        bw[1].name,
        "B3/B500 Baden-Baden / AS Baden-Baden A5/B500 (51)"
    );
    assert_eq!(bw[1].reference.as_deref(), Some("B 500"));
    assert_eq!(bw[1].dir, None);
    let (_, tf) = fixture("tenerife.counts.csv");
    assert_eq!(tf[0].dir, Some([50003.0, 46704.0]));
    assert_eq!(tf[0].dir1_bearing_deg, Some(229.4));
    assert!(parse_counts("id,lat\n").is_err(), "no magic line");
    assert!(
        parse_counts("# ocw-traffic-counts 1\n# source: x\nid,lat,lon,aadt\n1,1,1,1\n").is_err(),
        "no licence or notice"
    );
    assert_eq!(
        split_csv(r#"a,"b, c","d""e",f"#),
        vec!["a", "b, c", "d\"e", "f"]
    );
    assert_eq!(norm_ref("TF-1"), "TF1");
    assert_eq!(norm_ref("B 500"), "B500");
    assert!(ref_fits("H13", &["13".into()]));
}

#[test]
fn stations_match_the_carriageways_of_their_road() {
    let (w, proj, st, [up, down, up_next, down_far, cross, service]) = tenerife();
    let m = match_stations(
        &w.graph,
        &w.rules,
        &w.names,
        &proj,
        &st,
        &Params::default(),
        |_| true,
    );
    // AÑAZA: on direction 1's carriageway (the bearing), direction 2 on
    // the other – not on the TF-28 crossing right at it, not on the
    // service road.
    let a = m
        .anchors
        .iter()
        .find(|a| a.station == 0)
        .expect("AÑAZA matched");
    assert_eq!(a.edge, up);
    assert!((a.dist_m - 12.0).abs() < 0.5, "{a:?}");
    assert_eq!(a.volume, 50003.0);
    assert_eq!(a.partner, Some((down, 46704.0)));
    // CRUZ CHICA (TF-235) has no road here, GÜÍMAR is 15 km off.
    for s in [1, 2] {
        assert!(
            m.issues
                .iter()
                .any(|i| matches!(i, Issue::Unmatched { station, .. } if *station == s)),
            "{:?}",
            m.issues
        );
    }
    // SOMOSIERRA (TF-5): only a TF-28 road near – reported, not used.
    assert!(m.issues.iter().any(|i| matches!(
        i,
        Issue::RefMismatch { station: 3, wanted, found, .. } if wanted == "TF5" && found == &["TF28".to_string()]
    )), "{:?}", m.issues);
    assert_eq!(m.anchors.len(), 1);
    // The factors: the count on the counted carriageways, spread along the
    // TF-1 and falling off with distance; nothing on other roads.
    let f = flow_factors(&w.graph, &w.rules, &w.names, &m, |e| {
        unit(&w.graph, &w.rules, e)
    });
    let unit_trunk = 20_000.0 * 0.944;
    assert!((f64::from(f[up].unwrap()) - 50003.0 / unit_trunk).abs() < 0.01);
    assert!((f64::from(f[down].unwrap()) - 46704.0 / unit_trunk).abs() < 0.01);
    let near = f64::from(f[up_next].unwrap());
    let far = f64::from(f[down_far].unwrap());
    assert!(near > 2.0 && near < 50003.0 / unit_trunk, "{near}");
    assert!(
        (far - near).abs() < 0.05,
        "both 2 km from the station: {far} {near}"
    );
    assert_eq!(f[cross], None, "another road");
    assert_eq!(f[service], None);
}

/// Liechtenstein: no road numbers – the street in the station's name wins
/// over a nearer road (reported as ambiguous), two stations on one edge
/// are reported and averaged, a station outside is only counted.
#[test]
fn names_shared_edges_and_outside() {
    let (_, st) = fixture("liechtenstein.counts.csv");
    let zoll = &st[1];
    let proj = LocalProjection::new(zoll.lat, zoll.lon);
    let bridge = proj.project(st[2].lat, st[2].lon);
    let mut w = Draw::default();
    let land = w.road(
        &[[-300.0, -12.0], [300.0, -12.0]],
        RoadClass::Secondary,
        false,
        None,
        Some("Landstrasse"),
    );
    // The Zollstrasse: 18 m from the station, on past the bridge's station.
    let zs = w.road(
        &[
            [300.0, 18.0],
            [-100.0, 18.0],
            [bridge[0], bridge[1] + 5.0],
            [bridge[0] - 400.0, bridge[1] + 5.0],
        ],
        RoadClass::Secondary,
        false,
        None,
        Some("Zollstrasse"),
    );
    let m = match_stations(
        &w.graph,
        &w.rules,
        &w.names,
        &proj,
        &st,
        &Params::default(),
        |p| p[0].abs() < 3000.0 && p[1].abs() < 3000.0,
    );
    assert_eq!(m.outside, vec![0], "Bendern is 11 km north");
    let on: Vec<(usize, usize)> = m.anchors.iter().map(|a| (a.station, a.edge)).collect();
    assert_eq!(on, vec![(1, zs), (2, zs)]);
    assert!(m.issues.contains(&Issue::Ambiguous {
        station: 1,
        edges: [zs, land]
    }));
    assert!(m.issues.contains(&Issue::SharedEdge {
        edge: zs,
        stations: vec![1, 2]
    }));
    let f = flow_factors(&w.graph, &w.rules, &w.names, &m, |e| {
        unit(&w.graph, &w.rules, e)
    });
    let mean = (10229.0 + 12228.0) / 2.0 / (4500.0 * 0.944);
    assert!((f64::from(f[zs].unwrap()) - mean).abs() < 0.01);
    assert_eq!(f[land], None);
}

/// Baden-Württemberg: "B 500" matches an OSM way "B 500;B 3"; without a
/// junction the count spreads only along the same road, through
/// junctions it falls off faster, and far away it ends.
#[test]
fn spread_along_the_road_decays_by_distance_and_junctions() {
    let (_, st) = fixture("blackforest.counts.csv");
    let s = &st[0];
    let proj = LocalProjection::new(s.lat, s.lon);
    let mut w = Draw::default();
    // A B 500 west – east in 1 km edges from -2 to 30 km, a side road at
    // every vertex from 6.5 km on (junctions).
    let mut b500 = Vec::new();
    for k in -2..30 {
        // Edge k is centred on k km (the station in the middle of edge 0).
        let (a, b) = (f64::from(k) * 1000.0 - 500.0, f64::from(k) * 1000.0 + 500.0);
        b500.push(w.road(
            &[[a, 6.0], [b, 6.0]],
            RoadClass::Primary,
            false,
            Some(if k == 0 { "B 500;B 3" } else { "B 500" }),
            None,
        ));
        if k >= 6 {
            w.road(
                &[[b, 6.0], [b, 506.0]],
                RoadClass::Tertiary,
                false,
                Some("K 3758"),
                None,
            );
        }
    }
    let one = [st[0].clone()];
    let m = match_stations(
        &w.graph,
        &w.rules,
        &w.names,
        &proj,
        &one,
        &Params::default(),
        |_| true,
    );
    assert_eq!(m.anchors.len(), 1);
    assert_eq!(m.anchors[0].edge, b500[2], "the B 500;B 3 edge");
    let f = flow_factors(&w.graph, &w.rules, &w.names, &m, |e| {
        unit(&w.graph, &w.rules, e)
    });
    let counted = 9054.0 / (9000.0 * 0.944);
    let at_km = |k: usize| f64::from(f[b500[k + 2]].unwrap_or(1.0));
    assert!((at_km(0) - counted).abs() < 0.01);
    // Towards the class default (1) with distance, steeper past junctions.
    let excess = |k: usize| at_km(k) - 1.0;
    assert!(excess(1) > excess(4) && excess(4) > excess(7) && excess(7) > excess(10));
    let ratio = |a: usize, b: usize| excess(b) / excess(a);
    assert!(ratio(7, 10) < ratio(1, 4), "junctions cut faster");
    assert_eq!(f[b500[29]], None, "28 km on: out of reach");
    // The side roads are other roads.
    assert!(
        w.graph
            .edges
            .iter()
            .enumerate()
            .filter(|(_, e)| e.class == RoadClass::Tertiary)
            .all(|(i, _)| f[i].is_none())
    );
}

#[test]
fn deterministic_and_rerunnable() {
    let run = || {
        let (w, proj, st, _) = tenerife();
        let m = match_stations(
            &w.graph,
            &w.rules,
            &w.names,
            &proj,
            &st,
            &Params::default(),
            |_| true,
        );
        let f = flow_factors(&w.graph, &w.rules, &w.names, &m, |e| {
            unit(&w.graph, &w.rules, e)
        });
        (w, m, f)
    };
    let (mut w, m1, f1) = run();
    let (_, m2, f2) = run();
    assert_eq!(m1, m2);
    assert_eq!(f1, f2);
    // Applying twice gives the same data; a stale factor is reset to 1.
    let some = *w.rules.edges.keys().next().unwrap();
    w.rules.edges.get_mut(&some).unwrap().flow = 7.0;
    let n = apply_flows(&w.graph, &mut w.rules, &f1);
    let once = w.rules.clone();
    assert_eq!(apply_flows(&w.graph, &mut w.rules, &f1), n);
    assert_eq!(w.rules, once);
    assert_eq!(n, f1.iter().filter(|f| f.is_some()).count());
    let counted: Vec<f32> = w.rules.edges.values().map(|r| r.flow).collect();
    assert!(counted.iter().all(|&f| f == 1.0 || f > 1.5), "{counted:?}");
    // The map draws.
    let (_, proj, st, _) = tenerife();
    let svg = flow_svg("Test", &w.graph, &w.rules, &proj, &st, &m1);
    assert!(svg.starts_with("<svg") && svg.contains("AÑAZA"));
    assert_eq!(flow_colour(1.0), "#c9c8c4");
    assert_ne!(flow_colour(0.3), flow_colour(3.0));
}

/// A station may sit in a wide motorway's median: its own road number is
/// accepted up to 2.5 × the tolerance, an unnumbered road only within it.
#[test]
fn the_road_number_reaches_farther() {
    let (_, st) = fixture("blackforest.counts.csv");
    let s = [st[1].clone()];
    let proj = LocalProjection::new(s[0].lat, s[0].lon);
    for (reference, matched) in [(Some("B 500"), true), (None, false)] {
        let mut w = Draw::default();
        w.road(
            &[[-500.0, 70.0], [500.0, 70.0]],
            RoadClass::Trunk,
            false,
            reference,
            None,
        );
        let m = match_stations(
            &w.graph,
            &w.rules,
            &w.names,
            &proj,
            &s,
            &Params::default(),
            |_| true,
        );
        assert_eq!(m.anchors.len() == 1, matched, "{reference:?}: {m:?}");
    }
}
