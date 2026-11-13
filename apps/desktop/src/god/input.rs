//! God mode's input mapping (spec §14 Phase 13A, 13B), pure:
//!
//! - **G** toggles god mode (leaving lands on the road below), **Shift + G**
//!   flies back to the take-off spot; gamepad: **Select / Back + North**
//!   toggles (a chord, so it is not pressed by accident), **Select + West**
//!   flies back.
//! - **Cursor keys** move in the horizontal plane relative to the view's
//!   yaw (↑ forward, ↓ back, ← / → sideways); **+ / −** (main keyboard
//!   `=`/`+` and `-`, numeric keypad + / −) climb and descend. W / A / S /
//!   D stay with riding (open question Q3: [`WASD_FLIES`]).
//! - **Shift** boosts (×4), **Ctrl** is precise (×0.25), the **mouse
//!   wheel** scales the base speed; the **mouse** looks around.
//! - Gamepad: left stick moves, right stick looks, right / left trigger
//!   climb / descend, South (A / ✕) boosts.

use ocw_sim::god::FlightInput;

/// Open question Q3 (spec §14 Phase 13H): W / A / S / D also fly. Off (the
/// proposal: cursor keys only; A and W keep autodrive and wild mode).
pub const WASD_FLIES: bool = false;

/// Radians of view turn per pixel of mouse movement (the chase camera's
/// orbit speed, `camera::DRAG_SENSITIVITY`).
pub const LOOK_RAD_PER_PX: f64 = 0.005;
/// The gamepad's right stick turns the view this fast at full deflection
/// (rad/s).
pub const PAD_LOOK_RAD_S: f64 = 2.2;
/// Stick and trigger values below this are centred / released.
pub const DEAD_ZONE: f32 = 0.12;

/// The flight keys held this frame.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FlightKeys {
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
    /// `=` / `+` (main keyboard) or keypad +.
    pub plus: bool,
    /// `-` (main keyboard) or keypad −.
    pub minus: bool,
    pub shift: bool,
    pub ctrl: bool,
}

/// One gamepad's flight controls.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PadFlight {
    pub left_x: f32,
    pub left_y: f32,
    pub right_x: f32,
    pub right_y: f32,
    /// How far the triggers are pulled (0 … 1, above their rest).
    pub right_trigger: f32,
    pub left_trigger: f32,
    pub south: bool,
}

fn dead(v: f32) -> f64 {
    if v.abs() < DEAD_ZONE {
        0.0
    } else {
        f64::from(v.clamp(-1.0, 1.0))
    }
}

fn axis(plus: bool, minus: bool) -> f64 {
    f64::from(i8::from(plus) - i8::from(minus))
}

/// The flight input from the keys and the gamepads.
pub fn flight_input(k: &FlightKeys, pads: &[PadFlight]) -> FlightInput {
    let sum = |f: &dyn Fn(&PadFlight) -> f64| pads.iter().map(f).sum::<f64>();
    let clamp = |v: f64| v.clamp(-1.0, 1.0);
    FlightInput {
        forward: clamp(axis(k.up, k.down) + sum(&|p| dead(p.left_y))),
        strafe: clamp(axis(k.right, k.left) + sum(&|p| dead(p.left_x))),
        climb: clamp(
            axis(k.plus, k.minus)
                + sum(&|p| dead(p.right_trigger).max(0.0) - dead(p.left_trigger).max(0.0)),
        ),
        boost: k.shift || pads.iter().any(|p| p.south),
        precise: k.ctrl,
    }
}

/// The view's turn this frame (yaw, pitch; rad) from the mouse (pixels;
/// only while the pointer is captured or the left button held) and the
/// right sticks: moving right turns right (the yaw turns left for
/// positive values), moving up looks up (unless `invert_y`).
pub fn look_delta(mouse: (f32, f32), pads: &[PadFlight], invert_y: bool, dt: f64) -> (f64, f64) {
    let y = if invert_y { -1.0 } else { 1.0 };
    let mut yaw = -f64::from(mouse.0) * LOOK_RAD_PER_PX;
    let mut pitch = -f64::from(mouse.1) * LOOK_RAD_PER_PX * y;
    for p in pads {
        yaw -= dead(p.right_x) * PAD_LOOK_RAD_S * dt;
        pitch += dead(p.right_y) * PAD_LOOK_RAD_S * dt * y;
    }
    (yaw, pitch)
}

/// What the god-mode keys ask for this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GodKey {
    None,
    /// G, Select + North: enter, or leave (land below).
    Toggle,
    /// Shift + G, Select + West: fly back (only while flying).
    FlyBack,
}

/// Maps G (pressed this frame) with Shift held, and the gamepad chord.
pub fn god_key(g: bool, shift: bool, select_held: bool, north: bool, west: bool) -> GodKey {
    match (g, shift) {
        (true, true) => GodKey::FlyBack,
        (true, false) => GodKey::Toggle,
        _ if select_held && north => GodKey::Toggle,
        _ if select_held && west => GodKey::FlyBack,
        _ => GodKey::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_keys_move_and_plus_minus_climb() {
        let k = |f: &dyn Fn(&mut FlightKeys)| {
            let mut k = FlightKeys::default();
            f(&mut k);
            flight_input(&k, &[])
        };
        assert_eq!(k(&|k| k.up = true).forward, 1.0);
        assert_eq!(k(&|k| k.down = true).forward, -1.0);
        assert_eq!(k(&|k| k.left = true).strafe, -1.0);
        assert_eq!(k(&|k| k.right = true).strafe, 1.0);
        assert_eq!(k(&|k| k.plus = true).climb, 1.0);
        assert_eq!(k(&|k| k.minus = true).climb, -1.0);
        let both = k(&|k| {
            k.up = true;
            k.down = true;
        });
        assert_eq!(both.forward, 0.0);
        assert!(k(&|k| k.shift = true).boost);
        assert!(k(&|k| k.ctrl = true).precise);
        assert_eq!(
            flight_input(&FlightKeys::default(), &[]),
            FlightInput::default()
        );
    }

    #[test]
    fn gamepad_sticks_and_triggers() {
        let pad = PadFlight {
            left_x: 0.5,
            left_y: -1.0,
            right_trigger: 0.8,
            left_trigger: 0.05,
            south: true,
            ..Default::default()
        };
        let i = flight_input(&FlightKeys::default(), &[pad]);
        assert_eq!((i.forward, i.strafe), (-1.0, 0.5));
        assert!((i.climb - 0.8).abs() < 1e-6, "the left trigger rests");
        assert!(i.boost);
        // A resting stick does nothing.
        let rest = PadFlight {
            left_x: 0.05,
            left_y: -0.1,
            ..Default::default()
        };
        assert_eq!(
            flight_input(&FlightKeys::default(), &[rest]),
            FlightInput::default()
        );
        // Keys and pads together are clamped.
        let k = FlightKeys {
            down: true,
            ..Default::default()
        };
        assert_eq!(flight_input(&k, &[pad]).forward, -1.0);
    }

    #[test]
    fn the_mouse_and_the_right_stick_look() {
        let (yaw, pitch) = look_delta((10.0, -20.0), &[], false, 0.016);
        assert!(yaw < 0.0, "moving right turns right");
        assert!(pitch > 0.0, "moving up looks up");
        let (_, inverted) = look_delta((10.0, -20.0), &[], true, 0.016);
        assert_eq!(inverted, -pitch);
        let stick = PadFlight {
            right_x: 1.0,
            ..Default::default()
        };
        let (yaw, _) = look_delta((0.0, 0.0), &[stick], false, 0.5);
        assert!((yaw + PAD_LOOK_RAD_S * 0.5).abs() < 1e-9);
    }

    #[test]
    fn g_shift_g_and_the_gamepad_chord() {
        assert_eq!(god_key(true, false, false, false, false), GodKey::Toggle);
        assert_eq!(god_key(true, true, false, false, false), GodKey::FlyBack);
        assert_eq!(god_key(false, false, true, true, false), GodKey::Toggle);
        assert_eq!(god_key(false, false, true, false, true), GodKey::FlyBack);
        // North alone (J: rejoin the route) or Select alone: nothing.
        assert_eq!(god_key(false, false, false, true, false), GodKey::None);
        assert_eq!(god_key(false, false, true, false, false), GodKey::None);
        const { assert!(!WASD_FLIES) };
    }
}
