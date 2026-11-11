//! Keyboard controls (when the console is closed):
//! - ← / → lean the rider into a curve (held: further, up to 30°); the
//!   lean stays when released, the opposite key straightens up (it stops
//!   at upright until the key is pressed again); later they also choose
//!   the turn at junctions (spec §2.2.1),
//! - ↑ accelerates: each press adds `POWER_STEP_W` to the rider's power,
//!   holding adds `POWER_RAMP_W_PER_S` (up to `MAX_KEY_POWER_W`), on top of
//!   the device's power (virtual or real trainer); it also releases the brake,
//! - ↓ stops: keyboard and virtual-trainer power to 0, and the brake,
//! - PageUp / PageDown change the keyboard power by 10 W.

use bevy::prelude::*;
use ocw_devices::DeviceCommand;
use ocw_render::console::ConsoleState;

use crate::connection::{ClientState, Link};
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

fn steer(
    keys: Res<ButtonInput<KeyCode>>,
    console: Res<ConsoleState>,
    time: Res<Time>,
    mut client: ResMut<ClientState>,
    mut latched: Local<bool>,
) {
    if console.is_open() {
        return;
    }
    let dir = steer_direction(
        keys.pressed(KeyCode::ArrowLeft),
        keys.pressed(KeyCode::ArrowRight),
    );
    let lean = client.0.lean().unwrap_or(0.0);
    let next = lean_ramp(lean, dir, &mut latched, time.delta_secs_f64());
    if dir != 0.0 && (next != lean || client.0.lean().is_none()) {
        client.0.set_lean(next);
    }
}

fn throttle_and_brake(
    keys: Res<ButtonInput<KeyCode>>,
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
    if keys.just_pressed(KeyCode::ArrowDown) {
        // Stop: no keyboard power, no virtual trainer power, brake.
        key.watts = 0.0;
        devices.command_all(DeviceCommand::SetVirtualPower(0.0));
        if let Some(msg) = client.0.set_brake(true) {
            link.send_bytes(msg);
        }
    }
    if keys.just_pressed(KeyCode::ArrowUp)
        && let Some(msg) = client.0.set_brake(false)
    {
        link.send_bytes(msg);
    }
    key.watts = ramp(
        key.watts,
        keys.just_pressed(KeyCode::ArrowUp),
        keys.pressed(KeyCode::ArrowUp),
        time.delta_secs_f64(),
    );
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
    fn key_mapping() {
        assert_eq!(steer_direction(true, false), -1.0);
        assert_eq!(steer_direction(false, true), 1.0);
        assert_eq!(steer_direction(true, true), 0.0);
        assert_eq!(steer_direction(false, false), 0.0);
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
