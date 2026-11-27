//! The ANT+ bike-power pages (ANT+ Device Profile *Bicycle Power*, device
//! type 11):
//! - the **power-only page 0x10** – event count, pedal balance,
//!   instantaneous cadence, accumulated power, instantaneous power;
//! - the **torque pages** 0x11 (wheel torque: hub and wheel-based meters)
//!   and 0x12 (crank torque: crank and spider meters) – event count, ticks,
//!   instantaneous cadence, accumulated period (1/2048 s), accumulated
//!   torque (1/32 Nm). Power over the events since the last message is
//!   128π · Δtorque / Δperiod (W), the crank's cadence 60 · Δevents ·
//!   2048 / Δperiod (rpm), the wheel's revolutions per second 2048 ·
//!   Δevents / Δperiod; a new event with the same period is coasting (0 W);
//! - **calibration** (common page 0x01): the requests we send (manual zero
//!   0xAA, auto zero configuration 0xAB), the meter's answers (0xAC done,
//!   0xAF failed – each with the auto zero status and the zero offset) and
//!   its auto zero support page (0x12);
//! - the common pages 0x50 / 0x51 / 0x52 ([`super::common`]).
//!
//! A meter that sends the power-only page is read from it (the standard
//! page stays preferred: torque pages then give no power of their own,
//! for [`POWER_ONLY_FRESH`] after the last one); a meter that sends only
//! torque pages is read from those. Power is the average over the events
//! since the last message, as head units show it; no new event for
//! [`IDLE_AFTER`] is 0 W (coasting, or the meter lying still).

use std::time::Duration;

use super::common::{self, Common};
use super::hr::Battery;
use crate::Calibration;

/// The calibration page (requests and answers).
pub const PAGE_CALIBRATION: u8 = 0x01;
/// The power-only page.
pub const PAGE_POWER_ONLY: u8 = 0x10;
/// Wheel torque.
pub const PAGE_WHEEL_TORQUE: u8 = 0x11;
/// Crank torque.
pub const PAGE_CRANK_TORQUE: u8 = 0x12;
/// Common page: manufacturer information.
pub const PAGE_MANUFACTURER: u8 = common::PAGE_MANUFACTURER;
/// Common page: battery status.
pub const PAGE_BATTERY: u8 = common::PAGE_BATTERY;

/// Calibration ids (byte 1 of page 0x01).
pub mod cal {
    /// Host: a manual zero (general calibration request).
    pub const REQUEST: u8 = 0xAA;
    /// Host: auto zero on / off.
    pub const AUTO_ZERO_CONFIG: u8 = 0xAB;
    /// Meter: calibration done.
    pub const SUCCESS: u8 = 0xAC;
    /// Meter: calibration failed.
    pub const FAIL: u8 = 0xAF;
    /// Meter: auto zero support.
    pub const AUTO_ZERO_SUPPORT: u8 = 0x12;
}

/// No new power event for this long: 0 W.
pub const IDLE_AFTER: Duration = Duration::from_secs(3);
/// After a power-only page, torque pages give no power for this long.
pub const POWER_ONLY_FRESH: Duration = Duration::from_secs(5);
/// A crank cadence above this is a glitch (a counter reset), not cadence.
const MAX_PLAUSIBLE_RPM: f64 = 250.0;

/// What one message gave.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PowerReading {
    pub watts: f64,
    pub cadence_rpm: Option<f64>,
    /// Wheel torque meters: the wheel's revolutions per second.
    pub wheel_rps: Option<f64>,
}

impl PowerReading {
    fn new(watts: f64, cadence_rpm: Option<f64>) -> Self {
        Self {
            watts,
            cadence_rpm,
            wheel_rps: None,
        }
    }
}

/// One torque page's counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Torque {
    pub event: u8,
    pub ticks: u8,
    /// Instantaneous cadence (0xFF: none).
    pub cadence: u8,
    /// Accumulated period (1/2048 s).
    pub period: u16,
    /// Accumulated torque (1/32 Nm).
    pub torque: u16,
}

impl Torque {
    pub fn parse(p: &[u8; 8]) -> Self {
        Self {
            event: p[1],
            ticks: p[2],
            cadence: p[3],
            period: u16::from_le_bytes([p[4], p[5]]),
            torque: u16::from_le_bytes([p[6], p[7]]),
        }
    }

    /// The page (`page`: 0x11 or 0x12).
    pub fn page(&self, page: u8) -> [u8; 8] {
        let [p0, p1] = self.period.to_le_bytes();
        let [t0, t1] = self.torque.to_le_bytes();
        [page, self.event, self.ticks, self.cadence, p0, p1, t0, t1]
    }
}

/// Power (and the crank's cadence or the wheel's speed) between two torque
/// pages; `None`: no new event.
pub fn torque_reading(prev: Torque, cur: Torque, crank: bool) -> Option<PowerReading> {
    let events = cur.event.wrapping_sub(prev.event);
    if events == 0 {
        return None;
    }
    let period = cur.period.wrapping_sub(prev.period);
    let torque = cur.torque.wrapping_sub(prev.torque);
    let instant = (cur.cadence != 0xFF).then_some(f64::from(cur.cadence));
    if period == 0 {
        // Events without time: the crank (or wheel) stands, coasting.
        return Some(PowerReading {
            watts: 0.0,
            cadence_rpm: if crank { Some(0.0) } else { instant },
            wheel_rps: (!crank).then_some(0.0),
        });
    }
    let watts = 128.0 * std::f64::consts::PI * f64::from(torque) / f64::from(period);
    let per_s = f64::from(events) * 2048.0 / f64::from(period);
    Some(if crank {
        let rpm = per_s * 60.0;
        PowerReading::new(
            watts,
            if rpm <= MAX_PLAUSIBLE_RPM {
                Some(rpm)
            } else {
                instant
            },
        )
    } else {
        PowerReading {
            watts,
            cadence_rpm: instant,
            wheel_rps: Some(per_s),
        }
    })
}

/// One power meter's pages.
#[derive(Debug, Clone, Default)]
pub struct PowerDecoder {
    last: Option<(u8, u16)>,
    last_event_at: Option<Duration>,
    idle_sent: bool,
    /// The last power-only page.
    power_only_at: Option<Duration>,
    /// The last torque page (its page number) and when its event changed.
    torque: Option<(u8, Torque)>,
    torque_event_at: Option<Duration>,
    torque_idle_sent: bool,
    pub manufacturer: Option<u16>,
    pub model: Option<u16>,
    pub battery: Option<Battery>,
    /// The common pages.
    pub common: Common,
    /// Auto zero: supported and on / off (`None`: not said).
    pub auto_zero: Option<bool>,
    calibration: Option<Calibration>,
}

impl PowerDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// The meter sends torque pages only (no power-only page lately).
    pub fn torque_only(&self, now: Duration) -> bool {
        self.torque.is_some() && !self.power_only_fresh(now)
    }

    fn power_only_fresh(&self, now: Duration) -> bool {
        self.power_only_at
            .is_some_and(|t| now.saturating_sub(t) <= POWER_ONLY_FRESH)
    }

    /// A calibration answer that arrived (once).
    pub fn take_calibration(&mut self) -> Option<Calibration> {
        self.calibration.take()
    }

    pub fn on_page(&mut self, p: &[u8; 8], now: Duration) -> Option<PowerReading> {
        if self.common.on_page(p) {
            self.manufacturer = self.common.manufacturer;
            self.model = self.common.model;
            self.battery = self.common.battery;
            return None;
        }
        match p[0] {
            PAGE_POWER_ONLY => {
                self.power_only_at = Some(now);
                let event = p[1];
                let cadence = (p[3] != 0xFF).then_some(f64::from(p[3]));
                let accumulated = u16::from_le_bytes([p[4], p[5]]);
                let instant = u16::from_le_bytes([p[6], p[7]]);
                let reading = match self.last {
                    None => Some(PowerReading::new(f64::from(instant), cadence)),
                    Some((e, a)) if e != event => {
                        let events = event.wrapping_sub(e);
                        let watts = f64::from(accumulated.wrapping_sub(a)) / f64::from(events);
                        Some(PowerReading::new(watts, cadence))
                    }
                    Some(_) => {
                        // The same event again: coasting once it lasts.
                        let idle = self
                            .last_event_at
                            .is_some_and(|t| now.saturating_sub(t) > IDLE_AFTER);
                        if idle && !self.idle_sent {
                            self.idle_sent = true;
                            return Some(PowerReading::new(0.0, Some(0.0)));
                        }
                        return None;
                    }
                };
                self.last = Some((event, accumulated));
                self.last_event_at = Some(now);
                self.idle_sent = false;
                reading
            }
            PAGE_WHEEL_TORQUE | PAGE_CRANK_TORQUE => self.on_torque(p, now),
            PAGE_CALIBRATION => {
                self.on_calibration(p);
                None
            }
            _ => None,
        }
    }

    fn on_torque(&mut self, p: &[u8; 8], now: Duration) -> Option<PowerReading> {
        let page = p[0];
        let crank = page == PAGE_CRANK_TORQUE;
        let cur = Torque::parse(p);
        let prev = match self.torque {
            // A meter switching pages starts over.
            Some((pg, t)) if pg == page => Some(t),
            _ => None,
        };
        self.torque = Some((page, cur));
        let reading = match prev {
            None => {
                self.torque_event_at = Some(now);
                self.torque_idle_sent = false;
                None
            }
            Some(prev) => match torque_reading(prev, cur, crank) {
                Some(r) => {
                    self.torque_event_at = Some(now);
                    self.torque_idle_sent = false;
                    Some(r)
                }
                None => {
                    // Stale: the same event again.
                    let idle = self
                        .torque_event_at
                        .is_some_and(|t| now.saturating_sub(t) > IDLE_AFTER);
                    if idle && !self.torque_idle_sent {
                        self.torque_idle_sent = true;
                        Some(PowerReading {
                            watts: 0.0,
                            cadence_rpm: crank.then_some(0.0),
                            wheel_rps: (!crank).then_some(0.0),
                        })
                    } else {
                        None
                    }
                }
            },
        };
        // The power-only page is preferred while it comes.
        if self.power_only_fresh(now) {
            return None;
        }
        reading
    }

    fn on_calibration(&mut self, p: &[u8; 8]) {
        let auto = match p[2] {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        };
        // 0xFFFF: no calibration data.
        let offset = (p[6] != 0xFF || p[7] != 0xFF).then(|| i16::from_le_bytes([p[6], p[7]]));
        match p[1] {
            cal::SUCCESS => {
                if auto.is_some() {
                    self.auto_zero = auto;
                }
                self.calibration = Some(Calibration::Done {
                    offset,
                    auto_zero: auto,
                });
            }
            cal::FAIL => {
                if auto.is_some() {
                    self.auto_zero = auto;
                }
                self.calibration = Some(Calibration::Failed { offset });
            }
            cal::AUTO_ZERO_SUPPORT => {
                let supported = p[2] & 0x01 != 0;
                let on = p[2] & 0x02 != 0;
                let now = supported.then_some(on);
                if supported && self.auto_zero != now {
                    self.calibration = Some(Calibration::AutoZero { on });
                }
                self.auto_zero = now;
            }
            _ => {}
        }
    }
}

/// The manual zero request we send (acknowledged, page 0x01, 0xAA).
pub fn calibration_request() -> [u8; 8] {
    [
        PAGE_CALIBRATION,
        cal::REQUEST,
        0xFF,
        0xFF,
        0xFF,
        0xFF,
        0xFF,
        0xFF,
    ]
}

/// Auto zero on / off (page 0x01, 0xAB).
pub fn auto_zero_config(on: bool) -> [u8; 8] {
    [
        PAGE_CALIBRATION,
        cal::AUTO_ZERO_CONFIG,
        u8::from(on),
        0xFF,
        0xFF,
        0xFF,
        0xFF,
        0xFF,
    ]
}

/// A meter's calibration answer (the simulated stick, tests): done or
/// failed, the auto zero status (`None`: not supported), the offset.
pub fn calibration_response(ok: bool, auto_zero: Option<bool>, offset: i16) -> [u8; 8] {
    let [o0, o1] = offset.to_le_bytes();
    [
        PAGE_CALIBRATION,
        if ok { cal::SUCCESS } else { cal::FAIL },
        auto_zero.map_or(0xFF, u8::from),
        0xFF,
        0xFF,
        0xFF,
        o0,
        o1,
    ]
}

/// A meter's auto zero support page.
pub fn auto_zero_support(on: bool) -> [u8; 8] {
    [
        PAGE_CALIBRATION,
        cal::AUTO_ZERO_SUPPORT,
        0x01 | if on { 0x02 } else { 0 },
        0xFF,
        0xFF,
        0xFF,
        0xFF,
        0xFF,
    ]
}

/// A power-only page as a power meter sends it (the simulated stick).
pub fn power_only(event: u8, cadence: Option<u8>, accumulated: u16, instant: u16) -> [u8; 8] {
    let [a0, a1] = accumulated.to_le_bytes();
    let [i0, i1] = instant.to_le_bytes();
    [
        PAGE_POWER_ONLY,
        event,
        0xFF,
        cadence.unwrap_or(0xFF),
        a0,
        a1,
        i0,
        i1,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: Duration = Duration::from_millis(250);

    #[test]
    fn power_is_the_average_over_the_events() {
        let mut d = PowerDecoder::new();
        // The first page: its instantaneous power.
        let r = d.on_page(&power_only(10, Some(90), 1000, 250), S).unwrap();
        assert_eq!(r.watts, 250.0);
        assert_eq!(r.cadence_rpm, Some(90.0));
        // Two events, 460 W·events more: 230 W.
        let r = d
            .on_page(&power_only(12, Some(88), 1460, 240), S * 2)
            .unwrap();
        assert_eq!(r.watts, 230.0);
        // Rollovers of the count and the sum.
        let mut d = PowerDecoder::new();
        d.on_page(&power_only(255, None, 65500, 200), S);
        let r = d.on_page(&power_only(0, None, 164, 200), S * 2).unwrap();
        assert_eq!(r.watts, 200.0);
        assert_eq!(r.cadence_rpm, None);
    }

    #[test]
    fn no_new_event_is_coasting_after_a_while() {
        let mut d = PowerDecoder::new();
        let p = power_only(5, Some(80), 500, 200);
        assert!(d.on_page(&p, Duration::ZERO).is_some());
        assert!(d.on_page(&p, Duration::from_secs(2)).is_none());
        let r = d.on_page(&p, Duration::from_secs(4)).unwrap();
        assert_eq!(r.watts, 0.0);
        // Only once.
        assert!(d.on_page(&p, Duration::from_secs(5)).is_none());
    }

    #[test]
    fn common_pages() {
        let mut d = PowerDecoder::new();
        assert!(
            d.on_page(&[0x50, 0xFF, 0xFF, 3, 0x20, 0x00, 0x39, 0x05], S)
                .is_none()
        );
        assert_eq!(d.manufacturer, Some(32));
        assert_eq!(d.model, Some(0x0539));
        d.on_page(&[0x52, 0xFF, 0x01, 0, 0, 0, 128, 0x33], S);
        assert_eq!(d.battery.unwrap().status_text(), "ok");
        assert_eq!(d.battery.unwrap().volts, Some(3.5));
        d.on_page(&[0x51, 0xFF, 0xFF, 20, 0x39, 0x30, 0, 0], S);
        assert_eq!(d.common.serial, Some(12345));
    }

    fn crank(event: u8, ticks: u8, period: u16, torque: u16) -> [u8; 8] {
        Torque {
            event,
            ticks,
            cadence: 0xFF,
            period,
            torque,
        }
        .page(PAGE_CRANK_TORQUE)
    }

    #[test]
    fn crank_torque_byte_exact() {
        // The page as the profile lays it out: 0x12, events, ticks,
        // cadence, period LSB / MSB, torque LSB / MSB.
        let p = crank(0x05, 0x05, 0x0A00, 0x1234);
        assert_eq!(p, [0x12, 0x05, 0x05, 0xFF, 0x00, 0x0A, 0x34, 0x12]);
        assert_eq!(Torque::parse(&p).period, 2560);
        // The profile's worked example in spirit: one revolution in 0.8 s
        // (1638.4 / 2048 s) with 40 Nm average torque: 75 rpm, 314 W.
        let a = Torque::parse(&crank(10, 10, 1000, 2000));
        let b = Torque::parse(&crank(11, 11, 1000 + 1638, 2000 + 40 * 32));
        let r = torque_reading(a, b, true).unwrap();
        assert!((r.cadence_rpm.unwrap() - 75.02).abs() < 0.05, "{r:?}");
        // P = τ ω = 40 Nm × 2π / 0.7998 s.
        let expect = 40.0 * 2.0 * std::f64::consts::PI / (1638.0 / 2048.0);
        assert!((r.watts - expect).abs() < 0.01, "{} vs {expect}", r.watts);
        assert!((r.watts - 314.2).abs() < 0.5);
    }

    #[test]
    fn torque_pages_roll_over_and_coast() {
        let mut d = PowerDecoder::new();
        let t = |s: u64| Duration::from_millis(s);
        // The first page only starts the counters.
        assert!(d.on_page(&crank(254, 254, 65000, 65000), t(0)).is_none());
        // Event, period and torque all wrap: 2 events in 1.5 s with
        // 30 Nm each.
        let period = 65000u16.wrapping_add(3072);
        let torque = 65000u16.wrapping_add(2 * 30 * 32);
        let r = d.on_page(&crank(0, 0, period, torque), t(250)).unwrap();
        assert!((r.cadence_rpm.unwrap() - 80.0).abs() < 1e-9);
        let expect = 128.0 * std::f64::consts::PI * 1920.0 / 3072.0;
        assert!((r.watts - expect).abs() < 1e-9);
        assert!(d.torque_only(t(250)));
        // Coasting: a new event, the same period.
        let r = d.on_page(&crank(1, 0, period, torque), t(500)).unwrap();
        assert_eq!((r.watts, r.cadence_rpm), (0.0, Some(0.0)));
        // Stale: the same event again and again → 0 W once after 3 s.
        let p = crank(1, 0, period, torque);
        assert!(d.on_page(&p, t(1500)).is_none());
        let r = d.on_page(&p, t(3600)).unwrap();
        assert_eq!(r.watts, 0.0);
        assert!(d.on_page(&p, t(4000)).is_none(), "once");
    }

    #[test]
    fn wheel_torque_gives_power_and_the_wheel_speed() {
        let mut d = PowerDecoder::new();
        let w = |e: u8, period: u16, torque: u16| {
            Torque {
                event: e,
                ticks: e,
                cadence: 85,
                period,
                torque,
            }
            .page(PAGE_WHEEL_TORQUE)
        };
        d.on_page(&w(1, 0, 0), S);
        // 4 wheel revolutions in 1 s (8192 / 2048) with 8 Nm each.
        let r = d.on_page(&w(5, 2048, 4 * 8 * 32), S * 2).unwrap();
        assert!((r.wheel_rps.unwrap() - 4.0).abs() < 1e-9);
        assert_eq!(r.cadence_rpm, Some(85.0));
        let expect = 8.0 * 2.0 * std::f64::consts::PI * 4.0;
        assert!((r.watts - expect).abs() < 1e-9, "{}", r.watts);
    }

    #[test]
    fn the_power_only_page_is_preferred() {
        let mut d = PowerDecoder::new();
        assert!(d.on_page(&power_only(1, Some(90), 200, 200), S).is_some());
        d.on_page(&crank(1, 1, 0, 0), S * 2);
        assert!(
            d.on_page(&crank(2, 2, 1365, 1000), S * 3).is_none(),
            "torque is not a second power source"
        );
        assert!(!d.torque_only(S * 3));
        // The power-only page stops: torque takes over after 5 s.
        let later = S * 3 + POWER_ONLY_FRESH + S;
        assert!(d.on_page(&crank(3, 3, 2730, 2000), later).is_some());
        assert!(d.torque_only(later));
    }

    #[test]
    fn calibration_pages() {
        assert_eq!(
            calibration_request(),
            [0x01, 0xAA, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]
        );
        assert_eq!(
            auto_zero_config(true),
            [0x01, 0xAB, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]
        );
        let mut d = PowerDecoder::new();
        // Done: auto zero off, offset 512 (0x0200).
        let p = calibration_response(true, Some(false), 512);
        assert_eq!(p, [0x01, 0xAC, 0x00, 0xFF, 0xFF, 0xFF, 0x00, 0x02]);
        assert!(d.on_page(&p, S).is_none());
        assert_eq!(
            d.take_calibration(),
            Some(Calibration::Done {
                offset: Some(512),
                auto_zero: Some(false)
            })
        );
        assert_eq!(d.take_calibration(), None, "once");
        // Failed, a negative offset, auto zero not supported.
        d.on_page(&calibration_response(false, None, -7), S);
        assert_eq!(
            d.take_calibration(),
            Some(Calibration::Failed { offset: Some(-7) })
        );
        // No calibration data.
        d.on_page(&[0x01, 0xAC, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF], S);
        assert_eq!(
            d.take_calibration(),
            Some(Calibration::Done {
                offset: None,
                auto_zero: None
            })
        );
        // The auto zero support page: told when it changes.
        let mut d = PowerDecoder::new();
        d.on_page(&auto_zero_support(true), S);
        assert_eq!(d.auto_zero, Some(true));
        assert_eq!(
            d.take_calibration(),
            Some(Calibration::AutoZero { on: true })
        );
        d.on_page(&auto_zero_support(true), S);
        assert_eq!(d.take_calibration(), None);
    }
}
