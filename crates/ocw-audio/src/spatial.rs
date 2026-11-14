//! Where a sound is (spec §3.3, other riders): distance attenuation, stereo
//! panning by direction, air absorption and a subtle Doppler shift, for a
//! source relative to the listener. Pure and unit-tested; positions are the
//! world's local metres (`x`, `y` = up, `z`), as in [`ocw_world::RoutePoint`].

/// Full level up to this distance (m): a rider's wheel beside the own.
pub const REF_DISTANCE_M: f64 = 2.0;
/// Roll-off of the inverse-distance law (1 = physical: -6 dB per doubling).
pub const ROLLOFF: f64 = 1.0;
/// From here the level fades to silence …
pub const FADE_START_M: f64 = 50.0;
/// … reached here: beyond it a rider is not heard (and costs nothing).
pub const MAX_DISTANCE_M: f64 = 70.0;
/// Speed of sound (m/s).
pub const SPEED_OF_SOUND_MPS: f64 = 343.0;
/// Only this fraction of the physical Doppler shift is applied …
pub const DOPPLER_SCALE: f64 = 0.5;
/// … and never more than ±3 %: riders passing should not wail.
pub const DOPPLER_MAX: f64 = 0.03;
/// Air absorption: the low-pass cutoff right at the source (Hz) …
pub const AIR_CUTOFF_NEAR_HZ: f64 = 16_000.0;
/// … halved at this distance (m), a third at twice it, and so on.
pub const AIR_HALF_M: f64 = 10.0;
/// Sources behind the listener are this much quieter (head shadow) …
pub const BEHIND_GAIN: f64 = 0.75;
/// … and duller (factor on the cutoff).
pub const BEHIND_CUTOFF: f64 = 0.7;
/// Closer than this (horizontally, m) the pan narrows to the centre (a
/// source right at the head is not hard left or right).
pub const NEAR_FIELD_M: f64 = 1.0;

/// Who hears: the own rider's head (spec §3.3).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Listener {
    pub pos: [f64; 3],
    /// Horizontal unit vector the listener faces (`x`, `z`).
    pub forward: [f64; 2],
    pub velocity: [f64; 3],
}

/// How one source is heard.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub distance_m: f64,
    /// Overall level by distance (and head shadow), 0–1.
    pub gain: f32,
    /// −1 (left) … 1 (right).
    pub pan: f32,
    /// Per-channel gains: equal power, `left² + right² = 2 gain²` (a
    /// source straight ahead plays at `gain` on both).
    pub left: f32,
    pub right: f32,
    /// Low-pass for the air absorption (Hz).
    pub air_cutoff_hz: f32,
    /// Frequency factor (1 = none, > 1 coming closer).
    pub doppler: f32,
}

impl Placement {
    pub fn audible(&self) -> bool {
        self.gain > 0.0
    }
}

/// Level by distance: 1 up to [`REF_DISTANCE_M`], then the inverse-distance
/// law (`ref / (ref + rolloff (d - ref))`), faded smoothly to exactly 0
/// between [`FADE_START_M`] and [`MAX_DISTANCE_M`].
pub fn distance_gain(d: f64) -> f64 {
    if !d.is_finite() || d >= MAX_DISTANCE_M {
        return 0.0;
    }
    let d = d.max(0.0);
    if d >= MAX_DISTANCE_M {
        return 0.0;
    }
    let inv = if d <= REF_DISTANCE_M {
        1.0
    } else {
        REF_DISTANCE_M / (REF_DISTANCE_M + ROLLOFF * (d - REF_DISTANCE_M))
    };
    let fade = if d <= FADE_START_M {
        1.0
    } else {
        let x = (d - FADE_START_M) / (MAX_DISTANCE_M - FADE_START_M);
        1.0 - x * x * (3.0 - 2.0 * x)
    };
    inv * fade
}

/// The air absorption's low-pass cutoff at distance `d` (Hz): far riders
/// lose their hiss and click first.
pub fn air_cutoff_hz(d: f64) -> f64 {
    AIR_CUTOFF_NEAR_HZ * AIR_HALF_M / (AIR_HALF_M + d.max(0.0))
}

/// Equal-power stereo gains of `pan` (−1 left … 1 right), scaled so the
/// centre is 1 on both channels.
pub fn pan_gains(pan: f64) -> (f64, f64) {
    let a = (pan.clamp(-1.0, 1.0) + 1.0) * std::f64::consts::FRAC_PI_4;
    let s = std::f64::consts::SQRT_2;
    (s * a.cos(), s * a.sin())
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// The Doppler factor of a source at `pos` moving with `velocity`: the
/// physical `(c + v_listener) / (c - v_source)` (speeds towards each
/// other), scaled by [`DOPPLER_SCALE`] and kept within ±[`DOPPLER_MAX`].
pub fn doppler(listener: &Listener, pos: [f64; 3], velocity: [f64; 3]) -> f64 {
    let to = sub(pos, listener.pos);
    let d = dot(to, to).sqrt();
    if d < 1e-6 {
        return 1.0;
    }
    let u = [to[0] / d, to[1] / d, to[2] / d];
    let c = SPEED_OF_SOUND_MPS;
    let towards_source = dot(listener.velocity, u);
    let towards_listener = -dot(velocity, u);
    let f = (c + towards_source) / (c - towards_listener.min(c * 0.5));
    (1.0 + (f - 1.0) * DOPPLER_SCALE).clamp(1.0 - DOPPLER_MAX, 1.0 + DOPPLER_MAX)
}

/// How a source at `pos` moving with `velocity` is heard by `listener`.
pub fn place(listener: &Listener, pos: [f64; 3], velocity: [f64; 3]) -> Placement {
    let to = sub(pos, listener.pos);
    let distance = dot(to, to).sqrt();
    let horizontal = (to[0] * to[0] + to[2] * to[2]).sqrt();
    let [fx, fz] = listener.forward;
    // Right of the forward vector (as `RoutePoint::right`).
    let (rx, rz) = (-fz, fx);
    let (side, ahead) = if horizontal > 1e-9 {
        (
            (to[0] * rx + to[2] * rz) / horizontal,
            (to[0] * fx + to[2] * fz) / horizontal,
        )
    } else {
        (0.0, 1.0)
    };
    let pan = side * (horizontal / NEAR_FIELD_M).min(1.0);
    let behind = (-ahead).max(0.0);
    let gain = distance_gain(distance) * (1.0 - (1.0 - BEHIND_GAIN) * behind);
    let (l, r) = pan_gains(pan);
    let cutoff = air_cutoff_hz(distance) * (1.0 - (1.0 - BEHIND_CUTOFF) * behind);
    Placement {
        distance_m: distance,
        gain: gain as f32,
        pan: pan as f32,
        left: (gain * l) as f32,
        right: (gain * r) as f32,
        air_cutoff_hz: cutoff as f32,
        doppler: doppler(listener, pos, velocity) as f32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// At the origin, facing +x (right is +z), standing.
    fn listener() -> Listener {
        Listener {
            pos: [0.0; 3],
            forward: [1.0, 0.0],
            velocity: [0.0; 3],
        }
    }

    #[test]
    fn level_falls_with_distance_and_is_cut_off() {
        assert_eq!(distance_gain(0.0), 1.0);
        assert_eq!(distance_gain(REF_DISTANCE_M), 1.0);
        // Inverse distance: twice the distance, half the level.
        assert!((distance_gain(4.0) - 0.5).abs() < 1e-9);
        assert!((distance_gain(8.0) - 0.25).abs() < 1e-9);
        assert!((distance_gain(20.0) - 0.1).abs() < 1e-9);
        // Falling all the way, smoothly into the cut-off.
        let mut last = 1.0;
        for i in 1..=800 {
            let g = distance_gain(f64::from(i) * 0.1);
            assert!(g <= last, "{i}");
            last = g;
        }
        assert!(distance_gain(FADE_START_M) > 0.03);
        assert!(distance_gain(60.0) < distance_gain(FADE_START_M) * 0.6);
        assert!(distance_gain(MAX_DISTANCE_M - 0.5) < 1e-3);
        assert_eq!(distance_gain(MAX_DISTANCE_M), 0.0);
        assert_eq!(distance_gain(500.0), 0.0);
        assert_eq!(distance_gain(f64::NAN), 0.0);
        // The placement too: beyond the range not audible at all.
        let far = place(&listener(), [75.0, 0.0, 0.0], [0.0; 3]);
        assert!(!far.audible());
        assert_eq!((far.left, far.right), (0.0, 0.0));
        assert!(place(&listener(), [30.0, 0.0, 0.0], [0.0; 3]).audible());
    }

    #[test]
    fn pans_by_direction() {
        let l = listener();
        let at = |x: f64, z: f64| place(&l, [x, 0.0, z], [0.0; 3]);
        // Ahead: centre, both channels alike.
        let ahead = at(10.0, 0.0);
        assert_eq!(ahead.pan, 0.0);
        assert!((ahead.left - ahead.right).abs() < 1e-6);
        assert!((ahead.left - ahead.gain).abs() < 1e-6);
        // Right (+z) and left (-z).
        let right = at(0.0, 3.0);
        assert!((right.pan - 1.0).abs() < 1e-6);
        assert!(right.right > 1.4 * right.gain && right.left.abs() < 1e-6);
        let left = at(0.0, -3.0);
        assert!((left.pan + 1.0).abs() < 1e-6 && left.left > left.right);
        // Diagonal: in between; equal power at every angle.
        let diag = at(3.0, 3.0);
        assert!(diag.pan > 0.6 && diag.pan < 0.8, "{}", diag.pan);
        for p in [ahead, right, left, diag] {
            let power = p.left * p.left + p.right * p.right;
            assert!((power - 2.0 * p.gain * p.gain).abs() < 1e-5);
        }
        // Turning the listener turns the scene: facing +z, +z is ahead.
        let turned = Listener {
            forward: [0.0, 1.0],
            ..l
        };
        assert!(place(&turned, [0.0, 0.0, 3.0], [0.0; 3]).pan.abs() < 1e-6);
        assert!(place(&turned, [3.0, 0.0, 0.0], [0.0; 3]).pan < -0.99);
        // Right at the head: centred, not hard to one side.
        assert!(at(0.0, 0.3).pan < 0.31);
        // Behind: quieter and duller than ahead at the same distance.
        let behind = at(-10.0, 0.0);
        assert!(behind.gain < ahead.gain && behind.air_cutoff_hz < ahead.air_cutoff_hz);
    }

    #[test]
    fn air_absorbs_the_highs_with_distance() {
        assert_eq!(air_cutoff_hz(0.0), AIR_CUTOFF_NEAR_HZ);
        assert!((air_cutoff_hz(AIR_HALF_M) - AIR_CUTOFF_NEAR_HZ / 2.0).abs() < 1e-6);
        assert!(air_cutoff_hz(60.0) < 2500.0);
        let l = listener();
        let near = place(&l, [3.0, 0.0, 0.0], [0.0; 3]);
        let far = place(&l, [40.0, 0.0, 0.0], [0.0; 3]);
        assert!(far.air_cutoff_hz < near.air_cutoff_hz / 3.0);
    }

    #[test]
    fn doppler_is_subtle() {
        let l = listener();
        // Coming towards the listener at 15 m/s (overtaking from behind,
        // the listener standing): a little higher, gone away lower.
        let coming = doppler(&l, [-20.0, 0.0, 0.0], [15.0, 0.0, 0.0]);
        let going = doppler(&l, [20.0, 0.0, 0.0], [15.0, 0.0, 0.0]);
        assert!(coming > 1.01 && coming <= 1.0 + DOPPLER_MAX, "{coming}");
        assert!((1.0 - DOPPLER_MAX..0.99).contains(&going), "{going}");
        // Riding together at the same speed: no shift.
        let together = Listener {
            velocity: [10.0, 0.0, 0.0],
            ..l
        };
        assert!((doppler(&together, [5.0, 0.0, 1.0], [10.0, 0.0, 0.0]) - 1.0).abs() < 1e-9);
        // Absurd speeds stay within the cap.
        assert_eq!(
            doppler(&l, [-5.0, 0.0, 0.0], [300.0, 0.0, 0.0]),
            1.0 + DOPPLER_MAX
        );
        assert_eq!(doppler(&l, [0.0; 3], [9.0, 0.0, 0.0]), 1.0);
    }
}
