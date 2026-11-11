//! Bluetooth LE GATT payloads of the fitness profiles (spec §3.4, §5.3).
//!
//! Pure parsing and encoding, no Bluetooth stack: the BLE backend hands raw
//! characteristic values to these functions. All multi-byte fields are
//! little-endian, as in every Bluetooth SIG profile.
//!
//! - [`ftms`]: Fitness Machine Service (Indoor Bike Data, Feature, Control Point)
//! - [`cycling_power`]: Cycling Power Measurement and Feature
//! - [`csc`]: Cycling Speed and Cadence Measurement
//! - [`heart_rate`]: Heart Rate Measurement
//! - [`cadence`]: cadence from cumulative crank revolutions

pub mod cadence;
pub mod csc;
pub mod cycling_power;
pub mod ftms;
pub mod heart_rate;

/// Bluetooth SIG assigned numbers (16-bit UUIDs) used by OCW.
pub mod uuids {
    pub const FITNESS_MACHINE_SERVICE: u16 = 0x1826;
    pub const CYCLING_POWER_SERVICE: u16 = 0x1818;
    pub const CYCLING_SPEED_CADENCE_SERVICE: u16 = 0x1816;
    pub const HEART_RATE_SERVICE: u16 = 0x180D;

    pub const FITNESS_MACHINE_FEATURE: u16 = 0x2ACC;
    pub const INDOOR_BIKE_DATA: u16 = 0x2AD2;
    pub const SUPPORTED_RESISTANCE_LEVEL_RANGE: u16 = 0x2AD6;
    pub const SUPPORTED_POWER_RANGE: u16 = 0x2AD8;
    pub const FITNESS_MACHINE_CONTROL_POINT: u16 = 0x2AD9;
    pub const FITNESS_MACHINE_STATUS: u16 = 0x2ADA;
    pub const CYCLING_POWER_MEASUREMENT: u16 = 0x2A63;
    pub const CYCLING_POWER_FEATURE: u16 = 0x2A65;
    pub const CSC_MEASUREMENT: u16 = 0x2A5B;
    pub const HEART_RATE_MEASUREMENT: u16 = 0x2A37;

    /// The full 128-bit UUID of a 16-bit assigned number, as a string
    /// (`0000xxxx-0000-1000-8000-00805f9b34fb`).
    pub fn to_uuid128(short: u16) -> String {
        format!("0000{short:04x}-0000-1000-8000-00805f9b34fb")
    }

    /// The 16-bit assigned number of a SIG base UUID, if it is one.
    pub fn from_uuid128(uuid: &str) -> Option<u16> {
        let u = uuid.to_ascii_lowercase();
        let (head, tail) = u.split_at_checked(8)?;
        if tail != "-0000-1000-8000-00805f9b34fb" || !head.starts_with("0000") {
            return None;
        }
        u16::from_str_radix(&head[4..], 16).ok()
    }
}

/// Errors when decoding a characteristic value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    #[error("value too short: needed {needed} bytes at offset {offset}, have {len}")]
    TooShort {
        offset: usize,
        needed: usize,
        len: usize,
    },
    #[error("unexpected op code 0x{0:02x}")]
    UnexpectedOpCode(u8),
}

/// Little-endian reader over a characteristic value.
#[derive(Debug, Clone)]
pub(crate) struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn take<const N: usize>(&mut self) -> Result<[u8; N], ParseError> {
        let bytes = self
            .data
            .get(self.pos..self.pos + N)
            .ok_or(ParseError::TooShort {
                offset: self.pos,
                needed: N,
                len: self.data.len(),
            })?;
        self.pos += N;
        Ok(bytes.try_into().expect("slice has length N"))
    }

    pub(crate) fn u8(&mut self) -> Result<u8, ParseError> {
        Ok(self.take::<1>()?[0])
    }
    pub(crate) fn u16(&mut self) -> Result<u16, ParseError> {
        Ok(u16::from_le_bytes(self.take()?))
    }
    pub(crate) fn i16(&mut self) -> Result<i16, ParseError> {
        Ok(i16::from_le_bytes(self.take()?))
    }
    pub(crate) fn u24(&mut self) -> Result<u32, ParseError> {
        let b = self.take::<3>()?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], 0]))
    }
    pub(crate) fn u32(&mut self) -> Result<u32, ParseError> {
        Ok(u32::from_le_bytes(self.take()?))
    }
    pub(crate) fn skip(&mut self, n: usize) -> Result<(), ParseError> {
        if self.pos + n > self.data.len() {
            return Err(ParseError::TooShort {
                offset: self.pos,
                needed: n,
                len: self.data.len(),
            });
        }
        self.pos += n;
        Ok(())
    }
    pub(crate) fn remaining(&self) -> &'a [u8] {
        &self.data[self.pos..]
    }
}

/// Whether bit `n` of `flags` is set.
pub(crate) fn bit(flags: u32, n: u32) -> bool {
    flags & (1 << n) != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_round_trip() {
        let s = uuids::to_uuid128(uuids::INDOOR_BIKE_DATA);
        assert_eq!(s, "00002ad2-0000-1000-8000-00805f9b34fb");
        assert_eq!(uuids::from_uuid128(&s), Some(0x2AD2));
        assert_eq!(
            uuids::from_uuid128("00002AD2-0000-1000-8000-00805F9B34FB"),
            Some(0x2AD2)
        );
        // Vendor-specific 128-bit UUIDs are not SIG numbers.
        assert_eq!(
            uuids::from_uuid128("a026ee0b-0a7d-4ab3-97fa-f1500f9feb8b"),
            None
        );
        assert_eq!(uuids::from_uuid128("short"), None);
    }

    #[test]
    fn reader_reports_truncation() {
        let mut r = Reader::new(&[0x01, 0x02, 0x03]);
        assert_eq!(r.u16().unwrap(), 0x0201);
        assert_eq!(
            r.u16(),
            Err(ParseError::TooShort {
                offset: 2,
                needed: 2,
                len: 3
            })
        );
        assert_eq!(r.u8().unwrap(), 0x03);
        assert!(r.skip(1).is_err());
        let mut r = Reader::new(&[0xFF, 0xFF, 0x7F, 0x00]);
        assert_eq!(r.u24().unwrap(), 0x7F_FFFF);
        assert_eq!(Reader::new(&[0x18, 0xFC]).i16().unwrap(), -1000);
    }
}
