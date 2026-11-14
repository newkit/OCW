//! The measured forest of a region (spec §14 Phase 8 T3): the Copernicus
//! HRL *Dominant Leaf Type* and *Tree Cover Density* rasters that
//! `scripts/fetch-forest.sh` downloads into `data/<region>/forest/`,
//! resampled onto the package's forest grid (section `FRST`,
//! [`ocw_world::forest`]).
//!
//! The folder holds an `index.json`:
//!
//! ```json
//! { "kind": "forest", "crs": "laea", "pixel_m": 10,
//!   "west": 4250000, "north": 2700000, "width": 6000, "height": 4000,
//!   "leaf": "leaf-type.u8", "density": "density.u8",
//!   "sources": [ { "id": …, "name": …, "licence": …, "notice": …,
//!                  "url": …, "date": … } ] }
//! ```
//!
//! and two raw 8-bit rasters of `width × height` pixels of `pixel_m`
//! from the north-west corner (`west`, `north`), rows from the north: the
//! leaf type (0 no trees, 1 broadleaved, 2 coniferous, 254 unclassifiable,
//! 255 outside the coverage) and the tree cover density (0 … 100 %, 254
//! unclassifiable, 255 outside) – the HRL's own codes.
//!
//! Each forest cell (20 m by default) takes 4 × 4 points of the rasters
//! (5 m apart, so every 10 m pixel under the cell counts): the density is
//! their mean, the broadleaf share the broadleaved part of the tree pixels
//! among them. Local metres map to the CRS by the tile's affine fit
//! ([`TileAffine`]).

use std::collections::BTreeMap;
use std::path::Path;

use ocw_world::TileKey;
use ocw_world::WorldPackage;
use ocw_world::forest::{ForestCover, ForestTile, NO_DATA};
use ocw_world::geo::LocalProjection;
use ocw_world::look::ImagerySource;
use serde::Deserialize;

use crate::crs::Crs;
use crate::imagery::TileAffine;

/// HRL Dominant Leaf Type codes.
pub const LEAF_NONE: u8 = 0;
pub const LEAF_BROAD: u8 = 1;
pub const LEAF_CONIFER: u8 = 2;
/// Sub-samples per cell side.
const SUB: usize = 4;

#[derive(Debug, Clone, Deserialize)]
struct SourceEntry {
    id: String,
    name: String,
    licence: String,
    notice: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    date: String,
}

#[derive(Debug, Clone, Deserialize)]
struct IndexFile {
    kind: String,
    crs: String,
    pixel_m: f64,
    west: f64,
    north: f64,
    width: usize,
    height: usize,
    leaf: String,
    density: String,
    sources: Vec<SourceEntry>,
}

/// The leaf type and density rasters of a region.
#[derive(Debug, Clone)]
pub struct ForestRaster {
    pub sources: Vec<ImagerySource>,
    pub crs: Crs,
    pub pixel_m: f64,
    pub west: f64,
    pub north: f64,
    pub width: usize,
    pub height: usize,
    /// HRL leaf type codes, rows from the north.
    pub leaf: Vec<u8>,
    /// Tree cover density (0 … 100, 254 / 255 no data).
    pub density: Vec<u8>,
}

impl ForestRaster {
    /// Reads `dir/index.json` and its rasters.
    pub fn load(dir: &Path) -> Result<Self, String> {
        let p = dir.join("index.json");
        let text = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
        let ix: IndexFile =
            serde_json::from_str(&text).map_err(|e| format!("{}: {e}", p.display()))?;
        if ix.kind != "forest" {
            return Err(format!("{}: kind {} is not forest", p.display(), ix.kind));
        }
        let crs = Crs::parse(&ix.crs)?;
        let read = |name: &str| -> Result<Vec<u8>, String> {
            let path = dir.join(name);
            let b = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            if b.len() != ix.width * ix.height {
                return Err(format!(
                    "{}: {} bytes, expected {} × {}",
                    path.display(),
                    b.len(),
                    ix.width,
                    ix.height
                ));
            }
            Ok(b)
        };
        let resolution_m = ix.pixel_m as f32;
        Ok(Self {
            sources: ix
                .sources
                .iter()
                .map(|s| ImagerySource {
                    id: s.id.clone(),
                    name: s.name.clone(),
                    licence: s.licence.clone(),
                    notice: s.notice.clone(),
                    url: s.url.clone(),
                    date: s.date.clone(),
                    resolution_m,
                })
                .collect(),
            crs,
            pixel_m: ix.pixel_m,
            west: ix.west,
            north: ix.north,
            width: ix.width,
            height: ix.height,
            leaf: read(&ix.leaf)?,
            density: read(&ix.density)?,
        })
    }

    /// The (leaf type, density) pixel at CRS `(e, n)`, `None` outside the
    /// raster.
    pub fn pixel(&self, e: f64, n: f64) -> Option<(u8, u8)> {
        let fx = (e - self.west) / self.pixel_m;
        let fy = (self.north - n) / self.pixel_m;
        if !(fx >= 0.0 && fy >= 0.0) {
            return None;
        }
        let (x, y) = (fx as usize, fy as usize);
        if x >= self.width || y >= self.height {
            return None;
        }
        let i = y * self.width + x;
        Some((self.leaf[i], self.density[i]))
    }
}

/// One cell's samples → (density, broadleaf share), each [`NO_DATA`] when
/// nothing valid.
pub fn aggregate(samples: impl IntoIterator<Item = (u8, u8)>) -> (u8, u8) {
    let (mut n, mut sum, mut broad, mut trees) = (0u32, 0u32, 0u32, 0u32);
    for (leaf, dens) in samples {
        if dens <= 100 {
            n += 1;
            sum += u32::from(dens);
        }
        match leaf {
            LEAF_BROAD => {
                broad += 1;
                trees += 1;
            }
            LEAF_CONIFER => trees += 1,
            _ => {}
        }
    }
    let density = (sum + n / 2).checked_div(n).map_or(NO_DATA, |d| d as u8);
    let share = (100 * broad + trees / 2)
        .checked_div(trees)
        .map_or(NO_DATA, |s| s as u8);
    (density, share)
}

/// The forest planes of one tile (`None` when the raster has nothing
/// there).
pub fn tile_forest(
    r: &ForestRaster,
    proj: &LocalProjection,
    key: TileKey,
    size: f64,
    cells: u32,
) -> Option<ForestTile> {
    let aff = TileAffine::new(proj, r.crs, key, size);
    let n = cells as usize;
    let step = size / f64::from(cells);
    let o = key.origin(size);
    let mut density = vec![NO_DATA; n * n];
    let mut broadleaf = vec![NO_DATA; n * n];
    let mut any = false;
    for j in 0..n * n {
        let (c, row) = (j % n, j / n);
        let samples = (0..SUB * SUB).filter_map(|s| {
            let p = [
                o[0] + (c as f64 + ((s % SUB) as f64 + 0.5) / SUB as f64) * step,
                o[1] + (row as f64 + ((s / SUB) as f64 + 0.5) / SUB as f64) * step,
            ];
            let [e, nn] = aff.apply(p);
            r.pixel(e, nn)
        });
        let (d, b) = aggregate(samples);
        any |= d != NO_DATA;
        density[j] = d;
        broadleaf[j] = b;
    }
    any.then_some(ForestTile { density, broadleaf })
}

/// What [`add_forest`] did.
#[derive(Debug, Clone, Default)]
pub struct ForestStats {
    pub tiles: usize,
    pub tiles_outside: usize,
    /// Cells with data, their mean density and density-weighted
    /// broadleaf share.
    pub cells: usize,
    pub mean_density: f64,
    pub broadleaf_share: Option<f64>,
    /// Encoded size of the section.
    pub bytes: usize,
}

/// Measures the forest of every tile of `pkg` in `r` and stores it in the
/// package (`pkg.look.forest`, replacing one it has; nothing when the
/// raster covers no tile).
pub fn add_forest(pkg: &mut WorldPackage, r: &ForestRaster, cells: u32) -> ForestStats {
    let size = f64::from(pkg.tile_size_m);
    let proj = LocalProjection::new(pkg.origin.0, pkg.origin.1);
    let keys: Vec<TileKey> = pkg.tiles.keys().copied().collect();
    let tiles = crate::par::map(&keys, crate::par::threads(), |key| {
        tile_forest(r, &proj, *key, size, cells)
    });
    let mut st = ForestStats::default();
    let mut out = BTreeMap::new();
    for (key, t) in keys.into_iter().zip(tiles) {
        match t {
            Some(t) => {
                out.insert(key, t);
            }
            None => st.tiles_outside += 1,
        }
    }
    st.tiles = out.len();
    if out.is_empty() {
        pkg.look.forest = None;
        return st;
    }
    let cover = ForestCover {
        cells,
        sources: r.sources.clone(),
        tiles: out,
    };
    (st.cells, st.mean_density, st.broadleaf_share) = cover.stats();
    let mut b = Vec::new();
    ocw_world::forest::write_forest(&cover, &mut b);
    st.bytes = b.len();
    pkg.look.forest = Some(cover);
    st
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/forest")
    }

    #[test]
    fn cells_average_density_and_count_leaf_types() {
        // Two conifer pixels, one broadleaf, one without trees.
        let (d, b) = aggregate([(2, 80), (2, 90), (1, 70), (0, 0)]);
        assert_eq!((d, b), (60, 33));
        // No trees: density 0, no share.
        assert_eq!(aggregate([(0, 0), (0, 0)]), (0, NO_DATA));
        // Outside the coverage and unclassified: no data.
        assert_eq!(aggregate([(255, 255), (254, 254)]), (NO_DATA, NO_DATA));
        assert_eq!(aggregate([]), (NO_DATA, NO_DATA));
        // Unclassified pixels do not count.
        assert_eq!(aggregate([(254, 254), (1, 50)]), (50, 100));
    }

    /// The recorded fixture: a 600 m × 400 m cut of the HRL 2018 rasters
    /// south-east of Vaduz (LAEA, 10 m; see its manifest).
    #[test]
    fn the_fixture_loads_and_samples_by_crs() {
        let r = ForestRaster::load(&fixture()).unwrap();
        assert_eq!(r.crs, Crs::Laea);
        assert_eq!((r.width, r.height), (60, 40));
        assert!(
            r.sources
                .iter()
                .any(|s| s.notice.contains("Copernicus Land Monitoring Service"))
        );
        // Inside: valid codes; outside: none.
        let (e, n) = (r.west + 5.0, r.north - 5.0);
        let (leaf, dens) = r.pixel(e, n).unwrap();
        assert!(leaf <= 2 && dens <= 100, "{leaf} {dens}");
        assert_eq!(r.pixel(r.west - 1.0, n), None);
        assert_eq!(r.pixel(e, r.north + 1.0), None);
        assert_eq!(r.pixel(r.west + 600.5, n), None);
        // It holds woods of both types and open land.
        assert!(r.leaf.contains(&LEAF_BROAD) && r.leaf.contains(&LEAF_CONIFER));
        assert!(r.leaf.contains(&LEAF_NONE));
    }

    #[test]
    fn a_tile_over_the_fixture_gets_its_forest() {
        let r = ForestRaster::load(&fixture()).unwrap();
        // A package whose origin is the fixture's centre: tile (0, 0) and
        // its west neighbour cover parts of it, a far tile nothing.
        let (cx, cy) = (r.west + 300.0, r.north - 200.0);
        let (lat, lon) = Crs::Laea.inverse(cx, cy);
        let proj = LocalProjection::new(lat, lon);
        let t = tile_forest(&r, &proj, TileKey { x: 0, z: 0 }, 1000.0, 50).unwrap();
        // Local +z is north: the fixture reaches 200 m north and 300 m east
        // of the origin, so 10 × 15 cells of 20 m have data.
        let with: usize = t.density.iter().filter(|d| **d != NO_DATA).count();
        assert!((130..=180).contains(&with), "{with}");
        assert!(t.density.iter().all(|d| *d <= 100 || *d == NO_DATA));
        assert!(t.broadleaf.iter().any(|b| *b <= 100));
        assert!(tile_forest(&r, &proj, TileKey { x: 5, z: 5 }, 1000.0, 50).is_none());
        // The cell at the origin agrees with the raster pixels around it.
        let [e, n] =
            TileAffine::new(&proj, Crs::Laea, TileKey { x: 0, z: 0 }, 1000.0).apply([10.0, 10.0]);
        let px: Vec<(u8, u8)> = (0..4)
            .flat_map(|i| (0..4).map(move |k| (i, k)))
            .filter_map(|(i, k)| {
                r.pixel(e - 7.5 + 5.0 * f64::from(i), n - 7.5 + 5.0 * f64::from(k))
            })
            .collect();
        let (d, _) = aggregate(px);
        assert!(
            (i32::from(t.density[0]) - i32::from(d)).abs() <= 10,
            "{} vs {d}",
            t.density[0]
        );
        // Deterministic.
        assert_eq!(
            Some(t),
            tile_forest(&r, &proj, TileKey { x: 0, z: 0 }, 1000.0, 50)
        );
    }
}
