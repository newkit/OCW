//! The rows of the Ride and Navi lists (spec §3.2.1, M3), Bevy-free: World
//! packages, one package, Routes, Rides, GPX import. Rows that start
//! another ride ask first ([`Activate::Ask`]) and run a `ride.switch` /
//! `ride.import-gpx` line; the menu pages them when they are many.

use std::path::PathBuf;

use super::catalog::{KnownWorld, RideEntry, UserRoute};
use super::choice::{RideChoice, WorldMode};
use crate::settings::pages::{Activate, Page, Row};

/// What the lists show: read from disk when the menu opens and after a
/// switch or an import (not every frame).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Catalog {
    pub worlds: Vec<KnownWorld>,
    /// The ride now.
    pub current: Option<RideChoice>,
    /// The loaded package: its spec, name and routes (name, m), and
    /// whether it has a road graph (free ride, GPX import).
    pub world: Option<LoadedWorld>,
    pub user_routes: Vec<UserRoute>,
    pub rides: Vec<RideEntry>,
    pub gpx: Vec<PathBuf>,
    /// The local rider's position in the package's metres (x, z).
    pub here: Option<(f64, f64)>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LoadedWorld {
    pub spec: String,
    pub name: String,
    pub routes: Vec<(String, f64)>,
    pub has_graph: bool,
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

fn km(m: f64) -> String {
    format!("{:.1} km", m / 1000.0)
}

/// Shortens `s` to `max` characters (labels must fit the panel).
fn short(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max - 3).collect();
        t.push_str("...");
        t
    }
}

impl Catalog {
    /// Whether the package `spec` is the one ridden now.
    fn is_current(&self, spec: &str) -> bool {
        self.world.as_ref().is_some_and(|w| w.spec == spec)
    }
}

/// The rows of a Ride / Navi list (`page.is_ride_page()`); empty for a
/// World page whose index is no longer listed.
pub fn rows(page: Page, c: &Catalog) -> Vec<Row> {
    match page {
        Page::Worlds => {
            if c.worlds.is_empty() {
                return vec![info(
                    "No world packages found",
                    Some("see README: world packages".into()),
                )];
            }
            c.worlds
                .iter()
                .enumerate()
                .map(|(i, w)| Row {
                    label: short(&w.name, 28),
                    value: Some(if c.is_current(&w.spec) {
                        format!("{} - riding", w.detail)
                    } else {
                        w.detail.clone()
                    }),
                    left: None,
                    right: None,
                    confirm: Some(Activate::Open(Page::World(i))),
                })
                .collect()
        }
        Page::World(i) => {
            let Some(w) = c.worlds.get(i) else {
                return Vec::new();
            };
            let free = |start| RideChoice::World {
                world: w.spec.clone(),
                mode: WorldMode::FreeRide { start },
            };
            let mut out = vec![ask("Free ride", "random start", free(None).to_line())];
            if c.is_current(&w.spec) {
                if let Some(here) = c.here
                    && c.world.as_ref().is_some_and(|l| l.has_graph)
                {
                    out.push(ask(
                        "Free ride from here",
                        format!("{:.0}, {:.0}", here.0, here.1),
                        free(Some(here)).to_line(),
                    ));
                }
                let n = c.world.as_ref().map_or(0, |l| l.routes.len()) + c.user_routes.len();
                out.push(Row {
                    label: "Routes".into(),
                    value: Some(format!("{n}")),
                    left: None,
                    right: None,
                    confirm: Some(Activate::Open(Page::Routes)),
                });
            } else {
                out.push(info(
                    "Routes",
                    Some("listed once you ride here (Navi)".into()),
                ));
            }
            out.push(info("Package", Some(short(&w.detail, 30))));
            out
        }
        Page::Routes => {
            let Some(w) = &c.world else {
                return vec![info(
                    "No world package loaded",
                    Some("Ride -> World packages".into()),
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
                .map(|(name, m)| ask(short(name, 28), km(*m), route(name)))
                .collect();
            out.extend(c.user_routes.iter().map(|r| {
                ask(
                    short(&r.name, 28),
                    format!("{}, yours", km(r.length_m)),
                    route(&r.path.display().to_string()),
                )
            }));
            if out.is_empty() {
                out.push(info(
                    "No routes in this package",
                    Some("import one: Navi -> GPX import".into()),
                ));
            }
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
                        Some(format!("{} - {}", short(&r.route, 20), r.strava)),
                    )
                })
                .collect()
        }
        Page::GpxImport => {
            let Some(w) = c.world.as_ref().filter(|w| w.has_graph) else {
                return vec![info(
                    "Load a world package first",
                    Some("Ride -> World packages".into()),
                )];
            };
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
                        short(&name, 28),
                        format!("import, ride in {}", short(&w.name, 16)),
                        format!("ride.import-gpx \"{}\" --ride", p.display()),
                    )
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menu::model::{Action, Effect, MAX_ITEMS, Menu, Page as MenuPage, row_label};

    fn catalog() -> Catalog {
        Catalog {
            worlds: (0..9)
                .map(|i| KnownWorld {
                    spec: format!("/data/w{i}.ocwpkg"),
                    name: format!("World {i}"),
                    detail: "file, 3 MB".into(),
                })
                .collect(),
            current: None,
            world: Some(LoadedWorld {
                spec: "/data/w1.ocwpkg".into(),
                name: "World 1".into(),
                routes: vec![("Valley".into(), 12_400.0)],
                has_graph: true,
            }),
            user_routes: vec![UserRoute {
                path: "/home/r/My tour.ocw-graph-route".into(),
                name: "My tour".into(),
                length_m: 5_000.0,
            }],
            rides: vec![RideEntry {
                file: "2026-11-11_17-30-05.fit".into(),
                date: "2026-11-11 17:30".into(),
                route: "Hills".into(),
                strava: "uploaded",
            }],
            gpx: vec!["/home/u/Downloads/morning.gpx".into()],
            here: Some((120.4, -88.0)),
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
    fn worlds_are_paged_and_open_their_page() {
        let c = catalog();
        let rows_of = |p: Page| rows(p, &c);
        // Top → Ride → World packages.
        let mut m = open(&c, &[1, 0]);
        assert_eq!(m.page(), Some(MenuPage::Sub(Page::Worlds)));
        let l = m.labels();
        assert!(l.len() <= MAX_ITEMS, "{l:?}");
        assert_eq!(l[0], "World 0:  file, 3 MB");
        assert_eq!(l[1], "World 1:  file, 3 MB - riding");
        assert_eq!(&l[4..], ["Next (1/3)", "Back"]);
        // Next, then Next again: the focus stays on Next.
        m.apply_with(Action::Click(4), rows_of);
        assert_eq!(m.labels()[0], "World 4:  file, 3 MB");
        assert_eq!(m.labels()[m.focus()], "Next (2/3)");
        m.apply_with(Action::Confirm, rows_of);
        let l = m.labels();
        assert_eq!(l, ["World 8:  file, 3 MB", "Previous (3/3)", "Back"]);
        assert_eq!(l[m.focus()], "Previous (3/3)");
        // Left / right on a row without a value turn the pages too.
        m.apply_with(Action::Hover(0), rows_of);
        m.apply_with(Action::Left, rows_of);
        assert_eq!(m.labels()[0], "World 4:  file, 3 MB");
        m.apply_with(Action::Left, rows_of);
        m.apply_with(Action::Left, rows_of);
        assert_eq!(m.labels()[0], "World 0:  file, 3 MB", "stops at the first");
        // A world opens its page; Back returns to the list.
        m.apply_with(Action::Click(1), rows_of);
        assert_eq!(m.page(), Some(MenuPage::Sub(Page::World(1))));
        assert_eq!(
            m.labels(),
            [
                "Free ride:  random start",
                "Free ride from here:  120, -88",
                "Routes:  2",
                "Package:  file, 3 MB",
                "Back"
            ]
        );
        // Free ride asks first, then runs the switch and closes the menu.
        assert_eq!(m.apply_with(Action::Click(1), rows_of), Effect::None);
        assert_eq!(m.page(), Some(MenuPage::ConfirmSwitch));
        assert_eq!(m.labels()[m.focus()], "Cancel");
        assert_eq!(
            m.pending(),
            Some("ride.switch --world \"/data/w1.ocwpkg\" --free-ride --start 120,-88")
        );
        // Cancel: back, nothing run.
        assert_eq!(m.apply_with(Action::Confirm, rows_of), Effect::None);
        assert_eq!(m.page(), Some(MenuPage::Sub(Page::World(1))));
        assert_eq!(m.pending(), None);
        m.apply_with(Action::Click(0), rows_of);
        assert_eq!(
            m.apply_with(Action::Click(0), rows_of),
            Effect::Command("ride.switch --world \"/data/w1.ocwpkg\" --free-ride".into())
        );
        assert!(!m.is_open());
        // Another world: no position, no routes yet.
        let m = open(&c, &[1, 0, 2]);
        assert_eq!(m.page(), Some(MenuPage::Sub(Page::World(2))));
        assert_eq!(m.labels().len(), 4);
        assert!(m.labels()[1].starts_with("Routes:  listed once"));
    }

    #[test]
    fn routes_rides_and_gpx_rows_run_their_commands() {
        let c = catalog();
        let rows_of = |p: Page| rows(p, &c);
        // Top → Navi → Routes.
        let mut m = open(&c, &[3, 0]);
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
        let mut m = open(&c, &[3, 1]);
        assert_eq!(m.labels(), ["2026-11-11 17:30:  Hills - uploaded", "Back"]);
        assert_eq!(m.apply_with(Action::Click(0), rows_of), Effect::None);
        // GPX import: import and ride, after asking.
        let mut m = open(&c, &[3, 2]);
        assert_eq!(m.labels()[0], "morning.gpx:  import, ride in World 1");
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
        // Strava: the placeholder page.
        let m = open(&c, &[3, 3]);
        assert_eq!(m.page(), Some(MenuPage::NaviStrava));
    }

    #[test]
    fn without_a_world_the_lists_say_what_to_do() {
        let c = Catalog::default();
        for p in [Page::Worlds, Page::Routes, Page::Rides, Page::GpxImport] {
            let r = rows(p, &c);
            assert_eq!(r.len(), 1, "{p:?}");
            assert_eq!(r[0].confirm, None, "{p:?}");
        }
        // A World page of a world no longer listed is empty (the menu goes back).
        assert!(rows(Page::World(3), &c).is_empty());
        let mut m = open(&catalog(), &[1, 0, 4, 5, 0]);
        assert_eq!(m.page(), Some(MenuPage::Sub(Page::World(8))));
        m.apply_with(Action::Hover(0), |p| rows(p, &Catalog::default()));
        assert_eq!(m.page(), Some(MenuPage::Sub(Page::Worlds)));
    }

    #[test]
    fn rows_fit_the_ten_foot_limits() {
        let mut c = catalog();
        c.worlds[0].name = "A very long world package name that goes on".into();
        for p in [
            Page::Worlds,
            Page::World(0),
            Page::World(1),
            Page::Routes,
            Page::Rides,
            Page::GpxImport,
        ] {
            for row in rows(p, &c) {
                let l = row_label(&row);
                assert!(l.chars().count() <= 64, "{p:?}: {l}");
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
