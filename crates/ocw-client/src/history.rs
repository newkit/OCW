//! The training history (spec §14 Phase 7): a local list of the rider's
//! rides, one file per rider profile, built from the FIT recordings the
//! app writes ([`RideEntry::from_activity`], at the end of each ride, or
//! from an older FIT file read back with `ocw_fit::decode`).
//!
//! Each ride keeps its date, world and route, distance, time, climbing,
//! average and normalized power, heart rate, work, hearts earned, crashes
//! and damage, the workout, its power curve (best mean power for 5 s to
//! 60 min, [`CURVE_S`]) and its climbs ([`climbs`]). From these come the
//! weekly and monthly totals ([`History::totals`]) and the personal bests
//! ([`History::bests`]).
//!
//! **Format (versioned, for the Phase 5 account sync):** JSON,
//! `{"format": "ocw-history", "version": 1, "rider": NAME, "rides":
//! [...]}` – the desktop keeps it in `<data dir>/ocw/history/<rider>.json`,
//! the browser in `localStorage` (`ocw-history`). Fields are only ever
//! added (with defaults), never renamed; a file of a newer version than
//! this build knows is read but not written ([`HistoryError::TooNew`]),
//! so an older app never destroys what a newer one wrote.
//!
//! Pure (no files, no clock): the apps pass the time and the local UTC
//! offset; builds for `wasm32-unknown-unknown`.

use ocw_fit::{Activity, MarkerKind, Record};
use serde::{Deserialize, Serialize};

/// The file's `format` tag.
pub const FORMAT: &str = "ocw-history";
/// The format version this build writes (and the newest it reads).
pub const VERSION: u32 = 1;

/// The durations of the power curve (s): 5 s to 60 min.
pub const CURVE_S: [u32; 10] = [5, 10, 30, 60, 120, 300, 600, 1200, 1800, 3600];

/// A climb needs this much gain (m) …
pub const CLIMB_MIN_GAIN_M: f64 = 30.0;
/// … over at least this distance (m) …
pub const CLIMB_MIN_LENGTH_M: f64 = 300.0;
/// … at this average gradient or more.
pub const CLIMB_MIN_GRADE: f64 = 0.03;
/// A climb ends when the road drops this far below its top (m).
pub const CLIMB_END_DROP_M: f64 = 10.0;

/// One point of a power curve: the best mean power over `s` seconds.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CurvePoint {
    pub s: u32,
    pub w: f64,
}

/// A climb of a ride.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Climb {
    /// Where it starts (distance into the ride, m).
    pub start_m: f64,
    pub length_m: f64,
    pub gain_m: f64,
    pub time_s: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avg_power_w: Option<f64>,
}

impl Climb {
    /// Average gradient (0.05 = 5 %).
    pub fn grade(&self) -> f64 {
        if self.length_m > 0.0 {
            self.gain_m / self.length_m
        } else {
            0.0
        }
    }

    /// Climbing speed (VAM, m/h).
    pub fn vam(&self) -> f64 {
        if self.time_s > 0.0 {
            self.gain_m / self.time_s * 3600.0
        } else {
            0.0
        }
    }
}

/// One lap of a ride (spec 14 Phase 7, 7A): the FIT recording's laps –
/// a workout step, an event's start, lap or finish –, only for a ride of
/// more than one lap.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Lap {
    /// Where it starts: seconds and metres into the ride.
    pub start_s: f64,
    pub start_m: f64,
    pub elapsed_s: f64,
    pub distance_m: f64,
    #[serde(default)]
    pub ascent_m: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avg_power_w: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avg_hr_bpm: Option<f64>,
}

impl Lap {
    /// Average speed (m/s).
    pub fn speed_mps(&self) -> f64 {
        if self.elapsed_s > 0.0 {
            self.distance_m / self.elapsed_s
        } else {
            0.0
        }
    }
}

/// The laps of a recorded ride ([`Activity::lap_records`]); none for a
/// ride of one lap.
pub fn laps(a: &Activity) -> Vec<Lap> {
    let parts = a.lap_records();
    if parts.len() < 2 {
        return Vec::new();
    }
    parts
        .iter()
        .filter_map(|r| {
            let first = r.first()?;
            let s = ocw_fit::Summary::of(r);
            Some(Lap {
                start_s: round1(first.elapsed_s),
                start_m: round1(first.distance_m),
                elapsed_s: round1(s.elapsed_s),
                distance_m: round1(s.distance_m),
                ascent_m: round1(s.ascent_m),
                avg_power_w: round_opt(s.avg_power_w),
                avg_hr_bpm: round_opt(s.avg_heart_rate_bpm),
            })
        })
        .collect()
}

/// The ride's track for its map: the records' virtual GPS positions
/// (latitude, longitude), thinned to at most
/// [`ocw_map::ride::TRACK_MAX_POINTS`]; empty without positions (a route
/// off real roads).
pub fn track(a: &Activity) -> Vec<[f64; 2]> {
    let pts: Vec<[f64; 2]> = a
        .records
        .iter()
        .filter_map(|r| r.position.map(|(lat, lon)| [lat, lon]))
        .collect();
    ocw_map::ride::simplify(&pts, ocw_map::ride::TRACK_MAX_POINTS)
}

/// The rider's and the bike's condition when the ride ended (0–1, spec
/// §14 Phase 12E), with damage on.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EndDamage {
    pub rider: f64,
    pub bike: f64,
}

/// One ride of the history.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct RideEntry {
    /// Its id: the FIT file's stem (`2026-11-14_10-00-00`), unique per
    /// rider.
    pub id: String,
    /// Start (UTC Unix seconds).
    pub start_unix_s: i64,
    /// The world ("Black Forest - north", "Test route").
    #[serde(default)]
    pub world: String,
    /// The route ("Free ride", a route's name).
    #[serde(default)]
    pub route: String,
    pub distance_m: f64,
    /// From the first to the last record (s).
    pub elapsed_s: f64,
    /// Seconds with the bike moving.
    #[serde(default)]
    pub moving_s: f64,
    #[serde(default)]
    pub ascent_m: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avg_power_w: Option<f64>,
    /// Normalized power (30 s rolling mean, fourth-power mean).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub np_w: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_power_w: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avg_hr_bpm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_hr_bpm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avg_cadence_rpm: Option<f64>,
    /// Mechanical work (kJ).
    #[serde(default)]
    pub work_kj: f64,
    /// Hearts the ride earned (spec §14 Phase 4, 4E); `None`: unknown
    /// (imported from a FIT file).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hearts: Option<u32>,
    #[serde(default)]
    pub crashes: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub damage: Option<EndDamage>,
    /// The workout ridden (the last one started in the ride).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workout: Option<String>,
    #[serde(default)]
    pub power_curve: Vec<CurvePoint>,
    #[serde(default)]
    pub climbs: Vec<Climb>,
    /// Its laps (a ride of more than one lap; [`laps`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub laps: Vec<Lap>,
    /// Its track for the ride's map: latitude, longitude (degrees, to
    /// about a metre), thinned ([`track`]); empty off real roads.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub track: Vec<[f64; 2]>,
    /// The FIT file's name in the rides folder (desktop), if kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fit: Option<String>,
    /// Read from a FIT file afterwards (`history.import`): no hearts,
    /// damage, world or workout.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub imported: bool,
    /// The rider marked it as uploaded to Strava (by hand, in the
    /// browser: the app cannot know; spec §2.8.1 *Simple upload*). The
    /// API sync skips it, so a later API set-up uploads no duplicate.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub strava_marked: bool,
}

/// What the app knows of a ride beyond its FIT records.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RideContext {
    pub id: String,
    pub world: String,
    pub route: String,
    pub hearts: Option<u32>,
    pub damage: Option<EndDamage>,
    pub workout: Option<String>,
    pub fit: Option<String>,
    pub imported: bool,
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

fn round_opt(v: Option<f64>) -> Option<f64> {
    v.map(round1)
}

/// The ride's power once per second: each record's power for the time
/// since the one before (at most a minute; gaps longer than that count a
/// minute), missing power as 0.
pub fn per_second(records: &[Record]) -> Vec<f64> {
    let mut out = Vec::with_capacity(records.len());
    for w in records.windows(2) {
        let dt = (w[1].elapsed_s - w[0].elapsed_s).round().clamp(0.0, 60.0) as usize;
        let p = w[1].power_w.unwrap_or(0.0).max(0.0);
        out.extend(std::iter::repeat_n(p, dt));
    }
    out
}

/// The best mean power over `s` seconds of `watts` (one value a second);
/// `None` if the ride is shorter.
pub fn best_mean(watts: &[f64], s: usize) -> Option<f64> {
    if s == 0 || watts.len() < s {
        return None;
    }
    let mut sum: f64 = watts[..s].iter().sum();
    let mut best = sum;
    for i in s..watts.len() {
        sum += watts[i] - watts[i - s];
        best = best.max(sum);
    }
    Some(best / s as f64)
}

/// The power curve: the best mean power for each of [`CURVE_S`] the ride
/// is long enough for.
pub fn power_curve(watts: &[f64]) -> Vec<CurvePoint> {
    CURVE_S
        .iter()
        .filter_map(|&s| best_mean(watts, s as usize).map(|w| CurvePoint { s, w: round1(w) }))
        .collect()
}

/// Normalized power: the 30 s rolling mean's fourth-power mean, its
/// fourth root; `None` for rides under 30 s.
pub fn normalized_power(watts: &[f64]) -> Option<f64> {
    const W: usize = 30;
    if watts.len() < W {
        return None;
    }
    let mut sum: f64 = watts[..W].iter().sum();
    let mut acc = (sum / W as f64).powi(4);
    for i in W..watts.len() {
        sum += watts[i] - watts[i - W];
        acc += (sum / W as f64).powi(4);
    }
    let n = (watts.len() - W + 1) as f64;
    Some((acc / n).powf(0.25))
}

/// The climbs of a ride: stretches from a low point to a high point with
/// at least [`CLIMB_MIN_GAIN_M`] gain over [`CLIMB_MIN_LENGTH_M`] at
/// [`CLIMB_MIN_GRADE`] or more on average, ended where the road drops
/// [`CLIMB_END_DROP_M`] below the top (or the ride ends).
pub fn climbs(records: &[Record]) -> Vec<Climb> {
    let pts: Vec<(usize, f64)> = records
        .iter()
        .enumerate()
        .filter_map(|(i, r)| r.altitude_m.map(|a| (i, a)))
        .collect();
    let mut out = Vec::new();
    let Some(&first) = pts.first() else {
        return out;
    };
    let (mut low, mut high) = (first, first);
    let close = |low: (usize, f64), high: (usize, f64), out: &mut Vec<Climb>| {
        let (a, b) = (&records[low.0], &records[high.0]);
        let gain = high.1 - low.1;
        let length = b.distance_m - a.distance_m;
        if gain >= CLIMB_MIN_GAIN_M
            && length >= CLIMB_MIN_LENGTH_M
            && gain / length >= CLIMB_MIN_GRADE
        {
            let span = &records[low.0..=high.0];
            let p: Vec<f64> = span.iter().filter_map(|r| r.power_w).collect();
            out.push(Climb {
                start_m: round1(a.distance_m),
                length_m: round1(length),
                gain_m: round1(gain),
                time_s: round1(b.elapsed_s - a.elapsed_s),
                avg_power_w: (!p.is_empty())
                    .then(|| round1(p.iter().sum::<f64>() / p.len() as f64)),
            });
        }
    };
    for &p in &pts[1..] {
        if p.1 > high.1 {
            high = p;
        } else if p.1 < high.1 - CLIMB_END_DROP_M {
            close(low, high, &mut out);
            low = p;
            high = p;
        }
        // Level or lower than the start: the climb starts here.
        if p.1 <= low.1 {
            low = p;
            high = p;
        }
    }
    close(low, high, &mut out);
    out
}

impl RideEntry {
    /// The history entry of a recorded ride.
    pub fn from_activity(a: &Activity, ctx: RideContext) -> Self {
        let s = ocw_fit::Summary::of(&a.records);
        let watts = per_second(&a.records);
        let moving_s: f64 = a
            .records
            .windows(2)
            .filter(|w| w[1].speed_mps > 0.3)
            .map(|w| (w[1].elapsed_s - w[0].elapsed_s).clamp(0.0, 60.0))
            .sum();
        let has_power = a.records.iter().any(|r| r.power_w.is_some());
        Self {
            id: ctx.id,
            start_unix_s: a.start_unix_s,
            world: ctx.world,
            route: ctx.route,
            distance_m: round1(s.distance_m),
            elapsed_s: s.elapsed_s,
            moving_s,
            ascent_m: round1(s.ascent_m),
            avg_power_w: round_opt(s.avg_power_w),
            np_w: if has_power {
                round_opt(normalized_power(&watts))
            } else {
                None
            },
            max_power_w: round_opt(s.max_power_w),
            avg_hr_bpm: round_opt(s.avg_heart_rate_bpm),
            max_hr_bpm: round_opt(s.max_heart_rate_bpm),
            avg_cadence_rpm: round_opt(s.avg_cadence_rpm),
            work_kj: round1(s.work_kj),
            hearts: ctx.hearts,
            crashes: a
                .markers
                .iter()
                .filter(|m| m.kind == MarkerKind::Crash)
                .count() as u32,
            damage: ctx.damage,
            workout: ctx.workout,
            power_curve: if has_power {
                power_curve(&watts)
            } else {
                Vec::new()
            },
            climbs: climbs(&a.records),
            laps: laps(a),
            track: track(a),
            fit: ctx.fit,
            imported: ctx.imported,
            strava_marked: false,
        }
    }

    /// The best mean power over `s` seconds, if the curve has it.
    pub fn curve_at(&self, s: u32) -> Option<f64> {
        self.power_curve.iter().find(|p| p.s == s).map(|p| p.w)
    }
}

/// Why a history text could not be used.
#[derive(Debug, Clone, PartialEq)]
pub enum HistoryError {
    /// Not JSON, or not a history.
    Broken(String),
    /// Written by a newer app (its version): readable here as far as this
    /// build knows the fields, but never written back.
    TooNew(u32, Box<History>),
}

impl std::fmt::Display for HistoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Broken(e) => write!(f, "the training history is broken: {e}"),
            Self::TooNew(v, _) => write!(
                f,
                "the training history is of version {v}, newer than this app knows ({VERSION}): read-only"
            ),
        }
    }
}

/// A rider's rides.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct History {
    pub format: String,
    pub version: u32,
    #[serde(default)]
    pub rider: String,
    /// Oldest first.
    #[serde(default)]
    pub rides: Vec<RideEntry>,
}

impl Default for History {
    fn default() -> Self {
        Self::new("")
    }
}

/// Weeks or months.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    Week,
    Month,
}

/// The totals of one week or month.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Total {
    /// "Week of 9 Nov 2026", "November 2026".
    pub label: String,
    /// First day of the period (days since 1970-01-01, local).
    pub first_day: i64,
    pub rides: u32,
    pub distance_m: f64,
    pub time_s: f64,
    pub ascent_m: f64,
    pub work_kj: f64,
    pub hearts: u32,
}

/// A personal best and the ride it is from.
#[derive(Debug, Clone, PartialEq)]
pub struct Best<T> {
    pub value: T,
    /// The ride's index in [`History::rides`].
    pub ride: usize,
    pub start_unix_s: i64,
}

/// The rider's personal bests.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Bests {
    /// The best power for each of [`CURVE_S`] ridden so far.
    pub curve: Vec<(u32, Best<f64>)>,
    /// The climb with the most gain.
    pub biggest_climb: Option<Best<Climb>>,
    /// The climb ridden fastest upwards (highest VAM).
    pub fastest_climb: Option<Best<Climb>>,
    pub longest_ride: Option<Best<f64>>,
    pub most_climbing: Option<Best<f64>>,
}

impl Bests {
    pub fn power(&self, s: u32) -> Option<&Best<f64>> {
        self.curve.iter().find(|(d, _)| *d == s).map(|(_, b)| b)
    }
}

pub const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

pub const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// Local days since 1970-01-01 of `unix_s` at UTC offset `offset_s`.
pub fn local_day(unix_s: i64, offset_s: i32) -> i64 {
    (unix_s + i64::from(offset_s)).div_euclid(86_400)
}

/// "14 Nov 2026" of a local day.
pub fn day_label(day: i64) -> String {
    let (y, m, d, ..) = ocw_core::timestamp::civil(day * 86_400);
    format!("{d} {} {y}", &MONTHS[m as usize - 1][..3])
}

/// "Sat 14 Nov 2026 10:05" (local time).
pub fn start_label(unix_s: i64, offset_s: i32) -> String {
    let local = unix_s + i64::from(offset_s);
    let (_, _, _, h, mi, _) = ocw_core::timestamp::civil(local);
    let day = local.div_euclid(86_400);
    format!(
        "{} {} {h:02}:{mi:02}",
        WEEKDAYS[(day + 3).rem_euclid(7) as usize],
        day_label(day)
    )
}

/// The first day of `day`'s period and the period's label.
fn period_of(day: i64, period: Period) -> (i64, String) {
    match period {
        Period::Week => {
            let monday = day - (day + 3).rem_euclid(7);
            (monday, format!("Week of {}", day_label(monday)))
        }
        Period::Month => {
            let (y, m, d, ..) = ocw_core::timestamp::civil(day * 86_400);
            (
                day - i64::from(d) + 1,
                format!("{} {y}", MONTHS[m as usize - 1]),
            )
        }
    }
}

impl History {
    pub fn new(rider: &str) -> Self {
        Self {
            format: FORMAT.into(),
            version: VERSION,
            rider: rider.into(),
            rides: Vec::new(),
        }
    }

    /// Reads a history text. A newer version is returned inside
    /// [`HistoryError::TooNew`] (read-only).
    pub fn parse(text: &str) -> Result<Self, HistoryError> {
        let h: History =
            serde_json::from_str(text).map_err(|e| HistoryError::Broken(e.to_string()))?;
        if h.format != FORMAT {
            return Err(HistoryError::Broken(format!(
                "format {:?}, not {FORMAT:?}",
                h.format
            )));
        }
        if h.version > VERSION {
            return Err(HistoryError::TooNew(h.version, Box::new(h)));
        }
        let mut h = h;
        // Version 1 is the first; later versions migrate here.
        h.version = VERSION;
        h.rides.sort_by_key(|r| r.start_unix_s);
        Ok(h)
    }

    /// The text to save (pretty JSON).
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("a history serializes")
    }

    /// Adds a ride (replacing one with the same id), keeping the order.
    pub fn add(&mut self, mut ride: RideEntry) {
        // A ride added again keeps its Strava mark.
        ride.strava_marked |= self
            .rides
            .iter()
            .any(|r| r.id == ride.id && r.strava_marked);
        self.rides.retain(|r| r.id != ride.id);
        let at = self
            .rides
            .partition_point(|r| r.start_unix_s <= ride.start_unix_s);
        self.rides.insert(at, ride);
    }

    pub fn contains(&self, id: &str) -> bool {
        self.rides.iter().any(|r| r.id == id)
    }

    /// The rides, newest first (the menu's order), with their index.
    pub fn newest_first(&self) -> impl Iterator<Item = (usize, &RideEntry)> {
        self.rides.iter().enumerate().rev()
    }

    /// The `n`-th newest ride (0 = the newest).
    pub fn nth_newest(&self, n: usize) -> Option<&RideEntry> {
        self.rides.len().checked_sub(n + 1).map(|i| &self.rides[i])
    }

    /// Removes the ride with id `id` (spec 14 Phase 7, 7A: deleting a
    /// ride); the ride removed, if there was one.
    pub fn remove(&mut self, id: &str) -> Option<RideEntry> {
        let i = self.rides.iter().position(|r| r.id == id)?;
        Some(self.rides.remove(i))
    }

    /// The ride with id `id`, to change it (the Strava mark).
    pub fn ride_mut(&mut self, id: &str) -> Option<&mut RideEntry> {
        self.rides.iter_mut().find(|r| r.id == id)
    }

    /// Weekly (Monday to Sunday) or monthly totals, newest first; only
    /// periods with rides. `offset_s`: the local UTC offset.
    pub fn totals(&self, period: Period, offset_s: i32) -> Vec<Total> {
        let mut out: Vec<Total> = Vec::new();
        for r in self.rides.iter().rev() {
            let (first, label) = period_of(local_day(r.start_unix_s, offset_s), period);
            if out.last().is_none_or(|t| t.first_day != first) {
                out.push(Total {
                    label,
                    first_day: first,
                    ..Total::default()
                });
            }
            let t = out.last_mut().expect("pushed");
            t.rides += 1;
            t.distance_m += r.distance_m;
            t.time_s += r.elapsed_s;
            t.ascent_m += r.ascent_m;
            t.work_kj += r.work_kj;
            t.hearts += r.hearts.unwrap_or(0);
        }
        out
    }

    /// The totals of the period `now_unix` is in (no rides: zeros).
    pub fn current(&self, period: Period, now_unix: i64, offset_s: i32) -> Total {
        let (first, label) = period_of(local_day(now_unix, offset_s), period);
        self.totals(period, offset_s)
            .into_iter()
            .find(|t| t.first_day == first)
            .unwrap_or(Total {
                label,
                first_day: first,
                ..Total::default()
            })
    }

    /// The personal bests over all rides (on a tie the earlier ride).
    pub fn bests(&self) -> Bests {
        let mut b = Bests::default();
        fn better<T: Clone>(
            slot: &mut Option<Best<T>>,
            v: T,
            key: f64,
            cur: impl Fn(&T) -> f64,
            ride: usize,
            at: i64,
        ) {
            if slot.as_ref().is_none_or(|s| key > cur(&s.value)) {
                *slot = Some(Best {
                    value: v,
                    ride,
                    start_unix_s: at,
                });
            }
        }
        let mut curve: Vec<Option<Best<f64>>> = vec![None; CURVE_S.len()];
        for (i, r) in self.rides.iter().enumerate() {
            for (k, &s) in CURVE_S.iter().enumerate() {
                if let Some(w) = r.curve_at(s) {
                    better(&mut curve[k], w, w, |x| *x, i, r.start_unix_s);
                }
            }
            for c in &r.climbs {
                better(
                    &mut b.biggest_climb,
                    *c,
                    c.gain_m,
                    |x| x.gain_m,
                    i,
                    r.start_unix_s,
                );
                better(
                    &mut b.fastest_climb,
                    *c,
                    c.vam(),
                    Climb::vam,
                    i,
                    r.start_unix_s,
                );
            }
            if r.distance_m > 0.0 {
                better(
                    &mut b.longest_ride,
                    r.distance_m,
                    r.distance_m,
                    |x| *x,
                    i,
                    r.start_unix_s,
                );
            }
            if r.ascent_m > 0.0 {
                better(
                    &mut b.most_climbing,
                    r.ascent_m,
                    r.ascent_m,
                    |x| *x,
                    i,
                    r.start_unix_s,
                );
            }
        }
        b.curve = CURVE_S
            .iter()
            .zip(curve)
            .filter_map(|(&s, best)| best.map(|x| (s, x)))
            .collect();
        b
    }
}

/// A file name for a rider's history: the name with everything but
/// letters, digits, `-` and `_` replaced (`Anna B.` → `anna-b-.json`);
/// a name with nothing left is `rider`.
pub fn file_name(rider: &str) -> String {
    let slug: String = rider
        .trim()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c.to_lowercase().next().unwrap_or(c)
            } else {
                '-'
            }
        })
        .collect();
    let slug = if slug.chars().all(|c| c == '-') {
        "rider".to_string()
    } else {
        slug
    };
    format!("{slug}.json")
}

/// "1:22:05" or "22:05".
pub fn hms(s: f64) -> String {
    let s = s.max(0.0).round() as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// "5 s", "1 min", "20 min", "60 min".
pub fn duration_label(s: u32) -> String {
    if s < 60 {
        format!("{s} s")
    } else {
        format!("{} min", s / 60)
    }
}

/// Distance as "42.1 km" (or "26.2 mi").
pub fn distance_label(m: f64, imperial: bool) -> String {
    if imperial {
        format!("{:.1} mi", m / 1609.344)
    } else {
        format!("{:.1} km", m / 1000.0)
    }
}

/// Height as "512 m" (or "1680 ft").
pub fn height_label(m: f64, imperial: bool) -> String {
    if imperial {
        format!("{:.0} ft", m / 0.3048)
    } else {
        format!("{m:.0} m")
    }
}

/// "3 rides, 82.4 km, 3:10:00, 1240 m" (or "no rides").
pub fn total_summary(t: &Total, imperial: bool) -> String {
    match t.rides {
        0 => "no rides".into(),
        n => format!(
            "{n} ride{}, {}, {}, {}",
            if n == 1 { "" } else { "s" },
            distance_label(t.distance_m, imperial),
            hms(t.time_s),
            height_label(t.ascent_m, imperial)
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ride of `n` seconds at `watts(t)`, 8 m/s, altitude `alt(t)`.
    pub(crate) fn activity(
        start: i64,
        n: usize,
        watts: impl Fn(usize) -> f64,
        alt: impl Fn(usize) -> f64,
    ) -> Activity {
        Activity {
            start_unix_s: start,
            records: (0..n)
                .map(|i| Record {
                    elapsed_s: i as f64,
                    distance_m: 8.0 * i as f64,
                    speed_mps: 8.0,
                    power_w: Some(watts(i)),
                    cadence_rpm: Some(90.0),
                    heart_rate_bpm: Some(140.0),
                    altitude_m: Some(alt(i)),
                    position: None,
                })
                .collect(),
            product_name: "Open Cycling World".into(),
            software_version: 1,
            laps: Vec::new(),
            markers: Vec::new(),
            devices: Vec::new(),
        }
    }

    fn ctx(id: &str) -> RideContext {
        RideContext {
            id: id.into(),
            world: "Test".into(),
            route: "Loop".into(),
            ..RideContext::default()
        }
    }

    #[test]
    fn best_mean_power_over_sliding_windows() {
        let w = [100.0, 300.0, 200.0, 400.0, 100.0];
        assert_eq!(best_mean(&w, 1), Some(400.0));
        assert_eq!(best_mean(&w, 2), Some(300.0));
        assert_eq!(best_mean(&w, 5), Some(220.0));
        assert_eq!(best_mean(&w, 6), None);
        assert_eq!(best_mean(&w, 0), None);
    }

    #[test]
    fn the_power_curve_has_the_durations_the_ride_is_long_enough_for() {
        // 10 min: 200 W with a 30 s sprint at 600 W from minute 5.
        let a = activity(
            0,
            601,
            |i| {
                if (300..330).contains(&i) {
                    600.0
                } else {
                    200.0
                }
            },
            |_| 0.0,
        );
        let watts = per_second(&a.records);
        assert_eq!(watts.len(), 600);
        let curve = power_curve(&watts);
        let s: Vec<u32> = curve.iter().map(|p| p.s).collect();
        assert_eq!(s, [5, 10, 30, 60, 120, 300, 600]);
        let at = |d| curve.iter().find(|p| p.s == d).unwrap().w;
        assert_eq!(at(5), 600.0);
        assert_eq!(at(30), 600.0);
        assert_eq!(at(60), 400.0);
        assert_eq!(at(600), 220.0);
        // The curve never rises with the duration.
        assert!(curve.windows(2).all(|p| p[0].w >= p[1].w));
    }

    #[test]
    fn gaps_and_missing_power_count_as_zero() {
        let mut a = activity(0, 4, |_| 100.0, |_| 0.0);
        a.records[2].power_w = None;
        a.records[3].elapsed_s = 5.0; // a 3 s gap
        assert_eq!(per_second(&a.records), [100.0, 0.0, 100.0, 100.0, 100.0]);
    }

    #[test]
    fn normalized_power_weights_hard_efforts() {
        // Steady: NP equals the average.
        let steady = vec![250.0; 600];
        assert!((normalized_power(&steady).unwrap() - 250.0).abs() < 1e-9);
        // Alternating minutes of 100 and 400 W (avg 250): NP above it.
        let blocks: Vec<f64> = (0..1200)
            .map(|i| if i / 60 % 2 == 0 { 100.0 } else { 400.0 })
            .collect();
        let np = normalized_power(&blocks).unwrap();
        assert!(np > 290.0 && np < 330.0, "{np}");
        assert_eq!(normalized_power(&[300.0; 29]), None);
    }

    #[test]
    fn climbs_are_found_by_gain_length_and_gradient() {
        // Flat 1 km, 2 km climbing 6 % (120 m), 1 km down 50 m, a small
        // bump of 20 m, then a climb at the end.
        let alt = |i: usize| {
            let d = 8.0 * i as f64;
            match d {
                d if d < 1000.0 => 100.0,
                d if d < 3000.0 => 100.0 + (d - 1000.0) * 0.06,
                d if d < 4000.0 => 220.0 - (d - 3000.0) * 0.05,
                d if d < 4400.0 => 170.0 + (d - 4000.0) * 0.05,
                d if d < 4800.0 => 190.0 - (d - 4400.0) * 0.05,
                d => 170.0 + (d - 4800.0) * 0.08,
            }
        };
        let a = activity(0, 700, |_| 250.0, alt);
        let c = climbs(&a.records);
        assert_eq!(c.len(), 2, "{c:?}");
        assert!((c[0].start_m - 1000.0).abs() <= 8.0);
        assert!((c[0].gain_m - 120.0).abs() < 1.0);
        assert!((c[0].length_m - 2000.0).abs() <= 16.0);
        assert!((c[0].grade() - 0.06).abs() < 0.002);
        assert!((c[0].time_s - 250.0).abs() <= 2.0);
        assert!((c[0].vam() - 1728.0).abs() < 20.0);
        assert_eq!(c[0].avg_power_w, Some(250.0));
        // The climb at the end is closed with the ride.
        assert!(c[1].start_m > 4790.0 && c[1].gain_m > 30.0);
        // Flat rides have none.
        assert!(climbs(&activity(0, 300, |_| 200.0, |_| 50.0).records).is_empty());
    }

    #[test]
    fn an_entry_sums_up_the_ride() {
        let mut a = activity(
            1_762_862_400,
            3601,
            |i| if i % 2 == 0 { 300.0 } else { 100.0 },
            |i| 400.0 + i as f64 * 0.3,
        );
        a.markers.push(ocw_fit::Marker {
            elapsed_s: 100.0,
            kind: MarkerKind::Crash,
        });
        a.records[50].speed_mps = 0.0;
        let e = RideEntry::from_activity(
            &a,
            RideContext {
                hearts: Some(42),
                damage: Some(EndDamage {
                    rider: 0.9,
                    bike: 0.8,
                }),
                workout: Some("Sweet spot".into()),
                fit: Some("2025-11-11_12-00-00.fit".into()),
                ..ctx("2025-11-11_12-00-00")
            },
        );
        assert_eq!(e.distance_m, 28_800.0);
        assert_eq!(e.elapsed_s, 3600.0);
        assert_eq!(e.moving_s, 3599.0);
        assert!((e.ascent_m - 1080.0).abs() < 0.2);
        assert_eq!(e.avg_power_w, Some(200.0));
        assert_eq!(e.max_power_w, Some(300.0));
        assert_eq!(
            e.np_w,
            Some(200.0),
            "30 s means of 300/100 alternating are 200"
        );
        assert_eq!(e.avg_hr_bpm, Some(140.0));
        assert!((e.work_kj - 720.0).abs() < 0.2);
        assert_eq!(e.crashes, 1);
        assert_eq!(e.hearts, Some(42));
        assert_eq!(e.power_curve.len(), CURVE_S.len());
        assert_eq!(e.curve_at(3600), Some(200.0));
        assert_eq!(e.curve_at(5), Some(220.0));
        assert_eq!(e.climbs.len(), 1);
        // Without power: no curve, no NP.
        let mut b = activity(0, 100, |_| 0.0, |_| 0.0);
        b.records.iter_mut().for_each(|r| r.power_w = None);
        let e = RideEntry::from_activity(&b, ctx("x"));
        assert!(e.power_curve.is_empty() && e.np_w.is_none() && e.avg_power_w.is_none());
    }

    fn history() -> History {
        let mut h = History::new("Anna");
        // Mon 2026-11-09 08:00 UTC, Sat 2026-11-14 09:00, Sun 2026-11-01
        // 09:00; each 20 min climbing 5 % (480 m).
        let days = [
            (1_794_211_200, 200.0),
            (1_794_646_800, 280.0),
            (1_793_523_600, 240.0),
        ];
        for (n, (start, w)) in days.into_iter().enumerate() {
            let a = activity(start, 1201, move |_| w, |i| 100.0 + (i as f64) * 0.4);
            let mut c = ctx(&format!("ride-{n}"));
            c.hearts = Some(10 * (n as u32 + 1));
            h.add(RideEntry::from_activity(&a, c));
        }
        h
    }

    #[test]
    fn rides_are_kept_in_order_and_replaced_by_id() {
        let mut h = history();
        let ids: Vec<&str> = h.rides.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["ride-2", "ride-0", "ride-1"], "oldest first");
        assert_eq!(h.nth_newest(0).unwrap().id, "ride-1");
        assert_eq!(h.nth_newest(2).unwrap().id, "ride-2");
        assert!(h.nth_newest(3).is_none());
        let mut again = h.rides[0].clone();
        again.distance_m = 1.0;
        h.add(again);
        assert_eq!(h.rides.len(), 3);
        assert_eq!(h.rides[0].distance_m, 1.0);
        assert!(h.contains("ride-1") && !h.contains("nope"));
    }

    /// A ride's laps (spec 14 Phase 7, 7A): from the recording's lap
    /// marks, adding up to the ride; none for a ride of one lap.
    #[test]
    fn an_entry_keeps_its_laps() {
        let mut a = activity(
            1_794_211_200,
            1201,
            |i| if i < 600 { 200.0 } else { 300.0 },
            |i| 100.0 + if i < 600 { 0.0 } else { (i - 600) as f64 * 0.1 },
        );
        assert!(RideEntry::from_activity(&a, ctx("one")).laps.is_empty());
        a.laps = vec![600.0, 900.0];
        let e = RideEntry::from_activity(&a, ctx("three"));
        assert_eq!(e.laps.len(), 3);
        assert_eq!((e.laps[0].start_s, e.laps[0].start_m), (0.0, 0.0));
        assert_eq!((e.laps[1].start_s, e.laps[1].start_m), (600.0, 4800.0));
        assert_eq!(e.laps[0].elapsed_s, 600.0);
        assert_eq!(e.laps[1].distance_m, 2400.0);
        let total: f64 = e.laps.iter().map(|l| l.distance_m).sum();
        assert_eq!(total, e.distance_m);
        assert!((e.laps[0].avg_power_w.unwrap() - 200.0).abs() < 0.5);
        assert!(e.laps[2].avg_power_w.unwrap() > 299.0);
        assert!(e.laps[0].ascent_m < 0.1 && e.laps[2].ascent_m > 29.0);
        assert_eq!(e.laps[1].speed_mps(), 8.0);
        // Kept in the file; a ride without laps writes none.
        let mut h = History::new("Ada Rider");
        h.add(e.clone());
        let back = History::parse(&h.to_json()).unwrap();
        assert_eq!(back.rides[0].laps, e.laps);
        let mut one = History::new("Ada Rider");
        a.laps.clear();
        one.add(RideEntry::from_activity(&a, ctx("one")));
        assert!(!one.to_json().contains("\"laps\""));
    }

    /// The ride's track for its map: the virtual GPS positions, thinned
    /// and rounded; none off real roads.
    #[test]
    fn an_entry_keeps_its_thinned_track() {
        let mut a = activity(1_794_211_200, 1201, |_| 200.0, |_| 100.0);
        assert!(RideEntry::from_activity(&a, ctx("x")).track.is_empty());
        for (i, r) in a.records.iter_mut().enumerate() {
            let t = i as f64 / 1200.0 * std::f64::consts::TAU;
            r.position = Some((47.2 + 0.01 * t.sin(), 9.5 + 0.015 * t.cos()));
        }
        let e = RideEntry::from_activity(&a, ctx("x"));
        assert!(
            e.track.len() >= 10 && e.track.len() <= ocw_map::ride::TRACK_MAX_POINTS,
            "{}",
            e.track.len()
        );
        assert!((e.track[0][0] - 47.2).abs() < 1e-5 && (e.track[0][1] - 9.515).abs() < 1e-5);
        let mut h = History::new("Ada Rider");
        h.add(e.clone());
        assert_eq!(
            History::parse(&h.to_json()).unwrap().rides[0].track,
            e.track
        );
    }

    /// Deleting a ride (spec 14 Phase 7, 7A): by id, the others stay in
    /// order.
    #[test]
    fn a_ride_is_removed_by_id() {
        let mut h = history();
        let gone = h.remove("ride-0").expect("there");
        assert_eq!(gone.id, "ride-0");
        let ids: Vec<&str> = h.rides.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["ride-2", "ride-1"]);
        assert!(h.remove("ride-0").is_none());
        assert_eq!(h.rides.len(), 2);
    }

    #[test]
    fn weekly_and_monthly_totals() {
        let h = history();
        let weeks = h.totals(Period::Week, 0);
        assert_eq!(weeks.len(), 2);
        assert_eq!(weeks[0].label, "Week of 9 Nov 2026");
        assert_eq!(weeks[0].rides, 2);
        assert_eq!(weeks[0].hearts, 30);
        assert!((weeks[0].distance_m - 2.0 * 9600.0).abs() < 1.0);
        assert_eq!(
            weeks[1].label, "Week of 26 Oct 2026",
            "Sun 1 Nov is in that week"
        );
        let months = h.totals(Period::Month, 0);
        assert_eq!(months.len(), 1);
        assert_eq!(months[0].label, "November 2026");
        assert_eq!(months[0].rides, 3);
        assert!((months[0].time_s - 3600.0).abs() < 1e-9);
        // The local offset moves a ride across midnight: Sun 1 Nov 09:00
        // UTC is Sat 31 Oct 23:00 at UTC-10.
        let months = h.totals(Period::Month, -10 * 3600);
        assert_eq!(
            months.iter().map(|m| m.label.as_str()).collect::<Vec<_>>(),
            ["November 2026", "October 2026"]
        );
        // This week (Sat 14 Nov) and a week without rides.
        let now = 1_794_646_800 + 3600;
        assert_eq!(h.current(Period::Week, now, 0).rides, 2);
        let later = h.current(Period::Week, now + 14 * 86_400, 0);
        assert_eq!(
            (later.rides, later.label.as_str()),
            (0, "Week of 23 Nov 2026")
        );
        assert_eq!(total_summary(&later, false), "no rides");
        assert_eq!(
            total_summary(&weeks[0], false),
            "2 rides, 19.2 km, 40:00, 960 m"
        );
    }

    #[test]
    fn personal_bests_over_all_rides() {
        let h = history();
        let b = h.bests();
        let p20 = b.power(1200).unwrap();
        assert_eq!(p20.value, 280.0);
        assert_eq!(h.rides[p20.ride].id, "ride-1");
        assert_eq!(b.power(5).unwrap().value, 280.0);
        assert!(b.power(1800).is_none(), "no ride that long");
        let climb = b.biggest_climb.unwrap();
        assert!((climb.value.gain_m - 480.0).abs() < 0.5);
        assert_eq!(climb.ride, 0, "ties keep the earlier ride");
        assert!(b.longest_ride.is_some() && b.most_climbing.is_some());
        assert_eq!(History::new("x").bests(), Bests::default());
    }

    #[test]
    fn the_format_round_trips_and_is_versioned() {
        let h = history();
        let text = h.to_json();
        assert!(text.contains("\"format\": \"ocw-history\""));
        assert!(text.contains("\"version\": 1"));
        assert_eq!(History::parse(&text).unwrap(), h);
        // Unknown fields (a newer app's) are ignored; missing optional
        // ones take their defaults.
        let minimal = r#"{"format":"ocw-history","version":1,"rider":"A","rides":[
            {"id":"r","start_unix_s":5,"distance_m":10.0,"elapsed_s":3.0,"new_field":[1,2]}]}"#;
        let m = History::parse(minimal).unwrap();
        assert_eq!(m.rides[0].hearts, None);
        assert!(m.rides[0].power_curve.is_empty());
        assert!(!m.rides[0].strava_marked);
        // The Strava mark (spec §2.8.1 *Simple upload*) is written only
        // when set, read back, and kept when the ride is added again.
        assert!(!text.contains("strava_marked"));
        let mut marked = h.clone();
        let id = marked.rides[1].id.clone();
        marked.ride_mut(&id).unwrap().strava_marked = true;
        let t2 = marked.to_json();
        assert_eq!(t2.matches("\"strava_marked\": true").count(), 1);
        let back = History::parse(&t2).unwrap();
        assert!(back.rides[1].strava_marked && !back.rides[0].strava_marked);
        let mut again = back.clone();
        let mut same = back.rides[1].clone();
        same.strava_marked = false;
        again.add(same);
        assert!(again.ride_mut(&id).unwrap().strava_marked);
        // A newer version is read-only.
        let newer = text.replace("\"version\": 1", "\"version\": 2");
        match History::parse(&newer) {
            Err(HistoryError::TooNew(2, h2)) => assert_eq!(h2.rides.len(), 3),
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            History::parse("not json"),
            Err(HistoryError::Broken(_))
        ));
        assert!(matches!(
            History::parse(r#"{"format":"other","version":1}"#),
            Err(HistoryError::Broken(_))
        ));
    }

    #[test]
    fn labels_and_file_names() {
        assert_eq!(hms(4925.0), "1:22:05");
        assert_eq!(hms(65.0), "1:05");
        assert_eq!(duration_label(5), "5 s");
        assert_eq!(duration_label(1200), "20 min");
        assert_eq!(distance_label(42_140.0, false), "42.1 km");
        assert_eq!(distance_label(42_195.0, true), "26.2 mi");
        assert_eq!(height_label(512.0, true), "1680 ft");
        assert_eq!(start_label(1_794_646_800, 3600), "Sat 14 Nov 2026 10:00");
        assert_eq!(file_name("Anna B."), "anna-b-.json");
        assert_eq!(file_name("  "), "rider.json");
        assert_eq!(file_name("Ölf_2"), "ölf_2.json");
    }
}
