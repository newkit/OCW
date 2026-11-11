//! Cycling Power Service: measurement (0x2A63) and feature (0x2A65).

use super::cadence::CrankData;
use super::{ParseError, Reader, bit};

/// Wheel revolution data of a power meter (event time in 1/2048 s).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WheelData {
    pub revolutions: u32,
    pub event_time: u16,
}

/// Cycling Power Measurement notification.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Measurement {
    pub power_w: i16,
    /// Share of the reference pedal (usually left), in percent.
    pub pedal_power_balance_percent: Option<f64>,
    pub wheel: Option<WheelData>,
    pub crank: Option<CrankData>,
    pub accumulated_energy_kj: Option<u16>,
}

impl Measurement {
    pub fn parse(value: &[u8]) -> Result<Self, ParseError> {
        let mut r = Reader::new(value);
        let flags = u32::from(r.u16()?);
        let mut m = Self {
            power_w: r.i16()?,
            ..Self::default()
        };
        if bit(flags, 0) {
            m.pedal_power_balance_percent = Some(f64::from(r.u8()?) / 2.0);
        }
        if bit(flags, 2) {
            r.skip(2)?; // accumulated torque
        }
        if bit(flags, 4) {
            m.wheel = Some(WheelData {
                revolutions: r.u32()?,
                event_time: r.u16()?,
            });
        }
        if bit(flags, 5) {
            m.crank = Some(CrankData {
                revolutions: r.u16()?,
                event_time: r.u16()?,
            });
        }
        if bit(flags, 6) {
            r.skip(4)?; // extreme force magnitudes
        }
        if bit(flags, 7) {
            r.skip(4)?; // extreme torque magnitudes
        }
        if bit(flags, 8) {
            r.skip(3)?; // extreme angles
        }
        if bit(flags, 9) {
            r.skip(2)?; // top dead spot angle
        }
        if bit(flags, 10) {
            r.skip(2)?; // bottom dead spot angle
        }
        if bit(flags, 11) {
            m.accumulated_energy_kj = Some(r.u16()?);
        }
        Ok(m)
    }
}

/// Cycling Power Feature.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Features(pub u32);

impl Features {
    pub fn parse(value: &[u8]) -> Result<Self, ParseError> {
        Ok(Self(Reader::new(value).u32()?))
    }
    pub fn pedal_power_balance(&self) -> bool {
        bit(self.0, 0)
    }
    pub fn wheel_revolution_data(&self) -> bool {
        bit(self.0, 2)
    }
    pub fn crank_revolution_data(&self) -> bool {
        bit(self.0, 3)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn power_only() {
        let m = Measurement::parse(&[0x00, 0x00, 0x2C, 0x01]).unwrap();
        assert_eq!(m.power_w, 300);
        assert_eq!(m.crank, None);
    }

    #[test]
    fn power_with_crank_data() {
        // flags 0x0020: crank revolution data.
        let m = Measurement::parse(&[0x20, 0x00, 0xD2, 0x00, 0x39, 0x05, 0x00, 0x80]).unwrap();
        assert_eq!(m.power_w, 210);
        assert_eq!(
            m.crank,
            Some(CrankData {
                revolutions: 1337,
                event_time: 0x8000
            })
        );
    }

    #[test]
    fn skips_optional_fields_to_reach_later_ones() {
        let value = [
            0x75, 0x08, // flags: balance, torque, wheel, crank, extreme forces, energy
            0xFA, 0x00, // 250 W
            0x64, // balance 100 × 0.5 = 50 %
            0x00, 0x01, // accumulated torque (skipped)
            0x10, 0x00, 0x00, 0x00, 0x00, 0x08, // wheel: 16 revs, time 2048
            0x05, 0x00, 0x00, 0x04, // crank: 5 revs, time 1024
            0x01, 0x00, 0x02, 0x00, // extreme forces (skipped)
            0x0F, 0x00, // accumulated energy 15 kJ
        ];
        let m = Measurement::parse(&value).unwrap();
        assert_eq!(m.power_w, 250);
        assert_eq!(m.pedal_power_balance_percent, Some(50.0));
        assert_eq!(
            m.wheel,
            Some(WheelData {
                revolutions: 16,
                event_time: 2048
            })
        );
        assert_eq!(
            m.crank,
            Some(CrankData {
                revolutions: 5,
                event_time: 1024
            })
        );
        assert_eq!(m.accumulated_energy_kj, Some(15));
    }

    #[test]
    fn truncated_and_features() {
        assert!(Measurement::parse(&[0x20, 0x00, 0xD2, 0x00, 0x39]).is_err());
        assert!(Measurement::parse(&[0x00, 0x00, 0x2C]).is_err());
        let f = Features::parse(&[0x0D, 0x00, 0x00, 0x00]).unwrap();
        assert!(f.pedal_power_balance() && f.wheel_revolution_data() && f.crank_revolution_data());
        assert!(!Features(0).crank_revolution_data());
    }
}
