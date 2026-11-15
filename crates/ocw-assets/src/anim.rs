//! Riding animation (spec §2.6: "animation is driven by cadence, speed and
//! riding state"): which state a rider is in, how fast the cranks and
//! wheels turn, and the smoothed pose weights. Pure and Bevy-free.
//!
//! Beyond the riding states, [`Animator`] drives the body motion the
//! client knows from the ride (v3.47): cornering beyond the lean (from the
//! turn's curvature and the speed), free ride's U-turn loop (a [`Cue`]
//! from the path), the finish and the winner's victory (from the event's
//! results) and a foot down at a stop. Everything is a function of the
//! rider's state and game time, blended smoothly – the same for the own
//! rider and for others from snapshots; looks only, never physics.

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
    /// Curvature of the line ridden (1/m, + = a right-hand bend): the
    /// rider's `turn_curvature`, as the lean.
    pub curvature: f64,
    /// What else the client knows: a U-turn, the finish.
    pub cue: Cue,
}

/// What the client knows about a rider beyond speed and power (spec §2.6):
/// it drives the U-turn, finish and victory animations.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Cue {
    /// Free ride's U-turn loop (spec §5.2) ahead or under the rider.
    pub uturn: Option<UTurnCue>,
    /// Crossed the finish line of a race or time trial.
    pub finish: Option<FinishCue>,
    /// A race or time trial under way: metres to the finish line ahead
    /// (negative past it) – the bike throw at the line.
    pub to_line_m: Option<f64>,
}

/// Where a rider is in a U-turn: `progress` −1 … 0 over the run-up
/// (`ocw_world::path::UTURN_RUNUP_M` before the loop), 0 … 1 through the
/// loop; `side` +1 when the loop turns right, −1 left.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct UTurnCue {
    pub progress: f64,
    pub side: f64,
}

/// The finish line crossed `since_s` ago (game time); `winner`: the race's
/// winner (or a time trial's new best), who raises the arms.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FinishCue {
    pub since_s: f64,
    pub winner: bool,
}

/// How long after the line the finish animations run (s); the client
/// sends no finish cue after it.
pub const FINISH_CUE_S: f64 = 8.0;

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

/// Grade from which a rider on a TT bike leaves the extensions for the
/// base bar when climbing seated.
pub const TT_BASE_BAR_GRADE: f64 = 0.06;

/// Pose weights a state aims for (`aero_bars`: a TT bike, whose "hoods"
/// are the extensions).
pub fn targets(state: Riding, speed_mps: f64, grade: f64, aero_bars: bool) -> PoseParams {
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
        // On the TT bike: on the extensions until it gets steep.
        Riding::Climbing => {
            if !aero_bars || grade >= TT_BASE_BAR_GRADE {
                p.tops = 1.0;
            }
        }
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
/// [`BLEND_S`]; the body motions of v3.47 their own, [`BLEND_CUE_S`]);
/// the crank angle is left alone, the foot side taken over.
pub fn blend(current: &mut PoseParams, target: &PoseParams, dt: f32) {
    let k = 1.0 - (-dt / BLEND_S).exp();
    let step = |a: &mut f32, b: f32| *a += (b - *a) * k;
    step(&mut current.stand, target.stand);
    step(&mut current.tops, target.tops);
    step(&mut current.drops, target.drops);
    step(&mut current.tuck, target.tuck);
    step(&mut current.rock, target.rock);
    let [
        corner,
        look,
        head_roll,
        foot,
        straddle,
        arms_up,
        sit_up,
        throw,
    ] = BLEND_CUE_S;
    let ease = |a: &mut f32, b: f32, tau: f32| *a += (b - *a) * (1.0 - (-dt / tau).exp());
    ease(&mut current.corner, target.corner, corner);
    ease(&mut current.look, target.look, look);
    ease(&mut current.head_roll, target.head_roll, head_roll);
    ease(&mut current.foot, target.foot, foot);
    ease(&mut current.straddle, target.straddle, straddle);
    ease(&mut current.arms_up, target.arms_up, arms_up);
    ease(&mut current.sit_up, target.sit_up, sit_up);
    ease(&mut current.throw, target.throw, throw);
    current.foot_side = target.foot_side;
}

/// Time constants (s) of corner, look, head roll, foot, straddle, arms up,
/// sit up and the throw.
pub const BLEND_CUE_S: [f32; 8] = [0.3, 0.25, 0.25, 0.22, 0.4, 0.22, 0.45, 0.08];

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

/// The foot a rider puts down at a stop (0 left, 1 right), from the id:
/// seven riders in ten the left (proposal v3.47), stable.
pub fn stop_foot(id: u32) -> usize {
    let h = id.wrapping_mul(0x9E37_79B1) >> 16;
    usize::from(h % 10 >= 7)
}

/// Largest lean angle (rad, ~35°).
pub const MAX_LEAN: f64 = 0.61;

/// Lean into a curve (rad, + = to the right): a bike in a steady turn
/// leans by atan(v² κ / g).
pub fn lean_angle(curvature: f64, speed_mps: f64) -> f64 {
    (speed_mps * speed_mps * curvature / 9.81)
        .atan()
        .clamp(-MAX_LEAN, MAX_LEAN)
}

/// Sideways acceleration of a full cornering pose (m/s²).
pub const CORNER_FULL_MPS2: f64 = 3.0;
/// From this cornering share a freewheeling rider holds the outside
/// pedal down.
pub const CORNER_PEDAL_DOWN: f64 = 0.3;

/// How much a rider corners (−1 … 1, + = right): the sideways
/// acceleration v² κ over [`CORNER_FULL_MPS2`].
pub fn corner(curvature: f64, speed_mps: f64) -> f64 {
    (speed_mps * speed_mps * curvature / CORNER_FULL_MPS2).clamp(-1.0, 1.0)
}

/// 0 below `a`, 1 above `b`, smooth between.
pub fn smoothstep(a: f64, b: f64, x: f64) -> f64 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Rises from `a` to `b`, holds 1 until `c`, falls to 0 at `d`.
pub fn window(x: f64, a: f64, b: f64, c: f64, d: f64) -> f64 {
    smoothstep(a, b, x) * (1.0 - smoothstep(c, d, x))
}

/// The head's turn over the shoulder before a U-turn (rad).
pub const LOOK_BACK: f64 = 1.25;
/// The inside foot held out low through the U-turn loop (share of the
/// way to the ground).
pub const UTURN_FOOT: f64 = 0.5;
/// Crank angle of the clipped-in pedal at a stop: forward and up, about
/// two o'clock, ready to push off.
pub const READY_CRANK: f32 = 1.05;
/// The bike throw: only riders arriving at the line this fast (m/s).
pub const THROW_MPS: (f64, f64) = (8.0, 10.0);

/// What [`aim`] asks for beyond the pose weights.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Aim {
    /// The weights to blend towards.
    pub pose: PoseParams,
    /// Freewheel (the cranks stop pedalling): arms up, sitting up, the
    /// throw.
    pub coast: bool,
    /// Where freewheeling cranks are held: the outside pedal down in a
    /// bend, the clipped pedal ready at a stop (`None`: level).
    pub crank: Option<f32>,
}

/// The pose a rider aims for in `state` with `input` (spec §2.6): the
/// riding state's [`targets`], then cornering, the U-turn, the finish and
/// victory, and the foot down at a stop. `stop_foot`: the foot this rider
/// puts down ([`stop_foot`]). Pure: the same input, the same aim.
pub fn aim(state: Riding, i: &AnimInput, aero_bars: bool, stop_foot: usize) -> Aim {
    use std::f32::consts::{FRAC_PI_2, PI};
    let mut p = targets(state, i.speed_mps, i.grade, aero_bars);
    let mut coast = false;
    let mut crank = None;
    // Cornering: the upper body to the inside, the inside knee out, the
    // head into the bend and held nearer level.
    let mut c = corner(i.curvature, i.speed_mps);
    let mut look = 0.3 * c;
    let head_roll = 0.5 * lean_angle(i.curvature, i.speed_mps);
    let (mut foot, mut foot_side, mut straddle) = (0.0, stop_foot, 0.0);
    // The U-turn: a look over the shoulder on the run-up, then into the
    // loop; through it the inside foot held out low.
    if let Some(u) = i.cue.uturn {
        let side = if u.side < 0.0 { -1.0 } else { 1.0 };
        if u.progress < 0.0 {
            let check = window(u.progress, -0.9, -0.75, -0.5, -0.35);
            look = look * (1.0 - check)
                + side * LOOK_BACK * check
                + side * 0.45 * smoothstep(-0.2, 0.0, u.progress);
        } else {
            let through = 1.0 - smoothstep(0.8, 1.0, u.progress);
            look = side * 0.6 * through;
            c = side * c.abs().max(0.6 * through);
            foot = UTURN_FOOT * window(u.progress, 0.0, 0.12, 0.75, 0.95);
            foot_side = usize::from(side > 0.0);
        }
    }
    // A stop: unclip, the foot down, off the saddle onto the top tube; the
    // other pedal ready to push off.
    if state == Riding::Stopped {
        foot = 1.0;
        foot_side = stop_foot;
        straddle = 1.0;
        look = 0.0;
        c = 0.0;
        crank = Some(if stop_foot == 0 {
            READY_CRANK
        } else {
            READY_CRANK + PI
        });
    } else if c.abs() >= CORNER_PEDAL_DOWN {
        // Freewheeling through a bend: the outside pedal down.
        crank = Some(if c > 0.0 { 0.0 } else { PI });
    }
    // The finish: everyone sits up and freewheels; the winner first raises
    // both arms.
    let (mut arms_up, mut sit_up) = (0.0, 0.0);
    if let Some(f) = i.cue.finish {
        let t = f.since_s;
        if f.winner {
            arms_up = window(t, 0.1, 0.6, 3.4, 4.2);
            sit_up = window(t, 3.4, 4.2, 6.5, FINISH_CUE_S);
            coast = t < 4.2;
        } else {
            sit_up = window(t, 0.2, 0.9, 4.5, 6.0);
            coast = (0.2..4.5).contains(&t);
        }
    }
    // The bike throw: the last metres to the line at speed.
    let throw = match i.cue.to_line_m {
        Some(d) if i.cue.finish.is_none() => {
            smoothstep(THROW_MPS.0, THROW_MPS.1, i.speed_mps) * window(-d, -2.5, -0.4, 0.3, 1.5)
        }
        _ => 0.0,
    };
    if throw > 0.0 {
        coast = true;
        crank = Some(FRAC_PI_2);
    }
    let up = arms_up.max(sit_up) as f32;
    p.stand *= 1.0 - up;
    p.rock *= 1.0 - up;
    p.drops *= 1.0 - up;
    p.tuck *= 1.0 - up;
    p.tops = p.tops.max(sit_up as f32);
    p.corner = c as f32;
    p.look = look as f32;
    p.head_roll = head_roll as f32;
    p.foot = foot as f32;
    p.foot_side = foot_side;
    p.straddle = straddle as f32;
    p.arms_up = arms_up as f32;
    p.sit_up = sit_up as f32;
    p.throw = throw as f32;
    Aim {
        pose: p,
        coast,
        crank,
    }
}

/// The crank angle after `dt` easing to `target` the shorter way round
/// (freewheeling cranks set by the foot: a pedal down in a bend, ready at a
/// stop).
pub fn hold_crank(theta: f32, target: f32, dt: f32) -> f32 {
    use std::f32::consts::PI;
    let diff = (target - theta + PI).rem_euclid(TAU) - PI;
    let k = 1.0 - (-dt / 0.3).exp();
    (theta + diff * k).rem_euclid(TAU)
}

/// A rider's animation (spec §2.6): the riding state, the pose weights
/// and the cranks, updated every frame from the ride. Deterministic.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Animator {
    pub selector: Selector,
    pub params: PoseParams,
    /// The foot this rider puts down at a stop ([`stop_foot`]).
    pub stop_foot: usize,
}

/// The foot changes sides only once it is back on its pedal (below this).
pub const FOOT_SWITCH: f32 = 0.03;

impl Animator {
    pub fn new(id: u32) -> Self {
        let stop_foot = stop_foot(id);
        Self {
            selector: Selector::default(),
            params: PoseParams {
                foot_side: stop_foot,
                ..Default::default()
            },
            stop_foot,
        }
    }

    /// One frame of `dt` seconds; the pose weights to draw.
    pub fn update(&mut self, input: &AnimInput, aero_bars: bool, dt: f32) -> PoseParams {
        let state = self.selector.update(input, f64::from(dt));
        let aim = aim(state, input, aero_bars, self.stop_foot);
        let mut target = aim.pose;
        // One foot at a time: the other side comes out only once this one
        // is back on its pedal.
        if target.foot_side != self.params.foot_side {
            if self.params.foot < FOOT_SWITCH && self.params.straddle < FOOT_SWITCH {
                self.params.foot_side = target.foot_side;
            } else {
                target.foot_side = self.params.foot_side;
                target.foot = 0.0;
                target.straddle = 0.0;
            }
        }
        blend(&mut self.params, &target, dt);
        let cadence = if aim.coast {
            0.0
        } else {
            cadence(state, input.cadence_rpm)
        };
        self.params.crank = match aim.crank {
            Some(h) if cadence <= 0.0 => hold_crank(self.params.crank, h, dt),
            _ => advance_crank(self.params.crank, cadence, dt),
        };
        self.params
    }

    pub fn state(&self) -> Riding {
        self.selector.state
    }
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
            ..Default::default()
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
        let target = targets(Standing, 4.0, 0.1, false);
        assert_eq!(target.stand, 1.0);
        assert!(target.rock > 0.0);
        assert_eq!(targets(Climbing, 4.0, 0.06, false).tops, 1.0);
        assert_eq!(
            targets(Climbing, 6.0, 0.04, true).tops,
            0.0,
            "TT: on the extensions"
        );
        assert_eq!(targets(Climbing, 4.0, 0.07, true).tops, 1.0);
        assert_eq!(targets(Sprinting, 14.0, 0.0, false).drops, 1.0);
        assert_eq!(targets(Coasting, 16.0, -0.06, false).tuck, 1.0);
        assert_eq!(targets(Coasting, 8.0, 0.0, false), PoseParams::default());
        let mut p = PoseParams::default();
        blend(&mut p, &target, 0.02);
        assert!(p.stand > 0.0 && p.stand < 0.1);
        for _ in 0..200 {
            blend(&mut p, &target, 0.02);
        }
        assert!((p.stand - 1.0).abs() < 1e-3);
    }

    /// Frames of `n` × 1/60 s with `input`; the largest change of any
    /// body weight between two frames.
    fn run(a: &mut Animator, input: &AnimInput, n: usize) -> f32 {
        let mut worst = 0.0f32;
        for _ in 0..n {
            let before = a.params;
            let after = a.update(input, false, 1.0 / 60.0);
            for (x, y) in [
                (before.foot, after.foot),
                (before.straddle, after.straddle),
                (before.look, after.look),
                (before.corner, after.corner),
                (before.arms_up, after.arms_up),
                (before.sit_up, after.sit_up),
            ] {
                worst = worst.max((x - y).abs());
            }
        }
        worst
    }

    /// Foot down at a stop (spec §2.6, v3.47): the foot comes off and down
    /// when the speed reaches 0, the rider straddles the top tube with the
    /// other pedal ready; riding off clips back in. Smooth, both ways.
    #[test]
    fn a_foot_goes_down_at_a_stop_and_clips_back_in() {
        let mut a = Animator::new(7);
        let riding = input(8.0, None, 200.0, 0.0);
        run(&mut a, &riding, 60);
        assert_eq!(a.state(), Seated);
        assert!(a.params.foot < 1e-3 && a.params.straddle < 1e-3);
        let stopped = input(0.0, None, 0.0, 0.0);
        let jump = run(&mut a, &stopped, 120);
        assert_eq!(a.state(), Stopped);
        assert!(
            a.params.foot > 0.99 && a.params.straddle > 0.95,
            "{:?}",
            a.params
        );
        assert_eq!(a.params.foot_side, stop_foot(7));
        assert!(jump < 0.1, "smooth: {jump}");
        // The clipped pedal ready: forward and up.
        let ready = if a.stop_foot == 0 {
            READY_CRANK
        } else {
            READY_CRANK + std::f32::consts::PI
        };
        assert!((a.params.crank - ready).abs() < 0.02, "{}", a.params.crank);
        // Riding off: back in.
        let jump = run(&mut a, &riding, 90);
        assert!(
            a.params.foot < 0.01 && a.params.straddle < 0.05,
            "{:?}",
            a.params
        );
        assert!(jump < 0.1, "smooth: {jump}");
        // Most riders put the left foot down; stable per id.
        let left = (0..1000).filter(|id| stop_foot(*id) == 0).count();
        assert!((600..800).contains(&left), "{left}");
        assert_eq!(stop_foot(42), stop_foot(42));
    }

    /// Cornering beyond the lean: into the bend with the head, the upper
    /// body and the inside knee; freewheeling the outside pedal down.
    #[test]
    fn cornering_turns_the_head_and_holds_the_outside_pedal_down() {
        let bend = |curvature: f64, power: f64| AnimInput {
            curvature,
            ..input(10.0, None, power, 0.0)
        };
        // A right-hand bend of 40 m at 10 m/s: 2.5 m/s² sideways.
        let a = aim(Seated, &bend(1.0 / 40.0, 200.0), false, 0);
        assert!((a.pose.corner - 2.5 / 3.0).abs() < 1e-3);
        assert!(a.pose.look > 0.2 && a.pose.head_roll > 0.1, "{:?}", a.pose);
        let left = aim(Seated, &bend(-1.0 / 40.0, 200.0), false, 0);
        assert!(left.pose.corner < -0.8 && left.pose.look < -0.2);
        // Coasting: the outside pedal down (right bend: the left pedal at
        // the bottom, crank angle 0; left bend: the right one, π).
        let mut an = Animator::new(1);
        run(&mut an, &bend(1.0 / 40.0, 0.0), 120);
        assert_eq!(an.state(), Coasting);
        assert!(
            an.params.crank.min(TAU - an.params.crank) < 0.02,
            "{}",
            an.params.crank
        );
        let mut an = Animator::new(1);
        run(&mut an, &bend(-1.0 / 40.0, 0.0), 120);
        assert!((an.params.crank - std::f32::consts::PI).abs() < 0.02);
        // Straight on: nothing of it.
        let s = aim(Seated, &input(10.0, None, 200.0, 0.0), false, 0);
        assert_eq!(
            (s.pose.corner, s.pose.look, s.pose.head_roll),
            (0.0, 0.0, 0.0)
        );
        assert_eq!(s.crank, None);
    }

    /// The U-turn (spec §2.6, §5.2): a look over the shoulder on the
    /// run-up, then through the loop leaning in with the inside foot held
    /// out low; out of it back to normal.
    #[test]
    fn the_uturn_looks_back_then_holds_the_inside_foot_out() {
        let at = |progress: f64, side: f64| AnimInput {
            cue: Cue {
                uturn: Some(UTurnCue { progress, side }),
                ..Default::default()
            },
            ..input(2.5, None, 120.0, 0.0)
        };
        let check = aim(Seated, &at(-0.6, -1.0), false, 1);
        assert!(
            (check.pose.look + LOOK_BACK as f32).abs() < 0.01,
            "over the left shoulder"
        );
        assert_eq!(check.pose.foot, 0.0);
        let mid = aim(Seated, &at(0.5, -1.0), false, 1);
        assert!((mid.pose.foot - UTURN_FOOT as f32).abs() < 1e-6);
        assert_eq!(mid.pose.foot_side, 0, "the inside (left) foot");
        assert!(mid.pose.corner <= -0.6 && mid.pose.look < -0.5);
        assert_eq!(aim(Seated, &at(0.5, 1.0), false, 0).pose.foot_side, 1);
        let out = aim(Seated, &at(1.0, -1.0), false, 1);
        assert!(out.pose.foot.abs() < 1e-6 && out.pose.look.abs() < 1e-6);
        // Smooth through the whole turn, frame by frame (2.5 m/s, a 20 m
        // run-up and a 25 m loop).
        let mut a = Animator::new(3);
        let mut worst = 0.0f32;
        for i in 0..1080 {
            let d = i as f64 / 60.0 * 2.5 - 20.0;
            let p = if d < 0.0 {
                d / 20.0
            } else {
                (d / 25.0).min(1.0)
            };
            worst = worst.max(run(&mut a, &at(p, -1.0), 1));
        }
        assert!(worst < 0.1, "{worst}");
        // A different foot down before: the inside foot waits until it is
        // back on its pedal.
        let mut a = Animator::new(3);
        a.stop_foot = 1;
        run(&mut a, &input(0.0, None, 0.0, 0.0), 120);
        assert_eq!(a.params.foot_side, 1);
        let mut sides = Vec::new();
        for _ in 0..120 {
            a.update(&at(0.4, -1.0), false, 1.0 / 60.0);
            sides.push((a.params.foot_side, a.params.foot));
        }
        let switched = sides.iter().position(|(s, _)| *s == 0).expect("switches");
        assert!(sides[switched - 1].1 < FOOT_SWITCH, "only once back in");
        assert!(sides.last().unwrap().1 > 0.4);
    }

    /// The finish: everyone sits up and freewheels; the winner raises both
    /// arms first; at speed the bike is thrown at the line.
    #[test]
    fn finish_victory_and_the_bike_throw() {
        let finished = |since_s: f64, winner: bool| AnimInput {
            cue: Cue {
                finish: Some(FinishCue { since_s, winner }),
                ..Default::default()
            },
            ..input(12.0, None, 300.0, 0.0)
        };
        let w = aim(Seated, &finished(2.0, true), false, 0);
        assert_eq!(w.pose.arms_up, 1.0);
        assert!(w.coast);
        let w = aim(Seated, &finished(6.0, true), false, 0);
        assert!(w.pose.arms_up == 0.0 && w.pose.sit_up == 1.0);
        let r = aim(Seated, &finished(2.0, false), false, 0);
        assert_eq!(
            (r.pose.arms_up, r.pose.sit_up, r.pose.tops),
            (0.0, 1.0, 1.0)
        );
        assert!(r.coast);
        let late = aim(Seated, &finished(FINISH_CUE_S, true), false, 0);
        assert_eq!((late.pose.arms_up, late.pose.sit_up), (0.0, 0.0));
        // No standing while the arms are up (a sprinter at the line).
        let sprint = aim(Sprinting, &finished(2.0, true), false, 0);
        assert_eq!((sprint.pose.stand, sprint.pose.drops), (0.0, 0.0));
        // The throw: at the line at speed, not slowly, not after it.
        let line = |to_line: f64, speed: f64| AnimInput {
            cue: Cue {
                to_line_m: Some(to_line),
                ..Default::default()
            },
            ..input(speed, None, 600.0, 0.0)
        };
        assert_eq!(aim(Sprinting, &line(0.0, 13.0), false, 0).pose.throw, 1.0);
        assert_eq!(aim(Sprinting, &line(0.0, 5.0), false, 0).pose.throw, 0.0);
        assert_eq!(aim(Sprinting, &line(10.0, 13.0), false, 0).pose.throw, 0.0);
        assert_eq!(aim(Sprinting, &line(-3.0, 13.0), false, 0).pose.throw, 0.0);
        // Arms up blends in smoothly.
        let mut a = Animator::new(5);
        let mut worst = 0.0f32;
        for i in 0..480 {
            worst = worst.max(run(&mut a, &finished(i as f64 / 60.0, true), 1));
        }
        assert!(worst < 0.1, "{worst}");
    }

    /// The same ride animates the same way (replays, captures, others).
    #[test]
    fn the_animation_is_deterministic() {
        let ride: Vec<AnimInput> = (0..600)
            .map(|i| {
                let t = i as f64 / 60.0;
                AnimInput {
                    curvature: (t * 0.7).sin() / 30.0,
                    cue: Cue {
                        finish: (t > 6.0).then_some(FinishCue {
                            since_s: t - 6.0,
                            winner: true,
                        }),
                        ..Default::default()
                    },
                    ..input((10.0 - t).max(0.0), None, 200.0, 0.0)
                }
            })
            .collect();
        let mut a = Animator::new(9);
        let mut b = Animator::new(9);
        for i in &ride {
            assert_eq!(
                a.update(i, false, 1.0 / 60.0),
                b.update(i, false, 1.0 / 60.0)
            );
        }
    }
}
