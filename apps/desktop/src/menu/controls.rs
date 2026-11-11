//! Keys and gamepad buttons → menu [`Action`]s (spec §3.2.1).
//!
//! | | Keyboard / mouse | Gamepad |
//! |---|---|---|
//! | Open / close | Esc (closes the console first) | Start / Menu |
//! | Move focus | ↑ ↓, Tab / Shift+Tab, hover | D-pad, left stick (with repeat) |
//! | Confirm | Enter, Space, click | South (A / ✕) |
//! | Back (closes at the top) | Esc, Backspace, right click | East (B / ○) |
//! | Change a value | ← / → | D-pad left / right |
//! | Switch tab (Settings, Workshop) | Q / E, PgUp / PgDn | LB / RB |
//!
//! Esc never quits: at the top page Back only closes the menu.

use bevy::prelude::*;

use super::model::Action;

/// Held direction before the first repeat (s).
pub const REPEAT_DELAY_S: f64 = 0.4;
/// Time between repeats while held (s).
pub const REPEAT_INTERVAL_S: f64 = 0.13;
/// Left stick deflection that counts as up / down.
pub const STICK_NAV: f32 = 0.5;

/// The menu buttons pressed this frame (keyboard and gamepads combined).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Pressed {
    /// Esc: opens the menu, or goes back.
    pub escape: bool,
    /// Start / Menu: opens or closes the menu.
    pub start: bool,
    /// Backspace, East.
    pub back: bool,
    /// Enter, Space, South.
    pub confirm: bool,
    pub left: bool,
    pub right: bool,
    /// Tab switch: −1 previous, +1 next, 0 none.
    pub tab: i32,
    /// Focus steps from presses that do not repeat (Tab / Shift+Tab).
    pub step: i32,
    /// Up / down held this frame (−1 up, +1 down, 0 none): repeats.
    pub held: i32,
    /// Up / down pressed this frame (catches a tap shorter than a frame).
    pub tapped: i32,
}

impl Pressed {
    pub fn merge(self, o: Self) -> Self {
        Self {
            escape: self.escape || o.escape,
            start: self.start || o.start,
            back: self.back || o.back,
            confirm: self.confirm || o.confirm,
            left: self.left || o.left,
            right: self.right || o.right,
            tab: (self.tab + o.tab).signum(),
            step: self.step + o.step,
            held: (self.held + o.held).signum(),
            tapped: (self.tapped + o.tapped).signum(),
        }
    }
}

fn dir(up: bool, down: bool) -> i32 {
    i32::from(down) - i32::from(up)
}

/// The keyboard's menu keys.
pub fn keyboard(keys: &ButtonInput<KeyCode>) -> Pressed {
    let any = |k: &[KeyCode]| keys.any_just_pressed(k.iter().copied());
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let tab = keys.just_pressed(KeyCode::Tab);
    Pressed {
        escape: keys.just_pressed(KeyCode::Escape),
        start: false,
        back: keys.just_pressed(KeyCode::Backspace),
        confirm: any(&[KeyCode::Enter, KeyCode::NumpadEnter, KeyCode::Space]),
        left: keys.just_pressed(KeyCode::ArrowLeft),
        right: keys.just_pressed(KeyCode::ArrowRight),
        tab: dir(
            any(&[KeyCode::KeyQ, KeyCode::PageUp]),
            any(&[KeyCode::KeyE, KeyCode::PageDown]),
        ),
        step: match (tab, shift) {
            (false, _) => 0,
            (true, false) => 1,
            (true, true) => -1,
        },
        held: dir(
            keys.pressed(KeyCode::ArrowUp),
            keys.pressed(KeyCode::ArrowDown),
        ),
        tapped: dir(
            keys.just_pressed(KeyCode::ArrowUp),
            keys.just_pressed(KeyCode::ArrowDown),
        ),
    }
}

/// One gamepad's state, as far as the menu needs it.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PadState {
    pub start: bool,
    pub south: bool,
    pub east: bool,
    pub dpad_up: bool,
    pub dpad_down: bool,
    pub dpad_up_pressed: bool,
    pub dpad_down_pressed: bool,
    pub dpad_left: bool,
    pub dpad_right: bool,
    /// LB / RB (L1 / R1) pressed this frame.
    pub lb: bool,
    pub rb: bool,
    pub stick_y: f32,
}

impl PadState {
    pub fn of(p: &Gamepad) -> Self {
        Self {
            start: p.just_pressed(GamepadButton::Start),
            south: p.just_pressed(GamepadButton::South),
            east: p.just_pressed(GamepadButton::East),
            dpad_up: p.pressed(GamepadButton::DPadUp),
            dpad_down: p.pressed(GamepadButton::DPadDown),
            dpad_up_pressed: p.just_pressed(GamepadButton::DPadUp),
            dpad_down_pressed: p.just_pressed(GamepadButton::DPadDown),
            dpad_left: p.just_pressed(GamepadButton::DPadLeft),
            dpad_right: p.just_pressed(GamepadButton::DPadRight),
            lb: p.just_pressed(GamepadButton::LeftTrigger),
            rb: p.just_pressed(GamepadButton::RightTrigger),
            stick_y: p.get(GamepadAxis::LeftStickY).unwrap_or(0.0),
        }
    }
}

/// A gamepad's menu buttons: Start, South = confirm, East = back, D-pad and
/// the left stick (up is positive) move the focus.
pub fn gamepad(p: PadState) -> Pressed {
    let stick = if p.stick_y > STICK_NAV {
        -1
    } else if p.stick_y < -STICK_NAV {
        1
    } else {
        0
    };
    Pressed {
        escape: false,
        start: p.start,
        back: p.east,
        confirm: p.south,
        left: p.dpad_left,
        right: p.dpad_right,
        tab: dir(p.lb, p.rb),
        step: 0,
        held: (dir(p.dpad_up, p.dpad_down) + stick).signum(),
        tapped: dir(p.dpad_up_pressed, p.dpad_down_pressed),
    }
}

/// Repeat for a held up / down: one step at once, then after
/// [`REPEAT_DELAY_S`] one every [`REPEAT_INTERVAL_S`].
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct NavRepeat {
    dir: i32,
    wait_s: f64,
}

impl NavRepeat {
    /// The focus step for this frame (−1, 0, +1).
    pub fn update(&mut self, held: i32, tapped: i32, dt: f64) -> i32 {
        if held == 0 {
            self.dir = 0;
            return tapped;
        }
        if held != self.dir {
            self.dir = held;
            self.wait_s = REPEAT_DELAY_S;
            return held;
        }
        self.wait_s -= dt;
        if self.wait_s <= 0.0 {
            self.wait_s += REPEAT_INTERVAL_S;
            return held;
        }
        0
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// The actions for this frame's presses. `step` is the focus step from
/// [`NavRepeat`] and Tab.
pub fn actions(p: &Pressed, step: i32, menu_open: bool) -> Vec<Action> {
    if !menu_open {
        return if p.escape || p.start {
            vec![Action::Open]
        } else {
            Vec::new()
        };
    }
    let mut out = Vec::new();
    for _ in 0..step.unsigned_abs() {
        out.push(if step < 0 { Action::Up } else { Action::Down });
    }
    if p.left {
        out.push(Action::Left);
    }
    if p.right {
        out.push(Action::Right);
    }
    match p.tab {
        -1 => out.push(Action::TabPrev),
        1 => out.push(Action::TabNext),
        _ => {}
    }
    if p.confirm {
        out.push(Action::Confirm);
    } else if p.escape || p.back {
        out.push(Action::Back);
    }
    if p.start {
        out.push(Action::Close);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(pressed: &[KeyCode]) -> ButtonInput<KeyCode> {
        let mut k = ButtonInput::default();
        for &key in pressed {
            k.press(key);
        }
        k
    }

    #[test]
    fn keys_map_to_menu_actions() {
        use Action::*;
        let k = |pressed: &[KeyCode], open| {
            let p = keyboard(&keys(pressed));
            actions(&p, p.tapped + p.step, open)
        };
        // Closed: Esc opens, nothing else does anything (ride keys).
        assert_eq!(k(&[KeyCode::Escape], false), [Open]);
        assert!(k(&[KeyCode::Enter, KeyCode::ArrowUp, KeyCode::KeyA], false).is_empty());
        // Open.
        assert_eq!(k(&[KeyCode::ArrowDown], true), [Down]);
        assert_eq!(k(&[KeyCode::ArrowUp], true), [Up]);
        assert_eq!(k(&[KeyCode::Tab], true), [Down]);
        assert_eq!(k(&[KeyCode::ShiftLeft, KeyCode::Tab], true), [Up]);
        assert_eq!(k(&[KeyCode::Enter], true), [Confirm]);
        assert_eq!(k(&[KeyCode::NumpadEnter], true), [Confirm]);
        assert_eq!(k(&[KeyCode::Space], true), [Confirm]);
        assert_eq!(k(&[KeyCode::Escape], true), [Back]);
        assert_eq!(k(&[KeyCode::Backspace], true), [Back]);
        assert_eq!(k(&[KeyCode::ArrowLeft], true), [Left]);
        assert_eq!(k(&[KeyCode::ArrowRight], true), [Right]);
        assert_eq!(k(&[KeyCode::KeyQ], true), [TabPrev]);
        assert_eq!(k(&[KeyCode::PageDown], true), [TabNext]);
        assert!(
            k(&[KeyCode::KeyE], false).is_empty(),
            "a ride key when closed"
        );
        // Ride keys do nothing in the menu.
        for key in [KeyCode::KeyA, KeyCode::KeyR, KeyCode::KeyV, KeyCode::KeyU] {
            assert!(k(&[key], true).is_empty(), "{key:?}");
        }
    }

    #[test]
    fn gamepad_buttons_map_to_menu_actions() {
        use Action::*;
        let g = |p: PadState, open| {
            let p = gamepad(p);
            actions(&p, p.held, open)
        };
        let start = PadState {
            start: true,
            ..default()
        };
        assert_eq!(g(start, false), [Open]);
        assert_eq!(g(start, true), [Close]);
        let south = PadState {
            south: true,
            ..default()
        };
        // South (autodrive in the ride) does not open the menu.
        assert!(g(south, false).is_empty());
        assert_eq!(g(south, true), [Confirm]);
        let east = PadState {
            east: true,
            ..default()
        };
        assert!(g(east, false).is_empty());
        assert_eq!(g(east, true), [Back]);
        let up = PadState {
            dpad_up: true,
            ..default()
        };
        assert_eq!(g(up, true), [Up]);
        let stick_down = PadState {
            stick_y: -0.9,
            ..default()
        };
        assert_eq!(g(stick_down, true), [Down]);
        // Small stick deflections do not move the focus.
        let drift = PadState {
            stick_y: 0.3,
            ..default()
        };
        assert!(g(drift, true).is_empty());
        let left = PadState {
            dpad_left: true,
            ..default()
        };
        assert_eq!(g(left, true), [Left]);
        let rb = PadState {
            rb: true,
            ..default()
        };
        assert_eq!(g(rb, true), [TabNext]);
        let lb = PadState {
            lb: true,
            ..default()
        };
        assert_eq!(g(lb, true), [TabPrev]);
    }

    #[test]
    fn held_direction_repeats_after_a_delay() {
        let frame = 1.0 / 60.0;
        let mut r = NavRepeat::default();
        let mut steps = 0;
        // Held for 0.9 s: one at once, then repeats at 0.4, 0.53, 0.67, 0.8 s.
        for _ in 0..54 {
            steps += r.update(1, 0, frame);
        }
        assert_eq!(steps, 1 + 4);
        // Released: nothing; a short tap still counts once.
        assert_eq!(r.update(0, 0, frame), 0);
        assert_eq!(r.update(0, -1, frame), -1);
        // The other direction starts at once.
        assert_eq!(r.update(1, 0, frame), 1);
        assert_eq!(r.update(-1, 0, frame), -1);
        assert_eq!(r.update(-1, 0, frame), 0);
    }

    #[test]
    fn confirm_wins_over_back_in_the_same_frame() {
        let p = Pressed {
            confirm: true,
            back: true,
            ..default()
        };
        assert_eq!(actions(&p, 0, true), [Action::Confirm]);
    }
}
