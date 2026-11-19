//! The settings of both apps (spec §3.2.1; §14 Phase 10, B5 / B6): the
//! registry's values (`ocw_client::settings`) and where their text is
//! kept – `ocw.toml [settings]` on the desktop (its `SlotStore`), the page's
//! `localStorage` in the browser (the same TOML) – and the console
//! commands `set KEY VALUE [--temp]` and `get [KEY]`. A live setting
//! applies at once through its console line; the browser keeps the
//! settings it does not have (`ocw_client::settings::applies`) without
//! applying them.

use bevy::prelude::*;
use clap::{Arg, ArgAction, Command};
use ocw_client::settings::{Kind, SETTINGS, Scope, Settings, Value, applies, def};
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, ConsoleState, Permission};

use crate::{Platform, UiPlatform};

/// Where the settings' text is kept.
pub trait TextStore: Send + Sync {
    /// The saved text; `Ok(None)`: nothing saved yet.
    fn load(&self) -> Result<Option<String>, String>;
    fn save(&mut self, text: &str) -> Result<(), String>;
    /// "file: ~/.config/ocw/settings.toml", "this browser (localStorage …)".
    fn describe(&self) -> String;
}

/// A file (`settings.toml`; the desktop).
#[cfg(not(target_arch = "wasm32"))]
pub struct FileStore(pub std::path::PathBuf);

#[cfg(not(target_arch = "wasm32"))]
impl TextStore for FileStore {
    fn load(&self) -> Result<Option<String>, String> {
        match std::fs::read_to_string(&self.0) {
            Ok(t) => Ok(Some(t)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("{}: {e}", self.0.display())),
        }
    }
    fn save(&mut self, text: &str) -> Result<(), String> {
        if let Some(dir) = self.0.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::write(&self.0, text).map_err(|e| format!("{}: {e}", self.0.display()))
    }
    fn describe(&self) -> String {
        format!("file: {}", self.0.display())
    }
}

/// The settings and their store (`None`: not saved, e.g. a broken file
/// that must not be overwritten, or a page without storage).
#[derive(Resource, Default)]
pub struct UserSettings {
    pub settings: Settings,
    pub store: Option<Box<dyn TextStore>>,
}

impl UserSettings {
    /// Reads the store; a broken text keeps the defaults and is not
    /// overwritten. The warnings for the console.
    pub fn load(store: Option<Box<dyn TextStore>>) -> (Self, Vec<String>) {
        let Some(store) = store else {
            return (Self::default(), Vec::new());
        };
        let text = match store.load() {
            Ok(t) => t,
            Err(e) => {
                return (
                    Self::default(),
                    vec![format!("{e}; using the default settings, not saving them")],
                );
            }
        };
        match text.as_deref().map(Settings::parse) {
            None => (
                Self {
                    settings: Settings::default(),
                    store: Some(store),
                },
                Vec::new(),
            ),
            Some(Ok((settings, warnings))) => (
                Self {
                    settings,
                    store: Some(store),
                },
                warnings,
            ),
            Some(Err(e)) => (
                Self::default(),
                vec![format!(
                    "the saved settings are broken ({e}; {}); using the defaults, not saving them",
                    store.describe()
                )],
            ),
        }
    }

    /// The value of `key`, which must be registered.
    pub fn value(&self, key: &str) -> Value {
        self.settings.get(key).expect("a registered setting")
    }

    /// Whether the switch `key` is on.
    pub fn on(&self, key: &str) -> bool {
        self.value(key).as_bool() == Some(true)
    }

    /// Saves the settings to the store.
    pub fn save(&mut self) -> Result<(), String> {
        let text = self.settings.to_toml();
        match &mut self.store {
            Some(s) => s.save(&text),
            None => Err("not saved (no configuration directory or storage)".into()),
        }
    }

    /// Where the settings are kept, for `get`.
    pub fn location(&self) -> Option<String> {
        self.store.as_ref().map(|s| s.describe())
    }

    /// The apply commands of the saved values `platform` has (at the
    /// start).
    pub fn startup_commands(&self, platform: Platform) -> Vec<String> {
        self.settings
            .startup_commands()
            .into_iter()
            .filter(|line| {
                SETTINGS.iter().any(|d| {
                    applies(d.key, platform)
                        && d.apply_command(&self.value(d.key)).as_deref() == Some(line.as_str())
                })
            })
            .collect()
    }
}

impl ocw_client::voice::SettingsStore for UserSettings {
    fn settings_mut(&mut self) -> &mut Settings {
        &mut self.settings
    }
    fn save(&mut self) -> Result<(), String> {
        UserSettings::save(self)
    }
}

/// Sets and saves `key` (the machines' choices persist); quietly not
/// saved without a store (tests).
pub fn save_setting(world: &mut World, key: &str, value: &str) {
    if let Some(mut us) = world.get_resource_mut::<UserSettings>()
        && us.settings.set(key, value).is_ok()
        && let Err(e) = us.save()
    {
        debug!("{key} not saved: {e}");
    }
}

/// Whether the switch `key` is on (the registry's default without the
/// settings).
pub fn setting_on(world: &World, key: &str) -> bool {
    match world.get_resource::<UserSettings>() {
        Some(us) => us.on(key),
        None => def(key).is_some_and(|d| d.default_value().as_bool() == Some(true)),
    }
}

/// `set` and `get`; the app inserts [`UserSettings`] (else the defaults,
/// not saved).
pub struct SettingsPlugin;

impl Plugin for SettingsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<UserSettings>();
        register_commands(app);
    }
}

fn platform(world: &World) -> Platform {
    world
        .get_resource::<UiPlatform>()
        .map_or(Platform::Desktop, |p| p.0)
}

fn register_commands(app: &mut App) {
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("set")
            .about("Change a setting and save it (ocw.toml; in the browser: this browser) (--temp: this session only)")
            .arg(
                Arg::new("temp")
                    .long("temp")
                    .action(ArgAction::SetTrue)
                    .help("Only for this session, not saved"),
            )
            .arg(Arg::new("key").required(true))
            .arg(Arg::new("value").required(true).num_args(1..)),
        |world, m| {
            let key = m.get_one::<String>("key").expect("required").clone();
            let value = m
                .get_many::<String>("value")
                .expect("required")
                .cloned()
                .collect::<Vec<_>>()
                .join(" ");
            let temp = m.get_flag("temp");
            let platform = platform(world);
            let mut us = world.resource_mut::<UserSettings>();
            let v = if temp {
                us.settings.set_temp(&key, &value)?
            } else {
                us.settings.set(&key, &value)?
            };
            let mut out = format!("{key} = {v}");
            if temp {
                out.push_str(" (this session)");
            } else if let Err(e) = us.save() {
                out.push_str(&format!(" ({e})"));
            }
            let d = def(&key).expect("set checked it");
            if !applies(d.key, platform) {
                out.push_str(" (kept; not in this app)");
                return Ok(out);
            }
            match d.scope {
                Scope::Live => {
                    if let Some(line) = d.apply_command(&v) {
                        world.resource_mut::<ConsoleState>().queue(line);
                    }
                }
                Scope::NextStart => out.push_str(match platform {
                    Platform::Desktop => ", from the next start (R restarts)",
                    Platform::Browser => ", from the next start (reload the page)",
                }),
            }
            Ok(out)
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("get")
            .about("Show a setting (with its range and description), or all settings")
            .arg(Arg::new("key")),
        |world, m| {
            let platform = platform(world);
            let us = world.resource::<UserSettings>();
            let s = &us.settings;
            if let Some(key) = m.get_one::<String>("key") {
                let d = def(key).ok_or_else(|| format!("unknown setting {key:?}"))?;
                return Ok(format!(
                    "{}\n  {} ({}; default {}; {})",
                    s.line(key)?,
                    d.description,
                    d.kind_text(),
                    d.default,
                    match d.scope {
                        _ if !applies(d.key, platform) => "not in this app",
                        Scope::Live => "applies at once",
                        Scope::NextStart => "applies from the next start",
                    }
                ));
            }
            let mut out: Vec<String> = SETTINGS
                .iter()
                .filter(|d| applies(d.key, platform))
                .map(|d| s.line(d.key).expect("registered"))
                .collect();
            let unknown = s.unknown_keys();
            if !unknown.is_empty() {
                out.push(format!("unknown keys (kept): {}", unknown.join(", ")));
            }
            out.push(
                us.location()
                    .unwrap_or_else(|| "file: none (not saved)".into()),
            );
            Ok(out.join("\n"))
        },
    ))
    // Tab completion (spec §2.9): the keys of this app, a key's values.
    .console_values("set", "key", setting_keys)
    .console_values("get", "key", setting_keys)
    .console_values("set", "value", |world, q| {
        let key = q.words[1..].iter().find(|w| !w.starts_with("--"));
        match key.and_then(|k| def(k)) {
            Some(d) => match d.kind {
                Kind::Bool => vec!["on".into(), "off".into()],
                Kind::Choice(c) => c.iter().map(|s| (*s).to_owned()).collect(),
                Kind::Int { .. } => {
                    let now = world
                        .get_resource::<UserSettings>()
                        .map(|s| s.value(d.key).to_string());
                    now.into_iter()
                        .chain([d.default.to_owned()])
                        .collect()
                }
                Kind::Text => Vec::new(),
            },
            None => Vec::new(),
        }
    });
}

/// The setting keys that apply in this app (Tab completion of `set`, `get`).
fn setting_keys(world: &mut World, _: &ocw_render::completion::ValueQuery) -> Vec<String> {
    let platform = platform(world);
    SETTINGS
        .iter()
        .filter(|d| applies(d.key, platform))
        .map(|d| d.key.to_owned())
        .collect()
}

/// A store in memory (the tests' `localStorage` or file; the browser
/// app's tests use it too).
pub mod memory {
    use std::sync::{Arc, Mutex};

    use super::TextStore;

    /// A store in memory (the tests' `localStorage` or file).
    #[derive(Clone, Default)]
    pub struct MemoryStore(pub Arc<Mutex<Option<String>>>);

    impl TextStore for MemoryStore {
        fn load(&self) -> Result<Option<String>, String> {
            Ok(self.0.lock().unwrap().clone())
        }
        fn save(&mut self, text: &str) -> Result<(), String> {
            *self.0.lock().unwrap() = Some(text.to_string());
            Ok(())
        }
        fn describe(&self) -> String {
            "memory".into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::memory::MemoryStore;
    use super::*;
    use ocw_render::console::ConsoleCorePlugin;

    #[test]
    fn saved_text_round_trips_and_applies_at_the_start() {
        let mem = MemoryStore::default();
        let (mut us, w) = UserSettings::load(Some(Box::new(mem.clone())));
        assert!(w.is_empty());
        assert!(
            us.startup_commands(Platform::Browser).is_empty(),
            "all defaults"
        );
        us.settings.set("audio.master", "40").unwrap();
        us.settings.set("display.units", "imperial").unwrap();
        us.settings.set("display.fullscreen", "on").unwrap();
        us.settings.set("controls.invert_y", "on").unwrap();
        us.save().unwrap();
        let text = mem.load().unwrap().unwrap();
        assert!(text.contains("master = 40"), "{text}");
        let (again, _) = UserSettings::load(Some(Box::new(mem.clone())));
        let cmds = again.startup_commands(Platform::Browser);
        assert!(
            cmds.contains(&"audio.volume master 40".to_string()),
            "{cmds:?}"
        );
        assert!(cmds.contains(&"hud.units imperial".to_string()), "{cmds:?}");
        // God mode's settings apply in both apps (spec §14 Phase 13 G5).
        assert!(cmds.contains(&"god.invert on".to_string()), "{cmds:?}");
        assert!(
            again
                .startup_commands(Platform::Desktop)
                .contains(&"god.invert on".to_string())
        );
        assert!(
            !cmds.iter().any(|c| c.contains("fullscreen")),
            "the browser has its own fullscreen: {cmds:?}"
        );
    }

    #[test]
    fn a_broken_text_keeps_the_defaults_and_is_not_overwritten() {
        let mem = MemoryStore::default();
        *mem.0.lock().unwrap() = Some("not toml [".into());
        let (mut us, w) = UserSettings::load(Some(Box::new(mem.clone())));
        assert_eq!(w.len(), 1);
        assert!(us.save().is_err());
        assert_eq!(mem.load().unwrap().as_deref(), Some("not toml ["));
    }

    fn app(platform: Platform, mem: &MemoryStore) -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ConsoleCorePlugin, SettingsPlugin))
            .insert_resource(UiPlatform(platform))
            .insert_resource(UserSettings::load(Some(Box::new(mem.clone()))).0);
        app.update();
        app
    }

    fn run(app: &mut App, line: &str) -> String {
        app.world_mut().resource_mut::<ConsoleState>().queue(line);
        app.update();
        app.world().resource::<ConsoleState>().log().join("\n")
    }

    #[test]
    fn set_and_get_in_the_browser() {
        let mem = MemoryStore::default();
        let mut app = app(Platform::Browser, &mem);
        let log = run(&mut app, "set display.fullscreen on");
        assert!(log.contains("not in this app"), "{log}");
        let log = run(&mut app, "set ride.bots 3");
        assert!(log.contains("reload the page"), "{log}");
        let log = run(&mut app, "get");
        let log = &log[log.rfind("> get").unwrap()..];
        assert!(log.contains("ride.bots = 3"), "{log}");
        assert!(!log.contains("display.fullscreen ="), "{log}");
        assert!(log.contains("memory"), "{log}");
        // Saved: a new page finds it.
        let again = UserSettings::load(Some(Box::new(mem.clone()))).0;
        assert_eq!(again.value("ride.bots"), Value::Int(3));
    }
}
