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
//! there is one; the browser swaps in its resident tiles as they stream)
//! or, on a server, the trees baked per tile at load ([`bake_tile`]: each
//! candidate's trunk radius, deflated – the trees' inputs themselves are
//! hundreds of MB in a big world); the cells are made, cached and dropped
//! by the user (`ocw_sim::collide::WorldCache`). Radii are whole
//! millimetres on every path.

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

/// A trunk's radius in whole millimetres (0: none) – the same on every
/// path (the package's trees and a server's baked ones).
fn radius_mm(r: f32) -> u16 {
    (r * 1000.0).round().clamp(0.0, f32::from(u16::MAX)) as u16
}

/// The trees of a tile baked for a server (`ocw-server` has no renderer
/// whose package it could share, and the trees' inputs of a big world are
/// hundreds of MB: the northern Black Forest's 2,200 tiles 310 MB): each
/// candidate's trunk radius in millimetres (0: none), deflated, and the
/// planted trees. Made once at load ([`bake_tile`]); the cells are still
/// made on demand from it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BakedTile {
    /// `side²` little-endian `u16` radii (mm), deflated; empty: no trees.
    radii: Vec<u8>,
    /// The planted trees: foot (x, z) and radius.
    planted: Vec<([f32; 2], f32)>,
}

impl BakedTile {
    /// Bytes in memory.
    pub fn bytes(&self) -> usize {
        self.radii.len() + self.planted.len() * 12 + std::mem::size_of::<Self>()
    }
}

/// The grid's trees of tile `key` as (candidate, foot (x, z), radius in
/// mm), the soft ones left out; `None` without the tile.
fn tile_grid(
    pkg: &WorldPackage,
    key: TileKey,
    cols: std::ops::Range<usize>,
    rows: std::ops::Range<usize>,
) -> Option<Vec<(usize, [f32; 2], u16)>> {
    let tile = pkg.tiles.get(&key)?;
    let Some((lu, cells)) = tile_landuse(pkg, key) else {
        return Some(Vec::new());
    };
    let gt = GridTrees::of_tile(pkg, key);
    Some(
        landuse_trees_in(pkg, tile, lu, cells, cols, rows)
            .into_iter()
            .filter_map(|(idx, class, foot, s)| {
                let (sp, scale) = gt.tree(pkg, Some(class), foot, s);
                let r = radius_mm(sp.trunk_radius_m() * scale);
                (r > 0).then_some((idx, [foot[0], foot[2]], r))
            })
            .collect(),
    )
}

/// The planted trees of tile `key` (foot, radius).
fn tile_planted(pkg: &WorldPackage, key: TileKey) -> Vec<([f32; 2], f32)> {
    crate::plantings::planted_points(pkg, key, |_| true)
        .into_iter()
        .map(|p| (p.pos, p.width_m / 2.0))
        .collect()
}

/// Bakes tile `key` of `pkg` for [`Woods::baked`] (`None`: no such tile).
pub fn bake_tile(pkg: &WorldPackage, key: TileKey) -> Option<BakedTile> {
    let side = cells_per_side(pkg.tile_size_m);
    let grid = tile_grid(pkg, key, 0..side, 0..side)?;
    let radii = if grid.is_empty() {
        Vec::new()
    } else {
        let mut raw = vec![0u8; side * side * 2];
        for (idx, _, r) in grid {
            raw[idx * 2..idx * 2 + 2].copy_from_slice(&r.to_le_bytes());
        }
        miniz_oxide::deflate::compress_to_vec(&raw, 6)
    };
    Some(BakedTile {
        radii,
        planted: tile_planted(pkg, key),
    })
}

/// Where candidate `(i, k)` of tile `key` stands (x, z), as the placement
/// has it ([`crate::trees`]: the jittered 8 m grid, rounded to `f32`).
fn candidate_foot(key: TileKey, tile_size_m: f32, i: usize, k: usize) -> [f32; 2] {
    let size = f64::from(tile_size_m);
    let o = key.origin(size);
    let per_side = cells_per_side(tile_size_m) as i64;
    let (gx, gz) = (
        i64::from(key.x) * per_side + i as i64,
        i64::from(key.z) * per_side + k as i64,
    );
    let x = o[0] + (i as f64 + f64::from(crate::trees::hash(gx, gz, 2))) * TREE_CELL_M;
    let z = o[1] + (k as f64 + f64::from(crate::trees::hash(gx, gz, 3))) * TREE_CELL_M;
    [x as f32, z as f32]
}

/// Where the trees come from.
#[derive(Debug)]
enum Source {
    /// The package the renderer draws (or the browser's resident tiles).
    Package(RwLock<Option<Arc<WorldPackage>>>),
    /// A server's baked tiles.
    Baked(std::collections::BTreeMap<TileKey, BakedTile>),
}

/// The source of the trunks off the road: the package the trees are
/// drawn from (shared with the renderer) or a server's baked tiles.
#[derive(Debug)]
pub struct Woods {
    tile_size_m: f32,
    source: Source,
}

impl Woods {
    /// From the package the renderer draws (`Arc` shared: no copy).
    pub fn new(pkg: Arc<WorldPackage>) -> Self {
        Self {
            tile_size_m: pkg.tile_size_m,
            source: Source::Package(RwLock::new(Some(pkg))),
        }
    }

    /// Without a package yet (the browser: its tiles stream in,
    /// [`Self::set_package`]).
    pub fn empty(tile_size_m: f32) -> Self {
        Self {
            tile_size_m,
            source: Source::Package(RwLock::new(None)),
        }
    }

    /// From baked tiles ([`bake_tile`]; a server).
    pub fn baked(tile_size_m: f32, tiles: std::collections::BTreeMap<TileKey, BakedTile>) -> Self {
        Self {
            tile_size_m,
            source: Source::Baked(tiles),
        }
    }

    /// All tiles of `pkg` baked, one after the other ([`bake_tile`]; a
    /// server bakes them in parallel itself).
    pub fn bake(pkg: &WorldPackage) -> Self {
        let tiles = pkg
            .tiles
            .keys()
            .filter_map(|k| Some((*k, bake_tile(pkg, *k)?)))
            .collect();
        Self::baked(pkg.tile_size_m, tiles)
    }

    /// The package with the tiles now resident (the browser's streamed
    /// package, the same `Arc` as the renderer's); nothing for baked tiles.
    pub fn set_package(&self, pkg: Arc<WorldPackage>) {
        if let Source::Package(p) = &self.source {
            *p.write().expect("not poisoned") = Some(pkg);
        }
    }

    /// The package (`None`: baked, or none yet).
    pub fn package(&self) -> Option<Arc<WorldPackage>> {
        match &self.source {
            Source::Package(p) => p.read().expect("not poisoned").clone(),
            Source::Baked(_) => None,
        }
    }

    pub fn tile_size_m(&self) -> f32 {
        self.tile_size_m
    }

    /// Memory of the baked tiles (bytes; 0 for a shared package).
    pub fn baked_bytes(&self) -> usize {
        match &self.source {
            Source::Baked(t) => t.values().map(BakedTile::bytes).sum(),
            Source::Package(_) => 0,
        }
    }

    /// The package with tile `key` (and its land use) loaded, if any.
    fn with_tile(&self, key: TileKey) -> Option<Arc<WorldPackage>> {
        self.package().filter(|p| p.tiles.contains_key(&key))
    }

    /// Whether tile `key` is there to make trunks from (`false`: not
    /// loaded yet – ask again later, nothing to cache).
    pub fn has_tile(&self, key: TileKey) -> bool {
        match &self.source {
            Source::Baked(t) => t.contains_key(&key),
            Source::Package(_) => self.with_tile(key).is_some(),
        }
    }

    /// The grid's trees of `cell`: (candidate, foot, radius mm).
    fn cell_grid(&self, cell: WoodCell) -> Option<Vec<(usize, [f32; 2], u16)>> {
        let (cols, rows) = cell.candidates(self.tile_size_m);
        match &self.source {
            Source::Package(_) => {
                let pkg = self.with_tile(cell.tile)?;
                tile_grid(&pkg, cell.tile, cols, rows)
            }
            Source::Baked(tiles) => {
                let t = tiles.get(&cell.tile)?;
                if t.radii.is_empty() {
                    return Some(Vec::new());
                }
                let side = cells_per_side(self.tile_size_m);
                let raw =
                    miniz_oxide::inflate::decompress_to_vec_with_limit(&t.radii, side * side * 2)
                        .ok()?;
                let mut out = Vec::new();
                for k in rows {
                    for i in cols.clone() {
                        let idx = k * side + i;
                        let r = u16::from_le_bytes([raw[idx * 2], raw[idx * 2 + 1]]);
                        if r > 0 {
                            out.push((idx, candidate_foot(cell.tile, self.tile_size_m, i, k), r));
                        }
                    }
                }
                Some(out)
            }
        }
    }

    /// The trunks of the grid's trees in `cell` that `stored` does not
    /// hold, from the drawn trees (`None`: the tile is not loaded).
    pub fn grid_trunks(&self, cell: WoodCell, stored: &Stored) -> Option<Vec<Trunk>> {
        let side = cells_per_side(self.tile_size_m);
        let bits = cell.tile_bits();
        Some(
            self.cell_grid(cell)?
                .into_iter()
                .filter(|(idx, _, _)| !stored.has_cell(*idx, side))
                .map(|(idx, foot, r)| Trunk {
                    c: [f64::from(foot[0]), f64::from(foot[1])],
                    r: f64::from(r) / 1000.0,
                    id: 8 << 56 | bits | idx as u64,
                })
                .collect(),
        )
    }

    /// The trunks of the planted trees of tile `key` (orchard rows, mapped
    /// trees) that `stored` does not hold (`None`: not loaded).
    pub fn planted_trunks(&self, key: TileKey, stored: &Stored) -> Option<Vec<Trunk>> {
        let planted = match &self.source {
            Source::Package(_) => tile_planted(&*self.with_tile(key)?, key),
            Source::Baked(tiles) => tiles.get(&key)?.planted.clone(),
        };
        let bits = (key.x as u32 as u64 & 0xFFFF) << 40 | (key.z as u32 as u64 & 0xFFFF) << 24;
        Some(
            planted
                .into_iter()
                .filter(|(p, _)| !stored.planted.contains(p))
                .enumerate()
                .map(|(i, (p, r))| Trunk {
                    c: [f64::from(p[0]), f64::from(p[1])],
                    r: f64::from(r),
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
