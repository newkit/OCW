//! The VR controllers (spec §14 Phase 11, 11B *Input in VR*, X1 v4.52):
//! the Oculus Touch controllers of the Quest 1 as **one more gamepad**.
//!
//! In VR the hands are on the bars and riding stays on the trainer, the
//! keys and the gamepad (Q4, Q7); the world-space menus with a controller
//! ray come with X3. Until then the Touch controllers drive exactly what a
//! gamepad drives – the ride (throttle on the triggers, steering and the
//! junction exits on the left stick, the mode buttons) and the in-game
//! menu (Start, the stick as the D-pad, A / B) – plus the VR recentre:
//!
//! | Touch | Gamepad (Bevy) | Riding (default bindings) | Menu |
//! |---|---|---|---|
//! | ☰ menu (left) | Start | open the menu | close |
//! | A | South | autodrive | confirm |
//! | B | East | U-turn | back |
//! | X | West | wild mode | – |
//! | Y | North | back to the route | – |
//! | left stick | left stick + D-pad flicks | steer, exits (left / right), faster / stop (up / down) | move |
//! | right stick | right stick | (unicorn mode: look) | – |
//! | left / right trigger | LT2 / RT2 (analog) | stop / faster | – |
//! | left / right grip | LB / RB | – | previous / next tab |
//! | left / right stick click | L3 / R3 | Strava recording / mute the voices | – |
//! | **both grips held 0.6 s** | – | **recentre the view** | – |
//!
//! The binding tables ([`BINDINGS`]) are what the OpenXR layer suggests to
//! the runtime (`xrSuggestInteractionProfileBindings`) for the Touch
//! profile and the Khronos simple controller; [`PadMapper`] turns one
//! frame's [`ControllerState`] (after `xrSyncActions`) into gamepad values.
//! Pure: tested here without a headset.

use std::fmt::Write as _;

use crate::session::XrError;

/// Where the controllers come from: the OpenXR layer (`xrSyncActions` and
/// the action states, every frame) or the mock.
pub trait ControllerSource {
    /// This frame's state; `None` when the runtime has no controllers for
    /// us (no action set attached).
    fn sync_controllers(&mut self) -> Result<Option<ControllerState>, XrError>;
}

/// The Oculus Touch interaction profile (Quest 1 / 2 / Rift S over Link).
pub const TOUCH_PROFILE: &str = "/interaction_profiles/oculus/touch_controller";
/// The Khronos simple controller: every runtime knows it (a fallback).
pub const SIMPLE_PROFILE: &str = "/interaction_profiles/khr/simple_controller";

/// The action set's name (OpenXR names: lower-case, digits, `-_.`).
pub const ACTION_SET: &str = "ride";
pub const ACTION_SET_LOCALIZED: &str = "Riding";

/// What an input reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    /// A button (`…/click`): `XrAction` of type boolean.
    Button,
    /// A trigger or grip (`…/value`): float, 0 … 1.
    Value,
    /// A thumbstick: a 2D vector, −1 … 1 each.
    Stick,
}

/// One input of the controllers (one OpenXR action each).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Input {
    Menu,
    A,
    B,
    X,
    Y,
    LeftTrigger,
    RightTrigger,
    LeftGrip,
    RightGrip,
    LeftStick,
    RightStick,
    LeftStickClick,
    RightStickClick,
}

impl Input {
    pub const ALL: [Input; 13] = [
        Input::Menu,
        Input::A,
        Input::B,
        Input::X,
        Input::Y,
        Input::LeftTrigger,
        Input::RightTrigger,
        Input::LeftGrip,
        Input::RightGrip,
        Input::LeftStick,
        Input::RightStick,
        Input::LeftStickClick,
        Input::RightStickClick,
    ];
    pub const COUNT: usize = Self::ALL.len();

    fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|i| *i == self)
            .expect("every input is listed")
    }

    /// The action's name (OpenXR rules).
    pub fn name(self) -> &'static str {
        match self {
            Input::Menu => "menu",
            Input::A => "button_a",
            Input::B => "button_b",
            Input::X => "button_x",
            Input::Y => "button_y",
            Input::LeftTrigger => "trigger_left",
            Input::RightTrigger => "trigger_right",
            Input::LeftGrip => "grip_left",
            Input::RightGrip => "grip_right",
            Input::LeftStick => "stick_left",
            Input::RightStick => "stick_right",
            Input::LeftStickClick => "stick_left_click",
            Input::RightStickClick => "stick_right_click",
        }
    }

    /// The name the runtime shows in its binding editor.
    pub fn localized(self) -> &'static str {
        match self {
            Input::Menu => "Menu",
            Input::A => "A (confirm, autodrive)",
            Input::B => "B (back, U-turn)",
            Input::X => "X (wild mode)",
            Input::Y => "Y (back to the route)",
            Input::LeftTrigger => "Left trigger (stop)",
            Input::RightTrigger => "Right trigger (faster)",
            Input::LeftGrip => "Left grip",
            Input::RightGrip => "Right grip",
            Input::LeftStick => "Left stick (steer, exits, menu)",
            Input::RightStick => "Right stick",
            Input::LeftStickClick => "Left stick click",
            Input::RightStickClick => "Right stick click",
        }
    }

    pub fn kind(self) -> InputKind {
        match self {
            Input::LeftTrigger | Input::RightTrigger | Input::LeftGrip | Input::RightGrip => {
                InputKind::Value
            }
            Input::LeftStick | Input::RightStick => InputKind::Stick,
            _ => InputKind::Button,
        }
    }
}

/// A suggested binding: `input` on `path` of `profile`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Binding {
    pub profile: &'static str,
    pub input: Input,
    pub path: &'static str,
}

const fn touch(input: Input, path: &'static str) -> Binding {
    Binding {
        profile: TOUCH_PROFILE,
        input,
        path,
    }
}

const fn simple(input: Input, path: &'static str) -> Binding {
    Binding {
        profile: SIMPLE_PROFILE,
        input,
        path,
    }
}

/// Every suggested binding (OpenXR 1.0 §6.4: the Touch profile's
/// component paths; the simple controller has only select and menu).
pub const BINDINGS: &[Binding] = &[
    touch(Input::Menu, "/user/hand/left/input/menu/click"),
    touch(Input::A, "/user/hand/right/input/a/click"),
    touch(Input::B, "/user/hand/right/input/b/click"),
    touch(Input::X, "/user/hand/left/input/x/click"),
    touch(Input::Y, "/user/hand/left/input/y/click"),
    touch(Input::LeftTrigger, "/user/hand/left/input/trigger/value"),
    touch(Input::RightTrigger, "/user/hand/right/input/trigger/value"),
    touch(Input::LeftGrip, "/user/hand/left/input/squeeze/value"),
    touch(Input::RightGrip, "/user/hand/right/input/squeeze/value"),
    touch(Input::LeftStick, "/user/hand/left/input/thumbstick"),
    touch(Input::RightStick, "/user/hand/right/input/thumbstick"),
    touch(
        Input::LeftStickClick,
        "/user/hand/left/input/thumbstick/click",
    ),
    touch(
        Input::RightStickClick,
        "/user/hand/right/input/thumbstick/click",
    ),
    simple(Input::Menu, "/user/hand/left/input/menu/click"),
    simple(Input::A, "/user/hand/right/input/select/click"),
    simple(Input::B, "/user/hand/right/input/menu/click"),
];

/// The profiles [`BINDINGS`] suggests, in order.
pub fn profiles() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for b in BINDINGS {
        if !out.contains(&b.profile) {
            out.push(b.profile);
        }
    }
    out
}

/// The bindings of one profile.
pub fn bindings_of(profile: &str) -> impl Iterator<Item = &'static Binding> + '_ {
    BINDINGS.iter().filter(move |b| b.profile == profile)
}

/// One input's value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InputValue {
    Button(bool),
    Value(f32),
    Stick([f32; 2]),
}

impl InputValue {
    fn rest(kind: InputKind) -> Self {
        match kind {
            InputKind::Button => Self::Button(false),
            InputKind::Value => Self::Value(0.0),
            InputKind::Stick => Self::Stick([0.0; 2]),
        }
    }
}

/// The controllers after one `xrSyncActions`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ControllerState {
    values: [InputValue; Input::COUNT],
    /// Some action is bound and active (a controller is on and the session
    /// has the input focus).
    pub active: bool,
}

impl Default for ControllerState {
    fn default() -> Self {
        Self {
            values: Input::ALL.map(|i| InputValue::rest(i.kind())),
            active: false,
        }
    }
}

impl ControllerState {
    /// Every input at rest; `active` as given.
    pub fn at_rest(active: bool) -> Self {
        Self {
            active,
            ..Default::default()
        }
    }

    pub fn get(&self, i: Input) -> InputValue {
        self.values[i.index()]
    }

    /// Sets an input (a value of another kind is ignored).
    pub fn set(&mut self, i: Input, v: InputValue) {
        let ok = matches!(
            (i.kind(), v),
            (InputKind::Button, InputValue::Button(_))
                | (InputKind::Value, InputValue::Value(_))
                | (InputKind::Stick, InputValue::Stick(_))
        );
        if ok {
            self.values[i.index()] = v;
        }
    }

    pub fn button(&self, i: Input) -> bool {
        matches!(self.get(i), InputValue::Button(true))
    }

    pub fn value(&self, i: Input) -> f32 {
        match self.get(i) {
            InputValue::Value(v) => v,
            InputValue::Button(b) => f32::from(u8::from(b)),
            InputValue::Stick(s) => s[0].hypot(s[1]),
        }
    }

    pub fn stick(&self, i: Input) -> [f32; 2] {
        match self.get(i) {
            InputValue::Stick(s) => s,
            _ => [0.0; 2],
        }
    }

    /// What changed since `before`, for the log (`VR controllers: A down,
    /// trigger_right 0.75, stick_left (0.0, 1.0)`); analog values in steps
    /// of 0.25, so a resting hand writes nothing. Empty when nothing did.
    pub fn changes(&self, before: &ControllerState) -> String {
        let step = |v: f32| (v * 4.0).round() / 4.0;
        let mut out = String::new();
        let mut add = |s: String| {
            if !out.is_empty() {
                out.push_str(", ");
            }
            out.push_str(&s);
        };
        if self.active != before.active {
            add(if self.active {
                "active".into()
            } else {
                "inactive".into()
            });
        }
        for i in Input::ALL {
            match (self.get(i), before.get(i)) {
                (InputValue::Button(a), InputValue::Button(b)) if a != b => {
                    add(format!("{} {}", i.name(), if a { "down" } else { "up" }))
                }
                (InputValue::Value(a), InputValue::Value(b)) if step(a) != step(b) => {
                    add(format!("{} {:.2}", i.name(), step(a)))
                }
                (InputValue::Stick(a), InputValue::Stick(b))
                    if step(a[0]) != step(b[0]) || step(a[1]) != step(b[1]) =>
                {
                    let mut s = String::new();
                    let _ = write!(s, "{} ({:.2}, {:.2})", i.name(), step(a[0]), step(a[1]));
                    add(s)
                }
                _ => {}
            }
        }
        out
    }
}

/// A gamepad button (Bevy's `GamepadButton` names; the renderer maps them).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PadButton {
    South,
    East,
    West,
    North,
    Start,
    LeftThumb,
    RightThumb,
    /// LB / RB (Bevy: `LeftTrigger` / `RightTrigger`).
    LeftBumper,
    RightBumper,
    /// LT2 / RT2, analog (Bevy: `LeftTrigger2` / `RightTrigger2`).
    LeftTrigger2,
    RightTrigger2,
    DPadUp,
    DPadDown,
    DPadLeft,
    DPadRight,
}

/// A gamepad axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PadAxis {
    LeftStickX,
    LeftStickY,
    RightStickX,
    RightStickY,
}

/// The fixed map of controller buttons and analog inputs to gamepad
/// buttons (the sticks go to the axes and the D-pad, see [`PadMapper`]).
pub const PAD_BUTTONS: &[(Input, PadButton)] = &[
    (Input::Menu, PadButton::Start),
    (Input::A, PadButton::South),
    (Input::B, PadButton::East),
    (Input::X, PadButton::West),
    (Input::Y, PadButton::North),
    (Input::LeftStickClick, PadButton::LeftThumb),
    (Input::RightStickClick, PadButton::RightThumb),
    (Input::LeftGrip, PadButton::LeftBumper),
    (Input::RightGrip, PadButton::RightBumper),
    (Input::LeftTrigger, PadButton::LeftTrigger2),
    (Input::RightTrigger, PadButton::RightTrigger2),
];

/// A stick is flicked (a D-pad press) beyond this …
pub const FLICK_ON: f32 = 0.7;
/// … and released below this (hysteresis: no chatter at the edge).
pub const FLICK_OFF: f32 = 0.4;
/// Both grips beyond this …
pub const GRIP_HELD: f32 = 0.8;
/// … for this long recentre the view (once per squeeze).
pub const RECENTRE_HOLD_S: f32 = 0.6;

/// One frame's gamepad: every button's value (0 … 1), every axis.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PadFrame {
    pub buttons: Vec<(PadButton, f32)>,
    pub axes: Vec<(PadAxis, f32)>,
    /// The recentre gesture completed this frame.
    pub recentre: bool,
}

impl PadFrame {
    pub fn button(&self, b: PadButton) -> f32 {
        self.buttons
            .iter()
            .find(|(x, _)| *x == b)
            .map_or(0.0, |(_, v)| *v)
    }

    pub fn axis(&self, a: PadAxis) -> f32 {
        self.axes
            .iter()
            .find(|(x, _)| *x == a)
            .map_or(0.0, |(_, v)| *v)
    }
}

/// Turns controller states into gamepad frames: the left stick's flicks
/// as D-pad presses (with hysteresis) and the recentre gesture.
#[derive(Debug, Clone, Default)]
pub struct PadMapper {
    /// D-pad up, down, left, right held by the stick.
    flick: [bool; 4],
    grips_s: f32,
    recentred: bool,
}

impl PadMapper {
    pub fn map(&mut self, s: &ControllerState, dt_s: f32) -> PadFrame {
        // Inactive (no focus, controllers off): everything at rest.
        let rest = ControllerState::default();
        let s = if s.active { s } else { &rest };
        let mut buttons: Vec<(PadButton, f32)> = PAD_BUTTONS
            .iter()
            .map(|(i, b)| (*b, s.value(*i).clamp(0.0, 1.0)))
            .collect();
        let [lx, ly] = s.stick(Input::LeftStick);
        let [rx, ry] = s.stick(Input::RightStick);
        let dirs = [ly, -ly, -lx, lx];
        for (held, v) in self.flick.iter_mut().zip(dirs) {
            *held = if *held { v > FLICK_OFF } else { v > FLICK_ON };
        }
        for (b, held) in [
            PadButton::DPadUp,
            PadButton::DPadDown,
            PadButton::DPadLeft,
            PadButton::DPadRight,
        ]
        .into_iter()
        .zip(self.flick)
        {
            buttons.push((b, f32::from(u8::from(held))));
        }
        let grips = s.value(Input::LeftGrip) > GRIP_HELD && s.value(Input::RightGrip) > GRIP_HELD;
        let mut recentre = false;
        if grips {
            self.grips_s += dt_s.max(0.0);
            if self.grips_s >= RECENTRE_HOLD_S && !self.recentred {
                self.recentred = true;
                recentre = true;
            }
        } else {
            self.grips_s = 0.0;
            self.recentred = false;
        }
        PadFrame {
            buttons,
            axes: vec![
                (PadAxis::LeftStickX, lx),
                (PadAxis::LeftStickY, ly),
                (PadAxis::RightStickX, rx),
                (PadAxis::RightStickY, ry),
            ],
            recentre,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The component paths of the Oculus Touch profile (OpenXR 1.0,
    /// §6.4 *Interaction Profile Paths*), per hand.
    const TOUCH_LEFT: &[&str] = &[
        "x/click",
        "x/touch",
        "y/click",
        "y/touch",
        "menu/click",
        "squeeze/value",
        "trigger/value",
        "trigger/touch",
        "thumbstick",
        "thumbstick/x",
        "thumbstick/y",
        "thumbstick/click",
        "thumbstick/touch",
        "thumbrest/touch",
    ];
    const TOUCH_RIGHT: &[&str] = &[
        "a/click",
        "a/touch",
        "b/click",
        "b/touch",
        "system/click",
        "squeeze/value",
        "trigger/value",
        "trigger/touch",
        "thumbstick",
        "thumbstick/x",
        "thumbstick/y",
        "thumbstick/click",
        "thumbstick/touch",
        "thumbrest/touch",
    ];
    const SIMPLE_EACH: &[&str] = &["select/click", "menu/click"];

    fn component(path: &str) -> (&str, &str) {
        let rest = path.strip_prefix("/user/hand/").expect("a hand path");
        let (hand, rest) = rest.split_once('/').expect("hand/…");
        (hand, rest.strip_prefix("input/").expect("an input"))
    }

    /// Every suggested path exists in its profile, with the kind the
    /// action has (OpenXR rejects the whole suggestion otherwise:
    /// `XR_ERROR_PATH_UNSUPPORTED`).
    #[test]
    fn every_binding_is_a_real_component_of_its_profile() {
        for b in BINDINGS {
            let (hand, comp) = component(b.path);
            let list = match (b.profile, hand) {
                (TOUCH_PROFILE, "left") => TOUCH_LEFT,
                (TOUCH_PROFILE, "right") => TOUCH_RIGHT,
                (SIMPLE_PROFILE, "left" | "right") => SIMPLE_EACH,
                other => panic!("unknown {other:?}"),
            };
            assert!(list.contains(&comp), "{} not in {}", b.path, b.profile);
            let ok = match b.input.kind() {
                InputKind::Button => comp.ends_with("/click"),
                InputKind::Value => comp.ends_with("/value"),
                InputKind::Stick => comp == "thumbstick",
            };
            assert!(ok, "{:?} on {}", b.input, b.path);
            // The system button is the runtime's (Meta's menu): never ours.
            assert_ne!(comp, "system/click");
        }
    }

    /// The Touch profile binds every input exactly once; each profile binds
    /// a path once.
    #[test]
    fn the_touch_profile_binds_every_input_once() {
        for i in Input::ALL {
            let n = bindings_of(TOUCH_PROFILE).filter(|b| b.input == i).count();
            assert_eq!(n, 1, "{i:?}");
        }
        for p in profiles() {
            let mut paths: Vec<&str> = bindings_of(p).map(|b| b.path).collect();
            let n = paths.len();
            paths.sort();
            paths.dedup();
            assert_eq!(paths.len(), n, "{p}: a path twice");
        }
        assert_eq!(profiles(), vec![TOUCH_PROFILE, SIMPLE_PROFILE]);
    }

    /// OpenXR's naming rules for actions and action sets.
    #[test]
    fn action_names_follow_openxr_rules() {
        let ok = |s: &str| {
            !s.is_empty()
                && s.len() < 64
                && s.bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"-_.".contains(&c))
        };
        assert!(ok(ACTION_SET));
        let mut names: Vec<&str> = Input::ALL.iter().map(|i| i.name()).collect();
        for n in &names {
            assert!(ok(n), "{n}");
        }
        names.sort();
        names.dedup();
        assert_eq!(names.len(), Input::COUNT, "names are unique");
        assert!(Input::ALL.iter().all(|i| !i.localized().is_empty()));
    }

    /// Every controller button goes to its own gamepad button.
    #[test]
    fn every_button_has_its_own_pad_button() {
        let mut pads: Vec<PadButton> = PAD_BUTTONS.iter().map(|(_, b)| *b).collect();
        pads.sort();
        pads.dedup();
        assert_eq!(pads.len(), PAD_BUTTONS.len());
        for i in Input::ALL {
            let mapped = PAD_BUTTONS.iter().any(|(x, _)| *x == i);
            assert_eq!(mapped, i.kind() != InputKind::Stick, "{i:?}");
        }
        assert!(PAD_BUTTONS.contains(&(Input::Menu, PadButton::Start)));
        assert!(PAD_BUTTONS.contains(&(Input::A, PadButton::South)));
        assert!(PAD_BUTTONS.contains(&(Input::B, PadButton::East)));
    }

    fn state(f: impl FnOnce(&mut ControllerState)) -> ControllerState {
        let mut s = ControllerState::at_rest(true);
        f(&mut s);
        s
    }

    #[test]
    fn buttons_and_triggers_become_pad_values() {
        let mut m = PadMapper::default();
        let s = state(|s| {
            s.set(Input::A, InputValue::Button(true));
            s.set(Input::RightTrigger, InputValue::Value(0.6));
            s.set(Input::LeftStick, InputValue::Stick([0.3, -0.2]));
        });
        let f = m.map(&s, 0.014);
        assert_eq!(f.button(PadButton::South), 1.0);
        assert_eq!(f.button(PadButton::East), 0.0);
        assert!((f.button(PadButton::RightTrigger2) - 0.6).abs() < 1e-6);
        assert!((f.axis(PadAxis::LeftStickX) - 0.3).abs() < 1e-6);
        assert!((f.axis(PadAxis::LeftStickY) + 0.2).abs() < 1e-6);
        assert!(!f.recentre);
        // A value of the wrong kind is ignored.
        let mut s2 = s;
        s2.set(Input::A, InputValue::Value(0.5));
        assert_eq!(s2.get(Input::A), InputValue::Button(true));
    }

    /// Without focus (the runtime's menu over us) the pad rests.
    #[test]
    fn an_inactive_state_rests() {
        let mut m = PadMapper::default();
        let mut s = state(|s| s.set(Input::A, InputValue::Button(true)));
        s.active = false;
        let f = m.map(&s, 0.014);
        assert!(f.buttons.iter().all(|(_, v)| *v == 0.0));
        assert!(f.axes.iter().all(|(_, v)| *v == 0.0));
    }

    /// Stick flicks are D-pad presses with hysteresis.
    #[test]
    fn the_left_stick_flicks_the_dpad() {
        let mut m = PadMapper::default();
        let up = |y: f32| state(|s| s.set(Input::LeftStick, InputValue::Stick([0.0, y])));
        assert_eq!(m.map(&up(0.6), 0.01).button(PadButton::DPadUp), 0.0);
        assert_eq!(m.map(&up(0.8), 0.01).button(PadButton::DPadUp), 1.0);
        // Held until below the release level.
        assert_eq!(m.map(&up(0.5), 0.01).button(PadButton::DPadUp), 1.0);
        assert_eq!(m.map(&up(0.3), 0.01).button(PadButton::DPadUp), 0.0);
        let f = m.map(&up(-0.9), 0.01);
        assert_eq!(f.button(PadButton::DPadDown), 1.0);
        assert_eq!(f.button(PadButton::DPadUp), 0.0);
        let left = state(|s| s.set(Input::LeftStick, InputValue::Stick([-0.95, 0.0])));
        let f = m.map(&left, 0.01);
        assert_eq!(f.button(PadButton::DPadLeft), 1.0);
        assert_eq!(f.button(PadButton::DPadRight), 0.0);
    }

    /// Both grips held 0.6 s recentre once per squeeze.
    #[test]
    fn both_grips_held_recentre_once() {
        let mut m = PadMapper::default();
        let grips = |l: f32, r: f32| {
            state(|s| {
                s.set(Input::LeftGrip, InputValue::Value(l));
                s.set(Input::RightGrip, InputValue::Value(r));
            })
        };
        let mut fired = 0;
        for _ in 0..100 {
            fired += usize::from(m.map(&grips(1.0, 0.9), 0.014).recentre);
        }
        assert_eq!(fired, 1, "once while held");
        assert!(!m.map(&grips(1.0, 0.2), 0.014).recentre);
        // One grip alone never recentres.
        let mut fired = 0;
        for _ in 0..100 {
            fired += usize::from(m.map(&grips(1.0, 0.0), 0.014).recentre);
        }
        assert_eq!(fired, 0);
        // A short squeeze does not either.
        let mut m = PadMapper::default();
        let mut fired = 0;
        for _ in 0..30 {
            fired += usize::from(m.map(&grips(1.0, 1.0), 0.014).recentre);
        }
        assert_eq!(fired, 0, "0.42 s is too short");
    }

    #[test]
    fn changes_name_what_moved() {
        let a = state(|_| {});
        let b = state(|s| {
            s.set(Input::A, InputValue::Button(true));
            s.set(Input::RightTrigger, InputValue::Value(0.74));
            s.set(Input::LeftStick, InputValue::Stick([0.0, 0.98]));
        });
        assert_eq!(
            b.changes(&a),
            "button_a down, trigger_right 0.75, stick_left (0.00, 1.00)"
        );
        assert_eq!(b.changes(&b), "");
        // Small analog noise is not a change.
        let mut c = b;
        c.set(Input::RightTrigger, InputValue::Value(0.76));
        assert_eq!(c.changes(&b), "");
        assert_eq!(ControllerState::default().changes(&a), "inactive");
    }
}
