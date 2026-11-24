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
//! Desktop-only parts stay in `apps/desktop` (QUIC, files, captures, the
//! menus); they move into the
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
//! **Audio and voice (B5):** the desktop's synthesised game sounds through
//! Web Audio ([`audio`]: the shared `ocw-audio` mixer, started by the first
//! click or key – the browser's autoplay rule), voice chat over LiveKit's
//! JS SDK with the same controller, rooms and tokens as the desktop
//! ([`voice`]), the settings in `localStorage` ([`settings`]).
//!
//! **UI parity (B6):** the desktop's in-game menu, settings pages, time
//! and weather machines, events, collisions and rider list are the shared
//! `ocw_ui` plugins over this session ([`ui`]); keys, the gamepad (the
//! browser's Gamepad API through Bevy's gilrs), mouse and touch drive it;
//! every setting the browser has applies and is kept in `localStorage`;
//! screenshots, videos and the ride's FIT file are downloads named with
//! the time.
//!
//! **Unicorn mode (Phase 13 G5):** the desktop's god mode, shared as
//! `ocw_ui::god` – G (or World → Unicorn mode) flies, + / − by what the
//! key types, the gamepad, touch on the left half, the inspection (I, P),
//! the mouse look in the page's Pointer Lock ([`god`]); a shared ride
//! keeps the server's rules (the host flies).
//!
//! **Worlds over HTTP (B2):** `?world=liechtenstein&route=vaduz-malbun`
//! streams a world package from the server ([`world`]): manifest and
//! signature chain, then the base and the tiles around the rider with
//! range requests, every chunk verified, cached in OPFS, the tiles kept
//! resident around the rider within a memory budget.

pub mod account;
pub mod audio;
pub mod bridge;
#[cfg(target_arch = "wasm32")]
mod browser;
#[cfg(target_arch = "wasm32")]
mod browser_audio;
#[cfg(target_arch = "wasm32")]
mod browser_devices;
#[cfg(target_arch = "wasm32")]
mod browser_voice;
pub mod devices;
pub mod fitstore;
pub mod god;
pub mod gpu_memory;
pub mod heap;
pub mod memory;
#[cfg(test)]
mod memory_report;
pub mod page_config;
pub mod plugin;
pub mod screen;
pub mod session;
pub mod settings;
pub mod strava;
pub mod ui;
pub mod voice;
#[cfg(target_arch = "wasm32")]
mod webtransport;
#[cfg(feature = "webxr")]
pub mod webxr;
pub mod world;
pub mod xr;

#[path = "../../desktop/src/build_id.rs"]
mod build_id;

// Bevy renders with WebGPU or WebGL 2, chosen when it is compiled (spec
// §14 Phase 10 B7): a browser build takes exactly one of the features.
#[cfg(all(
    target_arch = "wasm32",
    not(any(feature = "webgpu", feature = "webgl2"))
))]
compile_error!(
    "ocw-web for wasm32 needs `--features webgpu` or `--features webgl2` (scripts/build-web.sh builds both)"
);
#[cfg(all(target_arch = "wasm32", feature = "webgpu", feature = "webgl2"))]
compile_error!("ocw-web: `webgpu` and `webgl2` are two builds, not one (Bevy would take WebGPU)");

/// The GPU API this build renders with: "WebGPU" or "WebGL2" in the
/// browser (spec §14 Phase 10 B7), "native" in the native tests.
pub const GPU_API: &str = if cfg!(feature = "webgl2") {
    "WebGL2"
} else if cfg!(feature = "webgpu") {
    "WebGPU"
} else {
    "native"
};

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

/// This build's id: version, commit, profile, "web", the GPU API
/// ([`GPU_API`], B7) and "preview" for the public preview (B8a).
pub fn build_id() -> String {
    format!(
        "{} web {GPU_API}{}",
        build_id::build_id(
            env!("CARGO_PKG_VERSION"),
            env!("OCW_COMMIT"),
            cfg!(debug_assertions)
        ),
        if cfg!(feature = "preview") {
            " preview"
        } else {
            ""
        }
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
/// hex, several separated by commas). `origin=LAT,LON` gives the test
/// route a geographic origin (the GPS line). Unknown keys and bad values
/// are reported and ignored.
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
            // Touch steering for this visit (spec §3.2.3; the tests).
            ("touch", _) => match value.as_str() {
                "auto" | "on" | "off" => config.touch = Some(value.clone()),
                _ => bad(),
            },
            // The quality preset for this visit (B7).
            ("quality", _) => match ocw_client::quality::QualitySetting::parse(value) {
                Some(q) => config.quality = Some(q.name().into()),
                None => bad(),
            },
            // The page's choice of build (index.html, B7), and its mark of
            // a ride reloaded on WebGL 2 after WebGPU failed (ocw-gpu.js).
            ("gpu", _) if matches!(value.as_str(), "webgpu" | "webgl2") => {}
            ("webgpu-failed", _) => {}
            // The world's memory (`memory::MemoryMode`; the page sets
            // `saver` after a page died in a ride, ocw-gpu.js).
            ("memory", _) => match memory::MemoryMode::parse(value) {
                Some(m) => config.memory = m,
                None => bad(),
            },
            // Read below (B3; the origin after the route; a join link).
            ("server" | "cert-hash" | "join", _) => {}
            // The accounts server (spec §9.1; `account`).
            ("auth", _) => {}
            ("origin", None) => {}
            _ => bad(),
        }
    }
    // A geographic origin for the test route (`origin=LAT,LON`): its
    // metres get a place on the globe, as a route file's have – the GPS
    // line, the sun and the FIT track follow (spec 3.40; the browser
    // test's GPS check).
    if world.is_none()
        && let Some((_, value)) = pairs.iter().find(|(k, _)| k == "origin")
    {
        match value
            .split_once(',')
            .and_then(|(a, b)| Some((a.trim().parse::<f64>().ok()?, b.trim().parse::<f64>().ok()?)))
        {
            Some((lat, lon)) if lat.abs() <= 85.0 && lon.abs() <= 180.0 => {
                config.route = config.route.clone().with_geo_origin((lat, lon));
            }
            _ => warnings.push(format!("origin={value}: ignored")),
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
    // `join=LINK` (spec §6.2 *Join links*): the server of an https://
    // link, its world and route.
    if let Some((_, value)) = pairs.iter().find(|(k, _)| k == "join") {
        match web_join(value) {
            Ok((r, link)) => {
                remote = Some(r);
                if let Some(w) = &link.world
                    && world.is_none()
                {
                    let mut opts = world::WorldOptions::new(w);
                    opts.route = link.route.clone();
                    world = Some(opts);
                } else if world.is_none() {
                    match link.route.as_deref() {
                        Some("hills") => config.route = Route::test_hills(),
                        Some("loop") => config.route = Route::test_loop(),
                        _ => {}
                    }
                }
            }
            Err(e) => warnings.push(format!("join={value}: {e}")),
        }
    }
    config.world = world;
    config.remote = remote;
    (config, warnings)
}

/// What the browser says about an `ocw://` link (spec §3.2.1 *Join*).
pub const NO_WEBTRANSPORT: &str = "this server has no WebTransport for browsers (an ocw:// link): ask its host for an https:// link, or use the desktop app";

/// A join link as the browser can use it: WebTransport with the
/// certificate's hash (browsers take a self-signed certificate only by
/// its hash).
pub fn web_join(text: &str) -> Result<(RemoteOptions, ocw_protocol::join::JoinLink), String> {
    use ocw_protocol::join::{JoinLink, LinkTransport};
    let link = JoinLink::parse(text)?;
    if link.transport != LinkTransport::WebTransport {
        return Err(NO_WEBTRANSPORT.into());
    }
    let cert = link.cert.as_deref().ok_or(
        "the browser needs the server's certificate hash (cert-hash= in the link) to connect",
    )?;
    let hash = ocw_world::manifest::Hex::<32>::parse(cert)
        .map_err(|_| "the certificate hash has 64 hex digits".to_string())?;
    Ok((
        RemoteOptions {
            url: link.web_url(),
            cert_hashes: vec![hash.0],
        },
        link,
    ))
}

/// The page's query for joining `link` (Ride → Join in the browser): the
/// page starts again on the shared ride – `server`, `cert-hash` and the
/// link's world and route replace the page's; its other options stay.
pub fn join_query(query: &str, link: &str) -> Result<String, String> {
    let (_, l) = web_join(link)?;
    let mut drop = vec!["server", "cert-hash", "join"];
    if l.world.is_some() {
        drop.extend(["world", "route", "free"]);
    } else if l.route.is_some() {
        drop.push("route");
    }
    let mut out = kept_pairs(query, &drop);
    out.push(format!("server={}", escape(&l.web_url())));
    out.push(format!("cert-hash={}", l.cert.clone().unwrap_or_default()));
    if let Some(w) = &l.world {
        out.push(format!("world={}", escape(w)));
    }
    if let Some(r) = &l.route {
        out.push(format!("route={}", escape(r)));
    }
    Ok(format!("?{}", out.join("&")))
}

/// The page's query for leaving the shared ride: without the server.
pub fn leave_query(query: &str) -> String {
    let out = kept_pairs(query, &["server", "cert-hash", "join"]);
    if out.is_empty() {
        String::new()
    } else {
        format!("?{}", out.join("&"))
    }
}

fn kept_pairs(query: &str, drop: &[&str]) -> Vec<String> {
    query
        .trim_start_matches('?')
        .split('&')
        .filter(|p| !p.is_empty())
        .filter(|p| !drop.contains(&p.split_once('=').map_or(*p, |(k, _)| k)))
        .map(str::to_owned)
        .collect()
}

/// Percent-encodes a query value.
fn escape(v: &str) -> String {
    let mut out = String::new();
    for b in v.bytes() {
        match b {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'.'
            | b'_'
            | b'~'
            | b'/'
            | b':'
            | b'@' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Text the page's paste event brought (`window.ocw.paste(text)`), for
/// Ride → Join's link field.
pub static PASTED: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// `window.ocw.paste(text)`: the page's paste event (index.html).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = paste)]
pub fn page_paste(text: &str) {
    *PASTED.lock().expect("not poisoned") = Some(text.to_owned());
    // Pasting while the menu's link field is open puts it there.
    queue_console("join.paste");
}

/// The browser app: Bevy with the canvas of the page, the shared renderer
/// and the in-browser ride session; the settings from the page's store.
pub fn app(config: SessionConfig) -> App {
    let (settings, warnings) = settings::WebSettings::load(settings::default_store());
    app_with_settings(config, settings, warnings)
}

/// [`app`] with the settings already read (the page applies the ones
/// read at the start to the config first).
pub fn app_with_settings(
    mut config: SessionConfig,
    mut settings: settings::WebSettings,
    mut warnings: Vec<String>,
) -> App {
    let mut app = App::new();
    visit_quality(&config, &mut settings, &mut warnings);
    // Phones keep fewer world tiles (the iPhone's restarts, 2026-11-22).
    let memory = apply_memory_mode(&mut config, &device_info());
    app.insert_resource(ocw_render::package::StreamRadiusCap(memory.ring_cap()));
    if memory != memory::MemoryMode::Normal {
        info!(
            "world memory: {} (fewer tiles in memory and on the GPU)",
            memory.name()
        );
    }
    app.insert_resource(settings)
        .insert_resource(settings::Warnings(warnings))
        // What the quality preset's `auto` decides by (B7).
        .insert_resource(ocw_ui::quality::QualityContext {
            api: gpu_api(),
            device: device_info(),
            // An immersive session sets it (14I W3, `webxr`).
            xr: None,
        });
    let record = config.record;
    let route = config.route.clone();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Open Cycling World".into(),
                    canvas: Some(CANVAS_ID.into()),
                    // The page's size from the first frame: Bevy's default
                    // 1280 × 720 at an iPhone's pixel ratio of 3 made three
                    // 3840 × 2160 targets (95 MB of GPU memory) for the
                    // first two seconds (the iPhone's memory, v5.42).
                    resolution: initial_resolution(),
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
            }),
    )
    // A renderer error goes to the page (B7): WebGPU falls back to WebGL 2,
    // otherwise the page shows it – never Bevy's quit with a black canvas.
    .insert_resource(bevy::render::error_handler::RenderErrorHandler(
        page_render_error,
    ))
    .insert_resource(Time::<Fixed>::from_hz(f64::from(ocw_core::SIM_TICK_HZ)))
    .insert_resource(RideView::new(route))
    .insert_resource(VersionLabel(build_id()))
    .insert_resource(AppKeys(WEB_KEYS.into()))
    // The quality preset's pixel-ratio cap (B7): fewer pixels on HiDPI
    // phones at Low and Medium.
    .add_systems(PostUpdate, cap_pixel_ratio)
    // A lost WebGL context stops the app (the iPhone's restarts, 2026-11-22).
    .add_systems(First, stop_when_gpu_lost)
    // Where the heap's start peak comes from (`window.ocw.heap().trace`).
    .add_systems(First, || {
        heap::FRAME.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        heap::mark(1);
    })
    .add_systems(PreUpdate, || heap::mark(2))
    .add_systems(Update, || heap::mark(3))
    .add_systems(PostUpdate, || heap::mark(4))
    .add_systems(Last, || heap::mark(5))
    .add_systems(Startup, || {
        info!("Open Cycling World {} starting", build_id())
    })
    .add_plugins(OcwRenderPlugins)
    // Phones: smaller shadow maps (the iPhone's memory, v5.42).
    .insert_resource(ocw_render::quality::ShadowMapCap(memory.shadow_map_cap()))
    .insert_resource(memory.tile_pace())
    .insert_resource(ocw_render::package::PlantThinning(memory.plant_thinning()))
    // Near the phone's budget the world steps down (`memory::MemoryGuard`).
    .add_systems(Update, memory::guard_memory)
    // The GPU memory for the status line and the browser test (B2).
    .add_plugins(gpu_memory::GpuMemoryPlugin);
    // The browser's WebTransport for a remote server (B3).
    #[cfg(target_arch = "wasm32")]
    app.insert_resource(plugin::MakeTransport(Box::new(|r| {
        Box::new(webtransport::WebTransportLink::new(r.clone()))
    })));
    // The page's Web Audio output and the voice shim (B5).
    #[cfg(target_arch = "wasm32")]
    app.insert_resource(audio::MakeAudioOut(Box::new(browser_audio::start)));
    // No voice on this page (the page configuration, B8a): the Voice page
    // and `voice.status` say why.
    if let Some(why) = config.voice_off.clone()
        && !app.world().contains_resource::<voice::MakeVoiceBackend>()
    {
        app.insert_resource(voice::MakeVoiceBackend(Box::new(move || {
            voice::unavailable(&why)
        })));
    }
    // Bevy allocates the point lights' shadow cube (1024² × 6, 24 MB) even
    // though no light of ours casts a point shadow: phones take a tiny one.
    if let Some(guard) = memory::MemoryGuard::for_mode(memory) {
        app.insert_resource(guard);
    }
    if let Some(size) = memory.photo_size() {
        app.insert_resource(size);
    }
    if let Some(size) = memory.point_shadow_px() {
        app.insert_resource(bevy::light::PointLightShadowMap { size });
    }
    app.add_plugins(plugin::WebSessionPlugin { config });
    // Settings first: audio and voice start with the saved values.
    app.add_plugins((
        settings::WebSettingsPlugin,
        audio::WebAudioPlugin,
        voice::WebVoicePlugin,
    ));
    // The desktop's menus and machines over this session, the downloads
    // (B6).
    app.add_plugins(ui::WebUiPlugin { record });
    // WebXR (spec §14 Phase 14, 14I W1): the immersive session's eyes.
    #[cfg(feature = "webxr")]
    app.add_plugins(webxr::WebXrPlugin);
    app
}

/// The memory mode of this visit (the page's `memory=`, else by the
/// device) applied to the world's options: a phone's residency and its
/// course (`memory::MemoryMode`). The page's start applies it before it
/// hands the options to the world's driver (v5.42: the driver once got
/// them unadjusted and decoded a phone's base whole); applying it again
/// changes nothing.
pub fn apply_memory_mode(
    config: &mut SessionConfig,
    device: &ocw_client::quality::DeviceInfo,
) -> memory::MemoryMode {
    let memory = memory::MemoryMode::resolve(config.memory, device);
    if let Some(w) = config.world.as_mut() {
        w.residency = memory.residency(w.residency);
        w.course_rings = memory.course_rings(w.residency);
    }
    memory
}

/// The window's size at the start: the page's (CSS pixels), never more
/// than Bevy's default; natively Bevy's default.
pub fn initial_resolution() -> bevy::window::WindowResolution {
    #[cfg(target_arch = "wasm32")]
    {
        let size = web_sys::window().map(|w| {
            let n = |v: Result<wasm_bindgen::JsValue, _>| {
                v.ok().and_then(|v| v.as_f64()).unwrap_or(0.0)
            };
            (n(w.inner_width()), n(w.inner_height()))
        });
        if let Some((w, h)) = size {
            let (w, h) = start_size(w, h);
            return bevy::window::WindowResolution::new(w, h);
        }
    }
    bevy::window::WindowResolution::default()
}

/// [`initial_resolution`]'s numbers: the page's size in whole pixels, at
/// most 1280 each way (a page without a size: Bevy's default 1280 × 720).
pub fn start_size(w: f64, h: f64) -> (u32, u32) {
    if !(w >= 1.0 && h >= 1.0) {
        return (1280, 720);
    }
    (w.min(1280.0) as u32, h.min(1280.0) as u32)
}

/// Whether the renderer's error was already reported to the page (the
/// handler runs every frame while rendering is stopped).
static RENDER_ERROR_REPORTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// What the page is told of a renderer error (`window.ocwRenderFailed(kind,
/// description)`): the error's kind and its first line, shortened.
pub fn render_error_report(error: &bevy::render::error_handler::RenderError) -> (String, String) {
    let first = error.description.lines().next().unwrap_or("").trim();
    let text: String = first.chars().take(300).collect();
    (format!("{:?}", error.ty), text)
}

/// The browser's renderer error handler (spec §14 Phase 10 B7): Bevy's
/// default quits the app on any renderer error, which leaves a page with
/// sound and a black canvas; here rendering stops, the app keeps running
/// and the page is told once (`window.ocwRenderFailed`; index.html: WebGPU
/// → the WebGL 2 build, else the error on the page).
pub fn page_render_error(
    error: &bevy::render::error_handler::RenderError,
    _main: &mut World,
    _render: &mut World,
) -> bevy::render::error_handler::RenderErrorPolicy {
    use std::sync::atomic::Ordering;
    if !RENDER_ERROR_REPORTED.swap(true, Ordering::Relaxed) {
        let (kind, text) = render_error_report(error);
        error!("renderer stopped ({kind}): {text}; the page falls back or shows it");
        #[cfg(target_arch = "wasm32")]
        browser::render_failed(&kind, &text);
    }
    bevy::render::error_handler::RenderErrorPolicy::StopRendering
}

/// The canvas' scale factor for a device pixel ratio `base` under the
/// preset's cap: `None` (the device's own) when it is within the cap.
pub fn pixel_ratio_override(base: f32, cap: Option<f32>) -> Option<f32> {
    cap.filter(|&c| base > c)
}

/// The preset's pixel-ratio cap where the browser can apply it: winit sizes
/// the canvas from `devicePixelContentBoxSize`, and without it (WebKit:
/// Safari, every browser on iOS) the drawing buffer stays at the device's
/// pixels while the override only shrinks the HUD and menus to a third on
/// an iPhone (measured in WebKit, 2026-11-15) – so there no cap.
pub fn usable_pixel_ratio_cap(cap: Option<f32>, device_pixel_box: bool) -> Option<f32> {
    cap.filter(|_| device_pixel_box)
}

/// Whether the browser reports `devicePixelContentBoxSize` to resize
/// observers (Chromium, Firefox; not WebKit); natively true.
fn device_pixel_box() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        thread_local! {
            static HAS: bool = web_sys::window()
                .and_then(|w| js_sys::Reflect::get(&w, &"ResizeObserverEntry".into()).ok())
                .and_then(|c| js_sys::Reflect::get(&c, &"prototype".into()).ok())
                .is_some_and(|p| js_sys::Reflect::has(&p, &"devicePixelContentBoxSize".into()).unwrap_or(false));
        }
        HAS.with(|h| *h)
    }
    #[cfg(not(target_arch = "wasm32"))]
    true
}

/// The canvas' scale factor during an immersive session (spec §14 Phase
/// 14, 14I W3; the `webxr` build sets it): the eyes draw into the canvas
/// and the page copies it into the XR layer's framebuffer, so the canvas
/// gets the framebuffer's pixels – the layer's render scale is then the
/// eyes' too, and the copy is 1 : 1. `None`: the preset's cap.
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq)]
pub struct XrCanvasScale(pub Option<f32>);

/// The scale factor that gives a canvas of `logical_width` CSS px the
/// framebuffer's `framebuffer_width` px (`None` for nonsense sizes).
pub fn xr_canvas_scale(logical_width: f32, framebuffer_width: f32) -> Option<f32> {
    (logical_width >= 1.0 && framebuffer_width >= 1.0)
        .then(|| (framebuffer_width / logical_width).clamp(0.25, 8.0))
}

/// Applies [`pixel_ratio_override`] to the page's window when the preset
/// or the device's pixel ratio changes (the browser only: the desktop's
/// window keeps its own scale).
fn cap_pixel_ratio(
    quality: Option<Res<ocw_render::quality::RenderQuality>>,
    xr: Option<Res<XrCanvasScale>>,
    mut windows: Query<&mut Window, With<bevy::window::PrimaryWindow>>,
) {
    let cap = usable_pixel_ratio_cap(
        quality.and_then(|q| q.0.max_pixel_ratio),
        device_pixel_box(),
    );
    for mut w in &mut windows {
        // In an immersive session the canvas is as large as the XR layer's
        // framebuffer (14I W3: its render scale), whatever the preset says.
        let xr = xr.as_ref().and_then(|x| x.0);
        let want = xr.or_else(|| pixel_ratio_override(w.resolution.base_scale_factor(), cap));
        if w.resolution.scale_factor_override() != want {
            w.resolution.set_scale_factor_override(want);
        }
    }
}

/// `?quality=low`: `display.quality` for this visit only (B7: a temporary
/// value, never saved).
pub fn visit_quality(
    config: &SessionConfig,
    settings: &mut settings::WebSettings,
    warnings: &mut Vec<String>,
) {
    if let Some(q) = &config.quality
        && let Err(e) = settings.settings.set_temp("display.quality", q)
    {
        warnings.push(format!("quality={q}: {e}"));
    }
    if let Some(t) = &config.touch
        && let Err(e) = settings.settings.set_temp("controls.touch_steer", t)
    {
        warnings.push(format!("touch={t}: {e}"));
    }
}

/// This build's GPU API for the quality preset (the native tests count as
/// the WebGPU build, the page's default).
pub fn gpu_api() -> ocw_client::quality::GpuApi {
    use ocw_client::quality::GpuApi;
    if cfg!(feature = "webgl2") {
        GpuApi::WebGl2
    } else {
        GpuApi::WebGpu
    }
}

/// The page's facts about the device (`window.ocwDeviceInfo`, set by
/// index.html); natively nothing known.
pub fn device_info() -> ocw_client::quality::DeviceInfo {
    #[cfg(target_arch = "wasm32")]
    {
        let text = web_sys::window()
            .and_then(|w| js_sys::Reflect::get(&w, &"ocwDeviceInfo".into()).ok())
            .and_then(|v| v.as_string())
            .unwrap_or_default();
        ocw_client::quality::DeviceInfo::from_json(&text)
    }
    #[cfg(not(target_arch = "wasm32"))]
    ocw_client::quality::DeviceInfo::default()
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

/// The page saw the GPU go (`webglcontextlost`; `window.ocw.gpuLost()`).
static GPU_LOST: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Marks the GPU as lost: the next frame stops the app.
pub fn mark_gpu_lost() {
    GPU_LOST.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Whether the app must stop for a lost GPU (and then clears the mark):
/// once per loss.
pub fn take_gpu_lost() -> bool {
    GPU_LOST.swap(false, std::sync::atomic::Ordering::Relaxed)
}

/// Stops the app when the page lost its WebGL context (spec 14 Phase 10
/// B7; the iPhone's restarts of 2026-11-22). wgpu's GL backend cannot
/// rebuild its device, and with the context gone the frames' buffers are
/// never freed: the wasm memory grew by ~45 MB a second (224 MB → 2.7 GB
/// in 35 s in WebKit) until iOS would kill the tab and Safari load it
/// again – a restart loop whenever iOS takes the GPU from the page. Now
/// the app ends there; the page loads the ride again (once in ten
/// minutes) or says what happened.
fn stop_when_gpu_lost(mut exit: MessageWriter<AppExit>) {
    if take_gpu_lost() {
        warn!(
            "the GPU was lost (WebGL context lost): the app stops, the page loads the ride again"
        );
        exit.write(AppExit::Success);
    }
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

/// `window.ocw.gpuLost()`: the page's canvas lost its WebGL context.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = gpuLost)]
pub fn page_gpu_lost() {
    mark_gpu_lost();
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

/// `window.ocw.audio()`: the game audio's state as JSON (locked / running,
/// the schedule, the level, the mix; B5).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = audio)]
pub fn page_audio() -> String {
    audio::audio_json()
}

/// `window.ocw.voice()`: the voice call's state as JSON (status, switches,
/// participants, devices, the echo test; B5).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = voice)]
pub fn page_voice() -> String {
    voice::page_json()
}

/// `window.ocw.menu()`: the in-game menu as JSON (open, title, items, the
/// focused item, status, hint; B6 – the browser test drives it by keys).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = menu)]
pub fn page_menu() -> String {
    ui::menu_json()
}

/// `window.ocw.consoleState()`: the console (open, input line, completion
/// candidates, selected, the last log lines) and the HUD's GPS line as
/// JSON (spec 3.40).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = consoleState)]
pub fn page_console_state() -> String {
    ui::console_json()
}

/// `window.ocw.hudMap()`: the HUD's navigation map as JSON (shown, its
/// box in CSS px, the banner, the zoom, night; spec §3.2) – the page moves
/// the Devices panel above it.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = hudMap)]
pub fn page_hud_map() -> String {
    ui::map_json()
}

/// `window.ocw.hudLayout()`: the HUD's layout as JSON (compact on a
/// phone, the scale, the boxes of the top panel, the top-right column and
/// the profile in CSS px, the top panel's lines; spec 3.2 *Phone layout*).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = hudLayout)]
pub fn page_hud_layout() -> String {
    ui::hud_layout_json()
}

/// `window.ocw.god()`: unicorn mode as JSON (phase, pointer, position,
/// height, the inspection's panel and crosshair, pins; Phase 13 G5 – the
/// browser test flies with it).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = god)]
pub fn page_god() -> String {
    god::god_json()
}

/// `window.ocw.steer()`: touch steering as JSON (the line, the lean,
/// autodrive, the fingers, the sides held, the hints; spec §3.2.3 – the
/// browser test taps with it).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = steer)]
pub fn page_steer() -> String {
    ui::steer_json()
}

/// `window.ocw.setting(key)`: a setting's value as text ("" for an unknown
/// key; B6).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = setting)]
pub fn page_setting(key: &str) -> String {
    ui::setting_text(key)
}

/// `window.ocw.xrFrame(data)`: the XR frame's poses and projections
/// (`web/ocw-xr.js`, before it steps the app; 14I W1).
#[cfg(all(target_arch = "wasm32", feature = "webxr"))]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = xrFrame)]
pub fn page_xr_frame(data: &[f32]) {
    webxr::page_frame(data);
}

/// `window.ocw.xrEvent(name, detail)`: the immersive session's events
/// (supported, enter, started, failed, visibility, ended).
#[cfg(all(target_arch = "wasm32", feature = "webxr"))]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = xrEvent)]
pub fn page_xr_event(name: &str, detail: &str) {
    webxr::page_event(name, detail);
}

/// `window.ocw.xr()`: the session, the eyes and the frame timing as JSON.
#[cfg(all(target_arch = "wasm32", feature = "webxr"))]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = xr)]
pub fn page_xr() -> String {
    webxr::page_json()
}

/// `window.ocw.xrReport(json)`: the headset run's report to the desktop
/// over the sensor bridge (14I W4); "" when queued, else why not.
#[cfg(all(target_arch = "wasm32", feature = "webxr"))]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = xrReport)]
pub fn page_xr_report(json: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(json) {
        Ok(data) => bridge::report("xr", data).err().unwrap_or_default(),
        Err(e) => format!("not JSON: {e}"),
    }
}

/// `window.ocw.xrPads(data)`: the controllers of the XR frame (WebXR
/// gamepads and grips, `web/ocw-xr.js`, before `xrFrame`; 14I W2).
#[cfg(all(target_arch = "wasm32", feature = "webxr"))]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = xrPads)]
pub fn page_xr_pads(data: &[f32]) {
    webxr::page_pads(data);
}

/// `window.ocw.xrLayer(userAgent)`: the quality level a session on this
/// headset runs and the XR layer's render scale and foveation (JSON; 14I
/// W3: the `quest1` level on a Quest 1).
#[cfg(all(target_arch = "wasm32", feature = "webxr"))]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = xrLayer)]
pub fn page_xr_layer(user_agent: &str) -> String {
    webxr::page_layer(user_agent)
}

/// `window.ocw.xrShot(json)`: a left-eye picture of the headset's view to
/// the desktop over the sensor bridge (14I W3, `vr.shot`'s counterpart:
/// `{"jpeg_base64": …, "w": …, "h": …}`, ≤ 16 KB); "" when queued, else why
/// not.
#[cfg(all(target_arch = "wasm32", feature = "webxr"))]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = xrShot)]
pub fn page_xr_shot(json: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(json) {
        Ok(data) => bridge::report("xr-shot", data).err().unwrap_or_default(),
        Err(e) => format!("not JSON: {e}"),
    }
}

/// `window.ocw.wasmMemory()`: the wasm memory in bytes.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = wasmMemory)]
pub fn page_memory() -> f64 {
    browser::memory_bytes() as f64
}

/// `window.ocw.heap()`: the live heap and its peak, the GPU memory
/// (JSON: `{"live":…,"peak":…,"wasm":…,"gpu_meshes":…,"gpu_buffers":…,
/// "gpu_textures":…}`, bytes; spec §14 Phase 10 B2).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = heap)]
pub fn page_heap() -> String {
    let g = gpu_memory::GpuMemory::now();
    serde_json::json!({
        "live": heap::live(),
        "peak": heap::peak(),
        "wasm": browser::memory_bytes(),
        "gpu_meshes": g.mesh_bytes,
        "gpu_buffers": g.slab_bytes,
        "gpu_textures": g.image_bytes,
        "meshes": g.meshes,
        "textures": g.images,
        "cpu_images": g.cpu_image_bytes,
        "cpu_meshes": g.cpu_mesh_bytes,
        "cpu_images_top": gpu_memory::cpu_images_top(),
        "trace": heap::trace_marks(),
    })
    .to_string()
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
    // Sign-in (spec §9.1): the backend's redirect brings the page back
    // with a code; the ride's options are the query it left with.
    let (query, account_start) = account::start_page(&query);
    let dev_page = is_dev_host(&host);
    let (mut config, mut warnings) = parse_query(&query, dev_page);
    config.access_token = account::tab_access_token();
    // The page configuration (config.json, B8a): the worlds' base, and on
    // a public page without a hosted server no multiplayer and no voice.
    let (page, page_error) = page_config::from_page();
    warnings.extend(page_error);
    page.apply(&mut config, dev_page, &mut warnings);
    // The settings read at the start, where the query leaves them out.
    let (settings, settings_warnings) = settings::WebSettings::load(settings::default_store());
    settings::apply_to_config(&settings, &mut config, &settings::query_keys(&query));
    // The world's options as the app sets them (a phone's residency and
    // course): the driver decodes the base with them.
    apply_memory_mode(&mut config, &device_info());
    let world = config.world.clone();
    let mut app = app_with_settings(config, settings, settings_warnings);
    app.insert_resource(account_start)
        .add_plugins(account::WebAccountPlugin);
    for w in warnings {
        warn!("page options: {w}");
    }
    if page.is_preview() {
        info!(
            "page: the public preview (worlds from {}; {}; {}; {})",
            page.worlds.as_deref().unwrap_or(world::WORLDS_BASE),
            page.multiplayer_off(dev_page)
                .unwrap_or("multiplayer on".into()),
            page.voice_off(dev_page).unwrap_or("voice on".into()),
            page.signin_off(dev_page).unwrap_or("sign-in on".into()),
        );
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

    /// A lost WebGL context stops the app once (the page loads the ride
    /// again): without the stop the wasm memory grew by ~45 MB a second
    /// until iOS killed the tab (2026-11-22).
    #[test]
    fn a_lost_gpu_stops_the_app_once() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_systems(First, stop_when_gpu_lost);
        app.update();
        assert!(app.world().resource::<Messages<AppExit>>().is_empty());
        mark_gpu_lost();
        app.update();
        assert_eq!(app.world().resource::<Messages<AppExit>>().len(), 1);
        assert!(!take_gpu_lost(), "the mark is used once");
    }

    /// A renderer error stops rendering and tells the page; the app is not
    /// quit (Bevy's default), so the page can fall back to WebGL 2 (B7).
    #[test]
    fn render_errors_go_to_the_page() {
        use bevy::render::error_handler::{ErrorType, RenderError, RenderErrorPolicy};
        let error = RenderError {
            ty: ErrorType::Validation,
            description: "In a set_pipeline command\n  more".into(),
            source: None,
        };
        assert_eq!(
            render_error_report(&error),
            (
                "Validation".to_string(),
                "In a set_pipeline command".to_string()
            )
        );
        let mut main = World::new();
        main.init_resource::<Messages<AppExit>>();
        let mut render = World::new();
        for _ in 0..2 {
            let policy = page_render_error(&error, &mut main, &mut render);
            assert!(matches!(policy, RenderErrorPolicy::StopRendering));
        }
        assert!(main.resource::<Messages<AppExit>>().is_empty());
        // The page's mark of a WebGPU fallback is a known option.
        let (_, w) = parse_query("route=hills&gpu=webgl2&quality=low&webgpu-failed=1", false);
        assert!(w.is_empty(), "{w:?}");
    }

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
        // A geographic origin for the test route (the GPS line, spec 3.40).
        let (c, w) = parse_query("route=loop&origin=47.13996,9.52151", false);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(c.route.geo_origin(), Some((47.13996, 9.52151)));
        assert_eq!(c.route.length(), Route::test_loop().length());
        let (c, w) = parse_query("origin=north", false);
        assert_eq!((w.len(), c.route.geo_origin()), (1, None));
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

    /// The quality preset for one visit and the page's build choice (B7).
    #[test]
    fn quality_and_gpu_options() {
        let (c, w) = parse_query("quality=low&gpu=webgl2", false);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(c.quality.as_deref(), Some("low"));
        let (c, w) = parse_query("quality=ultra&gpu=vulkan", false);
        assert_eq!(w.len(), 2, "{w:?}");
        assert!(c.quality.is_none());
        // `auto` here is the WebGPU build's: High on a desktop browser.
        assert_eq!(gpu_api(), ocw_client::quality::GpuApi::WebGpu);
        assert_eq!(device_info(), ocw_client::quality::DeviceInfo::default());
    }

    /// `?touch=on|off|auto` sets touch steering for the visit (spec
    /// §3.2.3), not saved.
    #[test]
    fn the_touch_option_sets_touch_steering_for_the_visit() {
        let (c, w) = parse_query("touch=on", false);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(c.touch.as_deref(), Some("on"));
        let (c, w) = parse_query("touch=maybe", false);
        assert_eq!((c.touch, w.len()), (None, 1));
        let (c, _) = parse_query("touch=off", false);
        let mut settings = settings::WebSettings::default();
        let mut warnings = Vec::new();
        visit_quality(&c, &mut settings, &mut warnings);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(settings.value("controls.touch_steer").to_string(), "off");
        assert!(
            !settings.settings.to_toml().contains("touch_steer"),
            "not saved"
        );
    }

    /// `?quality=low` runs Low for the visit without saving it; without
    /// it `auto` resolves for this build (WebGPU: High).
    #[test]
    fn the_page_option_sets_the_quality_for_the_visit() {
        let value = |query: &str| {
            let (config, _) = parse_query(query, false);
            let (mut s, mut w) = settings::WebSettings::load(None);
            visit_quality(&config, &mut s, &mut w);
            assert!(w.is_empty(), "{w:?}");
            let saved = s.settings.file_value("display.quality").is_some();
            (s.value("display.quality").to_string(), saved)
        };
        assert_eq!(value("quality=low"), ("low".to_string(), false));
        assert_eq!(value(""), ("auto".to_string(), false));
    }

    /// The pixel-ratio cap: a phone at 3 draws at 1 on Low; a screen within
    /// the cap and High keep the device's own.
    #[test]
    fn the_memory_mode_reaches_the_worlds_options_once() {
        // `memory=phone` on a desktop browser: the residency and the course
        // the driver gets (v5.42 regression: it got them unadjusted).
        let (mut config, _) = parse_query("world=liechtenstein&memory=phone", true);
        let desktop = ocw_client::quality::DeviceInfo::default();
        assert_eq!(
            apply_memory_mode(&mut config, &desktop),
            memory::MemoryMode::Phone
        );
        let w = config.world.clone().expect("a world");
        assert_eq!((w.residency.fetch_radius, w.residency.keep_radius), (2, 3));
        assert_eq!(w.course_rings, Some(4));
        apply_memory_mode(&mut config, &desktop);
        assert_eq!(config.world, Some(w), "a second time changes nothing");
        // A desktop without the option: the whole world.
        let (mut config, _) = parse_query("world=liechtenstein", true);
        assert_eq!(
            apply_memory_mode(&mut config, &desktop),
            memory::MemoryMode::Normal
        );
        assert_eq!(config.world.expect("a world").course_rings, None);
    }

    #[test]
    fn the_window_starts_at_the_pages_size() {
        // An iPhone's page: its CSS size, not 1280 × 720 (× 3 = 3840 ×
        // 2160 targets for the first frames).
        assert_eq!(start_size(393.0, 659.0), (393, 659));
        assert_eq!(start_size(2560.0, 1440.0), (1280, 1280));
        assert_eq!(start_size(0.0, 0.0), (1280, 720));
        assert_eq!(start_size(f64::NAN, 500.0), (1280, 720));
    }

    /// 14I W3: in VR the canvas gets the XR layer's pixels (the layer's
    /// render scale is the eyes'): a 1280 px wide page and a 1946 px layer
    /// framebuffer (two eyes at 0.8 of the Quest 1's 1216 px) draw at 1.52.
    #[test]
    fn the_canvas_takes_the_xr_layers_pixels() {
        let s = xr_canvas_scale(1280.0, 1946.0).unwrap();
        assert!((s - 1.5203).abs() < 1e-3, "{s}");
        assert_eq!(xr_canvas_scale(1280.0, 1280.0), Some(1.0));
        assert_eq!(xr_canvas_scale(0.0, 1946.0), None);
        assert_eq!(xr_canvas_scale(1280.0, 0.0), None);
        assert_eq!(xr_canvas_scale(10.0, 100_000.0), Some(8.0));
    }

    #[test]
    fn the_pixel_ratio_is_capped() {
        use ocw_render::quality::{QualityLevel, QualityPreset};
        let cap = |l| QualityPreset::of(l).max_pixel_ratio;
        assert_eq!(pixel_ratio_override(3.0, cap(QualityLevel::Low)), Some(1.0));
        assert_eq!(
            pixel_ratio_override(3.0, cap(QualityLevel::Medium)),
            Some(2.0)
        );
        assert_eq!(pixel_ratio_override(3.0, cap(QualityLevel::High)), None);
        assert_eq!(pixel_ratio_override(1.0, cap(QualityLevel::Low)), None);
        // WebKit sizes the canvas without devicePixelContentBoxSize: the
        // override would only shrink the UI (the iPhone, 2026-11-15).
        let low = cap(QualityLevel::Low);
        assert_eq!(usable_pixel_ratio_cap(low, true), Some(1.0));
        assert_eq!(usable_pixel_ratio_cap(low, false), None);
        assert_eq!(
            pixel_ratio_override(3.0, usable_pixel_ratio_cap(low, false)),
            None
        );
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
        assert!(id.ends_with(&format!(" web {GPU_API}")), "{id}");
        // Natively (the tests) neither browser build.
        assert_eq!(GPU_API, "native");
    }

    /// Ride → Join in the browser (spec §3.2.1): WebTransport links only,
    /// the page's query with the server, `join=` as a page option.
    #[test]
    fn join_links_in_the_browser() {
        let fp = "ef".repeat(32);
        let e = web_join(&format!("ocw://10.0.0.5?cert={fp}")).unwrap_err();
        assert_eq!(e, NO_WEBTRANSPORT);
        assert!(
            web_join("https://10.0.0.5")
                .unwrap_err()
                .contains("certificate hash")
        );
        let (r, _) = web_join(&format!("https://10.0.0.5:47801?cert-hash={fp}")).unwrap();
        assert_eq!(r.url, "https://10.0.0.5:47801/ocw/1");
        assert_eq!(r.cert_hashes, [[0xef; 32]]);
        // The page's other options stay; the link's world and route win.
        let q = join_query(
            "?world=old&route=x&bots=3&server=https://a",
            &format!(
                "https://10.0.0.5:47801?cert-hash={fp}&world=fiction/valley@2&route=Lake%20Loop"
            ),
        )
        .unwrap();
        assert_eq!(
            q,
            format!(
                "?bots=3&server=https://10.0.0.5:47801/ocw/1&cert-hash={fp}&world=fiction/valley@2&route=Lake%20Loop"
            )
        );
        assert_eq!(
            leave_query(&q),
            "?bots=3&world=fiction/valley@2&route=Lake%20Loop"
        );
        assert_eq!(leave_query("?server=x&cert-hash=y"), "");
        // join= at the start.
        let (c, w) = parse_query(
            &format!("join=https://10.0.0.5:47801?cert-hash={fp}%26route=loop"),
            false,
        );
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(c.remote.unwrap().url, "https://10.0.0.5:47801/ocw/1");
        assert_eq!(c.route.name(), Route::test_loop().name());
        let (c, w) = parse_query(&format!("join=ocw://h?cert={fp}"), false);
        assert!(
            c.remote.is_none() && w[0].contains("no WebTransport"),
            "{w:?}"
        );
    }
}
