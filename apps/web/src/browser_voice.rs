//! The page's voice shim as the voice backend (spec §14 Phase 10 B5;
//! wasm32 only, kept thin): [`JsShim`] calls `window.ocwVoice`
//! (`web/ocw-voice.js`, which wraps LiveKit's JS SDK `livekit-client`); the
//! adapter, the protocol and the controller are pure and tested natively
//! (`ocw_voice::web`, `ocw_voice::controller`). The confirmation sounds
//! play through the game audio (`crate::audio::VOICE_CUES`).

use js_sys::{Function, Reflect};
use ocw_voice::VoiceBackend;
use ocw_voice::web::{VoiceShim, WebVoiceBackend};
use wasm_bindgen::{JsCast, JsValue};

/// `window.ocwVoice`, if the page has it.
fn shim() -> Option<JsValue> {
    let w = web_sys::window()?;
    Reflect::get(&w, &JsValue::from_str("ocwVoice"))
        .ok()
        .filter(|v| v.is_object())
}

/// Calls `ocwVoice.<name>(args…)`; its result.
fn call(name: &str, args: &[JsValue]) -> Option<JsValue> {
    let s = shim()?;
    let f: Function = Reflect::get(&s, &JsValue::from_str(name))
        .ok()?
        .dyn_into()
        .ok()?;
    let a = js_sys::Array::new();
    for v in args {
        a.push(v);
    }
    f.apply(&s, &a).ok()
}

fn text(name: &str) -> String {
    call(name, &[])
        .and_then(|v| v.as_string())
        .unwrap_or_default()
}

/// The page's shim (a handle: the JS lives in the page).
pub struct JsShim;

impl VoiceShim for JsShim {
    fn available(&self) -> Result<(), String> {
        if shim().is_none() {
            return Err("the page has no voice shim (ocw-voice.js)".into());
        }
        match text("available") {
            why if why.is_empty() => Ok(()),
            why => Err(why),
        }
    }

    fn command(&mut self, json: &str) {
        call("command", &[JsValue::from_str(json)]);
    }

    fn poll(&mut self) -> String {
        text("poll")
    }

    fn devices(&mut self) -> String {
        text("devices")
    }

    fn levels(&self) -> String {
        text("levels")
    }
}

/// The browser's voice backend.
pub fn backend() -> Box<dyn VoiceBackend> {
    Box::new(WebVoiceBackend::new(
        JsShim,
        Box::new(|c| {
            crate::audio::VOICE_CUES
                .lock()
                .expect("not poisoned")
                .push(c)
        }),
    ))
}
