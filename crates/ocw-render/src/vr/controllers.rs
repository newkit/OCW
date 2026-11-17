//! The VR controllers as a gamepad (spec §14 Phase 11, v4.68): the
//! headset backend writes each frame's [`ControllerState`] into
//! [`VrState::controllers`]; here it becomes a Bevy gamepad named "VR
//! controllers", fed through Bevy's own raw gamepad events (so its
//! buttons press and release like a real pad's, with the same dead zones),
//! and everything that reads gamepads – the ride, the menu, unicorn mode,
//! the voice keys – works with the Touch controllers too
//! (`ocw_xr::controllers` has the table). Both grips held recentre the
//! view. Every change is logged (`VR controllers: button_a down, …`), so a
//! headset run's log shows that the buttons arrive.

use bevy::input::InputSystems;
use bevy::input::gamepad::{
    GamepadAxis, GamepadButton, GamepadSettings, RawGamepadAxisChangedEvent,
    RawGamepadButtonChangedEvent, RawGamepadEvent,
};
use bevy::prelude::*;
use ocw_xr::controllers::{ControllerState, PadAxis, PadButton, PadMapper};

use super::VrState;

/// The gamepad entity of the controllers.
#[derive(Component)]
pub struct VrPad;

/// The mapper and what was sent last.
#[derive(Resource, Default)]
struct PadFeed {
    mapper: PadMapper,
    entity: Option<Entity>,
    sent: Vec<(PadButton, f32)>,
    axes: Vec<(PadAxis, f32)>,
    logged: ControllerState,
}

pub fn plugin(app: &mut App) {
    app.init_resource::<PadFeed>()
        .add_systems(PreUpdate, feed.before(InputSystems));
}

pub fn button(b: PadButton) -> GamepadButton {
    match b {
        PadButton::South => GamepadButton::South,
        PadButton::East => GamepadButton::East,
        PadButton::West => GamepadButton::West,
        PadButton::North => GamepadButton::North,
        PadButton::Start => GamepadButton::Start,
        PadButton::LeftThumb => GamepadButton::LeftThumb,
        PadButton::RightThumb => GamepadButton::RightThumb,
        PadButton::LeftBumper => GamepadButton::LeftTrigger,
        PadButton::RightBumper => GamepadButton::RightTrigger,
        PadButton::LeftTrigger2 => GamepadButton::LeftTrigger2,
        PadButton::RightTrigger2 => GamepadButton::RightTrigger2,
        PadButton::DPadUp => GamepadButton::DPadUp,
        PadButton::DPadDown => GamepadButton::DPadDown,
        PadButton::DPadLeft => GamepadButton::DPadLeft,
        PadButton::DPadRight => GamepadButton::DPadRight,
    }
}

pub fn axis(a: PadAxis) -> GamepadAxis {
    match a {
        PadAxis::LeftStickX => GamepadAxis::LeftStickX,
        PadAxis::LeftStickY => GamepadAxis::LeftStickY,
        PadAxis::RightStickX => GamepadAxis::RightStickX,
        PadAxis::RightStickY => GamepadAxis::RightStickY,
    }
}

fn feed(
    mut commands: Commands,
    time: Res<Time>,
    mut state: ResMut<VrState>,
    mut pad: ResMut<PadFeed>,
    mut raw: MessageWriter<RawGamepadEvent>,
    menu: Option<Res<crate::menu::MenuView>>,
) {
    // The left stick moves through the menu only while it is open (Q10).
    pad.mapper.menu_open = menu.is_some_and(|m| m.open);
    let Some(c) = state.controllers else {
        // No controllers this frame: release what is held.
        if pad.entity.is_some() && (!pad.sent.is_empty() || !pad.axes.is_empty()) {
            let rest = ControllerState::default();
            send(&mut pad, &rest, 0.0, &mut raw);
        }
        return;
    };
    let changes = c.changes(&pad.logged);
    if !changes.is_empty() {
        info!("VR controllers: {changes}");
        pad.logged = c;
    }
    if pad.entity.is_none() {
        let e = commands
            .spawn((
                Name::new("VR controllers"),
                Gamepad::default(),
                GamepadSettings::default(),
                VrPad,
            ))
            .id();
        info!("VR controllers: a gamepad for the Touch controllers ({e})");
        pad.entity = Some(e);
        // The entity exists from the next frame on.
        return;
    }
    if send(&mut pad, &c, time.delta_secs(), &mut raw) {
        state.recentre_now = true;
        info!("VR controllers: both grips held – recentred");
    }
}

/// Sends what changed; returns the recentre gesture.
fn send(
    pad: &mut PadFeed,
    c: &ControllerState,
    dt: f32,
    raw: &mut MessageWriter<RawGamepadEvent>,
) -> bool {
    let Some(e) = pad.entity else {
        return false;
    };
    let f = pad.mapper.map(c, dt);
    for (b, v) in &f.buttons {
        let old = pad.sent.iter().find(|(x, _)| x == b).map(|(_, v)| *v);
        if old != Some(*v) && !(old.is_none() && *v == 0.0) {
            raw.write(RawGamepadEvent::Button(RawGamepadButtonChangedEvent::new(
                e,
                button(*b),
                *v,
            )));
        }
    }
    for (a, v) in &f.axes {
        let old = pad.axes.iter().find(|(x, _)| x == a).map(|(_, v)| *v);
        if old != Some(*v) && !(old.is_none() && *v == 0.0) {
            raw.write(RawGamepadEvent::Axis(RawGamepadAxisChangedEvent::new(
                e,
                axis(*a),
                *v,
            )));
        }
    }
    pad.sent = f.buttons;
    pad.axes = f.axes;
    f.recentre
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_xr::controllers::{Input, InputValue};

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::input::InputPlugin))
            .init_resource::<VrState>();
        plugin(&mut app);
        app
    }

    fn set(app: &mut App, f: impl FnOnce(&mut ControllerState)) {
        let mut s = ControllerState::at_rest(true);
        f(&mut s);
        app.world_mut().resource_mut::<VrState>().controllers = Some(s);
    }

    fn pad<R>(app: &mut App, f: impl FnOnce(&Gamepad) -> R) -> R {
        let mut q = app.world_mut().query_filtered::<&Gamepad, With<VrPad>>();
        f(q.single(app.world()).expect("the VR gamepad"))
    }

    /// The Touch controllers are a gamepad: A presses South (just pressed
    /// once, then held), the trigger is RT2's value, the stick the axes.
    #[test]
    fn the_controllers_press_gamepad_buttons() {
        let mut app = app();
        set(&mut app, |_| {});
        app.update();
        app.update();
        assert!(!pad(&mut app, |p| p.pressed(GamepadButton::South)));
        set(&mut app, |s| {
            s.set(Input::A, InputValue::Button(true));
            s.set(Input::RightTrigger, InputValue::Value(0.9));
            // The right stick rides (Q10): the gamepad's left stick, past
            // the dead zone ((0.5 − 0.15) / 0.85 ≈ 0.41).
            s.set(Input::RightStick, InputValue::Stick([-0.5, 0.0]));
        });
        app.update();
        pad(&mut app, |p| {
            assert!(p.just_pressed(GamepadButton::South));
            assert!((p.get(GamepadButton::RightTrigger2).unwrap() - 0.9).abs() < 1e-3);
            assert!((p.get(GamepadAxis::LeftStickX).unwrap() + 0.41).abs() < 0.05);
        });
        app.update();
        pad(&mut app, |p| {
            assert!(p.pressed(GamepadButton::South) && !p.just_pressed(GamepadButton::South));
        });
        // Released.
        set(&mut app, |_| {});
        app.update();
        assert!(pad(&mut app, |p| p.just_released(GamepadButton::South)));
        // The menu button is Start; with the menu open a left-stick flick
        // is the D-pad (Q10).
        app.insert_resource(crate::menu::MenuView {
            open: true,
            ..default()
        });
        set(&mut app, |s| {
            s.set(Input::Menu, InputValue::Button(true));
            s.set(Input::LeftStick, InputValue::Stick([0.0, -0.95]));
        });
        app.update();
        pad(&mut app, |p| {
            assert!(p.just_pressed(GamepadButton::Start));
            assert!(p.just_pressed(GamepadButton::DPadDown));
        });
    }

    /// Losing the controllers (no focus, headset off) releases everything.
    #[test]
    fn without_controllers_the_pad_releases() {
        let mut app = app();
        set(&mut app, |_| {});
        app.update();
        set(&mut app, |s| s.set(Input::B, InputValue::Button(true)));
        app.update();
        assert!(pad(&mut app, |p| p.pressed(GamepadButton::East)));
        app.world_mut().resource_mut::<VrState>().controllers = None;
        app.update();
        assert!(!pad(&mut app, |p| p.pressed(GamepadButton::East)));
    }

    #[test]
    fn both_grips_recentre() {
        let mut app = app();
        set(&mut app, |_| {});
        app.update();
        set(&mut app, |s| {
            s.set(Input::LeftGrip, InputValue::Value(1.0));
            s.set(Input::RightGrip, InputValue::Value(1.0));
        });
        // MinimalPlugins' time advances by real time: hold long enough.
        let start = std::time::Instant::now();
        while start.elapsed().as_secs_f32() < 0.8 {
            app.update();
            if app.world().resource::<VrState>().recentre_now {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(app.world().resource::<VrState>().recentre_now);
    }
}
