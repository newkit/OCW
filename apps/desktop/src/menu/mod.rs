//! In-game menu (spec §3.2.1, increment M1): Esc / Start opens it over the
//! running ride. The ride never pauses: the trainer, the ride time, the
//! recording and a video keep going; while the menu is open the steering
//! falls back to autodrive and junctions take their default exit
//! ([`ClientCore::hold_steering`]), restored on closing.
//!
//! Shared with the browser app since Phase 10 B6 (spec §14 Phase 10): the
//! model and the rows are `ocw_client::menu`, the Bevy glue (input →
//! model → `MenuView`, menu functions as console lines) `ocw_ui::menu`.
//! This module adds the desktop's own pages through [`MenuHooks`]: the
//! world catalog (Change world, About this world), routes, rides, GPX
//! import, workouts from the workouts folder, the rider profiles (god
//! mode's captured pointer is the shared menu's, `ocw_ui::god`).
//!
//! [`ClientCore::hold_steering`]: crate::client::ClientCore::hold_steering

pub mod model;

use bevy::prelude::*;
pub use ocw_client::menu::status_line;
pub use ocw_ui::menu::{MenuHooks, MenuState};

use crate::ride::RideCatalog;
use crate::settings::RiderProfiles;
use crate::settings::pages as settings_pages;
use model::Page;

/// The shared menu with the desktop's pages.
pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(MenuHooks {
            rows: desktop_rows,
            text: |world, page| {
                if let Page::Sub(p) = page {
                    let host = crate::ride::host::text(world, p);
                    if !host.is_empty() {
                        return host;
                    }
                }
                page_text(
                    page,
                    world.get_resource::<RideCatalog>(),
                    world
                        .get_resource::<crate::weather::WeatherMenu>()
                        .map(|w| &w.0),
                )
            },
            riders: |world| world.get_resource::<RiderProfiles>().map(|r| &r.riders),
            ..MenuHooks::default()
        });
        if !app.is_plugin_added::<ocw_ui::menu::MenuPlugin>() {
            app.add_plugins(ocw_ui::menu::MenuPlugin);
        }
    }
}

/// The rows of the desktop's own pages: the World and the Ride / Navi
/// lists (from the ride catalog).
fn desktop_rows(world: &World, page: settings_pages::Page) -> Option<Vec<settings_pages::Row>> {
    // Ride → Host a ride and the hosting's join link.
    if let Some(rows) = crate::ride::host::rows(world, page) {
        return Some(rows);
    }
    let empty = crate::ride::pages::Catalog::default();
    let catalog = world.get_resource::<RideCatalog>().map_or(&empty, |c| &c.0);
    if page.is_world_page() {
        let tm = world
            .get_resource::<crate::timemachine::TimeMachine>()
            .map(|t| t.0.clone())
            .unwrap_or_default();
        let weather = world
            .get_resource::<crate::weather::WeatherMenu>()
            .map(|w| w.0.clone())
            .unwrap_or_default();
        return Some(crate::ride::world::rows(page, catalog, &tm, &weather));
    }
    if page.is_ride_page() {
        return Some(crate::ride::pages::rows(page, catalog));
    }
    None
}

/// Text of a page beyond its fixed description (a data source's notice).
fn page_text(
    page: Page,
    catalog: Option<&RideCatalog>,
    weather: Option<&crate::weather::WeatherState>,
) -> Vec<String> {
    let clear = crate::weather::WeatherState::default();
    match (page, catalog) {
        (Page::Sub(p), Some(c)) if p.is_world_page() => {
            crate::ride::world::text(p, &c.0, weather.unwrap_or(&clear))
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Args;
    use crate::connection::{ClientState, ConnectionPlugin};
    use bevy::input::mouse::AccumulatedMouseMotion;
    use clap::Parser;
    use ocw_client::menu::hint;
    use ocw_render::RideView;
    use ocw_render::console::ConsoleCorePlugin;
    use ocw_render::console::ConsoleState;
    use ocw_render::menu::{InputContext, MenuPointer, MenuPointerEvent, MenuView};
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
            VoiceRider(0),
            VoiceBlocked,
            VoiceRoom,
            VoiceSelect,
            AudioVoice,
            TimeMachine,
            WeatherPresets,
            WeatherCustom,
            WeatherAir,
            WeatherReal,
            RidingAids,
            Graphics,
            ViewDistance,
            RiderList,
            Account,
            UnicornControls,
            Bindings,
            BindGroup(0),
            BindGroup(1),
            BindGroup(2),
            BindGroup(3),
            BindReset,
            Accessibility,
            History,
            HistoryRides,
            HistoryRide(0),
            HistoryClimbs(0),
            HistoryWeeks,
            HistoryMonths,
            HistoryBests,
            EventRules,
            TrustedKeys,
            TrustedKey(0),
            Devices,
            Device(0),
            Join,
            HostWorld,
            HostRoute,
            HostTime,
            HostWeather,
            HostOptions,
            HostSummary,
            HostLink,
            Calendar(ocw_client::menu::calendar::Cal::at((2026, 11, 16))),
            CalendarDates(ocw_client::menu::calendar::Cal::at((2026, 11, 16))),
            Season,
            Character,
            CharacterBody,
            CharacterFace,
            CharacterHair,
            CharacterFacialHair,
        ]
        .into_iter()
        .chain(ocw_client::menu::character::Attr::ALL.map(CharacterPick))
        .collect::<Vec<_>>();
        for p in &all {
            match p {
                Controls | Trainer | Strava | Display | Audio | Riders | Rider(_) | Workshop
                | Bike(_) | WorldHome | Worlds | World(_) | WorldRoutes(_) | WorldParts(_)
                | WorldInfo | WorldSun | WorldSources | WorldSource(_) | Weather | Routes
                | Rides | GpxImport | Workouts | Voice | VoiceDevices | VoiceParticipants
                | VoiceRider(_) | VoiceBlocked | VoiceRoom | VoiceSelect | AudioVoice
                | TimeMachine | WeatherPresets | WeatherCustom | WeatherAir | WeatherReal
                | RidingAids | Graphics | ViewDistance | RiderList | UnicornControls | Bindings
                | BindGroup(_) | BindReset | Accessibility | History | HistoryRides
                | HistoryRide(_) | HistoryClimbs(_) | HistoryWeeks | HistoryMonths
                | HistoryBests | EventRules | Account | TrustedKeys | TrustedKey(_) | Join
                | HostWorld | HostRoute | HostTime | HostWeather | HostOptions | HostSummary
                | HostLink | Devices | Device(_) | Calendar(_) | CalendarDates(_) | Season
                | Character | CharacterBody | CharacterFace | CharacterHair
                | CharacterFacialHair | CharacterPick(_) => {}
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
        // Every page is measured; the failures are listed together.
        let mut too_tall = Vec::new();
        let mut check = |view: MenuView| {
            let h = page_height_vh(&view, aspect, view.text.len());
            if h > PANEL_MAX_VH || fitted_paragraphs(&view, aspect) != view.text.len() {
                too_tall.push(format!("{}: {h:.1} vh", view.title));
            }
        };
        let pages = [
            Page::Top,
            Page::Ride,
            Page::Events,
            Page::HostBrowser,
            Page::StravaBrowser,
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
            // The weather pages with their longest status (a storm,
            // simulated, in a transition, set by the host).
            let mut weather = crate::weather::WeatherState {
                now: ocw_weather::presets::preset("storm"),
                mode: Some(ocw_weather::Mode::Simulated),
                sim_seed: 99_999,
                target: Some((ocw_weather::presets::preset("heavy-snow").unwrap(), 899.0)),
                control: "the host".into(),
                ..Default::default()
            };
            weather.ground = ocw_weather::Ground::settled(&weather.now.unwrap());
            let grid = match page {
                Page::Sub(settings_pages::Page::WeatherPresets) => {
                    Some(crate::weather::grid(&weather))
                }
                // The character editor's tiles (spec §14 Phase 4, 4N).
                Page::Sub(settings_pages::Page::CharacterPick(a)) => Some(
                    ocw_client::menu::character::grid(a, &ocw_client::riders::Rider::default()),
                ),
                _ => None,
            };
            // The calendar (spec §14 Phase 9 S9a): its days and buttons, the
            // longest day line, read-only.
            let cal_info = ocw_client::menu::calendar::CalendarInfo::default();
            let calendar = match page {
                Page::Sub(settings_pages::Page::Calendar(c)) => {
                    let rows = ocw_client::menu::calendar::rows(&c, &cal_info);
                    Some((
                        ocw_client::menu::calendar::view(&c, &cal_info, &rows),
                        rows.iter()
                            .map(model::row_label)
                            .chain(["Back".to_owned()])
                            .collect::<Vec<_>>(),
                    ))
                }
                _ => None,
            };
            let items: Vec<String> = match page {
                Page::Sub(_) if calendar.is_some() => calendar.as_ref().unwrap().1.clone(),
                // A grid: its cells.
                Page::Sub(_) if grid.is_some() => grid
                    .as_ref()
                    .unwrap()
                    .cells
                    .iter()
                    .map(|c| c.label.clone())
                    .collect(),
                // The pages with a chart have fixed rows: a ride's (climbs,
                // export, Back) and the bests (four and Back).
                Page::Sub(settings_pages::Page::HistoryRide(_)) => (0..3)
                    .map(|i| format!("Setting {i}:  < some value >"))
                    .collect(),
                Page::Sub(settings_pages::Page::HistoryBests) => (0..5)
                    .map(|i| format!("Setting {i}:  < some value >"))
                    .collect(),
                // The character editor's pages (side panel): their own
                // rows, as the menu labels them (spec §14 Phase 4, 4N).
                Page::Sub(p) if ocw_client::menu::character::is_editor_page(p) => {
                    ocw_client::menu::character::rows(p, &ocw_client::riders::Rider::default())
                        .iter()
                        .map(model::row_label)
                        .chain(["Back".to_string()])
                        .collect()
                }
                // A full page of a paged list: rows, Previous, Next, Back.
                Page::Sub(_) => (0..model::MAX_ITEMS)
                    .map(|i| format!("Setting {i}:  < some value >"))
                    .collect(),
                // The desktop's items (the browser's are checked below).
                _ => def
                    .items
                    .iter()
                    // The Ride page's longest mode: hosting.
                    .filter(|i| {
                        model::Platform::Desktop.shows(i)
                            && !matches!(i.when, model::When::Alone | model::When::Joined)
                    })
                    .map(|i| i.label.to_string())
                    .collect(),
            };
            // The longest dynamic text a page gets: the OSM notice; a ride
            // of the training history: its numbers and the power curve.
            let catalog = RideCatalog(crate::ride::world::tests::catalog());
            let history_text = match page {
                Page::Sub(settings_pages::Page::HistoryRide(_)) => vec![
                    "142.3 km in 10:22:05 (moving 10:20:11), 2512 m up, 4060 kJ, 1328 hearts.".to_string(),
                    "Power 215 W, NP 231 W, max 1640 W; heart rate 142 / 171 bpm; 12 crashes; rider 92 %, bike 88 %; workout: Over-unders with a long n...".to_string(),
                ],
                _ => Vec::new(),
            };
            let chart = match page {
                Page::Sub(
                    settings_pages::Page::HistoryRide(_) | settings_pages::Page::HistoryBests,
                ) => Some(ocw_render::menu::MenuChart {
                    caption: "Power curve (W): bars this ride, lines your best".into(),
                    bars: (0..10)
                        .map(|i| ocw_render::menu::ChartBar {
                            label: "60m".into(),
                            value: 1.0 - i as f32 * 0.05,
                            text: "1240".into(),
                            mark: Some(1.0),
                            best: false,
                        })
                        .collect(),
                }),
                _ => None,
            };
            check(MenuView {
                open: true,
                title: def.title.into(),
                status: match page {
                    Page::Top => "250 W   32.4 km/h   Trainer: virtual (SIM)".into(),
                    Page::Sub(p) if p.is_weather_page() => (0..18)
                        .map(|f| crate::weather::status(p, &weather, f))
                        .max_by_key(|s| s.len())
                        .unwrap(),
                    Page::Sub(settings_pages::Page::HistoryRide(_)) => {
                        "Sat 14 Nov 2026 10:05   Black Forest - north - Route to the Feldberg".into()
                    }
                    // The longest device message (cut to 56).
                    Page::Sub(settings_pages::Page::Devices) => "Wahoo KICKR 0001: connected (smart trainer (Wahoo...".into(),
                    Page::Sub(settings_pages::Page::BindGroup(_)) => {
                        "Conflict: V: Camera and Video recording; L: Camera and unicorn mode's flight keys".into()
                    }
                    Page::Sub(settings_pages::Page::Calendar(_)) => "Read only, set by the organizer · Wed 30 Sep 2026: sunrise 07:22 · sunset 19:14 · day 11 h 52 min · beech in autumn colours · snow line 2,750 m · first quarter".into(),
                    _ => String::new(),
                },
                text: def
                    .text
                    .iter()
                    .map(|&s| s.to_owned())
                    .chain(page_text(page, Some(&catalog), Some(&weather)))
                    .chain(history_text)
                    .collect(),
                items,
                focused: 0,
                hint: hint(page, 2, 3),
                dimmed: Vec::new(),
                grid,
                marks: Vec::new(),
                chart,
                emphasis: Vec::new(),
                wide: false,
                calendar: calendar.map(|c| c.0),
                side: matches!(page, Page::Sub(p) if ocw_client::menu::character::is_editor_page(p)),
            });
        }
        assert!(
            too_tall.is_empty(),
            "pages taller than {PANEL_MAX_VH} vh: {too_tall:?}"
        );
    }

    /// The desktop's ride, input and menu plugins without a window: a
    /// stepped local server (as in frame capture), the virtual trainer.
    fn app() -> App {
        app_with(|_| {})
    }

    /// [`app`] with more plugins (added before the first frame).
    fn app_with(extra: impl FnOnce(&mut App)) -> App {
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
        extra(&mut app);
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

    /// The menu app with the weather machine (spec §14 Phase 9, S5).
    fn weather_app() -> App {
        app_with(|app| {
            // 40 ms of game time per frame (as in frame capture): the
            // stepped server ticks twice a frame.
            app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(40),
            ))
            .add_plugins(crate::weather::WeatherPlugin);
        })
    }

    /// Runs `n` frames (the stepped local server ticks once per frame).
    fn frames(app: &mut App, n: usize) {
        for _ in 0..n {
            app.update();
        }
    }

    /// The target (or the weather now) of the session weather the client
    /// has from the server.
    fn server_weather(app: &App) -> ocw_weather::WeatherState {
        let info = app
            .world()
            .resource::<ClientState>()
            .0
            .weather()
            .info()
            .cloned()
            .expect("the server's weather");
        info.target.map_or(info.now, |t| t.0)
    }

    fn machine(app: &App) -> crate::weather::Machine {
        app.world()
            .resource::<crate::weather::WeatherMenu>()
            .machine
    }

    /// The weather machine with the keyboard alone: World → Weather →
    /// Presets (a grid: the arrows move), a preset with the transition,
    /// the custom editor's values, the transition, Back.
    #[test]
    fn the_weather_machine_with_the_keyboard() {
        let mut app = weather_app();
        frames(&mut app, 5);
        tap(&mut app, KeyCode::Escape);
        tap(&mut app, KeyCode::ArrowDown);
        tap(&mut app, KeyCode::ArrowDown);
        tap(&mut app, KeyCode::Enter);
        assert_eq!(view(&app).title, "World");
        for _ in 0..4 {
            tap(&mut app, KeyCode::ArrowDown);
        }
        tap(&mut app, KeyCode::Enter);
        let v = view(&app);
        assert_eq!(v.title, "Weather machine");
        assert!(v.status.ends_with("set by you"), "{}", v.status);
        assert_eq!(v.items[1], "Real weather:  off");
        assert!(!v.dimmed[1] && !v.dimmed[0]);
        // The transition: 1 min → 5 min with →.
        for _ in 0..4 {
            tap(&mut app, KeyCode::ArrowDown);
        }
        tap(&mut app, KeyCode::ArrowRight);
        frames(&mut app, 2);
        assert_eq!(machine(&app).transition_s, 300.0);
        assert_eq!(view(&app).items[4], "Transition:  < 5 min >");
        tap(&mut app, KeyCode::ArrowLeft);
        frames(&mut app, 2);
        assert_eq!(machine(&app).transition_s, 60.0);
        // Presets: the grid.
        tap(&mut app, KeyCode::ArrowUp);
        tap(&mut app, KeyCode::ArrowUp);
        tap(&mut app, KeyCode::Enter);
        let v = view(&app);
        assert_eq!(v.title, "Weather presets");
        let grid = v.grid.expect("a grid");
        assert_eq!((grid.columns, grid.cells.len()), (5, 18));
        assert!(grid.cells[0].marked, "clear now");
        assert!(v.hint.starts_with("Arrows move"));
        // → ↓ ↓: hail (row 3, column 2); Enter: in 1 min.
        tap(&mut app, KeyCode::ArrowRight);
        tap(&mut app, KeyCode::ArrowDown);
        tap(&mut app, KeyCode::ArrowDown);
        assert_eq!(view(&app).focused, 11);
        assert!(
            view(&app).status.starts_with("Hail:"),
            "{}",
            view(&app).status
        );
        tap(&mut app, KeyCode::Enter);
        frames(&mut app, 6);
        assert_eq!(
            server_weather(&app),
            ocw_weather::presets::preset("hail").unwrap()
        );
        let left = app
            .world()
            .resource::<crate::weather::WeatherMenu>()
            .target
            .map(|t| t.1);
        assert!(left.is_some_and(|l| l > 50.0 && l <= 60.0), "{left:?}");
        // Esc: back to the machine; Custom → cloud cover up.
        tap(&mut app, KeyCode::Escape);
        assert_eq!(view(&app).title, "Weather machine");
        tap(&mut app, KeyCode::ArrowDown);
        tap(&mut app, KeyCode::Enter);
        assert_eq!(view(&app).title, "Custom weather: sky");
        // The hail's cumulonimbus: cloud type → stratus (no lightning).
        tap(&mut app, KeyCode::ArrowDown);
        tap(&mut app, KeyCode::ArrowRight);
        frames(&mut app, 6);
        let w = server_weather(&app);
        assert_eq!(w.cloud_type, ocw_weather::CloudType::Cumulus, "wraps round");
        assert_eq!(w.lightning_per_min, 0.0);
        assert_eq!(view(&app).items[1], "Cloud type:  < cumulus >");
        assert_eq!(exits(&app), 0);
    }

    /// An Open-Meteo answer around now (rain, 3 mm/h, 9 °C), so the test
    /// does not depend on the day it runs.
    fn rainy_open_meteo() -> String {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let h0 = now - now % 3600 - 6 * 3600;
        let n = 13;
        let times: Vec<String> = (0..n).map(|i| (h0 + i * 3600).to_string()).collect();
        let col = |v: &str| vec![v; n as usize].join(",");
        format!(
            r#"{{"elevation":400.0,"hourly":{{"time":[{}],"temperature_2m":[{}],"relative_humidity_2m":[{}],"surface_pressure":[{}],"cloud_cover":[{}],"precipitation":[{}],"snowfall":[{}],"weather_code":[{}],"wind_speed_10m":[{}],"wind_gusts_10m":[{}],"wind_direction_10m":[{}]}}}}"#,
            times.join(","),
            col("9.0"),
            col("90"),
            col("960.0"),
            col("100"),
            col("3.0"),
            col("0.0"),
            col("63"),
            col("4.0"),
            col("9.0"),
            col("230")
        )
    }

    /// Real weather with the keyboard (spec §14 Phase 9, S7): World →
    /// Weather → Real weather; the data source to Open-Meteo, Enter turns
    /// it on – the stepped local server asks (recorded answers here, no
    /// network), the rain arrives with the attribution; ride a day queues
    /// the time machine and real weather.
    #[test]
    fn real_weather_with_the_keyboard() {
        let mut app = weather_app();
        let answer = rainy_open_meteo();
        crate::connection::with_stepped_server(app.world(), |s| {
            s.server_mut()
                .set_weather_fetcher(Box::new(ocw_server::realweather::FnFetcher::new(
                    move |f: &ocw_weather::real::feed::Fetch| match f.provider {
                        ocw_weather::real::Provider::OpenMeteo => Ok(answer.clone()),
                        p => Err(format!("{}: no sample", p.name())),
                    },
                )));
        })
        .expect("a stepped server");
        frames(&mut app, 5);
        tap(&mut app, KeyCode::Escape);
        tap(&mut app, KeyCode::ArrowDown);
        tap(&mut app, KeyCode::ArrowDown);
        tap(&mut app, KeyCode::Enter);
        for _ in 0..4 {
            tap(&mut app, KeyCode::ArrowDown);
        }
        tap(&mut app, KeyCode::Enter);
        assert_eq!(view(&app).title, "Weather machine");
        tap(&mut app, KeyCode::ArrowDown);
        tap(&mut app, KeyCode::Enter);
        let v = view(&app);
        assert_eq!(v.title, "Real weather");
        assert_eq!(v.items[0], "Real weather:  < off >");
        // The data source: ← from Automatic is Open-Meteo.
        tap(&mut app, KeyCode::ArrowDown);
        tap(&mut app, KeyCode::ArrowLeft);
        frames(&mut app, 2);
        assert_eq!(
            machine(&app).provider,
            ocw_weather::real::Choice::Only(ocw_weather::real::Provider::OpenMeteo)
        );
        assert_eq!(view(&app).items[1], "Data source:  < Open-Meteo >");
        // Enter on the first row: on.
        tap(&mut app, KeyCode::ArrowUp);
        tap(&mut app, KeyCode::Enter);
        frames(&mut app, 10);
        let v = view(&app);
        assert_eq!(v.items[0], "Real weather:  < on >", "{:?}", v.items);
        assert!(
            v.status.starts_with("Weather: Open-Meteo, CC BY 4.0"),
            "{}",
            v.status
        );
        let w = server_weather(&app);
        assert_eq!(w.precip, ocw_weather::Precip::Rain);
        assert!((w.precip_mm_h - 3.0).abs() < 0.01);
        assert_eq!(
            app.world()
                .resource::<ClientState>()
                .0
                .weather()
                .info()
                .unwrap()
                .mode,
            ocw_weather::Mode::Real
        );
        // The HUD's credit.
        let rv = app.world().resource::<RideView>();
        let a = ocw_render::hud::attribution_suffix(rv);
        assert_eq!(a, " | Weather: Open-Meteo, CC BY 4.0");
        // Ride a day: Enter on its row queues the lines.
        tap(&mut app, KeyCode::ArrowDown);
        tap(&mut app, KeyCode::ArrowDown);
        tap(&mut app, KeyCode::Enter);
        frames(&mut app, 2);
        let log = app.world().resource::<ConsoleState>().log().join("\n");
        assert!(log.contains("ride a day: Storm Lothar"), "{log}");
        // Off: the weather stays, fixed.
        tap(&mut app, KeyCode::ArrowUp);
        tap(&mut app, KeyCode::ArrowUp);
        tap(&mut app, KeyCode::Enter);
        frames(&mut app, 6);
        assert_eq!(view(&app).items[0], "Real weather:  < off >");
        assert_eq!(
            app.world()
                .resource::<ClientState>()
                .0
                .weather()
                .info()
                .unwrap()
                .mode,
            ocw_weather::Mode::Fixed
        );
        assert_eq!(exits(&app), 0);
    }

    /// The same with the gamepad alone: Start, the D-pad, A and B.
    #[test]
    fn the_weather_machine_with_the_gamepad() {
        use bevy::input::gamepad::{Gamepad, GamepadButton};
        let mut app = weather_app();
        frames(&mut app, 5);
        let pad = app.world_mut().spawn(Gamepad::default()).id();
        let press = |app: &mut App, b: GamepadButton| {
            app.world_mut()
                .get_mut::<Gamepad>(pad)
                .unwrap()
                .digital_mut()
                .press(b);
            app.update();
            let mut g = app.world_mut().get_mut::<Gamepad>(pad).unwrap();
            g.digital_mut().release(b);
            g.digital_mut().clear();
            app.update();
        };
        press(&mut app, GamepadButton::Start);
        press(&mut app, GamepadButton::DPadDown);
        press(&mut app, GamepadButton::DPadDown);
        press(&mut app, GamepadButton::South);
        for _ in 0..4 {
            press(&mut app, GamepadButton::DPadDown);
        }
        press(&mut app, GamepadButton::South);
        assert_eq!(view(&app).title, "Weather machine");
        // Random weather: the seed up, then A applies it.
        for _ in 0..5 {
            press(&mut app, GamepadButton::DPadDown);
        }
        press(&mut app, GamepadButton::DPadRight);
        frames(&mut app, 2);
        assert_eq!(machine(&app).seed, 2);
        press(&mut app, GamepadButton::South);
        frames(&mut app, 6);
        assert_eq!(server_weather(&app), ocw_weather::machine::random(2).1);
        // Presets with the D-pad: ← from the first cell wraps to Back.
        press(&mut app, GamepadButton::DPadUp);
        press(&mut app, GamepadButton::DPadUp);
        press(&mut app, GamepadButton::DPadUp);
        press(&mut app, GamepadButton::South);
        assert_eq!(view(&app).title, "Weather presets");
        press(&mut app, GamepadButton::DPadLeft);
        assert_eq!(view(&app).focused, 17);
        press(&mut app, GamepadButton::DPadLeft);
        press(&mut app, GamepadButton::DPadUp);
        assert_eq!(view(&app).focused, 11);
        // B: back.
        press(&mut app, GamepadButton::East);
        assert_eq!(view(&app).title, "Weather machine");
        // The mode: A switches to the simulation with the seed.
        press(&mut app, GamepadButton::DPadUp);
        press(&mut app, GamepadButton::DPadUp);
        press(&mut app, GamepadButton::South);
        frames(&mut app, 6);
        let c = &app.world().resource::<ClientState>().0;
        let info = c.weather().info().unwrap();
        assert_eq!(
            (info.mode, info.sim_seed),
            (ocw_weather::Mode::Simulated, 2)
        );
    }

    /// With the mouse alone (hover moves the focus, clicks select, the
    /// value rows' arrows step) and with touch alone (taps on the items
    /// and the arrows, the grid's Back cell instead of Esc).
    #[test]
    fn the_weather_machine_with_the_mouse_and_with_touch() {
        for touch in [false, true] {
            let mut app = weather_app();
            frames(&mut app, 5);
            let push = |app: &mut App, e| {
                app.world_mut().resource_mut::<MenuPointer>().0.push(e);
                app.update();
            };
            app.world_mut()
                .resource_mut::<ConsoleState>()
                .queue("menu open");
            frames(&mut app, 2);
            if !touch {
                app.insert_resource(AccumulatedMouseMotion {
                    delta: Vec2::new(4.0, 2.0),
                });
                push(&mut app, MenuPointerEvent::Hover(2));
                assert_eq!(view(&app).focused, 2);
            }
            push(&mut app, MenuPointerEvent::Click(2));
            push(&mut app, MenuPointerEvent::Click(4));
            assert_eq!(view(&app).title, "Weather machine");
            // Presets: tap / click a cell.
            push(&mut app, MenuPointerEvent::Click(2));
            assert!(view(&app).grid.is_some());
            push(&mut app, MenuPointerEvent::Click(4));
            frames(&mut app, 6);
            assert_eq!(
                server_weather(&app),
                ocw_weather::presets::preset("fog").unwrap()
            );
            // Back: the grid's Back cell (touch has no Esc), or a right click.
            if touch {
                push(&mut app, MenuPointerEvent::Click(17));
            } else {
                push(&mut app, MenuPointerEvent::Back);
            }
            assert_eq!(view(&app).title, "Weather machine");
            // Custom → Wind and temperature: the › arrow of the wind.
            push(&mut app, MenuPointerEvent::Click(3));
            push(&mut app, MenuPointerEvent::Click(5));
            assert_eq!(view(&app).title, "Custom: wind, temperature");
            let wind = server_weather(&app).wind_mps;
            push(&mut app, MenuPointerEvent::Step(0, 1));
            frames(&mut app, 6);
            // The fog's 0.5 m/s: up to the next whole metre per second.
            assert_eq!(wind, 0.5);
            assert_eq!(server_weather(&app).wind_mps, 1.0, "touch {touch}");
            // The ‹ arrow of the temperature.
            let temp = server_weather(&app).temperature_c;
            push(&mut app, MenuPointerEvent::Step(3, -1));
            frames(&mut app, 6);
            assert_eq!(server_weather(&app).temperature_c, temp - 1.0);
            // Back with the Back item (after the weather physics row, S6).
            push(&mut app, MenuPointerEvent::Click(5));
            assert_eq!(view(&app).title, "Custom weather: sky");
        }
    }

    /// An event's weather (spec §14 Phase 9, S5): set by the organiser
    /// with `event … --weather`, shown, and the machine dimmed until the
    /// event is over.
    #[test]
    fn an_events_weather_dims_the_machine_until_it_is_over() {
        let mut app = weather_app();
        frames(&mut app, 5);
        let run = |app: &mut App, line: &str| {
            app.world_mut().resource_mut::<ConsoleState>().queue(line);
            frames(app, 8);
        };
        run(
            &mut app,
            "event race --km 1 --bots 0 --weather heavy-snow temp=-8",
        );
        let mut want = ocw_weather::presets::preset("heavy-snow").unwrap();
        want.temperature_c = -8.0;
        assert_eq!(server_weather(&app), want);
        let st = app
            .world()
            .resource::<crate::weather::WeatherMenu>()
            .clone();
        assert!(!st.may_change);
        assert_eq!(
            st.event.as_deref(),
            Some("heavy snow, changed: heavy snow -8 °C")
        );
        // The menu shows it dimmed.
        run(&mut app, "menu open");
        tap(&mut app, KeyCode::ArrowDown);
        tap(&mut app, KeyCode::ArrowDown);
        tap(&mut app, KeyCode::Enter);
        for _ in 0..4 {
            tap(&mut app, KeyCode::ArrowDown);
        }
        tap(&mut app, KeyCode::Enter);
        let v = view(&app);
        assert_eq!(v.title, "Weather machine");
        assert!(v.dimmed[0] && v.dimmed[5], "{:?}", v.dimmed);
        assert!(
            v.status.ends_with("set by the event's organizer"),
            "{}",
            v.status
        );
        // The console is refused too; the weather stays.
        run(&mut app, "weather clear");
        assert_eq!(server_weather(&app), want);
        // The event is cancelled: the rider's again.
        run(&mut app, "event cancel");
        assert!(
            app.world()
                .resource::<crate::weather::WeatherMenu>()
                .may_change
        );
        run(&mut app, "weather clear");
        assert_eq!(
            server_weather(&app),
            ocw_weather::presets::preset("clear").unwrap()
        );
    }
}
