//! The ANT+ speed and cadence sensors (ANT+ Device Profile *Bicycle Speed
//! and Cadence*): device type 121 (combined speed & cadence, period 8086),
//! 122 (cadence, 8102) and 123 (speed, 8118).
//!
//! - **Combined (121)** has one page and no page number: crank event time
//!   (1/1024 s), crank revolutions, wheel event time, wheel revolutions –
//!   each 16 bits, rolling over.
//! - **Cadence (122) and speed (123)** send byte 0 = page number with the
//!   toggle bit (bit 7, flipped every 4 messages), bytes 4 – 7 = event
//!   time and cumulative revolutions on every page; the background pages
//!   1 (operating time), 2 (manufacturer, serial), 3 (versions, model),
//!   4 (battery) and 5 (speed sensors: stopped) mean something only once
//!   the toggle bit was seen to change – a legacy sensor never toggles and
//!   gives only the counters, as with heart-rate straps ([`super::hr`]).
//!
//! The rates come from consecutive messages with a new event (as
//! `gatt::cadence` does for Bluetooth): revolutions × 1024 / Δtime,
//! rollovers included; a repeated event is no news, and no new event for
//! [`STOP_AFTER`] is 0 (the crank or wheel stopped). The wheel's speed is
//! its revolutions per second times the circumference, which the app knows
//! (the rider's bike or the setting `devices.wheel_mm`).

use std::time::Duration;

use super::common::{fill_battery, software_version};
use super::hr::Battery;
use crate::{SensorInfo, SensorSource};

/// Messages without a toggle before a sensor counts as legacy.
pub const LEGACY_AFTER: u32 = 10;
/// No new event for this long: the crank / wheel stopped (0).
pub const STOP_AFTER: Duration = Duration::from_millis(3000);
/// Crank rates above this (250 rpm) are glitches.
const MAX_CRANK_PER_S: f64 = 250.0 / 60.0;
/// Wheel rates above this (about 150 km/h on a 2.1 m wheel) are glitches.
const MAX_WHEEL_PER_S: f64 = 20.0;

/// Which sensor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpdCadKind {
    /// Type 121: speed and cadence in one page.
    Combined,
    /// Type 122.
    Cadence,
    /// Type 123.
    Speed,
}

/// One counter: event time (1/1024 s) and cumulative revolutions.
#[derive(Debug, Clone, Default)]
pub struct Revolutions {
    last: Option<(u16, u16)>,
    last_event_at: Option<Duration>,
    /// Revolutions per second (`None`: not known yet).
    rate: Option<f64>,
    stop_sent: bool,
}

impl Revolutions {
    /// A message with these counters at `now`; returns a new rate (per
    /// second), 0 once after [`STOP_AFTER`] without a new event.
    pub fn update(&mut self, time: u16, revs: u16, now: Duration, max: f64) -> Option<f64> {
        let Some((t, r)) = self.last else {
            self.last = Some((time, revs));
            self.last_event_at = Some(now);
            return None;
        };
        if time == t {
            if revs != r {
                // A count without time: a glitch, start over.
                self.last = Some((time, revs));
            }
            return self.stopped(now);
        }
        let dt = time.wrapping_sub(t);
        let dr = revs.wrapping_sub(r);
        self.last = Some((time, revs));
        self.last_event_at = Some(now);
        self.stop_sent = false;
        let rate = f64::from(dr) * 1024.0 / f64::from(dt);
        if rate > max {
            return None;
        }
        self.rate = Some(rate);
        Some(rate)
    }

    /// 0 once, after [`STOP_AFTER`] without a new event.
    pub fn stopped(&mut self, now: Duration) -> Option<f64> {
        let idle = self
            .last_event_at
            .is_some_and(|t| now.saturating_sub(t) >= STOP_AFTER);
        if idle && !self.stop_sent && self.rate.is_some() {
            self.stop_sent = true;
            self.rate = Some(0.0);
            return Some(0.0);
        }
        None
    }

    pub fn rate(&self) -> Option<f64> {
        self.rate
    }
}

/// What one message gave (only what changed).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SpdCadReading {
    pub cadence_rpm: Option<f64>,
    pub wheel_rps: Option<f64>,
}

impl SpdCadReading {
    pub fn is_empty(&self) -> bool {
        self.cadence_rpm.is_none() && self.wheel_rps.is_none()
    }
}

/// One sensor's pages.
#[derive(Debug, Clone)]
pub struct SpdCadDecoder {
    pub kind: SpdCadKind,
    messages: u32,
    last_toggle: Option<bool>,
    toggled: bool,
    crank: Revolutions,
    wheel: Revolutions,
    pub manufacturer: Option<u8>,
    pub serial_upper: Option<u16>,
    pub hardware_version: Option<u8>,
    pub software_version: Option<u8>,
    pub model: Option<u8>,
    pub battery: Option<Battery>,
    /// Cumulative operating time (s).
    pub operating_s: Option<u32>,
    /// Speed sensors with page 5: the bike stands.
    pub stopped: Option<bool>,
}

impl SpdCadDecoder {
    pub fn new(kind: SpdCadKind) -> Self {
        Self {
            kind,
            messages: 0,
            last_toggle: None,
            toggled: false,
            crank: Revolutions::default(),
            wheel: Revolutions::default(),
            manufacturer: None,
            serial_upper: None,
            hardware_version: None,
            software_version: None,
            model: None,
            battery: None,
            operating_s: None,
            stopped: None,
        }
    }

    /// Whether the sensor gives a cadence.
    pub fn has_cadence(&self) -> bool {
        self.kind != SpdCadKind::Speed
    }

    /// Whether bytes 0 – 3 mean anything (the sensor toggles).
    pub fn pages_valid(&self) -> bool {
        self.toggled
    }

    /// A sensor that never toggled within the first messages.
    pub fn legacy(&self) -> bool {
        self.kind != SpdCadKind::Combined && !self.toggled && self.messages >= LEGACY_AFTER
    }

    pub fn on_page(&mut self, p: &[u8; 8], now: Duration) -> SpdCadReading {
        self.messages += 1;
        let mut out = SpdCadReading::default();
        if self.kind == SpdCadKind::Combined {
            out.cadence_rpm = self
                .crank
                .update(
                    u16::from_le_bytes([p[0], p[1]]),
                    u16::from_le_bytes([p[2], p[3]]),
                    now,
                    MAX_CRANK_PER_S,
                )
                .map(|r| r * 60.0);
            out.wheel_rps = self.wheel.update(
                u16::from_le_bytes([p[4], p[5]]),
                u16::from_le_bytes([p[6], p[7]]),
                now,
                MAX_WHEEL_PER_S,
            );
            return out;
        }
        let toggle = p[0] & 0x80 != 0;
        if self.last_toggle.is_some_and(|t| t != toggle) {
            self.toggled = true;
        }
        self.last_toggle = Some(toggle);
        match self.toggled.then_some(p[0] & 0x7F) {
            Some(1) => {
                self.operating_s = Some(u32::from_le_bytes([p[1], p[2], p[3], 0]) * 2);
            }
            Some(2) => {
                self.manufacturer = Some(p[1]);
                self.serial_upper = Some(u16::from_le_bytes([p[2], p[3]]));
            }
            Some(3) => {
                self.hardware_version = Some(p[1]);
                self.software_version = Some(p[2]);
                self.model = Some(p[3]);
            }
            Some(4) => self.battery = Some(Battery::parse(0xFF, p[2], p[3])),
            Some(5) if self.kind == SpdCadKind::Speed => self.stopped = Some(p[1] & 0x01 != 0),
            _ => {}
        }
        let time = u16::from_le_bytes([p[4], p[5]]);
        let revs = u16::from_le_bytes([p[6], p[7]]);
        match self.kind {
            SpdCadKind::Cadence => {
                out.cadence_rpm = self
                    .crank
                    .update(time, revs, now, MAX_CRANK_PER_S)
                    .map(|r| r * 60.0);
            }
            _ => out.wheel_rps = self.wheel.update(time, revs, now, MAX_WHEEL_PER_S),
        }
        out
    }

    /// The stop rule without a message (the sensor went quiet).
    pub fn on_tick(&mut self, now: Duration) -> SpdCadReading {
        SpdCadReading {
            cadence_rpm: self.crank.stopped(now).map(|r| r * 60.0),
            wheel_rps: self.wheel.stopped(now),
        }
    }

    /// What the sensor said about itself (`device_number`: the lower 16
    /// bits of the serial number).
    pub fn info(&self, source: SensorSource) -> SensorInfo {
        let mut i = SensorInfo::new(source);
        i.manufacturer = self.manufacturer.map(u16::from);
        i.product = self.model.map(u16::from);
        if let (Some(upper), SensorSource::AntPlus { device_number, .. }) =
            (self.serial_upper, source)
        {
            i.serial = Some(u32::from(upper) << 16 | u32::from(device_number));
        }
        i.hardware_version = self.hardware_version;
        i.software_version = self.software_version.map(|v| software_version(v, 0xFF));
        if let Some(b) = self.battery {
            fill_battery(&mut i, b);
        }
        i
    }
}

/// The combined sensor's page (the simulated stick, tests).
pub fn combined_page(
    crank_time: u16,
    crank_revs: u16,
    wheel_time: u16,
    wheel_revs: u16,
) -> [u8; 8] {
    let [a, b] = crank_time.to_le_bytes();
    let [c, d] = crank_revs.to_le_bytes();
    let [e, f] = wheel_time.to_le_bytes();
    let [g, h] = wheel_revs.to_le_bytes();
    [a, b, c, d, e, f, g, h]
}

/// A cadence or speed sensor's page: number, toggle, bytes 1 – 3, event
/// time, revolutions.
pub fn page(number: u8, toggle: bool, b1_3: [u8; 3], time: u16, revs: u16) -> [u8; 8] {
    let [t0, t1] = time.to_le_bytes();
    let [r0, r1] = revs.to_le_bytes();
    [
        number | if toggle { 0x80 } else { 0 },
        b1_3[0],
        b1_3[1],
        b1_3[2],
        t0,
        t1,
        r0,
        r1,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(v: u64) -> Duration {
        Duration::from_millis(v)
    }

    #[test]
    fn combined_page_byte_exact_with_rollovers() {
        let mut d = SpdCadDecoder::new(SpdCadKind::Combined);
        // Crank: 0x1000 ticks, 42 revs; wheel: 0x2000 ticks, 1000 revs.
        let p = combined_page(0x1000, 42, 0x2000, 1000);
        assert_eq!(p, [0x00, 0x10, 0x2A, 0x00, 0x00, 0x20, 0xE8, 0x03]);
        assert!(d.on_page(&p, ms(0)).is_empty(), "the first only starts");
        // 1 crank rev in 0.75 s (768 ticks): 80 rpm; 3 wheel revs in 1 s.
        let r = d.on_page(&combined_page(0x1300, 43, 0x2400, 1003), ms(250));
        assert!((r.cadence_rpm.unwrap() - 80.0).abs() < 1e-9);
        assert!((r.wheel_rps.unwrap() - 3.0).abs() < 1e-9);
        // Both counters and both times roll over.
        let mut d = SpdCadDecoder::new(SpdCadKind::Combined);
        d.on_page(&combined_page(65000, 65535, 65200, 65534), ms(0));
        let r = d.on_page(
            &combined_page(
                65000u16.wrapping_add(1024),
                1,
                65200u16.wrapping_add(512),
                0,
            ),
            ms(250),
        );
        assert!((r.cadence_rpm.unwrap() - 120.0).abs() < 1e-9, "{r:?}");
        assert!((r.wheel_rps.unwrap() - 4.0).abs() < 1e-9, "{r:?}");
    }

    #[test]
    fn a_repeated_event_is_no_news_and_a_stop_is_zero_once() {
        let mut d = SpdCadDecoder::new(SpdCadKind::Combined);
        d.on_page(&combined_page(0, 0, 0, 0), ms(0));
        d.on_page(&combined_page(683, 1, 512, 1), ms(250));
        let same = combined_page(683, 1, 512, 1);
        assert!(d.on_page(&same, ms(500)).is_empty());
        assert!(d.on_page(&same, ms(2500)).is_empty());
        let r = d.on_page(&same, ms(3300));
        assert_eq!(r.cadence_rpm, Some(0.0));
        assert_eq!(r.wheel_rps, Some(0.0));
        assert!(d.on_page(&same, ms(3600)).is_empty(), "once");
        assert!(d.on_tick(ms(9000)).is_empty());
        // Pedalling again.
        let r = d.on_page(&combined_page(1366, 2, 1024, 2), ms(9250));
        assert!(r.cadence_rpm.unwrap() > 80.0);
    }

    #[test]
    fn a_glitch_is_dropped() {
        let mut d = SpdCadDecoder::new(SpdCadKind::Combined);
        d.on_page(&combined_page(0, 0, 0, 0), ms(0));
        // 100 crank revs in 1 s: not cadence.
        let r = d.on_page(&combined_page(1024, 100, 1024, 3), ms(250));
        assert_eq!(r.cadence_rpm, None);
        assert_eq!(r.wheel_rps, Some(3.0));
    }

    #[test]
    fn cadence_and_speed_sensors_with_pages_and_legacy() {
        let mut d = SpdCadDecoder::new(SpdCadKind::Cadence);
        let mut revs = 0u16;
        let mut time = 0u16;
        for i in 0..20u32 {
            // 90 rpm: one rev every 2/3 s (683 ticks); a message per 250 ms.
            if i % 3 == 0 {
                revs = revs.wrapping_add(1);
                time = time.wrapping_add(683);
            }
            let toggle = (i / 4) % 2 == 1;
            let (n, b) = match i {
                8 => (2, [7, 0x34, 0x12]),
                12 => (3, [2, 31, 44]),
                16 => (4, [0xFF, 128, 0x23]),
                _ => (0, [0xFF; 3]),
            };
            let r = d.on_page(&page(n, toggle, b, time, revs), ms(250 * u64::from(i)));
            if let Some(c) = r.cadence_rpm {
                assert!((c - 89.96).abs() < 0.1, "{c}");
            }
            assert_eq!(r.wheel_rps, None);
        }
        assert!(d.pages_valid() && !d.legacy());
        assert_eq!(d.manufacturer, Some(7));
        assert_eq!(d.model, Some(44));
        let src = SensorSource::AntPlus {
            device_type: 122,
            device_number: 0x5678,
            trans_type: 1,
        };
        let i = d.info(src);
        assert_eq!(i.serial, Some(0x1234_5678));
        assert_eq!(i.software_version, Some(3.1));
        assert_eq!(i.battery_volts, Some(3.5));
        assert_eq!(i.battery_status, Some(2));
        // A legacy speed sensor: never toggles, the bytes 1 – 3 are junk.
        let mut d = SpdCadDecoder::new(SpdCadKind::Speed);
        for i in 0..12u16 {
            let r = d.on_page(
                &page(2, false, [9, 9, 9], i * 512, i),
                ms(250 * u64::from(i)),
            );
            if i > 0 {
                assert_eq!(r.wheel_rps, Some(2.0));
            }
            assert_eq!(r.cadence_rpm, None);
        }
        assert!(d.legacy());
        assert_eq!(d.manufacturer, None, "no pages from a legacy sensor");
        // Page 5: the bike stands.
        let mut d = SpdCadDecoder::new(SpdCadKind::Speed);
        d.on_page(&page(0, false, [0xFF; 3], 0, 0), ms(0));
        d.on_page(&page(5, true, [0x01, 0xFF, 0xFF], 0, 0), ms(250));
        assert_eq!(d.stopped, Some(true));
    }
}
