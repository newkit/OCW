//! Where the trees of a package stand (spec §4.6; moved from the renderer
//! for spec §14 Phase 12, K3): a jittered 8 m grid per tile, each
//! candidate kept with the chance of its land class, never near a road
//! (3 m beyond the edge) or under a building, never on rock faces. Pure
//! and deterministic: the renderer draws these trees, the world builder
//! writes which cells hold one into the section `OBST` (as the class of
//! the tree, [`tree_cells`]) and the server rebuilds their trunks from it
//! ([`trunks_from_cells`]) – the same trees everywhere, without the land
//! use or the terrain on the server.

use crate::landuse::{LandClass, LandUseTile};
use crate::package::{Tile, TileKey, WorldPackage};

/// The grid of tree candidates (m).
pub const TREE_CELL_M: f64 = 8.0;
/// Radius of a tree's trunk at the ground per unit of its scale (m; the
/// drawn trunk is a cone of this radius, 2.2 m high per unit).
pub const TRUNK_RADIUS_M: f64 = 0.25;

/// Where a tree stands (foot of the trunk, package metres) and its scale.
pub type TreeSite = ([f32; 3], f32);

/// The hash of the tree placement (seed: 1 the chance, 2 / 3 the jitter
/// along x / z, 4 the scale, 7 the placeholder patches, 11 the crown
/// shade): uniform in [0, 1).
pub fn hash(x: i64, z: i64, seed: u64) -> f32 {
    let mut h = (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (z as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ seed;
    h ^= h >> 33;
    h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    h ^= h >> 33;
    (h >> 40) as f32 / (1u64 << 24) as f32
}

/// Chance of a tree per 8 m × 8 m cell and the scale range, by land class.
pub fn tree_density(class: LandClass) -> (f32, f32, f32) {
    match class {
        LandClass::Forest => (0.85, 0.8, 1.5),
        LandClass::Orchard => (0.45, 0.45, 0.65),
        LandClass::Scrub => (0.3, 0.35, 0.6),
        LandClass::Park => (0.1, 0.7, 1.2),
        LandClass::Residential => (0.06, 0.6, 1.0),
        LandClass::Wetland => (0.05, 0.5, 0.9),
        LandClass::Default | LandClass::Meadow | LandClass::Farmland => (0.012, 0.7, 1.3),
        LandClass::Commercial | LandClass::Industrial => (0.01, 0.6, 1.0),
        LandClass::Vineyard
        | LandClass::Water
        | LandClass::Rock
        | LandClass::Scree
        | LandClass::Glacier
        | LandClass::Bare => (0.0, 1.0, 1.0),
    }
}

/// The land use of a tile, if the package has it.
pub fn tile_landuse(pkg: &WorldPackage, key: TileKey) -> Option<(&LandUseTile, u32)> {
    let l = pkg.landuse.as_ref()?;
    Some((l.tiles.get(&key)?, l.cells))
}

fn sub(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

fn dot(a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] * b[0]) + (a[1] * b[1])
}

/// Land-use cells of a tile where no tree may stand: near the road strips
/// (3 m beyond the edge) and under buildings (plus 1.5 m), also buildings
/// of the neighbouring tiles that reach into this one.
pub fn blocked_cells(pkg: &WorldPackage, tile: &Tile, cells: u32) -> Vec<bool> {
    let size = pkg.tile_size_m;
    let o = tile.key.origin(f64::from(size));
    let (ox, oz) = (o[0] as f32, o[1] as f32);
    let n = cells as usize;
    let cs = size / cells as f32;
    let mut blocked = vec![false; n * n];
    let cell_range = |lo: f32, hi: f32, origin: f32| {
        let a = ((lo - origin) / cs).floor().max(0.0) as usize;
        let b = (((hi - origin) / cs).floor() + 1.0).clamp(0.0, n as f32) as usize;
        a..b
    };
    for r in &tile.roads {
        // A cell is blocked if its centre is within the reach (half a cell
        // diagonal more, so no part of a blocked tree's cell is missed).
        let reach = r.width_m / 2.0 + 3.0 + cs * 0.71;
        for w in r.points.windows(2) {
            let (a, b) = ([w[0][0], w[0][2]], [w[1][0], w[1][2]]);
            let ab = sub(b, a);
            for k in cell_range(a[1].min(b[1]) - reach, a[1].max(b[1]) + reach, oz) {
                for i in cell_range(a[0].min(b[0]) - reach, a[0].max(b[0]) + reach, ox) {
                    let p = [ox + (i as f32 + 0.5) * cs, oz + (k as f32 + 0.5) * cs];
                    let l2 = dot(ab, ab);
                    let t = if l2 > 0.0 {
                        (dot(sub(p, a), ab) / l2).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    let d = sub(p, [a[0] + ab[0] * t, a[1] + ab[1] * t]);
                    if dot(d, d).sqrt() <= reach {
                        blocked[k * n + i] = true;
                    }
                }
            }
        }
    }
    if let Some(all) = &pkg.buildings {
        for dz in -1..=1 {
            for dx in -1..=1 {
                let key = TileKey {
                    x: tile.key.x + dx,
                    z: tile.key.z + dz,
                };
                for b in all.get(&key).into_iter().flatten() {
                    let (mut lo, mut hi) = ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]);
                    for p in &b.outline {
                        lo = [lo[0].min(p[0]), lo[1].min(p[1])];
                        hi = [hi[0].max(p[0]), hi[1].max(p[1])];
                    }
                    for k in cell_range(lo[1] - 1.5, hi[1] + 1.5, oz) {
                        for i in cell_range(lo[0] - 1.5, hi[0] + 1.5, ox) {
                            blocked[k * n + i] = true;
                        }
                    }
                }
            }
        }
    }
    blocked
}

/// Terrain height at (x, z) and whether it is a rock face there.
fn ground_and_steep(pkg: &WorldPackage, x: f64, z: f64) -> Option<(f64, bool)> {
    let (y, yx, yz) = (
        pkg.height(x, z)?,
        pkg.height(x + 2.0, z)?,
        pkg.height(x, z + 2.0)?,
    );
    Some((y, ((yx - y).powi(2) + (yz - y).powi(2)).sqrt() / 2.0 > 0.8))
}

/// Tree candidates per tile side.
pub fn cells_per_side(tile_size_m: f32) -> usize {
    (f64::from(tile_size_m) / TREE_CELL_M) as usize
}

/// The global grid cell of candidate `(i, k)` of a tile.
fn global_cell(key: TileKey, per_side: i64, i: i64, k: i64) -> (i64, i64) {
    (
        i64::from(key.x) * per_side + i,
        i64::from(key.z) * per_side + k,
    )
}

/// Where the tree of candidate `(i, k)` of a tile stands (x, z) and its
/// scale for land class `class`.
pub fn tree_at(
    key: TileKey,
    tile_size_m: f32,
    i: usize,
    k: usize,
    class: LandClass,
) -> ([f64; 2], f32) {
    let size = f64::from(tile_size_m);
    let o = key.origin(size);
    let per_side = cells_per_side(tile_size_m) as i64;
    let (gx, gz) = global_cell(key, per_side, i as i64, k as i64);
    let x = o[0] + (i as f64 + f64::from(hash(gx, gz, 2))) * TREE_CELL_M;
    let z = o[1] + (k as f64 + f64::from(hash(gx, gz, 3))) * TREE_CELL_M;
    let (_, s0, s1) = tree_density(class);
    ([x, z], s0 + (s1 - s0) * hash(gx, gz, 4))
}

/// The trees of a tile by land use, as `(candidate index i + k · side,
/// land class, foot, scale)`: a jittered 8 m grid, each candidate kept
/// with the chance of its class ([`tree_density`]), not in blocked cells
/// ([`blocked_cells`]) and not on rock faces.
fn landuse_trees(
    pkg: &WorldPackage,
    tile: &Tile,
    lu: &LandUseTile,
    cells: u32,
) -> Vec<(usize, LandClass, [f32; 3], f32)> {
    let size = f64::from(pkg.tile_size_m);
    let o = tile.key.origin(size);
    let per_side = cells_per_side(pkg.tile_size_m) as i64;
    let n = cells as usize;
    let cs = size / f64::from(cells);
    let blocked = blocked_cells(pkg, tile, cells);
    let mut sites = Vec::new();
    for k in 0..per_side {
        for i in 0..per_side {
            let (gx, gz) = global_cell(tile.key, per_side, i, k);
            let x = o[0] + (i as f64 + f64::from(hash(gx, gz, 2))) * TREE_CELL_M;
            let z = o[1] + (k as f64 + f64::from(hash(gx, gz, 3))) * TREE_CELL_M;
            let (c, r) = (
                (((x - o[0]) / cs) as usize).min(n - 1),
                (((z - o[1]) / cs) as usize).min(n - 1),
            );
            let class = lu.classes[r * n + c];
            let (chance, s0, s1) = tree_density(class);
            if hash(gx, gz, 1) >= chance || blocked[r * n + c] {
                continue;
            }
            let Some((y, too_steep)) = ground_and_steep(pkg, x, z) else {
                continue;
            };
            if too_steep {
                continue;
            }
            let s = s0 + (s1 - s0) * hash(gx, gz, 4);
            sites.push((
                (k * per_side + i) as usize,
                class,
                [x as f32, y as f32 - 0.3, z as f32],
                s,
            ));
        }
    }
    sites
}

/// Trees by land use (see [`landuse_trees`]).
pub fn landuse_tree_sites(
    pkg: &WorldPackage,
    tile: &Tile,
    lu: &LandUseTile,
    cells: u32,
) -> Vec<TreeSite> {
    landuse_trees(pkg, tile, lu, cells)
        .into_iter()
        .map(|(_, _, p, s)| (p, s))
        .collect()
}

/// Provisional vegetation (packages without land use): trees on a
/// jittered grid in patches, not on steep rock and never on a road.
pub fn placeholder_tree_sites(pkg: &WorldPackage, tile: &Tile) -> Vec<TreeSite> {
    let size = f64::from(pkg.tile_size_m);
    let o = tile.key.origin(size);
    let cell = 18.0;
    let cells = (size / cell) as i64;
    let mut sites = Vec::new();
    let road_pts: Vec<([f32; 2], f32)> = tile
        .roads
        .iter()
        .flat_map(|r| {
            r.points
                .iter()
                .map(move |p| ([p[0], p[2]], r.width_m / 2.0 + 5.0))
        })
        .collect();
    for k in 0..cells {
        for i in 0..cells {
            let (gx, gz) = (tile.key.x as i64 * cells + i, tile.key.z as i64 * cells + k);
            // Forest patches: coarse noise from a 6 × 6-cell lattice.
            let patch = hash(gx.div_euclid(6), gz.div_euclid(6), 7);
            if hash(gx, gz, 1) > patch * 0.9 {
                continue;
            }
            let x = o[0] + (i as f64 + f64::from(hash(gx, gz, 2))) * cell;
            let z = o[1] + (k as f64 + f64::from(hash(gx, gz, 3))) * cell;
            let p2 = [x as f32, z as f32];
            if road_pts.iter().any(|(q, r)| {
                let d = sub(*q, p2);
                dot(d, d) < r * r
            }) {
                continue;
            }
            let Some((y, too_steep)) = ground_and_steep(pkg, x, z) else {
                continue;
            };
            if too_steep {
                continue; // rock faces
            }
            let s = 0.7 + 0.7 * hash(gx, gz, 4);
            sites.push(([x as f32, y as f32 - 0.3, z as f32], s));
        }
    }
    sites
}

/// Where the trees of a tile stand: from the land use if the package has
/// it, else provisional patches.
pub fn tree_sites(pkg: &WorldPackage, tile: &Tile) -> Vec<TreeSite> {
    match tile_landuse(pkg, tile.key) {
        Some((lu, cells)) => landuse_tree_sites(pkg, tile, lu, cells),
        None => placeholder_tree_sites(pkg, tile),
    }
}

/// The trees of a tile as one byte per candidate cell (row-major along
/// +z, [`cells_per_side`]²): 0 = no tree, else the tree's land class
/// code plus 1 (its scale range, [`tree_density`]). `keep` decides by the foot
/// (x, z) which trees are written (the world builder: within reach of the
/// roads). `None` without land use (the placeholder trees are not
/// stored).
pub fn tree_cells(
    pkg: &WorldPackage,
    tile: &Tile,
    keep: impl Fn([f64; 2]) -> bool,
) -> Option<Vec<u8>> {
    let (lu, cells) = tile_landuse(pkg, tile.key)?;
    let side = cells_per_side(pkg.tile_size_m);
    let mut out = vec![0u8; side * side];
    for (idx, class, p, _) in landuse_trees(pkg, tile, lu, cells) {
        if keep([f64::from(p[0]), f64::from(p[2])]) {
            out[idx] = class.code() + 1;
        }
    }
    Some(out)
}

/// The trunks (centre x, z and radius, m) of the trees `cells` holds
/// ([`tree_cells`]) on tile `key`.
pub fn trunks_from_cells(key: TileKey, tile_size_m: f32, cells: &[u8]) -> Vec<([f64; 2], f64)> {
    let side = cells_per_side(tile_size_m);
    cells
        .iter()
        .enumerate()
        .filter(|(_, c)| **c != 0)
        .filter_map(|(idx, c)| {
            let class = LandClass::from_code(c - 1)?;
            let (p, s) = tree_at(key, tile_size_m, idx % side, idx / side, class);
            Some((p, TRUNK_RADIUS_M * f64::from(s)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::landuse::LandUse;
    use std::collections::BTreeMap;

    fn forest_package() -> WorldPackage {
        let key = TileKey { x: 0, z: 0 };
        let n = 11u32;
        let tile = Tile {
            key,
            heights: vec![100.0; (n * n) as usize],
            roads: Vec::new(),
        };
        WorldPackage {
            name: "Forest".into(),
            origin: (47.0, 9.0),
            tile_size_m: 1000.0,
            samples: n,
            tiles: BTreeMap::from([(key, tile)]),
            graph: None,
            landuse: Some(LandUse {
                cells: 4,
                tiles: BTreeMap::from([(
                    key,
                    LandUseTile {
                        classes: vec![LandClass::Forest; 16],
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
        }
    }

    #[test]
    fn stored_cells_give_back_the_drawn_trunks() {
        let pkg = forest_package();
        let tile = &pkg.tiles[&TileKey { x: 0, z: 0 }];
        let drawn = tree_sites(&pkg, tile);
        assert!(drawn.len() > 10_000, "{}", drawn.len());
        let cells = tree_cells(&pkg, tile, |_| true).unwrap();
        let trunks = trunks_from_cells(tile.key, pkg.tile_size_m, &cells);
        assert_eq!(trunks.len(), drawn.len());
        for ((p, r), (q, s)) in trunks.iter().zip(&drawn) {
            assert!((p[0] - f64::from(q[0])).abs() < 1e-3 && (p[1] - f64::from(q[2])).abs() < 1e-3);
            assert!((r - TRUNK_RADIUS_M * f64::from(*s)).abs() < 1e-9);
            // Forest trunks: 0.2–0.375 m.
            assert!((0.19..0.38).contains(r), "{r}");
        }
        // Only the kept ones.
        let half = tree_cells(&pkg, tile, |p| p[0] < 500.0).unwrap();
        let some = trunks_from_cells(tile.key, pkg.tile_size_m, &half);
        assert!(some.iter().all(|(p, _)| p[0] < 500.0) && some.len() < trunks.len());
    }
}
