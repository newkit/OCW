//! The Strava recording (spec §2.8.1, ST1), Bevy-free: S starts and
//! stops it; the part of the ride between S and S is its own FIT
//! activity (each S … S its own, Q1) with its own totals and laps, while
//! the ride's own recording goes on for the training history. Without
//! the Strava set-up (`strava.toml`) S records nothing and says so. The
//! app saves the activity (`<data dir>/ocw/rides/strava/`) and queues it
//! with the upload; the photos the ride photographer (spec §2.11) took
//! within it belong to it.

use crate::recording::{FinishedRide, RideRecorder, Sample};

/// What S does when Strava is not set up.
pub const NOT_SET_UP: &str = "Strava is not set up (strava.toml)";
/// How long a notice stays (s).
pub const NOTICE_S: f64 = 4.0;

/// What a start or stop did.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Started,
    /// Stopped; the activity (`None`: too short – fewer than two records).
    Stopped(Option<Box<FinishedRide>>),
    NotSetUp,
    /// Start while recording / stop while not.
    Unchanged,
}

/// The Strava recording of a ride.
pub struct StravaRecording {
    /// Strava is set up (`strava.toml` or the environment).
    pub configured: bool,
    route: String,
    recorder: Option<RideRecorder>,
    /// Game time it started.
    since: Option<f64>,
    notice: Option<(String, f64)>,
}

impl StravaRecording {
    pub fn new(configured: bool, route: impl Into<String>) -> Self {
        Self {
            configured,
            route: route.into(),
            recorder: None,
            since: None,
            notice: None,
        }
    }

    /// Another route or world: later recordings carry its name.
    pub fn set_route(&mut self, route: impl Into<String>) {
        self.route = route.into();
    }

    pub fn is_recording(&self) -> bool {
        self.recorder.is_some()
    }

    /// The badge's time: seconds recorded so far (`None`: not recording).
    pub fn elapsed_s(&self, game_s: f64) -> Option<f64> {
        self.since.map(|s| (game_s - s).max(0.0))
    }

    /// The notice to show at `game_s` (start, stop, not set up).
    pub fn notice(&self, game_s: f64) -> Option<&str> {
        self.notice
            .as_ref()
            .filter(|(_, at)| game_s - at < NOTICE_S)
            .map(|(m, _)| m.as_str())
    }

    pub fn say(&mut self, text: impl Into<String>, game_s: f64) {
        self.notice = Some((text.into(), game_s));
    }

    /// S: starts or stops.
    pub fn toggle(&mut self, game_s: f64) -> Outcome {
        if self.is_recording() {
            self.stop(game_s)
        } else {
            self.start(game_s)
        }
    }

    pub fn start(&mut self, game_s: f64) -> Outcome {
        if !self.configured {
            self.say(NOT_SET_UP, game_s);
            return Outcome::NotSetUp;
        }
        if self.is_recording() {
            return Outcome::Unchanged;
        }
        self.recorder = Some(RideRecorder::new(self.route.clone()));
        self.since = Some(game_s);
        self.say("● Strava recording started", game_s);
        Outcome::Started
    }

    /// Stops (S, leaving the ride, quitting): the activity.
    pub fn stop(&mut self, game_s: f64) -> Outcome {
        let Some(mut r) = self.recorder.take() else {
            return Outcome::Unchanged;
        };
        let since = self.since.take().unwrap_or(game_s);
        let ride = r.finish().map(|mut f| {
            f.meta.name = format!("Open Cycling World: {}", self.route);
            f.meta.description = format!(
                "Virtual ride on {} in Open Cycling World (Strava recording). The GPS track is virtual.",
                self.route
            );
            Box::new(f)
        });
        let text = match &ride {
            Some(f) => format!(
                "Strava recording saved: {}, {:.1} km",
                crate::history::hms(game_s - since),
                f.activity.records.last().map_or(0.0, |r| r.distance_m) / 1000.0
            ),
            None => "Strava recording too short, not saved".into(),
        };
        self.say(text, game_s);
        Outcome::Stopped(ride)
    }

    /// A record of the ride (the same samples as the ride's recording).
    pub fn observe(
        &mut self,
        track: &dyn ocw_world::Track,
        geo_origin: Option<(f64, f64)>,
        s: Sample,
        wall_unix_s: i64,
    ) {
        if let Some(r) = self.recorder.as_mut() {
            r.observe(track, geo_origin, s, wall_unix_s);
        }
    }

    /// A lap mark (a workout step, an event part) within the recording.
    pub fn mark_lap(&mut self, game_s: f64) {
        if let Some(r) = self.recorder.as_mut() {
            r.mark_lap(game_s);
        }
    }

    pub fn mark_crash(&mut self, game_s: f64) {
        if let Some(r) = self.recorder.as_mut() {
            r.mark_crash(game_s);
        }
    }

    /// The recording's start (UTC Unix s; after its first record).
    pub fn start_unix_s(&self) -> Option<i64> {
        self.recorder.as_ref().and_then(|r| r.start_unix_s())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_world::Route;

    fn sample(t: f64, d: f64) -> Sample {
        Sample {
            game_s: t,
            distance: d,
            speed: 8.0,
            power_w: 200.0,
            cadence_rpm: Some(88.0),
            heart_rate_bpm: Some(128.0),
            ..Sample::default()
        }
    }

    #[test]
    fn without_strava_set_up_s_records_nothing() {
        let mut s = StravaRecording::new(false, "Loop");
        assert_eq!(s.toggle(1.0), Outcome::NotSetUp);
        assert!(!s.is_recording());
        assert_eq!(s.notice(2.0), Some(NOT_SET_UP));
        assert_eq!(s.notice(1.0 + NOTICE_S), None);
        assert_eq!(s.elapsed_s(3.0), None);
    }

    /// S starts, S stops; stopping twice or leaving the ride after a stop
    /// changes nothing; leaving while recording stops and saves.
    #[test]
    fn s_starts_and_stops() {
        let route = Route::test_loop();
        let mut s = StravaRecording::new(true, "Loop");
        assert_eq!(s.stop(0.0), Outcome::Unchanged);
        assert_eq!(s.toggle(10.0), Outcome::Started);
        assert_eq!(s.start(11.0), Outcome::Unchanged);
        assert!(s.is_recording());
        assert_eq!(s.elapsed_s(70.0), Some(60.0));
        assert!(s.notice(10.5).unwrap().contains("started"));
        for i in 0..30 {
            s.observe(
                &route,
                None,
                sample(10.0 + f64::from(i), f64::from(i) * 8.0),
                1000 + i64::from(i),
            );
        }
        let Outcome::Stopped(Some(ride)) = s.toggle(40.0) else {
            panic!("stopped with an activity")
        };
        assert!(!s.is_recording());
        assert!(
            s.notice(40.0)
                .unwrap()
                .starts_with("Strava recording saved")
        );
        assert_eq!(ride.activity.records.len(), 30);
        assert!(ride.meta.description.contains("Strava recording"));
        // Leaving the ride (or quitting) while recording: stopped too.
        s.toggle(50.0);
        s.observe(&route, None, sample(50.0, 300.0), 2000);
        s.observe(&route, None, sample(51.0, 308.0), 2001);
        assert!(matches!(s.stop(52.0), Outcome::Stopped(Some(_))));
        assert_eq!(s.stop(53.0), Outcome::Unchanged);
        // Too short: nothing to save.
        s.toggle(60.0);
        assert_eq!(s.stop(60.5), Outcome::Stopped(None));
    }

    /// The segment's FIT: starts at S with its own clock and distance
    /// (the totals of the part only), ends at S, keeps the laps within.
    #[test]
    fn the_segment_is_its_own_activity() {
        let route = Route::test_loop();
        let mut ride = RideRecorder::new("Loop");
        let mut s = StravaRecording::new(true, "Loop");
        for i in 0..120 {
            let t = f64::from(i);
            let smp = sample(t, t * 8.0);
            if i == 40 {
                s.start(t);
            }
            if i == 60 {
                s.mark_lap(t);
            }
            ride.observe(&route, None, smp, 5000 + i64::from(i));
            s.observe(&route, None, smp, 5000 + i64::from(i));
            if i == 100 {
                let Outcome::Stopped(Some(seg)) = s.stop(t) else {
                    panic!()
                };
                let a = &seg.activity;
                assert_eq!(a.start_unix_s, 5040);
                assert_eq!(a.records.len(), 61);
                assert_eq!(a.records[0].elapsed_s, 0.0);
                assert_eq!(a.records.last().unwrap().elapsed_s, 60.0);
                assert_eq!(a.records[0].distance_m, 0.0);
                assert!((a.records.last().unwrap().distance_m - 480.0).abs() < 1e-6);
                assert_eq!(a.laps, vec![20.0]);
                // A valid FIT file.
                let bytes = ocw_fit::encode(a);
                assert_eq!(ocw_fit::crc16(0, &bytes), 0);
            }
        }
        // The ride's own recording went on.
        assert_eq!(ride.records().len(), 120);
    }
}
