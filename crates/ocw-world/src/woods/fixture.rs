//! A test world for the trunks off the road (used by the tests of
//! `ocw-world`, `ocw-sim` and `ocw-render`).

use crate::flora::Species;
use crate::landuse::{LandClass, LandUse, LandUseTile};
use crate::package::{RoadStrip, Tile, TileKey, WorldPackage};
use crate::vegetation::{FieldCrop, NO_ROWS, PlantedTree, Vegetation, VegetationTile};
use std::collections::BTreeMap;

/// The tile of [`wood_package`].
pub const KEY: TileKey = TileKey { x: 0, z: 0 };

/// A 1 km tile of forest on a slope at Vaduz (47.14 N, 9.52 E): a road
/// along x = 503 (its 3 m margin blocks the trees beside it), an orchard
/// with rows in the south-west 100 m and a mapped lime at (900, 900).
pub fn wood_package() -> WorldPackage {
    let n = 11u32;
    let heights = (0..n * n)
        .map(|i| 500.0 + (i % n) as f32 * 3.0 + (i / n) as f32 * 1.5)
        .collect();
    let class = crate::graph::RoadClass::Track;
    let tile = Tile {
        key: KEY,
        heights,
        roads: vec![RoadStrip {
            class,
            width_m: 3.0,
            points: vec![[503.0, 520.0, 0.0], [503.0, 520.0, 1000.0]],
            surface: crate::RoadSurface::of_class(class),
            dress: crate::RoadDress::of_class(class, 3.0),
            lit: Default::default(),
            condition: Default::default(),
        }],
    };
    let cells = 100usize;
    let mut crops = vec![0u8; cells * cells];
    let mut rows = vec![NO_ROWS; cells * cells];
    for r in 0..10 {
        for c in 0..10 {
            crops[r * cells + c] = FieldCrop::Orchard.code();
            rows[r * cells + c] = crate::vegetation::encode_direction(0.3);
        }
    }
    let mut pkg = WorldPackage {
        name: "Wood".into(),
        origin: (47.14, 9.52),
        tile_size_m: 1000.0,
        samples: n,
        tiles: BTreeMap::from([(KEY, tile)]),
        graph: None,
        landuse: Some(LandUse {
            cells: 200,
            tiles: BTreeMap::from([(
                KEY,
                LandUseTile {
                    classes: (0..200 * 200)
                        .map(|i| {
                            if i % 200 < 20 && i / 200 < 20 {
                                LandClass::Orchard
                            } else {
                                LandClass::Forest
                            }
                        })
                        .collect(),
                    water: Vec::new(),
                },
            )]),
        }),
        buildings: None,
        bridges: None,
        routes: Vec::new(),
        region: None,
        time_zone: None,
        look: Default::default(),
        obstacles: None,
        places: None,
        junctions: None,
    };
    pkg.look.vegetation = Some(Vegetation {
        cells: cells as u32,
        sources: Vec::new(),
        tiles: BTreeMap::from([(
            KEY,
            VegetationTile {
                crops,
                rows,
                trees: vec![PlantedTree {
                    pos: [900.0, 900.0],
                    species: Species::Lime,
                    height_m: 0.0,
                    source: crate::vegetation::TreeSource::Tree,
                }],
            },
        )]),
    });
    pkg
}
