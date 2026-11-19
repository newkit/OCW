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

/// The game time now (s).
pub fn game_now(world: &World) -> f64 {
    world
        .get_resource::<Time>()
        .map_or(0.0, |t| t.elapsed_secs_f64())
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
    /// Ends the ride and saves it; `Ok(None)` if there was nothing to save
    /// (the app saves with [`save_now`], which adds the training history).
    #[cfg(test)]
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
            // The simple Strava upload (spec §2.8.1 *Simple upload*).
            upload: Some(crate::strava::manual::upload),
            marked: Some(crate::strava::manual::marked),
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
    strava: Option<ResMut<crate::strava::recording::StravaRec>>,
) {
    // The Strava recording (spec §2.8.1): the same samples, its own FIT.
    if let Some(mut s) = strava
        && s.0.is_recording()
        && client.0.rider_id().is_some()
    {
        let t = time.elapsed_secs_f64();
        let c = &client.0;
        s.0.observe(c.track(), c.geo_origin(), sample(c, t), unix_now());
        if let Some(crash) = c.own_crash(t) {
            s.0.mark_crash(t - crash.elapsed_s);
        }
    }
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
        route: client
            .map(|c| c.course_name().to_string())
            .unwrap_or_default(),
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
    // The ride ends: a Strava recording in it ends and is saved too (spec
    // §2.8.1), the photographer starts counting again (spec §2.11).
    crate::strava::recording::stop(world);
    if let Some(mut p) = world.get_resource_mut::<ocw_ui::photographer::RidePhotographer>() {
        p.new_ride();
    }
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
    let Some(dir) = world
        .get_resource::<Recording>()
        .and_then(|r| r.dir.clone())
    else {
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

/// The Strava recordings (start, end: UTC Unix s) in `rides/strava/`.
pub fn strava_recordings(rides: &Path) -> Vec<(i64, i64)> {
    std::fs::read_dir(crate::strava::recording::strava_dir(rides))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "fit"))
        .filter_map(|p| {
            let a = ocw_fit::decode(&std::fs::read(p).ok()?).ok()?;
            let end = a.start_unix_s + a.records.last()?.elapsed_s.ceil() as i64;
            Some((a.start_unix_s, end))
        })
        .collect()
}

/// The photos of every ride in the history (spec §2.11) and those taken
/// in its Strava recordings (§2.8.1 ST3).
fn rides_photos(
    world: &World,
) -> std::collections::BTreeMap<String, ocw_client::menu::history::RidePhotos> {
    let out = std::collections::BTreeMap::new();
    let Some(dir) = world
        .get_resource::<Recording>()
        .and_then(|r| r.dir.clone())
    else {
        return out;
    };
    let Some(h) = world.get_resource::<ocw_ui::history::TrainingHistory>() else {
        return out;
    };
    let rides: Vec<(String, i64, f64)> = h
        .history
        .rides
        .iter()
        .map(|r| (r.id.clone(), r.start_unix_s, r.elapsed_s))
        .collect();
    photos_of_rides(&dir, &rides)
}

/// The photos of the rides (id, start, elapsed s) kept in `dir` – in the
/// photos folder or, for rides before v4.73, beside their sidecars – and
/// how many of them lie in a Strava recording (§2.8.1 ST3).
pub fn photos_of_rides(
    dir: &Path,
    rides: &[(String, i64, f64)],
) -> std::collections::BTreeMap<String, ocw_client::menu::history::RidePhotos> {
    let mut out = std::collections::BTreeMap::new();
    let recordings = strava_recordings(dir);
    for (id, start, elapsed) in rides {
        let r = HistoryRide {
            id,
            start_unix_s: *start,
            elapsed_s: *elapsed,
        };
        let files = crate::photos::ride_photos(dir, r.id);
        if files.is_empty() {
            continue;
        }
        let end = r.start_unix_s + r.elapsed_s.ceil() as i64;
        let within: Vec<&(i64, i64)> = recordings
            .iter()
            .filter(|(a, b)| *a <= end && *b >= r.start_unix_s)
            .collect();
        let in_strava = files
            .iter()
            .filter(|(_, _, t)| within.iter().any(|(a, b)| (*a..=*b).contains(t)))
            .count();
        out.insert(
            r.id.clone(),
            ocw_client::menu::history::RidePhotos {
                files: files.into_iter().map(|(f, s, _)| (f, s)).collect(),
                in_strava,
                recordings: within.len(),
            },
        );
    }
    out
}

/// How many Strava recordings lie within each ride of the history.
fn strava_recordings_by_ride(world: &World) -> std::collections::BTreeMap<String, usize> {
    let mut out = std::collections::BTreeMap::new();
    let Some(dir) = world
        .get_resource::<Recording>()
        .and_then(|r| r.dir.clone())
    else {
        return out;
    };
    let recordings = strava_recordings(&dir);
    if recordings.is_empty() {
        return out;
    }
    if let Some(h) = world.get_resource::<ocw_ui::history::TrainingHistory>() {
        for r in &h.history.rides {
            let end = r.start_unix_s + r.elapsed_s.ceil() as i64;
            let n = recordings
                .iter()
                .filter(|(a, _)| *a + 1 >= r.start_unix_s && *a <= end)
                .count();
            if n > 0 {
                out.insert(r.id.clone(), n);
            }
        }
    }
    out
}

struct HistoryRide<'a> {
    id: &'a String,
    start_unix_s: i64,
    elapsed_s: f64,
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
    let photos = rides_photos(world);
    // The Strava upload's way (API or browser) and each ride's Strava
    // recordings (spec §2.8.1 *Simple upload*).
    let strava_api = world
        .get_resource::<crate::strava::Strava>()
        .is_some_and(|s| s.api());
    // The automatic upload of the Strava recordings (spec 2.8.1 Q6).
    let strava_auto_off = world
        .get_resource::<crate::strava::Strava>()
        .is_some_and(|s| !s.auto());
    let recordings = strava_recordings_by_ride(world);
    // Each ride's Strava state and the open question (spec §2.8.1
    // *Recording without Strava set up, uploaded later*).
    let (strava, question) = crate::strava::history_states(world);
    if let Some(mut h) = world.get_resource_mut::<ocw_ui::history::TrainingHistory>() {
        h.photos = photos;
        h.strava_api = strava_api;
        h.strava_auto_off = strava_auto_off;
        h.recordings = recordings;
        h.strava = strava;
        h.strava_question = question;
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
    let mut out = format!(
        "{added} rides imported into {}'s training history",
        riding_name(world)
    );
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
    let name = ride
        .fit
        .as_ref()
        .ok_or("this ride's FIT file is not kept")?;
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
    strava: Option<ResMut<crate::strava::recording::StravaRec>>,
    mut last: Local<LapKey>,
) {
    let now = (
        workouts.map_or(0, |w| w.0.lap_count()),
        client.0.event_lap_segment(),
    );
    if starts_lap(*last, now) {
        let t = time.elapsed_secs_f64();
        rec.recorder.mark_lap(t);
        if let Some(mut s) = strava {
            s.0.mark_lap(t);
        }
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
            .about("End the ride, save it as a FIT file for the training history (a running Strava recording stops and is saved; whole rides never go to Strava); a new recording starts"),
        |world, _| {
            let saved = save_now(world, 0.0)?;
            match saved {
                Some(path) => {
                    info!(path = %path.display(), "ride saved");
                    // Whole rides never go to Strava (spec 2.8.1 Q6): the
                    // Strava recording's stop has said where it goes.
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
        Command::new("history.photos")
            .about("Open the folder with the ride photographer's photos of ride N (1 = the newest) – to add them in Strava by hand")
            .arg(clap::Arg::new("n").required(true)),
        |world, m| {
            let n: usize = m
                .get_one::<String>("n")
                .expect("required")
                .parse()
                .map_err(|_| "expected a ride number".to_string())?;
            let id = world
                .get_resource::<ocw_ui::history::TrainingHistory>()
                .and_then(|h| h.history.nth_newest(n.saturating_sub(1)).map(|r| r.id.clone()))
                .ok_or("no such ride")?;
            let dir = world
                .get_resource::<Recording>()
                .and_then(|r| r.dir.clone())
                .ok_or("no rides folder")?;
            // The activities folder (or an older Pictures/OCW), or the ride's own folder
            // for rides before v4.73 (spec §2.11 Q3).
            let folder = crate::photos::ride_photo_folder(&dir, &id)
                .ok_or("this ride has no photos")?;
            crate::strava::auth::open_browser(&folder.to_string_lossy())?;
            Ok(format!("opened {}", folder.display()))
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

    /// The Strava recordings find the ride's photos (spec §2.8.1 ST3,
    /// §2.11 Q3): those in the photos folder through the ride's sidecars
    /// and an older ride's beside them; only the ones taken within a
    /// Strava recording count for it.
    #[test]
    fn strava_recordings_find_the_photos_in_the_photos_folder() {
        use ocw_render::media::MediaTime;
        let dir = temp_dir("photos");
        let route = Route::test_loop();
        // A Strava recording from 0 to 2 s (Unix time; test samples).
        let mut r = RideRecorder::new("Loop");
        for i in 0..3 {
            r.observe(
                &route,
                route.geo_origin(),
                sample(f64::from(i), f64::from(i) * 8.0),
                0,
            );
        }
        let strava = crate::strava::recording::strava_dir(&dir);
        save_ride(&strava, &r.finish().unwrap()).unwrap();
        // (FIT counts from 1989-12-31: the test samples start there.)
        let recs = strava_recordings(&dir);
        assert_eq!(recs.len(), 1);
        let (t0, t1) = recs[0];
        assert_eq!(t1 - t0, 2);
        let at = |s: i64| MediaTime {
            unix_s: t0 + s,
            offset_s: 0,
        };
        let rgb = vec![50u8; 16 * 9 * 3];
        // The new ride: one photo in the recording, one after it, both in
        // the photos folder.
        let side = crate::photos::photos_dir(&dir, "new");
        for (n, t) in [(1usize, 1i64), (2, 60)] {
            let p = dir.join(format!("Pictures/OCW/OCW-new-photo-{n}.jpg"));
            let m = crate::photos::tests::meta(
                n,
                format!("OCW-new-photo-{n}.jpg"),
                Some(p.to_string_lossy().into_owned()),
                &at(t),
            );
            crate::photos::write_photo(&p, Some(&side), &rgb, 16, 9, &m, &at(t), "OCW").unwrap();
        }
        // An older ride: its photo beside the sidecar.
        let old = crate::photos::photos_dir(&dir, "old");
        let m = crate::photos::tests::meta(1, "photo-1.jpg".into(), None, &at(2));
        crate::photos::write_photo(
            &old.join("photo-1.jpg"),
            Some(&old),
            &rgb,
            16,
            9,
            &m,
            &at(2),
            "OCW",
        )
        .unwrap();
        let rides = [
            ("new".to_string(), t0, 100.0),
            ("old".to_string(), t0, 10.0),
            ("none".to_string(), t0, 10.0),
        ];
        let photos = photos_of_rides(&dir, &rides);
        let new = &photos["new"];
        assert_eq!(
            new.files,
            vec![
                ("OCW-new-photo-1.jpg".to_string(), 1501.0),
                ("OCW-new-photo-2.jpg".to_string(), 1502.0)
            ]
        );
        assert_eq!((new.in_strava, new.recordings), (1, 1));
        assert_eq!(new.summary(), "2 photos, 1 in 1 Strava recording");
        assert_eq!(
            photos["old"].files,
            vec![("photo-1.jpg".to_string(), 1501.0)]
        );
        assert_eq!(photos["old"].in_strava, 1);
        assert!(!photos.contains_key("none"));
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
