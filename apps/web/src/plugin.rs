//! Bevy glue of the browser app (spec §14 Phase 10, B1 / B3): moves the
//! frame time into the [`WebSession`] (the local server or the remote
//! link, and the virtual trainer), runs the client prediction at 50 Hz in
//! `FixedUpdate`, reads the keys and writes the [`RideView`] the shared
//! renderer (`ocw-render`) draws.

use bevy::prelude::*;
use clap::{Arg, Command, value_parser};
use ocw_client::client::Notice;
use ocw_client::input::{KeyThrottle, LINE_KEY_RATE_MPS, ThrottleKeys, lean_ramp, steer_direction};
use ocw_protocol::v1::{self, client_message::Msg as C};
use ocw_protocol::{client_message, encode};
use ocw_render::RideView;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, ConsoleState, Permission};
use ocw_render::menu::ride_input;
use ocw_sky::Zone;

use crate::session::{RemoteOptions, SessionConfig, Transport, WebSession};

/// The ride session (server, client, trainer) as a Bevy resource.
#[derive(Resource)]
pub struct Session(pub WebSession);

/// The time zone of the ride (the test routes: the default place).
#[derive(Resource)]
pub struct Place(pub Zone);

/// Makes the transport to a remote server (`?server=`, B3): the browser's
/// WebTransport (`crate::browser`), a fake in the tests. Without it a
/// remote config rides on the local server.
#[derive(Resource)]
pub struct MakeTransport(pub TransportFactory);

/// Makes a transport for a remote server's options.
pub type TransportFactory = Box<dyn Fn(&RemoteOptions) -> Box<dyn Transport> + Send + Sync>;

impl MakeTransport {
    /// The transport for the config's server, if it has one.
    pub fn for_config(world: &World, config: &SessionConfig) -> Option<Box<dyn Transport>> {
        let remote = config.remote.as_ref()?;
        let make = world.get_resource::<MakeTransport>()?;
        Some((make.0)(remote))
    }
}

/// The link's numbers as JSON for the page (`window.ocw.netStats()`).
static NET_STATS: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// The last published connection numbers (JSON; empty before the ride).
pub fn net_stats_json() -> String {
    NET_STATS.lock().expect("not poisoned").clone()
}

/// Seconds between the server's progress lines in the log (the headless
/// browser test reads them from the console).
pub const TICK_LOG_INTERVAL_S: f64 = 2.0;

pub struct WebSessionPlugin {
    pub config: SessionConfig,
}

impl Plugin for WebSessionPlugin {
    fn build(&self, app: &mut App) {
        match &self.config.world {
            // A world package (B2): the session starts when its first
            // tiles are there (`crate::world`).
            Some(options) => {
                app.add_plugins(crate::world::WebWorldPlugin {
                    options: options.clone(),
                    session: self.config.clone(),
                });
            }
            None => {
                let (zone, _) = ocw_client::sky::place_zone(None, None);
                let transport = MakeTransport::for_config(app.world(), &self.config);
                if let Some(r) = &self.config.remote {
                    if transport.is_some() {
                        info!(url = %r.url, "connecting to the remote server (WebTransport)");
                    } else {
                        warn!(url = %r.url, "no WebTransport here: riding on the local server");
                    }
                }
                app.insert_resource(Session(WebSession::new(self.config.clone(), transport)))
                    .insert_resource(Place(zone));
            }
        }
        // Dev commands (teleport …) only on the local server; a remote
        // server refuses them anyway.
        if self.config.remote.is_none() {
            app.add_systems(Startup, enable_dev_commands);
        }
        app.add_systems(PreUpdate, frame.run_if(riding))
            .add_systems(Last, (flush, publish_stats).run_if(riding))
            .add_systems(FixedUpdate, predict.run_if(resource_exists::<Session>))
            .add_systems(
                Update,
                (keys.run_if(ride_input), send_steer, update_view)
                    .chain()
                    .run_if(riding),
            )
            .add_systems(Update, crate::console_from_page);
        register_commands(app);
    }
}

/// A ride is running (with a world package only once it is loaded).
fn riding(session: Option<Res<Session>>, place: Option<Res<Place>>) -> bool {
    session.is_some() && place.is_some()
}

/// The local server allows dev commands (teleport …), as on the desktop.
fn enable_dev_commands(mut console: ResMut<ConsoleState>) {
    console.permission = Permission::Dev;
}

/// The frame's time into the session; the server's notices to the console.
fn frame(
    time: Res<Time>,
    mut session: ResMut<Session>,
    mut console: ResMut<ConsoleState>,
    mut next_log: Local<f64>,
) {
    let now = time.elapsed_secs_f64();
    let remote = session.0.is_remote();
    for notice in session.0.frame(time.delta_secs_f64(), now) {
        match notice {
            Notice::Joined { rider_id, route_id } if remote => {
                info!(rider = %rider_id, route = %route_id, "joined the remote server");
            }
            Notice::Joined { rider_id, route_id } => {
                info!(rider = %rider_id, route = %route_id, "joined the local server");
            }
            Notice::Rejected(reason) => {
                error!(%reason, "server rejected us");
                console.print(format!("server rejected connection: {reason}"));
            }
            Notice::CommandResult(text) => console.print(text),
            Notice::God { message, .. } => console.print(message),
            Notice::Voice(_) | Notice::WorldChange(_) => {}
        }
    }
    if now >= *next_log {
        *next_log = now + TICK_LOG_INTERVAL_S;
        let s = &session.0;
        let state = s.client.state();
        if let Some(r) = s.remote_link() {
            let t = r.timing.summary();
            let status = s.link_notice().unwrap_or_else(|| "connected".into());
            info!(
                snapshots = t.snapshots,
                others = r.others_seen.len(),
                acked_seq = r.acked_seq,
                distance_m = format!("{:.1}", state.distance),
                speed_kmh = format!("{:.1}", ocw_core::mps_to_kmh(state.speed)),
                power_w = s.power_w(),
                jitter_ms = format!("{:.1}", t.jitter_ms),
                %status,
                "remote server running"
            );
            return;
        }
        info!(
            ticks = s.ticks,
            distance_m = format!("{:.1}", state.distance),
            speed_kmh = format!("{:.1}", ocw_core::mps_to_kmh(state.speed)),
            power_w = s.power_w(),
            "local server running"
        );
    }
}

/// Sends what the frame's systems queued for a remote server (steering,
/// commands) without waiting for the next frame.
fn flush(time: Res<Time>, mut session: ResMut<Session>) {
    session.0.flush(time.elapsed_secs_f64());
}

/// Publishes the link's numbers for `window.ocw.netStats()` (twice a
/// second).
fn publish_stats(time: Res<Time>, session: Res<Session>, mut next: Local<f64>) {
    let now = time.elapsed_secs_f64();
    if now < *next {
        return;
    }
    *next = now + 0.5;
    *NET_STATS.lock().expect("not poisoned") = session.0.stats_json();
}

/// Client-side prediction of the own rider at the server's tick rate.
fn predict(mut session: ResMut<Session>) {
    session.0.client.predict_step();
}

/// The desktop's ride keys (spec §3.2.2; the gamepad follows in B6): ↑
/// adds keyboard power, ↓ stops, Page Up / Down ±10 W, A autodrive,
/// W wild mode, ← / → the line (autodrive) or the lean.
fn keys(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut session: ResMut<Session>,
    mut throttle: Local<KeyThrottle>,
    mut upright_pause: Local<f64>,
) {
    let s = &mut session.0;
    let dt = time.delta_secs_f64();
    let power_keys = ThrottleKeys {
        up_pressed: keys.just_pressed(KeyCode::ArrowUp),
        up_held: keys.pressed(KeyCode::ArrowUp),
        down_pressed: keys.just_pressed(KeyCode::ArrowDown),
        page_up: keys.just_pressed(KeyCode::PageUp),
        page_down: keys.just_pressed(KeyCode::PageDown),
    };
    s.throttle(&mut throttle, power_keys, dt, time.elapsed_secs_f64());
    if keys.just_pressed(KeyCode::KeyA) {
        let on = !s.client.autodrive();
        s.client.set_autodrive(on);
        info!("autodrive {}", if on { "on" } else { "off" });
    }
    if keys.just_pressed(KeyCode::KeyW) {
        let wild = !s.client.wild();
        s.client.set_wild(wild);
        info!("wild mode {}", if wild { "on" } else { "off" });
    }
    let dir = steer_direction(
        keys.pressed(KeyCode::ArrowLeft),
        keys.pressed(KeyCode::ArrowRight),
    );
    if s.client.autodrive() {
        if dir != 0.0 {
            s.client.steer_by(dir * LINE_KEY_RATE_MPS * dt);
        }
        return;
    }
    let lean = s.client.lean().unwrap_or(0.0);
    let next = lean_ramp(lean, dir, &mut upright_pause, dt);
    if dir != 0.0 && next != lean {
        s.client.set_lean(next);
    }
}

/// Sends the steering intent when it changed.
fn send_steer(time: Res<Time>, mut session: ResMut<Session>) {
    let s = &mut session.0;
    if let Some(msg) = s.client.steer_message(time.elapsed_secs_f64()) {
        s.send(&msg);
    }
    if let Some(msg) = s.client.turn_message() {
        s.send(&msg);
    }
}

/// The ride for the renderer: riders (interpolated between the 50 Hz
/// prediction steps), the event, the sky and the status line.
fn update_view(
    time: Res<Time>,
    fixed: Res<Time<Fixed>>,
    place: Res<Place>,
    mut session: ResMut<Session>,
    mut view: ResMut<RideView>,
) {
    let now = time.elapsed_secs_f64();
    let s = &mut session.0;
    view.elapsed_s = now;
    view.riders = s.client.view(now, fixed.overstep_fraction_f64());
    view.position = view
        .local()
        .and_then(|l| ocw_client::gps::position(s.client.track(), s.client.geo_origin(), &l.state));
    view.event = s.client.event_view(now);
    view.ghost = s.client.ghost_state(now);
    // A ride on a world package's road graph (B2): every rider's path,
    // the junction ahead, the route's progress (as on the desktop).
    let mut notice = None;
    if s.client.is_free_ride() {
        view.paths = s.client.paths();
        view.junction = s.client.junction_view();
        view.route_status = s.client.route_status();
        notice = s.client.notice();
    }
    // A remote link that is not up matters most (as on the desktop);
    // then collisions (spec §14 Phase 12C): the crash line, else the
    // crash feed.
    view.notice = s
        .link_notice()
        .or_else(|| s.client.crash_notice(now))
        .or(notice)
        .or_else(|| s.client.crash_feed(now).pop());
    view.status = s.status_line();
    let position = view.position;
    view.sky = ocw_client::sky::sky_view(
        &mut s.client,
        now,
        &place.0,
        position,
        ocw_sky::light::DEFAULT_NIGHT_FLOOR,
    );
}

/// Before a streamed world is loaded there is no ride yet.
const NO_RIDE: &str = "no ride yet (the world is still loading)";

fn session_mut(world: &mut World) -> Result<&mut WebSession, String> {
    world
        .get_resource_mut::<Session>()
        .map(|s| &mut s.into_inner().0)
        .ok_or_else(|| NO_RIDE.to_string())
}

/// `time …`: the time machine, as on the desktop (spec §14 Phase 9; the
/// browser test sets noon for its screenshots).
fn time_command(world: &mut World, args: &[&str]) -> Result<String, String> {
    use ocw_sky::machine::{self, ClockChange, TimeCommand};
    let cmd = machine::parse(args)?;
    let now = world.resource::<Time>().elapsed_secs_f64();
    let zone = world.get_resource::<Place>().ok_or(NO_RIDE)?.0.clone();
    let position = world.resource::<RideView>().position;
    let s = session_mut(world)?;
    let latlon = position
        .map(|p| (p.0, p.1))
        .or(s.client.geo_origin())
        .unwrap_or(ocw_sky::zone::DEFAULT_PLACE);
    let ms = s
        .client
        .world_ms(now)
        .ok_or("no world clock yet (the server sends it when you join)")?;
    if cmd == TimeCommand::Show {
        return Ok(format!(
            "world clock {ms} ms since 1970 (zone {})",
            zone.id()
        ));
    }
    let change =
        machine::resolve(&cmd, ms, &zone, latlon.0, latlon.1)?.ok_or("nothing to change")?;
    let msg = ocw_client::client::ClientCore::clock_message(change);
    s.send(&msg);
    Ok(match change {
        ClockChange::Fixed(_) => format!("time: {}", args.join(" ")),
        _ => String::new(),
    })
}

fn register_commands(app: &mut App) {
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("time")
            .about("World time at your place in the world, and the time machine")
            .long_about(ocw_sky::machine::USAGE)
            .arg(
                Arg::new("args")
                    .num_args(0..)
                    .allow_hyphen_values(true)
                    .action(clap::ArgAction::Append),
            ),
        |world, m| {
            let args: Vec<&str> = m
                .get_many::<String>("args")
                .map(|v| v.map(String::as_str).collect())
                .unwrap_or_default();
            time_command(world, &args)
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("world.info")
            .about("Show the streamed world: loading, tiles, memory, storage"),
        |world, _| {
            let s = world
                .get_resource::<crate::world::WorldStream>()
                .ok_or("no world package (the page has no ?world=)")?;
            let mut lines = s.screen.clone();
            if let Some(id) = &s.identity {
                lines.insert(0, format!("package {id}"));
            }
            lines.push(s.status_line(crate::wasm_memory_bytes()));
            Ok(lines.join("\n"))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("sim.power")
            .about("Set the virtual trainer's power (W)")
            .arg(
                Arg::new("watts")
                    .required(true)
                    .value_parser(value_parser!(f64)),
            ),
        |world, m| {
            let w = *m.get_one::<f64>("watts").expect("required");
            let set = session_mut(world)?.set_power(w);
            Ok(format!("virtual power {set:.0} W"))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::Dev,
        Command::new("teleport")
            .about("Move the local rider to a distance on the route (km)")
            .arg(
                Arg::new("km")
                    .required(true)
                    .value_parser(value_parser!(f64)),
            ),
        |world, m| {
            let km = *m.get_one::<f64>("km").expect("required");
            let msg = encode(&client_message(C::Dev(v1::DevCommand {
                cmd: Some(v1::dev_command::Cmd::TeleportDistanceM(km * 1000.0)),
            })));
            session_mut(world)?.send(&msg);
            // The server confirms; the prediction snaps on the next snapshot.
            Ok(String::new())
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("net.stats").about("Show connection statistics"),
        |world, _| {
            Ok(world
                .get_resource::<Session>()
                .ok_or(NO_RIDE)?
                .0
                .stats_text())
        },
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_render::console::ConsoleCorePlugin;

    /// The glue without a window or GPU: the session runs from Bevy's
    /// frame time, the view gets the riders and the sky, and the console
    /// commands reach the session.
    #[test]
    fn the_plugin_rides_headless() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::input::InputPlugin, ConsoleCorePlugin))
            .insert_resource(Time::<Fixed>::from_hz(f64::from(ocw_core::SIM_TICK_HZ)))
            .insert_resource(RideView::new(ocw_world::Route::test_hills()))
            .add_plugins(WebSessionPlugin {
                config: SessionConfig::default(),
            });
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_millis(800) {
            app.update();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let session = &app.world().resource::<Session>().0;
        assert!(session.ticks > 20, "{}", session.ticks);
        assert!(session.client.rider_id().is_some());
        let view = app.world().resource::<RideView>();
        assert_eq!(view.riders.len(), 1 + crate::session::DEFAULT_BOTS as usize);
        assert!(view.sky.is_some());
        assert!(view.status.starts_with("Virtual trainer 200 W"));
        app.world_mut()
            .resource_mut::<ConsoleState>()
            .queue("sim.power 260");
        app.update();
        app.update();
        assert_eq!(app.world().resource::<Session>().0.power_w(), 260.0);
    }

    /// A remote session (B3) through the plugin: the transport comes from
    /// [`MakeTransport`] (here the fake over an in-memory server), the HUD
    /// says "Connecting" until the link is up, the view gets the server's
    /// riders, no dev commands, `net.stats` and the page's JSON.
    #[test]
    fn the_plugin_rides_on_a_remote_server_headless() {
        use crate::session::tests::{FakeNet, FakeTransport};
        let game = std::sync::Arc::new(std::sync::Mutex::new(ocw_server::GameServer::new(
            ocw_world::Route::test_hills(),
            ocw_server::ServerConfig {
                bots: 2,
                ..Default::default()
            },
        )));
        let g = game.clone();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::input::InputPlugin, ConsoleCorePlugin))
            .insert_resource(Time::<Fixed>::from_hz(f64::from(ocw_core::SIM_TICK_HZ)))
            .insert_resource(RideView::new(ocw_world::Route::test_hills()))
            .insert_resource(MakeTransport(Box::new(move |_| {
                let mut t = FakeTransport::new(g.clone(), FakeNet::Normal);
                // Up after the first frames: "Connecting" shows first.
                t.connect_after_s = 0.2;
                Box::new(t)
            })))
            .add_plugins(WebSessionPlugin {
                config: SessionConfig {
                    name: "Browser".into(),
                    remote: Some(RemoteOptions {
                        url: "https://127.0.0.1:47801/ocw/1".into(),
                        cert_hashes: Vec::new(),
                    }),
                    ..SessionConfig::default()
                },
            });
        app.update();
        assert!(app.world().resource::<Session>().0.is_remote());
        assert_eq!(
            app.world().resource::<RideView>().notice.as_deref(),
            Some("Connecting to the server …")
        );
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_millis(1500) {
            app.update();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let view = app.world().resource::<RideView>();
        assert_eq!(view.riders.len(), 3, "the browser and two bots");
        assert!(view.notice.is_none(), "{:?}", view.notice);
        assert_eq!(
            app.world().resource::<ConsoleState>().permission,
            Permission::User
        );
        assert!(
            game.lock()
                .unwrap()
                .session()
                .snapshot()
                .iter()
                .any(|r| r.name == "Browser" && r.power_w == 200.0)
        );
        // The page's JSON (published twice a second; other tests publish
        // too, so only its shape is checked here).
        let json: serde_json::Value = serde_json::from_str(&net_stats_json()).unwrap();
        assert!(json.get("remote").is_some());
        let own: serde_json::Value =
            serde_json::from_str(&app.world().resource::<Session>().0.stats_json()).unwrap();
        assert_eq!(own["others_seen"].as_array().unwrap().len(), 2);
    }

    /// A world app (B2) over an in-memory server and OPFS stand-in.
    fn world_app(store: std::sync::Arc<ocw_worldstore::stream::MemoryStore>) -> App {
        use crate::world::tests::{driver, options, server};
        let opts = options();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::input::InputPlugin, ConsoleCorePlugin))
            .insert_resource(Time::<Fixed>::from_hz(f64::from(ocw_core::SIM_TICK_HZ)))
            .insert_resource(RideView::new(ocw_world::Route::test_hills()))
            .insert_resource(driver(server(), store, &opts))
            .add_plugins(WebSessionPlugin {
                config: SessionConfig {
                    world: Some(opts),
                    bots: 1,
                    power_w: 400.0,
                    ..SessionConfig::default()
                },
            });
        app
    }

    fn run_for(app: &mut App, ms: u64) {
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_millis(ms) {
            app.update();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// Worlds over HTTP, headless: the package streams in, the ride
    /// starts on its route once the first tiles are there, a teleport
    /// brings new tiles and drops the far ones (memory budget), and the
    /// second load comes from the cache.
    #[test]
    fn a_streamed_world_loads_rides_and_streams_tiles() {
        use crate::world::WorldStream;
        use ocw_render::package::PackageWorld;
        let store = std::sync::Arc::new(ocw_worldstore::stream::MemoryStore::default());
        let mut app = world_app(store.clone());
        app.update();
        // Before the world is there the commands say so.
        app.world_mut()
            .resource_mut::<ConsoleState>()
            .queue("sim.power 300");
        run_for(&mut app, 300);
        assert!(
            app.world().contains_resource::<Session>(),
            "the ride started"
        );
        let pw = app.world().resource::<PackageWorld>();
        // Fetch radius 1 at tile 0,0 of the 5 × 2 package: 4 tiles.
        assert_eq!(pw.home().tiles.len(), 4);
        assert!(pw.home().graph.is_some());
        assert!(app.world().resource::<RideView>().free_ride);
        {
            let s = app.world().resource::<WorldStream>();
            assert!(s.started);
            assert_eq!(s.identity.as_deref(), Some("test/strip@2"));
            let p = &s.shared.lock().unwrap().progress;
            assert_eq!(p.stats.cache_bytes, 0);
            assert!(p.stats.network_bytes > 0);
            assert!(p.first_tiles_ms.is_some());
        }
        run_for(&mut app, 500);
        let session = &app.world().resource::<Session>().0;
        assert!(session.client.state().distance > 0.5, "it rides");
        assert!(app.world().resource::<RideView>().route_status.is_some());
        // Teleport 3.5 km along the route (x ≈ 3600, tile 3): tiles 2..4
        // come, tile 0 (ring 3 > keep 2) is dropped.
        app.world_mut()
            .resource_mut::<ConsoleState>()
            .queue("teleport 3.5");
        run_for(&mut app, 1500);
        {
            let s = app.world().resource::<WorldStream>();
            let r = s.resident.as_ref().unwrap();
            assert!(r.dropped_total >= 2, "{}", s.status_line(None));
            assert!(r.is_resident(ocw_world::TileKey { x: 4, z: 0 }));
            assert!(!r.is_resident(ocw_world::TileKey { x: 0, z: 0 }));
            assert!(r.peak_bytes() >= r.resident_bytes());
        }
        let home = app.world().resource::<PackageWorld>().home().clone();
        assert!(home.tiles.contains_key(&ocw_world::TileKey { x: 4, z: 0 }));
        assert!(!home.tiles.contains_key(&ocw_world::TileKey { x: 0, z: 0 }));
        app.world_mut()
            .resource_mut::<ConsoleState>()
            .queue("world.info");
        app.update();
        // The second visit: base and tiles from the cache.
        let mut again = world_app(store);
        run_for(&mut again, 300);
        assert!(again.world().contains_resource::<Session>());
        let s = again.world().resource::<WorldStream>();
        let p = &s.shared.lock().unwrap().progress;
        assert_eq!(p.stats.network_bytes, 0, "{p:?}");
        assert!(p.stats.cache_chunks > 0);
    }
}
