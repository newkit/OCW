//! The page's Web Audio output (spec §14 Phase 10 B5; wasm32 only, kept
//! thin – the pacing, the autoplay gate and the mixing are pure and tested
//! natively in `ocw_audio::{schedule, unlock}` and `crate::audio`):
//!
//! - one `AudioContext` for the game sound, made at the start (suspended
//!   until a gesture, unless the browser allows autoplay);
//! - listeners for the first gestures (`pointerdown`, `keydown`,
//!   `touchend`, capture phase) that resume it **in the handler** – only
//!   there it counts in every browser (Safari);
//! - [`JsAudioOut`]: each chunk becomes an `AudioBuffer` played by an
//!   `AudioBufferSourceNode` started at its exact time on the context's
//!   clock;
//! - the outputs from `navigator.mediaDevices.enumerateDevices()` and the
//!   choice with `AudioContext.setSinkId` (Chromium 110+).

use std::cell::RefCell;
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use js_sys::{Function, Promise, Reflect};
use ocw_audio::unlock::{AudioGate, ContextState};
use wasm_bindgen::prelude::*;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::AudioContext;

use crate::audio::{AudioOut, NoOut, SharedGate};
use crate::browser::js;

thread_local! {
    static CONTEXT: RefCell<Option<AudioContext>> = const { RefCell::new(None) };
    /// The outputs as the browser listed them last: (label, device id).
    static OUTPUTS: RefCell<Vec<(String, String)>> = const { RefCell::new(Vec::new()) };
}

fn with_ctx<T>(f: impl FnOnce(&AudioContext) -> T) -> Option<T> {
    CONTEXT.with(|c| c.borrow().as_ref().map(f))
}

/// The context's `state` (a string: Safari also says "interrupted").
fn state_of(ctx: &AudioContext) -> ContextState {
    let s = Reflect::get(ctx, &JsValue::from_str("state"))
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_default();
    ContextState::parse(&s)
}

/// Makes the context and the gesture listeners (the app's start).
pub fn start() -> (Box<dyn AudioOut>, SharedGate) {
    let ctx = match AudioContext::new() {
        Ok(c) => c,
        Err(e) => {
            let why = format!("no Web Audio here ({})", js(e));
            warn!("audio: {why}");
            return (
                Box::new(NoOut),
                Arc::new(Mutex::new(AudioGate::new(Err(why)))),
            );
        }
    };
    let state = state_of(&ctx);
    info!(
        "audio: Web Audio at {} Hz, {:?} (the first click or key starts the sound)",
        ctx.sample_rate(),
        state
    );
    let gate: SharedGate = Arc::new(Mutex::new(AudioGate::new(Ok(state))));
    listen_for_gestures(&gate);
    CONTEXT.with(|c| *c.borrow_mut() = Some(ctx));
    refresh_outputs();
    (Box::new(JsAudioOut), gate)
}

/// Resumes the context in the handler of every gesture while it does not
/// run.
fn listen_for_gestures(gate: &SharedGate) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let gate = gate.clone();
    let handler = Closure::<dyn FnMut(web_sys::Event)>::new(move |_e: web_sys::Event| {
        let resume = gate.lock().expect("not poisoned").on_gesture();
        if !resume {
            return;
        }
        let Some(Ok(p)) = with_ctx(|c| c.resume()) else {
            return;
        };
        let gate = gate.clone();
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(e) = JsFuture::from(p).await {
                let e = js(e);
                warn!("audio: resume refused: {e}");
                gate.lock().expect("not poisoned").on_resume_failed(e);
            }
        });
    });
    for ev in ["pointerdown", "keydown", "touchend", "mousedown"] {
        let _ = window.add_event_listener_with_callback_and_bool(
            ev,
            handler.as_ref().unchecked_ref(),
            true,
        );
    }
    // The page lives as long as the app.
    handler.forget();
}

/// Reads the output devices again (labels only with a permission).
fn refresh_outputs() {
    let Some(md) = web_sys::window().and_then(|w| w.navigator().media_devices().ok()) else {
        return;
    };
    let Ok(p) = md.enumerate_devices() else {
        return;
    };
    wasm_bindgen_futures::spawn_local(async move {
        let Ok(list) = JsFuture::from(p).await else {
            return;
        };
        let list: js_sys::Array = list.unchecked_into();
        let mut out = Vec::new();
        for d in list.iter() {
            let get = |k: &str| {
                Reflect::get(&d, &JsValue::from_str(k))
                    .ok()
                    .and_then(|v| v.as_string())
                    .unwrap_or_default()
            };
            if get("kind") == "audiooutput" && !get("label").trim().is_empty() {
                out.push((get("label").trim().to_string(), get("deviceId")));
            }
        }
        OUTPUTS.with(|o| *o.borrow_mut() = out);
    });
}

/// The game sound's output in the page (the context lives in
/// [`CONTEXT`]; this handle is `Send`).
pub struct JsAudioOut;

impl AudioOut for JsAudioOut {
    fn state(&self) -> ContextState {
        with_ctx(state_of).unwrap_or(ContextState::Closed)
    }

    fn current_time(&self) -> f64 {
        with_ctx(|c| c.current_time()).unwrap_or(0.0)
    }

    fn sample_rate(&self) -> f64 {
        with_ctx(|c| f64::from(c.sample_rate())).unwrap_or(48_000.0)
    }

    fn play(&mut self, at: f64, stereo: &[f32]) {
        let frames = stereo.len() / 2;
        if frames == 0 {
            return;
        }
        let played = with_ctx(|c| -> Result<(), JsValue> {
            let buf = c.create_buffer(2, frames as u32, c.sample_rate())?;
            let mut ch = vec![0.0f32; frames];
            for side in 0..2 {
                for (i, s) in ch.iter_mut().enumerate() {
                    *s = stereo[2 * i + side];
                }
                buf.copy_to_channel(&ch, side as i32)?;
            }
            let src = c.create_buffer_source()?;
            src.set_buffer(Some(&buf));
            src.connect_with_audio_node(&c.destination())?;
            src.start_with_when(at)?;
            Ok(())
        });
        if let Some(Err(e)) = played {
            debug!("audio: a chunk was not played: {}", js(e));
        }
    }

    fn devices(&mut self) -> Vec<String> {
        refresh_outputs();
        OUTPUTS.with(|o| o.borrow().iter().map(|(l, _)| l.clone()).collect())
    }

    fn set_output(&mut self, name: Option<&str>) -> Result<(), String> {
        let id = match name {
            None => String::new(),
            Some(n) => OUTPUTS
                .with(|o| {
                    o.borrow()
                        .iter()
                        .find(|(l, _)| l == n || l.eq_ignore_ascii_case(n.trim()))
                        .map(|(_, id)| id.clone())
                })
                .ok_or_else(|| {
                    format!("no output {n:?} (audio.devices lists them; names need the microphone's permission)")
                })?,
        };
        with_ctx(|c| -> Result<(), String> {
            let f = Reflect::get(c, &JsValue::from_str("setSinkId"))
                .ok()
                .and_then(|f| f.dyn_into::<Function>().ok())
                .ok_or(
                    "this browser plays on its default output only (no AudioContext.setSinkId)",
                )?;
            let p: Promise = f
                .call1(c, &JsValue::from_str(&id))
                .map_err(js)?
                .unchecked_into();
            wasm_bindgen_futures::spawn_local(async move {
                if let Err(e) = JsFuture::from(p).await {
                    warn!("audio: the output was not changed: {}", js(e));
                }
            });
            Ok(())
        })
        .unwrap_or(Err("no audio context".into()))
    }
}
