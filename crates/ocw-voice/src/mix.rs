//! The voice mixer (spec §7: "mix on the client"): every remote voice
//! arrives as 48 kHz mono and is played with its own gain (spatial
//! attenuation × voice volume) and pan; the speaker mute silences all of
//! them. Also linear resampling for devices that do not run at 48 kHz and
//! the short confirmation sounds of the mute toggles (spec §2.4).
//!
//! Pure and allocation-light, so the audio callback can call it directly.

use std::collections::{BTreeMap, VecDeque};

/// The rate voices are received and sent at.
pub const VOICE_RATE: u32 = 48_000;
/// 10 ms at [`VOICE_RATE`]: WebRTC's frame size.
pub const FRAME_SAMPLES: usize = (VOICE_RATE / 100) as usize;
/// A voice buffers at most this much (oldest dropped): bounds the delay
/// when the output runs slower than the network delivers.
pub const MAX_BUFFER_SAMPLES: usize = FRAME_SAMPLES * 20;

/// Constant-power pan law: (left, right) gains for pan −1 … 1.
pub fn pan_gains(pan: f32) -> (f32, f32) {
    let angle = (pan.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
    (angle.cos(), angle.sin())
}

#[derive(Debug, Clone, Default)]
struct Voice {
    queue: VecDeque<f32>,
    gain: f32,
    pan: f32,
    /// What was played of it (after gain; 0 while the speaker is muted).
    meter: Meter,
}

/// A level meter for the HUD: the RMS of each block, rising at once and
/// falling by [`Meter::RELEASE`] per 10 ms.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Meter {
    level: f32,
}

impl Meter {
    /// Kept of the level per 10 ms.
    pub const RELEASE: f32 = 0.7;

    pub fn update(&mut self, block: &[f32]) {
        if block.is_empty() {
            return;
        }
        let rms = (block.iter().map(|s| s * s).sum::<f32>() / block.len() as f32).sqrt();
        let decay = Self::RELEASE.powf(block.len() as f32 / FRAME_SAMPLES as f32);
        self.level = rms.max(self.level * decay).min(1.0);
    }

    /// 0 … 1.
    pub fn level(&self) -> f32 {
        self.level
    }
}

/// Mixes the voices (keyed by participant identity) into stereo.
#[derive(Debug, Clone, Default)]
pub struct Mixer {
    voices: BTreeMap<String, Voice>,
    /// The speaker mute: voices are still received (unmuting is instant),
    /// but not played.
    pub muted: bool,
    /// A confirmation sound being played (mono samples).
    cue: VecDeque<f32>,
    /// Samples dropped because a voice's buffer was full.
    pub dropped: u64,
}

impl Mixer {
    /// Adds received samples (48 kHz mono, i16) of a voice.
    pub fn push(&mut self, identity: &str, samples: &[i16]) {
        let v = self
            .voices
            .entry(identity.to_owned())
            .or_insert_with(|| Voice {
                gain: 1.0,
                ..Default::default()
            });
        v.queue
            .extend(samples.iter().map(|&s| f32::from(s) / 32768.0));
        let over = v.queue.len().saturating_sub(MAX_BUFFER_SAMPLES);
        if over > 0 {
            v.queue.drain(..over);
            self.dropped += over as u64;
        }
    }

    pub fn set_placement(&mut self, identity: &str, gain: f32, pan: f32) {
        let v = self.voices.entry(identity.to_owned()).or_default();
        v.gain = gain.max(0.0);
        v.pan = pan;
    }

    pub fn remove(&mut self, identity: &str) {
        self.voices.remove(identity);
    }

    pub fn clear(&mut self) {
        self.voices.clear();
    }

    /// Drops what a voice has buffered (it keeps its placement).
    pub fn flush(&mut self, identity: &str) {
        if let Some(v) = self.voices.get_mut(identity) {
            v.queue.clear();
            v.meter = Meter::default();
        }
    }

    /// How loud a voice was played lately (0 … 1; 0 while muted).
    pub fn level(&self, identity: &str) -> f32 {
        self.voices.get(identity).map_or(0.0, |v| v.meter.level())
    }

    /// Queues a confirmation sound (played even with the speaker muted:
    /// it confirms the toggle).
    pub fn play_cue(&mut self, cue: Cue) {
        self.cue = cue_samples(cue).into();
    }

    pub fn buffered(&self, identity: &str) -> usize {
        self.voices.get(identity).map_or(0, |v| v.queue.len())
    }

    /// Fills `out` (interleaved stereo at 48 kHz, `out.len() / 2` frames)
    /// with the mix, clipped to −1 … 1. Returns the mono mix it played
    /// (the echo canceller's reference) in `reference` if given.
    pub fn pull_stereo(&mut self, out: &mut [f32], mut reference: Option<&mut Vec<f32>>) {
        out.fill(0.0);
        let frames = out.len() / 2;
        if let Some(r) = reference.as_deref_mut() {
            r.clear();
            r.resize(frames, 0.0);
        }
        let mut played = Vec::with_capacity(frames);
        for v in self.voices.values_mut() {
            let take = frames.min(v.queue.len());
            let (l, r) = pan_gains(v.pan);
            played.clear();
            played.resize(frames, 0.0);
            for (i, s) in v.queue.drain(..take).enumerate() {
                if self.muted {
                    continue;
                }
                let s = s * v.gain;
                played[i] = s;
                out[2 * i] += s * l;
                out[2 * i + 1] += s * r;
                if let Some(m) = reference.as_deref_mut() {
                    m[i] += s;
                }
            }
            v.meter.update(&played);
        }
        let take = frames.min(self.cue.len());
        for (i, s) in self.cue.drain(..take).enumerate() {
            out[2 * i] += s;
            out[2 * i + 1] += s;
            if let Some(m) = reference.as_deref_mut() {
                m[i] += s;
            }
        }
        for s in out.iter_mut() {
            *s = s.clamp(-1.0, 1.0);
        }
    }
}

/// Writes `stereo` (interleaved, 2 channels) to a device buffer with
/// `channels` channels: mono devices get the average, extra channels
/// silence.
pub fn to_channels(stereo: &[f32], channels: usize, out: &mut [f32]) {
    for (frame, o) in stereo
        .as_chunks::<2>()
        .0
        .iter()
        .zip(out.chunks_exact_mut(channels.max(1)))
    {
        match channels {
            0 => {}
            1 => o[0] = 0.5 * (frame[0] + frame[1]),
            _ => {
                o[0] = frame[0];
                o[1] = frame[1];
                o[2..].fill(0.0);
            }
        }
    }
}

/// Averages interleaved `channels`-channel samples to mono.
pub fn to_mono(interleaved: &[f32], channels: usize) -> Vec<f32> {
    let c = channels.max(1);
    interleaved
        .chunks_exact(c)
        .map(|f| f.iter().sum::<f32>() / c as f32)
        .collect()
}

pub fn to_i16(s: f32) -> i16 {
    (s.clamp(-1.0, 1.0) * 32767.0).round() as i16
}

/// Streaming linear resampler (mono). Good enough for speech; keeps its
/// position between calls, so blocks join without clicks.
#[derive(Debug, Clone)]
pub struct Resampler {
    from: u32,
    to: u32,
    /// Position of the next output sample in input samples, relative to
    /// `last` (the last input sample of the previous block).
    pos: f64,
    last: f32,
}

impl Resampler {
    pub fn new(from: u32, to: u32) -> Self {
        Self {
            from: from.max(1),
            to: to.max(1),
            pos: 1.0,
            last: 0.0,
        }
    }

    pub fn is_identity(&self) -> bool {
        self.from == self.to
    }

    /// Resamples a block, appending to `out`.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if self.is_identity() {
            out.extend_from_slice(input);
            return;
        }
        let step = f64::from(self.from) / f64::from(self.to);
        // Sample k of the extended block [last, input...] is at index k.
        let at = |k: usize| if k == 0 { self.last } else { input[k - 1] };
        let n = input.len();
        while self.pos <= n as f64 {
            let i = self.pos.floor() as usize;
            let frac = (self.pos - i as f64) as f32;
            let a = at(i);
            let b = if i < n { at(i + 1) } else { a };
            out.push(a + (b - a) * frac);
            self.pos += step;
        }
        if n > 0 {
            self.last = input[n - 1];
            self.pos -= n as f64;
        }
    }
}

/// The confirmation sounds (spec §2.4): a short rising tone for "on", a
/// falling one for "muted", two soft tones when someone joins or leaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cue {
    On,
    Muted,
    Joined,
    Left,
}

/// The cue's samples (48 kHz mono, about 0.12–0.2 s, quiet).
pub fn cue_samples(cue: Cue) -> Vec<f32> {
    let (tones, amp): (&[f32], f32) = match cue {
        Cue::On => (&[660.0, 990.0], 0.18),
        Cue::Muted => (&[990.0, 660.0], 0.18),
        Cue::Joined => (&[523.0, 784.0], 0.10),
        Cue::Left => (&[784.0, 523.0], 0.10),
    };
    let per = (VOICE_RATE as usize) * 6 / 100; // 60 ms per tone
    let mut out = Vec::with_capacity(per * tones.len());
    for &f in tones {
        for i in 0..per {
            let t = i as f32 / VOICE_RATE as f32;
            // Short fade in and out: no clicks.
            let env = (i.min(per - i) as f32 / (per as f32 * 0.2)).min(1.0);
            out.push(amp * env * (std::f32::consts::TAU * f * t).sin());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pan_law_keeps_power() {
        for p in [-1.0f32, -0.5, 0.0, 0.3, 1.0, 2.0] {
            let (l, r) = pan_gains(p);
            assert!((l * l + r * r - 1.0).abs() < 1e-5, "{p}");
        }
        let (l, r) = pan_gains(-1.0);
        assert!(l > 0.999 && r < 1e-3);
        let (l, r) = pan_gains(0.0);
        assert!((l - r).abs() < 1e-6);
    }

    #[test]
    fn mixes_voices_with_gain_and_pan() {
        let mut m = Mixer::default();
        m.push("rider-1", &[16384; 4]);
        m.push("rider-2", &[8192; 2]);
        m.set_placement("rider-1", 0.5, 0.0);
        m.set_placement("rider-2", 1.0, 1.0);
        let mut out = [0.0f32; 8];
        let mut reference = Vec::new();
        m.pull_stereo(&mut out, Some(&mut reference));
        let c = std::f32::consts::FRAC_1_SQRT_2 * 0.25;
        // Frames 0, 1: both voices; rider 2 only on the right.
        assert!((out[0] - c).abs() < 1e-5);
        assert!((out[1] - (c + 0.25)).abs() < 1e-5);
        // Frames 2, 3: rider 1 only.
        assert!((out[4] - c).abs() < 1e-5 && (out[5] - c).abs() < 1e-5);
        assert!((reference[0] - 0.5).abs() < 1e-5 && (reference[3] - 0.25).abs() < 1e-5);
        assert_eq!(m.buffered("rider-1"), 0);
        // Nothing buffered: silence.
        m.pull_stereo(&mut out, None);
        assert!(out.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn speaker_mute_silences_voices_but_not_the_cue() {
        let mut m = Mixer {
            muted: true,
            ..Default::default()
        };
        m.push("rider-1", &[20000; 480]);
        let mut out = vec![0.0f32; 960];
        m.pull_stereo(&mut out, None);
        assert!(out.iter().all(|&s| s == 0.0));
        // Consumed while muted: no backlog of old speech after unmuting.
        assert_eq!(m.buffered("rider-1"), 0);
        m.play_cue(Cue::On);
        m.pull_stereo(&mut out, None);
        assert!(out.iter().any(|&s| s.abs() > 0.05));
    }

    #[test]
    fn buffers_are_bounded_and_output_clipped() {
        let mut m = Mixer::default();
        m.push("a", &vec![32767; MAX_BUFFER_SAMPLES + 100]);
        assert_eq!(m.buffered("a"), MAX_BUFFER_SAMPLES);
        assert_eq!(m.dropped, 100);
        m.push("b", &[32767; 10]);
        m.set_placement("a", 3.0, 0.0);
        let mut out = [0.0f32; 20];
        m.pull_stereo(&mut out, None);
        assert!(out.iter().all(|&s| (-1.0..=1.0).contains(&s)));
        m.remove("a");
        assert_eq!(m.buffered("a"), 0);
    }

    #[test]
    fn channel_conversion() {
        let stereo = [0.2, 0.4, -0.2, 0.0];
        let mut mono = [0.0; 2];
        to_channels(&stereo, 1, &mut mono);
        assert!((mono[0] - 0.3).abs() < 1e-6 && (mono[1] + 0.1).abs() < 1e-6);
        let mut quad = [9.0; 8];
        to_channels(&stereo, 4, &mut quad);
        assert_eq!(quad, [0.2, 0.4, 0.0, 0.0, -0.2, 0.0, 0.0, 0.0]);
        let m = to_mono(&[0.2, 0.4, 1.0, 0.0], 2);
        assert!((m[0] - 0.3).abs() < 1e-6 && (m[1] - 0.5).abs() < 1e-6);
        assert_eq!(to_i16(1.5), 32767);
        assert_eq!(to_i16(-1.0), -32767);
        assert_eq!(to_i16(0.0), 0);
    }

    #[test]
    fn resampler_rates_and_continuity() {
        // 44.1 kHz → 48 kHz in blocks: the right number of samples.
        let mut r = Resampler::new(44_100, 48_000);
        let mut out = Vec::new();
        for _ in 0..100 {
            r.process(&[0.5; 441], &mut out);
        }
        let expected = 48_000.0 * 1.0;
        assert!((out.len() as f64 - expected).abs() <= 2.0, "{}", out.len());
        // A ramp stays a ramp across block borders (no jumps).
        let mut r = Resampler::new(48_000, 16_000);
        let ramp: Vec<f32> = (0..960).map(|i| i as f32).collect();
        let mut out = Vec::new();
        for block in ramp.chunks(100) {
            r.process(block, &mut out);
        }
        assert!((out.len() as i32 - 320).abs() <= 1);
        for w in out.windows(2).skip(1) {
            assert!((w[1] - w[0] - 3.0).abs() < 1e-3, "{w:?}");
        }
        let mut id = Resampler::new(48_000, 48_000);
        let mut o = Vec::new();
        id.process(&[0.1, 0.2], &mut o);
        assert_eq!(o, vec![0.1, 0.2]);
    }

    #[test]
    fn cues_are_short_quiet_and_distinct() {
        let on = cue_samples(Cue::On);
        let muted = cue_samples(Cue::Muted);
        assert!(on.len() < VOICE_RATE as usize / 4);
        assert!(on.iter().all(|s| s.abs() <= 0.2));
        assert_ne!(on, muted);
        // Fades: starts and ends silent.
        assert!(on[0].abs() < 1e-3 && on.last().unwrap().abs() < 0.02);
        assert!(cue_samples(Cue::Joined).iter().all(|s| s.abs() <= 0.1));
    }
}
