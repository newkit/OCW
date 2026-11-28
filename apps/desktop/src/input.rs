//! Keyboard controls (in the ride's input context: not while the menu or
//! the console is open, see `ocw_render::menu::InputContext`):
//! - A toggles autodrive (on at the start): the rider keeps to the middle
//!   of the road by itself, ← / → move its line. Without autodrive, ← / →
//!   lean the rider into a curve (held: further, up to 30°); the lean
//!   stays when released; the opposite key straightens up, pauses at
//!   upright for `LEAN_UPRIGHT_PAUSE_S` (so riding straight is easy to
//!   hit) and, still held, leans on to the other side. The road's own
//!   bends have to be steered,
//! - free ride (spec 2.2.1, 3.2.5): Delete / Q choose the junction's exit
//!   one step to the left, Page Down / E one step to the right (the
//!   gamepad: D-pad left / right), at any time and in every steering
//!   mode; ← / → only steer, never choose the exit. U (gamepad East:
//!   B / ○) turns round (↓ stays the brake), J back to the route,
//! - W toggles wild mode: not bound to the road; switching it on also
//!   switches autodrive off (upright, lean steering),
//! - gamepad: left stick left / right steers (leans, or moves the autodrive
//!   line); ↑ = D-pad up, left stick up or right trigger; ↓ = D-pad down,
//!   left stick down or left trigger; South (A / ✕) = autodrive, West
//!   (X / □) = wild mode,
//! - ↑ accelerates: each press adds `POWER_STEP_W` to the rider's power,
//!   holding adds `POWER_RAMP_W_PER_S` (up to `MAX_KEY_POWER_W`), on top of
//!   the device's power (virtual or real trainer); it also releases the brake,
//! - ↓ stops: keyboard and virtual-trainer power to 0, and the brake
//!   (Page Up / Page Down no longer change the power, spec 3.2.5).
//!
//! These are the defaults: every key and button here is rebindable
//! (spec 3.2.5, `ocw_client::bindings`, Settings → Controls → Input
//! mapping); the systems read them through `ocw_ui::bindings::ActiveBindings`.
//! The sticks and triggers stay fixed.

use bevy::prelude::*;
use ocw_devices::DeviceCommand;
use ocw_render::menu::ride_input;

use ocw_client::bindings::Action;
use ocw_ui::bindings::ActiveBindings;

use crate::connection::{ClientState, Link};
use crate::devices::Devices;
// The pure key logic (shared with the browser app, spec §14 Phase 10).
pub use ocw_client::input::*;

/// Keyboard power (↑ / ↓, the touch buttons; [`KeyThrottle`], shared with
/// the browser app).
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq)]
pub struct KeyPower(pub KeyThrottle);

pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<KeyPower>()
            // God mode (spec §14 Phase 13B): the riding keys (A, W, U, J,
            // the exit keys, the arrows' steering and power) do nothing in
            // flight.
            .add_systems(
                Update,
                (steer, throttle_and_brake)
                    .run_if(ride_input)
                    .run_if(crate::god::not_flying),
            );
    }
}

#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
fn steer(
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    time: Res<Time>,
    link: Res<Link>,
    bindings: Option<Res<ActiveBindings>>,
    mut client: ResMut<ClientState>,
    mut upright_pause: Local<f64>,
    mut stick_was_used: Local<bool>,
) {
    let default = ActiveBindings::default();
    let b = bindings.as_deref().unwrap_or(&default);
    let key = |a| b.key_just_pressed(a, &keys);
    let pad = |a| b.pad_just_pressed(a, pads.iter());
    // An event's results panel takes A / B (spec §2.3).
    let now = time.elapsed_secs_f64();
    let results = client.0.results_input(now);
    // Select + North / West are god mode's chords, not these.
    let select = pads.iter().any(|p| p.pressed(GamepadButton::Select));
    // Free ride: U-turn, rejoin, the junction exits (shared with the
    // browser app; ← / → never choose the exit, spec 3.2.5).
    let (msgs, said) = ocw_ui::controls::free_ride_keys(b, &keys, pads.iter(), &mut client.0, now);
    for line in said {
        info!("{line}");
    }
    for msg in msgs {
        link.send_bytes(msg);
    }
    let autodrive = key(Action::Autodrive) || pad(Action::Autodrive);
    if autodrive && results {
        client.0.dismiss_results(now);
    } else if autodrive {
        let on = !client.0.autodrive();
        client.0.set_autodrive(on);
        info!("autodrive {}", if on { "on" } else { "off" });
    }
    if key(Action::Wild) || (pad(Action::Wild) && !select) {
        let wild = !client.0.wild();
        client.0.set_wild(wild);
        info!("wild mode {}", if wild { "on" } else { "off" });
    }
    let dt = time.delta_secs_f64();
    let dir = steer_direction(
        b.pressed(Action::SteerLeft, &keys, pads.iter()),
        b.pressed(Action::SteerRight, &keys, pads.iter()),
    );
    let stick = pads
        .iter()
        .map(|p| p.get(GamepadAxis::LeftStickX).unwrap_or(0.0))
        .max_by(|a, b| a.abs().total_cmp(&b.abs()))
        .unwrap_or(0.0);
    let stick_active = stick.abs() >= STICK_DEAD_ZONE;
    if client.0.autodrive() {
        let shift = dir + if stick_active { f64::from(stick) } else { 0.0 };
        if shift != 0.0 {
            client.0.steer_by(shift * LINE_KEY_RATE_MPS * dt);
        }
        *stick_was_used = false;
        return;
    }
    if stick_active || *stick_was_used {
        // The stick sets the lean directly; back to centre = upright.
        client.0.set_lean(stick_lean(stick));
        *stick_was_used = stick_active;
        return;
    }
    let lean = client.0.lean().unwrap_or(0.0);
    let next = lean_ramp(lean, dir, &mut upright_pause, dt);
    if dir != 0.0 && next != lean {
        client.0.set_lean(next);
    }
}

#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
fn throttle_and_brake(
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<(Entity, &Gamepad)>,
    time: Res<Time>,
    devices: Res<Devices>,
    link: Res<Link>,
    mut client: ResMut<ClientState>,
    mut key: ResMut<KeyPower>,
    mut ride_pads: Local<ocw_ui::controls::RidePads>,
    bindings: Option<Res<ActiveBindings>>,
    touch_power: Option<ResMut<ocw_ui::touch::TouchPower>>,
) {
    let now = time.elapsed_secs_f64();
    let default = ActiveBindings::default();
    let b = bindings.as_deref().unwrap_or(&default);
    // The pads' throttle (shared with the browser app: `ocw_ui::controls`).
    let pad = ride_pads.read_bound(pads.iter(), b);
    if pad.down_pressed {
        info!("gamepad: stop (power 0, brake)");
    }
    // With the touch zones (spec 3.2.3: the top of the screen is faster,
    // the bottom slower).
    let mut throttle = ocw_ui::controls::throttle_keys(b, &keys, &pad);
    if let Some(mut t) = touch_power {
        throttle = throttle.or(t.take());
    }
    let (msgs, stop) = key
        .0
        .update(&mut client.0, throttle, time.delta_secs_f64(), now);
    if stop {
        // Stop: no virtual trainer power either.
        devices.command_all(DeviceCommand::SetVirtualPower(0.0));
    }
    for msg in msgs {
        link.send_bytes(msg);
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use bevy::input::touch::{TouchInput, TouchPhase};
    use bevy::time::TimeUpdateStrategy;
    use clap::Parser;
    use ocw_render::RideView;
    use ocw_render::console::ConsoleCorePlugin;
    use ocw_world::Route;

    use super::*;

    /// Touch steering in the desktop app (spec §3.2.3 Q6: Windows / Linux
    /// touch screens): Bevy's `TouchInput` on the desktop's local server
    /// steers like the arrow keys – a tap +0.5 m, a hold as the held key –
    /// and never ends autodrive.
    #[test]
    fn synthetic_touches_steer_like_the_arrow_keys() {
        let argv = ["ocw-desktop", "--bots", "0", "--power", "150", "--virtual"];
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::input::InputPlugin, ConsoleCorePlugin))
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
                20,
            )))
            .insert_resource(crate::Args::parse_from(argv))
            .insert_resource(RideView::new(Route::test_loop()))
            .insert_resource(crate::connection::Ghosts(None))
            .init_resource::<ocw_render::camera::CameraObstacles>()
            .add_plugins((
                crate::connection::ConnectionPlugin {
                    route: Route::test_loop(),
                },
                crate::devices::DevicesPlugin,
                InputPlugin,
                ocw_ui::touch::TouchSteerPlugin,
            ));
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(4)
            && app.world().resource::<ClientState>().0.rider_id().is_none()
        {
            app.update();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(app.world().resource::<ClientState>().0.rider_id().is_some());
        app.world_mut()
            .resource_mut::<ClientState>()
            .0
            .steer_to(0.0);
        let line = |app: &App| {
            app.world()
                .resource::<ClientState>()
                .0
                .line_target()
                .unwrap_or(0.0)
        };
        let touch = |app: &mut App, phase, x: f32| {
            app.world_mut().write_message(TouchInput {
                phase,
                position: Vec2::new(x, 500.0),
                window: Entity::PLACEHOLDER,
                force: None,
                id: 3,
            });
        };
        touch(&mut app, TouchPhase::Started, 1100.0);
        app.update();
        touch(&mut app, TouchPhase::Ended, 1100.0);
        app.update();
        let tapped = line(&app);
        assert!((tapped - 0.5).abs() < 1e-6, "{tapped}");
        touch(&mut app, TouchPhase::Started, 150.0);
        for _ in 0..20 {
            app.update();
        }
        touch(&mut app, TouchPhase::Ended, 150.0);
        app.update();
        let by_touch = tapped - line(&app);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::ArrowLeft);
        for _ in 0..20 {
            app.update();
        }
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release(KeyCode::ArrowLeft);
        app.update();
        let by_key = tapped - by_touch - line(&app);
        assert!((by_touch - by_key).abs() < 1e-6, "{by_touch} vs {by_key}");
        assert!((by_touch - 0.8).abs() < 1e-6, "{by_touch}");
        assert!(app.world().resource::<ClientState>().0.autodrive());
        let t = app.world().resource::<ocw_ui::touch::RideTouch>();
        assert!(t.active && !t.shown, "the key hid the hints");

        // The faster / slower zones (spec 3.2.3 *The on-screen controls*;
        // the virtual trainer): a tap at the top of the screen is ↑
        // (25 W), at the bottom 10 W less and, with no power left, a stop
        // – through this app's power keys, as in the other app.
        let tap_at = |app: &mut App, y: f32| {
            for phase in [TouchPhase::Started, TouchPhase::Ended] {
                app.world_mut().write_message(TouchInput {
                    phase,
                    position: Vec2::new(640.0, y),
                    window: Entity::PLACEHOLDER,
                    force: None,
                    id: 7,
                });
                app.update();
            }
            // The power is sent at most every 0.1 s.
            for _ in 0..8 {
                app.update();
            }
        };
        let client = |app: &App| app.world().resource::<ClientState>().0.clone_power();
        app.world_mut()
            .resource_mut::<ClientState>()
            .0
            .set_virtual_device(true);
        app.update();
        tap_at(&mut app, 100.0);
        assert_eq!(client(&app), (25.0, false));
        tap_at(&mut app, 650.0);
        assert_eq!(client(&app), (15.0, false));
        tap_at(&mut app, 650.0);
        tap_at(&mut app, 650.0);
        assert_eq!(client(&app), (0.0, false));
        tap_at(&mut app, 650.0);
        assert_eq!(client(&app), (0.0, true), "no power left: a stop");
        tap_at(&mut app, 100.0);
        assert_eq!(client(&app), (25.0, false), "the top rides on");
        assert!(
            (line(&app) - (tapped - 2.0 * by_touch)).abs() < 1e-6,
            "no zone steered"
        );
    }
}
