//! Trainer and sensor access (spec §3.4).
//!
//! Every hardware stack is a [`DeviceBackend`]. Backends run on their own
//! thread and talk to the app only through channels, so this crate never
//! depends on the engine. Backends: the [`VirtualTrainer`] (power set by
//! the user, e.g. via the console command `sim.power`) and the Bluetooth LE
//! backend ([`ble`]: FTMS trainers, power meters, cadence and heart-rate
//! sensors). ANT+ is deferred.

pub mod ble;
pub mod gatt;
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
    /// SIM mode: simulate this road (FTMS *Set Indoor Bike Simulation Parameters*).
    SetSimulation(SimulationParameters),
    /// ERG mode: target power in watts.
    SetTargetPower(f64),
    /// Virtual devices only: the power to report, in watts.
    SetVirtualPower(f64),
    /// Stop the backend thread.
    Shutdown,
}

/// Road parameters for a trainer in SIM mode.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SimulationParameters {
    /// Gradient as rise over run (0.05 = 5 %), after trainer difficulty.
    pub grade: f64,
    pub wind_speed_mps: f64,
    /// Rolling resistance coefficient.
    pub crr: f64,
    /// Wind resistance coefficient 0.5 · ρ · CdA, in kg/m.
    pub cw_kg_per_m: f64,
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
    #[error("Bluetooth: {0}")]
    Bluetooth(String),
}

/// A source of trainer / sensor data.
pub trait DeviceBackend: Send + 'static {
    /// Human-readable backend name, e.g. "Virtual trainer".
    fn name(&self) -> &str;

    /// Starts the backend on its own thread.
    fn start(self) -> Result<DeviceHandle, DeviceError>;
}
