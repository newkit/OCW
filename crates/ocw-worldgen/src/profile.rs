//! Road elevation profiles (spec §4.3): sampled from the DEM every 5 m,
//! smoothed (median, Gaussian), limited in gradient and gradient change
//! per road class, fixed to the junction heights at both ends; bridges and
//! tunnels interpolated between their ends.

use ocw_world::graph::{PROFILE_STEP_M, RoadClass, RoadGraph};

use crate::dem::Dem;
use crate::projection::LocalProjection;

/// Gradient limits of a road class: (max gradient, max gradient change per metre).
pub fn limits(class: RoadClass) -> (f64, f64) {
    match class {
        RoadClass::Trunk | RoadClass::Primary | RoadClass::Secondary => (0.15, 0.005),
        RoadClass::Tertiary | RoadClass::Unclassified | RoadClass::Residential => (0.20, 0.01),
        RoadClass::Cycleway => (0.20, 0.01),
        RoadClass::Service | RoadClass::Track | RoadClass::Path => (0.25, 0.01),
    }
}

/// Gaussian smoothing radius (sigma, in metres) of a road class: main
/// roads run evenly; SRTM is a surface model (trees, buildings), so short
/// bumps are noise.
pub fn smoothing_sigma_m(class: RoadClass) -> f64 {
    match class {
        RoadClass::Trunk | RoadClass::Primary | RoadClass::Secondary => 50.0,
        RoadClass::Tertiary | RoadClass::Unclassified => 40.0,
        _ => 30.0,
    }
}

/// Positions along a polyline every `step` metres, plus the end point.
pub fn resample(points: &[[f64; 2]], step: f64) -> Vec<[f64; 2]> {
    let mut out = vec![points[0]];
    let (mut walked, mut next) = (0.0, step);
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
        while len > 0.0 && next <= walked + len + 1e-9 {
            let t = ((next - walked) / len).min(1.0);
            out.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
            next += step;
        }
        walked += len;
    }
    let last = *points.last().expect("non-empty");
    let end = out.last().expect("non-empty");
    if (end[0] - last[0]).abs() + (end[1] - last[1]).abs() > 1e-6 {
        out.push(last);
    }
    out
}

fn median_filter(v: &[f64], half: usize) -> Vec<f64> {
    (0..v.len())
        .map(|i| {
            let (a, b) = (i.saturating_sub(half), (i + half + 1).min(v.len()));
            let mut w = v[a..b].to_vec();
            w.sort_by(f64::total_cmp);
            w[w.len() / 2]
        })
        .collect()
}

fn gaussian(v: &[f64], sigma_samples: f64) -> Vec<f64> {
    let r = (3.0 * sigma_samples).ceil() as i64;
    (0..v.len() as i64)
        .map(|i| {
            let (mut s, mut w) = (0.0, 0.0);
            for k in -r..=r {
                let j = (i + k).clamp(0, v.len() as i64 - 1) as usize;
                let wk = (-(k as f64).powi(2) / (2.0 * sigma_samples * sigma_samples)).exp();
                s += v[j] * wk;
                w += wk;
            }
            s / w
        })
        .collect()
}

/// Distances of the samples from the start (last interval may be shorter).
fn distances(n: usize, length: f64) -> Vec<f64> {
    (0..n)
        .map(|i| (i as f64 * PROFILE_STEP_M).min(length))
        .collect()
}

/// Limits gradient and gradient change while keeping the profile close to
/// `target`, with fixed heights at both ends.
pub fn constrain(
    target: &[f64],
    length: f64,
    start: f64,
    end: f64,
    max_grade: f64,
    max_change_per_m: f64,
) -> Vec<f64> {
    let n = target.len();
    if n < 2 {
        return target.to_vec();
    }
    let d = distances(n, length);
    // Gradients of the target per interval, clamped.
    let mut g: Vec<f64> = (0..n - 1)
        .map(|i| {
            let ds = (d[i + 1] - d[i]).max(1e-9);
            ((target[i + 1] - target[i]) / ds).clamp(-max_grade, max_grade)
        })
        .collect();
    // Limit the change between neighbouring intervals (forward and back).
    for _ in 0..3 {
        for i in 1..g.len() {
            let ds = (d[i + 1] - d[i - 1]) / 2.0;
            let m = max_change_per_m * ds;
            g[i] = g[i].clamp(g[i - 1] - m, g[i - 1] + m);
        }
        for i in (0..g.len().saturating_sub(1)).rev() {
            let ds = (d[i + 2] - d[i]) / 2.0;
            let m = max_change_per_m * ds;
            g[i] = g[i].clamp(g[i + 1] - m, g[i + 1] + m);
        }
    }
    // Meet the end height: distribute the missing height over the
    // intervals in proportion to their room below the gradient limit (so
    // the limit holds whenever the end is reachable at all), then a linear
    // rest for what is physically impossible within the limit.
    let ds: Vec<f64> = (0..n - 1).map(|i| d[i + 1] - d[i]).collect();
    let reached: f64 = g.iter().zip(&ds).map(|(g, ds)| g * ds).sum();
    let miss = end - start - reached;
    let room: Vec<f64> = g
        .iter()
        .map(|&g| {
            if miss > 0.0 {
                (max_grade - g).max(0.0)
            } else {
                (g + max_grade).max(0.0)
            }
        })
        .collect();
    let total: f64 = room.iter().zip(&ds).map(|(r, ds)| r * ds).sum();
    if total > 1e-12 {
        let frac = (miss.abs() / total).min(1.0) * miss.signum();
        for (gi, r) in g.iter_mut().zip(&room) {
            *gi += r * frac;
        }
    }
    let mut h = vec![start; n];
    for i in 1..n {
        h[i] = h[i - 1] + g[i - 1] * ds[i - 1];
    }
    let rest = end - h[n - 1];
    for i in 0..n {
        h[i] += rest * d[i] / length.max(1e-9);
    }
    h
}

/// Per-edge findings for the build log (spec §4.3, step 5).
#[derive(Debug, Clone, PartialEq)]
pub struct Suspicious {
    pub edge: usize,
    pub name: Option<String>,
    /// Steepest raw DEM gradient over 25 m.
    pub raw_grade: f64,
    /// Steepest gradient after smoothing.
    pub smoothed_grade: f64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProfileStats {
    pub samples: usize,
    pub min_m: f64,
    pub max_m: f64,
    pub max_grade: f64,
    pub bridges_tunnels: usize,
    pub missing_dem: usize,
    pub suspicious: Vec<Suspicious>,
}

/// Steepest gradient over windows of `window` samples.
pub fn steepest(h: &[f64], window: usize) -> f64 {
    if h.len() <= window {
        return 0.0;
    }
    (0..h.len() - window)
        .map(|i| (h[i + window] - h[i]).abs() / (window as f64 * PROFILE_STEP_M))
        .fold(0.0, f64::max)
}

/// Computes `profile` for every edge of `graph`.
///
/// 1. Sample the DEM every 5 m; median filter; Gaussian smoothing per class.
/// 2. Junction heights: the mean of the smoothed ends of the roads meeting
///    there (bridges and tunnels excluded; DEM at the vertex if none), so
///    noisy DEM samples at a single point cannot force impossible gradients.
/// 3. Constrain each road to its class limits between those heights;
///    bridges and tunnels are linear between their ends.
pub fn add_profiles(graph: &mut RoadGraph, dem: &Dem, proj: &LocalProjection) -> ProfileStats {
    let height = |p: [f64; 2]| {
        let (lat, lon) = proj.unproject(p);
        dem.elevation(lat, lon)
    };
    let mut stats = ProfileStats {
        min_m: f64::INFINITY,
        max_m: f64::NEG_INFINITY,
        ..Default::default()
    };
    // Pass 1: raw and smoothed profiles.
    let mut raws = Vec::with_capacity(graph.edges.len());
    let mut smooths = Vec::with_capacity(graph.edges.len());
    for e in &graph.edges {
        let pts = resample(&e.points, PROFILE_STEP_M);
        let raw: Vec<Option<f64>> = pts.iter().map(|&p| height(p)).collect();
        if raw.iter().any(Option::is_none) {
            stats.missing_dem += 1;
        }
        let fallback = raw.iter().flatten().next().copied().unwrap_or(0.0);
        let raw: Vec<f64> = raw.iter().map(|v| v.unwrap_or(fallback)).collect();
        let sigma = smoothing_sigma_m(e.class) / PROFILE_STEP_M;
        smooths.push(gaussian(&median_filter(&raw, 2), sigma));
        raws.push(raw);
    }
    // Pass 2: junction heights by consensus.
    let mut sum = vec![(0.0, 0u32); graph.vertices.len()];
    for (e, s) in graph.edges.iter().zip(&smooths) {
        if e.bridge || e.tunnel {
            continue;
        }
        for (v, h) in [(e.from.0, s[0]), (e.to.0, *s.last().expect("non-empty"))] {
            sum[v as usize].0 += h;
            sum[v as usize].1 += 1;
        }
    }
    let vertex_h: Vec<f64> = sum
        .iter()
        .zip(&graph.vertices)
        .map(|(&(s, n), &p)| {
            if n > 0 {
                s / f64::from(n)
            } else {
                height(p).unwrap_or(0.0)
            }
        })
        .collect();
    // Pass 3: constrained profiles.
    for (i, e) in graph.edges.iter_mut().enumerate() {
        let len = e.length_m();
        let (raw, smooth) = (&raws[i], &smooths[i]);
        let (start, end) = (vertex_h[e.from.0 as usize], vertex_h[e.to.0 as usize]);
        let (max_grade, max_change) = limits(e.class);
        let profile = if e.bridge || e.tunnel {
            stats.bridges_tunnels += 1;
            distances(smooth.len(), len)
                .iter()
                .map(|x| start + (end - start) * x / len.max(1e-9))
                .collect()
        } else {
            constrain(smooth, len, start, end, max_grade, max_change)
        };
        let raw_grade = steepest(raw, 5);
        let smoothed_grade = steepest(&profile, 5);
        if !(e.bridge || e.tunnel) && raw_grade > max_grade + 0.10 && len > 50.0 {
            stats.suspicious.push(Suspicious {
                edge: i,
                name: e.name.clone(),
                raw_grade,
                smoothed_grade,
            });
        }
        stats.samples += profile.len();
        stats.max_grade = stats.max_grade.max(smoothed_grade);
        for &h in &profile {
            stats.min_m = stats.min_m.min(h);
            stats.max_m = stats.max_m.max(h);
        }
        e.profile = profile;
    }
    stats
        .suspicious
        .sort_by(|a, b| b.raw_grade.total_cmp(&a.raw_grade));
    stats
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampling_every_five_metres() {
        let pts = resample(&[[0.0, 0.0], [12.0, 0.0]], 5.0);
        assert_eq!(pts, vec![[0.0, 0.0], [5.0, 0.0], [10.0, 0.0], [12.0, 0.0]]);
        // Across a corner: 3 m + 4 m legs, 7 m total.
        let pts = resample(&[[0.0, 0.0], [3.0, 0.0], [3.0, 4.0]], 5.0);
        assert_eq!(pts.len(), 3);
        assert!((pts[1][0] - 3.0).abs() < 1e-9 && (pts[1][1] - 2.0).abs() < 1e-9);
        // Exactly on the step: no duplicate end point.
        assert_eq!(resample(&[[0.0, 0.0], [10.0, 0.0]], 5.0).len(), 3);
    }

    #[test]
    fn spikes_are_removed_and_limits_hold() {
        // 1 km climbing 8 % with a 30 m DEM spike, end heights fixed.
        let n = 201;
        let len = 1000.0;
        let mut raw: Vec<f64> = (0..n).map(|i| i as f64 * 5.0 * 0.08).collect();
        raw[100] += 30.0;
        let smooth = gaussian(
            &median_filter(&raw, 2),
            smoothing_sigma_m(RoadClass::Primary) / 5.0,
        );
        let h = constrain(&smooth, len, 0.0, 80.0, 0.15, 0.005);
        assert_eq!(h[0], 0.0);
        assert!((h[n - 1] - 80.0).abs() < 1e-9);
        assert!((h[100] - 40.0).abs() < 1.0, "spike gone: {}", h[100]);
        for w in h.windows(2) {
            assert!(((w[1] - w[0]) / 5.0).abs() <= 0.15 + 1e-6);
        }
        for w in h.windows(3) {
            let (g1, g2) = ((w[1] - w[0]) / 5.0, (w[2] - w[1]) / 5.0);
            // Plus the small linear end correction.
            assert!((g2 - g1).abs() <= 0.005 * 5.0 + 1e-6, "{g1} -> {g2}");
        }
    }

    #[test]
    fn a_wall_is_limited_to_the_class_gradient() {
        // A 30 % DEM wall on a main road: limited to 15 %.
        let raw: Vec<f64> = (0..41).map(|i| (i as f64 * 5.0 * 0.30).min(30.0)).collect();
        let h = constrain(&raw, 200.0, 0.0, 30.0, 0.15, 0.005);
        assert!(steepest(&h, 1) <= 0.15 + 1e-6, "{}", steepest(&h, 1));
        assert!((h[40] - 30.0).abs() < 1e-9);
        assert!(steepest(&raw, 1) > 0.29);
    }

    #[test]
    fn impossible_ends_fall_back_to_a_linear_rest() {
        // 50 m up in 100 m on a 15 % road cannot be done within the limit.
        let raw = vec![0.0; 21];
        let h = constrain(&raw, 100.0, 0.0, 50.0, 0.15, 0.005);
        assert!((h[20] - 50.0).abs() < 1e-9);
        assert!((steepest(&h, 1) - 0.5).abs() < 1e-6, "everything at 50 %");
    }

    #[test]
    fn class_limits() {
        assert_eq!(limits(RoadClass::Primary).0, 0.15);
        assert_eq!(limits(RoadClass::Path).0, 0.25);
        assert!(limits(RoadClass::Residential).1 > limits(RoadClass::Primary).1);
    }
}
