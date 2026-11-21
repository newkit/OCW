//! What the navigation map draws from, in the home package's metres: the
//! loaded packages of the world set (land use and buildings are read from
//! them where they are, nothing is copied) and the joined road graph with
//! a grid index, so a map tile finds its roads without a scan.

use std::collections::HashMap;
use std::sync::Arc;

use ocw_world::graph::{RoadClass, RoadGraph};
use ocw_world::package::{TileKey, WorldPackage};
use ocw_world::world_set::Frame;

/// The road index's cell (m).
pub const ROAD_CELL_M: f64 = 250.0;

/// A loaded package and its place in the home metres.
#[derive(Debug, Clone)]
pub struct SourcePackage {
    pub pkg: Arc<WorldPackage>,
    pub frame: Frame,
}

/// A road of the navigation map (home metres).
#[derive(Debug, Clone, PartialEq)]
pub struct NaviRoad {
    pub class: RoadClass,
    pub name: Option<String>,
    pub tunnel: bool,
    pub points: Vec<[f64; 2]>,
    pub min: [f64; 2],
    pub max: [f64; 2],
}

/// The map's data (see the module docs).
#[derive(Debug, Clone, Default)]
pub struct MapSource {
    pub packages: Vec<SourcePackage>,
    roads: Vec<NaviRoad>,
    grid: HashMap<(i32, i32), Vec<u32>>,
}

fn cell_of(p: [f64; 2]) -> (i32, i32) {
    (
        (p[0] / ROAD_CELL_M).floor() as i32,
        (p[1] / ROAD_CELL_M).floor() as i32,
    )
}

fn box_of(points: &[[f64; 2]]) -> ([f64; 2], [f64; 2]) {
    points.iter().fold(
        ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]),
        |(lo, hi), p| {
            (
                [lo[0].min(p[0]), lo[1].min(p[1])],
                [hi[0].max(p[0]), hi[1].max(p[1])],
            )
        },
    )
}

impl MapSource {
    /// The map of `packages` with the roads of `graph` (home metres; the
    /// world set's joined graph). Without a graph, each package's own
    /// graph (or its tiles' road strips, version 1) is used.
    pub fn new(packages: Vec<SourcePackage>, graph: Option<&RoadGraph>) -> Self {
        let mut roads = Vec::new();
        let mut push = |class, name, tunnel, points: Vec<[f64; 2]>| {
            if points.len() >= 2 {
                let (min, max) = box_of(&points);
                roads.push(NaviRoad {
                    class,
                    name,
                    tunnel,
                    points,
                    min,
                    max,
                });
            }
        };
        match graph {
            Some(g) => {
                for e in &g.edges {
                    push(e.class, e.name.clone(), e.tunnel, e.points.clone());
                }
            }
            None => {
                for sp in &packages {
                    match &sp.pkg.graph {
                        Some(g) => {
                            for e in &g.edges {
                                let pts = e.points.iter().map(|p| sp.frame.apply(*p)).collect();
                                push(e.class, e.name.clone(), e.tunnel, pts);
                            }
                        }
                        None => {
                            for s in sp.pkg.tiles.values().flat_map(|t| &t.roads) {
                                let pts = s
                                    .points
                                    .iter()
                                    .map(|p| sp.frame.apply([f64::from(p[0]), f64::from(p[2])]))
                                    .collect();
                                push(s.class, None, false, pts);
                            }
                        }
                    }
                }
            }
        }
        let mut grid: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (i, r) in roads.iter().enumerate() {
            let (a, b) = (cell_of(r.min), cell_of(r.max));
            for cx in a.0..=b.0 {
                for cz in a.1..=b.1 {
                    grid.entry((cx, cz)).or_default().push(i as u32);
                }
            }
        }
        Self {
            packages,
            roads,
            grid,
        }
    }

    /// One package alone (its own metres).
    pub fn single(pkg: Arc<WorldPackage>) -> Self {
        let graph = pkg.graph.clone();
        Self::new(
            vec![SourcePackage {
                pkg,
                frame: Frame::IDENTITY,
            }],
            graph.as_deref(),
        )
    }

    pub fn roads(&self) -> &[NaviRoad] {
        &self.roads
    }

    /// The roads whose box meets `min`…`max`, each once, in index order.
    pub fn roads_in(&self, min: [f64; 2], max: [f64; 2]) -> Vec<&NaviRoad> {
        let (a, b) = (cell_of(min), cell_of(max));
        let mut ids: Vec<u32> = Vec::new();
        for cx in a.0..=b.0 {
            for cz in a.1..=b.1 {
                if let Some(v) = self.grid.get(&(cx, cz)) {
                    ids.extend_from_slice(v);
                }
            }
        }
        ids.sort_unstable();
        ids.dedup();
        ids.into_iter()
            .map(|i| &self.roads[i as usize])
            .filter(|r| {
                r.max[0] >= min[0] && r.min[0] <= max[0] && r.max[1] >= min[1] && r.min[1] <= max[1]
            })
            .collect()
    }

    /// The package tiles (with their package) that meet the home-metre
    /// box `min`…`max`.
    pub fn tiles_in(&self, min: [f64; 2], max: [f64; 2]) -> Vec<(&SourcePackage, TileKey)> {
        let mut out = Vec::new();
        for sp in &self.packages {
            let (a, b) = (sp.frame.invert(min), sp.frame.invert(max));
            let lo = [a[0].min(b[0]), a[1].min(b[1])];
            let hi = [a[0].max(b[0]), a[1].max(b[1])];
            let size = f64::from(sp.pkg.tile_size_m.max(1.0));
            let k0 = TileKey::containing(lo, size);
            let k1 = TileKey::containing(hi, size);
            for x in k0.x..=k1.x {
                for z in k0.z..=k1.z {
                    let k = TileKey { x, z };
                    if sp.pkg.tiles.contains_key(&k) {
                        out.push((sp, k));
                    }
                }
            }
        }
        out
    }

    /// Whether any package has a tile at home point `p`.
    pub fn has_tile_at(&self, p: [f64; 2]) -> bool {
        self.packages.iter().any(|sp| {
            let q = sp.frame.invert(p);
            let k = TileKey::containing(q, f64::from(sp.pkg.tile_size_m.max(1.0)));
            sp.pkg.tiles.contains_key(&k)
        })
    }
}
