//! The browser app's UI (spec §14 Phase 10, B6: UI parity): the same
//! menus, settings pages, time and weather machines, events, collisions,
//! rider list and unicorn mode (Phase 13 G5, [`crate::god`]) as the desktop (`ocw_ui`, over the session's client
//! through `ocw_ui::RideAccess`), with the browser's own pages through
//! [`ocw_ui::menu::MenuHooks`] – the world of the page and the bundled
//! workouts – and its captures as downloads instead of files:
//!
//! - **photos** (spec §2.11 Q3): screenshots (`screenshot`, Captures →
//!   Screenshot: the renderer's JPEG with the time in EXIF,
//!   `ocw_render::stamp`, named `OCW-2026-11-14_19-20-05.jpg`) and the ride
//!   photographer's photos (`OCW-<time>-photo-N.jpg`, EXIF time and GPS)
//!   go through [`PhotoSaver`]: into the photos folder the rider chose
//!   (Captures → Photos folder, `photos.folder choose`: the File System
//!   Access API's picker starting in Pictures, an `OCW` folder made in
//!   the chosen one, the handle kept in IndexedDB, the permission asked
//!   again with the next key or click after a reload), else as downloads;
//!   the HUD previews each for 3 s with where it went,
//! - **video** (`video start|stop|toggle`, Captures → Video): the page's
//!   `MediaRecorder` on the canvas (WebM, with the game sound when it
//!   plays), named the same way; the HUD shows REC as on the desktop,
//! - **the ride's FIT file** (`ride.save`, Captures → Download this ride):
//!   the shared recording (`ocw_client::recording`), `ocw-ride-<start>.fit`,
//! - **the simple Strava upload** (spec §2.8.1 *Simple upload*; a ride's
//!   page in the training history, `strava.upload-manual N`): the ride's
//!   FIT file as a download under its recognisable name
//!   (`ocw_client::strava::upload_name`) and Strava's upload page in a new
//!   tab ([`Tabs`]), with the steps on the ride's page and in the HUD.
//!
//! The downloads go through [`Downloads`]: the page's `window.ocwDownload`
//! in the browser, a list in the tests.

use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use clap::{Arg, Command};
use ocw_client::menu::pages::{self as sub, Row, WorldHome};
use ocw_client::recording::{LapKey, RideRecorder, download_name, observe_client, starts_lap};
use ocw_render::RideView;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, Permission};
use ocw_render::stamp::{MediaNow, ShotSink, capture_name};
use ocw_ui::menu::MenuHooks;

use crate::plugin::Session;

/// Where the browser's photos go (spec §2.11 Q3): (file name, MIME type,
/// bytes) → where they went, for the HUD's preview ("Saved to OCW",
/// "Downloaded OCW-….jpg").
#[derive(Resource, Clone)]
pub struct PhotoSaver(pub Arc<PhotoSaveFn>);

/// What [`PhotoSaver`] calls.
pub type PhotoSaveFn = dyn Fn(&str, &str, Vec<u8>) -> String + Send + Sync;

/// The preview's caption for a photo that went to `went` (the page's
/// answer: `folder:NAME` or `download`).
pub fn photo_went(went: &str, name: &str) -> String {
    match went.strip_prefix("folder:") {
        Some(folder) => format!("Saved to {folder}"),
        None => format!("Downloaded {name}"),
    }
}

impl PhotoSaver {
    /// Downloads only (the tests, a page without the folder API).
    pub fn downloads(d: Downloads) -> Self {
        Self(Arc::new(move |name, mime, bytes| {
            (d.0)(name, mime, bytes);
            photo_went("download", name)
        }))
    }

    /// The page's photos folder, else a download.
    #[cfg(target_arch = "wasm32")]
    fn page() -> Self {
        Self(Arc::new(|name, mime, bytes| {
            photo_went(&crate::browser::photo_save(name, mime, &bytes), name)
        }))
    }
}

/// The ride photographer's photo in the browser (spec §2.11 Q3): a JPEG
/// with EXIF time and GPS, `OCW-<time>-photo-N.jpg`, through the
/// [`PhotoSaver`] (encoded here on the page's thread).
fn save_ride_photo(
    world: &mut World,
    t: &ocw_client::photographer::Taken,
    image: Image,
) -> Option<String> {
    let saver = world.get_resource::<PhotoSaver>()?.clone();
    let now = world
        .get_resource::<MediaNow>()
        .map(|m| m.0)
        .unwrap_or_default();
    let software = ocw_render::media::software(
        world
            .get_resource::<ocw_render::hud::VersionLabel>()
            .map_or("", |l| l.0.as_str()),
    );
    let gps = ocw_ui::client(world)
        .and_then(|c| c.geo_origin())
        .map(|(lat0, lon0)| {
            let (lat, lon) = ocw_world::geo::LocalProjection::new(lat0, lon0)
                .unproject([t.pose.eye[0], t.pose.eye[2]]);
            ocw_render::media::GpsPosition {
                lat,
                lon,
                height_m: t.pose.eye[1],
            }
        });
    let rgb = match image.try_into_dynamic() {
        Ok(i) => i.to_rgb8(),
        Err(e) => {
            error!("ride photo: cannot convert the image: {e}");
            return None;
        }
    };
    let jpg = ocw_render::media::encode_jpeg(
        rgb.as_raw(),
        rgb.width(),
        rgb.height(),
        92,
        ocw_render::media::exif_tiff_gps(&now, &software, gps),
    )
    .map_err(|e| error!("ride photo: {e}"))
    .ok()?;
    let name = format!(
        "OCW-{}-photo-{}.jpg",
        ocw_render::stamp::stamp_of(&now),
        t.n
    );
    info!(file = %name, bytes = jpg.len(), "ride photo");
    Some((saver.0)(&name, "image/jpeg", jpg))
}

/// Where downloads go: (file name, MIME type, bytes).
#[derive(Resource, Clone)]
pub struct Downloads(pub Arc<DownloadFn>);

/// What [`Downloads`] calls: (file name, MIME type, bytes).
pub type DownloadFn = dyn Fn(&str, &str, Vec<u8>) + Send + Sync;

/// The downloads of a test (name, MIME type, bytes).
pub type DownloadLog = Arc<Mutex<Vec<(String, String, Vec<u8>)>>>;

impl Downloads {
    /// Downloads that are kept in `log` (the tests).
    pub fn logged(log: DownloadLog) -> Self {
        Self(Arc::new(move |name, mime, bytes| {
            log.lock()
                .expect("not poisoned")
                .push((name.into(), mime.into(), bytes));
        }))
    }

    fn page() -> Self {
        #[cfg(target_arch = "wasm32")]
        return Self(Arc::new(|name, mime, bytes| {
            crate::browser::download(name, mime, &bytes)
        }));
        #[cfg(not(target_arch = "wasm32"))]
        Self(Arc::new(|name, _, bytes| {
            info!("download {name}: {} bytes (no page)", bytes.len());
        }))
    }
}

/// Opens a page in a new tab (the page's `window.open`; a list in the
/// tests). `Err`: the browser blocked it.
#[derive(Resource, Clone)]
pub struct Tabs(pub Arc<TabFn>);

/// What [`Tabs`] calls with the URL.
pub type TabFn = dyn Fn(&str) -> Result<(), String> + Send + Sync;

impl Tabs {
    /// Tabs that are kept in `log` (the tests).
    pub fn logged(log: Arc<Mutex<Vec<String>>>) -> Self {
        Self(Arc::new(move |url| {
            log.lock().expect("not poisoned").push(url.into());
            Ok(())
        }))
    }

    fn page() -> Self {
        #[cfg(target_arch = "wasm32")]
        return Self(Arc::new(crate::browser::open_tab));
        #[cfg(not(target_arch = "wasm32"))]
        Self(Arc::new(|url| {
            info!("open {url} (no page)");
            Ok(())
        }))
    }
}

/// The HUD's Strava notice in the browser (S: the desktop app; the
/// upload's steps) and until when it shows (game time).
#[derive(Resource, Default)]
pub struct StravaNotice(pub Option<(String, f64)>);

/// The simple Strava upload of a history ride in the browser: the FIT
/// download under its recognisable name, Strava's upload page in a new
/// tab, the steps (the menu shows them on the ride's page, the HUD too).
fn web_upload(
    world: &mut World,
    ride: &ocw_client::history::RideEntry,
    _whole: bool,
) -> Result<String, String> {
    let bytes = world
        .get_resource::<ocw_ui::history::TrainingHistory>()
        .and_then(|h| h.fits.get(&ride.id).cloned())
        .ok_or("this ride's FIT file is not kept (only the rides of this page)")?;
    let name = ocw_client::strava::upload_name(
        ride.start_unix_s,
        ocw_ui::history::offset(ride.start_unix_s),
        &ride.world,
        &ride.route,
        None,
    );
    let downloads = world.resource::<Downloads>().clone();
    (downloads.0)(&name, "application/vnd.ant.fit", bytes);
    let tabs = world.resource::<Tabs>().clone();
    let mut steps = ocw_client::strava::upload_steps(std::slice::from_ref(&name), false, false);
    if let Err(e) = (tabs.0)(ocw_client::strava::UPLOAD_PAGE) {
        warn!("Strava's upload page: {e}");
        steps = format!(
            "Open {} in a new tab (the browser blocked it): sign in, choose the file {name} (just downloaded), then Upload.",
            ocw_client::strava::UPLOAD_PAGE
        );
    }
    let now = world
        .get_resource::<Time>()
        .map_or(0.0, |t| t.elapsed_secs_f64());
    world.resource_mut::<StravaNotice>().0 =
        Some((steps.clone(), now + ocw_client::strava::UPLOAD_NOTICE_S));
    Ok(steps)
}

/// The ride's recording for the FIT download (setting `ride.record`).
#[derive(Resource)]
pub struct WebRecording {
    pub recorder: RideRecorder,
    pub enabled: bool,
    last_lap: LapKey,
}

/// A video being recorded: its name and when it started (game time).
#[derive(Resource, Default)]
pub struct Video(pub Option<(String, f64)>);

pub struct WebUiPlugin {
    /// Record the ride (setting `ride.record`).
    pub record: bool,
}

impl Default for WebUiPlugin {
    fn default() -> Self {
        Self { record: true }
    }
}

impl Plugin for WebUiPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(MenuHooks {
            rows: web_rows,
            ..MenuHooks::default()
        })
        // The training history in the page's localStorage (spec §14
        // Phase 7); a ride's FIT file is kept for this page's session.
        .insert_resource(ocw_ui::history::HistoryHooks {
            store: |_, _| crate::settings::history_store(),
            rider: |_| ocw_client::riders::Rider::default().name,
            export: |world, ride, _| {
                let bytes = world
                    .get_resource::<ocw_ui::history::TrainingHistory>()
                    .and_then(|h| h.fits.get(&ride.id).cloned())
                    .ok_or("this ride's FIT file is not kept (only the rides of this page)")?;
                let name = download_name(ride.start_unix_s);
                let downloads = world.resource::<Downloads>().clone();
                (downloads.0)(&name, "application/vnd.ant.fit", bytes);
                Ok(format!("downloading {name}"))
            },
            import: None,
            refresh: ocw_ui::history::refresh_from_memory,
            // The simple Strava upload (spec §2.8.1); the mark is kept in
            // the history only (no API sync in the browser).
            upload: Some(web_upload),
            marked: None,
        })
        .add_plugins((
            ocw_ui::menu::MenuPlugin,
            ocw_ui::weather::WeatherPlugin,
            ocw_ui::timemachine::TimeMachinePlugin,
            ocw_ui::riders_page::RidersPagePlugin,
            ocw_ui::collisions::CollisionsPlugin,
            ocw_ui::events::EventsPlugin,
            // The rider status signs (spec §2.6), as on the desktop.
            ocw_ui::status::StatusPlugin,
            // `display.quality`, `quality` and `gpu` (B7).
            ocw_ui::quality::QualityPlugin,
            // Unicorn mode (spec §14 Phase 13 G5) with the page's Pointer
            // Lock and `window.ocw.god()`.
            crate::god::WebGodPlugin,
            // Touch steering (spec §3.2.3), as on the desktop.
            ocw_ui::touch::TouchSteerPlugin,
        ));
        // The ride photographer (spec §2.11): as on the desktop – its own
        // photos go to the photos folder or downloads (Q3, v4.73); the
        // other riders' photographers, drones and flashes are drawn and
        // heard (Q2, v4.38).
        app.add_plugins(ocw_ui::photographer::PhotographerPlugin { scale: 1.0 })
            .insert_resource(ocw_ui::photographer::PhotographerHooks {
                save: save_ride_photo,
                sound: |world, msg| {
                    if let Some(mut a) = world.get_resource_mut::<crate::audio::WebAudio>() {
                        a.apply(vec![msg]);
                    }
                },
            });
        if !app.world().contains_resource::<Downloads>() {
            app.insert_resource(Downloads::page());
        }
        if !app.world().contains_resource::<Tabs>() {
            app.insert_resource(Tabs::page());
        }
        app.init_resource::<StravaNotice>();
        if !app.world().contains_resource::<PhotoSaver>() {
            #[cfg(target_arch = "wasm32")]
            app.insert_resource(PhotoSaver::page());
            #[cfg(not(target_arch = "wasm32"))]
            app.insert_resource(PhotoSaver::downloads(
                app.world().resource::<Downloads>().clone(),
            ));
        }
        let saver = app.world().resource::<PhotoSaver>().clone();
        app.insert_resource(ShotSink(Arc::new(move |name, bytes| {
            let mime = if name.ends_with(".png") {
                "image/png"
            } else {
                "image/jpeg"
            };
            (saver.0)(name, mime, bytes)
        })))
        .insert_resource(WebRecording {
            recorder: RideRecorder::new("Open Cycling World"),
            enabled: self.record,
            last_lap: LapKey::default(),
        })
        .init_resource::<Video>()
        .add_systems(
            Update,
            (record, show_video, audio_devices, strava_key).after(ocw_ui::RideViewSet),
        )
        .add_systems(Last, (publish, publish_console, publish_steer, publish_map));
        // The trusted world keys in the page's localStorage (spec §4.7
        // *User keys*); the built-in roots cannot be added.
        // (Always: the menu plugin above put an empty default there.)
        {
            let mut builtin = ocw_worldstore::trust::TrustConfig {
                dev_roots: true,
                ..Default::default()
            }
            .builtin_store()
            .unwrap_or_default();
            builtin.roots.extend(ocw_worldstore::trust::preview_roots());
            app.insert_resource(ocw_ui::trust::TrustedKeys::load(
                crate::settings::trusted_keys_store(),
                builtin,
            ));
        }
        // Ride → Join (spec §3.2.1): the recent servers in localStorage;
        // joining starts the page again on the shared ride (WebTransport).
        app.insert_resource(ocw_ui::join::JoinUi::load(crate::settings::joins_store()))
            .insert_resource(ocw_ui::join::JoinHooks {
                connect: web_connect,
                cancel: |_| {},
                leave: web_leave,
                paste: |_| crate::PASTED.lock().expect("not poisoned").take(),
            })
            .add_systems(Update, ride_mode);
        register_commands(app);
    }
}

/// The page's query now (empty natively).
fn page_query() -> String {
    #[cfg(target_arch = "wasm32")]
    return web_sys::window()
        .and_then(|w| w.location().search().ok())
        .unwrap_or_default();
    #[cfg(not(target_arch = "wasm32"))]
    String::new()
}

/// Opens the page with `query` (the ride starts again with it).
fn open_page(query: &str) -> Result<(), String> {
    #[cfg(target_arch = "wasm32")]
    {
        let l = web_sys::window().ok_or("no window")?.location();
        let path = l.pathname().unwrap_or_default();
        l.assign(&format!("{path}{query}"))
            .map_err(|_| "the page cannot be opened".to_string())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = query;
        Ok(())
    }
}

/// The page that joins (the remembered list first: the page starts again).
fn web_connect(world: &mut World, link: ocw_protocol::join::JoinLink) -> Result<String, String> {
    let query = crate::join_query(&page_query(), &link.to_link())?;
    world.resource_mut::<ocw_ui::join::JoinUi>().remember(
        &link,
        link.world
            .as_deref()
            .or(link.route.as_deref())
            .unwrap_or(""),
    );
    info!(query = %query, "joining: the page starts again on the shared ride");
    open_page(&query)?;
    Ok(format!(
        "joining {}: the page starts again on the shared ride",
        link.server_spec()
    ))
}

fn web_leave(world: &mut World) -> Result<String, String> {
    let remote = world
        .get_resource::<crate::plugin::Session>()
        .is_some_and(|s| s.0.is_remote());
    if !remote {
        return Err("not in a shared ride".into());
    }
    open_page(&crate::leave_query(&page_query()))?;
    Ok("leaving the shared ride: the page starts again alone".into())
}

/// The Ride page's items: Leave in a shared ride.
fn ride_mode(
    session: Option<Res<crate::plugin::Session>>,
    mut mode: ResMut<ocw_ui::menu::MenuRideMode>,
) {
    use ocw_client::menu::model::RideMode;
    let m = if session.is_some_and(|s| s.0.is_remote()) {
        RideMode::Joined
    } else {
        RideMode::Alone
    };
    if mode.0 != m {
        mode.0 = m;
    }
}

/// The menu as JSON for the page (`window.ocw.menu()`).
static MENU_JSON: Mutex<String> = Mutex::new(String::new());

/// The settings' text values for the page (`window.ocw.setting(key)`).
static SETTINGS_TEXT: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

/// Touch steering's state for the page (`window.ocw.steer()`).
static STEER_JSON: Mutex<String> = Mutex::new(String::new());

/// The touch steering state as JSON (spec §3.2.3: the browser test).
pub fn steer_json() -> String {
    STEER_JSON.lock().expect("not poisoned").clone()
}

fn publish_steer(world: &mut World) {
    *STEER_JSON.lock().expect("not poisoned") = ocw_ui::touch::state_json(world);
}

/// The menu as it shows now, as JSON.
pub fn menu_json() -> String {
    MENU_JSON.lock().expect("not poisoned").clone()
}

/// A setting's value as text ("" if unknown or not loaded yet).
pub fn setting_text(key: &str) -> String {
    SETTINGS_TEXT
        .lock()
        .expect("not poisoned")
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.clone())
        .unwrap_or_default()
}

/// The menu's view as JSON (open, title, status, text, items, focused,
/// hint, dimmed).
pub fn view_json(v: &ocw_render::menu::MenuView) -> String {
    serde_json::json!({
        "open": v.open,
        "title": v.title,
        "status": v.status,
        "text": v.text,
        "items": v.items,
        "focused": v.focused,
        "hint": v.hint,
        "dimmed": v.dimmed,
        "emphasis": v.emphasis,
        "grid": v.grid.as_ref().map(|g| g.columns),
        // The calendar's month and its marked days (spec §14 Phase 9 S9a).
        "calendar": v.calendar.as_ref().map(|c| serde_json::json!({
            "title": c.title,
            "selected": c.days.iter().position(|d| d.selected),
            "bands": c.days.iter().filter(|d| d.band.is_some()).count(),
            "moon": c.days.iter().filter(|d| d.moon.is_some()).count(),
        })),
    })
    .to_string()
}

/// The console and the HUD's GPS line as JSON for the page
/// (`window.ocw.consoleState()`; the browser test's Tab completion and GPS
/// checks, spec 3.40).
static CONSOLE_JSON: Mutex<String> = Mutex::new(String::new());

/// The console's state as JSON: open, the input line, the completion
/// candidates and the selected one, the HUD's GPS line (null: hidden).
pub fn console_json() -> String {
    CONSOLE_JSON.lock().expect("not poisoned").clone()
}

/// The console's JSON (see [`console_json`]).
pub fn console_view_json(
    console: &ocw_render::console::ConsoleState,
    gps: Option<&ocw_render::hud::HudGpsLine>,
) -> String {
    let (candidates, selected) = console.candidates();
    serde_json::json!({
        "open": console.is_open(),
        "input": console.input(),
        "candidates": candidates,
        "selected": selected,
        "gps": gps.and_then(|g| g.0.clone()),
        "log": console.log().iter().rev().take(20).rev().collect::<Vec<_>>(),
    })
    .to_string()
}

fn publish_console(
    console: Option<Res<ocw_render::console::ConsoleState>>,
    gps: Option<Res<ocw_render::hud::HudGpsLine>>,
) {
    let Some(console) = console else { return };
    let changed = console.is_changed() || gps.as_ref().is_some_and(|g| g.is_changed());
    if changed || CONSOLE_JSON.lock().expect("not poisoned").is_empty() {
        *CONSOLE_JSON.lock().expect("not poisoned") = console_view_json(&console, gps.as_deref());
    }
}

/// The HUD's navigation map as JSON for the page (`window.ocw.hudMap()`:
/// the Devices panel steps above it; the browser test checks it).
static MAP_JSON: Mutex<String> = Mutex::new(String::new());

pub fn map_json() -> String {
    MAP_JSON.lock().expect("not poisoned").clone()
}

/// The map's JSON: shown, its box (CSS px: left, top, width, height),
/// the banner, the zoom, day or night, the tiles drawn so far.
pub fn map_view_json(info: &ocw_render::navi_map::NaviMapInfo) -> String {
    serde_json::json!({
        "shown": info.rect.is_some(),
        "rect": info.rect,
        "banner": info.banner,
        "level": info.level,
        "auto": info.auto,
        "night": info.shade == ocw_render::navi_map::Shade::Night,
        "tiles": info.tiles_drawn,
    })
    .to_string()
}

fn publish_map(info: Option<Res<ocw_render::navi_map::NaviMapInfo>>) {
    let Some(info) = info else { return };
    if info.is_changed() || MAP_JSON.lock().expect("not poisoned").is_empty() {
        *MAP_JSON.lock().expect("not poisoned") = map_view_json(&info);
    }
}

/// Publishes the menu and the settings for the page when they change.
fn publish(
    view: Res<ocw_render::menu::MenuView>,
    settings: Option<Res<crate::settings::WebSettings>>,
) {
    if view.is_changed() || MENU_JSON.lock().expect("not poisoned").is_empty() {
        *MENU_JSON.lock().expect("not poisoned") = view_json(&view);
    }
    if let Some(s) = settings
        && s.is_changed()
    {
        *SETTINGS_TEXT.lock().expect("not poisoned") = ocw_client::settings::SETTINGS
            .iter()
            .map(|d| (d.key.to_string(), s.value(d.key).to_string()))
            .collect();
    }
}

/// The outputs for Settings → Audio (the page's `setSinkId` list), once
/// a second.
fn audio_devices(
    time: Res<Time>,
    audio: Option<ResMut<crate::audio::WebAudio>>,
    settings: Option<Res<crate::settings::WebSettings>>,
    mut menu: ResMut<ocw_ui::menu::AudioDeviceMenu>,
    mut next: Local<f64>,
) {
    let now = time.elapsed_secs_f64();
    let Some(mut audio) = audio else { return };
    if now < *next {
        return;
    }
    *next = now + 1.0;
    let names = audio.devices();
    let saved = settings
        .map(|s| s.value("audio.output").to_string())
        .filter(|s| s != "default");
    let new = ocw_client::menu::pages::AudioDevices {
        fell_back: saved.as_ref().is_some_and(|s| !names.contains(s)),
        current: saved,
        names,
    };
    if menu.0 != new {
        menu.0 = new;
    }
}

/// The browser's own pages: the World page of the page's world (no
/// switching in the ride: another world is another page), the bundled
/// workouts.
fn web_rows(world: &World, page: sub::Page) -> Option<Vec<Row>> {
    match page {
        sub::Page::WorldHome => {
            let name = world
                .get_resource::<crate::world::WorldStream>()
                .and_then(|s| s.identity.clone())
                .or_else(|| {
                    world
                        .get_resource::<RideView>()
                        .map(|v| v.route.name().to_string())
                })
                .unwrap_or_default();
            let tm = world
                .get_resource::<ocw_ui::timemachine::TimeMachine>()
                .map(|t| t.0.clone())
                .unwrap_or_default();
            let weather = world
                .get_resource::<ocw_ui::weather::WeatherMenu>()
                .map(|w| w.0.clone())
                .unwrap_or_default();
            let home = WorldHome {
                name,
                // Unicorn mode (spec §14 Phase 13 G5): as on the desktop.
                god: Some(ocw_ui::god::MENU_NAME),
                ..WorldHome::default()
            };
            Some(sub::world_home_rows(&home, &tm, &weather))
        }
        sub::Page::Workouts => {
            let now = world
                .get_resource::<Session>()
                .and_then(|s| s.0.devices.workout.now());
            Some(sub::workout_rows(now.as_ref(), &sub::example_workouts()))
        }
        // The desktop's files, switches and profiles are not in the
        // browser's menu (`ocw_client::menu::model::Platform`).
        p if p.is_ride_page() => Some(Vec::new()),
        _ => None,
    }
}

fn unix_now() -> i64 {
    web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Records the ride once a second; FIT laps at workout steps and event
/// parts (as on the desktop).
fn record(time: Res<Time>, session: Option<Res<Session>>, mut rec: ResMut<WebRecording>) {
    let Some(session) = session else { return };
    if !rec.enabled {
        return;
    }
    let s = &session.0;
    let now = time.elapsed_secs_f64();
    observe_client(&mut rec.recorder, &s.client, now, unix_now());
    let key = (s.devices.workout.lap_count(), s.client.event_lap_segment());
    if starts_lap(rec.last_lap, key) {
        rec.recorder.mark_lap(now);
    }
    rec.last_lap = key;
}

/// REC in the HUD while a video is recorded (as on the desktop).
fn show_video(time: Res<Time>, video: Res<Video>, mut view: ResMut<RideView>) {
    let rec = video.0.as_ref().map(|(_, t0)| time.elapsed_secs_f64() - t0);
    if view.recording_s != rec {
        view.recording_s = rec;
    }
}

/// What the Strava key (S, spec §2.8.1) says in the browser.
pub const STRAVA_DESKTOP_ONLY: &str = "The Strava recording (S) needs the desktop app. Upload a ride: Navi > Training history > the ride";

/// S (or its pad button) in the browser: the notice that the Strava
/// recording needs the desktop app, for a few seconds; the HUD shows the
/// notice (that one or the upload's steps) while it lasts.
fn strava_key(
    time: Res<Time>,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    pads: Query<&Gamepad>,
    bindings: Option<Res<ocw_ui::bindings::ActiveBindings>>,
    context: Option<Res<ocw_render::menu::InputContext>>,
    mut notice: ResMut<StravaNotice>,
    mut view: ResMut<RideView>,
) {
    use ocw_client::bindings::Action;
    let now = time.elapsed_secs_f64();
    if let Some(keys) = keys
        && ocw_render::menu::ride_input(context)
        && (ocw_ui::bindings::key_just_pressed(bindings.as_deref(), Action::Strava, &keys)
            || ocw_ui::bindings::pad_just_pressed(bindings.as_deref(), Action::Strava, pads.iter()))
    {
        notice.0 = Some((
            STRAVA_DESKTOP_ONLY.to_string(),
            now + ocw_client::strava::NOTICE_S,
        ));
    }
    let msg = notice
        .0
        .as_ref()
        .filter(|(_, until)| now < *until)
        .map(|(m, _)| m.clone());
    if view.strava_message != msg {
        view.strava_message = msg;
    }
}

/// `ride.save`: the ride so far as a FIT download; a new recording starts.
fn save_ride(world: &mut World) -> Result<String, String> {
    let ride = world
        .resource_mut::<WebRecording>()
        .recorder
        .finish()
        .ok_or("nothing to save yet (a ride needs two records, one a second)")?;
    let name = download_name(ride.activity.start_unix_s);
    let bytes = ocw_fit::encode(&ride.activity);
    let (n, km) = (
        ride.activity.records.len(),
        ride.activity.records.last().map_or(0.0, |r| r.distance_m) / 1000.0,
    );
    let downloads = world.resource::<Downloads>().clone();
    (downloads.0)(&name, "application/vnd.ant.fit", bytes.clone());
    // The training history (spec §14 Phase 7), with the world of the page.
    let world_name = world
        .get_resource::<crate::world::WorldStream>()
        .and_then(|s| s.identity.clone())
        .unwrap_or_default();
    let route = world
        .get_resource::<RideView>()
        .map(|v| v.route.name().to_string())
        .unwrap_or_default();
    let entry = ocw_client::history::RideEntry::from_activity(
        &ride.activity,
        ocw_client::history::RideContext {
            id: ocw_client::recording::file_stem(ride.activity.start_unix_s),
            world: world_name,
            route,
            ..Default::default()
        },
    );
    let kept = match ocw_ui::history::add_ride(world, entry, Some(bytes)) {
        Ok(()) => "",
        Err(_) => " (the training history is not saved on this page)",
    };
    Ok(format!(
        "ride saved: {name} ({n} records, {km:.2} km); a new recording starts{kept}"
    ))
}

/// `video …`: the page's recorder on the canvas.
fn video(world: &mut World, action: &str) -> Result<String, String> {
    let recording = world.resource::<Video>().0.clone();
    let start = match action {
        "start" => true,
        "stop" => false,
        "status" => {
            return Ok(match &recording {
                Some((name, _)) => format!("recording {name}"),
                None => "no video is being recorded".into(),
            });
        }
        _ => recording.is_none(),
    };
    match (start, recording) {
        (true, Some((name, _))) => Ok(format!("already recording {name}")),
        (false, None) => Err("no video is being recorded".into()),
        (true, None) => {
            let t = world
                .get_resource::<MediaNow>()
                .copied()
                .unwrap_or_default();
            let name = capture_name(&t.0, "webm");
            start_video(&name)?;
            let now = world.resource::<Time>().elapsed_secs_f64();
            world.resource_mut::<Video>().0 = Some((name.clone(), now));
            Ok(format!("recording video {name}"))
        }
        (false, Some((name, _))) => {
            stop_video()?;
            world.resource_mut::<Video>().0 = None;
            Ok(format!("video {name}: downloading"))
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn start_video(name: &str) -> Result<(), String> {
    crate::browser::video_start(name)
}

#[cfg(target_arch = "wasm32")]
fn stop_video() -> Result<(), String> {
    crate::browser::video_stop()
}

#[cfg(not(target_arch = "wasm32"))]
fn start_video(_name: &str) -> Result<(), String> {
    Err("no video recorder outside the browser".into())
}

#[cfg(not(target_arch = "wasm32"))]
fn stop_video() -> Result<(), String> {
    Err("no video recorder outside the browser".into())
}

/// `photos.folder choose|forget|status`.
fn photos_folder(action: &str) -> Result<String, String> {
    #[cfg(target_arch = "wasm32")]
    {
        if !crate::browser::photo_folder_supported() {
            return Ok(
                "photos are downloads named OCW-… (this browser cannot keep a folder)".into(),
            );
        }
        match action {
            "choose" => {
                crate::browser::photo_folder_choose()?;
                return Ok(
                    "choose the photos folder (Pictures; an OCW folder is made in it)".into(),
                );
            }
            "forget" => {
                crate::browser::photo_folder_forget()?;
                return Ok("photos are downloads again".into());
            }
            _ => {}
        }
        Ok(match crate::browser::photo_folder() {
            Some(f) => format!("photos go to the folder {f}"),
            None => "photos are downloads named OCW-… (Captures -> Photos folder chooses a folder)"
                .into(),
        })
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = action;
        Ok("photos are downloads named OCW-… (no page)".into())
    }
}

fn register_commands(app: &mut App) {
    // `world trust …` (spec §4.7 *User keys*) and the trust dialog's lines
    // for the streamed world.
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("world")
            .about("The world keys you trust (the page's world: World -> About)")
            .subcommand_required(true)
            .subcommand(ocw_ui::trust::command())
            .subcommand(
                Command::new("continue")
                    .hide(true)
                    .about("Go on streaming the world (its key is trusted now)"),
            )
            .subcommand(
                Command::new("stop")
                    .hide(true)
                    .about("Do not load the world (its key was not trusted)"),
            ),
        |world, m| match m.subcommand() {
            Some(("trust", t)) => ocw_ui::trust::run(world, t),
            Some(("continue", _)) => crate::world::answer_trust(world, true),
            Some(("stop", _)) => crate::world::answer_trust(world, false),
            _ => Err("world trust list | add KEY | remove FINGERPRINT".into()),
        },
    ));
    // The photos folder (spec §2.11 Q3): the File System Access API.
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("photos.folder")
            .about("Where photos go: choose (a folder, the picker starts in Pictures; an OCW folder is made in it), forget (downloads), or nothing to show it")
            .arg(Arg::new("action").value_parser(["choose", "forget", "status"])),
        |_world, m| photos_folder(m.get_one::<String>("action").map_or("status", String::as_str)),
    ));
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("ride.save").about(
            "Download the ride so far as a FIT file (named with its start); a new recording starts",
        ),
        |world, _| save_ride(world),
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("video")
            .about(
                "Record a video of the ride (WebM, with the game sound), downloaded when it stops",
            )
            .arg(
                Arg::new("action")
                    .value_parser(["start", "stop", "toggle", "status"])
                    .default_value("toggle"),
            ),
        |world, m| {
            let a = m.get_one::<String>("action").expect("has default").clone();
            video(world, &a)
        },
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::SessionConfig;
    use ocw_render::console::{ConsoleCorePlugin, ConsoleState};
    use ocw_render::menu::MenuView;

    /// The browser's session and UI without a window: the menu by keys,
    /// the downloads into a list.
    fn app(log: DownloadLog) -> App {
        app_with(log, SessionConfig::default())
    }

    fn app_with(log: DownloadLog, config: SessionConfig) -> App {
        let mut app = App::new();
        // No InputPlugin: the tests press keys between the frames (its
        // PreUpdate would clear them first).
        app.add_plugins((MinimalPlugins, ConsoleCorePlugin))
            .init_resource::<ButtonInput<KeyCode>>()
            .insert_resource(Time::<Fixed>::from_hz(f64::from(ocw_core::SIM_TICK_HZ)))
            .insert_resource(RideView::new(ocw_world::Route::test_hills()))
            .insert_resource(Downloads::logged(log))
            .add_plugins(crate::plugin::WebSessionPlugin { config })
            .add_plugins((crate::settings::WebSettingsPlugin, WebUiPlugin::default()));
        app.update();
        app
    }

    /// Spec 3.40: Tab completion with the shared value providers (the
    /// browser has the desktop's `weather`, `time`, `set`), and the GPS
    /// position of a route with a geographic origin (`?origin=`).
    #[test]
    fn tab_completion_and_the_gps_position_in_the_browser() {
        let config = SessionConfig {
            route: ocw_world::Route::test_hills().with_geo_origin((47.13996, 9.52151)),
            ..SessionConfig::default()
        };
        let mut app = app_with(DownloadLog::default(), config);
        run_for(&mut app, 600);
        let tab = |app: &mut App, line: &str| {
            app.world_mut()
                .resource_mut::<ConsoleState>()
                .set_input(line);
            ocw_render::console::tab_complete(app.world_mut(), line.len(), false);
            app.update();
            let s = app.world().resource::<ConsoleState>();
            (s.input().to_owned(), s.candidates().0.to_vec())
        };
        assert_eq!(tab(&mut app, "weather thunder").0, "weather thunderstorm ");
        assert_eq!(tab(&mut app, "time golden").0, "time golden-hour ");
        assert_eq!(tab(&mut app, "set display.gp").0, "set display.gps ");
        assert_eq!(tab(&mut app, "set display.gps o").1, ["off", "on"]);
        assert_eq!(tab(&mut app, "rider.g").0, "rider.gps ");
        // Dev commands run on the browser's local server: offered.
        assert_eq!(tab(&mut app, "telep").0, "teleport ");
        let json: serde_json::Value = serde_json::from_str(&console_view_json(
            app.world().resource::<ConsoleState>(),
            None,
        ))
        .unwrap();
        assert_eq!(json["input"], "teleport ");
        // The GPS position: in the view and from `rider.gps`.
        let (lat, lon, _) = app.world().resource::<RideView>().position.expect("GPS");
        assert!((lat - 47.14).abs() < 0.05 && (lon - 9.52).abs() < 0.05);
        let out = console(&mut app, "rider.gps");
        assert!(out.contains("° N") && out.contains("° E"), "{out}");
    }

    fn run_for(app: &mut App, ms: u64) {
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_millis(ms) {
            app.update();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    fn tap(app: &mut App, key: KeyCode) {
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(key);
        app.update();
        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        keys.release(key);
        keys.clear();
        app.update();
    }

    fn view(app: &App) -> MenuView {
        app.world().resource::<MenuView>().clone()
    }

    fn console(app: &mut App, line: &str) -> String {
        app.world_mut().resource_mut::<ConsoleState>().queue(line);
        app.update();
        app.update();
        app.world().resource::<ConsoleState>().log().join("\n")
    }

    /// S in the browser (spec §2.8.1): the notice that Strava needs the
    /// desktop app.
    #[test]
    fn the_strava_key_says_it_needs_the_desktop_app() {
        let mut app = app(DownloadLog::default());
        run_for(&mut app, 200);
        assert_eq!(app.world().resource::<RideView>().strava_message, None);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyS);
        app.update();
        assert_eq!(
            app.world().resource::<RideView>().strava_message.as_deref(),
            Some(STRAVA_DESKTOP_ONLY)
        );
        assert_eq!(
            app.world().resource::<RideView>().strava_s,
            None,
            "no recording"
        );
    }

    #[test]
    fn the_browser_rides_with_the_desktops_menu() {
        let log = DownloadLog::default();
        let mut app = app(log.clone());
        run_for(&mut app, 600);
        // The sky and the weather from the shared machines.
        let rv = app.world().resource::<RideView>();
        assert!(rv.sky.is_some(), "the shared time machine");
        assert!(rv.weather.is_some(), "the shared weather");
        // Esc: the menu; World → Weather → the weather machine.
        tap(&mut app, KeyCode::Escape);
        assert_eq!(view(&app).title, "Open Cycling World");
        tap(&mut app, KeyCode::ArrowDown);
        tap(&mut app, KeyCode::ArrowDown);
        tap(&mut app, KeyCode::Enter);
        let v = view(&app);
        assert_eq!(v.title, "World");
        assert_eq!(v.items[0], "World:  Test Hills");
        assert!(v.items[1].starts_with("Time:"), "{:?}", v.items);
        assert!(v.items[2].starts_with("Weather:"), "{:?}", v.items);
        tap(&mut app, KeyCode::ArrowDown);
        tap(&mut app, KeyCode::ArrowDown);
        tap(&mut app, KeyCode::Enter);
        let v = view(&app);
        assert_eq!(v.title, "Weather machine");
        assert!(v.status.ends_with("set by you"), "{}", v.status);
        // Presets → hail: the server's weather changes.
        tap(&mut app, KeyCode::ArrowDown);
        tap(&mut app, KeyCode::ArrowDown);
        tap(&mut app, KeyCode::Enter);
        assert_eq!(view(&app).title, "Weather presets");
        tap(&mut app, KeyCode::ArrowRight);
        tap(&mut app, KeyCode::ArrowDown);
        tap(&mut app, KeyCode::ArrowDown);
        tap(&mut app, KeyCode::Enter);
        run_for(&mut app, 300);
        let w = app.world().resource::<ocw_ui::weather::WeatherMenu>();
        assert_eq!(
            w.target.map(|t| t.0),
            ocw_weather::presets::preset("hail"),
            "{w:?}"
        );
        // Close; the rider list (Navi) knows the bots.
        for _ in 0..4 {
            tap(&mut app, KeyCode::Escape);
        }
        assert!(!view(&app).open);
        console(&mut app, "menu open");
        for _ in 0..4 {
            tap(&mut app, KeyCode::ArrowDown);
        }
        tap(&mut app, KeyCode::Enter);
        assert_eq!(view(&app).title, "Navi");
        assert_eq!(
            view(&app).items,
            ["Training history", "Riders in this ride", "Back"]
        );
        tap(&mut app, KeyCode::ArrowDown);
        tap(&mut app, KeyCode::Enter);
        let v = view(&app);
        assert_eq!(v.title, "Riders in this ride");
        assert!(v.status.starts_with("6 riders"), "{}", v.status);
        // Ride → Workouts: the bundled examples.
        console(&mut app, "menu close");
        console(&mut app, "menu open");
        tap(&mut app, KeyCode::ArrowDown);
        tap(&mut app, KeyCode::Enter);
        assert_eq!(view(&app).title, "Ride");
        assert!(!view(&app).items.iter().any(|i| i == "Restart"));
        for _ in 0..1 {
            tap(&mut app, KeyCode::ArrowDown);
        }
        tap(&mut app, KeyCode::Enter);
        let v = view(&app);
        assert_eq!(v.title, "Workouts");
        assert!(v.items[0].ends_with("example"), "{:?}", v.items);
        assert_eq!(log.lock().unwrap().len(), 0);
    }

    /// Unicorn mode in the browser (spec §14 Phase 13 G5): World →
    /// Unicorn mode flies on the page's local server, the riding keys do
    /// nothing in flight (↓ would stop the trainer), the HUD's flight view
    /// with the inspection, `window.ocw.god()`'s JSON, G lands.
    #[test]
    fn unicorn_mode_flies_from_the_world_page() {
        let mut app = app(DownloadLog::default());
        run_for(&mut app, 600);
        tap(&mut app, KeyCode::Escape);
        tap(&mut app, KeyCode::ArrowDown);
        tap(&mut app, KeyCode::ArrowDown);
        tap(&mut app, KeyCode::Enter);
        let v = view(&app);
        assert_eq!(v.title, "World");
        let at = v
            .items
            .iter()
            .position(|i| i.starts_with(ocw_ui::god::MENU_NAME))
            .unwrap_or_else(|| panic!("{:?}", v.items));
        for _ in 0..at {
            tap(&mut app, KeyCode::ArrowDown);
        }
        tap(&mut app, KeyCode::Enter);
        console(&mut app, "menu close");
        let flying = |app: &App| app.world().resource::<ocw_ui::god::GodStatus>().flying;
        let start = std::time::Instant::now();
        while !flying(&app) && start.elapsed() < std::time::Duration::from_secs(5) {
            run_for(&mut app, 50);
        }
        assert!(flying(&app), "{}", console(&mut app, "god status"));
        let power = app.world().resource::<Session>().0.power_w();
        tap(&mut app, KeyCode::ArrowDown);
        run_for(&mut app, 300);
        assert_eq!(
            app.world().resource::<Session>().0.power_w(),
            power,
            "↓ flies slower, it does not stop the trainer"
        );
        let rv = app.world().resource::<RideView>();
        let f = rv.flight.as_ref().expect("the flight view");
        assert!(f.keys.contains("+/-: climb/descend"), "{}", f.keys);
        assert!(
            f.panel.iter().any(|l| l.starts_with("Position x")),
            "{:?}",
            f.panel
        );
        let json = crate::god::flight_json(
            app.world().resource::<ocw_ui::god::GodStatus>(),
            app.world().resource::<ocw_ui::god::FlightPointer>(),
            rv,
        );
        let j: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(j["flying"], true);
        assert!(j["pos"][1].as_f64().is_some());
        tap(&mut app, KeyCode::KeyG);
        let start = std::time::Instant::now();
        while app.world().resource::<ocw_ui::god::GodStatus>().active
            && start.elapsed() < std::time::Duration::from_secs(8)
        {
            run_for(&mut app, 50);
        }
        assert!(!app.world().resource::<ocw_ui::god::GodStatus>().active);
        assert!(app.world().resource::<RideView>().flight.is_none());
    }

    /// Spec §2.11 Q3: the browser's photos go through the photo saver –
    /// the chosen folder (the page) or a download – and the preview's
    /// caption says which.
    #[test]
    fn photos_are_saved_with_their_caption() {
        assert_eq!(photo_went("folder:OCW", "OCW-a.jpg"), "Saved to OCW");
        assert_eq!(photo_went("download", "OCW-a.jpg"), "Downloaded OCW-a.jpg");
        let log = DownloadLog::default();
        let mut app = app(log.clone());
        app.update();
        let sink = app.world().resource::<ShotSink>().clone();
        let went = (sink.0)("OCW-2026-11-17_10-00-00.jpg", vec![0xff, 0xd8]);
        assert_eq!(went, "Downloaded OCW-2026-11-17_10-00-00.jpg");
        let downloads = log.lock().unwrap().clone();
        assert_eq!(downloads.len(), 1);
        assert_eq!(
            (downloads[0].0.as_str(), downloads[0].1.as_str()),
            ("OCW-2026-11-17_10-00-00.jpg", "image/jpeg")
        );
        // The ride photographer is allowed in the browser now.
        assert!(
            app.world()
                .resource::<ocw_ui::photographer::RidePhotographer>()
                .allowed
        );
        assert!(console(&mut app, "photos.folder").contains("downloads named OCW-"));
    }

    #[test]
    fn the_rides_fit_file_is_a_download() {
        let log = DownloadLog::default();
        let mut app = app(log.clone());
        let early = console(&mut app, "ride.save");
        assert!(early.contains("nothing to save yet"), "{early}");
        run_for(&mut app, 2300);
        // Captures → Download this ride (FIT), with the keyboard.
        tap(&mut app, KeyCode::Escape);
        tap(&mut app, KeyCode::ArrowUp);
        tap(&mut app, KeyCode::Enter);
        assert_eq!(view(&app).title, "Captures");
        tap(&mut app, KeyCode::ArrowDown);
        tap(&mut app, KeyCode::ArrowDown);
        tap(&mut app, KeyCode::Enter);
        app.update();
        let downloads = log.lock().unwrap().clone();
        assert_eq!(
            downloads.len(),
            1,
            "{:?}",
            app.world().resource::<ConsoleState>().log()
        );
        let (name, mime, bytes) = &downloads[0];
        assert!(
            name.starts_with("ocw-ride-") && name.ends_with(".fit"),
            "{name}"
        );
        assert_eq!(mime, "application/vnd.ant.fit");
        assert_eq!(&bytes[8..12], b".FIT");
        assert_eq!(ocw_fit::crc16(0, bytes), 0);
        // Video and the real screenshot need the page.
        let out = console(&mut app, "video start");
        assert!(
            out.contains("no video recorder outside the browser"),
            "{out}"
        );
    }

    /// The simple Strava upload in the browser (spec §2.8.1 *Simple
    /// upload*): a saved ride's page has the row; it downloads the FIT
    /// file under its recognisable name, opens Strava's upload page in a
    /// new tab and shows the steps on the page and in the HUD; the mark
    /// stays in the history.
    #[test]
    fn a_ride_is_uploaded_to_strava_in_a_new_tab() {
        let log = DownloadLog::default();
        let tabs = Arc::new(Mutex::new(Vec::<String>::new()));
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ConsoleCorePlugin))
            .init_resource::<ButtonInput<KeyCode>>()
            .insert_resource(Time::<Fixed>::from_hz(f64::from(ocw_core::SIM_TICK_HZ)))
            .insert_resource(RideView::new(ocw_world::Route::test_hills()))
            .insert_resource(Downloads::logged(log.clone()))
            .insert_resource(Tabs::logged(tabs.clone()))
            .add_plugins(crate::plugin::WebSessionPlugin {
                config: SessionConfig::default(),
            })
            .add_plugins((crate::settings::WebSettingsPlugin, WebUiPlugin::default()));
        app.update();
        run_for(&mut app, 2300);
        let out = console(&mut app, "ride.save");
        assert!(out.contains("ride saved"), "{out}");
        // The ride's page: the upload row and the mark (no API here).
        let info = ocw_ui::history::info(app.world());
        assert!(!info.strava_api);
        let page = ocw_client::menu::pages::Page::HistoryRide(0);
        let rows = ocw_client::menu::history::rows(page, &info);
        let up = rows
            .iter()
            .find(|r| r.label == ocw_client::menu::history::UPLOAD_LABEL)
            .expect("the upload row");
        assert_eq!(
            up.confirm,
            Some(ocw_client::menu::pages::Activate::Command(
                "strava.upload-manual 1".into()
            ))
        );
        let out = console(&mut app, "strava.upload-manual 1");
        assert!(out.contains("Strava opens in a new tab"), "{out}");
        assert_eq!(
            *tabs.lock().unwrap(),
            vec![ocw_client::strava::UPLOAD_PAGE.to_string()]
        );
        let downloads = log.lock().unwrap().clone();
        assert_eq!(downloads.len(), 2, "the ride, then the upload's copy");
        let (name, mime, bytes) = &downloads[1];
        assert!(
            name.starts_with("OCW 20") && name.ends_with(".fit"),
            "{name}"
        );
        assert!(
            out.contains(name.as_str()),
            "the steps name the file: {out}"
        );
        assert_eq!(mime, "application/vnd.ant.fit");
        assert_eq!(bytes, &downloads[0].2);
        app.update();
        assert!(
            app.world()
                .resource::<RideView>()
                .strava_message
                .as_deref()
                .is_some_and(|m| m.starts_with("Strava opens in a new tab")),
            "the HUD shows the steps"
        );
        let info = ocw_ui::history::info(app.world());
        let text = ocw_client::menu::history::text(page, &info);
        assert!(text.iter().any(|l| l.contains("Strava opens in a new tab")));
        // The mark.
        console(&mut app, "strava.mark-uploaded 1 on");
        let info = ocw_ui::history::info(app.world());
        assert!(info.history.nth_newest(0).unwrap().strava_marked);
        let rows = ocw_client::menu::history::rows(page, &info);
        assert_eq!(
            rows.iter()
                .find(|r| r.label == ocw_client::menu::history::MARK_LABEL)
                .unwrap()
                .value
                .as_deref(),
            Some("yes")
        );
    }

    #[test]
    fn the_settings_pages_and_the_aids_reach_the_ride() {
        let mut app = app(DownloadLog::default());
        run_for(&mut app, 400);
        // The riding aids and damage: the shared commands on the session.
        let out = console(&mut app, "ride.proximity_cue on");
        assert!(out.contains("proximity cue on"), "{out}");
        let out = console(&mut app, "ride.damage on");
        assert!(!out.contains("error: ride.damage"), "{out}");
        run_for(&mut app, 300);
        console(&mut app, "dev.damage rider 60");
        run_for(&mut app, 300);
        assert!(
            app.world().resource::<RideView>().damage.is_some(),
            "the damage widget"
        );
        // An event from the Ride page's command.
        let out = console(&mut app, "event race --km 1 --bots 0");
        assert!(!out.contains("error: event"), "{out}");
        run_for(&mut app, 300);
        assert!(app.world().resource::<RideView>().event.is_some());
    }
}
