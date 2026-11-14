//! The game connection over WebTransport in the browser (spec §14 Phase 10
//! B3; wasm32 only, kept thin): `ocw_server::wire::ClientLink` (unit-tested
//! natively, with the session's fake transport) decides what goes how –
//! the Hello and early samples on the stream, samples as datagrams after
//! the Welcome, frames on the stream – and this only moves the bytes.
//!
//! Own bindings for `WebTransport` and the streams' readers and writers
//! (web-sys has WebTransport only behind `web_sys_unstable_apis`, a cfg
//! for the whole build); every call is structural, so any browser's
//! classes work.

use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use js_sys::{Promise, Reflect, Uint8Array};
use ocw_server::wire::{ClientLink, Outgoing};
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

use crate::browser::{js, sleep};
use crate::session::{RemoteOptions, Transport};

#[wasm_bindgen]
extern "C" {
    type WebTransport;
    #[wasm_bindgen(constructor, catch)]
    fn new(url: &str, options: &js_sys::Object) -> Result<WebTransport, JsValue>;
    #[wasm_bindgen(method, getter)]
    fn ready(this: &WebTransport) -> Promise;
    #[wasm_bindgen(method, getter)]
    fn closed(this: &WebTransport) -> Promise;
    #[wasm_bindgen(method, getter)]
    fn datagrams(this: &WebTransport) -> Datagrams;
    #[wasm_bindgen(method, js_name = createBidirectionalStream)]
    fn create_bidirectional_stream(this: &WebTransport) -> Promise;
    #[wasm_bindgen(method, js_name = getStats, catch)]
    fn get_stats(this: &WebTransport) -> Result<Promise, JsValue>;
    #[wasm_bindgen(method, js_name = close, catch)]
    fn close_with(this: &WebTransport, info: &js_sys::Object) -> Result<(), JsValue>;

    type Datagrams;
    #[wasm_bindgen(method, getter)]
    fn readable(this: &Datagrams) -> Readable;
    #[wasm_bindgen(method, getter)]
    fn writable(this: &Datagrams) -> Writable;
    #[wasm_bindgen(method, js_name = createWritable, catch)]
    fn create_writable(this: &Datagrams) -> Result<Writable, JsValue>;
    #[wasm_bindgen(method, getter, js_name = maxDatagramSize)]
    fn max_datagram_size(this: &Datagrams) -> JsValue;

    type Readable;
    #[wasm_bindgen(method, js_name = getReader)]
    fn get_reader(this: &Readable) -> Reader;
    type Reader;
    #[wasm_bindgen(method)]
    fn read(this: &Reader) -> Promise;

    type Writable;
    #[wasm_bindgen(method, js_name = getWriter)]
    fn get_writer(this: &Writable) -> Writer;
    type Writer;
    #[wasm_bindgen(method)]
    fn write(this: &Writer, chunk: &JsValue) -> Promise;
}

/// The link's clock: the wall clock in seconds, read when the bytes
/// arrive (the snapshot timing's interval and age; the handshake RTT).
fn page_s() -> f64 {
    wall_ms() / 1000.0
}

/// The wall clock in Unix ms with sub-ms precision (snapshot ages).
pub fn wall_ms() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map_or_else(js_sys::Date::now, |p| p.time_origin() + p.now())
}

/// One open connection's JS objects (not `Send`: kept in this thread).
struct JsLink {
    transport: WebTransport,
    stream: Writer,
    datagrams: Option<Writer>,
}

thread_local! {
    static LINKS: std::cell::RefCell<std::collections::HashMap<u32, JsLink>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}
static NEXT_LINK: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

type SharedLink = Arc<Mutex<ClientLink>>;

fn lock(link: &SharedLink) -> std::sync::MutexGuard<'_, ClientLink> {
    link.lock().expect("not poisoned")
}

/// The session's transport in the browser: connects on its first pump
/// (then it has the link to report to) and writes the link's bytes.
pub struct WebTransportLink {
    options: RemoteOptions,
    id: Option<u32>,
}

impl WebTransportLink {
    pub fn new(options: RemoteOptions) -> Self {
        Self { options, id: None }
    }
}

/// Writes one chunk; a failure shows in `closed`.
fn write(w: &Writer, bytes: &[u8]) {
    let p = w.write(&Uint8Array::from(bytes).into());
    wasm_bindgen_futures::spawn_local(async move {
        let _ = JsFuture::from(p).await;
    });
}

impl Transport for WebTransportLink {
    fn pump(&mut self, link: &SharedLink, _now_s: f64) {
        let id = match self.id {
            Some(id) => id,
            None => *self.id.insert(connect(&self.options, link.clone())),
        };
        let out = lock(link).take_outgoing();
        if out.is_empty() {
            return;
        }
        LINKS.with(|links| {
            let links = links.borrow();
            let Some(js) = links.get(&id) else { return };
            for o in out {
                match o {
                    Outgoing::Stream(b) => write(&js.stream, &b),
                    Outgoing::Datagram(b) => {
                        if let Some(d) = &js.datagrams {
                            write(d, &b);
                        }
                    }
                }
            }
        });
    }

    fn close(&mut self) {
        let Some(id) = self.id.take() else { return };
        if let Some(js) = LINKS.with(|l| l.borrow_mut().remove(&id)) {
            let info = js_sys::Object::new();
            let _ = Reflect::set(&info, &"closeCode".into(), &0.into());
            let _ = Reflect::set(&info, &"reason".into(), &"client quit".into());
            let _ = js.transport.close_with(&info);
        }
    }
}

/// `new WebTransport(url, {serverCertificateHashes})`; the rest in a task.
fn connect(options: &RemoteOptions, link: SharedLink) -> u32 {
    let id = NEXT_LINK.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let opts = js_sys::Object::new();
    if !options.cert_hashes.is_empty() {
        let hashes = js_sys::Array::new();
        for h in &options.cert_hashes {
            let o = js_sys::Object::new();
            let _ = Reflect::set(&o, &"algorithm".into(), &"sha-256".into());
            let _ = Reflect::set(&o, &"value".into(), &Uint8Array::from(&h[..]).into());
            hashes.push(&o);
        }
        let _ = Reflect::set(&opts, &"serverCertificateHashes".into(), &hashes.into());
    }
    let transport = match WebTransport::new(&options.url, &opts) {
        Ok(t) => t,
        Err(e) => {
            lock(&link).on_closed(format!("cannot connect: {}", js(e)));
            return id;
        }
    };
    info!(url = %options.url, hashes = options.cert_hashes.len(), "WebTransport: connecting");
    wasm_bindgen_futures::spawn_local(run_link(id, transport, link));
    id
}

fn get(o: &JsValue, k: &str) -> JsValue {
    Reflect::get(o, &k.into()).unwrap_or(JsValue::UNDEFINED)
}

/// Ready → the stream → the readers; then waits for the end.
async fn run_link(id: u32, transport: WebTransport, link: SharedLink) {
    if let Err(e) = JsFuture::from(transport.ready()).await {
        lock(&link).on_closed(format!("cannot connect: {}", js(e)));
        return;
    }
    let bidi = match JsFuture::from(transport.create_bidirectional_stream()).await {
        Ok(b) => b,
        Err(e) => {
            lock(&link).on_closed(format!("cannot open the stream: {}", js(e)));
            return;
        }
    };
    let stream_in: Readable = get(&bidi, "readable").unchecked_into();
    let stream_out: Writable = get(&bidi, "writable").unchecked_into();
    let dg = transport.datagrams();
    let max = dg
        .max_datagram_size()
        .as_f64()
        .filter(|m| *m > 0.0)
        .map(|m| m as usize);
    // createWritable() in newer browsers, `writable` before.
    let dg_out = dg.create_writable().unwrap_or_else(|_| dg.writable());
    let handle: WebTransport = JsValue::from(&transport).unchecked_into();
    LINKS.with(|l| {
        l.borrow_mut().insert(
            id,
            JsLink {
                transport: handle,
                stream: stream_out.get_writer(),
                datagrams: max.map(|_| dg_out.get_writer()),
            },
        )
    });
    lock(&link).on_connected(page_s(), max);
    info!(max_datagram = ?max, "WebTransport: connected");
    wasm_bindgen_futures::spawn_local(read_loop(stream_in.get_reader(), link.clone(), true));
    wasm_bindgen_futures::spawn_local(read_loop(dg.readable().get_reader(), link.clone(), false));
    wasm_bindgen_futures::spawn_local(rtt_loop(id, link.clone()));
    let reason = match JsFuture::from(transport.closed()).await {
        Ok(info) => {
            let reason = get(&info, "reason").as_string().unwrap_or_default();
            if reason.is_empty() {
                "the server closed the connection".to_owned()
            } else {
                format!("the server closed the connection: {reason}")
            }
        }
        Err(e) => format!("connection lost: {}", js(e)),
    };
    LINKS.with(|l| l.borrow_mut().remove(&id));
    warn!(%reason, "WebTransport: closed");
    lock(&link).on_closed(reason);
}

/// Reads the stream's bytes (`stream`) or the datagrams into the link.
async fn read_loop(reader: Reader, link: SharedLink, stream: bool) {
    loop {
        let Ok(r) = JsFuture::from(reader.read()).await else {
            // The connection ended: `closed` says why.
            return;
        };
        if get(&r, "done").as_bool().unwrap_or(true) {
            if stream {
                // Give `closed` the chance to tell the server's reason.
                sleep(500).await;
                lock(&link).on_stream_end();
            }
            return;
        }
        let bytes = Uint8Array::new(&get(&r, "value")).to_vec();
        let mut l = lock(&link);
        if l.is_closed() {
            return;
        }
        if stream {
            l.on_stream_data(page_s(), &bytes);
        } else {
            l.on_datagram(page_s(), bytes);
        }
    }
}

/// The transport's smoothed round trip (`getStats()`, where the browser
/// has it) once a second.
async fn rtt_loop(id: u32, link: SharedLink) {
    loop {
        sleep(1000).await;
        if lock(&link).is_closed() {
            return;
        }
        let stats = LINKS.with(|l| l.borrow().get(&id).map(|js| js.transport.get_stats()));
        let Some(Ok(p)) = stats else { continue };
        if let Ok(s) = JsFuture::from(p).await {
            let rtt = get(&s, "smoothedRtt").as_f64();
            lock(&link).set_transport_rtt(rtt.map(|ms| ms / 1000.0));
        }
    }
}
