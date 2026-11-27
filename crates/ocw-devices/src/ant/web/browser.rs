//! The browser side of WebUSB for ANT sticks (wasm32 only, kept thin): own
//! `wasm-bindgen` bindings for `navigator.usb` and `USBDevice` (web-sys
//! has them only behind `web_sys_unstable_apis`, as Web Bluetooth); every
//! call is structural, so a test double of `navigator.usb`
//! (`scripts/web-usb-fake.js`) works as well as Chromium's.
//!
//! [`JsUsbStick`] is the [`UsbStick`] over one `USBDevice`, [`JsUsbHost`]
//! the [`UsbHost`] over `navigator.usb` (`getDevices()`, the `connect` /
//! `disconnect` events); [`start`] runs the page's
//! [`WebStickService`](super::WebStickService) as a task on its event loop;
//! [`request`] opens the browser's chooser – it must be called from a user
//! gesture (a click, or a key a moment before), synchronously.

use std::future::Future;
use std::sync::Once;
use std::sync::atomic::{AtomicBool, Ordering};

use futures::FutureExt;
use futures::future::LocalBoxFuture;
use js_sys::{Array, DataView, Function, Promise, Reflect, Uint8Array};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

use super::{
    InResult, TransferStatus, UsbEndpoint, UsbHost, UsbStick, WebStickService, is_stick,
    request_options,
};
use crate::DeviceError;
use crate::ant::stick_name;
use crate::ble::web::JsTimer;

#[wasm_bindgen]
extern "C" {
    type Usb;
    #[wasm_bindgen(method, js_name = requestDevice, catch)]
    fn request_device(this: &Usb, options: &JsValue) -> Result<Promise, JsValue>;
    #[wasm_bindgen(method, js_name = getDevices, catch)]
    fn get_devices(this: &Usb) -> Result<Promise, JsValue>;
    #[wasm_bindgen(method, js_name = addEventListener)]
    fn add_listener(this: &Usb, kind: &str, f: &Function);

    /// A `USBDevice`.
    #[derive(Clone)]
    pub type UsbDevice;
    #[wasm_bindgen(method, getter, js_name = vendorId)]
    fn vendor_id(this: &UsbDevice) -> u16;
    #[wasm_bindgen(method, getter, js_name = productId)]
    fn product_id(this: &UsbDevice) -> u16;
    #[wasm_bindgen(method, getter)]
    fn configuration(this: &UsbDevice) -> JsValue;
    #[wasm_bindgen(method, catch)]
    fn open(this: &UsbDevice) -> Result<Promise, JsValue>;
    #[wasm_bindgen(method, catch)]
    fn close(this: &UsbDevice) -> Result<Promise, JsValue>;
    #[wasm_bindgen(method, js_name = selectConfiguration, catch)]
    fn select_configuration(this: &UsbDevice, value: u8) -> Result<Promise, JsValue>;
    #[wasm_bindgen(method, js_name = claimInterface, catch)]
    fn claim_interface(this: &UsbDevice, number: u8) -> Result<Promise, JsValue>;
    #[wasm_bindgen(method, js_name = releaseInterface, catch)]
    fn release_interface(this: &UsbDevice, number: u8) -> Result<Promise, JsValue>;
    #[wasm_bindgen(method, js_name = transferIn, catch)]
    fn transfer_in(this: &UsbDevice, endpoint: u8, length: u32) -> Result<Promise, JsValue>;
    #[wasm_bindgen(method, js_name = transferOut, catch)]
    fn transfer_out(this: &UsbDevice, endpoint: u8, data: &Uint8Array) -> Result<Promise, JsValue>;
    #[wasm_bindgen(method, js_name = clearHalt, catch)]
    fn clear_halt(this: &UsbDevice, direction: &str, endpoint: u8) -> Result<Promise, JsValue>;
    #[wasm_bindgen(method, catch)]
    fn reset(this: &UsbDevice) -> Result<Promise, JsValue>;
}

fn text_of(e: &JsValue, key: &str) -> Option<String> {
    Reflect::get(e, &key.into())
        .ok()
        .and_then(|v| v.as_string())
        .filter(|s| !s.is_empty())
}

/// A WebUSB error as a [`DeviceError`]: `SecurityError` (Linux without
/// the udev rule; a blocked device) → permission, the interface claimed
/// by another program (`NetworkError: Unable to claim interface.`) →
/// busy, `NotFoundError` (the device was disconnected) → no stick.
fn usb_error(e: JsValue) -> DeviceError {
    let name = text_of(&e, "name").unwrap_or_default();
    let message = text_of(&e, "message")
        .or_else(|| e.as_string())
        .unwrap_or_default();
    match name.as_str() {
        "SecurityError" => DeviceError::StickPermission,
        "NotFoundError" => DeviceError::NoStick,
        "NetworkError" if message.to_ascii_lowercase().contains("claim") => DeviceError::StickBusy,
        _ if name.is_empty() => DeviceError::Ant(message),
        _ => DeviceError::Ant(format!("{name}: {message}")),
    }
}

async fn wait(p: Result<Promise, JsValue>) -> Result<JsValue, DeviceError> {
    JsFuture::from(p.map_err(usb_error)?)
        .await
        .map_err(usb_error)
}

fn usb() -> Option<Usb> {
    let nav = Reflect::get(&js_sys::global(), &"navigator".into()).ok()?;
    let usb = Reflect::get(&nav, &"usb".into()).ok()?;
    (!usb.is_undefined() && !usb.is_null()).then(|| usb.unchecked_into())
}

/// Whether this browser has WebUSB (`navigator.usb`: Chromium browsers on
/// the desktop and on Android; Firefox, Safari and every browser on iOS
/// have none).
pub fn has_web_usb() -> bool {
    usb().is_some()
}

/// Set by the `connect` / `disconnect` events and a granted stick.
static CHANGED: AtomicBool = AtomicBool::new(false);

/// Opens the browser's chooser for an ANT stick. Must run synchronously in
/// a user gesture; resolves to the stick's name once the user picked one
/// (an error when the chooser was cancelled).
pub fn request() -> Result<impl Future<Output = Result<String, String>>, String> {
    let usb = usb().ok_or("this browser has no WebUSB")?;
    let options = js_sys::JSON::parse(&request_options()).map_err(err_text)?;
    let p = usb.request_device(&options).map_err(err_text)?;
    Ok(async move {
        let device: UsbDevice = JsFuture::from(p)
            .await
            .map(JsCast::unchecked_into)
            .map_err(err_text)?;
        CHANGED.store(true, Ordering::Relaxed);
        Ok(stick_name(device.product_id()).to_owned())
    })
}

fn err_text(e: JsValue) -> String {
    let name = text_of(&e, "name");
    let message = text_of(&e, "message").or_else(|| e.as_string());
    match (name, message) {
        (Some(n), Some(m)) => format!("{n}: {m}"),
        (None, Some(m)) => m,
        _ => format!("{e:?}"),
    }
}

/// The [`UsbStick`] over one `USBDevice`.
pub struct JsUsbStick {
    device: UsbDevice,
}

fn get(o: &JsValue, key: &str) -> JsValue {
    Reflect::get(o, &key.into()).unwrap_or(JsValue::UNDEFINED)
}

/// Interface 0's endpoints (its current alternate) in the selected
/// configuration.
fn endpoints_of(configuration: &JsValue) -> Vec<UsbEndpoint> {
    let interfaces = get(configuration, "interfaces");
    if !Array::is_array(&interfaces) {
        return Vec::new();
    }
    let Some(interface) = Array::from(&interfaces)
        .iter()
        .find(|i| get(i, "interfaceNumber").as_f64() == Some(0.0))
    else {
        return Vec::new();
    };
    let endpoints = get(&get(&interface, "alternate"), "endpoints");
    if !Array::is_array(&endpoints) {
        return Vec::new();
    }
    Array::from(&endpoints)
        .iter()
        .filter_map(|e| {
            Some(UsbEndpoint {
                number: get(&e, "endpointNumber").as_f64()? as u8,
                is_in: get(&e, "direction").as_string()? == "in",
                bulk: get(&e, "type").as_string()? == "bulk",
                packet_size: get(&e, "packetSize").as_f64().unwrap_or(64.0) as u16,
            })
        })
        .collect()
}

fn bytes_of(v: &JsValue) -> Vec<u8> {
    match v.dyn_ref::<DataView>() {
        Some(dv) => Uint8Array::new_with_byte_offset_and_length(
            &dv.buffer(),
            dv.byte_offset() as u32,
            dv.byte_length() as u32,
        )
        .to_vec(),
        None if v.is_undefined() || v.is_null() => Vec::new(),
        None => Uint8Array::new(v).to_vec(),
    }
}

fn status_of(result: &JsValue) -> TransferStatus {
    TransferStatus::parse(&get(result, "status").as_string().unwrap_or_default())
}

impl UsbStick for JsUsbStick {
    fn name(&self) -> String {
        stick_name(self.device.product_id()).to_owned()
    }

    fn open(&self) -> LocalBoxFuture<'static, Result<Vec<UsbEndpoint>, DeviceError>> {
        let d = self.device.clone();
        async move {
            wait(d.open()).await?;
            let config = d.configuration();
            if config.is_null() || config.is_undefined() {
                wait(d.select_configuration(1)).await?;
            }
            wait(d.claim_interface(0)).await?;
            Ok(endpoints_of(&d.configuration()))
        }
        .boxed_local()
    }

    fn transfer_in(&self, endpoint: u8, length: u32) -> LocalBoxFuture<'static, InResult> {
        let d = self.device.clone();
        async move {
            let r = wait(d.transfer_in(endpoint, length)).await?;
            Ok((status_of(&r), bytes_of(&get(&r, "data"))))
        }
        .boxed_local()
    }

    fn transfer_out(
        &self,
        endpoint: u8,
        data: Vec<u8>,
    ) -> LocalBoxFuture<'static, Result<TransferStatus, DeviceError>> {
        let d = self.device.clone();
        async move {
            let bytes = Uint8Array::from(data.as_slice());
            let r = wait(d.transfer_out(endpoint, &bytes)).await?;
            Ok(status_of(&r))
        }
        .boxed_local()
    }

    fn clear_halt(
        &self,
        is_in: bool,
        endpoint: u8,
    ) -> LocalBoxFuture<'static, Result<(), DeviceError>> {
        let d = self.device.clone();
        async move {
            wait(d.clear_halt(if is_in { "in" } else { "out" }, endpoint))
                .await
                .map(|_| ())
        }
        .boxed_local()
    }

    fn reset(&self) -> LocalBoxFuture<'static, Result<(), DeviceError>> {
        let d = self.device.clone();
        async move { wait(d.reset()).await.map(|_| ()) }.boxed_local()
    }

    fn close(&self) -> LocalBoxFuture<'static, ()> {
        let d = self.device.clone();
        async move {
            let _ = wait(d.release_interface(0)).await;
            let _ = wait(d.close()).await;
        }
        .boxed_local()
    }
}

/// The [`UsbHost`] over `navigator.usb`.
pub struct JsUsbHost;

impl UsbHost for JsUsbHost {
    type Stick = JsUsbStick;

    fn sticks(&self) -> LocalBoxFuture<'static, Vec<JsUsbStick>> {
        async move {
            let Some(p) = usb().and_then(|u| u.get_devices().ok()) else {
                return Vec::new();
            };
            match JsFuture::from(p).await {
                Ok(list) => Array::from(&list)
                    .iter()
                    .map(JsCast::unchecked_into::<UsbDevice>)
                    .filter(|d| is_stick(d.vendor_id(), d.product_id()))
                    .map(|device| JsUsbStick { device })
                    .collect(),
                Err(_) => Vec::new(),
            }
        }
        .boxed_local()
    }

    fn take_changed(&self) -> bool {
        CHANGED.swap(false, Ordering::Relaxed)
    }
}

/// Runs the page's stick service on its event loop (once), with
/// `navigator.usb`'s `connect` / `disconnect` events telling it to look.
pub fn start(service: WebStickService) {
    static LISTEN: Once = Once::new();
    LISTEN.call_once(|| {
        if let Some(usb) = usb() {
            let changed: Closure<dyn FnMut(JsValue)> = Closure::new(|_e: JsValue| {
                CHANGED.store(true, Ordering::Relaxed);
            });
            usb.add_listener("connect", changed.as_ref().unchecked_ref());
            usb.add_listener("disconnect", changed.as_ref().unchecked_ref());
            // For the page's lifetime.
            changed.forget();
        }
    });
    let started = web_time::Instant::now();
    wasm_bindgen_futures::spawn_local(async move {
        service
            .run(&JsUsbHost, &JsTimer, move || started.elapsed())
            .await;
    });
}
