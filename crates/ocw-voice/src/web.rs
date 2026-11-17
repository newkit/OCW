//! The browser's voice backend (spec §7, §14 Phase 10 B5): LiveKit's own
//! JS SDK (`livekit-client`, Apache-2.0) does the WebRTC, the page's thin
//! shim (`apps/web/web/ocw-voice.js`, `window.ocwVoice`) wraps it, and this
//! adapter turns the controller's [`BackendCommand`]s into the shim's JSON
//! commands and the shim's JSON events into [`BackendEvent`]s – so the
//! browser runs the **same** [`crate::VoiceController`] as the desktop:
//! the same rooms and tokens (over the game connection), the join policy,
//! retries, the mute switches, nearest-N and spatial placement, the echo
//! test.
//!
//! The boundary [`VoiceShim`] is four calls (a command, the events since
//! the last poll, the device lists, the levels); the browser side
//! implements it with `wasm-bindgen` calls on `window.ocwVoice`, the tests
//! with a fake that behaves like the shim. Pure, builds for wasm.
//!
//! The confirmation sounds ([`crate::mix::Cue`]) do not go to the shim:
//! the app plays them through its own Web Audio output (the `cue` sink).
//!
//! The shim's protocol (JSON):
//!
//! - commands: `{"op":"connect","url":…,"token":…}`, `{"op":"disconnect"}`,
//!   `{"op":"mic_muted","on":bool}`, `{"op":"speaker_muted","on":bool}`,
//!   `{"op":"input","device":name|null}`, `{"op":"output","device":name|null}`,
//!   `{"op":"subscribe","identity":…,"on":bool}`,
//!   `{"op":"place","identity":…,"gain":f,"pan":f}`,
//!   `{"op":"echo","active":bool,"delay_ms":n,"record_gain":f}`,
//!   `{"op":"subscribers","allowed":[identities]|null}` (who may subscribe
//!   to the own microphone: LiveKit's track subscription permissions; null
//!   = everybody; spec §7.1.7 Q9);
//! - events (an array): `{"type":"connected","room":…}`, `reconnecting`,
//!   `reconnected`, `{"type":"disconnected","reason":…}`,
//!   `{"type":"connect_failed","error":…}`,
//!   `{"type":"joined","identity":…,"name":…}`, `{"type":"left","identity":…}`,
//!   `{"type":"muted","identity":…,"muted":bool}`,
//!   `{"type":"speaking","identities":[…]}`, `{"type":"device_error","error":…}`;
//! - devices: `{"inputs":[names],"outputs":[names]}`; levels:
//!   `{"input":f,"echo":f}`.

use serde_json::{Value, json};

use crate::backend::{BackendCommand, BackendEvent, Levels, VoiceBackend};
use crate::mix::Cue;

/// What the adapter needs from the page (see the module docs).
pub trait VoiceShim: Send {
    /// `Err(why)` when the page cannot do voice (no shim, no
    /// livekit-client, no WebRTC, an insecure page without microphone
    /// access).
    fn available(&self) -> Result<(), String>;
    /// Hands the shim a command (JSON).
    fn command(&mut self, json: &str);
    /// The events since the last call (a JSON array; empty: none).
    fn poll(&mut self) -> String;
    /// The device lists (JSON); the shim refreshes them by itself.
    fn devices(&mut self) -> String;
    /// The levels (JSON).
    fn levels(&self) -> String;
}

/// A command as the shim's JSON; `None` for the cue (played by the app).
pub fn encode(cmd: &BackendCommand) -> Option<String> {
    let v = match cmd {
        BackendCommand::Connect { url, token } => {
            json!({"op": "connect", "url": url, "token": token})
        }
        BackendCommand::Disconnect => json!({"op": "disconnect"}),
        BackendCommand::SetMicMuted(on) => json!({"op": "mic_muted", "on": on}),
        BackendCommand::SetSpeakerMuted(on) => json!({"op": "speaker_muted", "on": on}),
        BackendCommand::SetInputDevice(d) => json!({"op": "input", "device": d}),
        BackendCommand::SetOutputDevice(d) => json!({"op": "output", "device": d}),
        BackendCommand::Subscribe { identity, on } => {
            json!({"op": "subscribe", "identity": identity, "on": on})
        }
        BackendCommand::SetPlacement {
            identity,
            gain,
            pan,
        } => json!({"op": "place", "identity": identity, "gain": gain, "pan": pan}),
        BackendCommand::SetEcho(p) => json!({
            "op": "echo",
            "active": p.active,
            "delay_ms": p.delay_ms,
            "record_gain": p.record_gain,
        }),
        BackendCommand::SetSubscribers(allowed) => {
            json!({"op": "subscribers", "allowed": allowed})
        }
        BackendCommand::PlayCue(_) => return None,
    };
    Some(v.to_string())
}

fn text(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// The shim's events (a JSON array); unknown or broken entries are
/// skipped (returned as the second list, for the log).
pub fn decode(json: &str) -> (Vec<BackendEvent>, Vec<String>) {
    let mut events = Vec::new();
    let mut bad = Vec::new();
    if json.trim().is_empty() {
        return (events, bad);
    }
    let list = match serde_json::from_str::<Value>(json) {
        Ok(Value::Array(a)) => a,
        Ok(other) => {
            bad.push(format!("not a list: {other}"));
            return (events, bad);
        }
        Err(e) => {
            bad.push(format!("{e}"));
            return (events, bad);
        }
    };
    for v in list {
        let identity = || text(&v, "identity");
        let e = match v.get("type").and_then(Value::as_str) {
            Some("connected") => BackendEvent::Connected {
                room: text(&v, "room"),
            },
            Some("reconnecting") => BackendEvent::Reconnecting,
            Some("reconnected") => BackendEvent::Reconnected,
            Some("disconnected") => BackendEvent::Disconnected {
                reason: text(&v, "reason"),
            },
            Some("connect_failed") => BackendEvent::ConnectFailed(text(&v, "error")),
            Some("joined") if !identity().is_empty() => BackendEvent::ParticipantJoined {
                identity: identity(),
                name: text(&v, "name"),
            },
            Some("left") if !identity().is_empty() => BackendEvent::ParticipantLeft {
                identity: identity(),
            },
            Some("muted") if !identity().is_empty() => BackendEvent::ParticipantMuted {
                identity: identity(),
                muted: v.get("muted").and_then(Value::as_bool).unwrap_or(false),
            },
            Some("speaking") => BackendEvent::Speaking(
                v.get("identities")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|i| i.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default(),
            ),
            Some("device_error") => BackendEvent::DeviceError(text(&v, "error")),
            _ => {
                bad.push(v.to_string());
                continue;
            }
        };
        events.push(e);
    }
    (events, bad)
}

/// The device lists from the shim's JSON (inputs, outputs).
pub fn decode_devices(json: &str) -> (Vec<String>, Vec<String>) {
    let v: Value = serde_json::from_str(json).unwrap_or(Value::Null);
    let list = |k: &str| -> Vec<String> {
        let mut out: Vec<String> = v
            .get(k)
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|n| n.as_str().map(|s| s.trim().to_string()))
                    .filter(|n| !n.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        // Names are how devices are chosen: one of each.
        let mut seen = std::collections::BTreeSet::new();
        out.retain(|n| seen.insert(n.clone()));
        out
    };
    (list("inputs"), list("outputs"))
}

/// The levels from the shim's JSON (0 … 1; missing = 0).
pub fn decode_levels(json: &str) -> Levels {
    let v: Value = serde_json::from_str(json).unwrap_or(Value::Null);
    let get = |k: &str| {
        v.get(k)
            .and_then(Value::as_f64)
            .map_or(0.0, |x| (x as f32).clamp(0.0, 1.0))
    };
    Levels {
        input: get("input"),
        echo: get("echo"),
    }
}

/// Where the confirmation sounds go (the app's audio output).
pub type CueSink = Box<dyn FnMut(Cue) + Send>;

/// The [`VoiceBackend`] of the browser app (see the module docs).
pub struct WebVoiceBackend<S: VoiceShim> {
    shim: S,
    cues: CueSink,
    /// Shim events that could not be read (for the log).
    bad: Vec<String>,
}

impl<S: VoiceShim> WebVoiceBackend<S> {
    pub fn new(shim: S, cues: CueSink) -> Self {
        Self {
            shim,
            cues,
            bad: Vec::new(),
        }
    }

    pub fn shim(&self) -> &S {
        &self.shim
    }

    pub fn shim_mut(&mut self) -> &mut S {
        &mut self.shim
    }

    /// Shim events that could not be read since the last call.
    pub fn take_bad(&mut self) -> Vec<String> {
        std::mem::take(&mut self.bad)
    }
}

impl<S: VoiceShim> VoiceBackend for WebVoiceBackend<S> {
    fn available(&self) -> Result<(), String> {
        self.shim.available()
    }

    fn input_devices(&mut self) -> Vec<String> {
        decode_devices(&self.shim.devices()).0
    }

    fn output_devices(&mut self) -> Vec<String> {
        decode_devices(&self.shim.devices()).1
    }

    fn send(&mut self, cmd: BackendCommand) {
        match encode(&cmd) {
            Some(json) => self.shim.command(&json),
            None => {
                if let BackendCommand::PlayCue(c) = cmd {
                    (self.cues)(c);
                }
            }
        }
    }

    fn poll(&mut self) -> Vec<BackendEvent> {
        let (events, bad) = decode(&self.shim.poll());
        if !bad.is_empty() {
            tracing::warn!("voice: unreadable shim events: {}", bad.join("; "));
            self.bad.extend(bad);
        }
        events
    }

    fn levels(&self) -> Levels {
        decode_levels(&self.shim.levels())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::controller::{Status, VoiceController, VoiceSettings};
    use crate::echo::EchoParams;
    use crate::mute::{MuteState, Switch};
    use crate::spatial::Peer;
    use ocw_core::RiderId;
    use ocw_protocol::v1;

    /// A fake of the page's shim: it acts like `ocw-voice.js` over a fake
    /// LiveKit server – connecting with a token succeeds and reports the
    /// others already in the room, a broken token fails, a "network drop"
    /// reconnects – and records what it was told.
    #[derive(Default)]
    struct Page {
        commands: Vec<Value>,
        events: Vec<Value>,
        connected: bool,
        mic_muted: Option<bool>,
        speaker_muted: Option<bool>,
        subscribed: Vec<String>,
        placed: Vec<(String, f64, f64)>,
        echo: Option<Value>,
        inputs: Vec<String>,
        unavailable: Option<String>,
        others: Vec<(String, String)>,
    }

    #[derive(Clone, Default)]
    struct Fake(Arc<Mutex<Page>>);

    impl Fake {
        fn page(&self) -> std::sync::MutexGuard<'_, Page> {
            self.0.lock().unwrap()
        }
    }

    impl VoiceShim for Fake {
        fn available(&self) -> Result<(), String> {
            match &self.page().unavailable {
                Some(e) => Err(e.clone()),
                None => Ok(()),
            }
        }
        fn command(&mut self, json: &str) {
            let v: Value = serde_json::from_str(json).expect("the adapter writes JSON");
            let mut p = self.page();
            match v["op"].as_str().unwrap() {
                "connect" => {
                    if v["token"] == "broken" {
                        p.events
                            .push(json!({"type": "connect_failed", "error": "401 invalid token"}));
                    } else {
                        p.connected = true;
                        p.events
                            .push(json!({"type": "connected", "room": "ocw-test"}));
                        let others = p.others.clone();
                        for (identity, name) in others {
                            p.events.push(
                                json!({"type": "joined", "identity": identity, "name": name}),
                            );
                        }
                    }
                }
                "disconnect" => {
                    p.connected = false;
                    p.events.push(json!({"type": "disconnected", "reason": ""}));
                }
                "mic_muted" => p.mic_muted = v["on"].as_bool(),
                "speaker_muted" => p.speaker_muted = v["on"].as_bool(),
                "subscribe" => {
                    let id = v["identity"].as_str().unwrap().to_string();
                    if v["on"].as_bool().unwrap() {
                        p.subscribed.push(id);
                    } else {
                        p.subscribed.retain(|s| *s != id);
                    }
                }
                "place" => {
                    let id = v["identity"].as_str().unwrap().to_string();
                    p.placed
                        .push((id, v["gain"].as_f64().unwrap(), v["pan"].as_f64().unwrap()));
                }
                "echo" => p.echo = Some(v.clone()),
                _ => {}
            }
            p.commands.push(v);
        }
        fn poll(&mut self) -> String {
            Value::Array(std::mem::take(&mut self.page().events)).to_string()
        }
        fn devices(&mut self) -> String {
            json!({"inputs": self.page().inputs, "outputs": ["Speakers", "Speakers", " "]})
                .to_string()
        }
        fn levels(&self) -> String {
            json!({"input": 0.25, "echo": 7.0}).to_string()
        }
    }

    fn backend(fake: &Fake, cues: &Arc<Mutex<Vec<Cue>>>) -> Box<WebVoiceBackend<Fake>> {
        let sink = cues.clone();
        Box::new(WebVoiceBackend::new(
            fake.clone(),
            Box::new(move |c| sink.lock().unwrap().push(c)),
        ))
    }

    fn grant(token: &str) -> v1::VoiceGrant {
        v1::VoiceGrant {
            url: "wss://voice.example".into(),
            token: token.into(),
            room: "ocw-test".into(),
            identity: "rider-1".into(),
            ..Default::default()
        }
    }

    #[test]
    fn commands_and_events_round_trip() {
        let c = encode(&BackendCommand::SetPlacement {
            identity: "rider-2".into(),
            gain: 0.5,
            pan: -0.25,
        })
        .unwrap();
        let v: Value = serde_json::from_str(&c).unwrap();
        assert_eq!(v["op"], "place");
        assert_eq!(v["gain"], 0.5);
        assert_eq!(
            encode(&BackendCommand::SetInputDevice(None)).unwrap(),
            r#"{"device":null,"op":"input"}"#
        );
        assert_eq!(encode(&BackendCommand::PlayCue(Cue::On)), None);
        assert_eq!(
            encode(&BackendCommand::SetSubscribers(Some(vec![
                "rider-3".into()
            ])))
            .unwrap(),
            r#"{"allowed":["rider-3"],"op":"subscribers"}"#
        );
        assert_eq!(
            encode(&BackendCommand::SetSubscribers(None)).unwrap(),
            r#"{"allowed":null,"op":"subscribers"}"#
        );
        let e = encode(&BackendCommand::SetEcho(EchoParams {
            active: true,
            delay_ms: 300,
            record_gain: 1.0,
        }))
        .unwrap();
        assert!(e.contains(r#""delay_ms":300"#), "{e}");
        let (events, bad) = decode(
            r#"[{"type":"connected","room":"r"},{"type":"joined","identity":"rider-2","name":"Ada"},
                {"type":"muted","identity":"rider-2","muted":true},{"type":"speaking","identities":["rider-2",3]},
                {"type":"reconnecting"},{"type":"reconnected"},{"type":"left","identity":"rider-2"},
                {"type":"device_error","error":"NotAllowedError"},{"type":"disconnected","reason":"server shutdown"},
                {"type":"joined"},{"type":"teleported"}]"#,
        );
        assert_eq!(
            events,
            [
                BackendEvent::Connected { room: "r".into() },
                BackendEvent::ParticipantJoined {
                    identity: "rider-2".into(),
                    name: "Ada".into()
                },
                BackendEvent::ParticipantMuted {
                    identity: "rider-2".into(),
                    muted: true
                },
                BackendEvent::Speaking(vec!["rider-2".into()]),
                BackendEvent::Reconnecting,
                BackendEvent::Reconnected,
                BackendEvent::ParticipantLeft {
                    identity: "rider-2".into()
                },
                BackendEvent::DeviceError("NotAllowedError".into()),
                BackendEvent::Disconnected {
                    reason: "server shutdown".into()
                },
            ]
        );
        assert_eq!(bad.len(), 2, "a join without identity, an unknown type");
        assert_eq!(decode(""), (vec![], vec![]));
        assert_eq!(decode("{}").1.len(), 1);
        assert_eq!(decode("[oops").1.len(), 1);
        assert_eq!(
            decode_devices(r#"{"inputs":["Mic"," Mic ",""],"outputs":[]}"#),
            (vec!["Mic".to_string()], vec![])
        );
        assert_eq!(
            decode_levels(r#"{"input":0.5,"echo":-1}"#),
            Levels {
                input: 0.5,
                echo: 0.0
            }
        );
        assert_eq!(decode_levels("garbage"), Levels::default());
    }

    /// The whole call through the shared controller: join with a token,
    /// the other rider is subscribed and placed by distance, the mute
    /// switches reach the shim and play their cue through the app, a drop
    /// reconnects, leaving disconnects.
    #[test]
    fn the_controller_runs_a_call_through_the_shim() {
        let fake = Fake::default();
        fake.page().others = vec![("rider-2".into(), "Ada".into())];
        fake.page().inputs = vec!["Built-in microphone".into()];
        let cues = Arc::new(Mutex::new(Vec::new()));
        let mut c = VoiceController::new(
            backend(&fake, &cues),
            VoiceSettings::default(),
            MuteState::default(),
        );
        assert_eq!(fake.page().mic_muted, Some(false), "told at the start");
        assert_eq!(fake.page().speaker_muted, Some(false));
        c.new_session(true);
        c.set_human_riders(2);
        let req = c.update(0.0, &[]).expect("a token request");
        assert_eq!(req.channel, v1::VoiceChannel::Session as i32);
        c.on_grant(&grant("good"), 0.1);
        assert_eq!(c.status(), Status::Connecting);
        let peer = Peer {
            rider: RiderId(2),
            gap_m: 10.0,
            lateral_m: 1.0,
        };
        c.update(0.2, &[peer]);
        assert_eq!(c.status(), Status::Connected);
        assert_eq!(c.room(), Some("ocw-test"));
        assert_eq!(fake.page().subscribed, ["rider-2"]);
        let (id, gain, pan) = fake.page().placed.last().cloned().unwrap();
        assert_eq!(id, "rider-2");
        assert!(gain > 0.0 && gain <= 1.0 && pan > 0.0, "{gain} {pan}");
        assert!(cues.lock().unwrap().contains(&crate::mix::Cue::Joined));
        // M: the microphone muted on the shim, the cue through the app.
        c.set_mic(Switch::Toggle);
        assert_eq!(fake.page().mic_muted, Some(true));
        assert_eq!(cues.lock().unwrap().last(), Some(&Cue::Muted));
        c.set_speaker(Switch::On);
        assert_eq!(fake.page().speaker_muted, Some(true));
        // The network drops: LiveKit reconnects by itself.
        fake.page().events.extend([json!({"type": "reconnecting"})]);
        c.update(1.0, &[peer]);
        assert_eq!(c.status(), Status::Reconnecting);
        fake.page().events.push(json!({"type": "reconnected"}));
        c.update(1.1, &[peer]);
        assert_eq!(c.status(), Status::Connected);
        // Devices and levels come from the shim.
        assert_eq!(c.input_devices(), ["Built-in microphone"]);
        assert_eq!(c.output_devices(), ["Speakers"]);
        assert_eq!(c.levels().input, 0.25);
        assert_eq!(c.levels().echo, 1.0, "clamped");
        // Voice turned off: the call is left.
        c.set_settings(VoiceSettings {
            enabled: false,
            ..Default::default()
        });
        c.update(2.0, &[peer]);
        assert!(!fake.page().connected);
        assert_eq!(c.status(), Status::Off);
    }

    /// A refused token: the shim's failure is retried with the back-off; a
    /// dropped call (not reconnected) is retried too.
    #[test]
    fn failures_are_retried() {
        let fake = Fake::default();
        let cues = Arc::new(Mutex::new(Vec::new()));
        let mut c = VoiceController::new(
            backend(&fake, &cues),
            VoiceSettings::default(),
            MuteState::default(),
        );
        c.new_session(true);
        c.set_human_riders(2);
        assert!(c.update(0.0, &[]).is_some());
        c.on_grant(&grant("broken"), 0.1);
        c.update(0.2, &[]);
        assert!(matches!(c.status(), Status::Failed(e) if e.contains("401")));
        assert!(c.update(1.0, &[]).is_none(), "waits 2 s");
        assert!(c.update(2.3, &[]).is_some(), "asks again");
        c.on_grant(&grant("good"), 2.4);
        c.update(2.5, &[]);
        assert_eq!(c.status(), Status::Connected);
        fake.page()
            .events
            .push(json!({"type": "disconnected", "reason": "signal lost"}));
        c.update(3.0, &[]);
        assert!(matches!(c.status(), Status::Failed(e) if e == "signal lost"));
        let msgs = c.take_messages();
        assert!(msgs.iter().any(|m| m.contains("signal lost")), "{msgs:?}");
    }

    /// The echo test on a solo ride: the loopback runs in the shim (the
    /// browser's microphone through a delay), placed like a voice.
    #[test]
    fn the_echo_test_drives_the_shims_loopback() {
        let fake = Fake::default();
        let cues = Arc::new(Mutex::new(Vec::new()));
        let mut c = VoiceController::new(
            backend(&fake, &cues),
            VoiceSettings::default(),
            MuteState::default(),
        );
        c.set_echo(true);
        c.set_echo_rider(Some(RiderId(9)));
        let near = Peer {
            rider: RiderId(9),
            gap_m: 15.0,
            lateral_m: 0.0,
        };
        c.update(0.0, &[near]);
        let echo = fake.page().echo.clone().expect("told");
        assert_eq!(echo["active"], true);
        assert_eq!(echo["delay_ms"], 300);
        assert_eq!(echo["record_gain"], 1.0);
        assert!(
            fake.page()
                .placed
                .iter()
                .any(|(i, _, _)| i == crate::echo::ECHO_IDENTITY),
            "placed as the echo voice"
        );
        c.set_echo(false);
        c.update(0.1, &[near]);
        assert_eq!(fake.page().echo.clone().unwrap()["active"], false);
    }

    #[test]
    fn an_unavailable_page_says_why() {
        let fake = Fake::default();
        fake.page().unavailable = Some("livekit-client is not loaded".into());
        let cues = Arc::new(Mutex::new(Vec::new()));
        let mut c = VoiceController::new(
            backend(&fake, &cues),
            VoiceSettings::default(),
            MuteState::default(),
        );
        c.new_session(true);
        c.set_human_riders(3);
        assert!(c.update(0.0, &[]).is_none(), "no token request");
        assert_eq!(
            c.status(),
            Status::Unavailable("livekit-client is not loaded".into())
        );
        // The switches still work (and are remembered by the app), but no
        // cue plays without voice.
        c.set_mic(Switch::On);
        assert!(c.mute().mic_muted);
        assert!(cues.lock().unwrap().is_empty());
    }
}
