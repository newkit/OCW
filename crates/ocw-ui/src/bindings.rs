//! Key and button rebinding in both apps (spec §3.2.1 M9): the bindings
//! of the settings (`keys.*`, `pad.*`, `ocw_client::bindings`) as Bevy
//! keys and buttons ([`ActiveBindings`], read by the apps' ride input),
//! the capture of a new key or button for the menu's binding rows
//! ([`BindCapture`], `bind --capture ID`) and the console command
//! `bind`.

use bevy::prelude::*;
use clap::{Arg, ArgAction, Command};
use ocw_client::bindings::{
    ACTIONS, Action, Bindings, Device, KEY_CHOICES, key_label, key_name, pad_label, pad_name,
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

/// The action waiting for its new key or button (the menu's binding rows).
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BindCapture(pub Option<Action>);

pub struct BindingsPlugin;

impl Plugin for BindingsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ActiveBindings>()
            .init_resource::<BindCapture>()
            .init_resource::<ocw_render::camera::CameraKey>()
            .add_systems(PreUpdate, refresh)
            .add_systems(Update, map_keys);
        register_commands(app);
    }
}

/// The navigation map's keys (spec §3.2 *Navigation map*): N shows or
/// hides it, Z steps the zoom (auto → 1 … 7 → auto); both as settings
/// (`display.map`, `display.map_zoom`), so they are kept.
pub fn map_keys(
    keys: Option<Res<ButtonInput<KeyCode>>>,
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
    if pressed(Action::Map) {
        let on = show.is_some_and(|s| s.0);
        console.queue(format!("set display.map {}", if on { "off" } else { "on" }));
    }
    if pressed(Action::MapZoom) {
        let next = zoom.map(|z| z.0).unwrap_or_default().cycle();
        console.queue(format!("set display.map_zoom {}", next.label()));
    }
}

/// The bindings from the settings (when they changed).
fn refresh(
    settings: Option<Res<UserSettings>>,
    mut active: ResMut<ActiveBindings>,
    mut camera: ResMut<ocw_render::camera::CameraKey>,
) {
    let Some(s) = settings else { return };
    if !s.is_changed() && !active.is_added() {
        return;
    }
    let b = Bindings::from_settings(&s.settings);
    if active.0 != b {
        camera.0 = key_code(b.key(Action::Camera));
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
    let id = action.def().id;
    let mut done: Option<(String, String)> = None;
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
            done = Some((format!("keys.{id}"), name));
        } else if let Some(name) = pads.iter().find_map(|b| pad_name_of(*b)) {
            done = Some((format!("pad.{id}"), name.to_string()));
        }
    }
    if cancel || done.is_some() {
        world.resource_mut::<BindCapture>().0 = None;
        if let Some((key, value)) = done {
            match set(world, &key, &value) {
                Ok(()) => info!("{key} = {value}"),
                Err(e) => warn!("{e}"),
            }
        }
    }
    true
}

/// The status line of a binding page: the capture's prompt, else the
/// conflicts.
pub fn status(world: &World, group: Option<usize>) -> String {
    if let Some(a) = world.get_resource::<BindCapture>().and_then(|c| c.0) {
        return format!(
            "Press the new key or gamepad button for {} (Esc / Start: cancel)",
            a.def().label
        );
    }
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
    match c.len() {
        0 => String::new(),
        _ => format!("Conflict: {}", c.join("; ")),
    }
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
                for line in Bindings::reset_lines() {
                    let mut w = line.split_whitespace().skip(1);
                    let (k, v) = (w.next().expect("key"), w.next().expect("value"));
                    world.resource_mut::<UserSettings>().settings.set(k, v)?;
                }
                if let Err(e) = world.resource_mut::<UserSettings>().save() {
                    debug!("bindings not saved: {e}");
                }
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
                world.resource_mut::<BindCapture>().0 = Some(action);
                return Ok(format!(
                    "press the new key or button for {} (Esc / Start: cancel)",
                    action.def().label
                ));
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
                    pad_name(input).ok_or_else(|| format!("unknown gamepad button {input:?}"))?,
                )
            } else {
                (
                    action.key_setting(),
                    key_name(input).ok_or_else(|| {
                        format!("{input:?} cannot be bound (Esc, ` and F12 are the menu's and the console's)")
                    })?,
                )
            };
            set(world, &key, value)?;
            let b = Bindings::from_settings(&world.resource::<UserSettings>().settings);
            let dev = if m.get_flag("pad") { Device::Pad } else { Device::Key };
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
        assert!(out.contains("keys.autodrive = Y"), "{out}");
        app.update();
        let b = app.world().resource::<ActiveBindings>().clone();
        assert!(!b.key_just_pressed(Action::Autodrive, &keys));
        keys.press(KeyCode::KeyY);
        assert!(b.key_just_pressed(Action::Autodrive, &keys));
        // A conflict is said; the camera key follows its binding.
        let out = run(&mut app, "bind camera V");
        assert!(
            out.contains("conflict: V: Camera and Video recording"),
            "{out}"
        );
        app.update();
        assert_eq!(
            app.world().resource::<ocw_render::camera::CameraKey>().0,
            Some(KeyCode::KeyV)
        );
        assert!(run(&mut app, "bind").contains("Conflict: V: Camera and Video recording"));
        assert!(run(&mut app, "bind camera Escape").contains("cannot be bound"));
        run(&mut app, "bind --reset");
        app.update();
        assert!(app.world().resource::<ActiveBindings>().is_default());
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
}
