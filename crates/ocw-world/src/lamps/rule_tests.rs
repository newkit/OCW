//! The rule of 2026-11-22: street lamps never stand in a road, only beside
//! it (spec: world look, *Street lamps*; 15H). Small worlds round a
//! junction, a second road alongside, a narrow street between houses, a
//! roundabout and a bridge.

use std::collections::BTreeMap;

use super::*;
use crate::bridges::{Bridge, BridgeMaterial, BridgeSource, BridgeStructure, Railing};
use crate::buildings::{Building, BuildingKind, Roof};
use crate::graph::RoadGraph;
use crate::junction::JunctionSet;
use crate::junction::fixtures::{self, Road};
use crate::obstacles::{ObstacleKind, Obstacles, PointObstacle, TileObstacles};
use crate::package::Tile;

const KEY: TileKey = TileKey { x: 0, z: 0 };
/// The fixtures lie round (0, 0); the worlds here move them into the
/// middle of tile (0, 0).
const MID: f64 = 500.0;

fn moved(mut g: RoadGraph) -> RoadGraph {
    for v in &mut g.vertices {
        *v = [v[0] + MID, v[1] + MID];
    }
    for e in &mut g.edges {
        for p in &mut e.points {
            *p = [p[0] + MID, p[1] + MID];
        }
    }
    g
}

/// A package of one 1 km tile holding `g`'s strips, all lit.
fn world(g: RoadGraph) -> WorldPackage {
    let roads = fixtures::strips(&g)
        .into_iter()
        .map(|(_, mut s)| {
            s.lit = RoadLit::Yes;
            s
        })
        .collect();
    WorldPackage {
        name: "lamps".into(),
        origin: (47.0, 9.0),
        tile_size_m: 1000.0,
        samples: 2,
        tiles: BTreeMap::from([(
            KEY,
            Tile {
                key: KEY,
                heights: vec![0.0; 4],
                roads,
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
        obstacles: None,
        places: None,
        junctions: None,
        traffic: None,
        aero: None,
    }
}

fn all_edges(pkg: &WorldPackage) -> Vec<u32> {
    (0..pkg.graph.as_ref().map_or(0, |g| g.edges.len()) as u32).collect()
}

/// What the rule did in a world: (regular lamps standing in a road,
/// placed lamps, placed lamps standing in a road, regular lamps).
struct Outcome {
    nominal: Vec<Lamp>,
    nominal_in_road: usize,
    placed: Vec<Lamp>,
    placed_in_road: usize,
}

fn outcome(pkg: &WorldPackage) -> Outcome {
    let g = pkg.graph.clone().unwrap();
    let js = JunctionSet::new(&g);
    let edges = all_edges(pkg);
    let space = LampSpace::new(pkg, KEY, &edges, Some(&js));
    let in_road = |l: &Lamp| {
        space
            .road_at(
                [f64::from(l.foot[0]), f64::from(l.foot[2])],
                f64::from(l.foot[1]),
                POLE_R_M,
            )
            .is_some()
    };
    let tile = &pkg.tiles[&KEY];
    let nominal: Vec<Lamp> = tile
        .roads
        .iter()
        .flat_map(|s| nominal_lamps(s, |_, _| false))
        .map(|n| n.lamp)
        .collect();
    let placed = tile_lamps(pkg, KEY, &space);
    // The same again: a pure function of the package.
    let again = {
        let js = JunctionSet::new(&g);
        let space = LampSpace::new(pkg, KEY, &edges, Some(&js));
        tile_lamps(pkg, KEY, &space)
    };
    assert_eq!(placed, again);
    Outcome {
        nominal_in_road: nominal.iter().filter(|l| in_road(l)).count(),
        placed_in_road: placed.iter().filter(|l| in_road(l)).count(),
        nominal,
        placed,
    }
}

/// Distance of a foot from the nearest edge centre line of the graph
/// (m), and that edge's half width.
fn nearest_road(g: &RoadGraph, foot: [f32; 3]) -> (f64, f64) {
    let p = [f64::from(foot[0]), f64::from(foot[2])];
    g.edges
        .iter()
        .flat_map(|e| {
            e.points.windows(2).map(move |w| {
                let (dx, dz) = (w[1][0] - w[0][0], w[1][1] - w[0][1]);
                let l2 = dx * dx + dz * dz;
                let t = (((p[0] - w[0][0]) * dx + (p[1] - w[0][1]) * dz) / l2).clamp(0.0, 1.0);
                (
                    (p[0] - w[0][0] - dx * t).hypot(p[1] - w[0][1] - dz * t),
                    e.width_m / 2.0,
                )
            })
        })
        .min_by(|a, b| (a.0 - a.1).total_cmp(&(b.0 - b.1)))
        .unwrap()
}

/// A T junction like [`fixtures::t_junction`], the main road's west arm
/// 91 m long: its last lamp (at 90 m, 36 m spacing) would stand on the
/// side road.
fn t_junction() -> RoadGraph {
    let v = [[0.0, 0.0], [-91.0, 0.0], [120.0, 0.0], [0.0, 120.0]];
    moved(fixtures::graph(
        &v,
        vec![
            Road::straight(&v, 1, 0, RoadClass::Primary, 7.5, |_| 0.0),
            Road::straight(&v, 0, 2, RoadClass::Primary, 7.5, |_| 0.0),
            Road::straight(&v, 0, 3, RoadClass::Residential, 5.0, |_| 0.0),
        ],
    ))
}

/// A T junction (main road 7.5 m along x, a side road 5 m to +z): the
/// regular row puts lamps on the side road's carriageway and on the
/// junction's surface; under the rule every foot stands beside the
/// roads, the lamps near the mouth moved along their road.
#[test]
fn a_lamp_next_to_a_junction_moves_off_the_side_road() {
    let pkg = world(t_junction());
    let o = outcome(&pkg);
    assert!(o.nominal_in_road > 0, "the test needs a lamp in the road");
    assert_eq!(o.placed_in_road, 0, "{:?}", o.placed);
    let g = pkg.graph.as_deref().unwrap();
    for l in &o.placed {
        let (d, half) = nearest_road(g, l.foot);
        assert!(d >= half + FOOT_CLEAR_M - 1e-3, "{l:?}: {d} {half}");
    }
    // Most lamps stay: they move a few metres, the junction does not
    // swallow them.
    assert!(
        o.placed.len() + 2 >= o.nominal.len(),
        "{} of {}",
        o.placed.len(),
        o.nominal.len()
    );
    // The heads still hang over their roads.
    for l in &o.placed {
        let h = l.head();
        let (d, half) = nearest_road(g, [h[0], h[1], h[2]]);
        assert!(d < half + 0.6, "{l:?}");
    }
}

/// Two residential roads (5 m) side by side, the second to the right of
/// the first (where its lamps stand, beyond a 1 m sidewalk): 6 m apart the
/// lamps step to the kerb between them; 5 m apart there is no room and
/// they are left out.
#[test]
fn a_second_road_alongside_moves_lamps_to_the_kerb_or_out() {
    let side_by_side = |gap: f64| {
        let v = [[0.0, 100.0], [0.0, -100.0], [gap, 100.0], [gap, -100.0]];
        moved(fixtures::graph(
            &v,
            vec![
                // Northwards (−z): right is +x, towards the second road.
                Road::straight(&v, 0, 1, RoadClass::Residential, 5.0, |_| 0.0),
                Road::straight(&v, 2, 3, RoadClass::Residential, 5.0, |_| 0.0),
            ],
        ))
    };
    let with_sidewalk = |gap: f64| {
        let mut pkg = world(side_by_side(gap));
        pkg.tiles.get_mut(&KEY).unwrap().roads[0].dress.sidewalk_m = [1.0, 1.0];
        pkg
    };
    let pkg = with_sidewalk(6.0);
    let o = outcome(&pkg);
    let first: Vec<&Lamp> = o.placed.iter().filter(|l| l.foot[0] < 504.0).collect();
    assert_eq!(o.placed_in_road, 0);
    assert!(o.nominal_in_road >= 6, "{}", o.nominal_in_road);
    assert_eq!(first.len(), 6, "{first:?}");
    for l in first {
        // At the kerb: 0.45 m outside the carriageway.
        assert!((l.foot[0] - (MID as f32 + 2.95)).abs() < 1e-3, "{l:?}");
    }
    let pkg = with_sidewalk(5.0);
    let o = outcome(&pkg);
    assert_eq!(o.placed_in_road, 0);
    assert!(o.placed.iter().all(|l| l.foot[0] > 504.0), "{:?}", o.placed);
}

fn house(x: [f32; 2], z: [f32; 2]) -> Building {
    Building {
        kind: BuildingKind::House,
        outline: vec![[x[0], z[0]], [x[1], z[0]], [x[1], z[1]], [x[0], z[1]]],
        holes: Vec::new(),
        base_y: 0.0,
        eave_y: 6.0,
        roof: Roof::Flat,
        tags: Default::default(),
        parts: Default::default(),
    }
}

/// A narrow street (5 m, sidewalks of 1.5 m) between rows of houses:
/// houses 1 m from the
/// carriageway leave room at the kerb; houses at the carriageway's edge
/// leave none – the lamps move along into the one gap between the houses
/// or are left out.
#[test]
fn a_narrow_street_between_buildings() {
    let street = || {
        let v = [[0.0, 100.0], [0.0, -100.0]];
        moved(fixtures::graph(
            &v,
            vec![Road::straight(
                &v,
                0,
                1,
                RoadClass::Residential,
                5.0,
                |_| 0.0,
            )],
        ))
    };
    let m = MID as f32;
    let mut pkg = world(street());
    pkg.tiles.get_mut(&KEY).unwrap().roads[0].dress.sidewalk_m = [1.5, 1.5];
    pkg.buildings = Some(BTreeMap::from([(
        KEY,
        vec![
            house([m + 3.5, m + 15.0], [m - 100.0, m + 100.0]),
            house([m - 15.0, m - 3.5], [m - 100.0, m + 100.0]),
        ],
    )]));
    let o = outcome(&pkg);
    assert_eq!(o.placed.len(), o.nominal.len());
    assert!(o.nominal.iter().all(|l| l.foot[0] > m + 3.0));
    assert!(
        o.placed
            .iter()
            .all(|l| (l.foot[0] - (m + 2.95)).abs() < 1e-3)
    );
    // Flush with the road, a 6 m gap between two houses at z = m + 4 … 10.
    pkg.buildings = Some(BTreeMap::from([(
        KEY,
        vec![
            house([m + 2.6, m + 15.0], [m - 100.0, m + 4.0]),
            house([m + 2.6, m + 15.0], [m + 10.0, m + 100.0]),
        ],
    )]));
    let o = outcome(&pkg);
    assert!(!o.placed.is_empty() && o.placed.len() < o.nominal.len());
    for l in &o.placed {
        assert!(
            l.foot[2] > m + 4.0 + 0.17 && l.foot[2] < m + 10.0 - 0.17,
            "{l:?}"
        );
    }
}

/// A small roundabout (ring 6 m wide round a 4 m island, four entries):
/// no lamp foot on the ring, the entries or the junction surfaces.
#[test]
fn a_roundabout_keeps_its_ring_free() {
    let mut pkg = world(moved(fixtures::small_roundabout()));
    // The entries (edges 0 – 3, from the ring outwards) as if they began
    // 14 m further up their roads: their first lamps stand 2 m from the
    // ring, on it.
    for s in &mut pkg.tiles.get_mut(&KEY).unwrap().roads[..4] {
        s.dress.start_m = 14.0;
    }
    let o = outcome(&pkg);
    assert!(o.nominal_in_road > 0, "the test needs a lamp in the road");
    assert_eq!(o.placed_in_road, 0, "{:?}", o.placed);
    // No pole on the ring's island either: it lies inside the ring's
    // carriageway (radius 7 m ± 3 m) …
    for l in &o.placed {
        let r = (f64::from(l.foot[0]) - MID).hypot(f64::from(l.foot[2]) - MID);
        assert!(
            !(4.0 - FOOT_CLEAR_M..=10.0 + FOOT_CLEAR_M).contains(&r),
            "{l:?} r {r}"
        );
    }
    assert!(!o.placed.is_empty());
}

/// A road over a bridge (deck 6 m + kerbs, 8 m above a road passing
/// under it): the lamps on the bridge stand on its parapet at the deck's
/// height, those of the road below are not under the deck, and a bridge of
/// the graph without a deck gets none.
#[test]
fn a_bridge_has_its_lamps_on_the_parapet() {
    // The road below's third lamp (at 80 m) would stand under the deck.
    let v = [[0.0, 100.0], [0.0, -100.0], [-83.0, 0.0], [100.0, 0.0]];
    let mut g = moved(fixtures::graph(
        &v,
        vec![
            Road::straight(&v, 0, 1, RoadClass::Residential, 6.0, |_| 8.0),
            Road::straight(&v, 2, 3, RoadClass::Residential, 5.0, |_| 0.0),
        ],
    ));
    g.edges[0].bridge = true;
    let m = MID as f32;
    let deck = Bridge {
        edge: g.edges[0].id,
        osm_way: 1,
        structure: BridgeStructure::Beam,
        material: BridgeMaterial::Concrete,
        source: BridgeSource::Tagged,
        railing: Railing::Parapet,
        width_m: 6.0,
        deck_m: 1.0,
        points: vec![[m, 8.0, m + 100.0], [m, 8.0, m - 100.0]],
        abutment_foot_y: [0.0, 0.0],
        piers: Vec::new(),
        arches: Vec::new(),
    };
    let mut pkg = world(g.clone());
    pkg.bridges = Some(BTreeMap::from([(KEY, vec![deck])]));
    let o = outcome(&pkg);
    assert_eq!(o.placed_in_road, 0);
    let on_bridge: Vec<&Lamp> = o.placed.iter().filter(|l| l.foot[1] > 4.0).collect();
    assert_eq!(on_bridge.len(), 6, "{on_bridge:?}");
    for l in &on_bridge {
        // Carriageway 3 m, kerb 0.6 m, the parapet's middle 0.175 m in.
        assert!((l.foot[0] - (m + 3.425)).abs() < 1e-3, "{l:?}");
    }
    // Above the road below too: the lower road does not block them.
    assert!(on_bridge.iter().any(|l| (l.foot[2] - m).abs() < 16.0));
    assert!(o.nominal.iter().any(|l| (l.foot[0] - m).abs() < 3.6));
    for l in o.placed.iter().filter(|l| l.foot[1] < 4.0) {
        assert!(
            (l.foot[0] - m).abs() > 3.6 + POLE_R_M as f32,
            "under the deck: {l:?}"
        );
    }
    // Without its deck the bridge has no room.
    let pkg = world(g);
    let o = outcome(&pkg);
    assert!(o.placed.iter().all(|l| l.foot[1] < 4.0), "{:?}", o.placed);
}

/// The point obstacles (OSM `barrier=*` nodes) stand in roads on purpose:
/// they do not change where lamps stand, and the lamps do not move them.
#[test]
fn point_obstacles_neither_block_nor_move() {
    let pkg = world(t_junction());
    let before = outcome(&pkg).placed;
    let mut with = pkg.clone();
    let mut obstacles = Obstacles::new(1000.0);
    let mut tile = TileObstacles::default();
    for l in &before {
        tile.points.push(PointObstacle {
            kind: ObstacleKind::Bollard,
            flags: 0,
            pos: [l.foot[0], l.foot[2]],
            angle: 0.0,
            width_m: 0.0,
            height_m: 1.0,
        });
    }
    obstacles.tiles.insert(KEY, tile.clone());
    with.obstacles = Some(obstacles);
    assert_eq!(outcome(&with).placed, before);
    assert_eq!(with.obstacles.unwrap().tiles[&KEY], tile);
}

/// A lamp near a tile border sees the roads of the next tile (from the
/// road graph): a road crossing just beyond the border, whose strip lies
/// in the other tile only, still moves it.
#[test]
fn the_roads_of_the_next_tile_count() {
    // The main road along z at x = 500 up to the tile border at z = 0 …
    // (its last lamp at 272 m, z = 0.5) …
    let v = [[500.0, 272.5], [500.0, 0.0], [400.0, -2.0], [600.0, -2.0]];
    let g = fixtures::graph(
        &v,
        vec![
            Road::straight(&v, 0, 1, RoadClass::Residential, 5.0, |_| 0.0),
            // … and a road along x 2 m beyond it, in tile (0, -1).
            Road::straight(&v, 2, 3, RoadClass::Residential, 5.0, |_| 0.0),
        ],
    );
    let mut pkg = world(g);
    // Only the first strip is this tile's.
    pkg.tiles.get_mut(&KEY).unwrap().roads.truncate(1);
    let js = JunctionSet::new(pkg.graph.as_deref().unwrap());
    let alone = LampSpace::new(&pkg, KEY, &[], None);
    let whole = LampSpace::new(&pkg, KEY, &[0, 1], Some(&js));
    let near_border = |l: &Lamp| l.foot[2] < 0.8;
    let a = tile_lamps(&pkg, KEY, &alone);
    let w = tile_lamps(&pkg, KEY, &whole);
    assert!(a.iter().any(near_border), "{a:?}");
    for l in &w {
        assert!(
            whole
                .road_at([f64::from(l.foot[0]), f64::from(l.foot[2])], 0.0, POLE_R_M)
                .is_none(),
            "{l:?}"
        );
    }
    // Moved back along its road to 2 m (the other road's edge at 0.5 m).
    assert!(!w.iter().any(near_border), "{w:?}");
    assert_eq!(a.len(), w.len());
}
