//! The Strava recording (spec §2.8.1, ST1), Bevy-free: S starts and
//! stops it; the part of the ride between S and S is its own FIT
//! activity (each S … S its own, Q1) with its own totals and laps, while
//! the ride's own recording goes on for the training history. The app
//! saves the activity (`<data dir>/ocw/rides/strava/`); with the API
//! set-up (`strava.toml`, the advanced way) it is queued with the
//! automatic upload, without it the notice points to the simple upload
//! in the browser ([`NOT_SET_UP`]). Only these recordings go to Strava,
//! never whole rides (spec 2.8.1 Q6): automatically when the setting
//! `strava.auto_upload` is on, else by hand. The photos the ride photographer
//! (spec §2.11) took within it belong to it.
//!
//! **The simple upload (no API app, spec §2.8.1 *Simple upload*):**
//! Strava's own upload page ([`UPLOAD_PAGE`]) takes our FIT files; the
//! apps open it with the file at hand (desktop: its folder open; browser:
//! downloaded) and show [`upload_steps`]. The file gets a name a rider
//! recognises in a file picker ([`upload_name`]).

use crate::recording::{FinishedRide, RideRecorder, Sample};

/// Strava's web upload page (accepts FIT files; free for every account).
pub const UPLOAD_PAGE: &str = "https://www.strava.com/upload/select";

/// Where a ride goes to Strava without the API set-up (the S recording's
/// stop and the end of a ride say it).
pub const NOT_SET_UP: &str =
    "Upload it to Strava in your browser: Navi > Training history > the ride";
/// What the desktop's stop notice adds without the set-up: the recording
/// waits and goes to Strava once Strava is set up (spec §2.8.1
/// *Recording without Strava set up, uploaded later*).
pub const WAITS_FOR_SET_UP: &str = "It will be uploaded once Strava is set up";
/// What the stop notice adds with the API set-up and the automatic
/// upload on (setting `strava.auto_upload`, spec 2.8.1 Q6).
pub const UPLOADS_AUTOMATICALLY: &str = "It goes to Strava automatically";
/// … with the automatic upload off: it waits for the rider's upload.
pub const WAITS_FOR_UPLOAD: &str =
    "Automatic upload is off: upload it from Navi > Training history > the ride";
/// Whole rides never go to Strava (spec 2.8.1 Q6): a ride page without a
/// Strava recording says how to make one.
pub const NO_RECORDING: &str = "no Strava recording – press S while riding";

/// Whether a recording that starts at `rec_start_unix_s` lies within a
/// ride from `ride_start_unix_s` lasting `ride_elapsed_s`: it starts at
/// or after the ride's first record (a second of slack) and before its
/// end. Both apps find a ride's Strava recordings with it.
pub fn within_ride(rec_start_unix_s: i64, ride_start_unix_s: i64, ride_elapsed_s: f64) -> bool {
    let end = ride_start_unix_s + ride_elapsed_s.ceil() as i64;
    rec_start_unix_s + 1 >= ride_start_unix_s && rec_start_unix_s <= end
}

/// How long a notice stays (s).
pub const NOTICE_S: f64 = 4.0;
/// How long a notice that points to the upload stays (s).
pub const UPLOAD_NOTICE_S: f64 = 12.0;

/// The steps of the simple upload. `files`: the file names to choose (the
/// first named, the others counted); `desktop`: the folder is open (else
/// the file was downloaded); `photos`: the ride's photos lie in the same
/// folder (desktop); `folder`: the open folder as the HUD names it (the
/// recording's own folder, `Activities/OCW/2026-11-18 Puerto de la Cruz`).
pub fn upload_steps(files: &[String], desktop: bool, photos: bool, folder: Option<&str>) -> String {
    let first = files.first().map_or("", String::as_str);
    let which = match files.len() {
        0 | 1 => format!("the file {first}"),
        n => format!(
            "the files {first} and the {} other{}",
            n - 1,
            if n == 2 { "" } else { "s" }
        ),
    };
    let (opens, at_hand) = match (desktop, folder) {
        (true, Some(f)) => (
            "Strava opens in your browser",
            format!("its folder {f} is open"),
        ),
        (true, None) => (
            "Strava opens in your browser",
            "its folder is open".to_string(),
        ),
        (false, Some(f)) => ("Strava opens in a new tab", format!("in {f}")),
        (false, None) => ("Strava opens in a new tab", "just downloaded".to_string()),
    };
    let mut s = format!("{opens}: sign in, choose {which} ({at_hand}), then Upload.");
    if photos {
        s.push_str(" Photos: add them on the activity page (they are in the same folder).");
    } else {
        s.push_str(" Photos: add them on the activity page.");
    }
    s
}

/// A text as part of a file name every system takes: letters (any
/// script), digits, spaces and `-_.,()'&+–`; anything else (`/ \ : * ? "
/// < > |`, control characters) becomes `-`, white space a space; runs of
/// spaces become one, no leading or trailing dots or spaces; at most
/// `max` characters.
pub fn safe_file_part(text: &str, max: usize) -> String {
    let mut out = String::new();
    for c in text.chars() {
        let c = if c.is_alphanumeric() || " -_.,()'&+–".contains(c) {
            c
        } else if c.is_whitespace() {
            ' '
        } else {
            '-'
        };
        if c == ' ' && (out.is_empty() || out.ends_with(' ')) {
            continue;
        }
        out.push(c);
    }
    let out: String = out.chars().take(max).collect();
    out.trim_matches(|c| c == ' ' || c == '.').to_string()
}

/// The FIT file's name for the upload, recognisable in a file picker:
/// `OCW 2026-11-18 10-05 Tenerife – Puerto de la Cruz–Teide.fit` (the
/// local start time, the world and the route; `tag` in brackets after
/// them, e.g. "Strava recording"). Only the copy for the upload (and the
/// browser's download) carries it; the saved rides keep their names, so
/// the history, the photos and the API sync stay as they are.
pub fn upload_name(
    start_unix_s: i64,
    offset_s: i32,
    world: &str,
    route: &str,
    tag: Option<&str>,
) -> String {
    let (y, mo, d, h, mi, _) = ocw_core::timestamp::civil(start_unix_s + i64::from(offset_s));
    let place = match (world.trim(), route.trim()) {
        ("", r) => r.to_string(),
        (w, r) if r.is_empty() || w == r => w.to_string(),
        (w, r) => format!("{w} – {r}"),
    };
    let mut name = format!("OCW {y:04}-{mo:02}-{d:02} {h:02}-{mi:02}");
    let place = safe_file_part(&place, 80);
    if !place.is_empty() {
        name.push(' ');
        name.push_str(&place);
    }
    if let Some(t) = tag {
        name.push_str(&format!(" ({})", safe_file_part(t, 40)));
    }
    format!("{name}.fit")
}

/// What a start or stop did.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Started,
    /// Stopped; the activity (`None`: too short – fewer than two records).
    Stopped(Option<Box<FinishedRide>>),
    /// Start while recording / stop while not.
    Unchanged,
}

/// The Strava recording of a ride.
pub struct StravaRecording {
    /// The automatic API upload is set up (`strava.toml` or the
    /// environment); without it the recording is uploaded in the browser.
    pub api: bool,
    /// Without the API set-up the app keeps the recording waiting and
    /// uploads it once the credentials are there (the desktop; the
    /// browser has no API upload): the stop notice says so.
    pub waits: bool,
    /// The automatic upload of the Strava recordings (setting
    /// `strava.auto_upload`, spec 2.8.1 Q6; default on): off, a recording
    /// waits for the rider's upload by hand; the stop notice says which.
    pub auto: bool,
    route: String,
    recorder: Option<RideRecorder>,
    /// Game time it started.
    since: Option<f64>,
    /// The notice, when it was shown and for how long.
    notice: Option<(String, f64, f64)>,
    /// Where the running recording is written, as the HUD names it
    /// (`Activities/OCW/2026-11-18 Puerto de la Cruz`, spec §2.8.1 *Where
    /// an S recording is written*); its stop notice says it.
    folder: Option<String>,
}

impl StravaRecording {
    pub fn new(api: bool, route: impl Into<String>) -> Self {
        Self {
            api,
            waits: false,
            auto: true,
            route: route.into(),
            recorder: None,
            since: None,
            notice: None,
            folder: None,
        }
    }

    /// Where the running recording goes (the app sets it at the start;
    /// the stop notice names it).
    pub fn set_folder(&mut self, label: Option<String>) {
        self.folder = label;
    }

    /// Where the running recording goes, as the HUD names it.
    pub fn folder(&self) -> Option<&str> {
        self.folder.as_deref()
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

    /// The notice to show at `game_s` (start, stop, the upload's steps).
    pub fn notice(&self, game_s: f64) -> Option<&str> {
        self.notice
            .as_ref()
            .filter(|(_, at, for_s)| game_s - at < *for_s)
            .map(|(m, ..)| m.as_str())
    }

    pub fn say(&mut self, text: impl Into<String>, game_s: f64) {
        self.say_for(text, game_s, NOTICE_S);
    }

    /// A notice that stays `for_s` seconds (the upload's steps).
    pub fn say_for(&mut self, text: impl Into<String>, game_s: f64, for_s: f64) {
        self.notice = Some((text.into(), game_s, for_s));
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
        let folder = self.folder.take();
        let ride = r.finish().map(|mut f| {
            f.meta.name = format!("Open Cycling World: {}", self.route);
            f.meta.description = format!(
                "Virtual ride on {} in Open Cycling World (Strava recording). The GPS track is virtual.",
                self.route
            );
            Box::new(f)
        });
        match &ride {
            Some(f) => {
                let saved = folder.map_or_else(|| "saved".to_string(), |f| format!("saved to {f}"));
                let text = format!(
                    "Strava recording {saved}: {}, {:.1} km",
                    crate::history::hms(game_s - since),
                    f.activity.records.last().map_or(0.0, |r| r.distance_m) / 1000.0
                );
                if self.api && self.auto {
                    self.say(format!("{text}. {UPLOADS_AUTOMATICALLY}."), game_s);
                } else if self.api {
                    // Automatic upload off: the rider uploads it.
                    self.say_for(
                        format!("{text}. {WAITS_FOR_UPLOAD}."),
                        game_s,
                        UPLOAD_NOTICE_S,
                    );
                } else {
                    // No API: the rider uploads it in the browser – or
                    // the desktop uploads it once Strava is set up (with
                    // the automatic upload on).
                    let waits = if self.waits && self.auto {
                        format!(" {WAITS_FOR_SET_UP}.")
                    } else {
                        String::new()
                    };
                    self.say_for(
                        format!("{text}.{waits} {NOT_SET_UP}"),
                        game_s,
                        UPLOAD_NOTICE_S,
                    );
                }
            }
            None => self.say("Strava recording too short, not saved", game_s),
        }
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

    /// Without the API set-up S records too (spec §2.8.1 *Simple
    /// upload*); its stop points to the upload in the browser, longer.
    #[test]
    fn without_the_api_s_records_and_points_to_the_browser_upload() {
        let route = Route::test_loop();
        let mut s = StravaRecording::new(false, "Loop");
        assert_eq!(s.toggle(1.0), Outcome::Started);
        assert!(s.is_recording());
        for i in 0..10 {
            s.observe(
                &route,
                None,
                sample(1.0 + f64::from(i), f64::from(i) * 8.0),
                100 + i64::from(i),
            );
        }
        assert!(matches!(s.toggle(11.0), Outcome::Stopped(Some(_))));
        let n = s.notice(11.0).unwrap();
        assert!(
            n.starts_with("Strava recording saved") && n.ends_with(NOT_SET_UP),
            "{n}"
        );
        assert!(!n.contains("strava.toml"));
        assert!(s.notice(11.0 + NOTICE_S + 1.0).is_some(), "stays longer");
        assert_eq!(s.notice(11.0 + UPLOAD_NOTICE_S), None);
        // With the API: the short notice, no pointer.
        let mut a = StravaRecording::new(true, "Loop");
        a.start(0.0);
        a.observe(&route, None, sample(0.0, 0.0), 1);
        a.observe(&route, None, sample(1.0, 8.0), 2);
        a.stop(2.0);
        assert!(!a.notice(2.0).unwrap().contains("browser"));
        assert!(
            a.notice(2.0)
                .unwrap()
                .ends_with(". It goes to Strava automatically."),
            "{:?}",
            a.notice(2.0)
        );
        assert_eq!(a.notice(2.0 + NOTICE_S), None);
        // The automatic upload off (spec 2.8.1 Q6): it waits for the
        // rider's upload, the notice says where, longer.
        let mut off = StravaRecording::new(true, "Loop");
        off.auto = false;
        off.start(0.0);
        off.observe(&route, None, sample(0.0, 0.0), 1);
        off.observe(&route, None, sample(1.0, 8.0), 2);
        off.stop(2.0);
        let n = off.notice(2.0).unwrap();
        assert!(n.ends_with(&format!(". {WAITS_FOR_UPLOAD}.")), "{n}");
        assert!(!n.contains(UPLOADS_AUTOMATICALLY), "{n}");
        assert!(off.notice(2.0 + NOTICE_S + 1.0).is_some());
        // Without the API and with it off: no promise of an upload.
        let mut w = StravaRecording::new(false, "Loop");
        w.waits = true;
        w.auto = false;
        w.start(0.0);
        w.observe(&route, None, sample(0.0, 0.0), 1);
        w.observe(&route, None, sample(1.0, 8.0), 2);
        w.stop(2.0);
        let n = w.notice(2.0).unwrap();
        assert!(
            !n.contains(WAITS_FOR_SET_UP) && n.ends_with(NOT_SET_UP),
            "{n}"
        );
        // The desktop keeps it waiting for the set-up (spec §2.8.1
        // *Recording without Strava set up*): the notice says it is saved
        // and will be uploaded, beside the browser's way.
        let mut d = StravaRecording::new(false, "Loop");
        d.waits = true;
        d.start(0.0);
        d.observe(&route, None, sample(0.0, 0.0), 1);
        d.observe(&route, None, sample(1.0, 8.0), 2);
        d.stop(2.0);
        let n = d.notice(2.0).unwrap();
        assert!(
            n.contains(". It will be uploaded once Strava is set up. ") && n.ends_with(NOT_SET_UP),
            "{n}"
        );
        assert!(
            !s.notice(11.0).unwrap().contains(WAITS_FOR_SET_UP),
            "the browser"
        );
    }

    /// The stop notice names the recording's folder (spec §2.8.1 *Where
    /// an S recording is written*); the next recording starts without it.
    #[test]
    fn the_stop_notice_names_the_folder() {
        let route = Route::test_loop();
        let mut s = StravaRecording::new(false, "Loop");
        s.start(0.0);
        s.set_folder(Some("Activities/OCW/2026-11-18 Puerto de la Cruz".into()));
        assert_eq!(
            s.folder(),
            Some("Activities/OCW/2026-11-18 Puerto de la Cruz")
        );
        s.observe(&route, None, sample(0.0, 0.0), 1);
        s.observe(&route, None, sample(1.0, 8.0), 2);
        s.stop(2.0);
        let n = s.notice(2.0).unwrap();
        assert!(
            n.starts_with(
                "Strava recording saved to Activities/OCW/2026-11-18 Puerto de la Cruz: 0:02"
            ),
            "{n}"
        );
        assert_eq!(s.folder(), None);
    }

    /// A ride's Strava recordings: those that start within it.
    #[test]
    fn recordings_within_a_ride() {
        assert!(within_ride(1000, 1000, 600.0));
        assert!(within_ride(999, 1000, 600.0), "a second of slack");
        assert!(within_ride(1600, 1000, 599.5));
        assert!(!within_ride(998, 1000, 600.0));
        assert!(!within_ride(1601, 1000, 600.0));
    }

    #[test]
    fn the_upload_page_is_stravas_own() {
        assert_eq!(UPLOAD_PAGE, "https://www.strava.com/upload/select");
        assert!(!NOT_SET_UP.contains("strava.toml"));
    }

    /// The upload's file name: recognisable (date, time, world, route),
    /// safe on every system.
    #[test]
    fn upload_names_are_recognisable_and_safe() {
        // 2026-11-18 09:05 UTC, shown at UTC+1 (the rider's own zone).
        let t = 1_794_992_700;
        assert_eq!(
            upload_name(t, 3600, "Tenerife", "Puerto de la Cruz–Teide", None),
            "OCW 2026-11-18 10-05 Tenerife – Puerto de la Cruz–Teide.fit"
        );
        assert_eq!(
            upload_name(t, 0, "", "Free ride", Some("Strava recording")),
            "OCW 2026-11-18 09-05 Free ride (Strava recording).fit"
        );
        assert_eq!(
            upload_name(t, 0, "Loop", "Loop", None),
            "OCW 2026-11-18 09-05 Loop.fit"
        );
        assert_eq!(upload_name(t, 0, "", "", None), "OCW 2026-11-18 09-05.fit");
        // Forbidden and control characters, runs of spaces, trailing dots.
        let n = upload_name(t, 0, "A/B\\C: *?\"<>|", "x\ty   z...", None);
        assert!(
            !n.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|', '\t']),
            "{n}"
        );
        assert_eq!(n, "OCW 2026-11-18 09-05 A-B-C- ------ – x y z.fit");
        assert_eq!(
            safe_file_part("  Black Forest - north  ", 80),
            "Black Forest - north"
        );
        assert_eq!(safe_file_part("Ünterwäld ...", 80), "Ünterwäld");
        assert_eq!(safe_file_part(&"x".repeat(200), 80).len(), 80);
    }

    #[test]
    fn the_steps_name_the_file_and_the_photos() {
        let one = vec!["OCW 2026-11-18 10-05 Loop.fit".to_string()];
        assert_eq!(
            upload_steps(&one, true, false, None),
            "Strava opens in your browser: sign in, choose the file OCW 2026-11-18 10-05 Loop.fit (its folder is open), then Upload. Photos: add them on the activity page."
        );
        let web = upload_steps(&one, false, false, None);
        assert!(
            web.starts_with("Strava opens in a new tab") && web.contains("just downloaded"),
            "{web}"
        );
        let two = vec!["a.fit".to_string(), "b.fit".to_string()];
        let s = upload_steps(&two, true, true, None);
        assert!(s.contains("the files a.fit and the 1 other ("), "{s}");
        assert!(s.contains("in the same folder"), "{s}");
        // The recording's own folder (spec §2.8.1 *Where an S recording
        // is written*) is named.
        let f = upload_steps(
            &one,
            true,
            true,
            Some("Activities/OCW/2026-11-18 Puerto de la Cruz"),
        );
        assert!(
            f.contains("(its folder Activities/OCW/2026-11-18 Puerto de la Cruz is open)"),
            "{f}"
        );
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
