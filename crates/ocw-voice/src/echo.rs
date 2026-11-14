//! The echo test (spec §7 *Echo bot*): an "Echo" rider stands on the road;
//! near it, everything the own microphone sends is played back from the
//! Echo rider's place after a short delay, so a rider can check the
//! microphone, the voice processing, the volume, the spatial placement and
//! the mute switches alone.
//!
//! Pure (no threads, clock or devices; builds for wasm):
//! - [`DelayLine`]: a fixed ring buffer, sample-exact, never grows;
//! - [`EchoConfig`]: the delay (default [`DEFAULT_DELAY_MS`]) and how near
//!   is near ([`EchoConfig::record_gain`]);
//! - [`EchoLoop`]: the loopback of one microphone stream: processed
//!   microphone blocks in, the delayed echo out (played as the voice
//!   [`ECHO_IDENTITY`] by the [`crate::mix::Mixer`]);
//! - [`LoopbackBackend`]: a [`VoiceBackend`] with only that path (no voice
//!   server, no devices): the offline tests and the demo WAV drive the
//!   controller through it exactly as the LiveKit backend's local loopback
//!   does with real devices.

use std::sync::{Arc, Mutex};

use crate::backend::{BackendCommand, BackendEvent, Levels, VoiceBackend};
use crate::mix::{FRAME_SAMPLES, Meter, Mixer, VOICE_RATE, to_i16};

/// The echo's delay unless set otherwise: long enough to hear the echo
/// apart from the own voice (a "couple of ms" would sound like a comb
/// filter, not an echo), short enough for a quick check.
pub const DEFAULT_DELAY_MS: u32 = 300;
pub const MIN_DELAY_MS: u32 = 20;
pub const MAX_DELAY_MS: u32 = 3000;
/// The mixer voice of the local loopback.
pub const ECHO_IDENTITY: &str = "echo";
/// The Echo rider's display name.
pub const ECHO_NAME: &str = "Echo";

/// Samples of `ms` milliseconds at [`VOICE_RATE`].
pub fn delay_samples(ms: u32) -> usize {
    (u64::from(ms) * u64::from(VOICE_RATE) / 1000) as usize
}

/// Clamps a delay to [`MIN_DELAY_MS`] … [`MAX_DELAY_MS`].
pub fn clamp_delay(ms: u32) -> u32 {
    ms.clamp(MIN_DELAY_MS, MAX_DELAY_MS)
}

/// A fixed delay: every sample comes out exactly `len` samples after it
/// went in, as many out as in. The buffer is allocated once.
#[derive(Debug, Clone)]
pub struct DelayLine {
    buf: Vec<f32>,
    pos: usize,
}

impl DelayLine {
    /// A delay of `samples`, starting silent.
    pub fn new(samples: usize) -> Self {
        Self {
            buf: vec![0.0; samples],
            pos: 0,
        }
    }

    pub fn delay(&self) -> usize {
        self.buf.len()
    }

    /// The buffer's allocated size (samples): never more than the delay.
    pub fn capacity(&self) -> usize {
        self.buf.capacity()
    }

    pub fn clear(&mut self) {
        self.buf.fill(0.0);
    }

    /// Delays `input`, appending as many samples to `out`.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if self.buf.is_empty() {
            out.extend_from_slice(input);
            return;
        }
        out.reserve(input.len());
        for &x in input {
            out.push(std::mem::replace(&mut self.buf[self.pos], x));
            self.pos += 1;
            if self.pos == self.buf.len() {
                self.pos = 0;
            }
        }
    }
}

/// When the Echo rider listens and how long it waits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EchoConfig {
    pub delay_ms: u32,
    /// Everything said this near is echoed in full …
    pub near_m: f64,
    /// … fading to nothing at this distance (beyond it nothing is
    /// recorded; what was recorded still plays out, quieter, from the
    /// Echo rider's place as the rider rides on).
    pub fade_m: f64,
    /// The microphone is opened for the echo only within this distance
    /// (the voice range, spec §7): no open device for an Echo rider far
    /// away.
    pub open_m: f64,
}

impl Default for EchoConfig {
    fn default() -> Self {
        Self {
            delay_ms: DEFAULT_DELAY_MS,
            near_m: 30.0,
            fade_m: 60.0,
            open_m: 300.0,
        }
    }
}

impl EchoConfig {
    /// How much of the microphone the Echo rider records at `distance_m`:
    /// 1 up to `near_m`, linearly to 0 at `fade_m`.
    pub fn record_gain(&self, distance_m: f64) -> f32 {
        let d = distance_m.abs();
        if !d.is_finite() || d >= self.fade_m {
            0.0
        } else if d <= self.near_m {
            1.0
        } else {
            ((self.fade_m - d) / (self.fade_m - self.near_m)) as f32
        }
    }
}

/// What the backend's loopback is told (changes only).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EchoParams {
    /// Run the loopback (opens the microphone if no call does).
    pub active: bool,
    pub delay_ms: u32,
    /// [`EchoConfig::record_gain`] at the Echo rider's distance.
    pub record_gain: f32,
}

impl Default for EchoParams {
    fn default() -> Self {
        Self {
            active: false,
            delay_ms: DEFAULT_DELAY_MS,
            record_gain: 0.0,
        }
    }
}

/// The loopback of the microphone stream: processed microphone blocks
/// (48 kHz mono, after noise suppression / gain control / echo
/// cancellation) in, the echo out after the delay. Input and output are
/// always the same length, so nothing drifts or piles up; the mixer plays
/// the echo like a received voice (its gain and pan, the speaker mute).
#[derive(Debug, Clone)]
pub struct EchoLoop {
    params: EchoParams,
    line: DelayLine,
    /// The record gain at the end of the last block (ramped, no clicks).
    gain: f32,
    input: Meter,
    scratch_in: Vec<f32>,
    scratch_out: Vec<f32>,
}

impl Default for EchoLoop {
    fn default() -> Self {
        Self::new()
    }
}

impl EchoLoop {
    pub fn new() -> Self {
        let params = EchoParams::default();
        Self {
            line: DelayLine::new(delay_samples(params.delay_ms)),
            params,
            gain: 0.0,
            input: Meter::default(),
            scratch_in: Vec::new(),
            scratch_out: Vec::new(),
        }
    }

    pub fn params(&self) -> EchoParams {
        self.params
    }

    pub fn active(&self) -> bool {
        self.params.active
    }

    pub fn set(&mut self, p: EchoParams) {
        let p = EchoParams {
            delay_ms: clamp_delay(p.delay_ms),
            record_gain: p.record_gain.clamp(0.0, 1.0),
            ..p
        };
        if p.delay_ms != self.params.delay_ms {
            self.line = DelayLine::new(delay_samples(p.delay_ms));
        }
        if !p.active {
            // Off: nothing old plays when it comes back.
            self.line.clear();
            self.gain = 0.0;
            self.input = Meter::default();
        }
        self.params = p;
    }

    /// The delay line (tests: its size never grows).
    pub fn line(&self) -> &DelayLine {
        &self.line
    }

    /// One block of the processed microphone; `mic_live`: the microphone
    /// is not muted (muted, silence goes in, so nothing said while muted
    /// is ever echoed). The delayed echo to play, `None` while inactive.
    pub fn capture(&mut self, block: &[i16], mic_live: bool) -> Option<Vec<i16>> {
        if !self.params.active {
            return None;
        }
        self.scratch_in.clear();
        self.scratch_in.extend(block.iter().map(|&s| {
            if mic_live {
                f32::from(s) / 32768.0
            } else {
                0.0
            }
        }));
        self.input.update(&self.scratch_in);
        // Ramp the record gain over the block.
        let (from, to) = (self.gain, self.params.record_gain);
        let n = self.scratch_in.len().max(1) as f32;
        for (i, s) in self.scratch_in.iter_mut().enumerate() {
            *s *= from + (to - from) * (i + 1) as f32 / n;
        }
        self.gain = to;
        self.scratch_out.clear();
        self.line.process(&self.scratch_in, &mut self.scratch_out);
        Some(self.scratch_out.iter().map(|&s| to_i16(s)).collect())
    }

    /// The microphone's level (0 … 1, smoothed): the HUD's input meter.
    pub fn input_level(&self) -> f32 {
        if self.params.active {
            self.input.level()
        } else {
            0.0
        }
    }
}

/// A voice backend with only the local echo path: no voice server, no
/// devices. Microphone samples go in with [`Self::push_microphone`], the
/// voice mix comes out of [`Self::pull_output`] – the same [`EchoLoop`] and
/// [`Mixer`] the LiveKit backend runs between the real devices, without
/// libwebrtc's processing (which only the `livekit` feature has).
#[derive(Debug, Default)]
pub struct LoopbackBackend {
    mixer: Mixer,
    echo: EchoLoop,
    mic_muted: bool,
    pending: Vec<i16>,
    events: Vec<BackendEvent>,
}

impl LoopbackBackend {
    pub fn new() -> Self {
        Self::default()
    }

    /// Microphone samples (48 kHz mono, −1 … 1), processed in 10 ms frames.
    pub fn push_microphone(&mut self, samples: &[f32]) {
        self.pending.extend(samples.iter().map(|&s| to_i16(s)));
        let whole = self.pending.len() / FRAME_SAMPLES * FRAME_SAMPLES;
        for frame in self.pending[..whole].as_chunks::<FRAME_SAMPLES>().0 {
            if let Some(out) = self.echo.capture(frame, !self.mic_muted) {
                self.mixer.push(ECHO_IDENTITY, &out);
            }
        }
        self.pending.drain(..whole);
    }

    /// The next `frames` of the voice mix (48 kHz, interleaved stereo).
    pub fn pull_output(&mut self, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0; frames * 2];
        self.mixer.pull_stereo(&mut out, None);
        out
    }

    pub fn echo(&self) -> &EchoLoop {
        &self.echo
    }

    pub fn mixer(&self) -> &Mixer {
        &self.mixer
    }
}

impl VoiceBackend for LoopbackBackend {
    fn available(&self) -> Result<(), String> {
        Ok(())
    }
    fn input_devices(&mut self) -> Vec<String> {
        Vec::new()
    }
    fn output_devices(&mut self) -> Vec<String> {
        Vec::new()
    }
    fn send(&mut self, cmd: BackendCommand) {
        match cmd {
            BackendCommand::Connect { .. } => self.events.push(BackendEvent::ConnectFailed(
                "the loopback backend has no voice server".into(),
            )),
            BackendCommand::Disconnect => {}
            BackendCommand::SetMicMuted(m) => self.mic_muted = m,
            BackendCommand::SetSpeakerMuted(m) => self.mixer.muted = m,
            BackendCommand::SetInputDevice(_) | BackendCommand::SetOutputDevice(_) => {}
            BackendCommand::Subscribe { identity, on } => {
                if !on {
                    self.mixer.remove(&identity);
                }
            }
            BackendCommand::SetPlacement {
                identity,
                gain,
                pan,
            } => self.mixer.set_placement(&identity, gain, pan),
            BackendCommand::PlayCue(c) => self.mixer.play_cue(c),
            BackendCommand::SetEcho(p) => {
                self.echo.set(p);
                if !p.active {
                    self.mixer.flush(ECHO_IDENTITY);
                }
            }
        }
    }
    fn poll(&mut self) -> Vec<BackendEvent> {
        std::mem::take(&mut self.events)
    }
    fn levels(&self) -> Levels {
        Levels {
            input: self.echo.input_level(),
            echo: self.mixer.level(ECHO_IDENTITY),
        }
    }
}

/// A loopback shared with the test that feeds it (the controller owns its
/// backend).
impl VoiceBackend for Arc<Mutex<LoopbackBackend>> {
    fn available(&self) -> Result<(), String> {
        Ok(())
    }
    fn input_devices(&mut self) -> Vec<String> {
        Vec::new()
    }
    fn output_devices(&mut self) -> Vec<String> {
        Vec::new()
    }
    fn send(&mut self, cmd: BackendCommand) {
        if let Ok(mut b) = self.lock() {
            b.send(cmd);
        }
    }
    fn poll(&mut self) -> Vec<BackendEvent> {
        self.lock().map(|mut b| b.poll()).unwrap_or_default()
    }
    fn levels(&self) -> Levels {
        self.lock().map(|b| b.levels()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_is_sample_exact_and_bounded() {
        let d = delay_samples(DEFAULT_DELAY_MS);
        assert_eq!(d, 14_400, "300 ms at 48 kHz");
        let mut line = DelayLine::new(d);
        let cap = line.capacity();
        // An impulse train in odd-sized blocks for 5 minutes: every impulse
        // comes out exactly `d` samples later.
        let total = 5 * 60 * VOICE_RATE as usize;
        let impulse = |i: usize| if i.is_multiple_of(7_919) { 1.0 } else { 0.0 };
        let mut out = Vec::new();
        let mut i = 0;
        let mut block = 0;
        while i < total {
            let n = [441, 480, 1, 1_024, 4_799][block % 5].min(total - i);
            let input: Vec<f32> = (i..i + n).map(impulse).collect();
            out.clear();
            line.process(&input, &mut out);
            assert_eq!(out.len(), n, "as many out as in");
            for (k, &y) in out.iter().enumerate() {
                let t = i + k;
                let want = if t >= d { impulse(t - d) } else { 0.0 };
                assert_eq!(y, want, "sample {t}");
            }
            i += n;
            block += 1;
        }
        assert_eq!(line.capacity(), cap, "the buffer never grew");
        assert_eq!(line.delay(), d);
        // A zero delay passes through.
        let mut z = DelayLine::new(0);
        let mut o = Vec::new();
        z.process(&[0.5, -0.5], &mut o);
        assert_eq!(o, vec![0.5, -0.5]);
    }

    #[test]
    fn delays_are_clamped() {
        assert_eq!(clamp_delay(0), MIN_DELAY_MS);
        assert_eq!(clamp_delay(10_000), MAX_DELAY_MS);
        assert_eq!(clamp_delay(450), 450);
        assert_eq!(delay_samples(20), 960);
        assert_eq!(delay_samples(3000), 144_000);
        let mut e = EchoLoop::new();
        e.set(EchoParams {
            active: true,
            delay_ms: 5,
            record_gain: 3.0,
        });
        assert_eq!(e.params().delay_ms, MIN_DELAY_MS);
        assert_eq!(e.params().record_gain, 1.0);
        assert_eq!(e.line().delay(), 960);
    }

    #[test]
    fn near_records_far_does_not() {
        let c = EchoConfig::default();
        assert_eq!(c.record_gain(0.0), 1.0);
        assert_eq!(c.record_gain(-30.0), 1.0, "behind too");
        assert!((c.record_gain(45.0) - 0.5).abs() < 1e-6);
        assert_eq!(c.record_gain(60.0), 0.0);
        assert_eq!(c.record_gain(500.0), 0.0);
        assert_eq!(c.record_gain(f64::NAN), 0.0);
        let mut last = 1.0;
        for d in 0..80 {
            let g = c.record_gain(f64::from(d));
            assert!(g <= last, "monotone at {d}");
            last = g;
        }
    }

    fn tone(n: usize, start: usize) -> Vec<i16> {
        (start..start + n)
            .map(|i| {
                let t = i as f32 / VOICE_RATE as f32;
                to_i16(0.5 * (std::f32::consts::TAU * 440.0 * t).sin())
            })
            .collect()
    }

    #[test]
    fn the_loop_delays_and_honours_the_microphone_mute() {
        let mut e = EchoLoop::new();
        assert_eq!(e.capture(&tone(480, 0), true), None, "off: nothing");
        e.set(EchoParams {
            active: true,
            delay_ms: 20,
            record_gain: 1.0,
        });
        // The gain ramps up from 0 in the first block (no click), then the
        // echo is the input 960 samples later.
        let mut out = Vec::new();
        for k in 0..6 {
            out.extend(e.capture(&tone(480, k * 480), true).unwrap());
        }
        assert!(out[..960].iter().all(|&s| s == 0), "silent for the delay");
        let input = tone(480 * 6, 0);
        for i in 960 + 480..out.len() {
            assert!(
                (i32::from(out[i]) - i32::from(input[i - 960])).abs() <= 1,
                "{i}"
            );
        }
        assert!(e.input_level() > 0.2, "{}", e.input_level());
        // Muted: silence goes in, so after the delay only silence comes out.
        let mut muted = Vec::new();
        for k in 6..12 {
            muted.extend(e.capture(&tone(480, k * 480), false).unwrap());
        }
        assert!(muted[960..].iter().all(|&s| s == 0));
        assert!(e.input_level() < 0.2, "the meter falls");
        // Turned off and on again: nothing old comes back.
        e.set(EchoParams {
            active: false,
            ..e.params()
        });
        e.set(EchoParams {
            active: true,
            ..e.params()
        });
        let again = e.capture(&[0; 960], true).unwrap();
        assert!(again.iter().all(|&s| s == 0));
    }
}
