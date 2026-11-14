//! The echo test end to end without a sound card or voice server (spec §7
//! *Echo bot*, §11.4): the controller drives the loopback backend (the
//! same echo loop and mixer the LiveKit backend runs between the real
//! devices) while a synthetic microphone signal goes in and the voice mix
//! comes out, in 10 ms steps like the audio devices.
//!
//! `OCW_ECHO_DEMO=/path/echo.wav cargo test -p ocw-voice --test echo_loopback`
//! also writes a demo (48 kHz 16-bit stereo): a synthetic voice riding past
//! the Echo rider – far, approaching (the echo fades in), beside it (full,
//! panned right), leaving (fading out).

use std::sync::{Arc, Mutex};

use ocw_core::RiderId;
use ocw_voice::echo::{DEFAULT_DELAY_MS, ECHO_IDENTITY, LoopbackBackend, delay_samples};
use ocw_voice::mix::{FRAME_SAMPLES, VOICE_RATE, pan_gains};
use ocw_voice::spatial::{Peer, SpatialConfig};
use ocw_voice::{EchoStatus, MuteState, Switch, VoiceController, VoiceSettings};

const ECHO: RiderId = RiderId(9);
const SR: f64 = VOICE_RATE as f64;

struct Rig {
    c: VoiceController,
    b: Arc<Mutex<LoopbackBackend>>,
    /// Samples pushed so far.
    t: usize,
    /// The output (interleaved stereo).
    out: Vec<f32>,
}

impl Rig {
    fn new() -> Self {
        let b = Arc::new(Mutex::new(LoopbackBackend::new()));
        let mut c = VoiceController::new(
            Box::new(b.clone()),
            VoiceSettings::default(),
            MuteState::default(),
        );
        // A solo ride on the local server: no voice server, no call.
        c.new_session(false);
        c.set_echo(true);
        c.set_echo_rider(Some(ECHO));
        Self {
            c,
            b,
            t: 0,
            out: Vec::new(),
        }
    }

    /// One 10 ms step: the controller sees the Echo rider at `gap` m
    /// (`lateral` m to the side), the microphone gets `mic(t)`.
    fn step(&mut self, gap: f64, lateral: f64, mic: &dyn Fn(usize) -> f32) {
        let peers = [Peer {
            rider: ECHO,
            gap_m: gap,
            lateral_m: lateral,
        }];
        self.c.update(self.t as f64 / SR, &peers);
        let block: Vec<f32> = (self.t..self.t + FRAME_SAMPLES).map(mic).collect();
        let mut b = self.b.lock().unwrap();
        b.push_microphone(&block);
        self.out.extend(b.pull_output(FRAME_SAMPLES));
        self.t += FRAME_SAMPLES;
    }

    fn run(&mut self, seconds: f64, gap: f64, lateral: f64, mic: &dyn Fn(usize) -> f32) {
        for _ in 0..(seconds * 100.0).round() as usize {
            self.step(gap, lateral, mic);
        }
    }

    fn left(&self) -> Vec<f32> {
        self.out.iter().step_by(2).copied().collect()
    }
    fn right(&self) -> Vec<f32> {
        self.out.iter().skip(1).step_by(2).copied().collect()
    }
}

fn sine(f: f64, amp: f64) -> impl Fn(usize) -> f32 {
    move |i| (amp * (std::f64::consts::TAU * f * i as f64 / SR).sin()) as f32
}

/// A tone burst from `a` to `b` seconds.
fn burst(a: f64, b: f64) -> impl Fn(usize) -> f32 {
    let s = sine(440.0, 0.5);
    move |i| {
        let t = i as f64 / SR;
        if (a..b).contains(&t) { s(i) } else { 0.0 }
    }
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

fn first_loud(x: &[f32]) -> Option<usize> {
    x.iter().position(|s| s.abs() > 1e-3)
}

#[test]
fn near_the_echo_rider_the_tone_comes_back_delayed_and_placed() {
    let mut r = Rig::new();
    let mic = burst(0.5, 0.7);
    // Beside the Echo rider: 5 m ahead, 2 m to the right.
    r.run(1.5, 5.0, 2.0, &mic);
    assert!(matches!(r.c.echo_status(), EchoStatus::Near { recording, .. } if recording == 1.0));
    let (left, right) = (r.left(), r.right());
    let onset = first_loud(&right).expect("the echo is heard");
    let input_onset = first_loud(&(0..r.t).map(&mic).collect::<Vec<_>>()).unwrap();
    // Sample-exact: the default 300 ms after the input.
    assert_eq!(onset - input_onset, delay_samples(DEFAULT_DELAY_MS));
    assert_eq!(DEFAULT_DELAY_MS, 300);
    // Level and pan: the spatial placement of a rider at that spot.
    let pl = SpatialConfig::default().place(&Peer {
        rider: ECHO,
        gap_m: 5.0,
        lateral_m: 2.0,
    });
    assert!(pl.pan > 0.1, "to the right: {}", pl.pan);
    let (gl, gr) = pan_gains(pl.pan);
    let span = |x: &[f32]| rms(&x[onset + 960..onset + 8_000]);
    let want = 0.5 / 2f32.sqrt() * pl.gain;
    assert!(
        (span(&right) - want * gr).abs() < 0.01,
        "{} vs {}",
        span(&right),
        want * gr
    );
    assert!((span(&left) - want * gl).abs() < 0.01);
    assert!(span(&right) > span(&left));
    // The HUD's meters saw the microphone and the echo.
    let levels = r.c.levels();
    assert!(levels.input < 0.05, "the burst ended: {levels:?}");
}

#[test]
fn the_echo_fades_with_distance_and_stops_far_away() {
    let level_at = |gap: f64| {
        let mut r = Rig::new();
        r.run(1.0, gap, 0.0, &sine(300.0, 0.5));
        let l = r.left();
        rms(&l[l.len() - 24_000..])
    };
    let near = level_at(8.0);
    let mid = level_at(40.0);
    let far = level_at(100.0);
    // 40 m: two thirds recorded (fading from 30 to 60 m), played at
    // 10 m / 40 m.
    let c = SpatialConfig::default();
    let expect = |gap: f64, rec: f32| {
        let (l, _) = pan_gains(0.0);
        0.5 / 2f32.sqrt() * rec * c.gain(gap) * l
    };
    assert!((near - expect(8.0, 1.0)).abs() < 0.005, "{near}");
    assert!((mid - expect(40.0, 2.0 / 3.0)).abs() < 0.005, "{mid}");
    assert!(far < 1e-6, "{far}");
    let mut r = Rig::new();
    r.run(0.2, 100.0, 0.0, &sine(300.0, 0.5));
    assert!(matches!(r.c.echo_status(), EchoStatus::Far { .. }));
}

#[test]
fn mute_switches_silence_the_echo() {
    // Microphone muted: nothing is echoed.
    let mut r = Rig::new();
    r.c.set_mic(Switch::On);
    r.run(1.0, 5.0, 0.0, &sine(300.0, 0.5));
    let l = r.left();
    // The mute's confirmation cue plays at the start; after it, silence.
    assert!(rms(&l[24_000..]) < 1e-6);
    // Speaker muted: the voices – the echo too – are silent.
    let mut r = Rig::new();
    r.c.set_speaker(Switch::On);
    r.run(1.0, 5.0, 0.0, &sine(300.0, 0.5));
    let l = r.left();
    assert!(rms(&l[24_000..]) < 1e-6);
    // Unmuted again: the echo comes back after the delay.
    r.c.set_speaker(Switch::Off);
    r.run(1.0, 5.0, 0.0, &sine(300.0, 0.5));
    let l = r.left();
    assert!(rms(&l[l.len() - 24_000..]) > 0.2);
    // The echo test turned off: silence, the loop is idle.
    r.c.set_echo(false);
    r.run(0.5, 5.0, 0.0, &sine(300.0, 0.5));
    let l = r.left();
    assert!(rms(&l[l.len() - 12_000..]) < 1e-6);
    assert!(!r.b.lock().unwrap().echo().active());
    assert_eq!(r.c.echo_status(), EchoStatus::Off);
}

#[test]
fn no_drift_and_no_growing_buffers_over_minutes() {
    let mut r = Rig::new();
    let cap = r.b.lock().unwrap().echo().line().capacity();
    let mic = sine(250.0, 0.3);
    for _ in 0..3 * 60 * 100 {
        r.step(5.0, 0.0, &mic);
        let b = r.b.lock().unwrap();
        assert!(b.mixer().buffered(ECHO_IDENTITY) <= FRAME_SAMPLES);
        drop(b);
        if r.out.len() > 4 * FRAME_SAMPLES {
            r.out.drain(..r.out.len() - 4 * FRAME_SAMPLES);
        }
    }
    let b = r.b.lock().unwrap();
    assert_eq!(b.echo().line().capacity(), cap);
    assert_eq!(b.mixer().dropped, 0);
    // Still exactly 300 ms behind after 3 minutes: the last output sample
    // is the input of 14 400 samples before.
    let last = r.out[r.out.len() - 2];
    let want = mic(r.t - 1 - delay_samples(DEFAULT_DELAY_MS));
    assert!(
        (last - want * pan_gains(0.0).0).abs() < 1e-3,
        "{last} vs {want}"
    );
}

#[test]
fn the_delay_can_be_changed() {
    let mut r = Rig::new();
    assert_eq!(r.c.set_echo_delay(5), 20);
    assert_eq!(r.c.set_echo_delay(99_999), 3000);
    assert_eq!(r.c.set_echo_delay(1000), 1000);
    let mic = burst(0.1, 0.3);
    r.run(1.5, 3.0, 0.0, &mic);
    let onset = first_loud(&r.left()).unwrap();
    let input_onset = first_loud(&(0..r.t).map(&mic).collect::<Vec<_>>()).unwrap();
    assert_eq!(onset - input_onset, 48_000);
}

/// A synthetic voice: a buzzing pitch (110–150 Hz) through two formants,
/// in syllables of about 0.2 s, with pauses between phrases.
fn voice(i: usize) -> f32 {
    let t = i as f64 / SR;
    let phrase = t % 4.0;
    if phrase > 2.6 {
        return 0.0;
    }
    let syllable = (std::f64::consts::PI * phrase * 4.5).sin().abs().powf(0.7);
    let f0 = 130.0 + 20.0 * (t * 1.3).sin();
    let vowel = (t * 4.5).floor() as usize % 3;
    let (f1, f2) = [(700.0, 1200.0), (400.0, 2000.0), (300.0, 900.0)][vowel];
    let mut s = 0.0;
    for k in 1..30 {
        let f = f0 * f64::from(k);
        let w = 1.0 / (1.0 + ((f - f1) / 120.0).powi(2)) + 0.5 / (1.0 + ((f - f2) / 180.0).powi(2));
        s += w * (std::f64::consts::TAU * f * t).sin() / f64::from(k).sqrt();
    }
    (0.25 * syllable * s) as f32
}

fn wav(samples: &[f32]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut b = Vec::with_capacity(44 + data_len as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&VOICE_RATE.to_le_bytes());
    b.extend_from_slice(&(VOICE_RATE * 4).to_le_bytes());
    b.extend_from_slice(&4u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        b.extend_from_slice(&v.to_le_bytes());
    }
    b
}

/// The demo ride: 24 s at 6 m/s past the Echo rider (it stands 1.5 m to
/// the right of the lane), from 80 m before it to 64 m past it.
#[test]
fn demo_ride_past_the_echo_rider() {
    let mut r = Rig::new();
    let mut dry = Vec::new();
    let steps = 24 * 100;
    for k in 0..steps {
        let t = k as f64 / 100.0;
        let gap = 80.0 - 6.0 * t;
        r.step(gap, 1.5, &voice);
        dry.extend((r.t - FRAME_SAMPLES..r.t).map(voice));
    }
    let (left, right) = (r.left(), r.right());
    let sec = |x: &[f32], a: f64, b: f64| rms(&x[(a * SR) as usize..(b * SR) as usize]);
    // Far (80 – 62 m, 0–3 s): silent; beside it (8–16 s, ±20 m): loud;
    // far behind (> 60 m, 23 – 24 s): silent again.
    assert!(sec(&left, 0.5, 3.0) < 1e-6);
    let beside = sec(&left, 9.0, 15.0) + sec(&right, 9.0, 15.0);
    assert!(beside > 0.03, "{beside}");
    let leaving = sec(&left, 18.0, 21.0) + sec(&right, 18.0, 21.0);
    assert!(
        leaving < 0.5 * beside && leaving > 0.0,
        "fading out: {leaving}"
    );
    assert!(sec(&left, 23.0, 24.0) < 1e-6);
    // Passing on the right: right louder while beside.
    assert!(sec(&right, 12.0, 14.0) > sec(&left, 12.0, 14.0));
    if let Ok(path) = std::env::var("OCW_ECHO_DEMO") {
        // The rider's own voice quietly in the centre (as heard in the
        // room), the echo as the app plays it.
        let mix: Vec<f32> = r
            .out
            .iter()
            .enumerate()
            .map(|(i, s)| s + 0.35 * dry[i / 2])
            .collect();
        std::fs::write(&path, wav(&mix)).expect("write the demo WAV");
        eprintln!("wrote {path}");
    }
}
