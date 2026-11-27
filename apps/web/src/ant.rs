//! ANT+ in the browser over WebUSB (spec 14 Phase 17 A5, 17E *The
//! browser*; Q3 (a) + (b)): the page's ANT stick – `ocw_devices::ant::web`,
//! the same pure stick core as the desktop's, on the page's event loop –
//! the network key the rider types (kept in this page's storage only,
//! never sent anywhere, never in a log), *Add an ANT+ stick* (the
//! browser's chooser, from the key press or click that ran it), *Find
//! ANT+ sensors* and the sensors found, which connect into the session's
//! devices like the Bluetooth ones (heart rate; a power meter gives the
//! ride its power).
//!
//! Where the browser has no WebUSB (Firefox, Safari, every browser on iOS)
//! the Devices page says so; on Windows it says the stick needs the WinUSB
//! driver (Zadig; Q7) – Garmin's own driver, which the desktop app uses,
//! is not one WebUSB can open.
//!
//! The page's test hooks (`window.ocw.antSim*`): a simulated stick (the
//! desktop's `--ant-sim`, `ocw_devices::ant::sim`) behind the fake
//! `navigator.usb` of `scripts/web-usb-fake.js`, so the browser test rides
//! with a simulated strap and power meter without hardware.

use std::sync::Mutex;

use bevy::prelude::*;
use clap::{Arg, ArgAction, Command};
use ocw_client::menu::devices::{ANT_ADD_COMMAND, AntFix, AntInfo, SCAN_SECONDS, WebUsbInfo};
use ocw_devices::ant::web::WebAntStick;
use ocw_devices::ant::{AntDeviceId, AntProfile, NetworkKey, StickStatus};
use ocw_devices::ble::web::DeviceKind;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, ConsoleState, Permission};
use ocw_ui::join::FieldEnd;
use ocw_ui::key_entry::KeyEntry;

use crate::devices::WebDevices;

/// The page's storage key of the ANT+ network key (16 hex digits; this
/// origin's `localStorage` only – never the settings, which an account may
/// sync, never a request).
pub const KEY_STORAGE: &str = "ocw.ant.key";
/// Where the key is kept (the ANT+ page's *Kept in*).
pub const KEPT_IN: &str = "this browser only (never sent)";

/// What the browser offers (set once at the start).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Env {
    /// `navigator.usb` is there.
    pub supported: bool,
    /// The user agent says Windows (the WinUSB note).
    pub windows: bool,
    /// The user agent says Linux, not Android (the udev rule).
    pub linux: bool,
}

impl Env {
    /// From the user agent and whether `navigator.usb` exists.
    pub fn of(user_agent: &str, supported: bool) -> Self {
        Self {
            supported,
            windows: user_agent.contains("Windows"),
            linux: user_agent.contains("Linux") && !user_agent.contains("Android"),
        }
    }
}

static ENV: Mutex<Env> = Mutex::new(Env {
    supported: false,
    windows: false,
    linux: false,
});
static STICK: Mutex<Option<WebAntStick>> = Mutex::new(None);
/// Lines from the page side (the chooser's answer).
static MESSAGES: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub fn env() -> Env {
    *ENV.lock().expect("not poisoned")
}

/// The page's stick service (`None`: no WebUSB, or not started yet).
pub fn stick() -> Option<WebAntStick> {
    STICK.lock().expect("not poisoned").clone()
}

#[cfg(target_arch = "wasm32")]
fn say(line: String) {
    info!("{line}");
    MESSAGES.lock().expect("not poisoned").push(line);
}

/// The page side's lines since the last call.
pub fn take_messages() -> Vec<String> {
    std::mem::take(&mut *MESSAGES.lock().expect("not poisoned"))
}

/// The session's device kind of an ANT+ sensor id.
pub fn kind_of(id: &str) -> Option<DeviceKind> {
    AntDeviceId::parse(id).map(|d| match d.profile {
        AntProfile::HeartRate => DeviceKind::HeartRate,
        AntProfile::BikePower => DeviceKind::PowerMeter,
        // An FE-C trainer delivers power like a trainer; its SIM / ERG control
        // in the browser follows the desktop's FE-C path (17E).
        AntProfile::FitnessEquipment => DeviceKind::Trainer,
    })
}

/// Devices → ANT+ in the browser: the shared page's info.
pub fn menu_info(
    env: Env,
    stick: Option<(StickStatus, bool)>,
    typing: Option<String>,
    scanning: Option<u32>,
) -> AntInfo {
    let Some((status, key_set)) = stick.filter(|_| env.supported) else {
        return AntInfo {
            state: "no WebUSB in this browser".into(),
            web: Some(WebUsbInfo {
                supported: false,
                windows: env.windows,
                ..Default::default()
            }),
            ..Default::default()
        };
    };
    let granted = status.stick().is_some();
    AntInfo {
        state: match &status {
            StickStatus::NoStick => "no stick added (or unplugged)".into(),
            s => s.text(),
        },
        key_set,
        typing,
        key_file: Some(KEPT_IN.into()),
        fix: (env.linux && matches!(status, StickStatus::NoPermission(_)))
            .then_some(AntFix::LinuxAccess),
        access: None,
        web: Some(WebUsbInfo {
            supported: true,
            windows: env.windows,
            granted,
            scanning,
        }),
    }
}

/// `devices.connect ant:…`: the sensor through the page's stick, as a
/// device of the ride.
pub fn connect(devices: &mut WebDevices, id: &str) -> Result<String, String> {
    let kind = kind_of(id).ok_or_else(|| format!("{id} is not an ANT+ sensor id"))?;
    let stick = stick().ok_or("ANT+ needs WebUSB (Chrome or Edge)")?;
    if !stick.has_key() {
        return Err("enter the ANT+ network key first (Devices -> ANT+)".into());
    }
    let name = devices.ant_name(id);
    let handle = stick.connect(id, &name).map_err(|e| e.to_string())?;
    Ok(devices.add(handle, kind, id.to_owned()))
}

/// `ant.scan`: a search for ANT+ sensors (`SCAN_SECONDS`).
fn scan(devices: &mut WebDevices, now_s: f64, stop: bool) -> Result<String, String> {
    let stick = stick().ok_or("ANT+ needs WebUSB (Chrome or Edge)")?;
    if stop {
        stick.stop_scan();
        devices.end_ant_scan();
        return Ok("the ANT+ search stopped".into());
    }
    let rx = stick
        .scan(std::time::Duration::from_secs(u64::from(SCAN_SECONDS)))
        .map_err(|e| e.to_string())?;
    Ok(devices.start_ant_scan(rx, now_s, f64::from(SCAN_SECONDS)))
}

/// The key from this page's storage.
#[cfg(target_arch = "wasm32")]
fn stored_key() -> Option<NetworkKey> {
    let text = web_sys::window()?
        .local_storage()
        .ok()??
        .get_item(KEY_STORAGE)
        .ok()??;
    NetworkKey::parse(&text).ok()
}

fn store_key(key: Option<NetworkKey>) -> Result<(), String> {
    #[cfg(target_arch = "wasm32")]
    {
        let storage = web_sys::window()
            .and_then(|w| w.local_storage().ok().flatten())
            .ok_or("this browser keeps no page storage (private window?)")?;
        match key {
            Some(k) => storage.set_item(KEY_STORAGE, &k.to_hex()),
            None => storage.remove_item(KEY_STORAGE),
        }
        .map_err(|_| "the page's storage refused it".to_string())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = key;
        Err("no page storage outside the browser".into())
    }
}

/// At the start: WebUSB or not; with it, the page's stick service with
/// the stored key (a stick granted before is found without the chooser).
pub fn start() {
    #[cfg(target_arch = "wasm32")]
    {
        let ua = web_sys::window()
            .and_then(|w| w.navigator().user_agent().ok())
            .unwrap_or_default();
        let supported = ocw_devices::ant::web::has_web_usb();
        *ENV.lock().expect("not poisoned") = Env::of(&ua, supported);
        if !supported {
            info!("no WebUSB in this browser: ANT+ sticks connect in Chrome or Edge");
            return;
        }
        let key = stored_key();
        info!(
            key = if key.is_some() { "set" } else { "not set" },
            "ANT+ over WebUSB"
        );
        let (stick, service) = WebAntStick::new(key);
        ocw_devices::ant::web::start(service);
        *STICK.lock().expect("not poisoned") = Some(stick);
    }
}

/// `ant.stick.add`: the browser's chooser for an ANT stick (in the user
/// gesture of the key press or click that ran it).
pub fn add_stick() -> Result<String, String> {
    if !env().supported {
        return Err(
            "this browser has no WebUSB (Firefox, Safari, iOS): ANT+ sticks connect in Chrome or Edge"
                .into(),
        );
    }
    #[cfg(target_arch = "wasm32")]
    {
        let chosen = ocw_devices::ant::web::request()?;
        wasm_bindgen_futures::spawn_local(async move {
            match chosen.await {
                Ok(name) => {
                    say(format!("{name} added: it is used once a sensor needs it"));
                    if let Some(s) = stick() {
                        s.look();
                    }
                }
                Err(e) => say(format!("no ANT stick added: {e}")),
            }
        });
        let mut line = "choose the ANT stick in the browser's list ...".to_string();
        if env().windows {
            line.push_str(" (on Windows it needs the WinUSB driver: Zadig)");
        }
        Ok(line)
    }
    #[cfg(not(target_arch = "wasm32"))]
    Err("no WebUSB outside the browser".into())
}

/// `window.ocw.addAntStick()`: the Devices panel's button (in its click
/// handler).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = addAntStick)]
pub fn page_add_stick() -> Result<String, wasm_bindgen::JsValue> {
    add_stick().map_err(|e| wasm_bindgen::JsValue::from_str(&e))
}

/// The ride's devices (`crate::plugin::Session`).
fn devices_mut(world: &mut World) -> Result<&mut WebDevices, String> {
    world
        .get_resource_mut::<crate::plugin::Session>()
        .map(|s| &mut s.into_inner().0.devices)
        .ok_or_else(|| "no ride yet (the world is still loading)".to_string())
}

/// `ant.key.save` (Enter in the field): the typed key into this page's
/// storage and to the stick. A key that does not read keeps the field
/// open. The key is never printed.
fn save_key(world: &mut World) -> Result<String, String> {
    let text = world.resource::<KeyEntry>().text().to_owned();
    let key = match NetworkKey::parse(&text) {
        Ok(k) => k,
        Err(e) => {
            world.resource_mut::<KeyEntry>().typing = true;
            return Err(format!("ANT+ network key: {e}"));
        }
    };
    world.resource_mut::<KeyEntry>().cancel();
    store_key(Some(key)).map_err(|e| format!("ANT+ network key not kept: {e}"))?;
    if let Some(s) = stick() {
        s.set_key(Some(key));
    }
    Ok("ANT+ network key kept in this browser only (never sent)".into())
}

fn clear_key() -> Result<String, String> {
    store_key(None)?;
    if let Some(s) = stick() {
        s.set_key(None);
    }
    Ok("ANT+ network key removed from this browser".into())
}

/// Enter / Esc in the key's field (Devices → ANT+).
fn poll_key_entry(world: &mut World) {
    let (end, paste) = {
        let Some(mut k) = world.get_resource_mut::<KeyEntry>() else {
            return;
        };
        (k.take_end(), k.take_paste())
    };
    if paste && let Some(text) = crate::PASTED.lock().expect("not poisoned").take() {
        world.resource_mut::<KeyEntry>().field.paste(&text);
    }
    let line = match end {
        Some(FieldEnd::Enter) => save_key(world),
        Some(FieldEnd::Escape) => {
            world.resource_mut::<KeyEntry>().cancel();
            Ok("the ANT+ network key was not changed".into())
        }
        None => return,
    };
    let line = line.unwrap_or_else(|e| e);
    if let Ok(d) = devices_mut(world) {
        d.say_line(line.clone());
    }
    world.resource_mut::<ConsoleState>().print(line);
}

/// The ANT+ commands of the browser (the shared ANT+ page's rows run them).
pub fn register(app: &mut App) {
    app.init_resource::<KeyEntry>()
        .add_systems(Update, poll_key_entry)
        .console_command(ConsoleCommand::new(
            Permission::User,
            Command::new(ANT_ADD_COMMAND).about(
                "Add an ANT+ USB stick: the browser's list of USB devices (Chrome / Edge, WebUSB; on Windows with the WinUSB driver)",
            ),
            |_, _| add_stick(),
        ))
        .console_command(ConsoleCommand::new(
            Permission::User,
            Command::new("ant.scan")
                .about("Find ANT+ heart-rate straps and power meters through the stick (10 s; they are listed on the Devices page)")
                .arg(
                    Arg::new("stop")
                        .long("stop")
                        .action(ArgAction::SetTrue)
                        .help("Stop the search"),
                ),
            |world, m| {
                let now = world.resource::<Time>().elapsed_secs_f64();
                scan(devices_mut(world)?, now, m.get_flag("stop"))
            },
        ))
        .console_command(ConsoleCommand::new(
            Permission::User,
            Command::new("ant.key.edit").about(
                "Type the ANT+ network key (Devices -> ANT+; Enter keeps it in this browser only, Esc cancels)",
            ),
            |world, _| {
                if !env().supported {
                    return Err("ANT+ needs WebUSB (Chrome or Edge)".into());
                }
                world.resource_mut::<KeyEntry>().start();
                Ok("typing the ANT+ network key: Enter keeps it, Esc cancels".into())
            },
        ))
        .console_command(ConsoleCommand::new(
            Permission::User,
            Command::new("ant.key.save").about("Keep the typed ANT+ network key (this browser only)"),
            |world, _| save_key(world),
        ))
        .console_command(ConsoleCommand::new(
            Permission::User,
            Command::new("ant.key.paste")
                .about("Paste the ANT+ network key (Ctrl+V while typing pastes the page's clipboard)"),
            |world, _| {
                let text = crate::PASTED
                    .lock()
                    .expect("not poisoned")
                    .take()
                    .ok_or("paste the key with Ctrl+V while typing it")?;
                let mut k = world.resource_mut::<KeyEntry>();
                if !k.typing {
                    k.start();
                }
                k.field.paste(&text);
                Ok("pasted".into())
            },
        ))
        .console_command(ConsoleCommand::new(
            Permission::User,
            Command::new("ant.key.cancel").about("Stop typing the ANT+ network key"),
            |world, _| {
                world.resource_mut::<KeyEntry>().cancel();
                Ok("the ANT+ network key was not changed".into())
            },
        ))
        .console_command(ConsoleCommand::new(
            Permission::User,
            Command::new("ant.key.clear")
                .about("Remove the ANT+ network key from this browser (ANT+ stops)"),
            |_, _| clear_key(),
        ));
    #[cfg(target_arch = "wasm32")]
    app.add_systems(Startup, start);
}

/// The ANT+ part of the devices' JSON (the browser test; never the key).
pub fn json(devices: &WebDevices) -> serde_json::Value {
    let env = env();
    let stick = stick();
    serde_json::json!({
        "supported": env.supported,
        "windows": env.windows,
        "status": stick.as_ref().map(|s| s.status().text()),
        "key_set": stick.as_ref().is_some_and(WebAntStick::has_key),
        "granted": stick.as_ref().is_some_and(|s| s.status().stick().is_some()),
        "found": devices.ant_found.iter().map(|f| f.id.clone()).collect::<Vec<_>>(),
        "scanning": devices.ant_scanning(),
    })
}

/// The frame's ANT+ part of the devices system: the search's finds, the
/// page side's lines, the menu's info; at the first frame the remembered
/// ANT+ sensors connect again through the stick.
pub fn frame(devices: &mut WebDevices, now_s: f64, first: bool, typing: Option<String>) {
    for line in take_messages() {
        devices.say_line(line);
    }
    devices.poll_ant_scan(now_s);
    let stick = stick();
    if first && let Some(s) = &stick {
        let ids: Vec<String> = devices
            .remembered
            .devices
            .iter()
            .filter(|d| AntDeviceId::is_ant(&d.id))
            .map(|d| d.id.clone())
            .collect();
        if !ids.is_empty() && s.has_key() {
            for id in ids {
                if let Err(e) = connect(devices, &id) {
                    devices.say_line(format!("{id}: {e}"));
                }
            }
        }
    }
    devices.ant_info = Some(menu_info(
        env(),
        stick.as_ref().map(|s| (s.status(), s.has_key())),
        typing,
        devices.ant_scan_left(now_s),
    ));
}

#[cfg(target_arch = "wasm32")]
mod sim {
    //! The page's test hooks: a simulated stick behind the fake
    //! `navigator.usb` (`scripts/web-usb-fake.js`). It hears sensors only
    //! with the made-up test key (`NetworkKey::TEST`), never the ANT+ one.

    use std::cell::RefCell;

    use ocw_devices::ant::NetworkKey;
    use ocw_devices::ant::sim::{SimRadio, parse_sensors};
    use wasm_bindgen::prelude::wasm_bindgen;
    use web_time::Instant;

    thread_local! {
        static SIM: RefCell<Option<(SimRadio, Instant, String)>> = const { RefCell::new(None) };
    }

    /// `window.ocw.antSimStart("hr:1234:141,power:5678:247")`: a fresh
    /// simulated stick with these sensors (also after a replug).
    #[wasm_bindgen(js_name = antSimStart)]
    pub fn start(spec: &str) -> Result<(), wasm_bindgen::JsValue> {
        let sensors = parse_sensors(spec).map_err(|e| wasm_bindgen::JsValue::from_str(&e))?;
        let mut radio = SimRadio::new(Some(NetworkKey::TEST));
        for s in sensors {
            radio.add(s);
        }
        SIM.with(|s| *s.borrow_mut() = Some((radio, Instant::now(), spec.to_owned())));
        Ok(())
    }

    /// `window.ocw.antSimHost(bytes)`: a `transferOut` to the stick.
    #[wasm_bindgen(js_name = antSimHost)]
    pub fn host(bytes: &[u8]) {
        SIM.with(|s| {
            if let Some((radio, t0, _)) = s.borrow_mut().as_mut() {
                radio.host(bytes, t0.elapsed());
            }
        });
    }

    /// `window.ocw.antSimRead()`: what the stick sends now (a
    /// `transferIn`; empty: nothing yet).
    #[wasm_bindgen(js_name = antSimRead)]
    pub fn read() -> Vec<u8> {
        SIM.with(|s| match s.borrow_mut().as_mut() {
            Some((radio, t0, _)) => {
                radio.step(t0.elapsed());
                radio.take_output()
            }
            None => Vec::new(),
        })
    }

    /// `window.ocw.antSimReplug()`: the stick plugged in again (it starts
    /// fresh, with the same sensors).
    #[wasm_bindgen(js_name = antSimReplug)]
    pub fn replug() {
        let spec = SIM.with(|s| s.borrow().as_ref().map(|(_, _, spec)| spec.clone()));
        if let Some(spec) = spec {
            let _ = start(&spec);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_says_where_ant_works() {
        let chrome_linux = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/131.0";
        let android = "Mozilla/5.0 (Linux; Android 14; Pixel 8) Chrome/131.0 Mobile";
        let edge_win = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/131.0 Edg/131.0";
        assert_eq!(
            Env::of(chrome_linux, true),
            Env {
                supported: true,
                windows: false,
                linux: true
            }
        );
        assert!(!Env::of(android, true).linux);
        assert!(Env::of(edge_win, true).windows);
        // Without WebUSB: the page says why, no stick rows.
        let a = menu_info(Env::of("Safari", false), None, None, None);
        assert!(!a.web.as_ref().unwrap().supported);
        // With it: the stick's state, the key kept in the browser.
        let env = Env::of(edge_win, true);
        let a = menu_info(env, Some((StickStatus::NoStick, false)), None, None);
        let w = a.web.clone().unwrap();
        assert!(w.supported && w.windows && !w.granted);
        assert_eq!(a.key_file.as_deref(), Some(KEPT_IN));
        assert!(!a.key_set);
        let a = menu_info(
            env,
            Some((StickStatus::Ready("ANT USB-m stick".into()), true)),
            Some("****|  4 of 16".into()),
            Some(6),
        );
        let w = a.web.clone().unwrap();
        assert!(w.granted && a.key_set && w.scanning == Some(6));
        assert_eq!(a.state, "ANT USB-m stick, in use");
        // A Linux permission: the rule's row.
        let a = menu_info(
            Env::of(chrome_linux, true),
            Some((StickStatus::NoPermission("ANT USB-m stick".into()), true)),
            None,
            None,
        );
        assert_eq!(a.fix, Some(AntFix::LinuxAccess));
        assert_eq!(kind_of("ant:120:1:1"), Some(DeviceKind::HeartRate));
        assert_eq!(kind_of("ant:11:5:5"), Some(DeviceKind::PowerMeter));
        assert_eq!(kind_of("abc"), None);
    }
}
