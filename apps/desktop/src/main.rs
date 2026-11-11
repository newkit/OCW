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
mod devices;
mod input;
mod known_devices;
mod monitor;
mod platform;
mod trainer;

use std::path::PathBuf;

use bevy::prelude::*;
use bevy::window::{MonitorSelection, PresentMode, WindowMode, WindowResolution};
use clap::Parser;
use ocw_core::SIM_TICK_HZ;
use ocw_render::console::{ConsoleState, Permission};
use ocw_render::{OcwRenderPlugins, RideView};
use ocw_world::Route;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum RouteChoice {
    Hills,
    Loop,
}

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
    /// Borderless fullscreen on the current monitor (ignores --window).
    #[arg(long)]
    pub fullscreen: bool,
    /// Borderless fullscreen on this monitor: index (0, 1, ...), resolution
    /// (2560x1600) or name substring (HDMI). Detected monitors are logged.
    #[arg(long, value_name = "MONITOR")]
    pub monitor: Option<String>,
    /// Test route: hills (steep, changing gradients) or loop (gentle).
    #[arg(long, value_enum, default_value_t = RouteChoice::Hills)]
    pub route: RouteChoice,
    /// Ride a route file from `world-builder route` (real roads and terrain).
    #[arg(long, value_name = "PATH")]
    pub route_file: Option<PathBuf>,
    /// World package from `world-builder package`: terrain, all roads and
    /// trees are streamed from it (use together with --route-file).
    #[arg(long, value_name = "PATH")]
    pub world: Option<PathBuf>,
    /// Use the virtual trainer even if Bluetooth devices are remembered.
    #[arg(long = "virtual")]
    pub virtual_trainer: bool,
    /// File with remembered devices (default: ~/.config/ocw/devices.toml).
    #[arg(long, value_name = "PATH")]
    pub devices_file: Option<PathBuf>,
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
    /// Frame capture: save every frame as DIR/frame_NNNNN.png with game time
    /// advancing exactly 1/--capture-fps per frame (smooth videos even when
    /// rendering is slow), then exit. The server runs stepped, in-process.
    #[arg(long, value_name = "DIR", conflicts_with = "screenshot")]
    pub capture: Option<PathBuf>,
    /// Frames per second of game time during capture.
    #[arg(long, default_value_t = 25, value_parser = clap::value_parser!(u32).range(1..=120))]
    pub capture_fps: u32,
    /// Seconds of game time to capture.
    #[arg(long, default_value_t = 6.0)]
    pub capture_seconds: f64,
    /// Seconds of game time to ride before capturing starts.
    #[arg(long, default_value_t = 3.0)]
    pub capture_warmup: f64,
}

fn parse_size(s: &str) -> Result<(u32, u32), String> {
    let (w, h) = s.split_once('x').ok_or("expected WIDTHxHEIGHT")?;
    Ok((
        w.parse().map_err(|_| "bad width")?,
        h.parse().map_err(|_| "bad height")?,
    ))
}

/// Loads an ocw-route file: the route and the terrain around it.
fn load_route_file(path: &std::path::Path) -> (Route, Option<ocw_world::ElevationGrid>) {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("{}: {e}", path.display());
        std::process::exit(1);
    });
    let file = ocw_world::RouteFile::parse(&text).unwrap_or_else(|e| {
        eprintln!("{}: {e}", path.display());
        std::process::exit(1);
    });
    let route = Route::from_points(
        file.name.clone(),
        file.name.clone(),
        file.points.clone(),
        false,
        file.width_m,
    )
    .unwrap_or_else(|e| {
        eprintln!("{}: {e}", path.display());
        std::process::exit(1);
    });
    (route, file.grid)
}

fn main() -> AppExit {
    let args = Args::parse();
    let (route, terrain_base) = match &args.route_file {
        Some(path) => load_route_file(path),
        None => (
            match args.route {
                RouteChoice::Hills => Route::test_hills(),
                RouteChoice::Loop => Route::test_loop(),
            },
            None,
        ),
    };
    let mut app = App::new();
    if let Some(path) = &args.world {
        let bytes = std::fs::read(path).unwrap_or_else(|e| {
            eprintln!("{}: {e}", path.display());
            std::process::exit(1);
        });
        let pkg = ocw_world::WorldPackage::from_bytes(&bytes).unwrap_or_else(|e| {
            eprintln!("{}: {e}", path.display());
            std::process::exit(1);
        });
        app.insert_resource(ocw_render::package::PackageWorld(std::sync::Arc::new(pkg)));
    }
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "Open Cycling World".into(),
            resolution: WindowResolution::new(args.window.0, args.window.1),
            present_mode: PresentMode::AutoVsync,
            mode: if args.fullscreen {
                WindowMode::BorderlessFullscreen(MonitorSelection::Current)
            } else {
                WindowMode::Windowed
            },
            ..default()
        }),
        ..default()
    }))
    // The client prediction runs in FixedUpdate at the same rate as the server tick.
    .insert_resource(Time::<Fixed>::from_hz(f64::from(SIM_TICK_HZ)))
    .insert_resource(RideView {
        terrain_base,
        ..RideView::new(route.clone())
    })
    .insert_resource(args.clone())
    .add_plugins(OcwRenderPlugins)
    .add_plugins((
        connection::ConnectionPlugin { route },
        devices::DevicesPlugin,
        input::InputPlugin,
        automation::AutomationPlugin,
        platform::PlatformPlugin,
        monitor::MonitorPlugin,
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
