//! Trainer and sensor access (spec §3.4).
//!
//! Every hardware stack is a [`DeviceBackend`]. Backends run on their own
//! thread (natively) or task (the browser's event loop) and talk to the app
//! only through channels, so this crate never depends on the engine.
//! Backends: the [`VirtualTrainer`] (power set by the user, e.g. via the
//! console command `sim.power`; [`FrameVirtualTrainer`] on the browser's
//! frame loop) and the Bluetooth LE backends ([`ble`]: FTMS trainers,
//! Wahoo KICKRs, power meters, cadence and heart-rate sensors – over
//! btleplug natively, over Web Bluetooth in the browser, spec §14 Phase 10
//! B4, with the same protocol). ANT+ is deferred.

pub mod ble;
pub mod gatt;
pub mod virtual_power;
#[cfg(not(target_arch = "wasm32"))]
mod virtual_trainer;

use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

pub use virtual_power::FrameVirtualTrainer;
#[cfg(not(target_arch = "wasm32"))]
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
    /// The device is connected and subscribed (again, after a reconnect);
    /// `kind` says what it is ("smart trainer (FTMS)", "heart-rate
    /// sensor", …: [`ble::protocol::TrainerProtocol::describe`]).
    Connected { kind: &'static str },
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
    /// Headwind along the rider's travel (m/s, negative = tailwind; spec
    /// §14 Phase 9, S6), for trainers that simulate wind.
    pub wind_speed_mps: f64,
    /// Rolling resistance coefficient.
    pub crr: f64,
    /// Wind resistance coefficient 0.5 · ρ · CdA, in kg/m.
    pub cw_kg_per_m: f64,
    /// Rider + bike mass in kg (Wahoo trainers simulate inertia with it;
    /// FTMS has no field for it).
    pub mass_kg: f64,
    /// The gradient with the wind folded in (spec §14 Phase 9, Q5), for
    /// trainers that cannot simulate wind; `grade` in calm air.
    pub wind_grade: f64,
}

impl SimulationParameters {
    /// What a trainer simulates of these parameters (spec §14 Phase 9,
    /// Q5): one that `takes_wind` (FTMS Indoor Bike Simulation, Wahoo, the
    /// virtual trainer) the road's gradient and the headwind; one without
    /// wind support the gradient with the wind folded in and no wind.
    /// Returns (gradient, wind in m/s).
    pub fn load(&self, takes_wind: bool) -> (f64, f64) {
        if takes_wind {
            (self.grade, self.wind_speed_mps)
        } else {
            (self.wind_grade, 0.0)
        }
    }
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

    /// Starts the backend on its own thread (in the browser: a task on the
    /// page's event loop).
    fn start(self) -> Result<DeviceHandle, DeviceError>;
}
