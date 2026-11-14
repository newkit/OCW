//! Bevy glue between the server link and [`ClientCore`]: the in-process
//! local server (default), or a remote server over QUIC (`--server`, spec
//! §6.2). Devices (trainers, sensors) are handled in `devices.rs`.

use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use clap::{Arg, Command, value_parser};
use ocw_protocol::v1::{self, client_message::Msg as C};
use ocw_protocol::{client_message, encode};
use ocw_render::RideView;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, ConsoleState, Permission};
use ocw_server::Course;
use ocw_server::local::{LocalConnection, LocalServer, SteppedServer};
use ocw_server::net::{ClientOptions, LinkStatus, NetClient, ServerTrust};
use ocw_sim::FreeRideStart;
use ocw_world::{RoadNetwork, Route, RoutePlan};

use crate::Args;
use crate::client::{ClientCore, Notice};
use crate::event::GhostOutcome;
use crate::ghost::GhostStore;

pub struct ConnectionPlugin {
    pub route: Route,
}

/// Free ride on the road graph (`--free-ride`): inserted by the app before
/// [`ConnectionPlugin`]; without it the session rides the route.
#[derive(Resource, Clone)]
pub struct FreeRideSetup {
    pub net: Arc<RoadNetwork>,
    pub start: FreeRideStart,
    /// (lat, lon) of the package's local metres.
    pub origin: (f64, f64),
    /// A route on the road graph to follow (`--route NAME`, `--route-gpx`).
    pub plan: Option<Arc<RoutePlan>>,
}

impl Plugin for ConnectionPlugin {
    fn build(&self, app: &mut App) {
        start_session(app.world_mut(), self.route.clone());
        if !app.world().contains_resource::<Ghosts>() {
            app.insert_resource(Ghosts(crate::ghost::default_dir().map(GhostStore::new)));
        }
        app.init_resource::<LinkNotice>()
            .init_resource::<WorldChangeInbox>()
            .add_systems(PreUpdate, (poll_server, link_status).chain())
            .add_systems(FixedUpdate, (step_server, predict).chain())
            .add_systems(Update, (send_steer, ghost_io, update_view));
        register_commands(app);
    }
}

/// When the current ride session started (game time, s): the HUD's ride
/// time counts from it (an in-app ride switch starts a new session).
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct SessionStart(pub f64);

/// Starts a ride session: the server (local thread, stepped or remote)
/// for `route` or the [`FreeRideSetup`] in `world`, and a new
/// [`ClientCore`] that joins it. Inserts [`Link`] and [`ClientState`],
/// replacing the old session's (dropping the old local server's link ends
/// its thread). Used at the start and by the ride switch (`ride` module).
pub fn start_session(world: &mut World, route: Route) {
    start_session_with(world, route, false);
}

/// A server's world change (spec §3.2.1), until the ride module loads it.
#[derive(Resource, Default)]
pub struct WorldChangeInbox(pub Option<v1::WorldChange>);

/// [`start_session`]; with `keep_link` a remote server's connection stays
/// and the new client joins on it (after the host switched the world: the
/// server restarted its session, the connection order – the host – stays).
pub fn start_session_with(world: &mut World, route: Route, keep_link: bool) {
    let args = world.resource::<Args>().clone();
    let free = world.get_resource::<FreeRideSetup>().cloned();
    // The rider profile's weight and bike (physics profile id) go to
    // the server in the hello and into the prediction (spec §2.5,
    // §3.2.1); the look is cosmetic, for the other clients.
    let rider = crate::settings::local_rider(world);
    let profile = rider.physics();
    let autodrive = crate::settings::autodrive_at_start(world);
    let (course, client) = match free {
        Some(FreeRideSetup {
            net,
            origin,
            plan: Some(plan),
            ..
        }) => (
            Course::RouteRide {
                net: net.clone(),
                plan: plan.clone(),
            },
            ClientCore::free_ride(net, args.name.clone(), profile)
                .with_geo_origin(origin)
                .with_route_plan(plan),
        ),
        Some(f) => (
            Course::FreeRide {
                net: f.net.clone(),
                start: f.start,
            },
            ClientCore::free_ride(f.net, args.name.clone(), profile).with_geo_origin(f.origin),
        ),
        None => (
            Course::Route(route.clone()),
            ClientCore::new(route, args.name.clone(), profile),
        ),
    };
    let rules = world
        .get_resource::<crate::collisions::CollisionSettings>()
        .copied()
        .unwrap_or_default();
    let mut client = client
        .with_autodrive_at_start(autodrive)
        .with_look(rider.look());
    client.set_proximity_cue(rules.proximity_cue);
    let start_m = match &course {
        Course::Route(route) => crate::start_point(&args, route),
        Course::FreeRide { .. } | Course::RouteRide { .. } => 0.0,
    };
    // The old session's link goes first: its local server thread ends.
    let old = world.remove_resource::<Link>();
    let kept = match old {
        Some(link) if keep_link && args.server.is_some() => {
            match link.server.into_inner().expect("not poisoned") {
                remote @ ServerLink::Remote(_) => Some(remote),
                _ => None,
            }
        }
        _ => None,
    };
    let mut server = if let Some(link) = kept {
        link
    } else if let Some(spec) = &args.server {
        ServerLink::Remote(Box::new(connect_remote(&args, spec)))
    } else if args.capture.is_some() {
        // Frame capture: the server shares the client's fixed-step game time.
        ServerLink::Stepped(Box::new(SteppedServer::new(course, args.bots, start_m)))
    } else {
        ServerLink::Threaded(
            LocalServer::spawn(course, args.bots, start_m)
                .expect("failed to start local server thread"),
        )
    };
    server.send(client.hello());
    // The rider's collision rules (spec §14 Phase 12A), if not the defaults.
    if let Some(m) = rules.rules_message() {
        server.send(m);
    }
    let now = world
        .get_resource::<Time>()
        .map_or(0.0, |t| t.elapsed_secs_f64());
    world.insert_resource(Link {
        server: Mutex::new(server),
    });
    world.insert_resource(ClientState(client));
    world.insert_resource(SessionStart(now));
}

/// Connects to a remote server (`--server ADDR`). Bad options end the
/// program with a message; a server that cannot be reached is shown in the
/// HUD instead (the link closes).
fn connect_remote(args: &Args, spec: &str) -> NetClient {
    let fail = |e: String| -> ! {
        eprintln!("--server {spec}: {e}");
        std::process::exit(2);
    };
    let (addr, name) = ocw_server::net::resolve(spec).unwrap_or_else(|e| fail(e));
    let trust = ServerTrust::for_server(addr, args.server_cert.as_deref(), args.insecure_dev)
        .unwrap_or_else(|e| fail(e));
    info!(%addr, "connecting to remote server");
    NetClient::connect(addr, &name, &trust, ClientOptions::default()).unwrap_or_else(|e| fail(e))
}

/// The server: the local one on its own real-time thread, stepped by the
/// app (frame capture with a fixed time step per frame), or remote.
pub enum ServerLink {
    Threaded(LocalConnection),
    Stepped(Box<SteppedServer>),
    Remote(Box<NetClient>),
}

impl ServerLink {
    fn send(&mut self, bytes: Vec<u8>) {
        match self {
            // A send error means the server thread ended; nothing to do then.
            Self::Threaded(c) => {
                let _ = c.to_server.send(bytes);
            }
            Self::Stepped(s) => s.send(&bytes),
            Self::Remote(c) => c.send(bytes),
        }
    }

    fn receive(&mut self) -> Vec<Vec<u8>> {
        match self {
            Self::Threaded(c) => c.from_server.try_iter().collect(),
            Self::Stepped(s) => s.drain(),
            Self::Remote(c) => c.receive(),
        }
    }

    /// A line for the HUD while the remote link is not up (none otherwise).
    fn hud_line(&self) -> Option<String> {
        match self {
            Self::Remote(c) => match c.status() {
                LinkStatus::Connecting => Some("Connecting to the server …".into()),
                LinkStatus::Connected { .. } => None,
                LinkStatus::Closed(reason) => Some(format!("Disconnected: {reason}")),
            },
            _ => None,
        }
    }
}

/// Link to the server (receivers are not `Sync`, hence the mutex).
#[derive(Resource)]
pub struct Link {
    server: Mutex<ServerLink>,
}

/// The remote link's HUD line ([`ServerLink::hud_line`]), updated every frame.
#[derive(Resource, Default)]
pub struct LinkNotice(pub Option<String>);

impl Link {
    pub fn send_bytes(&self, bytes: Vec<u8>) {
        self.server.lock().expect("not poisoned").send(bytes);
    }

    pub fn send(&self, msg: C) {
        self.send_bytes(encode(&client_message(msg)));
    }
}

#[derive(Resource)]
pub struct ClientState(pub ClientCore);

/// Where time-trial ghosts are kept (`None`: nowhere, e.g. in tests).
#[derive(Resource)]
pub struct Ghosts(pub Option<GhostStore>);

/// Loads the ghost a time trial needs and keeps a finished effort if it
/// is the best (spec §2.3).
fn ghost_io(
    ghosts: Res<Ghosts>,
    mut client: ResMut<ClientState>,
    mut console: ResMut<ConsoleState>,
) {
    if let Some(key) = client.0.wants_ghost() {
        let ghost = ghosts.0.as_ref().and_then(|g| g.load(&key));
        match &ghost {
            Some(g) => {
                info!(course = %key.course, length_m = key.length_m, best_s = g.duration_s(), "ghost loaded")
            }
            None => info!(course = %key.course, length_m = key.length_m, "no ghost yet"),
        }
        client.0.set_ghost(key, ghost);
    }
    if let Some((key, trace)) = client.0.take_finished_trace() {
        let outcome = match &ghosts.0 {
            Some(store) => store.save_if_better(&key, &trace),
            None => GhostOutcome::Failed,
        };
        let line = crate::event::outcome_line(outcome, trace.duration_s());
        info!(course = %key.course, length_m = key.length_m, time_s = trace.duration_s(), ?outcome, "time trial finished");
        console.print(format!(
            "time trial: {} - {line}",
            crate::event::race_time(trace.duration_s())
        ));
        client.0.set_ghost_outcome(outcome);
    }
}

/// Ticks the stepped server (frame capture) at the fixed physics rate.
fn step_server(link: Res<Link>) {
    if let ServerLink::Stepped(s) = &mut *link.server.lock().expect("not poisoned") {
        s.tick();
    }
}

/// Handles server messages: welcome, snapshots, command results.
///
/// Times are game time (Bevy's virtual clock): equal to wall-clock time in
/// normal runs, a fixed step per frame during frame capture.
fn poll_server(
    link: Res<Link>,
    time: Res<Time>,
    mut client: ResMut<ClientState>,
    mut console: ResMut<ConsoleState>,
    voice: Option<ResMut<crate::voice::PendingVoiceGrants>>,
    inbox: Option<ResMut<WorldChangeInbox>>,
) {
    let mut voice = voice;
    let mut inbox = inbox;
    let messages = link.server.lock().expect("not poisoned").receive();
    let now = time.elapsed_secs_f64();
    for bytes in messages {
        match client.0.on_server_message(&bytes, now) {
            Some(Notice::Joined { rider_id, route_id }) => {
                info!(rider = %rider_id, route = %route_id, "joined session");
            }
            Some(Notice::Rejected(reason)) => {
                error!(%reason, "server rejected us");
                console.print(format!("server rejected connection: {reason}"));
            }
            Some(Notice::CommandResult(text)) => console.print(text),
            Some(Notice::God { event, message }) => {
                info!(?event, "{message}");
                console.print(message);
            }
            Some(Notice::Voice(grant)) => {
                if let Some(v) = voice.as_mut() {
                    v.0.push(grant);
                }
            }
            Some(Notice::WorldChange(change)) => {
                info!(world = %change.package_id, version = change.version, by = %change.by, "the server switched the world");
                console.print(format!(
                    "{} switched the world to {} ({} v{})",
                    if change.by.is_empty() {
                        "the host"
                    } else {
                        change.by.as_str()
                    },
                    change.name,
                    change.package_id,
                    change.version
                ));
                if let Some(i) = inbox.as_mut() {
                    i.0 = Some(change);
                }
            }
            None => {}
        }
    }
}

/// Remote link: the HUD line, a console message when it drops, and the
/// snapshot age for the prediction (half the round trip).
fn link_status(
    link: Res<Link>,
    mut client: ResMut<ClientState>,
    mut notice: ResMut<LinkNotice>,
    mut console: ResMut<ConsoleState>,
) {
    let server = link.server.lock().expect("not poisoned");
    let ServerLink::Remote(c) = &*server else {
        return;
    };
    if let Some(rtt) = c.rtt() {
        client.0.set_snapshot_age(rtt.as_secs_f64() / 2.0);
    }
    let line = server.hud_line();
    if line != notice.0 {
        if let Some(l) = line.as_deref().filter(|l| l.starts_with("Disconnected")) {
            warn!("{l}");
            console.print(l.to_owned());
        }
        notice.0 = line;
    }
}

/// Client-side prediction of the local rider, at the fixed physics rate.
fn predict(mut client: ResMut<ClientState>) {
    client.0.predict_step();
}

/// Sends the steering intent when the target changed, and the exit choice
/// for the next junction (free ride).
fn send_steer(time: Res<Time>, link: Res<Link>, mut client: ResMut<ClientState>) {
    if let Some(msg) = client.0.steer_message(time.elapsed_secs_f64()) {
        link.send_bytes(msg);
    }
    if let Some(msg) = client.0.turn_message() {
        link.send_bytes(msg);
    }
}

/// Free ride: a U-turn (key U, console `ride.uturn`); sends it to the server.
pub fn u_turn(world: &mut World) -> Result<String, String> {
    let now = world.resource::<Time>().elapsed_secs_f64();
    let msg = world.resource_mut::<ClientState>().0.u_turn(now);
    match msg {
        Some(m) => {
            world.resource::<Link>().send_bytes(m);
            Ok("U-turn".into())
        }
        None => Err("no U-turn possible (free ride only, at most every 10 s)".into()),
    }
}

/// Route ride: back to the route (key J, console `ride.rejoin`).
pub fn rejoin_route(world: &mut World) -> Result<String, String> {
    let msg = world.resource_mut::<ClientState>().0.rejoin_route()?;
    world.resource::<Link>().send_bytes(msg);
    Ok("rejoining the route".into())
}

pub(crate) fn update_view(
    time: Res<Time>,
    fixed: Res<Time<Fixed>>,
    client: Res<ClientState>,
    link: Res<LinkNotice>,
    session: Option<Res<SessionStart>>,
    loading: Option<Res<crate::ride::Loading>>,
    mut view: ResMut<RideView>,
) {
    let now = time.elapsed_secs_f64();
    view.elapsed_s = now - session.map_or(0.0, |s| s.0);
    // Interpolate between the last two 50 Hz prediction steps.
    view.riders = client.0.view(now, fixed.overstep_fraction_f64());
    view.position = view
        .local()
        .and_then(|l| crate::gps::position(client.0.track(), client.0.geo_origin(), &l.state));
    view.event = client.0.event_view(now);
    view.ghost = client.0.ghost_state(now);
    if client.0.is_free_ride() {
        view.paths = client.0.paths();
        view.junction = client.0.junction_view();
        view.notice = client.0.notice();
        view.route_status = client.0.route_status();
    }
    // A dropped connection matters more than any ride notice; a ride
    // being loaded (ride switch) shows over the ride's own notices.
    if let Some(line) = &link.0 {
        view.notice = Some(line.clone());
    } else if let Some(line) = loading.as_ref().and_then(|l| l.0.clone()) {
        view.notice = Some(line);
    } else if !client.0.is_free_ride() {
        view.notice = None;
    }
    // Collisions (spec §14 Phase 12): the crash line first, then the
    // proximity cue and the crash feed (in events it is in the event panel).
    view.proximity = client.0.proximity();
    if link.0.is_none() {
        if let Some(line) = client.0.crash_notice(now) {
            view.notice = Some(line);
        } else if view.notice.is_none() {
            view.notice = match view.proximity {
                Some(-1) => Some("<< rider close on the left".into()),
                Some(1) => Some("rider close on the right >>".into()),
                Some(_) => Some("^ rider close in front".into()),
                None if view.event.is_none() => client.0.crash_feed(now).pop(),
                None => None,
            };
        }
    }
}

/// The console command `event` (spec §2.3): start a local event, cancel
/// it, or show its state.
fn event_command() -> Command {
    let finish = |c: Command| {
        c.arg(
            Arg::new("laps")
                .long("laps")
                .value_parser(value_parser!(u32).range(1..=100))
                .help("Laps of a loop"),
        )
        .arg(
            Arg::new("km")
                .long("km")
                .value_parser(value_parser!(f64))
                .conflicts_with("laps")
                .help("Distance to the finish (km)"),
        )
    };
    let bots = Arg::new("bots")
        .long("bots")
        .value_parser(value_parser!(u32).range(0..=50))
        .help("Bots in the field (default: the current ones)");
    Command::new("event")
        .about("Local events: a race, group ride or time trial against your ghost (spec 2.3)")
        .subcommand_required(true)
        .subcommand(
            finish(Command::new("race").about(
                "Race from the start formation to the finish (default: one lap of a loop, 5 km of a route that ends)",
            ))
            .arg(bots.clone()),
        )
        .subcommand(
            Command::new("group")
                .about("Group ride: the bots hold a pace and regroup; no ranking")
                .arg(
                    Arg::new("watts")
                        .long("watts")
                        .value_parser(value_parser!(f64))
                        .help("The group's pace (W; default 150)"),
                )
                .arg(bots),
        )
        .subcommand(finish(
            Command::new("tt").about("Time trial alone against your best effort (the ghost)"),
        ))
        .subcommand(Command::new("cancel").about("End the current event"))
        .subcommand(Command::new("status").about("The current event and its standings"))
        .subcommand(Command::new("dismiss").about("Close the results panel"))
}

/// Starts, cancels or shows the event (console `event ...`).
fn run_event(world: &mut World, m: &clap::ArgMatches) -> Result<String, String> {
    let (sub, args) = m
        .subcommand()
        .ok_or("event race | group | tt | cancel | status")?;
    let laps = || args.get_one::<u32>("laps").copied();
    let km = || args.get_one::<f64>("km").map(|k| k * 1000.0);
    let bots = || args.get_one::<u32>("bots").copied();
    let msg = match sub {
        "race" => ClientCore::event_start_message(
            &ocw_sim::EventSpec::Race {
                laps: laps(),
                length_m: km(),
            },
            bots(),
        ),
        "tt" => ClientCore::event_start_message(
            &ocw_sim::EventSpec::TimeTrial {
                laps: laps(),
                length_m: km(),
            },
            None,
        ),
        "group" => ClientCore::event_start_message(
            &ocw_sim::EventSpec::GroupRide {
                target_w: args.get_one::<f64>("watts").copied(),
            },
            bots(),
        ),
        "cancel" => ClientCore::event_cancel_message(),
        "dismiss" => {
            let now = world.resource::<Time>().elapsed_secs_f64();
            return if world.resource_mut::<ClientState>().0.dismiss_results(now) {
                Ok("results closed".into())
            } else {
                Err("no results panel open".into())
            };
        }
        _ => {
            let now = world.resource::<Time>().elapsed_secs_f64();
            return Ok(event_status(&world.resource::<ClientState>().0, now));
        }
    };
    // The server answers (printed when it arrives).
    world.resource::<Link>().send_bytes(msg);
    Ok(String::new())
}

/// `event status`: the event's title, the HUD lines and the standings.
pub fn event_status(client: &ClientCore, now_s: f64) -> String {
    let (Some(info), Some(view)) = (client.event_info(), client.event_view(now_s)) else {
        return "no event (event race | group | tt)".into();
    };
    let mut s = format!("{} ({:?})", view.title, info.phase);
    for l in &view.lines {
        s.push_str("\n  ");
        s.push_str(l);
    }
    for r in crate::event::result_rows(&info.kind, &info.standings, client.rider_id()) {
        s.push_str(&format!(
            "\n{:>4} {:<16} {:>10} {}{}",
            r.position,
            r.name,
            r.time,
            r.gap,
            if r.is_local { "  <- you" } else { "" }
        ));
    }
    s
}

fn register_commands(app: &mut App) {
    crate::collisions::register_commands(app);
    app.console_command(ConsoleCommand::new(
        Permission::User,
        event_command(),
        run_event,
    ))
    .console_command(ConsoleCommand::new(
        Permission::Dev,
        Command::new("teleport")
            .about("Move the local rider to a distance on the route (km)")
            .arg(Arg::new("km").required(true).value_parser(value_parser!(f64))),
        |world, m| {
            let km = *m.get_one::<f64>("km").expect("required");
            world.resource::<Link>().send(C::Dev(v1::DevCommand {
                cmd: Some(v1::dev_command::Cmd::TeleportDistanceM(km * 1000.0)),
            }));
            // The server confirms; the prediction snaps on the next snapshot.
            Ok(String::new())
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("net.stats").about("Show connection statistics"),
        |world, _| {
            let mode = match &*world.resource::<Link>().server.lock().expect("not poisoned") {
                ServerLink::Threaded(_) => "local (in-process thread)".to_owned(),
                ServerLink::Stepped(_) => "local (stepped, frame capture)".to_owned(),
                ServerLink::Remote(c) => format!(
                    "remote, QUIC: {}{}",
                    match c.status() {
                        LinkStatus::Connecting => "connecting".to_owned(),
                        LinkStatus::Connected { datagrams: true } => "connected (datagrams)".to_owned(),
                        LinkStatus::Connected { datagrams: false } => "connected (stream only)".to_owned(),
                        LinkStatus::Closed(r) => format!("closed: {r}"),
                    },
                    c.rtt()
                        .map(|r| format!(", rtt {:.1} ms", r.as_secs_f64() * 1000.0))
                        .unwrap_or_default()
                ),
            };
            let c = &world.resource::<ClientState>().0;
            let s = c.stats();
            Ok(format!(
                "server: {}\nrider: {}\nmessages in: {} ({} malformed)\nsnapshots: {} (last server tick {}, {} out of order)\nsamples out: {}\nprediction error at last snapshot: {:.3} m",
                mode,
                c.rider_id().map(|r| r.to_string()).unwrap_or("-".into()),
                s.messages_in,
                s.malformed,
                s.snapshots,
                s.last_snapshot_tick,
                s.stale_snapshots,
                s.samples_out,
                c.prediction_error()
            ))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("rider.gps").about("Show the rider's GPS position (lat, lon, altitude)"),
        |world, _| match world.resource::<RideView>().position {
            Some((lat, lon, alt)) => Ok(format!(
                "{}\n{lat:.6}, {lon:.6}",
                ocw_render::hud::gps_line(lat, lon, alt)
            )),
            None => Err("no GPS position (this route has no geographic origin)".into()),
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("rider.steer")
            .about("Steer to a lateral position on the road in metres (positive = right; or use the arrow keys)")
            .arg(
                Arg::new("metres")
                    .required(true)
                    .allow_negative_numbers(true)
                    .value_parser(value_parser!(f64)),
            ),
        |world, m| {
            let target = *m.get_one::<f64>("metres").expect("required");
            let mut c = world.resource_mut::<ClientState>();
            c.0.steer_to(target);
            match c.0.lateral_target() {
                Some(t) => Ok(format!("steering to {t:+.1} m")),
                None => Err("not riding yet".into()),
            }
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("rider.lean")
            .about("Lean into a curve by DEGREES (positive = right, 0 = straighten up; like the arrow keys)")
            .arg(
                Arg::new("degrees")
                    .required(true)
                    .allow_negative_numbers(true)
                    .value_parser(value_parser!(f64)),
            ),
        |world, m| {
            let deg = *m.get_one::<f64>("degrees").expect("required");
            let mut c = world.resource_mut::<ClientState>();
            c.0.set_lean(deg.to_radians());
            match c.0.lean() {
                Some(l) => Ok(format!("leaning {:+.0}°", l.to_degrees())),
                None => Err("not riding yet".into()),
            }
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("rider.autodrive")
            .about("Autodrive on (keeps to the middle of the road) or off (steer by leaning; key A)")
            .arg(Arg::new("on").required(true).value_parser(["on", "off"])),
        |world, m| {
            let on = m.get_one::<String>("on").expect("required") == "on";
            let mut c = world.resource_mut::<ClientState>();
            if c.0.rider_id().is_none() {
                return Err("not riding yet".into());
            }
            c.0.set_autodrive(on);
            Ok(format!("autodrive {}", if on { "on" } else { "off" }))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("rider.wild")
            .about(
                "Wild mode on (not bound to the road, ride anywhere; autodrive off) or off (key W)",
            )
            .arg(Arg::new("on").required(true).value_parser(["on", "off"])),
        |world, m| {
            let on = m.get_one::<String>("on").expect("required") == "on";
            world.resource_mut::<ClientState>().0.set_wild(on);
            Ok(format!("wild mode {}", if on { "on" } else { "off" }))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("rider.state").about("Print the local rider's state"),
        |world, _| {
            let c = &world.resource::<ClientState>().0;
            let s = c.state();
            Ok(format!(
                "distance {:.1} m, speed {:.2} km/h, power {:.0} W, grade {:+.1} %",
                s.distance,
                s.speed * 3.6,
                c.power_w(),
                c.track().sample(s.distance).grade * 100.0
            ))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("ride.turn")
            .about("Choose the exit at the next junction (free ride): left, right, straight, default, + / - (one exit right / left) or the exit's number from the left (0, 1, ...)")
            .arg(Arg::new("exit").required(true).allow_hyphen_values(true)),
        |world, m| {
            let exit = m.get_one::<String>("exit").expect("required").clone();
            let mut c = world.resource_mut::<ClientState>();
            let said = match exit.as_str() {
                "+" => c.0.turn(1),
                "-" => c.0.turn(-1),
                "default" => c.0.clear_exit(),
                "left" | "right" | "straight" => c.0.choose_direction(&exit),
                n => match n.parse::<usize>() {
                    Ok(i) => c.0.choose_exit(i),
                    Err(_) => return Err(format!("unknown exit {n:?}")),
                },
            };
            said.ok_or_else(|| "no such exit, or no junction ahead (free ride only)".into())
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("ride.uturn").about("Turn round (free ride; key U)"),
        |world, _| u_turn(world),
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("ride.rejoin")
            .about("Route ride: back to the route after leaving it (key J; the shortest way to it is added)"),
        |world, _| rejoin_route(world),
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("ride.route").about("Route ride: the route's name, progress and whether you are on it"),
        |world, _| {
            let c = &world.resource::<ClientState>().0;
            c.route_status()
                .map(|s| match c.on_route() {
                    Some(false) => format!("{s}\nride.rejoin (key J) returns to the route"),
                    _ => s,
                })
                .ok_or_else(|| "not riding a route (--world PKG --route NAME or --route-gpx FILE)".into())
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("ride.junction").about("Show the next junction's exits (free ride)"),
        |world, _| {
            let c = &world.resource::<ClientState>().0;
            let Some(j) = c.junction_view() else {
                return Ok("no junction ahead within the look-ahead".into());
            };
            let mut s = format!(
                "junction in {:.0} m{}:",
                j.distance_m,
                if j.locked { " (locked)" } else { "" }
            );
            for (i, e) in j.exits.iter().enumerate() {
                s.push_str(&format!(
                    "\n {i}: {:+4.0} deg  {}{}{}",
                    e.angle_deg,
                    e.name,
                    if i == j.selected { "  <- selected" } else { "" },
                    if j.route == Some(i) {
                        "  (route)"
                    } else if i == j.default {
                        "  (default)"
                    } else {
                        ""
                    }
                ));
            }
            Ok(s)
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::Dev,
        Command::new("teleport.xz")
            .about("Free ride: move to the road nearest local x / z (metres)")
            .arg(
                Arg::new("x")
                    .required(true)
                    .allow_negative_numbers(true)
                    .value_parser(value_parser!(f64)),
            )
            .arg(
                Arg::new("z")
                    .required(true)
                    .allow_negative_numbers(true)
                    .value_parser(value_parser!(f64)),
            ),
        |world, m| {
            let x = *m.get_one::<f64>("x").expect("required");
            let z = *m.get_one::<f64>("z").expect("required");
            world.resource::<Link>().send(C::Dev(v1::DevCommand {
                cmd: Some(v1::dev_command::Cmd::TeleportPoint(v1::TeleportPoint {
                    x_m: x,
                    z_m: z,
                })),
            }));
            Ok(String::new())
        },
    ));
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use clap::Parser;
    use ocw_render::console::ConsoleCorePlugin;

    use super::*;

    fn app() -> App {
        app_with(&[])
    }

    fn app_with(extra: &[&str]) -> App {
        let mut argv = vec!["ocw-desktop", "--bots", "0", "--power", "150", "--virtual"];
        argv.extend_from_slice(extra);
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ConsoleCorePlugin))
            .insert_resource(Args::parse_from(argv))
            .insert_resource(RideView::new(Route::test_loop()))
            .insert_resource(Ghosts(None))
            .add_plugins((
                ConnectionPlugin {
                    route: Route::test_loop(),
                },
                crate::devices::DevicesPlugin,
            ));
        app
    }

    fn run(app: &mut App, line: &str) -> Vec<String> {
        app.world_mut().resource_mut::<ConsoleState>().queue(line);
        app.update();
        app.world().resource::<ConsoleState>().log().to_vec()
    }

    /// Updates the app until `done` holds or the timeout expires.
    fn update_until(app: &mut App, done: impl Fn(&ClientCore) -> bool) -> bool {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(3) {
            app.update();
            if done(&app.world().resource::<ClientState>().0) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    /// A networked game server on 127.0.0.1 (ephemeral port) with `bots`
    /// test riders, run like `ocw-server --listen` until `stop` is set.
    fn remote_server(
        bots: u32,
    ) -> (
        std::net::SocketAddr,
        String,
        Arc<std::sync::atomic::AtomicBool>,
        std::thread::JoinHandle<()>,
    ) {
        use ocw_server::net::{NetServer, ServerIdentity};
        use std::sync::atomic::{AtomicBool, Ordering};
        let (tx, rx) = std::sync::mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let s = stop.clone();
        let thread = std::thread::spawn(move || {
            let id = ServerIdentity::self_signed().unwrap();
            let mut net = NetServer::bind("127.0.0.1:0".parse().unwrap(), &id).unwrap();
            tx.send((net.local_addr(), net.fingerprint().to_owned()))
                .unwrap();
            let mut game = ocw_server::GameServer::new(
                Route::test_loop(),
                ocw_server::ServerConfig {
                    bots,
                    ..Default::default()
                },
            );
            while !s.load(Ordering::SeqCst) {
                net.pump(&mut game);
                let out = game.tick();
                net.broadcast(&game, &out);
                std::thread::sleep(Duration::from_millis(20));
            }
            net.shutdown();
        });
        let (addr, fp) = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        (addr, fp, stop, thread)
    }

    #[test]
    fn rides_on_a_remote_server_and_survives_its_end() {
        let (addr, fp, stop, thread) = remote_server(3);
        let addr = addr.to_string();
        let mut app = app_with(&["--route", "loop", "--server", &addr, "--server-cert", &fp]);
        // The remote server's bots appear next to the local rider.
        assert!(update_until(&mut app, |c| c.rider_id().is_some()
            && c.stats().snapshots > 3));
        assert!(update_until(&mut app, |_| true));
        let view = app.world().resource::<RideView>();
        assert_eq!(view.riders.len(), 4, "3 remote bots and us");
        assert_eq!(view.notice, None);
        // Remote: no dev commands.
        assert_eq!(
            app.world().resource::<ConsoleState>().permission,
            Permission::User
        );
        assert!(
            run(&mut app, "net.stats")
                .iter()
                .any(|l| l.contains("remote, QUIC: connected"))
        );
        // The server goes away: a HUD line and a console message, no panic.
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
        thread.join().unwrap();
        let start = Instant::now();
        while app.world().resource::<RideView>().notice.is_none()
            && start.elapsed() < Duration::from_secs(5)
        {
            app.update();
            std::thread::sleep(Duration::from_millis(10));
        }
        let notice = app.world().resource::<RideView>().notice.clone();
        assert!(
            notice
                .as_deref()
                .is_some_and(|n| n.starts_with("Disconnected") && n.contains("shutting down")),
            "{notice:?}"
        );
        assert!(
            app.world()
                .resource::<ConsoleState>()
                .log()
                .iter()
                .any(|l| l.starts_with("Disconnected"))
        );
        // The app keeps running.
        app.update();
    }

    #[test]
    fn joins_local_server_and_publishes_view() {
        let mut app = app();
        assert!(update_until(&mut app, |c| c.stats().snapshots > 2));
        let view = app.world().resource::<RideView>();
        assert_eq!(view.riders.len(), 1);
        assert!(view.riders[0].is_local);
        assert!(view.local().is_some());
    }

    #[test]
    fn event_commands_start_show_and_cancel_an_event() {
        let mut app = app();
        assert!(update_until(&mut app, |c| c.rider_id().is_some()));
        let log = run(&mut app, "event race --laps 1 --km 2");
        assert!(
            log.iter().any(|l| l.contains("cannot be used with")),
            "{log:?}"
        );
        assert!(
            run(&mut app, "event status")
                .iter()
                .any(|l| l.starts_with("no event"))
        );
        run(&mut app, "event race --bots 2 --km 0.5");
        assert!(update_until(&mut app, |c| c.event_info().is_some()));
        let log = app.world().resource::<ConsoleState>().log().to_vec();
        assert!(
            log.iter().any(|l| l.contains("race over 0.5 km, 3 riders")),
            "{log:?}"
        );
        let log = run(&mut app, "event status");
        assert!(
            log.iter().any(|l| l.starts_with("Race - 0.5 km")),
            "{log:?}"
        );
        // The HUD gets the countdown.
        app.update();
        let view = app.world().resource::<RideView>();
        assert!(view.event.as_ref().is_some_and(|e| e.countdown.is_some()));
        assert!(
            run(&mut app, "event dismiss")
                .last()
                .unwrap()
                .contains("no results")
        );
        run(&mut app, "event cancel");
        assert!(update_until(&mut app, |c| c.event_info().is_none()));
        assert!(app.world().resource::<RideView>().event.is_none());
    }

    #[test]
    fn dev_commands_need_dev_permission() {
        let mut app = app();
        let log = run(&mut app, "sim.power 300");
        assert!(
            log.last().unwrap().contains("needs Dev permission"),
            "{log:?}"
        );
        let log = run(&mut app, "teleport 1");
        assert!(
            log.last().unwrap().contains("needs Dev permission"),
            "{log:?}"
        );
        // User commands work without Dev.
        assert!(
            run(&mut app, "net.stats")
                .iter()
                .any(|l| l.contains("snapshots"))
        );
        assert!(
            run(&mut app, "rider.state")
                .iter()
                .any(|l| l.starts_with("distance"))
        );
    }

    #[test]
    fn sim_power_and_teleport_reach_trainer_and_server() {
        let mut app = app();
        app.world_mut().resource_mut::<ConsoleState>().permission = Permission::Dev;
        assert!(update_until(&mut app, |c| c.rider_id().is_some()));
        assert!(
            run(&mut app, "sim.power 9000")
                .last()
                .unwrap()
                .contains("between 0 and 2500")
        );
        assert_eq!(
            run(&mut app, "sim.power 320").last().unwrap(),
            "virtual power 320 W"
        );
        assert!(update_until(&mut app, |c| (c.power_w() - 320.0).abs() < 1e-6));
        run(&mut app, "teleport 1.5");
        assert!(update_until(&mut app, |c| (c.state().distance - 1500.0)
            .abs()
            < 10.0));
        // The server's confirmation is printed to the console.
        assert!(
            app.world()
                .resource::<ConsoleState>()
                .log()
                .iter()
                .any(|l| l.contains("teleported to 1.500 km"))
        );
    }
}
