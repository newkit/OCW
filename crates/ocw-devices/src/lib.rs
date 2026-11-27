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
//! B4, with the same protocol) and the ANT+ sensors through an ANT USB
//! stick ([`ant`]: heart-rate straps and power meters, spec 3.4.1 and 14
//! Phase 17; the codec, channels, pages and the stick core are pure, the
//! stick service and its USB link native).

pub mod ant;
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
    /// A connection attempt failed (sent before the `Disconnected` that
    /// starts the next attempt): why, for the menu's status line.
    Failed { reason: FailReason },
    /// Cadence from a cadence sensor (ANT+ speed / cadence sensors, spec
    /// 14 Phase 17, 17E): crank revolutions, 0 once the crank stops. Power
    /// events carry the cadence of trainers and power meters.
    Cadence { rpm: f64, timestamp: Duration },
    /// Wheel revolutions per second from a speed sensor (0 once the wheel
    /// stops); the app turns it into a speed with the wheel's
    /// circumference ([`wheel_speed_mps`]). Information only: the ride's
    /// speed comes from the physics (5.2).
    WheelSpeed {
        revs_per_s: f64,
        timestamp: Duration,
    },
    /// A power meter answered a calibration ([`DeviceCommand::Calibrate`],
    /// [`DeviceCommand::SetAutoZero`]).
    Calibration(Calibration),
    /// What the sensor says about itself (manufacturer, product, serial,
    /// versions, battery): sent once connected and again when it changes
    /// (the FIT file's `device_info`, spec 14 Phase 17 A6).
    Info(SensorInfo),
}

/// The wheel's speed (m/s) from its revolutions per second.
pub fn wheel_speed_mps(revs_per_s: f64, circumference_m: f64) -> f64 {
    revs_per_s * circumference_m
}

/// A power meter's answer to a calibration (ANT+ common page 0x01).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Calibration {
    /// Manual zero done: the meter's zero offset (its own units; `None`
    /// if it does not say) and whether auto zero is on (`None`: not
    /// supported or not said).
    Done {
        offset: Option<i16>,
        auto_zero: Option<bool>,
    },
    /// The meter says the calibration failed (pedals moving?).
    Failed { offset: Option<i16> },
    /// No answer: the request was not delivered or not answered in time.
    NoAnswer,
    /// The meter supports auto zero, and whether it is on (its own page).
    AutoZero { on: bool },
}

impl Calibration {
    /// For the device page: "done, offset 512", "failed", ...
    pub fn text(&self) -> String {
        match self {
            Self::Done { offset, auto_zero } => {
                let mut s = "done".to_owned();
                if let Some(o) = offset {
                    s.push_str(&format!(", offset {o}"));
                }
                if let Some(a) = auto_zero {
                    s.push_str(if *a {
                        ", auto zero on"
                    } else {
                        ", auto zero off"
                    });
                }
                s
            }
            Self::Failed { offset: Some(o) } => {
                format!("failed (offset {o}): keep the cranks still")
            }
            Self::Failed { offset: None } => "failed: keep the cranks still".into(),
            Self::NoAnswer => "no answer from the meter".into(),
            Self::AutoZero { on } => format!("auto zero {}", if *on { "on" } else { "off" }),
        }
    }
}

/// How a sensor is reached (the FIT `source_type`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SensorSource {
    /// An ANT+ sensor: its device type, device number and transmission
    /// type.
    AntPlus {
        device_type: u8,
        device_number: u16,
        trans_type: u8,
    },
}

/// What a sensor says about itself (ANT+ common pages 80 / 81 / 82, the
/// heart-rate and speed / cadence background pages). `None`: not said
/// (yet).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SensorInfo {
    pub source: SensorSource,
    /// The ANT+ / FIT manufacturer id (1 Garmin, 32 Wahoo, …).
    pub manufacturer: Option<u16>,
    /// The manufacturer's model number.
    pub product: Option<u16>,
    pub serial: Option<u32>,
    pub hardware_version: Option<u8>,
    /// The software version (3.12 = 3.12).
    pub software_version: Option<f64>,
    pub battery_volts: Option<f64>,
    /// 1 new, 2 good, 3 ok, 4 low, 5 critical (ANT+ and FIT alike).
    pub battery_status: Option<u8>,
    pub battery_pct: Option<u8>,
}

impl SensorInfo {
    pub fn new(source: SensorSource) -> Self {
        Self {
            source,
            manufacturer: None,
            product: None,
            serial: None,
            hardware_version: None,
            software_version: None,
            battery_volts: None,
            battery_status: None,
            battery_pct: None,
        }
    }

    /// The same sensor (by its source).
    pub fn same(&self, other: &SensorInfo) -> bool {
        self.source == other.source
    }
}

/// Why a connection attempt failed (spec §3.2.1 *Devices*): what the menu
/// tells the rider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailReason {
    /// No Bluetooth adapter, or Bluetooth is off.
    NoAdapter,
    /// The device did not show up (out of range, asleep, or connected to
    /// another app: a trainer takes one connection per profile).
    NotFound,
    /// It showed up, but connecting or setting it up failed.
    Connect,
    /// ANT+: no ANT USB stick is plugged in.
    NoStick,
    /// ANT+: the stick is there but may not be opened (permission).
    NoPermission,
    /// ANT+: another program has the stick.
    StickBusy,
    /// ANT+: the ANT+ network key has not been entered yet.
    NoKey,
    /// ANT+: the stick is there but no driver we can use is bound to it
    /// (Windows: neither Garmin's libusb-win32 driver nor WinUSB).
    NoDriver,
}

impl FailReason {
    /// Classifies a backend error.
    pub fn of(e: &DeviceError) -> Self {
        match e {
            DeviceError::NoAdapter => Self::NoAdapter,
            DeviceError::NotFound(_) => Self::NotFound,
            DeviceError::Start(_) | DeviceError::Bluetooth(_) | DeviceError::Ant(_) => {
                Self::Connect
            }
            DeviceError::NoStick => Self::NoStick,
            DeviceError::StickPermission => Self::NoPermission,
            DeviceError::StickBusy => Self::StickBusy,
            DeviceError::NoNetworkKey => Self::NoKey,
            DeviceError::StickDriver(_) => Self::NoDriver,
        }
    }

    /// For the menu and the console.
    pub fn text(self) -> &'static str {
        match self {
            Self::NoAdapter => "no Bluetooth adapter (or Bluetooth is off)",
            Self::NotFound => "not found (out of range, asleep, or connected to another app)",
            Self::Connect => "the connection failed (busy with another app?)",
            Self::NoStick => "no ANT stick plugged in",
            Self::NoPermission => "cannot open the ANT stick (permission)",
            Self::StickBusy => "the ANT stick is busy with another app",
            Self::NoKey => "the ANT+ network key is needed (Devices -> ANT+)",
            Self::NoDriver => "the ANT stick needs a driver (Devices -> ANT+)",
        }
    }
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
    /// Power meters: a manual zero offset calibration (ANT+ common page
    /// 0x01; the cranks still, unloaded); answered by
    /// [`DeviceEvent::Calibration`].
    Calibrate,
    /// Power meters that support it: auto zero on or off.
    SetAutoZero(bool),
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
    #[error("no Bluetooth adapter found (or Bluetooth is off)")]
    NoAdapter,
    #[error("device {0} not found")]
    NotFound(String),
    #[error("ANT: {0}")]
    Ant(String),
    #[error("no ANT stick plugged in")]
    NoStick,
    #[error("cannot open the ANT stick (permission)")]
    StickPermission,
    #[error("the ANT stick is busy with another app")]
    StickBusy,
    #[error("the ANT+ network key is needed")]
    NoNetworkKey,
    /// The stick is plugged in, but no driver we can use is bound to it
    /// (the details for the log).
    #[error("the ANT stick has no usable driver: {0}")]
    StickDriver(String),
}

/// A source of trainer / sensor data.
pub trait DeviceBackend: Send + 'static {
    /// Human-readable backend name, e.g. "Virtual trainer".
    fn name(&self) -> &str;

    /// Starts the backend on its own thread (in the browser: a task on the
    /// page's event loop).
    fn start(self) -> Result<DeviceHandle, DeviceError>;
}
