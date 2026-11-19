//! Ride → Join on the desktop (spec §3.2.1 *Join*, §6.2 *Join links*):
//! joining a shared ride at runtime.
//!
//! 1. `join LINK` (the menu's Join, the console, `--join`) validates the
//!    link (`ocw_client::join::JoinFlow`) and opens a connection in the
//!    background – native QUIC for `ocw://`, WebTransport for `https://` –
//!    with the rider's hello *without* a world, so the server answers
//!    with its `Welcome` whatever it rides. The current ride goes on.
//! 2. The `Welcome` names the course: a test route, or a world package's
//!    road graph (its name and graph fingerprint) with a route or free
//!    ride. The package is looked up among the installed worlds; missing,
//!    the join fails with what to install (a `world=` manifest URL in the
//!    link: `--fetch-world URL`). A user key nobody here trusts asks first
//!    (the trust dialog, `join.continue` / `join.cancel`).
//! 3. The course is loaded like any ride switch (`super::start_load`);
//!    its road graph must be the server's. Then the session switches in
//!    one frame: the probe connection becomes the ride's link
//!    (`connection::PendingLink`) and what it received so far goes to the
//!    new client first (`connection::Replay`; a second hello would be
//!    ignored by the server).
//!
//! Any failure – the 10 s timeout, a certificate that is not the link's,
//! a refusal (sign-in: Settings → Account), a missing or other world –
//! keeps the current ride and is shown on the Join page. `leave` rides
//! on alone on the same world and route ([`super::restart_alone`]).

use bevy::prelude::*;
use ocw_client::join::{JoinFailure, JoinState, Replay, ServerCourse};
use ocw_protocol::join::{JoinLink, LinkTransport};
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, ConsoleState, Permission};
use ocw_server::net::{ClientOptions, LinkStatus, NetClient, ServerTrust};
use ocw_ui::join::{JoinHooks, JoinUi};

use super::choice::{RideChoice, WorldMode};
use super::{Follow, RideCatalog, Then};
use crate::Args;
use crate::connection::{PendingLink, ServerLink};

/// The join in progress: its connection and what it received.
#[derive(Resource)]
pub struct Probe {
    client: std::sync::Mutex<NetClient>,
    replay: Replay,
    link: JoinLink,
    /// The course, once the server's `Welcome` came.
    course: Option<ServerCourse>,
    /// The ride chosen for it, waiting for the trust dialog.
    waiting: Option<RideChoice>,
}

pub fn build(app: &mut App) {
    // The recent servers: `ocw.toml [servers]` (spec 3.2.4 Q5; the old
    // `joins.txt` is migrated into it). Tests keep no list (no files
    // outside their directories).
    let store = app
        .world()
        .get_resource::<crate::config::ConfigFiles>()
        .filter(|_| !cfg!(test))
        .map(|f| {
            Box::new(crate::config::ServersStore(f.section("servers")))
                as Box<dyn ocw_ui::settings::TextStore>
        });
    app.insert_resource(JoinUi::load(store))
        .insert_resource(JoinHooks {
            connect,
            cancel,
            leave,
            paste: |_| clipboard_text(),
        })
        .add_systems(Update, pump);
    app.console_command(ConsoleCommand::new(
        Permission::User,
        clap::Command::new("join.continue")
            .about("Go on joining after the trust dialog (the server's world key)"),
        |world, _| {
            let choice = world
                .get_resource_mut::<Probe>()
                .and_then(|mut p| p.waiting.take())
                .ok_or("no join is waiting")?;
            load(world, choice)
        },
    ));
}

/// The clipboard's text (Ctrl+V, *Paste link*).
pub fn clipboard_text() -> Option<String> {
    arboard::Clipboard::new().ok()?.get_text().ok()
}

/// Puts `text` on the clipboard (Copy link).
pub fn set_clipboard(text: &str) -> Result<(), String> {
    arboard::Clipboard::new()
        .and_then(|mut c| c.set_text(text.to_owned()))
        .map_err(|e| format!("no clipboard here: {e}"))
}

/// How the link's server is trusted: its fingerprint, or (this machine
/// only) any certificate.
fn trust_of(link: &JoinLink, loopback: bool) -> Result<ServerTrust, String> {
    match &link.cert {
        Some(c) => Ok(ServerTrust::Pinned(
            ocw_server::net::tls::parse_fingerprint(c)?,
        )),
        None if loopback => Ok(ServerTrust::InsecureLoopback),
        None => Err(link.check_trust().err().unwrap_or_default()),
    }
}

fn connect(world: &mut World, link: JoinLink) -> Result<String, String> {
    if world.contains_resource::<super::host::Hosting>() {
        return Err("you host a shared ride: stop hosting before joining another".into());
    }
    if world.resource::<super::Pending>().0.is_some() {
        return Err("a ride is being loaded; join when it is ready".into());
    }
    let client = match link.transport {
        LinkTransport::Quic => {
            let (addr, name) = ocw_server::net::resolve(&link.server_spec())?;
            let trust = trust_of(&link, addr.ip().is_loopback())?;
            NetClient::connect(addr, &name, &trust, ClientOptions::default())?
        }
        LinkTransport::WebTransport => {
            let trust = trust_of(&link, link.is_loopback())?;
            NetClient::connect_web(&link.web_url(), &trust, ClientOptions::default())?
        }
    };
    client.send(crate::connection::rider_hello(world));
    info!(server = %link.server_spec(), "joining a shared ride");
    let msg = format!("connecting to {} …", link.server_spec());
    world.insert_resource(Probe {
        client: std::sync::Mutex::new(client),
        replay: Replay::default(),
        link,
        course: None,
        waiting: None,
    });
    Ok(msg)
}

fn cancel(world: &mut World) {
    if let Some(p) = world.remove_resource::<Probe>()
        && let Ok(c) = p.client.into_inner()
    {
        c.close();
    }
}

fn leave(world: &mut World) -> Result<String, String> {
    if world.resource::<Args>().server.is_none() {
        return Err("not in a shared ride".into());
    }
    super::restart_alone(world);
    let msg = "left the shared ride: riding on alone".to_owned();
    info!("{msg}");
    Ok(msg)
}

/// Ends the join with `why` (the ride goes on).
fn fail(world: &mut World, why: JoinFailure) {
    cancel(world);
    let msg = why.message();
    warn!("join failed: {msg}");
    world.resource_mut::<JoinUi>().info.flow.fail(why);
    world
        .resource_mut::<ConsoleState>()
        .print(format!("not joined: {msg}"));
}

/// Each frame: the probe's messages, its state, the timeout.
fn pump(world: &mut World) {
    if !world.contains_resource::<Probe>() {
        return;
    }
    let now = world
        .get_resource::<Time>()
        .map_or(0.0, |t| t.elapsed_secs_f64());
    if world.resource_mut::<JoinUi>().info.flow.tick(now) {
        let why = JoinFailure::Timeout;
        fail(world, why);
        return;
    }
    let (messages, status) = {
        let p = world.resource::<Probe>();
        let c = p.client.lock().expect("not poisoned");
        (c.receive(), c.status())
    };
    let mut learned = None;
    for bytes in messages {
        if let Some(course) = world.resource_mut::<JoinUi>().info.flow.on_message(&bytes) {
            learned = Some(course);
        }
        world.resource_mut::<Probe>().replay.push(bytes);
    }
    if let JoinState::Failed { why, .. } = &world.resource::<JoinUi>().info.flow.state {
        let why = why.clone();
        fail(world, why);
        return;
    }
    if let LinkStatus::Closed(reason) = status {
        world.resource_mut::<JoinUi>().info.flow.on_closed(&reason);
        let why = match &world.resource::<JoinUi>().info.flow.state {
            JoinState::Failed { why, .. } => why.clone(),
            _ => JoinFailure::Closed(reason),
        };
        fail(world, why);
        return;
    }
    if let Some(course) = learned {
        info!(course = %course.describe(), "the server's course");
        world.resource_mut::<Probe>().course = Some(course.clone());
        match course_choice(world, &course) {
            Ok(choice) => {
                if let Err(e) = consent_then_load(world, choice) {
                    fail(world, JoinFailure::Load(e));
                }
            }
            Err(why) => fail(world, why),
        }
    }
}

/// The local ride for the server's course.
fn course_choice(world: &mut World, course: &ServerCourse) -> Result<RideChoice, JoinFailure> {
    let link = world.resource::<Probe>().link.clone();
    course_choice_for(world, &link, course)
}

/// [`course_choice`] for `link`.
pub(super) fn course_choice_for(
    world: &mut World,
    link: &JoinLink,
    course: &ServerCourse,
) -> Result<RideChoice, JoinFailure> {
    let Some((name, _)) = &course.world else {
        let test = super::choice::TEST_ROUTES
            .iter()
            .find(|r| super::setup::test_route(r).is_some_and(|t| t.id() == course.route_id));
        return match test {
            Some(r) => Ok(RideChoice::TestRoute((*r).to_owned())),
            None => Err(JoinFailure::WorldMissing(format!(
                "the server rides the route file {:?}, which this app cannot fetch; start with the same file: ocw-desktop --route-file FILE --join LINK",
                course.route_id
            ))),
        };
    };
    super::refresh_catalog(world);
    let worlds = world.resource::<RideCatalog>().0.worlds.clone();
    let wanted = link.world_id();
    let candidates: Vec<_> = worlds
        .iter()
        .flat_map(ocw_worldstore::discover::WorldEntry::members)
        .filter(|e| e.is_rideable())
        .filter(|e| e.summary.as_ref().is_some_and(|s| &s.name == name))
        .collect();
    // The link's id and version first, then the newest found.
    let entry = candidates
        .iter()
        .find(|e| wanted.is_some_and(|(id, v)| e.id == id && v.is_none_or(|v| e.version == v)))
        .or(candidates.first())
        .copied();
    let Some(entry) = entry else {
        let what = link
            .world
            .as_deref()
            .map_or_else(|| name.clone(), |w| format!("{name} ({w})"));
        let how = match link.world_url() {
            Some(url) => format!(
                "fetch it first: ocw-desktop --fetch-world {url} (or install it and use World -> Change world)"
            ),
            None => "install the same world package (World -> Change world lists those found), or ask the host for a link with world=MANIFEST-URL".into(),
        };
        return Err(JoinFailure::WorldMissing(format!(
            "the server rides {what}, which is not installed here: {how}"
        )));
    };
    Ok(RideChoice::World {
        world: entry.spec.clone(),
        mode: if course.graph_route.is_empty() {
            WorldMode::FreeRide { start: None }
        } else {
            WorldMode::Route(course.graph_route.clone())
        },
    })
}

/// The trust dialog first for a world signed by an unknown user key
/// (spec §4.7 *User keys*), then the load.
fn consent_then_load(world: &mut World, choice: RideChoice) -> Result<(), String> {
    if let RideChoice::World { world: spec, .. } = &choice {
        let opts = crate::worlds::load_options(world.resource::<Args>())?;
        match ocw_worldstore::consent_needed(spec, &opts) {
            Ok(Some(request)) => {
                world.resource_mut::<Probe>().waiting = Some(choice);
                let msg = ocw_ui::trust::ask(
                    world,
                    request,
                    Some("join.continue".into()),
                    Some("join.cancel".into()),
                    true,
                );
                world.resource_mut::<ConsoleState>().print(msg);
                return Ok(());
            }
            Ok(None) | Err(ocw_worldstore::LoadError::NotFound(..)) => {}
            Err(e) => return Err(format!("REFUSED: {e}")),
        }
    }
    load(world, choice).map(|_| ())
}

fn load(world: &mut World, choice: RideChoice) -> Result<String, String> {
    let course = world
        .get_resource::<Probe>()
        .and_then(|p| p.course.clone())
        .ok_or("no join in progress")?;
    let follow = Follow {
        name: course
            .world
            .as_ref()
            .map_or_else(|| course.route_id.clone(), |(n, _)| n.clone()),
        sha256: Vec::new(),
        graph_hash: course.world.as_ref().map(|(_, h)| *h),
        then: Then::Join,
    };
    super::start_load(world, choice, Some(follow))
}

/// The course is loaded: the probe becomes the next session's link and
/// the options say where the ride is (`--server`).
pub(super) fn take_link(world: &mut World, args: &mut Args) -> Result<(), String> {
    let p = world
        .remove_resource::<Probe>()
        .ok_or("the join was cancelled")?;
    let client = p
        .client
        .into_inner()
        .map_err(|_| "the join link broke".to_string())?;
    if let LinkStatus::Closed(reason) = client.status() {
        let why = JoinFailure::from_closed(&reason);
        world.resource_mut::<JoinUi>().info.flow.fail(why.clone());
        return Err(format!("not joined: {}", why.message()));
    }
    args.server = Some(p.link.server_spec());
    args.server_cert = p.link.cert.clone();
    args.insecure_dev = p.link.cert.is_none();
    world.resource_mut::<PendingLink>().0 = Some(std::sync::Mutex::new((
        ServerLink::Remote(Box::new(client)),
        p.replay.messages,
    )));
    Ok(())
}

/// Riding on the server now.
pub(super) fn switched(world: &mut World) {
    world.resource_mut::<ConsoleState>().permission = Permission::User;
    let mut ui = world.resource_mut::<JoinUi>();
    ui.info.flow.switched();
    let (link, course) = match &ui.info.flow.state {
        JoinState::Joined { link, course } => (link.clone(), course.describe()),
        _ => return,
    };
    ui.remember(&link, &course);
    let msg = format!("joined {}: {course}", link.server_spec());
    info!("{msg}");
    world.resource_mut::<ConsoleState>().print(msg);
}

pub(super) fn load_failed(world: &mut World, e: &str) {
    let why = if e.contains("differs from the server") {
        JoinFailure::WorldMismatch(e.to_owned())
    } else {
        JoinFailure::Load(e.to_owned())
    };
    fail(world, why);
}

/// `--join LINK`: the line to run at the start.
pub fn startup_line(link: &str) -> String {
    format!("join \"{}\"", link.replace('"', ""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_startup_line_quotes_the_link() {
        assert_eq!(
            startup_line("ocw://h?cert=ab&route=Lake Loop"),
            "join \"ocw://h?cert=ab&route=Lake Loop\""
        );
    }
}
