//! Integration test with the real pilot region (Liechtenstein). Needs
//! `scripts/fetch-pilot.sh`; skipped (with a message) without the data.

use std::collections::HashSet;
use std::path::PathBuf;

use ocw_worldgen::build::{Polygon, Region, build_road_graph};
use ocw_worldgen::osm::OsmData;

fn pilot(file: &str) -> Option<PathBuf> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../data/pilot")
        .join(file);
    p.exists().then_some(p)
}

#[test]
fn pilot_region_road_graph() {
    let (Some(pbf), Some(poly)) = (pilot("liechtenstein.osm.pbf"), pilot("liechtenstein.poly"))
    else {
        eprintln!("skipped: run scripts/fetch-pilot.sh for the pilot data");
        return;
    };
    let osm = OsmData::read_pbf(&pbf).expect("read pbf");
    let polygon = Polygon::parse_poly(&std::fs::read_to_string(poly).unwrap()).unwrap();
    let region = Region::from_polygon(polygon);
    let (g, stats) = build_road_graph(&osm, &region);
    eprintln!("{stats:#?}");
    assert!(stats.rideable_ways > 1000, "{stats:?}");
    assert!((300.0..5000.0).contains(&stats.length_km), "{stats:?}");
    assert!(
        stats.dead_end_edges > 0 && stats.boundary_edges > 0,
        "{stats:?}"
    );
    // One connected network.
    assert_eq!(g.components().len(), 1);
    // Stable edge IDs are unique.
    let ids: HashSet<_> = g.edges.iter().map(|e| e.id).collect();
    assert_eq!(ids.len(), g.edges.len(), "duplicate edge IDs");
    // Rebuilding gives the same IDs.
    let (again, _) = build_road_graph(&osm, &region);
    let again_ids: HashSet<_> = again.edges.iter().map(|e| e.id).collect();
    assert_eq!(ids, again_ids);
    // Everything lies within the region (±15 km of the centre).
    for e in &g.edges {
        for p in &e.points {
            assert!(p[0].abs() < 15_000.0 && p[1].abs() < 15_000.0, "{p:?}");
        }
    }
}
