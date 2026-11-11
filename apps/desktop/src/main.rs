//! Open Cycling World desktop client.
//!
//! Wires the pieces together (spec §6.1):
//! - a device backend (Phase 1: the virtual trainer) on its own thread,
//! - the in-process local server on its own thread at 50 Hz,
//! - client-side prediction of the local rider and interpolation of others,
//! - the Bevy renderer from `ocw-render`.

mod automation;
mod client;
mod connection;
mod input;

use std::path::PathBuf;

use bevy::prelude::*;
use bevy::window::{PresentMode, WindowResolution};
use clap::Parser;
use ocw_core::SIM_TICK_HZ;
use ocw_render::console::{ConsoleState, Permission};
use ocw_render::{OcwRenderPlugins, RideView};
use ocw_world::Route;

#[derive(Parser, Debug, Clone, Resource)]
#[command(version, about = "Open Cycling World desktop client")]
pub struct Args {
    /// Display name of the local rider.
    #[arg(long, default_value = "Rider")]
    pub name: String,
    /// Initial power of the virtual trainer in watts.
    #[arg(long, default_value_t = 200.0)]
    pub power: f64,
    /// Number of test riders on the local server.
    #[arg(long, default_value_t = 4)]
    pub bots: u32,
    /// Window size as WIDTHxHEIGHT.
    #[arg(long, default_value = "1280x720", value_parser = parse_size)]
    pub window: (u32, u32),
    /// Console commands to run at startup (in addition to autoexec.cfg).
    #[arg(long = "exec", value_name = "FILE")]
    pub exec: Vec<PathBuf>,
    /// Run a console command at startup (repeatable).
    #[arg(long = "cmd", value_name = "COMMAND")]
    pub commands: Vec<String>,
    /// Save a screenshot to this path after --screenshot-delay seconds, then exit.
    #[arg(long, value_name = "PATH")]
    pub screenshot: Option<PathBuf>,
    /// Seconds of riding before the screenshot is taken.
    #[arg(long, default_value_t = 5.0)]
    pub screenshot_delay: f64,
}

fn parse_size(s: &str) -> Result<(u32, u32), String> {
    let (w, h) = s.split_once('x').ok_or("expected WIDTHxHEIGHT")?;
    Ok((
        w.parse().map_err(|_| "bad width")?,
        h.parse().map_err(|_| "bad height")?,
    ))
}

fn main() -> AppExit {
    let args = Args::parse();
    let route = Route::test_loop();
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "Open Cycling World".into(),
            resolution: WindowResolution::new(args.window.0, args.window.1),
            present_mode: PresentMode::AutoVsync,
            ..default()
        }),
        ..default()
    }))
    // The client prediction runs in FixedUpdate at the same rate as the server tick.
    .insert_resource(Time::<Fixed>::from_hz(f64::from(SIM_TICK_HZ)))
    .insert_resource(RideView::new(route.clone()))
    .insert_resource(args.clone())
    .add_plugins(OcwRenderPlugins)
    .add_plugins((
        connection::ConnectionPlugin { route },
        input::InputPlugin,
        automation::AutomationPlugin,
    ))
    .add_systems(Startup, startup_commands);
    app.run()
}

/// Enables dev commands (local server) and queues autoexec / --exec / --cmd.
fn startup_commands(args: Res<Args>, mut console: ResMut<ConsoleState>) {
    // Phase 1 always rides on the in-process local server.
    console.permission = Permission::Dev;
    if std::path::Path::new("autoexec.cfg").exists() {
        console.queue("exec autoexec.cfg");
    }
    for file in &args.exec {
        console.queue(format!("exec \"{}\"", file.display()));
    }
    for cmd in &args.commands {
        console.queue(cmd.clone());
    }
}
