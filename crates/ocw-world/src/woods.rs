//! The trunks of the drawn trees away from the roads (spec 14 Phase 13,
//! Q10 (a), decided 2026-11-23): off the road every tree the renderer
//! draws is an obstacle. The package's section `OBST` stores the grid's
//! trees only within wild mode's reach of a road (60 m + 2 m); deeper in
//! a wood the trunks are made **on demand**, per [`WoodCell`] of 40 m
//! (5 × 5 candidates of the 8 m tree grid), from the very function the
//! renderer draws them with ([`crate::trees`]' placement, the species and
//! scale of [`crate::flora::GridTrees`], the trunk radius of
//! [`crate::flora::Species::trunk_radius_m`]) – deterministic and
//! wasm-safe, so the game server, the local servers and the clients'
//! prediction get the same trunks. The orchards' rows and the mapped
//! trees away from the roads come the same way
//! ([`crate::plantings::planted_points`]).
//!
//! **No doubles:** a candidate the section stores (its tree cell) and a
//! planted tree it stores (the same foot) are left out – those are the
//! road-side trees of [`crate::obstacles::WorldObstacles`].
//!
//! Shrubs, succulents and banana plants stay soft (no trunk). Packages
//! without land use (the provisional patches of the oldest packages) have
//! no trunks off the road; neither do neighbour packages (only the home
//! package's obstacles are loaded, as for `OBST`).
//!
//! [`Woods`] holds the package (an `Arc` shared with the renderer where
//! there is one; the browser swaps in its resident tiles as they stream);
//! the cells themselves are made, cached and dropped by the user
//! (`ocw_sim::collide::WorldCache`).

use std::sync::{Arc, RwLock};

use crate::flora::GridTrees;
use crate::package::{TileKey, WorldPackage};
use crate::trees::{TREE_CELL_M, cells_per_side, landuse_trees_in, tile_landuse};

/// Tree-grid candidates per side of a [`WoodCell`].
pub const CELL_CANDIDATES: usize = 5;
/// The side of a [`WoodCell`] (m).
pub const CELL_M: f64 = CELL_CANDIDATES as f64 * TREE_CELL_M;
/// The widest trunk off the road (m): a box query grown by it finds every
/// trunk whose foot lies in a cell outside the box (the largest drawn
/// trunk is a dragon tree's 0.55 m × 1.5).
pub const MAX_TRUNK_RADIUS_M: f64 = 1.0;

/// A cell of the trunks made on demand: candidates `cx · 5 …` and
/// `cz · 5 …` of a tile's tree grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WoodCell {
    pub tile: TileKey,
    pub cx: u16,
    pub cz: u16,
}

impl WoodCell {
    /// The cell holding the place (x, z).
    pub fn containing(p: [f64; 2], tile_size_m: f32) -> Self {
        let size = f64::from(tile_size_m);
        let tile = TileKey::containing(p, size);
        let o = tile.origin(size);
        let per = cells_per_tile(tile_size_m);
        let c = |v: f64| (((v / CELL_M).floor().max(0.0)) as usize).min(per - 1) as u16;
        Self {
            tile,
            cx: c(p[0] - o[0]),
            cz: c(p[1] - o[1]),
        }
    }

    /// Every cell a box (lo, hi) touches.
    pub fn covering(lo: [f64; 2], hi: [f64; 2], tile_size_m: f32) -> Vec<Self> {
        let size = f64::from(tile_size_m);
        let mut out = Vec::new();
        let (a, b) = (TileKey::containing(lo, size), TileKey::containing(hi, size));
        let per = cells_per_tile(tile_size_m);
        for tz in a.z..=b.z {
            for tx in a.x..=b.x {
                let tile = TileKey { x: tx, z: tz };
                let o = tile.origin(size);
                let range = |l: f64, h: f64, origin: f64| {
                    let a = ((l - origin) / CELL_M).floor().clamp(0.0, (per - 1) as f64) as usize;
                    let b = ((h - origin) / CELL_M).floor().clamp(0.0, (per - 1) as f64) as usize;
                    a..=b
                };
                for cz in range(lo[1], hi[1], o[1]) {
                    for cx in range(lo[0], hi[0], o[0]) {
                        out.push(Self {
                            tile,
                            cx: cx as u16,
                            cz: cz as u16,
                        });
                    }
                }
            }
        }
        out
    }

    /// The candidates of the tile's tree grid in the cell (columns, rows).
    pub fn candidates(&self, tile_size_m: f32) -> (std::ops::Range<usize>, std::ops::Range<usize>) {
        let side = cells_per_side(tile_size_m);
        let r = |c: u16| {
            let a = usize::from(c) * CELL_CANDIDATES;
            a.min(side)..(a + CELL_CANDIDATES).min(side)
        };
        // The last cell of a tile takes the candidates left over.
        let last = cells_per_tile(tile_size_m) - 1;
        let r_last = |c: u16| {
            let mut x = r(c);
            if usize::from(c) == last {
                x.end = side;
            }
            x
        };
        (r_last(self.cx), r_last(self.cz))
    }

    /// The id bits of the cell's tile (for the trunks' stable ids).
    fn tile_bits(&self) -> u64 {
        (self.tile.x as u32 as u64 & 0xFFFF) << 40 | (self.tile.z as u32 as u64 & 0xFFFF) << 24
    }
}

/// What the trees are placed from, alone (for a server, which has no
/// renderer to share the package with): the tiles (heights and road
/// strips), the land use, the buildings, the forest (`FRST`) and the
/// fields and mapped trees (`VEGE`) – not the graph, the bridges, the
/// routes, the obstacles, the colours and the junctions.
pub fn tree_inputs(pkg: &WorldPackage) -> WorldPackage {
    WorldPackage {
        name: pkg.name.clone(),
        origin: pkg.origin,
        tile_size_m: pkg.tile_size_m,
        samples: pkg.samples,
        tiles: pkg.tiles.clone(),
        graph: None,
        landuse: pkg.landuse.clone(),
        buildings: pkg.buildings.clone(),
        bridges: None,
        routes: Vec::new(),
        region: pkg.region.clone(),
        time_zone: None,
        look: crate::look::RegionLook {
            forest: pkg.look.forest.clone(),
            vegetation: pkg.look.vegetation.clone(),
            ..Default::default()
        },
        obstacles: None,
        places: None,
        junctions: None,
    }
}

/// Cells per tile side.
pub fn cells_per_tile(tile_size_m: f32) -> usize {
    (cells_per_side(tile_size_m) / CELL_CANDIDATES).max(1)
}

/// A trunk: its centre (x, z), radius (m) and stable id.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Trunk {
    pub c: [f64; 2],
    pub r: f64,
    pub id: u64,
}

/// What the section stores of a tile (the road-side trees), to leave out.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Stored {
    /// The tree cells (one byte per candidate; 0 = none) and their side.
    pub cells: Option<(usize, Vec<u8>)>,
    /// The feet of the planted trees.
    pub planted: Vec<[f32; 2]>,
}

impl Stored {
    fn has_cell(&self, idx: usize, side: usize) -> bool {
        self.cells
            .as_ref()
            .is_some_and(|(s, c)| *s == side && c.get(idx).is_some_and(|v| *v != 0))
    }
}

/// The source of the trunks off the road: the package the trees are
/// drawn from (shared with the renderer), its tile size.
#[derive(Debug)]
pub struct Woods {
    tile_size_m: f32,
    pkg: RwLock<Option<Arc<WorldPackage>>>,
}

impl Woods {
    /// From the package the renderer draws (`Arc` shared: no copy).
    pub fn new(pkg: Arc<WorldPackage>) -> Self {
        Self {
            tile_size_m: pkg.tile_size_m,
            pkg: RwLock::new(Some(pkg)),
        }
    }

    /// Without a package yet (the browser: its tiles stream in,
    /// [`Self::set_package`]).
    pub fn empty(tile_size_m: f32) -> Self {
        Self {
            tile_size_m,
            pkg: RwLock::new(None),
        }
    }

    /// The package with the tiles now resident (the browser's streamed
    /// package, the same `Arc` as the renderer's).
    pub fn set_package(&self, pkg: Arc<WorldPackage>) {
        *self.pkg.write().expect("not poisoned") = Some(pkg);
    }

    pub fn package(&self) -> Option<Arc<WorldPackage>> {
        self.pkg.read().expect("not poisoned").clone()
    }

    pub fn tile_size_m(&self) -> f32 {
        self.tile_size_m
    }

    /// The package with tile `key` (and its land use) loaded, if any.
    fn with_tile(&self, key: TileKey) -> Option<Arc<WorldPackage>> {
        self.package().filter(|p| p.tiles.contains_key(&key))
    }

    /// Whether tile `key` is there to make trunks from (`false`: not
    /// loaded yet – ask again later, nothing to cache).
    pub fn has_tile(&self, key: TileKey) -> bool {
        self.with_tile(key).is_some()
    }

    /// The trunks of the grid's trees in `cell` that `stored` does not
    /// hold, from the drawn trees (`None`: the tile is not loaded).
    pub fn grid_trunks(&self, cell: WoodCell, stored: &Stored) -> Option<Vec<Trunk>> {
        let pkg = self.with_tile(cell.tile)?;
        let tile = &pkg.tiles[&cell.tile];
        let Some((lu, cells)) = tile_landuse(&pkg, cell.tile) else {
            return Some(Vec::new());
        };
        let side = cells_per_side(pkg.tile_size_m);
        let (cols, rows) = cell.candidates(pkg.tile_size_m);
        let gt = GridTrees::of_tile(&pkg, cell.tile);
        let bits = cell.tile_bits();
        let mut out = Vec::new();
        for (idx, class, foot, s) in landuse_trees_in(&pkg, tile, lu, cells, cols, rows) {
            if stored.has_cell(idx, side) {
                continue;
            }
            let (sp, scale) = gt.tree(&pkg, Some(class), foot, s);
            let r = f64::from(sp.trunk_radius_m() * scale);
            if r <= 0.0 {
                continue;
            }
            out.push(Trunk {
                c: [f64::from(foot[0]), f64::from(foot[2])],
                r,
                id: 8 << 56 | bits | idx as u64,
            });
        }
        Some(out)
    }

    /// The trunks of the planted trees of tile `key` (orchard rows, mapped
    /// trees) that `stored` does not hold (`None`: not loaded).
    pub fn planted_trunks(&self, key: TileKey, stored: &Stored) -> Option<Vec<Trunk>> {
        let pkg = self.with_tile(key)?;
        let bits = (key.x as u32 as u64 & 0xFFFF) << 40 | (key.z as u32 as u64 & 0xFFFF) << 24;
        Some(
            crate::plantings::planted_points(&pkg, key, |_| true)
                .into_iter()
                .filter(|p| !stored.planted.contains(&p.pos))
                .enumerate()
                .map(|(i, p)| Trunk {
                    c: [f64::from(p.pos[0]), f64::from(p.pos[1])],
                    r: f64::from(p.width_m) / 2.0,
                    id: 9 << 56 | bits | i as u64,
                })
                .collect(),
        )
    }
}

#[doc(hidden)]
pub mod fixture;

#[cfg(test)]
mod tests;
