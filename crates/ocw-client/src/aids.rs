//! The two optional riding aids of spec §14 Phase 12D, pure logic for the
//! HUD's graphics (`ocw_render::aids_hud`); neither steers for the rider:
//!
//! - the **"last moment" warning**: an amber flash about [`WARN_S`] before
//!   a predicted hit – the own footprint and the others' moved on with
//!   their present velocities ([`time_to_contact`]) – and while the tyres
//!   slide on a slippery road (spec §14 Phase 9 S6: a lean beyond the
//!   grip, [`ocw_sim::collide::skid`]);
//! - the **proximity arrow**: an arrow pointing at the nearest rider close
//!   beside ([`BESIDE_M`]), in front ([`AHEAD_M`]) or behind ([`BEHIND_M`]),
//!   with its direction ([`proximity_direction`]).
//!
//! Positions are relative to the own rider in its path frame: metres
//! ahead along the road and metres to the right.

use ocw_render::{HitWarningView, WarningKind};
use ocw_sim::collide::{BODY_LENGTH_M, BODY_WIDTH_M, FALLEN_LENGTH_M, FALLEN_WIDTH_M};

/// The warning comes this long before a predicted hit (s).
pub const WARN_S: f64 = 1.0;
/// The prediction's step (s).
const STEP_S: f64 = 0.05;
/// A little room around the footprints for the prediction (m): a near
/// miss warns too.
pub const MARGIN_M: f64 = 0.1;
/// The proximity arrow: someone within this far beside (m, between the
/// footprints) …
pub const BESIDE_M: f64 = 1.0;
/// … this far in front (m, between the bodies) …
pub const AHEAD_M: f64 = 2.0;
/// … or this far behind (m, between the bodies).
pub const BEHIND_M: f64 = 2.0;
/// A skid warns from this much slip on (S6 → 12B; 1 = the fall).
pub const SLIP_WARN: f64 = 0.05;

/// Another rider (or a fallen bike) relative to the own rider.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Near {
    /// Metres ahead along the road (negative: behind) and to the right.
    pub ahead_m: f64,
    pub right_m: f64,
    pub speed: f64,
    /// Direction of travel relative to the road (rad, positive = right).
    pub heading: f64,
    /// Lying on the road (12B: a 1.8 × 1 m obstacle).
    pub fallen: bool,
}

/// The own rider's motion (speed m/s, heading to the road rad).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Me {
    pub speed: f64,
    pub heading: f64,
}

fn velocity(speed: f64, heading: f64) -> (f64, f64) {
    (speed * heading.cos(), speed * heading.sin())
}

/// The first time within [`WARN_S`] at which the footprints (moved on with
/// their velocities) touch, and the direction to the other rider then
/// (rad: 0 ahead, positive to the right, ±π behind). Riders already
/// touching (a bump under way) do not warn.
pub fn time_to_contact(me: Me, others: &[Near]) -> Option<(f64, f64)> {
    let (mv, md) = velocity(me.speed, me.heading);
    let mut best: Option<(f64, f64)> = None;
    for o in others {
        let (ov, od) = if o.fallen {
            (0.0, 0.0)
        } else {
            velocity(o.speed, o.heading)
        };
        let (len, wid) = if o.fallen {
            (FALLEN_LENGTH_M, FALLEN_WIDTH_M)
        } else {
            (BODY_LENGTH_M, BODY_WIDTH_M)
        };
        let reach_s = (BODY_LENGTH_M + len) / 2.0 + MARGIN_M;
        let reach_d = (BODY_WIDTH_M + wid) / 2.0 + MARGIN_M;
        let at = |t: f64| (o.ahead_m + (ov - mv) * t, o.right_m + (od - md) * t);
        let touch = |(s, d): (f64, f64)| s.abs() < reach_s && d.abs() < reach_d;
        if touch(at(0.0)) {
            continue;
        }
        let mut t = STEP_S;
        while t <= WARN_S + 1e-9 {
            let p = at(t);
            if touch(p) {
                if best.is_none_or(|(b, _)| t < b) {
                    best = Some((t, p.1.atan2(p.0)));
                }
                break;
            }
            t += STEP_S;
        }
    }
    best
}

/// The direction to the nearest rider close beside, in front or behind
/// (rad: 0 ahead, positive to the right, ±π behind); `None` when nobody
/// is that close.
pub fn proximity_direction(others: &[Near]) -> Option<f64> {
    let (len, wid) = (BODY_LENGTH_M, BODY_WIDTH_M);
    others
        .iter()
        .filter(|o| !o.fallen)
        .filter_map(|o| {
            let gap_along = o.ahead_m.abs() - len;
            let beside = o.ahead_m.abs() < len && o.right_m.abs() < wid + BESIDE_M;
            let front = o.ahead_m > 0.0 && gap_along < AHEAD_M && o.right_m.abs() < wid;
            let back = o.ahead_m < 0.0 && gap_along < BEHIND_M && o.right_m.abs() < wid;
            (beside || front || back)
                .then(|| (o.ahead_m.hypot(o.right_m), o.right_m.atan2(o.ahead_m)))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, angle)| angle)
}

/// The warning to show now: a slide first (the own tyres; `slip` from the
/// prediction), else a predicted hit within [`WARN_S`].
pub fn hit_warning(me: Me, others: &[Near], slip: f64) -> Option<HitWarningView> {
    if slip >= SLIP_WARN {
        return Some(HitWarningView {
            kind: WarningKind::Slide,
            in_s: 0.0,
            angle: 0.0,
        });
    }
    let (t, angle) = time_to_contact(me, others)?;
    Some(HitWarningView {
        kind: WarningKind::Touch,
        in_s: t as f32,
        angle: angle as f32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rider(ahead_m: f64, right_m: f64, speed: f64) -> Near {
        Near {
            ahead_m,
            right_m,
            speed,
            heading: 0.0,
            fallen: false,
        }
    }

    const ME: Me = Me {
        speed: 10.0,
        heading: 0.0,
    };

    #[test]
    fn warns_about_a_second_before_a_hit() {
        // A rider 7 m ahead at 4 m/s less: the gap of 5.25 m closes in
        // about 1.3 s – no warning yet; at 5.5 m ahead (≈ 0.9 s) it warns.
        assert_eq!(time_to_contact(ME, &[rider(7.0, 0.0, 6.0)]), None);
        let (t, angle) = time_to_contact(ME, &[rider(5.5, 0.0, 6.0)]).unwrap();
        assert!((0.8..=1.0).contains(&t), "{t}");
        assert!(angle.abs() < 0.2, "ahead: {angle}");
        // The same speed: never.
        assert_eq!(time_to_contact(ME, &[rider(3.0, 0.0, 10.0)]), None);
        // Beside in another line: never.
        assert_eq!(time_to_contact(ME, &[rider(4.0, 1.5, 6.0)]), None);
        // Already touching (a bump under way): no warning.
        assert_eq!(time_to_contact(ME, &[rider(1.0, 0.0, 6.0)]), None);
    }

    #[test]
    fn steering_into_a_rider_beside_warns_with_its_side() {
        // A rider 1 m to the left; the own rider steers left at 0.1 rad.
        let me = Me {
            speed: 8.0,
            heading: -0.1,
        };
        let (t, angle) = time_to_contact(me, &[rider(0.0, -1.0, 8.0)]).unwrap();
        assert!(t < 0.6, "{t}");
        assert!(angle < -1.2, "to the left: {angle}");
        // A fallen bike ahead is a hit too (it stands still).
        let fallen = Near {
            fallen: true,
            speed: 0.0,
            ..rider(10.0, 0.3, 0.0)
        };
        let (t, _) = time_to_contact(ME, &[fallen]).unwrap();
        assert!((0.75..=0.9).contains(&t), "{t}");
        // The nearest in time wins.
        let (t, _) = time_to_contact(ME, &[fallen, rider(4.0, 0.0, 4.0)]).unwrap();
        assert!(t < 0.5, "{t}");
    }

    #[test]
    fn the_arrow_points_at_the_rider_close_by() {
        assert_eq!(proximity_direction(&[]), None);
        // Beside on the right (0.9 m between the centres).
        let a = proximity_direction(&[rider(0.2, 0.9, 10.0)]).unwrap();
        assert!((a - std::f64::consts::FRAC_PI_2).abs() < 0.3, "{a}");
        // Close behind on the left: pointing back-left.
        let a = proximity_direction(&[rider(-2.5, -0.2, 10.0)]).unwrap();
        assert!(a < -2.5, "{a}");
        // In front.
        let a = proximity_direction(&[rider(3.0, 0.1, 10.0)]).unwrap();
        assert!(a.abs() < 0.1, "{a}");
        // Too far beside, ahead or behind: nothing.
        assert_eq!(proximity_direction(&[rider(0.0, 1.8, 10.0)]), None);
        assert_eq!(proximity_direction(&[rider(4.0, 0.0, 10.0)]), None);
        assert_eq!(proximity_direction(&[rider(-4.0, 0.0, 10.0)]), None);
        // The nearest of several.
        let a = proximity_direction(&[rider(3.5, 0.0, 10.0), rider(0.0, -1.0, 10.0)]).unwrap();
        assert!(a < -1.0, "{a}");
    }

    #[test]
    fn a_slide_warns_first() {
        let w = hit_warning(ME, &[rider(5.5, 0.0, 6.0)], 0.3).unwrap();
        assert_eq!(w.kind, WarningKind::Slide);
        let w = hit_warning(ME, &[rider(5.5, 0.0, 6.0)], 0.0).unwrap();
        assert_eq!(w.kind, WarningKind::Touch);
        assert!(w.in_s > 0.0 && w.in_s <= 1.0);
        assert_eq!(hit_warning(ME, &[], 0.0), None);
    }
}
