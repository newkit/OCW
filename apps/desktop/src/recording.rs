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
use ocw_fit::{Activity, Record};
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, Permission};
use serde::{Deserialize, Serialize};

use crate::Args;
use crate::connection::ClientState;

/// Seconds between two records.
pub const RECORD_INTERVAL_S: f64 = 1.0;
/// Rides shorter than this are not saved automatically on exit.
pub const MIN_AUTO_SAVE_S: f64 = 60.0;
/// A distance jump faster than this (m/s) is a teleport, not riding.
const MAX_PLAUSIBLE_SPEED_MPS: f64 = 50.0;

/// What the recorder needs from the client each frame.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Sample {
    /// Game time in seconds.
    pub game_s: f64,
    /// The rider's total distance (may jump on teleports).
    pub distance: f64,
    pub speed: f64,
    pub lateral_offset: f64,
    pub power_w: f64,
    pub cadence_rpm: Option<f64>,
    pub heart_rate_bpm: Option<f64>,
}

/// A finished ride, ready to be saved.
#[derive(Debug, Clone, PartialEq)]
pub struct FinishedRide {
    pub activity: Activity,
    pub meta: RideMeta,
}

/// Sidecar of a saved ride (`<ride>.toml`): what the Strava upload shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RideMeta {
    pub name: String,
    pub description: String,
    pub route: String,
}

pub struct RideRecorder {
    route_name: String,
    start_unix_s: Option<i64>,
    start_game_s: f64,
    /// Game time and raw distance of the last record.
    last: Option<(f64, f64)>,
    /// Distance ridden (teleports excluded).
    distance_m: f64,
    records: Vec<Record>,
    /// Lap marks (elapsed s) for the FIT laps.
    laps: Vec<f64>,
}

impl RideRecorder {
    pub fn new(route_name: impl Into<String>) -> Self {
        Self {
            route_name: route_name.into(),
            start_unix_s: None,
            start_game_s: 0.0,
            last: None,
            distance_m: 0.0,
            records: Vec::new(),
            laps: Vec::new(),
        }
    }

    /// Starts a new FIT lap at game time `game_s` (a workout step, an
    /// event's start or finish, an event lap). Ignored before the first
    /// record: the ride starts a lap anyway.
    pub fn mark_lap(&mut self, game_s: f64) {
        if self.start_unix_s.is_some() {
            self.laps.push(game_s - self.start_game_s);
        }
    }

    #[cfg(test)]
    pub fn laps(&self) -> &[f64] {
        &self.laps
    }

    /// Records `s` if a record is due; `wall_unix_s` is the wall clock (only
    /// the first record's is used: the start time). Returns whether it did.
    /// `track` is what the rider rides along (route or free-ride path),
    /// `geo_origin` the (lat, lon) of its local metres, for the GPS track.
    pub fn observe(
        &mut self,
        track: &dyn ocw_world::Track,
        geo_origin: Option<(f64, f64)>,
        s: Sample,
        wall_unix_s: i64,
    ) -> bool {
        if let Some((t, raw)) = self.last {
            let dt = s.game_s - t;
            if dt < RECORD_INTERVAL_S - 1e-6 {
                return false;
            }
            let delta = s.distance - raw;
            if (0.0..=MAX_PLAUSIBLE_SPEED_MPS * dt + 5.0).contains(&delta) {
                self.distance_m += delta;
            }
        } else {
            self.start_unix_s = Some(wall_unix_s);
            self.start_game_s = s.game_s;
        }
        self.last = Some((s.game_s, s.distance));
        let p = track.sample(s.distance).offset_position(s.lateral_offset);
        self.records.push(Record {
            elapsed_s: s.game_s - self.start_game_s,
            distance_m: self.distance_m,
            speed_mps: s.speed,
            power_w: Some(s.power_w),
            cadence_rpm: s.cadence_rpm,
            heart_rate_bpm: s.heart_rate_bpm,
            altitude_m: Some(p[1]),
            position: geo_origin.map(|(lat, lon)| {
                ocw_world::geo::LocalProjection::new(lat, lon).unproject([p[0], p[2]])
            }),
        });
        true
    }

    pub fn records(&self) -> &[Record] {
        &self.records
    }

    pub fn duration_s(&self) -> f64 {
        self.records.last().map_or(0.0, |r| r.elapsed_s)
    }

    pub fn distance_m(&self) -> f64 {
        self.distance_m
    }

    /// Ends the ride: returns it (if it has at least two records) and starts
    /// a new recording.
    pub fn finish(&mut self) -> Option<FinishedRide> {
        let records = std::mem::take(&mut self.records);
        let laps = std::mem::take(&mut self.laps);
        let start_unix_s = self.start_unix_s.take()?;
        self.last = None;
        self.distance_m = 0.0;
        if records.len() < 2 {
            return None;
        }
        Some(FinishedRide {
            activity: Activity {
                start_unix_s,
                records,
                product_name: "Open Cycling World".into(),
                software_version: software_version(),
                laps,
            },
            meta: RideMeta {
                name: format!("Open Cycling World: {}", self.route_name),
                description: format!(
                    "Virtual ride on {} in Open Cycling World. The GPS track is virtual.",
                    self.route_name
                ),
                route: self.route_name.clone(),
            },
        })
    }
}

/// Crate version as FIT `software_version` (major·100 + minor).
fn software_version() -> u16 {
    let major: u16 = env!("CARGO_PKG_VERSION_MAJOR").parse().unwrap_or(0);
    let minor: u16 = env!("CARGO_PKG_VERSION_MINOR").parse().unwrap_or(0);
    major * 100 + minor
}

/// `<data dir>/ocw/rides`; `OCW_DATA_DIR` overrides `<data dir>/ocw`.
pub fn default_rides_dir() -> Option<PathBuf> {
    let dir = match std::env::var_os("OCW_DATA_DIR") {
        Some(d) => PathBuf::from(d),
        None => dirs::data_dir()?.join("ocw"),
    };
    Some(dir.join("rides"))
}

/// File stem of a ride started at `unix_s` (UTC): `2026-11-11_12-00-00`.
pub fn file_stem(unix_s: i64) -> String {
    let days = unix_s.div_euclid(86_400);
    let secs = unix_s.rem_euclid(86_400);
    // Civil date from days since 1970-01-01 (H. Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}_{:02}-{:02}-{:02}",
        secs / 3600,
        secs % 3600 / 60,
        secs % 60
    )
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
    let c = &client.0;
    if !rec.enabled || c.rider_id().is_none() {
        return;
    }
    let s = c.state();
    let sample = Sample {
        game_s: time.elapsed_secs_f64(),
        distance: s.distance,
        speed: s.speed,
        lateral_offset: s.lateral_offset,
        power_w: c.power_w(),
        cadence_rpm: c.cadence_rpm(),
        heart_rate_bpm: c.heart_rate_bpm(),
    };
    rec.recorder
        .observe(c.track(), c.geo_origin(), sample, unix_now());
}

/// What starts a FIT lap: the workout's lap count, the event segment.
pub type LapKey = (u64, Option<(u32, u32)>);

/// Whether going from `before` to `now` starts a new FIT lap: a workout
/// start/step/end/stop, or a change of event part (start, lap, finish)
/// within the same event. Joining or leaving an event is no lap of its
/// own: the countdown and the finish are.
pub fn starts_lap(before: LapKey, now: LapKey) -> bool {
    let workout = now.0 != before.0;
    let event = match (before.1, now.1) {
        (Some((a, sa)), Some((b, sb))) => a != b || sa != sb,
        _ => false,
    };
    workout || event
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
    fn records_once_per_second() {
        let route = Route::test_loop();
        let mut r = RideRecorder::new("Loop");
        let mut t = 10.0;
        let mut n = 0;
        while t < 20.0 {
            n += usize::from(r.observe(
                &route,
                route.geo_origin(),
                sample(t, (t - 10.0) * 8.0),
                1000,
            ));
            t += 1.0 / 60.0;
        }
        assert_eq!(n, 10);
        assert_eq!(r.records().len(), 10);
        assert_eq!(r.records()[0].elapsed_s, 0.0);
        assert!((r.duration_s() - 9.0).abs() < 0.05);
        assert!((r.distance_m() - 72.0).abs() < 0.5);
        let rec = r.records()[3];
        assert_eq!(rec.power_w, Some(210.0));
        assert_eq!(rec.heart_rate_bpm, Some(130.0));
        // Altitude from the road; no GPS on the procedural test route.
        let alt = route.sample(rec.distance_m).position[1];
        assert!((rec.altitude_m.unwrap() - alt).abs() < 1.0);
        assert_eq!(rec.position, None);
    }

    #[test]
    fn lap_marks_go_into_the_fit_file() {
        let route = Route::test_loop();
        let mut r = RideRecorder::new("Loop");
        r.mark_lap(1.0); // before the first record: ignored
        for i in 0..30 {
            let t = 5.0 + f64::from(i);
            if i == 10 || i == 20 {
                r.mark_lap(t);
            }
            r.observe(&route, None, sample(t, f64::from(i) * 8.0), 1000);
        }
        assert_eq!(r.laps(), [10.0, 20.0]);
        let ride = r.finish().unwrap();
        assert_eq!(ride.activity.laps, [10.0, 20.0]);
        assert_eq!(ride.activity.lap_records().len(), 3);
        assert!(r.laps().is_empty(), "a new recording starts without laps");
    }

    #[test]
    fn workout_steps_and_event_parts_start_laps() {
        let none = (0, None);
        assert!(!starts_lap(none, none));
        assert!(starts_lap(none, (1, None)), "workout start");
        assert!(starts_lap((1, None), (2, None)), "workout step");
        // Joining an event: no lap; its start, laps and finish: laps.
        assert!(!starts_lap(none, (0, Some((7, 0)))));
        assert!(starts_lap((0, Some((7, 0))), (0, Some((7, 1)))));
        assert!(starts_lap((0, Some((7, 1))), (0, Some((7, 2)))));
        assert!(!starts_lap((0, Some((7, 2))), (0, Some((7, 2)))));
        assert!(!starts_lap((0, Some((7, 3))), none));
        // A new event right after another.
        assert!(starts_lap((0, Some((7, 3))), (0, Some((8, 0)))));
    }

    #[test]
    fn teleports_do_not_count_as_distance() {
        let route = Route::test_loop();
        let mut r = RideRecorder::new("Loop");
        r.observe(&route, route.geo_origin(), sample(0.0, 0.0), 0);
        r.observe(&route, route.geo_origin(), sample(1.0, 8.0), 0);
        r.observe(&route, route.geo_origin(), sample(2.0, 1508.0), 0); // teleport 1.5 km
        r.observe(&route, route.geo_origin(), sample(3.0, 1516.0), 0);
        r.observe(&route, route.geo_origin(), sample(4.0, 200.0), 0); // teleport back
        assert_eq!(r.distance_m(), 16.0);
    }

    #[test]
    fn routes_on_real_roads_get_a_gps_track() {
        let route = Route::test_loop().with_geo_origin((47.15, 9.55));
        let mut r = RideRecorder::new("Loop");
        r.observe(&route, route.geo_origin(), sample(0.0, 0.0), 0);
        let p = route.sample(0.0).position;
        let (lat, lon) = r.records()[0].position.unwrap();
        assert_eq!(Some((lat, lon)), route.lat_lon(p[0], p[2]));
        assert!((lat - 47.15).abs() < 0.1 && (lon - 9.55).abs() < 0.1);
    }

    #[test]
    fn finish_returns_the_ride_and_starts_over() {
        let route = Route::test_loop();
        let mut r = RideRecorder::new("Loop");
        assert_eq!(r.finish(), None);
        r.observe(&route, route.geo_origin(), sample(0.0, 0.0), 1_762_862_400);
        assert_eq!(r.finish(), None, "one record is not a ride");
        for i in 0..5 {
            r.observe(
                &route,
                route.geo_origin(),
                sample(f64::from(i), f64::from(i) * 8.0),
                1_762_862_400 + 99,
            );
        }
        let ride = r.finish().unwrap();
        assert_eq!(ride.activity.start_unix_s, 1_762_862_499);
        assert_eq!(ride.activity.records.len(), 5);
        assert_eq!(ride.meta.name, "Open Cycling World: Loop");
        assert!(ride.meta.description.contains("virtual"));
        assert!(r.records().is_empty() && r.distance_m() == 0.0);
    }

    #[test]
    fn file_stems_are_utc_dates() {
        assert_eq!(file_stem(0), "1970-01-01_00-00-00");
        assert_eq!(file_stem(1_762_862_400), "2025-11-11_12-00-00");
        assert_eq!(file_stem(951_827_696), "2000-02-29_12-34-56");
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
