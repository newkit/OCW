//! Plants in rows and mapped trees (spec §4.6.1, §14 Phase 8 T8): where
//! the vines, fruit trees, banana plants and avocados of the plantations of
//! section `VEGE` ([`crate::vegetation`]) stand, and the single trees OSM
//! maps. Pure and deterministic: the renderer draws them, the world
//! builder writes the solid ones (fruit trees, mapped trees) into `OBST`
//! as point obstacles ([`crate::obstacles::ObstacleKind::PlantedTree`]).
//!
//! Each field's rows are global lines in its direction `d` (the cell's row
//! byte): row `k` holds the points `p` with `dot(p, n) = k · between`
//! (`n` the normal of `d`), so a field's rows run on across cells and
//! tiles; along a row the plants stand every `along` metres (from
//! `dot(p, d) = j · along`), with a little jitter and a few gaps. Nothing
//! stands on a road (its half width plus [`ROAD_CLEARANCE_M`]) or in a
//! building.

use crate::flora::{FloraRegion, Species};
use crate::package::{Tile, TileKey, WorldPackage};
use crate::trees::hash;
use crate::vegetation::{FieldCrop, PlantedTree, VegetationTile};

/// How far beside a road's edge rows end (m): the headland where the
/// tractor turns.
pub const ROAD_CLEARANCE_M: f32 = 1.5;
/// The spacing of the clearance grid (m).
const CLEAR_CELL_M: f32 = 2.0;
/// The share of row places left empty (dead vines, missing trees).
pub const GAPS: f32 = 0.04;

/// The spacing of a plantation's rows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RowSpec {
    /// Between two rows (m).
    pub between_m: f32,
    /// Between two plants in a row (m).
    pub along_m: f32,
}

/// The rows of `crop` in `region` (spec §4.6.1 table 4): vineyards on wire
/// trellises (2.0 m × 1.1 m in the Alpine Rhine and the Black Forest,
/// wider on Tenerife), orchards of half-standard trees, banana plants about
/// 2 000 per hectare, avocados and citrus 6 m × 5 m, berry hedges; `None`
/// for crops not planted in rows (maize is drawn as a stand).
pub fn row_spec(crop: FieldCrop, region: FloraRegion) -> Option<RowSpec> {
    let tf = region == FloraRegion::Tenerife;
    let (between_m, along_m) = match crop {
        FieldCrop::Vineyard if tf => (2.4, 1.4),
        FieldCrop::Vineyard => (2.0, 1.1),
        FieldCrop::Orchard if tf => (6.0, 5.0),
        FieldCrop::Orchard => (7.0, 5.5),
        FieldCrop::Banana => (2.5, 2.2),
        FieldCrop::Subtropical => (6.0, 5.0),
        FieldCrop::Berries => (2.8, 1.0),
        _ => return None,
    };
    Some(RowSpec { between_m, along_m })
}

/// Where plants may stand on a tile: a grid of [`CLEAR_CELL_M`] cells,
/// blocked within a road's half width plus [`ROAD_CLEARANCE_M`] (of every
/// road strip of the tile) and inside buildings (plus 1 m).
pub struct Clearance {
    origin: [f32; 2],
    n: usize,
    blocked: Vec<bool>,
}

impl Clearance {
    pub fn new(pkg: &WorldPackage, tile: &Tile) -> Self {
        let size = pkg.tile_size_m;
        let o = tile.key.origin(f64::from(size));
        let origin = [o[0] as f32, o[1] as f32];
        let n = (size / CLEAR_CELL_M).ceil() as usize;
        let mut c = Self {
            origin,
            n,
            blocked: vec![false; n * n],
        };
        for r in &tile.roads {
            let reach = r.width_m / 2.0 + ROAD_CLEARANCE_M + CLEAR_CELL_M * 0.71;
            for w in r.points.windows(2) {
                c.block_segment([w[0][0], w[0][2]], [w[1][0], w[1][2]], reach);
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
                        c.block_box([lo[0] - 1.0, lo[1] - 1.0], [hi[0] + 1.0, hi[1] + 1.0]);
                    }
                }
            }
        }
        c
    }

    fn range(&self, lo: f32, hi: f32, origin: f32) -> std::ops::Range<usize> {
        let a = ((lo - origin) / CLEAR_CELL_M).floor().max(0.0) as usize;
        let b = (((hi - origin) / CLEAR_CELL_M).floor() + 1.0).clamp(0.0, self.n as f32) as usize;
        a.min(b)..b
    }

    fn block_box(&mut self, lo: [f32; 2], hi: [f32; 2]) {
        for k in self.range(lo[1], hi[1], self.origin[1]) {
            for i in self.range(lo[0], hi[0], self.origin[0]) {
                self.blocked[k * self.n + i] = true;
            }
        }
    }

    fn block_segment(&mut self, a: [f32; 2], b: [f32; 2], reach: f32) {
        let ab = [b[0] - a[0], b[1] - a[1]];
        let l2 = ab[0] * ab[0] + ab[1] * ab[1];
        for k in self.range(
            a[1].min(b[1]) - reach,
            a[1].max(b[1]) + reach,
            self.origin[1],
        ) {
            for i in self.range(
                a[0].min(b[0]) - reach,
                a[0].max(b[0]) + reach,
                self.origin[0],
            ) {
                let p = [
                    self.origin[0] + (i as f32 + 0.5) * CLEAR_CELL_M,
                    self.origin[1] + (k as f32 + 0.5) * CLEAR_CELL_M,
                ];
                let t = if l2 > 0.0 {
                    (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / l2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let d = [p[0] - a[0] - ab[0] * t, p[1] - a[1] - ab[1] * t];
                if d[0] * d[0] + d[1] * d[1] <= reach * reach {
                    self.blocked[k * self.n + i] = true;
                }
            }
        }
    }

    /// Whether a plant may stand at (x, z) (package metres; outside the
    /// tile: no).
    pub fn free(&self, x: f32, z: f32) -> bool {
        let (fi, fk) = (
            (x - self.origin[0]) / CLEAR_CELL_M,
            (z - self.origin[1]) / CLEAR_CELL_M,
        );
        if fi < 0.0 || fk < 0.0 {
            return false;
        }
        let (i, k) = (fi as usize, fk as usize);
        i < self.n && k < self.n && !self.blocked[k * self.n + i]
    }
}

/// One straight piece of a row (vines, berry hedges), package metres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RowRun {
    pub crop: FieldCrop,
    /// Its ends (x, z).
    pub a: [f32; 2],
    pub b: [f32; 2],
}

/// A plant of a plantation: where it stands (foot, package metres), what
/// it is, its scale and its turn about the vertical (radians).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RowPlant {
    pub pos: [f32; 3],
    pub crop: FieldCrop,
    pub species: Species,
    pub scale: f32,
    pub turn: f32,
}

/// The species of a plantation's plants.
pub fn plant_species(crop: FieldCrop, region: FloraRegion) -> Option<Species> {
    Some(match crop {
        FieldCrop::Orchard if region == FloraRegion::Tenerife => Species::FruitTree,
        FieldCrop::Orchard => Species::FruitTree,
        FieldCrop::Banana => Species::Banana,
        FieldCrop::Subtropical => Species::SubtropicalFruit,
        _ => return None,
    })
}

/// Whether a crop's rows are drawn as runs (continuous hedges of vines or
/// berries) rather than single plants.
pub fn runs(crop: FieldCrop) -> bool {
    matches!(crop, FieldCrop::Vineyard | FieldCrop::Berries)
}

/// The groups of cells of a tile planted in rows: (crop, row byte) and
/// the box of their cells (column / row ranges).
fn row_groups(v: &VegetationTile, cells: usize) -> Vec<((FieldCrop, u8), [usize; 4])> {
    let mut groups: Vec<((FieldCrop, u8), [usize; 4])> = Vec::new();
    if v.crops.is_empty() {
        return groups;
    }
    for r in 0..cells {
        for c in 0..cells {
            let i = r * cells + c;
            let Some(crop) = FieldCrop::from_code(v.crops[i]) else {
                continue;
            };
            let dir = v.rows[i];
            if dir == crate::vegetation::NO_ROWS
                || !(runs(crop) || plant_species(crop, FloraRegion::Other).is_some())
            {
                continue;
            }
            match groups.iter_mut().find(|g| g.0 == (crop, dir)) {
                Some(g) => {
                    g.1 = [g.1[0].min(c), g.1[1].max(c), g.1[2].min(r), g.1[3].max(r)];
                }
                None => groups.push(((crop, dir), [c, c, r, r])),
            }
        }
    }
    groups.sort_by_key(|g| (g.0.0.code(), g.0.1));
    groups
}

/// The rows of a tile: the row runs (vines, berries) and the row plants
/// (fruit trees, bananas, avocados), each on free ground ([`Clearance`]).
pub fn tile_rows(pkg: &WorldPackage, key: TileKey) -> (Vec<RowRun>, Vec<RowPlant>) {
    let (mut out_runs, mut plants) = (Vec::new(), Vec::new());
    let Some(veg) = pkg.look.vegetation.as_ref() else {
        return (out_runs, plants);
    };
    let (Some(v), Some(tile)) = (veg.tiles.get(&key), pkg.tiles.get(&key)) else {
        return (out_runs, plants);
    };
    let region = FloraRegion::at(pkg.origin.0, pkg.origin.1);
    let size = f64::from(pkg.tile_size_m);
    let o = key.origin(size);
    let n = veg.cells as usize;
    let cs = size / n as f64;
    let clear = Clearance::new(pkg, tile);
    let cell_of = |x: f64, z: f64| -> Option<usize> {
        let (c, r) = ((x - o[0]) / cs, (z - o[1]) / cs);
        (c >= 0.0 && r >= 0.0 && c < n as f64 && r < n as f64).then(|| r as usize * n + c as usize)
    };
    for ((crop, dir), bx) in row_groups(v, n) {
        let Some(spec) = row_spec(crop, region) else {
            continue;
        };
        let theta = f64::from(dir) / 256.0 * std::f64::consts::PI;
        let d = [theta.cos(), theta.sin()];
        let nrm = [-d[1], d[0]];
        let lo = [o[0] + bx[0] as f64 * cs, o[1] + bx[2] as f64 * cs];
        let hi = [
            o[0] + (bx[1] + 1) as f64 * cs,
            o[1] + (bx[3] + 1) as f64 * cs,
        ];
        let corners = [lo, [hi[0], lo[1]], [lo[0], hi[1]], hi];
        let proj = |p: [f64; 2], a: [f64; 2]| p[0] * a[0] + p[1] * a[1];
        let span = |a: [f64; 2]| {
            corners
                .iter()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), c| {
                    let v = proj(*c, a);
                    (l.min(v), h.max(v))
                })
        };
        let (nlo, nhi) = span(nrm);
        let (tlo, thi) = span(d);
        let between = f64::from(spec.between_m);
        let inside = |p: [f64; 2]| -> bool {
            cell_of(p[0], p[1])
                .is_some_and(|i| FieldCrop::from_code(v.crops[i]) == Some(crop) && v.rows[i] == dir)
                && clear.free(p[0] as f32, p[1] as f32)
        };
        for k in (nlo / between).ceil() as i64..=(nhi / between).floor() as i64 {
            let off = k as f64 * between;
            let at = |t: f64| [nrm[0] * off + d[0] * t, nrm[1] * off + d[1] * t];
            if runs(crop) {
                // Walk along the row in half metres: runs of free cells.
                let step = 0.5;
                let mut start: Option<f64> = None;
                let mut t = tlo;
                while t <= thi + step {
                    let ok = t <= thi && inside(at(t));
                    match (ok, start) {
                        (true, None) => start = Some(t),
                        (false, Some(s)) => {
                            let e = t - step;
                            if e - s >= 1.5 {
                                let (a, b) = (at(s), at(e));
                                out_runs.push(RowRun {
                                    crop,
                                    a: [a[0] as f32, a[1] as f32],
                                    b: [b[0] as f32, b[1] as f32],
                                });
                            }
                            start = None;
                        }
                        _ => {}
                    }
                    t += step;
                }
            } else if let Some(species) = plant_species(crop, region) {
                let along = f64::from(spec.along_m);
                for j in (tlo / along).ceil() as i64..=(thi / along).floor() as i64 {
                    let p = at(j as f64 * along);
                    // The jitter: a little along and across the row.
                    let (h1, h2) = (hash(k, j, 31), hash(k, j, 32));
                    let p = [
                        p[0] + d[0] * (f64::from(h1) - 0.5) * 0.3 * along
                            + nrm[0] * (f64::from(h2) - 0.5) * 0.15 * between,
                        p[1] + d[1] * (f64::from(h1) - 0.5) * 0.3 * along
                            + nrm[1] * (f64::from(h2) - 0.5) * 0.15 * between,
                    ];
                    if hash(k, j, 33) < GAPS || !inside(p) {
                        continue;
                    }
                    let Some(y) = pkg.height(p[0], p[1]) else {
                        continue;
                    };
                    plants.push(RowPlant {
                        pos: [p[0] as f32, y as f32 - 0.1, p[1] as f32],
                        crop,
                        species,
                        scale: 0.85 + 0.3 * hash(k, j, 34),
                        turn: hash(k, j, 35) * std::f32::consts::TAU,
                    });
                }
            }
        }
    }
    (out_runs, plants)
}

/// The mapped trees of a tile on free ground, with their ground height:
/// (foot, the tree).
pub fn mapped_trees(pkg: &WorldPackage, key: TileKey) -> Vec<([f32; 3], PlantedTree)> {
    let Some(v) = pkg.look.vegetation.as_ref().and_then(|v| v.tiles.get(&key)) else {
        return Vec::new();
    };
    v.trees
        .iter()
        .filter_map(|t| {
            let y = pkg.height(f64::from(t.pos[0]), f64::from(t.pos[1]))?;
            Some(([t.pos[0], y as f32 - 0.3, t.pos[1]], *t))
        })
        .collect()
}

/// The 8 m tree-grid cells (global indices, [`crate::trees::TREE_CELL_M`])
/// that hold a mapped tree: the tree grid leaves them empty.
pub fn mapped_cells(pkg: &WorldPackage, key: TileKey) -> Vec<(i64, i64)> {
    let Some(v) = pkg.look.vegetation.as_ref().and_then(|v| v.tiles.get(&key)) else {
        return Vec::new();
    };
    let s = crate::trees::TREE_CELL_M;
    let mut out: Vec<(i64, i64)> = v
        .trees
        .iter()
        .map(|t| {
            (
                (f64::from(t.pos[0]) / s).floor() as i64,
                (f64::from(t.pos[1]) / s).floor() as i64,
            )
        })
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// Whether the tree grid leaves the place (x, z) to a plantation: cells
/// planted in rows (orchards, bananas, avocados, vineyards).
pub fn planted(pkg: &WorldPackage, x: f64, z: f64) -> bool {
    let Some(v) = pkg.look.vegetation.as_ref() else {
        return false;
    };
    let (crop, rows) = v.at(f64::from(pkg.tile_size_m), x, z);
    rows.is_some() && (runs(crop) || plant_species(crop, FloraRegion::Other).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::RoadStrip;
    use crate::vegetation::{NO_ROWS, Vegetation, encode_direction};
    use std::collections::BTreeMap;

    /// A flat 1 km tile at Vaduz (47.14 N, 9.52 E) with land use, a
    /// vineyard in the west half (cells c < 50) with rows along +z, an
    /// orchard in the north-east quarter with rows at 30°, and a road along
    /// x = 250 through the vineyard.
    fn package(with_road: bool) -> WorldPackage {
        let key = TileKey { x: 0, z: 0 };
        let n = 11u32;
        let roads = if with_road {
            let class = crate::graph::RoadClass::Residential;
            vec![RoadStrip {
                class,
                width_m: 4.0,
                points: vec![[250.0, 100.0, 0.0], [250.0, 100.0, 1000.0]],
                surface: crate::RoadSurface::of_class(class),
                dress: crate::RoadDress::of_class(class, 4.0),
                lit: Default::default(),
                condition: Default::default(),
            }]
        } else {
            Vec::new()
        };
        let tile = Tile {
            key,
            heights: vec![100.0; (n * n) as usize],
            roads,
        };
        let cells = 100usize;
        let mut crops = vec![0u8; cells * cells];
        let mut rows = vec![NO_ROWS; cells * cells];
        for r in 0..cells {
            for c in 0..cells {
                let i = r * cells + c;
                if c < 50 {
                    crops[i] = FieldCrop::Vineyard.code();
                    rows[i] = encode_direction(std::f32::consts::FRAC_PI_2);
                } else if r >= 50 {
                    crops[i] = FieldCrop::Orchard.code();
                    rows[i] = encode_direction(30f32.to_radians());
                } else {
                    crops[i] = FieldCrop::Maize.code();
                }
            }
        }
        let mut pkg = WorldPackage {
            name: "Rows".into(),
            origin: (47.14, 9.52),
            tile_size_m: 1000.0,
            samples: n,
            tiles: BTreeMap::from([(key, tile)]),
            graph: None,
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
        };
        pkg.look.vegetation = Some(Vegetation {
            cells: cells as u32,
            sources: Vec::new(),
            tiles: BTreeMap::from([(
                key,
                VegetationTile {
                    crops,
                    rows,
                    trees: vec![PlantedTree {
                        pos: [700.0, 200.0],
                        species: Species::Lime,
                        height_m: 0.0,
                        source: crate::vegetation::TreeSource::Tree,
                    }],
                },
            )]),
        });
        pkg
    }

    #[test]
    fn vineyard_rows_follow_the_direction_and_spacing() {
        let pkg = package(false);
        let (runs, plants) = tile_rows(&pkg, TileKey { x: 0, z: 0 });
        let vines: Vec<&RowRun> = runs
            .iter()
            .filter(|r| r.crop == FieldCrop::Vineyard)
            .collect();
        // Rows along +z every 2 m across the 500 m wide vineyard.
        assert!((240..=252).contains(&vines.len()), "{} rows", vines.len());
        for r in &vines {
            assert!((r.a[0] - r.b[0]).abs() < 0.05, "along +z: {r:?}");
            assert!(r.a[0] >= 0.0 && r.a[0] <= 500.0);
            assert!((r.b[1] - r.a[1]).abs() > 990.0, "{r:?}");
        }
        let mut xs: Vec<f32> = vines.iter().map(|r| r.a[0]).collect();
        xs.sort_by(f32::total_cmp);
        for w in xs.windows(2) {
            assert!((w[1] - w[0] - 2.0).abs() < 0.05, "{w:?}");
        }
        // The orchard: trees in rows at 30°, about 7 × 5.5 m.
        let trees: Vec<&RowPlant> = plants
            .iter()
            .filter(|p| p.crop == FieldCrop::Orchard)
            .collect();
        let area = 500.0 * 500.0;
        let want = area / (7.0 * 5.5) * (1.0 - GAPS);
        assert!(
            (trees.len() as f32 - want).abs() < want * 0.06,
            "{} trees, want {want}",
            trees.len()
        );
        assert!(trees.iter().all(|t| t.species == Species::FruitTree));
        assert!(trees.iter().all(|t| t.pos[0] >= 500.0 && t.pos[2] >= 500.0));
        // Each tree lies near its row line (dot(p, n) = k · 7 m).
        let th = f32::from(encode_direction(30f32.to_radians())) / 256.0 * std::f32::consts::PI;
        let nrm = [-th.sin(), th.cos()];
        for t in &trees {
            let off = t.pos[0] * nrm[0] + t.pos[2] * nrm[1];
            let k = (off / 7.0).round();
            assert!((off - k * 7.0).abs() < 0.6, "{off}");
        }
        // Maize is not planted in rows here; deterministic.
        assert!(plants.iter().all(|p| p.crop != FieldCrop::Maize));
        assert_eq!(
            tile_rows(&pkg, TileKey { x: 0, z: 0 }),
            (runs.clone(), plants.clone())
        );
        // Another tile: nothing.
        assert_eq!(
            tile_rows(&pkg, TileKey { x: 5, z: 0 }),
            (Vec::new(), Vec::new())
        );
    }

    #[test]
    fn rows_stop_at_roads() {
        let pkg = package(true);
        let (runs, _) = tile_rows(&pkg, TileKey { x: 0, z: 0 });
        // No row within the road's half width + clearance of x = 250.
        for r in &runs {
            assert!((r.a[0] - 250.0).abs() > 2.0 + ROAD_CLEARANCE_M, "{r:?}");
        }
        let plain = tile_rows(&package(false), TileKey { x: 0, z: 0 }).0.len();
        assert!(runs.len() < plain);
    }

    #[test]
    fn row_specs_by_region_and_planted_cells() {
        let v = row_spec(FieldCrop::Vineyard, FloraRegion::AlpineRhine).unwrap();
        assert_eq!((v.between_m, v.along_m), (2.0, 1.1));
        let t = row_spec(FieldCrop::Vineyard, FloraRegion::Tenerife).unwrap();
        assert!(t.between_m > v.between_m);
        let b = row_spec(FieldCrop::Banana, FloraRegion::Tenerife).unwrap();
        let per_ha = 10_000.0 / (b.between_m * b.along_m);
        assert!((1600.0..2200.0).contains(&per_ha), "{per_ha}");
        assert_eq!(row_spec(FieldCrop::Cereal, FloraRegion::BlackForest), None);
        let pkg = package(false);
        assert!(planted(&pkg, 100.0, 100.0));
        assert!(planted(&pkg, 700.0, 700.0));
        assert!(!planted(&pkg, 700.0, 300.0)); // maize
        assert_eq!(mapped_cells(&pkg, TileKey { x: 0, z: 0 }), vec![(87, 25)]);
        let m = mapped_trees(&pkg, TileKey { x: 0, z: 0 });
        assert_eq!(m.len(), 1);
        assert!((m[0].0[1] - 99.7).abs() < 1e-4);
    }
}
