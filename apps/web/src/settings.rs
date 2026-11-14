//! The browser app's settings (spec §3.2.1, §14 Phase 10 B5): the
//! desktop's registry (`ocw_client::settings`: the same keys, defaults,
//! ranges and apply commands), its text kept in the page's `localStorage`
//! (key [`STORAGE_KEY`], the same TOML as `settings.toml`) instead of a
//! file. `set KEY VALUE [--temp]` and `get [KEY]` as on the desktop; for
//! now the audio and voice settings (`audio.*`, `voice.*`) take effect in
//! the browser – the others are kept and come with the menus (B6).

use bevy::prelude::*;
use clap::{Arg, ArgAction, Command};
use ocw_client::settings::{SETTINGS, Scope, Settings, def};
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, ConsoleState, Permission};

/// The `localStorage` key of the settings text.
pub const STORAGE_KEY: &str = "ocw-settings";

/// Whether a setting takes effect in the browser app yet.
pub fn in_browser(key: &str) -> bool {
    key.starts_with("audio.") || key.starts_with("voice.")
}

/// Where the text is kept: `localStorage` in the page, memory in tests.
pub trait TextStore: Send + Sync {
    fn load(&self) -> Option<String>;
    fn save(&mut self, text: &str) -> Result<(), String>;
}

/// The settings and their store.
#[derive(Resource, Default)]
pub struct WebSettings {
    pub settings: Settings,
    store: Option<Box<dyn TextStore>>,
}

impl WebSettings {
    /// Reads the store; a broken text keeps the defaults and is not
    /// overwritten (as on the desktop). The warnings for the console.
    pub fn load(store: Option<Box<dyn TextStore>>) -> (Self, Vec<String>) {
        let text = store.as_ref().and_then(|s| s.load());
        match text.as_deref().map(Settings::parse) {
            None => (
                Self {
                    settings: Settings::default(),
                    store,
                },
                Vec::new(),
            ),
            Some(Ok((settings, warnings))) => (Self { settings, store }, warnings),
            Some(Err(e)) => (
                Self {
                    settings: Settings::default(),
                    store: None,
                },
                vec![format!(
                    "the saved settings are broken ({e}); using the defaults, not saving them"
                )],
            ),
        }
    }

    /// The apply commands of the saved values the browser has (at the
    /// start).
    pub fn startup_commands(&self) -> Vec<String> {
        self.settings
            .startup_commands()
            .into_iter()
            .filter(|c| c.starts_with("audio.") || c.starts_with("voice."))
            .collect()
    }
}

impl ocw_client::voice::SettingsStore for WebSettings {
    fn settings_mut(&mut self) -> &mut Settings {
        &mut self.settings
    }
    fn save(&mut self) -> Result<(), String> {
        match &mut self.store {
            Some(s) => s.save(&self.settings.to_toml()),
            None => Err("no storage on this page".into()),
        }
    }
}

/// The page's `localStorage`.
#[cfg(target_arch = "wasm32")]
pub struct LocalStorage;

#[cfg(target_arch = "wasm32")]
impl TextStore for LocalStorage {
    fn load(&self) -> Option<String> {
        web_sys::window()?
            .local_storage()
            .ok()??
            .get_item(STORAGE_KEY)
            .ok()?
    }
    fn save(&mut self, text: &str) -> Result<(), String> {
        let s = web_sys::window()
            .and_then(|w| w.local_storage().ok().flatten())
            .ok_or("no localStorage on this page")?;
        s.set_item(STORAGE_KEY, text)
            .map_err(|_| "localStorage is full or blocked".to_string())
    }
}

/// Makes the store when the app starts (`localStorage` on wasm32).
#[derive(Resource)]
pub struct MakeStore(pub Box<dyn Fn() -> Option<Box<dyn TextStore>> + Send + Sync>);

pub struct WebSettingsPlugin;

impl Plugin for WebSettingsPlugin {
    fn build(&self, app: &mut App) {
        let store = match app.world().get_resource::<MakeStore>() {
            Some(m) => (m.0)(),
            None => default_store(),
        };
        let (settings, warnings) = WebSettings::load(store);
        app.insert_resource(settings)
            .insert_resource(Warnings(warnings))
            .add_systems(Startup, startup);
        register_commands(app);
    }
}

#[cfg(target_arch = "wasm32")]
fn default_store() -> Option<Box<dyn TextStore>> {
    Some(Box::new(LocalStorage))
}

#[cfg(not(target_arch = "wasm32"))]
fn default_store() -> Option<Box<dyn TextStore>> {
    None
}

#[derive(Resource)]
struct Warnings(Vec<String>);

/// The saved values' apply commands and the load warnings.
fn startup(settings: Res<WebSettings>, warnings: Res<Warnings>, mut console: ResMut<ConsoleState>) {
    for w in &warnings.0 {
        warn!("settings: {w}");
        console.print(format!("settings: {w}"));
    }
    for line in settings.startup_commands() {
        console.queue(line);
    }
}

fn register_commands(app: &mut App) {
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("set")
            .about("Change a setting and save it in this browser (--temp: this session only)")
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
            let mut ws = world.resource_mut::<WebSettings>();
            let v = if temp {
                ws.settings.set_temp(&key, &value)?
            } else {
                ws.settings.set(&key, &value)?
            };
            let mut out = format!("{key} = {v}");
            if temp {
                out.push_str(" (this session)");
            } else if let Err(e) = ocw_client::voice::SettingsStore::save(&mut *ws) {
                out.push_str(&format!(" (not saved: {e})"));
            }
            let d = def(&key).expect("set checked it");
            if !in_browser(&key) {
                out.push_str(" (kept; the browser app uses it with the menus, B6)");
            } else if d.scope == Scope::Live {
                if let Some(line) = d.apply_command(&v) {
                    world.resource_mut::<ConsoleState>().queue(line);
                }
            } else {
                out.push_str(", from the next start (reload the page)");
            }
            Ok(out)
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("get")
            .about(
                "Show a setting (with its range and description), or the audio and voice settings",
            )
            .arg(Arg::new("key")),
        |world, m| {
            let s = &world.resource::<WebSettings>().settings;
            if let Some(key) = m.get_one::<String>("key") {
                let d = def(key).ok_or_else(|| format!("unknown setting {key:?}"))?;
                return Ok(format!(
                    "{}\n  {} ({}; default {})",
                    s.line(key)?,
                    d.description,
                    d.kind_text(),
                    d.default
                ));
            }
            let mut out: Vec<String> = SETTINGS
                .iter()
                .filter(|d| in_browser(d.key))
                .map(|d| s.line(d.key).expect("registered"))
                .collect();
            out.push(format!(
                "saved in this browser (localStorage {STORAGE_KEY:?})"
            ));
            Ok(out.join("\n"))
        },
    ));
}

#[cfg(test)]
pub mod memory {
    use std::sync::{Arc, Mutex};

    use super::TextStore;

    /// A store in memory (the tests' `localStorage`).
    #[derive(Clone, Default)]
    pub struct MemoryStore(pub Arc<Mutex<Option<String>>>);

    impl TextStore for MemoryStore {
        fn load(&self) -> Option<String> {
            self.0.lock().unwrap().clone()
        }
        fn save(&mut self, text: &str) -> Result<(), String> {
            *self.0.lock().unwrap() = Some(text.to_string());
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::memory::MemoryStore;
    use super::*;
    use ocw_client::voice::SettingsStore;

    #[test]
    fn saved_text_round_trips_and_applies_at_the_start() {
        let mem = MemoryStore::default();
        let (mut ws, w) = WebSettings::load(Some(Box::new(mem.clone())));
        assert!(w.is_empty());
        assert!(ws.startup_commands().is_empty(), "all defaults");
        ws.settings.set("audio.master", "40").unwrap();
        ws.settings.set("voice.mic_muted", "on").unwrap();
        ws.settings.set("display.units", "imperial").unwrap();
        ws.save().unwrap();
        let text = mem.load().unwrap();
        assert!(
            text.contains("[audio]") && text.contains("master = 40"),
            "{text}"
        );
        let (again, _) = WebSettings::load(Some(Box::new(mem.clone())));
        let cmds = again.startup_commands();
        assert!(
            cmds.contains(&"audio.volume master 40".to_string()),
            "{cmds:?}"
        );
        assert!(
            cmds.iter().any(|c| c.starts_with("voice.apply")),
            "{cmds:?}"
        );
        assert!(
            !cmds.iter().any(|c| c.contains("units")),
            "not in the browser yet"
        );
    }

    #[test]
    fn a_broken_text_keeps_the_defaults_and_is_not_overwritten() {
        let mem = MemoryStore::default();
        *mem.0.lock().unwrap() = Some("not toml [".into());
        let (mut ws, w) = WebSettings::load(Some(Box::new(mem.clone())));
        assert_eq!(w.len(), 1);
        assert!(ws.save().is_err());
        assert_eq!(mem.load().as_deref(), Some("not toml ["));
        assert!(in_browser("voice.volume") && !in_browser("display.fps"));
    }
}
