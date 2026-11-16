use super::*;
use crate::graph::{Edge, VertexId};

/// The example of the module docs and spec §14 Phase 16 16C.
const EXAMPLE: &str = r#"# A hand-made example, not real map data.
ocwpatch = 1
package = "europe/testland"
title = "Testland road fixes"
author = "Ada Rider"

[[source]]
what = "surface checked on site"
licence = "CC0-1.0"
by = "Ada Rider"

[[op]]
op = "road"
way = 1
surface = "gravel"
width_m = 3.5
class = "track"
oneway = "forward"
name = "Alter Weg"
note = "gravel since the floods"

[[op]]
op = "road"
edge = "12"
bike = "closed"

[[op]]
op = "remove_road"
edge = "18446744073709551615"

[[op]]
op = "height_point"
id = "spike"
at = [47.0, 9.5]
height_m = 120.0
radius_m = 80

[[op]]
op = "move_height_point"
id = "spike"
raise_m = -2.5

[[op]]
op = "remove_height_point"
id = "old"
"#;

#[test]
fn example_parses_with_lines() {
    let (p, lines) = WorldPatch::parse_located(EXAMPLE).unwrap();
    assert_eq!(p.ocwpatch, 1);
    assert_eq!(p.package, "europe/testland");
    assert_eq!(p.sources.len(), 1);
    assert_eq!(p.ops.len(), 6);
    assert_eq!(lines, vec![12, 22, 27, 31, 38, 43]);
    let Operation::Road(r) = &p.ops[0] else {
        panic!("{:?}", p.ops[0])
    };
    assert_eq!(r.way, Some(1));
    assert_eq!(r.class, Some(PatchClass::Track));
    assert_eq!(r.oneway, Some(PatchOneway::Forward));
    assert_eq!(r.width_m, Some(3.5));
    let Operation::RemoveRoad(rr) = &p.ops[2] else {
        panic!()
    };
    // Edge ids beyond TOML's i64 survive as strings.
    assert_eq!(rr.edge, Some(EdgeId(u64::MAX)));
    let Operation::HeightPoint(h) = &p.ops[3] else {
        panic!()
    };
    // An integer for a float field is fine.
    assert_eq!(h.radius_m, Some(80.0));
}

#[test]
fn round_trip() {
    let p = WorldPatch::parse(EXAMPLE).unwrap();
    let text = p.to_toml();
    let again = WorldPatch::parse(&text).unwrap();
    assert_eq!(p, again, "{text}");
    // Serialising again gives the same text (deterministic).
    assert_eq!(text, again.to_toml());
    // A patch built in code round-trips too.
    let mut q = WorldPatch::new("europe/testland");
    q.description = Some("drei \"Zeilen\"\nund Umlaute: äöü".into());
    q.ops.push(Operation::HeightPoint(HeightPoint {
        id: "ramp".into(),
        node: Some(42),
        edge: Some(EdgeId(9_876_543_210_123)),
        raise_m: Some(1.25),
        ..Default::default()
    }));
    q.ops.push(Operation::Road(RoadEdit {
        edge: Some(EdgeId(7)),
        name: Some(String::new()),
        ..Default::default()
    }));
    assert_eq!(WorldPatch::parse(&q.to_toml()).unwrap(), q);
}

fn err(text: &str) -> PatchError {
    WorldPatch::parse(text).unwrap_err()
}

#[test]
fn errors_name_the_line() {
    let head = "ocwpatch = 1\npackage = \"x\"\n";
    let e = err(&format!("{head}\n[[op]]\nop = \"bridge\"\nway = 3\n"));
    assert!(matches!(e.line, Some(4 | 5)), "{e}");
    assert!(e.message.contains("bridge"), "{e}");

    let e = err(&format!(
        "{head}\n[[op]]\nop = \"road\"\nway = 3\nsurfce = \"gravel\"\n"
    ));
    assert!(e.line.is_some(), "{e}");
    assert!(e.message.contains("surfce"), "{e}");

    let e = err(&format!(
        "{head}[[op]]\nop = \"road\"\nway = 3\nsurface = \"lava\"\n"
    ));
    assert_eq!(e.line, Some(3), "{e}");
    assert!(e.message.contains("lava"), "{e}");

    let e = err(&format!(
        "{head}[[op]]\nop = \"road\"\nway = 3\nedge = \"4\"\nname = \"A\"\n"
    ));
    assert!(e.message.contains("exactly one target"), "{e}");

    let e = err(&format!("{head}[[op]]\nop = \"road\"\nway = 3\n"));
    assert!(e.message.contains("changes nothing"), "{e}");

    let e = err(&format!(
        "{head}[[op]]\nop = \"height_point\"\nid = \"a\"\nat = [47.0, 9.5]\nheight_m = 1.0\n\n[[op]]\nop = \"height_point\"\nid = \"a\"\nat = [47.0, 9.5]\nraise_m = 1.0\n"
    ));
    assert_eq!(e.line, Some(9), "{e}");
    assert!(e.message.contains("twice"), "{e}");

    let e = err(&format!(
        "{head}[[op]]\nop = \"height_point\"\nid = \"a\"\nat = [95.0, 9.5]\nheight_m = 1.0\n"
    ));
    assert!(e.message.contains("lat, lon"), "{e}");

    let e = err(&format!(
        "{head}[[op]]\nop = \"remove_road\"\nedge = \"x12\"\n"
    ));
    assert!(e.message.contains("x12"), "{e}");

    // Broken TOML.
    let e = err("ocwpatch = 1\npackage = \"x\"\n[[op]\n");
    assert_eq!(e.line, Some(3), "{e}");

    // Versions.
    assert!(err("package = \"x\"\n").message.contains("ocwpatch = 1"));
    assert!(
        err("ocwpatch = 2\npackage = \"x\"\nfuture = true\n")
            .message
            .contains("newer")
    );
    assert!(
        err("ocwpatch = \"one\"\npackage = \"x\"\n")
            .message
            .contains("format version")
    );
    // Sources need a licence.
    let e = err(&format!(
        "{head}[[source]]\nwhat = \"a GPX track\"\nlicence = \"\"\n"
    ));
    assert!(e.message.contains("licence"), "{e}");
    assert_eq!(e.to_string(), e.message);
}

// ---------------------------------------------------------------------------
// A small graph: a T at v1.
//
//   v0 ---- e0 (way 1) ---- v1 ---- e1 (way 1) ---- v2
//                           |
//                           e2 (way 2)
//                           |
//                           v3

fn proj() -> LocalProjection {
    LocalProjection::new(47.0, 9.5)
}

fn edge(id: u64, way: i64, from: u32, to: u32, pts: Vec<[f64; 2]>, height: f64) -> Edge {
    let mut e = Edge {
        id: EdgeId(id),
        from: VertexId(from),
        to: VertexId(to),
        points: pts,
        class: RoadClass::Residential,
        name: Some("Hauptweg".into()),
        width_m: RoadClass::Residential.default_width_m(),
        bridge: false,
        tunnel: false,
        dead_end_m: None,
        boundary: false,
        oneway: Oneway::No,
        osm_way: way,
        profile: Vec::new(),
    };
    let n = (e.length_m() / PROFILE_STEP_M).ceil() as usize + 1;
    e.profile = vec![height; n];
    e
}

fn graph() -> RoadGraph {
    let v = vec![[0.0, 0.0], [500.0, 0.0], [1000.0, 0.0], [500.0, 400.0]];
    RoadGraph {
        edges: vec![
            edge(10, 1, 0, 1, vec![v[0], v[1]], 100.0),
            edge(11, 1, 1, 2, vec![v[1], v[2]], 100.0),
            edge(12, 2, 1, 3, vec![v[1], v[3]], 100.0),
        ],
        vertices: v,
        boundary_vertices: Default::default(),
    }
}

fn stack(ops: Vec<Operation>) -> Vec<LoadedPatch> {
    let mut p = WorldPatch::new("europe/testland");
    p.ops = ops;
    // Through the text, as the builder reads it.
    vec![LoadedPatch::parse("test.ocwpatch", &p.to_toml()).unwrap()]
}

fn latlon(p: [f64; 2]) -> [f64; 2] {
    let (lat, lon) = proj().unproject(p);
    [lat, lon]
}

#[test]
fn road_attributes_on_edges_and_ways() {
    let s = stack(vec![
        Operation::Road(RoadEdit {
            edge: Some(EdgeId(10)),
            surface: Some("paving_stones".into()),
            class: Some(PatchClass::Track),
            oneway: Some(PatchOneway::Backward),
            name: Some(String::new()),
            ..Default::default()
        }),
        Operation::Road(RoadEdit {
            way: Some(2),
            width_m: Some(4.25),
            name: Some("Neuer Weg".into()),
            ..Default::default()
        }),
        Operation::Road(RoadEdit {
            edge: Some(EdgeId(999)),
            name: Some("nowhere".into()),
            ..Default::default()
        }),
    ]);
    let mut report = Report::new(&s);
    let plan = Plan::new(&s, &mut report);
    let mut g = graph();
    let surfaces = apply_road_edits(&plan, &mut g, Targets::All, &mut report);
    let e0 = &g.edges[0];
    assert_eq!(e0.class, RoadClass::Track);
    // The width followed the class (it was the old class's default).
    assert_eq!(e0.width_m, RoadClass::Track.default_width_m());
    assert_eq!(e0.oneway, Oneway::Backward);
    assert_eq!(e0.name, None);
    assert_eq!(surfaces.get(&EdgeId(10)), Some(&RoadSurface::Paving));
    assert_eq!(plan.edge_surfaces(), surfaces);
    assert_eq!(g.edges[2].width_m, 4.25);
    assert_eq!(g.edges[2].name.as_deref(), Some("Neuer Weg"));
    // Untouched.
    assert_eq!(g.edges[1], graph().edges[1]);
    // The missing edge is reported, not dropped.
    assert_eq!(report.missing_count(), 1);
    let miss = report.get(OpRef { patch: 0, op: 2 }).unwrap();
    assert!(
        matches!(&miss.outcome, Outcome::Missing(m) if m.contains("edge 999")),
        "{miss:?}"
    );
    assert!(miss.place.starts_with("test.ocwpatch:"), "{miss:?}");
    assert!(report.lines().iter().any(|l| l.contains("MISSING")));

    // EdgesOnly leaves way targets to the OSM stage.
    let mut report = Report::new(&s);
    let mut g = graph();
    apply_road_edits(&plan, &mut g, Targets::EdgesOnly, &mut report);
    assert_eq!(g.edges[2].width_m, RoadClass::Residential.default_width_m());
    assert!(matches!(
        report.get(OpRef { patch: 0, op: 1 }).unwrap().outcome,
        Outcome::Unchecked(_)
    ));
}

#[test]
fn removed_and_closed_roads_leave_the_graph() {
    let s = stack(vec![
        Operation::RemoveRoad(RemoveRoad {
            edge: Some(EdgeId(11)),
            ..Default::default()
        }),
        Operation::Road(RoadEdit {
            way: Some(2),
            bike: Some(BikeAccess::Closed),
            ..Default::default()
        }),
        Operation::RemoveRoad(RemoveRoad {
            way: Some(77),
            ..Default::default()
        }),
    ]);
    let mut report = Report::new(&s);
    let plan = Plan::new(&s, &mut report);
    let mut g = graph();
    apply_road_edits(&plan, &mut g, Targets::All, &mut report);
    assert_eq!(g.edges.len(), 1);
    assert_eq!(g.edges[0].id, EdgeId(10));
    assert_eq!(g.vertices.len(), 2);
    assert_eq!(report.missing_count(), 1);
    assert!(matches!(
        &report.get(OpRef { patch: 0, op: 2 }).unwrap().outcome,
        Outcome::Missing(m) if m.contains("way 77")
    ));
}

fn point(id: &str, at: [f64; 2], h: Height, radius: f64) -> Operation {
    let (height_m, raise_m) = match h {
        Height::Absolute(v) => (Some(v), None),
        Height::Raise(v) => (None, Some(v)),
    };
    Operation::HeightPoint(HeightPoint {
        id: id.into(),
        at: Some(latlon(at)),
        height_m,
        raise_m,
        radius_m: Some(radius),
        ..Default::default()
    })
}

fn heights_after(ops: Vec<Operation>) -> (RoadGraph, Report) {
    let s = stack(ops);
    let mut report = Report::new(&s);
    let plan = Plan::new(&s, &mut report);
    let mut g = graph();
    apply_height_points(&plan, &mut g, &proj(), &mut report);
    (g, report)
}

fn near(a: Option<f64>, b: f64, tol: f64) -> bool {
    a.is_some_and(|a| (a - b).abs() < tol)
}

#[test]
fn height_point_moves_the_profile_smoothly() {
    // 3 m beside the road, 250 m along e0.
    let (g, report) = heights_after(vec![point(
        "spike",
        [250.0, 3.0],
        Height::Absolute(90.0),
        100.0,
    )]);
    assert_eq!(report.missing_count(), 0, "{:?}", report.lines());
    let e0 = &g.edges[0];
    assert!(
        near(e0.height_at(250.0), 90.0, 1e-3),
        "{:?}",
        e0.height_at(250.0)
    );
    // Halfway out the correction is half; beyond the radius none.
    assert!(near(e0.height_at(200.0), 95.0, 1e-3));
    assert_eq!(e0.height_at(140.0), Some(100.0));
    assert_eq!(e0.height_at(360.0), Some(100.0));
    assert_eq!(g.edges[1], graph().edges[1]);
    // Smoothly: no step over 1 m per 5 m sample.
    for w in e0.profile.windows(2) {
        assert!((w[1] - w[0]).abs() < 1.0);
    }
    assert!(matches!(
        &report.get(OpRef { patch: 0, op: 0 }).unwrap().outcome,
        Outcome::Applied(m) if m.contains("edge 10")
    ));
}

#[test]
fn height_point_near_a_junction_keeps_it_continuous() {
    // 20 m before the junction v1, raised by 6 m over 100 m.
    let (g, _) = heights_after(vec![point("ramp", [480.0, 0.0], Height::Raise(6.0), 100.0)]);
    let e0 = &g.edges[0];
    assert!(near(e0.height_at(480.0), 106.0, 1e-3));
    let at_v1 = [
        *e0.profile.last().unwrap(),
        g.edges[1].profile[0],
        g.edges[2].profile[0],
    ];
    assert!(at_v1[0] > 103.0, "{at_v1:?}");
    for h in at_v1 {
        assert!((h - at_v1[0]).abs() < 1e-9, "{at_v1:?}");
    }
    // It fades out 80 m beyond the junction on both other roads.
    assert_eq!(g.edges[1].height_at(80.0), Some(100.0));
    assert_eq!(g.edges[2].height_at(80.0), Some(100.0));
    assert!(g.edges[2].height_at(40.0).unwrap() > 100.0);
}

#[test]
fn overlapping_height_points_are_solved_together() {
    let (g, report) = heights_after(vec![
        point("a", [200.0, 0.0], Height::Absolute(104.0), 150.0),
        point("b", [260.0, 0.0], Height::Absolute(98.0), 150.0),
    ]);
    let e0 = &g.edges[0];
    assert!(
        near(e0.height_at(200.0), 104.0, 1e-2),
        "{:?}",
        report.lines()
    );
    assert!(
        near(e0.height_at(260.0), 98.0, 1e-2),
        "{:?}",
        report.lines()
    );
}

#[test]
fn height_points_far_from_roads_or_on_unknown_nodes_are_reported() {
    let s = stack(vec![
        point("far", [250.0, 60.0], Height::Raise(1.0), 50.0),
        Operation::HeightPoint(HeightPoint {
            id: "node".into(),
            node: Some(5),
            raise_m: Some(1.0),
            ..Default::default()
        }),
        Operation::HeightPoint(HeightPoint {
            id: "gone".into(),
            node: Some(6),
            raise_m: Some(1.0),
            ..Default::default()
        }),
        // Right on e0, but restricted to e2.
        Operation::HeightPoint(HeightPoint {
            id: "wrong-edge".into(),
            at: Some(latlon([100.0, 0.0])),
            edge: Some(EdgeId(12)),
            raise_m: Some(1.0),
            ..Default::default()
        }),
    ]);
    let mut report = Report::new(&s);
    let mut plan = Plan::new(&s, &mut report);
    let mut g = graph();
    // Without the sources a node point is unchecked …
    apply_height_points(&plan, &mut g, &proj(), &mut report);
    assert!(matches!(
        report.get(OpRef { patch: 0, op: 1 }).unwrap().outcome,
        Outcome::Unchecked(_)
    ));
    // … with them it is placed (node 5 at x 700 on e1), or missing.
    let mut report = Report::new(&s);
    let node5 = latlon([700.0, 0.0]);
    plan.resolve_nodes(|n| (n == 5).then_some((node5[0], node5[1])), &mut report);
    let mut g = graph();
    apply_height_points(&plan, &mut g, &proj(), &mut report);
    assert!(near(g.edges[1].height_at(200.0), 101.0, 1e-3));
    let outcome = |op| report.get(OpRef { patch: 0, op }).unwrap().outcome.clone();
    assert!(matches!(outcome(0), Outcome::Missing(m) if m.contains("no road")));
    assert!(matches!(outcome(1), Outcome::Applied(_)));
    assert!(matches!(outcome(2), Outcome::Missing(m) if m.contains("node 6")));
    assert!(matches!(outcome(3), Outcome::Missing(m) if m.contains("edge 12")));
    assert_eq!(g.edges[0], graph().edges[0]);
}

#[test]
fn a_later_patch_moves_and_removes_earlier_points() {
    let base = {
        let mut p = WorldPatch::new("europe/testland");
        p.ops = vec![
            point("a", [100.0, 0.0], Height::Absolute(110.0), 50.0),
            point("b", [400.0, 0.0], Height::Absolute(90.0), 50.0),
        ];
        LoadedPatch::parse("official.ocwpatch", &p.to_toml()).unwrap()
    };
    let user = {
        let mut p = WorldPatch::new("europe/testland");
        p.ops = vec![
            Operation::MoveHeightPoint(MoveHeightPoint {
                id: "a".into(),
                at: Some(latlon([200.0, 0.0])),
                ..Default::default()
            }),
            Operation::RemoveHeightPoint(RemoveHeightPoint {
                id: "b".into(),
                ..Default::default()
            }),
            Operation::RemoveHeightPoint(RemoveHeightPoint {
                id: "c".into(),
                ..Default::default()
            }),
            point("a", [300.0, 0.0], Height::Raise(1.0), 50.0),
        ];
        LoadedPatch::parse("mine.ocwpatch", &p.to_toml()).unwrap()
    };
    let s = vec![base, user];
    let mut report = Report::new(&s);
    let plan = Plan::new(&s, &mut report);
    assert_eq!(plan.points.len(), 1);
    assert_eq!(plan.points[0].ops.len(), 2);
    let mut g = graph();
    apply_height_points(&plan, &mut g, &proj(), &mut report);
    let e0 = &g.edges[0];
    assert_eq!(e0.height_at(100.0), Some(100.0));
    assert!(near(e0.height_at(200.0), 110.0, 1e-3));
    assert_eq!(e0.height_at(400.0), Some(100.0));
    let outcome = |patch, op| report.get(OpRef { patch, op }).unwrap().outcome.clone();
    assert!(matches!(outcome(0, 0), Outcome::Applied(_)));
    assert!(matches!(outcome(0, 1), Outcome::Applied(m) if m.contains("removed")));
    assert!(matches!(outcome(1, 0), Outcome::Applied(_)));
    assert!(matches!(outcome(1, 2), Outcome::Missing(m) if m.contains("`c`")));
    assert!(matches!(outcome(1, 3), Outcome::Missing(m) if m.contains("already defined")));
    assert_eq!(report.missing_count(), 2);
}

#[test]
fn the_same_patch_gives_the_same_graph() {
    let ops = vec![
        point("a", [200.0, 0.0], Height::Absolute(104.0), 150.0),
        point("b", [520.0, 30.0], Height::Raise(-3.0), 120.0),
    ];
    let (a, _) = heights_after(ops.clone());
    let (b, _) = heights_after(ops);
    assert_eq!(a.fingerprint(), b.fingerprint());
    assert_eq!(a, b);
}

#[test]
fn height_points_between_samples_are_met() {
    // 252.5 m and 263.7 m along e0: between the 5 m profile samples.
    let (g, report) = heights_after(vec![
        point("a", [252.5, 0.0], Height::Absolute(103.0), 40.0),
        point("b", [263.7, 0.0], Height::Raise(-1.0), 40.0),
    ]);
    let e0 = &g.edges[0];
    assert!(
        near(e0.height_at(252.5), 103.0, 1e-4),
        "{:?}",
        report.lines()
    );
    assert!(
        near(e0.height_at(263.7), 99.0, 1e-4),
        "{:?}",
        report.lines()
    );
}

#[test]
fn every_broken_operation_is_listed_for_an_editor() {
    let mut p = WorldPatch::new("europe/testland");
    assert_eq!(p.header_problem(), None);
    p.ops = vec![
        Operation::Road(RoadEdit {
            edge: Some(EdgeId(10)),
            surface: Some("gravel".into()),
            ..Default::default()
        }),
        Operation::Road(RoadEdit {
            edge: Some(EdgeId(10)),
            width_m: Some(55.0),
            ..Default::default()
        }),
        Operation::RemoveRoad(RemoveRoad::default()),
        point("a", [250.0, 0.0], Height::Raise(1.0), 50.0),
        point("a", [260.0, 0.0], Height::Raise(1.0), 50.0),
    ];
    let problems = p.op_problems();
    assert_eq!(
        problems.iter().map(|(i, _)| *i).collect::<Vec<_>>(),
        vec![1, 2, 4]
    );
    assert!(problems[0].1.contains("1 – 30 m"));
    assert!(problems[2].1.contains("defined twice"));
    // `parse` refuses the first of them, with the same words.
    let e = WorldPatch::parse(&p.to_toml()).unwrap_err();
    assert!(e.message.starts_with("[[op]] 2 (road)"), "{e}");
    p.package = " ".into();
    assert!(p.header_problem().unwrap().contains("package"));
}

#[test]
fn the_preview_applies_a_stack_to_a_package_graph() {
    let s = stack(vec![
        Operation::Road(RoadEdit {
            edge: Some(EdgeId(11)),
            name: Some("Neuer Weg".into()),
            ..Default::default()
        }),
        Operation::RemoveRoad(RemoveRoad {
            way: Some(2),
            ..Default::default()
        }),
        Operation::RemoveRoad(RemoveRoad {
            edge: Some(EdgeId(99)),
            ..Default::default()
        }),
        point("dip", [250.0, 0.0], Height::Raise(-2.0), 60.0),
    ]);
    let (report, g) = preview(&graph(), (47.0, 9.5), &s);
    assert_eq!(report.missing_count(), 1, "{:?}", report.lines());
    assert!(matches!(
        report.get(OpRef { patch: 0, op: 2 }).unwrap().outcome,
        Outcome::Missing(_)
    ));
    assert_eq!(g.edges.len(), 2, "way 2 is gone");
    assert_eq!(g.edges[1].name.as_deref(), Some("Neuer Weg"));
    assert!(near(g.edges[0].height_at(250.0), 98.0, 1e-6));
    // The package's graph itself is unchanged.
    assert_eq!(graph().edges.len(), 3);
}
