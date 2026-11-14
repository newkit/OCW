//! Drafting (spec §5.2): riding behind others reduces the air resistance.
//!
//! The model is deliberately simple and smooth (no jumps when riders move):
//! - the wheel gap to a rider ahead reduces drag by up to 30 %, falling off
//!   exponentially (≈ 4 % at 10 m, none beyond 15 m);
//! - lateral offset reduces the effect with a Gaussian of 1.2 m width (one
//!   lane over still gives partial shelter);
//! - a second rider ahead adds 30 % of its own effect (bunch effect), with
//!   the total reduction capped at 40 %.
//!
//! The server applies it to all riders; the client predicts its own rider
//! with the positions of the others it knows.

/// Drag reduction directly on the wheel of the rider ahead.
pub const MAX_SINGLE_REDUCTION: f64 = 0.30;
/// Cap for the total reduction in a bunch.
pub const MAX_TOTAL_REDUCTION: f64 = 0.40;
/// Distance over which the effect falls to 1/e.
pub const DECAY_M: f64 = 5.0;
/// No effect beyond this wheel gap.
pub const MAX_GAP_M: f64 = 15.0;
/// Lateral width of the shelter.
pub const LATERAL_WIDTH_M: f64 = 1.2;
/// Bicycle length: distance between two riders' reference points at zero wheel gap.
pub const BIKE_LENGTH_M: f64 = 1.8;

/// Drag reduction (0–`MAX_SINGLE_REDUCTION`) from one rider `ahead_m` metres
/// further along the road (reference point to reference point) and
/// `lateral_m` to the side.
pub fn reduction(ahead_m: f64, lateral_m: f64) -> f64 {
    let gap = ahead_m - BIKE_LENGTH_M;
    if !(0.0..=MAX_GAP_M).contains(&gap) {
        // Behind us, side by side (overlapping wheels) or too far ahead.
        return 0.0;
    }
    // Fade out towards MAX_GAP_M so there is no step at the cut-off.
    let fade = 1.0 - (gap / MAX_GAP_M).powi(4);
    let lateral = (-(lateral_m / LATERAL_WIDTH_M).powi(2)).exp();
    MAX_SINGLE_REDUCTION * (-gap / DECAY_M).exp() * fade * lateral
}

/// Factor on the aerodynamic drag (1.0 = no draft) for a rider at
/// `(distance, lateral)` among `others`.
pub fn drag_factor(me: (f64, f64), others: impl IntoIterator<Item = (f64, f64)>) -> f64 {
    let (mut best, mut second) = (0.0f64, 0.0f64);
    for (d, l) in others {
        let r = reduction(d - me.0, l - me.1);
        if r > best {
            second = best;
            best = r;
        } else if r > second {
            second = r;
        }
    }
    1.0 - (best + 0.3 * second).min(MAX_TOTAL_REDUCTION)
}

/// [`drag_factor`] in a crosswind (spec §14 Phase 9, S6): the wake of each
/// rider ahead drifts `drift` metres sideways per metre behind it
/// (positive = to the right, [`crate::weather::wake_drift`]), so the
/// shelter lies on the lee side – echelons. `drift` 0 is [`drag_factor`].
pub fn drag_factor_in_wind(
    me: (f64, f64),
    others: impl IntoIterator<Item = (f64, f64)>,
    drift: f64,
) -> f64 {
    drag_factor(
        me,
        others.into_iter().map(|(d, l)| (d, l + drift * (d - me.0))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alone_or_ahead_gives_no_draft() {
        assert_eq!(drag_factor((100.0, 0.0), []), 1.0);
        // The only other rider is behind.
        assert_eq!(drag_factor((100.0, 0.0), [(95.0, 0.0)]), 1.0);
        // Side by side (overlapping) gives nothing.
        assert_eq!(drag_factor((100.0, 0.0), [(101.0, 0.0)]), 1.0);
        // Too far ahead.
        assert_eq!(drag_factor((100.0, 0.0), [(130.0, 0.0)]), 1.0);
    }

    #[test]
    fn reduction_falls_off_with_gap_and_offset() {
        let on_wheel = reduction(BIKE_LENGTH_M + 0.2, 0.0);
        assert!((0.28..=0.30).contains(&on_wheel), "{on_wheel}");
        let ten_m = reduction(BIKE_LENGTH_M + 10.0, 0.0);
        assert!((0.03..=0.045).contains(&ten_m), "{ten_m}");
        assert!(reduction(BIKE_LENGTH_M + 14.99, 0.0) < 0.002);
        // One lane over (1.2 m): about a third.
        let side = reduction(BIKE_LENGTH_M + 0.2, 1.2) / on_wheel;
        assert!((0.3..0.4).contains(&side), "{side}");
        // Monotone in the gap.
        let mut last = f64::INFINITY;
        for i in 0..150 {
            let r = reduction(BIKE_LENGTH_M + f64::from(i) * 0.1, 0.3);
            assert!(r <= last + 1e-12);
            last = r;
        }
    }

    /// Spec §14 Phase 9 9G: drafting offset in a crosswind.
    #[test]
    fn crosswind_moves_the_shelter_downwind() {
        let ahead = [(BIKE_LENGTH_M + 0.5, 0.0)];
        let behind = |l: f64, drift: f64| drag_factor_in_wind((0.0, l), ahead, drift);
        // Still air: straight behind is best.
        assert_eq!(behind(0.0, 0.0), drag_factor((0.0, 0.0), ahead));
        assert!(behind(0.0, 0.0) < behind(0.8, 0.0));
        // Wind from the left (the wake drifts right by 0.4 m per metre):
        // the best place is to the right of the wheel ahead, the place
        // straight behind loses most of the shelter.
        let drift = 0.4;
        let best = (-20..=40)
            .map(|i| f64::from(i) * 0.05)
            .min_by(|a, b| behind(*a, drift).total_cmp(&behind(*b, drift)))
            .unwrap();
        let expect = drift * (BIKE_LENGTH_M + 0.5);
        assert!((best - expect).abs() < 0.06, "{best} vs {expect}");
        assert!(behind(0.0, drift) > behind(expect, drift) + 0.08);
        assert!((behind(expect, drift) - behind(0.0, 0.0)).abs() < 1e-12);
        // Wind from the right: the shelter is on the left.
        assert!(behind(-expect, -drift) < behind(expect, -drift));
    }

    #[test]
    fn bunch_helps_but_is_capped() {
        let one = drag_factor((0.0, 0.0), [(2.0, 0.0)]);
        let two = drag_factor((0.0, 0.0), [(2.0, 0.0), (4.0, 0.0)]);
        assert!(two < one);
        let many: Vec<(f64, f64)> = (1..20).map(|i| (2.0 + f64::from(i) * 0.1, 0.0)).collect();
        assert!(drag_factor((0.0, 0.0), many) >= 1.0 - MAX_TOTAL_REDUCTION - 1e-12);
    }
}
