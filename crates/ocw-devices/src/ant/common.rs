//! The ANT+ common pages every newer sensor sends between its data pages
//! (ANT+ *Common Data Pages*): 0x50 manufacturer's information, 0x51
//! product information, 0x52 battery status. Power meters, FE-C trainers
//! and newer speed / cadence sensors send them; heart-rate straps and
//! older speed / cadence sensors have their own background pages (2, 3,
//! 7 / 2, 3, 4 – the same fields, [`super::hr`], [`super::spdcad`]).
//!
//! Collected into a [`SensorInfo`] for the FIT file's `device_info` (spec
//! 14 Phase 17, A6).

use super::hr::Battery;
use crate::{SensorInfo, SensorSource};

/// Common page: manufacturer's information.
pub const PAGE_MANUFACTURER: u8 = 0x50;
/// Common page: product information.
pub const PAGE_PRODUCT: u8 = 0x51;
/// Common page: battery status.
pub const PAGE_BATTERY: u8 = 0x52;

/// What a sensor's pages said about it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Common {
    pub manufacturer: Option<u16>,
    pub model: Option<u16>,
    pub hardware_version: Option<u8>,
    /// The main software revision and the supplemental one (0xFF: none).
    pub software: Option<(u8, u8)>,
    pub serial: Option<u32>,
    pub battery: Option<Battery>,
}

impl Common {
    /// Reads a common page; `false`: not one.
    pub fn on_page(&mut self, p: &[u8; 8]) -> bool {
        match p[0] {
            PAGE_MANUFACTURER => {
                if p[3] != 0xFF {
                    self.hardware_version = Some(p[3]);
                }
                self.manufacturer = Some(u16::from_le_bytes([p[4], p[5]]));
                self.model = Some(u16::from_le_bytes([p[6], p[7]]));
            }
            PAGE_PRODUCT => {
                if p[3] != 0xFF {
                    self.software = Some((p[3], p[2]));
                }
                let serial = u32::from_le_bytes([p[4], p[5], p[6], p[7]]);
                if serial != 0xFFFF_FFFF {
                    self.serial = Some(serial);
                }
            }
            PAGE_BATTERY => self.battery = Some(Battery::parse(0xFF, p[6], p[7])),
            _ => return false,
        }
        true
    }

    /// As a [`SensorInfo`] of `source`.
    pub fn info(&self, source: SensorSource) -> SensorInfo {
        let mut i = SensorInfo::new(source);
        i.manufacturer = self.manufacturer;
        i.product = self.model;
        i.serial = self.serial;
        i.hardware_version = self.hardware_version;
        i.software_version = self
            .software
            .map(|(main, supp)| software_version(main, supp));
        if let Some(b) = self.battery {
            fill_battery(&mut i, b);
        }
        i
    }
}

/// The software version of the ANT+ revision bytes: the main revision
/// counts tenths (12 = 1.2), the supplemental one (0xFF: none) adds
/// thousandths (12, 34 = 1.234).
pub fn software_version(main: u8, supplemental: u8) -> f64 {
    let v = f64::from(main) / 10.0;
    if supplemental == 0xFF {
        v
    } else {
        v + f64::from(supplemental) / 1000.0
    }
}

/// Puts a battery page into an info (status 0 / 7: not said).
pub fn fill_battery(i: &mut SensorInfo, b: Battery) {
    i.battery_volts = b.volts;
    i.battery_status = (1..=5).contains(&b.status).then_some(b.status);
    i.battery_pct = b.level_pct;
}

/// A common page as a sensor sends it (the simulated stick, tests):
/// manufacturer, hardware revision, model.
pub fn manufacturer_page(manufacturer: u16, hardware: u8, model: u16) -> [u8; 8] {
    let [m0, m1] = manufacturer.to_le_bytes();
    let [d0, d1] = model.to_le_bytes();
    [PAGE_MANUFACTURER, 0xFF, 0xFF, hardware, m0, m1, d0, d1]
}

/// Product information: software revisions, serial number.
pub fn product_page(main: u8, supplemental: u8, serial: u32) -> [u8; 8] {
    let [s0, s1, s2, s3] = serial.to_le_bytes();
    [PAGE_PRODUCT, 0xFF, supplemental, main, s0, s1, s2, s3]
}

/// Battery status: voltage (1/256 V steps) and status (1 new … 5
/// critical).
pub fn battery_page(volts: f64, status: u8) -> [u8; 8] {
    let coarse = (volts.floor() as u8).min(14);
    let fraction = ((volts - f64::from(coarse)) * 256.0)
        .round()
        .clamp(0.0, 255.0) as u8;
    [
        PAGE_BATTERY,
        0xFF,
        0xFF,
        0,
        0,
        0,
        fraction,
        coarse | (status & 0x07) << 4,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: SensorSource = SensorSource::AntPlus {
        device_type: 11,
        device_number: 4004,
        trans_type: 5,
    };

    #[test]
    fn common_pages_byte_exact() {
        let mut c = Common::default();
        // Page 80: HW revision 3, manufacturer 32 (Wahoo), model 0x0539.
        assert!(c.on_page(&[0x50, 0xFF, 0xFF, 3, 0x20, 0x00, 0x39, 0x05]));
        // Page 81: SW 2.5 (main 25, no supplemental), serial 0x01020304.
        assert!(c.on_page(&[0x51, 0xFF, 0xFF, 25, 0x04, 0x03, 0x02, 0x01]));
        // Page 82: 2.75 V (2 + 192/256), status 3 (ok).
        assert!(c.on_page(&[0x52, 0xFF, 0xFF, 0, 0, 0, 192, 0x32]));
        assert!(!c.on_page(&[0x10, 0, 0, 0, 0, 0, 0, 0]));
        let i = c.info(SRC);
        assert_eq!(i.manufacturer, Some(32));
        assert_eq!(i.product, Some(0x0539));
        assert_eq!(i.hardware_version, Some(3));
        assert_eq!(i.software_version, Some(2.5));
        assert_eq!(i.serial, Some(0x0102_0304));
        assert_eq!(i.battery_volts, Some(2.75));
        assert_eq!(i.battery_status, Some(3));
        assert_eq!(i.battery_pct, None);
        // Supplemental revision; no serial (0xFFFFFFFF).
        let mut c = Common::default();
        c.on_page(&[0x51, 0xFF, 34, 12, 0xFF, 0xFF, 0xFF, 0xFF]);
        assert!((c.info(SRC).software_version.unwrap() - 1.234).abs() < 1e-9);
        assert_eq!(c.serial, None);
    }

    #[test]
    fn the_page_builders_read_back() {
        let mut c = Common::default();
        c.on_page(&manufacturer_page(1, 4, 2777));
        c.on_page(&product_page(31, 0xFF, 987_654));
        c.on_page(&battery_page(3.0, 2));
        let i = c.info(SRC);
        assert_eq!(
            (i.manufacturer, i.product, i.hardware_version),
            (Some(1), Some(2777), Some(4))
        );
        assert_eq!(i.software_version, Some(3.1));
        assert_eq!(i.serial, Some(987_654));
        assert_eq!(i.battery_volts, Some(3.0));
        assert_eq!(i.battery_status, Some(2));
    }
}
