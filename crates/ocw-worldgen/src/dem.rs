//! Digital elevation models (spec §4.3): SRTM `.hgt` tiles and the grids
//! of high-resolution official DTMs (GeoTIFF, XYZ), best source first.
//!
//! An HGT tile covers 1° × 1° from its south-west corner (`N47E009`):
//! `n × n` big-endian `i16` heights in metres, rows from north to south,
//! columns from west to east; neighbouring tiles share their edge rows.
//! `-32768` marks voids. 1 arc-second tiles have n = 3601, 3 arc-second
//! tiles n = 1201.
//!
//! **Grids** ([`GridTile`], [`GridLayer`]) are tiles of a projected DTM in
//! its national coordinates ([`crate::crs`]): LGL DGM1 (1 m, delivered as
//! XYZ text, imported to 2 m GeoTIFFs by `world-builder dem-import`),
//! swissALTI3D (2 m cloud-optimised GeoTIFF). [`Dem::elevation`] samples
//! the first grid layer that covers a point and **feathers** it into the
//! next source over [`FEATHER_M`] at the edge of its coverage (a state
//! border, the end of the downloaded tiles, voids), so a 1 m bare-earth
//! model and SRTM's 30 m surface model meet without a cliff.
use std::path::Path;

pub use crate::dtm::{GridLayer, GridTile};

const VOID: i16 = -32768;

#[derive(Debug, Clone)]
pub struct HgtTile {
    pub south: i32,
    pub west: i32,
    pub size: usize,
    data: Vec<i16>,
}

#[derive(Debug, thiserror::Error)]
pub enum DemError {
    #[error("not an HGT tile name: {0} (expected e.g. N47E009.hgt)")]
    Name(String),
    #[error("HGT data has {0} bytes, not n × n × 2")]
    Size(usize),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    /// A grid file that cannot be used (format, georeference, alignment).
    #[error("{0}")]
    Format(String),
}

/// South-west corner from a tile name such as `N47E009` or `S12W077`.
pub fn parse_tile_name(name: &str) -> Option<(i32, i32)> {
    let name = name.get(..7)?;
    let lat: i32 = name.get(1..3)?.parse().ok()?;
    let lon: i32 = name.get(4..7)?.parse().ok()?;
    let lat = match &name[..1] {
        "N" => lat,
        "S" => -lat,
        _ => return None,
    };
    let lon = match &name[3..4] {
        "E" => lon,
        "W" => -lon,
        _ => return None,
    };
    Some((lat, lon))
}

impl HgtTile {
    pub fn from_bytes(south: i32, west: i32, bytes: &[u8]) -> Result<Self, DemError> {
        let samples = bytes.len() / 2;
        let size = (samples as f64).sqrt().round() as usize;
        if size < 2 || size * size * 2 != bytes.len() {
            return Err(DemError::Size(bytes.len()));
        }
        let data = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_be_bytes(*b))
            .collect();
        Ok(Self {
            south,
            west,
            size,
            data,
        })
    }

    pub fn read(path: &Path) -> Result<Self, DemError> {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        let (south, west) =
            parse_tile_name(name).ok_or_else(|| DemError::Name(name.to_string()))?;
        Self::from_bytes(south, west, &std::fs::read(path)?)
    }

    pub fn contains(&self, lat: f64, lon: f64) -> bool {
        (f64::from(self.south)..=f64::from(self.south) + 1.0).contains(&lat)
            && (f64::from(self.west)..=f64::from(self.west) + 1.0).contains(&lon)
    }

    fn at(&self, row: usize, col: usize) -> Option<f64> {
        let v = self.data[row * self.size + col];
        (v != VOID).then_some(f64::from(v))
    }

    /// Bilinear elevation at (lat, lon); voids are skipped (the valid
    /// corners are averaged with their weights). `None` outside the tile
    /// or if all four corners are voids.
    pub fn sample(&self, lat: f64, lon: f64) -> Option<f64> {
        if !self.contains(lat, lon) {
            return None;
        }
        let n = (self.size - 1) as f64;
        let fy = (f64::from(self.south) + 1.0 - lat) * n; // rows from the north
        let fx = (lon - f64::from(self.west)) * n;
        let (r, c) = (
            (fy.floor() as usize).min(self.size - 2),
            (fx.floor() as usize).min(self.size - 2),
        );
        let (ty, tx) = (fy - r as f64, fx - c as f64);
        let corners = [
            (self.at(r, c), (1.0 - tx) * (1.0 - ty)),
            (self.at(r, c + 1), tx * (1.0 - ty)),
            (self.at(r + 1, c), (1.0 - tx) * ty),
            (self.at(r + 1, c + 1), tx * ty),
        ];
        let (sum, weight) = corners
            .iter()
            .filter_map(|(v, w)| v.map(|v| (v * w, *w)))
            .fold((0.0, 0.0), |a, b| (a.0 + b.0, a.1 + b.1));
        if weight > 1e-12 {
            Some(sum / weight)
        } else {
            // On a void corner exactly: any valid corner.
            corners.iter().find_map(|(v, _)| *v)
        }
    }
}

/// Elevation sources, best first: the grid layers of high-resolution
/// DTMs in the order they were added, then the SRTM tiles.
#[derive(Debug, Clone, Default)]
pub struct Dem {
    pub tiles: Vec<HgtTile>,
    pub grids: Vec<GridLayer>,
}

/// Width of the band at the edge of a grid layer's coverage in which it is
/// blended into the next source (m).
pub const FEATHER_M: f64 = 300.0;

impl Dem {
    /// SRTM tiles only.
    pub fn from_hgt(tiles: Vec<HgtTile>) -> Self {
        Self {
            tiles,
            grids: Vec::new(),
        }
    }

    /// Adds a grid tile to the layer of its CRS, cell size and tile size
    /// (a new layer, after the existing ones, if there is none yet).
    /// Call [`Dem::finish`] when all tiles are in.
    pub fn add_grid(&mut self, tile: GridTile) -> Result<(), DemError> {
        match self.grids.iter_mut().find(|l| l.accepts(&tile)) {
            Some(layer) => layer.insert(tile),
            None => {
                let mut layer = GridLayer::new(&tile);
                layer.insert(tile)?;
                self.grids.push(layer);
                Ok(())
            }
        }
    }

    /// Computes the coverage of every grid layer (for the feathering).
    pub fn finish(&mut self) {
        for l in &mut self.grids {
            l.finish(FEATHER_M);
        }
    }

    /// Elevation at (lat, lon): the grid layers in order, each blended
    /// into the rest by its coverage weight, then SRTM.
    pub fn elevation(&self, lat: f64, lon: f64) -> Option<f64> {
        self.elevation_from(0, lat, lon)
    }

    fn elevation_from(&self, layer: usize, lat: f64, lon: f64) -> Option<f64> {
        let Some(l) = self.grids.get(layer) else {
            return self.tiles.iter().find_map(|t| t.sample(lat, lon));
        };
        let p = l.crs().forward(lat, lon);
        let w = l.weight(p[0], p[1]);
        let here = (w > 0.0).then(|| l.sample(p[0], p[1])).flatten();
        match here {
            Some(h) if w >= 1.0 => Some(h),
            Some(h) => match self.elevation_from(layer + 1, lat, lon) {
                Some(rest) => Some(w * h + (1.0 - w) * rest),
                None => Some(h),
            },
            None => self.elevation_from(layer + 1, lat, lon),
        }
    }

    /// Which source answers at (lat, lon): the index of the first grid
    /// layer with full weight, `grids.len()` for SRTM or a blend.
    pub fn source_at(&self, lat: f64, lon: f64) -> usize {
        self.grids
            .iter()
            .position(|l| {
                let p = l.crs().forward(lat, lon);
                l.weight(p[0], p[1]) >= 1.0 && l.sample(p[0], p[1]).is_some()
            })
            .unwrap_or(self.grids.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 3 × 3 tile: heights 100·row + 10·col (row 0 = north).
    fn tile() -> HgtTile {
        let mut bytes = Vec::new();
        for r in 0..3i16 {
            for c in 0..3i16 {
                bytes.extend((100 * r + 10 * c).to_be_bytes());
            }
        }
        HgtTile::from_bytes(47, 9, &bytes).unwrap()
    }

    #[test]
    fn tile_names() {
        assert_eq!(parse_tile_name("N47E009.hgt"), Some((47, 9)));
        assert_eq!(parse_tile_name("S12W077"), Some((-12, -77)));
        assert_eq!(parse_tile_name("X47E009"), None);
        assert_eq!(parse_tile_name("N4"), None);
    }

    #[test]
    fn sampling_is_bilinear_with_rows_from_the_north() {
        let t = tile();
        assert_eq!(t.size, 3);
        // North-west corner = row 0, col 0.
        assert_eq!(t.sample(48.0, 9.0), Some(0.0));
        // South-east corner = row 2, col 2.
        assert_eq!(t.sample(47.0, 10.0), Some(220.0));
        // Centre = row 1, col 1.
        assert_eq!(t.sample(47.5, 9.5), Some(110.0));
        // Between rows 0 and 1, cols 1 and 2.
        let v = t.sample(47.75, 9.75).unwrap();
        assert!((v - 65.0).abs() < 1e-9, "{v}");
        assert_eq!(t.sample(46.9, 9.5), None);
    }

    #[test]
    fn voids_are_skipped() {
        let mut bytes = Vec::new();
        for v in [10i16, VOID, 30, 40] {
            bytes.extend(v.to_be_bytes());
        }
        let t = HgtTile::from_bytes(0, 0, &bytes).unwrap();
        // Centre: average of the three valid corners.
        let v = t.sample(0.5, 0.5).unwrap();
        assert!((v - (10.0 + 30.0 + 40.0) / 3.0).abs() < 1e-9);
        // Exactly on the void corner: a valid neighbour instead.
        assert!(t.sample(1.0, 1.0).is_some());
        let all_void: Vec<u8> = (0..4).flat_map(|_| VOID.to_be_bytes()).collect();
        assert_eq!(
            HgtTile::from_bytes(0, 0, &all_void)
                .unwrap()
                .sample(0.5, 0.5),
            None
        );
    }

    #[test]
    fn grids_come_first_and_feather_into_srtm() {
        use crate::crs::Crs;
        // SRTM: 100 m everywhere on N47E008.
        let flat: Vec<u8> = (0..9).flat_map(|_| 100i16.to_be_bytes()).collect();
        let mut dem = Dem::from_hgt(vec![HgtTile::from_bytes(47, 8, &flat).unwrap()]);
        // A 2 × 2 km DTM at 500 m around 47.5 N 8.5 E (UTM 32, 100 m tiles).
        let utm = Crs::UtmNorth(32);
        let c = utm.forward(47.5, 8.5);
        let (e0, n0) = (
            (c[0] / 100.0).floor() * 100.0 - 1000.0,
            (c[1] / 100.0).floor() * 100.0 + 1000.0,
        );
        for ti in 0..20 {
            for tj in 0..20 {
                dem.add_grid(GridTile {
                    crs: utm,
                    cell: 10.0,
                    x0: e0 + f64::from(ti) * 100.0,
                    y0: n0 - f64::from(tj) * 100.0,
                    nx: 10,
                    ny: 10,
                    data: vec![500.0; 100],
                })
                .unwrap();
            }
        }
        dem.finish();
        assert_eq!(dem.grids.len(), 1);
        let at = |x: f64, y: f64| {
            let (lat, lon) = utm.inverse(x, y);
            dem.elevation(lat, lon).unwrap()
        };
        assert!((at(c[0], c[1]) - 500.0).abs() < 1e-6, "centre: the DTM");
        assert!(
            (at(c[0] + 5000.0, c[1]) - 100.0).abs() < 1e-6,
            "outside: SRTM"
        );
        let edge = at(e0 + 100.0, c[1]);
        assert!(100.0 < edge && edge < 500.0, "feathered: {edge}");
        let (lat, lon) = utm.inverse(c[0], c[1]);
        assert_eq!(dem.source_at(lat, lon), 0);
        let (lat, lon) = utm.inverse(c[0] + 5000.0, c[1]);
        assert_eq!(dem.source_at(lat, lon), 1);
        // A tile of another cell size makes a second layer.
        dem.add_grid(GridTile {
            crs: utm,
            cell: 2.0,
            x0: e0,
            y0: n0,
            nx: 50,
            ny: 50,
            data: vec![0.0; 2500],
        })
        .unwrap();
        assert_eq!(dem.grids.len(), 2);
    }

    #[test]
    fn bad_sizes_are_rejected() {
        assert!(HgtTile::from_bytes(0, 0, &[0; 6]).is_err());
        assert!(HgtTile::from_bytes(0, 0, &[0; 2]).is_err());
    }
}
