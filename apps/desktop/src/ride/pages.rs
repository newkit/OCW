//! The rows of the Ride and Navi lists (spec §3.2.1, M3), Bevy-free:
//! Routes, Rides, GPX import, Workouts (the World pages: [`super::world`]).
//! Rows that start another ride ask first ([`Activate::Ask`]) and run a
//! `ride.switch` / `ride.import-gpx` line; the menu pages them when they
//! are many.

use std::path::PathBuf;

use ocw_worldstore::discover::WorldEntry;

use super::catalog::{RideEntry, SkippedRoute, UserRoute};
use super::choice::{RideChoice, WorldMode};
use super::world::{RouteInfo, WorldDetails, km, route_value, short};
use crate::settings::pages::{Activate, Page, Row};

/// What the lists show: read from disk when the menu opens and after a
/// switch or an import (not every frame).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Catalog {
    /// World → Change world (`ocw_worldstore::discover`).
    pub worlds: Vec<WorldEntry>,
    /// The ride now.
    pub current: Option<RideChoice>,
    /// The loaded package: its spec, name and routes (name, m), and
    /// whether it has a road graph (free ride, GPX import).
    pub world: Option<LoadedWorld>,
    /// The user's routes for the loaded world: `routes/` next to the
    /// package (OCW Studio) and `<data dir>/ocw/routes/` (spec 16H Q19).
    pub user_routes: Vec<UserRoute>,
    /// Route files there that could not be read (listed as notes).
    pub skipped_routes: Vec<SkippedRoute>,
    pub rides: Vec<RideEntry>,
    pub gpx: Vec<PathBuf>,
    /// The local rider's position in the package's metres (x, z).
    pub here: Option<(f64, f64)>,
    /// Ride -> Workouts: the user's workouts and the bundled examples.
    pub workouts: Vec<WorkoutEntry>,
    /// The running workout: (name, "step 2/6"); kept current every frame.
    pub workout_now: Option<(String, String)>,
}

/// The Workouts list's entries (shared with the browser app).
pub use ocw_client::menu::pages::{WorkoutEntry, WorkoutSource};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LoadedWorld {
    pub spec: String,
    /// The package's own name (route files name it).
    pub name: String,
    pub routes: Vec<RouteInfo>,
    pub has_graph: bool,
    /// What World → About this world shows.
    pub details: WorldDetails,
}

fn info(label: &str, value: Option<String>) -> Row {
    Row {
        label: label.into(),
        value,
        left: None,
        right: None,
        confirm: None,
    }
}

fn ask(label: impl Into<String>, value: impl Into<String>, line: String) -> Row {
    Row {
        label: label.into(),
        value: Some(value.into()),
        left: None,
        right: None,
        confirm: Some(Activate::Ask(line)),
    }
}

/// A row label of at most this many characters fits in two lines.
#[cfg(test)]
pub use ocw_client::menu::pages::MAX_ROW_CHARS;

/// The rows of a Ride / Navi list (`page.is_ride_page()`); empty for a
/// World page whose index is no longer listed.
pub fn rows(page: Page, c: &Catalog) -> Vec<Row> {
    match page {
        Page::Routes => {
            let Some(w) = &c.world else {
                return vec![info(
                    "No world loaded",
                    Some("World -> Change world".into()),
                )];
            };
            let route = |r: &str| {
                RideChoice::World {
                    world: w.spec.clone(),
                    mode: WorldMode::Route(r.into()),
                }
                .to_line()
            };
            let mut out: Vec<Row> = w
                .routes
                .iter()
                .map(|r| ask(short(&r.name, 24), route_value(r), route(&r.name)))
                .collect();
            out.extend(c.user_routes.iter().map(|r| {
                ask(
                    short(&r.name, 24),
                    format!("{}, yours", km(r.length_m)),
                    route(&r.path.display().to_string()),
                )
            }));
            if out.is_empty() {
                out.push(info("No routes here", Some("Navi -> GPX import".into())));
            }
            // A broken route file is a note, not a ride.
            out.extend(c.skipped_routes.iter().map(|s| {
                let file = s.path.file_name().map_or_else(
                    || s.path.display().to_string(),
                    |n| n.to_string_lossy().into(),
                );
                // The kind of problem ("not a route"); the log has the rest.
                let kind = s.why.split(':').next().unwrap_or_default();
                info(
                    &short(&file, 18),
                    Some(short(&format!("skipped: {kind}"), 27)),
                )
            }));
            out
        }
        Page::Rides => {
            if c.rides.is_empty() {
                return vec![info("No saved rides yet", None)];
            }
            c.rides
                .iter()
                .map(|r| {
                    info(
                        &r.date,
                        Some(format!("{} - {}", short(&r.route, 14), r.strava)),
                    )
                })
                .collect()
        }
        Page::GpxImport => {
            if !c.world.as_ref().is_some_and(|w| w.has_graph) {
                return vec![info(
                    "Load a world first",
                    Some("World -> Change world".into()),
                )];
            }
            if c.gpx.is_empty() {
                return vec![info("No GPX files found", None)];
            }
            c.gpx
                .iter()
                .map(|p| {
                    let name = p
                        .file_name()
                        .map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into());
                    ask(
                        short(&name, 24),
                        "import and ride",
                        format!("ride.import-gpx \"{}\" --ride", p.display()),
                    )
                })
                .collect()
        }
        Page::Workouts => {
            ocw_client::menu::pages::workout_rows(c.workout_now.as_ref(), &c.workouts)
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menu::model::{Action, Effect, Menu, Page as MenuPage, row_label};

    fn catalog() -> Catalog {
        Catalog {
            worlds: Vec::new(),
            current: None,
            world: Some(LoadedWorld {
                spec: "/data/w1.ocwpkg".into(),
                name: "World 1".into(),
                routes: vec![RouteInfo {
                    name: "Valley".into(),
                    length_m: 12_400.0,
                    climb_m: None,
                }],
                has_graph: true,
                details: WorldDetails::default(),
            }),
            user_routes: vec![UserRoute {
                path: "/home/r/My tour.ocw-graph-route".into(),
                name: "My tour".into(),
                length_m: 5_000.0,
                source: super::super::catalog::RouteSource::NextToPackage,
            }],
            skipped_routes: Vec::new(),
            rides: vec![RideEntry {
                file: "2026-11-11_17-30-05.fit".into(),
                date: "2026-11-11 17:30".into(),
                route: "Hills".into(),
                strava: "uploaded",
            }],
            gpx: vec!["/home/u/Downloads/morning.gpx".into()],
            here: Some((120.4, -88.0)),
            workouts: vec![
                WorkoutEntry {
                    name: "My intervals".into(),
                    detail: "45:00".into(),
                    source: WorkoutSource::File("/home/u/ocw/workouts/my intervals.erg".into()),
                },
                WorkoutEntry {
                    name: "Demo: 2-minute intervals".into(),
                    detail: "2:00".into(),
                    source: WorkoutSource::Example("short-demo".into()),
                },
            ],
            workout_now: None,
        }
    }

    /// The menu over `c`'s rows, opened at `page`.
    fn open(c: &Catalog, path: &[usize]) -> Menu {
        let mut m = Menu::default();
        let rows = |p: Page| rows(p, c);
        m.apply_with(Action::Open, rows);
        for &i in path {
            m.apply_with(Action::Click(i), rows);
        }
        m
    }

    #[test]
    fn routes_rides_and_gpx_rows_run_their_commands() {
        let c = catalog();
        let rows_of = |p: Page| rows(p, &c);
        // Top → Navi → Routes (Navi is after World and Workshop).
        let mut m = open(&c, &[4, 0]);
        assert_eq!(
            m.labels(),
            ["Valley:  12.4 km", "My tour:  5.0 km, yours", "Back"]
        );
        m.apply_with(Action::Click(1), rows_of);
        assert_eq!(
            m.pending(),
            Some(
                "ride.switch --world \"/data/w1.ocwpkg\" --route \"/home/r/My tour.ocw-graph-route\""
            )
        );
        // Rides: shown, nothing to run.
        let mut m = open(&c, &[4, 1]);
        assert_eq!(m.labels(), ["2026-11-11 17:30:  Hills - uploaded", "Back"]);
        assert_eq!(m.apply_with(Action::Click(0), rows_of), Effect::None);
        // GPX import: import and ride, after asking.
        let mut m = open(&c, &[4, 3]);
        assert_eq!(m.labels()[0], "morning.gpx:  import and ride");
        m.apply_with(Action::Click(0), rows_of);
        assert_eq!(
            m.apply_with(Action::Up, rows_of),
            Effect::None,
            "to Ride now"
        );
        assert_eq!(
            m.apply_with(Action::Confirm, rows_of),
            Effect::Command("ride.import-gpx \"/home/u/Downloads/morning.gpx\" --ride".into())
        );
        // Strava: the account page (moved from Settings, which is full).
        let m = open(&c, &[4, 4]);
        assert_eq!(
            m.page(),
            Some(MenuPage::Sub(crate::settings::pages::Page::Strava))
        );
    }

    #[test]
    fn a_skipped_route_file_is_a_note_after_the_routes() {
        let mut c = catalog();
        c.skipped_routes = vec![SkippedRoute {
            path: "/data/routes/torn.ocwroute".into(),
            why: "not a route: not an ocw-graph-route 1 file".into(),
        }];
        let r = rows(Page::Routes, &c);
        let labels: Vec<String> = r.iter().map(row_label).collect();
        assert_eq!(labels.len(), 3, "{labels:?}");
        assert_eq!(labels[1], "My tour:  5.0 km, yours");
        assert!(
            labels[2].starts_with("torn.ocwroute:  skipped: not a route"),
            "{labels:?}"
        );
        assert_eq!(r[2].confirm, None, "a note starts nothing");
        assert!(labels[2].chars().count() <= MAX_ROW_CHARS, "{}", labels[2]);
    }

    #[test]
    fn without_a_world_the_lists_say_what_to_do() {
        let c = Catalog::default();
        for p in [Page::Routes, Page::Rides, Page::GpxImport] {
            let r = rows(p, &c);
            assert_eq!(r.len(), 1, "{p:?}");
            assert_eq!(r[0].confirm, None, "{p:?}");
        }
    }

    #[test]
    fn workouts_start_from_the_menu_and_can_be_skipped_and_stopped() {
        let mut c = catalog();
        // Top → Ride → Workouts.
        let mut m = open(&c, &[1, 1]);
        assert_eq!(m.page(), Some(MenuPage::Sub(Page::Workouts)));
        assert_eq!(
            m.labels(),
            [
                "My intervals:  45:00",
                "Demo: 2-minute intervals:  2:00, example",
                "Back"
            ]
        );
        // A workout starts at once and the menu closes (its panel shows).
        assert_eq!(
            m.apply_with(Action::Click(0), |p| rows(p, &c)),
            Effect::Command("workout start \"/home/u/ocw/workouts/my intervals.erg\"".into())
        );
        assert!(!m.is_open());
        let mut m = open(&c, &[1, 1]);
        assert_eq!(
            m.apply_with(Action::Click(1), |p| rows(p, &c)),
            Effect::Command("workout start --example short-demo".into())
        );
        // While one runs: its step, Skip (stays open) and Stop (closes).
        c.workout_now = Some(("Demo: 2-minute intervals".into(), "step 2/6".into()));
        let mut m = open(&c, &[1, 1]);
        let l = m.labels();
        assert_eq!(l[0], "Demo: 2-minute intervals:  step 2/6");
        assert_eq!(&l[1..3], ["Skip step", "Stop workout"]);
        assert_eq!(
            m.apply_with(Action::Click(1), |p| rows(p, &c)),
            Effect::Command("workout skip".into())
        );
        assert!(m.is_open());
        assert_eq!(
            m.apply_with(Action::Click(2), |p| rows(p, &c)),
            Effect::Command("workout stop".into())
        );
        assert!(!m.is_open());
        // Nothing found: a hint instead of an empty page.
        let empty = Catalog::default();
        let r = rows(Page::Workouts, &empty);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].confirm, None);
    }

    #[test]
    fn rows_fit_the_ten_foot_limits() {
        let mut c = catalog();
        c.world.as_mut().unwrap().routes[0].name =
            "A very long route name that goes on and on".into();
        for p in [Page::Routes, Page::Rides, Page::GpxImport, Page::Workouts] {
            for row in rows(p, &c) {
                let l = row_label(&row);
                // Two lines of the menu panel at most.
                assert!(l.chars().count() <= MAX_ROW_CHARS, "{p:?}: {l}");
                let missing = ocw_render::font::missing(&l);
                assert!(missing.is_empty(), "{p:?}: {l}: {missing:?}");
            }
            for t in p.notes() {
                assert!(ocw_render::font::missing(t).is_empty(), "{p:?}: {t}");
            }
        }
        assert_eq!(short("abcdef", 5), "ab...");
    }
}
