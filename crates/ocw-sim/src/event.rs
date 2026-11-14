//! Events (spec §2.3): group rides, races and time trials, decided by the
//! server (spec §6.1).
//!
//! An [`Event`] is a pure state machine over server ticks:
//! `Scheduled → Countdown → Running → Finished`. The session
//! ([`crate::Session`]) holds the field at the start line during the
//! countdown and feeds the riders' distances in every tick; the event
//! decides the start, who finished when (the crossing time interpolated
//! within the tick), DNFs and the ranking ([`Event::standings`]).
//!
//! The ghost of a time trial is not part of the server's event: it is the
//! rider's own best previous effort, recorded and replayed on the client as
//! a distance-over-time trace ([`GhostTrace`]).

use ocw_core::{RiderId, SIM_DT};

/// Length of the countdown before the start (s): "Get ready", then 3-2-1.
pub const COUNTDOWN_S: f64 = 5.0;
/// Ticks of the countdown.
pub const COUNTDOWN_TICKS: u64 = (COUNTDOWN_S / SIM_DT) as u64;
/// After the first finisher, the others have this long (s), or
/// [`DNF_GRACE_SHARE`] of the winner's time if that is longer, before
/// they are DNF.
pub const DNF_GRACE_MIN_S: f64 = 120.0;
pub const DNF_GRACE_SHARE: f64 = 0.25;
/// Default race / time-trial length on routes that end (m).
pub const DEFAULT_LENGTH_M: f64 = 5000.0;
/// Shortest event (m).
pub const MIN_LENGTH_M: f64 = 100.0;
/// Default group-ride pace (W): 2 W/kg for the default 75 kg rider.
pub const DEFAULT_GROUP_W: f64 = 150.0;
/// Group ride: bots ride within ± this share of the target power.
pub const GROUP_BAND: f64 = 0.1;
/// Group ride: power change per metre away from the group (share of the
/// target): a bot 10 m ahead eases off by 25 %.
pub const GROUP_REGROUP_PER_M: f64 = 0.025;

/// What kind of event, with its parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EventKind {
    /// Bots hold `target_w` (within [`GROUP_BAND`]) and regroup; no
    /// ranking, no finish.
    GroupRide { target_w: f64 },
    /// Everyone from the start formation to the finish `length_m` after the
    /// start line (`laps` of a loop, if given that way); ranked by finish
    /// time.
    Race { length_m: f64, laps: Option<u32> },
    /// One rider alone (no drafting) to the finish; the ghost is the
    /// client's.
    TimeTrial { length_m: f64, laps: Option<u32> },
}

impl EventKind {
    /// Distance from the start line to the finish (none for group rides).
    pub fn length_m(&self) -> Option<f64> {
        match *self {
            Self::GroupRide { .. } => None,
            Self::Race { length_m, .. } | Self::TimeTrial { length_m, .. } => Some(length_m),
        }
    }

    pub fn laps(&self) -> Option<u32> {
        match *self {
            Self::GroupRide { .. } => None,
            Self::Race { laps, .. } | Self::TimeTrial { laps, .. } => laps,
        }
    }

    /// Short name: "group ride", "race", "time trial".
    pub fn name(&self) -> &'static str {
        match self {
            Self::GroupRide { .. } => "group ride",
            Self::Race { .. } => "race",
            Self::TimeTrial { .. } => "time trial",
        }
    }

    /// Whether drafting is on (off in a time trial).
    pub fn drafting(&self) -> bool {
        !matches!(self, Self::TimeTrial { .. })
    }
}

/// An event as requested (console `event race --laps 2`, a schedule): the
/// session turns it into an [`EventKind`] for its course
/// ([`resolve_length`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EventSpec {
    GroupRide {
        target_w: Option<f64>,
    },
    Race {
        laps: Option<u32>,
        length_m: Option<f64>,
    },
    TimeTrial {
        laps: Option<u32>,
        length_m: Option<f64>,
    },
}

/// The finish of a race or time trial on a course `course_m` long (a loop,
/// or a route that ends) with the start line at `start_line_m`: `laps`
/// (loops only), or `length_m`, or by default one lap of a loop / up to
/// [`DEFAULT_LENGTH_M`] of a route that ends. Returns (length, laps).
pub fn resolve_length(
    laps: Option<u32>,
    length_m: Option<f64>,
    course_m: f64,
    is_loop: bool,
    start_line_m: f64,
) -> Result<(f64, Option<u32>), String> {
    let room = if is_loop {
        f64::INFINITY
    } else {
        course_m - start_line_m
    };
    let (length, laps) = match (laps, length_m) {
        (Some(_), Some(_)) => return Err("give either laps or a distance, not both".into()),
        (Some(_), None) if !is_loop => {
            return Err("laps need a loop; this route ends: give a distance (--km)".into());
        }
        (Some(0), None) => return Err("at least one lap".into()),
        (Some(n), None) => (f64::from(n) * course_m, Some(n)),
        (None, Some(l)) => (l, None),
        (None, None) if is_loop => (course_m, Some(1)),
        (None, None) => (DEFAULT_LENGTH_M.min(room), None),
    };
    if length.is_nan() || length < MIN_LENGTH_M {
        return Err(format!("too short: at least {MIN_LENGTH_M:.0} m"));
    }
    if length > room + 1e-6 {
        return Err(format!(
            "the route ends {:.2} km after the start line",
            room.max(0.0) / 1000.0
        ));
    }
    Ok((length, laps))
}

/// Where an event is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Phase {
    /// Announced; the countdown starts at `countdown_tick`.
    Scheduled,
    /// The field waits at the start line; GO at `go_tick`.
    Countdown,
    Running,
    /// Everyone finished or is DNF (races and time trials only).
    Finished,
}

/// One rider in an event.
#[derive(Debug, Clone, PartialEq)]
pub struct Entrant {
    pub id: RiderId,
    pub name: String,
    /// Seconds from GO to crossing the finish line.
    pub finish_s: Option<f64>,
    /// Did not finish (left the session, or over the time limit).
    pub dnf: bool,
}

/// A rider's place in the standings.
#[derive(Debug, Clone, PartialEq)]
pub struct Standing {
    pub id: RiderId,
    pub name: String,
    /// 1-based.
    pub position: u32,
    /// Distance past the start line (m; negative behind it).
    pub progress_m: f64,
    pub finish_s: Option<f64>,
    pub dnf: bool,
    /// Time to the rider ranked one place ahead (s): the difference of the
    /// finish times, or for riders still riding the gap in metres at their
    /// speed. `None` for the leader and DNFs.
    pub gap_ahead_s: Option<f64>,
}

/// Something that happened in a tick, for broadcasting.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EventChange {
    Phase(Phase),
    Finished { id: RiderId, time_s: f64 },
    Dnf(RiderId),
}

/// A rider's distance before and after this tick.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Progress {
    pub id: RiderId,
    pub before_m: f64,
    pub after_m: f64,
    pub speed: f64,
}

/// One event of a session.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub id: u32,
    pub kind: EventKind,
    pub phase: Phase,
    pub countdown_tick: u64,
    pub go_tick: u64,
    pub finished_tick: Option<u64>,
    /// Distance (along the course) of the start line; the finish is
    /// `kind.length_m()` after it.
    pub start_line_m: f64,
    pub entrants: Vec<Entrant>,
    /// The organiser's collision rule for this event (spec §14 Phase 12A;
    /// `None`: by its kind – group rides soft bumps only, races the full
    /// model, Q2).
    pub collisions: Option<crate::collide::Mode>,
    /// Whose physics count (spec §14 Phase 4, 4G): the organiser's choice,
    /// by default *Profile* for races and group rides, *Open* for a time
    /// trial ([`crate::build::PhysicsMode::for_event`]).
    pub physics: crate::build::PhysicsMode,
}

impl Event {
    /// An event whose countdown starts at `countdown_tick` (now, for an
    /// event started on demand).
    pub fn new(
        id: u32,
        kind: EventKind,
        countdown_tick: u64,
        start_line_m: f64,
        entrants: Vec<(RiderId, String)>,
    ) -> Self {
        Self {
            id,
            kind,
            phase: Phase::Scheduled,
            countdown_tick,
            go_tick: countdown_tick + COUNTDOWN_TICKS,
            finished_tick: None,
            collisions: None,
            physics: crate::build::PhysicsMode::for_event(&kind),
            start_line_m,
            entrants: entrants
                .into_iter()
                .map(|(id, name)| Entrant {
                    id,
                    name,
                    finish_s: None,
                    dnf: false,
                })
                .collect(),
        }
    }

    /// Whether riders are held at the start line (before GO).
    pub fn holds(&self) -> bool {
        self.phase < Phase::Running
    }

    /// Seconds since GO at `tick` (negative before).
    pub fn race_time_s(&self, tick: u64) -> f64 {
        (tick as f64 - self.go_tick as f64) * SIM_DT
    }

    pub fn entrant(&self, id: RiderId) -> Option<&Entrant> {
        self.entrants.iter().find(|e| e.id == id)
    }

    /// Advances to `tick` (the session's tick after stepping) with the
    /// riders' distances before and after it; riders missing from
    /// `riders` left the session (DNF).
    pub fn update(&mut self, tick: u64, riders: &[Progress]) -> Vec<EventChange> {
        let mut out = Vec::new();
        if self.phase == Phase::Scheduled && tick >= self.countdown_tick {
            self.phase = Phase::Countdown;
            out.push(EventChange::Phase(Phase::Countdown));
        }
        if self.phase == Phase::Countdown && tick >= self.go_tick {
            self.phase = Phase::Running;
            out.push(EventChange::Phase(Phase::Running));
            return out;
        }
        if self.phase != Phase::Running {
            return out;
        }
        let Some(length) = self.kind.length_m() else {
            return out;
        };
        let finish = self.start_line_m + length;
        for e in &mut self.entrants {
            if e.finish_s.is_some() || e.dnf {
                continue;
            }
            match riders.iter().find(|p| p.id == e.id) {
                None => {
                    e.dnf = true;
                    out.push(EventChange::Dnf(e.id));
                }
                Some(p) if p.after_m >= finish => {
                    // Where in the tick the line was crossed.
                    let step = p.after_m - p.before_m;
                    let frac = if step > 1e-9 {
                        ((finish - p.before_m) / step).clamp(0.0, 1.0)
                    } else {
                        1.0
                    };
                    let t = (tick as f64 - 1.0 + frac - self.go_tick as f64) * SIM_DT;
                    let t = t.max(0.0);
                    e.finish_s = Some(t);
                    out.push(EventChange::Finished {
                        id: e.id,
                        time_s: t,
                    });
                }
                Some(_) => {}
            }
        }
        // The time limit after the first finisher.
        if let Some(winner) = self
            .entrants
            .iter()
            .filter_map(|e| e.finish_s)
            .min_by(f64::total_cmp)
        {
            let limit = winner + DNF_GRACE_MIN_S.max(DNF_GRACE_SHARE * winner);
            if self.race_time_s(tick) > limit {
                for e in &mut self.entrants {
                    if e.finish_s.is_none() && !e.dnf {
                        e.dnf = true;
                        out.push(EventChange::Dnf(e.id));
                    }
                }
            }
        }
        if self.entrants.iter().all(|e| e.finish_s.is_some() || e.dnf) {
            self.phase = Phase::Finished;
            self.finished_tick = Some(tick);
            out.push(EventChange::Phase(Phase::Finished));
        }
        out
    }

    /// The ranking: finishers by time, then riders still riding by how far
    /// they are, then DNFs. `riders`: (id, distance, speed) now.
    pub fn standings(&self, riders: &[(RiderId, f64, f64)]) -> Vec<Standing> {
        let at = |id: RiderId| riders.iter().find(|r| r.0 == id).map(|r| (r.1, r.2));
        let mut rows: Vec<Standing> = self
            .entrants
            .iter()
            .map(|e| {
                let (d, _) = at(e.id).unwrap_or((self.start_line_m, 0.0));
                let progress = match (e.finish_s, self.kind.length_m()) {
                    (Some(_), Some(l)) => l,
                    _ => d - self.start_line_m,
                };
                Standing {
                    id: e.id,
                    name: e.name.clone(),
                    position: 0,
                    progress_m: progress,
                    finish_s: e.finish_s,
                    dnf: e.dnf,
                    gap_ahead_s: None,
                }
            })
            .collect();
        let class = |s: &Standing| match (s.finish_s, s.dnf) {
            (Some(_), _) => 0,
            (None, false) => 1,
            (None, true) => 2,
        };
        rows.sort_by(|a, b| {
            class(a)
                .cmp(&class(b))
                .then_with(|| match (a.finish_s, b.finish_s) {
                    (Some(x), Some(y)) => x.total_cmp(&y),
                    _ => b.progress_m.total_cmp(&a.progress_m),
                })
        });
        for i in 0..rows.len() {
            rows[i].position = i as u32 + 1;
            if i == 0 || rows[i].dnf {
                continue;
            }
            let (ahead, me) = (&rows[i - 1], &rows[i]);
            rows[i].gap_ahead_s = match (ahead.finish_s, me.finish_s) {
                (Some(a), Some(m)) => Some(m - a),
                _ => {
                    let speed = at(me.id).map_or(0.0, |r| r.1).max(1.0);
                    Some(((ahead.progress_m - me.progress_m).max(0.0)) / speed)
                }
            };
        }
        rows
    }
}

/// Group ride: a bot's power. `base_w` is the target, `personal` the bot's
/// own factor within ±[`GROUP_BAND`] (so the group is not in lockstep),
/// `ahead_m` how far the bot is ahead of the group's anchor (negative:
/// behind). Bots ahead ease off, bots behind push, so the group stays
/// together; between half and 1.6 times the target.
pub fn group_power_w(target_w: f64, personal: f64, ahead_m: f64) -> f64 {
    let p = personal.clamp(-GROUP_BAND, GROUP_BAND);
    let regroup = (-GROUP_REGROUP_PER_M * ahead_m).clamp(-0.5, 0.6);
    target_w * (1.0 + p + regroup)
}

/// Group ride: the point the bots regroup around. With client riders it
/// lies halfway between the bots' mean and the clients' (the group waits
/// for the riders, the riders do not have to chase it alone); without, at
/// the bots' mean.
pub fn group_anchor(bots: &[f64], clients: &[f64]) -> Option<f64> {
    let mean = |v: &[f64]| (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64);
    match (mean(bots), mean(clients)) {
        (Some(b), Some(c)) => Some(0.5 * (b + c)),
        (b, c) => b.or(c),
    }
}

/// A rider's effort as distance over time (from GO), for the ghost of a
/// time trial: samples `(t_s, progress_m)`, both increasing.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GhostTrace {
    samples: Vec<(f64, f64)>,
}

/// Sample interval of a ghost trace (s).
pub const GHOST_SAMPLE_S: f64 = 0.5;

impl GhostTrace {
    /// A trace from samples; out-of-order ones (time or distance going
    /// back) are dropped.
    pub fn from_samples(samples: impl IntoIterator<Item = (f64, f64)>) -> Self {
        let mut t = Self::default();
        for (time, d) in samples {
            t.push(time, d);
        }
        t
    }

    /// Adds a sample if it is newer and not behind the last one.
    pub fn push(&mut self, t_s: f64, progress_m: f64) {
        if !(t_s.is_finite() && progress_m.is_finite()) {
            return;
        }
        if let Some(&(lt, ld)) = self.samples.last()
            && (t_s <= lt || progress_m < ld)
        {
            return;
        }
        self.samples.push((t_s, progress_m));
    }

    pub fn samples(&self) -> &[(f64, f64)] {
        &self.samples
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Time of the last sample (the finish time of a complete trace).
    pub fn duration_s(&self) -> f64 {
        self.samples.last().map_or(0.0, |s| s.0)
    }

    /// Distance of the last sample.
    pub fn length_m(&self) -> f64 {
        self.samples.last().map_or(0.0, |s| s.1)
    }

    /// Progress at `t_s` (linear between samples; 0 before the first, the
    /// end after the last: the ghost waits at the finish).
    pub fn distance_at(&self, t_s: f64) -> f64 {
        let s = &self.samples;
        let Some(&(t0, d0)) = s.first() else {
            return 0.0;
        };
        if t_s <= t0 {
            return if t_s < 0.0 {
                0.0
            } else {
                d0 * (t_s / t0.max(1e-9)).min(1.0)
            };
        }
        let i = s.partition_point(|p| p.0 <= t_s);
        if i >= s.len() {
            return self.length_m();
        }
        let (a, b) = (s[i - 1], s[i]);
        a.1 + (b.1 - a.1) * (t_s - a.0) / (b.0 - a.0)
    }

    /// When the trace reached `progress_m` (linear; `None` beyond its end).
    pub fn time_at(&self, progress_m: f64) -> Option<f64> {
        let s = &self.samples;
        let &(t0, d0) = s.first()?;
        if progress_m <= d0 {
            return Some(if d0 > 1e-9 {
                t0 * (progress_m.max(0.0) / d0)
            } else {
                t0
            });
        }
        let i = s.partition_point(|p| p.1 < progress_m);
        if i >= s.len() {
            return None;
        }
        let (a, b) = (s[i - 1], s[i]);
        let span = b.1 - a.1;
        Some(if span > 1e-9 {
            a.0 + (b.0 - a.0) * (progress_m - a.1) / span
        } else {
            b.0
        })
    }

    /// Time gap to this ghost of a rider at `progress_m` at `t_s`
    /// (positive: behind the ghost, s). Past the ghost's finish the gap
    /// is measured against its finish time.
    pub fn gap_s(&self, t_s: f64, progress_m: f64) -> Option<f64> {
        if self.samples.is_empty() {
            return None;
        }
        let at = self
            .time_at(progress_m)
            .unwrap_or_else(|| self.duration_s());
        Some(t_s - at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(n: u32) -> Vec<(RiderId, String)> {
        (1..=n).map(|i| (RiderId(i), format!("r{i}"))).collect()
    }

    fn race(length_m: f64, n: u32) -> Event {
        Event::new(
            1,
            EventKind::Race {
                length_m,
                laps: None,
            },
            10,
            50.0,
            ids(n),
        )
    }

    #[test]
    fn phases_scheduled_countdown_running() {
        let mut e = race(1000.0, 2);
        assert_eq!(e.phase, Phase::Scheduled);
        assert!(e.holds());
        assert!(e.update(9, &[]).is_empty());
        assert_eq!(
            e.update(10, &[]),
            vec![EventChange::Phase(Phase::Countdown)]
        );
        assert!(e.holds());
        assert_eq!(e.go_tick, 10 + COUNTDOWN_TICKS);
        assert!(e.update(e.go_tick - 1, &[]).is_empty());
        assert_eq!(
            e.update(e.go_tick, &[]),
            vec![EventChange::Phase(Phase::Running)]
        );
        assert!(!e.holds());
        assert_eq!(e.race_time_s(e.go_tick + 50), 1.0);
    }

    fn running(mut e: Event) -> Event {
        e.update(e.countdown_tick, &[]);
        e.update(e.go_tick, &[]);
        e
    }

    fn at(id: u32, before: f64, after: f64) -> Progress {
        Progress {
            id: RiderId(id),
            before_m: before,
            after_m: after,
            speed: (after - before) / SIM_DT,
        }
    }

    #[test]
    fn finish_time_is_interpolated_within_the_tick() {
        let mut e = running(race(1000.0, 2));
        let go = e.go_tick;
        // 100 ticks later (2 s) rider 1 crosses 1050 m a quarter into the tick.
        let ch = e.update(go + 100, &[at(1, 1049.0, 1053.0), at(2, 900.0, 904.0)]);
        let EventChange::Finished { id, time_s } = ch[0] else {
            panic!("{ch:?}")
        };
        assert_eq!(id, RiderId(1));
        assert!((time_s - (99.25 * SIM_DT)).abs() < 1e-9, "{time_s}");
        assert_eq!(e.phase, Phase::Running);
        // Rider 2 finishes: the race is over.
        let ch = e.update(go + 200, &[at(1, 1100.0, 1104.0), at(2, 1048.0, 1052.0)]);
        assert!(ch.contains(&EventChange::Phase(Phase::Finished)), "{ch:?}");
        assert_eq!(e.finished_tick, Some(go + 200));
        // Finished: nothing more happens.
        assert!(e.update(go + 300, &[]).is_empty());
    }

    #[test]
    fn riders_who_leave_or_are_too_slow_are_dnf() {
        let mut e = running(race(1000.0, 3));
        let go = e.go_tick;
        // Rider 3 left the session.
        let ch = e.update(go + 1, &[at(1, 100.0, 101.0), at(2, 100.0, 101.0)]);
        assert_eq!(ch, vec![EventChange::Dnf(RiderId(3))]);
        // Rider 1 wins after 100 s.
        let t1 = go + 5000;
        e.update(t1, &[at(1, 1049.0, 1051.0), at(2, 500.0, 501.0)]);
        assert!(e.entrant(RiderId(1)).unwrap().finish_s.is_some());
        // Within the time limit rider 2 is still riding ...
        let limit_ticks = (DNF_GRACE_MIN_S / SIM_DT) as u64;
        e.update(t1 + limit_ticks - 10, &[at(2, 600.0, 601.0)]);
        assert!(!e.entrant(RiderId(2)).unwrap().dnf);
        // ... after it, DNF, and the race is over.
        let ch = e.update(t1 + limit_ticks + 10, &[at(2, 700.0, 701.0)]);
        assert!(ch.contains(&EventChange::Dnf(RiderId(2))));
        assert_eq!(e.phase, Phase::Finished);
    }

    #[test]
    fn standings_rank_finishers_riders_and_dnfs() {
        let mut e = running(race(1000.0, 4));
        let go = e.go_tick;
        e.entrants[1].finish_s = Some(100.0); // r2
        e.entrants[3].finish_s = Some(104.5); // r4
        e.entrants[2].dnf = true; // r3
        let _ = go;
        let s = e.standings(&[
            (RiderId(1), 50.0 + 900.0, 10.0),
            (RiderId(2), 1100.0, 8.0),
            (RiderId(4), 1080.0, 8.0),
        ]);
        let order: Vec<u32> = s.iter().map(|r| r.id.0).collect();
        assert_eq!(order, [2, 4, 1, 3]);
        assert_eq!(
            s.iter().map(|r| r.position).collect::<Vec<_>>(),
            [1, 2, 3, 4]
        );
        assert_eq!(s[0].gap_ahead_s, None);
        assert!((s[1].gap_ahead_s.unwrap() - 4.5).abs() < 1e-9);
        // Riding: 100 m behind the finisher at 10 m/s.
        assert!((s[2].progress_m - 900.0).abs() < 1e-9);
        assert!((s[2].gap_ahead_s.unwrap() - 10.0).abs() < 1e-9);
        assert_eq!(s[1].progress_m, 1000.0);
        assert!(s[3].dnf && s[3].gap_ahead_s.is_none());
    }

    #[test]
    fn standings_while_riding_order_by_distance() {
        let e = running(race(1000.0, 3));
        let s = e.standings(&[
            (RiderId(1), 60.0, 5.0),
            (RiderId(2), 80.0, 5.0),
            (RiderId(3), 70.0, 0.0),
        ]);
        assert_eq!(s.iter().map(|r| r.id.0).collect::<Vec<_>>(), [2, 3, 1]);
        // Standing still counts as 1 m/s (no infinite gap).
        assert!((s[1].gap_ahead_s.unwrap() - 10.0).abs() < 1e-9);
        assert!((s[2].gap_ahead_s.unwrap() - 2.0).abs() < 1e-9);
    }

    #[test]
    fn group_rides_never_finish() {
        let mut e = running(Event::new(
            2,
            EventKind::GroupRide { target_w: 150.0 },
            0,
            0.0,
            ids(2),
        ));
        assert!(
            e.update(e.go_tick + 100_000, &[at(1, 1e6, 1e6 + 1.0)])
                .is_empty()
        );
        assert_eq!(e.phase, Phase::Running);
        assert!(!EventKind::GroupRide { target_w: 1.0 }.length_m().is_some());
        assert!(
            !EventKind::TimeTrial {
                length_m: 1.0,
                laps: None
            }
            .drafting()
        );
    }

    #[test]
    fn group_power_regroups_within_limits() {
        assert_eq!(group_power_w(150.0, 0.0, 0.0), 150.0);
        assert!(group_power_w(150.0, 0.0, 10.0) < 150.0);
        assert!(group_power_w(150.0, 0.0, -10.0) > 150.0);
        assert_eq!(group_power_w(150.0, 0.0, 1000.0), 75.0);
        assert_eq!(group_power_w(150.0, 0.0, -1000.0), 240.0);
        // The personal factor stays in the band.
        assert!((group_power_w(100.0, 0.5, 0.0) - 110.0).abs() < 1e-9);
        assert_eq!(group_anchor(&[10.0, 20.0], &[]), Some(15.0));
        assert_eq!(group_anchor(&[10.0, 20.0], &[35.0]), Some(25.0));
        assert_eq!(group_anchor(&[], &[35.0]), Some(35.0));
        assert_eq!(group_anchor(&[], &[]), None);
    }

    #[test]
    fn event_length_from_laps_distance_or_default() {
        // A 4 km loop.
        assert_eq!(
            resolve_length(Some(2), None, 4000.0, true, 4000.0),
            Ok((8000.0, Some(2)))
        );
        assert_eq!(
            resolve_length(None, None, 4000.0, true, 4000.0),
            Ok((4000.0, Some(1)))
        );
        assert_eq!(
            resolve_length(None, Some(10_000.0), 4000.0, true, 4000.0),
            Ok((10_000.0, None))
        );
        assert!(resolve_length(Some(0), None, 4000.0, true, 0.0).is_err());
        assert!(resolve_length(Some(1), Some(1.0), 4000.0, true, 0.0).is_err());
        // A route that ends 12 km after its start; the line at 9 m.
        assert_eq!(
            resolve_length(None, None, 12_000.0, false, 9.0),
            Ok((DEFAULT_LENGTH_M, None))
        );
        assert_eq!(
            resolve_length(None, None, 3009.0, false, 9.0),
            Ok((3000.0, None))
        );
        assert!(resolve_length(Some(1), None, 12_000.0, false, 9.0).is_err());
        let e = resolve_length(None, Some(13_000.0), 12_000.0, false, 9.0).unwrap_err();
        assert!(e.contains("11.99 km"), "{e}");
        assert!(resolve_length(None, Some(50.0), 12_000.0, false, 9.0).is_err());
        assert!(resolve_length(None, None, 50.0, false, 9.0).is_err());
    }

    #[test]
    fn ghost_trace_interpolates_both_ways() {
        let g = GhostTrace::from_samples([(0.0, 0.0), (10.0, 50.0), (20.0, 150.0), (15.0, 99.0)]);
        assert_eq!(g.samples().len(), 3, "out-of-order sample dropped");
        assert_eq!(g.distance_at(-1.0), 0.0);
        assert_eq!(g.distance_at(5.0), 25.0);
        assert_eq!(g.distance_at(15.0), 100.0);
        assert_eq!(g.distance_at(99.0), 150.0, "waits at the finish");
        assert_eq!(g.time_at(25.0), Some(5.0));
        assert_eq!(g.time_at(100.0), Some(15.0));
        assert_eq!(g.time_at(151.0), None);
        assert_eq!(g.duration_s(), 20.0);
        assert_eq!(g.length_m(), 150.0);
        // At 100 m after 17 s: 2 s behind the ghost; after 13 s: 2 s ahead.
        assert_eq!(g.gap_s(17.0, 100.0), Some(2.0));
        assert_eq!(g.gap_s(13.0, 100.0), Some(-2.0));
        // Past the ghost's end: against its finish time.
        assert_eq!(g.gap_s(25.0, 160.0), Some(5.0));
        assert_eq!(GhostTrace::default().gap_s(1.0, 1.0), None);
    }
}
