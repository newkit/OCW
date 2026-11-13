//! The mixer: renders the ride sounds, the other riders and the playing
//! cues onto their buses, applies the bus gains (smoothed) and a soft
//! limiter, and writes interleaved samples for any channel count. The mix
//! is stereo: the own bike and the cues in the centre, the other riders
//! placed left and right ([`super::others`]); a mono device gets the
//! average, channels beyond the first two the centre. It runs inside the
//! output device's callback (or offline, for tests and the demo WAV) and
//! takes its input as [`MixerMsg`]s from the game.

use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use super::cues::{Cue, CueVoice};
use super::dsp::smoothing;
use super::mix::{Bus, BusGains};
use super::others::{OthersFrame, OthersSynth};
use super::ride::{RideParams, RideSynth};

/// Samples between parameter and filter updates.
const BLOCK: usize = 32;
/// Cues older than this when they arrive are dropped (no stale beeps
/// after the device was away).
const CUE_MAX_AGE: Duration = Duration::from_millis(400);
/// At most this many cues at once (a held key cannot pile them up).
const MAX_VOICES: usize = 16;
/// Make-up gain of the ride sounds against the cues (the demo ride sat
/// around -30 dBFS at the default volumes without it).
const RIDE_LEVEL: f32 = 1.6;

/// Input for the mixer.
#[derive(Debug, Clone, PartialEq)]
pub enum MixerMsg {
    /// The ride sounds' targets (sent every frame).
    Ride(RideParams),
    /// The other riders' sounds and places (every frame).
    Others(OthersFrame),
    /// Play a cue (with the time it was asked for).
    Cue(Cue, Instant),
    /// New bus gains (volume or mute changed).
    Gains(BusGains),
}

/// See the module docs.
pub struct Mixer {
    sr: f32,
    ride: RideSynth,
    ride_target: RideParams,
    ride_now: RideParams,
    others: OthersSynth,
    gains_target: BusGains,
    gains_now: BusGains,
    voices: Vec<CueVoice>,
    /// Coefficient of the per-block smoothing.
    smooth: f32,
    /// Samples into the current block.
    phase: usize,
    rx: Option<Receiver<MixerMsg>>,
    /// A video recording listens: every rendered buffer (stereo,
    /// interleaved) goes here too ([`super::record`]).
    tap: Option<Sender<Vec<f32>>>,
}

impl Mixer {
    pub fn new(sample_rate: u32, gains: BusGains) -> Self {
        let sr = sample_rate.max(8000) as f32;
        Self {
            sr,
            ride: RideSynth::new(sr),
            ride_target: RideParams::default(),
            ride_now: RideParams::default(),
            others: OthersSynth::new(sr),
            gains_target: gains,
            gains_now: gains,
            voices: Vec::new(),
            smooth: smoothing(0.06, sr, BLOCK),
            phase: 0,
            rx: None,
            tap: None,
        }
    }

    /// Takes its messages from `rx` before each buffer.
    pub fn with_receiver(mut self, rx: Receiver<MixerMsg>) -> Self {
        self.rx = Some(rx);
        self
    }

    pub fn sample_rate(&self) -> u32 {
        self.sr as u32
    }

    /// A device with another rate opened: the synthesis restarts at it
    /// (the targets and gains stay).
    pub fn set_sample_rate(&mut self, sample_rate: u32) {
        let sr = sample_rate.max(8000) as f32;
        if sr != self.sr {
            self.sr = sr;
            self.ride = RideSynth::new(sr);
            self.others = OthersSynth::new(sr);
            self.smooth = smoothing(0.06, sr, BLOCK);
            self.voices.clear();
        }
    }

    pub fn apply(&mut self, msg: MixerMsg) {
        match msg {
            MixerMsg::Ride(p) => self.ride_target = p,
            MixerMsg::Others(f) => self.others.apply(f),
            MixerMsg::Gains(g) => self.gains_target = g,
            MixerMsg::Cue(cue, at) => {
                if at.elapsed() <= CUE_MAX_AGE {
                    self.play(cue);
                }
            }
        }
    }

    /// Starts a cue now.
    pub fn play(&mut self, cue: Cue) {
        if self.voices.len() >= MAX_VOICES {
            self.voices.remove(0);
        }
        self.voices.push(CueVoice::new(cue, self.sr));
    }

    /// Cues playing now.
    #[cfg(test)]
    pub fn playing(&self) -> usize {
        self.voices.len()
    }

    fn drain(&mut self) {
        let msgs: Vec<MixerMsg> = match &self.rx {
            Some(rx) => rx.try_iter().collect(),
            None => return,
        };
        for m in msgs {
            self.apply(m);
        }
    }

    /// The other riders' voices playing now.
    #[cfg(test)]
    pub fn other_voices(&self) -> usize {
        self.others.voices()
    }

    /// Fills `out` (interleaved, `channels` per frame) with the mix: left
    /// and right on the first two channels, their average on a mono
    /// device and on any further channels.
    pub fn render(&mut self, out: &mut [f32], channels: usize) {
        self.drain();
        let channels = channels.max(1);
        let mut stereo = self
            .tap
            .as_ref()
            .map(|_| Vec::with_capacity(out.len() / channels * 2));
        for frame in out.chunks_mut(channels) {
            let (l, r) = self.next_sample();
            let mid = 0.5 * (l + r);
            match frame {
                [m] => *m = mid,
                [a, b, rest @ ..] => {
                    *a = l;
                    *b = r;
                    rest.fill(mid);
                }
                [] => {}
            }
            if let Some(s) = stereo.as_mut() {
                s.extend([l, r]);
            }
        }
        if let (Some(tap), Some(s)) = (&self.tap, stereo)
            && tap.send(s).is_err()
        {
            self.tap = None; // the recording ended
        }
    }

    /// Sends a copy of everything rendered from now on to `tap` (`None`:
    /// stop).
    pub fn set_tap(&mut self, tap: Option<Sender<Vec<f32>>>) {
        self.tap = tap;
    }

    /// The next stereo sample (left, right).
    fn next_sample(&mut self) -> (f32, f32) {
        if self.phase == 0 {
            self.ride_now
                .approach(&self.ride_target, BLOCK as f32 / self.sr);
            self.gains_now.approach(&self.gains_target, self.smooth);
            self.ride.set_filters(&self.ride_now);
            self.others.block(BLOCK);
        }
        self.phase = (self.phase + 1) % BLOCK;
        // Centre buses: music, effects, voice, others (cues only).
        let mut bus = [0.0f32; 4];
        bus[1] = RIDE_LEVEL * self.ride.sample(&self.ride_now);
        let sr = self.sr;
        for v in &mut self.voices {
            let s = v.sample(sr);
            match v.cue.bus() {
                Bus::Music => bus[0] += s,
                Bus::Effects => bus[1] += s,
                Bus::Voice => bus[2] += s,
                Bus::Others => bus[3] += s,
            }
        }
        self.voices.retain(|v| !v.done(sr));
        let g = &self.gains_now;
        let others = g.get(Bus::Others);
        let centre = bus[0] * g.get(Bus::Music)
            + bus[1] * g.get(Bus::Effects)
            + bus[2] * g.get(Bus::Voice)
            + bus[3] * others;
        let (ol, or) = self.others.sample();
        (
            soft_limit(centre + RIDE_LEVEL * others * ol),
            soft_limit(centre + RIDE_LEVEL * others * or),
        )
    }
}

/// Keeps the output within ±1 without hard clipping: linear up to 0.8,
/// then a smooth knee.
pub fn soft_limit(x: f32) -> f32 {
    const KNEE: f32 = 0.8;
    let a = x.abs();
    if a <= KNEE {
        x
    } else {
        let over = (a - KNEE) / (1.0 - KNEE);
        x.signum() * (KNEE + (1.0 - KNEE) * over.tanh())
    }
}

#[cfg(test)]
mod tests {
    use super::super::mix::Mix;
    use super::super::ride::{RideInput, ride_params};
    use super::*;
    use ocw_world::Surface;

    const SR: u32 = 48_000;

    fn full() -> BusGains {
        Mix {
            master: 100,
            music: 100,
            effects: 100,
            voice: 100,
            others: 100,
            mute: false,
        }
        .gains()
    }

    fn render(m: &mut Mixer, seconds: f32, channels: usize) -> Vec<f32> {
        let mut out = vec![0.0; (seconds * SR as f32) as usize * channels];
        // Device-sized buffers.
        for chunk in out.chunks_mut(512 * channels) {
            m.render(chunk, channels);
        }
        out
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    fn riding(kmh: f64) -> MixerMsg {
        MixerMsg::Ride(ride_params(&RideInput {
            speed_mps: kmh / 3.6,
            cadence_rpm: Some(90.0),
            power_w: 250.0,
            surface: Surface::Asphalt,
        }))
    }

    #[test]
    fn silence_without_input() {
        let mut m = Mixer::new(SR, full());
        assert!(render(&mut m, 0.2, 2).iter().all(|&s| s == 0.0));
    }

    #[test]
    fn every_channel_gets_the_mix() {
        let mut m = Mixer::new(SR, full());
        m.apply(riding(35.0));
        let out = render(&mut m, 0.3, 6);
        assert!(rms(&out) > 0.01);
        // Without other riders the mix is centred: alike everywhere.
        for f in out.chunks(6) {
            assert!(f.iter().all(|&s| s == f[0]));
        }
    }

    #[test]
    fn the_effects_volume_scales_the_ride_sounds() {
        let level = |effects: u8, mute: bool| {
            let g = Mix {
                master: 100,
                effects,
                mute,
                ..Mix::default()
            }
            .gains();
            let mut m = Mixer::new(SR, g);
            m.apply(riding(30.0));
            rms(&render(&mut m, 0.5, 1)[SR as usize / 4..])
        };
        let (loud, half) = (level(100, false), level(50, false));
        // 50 % is a quarter of the gain (squared curve), within the
        // limiter's linear range.
        assert!((half / loud - 0.25).abs() < 0.02, "{half} / {loud}");
        assert_eq!(level(100, true), 0.0);
    }

    #[test]
    fn cues_play_on_their_bus() {
        // Voice muted (0 %), effects on: the voice test tone is silent,
        // the effects one sounds.
        let g = Mix {
            master: 100,
            effects: 100,
            voice: 0,
            ..Mix::default()
        }
        .gains();
        let mut m = Mixer::new(SR, g);
        m.apply(MixerMsg::Cue(Cue::Test(Bus::Voice), Instant::now()));
        assert_eq!(m.playing(), 1);
        assert_eq!(rms(&render(&mut m, 0.3, 1)), 0.0);
        m.apply(MixerMsg::Cue(Cue::Test(Bus::Effects), Instant::now()));
        assert!(rms(&render(&mut m, 0.3, 1)) > 0.05);
        // Done after their length.
        render(&mut m, 1.0, 1);
        assert_eq!(m.playing(), 0);
    }

    #[test]
    fn stale_cues_are_dropped_and_voices_are_capped() {
        let mut m = Mixer::new(SR, full());
        let old = Instant::now() - Duration::from_secs(5);
        m.apply(MixerMsg::Cue(Cue::Finish, old));
        assert_eq!(m.playing(), 0);
        for _ in 0..40 {
            m.play(Cue::UiMove);
        }
        assert_eq!(m.playing(), MAX_VOICES);
    }

    #[test]
    fn messages_arrive_through_the_channel() {
        let (tx, rx) = std::sync::mpsc::sync_channel(8);
        let mut m = Mixer::new(SR, BusGains::default()).with_receiver(rx);
        tx.send(MixerMsg::Gains(full())).unwrap();
        tx.send(MixerMsg::Cue(Cue::CountdownGo, Instant::now()))
            .unwrap();
        let out = render(&mut m, 0.2, 2);
        assert!(rms(&out) > 0.05);
    }

    #[test]
    fn output_stays_within_range_when_everything_plays() {
        let mut m = Mixer::new(SR, full());
        m.apply(MixerMsg::Ride(ride_params(&RideInput {
            speed_mps: 25.0,
            cadence_rpm: Some(110.0),
            power_w: 900.0,
            surface: Surface::Gravel,
        })));
        for _ in 0..MAX_VOICES {
            m.play(Cue::Finish);
        }
        let out = render(&mut m, 1.0, 2);
        assert!(out.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
        assert!(soft_limit(5.0) <= 1.0 && soft_limit(-5.0) >= -1.0);
        assert_eq!(soft_limit(0.5), 0.5);
    }

    fn others_on_the_right(power: f64) -> MixerMsg {
        use super::super::others::{OtherRider, SOURCE_HEIGHT_M, frame};
        use super::super::spatial::Listener;
        let l = Listener {
            pos: [0.0, 1.5, 0.0],
            forward: [1.0, 0.0],
            velocity: [8.0, 0.0, 0.0],
        };
        let r = OtherRider {
            id: ocw_core::RiderId(4),
            pos: [0.5, SOURCE_HEIGHT_M, 2.0],
            velocity: [9.0, 0.0, 0.0],
            speed_mps: 9.0,
            power_w: power,
            cadence_rpm: None,
            surface: Surface::Asphalt,
        };
        MixerMsg::Others(frame(&l, &[r], &[]))
    }

    fn channel_rms(x: &[f32], ch: usize) -> f32 {
        rms(&x.iter().skip(ch).step_by(2).copied().collect::<Vec<_>>())
    }

    /// Another rider on the right is heard on the right channel; a mono
    /// device gets the average; the own bike stays in the centre.
    #[test]
    fn other_riders_are_placed_in_stereo() {
        let mut m = Mixer::new(SR, full());
        m.apply(others_on_the_right(0.0));
        let out = render(&mut m, 0.5, 2);
        let tail = &out[SR as usize / 2..];
        let (l, r) = (channel_rms(tail, 0), channel_rms(tail, 1));
        assert!(r > 0.003 && r > 3.0 * l, "{l} {r}");
        assert_eq!(m.other_voices(), 1);
        let mut mono = Mixer::new(SR, full());
        mono.apply(others_on_the_right(0.0));
        let one = render(&mut mono, 0.5, 1);
        for (i, s) in one.iter().enumerate().take(4800) {
            assert!((s - 0.5 * (out[2 * i] + out[2 * i + 1])).abs() < 1e-6);
        }
        // Their volume: 0 silences them, the own bike plays on.
        let quiet = Mix {
            master: 100,
            effects: 100,
            others: 0,
            ..Mix::default()
        }
        .gains();
        let mut m = Mixer::new(SR, quiet);
        m.apply(others_on_the_right(250.0));
        assert_eq!(rms(&render(&mut m, 0.3, 2)), 0.0);
        m.apply(riding(30.0));
        let out = render(&mut m, 0.3, 2);
        assert!(rms(&out) > 0.01);
        assert!(out.chunks(2).all(|f| f[0] == f[1]), "own bike centred");
    }

    #[test]
    fn another_sample_rate_restarts_the_synthesis() {
        let mut m = Mixer::new(44_100, full());
        m.apply(riding(30.0));
        m.play(Cue::Finish);
        m.set_sample_rate(96_000);
        assert_eq!((m.sample_rate(), m.playing()), (96_000, 0));
        let mut out = vec![0.0; 9600];
        m.render(&mut out, 1);
        assert!(rms(&out) > 0.0);
    }
}
