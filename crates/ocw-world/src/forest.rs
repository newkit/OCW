//! The measured forest of a region (spec §14 Phase 8 T3, §4.6): the
//! dominant leaf type (broadleaved / coniferous) and the tree cover density
//! of the Copernicus High Resolution Layers (HRL *Dominant Leaf Type* and
//! *Tree Cover Density*, reference year 2018, 10 m), resampled by the world
//! builder onto a coarser grid per tile. Which trees grow where: the
//! renderer takes each tree's leaf type from it
//! ([`crate::trees::measured_leaf`]), and the tree placement thins or fills
//! the land use's chance by the measured density
//! ([`crate::trees::chance_with_density`]) – OSM keeps the outlines of the
//! woods, the density their spacing.
//!
//! Optional package section `FRST` (spec §4.7; readers that do not know it
//! skip it, a package without it keeps the regional fallback mix
//! `ocw_sky::season::LeafMix` and the land use's chances):
//!
//! ```text
//! layout version: u32 (1)
//! cells per tile side: u32
//! source count: u8, then per source: id, name, licence, notice, url,
//!   date (each u32 length + UTF-8), resolution (m): f32
//! tile count: u32, then per tile: x, z: i32, length: u32 + bytes
//! ```
//!
//! Each tile's bytes are two `cells × cells` planes, row-major along +z
//! like the land use – the **density** (tree cover 0 … 100 %, 255 no data)
//! and the **broadleaf share** (of the measured tree pixels in the cell,
//! 0 … 100 %, 255 none: no trees measured there or no data) – each
//! predicted from its neighbours (LOCO-I, as the tint maps) and the
//! residuals deflated together ([`encode_tile`]). Cell `(c, r)` covers
//! local metres `[o.x + c·s, o.x + (c+1)·s] × [o.z + r·s, o.z + (r+1)·s]`
//! with `s = tile size / cells`.

use std::collections::BTreeMap;

use crate::look::{ImagerySource, check_version, get_str, predict, put_str, unpredict};
use crate::package::{Bytes, TileKey};

/// Section tag of the forest cover.
pub const FOREST_TAG: &[u8; 4] = b"FRST";
const FOREST_VERSION: u32 = 1;
/// Cells per tile side the world builder writes by default (20 m: the
/// 10 m HRL pixels in 2 × 2, finer than the woods' patches and coarser
/// than the 8 m tree grid).
pub const DEFAULT_CELLS: u32 = 50;
/// No data in a plane.
pub const NO_DATA: u8 = 255;

/// The forest of one cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForestCell {
    /// Tree cover density, 0 … 100 %.
    pub density: u8,
    /// Share of broadleaved trees among the measured tree pixels,
    /// 0 … 100 % (`None`: no trees measured in the cell).
    pub broadleaf: Option<u8>,
}

/// The two planes of one tile (`cells × cells` each, [`NO_DATA`] where the
/// source has nothing).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForestTile {
    pub density: Vec<u8>,
    pub broadleaf: Vec<u8>,
}

impl ForestTile {
    /// Cell `index` (row-major along +z); `None` without density data.
    pub fn get(&self, index: usize) -> Option<ForestCell> {
        let d = *self.density.get(index)?;
        if d > 100 {
            return None;
        }
        let b = self.broadleaf.get(index).copied().filter(|b| *b <= 100);
        Some(ForestCell {
            density: d,
            broadleaf: b,
        })
    }

    /// The cell holding local `(x, z)` of tile `key` (`None` outside the
    /// tile or without data).
    pub fn at(
        &self,
        key: TileKey,
        tile_size: f64,
        cells: u32,
        x: f64,
        z: f64,
    ) -> Option<ForestCell> {
        let o = key.origin(tile_size);
        let s = tile_size / f64::from(cells);
        let (c, r) = ((x - o[0]) / s, (z - o[1]) / s);
        let n = f64::from(cells);
        if !(c >= 0.0 && r >= 0.0 && c < n + 1e-9 && r < n + 1e-9) {
            return None;
        }
        let (c, r) = ((c as u32).min(cells - 1), (r as u32).min(cells - 1));
        self.get((r * cells + c) as usize)
    }
}

/// The forest cover of a package (section `FRST`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ForestCover {
    /// Cells per tile side.
    pub cells: u32,
    /// The sources (leaf type, density) with their notices.
    pub sources: Vec<ImagerySource>,
    /// Per tile its planes (tiles without any data are left out).
    pub tiles: BTreeMap<TileKey, ForestTile>,
}

impl ForestCover {
    /// The cell at local `(x, z)` (package metres).
    pub fn cell(&self, tile_size: f64, x: f64, z: f64) -> Option<ForestCell> {
        let key = TileKey::containing([x, z], tile_size);
        self.tiles.get(&key)?.at(key, tile_size, self.cells, x, z)
    }

    /// Mean density (0 … 1) and broadleaf share (0 … 1, weighted by the
    /// density) of the cells whose centres lie in the box `lo` … `hi`
    /// (local metres); `None` where no cell has data.
    pub fn mean_in(
        &self,
        tile_size: f64,
        lo: [f64; 2],
        hi: [f64; 2],
    ) -> Option<(f32, Option<f32>)> {
        let s = tile_size / f64::from(self.cells);
        let (mut n, mut dens, mut wb, mut b) = (0u32, 0.0f32, 0.0f32, 0.0f32);
        let first = |v: f64| ((v / s) - 0.5).ceil() as i64;
        let last = |v: f64| ((v / s) - 0.5).floor() as i64;
        for gz in first(lo[1])..=last(hi[1]) {
            for gx in first(lo[0])..=last(hi[0]) {
                let (x, z) = ((gx as f64 + 0.5) * s, (gz as f64 + 0.5) * s);
                let Some(c) = self.cell(tile_size, x, z) else {
                    continue;
                };
                let d = f32::from(c.density) / 100.0;
                n += 1;
                dens += d;
                if let Some(share) = c.broadleaf {
                    let w = d.max(0.01);
                    wb += w;
                    b += w * f32::from(share) / 100.0;
                }
            }
        }
        (n > 0).then(|| (dens / n as f32, (wb > 0.0).then(|| b / wb)))
    }

    /// Cells with data and the density-weighted broadleaf share over all
    /// of them, for reports.
    pub fn stats(&self) -> (usize, f64, Option<f64>) {
        let (mut n, mut dens, mut wb, mut b) = (0usize, 0.0f64, 0.0f64, 0.0f64);
        for t in self.tiles.values() {
            for i in 0..t.density.len() {
                if let Some(c) = t.get(i) {
                    n += 1;
                    let d = f64::from(c.density) / 100.0;
                    dens += d;
                    if let Some(s) = c.broadleaf {
                        wb += d;
                        b += d * f64::from(s) / 100.0;
                    }
                }
            }
        }
        (
            n,
            if n > 0 { dens / n as f64 } else { 0.0 },
            (wb > 0.0).then(|| b / wb),
        )
    }
}

/// Encodes a tile's planes (each `cells²`): LOCO-I residuals of both
/// planes, deflated.
pub fn encode_tile(t: &ForestTile, cells: u32) -> Vec<u8> {
    let n = cells as usize;
    assert!(
        t.density.len() == n * n && t.broadleaf.len() == n * n,
        "forest planes size"
    );
    let mut raw = predict(&t.density, n);
    raw.extend(predict(&t.broadleaf, n));
    miniz_oxide::deflate::compress_to_vec(&raw, 9)
}

/// Decodes [`encode_tile`]'s bytes.
pub fn decode_tile(bytes: &[u8], cells: u32) -> Result<ForestTile, String> {
    let n = cells as usize;
    let raw = miniz_oxide::inflate::decompress_to_vec_with_limit(bytes, 2 * n * n)
        .map_err(|e| format!("forest tile: {e:?}"))?;
    if raw.len() != 2 * n * n {
        return Err("forest tile: wrong size".into());
    }
    Ok(ForestTile {
        density: unpredict(&raw[..n * n], n),
        broadleaf: unpredict(&raw[n * n..], n),
    })
}

/// Writes the `FRST` payload.
pub fn write_forest(f: &ForestCover, b: &mut Vec<u8>) {
    b.extend(FOREST_VERSION.to_le_bytes());
    b.extend(f.cells.to_le_bytes());
    b.push(f.sources.len() as u8);
    for s in &f.sources {
        for t in [&s.id, &s.name, &s.licence, &s.notice, &s.url, &s.date] {
            put_str(b, t);
        }
        b.extend(s.resolution_m.to_le_bytes());
    }
    b.extend((f.tiles.len() as u32).to_le_bytes());
    for (k, t) in &f.tiles {
        b.extend(k.x.to_le_bytes());
        b.extend(k.z.to_le_bytes());
        let enc = encode_tile(t, f.cells);
        b.extend((enc.len() as u32).to_le_bytes());
        b.extend(enc);
    }
}

/// Reads the `FRST` payload (the tiles decoded).
pub fn read_forest(bytes: &[u8]) -> Result<ForestCover, String> {
    let mut r = Bytes::new(bytes);
    check_version(&mut r, FOREST_VERSION, "forest cover")?;
    let cells = r.u32()?;
    if cells == 0 || cells > 1000 {
        return Err(format!("forest cover: {cells} cells per tile"));
    }
    let mut sources = Vec::new();
    for _ in 0..r.u8()? {
        let mut s = [(); 6].map(|_| String::new());
        for t in &mut s {
            *t = get_str(&mut r)?;
        }
        let [id, name, licence, notice, url, date] = s;
        sources.push(ImagerySource {
            id,
            name,
            licence,
            notice,
            url,
            date,
            resolution_m: r.f32()?,
        });
    }
    let mut tiles = BTreeMap::new();
    for _ in 0..r.u32()? {
        let key = TileKey {
            x: r.i32()?,
            z: r.i32()?,
        };
        let n = r.u32()? as usize;
        tiles.insert(key, decode_tile(r.take(n)?, cells)?);
    }
    r.finish("forest cover")?;
    Ok(ForestCover {
        cells,
        sources,
        tiles,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A 4 × 4 tile at (0, 0): conifers in the west half (density 80),
    /// broadleaves in the east (density 60), the south-east cell without
    /// trees, the north-west corner without data.
    pub fn sample_forest() -> ForestCover {
        let n = 4usize;
        let mut density = vec![0u8; n * n];
        let mut broadleaf = vec![NO_DATA; n * n];
        for r in 0..n {
            for c in 0..n {
                let i = r * n + c;
                if c < 2 {
                    density[i] = 80;
                    broadleaf[i] = 0;
                } else {
                    density[i] = 60;
                    broadleaf[i] = 100;
                }
            }
        }
        density[n - 1] = 0; // r 0, c 3
        broadleaf[n - 1] = NO_DATA;
        density[(n - 1) * n] = NO_DATA; // r 3, c 0
        broadleaf[(n - 1) * n] = NO_DATA;
        ForestCover {
            cells: n as u32,
            sources: vec![ImagerySource {
                id: "hrl-dlt-2018".into(),
                name: "Copernicus HRL Dominant Leaf Type 2018".into(),
                licence: "Copernicus data (free, full and open; attribution)".into(),
                notice: "© European Union, Copernicus Land Monitoring Service 2018, European Environment Agency (EEA)".into(),
                url: "https://land.copernicus.eu".into(),
                date: "2018".into(),
                resolution_m: 10.0,
            }],
            tiles: BTreeMap::from([(TileKey { x: 0, z: 0 }, ForestTile { density, broadleaf })]),
        }
    }

    #[test]
    fn section_round_trips_and_rejects_garbage() {
        let f = sample_forest();
        let mut b = Vec::new();
        write_forest(&f, &mut b);
        assert_eq!(read_forest(&b).unwrap(), f);
        // A newer layout is "not supported" (the package reader skips it).
        let mut newer = b.clone();
        newer[0] = 9;
        assert!(read_forest(&newer).unwrap_err().contains("not supported"));
        // Truncated: an error, not a panic.
        assert!(read_forest(&b[..b.len() - 3]).is_err());
        assert!(decode_tile(&[1, 2, 3], 4).is_err());
    }

    #[test]
    fn noisy_planes_round_trip_exactly_and_compress() {
        let n = 50usize;
        let mut density = Vec::with_capacity(n * n);
        let mut broadleaf = Vec::with_capacity(n * n);
        for i in 0..n * n {
            let (r, c) = (i / n, i % n);
            let h = crate::trees::hash(c as i64, r as i64, 99);
            // A wood in the north with noise, a meadow in the south.
            if r < 30 {
                density.push((70.0 + 25.0 * h) as u8);
                broadleaf.push(if c < 20 { 0 } else { (h * 100.0) as u8 });
            } else {
                density.push(0);
                broadleaf.push(NO_DATA);
            }
        }
        let t = ForestTile { density, broadleaf };
        let enc = encode_tile(&t, n as u32);
        assert_eq!(decode_tile(&enc, n as u32).unwrap(), t);
        assert!(enc.len() < n * n, "{} bytes for {} cells", enc.len(), n * n);
    }

    #[test]
    fn cells_are_found_by_local_metres() {
        let f = sample_forest();
        // 1 km tile, 250 m cells.
        let c = f.cell(1000.0, 100.0, 300.0).unwrap();
        assert_eq!((c.density, c.broadleaf), (80, Some(0)));
        let c = f.cell(1000.0, 900.0, 300.0).unwrap();
        assert_eq!((c.density, c.broadleaf), (60, Some(100)));
        // The cell without trees: density 0, no leaf type.
        let c = f.cell(1000.0, 900.0, 100.0).unwrap();
        assert_eq!((c.density, c.broadleaf), (0, None));
        // No data, another tile, the far border.
        assert_eq!(f.cell(1000.0, 100.0, 900.0), None);
        assert_eq!(f.cell(1000.0, 1500.0, 300.0), None);
        assert_eq!(f.cell(1000.0, -1.0, 300.0), None);
        assert!(f.cell(1000.0, 999.99, 300.0).is_some());
    }

    #[test]
    fn means_weight_the_leaf_share_by_density() {
        let f = sample_forest();
        // The whole middle row r 1: two conifer cells (80), two broadleaf (60).
        let (d, b) = f.mean_in(1000.0, [0.0, 250.0], [1000.0, 500.0]).unwrap();
        assert!((d - 0.7).abs() < 1e-6, "{d}");
        let b = b.unwrap();
        assert!((b - 120.0 / 280.0).abs() < 1e-5, "{b}");
        // Only the treeless cell: density 0, no share.
        let (d, b) = f.mean_in(1000.0, [760.0, 10.0], [990.0, 240.0]).unwrap();
        assert_eq!((d, b), (0.0, None));
        // Outside: nothing.
        assert_eq!(f.mean_in(1000.0, [2000.0, 0.0], [2500.0, 500.0]), None);
        let (n, mean, share) = f.stats();
        assert_eq!(n, 15);
        assert!(mean > 0.5 && share.unwrap() > 0.3 && share.unwrap() < 0.5);
    }
}
