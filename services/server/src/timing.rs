//! Snapshot timing at a client (spec §6.3, §14 Phase 10 B3): how evenly
//! the 10 Hz snapshots arrive (interval and jitter) and how old they are
//! on arrival. Shared by `ocw-bot` (native QUIC or WebTransport) and the
//! browser app, so both are measured the same way. Wasm-safe, no I/O.
//!
//! **Age** is the client's wall clock at arrival minus the server's real
//! instant of the snapshot's tick (`WorldClock.world_unix_ms − offset_ms`):
//! the one-way delay plus the server's tick-schedule offset. It means
//! something only when client and server read the same clock (the same
//! machine, as in the browser test); between machines it includes their
//! clock difference.

use ocw_protocol::decode_server;
use ocw_protocol::v1::server_message::Msg as S;
use serde::{Deserialize, Serialize};

/// Samples kept (about 30 min at 10 Hz).
const MAX_SAMPLES: usize = 20_000;

/// Collects arrival times of snapshots.
#[derive(Debug, Clone, Default)]
pub struct SnapshotTiming {
    last_arrival_s: Option<f64>,
    last_tick: Option<u64>,
    snapshots: u64,
    stale: u64,
    intervals_ms: Vec<f64>,
    ages_ms: Vec<f64>,
}

/// The summary (milliseconds).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TimingSummary {
    pub snapshots: u64,
    /// Older than one already received (reordered datagrams), not counted.
    pub stale: u64,
    pub interval_mean_ms: f64,
    /// Standard deviation of the intervals.
    pub jitter_ms: f64,
    pub interval_p50_ms: f64,
    pub interval_p95_ms: f64,
    pub interval_p99_ms: f64,
    pub interval_max_ms: f64,
    /// Age on arrival (see the module docs); absent without a world clock.
    pub age_mean_ms: Option<f64>,
    pub age_p50_ms: Option<f64>,
    pub age_p95_ms: Option<f64>,
    pub age_max_ms: Option<f64>,
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let i = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[i.min(sorted.len() - 1)]
}

fn sorted(v: &[f64]) -> Vec<f64> {
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    s
}

impl SnapshotTiming {
    /// One snapshot of `server_tick` arrived at `arrival_s` (the client's
    /// monotonic clock); `age_ms` if known.
    pub fn record(&mut self, arrival_s: f64, server_tick: u64, age_ms: Option<f64>) {
        if self.last_tick.is_some_and(|t| server_tick <= t) {
            self.stale += 1;
            return;
        }
        self.last_tick = Some(server_tick);
        self.snapshots += 1;
        if let Some(prev) = self.last_arrival_s
            && self.intervals_ms.len() < MAX_SAMPLES
        {
            self.intervals_ms.push((arrival_s - prev) * 1000.0);
        }
        self.last_arrival_s = Some(arrival_s);
        if let Some(a) = age_ms.filter(|a| a.is_finite())
            && self.ages_ms.len() < MAX_SAMPLES
        {
            self.ages_ms.push(a);
        }
    }

    /// An encoded `ServerMessage` received at `arrival_s`, with the
    /// client's wall clock `client_unix_ms`: records it if a snapshot.
    pub fn on_message(&mut self, bytes: &[u8], arrival_s: f64, client_unix_ms: f64) {
        if !crate::wire::server_unreliable(bytes) {
            return;
        }
        let Ok(S::Snapshot(s)) = decode_server(bytes) else {
            return;
        };
        let age = s
            .clock
            .as_ref()
            .map(|c| client_unix_ms - (c.world_unix_ms - c.offset_ms) as f64);
        self.record(arrival_s, s.server_tick, age);
    }

    pub fn snapshots(&self) -> u64 {
        self.snapshots
    }

    pub fn summary(&self) -> TimingSummary {
        let iv = sorted(&self.intervals_ms);
        let n = iv.len().max(1) as f64;
        let mean = iv.iter().sum::<f64>() / n;
        let var = iv.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n;
        let ages = sorted(&self.ages_ms);
        let have_ages = !ages.is_empty();
        let opt = |v: f64| have_ages.then_some(v);
        TimingSummary {
            snapshots: self.snapshots,
            stale: self.stale,
            interval_mean_ms: mean,
            jitter_ms: var.sqrt(),
            interval_p50_ms: percentile(&iv, 0.5),
            interval_p95_ms: percentile(&iv, 0.95),
            interval_p99_ms: percentile(&iv, 0.99),
            interval_max_ms: iv.last().copied().unwrap_or(0.0),
            age_mean_ms: opt(ages.iter().sum::<f64>() / ages.len().max(1) as f64),
            age_p50_ms: opt(percentile(&ages, 0.5)),
            age_p95_ms: opt(percentile(&ages, 0.95)),
            age_max_ms: opt(ages.last().copied().unwrap_or(0.0)),
        }
    }
}

impl TimingSummary {
    /// One line for logs and `net.stats`.
    pub fn line(&self) -> String {
        let age = match (self.age_mean_ms, self.age_p95_ms, self.age_max_ms) {
            (Some(m), Some(p), Some(x)) => {
                format!(", age mean {m:.1} / p95 {p:.1} / max {x:.1} ms")
            }
            _ => String::new(),
        };
        format!(
            "{} snapshots ({} stale), interval mean {:.1} ms, jitter {:.1} ms, p95 {:.1} / p99 {:.1} / max {:.1} ms{age}",
            self.snapshots,
            self.stale,
            self.interval_mean_ms,
            self.jitter_ms,
            self.interval_p95_ms,
            self.interval_p99_ms,
            self.interval_max_ms
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_protocol::v1;
    use ocw_protocol::{encode, server_message};

    #[test]
    fn even_snapshots_have_no_jitter_and_stale_ones_are_counted() {
        let mut t = SnapshotTiming::default();
        for i in 0..11u64 {
            t.record(i as f64 * 0.1, 5 * (i + 1), Some(2.0));
        }
        // A reordered one.
        t.record(1.05, 10, Some(9.0));
        let s = t.summary();
        assert_eq!((s.snapshots, s.stale), (11, 1));
        assert!((s.interval_mean_ms - 100.0).abs() < 1e-6, "{s:?}");
        assert!(s.jitter_ms < 1e-6);
        assert!((s.interval_max_ms - 100.0).abs() < 1e-6);
        assert_eq!(s.age_mean_ms, Some(2.0));
        assert!(s.line().contains("jitter 0.0 ms"), "{}", s.line());
    }

    #[test]
    fn uneven_snapshots_show_jitter_and_their_percentiles() {
        let mut t = SnapshotTiming::default();
        let mut at = 0.0;
        for i in 0..101u64 {
            // Alternating 80 and 120 ms; every 50th 300 ms late.
            at += if i % 50 == 49 {
                0.3
            } else if i % 2 == 0 {
                0.08
            } else {
                0.12
            };
            t.record(at, i + 1, None);
        }
        let s = t.summary();
        assert_eq!(s.snapshots, 101);
        assert!(s.jitter_ms > 20.0, "{s:?}");
        assert!((s.interval_max_ms - 300.0).abs() < 1e-6);
        assert!(
            (s.interval_p50_ms - 120.0).abs() < 1e-6 || (s.interval_p50_ms - 80.0).abs() < 1e-6
        );
        assert_eq!(s.age_mean_ms, None);
        assert!(!s.line().contains("age"));
    }

    #[test]
    fn the_age_comes_from_the_world_clock_of_the_snapshot() {
        let mut t = SnapshotTiming::default();
        // The world clock 1 h ahead of real time (time machine): real =
        // world − offset.
        let snap = |tick: u64, real_ms: i64| {
            encode(&server_message(v1::server_message::Msg::Snapshot(
                v1::Snapshot {
                    server_tick: tick,
                    clock: Some(v1::WorldClock {
                        world_unix_ms: real_ms + 3_600_000,
                        offset_ms: 3_600_000,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )))
        };
        t.on_message(&snap(5, 1_000_000), 0.0, 1_000_004.0);
        t.on_message(&snap(10, 1_000_100), 0.1, 1_000_106.0);
        // Not a snapshot: ignored.
        let welcome = encode(&server_message(v1::server_message::Msg::Welcome(
            v1::Welcome::default(),
        )));
        t.on_message(&welcome, 0.2, 0.0);
        let s = t.summary();
        assert_eq!(s.snapshots, 2);
        assert_eq!(s.age_mean_ms, Some(5.0));
        assert_eq!(s.age_max_ms, Some(6.0));
    }
}
