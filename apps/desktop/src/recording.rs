//! Ride recording and FIT export (spec §2.8): the local rider's state is
//! sampled once per second; when the ride ends (app exit or `ride.save`) it
//! is written as a FIT activity to `<data dir>/ocw/rides/`, with the virtual
//! GPS track when the route lies on real roads.
//!
//! [`RideRecorder`] is engine-independent and unit-tested; the plugin below
//! only feeds it from [`ClientState`] and saves on exit.

use std::path::{Path, PathBuf};

use bevy::prelude::*;
use clap::Command;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, Permission};

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
        })
        .add_systems(Update, (mark_laps, record).chain())
        .add_systems(Last, save_on_exit);
        register_commands(app);
    }
}

fn record(time: Res<Time>, client: Res<ClientState>, mut rec: ResMut<Recording>) {
    if !rec.enabled {
        return;
    }
    observe_client(
        &mut rec.recorder,
        &client.0,
        time.elapsed_secs_f64(),
        unix_now(),
    );
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

fn save_on_exit(
    mut exit: MessageReader<AppExit>,
    mut rec: ResMut<Recording>,
    strava: Option<Res<crate::strava::Strava>>,
) {
    if exit.read().next().is_none() || !rec.enabled {
        return;
    }
    match rec.save(MIN_AUTO_SAVE_S) {
        Ok(Some(path)) => {
            info!(path = %path.display(), "ride saved");
            // Uploaded on the next start if the app exits first.
            if let Some(s) = strava {
                s.ride_saved();
            }
        }
        Ok(None) => info!("ride too short, not saved"),
        Err(e) => error!("cannot save ride: {e}"),
    }
}

fn register_commands(app: &mut App) {
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("ride.save")
            .about("End the ride, save it as a FIT file (and upload it to Strava if connected); a new recording starts"),
        |world, _| {
            let saved = world.resource_mut::<Recording>().save(0.0)?;
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
