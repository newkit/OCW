//! World → Time → Calendar and World → Time → Season in both apps (spec
//! §14 Phase 9, S9; shared like the time machine, B6): what the calendar
//! knows of the clock and the place ([`CalendarMenu`], refreshed every
//! frame), the season year of the rider's place and altitude (the season
//! model of S8 on the renderer's season buckets, so a day's band shows
//! what the world shows that day), and `time season …` (decision Q11: the
//! date jumps to the season's representative day, the time of day kept).
//! The page's rows and view are `ocw_client::menu::calendar`.

use std::sync::Arc;

use bevy::prelude::*;
use ocw_client::menu::calendar::{CalendarInfo, status_date};
use ocw_client::menu::model::Page;
use ocw_client::menu::pages::Page as Sub;
use ocw_render::RideView;
use ocw_render::region_look::{bucket_day, season_bucket};
use ocw_sky::calendar::{self as cal, SeasonYear};
use ocw_sky::season::SeasonModel;

use crate::timemachine::{TimeMachine, WorldPlace, place_latlon};

/// The calendar's knowledge of the clock and the place, as a resource.
#[derive(Resource, Debug, Clone, Default, Deref, DerefMut)]
pub struct CalendarMenu(pub CalendarInfo);

/// The season year is computed again when the place's climate (the
/// renderer's model key), the rider's altitude (per 25 m) or the measured
/// broadleaf share around (per 5 %) changes.
#[derive(Resource, Debug, Default, PartialEq)]
struct YearKey(Option<((&'static str, i32), i64, i32)>);

/// The broadleaf share around the rider, with the package and the place
/// (in 1/400 degrees) it was measured for.
#[derive(Resource, Debug, Default)]
struct ForestNear(Option<(usize, i64, i64)>, Option<f64>);

/// Half the side of the square around the rider whose forests count for
/// the season strength (m; spec §14 Phase 9 Q13).
pub const FOREST_AROUND_M: f64 = 1500.0;

/// The measured broadleaf share (0 – 1) of the forests within
/// [`FOREST_AROUND_M`] of (lat, lon) in the home package, if it has a forest
/// section with trees there.
pub fn broadleaf_around(world: &World, lat: f64, lon: f64) -> Option<f64> {
    let pkg = world
        .get_resource::<ocw_render::package::PackageWorld>()?
        .home()
        .clone();
    let forest = pkg.look.forest.as_ref()?;
    let [x, z] = ocw_world::geo::LocalProjection::new(pkg.origin.0, pkg.origin.1).project(lat, lon);
    let r = FOREST_AROUND_M;
    let (_, b) = forest.mean_in(f64::from(pkg.tile_size_m), [x - r, z - r], [x + r, z + r])?;
    b.map(f64::from)
}

/// The day the season model is evaluated on for a day of the year: the
/// middle of its season bucket, as the renderer (`ocw_render::season`).
pub fn model_day(doy: u32) -> f64 {
    bucket_day(season_bucket(doy))
}

/// The real date now at the place (Today).
fn today(zone: &ocw_sky::Zone) -> cal::Date {
    let ms = (ocw_render::stamp::wall_unix_s() * 1000.0) as i64;
    let l = zone.local(ocw_sky::time::clamp_range(ms));
    (l.year, l.month, l.day)
}

/// Refreshes [`CalendarMenu`] from the clock, the place and the rider
/// (after the time machine's sky, `ViewExtrasSet`).
pub fn update(world: &mut World) {
    let now = crate::now(world);
    let zone = world.resource::<WorldPlace>().zone.clone();
    let (latlon, alt, observed) = match (world.get_resource::<RideView>(), crate::client(world)) {
        (Some(v), Some(c)) => (
            v.sky
                .as_ref()
                .map_or_else(|| place_latlon(v, c), |s| s.sky.place),
            ocw_render::season::rider_alt(v),
            v.weather.as_ref().and_then(|w| w.snow),
        ),
        _ => (ocw_sky::zone::DEFAULT_PLACE, 500.0, None),
    };
    let (ms, may) = match crate::client_mut(world) {
        Some(mut c) => (c.world_ms(now), c.may_change_clock()),
        None => (None, false),
    };
    let control = world
        .get_resource::<TimeMachine>()
        .map(|t| t.control.clone())
        .unwrap_or_default();
    // The forests around, again when the rider has moved about 250 m or
    // the home package changed.
    let near_key = (
        world
            .get_resource::<ocw_render::package::PackageWorld>()
            .map_or(0, |p| Arc::as_ptr(p.home()) as usize),
        (latlon.0 * 400.0).round() as i64,
        (latlon.1 * 400.0).round() as i64,
    );
    let cached = world
        .get_resource::<ForestNear>()
        .filter(|f| f.0 == Some(near_key))
        .map(|f| f.1);
    let broadleaf = match cached {
        Some(b) => b,
        None => {
            let b = broadleaf_around(world, latlon.0, latlon.1);
            world.insert_resource(ForestNear(Some(near_key), b));
            b
        }
    };
    let key = (
        ocw_render::season::model_key(latlon.0, latlon.1),
        (alt / 25.0).round() as i64,
        broadleaf.map_or(-1, |b| (b * 20.0).round() as i32),
    );
    let prev = world.get_resource::<YearKey>().and_then(|k| k.0);
    let rebuild = prev != Some(key);
    let mut info = world.resource_mut::<CalendarMenu>();
    if rebuild {
        // The model (expensive) only for another climate; the year for
        // another altitude.
        let model = match (&info.model, prev) {
            (Some(m), Some(p)) if p.0 == key.0 => m.clone(),
            _ => Arc::new(SeasonModel::for_place(latlon.0, latlon.1)),
        };
        info.year = Some(Arc::new(SeasonYear::with_forest(
            &model, alt, model_day, broadleaf,
        )));
        info.model = Some(model);
        info.line = None;
    }
    let date = ms.map(|ms| zone.local(ms));
    info.date = date.map(|l| (l.year, l.month, l.day));
    info.minute = date.map_or(0, |l| (l.hour * 60 + l.minute) as u16);
    info.may_change = may;
    info.control = control;
    if info.zone.as_ref() != Some(&zone) {
        info.moon_for = None;
        info.line = None;
    }
    info.today = Some(today(&zone));
    info.zone = Some(zone);
    info.place = latlon;
    info.alt_m = alt;
    info.broadleaf = broadleaf;
    // Real weather's measured snow on the world date (S7 / S8).
    info.measured = match (observed, info.date, &info.model) {
        (Some(obs), Some(d), Some(model)) => {
            let doy = cal::day_of_year(d);
            let day = model.day(model_day(doy));
            let snow = day.snow_ramp(alt, Some(obs)).at(alt);
            let s = day.state(alt);
            let look = cal::look_of(&day.climate, &s, snow);
            let clim = info.year.as_ref().map(|y| y.look(d));
            (Some(look) != clim).then_some(look)
        }
        _ => None,
    };
    if rebuild {
        world.insert_resource(YearKey(Some(key)));
    }
}

/// The open calendar page's moon phases and its focused day's line
/// (between the menu's input and its view).
pub fn prepare(world: &mut World) {
    let (page, focus) = {
        let state = world.resource::<crate::menu::MenuState>();
        (state.menu.page(), state.menu.focus())
    };
    let Some(Page::Sub(Sub::Calendar(c))) = page else {
        return;
    };
    let Some(mut info) = world.get_resource_mut::<CalendarMenu>() else {
        return;
    };
    info.ensure_moon(c.year, c.month);
    if let Some(d) = status_date(&c, &info, focus) {
        info.ensure_line(d);
    }
}

/// `time season PICK`: the clock change to the season's day of the place.
pub fn season_change(
    world: &World,
    pick: ocw_sky::machine::SeasonPick,
    ms: i64,
    zone: &ocw_sky::Zone,
) -> Result<(ocw_sky::ClockChange, cal::Date), String> {
    let year = world
        .get_resource::<CalendarMenu>()
        .and_then(|c| c.year.clone())
        .ok_or("no season model yet")?;
    cal::season_change(pick, ms, zone, &year)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bevy::time::TimeUpdateStrategy;
    use ocw_client::client::ClientCore;
    use ocw_client::menu::pages::{self as sub, WorldHome};
    use ocw_render::console::{ConsoleCorePlugin, ConsoleState};
    use ocw_render::menu::MenuView;
    use ocw_server::local::FrameServer;
    use ocw_world::Route;

    use super::*;
    use crate::Platform;
    use crate::menu::MenuHooks;

    /// A ride on the in-memory local server, as the apps reach it.
    #[derive(Resource)]
    struct Ride {
        client: ClientCore,
        server: FrameServer,
    }

    fn step_ride(time: Res<Time>, mut ride: ResMut<Ride>) {
        let now = time.elapsed_secs_f64();
        let Ride { client, server } = &mut *ride;
        server.advance(time.delta_secs_f64());
        for m in server.drain() {
            let _ = client.on_server_message(&m, now);
        }
        client.predict_step();
    }

    fn app(platform: Platform) -> App {
        let route = Route::test_loop();
        let client = ClientCore::new(route.clone(), "Ada Rider", Default::default());
        let mut server = FrameServer::new(route.clone(), 0, 0.0);
        server.send(&client.hello());
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ConsoleCorePlugin))
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
                20,
            )))
            .init_resource::<ButtonInput<KeyCode>>()
            .insert_resource(crate::UiPlatform(platform))
            .insert_resource(RideView::new(route))
            .insert_resource(Ride { client, server })
            .insert_resource(crate::RideAccess {
                client: |w| w.get_resource::<Ride>().map(|r| &r.client),
                client_mut: |w| {
                    w.get_resource_mut::<Ride>()
                        .map(|r| r.map_unchanged(|r| &mut r.client))
                },
                send: |w, bytes| {
                    if let Some(mut r) = w.get_resource_mut::<Ride>() {
                        r.server.send(&bytes);
                    }
                },
            })
            // The World page as the browser builds it (the desktop's has
            // its catalog first; Time is a row of both).
            .insert_resource(MenuHooks {
                rows: |w, p| {
                    (p == sub::Page::WorldHome).then(|| {
                        let tm = w.resource::<TimeMachine>().0.clone();
                        let weather = w.resource::<crate::weather::WeatherMenu>().0.clone();
                        let home = WorldHome {
                            name: "Test loop".into(),
                            ..WorldHome::default()
                        };
                        sub::world_home_rows(&home, &tm, &weather)
                    })
                },
                ..MenuHooks::default()
            })
            .add_systems(Update, step_ride.in_set(crate::RideViewSet))
            .add_plugins((
                crate::settings::SettingsPlugin,
                crate::weather::WeatherPlugin,
                crate::timemachine::TimeMachinePlugin,
                crate::riders_page::RidersPagePlugin,
                crate::menu::MenuPlugin,
                // `season` and `season set` (the renderer's console command).
                ocw_render::season::SeasonPlugin,
            ));
        app
    }

    fn frames(app: &mut App, n: usize) {
        for _ in 0..n {
            app.update();
        }
    }

    fn tap(app: &mut App, keys: &[KeyCode]) {
        let mut input = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        for k in keys {
            input.press(*k);
        }
        app.update();
        let mut input = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        for k in keys {
            input.release(*k);
        }
        input.clear();
        app.update();
    }

    fn view(app: &App) -> MenuView {
        app.world().resource::<MenuView>().clone()
    }

    fn date(app: &App) -> Option<cal::Date> {
        app.world().resource::<CalendarMenu>().date
    }

    fn run(app: &mut App, line: &str) -> String {
        app.world_mut().resource_mut::<ConsoleState>().queue(line);
        frames(app, 3);
        app.world()
            .resource::<ConsoleState>()
            .log()
            .iter()
            .rev()
            .find(|l| !l.trim().is_empty() && !l.starts_with('>'))
            .cloned()
            .unwrap_or_default()
    }

    /// Down to the item starting with `prefix`, then Enter.
    fn open(app: &mut App, prefix: &str) {
        let v = view(app);
        let at = v
            .items
            .iter()
            .position(|i| i.starts_with(prefix))
            .unwrap_or_else(|| panic!("{prefix}: {:?}", v.items));
        for _ in 0..(at + v.items.len() - v.focused) % v.items.len() {
            tap(app, &[KeyCode::ArrowDown]);
        }
        tap(app, &[KeyCode::Enter]);
    }

    /// World → Time → Calendar by keys, in both apps (B6 parity): the
    /// month of the world date with the focus on it, ← / → by day with the
    /// day's line, Enter sets the date (the server's clock follows), Page
    /// Down a month, Shift+→ a minute on the stepper; Esc back. World →
    /// Time → Season jumps the date to the place's season, the time of
    /// day kept (Q11); `time season` and `season set` do the same.
    #[test]
    fn the_calendar_by_keys_in_both_apps() {
        use ocw_client::menu::calendar::{SELECT, TIME};
        for platform in [Platform::Desktop, Platform::Browser] {
            let mut app = app(platform);
            frames(&mut app, 10);
            run(&mut app, "time 2026-11-16 14:20");
            frames(&mut app, 5);
            assert_eq!(date(&app), Some((2026, 11, 16)), "{platform:?}");
            tap(&mut app, &[KeyCode::Escape]);
            open(&mut app, "World");
            open(&mut app, "Time");
            assert_eq!(view(&app).title, "Time machine");
            assert_eq!(view(&app).status, "Set by you");
            open(&mut app, "Day");
            let v = view(&app);
            assert_eq!(v.title, "November 2026", "{platform:?}");
            let c = v.calendar.clone().expect("a calendar");
            assert_eq!(v.focused, 21, "16 November");
            assert!(c.days[21].selected);
            assert_eq!(c.days.iter().filter(|d| d.band.is_some()).count(), 42);
            assert!(c.days.iter().any(|d| d.moon.as_deref() == Some("●")));
            assert!(
                v.status.starts_with("Mon 16 Nov 2026: sunrise"),
                "{}",
                v.status
            );
            assert!(v.hint.contains("PgUp/PgDn"), "{}", v.hint);
            // → a day, Enter sets it.
            tap(&mut app, &[KeyCode::ArrowRight]);
            assert!(view(&app).status.starts_with("Tue 17 Nov 2026"));
            tap(&mut app, &[KeyCode::Enter]);
            frames(&mut app, 5);
            assert_eq!(date(&app), Some((2026, 11, 17)), "{platform:?}");
            let v = view(&app);
            assert!(v.calendar.as_ref().unwrap().days[22].selected);
            assert_eq!(app.world().resource::<CalendarMenu>().minute, 14 * 60 + 20);
            // Page Down: December, the day kept.
            tap(&mut app, &[KeyCode::PageDown]);
            let v = view(&app);
            assert_eq!(v.title, "December 2026");
            assert!(v.status.starts_with("Thu 17 Dec 2026"), "{}", v.status);
            // Down past the last week (Thursday's column): Select; ← the
            // time stepper; Shift+→ a minute.
            for _ in 0..3 {
                tap(&mut app, &[KeyCode::ArrowDown]);
            }
            assert_eq!(view(&app).focused, SELECT);
            tap(&mut app, &[KeyCode::ArrowLeft]);
            assert_eq!(view(&app).focused, TIME);
            tap(&mut app, &[KeyCode::ShiftLeft, KeyCode::ArrowRight]);
            tap(&mut app, &[KeyCode::ArrowRight]);
            let v = view(&app);
            assert!(v.items[TIME].contains("14:36"), "{}", v.items[TIME]);
            // Esc: back, the date unchanged by browsing.
            tap(&mut app, &[KeyCode::Escape]);
            assert_eq!(view(&app).title, "Time machine");
            assert_eq!(date(&app), Some((2026, 11, 17)));
            // World → Time → Season → Summer: the date jumps, 14:20 kept.
            open(&mut app, "Season");
            assert_eq!(view(&app).title, "Season");
            open(&mut app, "Summer");
            frames(&mut app, 5);
            let info = app.world().resource::<CalendarMenu>().0.clone();
            let year = info.year.clone().unwrap();
            let summer = year.season_date(cal::Season::Summer, (2026, 11, 17));
            assert_eq!(info.date, Some(summer), "{platform:?}");
            assert_eq!(info.minute, 14 * 60 + 20, "the time of day kept");
            assert_eq!(view(&app).title, "Time machine");
            // The console: `time season`, `season set`, `time date`.
            let out = run(&mut app, "time season winter");
            assert!(out.contains("winter at your place"), "{out}");
            frames(&mut app, 3);
            let d = date(&app).unwrap();
            assert_eq!(d, year.season_date(cal::Season::Winter, summer), "{d:?}");
            run(&mut app, "season set autumn");
            frames(&mut app, 5);
            let d = date(&app).unwrap();
            assert_eq!(year.look(d), cal::Look::Autumn, "{d:?}");
            run(&mut app, "time date 2026-05-03 07:45");
            frames(&mut app, 3);
            assert_eq!(date(&app), Some((2026, 5, 3)));
            assert_eq!(app.world().resource::<CalendarMenu>().minute, 7 * 60 + 45);
        }
    }

    /// Regression (user feedback 2026-11-17: "i miss the select button
    /// that accepts and sets the selected date"): the calendar has a Select
    /// button showing the selected day; Enter (A) on it sets that day on
    /// the server's clock, the time of day kept, and goes back to World →
    /// Time; Esc after browsing changes nothing. Both apps.
    #[test]
    fn select_applies_the_selected_date_in_both_apps() {
        use ocw_client::menu::calendar::SELECT;
        for platform in [Platform::Desktop, Platform::Browser] {
            let mut app = app(platform);
            frames(&mut app, 10);
            run(&mut app, "time 2026-11-16 14:20");
            frames(&mut app, 5);
            tap(&mut app, &[KeyCode::Escape]);
            open(&mut app, "World");
            open(&mut app, "Time");
            open(&mut app, "Day");
            assert_eq!(view(&app).focused, 21, "16 November");
            // → → → Thursday the 19th, ↓ the 26th, ↓ Select.
            for k in [
                KeyCode::ArrowRight,
                KeyCode::ArrowRight,
                KeyCode::ArrowRight,
                KeyCode::ArrowDown,
                KeyCode::ArrowDown,
            ] {
                tap(&mut app, &[k]);
            }
            let v = view(&app);
            assert_eq!(v.focused, SELECT, "{platform:?}");
            assert_eq!(v.items[SELECT], "Select Thu 26 Nov 2026");
            assert!(!v.dimmed[SELECT]);
            assert!(v.status.starts_with("Thu 26 Nov 2026"), "{}", v.status);
            assert_eq!(
                v.calendar.as_ref().unwrap().controls[SELECT - 42],
                "Select Thu 26 Nov 2026"
            );
            assert_eq!(date(&app), Some((2026, 11, 16)), "browsing sets nothing");
            tap(&mut app, &[KeyCode::Enter]);
            frames(&mut app, 5);
            assert_eq!(date(&app), Some((2026, 11, 26)), "{platform:?}");
            assert_eq!(app.world().resource::<CalendarMenu>().minute, 14 * 60 + 20);
            let v = view(&app);
            assert_eq!(v.title, "Time machine", "back to World → Time");
            assert!(
                v.items.iter().any(|i| i.contains("Thu 26 Nov 2026")),
                "{:?}",
                v.items
            );
            // Browse again and Esc: the date kept.
            open(&mut app, "Day");
            assert_eq!(view(&app).items[SELECT], "Select Thu 26 Nov 2026");
            tap(&mut app, &[KeyCode::ArrowRight]);
            tap(&mut app, &[KeyCode::Escape]);
            frames(&mut app, 5);
            assert_eq!(view(&app).title, "Time machine");
            assert_eq!(date(&app), Some((2026, 11, 26)));
        }
    }
}
