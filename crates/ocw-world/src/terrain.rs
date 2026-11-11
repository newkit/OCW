//! Road-conformed terrain height (spec §4.3, step 4: terrain conformance).
//!
//! The road is the elevation reference. Inside a corridor (half the road width
//! plus a shoulder) the terrain lies just below the road surface; outside it
//! blends smoothly into a base height. Phase 1 has no elevation model, so the
//! base is a smoothed field of the road's own elevation (rolling hills that
//! follow the route) that fades to the mean level far away. In Phase 3 the
//! base comes from the DEM; the corridor blending stays the same.

use crate::Route;

/// Tuning of the terrain around a route.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TerrainParams {
    /// Flat strip beside the road edge, in metres.
    pub shoulder_m: f64,
    /// Width over which the terrain blends from road level to the base, in metres.
    pub blend_m: f64,
    /// How far the terrain lies below the road surface inside the corridor.
    pub road_clearance_m: f64,
    /// Smoothing radius (Gaussian sigma) of the base field, in metres.
    pub base_sigma_m: f64,
    /// Distance from the road where the base field starts fading to the mean level.
    pub fade_start_m: f64,
    /// Distance from the road where the base field equals the mean level.
    pub fade_end_m: f64,
}

impl Default for TerrainParams {
    fn default() -> Self {
        Self {
            shoulder_m: 4.0,
            blend_m: 60.0,
            road_clearance_m: 0.06,
            base_sigma_m: 220.0,
            fade_start_m: 250.0,
            fade_end_m: 650.0,
        }
    }
}

/// Spacing of the polyline the terrain is conformed to, in metres.
const ROAD_STEP_M: f64 = 2.0;
/// Cell size of the precomputed base field, in metres.
const BASE_CELL_M: f64 = 20.0;

/// Terrain height field around one route. Pure computation: no I/O, wasm-safe.
#[derive(Debug, Clone)]
pub struct Terrain {
    params: TerrainParams,
    corridor_m: f64,
    /// Road centreline samples (x, y, z), every `ROAD_STEP_M`.
    road: Vec<[f64; 3]>,
    /// Number of road segments (closes the loop for loop routes).
    segments: usize,
    /// Spatial hash: bucket → segment indices.
    buckets: std::collections::HashMap<(i64, i64), Vec<u32>>,
    bucket_m: f64,
    mean_height: f64,
    base: Grid,
}

/// Regular grid with bilinear interpolation.
#[derive(Debug, Clone)]
struct Grid {
    origin: [f64; 2],
    cell: f64,
    nx: usize,
    nz: usize,
    values: Vec<f64>,
}

impl Grid {
    fn sample(&self, x: f64, z: f64) -> f64 {
        let fx = ((x - self.origin[0]) / self.cell).clamp(0.0, (self.nx - 1) as f64);
        let fz = ((z - self.origin[1]) / self.cell).clamp(0.0, (self.nz - 1) as f64);
        let (ix, iz) = (
            (fx.floor() as usize).min(self.nx - 2),
            (fz.floor() as usize).min(self.nz - 2),
        );
        let (tx, tz) = (fx - ix as f64, fz - iz as f64);
        let v = |i: usize, k: usize| self.values[k * self.nx + i];
        let top = v(ix, iz) * (1.0 - tx) + v(ix + 1, iz) * tx;
        let bottom = v(ix, iz + 1) * (1.0 - tx) + v(ix + 1, iz + 1) * tx;
        top * (1.0 - tz) + bottom * tz
    }
}

/// Nearest point on the road centreline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoadProximity {
    /// Horizontal distance to the centreline, in metres.
    pub distance_m: f64,
    /// Road surface elevation at the nearest point.
    pub road_height: f64,
}

impl Terrain {
    pub fn from_route(route: &Route, params: TerrainParams) -> Self {
        let n = (route.length() / ROAD_STEP_M).ceil().max(1.0) as usize;
        let road: Vec<[f64; 3]> = (0..=n)
            .map(|i| {
                route
                    .sample((i as f64 * ROAD_STEP_M).min(route.length()))
                    .position
            })
            .collect();
        let segments = road.len() - 1;
        let corridor_m = route.road_width_m() / 2.0 + params.shoulder_m;
        let bucket_m = corridor_m + params.blend_m;
        let mut buckets: std::collections::HashMap<(i64, i64), Vec<u32>> = Default::default();
        for (i, seg) in road.windows(2).enumerate() {
            let mut keys = [key(seg[0], bucket_m), key(seg[1], bucket_m)];
            keys.sort_unstable();
            for k in keys
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>()
            {
                buckets.entry(k).or_default().push(i as u32);
            }
        }
        let mean_height = road.iter().map(|p| p[1]).sum::<f64>() / road.len() as f64;
        let base = base_field(&road, mean_height, &params);
        Self {
            params,
            corridor_m,
            road,
            segments,
            buckets,
            bucket_m,
            mean_height,
            base,
        }
    }

    pub fn params(&self) -> &TerrainParams {
        &self.params
    }

    /// Half-width of the flat corridor (road plus shoulder), in metres.
    pub fn corridor_half_width_m(&self) -> f64 {
        self.corridor_m
    }

    /// Height far away from the road.
    pub fn mean_height(&self) -> f64 {
        self.mean_height
    }

    /// Area in which the terrain differs from the mean height: `[min_x, min_z], [max_x, max_z]`.
    pub fn bounds(&self) -> ([f64; 2], [f64; 2]) {
        let b = &self.base;
        (
            b.origin,
            [
                b.origin[0] + b.cell * (b.nx - 1) as f64,
                b.origin[1] + b.cell * (b.nz - 1) as f64,
            ],
        )
    }

    /// Height of the base field (terrain without the road), at `(x, z)`.
    pub fn base_height(&self, x: f64, z: f64) -> f64 {
        self.base.sample(x, z)
    }

    /// Nearest road point within the blend range, if any.
    pub fn road_proximity(&self, x: f64, z: f64) -> Option<RoadProximity> {
        let (kx, kz) = key([x, 0.0, z], self.bucket_m);
        let mut best: Option<RoadProximity> = None;
        for dx in -1..=1 {
            for dz in -1..=1 {
                let Some(segs) = self.buckets.get(&(kx + dx, kz + dz)) else {
                    continue;
                };
                for &i in segs {
                    let (a, b) = (self.road[i as usize], self.road[i as usize + 1]);
                    let (d, t) = point_segment([x, z], [a[0], a[2]], [b[0], b[2]]);
                    if best.is_none_or(|p| d < p.distance_m) {
                        best = Some(RoadProximity {
                            distance_m: d,
                            road_height: a[1] + (b[1] - a[1]) * t,
                        });
                    }
                }
            }
        }
        best.filter(|p| p.distance_m <= self.bucket_m)
    }

    /// Terrain height at `(x, z)`.
    pub fn height(&self, x: f64, z: f64) -> f64 {
        let base = self.base_height(x, z);
        let Some(road) = self.road_proximity(x, z) else {
            return base;
        };
        let at_road = road.road_height - self.params.road_clearance_m;
        let t = ((road.distance_m - self.corridor_m) / self.params.blend_m).clamp(0.0, 1.0);
        at_road + (base - at_road) * smoothstep(t)
    }

    /// Number of road segments the terrain is conformed to.
    pub fn road_segments(&self) -> usize {
        self.segments
    }
}

fn key(p: [f64; 3], size: f64) -> (i64, i64) {
    ((p[0] / size).floor() as i64, (p[2] / size).floor() as i64)
}

fn smoothstep(t: f64) -> f64 {
    t * t * (3.0 - 2.0 * t)
}

/// Distance from `p` to segment `a`–`b` and the parameter of the closest point.
fn point_segment(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> (f64, f64) {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let len2 = ab[0] * ab[0] + ab[1] * ab[1];
    let t = if len2 > 0.0 {
        (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let c = [a[0] + ab[0] * t, a[1] + ab[1] * t];
    (((p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2)).sqrt(), t)
}

/// Smoothed road elevation (Gaussian-weighted), fading to the mean far away.
fn base_field(road: &[[f64; 3]], mean: f64, params: &TerrainParams) -> Grid {
    // Every 5th sample (10 m) is plenty for a field smoothed over ~200 m.
    let pts: Vec<[f64; 3]> = road.iter().step_by(5).copied().collect();
    let (mut min, mut max) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for p in &pts {
        min = [min[0].min(p[0]), min[1].min(p[2])];
        max = [max[0].max(p[0]), max[1].max(p[2])];
    }
    let margin = params.fade_end_m + BASE_CELL_M;
    let origin = [min[0] - margin, min[1] - margin];
    let nx = ((max[0] - min[0] + 2.0 * margin) / BASE_CELL_M).ceil() as usize + 1;
    let nz = ((max[1] - min[1] + 2.0 * margin) / BASE_CELL_M).ceil() as usize + 1;
    let two_s2 = 2.0 * params.base_sigma_m * params.base_sigma_m;
    let cutoff2 = (3.0 * params.base_sigma_m).powi(2);
    let mut values = Vec::with_capacity(nx * nz);
    for k in 0..nz {
        for i in 0..nx {
            let (x, z) = (
                origin[0] + i as f64 * BASE_CELL_M,
                origin[1] + k as f64 * BASE_CELL_M,
            );
            let (mut wsum, mut hsum, mut nearest2) = (0.0, 0.0, f64::INFINITY);
            for p in &pts {
                let d2 = (p[0] - x).powi(2) + (p[2] - z).powi(2);
                nearest2 = f64::min(nearest2, d2);
                if d2 < cutoff2 {
                    let w = (-d2 / two_s2).exp();
                    wsum += w;
                    hsum += w * p[1];
                }
            }
            let smoothed = if wsum > 0.0 { hsum / wsum } else { mean };
            let fade = ((nearest2.sqrt() - params.fade_start_m)
                / (params.fade_end_m - params.fade_start_m))
                .clamp(0.0, 1.0);
            values.push(smoothed + (mean - smoothed) * smoothstep(fade));
        }
    }
    Grid {
        origin,
        cell: BASE_CELL_M,
        nx,
        nz,
        values,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat_road(elevation: f64) -> Route {
        Route::from_points(
            "flat",
            "Flat",
            vec![[0.0, elevation, 0.0], [1000.0, elevation, 0.0]],
            false,
            6.0,
        )
        .unwrap()
    }

    fn loop_terrain() -> (Route, Terrain) {
        let route = Route::test_loop();
        let terrain = Terrain::from_route(&route, TerrainParams::default());
        (route, terrain)
    }

    #[test]
    fn flat_road_gives_flat_terrain() {
        let t = Terrain::from_route(&flat_road(10.0), TerrainParams::default());
        assert!((t.mean_height() - 10.0).abs() < 1e-9);
        let road = 10.0 - t.params().road_clearance_m;
        assert!((t.height(500.0, 0.0) - road).abs() < 1e-9);
        assert!((t.height(500.0, 5000.0) - 10.0).abs() < 1e-9);
        // Every point between road level - clearance and the base level.
        for z in 0..200 {
            let h = t.height(400.0, z as f64);
            assert!((road - 1e-9..=10.0 + 1e-9).contains(&h), "z {z}: {h}");
        }
    }

    #[test]
    fn terrain_lies_just_below_the_road_inside_the_corridor() {
        let (route, t) = loop_terrain();
        let half = route.road_width_m() / 2.0 + t.params().shoulder_m;
        let mut d = 0.0;
        while d < route.length() {
            let p = route.sample(d);
            for off in [-half, -route.road_width_m() / 2.0, 0.0, 1.7, half] {
                let q = p.offset_position(off * 0.999);
                let h = t.height(q[0], q[2]);
                let below = q[1] - h;
                assert!(
                    (below - t.params().road_clearance_m).abs() < 0.03,
                    "at {d:.0} m, offset {off:.1}: terrain {below:.3} m below road"
                );
            }
            d += 37.0;
        }
    }

    #[test]
    fn terrain_is_continuous_and_has_no_cliffs() {
        let (_, t) = loop_terrain();
        let ([x0, z0], [x1, z1]) = t.bounds();
        let mut max_slope: f64 = 0.0;
        let step = 1.0;
        let mut x = x0;
        while x < x1 {
            let mut z = z0;
            while z < z1 {
                let h = t.height(x, z);
                assert!(h.is_finite());
                let sx = (t.height(x + step, z) - h).abs() / step;
                let sz = (t.height(x, z + step) - h).abs() / step;
                max_slope = max_slope.max(sx).max(sz);
                z += 23.0;
            }
            x += 19.0;
        }
        // The test loop climbs at most ~5 %; the terrain may be steeper in the
        // blend zone but never a cliff.
        assert!(max_slope < 0.5, "max slope {max_slope:.2}");
    }

    #[test]
    fn far_away_terrain_is_at_mean_level() {
        let (_, t) = loop_terrain();
        let ([x0, z0], [x1, z1]) = t.bounds();
        for (x, z) in [(x0, z0), (x1, z1), (x0 - 1000.0, 0.0), (0.0, z1 + 5000.0)] {
            assert!((t.height(x, z) - t.mean_height()).abs() < 1e-6);
        }
    }

    #[test]
    fn base_field_follows_the_road_elevation() {
        // The highest part of the loop is lifted: the base field near there is
        // clearly above the base field near the lowest part.
        let (route, t) = loop_terrain();
        let samples: Vec<_> = (0..200)
            .map(|i| route.sample(i as f64 * route.length() / 200.0).position)
            .collect();
        let hi = samples.iter().max_by(|a, b| a[1].total_cmp(&b[1])).unwrap();
        let lo = samples.iter().min_by(|a, b| a[1].total_cmp(&b[1])).unwrap();
        assert!(hi[1] - lo[1] > 20.0);
        assert!(t.base_height(hi[0], hi[2]) - t.base_height(lo[0], lo[2]) > 10.0);
    }

    #[test]
    fn blend_stays_between_road_and_base_and_reaches_base() {
        let (route, t) = loop_terrain();
        let corridor = t.corridor_half_width_m();
        let blend = t.params().blend_m;
        assert!(70.0 > blend);
        for d in [300.0, 1200.0, 2500.0, 3700.0] {
            let p = route.sample(d);
            let road = p.position[1] - t.params().road_clearance_m;
            for side in [-1.0, 1.0] {
                for i in 0..=70 {
                    let q = p.offset_position(side * (corridor + i as f64));
                    let h = t.height(q[0], q[2]);
                    let base = t.base_height(q[0], q[2]);
                    // The nearest road point may be a few cm higher or lower
                    // than `p` on a grade; allow for that.
                    let (lo, hi) = (road.min(base) - 0.1, road.max(base) + 0.1);
                    assert!(
                        (lo..=hi).contains(&h),
                        "{d} m, {i} m out: {h:.2} not in {lo:.2}..{hi:.2}"
                    );
                    // Past the blend zone (measured to the nearest road point,
                    // which on the inside of a bend is not `p`), it is the base.
                    let nearest = t
                        .road_proximity(q[0], q[2])
                        .map_or(f64::INFINITY, |r| r.distance_m);
                    if nearest >= corridor + blend {
                        assert!((h - base).abs() < 1e-9);
                    }
                    if i == 70 {
                        assert!(
                            nearest > corridor + blend * 0.5,
                            "blend not reached at {d} m"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn proximity_finds_nearest_point() {
        let t = Terrain::from_route(&flat_road(3.0), TerrainParams::default());
        let p = t.road_proximity(250.0, -4.0).unwrap();
        assert!((p.distance_m - 4.0).abs() < 1e-9);
        assert!((p.road_height - 3.0).abs() < 1e-9);
        assert!(t.road_proximity(250.0, 500.0).is_none());
        assert_eq!(t.road_segments(), 500);
    }
}
