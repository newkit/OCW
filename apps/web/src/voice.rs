//! Voice chat in the browser (spec §2.4, §7, §14 Phase 10 B5): the same
//! [`VoiceController`] and per-frame logic as the desktop
//! (`ocw_client::voice::VoiceDriver`), on the browser's backend
//! (`ocw_voice::web::WebVoiceBackend`: LiveKit's JS SDK behind the page's
//! `window.ocwVoice` shim, `crate::browser_voice`).
//!
//! Always on, like a call: on a shared ride (a remote server with voice and
//! another human rider) the app asks the game server for a LiveKit token
//! over the game connection (WebTransport) and joins the room; the local
//! server in the page has no voice (a solo ride). `M` mutes the microphone,
//! `Shift+M` the speaker (all voices); the switches are kept in the
//! settings (`voice.mic_muted`, `voice.speaker_muted`). The HUD's voice
//! indicator is the shared one (`ocw_render::voice`). The echo test
//! (`voice.echo …`) works on solo rides too (the browser's microphone
//! through a delay in the shim). Console as on the desktop: `voice.status`,
//! `voice.mute-mic`, `voice.mute-speaker`, `voice.mute WHO`, `voice.group`,
//! `voice.devices`, `voice.echo`, `voice.apply`.

use std::sync::Mutex;

use bevy::prelude::*;
use clap::{Arg, ArgAction, Command};
use ocw_client::client::Notice;
use ocw_client::voice::{self as shared, SettingsStore, VoiceDriver, VoiceMenuInfo};
use ocw_protocol::v1;
use ocw_render::RideView;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, ConsoleState, Permission};
use ocw_render::menu::ride_input;
use ocw_render::voice::VoiceHud;
use ocw_voice::hotkeys::{Input, Toggle};
use ocw_voice::{MuteState, VoiceBackend, VoiceController, VoiceSettings};

use crate::plugin::Session;
use crate::settings::WebSettings;

/// Grants from the game server (`Notice::Voice`), until the next frame.
#[derive(Resource, Default)]
pub struct PendingVoiceGrants(pub Vec<v1::VoiceGrant>);

impl PendingVoiceGrants {
    /// Takes the voice grants out of the session's notices.
    pub fn collect(&mut self, notices: &[Notice]) {
        for n in notices {
            if let Notice::Voice(g) = n {
                self.0.push(g.clone());
            }
        }
    }
}

/// The voice controller (its backend is not `Sync`, hence the mutex).
#[derive(Resource)]
pub struct Voice(pub Mutex<VoiceController>);

/// What the menu's Voice pages show (shared with the desktop:
/// `ocw_ui::menu::VoiceMenu`; `window.ocw.voice()` reads it too).
pub use ocw_ui::menu::VoiceMenu as VoiceInfo;

/// Makes the backend when the app is built (the page's shim on wasm32, a
/// mock in the tests; without it voice is unavailable).
#[derive(Resource)]
pub struct MakeVoiceBackend(pub Box<dyn Fn() -> Box<dyn VoiceBackend> + Send + Sync>);

/// Why there is no voice backend (natively, outside the tests).
pub const NO_SHIM: &str = "voice needs the page's livekit-client (the browser app)";

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
struct Unavailable;

impl VoiceBackend for Unavailable {
    fn available(&self) -> Result<(), String> {
        Err(NO_SHIM.into())
    }
    fn input_devices(&mut self) -> Vec<String> {
        Vec::new()
    }
    fn output_devices(&mut self) -> Vec<String> {
        Vec::new()
    }
    fn send(&mut self, _: ocw_voice::BackendCommand) {}
    fn poll(&mut self) -> Vec<ocw_voice::BackendEvent> {
        Vec::new()
    }
}

pub struct WebVoicePlugin;

impl Plugin for WebVoicePlugin {
    fn build(&self, app: &mut App) {
        let backend = match app.world().get_resource::<MakeVoiceBackend>() {
            Some(m) => (m.0)(),
            None => default_backend(),
        };
        let mut controller =
            VoiceController::new(backend, VoiceSettings::default(), MuteState::default());
        // The saved settings, before the first frame (the startup commands
        // apply them again, as on the desktop).
        if let Some(ws) = app.world().get_resource::<WebSettings>() {
            shared::apply_settings(&mut controller, &ws.settings, false);
        }
        app.insert_resource(Voice(Mutex::new(controller)))
            .init_resource::<PendingVoiceGrants>()
            .init_resource::<VoiceInfo>()
            .init_resource::<VoiceHud>()
            .add_systems(
                Update,
                (hotkeys.run_if(ride_input), drive, publish)
                    .chain()
                    .after(crate::plugin::ViewUpdated),
            );
        register_commands(app);
    }
}

#[cfg(target_arch = "wasm32")]
fn default_backend() -> Box<dyn VoiceBackend> {
    crate::browser_voice::backend()
}

#[cfg(not(target_arch = "wasm32"))]
fn default_backend() -> Box<dyn VoiceBackend> {
    Box::new(Unavailable)
}

/// `M` / `Shift+M` and the gamepad's Select / R3 (the browser's Gamepad
/// API through Bevy's gilrs, B6) → the mute commands, as on the desktop.
fn hotkeys(
    keys: Option<Res<ButtonInput<KeyCode>>>,
    pads: Query<&Gamepad>,
    mut console: ResMut<ConsoleState>,
) {
    let mut inputs = Vec::new();
    if let Some(k) = keys
        && k.just_pressed(KeyCode::KeyM)
    {
        let shift = k.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
        inputs.push(Input::KeyM { shift });
    }
    for pad in &pads {
        if pad.just_pressed(GamepadButton::Select) {
            inputs.push(Input::PadSelect);
        }
        if pad.just_pressed(GamepadButton::RightThumb) {
            inputs.push(Input::PadRightStick);
        }
    }
    for c in shared::hotkey_commands(inputs) {
        console.queue(c);
    }
}

/// The shared per-frame logic: grants in, requests out over the session
/// (the remote server's WebTransport; the local server has no voice).
#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
fn drive(
    time: Res<Time>,
    voice: Res<Voice>,
    mut session: Option<ResMut<Session>>,
    view: Option<Res<RideView>>,
    mut grants: ResMut<PendingVoiceGrants>,
    mut info: ResMut<VoiceInfo>,
    mut hud: ResMut<VoiceHud>,
    mut console: ResMut<ConsoleState>,
    mut driver: Local<VoiceDriver>,
) {
    let mut c = voice.0.lock().expect("not poisoned");
    let frame = driver.frame(
        &mut c,
        session.as_deref().map(|s| &s.0.client),
        // One session per page load.
        1,
        std::mem::take(&mut grants.0),
        view.as_deref(),
        time.elapsed_secs_f64(),
    );
    if let Some(s) = session.as_deref_mut() {
        for bytes in &frame.send {
            s.0.send(bytes);
        }
    }
    for m in frame.console {
        info!("{m}");
        console.print(m);
    }
    info.set_if_neq(VoiceInfo(frame.menu));
    hud.set_if_neq(frame.hud);
}

/// The voice state as JSON for the page (`window.ocw.voice()`).
pub fn voice_json(c: &VoiceController, info: &VoiceMenuInfo, hud: &VoiceHud) -> String {
    let m = c.mute();
    let s = c.settings();
    serde_json::json!({
        "status": c.status().text(),
        "hud": info.status,
        "available": !matches!(c.status(), ocw_voice::Status::Unavailable(_)),
        "in_call": hud.visible,
        "room": c.room(),
        "group": c.group(),
        "mic_muted": m.mic_muted,
        "speaker_muted": m.speaker_muted,
        "enabled": s.enabled,
        "spatial": s.spatial,
        "volume": (s.volume * 100.0).round(),
        "input": s.input,
        "output": s.output,
        "inputs": info.inputs,
        "outputs": info.outputs,
        "participants": info.participants.iter().map(|p| serde_json::json!({
            "identity": p.identity, "name": p.name, "mic_muted": p.mic_muted,
            "muted_by_me": p.muted_by_me, "heard": p.heard,
        })).collect::<Vec<_>>(),
        "speaking": hud.speaking_names,
        "self_speaking": hud.self_speaking,
        "echo": {
            "on": c.echo_on(),
            "delay_ms": c.echo_delay_ms(),
            "status": c.echo_status().text(),
            "short": info.echo,
            "near": hud.echo.as_ref().is_some_and(|e| e.near),
            "input": c.levels().input,
            "level": c.levels().echo,
        },
    })
    .to_string()
}

static JSON: Mutex<String> = Mutex::new(String::new());

/// The last published voice state (`window.ocw.voice()`).
pub fn page_json() -> String {
    JSON.lock().expect("not poisoned").clone()
}

fn publish(
    time: Res<Time>,
    voice: Res<Voice>,
    info: Res<VoiceInfo>,
    hud: Res<VoiceHud>,
    view: Option<Res<RideView>>,
    mut next: Local<f64>,
) {
    let now = time.elapsed_secs_f64();
    if now < *next {
        return;
    }
    *next = now + 0.25;
    let c = voice.0.lock().expect("not poisoned");
    let mut v: serde_json::Value =
        serde_json::from_str(&voice_json(&c, &info.0, &hud)).unwrap_or_default();
    // The riders on the road and their voice identities (the panel and
    // the browser test match participants to riders by them).
    v["riders"] = view
        .map(|view| {
            view.riders
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "id": r.id.0, "name": r.name, "local": r.is_local,
                        "identity": ocw_protocol::voice_identity(r.id),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
        .into();
    *JSON.lock().expect("not poisoned") = v.to_string();
}

/// Runs `f` with the controller and the page's settings.
fn with_voice<T>(
    world: &mut World,
    f: impl FnOnce(&mut VoiceController, Option<&mut dyn SettingsStore>) -> T,
) -> T {
    if world.contains_resource::<WebSettings>() {
        world.resource_scope(|world, mut ws: Mut<WebSettings>| {
            let voice = world.resource::<Voice>();
            let mut c = voice.0.lock().expect("not poisoned");
            f(&mut c, Some(&mut *ws))
        })
    } else {
        let voice = world.resource::<Voice>();
        let mut c = voice.0.lock().expect("not poisoned");
        f(&mut c, None)
    }
}

fn switch_arg() -> Arg {
    Arg::new("state")
        .value_parser(["on", "off", "toggle"])
        .help("on = muted, off = not muted; none: toggle")
}

fn register_commands(app: &mut App) {
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("voice.echo")
            .about("Echo test: an Echo rider plays your microphone back to you, delayed, when you are near it (on, off, here = 20 m ahead of you, delay MS, status)")
            .arg(Arg::new("action").value_parser(["on", "off", "here", "delay", "status"]))
            .arg(Arg::new("ms").value_parser(clap::value_parser!(u32))),
        |world, m| {
            let action = m.get_one::<String>("action").map(String::as_str);
            let ms = m.get_one::<u32>("ms").copied();
            let connected = world.contains_resource::<Session>();
            let (text, send) =
                with_voice(world, |c, store| shared::echo_command(c, store, action, ms, connected))?;
            if let (Some(bytes), Some(mut s)) = (send, world.get_resource_mut::<Session>()) {
                s.0.send(&bytes);
            }
            Ok(text)
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("voice.mute-mic")
            .about("Mute or unmute your microphone (M); remembered")
            .arg(switch_arg()),
        |world, m| {
            let arg = m.get_one::<String>("state").map(String::as_str);
            with_voice(world, |c, store| shared::mute_command(c, store, Toggle::Mic, arg))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("voice.mute-speaker")
            .about("Mute or unmute all voices (Shift+M); game sounds stay; remembered")
            .arg(switch_arg()),
        |world, m| {
            let arg = m.get_one::<String>("state").map(String::as_str);
            with_voice(world, |c, store| shared::mute_command(c, store, Toggle::Speaker, arg))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("voice.apply")
            .about("Apply the [voice] settings (run by `set voice.*`)")
            .arg(Arg::new("ignored").num_args(0..).hide(true)),
        |world, _| {
            let Some(ws) = world.get_resource::<WebSettings>() else {
                return Ok(String::new());
            };
            let settings = ws.settings.clone();
            let voice = world.resource::<Voice>();
            let mut c = voice.0.lock().expect("not poisoned");
            shared::apply_settings(&mut c, &settings, false);
            Ok(String::new())
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("voice.status")
            .about("Show the voice call: status, room, switches, participants"),
        |world, _| {
            let voice = world.resource::<Voice>();
            let c = voice.0.lock().expect("not poisoned");
            Ok(shared::status_command(&c, None))
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
            shared::mute_who_command(&mut c, who, m.get_one::<String>("state").map(String::as_str))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("voice.group")
            .about("Talk in a named group (private room) instead of the whole session; without a name: back to the session")
            .arg(Arg::new("name"))
            .arg(Arg::new("leave").long("leave").action(ArgAction::SetTrue)),
        |world, m| {
            let name = m
                .get_one::<String>("name")
                .cloned()
                .filter(|_| !m.get_flag("leave"));
            let voice = world.resource::<Voice>();
            let mut c = voice.0.lock().expect("not poisoned");
            shared::group_command(&mut c, name)
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("voice.devices")
            .about("List microphones and speakers (set voice.input / voice.output NAME)"),
        |world, _| {
            let voice = world.resource::<Voice>();
            let mut c = voice.0.lock().expect("not poisoned");
            Ok(shared::devices_command(&mut c))
        },
    ));
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::settings::memory::MemoryStore;
    use ocw_render::console::ConsoleCorePlugin;
    use ocw_voice::backend::{BackendCommand, BackendEvent};

    /// The tests' backend: records commands, hands out queued events.
    #[derive(Clone, Default)]
    struct Shared(Arc<Mutex<(Vec<BackendCommand>, Vec<BackendEvent>)>>);

    impl VoiceBackend for Shared {
        fn available(&self) -> Result<(), String> {
            Ok(())
        }
        fn input_devices(&mut self) -> Vec<String> {
            vec!["Built-in microphone".into()]
        }
        fn output_devices(&mut self) -> Vec<String> {
            Vec::new()
        }
        fn send(&mut self, cmd: BackendCommand) {
            self.0.lock().unwrap().0.push(cmd);
        }
        fn poll(&mut self) -> Vec<BackendEvent> {
            std::mem::take(&mut self.0.lock().unwrap().1)
        }
    }

    fn app(backend: Shared, mem: MemoryStore) -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ConsoleCorePlugin));
        app.insert_resource(crate::settings::MakeStore(Box::new(move || {
            Some(Box::new(mem.clone()) as Box<dyn crate::settings::TextStore>)
        })));
        app.insert_resource(MakeVoiceBackend(Box::new(move || {
            Box::new(backend.clone()) as Box<dyn VoiceBackend>
        })));
        app.add_plugins((crate::settings::WebSettingsPlugin, WebVoicePlugin));
        app.update();
        app
    }

    fn run(app: &mut App, line: &str) -> String {
        let before = app.world().resource::<ConsoleState>().log().len();
        app.world_mut().resource_mut::<ConsoleState>().queue(line);
        app.update();
        let log = app.world().resource::<ConsoleState>().log();
        log[before.min(log.len())..].join("\n")
    }

    /// The switches: the console (and M / Shift+M) flip them, they reach
    /// the backend and the HUD and are saved in the page's storage; a
    /// reload starts with them.
    #[test]
    fn mute_switches_are_saved_and_shown() {
        let backend = Shared::default();
        let mem = MemoryStore::default();
        let mut a = app(backend.clone(), mem.clone());
        assert!(run(&mut a, "voice.mute-mic").contains("microphone muted"));
        assert!(
            backend
                .0
                .lock()
                .unwrap()
                .0
                .contains(&BackendCommand::SetMicMuted(true))
        );
        a.update();
        let hud = a.world().resource::<VoiceHud>().clone();
        assert!(hud.mic_muted && !hud.speaker_muted && !hud.visible);
        assert!(
            mem.0
                .lock()
                .unwrap()
                .as_deref()
                .unwrap()
                .contains("mic_muted = true")
        );
        // M with Shift: the speaker.
        a.init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ocw_render::menu::InputContext>();
        {
            let mut k = a.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            k.press(KeyCode::ShiftLeft);
            k.press(KeyCode::KeyM);
        }
        a.update();
        a.world_mut().resource_mut::<ButtonInput<KeyCode>>().clear();
        a.update();
        a.update();
        assert!(a.world().resource::<VoiceHud>().speaker_muted);
        // A reload: the saved switches from the start.
        let a2 = app(Shared::default(), mem.clone());
        let voice = a2.world().resource::<Voice>();
        let m = voice.0.lock().unwrap().mute();
        assert!(m.mic_muted && m.speaker_muted, "{m:?}");
    }

    /// A grant from the server connects; the call shows on the HUD and in
    /// the page's JSON.
    #[test]
    fn a_grant_joins_the_call() {
        let backend = Shared::default();
        let mut a = app(backend.clone(), MemoryStore::default());
        {
            let voice = a.world().resource::<Voice>();
            let mut c = voice.0.lock().unwrap();
            c.new_session(true);
            c.set_human_riders(2);
            assert!(c.update(0.0, &[]).is_some());
        }
        a.world_mut()
            .resource_mut::<PendingVoiceGrants>()
            .collect(&[Notice::Voice(v1::VoiceGrant {
                url: "wss://v".into(),
                token: "t".into(),
                room: "ocw-1".into(),
                identity: "rider-1".into(),
                ..Default::default()
            })]);
        a.update();
        backend.0.lock().unwrap().1.extend([
            BackendEvent::Connected {
                room: "ocw-1".into(),
            },
            BackendEvent::ParticipantJoined {
                identity: "rider-2".into(),
                name: "Ada".into(),
            },
        ]);
        a.update();
        let hud = a.world().resource::<VoiceHud>().clone();
        assert!(hud.visible, "{hud:?}");
        assert_eq!(hud.status, "connected (1)");
        let info = a.world().resource::<VoiceInfo>().0.clone();
        let voice = a.world().resource::<Voice>();
        let c = voice.0.lock().unwrap();
        let j: serde_json::Value = serde_json::from_str(&voice_json(&c, &info, &hud)).unwrap();
        assert_eq!(j["in_call"], true);
        assert_eq!(j["room"], "ocw-1");
        assert_eq!(j["participants"][0]["name"], "Ada");
        assert_eq!(j["inputs"][0], "Built-in microphone");
        assert_eq!(j["available"], true);
    }

    #[test]
    fn echo_and_settings_commands() {
        let mem = MemoryStore::default();
        let mut a = app(Shared::default(), mem.clone());
        assert!(run(&mut a, "voice.echo on").contains("echo test on (300 ms)"));
        assert!(run(&mut a, "voice.echo delay 500").contains("500 ms"));
        assert!(run(&mut a, "voice.echo").contains("delay 500 ms"));
        assert!(run(&mut a, "voice.echo here").contains("not connected"));
        let text = mem.0.lock().unwrap().clone().unwrap();
        assert!(text.contains("echo_bot = true") && text.contains("echo_delay_ms = 500"));
        // `set` applies through the same apply command.
        assert!(run(&mut a, "set voice.volume 60").contains("voice.volume = 60"));
        a.update();
        let voice = a.world().resource::<Voice>();
        assert_eq!(voice.0.lock().unwrap().settings().volume, 0.6);
        // Since B6 every setting the browser has applies.
        assert!(run(&mut a, "set display.units imperial").contains("display.units = imperial"));
        assert!(run(&mut a, "get").contains("voice.volume = 60"));
        assert!(run(&mut a, "voice.devices").contains("Built-in microphone"));
        assert!(run(&mut a, "voice.status").contains("voice: no voice on this server"));
        assert!(run(&mut a, "voice.group Bad").contains("invalid group name"));
    }

    #[test]
    fn without_the_shim_voice_is_unavailable() {
        let c = VoiceController::new(
            Box::new(Unavailable),
            VoiceSettings::default(),
            MuteState::default(),
        );
        assert!(c.status().text().contains("livekit-client"));
    }
}
