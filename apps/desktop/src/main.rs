//! Open Cycling World desktop client.
//!
//! Wires the pieces together (spec §6.1):
//! - a device backend (Phase 1: the virtual trainer) on its own thread,
//! - the in-process local server on its own thread at 50 Hz (or a remote
//!   server over QUIC with `--server`),
//! - client-side prediction of the local rider and interpolation of others,
//! - the Bevy renderer from `ocw-render`.

mod argsfile;
mod audio;
mod automation;
mod build_id;
mod client;
mod connection;
mod devices;
mod event;
mod ghost;
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
mod ride;
mod settings;
mod strava;
mod trainer;
mod video;
mod workout;
mod worlds;

use std::path::PathBuf;

use bevy::prelude::*;
use bevy::window::{MonitorSelection, PresentMode, WindowMode, WindowResolution};
use clap::{CommandFactory, FromArgMatches, Parser};
use ocw_core::SIM_TICK_HZ;
use ocw_render::console::{ConsoleState, Permission};
use ocw_render::hud::VersionLabel;
use ocw_render::{OcwRenderPlugins, RideView};
use ocw_world::Route;

impl Args {
    /// A route on the road graph: `--route-gpx`, or `--route NAME` (not a
    /// test route) with `--world`.
    pub fn graph_route(&self) -> bool {
        self.world.is_some()
            && (self.route_gpx.is_some() || ride::setup::test_route(&self.route).is_none())
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
    /// Display name of the local rider (default: the rider profile's).
    #[arg(long, default_value = "Rider")]
    pub name: String,
    /// Initial power of the virtual trainer in watts.
    #[arg(long, default_value_t = 200.0)]
    pub power: f64,
    /// Number of test riders on the local server.
    #[arg(long, default_value_t = 10)]
    pub bots: u32,
    /// Ride on a remote game server (`ocw-server --listen`) over QUIC
    /// instead of the in-process local server: HOST[:PORT] (default port
    /// 47800). Ride the same course as the server (--route / --world).
    #[arg(long, value_name = "ADDR", conflicts_with = "capture")]
    pub server: Option<String>,
    /// SHA-256 fingerprint of the server's certificate (ocw-server prints
    /// it at start); the connection is refused if it does not match.
    #[arg(long, value_name = "SHA256", requires = "server")]
    pub server_cert: Option<String>,
    /// Development only: accept any server certificate. Refused unless the
    /// server is on this machine (127.0.0.1 / ::1).
    #[arg(long, requires = "server", conflicts_with = "server_cert")]
    pub insecure_dev: bool,
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
    /// Ride the world package alone: do not join the installed
    /// neighbouring packages its roads continue into (spec §2.2.2).
    #[arg(long, requires = "world")]
    pub no_neighbours: bool,
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
    /// Functional threshold power in watts, for workout targets in % FTP
    /// (default: the rider profile's FTP, else 200 W).
    #[arg(long, value_parser = parse_ftp)]
    pub ftp: Option<f64>,
    /// Use the virtual trainer even if Bluetooth devices are remembered.
    #[arg(long = "virtual")]
    pub virtual_trainer: bool,
    /// File with remembered devices (default: ~/.config/ocw/devices.toml).
    #[arg(long, value_name = "PATH")]
    pub devices_file: Option<PathBuf>,
    /// Settings file (default: ~/.config/ocw/settings.toml); the rider
    /// profiles are read from riders.toml next to it. Command-line options
    /// override its values for the session.
    #[arg(long, value_name = "PATH")]
    pub settings_file: Option<PathBuf>,
    /// Ride as this rider profile (riders.toml) for this session, instead
    /// of the one chosen in the Workshop.
    #[arg(long, value_name = "NAME")]
    pub rider: Option<String>,
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
    /// Run a console command once the app has run SECONDS (game time; an
    /// in-app ride switch does not reset it), e.g. `--at "4=rider.lean 15"` (repeatable; for scripted rides).
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
    /// No sound: do not open an audio output (also off during --capture,
    /// whose game time does not run in real time).
    #[arg(long)]
    pub no_audio: bool,
}

impl Args {
    /// The audio output for this run (spec §3.3).
    pub fn audio_backend(&self) -> audio::Backend {
        if self.no_audio || self.capture.is_some() {
            audio::Backend::Null
        } else {
            audio::Backend::Device
        }
    }
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
    let matches = Args::command().get_matches_from(argv);
    let mut args = Args::from_arg_matches(&matches).unwrap_or_else(|e| e.exit());
    // settings.toml and riders.toml (spec §3.2.1): before anything reads
    // the options they fill in.
    let loaded = settings::load(&mut args, &matches);
    for w in &loaded.warnings {
        eprintln!("settings: {w}");
    }
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
    let setup = ride::setup::build(&args).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1);
    });
    let mut app = App::new();
    app.insert_resource(loaded.settings)
        .insert_resource(loaded.riders)
        .insert_resource(settings::LoadWarnings(loaded.warnings));
    let (route, terrain_base) = (setup.route.clone(), setup.terrain_base.clone());
    ride::insert_setup(app.world_mut(), setup);
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
            })
            // Bevy's own audio (rodio) only plays on the default device and
            // has no bus mixer; game audio is `audio::GameAudioPlugin`.
            .disable::<bevy::audio::AudioPlugin>(),
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
    .add_plugins(settings::SettingsPlugin)
    .add_plugins(audio::GameAudioPlugin {
        backend: args.audio_backend(),
    })
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
        ride::RidePlugin,
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
    time: Res<Time>,
    mut done: Local<usize>,
    mut console: ResMut<ConsoleState>,
) {
    let mut due: Vec<&(f64, String)> = args.timed.iter().collect();
    due.sort_by(|a, b| a.0.total_cmp(&b.0));
    while let Some((t, cmd)) = due.get(*done)
        && *t <= time.elapsed_secs_f64()
    {
        console.queue(cmd.clone());
        *done += 1;
    }
}

/// Enables dev commands (local server) and queues the settings' commands,
/// autoexec / --exec / --cmd (in this order: later ones win).
fn startup_commands(
    args: Res<Args>,
    settings: Option<Res<settings::UserSettings>>,
    mut console: ResMut<ConsoleState>,
) {
    // Dev commands only on the in-process local server (spec §2.9); a
    // remote server refuses them anyway.
    console.permission = if args.server.is_some() {
        Permission::User
    } else {
        Permission::Dev
    };
    if let Some(s) = settings {
        for line in s.settings.startup_commands() {
            console.queue(line);
        }
    }
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
    fn remote_server_options() {
        let a = Args::parse_from(["ocw-desktop"]);
        assert_eq!(a.server, None);
        let a = Args::parse_from([
            "ocw-desktop",
            "--server",
            "127.0.0.1:5000",
            "--insecure-dev",
        ]);
        assert_eq!(a.server.as_deref(), Some("127.0.0.1:5000"));
        assert!(a.insecure_dev);
        // Trust options need --server; a pin and --insecure-dev exclude
        // each other; capture runs stay local (stepped server).
        for bad in [
            &["ocw-desktop", "--insecure-dev"][..],
            &["ocw-desktop", "--server-cert", "ab"],
            &[
                "ocw-desktop",
                "--server",
                "h",
                "--server-cert",
                "ab",
                "--insecure-dev",
            ],
            &["ocw-desktop", "--server", "h", "--capture", "dir"],
        ] {
            assert!(Args::try_parse_from(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn audio_is_off_for_capture_and_no_audio() {
        let a = |v: &[&str]| Args::parse_from(v).audio_backend();
        assert_eq!(a(&["ocw-desktop"]), audio::Backend::Device);
        assert_eq!(a(&["ocw-desktop", "--no-audio"]), audio::Backend::Null);
        assert_eq!(
            a(&["ocw-desktop", "--capture", "frames"]),
            audio::Backend::Null
        );
    }

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
        assert_eq!((a.ftp, a.workout.clone()), (None, None));
        assert_eq!(workout::ftp_w(&a, None), 200.0);
        // The rider profile's FTP, unless --ftp is given.
        assert_eq!(workout::ftp_w(&a, Some(240)), 240.0);
        let a = Args::parse_from(["ocw-desktop", "--workout", "w.erg", "--ftp", "265"]);
        assert_eq!(a.workout, Some(PathBuf::from("w.erg")));
        assert_eq!(workout::ftp_w(&a, None), 265.0);
        assert_eq!(workout::ftp_w(&a, Some(240)), 265.0);
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
        assert!(!a.graph_route() && ride::setup::test_route(&a.route).is_some());
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
                "Alex",
                "--virtual",
                "--cmd",
                "teleport 3.3",
                "--at",
                "5=b",
            ]),
        ));
        // Single values: the command line wins; flags may repeat.
        assert_eq!(a.name, "Alex");
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
