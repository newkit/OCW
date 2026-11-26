//! The public statistics of a hosted server (spec 11.5 *Status page*):
//! what `https://APP.fly.dev/stats.json` and the status page at `/` show –
//! riders online now (people and bots apart), today's peak, rides started
//! and kilometres ridden today and since the counting began, the 50 Hz
//! tick's load, the riders in voice, the world and the server's version
//! and uptime.
//!
//! **Counts only.** Nothing here takes a rider's name, account, address or
//! position: the game loop hands over numbers ([`Observation`]), so the
//! JSON cannot leak them. "Today" is the UTC day. The totals may be kept
//! across restarts in a small file ([`Saved`], `ocw-server
//! --public-stats-file`); without it they start again at every start.
//!
//! Pure (times are passed in), so it is tested without a clock.

use std::time::Duration;

use ocw_core::{SIM_DT, SIM_TICK_HZ};
use serde::{Deserialize, Serialize};

/// Speeds above this (m/s, 108 km/h) are counted as this much: a glitch
/// or a teleport must not add hundreds of kilometres.
pub const MAX_COUNTED_MPS: f64 = 30.0;

/// The tick load is the mean and 95th percentile over this window.
pub const TICK_WINDOW_S: u32 = 60;

/// What the game loop counts in one tick (no names, no positions).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Observation {
    /// Riders on the road now: people, and bots (server bots, the Echo
    /// rider, `ocw-bot` clients).
    pub humans: u32,
    pub bots: u32,
    /// The sum of their speeds (m/s), each capped at [`MAX_COUNTED_MPS`].
    pub humans_mps: f64,
    pub bots_mps: f64,
    /// Rides started since the game server started (a monotonic counter:
    /// a new connection's first hello; a world switch is no new ride).
    pub rides_humans: u64,
    pub rides_bots: u64,
    /// Riders who say they are in the voice room now.
    pub voice: u32,
}

impl Observation {
    /// Adds one rider with its speed (m/s).
    pub fn add(&mut self, bot: bool, speed_mps: f64) {
        let v = if speed_mps.is_finite() {
            speed_mps.abs().min(MAX_COUNTED_MPS)
        } else {
            0.0
        };
        if bot {
            self.bots += 1;
            self.bots_mps += v;
        } else {
            self.humans += 1;
            self.humans_mps += v;
        }
    }
}

/// Rides and kilometres, people and bots apart.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Totals {
    pub rides_humans: u64,
    pub rides_bots: u64,
    pub km_humans: f64,
    pub km_bots: f64,
}

/// One UTC day: its date, the most riders at once, and its totals.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Day {
    /// `YYYY-MM-DD` (UTC).
    pub date: String,
    pub peak_humans: u32,
    pub peak_bots: u32,
    #[serde(flatten)]
    pub totals: Totals,
}

/// What is kept across restarts (`--public-stats-file`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Saved {
    /// When the counting began (Unix s).
    pub since_unix_s: i64,
    pub total: Totals,
    pub today: Day,
}

/// The world of the session, for the page.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct WorldNote {
    /// The package's manifest id (`liechtenstein`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The region's name for people (`Liechtenstein`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<u64>,
    /// The route ridden (none: a free ride on all the world's roads).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route: Option<String>,
}

/// Riders now.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct Now {
    pub humans: u32,
    pub bots: u32,
    /// In the voice room.
    pub voice: u32,
}

/// The tick's load over the last [`TICK_WINDOW_S`] seconds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct TickLoad {
    pub hz: u32,
    pub budget_ms: f64,
    pub window_s: u32,
    pub mean_ms: f64,
    pub p95_ms: f64,
    pub max_ms: f64,
}

/// The body of `/stats.json`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StatsReport {
    pub server: &'static str,
    pub version: String,
    pub now_unix_s: i64,
    pub started_unix_s: i64,
    pub uptime_s: i64,
    /// When the totals began (the first start with the stats file; else
    /// this start).
    pub since_unix_s: i64,
    pub world: WorldNote,
    pub riders: Now,
    pub today: Day,
    pub total: Totals,
    pub tick: TickLoad,
}

/// The counters.
#[derive(Debug, Clone)]
pub struct PublicStats {
    started_unix_s: i64,
    saved: Saved,
    /// The ride counters at the last observation.
    last_rides: (u64, u64),
    now: Now,
    /// Tick work in µs, a ring over the window.
    ticks: Vec<u32>,
    next_tick: usize,
}

impl PublicStats {
    /// Counting from `started_unix_s`, with nothing before.
    pub fn new(started_unix_s: i64) -> Self {
        Self::resume(
            started_unix_s,
            Saved {
                since_unix_s: started_unix_s,
                ..Default::default()
            },
        )
    }

    /// Counting on from what was saved (today's numbers only if it is
    /// still the same day: [`Self::observe`] starts a new day otherwise).
    pub fn resume(started_unix_s: i64, saved: Saved) -> Self {
        let mut saved = saved;
        if saved.since_unix_s <= 0 || saved.since_unix_s > started_unix_s {
            saved.since_unix_s = started_unix_s;
        }
        let window = (TICK_WINDOW_S * SIM_TICK_HZ) as usize;
        Self {
            started_unix_s,
            saved,
            last_rides: (0, 0),
            now: Now::default(),
            ticks: Vec::with_capacity(window),
            next_tick: 0,
        }
    }

    /// One tick of the game loop at `now_unix_s`, `dt_s` simulated seconds
    /// long (the fixed tick, [`SIM_DT`]).
    pub fn observe(&mut self, now_unix_s: i64, dt_s: f64, o: &Observation) {
        let date = utc_date(now_unix_s);
        if self.saved.today.date != date {
            self.saved.today = Day {
                date,
                ..Default::default()
            };
        }
        // Rides: what the counter rose by (a smaller counter – a new game
        // server – counts from 0).
        let rose = |now: u64, last: u64| if now >= last { now - last } else { now };
        let rides = (
            rose(o.rides_humans, self.last_rides.0),
            rose(o.rides_bots, self.last_rides.1),
        );
        self.last_rides = (o.rides_humans, o.rides_bots);
        let km = (o.humans_mps * dt_s / 1000.0, o.bots_mps * dt_s / 1000.0);
        for t in [&mut self.saved.today.totals, &mut self.saved.total] {
            t.rides_humans += rides.0;
            t.rides_bots += rides.1;
            t.km_humans += km.0;
            t.km_bots += km.1;
        }
        let day = &mut self.saved.today;
        day.peak_humans = day.peak_humans.max(o.humans);
        day.peak_bots = day.peak_bots.max(o.bots);
        self.now = Now {
            humans: o.humans,
            bots: o.bots,
            voice: o.voice,
        };
    }

    /// One tick's work (physics, bots, snapshots).
    pub fn record_tick(&mut self, took: Duration) {
        let us = took.as_micros().min(u128::from(u32::MAX)) as u32;
        if self.ticks.len() < self.ticks.capacity() {
            self.ticks.push(us);
        } else {
            self.ticks[self.next_tick] = us;
            self.next_tick = (self.next_tick + 1) % self.ticks.len();
        }
    }

    /// The tick load over the window.
    pub fn tick_load(&self) -> TickLoad {
        let mut t: Vec<u32> = self.ticks.clone();
        t.sort_unstable();
        let ms = |us: u32| f64::from(us) / 1000.0;
        let (mean_ms, p95_ms, max_ms) = if t.is_empty() {
            (0.0, 0.0, 0.0)
        } else {
            let sum: u64 = t.iter().map(|&u| u64::from(u)).sum();
            // The nearest-rank 95th percentile.
            let rank = ((t.len() as f64) * 0.95).ceil() as usize;
            (
                sum as f64 / t.len() as f64 / 1000.0,
                ms(t[rank.clamp(1, t.len()) - 1]),
                ms(t[t.len() - 1]),
            )
        };
        TickLoad {
            hz: SIM_TICK_HZ,
            budget_ms: SIM_DT * 1000.0,
            window_s: TICK_WINDOW_S,
            mean_ms: round3(mean_ms),
            p95_ms: round3(p95_ms),
            max_ms: round3(max_ms),
        }
    }

    /// What to keep across restarts.
    pub fn saved(&self) -> &Saved {
        &self.saved
    }

    /// The report at `now_unix_s`.
    pub fn report(&self, now_unix_s: i64, version: &str, world: &WorldNote) -> StatsReport {
        let round = |t: &Totals| Totals {
            km_humans: round3(t.km_humans),
            km_bots: round3(t.km_bots),
            ..t.clone()
        };
        let mut today = self.saved.today.clone();
        // A day without an observation yet (the first second).
        if today.date.is_empty() {
            today.date = utc_date(now_unix_s);
        }
        today.totals = round(&today.totals);
        StatsReport {
            server: "ocw-server",
            version: version.to_owned(),
            now_unix_s,
            started_unix_s: self.started_unix_s,
            uptime_s: (now_unix_s - self.started_unix_s).max(0),
            since_unix_s: self.saved.since_unix_s,
            world: world.clone(),
            riders: self.now,
            today,
            total: round(&self.saved.total),
            tick: self.tick_load(),
        }
    }
}

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

/// `YYYY-MM-DD` of a Unix time (UTC; the civil-from-days algorithm).
pub fn utc_date(unix_s: i64) -> String {
    let z = unix_s.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-11-26 12:00 UTC.
    const NOON: i64 = 1_795_694_400;

    fn obs(humans: &[f64], bots: &[f64], rides: (u64, u64)) -> Observation {
        let mut o = Observation {
            rides_humans: rides.0,
            rides_bots: rides.1,
            ..Default::default()
        };
        for &v in humans {
            o.add(false, v);
        }
        for &v in bots {
            o.add(true, v);
        }
        o
    }

    #[test]
    fn dates_are_utc_days() {
        assert_eq!(utc_date(0), "1970-01-01");
        assert_eq!(utc_date(NOON), "2026-11-26");
        assert_eq!(utc_date(NOON + 12 * 3600 - 1), "2026-11-26");
        assert_eq!(utc_date(NOON + 12 * 3600), "2026-11-27");
        assert_eq!(utc_date(951_782_400), "2000-02-29");
    }

    #[test]
    fn people_and_bots_are_counted_apart() {
        let mut s = PublicStats::new(NOON);
        // Two people at 10 m/s, one bot at 5 m/s, for 100 s.
        for _ in 0..(100 * SIM_TICK_HZ) {
            s.observe(NOON, SIM_DT, &obs(&[10.0, 10.0], &[5.0], (2, 1)));
        }
        let r = s.report(NOON + 100, "0.0.1", &WorldNote::default());
        assert_eq!((r.riders.humans, r.riders.bots), (2, 1));
        assert!((r.today.totals.km_humans - 2.0).abs() < 1e-6, "{r:?}");
        assert!((r.today.totals.km_bots - 0.5).abs() < 1e-6);
        assert_eq!(
            (r.today.totals.rides_humans, r.today.totals.rides_bots),
            (2, 1)
        );
        assert_eq!(r.total, r.today.totals);
        assert_eq!(r.uptime_s, 100);
    }

    #[test]
    fn joins_and_leaves_move_the_count_and_the_peak_stays() {
        let mut s = PublicStats::new(NOON);
        s.observe(NOON, SIM_DT, &obs(&[1.0], &[], (1, 0)));
        s.observe(NOON, SIM_DT, &obs(&[1.0, 1.0, 1.0], &[0.0], (3, 1)));
        // Two leave, one more joins later.
        s.observe(NOON, SIM_DT, &obs(&[1.0], &[0.0], (3, 1)));
        s.observe(NOON, SIM_DT, &obs(&[1.0, 1.0], &[0.0], (4, 1)));
        let r = s.report(NOON, "v", &WorldNote::default());
        assert_eq!(r.riders.humans, 2);
        assert_eq!((r.today.peak_humans, r.today.peak_bots), (3, 1));
        assert_eq!(r.today.totals.rides_humans, 4);
    }

    #[test]
    fn a_new_day_starts_from_zero_but_the_totals_go_on() {
        let mut s = PublicStats::new(NOON);
        s.observe(NOON, 1000.0, &obs(&[10.0, 10.0], &[], (2, 0)));
        let next_day = NOON + 86_400;
        s.observe(next_day, 1.0, &obs(&[0.0], &[], (3, 0)));
        let r = s.report(next_day, "v", &WorldNote::default());
        assert_eq!(r.today.date, "2026-11-27");
        assert_eq!(r.today.peak_humans, 1);
        assert_eq!(r.today.totals.rides_humans, 1);
        assert_eq!(r.today.totals.km_humans, 0.0);
        assert_eq!(r.total.rides_humans, 3);
        assert!((r.total.km_humans - 20.0).abs() < 1e-9);
    }

    #[test]
    fn glitches_count_at_most_the_cap() {
        let mut o = Observation::default();
        o.add(false, 5000.0);
        o.add(false, f64::NAN);
        o.add(true, -3.0);
        assert_eq!(o.humans_mps, MAX_COUNTED_MPS);
        assert_eq!(o.bots_mps, 3.0);
        assert_eq!((o.humans, o.bots), (2, 1));
    }

    #[test]
    fn totals_carry_over_a_restart_and_the_day_if_it_is_the_same() {
        let mut s = PublicStats::new(NOON - 3600);
        s.observe(NOON - 1, 500.0, &obs(&[10.0], &[], (1, 0)));
        let saved = s.saved().clone();
        let text = serde_json::to_string(&saved).unwrap();
        let back: Saved = serde_json::from_str(&text).unwrap();
        assert_eq!(back, saved);
        // A restart the same day: a new game server counts its rides from 0.
        let mut s = PublicStats::resume(NOON, back.clone());
        s.observe(NOON, 1.0, &obs(&[0.0], &[], (1, 0)));
        let r = s.report(NOON, "v", &WorldNote::default());
        assert_eq!(r.since_unix_s, NOON - 3600);
        assert_eq!(r.started_unix_s, NOON);
        assert_eq!(r.today.totals.rides_humans, 2);
        assert!((r.today.totals.km_humans - 5.0).abs() < 1e-9);
        // The next day: today starts again, the totals stay.
        let mut s = PublicStats::resume(NOON + 86_400, back);
        s.observe(NOON + 86_400, 1.0, &obs(&[], &[], (0, 0)));
        let r = s.report(NOON + 86_400, "v", &WorldNote::default());
        assert_eq!(r.today.totals, Totals::default());
        assert_eq!(r.total.rides_humans, 1);
    }

    #[test]
    fn the_tick_load_is_mean_and_p95_of_the_window() {
        let mut s = PublicStats::new(NOON);
        assert_eq!(s.tick_load().p95_ms, 0.0);
        // 95 ticks of 1 ms, 5 of 10 ms.
        for i in 0..100 {
            s.record_tick(Duration::from_millis(if i < 95 { 1 } else { 10 }));
        }
        let t = s.tick_load();
        assert!((t.mean_ms - 1.45).abs() < 1e-9, "{t:?}");
        assert_eq!((t.p95_ms, t.max_ms), (1.0, 10.0));
        assert_eq!((t.hz, t.budget_ms), (50, 20.0));
        // The window forgets: a full window of 2 ms ticks.
        for _ in 0..(TICK_WINDOW_S * SIM_TICK_HZ) {
            s.record_tick(Duration::from_millis(2));
        }
        let t = s.tick_load();
        assert_eq!((t.mean_ms, t.p95_ms, t.max_ms), (2.0, 2.0, 2.0));
    }

    /// The JSON has counts only: no rider names, no addresses, no
    /// positions (spec 11.5 *Status page*).
    #[test]
    fn the_report_has_no_names_addresses_or_positions() {
        let mut s = PublicStats::new(NOON);
        s.observe(NOON, SIM_DT, &obs(&[8.0], &[0.0], (1, 1)));
        let world = WorldNote {
            id: Some("liechtenstein".into()),
            name: Some("Liechtenstein".into()),
            version: Some(10),
            route: None,
        };
        let v = serde_json::to_value(s.report(NOON + 5, "0.2.6", &world)).unwrap();
        fn keys(v: &serde_json::Value, out: &mut Vec<String>) {
            if let Some(o) = v.as_object() {
                for (k, x) in o {
                    out.push(k.clone());
                    keys(x, out);
                }
            }
        }
        let mut all = Vec::new();
        keys(&v, &mut all);
        for k in &all {
            for bad in [
                "ip",
                "addr",
                "position",
                "rider_name",
                "account",
                "x",
                "z",
                "lat",
            ] {
                assert_ne!(k, bad);
            }
            assert!(!k.contains("addr") && !k.contains("pos"), "{k}");
        }
        assert_eq!(v["world"]["name"], "Liechtenstein");
        assert_eq!(v["riders"]["humans"], 1);
        assert_eq!(v["today"]["date"], "2026-11-26");
        assert_eq!(v["today"]["rides_humans"], 1);
    }
}
