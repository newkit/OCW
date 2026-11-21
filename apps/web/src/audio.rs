//! Game audio in the browser (spec §3.3, §14 Phase 10 B5): the desktop's
//! sounds – the shared `ocw-audio` mixer with its synthesised wind, tyres,
//! freehub, chain, the other riders, rain, hail, gusts, thunder, crashes,
//! the rub and the UI cues – fed by the same `ocw_client::audio::AudioFeed`
//! and played through Web Audio.
//!
//! Bevy's audio plugin stays off: it plays decoded sample files (rodio),
//! and every OCW sound is synthesised by our mixer. Web Audio has no
//! callback into wasm on the page's thread, so the mixer renders in the
//! frame loop: [`WebAudio::pump`] asks the [`Scheduler`] for the chunks due
//! (a lead of 0.1–0.3 s over the context's clock) and hands each to the
//! [`AudioOut`] as a buffer at its exact start time. The browser's
//! autoplay rule is the [`AudioGate`]: nothing plays before the first user
//! gesture (the page's listeners resume the context in the handler).
//!
//! [`AudioOut`] is the boundary: `crate::browser_audio` on wasm32 (an
//! `AudioContext`), a fake in the tests. Console as on the desktop:
//! `audio.volume`, `audio.mute`, `audio.status`, `audio.play`,
//! `audio.devices`, `audio.output`; the settings `audio.*` (the shared
//! registry, saved in `localStorage`: `crate::settings`).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use clap::{Arg, Command, value_parser};
use ocw_audio::cues::Cue;
use ocw_audio::mix::{Bus, Mix};
use ocw_audio::mixer::{Mixer, MixerMsg};
use ocw_audio::schedule::{CHUNK_FRAMES, Scheduler};
use ocw_audio::unlock::{AudioGate, ContextState};
use ocw_client::audio::{AudioFeed, FeedFrame, mix_line, on_off};
use ocw_render::RideView;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, ConsoleState, Permission};
use web_time::Instant;

use crate::plugin::Session;

/// The page's audio output (see the module docs).
pub trait AudioOut: Send + Sync {
    /// The context's state now.
    fn state(&self) -> ContextState;
    /// The context's clock (`currentTime`, s).
    fn current_time(&self) -> f64;
    fn sample_rate(&self) -> f64;
    /// Plays interleaved stereo samples from `at` on the context's clock.
    fn play(&mut self, at: f64, stereo: &[f32]);
    /// The output devices the browser named (empty until it may tell:
    /// labels need a permission, e.g. the microphone's).
    fn devices(&mut self) -> Vec<String> {
        Vec::new()
    }
    /// Plays on the named device (`None`: the default).
    fn set_output(&mut self, name: Option<&str>) -> Result<(), String> {
        match name {
            None => Ok(()),
            Some(_) => Err("this browser plays on its default output only".into()),
        }
    }
}

/// The gate as the page's gesture listeners and the app share it.
pub type SharedGate = Arc<Mutex<AudioGate>>;

/// No output (natively, before the page made a context).
pub struct NoOut;

impl AudioOut for NoOut {
    fn state(&self) -> ContextState {
        ContextState::Suspended
    }
    fn current_time(&self) -> f64 {
        0.0
    }
    fn sample_rate(&self) -> f64 {
        48_000.0
    }
    fn play(&mut self, _: f64, _: &[f32]) {}
}

/// What `window.ocw.audio()` and `audio.status` report.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AudioStats {
    pub state: &'static str,
    pub sample_rate: f64,
    pub lead_s: f64,
    pub chunks: u64,
    pub underruns: u32,
    pub scheduled_s: f64,
    /// Chunks with sound in them (RMS above −80 dBFS).
    pub audible_chunks: u64,
    /// The last second's loudest chunk (RMS, 0 … 1).
    pub level: f32,
    pub gestures: u32,
    pub resumes: u32,
}

/// The game audio of the browser app.
#[derive(Resource)]
pub struct WebAudio {
    out: Box<dyn AudioOut>,
    gate: SharedGate,
    /// In a mutex only to be `Sync` (its message channel is not); the
    /// systems reach it with `get_mut`, never locking.
    mixer: Mutex<Mixer>,
    scheduler: Scheduler,
    pub mix: Mix,
    /// The output setting (`audio.output`: a device name or `default`).
    pub output: String,
    feed: AudioFeed,
    /// One chunk, interleaved stereo.
    buf: Vec<f32>,
    /// The voice confirmation sounds (at the context's rate, mono).
    voice_cues: VecDeque<f32>,
    audible: u64,
    /// The loudest chunk lately and when it was measured (s, the clock).
    level: (f32, f64),
}

/// Frames of the chunk with sound in them: RMS above −80 dBFS.
const AUDIBLE_RMS: f32 = 1e-4;

impl WebAudio {
    pub fn new(out: Box<dyn AudioOut>, gate: SharedGate, mix: Mix, output: String) -> Self {
        let rate = out.sample_rate();
        Self {
            mixer: Mutex::new(Mixer::new(rate.round() as u32, mix.gains())),
            scheduler: Scheduler::new(rate),
            out,
            gate,
            mix,
            output,
            feed: AudioFeed::default(),
            buf: vec![0.0; CHUNK_FRAMES * 2],
            voice_cues: VecDeque::new(),
            audible: 0,
            level: (0.0, 0.0),
        }
    }

    pub fn gate(&self) -> &SharedGate {
        &self.gate
    }

    /// The mixer plays (the context runs).
    pub fn plays(&self) -> bool {
        self.gate.lock().expect("not poisoned").plays()
    }

    /// Hands the frame's messages to the mixer. Cues wait for nobody: while
    /// the sound is locked they are dropped (no burst of old beeps at the
    /// first click).
    pub fn apply(&mut self, msgs: Vec<MixerMsg>) {
        let plays = self.plays();
        for m in msgs {
            if matches!(
                m,
                MixerMsg::Cue(..) | MixerMsg::PlacedCue(..) | MixerMsg::SpatialCue(..)
            ) && !plays
            {
                continue;
            }
            self.mixer().apply(m);
        }
    }

    fn mixer(&mut self) -> &mut Mixer {
        self.mixer.get_mut().expect("not poisoned")
    }

    /// The feed's messages for this frame (the shared `AudioFeed`).
    pub fn feed(&mut self, frame: &FeedFrame) {
        let msgs = self.feed.frame(frame);
        self.apply(msgs);
    }

    pub fn play(&mut self, cue: Cue) {
        self.apply(vec![MixerMsg::Cue(cue, Instant::now())]);
    }

    /// The bus gains changed (`audio.volume`, `audio.mute`).
    pub fn send_gains(&mut self) {
        let gains = self.mix.gains();
        self.mixer().apply(MixerMsg::Gains(gains));
    }

    /// A voice confirmation sound (`ocw_voice::mix::cue_samples`: 48 kHz
    /// mono), played over the game sound like the desktop's voice output.
    pub fn voice_cue(&mut self, samples_48k: &[f32]) {
        if !self.plays() {
            return;
        }
        let rate = self.scheduler.sample_rate();
        let mut out = Vec::new();
        ocw_voice::mix::Resampler::new(48_000, rate.round() as u32).process(samples_48k, &mut out);
        // Overlapping cues add up.
        for (i, s) in out.into_iter().enumerate() {
            match self.voice_cues.get_mut(i) {
                Some(v) => *v += s,
                None => self.voice_cues.push_back(s),
            }
        }
    }

    /// Renders and schedules what is due (every frame).
    pub fn pump(&mut self) {
        let state = self.out.state();
        let plays = {
            let mut g = self.gate.lock().expect("not poisoned");
            g.on_state(state);
            g.plays()
        };
        if !plays {
            self.scheduler.reset();
            return;
        }
        let now = self.out.current_time();
        for at in self.scheduler.due(now) {
            self.mixer
                .get_mut()
                .expect("not poisoned")
                .render(&mut self.buf, 2);
            for frame in self.buf.chunks_mut(2) {
                let Some(c) = self.voice_cues.pop_front() else {
                    break;
                };
                frame[0] = ocw_audio::mixer::soft_limit(frame[0] + c);
                frame[1] = ocw_audio::mixer::soft_limit(frame[1] + c);
            }
            let rms = (self.buf.iter().map(|s| s * s).sum::<f32>() / self.buf.len() as f32).sqrt();
            if rms > AUDIBLE_RMS {
                self.audible += 1;
            }
            if rms >= self.level.0 || now - self.level.1 > 1.0 {
                self.level = (rms, now);
            }
            self.out.play(at, &self.buf);
        }
    }

    pub fn stats(&self) -> AudioStats {
        let g = self.gate.lock().expect("not poisoned");
        let s = self.scheduler.stats();
        let (gestures, resumes) = g.counts();
        AudioStats {
            state: g.word(),
            sample_rate: self.scheduler.sample_rate(),
            lead_s: self.scheduler.lead_s(),
            chunks: s.chunks,
            underruns: s.underruns,
            scheduled_s: s.scheduled_s,
            audible_chunks: self.audible,
            level: self.level.0,
            gestures,
            resumes,
        }
    }

    /// The page's hint while the sound waits for a gesture.
    pub fn hint(&self) -> Option<&'static str> {
        self.gate.lock().expect("not poisoned").hint()
    }

    /// `audio.status`'s first line.
    pub fn status_line(&self) -> String {
        let s = self.stats();
        let g = self.gate.lock().expect("not poisoned");
        let mut line = match g.state() {
            ocw_audio::unlock::Unlock::Unavailable(why) => format!("no audio output: {why}"),
            ocw_audio::unlock::Unlock::Locked | ocw_audio::unlock::Unlock::Resuming => {
                "waiting for a click or key press (the browser's autoplay rule)".to_string()
            }
            ocw_audio::unlock::Unlock::Closed => "the audio context was closed".to_string(),
            ocw_audio::unlock::Unlock::Running => format!(
                "playing on {} ({:.0} Hz, {:.0} ms ahead, {} underruns)",
                if self.output == "default" {
                    "the browser's output"
                } else {
                    &self.output
                },
                s.sample_rate,
                s.lead_s * 1000.0,
                s.underruns
            ),
        };
        if let Some(e) = g.last_error() {
            line.push_str(&format!("; last resume refused: {e}"));
        }
        line
    }

    /// `window.ocw.audio()` (JSON).
    pub fn json(&self) -> String {
        let s = self.stats();
        serde_json::json!({
            "state": s.state,
            "hint": self.hint(),
            "sample_rate": s.sample_rate,
            "lead_ms": (s.lead_s * 1000.0).round(),
            "chunks": s.chunks,
            "underruns": s.underruns,
            "scheduled_s": s.scheduled_s,
            "audible_chunks": s.audible_chunks,
            "level": s.level,
            "gestures": s.gestures,
            "resumes": s.resumes,
            "mix": {
                "master": self.mix.master, "effects": self.mix.effects, "others": self.mix.others,
                "music": self.mix.music, "voice": self.mix.voice, "mute": self.mix.mute,
            },
            "output": self.output,
        })
        .to_string()
    }

    pub fn devices(&mut self) -> Vec<String> {
        self.out.devices()
    }

    pub fn set_output(&mut self, name: &str) -> Result<String, String> {
        let choice = name.trim();
        let named =
            (!choice.is_empty() && !choice.eq_ignore_ascii_case("default")).then_some(choice);
        self.out.set_output(named)?;
        self.output = named.unwrap_or("default").to_string();
        Ok(format!("audio: output {}", self.output))
    }
}

/// Makes the output when the app starts (the page's `AudioContext` on
/// wasm32; a fake in the tests).
#[derive(Resource)]
pub struct MakeAudioOut(pub Box<dyn Fn() -> (Box<dyn AudioOut>, SharedGate) + Send + Sync>);

/// The voice confirmation sounds from the voice backend, until the audio
/// plays them.
pub static VOICE_CUES: Mutex<Vec<ocw_voice::mix::Cue>> = Mutex::new(Vec::new());

/// The last published state for `window.ocw.audio()`.
static JSON: Mutex<String> = Mutex::new(String::new());

pub fn audio_json() -> String {
    JSON.lock().expect("not poisoned").clone()
}

pub struct WebAudioPlugin;

impl Plugin for WebAudioPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PreStartup, start).add_systems(
            Update,
            (feed, pump, publish)
                .chain()
                .after(crate::plugin::ViewUpdated)
                .run_if(resource_exists::<WebAudio>),
        );
        register_commands(app);
    }
}

fn start(world: &mut World) {
    let (out, gate) = match world.get_resource::<MakeAudioOut>() {
        Some(m) => (m.0)(),
        None => (
            Box::new(NoOut) as Box<dyn AudioOut>,
            Arc::new(Mutex::new(AudioGate::new(Err(
                "no Web Audio here".to_string()
            )))),
        ),
    };
    let (mix, output) = world
        .get_resource::<crate::settings::WebSettings>()
        .map(|s| ocw_client::audio::settings_mix(&s.settings))
        .unwrap_or_else(|| (Mix::default(), "default".into()));
    world.insert_resource(WebAudio::new(out, gate, mix, output));
}

/// The ride into the mixer (the shared feed), and the voice cues.
fn feed(
    time: Res<Time>,
    session: Option<Res<Session>>,
    view: Option<Res<RideView>>,
    tunnels: Option<Res<ocw_render::tunnel::Tunnels>>,
    mut audio: ResMut<WebAudio>,
) {
    let s = session.as_deref().map(|s| &s.0);
    audio.feed(&FeedFrame {
        client: s.map(|s| &s.client),
        view: view.as_deref(),
        tunnels: tunnels.as_deref(),
        // The menus come with B6.
        menu: None,
        workout_laps: s.map(|s| s.devices.workout.lap_count()),
        dt_s: time.delta_secs_f64(),
    });
    let cues = std::mem::take(&mut *VOICE_CUES.lock().expect("not poisoned"));
    for c in cues {
        audio.voice_cue(&ocw_voice::mix::cue_samples(c));
    }
}

fn pump(mut audio: ResMut<WebAudio>) {
    audio.pump();
}

/// `window.ocw.audio()` (four times a second) and the console's hint once.
fn publish(
    time: Res<Time>,
    audio: Res<WebAudio>,
    mut console: ResMut<ConsoleState>,
    mut next: Local<f64>,
    mut hinted: Local<bool>,
) {
    if !*hinted && let Some(h) = audio.hint() {
        *hinted = true;
        console.print(h);
    }
    let now = time.elapsed_secs_f64();
    if now < *next {
        return;
    }
    *next = now + 0.25;
    *JSON.lock().expect("not poisoned") = audio.json();
}

fn audio_mut(world: &mut World) -> Result<Mut<'_, WebAudio>, String> {
    world
        .get_resource_mut::<WebAudio>()
        .ok_or_else(|| "audio is not running".to_string())
}

fn register_commands(app: &mut App) {
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("audio.volume")
            .about("Set a volume for this session (set audio.master … saves it)")
            .arg(
                Arg::new("bus")
                    .required(true)
                    .value_parser(["master", "music", "effects", "voice", "others"]),
            )
            .arg(
                Arg::new("percent")
                    .required(true)
                    .value_parser(value_parser!(u8).range(0..=100)),
            ),
        |world, m| {
            let bus = m.get_one::<String>("bus").expect("required").clone();
            let p = *m.get_one::<u8>("percent").expect("required");
            let mut a = audio_mut(world)?;
            match Bus::parse(&bus) {
                Some(b) => a.mix.set_volume(b, p),
                None => a.mix.master = p,
            }
            a.send_gains();
            Ok(format!("audio: {}", mix_line(&a.mix)))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("audio.mute")
            .about("Mute or unmute all sound (set audio.mute … saves it)")
            .arg(Arg::new("state").default_value("toggle")),
        |world, m| {
            let state = on_off(m.get_one::<String>("state").expect("default"))?;
            let mut a = audio_mut(world)?;
            a.mix.mute = state.unwrap_or(!a.mix.mute);
            a.send_gains();
            Ok(format!("audio: {}", if a.mix.mute { "muted" } else { "on" }))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("audio.output")
            .about("Play on this output device (a name from audio.devices, or default)")
            .arg(Arg::new("name").required(true).num_args(1..)),
        |world, m| {
            let name = m
                .get_many::<String>("name")
                .expect("required")
                .cloned()
                .collect::<Vec<_>>()
                .join(" ");
            audio_mut(world)?.set_output(&name)
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("audio.devices").about("List the output devices (* = playing now)"),
        |world, _| {
            let mut a = audio_mut(world)?;
            let current = a.output.clone();
            let mut out: Vec<String> = a
                .devices()
                .into_iter()
                .map(|d| format!("{} {d}", if d == current { '*' } else { ' ' }))
                .collect();
            if out.is_empty() {
                out.push(
                    "the browser names its outputs only with a permission (the microphone's); playing on its default".into(),
                );
            }
            out.push(format!("output setting: {current}"));
            Ok(out.join("\n"))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("audio.status").about("Show the output and the mix"),
        |world, _| {
            let a = audio_mut(world)?;
            let s = a.stats();
            Ok(format!(
                "audio: {}\nmix: {}\nscheduled {:.1} s in {} chunks ({} with sound)",
                a.status_line(),
                mix_line(&a.mix),
                s.scheduled_s,
                s.chunks,
                s.audible_chunks
            ))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("audio.play")
            .about("Play a sound: move, value, confirm, back, beep, go, finish, step, shift, test, test-music, test-effects, test-voice, test-others")
            .arg(Arg::new("sound").required(true)),
        |world, m| {
            let name = m.get_one::<String>("sound").expect("required");
            let cue = Cue::parse(name).ok_or_else(|| {
                let names: Vec<&str> = Cue::NAMES.iter().map(|(n, _)| *n).collect();
                format!("unknown sound {name:?}: {}", names.join(", "))
            })?;
            audio_mut(world)?.play(cue);
            Ok(String::new())
        },
    ));
}

#[cfg(test)]
pub mod fake {
    //! An `AudioContext` stand-in: a clock the test moves, a state, the
    //! buffers it was given.
    use super::*;

    #[derive(Default)]
    pub struct FakeState {
        pub state: Option<ContextState>,
        pub time: f64,
        pub played: Vec<(f64, Vec<f32>)>,
        pub sink: Option<String>,
    }

    #[derive(Clone, Default)]
    pub struct FakeOut(pub Arc<Mutex<FakeState>>);

    impl FakeOut {
        pub fn s(&self) -> std::sync::MutexGuard<'_, FakeState> {
            self.0.lock().unwrap()
        }
    }

    impl AudioOut for FakeOut {
        fn state(&self) -> ContextState {
            self.s().state.unwrap_or(ContextState::Suspended)
        }
        fn current_time(&self) -> f64 {
            self.s().time
        }
        fn sample_rate(&self) -> f64 {
            44_100.0
        }
        fn play(&mut self, at: f64, stereo: &[f32]) {
            self.s().played.push((at, stereo.to_vec()));
        }
        fn devices(&mut self) -> Vec<String> {
            vec!["Speakers".into(), "USB Headset".into()]
        }
        fn set_output(&mut self, name: Option<&str>) -> Result<(), String> {
            self.s().sink = name.map(str::to_string);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::FakeOut;
    use super::*;
    use ocw_audio::ride::{RideInput, ride_params};

    fn audio() -> (WebAudio, FakeOut) {
        let out = FakeOut::default();
        let gate = Arc::new(Mutex::new(AudioGate::new(Ok(ContextState::Suspended))));
        let full = Mix {
            master: 100,
            effects: 100,
            ..Mix::default()
        };
        (
            WebAudio::new(Box::new(out.clone()), gate, full, "default".into()),
            out,
        )
    }

    fn riding() -> MixerMsg {
        MixerMsg::Ride(ride_params(&RideInput {
            speed_mps: 10.0,
            cadence_rpm: Some(90.0),
            power_w: 250.0,
            surface: ocw_world::Surface::Asphalt,
        }))
    }

    /// Nothing is rendered or played before the first gesture; after it
    /// and the context running, the chunks come back to back at the
    /// context's rate, with the ride's sound in them.
    #[test]
    fn silent_until_the_gesture_then_plays_back_to_back() {
        let (mut a, out) = audio();
        a.apply(vec![riding(), MixerMsg::Cue(Cue::Finish, Instant::now())]);
        for i in 0..30 {
            out.s().time = f64::from(i) / 60.0;
            a.pump();
        }
        assert!(out.s().played.is_empty(), "locked");
        assert_eq!(a.stats().state, "locked");
        assert!(a.hint().is_some());
        // The page's listener: a click resumes the context.
        assert!(a.gate().lock().unwrap().on_gesture());
        assert_eq!(a.stats().state, "resuming");
        out.s().state = Some(ContextState::Running);
        for i in 30..150 {
            out.s().time = f64::from(i) / 60.0;
            a.pump();
        }
        let s = a.stats();
        assert_eq!(s.state, "running");
        assert_eq!(s.sample_rate, 44_100.0);
        assert_eq!(s.underruns, 0);
        let played = &out.s().played;
        assert!(played.len() > 80, "{} chunks in 2 s", played.len());
        for w in played.windows(2) {
            let dt = w[1].0 - w[0].0;
            assert!((dt - CHUNK_FRAMES as f64 / 44_100.0).abs() < 1e-9);
            assert_eq!(w[0].1.len(), CHUNK_FRAMES * 2, "stereo");
        }
        assert!(
            s.audible_chunks as usize > played.len() / 2,
            "the ride is heard"
        );
        assert!(s.level > 0.001);
        assert_eq!(a.hint(), None);
        let j: serde_json::Value = serde_json::from_str(&a.json()).unwrap();
        assert_eq!(j["state"], "running");
        assert_eq!(j["mix"]["master"], 100);
    }

    /// Cues asked for while locked are dropped (no burst at the first
    /// click); afterwards they play. Muting silences everything.
    #[test]
    fn cues_wait_for_nobody_and_mute_silences() {
        let (mut a, out) = audio();
        a.play(Cue::Finish);
        a.gate().lock().unwrap().on_gesture();
        out.s().state = Some(ContextState::Running);
        out.s().time = 1.0;
        a.pump();
        assert_eq!(a.stats().audible_chunks, 0, "the old cue was dropped");
        a.play(Cue::Test(Bus::Effects));
        out.s().time = 1.1;
        a.pump();
        assert!(a.stats().audible_chunks > 0);
        a.mix.mute = true;
        a.send_gains();
        let before = out.s().played.len();
        for i in 0..60 {
            out.s().time = 1.5 + f64::from(i) / 60.0;
            a.pump();
        }
        let played = &out.s().played;
        // The gains glide down (60 ms), then (nearly) silence.
        let tail = &played[before + 30..];
        assert!(!tail.is_empty());
        let peak = tail
            .iter()
            .flat_map(|(_, b)| b.iter())
            .fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak < 1e-3, "{peak}");
    }

    /// The voice cues (48 kHz) are resampled to the context's rate and
    /// mixed in; the output can be chosen by name.
    #[test]
    fn voice_cues_and_the_output() {
        let (mut a, out) = audio();
        a.gate().lock().unwrap().on_gesture();
        out.s().state = Some(ContextState::Running);
        a.pump();
        let cue = ocw_voice::mix::cue_samples(ocw_voice::mix::Cue::On);
        a.voice_cue(&cue);
        let expect = (cue.len() as f64 * 44_100.0 / 48_000.0) as usize;
        assert!(
            a.voice_cues.len().abs_diff(expect) <= 2,
            "{}",
            a.voice_cues.len()
        );
        out.s().time = 0.12;
        a.pump();
        assert!(a.stats().audible_chunks > 0);
        assert_eq!(
            a.set_output("USB Headset").unwrap(),
            "audio: output USB Headset"
        );
        assert_eq!(out.s().sink.as_deref(), Some("USB Headset"));
        assert_eq!(a.set_output("Default").unwrap(), "audio: output default");
        assert_eq!(out.s().sink, None);
        assert!(
            a.status_line()
                .starts_with("playing on the browser's output")
        );
        // The browser suspends the context (Safari: interrupted): locked
        // again, nothing scheduled until the next gesture.
        out.s().state = Some(ContextState::Interrupted);
        let n = out.s().played.len();
        out.s().time = 5.0;
        a.pump();
        assert_eq!(out.s().played.len(), n);
        assert_eq!(a.stats().state, "locked");
    }

    #[test]
    fn without_web_audio_it_says_so() {
        let gate = Arc::new(Mutex::new(AudioGate::new(Err("no AudioContext".into()))));
        let mut a = WebAudio::new(Box::new(NoOut), gate, Mix::default(), "default".into());
        a.pump();
        assert_eq!(a.status_line(), "no audio output: no AudioContext");
        assert!(NoOut.set_output_default_only());
    }

    impl NoOut {
        fn set_output_default_only(mut self) -> bool {
            self.set_output(None).is_ok() && self.set_output(Some("x")).is_err()
        }
    }
}
