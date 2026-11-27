//! ANT+ sensors through an ANT USB stick (spec 3.4.1, 14 Phase 17):
//! heart-rate straps first, bike power meters and FE-C trainers on the
//! same base.
//!
//! Layers (as [`crate::ble`]):
//! - [`codec`] (pure): the ANT serial messages (sync, length, id, data,
//!   checksum), a resyncing stream decoder, the message ids and codes;
//! - [`channel`] (pure): one channel's state machine – the setup commands
//!   one at a time, each waiting for its response; searching, tracking,
//!   lost, closed;
//! - [`hr`] (pure): the ANT+ heart-rate pages (toggle bit, legacy straps,
//!   beat time / count, R-R intervals, staleness, manufacturer / product /
//!   battery pages); [`power`] (pure): the bike-power pages – power-only
//!   0x10, wheel and crank torque 0x11 / 0x12, calibration 0x01;
//!   [`spdcad`] (pure): speed and cadence sensors (types 121 – 123);
//!   [`common`] (pure): the common pages 0x50 – 0x52 (manufacturer,
//!   product, battery: the FIT file's `device_info`);
//!   [`fec`] (pure): the FE-C trainer pages – data pages 16 / 25 and the
//!   control pages 49 / 50 / 51 / 55, sent as acknowledged messages one
//!   at a time;
//! - [`stick`] (pure): the stick core – reset, capabilities, network key,
//!   extended data once; channels handed out to sensors and the scan,
//!   messages routed by channel number, reconnects. Driven by bytes and a
//!   clock ([`std::time::Duration`] since start), so it runs anywhere;
//! - [`sim`] (pure): a simulated stick with simulated straps and power
//!   meters that answers like a real one (tests, `--ant-sim`);
//! - `service` (native): the stick service on its own thread over an
//!   [`AntTransport`] (open the stick) / [`AntLink`] (bytes in and out),
//!   the [`AntStick`] handle the app shares and [`AntDevice`], the
//!   [`crate::DeviceBackend`];
//! - `usb` (native, feature `ant-usb`): the transport over `nusb` (pure
//!   Rust: Linux usbfs with the kernel's serial driver detached, Windows
//!   WinUSB, macOS IOKit);
//! - [`libusb0`] (native): the transport over Garmin's own libusb-win32
//!   driver on Windows (spec 14 Phase 17 Q7 (d)): the libusb-0.1 calls
//!   behind the [`libusb0::Usb0`] seam (the system's `libusb0.dll`, loaded
//!   at run time on Windows with `ant-usb`; a fake in tests), and
//!   [`Fallback`], which tries `nusb` (WinUSB) first and then it;
//! - [`web`] (pure, the browser binding wasm32 only): the stick over the
//!   browser's WebUSB (spec 14 Phase 17 A5) – the same [`stick::StickCore`]
//!   on the page's event loop.
//!
//! The ANT+ network key is never in this crate (spec 14 Phase 17 Q1 (b)):
//! the rider enters it once, the app keeps it in `secrets.toml` and hands
//! it in as a [`NetworkKey`], whose `Debug` never shows it.

pub mod channel;
pub mod codec;
pub mod common;
pub mod fec;
pub mod hr;
pub mod power;
pub mod sim;
pub mod spdcad;
pub mod stick;
pub mod web;

#[cfg(not(target_arch = "wasm32"))]
pub mod libusb0;
#[cfg(not(target_arch = "wasm32"))]
mod service;
#[cfg(all(not(target_arch = "wasm32"), feature = "ant-usb"))]
pub mod usb;

#[cfg(not(target_arch = "wasm32"))]
pub use service::{
    AntDevice, AntLink, AntStick, AntTransport, Fallback, SimTransport, fallback_error,
};

use std::fmt;
use std::time::Duration;

use crate::ble::{FoundDevice, Profile};

/// The ANT+ radio frequency: 2457 MHz (2400 + 57).
pub const ANT_PLUS_FREQUENCY: u8 = 57;

/// Pause before a closed channel searches again (the same as BLE's).
pub const RECONNECT_DELAY: Duration = crate::ble::RECONNECT_DELAY;

/// USB vendor id of Garmin / Dynastream ANT sticks.
pub const USB_VENDOR: u16 = 0x0fcf;
/// ANT USB2 (older, larger).
pub const USB_PRODUCT_USB2: u16 = 0x1008;
/// ANT USB-m (the current, small one; most clones copy it).
pub const USB_PRODUCT_USBM: u16 = 0x1009;

/// The sticks' names by USB product id.
pub fn stick_name(product: u16) -> &'static str {
    match product {
        USB_PRODUCT_USBM => "ANT USB-m stick",
        USB_PRODUCT_USB2 => "ANT USB2 stick",
        _ => "ANT USB stick",
    }
}

/// The 8-byte network key of a network (for ANT+: the ANT+ network key,
/// entered by the rider). `Debug` and `Display` never show the bytes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct NetworkKey([u8; 8]);

impl NetworkKey {
    /// A made-up key for tests and the simulated stick (not the ANT+ key).
    pub const TEST: Self = Self([0x0A, 0x1B, 0x2C, 0x3D, 0x4E, 0x5F, 0x60, 0x71]);

    pub fn new(bytes: [u8; 8]) -> Self {
        Self(bytes)
    }

    pub fn bytes(&self) -> &[u8; 8] {
        &self.0
    }

    /// Reads a key as people copy it: 16 hex digits, with or without
    /// spaces, commas, colons, dashes or `0x` before each byte
    /// ("B9 A5 …", "0xB9, 0xA5, …", "b9a5…").
    pub fn parse(text: &str) -> Result<Self, KeyError> {
        let mut digits = String::new();
        let lower = text.trim().to_ascii_lowercase();
        let cleaned = lower.replace("0x", " ");
        for c in cleaned.chars() {
            match c {
                c if c.is_ascii_hexdigit() => digits.push(c),
                ' ' | ',' | ':' | '-' | '_' | '\t' | '{' | '}' | '[' | ']' => {}
                _ => return Err(KeyError::NotHex),
            }
        }
        if digits.len() != 16 {
            return Err(KeyError::Length(digits.len()));
        }
        let mut out = [0u8; 8];
        for (i, b) in out.iter_mut().enumerate() {
            *b = u8::from_str_radix(&digits[2 * i..2 * i + 2], 16).map_err(|_| KeyError::NotHex)?;
        }
        Ok(Self(out))
    }

    /// 16 hex digits (for `secrets.toml` only – never for logs).
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02X}")).collect()
    }
}

impl fmt::Debug for NetworkKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NetworkKey(<hidden>)")
    }
}

impl fmt::Display for NetworkKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<hidden>")
    }
}

/// Why a typed key is not a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    #[error("the key has letters that are not hex digits (0-9, A-F)")]
    NotHex,
    #[error("the key has 16 hex digits (8 bytes); this has {0}")]
    Length(usize),
}

/// The ANT+ device profiles OCW receives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AntProfile {
    /// Heart-rate monitor (device type 120, period 8070).
    HeartRate,
    /// Bike power (device type 11, period 8182): the power-only page, the
    /// torque pages, calibration.
    BikePower,
    /// Combined speed and cadence sensor (device type 121, period 8086).
    SpeedCadence,
    /// Cadence sensor (device type 122, period 8102).
    Cadence,
    /// Speed sensor (device type 123, period 8118).
    Speed,
    /// Fitness equipment, FE-C (device type 17, period 8192): a smart
    /// trainer, data pages 16 / 25 and control (SIM / ERG).
    FitnessEquipment,
}

impl AntProfile {
    pub const ALL: [Self; 6] = [
        Self::HeartRate,
        Self::BikePower,
        Self::SpeedCadence,
        Self::Cadence,
        Self::Speed,
        Self::FitnessEquipment,
    ];

    /// The ANT+ device type.
    pub fn device_type(self) -> u8 {
        match self {
            Self::HeartRate => 120,
            Self::BikePower => 11,
            Self::SpeedCadence => 121,
            Self::Cadence => 122,
            Self::Speed => 123,
            Self::FitnessEquipment => 17,
        }
    }

    /// Whether it gives a cadence of its own (a cadence sensor, not a
    /// power meter).
    pub fn is_cadence_sensor(self) -> bool {
        matches!(self, Self::SpeedCadence | Self::Cadence)
    }

    pub fn from_device_type(t: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.device_type() == t & 0x7F)
    }

    /// The channel period (1/32768 s): 8070 ≈ 4.06 Hz, 8182 ≈ 4.00 Hz,
    /// speed / cadence 8086, 8102, 8118 (≈ 4 Hz each), FE-C 8192 = 4 Hz.
    pub fn period(self) -> u16 {
        match self {
            Self::HeartRate => 8070,
            Self::BikePower => 8182,
            Self::SpeedCadence => 8086,
            Self::Cadence => 8102,
            Self::Speed => 8118,
            Self::FitnessEquipment => 8192,
        }
    }

    /// What the device page says once connected.
    pub fn kind(self) -> &'static str {
        match self {
            Self::HeartRate => "heart-rate sensor (ANT+)",
            Self::BikePower => "power meter (ANT+)",
            Self::SpeedCadence => "speed/cadence sensor (ANT+)",
            Self::Cadence => "cadence sensor (ANT+)",
            Self::Speed => "speed sensor (ANT+)",
            Self::FitnessEquipment => "smart trainer (ANT+ FE-C)",
        }
    }

    /// The list's name: "ANT+ heart rate 12345".
    pub fn label(self) -> &'static str {
        match self {
            Self::HeartRate => "ANT+ heart rate",
            Self::BikePower => "ANT+ power",
            Self::SpeedCadence => "ANT+ speed/cadence",
            Self::Cadence => "ANT+ cadence",
            Self::Speed => "ANT+ speed",
            Self::FitnessEquipment => "ANT+ trainer",
        }
    }

    /// The BLE profile with the same role (the Devices page's roles come
    /// from them: `ocw_client::menu::devices::Role::of`).
    pub fn role_profile(self) -> Profile {
        match self {
            Self::HeartRate => Profile::HeartRate,
            Self::BikePower => Profile::CyclingPower,
            Self::SpeedCadence | Self::Cadence | Self::Speed => Profile::SpeedCadence,
            // A trainer, as a BLE FTMS one (it replaces the virtual
            // trainer and takes SIM / ERG).
            Self::FitnessEquipment => Profile::FitnessMachine,
        }
    }
}

/// A channel ID: what identifies a sensor on the air.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ChannelId {
    /// 0 = any (wildcard pairing).
    pub device_number: u16,
    /// The device type (bit 7 is the pairing bit); 0 = any.
    pub device_type: u8,
    /// 0 = any.
    pub trans_type: u8,
}

impl ChannelId {
    pub fn new(device_number: u16, device_type: u8, trans_type: u8) -> Self {
        Self {
            device_number,
            device_type,
            trans_type,
        }
    }

    /// Device number 0 or transmission type 0: matches several sensors.
    pub fn is_wildcard(&self) -> bool {
        self.device_number == 0 || self.trans_type == 0
    }

    /// Whether `other` (a sensor's real id) matches this search id.
    pub fn matches(&self, other: &ChannelId) -> bool {
        (self.device_number == 0 || self.device_number == other.device_number)
            && (self.device_type & 0x7F == 0 || self.device_type & 0x7F == other.device_type & 0x7F)
            && (self.trans_type == 0 || self.trans_type == other.trans_type)
    }

    pub fn to_bytes(self) -> [u8; 4] {
        let [lo, hi] = self.device_number.to_le_bytes();
        [lo, hi, self.device_type, self.trans_type]
    }

    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        match b {
            [lo, hi, t, tt, ..] => Some(Self::new(u16::from_le_bytes([*lo, *hi]), *t, *tt)),
            _ => None,
        }
    }
}

/// A paired ANT+ device as the app knows it: `ant:<type>:<number>:<trans>`
/// (`ant:120:12345:1`), the id in the Devices page and `devices.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AntDeviceId {
    pub profile: AntProfile,
    pub device_number: u16,
    pub trans_type: u8,
}

impl AntDeviceId {
    /// The prefix of every ANT id.
    pub const PREFIX: &'static str = "ant:";

    pub fn new(profile: AntProfile, device_number: u16, trans_type: u8) -> Self {
        Self {
            profile,
            device_number,
            trans_type,
        }
    }

    pub fn is_ant(id: &str) -> bool {
        id.starts_with(Self::PREFIX)
    }

    pub fn parse(id: &str) -> Option<Self> {
        let rest = id.strip_prefix(Self::PREFIX)?;
        let mut parts = rest.split(':');
        let t: u8 = parts.next()?.parse().ok()?;
        let n: u16 = parts.next()?.parse().ok()?;
        let tt: u8 = parts.next().map_or(Some(0), |s| s.parse().ok())?;
        if parts.next().is_some() || n == 0 {
            return None;
        }
        Some(Self::new(AntProfile::from_device_type(t)?, n, tt))
    }

    pub fn of(id: ChannelId) -> Option<Self> {
        (id.device_number != 0).then_some(Self::new(
            AntProfile::from_device_type(id.device_type)?,
            id.device_number,
            id.trans_type,
        ))
    }

    /// The channel ID to pair with exactly this device.
    pub fn channel_id(&self) -> ChannelId {
        ChannelId::new(
            self.device_number,
            self.profile.device_type(),
            self.trans_type,
        )
    }

    /// "ANT+ heart rate 12345".
    pub fn name(&self) -> String {
        format!("{} {}", self.profile.label(), self.device_number)
    }

    /// As a scan result.
    pub fn found(&self, rssi: Option<i8>) -> FoundDevice {
        FoundDevice {
            id: self.to_string(),
            name: Some(self.name()),
            profiles: vec![self.profile.role_profile()],
            rssi: rssi.map(i16::from),
        }
    }
}

impl fmt::Display for AntDeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}{}:{}:{}",
            Self::PREFIX,
            self.profile.device_type(),
            self.device_number,
            self.trans_type
        )
    }
}

/// What the stick service says about the stick (the Devices page's ANT+
/// row).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum StickStatus {
    /// No network key yet: nothing is opened (the rider enters it first).
    #[default]
    NoKey,
    /// No ANT stick plugged in.
    NoStick,
    /// A stick is plugged in (not opened: no ANT+ device in use).
    Found(String),
    /// Opened and set up (its name).
    Ready(String),
    /// Being opened and set up.
    Opening(String),
    /// It is there but cannot be opened (permission; the fix depends on the
    /// system: spec 14 Phase 17 Q7 / Q8).
    NoPermission(String),
    /// Another program uses it.
    Busy(String),
    /// It is there, but no driver we can use is bound to it (Windows:
    /// neither Garmin's libusb-win32 driver nor WinUSB; Q7 (d)).
    NoDriver(String),
    /// Something else failed.
    Failed(String),
}

impl StickStatus {
    /// For the Devices page.
    pub fn text(&self) -> String {
        match self {
            Self::NoKey => "network key needed".into(),
            Self::NoStick => "no stick".into(),
            Self::Found(n) => format!("{n}, ready"),
            Self::Ready(n) => format!("{n}, in use"),
            Self::Opening(n) => format!("{n}, opening ..."),
            Self::NoPermission(_) => "cannot open the stick (permission)".into(),
            Self::Busy(_) => "the stick is busy with another app".into(),
            Self::NoDriver(_) => "the stick needs a driver".into(),
            Self::Failed(e) => format!("failed: {e}"),
        }
    }

    /// The stick's name, if one is plugged in.
    pub fn stick(&self) -> Option<&str> {
        match self {
            Self::Found(n)
            | Self::Ready(n)
            | Self::Opening(n)
            | Self::NoPermission(n)
            | Self::Busy(n)
            | Self::NoDriver(n) => Some(n),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_parse_as_people_copy_them_and_never_print() {
        let k = NetworkKey::parse("0A 1B 2C 3D 4E 5F 60 71").unwrap();
        assert_eq!(k, NetworkKey::TEST);
        assert_eq!(
            NetworkKey::parse("0x0a, 0x1b, 0x2c, 0x3d, 0x4e, 0x5f, 0x60, 0x71").unwrap(),
            k
        );
        assert_eq!(NetworkKey::parse("0a1b2c3d4e5f6071").unwrap(), k);
        assert_eq!(NetworkKey::parse("0A:1B:2C:3D:4E:5F:60:71").unwrap(), k);
        assert_eq!(k.to_hex(), "0A1B2C3D4E5F6071");
        assert_eq!(NetworkKey::parse("0a1b2c"), Err(KeyError::Length(6)));
        assert_eq!(NetworkKey::parse("zz1b2c3d4e5f6071"), Err(KeyError::NotHex));
        // Never in logs.
        let shown = format!("{k:?} {k}");
        assert!(!shown.to_ascii_uppercase().contains("0A1B"), "{shown}");
        assert!(!shown.to_ascii_lowercase().contains("1b"), "{shown}");
    }

    #[test]
    fn device_ids_round_trip() {
        let id = AntDeviceId::new(AntProfile::HeartRate, 12345, 1);
        assert_eq!(id.to_string(), "ant:120:12345:1");
        assert_eq!(AntDeviceId::parse("ant:120:12345:1"), Some(id));
        assert_eq!(id.name(), "ANT+ heart rate 12345");
        let p = AntDeviceId::parse("ant:11:777:5").unwrap();
        assert_eq!(p.profile, AntProfile::BikePower);
        assert_eq!(AntDeviceId::parse("ant:120:0:1"), None);
        let c = AntDeviceId::parse("ant:121:2222:1").unwrap();
        assert_eq!(c.profile, AntProfile::SpeedCadence);
        assert_eq!(c.name(), "ANT+ speed/cadence 2222");
        assert_eq!(c.found(None).profiles, [Profile::SpeedCadence]);
        assert_eq!(
            AntDeviceId::parse("ant:122:7:1").unwrap().profile,
            AntProfile::Cadence
        );
        assert_eq!(
            AntDeviceId::parse("ant:123:8:1").unwrap().profile,
            AntProfile::Speed
        );
        assert_eq!(AntProfile::Speed.period(), 8118);
        let t = AntDeviceId::parse("ant:17:5:5").unwrap();
        assert_eq!(t.profile, AntProfile::FitnessEquipment);
        assert_eq!(t.name(), "ANT+ trainer 5");
        assert_eq!(t.found(None).profiles, [Profile::FitnessMachine]);
        assert_eq!(t.profile.period(), 8192);
        assert_eq!(AntDeviceId::parse("ant:18:5:1"), None);
        assert_eq!(AntDeviceId::parse("hci0/dev_AA"), None);
        assert!(AntDeviceId::is_ant("ant:120:1:1"));
        let f = id.found(Some(-55));
        assert_eq!(f.profiles, [Profile::HeartRate]);
        assert_eq!(f.rssi, Some(-55));
    }

    #[test]
    fn channel_ids_match_wildcards() {
        let any_hr = ChannelId::new(0, 120, 0);
        let strap = ChannelId::new(12345, 120, 1);
        assert!(any_hr.is_wildcard() && !strap.is_wildcard());
        assert!(any_hr.matches(&strap));
        // The pairing bit does not matter.
        assert!(ChannelId::new(12345, 120 | 0x80, 1).matches(&strap));
        assert!(!ChannelId::new(0, 11, 0).matches(&strap));
        assert!(!ChannelId::new(54321, 120, 0).matches(&strap));
        assert_eq!(ChannelId::from_bytes(&strap.to_bytes()), Some(strap));
    }
}
