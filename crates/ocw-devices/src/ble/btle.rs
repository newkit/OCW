//! The btleplug implementation (native: BlueZ on Linux, WinRT on Windows,
//! CoreBluetooth on macOS): [`BtleLink`] (a [`GattLink`]), [`scan`], and the
//! [`BleDevice`] backend (own thread with a Tokio runtime, auto-reconnect
//! through the shared [`runner::supervise`]).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use btleplug::api::{
    Central, CharPropFlags, Characteristic, Manager as _, Peripheral as _, ScanFilter, WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures::StreamExt;
use uuid::Uuid;

use super::runner::{self, GattLink, Notifications, TokioTimer};
use super::{FoundDevice, Profile, RECONNECT_DELAY, merge_found};
use crate::gatt::uuids;
use crate::{DeviceBackend, DeviceCommand, DeviceError, DeviceEvent, DeviceHandle};

fn sig_uuid(short: u16) -> Uuid {
    Uuid::from_u128((u128::from(short) << 96) | 0x0000_0000_0000_1000_8000_0080_5f9b_34fb)
}

fn short_uuid(uuid: &Uuid) -> Option<u16> {
    uuids::short_of(&uuid.to_string())
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
        .ok_or(DeviceError::NoAdapter)
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
    let mut found: Vec<FoundDevice> = Vec::new();
    scan_live(duration, &AtomicBool::new(false), &mut |d| {
        merge_found(&mut found, d);
    })?;
    found.sort_by_key(|d| std::cmp::Reverse(d.rssi));
    Ok(found)
}

/// How often a live scan looks at the adapter's devices.
const SCAN_POLL: Duration = Duration::from_millis(500);

/// Scans for fitness devices for up to `duration` (blocking; own runtime)
/// and reports each one as it appears, and again when its name or signal
/// changes (spec §3.2.1 *Devices*: the list grows live). `stop` ends the
/// scan early.
pub fn scan_live(
    duration: Duration,
    stop: &AtomicBool,
    on_found: &mut dyn FnMut(FoundDevice),
) -> Result<(), DeviceError> {
    runtime()?.block_on(async {
        let adapter = first_adapter().await?;
        adapter.start_scan(scan_filter()).await.map_err(ble_err)?;
        let deadline = Instant::now() + duration;
        let mut seen: Vec<FoundDevice> = Vec::new();
        let result = loop {
            match adapter.peripherals().await {
                Ok(ps) => {
                    for p in ps {
                        if let Some(d) = describe(&p).await
                            && merge_found(&mut seen, d.clone())
                        {
                            on_found(d);
                        }
                    }
                }
                Err(e) => break Err(ble_err(e)),
            }
            if stop.load(Ordering::Relaxed) || Instant::now() >= deadline {
                break Ok(());
            }
            tokio::time::sleep(SCAN_POLL.min(deadline.saturating_duration_since(Instant::now())))
                .await;
        };
        let _ = adapter.stop_scan().await;
        result
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
    let connect = || async {
        let adapter = first_adapter().await?;
        let Some(p) = find(&adapter, &id).await? else {
            return Err(DeviceError::NotFound(id.clone()));
        };
        BtleLink::connect(p).await
    };
    runner::supervise(
        connect,
        &commands,
        &events,
        &TokioTimer,
        RECONNECT_DELAY,
        &id,
    )
    .await;
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
        let wahoo = Uuid::parse_str(crate::gatt::wahoo::CONTROL_POINT_UUID).unwrap();
        assert_eq!(short_uuid(&wahoo), Some(uuids::WAHOO_TRAINER_CONTROL));
    }

    #[test]
    fn scan_filter_has_the_four_profiles() {
        assert_eq!(scan_filter().services.len(), 4);
    }
}
