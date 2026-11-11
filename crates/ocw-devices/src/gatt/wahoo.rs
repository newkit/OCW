//! Wahoo's proprietary trainer control (older KICKR models without FTMS).
//!
//! A write/indicate characteristic in the Cycling Power service,
//! `a026e005-0a7d-4ab3-97fa-f1500f9feb8b`. Not officially documented; the
//! byte layout follows open-source implementations (e.g. GoldenCheetah) and
//! is verified against a real KICKR with the `ble` example (spec §14).
//!
//! Commands are `[op code, little-endian parameters...]`; the trainer
//! answers with an indication `[0x01, op code, result, echoed parameters...]`
//! where result 0x01 is success (observed on a KICKR: unlock answers with
//! result 0x02 when no unlock is needed).

use super::{ParseError, Reader};

/// The control characteristic's full UUID.
pub const CONTROL_POINT_UUID: &str = "a026e005-0a7d-4ab3-97fa-f1500f9feb8b";

pub mod op {
    pub const UNLOCK: u8 = 0x20;
    pub const SET_ERG: u8 = 0x42;
    pub const SET_SIM_MODE: u8 = 0x43;
    pub const SET_GRADE: u8 = 0x46;
    pub const SET_WIND_SPEED: u8 = 0x47;
}

/// A request to the Wahoo trainer control point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Request {
    /// Must be sent once before any other request.
    Unlock,
    /// ERG mode with this target in watts.
    Erg(f64),
    /// Switches to SIM mode with the rider + bike mass (kg), the rolling
    /// resistance coefficient and the wind resistance coefficient
    /// 0.5 · ρ · CdA (kg/m).
    SimMode {
        mass_kg: f64,
        crr: f64,
        cw_kg_per_m: f64,
    },
    /// SIM gradient as rise over run (−1 … 1).
    Grade(f64),
    /// SIM wind speed in m/s (headwind positive).
    WindSpeed(f64),
}

fn u16_scaled(value: f64, scale: f64) -> [u8; 2] {
    ((value * scale).round().clamp(0.0, f64::from(u16::MAX)) as u16).to_le_bytes()
}

impl Request {
    pub fn op_code(&self) -> u8 {
        match self {
            Self::Unlock => op::UNLOCK,
            Self::Erg(_) => op::SET_ERG,
            Self::SimMode { .. } => op::SET_SIM_MODE,
            Self::Grade(_) => op::SET_GRADE,
            Self::WindSpeed(_) => op::SET_WIND_SPEED,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![self.op_code()];
        match *self {
            Self::Unlock => out.extend([0xEE, 0xFC]),
            Self::Erg(watts) => out.extend(u16_scaled(watts, 1.0)),
            Self::SimMode {
                mass_kg,
                crr,
                cw_kg_per_m,
            } => {
                out.extend(u16_scaled(mass_kg, 100.0));
                out.extend(u16_scaled(crr, 10_000.0));
                out.extend(u16_scaled(cw_kg_per_m, 1000.0));
            }
            // −1 … 1 mapped onto 0 … 65535.
            Self::Grade(g) => out.extend(u16_scaled((g.clamp(-1.0, 1.0) + 1.0) / 2.0, 65_535.0)),
            // Offset by 32.768 m/s so negative (tailwind) values fit.
            Self::WindSpeed(v) => out.extend(u16_scaled(v + 32.768, 1000.0)),
        }
        out
    }
}

/// Indication answering a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Response {
    pub op_code: u8,
    /// Result byte (0x01 = success).
    pub result: u8,
}

impl Response {
    pub fn parse(value: &[u8]) -> Result<Self, ParseError> {
        let mut r = Reader::new(value);
        let _response_code = r.u8()?;
        Ok(Self {
            op_code: r.u8()?,
            result: r.u8()?,
        })
    }

    /// Success, or an unlock that was not needed.
    pub fn is_ok(&self) -> bool {
        self.result == 0x01 || self.op_code == op::UNLOCK
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodings() {
        assert_eq!(Request::Unlock.encode(), [0x20, 0xEE, 0xFC]);
        assert_eq!(Request::Erg(250.0).encode(), [0x42, 0xFA, 0x00]);
        assert_eq!(Request::Erg(-5.0).encode(), [0x42, 0x00, 0x00]);
        // 83 kg → 8300, crr 0.004 → 40, cw 0.196 → 196
        assert_eq!(
            Request::SimMode {
                mass_kg: 83.0,
                crr: 0.004,
                cw_kg_per_m: 0.196
            }
            .encode(),
            [0x43, 0x6C, 0x20, 0x28, 0x00, 0xC4, 0x00]
        );
        // 0 % → 32768 (rounded from 32767.5), +100 % → 65535, −100 % → 0
        assert_eq!(Request::Grade(0.0).encode(), [0x46, 0x00, 0x80]);
        assert_eq!(Request::Grade(1.0).encode(), [0x46, 0xFF, 0xFF]);
        assert_eq!(Request::Grade(-3.0).encode(), [0x46, 0x00, 0x00]);
        // +5 %: (1.05 / 2) × 65535 = 34405.875 → 34406 = 0x8666
        assert_eq!(Request::Grade(0.05).encode(), [0x46, 0x66, 0x86]);
        // No wind → 32768.
        assert_eq!(Request::WindSpeed(0.0).encode(), [0x47, 0x00, 0x80]);
    }

    /// Byte sequences recorded from a Wahoo KICKR (marge, November 2026).
    #[test]
    fn recorded_responses() {
        let grade = Response::parse(&[1, 70, 1, 0, 30, 133]).unwrap();
        assert_eq!((grade.op_code, grade.result), (op::SET_GRADE, 1));
        assert!(grade.is_ok());
        assert!(Response::parse(&[1, 67, 1, 0]).unwrap().is_ok()); // SIM mode
        assert!(Response::parse(&[1, 66, 1, 0, 120, 0]).unwrap().is_ok()); // ERG 120 W
        // Unlock answered with result 2: not needed, still fine.
        assert!(Response::parse(&[1, 32, 2]).unwrap().is_ok());
        // Any other non-1 result is a failure.
        assert!(!Response::parse(&[1, 70, 2]).unwrap().is_ok());
        assert!(Response::parse(&[1, 70]).is_err());
    }
}
