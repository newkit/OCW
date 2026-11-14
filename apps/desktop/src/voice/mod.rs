//! Voice chat in the desktop app (spec §2.4, §7, M8): Bevy glue around
//! [`ocw_voice::VoiceController`].
//!
//! Always on, like a call: on a shared ride the app asks the game server
//! for a LiveKit token and joins the voice room with an open microphone;
//! solo rides do not connect. `M` / gamepad Select mute the microphone,
//! `Shift+M` / R3 the speaker (all voices); the state is kept in
//! `settings.toml` (`[voice]`). The menu page is [`page`].
//!
//! With the cargo feature `voice` off (the default) the controller runs on
//! the null backend: the switches, settings and menu work, the status says
//! voice is not built in.

use std::sync::Mutex;

use bevy::prelude::*;
use clap::{Arg, ArgAction, Command};
use ocw_protocol::v1;
use ocw_render::RideView;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, ConsoleState, Permission};
use ocw_render::menu::ride_input;
use ocw_render::voice::VoiceHud;
use ocw_voice::hotkeys::{Input, Toggle, toggle};
use ocw_voice::{MuteState, VoiceController, VoiceSettings};

use ocw_client::voice::{self as voice_cmd, SettingsStore};

use crate::connection::{ClientState, Link, SessionStart};
use crate::settings::UserSettings;

/// Grants from the game server, handed over by the connection.
#[derive(Resource, Default)]
pub struct PendingVoiceGrants(pub Vec<v1::VoiceGrant>);

/// The voice controller (its backend is not `Sync`, hence the mutex).
#[derive(Resource)]
pub struct Voice(pub Mutex<VoiceController>);

/// What the menu's voice pages show, refreshed every frame (shared with
/// the browser app: `ocw_ui::menu::VoiceMenu`).
pub use ocw_ui::menu::VoiceMenu as VoiceMenuInfo;

pub struct VoicePlugin;

impl Plugin for VoicePlugin {
    fn build(&self, app: &mut App) {
        let mut controller = VoiceController::new(
            ocw_voice::default_backend(),
            VoiceSettings::default(),
            MuteState::default(),
        );
        // `--echo-bot`: the echo test from the start (the settings are
        // applied at the start only where they differ from the defaults).
        if app
            .world()
            .get_resource::<crate::Args>()
            .is_some_and(|a| a.echo_bot)
        {
            controller.set_echo(true);
        }
        app.insert_resource(Voice(Mutex::new(controller)))
            .init_resource::<PendingVoiceGrants>()
            .init_resource::<VoiceMenuInfo>()
            .init_resource::<VoiceHud>()
            .add_systems(Update, (hotkeys.run_if(ride_input), drive).chain());
        register_commands(app);
    }
}

// ------------------------------------------------------------- hotkeys

/// The Bevy inputs of the voice hotkeys (the mapping is
/// [`ocw_voice::hotkeys::toggle`]).
pub fn key_input(m_pressed: bool, shift_held: bool) -> Option<Input> {
    m_pressed.then_some(Input::KeyM { shift: shift_held })
}

pub fn pad_input(button: GamepadButton) -> Option<Input> {
    match button {
        GamepadButton::Select => Some(Input::PadSelect),
        GamepadButton::RightThumb => Some(Input::PadRightStick),
        _ => None,
    }
}

fn hotkeys(
    keys: Option<Res<ButtonInput<KeyCode>>>,
    pads: Query<&Gamepad>,
    mut console: ResMut<ConsoleState>,
) {
    let mut toggles = Vec::new();
    if let Some(k) = keys {
        let shift = k.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
        toggles.extend(key_input(k.just_pressed(KeyCode::KeyM), shift).map(toggle));
    }
    for pad in &pads {
        for b in [GamepadButton::Select, GamepadButton::RightThumb] {
            if pad.just_pressed(b) {
                toggles.extend(pad_input(b).map(toggle));
            }
        }
    }
    for t in toggles {
        console.queue(t.command());
    }
}

// ------------------------------------------------------------- the ride

/// The shared per-frame voice logic (`ocw_client::voice::VoiceDriver`):
/// grants in, token and echo requests out to the game server, the HUD and
/// the menu page.
#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
fn drive(
    time: Res<Time>,
    voice: Res<Voice>,
    client: Option<Res<ClientState>>,
    session: Option<Res<SessionStart>>,
    link: Option<Res<Link>>,
    view: Option<Res<RideView>>,
    mut grants: ResMut<PendingVoiceGrants>,
    mut menu: ResMut<VoiceMenuInfo>,
    mut hud: ResMut<VoiceHud>,
    mut console: ResMut<ConsoleState>,
    mut driver: Local<ocw_client::voice::VoiceDriver>,
) {
    let mut c = voice.0.lock().expect("not poisoned");
    let frame = driver.frame(
        &mut c,
        client.as_deref().map(|c| &c.0),
        session.as_ref().map_or(0, |s| s.0.to_bits()),
        std::mem::take(&mut grants.0),
        view.as_deref(),
        time.elapsed_secs_f64(),
    );
    if let Some(link) = &link {
        for bytes in frame.send {
            link.send_bytes(bytes);
        }
    }
    for m in frame.console {
        info!("{m}");
        console.print(m);
    }
    menu.set_if_neq(VoiceMenuInfo(frame.menu));
    hud.set_if_neq(frame.hud);
}

// ------------------------------------------------------------- commands

fn switch_arg() -> Arg {
    Arg::new("state")
        .value_parser(["on", "off", "toggle"])
        .help("on = muted, off = not muted; none: toggle")
}

/// Runs `f` with the voice controller and the user's settings (if the app
/// has them).
fn with_voice<T>(
    world: &mut World,
    f: impl FnOnce(&mut VoiceController, Option<&mut dyn SettingsStore>) -> T,
) -> T {
    if world.contains_resource::<UserSettings>() {
        world.resource_scope(|world, mut us: Mut<UserSettings>| {
            let voice = world.resource::<Voice>();
            let mut c = voice.0.lock().expect("not poisoned");
            f(&mut c, Some(&mut *us))
        })
    } else {
        let voice = world.resource::<Voice>();
        let mut c = voice.0.lock().expect("not poisoned");
        f(&mut c, None)
    }
}

/// Sets a mute switch: saved in `settings.toml` (remembered across rides
/// and restarts) and applied to the call.
fn set_mute(world: &mut World, which: Toggle, arg: Option<&str>) -> Result<String, String> {
    with_voice(world, |c, store| {
        voice_cmd::mute_command(c, store, which, arg)
    })
}

/// Applies the `[voice]` settings to the controller (the settings' apply
/// command, at the start and on every change).
fn apply_settings(world: &mut World) {
    let Some(us) = world.get_resource::<UserSettings>() else {
        return;
    };
    let settings = us.settings.clone();
    let echo_bot = world
        .get_resource::<crate::Args>()
        .is_some_and(|a| a.echo_bot);
    let voice = world.resource::<Voice>();
    let mut c = voice.0.lock().expect("not poisoned");
    voice_cmd::apply_settings(&mut c, &settings, echo_bot);
}

/// `voice.echo [on|off|here|delay MS|status]` (spec §7 *Echo bot*).
fn echo_command(
    world: &mut World,
    action: Option<&str>,
    ms: Option<u32>,
) -> Result<String, String> {
    let connected = world.contains_resource::<Link>();
    let (text, send) = with_voice(world, |c, store| {
        voice_cmd::echo_command(c, store, action, ms, connected)
    })?;
    if let (Some(bytes), Some(link)) = (send, world.get_resource::<Link>()) {
        link.send_bytes(bytes);
    }
    Ok(text)
}

fn register_commands(app: &mut App) {
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("voice.echo")
            .about("Echo test: an Echo rider plays your microphone back to you, delayed, when you are near it (on, off, here = 20 m ahead of you, delay MS, status)")
            .arg(Arg::new("action").value_parser(["on", "off", "here", "delay", "status"]))
            .arg(Arg::new("ms").value_parser(clap::value_parser!(u32))),
        |world, m| {
            echo_command(
                world,
                m.get_one::<String>("action").map(String::as_str),
                m.get_one::<u32>("ms").copied(),
            )
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("voice.mute-mic")
            .about("Mute or unmute your microphone (M, gamepad Select); remembered")
            .arg(switch_arg()),
        |world, m| set_mute(world, Toggle::Mic, m.get_one::<String>("state").map(String::as_str)),
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("voice.mute-speaker")
            .about("Mute or unmute all voices (Shift+M, gamepad R3); game sounds stay; remembered")
            .arg(switch_arg()),
        |world, m| {
            set_mute(
                world,
                Toggle::Speaker,
                m.get_one::<String>("state").map(String::as_str),
            )
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("voice.apply")
            .about("Apply the [voice] settings (run by `set voice.*`)")
            .arg(Arg::new("ignored").num_args(0..).hide(true)),
        |world, _| {
            apply_settings(world);
            Ok(String::new())
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("voice.status").about("Show the voice call: status, room, switches, participants"),
        |world, _| {
            let voice = world.resource::<Voice>();
            let c = voice.0.lock().expect("not poisoned");
            let note = (!ocw_voice::BUILT_IN).then_some(ocw_voice::backend::NOT_BUILT_IN);
            Ok(voice_cmd::status_command(&c, note))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("voice.mute")
            .about("Mute or unmute one participant for yourself (rider id, identity or name)")
            .arg(Arg::new("who").required(true))
            .arg(switch_arg()),
        |world, m| {
            let who = m.get_one::<String>("who").expect("required");
            let voice = world.resource::<Voice>();
            let mut c = voice.0.lock().expect("not poisoned");
            voice_cmd::mute_who_command(&mut c, who, m.get_one::<String>("state").map(String::as_str))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("voice.group")
            .about("Talk in a named group (private room) instead of the whole session; without a name: back to the session")
            .arg(Arg::new("name"))
            .arg(Arg::new("leave").long("leave").action(ArgAction::SetTrue)),
        |world, m| {
            let name = m.get_one::<String>("name").cloned().filter(|_| !m.get_flag("leave"));
            let voice = world.resource::<Voice>();
            let mut c = voice.0.lock().expect("not poisoned");
            voice_cmd::group_command(&mut c, name)
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("voice.devices").about("List microphones and speakers (set voice.input / voice.output NAME)"),
        |world, _| {
            let voice = world.resource::<Voice>();
            let mut c = voice.0.lock().expect("not poisoned");
            Ok(voice_cmd::devices_command(&mut c))
        },
    ));
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use ocw_render::console::ConsoleCorePlugin;

    use super::*;
    use crate::Args;

    #[test]
    fn hotkeys_keyboard_and_gamepad() {
        let t = |i: Option<Input>| i.map(toggle);
        assert_eq!(t(key_input(true, false)), Some(Toggle::Mic));
        assert_eq!(t(key_input(true, true)), Some(Toggle::Speaker));
        assert_eq!(key_input(false, true), None);
        assert_eq!(t(pad_input(GamepadButton::Select)), Some(Toggle::Mic));
        assert_eq!(
            t(pad_input(GamepadButton::RightThumb)),
            Some(Toggle::Speaker)
        );
        assert_eq!(
            pad_input(GamepadButton::Start),
            None,
            "Start opens the menu"
        );
        assert_eq!(pad_input(GamepadButton::South), None);
    }

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ConsoleCorePlugin))
            .insert_resource(Args::parse_from(["ocw-desktop"]))
            .insert_resource(UserSettings::default())
            .add_plugins((crate::settings::SettingsPlugin, VoicePlugin));
        app
    }

    /// Runs a console line; the log lines it added, joined.
    fn run(app: &mut App, line: &str) -> String {
        let before = app.world().resource::<ConsoleState>().log().len();
        app.world_mut().resource_mut::<ConsoleState>().queue(line);
        app.update();
        let log = app.world().resource::<ConsoleState>().log();
        log[before.min(log.len())..].join("\n")
    }

    /// The console commands (also run by the hotkeys and the menu) flip
    /// the switches, remember them in the settings and reach the HUD.
    #[test]
    fn mute_commands_are_remembered_and_shown() {
        let mut app = app();
        assert!(run(&mut app, "voice.mute-mic").contains("microphone muted"));
        let us = app.world().resource::<UserSettings>();
        assert_eq!(us.value("voice.mic_muted").as_bool(), Some(true));
        assert!(run(&mut app, "voice.mute-mic toggle").contains("microphone on"));
        assert!(run(&mut app, "voice.mute-speaker on").contains("speaker muted"));
        assert!(
            run(&mut app, "voice.mute-speaker on").contains("speaker muted"),
            "idempotent"
        );
        app.update();
        let hud = app.world().resource::<VoiceHud>();
        assert!(hud.speaker_muted && !hud.mic_muted);
        // Not in a call (no shared ride): the indicator is hidden.
        assert!(!hud.visible);
        // A setting changed with `set` reaches the controller.
        app.world_mut()
            .resource_mut::<ConsoleState>()
            .queue("set voice.mic_muted on");
        app.update();
        app.update();
        let voice = app.world().resource::<Voice>();
        assert!(voice.0.lock().unwrap().mute().mic_muted);
        let status = run(&mut app, "voice.status");
        assert!(status.contains("microphone"), "{status}");
        assert!(run(&mut app, "voice.group Bad").contains("invalid group name"));
        assert!(run(&mut app, "voice.mute nobody").contains("nobody called"));
    }

    #[test]
    fn echo_console_commands() {
        let mut app = app();
        assert!(run(&mut app, "voice.echo on").contains("echo test on (300 ms)"));
        let us = app.world().resource::<UserSettings>();
        assert_eq!(
            us.value("audio.echo_bot").as_bool(),
            Some(true),
            "remembered"
        );
        assert!(run(&mut app, "voice.echo delay 750").contains("750 ms"));
        assert!(
            run(&mut app, "voice.echo delay 5").contains("20 ms"),
            "clamped"
        );
        assert!(run(&mut app, "voice.echo delay").contains("20 to 3000"));
        let status = run(&mut app, "voice.echo");
        assert!(
            status.contains("echo test:") && status.contains("20 ms"),
            "{status}"
        );
        // No server here: "here" needs one.
        assert!(run(&mut app, "voice.echo here").contains("not connected"));
        assert!(run(&mut app, "voice.echo off").contains("echo test off"));
        let voice = app.world().resource::<Voice>();
        assert!(!voice.0.lock().unwrap().echo_on());
        app.update();
        // The HUD shows nothing of the echo while it is off.
        assert_eq!(app.world().resource::<VoiceHud>().echo, None);
        run(&mut app, "voice.echo on");
        app.update();
        let hud = app.world().resource::<VoiceHud>().echo.clone().unwrap();
        assert!(hud.text.starts_with("echo test:"), "{hud:?}");
    }

    /// `--echo-bot` turns the echo test on from the start.
    #[test]
    fn echo_bot_flag_turns_the_echo_test_on() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ConsoleCorePlugin))
            .insert_resource(Args::parse_from(["ocw-desktop", "--echo-bot"]))
            .insert_resource(UserSettings::default())
            .add_plugins((crate::settings::SettingsPlugin, VoicePlugin));
        app.update();
        let voice = app.world().resource::<Voice>();
        assert!(voice.0.lock().unwrap().echo_on());
        assert!(app.world().resource::<VoiceHud>().echo.is_some());
    }

    /// The hotkeys queue the mute commands while riding.
    #[test]
    fn m_key_mutes_the_microphone() {
        let mut app = app();
        app.init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ocw_render::menu::InputContext>();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyM);
        app.update();
        // Without the input plugin nothing clears the press: once only.
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear();
        app.update();
        app.update();
        let voice = app.world().resource::<Voice>();
        assert!(voice.0.lock().unwrap().mute().mic_muted);
    }
}
