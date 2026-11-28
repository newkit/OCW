//! Terrain shadows from the height field (spec 4.5 *Terrain shadows*,
//! v7.09): where the mountains around hide the sun. The renderer's
//! cascaded shadow maps reach at most 250 m from the camera and the terrain
//! never cast into them, so no ridge shaded a valley: a slope the sun could
//! not reach was lit like an open one at any distance. This module finds,
//! for one direction of the sun, the **shadow line** over a grid of the
//! whole world set: the height above which a point is in the sun.
//!
//! For a sun at elevation θ and the distance `s` along the ground towards
//! it, a point at height `y` sees the sun if no terrain `h(s')` (s' > s)
//! rises above the ray `y + (s' − s)·tan θ`. With `g(s) = h(s) − s·tan θ`
//! the shadow line is `S(s) = s·tan θ + max_{s' > s} g(s')`: one running
//! maximum along each line of the grid laid out along the sun's azimuth –
//! O(cells), a few milliseconds for a 1024 × 1024 grid. A fragment at
//! height `y` is lit when `y > S` (smoothly over the penumbra, which grows
//! with the distance to the occluding ridge: the sun's disc is 0.53°).
//! Comparing heights, not a lit/shaded flag, lets a tree crown or a roof
//! above a shaded slope stay in the sun and keeps the 10 m near terrain and
//! the 100 m far terrain on the same answer.
//!
//! Pure and deterministic (no threads, no clock): it builds for wasm.

use crate::package::WorldPackage;
use crate::world_set::WorldSet;

/// The finest grid cell (m): half the far terrain's 100 m grid.
pub const MIN_CELL_M: f64 = 50.0;
/// The most grid nodes per side; a larger world gets larger cells.
pub const MAX_SIDE: usize = 1024;
/// The lowest sun the map is made for (degrees): below it the shadows
/// would reach across the whole world (the light is nearly gone anyway).
pub const MIN_ELEVATION_DEG: f64 = 0.5;
/// Half the sun's angular diameter (radians): the penumbra's half width
/// per metre of distance to the occluder.
pub const SUN_HALF_ANGLE: f64 = 0.004_63;
/// The penumbra's half height at least (m): the grid's heights and the
/// mesh's differ by a few metres.
pub const MIN_SOFT_M: f64 = 3.0;

/// Terrain heights on a regular grid in home metres (x east, z south):
/// node (i, k) lies at `origin + (i, k) · cell_m`.
#[derive(Debug, Clone, PartialEq)]
pub struct HeightGrid {
    pub origin: [f64; 2],
    pub cell_m: f64,
    pub nx: usize,
    pub nz: usize,
    /// Row-major (k · nx + i); nodes without terrain get the lowest height.
    pub heights: Vec<f32>,
    pub min: f32,
    pub max: f32,
}

impl HeightGrid {
    /// Samples `height` (home metres → m, `None` = no terrain) over the box
    /// `[min x, min z, max x, max z]` with cells of `cell_m` or more (at
    /// most `max_side` nodes per side), the origin on a multiple of the
    /// cell. `None` without any terrain.
    pub fn sample(
        bounds: [f64; 4],
        cell_m: f64,
        max_side: usize,
        height: impl Fn([f64; 2]) -> Option<f64>,
    ) -> Option<Self> {
        let span = (bounds[2] - bounds[0]).max(bounds[3] - bounds[1]).max(1.0);
        let max_side = max_side.max(2);
        let mut cell = cell_m.max(1.0);
        if span / cell + 2.0 > max_side as f64 {
            cell = span / (max_side as f64 - 2.0);
        }
        let origin = [
            (bounds[0] / cell).floor() * cell,
            (bounds[1] / cell).floor() * cell,
        ];
        let nx = (((bounds[2] - origin[0]) / cell).ceil() as usize + 1).clamp(2, max_side);
        let nz = (((bounds[3] - origin[1]) / cell).ceil() as usize + 1).clamp(2, max_side);
        let mut heights = vec![f32::NAN; nx * nz];
        let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
        for k in 0..nz {
            for i in 0..nx {
                let p = [origin[0] + i as f64 * cell, origin[1] + k as f64 * cell];
                // A node on the far border of the last tile: its height
                // from just inside.
                let e = 0.01;
                let h = height(p)
                    .or_else(|| height([p[0] - e, p[1]]))
                    .or_else(|| height([p[0], p[1] - e]))
                    .or_else(|| height([p[0] - e, p[1] - e]));
                if let Some(h) = h {
                    let h = h as f32;
                    heights[k * nx + i] = h;
                    lo = lo.min(h);
                    hi = hi.max(h);
                }
            }
        }
        if !lo.is_finite() {
            return None;
        }
        for h in &mut heights {
            if h.is_nan() {
                *h = lo;
            }
        }
        Some(Self {
            origin,
            cell_m: cell,
            nx,
            nz,
            heights,
            min: lo,
            max: hi,
        })
    }

    /// The grid of one package (its own metres).
    pub fn of_package(pkg: &WorldPackage, cell_m: f64, max_side: usize) -> Option<Self> {
        let size = f64::from(pkg.tile_size_m);
        let b = pkg
            .tiles
            .iter()
            .filter(|(_, t)| !t.heights.is_empty())
            .fold(None::<[f64; 4]>, |b, (k, _)| {
                let o = k.origin(size);
                let t = [o[0], o[1], o[0] + size, o[1] + size];
                Some(b.map_or(t, |b| {
                    [
                        b[0].min(t[0]),
                        b[1].min(t[1]),
                        b[2].max(t[2]),
                        b[3].max(t[3]),
                    ]
                }))
            })?;
        Self::sample(b, cell_m, max_side, |p| pkg.height(p[0], p[1]))
    }

    /// The grid of the resident packages of a world set (home metres; the
    /// package that owns the ground at a node gives its height).
    pub fn of_set(set: &WorldSet, cell_m: f64, max_side: usize) -> Option<Self> {
        let mut b: Option<[f64; 4]> = None;
        for m in set.members().iter().filter(|m| m.is_resident()) {
            let (lo, hi) = m.tile_bounds();
            let t = [lo[0], lo[1], hi[0], hi[1]];
            b = Some(b.map_or(t, |b| {
                [
                    b[0].min(t[0]),
                    b[1].min(t[1]),
                    b[2].max(t[2]),
                    b[3].max(t[3]),
                ]
            }));
        }
        let height = |p: [f64; 2]| {
            let at = |i: usize| {
                let m = &set.members()[i];
                let q = m.frame.invert(p);
                m.package()?.height(q[0], q[1])
            };
            set.owner(p).and_then(at)
        };
        Self::sample(b?, cell_m, max_side, height)
    }

    fn node(&self, i: usize, k: usize) -> f32 {
        self.heights[k.min(self.nz - 1) * self.nx + i.min(self.nx - 1)]
    }

    /// Bilinear height at `p` (home metres); `None` outside the grid.
    pub fn height(&self, p: [f64; 2]) -> Option<f32> {
        let fx = (p[0] - self.origin[0]) / self.cell_m;
        let fz = (p[1] - self.origin[1]) / self.cell_m;
        if fx < 0.0 || fz < 0.0 || fx > (self.nx - 1) as f64 || fz > (self.nz - 1) as f64 {
            return None;
        }
        let (i, k) = (fx.floor() as usize, fz.floor() as usize);
        let (tx, tz) = ((fx - i as f64) as f32, (fz - k as f64) as f32);
        let top = self.node(i, k) * (1.0 - tx) + self.node(i + 1, k) * tx;
        let bottom = self.node(i, k + 1) * (1.0 - tx) + self.node(i + 1, k + 1) * tx;
        Some(top * (1.0 - tz) + bottom * tz)
    }

    /// Bytes held (memory budget).
    pub fn bytes(&self) -> usize {
        self.heights.len() * 4
    }
}

/// The shadow line of a [`HeightGrid`] for one sun direction, on the same
/// nodes: `line[k · nx + i] = (S, soft)` – a point at height `y` is in the
/// sun for `y > S + soft`, in the shadow for `y < S − soft`.
#[derive(Debug, Clone, PartialEq)]
pub struct ShadowLines {
    pub origin: [f64; 2],
    pub cell_m: f64,
    pub nx: usize,
    pub nz: usize,
    pub line: Vec<[f32; 2]>,
    /// The direction to the sun it was made for (x east, y up, z south).
    pub towards: [f32; 3],
}

/// The shadow line for the direction to the sun `towards` (x east, y up,
/// z south; need not be normalised). A sun higher than ~89.9° casts no
/// shadows; one below [`MIN_ELEVATION_DEG`] is taken at that elevation.
pub fn shadow_lines(grid: &HeightGrid, towards: [f32; 3]) -> ShadowLines {
    let t = [
        f64::from(towards[0]),
        f64::from(towards[1]),
        f64::from(towards[2]),
    ];
    let flat = (t[0] * t[0] + t[2] * t[2]).sqrt();
    let n = grid.nx * grid.nz;
    // Far below any terrain: lit.
    let open = [grid.min - 1000.0, MIN_SOFT_M as f32];
    let mut out = ShadowLines {
        origin: grid.origin,
        cell_m: grid.cell_m,
        nx: grid.nx,
        nz: grid.nz,
        line: vec![open; n],
        towards,
    };
    if flat < 1e-3 * t[1].abs().max(1e-9) {
        return out;
    }
    let tan = (t[1] / flat).max(MIN_ELEVATION_DEG.to_radians().tan());
    let d = [t[0] / flat, t[2] / flat];
    let perp = [-d[1], d[0]];
    let c = grid.cell_m;
    // The grid's corners in (u along d, w along perp), relative to origin.
    let w_x = (grid.nx - 1) as f64 * c;
    let w_z = (grid.nz - 1) as f64 * c;
    let corners = [[0.0, 0.0], [w_x, 0.0], [0.0, w_z], [w_x, w_z]];
    let proj = |p: [f64; 2], a: [f64; 2]| p[0] * a[0] + p[1] * a[1];
    let (mut u0, mut u1, mut w0, mut w1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for p in corners {
        u0 = u0.min(proj(p, d));
        u1 = u1.max(proj(p, d));
        w0 = w0.min(proj(p, perp));
        w1 = w1.max(proj(p, perp));
    }
    // The lines: half a cell apart across (so every node lies near one),
    // a cell apart along.
    let step_w = c * 0.5;
    let lines = ((w1 - w0) / step_w).ceil() as usize + 1;
    let steps = ((u1 - u0) / c).ceil() as usize + 1;
    // Each node takes the line sample nearest to it (the lines are dense
    // enough): accumulate by distance weight.
    let mut acc = vec![[0.0f64; 3]; n];
    let soft_grid = 0.5 * c * tan;
    for j in 0..lines {
        let w = w0 + j as f64 * step_w;
        let mut best = f64::NEG_INFINITY; // max g(s') over s' > s
        let mut best_u = u1;
        for q in 0..steps {
            let u = u1 - q as f64 * c;
            let p = [
                grid.origin[0] + u * d[0] + w * perp[0],
                grid.origin[1] + u * d[1] + w * perp[1],
            ];
            let Some(h) = grid.height(p) else { continue };
            let h = f64::from(h);
            let g = h - u * tan;
            let (line, soft) = if best.is_finite() {
                let dist = best_u - u;
                (
                    best + u * tan,
                    (MIN_SOFT_M + SUN_HALF_ANGLE * dist + soft_grid) as f32,
                )
            } else {
                (f64::from(open[0]), open[1])
            };
            // Splat to the 4 nodes around p (bilinear weights).
            let fx = (p[0] - grid.origin[0]) / c;
            let fz = (p[1] - grid.origin[1]) / c;
            let (i, k) = (fx.floor() as usize, fz.floor() as usize);
            let (tx, tz) = (fx - i as f64, fz - k as f64);
            for (di, dk, wt) in [
                (0, 0, (1.0 - tx) * (1.0 - tz)),
                (1, 0, tx * (1.0 - tz)),
                (0, 1, (1.0 - tx) * tz),
                (1, 1, tx * tz),
            ] {
                let (ii, kk) = (i + di, k + dk);
                if ii < grid.nx && kk < grid.nz && wt > 1e-6 {
                    let a = &mut acc[kk * grid.nx + ii];
                    a[0] += wt * line;
                    a[1] += wt * f64::from(soft);
                    a[2] += wt;
                }
            }
            if g > best {
                best = g;
                best_u = u;
            }
        }
    }
    for (o, a) in out.line.iter_mut().zip(&acc) {
        if a[2] > 0.0 {
            *o = [(a[0] / a[2]) as f32, (a[1] / a[2]) as f32];
        }
    }
    out
}

impl ShadowLines {
    /// Bilinear (S, soft) at `p` (home metres); `None` outside.
    pub fn at(&self, p: [f64; 2]) -> Option<[f32; 2]> {
        let fx = (p[0] - self.origin[0]) / self.cell_m;
        let fz = (p[1] - self.origin[1]) / self.cell_m;
        if fx < 0.0 || fz < 0.0 || fx > (self.nx - 1) as f64 || fz > (self.nz - 1) as f64 {
            return None;
        }
        let (i, k) = (fx.floor() as usize, fz.floor() as usize);
        let (tx, tz) = ((fx - i as f64) as f32, (fz - k as f64) as f32);
        let node =
            |i: usize, k: usize| self.line[k.min(self.nz - 1) * self.nx + i.min(self.nx - 1)];
        let mix =
            |a: [f32; 2], b: [f32; 2], t: f32| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
        let top = mix(node(i, k), node(i + 1, k), tx);
        let bottom = mix(node(i, k + 1), node(i + 1, k + 1), tx);
        Some(mix(top, bottom, tz))
    }

    /// How much of the sun a point at `p`, height `y` sees (0 … 1; 1
    /// outside the grid): the renderer's `terrain_shadow.wesl` does the same.
    pub fn sun(&self, p: [f64; 2], y: f32) -> f32 {
        match self.at(p) {
            Some([s, soft]) => visibility(y, s, soft),
            None => 1.0,
        }
    }

    /// Bytes held.
    pub fn bytes(&self) -> usize {
        self.line.len() * 8
    }
}

/// The sun seen from height `y` under the shadow line `s` with penumbra
/// half height `soft` (a smoothstep from `s − soft` to `s + soft`, lifted
/// by [`BIAS_M`] so terrain exactly on its own line stays lit).
pub fn visibility(y: f32, s: f32, soft: f32) -> f32 {
    let soft = soft.max(MIN_SOFT_M as f32);
    let t = ((y + BIAS_M - (s - soft)) / (2.0 * soft)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Lift of the fragment over the shadow line (m): the grid's bilinear
/// heights and the drawn mesh differ by a few metres on a lit slope.
pub const BIAS_M: f32 = 3.0;

#[cfg(test)]
mod tests {
    use super::*;

    /// A ridge running north–south at x = 0 (top 600 m, flanks 30 %), a
    /// valley floor at 0 m on both sides out to ±5 km.
    fn ridge(p: [f64; 2]) -> Option<f64> {
        Some((600.0 - 0.3 * p[0].abs()).max(0.0))
    }

    fn grid(cell: f64) -> HeightGrid {
        HeightGrid::sample([-5000.0, -5000.0, 5000.0, 5000.0], cell, MAX_SIDE, ridge).unwrap()
    }

    /// The sun in the west (azimuth 270°) at `elev` degrees.
    fn west_sun(elev: f64) -> [f32; 3] {
        let e = elev.to_radians();
        [-(e.cos()) as f32, e.sin() as f32, 0.0]
    }

    #[test]
    fn valley_behind_the_ridge_is_in_shadow() {
        let g = grid(50.0);
        let lines = shadow_lines(&g, west_sun(10.0));
        // East of the ridge the shadow of its top reaches 600 / tan 10° =
        // 3.4 km: over the valley floor from the foot (x = 2000) to 3400.
        assert!(lines.sun([2500.0, 0.0], 0.0) < 0.05);
        assert!(lines.sun([3200.0, 0.0], 0.0) < 0.05);
        assert!(lines.sun([4500.0, 0.0], 0.0) > 0.95);
        // Beyond its end, west of the ridge and on its sunny flank: lit.
        assert!(lines.sun([-3000.0, 0.0], 0.0) > 0.95);
        assert!(lines.sun([-1000.0, 1000.0], ridge([-1000.0, 0.0]).unwrap() as f32) > 0.95);
        // The east flank (30 % falling away from a 10° sun) is in shadow.
        let y = ridge([1000.0, 0.0]).unwrap() as f32;
        assert!(lines.sun([1000.0, 0.0], y) < 0.05);
        // A tree crown 30 m above the valley floor near the shadow's end
        // sees the sun earlier than the ground.
        let end = 600.0 / 10f64.to_radians().tan();
        let near_end = [end - 150.0, 0.0];
        assert!(lines.sun(near_end, 30.0) > lines.sun(near_end, 0.0));
    }

    #[test]
    fn shadow_end_is_where_the_geometry_says() {
        let g = grid(50.0);
        for elev in [8.0, 10.0, 14.0] {
            let lines = shadow_lines(&g, west_sun(elev));
            let end = 600.0 / elev.to_radians().tan();
            let x = (0..10_000)
                .map(|i| f64::from(i) * 1.0)
                .find(|&x| x > 2000.0 && lines.sun([x, 0.0], 0.0) > 0.5)
                .unwrap();
            assert!((x - end).abs() < 120.0, "sun {elev}°: end {x} want {end}");
        }
    }

    #[test]
    fn same_answer_at_both_grid_sizes() {
        // The far terrain's coarse grid and a finer one agree away from
        // the penumbra: the answer does not depend on the sampling.
        let lines_fine = shadow_lines(&grid(50.0), west_sun(10.0));
        let lines_coarse = shadow_lines(&grid(100.0), west_sun(10.0));
        for x in (-4800i32..4800).step_by(100) {
            let p = [f64::from(x), 300.0];
            let y = ridge(p).unwrap() as f32;
            let (a, b) = (lines_fine.sun(p, y), lines_coarse.sun(p, y));
            let end = 600.0 / 10f64.to_radians().tan();
            if (f64::from(x) - end).abs() > 200.0 && x.abs() > 100 {
                assert!((a - b).abs() < 0.1, "x {x}: {a} vs {b}");
            }
        }
    }

    #[test]
    fn rotated_sun_gives_the_same_shadow() {
        // The same ridge turned by 37°, the sun turned with it.
        let a = 37f64.to_radians();
        let (c, s) = (a.cos(), a.sin());
        let turned = |p: [f64; 2]| {
            // Back into the ridge's axes.
            let x = c * p[0] + s * p[1];
            ridge([x, 0.0])
        };
        let g =
            HeightGrid::sample([-5000.0, -5000.0, 5000.0, 5000.0], 50.0, MAX_SIDE, turned).unwrap();
        let sun = west_sun(10.0);
        let towards = [
            (c * f64::from(sun[0])) as f32,
            sun[1],
            (s * f64::from(sun[0])) as f32,
        ];
        let lines = shadow_lines(&g, towards);
        for x in [-3000.0, -1000.0, 1000.0, 2500.0, 4500.0] {
            let p = [c * x, s * x];
            let y = ridge([x, 0.0]).unwrap() as f32;
            let want = shadow_lines(&grid(50.0), sun).sun([x, 0.0], y);
            let got = lines.sun(p, y);
            assert!((got - want).abs() < 0.15, "x {x}: {got} vs {want}");
        }
    }

    #[test]
    fn high_sun_and_flat_ground_cast_nothing() {
        let flat = HeightGrid::sample([0.0, 0.0, 2000.0, 2000.0], 50.0, MAX_SIDE, |_| Some(500.0))
            .unwrap();
        let lines = shadow_lines(&flat, west_sun(3.0));
        assert!(
            lines
                .line
                .iter()
                .all(|l| visibility(500.0, l[0], l[1]) > 0.99)
        );
        let lines = shadow_lines(&grid(50.0), [0.0, 1.0, 0.0]);
        assert!(lines.sun([2500.0, 0.0], 0.0) > 0.99);
    }

    #[test]
    fn deterministic_and_bounded() {
        let g = grid(50.0);
        let a = shadow_lines(&g, west_sun(7.0));
        let b = shadow_lines(&g, west_sun(7.0));
        assert_eq!(a, b);
        // A 10 km world at 50 m: 201 × 201 nodes, under 400 KB each.
        assert_eq!((g.nx, g.nz), (201, 201));
        assert!(g.bytes() < 200_000 && a.bytes() < 400_000);
        // The largest grid: MAX_SIDE² nodes, 4 MB of heights, 8 MB of lines.
        let big = HeightGrid::sample([0.0, 0.0, 200_000.0, 90_000.0], 50.0, MAX_SIDE, |_| {
            Some(0.0)
        })
        .unwrap();
        assert!(
            big.nx <= MAX_SIDE && big.nz <= MAX_SIDE,
            "{} {}",
            big.nx,
            big.nz
        );
        assert!(big.cell_m > 190.0);
    }

    #[test]
    fn grid_samples_the_heights() {
        let g = grid(100.0);
        assert_eq!(g.height([0.0, 0.0]), Some(600.0));
        assert_eq!(g.height([1000.0, 50.0]), Some(300.0));
        assert_eq!(g.height([9000.0, 0.0]), None);
        assert_eq!((g.min, g.max), (0.0, 600.0));
    }
}
