//! The chase camera's reaction to the own rider's crash or a hard bump
//! (spec §14 Phase 12C): a short **shake** (the eye jolts and rolls a
//! little, gone within [`SHAKE_S`]) and, while the rider is down, a
//! **wider orbit** (further out, higher and turned a little to the side the
//! rider fell to, so the fall and the bike lying apart are in view; eased
//! in over the fall, out over the remount).
//!
//! Everything is a pure function of the time since the event – the crash's
//! elapsed time from the server, the bump's server tick – so a capture at
//! a fixed game-time step (`ocw-desktop --capture`) renders it smoothly and
//! identically every run. The shake can be turned off (motion comfort,
//! setting `display.camera_shake`); the wider orbit is slow and stays.

use ocw_render::CameraFx;
use ocw_sim::collide::{self, Bump, CrashPhase, CrashView};

/// How long a shake lasts (s).
pub const SHAKE_S: f64 = 0.8;
/// It builds up over this long (s) …
pub const SHAKE_ATTACK_S: f64 = 0.04;
/// The eye's largest jolt (m) and roll (rad, about 2°) for a crash.
pub const SHAKE_M: f64 = 0.12;
pub const SHAKE_ROLL_RAD: f64 = 0.035;
/// A bump shakes the camera from this wobble strength on (rad: a closing
/// speed of about 0.9 m/s) – a "hard bump"; softer bumps only wobble.
pub const HARD_BUMP_RAD: f64 = 0.07;
/// A hard bump shakes with this share of a crash's strength at the
/// threshold, up to [`BUMP_SHAKE_MAX`] at the strongest wobble.
pub const BUMP_SHAKE_MIN: f64 = 0.25;
pub const BUMP_SHAKE_MAX: f64 = 0.6;

/// The shake's envelope `age_s` after the event (0–1): up over
/// [`SHAKE_ATTACK_S`], then fading out smoothly to 0 at [`SHAKE_S`].
pub fn shake_envelope(age_s: f64) -> f64 {
    if !(0.0..SHAKE_S).contains(&age_s) {
        return 0.0;
    }
    let attack = (age_s / SHAKE_ATTACK_S).min(1.0);
    let fade = 1.0 - age_s / SHAKE_S;
    attack * fade * fade
}

/// The shake's strength of a bump (0: not a hard bump).
pub fn bump_strength(wobble_rad: f64) -> f64 {
    if wobble_rad < HARD_BUMP_RAD {
        return 0.0;
    }
    let span = (collide::WOBBLE_MAX_RAD - HARD_BUMP_RAD).max(1e-6);
    let x = ((wobble_rad - HARD_BUMP_RAD) / span).clamp(0.0, 1.0);
    BUMP_SHAKE_MIN + (BUMP_SHAKE_MAX - BUMP_SHAKE_MIN) * x
}

/// The shake `age_s` after an event of `strength` (1 = a crash): the eye's
/// offset (m: right, up, forward) and roll (rad). A sum of sines at
/// unrelated frequencies under the envelope: smooth, not periodic over
/// the shake, the same for the same age.
pub fn shake(age_s: f64, strength: f64) -> ([f32; 3], f32) {
    let e = shake_envelope(age_s) * strength.clamp(0.0, 1.0);
    if e == 0.0 {
        return ([0.0; 3], 0.0);
    }
    let w = |hz: f64, phase: f64| (std::f64::consts::TAU * hz * age_s + phase).sin();
    let n = |a: f64, b: f64, c: f64, d: f64| (w(a, b) + 0.5 * w(c, d)) / 1.5;
    let m = SHAKE_M * e;
    (
        [
            (m * n(7.0, 1.3, 11.0, 0.2)) as f32,
            (m * n(9.0, 0.4, 13.0, 2.0)) as f32,
            (0.5 * m * n(5.0, 2.6, 12.0, 1.1)) as f32,
        ],
        (SHAKE_ROLL_RAD * e * n(6.0, 0.9, 10.0, 2.4)) as f32,
    )
}

/// How far the orbit is widened while down (0–1): eased in over the fall,
/// held while lying, eased out over the remount.
pub fn down_orbit(crash: &CrashView) -> f64 {
    let smooth = |x: f64| {
        let x = x.clamp(0.0, 1.0);
        x * x * (3.0 - 2.0 * x)
    };
    match crash.phase {
        CrashPhase::Falling => smooth(crash.elapsed_s / collide::FALL_S),
        CrashPhase::Down => 1.0,
        CrashPhase::Remount => {
            let into = crash.elapsed_s - collide::FALL_S - crash.down_s;
            1.0 - smooth(into / collide::REMOUNT_S)
        }
    }
}

/// The camera's reaction now: the own rider's crash (from the server), its
/// last bump and how long ago it was (s), and whether the shake is on.
pub fn camera_fx(
    crash: Option<&CrashView>,
    bump: Option<(&Bump, f64)>,
    shake_on: bool,
) -> CameraFx {
    let mut fx = CameraFx::default();
    let mut add = |age: f64, strength: f64| {
        let (o, r) = shake(age, strength);
        for (a, b) in fx.shake.iter_mut().zip(o) {
            *a += b;
        }
        fx.roll += r;
    };
    if let Some(c) = crash {
        if shake_on {
            add(c.elapsed_s, 1.0);
        }
        fx.orbit = down_orbit(c) as f32;
        // The push's side: the rider falls (and the camera turns) that way.
        fx.side = if c.impulse[1] < 0.0 { -1.0 } else { 1.0 };
    } else if let Some((b, age)) = bump
        && shake_on
    {
        add(age, bump_strength(b.strength));
    }
    fx
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_sim::collide::ContactKind;

    fn crash(elapsed_s: f64) -> CrashView {
        CrashView {
            phase: collide::crash_phase(elapsed_s, 3.0).unwrap(),
            elapsed_s,
            down_s: 3.0,
            kind: ContactKind::WheelTouch,
            by: None,
            impulse: [-1.0, -0.8],
            speed: 9.0,
        }
    }

    #[test]
    fn the_shake_rises_fast_and_is_gone_after_0_8_s() {
        assert_eq!(shake_envelope(-0.01), 0.0);
        assert_eq!(shake_envelope(0.0), 0.0);
        assert!(shake_envelope(SHAKE_ATTACK_S) > 0.85);
        // Only down after the attack, to zero at the end.
        let mut last = shake_envelope(SHAKE_ATTACK_S);
        let mut t = SHAKE_ATTACK_S;
        while t < SHAKE_S {
            let e = shake_envelope(t);
            assert!(e <= last + 1e-12, "{t}");
            last = e;
            t += 0.01;
        }
        assert!(shake_envelope(SHAKE_S - 0.01) < 0.01);
        assert_eq!(shake_envelope(SHAKE_S), 0.0);
        assert_eq!(shake_envelope(5.0), 0.0);
    }

    #[test]
    fn the_shake_is_smooth_bounded_and_repeatable() {
        let mut prev = shake(0.0, 1.0);
        let mut biggest: f32 = 0.0;
        for i in 1..=100 {
            let t = f64::from(i) * 0.01;
            let s = shake(t, 1.0);
            assert_eq!(s, shake(t, 1.0));
            for k in 0..3 {
                assert!(s.0[k].abs() <= SHAKE_M as f32 + 1e-6);
                // At 100 Hz sampling no jump over a few centimetres.
                assert!((s.0[k] - prev.0[k]).abs() < 0.08, "{t}: {s:?}");
                biggest = biggest.max(s.0[k].abs());
            }
            assert!(s.1.abs() <= SHAKE_ROLL_RAD as f32 + 1e-6);
            prev = s;
        }
        assert!(biggest > 0.03, "it shakes: {biggest}");
        assert_eq!(shake(1.0, 1.0), ([0.0; 3], 0.0));
        // Half the strength, half the shake.
        let (a, b) = (shake(0.1, 1.0), shake(0.1, 0.5));
        assert!((a.0[0] * 0.5 - b.0[0]).abs() < 1e-6);
    }

    #[test]
    fn hard_bumps_shake_soft_ones_do_not() {
        assert_eq!(bump_strength(collide::WOBBLE_MIN_RAD), 0.0);
        assert_eq!(bump_strength(HARD_BUMP_RAD - 0.001), 0.0);
        assert!((bump_strength(HARD_BUMP_RAD) - BUMP_SHAKE_MIN).abs() < 1e-9);
        assert!((bump_strength(collide::WOBBLE_MAX_RAD) - BUMP_SHAKE_MAX).abs() < 1e-9);
        let hard = Bump {
            tick: 10,
            strength: collide::WOBBLE_MAX_RAD,
            side: 1.0,
        };
        let soft = Bump {
            strength: collide::WOBBLE_MIN_RAD,
            ..hard
        };
        assert_ne!(camera_fx(None, Some((&hard, 0.1)), true).shake, [0.0; 3]);
        assert_eq!(
            camera_fx(None, Some((&soft, 0.1)), true),
            CameraFx::default()
        );
        // The setting off (motion comfort): no shake.
        assert_eq!(
            camera_fx(None, Some((&hard, 0.1)), false),
            CameraFx::default()
        );
    }

    #[test]
    fn the_orbit_widens_while_down_and_eases_back() {
        assert_eq!(down_orbit(&crash(0.0)), 0.0);
        assert!(down_orbit(&crash(0.4)) > 0.3 && down_orbit(&crash(0.4)) < 0.7);
        assert_eq!(down_orbit(&crash(collide::FALL_S + 1.0)), 1.0);
        let remount = collide::FALL_S + 3.0;
        assert!((down_orbit(&crash(remount + 0.01)) - 1.0).abs() < 0.01);
        assert!(down_orbit(&crash(remount + collide::REMOUNT_S - 0.01)) < 0.01);
        // A crash shakes at once and turns to the side of the fall; with
        // the shake off the orbit still widens.
        let fx = camera_fx(Some(&crash(0.1)), None, true);
        assert_ne!(fx.shake, [0.0; 3]);
        assert_eq!(fx.side, -1.0);
        let calm = camera_fx(Some(&crash(2.0)), None, false);
        assert_eq!((calm.shake, calm.roll, calm.orbit), ([0.0; 3], 0.0, 1.0));
    }
}
