//! Bevy glue of the browser app (spec §14 Phase 10, B1 / B3 / B4): moves
//! the frame time into the [`WebSession`] (the local server or the remote
//! link, the devices and the virtual trainer), runs the client prediction
//! at 50 Hz in `FixedUpdate`, reads the keys, hands the devices the page
//! connected to the session and writes the [`RideView`] the shared
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

use crate::devices::Bluetooth;
use crate::session::{RemoteOptions, SessionConfig, Transport, WebSession};

/// The ride session (server, client, trainer) as a Bevy resource.
#[derive(Resource)]
pub struct Session(pub WebSession);

/// The ride is ready (a world package: once its first tiles are there);
/// the zone of its place (the shared time machine resolves its own,
/// `ocw_ui::timemachine::WorldPlace`).
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
        // The shared UI (spec §14 Phase 10, B6: the desktop's menus, time
        // and weather machines, events, collisions) reaches the session's
        // client through this.
        app.insert_resource(ocw_ui::RideAccess {
            client: |w| w.get_resource::<Session>().map(|s| &s.0.client),
            client_mut: |w| {
                w.get_resource_mut::<Session>()
                    .map(|s| s.map_unchanged(|s| &mut s.0.client))
            },
            send: |w, bytes| {
                if let Some(mut s) = w.get_resource_mut::<Session>() {
                    s.0.send(&bytes);
                }
            },
        })
        .insert_resource(ocw_ui::UiPlatform(ocw_ui::Platform::Browser))
        .configure_sets(Update, ViewUpdated.in_set(ocw_ui::RideViewSet));
        app.add_systems(PreUpdate, frame.run_if(riding))
            .add_systems(Last, (flush, publish_stats).run_if(riding))
            .add_systems(FixedUpdate, predict.run_if(resource_exists::<Session>))
            .add_systems(
                Update,
                // Unicorn mode (spec §14 Phase 13B): the riding keys do
                // nothing in flight, as on the desktop.
                (
                    keys.run_if(ride_input).run_if(ocw_ui::god::not_flying),
                    send_steer,
                    update_view,
                )
                    .chain()
                    .in_set(ViewUpdated)
                    .run_if(riding),
            )
            .add_systems(Update, crate::console_from_page)
            .add_systems(Update, devices.after(update_view).run_if(riding));
        #[cfg(target_arch = "wasm32")]
        app.add_systems(Startup, crate::browser_devices::start);
        register_commands(app);
    }
}

/// The frame's ride view is written (the audio and voice systems run
/// after it).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct ViewUpdated;

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
    grants: Option<ResMut<crate::voice::PendingVoiceGrants>>,
    mut next_log: Local<f64>,
) {
    let now = time.elapsed_secs_f64();
    let remote = session.0.is_remote();
    let notices = session.0.frame(time.delta_secs_f64(), now);
    // The voice token grants (spec §7) go to the voice controller.
    if let Some(mut g) = grants {
        g.collect(&notices);
    }
    for notice in notices {
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

/// The desktop's ride keys and gamepad (spec §3.2.2; the pads through the
/// browser's Gamepad API, B6): ↑ / D-pad up / the right trigger add
/// power, ↓ / D-pad down / the left trigger stop, Page Up / Down ±10 W,
/// A / South autodrive (South closes an event's results first), W / West
/// wild mode, ← / → and the left stick the line (autodrive) or the lean.
/// The keys and buttons are the bindings' (spec §3.2.1 M9, shared with the
/// desktop: `ocw_ui::bindings`).
#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
fn keys(
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<(Entity, &Gamepad)>,
    time: Res<Time>,
    mut session: ResMut<Session>,
    mut throttle: Local<KeyThrottle>,
    mut upright_pause: Local<f64>,
    mut ride_pads: Local<ocw_ui::controls::RidePads>,
    mut stick_was_used: Local<bool>,
    bindings: Option<Res<ocw_ui::bindings::ActiveBindings>>,
) {
    use ocw_client::bindings::Action;
    let default = ocw_ui::bindings::ActiveBindings::default();
    let b = bindings.as_deref().unwrap_or(&default);
    let s = &mut session.0;
    let dt = time.delta_secs_f64();
    let now = time.elapsed_secs_f64();
    let pad = ride_pads.read_bound(pads.iter(), b);
    let pressed = |a| b.just_pressed(a, &keys, pads.iter().map(|(_, p)| p));
    let power_keys: ThrottleKeys = ocw_ui::controls::throttle_keys(b, &keys, &pad);
    s.throttle(&mut throttle, power_keys, dt, now);
    // An event's results panel takes A / South first (spec §2.3), as on
    // the desktop.
    let autodrive = pressed(Action::Autodrive);
    if autodrive && s.client.results_input(now) {
        s.client.dismiss_results(now);
    } else if autodrive {
        let on = !s.client.autodrive();
        s.client.set_autodrive(on);
        info!("autodrive {}", if on { "on" } else { "off" });
    }
    if pressed(Action::Wild) {
        let wild = !s.client.wild();
        s.client.set_wild(wild);
        info!("wild mode {}", if wild { "on" } else { "off" });
    }
    let dir = steer_direction(
        b.pressed(Action::SteerLeft, &keys, pads.iter().map(|(_, p)| p)),
        b.pressed(Action::SteerRight, &keys, pads.iter().map(|(_, p)| p)),
    );
    let stick_active = pad.stick_x.abs() >= ocw_client::input::STICK_DEAD_ZONE;
    if s.client.autodrive() {
        let shift = dir
            + if stick_active {
                f64::from(pad.stick_x)
            } else {
                0.0
            };
        if shift != 0.0 {
            s.client.steer_by(shift * LINE_KEY_RATE_MPS * dt);
        }
        *stick_was_used = false;
        return;
    }
    if stick_active || *stick_was_used {
        // The stick sets the lean directly; back to centre = upright.
        s.client
            .set_lean(ocw_client::input::stick_lean(pad.stick_x));
        *stick_was_used = stick_active;
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

/// The ride for the renderer: the same view as the desktop's
/// (`ocw_client::view::fill`: riders interpolated between the 50 Hz
/// prediction steps, the event, the notices, the riding aids, the damage
/// widget) and the status line; the sky and the weather come from the
/// shared time and weather machines (`ocw_ui`).
fn update_view(
    time: Res<Time>,
    fixed: Res<Time<Fixed>>,
    mut session: ResMut<Session>,
    shake: Option<Res<ocw_ui::collisions::CameraShake>>,
    access: Option<Res<ocw_render::access::Accessibility>>,
    mut view: ResMut<RideView>,
) {
    let now = time.elapsed_secs_f64();
    let s = &mut session.0;
    let link = s.link_notice();
    ocw_client::view::fill(
        &mut view,
        &s.client,
        now,
        now,
        fixed.overstep_fraction_f64(),
        ocw_client::view::Notices {
            link: link.as_deref(),
            loading: None,
        },
        // Reduced motion (spec §3.2.1 M9): no shake.
        shake.is_none_or(|s| s.0) && !access.is_some_and(|a| a.reduced_motion),
    );
    view.status = s.status_line();
}

/// The devices (B4): the ones the page connected start in the session;
/// connects, drops and workout steps to the console; the workout panel;
/// the devices' JSON for the page's overlay (twice a second); the shared
/// Settings → Trainer → Devices page.
#[allow(clippy::too_many_arguments)]
fn devices(
    time: Res<Time>,
    mut session: ResMut<Session>,
    mut console: ResMut<ConsoleState>,
    mut view: ResMut<RideView>,
    menu: Option<ResMut<ocw_ui::menu::DevicesMenu>>,
    mut said: Local<bool>,
    mut next: Local<f64>,
    mut loaded: Local<bool>,
) {
    let s = &mut session.0;
    #[cfg(target_arch = "wasm32")]
    {
        use ocw_devices::DeviceBackend;
        if !*loaded {
            // The first frame of the ride: this origin's devices.
            s.devices.remembered = crate::browser_devices::load_remembered();
        }
        s.devices.bluetooth = crate::browser_devices::bluetooth();
        let (pending, lines) = crate::browser_devices::take_pending();
        for line in lines {
            console.print(line);
        }
        for (backend, kind) in pending {
            let id = backend.device_id().to_string();
            let name = backend.name().to_string();
            match backend.start() {
                Ok(handle) => {
                    let line = s.devices.add(handle, kind, id);
                    info!("{line}");
                    console.print(line);
                }
                Err(e) => console.print(format!("{name}: {e}")),
            }
        }
    }
    // Remembered once connected (spec §3.2.1 *Devices*): saved then.
    if std::mem::take(&mut s.devices.remembered_changed) {
        persist(s);
    }
    if let Some(mut menu) = menu {
        let info = s.devices.menu_info();
        if menu.0 != info {
            menu.0 = info;
        }
    }
    *loaded = true;
    if !*said && s.devices.bluetooth != Bluetooth::Unknown {
        *said = true;
        match s.devices.bluetooth {
            Bluetooth::Missing => console.print(crate::devices::NO_WEB_BLUETOOTH),
            Bluetooth::NoAdapter => console.print(crate::devices::NO_ADAPTER),
            _ => {}
        }
    }
    for line in s.devices.take_messages() {
        info!("{line}");
        console.print(line);
    }
    let panel = s.devices.workout.panel(s.devices.ftp_w);
    if view.workout != panel {
        view.workout = panel;
    }
    let now = time.elapsed_secs_f64();
    if now >= *next {
        *next = now + 0.5;
        let json = devices_json(s);
        #[cfg(target_arch = "wasm32")]
        crate::browser_devices::publish(json);
        #[cfg(not(target_arch = "wasm32"))]
        let _ = json;
    }
}

/// The devices' JSON with the rider's power (the page's overlay and the
/// browser test).
pub fn devices_json(s: &WebSession) -> String {
    let mut v = s.devices.json(s.power_w());
    v["rider_power_w"] = s.client.power_w().into();
    v["virtual_sample"] = s.client.virtual_power().into();
    v["distance_m"] = s.client.state().distance.into();
    v["joined"] = s.client.rider_id().is_some().into();
    v.to_string()
}

/// Saves the remembered devices (the page's `localStorage`).
fn persist(s: &WebSession) {
    #[cfg(target_arch = "wasm32")]
    crate::browser_devices::save_remembered(&s.devices.remembered);
    #[cfg(not(target_arch = "wasm32"))]
    let _ = s;
}

/// Before a streamed world is loaded there is no ride yet.
const NO_RIDE: &str = "no ride yet (the world is still loading)";

fn session_mut(world: &mut World) -> Result<&mut WebSession, String> {
    world
        .get_resource_mut::<Session>()
        .map(|s| &mut s.into_inner().0)
        .ok_or_else(|| NO_RIDE.to_string())
}

/// `devices.connect`: the browser's chooser (Web Bluetooth needs the key
/// press that ran the command: a user gesture).
fn connect_device(kind: ocw_devices::ble::web::DeviceKind) -> Result<String, String> {
    #[cfg(target_arch = "wasm32")]
    return crate::browser_devices::connect(kind);
    #[cfg(not(target_arch = "wasm32"))]
    Err(format!(
        "no Web Bluetooth outside the browser ({})",
        kind.label()
    ))
}

/// `workout list`: the bundled examples (the browser has no workouts
/// folder yet).
fn workout_list() -> String {
    ocw_client::workout::EXAMPLES
        .iter()
        .filter_map(|(file, text)| {
            let w = ocw_sim::workout::Workout::parse_file(file, text).ok()?;
            Some(format!(
                "{} ({}): workout start --example {}",
                w.name,
                ocw_sim::workout::format_duration(w.duration_s()),
                ocw_client::workout::example_name(file)
            ))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The browser's rider is anonymous (spec §14 Phase 10 Q5): a plain
/// default rider, no customization – the mirror is locked.
fn locked() -> Result<String, String> {
    if ocw_core::character::may_customize(true) {
        return Ok(String::new());
    }
    Err(ocw_core::character::LOCKED_MIRROR.into())
}

fn register_commands(app: &mut App) {
    app.console_command(ConsoleCommand::new(
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
        Command::new("workshop")
            .about("The workshop and the changing corner (locked for anonymous riders)")
            .arg(Arg::new("args").num_args(0..).allow_hyphen_values(true)),
        |_, _| locked(),
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("character")
            .about("The rider's character (locked for anonymous riders)")
            .arg(Arg::new("args").num_args(0..).allow_hyphen_values(true)),
        |_, _| locked(),
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("devices.list")
            .about("Show Web Bluetooth, the connected and the remembered devices"),
        |world, _| Ok(session_mut(world)?.devices.list()),
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("devices.connect")
            .about("Choose a Bluetooth trainer, power meter or heart-rate strap in the browser's list (Chrome / Edge; the page's buttons do the same)")
            .arg(
                Arg::new("kind")
                    .required(true)
                    .value_parser(["trainer", "power", "hr"]),
            ),
        |_world, m| {
            let kind = m.get_one::<String>("kind").expect("required");
            let kind = ocw_devices::ble::web::DeviceKind::parse(kind).expect("checked by clap");
            connect_device(kind)
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("devices.forget")
            .about("Disconnect and forget a device (name or id), or all")
            .arg(Arg::new("device").required(true)),
        |world, m| {
            let which = m.get_one::<String>("device").expect("required");
            let s = session_mut(world)?;
            match s.devices.forget(which) {
                0 => Err(format!("no device {which}")),
                n => {
                    persist(s);
                    Ok(format!("forgot {n} device(s)"))
                }
            }
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("trainer.mode")
            .about("Trainer control: sim (simulate the road), erg (hold a power), off")
            .arg(
                Arg::new("mode")
                    .required(true)
                    .value_parser(["sim", "erg", "off"]),
            ),
        |world, m| {
            let mode = m.get_one::<String>("mode").expect("required");
            session_mut(world)?.devices.set_mode(mode)
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("trainer.difficulty")
            .about("Trainer difficulty in percent (scales the road gradient in SIM mode)")
            .arg(
                Arg::new("percent")
                    .required(true)
                    .value_parser(value_parser!(f64)),
            ),
        |world, m| {
            let p = *m.get_one::<f64>("percent").expect("required");
            if !(0.0..=100.0).contains(&p) {
                return Err("difficulty must be between 0 and 100 %".into());
            }
            session_mut(world)?
                .devices
                .trainer
                .set_difficulty(p / 100.0);
            Ok(format!("trainer difficulty {p:.0} %"))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("erg.target")
            .about("ERG mode with this target power in watts")
            .arg(
                Arg::new("watts")
                    .required(true)
                    .value_parser(value_parser!(f64)),
            ),
        |world, m| {
            let watts = *m.get_one::<f64>("watts").expect("required");
            if !(0.0..=2000.0).contains(&watts) {
                return Err("target must be between 0 and 2000 W".into());
            }
            session_mut(world)?
                .devices
                .trainer
                .set_mode(ocw_client::trainer::TrainerMode::Erg { target_w: watts });
            Ok(format!("trainer mode ERG {watts:.0} W"))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("workout")
            .about("Structured workouts in ERG mode: start --example <name>, stop, skip, pause, resume, status, list")
            .subcommand_required(true)
            .subcommand(
                Command::new("start")
                    .about("Start a bundled example (or the loaded workout again)")
                    .arg(
                        Arg::new("example")
                            .long("example")
                            .help("short-demo, sweet-spot, two-by-five, over-unders"),
                    ),
            )
            .subcommand(Command::new("list").about("List the bundled examples"))
            .subcommand(Command::new("stop").about("Stop the workout; the trainer returns to its previous mode"))
            .subcommand(Command::new("skip").about("Skip to the next step"))
            .subcommand(Command::new("pause").about("Pause the workout clock"))
            .subcommand(Command::new("resume").about("Resume after workout pause"))
            .subcommand(Command::new("status").about("Show the step, target and time left")),
        |world, m| {
            let d = &mut session_mut(world)?.devices;
            let ftp = d.ftp_w;
            let Some((sub, sm)) = m.subcommand() else {
                return Err("expected a subcommand".into());
            };
            match sub {
                "start" => {
                    if let Some(e) = sm.get_one::<String>("example") {
                        d.workout.load(ocw_client::workout::example(e)?)?;
                    }
                    d.workout.start()
                }
                "list" => Ok(workout_list()),
                "stop" => d.workout.stop(&mut d.trainer),
                "skip" => d.workout.skip(ftp),
                "pause" => d.workout.pause(),
                "resume" => d.workout.resume(),
                "status" => d.workout.status(ftp),
                other => Err(format!("unknown subcommand {other}")),
            }
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
            })
            // The sky from the shared time machine (B6).
            .add_plugins(crate::ui::WebUiPlugin::default());
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
        assert!(
            view.status
                .starts_with("Virtual trainer 200 W | trainer SIM")
        );
        app.world_mut()
            .resource_mut::<ConsoleState>()
            .queue("sim.power 260");
        app.update();
        app.update();
        assert_eq!(app.world().resource::<Session>().0.power_w(), 260.0);
        // The anonymous rider's mirror is locked (spec 4C, Phase 10 Q5).
        for line in ["character --skin 3", "workshop open"] {
            app.world_mut().resource_mut::<ConsoleState>().queue(line);
            app.update();
            let log = app.world().resource::<ConsoleState>().log().join("\n");
            assert!(log.contains("log in to customize"), "{line}: {log}");
        }
        let me = app.world().resource::<RideView>().riders[0].look;
        assert_eq!(me, Some(ocw_core::RiderLook::anonymous()));
        // The trainer commands (B4), as on the desktop.
        for (line, want) in [
            ("erg.target 230", "trainer mode ERG 230 W"),
            ("trainer.mode erg", "trainer mode ERG 230 W"),
            ("trainer.difficulty 80", "trainer difficulty 80 %"),
            ("trainer.mode sim", "trainer mode SIM"),
            (
                "devices.connect trainer",
                "error: no Web Bluetooth outside the browser (trainer)",
            ),
            ("devices.forget nope", "error: no device nope"),
        ] {
            app.world_mut().resource_mut::<ConsoleState>().queue(line);
            app.update();
            let log = app.world().resource::<ConsoleState>().log();
            assert_eq!(log.last().map(String::as_str), Some(want), "{line}");
        }
        let list = {
            app.world_mut()
                .resource_mut::<ConsoleState>()
                .queue("devices.list");
            app.update();
            app.world().resource::<ConsoleState>().log().join("\n")
        };
        assert!(list.contains("running: none (virtual trainer)"), "{list}");
        // A workout drives ERG; the panel shows in the view.
        app.world_mut()
            .resource_mut::<ConsoleState>()
            .queue("workout start --example short-demo");
        app.update();
        app.update();
        let s = &app.world().resource::<Session>().0;
        assert!(
            matches!(
                s.devices.trainer.mode(),
                ocw_client::trainer::TrainerMode::Erg { .. }
            ),
            "{}",
            s.devices.trainer.mode()
        );
        assert!(app.world().resource::<RideView>().workout.is_some());
        let json: serde_json::Value =
            serde_json::from_str(&devices_json(&app.world().resource::<Session>().0)).unwrap();
        assert_eq!(json["virtual"], true);
        assert!(json["last_command"]["erg_w"].is_number(), "{json}");
        assert!(workout_list().contains("--example sweet-spot"));
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
