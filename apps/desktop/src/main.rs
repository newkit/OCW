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
mod icon;
mod input;
mod known_devices;
mod monitor;
mod platform;
mod recording;
mod strava;
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
    /// trees are streamed from it (use together with --route-file, or
    /// --free-ride).
    #[arg(long, value_name = "PATH")]
    pub world: Option<PathBuf>,
    /// Free ride on the world package's road graph (spec §2.2.1): choose
    /// the exit at junctions (← / → or Q / E), U-turn with U. Needs
    /// --world with a version 2 package (it has the road graph).
    #[arg(long, requires = "world")]
    pub free_ride: bool,
    /// Free ride start: the road nearest local X,Z metres (x east, z
    /// south of the package origin). Default: a random road (or a fixed one
    /// for --screenshot / --capture runs; see --random-start).
    #[arg(long, value_name = "X,Z", value_parser = parse_xz, allow_hyphen_values = true)]
    pub start: Option<(f64, f64)>,
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
    /// Start at a random point of the route (everyone, bots too). Default:
    /// on, except for --screenshot / --capture runs (reproducible).
    #[arg(long, value_name = "BOOL", action = clap::ArgAction::Set)]
    pub random_start: Option<bool>,
    /// Record the ride and save it as a FIT file on exit (spec §2.8).
    /// Default: on, except for --screenshot / --capture runs.
    #[arg(long, value_name = "BOOL", action = clap::ArgAction::Set)]
    pub record: Option<bool>,
    /// Run a console command once the ride has lasted SECONDS (session
    /// time), e.g. `--at "4=rider.lean 15"` (repeatable; for scripted rides).
    #[arg(long = "at", value_name = "SECONDS=COMMAND", value_parser = parse_timed)]
    pub timed: Vec<(f64, String)>,
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

fn parse_xz(s: &str) -> Result<(f64, f64), String> {
    let (x, z) = s.split_once(',').ok_or("expected X,Z")?;
    let p = |v: &str| v.trim().parse::<f64>().map_err(|e| format!("{v}: {e}"));
    Ok((p(x)?, p(z)?))
}

/// Free ride: the road network of the package and where to start.
fn free_ride_setup(args: &Args, pkg: &ocw_world::WorldPackage) -> connection::FreeRideSetup {
    let Some(graph) = pkg.graph.clone() else {
        eprintln!(
            "{}: no road graph in this world package; rebuild it with `world-builder package` (format OCWPKG02)",
            args.world
                .as_deref()
                .unwrap_or(std::path::Path::new("?"))
                .display()
        );
        std::process::exit(1);
    };
    let net = std::sync::Arc::new(ocw_world::RoadNetwork::new(graph));
    let start = match args.start {
        Some((x, z)) => net
            .nearest([x, z])
            .map(|(edge, offset)| (edge, offset, net.edge(edge).oneway.allows(true))),
        None => {
            let random = args
                .random_start
                .unwrap_or(args.screenshot.is_none() && args.capture.is_none());
            let seed = if random {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos() as u64)
            } else {
                1
            };
            net.random_start(seed)
        }
    };
    let Some((edge, offset_m, forward)) = start else {
        eprintln!("the road graph is empty");
        std::process::exit(1);
    };
    let e = net.edge(edge);
    println!(
        "free ride from {} ({:?}), {:.0} m along edge {}",
        e.name.as_deref().unwrap_or("unnamed road"),
        e.class,
        offset_m,
        e.id.0
    );
    connection::FreeRideSetup {
        origin: pkg.origin,
        net,
        start: ocw_sim::FreeRideStart {
            edge,
            offset_m,
            forward,
        },
    }
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
    })
    .with_surfaces(file.surfaces.clone())
    .with_geo_origin(file.origin);
    (route, file.grid)
}

fn main() -> AppExit {
    let args = Args::parse();
    // Desktop entry and icons for menus, docks and file managers (not for
    // the headless screenshot / capture runs).
    #[cfg(target_os = "linux")]
    if args.screenshot.is_none() && args.capture.is_none() {
        match icon::install_desktop_integration() {
            Ok(0) => {}
            Ok(n) => info!("desktop integration: {n} files written"),
            Err(e) => warn!("desktop integration: {e}"),
        }
    }
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
        if args.free_ride {
            app.insert_resource(free_ride_setup(&args, &pkg));
        }
        app.insert_resource(ocw_render::package::PackageWorld(std::sync::Arc::new(pkg)));
    }
    let free_ride = app.world().contains_resource::<connection::FreeRideSetup>();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "Open Cycling World".into(),
            // App id (Wayland) / WM_CLASS (X11): the desktop entry and its icon.
            name: Some(icon::APP_ID.into()),
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
        free_ride,
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
        icon::IconPlugin,
    ))
    .add_plugins((
        recording::RecordingPlugin::default(),
        strava::StravaPlugin::default(),
    ))
    .add_systems(Startup, startup_commands)
    .add_systems(Update, timed_commands);
    app.run()
}

/// Where everyone starts on the route, in metres (`--random-start`).
pub fn start_point(args: &Args, route: &Route) -> f64 {
    let random = args
        .random_start
        .unwrap_or(args.screenshot.is_none() && args.capture.is_none());
    if !random {
        return 0.0;
    }
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    let start = random_start_m(route.length(), route.is_loop(), seed);
    info!("random start at {:.2} km", start / 1000.0);
    start
}

/// A start `seed` picks on a route of `length` m; on a route that ends,
/// at least 1 km before the end (or the start, if it is shorter).
pub fn random_start_m(length: f64, is_loop: bool, seed: u64) -> f64 {
    let usable = if is_loop {
        length
    } else {
        (length - 1000.0).max(0.0)
    };
    // SplitMix64 step: spreads close seeds (clock nanoseconds) evenly.
    let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    usable * (z >> 11) as f64 / (1u64 << 53) as f64
}

/// `SECONDS=COMMAND` of `--at`.
fn parse_timed(s: &str) -> Result<(f64, String), String> {
    let (t, cmd) = s.split_once('=').ok_or("expected SECONDS=COMMAND")?;
    let t: f64 = t.trim().parse().map_err(|e| format!("{t}: {e}"))?;
    if t.is_nan() || t < 0.0 || cmd.trim().is_empty() {
        return Err("expected SECONDS >= 0 and a command".into());
    }
    Ok((t, cmd.trim().to_string()))
}

/// Queues the `--at` commands whose time has come (each once).
fn timed_commands(
    args: Res<Args>,
    view: Res<RideView>,
    mut done: Local<usize>,
    mut console: ResMut<ConsoleState>,
) {
    let mut due: Vec<&(f64, String)> = args.timed.iter().collect();
    due.sort_by(|a, b| a.0.total_cmp(&b.0));
    while let Some((t, cmd)) = due.get(*done)
        && *t <= view.elapsed_s
    {
        console.queue(cmd.clone());
        *done += 1;
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_ride_arguments() {
        let a = Args::parse_from([
            "ocw-desktop",
            "--world",
            "w.ocwpkg",
            "--free-ride",
            "--start",
            "-1200.5,5300",
        ]);
        assert!(a.free_ride);
        assert_eq!(a.start, Some((-1200.5, 5300.0)));
        // --free-ride needs a world package.
        assert!(Args::try_parse_from(["ocw-desktop", "--free-ride"]).is_err());
        assert!(parse_xz("1;2").is_err());
    }

    #[test]
    fn timed_commands_parse() {
        assert_eq!(
            parse_timed("4.5=rider.lean -15"),
            Ok((4.5, "rider.lean -15".to_string()))
        );
        assert!(parse_timed("rider.lean 15").is_err());
        assert!(parse_timed("-1=x").is_err());
        assert!(parse_timed("2=  ").is_err());
    }

    #[test]
    fn random_starts_spread_over_the_route() {
        let starts: Vec<f64> = (0..200)
            .map(|s| random_start_m(13_000.0, false, s))
            .collect();
        assert!(starts.iter().all(|&s| (0.0..=12_000.0).contains(&s)));
        // Neighbouring seeds land far apart; the whole route is used.
        assert!((starts[0] - starts[1]).abs() > 100.0);
        assert!(starts.iter().any(|&s| s < 2000.0) && starts.iter().any(|&s| s > 10_000.0));
        // Short open routes start at the beginning; loops anywhere.
        assert_eq!(random_start_m(800.0, false, 7), 0.0);
        assert!(random_start_m(800.0, true, 7) > 0.0);
    }
}
