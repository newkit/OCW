//! A test world with every road shape for the road test (used by the
//! tests of `ocw-world` and `world-builder`).

use super::*;
use crate::graph::{Edge, EdgeId, Oneway, VertexId};
use crate::junction::section::{JunctionIndex, StoredPatch, StoredPoint};
use crate::package::{RoadStrip, Tile};
use crate::vegetation::{PlantedTree, TreeSource};
use std::collections::BTreeMap;

pub const FLAT_Y: f32 = 500.0;

fn strip(class: RoadClass, width: f32, pts: &[[f32; 3]]) -> RoadStrip {
    RoadStrip {
        class,
        width_m: width,
        points: pts.to_vec(),
        surface: crate::RoadSurface::of_class(class),
        dress: crate::RoadDress::of_class(class, width),
        lit: Default::default(),
        condition: Default::default(),
    }
}

fn edge(
    id: u64,
    from: u32,
    to: u32,
    pts: &[[f64; 2]],
    class: RoadClass,
    width: f64,
    y: f64,
) -> Edge {
    let len: f64 = pts
        .windows(2)
        .map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]))
        .sum();
    let n = (len / crate::graph::PROFILE_STEP_M).ceil() as usize + 1;
    Edge {
        id: EdgeId(id),
        from: VertexId(from),
        to: VertexId(to),
        points: pts.to_vec(),
        class,
        name: None,
        width_m: width,
        bridge: false,
        tunnel: false,
        dead_end_m: None,
        boundary: false,
        oneway: Oneway::No,
        osm_way: id as i64,
        profile: vec![y; n],
    }
}

fn flat_tile(key: TileKey, roads: Vec<RoadStrip>) -> Tile {
    let n = 11u32;
    Tile {
        key,
        heights: vec![FLAT_Y; (n * n) as usize],
        roads,
    }
}

/// A flat 2 × 1 km world of forest with every road shape: a primary road
/// along z = 300 with a junction at x = 400 (a side road south, a stored
/// junction patch), a hairpin, a roundabout ring, a bridge over the
/// valley floor, a forest track, a path, and a residential road just
/// across the tile border at x = 1000 whose strips live in the next tile
/// only; mapped trees and a tree row on and beside the roads.
pub fn world() -> WorldPackage {
    use RoadClass::*;
    let k0 = TileKey { x: 0, z: 0 };
    let k1 = TileKey { x: 1, z: 0 };
    let y = FLAT_Y + 0.3;
    // The hairpin (a U-turn of 12 m radius at x = 700).
    let hairpin: Vec<[f32; 3]> = (0..=12)
        .map(|i| {
            let a = std::f32::consts::PI * (i as f32 / 12.0) - std::f32::consts::FRAC_PI_2;
            [700.0 + 12.0 * a.cos(), y, 600.0 + 12.0 * a.sin()]
        })
        .collect();
    let hairpin = [vec![[600.0, y, 588.0]], hairpin, vec![[600.0, y, 612.0]]].concat();
    // The roundabout (radius 15 m round (200, 700)).
    let ring: Vec<[f32; 3]> = (0..=24)
        .map(|i| {
            let a = std::f32::consts::TAU * i as f32 / 24.0;
            [200.0 + 15.0 * a.cos(), y, 700.0 + 15.0 * a.sin()]
        })
        .collect();
    let roads0 = vec![
        strip(Primary, 7.5, &[[0.0, y, 300.0], [1000.0, y, 300.0]]),
        strip(Residential, 5.0, &[[400.0, y, 300.0], [400.0, y, 0.0]]),
        strip(Tertiary, 6.0, &hairpin),
        strip(Unclassified, 6.0, &ring),
        // The bridge: 12 m over the valley floor.
        strip(
            Secondary,
            6.5,
            &[[100.0, y + 12.0, 900.0], [500.0, y + 12.0, 900.0]],
        ),
        strip(Track, 3.0, &[[800.0, y, 0.0], [800.0, y, 1000.0]]),
        strip(Path, 1.8, &[[50.0, y, 0.0], [50.0, y, 1000.0]]),
    ];
    // Just across the border: its strips are tile (1, 0)'s.
    let roads1 = vec![strip(
        Residential,
        5.0,
        &[[1002.0, y, 0.0], [1002.0, y, 1000.0]],
    )];
    let tiles = BTreeMap::from([(k0, flat_tile(k0, roads0)), (k1, flat_tile(k1, roads1))]);
    let yy = f64::from(y);
    let graph = RoadGraph {
        vertices: vec![
            [0.0, 300.0],
            [400.0, 300.0],
            [1000.0, 300.0],
            [400.0, 0.0],
            [1002.0, 0.0],
            [1002.0, 1000.0],
        ],
        edges: vec![
            edge(1, 0, 1, &[[0.0, 300.0], [400.0, 300.0]], Primary, 7.5, yy),
            edge(
                2,
                1,
                2,
                &[[400.0, 300.0], [1000.0, 300.0]],
                Primary,
                7.5,
                yy,
            ),
            edge(
                3,
                1,
                3,
                &[[400.0, 300.0], [400.0, 0.0]],
                Residential,
                5.0,
                yy,
            ),
            edge(
                4,
                4,
                5,
                &[[1002.0, 0.0], [1002.0, 1000.0]],
                Residential,
                5.0,
                yy,
            ),
        ],
        boundary_vertices: Default::default(),
    };
    // The junction's surface: a square of 10 m round (400, 300) with the
    // corners rounded off the arms (the stored outline of JNCT).
    let outline: Vec<StoredPoint> = [[-10.0, -10.0], [10.0, -10.0], [10.0, 10.0], [-10.0, 10.0]]
        .iter()
        .map(|d| StoredPoint { d: *d, mouth: None })
        .collect();
    let index = JunctionIndex {
        flags: 0,
        fingerprint: 0,
        vertex_count: graph.vertices.len() as u32,
        rules: vec![(
            1,
            StoredRule::Patch(StoredPatch {
                arms: Vec::new(),
                outline,
                members: Vec::new(),
                inner: Vec::new(),
                run_on: Vec::new(),
            }),
        )],
    };
    let lu = |_: usize| crate::landuse::LandClass::Forest;
    let landuse = crate::landuse::LandUse {
        cells: 100,
        tiles: [k0, k1]
            .into_iter()
            .map(|k| {
                (
                    k,
                    crate::landuse::LandUseTile {
                        classes: (0..100 * 100).map(lu).collect(),
                        water: Vec::new(),
                    },
                )
            })
            .collect(),
    };
    let tree = |x: f32, z: f32, source| PlantedTree {
        pos: [x, z],
        species: Species::Lime,
        height_m: 0.0,
        source,
    };
    let mut pkg = WorldPackage {
        name: "Every road".into(),
        origin: (47.14, 9.52),
        tile_size_m: 1000.0,
        samples: 11,
        tiles,
        graph: Some(Arc::new(graph)),
        landuse: Some(landuse),
        buildings: None,
        bridges: None,
        routes: Vec::new(),
        region: None,
        time_zone: None,
        look: Default::default(),
        obstacles: None,
        places: None,
        junctions: Some(Arc::new(JunctionData {
            index: Some(Arc::new(index)),
            strip_edges: None,
            mini_roundabouts: None,
        })),
        traffic: None,
        aero: None,
    };
    pkg.look.vegetation = Some(crate::vegetation::Vegetation {
        cells: 100,
        sources: Vec::new(),
        tiles: BTreeMap::from([(
            k0,
            crate::vegetation::VegetationTile {
                crops: Vec::new(),
                rows: Vec::new(),
                trees: vec![
                    // On the primary road's lane.
                    tree(250.0, 301.0, TreeSource::Tree),
                    // In the junction's corner (off both strips).
                    tree(408.0, 308.5, TreeSource::Tree),
                    // On the roundabout's ring, on the hairpin, on the track.
                    tree(215.0, 700.0, TreeSource::Tree),
                    tree(712.0, 600.0, TreeSource::Tree),
                    tree(800.5, 500.0, TreeSource::Tree),
                    // A tree row across the path, and one beside the road
                    // near the border, on the next tile's road.
                    tree(50.0, 450.0, TreeSource::TreeRow),
                    tree(998.5, 600.0, TreeSource::TreeRow),
                    // Well off every road: stays.
                    tree(600.0, 450.0, TreeSource::Tree),
                    // Under the bridge, short enough (a lime of 16 m under a
                    // deck 12 m up does not fit: left out).
                    tree(300.0, 900.0, TreeSource::Tree),
                ],
            },
        )]),
    });
    pkg
}
