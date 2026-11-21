//! The input mapping: rebindable keys and gamepad buttons of the ride
//! (spec 3.2.5, 3.2.1 M9), shared by the desktop and the browser app.
//!
//! Every ride action ([`Action`]) has one or two keyboard keys
//! ([`MAX_KEYS`]; the junction exits have Delete / Q and Page Down / E)
//! and one gamepad button, each a setting of the registry (`keys.<id>`
//! and `pad.<id>`: `[settings.keys]` and `[settings.pad]` of `ocw.toml`
//! – only the changed ones; the browser's `localStorage` keeps the same
//! text), so `set keys.autodrive Z`, `bind`, the menu (Settings →
//! Controls → Input mapping) and `autoexec.cfg` share one path, and the
//! defaults are the registry's.
//!
//! **Fixed (not rebindable):** the menu and console keys (Esc, Enter,
//! the arrows in the menu, \` and F12; the pad's Start, A / B in the
//! menu) – the menu reads its own keys, whatever the ride's are, so the
//! mapping can never lock the user out –, the gamepad's analog stick and
//! triggers (steering, throttle, brake), the gamepad's god mode chords
//! (Select + North / West) and **unicorn mode's flight keys**
//! ([`GOD_KEYS`]: the cursor keys, + / − and L) – W, A, S and D never fly
//! (spec §14 Phase 13 Q3, decided: `WASD_FLIES = false`), whatever is
//! bound to them.
//!
//! **Conflicts** ([`Bindings::conflicts`]): two actions on one key or
//! button, or an action that also works in unicorn mode on one of its
//! flight keys. A key or button **captured** on the Input mapping page
//! moves from the action that had it ([`Bindings::assign`]: a swap), so
//! the capture never makes a double binding; stepping with ← / → or
//! `set` may, and those are marked on the page and listed by `bind`
//! (both actions then fire).
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
    Strava,
    MuteMic,
    MuteVoices,
    /// The HUD's navigation map on / off and its zoom (spec §3.2).
    Map,
    MapZoom,
}

/// The groups of the Input mapping page (one sub page each, at most
/// six rows: the ten-foot rule; four groups, so the page with the pad's
/// Confirm button and the reset fits six rows).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Group {
    /// Power and steering.
    Riding,
    /// Autodrive, wild mode, junctions, U-turn, rejoin.
    Modes,
    /// Camera, video, fullscreen, restart, unicorn mode, Strava.
    App,
    /// Voice chat and the navigation map.
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
            Group::Voice => "Voice chat and map",
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
pub const ACTIONS: [ActionDef; 20] = [
    def(
        Action::Faster,
        "faster",
        "Faster (power +)",
        Group::Riding,
        "Up",
        "DPadUp",
    ),
    def(
        Action::Stop,
        "stop",
        "Stop (power 0, brake)",
        Group::Riding,
        "Down",
        // D-pad down zooms the navigation map (spec 3.2 Q6, 2026-11-19);
        // the left stick down and the left trigger still stop.
        "none",
    ),
    def(
        Action::SteerLeft,
        "steer_left",
        "Steer left",
        Group::Riding,
        "Left",
        "none",
    ),
    def(
        Action::SteerRight,
        "steer_right",
        "Steer right",
        Group::Riding,
        "Right",
        "none",
    ),
    def(
        Action::Autodrive,
        "autodrive",
        "Autodrive",
        Group::Modes,
        "A",
        "South",
    ),
    def(Action::Wild, "wild", "Wild mode", Group::Modes, "W", "West"),
    def(Action::UTurn, "u_turn", "U-turn", Group::Modes, "U", "East"),
    def(
        Action::Rejoin,
        "rejoin",
        "Back to the route",
        Group::Modes,
        "J",
        "North",
    ),
    def(
        Action::ExitLeft,
        "exit_left",
        "Junction: exit left",
        Group::Modes,
        // Spec 3.2.5 (2026-11-21): Delete and Page Down choose the exits,
        // Q / E stay as second keys; ← / → only steer.
        "Delete,Q",
        "DPadLeft",
    ),
    def(
        Action::ExitRight,
        "exit_right",
        "Junction: exit right",
        Group::Modes,
        "PageDown,E",
        "DPadRight",
    ),
    flying(def(
        Action::Camera,
        "camera",
        "Camera",
        Group::App,
        "C",
        "none",
    )),
    desktop(flying(def(
        Action::Video,
        "video",
        "Video recording",
        Group::App,
        "V",
        "none",
    ))),
    desktop(flying(def(
        Action::Fullscreen,
        "fullscreen",
        "Fullscreen",
        Group::App,
        "F11",
        "none",
    ))),
    desktop(def(
        Action::Restart,
        "restart",
        "Restart",
        Group::App,
        "R",
        "none",
    )),
    flying(def(
        Action::GodMode,
        "god",
        "Unicorn mode",
        Group::App,
        "G",
        "none",
    )),
    // The Strava recording (spec §2.8.1 ST1; the browser says that Strava
    // needs the desktop app).
    flying(def(
        Action::Strava,
        "strava",
        "Strava recording",
        Group::App,
        "S",
        "L3",
    )),
    flying(def(
        Action::MuteMic,
        "mute_mic",
        "Mute microphone",
        Group::Voice,
        "M",
        "Select",
    )),
    flying(def(
        Action::MuteVoices,
        "mute_voices",
        "Mute voices",
        Group::Voice,
        "Shift+M",
        "R3",
    )),
    // The HUD's navigation map (spec §3.2 *Navigation map*).
    // Both also in unicorn flight, where the map shows centred on the
    // unicorn (spec 3.2 Q7).
    flying(def(
        Action::Map,
        "map",
        "Navigation map on / off",
        Group::Voice,
        "N",
        "none",
    )),
    // The zoom on the d-pad (spec 3.2 Q6, decided 2026-11-19: "yes, the
    // zoom on a d-pad direction"): D-pad down, which "stop" gave up.
    flying(def(
        Action::MapZoom,
        "map_zoom",
        "Navigation map zoom",
        Group::Voice,
        "Z",
        "DPadDown",
    )),
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
    "none",
    "A",
    "B",
    "C",
    "D",
    "E",
    "F",
    "G",
    "H",
    "I",
    "J",
    "K",
    "L",
    "M",
    "N",
    "O",
    "P",
    "Q",
    "R",
    "S",
    "T",
    "U",
    "V",
    "W",
    "X",
    "Y",
    "Z",
    "0",
    "1",
    "2",
    "3",
    "4",
    "5",
    "6",
    "7",
    "8",
    "9",
    "F1",
    "F2",
    "F3",
    "F4",
    "F5",
    "F6",
    "F7",
    "F8",
    "F9",
    "F10",
    "F11",
    "Up",
    "Down",
    "Left",
    "Right",
    "PageUp",
    "PageDown",
    "Home",
    "End",
    "Insert",
    "Delete",
    "Space",
    "Enter",
    "Tab",
    "Backspace",
    "Num0",
    "Num1",
    "Num2",
    "Num3",
    "Num4",
    "Num5",
    "Num6",
    "Num7",
    "Num8",
    "Num9",
    "Num+",
    "Num-",
    "Num*",
    "Num/",
    "Num.",
    "NumEnter",
    "Minus",
    "Equal",
    "BracketLeft",
    "BracketRight",
    "Semicolon",
    "Quote",
    "Comma",
    "Period",
    "Slash",
    "Backslash",
    "Shift+A",
    "Shift+B",
    "Shift+C",
    "Shift+D",
    "Shift+E",
    "Shift+F",
    "Shift+G",
    "Shift+H",
    "Shift+I",
    "Shift+J",
    "Shift+K",
    "Shift+L",
    "Shift+M",
    "Shift+N",
    "Shift+O",
    "Shift+P",
    "Shift+Q",
    "Shift+R",
    "Shift+S",
    "Shift+T",
    "Shift+U",
    "Shift+V",
    "Shift+W",
    "Shift+X",
    "Shift+Y",
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

/// The keys of a key binding's value: "Delete,Q" → Delete, Q; "none" →
/// nothing.
pub fn key_list(value: &str) -> impl Iterator<Item = &str> {
    value
        .split(',')
        .map(str::trim)
        .filter(|k| !k.is_empty() && *k != "none")
}

/// At most this many keys per action (spec 3.2.5): a main key and a
/// second one (the junction exits: Delete / Q, Page Down / E).
pub const MAX_KEYS: usize = 2;

/// A typed key binding in its saved form: one or two key names (any
/// case, comma separated: "delete, q" → "Delete,Q"), or "none".
pub fn parse_keys(text: &str) -> Result<String, String> {
    let text = text.trim();
    if text.eq_ignore_ascii_case("none") {
        return Ok("none".into());
    }
    let mut out: Vec<&str> = Vec::new();
    for part in text.split(',') {
        let k = key_name(part).filter(|k| *k != "none").ok_or_else(|| {
            format!(
                "{:?} is not a key that can be bound (Esc, ` and F12 are the menu's and the console's)",
                part.trim()
            )
        })?;
        if !out.contains(&k) {
            out.push(k);
        }
    }
    if out.len() > MAX_KEYS {
        return Err(format!("at most {MAX_KEYS} keys per action"));
    }
    Ok(out.join(","))
}

/// A key binding as shown: "Delete / Q", "-" for none.
pub fn key_label(value: &str) -> String {
    let keys: Vec<&str> = key_list(value).collect();
    if keys.is_empty() {
        "-".into()
    } else {
        keys.join(" / ")
    }
}

/// A key as the HUD's key line says it: "Del", "PgDn", "up".
pub fn hud_key(name: &str) -> String {
    match name {
        "Delete" => "Del".into(),
        "PageDown" => "PgDn".into(),
        "PageUp" => "PgUp".into(),
        "Up" => "up".into(),
        "Down" => "down".into(),
        "Left" => "left".into(),
        "Right" => "right".into(),
        other => other.into(),
    }
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

/// What a captured key or button did ([`Bindings::assign`]): the
/// settings to change (`None`: back to the default, out of the file) and
/// the line for the page's status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assigned {
    pub changes: Vec<(String, Option<String>)>,
    pub note: String,
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

    /// The key binding of `a` as saved ("Delete,Q"; "none": unbound).
    pub fn key(&self, a: Action) -> &str {
        &self.keys[index(a)]
    }

    /// The keys of `a` (none, one or two).
    pub fn keys_of(&self, a: Action) -> impl Iterator<Item = &str> {
        key_list(self.key(a))
    }

    /// The first key of `a` ("none" when unbound).
    pub fn first_key(&self, a: Action) -> &str {
        self.keys_of(a).next().unwrap_or("none")
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

    /// The inputs of `a` on `d` (keys: none, one or two; a button: none
    /// or one).
    fn inputs(&self, a: Action, d: Device) -> Vec<&str> {
        match d {
            Device::Key => self.keys_of(a).collect(),
            Device::Pad => Some(self.pad(a))
                .filter(|p| *p != "none")
                .into_iter()
                .collect(),
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

    /// The actions whose binding differs from its default (keyboard or
    /// gamepad).
    pub fn changed(&self) -> Vec<Action> {
        let d = Self::default();
        ACTIONS
            .iter()
            .map(|a| a.action)
            .filter(|a| self.key(*a) != d.key(*a) || self.pad(*a) != d.pad(*a))
            .collect()
    }

    /// The conflicts among the actions of `platform` (both ends of a
    /// shared key are listed, each with the other).
    pub fn conflicts(&self, desktop: bool) -> Vec<Conflict> {
        let mut out = Vec::new();
        let on = |d: &ActionDef| desktop || !d.desktop_only;
        for device in [Device::Key, Device::Pad] {
            for a in ACTIONS.iter().filter(|d| on(d)) {
                for input in self.inputs(a.action, device) {
                    for b in ACTIONS.iter().filter(|d| on(d) && d.action != a.action) {
                        if self.inputs(b.action, device).contains(&input) {
                            out.push(Conflict {
                                action: a.action,
                                device,
                                input: input.into(),
                                with: Some(b.action),
                            });
                        }
                    }
                    if device == Device::Key && a.in_flight && GOD_KEYS.contains(&input) {
                        out.push(Conflict {
                            action: a.action,
                            device,
                            input: input.into(),
                            with: None,
                        });
                    }
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

    /// A captured key or button for `a` (spec 3.2.5): it becomes `a`'s
    /// only input on that device. **Taken by another action** (on this
    /// platform), it is moved, never doubled: an action that had it as
    /// its second key just loses it; an action left without any input
    /// gets `a`'s old one (a swap; unbound if `a` had none). A flight key
    /// of unicorn mode stays a marked conflict (the riding keys are off
    /// in flight). Values back at their default leave the file.
    pub fn assign(&self, a: Action, device: Device, input: &str, desktop: bool) -> Assigned {
        let shown = |v: &str| match device {
            Device::Key => key_label(v),
            Device::Pad => pad_label(v).to_string(),
        };
        if self.inputs(a, device) == [input] {
            return Assigned {
                changes: Vec::new(),
                note: format!("{}: {} (unchanged)", a.def().label, shown(input)),
            };
        }
        let mut next = self.clone();
        let old: Vec<&str> = self
            .inputs(a, device)
            .into_iter()
            .filter(|k| *k != input)
            .collect();
        let mut notes = Vec::new();
        let mut touched = vec![a];
        next.set(a, device, input);
        for o in ACTIONS
            .iter()
            .filter(|d| (desktop || !d.desktop_only) && d.action != a)
        {
            let theirs = self.inputs(o.action, device);
            if !theirs.contains(&input) {
                continue;
            }
            let rest: Vec<&str> = theirs.into_iter().filter(|k| *k != input).collect();
            let value = if !rest.is_empty() {
                let v = rest.join(",");
                notes.push(format!("{} keeps {}", o.label, shown(&v)));
                v
            } else if !old.is_empty() {
                let v = old.join(",");
                notes.push(format!("{} now has {}", o.label, shown(&v)));
                v
            } else {
                notes.push(format!("{} is now unbound", o.label));
                "none".into()
            };
            next.set(o.action, device, &value);
            touched.push(o.action);
        }
        let default = Self::default();
        let changes = touched
            .iter()
            .map(|x| {
                let setting = match device {
                    Device::Key => x.key_setting(),
                    Device::Pad => x.pad_setting(),
                };
                let v = next.get(*x, device);
                (
                    setting,
                    (v != default.get(*x, device)).then(|| v.to_string()),
                )
            })
            .collect();
        let mut note = format!("{}: {}", a.def().label, shown(input));
        if !notes.is_empty() {
            note.push_str(&format!(
                " ({} was taken: {})",
                shown(input),
                notes.join(", ")
            ));
        }
        if let Some(c) = next
            .conflict_of(a, device, desktop)
            .filter(|c| c.with.is_none())
        {
            note.push_str(&format!(" - conflict: {}", c.describe()));
        }
        Assigned { changes, note }
    }

    /// Whether `a`'s key fires: `down(name)` says whether the key `name`
    /// (without Shift) is down / went down this frame, `shift` whether
    /// Shift is held. `Shift+X` needs Shift; a plain `X` does not fire
    /// with Shift when another action is on `Shift+X`.
    pub fn key_fires(&self, a: Action, down: impl Fn(&str) -> bool, shift: bool) -> bool {
        self.keys_of(a).any(|k| match k.strip_prefix("Shift+") {
            Some(base) => shift && down(base),
            None => {
                down(k)
                    && !(shift
                        && self
                            .keys
                            .iter()
                            .flat_map(|o| key_list(o))
                            .any(|o| o.strip_prefix("Shift+") == Some(k)))
            }
        })
    }

    /// Every binding setting (`keys.*` and `pad.*` of every action).
    pub fn setting_keys() -> Vec<String> {
        ACTIONS
            .iter()
            .flat_map(|d| [d.action.key_setting(), d.action.pad_setting()])
            .collect()
    }

    /// The HUD's ride key lines with these keys (spec 3.2): a route ride,
    /// a free ride, a plain ride and the junction fan's hint.
    pub fn hud_lines(&self) -> HudKeys {
        let k = |a: Action| {
            let keys: Vec<String> = self.keys_of(a).map(hud_key).collect();
            if keys.is_empty() {
                "-".into()
            } else {
                keys.join("/")
            }
        };
        let base = format!(
            "{}: faster, {}: stop, {}/{}: steer",
            k(Action::Faster),
            k(Action::Stop),
            k(Action::SteerLeft),
            k(Action::SteerRight)
        );
        let exits = format!("{} {}: exit", k(Action::ExitLeft), k(Action::ExitRight));
        HudKeys {
            route: format!(
                "{base}, {exits} (another than the route's leaves it), {}: rejoin the route, {}: U-turn, {}: autodrive",
                k(Action::Rejoin),
                k(Action::UTurn),
                k(Action::Autodrive)
            ),
            free: format!(
                "{base}, {exits}, {}: U-turn, {}: autodrive, {}: wild",
                k(Action::UTurn),
                k(Action::Autodrive),
                k(Action::Wild)
            ),
            plain: format!(
                "{base}, {}: autodrive, {}: wild",
                k(Action::Autodrive),
                k(Action::Wild)
            ),
            junction: format!(
                "{}  < exit >  {}",
                k(Action::ExitLeft),
                k(Action::ExitRight)
            ),
        }
    }
}

/// The HUD's key lines ([`Bindings::hud_lines`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HudKeys {
    pub route: String,
    pub free: String,
    pub plain: String,
    /// Under the junction fan: "Del/Q  < exit >  PgDn/E".
    pub junction: String,
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
        // Spec 3.2 Q6: the map's zoom on D-pad down; "stop" keeps the
        // left stick and the left trigger.
        assert_eq!(b.pad(Action::MapZoom), "DPadDown");
        assert_eq!(b.pad(Action::Stop), "none");
        assert_eq!(b.pad(Action::Faster), "DPadUp");
        assert!(b.is_default());
        assert_eq!(b.conflicts(true), []);
        assert_eq!(b.conflicts(false), []);
        // Every default is a choice the settings accept.
        for d in &ACTIONS {
            assert_eq!(parse_keys(d.key).as_deref(), Ok(d.key), "{}", d.key);
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
            b.conflict_of(Action::Camera, Device::Key, true)
                .unwrap()
                .with,
            Some(Action::Video)
        );
        assert_eq!(
            b.conflict_of(Action::Video, Device::Key, true)
                .unwrap()
                .describe(),
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
        // The browser flies too (Phase 13 G5).
        assert!(b.conflict_of(Action::Camera, Device::Key, false).is_some());
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
        // Reset: every binding back, and out of the file.
        for k in Bindings::setting_keys() {
            s.unset(&k).unwrap();
        }
        assert!(Bindings::from_settings(&s).is_default());
        assert_eq!(s.to_toml().trim(), "", "nothing saved: {}", s.to_toml());
    }

    /// Spec 3.2.5 (2026-11-21): Delete / Page Down choose the junction's
    /// exit to the left / right, Q / E stay as second keys; ← / → only
    /// steer; Page Up / Page Down no longer change the power.
    #[test]
    fn the_exits_are_on_delete_and_page_down_with_q_and_e() {
        let b = Bindings::default();
        assert_eq!(b.key(Action::ExitLeft), "Delete,Q");
        assert_eq!(b.key(Action::ExitRight), "PageDown,E");
        assert_eq!(key_label(b.key(Action::ExitLeft)), "Delete / Q");
        for (k, a) in [
            ("Delete", Action::ExitLeft),
            ("Q", Action::ExitLeft),
            ("PageDown", Action::ExitRight),
            ("E", Action::ExitRight),
        ] {
            assert!(b.key_fires(a, |n| n == k, false), "{k}");
        }
        for k in ["Left", "Right", "PageUp"] {
            assert!(!b.key_fires(Action::ExitLeft, |n| n == k, false), "{k}");
            assert!(!b.key_fires(Action::ExitRight, |n| n == k, false), "{k}");
        }
        assert!(b.key_fires(Action::SteerLeft, |n| n == "Left", false));
        // No action is on Page Up; Page Down is only the exit.
        for d in &ACTIONS {
            assert!(!b.key_fires(d.action, |n| n == "PageUp", false), "{}", d.id);
            assert_eq!(
                b.key_fires(d.action, |n| n == "PageDown", false),
                d.action == Action::ExitRight,
                "{}",
                d.id
            );
        }
        // The gamepad keeps its D-pad for the exits; the stick steers.
        assert_eq!(b.pad(Action::ExitLeft), "DPadLeft");
        assert_eq!(b.pad(Action::SteerLeft), "none");
        let h = b.hud_lines();
        assert!(h.free.contains("Del/Q PgDn/E: exit"), "{}", h.free);
        assert!(h.free.contains("left/right: steer"), "{}", h.free);
        assert_eq!(h.junction, "Del/Q  < exit >  PgDn/E");
    }

    #[test]
    fn key_lists_parse_and_show() {
        assert_eq!(parse_keys("delete, q").as_deref(), Ok("Delete,Q"));
        assert_eq!(parse_keys("Q,Q").as_deref(), Ok("Q"));
        assert_eq!(parse_keys("NONE").as_deref(), Ok("none"));
        assert!(parse_keys("A,B,C").is_err(), "at most two");
        assert!(parse_keys("Escape").is_err());
        assert!(parse_keys("A,").is_err());
        assert_eq!(key_label("none"), "-");
        // A file of an older version: the retired power keys and the old
        // exit defaults are read as not set.
        let (s, warnings) = Settings::parse(
            "[keys]\npower_up = \"PageUp\"\nexit_left = \"Q\"\nexit_right = \"X\"\n[pad]\npower_down = \"none\"\n",
        )
        .unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        let b = Bindings::from_settings(&s);
        assert_eq!(b.key(Action::ExitLeft), "Delete,Q");
        assert_eq!(b.key(Action::ExitRight), "X", "a choice of its own stays");
        assert!(!s.to_toml().contains("power"), "{}", s.to_toml());
    }

    fn change(key: &str, value: Option<&str>) -> (String, Option<String>) {
        (key.to_string(), value.map(str::to_string))
    }

    /// The capture's conflict rule (spec 3.2.5): a key taken by another
    /// action moves; an action left without input gets the old one.
    #[test]
    fn a_captured_key_moves_from_another_action_by_a_swap() {
        let b = Bindings::default();
        // W is wild mode's only key: autodrive takes it, wild gets A.
        let r = b.assign(Action::Autodrive, Device::Key, "W", true);
        assert_eq!(
            r.changes,
            vec![
                change("keys.autodrive", Some("W")),
                change("keys.wild", Some("A"))
            ]
        );
        assert_eq!(r.note, "Autodrive: W (W was taken: Wild mode now has A)");
        // Q is the exit's second key: the exit keeps Delete, the camera
        // takes Q.
        let r = b.assign(Action::Camera, Device::Key, "Q", true);
        assert_eq!(
            r.changes,
            vec![
                change("keys.camera", Some("Q")),
                change("keys.exit_left", Some("Delete"))
            ]
        );
        assert!(
            r.note.contains("Junction: exit left keeps Delete"),
            "{}",
            r.note
        );
        // A key nobody has: only the action; back to its default it
        // leaves the file.
        let r = b.assign(Action::UTurn, Device::Key, "F5", true);
        assert_eq!(r.changes, vec![change("keys.u_turn", Some("F5"))]);
        let mut moved = b.clone();
        moved.set(Action::UTurn, Device::Key, "F5");
        let r = moved.assign(Action::UTurn, Device::Key, "U", true);
        assert_eq!(r.changes, vec![change("keys.u_turn", None)]);
        // Unchanged.
        assert!(
            b.assign(Action::Wild, Device::Key, "W", true)
                .changes
                .is_empty()
        );
        // A pad button: South is autodrive's; wild mode takes it and
        // autodrive gets West.
        let r = b.assign(Action::Wild, Device::Pad, "South", true);
        assert_eq!(
            r.changes,
            vec![
                change("pad.wild", Some("South")),
                change("pad.autodrive", Some("West"))
            ]
        );
        // Stop has no button: autodrive's South moves and autodrive is
        // unbound.
        let r = b.assign(Action::Stop, Device::Pad, "South", true);
        assert_eq!(
            r.changes,
            vec![
                change("pad.stop", Some("South")),
                change("pad.autodrive", Some("none"))
            ]
        );
        assert!(r.note.contains("Autodrive is now unbound"), "{}", r.note);
        // Applied, no conflict is left.
        let mut s = Settings::default();
        for (k, v) in b.assign(Action::Autodrive, Device::Key, "W", true).changes {
            match v {
                Some(v) => s.set(&k, &v).map(|_| ()),
                None => s.unset(&k),
            }
            .unwrap();
        }
        assert_eq!(Bindings::from_settings(&s).conflicts(true), []);
        // A desktop-only key in the browser is not taken from it.
        let r = b.assign(Action::Camera, Device::Key, "V", false);
        assert_eq!(r.changes.len(), 1, "{r:?}");
        // A flight key of unicorn mode stays a marked conflict.
        let r = b.assign(Action::Camera, Device::Key, "L", true);
        assert!(
            r.note.contains("conflict: L: Camera and unicorn"),
            "{}",
            r.note
        );
    }

    #[test]
    fn stepping_through_the_choices_wraps() {
        assert_eq!(step_choice(KEY_CHOICES, "A", 1), "B");
        assert_eq!(step_choice(KEY_CHOICES, "none", -1), "Shift+Z");
        assert_eq!(step_choice(PAD_CHOICES, "DPadRight", 1), "none");
    }
}
