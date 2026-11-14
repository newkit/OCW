//! In-game menu (spec §3.2.1, increment M1): Esc / Start opens it over the
//! running ride. The ride never pauses: the trainer, the ride time, the
//! recording and a video keep going; while the menu is open the steering
//! falls back to autodrive and junctions take their default exit
//! ([`ClientCore::hold_steering`]), restored on closing.
//!
//! [`model`] is the Bevy-free menu (pages, focus, page stack), [`controls`]
//! maps keys and buttons onto its actions; this module is the Bevy glue:
//! input → model → [`MenuView`] (drawn by `ocw_render::menu`), menu
//! functions run as console command lines.
//!
//! [`ClientCore::hold_steering`]: crate::client::ClientCore::hold_steering

pub mod controls;
pub mod model;

use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use clap::{Arg, Command};
use ocw_core::mps_to_kmh;
use ocw_render::RideView;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, ConsoleState, Permission};
use ocw_render::menu::{InputContext, MenuCorePlugin, MenuPointer, MenuPointerEvent, MenuView};

use crate::audio::AudioDeviceList;
use crate::connection::ClientState;
use crate::ride::RideCatalog;
use crate::settings::pages as settings_pages;
use crate::settings::registry::Settings;
use crate::settings::riders::Riders;
use crate::settings::{RiderProfiles, UserSettings};
use controls::{NavRepeat, PadState};
use model::{Action, Effect, Menu, Page};

pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<MenuCorePlugin>() {
            app.add_plugins(MenuCorePlugin);
        }
        app.init_resource::<MenuState>()
            .add_systems(Update, (menu_input, write_view).chain())
            .console_command(ConsoleCommand::new(
                Permission::User,
                Command::new("menu")
                    .about("Open, close or drive the in-game menu (Esc / gamepad Start)")
                    .arg(
                        Arg::new("action")
                            .value_parser([
                                "open",
                                "close",
                                "toggle",
                                "up",
                                "down",
                                "left",
                                "right",
                                "select",
                                "back",
                                "tab-next",
                                "tab-prev",
                                "workshop-page",
                            ])
                            .default_value("toggle"),
                    ),
                |world, m| {
                    let mut state = world.resource_mut::<MenuState>();
                    let open = state.menu.is_open();
                    let action = match m.get_one::<String>("action").map(String::as_str) {
                        Some("open") => Action::Open,
                        Some("close") => Action::Close,
                        Some("up") => Action::Up,
                        Some("down") => Action::Down,
                        Some("left") => Action::Left,
                        Some("right") => Action::Right,
                        Some("tab-next") => Action::TabNext,
                        Some("tab-prev") => Action::TabPrev,
                        Some("select") => Action::Confirm,
                        Some("back") => Action::Back,
                        Some("workshop-page") => Action::RiderPage,
                        _ if open => Action::Close,
                        _ => Action::Open,
                    };
                    state.queued.push(action);
                    Ok(String::new())
                },
            ));
    }
}

/// The menu model and its input state.
#[derive(Resource, Default)]
pub struct MenuState {
    pub menu: Menu,
    repeat: NavRepeat,
    /// Actions from the console command `menu`, applied next frame.
    queued: Vec<Action>,
}

/// The voice, time-machine and weather state the menu pages show.
type MenuInfos<'w> = (
    Option<Res<'w, crate::voice::VoiceMenuInfo>>,
    Option<Res<'w, crate::timemachine::TimeMachineInfo>>,
    Option<Res<'w, crate::weather::WeatherState>>,
);

#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
fn menu_input(
    keys: Option<Res<ButtonInput<KeyCode>>>,
    pads: Query<&Gamepad>,
    motion: Option<Res<AccumulatedMouseMotion>>,
    time: Res<Time>,
    context: Res<InputContext>,
    mut console: ResMut<ConsoleState>,
    mut pointer: ResMut<MenuPointer>,
    mut state: ResMut<MenuState>,
    client: Option<ResMut<ClientState>>,
    settings: Option<Res<UserSettings>>,
    riders: Option<Res<RiderProfiles>>,
    catalog: Option<Res<RideCatalog>>,
    audio: Option<Res<AudioDeviceList>>,
    (voice, time_machine, weather): MenuInfos<'_>,
    flight: Option<ResMut<crate::god::FlightPointer>>,
) {
    let mut pressed = pads.iter().fold(
        keys.map(|k| controls::keyboard(&k)).unwrap_or_default(),
        |p, pad| p.merge(controls::gamepad(PadState::of(pad))),
    );
    // God mode (spec §14 Phase 13A): Esc first frees the captured pointer;
    // the next Esc opens the menu.
    if let Some(mut f) = flight
        && f.captured
        && pressed.escape
        && !state.menu.is_open()
        && !console.is_open()
    {
        f.captured = false;
        pressed.escape = false;
    }
    let mut actions = std::mem::take(&mut state.queued);
    let pointer_events = std::mem::take(&mut pointer.0);
    let open = state.menu.is_open();
    let mut client = client;
    let now = time.elapsed_secs_f64();
    let results =
        !open && !console.is_open() && client.as_ref().is_some_and(|c| c.0.results_showing(now));
    if results && (pressed.escape || pressed.confirm || pressed.back) {
        // An event's results panel is up: Esc / Enter / B close it first
        // (spec §2.3), the menu opens on the next press.
        if let Some(c) = client.as_mut() {
            c.0.dismiss_results(now);
        }
        state.repeat.reset();
    } else if console.is_open() {
        // The console has the keys; Esc closes it first (never the app).
        if pressed.escape {
            console.set_open(false);
        }
        state.repeat.reset();
    } else if (!open && *context != InputContext::Workshop) || *context == InputContext::Menu {
        // Navigation starts once the frame began in the menu's context:
        // the key that opened the menu does not also move its focus.
        let step = if open {
            state
                .repeat
                .update(pressed.held, pressed.tapped, time.delta_secs_f64())
                + pressed.step
        } else {
            0
        };
        actions.extend(controls::actions(&pressed, step, open));
        // Hover only when the mouse moved: rows appearing under a resting
        // pointer (a new page) must not take the focus.
        let moved = motion.is_some_and(|m| m.delta != Vec2::ZERO);
        actions.extend(pointer_events.into_iter().filter_map(|e| match e {
            MenuPointerEvent::Hover(i) => moved.then_some(Action::Hover(i)),
            MenuPointerEvent::Click(i) => Some(Action::Click(i)),
            MenuPointerEvent::Back => Some(Action::Back),
        }));
    }
    let rows = |p| {
        sub_rows(
            p,
            settings.as_deref(),
            riders.as_deref(),
            catalog.as_deref(),
            audio.as_deref(),
            voice.as_deref(),
            (time_machine.as_deref(), weather.as_deref()),
        )
    };
    for action in actions {
        if let Effect::Command(line) = state.menu.apply_with(action, rows) {
            console.queue(line);
        }
    }
    let now_open = state.menu.is_open();
    if now_open != open {
        if !now_open {
            state.repeat.reset();
        }
        info!("menu {}", if now_open { "opened" } else { "closed" });
        if let Some(mut c) = client {
            if now_open {
                c.0.hold_steering();
            } else {
                c.0.release_steering();
            }
        }
    }
}

/// The status strip of the top page: power, speed, trainer and steering.
pub fn status_line(view: &RideView) -> String {
    match view.local() {
        Some(r) => format!(
            "{:.0} W   {:.1} km/h   {}",
            r.power_w,
            mps_to_kmh(r.state.speed),
            view.status
        ),
        None => view.status.clone(),
    }
}

/// The control hint at the bottom of `page` (`list_pages` pages of a
/// list, `depth` pages open).
pub fn hint(page: Page, list_pages: usize, depth: usize) -> String {
    format!(
        "Up/Down move   {}Enter/A select   Esc/B {}",
        match page {
            Page::Sub(p) if p.is_ride_page() && list_pages > 1 => "Left/Right page   ",
            Page::Sub(p) if p.is_ride_page() => "",
            Page::Sub(_) => "Left/Right change   Q/E, LB/RB tab   ",
            _ => "",
        },
        if depth > 1 { "back" } else { "close" }
    )
}

/// The rows of a Settings sub page, the Workshop or a Ride / Navi list
/// (defaults without the settings or ride plugin, e.g. in tests of the
/// menu alone).
fn sub_rows(
    page: settings_pages::Page,
    settings: Option<&UserSettings>,
    riders: Option<&RiderProfiles>,
    catalog: Option<&RideCatalog>,
    audio: Option<&AudioDeviceList>,
    voice: Option<&crate::voice::VoiceMenuInfo>,
    (time_machine, weather): (
        Option<&crate::timemachine::TimeMachineInfo>,
        Option<&crate::weather::WeatherState>,
    ),
) -> Vec<settings_pages::Row> {
    let no_time = crate::timemachine::TimeMachineInfo::default();
    let time_machine = time_machine.unwrap_or(&no_time);
    if page == settings_pages::Page::TimeMachine {
        return crate::timemachine::rows(time_machine);
    }
    if page.is_world_page() {
        let empty = crate::ride::pages::Catalog::default();
        let clear = crate::weather::WeatherState::default();
        return crate::ride::world::rows(
            page,
            catalog.map_or(&empty, |c| &c.0),
            time_machine,
            weather.unwrap_or(&clear),
        );
    }
    let default_settings = Settings::default();
    if page.is_voice_page() {
        let empty = crate::voice::VoiceMenuInfo::default();
        return crate::voice::page::rows(
            page,
            settings.map_or(&default_settings, |s| &s.settings),
            voice.unwrap_or(&empty),
        );
    }
    if page.is_ride_page() {
        let empty = crate::ride::pages::Catalog::default();
        return crate::ride::pages::rows(page, catalog.map_or(&empty, |c| &c.0));
    }
    let default_settings = Settings::default();
    if page == settings_pages::Page::Audio {
        let empty = settings_pages::AudioDevices::default();
        return settings_pages::audio_rows(
            settings.map_or(&default_settings, |s| &s.settings),
            audio.map_or(&empty, |a| &a.0),
        );
    }
    let default_riders = Riders::default();
    settings_pages::rows(
        page,
        settings.map_or(&default_settings, |s| &s.settings),
        riders.map_or(&default_riders, |r| &r.riders),
    )
}

/// Text of a page beyond its fixed description (a data source's notice).
fn page_text(page: Page, catalog: Option<&RideCatalog>) -> Vec<String> {
    match (page, catalog) {
        (Page::Sub(p), Some(c)) if p.is_world_page() => crate::ride::world::text(p, &c.0),
        _ => Vec::new(),
    }
}

#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
fn write_view(
    mut state: ResMut<MenuState>,
    ride: Option<Res<RideView>>,
    settings: Option<Res<UserSettings>>,
    riders: Option<Res<RiderProfiles>>,
    catalog: Option<Res<RideCatalog>>,
    // Audio and voice device lists, grouped (system parameter limit).
    (audio, voice): (
        Option<Res<AudioDeviceList>>,
        Option<Res<crate::voice::VoiceMenuInfo>>,
    ),
    (time_machine, weather): (
        Option<Res<crate::timemachine::TimeMachineInfo>>,
        Option<Res<crate::weather::WeatherState>>,
    ),
    mut view: ResMut<MenuView>,
) {
    // Values changed by the menu's commands (or the console) show at once.
    let rows = |p| {
        sub_rows(
            p,
            settings.as_deref(),
            riders.as_deref(),
            catalog.as_deref(),
            audio.as_deref(),
            voice.as_deref(),
            (time_machine.as_deref(), weather.as_deref()),
        )
    };
    state.menu.refresh(&rows);
    let menu = &state.menu;
    let new = match menu.page() {
        None => MenuView::default(),
        Some(page) => {
            let def = page.def();
            MenuView {
                open: true,
                title: def.title.into(),
                status: match (page, &ride) {
                    (Page::Top, Some(r)) => status_line(r),
                    _ => String::new(),
                },
                text: def
                    .text
                    .iter()
                    .map(|&s| s.to_owned())
                    .chain(page_text(page, catalog.as_deref()))
                    .collect(),
                items: menu.labels(),
                focused: menu.focus(),
                hint: hint(page, menu.list_pages(), menu.depth()),
            }
        }
    };
    view.set_if_neq(new);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Args;
    use crate::connection::ConnectionPlugin;
    use clap::Parser;
    use ocw_render::console::ConsoleCorePlugin;
    use ocw_world::Route;

    /// Every sub page (a new one must be added here: the match below does
    /// not compile otherwise).
    fn all_sub_pages() -> Vec<settings_pages::Page> {
        use settings_pages::Page::*;
        let all = vec![
            Controls,
            Trainer,
            Strava,
            Display,
            Audio,
            Riders,
            Rider(0),
            Workshop,
            Bike(0),
            WorldHome,
            Worlds,
            World(0),
            WorldRoutes(0),
            WorldParts(0),
            WorldInfo,
            WorldSun,
            WorldSources,
            WorldSource(0),
            Weather,
            Routes,
            Rides,
            GpxImport,
            Workouts,
            Voice,
            VoiceDevices,
            VoiceParticipants,
            AudioVoice,
            TimeMachine,
        ];
        for p in &all {
            match p {
                Controls | Trainer | Strava | Display | Audio | Riders | Rider(_) | Workshop
                | Bike(_) | WorldHome | Worlds | World(_) | WorldRoutes(_) | WorldParts(_)
                | WorldInfo | WorldSun | WorldSources | WorldSource(_) | Weather | Routes
                | Rides | GpxImport | Workouts | Voice | VoiceDevices | VoiceParticipants
                | AudioVoice | TimeMachine => {}
            }
        }
        all
    }

    /// Regression (scene `menu-ride`): the Ride page ran off the bottom of
    /// a 1280x720 window and "Back" was cut off. Every page fits into the
    /// panel at 16:9 (all sizes are in % of the window height, so this
    /// holds for 1280x720 and every larger 16:9 window) with all its text;
    /// sub pages with as many items as a page can have (paged lists).
    #[test]
    fn every_menu_page_fits_a_16_9_window_with_its_text() {
        use ocw_render::menu::{PANEL_MAX_VH, fitted_paragraphs, page_height_vh};
        let aspect = 16.0 / 9.0;
        let check = |view: MenuView| {
            let h = page_height_vh(&view, aspect, view.text.len());
            assert!(h <= PANEL_MAX_VH, "{}: {h:.1} vh", view.title);
            assert_eq!(fitted_paragraphs(&view, aspect), view.text.len());
        };
        let pages = [
            Page::Top,
            Page::Ride,
            Page::Navi,
            Page::Settings,
            Page::ConfirmRestart,
            Page::ConfirmQuit,
            Page::ConfirmSwitch,
            Page::ConfirmEvent,
        ];
        let sub = all_sub_pages().into_iter().map(Page::Sub);
        for page in pages.into_iter().chain(sub) {
            let def = page.def();
            let items: Vec<String> = match page {
                // A full page of a paged list: rows, Previous, Next, Back.
                Page::Sub(_) => (0..model::MAX_ITEMS)
                    .map(|i| format!("Setting {i}:  < some value >"))
                    .collect(),
                _ => def.items.iter().map(|i| i.label.to_string()).collect(),
            };
            // The longest dynamic text a page gets: the OSM notice.
            let catalog = RideCatalog(crate::ride::world::tests::catalog());
            check(MenuView {
                open: true,
                title: def.title.into(),
                status: match page {
                    Page::Top => "250 W   32.4 km/h   Trainer: virtual (SIM)".into(),
                    _ => String::new(),
                },
                text: def
                    .text
                    .iter()
                    .map(|&s| s.to_owned())
                    .chain(page_text(page, Some(&catalog)))
                    .collect(),
                items,
                focused: 0,
                hint: hint(page, 2, 3),
            });
        }
    }

    /// The desktop's ride, input and menu plugins without a window: a
    /// stepped local server (as in frame capture), the virtual trainer.
    fn app() -> App {
        let capture = std::env::temp_dir().join(format!("ocw-menu-test-{}", std::process::id()));
        let args = Args::parse_from([
            "ocw-desktop",
            "--bots",
            "0",
            "--virtual",
            "--capture",
            capture.to_str().unwrap(),
        ]);
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ConsoleCorePlugin))
            .add_message::<AppExit>()
            .init_resource::<Exits>()
            .add_systems(
                Last,
                |mut exit: MessageReader<AppExit>, mut n: ResMut<Exits>| n.0 += exit.read().count(),
            )
            .init_resource::<ButtonInput<KeyCode>>()
            .insert_resource(args)
            .insert_resource(RideView::new(Route::test_loop()))
            .insert_resource(crate::connection::Ghosts(None))
            .add_plugins((
                ConnectionPlugin {
                    route: Route::test_loop(),
                },
                crate::devices::DevicesPlugin,
                crate::input::InputPlugin,
                MenuPlugin,
            ));
        app.update();
        assert!(
            app.world().resource::<ClientState>().0.rider_id().is_some(),
            "joined"
        );
        app
    }

    /// Presses `key` for one frame, then releases it.
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

    /// App exits requested so far.
    #[derive(Resource, Default)]
    struct Exits(usize);

    fn exits(app: &App) -> usize {
        app.world().resource::<Exits>().0
    }

    fn client(app: &mut App) -> Mut<'_, ClientState> {
        app.world_mut().resource_mut::<ClientState>()
    }

    /// The server's word that the local rider finished event `id` (a time
    /// trial): the results panel comes up.
    fn finish_event(app: &mut App, id: u32) {
        use ocw_protocol::v1::server_message::Msg as S;
        let now = app.world().resource::<Time>().elapsed_secs_f64();
        let mut c = client(app);
        let own = c.0.rider_id().unwrap();
        let mut ev = ocw_sim::Event::new(
            id,
            ocw_sim::EventKind::TimeTrial {
                length_m: 1000.0,
                laps: None,
            },
            0,
            0.0,
            vec![(own, "me".into())],
        );
        ev.phase = ocw_sim::EventPhase::Finished;
        ev.entrants[0].finish_s = Some(100.0);
        let standings = ev.standings(&[(own, 1000.0, 0.0)]);
        let bytes = ocw_protocol::encode(&ocw_protocol::server_message(S::Event(
            ocw_protocol::event_state(Some(&ev), &standings, 0),
        )));
        c.0.on_server_message(&bytes, now);
        assert!(c.0.results_showing(now));
    }

    #[test]
    fn esc_and_a_close_the_results_first() {
        let mut app = app();
        finish_event(&mut app, 1);
        tap(&mut app, KeyCode::Escape);
        assert!(
            !view(&app).open,
            "Esc closed the results, not opened the menu"
        );
        let now = app.world().resource::<Time>().elapsed_secs_f64();
        assert!(!client(&mut app).0.results_showing(now));
        // A closes them too, without switching autodrive off.
        finish_event(&mut app, 2);
        tap(&mut app, KeyCode::KeyA);
        let now = app.world().resource::<Time>().elapsed_secs_f64();
        assert!(!client(&mut app).0.results_showing(now));
        assert!(client(&mut app).0.autodrive());
        // Without results Esc opens the menu as always.
        tap(&mut app, KeyCode::Escape);
        assert!(view(&app).open);
    }

    #[test]
    fn esc_opens_the_menu_and_enter_selects_resume() {
        let mut app = app();
        assert!(!view(&app).open);
        tap(&mut app, KeyCode::Escape);
        let v = view(&app);
        assert!(v.open);
        assert_eq!(v.items[v.focused], "Resume");
        assert_eq!(*app.world().resource::<InputContext>(), InputContext::Menu);
        tap(&mut app, KeyCode::Enter);
        assert!(!view(&app).open);
        assert_eq!(*app.world().resource::<InputContext>(), InputContext::Ride);
        assert_eq!(exits(&app), 0);
    }

    #[test]
    fn esc_closes_the_console_first_and_never_quits() {
        let mut app = app();
        app.world_mut()
            .resource_mut::<ConsoleState>()
            .set_open(true);
        app.update();
        tap(&mut app, KeyCode::Escape);
        assert!(!app.world().resource::<ConsoleState>().is_open());
        assert!(!view(&app).open, "the menu stays closed");
        // Esc on the top page closes the menu, never the app.
        for _ in 0..3 {
            tap(&mut app, KeyCode::Escape);
            assert!(view(&app).open);
            tap(&mut app, KeyCode::Escape);
            assert!(!view(&app).open);
        }
        // From a sub page: back, then closed.
        tap(&mut app, KeyCode::Escape);
        tap(&mut app, KeyCode::ArrowDown);
        tap(&mut app, KeyCode::Enter);
        assert_eq!(view(&app).title, "Ride");
        tap(&mut app, KeyCode::Escape);
        assert_eq!(view(&app).title, "Open Cycling World");
        tap(&mut app, KeyCode::Escape);
        assert!(!view(&app).open);
        assert_eq!(exits(&app), 0);
        // The console over the menu: Esc closes the console, the menu stays.
        tap(&mut app, KeyCode::Escape);
        app.world_mut()
            .resource_mut::<ConsoleState>()
            .set_open(true);
        app.update();
        tap(&mut app, KeyCode::Escape);
        assert!(!app.world().resource::<ConsoleState>().is_open());
        assert!(view(&app).open);
        assert_eq!(exits(&app), 0);
    }

    #[test]
    fn ride_keys_are_ignored_while_the_menu_is_open() {
        let mut app = app();
        assert!(client(&mut app).0.autodrive());
        // In the ride: A switches autodrive off.
        tap(&mut app, KeyCode::KeyA);
        assert!(!client(&mut app).0.autodrive());
        tap(&mut app, KeyCode::KeyA);
        assert!(client(&mut app).0.autodrive());
        tap(&mut app, KeyCode::Escape);
        assert!(view(&app).open);
        // In the menu: A, W, ↑ and ← do nothing to the ride.
        let line = client(&mut app).0.lateral_target();
        for key in [
            KeyCode::KeyA,
            KeyCode::KeyW,
            KeyCode::ArrowUp,
            KeyCode::ArrowLeft,
        ] {
            tap(&mut app, key);
        }
        let c = client(&mut app);
        assert!(c.0.autodrive() && !c.0.wild());
        assert_eq!(c.0.key_power_w(), 0.0);
        assert_eq!(c.0.lateral_target(), line);
        // ↑ moved the menu's focus instead (wrapped to Quit).
        assert_eq!(view(&app).items[view(&app).focused], "Quit");
        // R from the menu: no restart (only the confirmed menu item).
        tap(&mut app, KeyCode::KeyR);
        assert_eq!(exits(&app), 0);
    }

    #[test]
    fn steering_falls_back_to_autodrive_and_is_restored() {
        let mut app = app();
        client(&mut app).0.set_lean(0.15);
        assert!(!client(&mut app).0.autodrive());
        tap(&mut app, KeyCode::Escape);
        assert!(client(&mut app).0.autodrive() && client(&mut app).0.steering_held());
        // Through a sub page and back: still held.
        tap(&mut app, KeyCode::ArrowDown);
        tap(&mut app, KeyCode::Enter);
        tap(&mut app, KeyCode::Backspace);
        assert!(client(&mut app).0.autodrive());
        tap(&mut app, KeyCode::Escape);
        let c = client(&mut app);
        assert!(!c.0.steering_held());
        assert_eq!(c.0.lean(), Some(0.0), "manual again, upright");
    }

    #[test]
    fn the_ride_goes_on_under_the_menu() {
        let mut app = app();
        tap(&mut app, KeyCode::Escape);
        let t0 = app.world().resource::<Time>().elapsed_secs_f64();
        let n0 = client(&mut app).0.stats().samples_out;
        std::thread::sleep(std::time::Duration::from_millis(300));
        for _ in 0..20 {
            app.update();
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        assert!(view(&app).open);
        // Time runs and the virtual trainer's samples still reach the server.
        assert!(app.world().resource::<Time>().elapsed_secs_f64() > t0 + 0.3);
        assert!(client(&mut app).0.stats().samples_out > n0);
        let status = app.world().resource::<RideView>().status.clone();
        assert!(status.contains("trainer SIM"), "{status}");
        assert!(view(&app).status.contains("km/h"), "{:?}", view(&app));
    }

    #[test]
    fn restart_and_quit_need_a_confirmation() {
        let mut app = app();
        // Quit: Esc, ↑ (wraps to Quit), Enter → the confirmation, Cancel focused.
        tap(&mut app, KeyCode::Escape);
        tap(&mut app, KeyCode::ArrowUp);
        tap(&mut app, KeyCode::Enter);
        let v = view(&app);
        assert_eq!(
            (v.title.as_str(), v.items[v.focused].as_str()),
            ("Quit?", "Cancel")
        );
        tap(&mut app, KeyCode::Enter);
        assert_eq!(view(&app).title, "Open Cycling World");
        assert_eq!(exits(&app), 0);
        // Confirmed: the console's quit (the ride is saved on the exit).
        tap(&mut app, KeyCode::Enter);
        tap(&mut app, KeyCode::ArrowUp);
        tap(&mut app, KeyCode::Enter);
        assert_eq!(exits(&app), 1);
    }

    #[test]
    fn mouse_hover_focuses_only_when_moved_and_clicks_select() {
        let mut app = app();
        app.world_mut()
            .resource_mut::<ConsoleState>()
            .queue("menu open");
        app.update();
        app.update();
        assert!(view(&app).open);
        let push = |app: &mut App, e| app.world_mut().resource_mut::<MenuPointer>().0.push(e);
        push(&mut app, MenuPointerEvent::Hover(3));
        app.update();
        assert_eq!(view(&app).focused, 0, "the mouse did not move");
        app.insert_resource(AccumulatedMouseMotion {
            delta: Vec2::new(3.0, 1.0),
        });
        push(&mut app, MenuPointerEvent::Hover(3));
        app.update();
        assert_eq!(view(&app).focused, 3);
        push(&mut app, MenuPointerEvent::Click(5));
        app.update();
        assert_eq!(view(&app).title, "Settings");
        push(&mut app, MenuPointerEvent::Back);
        app.update();
        assert_eq!(view(&app).title, "Open Cycling World");
        push(&mut app, MenuPointerEvent::Click(0));
        app.update();
        assert!(!view(&app).open);
    }
}
