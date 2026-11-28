//! Key and button rebinding in both apps (spec §3.2.1 M9): the bindings
//! of the settings (`keys.*`, `pad.*`, `ocw_client::bindings`) as Bevy
//! keys and buttons ([`ActiveBindings`], read by the apps' ride input),
//! the capture of a new key or button for the menu's binding rows
//! ([`BindCapture`], `bind --capture ID`) and the console command
//! `bind`.

use bevy::prelude::*;
use clap::{Arg, ArgAction, Command};
use ocw_client::bindings::{
    ACTIONS, Action, Bindings, Device, KEY_CHOICES, key_label, pad_label, pad_name, parse_keys,
};
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, Permission};

use crate::Platform;
use crate::settings::UserSettings;

/// Key names (as in `KEY_CHOICES`, without `Shift+`) and Bevy's keys.
const KEYS: &[(&str, KeyCode)] = &[
    ("A", KeyCode::KeyA),
    ("B", KeyCode::KeyB),
    ("C", KeyCode::KeyC),
    ("D", KeyCode::KeyD),
    ("E", KeyCode::KeyE),
    ("F", KeyCode::KeyF),
    ("G", KeyCode::KeyG),
    ("H", KeyCode::KeyH),
    ("I", KeyCode::KeyI),
    ("J", KeyCode::KeyJ),
    ("K", KeyCode::KeyK),
    ("L", KeyCode::KeyL),
    ("M", KeyCode::KeyM),
    ("N", KeyCode::KeyN),
    ("O", KeyCode::KeyO),
    ("P", KeyCode::KeyP),
    ("Q", KeyCode::KeyQ),
    ("R", KeyCode::KeyR),
    ("S", KeyCode::KeyS),
    ("T", KeyCode::KeyT),
    ("U", KeyCode::KeyU),
    ("V", KeyCode::KeyV),
    ("W", KeyCode::KeyW),
    ("X", KeyCode::KeyX),
    ("Y", KeyCode::KeyY),
    ("Z", KeyCode::KeyZ),
    ("0", KeyCode::Digit0),
    ("1", KeyCode::Digit1),
    ("2", KeyCode::Digit2),
    ("3", KeyCode::Digit3),
    ("4", KeyCode::Digit4),
    ("5", KeyCode::Digit5),
    ("6", KeyCode::Digit6),
    ("7", KeyCode::Digit7),
    ("8", KeyCode::Digit8),
    ("9", KeyCode::Digit9),
    ("F1", KeyCode::F1),
    ("F2", KeyCode::F2),
    ("F3", KeyCode::F3),
    ("F4", KeyCode::F4),
    ("F5", KeyCode::F5),
    ("F6", KeyCode::F6),
    ("F7", KeyCode::F7),
    ("F8", KeyCode::F8),
    ("F9", KeyCode::F9),
    ("F10", KeyCode::F10),
    ("F11", KeyCode::F11),
    ("Up", KeyCode::ArrowUp),
    ("Down", KeyCode::ArrowDown),
    ("Left", KeyCode::ArrowLeft),
    ("Right", KeyCode::ArrowRight),
    ("PageUp", KeyCode::PageUp),
    ("PageDown", KeyCode::PageDown),
    ("Home", KeyCode::Home),
    ("End", KeyCode::End),
    ("Insert", KeyCode::Insert),
    ("Delete", KeyCode::Delete),
    ("Space", KeyCode::Space),
    ("Enter", KeyCode::Enter),
    ("Tab", KeyCode::Tab),
    ("Backspace", KeyCode::Backspace),
    ("Num0", KeyCode::Numpad0),
    ("Num1", KeyCode::Numpad1),
    ("Num2", KeyCode::Numpad2),
    ("Num3", KeyCode::Numpad3),
    ("Num4", KeyCode::Numpad4),
    ("Num5", KeyCode::Numpad5),
    ("Num6", KeyCode::Numpad6),
    ("Num7", KeyCode::Numpad7),
    ("Num8", KeyCode::Numpad8),
    ("Num9", KeyCode::Numpad9),
    ("Num+", KeyCode::NumpadAdd),
    ("Num-", KeyCode::NumpadSubtract),
    ("Num*", KeyCode::NumpadMultiply),
    ("Num/", KeyCode::NumpadDivide),
    ("Num.", KeyCode::NumpadDecimal),
    ("NumEnter", KeyCode::NumpadEnter),
    ("Minus", KeyCode::Minus),
    ("Equal", KeyCode::Equal),
    ("BracketLeft", KeyCode::BracketLeft),
    ("BracketRight", KeyCode::BracketRight),
    ("Semicolon", KeyCode::Semicolon),
    ("Quote", KeyCode::Quote),
    ("Comma", KeyCode::Comma),
    ("Period", KeyCode::Period),
    ("Slash", KeyCode::Slash),
    ("Backslash", KeyCode::Backslash),
];

const PADS: &[(&str, GamepadButton)] = &[
    ("South", GamepadButton::South),
    ("East", GamepadButton::East),
    ("North", GamepadButton::North),
    ("West", GamepadButton::West),
    ("LB", GamepadButton::LeftTrigger),
    ("RB", GamepadButton::RightTrigger),
    ("Select", GamepadButton::Select),
    ("L3", GamepadButton::LeftThumb),
    ("R3", GamepadButton::RightThumb),
    ("DPadUp", GamepadButton::DPadUp),
    ("DPadDown", GamepadButton::DPadDown),
    ("DPadLeft", GamepadButton::DPadLeft),
    ("DPadRight", GamepadButton::DPadRight),
];

/// Bevy's key of a key name (`Shift+` dropped); `None`: "none".
pub fn key_code(name: &str) -> Option<KeyCode> {
    let base = name.strip_prefix("Shift+").unwrap_or(name);
    KEYS.iter().find(|(n, _)| *n == base).map(|(_, k)| *k)
}

/// The name of a Bevy key (`None`: not bindable).
pub fn key_name_of(code: KeyCode) -> Option<&'static str> {
    KEYS.iter().find(|(_, k)| *k == code).map(|(n, _)| *n)
}

pub fn pad_button(name: &str) -> Option<GamepadButton> {
    PADS.iter().find(|(n, _)| *n == name).map(|(_, b)| *b)
}

pub fn pad_name_of(b: GamepadButton) -> Option<&'static str> {
    PADS.iter().find(|(_, x)| *x == b).map(|(n, _)| *n)
}

fn shift(keys: &ButtonInput<KeyCode>) -> bool {
    keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight])
}

/// The bindings in effect (from the settings, refreshed when they change).
#[derive(Resource, Debug, Clone, Default, PartialEq, Deref)]
pub struct ActiveBindings(pub Bindings);

impl ActiveBindings {
    fn key_fires(&self, a: Action, keys: &ButtonInput<KeyCode>, just: bool) -> bool {
        self.0.key_fires(
            a,
            |n| {
                key_code(n).is_some_and(|k| {
                    if just {
                        keys.just_pressed(k)
                    } else {
                        keys.pressed(k)
                    }
                })
            },
            shift(keys),
        )
    }

    /// The action's key went down this frame.
    pub fn key_just_pressed(&self, a: Action, keys: &ButtonInput<KeyCode>) -> bool {
        self.key_fires(a, keys, true)
    }

    /// The action's key is held.
    pub fn key_pressed(&self, a: Action, keys: &ButtonInput<KeyCode>) -> bool {
        self.key_fires(a, keys, false)
    }

    /// The action's button went down on any pad this frame.
    pub fn pad_just_pressed<'a>(
        &self,
        a: Action,
        pads: impl IntoIterator<Item = &'a Gamepad>,
    ) -> bool {
        pad_button(self.0.pad(a)).is_some_and(|b| pads.into_iter().any(|p| p.just_pressed(b)))
    }

    /// The action's button is held on any pad.
    pub fn pad_pressed<'a>(&self, a: Action, pads: impl IntoIterator<Item = &'a Gamepad>) -> bool {
        pad_button(self.0.pad(a)).is_some_and(|b| pads.into_iter().any(|p| p.pressed(b)))
    }

    /// Key or button went down this frame.
    pub fn just_pressed<'a>(
        &self,
        a: Action,
        keys: &ButtonInput<KeyCode>,
        pads: impl IntoIterator<Item = &'a Gamepad>,
    ) -> bool {
        self.key_just_pressed(a, keys) || self.pad_just_pressed(a, pads)
    }

    /// Key or button held.
    pub fn pressed<'a>(
        &self,
        a: Action,
        keys: &ButtonInput<KeyCode>,
        pads: impl IntoIterator<Item = &'a Gamepad>,
    ) -> bool {
        self.key_pressed(a, keys) || self.pad_pressed(a, pads)
    }
}

/// Whether `a`'s key went down this frame, with `b` or (none inserted)
/// the default bindings.
pub fn key_just_pressed(
    b: Option<&ActiveBindings>,
    a: Action,
    keys: &ButtonInput<KeyCode>,
) -> bool {
    match b {
        Some(b) => b.key_just_pressed(a, keys),
        None => ActiveBindings::default().key_just_pressed(a, keys),
    }
}

/// Whether `a`'s pad button went down this frame on any pad.
pub fn pad_just_pressed<'a>(
    b: Option<&ActiveBindings>,
    a: Action,
    pads: impl IntoIterator<Item = &'a Gamepad>,
) -> bool {
    match b {
        Some(b) => b.pad_just_pressed(a, pads),
        None => ActiveBindings::default().pad_just_pressed(a, pads),
    }
}

/// The voice hotkeys of this frame (spec §2.4): the mute toggles of the
/// bound keys and buttons (defaults M / Shift+M, Select / R3).
pub fn voice_toggles<'a>(
    b: Option<&ActiveBindings>,
    keys: Option<&ButtonInput<KeyCode>>,
    pads: impl IntoIterator<Item = &'a Gamepad>,
) -> Vec<ocw_voice::hotkeys::Toggle> {
    use ocw_voice::hotkeys::Toggle;
    let pads: Vec<&Gamepad> = pads.into_iter().collect();
    let mut out = Vec::new();
    for (a, t) in [
        (Action::MuteMic, Toggle::Mic),
        (Action::MuteVoices, Toggle::Speaker),
    ] {
        if keys.is_some_and(|k| key_just_pressed(b, a, k))
            || pad_just_pressed(b, a, pads.iter().copied())
        {
            out.push(t);
        }
    }
    out
}

/// The action waiting for its new key or button (the Input mapping
/// page's rows) and what the last capture or reset did (the page's
/// status line until the next one).
#[derive(Resource, Debug, Clone, Default, PartialEq, Eq)]
pub struct BindCapture(pub Option<Action>, pub Option<String>);

pub struct BindingsPlugin;

impl Plugin for BindingsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ActiveBindings>()
            .init_resource::<BindCapture>()
            .init_resource::<ocw_render::camera::CameraKey>()
            .init_resource::<ocw_render::hud::RideKeys>()
            .add_systems(PreUpdate, refresh)
            .add_systems(Update, map_keys);
        register_commands(app);
    }
}

/// What an app does with a shared binding's action (an app that adds
/// the shared menu and its bindings without every ride feature, such as
/// OCW Studio, whose console knows no `set display.map`).
#[derive(Debug, Clone, Copy)]
pub enum ActionUse {
    /// The shared behaviour (the game's console line).
    Shared,
    /// Nothing happens (the app has no such feature).
    Drop,
    /// The app's own function instead, run with the world.
    Run(fn(&mut World)),
}

/// The app's hook for the shared bindings' actions; without it (the game,
/// the browser) every action does what it does in a ride.
#[derive(Resource, Debug, Clone, Copy)]
pub struct BindingHooks {
    pub action: fn(Action) -> ActionUse,
}

impl Default for BindingHooks {
    fn default() -> Self {
        Self {
            action: |_| ActionUse::Shared,
        }
    }
}

/// The navigation map's keys (spec §3.2 *Navigation map*): N shows or
/// hides it, Z steps the zoom (auto → 1 … 7 → auto); both as settings
/// (`display.map`, `display.map_zoom`), so they are kept. An app's
/// [`BindingHooks`] may drop them or map them to its own.
#[allow(clippy::too_many_arguments)]
pub fn map_keys(
    keys: Option<Res<ButtonInput<KeyCode>>>,
    hooks: Option<Res<BindingHooks>>,
    mut commands: Commands,
    pads: Query<&Gamepad>,
    bindings: Option<Res<ActiveBindings>>,
    context: Option<Res<ocw_render::menu::InputContext>>,
    show: Option<Res<ocw_render::navi_map::ShowMap>>,
    zoom: Option<Res<ocw_render::navi_map::MapZoom>>,
    console: Option<ResMut<ocw_render::console::ConsoleState>>,
) {
    let (Some(keys), Some(mut console)) = (keys, console) else {
        return;
    };
    if !ocw_render::menu::ride_input(context) {
        return;
    }
    let b = bindings.as_deref();
    let pressed = |a: Action| key_just_pressed(b, a, &keys) || pad_just_pressed(b, a, pads.iter());
    let hook = hooks.map_or(BindingHooks::default().action, |h| h.action);
    // `true`: the shared behaviour.
    let mut shared = |a: Action| {
        if !pressed(a) {
            return false;
        }
        match hook(a) {
            ActionUse::Shared => true,
            ActionUse::Drop => false,
            ActionUse::Run(f) => {
                commands.queue(move |w: &mut World| f(w));
                false
            }
        }
    };
    if shared(Action::Map) {
        let on = show.is_some_and(|s| s.0);
        console.queue(format!("set display.map {}", if on { "off" } else { "on" }));
    }
    if shared(Action::MapZoom) {
        let next = zoom.map(|z| z.0).unwrap_or_default().cycle();
        console.queue(format!("set display.map_zoom {}", next.label()));
    }
}

/// The bindings from the settings (when they changed).
fn refresh(
    settings: Option<Res<UserSettings>>,
    mut active: ResMut<ActiveBindings>,
    mut camera: ResMut<ocw_render::camera::CameraKey>,
    mut hud: ResMut<ocw_render::hud::RideKeys>,
    mut voice: Option<ResMut<ocw_render::voice_panel::VoiceKeys>>,
) {
    let Some(s) = settings else { return };
    if !s.is_changed() && !active.is_added() {
        return;
    }
    let b = Bindings::from_settings(&s.settings);
    if active.0 != b || hud.is_added() {
        camera.0 = key_code(b.first_key(Action::Camera));
        // The HUD's key lines and the junction fan's hint say the keys
        // in effect (spec 3.2.5).
        let h = b.hud_lines();
        *hud = ocw_render::hud::RideKeys {
            route: h.route,
            free: h.free,
            plain: h.plain,
            junction: h.junction,
        };
        // The voice status panel's hint: the key that turns My voice on.
        if let Some(v) = voice.as_deref_mut() {
            v.mic = match b.first_key(Action::MuteMic) {
                "none" => String::new(),
                k => k.to_owned(),
            };
        }
        active.0 = b;
    }
}

fn desktop(world: &World) -> bool {
    world
        .get_resource::<crate::UiPlatform>()
        .is_none_or(|p| p.0 == Platform::Desktop)
}

/// Sets and saves a binding setting.
fn set(world: &mut World, key: &str, value: &str) -> Result<(), String> {
    let mut us = world.resource_mut::<UserSettings>();
    us.settings.set(key, value)?;
    if let Err(e) = us.save() {
        debug!("{key} not saved: {e}");
    }
    Ok(())
}

/// The menu's capture: the first key or pad button pressed becomes the
/// waiting action's (Esc or Start cancels). Returns whether a capture is
/// waiting (the menu then ignores this frame's input).
pub fn capture(world: &mut World) -> bool {
    let Some(action) = world.get_resource::<BindCapture>().and_then(|c| c.0) else {
        return false;
    };
    let keys = world.get_resource::<ButtonInput<KeyCode>>().cloned();
    let mut q = world.query::<&Gamepad>();
    let pads: Vec<GamepadButton> = q
        .iter(world)
        .flat_map(|p| p.get_just_pressed().copied().collect::<Vec<_>>())
        .collect();
    let cancel = keys
        .as_ref()
        .is_some_and(|k| k.just_pressed(KeyCode::Escape))
        || pads.contains(&GamepadButton::Start);
    let mut done: Option<(Device, String)> = None;
    if !cancel {
        if let Some(k) = &keys
            && let Some(name) = k.get_just_pressed().find_map(|c| key_name_of(*c))
        {
            let shifted = format!("Shift+{name}");
            let name = if shift(k) && KEY_CHOICES.contains(&shifted.as_str()) {
                shifted
            } else {
                name.to_string()
            };
            done = Some((Device::Key, name));
        } else if let Some(name) = pads.iter().find_map(|b| pad_name_of(*b)) {
            done = Some((Device::Pad, name.to_string()));
        }
    }
    if cancel || done.is_some() {
        world.resource_mut::<BindCapture>().0 = None;
        let note = match done {
            Some((device, value)) => assign(world, action, device, &value),
            None => format!("{}: unchanged (cancelled)", action.def().label),
        };
        info!("{note}");
        world.resource_mut::<BindCapture>().1 = Some(note);
    }
    true
}

/// Binds the captured `value` to `action` with the swap rule of
/// [`Bindings::assign`], saves only what differs from the defaults and
/// returns the page's note.
pub fn assign(world: &mut World, action: Action, device: Device, value: &str) -> String {
    let desktop = desktop(world);
    let b = Bindings::from_settings(&world.resource::<UserSettings>().settings);
    let r = b.assign(action, device, value, desktop);
    if r.changes.is_empty() {
        return r.note;
    }
    let mut us = world.resource_mut::<UserSettings>();
    for (key, v) in &r.changes {
        let done = match v {
            Some(v) => us.settings.set(key, v).map(|_| ()),
            None => us.settings.unset(key),
        };
        if let Err(e) = done {
            warn!("{e}");
        }
    }
    if let Err(e) = us.save() {
        debug!("bindings not saved: {e}");
    }
    if let Some(mut c) = world.get_resource_mut::<BindCapture>() {
        c.1 = Some(r.note.clone());
    }
    r.note
}

/// Every key and button back to its default: out of the file.
pub fn reset(world: &mut World) -> Result<(), String> {
    let mut us = world.resource_mut::<UserSettings>();
    for key in Bindings::setting_keys() {
        us.settings.unset(&key)?;
    }
    if let Err(e) = us.save() {
        debug!("bindings not saved: {e}");
    }
    if let Some(mut c) = world.get_resource_mut::<BindCapture>() {
        c.1 = Some("All keys and buttons are back to their defaults".into());
    }
    Ok(())
}

/// The status line of a binding page: the capture's prompt, else the
/// conflicts.
pub fn status(world: &World, group: Option<usize>) -> String {
    let capture = world.get_resource::<BindCapture>();
    if let Some(a) = capture.and_then(|c| c.0) {
        return capture_prompt(a);
    }
    let note = capture.and_then(|c| c.1.clone());
    let Some(b) = world.get_resource::<ActiveBindings>() else {
        return String::new();
    };
    let c: Vec<String> = b
        .conflicts(desktop(world))
        .into_iter()
        .filter(|c| group.is_none_or(|g| c.action.def().group.index() == g))
        .filter(|c| c.with.is_none_or(|w| c.action < w))
        .map(|c| c.describe())
        .collect();
    let conflicts = match c.len() {
        0 => String::new(),
        _ => format!("Conflict: {}", c.join("; ")),
    };
    match note {
        Some(n) if conflicts.is_empty() => n,
        Some(n) => format!("{n}. {conflicts}"),
        None => conflicts,
    }
}

/// The capture's prompt (spec 3.2.5).
pub fn capture_prompt(a: Action) -> String {
    format!(
        "Press a key or gamepad button for {}... Esc / Start cancels",
        a.def().label
    )
}

/// The actions of a binding group's rows on this platform.
pub fn group_actions(world: &World, g: usize) -> Vec<Action> {
    let d = desktop(world);
    let Some(group) = ocw_client::bindings::Group::ALL.get(g) else {
        return Vec::new();
    };
    ACTIONS
        .iter()
        .filter(|a| a.group == *group && (d || !a.desktop_only))
        .map(|a| a.action)
        .collect()
}

/// Whether `a` has a conflict (on this platform).
pub fn in_conflict(world: &World, a: Action) -> bool {
    world
        .get_resource::<ActiveBindings>()
        .is_some_and(|b| b.conflicts(desktop(world)).iter().any(|c| c.action == a))
}

fn list(world: &World) -> String {
    let Some(b) = world.get_resource::<ActiveBindings>() else {
        return String::new();
    };
    let d = desktop(world);
    let mut out: Vec<String> = ACTIONS
        .iter()
        .filter(|a| d || !a.desktop_only)
        .map(|a| {
            format!(
                "{:<12} {:<22} key {:<10} pad {}",
                a.id,
                a.label,
                key_label(b.key(a.action)),
                pad_label(b.pad(a.action))
            )
        })
        .collect();
    let s = status(world, None);
    if !s.is_empty() {
        out.push(s);
    }
    out.join("\n")
}

fn register_commands(app: &mut App) {
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("bind")
            .about("Show the key and button bindings, or bind ACTION to a KEY (--pad: a gamepad BUTTON); --reset: all defaults; --capture ACTION: the next key or button")
            .arg(Arg::new("action"))
            .arg(Arg::new("input"))
            .arg(
                Arg::new("pad")
                    .long("pad")
                    .action(ArgAction::SetTrue)
                    .help("INPUT is a gamepad button (South, East, North, West, LB, RB, Select, L3, R3, DPadUp …)"),
            )
            .arg(
                Arg::new("reset")
                    .long("reset")
                    .action(ArgAction::SetTrue)
                    .help("Every key and button back to its default"),
            )
            .arg(
                Arg::new("capture")
                    .long("capture")
                    .action(ArgAction::SetTrue)
                    .help("Wait for the next key or button for ACTION (Esc / Start: cancel)"),
            ),
        |world, m| {
            if m.get_flag("reset") {
                reset(world)?;
                return Ok("all keys and buttons are back to their defaults".into());
            }
            let Some(id) = m.get_one::<String>("action") else {
                return Ok(list(world));
            };
            let action = Action::by_id(id).ok_or_else(|| {
                format!(
                    "unknown action {id:?} (one of {})",
                    ACTIONS.iter().map(|a| a.id).collect::<Vec<_>>().join(", ")
                )
            })?;
            if m.get_flag("capture") {
                *world.resource_mut::<BindCapture>() = BindCapture(Some(action), None);
                return Ok(capture_prompt(action));
            }
            let Some(input) = m.get_one::<String>("input") else {
                let b = world.resource::<ActiveBindings>();
                return Ok(format!(
                    "{}: key {}, pad {}",
                    action.def().label,
                    key_label(b.key(action)),
                    pad_label(b.pad(action))
                ));
            };
            let (key, value) = if m.get_flag("pad") {
                (
                    action.pad_setting(),
                    pad_name(input)
                        .ok_or_else(|| format!("unknown gamepad button {input:?}"))?
                        .to_string(),
                )
            } else {
                (
                    action.key_setting(),
                    parse_keys(input).map_err(|_| {
                        format!("{input:?} cannot be bound (Esc, ` and F12 are the menu's and the console's)")
                    })?,
                )
            };
            let dev = if m.get_flag("pad") { Device::Pad } else { Device::Key };
            // One key or button: as the capture (it moves from another
            // action, spec 3.2.5); two keys or none: as set.
            if value != "none" && !value.contains(',') {
                return Ok(assign(world, action, dev, &value));
            }
            set(world, &key, &value)?;
            let b = Bindings::from_settings(&world.resource::<UserSettings>().settings);
            let mut out = format!("{key} = {value}");
            if let Some(c) = b.conflict_of(action, dev, desktop(world)) {
                out.push_str(&format!(" (conflict: {})", c.describe()));
            }
            Ok(out)
        },
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_render::console::{ConsoleCorePlugin, ConsoleState};

    #[test]
    fn every_key_and_button_choice_has_a_bevy_input() {
        for k in KEY_CHOICES.iter().filter(|k| **k != "none") {
            assert!(key_code(k).is_some(), "{k}");
        }
        for p in ocw_client::bindings::PAD_CHOICES
            .iter()
            .filter(|p| **p != "none")
        {
            assert!(pad_button(p).is_some(), "{p}");
        }
        assert_eq!(key_name_of(KeyCode::Escape), None);
        assert_eq!(key_name_of(KeyCode::Backquote), None);
        assert_eq!(pad_name_of(GamepadButton::Start), None);
        assert_eq!(key_code("Shift+M"), Some(KeyCode::KeyM));
    }

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ConsoleCorePlugin))
            .init_resource::<ButtonInput<KeyCode>>()
            .add_plugins((crate::settings::SettingsPlugin, BindingsPlugin));
        app.update();
        app
    }

    fn run(app: &mut App, line: &str) -> String {
        let n = app.world().resource::<ConsoleState>().log().len();
        app.world_mut().resource_mut::<ConsoleState>().queue(line);
        app.update();
        app.world().resource::<ConsoleState>().log()[n..].join("\n")
    }

    #[test]
    fn bindings_follow_the_settings_and_fire_on_their_keys() {
        let mut app = app();
        let mut keys = ButtonInput::<KeyCode>::default();
        keys.press(KeyCode::KeyA);
        let b = app.world().resource::<ActiveBindings>().clone();
        assert!(b.key_just_pressed(Action::Autodrive, &keys));
        let out = run(&mut app, "bind autodrive Y");
        assert!(out.ends_with("Autodrive: Y"), "{out}");
        app.update();
        let b = app.world().resource::<ActiveBindings>().clone();
        assert!(!b.key_just_pressed(Action::Autodrive, &keys));
        keys.press(KeyCode::KeyY);
        assert!(b.key_just_pressed(Action::Autodrive, &keys));
        // A key another action has moves (spec 3.2.5): the video gets C;
        // the camera key follows its binding.
        let out = run(&mut app, "bind camera V");
        assert!(
            out.ends_with("Camera: V (V was taken: Video recording now has C)"),
            "{out}"
        );
        app.update();
        assert_eq!(
            app.world().resource::<ocw_render::camera::CameraKey>().0,
            Some(KeyCode::KeyV)
        );
        let b = app.world().resource::<ActiveBindings>().clone();
        assert_eq!(b.key(Action::Video), "C");
        assert_eq!(b.conflicts(true), []);
        // set makes a double binding: marked and listed.
        run(&mut app, "set keys.video V");
        app.update();
        assert!(run(&mut app, "bind").contains("Conflict: V: Camera and Video recording"));
        assert!(run(&mut app, "bind camera Escape").contains("cannot be bound"));
        // Two keys.
        assert!(run(&mut app, "bind u_turn U,F6").contains("keys.u_turn = U,F6"));
        run(&mut app, "bind --reset");
        app.update();
        assert!(app.world().resource::<ActiveBindings>().is_default());
        // Nothing of the mapping is left in the settings.
        let s = app.world().resource::<crate::settings::UserSettings>();
        assert!(
            !s.settings.to_toml().contains("keys"),
            "{}",
            s.settings.to_toml()
        );
    }

    /// Spec 3.2.5: the HUD's key lines and the junction fan's hint follow
    /// the mapping; the defaults are the HUD's own.
    #[test]
    fn the_hud_key_lines_follow_the_mapping() {
        let h = Bindings::default().hud_lines();
        let d = ocw_render::hud::RideKeys::default();
        assert_eq!(
            (h.route, h.free, h.plain, h.junction),
            (d.route, d.free, d.plain, d.junction)
        );
        let mut app = app();
        run(&mut app, "bind exit_left F5");
        app.update();
        let k = app.world().resource::<ocw_render::hud::RideKeys>();
        assert!(k.free.contains("F5 PgDn/E: exit"), "{}", k.free);
        assert_eq!(k.junction, "F5  < exit >  PgDn/E");
    }

    /// Spec 3.2 Q6: a press of D-pad down steps the navigation map's zoom
    /// (auto → 1 → 2 …) through the setting, as Z does.
    #[test]
    fn d_pad_down_steps_the_map_zoom_setting() {
        let mut app = app();
        app.init_resource::<ocw_render::navi_map::MapZoom>();
        let mut pad = Gamepad::default();
        pad.digital_mut().press(GamepadButton::DPadDown);
        let e = app.world_mut().spawn(pad).id();
        app.update();
        app.world_mut()
            .get_mut::<Gamepad>(e)
            .unwrap()
            .digital_mut()
            .clear();
        app.update();
        app.update();
        let s = app.world().resource::<crate::settings::UserSettings>();
        assert_eq!(s.value("display.map_zoom").to_string(), "1");
    }

    /// An app's hook drops or maps the shared map keys (OCW Studio: N its
    /// own 2D map, Z nothing) – no `set display.map…` line is queued.
    #[test]
    fn an_app_hook_drops_or_maps_the_map_keys() {
        #[derive(Resource, Default)]
        struct Toggled(u32);
        let press = |app: &mut App, k: KeyCode| {
            {
                let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
                keys.reset_all();
                keys.press(k);
            }
            app.update();
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .reset_all();
            app.update();
        };
        let lines = |app: &App| app.world().resource::<ConsoleState>().log().join("\n");
        // Without a hook: N and Z set the game's settings.
        let mut game = app();
        game.init_resource::<ocw_render::navi_map::MapZoom>();
        press(&mut game, KeyCode::KeyN);
        press(&mut game, KeyCode::KeyZ);
        let s = game.world().resource::<crate::settings::UserSettings>();
        assert_eq!(s.value("display.map_zoom").to_string(), "1");
        // With the hook: N runs the app's function, Z is dropped.
        let mut studio = app();
        studio
            .init_resource::<Toggled>()
            .init_resource::<ocw_render::navi_map::MapZoom>()
            .insert_resource(BindingHooks {
                action: |a| match a {
                    Action::Map => ActionUse::Run(|w| w.resource_mut::<Toggled>().0 += 1),
                    Action::MapZoom => ActionUse::Drop,
                    _ => ActionUse::Shared,
                },
            });
        let before = lines(&studio);
        press(&mut studio, KeyCode::KeyN);
        press(&mut studio, KeyCode::KeyZ);
        press(&mut studio, KeyCode::KeyN);
        assert_eq!(studio.world().resource::<Toggled>().0, 2);
        assert_eq!(lines(&studio), before, "nothing queued on the console");
        let s = studio.world().resource::<crate::settings::UserSettings>();
        assert_eq!(s.value("display.map_zoom").to_string(), "auto");
    }

    #[test]
    fn the_capture_takes_the_next_key_or_button() {
        let mut app = app();
        run(&mut app, "bind --capture wild");
        assert_eq!(app.world().resource::<BindCapture>().0, Some(Action::Wild));
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyK);
        assert!(capture(app.world_mut()));
        assert_eq!(app.world().resource::<BindCapture>().0, None);
        app.update();
        assert_eq!(
            app.world().resource::<ActiveBindings>().key(Action::Wild),
            "K"
        );
        // Shift held: the Shift binding; Esc cancels.
        run(&mut app, "bind --capture camera");
        {
            let mut k = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            k.reset_all();
            k.press(KeyCode::ShiftLeft);
            k.press(KeyCode::KeyN);
        }
        capture(app.world_mut());
        app.update();
        assert_eq!(
            app.world().resource::<ActiveBindings>().key(Action::Camera),
            "Shift+N"
        );
        run(&mut app, "bind --capture camera");
        {
            let mut k = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            k.reset_all();
            k.press(KeyCode::Escape);
        }
        assert!(capture(app.world_mut()));
        assert_eq!(app.world().resource::<BindCapture>().0, None);
        app.update();
        assert_eq!(
            app.world().resource::<ActiveBindings>().key(Action::Camera),
            "Shift+N"
        );
        assert!(!capture(app.world_mut()), "nothing waits");
    }

    /// Spec 3.2.5: the capture's prompt, a captured key that another
    /// action has (it moves, the other action gets the old key: no
    /// conflict), a captured pad button, the status line saying so.
    #[test]
    fn a_captured_key_of_another_action_swaps_and_the_page_says_so() {
        let mut app = app();
        run(&mut app, "bind --capture exit_left");
        assert_eq!(
            status(app.world(), None),
            "Press a key or gamepad button for Junction: exit left... Esc / Start cancels"
        );
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyU);
        assert!(capture(app.world_mut()));
        app.update();
        let b = app.world().resource::<ActiveBindings>().clone();
        assert_eq!(b.key(Action::ExitLeft), "U");
        assert_eq!(b.key(Action::UTurn), "Delete,Q", "the swap");
        assert_eq!(b.conflicts(true), []);
        assert_eq!(
            status(app.world(), None),
            "Junction: exit left: U (U was taken: U-turn now has Delete / Q)"
        );
        // Saved: only the two changed keys.
        let toml = app
            .world()
            .resource::<crate::settings::UserSettings>()
            .settings
            .to_toml();
        assert!(toml.contains("exit_left = \"U\""), "{toml}");
        assert!(toml.contains("u_turn = \"Delete,Q\""), "{toml}");
        assert!(!toml.contains("autodrive"), "{toml}");
        // A pad button.
        run(&mut app, "bind --capture u_turn");
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .reset_all();
        let mut pad = Gamepad::default();
        pad.digital_mut().press(GamepadButton::LeftTrigger);
        app.world_mut().spawn(pad);
        assert!(capture(app.world_mut()));
        app.update();
        assert_eq!(
            app.world().resource::<ActiveBindings>().pad(Action::UTurn),
            "LB"
        );
    }
}
