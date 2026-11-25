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
    /// Who reads the connection ("worker", "main thread"; 10G *B3
    /// worker*).
    reader: &'static str,
}

static STATUS: Mutex<Status> = Mutex::new(Status {
    code: None,
    state: String::new(),
    supported: true,
    json: serde_json::Value::Null,
    reader: "",
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
    v["reader"] = s.reader.into();
    #[cfg(target_arch = "wasm32")]
    {
        v["reader_note"] = crate::netconn::fallback_reason().into();
    }
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
    use js_sys::Reflect;
    use ocw_bridge::PairingCode;
    use ocw_bridge::cert::browser_hashes;
    use ocw_bridge::msg::{Frames, ToBrowser, ToHost, frame};
    use ocw_bridge::relay::{Phase, Session};
    use ocw_devices::{DeviceCommand, DeviceEvent, DeviceHandle, FailReason};
    use wasm_bindgen_futures::spawn_local;

    use super::{BRIDGE_NAME, CODE_KEY, SESSION_KEY, set_state, status};
    use crate::browser::sleep;
    use crate::netconn::{self, ConnOptions, NetEvent, Stage};

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

    fn write(id: u32, m: &ToHost) {
        netconn::send(id, vec![frame(m)], Vec::new());
    }

    fn close(id: u32, reason: &str) {
        netconn::close(id, 0, reason);
    }

    /// A stamp of the network (the wall clock) on the page's clock.
    fn page_ms_of(wall_ms: f64) -> f64 {
        let origin = web_sys::window()
            .and_then(|w| w.performance())
            .map_or(0.0, |p| p.time_origin());
        wall_ms - origin
    }

    /// What the connection's events left for the loop.
    #[derive(Default)]
    struct Link {
        open: bool,
        done: Option<String>,
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
        let url = code.url();
        let link: Rc<RefCell<Link>> = Rc::default();
        // Every message as it arrives (read in the network worker where the
        // browser lets it, stamped when it came; 10G *B3 worker*): the ride
        // takes the events in its next frame.
        let id = {
            let session = session.clone();
            let events = events.clone();
            let link = link.clone();
            let mut frames = Frames::default();
            netconn::open(
                ConnOptions {
                    url: url.clone(),
                    hashes: browser_hashes(code.secret, unix_s).to_vec(),
                    datagrams: false,
                    rtt: false,
                    game: false,
                },
                move |ev| {
                    let mut l = link.borrow_mut();
                    if l.done.is_some() {
                        return;
                    }
                    match ev {
                        NetEvent::Connected { .. } => l.open = true,
                        NetEvent::Stream { at_ms, bytes } => {
                            frames.push(&bytes);
                            let at = page_ms_of(at_ms);
                            while let Some(m) = frames.next_message::<ToBrowser>() {
                                let m = match m {
                                    Ok(m) => m,
                                    Err(e) => {
                                        l.done = Some(e.to_string());
                                        return;
                                    }
                                };
                                let closed_msg = match &m {
                                    ToBrowser::Closed { why } => Some(why.clone()),
                                    _ => None,
                                };
                                for e in session.borrow_mut().on_message(m, at) {
                                    let _ = events.send(e);
                                }
                                if let Some(why) = closed_msg {
                                    l.done = Some(why);
                                    return;
                                }
                            }
                        }
                        NetEvent::StreamEnd => {
                            l.done = Some("the desktop app closed the connection".into())
                        }
                        NetEvent::Closed { reason, error } => {
                            l.done = Some(if reason.is_empty() && !error {
                                "the desktop app closed the connection".into()
                            } else {
                                reason
                            })
                        }
                        NetEvent::Failed {
                            stage: Stage::Ready,
                            error,
                        } => l.done = Some(format!("cannot reach the desktop app: {error}")),
                        NetEvent::Failed { error, .. } => l.done = Some(error),
                        NetEvent::Datagram { .. } | NetEvent::Rtt(_) => {}
                    }
                },
            )
        };
        // Until the stream is open (a stop ends the wait).
        loop {
            if RUN.load(Ordering::Relaxed) != run {
                close(id, "replaced");
                return End::Shutdown;
            }
            if let Some(why) = link.borrow_mut().done.take() {
                close(id, "lost");
                return End::Lost(why);
            }
            if link.borrow().open {
                break;
            }
            sleep(10).await;
        }
        for m in session.borrow_mut().on_open(page_ms()) {
            write(id, &m);
        }
        let reader = netconn::reader(id).map_or("", |r| r.as_str());
        status().reader = reader;
        info!(%url, reader, "sensor bridge: connected");
        let mut next_ping = 0.0;
        loop {
            if RUN.load(Ordering::Relaxed) != run {
                close(id, "replaced");
                return End::Shutdown;
            }
            for cmd in commands.try_iter() {
                if cmd == DeviceCommand::Shutdown {
                    write(id, &ToHost::Bye);
                    sleep(50).await;
                    close(id, "stopped");
                    return End::Shutdown;
                }
                let m = session.borrow_mut().control(cmd, page_ms());
                if let Some(m) = m {
                    write(id, &m);
                }
            }
            if matches!(session.borrow().phase, Phase::Paired { .. }) {
                for (kind, data) in super::take_reports() {
                    write(id, &ToHost::Report { kind, data });
                }
            }
            let now = page_ms();
            if now >= next_ping {
                next_ping = now + 1000.0;
                let p = session.borrow_mut().ping(now);
                write(id, &p);
            }
            {
                let s = session.borrow();
                if let Phase::Refused(why) = &s.phase {
                    let why = why.clone();
                    drop(s);
                    close(id, "refused");
                    return End::Refused(why);
                }
                publish(code, &s);
            }
            if let Some(why) = link.borrow_mut().done.take() {
                close(id, "lost");
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
