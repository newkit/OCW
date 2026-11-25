//! How late a snapshot arrives, in physics ticks (spec 6.3, 6.5).
//!
//! The server sends a snapshot every fifth tick (10 Hz), but the client
//! only reads its messages once per rendered frame. At 60 Hz a snapshot
//! waits up to 17 ms, at 25 Hz (a slow machine, the 25 fps `--capture`
//! recordings) up to 40 ms – two physics ticks – and the wait changes from
//! snapshot to snapshot with the beat of the frames against the 10 Hz.
//! Taken as fresh, every late snapshot pulled the own prediction back by
//! up to a tick's ride (16 cm at 30 km/h) and stretched or squeezed the
//! others' interpolation between two snapshots (80 / 120 ms instead of
//! 100 ms: their speed jumping by ±20 % at 10 Hz) – the rider and the
//! others jumped.
//!
//! [`ArrivalClock`] counts the client's prediction steps and compares
//! them with the snapshot's server tick: the freshest snapshot of the last
//! [`WINDOW_S`] is "on time", and any other is late by the ticks it lags
//! behind that one. The prediction extrapolates a snapshot by its lateness
//! (plus the network's age), and the others' interpolation places it at
//! the time it would have arrived on time.
//!
//! Where the transport stamps each message on arrival (the browser's
//! network worker, spec 14 Phase 10, 10G *B3 worker*), the client knows
//! how many steps it had taken when the snapshot came, not only when it
//! read it: [`ArrivalClock::late_ticks_stamped`] keeps that lead (a
//! fraction of a step too) as the snapshot's in the window, so a message
//! that waited for the page sets no later baseline and counts as late by
//! exactly the steps taken since it came.

use std::collections::VecDeque;

/// The window in which the freshest arrival sets "on time" (s).
pub const WINDOW_S: f64 = 2.0;
/// Lateness beyond this many ticks is not a late snapshot but a new
/// clock (a new session, a world switch, a long stall): the window starts
/// again.
pub const MAX_LATE_TICKS: i64 = 25;

/// Prediction steps against server ticks (see the module docs).
#[derive(Debug, Clone, Default)]
pub struct ArrivalClock {
    steps: u64,
    /// (arrival time, prediction steps − server tick) of the recent
    /// snapshots (fractional where the arrival was stamped).
    window: VecDeque<(f64, f64)>,
}

impl ArrivalClock {
    /// One prediction step (a fixed physics tick of the client).
    pub fn step(&mut self) {
        self.steps += 1;
    }

    /// A snapshot of `server_tick` arrived at `now_s`: how many ticks it
    /// is later than the freshest one of the last [`WINDOW_S`].
    pub fn late_ticks(&mut self, server_tick: u64, now_s: f64) -> u32 {
        self.late_ticks_stamped(server_tick, now_s, 0.0)
    }

    /// The same for a snapshot stamped on arrival: `steps_since` (≥ 0,
    /// fractional) prediction steps were taken between its arrival and
    /// now. Its lead at arrival goes into the window; the lateness is the
    /// steps now against the freshest lead, rounded to whole steps.
    pub fn late_ticks_stamped(&mut self, server_tick: u64, now_s: f64, steps_since: f64) -> u32 {
        let now_lead = self.steps as f64 - server_tick as f64;
        let lead = now_lead - steps_since.max(0.0);
        while self
            .window
            .front()
            .is_some_and(|(at, _)| *at < now_s - WINDOW_S || *at > now_s)
        {
            self.window.pop_front();
        }
        let fresh = self
            .window
            .iter()
            .map(|(_, l)| *l)
            .fold(f64::INFINITY, f64::min);
        self.window.push_back((now_s, lead));
        let fresh = fresh.min(lead);
        if now_lead - fresh > MAX_LATE_TICKS as f64 {
            // A new clock: start again from this snapshot.
            self.window.clear();
            self.window.push_back((now_s, lead));
            return (now_lead - lead).round().max(0.0) as u32;
        }
        (now_lead - fresh).round().max(0.0) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 25 Hz frames, two steps each, snapshots every fifth tick read at
    /// the end of the frame: some arrive fresh, some a tick late.
    #[test]
    fn snapshots_read_a_tick_late_are_found() {
        let mut c = ArrivalClock::default();
        let mut tick = 0u64;
        let mut lates = Vec::new();
        for frame in 0..50 {
            let mut pending = None;
            for _ in 0..2 {
                tick += 1;
                c.step();
                if tick.is_multiple_of(5) {
                    pending = Some(tick);
                }
            }
            if let Some(t) = pending {
                lates.push(c.late_ticks(t, f64::from(frame) * 0.04));
            }
        }
        // Ticks 5, 15, … come on the first step of their frame: one late.
        assert!(lates.iter().all(|l| *l <= 1), "{lates:?}");
        assert!(lates.contains(&0) && lates.contains(&1), "{lates:?}");
    }

    #[test]
    fn on_time_snapshots_are_not_late() {
        let mut c = ArrivalClock::default();
        for i in 1..=100u64 {
            c.step();
            if i.is_multiple_of(5) {
                assert_eq!(c.late_ticks(i + 7, i as f64 * 0.02), 0);
            }
        }
    }

    /// Stamped arrivals (10G *B3 worker*): a snapshot that waited two
    /// frames for the page is late by the steps since it came, and sets
    /// no later baseline for the next ones.
    #[test]
    fn stamped_snapshots_are_late_by_the_steps_since_they_came() {
        let mut c = ArrivalClock::default();
        for _ in 0..10 {
            c.step();
        }
        // Came on time, read at once.
        assert_eq!(c.late_ticks_stamped(10, 0.0, 0.0), 0);
        for _ in 0..5 {
            c.step();
        }
        // Came on time (tick 15 at step 15) but read 4 steps later.
        for _ in 0..4 {
            c.step();
        }
        assert_eq!(c.late_ticks_stamped(15, 0.1, 4.0), 4);
        // The window keeps the lead it came with (0), not the 4 it was
        // read with. A snapshot ahead of the prediction (tick 25 at step
        // 21, came 0.4 steps ago) is the freshest and not late.
        c.step();
        c.step();
        assert_eq!(c.late_ticks_stamped(25, 0.2, 0.4), 0);
        // A stamp a fraction of a step late rounds to the nearest step.
        for _ in 0..5 {
            c.step();
        }
        assert_eq!(c.late_ticks_stamped(30, 0.3, 1.4), 1);
        assert_eq!(c.late_ticks_stamped(30, 0.3, 1.6), 2);
    }

    /// The client fell behind for good (its clock was clamped after a
    /// stall): the new lead is fresh at once.
    #[test]
    fn a_smaller_lead_is_fresh_at_once() {
        let mut c = ArrivalClock::default();
        for _ in 0..10 {
            c.step();
        }
        assert_eq!(c.late_ticks(10, 0.0), 0);
        assert_eq!(c.late_ticks(30, 0.1), 0);
        c.step();
        assert_eq!(c.late_ticks(30, 0.2), 1);
    }

    /// A larger lead stays late only within the window, and a new clock
    /// (a server tick far behind) starts again.
    #[test]
    fn the_window_forgets_and_a_new_clock_restarts() {
        let mut c = ArrivalClock::default();
        assert_eq!(c.late_ticks(10, 0.0), 0);
        c.step();
        c.step();
        assert_eq!(c.late_ticks(10, 0.5), 2);
        assert_eq!(c.late_ticks(10, 3.0), 0, "the fresh one left the window");
        // A new session: server ticks from 0 again.
        for _ in 0..1000 {
            c.step();
        }
        assert_eq!(c.late_ticks(1, 3.1), 0);
        c.step();
        assert_eq!(c.late_ticks(2, 3.2), 0);
    }
}
