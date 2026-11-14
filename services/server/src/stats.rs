//! Tick timing and session statistics of the headless server, written as
//! JSON by `ocw-server --stats-file` (read by the multiplayer test's
//! verdict, `ocw-bot verdict`, spec §11.4).

use std::time::Duration;

use ocw_core::SIM_DT;
use serde::{Deserialize, Serialize};

/// Timing of the game loop's ticks.
#[derive(Debug, Clone, Default)]
pub struct TickStats {
    ticks: u64,
    busy: Duration,
    worst: Duration,
    over_budget: u64,
    late: u64,
    worst_late: Duration,
    peak_riders: usize,
}

impl TickStats {
    /// One tick: how long its work took (`took`), how late it started
    /// against the fixed 50 Hz schedule (`late`), and the riders in the
    /// session after it.
    pub fn record(&mut self, took: Duration, late: Duration, riders: usize) {
        self.ticks += 1;
        self.busy += took;
        self.worst = self.worst.max(took);
        let budget = Duration::from_secs_f64(SIM_DT);
        if took > budget {
            self.over_budget += 1;
        }
        // A tick that starts a whole tick late means the loop fell behind
        // (it then catches up without sleeping).
        if late > budget {
            self.late += 1;
        }
        self.worst_late = self.worst_late.max(late);
        self.peak_riders = self.peak_riders.max(riders);
    }

    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    /// Mean work per tick in microseconds.
    pub fn mean_tick_us(&self) -> f64 {
        self.busy.as_secs_f64() * 1e6 / self.ticks.max(1) as f64
    }

    pub fn max_tick_us(&self) -> f64 {
        self.worst.as_secs_f64() * 1e6
    }

    /// Ticks whose work took longer than the tick (20 ms).
    pub fn over_budget(&self) -> u64 {
        self.over_budget
    }

    /// Ticks that started more than one tick behind the schedule.
    pub fn late_ticks(&self) -> u64 {
        self.late
    }

    pub fn max_late_ms(&self) -> f64 {
        self.worst_late.as_secs_f64() * 1e3
    }

    pub fn peak_riders(&self) -> usize {
        self.peak_riders
    }
}

/// What `ocw-server --stats-file` writes at the end of a session.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ServerReport {
    /// Simulated seconds and ticks run.
    pub sim_time_s: f64,
    pub ticks: u64,
    /// Whether the server ran in real time (not `--fast`).
    pub real_time: bool,
    pub mean_tick_us: f64,
    pub max_tick_us: f64,
    pub budget_us: f64,
    pub over_budget_ticks: u64,
    pub late_ticks: u64,
    pub max_late_ms: f64,
    /// Most riders in the session at once (server bots included).
    pub peak_riders: usize,
    /// Server-side bots.
    pub bots: u32,
    /// Display names of the clients that joined, and of those that left
    /// before the end (disconnects; at the end everyone is still there).
    pub joined: Vec<String>,
    pub left: Vec<String>,
    /// Riders still in the session at the end, server bots included.
    pub riders_at_end: usize,
    /// The course: route id (empty for a ride on a road graph) and the
    /// SHA-256 of the route file (`--route-file`; empty otherwise).
    pub route_id: String,
    pub route_sha256: String,
    /// The WebTransport endpoint (spec §14 Phase 10 B3, `--webtransport`):
    /// sessions that came up, messages received, of those datagrams.
    #[serde(default)]
    pub web_sessions: u64,
    #[serde(default)]
    pub web_messages_in: u64,
    #[serde(default)]
    pub web_datagrams_in: u64,
}

impl ServerReport {
    /// Fills the timing fields from `stats`.
    pub fn with_timing(mut self, stats: &TickStats, real_time: bool) -> Self {
        self.ticks = stats.ticks();
        self.real_time = real_time;
        self.mean_tick_us = stats.mean_tick_us();
        self.max_tick_us = stats.max_tick_us();
        self.budget_us = SIM_DT * 1e6;
        self.over_budget_ticks = stats.over_budget();
        self.late_ticks = stats.late_ticks();
        self.max_late_ms = stats.max_late_ms();
        self.peak_riders = stats.peak_riders();
        self
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("plain data serializes")
    }

    pub fn from_json(text: &str) -> Result<Self, String> {
        serde_json::from_str(text).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn counts_slow_and_late_ticks() {
        let mut s = TickStats::default();
        s.record(MS, Duration::ZERO, 3);
        s.record(3 * MS, MS, 5);
        // Work longer than a tick, and a start a whole tick late.
        s.record(25 * MS, 30 * MS, 4);
        assert_eq!(s.ticks(), 3);
        assert!((s.mean_tick_us() - 29_000.0 / 3.0).abs() < 1.0);
        assert!((s.max_tick_us() - 25_000.0).abs() < 1.0);
        assert_eq!((s.over_budget(), s.late_ticks()), (1, 1));
        assert!((s.max_late_ms() - 30.0).abs() < 1e-9);
        assert_eq!(s.peak_riders(), 5);
    }

    #[test]
    fn an_empty_session_has_no_division_by_zero() {
        let s = TickStats::default();
        assert_eq!(s.mean_tick_us(), 0.0);
    }

    #[test]
    fn the_report_round_trips_as_json() {
        let mut s = TickStats::default();
        s.record(2 * MS, Duration::ZERO, 2);
        let r = ServerReport {
            sim_time_s: 12.5,
            joined: vec!["linux-1".into(), "Kiosk".into()],
            left: vec!["linux-1".into()],
            riders_at_end: 1,
            route_id: "test-loop".into(),
            ..Default::default()
        }
        .with_timing(&s, true);
        assert_eq!(r.ticks, 1);
        assert!((r.budget_us - 20_000.0).abs() < 1e-6);
        let back = ServerReport::from_json(&r.to_json()).unwrap();
        assert_eq!(back, r);
        assert!(ServerReport::from_json("{").is_err());
    }
}
