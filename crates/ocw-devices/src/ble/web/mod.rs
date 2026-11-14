//! Trainers and sensors over the browser's Web Bluetooth (spec §14 Phase 10
//! B4; Chromium only, Q1).
//!
//! The browser hands the page `BluetoothDevice` objects (after its device
//! chooser: [`DeviceKind::request_options`]) with GATT services and
//! characteristics addressed by 128-bit UUID strings. This module turns
//! them into a [`GattLink`] ([`WebGattLink`]) so the **same** runner,
//! protocol state machine and GATT codecs as on the desktop drive them:
//! nothing of FTMS, Cycling Power, CSC, Heart Rate or the Wahoo control is
//! implemented twice.
//!
//! - [`WebGatt`] is the boundary: the few things the adapter needs from a
//!   browser device (connect, discover, read, write, notifications, the
//!   disconnect event). In the browser it is a thin binding over
//!   `navigator.bluetooth` (`browser`, wasm32 only); in the tests a fake.
//! - [`WebGattLink`] (pure): service discovery → SIG numbers (and Wahoo's
//!   128-bit control point), the write type per characteristic, the
//!   notifications as a stream that ends with the disconnect event.
//! - [`run_device`] (pure): [`runner::supervise`] over a device: connect,
//!   subscribe, SIM / ERG writes, reconnect after a drop, `disconnect()` at
//!   shutdown.
//! - [`Remembered`]: the devices this origin used (kind, browser id, name),
//!   kept in the page's storage; with the browser's permission
//!   persistence (`navigator.bluetooth.getDevices()`) they reconnect
//!   without the chooser.

use std::cell::RefCell;
use std::collections::BTreeMap;

use futures::StreamExt;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};

use super::Profile;
use super::runner::{self, GattLink, Notifications, RunEnd, Timer};
use crate::gatt::{uuids, wahoo};
use crate::{DeviceCommand, DeviceError, DeviceEvent};

#[cfg(target_arch = "wasm32")]
mod browser;
#[cfg(target_arch = "wasm32")]
pub use browser::{
    BluetoothDevice, JsGatt, JsTimer, WebBluetoothBackend, availability, has_web_bluetooth,
    permitted_devices, request,
};

/// Wahoo's own services on KICKRs (asked for as optional services, so the
/// control point is reachable whichever service a model puts it in; the
/// common one is in Cycling Power).
pub const WAHOO_SERVICES: [&str; 2] = [
    "a026ee01-0a7d-4ab3-97fa-f1500f9feb8b",
    "a026ee0b-0a7d-4ab3-97fa-f1500f9feb8b",
];

/// What a page button asks the browser's chooser for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DeviceKind {
    /// A smart trainer: FTMS, or a Wahoo KICKR (Cycling Power + Wahoo
    /// control); it also delivers power.
    Trainer,
    /// A power meter (Cycling Power) or a speed / cadence sensor (CSC).
    PowerMeter,
    HeartRate,
}

impl DeviceKind {
    pub const ALL: [Self; 3] = [Self::Trainer, Self::PowerMeter, Self::HeartRate];

    /// The page's and the console's name: `trainer`, `power`, `hr`.
    pub fn key(self) -> &'static str {
        match self {
            Self::Trainer => "trainer",
            Self::PowerMeter => "power",
            Self::HeartRate => "hr",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "trainer" => Some(Self::Trainer),
            "power" | "power-meter" | "cadence" => Some(Self::PowerMeter),
            "hr" | "heart-rate" | "heartrate" => Some(Self::HeartRate),
            _ => None,
        }
    }

    /// For buttons and messages.
    pub fn label(self) -> &'static str {
        match self {
            Self::Trainer => "trainer",
            Self::PowerMeter => "power meter",
            Self::HeartRate => "heart rate",
        }
    }

    /// The services the chooser filters on (any one of them matches).
    pub fn filter_services(self) -> Vec<u16> {
        match self {
            Self::Trainer => vec![uuids::FITNESS_MACHINE_SERVICE, uuids::CYCLING_POWER_SERVICE],
            Self::PowerMeter => vec![
                uuids::CYCLING_POWER_SERVICE,
                uuids::CYCLING_SPEED_CADENCE_SERVICE,
            ],
            Self::HeartRate => vec![uuids::HEART_RATE_SERVICE],
        }
    }

    /// The options of `navigator.bluetooth.requestDevice` as JSON: one
    /// filter per service (and KICKRs by name, which do not always
    /// advertise a service), and every fitness service plus Wahoo's as
    /// optional services – a site may only open the services it asked
    /// for, and a trainer is also a power meter, a strap is sometimes
    /// in a trainer.
    pub fn request_options(self) -> String {
        let mut filters: Vec<String> = self
            .filter_services()
            .into_iter()
            .map(|s| format!(r#"{{"services":["{}"]}}"#, uuids::to_uuid128(s)))
            .collect();
        if self == Self::Trainer {
            filters.push(r#"{"namePrefix":"KICKR"}"#.into());
        }
        let optional: Vec<String> = optional_services()
            .into_iter()
            .map(|u| format!("\"{u}\""))
            .collect();
        format!(
            r#"{{"filters":[{}],"optionalServices":[{}]}}"#,
            filters.join(","),
            optional.join(",")
        )
    }
}

/// Every service OCW reads: the four fitness profiles and Wahoo's.
pub fn optional_services() -> Vec<String> {
    Profile::all()
        .into_iter()
        .map(|p| uuids::to_uuid128(p.service()))
        .chain(WAHOO_SERVICES.iter().map(|s| s.to_string()))
        .collect()
}

/// A characteristic's properties as the browser reports them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CharProps {
    pub read: bool,
    pub write: bool,
    pub write_without_response: bool,
    pub notify: bool,
    pub indicate: bool,
}

/// A characteristic the browser found (service and characteristic UUID
/// strings, as the browser gives them).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebCharacteristic {
    pub service: String,
    pub uuid: String,
    pub props: CharProps,
}

/// What the browser reports between calls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GattEvent {
    /// `characteristicvaluechanged` of the characteristic at `index` (in
    /// the order of the last [`WebGatt::discover`]).
    Value { index: usize, value: Vec<u8> },
    /// `gattserverdisconnected`.
    Disconnected,
}

/// A browser `BluetoothDevice` as the adapter uses it. Indices address the
/// characteristics of the last [`discover`](Self::discover).
#[allow(async_fn_in_trait)] // the page's event loop: one thread
pub trait WebGatt {
    /// `device.gatt.connect()` (also again, after a drop).
    async fn connect(&self) -> Result<(), DeviceError>;
    /// `device.gatt.connected`.
    fn connected(&self) -> bool;
    /// The permitted primary services' characteristics.
    async fn discover(&self) -> Result<Vec<WebCharacteristic>, DeviceError>;
    async fn read(&self, index: usize) -> Result<Vec<u8>, DeviceError>;
    /// `writeValueWithResponse` / `writeValueWithoutResponse`.
    async fn write(
        &self,
        index: usize,
        value: &[u8],
        with_response: bool,
    ) -> Result<(), DeviceError>;
    /// `startNotifications()` (notifications or indications).
    async fn start_notifications(&self, index: usize) -> Result<(), DeviceError>;
    /// Where values and the disconnect go from now on (one sink per
    /// connection).
    fn set_events(&self, sink: UnboundedSender<GattEvent>);
    /// `device.gatt.disconnect()`.
    fn disconnect(&self);
}

/// The [`GattLink`] over a connected [`WebGatt`]: what the shared runner
/// and protocol drive.
pub struct WebGattLink<'a, G: WebGatt> {
    gatt: &'a G,
    /// OCW's number → (index, properties); the first of a number wins.
    by_number: BTreeMap<u16, (usize, CharProps)>,
    events: RefCell<Option<UnboundedReceiver<GattEvent>>>,
}

impl<'a, G: WebGatt> WebGattLink<'a, G> {
    /// Connects, discovers the characteristics OCW knows and takes over
    /// the device's events.
    pub async fn open(gatt: &'a G) -> Result<Self, DeviceError> {
        if !gatt.connected() {
            gatt.connect().await?;
        }
        let found = gatt.discover().await?;
        let (tx, rx) = unbounded();
        gatt.set_events(tx);
        Ok(Self::new(gatt, &found, rx))
    }

    fn new(gatt: &'a G, found: &[WebCharacteristic], events: UnboundedReceiver<GattEvent>) -> Self {
        let mut by_number = BTreeMap::new();
        for (i, c) in found.iter().enumerate() {
            if let Some(n) = uuids::short_of(&c.uuid) {
                by_number.entry(n).or_insert((i, c.props));
            }
        }
        Self {
            gatt,
            by_number,
            events: RefCell::new(Some(events)),
        }
    }

    fn get(&self, c: u16) -> Result<(usize, CharProps), DeviceError> {
        self.by_number
            .get(&c)
            .copied()
            .ok_or_else(|| DeviceError::Bluetooth(format!("characteristic 0x{c:04x} not found")))
    }
}

impl<G: WebGatt> GattLink for WebGattLink<'_, G> {
    fn characteristics(&self) -> Vec<u16> {
        self.by_number.keys().copied().collect()
    }

    async fn read(&self, c: u16) -> Result<Vec<u8>, DeviceError> {
        self.gatt.read(self.get(c)?.0).await
    }

    async fn write(&self, c: u16, value: &[u8]) -> Result<(), DeviceError> {
        let (i, props) = self.get(c)?;
        // With response where the characteristic allows it (control points
        // do), as btleplug's link does.
        let with_response = props.write || !props.write_without_response;
        self.gatt.write(i, value, with_response).await
    }

    async fn subscribe(&self, c: u16) -> Result<(), DeviceError> {
        self.gatt.start_notifications(self.get(c)?.0).await
    }

    async fn notifications(&self) -> Result<Notifications, DeviceError> {
        let rx = self
            .events
            .borrow_mut()
            .take()
            .ok_or_else(|| DeviceError::Bluetooth("notifications taken twice".into()))?;
        let numbers: BTreeMap<usize, u16> =
            self.by_number.iter().map(|(n, (i, _))| (*i, *n)).collect();
        // The stream ends with the disconnect: the runner sees it at once.
        Ok(Box::pin(
            rx.take_while(|e| futures::future::ready(*e != GattEvent::Disconnected))
                .filter_map(move |e| {
                    futures::future::ready(match e {
                        GattEvent::Value { index, value } => {
                            numbers.get(&index).map(|n| (*n, value))
                        }
                        GattEvent::Disconnected => None,
                    })
                }),
        ))
    }

    async fn is_connected(&self) -> bool {
        self.gatt.connected()
    }
}

/// One browser device until shutdown: connect, run the shared protocol,
/// reconnect after a drop (every `reconnect_delay`; the backend uses
/// [`RECONNECT_DELAY`](super::RECONNECT_DELAY)), and `gatt.disconnect()` at the end. The events
/// and commands are the backend's channels (`DeviceHandle`).
pub async fn run_device<G: WebGatt, T: Timer>(
    gatt: &G,
    commands: &std::sync::mpsc::Receiver<DeviceCommand>,
    events: &std::sync::mpsc::Sender<DeviceEvent>,
    timer: &T,
    reconnect_delay: std::time::Duration,
    name: &str,
) -> RunEnd {
    let end = runner::supervise(
        || WebGattLink::open(gatt),
        commands,
        events,
        timer,
        reconnect_delay,
        name,
    )
    .await;
    gatt.disconnect();
    end
}

/// A device this origin used: what it was connected as, the browser's id
/// for it (`BluetoothDevice.id`, per origin) and its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RememberedDevice {
    pub kind: DeviceKind,
    pub id: String,
    pub name: String,
}

/// The devices this origin used, newest first (one per kind and id),
/// stored as text in the page's storage (`localStorage`):
/// `kind<TAB>id<TAB>name` lines.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Remembered {
    pub devices: Vec<RememberedDevice>,
}

impl Remembered {
    /// The page's storage key.
    pub const STORAGE_KEY: &'static str = "ocw.devices";
    /// At most this many are kept.
    pub const MAX: usize = 8;

    /// Reads the stored text; broken lines are skipped.
    pub fn parse(text: &str) -> Self {
        let devices = text
            .lines()
            .filter_map(|l| {
                let mut f = l.splitn(3, '\t');
                let kind = DeviceKind::parse(f.next()?)?;
                let id = f.next()?.trim();
                let name = f.next().unwrap_or("").trim();
                (!id.is_empty()).then(|| RememberedDevice {
                    kind,
                    id: id.into(),
                    name: name.into(),
                })
            })
            .take(Self::MAX)
            .collect();
        Self { devices }
    }

    pub fn to_text(&self) -> String {
        self.devices
            .iter()
            .map(|d| {
                let clean = |s: &str| s.replace(['\t', '\n', '\r'], " ");
                format!("{}\t{}\t{}\n", d.kind.key(), clean(&d.id), clean(&d.name))
            })
            .collect()
    }

    /// Remembers a device (first in the list).
    pub fn remember(&mut self, device: RememberedDevice) {
        self.devices.retain(|d| d.id != device.id);
        self.devices.insert(0, device);
        self.devices.truncate(Self::MAX);
    }

    /// Forgets a device by id or name (any case), or all with `all`;
    /// returns how many were forgotten.
    pub fn forget(&mut self, which: &str) -> usize {
        let before = self.devices.len();
        if which == "all" {
            self.devices.clear();
        } else {
            self.devices
                .retain(|d| d.id != which && !d.name.eq_ignore_ascii_case(which));
        }
        before - self.devices.len()
    }

    /// The kind a browser device was used as (for the permitted devices
    /// of `getDevices()` after a reload).
    pub fn kind_of(&self, id: &str) -> Option<DeviceKind> {
        self.devices.iter().find(|d| d.id == id).map(|d| d.kind)
    }
}

/// Whether the Wahoo control point UUID is the one the adapter maps.
pub fn is_wahoo_control(uuid: &str) -> bool {
    uuid.eq_ignore_ascii_case(wahoo::CONTROL_POINT_UUID)
}

#[cfg(test)]
pub(crate) mod fake {
    //! A fake browser device: the JS objects' behaviour as the adapter
    //! sees it (services, properties, values, indications, drops).

    use std::cell::{Cell, RefCell};

    use super::*;
    use crate::gatt::ftms;

    pub struct FakeDevice {
        pub chars: Vec<WebCharacteristic>,
        pub connected: Cell<bool>,
        pub connects: Cell<u32>,
        /// Connection attempts that fail before one succeeds.
        pub fail_connects: Cell<u32>,
        pub disconnects: Cell<u32>,
        /// (characteristic UUID, value, with response).
        pub writes: RefCell<Vec<(String, Vec<u8>, bool)>>,
        pub notifying: RefCell<Vec<String>>,
        pub sink: RefCell<Option<UnboundedSender<GattEvent>>>,
        /// FTMS Feature value.
        pub features: Vec<u8>,
    }

    fn sig(n: u16) -> String {
        uuids::to_uuid128(n)
    }

    fn ch(service: u16, uuid: String, props: CharProps) -> WebCharacteristic {
        WebCharacteristic {
            service: sig(service),
            uuid,
            props,
        }
    }

    const NOTIFY: CharProps = CharProps {
        read: false,
        write: false,
        write_without_response: false,
        notify: true,
        indicate: false,
    };
    const CONTROL: CharProps = CharProps {
        read: false,
        write: true,
        write_without_response: false,
        notify: false,
        indicate: true,
    };
    const READ: CharProps = CharProps {
        read: true,
        write: false,
        write_without_response: false,
        notify: false,
        indicate: false,
    };

    impl FakeDevice {
        fn with(chars: Vec<WebCharacteristic>) -> Self {
            let mut features = ((1u32 << 1) | (1 << 14)).to_le_bytes().to_vec();
            features.extend(((1u32 << 3) | (1 << 13)).to_le_bytes());
            Self {
                chars,
                connected: Cell::new(false),
                connects: Cell::new(0),
                fail_connects: Cell::new(0),
                disconnects: Cell::new(0),
                writes: RefCell::new(Vec::new()),
                notifying: RefCell::new(Vec::new()),
                sink: RefCell::new(None),
                features,
            }
        }

        /// An FTMS trainer that also offers Cycling Power and the Device
        /// Information's characteristics (which OCW ignores).
        pub fn ftms_trainer() -> Self {
            Self::with(vec![
                ch(0x180A, sig(0x2A29), READ),
                ch(
                    uuids::FITNESS_MACHINE_SERVICE,
                    sig(uuids::FITNESS_MACHINE_FEATURE),
                    READ,
                ),
                ch(
                    uuids::FITNESS_MACHINE_SERVICE,
                    sig(uuids::INDOOR_BIKE_DATA),
                    NOTIFY,
                ),
                ch(
                    uuids::FITNESS_MACHINE_SERVICE,
                    sig(uuids::FITNESS_MACHINE_CONTROL_POINT),
                    CONTROL,
                ),
                ch(
                    uuids::CYCLING_POWER_SERVICE,
                    sig(uuids::CYCLING_POWER_MEASUREMENT),
                    NOTIFY,
                ),
            ])
        }

        /// An older KICKR: Cycling Power and Wahoo's control point (upper
        /// case, as some browsers print it), written with response.
        pub fn kickr() -> Self {
            Self::with(vec![
                ch(
                    uuids::CYCLING_POWER_SERVICE,
                    sig(uuids::CYCLING_POWER_MEASUREMENT),
                    NOTIFY,
                ),
                ch(
                    uuids::CYCLING_POWER_SERVICE,
                    wahoo::CONTROL_POINT_UUID.to_ascii_uppercase(),
                    CONTROL,
                ),
            ])
        }

        pub fn heart_rate_strap() -> Self {
            Self::with(vec![ch(
                uuids::HEART_RATE_SERVICE,
                sig(uuids::HEART_RATE_MEASUREMENT),
                NOTIFY,
            )])
        }

        fn index(&self, number: u16) -> usize {
            self.chars
                .iter()
                .position(|c| uuids::short_of(&c.uuid) == Some(number))
                .expect("offered")
        }

        /// A value from the device (if it notifies on it).
        pub fn notify(&self, number: u16, value: &[u8]) {
            let i = self.index(number);
            if !self.notifying.borrow().contains(&self.chars[i].uuid) {
                return;
            }
            if let Some(s) = self.sink.borrow().as_ref() {
                let _ = s.unbounded_send(GattEvent::Value {
                    index: i,
                    value: value.to_vec(),
                });
            }
        }

        /// The device goes out of range: `gattserverdisconnected`.
        pub fn drop_link(&self) {
            self.connected.set(false);
            self.notifying.borrow_mut().clear();
            if let Some(s) = self.sink.borrow_mut().take() {
                let _ = s.unbounded_send(GattEvent::Disconnected);
            }
        }

        /// The control point writes (bytes), in order.
        pub fn control_writes(&self) -> Vec<Vec<u8>> {
            self.writes.borrow().iter().map(|w| w.1.clone()).collect()
        }
    }

    impl WebGatt for FakeDevice {
        async fn connect(&self) -> Result<(), DeviceError> {
            self.connects.set(self.connects.get() + 1);
            if self.fail_connects.get() > 0 {
                self.fail_connects.set(self.fail_connects.get() - 1);
                return Err(DeviceError::Bluetooth(
                    "NetworkError: Bluetooth Device is no longer in range.".into(),
                ));
            }
            self.connected.set(true);
            Ok(())
        }
        fn connected(&self) -> bool {
            self.connected.get()
        }
        async fn discover(&self) -> Result<Vec<WebCharacteristic>, DeviceError> {
            Ok(self.chars.clone())
        }
        async fn read(&self, index: usize) -> Result<Vec<u8>, DeviceError> {
            match uuids::short_of(&self.chars[index].uuid) {
                Some(uuids::FITNESS_MACHINE_FEATURE) => Ok(self.features.clone()),
                _ => Ok(b"Fake".to_vec()),
            }
        }
        async fn write(
            &self,
            index: usize,
            value: &[u8],
            with_response: bool,
        ) -> Result<(), DeviceError> {
            if !self.connected.get() {
                return Err(DeviceError::Bluetooth(
                    "GATT Server is disconnected.".into(),
                ));
            }
            let c = &self.chars[index];
            self.writes
                .borrow_mut()
                .push((c.uuid.clone(), value.to_vec(), with_response));
            // The trainer answers on the control point (an indication).
            let answer = match uuids::short_of(&c.uuid) {
                Some(uuids::FITNESS_MACHINE_CONTROL_POINT) => {
                    vec![ftms::op::RESPONSE, value[0], 0x01]
                }
                Some(uuids::WAHOO_TRAINER_CONTROL) => vec![0x01, value[0], 0x01],
                _ => return Ok(()),
            };
            if let Some(s) = self.sink.borrow().as_ref() {
                let _ = s.unbounded_send(GattEvent::Value {
                    index,
                    value: answer,
                });
            }
            Ok(())
        }
        async fn start_notifications(&self, index: usize) -> Result<(), DeviceError> {
            self.notifying
                .borrow_mut()
                .push(self.chars[index].uuid.clone());
            Ok(())
        }
        fn set_events(&self, sink: UnboundedSender<GattEvent>) {
            *self.sink.borrow_mut() = Some(sink);
        }
        fn disconnect(&self) {
            self.disconnects.set(self.disconnects.get() + 1);
            self.connected.set(false);
            self.sink.borrow_mut().take();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::fake::FakeDevice;
    use super::*;
    use crate::SimulationParameters;
    use crate::ble::runner::TokioTimer;
    use crate::gatt::ftms;

    fn sim(grade: f64) -> DeviceCommand {
        DeviceCommand::SetSimulation(SimulationParameters {
            grade,
            wind_speed_mps: 0.0,
            crr: 0.004,
            cw_kg_per_m: 0.2,
            mass_kg: 83.0,
        })
    }

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
    }

    async fn ms(n: u64) {
        tokio::time::sleep(Duration::from_millis(n)).await;
    }

    /// The reconnect pause in the tests.
    const AGAIN: Duration = Duration::from_millis(300);

    /// 250 W at 90 rpm as FTMS Indoor Bike Data.
    const BIKE_DATA: [u8; 8] = [0x44, 0x00, 0, 0, 0xB4, 0, 0xFA, 0];

    #[test]
    fn request_options_ask_for_every_fitness_service() {
        let t = DeviceKind::Trainer.request_options();
        assert_eq!(
            t,
            concat!(
                r#"{"filters":[{"services":["00001826-0000-1000-8000-00805f9b34fb"]},"#,
                r#"{"services":["00001818-0000-1000-8000-00805f9b34fb"]},{"namePrefix":"KICKR"}],"#,
                r#""optionalServices":["00001826-0000-1000-8000-00805f9b34fb","#,
                r#""00001818-0000-1000-8000-00805f9b34fb","00001816-0000-1000-8000-00805f9b34fb","#,
                r#""0000180d-0000-1000-8000-00805f9b34fb","a026ee01-0a7d-4ab3-97fa-f1500f9feb8b","#,
                r#""a026ee0b-0a7d-4ab3-97fa-f1500f9feb8b"]}"#
            )
        );
        let hr = DeviceKind::HeartRate.request_options();
        assert!(
            hr.starts_with(r#"{"filters":[{"services":["0000180d-"#),
            "{hr}"
        );
        assert!(!hr.contains("namePrefix"));
        assert!(
            DeviceKind::PowerMeter
                .request_options()
                .contains("00001816-")
        );
        for k in DeviceKind::ALL {
            assert_eq!(DeviceKind::parse(k.key()), Some(k));
        }
        assert_eq!(DeviceKind::parse("Heart-Rate"), Some(DeviceKind::HeartRate));
        assert_eq!(DeviceKind::parse("toaster"), None);
    }

    /// The adapter maps the browser's UUID strings to OCW's numbers (the
    /// protocol ignores the ones it does not use, like the manufacturer
    /// name), and writes with response where the characteristic allows it.
    #[test]
    fn discovery_maps_uuids_and_write_types() {
        let fake = FakeDevice::ftms_trainer();
        let link = rt().block_on(WebGattLink::open(&fake)).unwrap();
        assert_eq!(fake.connects.get(), 1);
        assert_eq!(
            link.characteristics(),
            vec![
                0x2A29,
                uuids::CYCLING_POWER_MEASUREMENT,
                uuids::FITNESS_MACHINE_FEATURE,
                uuids::INDOOR_BIKE_DATA,
                uuids::FITNESS_MACHINE_CONTROL_POINT,
            ]
        );
        rt().block_on(async {
            link.write(uuids::FITNESS_MACHINE_CONTROL_POINT, &[0x00])
                .await
                .unwrap();
            assert!(link.write(0x2A37, &[1]).await.is_err());
            assert_eq!(
                link.read(uuids::FITNESS_MACHINE_FEATURE).await.unwrap(),
                fake.features
            );
        });
        assert_eq!(
            fake.writes.borrow()[0],
            (uuids::to_uuid128(0x2AD9), vec![0x00], true)
        );
        // The KICKR's 128-bit control point, in upper case.
        let kickr = FakeDevice::kickr();
        let link = rt().block_on(WebGattLink::open(&kickr)).unwrap();
        assert_eq!(
            link.characteristics(),
            vec![
                uuids::CYCLING_POWER_MEASUREMENT,
                uuids::WAHOO_TRAINER_CONTROL
            ]
        );
        assert!(is_wahoo_control(&kickr.chars[1].uuid));
    }

    /// Notifications reach the protocol by number; the stream ends with
    /// the browser's disconnect event.
    #[test]
    fn notifications_end_with_the_disconnect() {
        let fake = FakeDevice::ftms_trainer();
        let got = rt().block_on(async {
            let link = WebGattLink::open(&fake).await.unwrap();
            let mut stream = link.notifications().await.unwrap();
            link.subscribe(uuids::INDOOR_BIKE_DATA).await.unwrap();
            fake.notify(uuids::INDOOR_BIKE_DATA, &BIKE_DATA);
            // Not subscribed: nothing.
            fake.notify(uuids::CYCLING_POWER_MEASUREMENT, &[0, 0, 10, 0]);
            fake.drop_link();
            let mut got = Vec::new();
            while let Some(n) = stream.next().await {
                got.push(n);
            }
            assert!(!link.is_connected().await);
            assert!(link.notifications().await.is_err());
            got
        });
        assert_eq!(got, vec![(uuids::INDOOR_BIKE_DATA, BIKE_DATA.to_vec())]);
    }

    /// The whole trainer flow over the Web Bluetooth adapter and the
    /// shared runner and protocol: connect → subscribe → power events →
    /// SIM writes → ERG writes → the device drops → reconnect (the ERG
    /// target again) → shutdown disconnects.
    #[test]
    fn trainer_flow_connect_sim_erg_drop_reconnect() {
        let fake = FakeDevice::ftms_trainer();
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (ev_tx, ev_rx) = mpsc::channel();
        let end = rt().block_on(async {
            let device = run_device(&fake, &cmd_rx, &ev_tx, &TokioTimer, AGAIN, "fake");
            let script = async {
                ms(150).await;
                assert_eq!(fake.connects.get(), 1);
                assert_eq!(
                    *fake.notifying.borrow(),
                    vec![
                        uuids::to_uuid128(uuids::INDOOR_BIKE_DATA),
                        uuids::to_uuid128(uuids::CYCLING_POWER_MEASUREMENT),
                        uuids::to_uuid128(uuids::FITNESS_MACHINE_CONTROL_POINT),
                    ]
                );
                fake.notify(uuids::INDOOR_BIKE_DATA, &BIKE_DATA);
                // SIM: 3 %, then 5 %.
                cmd_tx.send(sim(0.03)).unwrap();
                ms(250).await;
                cmd_tx.send(sim(0.05)).unwrap();
                ms(250).await;
                // ERG 200 W.
                cmd_tx.send(DeviceCommand::SetTargetPower(200.0)).unwrap();
                ms(250).await;
                let before = fake.control_writes().len();
                fake.drop_link();
                // Reconnects after RECONNECT_DELAY.
                ms(AGAIN.as_millis() as u64 + 600).await;
                assert_eq!(fake.connects.get(), 2);
                fake.notify(uuids::INDOOR_BIKE_DATA, &BIKE_DATA);
                ms(100).await;
                cmd_tx.send(DeviceCommand::Shutdown).unwrap();
                before
            };
            let (end, before) = tokio::join!(device, script);
            (end, before)
        });
        let (end, before) = end;
        assert_eq!(end, RunEnd::Shutdown);
        assert_eq!(fake.disconnects.get(), 1);
        let w = fake.control_writes();
        assert_eq!(w[0], vec![ftms::op::REQUEST_CONTROL]);
        assert_eq!(w[1], vec![ftms::op::START_OR_RESUME]);
        // SIM 3 % (0.01 % units: 300) and 5 %.
        assert_eq!(w[2][0], ftms::op::SET_INDOOR_BIKE_SIMULATION);
        assert_eq!(i16::from_le_bytes([w[2][3], w[2][4]]), 300);
        assert_eq!(i16::from_le_bytes([w[3][3], w[3][4]]), 500);
        assert_eq!(w[4], vec![ftms::op::SET_TARGET_POWER, 200, 0]);
        assert_eq!(before, 5);
        // After the reconnect: control, start, the ERG target again.
        assert_eq!(
            w[5..],
            [
                vec![ftms::op::REQUEST_CONTROL],
                vec![ftms::op::START_OR_RESUME],
                vec![ftms::op::SET_TARGET_POWER, 200, 0]
            ]
        );
        assert!(fake.writes.borrow().iter().all(|(_, _, resp)| *resp));
        let events: Vec<DeviceEvent> = ev_rx.try_iter().collect();
        let power = events
            .iter()
            .filter(|e| matches!(e, DeviceEvent::Power { watts, cadence_rpm: Some(c), .. } if *watts == 250.0 && *c == 90.0))
            .count();
        assert_eq!(power, 2, "{events:?}");
        let states: Vec<&DeviceEvent> = events
            .iter()
            .filter(|e| !matches!(e, DeviceEvent::Power { .. }))
            .collect();
        assert_eq!(
            states,
            [
                &DeviceEvent::Connected {
                    kind: "smart trainer (FTMS)"
                },
                &DeviceEvent::Disconnected,
                &DeviceEvent::Connected {
                    kind: "smart trainer (FTMS)"
                },
            ]
        );
    }

    /// A KICKR over the adapter: Wahoo's control (unlock, SIM mode, grade;
    /// ERG), power from Cycling Power; a failed reconnect is retried.
    #[test]
    fn kickr_wahoo_control_and_failed_reconnects() {
        let fake = FakeDevice::kickr();
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (ev_tx, ev_rx) = mpsc::channel();
        let end = rt().block_on(async {
            let device = run_device(&fake, &cmd_rx, &ev_tx, &TokioTimer, AGAIN, "kickr");
            let script = async {
                ms(150).await;
                cmd_tx.send(sim(0.04)).unwrap();
                fake.notify(uuids::CYCLING_POWER_MEASUREMENT, &[0, 0, 0xB4, 0]);
                ms(300).await;
                cmd_tx.send(DeviceCommand::SetTargetPower(150.0)).unwrap();
                ms(300).await;
                // Out of range: the first reconnect fails, the second works.
                fake.fail_connects.set(1);
                fake.drop_link();
                ms(2 * AGAIN.as_millis() as u64 + 600).await;
                cmd_tx.send(DeviceCommand::Shutdown).unwrap();
            };
            let (end, ()) = tokio::join!(device, script);
            end
        });
        assert_eq!(end, RunEnd::Shutdown);
        assert_eq!(fake.connects.get(), 3);
        let ops: Vec<u8> = fake.control_writes().iter().map(|w| w[0]).collect();
        // Unlock, SIM mode, grade; ERG; after the reconnect: unlock, ERG.
        assert_eq!(
            ops,
            [
                wahoo::op::UNLOCK,
                wahoo::op::SET_SIM_MODE,
                wahoo::op::SET_GRADE,
                wahoo::op::SET_ERG,
                wahoo::op::UNLOCK,
                wahoo::op::SET_ERG
            ]
        );
        let events: Vec<DeviceEvent> = ev_rx.try_iter().collect();
        assert!(
            events
                .iter()
                .any(|e| matches!(e, DeviceEvent::Power { watts, .. } if *watts == 180.0))
        );
        let disconnects = events
            .iter()
            .filter(|e| **e == DeviceEvent::Disconnected)
            .count();
        assert_eq!(disconnects, 2, "{events:?}");
    }

    /// A heart-rate strap: no control writes, heart rate events.
    #[test]
    fn heart_rate_strap() {
        let fake = FakeDevice::heart_rate_strap();
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (ev_tx, ev_rx) = mpsc::channel();
        rt().block_on(async {
            let device = run_device(&fake, &cmd_rx, &ev_tx, &TokioTimer, AGAIN, "hr");
            let script = async {
                ms(150).await;
                cmd_tx.send(sim(0.02)).unwrap();
                fake.notify(uuids::HEART_RATE_MEASUREMENT, &[0x00, 142]);
                ms(200).await;
                cmd_tx.send(DeviceCommand::Shutdown).unwrap();
            };
            tokio::join!(device, script);
        });
        assert!(fake.writes.borrow().is_empty());
        let events: Vec<DeviceEvent> = ev_rx.try_iter().collect();
        assert!(matches!(
            events[..],
            [
                DeviceEvent::Connected { kind: "heart-rate sensor" },
                DeviceEvent::HeartRate { bpm, .. }
            ] if bpm == 142.0
        ));
    }

    #[test]
    fn remembered_devices_round_trip() {
        let mut r = Remembered::default();
        r.remember(RememberedDevice {
            kind: DeviceKind::Trainer,
            id: "abc==".into(),
            name: "KICKR CORE 1234".into(),
        });
        r.remember(RememberedDevice {
            kind: DeviceKind::HeartRate,
            id: "x\ty".into(),
            name: "TICKR".into(),
        });
        // Again: moves to the front, no duplicate.
        r.remember(RememberedDevice {
            kind: DeviceKind::Trainer,
            id: "abc==".into(),
            name: "KICKR CORE 1234".into(),
        });
        let text = r.to_text();
        assert_eq!(text, "trainer\tabc==\tKICKR CORE 1234\nhr\tx y\tTICKR\n");
        let back = Remembered::parse(&format!("{text}garbage\nhr\t\tno id\n"));
        assert_eq!(back.devices.len(), 2);
        assert_eq!(back.kind_of("abc=="), Some(DeviceKind::Trainer));
        assert_eq!(back.kind_of("x y"), Some(DeviceKind::HeartRate));
        let mut back = back;
        assert_eq!(back.forget("tickr"), 1);
        assert_eq!(back.forget("nothing"), 0);
        assert_eq!(back.forget("all"), 1);
        assert!(back.devices.is_empty());
        let many = Remembered {
            devices: (0..20)
                .map(|i| RememberedDevice {
                    kind: DeviceKind::PowerMeter,
                    id: format!("id{i}"),
                    name: String::new(),
                })
                .collect(),
        };
        assert_eq!(
            Remembered::parse(&many.to_text()).devices.len(),
            Remembered::MAX
        );
    }
}
