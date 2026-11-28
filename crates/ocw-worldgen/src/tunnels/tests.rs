use std::collections::BTreeMap;

use ocw_world::graph::RoadClass;
use ocw_world::junction::fixtures::{self, Road};
use ocw_world::package::Tile;

use super::*;

const N: u32 = 201;
const SIZE: f32 = 1000.0;

/// One 1 km tile (5 m grid) of `ground` with a straight tunnel along x
/// from 300 to 700 at z 500, the road at 100 m; and, when `over`, a road
/// crossing it on the terrain at x 500.
fn package(ground: impl Fn(f64, f64) -> f64, over: bool) -> WorldPackage {
    let mut v = vec![[300.0, 500.0], [700.0, 500.0]];
    if over {
        v.extend([[500.0, 300.0], [500.0, 700.0]]);
    }
    let mut roads = vec![Road::straight(&v, 0, 1, RoadClass::Secondary, 7.0, |_| {
        100.0
    })];
    if over {
        roads.push(Road::straight(
            &v,
            2,
            3,
            RoadClass::Residential,
            5.0,
            |_| 103.0,
        ));
    }
    let mut g = fixtures::graph(&v, roads);
    g.edges[0].tunnel = true;
    let key = TileKey { x: 0, z: 0 };
    let sp = f64::from(SIZE) / f64::from(N - 1);
    let mut heights = Vec::new();
    for k in 0..N {
        for i in 0..N {
            heights.push(ground(f64::from(i) * sp, f64::from(k) * sp) as f32);
        }
    }
    let mut tiles = BTreeMap::new();
    tiles.insert(
        key,
        Tile {
            key,
            heights,
            roads: Vec::new(),
        },
    );
    WorldPackage {
        name: "Tunnels".into(),
        origin: (42.5, 1.5),
        tile_size_m: SIZE,
        samples: N,
        tiles,
        graph: Some(g.into()),
        landuse: None,
        buildings: None,
        bridges: None,
        routes: vec![],
        region: None,
        time_zone: None,
        look: Default::default(),
        obstacles: None,
        places: None,
        junctions: None,
        traffic: None,
        aero: None,
    }
}

fn none() -> TunnelTags {
    TunnelTags::new()
}

fn tunnel(pkg: &WorldPackage) -> Edge {
    pkg.graph.as_ref().expect("graph").edges[0].clone()
}

/// A ridge over x 380 … 620 rising steeply from just below the road.
fn ridge(height: f64) -> impl Fn(f64, f64) -> f64 {
    move |x: f64, _z: f64| 99.7 + ((120.0 - (x - 500.0).abs()) * 1.2).clamp(0.0, height)
}

#[test]
fn a_tunnel_through_a_hill_passes_the_check() {
    let mut pkg = package(ridge(60.0), false);
    assert!(check(&pkg, &none()).is_empty());
    let st = shape(&mut pkg, &none());
    assert_eq!(st.tunnels, 1);
    assert!(check(&pkg, &none()).is_empty());
    // Stored: open approaches, bored through the ridge.
    let secs = &pkg.look.tunnels.as_ref().expect("stored").edges[&tunnel(&pkg).id.0];
    assert_eq!(secs[0].kind, SectionKind::Open);
    // The portal is where the road meets the ridge (x ≈ 380, 80 m along),
    // not at the OSM end (x 300).
    let b = secs
        .iter()
        .find(|s| s.kind == SectionKind::Bored)
        .expect("bored");
    assert!((b.from - 80.0).abs() < 8.0, "{secs:?}");
}

#[test]
fn a_shallow_tunnel_gets_a_mound_and_then_passes() {
    // The DEM only 3.3 m over the road along the middle: the tube stood
    // 3 m out of it.
    let mut pkg = package(ridge(3.3), false);
    let before = check(&pkg, &none());
    assert!(before.iter().any(|e| e.3 > 3.0), "{before:?}");
    let st = shape(&mut pkg, &none());
    assert!(st.raised > 0 && st.max_raise_m > 4.0, "{st:?}");
    assert!(
        check(&pkg, &none()).is_empty(),
        "{:?}",
        check(&pkg, &none())
    );
    // The mound falls off into the terrain: 40 m beside the tunnel the
    // ground is as it was.
    let far = pkg.surface_height(500.0, 540.0).expect("ground");
    assert!((far - ridge(3.3)(500.0, 540.0)).abs() < 1e-3, "{far}");
    let m = measure(&pkg, &none());
    assert_eq!(m[0].exposed_m, 0.0);
}

#[test]
fn under_another_road_no_mound_but_an_underpass() {
    let mut pkg = package(ridge(3.3), true);
    // Before: the tube stands out of the crossing road's embankment too.
    assert!(!check(&pkg, &none()).is_empty());
    let st = shape(&mut pkg, &none());
    assert!(st.kept_for_roads > 0, "{st:?}");
    // The crossing road keeps its terrain beside our road.
    let h = pkg.surface_height(500.0, 450.0).expect("ground");
    assert!((h - ridge(3.3)(500.0, 450.0)).abs() < 1e-3, "{h}");
    let h = pkg.surface_height(500.0, 490.0).expect("ground");
    assert!((h - ridge(3.3)(500.0, 490.0)).abs() < 1e-3, "{h}");
    // Where it crosses (x 500, 200 m along) our road is open: an
    // underpass, the crossing road its roof; the rest stays bored.
    assert!(st.cut_open_m > 0.0, "{st:?}");
    let secs = &pkg.look.tunnels.as_ref().expect("stored").edges[&tunnel(&pkg).id.0];
    assert_eq!(section_kind_at(secs, 200.0), SectionKind::Open, "{secs:?}");
    assert_eq!(section_kind_at(secs, 150.0), SectionKind::Bored, "{secs:?}");
    assert!(
        check(&pkg, &none()).is_empty(),
        "{:?}",
        check(&pkg, &none())
    );
}

#[test]
fn carving_keeps_long_bored_pieces_only() {
    let secs = [
        Section {
            from: 0.0,
            to: 20.0,
            kind: SectionKind::Open,
        },
        Section {
            from: 20.0,
            to: 120.0,
            kind: SectionKind::Bored,
        },
    ];
    let c = carve_open(&secs, 25.0, 60.0);
    assert_eq!(c.len(), 2, "{c:?}");
    assert_eq!(
        c[0],
        Section {
            from: 0.0,
            to: 60.0,
            kind: SectionKind::Open
        }
    );
    assert_eq!(c[1].kind, SectionKind::Bored);
}

#[test]
fn a_gallery_on_a_slope_and_an_avalanche_protector() {
    // A slope rising to the right of the road (+z): the hill over the
    // walls on one side, open on the other.
    let slope = |_x: f64, z: f64| 100.0 + (z - 500.0) * 1.2;
    let mut pkg = package(slope, false);
    let (_, secs) = edge_sections(&pkg, &tunnel(&pkg), None);
    assert!(
        secs.iter()
            .all(|s| s.kind == SectionKind::Gallery { hill_right: true }),
        "{secs:?}"
    );
    shape(&mut pkg, &none());
    assert!(check(&pkg, &none()).is_empty());
    // The floor laid under the road: neither floating nor buried.
    let h = pkg.surface_height(500.0, 500.0).expect("ground");
    assert!((h - (100.0 - UNDER_ROAD_M)).abs() < 0.05, "{h}");
    // Tagged avalanche_protector on gentle ground: a gallery anyway, its
    // hill on the higher side (left, −z).
    let gentle = |_x: f64, z: f64| 100.0 - (z - 500.0) * 0.3;
    let pkg = package(gentle, false);
    let (_, secs) = edge_sections(&pkg, &tunnel(&pkg), Some("avalanche_protector"));
    assert_eq!(secs.len(), 1);
    assert_eq!(secs[0].kind, SectionKind::Gallery { hill_right: false });
}

#[test]
fn a_short_underpass_is_open_road_and_so_is_a_building_passage() {
    let flat = |_x: f64, _z: f64| 99.0;
    let mut pkg = package(flat, false);
    let (_, secs) = edge_sections(&pkg, &tunnel(&pkg), None);
    assert!(secs.iter().all(|s| s.kind == SectionKind::Open), "{secs:?}");
    shape(&mut pkg, &none());
    // Filled up under the road (1 m: within FILL_MAX_M).
    let h = pkg.surface_height(500.0, 500.0).expect("ground");
    assert!((h - (100.0 - UNDER_ROAD_M)).abs() < 0.05, "{h}");
    assert!(check(&pkg, &none()).is_empty());
    let deep = package(ridge(40.0), false);
    let (_, secs) = edge_sections(&deep, &tunnel(&deep), Some("building_passage"));
    assert_eq!(secs.len(), 1);
    assert_eq!(secs[0].kind, SectionKind::Open);
}

#[test]
fn a_road_high_over_a_valley_is_not_filled_into_a_dam() {
    let valley = |_x: f64, _z: f64| 80.0;
    let mut pkg = package(valley, false);
    let st = shape(&mut pkg, &none());
    assert!(st.left_floating > 0, "{st:?}");
    let h = pkg.surface_height(500.0, 500.0).expect("ground");
    assert!((h - 80.0).abs() < 1e-3, "{h}");
}

#[test]
fn shaping_is_deterministic_and_the_sections_round_trip() {
    let mut a = package(ridge(3.3), false);
    let mut b = package(ridge(3.3), false);
    assert_eq!(shape(&mut a, &none()), shape(&mut b, &none()));
    assert_eq!(a.to_bytes(), b.to_bytes());
    let back = WorldPackage::from_bytes(&a.to_bytes()).expect("read");
    assert_eq!(back.look.tunnels, a.look.tunnels);
    // The lint sees the same.
    let r = crate::lint::lint(&back, None);
    assert_eq!(r.count("tunnel.exposed"), 0);
    assert!(r.get("tunnel.bored").amount > 100.0);
}
