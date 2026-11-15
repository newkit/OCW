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

use std::sync::Mutex;
use std::sync::atomic::Ordering;

use bevy::prelude::*;
use clap::{Arg, Command, value_parser};
use ocw_client::host::{HostPlan, HostWorld};
use ocw_client::menu::pages::{Activate, Page, Row};
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, ConsoleState, Permission};
use ocw_server::net::{NetServer, ServerIdentity};

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
}

pub fn build(app: &mut App) {
    app.init_resource::<HostPlanState>()
        .add_systems(Update, ride_mode);
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("host.plan")
            .about("Change the plan of Ride -> Host a ride (world N, route NAME|free, time, day, weather, bots, open, port)")
            .arg(Arg::new("words").num_args(1..).required(true).allow_hyphen_values(true)),
        |world, m| {
            let words: Vec<String> = m
                .get_many::<String>("words")
                .map(|v| v.cloned().collect())
                .unwrap_or_default();
            let words: Vec<&str> = words.iter().map(String::as_str).collect();
            let worlds = host_worlds(world);
            world.resource_mut::<HostPlanState>().0.apply(&words, &worlds)
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
                    .arg(Arg::new("port").long("port").value_parser(value_parser!(u16))),
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
                for l in lines {
                    let words: Vec<&str> = l.split_whitespace().collect();
                    world.resource_mut::<HostPlanState>().0.apply(&words, &worlds)?;
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
        "hosting: the port is open, loading the course"
    );
    world.insert_resource(Starting {
        net: Mutex::new(Some(net)),
        plan: plan.clone(),
        world_id,
        fingerprint: net_fingerprint,
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
            world.remove_resource::<Starting>();
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
    };
    world.resource_mut::<HostStart>().0 = Some(Mutex::new(next));
    Ok(())
}

/// Hosting: the links, the plan's time and weather.
pub(super) fn started(world: &mut World) {
    let Some(s) = world.remove_resource::<Starting>() else {
        return;
    };

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
    let links =
        ocw_server::net::join_links(local, &fingerprint, world_id.as_deref(), route.as_deref());
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
    let msg = status_of(&h, 1);
    info!("{msg}");
    console.print(msg);
    world.insert_resource(h);
}

pub(super) fn load_failed(world: &mut World, e: &str) {
    // Dropping the endpoint closes the port again.
    world.remove_resource::<Starting>();
    let msg = format!("not hosting: cannot load the ride: {e}");
    warn!("{msg}");
    world.resource_mut::<ConsoleState>().print(msg);
}

/// `host stop`.
pub fn stop(world: &mut World) -> Result<String, String> {
    if world.remove_resource::<Starting>().is_some() {
        return Ok("hosting cancelled".into());
    }
    let h = world.remove_resource::<Hosting>().ok_or("not hosting")?;
    // The new local session ends the hosted server's thread (its link is
    // dropped): the others hear "server shutting down".
    super::restart_alone(world);
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

fn status_of(h: &Hosting, riders: u32) -> String {
    let mut out = format!(
        "hosting on UDP {} ({riders} rider{}); others join with:\n",
        h.port,
        if riders == 1 { "" } else { "s" }
    );
    for l in &h.links {
        out.push_str(&format!("  {l}\n"));
    }
    out.push_str(&format!("certificate SHA-256 {}\n", h.fingerprint));
    out.push_str(&firewall_hint(h.port));
    out
}

/// What others may need (spec §6.2; README *Riding on a server on the
/// internet*).
pub fn firewall_hint(port: u16) -> String {
    format!(
        "Others may need UDP {port} open in this computer's firewall; over the internet also a port forwarding of UDP {port} on the router (README: Riding on a server on the internet)."
    )
}

/// `host status`.
pub fn status(world: &World) -> String {
    if world.contains_resource::<Starting>() {
        return "hosting: loading the course …".into();
    }
    match world.get_resource::<Hosting>() {
        Some(h) => status_of(h, riders(world)),
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
    let plan = world
        .get_resource::<HostPlanState>()
        .map(|p| p.0.clone())
        .unwrap_or_default();
    match page {
        Page::HostLink => {
            let h = world.get_resource::<Hosting>()?;
            let mut out: Vec<Row> = h
                .links
                .iter()
                .map(|l| ocw_client::menu::pages::info_row(&short_link(l), None))
                .collect();
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

/// The link page's text: the links in full and the firewall hint.
pub fn text(world: &World, page: Page) -> Vec<String> {
    match (page, world.get_resource::<Hosting>()) {
        (Page::HostLink, Some(h)) => {
            let mut out = vec![format!(
                "{} rider(s) on UDP {}. Others join with Ride -> Join and this link:",
                riders(world),
                h.port
            )];
            out.extend(h.links.iter().cloned());
            out.push(firewall_hint(h.port));
            out
        }
        _ => Vec::new(),
    }
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
        assert!(firewall_hint(47800).contains("UDP 47800"));
    }
}
