//! Riding animation (spec §2.6: "animation is driven by cadence, speed and
//! riding state"): which state a rider is in, how fast the cranks and
//! wheels turn, and the smoothed pose weights. Pure and Bevy-free.

use std::f32::consts::TAU;

use crate::pose::PoseParams;

/// What the rider is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Riding {
    /// Not moving: cranks level.
    #[default]
    Stopped,
    /// Rolling without pedalling: cranks level, still.
    Coasting,
    /// Normal pedalling, hands on the hoods.
    Seated,
    /// Seated on a climb, hands on the tops.
    Climbing,
    /// Out of the saddle on a steep climb, the bike rocking.
    Standing,
    /// Out of the saddle, hands on the drops, rocking hard.
    Sprinting,
}

/// What the state is chosen from.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AnimInput {
    pub speed_mps: f64,
    /// Measured cadence (only the local rider has one; others: `None`).
    pub cadence_rpm: Option<f64>,
    pub power_w: f64,
    /// Gradient at the rider (rise over run).
    pub grade: f64,
    /// Per-rider shift of the climbing thresholds (rise over run, ±0.015):
    /// a group does not stand up all at once.
    pub bias: f64,
}

/// Thresholds (with hysteresis: enter / leave).
pub const STOP_MPS: (f64, f64) = (0.3, 0.6);
pub const PEDAL_RPM: (f64, f64) = (15.0, 25.0);
pub const PEDAL_W: (f64, f64) = (10.0, 20.0);
pub const CLIMB_GRADE: (f64, f64) = (0.04, 0.03);
pub const STAND_GRADE: (f64, f64) = (0.085, 0.065);
/// Standing needs some effort (not crawling up at 60 W).
pub const STAND_W: f64 = 120.0;
pub const SPRINT_W: (f64, f64) = (650.0, 480.0);
pub const SPRINT_MPS: f64 = 7.0;
/// Shortest time between two pedalling states (s): no flicker at a
/// threshold. Stopping and coasting change at once.
pub const MIN_DWELL_S: f64 = 0.8;

/// The state for `input`, given the one before (hysteresis, no timing).
pub fn select(prev: Riding, i: &AnimInput) -> Riding {
    use Riding::*;
    let moving = if prev == Stopped {
        i.speed_mps > STOP_MPS.1
    } else {
        i.speed_mps >= STOP_MPS.0
    };
    if !moving {
        return Stopped;
    }
    let was_pedalling = matches!(prev, Seated | Climbing | Standing | Sprinting);
    let pedalling = match i.cadence_rpm {
        Some(c) => {
            c >= if was_pedalling {
                PEDAL_RPM.0
            } else {
                PEDAL_RPM.1
            }
        }
        None => i.power_w >= if was_pedalling { PEDAL_W.0 } else { PEDAL_W.1 },
    };
    if !pedalling {
        return Coasting;
    }
    let sprint = if prev == Sprinting {
        SPRINT_W.1
    } else {
        SPRINT_W.0
    };
    if i.power_w >= sprint && i.speed_mps >= SPRINT_MPS {
        return Sprinting;
    }
    let g = i.grade - i.bias;
    let stand = if prev == Standing {
        STAND_GRADE.1
    } else {
        STAND_GRADE.0
    };
    if g >= stand && i.power_w >= STAND_W {
        return Standing;
    }
    let climb = if matches!(prev, Climbing | Standing) {
        CLIMB_GRADE.1
    } else {
        CLIMB_GRADE.0
    };
    if g >= climb {
        return Climbing;
    }
    Seated
}

/// [`select`] with a minimum time between pedalling states.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Selector {
    pub state: Riding,
    /// Seconds in `state`.
    pub held_s: f64,
}

impl Selector {
    pub fn update(&mut self, input: &AnimInput, dt: f64) -> Riding {
        self.held_s += dt;
        let next = select(self.state, input);
        let pedalling = |r: Riding| !matches!(r, Riding::Stopped | Riding::Coasting);
        let switch = next != self.state
            && (!pedalling(next) || !pedalling(self.state) || self.held_s >= MIN_DWELL_S);
        if switch {
            self.state = next;
            self.held_s = 0.0;
        }
        self.state
    }
}

/// Cadence shown (rpm): the measured one while pedalling, else a typical
/// one for the state (others' cadence is not sent over the network).
pub fn cadence(state: Riding, measured: Option<f64>) -> f64 {
    let typical = match state {
        Riding::Stopped | Riding::Coasting => 0.0,
        Riding::Seated => 88.0,
        Riding::Climbing => 80.0,
        Riding::Standing => 66.0,
        Riding::Sprinting => 108.0,
    };
    match measured {
        Some(c) if typical > 0.0 => c.clamp(0.0, 200.0),
        _ => typical,
    }
}

/// Pose weights a state aims for.
pub fn targets(state: Riding, speed_mps: f64, grade: f64) -> PoseParams {
    let mut p = PoseParams::default();
    match state {
        Riding::Stopped | Riding::Seated => {}
        Riding::Coasting => {
            // Fast descents: down on the drops, tucked.
            if speed_mps > 12.0 && grade < -0.02 {
                p.drops = 1.0;
                p.tuck = 1.0;
            }
        }
        Riding::Climbing => p.tops = 1.0,
        Riding::Standing => {
            p.stand = 1.0;
            p.rock = 0.1;
        }
        Riding::Sprinting => {
            p.stand = 1.0;
            p.drops = 1.0;
            p.tuck = 0.5;
            p.rock = 0.16;
        }
    }
    p
}

/// Moves the weights of `current` towards `target` (time constant
/// [`BLEND_S`]); the crank angle is left alone.
pub fn blend(current: &mut PoseParams, target: &PoseParams, dt: f32) {
    let k = 1.0 - (-dt / BLEND_S).exp();
    let step = |a: &mut f32, b: f32| *a += (b - *a) * k;
    step(&mut current.stand, target.stand);
    step(&mut current.tops, target.tops);
    step(&mut current.drops, target.drops);
    step(&mut current.tuck, target.tuck);
    step(&mut current.rock, target.rock);
}

/// Time constant of pose changes (s).
pub const BLEND_S: f32 = 0.35;

/// The crank angle after `dt`: turning at `cadence_rpm`; with no cadence
/// it eases to the nearest level position (pedals at 3 and 9 o'clock).
pub fn advance_crank(theta: f32, cadence_rpm: f64, dt: f32) -> f32 {
    use std::f32::consts::{FRAC_PI_2, PI};
    if cadence_rpm > 0.0 {
        return (theta + cadence_rpm as f32 / 60.0 * TAU * dt).rem_euclid(TAU);
    }
    let theta = theta.rem_euclid(TAU);
    // Level: π/2 or 3π/2; turn on forwards to the next one, never back.
    let level = if theta <= FRAC_PI_2 {
        FRAC_PI_2
    } else if theta <= FRAC_PI_2 + PI {
        FRAC_PI_2 + PI
    } else {
        FRAC_PI_2 + TAU
    };
    let k = 1.0 - (-dt / 0.25).exp();
    (theta + (level - theta) * k).rem_euclid(TAU)
}

/// The wheel angle after `dt` at `speed_mps` (rolling without slip).
pub fn advance_wheel(angle: f32, speed_mps: f64, wheel_radius_m: f32, dt: f32) -> f32 {
    (angle + speed_mps as f32 / wheel_radius_m.max(0.1) * dt).rem_euclid(TAU)
}

/// A rider's threshold bias from their id: −0.015…+0.015, stable.
pub fn bias(id: u32) -> f64 {
    let h = id.wrapping_mul(2_654_435_761) >> 16;
    (f64::from(h % 1000) / 999.0 - 0.5) * 0.03
}

#[cfg(test)]
mod tests {
    use super::*;
    use Riding::*;

    fn input(speed: f64, cadence: Option<f64>, power: f64, grade: f64) -> AnimInput {
        AnimInput {
            speed_mps: speed,
            cadence_rpm: cadence,
            power_w: power,
            grade,
            bias: 0.0,
        }
    }

    #[test]
    fn states_follow_cadence_speed_power_and_grade() {
        assert_eq!(select(Seated, &input(0.0, Some(90.0), 200.0, 0.0)), Stopped);
        // Measured cadence decides pedalling, whatever the power says.
        assert_eq!(
            select(Seated, &input(10.0, Some(0.0), 200.0, 0.0)),
            Coasting
        );
        assert_eq!(select(Seated, &input(10.0, Some(90.0), 200.0, 0.0)), Seated);
        // Others (no cadence): power.
        assert_eq!(select(Seated, &input(10.0, None, 0.0, -0.05)), Coasting);
        assert_eq!(select(Seated, &input(10.0, None, 220.0, 0.0)), Seated);
        assert_eq!(select(Seated, &input(5.0, None, 250.0, 0.05)), Climbing);
        assert_eq!(select(Seated, &input(4.0, None, 300.0, 0.10)), Standing);
        // Standing needs effort.
        assert_eq!(select(Seated, &input(2.0, None, 80.0, 0.10)), Climbing);
        assert_eq!(select(Seated, &input(12.0, None, 900.0, 0.0)), Sprinting);
        // A slow start at high power is no sprint.
        assert_eq!(select(Seated, &input(3.0, None, 900.0, 0.0)), Seated);
    }

    #[test]
    fn thresholds_have_hysteresis() {
        // 7 % lies between the standing thresholds: stays as it was.
        let i = input(4.0, None, 300.0, 0.07);
        assert_eq!(select(Standing, &i), Standing);
        assert_eq!(select(Climbing, &i), Climbing);
        // Sprint: on above 650 W, off below 480 W.
        let i = input(12.0, None, 550.0, 0.0);
        assert_eq!(select(Sprinting, &i), Sprinting);
        assert_eq!(select(Seated, &i), Seated);
        // Pedalling: 20 rpm keeps pedalling, does not start it.
        let i = input(8.0, Some(20.0), 100.0, 0.0);
        assert_eq!(select(Seated, &i), Seated);
        assert_eq!(select(Coasting, &i), Coasting);
        // Stop / start.
        assert_eq!(select(Stopped, &input(0.5, None, 200.0, 0.0)), Stopped);
        assert_eq!(select(Seated, &input(0.5, None, 200.0, 0.0)), Seated);
        // The bias moves the climbing thresholds.
        let mut i = input(4.0, None, 300.0, 0.09);
        i.bias = 0.015;
        assert_eq!(select(Seated, &i), Climbing);
        assert!((-0.015..=0.015).contains(&bias(7)));
        assert_ne!(bias(1), bias(2));
    }

    #[test]
    fn the_selector_waits_before_switching_pedalling_states() {
        let mut s = Selector {
            state: Seated,
            held_s: 10.0,
        };
        let climb = input(4.0, None, 300.0, 0.10);
        assert_eq!(s.update(&climb, 0.02), Standing);
        // Back to flat at once: held only 0.02 s, stays standing.
        let flat = input(8.0, None, 200.0, 0.0);
        assert_eq!(s.update(&flat, 0.02), Standing);
        for _ in 0..50 {
            s.update(&flat, 0.02);
        }
        assert_eq!(s.state, Seated);
        // Coasting is immediate.
        assert_eq!(s.update(&input(8.0, None, 0.0, 0.0), 0.02), Coasting);
        assert_eq!(s.update(&flat, 0.02), Seated);
    }

    #[test]
    fn cadence_is_measured_or_typical() {
        assert_eq!(cadence(Seated, Some(95.0)), 95.0);
        assert_eq!(cadence(Coasting, Some(95.0)), 0.0);
        assert!(cadence(Standing, None) < cadence(Seated, None));
        assert!(cadence(Sprinting, None) > cadence(Seated, None));
        assert_eq!(cadence(Stopped, None), 0.0);
    }

    #[test]
    fn cranks_turn_with_cadence_and_stop_level() {
        // 90 rpm: 1.5 turns per second.
        let t = advance_crank(0.0, 90.0, 0.1);
        assert!((t - 0.15 * TAU).abs() < 1e-5);
        // Freewheeling: eases forward to level and stays there.
        let mut t = 0.3;
        for _ in 0..200 {
            let next = advance_crank(t, 0.0, 0.02);
            assert!(next >= t - 1e-6 || next < 0.1, "never backwards");
            t = next;
        }
        assert!((t - std::f32::consts::FRAC_PI_2).abs() < 1e-3, "{t}");
        let mut t = 5.0;
        for _ in 0..200 {
            t = advance_crank(t, 0.0, 0.02);
        }
        assert!(
            (t - std::f32::consts::FRAC_PI_2).abs() < 1e-3,
            "past 3π/2: on to π/2, {t}"
        );
        // Wheels: 10 m/s on a 0.34 m wheel ≈ 29.4 rad/s.
        let a = advance_wheel(0.0, 10.0, 0.34, 0.01);
        assert!((a - 0.294).abs() < 1e-3);
    }

    #[test]
    fn pose_weights_blend_smoothly() {
        let target = targets(Standing, 4.0, 0.1);
        assert_eq!(target.stand, 1.0);
        assert!(target.rock > 0.0);
        assert_eq!(targets(Climbing, 4.0, 0.06).tops, 1.0);
        assert_eq!(targets(Sprinting, 14.0, 0.0).drops, 1.0);
        assert_eq!(targets(Coasting, 16.0, -0.06).tuck, 1.0);
        assert_eq!(targets(Coasting, 8.0, 0.0), PoseParams::default());
        let mut p = PoseParams::default();
        blend(&mut p, &target, 0.02);
        assert!(p.stand > 0.0 && p.stand < 0.1);
        for _ in 0..200 {
            blend(&mut p, &target, 0.02);
        }
        assert!((p.stand - 1.0).abs() < 1e-3);
    }
}
