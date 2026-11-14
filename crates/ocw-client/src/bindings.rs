//! Rebindable keys and gamepad buttons of the ride (spec §3.2.1 M9, §14
//! Phase 7), shared by the desktop and the browser app.
//!
//! Every ride action ([`Action`]) has one keyboard key and one gamepad
//! button, each a setting of the registry (`keys.<id>` and `pad.<id>`,
//! sections `[keys]` and `[pad]` of `settings.toml` – the browser's
//! `localStorage` keeps the same text), so `set keys.autodrive Z`, the
//! menu (Settings → Controls → Keys and buttons) and `autoexec.cfg` share
//! one path, and the defaults are the registry's.
//!
//! **Fixed (not rebindable):** the menu and console keys (Esc, Enter,
//! the arrows in the menu, \` and F12; the pad's Start), the gamepad's
//! analog stick and triggers (steering, throttle, brake), the gamepad's
//! god mode chords (Select + North / West) and **unicorn mode's flight
//! keys** ([`GOD_KEYS`]: the cursor keys, + / − and L) – W, A, S and D never
//! fly (spec §14 Phase 13 Q3, decided: `WASD_FLIES = false`), whatever is
//! bound to them.
//!
//! **Conflicts** ([`Bindings::conflicts`]): two actions on one key or
//! button, or an action that also works in unicorn mode on one of its
//! flight keys. They are allowed (the user may be half-way through
//! swapping two keys) but marked on the page and listed by `bind`; both
//! actions then fire.
//!
//! Key names are layout-independent positions (Bevy's `KeyCode` by its
//! US-layout name: "A" is the key right of Caps Lock); `Shift+X` needs
//! Shift held, a plain `X` then does not fire if some action has
//! `Shift+X`.

use crate::settings::Settings;

/// A rebindable ride action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Action {
    Faster,
    Stop,
    SteerLeft,
    SteerRight,
    PowerUp,
    PowerDown,
    Autodrive,
    Wild,
    UTurn,
    Rejoin,
    ExitLeft,
    ExitRight,
    Camera,
    Video,
    Fullscreen,
    Restart,
    GodMode,
    MuteMic,
    MuteVoices,
}

/// The groups of the Keys and buttons page (one sub page each, at most
/// six rows: the ten-foot rule).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Group {
    /// Power and steering.
    Riding,
    /// Autodrive, wild mode, junctions, U-turn, rejoin.
    Modes,
    /// Camera, video, fullscreen, restart, unicorn mode.
    App,
    /// Voice chat.
    Voice,
}

impl Group {
    pub const ALL: [Group; 4] = [Group::Riding, Group::Modes, Group::App, Group::Voice];

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|g| *g == self).expect("listed")
    }

    pub fn label(self) -> &'static str {
        match self {
            Group::Riding => "Power and steering",
            Group::Modes => "Autodrive and junctions",
            Group::App => "Camera, video, app",
            Group::Voice => "Voice chat",
        }
    }
}

/// An action's definition: its setting id, label, group and defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionDef {
    pub action: Action,
    /// `keys.<id>` / `pad.<id>`.
    pub id: &'static str,
    pub label: &'static str,
    pub group: Group,
    /// The default key ("none": unbound).
    pub key: &'static str,
    pub pad: &'static str,
    /// Also works in unicorn mode (spec §14 Phase 13): must not be on a
    /// flight key.
    pub in_flight: bool,
    /// Only the desktop has it (the browser leaves the rows out).
    pub desktop_only: bool,
}

const fn def(
    action: Action,
    id: &'static str,
    label: &'static str,
    group: Group,
    key: &'static str,
    pad: &'static str,
) -> ActionDef {
    ActionDef {
        action,
        id,
        label,
        group,
        key,
        pad,
        in_flight: false,
        desktop_only: false,
    }
}

const fn flying(mut d: ActionDef) -> ActionDef {
    d.in_flight = true;
    d
}

const fn desktop(mut d: ActionDef) -> ActionDef {
    d.desktop_only = true;
    d
}

/// All actions with today's keys and buttons as defaults (spec §3.2.1,
/// §3.2.2, the README's key table).
pub const ACTIONS: [ActionDef; 19] = [
    def(Action::Faster, "faster", "Faster (power +)", Group::Riding, "Up", "DPadUp"),
    def(Action::Stop, "stop", "Stop (power 0, brake)", Group::Riding, "Down", "DPadDown"),
    def(Action::SteerLeft, "steer_left", "Steer left", Group::Riding, "Left", "none"),
    def(Action::SteerRight, "steer_right", "Steer right", Group::Riding, "Right", "none"),
    def(Action::PowerUp, "power_up", "Power +10 W", Group::Riding, "PageUp", "none"),
    def(Action::PowerDown, "power_down", "Power -10 W", Group::Riding, "PageDown", "none"),
    def(Action::Autodrive, "autodrive", "Autodrive", Group::Modes, "A", "South"),
    def(Action::Wild, "wild", "Wild mode", Group::Modes, "W", "West"),
    def(Action::UTurn, "u_turn", "U-turn", Group::Modes, "U", "East"),
    def(Action::Rejoin, "rejoin", "Back to the route", Group::Modes, "J", "North"),
    def(Action::ExitLeft, "exit_left", "Junction: exit left", Group::Modes, "Q", "DPadLeft"),
    def(Action::ExitRight, "exit_right", "Junction: exit right", Group::Modes, "E", "DPadRight"),
    flying(def(Action::Camera, "camera", "Camera", Group::App, "C", "none")),
    desktop(flying(def(Action::Video, "video", "Video recording", Group::App, "V", "none"))),
    desktop(flying(def(Action::Fullscreen, "fullscreen", "Fullscreen", Group::App, "F11", "none"))),
    desktop(def(Action::Restart, "restart", "Restart", Group::App, "R", "none")),
    desktop(flying(def(Action::GodMode, "god", "Unicorn mode", Group::App, "G", "none"))),
    flying(def(Action::MuteMic, "mute_mic", "Mute microphone", Group::Voice, "M", "Select")),
    flying(def(Action::MuteVoices, "mute_voices", "Mute voices", Group::Voice, "Shift+M", "R3")),
];

impl Action {
    pub fn def(self) -> &'static ActionDef {
        ACTIONS
            .iter()
            .find(|d| d.action == self)
            .expect("every action is listed")
    }

    pub fn by_id(id: &str) -> Option<Action> {
        ACTIONS.iter().find(|d| d.id == id).map(|d| d.action)
    }

    /// The setting of its key, `keys.<id>`.
    pub fn key_setting(self) -> String {
        format!("keys.{}", self.def().id)
    }

    /// The setting of its button, `pad.<id>`.
    pub fn pad_setting(self) -> String {
        format!("pad.{}", self.def().id)
    }
}

/// Keyboard or gamepad.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Device {
    Key,
    Pad,
}

/// The key names a binding can take (positions, US-layout names), "none"
/// first. Not offered: Esc, \` and F12 (menu and console), the modifier
/// keys alone.
pub const KEY_CHOICES: &[&str] = &[
    "none", "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q",
    "R", "S", "T", "U", "V", "W", "X", "Y", "Z", "0", "1", "2", "3", "4", "5", "6", "7", "8", "9",
    "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "Up", "Down", "Left",
    "Right", "PageUp", "PageDown", "Home", "End", "Insert", "Delete", "Space", "Enter", "Tab",
    "Backspace", "Num0", "Num1", "Num2", "Num3", "Num4", "Num5", "Num6", "Num7", "Num8", "Num9",
    "Num+", "Num-", "Num*", "Num/", "Num.", "NumEnter", "Minus", "Equal", "BracketLeft",
    "BracketRight", "Semicolon", "Quote", "Comma", "Period", "Slash", "Backslash", "Shift+A",
    "Shift+B", "Shift+C", "Shift+D", "Shift+E", "Shift+F", "Shift+G", "Shift+H", "Shift+I",
    "Shift+J", "Shift+K", "Shift+L", "Shift+M", "Shift+N", "Shift+O", "Shift+P", "Shift+Q",
    "Shift+R", "Shift+S", "Shift+T", "Shift+U", "Shift+V", "Shift+W", "Shift+X", "Shift+Y",
    "Shift+Z",
];

/// The gamepad buttons a binding can take ("none" first). Not offered:
/// Start (the menu), the triggers LT / RT (the throttle and the brake,
/// read as analog values).
pub const PAD_CHOICES: &[&str] = &[
    "none",
    "South",
    "East",
    "North",
    "West",
    "LB",
    "RB",
    "Select",
    "L3",
    "R3",
    "DPadUp",
    "DPadDown",
    "DPadLeft",
    "DPadRight",
];

/// Unicorn mode's fixed flight keys (spec §14 Phase 13B, Q3): the cursor
/// keys, the + / − keys (the keypad's and the layouts' – US `=` / `-`,
/// German `+` / `-` on `]` / `/`) and L (look at). Never W, A, S, D.
pub const GOD_KEYS: &[&str] = &[
    "Up",
    "Down",
    "Left",
    "Right",
    "Num+",
    "Num-",
    "Equal",
    "Minus",
    "BracketRight",
    "Slash",
    "L",
];

/// A pad button as shown: "D-pad up", "A / Cross (South)".
pub fn pad_label(name: &str) -> &str {
    match name {
        "South" => "South (A)",
        "East" => "East (B)",
        "North" => "North (Y)",
        "West" => "West (X)",
        "DPadUp" => "D-pad up",
        "DPadDown" => "D-pad down",
        "DPadLeft" => "D-pad left",
        "DPadRight" => "D-pad right",
        "none" => "-",
        other => other,
    }
}

/// A key as shown ("-" for none).
pub fn key_label(name: &str) -> &str {
    if name == "none" { "-" } else { name }
}

/// A conflict of one binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub action: Action,
    pub device: Device,
    /// The key or button.
    pub input: String,
    /// The other action on it; `None`: a flight key of unicorn mode.
    pub with: Option<Action>,
}

impl Conflict {
    /// "V: Camera and Video recording", "L: Camera and unicorn mode's flight".
    pub fn describe(&self) -> String {
        let input = match self.device {
            Device::Key => self.input.clone(),
            Device::Pad => pad_label(&self.input).to_string(),
        };
        match self.with {
            Some(w) => format!("{input}: {} and {}", self.action.def().label, w.def().label),
            None => format!(
                "{input}: {} and unicorn mode's flight keys",
                self.action.def().label
            ),
        }
    }
}

/// The keys and buttons of all actions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bindings {
    keys: Vec<String>,
    pads: Vec<String>,
}

impl Default for Bindings {
    fn default() -> Self {
        Self {
            keys: ACTIONS.iter().map(|d| d.key.to_string()).collect(),
            pads: ACTIONS.iter().map(|d| d.pad.to_string()).collect(),
        }
    }
}

fn index(a: Action) -> usize {
    ACTIONS.iter().position(|d| d.action == a).expect("listed")
}

impl Bindings {
    /// The bindings of the settings (`keys.*`, `pad.*`).
    pub fn from_settings(s: &Settings) -> Self {
        let mut b = Self::default();
        for (i, d) in ACTIONS.iter().enumerate() {
            if let Ok(v) = s.get(&format!("keys.{}", d.id)) {
                b.keys[i] = v.to_string();
            }
            if let Ok(v) = s.get(&format!("pad.{}", d.id)) {
                b.pads[i] = v.to_string();
            }
        }
        b
    }

    /// The key of `a` ("none": unbound).
    pub fn key(&self, a: Action) -> &str {
        &self.keys[index(a)]
    }

    pub fn pad(&self, a: Action) -> &str {
        &self.pads[index(a)]
    }

    pub fn get(&self, a: Action, d: Device) -> &str {
        match d {
            Device::Key => self.key(a),
            Device::Pad => self.pad(a),
        }
    }

    /// Binds `a` (the setting does this; tests and previews).
    pub fn set(&mut self, a: Action, d: Device, input: &str) {
        let slot = match d {
            Device::Key => &mut self.keys[index(a)],
            Device::Pad => &mut self.pads[index(a)],
        };
        *slot = input.to_string();
    }

    /// Whether every binding is its default.
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// The conflicts among the actions of `platform` (both ends of a
    /// shared key are listed, each with the other).
    pub fn conflicts(&self, desktop: bool) -> Vec<Conflict> {
        let mut out = Vec::new();
        let on = |d: &ActionDef| desktop || !d.desktop_only;
        for device in [Device::Key, Device::Pad] {
            for a in ACTIONS.iter().filter(|d| on(d)) {
                let input = self.get(a.action, device);
                if input == "none" {
                    continue;
                }
                for b in ACTIONS.iter().filter(|d| on(d) && d.action != a.action) {
                    if self.get(b.action, device) == input {
                        out.push(Conflict {
                            action: a.action,
                            device,
                            input: input.into(),
                            with: Some(b.action),
                        });
                    }
                }
                if device == Device::Key && a.in_flight && desktop && GOD_KEYS.contains(&input) {
                    out.push(Conflict {
                        action: a.action,
                        device,
                        input: input.into(),
                        with: None,
                    });
                }
            }
        }
        out
    }

    /// The first conflict of `a`'s binding on `device`, if any.
    pub fn conflict_of(&self, a: Action, device: Device, desktop: bool) -> Option<Conflict> {
        self.conflicts(desktop)
            .into_iter()
            .find(|c| c.action == a && c.device == device)
    }

    /// Whether `a`'s key fires: `down(name)` says whether the key `name`
    /// (without Shift) is down / went down this frame, `shift` whether
    /// Shift is held. `Shift+X` needs Shift; a plain `X` does not fire
    /// with Shift when another action is on `Shift+X`.
    pub fn key_fires(&self, a: Action, down: impl Fn(&str) -> bool, shift: bool) -> bool {
        let k = self.key(a);
        if k == "none" {
            return false;
        }
        match k.strip_prefix("Shift+") {
            Some(base) => shift && down(base),
            None => {
                down(k) && !(shift && self.keys.iter().any(|o| o.strip_prefix("Shift+") == Some(k)))
            }
        }
    }

    /// The console lines that set every binding back to its default.
    pub fn reset_lines() -> Vec<String> {
        ACTIONS
            .iter()
            .flat_map(|d| {
                [
                    format!("set keys.{} {}", d.id, d.key),
                    format!("set pad.{} {}", d.id, d.pad),
                ]
            })
            .collect()
    }
}

/// The step from `current` through `choices` (wrapping), skipping
/// nothing: ← / → on a binding row.
pub fn step_choice<'a>(choices: &[&'a str], current: &str, dir: i32) -> &'a str {
    let n = choices.len() as i32;
    let at = choices.iter().position(|c| *c == current).unwrap_or(0) as i32;
    choices[(at + dir.signum()).rem_euclid(n) as usize]
}

/// The key name of a typed key name, any case ("a" → "A", "pageup" →
/// "PageUp", "shift+m" → "Shift+M").
pub fn key_name(text: &str) -> Option<&'static str> {
    KEY_CHOICES
        .iter()
        .find(|k| k.eq_ignore_ascii_case(text.trim()))
        .copied()
}

pub fn pad_name(text: &str) -> Option<&'static str> {
    PAD_CHOICES
        .iter()
        .find(|k| k.eq_ignore_ascii_case(text.trim()))
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_todays_keys_without_conflicts() {
        let b = Bindings::default();
        assert_eq!(b.key(Action::Autodrive), "A");
        assert_eq!(b.pad(Action::Autodrive), "South");
        assert_eq!(b.key(Action::MuteVoices), "Shift+M");
        assert_eq!(b.key(Action::Faster), "Up");
        assert!(b.is_default());
        assert_eq!(b.conflicts(true), []);
        assert_eq!(b.conflicts(false), []);
        // Every default is a choice the settings accept.
        for d in &ACTIONS {
            assert!(KEY_CHOICES.contains(&d.key), "{}", d.key);
            assert!(PAD_CHOICES.contains(&d.pad), "{}", d.pad);
            assert_eq!(Action::by_id(d.id), Some(d.action));
            // The registry has both settings with the same defaults.
            let k = crate::settings::def(&d.action.key_setting()).expect("keys.*");
            assert_eq!(k.default, d.key);
            let p = crate::settings::def(&d.action.pad_setting()).expect("pad.*");
            assert_eq!(p.default, d.pad);
            let browser = crate::menu::model::Platform::Browser;
            assert_eq!(
                crate::settings::applies(k.key, browser),
                !d.desktop_only,
                "{}",
                k.key
            );
        }
        // Each group fits a page (six rows, the ten-foot rule).
        for g in Group::ALL {
            let n = ACTIONS.iter().filter(|d| d.group == g).count();
            assert!((1..=6).contains(&n), "{g:?}: {n}");
        }
    }

    #[test]
    fn reserved_inputs_cannot_be_chosen() {
        for k in ["Escape", "Esc", "Backquote", "F12", "ShiftLeft", "Shift"] {
            assert_eq!(key_name(k), None, "{k}");
        }
        for p in ["Start", "LT", "RT"] {
            assert_eq!(pad_name(p), None, "{p}");
        }
        assert_eq!(key_name("pageup"), Some("PageUp"));
        assert_eq!(key_name(" shift+m "), Some("Shift+M"));
        assert_eq!(pad_name("dpadleft"), Some("DPadLeft"));
    }

    #[test]
    fn two_actions_on_one_key_are_a_conflict_both_ways() {
        let mut b = Bindings::default();
        b.set(Action::Camera, Device::Key, "V");
        let c = b.conflicts(true);
        assert_eq!(c.len(), 2, "{c:?}");
        assert_eq!(
            b.conflict_of(Action::Camera, Device::Key, true).unwrap().with,
            Some(Action::Video)
        );
        assert_eq!(
            b.conflict_of(Action::Video, Device::Key, true).unwrap().describe(),
            "V: Video recording and Camera"
        );
        // Video is a desktop key: no conflict in the browser.
        assert_eq!(b.conflicts(false), []);
        // Pads too; "none" never conflicts.
        let mut b = Bindings::default();
        b.set(Action::Wild, Device::Pad, "South");
        assert_eq!(
            b.conflict_of(Action::Autodrive, Device::Pad, true)
                .unwrap()
                .describe(),
            "South (A): Autodrive and Wild mode"
        );
        let mut b = Bindings::default();
        b.set(Action::Camera, Device::Pad, "none");
        b.set(Action::Video, Device::Pad, "none");
        assert_eq!(b.conflicts(true), []);
    }

    /// The no-WASD rule of unicorn mode (spec §14 Phase 13 Q3): its flight
    /// keys are fixed, never W / A / S / D; an action that also works in
    /// flight must not sit on one of them.
    #[test]
    fn unicorn_mode_keeps_its_flight_keys_without_wasd() {
        for k in ["W", "A", "S", "D"] {
            assert!(!GOD_KEYS.contains(&k), "{k} must not fly");
        }
        let mut b = Bindings::default();
        // A riding key on an arrow: fine (riding keys are off in flight).
        b.set(Action::Autodrive, Device::Key, "W");
        b.set(Action::Wild, Device::Key, "A");
        assert_eq!(b.conflicts(true), []);
        // The camera (which works in flight) on L: a conflict.
        b.set(Action::Camera, Device::Key, "L");
        let c = b.conflict_of(Action::Camera, Device::Key, true).unwrap();
        assert_eq!(c.with, None);
        assert_eq!(c.describe(), "L: Camera and unicorn mode's flight keys");
        // The browser has no unicorn mode.
        assert_eq!(b.conflict_of(Action::Camera, Device::Key, false), None);
    }

    #[test]
    fn shift_bindings_need_shift_and_take_the_plain_key_from_others() {
        let b = Bindings::default();
        let m = |k: &str| k == "M";
        assert!(b.key_fires(Action::MuteMic, m, false));
        assert!(!b.key_fires(Action::MuteMic, m, true), "Shift+M is taken");
        assert!(b.key_fires(Action::MuteVoices, m, true));
        assert!(!b.key_fires(Action::MuteVoices, m, false));
        // A plain key without a Shift twin fires with Shift too.
        assert!(b.key_fires(Action::Autodrive, |k| k == "A", true));
        let mut b = Bindings::default();
        b.set(Action::Camera, Device::Key, "none");
        assert!(!b.key_fires(Action::Camera, |_| true, false));
    }

    #[test]
    fn bindings_come_from_the_settings_and_reset_to_the_defaults() {
        let (mut s, warnings) =
            Settings::parse("[keys]\nautodrive = \"Z\"\n[pad]\nu_turn = \"LB\"\n").unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        let b = Bindings::from_settings(&s);
        assert_eq!(b.key(Action::Autodrive), "Z");
        assert_eq!(b.pad(Action::UTurn), "LB");
        assert_eq!(b.key(Action::Wild), "W", "the rest: defaults");
        assert!(!b.is_default());
        // Saved and read again.
        let (s2, _) = Settings::parse(&s.to_toml()).unwrap();
        assert_eq!(Bindings::from_settings(&s2), b);
        // A reserved or unknown key in the file falls back to the default.
        let (s3, warnings) = Settings::parse("[keys]\ncamera = \"Escape\"\n").unwrap();
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(Bindings::from_settings(&s3).key(Action::Camera), "C");
        // Reset: every binding back.
        for line in Bindings::reset_lines() {
            let mut w = line.split_whitespace().skip(1);
            let (k, v) = (w.next().unwrap(), w.next().unwrap());
            s.set(k, v).unwrap();
        }
        assert!(Bindings::from_settings(&s).is_default());
    }

    #[test]
    fn stepping_through_the_choices_wraps() {
        assert_eq!(step_choice(KEY_CHOICES, "A", 1), "B");
        assert_eq!(step_choice(KEY_CHOICES, "none", -1), "Shift+Z");
        assert_eq!(step_choice(PAD_CHOICES, "DPadRight", 1), "none");
    }
}
