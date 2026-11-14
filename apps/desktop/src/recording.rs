//! Ride recording and FIT export (spec §2.8): the local rider's state is
//! sampled once per second; when the ride ends (app exit or `ride.save`) it
//! is written as a FIT activity to `<data dir>/ocw/rides/`, with the virtual
//! GPS track when the route lies on real roads.
//!
//! [`RideRecorder`] is engine-independent and unit-tested; the plugin below
//! only feeds it from [`ClientState`] and saves on exit.
//!
//! Each saved ride also goes into the rider's **training history** (spec
//! §14 Phase 7, `ocw_ui::history`): `<data dir>/ocw/history/<rider>.json`,
//! with the world, the hearts it earned, the damage at the end and the
//! workout ([`save_now`]); `history.import` adds older FIT files of the
//! rides folder, `history.export N` copies a ride's FIT file to the
//! downloads folder ([`HistoryHooks`]).

use std::path::{Path, PathBuf};

use bevy::prelude::*;
use clap::Command;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, Permission};

use ocw_client::history::{EndDamage, RideContext, RideEntry};
use ocw_ui::history::HistoryHooks;
use ocw_ui::settings::FileStore;

use crate::Args;
use crate::connection::ClientState;

pub use ocw_client::recording::*;

/// `<data dir>/ocw/rides`; `OCW_DATA_DIR` overrides `<data dir>/ocw`.
pub fn default_rides_dir() -> Option<PathBuf> {
    let dir = match std::env::var_os("OCW_DATA_DIR") {
        Some(d) => PathBuf::from(d),
        None => dirs::data_dir()?.join("ocw"),
    };
    Some(dir.join("rides"))
}

/// Writes `<dir>/<stem>.fit` and its `.toml` sidecar; returns the FIT path.
/// The FIT file is written last, through a temporary name, so the Strava
/// sync never sees a half-written ride.
pub fn save_ride(dir: &Path, ride: &FinishedRide) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut stem = file_stem(ride.activity.start_unix_s);
    let mut n = 1;
    while dir.join(format!("{stem}.fit")).exists() {
        n += 1;
        stem = format!("{}-{n}", file_stem(ride.activity.start_unix_s));
    }
    let meta = toml::to_string_pretty(&ride.meta).map_err(|e| e.to_string())?;
    let meta_path = dir.join(format!("{stem}.toml"));
    std::fs::write(&meta_path, meta).map_err(|e| format!("{}: {e}", meta_path.display()))?;
    let path = dir.join(format!("{stem}.fit"));
    let tmp = dir.join(format!("{stem}.fit.part"));
    std::fs::write(&tmp, ocw_fit::encode(&ride.activity))
        .map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// Reads a ride's sidecar, if there is one.
pub fn load_meta(fit_path: &Path) -> Option<RideMeta> {
    let text = std::fs::read_to_string(fit_path.with_extension("toml")).ok()?;
    toml::from_str(&text).ok()
}

pub(crate) fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

// ---------------------------------------------------------------------------
// Bevy glue

/// Records the local rider; `dir` overrides [`default_rides_dir`].
#[derive(Default)]
pub struct RecordingPlugin {
    pub dir: Option<PathBuf>,
}

#[derive(Resource)]
pub struct Recording {
    pub recorder: RideRecorder,
    pub dir: Option<PathBuf>,
    pub enabled: bool,
    /// The server's hearts count at the ride's first record (the hearts
    /// the ride earned are the difference at its end).
    pub hearts_start: Option<u32>,
    /// The last workout ridden in this ride.
    pub workout: Option<String>,
}

impl Recording {
    /// Ends the ride and saves it; `Ok(None)` if there was nothing to save.
    pub fn save(&mut self, min_duration_s: f64) -> Result<Option<PathBuf>, String> {
        if self.recorder.duration_s() < min_duration_s {
            return Ok(None);
        }
        let Some(ride) = self.recorder.finish() else {
            return Ok(None);
        };
        let dir = self.dir.as_deref().ok_or("no data directory for rides")?;
        save_ride(dir, &ride).map(Some)
    }
}

impl Plugin for RecordingPlugin {
    fn build(&self, app: &mut App) {
        let args = app.world().resource::<Args>().clone();
        // Scripted screenshot / capture runs do not fill the ride history.
        let enabled = args
            .record
            .unwrap_or(args.screenshot.is_none() && args.capture.is_none());
        let route = app
            .world()
            .resource::<ClientState>()
            .0
            .course_name()
            .to_string();
        app.insert_resource(Recording {
            recorder: RideRecorder::new(route),
            dir: self.dir.clone().or_else(default_rides_dir),
            enabled,
            hearts_start: None,
            workout: None,
        })
        .insert_resource(HistoryHooks {
            store: history_store,
            rider: riding_name,
            export: export_fit,
            import: Some(import_rides),
            refresh: refresh_history,
        })
        .add_systems(Update, (mark_laps, record).chain())
        .add_systems(Last, save_on_exit);
        register_commands(app);
    }
}

fn record(
    time: Res<Time>,
    client: Res<ClientState>,
    workouts: Option<Res<crate::workout::Workouts>>,
    mut rec: ResMut<Recording>,
) {
    if !rec.enabled {
        return;
    }
    let started = rec.recorder.records().is_empty();
    observe_client(
        &mut rec.recorder,
        &client.0,
        time.elapsed_secs_f64(),
        unix_now(),
    );
    if started && !rec.recorder.records().is_empty() {
        rec.hearts_start = Some(client.0.server_hearts());
        rec.workout = None;
    }
    if let Some((name, _)) = workouts.and_then(|w| w.0.now())
        && rec.workout.as_deref() != Some(name.as_str())
    {
        rec.workout = Some(name);
    }
}

/// The rider riding now (their history).
pub fn riding_name(world: &World) -> String {
    world
        .get_resource::<crate::settings::RiderProfiles>()
        .map_or_else(
            || crate::settings::riders::Rider::default().name,
            |r| r.riding.name.clone(),
        )
}

/// `<data dir>/ocw/history` (next to the rides folder).
fn history_dir(world: &World) -> Option<PathBuf> {
    let rides = world.get_resource::<Recording>()?.dir.clone()?;
    Some(rides.parent()?.join("history"))
}

fn history_store(world: &World, rider: &str) -> Option<Box<dyn ocw_ui::settings::TextStore>> {
    let dir = history_dir(world)?;
    Some(Box::new(FileStore(
        dir.join(ocw_client::history::file_name(rider)),
    )))
}

/// What the app knows of the ride ending now beyond its records.
fn ride_context(world: &World, hearts_start: Option<u32>, workout: Option<String>) -> RideContext {
    let client = world.get_resource::<ClientState>().map(|c| &c.0);
    let args = world.get_resource::<Args>();
    let earns = args.is_some_and(|a| {
        crate::workshop::wallet::earns(a.server.is_none(), a.capture.is_some(), a.no_hearts)
    });
    RideContext {
        world: world
            .get_resource::<crate::ride::RideCatalog>()
            .map(|c| crate::ride::world::current_name(&c.0))
            .filter(|n| n != "-")
            .unwrap_or_default(),
        route: client.map(|c| c.course_name().to_string()).unwrap_or_default(),
        hearts: client.map(|c| {
            if earns {
                c.server_hearts().saturating_sub(hearts_start.unwrap_or(0))
            } else {
                0
            }
        }),
        damage: client
            .filter(|c| c.damage_on())
            .and_then(|c| c.own_damage())
            .map(|d| EndDamage {
                rider: d.rider,
                bike: d.bike(),
            }),
        workout,
        ..RideContext::default()
    }
}

/// Ends the ride and saves it (when it is at least `min_duration_s`
/// long) as a FIT file and into the rider's training history.
pub fn save_now(world: &mut World, min_duration_s: f64) -> Result<Option<PathBuf>, String> {
    let Some(rec) = world.get_resource::<Recording>() else {
        return Ok(None);
    };
    if rec.recorder.duration_s() < min_duration_s {
        return Ok(None);
    }
    let ctx = ride_context(world, rec.hearts_start, rec.workout.clone());
    let mut rec = world.resource_mut::<Recording>();
    let Some(ride) = rec.recorder.finish() else {
        return Ok(None);
    };
    rec.hearts_start = None;
    rec.workout = None;
    let dir = rec.dir.clone().ok_or("no data directory for rides")?;
    let path = save_ride(&dir, &ride)?;
    let id = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let entry = RideEntry::from_activity(
        &ride.activity,
        RideContext {
            id,
            fit: path.file_name().map(|s| s.to_string_lossy().into_owned()),
            ..ctx
        },
    );
    if let Err(e) = ocw_ui::history::add_ride(world, entry, None) {
        warn!("training history: {e}");
    }
    Ok(Some(path))
}

/// The ids of every rider's history (an imported FIT file is in none).
fn ids_in_histories(world: &World) -> std::collections::BTreeSet<String> {
    let mut ids: std::collections::BTreeSet<String> = world
        .get_resource::<ocw_ui::history::TrainingHistory>()
        .map(|h| h.history.rides.iter().map(|r| r.id.clone()).collect())
        .unwrap_or_default();
    if let Some(dir) = history_dir(world)
        && let Ok(files) = std::fs::read_dir(dir)
    {
        for f in files.flatten() {
            if let Ok(text) = std::fs::read_to_string(f.path())
                && let Ok(h) = ocw_client::history::History::parse(&text)
            {
                ids.extend(h.rides.into_iter().map(|r| r.id));
            }
        }
    }
    ids
}

/// The FIT files of the rides folder (stem, path), oldest first.
fn fit_files(world: &World) -> Vec<(String, PathBuf)> {
    let Some(dir) = world.get_resource::<Recording>().and_then(|r| r.dir.clone()) else {
        return Vec::new();
    };
    let mut out: Vec<(String, PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "fit"))
        .filter_map(|p| Some((p.file_stem()?.to_string_lossy().into_owned(), p)))
        .collect();
    out.sort();
    out
}

/// What can be imported and exported now.
fn refresh_history(world: &mut World) {
    let files = fit_files(world);
    let ids = ids_in_histories(world);
    let importable = files.iter().filter(|(s, _)| !ids.contains(s)).count();
    let names: std::collections::BTreeSet<String> = files
        .iter()
        .filter_map(|(_, p)| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();
    if let Some(mut h) = world.get_resource_mut::<ocw_ui::history::TrainingHistory>() {
        h.importable = Some(importable);
        h.exportable = h
            .history
            .rides
            .iter()
            .filter(|r| r.fit.as_ref().is_some_and(|f| names.contains(f)))
            .map(|r| r.id.clone())
            .collect();
    }
}

/// `history.import`: the rides folder's FIT files in no history yet go
/// into the riding rider's (marked imported: no hearts, damage or world).
fn import_rides(world: &mut World) -> Result<String, String> {
    let ids = ids_in_histories(world);
    let mut added = 0;
    let mut failed = Vec::new();
    for (stem, path) in fit_files(world) {
        if ids.contains(&stem) {
            continue;
        }
        let read = std::fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|b| ocw_fit::decode(&b).map_err(|e| e.to_string()));
        match read {
            Ok(a) => {
                let route = load_meta(&path).map(|m| m.route).unwrap_or_default();
                let entry = RideEntry::from_activity(
                    &a,
                    RideContext {
                        id: stem.clone(),
                        route,
                        fit: path.file_name().map(|s| s.to_string_lossy().into_owned()),
                        imported: true,
                        ..RideContext::default()
                    },
                );
                if let Some(mut h) = world.get_resource_mut::<ocw_ui::history::TrainingHistory>() {
                    h.history.add(entry);
                }
                added += 1;
            }
            Err(e) => failed.push(format!("{stem}: {e}")),
        }
    }
    let saved = world
        .get_resource_mut::<ocw_ui::history::TrainingHistory>()
        .map(|mut h| h.save());
    refresh_history(world);
    let mut out = format!("{added} rides imported into {}'s training history", riding_name(world));
    if let Some(Err(e)) = saved {
        out.push_str(&format!(" (not saved: {e})"));
    }
    if !failed.is_empty() {
        out.push_str(&format!("; not readable: {}", failed.join(", ")));
    }
    Ok(out)
}

/// `history.export N [--to DIR]`: copies the ride's FIT file (to the
/// downloads folder by default) as `ocw-ride-<start>.fit`.
fn export_fit(world: &mut World, ride: &RideEntry, to: Option<&str>) -> Result<String, String> {
    let dir = world
        .get_resource::<Recording>()
        .and_then(|r| r.dir.clone())
        .ok_or("no rides folder")?;
    let name = ride.fit.as_ref().ok_or("this ride's FIT file is not kept")?;
    let from = dir.join(name);
    let target = match to {
        Some(t) => PathBuf::from(t),
        None => dirs::download_dir()
            .or_else(|| dir.parent().map(Path::to_path_buf))
            .ok_or("no downloads folder (use --to DIR)")?,
    };
    std::fs::create_dir_all(&target).map_err(|e| format!("{}: {e}", target.display()))?;
    let out = target.join(download_name(ride.start_unix_s));
    std::fs::copy(&from, &out).map_err(|e| format!("{}: {e}", from.display()))?;
    Ok(format!("exported to {}", out.display()))
}

/// Marks FIT laps at workout steps and event starts, laps and finishes.
fn mark_laps(
    time: Res<Time>,
    client: Res<ClientState>,
    workouts: Option<Res<crate::workout::Workouts>>,
    mut rec: ResMut<Recording>,
    mut last: Local<LapKey>,
) {
    let now = (
        workouts.map_or(0, |w| w.0.lap_count()),
        client.0.event_lap_segment(),
    );
    if starts_lap(*last, now) {
        let t = time.elapsed_secs_f64();
        rec.recorder.mark_lap(t);
    }
    *last = now;
}

fn save_on_exit(mut exit: MessageReader<AppExit>, rec: Res<Recording>, mut commands: Commands) {
    if exit.read().next().is_none() || !rec.enabled {
        return;
    }
    commands.queue(|world: &mut World| match save_now(world, MIN_AUTO_SAVE_S) {
        Ok(Some(path)) => {
            info!(path = %path.display(), "ride saved");
            // Uploaded on the next start if the app exits first.
            if let Some(s) = world.get_resource::<crate::strava::Strava>() {
                s.ride_saved();
            }
        }
        Ok(None) => info!("ride too short, not saved"),
        Err(e) => error!("cannot save ride: {e}"),
    });
}

fn register_commands(app: &mut App) {
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("ride.save")
            .about("End the ride, save it as a FIT file (and upload it to Strava if connected); a new recording starts"),
        |world, _| {
            let saved = save_now(world, 0.0)?;
            match saved {
                Some(path) => {
                    info!(path = %path.display(), "ride saved");
                    if let Some(s) = world.get_resource::<crate::strava::Strava>() {
                        s.ride_saved();
                    }
                    Ok(format!("ride saved to {}", path.display()))
                }
                None => Err("nothing recorded yet".into()),
            }
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("ride.status").about("Show the ride recording"),
        |world, _| {
            let rec = world.resource::<Recording>();
            if !rec.enabled {
                return Ok("recording off (--record true turns it on)".into());
            }
            let r = &rec.recorder;
            Ok(format!(
                "recording: {} records, {:.0} s, {:.2} km{}; rides go to {}",
                r.records().len(),
                r.duration_s(),
                r.distance_m() / 1000.0,
                if r.records().last().is_some_and(|r| r.position.is_some()) {
                    ", with GPS track"
                } else {
                    ""
                },
                rec.dir
                    .as_deref()
                    .map_or("(no data directory)".into(), |d| d.display().to_string())
            ))
        },
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_world::Route;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ocw-rec-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn sample(t: f64, distance: f64) -> Sample {
        Sample {
            game_s: t,
            distance,
            speed: 8.0,
            power_w: 210.0,
            cadence_rpm: Some(90.0),
            heart_rate_bpm: Some(130.0),
            ..Sample::default()
        }
    }

    #[test]
    fn saved_rides_are_valid_fit_files_with_a_sidecar() {
        let dir = temp_dir("save");
        let route = Route::test_loop();
        let mut r = RideRecorder::new("Loop");
        for i in 0..3 {
            r.observe(
                &route,
                route.geo_origin(),
                sample(f64::from(i), f64::from(i) * 8.0),
                0,
            );
        }
        let ride = r.finish().unwrap();
        let a = save_ride(&dir, &ride).unwrap();
        let b = save_ride(&dir, &ride).unwrap();
        assert_eq!(a, dir.join("1970-01-01_00-00-00.fit"));
        assert_eq!(b, dir.join("1970-01-01_00-00-00-2.fit"), "never overwrites");
        let bytes = std::fs::read(&a).unwrap();
        assert_eq!(&bytes[8..12], b".FIT");
        assert_eq!(ocw_fit::crc16(0, &bytes), 0);
        assert_eq!(load_meta(&a), Some(ride.meta));
        assert!(!dir.join("1970-01-01_00-00-00.fit.part").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn short_rides_are_not_auto_saved() {
        let dir = temp_dir("short");
        let route = Route::test_loop();
        let mut rec = Recording {
            recorder: RideRecorder::new("Loop"),
            dir: Some(dir.clone()),
            enabled: true,
            hearts_start: None,
            workout: None,
        };
        for i in 0..10 {
            rec.recorder.observe(
                &route,
                route.geo_origin(),
                sample(f64::from(i), f64::from(i) * 8.0),
                0,
            );
        }
        assert_eq!(rec.save(MIN_AUTO_SAVE_S), Ok(None));
        let path = rec.save(0.0).unwrap().unwrap();
        assert!(path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
