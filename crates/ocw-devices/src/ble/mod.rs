//! Bluetooth LE backend (spec §3.4): FTMS smart trainers, Cycling Power
//! meters, cadence and heart-rate sensors, via `btleplug` (BlueZ on Linux,
//! WinRT on Windows, CoreBluetooth on macOS).
//!
//! Layers:
//! - [`protocol`]: pure state machine (which data to use, FTMS control),
//! - [`runner`]: async loop moving bytes between a [`GattLink`] and the protocol,
//! - this module: btleplug implementation of [`GattLink`], scanning, and the
//!   [`BleDevice`] backend (own thread with a Tokio runtime, auto-reconnect).

pub mod protocol;
pub mod runner;

use std::collections::BTreeMap;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use btleplug::api::{
    Central, CharPropFlags, Characteristic, Manager as _, Peripheral as _, ScanFilter, WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures::StreamExt;
use uuid::Uuid;

pub use runner::{GattLink, Notifications, RunEnd};

use crate::gatt::uuids;
use crate::{DeviceBackend, DeviceCommand, DeviceError, DeviceEvent, DeviceHandle};

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

/// A device found by [`scan`].
#[derive(Debug, Clone, PartialEq)]
pub struct FoundDevice {
    /// Platform identifier, used to connect (stable per machine).
    pub id: String,
    pub name: Option<String>,
    pub profiles: Vec<Profile>,
    pub rssi: Option<i16>,
}

fn sig_uuid(short: u16) -> Uuid {
    Uuid::from_u128((u128::from(short) << 96) | 0x0000_0000_0000_1000_8000_0080_5f9b_34fb)
}

fn short_uuid(uuid: &Uuid) -> Option<u16> {
    uuids::from_uuid128(&uuid.to_string())
}

fn ble_err(e: impl std::fmt::Display) -> DeviceError {
    DeviceError::Bluetooth(e.to_string())
}

async fn first_adapter() -> Result<Adapter, DeviceError> {
    let manager = Manager::new().await.map_err(ble_err)?;
    manager
        .adapters()
        .await
        .map_err(ble_err)?
        .into_iter()
        .next()
        .ok_or_else(|| DeviceError::Bluetooth("no Bluetooth adapter found".into()))
}

fn scan_filter() -> ScanFilter {
    ScanFilter {
        services: Profile::all()
            .into_iter()
            .map(|p| sig_uuid(p.service()))
            .collect(),
    }
}

async fn describe(p: &Peripheral) -> Option<FoundDevice> {
    let props = p.properties().await.ok()??;
    let services: Vec<u16> = props.services.iter().filter_map(short_uuid).collect();
    let profiles = Profile::from_services(&services);
    if profiles.is_empty() {
        return None;
    }
    Some(FoundDevice {
        id: p.id().to_string(),
        name: props.local_name.or(props.advertisement_name),
        profiles,
        rssi: props.rssi,
    })
}

/// Scans for fitness devices for `duration` (blocking; own runtime).
pub fn scan(duration: Duration) -> Result<Vec<FoundDevice>, DeviceError> {
    runtime()?.block_on(async {
        let adapter = first_adapter().await?;
        adapter.start_scan(scan_filter()).await.map_err(ble_err)?;
        tokio::time::sleep(duration).await;
        let _ = adapter.stop_scan().await;
        let mut found = Vec::new();
        for p in adapter.peripherals().await.map_err(ble_err)? {
            if let Some(d) = describe(&p).await {
                found.push(d);
            }
        }
        found.sort_by_key(|d| std::cmp::Reverse(d.rssi));
        Ok(found)
    })
}

fn runtime() -> Result<tokio::runtime::Runtime, DeviceError> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| DeviceError::Start(e.to_string()))
}

/// [`GattLink`] over a connected btleplug peripheral.
pub struct BtleLink {
    peripheral: Peripheral,
    characteristics: BTreeMap<u16, Characteristic>,
}

impl BtleLink {
    async fn connect(peripheral: Peripheral) -> Result<Self, DeviceError> {
        if !peripheral.is_connected().await.unwrap_or(false) {
            peripheral.connect().await.map_err(ble_err)?;
        }
        peripheral.discover_services().await.map_err(ble_err)?;
        let characteristics = peripheral
            .characteristics()
            .into_iter()
            .filter_map(|c| short_uuid(&c.uuid).map(|s| (s, c)))
            .collect();
        Ok(Self {
            peripheral,
            characteristics,
        })
    }

    fn get(&self, c: u16) -> Result<&Characteristic, DeviceError> {
        self.characteristics
            .get(&c)
            .ok_or_else(|| DeviceError::Bluetooth(format!("characteristic 0x{c:04x} not found")))
    }
}

impl GattLink for BtleLink {
    fn characteristics(&self) -> Vec<u16> {
        self.characteristics.keys().copied().collect()
    }

    async fn read(&self, c: u16) -> Result<Vec<u8>, DeviceError> {
        self.peripheral.read(self.get(c)?).await.map_err(ble_err)
    }

    async fn write(&self, c: u16, value: &[u8]) -> Result<(), DeviceError> {
        let ch = self.get(c)?;
        let kind = if ch.properties.contains(CharPropFlags::WRITE) {
            WriteType::WithResponse
        } else {
            WriteType::WithoutResponse
        };
        self.peripheral
            .write(ch, value, kind)
            .await
            .map_err(ble_err)
    }

    async fn subscribe(&self, c: u16) -> Result<(), DeviceError> {
        self.peripheral
            .subscribe(self.get(c)?)
            .await
            .map_err(ble_err)
    }

    async fn notifications(&self) -> Result<Notifications, DeviceError> {
        let stream = self.peripheral.notifications().await.map_err(ble_err)?;
        Ok(Box::pin(stream.filter_map(|n| async move {
            short_uuid(&n.uuid).map(|c| (c, n.value))
        })))
    }

    async fn is_connected(&self) -> bool {
        self.peripheral.is_connected().await.unwrap_or(false)
    }
}

/// How long to look for the device before each connection attempt.
const FIND_TIMEOUT: Duration = Duration::from_secs(20);
/// Pause between reconnection attempts.
const RECONNECT_DELAY: Duration = Duration::from_secs(5);

/// A Bluetooth LE trainer or sensor, identified by the id from [`scan`].
/// Reconnects automatically until shut down.
pub struct BleDevice {
    id: String,
    name: String,
}

impl BleDevice {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
        }
    }
}

impl DeviceBackend for BleDevice {
    fn name(&self) -> &str {
        &self.name
    }

    fn start(self) -> Result<DeviceHandle, DeviceError> {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (ev_tx, ev_rx) = mpsc::channel();
        let rt = runtime()?;
        let id = self.id.clone();
        std::thread::Builder::new()
            .name(format!("ocw-ble-{}", self.name))
            .spawn(move || rt.block_on(device_loop(id, cmd_rx, ev_tx)))
            .map_err(|e| DeviceError::Start(e.to_string()))?;
        Ok(DeviceHandle {
            name: self.name,
            events: ev_rx,
            commands: cmd_tx,
        })
    }
}

async fn find(adapter: &Adapter, id: &str) -> Result<Option<Peripheral>, DeviceError> {
    adapter.start_scan(scan_filter()).await.map_err(ble_err)?;
    let deadline = Instant::now() + FIND_TIMEOUT;
    let mut found = None;
    while Instant::now() < deadline && found.is_none() {
        found = adapter
            .peripherals()
            .await
            .map_err(ble_err)?
            .into_iter()
            .find(|p| p.id().to_string() == id);
        if found.is_none() {
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }
    let _ = adapter.stop_scan().await;
    Ok(found)
}

async fn device_loop(id: String, commands: Receiver<DeviceCommand>, events: Sender<DeviceEvent>) {
    let started = Instant::now();
    let mut last_target = None;
    loop {
        let attempt = async {
            let adapter = first_adapter().await?;
            let Some(p) = find(&adapter, &id).await? else {
                return Err(DeviceError::Bluetooth(format!("device {id} not found")));
            };
            let link = BtleLink::connect(p).await?;
            runner::run(&link, &commands, &events, started, &mut last_target).await
        };
        match attempt.await {
            Ok(RunEnd::Shutdown) => return,
            Ok(RunEnd::Disconnected) => tracing::warn!(device = %id, "disconnected"),
            Err(e) => tracing::warn!(device = %id, "{e}"),
        }
        if events.send(DeviceEvent::Disconnected).is_err() {
            return; // app gone
        }
        // Wait before reconnecting, but react to shutdown (and keep the latest target).
        let until = Instant::now() + RECONNECT_DELAY;
        while let Some(left) = until.checked_duration_since(Instant::now()) {
            match commands.recv_timeout(left.min(Duration::from_millis(200))) {
                Ok(DeviceCommand::Shutdown) | Err(RecvTimeoutError::Disconnected) => return,
                Ok(cmd @ (DeviceCommand::SetSimulation(_) | DeviceCommand::SetTargetPower(_))) => {
                    last_target = Some(cmd);
                }
                Ok(_) | Err(RecvTimeoutError::Timeout) => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sig_uuids_round_trip() {
        let u = sig_uuid(uuids::FITNESS_MACHINE_SERVICE);
        assert_eq!(u.to_string(), "00001826-0000-1000-8000-00805f9b34fb");
        assert_eq!(short_uuid(&u), Some(0x1826));
        assert_eq!(short_uuid(&Uuid::nil()), None);
    }

    #[test]
    fn profiles_from_advertised_services() {
        assert_eq!(
            Profile::from_services(&[0x180D, 0x1826, 0x180F]),
            vec![Profile::FitnessMachine, Profile::HeartRate]
        );
        assert!(Profile::from_services(&[0x180F]).is_empty());
        let filter = scan_filter();
        assert_eq!(filter.services.len(), 4);
    }
}
