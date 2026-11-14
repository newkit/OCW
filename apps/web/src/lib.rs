//! Open Cycling World in the browser (spec §14 Phase 10, milestone B1).
//!
//! The same client as the desktop app, built for `wasm32-unknown-unknown`
//! and run by the browser on WebGPU: the shared renderer
//! ([`ocw_render::OcwRenderPlugins`]: world, riders, chase camera, HUD,
//! console, sky), the shared client loop ([`ocw_client`]), and the same
//! `GameServer` as the local server – driven by the frame loop
//! ([`ocw_server::local::FrameServer`]) because wasm has no threads. Power
//! comes from the virtual trainer (↑ / ↓) until a Bluetooth trainer or
//! power meter is connected (B4, below).
//!
//! Desktop-only parts stay in `apps/desktop` (BLE, cpal audio, QUIC,
//! files, voice, captures, the menus' settings files); they move into the
//! shared crates behind web backends milestone by milestone (B2–B6), so
//! both apps stay one client (spec 10B, full parity).
//!
//! The page passes options in its query string (see [`SessionConfig`] and
//! [`parse_query`]): `?route=loop&bots=3&power=250&name=Ada`.
//!
//! **Multiplayer (B3):** `?server=https://HOST:47801&cert-hash=HEX&name=Ada`
//! joins a remote game server's session over WebTransport (`ocw-server
//! --listen --webtransport` prints the options) – the same sessions as
//! desktop and bot riders, with the same `ClientCore` prediction and
//! reconciliation; the rider is the anonymous one (plain bike and look,
//! no customization: Q5) until accounts exist (Phase 5).
//!
//! **Devices (B4):** the page's buttons (or `devices.connect trainer|power|hr`)
//! open the browser's Web Bluetooth chooser; the device runs the same
//! `ocw-devices` protocol as on the desktop (FTMS, Cycling Power, CSC,
//! Heart Rate, Wahoo) with SIM / ERG control and workouts ([`devices`]);
//! browsers without Web Bluetooth ride with virtual power and say so.
//!
//! **Worlds over HTTP (B2):** `?world=liechtenstein&route=vaduz-malbun`
//! streams a world package from the server ([`world`]): manifest and
//! signature chain, then the base and the tiles around the rider with
//! range requests, every chunk verified, cached in OPFS, the tiles kept
//! resident around the rider within a memory budget.

#[cfg(target_arch = "wasm32")]
mod browser;
#[cfg(target_arch = "wasm32")]
mod browser_devices;
pub mod devices;
pub mod plugin;
pub mod session;
#[cfg(target_arch = "wasm32")]
mod webtransport;
pub mod world;

#[path = "../../desktop/src/build_id.rs"]
mod build_id;

use bevy::prelude::*;
use ocw_render::hud::{AppKeys, VersionLabel};
use ocw_render::{OcwRenderPlugins, RideView};
use ocw_world::Route;

pub use session::{RemoteOptions, SessionConfig};

/// The default port of the server's WebTransport endpoint
/// (`ocw_server::net::webtransport::DEFAULT_PORT`; that module is native
/// only).
pub const DEFAULT_WEB_PORT: u16 = 47801;
/// The session path (`ocw_server::net::webtransport::PATH`).
pub const WEB_PATH: &str = "/ocw/1";

/// The page's `server` option as a WebTransport URL: `https://HOST[:PORT]
/// [/PATH]`, the port defaulting to [`DEFAULT_WEB_PORT`], the path to
/// [`WEB_PATH`].
pub fn server_url(spec: &str) -> Result<String, String> {
    let rest = spec
        .trim()
        .strip_prefix("https://")
        .ok_or("the server must be an https:// URL (WebTransport)")?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    let path = if matches!(path, "" | "/") {
        WEB_PATH
    } else {
        path
    };
    // [IPv6]:PORT, HOST:PORT, HOST.
    let (host, port) = if let Some(v6) = authority.strip_prefix('[') {
        let (h, after) = v6.split_once(']').ok_or("bad IPv6 address")?;
        (format!("[{h}]"), after.strip_prefix(':'))
    } else {
        match authority.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), Some(p)),
            None => (authority.to_string(), None),
        }
    };
    if host.is_empty() || host == "[]" {
        return Err("no host".into());
    }
    let port = match port {
        Some(p) => p.parse::<u16>().map_err(|_| format!("bad port {p:?}"))?,
        None => DEFAULT_WEB_PORT,
    };
    Ok(format!("https://{host}:{port}{path}"))
}

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

/// Decodes `%XX` escapes and `+` of a query value.
fn unescape(value: &str) -> String {
    let b = value.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                let hex = |c: u8| (c as char).to_digit(16);
                match (hex(b[i + 1]), hex(b[i + 2])) {
                    (Some(h), Some(l)) => {
                        out.push((h * 16 + l) as u8);
                        i += 2;
                    }
                    _ => out.push(b'%'),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The ride options from the page's query string (`?a=1&b=2`, the `?`
/// optional): `route` (hills, loop; with `world` a route of the package),
/// `bots` (0–50), `power` (W), `start` (km), `name`, `ftp` (W, for
/// workouts), and the world
/// package (spec §14 Phase 10 B2): `world` (a manifest URL, `ID@VERSION`
/// or `ID`), `free=1` (a free ride), `tiles` (fetch radius in tiles),
/// `keep` (keep radius), `memory-mb` (budget of the resident tiles),
/// `cache-mb` (OPFS budget), `cache=0`. On a development page
/// (`dev_page`: localhost) also `allow-unsigned=1`, `trust=dev` and
/// `trust-root=HEX` (a root public key). A remote server (B3): `server`
/// (`https://HOST[:PORT]`) and `cert-hash` (its certificate's SHA-256 in
/// hex, several separated by commas). Unknown keys and bad values are
/// reported and ignored.
pub fn parse_query(query: &str, dev_page: bool) -> (SessionConfig, Vec<String>) {
    let mut config = SessionConfig::default();
    let mut warnings = Vec::new();
    let mut pairs: Vec<(String, String)> = Vec::new();
    for pair in query.trim_start_matches('?').split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        pairs.push((key.to_string(), unescape(value)));
    }
    let mut world = pairs
        .iter()
        .find(|(k, _)| k == "world")
        .map(|(_, v)| world::WorldOptions::new(v.trim()));
    if world.as_ref().is_some_and(|w| w.spec.is_empty()) {
        warnings.push("world=: ignored".into());
        world = None;
    }
    for (key, value) in &pairs {
        let mut bad = || warnings.push(format!("{key}={value}: ignored"));
        match (key.as_str(), world.as_mut()) {
            ("world", _) => {}
            ("route", Some(w)) if !value.trim().is_empty() => w.route = Some(value.trim().into()),
            ("free", Some(w)) => w.free_ride = value != "0",
            ("tiles", Some(w)) => match value.parse::<i32>() {
                Ok(r @ 1..=8) => {
                    w.residency.fetch_radius = r;
                    w.residency.keep_radius = w.residency.keep_radius.max(r + 1);
                }
                _ => bad(),
            },
            ("keep", Some(w)) => match value.parse::<i32>() {
                Ok(r @ 2..=12) => w.residency.keep_radius = r,
                _ => bad(),
            },
            ("memory-mb", Some(w)) => match value.parse::<u64>() {
                Ok(m @ 1..=4096) => w.residency.budget_bytes = m << 20,
                _ => bad(),
            },
            ("cache-mb", Some(w)) => match value.parse::<u64>() {
                Ok(m @ 1..=65536) => w.cache_budget = m << 20,
                _ => bad(),
            },
            ("cache", Some(w)) => w.use_cache = value != "0",
            ("allow-unsigned", Some(w)) if dev_page => w.allow_unsigned = value != "0",
            ("trust", Some(w)) if dev_page && value == "dev" => w.trust_dev = true,
            ("trust-root", Some(w)) if dev_page => {
                match ocw_world::manifest::Hex::<32>::parse(value.trim()) {
                    Ok(h) => w.trust_roots.push(h.0),
                    Err(_) => bad(),
                }
            }
            ("route", None) => match value.as_str() {
                "hills" => config.route = Route::test_hills(),
                "loop" => config.route = Route::test_loop(),
                _ => bad(),
            },
            ("bots", _) => match value.parse::<u32>() {
                Ok(n) if n <= 50 => config.bots = n,
                _ => bad(),
            },
            ("power", _) => match value.parse::<f64>() {
                Ok(w) if (0.0..=session::MAX_POWER_W).contains(&w) => config.power_w = w,
                _ => bad(),
            },
            ("start", _) => match value.parse::<f64>() {
                Ok(km) if km >= 0.0 && km.is_finite() => config.start_m = km * 1000.0,
                _ => bad(),
            },
            ("ftp", _) => match value.parse::<f64>() {
                Ok(w) if (50.0..=600.0).contains(&w) => config.ftp_w = w,
                _ => bad(),
            },
            ("name", _) if !value.trim().is_empty() => {
                config.name = value.trim().chars().take(32).collect()
            }
            // Read below (B3).
            ("server" | "cert-hash", _) => {}
            _ => bad(),
        }
    }
    let mut remote: Option<RemoteOptions> = None;
    for (key, value) in &pairs {
        if key == "server" {
            match server_url(value) {
                Ok(url) => {
                    remote = Some(RemoteOptions {
                        url,
                        cert_hashes: Vec::new(),
                    })
                }
                Err(e) => warnings.push(format!("server={value}: {e}")),
            }
        }
    }
    for (key, value) in &pairs {
        if key != "cert-hash" {
            continue;
        }
        let Some(r) = remote.as_mut() else {
            warnings.push(format!("cert-hash={value}: ignored (no server)"));
            continue;
        };
        for h in value.split(',').map(|h| h.trim().replace(':', "")) {
            match ocw_world::manifest::Hex::<32>::parse(&h) {
                Ok(hash) => r.cert_hashes.push(hash.0),
                Err(_) => warnings.push(format!("cert-hash={h}: not a SHA-256 (64 hex digits)")),
            }
        }
    }
    config.world = world;
    config.remote = remote;
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
    .add_plugins(OcwRenderPlugins);
    // The browser's WebTransport for a remote server (B3).
    #[cfg(target_arch = "wasm32")]
    app.insert_resource(plugin::MakeTransport(Box::new(|r| {
        Box::new(webtransport::WebTransportLink::new(r.clone()))
    })));
    app.add_plugins(plugin::WebSessionPlugin { config });
    app
}

/// Console lines from the page (`window.ocw.console("teleport 3")`, the
/// browser test), until a frame runs them.
static PAGE_CONSOLE: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Queues a console line from the page.
pub fn queue_console(line: &str) {
    PAGE_CONSOLE
        .lock()
        .expect("not poisoned")
        .push(line.to_string());
}

/// Hands the page's console lines to the console.
pub(crate) fn console_from_page(console: Option<ResMut<ocw_render::console::ConsoleState>>) {
    let Some(mut console) = console else { return };
    let lines = std::mem::take(&mut *PAGE_CONSOLE.lock().expect("not poisoned"));
    for line in lines {
        console.queue(line);
    }
}

/// The wasm module's memory (bytes; it only grows, so this is its peak);
/// `None` natively.
#[cfg(target_arch = "wasm32")]
pub fn wasm_memory_bytes() -> Option<u64> {
    Some(browser::memory_bytes())
}

/// The wasm module's memory: `None` natively.
#[cfg(not(target_arch = "wasm32"))]
pub fn wasm_memory_bytes() -> Option<u64> {
    None
}

/// `window.ocw.console(line)`: runs a console command (the browser test
/// drives the ride with it).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = console)]
pub fn page_console(line: &str) {
    queue_console(line);
}

/// `window.ocw.netStats()`: the connection's numbers as JSON (the remote
/// link's state, round trip, snapshot timing, the other riders seen).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = netStats)]
pub fn page_net_stats() -> String {
    plugin::net_stats_json()
}

/// `window.ocw.wasmMemory()`: the wasm memory in bytes.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = wasmMemory)]
pub fn page_memory() -> f64 {
    browser::memory_bytes() as f64
}

/// The entry point the wasm-bindgen glue calls when the module is loaded.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    let location = web_sys::window().map(|w| w.location());
    let query = location
        .as_ref()
        .and_then(|l| l.search().ok())
        .unwrap_or_default();
    let host = location
        .as_ref()
        .and_then(|l| l.hostname().ok())
        .unwrap_or_default();
    let (config, warnings) = parse_query(&query, is_dev_host(&host));
    let world = config.world.clone();
    let mut app = app(config);
    for w in warnings {
        warn!("page options: {w}");
    }
    browser::start_world(&mut app, world);
    app.run();
}

/// Development pages (the dev server): only there the page may relax
/// the package trust (`allow-unsigned`, `trust=dev`, `trust-root`).
pub fn is_dev_host(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "[::1]" | "::1")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_options() {
        let (c, w) = parse_query(
            "?route=loop&bots=3&power=250&start=1.5&name=Ada+L&ftp=260",
            false,
        );
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(c.ftp_w, 260.0);
        assert_eq!(c.route.length(), Route::test_loop().length());
        assert_eq!((c.bots, c.power_w, c.start_m), (3, 250.0, 1500.0));
        assert_eq!(c.name, "Ada L");
        assert!(c.world.is_none());
        // Defaults: hills, 5 bots, 200 W.
        let (c, w) = parse_query("", false);
        assert!(w.is_empty());
        assert_eq!(c.route.length(), Route::test_hills().length());
        assert_eq!((c.bots, c.power_w), (5, 200.0));
        // Bad values are reported and ignored.
        let (c, w) = parse_query("bots=500&power=-3&route=moon&x=1&start=nan&ftp=9", false);
        assert_eq!(w.len(), 6, "{w:?}");
        assert_eq!((c.bots, c.power_w, c.start_m), (5, 200.0, 0.0));
    }

    /// The world options (B2): with `world`, `route` names a package route;
    /// the trust options only on development pages.
    #[test]
    fn world_options() {
        let q = "world=liechtenstein&route=Vaduz%E2%80%93Malbun&tiles=2&keep=4&memory-mb=64&cache-mb=300&allow-unsigned=1&trust=dev";
        let (c, w) = parse_query(q, true);
        assert!(w.is_empty(), "{w:?}");
        let world = c.world.unwrap();
        assert_eq!(world.spec, "liechtenstein");
        assert_eq!(world.route.as_deref(), Some("Vaduz–Malbun"));
        assert_eq!(
            (world.residency.fetch_radius, world.residency.keep_radius),
            (2, 4)
        );
        assert_eq!(world.residency.budget_bytes, 64 << 20);
        assert_eq!(world.cache_budget, 300 << 20);
        assert!(world.allow_unsigned && world.trust_dev && world.use_cache);
        // The test route stays the placeholder of the view.
        assert_eq!(c.route.length(), Route::test_hills().length());
        // Not on a development page: the trust options are ignored.
        let (c, w) = parse_query(q, false);
        assert_eq!(w.len(), 2, "{w:?}");
        let world = c.world.unwrap();
        assert!(!world.allow_unsigned && !world.trust_dev);
        // A root key, a free ride, no cache; bad values.
        let root = "ab".repeat(32);
        let (c, w) = parse_query(
            &format!("world=x/y@3&free=1&cache=0&trust-root={root}&tiles=99&keep=1"),
            true,
        );
        assert_eq!(w.len(), 2, "{w:?}");
        let world = c.world.unwrap();
        assert!(world.free_ride && !world.use_cache);
        assert_eq!(world.trust_roots, [[0xab; 32]]);
        // Without a world, the world options are unknown keys.
        let (_, w) = parse_query("tiles=2&world=", false);
        assert_eq!(w.len(), 2, "{w:?}");
        assert!(is_dev_host("127.0.0.1") && !is_dev_host("ocw.example"));
        assert_eq!(unescape("a%2Fb+c%zz%"), "a/b c%zz%");
    }

    /// The remote server (B3): `server` and `cert-hash`.
    #[test]
    fn server_options() {
        let hash = "ab".repeat(32);
        let q = format!("server=https%3A%2F%2F127.0.0.1%3A47801&cert-hash={hash}&name=Web");
        let (c, w) = parse_query(&q, false);
        assert!(w.is_empty(), "{w:?}");
        let r = c.remote.unwrap();
        assert_eq!(r.url, "https://127.0.0.1:47801/ocw/1");
        assert_eq!(r.cert_hashes, [[0xab; 32]]);
        assert_eq!(c.name, "Web");
        // Two hashes (before and after a renewal), with colons.
        let colons: Vec<String> = (0..32).map(|_| "CD".to_string()).collect();
        let (c, w) = parse_query(
            &format!(
                "server=https://ocw.example&cert-hash={hash},{}",
                colons.join(":")
            ),
            false,
        );
        assert!(w.is_empty(), "{w:?}");
        let r = c.remote.unwrap();
        assert_eq!(r.url, "https://ocw.example:47801/ocw/1");
        assert_eq!(r.cert_hashes, [[0xab; 32], [0xcd; 32]]);
        // Without a hash the browser checks the certificate itself (B8).
        let (c, _) = parse_query("server=https://[::1]:9000/x", false);
        assert_eq!(c.remote.unwrap().url, "https://[::1]:9000/x");
        // Bad values.
        let (c, w) = parse_query("server=http://x&cert-hash=12", false);
        assert!(c.remote.is_none());
        assert_eq!(w.len(), 2, "{w:?}");
        let (c, w) = parse_query("server=https://x:99999&cert-hash=zz", false);
        assert!(c.remote.is_none() && w.len() == 2, "{w:?}");
        assert!(server_url("https://").is_err());
        assert_eq!(server_url("https://h/").unwrap(), "https://h:47801/ocw/1");
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
