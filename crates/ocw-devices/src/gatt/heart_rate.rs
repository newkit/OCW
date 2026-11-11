//! Heart Rate Service: measurement (0x2A37).

use super::{ParseError, Reader, bit};

/// Heart Rate Measurement notification.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Measurement {
    pub bpm: u16,
    /// Skin contact, if the sensor reports it.
    pub contact: Option<bool>,
    pub energy_expended_kj: Option<u16>,
    /// RR intervals in seconds (resolution 1/1024 s).
    pub rr_intervals_s: Vec<f64>,
}

impl Measurement {
    pub fn parse(value: &[u8]) -> Result<Self, ParseError> {
        let mut r = Reader::new(value);
        let flags = u32::from(r.u8()?);
        let bpm = if bit(flags, 0) {
            r.u16()?
        } else {
            u16::from(r.u8()?)
        };
        // Bit 2: contact detection supported; bit 1: contact detected.
        let contact = bit(flags, 2).then(|| bit(flags, 1));
        let energy_expended_kj = if bit(flags, 3) { Some(r.u16()?) } else { None };
        let mut rr_intervals_s = Vec::new();
        if bit(flags, 4) {
            while r.remaining().len() >= 2 {
                rr_intervals_s.push(f64::from(r.u16()?) / 1024.0);
            }
        }
        Ok(Self {
            bpm,
            contact,
            energy_expended_kj,
            rr_intervals_s,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eight_bit_heart_rate() {
        let m = Measurement::parse(&[0x00, 0x48]).unwrap();
        assert_eq!(m.bpm, 72);
        assert_eq!(m.contact, None);
        assert!(m.rr_intervals_s.is_empty());
    }

    #[test]
    fn sixteen_bit_with_contact_energy_and_rr() {
        // flags: 16-bit (0), contact detected (1), supported (2), energy (3), RR (4)
        let m = Measurement::parse(&[
            0x1F, 0x2C, 0x01, // 300 bpm (16-bit)
            0x64, 0x00, // 100 kJ
            0x00, 0x04, 0x00, 0x02, // RR 1.0 s, 0.5 s
        ])
        .unwrap();
        assert_eq!(m.bpm, 300);
        assert_eq!(m.contact, Some(true));
        assert_eq!(m.energy_expended_kj, Some(100));
        assert_eq!(m.rr_intervals_s, vec![1.0, 0.5]);
    }

    #[test]
    fn no_contact_and_truncation() {
        let m = Measurement::parse(&[0x04, 0x00]).unwrap();
        assert_eq!(m.contact, Some(false));
        assert!(Measurement::parse(&[0x01, 0x2C]).is_err());
        assert!(Measurement::parse(&[]).is_err());
    }
}
