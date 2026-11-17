//! The frame loop's health (spec §14 Phase 11, 11I, v4.64): what every
//! OpenXR frame call costs, how many frames reach the headset per second,
//! the longest gap between two `xrEndFrame`s, the app's, the render
//! thread's and the GPU's time – and, at once, every call that failed
//! (which one, after how long), every call slower than
//! [`LoopWatch::slow_ms`], a frame loop that stalls (no `xrEndFrame` for
//! [`LoopWatch::stall_ms`]) and a running session the runtime does not
//! show (frames flow, but it stays *synchronized*).
//!
//! The VR kiosk on Weela lost its session every minute (2026-11-17) and
//! its log could not say which call failed or how the loop ran before:
//! now each [`LoopWatch::every_s`] seconds one line tells, and the lines
//! of [`LoopWatch::take_lines`] go to the log. Pure: the binding passes
//! the times (seconds or milliseconds on any clock).

use crate::frame::{FrameCall, FrameFailure, FrameTiming, SubmitCounts, Submitted};

/// A running mean and maximum.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Stat {
    pub n: u32,
    sum: f64,
    pub max: f32,
}

impl Stat {
    pub fn add(&mut self, x: f32) {
        self.n += 1;
        self.sum += f64::from(x);
        self.max = self.max.max(x);
    }

    pub fn mean(&self) -> f32 {
        if self.n == 0 {
            0.0
        } else {
            (self.sum / f64::from(self.n)) as f32
        }
    }
}

/// Lines held back per window at most (a failing loop cannot flood the log).
const MAX_LINES: usize = 24;

#[derive(Debug, Clone, PartialEq)]
pub struct LoopWatch {
    /// Seconds per report line.
    pub every_s: f64,
    /// A single call slower than this (ms) is logged at once.
    pub slow_ms: f32,
    /// No `xrEndFrame` for this long (ms) while the loop runs: a stall.
    pub stall_ms: f32,
    /// A session that runs (frames ended) but is not shown for this long
    /// (s) is reported.
    pub unseen_s: f64,
    window_start_s: Option<f64>,
    waits: u64,
    window: SubmitCounts,
    /// All frames since the start.
    pub total: SubmitCounts,
    calls: [Stat; FrameCall::ALL.len()],
    app_ms: Stat,
    render_ms: Stat,
    gpu_ms: Stat,
    period_ms: f32,
    last_end_s: Option<f64>,
    max_end_gap_ms: f32,
    stalled_since_s: Option<f64>,
    /// The current session: when its frame loop started, frames ended in
    /// it, whether it was shown, whether its "not shown" was reported.
    session_start_s: Option<f64>,
    session_ends: u64,
    session_seen: bool,
    unseen_reported: bool,
    lines: Vec<String>,
    dropped_lines: u32,
    last_report: String,
}

impl Default for LoopWatch {
    fn default() -> Self {
        Self {
            every_s: 10.0,
            slow_ms: 100.0,
            stall_ms: 500.0,
            unseen_s: 10.0,
            window_start_s: None,
            waits: 0,
            window: SubmitCounts::default(),
            total: SubmitCounts::default(),
            calls: [Stat::default(); FrameCall::ALL.len()],
            app_ms: Stat::default(),
            render_ms: Stat::default(),
            gpu_ms: Stat::default(),
            period_ms: 0.0,
            last_end_s: None,
            max_end_gap_ms: 0.0,
            stalled_since_s: None,
            session_start_s: None,
            session_ends: 0,
            session_seen: false,
            unseen_reported: false,
            lines: Vec::new(),
            dropped_lines: 0,
            last_report: String::new(),
        }
    }
}

impl LoopWatch {
    fn line(&mut self, l: String) {
        if self.lines.len() < MAX_LINES {
            self.lines.push(l);
        } else {
            self.dropped_lines += 1;
        }
    }

    /// One call took `ms`.
    pub fn call(&mut self, c: FrameCall, ms: f32) {
        self.calls[c.index()].add(ms);
        if ms >= self.slow_ms {
            self.line(format!("VR loop: {} took {ms:.0} ms", c.name()));
        }
    }

    /// `xrWaitFrame` returned; `app_ms`: the app's own time since the last
    /// wait returned (not the wait itself).
    pub fn waited(&mut self, t: &FrameTiming, app_ms: Option<f32>) {
        self.waits += 1;
        if t.period_ns > 0 {
            self.period_ms = t.period_ns as f32 / 1e6;
        }
        if let Some(ms) = app_ms {
            self.app_ms.add(ms);
        }
    }

    /// The render thread's time from `xrBeginFrame` to the frame's
    /// finish: the render graph, the mirror's present.
    pub fn rendered(&mut self, ms: f32) {
        self.render_ms.add(ms);
    }

    /// The GPU finished the frame's copy `ms` after it was submitted (at
    /// most: wgpu reports it at its next poll).
    pub fn gpu_done(&mut self, ms: f32) {
        self.gpu_ms.add(ms);
    }

    /// A frame call on the main thread failed.
    pub fn failed(&mut self, f: &FrameFailure) {
        self.line(format!("VR frame: {}", f.line()));
    }

    /// A begun frame's end, at `now_s`.
    pub fn submitted(&mut self, r: &Result<Submitted, FrameFailure>, now_s: f64) {
        self.window.count(r);
        self.total.count(r);
        match r {
            Ok(_) => {
                if let Some(last) = self.last_end_s {
                    let gap = ((now_s - last) * 1000.0) as f32;
                    self.max_end_gap_ms = self.max_end_gap_ms.max(gap);
                }
                if let Some(since) = self.stalled_since_s.take() {
                    // `since`: the last frame before the gap.
                    let ms = (now_s - since) * 1000.0;
                    self.line(format!(
                        "VR loop: frames reach the runtime again after {ms:.0} ms"
                    ));
                }
                self.last_end_s = Some(now_s);
                self.session_ends += 1;
            }
            Err(f) => self.failed(f),
        }
    }

    /// A new session's frame loop starts (READY, `xrBeginSession`).
    pub fn session_started(&mut self, now_s: f64) {
        self.session_start_s = Some(now_s);
        self.session_ends = 0;
        self.session_seen = false;
        self.unseen_reported = false;
        self.last_end_s = None;
        self.stalled_since_s = None;
    }

    /// Each frame on the main thread: `looping` – the frame loop should
    /// run; `shown` – the runtime shows the session (VISIBLE / FOCUSED).
    pub fn check(&mut self, now_s: f64, looping: bool, shown: bool) {
        if !looping {
            self.last_end_s = None;
            self.stalled_since_s = None;
            self.session_start_s = None;
            return;
        }
        if self.session_start_s.is_none() {
            self.session_started(now_s);
        }
        self.session_seen |= shown;
        let since = self.last_end_s.or(self.session_start_s).unwrap_or(now_s);
        if self.stalled_since_s.is_none() && (now_s - since) * 1000.0 >= f64::from(self.stall_ms) {
            self.stalled_since_s = Some(since);
            self.line(format!(
                "VR loop: no frame reached the runtime for {:.0} ms (xrEndFrame), {} in this session",
                (now_s - since) * 1000.0,
                self.session_ends
            ));
        }
        if let Some(start) = self.session_start_s
            && !self.session_seen
            && !self.unseen_reported
            && now_s - start >= self.unseen_s
        {
            self.unseen_reported = true;
            self.line(format!(
                "VR loop: the session runs for {:.0} s with {} frames ended, but the runtime does not show it (synchronized, never visible)",
                now_s - start,
                self.session_ends
            ));
        }
    }

    /// The frames ended in the current session.
    pub fn session_frames(&self) -> u64 {
        self.session_ends
    }

    /// Whether the current session was shown at all.
    pub fn session_shown(&self) -> bool {
        self.session_seen
    }

    /// The lines for the log: the immediate ones, and every
    /// [`Self::every_s`] the report (then a new window starts).
    pub fn take_lines(&mut self, now_s: f64) -> Vec<String> {
        let start = *self.window_start_s.get_or_insert(now_s);
        let secs = now_s - start;
        if secs >= self.every_s {
            if self.waits > 0 || self.window != SubmitCounts::default() {
                let r = self.report(secs);
                self.last_report = r.clone();
                self.line(r);
            }
            if self.dropped_lines > 0 {
                let n = std::mem::take(&mut self.dropped_lines);
                self.lines.push(format!("VR loop: {n} more lines dropped"));
            }
            self.reset_window(now_s);
        }
        std::mem::take(&mut self.lines)
    }

    fn reset_window(&mut self, now_s: f64) {
        self.window_start_s = Some(now_s);
        self.waits = 0;
        self.window = SubmitCounts::default();
        self.calls = [Stat::default(); FrameCall::ALL.len()];
        self.app_ms = Stat::default();
        self.render_ms = Stat::default();
        self.gpu_ms = Stat::default();
        self.max_end_gap_ms = 0.0;
    }

    /// The last report (`vr status`).
    pub fn last_report(&self) -> &str {
        &self.last_report
    }

    /// The window's line: "VR loop 10 s: 72.0 waits/s, 71.8 shown/s, 0.2
    /// empty/s, 0 failed; period 13.9 ms; …".
    pub fn report(&self, secs: f64) -> String {
        let per_s = |n: u64| if secs > 0.0 { n as f64 / secs } else { 0.0 };
        let stat = |s: &Stat| format!("{:.1} (max {:.1})", s.mean(), s.max);
        let mut out = format!(
            "VR loop {secs:.0} s: {:.1} waits/s, {:.1} shown/s, {:.1} empty/s, {} failed; period {:.1} ms; longest gap between frames {:.0} ms; app {} ms, render {} ms, GPU done ≤ {} ms",
            per_s(self.waits),
            per_s(self.window.shown),
            per_s(self.window.empty),
            self.window.failed,
            self.period_ms,
            self.max_end_gap_ms,
            stat(&self.app_ms),
            stat(&self.render_ms),
            stat(&self.gpu_ms),
        );
        for c in FrameCall::ALL {
            let s = &self.calls[c.index()];
            if s.n > 0 && !matches!(c, FrameCall::PollEvent) {
                out.push_str(&format!("; {} {}", c.name(), stat(s)));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::XrError;

    fn timing() -> FrameTiming {
        FrameTiming {
            display_time_ns: 0,
            period_ns: 13_888_889,
            should_render: true,
        }
    }

    #[test]
    fn a_report_line_comes_every_window() {
        let mut w = LoopWatch::default();
        let period = 1.0 / 72.0;
        assert!(w.take_lines(0.0).is_empty());
        for i in 0..720 {
            let now = f64::from(i) * period;
            w.waited(&timing(), Some(9.0));
            w.call(FrameCall::WaitFrame, 4.0);
            w.call(FrameCall::EndFrame, 0.5);
            w.rendered(11.0);
            w.submitted(&Ok(Submitted::Shown), now);
            w.check(now, true, true);
            assert!(w.take_lines(now).is_empty(), "frame {i}");
        }
        let lines = w.take_lines(10.0);
        assert_eq!(lines.len(), 1, "{lines:?}");
        let l = &lines[0];
        assert!(
            l.starts_with("VR loop 10 s: 72.0 waits/s, 72.0 shown/s"),
            "{l}"
        );
        assert!(l.contains("period 13.9 ms"), "{l}");
        assert!(l.contains("longest gap between frames 14 ms"), "{l}");
        assert!(l.contains("xrWaitFrame 4.0 (max 4.0)"), "{l}");
        assert!(l.contains("app 9.0 (max 9.0) ms"), "{l}");
        assert_eq!(w.last_report(), l);
        assert_eq!(w.total.shown, 720);
        // A new window.
        assert!(w.take_lines(15.0).is_empty());
    }

    #[test]
    fn failures_and_slow_calls_are_logged_at_once() {
        let mut w = LoopWatch::default();
        w.call(FrameCall::WaitFrame, 2300.0);
        w.submitted(
            &Err(FrameFailure {
                call: FrameCall::EndFrame,
                error: XrError::SessionLost,
                ms: 1.5,
            }),
            1.0,
        );
        let lines = w.take_lines(1.0);
        assert_eq!(
            lines,
            [
                "VR loop: xrWaitFrame took 2300 ms",
                "VR frame: xrEndFrame failed after 1.5 ms: the VR session was lost"
            ]
        );
        assert_eq!(w.total.failed, 1);
    }

    /// A frame loop that runs but whose frames stop reaching the runtime
    /// is reported once, and again when they flow.
    #[test]
    fn a_stall_is_reported_once_and_its_end() {
        let mut w = LoopWatch::default();
        w.check(0.0, true, true);
        w.submitted(&Ok(Submitted::Shown), 0.0);
        w.check(0.4, true, true);
        assert!(w.take_lines(0.4).is_empty());
        w.check(0.6, true, true);
        w.check(0.9, true, true);
        let l = w.take_lines(0.9);
        assert_eq!(l.len(), 1);
        assert!(
            l[0].contains("no frame reached the runtime for 600 ms"),
            "{l:?}"
        );
        w.submitted(&Ok(Submitted::Shown), 2.0);
        let l = w.take_lines(2.0);
        assert!(l[0].contains("again after 2000 ms"), "{l:?}");
        // No frame loop (headset off): nothing to wait for.
        w.check(10.0, false, false);
        w.check(20.0, false, false);
        assert!(w.take_lines(5.0).is_empty());
    }

    /// Regression (2026-11-17, Weela): every second session after a
    /// reconnect stayed *synchronized* for a minute – the log only showed
    /// "ready". Now it says so, with the frames ended in it.
    #[test]
    fn a_session_the_runtime_never_shows_is_detected() {
        let mut w = LoopWatch::default();
        let period = 1.0 / 72.0;
        for i in 0..=(72 * 11) {
            let now = f64::from(i) * period;
            w.submitted(&Ok(Submitted::Empty), now);
            w.check(now, true, false);
        }
        let l = w.take_lines(11.0);
        assert!(
            l.iter()
                .any(|l| l.contains("does not show it") && l.contains("frames ended")),
            "{l:?}"
        );
        assert!(!w.session_shown());
        // Reported once per session; a new session starts over.
        w.check(12.0, true, false);
        assert!(
            w.take_lines(12.0)
                .iter()
                .all(|l| !l.contains("does not show"))
        );
        w.session_started(13.0);
        assert_eq!(w.session_frames(), 0);
        w.check(13.5, true, true);
        assert!(w.session_shown());
    }

    #[test]
    fn a_failing_loop_cannot_flood_the_log() {
        let mut w = LoopWatch::default();
        for _ in 0..1000 {
            w.call(FrameCall::EndFrame, 500.0);
        }
        let l = w.take_lines(0.0);
        assert_eq!(l.len(), MAX_LINES);
        let l = w.take_lines(11.0);
        assert!(l.iter().any(|l| l.contains("lines dropped")), "{l:?}");
    }
}
