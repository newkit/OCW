//! Cycling Speed and Cadence: measurement (0x2A5B).

use super::cadence::CrankData;
use super::{ParseError, Reader, bit};

/// CSC Measurement notification (event times in 1/1024 s).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Measurement {
    /// Cumulative wheel revolutions and last wheel event time.
    pub wheel: Option<(u32, u16)>,
    pub crank: Option<CrankData>,
}

impl Measurement {
    pub fn parse(value: &[u8]) -> Result<Self, ParseError> {
        let mut r = Reader::new(value);
        let flags = u32::from(r.u8()?);
        let mut m = Self::default();
        if bit(flags, 0) {
            m.wheel = Some((r.u32()?, r.u16()?));
        }
        if bit(flags, 1) {
            m.crank = Some(CrankData {
                revolutions: r.u16()?,
                event_time: r.u16()?,
            });
        }
        Ok(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crank_only_sensor() {
        let m = Measurement::parse(&[0x02, 0x2A, 0x00, 0x00, 0x10]).unwrap();
        assert_eq!(m.wheel, None);
        assert_eq!(
            m.crank,
            Some(CrankData {
                revolutions: 42,
                event_time: 0x1000
            })
        );
    }

    #[test]
    fn wheel_and_crank() {
        let m = Measurement::parse(&[
            0x03, 0xE8, 0x03, 0x00, 0x00, 0x00, 0x04, 0x07, 0x00, 0x00, 0x02,
        ])
        .unwrap();
        assert_eq!(m.wheel, Some((1000, 1024)));
        assert_eq!(m.crank.unwrap().revolutions, 7);
        assert!(Measurement::parse(&[0x03, 0xE8, 0x03]).is_err());
    }
}
