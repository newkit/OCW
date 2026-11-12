//! Switching the ride in the running app (spec §3.2.1, M3): `ride.switch`
//! loads another course (test route, route file, world package in free
//! ride or on one of its routes) on a background thread while the old ride
//! goes on, with a loading notice in the HUD. When it is loaded, the
//! session is rebuilt in one frame:
//!
//! 1. the ride so far is saved as a FIT file (as on exit),
//! 2. the ride options in [`Args`] and the world resources (`WorldInfo`,
//!    `PackageWorld`, `FreeRideSetup`) are replaced, the `RideView` gets
//!    the new route,
//! 3. a new session starts ([`connection::start_session`]): the old local
//!    server's link is dropped (its thread ends), a new server and a new
//!    `ClientCore` join (the rider profile chosen now),
//! 4. the recorder starts a new ride and the render world is rebuilt
//!    ([`ocw_render::rebuild_scene`]).
//!
//! Devices (BLE trainer, sensors, gamepads), the trainer mode, a running
//! video, the camera, HUD, menu and console stay. `ride.import-gpx` matches
//! a GPX file onto the loaded package's roads and saves it as a route of the
//! user. [`catalog`] and [`pages`] are the Ride / Navi lists.

pub mod catalog;
pub mod choice;
pub mod pages;
pub mod setup;

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, TryRecvError};

use bevy::prelude::*;
use clap::{Arg, ArgAction, Command, value_parser};
use ocw_render::RideView;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, ConsoleState, Permission};

use crate::Args;
use crate::connection::{self, ClientState, FreeRideSetup};
use crate::recording::{MIN_AUTO_SAVE_S, Recording, RideRecorder};
use choice::{RideChoice, SwitchArgs, WorldMode};
use pages::{Catalog, LoadedWorld};
use setup::RideSetup;

/// The ride being loaded, for the HUD ("Loading … ").
#[derive(Resource, Default, Debug, Clone, PartialEq)]
pub struct Loading(pub Option<String>);

/// The ride now: what it is, its package's road network (GPX import).
#[derive(Resource, Clone)]
pub struct CurrentRide {
    pub choice: RideChoice,
    pub world: Option<CurrentWorld>,
    /// The ride was switched in the app: R restarts this ride in the app
    /// (the process' command line would start the first one again).
    pub switched: bool,
}

#[derive(Clone)]
pub struct CurrentWorld {
    pub spec: String,
    /// The package's own name (route files name it).
    pub name: String,
    pub routes: Vec<(String, f64)>,
    pub net: Option<std::sync::Arc<ocw_world::RoadNetwork>>,
    pub origin: (f64, f64),
}

/// The lists of the Ride and Navi pages, read when the menu opens.
#[derive(Resource, Default)]
pub struct RideCatalog(pub Catalog);

/// A switch being loaded: the options of the new ride and the result.
#[derive(Resource, Default)]
struct Pending(Option<Mutex<Receiver<Loaded>>>);

/// The options of a new ride and what loading them gave.
type Loaded = (Args, Result<RideSetup, String>);

/// Inserts the resources of a loaded ride (replacing or removing the old
/// ones): world info and render world, free ride, the current ride.
pub fn insert_setup(world: &mut World, setup: RideSetup) {
    let switched = world
        .get_resource::<CurrentRide>()
        .is_some_and(|c| c.switched);
    let mut current = CurrentRide {
        choice: setup.choice,
        world: None,
        switched,
    };
    match setup.free {
        Some(f) => world.insert_resource(f),
        None => {
            world.remove_resource::<FreeRideSetup>();
        }
    }
    match setup.world {
        Some(w) => {
            current.world = Some(CurrentWorld {
                spec: w.info.spec.clone(),
                name: w.render.home().name.clone(),
                routes: w.routes,
                net: w.net,
                origin: w.origin,
            });
            world.insert_resource(w.info);
            world.insert_resource(w.render);
        }
        None => {
            world.remove_resource::<crate::worlds::WorldInfo>();
            world.remove_resource::<ocw_render::package::PackageWorld>();
        }
    }
    world.insert_resource(current);
}

/// `<data dir>/ocw` (`OCW_DATA_DIR` overrides), as for the rides.
pub fn data_dir() -> Option<PathBuf> {
    crate::recording::default_rides_dir().and_then(|d| d.parent().map(Path::to_path_buf))
}

/// Where imported routes are saved: `<data dir>/ocw/routes`.
pub fn routes_dir() -> Option<PathBuf> {
    data_dir().map(|d| d.join("routes"))
}

/// Directories searched for world package files: next to the app (the
/// kiosk installs one there), `data/` and `data/pilot/` of the working
/// directory (development), the directory of the package ridden now.
fn world_dirs(args: &Args) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(d) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
    {
        dirs.push(d);
    }
    dirs.push(PathBuf::from("data"));
    dirs.push(PathBuf::from("data/pilot"));
    if let Some(w) = &args.world
        && let Some(parent) = Path::new(w).parent()
        && Path::new(w).is_file()
    {
        dirs.push(parent.to_path_buf());
    }
    dirs
}

/// GPX files offered for import: `<data dir>/ocw/gpx` and Downloads.
fn gpx_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = data_dir().map(|d| d.join("gpx")).into_iter().collect();
    dirs.extend(dirs::download_dir());
    dirs
}

/// Reads the Ride / Navi lists again (directories, cache, rides).
pub fn refresh_catalog(world: &mut World) {
    let args = world.resource::<Args>().clone();
    let current = world.get_resource::<CurrentRide>().cloned();
    let here = world.get_resource::<ClientState>().and_then(|c| {
        let c = &c.0;
        c.rider_id()?;
        let p = c.track().sample(c.state().distance).position;
        Some((p[0], p[2]))
    });
    let rides_dir = world
        .get_resource::<Recording>()
        .and_then(|r| r.dir.clone());
    let mut c = Catalog {
        worlds: catalog::known_worlds(&crate::worlds::cache(&args), &world_dirs(&args)),
        gpx: catalog::gpx_files(&gpx_dirs()),
        here,
        ..Catalog::default()
    };
    if let Some(dir) = &rides_dir {
        let sync =
            crate::strava::sync::SyncState::load(&dir.join("strava-sync.toml")).unwrap_or_default();
        c.rides = catalog::recent_rides(dir, &sync, 20);
    }
    if let Some(cur) = current {
        if let Some(w) = &cur.world {
            // The package ridden now is always listed.
            if !c.worlds.iter().any(|k| k.spec == w.spec) {
                c.worlds.insert(
                    0,
                    catalog::KnownWorld {
                        spec: w.spec.clone(),
                        name: choice::world_name(&w.spec),
                        detail: "loaded".into(),
                    },
                );
            }
            if let Some(dir) = routes_dir() {
                c.user_routes = catalog::user_routes(&dir, &w.name);
            }
            c.world = Some(LoadedWorld {
                spec: w.spec.clone(),
                name: w.name.clone(),
                routes: w.routes.clone(),
                has_graph: w.net.is_some(),
            });
        }
        c.current = Some(cur.choice);
    }
    world.insert_resource(RideCatalog(c));
}

pub struct RidePlugin;

impl Plugin for RidePlugin {
    fn build(&self, app: &mut App) {
        if !app.world().contains_resource::<CurrentRide>() {
            // Apps (tests) that set the ride up without `insert_setup`.
            let choice = RideChoice::from_args(app.world().resource::<Args>());
            app.insert_resource(CurrentRide {
                choice,
                world: None,
                switched: false,
            });
        }
        app.init_resource::<Loading>()
            .init_resource::<Pending>()
            .init_resource::<RideCatalog>()
            .add_systems(Startup, refresh_catalog)
            .add_systems(Update, (finish_switch, refresh_on_menu_open));
        register_commands(app);
    }
}

/// Reads the lists again when the menu opens.
fn refresh_on_menu_open(world: &mut World, mut was_open: Local<bool>) {
    let open = world
        .get_resource::<crate::menu::MenuState>()
        .is_some_and(|m| m.menu.is_open());
    if open && !*was_open {
        refresh_catalog(world);
    }
    *was_open = open;
}

/// Starts loading `choice` on a background thread (the ride goes on).
pub fn begin_switch(world: &mut World, choice: RideChoice) -> Result<String, String> {
    let args = world.resource::<Args>();
    if args.server.is_some() {
        return Err("riding on a remote server: the server chooses the course".into());
    }
    if world.resource::<Pending>().0.is_some() {
        return Err("another ride is being loaded".into());
    }
    let mut new = args.clone();
    choice.apply_to(&mut new);
    let what = choice.describe();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("ocw-ride-load".into())
        .spawn(move || {
            let result = setup::build(&new);
            let _ = tx.send((new, result));
        })
        .map_err(|e| format!("cannot start loading: {e}"))?;
    world.resource_mut::<Pending>().0 = Some(Mutex::new(rx));
    world.resource_mut::<Loading>().0 = Some(format!("Loading {what} ..."));
    info!("ride switch: loading {what}");
    Ok(format!("loading {what}"))
}

/// Applies a loaded switch (or reports why it failed; the ride goes on).
fn finish_switch(world: &mut World) {
    let result = {
        let mut pending = world.resource_mut::<Pending>();
        let Some(rx) = pending.0.as_ref() else {
            return;
        };
        let r = match rx.lock().expect("not poisoned").try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => (
                Args::default_for_error(),
                Err("the loading thread ended".into()),
            ),
        };
        pending.0 = None;
        r
    };
    world.resource_mut::<Loading>().0 = None;
    match result {
        (args, Ok(setup)) => {
            let what = setup.choice.describe();
            apply_switch(world, args, setup);
            info!("ride switch: riding {what}");
            world
                .resource_mut::<ConsoleState>()
                .print(format!("riding {what}"));
        }
        (_, Err(e)) => {
            warn!("ride switch: {e}");
            world
                .resource_mut::<ConsoleState>()
                .print(format!("ride switch failed: {e}"));
            if let Some(mut v) = world.get_resource_mut::<RideView>() {
                v.app_message = Some(format!("Could not load the ride: {}", first_line(&e)));
            }
        }
    }
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or(s)
}

impl Args {
    /// Only used when the loading thread died (never sent its options).
    fn default_for_error() -> Self {
        use clap::Parser;
        Args::parse_from(["ocw-desktop"])
    }
}

/// Saves the ride so far like on exit (rides under a minute are dropped).
fn save_ride(world: &mut World) {
    let Some(mut rec) = world.get_resource_mut::<Recording>() else {
        return;
    };
    if !rec.enabled {
        return;
    }
    match rec.save(MIN_AUTO_SAVE_S) {
        Ok(Some(path)) => {
            info!(path = %path.display(), "ride saved");
            if let Some(s) = world.get_resource::<crate::strava::Strava>() {
                s.ride_saved();
            }
            world
                .resource_mut::<ConsoleState>()
                .print(format!("ride saved to {}", path.display()));
        }
        Ok(None) => info!("ride too short, not saved"),
        Err(e) => error!("cannot save ride: {e}"),
    }
}

/// The session rebuild (see the module docs), in one frame.
pub fn apply_switch(world: &mut World, mut args: Args, setup: RideSetup) {
    // 1. The old ride is over.
    save_ride(world);
    // The rider chosen now (the Workshop's choice applies from the next
    // ride), unless the command line named one.
    if let Some(mut p) = world.get_resource_mut::<crate::settings::RiderProfiles>()
        && args.rider.is_none()
    {
        let active = p.riders.active();
        if args.name == p.riding.name {
            args.name.clone_from(&active.name);
        }
        p.riding = active;
    }
    world.insert_resource(args);
    // 2. The new course.
    let route = setup.route.clone();
    let terrain = setup.terrain_base.clone();
    insert_setup(world, setup);
    world.resource_mut::<CurrentRide>().switched = true;
    let free_ride = world.contains_resource::<FreeRideSetup>();
    if let Some(mut v) = world.get_resource_mut::<RideView>() {
        v.route = route.clone();
        v.terrain_base = terrain;
        v.free_ride = free_ride;
        v.riders.clear();
        v.paths.clear();
        v.junction = None;
        v.notice = None;
        v.route_status = None;
        v.position = None;
        // A running event ends with its session (server and client are new).
        v.event = None;
    }
    // 3. A new session (the old server thread ends with its link).
    connection::start_session(world, route);
    if world
        .get_resource::<crate::menu::MenuState>()
        .is_some_and(|m| m.menu.is_open())
    {
        world.resource_mut::<ClientState>().0.hold_steering();
    }
    // 4. A new recording, the new world drawn.
    let name = world.resource::<ClientState>().0.course_name().to_string();
    if let Some(mut rec) = world.get_resource_mut::<Recording>() {
        rec.recorder = RideRecorder::new(name);
    }
    ocw_render::rebuild_scene(world);
    refresh_catalog(world);
}

/// `ride.import-gpx FILE [--ride]`: matches the GPX file onto the loaded
/// package's road graph (spec §2.7) and saves it as an `ocw-graph-route`
/// file of the user (`<data dir>/ocw/routes/`), listed under Navi →
/// Routes; `--ride` rides it at once.
pub fn import_gpx(
    world: &mut World,
    file: &Path,
    dir: &Path,
    ride: bool,
) -> Result<String, String> {
    let cur = world.resource::<CurrentRide>();
    let w = cur.world.as_ref().ok_or(
        "no world package loaded: GPX routes are matched onto its roads (ride.switch --world PKG first)",
    )?;
    let net = w
        .net
        .as_ref()
        .ok_or("this world package has no road graph (format OCWPKG02 needed)")?;
    let report = setup::match_gpx_file(file, net, w.origin)?;
    let route = ocw_world::GraphRoute::from_plan(&report.plan, net);
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = catalog::route_file_name(dir, &route.name);
    std::fs::write(&path, route.to_text(Some((&w.name, net.fingerprint()))))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let spec = w.spec.clone();
    let mut out = format!("{}\nsaved as {}", report.summary(), path.display());
    if ride {
        let choice = RideChoice::World {
            world: spec,
            mode: WorldMode::Route(path.display().to_string()),
        };
        out.push('\n');
        out.push_str(&begin_switch(world, choice)?);
    } else {
        refresh_catalog(world);
    }
    Ok(out)
}

fn register_commands(app: &mut App) {
    app.console_command(ConsoleCommand::new(
        Permission::User,
        choice::command(),
        |world, m| {
            let choice = RideChoice::from_switch(&SwitchArgs::from_matches(m))?;
            begin_switch(world, choice)
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("ride.restart")
            .about("Start the current ride again in the app (new session and start point; the ride so far is saved)"),
        |world, _| {
            let choice = world.resource::<CurrentRide>().choice.clone();
            begin_switch(world, choice)
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("ride.current").about("What is ridden now, as a ride.switch line"),
        |world, _| {
            let c = world.resource::<CurrentRide>();
            Ok(format!("{}\n{}", c.choice.describe(), c.choice.to_line()))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("ride.import-gpx")
            .about("Match a GPX file onto the roads of the loaded world package and save it as your route (Navi -> Routes)")
            .arg(
                Arg::new("file")
                    .required(true)
                    .value_parser(value_parser!(PathBuf)),
            )
            .arg(
                Arg::new("ride")
                    .long("ride")
                    .action(ArgAction::SetTrue)
                    .help("Ride it at once"),
            ),
        |world, m| {
            let file = m.get_one::<PathBuf>("file").expect("required").clone();
            let dir = routes_dir().ok_or("no data directory for routes")?;
            import_gpx(world, &file, &dir, m.get_flag("ride"))
        },
    ));
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use clap::Parser;
    use ocw_render::console::ConsoleCorePlugin;
    use ocw_world::Route;

    use super::*;

    /// The desktop's ride plugins without a window, on the test loop,
    /// recording into a temporary directory.
    fn app(name: &str) -> (App, PathBuf) {
        let rides = std::env::temp_dir().join(format!("ocw-ride-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&rides);
        let args = Args::parse_from([
            "ocw-desktop",
            "--bots",
            "1",
            "--virtual",
            "--route",
            "loop",
            "--record",
            "true",
        ]);
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ConsoleCorePlugin))
            .init_resource::<ButtonInput<KeyCode>>()
            .insert_resource(args.clone())
            .insert_resource(RideView::new(Route::test_loop()));
        insert_setup(app.world_mut(), setup::build(&args).unwrap());
        app.add_plugins((
            connection::ConnectionPlugin {
                route: Route::test_loop(),
            },
            crate::devices::DevicesPlugin,
            crate::recording::RecordingPlugin {
                dir: Some(rides.clone()),
            },
            crate::menu::MenuPlugin,
            RidePlugin,
        ));
        app.world_mut().resource_mut::<ConsoleState>().permission = Permission::Dev;
        app.update();
        (app, rides)
    }

    fn run(app: &mut App, line: &str) -> String {
        app.world_mut().resource_mut::<ConsoleState>().queue(line);
        app.update();
        app.world().resource::<ConsoleState>().log().join("\n")
    }

    fn update_until(app: &mut App, done: impl Fn(&World) -> bool) -> bool {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(10) {
            app.update();
            if done(app.world()) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    fn client(w: &World) -> &crate::client::ClientCore {
        &w.resource::<ClientState>().0
    }

    #[test]
    fn switching_starts_a_new_session_and_keeps_the_devices() {
        let (mut app, rides) = app("switch");
        assert!(update_until(&mut app, |w| client(w).rider_id().is_some()
            && w.resource::<RideView>().riders.len() == 2));
        assert_eq!(
            client(app.world()).route().name(),
            Route::test_loop().name()
        );
        let samples = client(app.world()).stats().samples_out;
        // A minute and more of riding: the recording is saved on the switch.
        {
            let mut rec = app.world_mut().resource_mut::<Recording>();
            let track = Route::test_loop();
            for t in 0..70 {
                rec.recorder.observe(
                    &track,
                    None,
                    crate::recording::Sample {
                        game_s: f64::from(t),
                        distance: f64::from(t) * 8.0,
                        speed: 8.0,
                        power_w: 200.0,
                        ..Default::default()
                    },
                    1_700_000_000,
                );
            }
        }
        // A running event ends with the switch.
        run(&mut app, "event race --bots 2 --km 0.5");
        assert!(update_until(&mut app, |w| client(w).event_info().is_some()));
        let log = run(&mut app, "ride.switch --route hills");
        assert!(log.contains("loading test route hills"), "{log}");
        assert!(update_until(&mut app, |w| w
            .resource::<Loading>()
            .0
            .is_none()));
        let w = app.world();
        assert!(
            w.resource::<ConsoleState>()
                .log()
                .iter()
                .any(|l| l == "riding test route hills")
        );
        // The new course: route, view, options, current ride.
        assert_eq!(client(w).route().name(), Route::test_hills().name());
        assert_eq!(
            w.resource::<RideView>().route.name(),
            Route::test_hills().name()
        );
        assert_eq!(w.resource::<Args>().route, "hills");
        assert!(w.resource::<CurrentRide>().switched);
        assert_eq!(
            w.resource::<CurrentRide>().choice,
            RideChoice::TestRoute("hills".into())
        );
        // The old ride was saved; the new recording is empty.
        let saved = crate::strava::sync::ride_files(&rides);
        assert_eq!(saved.len(), 1, "{saved:?}");
        assert_eq!(
            crate::recording::load_meta(&rides.join(&saved[0]))
                .unwrap()
                .route,
            Route::test_loop().name()
        );
        assert!(w.resource::<Recording>().recorder.records().len() <= 1);
        // The new session: joined again, its bot appears, the ride time
        // starts again, the virtual trainer's samples still arrive.
        assert!(update_until(&mut app, |w| client(w).rider_id().is_some()
            && w.resource::<RideView>().riders.len() == 2));
        assert!(app.world().resource::<RideView>().elapsed_s < 5.0);
        let n0 = client(app.world()).stats().samples_out;
        assert!(update_until(&mut app, |w| client(w).stats().samples_out > n0 + 2));
        let _ = samples;
        assert!(
            app.world()
                .resource::<RideView>()
                .status
                .contains("trainer SIM"),
            "the trainer stays"
        );
        assert!(client(app.world()).event_info().is_none());
        assert!(app.world().resource::<RideView>().event.is_none());
        // The ride switch is scriptable; R now restarts this ride in the app.
        assert!(run(&mut app, "ride.current").contains("ride.switch --route hills"));
    }

    #[test]
    fn a_bad_switch_keeps_the_ride() {
        let (mut app, _) = app("bad");
        let missing = std::env::temp_dir().join("ocw-no-such.ocwpkg");
        let log = run(
            &mut app,
            &format!("ride.switch --world \"{}\"", missing.display()),
        );
        assert!(log.contains("loading"), "{log}");
        assert!(update_until(&mut app, |w| w
            .resource::<Loading>()
            .0
            .is_none()));
        let w = app.world();
        assert!(
            w.resource::<ConsoleState>()
                .log()
                .iter()
                .any(|l| l.starts_with("ride switch failed")),
            "{:?}",
            w.resource::<ConsoleState>().log()
        );
        assert_eq!(client(w).route().name(), Route::test_loop().name());
        assert!(!w.resource::<CurrentRide>().switched);
        assert!(w.resource::<RideView>().app_message.is_some());
        // Bad options are refused at once.
        let log = run(&mut app, "ride.switch --route Valley");
        assert!(log.contains("not a test route"), "{log}");
        // Without a world package there is nothing to match a GPX onto.
        let log = run(&mut app, "ride.import-gpx some.gpx");
        assert!(log.contains("no world package loaded"), "{log}");
    }

    #[test]
    fn the_menu_switches_after_asking() {
        let (mut app, _) = app("menu");
        for line in ["menu open", "menu down", "menu select"] {
            run(&mut app, line);
        }
        // Ride → Change ride → Test route: Hills → "Change the ride?" → Ride now.
        for line in [
            "menu up",
            "menu up",
            "menu select",
            "menu down",
            "menu select",
            "menu up",
            "menu select",
        ] {
            run(&mut app, line);
        }
        // Menu actions from the console apply in the next frame.
        app.update();
        app.update();
        assert!(
            !app.world()
                .resource::<crate::menu::MenuState>()
                .menu
                .is_open()
        );
        assert!(update_until(&mut app, |w| w
            .resource::<CurrentRide>()
            .switched
            && w.resource::<Loading>().0.is_none()));
        assert_eq!(
            client(app.world()).route().name(),
            Route::test_hills().name()
        );
    }

    #[test]
    fn a_remote_ride_cannot_switch() {
        let (mut app, _) = app("remote");
        app.world_mut().resource_mut::<Args>().server = Some("127.0.0.1:1".into());
        let log = run(&mut app, "ride.switch --route hills");
        assert!(log.contains("remote server"), "{log}");
    }
}
