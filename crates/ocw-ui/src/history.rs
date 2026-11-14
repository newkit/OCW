//! The training history in both apps (spec §14 Phase 7): the rider's
//! rides ([`TrainingHistory`], `ocw_client::history`) kept in a
//! [`TextStore`] per rider profile – a file on the desktop
//! (`<data dir>/ocw/history/<rider>.json`), `localStorage` in the browser
//! – the Navi → Training history pages (`ocw_client::menu::history`) and
//! the console commands `history`, `history.show N`, `history.bests`,
//! `history.export N [--to DIR]` and `history.import`.
//!
//! The app adds each saved ride ([`add_ride`]) and says through
//! [`HistoryHooks`] where a rider's history is kept, who rides, how a
//! ride's FIT file is exported and (the desktop) how older rides are
//! imported.

use std::collections::{BTreeMap, BTreeSet};

use bevy::prelude::*;
use clap::{Arg, Command};
use ocw_client::history::{
    CURVE_S, History, HistoryError, Period, RideEntry, distance_label, duration_label, hms,
    start_label, total_summary,
};
use ocw_client::menu::history::HistoryInfo;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, Permission};

use crate::settings::{TextStore, UserSettings};

/// Exports a ride's FIT file (`to`: a folder, else the app's place).
pub type ExportFn = fn(&mut World, &RideEntry, Option<&str>) -> Result<String, String>;
/// Where a rider's history is kept.
pub type StoreFn = fn(&World, &str) -> Option<Box<dyn TextStore>>;
/// Imports older rides.
pub type ImportFn = fn(&mut World) -> Result<String, String>;

/// What the app provides.
#[derive(Resource, Clone, Copy)]
pub struct HistoryHooks {
    /// Where a rider's history is kept (`None`: nowhere, not saved).
    pub store: StoreFn,
    /// The rider whose rides are recorded now.
    pub rider: fn(&World) -> String,
    /// Exports a ride's FIT file (to `to`, else the app's place: the
    /// downloads folder, a download).
    pub export: ExportFn,
    /// Imports older rides (the desktop's saved FIT files).
    pub import: Option<ImportFn>,
    /// Refreshes what can be imported and exported (when the history
    /// changed or the menu opens).
    pub refresh: fn(&mut World),
}

impl Default for HistoryHooks {
    fn default() -> Self {
        Self {
            store: |_, _| None,
            rider: |_| "Rider".into(),
            export: |world, ride, _| {
                let bytes = world
                    .get_resource::<TrainingHistory>()
                    .and_then(|h| h.fits.get(&ride.id).cloned())
                    .ok_or("this ride's FIT file is not kept")?;
                Ok(format!("{} bytes", bytes.len()))
            },
            import: None,
            refresh: refresh_from_memory,
        }
    }
}

/// The rides whose FIT file is kept in memory can be exported (the
/// browser's rides of this page).
pub fn refresh_from_memory(world: &mut World) {
    if let Some(mut h) = world.get_resource_mut::<TrainingHistory>() {
        h.exportable = h.fits.keys().cloned().collect();
    }
}

/// The rider's history and where it is kept.
#[derive(Resource, Default)]
pub struct TrainingHistory {
    pub history: History,
    pub rider: Option<String>,
    store: Option<Box<dyn TextStore>>,
    /// Why it is not saved (a newer or broken file).
    pub read_only: Option<String>,
    /// FIT files kept in memory (the browser: this page's rides).
    pub fits: BTreeMap<String, Vec<u8>>,
    /// Saved rides not in the history yet (`None`: no import).
    pub importable: Option<usize>,
    /// The rides whose FIT file can be exported.
    pub exportable: BTreeSet<String>,
}

impl TrainingHistory {
    /// Reads `rider`'s history from `store`.
    pub fn load(rider: &str, store: Option<Box<dyn TextStore>>) -> Self {
        let mut h = Self {
            history: History::new(rider),
            rider: Some(rider.to_string()),
            ..Self::default()
        };
        let text = match store.as_ref().map(|s| s.load()) {
            Some(Ok(t)) => t,
            Some(Err(e)) => {
                h.read_only = Some(format!("{e}; not saved"));
                None
            }
            None => None,
        };
        match text.as_deref().map(History::parse) {
            None => {}
            Some(Ok(hist)) => h.history = hist,
            Some(Err(HistoryError::TooNew(v, hist))) => {
                h.read_only = Some(HistoryError::TooNew(v, hist.clone()).to_string());
                h.history = *hist;
            }
            Some(Err(e)) => {
                h.read_only = Some(format!("{e}; kept as it is, not saved"));
            }
        }
        h.history.rider = rider.to_string();
        h.store = store;
        h
    }

    /// Saves the history (not a read-only one).
    pub fn save(&mut self) -> Result<(), String> {
        if let Some(why) = &self.read_only {
            return Err(why.clone());
        }
        match &mut self.store {
            Some(s) => s.save(&self.history.to_json()),
            None => Err("not saved (no data directory or storage)".into()),
        }
    }

    /// Where it is kept.
    pub fn location(&self) -> String {
        self.store
            .as_ref()
            .map_or_else(|| "not saved".into(), |s| s.describe())
    }
}

/// The pages' content now.
pub fn info(world: &World) -> HistoryInfo {
    let now = ocw_render::stamp::wall_unix_s() as i64;
    let imperial = world
        .get_resource::<UserSettings>()
        .is_some_and(|s| s.value("display.units").to_string() == "imperial");
    match world.get_resource::<TrainingHistory>() {
        Some(h) => HistoryInfo {
            history: h.history.clone(),
            now_unix: now,
            offset_s: offset(now),
            imperial,
            importable: h.importable,
            exportable: h.exportable.clone(),
            read_only: h.read_only.clone(),
        },
        None => HistoryInfo {
            now_unix: now,
            offset_s: offset(now),
            imperial,
            ..HistoryInfo::default()
        },
    }
}

/// The local UTC offset (s) at `unix_s`.
pub fn offset(unix_s: i64) -> i32 {
    ocw_render::media::local_offset_s(unix_s)
}

fn hooks(world: &World) -> HistoryHooks {
    world
        .get_resource::<HistoryHooks>()
        .copied()
        .unwrap_or_default()
}

/// Adds a ride and saves the history.
pub fn add_ride(world: &mut World, ride: RideEntry, fit: Option<Vec<u8>>) -> Result<(), String> {
    let id = ride.id.clone();
    let result = {
        let mut h = world.get_resource_or_insert_with(TrainingHistory::default);
        h.history.add(ride);
        if let Some(bytes) = fit {
            h.fits.insert(id, bytes);
        }
        h.save()
    };
    (hooks(world).refresh)(world);
    result
}

pub struct HistoryPlugin;

impl Plugin for HistoryPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TrainingHistory>()
            .add_systems(PreUpdate, follow_rider);
        register_commands(app);
    }
}

/// Loads the history of the rider riding now (at the start and when the
/// rider changes); refreshes the import and export lists when the menu
/// opens.
fn follow_rider(world: &mut World, mut was_open: Local<bool>) {
    let hooks = hooks(world);
    let rider = (hooks.rider)(world);
    let loaded = world
        .get_resource::<TrainingHistory>()
        .and_then(|h| h.rider.clone());
    let open = world
        .get_resource::<ocw_render::menu::MenuView>()
        .is_some_and(|v| v.open);
    if loaded.as_deref() != Some(rider.as_str()) {
        let store = (hooks.store)(world, &rider);
        let fits = world
            .get_resource_mut::<TrainingHistory>()
            .map(|mut h| std::mem::take(&mut h.fits))
            .unwrap_or_default();
        let mut h = TrainingHistory::load(&rider, store);
        h.fits = fits;
        if let Some(why) = &h.read_only {
            warn!("training history of {rider}: {why}");
        }
        world.insert_resource(h);
        (hooks.refresh)(world);
    } else if open && !*was_open {
        (hooks.refresh)(world);
    }
    *was_open = open;
}

fn ride_line(n: usize, r: &RideEntry, offset_s: i32) -> String {
    format!(
        "{n:>3}. {}  {:<20} {:>9} {:>8}{}",
        start_label(r.start_unix_s, offset_s),
        if r.world.is_empty() {
            &r.route
        } else {
            &r.world
        },
        distance_label(r.distance_m, false),
        hms(r.elapsed_s),
        r.avg_power_w
            .map_or(String::new(), |p| format!("  {p:.0} W"))
    )
}

/// The `n`-th newest ride (1-based, as `history` numbers them).
fn nth(world: &World, n: usize) -> Result<RideEntry, String> {
    let h = world
        .get_resource::<TrainingHistory>()
        .ok_or("no training history")?;
    n.checked_sub(1)
        .and_then(|k| h.history.nth_newest(k))
        .cloned()
        .ok_or_else(|| format!("no ride {n} (history lists {})", h.history.rides.len()))
}

fn number(m: &clap::ArgMatches) -> Result<usize, String> {
    m.get_one::<String>("n")
        .expect("required")
        .parse()
        .map_err(|_| "expected the ride's number (history lists them)".into())
}

fn register_commands(app: &mut App) {
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("history").about("The training history: this week, this month and the rides, newest first"),
        |world, _| {
            let i = info(world);
            let h = &i.history;
            let mut out = vec![
                format!("training history of {}: {} rides", h.rider, h.rides.len()),
                format!(
                    "this week: {}",
                    total_summary(&h.current(Period::Week, i.now_unix, i.offset_s), i.imperial)
                ),
                format!(
                    "this month: {}",
                    total_summary(&h.current(Period::Month, i.now_unix, i.offset_s), i.imperial)
                ),
            ];
            for (k, (_, r)) in h.newest_first().enumerate().take(20) {
                out.push(ride_line(k + 1, r, i.offset_s));
            }
            if let Some(why) = &i.read_only {
                out.push(why.clone());
            }
            if let Some(t) = world.get_resource::<TrainingHistory>() {
                out.push(t.location());
            }
            Ok(out.join("\n"))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("history.show")
            .about("A ride of the training history in full (N: its number in `history`)")
            .arg(Arg::new("n").required(true)),
        |world, m| {
            let n = number(m)?;
            nth(world, n)?;
            let i = info(world);
            let page = ocw_client::menu::pages::Page::HistoryRide(n - 1);
            let mut out = vec![ocw_client::menu::history::status(page, &i)];
            out.extend(ocw_client::menu::history::text(page, &i));
            let r = i.history.nth_newest(n - 1).expect("checked");
            if !r.power_curve.is_empty() {
                out.push(format!(
                    "power curve: {}",
                    r.power_curve
                        .iter()
                        .map(|p| format!("{} {:.0} W", duration_label(p.s), p.w))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            for c in &r.climbs {
                out.push(format!(
                    "climb at {}: {} long, {:.0} m at {:.1} %, {}, VAM {:.0} m/h",
                    distance_label(c.start_m, false),
                    distance_label(c.length_m, false),
                    c.gain_m,
                    c.grade() * 100.0,
                    hms(c.time_s),
                    c.vam()
                ));
            }
            Ok(out.join("\n"))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("history.bests").about("Personal bests: the power curve (5 s to 60 min) and the climbs"),
        |world, _| {
            let i = info(world);
            let b = i.history.bests();
            let mut out = Vec::new();
            for s in CURVE_S {
                if let Some(p) = b.power(s) {
                    out.push(format!(
                        "{:>6}: {:.0} W ({})",
                        duration_label(s),
                        p.value,
                        start_label(p.start_unix_s, i.offset_s)
                    ));
                }
            }
            if let Some(c) = &b.biggest_climb {
                out.push(format!(
                    "biggest climb: {:.0} m at {:.1} % ({})",
                    c.value.gain_m,
                    c.value.grade() * 100.0,
                    start_label(c.start_unix_s, i.offset_s)
                ));
            }
            if let Some(c) = &b.fastest_climb {
                out.push(format!(
                    "fastest climbing: VAM {:.0} m/h ({})",
                    c.value.vam(),
                    start_label(c.start_unix_s, i.offset_s)
                ));
            }
            if out.is_empty() {
                out.push("no rides yet".into());
            }
            Ok(out.join("\n"))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("history.export")
            .about("Export a ride's FIT file (N: its number in `history`; desktop: to the downloads folder or --to DIR; browser: a download)")
            .arg(Arg::new("n").required(true))
            .arg(Arg::new("to").long("to")),
        |world, m| {
            let ride = nth(world, number(m)?)?;
            let to = m.get_one::<String>("to").cloned();
            (hooks(world).export)(world, &ride, to.as_deref())
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("history.import")
            .about("Add the saved rides (FIT files) that are not in the training history yet to this rider's"),
        |world, _| match hooks(world).import {
            Some(f) => f(world),
            None => Err("this app has no saved rides to import".into()),
        },
    ));
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::settings::memory::MemoryStore;
    use ocw_client::history::RideContext;
    use ocw_render::console::{ConsoleCorePlugin, ConsoleState};

    pub fn entry(id: &str, start: i64, watts: f64) -> RideEntry {
        let a = ocw_fit::Activity {
            start_unix_s: start,
            records: (0..601)
                .map(|i| ocw_fit::Record {
                    elapsed_s: i as f64,
                    distance_m: 8.0 * i as f64,
                    speed_mps: 8.0,
                    power_w: Some(watts),
                    altitude_m: Some(100.0 + i as f64 * 0.4),
                    ..Default::default()
                })
                .collect(),
            product_name: String::new(),
            software_version: 1,
            laps: Vec::new(),
            markers: Vec::new(),
        };
        RideEntry::from_activity(
            &a,
            RideContext {
                id: id.into(),
                world: "Test".into(),
                route: "Loop".into(),
                ..Default::default()
            },
        )
    }

    #[test]
    fn rides_are_saved_per_rider_and_read_back() {
        let mem = MemoryStore::default();
        let mut h = TrainingHistory::load("Anna", Some(Box::new(mem.clone())));
        assert!(h.history.rides.is_empty() && h.read_only.is_none());
        h.history.add(entry("a", 1_794_211_200, 200.0));
        h.save().unwrap();
        let again = TrainingHistory::load("Anna", Some(Box::new(mem.clone())));
        assert_eq!(again.history, h.history);
        // A newer version is read, never overwritten.
        let newer = mem
            .0
            .lock()
            .unwrap()
            .clone()
            .unwrap()
            .replace("\"version\": 1", "\"version\": 7");
        *mem.0.lock().unwrap() = Some(newer.clone());
        let mut n = TrainingHistory::load("Anna", Some(Box::new(mem.clone())));
        assert_eq!(n.history.rides.len(), 1);
        assert!(n.read_only.as_deref().unwrap().contains("version 7"));
        assert!(n.save().is_err());
        assert_eq!(mem.0.lock().unwrap().as_deref(), Some(newer.as_str()));
    }

    #[test]
    fn the_commands_list_show_and_export() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ConsoleCorePlugin))
            .add_plugins((crate::settings::SettingsPlugin, HistoryPlugin));
        app.update();
        add_ride(app.world_mut(), entry("old", 1_794_211_200, 200.0), None).unwrap_err();
        add_ride(
            app.world_mut(),
            entry("new", 1_794_211_200 + 86_400, 260.0),
            Some(vec![1, 2, 3]),
        )
        .unwrap_err();
        let run = |app: &mut App, line: &str| {
            let n = app.world().resource::<ConsoleState>().log().len();
            app.world_mut().resource_mut::<ConsoleState>().queue(line);
            app.update();
            app.world().resource::<ConsoleState>().log()[n..].join("\n")
        };
        let out = run(&mut app, "history");
        assert!(out.contains("2 rides"), "{out}");
        // Newest first (the day depends on the machine's time zone).
        assert!(out.contains("  1. ") && out.contains("  2. "), "{out}");
        let out = run(&mut app, "history.show 1");
        assert!(out.contains("power curve: 5 s 260 W"), "{out}");
        assert!(out.contains("climb at 0.0 km"), "{out}");
        assert!(run(&mut app, "history.bests").contains("10 min: 260 W"));
        assert!(run(&mut app, "history.export 1").contains("3 bytes"));
        assert!(run(&mut app, "history.export 2").contains("not kept"));
        assert!(run(&mut app, "history.show 3").contains("no ride 3"));
        assert_eq!(
            app.world().resource::<TrainingHistory>().exportable,
            ["new".to_string()].into()
        );
    }
}
