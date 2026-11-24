//! The local sensor bridge in the browser (spec 14 Phase 10, 10D.2 *Built
//! (local bridge)*): Settings → Trainer → Devices → **Use a trainer from
//! another device** – the rider types the code the desktop app shows (or
//! opens its link, `?bridge=CODE`), and the desktop's trainer becomes a
//! device of this ride: its power, cadence and heart rate come in as
//! device events (the `SensorSample`s go to the server as with a Bluetooth
//! trainer), the ride's SIM / ERG control goes back.
//!
//! The logic is `ocw_bridge::relay::Session` (tested natively); this moves
//! the bytes over WebTransport (`serverCertificateHashes` derived from the
//! code) and keeps the state the menu and `window.ocw.devices()` show. The
//! code is kept in this origin's `localStorage`, so a reload reconnects.

use std::sync::Mutex;

use ocw_client::menu::devices::BridgeInfo;

/// The bridge's id in the device list (never remembered as a Bluetooth
/// device).
pub const BRIDGE_ID: &str = "bridge";
/// Its name in the device list and the HUD.
pub const BRIDGE_NAME: &str = "Desktop trainer (bridge)";
/// The code of this origin (`localStorage`).
pub const CODE_KEY: &str = "ocw.bridge.code";
/// This browser's page session (`localStorage`): a reload is the same page
/// for the desktop, which lets it take over at once.
pub const SESSION_KEY: &str = "ocw.bridge.session";

/// What the menu and the page show.
#[derive(Debug, Clone)]
struct Status {
    code: Option<String>,
    state: String,
    supported: bool,
    json: serde_json::Value,
}

static STATUS: Mutex<Status> = Mutex::new(Status {
    code: None,
    state: String::new(),
    supported: true,
    json: serde_json::Value::Null,
});

/// A code typed in the console or given by the page, until the ride takes
/// it.
static PENDING: Mutex<Option<String>> = Mutex::new(None);

fn status() -> std::sync::MutexGuard<'static, Status> {
    STATUS.lock().expect("not poisoned")
}

/// The Devices page's bridge rows.
pub fn info() -> BridgeInfo {
    let s = status();
    BridgeInfo::Client {
        code: s.code.clone(),
        state: s.state.clone(),
        supported: s.supported,
    }
}

/// The bridge's state for `window.ocw.devices()` (the page and the test).
pub fn json() -> serde_json::Value {
    let s = status();
    let mut v = s.json.clone();
    if !v.is_object() {
        v = serde_json::json!({});
    }
    v["code"] = s.code.clone().into();
    v["state"] = s.state.clone().into();
    v["supported"] = s.supported.into();
    v
}

fn set_state(code: Option<String>, state: impl Into<String>) {
    let mut s = status();
    s.code = code;
    s.state = state.into();
}

/// A code for the ride to use (`bridge.pair CODE`, the page option).
pub fn request(code: String) {
    *PENDING.lock().expect("not poisoned") = Some(code);
}

/// The code waiting for the ride.
pub fn take_request() -> Option<String> {
    PENDING.lock().expect("not poisoned").take()
}

/// Reports waiting for the connection (spec 14I W4: the headset run's
/// numbers go to the desktop, the only way back from a standalone
/// headset's browser). The newest [`MAX_WAITING`] are kept.
static OUTBOX: Mutex<Vec<(String, serde_json::Value)>> = Mutex::new(Vec::new());
/// Reports kept while the bridge is not paired.
pub const MAX_WAITING: usize = 8;

/// Sends a report (`kind`, the page's JSON) to the desktop with the next
/// bridge step: `Err` when it is too large or no bridge was started; it
/// waits while connecting.
pub fn report(kind: &str, data: serde_json::Value) -> Result<(), String> {
    if data.to_string().len() > ocw_bridge::msg::MAX_REPORT {
        return Err(format!(
            "the report is over {} bytes",
            ocw_bridge::msg::MAX_REPORT
        ));
    }
    if status().code.is_none() {
        return Err("no sensor bridge: pair with the desktop app first".into());
    }
    let mut q = OUTBOX.lock().expect("not poisoned");
    q.push((kind.to_owned(), data));
    let over = q.len().saturating_sub(MAX_WAITING);
    q.drain(..over);
    Ok(())
}

/// The reports to send now.
pub fn take_reports() -> Vec<(String, serde_json::Value)> {
    std::mem::take(&mut *OUTBOX.lock().expect("not poisoned"))
}

/// Stops the bridge: the state says so (the device's Shutdown ends the
/// connection with a goodbye).
pub fn stopped() {
    set_state(None, "off");
    status().json = serde_json::Value::Null;
    #[cfg(target_arch = "wasm32")]
    web::save_code(None);
}

#[cfg(target_arch = "wasm32")]
pub use web::{page_code, prompt_code, start, supported};

#[cfg(not(target_arch = "wasm32"))]
pub fn start(_code: ocw_bridge::PairingCode) -> Result<ocw_devices::DeviceHandle, String> {
    Err("the sensor bridge runs in the browser".into())
}

#[cfg(target_arch = "wasm32")]
mod web {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::mpsc::{self, Receiver, Sender};

    use bevy::prelude::{info, warn};
    use js_sys::{Promise, Reflect, Uint8Array};
    use ocw_bridge::PairingCode;
    use ocw_bridge::cert::browser_hashes;
    use ocw_bridge::msg::{Frames, ToBrowser, ToHost, frame};
    use ocw_bridge::relay::{Phase, Session};
    use ocw_devices::{DeviceCommand, DeviceEvent, DeviceHandle, FailReason};
    use wasm_bindgen::prelude::wasm_bindgen;
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::{JsFuture, spawn_local};

    use super::{BRIDGE_NAME, CODE_KEY, SESSION_KEY, set_state, status};
    use crate::browser::{js, sleep};

    #[wasm_bindgen]
    extern "C" {
        type WebTransport;
        #[wasm_bindgen(constructor, catch)]
        fn new(url: &str, options: &js_sys::Object) -> Result<WebTransport, JsValue>;
        #[wasm_bindgen(method, getter)]
        fn ready(this: &WebTransport) -> Promise;
        #[wasm_bindgen(method, getter)]
        fn closed(this: &WebTransport) -> Promise;
        #[wasm_bindgen(method, js_name = createBidirectionalStream)]
        fn create_bidirectional_stream(this: &WebTransport) -> Promise;
        #[wasm_bindgen(method, js_name = close, catch)]
        fn close_with(this: &WebTransport, info: &js_sys::Object) -> Result<(), JsValue>;

        type BridgeReader;
        #[wasm_bindgen(method)]
        fn read(this: &BridgeReader) -> Promise;
        type BridgeWriter;
        #[wasm_bindgen(method)]
        fn write(this: &BridgeWriter, chunk: &JsValue) -> Promise;
    }

    fn get(o: &JsValue, k: &str) -> JsValue {
        Reflect::get(o, &k.into()).unwrap_or(JsValue::UNDEFINED)
    }

    fn call0(o: &JsValue, f: &str) -> JsValue {
        get(o, f)
            .dyn_ref::<js_sys::Function>()
            .and_then(|f| f.call0(o).ok())
            .unwrap_or(JsValue::UNDEFINED)
    }

    /// Whether the browser has WebTransport.
    pub fn supported() -> bool {
        let s = Reflect::has(&js_sys::global(), &"WebTransport".into()).unwrap_or(false);
        status().supported = s;
        s
    }

    fn storage() -> Option<web_sys::Storage> {
        web_sys::window()?.local_storage().ok()?
    }

    pub(super) fn save_code(code: Option<&str>) {
        if let Some(s) = storage() {
            let _ = match code {
                Some(c) => s.set_item(CODE_KEY, c),
                None => s.remove_item(CODE_KEY),
            };
        }
    }

    /// The code to use at the start: the page's `?bridge=CODE`, else the one
    /// this origin used last.
    pub fn page_code() -> Option<String> {
        let search = web_sys::window()?.location().search().ok()?;
        let from_page = search
            .trim_start_matches('?')
            .split('&')
            .find_map(|kv| kv.strip_prefix("bridge="))
            .map(str::to_owned)
            .filter(|c| !c.is_empty());
        from_page.or_else(|| storage()?.get_item(CODE_KEY).ok().flatten())
    }

    /// Asks for the code (the page's own prompt: the system keyboard on a
    /// headset or a tablet).
    pub fn prompt_code() -> Option<String> {
        web_sys::window()?
            .prompt_with_message(
                "Pairing code from the desktop app (Settings → Trainer → Devices → Share with a browser):",
            )
            .ok()
            .flatten()
            .filter(|c| !c.trim().is_empty())
    }

    fn page_ms() -> f64 {
        web_sys::window()
            .and_then(|w| w.performance())
            .map_or_else(js_sys::Date::now, |p| p.now())
    }

    fn session_id() -> String {
        let fresh = || format!("{:016x}", (js_sys::Math::random() * 2f64.powi(53)) as u64);
        let Some(s) = storage() else { return fresh() };
        if let Ok(Some(id)) = s.get_item(SESSION_KEY) {
            return id;
        }
        let id = fresh();
        let _ = s.set_item(SESSION_KEY, &id);
        id
    }

    fn user_agent() -> String {
        web_sys::window()
            .and_then(|w| w.navigator().user_agent().ok())
            .unwrap_or_default()
    }

    /// The running bridge (a newer start ends the older loop).
    static RUN: AtomicU32 = AtomicU32::new(0);

    /// Starts the bridge for `code`: a device of the ride.
    pub fn start(code: PairingCode) -> Result<DeviceHandle, String> {
        if !supported() {
            return Err("this browser has no WebTransport, which the sensor bridge needs".into());
        }
        let (events_tx, events) = mpsc::channel();
        let (commands, commands_rx) = mpsc::channel();
        let run = RUN.fetch_add(1, Ordering::Relaxed) + 1;
        save_code(Some(&code.compact()));
        set_state(Some(code.to_string()), "connecting");
        spawn_local(run_bridge(run, code, events_tx, commands_rx));
        Ok(DeviceHandle {
            name: BRIDGE_NAME.into(),
            events,
            commands,
        })
    }

    enum End {
        Shutdown,
        Refused(String),
        Lost(String),
    }

    type Shared = Rc<RefCell<Session>>;

    async fn run_bridge(
        run: u32,
        code: PairingCode,
        events: Sender<DeviceEvent>,
        commands: Receiver<DeviceCommand>,
    ) {
        let client = ocw_bridge::browser_name(&user_agent());
        let session: Shared = Rc::new(RefCell::new(Session::new(
            code.secret,
            session_id(),
            client,
            page_ms(),
        )));
        loop {
            match connect_once(run, &code, &session, &events, &commands).await {
                End::Shutdown => {
                    info!("sensor bridge: stopped");
                    return;
                }
                End::Refused(why) => {
                    warn!(%why, "sensor bridge: refused");
                    set_state(Some(code.to_string()), format!("refused: {why}"));
                    let _ = events.send(DeviceEvent::Failed {
                        reason: FailReason::Connect,
                    });
                    return;
                }
                End::Lost(why) => {
                    for e in session.borrow_mut().on_close() {
                        let _ = events.send(e);
                    }
                    let wait = session.borrow_mut().backoff.next_ms();
                    info!(%why, wait_ms = wait, "sensor bridge: reconnecting");
                    set_state(Some(code.to_string()), format!("reconnecting ({why})"));
                    // Wait, still hearing a stop.
                    let until = page_ms() + wait;
                    while page_ms() < until {
                        let mut stop = RUN.load(Ordering::Relaxed) != run;
                        for c in commands.try_iter() {
                            if c == DeviceCommand::Shutdown {
                                stop = true;
                            } else {
                                // Kept for the next hello.
                                let _ = session.borrow_mut().control(c, page_ms());
                            }
                        }
                        if stop {
                            return;
                        }
                        sleep(100).await;
                    }
                }
            }
        }
    }

    fn write(w: &BridgeWriter, m: &ToHost) {
        let p = w.write(&Uint8Array::from(&frame(m)[..]).into());
        spawn_local(async move {
            let _ = JsFuture::from(p).await;
        });
    }

    fn close(t: &WebTransport, reason: &str) {
        let info = js_sys::Object::new();
        let _ = Reflect::set(&info, &"closeCode".into(), &0.into());
        let _ = Reflect::set(&info, &"reason".into(), &reason.into());
        let _ = t.close_with(&info);
    }

    /// One connection: until it is lost, refused or stopped.
    async fn connect_once(
        run: u32,
        code: &PairingCode,
        session: &Shared,
        events: &Sender<DeviceEvent>,
        commands: &Receiver<DeviceCommand>,
    ) -> End {
        let unix_s = (js_sys::Date::now() / 1000.0) as i64;
        let hashes = js_sys::Array::new();
        for h in browser_hashes(code.secret, unix_s) {
            let o = js_sys::Object::new();
            let _ = Reflect::set(&o, &"algorithm".into(), &"sha-256".into());
            let _ = Reflect::set(&o, &"value".into(), &Uint8Array::from(&h[..]).into());
            hashes.push(&o);
        }
        let opts = js_sys::Object::new();
        let _ = Reflect::set(&opts, &"serverCertificateHashes".into(), &hashes.into());
        let url = code.url();
        let t = match WebTransport::new(&url, &opts) {
            Ok(t) => t,
            Err(e) => return End::Lost(js(e)),
        };
        if let Err(e) = JsFuture::from(t.ready()).await {
            return End::Lost(format!("cannot reach the desktop app: {}", js(e)));
        }
        let bidi = match JsFuture::from(t.create_bidirectional_stream()).await {
            Ok(b) => b,
            Err(e) => return End::Lost(js(e)),
        };
        let writer: BridgeWriter = call0(&get(&bidi, "writable"), "getWriter").unchecked_into();
        let reader: BridgeReader = call0(&get(&bidi, "readable"), "getReader").unchecked_into();
        for m in session.borrow_mut().on_open(page_ms()) {
            write(&writer, &m);
        }
        info!(%url, "sensor bridge: connected");
        // The reader: every message as it arrives (the ride takes the
        // events in its next frame).
        let done: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
        {
            let session = session.clone();
            let events = events.clone();
            let done = done.clone();
            let closed = t.closed();
            spawn_local(async move {
                let mut frames = Frames::default();
                loop {
                    let r = match JsFuture::from(reader.read()).await {
                        Ok(r) => r,
                        Err(e) => {
                            *done.borrow_mut() = Some(js(e));
                            return;
                        }
                    };
                    if get(&r, "done").as_bool().unwrap_or(true) {
                        let why = match JsFuture::from(closed).await {
                            Ok(i) => get(&i, "reason").as_string().unwrap_or_default(),
                            Err(e) => js(e),
                        };
                        *done.borrow_mut() = Some(if why.is_empty() {
                            "the desktop app closed the connection".into()
                        } else {
                            why
                        });
                        return;
                    }
                    frames.push(&Uint8Array::new(&get(&r, "value")).to_vec());
                    while let Some(m) = frames.next_message::<ToBrowser>() {
                        let m = match m {
                            Ok(m) => m,
                            Err(e) => {
                                *done.borrow_mut() = Some(e.to_string());
                                return;
                            }
                        };
                        let closed_msg = match &m {
                            ToBrowser::Closed { why } => Some(why.clone()),
                            _ => None,
                        };
                        for e in session.borrow_mut().on_message(m, page_ms()) {
                            let _ = events.send(e);
                        }
                        if let Some(why) = closed_msg {
                            *done.borrow_mut() = Some(why);
                        }
                    }
                }
            });
        }
        let mut next_ping = 0.0;
        loop {
            if RUN.load(Ordering::Relaxed) != run {
                close(&t, "replaced");
                return End::Shutdown;
            }
            for cmd in commands.try_iter() {
                if cmd == DeviceCommand::Shutdown {
                    write(&writer, &ToHost::Bye);
                    sleep(50).await;
                    close(&t, "stopped");
                    return End::Shutdown;
                }
                let m = session.borrow_mut().control(cmd, page_ms());
                if let Some(m) = m {
                    write(&writer, &m);
                }
            }
            if matches!(session.borrow().phase, Phase::Paired { .. }) {
                for (kind, data) in super::take_reports() {
                    write(&writer, &ToHost::Report { kind, data });
                }
            }
            let now = page_ms();
            if now >= next_ping {
                next_ping = now + 1000.0;
                let p = session.borrow_mut().ping(now);
                write(&writer, &p);
            }
            {
                let s = session.borrow();
                if let Phase::Refused(why) = &s.phase {
                    let why = why.clone();
                    drop(s);
                    close(&t, "refused");
                    return End::Refused(why);
                }
                publish(code, &s);
            }
            if let Some(why) = done.borrow_mut().take() {
                close(&t, "lost");
                return End::Lost(why);
            }
            sleep(25).await;
        }
    }

    /// The session's state for the menu and `window.ocw.devices()`.
    fn publish(code: &PairingCode, s: &Session) {
        let state = match &s.phase {
            Phase::Paired { host } => format!("paired with {host}"),
            Phase::Connecting => "connecting".into(),
            Phase::Refused(why) => format!("refused: {why}"),
        };
        let (offset, best_rtt) = s.clock().unzip();
        let mut st = status();
        st.code = Some(code.to_string());
        st.state = state;
        st.json = serde_json::json!({
            "paired": matches!(s.phase, Phase::Paired { .. }),
            "rtt_ms": s.rtt_ms(),
            "best_rtt_ms": best_rtt,
            "clock_offset_ms": offset,
            "sample_age_ms": s.sample_age_ms,
            "control_rtt_ms": s.control_rtt_ms,
            "readings": s.readings,
            "dropped": s.dropped,
            "controls_applied": s.controls_applied,
            "desktop_devices": s.devices.iter().map(|(n, st)| serde_json::json!({"name": n, "state": format!("{st:?}")})).collect::<Vec<_>>(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 14I W4: the headset page's reports wait for a started bridge (the
    /// newest few kept), too large ones are refused.
    #[test]
    fn reports_wait_for_the_bridge_and_keep_the_newest() {
        set_state(None, "off");
        assert!(report("xr", serde_json::json!({"n": 0})).is_err());
        set_state(Some("7K2M-Q9XP-4DA".into()), "connecting");
        for n in 0..(MAX_WAITING + 3) {
            report("xr", serde_json::json!({ "n": n })).unwrap();
        }
        let big = "x".repeat(ocw_bridge::msg::MAX_REPORT + 1);
        assert!(report("xr", serde_json::json!({ "big": big })).is_err());
        let sent = take_reports();
        assert_eq!(sent.len(), MAX_WAITING);
        assert_eq!(sent[0].1["n"], 3);
        assert!(take_reports().is_empty());
        stopped();
    }
}
