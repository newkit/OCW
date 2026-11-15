//! God mode's input mapping (spec §14 Phase 13A, 13B), pure:
//!
//! - **G** toggles god mode (leaving lands on the road below), **Shift + G**
//!   flies back to the take-off spot; gamepad: **Select / Back + North**
//!   toggles (a chord, so it is not pressed by accident), **Select + West**
//!   flies back.
//! - **Cursor keys** (plane, the default): ↑ / ↓ the throttle (faster /
//!   slower along the heading; it holds when released), ← / → bank and
//!   turn (hover: move in the horizontal plane relative to the view's yaw,
//!   ↑ forward, ↓ back, ← / → sideways). **+ / −** climb and descend. W /
//!   A / S / D stay with riding (open question Q3: [`WASD_FLIES`]).
//! - **+ / − by what the key types** ([`LayoutKeys`], spec §14 Phase 13B,
//!   v2.64): the key that types `+` (or `=`, the US `+` key without Shift)
//!   climbs, the key that types `-` (or `_`) descends, wherever the layout
//!   puts it – on a German keyboard the `+` key is physically the US `]`
//!   and `-` the US `/` (bug 2026-11-13: only the keypad worked there). A
//!   key keeps its role while held and for the session, so Shift (boost)
//!   with the German `+` (which types `*`) climbs too once that key was
//!   pressed alone. The numeric keypad + / − always work.
//! - **Shift** boosts (×4), **Ctrl** is precise (×0.25), the **mouse
//!   wheel** scales the base speed; the **mouse** looks around (plane:
//!   without changing the course, easing back after a moment; **L** or the
//!   middle mouse button looks straight ahead again).
//! - Gamepad: left stick ← / → bank (hover: strafe), ↑ / ↓ throttle
//!   (hover: forward / back), right stick looks, right / left trigger climb
//!   / descend, South (A / ✕) boosts.
//!
//! The other keys of riding and flying are letters, arrows, function and
//! editing keys, matched by position (`KeyCode`) as before; the console's
//! `` ` `` is the key below Esc on every layout (`^` on a German one).

use std::collections::HashMap;

use bevy::input::keyboard::{Key, KeyCode};
use ocw_sim::god::FlightInput;

/// What a layout-dependent flight key does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyRole {
    /// Climb (+).
    Plus,
    /// Descend (−).
    Minus,
}

/// The role of a key by what it types (the logical key): `+` and `=` climb,
/// `-` and `_` descend.
pub fn role_of(logical: &Key) -> Option<KeyRole> {
    match logical {
        Key::Character(s) => match s.as_str() {
            "+" | "=" => Some(KeyRole::Plus),
            "-" | "_" => Some(KeyRole::Minus),
            _ => None,
        },
        _ => None,
    }
}

/// The physical keys that always climb / descend (the numeric keypad).
pub const KEYPAD_PLUS: KeyCode = KeyCode::NumpadAdd;
pub const KEYPAD_MINUS: KeyCode = KeyCode::NumpadSubtract;

/// The + / − keys of the keyboard's layout, learnt from the key events
/// (pure; fed by `god::layout_keys`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LayoutKeys {
    /// The role each physical key had when it typed `+` / `-` (kept for
    /// the session: Shift changes what it types, not what it is).
    learned: HashMap<KeyCode, KeyRole>,
    /// The keys held now and their role.
    held: HashMap<KeyCode, KeyRole>,
}

impl LayoutKeys {
    /// A key event: `code` (physical) typed `logical`, pressed or released.
    pub fn on_key(&mut self, code: KeyCode, logical: &Key, pressed: bool) {
        if !pressed {
            self.held.remove(&code);
            return;
        }
        let role = match role_of(logical) {
            Some(r) => {
                self.learned.insert(code, r);
                Some(r)
            }
            None => self.learned.get(&code).copied(),
        };
        match role {
            Some(r) => {
                self.held.insert(code, r);
            }
            None => {
                self.held.remove(&code);
            }
        }
    }

    /// Whether a key of `role` is held; `pressed` is the physical key state
    /// (a release lost to a focus change does not stick), the keypad
    /// counts always.
    pub fn held(&self, role: KeyRole, pressed: &dyn Fn(KeyCode) -> bool) -> bool {
        let keypad = match role {
            KeyRole::Plus => KEYPAD_PLUS,
            KeyRole::Minus => KEYPAD_MINUS,
        };
        pressed(keypad) || self.held.iter().any(|(&c, &r)| r == role && pressed(c))
    }
}

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
    /// The key typing `+` / `=` ([`LayoutKeys`]) or keypad +.
    pub plus: bool,
    /// The key typing `-` / `_` or keypad −.
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

/// The flight input from the keys and the gamepads: ↑ / ↓ and the left
/// stick's y the throttle (hover: forward), ← / → and its x the bank
/// (hover: strafe), + / − and the triggers the climb.
pub fn flight_input(k: &FlightKeys, pads: &[PadFlight]) -> FlightInput {
    let sum = |f: &dyn Fn(&PadFlight) -> f64| pads.iter().map(f).sum::<f64>();
    let clamp = |v: f64| v.clamp(-1.0, 1.0);
    FlightInput {
        forward: clamp(axis(k.up, k.down) + sum(&|p| dead(p.left_y))),
        side: clamp(axis(k.right, k.left) + sum(&|p| dead(p.left_x))),
        climb: clamp(
            axis(k.plus, k.minus)
                + sum(&|p| dead(p.right_trigger).max(0.0) - dead(p.left_trigger).max(0.0)),
        ),
        boost: k.shift || pads.iter().any(|p| p.south),
        precise: k.ctrl,
        set_throttle: None,
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
        assert_eq!(k(&|k| k.left = true).side, -1.0);
        assert_eq!(k(&|k| k.right = true).side, 1.0);
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
        // Left stick: y the throttle (↓: slower), x the bank (right).
        assert_eq!((i.forward, i.side), (-1.0, 0.5));
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

    fn ch(s: &str) -> Key {
        Key::Character(s.into())
    }

    /// Bug 2026-11-13: on a German keyboard the main keyboard's + / − did
    /// nothing (they were matched by their US positions, `=` and `-`).
    #[test]
    fn plus_and_minus_by_what_the_key_types_us_and_german() {
        let pressed_set = |codes: &[KeyCode]| {
            let codes = codes.to_vec();
            move |c: KeyCode| codes.contains(&c)
        };
        // German (QWERTZ): `+` is the key at the US `]`, `-` at the US `/`.
        let mut k = LayoutKeys::default();
        k.on_key(KeyCode::BracketRight, &ch("+"), true);
        let held = pressed_set(&[KeyCode::BracketRight]);
        assert!(k.held(KeyRole::Plus, &held));
        assert!(!k.held(KeyRole::Minus, &held));
        k.on_key(KeyCode::BracketRight, &ch("+"), false);
        assert!(!k.held(KeyRole::Plus, &pressed_set(&[])));
        k.on_key(KeyCode::Slash, &ch("-"), true);
        assert!(k.held(KeyRole::Minus, &pressed_set(&[KeyCode::Slash])));
        k.on_key(KeyCode::Slash, &ch("-"), false);
        // Shift (boost) + the German `+` types `*`: still climbs (learnt).
        k.on_key(KeyCode::BracketRight, &ch("*"), true);
        assert!(k.held(KeyRole::Plus, &held));
        // … and Shift + `-` types `_`.
        k.on_key(KeyCode::Slash, &ch("_"), true);
        assert!(k.held(KeyRole::Minus, &pressed_set(&[KeyCode::Slash])));
        // The German `ß` / `?` key (the US `-` position) does nothing.
        let mut k = LayoutKeys::default();
        k.on_key(KeyCode::Minus, &ch("ß"), true);
        let at_us_minus = pressed_set(&[KeyCode::Minus]);
        assert!(!k.held(KeyRole::Minus, &at_us_minus) && !k.held(KeyRole::Plus, &at_us_minus));
        // The German `´` key (the US `=` position) is a dead key: nothing.
        k.on_key(KeyCode::Equal, &Key::Dead(Some('´')), true);
        assert!(!k.held(KeyRole::Plus, &pressed_set(&[KeyCode::Equal])));

        // US: `=` (+ without Shift) and Shift + `=` climb, `-` descends,
        // `/` and `]` do nothing.
        let mut k = LayoutKeys::default();
        k.on_key(KeyCode::Equal, &ch("="), true);
        assert!(k.held(KeyRole::Plus, &pressed_set(&[KeyCode::Equal])));
        k.on_key(KeyCode::Equal, &ch("="), false);
        k.on_key(KeyCode::Equal, &ch("+"), true);
        assert!(k.held(KeyRole::Plus, &pressed_set(&[KeyCode::Equal])));
        k.on_key(KeyCode::Minus, &ch("-"), true);
        assert!(k.held(KeyRole::Minus, &pressed_set(&[KeyCode::Minus])));
        for (code, s) in [(KeyCode::Slash, "/"), (KeyCode::BracketRight, "]")] {
            let mut k = LayoutKeys::default();
            k.on_key(code, &ch(s), true);
            let p = pressed_set(&[code]);
            assert!(
                !k.held(KeyRole::Plus, &p) && !k.held(KeyRole::Minus, &p),
                "{s}"
            );
        }
        // The keypad on every layout, without any event learnt.
        let k = LayoutKeys::default();
        assert!(k.held(KeyRole::Plus, &pressed_set(&[KeyCode::NumpadAdd])));
        assert!(k.held(KeyRole::Minus, &pressed_set(&[KeyCode::NumpadSubtract])));
        // A release lost to a focus change does not keep climbing.
        let mut k = LayoutKeys::default();
        k.on_key(KeyCode::BracketRight, &ch("+"), true);
        assert!(!k.held(KeyRole::Plus, &pressed_set(&[])));
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
