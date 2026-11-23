//! The test world of riding off the road (spec §14 Phase 13A), shared
//! by the tests of the simulation, the client and the apps: 2 × 1 tiles
//! with a road, a meadow, a lake, a wood, rock, a house, an obstacle, a
//! 40 % ramp and a 200 m cliff. Not used by the game itself.

use std::collections::BTreeMap;
use std::sync::Arc;

use ocw_world::RoadNetwork;
use ocw_world::graph::{Edge, EdgeId, Oneway, RoadClass, RoadGraph, VertexId};
use ocw_world::ground::GroundMap;
use ocw_world::landuse::{LandClass, LandUse, LandUseTile};
use ocw_world::obstacles::{ObstacleKind, Obstacles, PointObstacle, TileObstacles};
use ocw_world::{Tile, TileKey, WorldPackage};

/// The test world's height (m) at (x, z): flat around the road along
/// z = 500 and north of it; south of z = 400 a 40 % ramp up (x < 1000)
/// or a 200 m cliff at z = 300 (x ≥ 1000).
pub fn height(x: f64, z: f64) -> f64 {
    if z >= 400.0 {
        0.0
    } else if x < 1000.0 {
        0.4 * (400.0 - z)
    } else if z > 300.0 {
        0.0
    } else {
        200.0
    }
}

/// What covers the test world: a lake, a wood and rock north of the road,
/// meadow elsewhere.
pub fn class(x: f64, z: f64) -> LandClass {
    if (1200.0..1400.0).contains(&x) && (600.0..800.0).contains(&z) {
        LandClass::Water
    } else if (600.0..800.0).contains(&x) && z >= 600.0 {
        LandClass::Forest
    } else if (1600.0..1800.0).contains(&x) && (600.0..800.0).contains(&z) {
        LandClass::Rock
    } else {
        LandClass::Meadow
    }
}

/// The house of the test world (its footprint) and a tree.
pub const HOUSE: [[f32; 2]; 4] = [
    [300.0, 700.0],
    [312.0, 700.0],
    [312.0, 710.0],
    [300.0, 710.0],
];
/// The lake's water level (m).
pub const LAKE_LEVEL: f64 = 0.5;
pub const TREE: [f32; 2] = [500.0, 700.0];

/// The test world: 2 × 1 tiles of 1 km (10 m grid, 5 m land use), the
/// road along z = 500 from x = 0 to 2000 (two edges, 6 m wide), a lake, a
/// wood, rock, a house and a tree.
pub fn world() -> Arc<RoadNetwork> {
    network(package(), false)
}

/// [`world`] with its drawn trees as trunks off the road (spec 14 Phase
/// 13 Q10 (a)): the wood (x 600 … 800, z ≥ 600) is dense with them.
pub fn wooded_world() -> Arc<RoadNetwork> {
    network(package(), true)
}

/// The test world's package.
pub fn package() -> WorldPackage {
    let n = 101u32;
    let mut tiles = BTreeMap::new();
    let mut lu = BTreeMap::new();
    for tx in 0..2 {
        let key = TileKey { x: tx, z: 0 };
        let o = key.origin(1000.0);
        let mut heights = Vec::new();
        for k in 0..n {
            for i in 0..n {
                heights.push(height(o[0] + f64::from(i) * 10.0, o[1] + f64::from(k) * 10.0) as f32);
            }
        }
        tiles.insert(
            key,
            Tile {
                key,
                heights,
                roads: Vec::new(),
            },
        );
        let mut classes = Vec::new();
        for r in 0..200 {
            for c in 0..200 {
                classes.push(class(
                    o[0] + (f64::from(c) + 0.5) * 5.0,
                    o[1] + (f64::from(r) + 0.5) * 5.0,
                ));
            }
        }
        // The lake's surface 0.5 m above the ground, at its vertices and
        // the ring around them.
        let mut water = Vec::new();
        for k in 0..n {
            for i in 0..n {
                let (x, z) = (o[0] + f64::from(i) * 10.0, o[1] + f64::from(k) * 10.0);
                if (1190.0..=1410.0).contains(&x) && (590.0..=810.0).contains(&z) {
                    water.push((k * n + i, LAKE_LEVEL as f32));
                }
            }
        }
        lu.insert(key, LandUseTile { classes, water });
    }
    let house = ocw_world::buildings::Building {
        kind: ocw_world::buildings::BuildingKind::House,
        outline: HOUSE.to_vec(),
        holes: vec![],
        base_y: -0.5,
        eave_y: 6.0,
        roof: ocw_world::buildings::Roof::Flat,
        tags: Default::default(),
        parts: Default::default(),
    };
    let mut obst = Obstacles::new(1000.0);
    obst.tiles.insert(
        TileKey { x: 0, z: 0 },
        TileObstacles {
            points: vec![PointObstacle {
                kind: ObstacleKind::Block,
                flags: 0,
                pos: TREE,
                angle: 0.0,
                width_m: 0.0,
                height_m: 1.0,
            }],
            ..Default::default()
        },
    );
    WorldPackage {
        name: "offroad-test".into(),
        origin: (47.14, 9.52),
        tile_size_m: 1000.0,
        samples: n,
        tiles,
        graph: None,
        landuse: Some(LandUse {
            cells: 200,
            tiles: lu,
        }),
        buildings: Some(
            [(TileKey { x: 0, z: 0 }, vec![house])]
                .into_iter()
                .collect(),
        ),
        bridges: None,
        routes: Vec::new(),
        region: None,
        time_zone: None,
        look: Default::default(),
        obstacles: Some(obst),
        places: None,
        junctions: None,
    }
}

/// The road network of `pkg` (its obstacles, ground and, with `woods`,
/// its trees off the road).
pub fn network(pkg: WorldPackage, woods: bool) -> Arc<RoadNetwork> {
    let vertices = vec![[0.0, 500.0], [1000.0, 500.0], [2000.0, 500.0]];
    let edges = (0..2u32)
        .map(|k| Edge {
            id: EdgeId(900 + u64::from(k)),
            from: VertexId(k),
            to: VertexId(k + 1),
            points: vec![vertices[k as usize], vertices[k as usize + 1]],
            class: RoadClass::Tertiary,
            name: Some("Testweg".into()),
            width_m: 6.0,
            bridge: false,
            tunnel: false,
            dead_end_m: None,
            boundary: false,
            oneway: Oneway::No,
            osm_way: i64::from(k),
            profile: Vec::new(),
        })
        .collect();
    let graph = RoadGraph {
        vertices,
        edges,
        boundary_vertices: Default::default(),
    };
    Arc::new(
        RoadNetwork::new(graph)
            .with_obstacles(ocw_world::obstacles::WorldObstacles::from_package(&pkg).map(Arc::new))
            .with_ground(Some(Arc::new(GroundMap::from_package(&pkg))))
            .with_woods(woods.then(|| Arc::new(ocw_world::woods::Woods::new(Arc::new(pkg))))),
    )
}
