//! The ANT+ bike-power pages (ANT+ Device Profile *Bicycle Power*, device
//! type 11): the **power-only page 0x10** – event count, pedal balance,
//! instantaneous cadence, accumulated power, instantaneous power – and the
//! common pages 0x50 (manufacturer) and 0x52 (battery). The torque pages
//! (0x11 wheel, 0x12 crank) and calibration (0x01) come later.
//!
//! Power is the average over the events since the last message
//! (accumulated power difference / event count difference), as head units
//! show it; no new event for [`IDLE_AFTER`] is 0 W (coasting).

use std::time::Duration;

use super::hr::Battery;

/// The power-only page.
pub const PAGE_POWER_ONLY: u8 = 0x10;
/// Common page: manufacturer information.
pub const PAGE_MANUFACTURER: u8 = 0x50;
/// Common page: battery status.
pub const PAGE_BATTERY: u8 = 0x52;

/// No new power event for this long: 0 W.
pub const IDLE_AFTER: Duration = Duration::from_secs(3);

/// What one message gave.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PowerReading {
    pub watts: f64,
    pub cadence_rpm: Option<f64>,
}

/// One power meter's pages.
#[derive(Debug, Clone, Default)]
pub struct PowerDecoder {
    last: Option<(u8, u16)>,
    last_event_at: Option<Duration>,
    idle_sent: bool,
    pub manufacturer: Option<u16>,
    pub model: Option<u16>,
    pub battery: Option<Battery>,
}

impl PowerDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn on_page(&mut self, p: &[u8; 8], now: Duration) -> Option<PowerReading> {
        match p[0] {
            PAGE_POWER_ONLY => {
                let event = p[1];
                let cadence = (p[3] != 0xFF).then_some(f64::from(p[3]));
                let accumulated = u16::from_le_bytes([p[4], p[5]]);
                let instant = u16::from_le_bytes([p[6], p[7]]);
                let reading = match self.last {
                    None => Some(PowerReading {
                        watts: f64::from(instant),
                        cadence_rpm: cadence,
                    }),
                    Some((e, a)) if e != event => {
                        let events = event.wrapping_sub(e);
                        let watts = f64::from(accumulated.wrapping_sub(a)) / f64::from(events);
                        Some(PowerReading {
                            watts,
                            cadence_rpm: cadence,
                        })
                    }
                    Some(_) => {
                        // The same event again: coasting once it lasts.
                        let idle = self
                            .last_event_at
                            .is_some_and(|t| now.saturating_sub(t) > IDLE_AFTER);
                        if idle && !self.idle_sent {
                            self.idle_sent = true;
                            return Some(PowerReading {
                                watts: 0.0,
                                cadence_rpm: Some(0.0),
                            });
                        }
                        return None;
                    }
                };
                self.last = Some((event, accumulated));
                self.last_event_at = Some(now);
                self.idle_sent = false;
                reading
            }
            PAGE_MANUFACTURER => {
                self.manufacturer = Some(u16::from_le_bytes([p[4], p[5]]));
                self.model = Some(u16::from_le_bytes([p[6], p[7]]));
                None
            }
            PAGE_BATTERY => {
                self.battery = Some(Battery::parse(0xFF, p[6], p[7]));
                None
            }
            _ => None,
        }
    }
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
    }
}
