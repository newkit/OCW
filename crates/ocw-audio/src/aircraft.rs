//! The sounds of the simulated aircraft (spec 14 Phase 18, 18A *Sound*,
//! 18C, T8): a distant jet's rumble, a small plane's propeller drone, a
//! helicopter's blade thump, a glider's whistle and a balloon's burner –
//! synthesised, placed by where the aircraft *was* when the sound now
//! arriving left it (the caller passes that place: the delay of sound,
//! `ocw_sim::air::heard`), attenuated by the distance and dulled by the
//! air, with the Doppler shift of its speed toward the listener. At most
//! [`VOICES`] aircraft are heard at once (18A: two), the loudest.

use std::f32::consts::TAU;

use crate::dsp::{Noise, Svf};

/// Aircraft heard at once.
pub const VOICES: usize = 2;
/// The speed of sound (m/s).
const C: f64 = 343.0;
/// Silent beyond this (m).
pub const MAX_M: f64 = 25_000.0;

/// What makes the sound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Jet,
    Prop,
    Rotor,
    Glider,
    Burner,
    /// A UFO's eerie hum (spec 18E T9): two slow, detuned low tones and a
    /// wavering high one, like a theremin.
    Ufo,
}

impl Engine {
    /// The distance (m) at which it is as loud as a near rider's sounds,
    /// and the inverse-distance law beyond.
    pub fn reference_m(self) -> f64 {
        match self {
            Self::Jet => 900.0,
            Self::Prop => 120.0,
            Self::Rotor => 260.0,
            Self::Glider => 25.0,
            Self::Burner => 60.0,
            Self::Ufo => 140.0,
        }
    }
}

/// One aircraft as it is heard now.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AircraftSound {
    pub id: u64,
    pub engine: Engine,
    pub left: f32,
    pub right: f32,
    /// Frequency factor (Doppler).
    pub doppler: f32,
    /// Low-pass of the air (Hz).
    pub cutoff_hz: f32,
    /// Engine / rotor speed 0 … 1.
    pub power: f32,
}

impl AircraftSound {
    /// The sound of a source at `pos` moving at `vel` (where and how fast
    /// it was when the sound left it) for a listener at `ear` facing
    /// `forward` (x, z); `None` when it cannot be heard.
    pub fn placed(
        id: u64,
        engine: Engine,
        ear: [f64; 3],
        forward: [f64; 2],
        pos: [f64; 3],
        vel: [f64; 3],
        power: f64,
    ) -> Option<Self> {
        let d = [pos[0] - ear[0], pos[1] - ear[1], pos[2] - ear[2]];
        let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        if !dist.is_finite() || dist >= MAX_M || power <= 0.0 {
            return None;
        }
        let r = engine.reference_m();
        let mut gain = (r / dist.max(r)).min(1.0);
        // Fade out over the last fifth.
        gain *= ((MAX_M - dist) / (0.2 * MAX_M)).clamp(0.0, 1.0);
        gain *= power.clamp(0.0, 1.0).sqrt();
        if gain < 1e-4 {
            return None;
        }
        // Pan by the horizontal direction (centred when overhead).
        let flat = d[0].hypot(d[2]);
        let f = {
            let l = forward[0].hypot(forward[1]);
            if l > 1e-9 {
                [forward[0] / l, forward[1] / l]
            } else {
                [0.0, -1.0]
            }
        };
        let right = [-f[1], f[0]];
        let pan = if flat > 1e-6 {
            (d[0] * right[0] + d[2] * right[1]) / flat * (flat / dist)
        } else {
            0.0
        };
        let behind = if flat > 1e-6 && (d[0] * f[0] + d[2] * f[1]) < 0.0 {
            0.85
        } else {
            1.0
        };
        let g = gain * behind;
        // Radial speed toward the listener: higher when coming closer.
        let toward = -(vel[0] * d[0] + vel[1] * d[1] + vel[2] * d[2]) / dist.max(1.0);
        let doppler = (C / (C - toward.clamp(-200.0, 200.0))).clamp(0.82, 1.2);
        Some(Self {
            id,
            engine,
            left: (g * (1.0 - pan).max(0.0).sqrt()) as f32,
            right: (g * (1.0 + pan).max(0.0).sqrt()) as f32,
            doppler: doppler as f32,
            cutoff_hz: (16_000.0 * 250.0 / (250.0 + dist)).max(120.0) as f32,
            power: power.clamp(0.0, 1.0) as f32,
        })
    }

    /// The loudness used to pick the voices.
    pub fn level(&self) -> f32 {
        self.left.max(self.right)
    }
}

/// The loudest `VOICES` of `sounds`.
pub fn loudest(mut sounds: Vec<AircraftSound>) -> Vec<AircraftSound> {
    sounds.sort_by(|a, b| b.level().total_cmp(&a.level()).then(a.id.cmp(&b.id)));
    sounds.truncate(VOICES);
    sounds
}

/// One voice's synthesis.
#[derive(Debug, Clone)]
struct Voice {
    id: Option<u64>,
    engine: Engine,
    noise: Noise,
    brown: f32,
    air: Svf,
    body: Svf,
    phase: [f32; 3],
    /// Smoothed left, right, doppler, cutoff.
    now: [f32; 4],
    target: Option<AircraftSound>,
}

impl Voice {
    fn new(seed: u32, sr: f32) -> Self {
        Self {
            id: None,
            engine: Engine::Jet,
            noise: Noise::new(seed),
            brown: 0.0,
            air: Svf::new(2000.0, 0.7, sr),
            body: Svf::new(150.0, 1.2, sr),
            phase: [0.0; 3],
            now: [0.0, 0.0, 1.0, 2000.0],
            target: None,
        }
    }

    fn sample(&mut self, sr: f32) -> (f32, f32) {
        let k = 1.0 - (-1.0 / (0.08 * sr)).exp();
        let t = self
            .target
            .map_or([0.0, 0.0, self.now[2], self.now[3]], |s| {
                [s.left, s.right, s.doppler, s.cutoff_hz]
            });
        for (n, t) in self.now.iter_mut().zip(t) {
            *n += (t - *n) * k;
        }
        let [l, r, doppler, cutoff] = self.now;
        if l < 1e-5 && r < 1e-5 {
            return (0.0, 0.0);
        }
        let power = self.target.map_or(1.0, |s| s.power);
        self.air.set(cutoff.clamp(80.0, 16_000.0), 0.7, sr);
        let w = self.noise.white();
        let x = match self.engine {
            Engine::Jet => {
                // A deep roar with a faint whine close by.
                self.brown = (self.brown + 0.06 * w) * 0.996;
                self.body.set(110.0 * doppler, 0.9, sr);
                let rumble = self.body.process(self.brown).band * 2.0 + self.brown * 0.6;
                self.phase[0] = (self.phase[0] + 2300.0 * doppler / sr).fract();
                let whine = (TAU * self.phase[0]).sin() * 0.04 * (cutoff / 16_000.0).powi(2);
                0.55 * rumble + whine
            }
            Engine::Prop => {
                let f0 = 82.0 * doppler * (0.6 + 0.4 * power);
                self.phase[0] = (self.phase[0] + f0 / sr).fract();
                let a = TAU * self.phase[0];
                let mut h = 0.0;
                for n in 1..7 {
                    h += (n as f32 * a).sin() / n as f32;
                }
                0.12 * h + 0.05 * w
            }
            Engine::Rotor => {
                // The blade passes: noise in pulses, the tail rotor's buzz.
                let f0 = 21.0 * doppler * (0.7 + 0.3 * power);
                self.phase[0] = (self.phase[0] + f0 / sr).fract();
                let p = self.phase[0] - 0.5;
                let pulse = (-(p * 9.0).powi(2)).exp();
                self.phase[1] = (self.phase[1] + 96.0 * doppler / sr).fract();
                let tail = (TAU * self.phase[1]).sin() + 0.4 * (2.0 * TAU * self.phase[1]).sin();
                self.body.set(160.0 * doppler, 1.0, sr);
                let thump = self.body.process(w).band;
                0.5 * thump * (0.15 + pulse) + 0.03 * tail + 0.15 * pulse * (TAU * p).cos()
            }
            Engine::Glider => {
                self.body.set(1100.0 * doppler, 2.0, sr);
                0.15 * self.body.process(w).band
            }
            Engine::Burner => {
                self.body.set(700.0, 0.6, sr);
                0.35 * self.body.process(w).low + 0.1 * w
            }
            Engine::Ufo => {
                // Two low tones beating slowly, a wavering high one.
                self.phase[0] = (self.phase[0] + 92.0 * doppler / sr).fract();
                self.phase[1] = (self.phase[1] + 93.7 * doppler / sr).fract();
                self.phase[2] = (self.phase[2] + 0.31 / sr).fract();
                let vib = (TAU * self.phase[2]).sin();
                self.brown = (self.brown
                    + (620.0 + 90.0 * vib) * doppler * (0.7 + 0.3 * power) / sr)
                    .fract();
                let low = (TAU * self.phase[0]).sin()
                    + (TAU * self.phase[1]).sin()
                    + 0.3 * (2.0 * TAU * self.phase[0]).sin();
                let high = (TAU * self.brown).sin();
                0.16 * low + 0.06 * high * (0.6 + 0.4 * vib) + 0.01 * w
            }
        };
        let y = self.air.process(x).low;
        (y * l, y * r)
    }
}

/// The aircraft voices.
#[derive(Debug, Clone)]
pub struct AircraftSynth {
    sr: f32,
    voices: Vec<Voice>,
}

impl AircraftSynth {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sr: sample_rate,
            voices: (0..VOICES as u32)
                .map(|i| Voice::new(0xA1E0 + i, sample_rate))
                .collect(),
        }
    }

    /// This frame's sounds (at most [`VOICES`]; an aircraft keeps its
    /// voice while it is heard).
    pub fn apply(&mut self, sounds: &[AircraftSound]) {
        let sounds = &sounds[..sounds.len().min(VOICES)];
        for v in &mut self.voices {
            v.target =
                v.id.and_then(|id| sounds.iter().find(|s| s.id == id).copied());
            if v.target.is_none() {
                v.id = None;
            }
        }
        for s in sounds {
            if self.voices.iter().any(|v| v.id == Some(s.id)) {
                continue;
            }
            // A free voice, the quietest first.
            if let Some(v) = self
                .voices
                .iter_mut()
                .filter(|v| v.id.is_none())
                .min_by(|a, b| (a.now[0] + a.now[1]).total_cmp(&(b.now[0] + b.now[1])))
            {
                v.id = Some(s.id);
                v.engine = s.engine;
                v.target = Some(*s);
                // A new source starts silent and fades in.
                v.now[0] = 0.0;
                v.now[1] = 0.0;
                v.now[2] = s.doppler;
                v.now[3] = s.cutoff_hz;
            }
        }
    }

    pub fn sample(&mut self) -> (f32, f32) {
        let sr = self.sr;
        self.voices.iter_mut().fold((0.0, 0.0), |(l, r), v| {
            let (a, b) = v.sample(sr);
            (l + a, r + b)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(s: &mut AircraftSynth, n: usize) -> (f32, f32) {
        let (mut a, mut b) = (0.0, 0.0);
        for _ in 0..n {
            let (l, r) = s.sample();
            a += l * l;
            b += r * r;
        }
        ((a / n as f32).sqrt(), (b / n as f32).sqrt())
    }

    #[test]
    fn far_is_quieter_duller_and_placed() {
        let ear = [0.0, 0.0, 0.0];
        let north = [0.0, -1.0];
        // A jet 2 km east, flying north: on the right.
        let near = AircraftSound::placed(
            1,
            Engine::Jet,
            ear,
            north,
            [2000.0, 900.0, 0.0],
            [0.0, 0.0, -230.0],
            1.0,
        )
        .unwrap();
        assert!(near.right > 2.0 * near.left, "{near:?}");
        let far = AircraftSound::placed(
            1,
            Engine::Jet,
            ear,
            north,
            [9000.0, 10_000.0, 0.0],
            [0.0, 0.0, -230.0],
            1.0,
        )
        .unwrap();
        assert!(far.level() < near.level() * 0.4);
        assert!(far.cutoff_hz < near.cutoff_hz && far.cutoff_hz < 600.0);
        assert!(
            AircraftSound::placed(
                1,
                Engine::Jet,
                ear,
                north,
                [30_000.0, 0.0, 0.0],
                [0.0; 3],
                1.0
            )
            .is_none()
        );
        // Coming closer: higher; going away: lower.
        let coming = AircraftSound::placed(
            2,
            Engine::Prop,
            ear,
            north,
            [0.0, 100.0, -1000.0],
            [0.0, 0.0, 50.0],
            1.0,
        )
        .unwrap();
        let going = AircraftSound::placed(
            2,
            Engine::Prop,
            ear,
            north,
            [0.0, 100.0, -1000.0],
            [0.0, 0.0, -50.0],
            1.0,
        )
        .unwrap();
        assert!(coming.doppler > 1.1 && going.doppler < 0.9);
    }

    #[test]
    fn two_voices_fade_in_and_out() {
        let mut s = AircraftSynth::new(48_000.0);
        assert_eq!(rms(&mut s, 4800), (0.0, 0.0));
        let ear = [0.0; 3];
        let sounds: Vec<AircraftSound> = [
            (1, Engine::Rotor, [300.0, 200.0, 0.0]),
            (2, Engine::Jet, [-3000.0, 2000.0, 0.0]),
            (3, Engine::Prop, [0.0, 400.0, -5000.0]),
        ]
        .into_iter()
        .filter_map(|(id, e, p)| {
            AircraftSound::placed(id, e, ear, [0.0, -1.0], p, [10.0, 0.0, 0.0], 1.0)
        })
        .collect();
        let pick = loudest(sounds);
        assert_eq!(pick.len(), 2);
        s.apply(&pick);
        let (l, r) = rms(&mut s, 48_000);
        assert!(l > 1e-3 && r > 1e-3, "{l} {r}");
        s.apply(&[]);
        rms(&mut s, 48_000);
        let (l, r) = rms(&mut s, 4800);
        assert!(l < 1e-4 && r < 1e-4, "{l} {r}");
    }
}
