//! The page's device flow (spec §14 Phase 10 B4; wasm32 only, kept thin):
//! the overlay's buttons call [`page_connect`] in their click handler, so
//! the browser's chooser opens within the user gesture Web Bluetooth
//! needs; the chosen device becomes a `WebBluetoothBackend` (the shared
//! `ocw-devices` protocol on the page's event loop), which the Bevy glue
//! (`crate::plugin`) hands to the session. The devices of this origin are
//! remembered in `localStorage`; with Chromium's permission persistence
//! (`navigator.bluetooth.getDevices()`) they reconnect after a reload
//! without the chooser.

use std::sync::Mutex;

use bevy::prelude::*;
use ocw_devices::ble::web::{self, DeviceKind, Remembered, WebBluetoothBackend};
use wasm_bindgen::JsValue;
use wasm_bindgen::prelude::wasm_bindgen;

use crate::devices::Bluetooth;

/// Devices the user picked (or the browser still permits), until the
/// session takes them.
static PENDING: Mutex<Vec<(WebBluetoothBackend, DeviceKind)>> = Mutex::new(Vec::new());
/// Lines for the console from the page side (a cancelled chooser …).
static MESSAGES: Mutex<Vec<String>> = Mutex::new(Vec::new());
static BLUETOOTH: Mutex<Bluetooth> = Mutex::new(Bluetooth::Unknown);
/// The devices as JSON for the overlay (`window.ocw.devices()`).
static JSON: Mutex<String> = Mutex::new(String::new());

fn say(line: String) {
    info!("{line}");
    MESSAGES.lock().expect("not poisoned").push(line);
}

/// The devices the page has to hand over, and its console lines.
pub fn take_pending() -> (Vec<(WebBluetoothBackend, DeviceKind)>, Vec<String>) {
    (
        std::mem::take(&mut *PENDING.lock().expect("not poisoned")),
        std::mem::take(&mut *MESSAGES.lock().expect("not poisoned")),
    )
}

pub fn bluetooth() -> Bluetooth {
    *BLUETOOTH.lock().expect("not poisoned")
}

pub fn publish(json: String) {
    *JSON.lock().expect("not poisoned") = json;
}

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

/// The remembered devices of this origin (`localStorage`).
pub fn load_remembered() -> Remembered {
    storage()
        .and_then(|s| s.get_item(Remembered::STORAGE_KEY).ok().flatten())
        .map(|t| Remembered::parse(&t))
        .unwrap_or_default()
}

pub fn save_remembered(r: &Remembered) {
    if let Some(s) = storage() {
        let _ = s.set_item(Remembered::STORAGE_KEY, &r.to_text());
    }
}

/// At the start: is there Web Bluetooth and an adapter; and the devices
/// the browser still permits that this origin used – they connect without
/// the chooser.
pub fn start() {
    if !web::has_web_bluetooth() {
        *BLUETOOTH.lock().expect("not poisoned") = Bluetooth::Missing;
        warn!("{}", crate::devices::NO_WEB_BLUETOOTH);
        return;
    }
    wasm_bindgen_futures::spawn_local(async {
        let state = match web::availability().await {
            Some(false) => Bluetooth::NoAdapter,
            _ => Bluetooth::Available,
        };
        *BLUETOOTH.lock().expect("not poisoned") = state;
        info!(?state, "Web Bluetooth");
        let remembered = load_remembered();
        for device in web::permitted_devices().await {
            let Some(kind) = remembered.kind_of(&device.id()) else {
                continue;
            };
            let backend = WebBluetoothBackend::new(device);
            say(format!(
                "{}: permitted on this site, reconnecting as {}",
                ocw_devices::DeviceBackend::name(&backend),
                kind.label()
            ));
            PENDING.lock().expect("not poisoned").push((backend, kind));
        }
    });
}

/// Opens the chooser for `kind` (`trainer`, `power`, `hr`). Call it in a
/// user gesture: a click, or a key (the console's Enter) a moment before.
pub fn connect(kind: DeviceKind) -> Result<String, String> {
    if bluetooth() == Bluetooth::Missing {
        return Err(crate::devices::NO_WEB_BLUETOOTH.into());
    }
    let chosen = web::request(kind)?;
    wasm_bindgen_futures::spawn_local(async move {
        match chosen.await {
            Ok(device) => {
                let backend = WebBluetoothBackend::new(device);
                PENDING.lock().expect("not poisoned").push((backend, kind));
            }
            Err(e) => say(format!("no {} connected: {e}", kind.label())),
        }
    });
    Ok(format!(
        "choose your {} in the browser's list …",
        kind.label()
    ))
}

/// `window.ocw.connectDevice(kind)`: the overlay's buttons (in their click
/// handler).
#[wasm_bindgen(js_name = connectDevice)]
pub fn page_connect(kind: &str) -> Result<String, JsValue> {
    let kind = DeviceKind::parse(kind)
        .ok_or_else(|| JsValue::from_str(&format!("unknown device kind {kind}")))?;
    connect(kind).map_err(|e| JsValue::from_str(&e))
}

/// `window.ocw.devices()`: the devices, the trainer and Web Bluetooth's
/// state as JSON (the overlay and the browser test read it).
#[wasm_bindgen(js_name = devices)]
pub fn page_devices() -> String {
    JSON.lock().expect("not poisoned").clone()
}
