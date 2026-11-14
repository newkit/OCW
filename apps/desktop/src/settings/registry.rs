//! The settings registry (spec §3.2.1): typed variables with a key, type,
//! default, range or choices, description and scope, stored in
//! `settings.toml`. Bevy-free; the console commands `set` / `get` and the
//! menu pages read and write it through [`Settings`].
//!
//! Layers, lowest first: the registry default < `settings.toml` <
//! `autoexec.cfg` (it runs `set` commands like the console) < the command
//! line and `set --temp`. The last two only hold for the session and are
//! never saved.
//!
//! The file has one section per key prefix (`display.fps` is `fps` in
//! `[display]`). Keys the registry does not know are kept as they are and
//! written back; a value of the wrong type or out of range is reported and
//! the default used (the file entry stays until the setting is set again).

use std::collections::BTreeMap;
use std::path::Path;

/// The type of a setting and its allowed values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    /// `on` / `off` (also true / false, yes / no, 1 / 0).
    Bool,
    /// A whole number in `min..=max`; menus change it by `step`.
    Int { min: i64, max: i64, step: i64 },
    /// One of these words.
    Choice(&'static [&'static str]),
    /// Free text (a device name), at most [`MAX_TEXT_LEN`] characters.
    Text,
}

/// Longest [`Kind::Text`] value.
pub const MAX_TEXT_LEN: usize = 200;

/// When a changed value takes effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// At once (the apply command runs on every change, and at the start).
    Live,
    /// At the next start of the app (`reset` / R restarts it).
    NextStart,
}

/// What the app does with a setting at the start (and, for
/// [`Scope::Live`], on every change).
#[derive(Debug, Clone, Copy)]
pub enum Apply {
    /// Run this console command line; `{}` is replaced with the value.
    Command(&'static str),
    /// The app reads it at the start: the command-line option of this name
    /// (`bots` for `--bots`), which overrides it for the session.
    Arg(&'static str),
    /// The app reads it at the start (no command-line option).
    App,
}

/// One registered setting.
#[derive(Debug, Clone, Copy)]
pub struct SettingDef {
    pub key: &'static str,
    pub kind: Kind,
    /// The default, written as in the console (`on`, `50`, `chase`).
    pub default: &'static str,
    pub description: &'static str,
    pub scope: Scope,
    pub apply: Apply,
}

/// A setting's value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Text(String),
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bool(b) => f.write_str(if *b { "on" } else { "off" }),
            Self::Int(i) => write!(f, "{i}"),
            Self::Text(s) => f.write_str(s),
        }
    }
}

impl Value {
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    fn to_toml(&self) -> toml::Value {
        match self {
            Self::Bool(b) => toml::Value::Boolean(*b),
            Self::Int(i) => toml::Value::Integer(*i),
            Self::Text(s) => toml::Value::String(s.clone()),
        }
    }
}

/// Where a setting's current value comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Default,
    File,
    /// `set --temp`: this session only.
    Temp,
    /// A command-line option: this session only.
    CommandLine,
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Default => "default",
            Self::File => "settings.toml",
            Self::Temp => "temporary",
            Self::CommandLine => "command line",
        })
    }
}

/// All settings, in menu order (spec §3.2.1, Settings pages).
pub const SETTINGS: &[SettingDef] = &[
    SettingDef {
        key: "controls.autodrive",
        kind: Kind::Bool,
        default: "on",
        description: "Autodrive on at the start of a ride (A toggles it while riding)",
        scope: Scope::NextStart,
        apply: Apply::App,
    },
    SettingDef {
        key: "controls.invert_y",
        kind: Kind::Bool,
        default: "off",
        description: "God mode: moving the mouse up looks down (spec §14 Phase 13B)",
        scope: Scope::Live,
        apply: Apply::Command("god.invert {}"),
    },
    SettingDef {
        key: "trainer.difficulty",
        kind: Kind::Int {
            min: 0,
            max: 100,
            step: 5,
        },
        default: "50",
        description: "Trainer difficulty in percent: how much of the road gradient SIM mode sends",
        scope: Scope::Live,
        apply: Apply::Command("trainer.difficulty {}"),
    },
    SettingDef {
        key: "trainer.virtual_power",
        kind: Kind::Int {
            min: 0,
            max: 2000,
            step: 10,
        },
        default: "200",
        description: "Power of the virtual trainer at the start, in watts",
        scope: Scope::NextStart,
        apply: Apply::Arg("power"),
    },
    SettingDef {
        key: "display.hud",
        kind: Kind::Bool,
        default: "on",
        description: "Show the HUD (power, speed, route, keys)",
        scope: Scope::Live,
        apply: Apply::Command("hud {}"),
    },
    SettingDef {
        key: "display.units",
        kind: Kind::Choice(&["metric", "imperial"]),
        default: "metric",
        description: "Units of speed and distance in the HUD (km/h and km, or mph and mi)",
        scope: Scope::Live,
        apply: Apply::Command("hud.units {}"),
    },
    SettingDef {
        key: "display.fps",
        kind: Kind::Bool,
        default: "off",
        description: "Show the FPS counter",
        scope: Scope::Live,
        apply: Apply::Command("fps {}"),
    },
    SettingDef {
        key: "display.camera",
        kind: Kind::Choice(&["chase", "side", "top"]),
        default: "chase",
        description: "Camera perspective",
        scope: Scope::Live,
        apply: Apply::Command("camera.mode {}"),
    },
    SettingDef {
        key: "display.night_brightness",
        kind: Kind::Int {
            min: 0,
            max: 50,
            step: 5,
        },
        default: "10",
        description: "Least light at night in % of daylight (0 = realistic darkness; spec §14 Phase 9)",
        scope: Scope::Live,
        apply: Apply::Command("sky.night {}"),
    },
    SettingDef {
        key: "display.sky",
        kind: Kind::Choice(&["atmosphere", "simple"]),
        default: "atmosphere",
        description: "Sky: physically based atmosphere, or one plain colour (for weak GPUs)",
        scope: Scope::Live,
        apply: Apply::Command("sky.mode {}"),
    },
    SettingDef {
        key: "display.bike_lights",
        kind: Kind::Choice(&["auto", "on", "off"]),
        default: "auto",
        description: "Bike lights of all riders: from dusk to dawn (auto), always or never",
        scope: Scope::Live,
        apply: Apply::Command("sky.lights {}"),
    },
    SettingDef {
        key: "display.orthophoto",
        kind: Kind::Bool,
        default: "on",
        description: "Ground: the real place's orthophoto near the roads, where the world package has it (spec §14 Phase 8 T7)",
        scope: Scope::Live,
        apply: Apply::Command("ortho {}"),
    },
    SettingDef {
        key: "display.unicorn_trail",
        kind: Kind::Bool,
        default: "on",
        description: "Effects: the rainbow trail behind the unicorns of god mode",
        scope: Scope::Live,
        apply: Apply::Command("god.trail {}"),
    },
    SettingDef {
        key: "display.fullscreen",
        kind: Kind::Bool,
        default: "off",
        description: "Borderless fullscreen on the current monitor (F11 toggles it while riding)",
        scope: Scope::NextStart,
        apply: Apply::Arg("fullscreen"),
    },
    SettingDef {
        key: "ride.bots",
        kind: Kind::Int {
            min: 0,
            max: 50,
            step: 1,
        },
        default: "10",
        description: "Number of test riders (bots) on the local server",
        scope: Scope::NextStart,
        apply: Apply::Arg("bots"),
    },
    SettingDef {
        key: "ride.random_start",
        kind: Kind::Bool,
        default: "on",
        description: "Start at a random point of the route or the road graph",
        scope: Scope::NextStart,
        apply: Apply::Arg("random_start"),
    },
    SettingDef {
        key: "ride.record",
        kind: Kind::Bool,
        default: "on",
        description: "Record rides and save them as FIT files",
        scope: Scope::NextStart,
        apply: Apply::Arg("record"),
    },
    // Audio (spec §3.3): volumes in percent, master × bus.
    SettingDef {
        key: "audio.master",
        kind: Kind::Int {
            min: 0,
            max: 100,
            step: 5,
        },
        default: "80",
        description: "Master volume in percent (scales all buses)",
        scope: Scope::Live,
        apply: Apply::Command("audio.volume master {}"),
    },
    SettingDef {
        key: "audio.effects",
        kind: Kind::Int {
            min: 0,
            max: 100,
            step: 5,
        },
        default: "80",
        description: "Volume of the game sounds (bike, wind, tyres, menu, event cues) in percent",
        scope: Scope::Live,
        apply: Apply::Command("audio.volume effects {}"),
    },
    SettingDef {
        key: "audio.others",
        kind: Kind::Int {
            min: 0,
            max: 100,
            step: 5,
        },
        default: "80",
        description: "Volume of the other riders' bikes in percent (a part of the effects: master × effects × others)",
        scope: Scope::Live,
        apply: Apply::Command("audio.volume others {}"),
    },
    SettingDef {
        key: "audio.music",
        kind: Kind::Int {
            min: 0,
            max: 100,
            step: 5,
        },
        default: "60",
        description: "Music volume in percent (no music yet: not on the Audio page until there is)",
        scope: Scope::Live,
        apply: Apply::Command("audio.volume music {}"),
    },
    SettingDef {
        key: "audio.voice",
        kind: Kind::Int {
            min: 0,
            max: 100,
            step: 5,
        },
        default: "100",
        description: "Voice chat volume in percent (voice chat comes in Phase 6)",
        scope: Scope::Live,
        apply: Apply::Command("audio.volume voice {}"),
    },
    SettingDef {
        key: "audio.mute",
        kind: Kind::Bool,
        default: "off",
        description: "Mute all sound",
        scope: Scope::Live,
        apply: Apply::Command("audio.mute {}"),
    },
    SettingDef {
        key: "audio.output",
        kind: Kind::Text,
        default: "default",
        description: "Output device by name (audio.devices lists them), or default; a missing device falls back to the default",
        scope: Scope::Live,
        apply: Apply::Command("audio.output \"{}\""),
    },
    // Voice chat (spec §2.4, §7, M8): always on like a call; every change
    // is handed to the voice controller by `voice.apply`.
    SettingDef {
        key: "voice.enabled",
        kind: Kind::Bool,
        default: "on",
        description: "Voice chat: join the voice room automatically on shared rides",
        scope: Scope::Live,
        apply: Apply::Command("voice.apply enabled {}"),
    },
    SettingDef {
        key: "voice.mic_muted",
        kind: Kind::Bool,
        default: "off",
        description: "Own microphone muted (M, gamepad Select)",
        scope: Scope::Live,
        apply: Apply::Command("voice.apply mic_muted {}"),
    },
    SettingDef {
        key: "voice.speaker_muted",
        kind: Kind::Bool,
        default: "off",
        description: "Speaker muted: no voices (Shift+M, gamepad R3); game sounds stay",
        scope: Scope::Live,
        apply: Apply::Command("voice.apply speaker_muted {}"),
    },
    SettingDef {
        key: "voice.volume",
        kind: Kind::Int {
            min: 0,
            max: 200,
            step: 10,
        },
        default: "100",
        description: "Voice volume in percent",
        scope: Scope::Live,
        apply: Apply::Command("voice.apply volume {}"),
    },
    SettingDef {
        key: "voice.spatial",
        kind: Kind::Bool,
        default: "on",
        description: "Spatial voice: riders further away on the road sound quieter",
        scope: Scope::Live,
        apply: Apply::Command("voice.apply spatial {}"),
    },
    SettingDef {
        key: "voice.max_voices",
        kind: Kind::Int {
            min: 1,
            max: 20,
            step: 1,
        },
        default: "10",
        description: "Hear at most this many riders at once (the nearest)",
        scope: Scope::Live,
        apply: Apply::Command("voice.apply max_voices {}"),
    },
    SettingDef {
        key: "voice.input",
        kind: Kind::Text,
        default: "default",
        description: "Microphone by name (default: the system's)",
        scope: Scope::Live,
        apply: Apply::Command("voice.apply input {}"),
    },
    SettingDef {
        key: "voice.output",
        kind: Kind::Text,
        default: "default",
        description: "Speaker for voices by name (default: the system's)",
        scope: Scope::Live,
        apply: Apply::Command("voice.apply output {}"),
    },
];

/// The setting registered under `key`.
pub fn def(key: &str) -> Option<&'static SettingDef> {
    SETTINGS.iter().find(|d| d.key == key)
}

impl SettingDef {
    /// Parses a value as typed in the console.
    pub fn parse(&self, text: &str) -> Result<Value, String> {
        let text = text.trim();
        match self.kind {
            Kind::Bool => match text.to_ascii_lowercase().as_str() {
                "on" | "true" | "yes" | "1" => Ok(Value::Bool(true)),
                "off" | "false" | "no" | "0" => Ok(Value::Bool(false)),
                _ => Err(format!("{}: expected on or off, not {text:?}", self.key)),
            },
            Kind::Int { .. } => {
                let v = text
                    .parse::<i64>()
                    .map_err(|_| format!("{}: expected a whole number, not {text:?}", self.key))?;
                self.check(Value::Int(v))
            }
            Kind::Choice(choices) => choices
                .iter()
                .find(|c| c.eq_ignore_ascii_case(text))
                .map(|c| Value::Text((*c).to_string()))
                .ok_or_else(|| {
                    format!(
                        "{}: expected one of {}, not {text:?}",
                        self.key,
                        choices.join(", ")
                    )
                }),
            Kind::Text => {
                if text.is_empty() {
                    Err(format!("{}: expected a name", self.key))
                } else if text.chars().count() > MAX_TEXT_LEN || text.contains(['\n', '"']) {
                    Err(format!(
                        "{}: at most {MAX_TEXT_LEN} characters on one line, without quotes",
                        self.key
                    ))
                } else {
                    Ok(Value::Text(text.to_string()))
                }
            }
        }
    }

    /// Converts a value from the settings file.
    fn value_from_toml(&self, v: &toml::Value) -> Result<Value, String> {
        match (self.kind, v) {
            (Kind::Bool, toml::Value::Boolean(b)) => Ok(Value::Bool(*b)),
            (Kind::Int { .. }, toml::Value::Integer(i)) => self.check(Value::Int(*i)),
            (Kind::Choice(_) | Kind::Text, toml::Value::String(s)) => self.parse(s),
            (_, other) => Err(format!(
                "{}: expected {}, not {other}",
                self.key,
                self.kind_text()
            )),
        }
    }

    fn check(&self, v: Value) -> Result<Value, String> {
        match (self.kind, &v) {
            (Kind::Int { min, max, .. }, Value::Int(i)) if !(min..=max).contains(i) => Err(
                format!("{}: {i} is out of range ({min} to {max})", self.key),
            ),
            _ => Ok(v),
        }
    }

    /// The default value.
    pub fn default_value(&self) -> Value {
        self.parse(self.default)
            .expect("registry defaults are valid (tested)")
    }

    /// "on / off", "0 to 100", "chase, side, top".
    pub fn kind_text(&self) -> String {
        match self.kind {
            Kind::Bool => "on / off".into(),
            Kind::Int { min, max, .. } => format!("{min} to {max}"),
            Kind::Choice(c) => c.join(", "),
            Kind::Text => "a name".into(),
        }
    }

    /// The value one step down (`dir` < 0) or up from `v`, for menus:
    /// toggles a switch, moves a number by its step (within the range),
    /// cycles through the choices; leaves text as it is.
    pub fn step(&self, v: &Value, dir: i32) -> Value {
        match (self.kind, v) {
            (Kind::Bool, Value::Bool(b)) => Value::Bool(!b),
            (Kind::Int { min, max, step }, Value::Int(i)) => {
                Value::Int((i + step * i64::from(dir.signum())).clamp(min, max))
            }
            (Kind::Choice(c), Value::Text(s)) => {
                let n = c.len() as i32;
                let at = c.iter().position(|x| x == s).unwrap_or(0) as i32;
                Value::Text(c[(at + dir.signum()).rem_euclid(n) as usize].to_string())
            }
            (Kind::Text, Value::Text(_)) => v.clone(),
            _ => self.default_value(),
        }
    }

    /// The console command line that applies `v`, if the setting has one.
    pub fn apply_command(&self, v: &Value) -> Option<String> {
        match self.apply {
            Apply::Command(template) => Some(template.replace("{}", &v.to_string())),
            Apply::Arg(_) | Apply::App => None,
        }
    }

    /// (section, name) in the file.
    fn file_key(&self) -> (&'static str, &'static str) {
        self.key.split_once('.').expect("keys are section.name")
    }
}

/// The settings of this app: the file's content and the session values.
#[derive(Debug, Clone, Default)]
pub struct Settings {
    /// `settings.toml` as read, with saved changes (unknown keys kept).
    doc: toml::Table,
    /// Session-only values (`set --temp`, command-line options).
    session: BTreeMap<&'static str, (Value, Source)>,
}

impl Settings {
    /// Reads settings from the file text. Returns them and the warnings
    /// (unknown keys, invalid values); a syntax error is an error.
    pub fn parse(text: &str) -> Result<(Self, Vec<String>), String> {
        let doc: toml::Table = toml::from_str(text).map_err(|e| e.to_string())?;
        let s = Self {
            doc,
            session: BTreeMap::new(),
        };
        let mut warnings = Vec::new();
        for key in s.unknown_keys() {
            warnings.push(format!("unknown setting {key} (kept in the file)"));
        }
        for d in SETTINGS {
            if let Some(Err(e)) = s.file_entry(d).map(|v| d.value_from_toml(v)) {
                warnings.push(format!("{e}; using the default {}", d.default));
            }
        }
        Ok((s, warnings))
    }

    /// Reads the file; a missing file means all defaults.
    pub fn load(path: &Path) -> Result<(Self, Vec<String>), String> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok((Self::default(), Vec::new())),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    /// The file's content (known and unknown keys).
    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(&self.doc).expect("a TOML table serializes")
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::write(path, self.to_toml()).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Keys in the file the registry does not know, as `section.name`.
    pub fn unknown_keys(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (section, v) in &self.doc {
            match v {
                toml::Value::Table(t) => {
                    for name in t.keys() {
                        let key = format!("{section}.{name}");
                        if def(&key).is_none() {
                            out.push(key);
                        }
                    }
                }
                _ => out.push(section.clone()),
            }
        }
        out
    }

    fn file_entry(&self, d: &SettingDef) -> Option<&toml::Value> {
        let (section, name) = d.file_key();
        self.doc.get(section)?.as_table()?.get(name)
    }

    /// The value from the file, if it is there and valid.
    pub fn file_value(&self, key: &str) -> Option<Value> {
        let d = def(key)?;
        self.file_entry(d).and_then(|v| d.value_from_toml(v).ok())
    }

    /// The current value and where it comes from.
    pub fn get_with_source(&self, key: &str) -> Result<(Value, Source), String> {
        let d = def(key).ok_or_else(|| unknown(key))?;
        if let Some((v, s)) = self.session.get(d.key) {
            return Ok((v.clone(), *s));
        }
        Ok(match self.file_value(key) {
            Some(v) => (v, Source::File),
            None => (d.default_value(), Source::Default),
        })
    }

    pub fn get(&self, key: &str) -> Result<Value, String> {
        self.get_with_source(key).map(|(v, _)| v)
    }

    /// Sets a value to be saved (`set KEY VALUE`); it replaces a session
    /// value of the key too.
    pub fn set(&mut self, key: &str, text: &str) -> Result<Value, String> {
        let d = def(key).ok_or_else(|| unknown(key))?;
        let v = d.parse(text)?;
        let (section, name) = d.file_key();
        let table = self
            .doc
            .entry(section)
            .or_insert_with(|| toml::Value::Table(toml::Table::new()));
        if !table.is_table() {
            // A top-level value with the section's name: replaced.
            *table = toml::Value::Table(toml::Table::new());
        }
        table
            .as_table_mut()
            .expect("a table")
            .insert(name.to_string(), v.to_toml());
        self.session.remove(d.key);
        Ok(v)
    }

    /// Sets a value for this session only (`set --temp`).
    pub fn set_temp(&mut self, key: &str, text: &str) -> Result<Value, String> {
        let d = def(key).ok_or_else(|| unknown(key))?;
        let v = d.parse(text)?;
        self.session.insert(d.key, (v.clone(), Source::Temp));
        Ok(v)
    }

    /// A command-line option overrides the setting for this session.
    pub fn set_command_line(&mut self, key: &str, v: Value) {
        if let Some(d) = def(key) {
            self.session.insert(d.key, (v, Source::CommandLine));
        }
    }

    /// The command lines that apply the settings at the start, in
    /// registry order (before `autoexec.cfg` and `--cmd`). Settings at
    /// their default are skipped: the app starts with the defaults anyway.
    pub fn startup_commands(&self) -> Vec<String> {
        SETTINGS
            .iter()
            .filter_map(|d| {
                let (v, source) = self.get_with_source(d.key).ok()?;
                if source == Source::Default {
                    return None;
                }
                d.apply_command(&v)
            })
            .collect()
    }

    /// `key = value (source)`.
    pub fn line(&self, key: &str) -> Result<String, String> {
        let (v, source) = self.get_with_source(key)?;
        Ok(format!("{key} = {v} ({source})"))
    }
}

fn unknown(key: &str) -> String {
    format!("unknown setting {key:?} (get lists them)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_is_valid() {
        let mut keys = std::collections::BTreeSet::new();
        for d in SETTINGS {
            assert!(keys.insert(d.key), "duplicate {}", d.key);
            let (section, name) = d.key.split_once('.').expect("section.name");
            assert!(!section.is_empty() && !name.is_empty() && !name.contains('.'));
            // Defaults parse and are in range; the step stays in range.
            let v = d.parse(d.default).unwrap_or_else(|e| panic!("{e}"));
            for dir in [-1, 1] {
                let stepped = d.step(&v, dir);
                assert!(d.parse(&stepped.to_string()).is_ok(), "{}", d.key);
            }
            assert!(!d.description.is_empty());
            if let Kind::Int { min, max, step } = d.kind {
                assert!(min < max && step > 0);
            }
            if let Apply::Command(c) = d.apply {
                assert!(c.contains("{}"), "{}", d.key);
            }
        }
    }

    #[test]
    fn values_parse_by_type_and_range() {
        let fps = def("display.fps").unwrap();
        assert_eq!(fps.parse("ON"), Ok(Value::Bool(true)));
        assert_eq!(fps.parse("0"), Ok(Value::Bool(false)));
        assert!(fps.parse("maybe").is_err());
        let d = def("trainer.difficulty").unwrap();
        assert_eq!(d.parse(" 80 "), Ok(Value::Int(80)));
        assert!(d.parse("101").unwrap_err().contains("out of range"));
        assert!(d.parse("-1").is_err());
        assert!(d.parse("12.5").is_err());
        let cam = def("display.camera").unwrap();
        assert_eq!(cam.parse("Top"), Ok(Value::Text("top".into())));
        assert!(cam.parse("bird").unwrap_err().contains("chase, side, top"));
    }

    #[test]
    fn text_settings_hold_device_names() {
        let d = def("voice.input").unwrap();
        assert_eq!(
            d.parse(" USB Headset "),
            Ok(Value::Text("USB Headset".into()))
        );
        assert!(d.parse("").is_err());
        assert!(d.parse("a\"b").is_err());
        assert!(d.parse(&"x".repeat(MAX_TEXT_LEN + 1)).is_err());
        let v = Value::Text("USB Headset".into());
        assert_eq!(d.step(&v, 1), v, "devices are stepped by the voice page");
        let (s, warnings) =
            Settings::parse("[voice]\ninput = \"USB Headset\"\nmic_muted = true\n").unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(s.get("voice.input"), Ok(v));
        assert_eq!(s.get("voice.mic_muted"), Ok(Value::Bool(true)));
        // First start: microphone and speaker on (not muted).
        let s = Settings::default();
        assert_eq!(s.get("voice.mic_muted"), Ok(Value::Bool(false)));
        assert_eq!(s.get("voice.speaker_muted"), Ok(Value::Bool(false)));
        assert_eq!(s.get("voice.enabled"), Ok(Value::Bool(true)));
    }

    #[test]
    fn steps_toggle_clamp_and_cycle() {
        let d = def("trainer.difficulty").unwrap();
        assert_eq!(d.step(&Value::Int(50), 1), Value::Int(55));
        assert_eq!(d.step(&Value::Int(100), 1), Value::Int(100));
        assert_eq!(d.step(&Value::Int(3), -1), Value::Int(0));
        let fps = def("display.fps").unwrap();
        assert_eq!(fps.step(&Value::Bool(false), -1), Value::Bool(true));
        let cam = def("display.camera").unwrap();
        let chase = Value::Text("chase".into());
        assert_eq!(cam.step(&chase, 1), Value::Text("side".into()));
        assert_eq!(cam.step(&chase, -1), Value::Text("top".into()));
    }

    #[test]
    fn toml_round_trip_keeps_unknown_keys() {
        let text = "\
top_level = 3

[display]
fps = true
hud = false
future_option = \"x\"

[sound]
master = 80
";
        let (mut s, warnings) = Settings::parse(text).unwrap();
        assert_eq!(s.get("display.fps"), Ok(Value::Bool(true)));
        assert_eq!(
            s.get_with_source("display.hud"),
            Ok((Value::Bool(false), Source::File))
        );
        assert_eq!(
            s.get_with_source("display.camera"),
            Ok((Value::Text("chase".into()), Source::Default))
        );
        let mut unknown = s.unknown_keys();
        unknown.sort();
        assert_eq!(
            unknown,
            ["display.future_option", "sound.master", "top_level"]
        );
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        s.set("trainer.difficulty", "70").unwrap();
        s.set("display.fps", "off").unwrap();
        let (again, warnings) = Settings::parse(&s.to_toml()).unwrap();
        assert_eq!(warnings.len(), 3);
        assert_eq!(again.get("trainer.difficulty"), Ok(Value::Int(70)));
        assert_eq!(again.get("display.fps"), Ok(Value::Bool(false)));
        assert_eq!(again.get("display.hud"), Ok(Value::Bool(false)));
        let text = again.to_toml();
        assert!(text.contains("future_option = \"x\"") && text.contains("master = 80"));
        assert!(text.contains("top_level = 3"), "{text}");
        assert!(text.contains("[trainer]\ndifficulty = 70"), "{text}");
    }

    #[test]
    fn invalid_values_fall_back_to_the_default_with_a_warning() {
        let text = "[trainer]\ndifficulty = 300\n[display]\ncamera = \"bird\"\nfps = \"yes please\"\nhud = 1\n";
        let (s, warnings) = Settings::parse(text).unwrap();
        assert_eq!(s.get("trainer.difficulty"), Ok(Value::Int(50)));
        assert_eq!(s.get("display.camera"), Ok(Value::Text("chase".into())));
        assert_eq!(s.get("display.fps"), Ok(Value::Bool(false)));
        assert_eq!(s.get("display.hud"), Ok(Value::Bool(true)));
        assert_eq!(warnings.len(), 4, "{warnings:?}");
        assert!(warnings[0].contains("out of range") && warnings[0].contains("default 50"));
        // The invalid entries stay in the file until set again.
        assert!(s.to_toml().contains("difficulty = 300"));
        // A broken file is an error (the caller keeps it, uses defaults).
        assert!(Settings::parse("[display\nfps = true").is_err());
    }

    #[test]
    fn precedence_default_file_temp_command_line() {
        let (mut s, _) = Settings::parse("[ride]\nbots = 2\n").unwrap();
        assert_eq!(
            s.line("ride.bots").unwrap(),
            "ride.bots = 2 (settings.toml)"
        );
        s.set_temp("ride.bots", "7").unwrap();
        assert_eq!(
            s.get_with_source("ride.bots"),
            Ok((Value::Int(7), Source::Temp))
        );
        // Temporary values are not saved.
        assert!(s.to_toml().contains("bots = 2"));
        s.set_command_line("ride.bots", Value::Int(9));
        assert_eq!(
            s.get_with_source("ride.bots"),
            Ok((Value::Int(9), Source::CommandLine))
        );
        assert!(!s.to_toml().contains('9'));
        // `set` saves and replaces the session value.
        s.set("ride.bots", "3").unwrap();
        assert_eq!(
            s.get_with_source("ride.bots"),
            Ok((Value::Int(3), Source::File))
        );
        assert!(s.set("ride.bots", "51").is_err());
        assert!(
            s.set("no.such", "1")
                .unwrap_err()
                .contains("unknown setting")
        );
        assert!(s.get("no.such").is_err());
    }

    #[test]
    fn startup_commands_apply_the_non_default_settings() {
        let (mut s, _) = Settings::parse(
            "[display]\nfps = true\ncamera = \"top\"\nunits = \"imperial\"\n[ride]\nbots = 1\n[controls]\nautodrive = false\n",
        )
        .unwrap();
        s.set_temp("trainer.difficulty", "80").unwrap();
        assert_eq!(
            s.startup_commands(),
            [
                "trainer.difficulty 80",
                "hud.units imperial",
                "fps on",
                "camera.mode top",
            ]
        );
        assert!(Settings::default().startup_commands().is_empty());
    }

    /// The Audio page's settings (M5): saved in `[audio]`, read back, and
    /// applied by their commands.
    #[test]
    fn audio_settings_round_trip() {
        let mut s = Settings::default();
        s.set("audio.master", "55").unwrap();
        s.set("audio.mute", "on").unwrap();
        s.set("audio.output", " USB Audio Device, Analog ").unwrap();
        assert!(s.set("audio.output", "say \"hi\"").is_err());
        assert!(s.set("audio.output", "  ").is_err());
        assert!(s.set("audio.voice", "101").is_err());
        let text = s.to_toml();
        assert!(text.contains("[audio]"), "{text}");
        let (again, warnings) = Settings::parse(&text).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(again.get("audio.master"), Ok(Value::Int(55)));
        assert_eq!(again.get("audio.mute"), Ok(Value::Bool(true)));
        assert_eq!(again.get("audio.effects"), Ok(Value::Int(80)));
        assert_eq!(again.get("audio.others"), Ok(Value::Int(80)));
        assert!(s.set("audio.others", "-5").is_err());
        let out = again.get("audio.output").unwrap();
        assert_eq!(out, Value::Text("USB Audio Device, Analog".into()));
        let d = def("audio.output").unwrap();
        assert_eq!(d.step(&out, 1), out, "text does not step");
        assert_eq!(
            again.startup_commands(),
            [
                "audio.volume master 55",
                "audio.mute on",
                "audio.output \"USB Audio Device, Analog\"",
            ]
        );
        // A number in the file where a name belongs: the default.
        let (bad, w) = Settings::parse("[audio]\noutput = 3\n").unwrap();
        assert_eq!(bad.get("audio.output"), Ok(Value::Text("default".into())));
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn load_and_save_files() {
        let dir = std::env::temp_dir().join(format!("ocw-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("nested/settings.toml");
        let (mut s, w) = Settings::load(&path).unwrap();
        assert!(w.is_empty());
        s.set("display.units", "imperial").unwrap();
        s.save(&path).unwrap();
        let (again, _) = Settings::load(&path).unwrap();
        assert_eq!(
            again.get("display.units"),
            Ok(Value::Text("imperial".into()))
        );
        std::fs::write(&path, "not toml [").unwrap();
        assert!(Settings::load(&path).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
