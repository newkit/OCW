//! The ANT+ heart-rate pages (ANT+ Device Profile *Heart Rate Monitor*,
//! spec 3.4.1): 8 bytes each.
//!
//! - Byte 0: bits 0–6 the page number, bit 7 the **page toggle bit**
//!   (flips every 4 messages on straps that send the newer pages).
//!   **Legacy straps** never toggle and only bytes 4–7 mean anything: if
//!   the bit has not changed within the first [`LEGACY_AFTER`] messages,
//!   bytes 0–3 are ignored.
//! - Bytes 4–7 of every page: the heart beat event time (1/1024 s, rolls
//!   over every 64 s), the heart beat count (rolls over), the computed
//!   heart rate (bpm, 0 = invalid).
//! - Pages: 0 default, 1 operating time, 2 manufacturer, 3 product, 4
//!   previous heart beat event time (R-R intervals), 5 swim, 6
//!   capabilities, 7 battery.
//! - R-R intervals: from page 4's previous beat time, else from the last
//!   message's beat time when the count went up by exactly 1 (a missed beat
//!   gives none).
//! - Staleness: no new beat for [`STALE_AFTER`] → no heart rate (a strap on
//!   the table is not a frozen 72 bpm in the FIT file).

use std::time::Duration;

/// Messages without a toggle before a strap counts as legacy.
pub const LEGACY_AFTER: u32 = 10;
/// No new beat for this long: the heart rate is invalid.
pub const STALE_AFTER: Duration = Duration::from_secs(5);

/// Battery status (page 7).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Battery {
    /// Level in % (if the strap says).
    pub level_pct: Option<u8>,
    /// Voltage (if the strap says).
    pub volts: Option<f64>,
    /// 1 new, 2 good, 3 ok, 4 low, 5 critical.
    pub status: u8,
}

impl Battery {
    pub fn status_text(&self) -> &'static str {
        match self.status {
            1 => "new",
            2 => "good",
            3 => "ok",
            4 => "low",
            5 => "critical",
            _ => "unknown",
        }
    }

    /// Reads the battery bytes of a page: level, fractional voltage
    /// (1/256 V), descriptive field (bits 0–3 coarse voltage, 4–6 status).
    pub fn parse(level: u8, fraction: u8, descriptive: u8) -> Self {
        let coarse = descriptive & 0x0F;
        Self {
            level_pct: (level <= 100).then_some(level),
            volts: (coarse != 0x0F).then(|| f64::from(coarse) + f64::from(fraction) / 256.0),
            status: (descriptive >> 4) & 0x07,
        }
    }
}

/// What one message gave.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HrReading {
    /// The computed heart rate; `None`: invalid (0) or stale.
    pub bpm: Option<u8>,
    /// R-R intervals in seconds that ended with this message.
    pub rr_s: Vec<f64>,
    /// A new beat (the count went up).
    pub beat: bool,
}

/// The state of one strap's pages.
#[derive(Debug, Clone, Default)]
pub struct HrDecoder {
    messages: u32,
    last_toggle: Option<bool>,
    toggled: bool,
    last_time: Option<u16>,
    last_count: Option<u8>,
    last_beat_at: Option<Duration>,
    pub manufacturer: Option<u8>,
    pub serial_upper: Option<u16>,
    pub hardware_version: Option<u8>,
    pub software_version: Option<u8>,
    pub model: Option<u8>,
    pub battery: Option<Battery>,
}

impl HrDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether bytes 0–3 mean anything (the strap toggles).
    pub fn pages_valid(&self) -> bool {
        self.toggled
    }

    /// A strap that never toggled within the first messages.
    pub fn legacy(&self) -> bool {
        !self.toggled && self.messages >= LEGACY_AFTER
    }

    /// No new beat for [`STALE_AFTER`].
    pub fn stale(&self, now: Duration) -> bool {
        self.last_beat_at
            .is_some_and(|t| now.saturating_sub(t) > STALE_AFTER)
    }

    pub fn on_page(&mut self, p: &[u8; 8], now: Duration) -> HrReading {
        self.messages += 1;
        let toggle = p[0] & 0x80 != 0;
        if self.last_toggle.is_some_and(|t| t != toggle) {
            self.toggled = true;
        }
        self.last_toggle = Some(toggle);
        let page = self.toggled.then_some(p[0] & 0x7F);
        let time = u16::from_le_bytes([p[4], p[5]]);
        let count = p[6];
        let computed = p[7];
        let mut out = HrReading::default();
        match page {
            Some(2) => {
                self.manufacturer = Some(p[1]);
                self.serial_upper = Some(u16::from_le_bytes([p[2], p[3]]));
            }
            Some(3) => {
                self.hardware_version = Some(p[1]);
                self.software_version = Some(p[2]);
                self.model = Some(p[3]);
            }
            Some(7) => self.battery = Some(Battery::parse(p[1], p[2], p[3])),
            _ => {}
        }
        let previous_from_page = (page == Some(4)).then(|| u16::from_le_bytes([p[2], p[3]]));
        match self.last_count {
            None => {
                // The first message: a beat time to start from; page 4
                // already gives an interval.
                self.last_beat_at = Some(now);
                if let Some(prev) = previous_from_page {
                    push_rr(&mut out.rr_s, time, prev);
                }
            }
            Some(last) if last != count => {
                out.beat = true;
                self.last_beat_at = Some(now);
                let beats = count.wrapping_sub(last);
                match (previous_from_page, self.last_time) {
                    (Some(prev), _) => push_rr(&mut out.rr_s, time, prev),
                    (None, Some(prev)) if beats == 1 => push_rr(&mut out.rr_s, time, prev),
                    _ => {}
                }
            }
            Some(_) => {}
        }
        self.last_count = Some(count);
        self.last_time = Some(time);
        out.bpm = (computed != 0 && !self.stale(now)).then_some(computed);
        out
    }
}

/// An R-R interval if it is a plausible one (0.2 – 3 s: 20 – 300 bpm).
fn push_rr(out: &mut Vec<f64>, time: u16, prev: u16) {
    let rr = f64::from(time.wrapping_sub(prev)) / 1024.0;
    if (0.2..=3.0).contains(&rr) {
        out.push(rr);
    }
}

/// A page as a strap sends it (the simulated stick, tests).
pub fn page(number: u8, toggle: bool, b1_3: [u8; 3], time: u16, count: u8, bpm: u8) -> [u8; 8] {
    let [t0, t1] = time.to_le_bytes();
    [
        number | if toggle { 0x80 } else { 0 },
        b1_3[0],
        b1_3[1],
        b1_3[2],
        t0,
        t1,
        count,
        bpm,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: Duration = Duration::from_millis(246); // ≈ 1 / 4.06 Hz

    /// A strap's message stream: one beat per message at 1 s (60 bpm is
    /// too slow for 4 Hz; here every message is a new beat for clarity).
    fn feed(d: &mut HrDecoder, pages: &[[u8; 8]]) -> Vec<HrReading> {
        pages
            .iter()
            .enumerate()
            .map(|(i, p)| d.on_page(p, S * i as u32))
            .collect()
    }

    #[test]
    fn pages_with_the_toggle_bit_are_read() {
        let mut d = HrDecoder::new();
        let mut pages = Vec::new();
        // Four page-4 messages, toggle 0; then four with toggle 1 …
        for i in 0..12u16 {
            let toggle = (i / 4) % 2 == 1;
            let time = 1024 + i * 512; // a beat every 0.5 s
            let prev = time - 512;
            pages.push(page(
                4,
                toggle,
                [0xFF, prev.to_le_bytes()[0], prev.to_le_bytes()[1]],
                time,
                i as u8,
                120,
            ));
        }
        let r = feed(&mut d, &pages);
        assert!(d.pages_valid() && !d.legacy());
        assert!(r.iter().all(|r| r.bpm == Some(120)));
        // Before the first toggle bytes 0–3 are not trusted; consecutive
        // beats still give the interval.
        assert_eq!(r[1].rr_s, [0.5]);
        // After it: page 4 gives the previous beat time.
        assert!(r[6].rr_s == [0.5] && r[6].beat);
    }

    #[test]
    fn background_pages_manufacturer_product_battery() {
        let mut d = HrDecoder::new();
        let mut pages = vec![
            page(0, false, [0xFF; 3], 100, 1, 70),
            page(0, true, [0xFF; 3], 100, 1, 70),
        ];
        pages.push(page(2, true, [0x01, 0x34, 0x12], 100, 1, 70));
        pages.push(page(3, true, [5, 9, 42], 100, 1, 70));
        // 87 %, 2.75 V (2 + 192/256), status good (2).
        pages.push(page(7, true, [87, 192, 0x22], 100, 1, 70));
        feed(&mut d, &pages);
        assert_eq!(d.manufacturer, Some(1));
        assert_eq!(d.serial_upper, Some(0x1234));
        assert_eq!(
            (d.hardware_version, d.software_version, d.model),
            (Some(5), Some(9), Some(42))
        );
        let b = d.battery.unwrap();
        assert_eq!(b.level_pct, Some(87));
        assert_eq!(b.volts, Some(2.75));
        assert_eq!(b.status_text(), "good");
        assert_eq!(
            Battery::parse(0xFF, 0, 0x4F),
            Battery {
                level_pct: None,
                volts: None,
                status: 4
            }
        );
    }

    #[test]
    fn a_legacy_strap_uses_only_bytes_4_to_7() {
        let mut d = HrDecoder::new();
        // Never toggles; bytes 0–3 are junk that would look like page 7.
        let pages: Vec<[u8; 8]> = (0..15u16)
            .map(|i| page(7, false, [3, 3, 3], 2000 + i * 600, i as u8, 96))
            .collect();
        let r = feed(&mut d, &pages);
        assert!(d.legacy());
        assert_eq!(d.battery, None);
        assert!(r.iter().all(|r| r.bpm == Some(96)));
        let rr = 600.0 / 1024.0;
        assert_eq!(r[14].rr_s, [rr]);
    }

    #[test]
    fn zero_is_no_sample_and_rollovers_are_handled() {
        let mut d = HrDecoder::new();
        assert_eq!(d.on_page(&page(0, false, [0; 3], 0, 0, 0), S).bpm, None);
        // Beat time and count roll over: 65000 → 400 is 936 ticks.
        let mut d = HrDecoder::new();
        d.on_page(&page(0, false, [0; 3], 65000, 255, 80), S);
        let r = d.on_page(&page(0, false, [0; 3], 400, 0, 80), S * 2);
        assert!(r.beat);
        assert_eq!(r.rr_s, [936.0 / 1024.0]);
    }

    #[test]
    fn a_missed_beat_gives_no_interval_but_page_4_does() {
        let mut d = HrDecoder::new();
        d.on_page(&page(0, false, [0; 3], 1000, 10, 80), S);
        // Count +2: a beat was missed between the messages.
        let r = d.on_page(&page(0, false, [0; 3], 2600, 12, 80), S * 2);
        assert!(r.beat && r.rr_s.is_empty());
        // A toggling strap's page 4 knows the beat before.
        let mut d = HrDecoder::new();
        d.on_page(&page(4, false, [0xFF, 0, 0], 1000, 10, 80), S);
        d.on_page(&page(4, true, [0xFF, 0xE8, 0x03], 1800, 11, 80), S * 2);
        let prev = 2600u16 - 800;
        let r = d.on_page(
            &page(
                4,
                true,
                [0xFF, prev.to_le_bytes()[0], prev.to_le_bytes()[1]],
                2600,
                13,
                80,
            ),
            S * 3,
        );
        assert_eq!(r.rr_s, [800.0 / 1024.0]);
    }

    #[test]
    fn a_strap_without_new_beats_goes_stale() {
        let mut d = HrDecoder::new();
        let p = page(0, false, [0; 3], 5000, 7, 72);
        assert_eq!(d.on_page(&p, Duration::ZERO).bpm, Some(72));
        // The same beat for 4 s: still valid; after 5 s: none.
        assert_eq!(d.on_page(&p, Duration::from_secs(4)).bpm, Some(72));
        assert!(!d.stale(Duration::from_secs(5)));
        assert_eq!(d.on_page(&p, Duration::from_secs(6)).bpm, None);
        assert!(d.stale(Duration::from_secs(6)));
        // A new beat: valid again.
        let p2 = page(0, false, [0; 3], 5800, 8, 74);
        assert_eq!(d.on_page(&p2, Duration::from_secs(7)).bpm, Some(74));
    }
}
