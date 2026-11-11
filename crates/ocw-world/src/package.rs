//! World package (spec §4.5, §4.7): a region cut into 1 km tiles, each with
//! road-conformed terrain heights and the road strips on it.
//!
//! Binary, little-endian (`OCWPKG01`):
//!
//! ```text
//! magic "OCWPKG01"
//! name: u32 length + UTF-8          origin lat, lon: f64 f64
//! tile size (m): f32                grid samples per side n: u32
//! tile count: u32, then per tile:
//!   tile x, z: i32 i32              n × n heights: f32 (rows along +z)
//!   road count: u32, then per road:
//!     class: u8   width: f32   point count: u32   points: [f32; 3] (x, y, z)
//! ```
//!
//! Tile `(x, z)` covers local metres `[x·size, (x+1)·size] × [z·size, (z+1)·size]`;
//! the grid includes both borders, so neighbouring tiles share their edge
//! samples (no cracks). Signing (§4.7) comes with the distribution step.

use std::collections::BTreeMap;

use crate::graph::RoadClass;

pub const MAGIC: &[u8; 8] = b"OCWPKG01";

/// Index of a tile in the package grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileKey {
    pub x: i32,
    pub z: i32,
}

impl TileKey {
    pub fn containing(p: [f64; 2], size: f64) -> Self {
        Self {
            x: (p[0] / size).floor() as i32,
            z: (p[1] / size).floor() as i32,
        }
    }

    pub fn origin(&self, size: f64) -> [f64; 2] {
        [f64::from(self.x) * size, f64::from(self.z) * size]
    }

    /// Chebyshev distance in tiles.
    pub fn ring(&self, other: TileKey) -> i32 {
        (self.x - other.x).abs().max((self.z - other.z).abs())
    }
}

/// A road strip on a tile: centreline with heights, local metres.
#[derive(Debug, Clone, PartialEq)]
pub struct RoadStrip {
    pub class: RoadClass,
    pub width_m: f32,
    pub points: Vec<[f32; 3]>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tile {
    pub key: TileKey,
    /// `n × n` terrain heights, row-major along +z, covering the tile with
    /// both borders.
    pub heights: Vec<f32>,
    pub roads: Vec<RoadStrip>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorldPackage {
    pub name: String,
    /// Projection centre of the local metres (lat, lon).
    pub origin: (f64, f64),
    pub tile_size_m: f32,
    /// Grid samples per tile side (including both borders).
    pub samples: u32,
    pub tiles: BTreeMap<TileKey, Tile>,
}

impl WorldPackage {
    pub fn spacing_m(&self) -> f64 {
        f64::from(self.tile_size_m) / f64::from(self.samples - 1)
    }

    /// Terrain height at local (x, z), bilinear within its tile.
    pub fn height(&self, x: f64, z: f64) -> Option<f64> {
        let size = f64::from(self.tile_size_m);
        let key = TileKey::containing([x, z], size);
        let tile = self.tiles.get(&key)?;
        let o = key.origin(size);
        let n = self.samples as usize;
        let s = self.spacing_m();
        let (fx, fz) = (
            ((x - o[0]) / s).clamp(0.0, (n - 1) as f64),
            ((z - o[1]) / s).clamp(0.0, (n - 1) as f64),
        );
        let (i, k) = ((fx as usize).min(n - 2), (fz as usize).min(n - 2));
        let (tx, tz) = (fx - i as f64, fz - k as f64);
        let h = |i: usize, k: usize| f64::from(tile.heights[k * n + i]);
        let top = h(i, k) * (1.0 - tx) + h(i + 1, k) * tx;
        let bottom = h(i, k + 1) * (1.0 - tx) + h(i + 1, k + 1) * tx;
        Some(top * (1.0 - tz) + bottom * tz)
    }

    /// Height of the rendered terrain surface: the tile grid triangulated
    /// like the client mesh (cells split along (i+1, k)–(i, k+1)).
    pub fn surface_height(&self, x: f64, z: f64) -> Option<f64> {
        let size = f64::from(self.tile_size_m);
        let key = TileKey::containing([x, z], size);
        let tile = self.tiles.get(&key)?;
        let o = key.origin(size);
        let n = self.samples as usize;
        let s = self.spacing_m();
        let (fx, fz) = (
            ((x - o[0]) / s).clamp(0.0, (n - 1) as f64),
            ((z - o[1]) / s).clamp(0.0, (n - 1) as f64),
        );
        let (i, k) = ((fx as usize).min(n - 2), (fz as usize).min(n - 2));
        let (tx, tz) = (fx - i as f64, fz - k as f64);
        let h = |i: usize, k: usize| f64::from(tile.heights[k * n + i]);
        let (h00, h10, h01, h11) = (h(i, k), h(i + 1, k), h(i, k + 1), h(i + 1, k + 1));
        Some(if tx + tz <= 1.0 {
            h00 + tx * (h10 - h00) + tz * (h01 - h00)
        } else {
            h11 + (1.0 - tx) * (h01 - h11) + (1.0 - tz) * (h10 - h11)
        })
    }

    /// Where the terrain surface rises above a road (it would show through
    /// the road surface): sampled every `step` m along each road strip and
    /// every 0.25 m across it. Returns (worst excess in m, its position,
    /// number of samples above the road, number of samples).
    pub fn road_pokes(&self, step: f64) -> (f64, [f64; 2], usize, usize) {
        let (mut worst, mut at, mut above, mut total) = (0.0f64, [0.0, 0.0], 0, 0);
        for tile in self.tiles.values() {
            for r in &tile.roads {
                let half = f64::from(r.width_m) / 2.0;
                for w in r.points.windows(2) {
                    let (a, b) = (w[0].map(f64::from), w[1].map(f64::from));
                    let len = ((b[0] - a[0]).powi(2) + (b[2] - a[2]).powi(2)).sqrt();
                    if len < 1e-6 {
                        continue;
                    }
                    let (dx, dz) = ((b[0] - a[0]) / len, (b[2] - a[2]) / len);
                    let steps = (len / step).ceil() as usize;
                    for j in 0..steps {
                        let t = j as f64 * step / len;
                        let (cx, cy, cz) = (
                            a[0] + (b[0] - a[0]) * t,
                            a[1] + (b[1] - a[1]) * t,
                            a[2] + (b[2] - a[2]) * t,
                        );
                        let mut off = -half;
                        while off <= half + 1e-9 {
                            let (x, z) = (cx - dz * off, cz + dx * off);
                            if let Some(h) = self.surface_height(x, z) {
                                total += 1;
                                let excess = h - cy;
                                if excess > 0.0 {
                                    above += 1;
                                    if excess > worst {
                                        worst = excess;
                                        at = [x, z];
                                    }
                                }
                            }
                            off += 0.25;
                        }
                    }
                }
            }
        }
        (worst, at, above, total)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(MAGIC);
        b.extend((self.name.len() as u32).to_le_bytes());
        b.extend(self.name.as_bytes());
        b.extend(self.origin.0.to_le_bytes());
        b.extend(self.origin.1.to_le_bytes());
        b.extend(self.tile_size_m.to_le_bytes());
        b.extend(self.samples.to_le_bytes());
        b.extend((self.tiles.len() as u32).to_le_bytes());
        for t in self.tiles.values() {
            b.extend(t.key.x.to_le_bytes());
            b.extend(t.key.z.to_le_bytes());
            for h in &t.heights {
                b.extend(h.to_le_bytes());
            }
            b.extend((t.roads.len() as u32).to_le_bytes());
            for r in &t.roads {
                b.push(class_code(r.class));
                b.extend(r.width_m.to_le_bytes());
                b.extend((r.points.len() as u32).to_le_bytes());
                for p in &r.points {
                    for c in p {
                        b.extend(c.to_le_bytes());
                    }
                }
            }
        }
        b
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let mut r = Bytes { b: bytes, pos: 0 };
        if r.take(8)? != MAGIC {
            return Err("not an OCW world package (OCWPKG01)".into());
        }
        let name_len = r.u32()? as usize;
        let name = String::from_utf8(r.take(name_len)?.to_vec()).map_err(|e| e.to_string())?;
        let origin = (r.f64()?, r.f64()?);
        let tile_size_m = r.f32()?;
        let samples = r.u32()?;
        if samples < 2 {
            return Err("tile grid needs at least 2 samples".into());
        }
        let count = r.u32()?;
        let mut tiles = BTreeMap::new();
        for _ in 0..count {
            let key = TileKey {
                x: r.i32()?,
                z: r.i32()?,
            };
            let n = samples as usize * samples as usize;
            let heights = (0..n).map(|_| r.f32()).collect::<Result<_, _>>()?;
            let roads_n = r.u32()?;
            let mut roads = Vec::with_capacity(roads_n as usize);
            for _ in 0..roads_n {
                let class = class_from(r.u8()?)?;
                let width_m = r.f32()?;
                let pts = r.u32()?;
                let points = (0..pts)
                    .map(|_| Ok([r.f32()?, r.f32()?, r.f32()?]))
                    .collect::<Result<_, String>>()?;
                roads.push(RoadStrip {
                    class,
                    width_m,
                    points,
                });
            }
            tiles.insert(
                key,
                Tile {
                    key,
                    heights,
                    roads,
                },
            );
        }
        if r.pos != bytes.len() {
            return Err("trailing bytes after the last tile".into());
        }
        Ok(Self {
            name,
            origin,
            tile_size_m,
            samples,
            tiles,
        })
    }
}

const CLASSES: [RoadClass; 10] = [
    RoadClass::Trunk,
    RoadClass::Primary,
    RoadClass::Secondary,
    RoadClass::Tertiary,
    RoadClass::Unclassified,
    RoadClass::Residential,
    RoadClass::Service,
    RoadClass::Track,
    RoadClass::Cycleway,
    RoadClass::Path,
];

fn class_code(c: RoadClass) -> u8 {
    CLASSES
        .iter()
        .position(|&x| x == c)
        .expect("all classes listed") as u8
}

fn class_from(code: u8) -> Result<RoadClass, String> {
    CLASSES
        .get(usize::from(code))
        .copied()
        .ok_or_else(|| format!("unknown road class {code}"))
}

struct Bytes<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Bytes<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let s = self
            .b
            .get(self.pos..self.pos + n)
            .ok_or("package truncated")?;
        self.pos += n;
        Ok(s)
    }
    fn arr<const N: usize>(&mut self) -> Result<[u8; N], String> {
        Ok(self.take(N)?.try_into().expect("length N"))
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.arr()?))
    }
    fn i32(&mut self) -> Result<i32, String> {
        Ok(i32::from_le_bytes(self.arr()?))
    }
    fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_le_bytes(self.arr()?))
    }
    fn f64(&mut self) -> Result<f64, String> {
        Ok(f64::from_le_bytes(self.arr()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package() -> WorldPackage {
        let mut tiles = BTreeMap::new();
        for (x, z) in [(0, 0), (1, 0), (-1, 2)] {
            let key = TileKey { x, z };
            tiles.insert(
                key,
                Tile {
                    key,
                    // 3 × 3 grid: height = 10·x_index + z_index + 100·tile x
                    heights: (0..9)
                        .map(|i| (i % 3 * 10 + i / 3 + 100 * x) as f32)
                        .collect(),
                    roads: vec![RoadStrip {
                        class: RoadClass::Secondary,
                        width_m: 6.5,
                        points: vec![[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]],
                    }],
                },
            );
        }
        WorldPackage {
            name: "Test".into(),
            origin: (47.15, 9.55),
            tile_size_m: 1000.0,
            samples: 3,
            tiles,
        }
    }

    #[test]
    fn binary_round_trip() {
        let p = package();
        let bytes = p.to_bytes();
        assert_eq!(&bytes[..8], MAGIC);
        assert_eq!(WorldPackage::from_bytes(&bytes).unwrap(), p);
    }

    #[test]
    fn broken_packages_are_rejected() {
        let bytes = package().to_bytes();
        assert!(WorldPackage::from_bytes(&bytes[..bytes.len() - 1]).is_err());
        assert!(WorldPackage::from_bytes(b"OCWPKG99").is_err());
        let mut more = bytes.clone();
        more.push(0);
        assert!(WorldPackage::from_bytes(&more).is_err());
        let mut bad_class = package();
        bad_class.tiles.values_mut().next().unwrap().roads[0].class = RoadClass::Path;
        let mut b = bad_class.to_bytes();
        // Corrupt the class byte of the first road.
        let pos = b
            .windows(4)
            .position(|w| w == 6.5f32.to_le_bytes())
            .unwrap()
            - 1;
        b[pos] = 42;
        assert!(WorldPackage::from_bytes(&b).unwrap_err().contains("class"));
    }

    #[test]
    fn heights_are_bilinear_within_tiles() {
        let p = package();
        assert_eq!(p.spacing_m(), 500.0);
        assert_eq!(p.height(0.0, 0.0), Some(0.0));
        assert_eq!(p.height(500.0, 500.0), Some(11.0));
        assert_eq!(p.height(250.0, 0.0), Some(5.0));
        assert_eq!(p.height(1500.0, 0.0), Some(110.0));
        assert_eq!(p.height(5000.0, 0.0), None);
        assert_eq!(
            TileKey::containing([-1.0, 2500.0], 1000.0),
            TileKey { x: -1, z: 2 }
        );
        assert_eq!(TileKey { x: 0, z: 0 }.ring(TileKey { x: -2, z: 1 }), 2);
    }
}
