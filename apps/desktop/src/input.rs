//! Keyboard controls (when the console is closed):
//! - ← / → steer across the road (held: 2 m/s, the rider follows at up to
//!   1.5 m/s); later they also choose the turn at junctions (spec §2.2.1),
//! - ↑ / ↓ and PageUp / PageDown change the virtual trainer's power by 10 W.

use bevy::prelude::*;
use ocw_devices::DeviceCommand;
use ocw_render::console::ConsoleState;

use crate::connection::ClientState;
use crate::devices::Devices;

pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (steer, adjust_virtual_power));
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

fn adjust_virtual_power(
    keys: Res<ButtonInput<KeyCode>>,
    console: Res<ConsoleState>,
    devices: Res<Devices>,
    client: Res<ClientState>,
) {
    if console.is_open() {
        return;
    }
    let delta = power_delta(
        keys.any_just_pressed([KeyCode::PageUp, KeyCode::ArrowUp]),
        keys.any_just_pressed([KeyCode::PageDown, KeyCode::ArrowDown]),
    );
    if delta != 0.0 {
        let watts = (client.0.power_w() + delta).clamp(0.0, 2500.0);
        devices.command_all(DeviceCommand::SetVirtualPower(watts));
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
}
