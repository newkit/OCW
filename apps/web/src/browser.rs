//! The browser's side of worlds over HTTP (spec §14 Phase 10 B2; wasm32
//! only, kept thin – the logic is in `ocw_worldstore::stream` and
//! `crate::world`, unit-tested natively):
//!
//! - [`BrowserFetch`]: `fetch()` with a `Range` header;
//! - [`OpfsStore`]: the chunk cache's files in the Origin Private File
//!   System (`navigator.storage.getDirectory()`, folder `ocw-worlds`);
//! - the storage estimate and persistence request (`navigator.storage`);
//! - the driver task: [`StreamDriver::step`] in a `spawn_local` loop,
//!   sleeping while idle;
//! - the loading screen: the page's `#ocw-status` shows the progress lines.

use std::ops::Range;
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use js_sys::{Promise, Reflect, Uint8Array};
use ocw_worldstore::stream::driver::{StreamDriver, StreamShared};
use ocw_worldstore::stream::progress::Stage;
use ocw_worldstore::stream::{ChunkStore, HttpResponse, RangeFetch};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    FileSystemDirectoryHandle, FileSystemFileHandle, FileSystemGetDirectoryOptions,
    FileSystemGetFileOptions, FileSystemWritableFileStream, Headers, Request, RequestInit,
    Response,
};

use crate::world::{WorldOptions, WorldStream};

/// The OPFS folder of the chunk cache.
pub const OPFS_DIR: &str = "ocw-worlds";

/// A JS error as text.
pub(crate) fn js(e: JsValue) -> String {
    if let Some(err) = e.dyn_ref::<js_sys::Error>() {
        return String::from(err.message());
    }
    if let Some(d) = e.dyn_ref::<web_sys::DomException>() {
        return format!("{}: {}", d.name(), d.message());
    }
    e.as_string().unwrap_or_else(|| format!("{e:?}"))
}

fn not_found(e: &JsValue) -> bool {
    e.dyn_ref::<web_sys::DomException>()
        .is_some_and(|d| d.name() == "NotFoundError")
}

/// The wasm module's memory in bytes.
pub fn memory_bytes() -> u64 {
    wasm_bindgen::memory()
        .dyn_into::<js_sys::WebAssembly::Memory>()
        .ok()
        .and_then(|m| m.buffer().dyn_into::<js_sys::ArrayBuffer>().ok())
        .map_or(0, |b| u64::from(b.byte_length()))
}

/// Milliseconds since the page started.
pub(crate) fn now_ms() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map_or(0.0, |p| p.now())
}

pub(crate) async fn sleep(ms: i32) {
    let p = Promise::new(&mut |resolve, _| {
        if let Some(w) = web_sys::window() {
            let _ = w.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        }
    });
    let _ = JsFuture::from(p).await;
}

/// `fetch()` with `Range: bytes=a-b`.
pub struct BrowserFetch;

impl RangeFetch for BrowserFetch {
    async fn get(&self, url: &str, range: Option<Range<u64>>) -> Result<HttpResponse, String> {
        let window = web_sys::window().ok_or("no window")?;
        let init = RequestInit::new();
        init.set_method("GET");
        let headers = Headers::new().map_err(js)?;
        if let Some(r) = &range {
            headers
                .set(
                    "Range",
                    &format!("bytes={}-{}", r.start, r.end.saturating_sub(1)),
                )
                .map_err(js)?;
        }
        init.set_headers(&headers);
        let req = Request::new_with_str_and_init(url, &init).map_err(js)?;
        let resp: Response = JsFuture::from(window.fetch_with_request(&req))
            .await
            .map_err(|e| format!("{url}: {}", js(e)))?
            .dyn_into()
            .map_err(js)?;
        let status = resp.status();
        let buf = JsFuture::from(resp.array_buffer().map_err(js)?)
            .await
            .map_err(|e| format!("{url}: {}", js(e)))?;
        Ok(HttpResponse {
            status,
            body: Uint8Array::new(&buf).to_vec(),
        })
    }
}

/// The chunk files in the Origin Private File System.
pub struct OpfsStore {
    dir: FileSystemDirectoryHandle,
}

impl OpfsStore {
    pub async fn open() -> Result<Self, String> {
        let window = web_sys::window().ok_or("no window")?;
        let root: FileSystemDirectoryHandle =
            JsFuture::from(window.navigator().storage().get_directory())
                .await
                .map_err(js)?
                .dyn_into()
                .map_err(js)?;
        let opts = FileSystemGetDirectoryOptions::new();
        opts.set_create(true);
        let dir = JsFuture::from(root.get_directory_handle_with_options(OPFS_DIR, &opts))
            .await
            .map_err(js)?
            .dyn_into()
            .map_err(js)?;
        Ok(Self { dir })
    }
}

impl ChunkStore for OpfsStore {
    async fn read(&self, name: &str) -> Result<Option<Vec<u8>>, String> {
        let handle = match JsFuture::from(self.dir.get_file_handle(name)).await {
            Ok(h) => h,
            Err(e) if not_found(&e) => return Ok(None),
            Err(e) => return Err(js(e)),
        };
        let handle: FileSystemFileHandle = handle.dyn_into().map_err(js)?;
        let file: web_sys::File = JsFuture::from(handle.get_file())
            .await
            .map_err(js)?
            .dyn_into()
            .map_err(js)?;
        let buf = JsFuture::from(file.array_buffer()).await.map_err(js)?;
        Ok(Some(Uint8Array::new(&buf).to_vec()))
    }

    async fn write(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        let opts = FileSystemGetFileOptions::new();
        opts.set_create(true);
        let handle: FileSystemFileHandle =
            JsFuture::from(self.dir.get_file_handle_with_options(name, &opts))
                .await
                .map_err(js)?
                .dyn_into()
                .map_err(js)?;
        let w: FileSystemWritableFileStream = JsFuture::from(handle.create_writable())
            .await
            .map_err(js)?
            .dyn_into()
            .map_err(js)?;
        // A copy owned by JS: the stream may read it after wasm memory grew.
        let data = Uint8Array::from(bytes);
        JsFuture::from(w.write_with_js_u8_array(&data).map_err(js)?)
            .await
            .map_err(js)?;
        JsFuture::from(w.close()).await.map_err(js)?;
        Ok(())
    }

    async fn remove(&self, name: &str) -> Result<(), String> {
        match JsFuture::from(self.dir.remove_entry(name)).await {
            Ok(_) => Ok(()),
            Err(e) if not_found(&e) => Ok(()),
            Err(e) => Err(js(e)),
        }
    }
}

/// The browser's quota for the origin and whether its storage persists
/// (asks for persistence first).
async fn storage_estimate(ask: bool) -> (Option<u64>, Option<bool>) {
    let Some(window) = web_sys::window() else {
        return (None, None);
    };
    let storage = window.navigator().storage();
    if ask && let Ok(p) = storage.persist() {
        let _ = JsFuture::from(p).await;
    }
    let quota = match storage.estimate() {
        Ok(p) => JsFuture::from(p)
            .await
            .ok()
            .and_then(|e| Reflect::get(&e, &"quota".into()).ok())
            .and_then(|q| q.as_f64())
            .map(|q| q as u64),
        Err(_) => None,
    };
    let persisted = match storage.persisted() {
        Ok(p) => JsFuture::from(p).await.ok().and_then(|v| v.as_bool()),
        Err(_) => None,
    };
    (quota, persisted)
}

/// Starts the world's driver task (with `?world=`) and the loading screen.
pub fn start_world(app: &mut App, world: Option<WorldOptions>) {
    app.add_systems(Update, loading_screen);
    let Some(opts) = world else { return };
    let Some(shared) = app
        .world()
        .get_resource::<WorldStream>()
        .map(|s| s.shared.clone())
    else {
        return;
    };
    let now_unix_s = (js_sys::Date::now() / 1000.0) as i64;
    let config = match opts.stream_config(now_unix_s) {
        Ok(c) => c,
        Err(e) => {
            let mut s = shared.lock().expect("not poisoned");
            s.progress.stage = Stage::Failed;
            s.progress.error = Some(e);
            return;
        }
    };
    wasm_bindgen_futures::spawn_local(drive(shared, config, opts.use_cache));
}

async fn drive(
    shared: Arc<Mutex<StreamShared>>,
    config: ocw_worldstore::stream::driver::StreamConfig,
    use_cache: bool,
) {
    let store = if use_cache {
        match OpfsStore::open().await {
            Ok(s) => Some(s),
            Err(e) => {
                warn!("no OPFS ({e}): the world is not cached");
                None
            }
        }
    } else {
        None
    };
    let (quota, persisted) = storage_estimate(store.is_some()).await;
    {
        let mut s = shared.lock().expect("not poisoned");
        s.quota = quota;
        s.persisted = persisted;
    }
    let mut d = StreamDriver::new(BrowserFetch, store, config, Box::new(now_ms));
    let mut idle = 0u32;
    loop {
        if d.step(&shared).await {
            idle = 0;
            continue;
        }
        if d.failed() {
            break;
        }
        idle += 1;
        // The storage line now and then (other tabs use the quota too).
        if idle % 200 == 0 {
            let (quota, persisted) = storage_estimate(false).await;
            let mut s = shared.lock().expect("not poisoned");
            s.quota = quota;
            s.persisted = persisted;
        }
        sleep(25).await;
    }
}

/// The page's status element shows the loading screen until the ride
/// starts (and the error if loading failed).
fn loading_screen(stream: Option<Res<WorldStream>>, mut shown: Local<Option<String>>) {
    let text = match &stream {
        Some(s) if !s.started => s.screen.join("\n"),
        _ => String::new(),
    };
    if shown.as_deref() == Some(text.as_str()) {
        return;
    }
    let Some(el) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id("ocw-status"))
    else {
        return;
    };
    if text.is_empty() {
        let _ = el.set_attribute("hidden", "");
    } else {
        let _ = el.remove_attribute("hidden");
        el.set_text_content(Some(&text));
        let failed = stream
            .as_ref()
            .is_some_and(|s| s.failed_to_load().is_some());
        let _ = el.set_attribute("class", if failed { "error" } else { "loading" });
    }
    *shown = Some(text);
}
