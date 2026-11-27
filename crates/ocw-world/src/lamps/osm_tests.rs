//! Street lamps mapped in OSM (`highway=street_lamp` nodes, section
//! `OBST`): placed where they are mapped, moved out of a road by the rule
//! of 2026-11-22, the regular rows skipping the spots beside them, and the
//! same lamps for the server ([`package_lamps`]).

use std::collections::BTreeMap;

use super::*;
use crate::graph::RoadGraph;
use crate::junction::JunctionSet;
use crate::junction::fixtures::{self, Road};
use crate::obstacles::{Obstacles, TileObstacles};
use crate::package::Tile;

const KEY: TileKey = TileKey { x: 0, z: 0 };

/// A residential road (6 m) along x at z = 500 through tile (0, 0), and a
/// side road (5 m) from its middle to +z: a T junction at (500, 500).
fn roads() -> RoadGraph {
    let v = [
        [100.0, 500.0],
        [500.0, 500.0],
        [900.0, 500.0],
        [500.0, 800.0],
    ];
    fixtures::graph(
        &v,
        vec![
            Road::straight(&v, 0, 1, RoadClass::Residential, 6.0, |_| 0.0),
            Road::straight(&v, 1, 2, RoadClass::Residential, 6.0, |_| 0.0),
            Road::straight(&v, 1, 3, RoadClass::Residential, 5.0, |_| 0.0),
        ],
    )
}

fn lamp_at(x: f32, z: f32) -> PointObstacle {
    PointObstacle {
        kind: ObstacleKind::StreetLamp,
        flags: 0,
        pos: [x, z],
        angle: 0.0,
        width_m: 0.0,
        height_m: 0.0,
    }
}

/// The roads' package (all lit), with these mapped lamps.
fn world(osm: Vec<PointObstacle>) -> WorldPackage {
    let g = roads();
    let strips = fixtures::strips(&g)
        .into_iter()
        .map(|(_, mut s)| {
            s.lit = RoadLit::Yes;
            s
        })
        .collect();
    let obstacles = (!osm.is_empty()).then(|| {
        let mut o = Obstacles::new(1000.0);
        o.tiles.insert(
            KEY,
            TileObstacles {
                points: osm,
                ..Default::default()
            },
        );
        o
    });
    WorldPackage {
        name: "osm lamps".into(),
        origin: (47.0, 9.0),
        tile_size_m: 1000.0,
        samples: 2,
        tiles: BTreeMap::from([(
            KEY,
            Tile {
                key: KEY,
                heights: vec![0.0; 4],
                roads: strips,
            },
        )]),
        graph: Some(std::sync::Arc::new(g)),
        landuse: None,
        buildings: None,
        bridges: None,
        routes: Vec::new(),
        region: None,
        time_zone: None,
        look: Default::default(),
        obstacles,
        places: None,
        junctions: None,
        traffic: None,
        aero: None,
    }
}

fn placed(pkg: &WorldPackage) -> (Vec<Lamp>, LampCounts) {
    let spaces = PackageSpaces::new(pkg);
    tile_lamps_counted(pkg, KEY, &spaces.space(KEY))
}

fn in_road(pkg: &WorldPackage, l: &Lamp) -> Option<Blocked> {
    let spaces = PackageSpaces::new(pkg);
    let space = spaces.space(KEY);
    space
        .road_at(
            [f64::from(l.foot[0]), f64::from(l.foot[2])],
            f64::from(l.foot[1]),
            POLE_R_M,
        )
        .map(|(b, _)| b)
}

fn near(l: &Lamp, x: f32, z: f32, m: f32) -> bool {
    (l.foot[0] - x).hypot(l.foot[2] - z) < m
}

/// A lamp mapped beside the road stands where it is mapped, its arm over
/// the road; the regular lamps within half the spacing beside the same
/// road are left out (no double lamps), the others stay.
#[test]
fn a_mapped_lamp_stands_where_mapped_and_the_row_skips_it() {
    let plain = world(Vec::new());
    let (rows, c0) = placed(&plain);
    assert_eq!(c0.osm + c0.osm_left_out + c0.rows_skipped, 0);
    // The regular lamp nearest to x = 300 on the road's side (z < 500 for
    // the eastward road: its right is +z … whichever side it is).
    let reg = rows
        .iter()
        .filter(|l| (l.foot[2] - 500.0).abs() < 10.0)
        .min_by(|a, b| {
            (a.foot[0] - 300.0)
                .abs()
                .total_cmp(&(b.foot[0] - 300.0).abs())
        })
        .copied()
        .unwrap();
    let side = (reg.foot[2] - 500.0).signum();
    let mapped = lamp_at(reg.foot[0] + 3.0, 500.0 + side * 5.0);
    let pkg = world(vec![mapped]);
    let (lamps, c) = placed(&pkg);
    assert_eq!((c.osm, c.osm_moved, c.osm_left_out), (1, 0, 0));
    assert!(c.rows_skipped >= 1, "{c:?}");
    assert_eq!(c.rows + c.rows_skipped, c0.rows);
    let osm = lamps[0];
    assert_eq!([osm.foot[0], osm.foot[2]], mapped.pos);
    // Its arm points at the road (−side in z) and hangs over it.
    assert!((osm.toward[1] + side).abs() < 1e-4 && osm.toward[0].abs() < 1e-4);
    assert!((osm.head()[2] - 500.0).abs() < 4.0, "{osm:?}");
    assert_eq!(osm.height_m, lamp_height_m(RoadClass::Residential));
    // The regular lamp beside it is gone, no other lamp within 10 m.
    assert!(
        !lamps[1..]
            .iter()
            .any(|l| near(l, reg.foot[0], reg.foot[2], 0.1))
    );
    assert!(
        lamps[1..]
            .iter()
            .all(|l| !near(l, mapped.pos[0], mapped.pos[1], 10.0))
    );
    // Lamps farther along the road stay.
    assert!(
        lamps[1..]
            .iter()
            .any(|l| near(l, reg.foot[0] + 64.0, reg.foot[2], 1.0))
    );
    // OSM's height is used when it is plausible.
    let mut tall = mapped;
    tall.height_m = 10.0;
    assert_eq!(placed(&world(vec![tall])).0[0].height_m, 10.0);
}

/// The rule for mapped lamps: a node on the carriageway (an OSM lamp
/// mapped on the road) is moved to the nearest side – the side of the
/// node, at the kerb – and never stands in a road; a node on the centre
/// line goes to the right-hand side.
#[test]
fn a_mapped_lamp_on_the_carriageway_moves_to_the_nearest_side() {
    for (z, want_side) in [(501.5f32, 1.0f32), (498.0, -1.0), (500.0, 0.0)] {
        let pkg = world(vec![lamp_at(250.0, z)]);
        let (lamps, c) = placed(&pkg);
        assert_eq!((c.osm, c.osm_moved), (1, 1), "{z}");
        let l = lamps[0];
        assert_eq!(in_road(&pkg, &l), None, "{l:?}");
        // At the kerb: 3 m half width + 0.45 m.
        let off = l.foot[2] - 500.0;
        assert!((off.abs() - 3.45).abs() < 1e-3, "{z}: {l:?}");
        if want_side != 0.0 {
            assert_eq!(off.signum(), want_side, "{z}");
        }
        assert!((l.foot[0] - 250.0).abs() < 1e-3);
        // The arm over the road.
        assert!((l.head()[2] - 500.0).abs() < off.abs());
    }
}

/// A node on the junction's surface is moved off it; one far from every
/// road (a park) is left out.
#[test]
fn a_mapped_lamp_on_a_junction_moves_off_and_far_ones_are_left_out() {
    let pkg = world(vec![lamp_at(502.0, 503.0), lamp_at(300.0, 560.0)]);
    let (lamps, c) = placed(&pkg);
    assert_eq!((c.osm, c.osm_moved, c.osm_left_out), (1, 1, 1), "{lamps:?}");
    let l = lamps[0];
    assert_eq!(in_road(&pkg, &l), None, "{l:?}");
    assert!(near(&l, 502.0, 503.0, 10.0), "{l:?}");
    // Every lamp of the tile beside the roads.
    for l in &lamps {
        assert_eq!(in_road(&pkg, l), None, "{l:?}");
    }
}

/// Packages without mapped lamps (all of before) keep their lamps exactly:
/// the regular rows only; a section `OBST` with other points changes
/// nothing either.
#[test]
fn packages_without_mapped_lamps_are_unchanged() {
    let pkg = world(Vec::new());
    let g = pkg.graph.clone().unwrap();
    let js = JunctionSet::new(&g);
    let edges: Vec<u32> = (0..g.edges.len() as u32).collect();
    let space = LampSpace::new(&pkg, KEY, &edges, Some(&js));
    let town = town_at(&pkg);
    let rows: Vec<Lamp> = pkg.tiles[&KEY]
        .roads
        .iter()
        .flat_map(|s| strip_lamps(s, &town, &space))
        .collect();
    assert!(!rows.is_empty());
    assert_eq!(tile_lamps(&pkg, KEY, &space), rows);
    let mut post = lamp_at(250.0, 520.0);
    post.kind = ObstacleKind::Post;
    let with_post = world(vec![post]);
    assert_eq!(placed(&with_post).0, rows);
}

/// The server's lamps ([`package_lamps`]) are the lamps each tile places,
/// whatever order or thread: a pure function of the package.
#[test]
fn package_lamps_are_the_tiles_lamps() {
    let mut pkg = world(vec![lamp_at(250.0, 501.0), lamp_at(620.0, 494.0)]);
    // A second tile with the road's continuation.
    let mut t2 = pkg.tiles[&KEY].clone();
    t2.key = TileKey { x: 1, z: 0 };
    for s in &mut t2.roads {
        for p in &mut s.points {
            p[0] += 1000.0;
        }
    }
    pkg.tiles.insert(t2.key, t2);
    let all = package_lamps(&pkg);
    let spaces = PackageSpaces::new(&pkg);
    for key in pkg.tiles.keys() {
        assert_eq!(
            all.get(key).cloned().unwrap_or_default(),
            tile_lamps(&pkg, *key, &spaces.space(*key)),
            "{key:?}"
        );
    }
    assert_eq!(package_lamps(&pkg), all);
    let (_, c) = package_lamps_counted(&pkg);
    assert_eq!(c.osm, 2);
    // A streamed base (no tiles) has none.
    let mut base = pkg.clone();
    base.tiles.clear();
    assert!(package_lamps(&base).is_empty());
}
