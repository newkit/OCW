//! Keyboard controls (when the console is closed):
//! - ← / → steer across the road (held: 2 m/s, the rider follows at up to
//!   1.5 m/s); later they also choose the turn at junctions (spec §2.2.1),
//! - ↑ (held) accelerates: the virtual trainer's power rises by
//!   `POWER_RAMP_W_PER_S` up to `MAX_KEY_POWER_W`, and releases the brake,
//! - ↓ stops: virtual power 0 and the brake (with a real trainer only the
//!   brake acts; its power comes from the rider's legs),
//! - PageUp / PageDown change the virtual power by 10 W.

use bevy::prelude::*;
use ocw_devices::DeviceCommand;
use ocw_render::console::ConsoleState;

use crate::connection::{ClientState, Link};
use crate::devices::Devices;

pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        let start = app.world().resource::<crate::Args>().power;
        app.insert_resource(VirtualPowerTarget {
            watts: start,
            sent: start,
        })
        .add_systems(Update, (steer, throttle_and_brake));
    }
}

/// Power rise per second while ↑ is held.
pub const POWER_RAMP_W_PER_S: f64 = 200.0;
/// Highest virtual power reachable with ↑.
pub const MAX_KEY_POWER_W: f64 = 1000.0;

/// The virtual power the keys aim for (the trainer reports only at 4 Hz,
/// so ramping from its last report would stutter).
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct VirtualPowerTarget {
    pub watts: f64,
    /// Last value sent to the trainer.
    pub sent: f64,
}

/// New power target after `dt` seconds with ↑ held or not.
pub fn ramp(watts: f64, up_held: bool, dt: f64) -> f64 {
    if up_held {
        (watts + POWER_RAMP_W_PER_S * dt).min(MAX_KEY_POWER_W.max(watts))
    } else {
        watts
    }
}

/// How fast the steering target moves while an arrow key is held, in m/s.
pub const STEER_KEY_RATE_MPS: f64 = 2.0;

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
) {
    if console.is_open() {
        return;
    }
    let dir = steer_direction(
        keys.pressed(KeyCode::ArrowLeft),
        keys.pressed(KeyCode::ArrowRight),
    );
    if dir != 0.0 {
        client
            .0
            .steer_by(dir * STEER_KEY_RATE_MPS * time.delta_secs_f64());
    }
}

fn throttle_and_brake(
    keys: Res<ButtonInput<KeyCode>>,
    console: Res<ConsoleState>,
    time: Res<Time>,
    devices: Res<Devices>,
    link: Res<Link>,
    mut client: ResMut<ClientState>,
    mut target: ResMut<VirtualPowerTarget>,
) {
    if console.is_open() {
        return;
    }
    let up = keys.pressed(KeyCode::ArrowUp);
    if keys.just_pressed(KeyCode::ArrowDown) {
        target.watts = 0.0;
        if let Some(msg) = client.0.set_brake(true) {
            link.send_bytes(msg);
        }
    }
    if keys.just_pressed(KeyCode::ArrowUp)
        && let Some(msg) = client.0.set_brake(false)
    {
        link.send_bytes(msg);
    }
    target.watts = ramp(target.watts, up, time.delta_secs_f64());
    target.watts = (target.watts
        + power_delta(
            keys.just_pressed(KeyCode::PageUp),
            keys.just_pressed(KeyCode::PageDown),
        ))
    .clamp(0.0, 2500.0);
    // Send when it changed noticeably (or reached 0), not every frame.
    if (target.watts - target.sent).abs() >= 5.0 || (target.watts == 0.0 && target.sent != 0.0) {
        target.sent = target.watts;
        devices.command_all(DeviceCommand::SetVirtualPower(target.watts));
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
        assert_eq!(power_delta(true, false), 10.0);
        assert_eq!(power_delta(false, true), -10.0);
        assert_eq!(power_delta(true, true), 0.0);
    }

    #[test]
    fn up_ramps_the_power_and_stops_at_the_cap() {
        assert_eq!(ramp(200.0, false, 1.0), 200.0);
        assert_eq!(ramp(200.0, true, 0.5), 300.0);
        assert_eq!(ramp(950.0, true, 1.0), MAX_KEY_POWER_W);
        // Above the cap (set via the console) the key does not lower it.
        assert_eq!(ramp(1500.0, true, 1.0), 1500.0);
    }
}
