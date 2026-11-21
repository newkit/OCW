//! Ride → Host a ride on the desktop (spec §3.2.1 *Host a ride*, §6.2):
//! a shared ride served by this app.
//!
//! The wizard (`ocw_client::host`: world, route, date and time, weather,
//! options, summary) fills a [`HostPlanState`] through `host.plan …`
//! lines. *Start hosting* (`host start`) binds the QUIC endpoint on
//! `0.0.0.0:PORT` first – a port in use (another server, perhaps the
//! kiosk's) or a refused bind is reported at once and the ride goes on –
//! with a certificate kept in `<data dir>/ocw/host-tls`, so the
//! fingerprint (and the join link) survive restarts. Then the course is
//! loaded like any ride switch and the session starts on the app's own
//! local server with that endpoint pumped by its thread
//! (`LocalServer::spawn_hosting`): the server runs in the app, no
//! separate binary, and stops with it. The app's rider is the first, so
//! it is the host. The plan's time and weather are set as the host's
//! time machine and weather machine lines.
//!
//! While hosting, the Ride page shows *Hosting – N riders*, *Show join
//! link* (with *Copy link*) and *Stop hosting*; `host status` prints the
//! links. Stopping closes the server (the others hear "server shutting
//! down") and the host rides on alone on the same world, route, time and
//! weather.
//!
//! **Voice and the router (spec §7.1.6 V2).** With the port bound, the
//! hosting's services start on their own thread while the course loads
//! (`ocw_server::hosting`, the same code as `ocw-server --listen`): the
//! router's ports opened by UPnP when the setting `host.upnp` is on (the
//! default; *Open the router's ports* on the Options page) and the
//! bundled `livekit-server` (next to the app, else on `PATH`) with fresh
//! keys, its media address the router's public address when there is one,
//! else STUN – unless the setting `host.voice` is off (*Voice* on the
//! Options page, spec §7.1.7 Q15): then no voice server starts, its
//! ports are not opened and the hosted server offers no voice (like
//! `ocw-server --voice off`; the riders' apps say "no voice on this
//! server" and show no voice signs). The session starts when both are done (the voice server's
//! keys go into the hosted server's config); neither ever stops the
//! hosting – the pages say what happened. *Stop hosting*, a failed load
//! and quitting stop the voice server and remove the mappings. The join
//! link page and `host status` list the ports with the firewall /
//! port-forwarding hint and the router's result; with a public address
//! from the router the first link names it.

use std::sync::Mutex;
use std::sync::atomic::Ordering;

use bevy::prelude::*;
use clap::{Arg, Command, value_parser};
use ocw_client::host::{HostNet, HostPlan, HostRouter, HostVoice, HostWorld};
use ocw_client::menu::pages::{Activate, Page, Row};
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, ConsoleState, Permission};
use ocw_server::hosting::{HostServices, HostStatus, HostingOptions, VoicePlan, VoiceState};
use ocw_server::net::{NetServer, ServerIdentity};
use ocw_server::portmap::{Discover, MapState};

use super::choice::{RideChoice, WorldMode};
use super::{Follow, RideCatalog, Then};
use crate::connection::{HostCounts, HostNext, HostStart};

/// The wizard's choices.
#[derive(Resource, Default)]
pub struct HostPlanState(pub HostPlan);

/// While hosting: what others join.
#[derive(Resource, Debug, Clone)]
pub struct Hosting {
    pub port: u16,
    pub fingerprint: String,
    /// The join links (one per address of this machine).
    pub links: Vec<String>,
    pub plan: HostPlan,
}

/// A host start whose course is being loaded.
#[derive(Resource)]
struct Starting {
    net: Mutex<Option<NetServer>>,
    plan: HostPlan,
    world_id: Option<String>,
    fingerprint: String,
    /// Router ports and voice server, starting (dropped: stopped).
    services: Option<HostServices>,
}

/// While hosting: its voice server and router ports (dropped when the
/// hosting stops – also at quit, [`stop_on_exit`]).
#[derive(Resource)]
pub struct HostRun(pub Option<HostServices>);

/// What *Start hosting* starts beside the server instead of the real
/// voice server and router (tests: a fake `livekit-server`, a router in
/// memory). Taken at the start.
#[derive(Resource, Default)]
pub struct HostServicesFor {
    pub voice: Mutex<Option<VoicePlan>>,
    pub router: Mutex<Option<Discover>>,
}

pub fn build(app: &mut App) {
    app.init_resource::<HostPlanState>()
        .add_systems(Update, ride_mode)
        .add_systems(Last, stop_on_exit);
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("host.plan")
            .about("Change the plan of Ride -> Host a ride (world N, route NAME|free, time, day, weather, bots, open, port, upnp, voice)")
            .arg(Arg::new("words").num_args(1..).required(true).allow_hyphen_values(true)),
        |world, m| {
            let words: Vec<String> = m
                .get_many::<String>("words")
                .map(|v| v.cloned().collect())
                .unwrap_or_default();
            let words: Vec<&str> = words.iter().map(String::as_str).collect();
            let worlds = host_worlds(world);
            sync_settings(world);
            let out = world.resource_mut::<HostPlanState>().0.apply(&words, &worlds)?;
            // *Open the router's ports* and *Voice* are remembered
            // (settings host.upnp, host.voice).
            let plan = &world.resource::<HostPlanState>().0;
            let saved = match words.first() {
                Some(&"upnp") => Some(("host.upnp", plan.upnp)),
                Some(&"voice") => Some(("host.voice", plan.voice)),
                _ => None,
            };
            if let Some((key, on)) = saved {
                ocw_ui::settings::save_setting(world, key, if on { "on" } else { "off" });
            }
            Ok(out)
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("host")
            .about("Host a shared ride on this computer (UDP 0.0.0.0:47800): start, status, stop, copy")
            .subcommand(
                Command::new("start")
                    .about("Start hosting the plan of Ride -> Host a ride (the options change it)")
                    .arg(Arg::new("world").long("world").value_name("ID|NAME|N"))
                    .arg(Arg::new("route").long("route").value_name("NAME|free"))
                    .arg(Arg::new("time").long("time").value_name("real|PRESET"))
                    .arg(Arg::new("weather").long("weather").value_name("simulated|real|PRESET"))
                    .arg(Arg::new("bots").long("bots").value_parser(value_parser!(u32)))
                    .arg(Arg::new("port").long("port").value_parser(value_parser!(u16)))
                    .arg(
                        Arg::new("upnp")
                            .long("upnp")
                            .value_name("on|off")
                            .help("Open the router's ports automatically (setting host.upnp)"),
                    )
                    .arg(
                        Arg::new("voice")
                            .long("voice")
                            .value_name("on|off")
                            .help("Voice chat on the hosted ride (setting host.voice)"),
                    ),
            )
            .subcommand(Command::new("status").about("Hosting? The riders and the join links"))
            .subcommand(Command::new("stop").about("Stop hosting: ride on alone"))
            .subcommand(Command::new("copy").about("Copy the join link to the clipboard")),
        |world, m| match m.subcommand() {
            Some(("start", s)) => {
                let get = |k: &str| s.get_one::<String>(k).cloned();
                let mut lines: Vec<String> = Vec::new();
                if let Some(w) = get("world") {
                    let worlds = host_worlds(world);
                    let n = find_world(&worlds, &w).ok_or_else(|| format!("no world {w:?} to host (World -> Change world lists them)"))?;
                    lines.push(format!("world {n}"));
                }
                if let Some(r) = get("route") {
                    lines.push(format!("route {r}"));
                }
                if let Some(t) = get("time") {
                    lines.push(format!("time {t}"));
                }
                if let Some(w) = get("weather") {
                    lines.push(format!("weather {w}"));
                }
                if let Some(b) = s.get_one::<u32>("bots") {
                    lines.push(format!("bots {b}"));
                }
                if let Some(p) = s.get_one::<u16>("port") {
                    lines.push(format!("port {p}"));
                }
                let worlds = host_worlds(world);
                sync_settings(world);
                for l in lines {
                    let words: Vec<&str> = l.split_whitespace().collect();
                    world.resource_mut::<HostPlanState>().0.apply(&words, &worlds)?;
                }
                // This start only (the setting stays).
                if let Some(u) = get("upnp") {
                    world.resource_mut::<HostPlanState>().0.apply(&["upnp", u.as_str()], &worlds)?;
                }
                if let Some(v) = get("voice") {
                    world.resource_mut::<HostPlanState>().0.apply(&["voice", v.as_str()], &worlds)?;
                }
                start(world)
            }
            Some(("stop", _)) => stop(world),
            Some(("copy", _)) => {
                let link = world
                    .get_resource::<Hosting>()
                    .and_then(|h| h.links.first().cloned())
                    .ok_or("not hosting")?;
                super::join::set_clipboard(&link)?;
                Ok(format!("copied: {link}"))
            }
            _ => Ok(status(world)),
        },
    ));
}

/// The worlds to host on: the test routes, then the rideable packages
/// with a road graph (from the ride catalog).
pub fn host_worlds(world: &mut World) -> Vec<HostWorld> {
    if world.resource::<RideCatalog>().0.worlds.is_empty() {
        super::refresh_catalog(world);
    }
    worlds_of(&world.resource::<RideCatalog>().0)
}

/// [`host_worlds`] from a catalog (the menu's rows read it as it is).
pub fn worlds_of(c: &super::pages::Catalog) -> Vec<HostWorld> {
    let mut out = vec![HostWorld::test_routes()];
    for e in c.worlds.iter().filter(|e| e.is_rideable()) {
        let Some(s) = &e.summary else { continue };
        if !s.has_graph {
            continue;
        }
        out.push(HostWorld {
            label: e.name.clone(),
            spec: Some(e.spec.clone()),
            routes: s.routes.iter().map(|r| r.name.clone()).collect(),
            free_ride: true,
        });
    }
    out
}

fn find_world(worlds: &[HostWorld], w: &str) -> Option<usize> {
    if let Ok(n) = w.parse::<usize>() {
        return (n < worlds.len()).then_some(n);
    }
    worlds.iter().position(|h| {
        h.label.eq_ignore_ascii_case(w)
            || h.spec
                .as_deref()
                .is_some_and(|s| s == w || s.starts_with(&format!("{w}@")))
    })
}

/// The ride the plan describes.
fn plan_choice(
    plan: &HostPlan,
    worlds: &[HostWorld],
) -> Result<(RideChoice, Option<String>), String> {
    let w = plan.world_of(worlds).ok_or("no world to host")?;
    Ok(match &w.spec {
        None => (
            RideChoice::TestRoute(plan.route.clone().unwrap_or_else(|| "hills".into())),
            None,
        ),
        Some(spec) => (
            RideChoice::World {
                world: spec.clone(),
                mode: match &plan.route {
                    Some(r) => WorldMode::Route(r.clone()),
                    None => WorldMode::FreeRide { start: None },
                },
            },
            Some(spec.clone()),
        ),
    })
}

/// Why binding failed, in words (the port in use: join the server there).
pub fn bind_error(port: u16, e: &str) -> String {
    let lower = e.to_ascii_lowercase();
    if lower.contains("in use")
        || lower.contains("os error 98")
        || lower.contains("os error 10048")
        || lower.contains("os error 48")
    {
        format!(
            "UDP port {port} is in use: another game server runs on this computer (perhaps the kiosk's). Join it instead: join ocw://localhost:{port} – or choose another port"
        )
    } else {
        format!("cannot open UDP port {port} for others: {e}")
    }
}

/// Where the hosted server's certificate is kept.
fn tls_dir() -> Option<std::path::PathBuf> {
    // Tests keep nothing (a new certificate each time).
    if cfg!(test) {
        return None;
    }
    super::data_dir().map(|d| d.join("host-tls"))
}

/// `host start`: binds, then loads the plan's course (see the module docs).
pub fn start(world: &mut World) -> Result<String, String> {
    if world.contains_resource::<Hosting>() || world.contains_resource::<Starting>() {
        return Err("already hosting (host status, host stop)".into());
    }
    if world.resource::<crate::Args>().server.is_some() {
        return Err("you ride on another server: leave the shared ride first".into());
    }
    let plan = world.resource::<HostPlanState>().0.clone();
    let worlds = host_worlds(world);
    let (choice, world_id) = plan_choice(&plan, &worlds)?;
    let identity = match tls_dir() {
        Some(d) => ServerIdentity::load_or_create(&d),
        None => ServerIdentity::self_signed(),
    }?;
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], plan.port));
    let net = NetServer::bind(addr, &identity).map_err(|e| bind_error(plan.port, &e))?;
    let net_fingerprint = net.fingerprint().to_owned();
    info!(
        port = plan.port,
        upnp = plan.upnp,
        voice = plan.voice,
        "hosting: the port is open, loading the course; starting the voice server"
    );
    // The router's ports and the voice server, while the course loads.
    let services = HostServices::spawn(services_options(world, addr, plan.upnp, plan.voice))
        .map_err(|e| format!("cannot start the hosting's voice server: {e}"))?;
    world.insert_resource(Starting {
        net: Mutex::new(Some(net)),
        plan: plan.clone(),
        world_id,
        fingerprint: net_fingerprint,
        services: Some(services),
    });
    let follow = Follow {
        name: plan.route_label(),
        sha256: Vec::new(),
        graph_hash: None,
        then: Then::Host,
    };
    match super::start_load(world, choice, Some(follow)) {
        Ok(m) => Ok(format!("hosting: {m}")),
        Err(e) => {
            if let Some(mut s) = world.remove_resource::<Starting>() {
                release(s.services.take());
            }
            Err(e)
        }
    }
}

/// The course is loaded: the next session's server is the hosted one.
pub(super) fn take_server(world: &mut World) -> Result<(), String> {
    let s = world
        .get_resource::<Starting>()
        .ok_or("hosting was stopped")?;
    let net = s
        .net
        .lock()
        .expect("not poisoned")
        .take()
        .ok_or("hosting was stopped")?;
    let next = HostNext {
        net,
        bots: s.plan.bots,
        open: s.plan.open_conditions,
        voice: s.services.as_ref().and_then(HostServices::voice_config),
    };
    world.resource_mut::<HostStart>().0 = Some(Mutex::new(next));
    Ok(())
}

/// Hosting: the links, the plan's time and weather.
pub(super) fn started(world: &mut World) {
    let Some(mut s) = world.remove_resource::<Starting>() else {
        return;
    };
    let services = s.services.take();
    let public_ip = services.as_ref().and_then(|v| v.status().public_ip);
    world.insert_resource(HostRun(services));

    let fingerprint = s.fingerprint.clone();
    let route = match world.resource::<super::CurrentRide>().choice.clone() {
        RideChoice::TestRoute(r) => Some(r),
        RideChoice::World {
            mode: WorldMode::Route(r),
            ..
        } => Some(r),
        _ => None,
    };
    let world_id = s.world_id.as_ref().map(|spec| {
        world
            .resource::<RideCatalog>()
            .0
            .worlds
            .iter()
            .flat_map(ocw_worldstore::discover::WorldEntry::members)
            .find(|e| &e.spec == spec)
            .map_or_else(
                || spec.clone(),
                |e| {
                    if e.id.starts_with("local/") {
                        e.id.clone()
                    } else {
                        format!("{}@{}", e.id, e.version)
                    }
                },
            )
    });
    let local = std::net::SocketAddr::from(([0, 0, 0, 0], s.plan.port));
    // The router's public address first: the link for riders from the
    // internet (spec §7.1.7 Q4, Q5).
    let links = ocw_server::net::join_links_with(
        local,
        &fingerprint,
        world_id.as_deref(),
        route.as_deref(),
        public_ip,
    );
    let net = net_view(world, s.plan.port);
    let mut console = world.resource_mut::<ConsoleState>();
    for line in s.plan.condition_lines() {
        console.queue(line);
    }
    console.permission = Permission::User;
    let h = Hosting {
        port: s.plan.port,
        fingerprint,
        links,
        plan: s.plan,
    };
    let msg = status_of(&h, 1, &net);
    info!("{msg}");
    console.print(msg);
    world.insert_resource(h);
}

pub(super) fn load_failed(world: &mut World, e: &str) {
    // Dropping the endpoint closes the port again; the voice server and
    // the router's ports stop too.
    if let Some(mut s) = world.remove_resource::<Starting>() {
        release(s.services.take());
    }
    let msg = format!("not hosting: cannot load the ride: {e}");
    warn!("{msg}");
    world.resource_mut::<ConsoleState>().print(msg);
}

/// `host stop`.
pub fn stop(world: &mut World) -> Result<String, String> {
    if let Some(mut s) = world.remove_resource::<Starting>() {
        release(s.services.take());
        return Ok("hosting cancelled".into());
    }
    let h = world.remove_resource::<Hosting>().ok_or("not hosting")?;
    // The new local session ends the hosted server's thread (its link is
    // dropped): the others hear "server shutting down".
    super::restart_alone(world);
    // Then the voice server stops and the router's ports close.
    if let Some(mut run) = world.remove_resource::<HostRun>() {
        release(run.0.take());
    }
    let mut console = world.resource_mut::<ConsoleState>();
    for line in h.plan.condition_lines() {
        console.queue(line);
    }
    info!("hosting stopped");
    Ok("hosting stopped: riding on alone (same world, route, time and weather)".into())
}

fn riders(world: &World) -> u32 {
    world
        .get_resource::<HostCounts>()
        .map_or(1, |c| c.0.riders.load(Ordering::Relaxed).max(1) as u32)
}

fn status_of(h: &Hosting, riders: u32, net: &HostNet) -> String {
    let mut out = format!(
        "hosting on UDP {} ({riders} rider{}); others join with:\n",
        h.port,
        if riders == 1 { "" } else { "s" }
    );
    for l in &h.links {
        out.push_str(&format!("  {l}\n"));
    }
    out.push_str(&format!("certificate SHA-256 {}\n", h.fingerprint));
    // The ports, the router's result and the hint (spec §7.1.6 V2).
    for l in net.lines() {
        out.push_str(&l);
        out.push('\n');
    }
    out.push_str(&format!("router: {}", net.router_label()));
    out
}

/// Stops a hosting's voice server and removes its router ports on a
/// thread of its own (a voice server still starting may take seconds;
/// the ride goes on meanwhile).
fn release(services: Option<HostServices>) {
    if let Some(s) = services {
        let _ = std::thread::Builder::new()
            .name("ocw-host-stop".into())
            .spawn(move || s.stop());
    }
}

/// Quitting (or the app's restart) stops the voice server and removes
/// the router's ports before the app ends.
fn stop_on_exit(
    mut exit: MessageReader<AppExit>,
    run: Option<ResMut<HostRun>>,
    starting: Option<ResMut<Starting>>,
) {
    if exit.read().next().is_none() {
        return;
    }
    if let Some(mut r) = run {
        drop(r.0.take());
    }
    if let Some(mut s) = starting {
        drop(s.services.take());
    }
}

/// The plan's router and voice switches follow the settings `host.upnp`
/// and `host.voice` (spec §7.1.7 Q4, Q15: both on by default).
fn sync_settings(world: &mut World) {
    let upnp = ocw_ui::settings::setting_on(world, "host.upnp");
    let voice = ocw_ui::settings::setting_on(world, "host.voice");
    let plan = &mut world.resource_mut::<HostPlanState>().0;
    plan.upnp = upnp;
    plan.voice = voice;
}

/// The plan as the pages show it: the wizard's choices with the saved
/// switches.
fn shown_plan(world: &World) -> HostPlan {
    let mut plan = world
        .get_resource::<HostPlanState>()
        .map(|p| p.0.clone())
        .unwrap_or_default();
    plan.upnp = ocw_ui::settings::setting_on(world, "host.upnp");
    plan.voice = ocw_ui::settings::setting_on(world, "host.voice");
    plan
}

/// What runs beside the hosted server (see the module docs): the voice
/// server found next to the app or on `PATH` when `voice`, the router by
/// UPnP when `upnp` – or what a test put into [`HostServicesFor`]. Unit
/// tests never start a real voice server or ask the network's router.
fn services_options(
    world: &World,
    listen: std::net::SocketAddr,
    upnp: bool,
    voice: bool,
) -> HostingOptions {
    use ocw_server::voice::launcher::{self, LaunchOptions};
    let mut opts = HostingOptions::new(listen);
    if let Some(f) = world.get_resource::<HostServicesFor>() {
        let plan = f.voice.lock().expect("not poisoned").take();
        opts.voice = plan.filter(|_| voice);
        opts.voice_off = if voice {
            "no voice server in this test"
        } else {
            ocw_client::host::VOICE_TURNED_OFF
        }
        .into();
        opts.upnp = f
            .router
            .lock()
            .expect("not poisoned")
            .take()
            .filter(|_| upnp);
        return opts;
    }
    if cfg!(test) {
        opts.voice_off = "no voice server in unit tests".into();
        return opts;
    }
    let exe = std::env::current_exe().ok();
    // *Voice* off (spec §7.1.7 Q15): no voice server, as `--voice off`.
    let found = if voice {
        launcher::find_binary(
            None,
            exe.as_deref().and_then(std::path::Path::parent),
            std::env::var_os("PATH").as_deref(),
        )
    } else {
        Err(ocw_client::host::VOICE_TURNED_OFF.to_owned())
    };
    match found {
        Ok(binary) => {
            // OCW_HOST_VOICE_PORTS=SIGNAL,TCP,UDP: other ports than 7880,
            // 7881, 7882 (another voice server on this machine, tests).
            let ports = match std::env::var("OCW_HOST_VOICE_PORTS") {
                Ok(p) => launcher::VoicePorts::parse(&p).unwrap_or_else(|e| {
                    warn!("OCW_HOST_VOICE_PORTS: {e}; using the default ports");
                    launcher::VoicePorts::default()
                }),
                Err(_) => launcher::VoicePorts::default(),
            };
            opts.voice = Some(VoicePlan {
                launch: LaunchOptions {
                    ports,
                    ..LaunchOptions::new(binary, launcher::default_state_dir())
                },
                public_host: None,
            });
        }
        Err(e) if !voice => {
            info!("hosting without voice: {e}");
            opts.voice_off = e;
        }
        Err(e) => {
            warn!("hosting without voice: {e}");
            opts.voice_off = e;
        }
    }
    if upnp {
        opts.upnp = Some(ocw_server::portmap::upnp::discover());
    }
    opts
}

/// Whether a host start may begin its session: the voice server and the
/// router's ports are done (started or failed). The ride module waits
/// for it after loading the course.
pub(super) fn services_ready(world: &World) -> bool {
    world
        .get_resource::<Starting>()
        .and_then(|s| s.services.as_ref())
        .is_none_or(HostServices::is_ready)
}

/// The hosting's network for the pages (the shared `HostNet`).
pub fn net_view(world: &World, port: u16) -> HostNet {
    let status = world
        .get_resource::<HostRun>()
        .and_then(|r| r.0.as_ref())
        .or_else(|| {
            world
                .get_resource::<Starting>()
                .and_then(|s| s.services.as_ref())
        })
        .map(HostServices::status);
    net_of(status.as_ref(), port)
}

/// [`HostNet`] from the hosting services' status.
pub fn net_of(status: Option<&HostStatus>, port: u16) -> HostNet {
    let Some(st) = status else {
        return HostNet {
            game_udp: port,
            web_udp: None,
            voice: HostVoice::Off("not started".into()),
            router: HostRouter::Off,
        };
    };
    let voice = match &st.voice {
        VoiceState::On(p) => HostVoice::On {
            signal: p.signal,
            tcp: p.tcp,
            udp: p.udp,
        },
        VoiceState::Starting => HostVoice::Starting,
        VoiceState::Off(why) | VoiceState::Failed(why) => HostVoice::Off(why.clone()),
    };
    let router = match &st.router {
        MapState::Off => HostRouter::Off,
        MapState::Searching => HostRouter::Searching,
        MapState::NotAvailable(why) => HostRouter::NotAvailable(why.clone()),
        MapState::Failed(why) => HostRouter::Failed(why.clone()),
        m @ MapState::Mapped {
            mapped,
            failed,
            external,
            not_public,
            ..
        } => {
            let mut notes: Vec<String> = failed
                .iter()
                .map(|(p, why)| format!("{p} not opened: {why}."))
                .collect();
            if *not_public && let Some(ip) = external {
                notes.push(format!(
                    "The router's own address {ip} is not public (double NAT or carrier-grade NAT): riders from the internet still cannot reach this computer."
                ));
            }
            HostRouter::Opened {
                public: m.public_ip().map(|ip| ip.to_string()),
                ports: mapped.len(),
                of: mapped.len() + failed.len(),
                note: (!notes.is_empty()).then(|| notes.join(" ")),
            }
        }
    };
    HostNet {
        game_udp: port,
        web_udp: None,
        voice,
        router,
    }
}

/// `host status`.
pub fn status(world: &World) -> String {
    if world.contains_resource::<Starting>() {
        let s = world.resource::<Starting>();
        let st = s.services.as_ref().map(HostServices::status);
        let mut out = "hosting: loading the course …".to_owned();
        if let Some(st) = st {
            out.push_str(&format!(
                "\n{}\n{}",
                st.voice.summary(),
                st.router.summary()
            ));
        }
        return out;
    }
    match world.get_resource::<Hosting>() {
        Some(h) => status_of(h, riders(world), &net_view(world, h.port)),
        None => "not hosting (Ride -> Host a ride, host start)".into(),
    }
}

/// The menu's ride mode: hosting, joined or alone.
fn ride_mode(
    hosting: Option<Res<Hosting>>,
    counts: Option<Res<HostCounts>>,
    args: Res<crate::Args>,
    mut mode: ResMut<ocw_ui::menu::MenuRideMode>,
) {
    use ocw_client::menu::model::RideMode;
    let m = if hosting.is_some() {
        RideMode::Hosting {
            riders: counts.map_or(1, |c| c.0.riders.load(Ordering::Relaxed).max(1) as u32),
        }
    } else if args.server.is_some() {
        RideMode::Joined
    } else {
        RideMode::Alone
    };
    if mode.0 != m {
        mode.0 = m;
    }
}

/// The rows of the wizard and of the join link page.
pub fn rows(world: &World, page: Page) -> Option<Vec<Row>> {
    let plan = shown_plan(world);
    match page {
        Page::HostLink => {
            let h = world.get_resource::<Hosting>()?;
            // At most two links (the internet's and the first LAN one;
            // `host status` lists all), the ports and the router: the
            // page stays within the ten-foot rule.
            let mut out: Vec<Row> = h
                .links
                .iter()
                .take(2)
                .map(|l| ocw_client::menu::pages::info_row(&short_link(l), None))
                .collect();
            out.extend(net_view(world, h.port).rows());
            out.push(Row::action("Copy link", "host copy"));
            out.push(Row {
                label: "Stop hosting".into(),
                value: None,
                left: None,
                right: None,
                confirm: Some(Activate::Command("host stop".into())),
            });
            Some(out)
        }
        Page::HostWorld
        | Page::HostRoute
        | Page::HostTime
        | Page::HostWeather
        | Page::HostOptions
        | Page::HostSummary => {
            let empty = super::pages::Catalog::default();
            let c = world.get_resource::<RideCatalog>().map_or(&empty, |c| &c.0);
            // Sign-in on a hosted server needs the accounts backend's
            // keys in the app (not yet): the row stays hidden.
            Some(ocw_client::host::rows(page, &plan, &worlds_of(c), false))
        }
        _ => None,
    }
}

/// The link page's text: the links in full, the ports, the router and
/// the firewall hint; the summary's: the ports it will open.
pub fn text(world: &World, page: Page) -> Vec<String> {
    match (page, world.get_resource::<Hosting>()) {
        (Page::HostSummary, _) => ocw_client::host::summary_lines(&shown_plan(world)),
        (Page::HostLink, Some(h)) => {
            let mut out = vec![format!(
                "{} rider(s) on UDP {}. Others join with Ride -> Join and this link (Copy link):",
                riders(world),
                h.port
            )];
            // The first link in full, in pieces the panel can wrap (a
            // fingerprint has no spaces); `host status` prints them all.
            out.extend(h.links.first().map(|l| link_lines(l)));
            out.extend(net_view(world, h.port).lines());
            out
        }
        _ => Vec::new(),
    }
}

/// A link with spaces every 32 characters, so the panel can wrap it.
pub fn link_lines(l: &str) -> String {
    let chars: Vec<char> = l.chars().collect();
    chars
        .chunks(32)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join(" ")
}

/// A link cut for a row: host and the start of the fingerprint.
fn short_link(l: &str) -> String {
    ocw_client::menu::pages::short_label(l, ocw_client::menu::pages::MAX_ROW_CHARS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_port_in_use_offers_to_join_the_server_there() {
        let m = bind_error(
            47800,
            "cannot listen on 0.0.0.0:47800: Address already in use (os error 98)",
        );
        assert!(
            m.contains("in use") && m.contains("join ocw://localhost:47800"),
            "{m}"
        );
        let m = bind_error(
            47800,
            "cannot listen on 0.0.0.0:47800: Permission denied (os error 13)",
        );
        assert!(m.starts_with("cannot open UDP port 47800"), "{m}");
    }

    #[test]
    fn worlds_and_plans() {
        let worlds = vec![
            HostWorld::test_routes(),
            HostWorld {
                label: "Fictional Valley".into(),
                spec: Some("fiction/valley@2".into()),
                routes: vec!["Lake Loop".into()],
                free_ride: true,
            },
        ];
        assert_eq!(find_world(&worlds, "fiction/valley"), Some(1));
        assert_eq!(find_world(&worlds, "fictional valley"), Some(1));
        assert_eq!(find_world(&worlds, "0"), Some(0));
        assert_eq!(find_world(&worlds, "9"), None);
        let mut plan = HostPlan::default();
        assert_eq!(
            plan_choice(&plan, &worlds).unwrap().0,
            RideChoice::TestRoute("hills".into())
        );
        plan.apply(&["world", "1"], &worlds).unwrap();
        assert_eq!(
            plan_choice(&plan, &worlds).unwrap().0,
            RideChoice::World {
                world: "fiction/valley@2".into(),
                mode: WorldMode::FreeRide { start: None }
            }
        );
        plan.apply(&["route", "Lake", "Loop"], &worlds).unwrap();
        assert!(matches!(
            plan_choice(&plan, &worlds).unwrap().0,
            RideChoice::World { mode: WorldMode::Route(r), .. } if r == "Lake Loop"
        ));
        // The hint (spec §6.2, §7.1.6 V2): the ports, firewall and router.
        let net = net_of(None, 47800);
        assert!(net.lines()[0].starts_with("Ports: UDP 47800 (game)"));
        assert!(net.lines()[1].contains("forward them on the router"));
    }
}
