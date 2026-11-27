//! The terrain under the flights (spec 14 Phase 18, 18E T8: "no aircraft
//! inside the terrain"): a coarse grid of the highest ground per cell,
//! built once from the package's height tiles, dilated by one cell so that
//! a straight piece of a path no longer than a cell lies over cells whose
//! highest point the grid already holds at either end.
//!
//! Every client of a session has the same package, so every client gets
//! the same grid and the same flights. Outside the package (the sea, the
//! land beyond the extract) the grid says 0 m: nothing is drawn there that
//! an aircraft could fly into.

use ocw_world::package::{TileKey, WorldPackage};

/// Side of a cell (m).
pub const CELL_M: f64 = 500.0;

/// The highest ground per cell, dilated by one cell.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Relief {
    /// x, z of the first cell's corner (m).
    origin: [f64; 2],
    nx: usize,
    nz: usize,
    /// Dilated maximum per cell (m), row-major along +z.
    max: Vec<f32>,
}

impl Relief {
    /// The grid of a package's tiles (heights every second sample: the
    /// 20 m grid; a margin of 15 m covers what it misses between them).
    pub fn of_package(pkg: &WorldPackage) -> Self {
        let size = f64::from(pkg.tile_size_m);
        let n = pkg.samples as usize;
        let spacing = pkg.spacing_m();
        let mut min = [f64::INFINITY; 2];
        let mut max = [f64::NEG_INFINITY; 2];
        for key in pkg.tiles.keys() {
            let o = key.origin(size);
            min = [min[0].min(o[0]), min[1].min(o[1])];
            max = [max[0].max(o[0] + size), max[1].max(o[1] + size)];
        }
        let samples = pkg
            .tiles
            .iter()
            .filter(move |(_, t)| n >= 2 && t.heights.len() >= n * n)
            .flat_map(move |(key, tile): (&TileKey, _)| {
                let o = key.origin(size);
                (0..n).step_by(2).flat_map(move |k| {
                    (0..n).step_by(2).filter_map(move |i| {
                        let h = tile.heights[k * n + i];
                        h.is_finite()
                            .then(|| (o[0] + i as f64 * spacing, o[1] + k as f64 * spacing, h))
                    })
                })
            });
        Self::from_samples(min, max, samples)
    }

    /// The grid of height samples `(x, z, height)` within `min` … `max`.
    pub fn from_samples(
        min: [f64; 2],
        max: [f64; 2],
        samples: impl Iterator<Item = (f64, f64, f32)>,
    ) -> Self {
        if !(min[0].is_finite() && max[0] > min[0] && max[1] > min[1]) {
            return Self::default();
        }
        let nx = ((max[0] - min[0]) / CELL_M).ceil() as usize + 1;
        let nz = ((max[1] - min[1]) / CELL_M).ceil() as usize + 1;
        let mut raw = vec![f32::NEG_INFINITY; nx * nz];
        for (x, z, h) in samples {
            let i = (((x - min[0]) / CELL_M).floor() as usize).min(nx - 1);
            let k = (((z - min[1]) / CELL_M).floor() as usize).min(nz - 1);
            let c = &mut raw[k * nx + i];
            *c = c.max(h + 15.0);
        }
        let mut dilated = vec![0.0f32; nx * nz];
        for k in 0..nz {
            for i in 0..nx {
                let mut m = 0.0f32;
                for dk in k.saturating_sub(1)..(k + 2).min(nz) {
                    for di in i.saturating_sub(1)..(i + 2).min(nx) {
                        m = m.max(raw[dk * nx + di]);
                    }
                }
                dilated[k * nx + i] = m;
            }
        }
        Self {
            origin: min,
            nx,
            nz,
            max: dilated,
        }
    }

    /// The grid's corners (x, z), if it has any.
    pub fn bounds(&self) -> Option<([f64; 2], [f64; 2])> {
        (!self.max.is_empty()).then(|| {
            (
                self.origin,
                [
                    self.origin[0] + self.nx as f64 * CELL_M,
                    self.origin[1] + self.nz as f64 * CELL_M,
                ],
            )
        })
    }

    /// Whether the grid holds any ground.
    pub fn is_empty(&self) -> bool {
        self.max.is_empty()
    }

    /// The highest ground around (x, z) (m; 0 outside the grid): a path
    /// over it in pieces of at most [`CELL_M`] stays clear of the ground.
    pub fn max_at(&self, x: f64, z: f64) -> f64 {
        if self.max.is_empty() {
            return 0.0;
        }
        let fi = ((x - self.origin[0]) / CELL_M).floor();
        let fk = ((z - self.origin[1]) / CELL_M).floor();
        if fi < 0.0 || fk < 0.0 || fi >= self.nx as f64 || fk >= self.nz as f64 {
            return 0.0;
        }
        f64::from(self.max[fk as usize * self.nx + fi as usize])
    }

    /// The lowest of the cells' highs within `r` of (x, z) and its place:
    /// a low, open spot (a balloon's launch field).
    pub fn lowest_near(&self, x: f64, z: f64, r: f64) -> Option<[f64; 2]> {
        let mut best: Option<(f32, [f64; 2])> = None;
        for k in 0..self.nz {
            for i in 0..self.nx {
                let c = [
                    self.origin[0] + (i as f64 + 0.5) * CELL_M,
                    self.origin[1] + (k as f64 + 0.5) * CELL_M,
                ];
                let h = self.max[k * self.nx + i];
                if (c[0] - x).hypot(c[1] - z) > r || h <= 15.0 {
                    continue;
                }
                if best.is_none_or(|b| h < b.0) {
                    best = Some((h, c));
                }
            }
        }
        best.map(|b| b.1)
    }
}
