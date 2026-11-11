//! Trainer and sensor access (spec §3.4).
//!
//! Every hardware stack is a [`DeviceBackend`]. Backends run on their own
//! thread and talk to the app only through channels, so this crate never
//! depends on the engine. Phase 1 ships the [`VirtualTrainer`] (power set by
//! the user, e.g. via the console command `sim.power`). The Bluetooth (FTMS)
//! backend follows in Phase 2; ANT+ is deferred.

mod virtual_trainer;

use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

pub use virtual_trainer::VirtualTrainer;

/// A measurement from a device.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DeviceEvent {
    /// Power (and optionally cadence) from a trainer or power meter.
    Power {
        watts: f64,
        cadence_rpm: Option<f64>,
        /// Time since the backend started.
        timestamp: Duration,
    },
    /// Heart rate from a heart-rate sensor.
    HeartRate { bpm: f64, timestamp: Duration },
    /// The backend lost its device.
    Disconnected,
}

/// Commands sent from the app to a backend.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DeviceCommand {
    /// Simulation mode: road gradient (rise over run) to be applied as resistance.
    SetGrade(f64),
    /// ERG mode: target power in watts.
    SetTargetPower(f64),
    /// Virtual devices only: the power to report, in watts.
    SetVirtualPower(f64),
    /// Stop the backend thread.
    Shutdown,
}

/// The app side of a running backend.
pub struct DeviceHandle {
    pub name: String,
    pub events: Receiver<DeviceEvent>,
    pub commands: Sender<DeviceCommand>,
}

#[derive(Debug, thiserror::Error)]
pub enum DeviceError {
    #[error("failed to start device backend: {0}")]
    Start(String),
}

/// A source of trainer / sensor data.
pub trait DeviceBackend: Send + 'static {
    /// Human-readable backend name, e.g. "Virtual trainer".
    fn name(&self) -> &str;

    /// Starts the backend on its own thread.
    fn start(self) -> Result<DeviceHandle, DeviceError>;
}
