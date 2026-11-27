//! Ride → Join in both apps (spec §3.2.1 *Join*, §6.2 *Join links*): the
//! page's text fields and their typing, the recent servers and the
//! console commands. Connecting is the app's ([`JoinHooks`]): the desktop
//! opens a QUIC (or WebTransport) connection and switches the ride at
//! runtime, the browser joins over WebTransport.
//!
//! - [`JoinUi`]: the fields, the [`JoinFlow`], the recent list and its
//!   store (the desktop `<config>/ocw/joins.txt`, the browser
//!   `localStorage`).
//! - [`capture`]: while a field is being typed into, this frame's keys
//!   are the field's (typed text, ← / →, Home / End, Backspace / Delete,
//!   Ctrl+V / ⌘V, Enter / Esc end); a gamepad turns the character wheel
//!   (↑ / ↓), moves (← / →), deletes (B / X) and ends (A / Start).
//! - Console: `join [LINK|N] [--cert HEX]` (alone: the recent list),
//!   `leave`, `ride.join` / `ride.leave` (the same), and the page's
//!   `join.edit link|fingerprint`, `join.paste`, `join.go`, `join.cancel`.

use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::prelude::*;
use clap::{Arg, Command};
use ocw_client::join::{JoinFailure, RecentJoins};
use ocw_client::menu::join::{Field, JoinPageInfo};
use ocw_protocol::join::JoinLink;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, ConsoleState, Permission};

use crate::settings::TextStore;

/// What the app does for a join.
#[derive(Resource, Clone, Copy)]
pub struct JoinHooks {
    /// Starts connecting to the validated link (the flow is `Connecting`);
    /// an `Err` fails the join at once.
    pub connect: fn(&mut World, JoinLink) -> Result<String, String>,
    /// Stops a join in progress (closes its connection).
    pub cancel: fn(&mut World),
    /// Leaves the shared ride: riding alone on the same course.
    pub leave: fn(&mut World) -> Result<String, String>,
    /// The clipboard's text (the desktop's clipboard, the browser's last
    /// paste event).
    pub paste: fn(&mut World) -> Option<String>,
    /// Joins the public server (spec 11.5: its join info, the world
    /// check); `true`: download its world first.
    pub public: fn(&mut World, bool) -> Result<String, String>,
}

impl Default for JoinHooks {
    fn default() -> Self {
        Self {
            connect: |_, _| Err("this app cannot join shared rides".into()),
            cancel: |_| {},
            leave: |_| Err("not in a shared ride".into()),
            paste: |_| None,
            public: |_, _| Err("this app joins the public server from its page".into()),
        }
    }
}

/// The Join page's state and the recent servers' store.
#[derive(Resource, Default)]
pub struct JoinUi {
    pub info: JoinPageInfo,
    pub store: Option<Box<dyn TextStore>>,
}

impl JoinUi {
    /// The recent list from `store`.
    pub fn load(store: Option<Box<dyn TextStore>>) -> Self {
        let recent = store
            .as_ref()
            .and_then(|s| s.load().ok().flatten())
            .map(|t| RecentJoins::parse(&t))
            .unwrap_or_default();
        Self {
            info: JoinPageInfo {
                recent,
                ..Default::default()
            },
            store,
        }
    }

    /// The join succeeded: the server goes to the front of the list.
    pub fn remember(&mut self, link: &JoinLink, world: &str) {
        self.info.recent.push(link, world);
        let text = self.info.recent.to_text();
        if let Some(s) = self.store.as_mut()
            && let Err(e) = s.save(&text)
        {
            warn!("recent servers not saved: {e}");
        }
    }
}

pub struct JoinPlugin;

impl Plugin for JoinPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<JoinUi>()
            .init_resource::<JoinHooks>()
            .add_message::<KeyboardInput>();
        let join_cmd = |name: &'static str| {
            Command::new(name)
                .about("Join a shared ride by its link (ocw://HOST:PORT?cert=...); alone: the recent servers; N: the N-th of them")
                // Quote a pasted `ocw-desktop --server …` line (its words start with --).
                .arg(Arg::new("link").num_args(0..))
                .arg(
                    Arg::new("cert")
                        .long("cert")
                        .value_name("SHA256")
                        .help("The certificate fingerprint when the link has none"),
                )
        };
        for name in ["join", "ride.join"] {
            app.console_command(ConsoleCommand::new(
                Permission::User,
                join_cmd(name),
                |world, m| {
                    let words: Vec<String> = m
                        .get_many::<String>("link")
                        .map(|v| v.cloned().collect())
                        .unwrap_or_default();
                    let cert = m.get_one::<String>("cert").cloned().unwrap_or_default();
                    join_line(world, &words.join(" "), &cert)
                },
            ));
        }
        for name in ["leave", "ride.leave"] {
            app.console_command(ConsoleCommand::new(
                Permission::User,
                Command::new(name)
                    .about("Leave the shared ride: ride on alone on the same world and route"),
                |world, _| leave(world),
            ));
        }
        app.console_command(ConsoleCommand::new(
            Permission::User,
            Command::new("join.public")
                .about("Join the public server (net.public_server; for now ocw-server.fly.dev): fetch its join info, check its world, join")
                .arg(
                    Arg::new("download")
                        .long("download")
                        .action(clap::ArgAction::SetTrue)
                        .help("Download its world first (when the server names a download)"),
                ),
            |world, m| {
                let hooks = *world.resource::<JoinHooks>();
                (hooks.public)(world, m.get_flag("download"))
            },
        ));
        app.console_command(ConsoleCommand::new(
            Permission::User,
            Command::new("join.edit")
                .about("Type into a field of Ride -> Join (Enter or Esc ends)")
                .arg(
                    Arg::new("field")
                        .value_parser(["link", "fingerprint"])
                        .default_value("link"),
                ),
            |world, m| {
                let field = match m.get_one::<String>("field").map(String::as_str) {
                    Some("fingerprint") => Field::Fingerprint,
                    _ => Field::Link,
                };
                let mut ui = world.resource_mut::<JoinUi>();
                ui.info.typing = Some(field);
                let f = field_mut(&mut ui.info, field);
                f.end();
                Ok("typing: Enter or Esc ends, Ctrl+V pastes".into())
            },
        ))
        .console_command(ConsoleCommand::new(
            Permission::User,
            Command::new("join.paste").about("Paste the clipboard into the link field"),
            |world, _| paste_into(world, Field::Link),
        ))
        .console_command(ConsoleCommand::new(
            Permission::User,
            Command::new("join.go").about("Join the link of Ride -> Join"),
            |world, _| {
                let (link, cert) = {
                    let ui = &world.resource::<JoinUi>().info;
                    (ui.link.text.clone(), ui.fingerprint.text.clone())
                };
                join_line(world, &link, &cert)
            },
        ))
        .console_command(ConsoleCommand::new(
            Permission::User,
            Command::new("join.cancel").about("Stop joining (the ride goes on)"),
            |world, _| {
                if !world.resource::<JoinUi>().info.flow.busy() {
                    return Err("no join in progress".into());
                }
                let hooks = *world.resource::<JoinHooks>();
                (hooks.cancel)(world);
                world
                    .resource_mut::<JoinUi>()
                    .info
                    .flow
                    .fail(JoinFailure::Cancelled);
                Ok("not joining".into())
            },
        ));
    }
}

fn field_mut(info: &mut JoinPageInfo, field: Field) -> &mut ocw_client::menu::join::TextField {
    match field {
        Field::Link => &mut info.link,
        Field::Fingerprint => &mut info.fingerprint,
    }
}

fn paste_into(world: &mut World, field: Field) -> Result<String, String> {
    let hooks = *world.resource::<JoinHooks>();
    let text = (hooks.paste)(world).ok_or("the clipboard has no text")?;
    let mut ui = world.resource_mut::<JoinUi>();
    let f = field_mut(&mut ui.info, field);
    if field == Field::Link && ocw_protocol::join::JoinLink::parse(&text).is_ok() {
        // A whole link replaces what was there.
        f.clear();
    }
    f.paste(&text);
    Ok(format!("pasted {} characters", text.trim().chars().count()))
}

/// `join …`: a link (with `cert` when it has none), `N` of the recent
/// list, or nothing (the list).
pub fn join_line(world: &mut World, line: &str, cert: &str) -> Result<String, String> {
    let line = line.trim();
    if line.is_empty() {
        return Ok(world.resource::<JoinUi>().info.recent.listing());
    }
    // `join public` (`--join public`): the public server (spec 11.5).
    if line.eq_ignore_ascii_case("public") {
        let hooks = *world.resource::<JoinHooks>();
        return (hooks.public)(world, false);
    }
    let line = match line.parse::<usize>() {
        Ok(n) => world
            .resource::<JoinUi>()
            .info
            .recent
            .get(n)
            .map(|e| e.link.clone())
            .ok_or_else(|| format!("no recent server {n} (join alone lists them)"))?,
        Err(_) => line.to_owned(),
    };
    let now = crate::now(world);
    let started = {
        let mut ui = world.resource_mut::<JoinUi>();
        ui.info.typing = None;
        // The link of a recent server or the console shows in the field.
        if ui.info.link.text != line {
            ui.info.link = ocw_client::menu::join::TextField::new(&line);
        }
        ui.info.flow.start(&line, cert, now)
    };
    let link = started.map_err(|e| e.message())?;
    let hooks = *world.resource::<JoinHooks>();
    match (hooks.connect)(world, link.clone()) {
        Ok(msg) => Ok(msg),
        Err(e) => {
            let why = if e.contains("WebTransport") {
                JoinFailure::Unsupported(e.clone())
            } else {
                JoinFailure::Invalid(e.clone())
            };
            world.resource_mut::<JoinUi>().info.flow.fail(why);
            Err(e)
        }
    }
}

/// `leave`.
pub fn leave(world: &mut World) -> Result<String, String> {
    let hooks = *world.resource::<JoinHooks>();
    if world.resource::<JoinUi>().info.flow.busy() {
        (hooks.cancel)(world);
    }
    let r = (hooks.leave)(world);
    if r.is_ok() {
        world.resource_mut::<JoinUi>().info.flow.leave();
    }
    r
}

/// Where [`capture`] has read the typed keys up to.
#[derive(Resource, Default)]
struct KeyCursor(bevy::ecs::message::MessageCursor<KeyboardInput>);

/// While a field is typed into: this frame's input is the field's (the
/// menu's own keys wait). `true`: the input was taken.
pub fn capture(world: &mut World) -> bool {
    world.init_resource::<KeyCursor>();
    // The keys since the last frame (read without taking them from the
    // other readers).
    let typed: Vec<KeyboardInput> = world.resource_scope(|world, mut c: Mut<KeyCursor>| {
        world
            .get_resource::<Messages<KeyboardInput>>()
            .map(|m| c.0.read(m).cloned().collect())
            .unwrap_or_default()
    });
    let Some(field) = world.get_resource::<JoinUi>().and_then(|u| u.info.typing) else {
        return false;
    };
    let pads = pads_pressed(world);
    let ctrl = ctrl_held(world);
    let paste = {
        let mut ui = world.resource_mut::<JoinUi>();
        let f = field_mut(&mut ui.info, field);
        let (paste, ended) = edit_field(f, &typed, &pads, ctrl);
        if ended.is_some() {
            ui.info.typing = None;
        }
        paste
    };
    if paste && let Err(e) = paste_into(world, field) {
        world.resource_mut::<ConsoleState>().print(e);
    }
    true
}

/// How typing into a field ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldEnd {
    /// Enter (a pad's A / Start): done.
    Enter,
    /// Esc: cancelled.
    Escape,
}

/// This frame's keys and pad buttons into a text field (shared by the
/// Join page's fields and the ANT+ network key, spec 14 Phase 17): typed
/// text, ← / →, Home / End, Backspace / Delete; a gamepad turns the
/// character wheel (↑ / ↓), moves (← / →), deletes (B / X) and ends (A /
/// Start). Returns whether a paste was asked for (Ctrl+V / ⌘V) and how
/// the typing ended, if it did.
pub fn edit_field(
    f: &mut ocw_client::menu::join::TextField,
    typed: &[KeyboardInput],
    pads: &[GamepadButton],
    ctrl: bool,
) -> (bool, Option<FieldEnd>) {
    let mut paste = false;
    let mut end = None;
    for e in typed.iter().filter(|e| e.state.is_pressed()) {
        match &e.logical_key {
            Key::Enter => end = Some(FieldEnd::Enter),
            Key::Escape => end = Some(FieldEnd::Escape),
            Key::Backspace => f.backspace(),
            Key::Delete => f.delete(),
            Key::ArrowLeft => f.left(),
            Key::ArrowRight => f.right(),
            Key::Home => f.home(),
            Key::End => f.end(),
            Key::Paste => paste = true,
            Key::Character(c) if ctrl && c.eq_ignore_ascii_case("v") => paste = true,
            Key::Character(_) if ctrl => {}
            Key::Space => f.insert(" "),
            _ => {
                if let Some(t) = &e.text
                    && !ctrl
                {
                    f.insert(t);
                }
            }
        }
    }
    for b in pads {
        match b {
            GamepadButton::DPadUp => f.wheel(1),
            GamepadButton::DPadDown => f.wheel(-1),
            GamepadButton::DPadLeft => f.left(),
            GamepadButton::DPadRight => f.right(),
            GamepadButton::East | GamepadButton::West => f.backspace(),
            GamepadButton::South | GamepadButton::Start => end = Some(FieldEnd::Enter),
            _ => {}
        }
    }
    (paste, end)
}

/// Whether Ctrl (⌘) is held.
pub fn ctrl_held(world: &World) -> bool {
    world
        .get_resource::<ButtonInput<KeyCode>>()
        .is_some_and(|k| {
            k.any_pressed([
                KeyCode::ControlLeft,
                KeyCode::ControlRight,
                KeyCode::SuperLeft,
                KeyCode::SuperRight,
            ])
        })
}

/// The pad buttons pressed this frame.
pub fn pads_pressed(world: &mut World) -> Vec<GamepadButton> {
    let mut q = world.query::<&Gamepad>();
    q.iter(world)
        .flat_map(|p| p.get_just_pressed().copied().collect::<Vec<_>>())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_render::console::ConsoleCorePlugin;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Memory(Arc<Mutex<Option<String>>>);
    impl TextStore for Memory {
        fn load(&self) -> Result<Option<String>, String> {
            Ok(self.0.lock().unwrap().clone())
        }
        fn save(&mut self, text: &str) -> Result<(), String> {
            *self.0.lock().unwrap() = Some(text.into());
            Ok(())
        }
        fn describe(&self) -> String {
            "memory".into()
        }
    }

    #[derive(Resource, Default)]
    struct Connected(Vec<String>);

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ConsoleCorePlugin, JoinPlugin))
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<Connected>()
            .insert_resource(JoinHooks {
                connect: |w, l| {
                    w.resource_mut::<Connected>().0.push(l.to_link());
                    Ok(format!("connecting to {}", l.server_spec()))
                },
                cancel: |_| {},
                leave: |_| Ok("riding alone".into()),
                paste: |_| Some("  ocw://localhost:5000\n".into()),
                public: |w, download| {
                    w.resource_mut::<Connected>()
                        .0
                        .push(format!("public download={download}"));
                    Ok("finding the public server".into())
                },
            });
        app.update();
        app
    }

    fn run(app: &mut App, line: &str) -> String {
        app.world_mut().resource_mut::<ConsoleState>().queue(line);
        app.update();
        app.world().resource::<ConsoleState>().log().join("\n")
    }

    fn key(app: &mut App, key: Key, text: Option<&str>) {
        app.world_mut().write_message(KeyboardInput {
            key_code: KeyCode::KeyA,
            logical_key: key,
            state: bevy::input::ButtonState::Pressed,
            text: text.map(Into::into),
            repeat: false,
            window: Entity::PLACEHOLDER,
        });
        capture(app.world_mut());
    }

    #[test]
    fn typing_into_the_link_field_and_ending_with_enter() {
        let mut app = app();
        assert!(!capture(app.world_mut()), "not typing");
        run(&mut app, "join.edit link");
        assert_eq!(
            app.world().resource::<JoinUi>().info.typing,
            Some(Field::Link)
        );
        for c in "localhosx".chars() {
            key(
                &mut app,
                Key::Character(c.to_string().into()),
                Some(&c.to_string()),
            );
        }
        key(&mut app, Key::Backspace, None);
        key(&mut app, Key::Character("t".into()), Some("t"));
        key(&mut app, Key::ArrowLeft, None);
        key(&mut app, Key::Home, None);
        key(&mut app, Key::End, None);
        key(&mut app, Key::Enter, None);
        let ui = &app.world().resource::<JoinUi>().info;
        assert_eq!(ui.link.text, "localhost");
        assert_eq!(ui.typing, None);
    }

    #[test]
    fn paste_and_join_validate_then_connect_and_remember() {
        let mut app = app();
        let log = run(&mut app, "join.paste");
        assert!(log.contains("pasted 20 characters"), "{log}");
        assert_eq!(
            app.world().resource::<JoinUi>().info.link.text,
            "ocw://localhost:5000"
        );
        let log = run(&mut app, "join.go");
        assert!(log.contains("connecting to localhost:5000"), "{log}");
        assert!(app.world().resource::<JoinUi>().info.flow.busy());
        // A second one waits for the first.
        let log = run(&mut app, "join localhost");
        assert!(log.contains("already in progress"), "{log}");
        let log = run(&mut app, "join.cancel");
        assert!(log.contains("not joining"), "{log}");
        // The localhost rule: a remote host needs the fingerprint.
        let log = run(&mut app, "join ocw://10.1.2.3");
        assert!(
            log.contains("needs the server's certificate fingerprint"),
            "{log}"
        );
        assert_eq!(app.world().resource::<Connected>().0.len(), 1);
        let fp = "ab".repeat(32);
        run(&mut app, &format!("join 10.1.2.3 --cert {fp}"));
        assert_eq!(
            app.world().resource::<Connected>().0[1],
            format!("ocw://10.1.2.3?cert={fp}")
        );
        // The recent list and its store.
        let store = Memory::default();
        let mut ui = JoinUi::load(Some(Box::new(store.clone())));
        ui.remember(&JoinLink::new("10.1.2.3", 47800), "route hills");
        assert!(
            store
                .0
                .lock()
                .unwrap()
                .as_deref()
                .unwrap()
                .contains("ocw://10.1.2.3\troute hills")
        );
        assert_eq!(
            JoinUi::load(Some(Box::new(store)))
                .info
                .recent
                .entries
                .len(),
            1
        );
        app.insert_resource(ui);
        let log = run(&mut app, "join");
        assert!(log.contains("1. 10.1.2.3:47800 – route hills"), "{log}");
        run(&mut app, "join.cancel");
        let log = run(&mut app, "join 2");
        assert!(log.contains("no recent server 2"), "{log}");
        let log = run(&mut app, "leave");
        assert!(log.contains("riding alone"), "{log}");
    }

    /// `join public` (`--join public`) and the page's `join.public` go to
    /// the app's public-server hook (spec 11.5); no link is parsed.
    #[test]
    fn join_public_goes_to_the_hook() {
        let mut app = app();
        let log = run(&mut app, "join public");
        assert!(log.contains("finding the public server"), "{log}");
        run(&mut app, "join.public --download");
        assert_eq!(
            app.world().resource::<Connected>().0,
            ["public download=false", "public download=true"]
        );
        assert!(!app.world().resource::<JoinUi>().info.flow.busy());
        // An app without one says so.
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ConsoleCorePlugin, JoinPlugin));
        app.update();
        let log = run(&mut app, "join public");
        assert!(log.contains("from its page"), "{log}");
    }
}
