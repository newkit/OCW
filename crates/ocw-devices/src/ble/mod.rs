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
pub use btle::{BleDevice, BtleLink, scan};
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_from_advertised_services() {
        assert_eq!(
            Profile::from_services(&[0x180D, 0x1826, 0x180F]),
            vec![Profile::FitnessMachine, Profile::HeartRate]
        );
        assert!(Profile::from_services(&[0x180F]).is_empty());
    }
}
