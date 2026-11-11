//! Open Cycling World desktop client.
//!
//! Wires the pieces together (spec §6.1):
//! - a device backend (Phase 1: the virtual trainer) on its own thread,
//! - the in-process local server on its own thread at 50 Hz,
//! - client-side prediction of the local rider and interpolation of others,
//! - the Bevy renderer from `ocw-render`.

mod argsfile;
mod automation;
mod build_id;
mod client;
mod connection;
mod devices;
mod gps;
mod icon;
mod input;
mod known_devices;
mod logfile;
mod menu;
mod monitor;
mod platform;
mod recording;
mod restart;
mod strava;
mod trainer;
mod video;
mod workout;
mod worlds;

use std::path::PathBuf;

use bevy::prelude::*;
use bevy::window::{MonitorSelection, PresentMode, WindowMode, WindowResolution};
use clap::Parser;
use ocw_core::SIM_TICK_HZ;
use ocw_render::console::{ConsoleState, Permission};
use ocw_render::hud::VersionLabel;
use ocw_render::{OcwRenderPlugins, RideView};
use ocw_world::Route;

/// The built-in test routes.
fn test_route(name: &str) -> Option<Route> {
    match name {
        "hills" => Some(Route::test_hills()),
        "loop" => Some(Route::test_loop()),
        _ => None,
    }
}

impl Args {
    /// A route on the road graph: `--route-gpx`, or `--route NAME` (not a
    /// test route) with `--world`.
    pub fn graph_route(&self) -> bool {
        self.world.is_some() && (self.route_gpx.is_some() || test_route(&self.route).is_none())
    }
}

#[derive(Parser, Debug, Clone, Resource)]
// A later occurrence of an option replaces an earlier one, so the command
// line overrides the defaults file (argsfile.rs); repeatable options append.
#[command(
    version = build_id(),
    about = "Open Cycling World desktop client",
    args_override_self = true
)]
pub struct Args {
    /// Display name of the local rider.
    #[arg(long, default_value = "Rider")]
    pub name: String,
    /// Initial power of the virtual trainer in watts.
    #[arg(long, default_value_t = 200.0)]
    pub power: f64,
    /// Number of test riders on the local server.
    #[arg(long, default_value_t = 10)]
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
    /// Test route: hills (steep, changing gradients) or loop (gentle). With
    /// --world: a route of the package by name (`world-builder gpx` /
    /// `route --world`), or an `ocw-graph-route` file; ridden on the road
    /// graph (junction arrows show the route, another exit leaves it).
    #[arg(long, default_value = "hills")]
    pub route: String,
    /// Ride a GPX track or route (spec §2.7): matched onto the road graph of
    /// --world at the start; problems (off-road stretches) are reported.
    #[arg(long, value_name = "PATH", requires = "world")]
    pub route_gpx: Option<PathBuf>,
    /// Ride a route file from `world-builder route` (real roads and terrain).
    #[arg(long, value_name = "PATH")]
    pub route_file: Option<PathBuf>,
    /// World package: a file from `world-builder package`, or a package id
    /// from the local cache (`europe/liechtenstein` or `ID@VERSION`, see
    /// --list-worlds). Terrain, all roads and trees are streamed from it
    /// (use together with --route-file, or --free-ride). It is verified
    /// against its manifest on load (spec §4.7).
    #[arg(long, value_name = "PATH|ID")]
    pub world: Option<String>,
    /// Accept world packages that are not signed by a trusted key from the
    /// cache too (with a warning). Local files are accepted with a warning
    /// anyway.
    #[arg(long, conflicts_with = "require_signed")]
    pub allow_unsigned: bool,
    /// Refuse world packages that are not signed by a trusted key, also
    /// local files.
    #[arg(long)]
    pub require_signed: bool,
    /// Also trust world packages signed under this root public key file
    /// (repeatable; also every *.pub.json in ~/.config/ocw/trusted-roots/).
    #[arg(long, value_name = "FILE")]
    pub trust_root: Vec<PathBuf>,
    /// Trust the development root keys (always on in debug builds).
    #[arg(long)]
    pub trust_dev_root: bool,
    /// World package cache (default: $OCW_WORLD_CACHE or
    /// ~/.local/share/ocw/worlds).
    #[arg(long, value_name = "DIR")]
    pub world_cache: Option<PathBuf>,
    /// Download a world package into the cache (resumable, verified): the
    /// http(s) URL or local path of its manifest.json. Then exits, unless
    /// --world is given.
    #[arg(long, value_name = "URL")]
    pub fetch_world: Option<String>,
    /// List the world packages in the cache and exit.
    #[arg(long)]
    pub list_worlds: bool,
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
    /// Structured workout to ride in ERG mode (spec §2.8): a `.toml`
    /// workout, or an imported `.erg` / `.mrc` file. Loaded and started at
    /// once; the workout clock starts when you pedal.
    #[arg(long, value_name = "FILE")]
    pub workout: Option<PathBuf>,
    /// Functional threshold power in watts, for workout targets in % FTP.
    #[arg(long, default_value_t = 200.0, value_parser = parse_ftp)]
    pub ftp: f64,
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
    /// Ignore the default arguments file (ocw-desktop.args beside the
    /// executable, or $OCW_ARGS_FILE).
    #[arg(long)]
    pub no_args_file: bool,
}

fn parse_size(s: &str) -> Result<(u32, u32), String> {
    let (w, h) = s.split_once('x').ok_or("expected WIDTHxHEIGHT")?;
    Ok((
        w.parse().map_err(|_| "bad width")?,
        h.parse().map_err(|_| "bad height")?,
    ))
}

fn parse_ftp(s: &str) -> Result<f64, String> {
    match s.trim().parse::<f64>() {
        Ok(w) if (50.0..=600.0).contains(&w) => Ok(w),
        Ok(w) => Err(format!("{w} W: expected 50–600 W")),
        Err(e) => Err(format!("{s}: {e}")),
    }
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
            args.world.as_deref().unwrap_or("?")
        );
        std::process::exit(1);
    };
    let net =
        std::sync::Arc::new(ocw_world::RoadNetwork::new(graph).with_package_name(pkg.name.clone()));
    if args.graph_route() {
        let plan = graph_route_plan(args, pkg, &net).unwrap_or_else(|e| {
            eprintln!("{e}");
            std::process::exit(1);
        });
        println!(
            "route {}: {:.1} km, {} steps{}",
            plan.name,
            plan.length_m() / 1000.0,
            plan.steps.len(),
            if plan.is_loop { ", a loop" } else { "" }
        );
        let (edge, offset_m, forward) = plan.start();
        return connection::FreeRideSetup {
            origin: pkg.origin,
            net,
            start: ocw_sim::FreeRideStart {
                edge,
                offset_m,
                forward,
            },
            plan: Some(std::sync::Arc::new(plan)),
        };
    }
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
        plan: None,
    }
}

/// The route to ride on the road graph: a GPX file matched now, an
/// `ocw-graph-route` file, or a route of the package by name.
fn graph_route_plan(
    args: &Args,
    pkg: &ocw_world::WorldPackage,
    net: &ocw_world::RoadNetwork,
) -> Result<ocw_world::RoutePlan, String> {
    if let Some(path) = &args.route_gpx {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let gpx =
            ocw_world::gpx::Gpx::parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        let name = gpx.name.clone().unwrap_or_else(|| {
            path.file_stem()
                .map_or("GPX route".into(), |s| s.to_string_lossy().into_owned())
        });
        let report = ocw_world::matching::match_gpx(
            net,
            &gpx,
            pkg.origin,
            &name,
            &ocw_world::matching::MatchParams::default(),
        )
        .map_err(|e| format!("{}: {e}", path.display()))?;
        println!("{}", report.summary());
        return Ok(report.plan);
    }
    let file = std::path::Path::new(&args.route);
    if file.is_file() {
        let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
        let (route, package) =
            ocw_world::GraphRoute::parse(&text).map_err(|e| format!("{}: {e}", file.display()))?;
        if let Some((name, hash)) = package
            && hash != net.fingerprint()
        {
            eprintln!(
                "{}: made for {name} ({hash:016x}), this package is {} ({:016x}); trying the stable edge IDs",
                file.display(),
                pkg.name,
                net.fingerprint()
            );
        }
        return route.resolve(net).map_err(|e| {
            format!(
                "{}: the route does not fit this package's road graph: {e} (re-match it with world-builder gpx)",
                file.display()
            )
        });
    }
    let route = pkg.route(&args.route).ok_or_else(|| {
        let names: Vec<&str> = pkg.routes.iter().map(|r| r.name.as_str()).collect();
        format!(
            "no route {:?} in {} (it has: {}); add one with `world-builder gpx`",
            args.route,
            pkg.name,
            if names.is_empty() {
                "none".to_string()
            } else {
                names.join(", ")
            }
        )
    })?;
    route
        .resolve(net)
        .map_err(|e| format!("route {:?} does not fit the road graph: {e}", route.name))
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

/// This binary's build id (version, commit, profile; see `build_id`), shown
/// in the HUD and by `--version`.
pub fn build_id() -> &'static str {
    static ID: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        build_id::build_id(
            env!("CARGO_PKG_VERSION"),
            env!("OCW_COMMIT"),
            cfg!(debug_assertions),
        )
    });
    &ID
}

fn main() -> AppExit {
    let (argv, defaults) = argsfile::args_os();
    if let Some(msg) = &defaults {
        eprintln!("{msg}");
    }
    let args = Args::parse_from(argv);
    worlds::handle_cli(&args);
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
            test_route(&args.route).unwrap_or_else(|| {
                if args.world.is_none() {
                    eprintln!(
                        "--route {}: not a test route (hills, loop); package routes need --world",
                        args.route
                    );
                    std::process::exit(1);
                }
                Route::test_hills()
            }),
            None,
        ),
    };
    let mut app = App::new();
    if let Some(spec) = &args.world {
        let loaded = worlds::load(&args, spec);
        app.insert_resource(worlds::WorldInfo::of(spec, &loaded));
        let pkg = loaded.package;
        if args.free_ride || args.graph_route() {
            app.insert_resource(free_ride_setup(&args, &pkg));
        }
        app.insert_resource(ocw_render::package::PackageWorld(std::sync::Arc::new(pkg)));
    }
    let free_ride = app.world().contains_resource::<connection::FreeRideSetup>();
    app.add_plugins(
        DefaultPlugins
            // macOS app bundle: log to ~/Library/Logs/OCW (no terminal).
            .set(bevy::log::LogPlugin {
                fmt_layer: logfile::fmt_layer,
                ..default()
            })
            .set(WindowPlugin {
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
            }),
    )
    // The client prediction runs in FixedUpdate at the same rate as the server tick.
    .insert_resource(Time::<Fixed>::from_hz(f64::from(SIM_TICK_HZ)))
    .insert_resource(RideView {
        terrain_base,
        free_ride,
        ..RideView::new(route.clone())
    })
    .insert_resource(args.clone())
    .insert_resource(VersionLabel(build_id().to_string()))
    .add_systems(Startup, || info!("Open Cycling World {}", build_id()))
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
        restart::RestartPlugin,
        video::VideoPlugin,
        worlds::WorldsPlugin,
        menu::MenuPlugin,
        workout::WorkoutPlugin,
    ))
    .add_systems(Startup, startup_commands)
    .add_systems(Update, timed_commands);
    let exit = app.run();
    if restart::requested() {
        // Everything is shut down (ride saved, devices stopped): start afresh.
        drop(app);
        let e = restart::exec_restart();
        eprintln!("restart failed: {e}");
    }
    exit
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
    // Relative to the working directory, which is `/` for an app started
    // from Finder: the macOS app bundle does not read one.
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
    fn ten_bots_by_default() {
        assert_eq!(Args::parse_from(["ocw-desktop"]).bots, 10);
        assert_eq!(Args::parse_from(["ocw-desktop", "--bots", "2"]).bots, 2);
    }

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
        // A cache id works as --world too; the trust flags exclude each other.
        let a = Args::parse_from(["ocw-desktop", "--world", "europe/liechtenstein@3"]);
        assert_eq!(a.world.as_deref(), Some("europe/liechtenstein@3"));
        assert!(
            Args::try_parse_from(["ocw-desktop", "--allow-unsigned", "--require-signed"]).is_err()
        );
        assert!(parse_xz("1;2").is_err());
    }

    #[test]
    fn workout_arguments() {
        let a = Args::parse_from(["ocw-desktop"]);
        assert_eq!((a.ftp, a.workout.clone()), (200.0, None));
        assert_eq!(workout::ftp_w(&a), 200.0);
        let a = Args::parse_from(["ocw-desktop", "--workout", "w.erg", "--ftp", "265"]);
        assert_eq!(a.workout, Some(PathBuf::from("w.erg")));
        assert_eq!(workout::ftp_w(&a), 265.0);
        assert!(Args::try_parse_from(["ocw-desktop", "--ftp", "5"]).is_err());
        assert!(Args::try_parse_from(["ocw-desktop", "--ftp", "lots"]).is_err());
    }

    #[test]
    fn version_shows_the_build_id() {
        let id = build_id();
        assert!(id.starts_with(env!("CARGO_PKG_VERSION")));
        assert_eq!(id.ends_with(" (debug)"), cfg!(debug_assertions));
        let err = Args::try_parse_from(["ocw-desktop", "--version"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::DisplayVersion);
        assert!(err.to_string().contains(id));
    }

    #[test]
    fn route_arguments() {
        // Test routes stay test routes, also with a world package.
        let a = Args::parse_from(["ocw-desktop", "--route", "loop"]);
        assert!(!a.graph_route() && test_route(&a.route).is_some());
        let a = Args::parse_from(["ocw-desktop", "--world", "w.ocwpkg", "--route", "hills"]);
        assert!(!a.graph_route());
        // A package route by name, or a GPX file: rides on the road graph.
        let a = Args::parse_from(["ocw-desktop", "--world", "w.ocwpkg", "--route", "Rheintal"]);
        assert!(a.graph_route());
        let a = Args::parse_from([
            "ocw-desktop",
            "--world",
            "w.ocwpkg",
            "--route-gpx",
            "ride.gpx",
        ]);
        assert!(a.graph_route());
        assert!(Args::try_parse_from(["ocw-desktop", "--route-gpx", "ride.gpx"]).is_err());
    }

    #[test]
    fn command_line_overrides_the_args_file() {
        let file = argsfile::split(
            "--world /var/tmp/ocw/w.ocwpkg --virtual --name Kiosk\n--cmd 'hud on' --at 2=a\n",
        )
        .unwrap();
        let argv = |v: &[&str]| v.iter().map(std::ffi::OsString::from).collect::<Vec<_>>();
        let a = Args::parse_from(argsfile::merge(
            file.clone(),
            argv(&[
                "ocw-desktop",
                "--route-file",
                "r.route",
                "--name",
                "Nils",
                "--virtual",
                "--cmd",
                "teleport 3.3",
                "--at",
                "5=b",
            ]),
        ));
        // Single values: the command line wins; flags may repeat.
        assert_eq!(a.name, "Nils");
        assert_eq!(a.world.as_deref(), Some("/var/tmp/ocw/w.ocwpkg"));
        assert_eq!(a.route_file, Some(PathBuf::from("r.route")));
        assert!(a.virtual_trainer);
        // Repeatable options accumulate, the file's first.
        assert_eq!(a.commands, ["hud on", "teleport 3.3"]);
        assert_eq!(a.timed, [(2.0, "a".into()), (5.0, "b".into())]);
        let a = Args::parse_from(argsfile::merge(
            file,
            argv(&["ocw-desktop", "--world", "other.ocwpkg"]),
        ));
        assert_eq!(a.world.as_deref(), Some("other.ocwpkg"));
        assert_eq!(a.name, "Kiosk");
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
