//! The training history in both apps (spec §14 Phase 7): the rider's
//! rides ([`TrainingHistory`], `ocw_client::history`) kept in a
//! [`TextStore`] per rider profile – a file on the desktop
//! (`<data dir>/ocw/history/<rider>.json`), `localStorage` in the browser
//! – the Navi → Training history pages (`ocw_client::menu::history`) and
//! the console commands `history`, `history.show N`, `history.bests`,
//! `history.export N [--to DIR]`, `history.import`, `history.delete N`
//! (the ride's entry and its FIT file; spec 14 Phase 7, 7A), and the simple Strava
//! upload's `strava.upload-manual [N]` (the ride's Strava recordings only,
//! never the whole ride: spec 2.8.1 Q6) and `strava.mark-uploaded
//! N [on|off|toggle]` (spec §2.8.1 *Simple upload*).
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
/// The simple Strava upload of a ride's Strava recordings (S … S; whole
/// rides never go to Strava, spec 2.8.1 Q6): the files at hand, Strava's
/// upload page open; the steps to show.
pub type UploadFn = fn(&mut World, &RideEntry) -> Result<String, String>;
/// A ride was marked as uploaded to Strava or unmarked (the desktop tells
/// its API sync).
pub type MarkFn = fn(&mut World, &RideEntry);
/// Deletes a ride's own files (its FIT file: the desktop's rides folder,
/// the browser's OPFS) before its entry leaves the history; what it did
/// ("FIT file deleted"). An error keeps the ride (spec 14 Phase 7, 7A).
/// The Strava recordings and photos in the activities folder stay.
pub type DeleteFn = fn(&mut World, &RideEntry) -> Result<String, String>;

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
    /// The simple Strava upload (spec §2.8.1; `None`: not in this app).
    pub upload: Option<UploadFn>,
    /// Told when a ride's Strava mark changes.
    pub marked: Option<MarkFn>,
    /// Deletes a ride's files (`None`: only the FIT kept in memory).
    pub delete: Option<DeleteFn>,
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
            upload: None,
            marked: None,
            delete: None,
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
    /// Where the FIT files are kept, if that needs saying (the browser
    /// without its file storage: only while the page is open).
    pub kept_note: Option<String>,
    /// FIT files kept in memory (the browser: this page's rides).
    pub fits: BTreeMap<String, Vec<u8>>,
    /// Saved rides not in the history yet (`None`: no import).
    pub importable: Option<usize>,
    /// The rides whose FIT file can be exported.
    pub exportable: BTreeSet<String>,
    /// The ride photographer's photos by ride id (spec §2.11; the app
    /// fills it when the history is refreshed).
    pub photos: BTreeMap<String, ocw_client::menu::history::RidePhotos>,
    /// The automatic API upload is set up (the desktop's `strava.toml`).
    pub strava_api: bool,
    /// The automatic upload of the Strava recordings is off (setting
    /// `strava.auto_upload`, the desktop).
    pub strava_auto_off: bool,
    /// The Strava recordings within each ride (by ride id).
    pub recordings: BTreeMap<String, usize>,
    /// The last simple upload: the ride's id and its steps.
    pub upload: Option<(String, String)>,
    /// Each ride's Strava state (the desktop's API sync; spec §2.8.1).
    pub strava: BTreeMap<String, ocw_client::menu::history::StravaState>,
    /// The open question: upload this many waiting recordings?
    pub strava_question: Option<usize>,
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
            kept_note: h.kept_note.clone(),
            photos: h.photos.clone(),
            strava_api: h.strava_api,
            strava_auto_off: h.strava_auto_off,
            recordings: h.recordings.clone(),
            upload: h.upload.clone(),
            strava: h.strava.clone(),
            strava_question: h.strava_question,
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

/// Deletes ride `n` (1 = the newest; spec 14 Phase 7, 7A): the app's
/// hook deletes its files first (an error keeps the ride), then the entry
/// leaves the history, which is saved. The ride's Strava recordings and
/// photos stay where they are (the activities folder; a recording waiting
/// for Strava is still uploaded).
pub fn delete_ride(world: &mut World, n: usize) -> Result<String, String> {
    let ride = nth(world, n)?;
    if let Some(why) = world
        .get_resource::<TrainingHistory>()
        .and_then(|h| h.read_only.clone())
    {
        return Err(format!("not deleted: {why}"));
    }
    let hooks = hooks(world);
    let files = match hooks.delete {
        Some(f) => f(world, &ride)?,
        None => String::new(),
    };
    let saved = {
        let mut h = world.resource_mut::<TrainingHistory>();
        h.history.remove(&ride.id);
        h.fits.remove(&ride.id);
        h.exportable.remove(&ride.id);
        if h.upload.as_ref().is_some_and(|(id, _)| *id == ride.id) {
            h.upload = None;
        }
        h.save()
    };
    (hooks.refresh)(world);
    let mut out = format!(
        "ride of {} ({}) deleted from the training history",
        start_label(ride.start_unix_s, offset(ride.start_unix_s)),
        distance_label(ride.distance_m, false)
    );
    if !files.is_empty() {
        out.push_str(&format!("; {files}"));
    }
    if let Err(e) = saved {
        out.push_str(&format!(" (not saved: {e})"));
    }
    Ok(out)
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

/// Ride `n` of the history (1 = the newest).
pub fn nth(world: &World, n: usize) -> Result<RideEntry, String> {
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
        Command::new("strava.upload-manual")
            .about("Upload a ride's Strava recordings (S … S) to Strava in your browser, no API app needed (N: its number in `history`, 1 = the newest): the FIT files at hand, Strava's upload page open. Whole rides never go to Strava")
            .arg(Arg::new("n").default_value("1")),
        |world, m| {
            let ride = nth(world, number(m)?)?;
            let upload = hooks(world)
                .upload
                .ok_or("this app has no Strava upload")?;
            let steps = upload(world, &ride)?;
            if let Some(mut h) = world.get_resource_mut::<TrainingHistory>() {
                h.upload = Some((ride.id.clone(), steps.clone()));
            }
            Ok(steps)
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("strava.mark-uploaded")
            .about("Mark ride N as uploaded to Strava (by hand: the app cannot know), or unmark it; the API sync skips marked rides")
            .arg(Arg::new("n").required(true))
            .arg(
                Arg::new("state")
                    .value_parser(["on", "off", "toggle"])
                    .default_value("toggle"),
            ),
        |world, m| {
            let ride = nth(world, number(m)?)?;
            let on = match m.get_one::<String>("state").map(String::as_str) {
                Some("on") => true,
                Some("off") => false,
                _ => !ride.strava_marked,
            };
            let (entry, saved) = {
                let mut h = world.resource_mut::<TrainingHistory>();
                let r = h.history.ride_mut(&ride.id).ok_or("no such ride")?;
                r.strava_marked = on;
                let entry = r.clone();
                (entry, h.save())
            };
            if let Some(f) = hooks(world).marked {
                f(world, &entry);
            }
            let mut out = format!(
                "ride of {} {}",
                start_label(entry.start_unix_s, offset(entry.start_unix_s)),
                if on {
                    "marked as uploaded to Strava (the API sync skips it)"
                } else {
                    "not marked as uploaded to Strava"
                }
            );
            if let Err(e) = saved {
                out.push_str(&format!(" (not saved: {e})"));
            }
            Ok(out)
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("history.delete")
            .about("Delete ride N from the training history (N: its number in `history`, 1 = the newest) with its FIT file; its Strava recordings and photos stay in the activities folder. The menu asks first")
            .arg(Arg::new("n").required(true)),
        |world, m| delete_ride(world, number(m)?),
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

    fn deleted(world: &mut World, ride: &RideEntry) -> Result<String, String> {
        if ride.id == "stuck" {
            return Err("cannot delete stuck.fit: busy".into());
        }
        world.insert_resource(Told(ride.id.clone(), true));
        Ok("FIT file deleted".into())
    }

    /// Deleting a ride (spec 14 Phase 7, 7A): the app's files first, then
    /// the entry, saved; an error of the files keeps the ride; a
    /// read-only history deletes nothing.
    #[test]
    fn a_ride_is_deleted_with_its_files() {
        let mem = MemoryStore::default();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ConsoleCorePlugin))
            .add_plugins((crate::settings::SettingsPlugin, HistoryPlugin));
        app.update();
        let run = |app: &mut App, line: &str| {
            let n = app.world().resource::<ConsoleState>().log().len();
            app.world_mut().resource_mut::<ConsoleState>().queue(line);
            app.update();
            app.world().resource::<ConsoleState>().log()[n..].join("\n")
        };
        {
            let mut h = TrainingHistory::load("Rider", Some(Box::new(mem.clone())));
            h.history.add(entry("stuck", 1_794_211_200, 200.0));
            h.history.add(entry("old", 1_794_211_200 + 3_600, 210.0));
            h.history.add(entry("new", 1_794_211_200 + 86_400, 260.0));
            h.fits.insert("new".into(), vec![1, 2, 3]);
            h.upload = Some(("new".into(), "steps".into()));
            app.insert_resource(h);
        }
        app.insert_resource(HistoryHooks {
            delete: Some(deleted),
            ..HistoryHooks::default()
        });
        let out = run(&mut app, "history.delete 1");
        assert!(
            out.contains("deleted from the training history; FIT file deleted"),
            "{out}"
        );
        assert_eq!(
            app.world().get_resource::<Told>(),
            Some(&Told("new".into(), true))
        );
        {
            let h = app.world().resource::<TrainingHistory>();
            assert!(!h.history.contains("new") && h.fits.is_empty() && h.upload.is_none());
        }
        let again = TrainingHistory::load("Rider", Some(Box::new(mem.clone())));
        let ids: Vec<&str> = again.history.rides.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["stuck", "old"]);
        // The files cannot go: the ride stays.
        let out = run(&mut app, "history.delete 2");
        assert!(out.contains("busy"), "{out}");
        assert_eq!(
            app.world()
                .resource::<TrainingHistory>()
                .history
                .rides
                .len(),
            2
        );
        assert!(run(&mut app, "history.delete 3").contains("no ride 3"));
        // A history of a newer version is read-only: nothing is deleted.
        app.world_mut().resource_mut::<TrainingHistory>().read_only = Some("version 7".into());
        assert!(run(&mut app, "history.delete 1").contains("not deleted"));
        assert_eq!(
            app.world()
                .resource::<TrainingHistory>()
                .history
                .rides
                .len(),
            2
        );
    }

    fn uploaded(_: &mut World, ride: &RideEntry) -> Result<String, String> {
        Ok(format!("steps for {}", ride.id))
    }

    fn told(world: &mut World, ride: &RideEntry) {
        world.insert_resource(Told(ride.id.clone(), ride.strava_marked));
    }

    #[derive(Resource, Debug, PartialEq)]
    struct Told(String, bool);

    /// The simple Strava upload's commands (spec §2.8.1): the app's upload
    /// runs and its steps show on the ride's page; the mark persists in
    /// the history, the app is told (the desktop's API sync).
    #[test]
    fn the_strava_upload_and_mark_commands() {
        let mem = MemoryStore::default();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ConsoleCorePlugin))
            .add_plugins((crate::settings::SettingsPlugin, HistoryPlugin));
        app.update();
        let run = |app: &mut App, line: &str| {
            let n = app.world().resource::<ConsoleState>().log().len();
            app.world_mut().resource_mut::<ConsoleState>().queue(line);
            app.update();
            app.world().resource::<ConsoleState>().log()[n..].join("\n")
        };
        // No upload in this app (the default hooks).
        assert!(run(&mut app, "strava.upload-manual").contains("no ride 1"));
        {
            let mut h = TrainingHistory::load("Rider", Some(Box::new(mem.clone())));
            h.history.add(entry("old", 1_794_211_200, 200.0));
            h.history.add(entry("new", 1_794_211_200 + 86_400, 260.0));
            app.insert_resource(h);
        }
        assert!(run(&mut app, "strava.upload-manual").contains("no Strava upload"));
        app.insert_resource(HistoryHooks {
            upload: Some(uploaded),
            marked: Some(told),
            ..HistoryHooks::default()
        });
        let out = run(&mut app, "strava.upload-manual");
        assert!(out.contains("steps for new"), "{out}");
        // Whole rides never go to Strava (spec 2.8.1 Q6): no --whole.
        assert!(run(&mut app, "strava.upload-manual 2 --whole").contains("error"));
        assert!(run(&mut app, "strava.upload-manual 2").contains("steps for old"));
        let i = info(app.world());
        assert_eq!(i.upload, Some(("old".into(), "steps for old".into())));
        let page = ocw_client::menu::pages::Page::HistoryRide(1);
        assert!(
            ocw_client::menu::history::text(page, &i)
                .iter()
                .any(|l| l == "steps for old")
        );
        // The mark: on, toggled, persisted, told.
        let out = run(&mut app, "strava.mark-uploaded 2 on");
        assert!(out.contains("marked as uploaded"), "{out}");
        assert_eq!(
            app.world().get_resource::<Told>(),
            Some(&Told("old".into(), true))
        );
        let again = TrainingHistory::load("Rider", Some(Box::new(mem.clone())));
        assert!(
            again
                .history
                .rides
                .iter()
                .find(|r| r.id == "old")
                .unwrap()
                .strava_marked
        );
        assert!(
            !again
                .history
                .rides
                .iter()
                .find(|r| r.id == "new")
                .unwrap()
                .strava_marked
        );
        run(&mut app, "strava.mark-uploaded 2");
        assert_eq!(
            app.world().get_resource::<Told>(),
            Some(&Told("old".into(), false))
        );
        let again = TrainingHistory::load("Rider", Some(Box::new(mem.clone())));
        assert!(!again.history.rides.iter().any(|r| r.strava_marked));
        assert!(run(&mut app, "strava.mark-uploaded 3").contains("no ride 3"));
    }
}
