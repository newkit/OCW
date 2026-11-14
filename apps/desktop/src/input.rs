//! Keyboard controls (in the ride's input context: not while the menu or
//! the console is open, see `ocw_render::menu::InputContext`):
//! - A toggles autodrive (on at the start): the rider keeps to the middle
//!   of the road by itself, ← / → move its line. Without autodrive, ← / →
//!   lean the rider into a curve (held: further, up to 30°); the lean
//!   stays when released; the opposite key straightens up, pauses at
//!   upright for `LEAN_UPRIGHT_PAUSE_S` (so riding straight is easy to
//!   hit) and, still held, leans on to the other side. The road's own
//!   bends have to be steered,
//! - free ride (spec §2.2.1): while a junction is ahead (within the
//!   look-ahead, before the lock point) and autodrive is on, ← / → choose
//!   the exit instead (one exit per press; the line stays). Q / E (and the
//!   gamepad's D-pad left / right) choose the exit at any time, also for
//!   manual steering, where the arrows keep leaning. U (gamepad East:
//!   B / ○) turns round (↓ stays the brake),
//! - W toggles wild mode: not bound to the road; switching it on also
//!   switches autodrive off (upright, lean steering),
//! - gamepad: left stick left / right steers (leans, or moves the autodrive
//!   line); ↑ = D-pad up, left stick up or right trigger; ↓ = D-pad down,
//!   left stick down or left trigger; South (A / ✕) = autodrive, West
//!   (X / □) = wild mode,
//! - ↑ accelerates: each press adds `POWER_STEP_W` to the rider's power,
//!   holding adds `POWER_RAMP_W_PER_S` (up to `MAX_KEY_POWER_W`), on top of
//!   the device's power (virtual or real trainer); it also releases the brake,
//! - ↓ stops: keyboard and virtual-trainer power to 0, and the brake,
//! - PageUp / PageDown change the keyboard power by 10 W.

use bevy::prelude::*;
use ocw_devices::DeviceCommand;
use ocw_render::menu::ride_input;

use crate::connection::{ClientState, Link};
use crate::devices::Devices;
// The pure key logic (shared with the browser app, spec §14 Phase 10).
pub use ocw_client::input::*;

/// Keyboard power (↑ / ↓, PageUp / PageDown; [`KeyThrottle`], shared with
/// the browser app).
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq)]
pub struct KeyPower(pub KeyThrottle);

pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<KeyPower>()
            // God mode (spec §14 Phase 13B): the riding keys (A, W, U, J,
            // Q / E, the arrows' steering and power) do nothing in flight.
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
    mut client: ResMut<ClientState>,
    mut upright_pause: Local<f64>,
    mut stick_was_used: Local<bool>,
) {
    let pad_pressed = |b: GamepadButton| pads.iter().any(|p| p.just_pressed(b));
    // An event's results panel takes A / B (spec §2.3).
    let now = time.elapsed_secs_f64();
    let results = client.0.results_input(now);
    if client.0.is_free_ride() {
        if (keys.just_pressed(KeyCode::KeyU) || (pad_pressed(GamepadButton::East) && !results))
            && let Some(msg) = client.0.u_turn(time.elapsed_secs_f64())
        {
            info!("U-turn");
            link.send_bytes(msg);
        }
        // Route ride: back to the route after leaving it (Select + North
        // is god mode's chord, not this).
        let select = pads.iter().any(|p| p.pressed(GamepadButton::Select));
        if keys.just_pressed(KeyCode::KeyJ) || (pad_pressed(GamepadButton::North) && !select) {
            match client.0.rejoin_route() {
                Ok(msg) => {
                    info!("rejoining the route");
                    link.send_bytes(msg);
                }
                Err(e) => info!("no rejoin: {e}"),
            }
        }
        let (key, arrows_taken) = junction_key(
            keys.just_pressed(KeyCode::KeyQ) || pad_pressed(GamepadButton::DPadLeft),
            keys.just_pressed(KeyCode::KeyE) || pad_pressed(GamepadButton::DPadRight),
            keys.just_pressed(KeyCode::ArrowLeft),
            keys.just_pressed(KeyCode::ArrowRight),
            client.0.autodrive(),
            client.0.junction_active(),
        );
        if let JunctionKey::Select(steps) = key
            && let Some(said) = client.0.turn(steps)
        {
            info!("{said}");
        }
        if arrows_taken {
            return;
        }
    }
    if (keys.just_pressed(KeyCode::KeyA) || pad_pressed(GamepadButton::South)) && results {
        client.0.dismiss_results(now);
    } else if keys.just_pressed(KeyCode::KeyA) || pad_pressed(GamepadButton::South) {
        let on = !client.0.autodrive();
        client.0.set_autodrive(on);
        info!("autodrive {}", if on { "on" } else { "off" });
    }
    let select = pads.iter().any(|p| p.pressed(GamepadButton::Select));
    if keys.just_pressed(KeyCode::KeyW) || (pad_pressed(GamepadButton::West) && !select) {
        let wild = !client.0.wild();
        client.0.set_wild(wild);
        info!("wild mode {}", if wild { "on" } else { "off" });
    }
    let dt = time.delta_secs_f64();
    let dir = steer_direction(
        keys.pressed(KeyCode::ArrowLeft),
        keys.pressed(KeyCode::ArrowRight),
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
    mut pad_was: Local<(bool, bool)>,
    mut trigger_rest: Local<std::collections::HashMap<Entity, [Option<f32>; 2]>>,
) {
    let now = time.elapsed_secs_f64();
    trigger_rest.retain(|e, _| pads.contains(*e));
    let (pad_up, pad_down) = pads.iter().fold((false, false), |(u, d), (e, p)| {
        let rest = trigger_rest.entry(e).or_default();
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
            p.pressed(GamepadButton::DPadUp),
            p.pressed(GamepadButton::DPadDown),
            rt,
            lt,
        );
        (u || pu, d || pd)
    });
    let (pad_up_pressed, pad_down_pressed) = (pad_up && !pad_was.0, pad_down && !pad_was.1);
    *pad_was = (pad_up, pad_down);
    if pad_down_pressed {
        info!("gamepad: stop (power 0, brake)");
    }
    let throttle = ThrottleKeys {
        up_pressed: keys.just_pressed(KeyCode::ArrowUp) || pad_up_pressed,
        up_held: keys.pressed(KeyCode::ArrowUp) || pad_up,
        down_pressed: keys.just_pressed(KeyCode::ArrowDown) || pad_down_pressed,
        page_up: keys.just_pressed(KeyCode::PageUp),
        page_down: keys.just_pressed(KeyCode::PageDown),
    };
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
