//! The page's WebTransport connections, read in a Web Worker (spec 14
//! Phase 10, 10G *B3 worker*): the game server's (B3, `webtransport`) and
//! the sensor bridge's (10D.2, `bridge`).
//!
//! On the page's own thread the readers' promises resolve only between the
//! frames Bevy draws, so every message counted as having come when the
//! frame read it: arrivals were quantised to the frame (16 ms at 60 fps,
//! 40 ms at 25). `web/ocw-net-worker.js` holds the connection instead,
//! stamps each message with the wall clock when it comes off the network
//! and posts it with its bytes (transferred, not copied) and a running
//! number; the page still reads it in its next frame, but knows when it
//! came. No `SharedArrayBuffer`: GitHub Pages sends no COOP / COEP
//! headers, so the page is not cross-origin isolated; `postMessage` is
//! enough at 10 snapshots a second.
//!
//! Where the worker cannot hold the connection – no `Worker`, the script
//! does not load, a worker without `WebTransport` (it says so in its
//! hello), or the page option `net-worker=off` – the same connection runs
//! on the page's thread as before, with the same [`NetEvent`]s: the users
//! see no difference but the timestamps.
//!
//! The pure part (here, tested natively): the worker's messages
//! ([`decode`]) and their order and timestamps ([`Arrivals`]). The browser
//! part (`web`, wasm32 only) moves them.

use std::collections::HashMap;

/// Who reads a connection (page option `net-worker`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReaderMode {
    /// The worker where it can, else the page's thread (`net-worker=on`).
    Auto,
    /// The sensor bridge in the worker, the game link on the page's thread
    /// (`net-worker=bridge`, the default: the own rider moved less smoothly
    /// with the game link in the worker, 10G *Open (B3)*).
    #[default]
    BridgeOnly,
    /// Always the page's thread (`net-worker=off`: the comparison).
    MainThread,
    /// The worker says it has no WebTransport (`net-worker=refuse`, the
    /// fallback test): the page's thread takes over.
    Refuse,
}

impl ReaderMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "auto" | "on" | "1" => Some(Self::Auto),
            "bridge" => Some(Self::BridgeOnly),
            "off" | "0" | "main" => Some(Self::MainThread),
            "refuse" => Some(Self::Refuse),
            _ => None,
        }
    }

    /// Whether a connection goes to the worker (`game`: the game server's,
    /// else the sensor bridge's); where not, why.
    pub fn worker_for(self, game: bool) -> Result<(), &'static str> {
        match self {
            Self::Auto | Self::Refuse => Ok(()),
            Self::BridgeOnly if !game => Ok(()),
            Self::BridgeOnly => Err("net-worker=bridge: the game link on the page's thread"),
            Self::MainThread => Err("net-worker=off"),
        }
    }
}

/// Where a connection's messages are read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reader {
    Worker,
    MainThread,
}

impl Reader {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Worker => "worker",
            Self::MainThread => "main thread",
        }
    }
}

/// Which step of opening a connection failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// `new WebTransport` threw (a bad URL, a bad hash).
    New,
    /// `ready` rejected (nothing there, the certificate refused).
    Ready,
    /// The bidirectional stream could not be opened.
    Stream,
}

/// What happens on a connection, the same from the worker and from the
/// page's thread. `at_ms`: the wall clock (Unix ms, sub-ms) when the bytes
/// came off the network.
#[derive(Debug, Clone, PartialEq)]
pub enum NetEvent {
    Connected {
        at_ms: f64,
        max_datagram: Option<usize>,
    },
    Stream {
        at_ms: f64,
        bytes: Vec<u8>,
    },
    Datagram {
        at_ms: f64,
        bytes: Vec<u8>,
    },
    /// The stream ended (half a second after, so `Closed` can say why
    /// first).
    StreamEnd,
    Failed {
        stage: Stage,
        error: String,
    },
    /// The connection ended: the peer's reason (`error`: false), or the
    /// browser's error (`error`: true).
    Closed {
        reason: String,
        error: bool,
    },
    /// The transport's smoothed round trip (ms; `None`: not reported).
    Rtt(Option<f64>),
}

impl NetEvent {
    /// The arrival stamp, for the events that carry one.
    pub fn at_ms(&self) -> Option<f64> {
        match self {
            Self::Connected { at_ms, .. }
            | Self::Stream { at_ms, .. }
            | Self::Datagram { at_ms, .. } => Some(*at_ms),
            _ => None,
        }
    }

    fn set_at_ms(&mut self, t: f64) {
        if let Self::Connected { at_ms, .. }
        | Self::Stream { at_ms, .. }
        | Self::Datagram { at_ms, .. } = self
        {
            *at_ms = t;
        }
    }
}

/// One message of the worker as its fields (the browser part reads them
/// off the JS object; the tests write them).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Raw {
    pub seq: f64,
    pub id: Option<f64>,
    pub k: String,
    pub t: Option<f64>,
    pub b: Option<Vec<u8>>,
    pub max: Option<f64>,
    pub wt: Option<bool>,
    pub stage: Option<String>,
    pub error: Option<String>,
    pub reason: Option<String>,
    pub err_flag: Option<bool>,
    pub ms: Option<f64>,
}

/// A decoded worker message.
#[derive(Debug, Clone, PartialEq)]
pub enum WorkerMsg {
    /// The worker started; `wt`: it has WebTransport.
    Hello {
        wt: bool,
    },
    /// This connection cannot be read in the worker: the page's thread
    /// takes it.
    Unsupported {
        id: u32,
    },
    Event {
        id: u32,
        event: NetEvent,
    },
}

/// Reads one worker message (see `web/ocw-net-worker.js`).
pub fn decode(r: Raw) -> Result<WorkerMsg, String> {
    if r.k == "hello" {
        return Ok(WorkerMsg::Hello {
            wt: r.wt.unwrap_or(false),
        });
    }
    let id = r
        .id
        .filter(|i| i.is_finite() && *i >= 0.0 && *i <= f64::from(u32::MAX) && i.fract() == 0.0)
        .ok_or_else(|| format!("{}: no connection id", r.k))? as u32;
    let at = || {
        r.t.filter(|t| t.is_finite())
            .ok_or_else(|| format!("{}: no time", r.k))
    };
    let event = match r.k.as_str() {
        "unsupported" => return Ok(WorkerMsg::Unsupported { id }),
        "connected" => NetEvent::Connected {
            at_ms: at()?,
            max_datagram: r.max.filter(|m| *m > 0.0).map(|m| m as usize),
        },
        "s" => NetEvent::Stream {
            at_ms: at()?,
            bytes: r.b.ok_or("s: no bytes")?,
        },
        "d" => NetEvent::Datagram {
            at_ms: at()?,
            bytes: r.b.ok_or("d: no bytes")?,
        },
        "end" => NetEvent::StreamEnd,
        "failed" => NetEvent::Failed {
            stage: match r.stage.as_deref() {
                Some("new") => Stage::New,
                Some("stream") => Stage::Stream,
                _ => Stage::Ready,
            },
            error: r.error.unwrap_or_default(),
        },
        "closed" => NetEvent::Closed {
            reason: r.reason.unwrap_or_default(),
            error: r.err_flag.unwrap_or(false),
        },
        "rtt" => NetEvent::Rtt(r.ms.filter(|m| m.is_finite() && *m >= 0.0)),
        k => return Err(format!("unknown worker message {k:?}")),
    };
    Ok(WorkerMsg::Event { id, event })
}

/// The worker's messages in order, with their stamps kept sane: the
/// running number must go up by one (`postMessage` keeps the order, so a
/// gap or a step back is a bug, counted), each connection's stamps never
/// go back, and no stamp is later than the moment the page reads it (the
/// worker's and the page's clocks are the same wall clock, but rounded
/// apart).
#[derive(Debug, Clone, Default)]
pub struct Arrivals {
    last_seq: Option<u64>,
    last_at: HashMap<u32, f64>,
    pub stats: ArrivalStats,
}

/// What [`Arrivals`] saw.
#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize)]
pub struct ArrivalStats {
    /// Messages from the worker.
    pub messages: u64,
    /// Running numbers that did not follow the last one.
    pub out_of_order: u64,
    /// Stamps moved: later than the page's clock, or before the
    /// connection's last one.
    pub clamped: u64,
    /// How long the stamped messages waited for the page's thread (ms):
    /// the frame quantisation the worker takes out of the timestamps.
    pub wait_ms_mean: f64,
    pub wait_ms_max: f64,
    #[serde(skip)]
    wait_ms_sum: f64,
    #[serde(skip)]
    stamped: u64,
}

impl Arrivals {
    /// A message with running number `seq` for connection `id` (none: the
    /// hello), read on the page at `now_ms`: its event with the stamp
    /// made sane.
    pub fn accept(&mut self, seq: u64, id: Option<u32>, event: Option<&mut NetEvent>, now_ms: f64) {
        self.stats.messages += 1;
        if self.last_seq.is_some_and(|l| seq != l + 1) {
            self.stats.out_of_order += 1;
        }
        self.last_seq = Some(self.last_seq.map_or(seq, |l| l.max(seq)));
        let (Some(id), Some(ev)) = (id, event) else {
            return;
        };
        let Some(at) = ev.at_ms() else { return };
        let floor = self.last_at.get(&id).copied().unwrap_or(f64::NEG_INFINITY);
        let t = at.min(now_ms).max(floor);
        if t != at {
            self.stats.clamped += 1;
            ev.set_at_ms(t);
        }
        self.last_at.insert(id, t);
        let wait = (now_ms - t).max(0.0);
        self.stats.stamped += 1;
        self.stats.wait_ms_sum += wait;
        self.stats.wait_ms_mean = self.stats.wait_ms_sum / self.stats.stamped as f64;
        self.stats.wait_ms_max = self.stats.wait_ms_max.max(wait);
    }

    /// A connection ended: its stamps are forgotten.
    pub fn forget(&mut self, id: u32) {
        self.last_at.remove(&id);
    }
}

/// How many times a game connection whose handshake got no answer is
/// opened again before the join fails (spec 11.5).
pub const HANDSHAKE_RETRIES: u32 = 3;

/// Whether a game connection opens again after `ev`, and in how many
/// seconds; `failed_before`: its handshakes that already failed (spec
/// 11.5). Only a handshake that failed (`ready` rejected: nothing came
/// back, as on Fly.io now and then, where a new UDP flow is lost before it
/// reaches the machine while the next one gets through) is tried again,
/// each time as a new connection (a new UDP port, so a new flow), after
/// 0.5, 1 and 2 s; a refused certificate is tried again too, which costs
/// only a few seconds. A bad URL (`new`), a stream that cannot open or a
/// connection that ends after it was up are not.
pub fn handshake_retry_s(ev: &NetEvent, failed_before: u32) -> Option<f64> {
    match ev {
        NetEvent::Failed {
            stage: Stage::Ready,
            ..
        } if failed_before < HANDSHAKE_RETRIES => Some(0.5 * f64::from(1u32 << failed_before)),
        _ => None,
    }
}

/// The text a link shows for an event that ends it (the game server's
/// wording, as before the worker).
pub fn server_end(ev: &NetEvent) -> Option<String> {
    match ev {
        NetEvent::Failed {
            stage: Stage::Stream,
            error,
        } => Some(format!("cannot open the stream: {error}")),
        NetEvent::Failed { error, .. } => Some(format!("cannot connect: {error}")),
        NetEvent::Closed {
            reason,
            error: true,
        } => Some(format!("connection lost: {reason}")),
        NetEvent::Closed { reason, .. } if reason.is_empty() => {
            Some("the server closed the connection".into())
        }
        NetEvent::Closed { reason, .. } => {
            Some(format!("the server closed the connection: {reason}"))
        }
        _ => None,
    }
}

#[cfg(target_arch = "wasm32")]
pub use web::{close, fallback_reason, open, reader, send, set_default_mode, stats};

/// What to open.
#[derive(Debug, Clone)]
pub struct ConnOptions {
    pub url: String,
    /// `serverCertificateHashes` (SHA-256).
    pub hashes: Vec<[u8; 32]>,
    /// Datagrams too (the game server), or the stream only (the bridge).
    pub datagrams: bool,
    /// Report the smoothed round trip once a second.
    pub rtt: bool,
    /// The game server's connection (else the sensor bridge's).
    pub game: bool,
}

#[cfg(target_arch = "wasm32")]
mod web {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    use bevy::prelude::{info, warn};
    use js_sys::{Array, Promise, Reflect, Uint8Array};
    use wasm_bindgen::prelude::{Closure, wasm_bindgen};
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::{JsFuture, spawn_local};

    use super::*;
    use crate::browser::{js, sleep};

    /// The worker's script, next to the page.
    const SCRIPT: &str = "ocw-net-worker.js";
    /// No hello in this time: the worker did not start. (A page busy
    /// loading a world may hold the hello's message for seconds: the timer
    /// then gives the queued messages another moment before it decides.)
    const HELLO_MS: i32 = 8000;

    #[wasm_bindgen]
    extern "C" {
        type Worker;
        #[wasm_bindgen(constructor, catch)]
        fn new(url: &str) -> Result<Worker, JsValue>;
        #[wasm_bindgen(method, js_name = postMessage, catch)]
        fn post(this: &Worker, msg: &JsValue, transfer: &Array) -> Result<(), JsValue>;
        #[wasm_bindgen(method, setter)]
        fn set_onmessage(this: &Worker, f: &js_sys::Function);
        #[wasm_bindgen(method, setter)]
        fn set_onerror(this: &Worker, f: &js_sys::Function);

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
        fn get_reader(this: &Readable) -> StreamReader;
        type StreamReader;
        #[wasm_bindgen(method)]
        fn read(this: &StreamReader) -> Promise;

        type Writable;
        #[wasm_bindgen(method, js_name = getWriter)]
        fn get_writer(this: &Writable) -> Writer;
        type Writer;
        #[wasm_bindgen(method)]
        fn write(this: &Writer, chunk: &JsValue) -> Promise;
    }

    /// The wall clock in Unix ms with sub-ms precision (the worker stamps
    /// with the same clock).
    pub fn wall_ms() -> f64 {
        web_sys::window()
            .and_then(|w| w.performance())
            .map_or_else(js_sys::Date::now, |p| p.time_origin() + p.now())
    }

    type Handler = Rc<RefCell<Box<dyn FnMut(NetEvent)>>>;

    enum Backend {
        /// Waiting for the worker's hello.
        Waiting,
        Worker,
        /// On the page's thread (its writers once open).
        Main(Option<MainJs>),
    }

    struct MainJs {
        transport: WebTransport,
        stream: Writer,
        datagrams: Option<Writer>,
    }

    struct Conn {
        options: ConnOptions,
        handler: Handler,
        backend: Backend,
        /// Sent before the page's thread had the stream open.
        early: Vec<Chunks>,
    }

    /// Bytes for the stream and datagrams, sent together.
    type Chunks = (Vec<Vec<u8>>, Vec<Vec<u8>>);

    enum WorkerState {
        NotStarted,
        Starting(Worker),
        Ready(Worker),
        Failed,
    }

    struct Net {
        mode: ReaderMode,
        worker: WorkerState,
        conns: HashMap<u32, Conn>,
        next: u32,
        arrivals: Arrivals,
        /// Why the last connection went to the page's thread.
        fallback: String,
    }

    thread_local! {
        static NET: RefCell<Net> = RefCell::new(Net {
            mode: ReaderMode::Auto,
            worker: WorkerState::NotStarted,
            conns: HashMap::new(),
            next: 1,
            arrivals: Arrivals::default(),
            fallback: String::new(),
        });
    }

    /// The page option `net-worker` (before the first connection).
    pub fn set_default_mode(mode: ReaderMode) {
        NET.with(|n| n.borrow_mut().mode = mode);
    }

    /// Why the last connection was read on the page's thread ("" none).
    pub fn fallback_reason() -> String {
        NET.with(|n| n.borrow().fallback.clone())
    }

    /// What the worker's messages looked like (for `net.stats`).
    pub fn stats() -> ArrivalStats {
        NET.with(|n| n.borrow().arrivals.stats)
    }

    /// Who reads connection `id` (`None`: not decided yet, or closed).
    pub fn reader(id: u32) -> Option<Reader> {
        NET.with(|n| match n.borrow().conns.get(&id)?.backend {
            Backend::Waiting => None,
            Backend::Worker => Some(Reader::Worker),
            Backend::Main(_) => Some(Reader::MainThread),
        })
    }

    fn deliver(handler: &Handler, ev: NetEvent) {
        (handler.borrow_mut())(ev);
    }

    /// Opens a connection; `handler` gets its events (on the page's
    /// thread, never inside another call of this module).
    pub fn open(options: ConnOptions, handler: impl FnMut(NetEvent) + 'static) -> u32 {
        let handler: Handler = Rc::new(RefCell::new(Box::new(handler)));
        let game = options.game;
        let (id, mode) = NET.with(|n| {
            let mut n = n.borrow_mut();
            let id = n.next;
            n.next += 1;
            n.conns.insert(
                id,
                Conn {
                    options,
                    handler,
                    backend: Backend::Waiting,
                    early: Vec::new(),
                },
            );
            (id, n.mode)
        });
        if let Err(why) = mode.worker_for(game) {
            to_main(id, why);
            return id;
        }
        match ensure_worker() {
            Some(true) => to_worker(id),
            Some(false) => {} // the hello sends it
            None => to_main(id, "no network worker"),
        }
        id
    }

    /// Starts the worker once: `Some(true)` ready, `Some(false)` starting,
    /// `None` it cannot run.
    fn ensure_worker() -> Option<bool> {
        let state = NET.with(|n| match &n.borrow().worker {
            WorkerState::NotStarted => None,
            WorkerState::Starting(_) => Some(Some(false)),
            WorkerState::Ready(_) => Some(Some(true)),
            WorkerState::Failed => Some(None),
        });
        if let Some(s) = state {
            return s;
        }
        let has_worker = Reflect::has(&js_sys::global(), &"Worker".into()).unwrap_or(false);
        let worker = match has_worker.then(|| Worker::new(SCRIPT)) {
            Some(Ok(w)) => w,
            Some(Err(e)) => {
                warn!(error = %js(e), "network worker: cannot start; reading on the page's thread");
                NET.with(|n| n.borrow_mut().worker = WorkerState::Failed);
                return None;
            }
            None => {
                NET.with(|n| n.borrow_mut().worker = WorkerState::Failed);
                return None;
            }
        };
        let on_message = Closure::<dyn FnMut(JsValue)>::new(on_message);
        worker.set_onmessage(on_message.as_ref().unchecked_ref());
        on_message.forget();
        let on_error = Closure::<dyn FnMut(JsValue)>::new(|e: JsValue| {
            let msg = Reflect::get(&e, &"message".into())
                .ok()
                .and_then(|m| m.as_string())
                .unwrap_or_else(|| "the script did not load".into());
            worker_failed(&msg);
        });
        worker.set_onerror(on_error.as_ref().unchecked_ref());
        on_error.forget();
        NET.with(|n| n.borrow_mut().worker = WorkerState::Starting(worker));
        spawn_local(async {
            sleep(HELLO_MS).await;
            for _ in 0..3 {
                // Let a hello queued behind a long frame come first.
                sleep(100).await;
                if !NET.with(|n| matches!(n.borrow().worker, WorkerState::Starting(_))) {
                    return;
                }
            }
            worker_failed("no hello");
        });
        Some(false)
    }

    /// The worker cannot be used: what waits for it goes to the page's
    /// thread, what it held is lost.
    fn worker_failed(why: &str) {
        let (waiting, lost) = NET.with(|n| {
            let mut n = n.borrow_mut();
            if matches!(n.worker, WorkerState::Failed) {
                return (Vec::new(), Vec::new());
            }
            n.worker = WorkerState::Failed;
            let mut waiting = Vec::new();
            let mut lost = Vec::new();
            for (id, c) in &n.conns {
                match c.backend {
                    Backend::Waiting => waiting.push(*id),
                    Backend::Worker => lost.push((*id, c.handler.clone())),
                    Backend::Main(_) => {}
                }
            }
            for (id, _) in &lost {
                n.conns.remove(id);
            }
            (waiting, lost)
        });
        warn!(%why, "network worker: failed; reading on the page's thread");
        for id in waiting {
            to_main(id, why);
        }
        for (_, h) in lost {
            deliver(
                &h,
                NetEvent::Closed {
                    reason: format!("the network worker failed: {why}"),
                    error: true,
                },
            );
        }
    }

    fn set(o: &js_sys::Object, k: &str, v: &JsValue) {
        let _ = Reflect::set(o, &k.into(), v);
    }

    /// Hands connection `id` to the worker.
    fn to_worker(id: u32) {
        let got = NET.with(|n| {
            let mut n = n.borrow_mut();
            let mode = n.mode;
            let WorkerState::Ready(w) = &n.worker else {
                return None;
            };
            let w: Worker = JsValue::from(w).unchecked_into();
            let c = n.conns.get_mut(&id)?;
            c.backend = Backend::Worker;
            Some((w, mode, c.options.clone(), std::mem::take(&mut c.early)))
        });
        let Some((w, mode, options, early)) = got else {
            return;
        };
        let msg = js_sys::Object::new();
        set(&msg, "op", &"connect".into());
        set(&msg, "id", &id.into());
        set(&msg, "url", &options.url.as_str().into());
        let hashes = Array::new();
        for h in &options.hashes {
            hashes.push(&Uint8Array::from(&h[..]));
        }
        set(&msg, "hashes", &hashes);
        set(&msg, "datagrams", &options.datagrams.into());
        set(&msg, "rtt", &options.rtt.into());
        set(&msg, "refuse", &(mode == ReaderMode::Refuse).into());
        if w.post(&msg, &Array::new()).is_err() {
            to_main(id, "cannot post to the worker");
            return;
        }
        info!(id, "network worker: connecting");
        for (s, d) in early {
            post_send(&w, id, s, d);
        }
    }

    fn on_message(e: JsValue) {
        let now = wall_ms();
        let d = Reflect::get(&e, &"data".into()).unwrap_or(JsValue::UNDEFINED);
        let get = |k: &str| Reflect::get(&d, &k.into()).unwrap_or(JsValue::UNDEFINED);
        let raw = Raw {
            seq: get("seq").as_f64().unwrap_or(0.0),
            id: get("id").as_f64(),
            k: get("k").as_string().unwrap_or_default(),
            t: get("t").as_f64(),
            b: {
                let b = get("b");
                (!b.is_undefined() && !b.is_null()).then(|| Uint8Array::new(&b).to_vec())
            },
            max: get("max").as_f64(),
            wt: get("wt").as_bool(),
            stage: get("stage").as_string(),
            error: get("error").as_string(),
            reason: get("reason").as_string(),
            err_flag: get("error").as_bool(),
            ms: get("ms").as_f64(),
        };
        let seq = raw.seq as u64;
        let msg = match decode(raw) {
            Ok(m) => m,
            Err(e) => {
                warn!(error = %e, "network worker: bad message");
                return;
            }
        };
        match msg {
            WorkerMsg::Hello { wt } => {
                NET.with(|n| n.borrow_mut().arrivals.accept(seq, None, None, now));
                if !wt {
                    worker_failed("no WebTransport in a worker here");
                    return;
                }
                let waiting = NET.with(|n| {
                    let mut n = n.borrow_mut();
                    let old = std::mem::replace(&mut n.worker, WorkerState::Failed);
                    n.worker = match old {
                        WorkerState::Starting(w) | WorkerState::Ready(w) => WorkerState::Ready(w),
                        other => other,
                    };
                    n.conns
                        .iter()
                        .filter(|(_, c)| matches!(c.backend, Backend::Waiting))
                        .map(|(id, _)| *id)
                        .collect::<Vec<_>>()
                });
                info!("network worker: started");
                for id in waiting {
                    to_worker(id);
                }
            }
            WorkerMsg::Unsupported { id } => {
                NET.with(|n| n.borrow_mut().arrivals.accept(seq, Some(id), None, now));
                to_main(id, "the worker has no WebTransport");
            }
            WorkerMsg::Event { id, mut event } => {
                let handler = NET.with(|n| {
                    let mut n = n.borrow_mut();
                    n.arrivals.accept(seq, Some(id), Some(&mut event), now);
                    let c = n.conns.get(&id)?;
                    matches!(c.backend, Backend::Worker).then(|| c.handler.clone())
                });
                let ends = matches!(event, NetEvent::Closed { .. } | NetEvent::Failed { .. });
                if let Some(h) = handler {
                    deliver(&h, event);
                }
                if ends {
                    NET.with(|n| {
                        let mut n = n.borrow_mut();
                        n.conns.remove(&id);
                        n.arrivals.forget(id);
                    });
                }
            }
        }
    }

    fn post_send(w: &Worker, id: u32, stream: Vec<Vec<u8>>, datagrams: Vec<Vec<u8>>) {
        let msg = js_sys::Object::new();
        set(&msg, "op", &"send".into());
        set(&msg, "id", &id.into());
        let transfer = Array::new();
        let list = |items: Vec<Vec<u8>>| {
            let a = Array::new();
            for b in items {
                let u = Uint8Array::from(&b[..]);
                transfer.push(&u.buffer());
                a.push(&u);
            }
            a
        };
        set(&msg, "s", &list(stream));
        set(&msg, "d", &list(datagrams));
        let _ = w.post(&msg, &transfer);
    }

    /// Sends bytes on connection `id`: on its stream, and as datagrams
    /// (dropped where the connection has none).
    pub fn send(id: u32, stream: Vec<Vec<u8>>, datagrams: Vec<Vec<u8>>) {
        NET.with(|n| {
            let mut n = n.borrow_mut();
            let n = &mut *n;
            let Some(c) = n.conns.get_mut(&id) else {
                return;
            };
            match &c.backend {
                Backend::Worker => {
                    if let WorkerState::Ready(w) = &n.worker {
                        post_send(w, id, stream, datagrams);
                    }
                }
                Backend::Main(Some(js)) => {
                    for b in stream {
                        write(&js.stream, &b);
                    }
                    if let Some(d) = &js.datagrams {
                        for b in datagrams {
                            write(d, &b);
                        }
                    }
                }
                Backend::Waiting | Backend::Main(None) => c.early.push((stream, datagrams)),
            }
        });
    }

    /// Closes connection `id` (`code`, `reason` to the peer); no more
    /// events.
    pub fn close(id: u32, code: u32, reason: &str) {
        let (conn, worker) = NET.with(|n| {
            let mut n = n.borrow_mut();
            n.arrivals.forget(id);
            let c = n.conns.remove(&id);
            let w = match &n.worker {
                WorkerState::Ready(w) => Some(JsValue::from(w).unchecked_into::<Worker>()),
                _ => None,
            };
            (c, w)
        });
        let Some(c) = conn else { return };
        match c.backend {
            Backend::Worker => {
                if let Some(w) = worker {
                    let msg = js_sys::Object::new();
                    set(&msg, "op", &"close".into());
                    set(&msg, "id", &id.into());
                    set(&msg, "code", &code.into());
                    set(&msg, "reason", &reason.into());
                    let _ = w.post(&msg, &Array::new());
                }
            }
            Backend::Main(Some(js)) => {
                let info = js_sys::Object::new();
                set(&info, "closeCode", &code.into());
                set(&info, "reason", &reason.into());
                let _ = js.transport.close_with(&info);
            }
            Backend::Waiting | Backend::Main(None) => {}
        }
    }

    /// Writes one chunk; a failure shows in `closed`.
    fn write(w: &Writer, bytes: &[u8]) {
        let p = w.write(&Uint8Array::from(bytes).into());
        spawn_local(async move {
            let _ = JsFuture::from(p).await;
        });
    }

    fn get(o: &JsValue, k: &str) -> JsValue {
        Reflect::get(o, &k.into()).unwrap_or(JsValue::UNDEFINED)
    }

    /// The handler of `id` while it is read on the page's thread.
    fn main_handler(id: u32) -> Option<Handler> {
        NET.with(|n| {
            let n = n.borrow();
            let c = n.conns.get(&id)?;
            matches!(c.backend, Backend::Main(_)).then(|| c.handler.clone())
        })
    }

    /// An event of a connection on the page's thread (`false`: it is gone).
    fn main_event(id: u32, ev: NetEvent) -> bool {
        let Some(h) = main_handler(id) else {
            return false;
        };
        let ends = matches!(ev, NetEvent::Closed { .. } | NetEvent::Failed { .. });
        deliver(&h, ev);
        if ends {
            NET.with(|n| n.borrow_mut().conns.remove(&id));
        }
        true
    }

    /// Connection `id` on the page's thread, as before the worker.
    fn to_main(id: u32, why: &str) {
        let options = NET.with(|n| {
            let mut n = n.borrow_mut();
            n.fallback = why.to_owned();
            let c = n.conns.get_mut(&id)?;
            c.backend = Backend::Main(None);
            Some(c.options.clone())
        });
        let Some(options) = options else { return };
        info!(id, %why, "WebTransport read on the page's thread");
        let opts = js_sys::Object::new();
        if !options.hashes.is_empty() {
            let hashes = Array::new();
            for h in &options.hashes {
                let o = js_sys::Object::new();
                set(&o, "algorithm", &"sha-256".into());
                set(&o, "value", &Uint8Array::from(&h[..]).into());
                hashes.push(&o);
            }
            set(&opts, "serverCertificateHashes", &hashes);
        }
        match WebTransport::new(&options.url, &opts) {
            Ok(t) => spawn_local(run_main(id, t, options)),
            Err(e) => {
                main_event(
                    id,
                    NetEvent::Failed {
                        stage: Stage::New,
                        error: js(e),
                    },
                );
            }
        }
    }

    /// Ready → the stream → the readers; then waits for the end.
    async fn run_main(id: u32, transport: WebTransport, options: ConnOptions) {
        if let Err(e) = JsFuture::from(transport.ready()).await {
            main_event(
                id,
                NetEvent::Failed {
                    stage: Stage::Ready,
                    error: js(e),
                },
            );
            return;
        }
        let bidi = match JsFuture::from(transport.create_bidirectional_stream()).await {
            Ok(b) => b,
            Err(e) => {
                main_event(
                    id,
                    NetEvent::Failed {
                        stage: Stage::Stream,
                        error: js(e),
                    },
                );
                return;
            }
        };
        let stream_in: Readable = get(&bidi, "readable").unchecked_into();
        let stream_out: Writable = get(&bidi, "writable").unchecked_into();
        let mut max = None;
        let mut dg_writer = None;
        let mut dg_reader = None;
        if options.datagrams {
            let dg = transport.datagrams();
            max = dg
                .max_datagram_size()
                .as_f64()
                .filter(|m| *m > 0.0)
                .map(|m| m as usize);
            // createWritable() in newer browsers, `writable` before.
            let out = dg.create_writable().unwrap_or_else(|_| dg.writable());
            dg_writer = max.map(|_| out.get_writer());
            dg_reader = Some(dg.readable().get_reader());
        }
        let handle: WebTransport = JsValue::from(&transport).unchecked_into();
        let early = NET.with(|n| {
            let mut n = n.borrow_mut();
            let c = n.conns.get_mut(&id)?;
            c.backend = Backend::Main(Some(MainJs {
                transport: handle,
                stream: stream_out.get_writer(),
                datagrams: dg_writer,
            }));
            Some(std::mem::take(&mut c.early))
        });
        let Some(early) = early else {
            // Closed while connecting.
            let info = js_sys::Object::new();
            set(&info, "closeCode", &0.into());
            let _ = transport.close_with(&info);
            return;
        };
        if !main_event(
            id,
            NetEvent::Connected {
                at_ms: wall_ms(),
                max_datagram: max,
            },
        ) {
            return;
        }
        for (s, d) in early {
            send(id, s, d);
        }
        spawn_local(read_main(id, stream_in.get_reader(), true));
        if let Some(r) = dg_reader {
            spawn_local(read_main(id, r, false));
        }
        if options.rtt {
            spawn_local(rtt_main(id));
        }
        let ev = match JsFuture::from(transport.closed()).await {
            Ok(info) => NetEvent::Closed {
                reason: get(&info, "reason").as_string().unwrap_or_default(),
                error: false,
            },
            Err(e) => NetEvent::Closed {
                reason: js(e),
                error: true,
            },
        };
        main_event(id, ev);
    }

    /// Reads the stream's bytes (`stream`) or the datagrams.
    async fn read_main(id: u32, reader: StreamReader, stream: bool) {
        loop {
            let Ok(r) = JsFuture::from(reader.read()).await else {
                // The connection ended: `closed` says why.
                return;
            };
            let at_ms = wall_ms();
            if get(&r, "done").as_bool().unwrap_or(true) {
                if stream {
                    // Give `closed` the chance to tell the peer's reason.
                    sleep(500).await;
                    main_event(id, NetEvent::StreamEnd);
                }
                return;
            }
            let bytes = Uint8Array::new(&get(&r, "value")).to_vec();
            let ev = if stream {
                NetEvent::Stream { at_ms, bytes }
            } else {
                NetEvent::Datagram { at_ms, bytes }
            };
            if !main_event(id, ev) {
                return;
            }
        }
    }

    /// The transport's smoothed round trip (`getStats()`, where the
    /// browser has it) once a second.
    async fn rtt_main(id: u32) {
        loop {
            sleep(1000).await;
            let stats = NET.with(|n| match &n.borrow().conns.get(&id)?.backend {
                Backend::Main(Some(js)) => Some(js.transport.get_stats()),
                _ => None,
            });
            let Some(stats) = stats else { return };
            let Ok(p) = stats else { continue };
            if let Ok(s) = JsFuture::from(p).await
                && !main_event(id, NetEvent::Rtt(get(&s, "smoothedRtt").as_f64()))
            {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(seq: u64, id: u32, k: &str, t: f64) -> Raw {
        Raw {
            seq: seq as f64,
            id: Some(f64::from(id)),
            k: k.into(),
            t: Some(t),
            b: Some(vec![seq as u8]),
            ..Default::default()
        }
    }

    #[test]
    fn handshakes_without_an_answer_are_tried_again_on_a_new_flow() {
        let ready = NetEvent::Failed {
            stage: Stage::Ready,
            error: "QUIC_NETWORK_IDLE_TIMEOUT".into(),
        };
        let delays: Vec<_> = (0..4).map(|n| handshake_retry_s(&ready, n)).collect();
        assert_eq!(delays, [Some(0.5), Some(1.0), Some(2.0), None]);
        for ev in [
            NetEvent::Failed {
                stage: Stage::New,
                error: "bad url".into(),
            },
            NetEvent::Failed {
                stage: Stage::Stream,
                error: "no stream".into(),
            },
            NetEvent::Closed {
                reason: "lost".into(),
                error: true,
            },
            NetEvent::StreamEnd,
        ] {
            assert_eq!(handshake_retry_s(&ev, 0), None, "{ev:?}");
        }
    }

    #[test]
    fn worker_messages_decode() {
        assert_eq!(
            decode(Raw {
                k: "hello".into(),
                wt: Some(true),
                ..Default::default()
            }),
            Ok(WorkerMsg::Hello { wt: true })
        );
        assert_eq!(
            decode(Raw {
                k: "unsupported".into(),
                id: Some(3.0),
                ..Default::default()
            }),
            Ok(WorkerMsg::Unsupported { id: 3 })
        );
        let c = Raw {
            max: Some(1200.0),
            ..raw(2, 1, "connected", 10.5)
        };
        assert_eq!(
            decode(c),
            Ok(WorkerMsg::Event {
                id: 1,
                event: NetEvent::Connected {
                    at_ms: 10.5,
                    max_datagram: Some(1200)
                }
            })
        );
        // No datagrams (null / 0): none.
        let c = Raw {
            max: None,
            ..raw(2, 1, "connected", 10.5)
        };
        assert!(matches!(
            decode(c),
            Ok(WorkerMsg::Event {
                event: NetEvent::Connected {
                    max_datagram: None,
                    ..
                },
                ..
            })
        ));
        assert_eq!(
            decode(raw(3, 1, "d", 11.0)),
            Ok(WorkerMsg::Event {
                id: 1,
                event: NetEvent::Datagram {
                    at_ms: 11.0,
                    bytes: vec![3]
                }
            })
        );
        assert!(matches!(
            decode(raw(4, 1, "s", 12.0)),
            Ok(WorkerMsg::Event {
                event: NetEvent::Stream { .. },
                ..
            })
        ));
        let f = Raw {
            stage: Some("stream".into()),
            error: Some("WebTransportError: no".into()),
            ..raw(5, 1, "failed", 0.0)
        };
        let Ok(WorkerMsg::Event { event, .. }) = decode(f) else {
            panic!()
        };
        assert_eq!(
            server_end(&event).as_deref(),
            Some("cannot open the stream: WebTransportError: no")
        );
        let closed = |reason: &str, error| {
            let Ok(WorkerMsg::Event { event, .. }) = decode(Raw {
                reason: Some(reason.into()),
                err_flag: Some(error),
                ..raw(6, 1, "closed", 0.0)
            }) else {
                panic!()
            };
            server_end(&event).unwrap()
        };
        assert_eq!(closed("", false), "the server closed the connection");
        assert_eq!(
            closed("shutdown", false),
            "the server closed the connection: shutdown"
        );
        assert_eq!(closed("reset", true), "connection lost: reset");
        assert_eq!(
            decode(Raw {
                ms: Some(4.5),
                ..raw(7, 1, "rtt", 0.0)
            }),
            Ok(WorkerMsg::Event {
                id: 1,
                event: NetEvent::Rtt(Some(4.5))
            })
        );
        // Broken messages are refused, not guessed.
        assert!(
            decode(Raw {
                t: None,
                ..raw(8, 1, "d", 0.0)
            })
            .is_err()
        );
        assert!(
            decode(Raw {
                b: None,
                ..raw(8, 1, "s", 0.0)
            })
            .is_err()
        );
        assert!(
            decode(Raw {
                id: None,
                ..raw(8, 1, "d", 0.0)
            })
            .is_err()
        );
        assert!(
            decode(Raw {
                id: Some(1.5),
                ..raw(8, 1, "d", 0.0)
            })
            .is_err()
        );
        assert!(decode(raw(8, 1, "x", 0.0)).is_err());
    }

    /// The stamps the page uses: in order they pass unchanged, however
    /// long they waited for the frame; the wait is measured.
    #[test]
    fn stamps_in_order_pass_and_the_wait_is_measured() {
        let mut a = Arrivals::default();
        a.accept(1, None, None, 1000.0); // the hello
        // Three datagrams 100 ms apart, read by frames 40 ms apart.
        let mut got = Vec::new();
        for (i, (t, now)) in [(1003.0, 1040.0), (1103.0, 1120.0), (1203.0, 1240.0)]
            .into_iter()
            .enumerate()
        {
            let mut ev = NetEvent::Datagram {
                at_ms: t,
                bytes: vec![],
            };
            a.accept(i as u64 + 2, Some(1), Some(&mut ev), now);
            got.push(ev.at_ms().unwrap());
        }
        assert_eq!(got, vec![1003.0, 1103.0, 1203.0]);
        assert_eq!(a.stats.messages, 4);
        assert_eq!(a.stats.out_of_order, 0);
        assert_eq!(a.stats.clamped, 0);
        assert!((a.stats.wait_ms_mean - (37.0 + 17.0 + 37.0) / 3.0).abs() < 1e-9);
        assert_eq!(a.stats.wait_ms_max, 37.0);
    }

    /// A stamp ahead of the page's clock (the two clocks rounded apart)
    /// is the page's now; one before the connection's last stays at the
    /// last; another connection has its own; gaps in the running numbers
    /// are counted.
    #[test]
    fn stamps_never_run_ahead_or_back_and_gaps_are_counted() {
        let mut a = Arrivals::default();
        let ev = |t| NetEvent::Stream {
            at_ms: t,
            bytes: vec![],
        };
        let mut e = ev(2000.4);
        a.accept(1, Some(1), Some(&mut e), 2000.0);
        assert_eq!(e.at_ms(), Some(2000.0));
        let mut e = ev(1999.0);
        a.accept(2, Some(1), Some(&mut e), 2001.0);
        assert_eq!(e.at_ms(), Some(2000.0), "not before the last");
        let mut e = ev(1999.0);
        a.accept(3, Some(2), Some(&mut e), 2001.0);
        assert_eq!(e.at_ms(), Some(1999.0), "connection 2 has its own");
        assert_eq!(a.stats.clamped, 2);
        assert_eq!(a.stats.out_of_order, 0);
        // A message lost or swapped in between: counted.
        let mut e = ev(2002.0);
        a.accept(5, Some(1), Some(&mut e), 2003.0);
        let mut e = ev(2002.5);
        a.accept(4, Some(1), Some(&mut e), 2003.0);
        assert_eq!(a.stats.out_of_order, 2);
        // A connection that ended starts again.
        a.forget(1);
        let mut e = ev(10.0);
        a.accept(6, Some(1), Some(&mut e), 2004.0);
        assert_eq!(e.at_ms(), Some(10.0));
        // Events without a stamp only count.
        a.accept(7, Some(1), Some(&mut NetEvent::StreamEnd), 2005.0);
        assert_eq!(a.stats.messages, 7);
    }

    #[test]
    fn reader_modes_parse() {
        assert_eq!(ReaderMode::parse("auto"), Some(ReaderMode::Auto));
        assert_eq!(ReaderMode::parse("off"), Some(ReaderMode::MainThread));
        assert_eq!(ReaderMode::parse("refuse"), Some(ReaderMode::Refuse));
        assert_eq!(ReaderMode::parse("on"), Some(ReaderMode::Auto));
        assert_eq!(ReaderMode::parse("bridge"), Some(ReaderMode::BridgeOnly));
        // Who goes to the worker: (game link, bridge).
        let to_worker = |m: ReaderMode| (m.worker_for(true).is_ok(), m.worker_for(false).is_ok());
        assert_eq!(to_worker(ReaderMode::Auto), (true, true));
        assert_eq!(to_worker(ReaderMode::BridgeOnly), (false, true));
        assert_eq!(to_worker(ReaderMode::MainThread), (false, false));
        assert_eq!(to_worker(ReaderMode::Refuse), (true, true));
        assert_eq!(ReaderMode::parse("maybe"), None);
        assert_eq!(Reader::Worker.as_str(), "worker");
        assert_eq!(Reader::MainThread.as_str(), "main thread");
    }
}
