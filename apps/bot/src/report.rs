//! What a bot client reports (one JSON line per process, `OCW-BOT-REPORT`)
//! and the multiplayer test's verdict over the server's statistics and all
//! bot reports (spec §11.4, `ocw-bot verdict`).

use serde::{Deserialize, Serialize};

use ocw_server::stats::ServerReport;

/// The marker in front of a bot process's report on its standard output.
pub const REPORT_MARKER: &str = "OCW-BOT-REPORT ";

/// Close reasons of a normal end: the server's shutdown, or the bot's own.
pub const NORMAL_CLOSE: [&str; 2] = ["server shutting down", "bot finished"];

/// What a client sees when the server's closing message was lost: the
/// server's endpoint answers with a reset, or nothing comes any more.
pub const LOST_CLOSE: [&str; 2] = ["reset by peer", "timed out"];

/// A rider whose last snapshot is this close to the server's last tick
/// (ticks; 5 s) and whose link then broke saw the server's end.
pub const END_SLACK_TICKS: u64 = 5 * ocw_core::SIM_TICK_HZ as u64;

/// Whether a rider's link ended normally: the server's shutdown or the
/// bot's own end, or a lost closing message right at the server's end
/// (`server_ticks`: the ticks the server ran).
pub fn ended_normally(r: &RiderReport, server_ticks: u64) -> bool {
    match &r.closed {
        None => true,
        Some(c) if NORMAL_CLOSE.iter().any(|n| c.contains(n)) => true,
        Some(c) => {
            LOST_CLOSE.iter().any(|n| c.contains(n))
                && r.last_tick + END_SLACK_TICKS >= server_ticks
        }
    }
}

/// A note when the machine was suspended (sleep) during the run: the
/// monotonic clock stopped while the wall clock went on.
pub fn suspend_note(suspended_s: f64) -> Option<String> {
    (suspended_s > 0.5).then(|| {
        format!(
            "the machine was suspended for {suspended_s:.1} s during the run (sleep): \
             snapshots stop and the link may break meanwhile"
        )
    })
}

/// One bot rider's view of the session. Times are seconds since the bot
/// process started.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RiderReport {
    pub name: String,
    pub route_id: String,
    pub joined_at_s: Option<f64>,
    /// Why the server (or the bot itself, course mismatch) refused the join.
    pub rejected: Option<String>,
    /// Why and when the link ended.
    pub closed: Option<String>,
    pub closed_at_s: f64,
    pub snapshots: u64,
    pub stale_snapshots: u64,
    pub malformed: u64,
    pub last_tick: u64,
    pub first_snapshot_s: Option<f64>,
    pub last_snapshot_s: Option<f64>,
    /// Longest time without a snapshot (after the first).
    pub max_snapshot_gap_s: f64,
    /// Most riders in one snapshot (the bot itself included).
    pub max_riders_seen: u32,
    /// Display names of the other riders seen, and of those that moved.
    pub others_seen: Vec<String>,
    pub others_moving: Vec<String>,
    /// Own distance at the first and the last own snapshot (m).
    pub start_m: Option<f64>,
    pub end_m: f64,
    pub own_first_s: Option<f64>,
    pub max_speed_mps: f64,
    pub samples_sent: u64,
    /// Highest sample sequence number the server reported as applied.
    pub acked_seq: u32,
    pub samples_acked: bool,
    /// Prediction error at reconciliation (after the warm-up), metres.
    pub prediction_samples: u64,
    pub prediction_error_sum_m: f64,
    pub prediction_error_max_m: f64,
    pub rtt_ms_max: f64,
    /// It found the rider it follows (`--follow`) in the session.
    pub following: bool,
}

impl RiderReport {
    pub fn prediction_error_mean_m(&self) -> f64 {
        self.prediction_error_sum_m / self.prediction_samples.max(1) as f64
    }

    /// Metres ridden between the first and the last own snapshot.
    pub fn ridden_m(&self) -> f64 {
        self.start_m.map_or(0.0, |s| self.end_m - s)
    }
}

/// One bot process (`ocw-bot`): its riders on one machine.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BotReport {
    /// The label given with `--label` (e.g. the OS), never a host name.
    pub label: String,
    pub os: String,
    pub route_id: String,
    pub route_sha256: String,
    pub seconds: f64,
    /// Seconds the machine was suspended (wall clock minus monotonic clock).
    #[serde(default)]
    pub suspended_s: f64,
    pub riders: Vec<RiderReport>,
}

impl BotReport {
    /// The report as one line with the marker, for standard output.
    pub fn to_line(&self) -> String {
        format!(
            "{REPORT_MARKER}{}",
            serde_json::to_string(self).expect("plain data")
        )
    }

    /// Every report in a process's output (lines with the marker).
    pub fn from_output(text: &str) -> Vec<Result<Self, String>> {
        text.lines()
            .filter_map(|l| l.trim_end_matches('\r').split_once(REPORT_MARKER))
            .map(|(_, json)| serde_json::from_str(json).map_err(|e| e.to_string()))
            .collect()
    }
}

/// What the verdict expects.
#[derive(Debug, Clone, PartialEq)]
pub struct Expect {
    /// Bot processes (machines) that were started.
    pub bot_processes: usize,
    /// Riders per process.
    pub riders_per_process: u32,
    /// Display names of the GUI clients that take part (the kiosk, a
    /// headless camera client).
    pub clients: Vec<String>,
    /// Minimum metres every bot rider must ride.
    pub min_ridden_m: f64,
    /// Snapshot rate as a share of 10 Hz over the connected time.
    pub min_snapshot_share: f64,
    pub max_snapshot_gap_s: f64,
    pub max_prediction_error_mean_m: f64,
    pub max_prediction_error_m: f64,
    /// Share of ticks that may start a whole tick late / take longer than a tick.
    pub max_late_share: f64,
    pub max_over_budget_share: f64,
    /// Mean tick work as a share of the 20 ms budget.
    pub max_mean_tick_share: f64,
}

impl Default for Expect {
    fn default() -> Self {
        Self {
            bot_processes: 1,
            riders_per_process: 1,
            clients: Vec::new(),
            min_ridden_m: 20.0,
            min_snapshot_share: 0.9,
            max_snapshot_gap_s: 1.0,
            max_prediction_error_mean_m: 0.5,
            max_prediction_error_m: ocw_sim::prediction::SNAP_THRESHOLD_M,
            max_late_share: 0.01,
            max_over_budget_share: 0.001,
            max_mean_tick_share: 0.25,
        }
    }
}

/// One check of the verdict.
#[derive(Debug, Clone, PartialEq)]
pub struct Check {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

/// The verdict: every check, passed or not.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Verdict {
    pub checks: Vec<Check>,
}

impl Verdict {
    pub fn passed(&self) -> bool {
        !self.checks.is_empty() && self.checks.iter().all(|c| c.ok)
    }

    fn check(&mut self, name: impl Into<String>, ok: bool, detail: impl Into<String>) {
        self.checks.push(Check {
            name: name.into(),
            ok,
            detail: detail.into(),
        });
    }

    /// Human-readable lines: `PASS name: detail` / `FAIL …`, then the result.
    pub fn lines(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .checks
            .iter()
            .map(|c| {
                format!(
                    "{} {}: {}",
                    if c.ok { "PASS" } else { "FAIL" },
                    c.name,
                    c.detail
                )
            })
            .collect();
        let failed = self.checks.iter().filter(|c| !c.ok).count();
        out.push(if self.passed() {
            format!("RESULT: PASS ({} checks)", self.checks.len())
        } else {
            format!(
                "RESULT: FAIL ({failed} of {} checks failed)",
                self.checks.len()
            )
        });
        out
    }
}

/// Checks the server's statistics and the bots' reports against `expect`.
pub fn verdict(server: &ServerReport, bots: &[BotReport], expect: &Expect) -> Verdict {
    let mut v = Verdict::default();
    let riders: Vec<(&BotReport, &RiderReport)> = bots
        .iter()
        .flat_map(|b| b.riders.iter().map(move |r| (b, r)))
        .collect();
    let want_bots = expect.bot_processes * expect.riders_per_process as usize;
    v.check(
        "bot processes reported",
        bots.len() == expect.bot_processes && riders.len() == want_bots,
        format!(
            "{} of {} processes, {} of {want_bots} riders",
            bots.len(),
            expect.bot_processes,
            riders.len()
        ),
    );
    // Server side.
    let mut clients: Vec<String> = riders.iter().map(|(_, r)| r.name.clone()).collect();
    clients.extend(expect.clients.iter().cloned());
    let missing: Vec<&String> = clients
        .iter()
        .filter(|n| !server.joined.contains(n))
        .collect();
    v.check(
        "all clients joined the server",
        missing.is_empty(),
        if missing.is_empty() {
            format!("{} joined", server.joined.len())
        } else {
            format!("missing: {missing:?}")
        },
    );
    v.check(
        "no client left before the end",
        server.left.is_empty(),
        if server.left.is_empty() {
            "none".to_owned()
        } else {
            format!("left early: {:?}", server.left)
        },
    );
    let total = clients.len() + server.bots as usize;
    v.check(
        "all riders in the session at once",
        server.peak_riders >= total,
        format!("peak {} of {total}", server.peak_riders),
    );
    let ticks = server.ticks.max(1) as f64;
    v.check(
        "server ran in real time",
        server.real_time && server.ticks > 0,
        format!("{} ticks, {:.1} s", server.ticks, server.sim_time_s),
    );
    v.check(
        "server tick timing",
        server.mean_tick_us <= expect.max_mean_tick_share * server.budget_us
            && server.over_budget_ticks as f64 <= expect.max_over_budget_share * ticks
            && server.late_ticks as f64 <= expect.max_late_share * ticks,
        format!(
            "mean {:.0} µs, max {:.0} µs (budget {:.0} µs), {} over budget, {} late (max {:.1} ms)",
            server.mean_tick_us,
            server.max_tick_us,
            server.budget_us,
            server.over_budget_ticks,
            server.late_ticks,
            server.max_late_ms
        ),
    );
    // The course: every bot loaded the server's route.
    let wrong: Vec<String> = bots
        .iter()
        .filter(|b| {
            b.route_id != server.route_id
                || (!server.route_sha256.is_empty() && b.route_sha256 != server.route_sha256)
        })
        .map(|b| format!("{} rides {:?}", b.label, b.route_id))
        .collect();
    v.check(
        "same course everywhere",
        wrong.is_empty(),
        if wrong.is_empty() {
            format!(
                "{:?}{}",
                server.route_id,
                if server.route_sha256.is_empty() {
                    String::new()
                } else {
                    format!(
                        " (sha256 {}…)",
                        &server.route_sha256[..12.min(server.route_sha256.len())]
                    )
                }
            )
        } else {
            wrong.join(", ")
        },
    );
    // Client side, per rider.
    for (b, r) in &riders {
        let who = format!("{} [{}]", r.name, b.label);
        let mut problems = Vec::new();
        if let Some(why) = &r.rejected {
            problems.push(format!("rejected: {why}"));
        }
        if r.joined_at_s.is_none() {
            problems.push("never joined".to_owned());
        }
        if !ended_normally(r, server.ticks) {
            problems.push(format!(
                "disconnected: {} (last snapshot at tick {} of {})",
                r.closed.as_deref().unwrap_or_default(),
                r.last_tick,
                server.ticks
            ));
        }
        let connected_s = match (r.first_snapshot_s, r.last_snapshot_s) {
            (Some(a), Some(b)) => b - a,
            _ => 0.0,
        };
        let wanted = (connected_s * 10.0 * expect.min_snapshot_share).floor() as u64;
        if r.snapshots == 0 || r.snapshots < wanted {
            problems.push(format!(
                "{} snapshots in {connected_s:.0} s (want ≥ {wanted})",
                r.snapshots
            ));
        }
        if r.max_snapshot_gap_s > expect.max_snapshot_gap_s {
            problems.push(format!("snapshot gap {:.2} s", r.max_snapshot_gap_s));
        }
        let others: Vec<&String> = clients.iter().filter(|n| **n != r.name).collect();
        let unseen: Vec<&&String> = others
            .iter()
            .filter(|n| !r.others_seen.contains(n))
            .collect();
        if !unseen.is_empty() {
            problems.push(format!("did not see {unseen:?}"));
        }
        let still: Vec<&&String> = others
            .iter()
            .filter(|n| r.others_seen.contains(n) && !r.others_moving.contains(n))
            .collect();
        if !still.is_empty() {
            problems.push(format!("saw {still:?} standing still"));
        }
        if (r.max_riders_seen as usize) < total {
            problems.push(format!(
                "at most {} of {total} riders in a snapshot",
                r.max_riders_seen
            ));
        }
        if r.ridden_m() < expect.min_ridden_m {
            problems.push(format!("rode only {:.0} m", r.ridden_m()));
        }
        if !r.samples_acked {
            problems.push("the server applied none of its samples".to_owned());
        }
        if r.prediction_samples == 0 {
            problems.push("no prediction samples".to_owned());
        } else if r.prediction_error_mean_m() > expect.max_prediction_error_mean_m
            || r.prediction_error_max_m > expect.max_prediction_error_m
        {
            problems.push(format!(
                "prediction off: mean {:.2} m, max {:.2} m",
                r.prediction_error_mean_m(),
                r.prediction_error_max_m
            ));
        }
        let detail = format!(
            "{} snapshots ({} stale), max gap {:.2} s, saw {} riders, rode {:.0} m, prediction mean {:.3} m / max {:.2} m, rtt ≤ {:.0} ms",
            r.snapshots,
            r.stale_snapshots,
            r.max_snapshot_gap_s,
            r.max_riders_seen,
            r.ridden_m(),
            r.prediction_error_mean_m(),
            r.prediction_error_max_m,
            r.rtt_ms_max
        );
        let detail = match suspend_note(b.suspended_s) {
            Some(n) => format!("{detail}; note: {n}"),
            None => detail,
        };
        v.check(
            format!("rider {who}"),
            problems.is_empty(),
            if problems.is_empty() {
                detail
            } else {
                format!("{}; {detail}", problems.join("; "))
            },
        );
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rider(name: &str, others: &[&str]) -> RiderReport {
        RiderReport {
            name: name.into(),
            route_id: "loop".into(),
            joined_at_s: Some(1.0),
            closed: Some("the server closed the connection: server shutting down".into()),
            last_tick: 3100,
            snapshots: 600,
            first_snapshot_s: Some(1.0),
            last_snapshot_s: Some(61.0),
            max_snapshot_gap_s: 0.2,
            max_riders_seen: 3,
            others_seen: others.iter().map(|s| s.to_string()).collect(),
            others_moving: others.iter().map(|s| s.to_string()).collect(),
            start_m: Some(0.0),
            end_m: 400.0,
            samples_sent: 240,
            acked_seq: 238,
            samples_acked: true,
            prediction_samples: 500,
            prediction_error_sum_m: 25.0,
            prediction_error_max_m: 0.4,
            ..Default::default()
        }
    }

    fn setup() -> (ServerReport, Vec<BotReport>, Expect) {
        let server = ServerReport {
            sim_time_s: 62.0,
            ticks: 3100,
            real_time: true,
            mean_tick_us: 300.0,
            max_tick_us: 4000.0,
            budget_us: 20_000.0,
            peak_riders: 3,
            joined: vec!["linux-1".into(), "mac-1".into(), "Kiosk".into()],
            riders_at_end: 3,
            route_id: "loop".into(),
            ..Default::default()
        };
        let bot = |label: &str, r: RiderReport| BotReport {
            label: label.into(),
            os: "linux".into(),
            route_id: "loop".into(),
            riders: vec![r],
            ..Default::default()
        };
        let bots = vec![
            bot("linux", rider("linux-1", &["mac-1", "Kiosk"])),
            bot("macos", rider("mac-1", &["linux-1", "Kiosk"])),
        ];
        let expect = Expect {
            bot_processes: 2,
            riders_per_process: 1,
            clients: vec!["Kiosk".into()],
            ..Default::default()
        };
        (server, bots, expect)
    }

    #[test]
    fn a_good_session_passes() {
        let (server, bots, expect) = setup();
        let v = verdict(&server, &bots, &expect);
        assert!(v.passed(), "{:#?}", v.lines());
        assert!(v.lines().last().unwrap().starts_with("RESULT: PASS"));
    }

    #[test]
    fn each_problem_fails_its_check() {
        let failed = |f: &dyn Fn(&mut ServerReport, &mut Vec<BotReport>)| {
            let (mut s, mut b, e) = setup();
            f(&mut s, &mut b);
            let v = verdict(&s, &b, &e);
            assert!(!v.passed());
            v.checks
                .iter()
                .filter(|c| !c.ok)
                .map(|c| c.name.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            failed(&|s, _| s.joined.retain(|n| n != "Kiosk")),
            ["all clients joined the server"]
        );
        assert_eq!(
            failed(&|s, _| s.left.push("mac-1".into())),
            ["no client left before the end"]
        );
        assert_eq!(failed(&|s, _| s.late_ticks = 100), ["server tick timing"]);
        assert_eq!(
            failed(&|_, b| {
                b.pop();
            }),
            ["bot processes reported"]
        );
        assert_eq!(
            failed(&|_, b| b[1].route_id = "hills".into()),
            ["same course everywhere"]
        );
        let rider_fails = |f: &dyn Fn(&mut RiderReport)| failed(&|_, b| f(&mut b[0].riders[0]));
        let one = vec!["rider linux-1 [linux]".to_owned()];
        assert_eq!(
            rider_fails(&|r| {
                // Lost a minute before the end: a real disconnect.
                r.closed = Some("connection lost: timed out".into());
                r.last_tick = 100;
            }),
            one
        );
        // A reset right at the server's end: its closing message was lost.
        let (s, mut b, e) = setup();
        b[0].riders[0].closed = Some("connection lost: reset by peer".into());
        b[0].riders[0].last_tick = 3000;
        assert!(verdict(&s, &b, &e).passed());
        b[0].riders[0].last_tick = 2000;
        assert!(!verdict(&s, &b, &e).passed());
        assert_eq!(rider_fails(&|r| r.snapshots = 100), one);
        assert_eq!(rider_fails(&|r| r.max_snapshot_gap_s = 3.0), one);
        assert_eq!(
            rider_fails(&|r| r.others_seen.retain(|n| n != "Kiosk")),
            one
        );
        assert_eq!(rider_fails(&|r| r.others_moving.clear()), one);
        assert_eq!(rider_fails(&|r| r.end_m = 5.0), one);
        assert_eq!(rider_fails(&|r| r.samples_acked = false), one);
        assert_eq!(rider_fails(&|r| r.prediction_error_max_m = 9.0), one);
        assert_eq!(
            rider_fails(&|r| r.rejected = Some("world package mismatch".into())),
            one
        );
        assert_eq!(rider_fails(&|r| r.max_riders_seen = 2), one);
    }

    #[test]
    fn suspends_are_noted() {
        assert!(suspend_note(0.1).is_none());
        assert!(suspend_note(2.4).unwrap().contains("2.4 s"));
        let (s, mut b, e) = setup();
        b[1].suspended_s = 3.0;
        let v = verdict(&s, &b, &e);
        assert!(v.passed());
        assert!(v.lines().iter().any(|l| l.contains("suspended for 3.0 s")));
    }

    #[test]
    fn an_empty_verdict_does_not_pass() {
        assert!(!Verdict::default().passed());
    }

    #[test]
    fn reports_are_found_in_process_output() {
        let (_, bots, _) = setup();
        let out = format!(
            "INFO starting\r\n{}\r\nnoise\n{}\nOCW-BOT-REPORT {{broken\n",
            bots[0].to_line(),
            bots[1].to_line()
        );
        let found = BotReport::from_output(&out);
        assert_eq!(found.len(), 3);
        assert_eq!(found[0].as_ref().unwrap(), &bots[0]);
        assert_eq!(found[1].as_ref().unwrap(), &bots[1]);
        assert!(found[2].is_err());
    }
}
