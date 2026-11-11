//! Keyboard controls (when the console is closed):
//! - A toggles autodrive (on at the start): the rider keeps to the middle
//!   of the road by itself, ← / → move its line. Without autodrive, ← / →
//!   lean the rider into a curve (held: further, up to 30°); the lean
//!   stays when released, the opposite key straightens up (it stops at
//!   upright until the key is pressed again), and the road's own bends
//!   have to be steered,
//! - free ride (spec §2.2.1): while a junction is ahead (within the
//!   look-ahead, before the lock point) and autodrive is on, ← / → choose
//!   the exit instead (one exit per press; the line stays). Q / E (and the
//!   gamepad's D-pad left / right) choose the exit at any time, also for
//!   manual steering, where the arrows keep leaning. U (D-pad down) turns
//!   round (↓ stays the brake),
//! - W toggles wild mode: not bound to the road,
//! - gamepad: left stick steers (leans, or moves the autodrive line),
//!   right trigger = ↑, left trigger = ↓, South (A) = autodrive, West (X)
//!   = wild mode,
//! - ↑ accelerates: each press adds `POWER_STEP_W` to the rider's power,
//!   holding adds `POWER_RAMP_W_PER_S` (up to `MAX_KEY_POWER_W`), on top of
//!   the device's power (virtual or real trainer); it also releases the brake,
//! - ↓ stops: keyboard and virtual-trainer power to 0, and the brake,
//! - PageUp / PageDown change the keyboard power by 10 W.

use bevy::prelude::*;
use ocw_devices::DeviceCommand;
use ocw_render::console::ConsoleState;

use crate::connection::{ClientState, Link};

/// Free ride: what the junction keys do this frame (spec §2.2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JunctionKey {
    /// Not a junction key: steer as usual.
    None,
    /// Move the exit selection by this many exits (negative = left).
    Select(i32),
}

/// Maps the keys to a junction action: Q / E always; the arrows only in
/// autodrive while a junction is active (then they do not move the line).
/// Returns the action and whether the arrows are taken by the junction.
pub fn junction_key(
    q: bool,
    e: bool,
    left: bool,
    right: bool,
    autodrive: bool,
    junction_active: bool,
) -> (JunctionKey, bool) {
    let arrows = autodrive && junction_active;
    let mut steps = i32::from(e) - i32::from(q);
    if arrows {
        steps += i32::from(right) - i32::from(left);
    }
    let key = if steps == 0 {
        JunctionKey::None
    } else {
        JunctionKey::Select(steps)
    };
    (key, arrows)
}
use crate::devices::Devices;

pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<KeyPower>()
            .add_systems(Update, (steer, throttle_and_brake));
    }
}

/// Power added by each press of ↑.
pub const POWER_STEP_W: f64 = 25.0;
/// Power rise per second while ↑ is held.
pub const POWER_RAMP_W_PER_S: f64 = 250.0;
/// Highest keyboard power reachable with ↑.
pub const MAX_KEY_POWER_W: f64 = 1000.0;
/// Minimum time between power updates while ↑ is held.
pub const KEY_SEND_INTERVAL_S: f64 = 0.1;

/// Keyboard power (↑ / ↓, PageUp / PageDown): added to the rider's device
/// power (virtual or real trainer), so the keys work with any device.
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq)]
pub struct KeyPower {
    pub watts: f64,
    pub last_sent_s: f64,
}

/// New power target after `dt` seconds: `pressed` = ↑ went down this frame
/// (a step), `held` = ↑ is down (a ramp).
pub fn ramp(watts: f64, pressed: bool, held: bool, dt: f64) -> f64 {
    let step = if pressed { POWER_STEP_W } else { 0.0 };
    let rise = if held { POWER_RAMP_W_PER_S * dt } else { 0.0 };
    if step + rise == 0.0 {
        return watts;
    }
    (watts + step + rise).min(MAX_KEY_POWER_W.max(watts))
}

/// How fast the autodrive line moves while an arrow key is held, in m/s.
pub const LINE_KEY_RATE_MPS: f64 = 2.0;
/// Gamepad stick values below this are treated as centred.
pub const STICK_DEAD_ZONE: f32 = 0.08;

/// Lean (rad) for a gamepad stick deflection (−1 … 1): proportional, so
/// a centred stick rides upright.
pub fn stick_lean(x: f32) -> f64 {
    let x = if x.abs() < STICK_DEAD_ZONE {
        0.0
    } else {
        x.clamp(-1.0, 1.0)
    };
    f64::from(x) * ocw_sim::physics::MAX_STEER_LEAN
}

/// How fast the lean changes while an arrow key is held, in rad/s (~23°/s).
pub const LEAN_KEY_RATE: f64 = 0.4;

/// New lean (rad) after `dt` with the arrow key direction `dir`. Bringing
/// the lean back through upright stops there (`latched`) until the keys
/// are released, so straightening up is easy.
pub fn lean_ramp(lean: f64, dir: f64, latched: &mut bool, dt: f64) -> f64 {
    if dir == 0.0 {
        *latched = false;
        return lean;
    }
    if *latched {
        return lean;
    }
    let max = ocw_sim::physics::MAX_STEER_LEAN;
    let next = (lean + dir * LEAN_KEY_RATE * dt).clamp(-max, max);
    if lean != 0.0 && (next == 0.0 || next.signum() != lean.signum()) {
        *latched = true;
        return 0.0;
    }
    next
}

/// Steering direction from the arrow keys: −1 left, +1 right, 0 none.
pub fn steer_direction(left: bool, right: bool) -> f64 {
    f64::from(i8::from(right) - i8::from(left))
}

/// Power change in watts from the keys pressed this frame.
pub fn power_delta(up: bool, down: bool) -> f64 {
    10.0 * f64::from(i8::from(up) - i8::from(down))
}

#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
fn steer(
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    console: Res<ConsoleState>,
    time: Res<Time>,
    link: Res<Link>,
    mut client: ResMut<ClientState>,
    mut latched: Local<bool>,
    mut stick_was_used: Local<bool>,
) {
    if console.is_open() {
        return;
    }
    let pad_pressed = |b: GamepadButton| pads.iter().any(|p| p.just_pressed(b));
    if client.0.is_free_ride() {
        if (keys.just_pressed(KeyCode::KeyU) || pad_pressed(GamepadButton::DPadDown))
            && let Some(msg) = client.0.u_turn(time.elapsed_secs_f64())
        {
            info!("U-turn");
            link.send_bytes(msg);
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
    if keys.just_pressed(KeyCode::KeyA) || pad_pressed(GamepadButton::South) {
        let on = !client.0.autodrive();
        client.0.set_autodrive(on);
        info!("autodrive {}", if on { "on" } else { "off" });
    }
    if keys.just_pressed(KeyCode::KeyW) || pad_pressed(GamepadButton::West) {
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
    let next = lean_ramp(lean, dir, &mut latched, dt);
    if dir != 0.0 && next != lean {
        client.0.set_lean(next);
    }
}

#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
fn throttle_and_brake(
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    console: Res<ConsoleState>,
    time: Res<Time>,
    devices: Res<Devices>,
    link: Res<Link>,
    mut client: ResMut<ClientState>,
    mut key: ResMut<KeyPower>,
) {
    if console.is_open() {
        return;
    }
    let now = time.elapsed_secs_f64();
    let pad = |f: &dyn Fn(&Gamepad) -> bool| pads.iter().any(f);
    let up_pressed = keys.just_pressed(KeyCode::ArrowUp)
        || pad(&|p| p.just_pressed(GamepadButton::RightTrigger2));
    let up_held =
        keys.pressed(KeyCode::ArrowUp) || pad(&|p| p.pressed(GamepadButton::RightTrigger2));
    let down_pressed = keys.just_pressed(KeyCode::ArrowDown)
        || pad(&|p| p.just_pressed(GamepadButton::LeftTrigger2));
    if down_pressed {
        // Stop: no keyboard power, no virtual trainer power, brake.
        key.watts = 0.0;
        devices.command_all(DeviceCommand::SetVirtualPower(0.0));
        if let Some(msg) = client.0.set_brake(true) {
            link.send_bytes(msg);
        }
    }
    if up_pressed && let Some(msg) = client.0.set_brake(false) {
        link.send_bytes(msg);
    }
    key.watts = ramp(key.watts, up_pressed, up_held, time.delta_secs_f64());
    key.watts = (key.watts
        + power_delta(
            keys.just_pressed(KeyCode::PageUp),
            keys.just_pressed(KeyCode::PageDown),
        ))
    .clamp(0.0, 2500.0);
    // Send the rider's new power at most every KEY_SEND_INTERVAL_S while it
    // changes, and at once when it reached 0.
    let changed = (key.watts - client.0.key_power_w()).abs() > 1e-9;
    if changed && (now - key.last_sent_s >= KEY_SEND_INTERVAL_S || key.watts == 0.0) {
        key.last_sent_s = now;
        if let Some(msg) = client.0.set_key_power(key.watts) {
            link.send_bytes(msg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn junction_keys_take_the_arrows_only_in_autodrive_near_a_junction() {
        use JunctionKey::*;
        // Q / E always.
        assert_eq!(
            junction_key(true, false, false, false, false, false),
            (Select(-1), false)
        );
        assert_eq!(
            junction_key(false, true, false, false, true, false),
            (Select(1), false)
        );
        // Arrows: only with autodrive and an active junction.
        assert_eq!(
            junction_key(false, false, true, false, true, true),
            (Select(-1), true)
        );
        assert_eq!(
            junction_key(false, false, false, true, true, false),
            (None, false)
        );
        // Manual steering keeps the arrows for leaning.
        assert_eq!(
            junction_key(false, false, false, true, false, true),
            (None, false)
        );
        // Taken even when nothing is pressed (the line does not move).
        assert_eq!(
            junction_key(false, false, false, false, true, true),
            (None, true)
        );
    }

    #[test]
    fn key_mapping() {
        assert_eq!(steer_direction(true, false), -1.0);
        assert_eq!(steer_direction(false, true), 1.0);
        assert_eq!(steer_direction(true, true), 0.0);
        assert_eq!(steer_direction(false, false), 0.0);
    }

    #[test]
    fn stick_leans_proportionally_with_a_dead_zone() {
        assert_eq!(stick_lean(0.05), 0.0);
        assert_eq!(stick_lean(-0.05), 0.0);
        assert!((stick_lean(0.5) - 0.5 * ocw_sim::physics::MAX_STEER_LEAN).abs() < 1e-6);
        assert_eq!(stick_lean(-3.0), -ocw_sim::physics::MAX_STEER_LEAN);
    }

    #[test]
    fn lean_keys_ramp_hold_and_stop_at_upright() {
        let mut latched = false;
        // Hold right for half a second: leaning right.
        let mut lean = 0.0;
        for _ in 0..30 {
            lean = lean_ramp(lean, 1.0, &mut latched, 1.0 / 60.0);
        }
        assert!((lean - 0.2).abs() < 1e-9, "{lean}");
        // Released: the lean stays.
        assert_eq!(lean_ramp(lean, 0.0, &mut latched, 1.0), lean);
        // Left straightens up and stops at upright while held.
        for _ in 0..120 {
            lean = lean_ramp(lean, -1.0, &mut latched, 1.0 / 60.0);
        }
        assert_eq!(lean, 0.0);
        // Released and pressed again: now it leans left.
        lean = lean_ramp(lean, 0.0, &mut latched, 1.0 / 60.0);
        lean = lean_ramp(lean, -1.0, &mut latched, 0.5);
        assert!(lean < 0.0);
        // Never beyond the limit.
        lean = lean_ramp(lean, -1.0, &mut latched, 100.0);
        assert_eq!(lean, -ocw_sim::physics::MAX_STEER_LEAN);
        assert_eq!(power_delta(true, false), 10.0);
        assert_eq!(power_delta(false, true), -10.0);
        assert_eq!(power_delta(true, true), 0.0);
    }

    #[test]
    fn up_ramps_the_power_and_stops_at_the_cap() {
        assert_eq!(ramp(200.0, false, false, 1.0), 200.0);
        // A short tap (one frame at 144 FPS) adds a full step.
        let tap = ramp(200.0, true, true, 1.0 / 144.0);
        assert!((tap - 226.7).abs() < 0.1, "{tap}");
        // Holding: +250 W/s.
        assert_eq!(ramp(200.0, false, true, 0.4), 300.0);
        assert_eq!(ramp(990.0, true, true, 1.0), MAX_KEY_POWER_W);
        // Above the cap (set via the console) the key does not lower it.
        assert_eq!(ramp(1500.0, false, true, 1.0), 1500.0);
    }
}
