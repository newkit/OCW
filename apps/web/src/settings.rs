//! The browser app's settings (spec §3.2.1, §14 Phase 10 B5 / B6): the
//! desktop's registry (`ocw_client::settings`: the same keys, defaults,
//! ranges and apply commands) and the shared resource and commands
//! (`ocw_ui::settings`: `set KEY VALUE [--temp]`, `get [KEY]`), the text
//! kept in the page's `localStorage` (key [`STORAGE_KEY`], the same TOML
//! as `settings.toml`) instead of a file. Every setting the browser has
//! takes effect (`ocw_client::settings::applies`): the live ones through
//! their console lines at the start and on every change, the ones read at
//! the start (`ride.bots`, `trainer.virtual_power`, `ride.record`,
//! `controls.autodrive`) where the page's query does not give them
//! ([`apply_to_config`]).

use bevy::prelude::*;
use ocw_client::settings::Value;
use ocw_render::console::ConsoleState;
use ocw_ui::Platform;
pub use ocw_ui::settings::{TextStore, UserSettings as WebSettings};

use crate::session::SessionConfig;

/// The `localStorage` key of the settings text.
pub const STORAGE_KEY: &str = "ocw-settings";

/// The page's `localStorage`.
#[cfg(target_arch = "wasm32")]
pub struct LocalStorage;

#[cfg(target_arch = "wasm32")]
impl TextStore for LocalStorage {
    fn load(&self) -> Result<Option<String>, String> {
        let storage = web_sys::window()
            .and_then(|w| w.local_storage().ok().flatten())
            .ok_or("no localStorage on this page")?;
        storage
            .get_item(STORAGE_KEY)
            .map_err(|_| "localStorage is blocked".to_string())
    }
    fn save(&mut self, text: &str) -> Result<(), String> {
        let s = web_sys::window()
            .and_then(|w| w.local_storage().ok().flatten())
            .ok_or("no localStorage on this page")?;
        s.set_item(STORAGE_KEY, text)
            .map_err(|_| "localStorage is full or blocked".to_string())
    }
    fn describe(&self) -> String {
        format!("saved in this browser (localStorage {STORAGE_KEY:?})")
    }
}

/// The `localStorage` key of the training history (spec §14 Phase 7; the
/// browser's one anonymous rider).
pub const HISTORY_KEY: &str = "ocw-history";

/// A text under `key` in the page's `localStorage`.
#[cfg(target_arch = "wasm32")]
pub struct LocalKey(pub &'static str);

#[cfg(target_arch = "wasm32")]
impl TextStore for LocalKey {
    fn load(&self) -> Result<Option<String>, String> {
        let storage = web_sys::window()
            .and_then(|w| w.local_storage().ok().flatten())
            .ok_or("no localStorage on this page")?;
        storage
            .get_item(self.0)
            .map_err(|_| "localStorage is blocked".to_string())
    }
    fn save(&mut self, text: &str) -> Result<(), String> {
        let s = web_sys::window()
            .and_then(|w| w.local_storage().ok().flatten())
            .ok_or("no localStorage on this page")?;
        s.set_item(self.0, text)
            .map_err(|_| "localStorage is full or blocked".to_string())
    }
    fn describe(&self) -> String {
        format!("saved in this browser (localStorage {:?})", self.0)
    }
}

/// The training history's store: `localStorage` in the browser, none
/// natively (tests).
pub fn history_store() -> Option<Box<dyn TextStore>> {
    #[cfg(target_arch = "wasm32")]
    return Some(Box::new(LocalKey(HISTORY_KEY)));
    #[cfg(not(target_arch = "wasm32"))]
    None
}

/// Makes the store when the app starts (`localStorage` on wasm32).
#[derive(Resource)]
pub struct MakeStore(pub Box<dyn Fn() -> Option<Box<dyn TextStore>> + Send + Sync>);

/// The page's store: `localStorage` in the browser, none natively.
pub fn default_store() -> Option<Box<dyn TextStore>> {
    #[cfg(target_arch = "wasm32")]
    return Some(Box::new(LocalStorage));
    #[cfg(not(target_arch = "wasm32"))]
    None
}

/// The settings read at the start, where the page's query does not give
/// the option (`query_keys`: the keys it has): bots, the virtual power,
/// recording the ride, autodrive.
pub fn apply_to_config(settings: &WebSettings, config: &mut SessionConfig, query_keys: &[&str]) {
    let given = |k: &str| query_keys.contains(&k);
    if !given("bots")
        && let Value::Int(n) = settings.value("ride.bots")
        && settings.settings.file_value("ride.bots").is_some()
    {
        config.bots = n.clamp(0, 50) as u32;
    }
    if !given("power")
        && let Value::Int(w) = settings.value("trainer.virtual_power")
        && settings
            .settings
            .file_value("trainer.virtual_power")
            .is_some()
    {
        config.power_w = w as f64;
    }
    config.record = settings.on("ride.record");
    config.autodrive = settings.on("controls.autodrive");
}

/// The keys of a page query (`?a=1&b` → a, b).
pub fn query_keys(query: &str) -> Vec<&str> {
    query
        .trim_start_matches('?')
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| p.split_once('=').map_or(p, |(k, _)| k))
        .collect()
}

pub struct WebSettingsPlugin;

impl Plugin for WebSettingsPlugin {
    fn build(&self, app: &mut App) {
        if !app.world().contains_resource::<WebSettings>() {
            let store = match app.world().get_resource::<MakeStore>() {
                Some(m) => (m.0)(),
                None => default_store(),
            };
            let (settings, warnings) = WebSettings::load(store);
            app.insert_resource(settings)
                .insert_resource(Warnings(warnings));
        }
        app.add_plugins(ocw_ui::settings::SettingsPlugin)
            .add_systems(Startup, startup);
    }
}

/// Problems found while loading, printed to the console at the start.
#[derive(Resource, Default)]
pub struct Warnings(pub Vec<String>);

/// The saved values' apply commands and the load warnings.
fn startup(
    settings: Res<WebSettings>,
    warnings: Option<Res<Warnings>>,
    mut console: ResMut<ConsoleState>,
) {
    for w in warnings.iter().flat_map(|w| w.0.iter()) {
        warn!("settings: {w}");
        console.print(format!("settings: {w}"));
    }
    for line in settings.startup_commands(Platform::Browser) {
        console.queue(line);
    }
}

#[cfg(test)]
pub mod memory {
    use std::sync::{Arc, Mutex};

    use super::TextStore;

    /// A store in memory (the tests' `localStorage`).
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
    use ocw_client::voice::SettingsStore;

    #[test]
    fn saved_text_round_trips_and_applies_at_the_start() {
        let mem = MemoryStore::default();
        let (mut ws, w) = WebSettings::load(Some(Box::new(mem.clone())));
        assert!(w.is_empty());
        assert!(
            ws.startup_commands(Platform::Browser).is_empty(),
            "all defaults"
        );
        ws.settings.set("audio.master", "40").unwrap();
        ws.settings.set("voice.mic_muted", "on").unwrap();
        ws.settings.set("display.units", "imperial").unwrap();
        SettingsStore::save(&mut ws).unwrap();
        let text = mem.load().unwrap().unwrap();
        assert!(
            text.contains("[audio]") && text.contains("master = 40"),
            "{text}"
        );
        let (again, _) = WebSettings::load(Some(Box::new(mem.clone())));
        let cmds = again.startup_commands(Platform::Browser);
        assert!(
            cmds.contains(&"audio.volume master 40".to_string()),
            "{cmds:?}"
        );
        assert!(
            cmds.iter().any(|c| c.starts_with("voice.apply")),
            "{cmds:?}"
        );
        // Since B6 every setting the browser has applies.
        assert!(cmds.contains(&"hud.units imperial".to_string()), "{cmds:?}");
    }

    #[test]
    fn a_broken_text_keeps_the_defaults_and_is_not_overwritten() {
        let mem = MemoryStore::default();
        *mem.0.lock().unwrap() = Some("not toml [".into());
        let (mut ws, w) = WebSettings::load(Some(Box::new(mem.clone())));
        assert_eq!(w.len(), 1);
        assert!(SettingsStore::save(&mut ws).is_err());
        assert_eq!(mem.load().unwrap().as_deref(), Some("not toml ["));
    }

    #[test]
    fn settings_read_at_the_start_fill_what_the_query_leaves_out() {
        let mem = MemoryStore::default();
        let (mut ws, _) = WebSettings::load(Some(Box::new(mem)));
        let mut c = SessionConfig::default();
        apply_to_config(&ws, &mut c, &[]);
        assert_eq!((c.bots, c.power_w), (5, 200.0), "the page's defaults");
        assert!(c.record && c.autodrive);
        ws.settings.set("ride.bots", "2").unwrap();
        ws.settings.set("trainer.virtual_power", "150").unwrap();
        ws.settings.set("ride.record", "off").unwrap();
        ws.settings.set("controls.autodrive", "off").unwrap();
        let mut c = SessionConfig::default();
        apply_to_config(&ws, &mut c, &query_keys("?route=loop"));
        assert_eq!((c.bots, c.power_w), (2, 150.0));
        assert!(!c.record && !c.autodrive);
        // The query wins.
        let mut c = SessionConfig {
            bots: 7,
            ..SessionConfig::default()
        };
        apply_to_config(&ws, &mut c, &query_keys("bots=7&power"));
        assert_eq!((c.bots, c.power_w), (7, 200.0));
        assert_eq!(query_keys("?a=1&b&&c=x=y"), ["a", "b", "c"]);
    }
}
