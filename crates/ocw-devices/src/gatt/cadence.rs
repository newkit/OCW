//! Cadence from cumulative crank revolutions (Cycling Power, CSC).
//!
//! Sensors report a 16-bit revolution counter and the 16-bit time of the last
//! crank event (1/1024 s); both wrap around. Cadence is the change between two
//! notifications that carry a new event.

use std::time::Duration;

/// Crank revolution data as sent by Cycling Power and CSC sensors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CrankData {
    pub revolutions: u16,
    /// Time of the last crank event in 1/1024 s.
    pub event_time: u16,
}

/// Without a new crank event for this long, the rider has stopped pedaling.
pub const STOP_TIMEOUT: Duration = Duration::from_millis(2500);
/// Readings above this are glitches (e.g. a counter reset), not cadence.
const MAX_PLAUSIBLE_RPM: f64 = 250.0;

/// Turns successive [`CrankData`] into cadence.
#[derive(Debug, Clone, Default)]
pub struct CadenceCalculator {
    last: Option<CrankData>,
    /// Receive time of the last notification with a new crank event.
    last_event_at: Option<Duration>,
    cadence: Option<f64>,
}

impl CadenceCalculator {
    /// Feeds one notification received at `now` (any monotonic clock).
    /// Returns the current cadence in rpm, or `None` while unknown.
    pub fn update(&mut self, data: CrankData, now: Duration) -> Option<f64> {
        let Some(prev) = self.last else {
            self.last = Some(data);
            self.last_event_at = Some(now);
            return None;
        };
        if data.event_time == prev.event_time {
            // Repeated notification without a new crank event.
            if self
                .last_event_at
                .is_some_and(|t| now.saturating_sub(t) >= STOP_TIMEOUT)
            {
                self.cadence = Some(0.0);
            }
            return self.cadence;
        }
        let revs = data.revolutions.wrapping_sub(prev.revolutions);
        let ticks = data.event_time.wrapping_sub(prev.event_time);
        self.last = Some(data);
        self.last_event_at = Some(now);
        let rpm = f64::from(revs) * 60.0 * 1024.0 / f64::from(ticks);
        if rpm <= MAX_PLAUSIBLE_RPM {
            self.cadence = Some(rpm);
        }
        self.cadence
    }

    /// Current cadence without new data (also applies the stop timeout).
    pub fn current(&mut self, now: Duration) -> Option<f64> {
        if self
            .last_event_at
            .is_some_and(|t| now.saturating_sub(t) >= STOP_TIMEOUT)
            && self.cadence.is_some()
        {
            self.cadence = Some(0.0);
        }
        self.cadence
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(v: u64) -> Duration {
        Duration::from_millis(v)
    }

    #[test]
    fn computes_cadence_from_revolution_deltas() {
        let mut c = CadenceCalculator::default();
        assert_eq!(
            c.update(
                CrankData {
                    revolutions: 10,
                    event_time: 1024
                },
                ms(0)
            ),
            None
        );
        // 2 revolutions in 1 s (1024 ticks) = 120 rpm.
        let rpm = c.update(
            CrankData {
                revolutions: 12,
                event_time: 2048,
            },
            ms(1000),
        );
        assert_eq!(rpm, Some(120.0));
        // 1 revolution in 2/3 s = 90 rpm.
        let rpm = c.update(
            CrankData {
                revolutions: 13,
                event_time: 2048 + 683,
            },
            ms(1700),
        );
        assert!((rpm.unwrap() - 89.96).abs() < 0.01);
    }

    #[test]
    fn handles_counter_and_timer_wraparound() {
        let mut c = CadenceCalculator::default();
        c.update(
            CrankData {
                revolutions: 65_535,
                event_time: 65_000,
            },
            ms(0),
        );
        // +1 revolution, +1024 ticks across the u16 wrap of both counters.
        let rpm = c.update(
            CrankData {
                revolutions: 0,
                event_time: 488,
            },
            ms(1000),
        );
        assert_eq!(rpm, Some(60.0));
    }

    #[test]
    fn repeated_event_keeps_cadence_then_drops_to_zero() {
        let mut c = CadenceCalculator::default();
        c.update(
            CrankData {
                revolutions: 0,
                event_time: 0,
            },
            ms(0),
        );
        c.update(
            CrankData {
                revolutions: 1,
                event_time: 683,
            },
            ms(667),
        );
        let same = CrankData {
            revolutions: 1,
            event_time: 683,
        };
        // Sensors repeat the last event at ~1-4 Hz while the crank turns slowly.
        assert!(c.update(same, ms(1000)).unwrap() > 80.0);
        assert_eq!(c.update(same, ms(667 + 2500)), Some(0.0));
        // current() applies the timeout without new data, too.
        let mut c2 = CadenceCalculator::default();
        c2.update(
            CrankData {
                revolutions: 0,
                event_time: 0,
            },
            ms(0),
        );
        c2.update(
            CrankData {
                revolutions: 1,
                event_time: 1024,
            },
            ms(1000),
        );
        assert_eq!(c2.current(ms(2000)), Some(60.0));
        assert_eq!(c2.current(ms(4000)), Some(0.0));
    }

    #[test]
    fn implausible_jumps_are_ignored() {
        let mut c = CadenceCalculator::default();
        c.update(
            CrankData {
                revolutions: 0,
                event_time: 0,
            },
            ms(0),
        );
        c.update(
            CrankData {
                revolutions: 1,
                event_time: 1024,
            },
            ms(1000),
        );
        // Counter reset by the sensor: 500 revolutions in 1 s.
        let rpm = c.update(
            CrankData {
                revolutions: 501,
                event_time: 2048,
            },
            ms(2000),
        );
        assert_eq!(rpm, Some(60.0));
    }
}
