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

pub mod page;

use std::sync::Mutex;

use bevy::prelude::*;
use clap::{Arg, ArgAction, Command};
use ocw_core::RiderId;
use ocw_protocol::v1;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, ConsoleState, Permission};
use ocw_render::menu::ride_input;
use ocw_render::voice::VoiceHud;
use ocw_render::{RideView, RiderView};
use ocw_voice::hotkeys::{Input, Toggle, toggle};
use ocw_voice::spatial::Peer;
use ocw_voice::{MuteState, Status, Switch, VoiceController, VoiceSettings};
use ocw_world::Track;

use crate::client::ClientCore;
use crate::connection::{ClientState, Link, SessionStart};
use crate::settings::UserSettings;
use crate::settings::registry::Settings;

/// Grants from the game server, handed over by the connection.
#[derive(Resource, Default)]
pub struct PendingVoiceGrants(pub Vec<v1::VoiceGrant>);

/// The voice controller (its backend is not `Sync`, hence the mutex).
#[derive(Resource)]
pub struct Voice(pub Mutex<VoiceController>);

/// What the menu's voice pages show, refreshed every frame.
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct VoiceMenuInfo {
    pub status: String,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    /// The saved device is not present (the default is used).
    pub input_missing: bool,
    pub output_missing: bool,
    pub participants: Vec<MenuParticipant>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MenuParticipant {
    pub identity: String,
    pub name: String,
    pub muted_by_me: bool,
    pub mic_muted: bool,
    pub heard: Option<f32>,
}

pub struct VoicePlugin;

impl Plugin for VoicePlugin {
    fn build(&self, app: &mut App) {
        let controller = VoiceController::new(
            ocw_voice::default_backend(),
            VoiceSettings::default(),
            MuteState::default(),
        );
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

/// Where the other riders are relative to the local one (spec §7): on a
/// route the gap along it (shortest way round on a loop); in free ride,
/// where everyone rides their own path, the straight-line distance.
pub fn peers(view: &RideView) -> Vec<Peer> {
    let Some(own) = view.local() else {
        return Vec::new();
    };
    let position = |r: &RiderView| -> Option<[f64; 3]> {
        if view.free_ride {
            view.paths
                .get(&r.id)
                .map(|p| p.sample(r.state.distance).position)
        } else {
            None
        }
    };
    let own_pos = position(own);
    view.riders
        .iter()
        .filter(|r| !r.is_local && r.id != ocw_render::riders::GHOST_ID)
        .filter_map(|r| {
            let gap_m = if view.free_ride {
                let (a, b) = (own_pos?, position(r)?);
                (a[0] - b[0]).hypot(a[2] - b[2])
            } else {
                let mut gap = r.state.distance - own.state.distance;
                let len = view.route.length();
                if view.route.is_loop() && len > 0.0 {
                    gap = (gap + len / 2.0).rem_euclid(len) - len / 2.0;
                }
                gap
            };
            Some(Peer {
                rider: r.id,
                gap_m,
                lateral_m: r.state.lateral_offset - own.state.lateral_offset,
            })
        })
        .collect()
}

/// The voice settings from `settings.toml` (with the session layers).
pub fn settings_of(s: &Settings) -> (VoiceSettings, MuteState) {
    let on = |k: &str| s.get(k).ok().and_then(|v| v.as_bool()).unwrap_or(false);
    let int = |k: &str, d: i64| match s.get(k) {
        Ok(crate::settings::registry::Value::Int(i)) => i,
        _ => d,
    };
    let device = |k: &str| {
        s.get(k)
            .ok()
            .map(|v| v.to_string())
            .filter(|v| !v.is_empty() && v != "default")
    };
    (
        VoiceSettings {
            enabled: on("voice.enabled"),
            spatial: on("voice.spatial"),
            volume: int("voice.volume", 100) as f32 / 100.0,
            max_voices: int("voice.max_voices", 10).max(1) as usize,
            input: device("voice.input"),
            output: device("voice.output"),
        },
        MuteState {
            mic_muted: on("voice.mic_muted"),
            speaker_muted: on("voice.speaker_muted"),
        },
    )
}

/// The HUD text of a status (with the number of others in the room).
fn status_text(status: &Status, others: usize) -> String {
    match status {
        Status::Connected => format!("connected ({others})"),
        // The reason is in the console; the HUD stays short.
        Status::Failed(_) => "failed, retrying".into(),
        s => s.text(),
    }
}

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
    mut seen: Local<Option<(u64, bool)>>,
) {
    let mut c = voice.0.lock().expect("not poisoned");
    let now = time.elapsed_secs_f64();
    if let Some(client) = &client {
        // A new session (start, ride switch) or its welcome: the call of
        // the old one ends; whether the new server offers voice.
        let key = (
            session.as_ref().map_or(0, |s| s.0.to_bits()),
            client.0.rider_id().is_some(),
        );
        if *seen != Some(key) {
            *seen = Some(key);
            c.new_session(client.0.voice_offered());
        }
        c.set_human_riders(client.0.human_riders());
    }
    for g in grants.0.drain(..) {
        c.on_grant(&g, now);
    }
    let peers = view.as_deref().map(peers).unwrap_or_default();
    if let Some(req) = c.update(now, &peers)
        && let Some(link) = &link
    {
        link.send_bytes(ClientCore::voice_request_message(req));
    }
    for m in c.take_messages() {
        info!("{m}");
        console.print(m);
    }
    let status = c.status();
    let participants: Vec<MenuParticipant> = c
        .participants()
        .map(|p| MenuParticipant {
            identity: p.identity.clone(),
            name: if p.name.is_empty() {
                p.identity.clone()
            } else {
                p.name.clone()
            },
            muted_by_me: p.muted_by_me,
            mic_muted: p.mic_muted,
            heard: p.heard,
        })
        .collect();
    let text = status_text(&status, participants.len());
    let inputs = c.input_devices();
    let outputs = c.output_devices();
    let s = c.settings().clone();
    let missing = |saved: &Option<String>, present: &[String]| {
        matches!(
            ocw_voice::device::choose(saved.as_deref(), present),
            ocw_voice::device::Choice::Default { missing: Some(_) }
        )
    };
    let info = VoiceMenuInfo {
        status: text.clone(),
        input_missing: missing(&s.input, &inputs),
        output_missing: missing(&s.output, &outputs),
        inputs,
        outputs,
        participants,
    };
    menu.set_if_neq(info);
    let speaking = c.speaking_riders();
    let own = client.as_ref().and_then(|c| c.0.rider_id());
    let names: Vec<String> = c
        .participants()
        .filter(|p| p.speaking && p.rider != own)
        .map(|p| p.name.clone())
        .collect();
    let mute = c.mute();
    hud.set_if_neq(VoiceHud {
        visible: status.in_call(),
        mic_muted: mute.mic_muted,
        speaker_muted: mute.speaker_muted,
        status: text,
        self_speaking: c.self_speaking(),
        speaking: speaking.into_iter().filter(|r| Some(*r) != own).collect(),
        speaking_names: names,
    });
}

// ------------------------------------------------------------- commands

fn switch_arg() -> Arg {
    Arg::new("state")
        .value_parser(["on", "off", "toggle"])
        .help("on = muted, off = not muted; none: toggle")
}

/// Sets a mute switch: saved in `settings.toml` (remembered across rides
/// and restarts) and applied to the call.
fn set_mute(world: &mut World, which: Toggle, arg: Option<&str>) -> Result<String, String> {
    let s = Switch::parse(arg)?;
    let key = match which {
        Toggle::Mic => "voice.mic_muted",
        Toggle::Speaker => "voice.speaker_muted",
    };
    let mut saved = String::new();
    if let Some(mut us) = world.get_resource_mut::<UserSettings>() {
        let now = us.value(key).as_bool().unwrap_or(false);
        let new = s.apply(now);
        us.settings.set(key, if new { "on" } else { "off" })?;
        if let Err(e) = us.save() {
            saved = format!(" (not saved: {e})");
        }
    }
    let voice = world.resource::<Voice>();
    let mut c = voice.0.lock().expect("not poisoned");
    match which {
        Toggle::Mic => {
            c.set_mic(s);
        }
        Toggle::Speaker => {
            c.set_speaker(s);
        }
    }
    let m = c.mute();
    Ok(match which {
        Toggle::Mic if m.mic_muted => format!("microphone muted{saved}"),
        Toggle::Mic => format!("microphone on{saved}"),
        Toggle::Speaker if m.speaker_muted => format!("speaker muted: no voices{saved}"),
        Toggle::Speaker => format!("speaker on{saved}"),
    })
}

/// Applies the `[voice]` settings to the controller (the settings' apply
/// command, at the start and on every change).
fn apply_settings(world: &mut World) {
    let Some(us) = world.get_resource::<UserSettings>() else {
        return;
    };
    let (settings, mute) = settings_of(&us.settings);
    let voice = world.resource::<Voice>();
    let mut c = voice.0.lock().expect("not poisoned");
    c.set_settings(settings);
    let on = |b: bool| if b { Switch::On } else { Switch::Off };
    c.set_mic(on(mute.mic_muted));
    c.set_speaker(on(mute.speaker_muted));
}

fn register_commands(app: &mut App) {
    app.console_command(ConsoleCommand::new(
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
            let m = c.mute();
            let mut out = vec![
                format!("voice: {}", c.status().text()),
                format!(
                    "microphone {}, speaker {}",
                    if m.mic_muted { "muted" } else { "on" },
                    if m.speaker_muted { "muted" } else { "on" }
                ),
            ];
            if let Some(r) = c.room() {
                out.push(format!("room: {r}"));
            }
            if let Some(g) = c.group() {
                out.push(format!("group: {g}"));
            }
            for p in c.participants() {
                out.push(format!(
                    "  {} ({}){}{}{}",
                    p.name,
                    p.identity,
                    if p.speaking { ", talking" } else { "" },
                    if p.muted_by_me { ", muted by you" } else { "" },
                    p.heard.map(|g| format!(", heard {:.0} %", g * 100.0)).unwrap_or_default()
                ));
            }
            if !ocw_voice::BUILT_IN {
                out.push(format!("({})", ocw_voice::backend::NOT_BUILT_IN));
            }
            Ok(out.join("\n"))
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
            let s = Switch::parse(m.get_one::<String>("state").map(String::as_str))?;
            let voice = world.resource::<Voice>();
            let mut c = voice.0.lock().expect("not poisoned");
            let rider = who.parse::<u32>().ok().map(RiderId);
            let identity = c
                .participants()
                .find(|p| p.identity == *who || p.name == *who || (rider.is_some() && p.rider == rider))
                .map(|p| (p.identity.clone(), p.name.clone()))
                .ok_or_else(|| format!("nobody called {who:?} in the voice room"))?;
            let muted = c.mute_participant(&identity.0, s);
            Ok(format!("{} {}", identity.1, if muted { "muted" } else { "unmuted" }))
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
            c.set_group(name.clone())?;
            Ok(match name {
                Some(n) => format!("voice group {n}: joining"),
                None => "voice: the whole session".into(),
            })
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("voice.devices").about("List microphones and speakers (set voice.input / voice.output NAME)"),
        |world, _| {
            let voice = world.resource::<Voice>();
            let mut c = voice.0.lock().expect("not poisoned");
            let s = c.settings().clone();
            let list = |title: &str, present: Vec<String>, saved: &Option<String>| {
                let mut out = vec![format!(
                    "{title}: {}",
                    saved.as_deref().unwrap_or("default")
                )];
                out.extend(present.into_iter().map(|d| format!("  {d}")));
                out
            };
            let mut out = list("microphone", c.input_devices(), &s.input);
            out.extend(list("speaker", c.output_devices(), &s.output));
            Ok(out.join("\n"))
        },
    ));
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use ocw_render::console::ConsoleCorePlugin;
    use ocw_world::Route;

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

    fn rider(id: u32, d: f64, lat: f64, local: bool) -> RiderView {
        RiderView {
            id: RiderId(id),
            name: format!("R{id}"),
            state: ocw_sim::RiderState {
                distance: d,
                lateral_offset: lat,
                ..Default::default()
            },
            power_w: 0.0,
            cadence_rpm: None,
            heart_rate_bpm: None,
            is_local: local,
            bike: Default::default(),
            look: None,
        }
    }

    #[test]
    fn peers_by_gap_along_the_route() {
        let route = Route::test_loop();
        let len = route.length();
        let mut view = RideView::new(route);
        view.riders = vec![
            rider(1, 100.0, 0.5, true),
            rider(2, 130.0, -1.0, false),
            rider(3, len - 50.0, 0.5, false),
            rider(ocw_render::riders::GHOST_ID.0, 100.0, 0.0, false),
        ];
        let p = peers(&view);
        assert_eq!(p.len(), 2, "no ghost, not the own rider");
        assert_eq!(p[0].rider, RiderId(2));
        assert!((p[0].gap_m - 30.0).abs() < 1e-9);
        assert!((p[0].lateral_m + 1.5).abs() < 1e-9);
        // On a loop the shortest way round: 150 m behind, not len-150 ahead.
        assert!(view.route.is_loop());
        assert!((p[1].gap_m + 150.0).abs() < 1e-6, "{}", p[1].gap_m);
        view.riders.retain(|r| !r.is_local);
        assert!(peers(&view).is_empty(), "no local rider yet");
    }

    #[test]
    fn settings_map_onto_the_controller() {
        let (s, _) = Settings::parse(
            "[voice]\nvolume = 80\nmax_voices = 4\ninput = \"Headset\"\nspeaker_muted = true\nspatial = false\n",
        )
        .unwrap();
        let (v, m) = settings_of(&s);
        assert_eq!(v.volume, 0.8);
        assert_eq!(v.max_voices, 4);
        assert_eq!(v.input.as_deref(), Some("Headset"));
        assert_eq!(v.output, None, "default");
        assert!(!v.spatial && v.enabled);
        assert_eq!(
            m,
            MuteState {
                mic_muted: false,
                speaker_muted: true
            }
        );
        // First start: both on.
        let (_, m) = settings_of(&Settings::default());
        assert_eq!(m, MuteState::default());
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
