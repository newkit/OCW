//! Open Cycling World in the browser (spec §14 Phase 10, milestone B1).
//!
//! The same client as the desktop app, built for `wasm32-unknown-unknown`
//! and run by the browser on WebGPU: the shared renderer
//! ([`ocw_render::OcwRenderPlugins`]: world, riders, chase camera, HUD,
//! console, sky), the shared client loop ([`ocw_client`]), and the same
//! `GameServer` as the local server – driven by the frame loop
//! ([`ocw_server::local::FrameServer`]) because wasm has no threads. Power
//! comes from the virtual trainer (↑ / ↓; Web Bluetooth follows in B4).
//!
//! Desktop-only parts stay in `apps/desktop` (BLE, cpal audio, QUIC,
//! files, voice, captures, the menus' settings files); they move into the
//! shared crates behind web backends milestone by milestone (B2–B6), so
//! both apps stay one client (spec 10B, full parity).
//!
//! The page passes options in its query string (see [`SessionConfig`] and
//! [`parse_query`]): `?route=loop&bots=3&power=250&name=Ada`.

pub mod plugin;
pub mod session;

#[path = "../../desktop/src/build_id.rs"]
mod build_id;

use bevy::prelude::*;
use ocw_render::hud::{AppKeys, VersionLabel};
use ocw_render::{OcwRenderPlugins, RideView};
use ocw_world::Route;

pub use session::SessionConfig;

/// The id of the page's canvas (`web/index.html`).
pub const CANVAS_ID: &str = "#ocw-canvas";

/// The browser app's own keys in the HUD (no restart, video or screen
/// keys: the browser has its own fullscreen).
pub const WEB_KEYS: &str = "mouse: camera, C: camera reset | ` or F12: console";

/// This build's id: version, commit, profile, and "web".
pub fn build_id() -> String {
    format!(
        "{} web",
        build_id::build_id(
            env!("CARGO_PKG_VERSION"),
            env!("OCW_COMMIT"),
            cfg!(debug_assertions)
        )
    )
}

/// The ride options from the page's query string (`?a=1&b=2`, the `?`
/// optional): `route` (hills, loop), `bots` (0–50), `power` (W), `start`
/// (km), `name`. Unknown keys and bad values are reported and ignored.
pub fn parse_query(query: &str) -> (SessionConfig, Vec<String>) {
    let mut config = SessionConfig::default();
    let mut warnings = Vec::new();
    for pair in query.trim_start_matches('?').split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let value = value.replace('+', " ");
        let mut bad = || warnings.push(format!("{key}={value}: ignored"));
        match key {
            "route" => match value.as_str() {
                "hills" => config.route = Route::test_hills(),
                "loop" => config.route = Route::test_loop(),
                _ => bad(),
            },
            "bots" => match value.parse::<u32>() {
                Ok(n) if n <= 50 => config.bots = n,
                _ => bad(),
            },
            "power" => match value.parse::<f64>() {
                Ok(w) if (0.0..=session::MAX_POWER_W).contains(&w) => config.power_w = w,
                _ => bad(),
            },
            "start" => match value.parse::<f64>() {
                Ok(km) if km >= 0.0 && km.is_finite() => config.start_m = km * 1000.0,
                _ => bad(),
            },
            "name" if !value.trim().is_empty() => {
                config.name = value.trim().chars().take(32).collect()
            }
            _ => bad(),
        }
    }
    (config, warnings)
}

/// The browser app: Bevy with the canvas of the page, the shared renderer
/// and the in-browser ride session.
pub fn app(config: SessionConfig) -> App {
    let mut app = App::new();
    let route = config.route.clone();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Open Cycling World".into(),
                    canvas: Some(CANVAS_ID.into()),
                    // The canvas fills the page; the arrows and Page Up /
                    // Down ride instead of scrolling it.
                    fit_canvas_to_parent: true,
                    prevent_default_event_handling: true,
                    ..default()
                }),
                ..default()
            })
            // Everything the app loads is embedded: no `.meta` requests.
            .set(AssetPlugin {
                meta_check: bevy::asset::AssetMetaCheck::Never,
                ..default()
            })
            // No game audio in B1 (Web Audio comes with B5).
            .disable::<bevy::audio::AudioPlugin>(),
    )
    .insert_resource(Time::<Fixed>::from_hz(f64::from(ocw_core::SIM_TICK_HZ)))
    .insert_resource(RideView::new(route))
    .insert_resource(VersionLabel(build_id()))
    .insert_resource(AppKeys(WEB_KEYS.into()))
    .add_systems(Startup, || {
        info!("Open Cycling World {} starting", build_id())
    })
    .add_plugins(OcwRenderPlugins)
    .add_plugins(plugin::WebSessionPlugin { config });
    app
}

/// The entry point the wasm-bindgen glue calls when the module is loaded.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    let query = web_sys::window()
        .and_then(|w| w.location().search().ok())
        .unwrap_or_default();
    let (config, warnings) = parse_query(&query);
    let mut app = app(config);
    for w in warnings {
        warn!("page options: {w}");
    }
    app.run();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_options() {
        let (c, w) = parse_query("?route=loop&bots=3&power=250&start=1.5&name=Ada+L");
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(c.route.length(), Route::test_loop().length());
        assert_eq!((c.bots, c.power_w, c.start_m), (3, 250.0, 1500.0));
        assert_eq!(c.name, "Ada L");
        // Defaults: hills, 5 bots, 200 W.
        let (c, w) = parse_query("");
        assert!(w.is_empty());
        assert_eq!(c.route.length(), Route::test_hills().length());
        assert_eq!((c.bots, c.power_w), (5, 200.0));
        // Bad values are reported and ignored.
        let (c, w) = parse_query("bots=500&power=-3&route=moon&x=1&start=nan");
        assert_eq!(w.len(), 5, "{w:?}");
        assert_eq!((c.bots, c.power_w, c.start_m), (5, 200.0, 0.0));
    }

    #[test]
    fn the_hud_lists_no_desktop_only_keys() {
        for key in ["R: restart", "V: video", "F11"] {
            assert!(!WEB_KEYS.contains(key), "{key}");
        }
        assert!(ocw_render::hud::DESKTOP_KEYS.contains("R: restart"));
    }

    #[test]
    fn build_id_says_web() {
        let id = build_id();
        assert!(id.starts_with(env!("CARGO_PKG_VERSION")));
        assert!(id.ends_with(" web"));
    }
}
