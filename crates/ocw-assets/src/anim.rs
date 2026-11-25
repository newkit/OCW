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
    /// The rider's status signs (spec §2.6): the phone, the VR head tilt.
    pub signs: Signs,
    /// The game time (s): the server's tick time every client shares
    /// (the grip schedule, spec 14 Phase 4 Q31).
    pub time_s: f64,
    /// The power this rider sustains (W; 0 = not known yet): a slow
    /// average of its pedalling power, kept by the [`Animator`] – a surge
    /// well above it is a sprint ([`sprint_threshold`]).
    pub sustained_w: f64,
}

/// The rider status signs as one viewer sees them (spec §2.6, §7.1.7
/// Q11): what a rider is doing outside the ride. Looks only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Signs {
    /// The microphone is muted: an X over the mouth (everybody sees it).
    pub voice_muted: bool,
    /// In the menu (§2.6 Q2): the phone comes out of the back pocket.
    pub in_menu: bool,
    /// Riding in VR: the VR headset (and no phone in the menu).
    pub in_vr: bool,
    /// This rider cannot hear the viewer: headphones (per viewer, Q11).
    pub cannot_hear_you: bool,
}

/// The phone's grab: pocket to hand to in front of the stem (s).
pub const PHONE_GRAB_S: f32 = 1.2;
/// The phone's store: back into the pocket, the hand back to the bar (s).
pub const PHONE_STORE_S: f32 = 1.0;
/// Menu visits shorter than this show nothing (no flicker from a quick
/// Esc, s).
pub const PHONE_DELAY_S: f32 = 0.4;
/// The worn signs scale in and out in this time (s).
pub const SIGN_SCALE_S: f32 = 0.25;
/// The phone's timeline (0 stored … 1 held): the hand is at the pocket
/// between these, and takes the phone out at [`PHONE_PICK`].
pub const PHONE_AT_POCKET: (f32, f32) = (0.42, 0.56);
pub const PHONE_PICK: f32 = 0.49;
/// The head tilts down to the phone (rad) and, for a VR rider in the menu,
/// a little (the menu is a panel in the headset, *proposal*).
pub const PHONE_HEAD_DOWN: f32 = 0.35;
pub const VR_HEAD_DOWN: f32 = 0.15;
/// The torso's twist to the right while the hand reaches the pocket (rad,
/// 10°); the head follows half.
pub const PHONE_TWIST: f32 = 0.175;
/// The VR rider's head tilt eases with this time constant (s).
pub const VR_TILT_S: f32 = 0.3;

/// The status signs' animation (spec §2.6): the phone's grab / hold /
/// store timeline and the worn signs' scale. Deterministic, the same on
/// every client (driven by the flags of the snapshots).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct StatusAnim {
    /// The phone's timeline: 0 in the pocket … 1 held in front of the stem.
    pub phone: f32,
    /// How long the menu has been open (s; 0 while closed).
    pub menu_s: f32,
    /// Scale of the headphones, the X mouth and the VR headset (0 … 1).
    pub headphones: f32,
    pub mouth: f32,
    pub headset: f32,
    /// The VR rider's head tilt in the menu (0 … 1).
    pub vr_look: f32,
}

impl StatusAnim {
    /// One frame of `dt` seconds with the signs `s`.
    pub fn update(&mut self, s: Signs, dt: f32) {
        self.menu_s = if s.in_menu { self.menu_s + dt } else { 0.0 };
        // Out once the menu stayed open; turns back at once while the
        // phone is still on its way (opened again during the store).
        let wants = s.in_menu && !s.in_vr && (self.menu_s >= PHONE_DELAY_S || self.phone > 0.0);
        self.phone = if wants {
            self.phone + dt / PHONE_GRAB_S
        } else {
            self.phone - dt / PHONE_STORE_S
        }
        .clamp(0.0, 1.0);
        let toward = |v: &mut f32, on: bool, t: f32| {
            let step = dt / t;
            *v = if on {
                (*v + step).min(1.0)
            } else {
                (*v - step).max(0.0)
            };
        };
        toward(&mut self.headphones, s.cannot_hear_you, SIGN_SCALE_S);
        toward(&mut self.mouth, s.voice_muted, SIGN_SCALE_S);
        toward(&mut self.headset, s.in_vr, SIGN_SCALE_S);
        let k = 1.0 - (-dt / VR_TILT_S).exp();
        let aim = if s.in_vr && s.in_menu { 1.0 } else { 0.0 };
        self.vr_look += (aim - self.vr_look) * k;
    }

    /// The phone is in the hand (else in the pocket, hidden).
    pub fn phone_in_hand(&self) -> bool {
        self.phone >= PHONE_PICK
    }

    /// Weight of the one-handed pose (the right hand off the bar).
    pub fn one_hand(&self) -> f32 {
        smoothstep(0.0, 0.2, f64::from(self.phone)) as f32
    }

    /// Writes the signs' pose into `p` (a copy of the riding pose): the
    /// phone's timeline, the one-handed seated pose overriding standing,
    /// sprinting and the hands on the tops or drops, the VR head tilt.
    pub fn apply(&self, p: &mut PoseParams) {
        let w = self.one_hand();
        p.phone = self.phone;
        p.one_hand = w;
        for v in [
            &mut p.stand,
            &mut p.tops,
            &mut p.drops,
            &mut p.tuck,
            &mut p.rock,
            &mut p.arms_up,
            &mut p.throw,
            &mut p.sprint,
        ] {
            *v *= 1.0 - w;
        }
        p.head_down = VR_HEAD_DOWN * self.vr_look;
    }
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
/// A sprint is also a surge well above the power the rider sustains (spec
/// 2.6, v5.84): this many times it (enter / leave) …
pub const SURGE: (f64, f64) = (1.5, 1.25);
/// … and at least this much (W; enter, leave): a weak rider's sprint, not
/// a strong rider's attack on a climb.
pub const SURGE_MIN_W: (f64, f64) = (350.0, 280.0);
/// With a measured cadence a sprint spins this fast (rpm; enter, leave).
pub const SPRINT_RPM: (f64, f64) = (88.0, 72.0);
/// The sustained power's time constant (s): about the last few minutes
/// of pedalling (coasting leaves it as it was).
pub const SUSTAINED_S: f64 = 90.0;

/// The power from which a rider sprints (W): `SPRINT_W` or, lower, a surge
/// above the power it sustains (`sustained_w`, 0 = unknown); `staying`:
/// already sprinting (the lower thresholds).
pub fn sprint_threshold(sustained_w: f64, staying: bool) -> f64 {
    let i = usize::from(staying);
    let abs = [SPRINT_W.0, SPRINT_W.1][i];
    if sustained_w <= 0.0 {
        return abs;
    }
    let surge = [SURGE.0, SURGE.1][i] * sustained_w;
    abs.min(surge.max([SURGE_MIN_W.0, SURGE_MIN_W.1][i]))
}

/// Whether `i` is a sprint (`staying`: already sprinting): the speed, the
/// power above [`sprint_threshold`] and, measured, a high cadence.
pub fn is_sprint(i: &AnimInput, staying: bool) -> bool {
    let spin = match i.cadence_rpm {
        Some(c) => c >= if staying { SPRINT_RPM.1 } else { SPRINT_RPM.0 },
        None => true,
    };
    spin && i.speed_mps >= SPRINT_MPS && i.power_w >= sprint_threshold(i.sustained_w, staying)
}
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
    if is_sprint(i, prev == Sprinting) {
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
        // The sprint (v5.84): out of the saddle, the hands low in the
        // hooks, the hips over the cranks, the torso low, the bike rocked
        // a few degrees from side to side with the strokes. On a TT bike
        // its drops are the base bar's ends, on a flat bar the grips.
        Riding::Sprinting => {
            p.stand = 1.0;
            p.drops = 1.0;
            p.tuck = 0.6;
            p.rock = SPRINT_ROCK;
            p.sprint = 1.0;
        }
    }
    p
}

/// The bike's rocking in a sprint (rad amplitude, ±7°): away from the
/// pedal pushing down, the rider's body counter-swaying above it.
pub const SPRINT_ROCK: f32 = 0.12;
/// A rider in VR sees the own bike rock this share of it (the real bars
/// stay still under the real hands).
pub const VR_ROCK_SHARE: f32 = 0.35;

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
    step(&mut current.sprint, target.sprint);
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
    p.sprint *= 1.0 - up;
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

/// The grip schedule's mean time between two changes (s; spec 14 Phase 4
/// Q31): every change falls within ±[`GRIP_JITTER_S`] of a multiple of it,
/// so a grip is held 20 – 90 s.
pub const GRIP_PERIOD_S: f64 = 55.0;
pub const GRIP_JITTER_S: f64 = 17.0;
/// A grip change takes this long (s): the hand lets go and takes hold.
pub const GRIP_BLEND_S: f32 = 0.4;
/// Cruising (seated, coasting) a handshake spell lasts only this share
/// of its time, then the hands drape again; on a climb a draped spell
/// only this share, then the hands take the handshake grip (or leave the
/// tops for the hoods).
pub const CRUISE_HANDSHAKE_SHARE: f64 = 0.4;
pub const CLIMB_DRAPED_SHARE: f64 = 0.35;

/// A stable hash of a rider and a number (0 … 1).
fn unit_hash(id: u32, k: i64) -> f64 {
    let mut h = u64::from(id) ^ (k as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h ^= h >> 33;
    h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    h ^= h >> 33;
    h = h.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    h ^= h >> 33;
    (h >> 11) as f64 / (1u64 << 53) as f64
}

/// When the `k`-th grip change of rider `id` falls (game time, s).
pub fn grip_change_s(id: u32, k: i64) -> f64 {
    k as f64 * GRIP_PERIOD_S + (2.0 * unit_hash(id, k) - 1.0) * GRIP_JITTER_S
}

/// Which grip on the hoods rider `id` holds at game time `t_s` in `state`
/// (spec 14 Phase 4 Q31): `true` the handshake grip, `false` draped. The
/// grips take turns in spells of 20 – 90 s, each rider its own (seeded by
/// the id, read from the game time every client shares, so all of them
/// show the same); cruising most of the time draped, on a climb and out of
/// the saddle mostly the handshake. A pure function: no state, no frame
/// rate.
pub fn grip_schedule(id: u32, t_s: f64, state: Riding) -> bool {
    if state == Riding::Standing {
        return true;
    }
    let mut k = (t_s / GRIP_PERIOD_S).floor() as i64;
    // The spell holding t: between change k and k + 1.
    while grip_change_s(id, k) > t_s {
        k -= 1;
    }
    while grip_change_s(id, k + 1) <= t_s {
        k += 1;
    }
    let (from, to) = (grip_change_s(id, k), grip_change_s(id, k + 1));
    let into = (t_s - from) / (to - from);
    let first = unit_hash(id, -1) < 0.5;
    let handshake = (k.rem_euclid(2) == 0) == first;
    match state {
        Riding::Climbing => handshake || into >= CLIMB_DRAPED_SHARE,
        _ => handshake && into < CRUISE_HANDSHAKE_SHARE,
    }
}

/// A rider's animation (spec §2.6): the riding state, the pose weights
/// and the cranks, updated every frame from the ride. Deterministic.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Animator {
    pub selector: Selector,
    /// The riding pose's weights (without the status signs' overrides).
    pub params: PoseParams,
    /// The status signs (spec §2.6).
    pub status: StatusAnim,
    /// The foot this rider puts down at a stop ([`stop_foot`]).
    pub stop_foot: usize,
    /// The rider's id (the grip schedule's seed).
    pub id: u32,
    /// The power it sustains (W, 0 = no pedalling seen yet).
    pub sustained_w: f64,
    /// The grip change's progress (0 draped … 1 handshake, linear; drawn
    /// eased).
    pub grip: f32,
    /// A grip held whatever the schedule says (scenes: `dev.grip`).
    pub grip_override: Option<bool>,
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
            status: StatusAnim::default(),
            id,
            sustained_w: 0.0,
            grip: 0.0,
            grip_override: None,
        }
    }

    /// Follows the power the rider sustains: a slow average of its
    /// pedalling power (coasting and stops leave it).
    fn sustain(&mut self, power_w: f64, dt: f64) {
        if power_w < PEDAL_W.1 || !power_w.is_finite() {
            return;
        }
        self.sustained_w = if self.sustained_w <= 0.0 {
            power_w
        } else {
            self.sustained_w + (power_w - self.sustained_w) * (1.0 - (-dt / SUSTAINED_S).exp())
        };
    }

    /// The grip on the hoods drawn now (0 draped … 1 handshake): the
    /// change eased over [`GRIP_BLEND_S`].
    pub fn handshake(&self) -> f32 {
        smoothstep(0.0, 1.0, f64::from(self.grip)) as f32
    }

    /// One frame of `dt` seconds; the pose weights to draw.
    pub fn update(&mut self, input: &AnimInput, aero_bars: bool, dt: f32) -> PoseParams {
        // A sprint is a surge well above the power the rider sustains.
        let mut input = *input;
        if input.sustained_w <= 0.0 {
            input.sustained_w = self.sustained_w;
        }
        let input = &input;
        let state = self.selector.update(input, f64::from(dt));
        self.sustain(input.power_w, f64::from(dt));
        let aim = aim(state, input, aero_bars, self.stop_foot);
        let mut target = aim.pose;
        // The grip on the hoods (Q31): the schedule's, changed over
        // GRIP_BLEND_S; on a climb a handshake spell takes the hands from
        // the tops to the hoods.
        let handshake = self
            .grip_override
            .unwrap_or_else(|| grip_schedule(self.id, input.time_s, state));
        if state == Riding::Climbing && handshake && !aero_bars && self.grip_override.is_none() {
            target.tops = 0.0;
        }
        let step = dt / GRIP_BLEND_S;
        self.grip = if handshake {
            (self.grip + step).min(1.0)
        } else {
            (self.grip - step).max(0.0)
        };
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
        // The status signs (spec §2.6): the phone overrides the hands.
        self.status.update(input.signs, dt);
        let mut out = self.params;
        out.handshake = self.handshake();
        // In VR the own bike rocks less: the real bars stay still.
        if input.signs.in_vr {
            out.rock *= VR_ROCK_SHARE;
        }
        self.status.apply(&mut out);
        out
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

/// The status signs' timeline (spec §2.6 *Rider status signs*).
#[cfg(test)]
mod status_tests {
    use super::*;

    const DT: f32 = 1.0 / 60.0;

    fn run(a: &mut StatusAnim, s: Signs, seconds: f32) -> Vec<f32> {
        let n = (seconds / DT).round() as usize;
        (0..n)
            .map(|_| {
                a.update(s, DT);
                a.phone
            })
            .collect()
    }

    fn menu() -> Signs {
        Signs {
            in_menu: true,
            ..Default::default()
        }
    }

    #[test]
    fn the_phone_comes_out_smoothly_and_goes_back() {
        let mut a = StatusAnim::default();
        let out = run(&mut a, menu(), 2.0);
        // Nothing for the first 0.4 s, then the grab in 1.2 s.
        let delay = (PHONE_DELAY_S / DT) as usize;
        assert!(out[..delay - 1].iter().all(|p| *p == 0.0));
        assert_eq!(a.phone, 1.0);
        let grab_frames = out.iter().filter(|p| **p > 0.0 && **p < 1.0).count();
        assert!((grab_frames as f32 * DT - PHONE_GRAB_S).abs() < 0.05);
        // Frame by frame: never a jump.
        for w in out.windows(2) {
            assert!((w[1] - w[0]).abs() <= DT / PHONE_GRAB_S + 1e-6);
        }
        assert!(a.phone_in_hand());
        let back = run(&mut a, Signs::default(), 1.2);
        assert_eq!(a.phone, 0.0);
        let store = back.iter().filter(|p| **p > 0.0).count();
        assert!((store as f32 * DT - PHONE_STORE_S).abs() < 0.05);
        for w in back.windows(2) {
            assert!((w[1] - w[0]).abs() <= DT / PHONE_STORE_S + 1e-6);
        }
    }

    #[test]
    fn short_menu_visits_show_nothing() {
        let mut a = StatusAnim::default();
        for _ in 0..5 {
            run(&mut a, menu(), 0.35);
            assert_eq!(a.phone, 0.0);
            run(&mut a, Signs::default(), 0.2);
        }
        assert_eq!(a.phone, 0.0);
        assert_eq!(a.one_hand(), 0.0);
    }

    #[test]
    fn the_grab_and_the_store_reverse_mid_way() {
        let mut a = StatusAnim::default();
        run(&mut a, menu(), PHONE_DELAY_S + 0.5);
        let mid = a.phone;
        assert!(mid > 0.2 && mid < 0.6, "{mid}");
        // Closed during the grab: back from where the hand is.
        a.update(Signs::default(), DT);
        assert!(a.phone < mid && mid - a.phone < 0.02);
        run(&mut a, Signs::default(), 0.2);
        let lower = a.phone;
        assert!(lower > 0.0);
        // Opened again during the store: turns back at once (no new
        // delay, no jump).
        a.update(menu(), DT);
        assert!(a.phone > lower && a.phone - lower < 0.02);
    }

    #[test]
    fn a_vr_rider_tilts_the_head_instead_of_taking_out_the_phone() {
        let mut a = StatusAnim::default();
        let vr_menu = Signs {
            in_menu: true,
            in_vr: true,
            ..Default::default()
        };
        run(&mut a, vr_menu, 2.0);
        assert_eq!(a.phone, 0.0);
        assert!(a.vr_look > 0.95);
        assert_eq!(a.headset, 1.0);
        let mut p = PoseParams::default();
        a.apply(&mut p);
        assert!((p.head_down - VR_HEAD_DOWN).abs() < 0.01);
        assert_eq!(p.one_hand, 0.0);
        // The headset off again: it scales out in 0.25 s.
        let out: Vec<f32> = (0..20)
            .map(|_| {
                a.update(Signs::default(), DT);
                a.headset
            })
            .collect();
        assert_eq!(a.headset, 0.0);
        assert!(out[0] > 0.9 && out.iter().filter(|h| **h > 0.0).count() <= 15);
    }

    #[test]
    fn the_worn_signs_scale_in_and_out() {
        let mut a = StatusAnim::default();
        let s = Signs {
            voice_muted: true,
            cannot_hear_you: true,
            ..Default::default()
        };
        a.update(s, 0.1);
        assert!(a.mouth > 0.3 && a.mouth < 0.5 && a.headphones == a.mouth);
        a.update(s, 0.2);
        assert_eq!((a.mouth, a.headphones, a.headset), (1.0, 1.0, 0.0));
        a.update(Signs::default(), 0.3);
        assert_eq!((a.mouth, a.headphones), (0.0, 0.0));
    }

    #[test]
    fn the_phone_overrides_standing_and_sprinting_with_one_hand() {
        let mut an = Animator::new(3);
        let sprint = AnimInput {
            speed_mps: 12.0,
            power_w: 900.0,
            signs: menu(),
            ..Default::default()
        };
        let mut p = PoseParams::default();
        for _ in 0..240 {
            p = an.update(&sprint, false, DT);
        }
        // The riding state is still the sprint, the pose is seated.
        assert_eq!(an.state(), Riding::Sprinting);
        assert!(an.params.stand > 0.9 && an.params.drops > 0.9);
        assert_eq!(p.one_hand, 1.0);
        assert!(p.stand < 1e-6 && p.drops < 1e-6 && p.rock < 1e-6);
        assert_eq!(p.phone, 1.0);
        // The phone stored: both hands back, standing again.
        let mut sprint = sprint;
        sprint.signs = Signs::default();
        for _ in 0..120 {
            p = an.update(&sprint, false, DT);
        }
        assert_eq!(p.one_hand, 0.0);
        assert!(p.stand > 0.9);
    }

    #[test]
    fn the_status_animation_is_deterministic() {
        let mut a = StatusAnim::default();
        let mut b = StatusAnim::default();
        let seq = [menu(), menu(), Signs::default(), menu()];
        for (i, s) in seq.iter().cycle().take(400).enumerate() {
            let dt = if i % 3 == 0 { 0.02 } else { 0.013 };
            a.update(*s, dt);
            b.update(*s, dt);
        }
        assert_eq!(a, b);
    }
}

/// The hoods' grip schedule and the sprint (spec 14 Phase 4 Q31, v5.84).
#[cfg(test)]
mod grip_tests {
    use super::*;
    use Riding::*;

    /// The spells of one grip over `hours` of game time in `state`, as
    /// (handshake, length s), sampled every 0.25 s.
    fn spells(id: u32, state: Riding, hours: f64) -> Vec<(bool, f64)> {
        let mut out: Vec<(bool, f64)> = Vec::new();
        let n = (hours * 3600.0 * 4.0) as usize;
        for i in 0..n {
            let g = grip_schedule(id, 1000.0 + i as f64 * 0.25, state);
            match out.last_mut() {
                Some((h, len)) if *h == g => *len += 0.25,
                _ => out.push((g, 0.25)),
            }
        }
        out
    }

    fn share(spells: &[(bool, f64)]) -> f64 {
        let all: f64 = spells.iter().map(|s| s.1).sum();
        spells.iter().filter(|s| s.0).map(|s| s.1).sum::<f64>() / all
    }

    /// The grips take turns from time to time: on the hoods, a draped
    /// spell and a handshake spell each last 20 – 90 s when the state
    /// does not change it; cruising mostly draped, on a climb mostly the
    /// handshake (or the hoods instead of the tops), out of the saddle
    /// always the handshake. Each rider its own rhythm.
    #[test]
    fn the_grips_take_turns_every_20_to_90_seconds() {
        for id in [1u32, 2, 7, 1234, 99_999] {
            // The raw turns: every change 20 – 90 s after the last.
            for k in 0..200 {
                let d = grip_change_s(id, k + 1) - grip_change_s(id, k);
                assert!((20.0..=90.0).contains(&d), "rider {id} spell {k}: {d} s");
            }
            let cruise = spells(id, Seated, 2.0);
            // The first and last spells are cut by the sampled window.
            for (h, len) in &cruise[1..cruise.len() - 1] {
                assert!(
                    *len >= 8.0 && *len <= 3.0 * 90.0,
                    "rider {id}: {h} for {len} s"
                );
            }
            let c = share(&cruise);
            assert!((0.1..0.35).contains(&c), "rider {id} cruising: {c}");
            let climb = share(&spells(id, Climbing, 2.0));
            assert!(climb > 0.6, "rider {id} climbing: {climb}");
            assert_eq!(share(&spells(id, Standing, 0.2)), 1.0);
        }
        // Riders differ.
        let a: Vec<bool> = (0..200)
            .map(|i| grip_schedule(1, i as f64 * 10.0, Seated))
            .collect();
        let b: Vec<bool> = (0..200)
            .map(|i| grip_schedule(2, i as f64 * 10.0, Seated))
            .collect();
        assert_ne!(a, b);
    }

    /// Every client shows the same: the schedule is a function of the
    /// rider and the game time only, and the animator reaches the same
    /// grip at any frame rate.
    #[test]
    fn the_grip_schedule_is_deterministic_at_any_frame_rate() {
        for t in [0.0, 13.7, 600.25, 86_400.5] {
            assert_eq!(grip_schedule(42, t, Seated), grip_schedule(42, t, Seated));
        }
        let ride = |fps: f32| {
            let mut a = Animator::new(42);
            let dt = 1.0 / fps;
            let mut out = Vec::new();
            let n = 600 * fps as i64;
            for i in 0..n {
                let t = i as f64 / f64::from(fps);
                let p = a.update(
                    &AnimInput {
                        speed_mps: 10.0,
                        power_w: 220.0,
                        time_s: t,
                        ..Default::default()
                    },
                    false,
                    dt,
                );
                // Sampled once per game second.
                if i % fps as i64 == 0 {
                    out.push(p.handshake);
                }
            }
            out
        };
        let (a, b) = (ride(60.0), ride(30.0));
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(&b) {
            assert!((x - y).abs() < 0.15, "{x} vs {y}");
        }
        assert!(a.iter().any(|h| *h > 0.99) && a.iter().any(|h| *h < 0.01));
    }

    /// A change of grip takes GRIP_BLEND_S (0.4 s), eased: no snapping.
    #[test]
    fn a_grip_change_blends_over_four_tenths_of_a_second() {
        let dt = 1.0 / 60.0;
        let mut a = Animator::new(3);
        let i = AnimInput {
            speed_mps: 10.0,
            power_w: 220.0,
            ..Default::default()
        };
        a.grip_override = Some(false);
        for _ in 0..30 {
            a.update(&i, false, dt);
        }
        assert_eq!(a.update(&i, false, dt).handshake, 0.0);
        a.grip_override = Some(true);
        let mut seen = vec![0.0f32];
        for _ in 0..40 {
            seen.push(a.update(&i, false, dt).handshake);
        }
        let done = seen
            .iter()
            .position(|h| *h >= 1.0)
            .expect("reaches the handshake");
        let s = done as f32 * dt;
        assert!(
            (GRIP_BLEND_S - dt..=GRIP_BLEND_S + dt).contains(&s),
            "{s} s"
        );
        // Eased: slow at the ends, never more than 2.5 % of the way in a
        // frame at 60 fps.
        for w in seen.windows(2) {
            assert!(w[1] >= w[0] && w[1] - w[0] < 0.07, "{w:?}");
        }
        assert!(seen[1] < 0.01, "eased in: {}", seen[1]);
        // And back.
        a.grip_override = Some(false);
        let back: Vec<f32> = (0..40).map(|_| a.update(&i, false, dt).handshake).collect();
        assert_eq!(*back.last().unwrap(), 0.0);
        assert!(back.windows(2).all(|w| w[1] <= w[0]));
    }

    fn effort(speed: f64, cadence: Option<f64>, power: f64, sustained: f64) -> AnimInput {
        AnimInput {
            speed_mps: speed,
            cadence_rpm: cadence,
            power_w: power,
            sustained_w: sustained,
            ..Default::default()
        }
    }

    /// A sprint is a surge well above the power the rider sustains and,
    /// measured, a high cadence – or simply very high power; never a
    /// steady effort, however hard.
    #[test]
    fn a_sprint_is_a_surge_well_above_the_sustained_power() {
        // A rider who sustains 180 W sprints at 420 W, spinning.
        assert_eq!(
            select(Seated, &effort(12.0, Some(105.0), 420.0, 180.0)),
            Sprinting
        );
        // Not grinding at 70 rpm, and not slowly.
        assert_eq!(
            select(Seated, &effort(12.0, Some(70.0), 420.0, 180.0)),
            Seated
        );
        assert_eq!(
            select(Seated, &effort(5.0, Some(105.0), 420.0, 180.0)),
            Seated
        );
        // Others (no cadence): the power alone.
        assert_eq!(select(Seated, &effort(12.0, None, 420.0, 180.0)), Sprinting);
        // A rider sustaining 320 W rides 420 W seated; 700 W is a sprint.
        assert_eq!(
            select(Seated, &effort(12.0, Some(100.0), 420.0, 320.0)),
            Seated
        );
        assert_eq!(
            select(Seated, &effort(12.0, Some(100.0), 700.0, 320.0)),
            Sprinting
        );
        // Hysteresis: the weak rider's sprint goes on down to 280 W.
        assert_eq!(
            select(Sprinting, &effort(12.0, Some(95.0), 300.0, 190.0)),
            Sprinting
        );
        assert_eq!(
            select(Sprinting, &effort(12.0, Some(95.0), 250.0, 190.0)),
            Seated
        );
        assert_eq!(
            select(Sprinting, &effort(12.0, Some(65.0), 400.0, 190.0)),
            Seated
        );
        // Unknown sustained power: only the absolute threshold.
        assert_eq!(sprint_threshold(0.0, false), SPRINT_W.0);
    }

    /// The animator learns the sustained power from the ride: a steady
    /// 500 W (the kiosk's show mode) never sprints; a 180 W rider's surge
    /// to 450 W does, standing on the drops, and sits down on the hoods
    /// when it ends.
    #[test]
    fn the_animator_sprints_on_a_surge_and_sits_down_after() {
        let dt = 1.0 / 30.0;
        let ride = |a: &mut Animator, power: f64, cadence: f64, seconds: f32| {
            let mut p = PoseParams::default();
            for _ in 0..(seconds / dt) as usize {
                p = a.update(&effort(11.0, Some(cadence), power, 0.0), false, dt);
            }
            p
        };
        let mut steady = Animator::new(1);
        for _ in 0..20 {
            ride(&mut steady, 500.0, 95.0, 10.0);
            assert_ne!(steady.state(), Sprinting);
        }
        let mut a = Animator::new(2);
        ride(&mut a, 180.0, 90.0, 300.0);
        assert!((a.sustained_w - 180.0).abs() < 1.0, "{}", a.sustained_w);
        let p = ride(&mut a, 450.0, 110.0, 3.0);
        assert_eq!(a.state(), Sprinting);
        assert!(p.stand > 0.95 && p.drops > 0.95 && p.sprint > 0.95, "{p:?}");
        assert!((p.rock - SPRINT_ROCK).abs() < 0.01);
        // A 12 s sprint lifts the sustained power only a little.
        ride(&mut a, 450.0, 110.0, 9.0);
        assert_eq!(a.state(), Sprinting);
        assert!(a.sustained_w < 240.0, "{}", a.sustained_w);
        let p = ride(&mut a, 170.0, 88.0, 3.0);
        assert_eq!(a.state(), Seated);
        assert!(p.stand < 0.05 && p.drops < 0.05 && p.sprint < 0.05, "{p:?}");
    }

    /// A rider in VR sees the own bike rock less (the real bars stay
    /// still); everyone else as before.
    #[test]
    fn a_vr_riders_bike_rocks_less() {
        let mut a = Animator::new(4);
        let mut b = Animator::new(4);
        let i = effort(12.0, None, 900.0, 0.0);
        let vr = AnimInput {
            signs: Signs {
                in_vr: true,
                ..Default::default()
            },
            ..i
        };
        let (mut p, mut q) = (PoseParams::default(), PoseParams::default());
        for _ in 0..120 {
            p = a.update(&i, false, 1.0 / 60.0);
            q = b.update(&vr, false, 1.0 / 60.0);
        }
        assert!(
            (q.rock - p.rock * VR_ROCK_SHARE).abs() < 1e-4,
            "{} {}",
            p.rock,
            q.rock
        );
    }
}
