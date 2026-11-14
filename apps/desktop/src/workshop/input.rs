//! The workshop's input (spec §3.2.2, §14 Phase 4, 4A/4B): keyboard,
//! mouse, gamepad and touch mapped onto abstract [`Act`]s, pure. The
//! scene model sees only actions, so it is tested once with actions and
//! once per device.
//!
//! | Action | Keyboard | Mouse | Gamepad | Touch |
//! |---|---|---|---|---|
//! | Move | arrows | hover | D-pad, left stick | – |
//! | Confirm | Enter, Space | left click | South | tap |
//! | Back | Esc, Backspace | right click | East | two-finger tap, ‹ Back button |
//! | Slot ± | Q / E, PgUp / PgDn | – | LB / RB | – |
//! | Compare | C | middle click | North | long press |
//! | Action (buy …) | X | – | West | the card's buttons |
//! | Orbit | I J K L, Shift + arrows | left drag | right stick | one-finger drag |
//! | Zoom | + / − | wheel | RT / LT | pinch |
//! | Reset view | Home | – | R3 | double tap |
//! | Cycle bikes | ← / → | – | D-pad | swipe |

use bevy::input::keyboard::KeyCode;
use bevy::input::{ButtonInput, mouse::MouseButton};
use bevy::math::Vec2;

/// An abstract workshop action.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Act {
    Left,
    Right,
    Up,
    Down,
    Confirm,
    Back,
    SlotPrev,
    SlotNext,
    /// Secondary: pin / unpin a part for comparing.
    Compare,
    /// Tertiary: the focused row's action (buy, equip, sell …).
    Action,
    /// Turn the stand view (rad: yaw, pitch); with a flick's frame time.
    Orbit(Vec2, f32),
    /// Zoom steps (positive = closer).
    Zoom(f32),
    ResetView,
    /// The pointer moved to this window position (logical px).
    Point(Vec2),
    /// Click / tap at this position.
    Activate(Vec2),
    /// A horizontal swipe: −1 left, +1 right (cycles the bikes).
    Swipe(i32),
    /// The pointer is over picker row `i` / clicked it (the panel's UI).
    Row(usize),
    RowClick(usize),
    /// Dialog button `i` was clicked / tapped.
    Button(usize),
    /// The on-screen ‹ Back button (touch mode, spec §3.2.2).
    BackButton,
}

/// The device of the last meaningful input: the hint bar's glyphs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ActiveInput {
    #[default]
    Keyboard,
    Mouse,
    Gamepad,
    Touch,
}

/// Orbit speed of the keys (rad per second held).
pub const KEY_ORBIT_RAD_S: f32 = 1.6;
/// Mouse / touch drag orbit (rad per logical px).
pub const DRAG_RAD_PER_PX: f32 = 0.006;
/// Right-stick orbit at full deflection (rad/s).
pub const STICK_ORBIT_RAD_S: f32 = 2.2;
/// Stick deflection that counts.
pub const DEAD_ZONE: f32 = 0.2;
/// Mouse motion under this does not switch to mouse hints (px).
pub const MOUSE_NUDGE_PX: f32 = 4.0;

/// The keyboard's actions this frame (`dt`: frame time, for held keys).
pub fn keyboard(keys: &ButtonInput<KeyCode>, dt: f32) -> Vec<Act> {
    let tap = |k: KeyCode| keys.just_pressed(k);
    let any = |k: &[KeyCode]| keys.any_just_pressed(k.iter().copied());
    let held = |k: KeyCode| keys.pressed(k);
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let mut out = Vec::new();
    if !shift {
        for (k, a) in [
            (KeyCode::ArrowLeft, Act::Left),
            (KeyCode::ArrowRight, Act::Right),
            (KeyCode::ArrowUp, Act::Up),
            (KeyCode::ArrowDown, Act::Down),
        ] {
            if tap(k) {
                out.push(a);
            }
        }
    }
    if any(&[KeyCode::Enter, KeyCode::NumpadEnter, KeyCode::Space]) {
        out.push(Act::Confirm);
    }
    if any(&[KeyCode::Escape, KeyCode::Backspace]) {
        out.push(Act::Back);
    }
    if any(&[KeyCode::KeyQ, KeyCode::PageUp]) {
        out.push(Act::SlotPrev);
    }
    if any(&[KeyCode::KeyE, KeyCode::PageDown]) {
        out.push(Act::SlotNext);
    }
    if tap(KeyCode::KeyC) {
        out.push(Act::Compare);
    }
    if tap(KeyCode::KeyX) {
        out.push(Act::Action);
    }
    if tap(KeyCode::Home) {
        out.push(Act::ResetView);
    }
    // Orbit: I J K L, or Shift + arrows, while held.
    let axis = |neg: bool, pos: bool| f32::from(u8::from(pos)) - f32::from(u8::from(neg));
    let yaw = axis(
        held(KeyCode::KeyJ) || (shift && held(KeyCode::ArrowLeft)),
        held(KeyCode::KeyL) || (shift && held(KeyCode::ArrowRight)),
    );
    let pitch = axis(
        held(KeyCode::KeyK) || (shift && held(KeyCode::ArrowDown)),
        held(KeyCode::KeyI) || (shift && held(KeyCode::ArrowUp)),
    );
    if yaw != 0.0 || pitch != 0.0 {
        out.push(Act::Orbit(
            Vec2::new(yaw, pitch) * KEY_ORBIT_RAD_S * dt,
            0.0,
        ));
    }
    let zoom = axis(
        held(KeyCode::Minus) || held(KeyCode::NumpadSubtract),
        held(KeyCode::Equal) || held(KeyCode::NumpadAdd),
    );
    if zoom != 0.0 {
        out.push(Act::Zoom(zoom * 4.0 * dt));
    }
    out
}

/// One gamepad's state, as far as the workshop needs it.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Pad {
    pub south: bool,
    pub east: bool,
    pub north: bool,
    pub west: bool,
    pub lb: bool,
    pub rb: bool,
    pub r3: bool,
    pub dpad_left: bool,
    pub dpad_right: bool,
    pub dpad_up: bool,
    pub dpad_down: bool,
    /// Left stick (x right, y up) and right stick.
    pub left: Vec2,
    pub right: Vec2,
    /// Analog triggers (0 … 1).
    pub lt: f32,
    pub rt: f32,
}

impl Pad {
    pub fn of(p: &bevy::input::gamepad::Gamepad) -> Self {
        use bevy::input::gamepad::{GamepadAxis as A, GamepadButton as B};
        let axis = |a| p.get(a).unwrap_or(0.0);
        Self {
            south: p.just_pressed(B::South),
            east: p.just_pressed(B::East),
            north: p.just_pressed(B::North),
            west: p.just_pressed(B::West),
            lb: p.just_pressed(B::LeftTrigger),
            rb: p.just_pressed(B::RightTrigger),
            r3: p.just_pressed(B::RightThumb),
            dpad_left: p.just_pressed(B::DPadLeft),
            dpad_right: p.just_pressed(B::DPadRight),
            dpad_up: p.just_pressed(B::DPadUp),
            dpad_down: p.just_pressed(B::DPadDown),
            left: Vec2::new(axis(A::LeftStickX), axis(A::LeftStickY)),
            right: Vec2::new(axis(A::RightStickX), axis(A::RightStickY)),
            lt: p.get(B::LeftTrigger2).unwrap_or(0.0),
            rt: p.get(B::RightTrigger2).unwrap_or(0.0),
        }
    }

    /// Whether anything is pressed or deflected (the hint bar's device).
    pub fn active(&self) -> bool {
        self.south
            || self.east
            || self.north
            || self.west
            || self.lb
            || self.rb
            || self.r3
            || self.dpad_left
            || self.dpad_right
            || self.dpad_up
            || self.dpad_down
            || self.left.length() > DEAD_ZONE * 2.0
            || self.right.length() > DEAD_ZONE * 2.0
            || self.lt > DEAD_ZONE
            || self.rt > DEAD_ZONE
    }
}

/// The left stick as a direction step, once per push (it must come back
/// to the middle before the next).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct StickLatch {
    pushed: bool,
}

impl StickLatch {
    pub fn update(&mut self, stick: Vec2) -> Option<Act> {
        if stick.length() < 0.35 {
            self.pushed = false;
            return None;
        }
        if self.pushed || stick.length() < 0.6 {
            return None;
        }
        self.pushed = true;
        Some(if stick.x.abs() > stick.y.abs() {
            if stick.x > 0.0 { Act::Right } else { Act::Left }
        } else if stick.y > 0.0 {
            Act::Up
        } else {
            Act::Down
        })
    }
}

/// A gamepad's actions this frame.
pub fn gamepad(p: &Pad, latch: &mut StickLatch, dt: f32) -> Vec<Act> {
    let mut out = Vec::new();
    for (on, a) in [
        (p.dpad_left, Act::Left),
        (p.dpad_right, Act::Right),
        (p.dpad_up, Act::Up),
        (p.dpad_down, Act::Down),
        (p.south, Act::Confirm),
        (p.east, Act::Back),
        (p.north, Act::Compare),
        (p.west, Act::Action),
        (p.lb, Act::SlotPrev),
        (p.rb, Act::SlotNext),
        (p.r3, Act::ResetView),
    ] {
        if on {
            out.push(a);
        }
    }
    out.extend(latch.update(p.left));
    if p.right.length() > DEAD_ZONE {
        out.push(Act::Orbit(p.right * STICK_ORBIT_RAD_S * dt, 0.0));
    }
    let zoom = p.rt - p.lt;
    if zoom.abs() > DEAD_ZONE {
        out.push(Act::Zoom(zoom * 4.0 * dt));
    }
    out
}

/// The mouse this frame.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Mouse {
    /// Cursor position (logical px), if over the window.
    pub at: Option<Vec2>,
    pub moved: Vec2,
    pub wheel: f32,
}

/// The mouse's actions: hover (only when it moved), clicks (a left
/// press-and-release without dragging), left drag = orbit, wheel = zoom,
/// right click = back, middle click = compare.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MouseState {
    /// Dragged since the left button went down (px).
    dragged: f32,
}

impl MouseState {
    pub fn update(&mut self, buttons: &ButtonInput<MouseButton>, m: Mouse, dt: f32) -> Vec<Act> {
        let mut out = Vec::new();
        let Some(at) = m.at else {
            return out;
        };
        if buttons.just_pressed(MouseButton::Left) {
            self.dragged = 0.0;
        }
        if buttons.pressed(MouseButton::Left) && m.moved != Vec2::ZERO {
            self.dragged += m.moved.length();
            if self.dragged > 6.0 {
                out.push(Act::Orbit(
                    Vec2::new(-m.moved.x, m.moved.y) * DRAG_RAD_PER_PX,
                    dt,
                ));
            }
        } else if m.moved.length() > 0.0 {
            out.push(Act::Point(at));
        }
        if buttons.just_released(MouseButton::Left) && self.dragged <= 6.0 {
            out.push(Act::Activate(at));
        }
        if buttons.just_pressed(MouseButton::Right) {
            out.push(Act::Back);
        }
        if buttons.just_pressed(MouseButton::Middle) {
            out.push(Act::Compare);
        }
        if m.wheel != 0.0 {
            out.push(Act::Zoom(m.wheel));
        }
        out
    }
}

/// A touch event for the recogniser.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Touch {
    Down { id: u64, at: Vec2, t: f64 },
    Move { id: u64, at: Vec2, t: f64 },
    Up { id: u64, at: Vec2, t: f64 },
}

/// Tap / double tap / long press / drag / swipe / pinch / two-finger tap
/// (spec §3.2.2 *Gesture recogniser*), pure.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Gestures {
    fingers: Vec<(u64, Vec2, Vec2, f64)>,
    /// The touch is a drag (moved beyond 10 px) or became something else.
    dragging: bool,
    long_pressed: bool,
    two_finger: bool,
    pinch_dist: Option<f32>,
    last_tap: Option<(Vec2, f64)>,
    /// The last move: position and time (the swipe speed).
    last_move: Option<(Vec2, f64)>,
    velocity: Vec2,
}

/// Movement under this is still a tap (px).
pub const TAP_SLOP_PX: f32 = 10.0;
pub const TAP_MAX_S: f64 = 0.3;
pub const LONG_PRESS_S: f64 = 0.5;
/// A drag released faster than this is a swipe (px / ms).
pub const SWIPE_PX_MS: f32 = 0.5;

impl Gestures {
    #[cfg(test)]
    pub fn fingers(&self) -> usize {
        self.fingers.len()
    }

    /// The actions of one touch event.
    pub fn event(&mut self, e: Touch) -> Vec<Act> {
        let mut out = Vec::new();
        match e {
            Touch::Down { id, at, t } => {
                if self.fingers.is_empty() {
                    self.dragging = false;
                    self.long_pressed = false;
                    self.two_finger = false;
                    self.velocity = Vec2::ZERO;
                }
                self.fingers.push((id, at, at, t));
                self.last_move = Some((at, t));
                if self.fingers.len() == 2 {
                    self.two_finger = true;
                    self.pinch_dist = Some(self.fingers[0].2.distance(self.fingers[1].2));
                }
                if self.fingers.len() > 2 {
                    // A third finger cancels.
                    self.dragging = true;
                    self.pinch_dist = None;
                }
            }
            Touch::Move { id, at, t } => {
                let Some(i) = self.fingers.iter().position(|f| f.0 == id) else {
                    return out;
                };
                let prev = self.fingers[i].2;
                self.fingers[i].2 = at;
                match self.fingers.len() {
                    1 => {
                        if at.distance(self.fingers[i].1) > TAP_SLOP_PX {
                            self.dragging = true;
                        }
                        if self.dragging && !self.long_pressed && !self.two_finger {
                            let d = at - prev;
                            if let Some((p, pt)) = self.last_move {
                                let ms = ((t - pt) * 1000.0).max(1.0) as f32;
                                self.velocity = (at - p) / ms;
                            }
                            out.push(Act::Orbit(Vec2::new(-d.x, d.y) * DRAG_RAD_PER_PX, 0.0));
                        }
                        self.last_move = Some((at, t));
                    }
                    2 => {
                        let d = self.fingers[0].2.distance(self.fingers[1].2);
                        if let Some(before) = self.pinch_dist
                            && before > 1.0
                            && (d - before).abs() > 2.0
                        {
                            out.push(Act::Zoom((d / before).ln() / 0.128));
                            self.pinch_dist = Some(d);
                            self.dragging = true;
                        }
                    }
                    _ => {}
                }
            }
            Touch::Up { id, at, t } => {
                let Some(i) = self.fingers.iter().position(|f| f.0 == id) else {
                    return out;
                };
                let (_, start, _, t0) = self.fingers.remove(i);
                if self.two_finger {
                    if self.fingers.is_empty() && !self.dragging && t - t0 < TAP_MAX_S * 2.0 {
                        out.push(Act::Back);
                    }
                    if self.fingers.is_empty() {
                        self.two_finger = false;
                    }
                    return out;
                }
                if self.long_pressed {
                    return out;
                }
                if !self.dragging && t - t0 < TAP_MAX_S && at.distance(start) <= TAP_SLOP_PX {
                    let double = self
                        .last_tap
                        .is_some_and(|(p, pt)| t - pt < TAP_MAX_S && p.distance(at) < 20.0);
                    if double {
                        self.last_tap = None;
                        out.push(Act::ResetView);
                    } else {
                        self.last_tap = Some((at, t));
                        out.push(Act::Activate(at));
                    }
                } else if self.dragging {
                    let v = self.velocity;
                    if v.x.abs() > SWIPE_PX_MS && v.x.abs() > v.y.abs() * 1.5 {
                        out.push(Act::Swipe(if v.x > 0.0 { 1 } else { -1 }));
                    }
                }
            }
        }
        out
    }

    /// Called every frame: a finger held still for [`LONG_PRESS_S`] is a
    /// long press (compare).
    pub fn tick(&mut self, now: f64) -> Vec<Act> {
        if self.fingers.len() == 1 && !self.dragging && !self.long_pressed {
            let (_, start, at, t0) = self.fingers[0];
            if now - t0 >= LONG_PRESS_S && at.distance(start) <= TAP_SLOP_PX {
                self.long_pressed = true;
                return vec![Act::Compare];
            }
        }
        Vec::new()
    }
}

/// The hint bar's text for `device` in the scene state `ctx`.
pub fn hint(device: ActiveInput, ctx: HintCtx) -> String {
    use ActiveInput::*;
    let (sel, back, cmp, act, slot, orbit, cycle) = match device {
        Keyboard => (
            "Enter select",
            "Esc back",
            "C compare",
            "X buy/sell",
            "Q/E part",
            "IJKL / +- view",
            "←/→ bikes",
        ),
        Mouse => (
            "Click select",
            "Right click back",
            "Middle click compare",
            "Click the buttons",
            "Click a marker",
            "Drag / wheel view",
            "Click a bike",
        ),
        Gamepad => (
            "Ⓐ select",
            "Ⓑ back",
            "Ⓨ compare",
            "Ⓧ buy/sell",
            "LB/RB part",
            "R-stick / LT RT view",
            "◀/▶ bikes",
        ),
        Touch => (
            "Tap to select",
            "Two-finger tap: back",
            "Long press: compare",
            "Tap the buttons",
            "Tap a marker",
            "Drag / pinch: view",
            "Swipe: bikes",
        ),
    };
    let parts: Vec<&str> = match ctx {
        HintCtx::Room => vec![cycle, "↑/↓ zones", sel, back],
        HintCtx::Stand => vec![slot, sel, orbit, back],
        HintCtx::Picker => vec![sel, cmp, act, slot, back],
        HintCtx::Dialog => vec![sel, back],
        HintCtx::Corner => vec![sel, back],
    };
    let parts: Vec<&str> = parts
        .into_iter()
        .map(|p| {
            if device != Keyboard && p == "↑/↓ zones" {
                match device {
                    Gamepad => "▲/▼ zones",
                    Mouse => "Click bench / mirror",
                    Touch => "Tap bench / mirror",
                    Keyboard => p,
                }
            } else {
                p
            }
        })
        .collect();
    parts.join("   ")
}

/// Which hints apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HintCtx {
    Room,
    Stand,
    Picker,
    Dialog,
    Corner,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(pressed: &[KeyCode]) -> ButtonInput<KeyCode> {
        let mut k = ButtonInput::default();
        for p in pressed {
            k.press(*p);
        }
        k
    }

    #[test]
    fn keyboard_maps_every_action() {
        let k = |p: &[KeyCode]| keyboard(&keys(p), 0.1);
        assert_eq!(k(&[KeyCode::ArrowLeft]), vec![Act::Left]);
        assert_eq!(k(&[KeyCode::ArrowDown]), vec![Act::Down]);
        assert_eq!(k(&[KeyCode::Enter]), vec![Act::Confirm]);
        assert_eq!(k(&[KeyCode::Space]), vec![Act::Confirm]);
        assert_eq!(k(&[KeyCode::Escape]), vec![Act::Back]);
        assert_eq!(k(&[KeyCode::KeyQ]), vec![Act::SlotPrev]);
        assert_eq!(k(&[KeyCode::PageDown]), vec![Act::SlotNext]);
        assert_eq!(k(&[KeyCode::KeyC]), vec![Act::Compare]);
        assert_eq!(k(&[KeyCode::KeyX]), vec![Act::Action]);
        assert_eq!(k(&[KeyCode::Home]), vec![Act::ResetView]);
        // Orbit: J / L yaw, I / K pitch; Shift + arrows too, not the moves.
        let Act::Orbit(d, _) = k(&[KeyCode::KeyL])[0] else {
            panic!()
        };
        assert!(d.x > 0.0 && d.y == 0.0);
        let a = k(&[KeyCode::ShiftLeft, KeyCode::ArrowUp]);
        assert!(
            matches!(a.as_slice(), [Act::Orbit(d, _)] if d.y > 0.0),
            "{a:?}"
        );
        assert!(matches!(k(&[KeyCode::Equal])[0], Act::Zoom(z) if z > 0.0));
        assert!(matches!(k(&[KeyCode::Minus])[0], Act::Zoom(z) if z < 0.0));
    }

    #[test]
    fn gamepad_maps_every_action() {
        let mut latch = StickLatch::default();
        let mut g = |p: Pad| gamepad(&p, &mut latch, 0.1);
        assert_eq!(
            g(Pad {
                south: true,
                ..Default::default()
            }),
            vec![Act::Confirm]
        );
        assert_eq!(
            g(Pad {
                east: true,
                ..Default::default()
            }),
            vec![Act::Back]
        );
        assert_eq!(
            g(Pad {
                north: true,
                ..Default::default()
            }),
            vec![Act::Compare]
        );
        assert_eq!(
            g(Pad {
                west: true,
                ..Default::default()
            }),
            vec![Act::Action]
        );
        assert_eq!(
            g(Pad {
                lb: true,
                ..Default::default()
            }),
            vec![Act::SlotPrev]
        );
        assert_eq!(
            g(Pad {
                rb: true,
                ..Default::default()
            }),
            vec![Act::SlotNext]
        );
        assert_eq!(
            g(Pad {
                dpad_right: true,
                ..Default::default()
            }),
            vec![Act::Right]
        );
        assert_eq!(
            g(Pad {
                r3: true,
                ..Default::default()
            }),
            vec![Act::ResetView]
        );
        // The left stick steps once per push.
        let push = Pad {
            left: Vec2::new(0.0, -0.9),
            ..Default::default()
        };
        assert_eq!(g(push), vec![Act::Down]);
        assert_eq!(g(push), vec![]);
        assert_eq!(g(Pad::default()), vec![]);
        assert_eq!(g(push), vec![Act::Down]);
        let a = g(Pad {
            right: Vec2::new(1.0, 0.0),
            rt: 1.0,
            ..Default::default()
        });
        assert!(matches!(a[0], Act::Orbit(d, _) if d.x > 0.0));
        assert!(matches!(a[1], Act::Zoom(z) if z > 0.0));
        assert!(
            Pad {
                lt: 0.5,
                ..Default::default()
            }
            .active()
        );
        assert!(!Pad::default().active());
    }

    #[test]
    fn mouse_hover_click_drag_wheel() {
        let mut s = MouseState::default();
        let mut b = ButtonInput::<MouseButton>::default();
        let at = Vec2::new(100.0, 100.0);
        let m = |moved: Vec2, wheel: f32| Mouse {
            at: Some(at),
            moved,
            wheel,
        };
        // Resting: nothing; moving: hover.
        assert_eq!(s.update(&b, m(Vec2::ZERO, 0.0), 0.016), vec![]);
        assert_eq!(s.update(&b, m(Vec2::X, 0.0), 0.016), vec![Act::Point(at)]);
        // Press and release in place: a click.
        b.press(MouseButton::Left);
        assert_eq!(s.update(&b, m(Vec2::ZERO, 0.0), 0.016), vec![]);
        b.clear();
        b.release(MouseButton::Left);
        assert_eq!(
            s.update(&b, m(Vec2::ZERO, 0.0), 0.016),
            vec![Act::Activate(at)]
        );
        // A drag orbits and is no click.
        b.clear();
        b.press(MouseButton::Left);
        s.update(&b, m(Vec2::ZERO, 0.0), 0.016);
        b.clear();
        let a = s.update(&b, m(Vec2::new(30.0, 0.0), 0.0), 0.016);
        assert!(
            matches!(a[0], Act::Orbit(d, dt) if d.x < 0.0 && dt > 0.0),
            "{a:?}"
        );
        b.release(MouseButton::Left);
        assert!(
            !s.update(&b, m(Vec2::ZERO, 0.0), 0.016)
                .contains(&Act::Activate(at))
        );
        b.clear();
        b.press(MouseButton::Right);
        assert_eq!(s.update(&b, m(Vec2::ZERO, 0.0), 0.016), vec![Act::Back]);
        b.clear();
        b.press(MouseButton::Middle);
        assert_eq!(s.update(&b, m(Vec2::ZERO, 0.0), 0.016), vec![Act::Compare]);
        b.clear();
        assert_eq!(
            s.update(&b, m(Vec2::ZERO, 2.0), 0.016),
            vec![Act::Zoom(2.0)]
        );
    }

    fn down(id: u64, x: f32, t: f64) -> Touch {
        Touch::Down {
            id,
            at: Vec2::new(x, 300.0),
            t,
        }
    }
    fn mv(id: u64, x: f32, t: f64) -> Touch {
        Touch::Move {
            id,
            at: Vec2::new(x, 300.0),
            t,
        }
    }
    fn up(id: u64, x: f32, t: f64) -> Touch {
        Touch::Up {
            id,
            at: Vec2::new(x, 300.0),
            t,
        }
    }

    #[test]
    fn touch_tap_double_tap_long_press() {
        let mut g = Gestures::default();
        assert_eq!(g.event(down(1, 100.0, 0.0)), vec![]);
        assert_eq!(
            g.event(up(1, 103.0, 0.1)),
            vec![Act::Activate(Vec2::new(103.0, 300.0))]
        );
        g.event(down(2, 104.0, 0.25));
        assert_eq!(
            g.event(up(2, 104.0, 0.3)),
            vec![Act::ResetView],
            "double tap"
        );
        // A slow tap is no tap; a long press compares (once).
        g.event(down(3, 400.0, 2.0));
        assert_eq!(g.tick(2.3), vec![]);
        assert_eq!(g.tick(2.55), vec![Act::Compare]);
        assert_eq!(g.tick(2.8), vec![]);
        assert_eq!(g.event(up(3, 400.0, 3.0)), vec![]);
    }

    #[test]
    fn touch_drag_orbits_and_a_fast_one_swipes() {
        let mut g = Gestures::default();
        g.event(down(1, 100.0, 0.0));
        let a = g.event(mv(1, 130.0, 0.05));
        assert!(matches!(a[0], Act::Orbit(d, _) if d.x < 0.0), "{a:?}");
        g.event(mv(1, 250.0, 0.1));
        assert_eq!(g.event(up(1, 260.0, 0.12)), vec![Act::Swipe(1)]);
        // A slow drag is no swipe.
        g.event(down(1, 500.0, 1.0));
        g.event(mv(1, 520.0, 1.2));
        g.event(mv(1, 540.0, 1.4));
        assert_eq!(g.event(up(1, 540.0, 1.5)), vec![]);
    }

    #[test]
    fn touch_pinch_zooms_and_two_finger_tap_goes_back() {
        let mut g = Gestures::default();
        g.event(down(1, 100.0, 0.0));
        g.event(down(2, 200.0, 0.01));
        let a = g.event(mv(2, 300.0, 0.05));
        assert!(
            matches!(a[0], Act::Zoom(z) if z > 0.0),
            "spread = closer: {a:?}"
        );
        g.event(up(1, 100.0, 0.2));
        assert_eq!(g.event(up(2, 300.0, 0.21)), vec![], "a pinch is no tap");
        g.event(down(1, 100.0, 1.0));
        g.event(down(2, 200.0, 1.01));
        g.event(up(1, 100.0, 1.1));
        assert_eq!(g.event(up(2, 200.0, 1.12)), vec![Act::Back]);
        assert_eq!(g.fingers(), 0);
        // A third finger cancels.
        g.event(down(1, 100.0, 2.0));
        g.event(down(2, 200.0, 2.0));
        g.event(down(3, 300.0, 2.0));
        for id in [1, 2, 3] {
            assert_eq!(g.event(up(id, 100.0, 2.1)), vec![]);
        }
    }

    #[test]
    fn hints_follow_the_device() {
        assert!(hint(ActiveInput::Keyboard, HintCtx::Room).contains("Enter"));
        assert!(hint(ActiveInput::Gamepad, HintCtx::Picker).contains("Ⓨ compare"));
        assert!(hint(ActiveInput::Touch, HintCtx::Picker).contains("Long press"));
        assert!(hint(ActiveInput::Mouse, HintCtx::Stand).contains("Drag"));
        assert!(hint(ActiveInput::Touch, HintCtx::Room).contains("Swipe"));
    }
}
