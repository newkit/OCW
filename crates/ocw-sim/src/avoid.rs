//! Autodrive avoidance (spec §14 Phase 12, 12D): autodrive riders and
//! every bot look ahead along their path for riders and fallen bikes in
//! their line. When the time to collision falls below [`TTC_S`] they move
//! their line to the free side – overtaking on the region's side (the left
//! in right-hand traffic countries), on narrow roads either side – and
//! move back once past; when both sides are blocked they hold back at a
//! [`HOLD_GAP_M`] wheel gap (braking virtually: the speed above the one in
//! front is shed; the rider's power still counts, so a strong rider sits
//! on the wheel until there is a gap).
//!
//! Pure; the same code on the server (every autodrive rider and bot) and
//! on the client (the own rider's prediction), so they agree. Manual
//! riders get no help.

use crate::collide::{BODY_LENGTH_M, BODY_WIDTH_M};

/// Look-ahead: this many seconds of riding, at least [`MIN_LOOKAHEAD_M`].
pub const LOOKAHEAD_S: f64 = 4.0;
pub const MIN_LOOKAHEAD_M: f64 = 15.0;
/// Below this time to collision autodrive moves its line (s).
pub const TTC_S: f64 = 3.0;
/// The wheel gap autodrive holds behind a rider it cannot pass (m).
pub const HOLD_GAP_M: f64 = 0.5;
/// How strongly the gap is held: the allowed speed above the one in front
/// per metre of gap beyond [`HOLD_GAP_M`] (1/s).
pub const HOLD_GAIN: f64 = 1.5;
/// Clearance kept beside the other rider when passing, between the
/// footprints (m).
pub const PASS_CLEARANCE_M: f64 = 0.4;
/// Riders this close sideways (between footprints, m) are in the line.
pub const LINE_CLEARANCE_M: f64 = 0.25;
/// Roads narrower than this may be passed on either side (m).
pub const NARROW_ROAD_M: f64 = 4.5;
/// How long a change of line takes, for riders closing from behind (s).
pub const CHANGE_S: f64 = 2.0;

/// Another rider (or a fallen bike) seen from the avoiding rider, in its
/// path frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Other {
    /// Distance ahead along the path (m; negative = behind).
    pub ahead_m: f64,
    /// Lateral offset (m, positive = right).
    pub lateral_m: f64,
    /// The line it steers to (its lateral target; a fallen bike: where it lies).
    pub target_m: f64,
    pub speed: f64,
    /// Half its footprint's length and width (m).
    pub half_length: f64,
    pub half_width: f64,
}

impl Other {
    /// A riding rider.
    pub fn rider(ahead_m: f64, lateral_m: f64, target_m: f64, speed: f64) -> Self {
        Self {
            ahead_m,
            lateral_m,
            target_m,
            speed,
            half_length: BODY_LENGTH_M / 2.0,
            half_width: BODY_WIDTH_M / 2.0,
        }
    }

    /// A fallen rider and bike lying on the road.
    pub fn fallen(ahead_m: f64, lateral_m: f64) -> Self {
        Self {
            ahead_m,
            lateral_m,
            target_m: lateral_m,
            speed: 0.0,
            half_length: crate::collide::FALLEN_LENGTH_M / 2.0,
            half_width: crate::collide::FALLEN_WIDTH_M / 2.0,
        }
    }
}

/// The avoiding rider.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Me {
    pub lateral_m: f64,
    pub speed: f64,
    /// The rider's own line (the autodrive target the rider chose).
    pub lane_m: f64,
    /// Largest offset either side ([`crate::physics::lateral_limit`]).
    pub limit_m: f64,
    pub road_width_m: f64,
}

/// What autodrive keeps between ticks: the line it moved to while passing.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AvoidState {
    pub pass_m: Option<f64>,
}

/// This tick's avoidance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plan {
    /// The lateral target to steer to.
    pub target_m: f64,
    /// Ride no faster than this (m/s; the virtual brake), if set.
    pub speed_cap: Option<f64>,
    /// Blocked on both sides: holding back behind a rider.
    pub holding: bool,
}

/// The overtaking side from the region's traffic side: `true` = pass on
/// the left (right-hand traffic).
pub fn overtake_left(left_hand_traffic: bool) -> bool {
    !left_hand_traffic
}

fn half_w() -> f64 {
    BODY_WIDTH_M / 2.0
}

fn half_l() -> f64 {
    BODY_LENGTH_M / 2.0
}

/// Whether `o` lies (now or within `horizon` s, closing at its relative
/// speed) between `from` and `to` metres ahead.
fn within(o: &Other, me: &Me, from: f64, to: f64, horizon: f64) -> bool {
    let later = o.ahead_m + (o.speed - me.speed) * horizon;
    let (lo, hi) = (o.ahead_m.min(later), o.ahead_m.max(later));
    hi >= from && lo <= to
}

/// Whether `o` is in the line at `lane` (by where it is or where it goes).
fn in_line(o: &Other, lane: f64, extra: f64) -> bool {
    let clear = half_w() + o.half_width + extra;
    (o.lateral_m - lane).abs() < clear || (o.target_m - lane).abs() < clear
}

/// Whether the line at `lane` is free from just behind to the look-ahead,
/// and nobody rides beside between the rider and it.
fn free(lane: f64, me: &Me, others: &[Other], ahead_m: f64) -> bool {
    if lane.abs() > me.limit_m + 1e-9 {
        return false;
    }
    let (lo, hi) = (me.lateral_m.min(lane), me.lateral_m.max(lane));
    !others.iter().any(|o| {
        let reach = half_l() + o.half_length;
        let in_lane = in_line(o, lane, LINE_CLEARANCE_M)
            && within(o, me, -(reach + 1.0), ahead_m + reach, CHANGE_S);
        // Beside: anywhere between here and the new line.
        let clear = half_w() + o.half_width + LINE_CLEARANCE_M;
        let beside = o.lateral_m > lo - clear
            && o.lateral_m < hi + clear
            && within(o, me, -(reach + 0.5), reach + 0.5, 1.0);
        in_lane || beside
    })
}

/// One tick of autodrive avoidance for `me` among `others`.
pub fn plan(me: &Me, others: &[Other], overtake_left: bool, state: &mut AvoidState) -> Plan {
    let v = me.speed;
    let ahead_m = (LOOKAHEAD_S * v).max(MIN_LOOKAHEAD_M);
    let line = state.pass_m.unwrap_or(me.lane_m);
    // The nearest rider ahead in the line that needs a reaction.
    let blocker = others
        .iter()
        .filter(|o| {
            let reach = half_l() + o.half_length;
            o.ahead_m > 0.0
                && o.ahead_m < ahead_m + reach
                && ((o.lateral_m - line).abs() < half_w() + o.half_width + LINE_CLEARANCE_M
                    || (o.lateral_m - me.lateral_m).abs()
                        < half_w() + o.half_width + LINE_CLEARANCE_M)
        })
        .filter(|o| {
            let gap = o.ahead_m - (half_l() + o.half_length);
            let ttc = if v > o.speed {
                gap / (v - o.speed)
            } else {
                f64::INFINITY
            };
            ttc < TTC_S || gap < HOLD_GAP_M + 1.0
        })
        .min_by(|a, b| a.ahead_m.total_cmp(&b.ahead_m));
    let mut holding = false;
    match blocker {
        Some(b) => {
            let preferred = if overtake_left { -1.0 } else { 1.0 };
            let mut sides = vec![preferred];
            if me.road_width_m < NARROW_ROAD_M {
                sides.push(-preferred);
            }
            let shift = half_w() + b.half_width + PASS_CLEARANCE_M;
            let chosen = sides
                .into_iter()
                .map(|side| b.lateral_m + side * shift)
                .find(|lane| free(*lane, me, others, ahead_m));
            match chosen {
                Some(lane) => state.pass_m = Some(lane),
                None => holding = true,
            }
        }
        None => {
            // Past: back to the own line when it is free.
            if state.pass_m.is_some() && free(me.lane_m, me, others, ahead_m) {
                state.pass_m = None;
            }
        }
    }
    let target_m = state
        .pass_m
        .unwrap_or(me.lane_m)
        .clamp(-me.limit_m, me.limit_m);
    // The virtual brake: never closer than the wheel gap to a rider ahead
    // in the way (where the rider is or steers to).
    let speed_cap = others
        .iter()
        .filter(|o| {
            let clear = half_w() + o.half_width + 0.15;
            o.ahead_m > 0.0
                && o.ahead_m < ahead_m
                && ((o.lateral_m - me.lateral_m).abs() < clear
                    || (holding && (o.lateral_m - target_m).abs() < clear))
        })
        .map(|o| {
            let gap = o.ahead_m - (half_l() + o.half_length);
            (o.speed + HOLD_GAIN * (gap - HOLD_GAP_M)).max(0.0)
        })
        .reduce(f64::min);
    Plan {
        target_m,
        speed_cap,
        holding,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn me(lateral: f64, speed: f64) -> Me {
        Me {
            lateral_m: lateral,
            speed,
            lane_m: lateral,
            limit_m: 2.4,
            road_width_m: 6.0,
        }
    }

    #[test]
    fn a_free_road_needs_nothing() {
        let mut st = AvoidState::default();
        let p = plan(&me(0.0, 10.0), &[], true, &mut st);
        assert_eq!(p.target_m, 0.0);
        assert_eq!(p.speed_cap, None);
        assert!(!p.holding);
    }

    #[test]
    fn passes_on_the_regions_side() {
        // A slower rider 12 m ahead in the line: 3 m/s faster = 3.4 s …
        let slow = Other::rider(12.0, 0.0, 0.0, 7.0);
        let mut st = AvoidState::default();
        let p = plan(&me(0.0, 10.0), &[slow], true, &mut st);
        // … not yet below 3 s; at 10 m it is.
        assert_eq!(p.target_m, 0.0, "{p:?}");
        let slow = Other::rider(10.0, 0.0, 0.0, 7.0);
        let p = plan(&me(0.0, 10.0), &[slow], true, &mut st);
        assert!(
            (p.target_m + 0.9).abs() < 1e-9,
            "left in right-hand traffic: {p:?}"
        );
        // In a left-hand traffic region on the right.
        let mut st = AvoidState::default();
        let p = plan(&me(0.0, 10.0), &[slow], false, &mut st);
        assert!((p.target_m - 0.9).abs() < 1e-9, "{p:?}");
        // Past the rider, back to the own line.
        let behind = Other::rider(-6.0, 0.0, 0.0, 7.0);
        let p = plan(
            &Me {
                lateral_m: 0.9,
                ..me(0.0, 10.0)
            },
            &[behind],
            false,
            &mut st,
        );
        assert_eq!(p.target_m, 0.0, "{p:?}");
        assert_eq!(st.pass_m, None);
    }

    #[test]
    fn holds_back_when_both_sides_are_blocked() {
        // Slow rider ahead; the passing line on the left is taken by a
        // rider beside, the road is wide (no passing on the right).
        let ahead = Other::rider(6.0, 0.0, 0.0, 7.0);
        let beside = Other::rider(0.0, -0.9, -0.9, 10.0);
        let mut st = AvoidState::default();
        let p = plan(&me(0.0, 10.0), &[ahead, beside], true, &mut st);
        assert!(p.holding, "{p:?}");
        assert_eq!(p.target_m, 0.0);
        // The cap: the speed of the one in front plus the gap beyond 0.5 m.
        let gap = 6.0 - 1.75;
        let cap = 7.0 + HOLD_GAIN * (gap - HOLD_GAP_M);
        assert!((p.speed_cap.unwrap() - cap).abs() < 1e-9, "{p:?}");
        // At the wheel gap: exactly the speed of the one in front.
        let close = Other::rider(1.75 + HOLD_GAP_M, 0.0, 0.0, 7.0);
        let p = plan(&me(0.0, 9.0), &[close, beside], true, &mut st);
        assert!((p.speed_cap.unwrap() - 7.0).abs() < 1e-9, "{p:?}");
        // On a narrow road the other side is tried.
        let narrow = Me {
            road_width_m: 4.0,
            limit_m: 1.4,
            ..me(0.0, 10.0)
        };
        let mut st = AvoidState::default();
        let p = plan(&narrow, &[ahead, beside], true, &mut st);
        assert!((p.target_m - 0.9).abs() < 1e-9 && !p.holding, "{p:?}");
    }

    #[test]
    fn never_cuts_across_a_rider_coming_from_behind() {
        // The left line is free ahead, but a fast rider comes up there
        // from 4 m behind: no move, hold back instead.
        let ahead = Other::rider(6.0, 0.0, 0.0, 7.0);
        let fast = Other::rider(-4.0, -0.9, -0.9, 12.0);
        let mut st = AvoidState::default();
        let p = plan(&me(0.0, 10.0), &[ahead, fast], true, &mut st);
        assert!(p.holding && p.target_m == 0.0, "{p:?}");
    }

    #[test]
    fn steers_round_a_fallen_bike() {
        let fallen = Other::fallen(9.0, 0.2);
        let mut st = AvoidState::default();
        let p = plan(&me(0.0, 6.0), &[fallen], true, &mut st);
        // Left of it: its half width 0.5, ours 0.25, 0.4 clear.
        assert!((p.target_m - (0.2 - 1.15)).abs() < 1e-9, "{p:?}");
        // Blocked everywhere: stops at the wheel gap behind it.
        let narrow = Me {
            limit_m: 0.3,
            road_width_m: 5.0,
            ..me(0.0, 6.0)
        };
        let mut st = AvoidState::default();
        let close = Other::fallen(1.775 + HOLD_GAP_M, 0.0);
        let p = plan(&narrow, &[close], true, &mut st);
        assert!(p.holding && p.speed_cap == Some(0.0), "{p:?}");
    }
}
