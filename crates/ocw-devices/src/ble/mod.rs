//! Bluetooth LE backends (spec §3.4, §5.3): FTMS smart trainers, Wahoo
//! KICKRs, Cycling Power meters, cadence and heart-rate sensors.
//!
//! Layers, shared by the desktop and the browser app (spec §14 Phase 10
//! B4: one protocol, no second implementation):
//! - [`protocol`]: pure state machine (which data to use, FTMS / Wahoo control),
//! - [`runner`]: async loop moving bytes between a [`GattLink`] and the
//!   protocol, and the reconnect loop ([`runner::supervise`]), on any
//!   executor (a [`runner::Timer`] gives it time),
//! - [`web`]: the [`GattLink`] over the browser's Web Bluetooth objects
//!   (pure adapter, tested natively against a fake; the JS bindings and the
//!   `WebBluetoothBackend` are wasm32 only),
//! - `btle` (native): the btleplug [`GattLink`], scanning and the
//!   [`BleDevice`] backend (own thread with a Tokio runtime).

pub mod protocol;
pub mod runner;
pub mod web;

#[cfg(not(target_arch = "wasm32"))]
mod btle;

use std::time::Duration;

#[cfg(not(target_arch = "wasm32"))]
pub use btle::{BleDevice, BtleLink, scan, scan_live};
pub use runner::{GattLink, Notifications, RunEnd};

use crate::gatt::uuids;

/// Pause between reconnection attempts.
pub const RECONNECT_DELAY: Duration = Duration::from_secs(5);

/// Fitness profiles a device advertises.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Profile {
    FitnessMachine,
    CyclingPower,
    SpeedCadence,
    HeartRate,
}

impl Profile {
    pub fn service(self) -> u16 {
        match self {
            Self::FitnessMachine => uuids::FITNESS_MACHINE_SERVICE,
            Self::CyclingPower => uuids::CYCLING_POWER_SERVICE,
            Self::SpeedCadence => uuids::CYCLING_SPEED_CADENCE_SERVICE,
            Self::HeartRate => uuids::HEART_RATE_SERVICE,
        }
    }

    pub fn all() -> [Self; 4] {
        [
            Self::FitnessMachine,
            Self::CyclingPower,
            Self::SpeedCadence,
            Self::HeartRate,
        ]
    }

    /// Profiles among advertised service numbers.
    pub fn from_services(services: &[u16]) -> Vec<Self> {
        Self::all()
            .into_iter()
            .filter(|p| services.contains(&p.service()))
            .collect()
    }
}

/// A device found by a scan (native: `scan`).
#[derive(Debug, Clone, PartialEq)]
pub struct FoundDevice {
    /// Platform identifier, used to connect (stable per machine).
    pub id: String,
    pub name: Option<String>,
    pub profiles: Vec<Profile>,
    pub rssi: Option<i16>,
}

/// Adds a found device to `list`, or updates the one with its id (name,
/// profiles, signal). Returns whether anything changed (a new device, or
/// a new name or signal to show).
pub fn merge_found(list: &mut Vec<FoundDevice>, d: FoundDevice) -> bool {
    match list.iter_mut().find(|f| f.id == d.id) {
        Some(f) if *f == d => false,
        Some(f) => {
            // A later report without a name or signal keeps the known one.
            let name = d.name.or_else(|| f.name.clone());
            let rssi = d.rssi.or(f.rssi);
            let changed = name != f.name || rssi != f.rssi || d.profiles != f.profiles;
            *f = FoundDevice { name, rssi, ..d };
            changed
        }
        None => {
            list.push(d);
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn found_devices_merge_by_id() {
        let d = |id: &str, name: Option<&str>, rssi| FoundDevice {
            id: id.into(),
            name: name.map(Into::into),
            profiles: vec![Profile::CyclingPower],
            rssi,
        };
        let mut l = Vec::new();
        assert!(merge_found(&mut l, d("a", Some("KICKR"), None)));
        assert!(!merge_found(&mut l, d("a", Some("KICKR"), None)));
        // A signal arrives: an update.
        assert!(merge_found(&mut l, d("a", None, Some(-60))));
        assert_eq!(l[0].name.as_deref(), Some("KICKR"));
        assert_eq!(l[0].rssi, Some(-60));
        // Nothing new (no name, no signal): no update.
        assert!(!merge_found(&mut l, d("a", None, None)));
        assert!(merge_found(&mut l, d("b", None, None)));
        assert_eq!(l.len(), 2);
    }

    #[test]
    fn profiles_from_advertised_services() {
        assert_eq!(
            Profile::from_services(&[0x180D, 0x1826, 0x180F]),
            vec![Profile::FitnessMachine, Profile::HeartRate]
        );
        assert!(Profile::from_services(&[0x180F]).is_empty());
    }
}
