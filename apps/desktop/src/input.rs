//! Keyboard controls (when the console is closed).

use bevy::prelude::*;
use ocw_devices::DeviceCommand;
use ocw_render::console::ConsoleState;

use crate::connection::ClientState;
use crate::devices::Devices;

pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, adjust_virtual_power);
    }
}

/// PageUp / PageDown change the virtual trainer's power by 10 W.
fn adjust_virtual_power(
    keys: Res<ButtonInput<KeyCode>>,
    console: Res<ConsoleState>,
    devices: Res<Devices>,
    client: Res<ClientState>,
) {
    if console.is_open() {
        return;
    }
    let delta = if keys.just_pressed(KeyCode::PageUp) {
        10.0
    } else if keys.just_pressed(KeyCode::PageDown) {
        -10.0
    } else {
        return;
    };
    let watts = (client.0.power_w() + delta).clamp(0.0, 2500.0);
    devices.command_all(DeviceCommand::SetVirtualPower(watts));
}
