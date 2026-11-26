//! Rivers and streams along their real course (spec 4.6 *Water surfaces*,
//! v6.58): the OSM waterway lines with their width and water level, cut
//! into the tiles, so the client draws each as a ribbon of water at its
//! level instead of the grid cells of the land use (which turned every
//! river into a staircase: a river 12 m wide on the 10 m grid).
//!
//! **Section `WATR`** (optional, additive; older readers skip it):
//!
//! ```text
//! layout version: u8 (1)
//! tile count: u32, then per tile:
//!   tile x, z: i32 i32
//!   line count: u32, per line:
//!     width: u16 (cm), point count: u32, points: [f32; 4] (x, level, z;
//!     package metres, in flow direction; the surface's width there – a
//!     river through a narrow riverbank area takes the area's width)
//! ```
//!
//! A line belongs to the tile of each of its segments' midpoints; a line
//! crossing a tile border is cut there with its border segment in both
//! tiles (the pieces overlap by one segment, the client draws both).

use std::collections::BTreeMap;

use crate::package::{Bytes, TileKey};

/// Section tag.
pub const WATER_LINES_TAG: &[u8; 4] = b"WATR";
const WATER_LINES_VERSION: u8 = 1;

/// One piece of a river or stream in a tile.
#[derive(Debug, Clone, PartialEq)]
pub struct WaterLine {
    /// The line's width (m; OSM `width` or its kind's default).
    pub width_m: f32,
    /// (x, level, z, width of the surface there) in package metres, in
    /// flow direction.
    pub points: Vec<[f32; 4]>,
}

impl WaterLine {
    pub fn half_width(&self) -> f32 {
        self.width_m / 2.0
    }

    /// Distance from (x, z) to the centre line, the water level and the
    /// half width of the surface at the nearest point of it.
    pub fn nearest(&self, x: f32, z: f32) -> Option<(f32, f32, f32)> {
        let mut best: Option<(f32, f32, f32)> = None;
        for w in self.points.windows(2) {
            let (a, b) = (w[0], w[1]);
            let (dx, dz) = (b[0] - a[0], b[2] - a[2]);
            let len2 = dx * dx + dz * dz;
            let t = if len2 > 0.0 {
                (((x - a[0]) * dx + (z - a[2]) * dz) / len2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let (px, pz) = (a[0] + dx * t, a[2] + dz * t);
            let d = (x - px).hypot(z - pz);
            if best.is_none_or(|(bd, _, _)| d < bd) {
                best = Some((
                    d,
                    a[1] + (b[1] - a[1]) * t,
                    (a[3] + (b[3] - a[3]) * t) / 2.0,
                ));
            }
        }
        best
    }
}

/// The rivers and streams of a package, by tile.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WaterLines {
    pub tiles: BTreeMap<TileKey, Vec<WaterLine>>,
}

impl WaterLines {
    /// Cuts whole lines (package metres) into the tiles of `tile_size`
    /// (see the module).
    pub fn from_lines(lines: &[WaterLine], tile_size: f64) -> Self {
        let mut tiles: BTreeMap<TileKey, Vec<WaterLine>> = BTreeMap::new();
        for l in lines {
            let mut current: Option<(TileKey, Vec<[f32; 4]>)> = None;
            for w in l.points.windows(2) {
                let mid = [
                    f64::from(w[0][0] + w[1][0]) / 2.0,
                    f64::from(w[0][2] + w[1][2]) / 2.0,
                ];
                let key = TileKey::containing(mid, tile_size);
                match current.as_mut() {
                    Some((k, pts)) if *k == key => pts.push(w[1]),
                    _ => {
                        if let Some((k, mut pts)) = current.take() {
                            // The border segment in both tiles.
                            pts.push(w[1]);
                            tiles.entry(k).or_default().push(WaterLine {
                                width_m: l.width_m,
                                points: pts,
                            });
                        }
                        current = Some((key, vec![w[0], w[1]]));
                    }
                }
            }
            if let Some((k, pts)) = current {
                tiles.entry(k).or_default().push(WaterLine {
                    width_m: l.width_m,
                    points: pts,
                });
            }
        }
        Self { tiles }
    }

    /// The river or stream at (x, z) whose surface covers it, with its
    /// level (the highest, where two overlap).
    pub fn level_at(&self, tile_size: f64, x: f64, z: f64) -> Option<f64> {
        let key = TileKey::containing([x, z], tile_size);
        let lines = self.tiles.get(&key)?;
        lines
            .iter()
            .filter_map(|l| {
                let (d, level, half) = l.nearest(x as f32, z as f32)?;
                (d <= half).then_some(f64::from(level))
            })
            .reduce(f64::max)
    }
}

pub fn write_water_lines(w: &WaterLines, b: &mut Vec<u8>) {
    b.push(WATER_LINES_VERSION);
    b.extend((w.tiles.len() as u32).to_le_bytes());
    for (key, lines) in &w.tiles {
        b.extend(key.x.to_le_bytes());
        b.extend(key.z.to_le_bytes());
        b.extend((lines.len() as u32).to_le_bytes());
        for l in lines {
            let cm = (l.width_m * 100.0).round().clamp(0.0, f32::from(u16::MAX)) as u16;
            b.extend(cm.to_le_bytes());
            b.extend((l.points.len() as u32).to_le_bytes());
            for p in &l.points {
                for v in p {
                    b.extend(v.to_le_bytes());
                }
            }
        }
    }
}

pub fn read_water_lines(payload: &[u8]) -> Result<WaterLines, String> {
    let mut r = Bytes::new(payload);
    let version = r.u8()?;
    if version != WATER_LINES_VERSION {
        return Err(format!("water lines layout {version} not supported"));
    }
    let n = r.u32()? as usize;
    let mut tiles = BTreeMap::new();
    for _ in 0..n {
        let key = TileKey {
            x: r.i32()?,
            z: r.i32()?,
        };
        let count = r.u32()? as usize;
        let mut lines = Vec::with_capacity(count.min(1 << 16));
        for _ in 0..count {
            let width_m = f32::from(u16::from_le_bytes(r.arr::<2>()?)) / 100.0;
            let pc = r.u32()? as usize;
            let mut points = Vec::with_capacity(pc.min(1 << 20));
            for _ in 0..pc {
                points.push([r.f32()?, r.f32()?, r.f32()?, r.f32()?]);
            }
            lines.push(WaterLine { width_m, points });
        }
        tiles.insert(key, lines);
    }
    r.finish("water lines")?;
    Ok(WaterLines { tiles })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_cut_into_tiles_and_round_trip() {
        let line = WaterLine {
            width_m: 12.5,
            points: vec![
                [10.0, 405.0, 50.0, 12.5],
                [60.0, 404.5, 50.0, 12.5],
                [140.0, 404.0, 60.0, 12.5],
                [180.0, 403.0, 70.0, 12.5],
                [260.0, 402.0, 90.0, 12.5],
            ],
        };
        let w = WaterLines::from_lines(std::slice::from_ref(&line), 100.0);
        let k = |x| TileKey { x, z: 0 };
        assert_eq!(w.tiles.len(), 3);
        // Tile 0: the first segment and the border segment (mid x 100).
        assert_eq!(w.tiles[&k(0)][0].points.len(), 3);
        // Tile 1 starts with that border segment.
        assert_eq!(w.tiles[&k(1)][0].points[0], line.points[1]);
        assert_eq!(w.tiles[&k(2)][0].points.last(), line.points.last());
        let mut b = Vec::new();
        write_water_lines(&w, &mut b);
        assert_eq!(read_water_lines(&b).unwrap(), w);
        // On the surface: the level there; beside it: none.
        let at = w.level_at(100.0, 160.0, 65.0).unwrap();
        assert!((at - 403.5).abs() < 0.2, "{at}");
        assert_eq!(w.level_at(100.0, 160.0, 80.0), None);
        // A newer layout is refused (the reader skips the section).
        b[0] = 9;
        assert!(read_water_lines(&b).unwrap_err().contains("not supported"));
    }
}
