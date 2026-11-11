//! Fitness Machine Service (FTMS v1.0): Indoor Bike Data, features, control point.

use super::{ParseError, Reader, bit};

/// Indoor Bike Data (0x2AD2) notification. Absent fields are `None`.
///
/// A trainer may split its data over several notifications ("More Data"
/// flag); each one is parsed on its own.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct IndoorBikeData {
    pub speed_kmh: Option<f64>,
    pub average_speed_kmh: Option<f64>,
    pub cadence_rpm: Option<f64>,
    pub average_cadence_rpm: Option<f64>,
    pub total_distance_m: Option<u32>,
    pub resistance_level: Option<i16>,
    pub power_w: Option<i16>,
    pub average_power_w: Option<i16>,
    pub total_energy_kcal: Option<u16>,
    pub heart_rate_bpm: Option<u8>,
    pub elapsed_time_s: Option<u16>,
    pub remaining_time_s: Option<u16>,
}

impl IndoorBikeData {
    pub fn parse(value: &[u8]) -> Result<Self, ParseError> {
        let mut r = Reader::new(value);
        let flags = u32::from(r.u16()?);
        let mut d = Self::default();
        // Bit 0 is "More Data": the instantaneous speed is present when it is 0.
        if !bit(flags, 0) {
            d.speed_kmh = Some(f64::from(r.u16()?) / 100.0);
        }
        if bit(flags, 1) {
            d.average_speed_kmh = Some(f64::from(r.u16()?) / 100.0);
        }
        if bit(flags, 2) {
            d.cadence_rpm = Some(f64::from(r.u16()?) / 2.0);
        }
        if bit(flags, 3) {
            d.average_cadence_rpm = Some(f64::from(r.u16()?) / 2.0);
        }
        if bit(flags, 4) {
            d.total_distance_m = Some(r.u24()?);
        }
        if bit(flags, 5) {
            d.resistance_level = Some(r.i16()?);
        }
        if bit(flags, 6) {
            d.power_w = Some(r.i16()?);
        }
        if bit(flags, 7) {
            d.average_power_w = Some(r.i16()?);
        }
        if bit(flags, 8) {
            d.total_energy_kcal = Some(r.u16()?);
            r.skip(3)?; // energy per hour (u16), per minute (u8)
        }
        if bit(flags, 9) {
            d.heart_rate_bpm = Some(r.u8()?);
        }
        if bit(flags, 10) {
            r.skip(1)?; // metabolic equivalent
        }
        if bit(flags, 11) {
            d.elapsed_time_s = Some(r.u16()?);
        }
        if bit(flags, 12) {
            d.remaining_time_s = Some(r.u16()?);
        }
        Ok(d)
    }
}

/// Fitness Machine Feature (0x2ACC): what the trainer measures and accepts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Features {
    pub machine: u32,
    pub target_setting: u32,
}

impl Features {
    pub fn parse(value: &[u8]) -> Result<Self, ParseError> {
        let mut r = Reader::new(value);
        Ok(Self {
            machine: r.u32()?,
            target_setting: r.u32()?,
        })
    }

    pub fn measures_cadence(&self) -> bool {
        bit(self.machine, 1)
    }
    pub fn measures_resistance_level(&self) -> bool {
        bit(self.machine, 7)
    }
    pub fn measures_heart_rate(&self) -> bool {
        bit(self.machine, 10)
    }
    pub fn measures_power(&self) -> bool {
        bit(self.machine, 14)
    }
    /// Accepts *Set Target Resistance Level* (fallback mode).
    pub fn supports_resistance_target(&self) -> bool {
        bit(self.target_setting, 2)
    }
    /// Accepts *Set Target Power* (ERG mode).
    pub fn supports_power_target(&self) -> bool {
        bit(self.target_setting, 3)
    }
    /// Accepts *Set Indoor Bike Simulation Parameters* (SIM mode).
    pub fn supports_simulation(&self) -> bool {
        bit(self.target_setting, 13)
    }
    pub fn supports_spin_down(&self) -> bool {
        bit(self.target_setting, 15)
    }
}

/// Supported Power Range (0x2AD8), in watts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PowerRange {
    pub min_w: i16,
    pub max_w: i16,
    pub increment_w: u16,
}

impl PowerRange {
    pub fn parse(value: &[u8]) -> Result<Self, ParseError> {
        let mut r = Reader::new(value);
        Ok(Self {
            min_w: r.i16()?,
            max_w: r.i16()?,
            increment_w: r.u16()?,
        })
    }

    /// Clamps a target to the range and rounds it to the increment.
    pub fn clamp(&self, watts: f64) -> i16 {
        let w = watts.clamp(f64::from(self.min_w), f64::from(self.max_w));
        let inc = f64::from(self.increment_w.max(1));
        let snapped = f64::from(self.min_w) + ((w - f64::from(self.min_w)) / inc).round() * inc;
        snapped.min(f64::from(self.max_w)) as i16
    }
}

/// A request written to the Fitness Machine Control Point (0x2AD9).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ControlRequest {
    /// Must be accepted before any other request.
    RequestControl,
    Reset,
    /// Resistance level, 0.0–25.5 (unitless, resolution 0.1).
    SetTargetResistance(f64),
    /// ERG mode target in watts.
    SetTargetPower(f64),
    StartOrResume,
    Stop,
    Pause,
    /// SIM mode. `grade` is rise over run (0.05 = 5 %), `crr` the rolling
    /// resistance coefficient, `cw_kg_per_m` the wind resistance coefficient
    /// (0.5 · ρ · CdA).
    SetSimulation {
        wind_speed_mps: f64,
        grade: f64,
        crr: f64,
        cw_kg_per_m: f64,
    },
    StartSpinDown,
    IgnoreSpinDown,
}

pub mod op {
    pub const REQUEST_CONTROL: u8 = 0x00;
    pub const RESET: u8 = 0x01;
    pub const SET_TARGET_RESISTANCE: u8 = 0x04;
    pub const SET_TARGET_POWER: u8 = 0x05;
    pub const START_OR_RESUME: u8 = 0x07;
    pub const STOP_OR_PAUSE: u8 = 0x08;
    pub const SET_INDOOR_BIKE_SIMULATION: u8 = 0x11;
    pub const SPIN_DOWN_CONTROL: u8 = 0x13;
    pub const RESPONSE: u8 = 0x80;
}

fn scaled_i16(value: f64, scale: f64) -> [u8; 2] {
    ((value * scale)
        .round()
        .clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16)
        .to_le_bytes()
}

fn scaled_u8(value: f64, scale: f64) -> u8 {
    (value * scale).round().clamp(0.0, 255.0) as u8
}

impl ControlRequest {
    pub fn op_code(&self) -> u8 {
        match self {
            Self::RequestControl => op::REQUEST_CONTROL,
            Self::Reset => op::RESET,
            Self::SetTargetResistance(_) => op::SET_TARGET_RESISTANCE,
            Self::SetTargetPower(_) => op::SET_TARGET_POWER,
            Self::StartOrResume => op::START_OR_RESUME,
            Self::Stop | Self::Pause => op::STOP_OR_PAUSE,
            Self::SetSimulation { .. } => op::SET_INDOOR_BIKE_SIMULATION,
            Self::StartSpinDown | Self::IgnoreSpinDown => op::SPIN_DOWN_CONTROL,
        }
    }

    /// The bytes to write. Out-of-range values are clamped to the field range.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![self.op_code()];
        match *self {
            Self::RequestControl | Self::Reset | Self::StartOrResume => {}
            Self::Stop => out.push(0x01),
            Self::Pause => out.push(0x02),
            Self::SetTargetResistance(level) => out.push(scaled_u8(level, 10.0)),
            Self::SetTargetPower(watts) => out.extend(scaled_i16(watts, 1.0)),
            Self::SetSimulation {
                wind_speed_mps,
                grade,
                crr,
                cw_kg_per_m,
            } => {
                out.extend(scaled_i16(wind_speed_mps, 1000.0));
                // Field unit is 0.01 %; `grade` is a fraction.
                out.extend(scaled_i16(grade, 10_000.0));
                out.push(scaled_u8(crr, 10_000.0));
                out.push(scaled_u8(cw_kg_per_m, 100.0));
            }
            Self::StartSpinDown => out.push(0x01),
            Self::IgnoreSpinDown => out.push(0x02),
        }
        out
    }
}

/// Result code of a control point response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultCode {
    Success,
    NotSupported,
    InvalidParameter,
    OperationFailed,
    ControlNotPermitted,
    Other(u8),
}

impl From<u8> for ResultCode {
    fn from(v: u8) -> Self {
        match v {
            0x01 => Self::Success,
            0x02 => Self::NotSupported,
            0x03 => Self::InvalidParameter,
            0x04 => Self::OperationFailed,
            0x05 => Self::ControlNotPermitted,
            other => Self::Other(other),
        }
    }
}

/// Control point indication answering a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlResponse {
    pub request_op_code: u8,
    pub result: ResultCode,
    /// Response parameters (e.g. spin-down target speeds).
    pub parameters: Vec<u8>,
}

impl ControlResponse {
    pub fn parse(value: &[u8]) -> Result<Self, ParseError> {
        let mut r = Reader::new(value);
        let code = r.u8()?;
        if code != op::RESPONSE {
            return Err(ParseError::UnexpectedOpCode(code));
        }
        Ok(Self {
            request_op_code: r.u8()?,
            result: r.u8()?.into(),
            parameters: r.remaining().to_vec(),
        })
    }

    pub fn is_success(&self) -> bool {
        self.result == ResultCode::Success
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Speed, cadence, power and heart rate in one notification.
    #[test]
    fn indoor_bike_data_typical_notification() {
        // flags 0x0244: cadence (bit 2), power (bit 6), heart rate (bit 9);
        // bit 0 clear, so the speed is present.
        let value = [
            0x44, 0x02, // flags
            0xAD, 0x0C, // speed 3245 × 0.01 = 32.45 km/h
            0xB4, 0x00, // cadence 180 × 0.5 = 90 rpm
            0xFA, 0x00, // power 250 W
            0x8E, // heart rate 142 bpm
        ];
        let d = IndoorBikeData::parse(&value).unwrap();
        assert_eq!(d.speed_kmh, Some(32.45));
        assert_eq!(d.cadence_rpm, Some(90.0));
        assert_eq!(d.power_w, Some(250));
        assert_eq!(d.heart_rate_bpm, Some(142));
        assert_eq!(d.total_distance_m, None);
    }

    #[test]
    fn indoor_bike_data_all_fields() {
        let value = [
            0xFF, 0x1F, // flags: bits 0–12, "More Data" set (no speed)
            0x10, 0x27, // average speed 100.00 km/h
            0x01, 0x00, // cadence 0.5 rpm
            0x02, 0x00, // average cadence 1 rpm
            0x40, 0xE2, 0x01, // distance 123456 m
            0xF6, 0xFF, // resistance -10
            0x18, 0xFC, // power -1000 W (signed)
            0x2C, 0x01, // average power 300 W
            0xE8, 0x03, 0x00, 0x00, 0x00, // energy 1000 kcal, per h, per min
            0x50, // heart rate 80
            0x0A, // MET
            0x10, 0x0E, // elapsed 3600 s
            0x3C, 0x00, // remaining 60 s
        ];
        let d = IndoorBikeData::parse(&value).unwrap();
        assert_eq!(d.speed_kmh, None);
        assert_eq!(d.average_speed_kmh, Some(100.0));
        assert_eq!(d.cadence_rpm, Some(0.5));
        assert_eq!(d.average_cadence_rpm, Some(1.0));
        assert_eq!(d.total_distance_m, Some(123_456));
        assert_eq!(d.resistance_level, Some(-10));
        assert_eq!(d.power_w, Some(-1000));
        assert_eq!(d.average_power_w, Some(300));
        assert_eq!(d.total_energy_kcal, Some(1000));
        assert_eq!(d.heart_rate_bpm, Some(80));
        assert_eq!(d.elapsed_time_s, Some(3600));
        assert_eq!(d.remaining_time_s, Some(60));
    }

    #[test]
    fn indoor_bike_data_truncated_is_an_error() {
        // Power flagged but missing.
        assert!(matches!(
            IndoorBikeData::parse(&[0x44, 0x00, 0xAD, 0x0C, 0xB4, 0x00]),
            Err(ParseError::TooShort { .. })
        ));
        assert!(IndoorBikeData::parse(&[0x44]).is_err());
    }

    #[test]
    fn features() {
        // Machine: cadence, resistance level, heart rate, power.
        // Target: resistance, power, simulation, spin down.
        let machine: u32 = (1 << 1) | (1 << 7) | (1 << 10) | (1 << 14);
        let target: u32 = (1 << 2) | (1 << 3) | (1 << 13) | (1 << 15);
        let mut value = machine.to_le_bytes().to_vec();
        value.extend(target.to_le_bytes());
        let f = Features::parse(&value).unwrap();
        assert!(f.measures_cadence() && f.measures_power() && f.measures_heart_rate());
        assert!(f.measures_resistance_level());
        assert!(f.supports_simulation() && f.supports_power_target());
        assert!(f.supports_resistance_target() && f.supports_spin_down());
        let none = Features::parse(&[0; 8]).unwrap();
        assert!(!none.supports_simulation() && !none.supports_power_target());
        assert!(Features::parse(&[0; 7]).is_err());
    }

    #[test]
    fn power_range_clamps_and_snaps() {
        // 0–2000 W in steps of 5 W.
        let r = PowerRange::parse(&[0x00, 0x00, 0xD0, 0x07, 0x05, 0x00]).unwrap();
        assert_eq!((r.min_w, r.max_w, r.increment_w), (0, 2000, 5));
        assert_eq!(r.clamp(252.4), 250);
        assert_eq!(r.clamp(253.0), 255);
        assert_eq!(r.clamp(-20.0), 0);
        assert_eq!(r.clamp(5000.0), 2000);
        let zero_inc = PowerRange {
            increment_w: 0,
            ..r
        };
        assert_eq!(zero_inc.clamp(101.4), 101);
    }

    #[test]
    fn control_point_encoding() {
        assert_eq!(ControlRequest::RequestControl.encode(), [0x00]);
        assert_eq!(ControlRequest::Reset.encode(), [0x01]);
        assert_eq!(ControlRequest::StartOrResume.encode(), [0x07]);
        assert_eq!(ControlRequest::Stop.encode(), [0x08, 0x01]);
        assert_eq!(ControlRequest::Pause.encode(), [0x08, 0x02]);
        assert_eq!(
            ControlRequest::SetTargetPower(250.0).encode(),
            [0x05, 0xFA, 0x00]
        );
        assert_eq!(
            ControlRequest::SetTargetPower(1e6).encode(),
            [0x05, 0xFF, 0x7F]
        );
        assert_eq!(
            ControlRequest::SetTargetResistance(12.3).encode(),
            [0x04, 123]
        );
        assert_eq!(
            ControlRequest::SetTargetResistance(99.0).encode(),
            [0x04, 255]
        );
        assert_eq!(ControlRequest::StartSpinDown.encode(), [0x13, 0x01]);
        assert_eq!(ControlRequest::IgnoreSpinDown.encode(), [0x13, 0x02]);
    }

    #[test]
    fn simulation_parameters_encoding() {
        let sim = ControlRequest::SetSimulation {
            wind_speed_mps: 0.0,
            grade: 0.045, // 4.5 % → 450 × 0.01 %
            crr: 0.004,   // 40 × 0.0001
            cw_kg_per_m: 0.51,
        };
        assert_eq!(sim.encode(), [0x11, 0x00, 0x00, 0xC2, 0x01, 0x28, 0x33]);
        let downhill = ControlRequest::SetSimulation {
            wind_speed_mps: -1.5,
            grade: -0.08,
            crr: 0.0,
            cw_kg_per_m: 0.0,
        };
        // wind -1500 = 0xFA24, grade -800 = 0xFCE0
        assert_eq!(
            downhill.encode(),
            [0x11, 0x24, 0xFA, 0xE0, 0xFC, 0x00, 0x00]
        );
    }

    #[test]
    fn control_point_responses() {
        let ok = ControlResponse::parse(&[0x80, 0x11, 0x01]).unwrap();
        assert!(ok.is_success());
        assert_eq!(ok.request_op_code, op::SET_INDOOR_BIKE_SIMULATION);
        let denied = ControlResponse::parse(&[0x80, 0x05, 0x05]).unwrap();
        assert_eq!(denied.result, ResultCode::ControlNotPermitted);
        let spin = ControlResponse::parse(&[0x80, 0x13, 0x01, 0x10, 0x0E, 0x34, 0x08]).unwrap();
        assert_eq!(spin.parameters, [0x10, 0x0E, 0x34, 0x08]);
        assert_eq!(
            ControlResponse::parse(&[0x80, 0x00, 0x09]).unwrap().result,
            ResultCode::Other(9)
        );
        assert_eq!(
            ControlResponse::parse(&[0x05, 0x00, 0x01]),
            Err(ParseError::UnexpectedOpCode(0x05))
        );
        assert!(ControlResponse::parse(&[0x80, 0x00]).is_err());
    }
}
