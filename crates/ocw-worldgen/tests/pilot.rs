//! Integration test with the real pilot region (Liechtenstein). Needs
//! `scripts/fetch-pilot.sh`; skipped (with a message) without the data.

use std::collections::HashSet;
use std::path::PathBuf;

use ocw_worldgen::build::{Polygon, Region, build_road_graph};
use ocw_worldgen::dem::{Dem, HgtTile};
use ocw_worldgen::osm::OsmData;
use ocw_worldgen::profile::{add_profiles, limits, steepest};

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

#[test]
fn pilot_region_elevation_profiles() {
    let (Some(pbf), Some(poly), Some(hgt)) = (
        pilot("liechtenstein.osm.pbf"),
        pilot("liechtenstein.poly"),
        pilot("N47E009.hgt"),
    ) else {
        eprintln!("skipped: run scripts/fetch-pilot.sh for the pilot data");
        return;
    };
    let osm = OsmData::read_pbf(&pbf).unwrap();
    let region =
        Region::from_polygon(Polygon::parse_poly(&std::fs::read_to_string(poly).unwrap()).unwrap());
    let (mut g, _) = build_road_graph(&osm, &region);
    let dem = Dem {
        tiles: vec![HgtTile::read(&hgt).unwrap()],
    };
    let stats = add_profiles(&mut g, &dem, &region.projection());
    eprintln!(
        "elevation {:.0}-{:.0} m, steepest {:.1} %, {} suspicious, {} bridges/tunnels",
        stats.min_m,
        stats.max_m,
        stats.max_grade * 100.0,
        stats.suspicious.len(),
        stats.bridges_tunnels
    );
    // The Rhine valley is at ~430 m, Malbun at ~1600 m.
    assert!((400.0..500.0).contains(&stats.min_m), "{}", stats.min_m);
    assert!((1400.0..2200.0).contains(&stats.max_m), "{}", stats.max_m);
    assert_eq!(stats.missing_dem, 0);
    // Profiles everywhere; continuous at junctions.
    assert!(g.edges.iter().all(|e| e.profile.len() >= 2));
    let mut at_vertex: std::collections::HashMap<u32, f64> = Default::default();
    for e in &g.edges {
        for (v, h) in [
            (e.from.0, e.profile[0]),
            (e.to.0, *e.profile.last().unwrap()),
        ] {
            let prev = *at_vertex.entry(v).or_insert(h);
            assert!((prev - h).abs() < 1e-6, "junction {v}: {prev} vs {h}");
        }
    }
    // Every edge respects its class limit, also short connectors between
    // roads filtered on their own and bridges (before the limit across
    // junctions, 42 edges did not: up to 203 %, riders stalled on them).
    for (i, e) in g.edges.iter().enumerate() {
        let s = steepest(&e.profile, 1);
        assert!(
            s <= limits(e.class).0 + 0.01,
            "edge {i} ({:?}, {:.1} m): {:.1} %",
            e.class,
            e.length_m(),
            s * 100.0
        );
    }
}

#[test]
fn pilot_package_has_no_terrain_through_roads() {
    let (Some(pbf), Some(poly), Some(hgt)) = (
        pilot("liechtenstein.osm.pbf"),
        pilot("liechtenstein.poly"),
        pilot("N47E009.hgt"),
    ) else {
        eprintln!("skipped: run scripts/fetch-pilot.sh for the pilot data");
        return;
    };
    let osm = OsmData::read_pbf(&pbf).unwrap();
    let region =
        Region::from_polygon(Polygon::parse_poly(&std::fs::read_to_string(poly).unwrap()).unwrap());
    let proj = region.projection();
    let (mut g, _) = build_road_graph(&osm, &region);
    let dem = Dem {
        tiles: vec![HgtTile::read(&hgt).unwrap()],
    };
    add_profiles(&mut g, &dem, &proj);
    let base = |x: f64, z: f64| {
        let (lat, lon) = proj.unproject([x, z]);
        dem.elevation(lat, lon).unwrap_or(0.0)
    };
    let pkg =
        ocw_worldgen::package::build_package("pilot", region.center(), &g, base, 1000.0, 101, 0);
    let (worst, at, above, total) = pkg.road_pokes(2.0);
    eprintln!("terrain above roads: {above} of {total}, worst {worst:.2} m at {at:?}");
    // Remaining cases are only where two roads cross at different heights
    // without a bridge tag; they must be rare and small.
    assert!((above as f64) < total as f64 * 0.001, "{above} of {total}");
}
