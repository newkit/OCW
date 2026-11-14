//! The client's side of events (spec §2.3), Bevy-free: the server's event
//! state → HUD texts ([`EventView`]): the countdown, position and gaps, the
//! results panel; holding the prediction at the start line during the
//! countdown; and the time trial's ghost: the own effort is recorded as a
//! distance-over-time trace and the best one per course and length is
//! replayed (stored by `ghost.rs`).
//!
//! The server decides start, finish and ranking; the ghost is the client's
//! alone (a rendering and a gap, spec §2.3): the server never needs it.

use ocw_core::{RiderId, SIM_DT};
use ocw_protocol::EventInfo;
use ocw_render::{EventView, ResultRow, ResultsView};
use ocw_sim::event::GHOST_SAMPLE_S;
use ocw_sim::{EventKind, EventPhase, GhostTrace, Standing};

/// "GO!" stays this long after the start (s).
pub const GO_SHOW_S: f64 = 1.5;
/// The results panel closes by itself after this long (s).
pub const RESULTS_SHOW_S: f64 = 30.0;
/// Most rows of the results panel (ten-foot UI), plus the local rider's.
pub const RESULT_ROWS: usize = 10;
/// Group ride: within this distance of the group's centre you are "in the
/// group" (m).
pub const IN_GROUP_M: f64 = 15.0;

/// Which ghost: the course (route id; routes are ridden in their own
/// direction) and the time trial's length.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GhostKey {
    pub course: String,
    pub length_m: u32,
}

impl GhostKey {
    /// File name stem: the course id with anything but letters, digits,
    /// `-` and `_` replaced, and the length: `test-hills_4120m`.
    pub fn file_stem(&self) -> String {
        let course: String = self
            .course
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        format!("{course}_{}m", self.length_m)
    }
}

/// What saving a finished time trial's trace did.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GhostOutcome {
    /// The first effort on this course and length: now the ghost.
    First,
    /// Faster than the stored best (its time): the new ghost.
    NewBest { previous_s: f64 },
    /// Slower than the stored best (kept).
    Slower { best_s: f64 },
    /// Could not be saved (the reason is logged).
    Failed,
}

#[derive(Debug, Default)]
pub struct EventClient {
    info: Option<EventInfo>,
    /// The course id for ghosts.
    course: String,
    /// The own effort of the running time trial (event id, trace).
    trace: Option<(u32, GhostTrace)>,
    /// A finished time trial's trace, to save.
    finished_trace: Option<(GhostKey, GhostTrace)>,
    /// The ghost for a key (`None` inside: there is none yet).
    ghost: Option<(GhostKey, Option<GhostTrace>)>,
    outcome: Option<GhostOutcome>,
    /// Client time the results panel appeared, for which event.
    results_since: Option<(u32, f64)>,
    dismissed: Option<u32>,
    /// The event whose own trace is complete (recorded once).
    completed: Option<u32>,
    /// Client time the results panel was closed.
    dismissed_at: Option<f64>,
}

/// "10:12.3", "1:02:03.4".
pub fn race_time(seconds: f64) -> String {
    let tenths = (seconds.max(0.0) * 10.0).round() as u64;
    let (s, t) = (tenths / 10, tenths % 10);
    if s >= 3600 {
        format!("{}:{:02}:{:02}.{t}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}.{t}", s / 60, s % 60)
    }
}

/// A time gap: "+4.2 s", "-0.8 s", "+1:04.2".
pub fn gap(seconds: f64) -> String {
    let sign = if seconds < 0.0 { '-' } else { '+' };
    let a = seconds.abs();
    if (a * 10.0).round() >= 600.0 {
        format!("{sign}{}", race_time(a))
    } else {
        format!("{sign}{a:.1} s")
    }
}

/// The big countdown text `remaining_s` before GO (negative after it):
/// "Get ready" above 3 s, then "3", "2", "1", and "GO!" for
/// [`GO_SHOW_S`].
pub fn countdown_text(remaining_s: f64) -> Option<String> {
    if remaining_s > 3.0 {
        Some("Get ready".into())
    } else if remaining_s > 0.0 {
        Some(format!("{}", remaining_s.ceil() as u32))
    } else if remaining_s > -GO_SHOW_S {
        Some("GO!".into())
    } else {
        None
    }
}

/// "1 lap (4.0 km)", "5.0 km".
pub fn distance_label(length_m: f64, laps: Option<u32>) -> String {
    ocw_server::distance_label(length_m, laps)
}

/// The event's title: "Race - 2 laps (8.1 km)", "Group ride - 150 W".
pub fn title(kind: &EventKind) -> String {
    match *kind {
        EventKind::GroupRide { target_w } => format!("Group ride - {target_w:.0} W"),
        EventKind::Race { length_m, laps } => format!("Race - {}", distance_label(length_m, laps)),
        EventKind::TimeTrial { length_m, laps } => {
            format!("Time trial - {}", distance_label(length_m, laps))
        }
    }
}

/// "Lap 2/3 | 1.23 km to go" (laps) or "1.23 km to go".
pub fn to_go(kind: &EventKind, progress_m: f64) -> Option<String> {
    let length = kind.length_m()?;
    let left = (length - progress_m.max(0.0)).max(0.0);
    let km = format!("{:.2} km to go", left / 1000.0);
    Some(match kind.laps() {
        Some(n) if n > 1 => {
            let lap_m = length / f64::from(n);
            let lap = ((progress_m.max(0.0) / lap_m).floor() as u32 + 1).min(n);
            format!("Lap {lap}/{n} | {km}")
        }
        _ => km,
    })
}

/// The race line: "P3/11  +4.2 s to Bot 4", or for the leader
/// "P1/11  leading by 2.0 s".
pub fn position_line(standings: &[Standing], own: RiderId) -> Option<String> {
    let i = standings.iter().position(|s| s.id == own)?;
    let me = &standings[i];
    let n = standings.len();
    let detail = if i == 0 {
        standings
            .get(1)
            .and_then(|next| next.gap_ahead_s)
            .map(|g| format!("  leading by {g:.1} s"))
            .unwrap_or_default()
    } else {
        me.gap_ahead_s
            .map(|g| format!("  {} to {}", gap(g), standings[i - 1].name))
            .unwrap_or_default()
    };
    Some(format!("P{}/{n}{detail}", me.position))
}

/// The ghost line of a time trial: "Ghost +3.4 s (behind)" /
/// "Ghost -2.1 s (ahead)".
pub fn ghost_line(gap_s: f64) -> String {
    let side = if gap_s > 0.05 {
        " (behind)"
    } else if gap_s < -0.05 {
        " (ahead)"
    } else {
        ""
    };
    format!("Ghost {}{side}", gap(gap_s))
}

/// Group ride: where the rider is relative to the group's centre
/// (`ahead_m` = rider − group, m).
pub fn group_line(riders: usize, ahead_m: Option<f64>) -> String {
    let place = match ahead_m {
        None => String::new(),
        Some(a) if a.abs() <= IN_GROUP_M => " | in the group".into(),
        Some(a) if a > 0.0 => format!(" | {a:.0} m ahead of the group"),
        Some(a) => format!(" | {:.0} m behind the group", -a),
    };
    format!("{riders} riders{place}")
}

/// The results table: finishers by time with the gap to the winner, riders
/// still on the course ("riding", km to go), DNFs; at most
/// [`RESULT_ROWS`] plus the local rider's row.
pub fn result_rows(
    kind: &EventKind,
    standings: &[Standing],
    own: Option<RiderId>,
) -> Vec<ResultRow> {
    let winner = standings.first().and_then(|s| s.finish_s);
    let length = kind.length_m().unwrap_or(0.0);
    let row = |s: &Standing| {
        let (position, time, gap_text) = match (s.finish_s, s.dnf) {
            (Some(t), _) => (
                s.position.to_string(),
                race_time(t),
                match winner {
                    Some(w) if s.position > 1 => gap(t - w),
                    _ => String::new(),
                },
            ),
            (None, true) => ("DNF".into(), "-".into(), String::new()),
            (None, false) => (
                s.position.to_string(),
                "riding".into(),
                format!("{:.2} km to go", (length - s.progress_m).max(0.0) / 1000.0),
            ),
        };
        ResultRow {
            position,
            name: s.name.clone(),
            time,
            gap: gap_text,
            is_local: Some(s.id) == own,
        }
    };
    let mut rows: Vec<ResultRow> = standings.iter().take(RESULT_ROWS).map(row).collect();
    if let Some(me) = standings
        .iter()
        .skip(RESULT_ROWS)
        .find(|s| Some(s.id) == own)
    {
        rows.push(row(me));
    }
    rows
}

/// The time trial's result line about the ghost.
pub fn outcome_line(outcome: GhostOutcome, time_s: f64) -> String {
    match outcome {
        GhostOutcome::First => "First effort on this course: it is your ghost now".into(),
        GhostOutcome::NewBest { previous_s } => format!(
            "New best! {:.1} s faster than your ghost ({})",
            previous_s - time_s,
            race_time(previous_s)
        ),
        GhostOutcome::Slower { best_s } => format!(
            "{:.1} s slower than your best ({}), the ghost stays",
            time_s - best_s,
            race_time(best_s)
        ),
        GhostOutcome::Failed => "The ghost could not be saved".into(),
    }
}

impl EventClient {
    /// `course`: the route id (for ghosts).
    pub fn new(course: impl Into<String>) -> Self {
        Self {
            course: course.into(),
            ..Default::default()
        }
    }

    pub fn info(&self) -> Option<&EventInfo> {
        self.info.as_ref()
    }

    /// The time trial's ghost key.
    fn key(&self) -> Option<GhostKey> {
        let info = self.info.as_ref()?;
        match info.kind {
            EventKind::TimeTrial { length_m, .. } => Some(GhostKey {
                course: self.course.clone(),
                length_m: length_m.round() as u32,
            }),
            _ => None,
        }
    }

    /// A new event state from the server at client time `now_s`. Returns
    /// true when a new event started (the own rider was moved to the start
    /// formation: the steering starts again from there).
    pub fn on_state(&mut self, info: Option<EventInfo>, own: Option<RiderId>, now_s: f64) -> bool {
        let new = match (&self.info, &info) {
            (_, None) => false,
            (Some(old), Some(i)) => old.id != i.id,
            (None, Some(_)) => true,
        };
        if new {
            self.outcome = None;
            self.results_since = None;
            self.finished_trace = None;
            self.trace = None;
            // Load the ghost again: the best may have changed.
            self.ghost = None;
        }
        self.info = info;
        let Some(info) = &self.info else {
            self.trace = None;
            return false;
        };
        if self.trace.is_none()
            && self.completed != Some(info.id)
            && matches!(info.kind, EventKind::TimeTrial { .. })
        {
            self.trace = Some((info.id, GhostTrace::default()));
        }
        // The own finish: complete the trace, show the results.
        let mine = own.and_then(|id| info.standings.iter().find(|s| s.id == id));
        let own_done = mine.is_some_and(|s| s.finish_s.is_some() || s.dnf);
        if (own_done || info.phase == EventPhase::Finished)
            && info.kind.length_m().is_some()
            && self.results_since.is_none_or(|(id, _)| id != info.id)
        {
            self.results_since = Some((info.id, now_s));
        }
        if let Some(t) = mine.and_then(|s| s.finish_s)
            && let Some(key) = self.key()
            && let Some((id, mut trace)) = self.trace.take()
            && id == info.id
        {
            trace.push(t, info.kind.length_m().unwrap_or(0.0));
            self.finished_trace = Some((key, trace));
            self.completed = Some(id);
        }
        new
    }

    /// The own rider's authoritative distance at server tick `tick`:
    /// recorded for the ghost while a time trial runs.
    pub fn on_own_snapshot(&mut self, tick: u64, distance: f64) {
        let Some(info) = &self.info else { return };
        let Some((id, trace)) = &mut self.trace else {
            return;
        };
        if *id != info.id || info.phase != EventPhase::Running {
            return;
        }
        let t = info.race_time_s(tick as f64);
        let p = distance - info.start_line_m;
        if t >= 0.0
            && trace
                .samples()
                .last()
                .is_none_or(|l| t - l.0 >= GHOST_SAMPLE_S - 1e-9)
        {
            trace.push(t, p.max(0.0));
        }
    }

    /// Which part of the event the own rider is in, for the ride's FIT laps
    /// (spec §2.8): (event id, 0 before the start, 1… the lap being
    /// ridden, laps + 1 after the own finish or the event's end). `None`
    /// without an event or when the own rider is not in it.
    pub fn lap_segment(&self, own: Option<RiderId>) -> Option<(u32, u32)> {
        let info = self.info.as_ref()?;
        let mine = info.standings.iter().find(|s| Some(s.id) == own)?;
        let laps = info.kind.laps().unwrap_or(1).max(1);
        let segment = if mine.finish_s.is_some() || mine.dnf || info.phase == EventPhase::Finished {
            laps + 1
        } else if info.phase < EventPhase::Running {
            0
        } else {
            match info.kind.length_m() {
                Some(length) if length > 0.0 => {
                    let lap_m = length / f64::from(laps);
                    (mine.progress_m.max(0.0) / lap_m).floor() as u32 + 1
                }
                _ => 1,
            }
            .min(laps)
        };
        Some((info.id, segment))
    }

    /// Whether the own rider is held at the start line (the countdown).
    pub fn holds(&self, own: Option<RiderId>) -> bool {
        self.info.as_ref().is_some_and(|i| {
            i.phase < EventPhase::Running
                && own.is_some_and(|id| i.standings.iter().any(|s| s.id == id))
        })
    }

    /// The ghost to load (a time trial whose ghost is not loaded yet).
    pub fn wants_ghost(&self) -> Option<GhostKey> {
        let key = self.key()?;
        match &self.ghost {
            Some((k, _)) if *k == key => None,
            _ => Some(key),
        }
    }

    /// The loaded ghost for `key` (`None`: there is none yet).
    pub fn set_ghost(&mut self, key: GhostKey, ghost: Option<GhostTrace>) {
        self.ghost = Some((key, ghost));
    }

    fn ghost(&self) -> Option<&GhostTrace> {
        let key = self.key()?;
        match &self.ghost {
            Some((k, Some(g))) if *k == key => Some(g),
            _ => None,
        }
    }

    /// A finished time trial's trace to save as the ghost if it is the best.
    pub fn take_finished_trace(&mut self) -> Option<(GhostKey, GhostTrace)> {
        self.finished_trace.take()
    }

    /// What saving the trace did (for the results panel).
    pub fn set_outcome(&mut self, outcome: GhostOutcome) {
        self.outcome = Some(outcome);
    }

    /// The ghost's course distance at server tick `tick` while a time
    /// trial runs (at the start line in the countdown), until the own
    /// finish.
    pub fn ghost_distance(&self, tick: f64, own: Option<RiderId>) -> Option<f64> {
        let info = self.info.as_ref()?;
        let ghost = self.ghost()?;
        if info.phase == EventPhase::Finished || self.own_finished(own) {
            return None;
        }
        let t = info.race_time_s(tick);
        Some(info.start_line_m + ghost.distance_at(t))
    }

    fn own_finished(&self, own: Option<RiderId>) -> bool {
        let Some(info) = &self.info else { return false };
        own.and_then(|id| info.standings.iter().find(|s| s.id == id))
            .is_some_and(|s| s.finish_s.is_some() || s.dnf)
    }

    /// Whether the results panel is up at client time `now_s`.
    pub fn results_showing(&self, now_s: f64) -> bool {
        match (self.results_since, &self.info) {
            (Some((id, since)), Some(info)) => {
                info.id == id && self.dismissed != Some(id) && now_s - since < RESULTS_SHOW_S
            }
            _ => false,
        }
    }

    /// Closes the results panel (Esc / A); false if none was up.
    pub fn dismiss_results(&mut self, now_s: f64) -> bool {
        if !self.results_showing(now_s) {
            return false;
        }
        self.dismissed = self.results_since.map(|(id, _)| id);
        self.dismissed_at = Some(now_s);
        true
    }

    /// Whether the results panel takes the input at `now_s`: it is up, or
    /// was closed at this very time (this frame), so the key that closed
    /// it does nothing else.
    pub fn results_input(&self, now_s: f64) -> bool {
        self.results_showing(now_s) || self.dismissed_at == Some(now_s)
    }

    /// The HUD's view at client time `now_s` and (estimated) server tick
    /// `tick`; `own_distance` is the predicted own distance, `others` the
    /// other riders' distances (group ride).
    pub fn view(
        &self,
        now_s: f64,
        tick: f64,
        own: Option<RiderId>,
        own_distance: f64,
        others: &[f64],
    ) -> Option<EventView> {
        let info = self.info.as_ref()?;
        let t = info.race_time_s(tick);
        let progress = own_distance - info.start_line_m;
        let mut view = EventView {
            countdown: match info.phase {
                EventPhase::Scheduled | EventPhase::Countdown => countdown_text(-t.min(-1e-9)),
                EventPhase::Running => countdown_text(-t),
                EventPhase::Finished => None,
            },
            title: title(&info.kind),
            ..Default::default()
        };
        let mine = own.and_then(|id| info.standings.iter().find(|s| s.id == id));
        match info.kind {
            EventKind::GroupRide { .. } => {
                let ahead = (!others.is_empty())
                    .then(|| own_distance - others.iter().sum::<f64>() / others.len() as f64);
                view.lines.push(group_line(others.len() + 1, ahead));
            }
            EventKind::Race { .. } | EventKind::TimeTrial { .. } => {
                let is_tt = matches!(info.kind, EventKind::TimeTrial { .. });
                if let Some(me) = mine
                    && let Some(time) = me.finish_s
                {
                    view.lines.push(if is_tt {
                        format!("Finished in {}", race_time(time))
                    } else {
                        format!(
                            "Finished P{}/{} in {}",
                            me.position,
                            info.standings.len(),
                            race_time(time)
                        )
                    });
                } else if mine.is_some_and(|m| m.dnf) {
                    view.lines.push("DNF (over the time limit)".into());
                } else if info.phase < EventPhase::Running {
                    // On the start line: the field, no times yet.
                    let n = info.standings.len();
                    view.lines.push(if is_tt {
                        "Alone on the start line".into()
                    } else {
                        format!("{n} riders on the start line")
                    });
                    if is_tt {
                        view.lines.push(match self.ghost() {
                            Some(g) => format!("Ghost: your best {}", race_time(g.duration_s())),
                            None => "No ghost yet: this effort becomes it".into(),
                        });
                    }
                } else {
                    if !is_tt
                        && let Some(line) = own.and_then(|id| position_line(&info.standings, id))
                    {
                        view.lines.push(line);
                    }
                    let mut line = format!("Time {}", race_time(t.max(0.0)));
                    if let Some(go) = to_go(&info.kind, progress) {
                        line.push_str(" | ");
                        line.push_str(&go);
                    }
                    view.lines.push(line);
                    if is_tt {
                        view.lines.push(match self.ghost() {
                            Some(g) if t > 0.0 => {
                                ghost_line(g.gap_s(t, progress.max(0.0)).unwrap_or(0.0))
                            }
                            Some(g) => format!("Ghost: your best {}", race_time(g.duration_s())),
                            None => "No ghost yet: this effort becomes it".into(),
                        });
                    }
                }
                if self.results_showing(now_s) {
                    let mut footer = Vec::new();
                    if is_tt
                        && let (Some(o), Some(t)) = (self.outcome, mine.and_then(|m| m.finish_s))
                    {
                        footer.push(outcome_line(o, t));
                    }
                    footer.push("Esc / A: close".into());
                    view.results = Some(ResultsView {
                        title: format!(
                            "{} - {}",
                            title(&info.kind),
                            if info.phase == EventPhase::Finished {
                                "results"
                            } else {
                                "provisional results"
                            }
                        ),
                        rows: result_rows(&info.kind, &info.standings, own),
                        footer,
                    });
                }
            }
        }
        Some(view)
    }
}

/// Server tick now, estimated from the last snapshot (`tick` at client
/// time `at_s`); at most half a second ahead of it.
pub fn estimate_tick(tick: u64, at_s: f64, now_s: f64) -> f64 {
    tick as f64 + ((now_s - at_s).clamp(0.0, 0.5) / SIM_DT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_sim::event::COUNTDOWN_TICKS;

    fn standing(
        id: u32,
        pos: u32,
        progress: f64,
        finish: Option<f64>,
        gap: Option<f64>,
    ) -> Standing {
        Standing {
            id: RiderId(id),
            name: format!("R{id}"),
            position: pos,
            progress_m: progress,
            finish_s: finish,
            dnf: false,
            gap_ahead_s: gap,
        }
    }

    fn info(kind: EventKind, phase: EventPhase, standings: Vec<Standing>) -> EventInfo {
        EventInfo {
            id: 1,
            kind,
            phase,
            countdown_tick: 0,
            go_tick: COUNTDOWN_TICKS,
            server_tick: 0,
            start_line_m: 100.0,
            finished_tick: None,
            standings,
            collisions: ocw_sim::collide::Mode::Off,
        }
    }

    const RACE: EventKind = EventKind::Race {
        length_m: 8000.0,
        laps: Some(2),
    };
    const TT: EventKind = EventKind::TimeTrial {
        length_m: 1000.0,
        laps: None,
    };

    #[test]
    fn lap_segments_for_the_fit_laps() {
        let own = Some(RiderId(1));
        let mut c = EventClient::new("loop");
        assert_eq!(c.lap_segment(own), None);
        let at = |c: &mut EventClient, phase, progress, finish| {
            c.on_state(
                Some(info(
                    RACE,
                    phase,
                    vec![standing(1, 1, progress, finish, None)],
                )),
                own,
                0.0,
            );
            c.lap_segment(own)
        };
        assert_eq!(at(&mut c, EventPhase::Countdown, 0.0, None), Some((1, 0)));
        assert_eq!(at(&mut c, EventPhase::Running, 10.0, None), Some((1, 1)));
        assert_eq!(at(&mut c, EventPhase::Running, 4001.0, None), Some((1, 2)));
        // Past the line but not yet finished by the server: still lap 2.
        assert_eq!(at(&mut c, EventPhase::Running, 8001.0, None), Some((1, 2)));
        assert_eq!(
            at(&mut c, EventPhase::Running, 8001.0, Some(600.0)),
            Some((1, 3))
        );
        // Not an entrant: no laps.
        assert_eq!(c.lap_segment(Some(RiderId(9))), None);
        // A time trial without laps: before, riding, after.
        c.on_state(
            Some(info(
                TT,
                EventPhase::Running,
                vec![standing(1, 1, 500.0, None, None)],
            )),
            own,
            0.0,
        );
        assert_eq!(c.lap_segment(own), Some((1, 1)));
    }

    #[test]
    fn times_and_gaps_format() {
        assert_eq!(race_time(612.34), "10:12.3");
        assert_eq!(race_time(3723.45), "1:02:03.5");
        assert_eq!(race_time(-1.0), "0:00.0");
        assert_eq!(gap(4.21), "+4.2 s");
        assert_eq!(gap(-0.8), "-0.8 s");
        assert_eq!(gap(64.2), "+1:04.2");
        assert_eq!(gap(59.99), "+1:00.0");
    }

    #[test]
    fn countdown_says_get_ready_then_3_2_1_go() {
        let texts: Vec<Option<String>> = [5.0, 3.5, 3.0, 2.2, 1.0, 0.01, 0.0, -1.0, -1.6]
            .iter()
            .map(|&r| countdown_text(r))
            .collect();
        let s = |x: &str| Some(x.to_string());
        assert_eq!(
            texts,
            [
                s("Get ready"),
                s("Get ready"),
                s("3"),
                s("3"),
                s("1"),
                s("1"),
                s("GO!"),
                s("GO!"),
                None
            ]
        );
    }

    #[test]
    fn race_lines_show_position_gap_and_distance_to_go() {
        let st = vec![
            standing(5, 1, 3000.0, None, None),
            standing(2, 2, 2980.0, None, Some(2.0)),
            standing(1, 3, 2950.0, None, Some(4.21)),
        ];
        assert_eq!(
            position_line(&st, RiderId(1)).unwrap(),
            "P3/3  +4.2 s to R2"
        );
        assert_eq!(
            position_line(&st, RiderId(5)).unwrap(),
            "P1/3  leading by 2.0 s"
        );
        assert_eq!(position_line(&st, RiderId(9)), None);
        assert_eq!(to_go(&RACE, 2950.0).unwrap(), "Lap 1/2 | 5.05 km to go");
        assert_eq!(to_go(&RACE, 4100.0).unwrap(), "Lap 2/2 | 3.90 km to go");
        assert_eq!(to_go(&RACE, 9000.0).unwrap(), "Lap 2/2 | 0.00 km to go");
        assert_eq!(to_go(&TT, -5.0).unwrap(), "1.00 km to go");
        assert_eq!(to_go(&EventKind::GroupRide { target_w: 1.0 }, 0.0), None);
        // The whole view while racing (2 s after GO).
        let mut c = EventClient::new("loop");
        c.on_state(
            Some(info(RACE, EventPhase::Running, st)),
            Some(RiderId(1)),
            0.0,
        );
        let v = c
            .view(
                0.0,
                COUNTDOWN_TICKS as f64 + 50.0,
                Some(RiderId(1)),
                3050.0,
                &[],
            )
            .unwrap();
        assert_eq!(v.title, "Race - 2 laps (8.0 km)");
        assert_eq!(v.countdown.as_deref(), Some("GO!"));
        assert_eq!(
            v.lines,
            [
                "P3/3  +4.2 s to R2",
                "Time 0:01.0 | Lap 1/2 | 5.05 km to go"
            ]
        );
        assert!(v.results.is_none());
    }

    #[test]
    fn countdown_holds_the_entrants_only() {
        let mut c = EventClient::new("loop");
        let i = info(
            RACE,
            EventPhase::Countdown,
            vec![standing(1, 1, 0.0, None, None)],
        );
        assert!(
            c.on_state(Some(i.clone()), Some(RiderId(1)), 0.0),
            "a new event"
        );
        assert!(
            !c.on_state(Some(i.clone()), Some(RiderId(1)), 0.1),
            "the same"
        );
        assert!(c.holds(Some(RiderId(1))));
        assert!(!c.holds(Some(RiderId(2))), "joined later: not in it");
        let v = c.view(0.0, 10.0, Some(RiderId(1)), 100.0, &[]).unwrap();
        assert_eq!(v.countdown.as_deref(), Some("Get ready"));
        assert_eq!(v.lines, ["1 riders on the start line"]);
        let v = c
            .view(
                0.0,
                COUNTDOWN_TICKS as f64 - 60.0,
                Some(RiderId(1)),
                100.0,
                &[],
            )
            .unwrap();
        assert_eq!(v.countdown.as_deref(), Some("2"));
        c.on_state(
            Some(info(
                RACE,
                EventPhase::Running,
                vec![standing(1, 1, 0.0, None, None)],
            )),
            Some(RiderId(1)),
            1.0,
        );
        assert!(!c.holds(Some(RiderId(1))));
        // Cancelled.
        c.on_state(None, Some(RiderId(1)), 2.0);
        assert!(c.view(2.0, 0.0, Some(RiderId(1)), 0.0, &[]).is_none());
        assert!(!c.holds(Some(RiderId(1))));
    }

    #[test]
    fn results_rows_and_panel_until_dismissed_or_timeout() {
        let mut dnf = standing(4, 4, 500.0, None, None);
        dnf.dnf = true;
        let st = vec![
            standing(2, 1, 8000.0, Some(600.0), None),
            standing(1, 2, 8000.0, Some(604.3), Some(4.3)),
            standing(3, 3, 7000.0, None, Some(80.0)),
            dnf,
        ];
        let rows = result_rows(&RACE, &st, Some(RiderId(1)));
        let cells: Vec<[&str; 4]> = rows
            .iter()
            .map(|r| {
                [
                    r.position.as_str(),
                    r.name.as_str(),
                    r.time.as_str(),
                    r.gap.as_str(),
                ]
            })
            .collect();
        assert_eq!(
            cells,
            [
                ["1", "R2", "10:00.0", ""],
                ["2", "R1", "10:04.3", "+4.3 s"],
                ["3", "R3", "riding", "1.00 km to go"],
                ["DNF", "R4", "-", ""],
            ]
        );
        assert!(rows[1].is_local && !rows[0].is_local);
        // Long fields: the top ten and the local rider.
        let many: Vec<Standing> = (1..=20)
            .map(|i| standing(i, i, 8000.0, Some(600.0 + f64::from(i)), None))
            .collect();
        let rows = result_rows(&RACE, &many, Some(RiderId(15)));
        assert_eq!(rows.len(), RESULT_ROWS + 1);
        assert_eq!(rows.last().unwrap().position, "15");
        // The panel appears with the own finish and closes on Esc / A ...
        let mut c = EventClient::new("loop");
        c.on_state(
            Some(info(RACE, EventPhase::Running, st.clone())),
            Some(RiderId(1)),
            50.0,
        );
        assert!(c.results_showing(51.0));
        let v = c.view(51.0, 1e5, Some(RiderId(1)), 0.0, &[]).unwrap();
        let r = v.results.unwrap();
        assert!(r.title.ends_with("provisional results"), "{}", r.title);
        assert_eq!(v.lines, ["Finished P2/4 in 10:04.3"]);
        assert!(c.dismiss_results(52.0));
        assert!(!c.results_showing(52.0));
        assert!(c.results_input(52.0), "the closing key does nothing else");
        assert!(!c.results_input(52.1));
        assert!(!c.dismiss_results(52.0));
        // ... and stays closed when the race is over.
        c.on_state(
            Some(info(RACE, EventPhase::Finished, st.clone())),
            Some(RiderId(1)),
            60.0,
        );
        assert!(!c.results_showing(61.0));
        // Not dismissed: it closes by itself.
        let mut c = EventClient::new("loop");
        c.on_state(
            Some(info(RACE, EventPhase::Finished, st)),
            Some(RiderId(9)),
            0.0,
        );
        assert!(c.results_showing(RESULTS_SHOW_S - 1.0));
        assert!(!c.results_showing(RESULTS_SHOW_S + 1.0));
    }

    #[test]
    fn group_ride_shows_the_place_in_the_group() {
        assert_eq!(group_line(5, Some(3.0)), "5 riders | in the group");
        assert_eq!(
            group_line(5, Some(40.0)),
            "5 riders | 40 m ahead of the group"
        );
        assert_eq!(
            group_line(5, Some(-22.4)),
            "5 riders | 22 m behind the group"
        );
        assert_eq!(group_line(1, None), "1 riders");
        let mut c = EventClient::new("loop");
        c.on_state(
            Some(info(
                EventKind::GroupRide { target_w: 150.0 },
                EventPhase::Running,
                vec![],
            )),
            Some(RiderId(1)),
            0.0,
        );
        let v = c
            .view(0.0, 1e4, Some(RiderId(1)), 100.0, &[150.0, 170.0])
            .unwrap();
        assert_eq!(v.title, "Group ride - 150 W");
        assert_eq!(v.lines, ["3 riders | 60 m behind the group"]);
        assert!(!c.holds(Some(RiderId(1))) && c.wants_ghost().is_none());
    }

    #[test]
    fn time_trial_records_the_trace_and_races_the_ghost() {
        let own = Some(RiderId(1));
        let mut c = EventClient::new("test hills/fwd");
        let go = COUNTDOWN_TICKS;
        c.on_state(
            Some(info(
                TT,
                EventPhase::Countdown,
                vec![standing(1, 1, 0.0, None, None)],
            )),
            own,
            0.0,
        );
        let key = c.wants_ghost().unwrap();
        assert_eq!(key.file_stem(), "test_hills_fwd_1000m");
        // The stored best: 1000 m in 100 s at an even pace.
        let best = GhostTrace::from_samples([(0.0, 0.0), (100.0, 1000.0)]);
        c.set_ghost(key.clone(), Some(best));
        assert!(c.wants_ghost().is_none());
        // In the countdown the ghost waits at the line.
        assert_eq!(c.ghost_distance(10.0, own), Some(100.0));
        let v = c.view(0.0, 10.0, own, 100.0, &[]).unwrap();
        assert_eq!(v.lines.last().unwrap(), "Ghost: your best 1:40.0");
        // Riding: 20 s after GO the ghost is at 200 m; at 180 m we are 2 s behind.
        c.on_state(
            Some(info(
                TT,
                EventPhase::Running,
                vec![standing(1, 1, 0.0, None, None)],
            )),
            own,
            5.0,
        );
        for k in 0..=1000u64 {
            // 9 m/s, sampled every snapshot (5 ticks).
            let tick = go + k * 5;
            c.on_own_snapshot(tick, 100.0 + 9.0 * (k * 5) as f64 * SIM_DT);
        }
        let t20 = go as f64 + 20.0 / SIM_DT;
        assert_eq!(c.ghost_distance(t20, own), Some(300.0));
        let v = c.view(25.0, t20, own, 280.0, &[]).unwrap();
        assert_eq!(v.lines[0], "Time 0:20.0 | 0.82 km to go");
        assert_eq!(v.lines[1], "Ghost +2.0 s (behind)");
        // Finished after 111.1 s: the trace is complete, the ghost gone.
        c.on_state(
            Some(info(
                TT,
                EventPhase::Finished,
                vec![standing(1, 1, 1000.0, Some(111.1), None)],
            )),
            own,
            120.0,
        );
        let (k, trace) = c.take_finished_trace().unwrap();
        assert_eq!(k, key);
        assert_eq!(trace.samples().last(), Some(&(111.1, 1000.0)));
        assert!((trace.samples()[1].0 - GHOST_SAMPLE_S).abs() < 1e-9, "2 Hz");
        assert!(c.take_finished_trace().is_none());
        // Once per event, also when the state comes again.
        c.on_state(
            Some(info(
                TT,
                EventPhase::Finished,
                vec![standing(1, 1, 1000.0, Some(111.1), None)],
            )),
            own,
            120.5,
        );
        assert!(c.take_finished_trace().is_none());
        assert_eq!(c.ghost_distance(t20, own), None);
        c.set_outcome(GhostOutcome::Slower { best_s: 100.0 });
        let r = c
            .view(121.0, 1e5, own, 1100.0, &[])
            .unwrap()
            .results
            .unwrap();
        assert_eq!(r.rows.len(), 1);
        assert_eq!(
            r.footer[0],
            "11.1 s slower than your best (1:40.0), the ghost stays"
        );
        assert_eq!(
            outcome_line(GhostOutcome::NewBest { previous_s: 100.0 }, 98.0),
            "New best! 2.0 s faster than your ghost (1:40.0)"
        );
        // No ghost yet.
        let mut c = EventClient::new("x");
        c.on_state(
            Some(info(
                TT,
                EventPhase::Running,
                vec![standing(1, 1, 0.0, None, None)],
            )),
            own,
            0.0,
        );
        c.set_ghost(c.wants_ghost().unwrap(), None);
        let v = c.view(0.0, go as f64 + 50.0, own, 110.0, &[]).unwrap();
        assert_eq!(
            v.lines.last().unwrap(),
            "No ghost yet: this effort becomes it"
        );
        assert_eq!(c.ghost_distance(go as f64, own), None);
    }

    #[test]
    fn server_tick_estimate_is_capped() {
        assert!((estimate_tick(100, 10.0, 10.1) - 105.0).abs() < 1e-9);
        assert_eq!(estimate_tick(100, 10.0, 15.0), 125.0);
        assert_eq!(estimate_tick(100, 10.0, 9.0), 100.0);
    }
}
