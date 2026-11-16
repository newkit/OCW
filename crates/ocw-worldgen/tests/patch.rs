//! World patches through the build pipeline (spec §14 Phase 16 E5 step 1,
//! 16G): a patch file applied to the hand-made mini region (the fixtures
//! of tests/e2e.rs) and a synthetic DEM, in the order `world-builder
//! package --patch` uses; the effect checked in the package, a rebuild
//! from the newer extract reports the operation whose target is gone, and
//! the same sources with the same patch give byte-identical packages.
//!
//! No network, GPU or pilot data needed (the pilot test at the end is
//! skipped without `scripts/fetch-pilot.sh`).

use std::path::{Path, PathBuf};

use ocw_world::graph::{EdgeId, Oneway, RoadClass, RoadGraph};
use ocw_world::patch::{LoadedPatch, OpRef, Outcome, Report};
use ocw_world::{RoadSurface, WorldPackage};
use ocw_worldgen::build::{Polygon, Region, build_road_graph_with};
use ocw_worldgen::dem::{Dem, HgtTile};
use ocw_worldgen::osm::OsmData;
use ocw_worldgen::patch::Patching;
use ocw_worldgen::profile::add_profiles;

/// Synthetic 1" DEM tile N46E009: a gentle slope.
fn dem() -> Dem {
    let n = 3601usize;
    let mut bytes = Vec::with_capacity(n * n * 2);
    for row in 0..n {
        let lat = 47.0 - row as f64 / (n - 1) as f64;
        for col in 0..n {
            let lon = 9.0 + col as f64 / (n - 1) as f64;
            let h = 500.0 + 2000.0 * (lat - 46.49) + 1500.0 * (lon - 9.49);
            bytes.extend((h.round() as i16).to_be_bytes());
        }
    }
    Dem::from_hgt(vec![HgtTile::from_bytes(46, 9, &bytes).unwrap()])
}

fn region() -> Region {
    Region::bbox(46.490, 9.485, 46.510, 9.515)
}

fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
}

/// The world builder's pipeline with a patch stack: OSM (patched) →
/// graph (edge edits before cleanup) → profiles → height points →
/// package bytes with the road surfaces.
fn build(osm_xml: &str, patches: &[&str], dem: &Dem) -> (RoadGraph, Vec<u8>, Report) {
    let stack: Vec<LoadedPatch> = patches
        .iter()
        .enumerate()
        .map(|(i, t)| LoadedPatch::parse(format!("p{i}.ocwpatch"), t).unwrap())
        .collect();
    let mut patching = Patching::new(&stack);
    let mut osm = OsmData::parse_osm_xml(osm_xml).unwrap();
    patching.on_osm(&mut osm);
    let region = region();
    let proj = region.projection();
    let (mut graph, _) = build_road_graph_with(&osm, &region, |g| patching.on_raw_graph(g));
    add_profiles(&mut graph, dem, &proj);
    patching.on_profiles(&mut graph, &proj);
    let base = |x: f64, z: f64| {
        let (lat, lon) = proj.unproject([x, z]);
        dem.elevation(lat, lon).unwrap_or(0.0)
    };
    let mut pkg =
        ocw_worldgen::package::build_package("Mini", region.center(), &graph, base, 1000.0, 51, 1);
    let way_surfaces = ocw_worldgen::filter::way_surfaces(&osm);
    ocw_worldgen::package::apply_road_surfaces(&mut pkg, &graph, |e| {
        patching
            .surface(e)
            .or_else(|| way_surfaces.get(&e.osm_way).copied())
            .unwrap_or_else(|| RoadSurface::of_class(e.class))
    });
    (graph, pkg.to_bytes(), patching.report)
}

fn edges_of_way(g: &RoadGraph, way: i64) -> Vec<usize> {
    (0..g.edges.len())
        .filter(|&i| g.edges[i].osm_way == way)
        .collect()
}

/// The eastern edge of the Hauptstrasse (3-4-5): its stable id survives
/// the update to mini-v2.
fn hauptstrasse_east(g: &RoadGraph) -> EdgeId {
    let e = edges_of_way(g, 100)
        .into_iter()
        .max_by(|&a, &b| g.edges[a].points[0][0].total_cmp(&g.edges[b].points[0][0]))
        .unwrap();
    g.edges[e].id
}

fn patch_text(east: EdgeId) -> String {
    format!(
        r#"# Hand-made test patch for the mini region (not real map data).
ocwpatch = 1
package = "mini"
title = "Mini fixes"
author = "Ada Rider"

[[op]]
op = "road"
way = 102
surface = "gravel"
name = "Kiesweg"
class = "track"

[[op]]
op = "road"
edge = "{east}"
surface = "paving_stones"
width_m = 9.0
oneway = "forward"

[[op]]
op = "remove_road"
way = 105
note = "the one-way short cut is closed"

[[op]]
op = "height_point"
id = "dip"
node = 2
raise_m = -4.0
radius_m = 120
"#,
        east = east.0
    )
}

/// Road height at the point of the roads nearest to `p`.
fn road_at(g: &RoadGraph, p: [f64; 2]) -> f64 {
    let mut best = (f64::INFINITY, 0.0);
    for e in &g.edges {
        let mut arc = 0.0;
        for w in e.points.windows(2) {
            let d = [w[1][0] - w[0][0], w[1][1] - w[0][1]];
            let l = d[0].hypot(d[1]);
            let t = (((p[0] - w[0][0]) * d[0] + (p[1] - w[0][1]) * d[1]) / (l * l)).clamp(0.0, 1.0);
            let dist = (w[0][0] + d[0] * t - p[0]).hypot(w[0][1] + d[1] * t - p[1]);
            if dist < best.0 {
                best = (dist, e.height_at(arc + l * t).unwrap());
            }
            arc += l;
        }
    }
    assert!(best.0 < 1.0, "no road at {p:?} ({} m)", best.0);
    best.1
}

#[test]
fn patch_changes_the_built_package() {
    let dem = dem();
    let v1 = fixture("mini-v1.osm");
    let (plain, plain_bytes, _) = build(&v1, &[], &dem);
    let east = hauptstrasse_east(&plain);
    let text = patch_text(east);
    let (g, bytes, report) = build(&v1, &[&text], &dem);
    assert_eq!(report.missing_count(), 0, "{:#?}", report.lines());
    assert_eq!(report.unchecked_count(), 0, "{:#?}", report.lines());
    assert_ne!(bytes, plain_bytes);

    // Way 102: class, name (OSM tags) and the surface in the package.
    for i in edges_of_way(&g, 102) {
        assert_eq!(g.edges[i].class, RoadClass::Track);
        assert_eq!(g.edges[i].name.as_deref(), Some("Kiesweg"));
    }
    // The edge target.
    let e = g.edges.iter().find(|e| e.id == east).unwrap();
    assert_eq!(e.width_m, 9.0);
    assert_eq!(e.oneway, Oneway::Forward);
    // The removed way is out of the ridable graph.
    assert!(edges_of_way(&plain, 105).len() == 1 && edges_of_way(&g, 105).is_empty());

    // The package has it all: the graph, the strips' surfaces and widths.
    let pkg = WorldPackage::from_bytes(&bytes).unwrap();
    let pg = pkg.graph.as_ref().expect("package graph");
    // (Profiles are stored as f32, so compare the attributes.)
    let attrs = |g: &RoadGraph| {
        g.edges
            .iter()
            .map(|e| (e.id, e.class, e.width_m, e.oneway, e.name.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(attrs(pg), attrs(&g));
    let strips: Vec<_> = pkg.tiles.values().flat_map(|t| t.roads.iter()).collect();
    assert!(
        strips
            .iter()
            .any(|s| s.class == RoadClass::Track && s.surface == RoadSurface::Gravel)
    );
    assert!(
        strips
            .iter()
            .any(|s| s.width_m == 9.0 && s.surface == RoadSurface::Paving)
    );

    // The height point at node 2 (46.5000, 9.4950): the road 4 m lower,
    // and the terrain conformed to it (§4.3 step 4).
    let proj = region().projection();
    let p = proj.project(46.5000, 9.4950);
    let (before, after) = (road_at(&plain, p), road_at(&g, p));
    assert!((before - 4.0 - after).abs() < 0.01, "{before} → {after}");
    let plain_pkg = WorldPackage::from_bytes(&plain_bytes).unwrap();
    let (t0, t1) = (
        plain_pkg.height(p[0], p[1]).unwrap(),
        pkg.height(p[0], p[1]).unwrap(),
    );
    assert!((t1 - after).abs() < 1.0, "terrain {t1} under road {after}");
    assert!(t0 - t1 > 3.0, "terrain {t0} → {t1}");
    // 200 m away along the road nothing changed.
    let far = proj.project(46.5000, 9.4924);
    assert!((road_at(&plain, far) - road_at(&g, far)).abs() < 1e-9);
    match &report.get(OpRef { patch: 0, op: 3 }).unwrap().outcome {
        Outcome::Applied(m) => assert!(m.contains("→"), "{m}"),
        o => panic!("{o:?}"),
    }
}

#[test]
fn same_sources_and_patch_give_identical_bytes() {
    let dem = dem();
    let v1 = fixture("mini-v1.osm");
    let (plain, _, _) = build(&v1, &[], &dem);
    let text = patch_text(hauptstrasse_east(&plain));
    let (_, a, _) = build(&v1, &[&text], &dem);
    let (_, b, _) = build(&v1, &[&text], &dem);
    assert_eq!(a, b);
    // Reading and writing the patch again changes nothing either.
    let again = ocw_world::patch::WorldPatch::parse(&text)
        .unwrap()
        .to_toml();
    let (_, c, _) = build(&v1, &[&again], &dem);
    assert_eq!(a, c);
}

#[test]
fn a_newer_extract_reports_the_gone_target() {
    let dem = dem();
    let v1 = fixture("mini-v1.osm");
    let (plain, _, _) = build(&v1, &[], &dem);
    let text = patch_text(hauptstrasse_east(&plain));
    // mini-v2: way 105 deleted, Hauptstrasse east still there.
    let (g, _, report) = build(&fixture("mini-v2.osm"), &[&text], &dem);
    assert_eq!(report.missing_count(), 1, "{:#?}", report.lines());
    let entry = report.get(OpRef { patch: 0, op: 2 }).unwrap();
    assert!(
        matches!(&entry.outcome, Outcome::Missing(m) if m.contains("way 105")),
        "{entry:?}"
    );
    assert_eq!(entry.place, "p0.ocwpatch:21");
    // The rest still applies.
    let e = g
        .edges
        .iter()
        .find(|e| e.id == hauptstrasse_east(&plain))
        .unwrap();
    assert_eq!(e.width_m, 9.0);
    assert!(
        report
            .lines()
            .last()
            .unwrap()
            .contains("1 missing (target gone, not applied)")
    );
}

/// The pilot data: `data/pilot`, or `$OCW_PILOT_DIR` (an agent checkout
/// on a build machine points it at the main checkout's data).
fn pilot(file: &str) -> Option<PathBuf> {
    let dir = std::env::var_os("OCW_PILOT_DIR").map_or_else(
        || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/pilot"),
        PathBuf::from,
    );
    let p = dir.join(file);
    p.exists().then_some(p)
}

/// The Liechtenstein pilot sources (graph level, no DEM: a fast check
/// that real OSM ways and stable edge ids are found and changed).
#[test]
fn patch_on_the_pilot_sources() {
    let (Some(pbf), Some(poly)) = (pilot("liechtenstein.osm.pbf"), pilot("liechtenstein.poly"))
    else {
        eprintln!("skipped: run scripts/fetch-pilot.sh for the pilot data");
        return;
    };
    let osm = OsmData::read_pbf(&pbf).unwrap();
    let region =
        Region::from_polygon(Polygon::parse_poly(&std::fs::read_to_string(poly).unwrap()).unwrap());
    let (plain, _) = build_road_graph_with(&osm, &region, |_| {});
    // The longest named way and the longest edge of another way.
    let way = plain
        .edges
        .iter()
        .filter(|e| e.name.is_some())
        .max_by(|a, b| a.length_m().total_cmp(&b.length_m()))
        .unwrap()
        .osm_way;
    let edge = plain
        .edges
        .iter()
        .filter(|e| e.osm_way != way && !e.boundary)
        .max_by(|a, b| a.length_m().total_cmp(&b.length_m()))
        .unwrap()
        .id;
    let text = format!(
        "ocwpatch = 1\npackage = \"europe/liechtenstein\"\n\n[[op]]\nop = \"road\"\nway = {way}\nname = \"Testweg\"\nsurface = \"gravel\"\n\n[[op]]\nop = \"road\"\nedge = \"{}\"\nwidth_m = 12.5\n\n[[op]]\nop = \"remove_road\"\nway = 1\n",
        edge.0
    );
    let stack = vec![LoadedPatch::parse("pilot.ocwpatch", &text).unwrap()];
    let mut patching = Patching::new(&stack);
    let mut osm = osm;
    patching.on_osm(&mut osm);
    let (g, _) = build_road_graph_with(&osm, &region, |g| patching.on_raw_graph(g));
    assert!(
        edges_of_way(&g, way)
            .iter()
            .all(|&i| g.edges[i].name.as_deref() == Some("Testweg"))
    );
    assert!(!edges_of_way(&g, way).is_empty());
    assert_eq!(g.edges.iter().find(|e| e.id == edge).unwrap().width_m, 12.5);
    // OSM way 1 is not a Liechtenstein road.
    assert_eq!(patching.report.missing_count(), 1);
    let way_ids = |g: &RoadGraph| {
        g.edges
            .iter()
            .map(|e| e.id)
            .collect::<std::collections::BTreeSet<_>>()
    };
    assert_eq!(way_ids(&g), way_ids(&plain));
}
