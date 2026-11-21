//! Bevy's keyboard and gamepads → the menu's buttons
//! (`ocw_client::menu::controls::Pressed`, spec §3.2.1). The mapping table
//! is in `ocw_client::menu::controls`. In the browser the gamepads come
//! through the same `Gamepad` entities: Bevy's gilrs backend reads the
//! browser's Gamepad API on wasm32 (spec §14 Phase 10, B6).

use bevy::prelude::*;
pub use ocw_client::menu::controls::*;

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
        fine: shift,
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

/// One gamepad's menu buttons this frame; `swap`: Confirm is East and
/// Back South (setting `controls.pad_confirm east`, spec §3.2.1 M9).
pub fn pad_state(p: &Gamepad, swap: bool) -> PadState {
    let (confirm, back) = if swap {
        (GamepadButton::East, GamepadButton::South)
    } else {
        (GamepadButton::South, GamepadButton::East)
    };
    PadState {
        start: p.just_pressed(GamepadButton::Start),
        south: p.just_pressed(confirm),
        east: p.just_pressed(back),
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

/// The keyboard and every gamepad, merged (`swap`: the pads' Confirm and
/// Back swapped).
pub fn pressed<'a>(
    keys: Option<&ButtonInput<KeyCode>>,
    pads: impl IntoIterator<Item = &'a Gamepad>,
    swap: bool,
) -> Pressed {
    pads.into_iter()
        .fold(keys.map(keyboard).unwrap_or_default(), |p, pad| {
            p.merge(gamepad(pad_state(pad, swap)))
        })
}

/// The ride's gamepad input of one frame from all pads (spec §3.2.2; the
/// desktop and the browser alike): "faster" and "stop" from the D-pad,
/// the left stick and the triggers (`ocw_client::input::pad_throttle`),
/// the steering stick, the buttons pressed this frame.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PadRide {
    /// "Faster" is held / went down this frame; "stop" went down.
    pub up: bool,
    pub up_pressed: bool,
    pub down_pressed: bool,
    /// The left stick's X with the largest deflection (−1 … 1).
    pub stick_x: f32,
}

/// What [`RidePads`] remembers between frames: the triggers' resting
/// values per pad and last frame's throttle.
#[derive(Debug, Default)]
pub struct RidePads {
    was: (bool, bool),
    rest: std::collections::HashMap<Entity, [Option<f32>; 2]>,
}

impl RidePads {
    /// This frame's [`PadRide`] from `pads` with the default buttons.
    pub fn read<'a>(&mut self, pads: impl IntoIterator<Item = (Entity, &'a Gamepad)>) -> PadRide {
        self.read_bound(pads, &crate::bindings::ActiveBindings::default())
    }

    /// This frame's [`PadRide`]: "faster" and "stop" on the buttons of the
    /// bindings (spec §3.2.1 M9; the stick and the triggers are fixed).
    pub fn read_bound<'a>(
        &mut self,
        pads: impl IntoIterator<Item = (Entity, &'a Gamepad)>,
        bindings: &crate::bindings::ActiveBindings,
    ) -> PadRide {
        use ocw_client::bindings::Action;
        use ocw_client::input::{pad_throttle, trigger_pull};
        let mut seen = Vec::new();
        let mut stick_x = 0.0f32;
        let (mut up, mut down) = (false, false);
        for (e, p) in pads {
            seen.push(e);
            let rest = self.rest.entry(e).or_default();
            let rt = trigger_pull(
                p.get(GamepadButton::RightTrigger2).unwrap_or(0.0),
                &mut rest[0],
            );
            let lt = trigger_pull(
                p.get(GamepadButton::LeftTrigger2).unwrap_or(0.0),
                &mut rest[1],
            );
            let (pu, pd) = pad_throttle(
                p.get(GamepadAxis::LeftStickY).unwrap_or(0.0),
                bindings.pad_pressed(Action::Faster, [p]),
                bindings.pad_pressed(Action::Stop, [p]),
                rt,
                lt,
            );
            up |= pu;
            down |= pd;
            let x = p.get(GamepadAxis::LeftStickX).unwrap_or(0.0);
            if x.abs() > stick_x.abs() {
                stick_x = x;
            }
        }
        self.rest.retain(|e, _| seen.contains(e));
        let out = PadRide {
            up,
            up_pressed: up && !self.was.0,
            down_pressed: down && !self.was.1,
            stick_x,
        };
        self.was = (up, down);
        out
    }
}

/// The keyboard power's keys of this frame (bound: Faster, Stop) with the
/// pads' throttle. No key changes the power by 10 W (spec 3.2.5: Page Up
/// / Page Down no longer do; the touch buttons' ±10 W are added by the
/// apps).
pub fn throttle_keys(
    b: &crate::bindings::ActiveBindings,
    keys: &ButtonInput<KeyCode>,
    pad: &PadRide,
) -> ocw_client::input::ThrottleKeys {
    use ocw_client::bindings::Action;
    ocw_client::input::ThrottleKeys {
        up_pressed: b.key_just_pressed(Action::Faster, keys) || pad.up_pressed,
        up_held: b.key_pressed(Action::Faster, keys) || pad.up,
        down_pressed: b.key_just_pressed(Action::Stop, keys) || pad.down_pressed,
        plus_10: false,
        minus_10: false,
    }
}

/// The free ride's keys of one frame, the same in both apps (spec 2.2.1,
/// 3.2.5): U-turn (U, East unless an event's results panel takes it),
/// back to the route (J, North unless Select is held: the unicorn's
/// chord) and the junction exits (Delete / Q and Page Down / E, the
/// D-pad left / right). ← / → never choose an exit: they steer. Returns
/// the messages for the server and the log lines; the exit choice goes
/// out with the client's `turn_message`.
pub fn free_ride_keys<'a>(
    b: &crate::bindings::ActiveBindings,
    keys: &ButtonInput<KeyCode>,
    pads: impl IntoIterator<Item = &'a Gamepad>,
    client: &mut ocw_client::client::ClientCore,
    now: f64,
) -> (Vec<Vec<u8>>, Vec<String>) {
    use ocw_client::bindings::Action;
    use ocw_client::input::{JunctionKey, junction_key};
    let (mut msgs, mut said) = (Vec::new(), Vec::new());
    if !client.is_free_ride() {
        return (msgs, said);
    }
    let pads: Vec<&Gamepad> = pads.into_iter().collect();
    let key = |a| b.key_just_pressed(a, keys);
    let pad = |a| b.pad_just_pressed(a, pads.iter().copied());
    let results = client.results_input(now);
    let select = pads.iter().any(|p| p.pressed(GamepadButton::Select));
    if (key(Action::UTurn) || (pad(Action::UTurn) && !results))
        && let Some(msg) = client.u_turn(now)
    {
        said.push("U-turn".into());
        msgs.push(msg);
    }
    if key(Action::Rejoin) || (pad(Action::Rejoin) && !select) {
        match client.rejoin_route() {
            Ok(msg) => {
                said.push("rejoining the route".into());
                msgs.push(msg);
            }
            Err(e) => said.push(format!("no rejoin: {e}")),
        }
    }
    if let JunctionKey::Select(steps) = junction_key(
        key(Action::ExitLeft) || pad(Action::ExitLeft),
        key(Action::ExitRight) || pad(Action::ExitRight),
    ) && let Some(s) = client.turn(steps)
    {
        said.push(s);
    }
    (msgs, said)
}

/// Whether any pad pressed `b` this frame.
pub fn any_just_pressed<'a>(pads: impl IntoIterator<Item = &'a Gamepad>, b: GamepadButton) -> bool {
    pads.into_iter().any(|p| p.just_pressed(b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_client::menu::model::Action;

    #[test]
    fn the_ride_pads_read_throttle_and_stick() {
        let mut pads = RidePads::default();
        let mut pad = Gamepad::default();
        let e = Entity::from_raw_u32(7).unwrap();
        assert_eq!(pads.read([(e, &pad)]), PadRide::default());
        pad.digital_mut().press(GamepadButton::DPadUp);
        let r = pads.read([(e, &pad)]);
        assert!(r.up && r.up_pressed && !r.down_pressed);
        let r = pads.read([(e, &pad)]);
        assert!(r.up && !r.up_pressed, "held");
        pad.digital_mut().release(GamepadButton::DPadUp);
        // D-pad down zooms the navigation map now (spec 3.2 Q6), it no
        // longer stops; the left stick down does.
        pad.digital_mut().press(GamepadButton::DPadDown);
        let r = pads.read([(e, &pad)]);
        assert!(!r.up && !r.down_pressed, "D-pad down is the map's zoom");
        pad.digital_mut().release(GamepadButton::DPadDown);
        pad.analog_mut().set(GamepadAxis::LeftStickY, -0.9);
        let r = pads.read([(e, &pad)]);
        assert!(!r.up && r.down_pressed, "the stick stops");
    }

    /// Spec 3.2 Q6 (2026-11-19: "yes, the zoom on a d-pad direction"):
    /// D-pad down steps the navigation map's zoom (auto → 1 … 7 → auto),
    /// in a ride and in unicorn flight; the bindings say so.
    #[test]
    fn d_pad_down_steps_the_map_zoom() {
        use crate::bindings::{ActiveBindings, pad_just_pressed};
        use ocw_client::bindings::Action as Bind;
        let b = ActiveBindings::default();
        let mut pad = Gamepad::default();
        assert!(!pad_just_pressed(Some(&b), Bind::MapZoom, [&pad]));
        pad.digital_mut().press(GamepadButton::DPadDown);
        assert!(pad_just_pressed(Some(&b), Bind::MapZoom, [&pad]));
        assert!(!pad_just_pressed(Some(&b), Bind::Stop, [&pad]));
        assert!(Bind::MapZoom.def().in_flight && Bind::Map.def().in_flight);
    }

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
        // Shift: fine steps (the calendar's time stepper, spec 9J).
        assert_eq!(
            k(&[KeyCode::ShiftLeft, KeyCode::ArrowRight], true),
            [Fine(1)]
        );
        assert_eq!(
            k(&[KeyCode::ShiftRight, KeyCode::ArrowLeft], true),
            [Fine(-1)]
        );
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
    fn a_gamepad_entity_reads_its_buttons() {
        let mut pad = Gamepad::default();
        pad.digital_mut().press(GamepadButton::Start);
        let p = pressed(None, [&pad], false);
        assert!(p.start);
        assert_eq!(actions(&p, 0, false), [Action::Open]);
        assert_eq!(pad_state(&pad, false).stick_y, 0.0);
    }

    /// M9: Confirm and Back swapped (a Nintendo layout).
    #[test]
    fn confirm_and_back_can_be_swapped() {
        let mut pad = Gamepad::default();
        pad.digital_mut().press(GamepadButton::East);
        let p = pressed(None, [&pad], false);
        assert_eq!(actions(&p, 0, true), [Action::Back]);
        let p = pressed(None, [&pad], true);
        assert_eq!(actions(&p, 0, true), [Action::Confirm]);
        let mut pad = Gamepad::default();
        pad.digital_mut().press(GamepadButton::South);
        assert_eq!(
            actions(&pressed(None, [&pad], true), 0, true),
            [Action::Back]
        );
    }
}
