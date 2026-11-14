//! The browser side of Web Bluetooth (wasm32 only, kept thin): own
//! `wasm-bindgen` bindings for `navigator.bluetooth`, `BluetoothDevice`,
//! its GATT server, services and characteristics (web-sys has them only
//! behind `web_sys_unstable_apis`); every call is structural, so a test
//! double of `navigator.bluetooth` works as well as Chromium's.
//!
//! [`JsGatt`] is the [`WebGatt`] over one `BluetoothDevice`;
//! [`WebBluetoothBackend`] the [`DeviceBackend`] that runs
//! [`run_device`](super::run_device) for it as a task on the page's event
//! loop. [`request`] opens the browser's chooser – it must be called from
//! a user gesture (a click), synchronously.

use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use futures::channel::mpsc::UnboundedSender;
use js_sys::{Array, DataView, Function, Promise, Reflect, Uint8Array};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

use super::super::RECONNECT_DELAY;
use super::super::runner::Timer;
use super::{CharProps, DeviceKind, GattEvent, WebCharacteristic, WebGatt, run_device};
use crate::{DeviceBackend, DeviceError, DeviceHandle};

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = setTimeout)]
    fn set_timeout(handler: &Function, ms: i32) -> JsValue;

    type Bluetooth;
    #[wasm_bindgen(method, js_name = requestDevice, catch)]
    fn request_device(this: &Bluetooth, options: &JsValue) -> Result<Promise, JsValue>;
    #[wasm_bindgen(method, js_name = getDevices, catch)]
    fn get_devices(this: &Bluetooth) -> Result<Promise, JsValue>;
    #[wasm_bindgen(method, js_name = getAvailability, catch)]
    fn get_availability(this: &Bluetooth) -> Result<Promise, JsValue>;

    /// A `BluetoothDevice`.
    #[derive(Clone)]
    pub type BluetoothDevice;
    #[wasm_bindgen(method, getter)]
    pub fn id(this: &BluetoothDevice) -> String;
    #[wasm_bindgen(method, getter)]
    pub fn name(this: &BluetoothDevice) -> Option<String>;
    #[wasm_bindgen(method, getter)]
    fn gatt(this: &BluetoothDevice) -> Option<GattServer>;
    #[wasm_bindgen(method, js_name = addEventListener)]
    fn add_device_listener(this: &BluetoothDevice, kind: &str, f: &Function);
    #[wasm_bindgen(method, js_name = removeEventListener)]
    fn remove_device_listener(this: &BluetoothDevice, kind: &str, f: &Function);

    type GattServer;
    #[wasm_bindgen(method, getter)]
    fn connected(this: &GattServer) -> bool;
    #[wasm_bindgen(method, catch)]
    fn connect(this: &GattServer) -> Result<Promise, JsValue>;
    #[wasm_bindgen(method, catch)]
    fn disconnect(this: &GattServer) -> Result<(), JsValue>;
    #[wasm_bindgen(method, js_name = getPrimaryServices, catch)]
    fn get_primary_services(this: &GattServer) -> Result<Promise, JsValue>;

    type GattService;
    #[wasm_bindgen(method, getter)]
    fn uuid(this: &GattService) -> String;
    #[wasm_bindgen(method, js_name = getCharacteristics, catch)]
    fn get_characteristics(this: &GattService) -> Result<Promise, JsValue>;

    #[derive(Clone)]
    type GattCharacteristic;
    #[wasm_bindgen(method, getter, js_name = uuid)]
    fn char_uuid(this: &GattCharacteristic) -> String;
    #[wasm_bindgen(method, getter)]
    fn properties(this: &GattCharacteristic) -> JsValue;
    #[wasm_bindgen(method, js_name = readValue, catch)]
    fn read_value(this: &GattCharacteristic) -> Result<Promise, JsValue>;
    #[wasm_bindgen(method, js_name = writeValueWithResponse, catch)]
    fn write_with_response(
        this: &GattCharacteristic,
        value: &Uint8Array,
    ) -> Result<Promise, JsValue>;
    #[wasm_bindgen(method, js_name = writeValueWithoutResponse, catch)]
    fn write_without_response(
        this: &GattCharacteristic,
        value: &Uint8Array,
    ) -> Result<Promise, JsValue>;
    #[wasm_bindgen(method, js_name = startNotifications, catch)]
    fn start_notifications(this: &GattCharacteristic) -> Result<Promise, JsValue>;
    #[wasm_bindgen(method, js_name = addEventListener)]
    fn add_listener(this: &GattCharacteristic, kind: &str, f: &Function);
    #[wasm_bindgen(method, js_name = removeEventListener)]
    fn remove_listener(this: &GattCharacteristic, kind: &str, f: &Function);
}

/// A JS error as text: `name: message` (DOMException, Error), else as is.
fn js(e: JsValue) -> DeviceError {
    let text = |k: &str| {
        Reflect::get(&e, &k.into())
            .ok()
            .and_then(|v| v.as_string())
            .filter(|s| !s.is_empty())
    };
    DeviceError::Bluetooth(match (text("name"), text("message")) {
        (Some(n), Some(m)) => format!("{n}: {m}"),
        (None, Some(m)) => m,
        _ => e.as_string().unwrap_or_else(|| format!("{e:?}")),
    })
}

async fn wait(p: Result<Promise, JsValue>) -> Result<JsValue, DeviceError> {
    JsFuture::from(p.map_err(js)?).await.map_err(js)
}

fn bluetooth() -> Option<Bluetooth> {
    let nav = Reflect::get(&js_sys::global(), &"navigator".into()).ok()?;
    let bt = Reflect::get(&nav, &"bluetooth".into()).ok()?;
    (!bt.is_undefined() && !bt.is_null()).then(|| bt.unchecked_into())
}

/// Whether this browser has Web Bluetooth (`navigator.bluetooth`;
/// Chromium – Firefox and Safari have none, spec §14 Phase 10 Q1).
pub fn has_web_bluetooth() -> bool {
    bluetooth().is_some()
}

/// `navigator.bluetooth.getAvailability()`: whether there is an adapter
/// (`None`: no Web Bluetooth, or the browser does not say).
pub async fn availability() -> Option<bool> {
    let p = bluetooth()?.get_availability().ok()?;
    JsFuture::from(p).await.ok()?.as_bool()
}

/// Opens the browser's device chooser for `kind`. Must run synchronously
/// in a user gesture (a click handler); the device comes when the user
/// picked one (an error when the chooser was cancelled).
pub fn request(
    kind: DeviceKind,
) -> Result<impl Future<Output = Result<BluetoothDevice, String>>, String> {
    let bt = bluetooth().ok_or("this browser has no Web Bluetooth")?;
    let options = js_sys::JSON::parse(&kind.request_options()).map_err(err_text)?;
    let p = bt.request_device(&options).map_err(err_text)?;
    Ok(async move {
        JsFuture::from(p)
            .await
            .map(JsCast::unchecked_into)
            .map_err(err_text)
    })
}

fn err_text(e: JsValue) -> String {
    match js(e) {
        DeviceError::Bluetooth(s) | DeviceError::Start(s) => s,
    }
}

/// The devices this origin may use without the chooser
/// (`navigator.bluetooth.getDevices()`: Chromium with the persistent
/// permissions backend); empty where the browser has no such list.
pub async fn permitted_devices() -> Vec<BluetoothDevice> {
    let Some(p) = bluetooth().and_then(|b| b.get_devices().ok()) else {
        return Vec::new();
    };
    match JsFuture::from(p).await {
        Ok(list) => Array::from(&list)
            .iter()
            .map(JsCast::unchecked_into)
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// [`Timer`] on `setTimeout`.
pub struct JsTimer;

impl Timer for JsTimer {
    fn sleep(&self, duration: Duration) -> impl Future<Output = ()> {
        let ms = duration.as_millis().min(i32::MAX as u128) as i32;
        let p = Promise::new(&mut |resolve, _| {
            let _ = set_timeout(&resolve, ms);
        });
        async move {
            let _ = JsFuture::from(p).await;
        }
    }
}

type Sink = Rc<RefCell<Option<UnboundedSender<GattEvent>>>>;
type Listener = Closure<dyn FnMut(JsValue)>;

/// The [`WebGatt`] over one `BluetoothDevice`.
pub struct JsGatt {
    device: BluetoothDevice,
    chars: RefCell<Vec<GattCharacteristic>>,
    /// The value listeners of the current characteristics (removed when
    /// they are discovered again).
    listeners: RefCell<Vec<(GattCharacteristic, Listener)>>,
    sink: Sink,
    on_disconnect: Listener,
}

impl JsGatt {
    pub fn new(device: BluetoothDevice) -> Self {
        let sink: Sink = Rc::default();
        let s = sink.clone();
        let on_disconnect: Listener = Closure::new(move |_e: JsValue| {
            if let Some(tx) = s.borrow_mut().take() {
                let _ = tx.unbounded_send(GattEvent::Disconnected);
            }
        });
        device.add_device_listener(
            "gattserverdisconnected",
            on_disconnect.as_ref().unchecked_ref(),
        );
        Self {
            device,
            chars: RefCell::default(),
            listeners: RefCell::default(),
            sink,
            on_disconnect,
        }
    }

    fn server(&self) -> Result<GattServer, DeviceError> {
        self.device
            .gatt()
            .ok_or_else(|| DeviceError::Bluetooth("the device has no GATT server".into()))
    }

    fn char(&self, index: usize) -> Result<GattCharacteristic, DeviceError> {
        self.chars
            .borrow()
            .get(index)
            .cloned()
            .ok_or_else(|| DeviceError::Bluetooth(format!("no characteristic #{index}")))
    }

    fn clear_listeners(&self) {
        for (c, l) in self.listeners.borrow_mut().drain(..) {
            c.remove_listener("characteristicvaluechanged", l.as_ref().unchecked_ref());
        }
    }
}

impl Drop for JsGatt {
    fn drop(&mut self) {
        self.clear_listeners();
        self.device.remove_device_listener(
            "gattserverdisconnected",
            self.on_disconnect.as_ref().unchecked_ref(),
        );
    }
}

fn flag(props: &JsValue, key: &str) -> bool {
    Reflect::get(props, &key.into())
        .ok()
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

fn bytes_of(v: &JsValue) -> Vec<u8> {
    match v.dyn_ref::<DataView>() {
        Some(dv) => Uint8Array::new_with_byte_offset_and_length(
            &dv.buffer(),
            dv.byte_offset() as u32,
            dv.byte_length() as u32,
        )
        .to_vec(),
        None => Uint8Array::new(v).to_vec(),
    }
}

impl WebGatt for JsGatt {
    async fn connect(&self) -> Result<(), DeviceError> {
        wait(self.server()?.connect()).await.map(|_| ())
    }

    fn connected(&self) -> bool {
        self.device.gatt().is_some_and(|g| g.connected())
    }

    async fn discover(&self) -> Result<Vec<WebCharacteristic>, DeviceError> {
        let services = Array::from(&wait(self.server()?.get_primary_services()).await?);
        let mut found = Vec::new();
        let mut objects = Vec::new();
        for s in services.iter() {
            let s: GattService = s.unchecked_into();
            // A service without characteristics rejects (NotFoundError).
            let Ok(chars) = wait(s.get_characteristics()).await else {
                continue;
            };
            for c in Array::from(&chars).iter() {
                let c: GattCharacteristic = c.unchecked_into();
                let p = c.properties();
                found.push(WebCharacteristic {
                    service: s.uuid(),
                    uuid: c.char_uuid(),
                    props: CharProps {
                        read: flag(&p, "read"),
                        write: flag(&p, "write"),
                        write_without_response: flag(&p, "writeWithoutResponse"),
                        notify: flag(&p, "notify"),
                        indicate: flag(&p, "indicate"),
                    },
                });
                objects.push(c);
            }
        }
        self.clear_listeners();
        *self.chars.borrow_mut() = objects;
        Ok(found)
    }

    async fn read(&self, index: usize) -> Result<Vec<u8>, DeviceError> {
        let v = wait(self.char(index)?.read_value()).await?;
        Ok(bytes_of(&v))
    }

    async fn write(
        &self,
        index: usize,
        value: &[u8],
        with_response: bool,
    ) -> Result<(), DeviceError> {
        let c = self.char(index)?;
        let data = Uint8Array::from(value);
        let p = if with_response {
            c.write_with_response(&data)
        } else {
            c.write_without_response(&data)
        };
        wait(p).await.map(|_| ())
    }

    async fn start_notifications(&self, index: usize) -> Result<(), DeviceError> {
        let c = self.char(index)?;
        let sink = self.sink.clone();
        let listener: Listener = Closure::new(move |e: JsValue| {
            let value = Reflect::get(&e, &"target".into())
                .and_then(|t| Reflect::get(&t, &"value".into()))
                .map(|v| bytes_of(&v))
                .unwrap_or_default();
            if let Some(tx) = sink.borrow().as_ref() {
                let _ = tx.unbounded_send(GattEvent::Value { index, value });
            }
        });
        // The listener first: the first value may come with the answer.
        c.add_listener(
            "characteristicvaluechanged",
            listener.as_ref().unchecked_ref(),
        );
        self.listeners.borrow_mut().push((c.clone(), listener));
        wait(c.start_notifications()).await.map(|_| ())
    }

    fn set_events(&self, sink: UnboundedSender<GattEvent>) {
        *self.sink.borrow_mut() = Some(sink);
    }

    fn disconnect(&self) {
        self.sink.borrow_mut().take();
        if let Ok(s) = self.server() {
            let _ = s.disconnect();
        }
    }
}

thread_local! {
    /// Devices handed to backends, until they start (JS objects stay on
    /// this thread; the backend itself must be `Send`).
    static WAITING: RefCell<HashMap<u32, BluetoothDevice>> = RefCell::new(HashMap::new());
}
static NEXT: AtomicU32 = AtomicU32::new(1);

/// A Web Bluetooth trainer or sensor: the shared protocol over [`JsGatt`],
/// reconnecting by itself after a drop (`gatt.connect()` again, no
/// chooser), as a task on the page's event loop.
pub struct WebBluetoothBackend {
    name: String,
    id: String,
    slot: u32,
}

impl WebBluetoothBackend {
    pub fn new(device: BluetoothDevice) -> Self {
        let slot = NEXT.fetch_add(1, Ordering::Relaxed);
        let name = device
            .name()
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| "Bluetooth device".into());
        let id = device.id();
        WAITING.with(|w| w.borrow_mut().insert(slot, device));
        Self { name, id, slot }
    }

    /// The browser's id of the device (per origin; for the remembered
    /// devices).
    pub fn device_id(&self) -> &str {
        &self.id
    }
}

impl DeviceBackend for WebBluetoothBackend {
    fn name(&self) -> &str {
        &self.name
    }

    fn start(self) -> Result<DeviceHandle, DeviceError> {
        let device = WAITING
            .with(|w| w.borrow_mut().remove(&self.slot))
            .ok_or_else(|| DeviceError::Start("device already started".into()))?;
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (ev_tx, ev_rx) = mpsc::channel();
        let name = self.name.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let gatt = JsGatt::new(device);
            run_device(&gatt, &cmd_rx, &ev_tx, &JsTimer, RECONNECT_DELAY, &name).await;
        });
        Ok(DeviceHandle {
            name: self.name,
            events: ev_rx,
            commands: cmd_tx,
        })
    }
}
