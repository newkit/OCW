//! The sounds of the road vehicles (spec 14 Phase 18, 18A *Sound*, 18E
//! T7): each vehicle's tyre roar and engine by its class and speed – a
//! car's petrol hum, a van's, a lorry's and a bus's diesel (louder), a
//! motorbike's rasp, a tractor's slow thump – placed where it is, fading
//! with the distance, dulled by the air, with the Doppler shift of its
//! speed toward the listener (the full physical one, within ±15 %: a car
//! passing at 80 km/h drops about two semitones). At most
//! [`VOICES`] vehicles are heard at once (18A: the eight nearest), the
//! loudest; one that keeps its voice is not swapped for a slightly louder
//! one. The pass-by whoosh, the honks and the cue of a vehicle closing in
//! from behind are one-shot cues ([`crate::cues::Cue`]).

use std::f32::consts::TAU;

use crate::dsp::{Noise, Svf};
use crate::spatial::Listener;

/// Vehicles heard at once (spec 18A: the 8 nearest vehicle voices).
pub const VOICES: usize = 8;
/// Silent beyond this (m) …
pub const MAX_M: f64 = 320.0;
/// … fading from here.
pub const FADE_M: f64 = 220.0;
/// The speed of sound (m/s).
const C: f64 = 343.0;
/// The Doppler factor stays within this.
pub const DOPPLER_RANGE: (f64, f64) = (0.85, 1.15);
/// A voice keeps its vehicle unless another is this much louder.
pub const KEEP_FACTOR: f32 = 1.5;

/// What kind of noise it makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Car,
    Van,
    Lorry,
    Bus,
    Motorbike,
    Tractor,
}

impl Kind {
    /// The distance (m) up to which it plays at full level, the
    /// inverse-distance law beyond: the louder the vehicle, the farther.
    pub fn reference_m(self) -> f64 {
        match self {
            Self::Car => 5.0,
            Self::Van => 6.5,
            Self::Lorry => 14.0,
            Self::Bus => 11.0,
            Self::Motorbike => 12.0,
            Self::Tractor => 10.0,
        }
    }

    /// The engine's firing rate at idle and its rise per m/s (Hz), the
    /// speed span of one gear (m/s).
    fn engine(self) -> (f32, f32, f32) {
        match self {
            Self::Car => (34.0, 1.9, 9.0),
            Self::Van => (30.0, 1.6, 8.0),
            Self::Lorry => (22.0, 1.1, 5.5),
            Self::Bus => (24.0, 1.2, 6.0),
            Self::Motorbike => (55.0, 4.2, 11.0),
            Self::Tractor => (16.0, 2.0, 3.0),
        }
    }
}

/// One vehicle as it is heard now.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VehicleSound {
    pub id: u32,
    pub kind: Kind,
    pub left: f32,
    pub right: f32,
    /// Frequency factor (Doppler).
    pub doppler: f32,
    /// Low-pass of the air (Hz).
    pub cutoff_hz: f32,
    /// Its speed (m/s): the engine's pitch and the tyres' roar.
    pub speed_mps: f32,
    /// A wet road: the tyres hiss.
    pub wet: bool,
}

/// The overall level of the vehicles (a car at its reference distance
/// is as loud as the own bike's wind at 30 km/h).
pub const LEVEL: f64 = 0.45;

impl VehicleSound {
    /// The sound of a vehicle at `pos` moving at `vel` (m/s, x y z) for
    /// `listener`; `mask` (0 – 1) lowers everything under wind and rain;
    /// `None` when it cannot be heard.
    #[allow(clippy::too_many_arguments)]
    pub fn placed(
        id: u32,
        kind: Kind,
        listener: &Listener,
        pos: [f64; 3],
        vel: [f64; 3],
        wet: bool,
        mask: f64,
    ) -> Option<Self> {
        let d = [
            pos[0] - listener.pos[0],
            pos[1] - listener.pos[1],
            pos[2] - listener.pos[2],
        ];
        let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        if !dist.is_finite() || dist >= MAX_M {
            return None;
        }
        let gain = attenuation(kind, dist) * LEVEL * mask.clamp(0.0, 1.0);
        if gain < 1e-4 {
            return None;
        }
        let f = {
            let l = listener.forward[0].hypot(listener.forward[1]);
            if l > 1e-9 {
                [listener.forward[0] / l, listener.forward[1] / l]
            } else {
                [0.0, -1.0]
            }
        };
        let right = [-f[1], f[0]];
        let flat = d[0].hypot(d[2]);
        // Narrow the pan right beside the head.
        let pan = if flat > 1e-6 {
            ((d[0] * right[0] + d[2] * right[1]) / flat * (flat / 1.5).min(1.0)).clamp(-1.0, 1.0)
        } else {
            0.0
        };
        let behind = if flat > 1e-6 {
            (-(d[0] * f[0] + d[2] * f[1]) / flat).max(0.0)
        } else {
            0.0
        };
        let g = gain * (1.0 - 0.2 * behind);
        let doppler = doppler(listener, pos, vel);
        let speed = (vel[0] * vel[0] + vel[1] * vel[1] + vel[2] * vel[2]).sqrt();
        Some(Self {
            id,
            kind,
            left: (g * (1.0 - pan).max(0.0).sqrt()) as f32,
            right: (g * (1.0 + pan).max(0.0).sqrt()) as f32,
            doppler: doppler as f32,
            cutoff_hz: (14_000.0 * 30.0 / (30.0 + dist) * (1.0 - 0.3 * behind)).max(250.0) as f32,
            speed_mps: speed as f32,
            wet,
        })
    }

    /// The loudness used to pick the voices.
    pub fn level(&self) -> f32 {
        self.left.max(self.right)
    }
}

/// The level by distance: 1 up to the kind's reference distance, then
/// inverse with the distance, faded to exactly 0 between [`FADE_M`] and
/// [`MAX_M`].
pub fn attenuation(kind: Kind, dist: f64) -> f64 {
    if !dist.is_finite() || dist >= MAX_M {
        return 0.0;
    }
    let r = kind.reference_m();
    let inv = r / dist.max(r);
    let fade = if dist <= FADE_M {
        1.0
    } else {
        let x = (dist - FADE_M) / (MAX_M - FADE_M);
        1.0 - x * x * (3.0 - 2.0 * x)
    };
    inv * fade
}

/// The Doppler factor of a source at `pos` moving at `vel` for
/// `listener` (moving too): `(c + v_listener) / (c − v_source)` toward
/// each other, within [`DOPPLER_RANGE`].
pub fn doppler(listener: &Listener, pos: [f64; 3], vel: [f64; 3]) -> f64 {
    let d = [
        pos[0] - listener.pos[0],
        pos[1] - listener.pos[1],
        pos[2] - listener.pos[2],
    ];
    let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    if dist < 1e-6 {
        return 1.0;
    }
    let u = [d[0] / dist, d[1] / dist, d[2] / dist];
    let towards_source =
        listener.velocity[0] * u[0] + listener.velocity[1] * u[1] + listener.velocity[2] * u[2];
    let towards_listener = -(vel[0] * u[0] + vel[1] * u[1] + vel[2] * u[2]);
    ((C + towards_source) / (C - towards_listener.min(C * 0.5)))
        .clamp(DOPPLER_RANGE.0, DOPPLER_RANGE.1)
}

/// The `VOICES` loudest of `sounds`, keeping the ones `playing` unless a
/// new one is [`KEEP_FACTOR`] louder than the quietest kept.
pub fn select(mut sounds: Vec<VehicleSound>, playing: &[u32]) -> Vec<VehicleSound> {
    let rank = |s: &VehicleSound| {
        if playing.contains(&s.id) {
            s.level() * KEEP_FACTOR
        } else {
            s.level()
        }
    };
    sounds.sort_by(|a, b| rank(b).total_cmp(&rank(a)).then(a.id.cmp(&b.id)));
    sounds.truncate(VOICES);
    sounds
}

/// One voice's synthesis.
#[derive(Debug, Clone)]
struct Voice {
    id: Option<u32>,
    kind: Kind,
    noise: Noise,
    tyre: Svf,
    hiss: Svf,
    body: Svf,
    air: Svf,
    phase: f32,
    knock: f32,
    /// Smoothed left, right, doppler, cutoff, speed.
    now: [f32; 5],
    target: Option<VehicleSound>,
}

impl Voice {
    fn new(seed: u32, sr: f32) -> Self {
        Self {
            id: None,
            kind: Kind::Car,
            noise: Noise::new(seed),
            tyre: Svf::new(700.0, 0.8, sr),
            hiss: Svf::new(3500.0, 0.9, sr),
            body: Svf::new(120.0, 1.0, sr),
            air: Svf::new(8000.0, 0.7, sr),
            phase: 0.0,
            knock: 0.0,
            now: [0.0, 0.0, 1.0, 8000.0, 0.0],
            target: None,
        }
    }

    fn sample(&mut self, sr: f32) -> (f32, f32) {
        let k = 1.0 - (-1.0 / (0.06 * sr)).exp();
        let t = self
            .target
            .map_or([0.0, 0.0, self.now[2], self.now[3], self.now[4]], |s| {
                [s.left, s.right, s.doppler, s.cutoff_hz, s.speed_mps]
            });
        for (n, t) in self.now.iter_mut().zip(t) {
            *n += (t - *n) * k;
        }
        let [l, r, doppler, cutoff, speed] = self.now;
        if l < 1e-5 && r < 1e-5 {
            return (0.0, 0.0);
        }
        let wet = self.target.is_some_and(|s| s.wet);
        let w = self.noise.white();
        // The tyres: a roar growing with the speed, a hiss on a wet road.
        let roll = (speed / 25.0).clamp(0.0, 1.6);
        self.tyre.set(500.0 + 40.0 * speed * doppler, 0.7, sr);
        let mut tyres = self.tyre.process(w).band * 0.9 * roll.powf(1.5);
        if wet {
            tyres += self.hiss.process(w).band * 0.7 * roll;
        }
        // The engine: firing pulses rising through the gears.
        let (idle, rise, span) = self.kind.engine();
        let gear = (speed / span).fract();
        let f0 = (idle + rise * span * (0.35 + 0.65 * gear)) * doppler;
        self.phase = (self.phase + f0 / sr).fract();
        let a = TAU * self.phase;
        let engine = match self.kind {
            Kind::Motorbike => {
                // A rasp: a bright saw with odd harmonics.
                let saw = 2.0 * self.phase - 1.0;
                0.22 * saw + 0.08 * (3.0 * a).sin() + 0.03 * w
            }
            Kind::Lorry | Kind::Bus | Kind::Tractor => {
                // Diesel: a low rumble with a knock on each firing.
                if self.phase < f0 / sr {
                    self.knock = 1.0;
                }
                self.knock *= 1.0 - 40.0 / sr;
                self.body.set(90.0 * doppler, 1.1, sr);
                let rumble = self.body.process(w).band;
                0.35 * a.sin() + 0.12 * (2.0 * a).sin() + 0.5 * rumble * (0.3 + self.knock)
            }
            Kind::Car | Kind::Van => {
                0.2 * a.sin() + 0.07 * (2.0 * a).sin() + 0.03 * (4.0 * a).sin()
            }
        };
        let engine_level = match self.kind {
            Kind::Car => 0.35,
            Kind::Van => 0.45,
            Kind::Lorry | Kind::Bus => 0.8,
            Kind::Motorbike => 0.9,
            Kind::Tractor => 0.9,
        };
        self.air.set(cutoff.clamp(150.0, 16_000.0), 0.7, sr);
        let y = self.air.process(tyres + engine * engine_level).low;
        (y * l, y * r)
    }
}

/// The vehicle voices.
#[derive(Debug, Clone)]
pub struct VehicleSynth {
    sr: f32,
    voices: Vec<Voice>,
}

impl VehicleSynth {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sr: sample_rate,
            voices: (0..VOICES as u32)
                .map(|i| Voice::new(0x7EA0 + i, sample_rate))
                .collect(),
        }
    }

    /// The vehicles that have a voice now.
    pub fn playing(&self) -> Vec<u32> {
        self.voices.iter().filter_map(|v| v.id).collect()
    }

    /// This frame's sounds (at most [`VOICES`]; a vehicle keeps its voice
    /// while it is heard, a new one fades in).
    pub fn apply(&mut self, sounds: &[VehicleSound]) {
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
            if let Some(v) = self
                .voices
                .iter_mut()
                .filter(|v| v.id.is_none())
                .min_by(|a, b| (a.now[0] + a.now[1]).total_cmp(&(b.now[0] + b.now[1])))
            {
                v.id = Some(s.id);
                v.kind = s.kind;
                v.target = Some(*s);
                v.now = [0.0, 0.0, s.doppler, s.cutoff_hz, s.speed_mps];
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

    fn ear() -> Listener {
        Listener {
            pos: [0.0, 1.5, 0.0],
            forward: [0.0, -1.0],
            velocity: [0.0; 3],
        }
    }

    fn rms(s: &mut VehicleSynth, n: usize) -> (f32, f32) {
        let (mut a, mut b) = (0.0, 0.0);
        for _ in 0..n {
            let (l, r) = s.sample();
            a += l * l;
            b += r * r;
        }
        ((a / n as f32).sqrt(), (b / n as f32).sqrt())
    }

    #[test]
    fn doppler_of_a_car_passing_at_80() {
        let v = 80.0 / 3.6;
        // Coming straight at the listener from 100 m ahead, then going
        // away behind: c / (c − v) and c / (c + v).
        let coming = doppler(&ear(), [0.0, 1.5, -100.0], [0.0, 0.0, v]);
        let going = doppler(&ear(), [0.0, 1.5, 100.0], [0.0, 0.0, v]);
        assert!((coming - C / (C - v)).abs() < 1e-9, "{coming}");
        assert!((going - C / (C + v)).abs() < 1e-9, "{going}");
        // About 1.07 and 0.94: a drop of ~2.2 semitones as it passes.
        let semitones = 12.0 * (coming / going).log2();
        assert!((2.0..2.5).contains(&semitones), "{semitones}");
        // Beside: none.
        assert!((doppler(&ear(), [3.0, 1.5, 0.0], [0.0, 0.0, v]) - 1.0).abs() < 1e-9);
        // A riding listener toward a standing source hears it higher.
        let riding = Listener {
            velocity: [0.0, 0.0, -10.0],
            ..ear()
        };
        assert!(doppler(&riding, [0.0, 1.5, -50.0], [0.0; 3]) > 1.02);
        // Clamped.
        assert_eq!(
            doppler(&ear(), [0.0, 1.5, -100.0], [0.0, 0.0, 120.0]),
            DOPPLER_RANGE.1
        );
    }

    #[test]
    fn attenuation_by_distance_and_class() {
        // Full up to the reference, inverse beyond, nothing at the end.
        assert_eq!(attenuation(Kind::Car, 3.0), 1.0);
        assert!((attenuation(Kind::Car, 10.0) - 0.5).abs() < 1e-9);
        assert!((attenuation(Kind::Car, 50.0) - 0.1).abs() < 1e-9);
        assert_eq!(attenuation(Kind::Car, MAX_M), 0.0);
        assert!(attenuation(Kind::Car, 250.0) < 5.0 / 250.0);
        // A lorry is louder than a car at the same distance (about 9 dB).
        let db = 20.0 * (attenuation(Kind::Lorry, 40.0) / attenuation(Kind::Car, 40.0)).log10();
        assert!((8.0..10.0).contains(&db), "{db}");
        assert!(attenuation(Kind::Motorbike, 40.0) > attenuation(Kind::Van, 40.0));
        assert!(attenuation(Kind::Bus, 40.0) > attenuation(Kind::Car, 40.0));
    }

    #[test]
    fn placed_left_right_and_quieter_under_rain() {
        // A car 10 m to the right.
        let s = VehicleSound::placed(1, Kind::Car, &ear(), [10.0, 0.5, 0.0], [0.0; 3], false, 1.0)
            .unwrap();
        assert!(s.right > 3.0 * s.left, "{s:?}");
        let masked =
            VehicleSound::placed(1, Kind::Car, &ear(), [10.0, 0.5, 0.0], [0.0; 3], false, 0.5)
                .unwrap();
        assert!((masked.level() - s.level() * 0.5).abs() < 1e-6);
        assert!(
            VehicleSound::placed(
                1,
                Kind::Car,
                &ear(),
                [400.0, 0.5, 0.0],
                [0.0; 3],
                false,
                1.0
            )
            .is_none()
        );
        // Far is duller.
        let far = VehicleSound::placed(
            1,
            Kind::Car,
            &ear(),
            [150.0, 0.5, 0.0],
            [0.0; 3],
            false,
            1.0,
        )
        .unwrap();
        assert!(far.cutoff_hz < s.cutoff_hz / 3.0);
    }

    #[test]
    fn eight_voices_at_most_and_kept() {
        let sounds: Vec<VehicleSound> = (0..20u32)
            .filter_map(|i| {
                VehicleSound::placed(
                    i,
                    Kind::Car,
                    &ear(),
                    [3.0 + f64::from(i) * 8.0, 0.5, -10.0],
                    [0.0, 0.0, 15.0],
                    false,
                    1.0,
                )
            })
            .collect();
        assert_eq!(sounds.len(), 20);
        let pick = select(sounds.clone(), &[]);
        assert_eq!(pick.len(), VOICES);
        // The nearest eight.
        let ids: Vec<u32> = pick.iter().map(|s| s.id).collect();
        assert_eq!(ids, (0..8).collect::<Vec<_>>());
        // Number 9 playing keeps its voice against the slightly louder 7.
        let keep = select(sounds, &[9]);
        assert!(keep.iter().any(|s| s.id == 9));
        assert!(!keep.iter().any(|s| s.id == 7));
        let mut synth = VehicleSynth::new(48_000.0);
        synth.apply(&pick);
        assert_eq!(synth.playing().len(), VOICES);
        let (l, r) = rms(&mut synth, 24_000);
        assert!(l > 1e-3 && r > l, "{l} {r}");
        // Gone: fades out.
        synth.apply(&[]);
        assert!(synth.playing().is_empty());
        rms(&mut synth, 24_000);
        let (l, r) = rms(&mut synth, 4800);
        assert!(l < 1e-4 && r < 1e-4, "{l} {r}");
    }

    #[test]
    fn lorries_and_motorbikes_are_louder_than_cars() {
        let loud = |kind| {
            let mut s = VehicleSynth::new(48_000.0);
            let v = VehicleSound::placed(
                1,
                kind,
                &ear(),
                [0.0, 0.5, -20.0],
                [0.0, 0.0, 15.0],
                false,
                1.0,
            )
            .unwrap();
            s.apply(&[v]);
            rms(&mut s, 12_000);
            let (l, r) = rms(&mut s, 24_000);
            (l + r) / 2.0
        };
        let car = loud(Kind::Car);
        assert!(car > 1e-3);
        assert!(loud(Kind::Lorry) > 1.5 * car);
        assert!(loud(Kind::Motorbike) > 1.5 * car);
        assert!(loud(Kind::Bus) > car);
    }
}
