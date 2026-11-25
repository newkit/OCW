//! Ride recording for the FIT export (spec §2.8), shared by the desktop
//! and browser apps (spec §14 Phase 10, B6): the local rider's state is
//! sampled once per second ([`RideRecorder`]); when the ride ends it is a
//! FIT activity ([`FinishedRide`], `ocw_fit::encode`) – a file on the
//! desktop (`<data dir>/ocw/rides/`), a download in the browser – with the
//! virtual GPS track when the route lies on real roads.

use ocw_fit::{Activity, Marker, MarkerKind, Record};
use serde::{Deserialize, Serialize};

use crate::client::ClientCore;

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
    /// In god mode (spec §14 Phase 13E): no distance credited (also not
    /// the step to the landing spot).
    pub flying: bool,
    /// The own ride is paused (spec 14 Phase 10, 10D.2: the trainer is
    /// shared with a browser): nothing is recorded, and the distance
    /// across the pause is not credited.
    pub paused: bool,
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
    /// A Strava recording's own copy in the activities folder (spec
    /// §2.8.1 *Where an S recording is written*): the FIT file's path
    /// there, which the simple upload opens. Not in older sidecars.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
}

pub struct RideRecorder {
    route_name: String,
    start_unix_s: Option<i64>,
    start_game_s: f64,
    /// Game time and raw distance of the last record.
    last: Option<(f64, f64)>,
    /// The last record was in god mode.
    was_flying: bool,
    /// Distance ridden (teleports excluded).
    distance_m: f64,
    records: Vec<Record>,
    /// Lap marks (elapsed s) for the FIT laps.
    laps: Vec<f64>,
    /// God mode's start and end (FIT markers, spec §14 Phase 13E) and the
    /// crashes (spec §14 Phase 12E).
    markers: Vec<Marker>,
    /// Game time of the last crash marked.
    last_crash_s: Option<f64>,
    /// A pause began since the last record (10D.2): the next record
    /// credits no distance (the rider's roll to a stop).
    after_pause: bool,
}

impl RideRecorder {
    pub fn new(route_name: impl Into<String>) -> Self {
        Self {
            route_name: route_name.into(),
            start_unix_s: None,
            start_game_s: 0.0,
            last: None,
            was_flying: false,
            distance_m: 0.0,
            records: Vec::new(),
            laps: Vec::new(),
            markers: Vec::new(),
            last_crash_s: None,
            after_pause: false,
        }
    }

    /// A crash that began at game time `game_s` (spec §14 Phase 12E: a
    /// FIT marker per crash); the same crash seen again is not marked
    /// twice. Ignored before the first record.
    pub fn mark_crash(&mut self, game_s: f64) {
        if self.start_unix_s.is_none()
            || self.last_crash_s.is_some_and(|t| (t - game_s).abs() < 1.0)
        {
            return;
        }
        self.last_crash_s = Some(game_s);
        self.markers.push(Marker {
            elapsed_s: (game_s - self.start_game_s).max(0.0),
            kind: MarkerKind::Crash,
        });
    }

    /// Starts a new FIT lap at game time `game_s` (a workout step, an
    /// event's start or finish, an event lap). Ignored before the first
    /// record: the ride starts a lap anyway.
    pub fn mark_lap(&mut self, game_s: f64) {
        if self.start_unix_s.is_some() {
            self.laps.push(game_s - self.start_game_s);
        }
    }

    /// The ride's start (UTC Unix s), from its first record on.
    pub fn start_unix_s(&self) -> Option<i64> {
        self.start_unix_s
    }

    /// The lap marks so far (elapsed s).
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
        if s.paused {
            self.after_pause |= self.last.is_some();
            return false;
        }
        if let Some((t, raw)) = self.last {
            let dt = s.game_s - t;
            if dt < RECORD_INTERVAL_S - 1e-6 {
                return false;
            }
            let delta = s.distance - raw;
            let flew = s.flying || self.was_flying || std::mem::take(&mut self.after_pause);
            if !flew && (0.0..=MAX_PLAUSIBLE_SPEED_MPS * dt + 5.0).contains(&delta) {
                self.distance_m += delta;
            }
        } else {
            self.start_unix_s = Some(wall_unix_s);
            self.start_game_s = s.game_s;
        }
        // God mode began or ended (also a ride that starts flying).
        let was = self.was_flying && !self.records.is_empty();
        if s.flying != was && (s.flying || !self.records.is_empty()) {
            self.markers.push(Marker {
                elapsed_s: s.game_s - self.start_game_s,
                kind: if s.flying {
                    MarkerKind::GodModeStart
                } else {
                    MarkerKind::GodModeEnd
                },
            });
        }
        self.last = Some((s.game_s, s.distance));
        self.was_flying = s.flying;
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
        let markers = std::mem::take(&mut self.markers);
        let start_unix_s = self.start_unix_s.take()?;
        self.last = None;
        self.after_pause = false;
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
                markers,
            },
            meta: RideMeta {
                name: format!("Open Cycling World: {}", self.route_name),
                description: format!(
                    "Virtual ride on {} in Open Cycling World. The GPS track is virtual.",
                    self.route_name
                ),
                route: self.route_name.clone(),
                folder: None,
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

/// File stem of a ride started at `unix_s` (UTC): `2026-11-11_12-00-00`.
pub fn file_stem(unix_s: i64) -> String {
    let (year, month, day, h, m, sec) = ocw_core::timestamp::civil(unix_s);
    format!("{year:04}-{month:02}-{day:02}_{h:02}-{m:02}-{sec:02}")
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

/// What the recorder takes from the client at game time `game_s`.
pub fn sample(client: &ClientCore, game_s: f64) -> Sample {
    let s = client.state();
    Sample {
        game_s,
        distance: s.distance,
        speed: s.speed,
        lateral_offset: s.lateral_offset,
        power_w: client.power_w(),
        cadence_rpm: client.cadence_rpm(),
        heart_rate_bpm: client.heart_rate_bpm(),
        flying: client.god().active(),
        paused: client.paused(),
    }
}

/// One frame of recording: a record when one is due, and a crash's FIT
/// marker at its start (spec §14 Phase 12E). `wall_unix_s` is the wall
/// clock (the start time of the ride).
pub fn observe_client(rec: &mut RideRecorder, client: &ClientCore, game_s: f64, wall_unix_s: i64) {
    if client.rider_id().is_none() {
        return;
    }
    rec.observe(
        client.track(),
        client.geo_origin(),
        sample(client, game_s),
        wall_unix_s,
    );
    if let Some(crash) = client.own_crash(game_s) {
        rec.mark_crash(game_s - crash.elapsed_s);
    }
}

/// The ride's FIT file name: `ocw-ride-2026-11-11_12-00-00.fit` (the
/// browser's download; the desktop's files are named by the stem alone).
pub fn download_name(start_unix_s: i64) -> String {
    format!("ocw-ride-{}.fit", file_stem(start_unix_s))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_world::Route;

    #[test]
    fn a_crash_is_a_fit_marker() {
        let mut r = RideRecorder::new("Loop");
        r.mark_crash(1.0);
        assert!(r.markers.is_empty(), "before the first record");
        r.observe(
            &ocw_world::Route::test_loop(),
            None,
            sample(10.0, 0.0),
            1_800_000_000,
        );
        r.mark_crash(12.0);
        r.mark_crash(12.3);
        r.mark_crash(40.0);
        r.observe(
            &ocw_world::Route::test_loop(),
            None,
            sample(41.0, 200.0),
            1_800_000_031,
        );
        let ride = r.finish().unwrap();
        let crashes: Vec<f64> = ride
            .activity
            .markers
            .iter()
            .filter(|m| m.kind == MarkerKind::Crash)
            .map(|m| m.elapsed_s)
            .collect();
        assert_eq!(crashes, vec![2.0, 30.0], "one marker per crash");
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

    /// The Sharing screen (spec 14 Phase 10, 10D.2): while the own ride is
    /// paused nothing is recorded, and the metres it rolled to a stop are
    /// not credited when it resumes; riding on counts again.
    #[test]
    fn a_paused_ride_is_left_out_of_the_recording() {
        let route = Route::test_loop();
        let mut r = RideRecorder::new("Loop");
        let paused = |t, d| Sample {
            paused: true,
            ..sample(t, d)
        };
        r.observe(&route, route.geo_origin(), sample(0.0, 0.0), 0);
        r.observe(&route, route.geo_origin(), sample(1.0, 8.0), 0);
        for t in 2..60 {
            assert!(!r.observe(&route, route.geo_origin(), paused(t as f64, 20.0), 0));
        }
        assert_eq!(r.records().len(), 2, "no records while paused");
        assert_eq!(r.distance_m(), 8.0);
        // Resumed where the rider stopped (12 m on): that roll is not
        // credited, the riding after it is.
        assert!(r.observe(&route, route.geo_origin(), sample(60.0, 20.0), 0));
        assert_eq!(r.distance_m(), 8.0);
        r.observe(&route, route.geo_origin(), sample(61.0, 28.0), 0);
        assert_eq!(r.distance_m(), 16.0);
        // A pause before the first record records nothing either.
        let mut fresh = RideRecorder::new("Loop");
        assert!(!fresh.observe(&route, route.geo_origin(), paused(0.0, 0.0), 0));
        assert!(fresh.records().is_empty() && fresh.start_unix_s().is_none());
    }

    /// God mode (spec §14 Phase 13E): ride time goes on, no distance is
    /// credited while flying, nor the step to the landing spot.
    #[test]
    fn flying_in_god_mode_credits_no_distance() {
        let route = Route::test_loop();
        let mut r = RideRecorder::new("Loop");
        let flying = |t, d| Sample {
            flying: true,
            speed: 0.0,
            ..sample(t, d)
        };
        r.observe(&route, route.geo_origin(), sample(0.0, 0.0), 0);
        r.observe(&route, route.geo_origin(), sample(1.0, 8.0), 0);
        r.observe(&route, route.geo_origin(), flying(2.0, 8.0), 0);
        r.observe(&route, route.geo_origin(), flying(3.0, 8.0), 0);
        // Landed 40 m further along the route.
        r.observe(&route, route.geo_origin(), sample(4.0, 48.0), 0);
        r.observe(&route, route.geo_origin(), sample(5.0, 56.0), 0);
        assert_eq!(r.distance_m(), 16.0);
        assert_eq!(r.records().len(), 6, "the ride time goes on");
        // A FIT marker at both ends of the flight.
        let ride = r.finish().unwrap();
        assert_eq!(
            ride.activity.markers,
            vec![
                Marker {
                    elapsed_s: 2.0,
                    kind: MarkerKind::GodModeStart
                },
                Marker {
                    elapsed_s: 4.0,
                    kind: MarkerKind::GodModeEnd
                },
            ]
        );
        // A ride that starts in the air: marked from its first record.
        let mut r = RideRecorder::new("Loop");
        r.observe(&route, None, flying(0.0, 0.0), 0);
        r.observe(&route, None, flying(1.0, 0.0), 0);
        let ride = r.finish().unwrap();
        assert_eq!(ride.activity.markers.len(), 1);
        assert_eq!(ride.activity.markers[0].kind, MarkerKind::GodModeStart);
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
    fn download_names_carry_the_start_time() {
        assert_eq!(
            download_name(1_762_862_400),
            "ocw-ride-2025-11-11_12-00-00.fit"
        );
    }
}
