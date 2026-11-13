//! God mode's touch controls (spec §14 Phase 13B, §3.2.2), pure:
//!
//! - **one finger** dragged on the 3D view looks around (as the mouse:
//!   moving right looks right, moving up looks up; plane: off the course,
//!   easing back after a moment);
//! - **two fingers** are the stick (as the gamepad's left stick): the
//!   centroid's offset from where they came down – plane: up / down the
//!   throttle (faster / slower), sideways the bank (the turn); hover:
//!   forward / back and strafe – full at [`STICK_PX`]; spreading or
//!   pinching them scales the speed (as the wheel);
//! - the on-screen **+ / −** buttons at the right edge climb and descend
//!   while held ([`ocw_render::inspect_hud::touch_buttons`]: the layout
//!   that is drawn);
//! - a third finger cancels (nothing moves).
//!
//! A touch that came down on a button only presses the button; the others
//! count for looking and moving.

use bevy::math::Vec2;
use ocw_render::inspect_hud::touch_buttons;
use ocw_sim::god::FlightInput;

use super::input::LOOK_RAD_PER_PX;

/// The two-finger stick is at full deflection this far from where the
/// fingers came down (logical px).
pub const STICK_PX: f32 = 90.0;
/// Two-finger offsets below this are centred (px).
pub const STICK_DEAD_PX: f32 = 10.0;
/// The overlay stays this long after the last touch (s, spec §3.2.2).
pub const OVERLAY_S: f64 = 8.0;

/// One finger on the screen (logical px from the window's top left).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Finger {
    pub id: u64,
    pub pos: Vec2,
    pub start: Vec2,
    pub prev: Vec2,
}

/// What the fingers ask for this frame.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TouchFlight {
    pub input: FlightInput,
    /// The view's turn (yaw, pitch; rad), as [`super::input::look_delta`].
    pub look: (f64, f64),
    /// Wheel steps of the speed scale (positive = faster).
    pub wheel: f64,
}

fn stick(offset: f32) -> f64 {
    if offset.abs() < STICK_DEAD_PX {
        0.0
    } else {
        f64::from((offset / STICK_PX).clamp(-1.0, 1.0))
    }
}

/// Maps the fingers on a window of `size` (logical px) to the flight.
pub fn touch_flight(fingers: &[Finger], size: Vec2, invert_y: bool) -> TouchFlight {
    let mut out = TouchFlight::default();
    let buttons = touch_buttons(size);
    let on_button = |f: &Finger| buttons.iter().find(|b| b.contains(f.start));
    let mut climb = 0.0;
    let mut free: Vec<&Finger> = Vec::new();
    for f in fingers {
        match on_button(f) {
            // Held while the finger stays on it.
            Some(b) if b.contains(f.pos) => climb += if b.climb { 1.0 } else { -1.0 },
            Some(_) => {}
            None => free.push(f),
        }
    }
    out.input.climb = f64::clamp(climb, -1.0, 1.0);
    match free.as_slice() {
        [f] => {
            let d = f.pos - f.prev;
            let y = if invert_y { -1.0 } else { 1.0 };
            out.look = (
                -f64::from(d.x) * LOOK_RAD_PER_PX,
                -f64::from(d.y) * LOOK_RAD_PER_PX * y,
            );
        }
        [a, b] => {
            let now = (a.pos + b.pos) / 2.0;
            let start = (a.start + b.start) / 2.0;
            let off = now - start;
            out.input.forward = stick(-off.y);
            out.input.side = stick(off.x);
            let (d, d_prev) = (a.pos.distance(b.pos), a.prev.distance(b.prev));
            if d > 1.0 && d_prev > 1.0 {
                out.wheel = f64::from(d / d_prev).ln() / ocw_sim::god::SCALE_STEP.ln();
            }
        }
        _ => {}
    }
    out
}

/// Whether the touch controls are shown: within [`OVERLAY_S`] of the last
/// touch.
pub fn overlay_shown(last_touch_s: Option<f64>, now_s: f64) -> bool {
    last_touch_s.is_some_and(|t| now_s - t < OVERLAY_S)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: Vec2 = Vec2::new(1280.0, 720.0);

    fn finger(id: u64, start: (f32, f32), prev: (f32, f32), pos: (f32, f32)) -> Finger {
        Finger {
            id,
            start: start.into(),
            prev: prev.into(),
            pos: pos.into(),
        }
    }

    #[test]
    fn one_finger_looks_like_the_mouse() {
        let f = finger(1, (600.0, 400.0), (600.0, 400.0), (620.0, 390.0));
        let t = touch_flight(&[f], SIZE, false);
        let mouse = super::super::input::look_delta((20.0, -10.0), &[], false, 0.016);
        assert_eq!(t.look, mouse);
        assert_eq!(t.input, FlightInput::default(), "looking does not move");
        let inv = touch_flight(&[f], SIZE, true);
        assert_eq!(inv.look.1, -t.look.1);
    }

    #[test]
    fn two_fingers_move_and_pinch_scales_the_speed() {
        // Both pushed up by a full stick: throttle up (hover: forward); to
        // the right a little: a gentle right bank (hover: strafe).
        let a = finger(1, (500.0, 400.0), (540.0, 300.0), (540.0, 300.0));
        let b = finger(2, (700.0, 400.0), (740.0, 300.0), (740.0, 300.0));
        let t = touch_flight(&[a, b], SIZE, false);
        assert_eq!(t.input.forward, 1.0);
        assert!((t.input.side - 40.0 / f64::from(STICK_PX)).abs() < 1e-6);
        // As the gamepad's left stick would (up, a bit right).
        let pad = super::super::input::PadFlight {
            left_y: 1.0,
            left_x: 40.0 / STICK_PX,
            ..Default::default()
        };
        let by_pad = super::super::input::flight_input(&Default::default(), &[pad]);
        assert_eq!(t.input.forward, by_pad.forward);
        assert!((t.input.side - by_pad.side).abs() < 1e-6);
        // Pulled down and to the left: slower, banking left.
        let a = finger(1, (500.0, 300.0), (420.0, 400.0), (420.0, 400.0));
        let b = finger(2, (700.0, 300.0), (620.0, 400.0), (620.0, 400.0));
        let t = touch_flight(&[a, b], SIZE, false);
        assert!(
            t.input.forward < -0.9 && t.input.side < -0.8,
            "{:?}",
            t.input
        );
        assert_eq!(t.look, (0.0, 0.0), "two fingers do not look");
        assert_eq!(t.wheel, 0.0);
        // Spread from 200 to 250 px apart: faster by that factor.
        let a = finger(1, (500.0, 400.0), (500.0, 400.0), (475.0, 400.0));
        let b = finger(2, (700.0, 400.0), (700.0, 400.0), (725.0, 400.0));
        let t = touch_flight(&[a, b], SIZE, false);
        let factor = ocw_sim::god::SCALE_STEP.powf(t.wheel);
        assert!((factor - 1.25).abs() < 1e-4, "{factor}");
        assert_eq!(t.input.forward, 0.0, "a centred pinch does not move");
        // Three fingers: nothing.
        let c = finger(3, (100.0, 100.0), (100.0, 100.0), (300.0, 300.0));
        assert_eq!(
            touch_flight(&[a, b, c], SIZE, false),
            TouchFlight::default()
        );
    }

    #[test]
    fn the_buttons_climb_and_descend_while_held() {
        let [up, down] = touch_buttons(SIZE);
        let at = |c: Vec2| (c.x, c.y);
        let press = |b: Vec2| finger(9, at(b), at(b), at(b));
        assert_eq!(
            touch_flight(&[press(up.centre)], SIZE, false).input.climb,
            1.0
        );
        assert_eq!(
            touch_flight(&[press(down.centre)], SIZE, false).input.climb,
            -1.0
        );
        // Slid off the button: released (and it does not look either).
        let off = finger(9, at(up.centre), at(up.centre), (400.0, 300.0));
        assert_eq!(touch_flight(&[off], SIZE, false), TouchFlight::default());
        // Climbing with one finger, looking with another.
        let look = finger(1, (300.0, 300.0), (300.0, 300.0), (310.0, 300.0));
        let t = touch_flight(&[press(up.centre), look], SIZE, false);
        assert_eq!(t.input.climb, 1.0);
        assert!(t.look.0 < 0.0);
    }

    #[test]
    fn the_overlay_fades_after_eight_seconds() {
        assert!(!overlay_shown(None, 10.0));
        assert!(overlay_shown(Some(5.0), 12.9));
        assert!(!overlay_shown(Some(5.0), 13.1));
    }
}
