//! Fitting a route file's heights onto a world package's road surface
//! (spec §4.3: the road is the elevation reference).
//!
//! A route file (`world-builder route`) and a package built from different
//! terrain models disagree by metres: on 2026-11-13 the kiosk rode a route
//! file from swissALTI3D in a package rebuilt from SRTM, up to 15 m below
//! the drawn road before the Steg tunnel, so the rider sank into the road
//! and the tunnel finder (`ocw-render::tunnel`) saw a 1.7 km "tunnel"
//! instead of 0.65 km. Both come from the same OSM roads, so the route lies
//! on the package's road strips in plan view; only the heights differ.
//! [`fit_heights`] moves each route point onto the strip under it (the one
//! that continues the route's own height best), and carries the height
//! offset smoothly across stretches without strips (tunnels, gaps).

/// Largest height difference that counts as "on the road" (m): a route
/// that differs more somewhere is fitted.
pub const FIT_TOLERANCE_M: f64 = 0.25;

/// Largest change of the height offset from one route point to the next
/// with a strip under it (m); a strip further off is another road.
pub const MAX_STEP_M: f64 = 3.0;
/// … plus this much per metre since the last point on a strip (m/m).
pub const MAX_DRIFT: f64 = 0.05;

/// A point without an accepted strip snaps onto any strip this close to
/// its carried-over height (m).
pub const SNAP_M: f64 = 1.0;

/// The result of [`fit_heights`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HeightFit {
    /// Points with a road strip under them.
    pub on_roads: usize,
    /// Largest |route − road| before fitting (m).
    pub worst_m: f64,
    /// Mean route − road before fitting (m).
    pub mean_m: f64,
    /// Whether the heights were changed (worst above [`FIT_TOLERANCE_M`]).
    pub fitted: bool,
}

/// Moves the heights of `points` (x, y up, z) onto the road surface given
/// by `roads` (centre line heights of the road strips covering (x, z)),
/// if they differ anywhere by more than [`FIT_TOLERANCE_M`]. At each point
/// the strip nearest to the route height plus the offset found at the
/// previous point is taken (so a road crossing above or below does not
/// pull the route); between points with strips the offset is interpolated
/// linearly along the route, before the first and after the last it is
/// held. A fitted route is first resampled to points at most
/// [`DENSE_M`] apart, so it follows the strips' profile between its own
/// (sometimes sparse) points.
pub fn fit_heights(
    points: &mut Vec<[f64; 3]>,
    roads: impl Fn(f64, f64, Option<[f64; 2]>) -> Vec<f64>,
) -> HeightFit {
    let fit = fit_slice(points, &roads, false);
    if fit.fitted {
        *points = densify(points, DENSE_M);
        fit_slice(points, &roads, true);
    }
    fit
}

/// Points along a fitted route are at most this far apart (m), the
/// spacing of the package's road profiles.
pub const DENSE_M: f64 = 5.0;

/// `points` with points inserted on straight lines so that none are more
/// than `step` apart in plan.
fn densify(points: &[[f64; 3]], step: f64) -> Vec<[f64; 3]> {
    let mut out = Vec::with_capacity(points.len());
    for (i, p) in points.iter().enumerate() {
        if i > 0 {
            let q = points[i - 1];
            let len = ((p[0] - q[0]).powi(2) + (p[2] - q[2]).powi(2)).sqrt();
            let k = (len / step).ceil() as usize;
            for j in 1..k {
                let t = j as f64 / k as f64;
                out.push([0, 1, 2].map(|c| q[c] + (p[c] - q[c]) * t));
            }
        }
        out.push(*p);
    }
    out
}

fn fit_slice(
    points: &mut [[f64; 3]],
    roads: &impl Fn(f64, f64, Option<[f64; 2]>) -> Vec<f64>,
    apply: bool,
) -> HeightFit {
    let n = points.len();
    let mut offsets: Vec<Option<f64>> = vec![None; n];
    // The strips under a point along the route's direction there.
    let under = |i: usize| {
        let (a, b) = (points[i.saturating_sub(1)], points[(i + 1).min(n - 1)]);
        let (dx, dz) = (b[0] - a[0], b[2] - a[2]);
        let len = (dx * dx + dz * dz).sqrt().max(1e-9);
        roads(points[i][0], points[i][2], Some([dx / len, dz / len]))
    };
    // Start with the offset of the first point that has exactly one strip.
    let mut prev = (0..n).find_map(|i| match under(i).as_slice() {
        [h] => Some(h - points[i][1]),
        _ => None,
    });
    let along = cumulative_length(points);
    let mut prev_at = 0.0;
    for (i, p) in points.iter().enumerate() {
        let want = p[1] + prev.unwrap_or(0.0);
        // A strip far from where the route's road continues is another
        // road (over a tunnel, under a bridge): not this one. The two
        // terrain models drift apart along a gap (a tunnel), so the bound
        // grows with its length.
        let bound = MAX_STEP_M + MAX_DRIFT * (along[i] - prev_at);
        let best = under(i)
            .into_iter()
            .filter(|h| prev.is_none() || (h - want).abs() <= bound)
            .min_by(|a, b| (a - want).abs().total_cmp(&(b - want).abs()));
        if let Some(h) = best {
            offsets[i] = Some(h - p[1]);
            prev = offsets[i];
            prev_at = along[i];
        }
    }
    let known: Vec<(usize, f64)> = offsets
        .iter()
        .enumerate()
        .filter_map(|(i, o)| o.map(|o| (i, o)))
        .collect();
    let on_roads = known.len();
    let worst_m = known.iter().map(|(_, o)| o.abs()).fold(0.0, f64::max);
    let mean_m = if on_roads == 0 {
        0.0
    } else {
        -known.iter().map(|(_, o)| o).sum::<f64>() / on_roads as f64
    };
    let fitted = worst_m > FIT_TOLERANCE_M;
    if fitted && apply {
        let mut k = 0; // known[k] is the next known point at or after i
        for i in 0..n {
            while k < known.len() && known[k].0 < i {
                k += 1;
            }
            let offset = match (k.checked_sub(1).map(|j| known[j]), known.get(k)) {
                (_, Some(&(j, o))) if j == i => o,
                (Some((a, oa)), Some(&(b, ob))) => {
                    let t = (along[i] - along[a]) / (along[b] - along[a]).max(1e-9);
                    oa + (ob - oa) * t
                }
                (Some((_, o)), None) | (None, Some(&(_, o))) => o,
                (None, None) => 0.0,
            };
            let y = points[i][1] + offset;
            // Where the offset was carried across (no strip along the
            // route accepted: a junction, a hairpin), a strip close to it
            // is the road after all.
            let snapped = (offsets[i].is_none())
                .then(|| {
                    roads(points[i][0], points[i][2], None)
                        .into_iter()
                        .filter(|h| (h - y).abs() <= SNAP_M)
                        .min_by(|a, b| (a - y).abs().total_cmp(&(b - y).abs()))
                })
                .flatten();
            points[i][1] = snapped.unwrap_or(y);
        }
    }
    HeightFit {
        on_roads,
        worst_m,
        mean_m,
        fitted,
    }
}

fn cumulative_length(points: &[[f64; 3]]) -> Vec<f64> {
    let mut out = Vec::with_capacity(points.len());
    let mut s = 0.0;
    for (i, p) in points.iter().enumerate() {
        if i > 0 {
            let q = points[i - 1];
            s += ((p[0] - q[0]).powi(2) + (p[2] - q[2]).powi(2)).sqrt();
        }
        out.push(s);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A straight road along +x rising 5 %; its strip is 10 m lower than
    /// the route between x 0 … 400 and x 600 … 1000 (another terrain
    /// model); x 400 … 600 is a tunnel (no strip); a bridge 8 m above the
    /// route crosses at x 200.
    fn road(x: f64, _z: f64, _dir: Option<[f64; 2]>) -> Vec<f64> {
        let mut h = Vec::new();
        if !(400.0..600.0).contains(&x) {
            h.push(100.0 + 0.05 * x - 10.0 - if x >= 600.0 { 2.0 } else { 0.0 });
        }
        if (195.0..205.0).contains(&x) {
            h.push(100.0 + 0.05 * x + 8.0);
        }
        // A road on the hill 40 m above the tunnel: the only strip there.
        if (480.0..490.0).contains(&x) {
            h.push(100.0 + 0.05 * x + 40.0);
        }
        h
    }

    fn route() -> Vec<[f64; 3]> {
        (0..=200)
            .map(|i| {
                let x = f64::from(i) * 5.0;
                [x, 100.0 + 0.05 * x, 0.0]
            })
            .collect()
    }

    #[test]
    fn a_route_off_the_roads_is_moved_onto_them() {
        let mut pts = route();
        let fit = fit_heights(&mut pts, road);
        assert!(fit.fitted);
        assert!((fit.worst_m - 12.0).abs() < 1e-9, "{fit:?}");
        assert!(fit.mean_m > 9.9 && fit.mean_m < 12.0, "{fit:?}");
        for p in &pts {
            let x = p[0];
            let expected = if x < 400.0 {
                100.0 + 0.05 * x - 10.0
            } else if x >= 600.0 {
                100.0 + 0.05 * x - 12.0
            } else {
                // In the tunnel the offset goes from -10 to -12 m.
                let t = (x - 395.0) / (600.0 - 395.0);
                100.0 + 0.05 * x - 10.0 - 2.0 * t
            };
            // The bridge above (x 200) does not pull the route up.
            assert!((p[1] - expected).abs() < 1e-6, "{p:?} vs {expected}");
        }
    }

    /// A sparse route (points 50 m apart) over a crest between its points
    /// follows the strip's profile once fitted.
    #[test]
    fn a_sparse_route_follows_the_strip_between_its_points() {
        let crest = |x: f64, _z: f64, _d: Option<[f64; 2]>| {
            vec![90.0 + 2.0 * (1.0 - ((x - 25.0) / 25.0).powi(2))]
        };
        let mut pts = vec![[0.0, 100.0, 0.0], [50.0, 100.0, 0.0]];
        let fit = fit_heights(&mut pts, crest);
        assert!(fit.fitted);
        assert_eq!(pts.len(), 11);
        for p in &pts {
            assert!((p[1] - crest(p[0], 0.0, None)[0]).abs() < 1e-9, "{p:?}");
        }
    }

    #[test]
    fn a_route_on_the_roads_is_left_as_it_is() {
        let mut pts = route();
        let on = |x: f64, z: f64, d: Option<[f64; 2]>| -> Vec<f64> {
            road(x, z, d)
                .into_iter()
                .map(|h| h + 10.0 + if x >= 600.0 { 2.0 } else { 0.0 })
                .collect()
        };
        let before = pts.clone();
        let fit = fit_heights(&mut pts, on);
        assert!(!fit.fitted, "{fit:?}");
        assert!(fit.worst_m < 1e-9);
        assert_eq!(pts, before);
        // Without any roads nothing changes either.
        let fit = fit_heights(&mut pts, |_, _, _| Vec::new());
        assert_eq!((fit.on_roads, fit.fitted), (0, false));
        assert_eq!(pts, before);
    }
}
