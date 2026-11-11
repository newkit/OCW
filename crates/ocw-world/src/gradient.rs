//! Gradient look-ahead for the HUD and the camera (spec §3.2 "Gradient
//! feel"): the average gradient over the next metres, an elevation profile
//! of the road around the rider, and the gradient bands it is coloured by.
//!
//! Everything is a function of a [`Track`] and the rider's distance, so it
//! works on routes and on free-ride paths alike. In free ride the path is
//! only known up to the next undecided junction; pass the
//! [`RidePath::preview`](crate::RidePath::preview) (the default exits
//! ahead) to see further. Beyond a track's [`Track::extent`] nothing is
//! sampled: the profile ends there, and averages use the part that is
//! known.

use crate::track::Track;

/// Look-ahead of the HUD's average gradient (m).
pub const AVERAGE_AHEAD_M: f64 = 100.0;

/// Shortest known stretch an average is computed over (m); a track that
/// ends sooner gives the gradient at the rider.
pub const MIN_AVERAGE_M: f64 = 10.0;

/// The part of `[from, from + len]` that `track` knows (see
/// [`Track::extent`]), or `None` if less than `min_len` of it.
fn known_span(track: &dyn Track, from: f64, len: f64, min_len: f64) -> Option<(f64, f64)> {
    let (a, b) = match track.extent() {
        Some((lo, hi)) => (from.max(lo), (from + len).min(hi)),
        None => (from, from + len),
    };
    (b - a >= min_len).then_some((a, b))
}

/// Average gradient (rise over run, 0.05 = 5 %) of `track` from `from`
/// over the next `ahead_m` metres: the height difference over the
/// horizontal distance. Where the track ends sooner (a route's finish, the
/// end of a free-ride path) the known part counts; under
/// [`MIN_AVERAGE_M`] of it, the gradient at `from`.
pub fn average_grade(track: &dyn Track, from: f64, ahead_m: f64) -> f64 {
    match known_span(track, from, ahead_m.max(0.0), MIN_AVERAGE_M) {
        Some((a, b)) => (track.sample(b).position[1] - track.sample(a).position[1]) / (b - a),
        None => track.sample(from).grade,
    }
}

/// Gradient bands of the elevation profile (spec §3.2). The limits are the
/// usual ones of cycling climb profiles: descents, flat to gentle, a
/// steady climb, hard, very hard, and walls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GradeBand {
    /// Below −2 %.
    Descent,
    /// −2 % up to 3 %.
    Flat,
    /// 3 % up to 6 %.
    Moderate,
    /// 6 % up to 9 %.
    Hard,
    /// 9 % up to 12 %.
    VeryHard,
    /// 12 % and more.
    Wall,
}

impl GradeBand {
    /// The band of gradient `grade` (rise over run).
    pub fn of(grade: f64) -> Self {
        let pct = grade * 100.0;
        if pct < -2.0 {
            Self::Descent
        } else if pct < 3.0 {
            Self::Flat
        } else if pct < 6.0 {
            Self::Moderate
        } else if pct < 9.0 {
            Self::Hard
        } else if pct < 12.0 {
            Self::VeryHard
        } else {
            Self::Wall
        }
    }
}

/// What part of the road an [`ElevationProfile`] shows and how.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProfileSpec {
    /// Shown behind the rider (m).
    pub behind_m: f64,
    /// Shown ahead of the rider (m).
    pub ahead_m: f64,
    /// Distance between samples (m).
    pub spacing_m: f64,
    /// Smallest height range the profile is scaled to (m): a flat road
    /// stays flat instead of its last metre filling the box.
    pub min_range_m: f64,
}

impl Default for ProfileSpec {
    /// The HUD's: 150 m behind, 1.5 km ahead, every 15 m, at least 40 m
    /// high (on a 300 × 60 px box about 8× vertical exaggeration).
    fn default() -> Self {
        Self {
            behind_m: 150.0,
            ahead_m: 1500.0,
            spacing_m: 15.0,
            min_range_m: 40.0,
        }
    }
}

/// Elevation of the road around the rider, for the HUD's mini profile.
#[derive(Debug, Clone, PartialEq)]
pub struct ElevationProfile {
    /// Track distance of the first slot (m): rider − `behind_m`, rounded
    /// down to a multiple of `spacing_m`.
    pub start_m: f64,
    pub spacing_m: f64,
    /// Road height (m) per slot, every `spacing_m` from `start_m`; `None`
    /// where the track is not known (before its start, after its end).
    pub heights: Vec<Option<f64>>,
    /// The rider's position as a fraction of the width (0 … 1).
    pub rider_frac: f64,
    /// Height range the profile is drawn in (m): the heights' range,
    /// widened around its middle to at least `min_range_m`.
    pub low: f64,
    pub high: f64,
}

impl ElevationProfile {
    /// Samples `track` around a rider at `rider_m`.
    pub fn sample(track: &dyn Track, rider_m: f64, spec: &ProfileSpec) -> Self {
        let spacing = spec.spacing_m.max(0.1);
        // Slots on a fixed grid of the track distance: they stay put on the
        // road while the rider moves (no shimmer from resampling), and the
        // window moves on by one slot at a time.
        let start = ((rider_m - spec.behind_m.max(0.0)) / spacing).floor() * spacing;
        let n = ((spec.behind_m.max(0.0) + spec.ahead_m.max(0.0)) / spacing).round() as usize + 1;
        let width = (n - 1) as f64 * spacing;
        let extent = track.extent();
        let heights: Vec<Option<f64>> = (0..n)
            .map(|i| {
                let d = start + i as f64 * spacing;
                let known = extent.is_none_or(|(lo, hi)| d >= lo - 1e-6 && d <= hi + 1e-6);
                known.then(|| track.sample(d).position[1])
            })
            .collect();
        let (lo, hi) = heights
            .iter()
            .flatten()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &h| {
                (lo.min(h), hi.max(h))
            });
        let (low, high) = if lo > hi {
            (0.0, spec.min_range_m.max(1.0))
        } else {
            scaled_range(lo, hi, spec.min_range_m)
        };
        Self {
            start_m: start,
            spacing_m: spacing,
            heights,
            rider_frac: if width > 0.0 {
                ((rider_m - start) / width).clamp(0.0, 1.0)
            } else {
                0.0
            },
            low,
            high,
        }
    }

    /// Height of slot `i` as a fraction of the box (0 = bottom, 1 = top);
    /// the lowest point keeps `base` so it still shows.
    pub fn level(&self, i: usize, base: f64) -> Option<f64> {
        let h = self.heights.get(i).copied().flatten()?;
        let t = ((h - self.low) / (self.high - self.low)).clamp(0.0, 1.0);
        Some(base + (1.0 - base) * t)
    }

    /// Gradient from slot `i` to the next (rise over run); `None` at the
    /// end or where either is unknown.
    pub fn grade(&self, i: usize) -> Option<f64> {
        let a = self.heights.get(i).copied().flatten()?;
        let b = self.heights.get(i + 1).copied().flatten()?;
        Some((b - a) / self.spacing_m)
    }

    /// Climbing and descending (m, both ≥ 0) ahead of the rider over the
    /// known part of the profile.
    pub fn ahead_gain_loss(&self) -> (f64, f64) {
        let first = ((self.heights.len() - 1) as f64 * self.rider_frac).round() as usize;
        let known: Vec<f64> = self.heights[first..].iter().map_while(|h| *h).collect();
        known.windows(2).fold((0.0, 0.0), |(up, down), w| {
            let d = w[1] - w[0];
            (up + d.max(0.0), down + (-d).max(0.0))
        })
    }
}

/// `(lo, hi)` widened around its middle to at least `min_range`.
pub fn scaled_range(lo: f64, hi: f64, min_range: f64) -> (f64, f64) {
    let range = hi - lo;
    if range >= min_range {
        (lo, hi)
    } else {
        let mid = (lo + hi) / 2.0;
        (mid - min_range / 2.0, mid + min_range / 2.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::route::Route;

    /// A straight road along x: flat for 200 m, then +8 % for 500 m,
    /// then −4 % for 300 m (heights 0 → 40 → 28 m).
    fn ramp() -> Route {
        let pts = (0..=200)
            .map(|i| {
                let x = f64::from(i) * 5.0;
                let y = if x <= 200.0 {
                    0.0
                } else if x <= 700.0 {
                    (x - 200.0) * 0.08
                } else {
                    40.0 - (x - 700.0) * 0.04
                };
                [x, y, 0.0]
            })
            .collect();
        Route::from_points("ramp", "Ramp", pts, false, 6.0).unwrap()
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn average_gradient_ahead() {
        let r = ramp();
        assert!(close(average_grade(&r, 0.0, 100.0), 0.0));
        assert!(close(average_grade(&r, 300.0, 100.0), 0.08));
        // Half flat, half climbing: the average.
        assert!(close(average_grade(&r, 150.0, 100.0), 0.04));
        // Over the top: +8 % for 50 m, −4 % for 50 m.
        assert!(close(average_grade(&r, 650.0, 100.0), 0.02));
        // The route ends after 60 m: the known 60 m count.
        assert!(close(average_grade(&r, 940.0, 100.0), -0.04));
        // At the very end: the gradient there.
        assert!(close(
            average_grade(&r, 999.0, 100.0),
            r.sample(999.0).grade
        ));
    }

    #[test]
    fn average_gradient_on_a_loop_wraps() {
        let r = Route::test_hills();
        let len = r.length();
        let expected = (r.sample(50.0).position[1] - r.sample(len - 50.0).position[1]) / 100.0;
        assert!((average_grade(&r, len - 50.0, 100.0) - expected).abs() < 1e-9);
    }

    #[test]
    fn grade_bands() {
        use GradeBand::*;
        let pct = |p: f64| GradeBand::of(p / 100.0);
        assert_eq!(pct(-8.0), Descent);
        assert_eq!(pct(-2.0), Flat);
        assert_eq!(pct(0.0), Flat);
        assert_eq!(pct(2.9), Flat);
        assert_eq!(pct(3.0), Moderate);
        assert_eq!(pct(6.0), Hard);
        assert_eq!(pct(9.4), VeryHard);
        assert_eq!(pct(12.0), Wall);
        assert_eq!(pct(25.0), Wall);
        assert!(Descent < Flat && Hard < Wall);
    }

    #[test]
    fn profile_samples_around_the_rider() {
        let r = ramp();
        let spec = ProfileSpec {
            behind_m: 100.0,
            ahead_m: 400.0,
            spacing_m: 50.0,
            min_range_m: 10.0,
        };
        let p = ElevationProfile::sample(&r, 300.0, &spec);
        assert_eq!(p.heights.len(), 11);
        assert!(close(p.start_m, 200.0));
        assert!(close(p.rider_frac, 0.2));
        assert!(close(p.heights[0].unwrap(), 0.0));
        assert!(close(p.heights[10].unwrap(), 40.0));
        assert_eq!((p.low, p.high), (0.0, 40.0));
        assert!(close(p.level(0, 0.1).unwrap(), 0.1));
        assert!(close(p.level(10, 0.1).unwrap(), 1.0));
        assert!(close(p.grade(3).unwrap(), 0.08));
        assert_eq!(p.grade(10), None);
        let (up, down) = p.ahead_gain_loss();
        assert!(close(up, 32.0) && close(down, 0.0), "{up} {down}");
    }

    #[test]
    fn profile_stops_where_the_track_is_unknown() {
        let r = ramp();
        let p = ElevationProfile::sample(&r, 50.0, &ProfileSpec::default());
        // 150 m behind a rider at 50 m: the first slots (from −105 m) are
        // before the start.
        assert!(close(p.start_m, -105.0));
        assert!(close(p.rider_frac, 155.0 / 1650.0));
        let unknown_before = p.heights.iter().take_while(|h| h.is_none()).count();
        assert_eq!(unknown_before, 7);
        // The slots stay on the same grid while the rider moves.
        let q = ElevationProfile::sample(&r, 58.0, &ProfileSpec::default());
        assert_eq!((q.start_m, q.heights.len()), (p.start_m, p.heights.len()));
        assert!(q.rider_frac > p.rider_frac);
        let p = ElevationProfile::sample(&r, 900.0, &ProfileSpec::default());
        // Only 100 m ahead are known.
        let known = p.heights.iter().flatten().count();
        assert_eq!(known, (250.0f64 / 15.0).floor() as usize + 1);
        assert!(p.heights.last().unwrap().is_none());
        assert_eq!(p.level(p.heights.len() - 1, 0.1), None);
        assert!(p.ahead_gain_loss().1 > 3.0);
    }

    #[test]
    fn flat_roads_stay_flat() {
        // 2 m of height difference in a 40 m minimum range.
        assert_eq!(scaled_range(100.0, 102.0, 40.0), (81.0, 121.0));
        assert_eq!(scaled_range(0.0, 80.0, 40.0), (0.0, 80.0));
        let r = ramp();
        let spec = ProfileSpec {
            behind_m: 0.0,
            ahead_m: 150.0,
            ..ProfileSpec::default()
        };
        let p = ElevationProfile::sample(&r, 0.0, &spec);
        assert_eq!((p.low, p.high), (-20.0, 20.0));
        assert!((0..p.heights.len()).all(|i| close(p.level(i, 0.0).unwrap(), 0.5)));
        assert_eq!(p.rider_frac, 0.0);
        assert_eq!(p.ahead_gain_loss(), (0.0, 0.0));
    }
}
