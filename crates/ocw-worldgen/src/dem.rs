//! Digital elevation model (spec §4.3): SRTM `.hgt` tiles.
//!
//! An HGT tile covers 1° × 1° from its south-west corner (`N47E009`):
//! `n × n` big-endian `i16` heights in metres, rows from north to south,
//! columns from west to east; neighbouring tiles share their edge rows.
//! `-32768` marks voids. 1 arc-second tiles have n = 3601, 3 arc-second
//! tiles n = 1201.

use std::path::Path;

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

/// A set of tiles.
#[derive(Debug, Clone, Default)]
pub struct Dem {
    pub tiles: Vec<HgtTile>,
}

impl Dem {
    pub fn elevation(&self, lat: f64, lon: f64) -> Option<f64> {
        self.tiles.iter().find_map(|t| t.sample(lat, lon))
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
    fn bad_sizes_are_rejected() {
        assert!(HgtTile::from_bytes(0, 0, &[0; 6]).is_err());
        assert!(HgtTile::from_bytes(0, 0, &[0; 2]).is_err());
    }
}
