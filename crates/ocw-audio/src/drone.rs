//! The ride photographer's drone (spec §2.11): the buzz of four small
//! rotors, synthesised – four detuned blade tones (a few harmonics each,
//! the blades' beating) and a breath of air noise – placed like the other
//! spatial sounds ([`crate::spatial::place`]: distance, direction, air
//! absorption, Doppler). The app sends a [`DroneSound`] every frame while
//! the drone flies (`MixerMsg::Drone`), `None` when it is gone.

use std::f32::consts::TAU;

use crate::dsp::{Noise, Svf};
use crate::spatial::Placement;

/// The blade tone at full speed (Hz) and the rotors' detuning.
pub const BLADE_HZ: f32 = 172.0;
const DETUNE: [f32; 4] = [1.0, 1.013, 0.987, 1.021];

/// How the drone is heard now.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DroneSound {
    /// Channel gains (equal power, from [`Placement`]).
    pub left: f32,
    pub right: f32,
    /// Rotor speed 0 (still) … 1 (flying).
    pub rotor: f32,
    /// Frequency factor (Doppler).
    pub doppler: f32,
    /// Low-pass for the air (Hz).
    pub cutoff_hz: f32,
}

impl DroneSound {
    /// The sound of a drone placed at `p` with rotors at `rotor`.
    pub fn placed(p: &Placement, rotor: f64) -> Self {
        Self {
            left: p.left,
            right: p.right,
            rotor: rotor.clamp(0.0, 1.0) as f32,
            doppler: p.doppler,
            cutoff_hz: p.air_cutoff_hz,
        }
    }

    /// Whether it makes a sound at all.
    pub fn audible(&self) -> bool {
        self.rotor > 0.0 && (self.left > 0.0 || self.right > 0.0)
    }
}

/// The drone's synthesis.
#[derive(Debug, Clone)]
pub struct DroneSynth {
    sr: f32,
    phases: [f32; 4],
    flutter: f32,
    noise: Noise,
    air: Svf,
    /// Smoothed gains and rotor speed (no clicks between frames).
    now: [f32; 3],
}

impl DroneSynth {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sr: sample_rate,
            phases: [0.0, 0.31, 0.57, 0.83],
            flutter: 0.0,
            noise: Noise::new(0xD20E),
            air: Svf::new(4000.0, 0.7, sample_rate),
            now: [0.0; 3],
        }
    }

    /// The next stereo sample for `s` (`None`: fading out).
    pub fn sample(&mut self, s: Option<&DroneSound>) -> (f32, f32) {
        let target = s.map_or([0.0, 0.0, 0.0], |s| [s.left, s.right, s.rotor]);
        let k = 1.0 - (-1.0 / (0.03 * self.sr)).exp();
        for (n, t) in self.now.iter_mut().zip(target) {
            *n += (t - *n) * k;
        }
        let [l, r, rotor] = self.now;
        if rotor < 1e-4 || (l < 1e-5 && r < 1e-5) {
            return (0.0, 0.0);
        }
        if let Some(s) = s {
            self.air
                .set(s.cutoff_hz.clamp(400.0, 16_000.0), 0.7, self.sr);
        }
        let doppler = s.map_or(1.0, |s| s.doppler);
        self.flutter = (self.flutter + 7.3 / self.sr).fract();
        let wobble = 1.0 + 0.012 * (TAU * self.flutter).sin();
        let mut x = 0.0;
        for (i, ph) in self.phases.iter_mut().enumerate() {
            let f = BLADE_HZ * rotor * DETUNE[i] * doppler * wobble;
            *ph = (*ph + f / self.sr).fract();
            let a = TAU * *ph;
            // A few harmonics: the blades' buzz.
            x += a.sin() + 0.5 * (2.0 * a).sin() + 0.33 * (3.0 * a).sin() + 0.2 * (5.0 * a).sin();
        }
        let air = self.noise.white() * 0.6;
        let y = self.air.process(0.05 * x + 0.04 * air).low * rotor;
        (y * l, y * r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(s: &mut DroneSynth, d: Option<&DroneSound>, n: usize) -> (f32, f32) {
        let (mut a, mut b) = (0.0, 0.0);
        for _ in 0..n {
            let (l, r) = s.sample(d);
            a += l * l;
            b += r * r;
        }
        ((a / n as f32).sqrt(), (b / n as f32).sqrt())
    }

    #[test]
    fn the_buzz_follows_the_drone() {
        let mut s = DroneSynth::new(48_000.0);
        // Still: silent.
        assert_eq!(rms(&mut s, None, 4800), (0.0, 0.0));
        let near_left = DroneSound {
            left: 0.9,
            right: 0.2,
            rotor: 1.0,
            doppler: 1.0,
            cutoff_hz: 12_000.0,
        };
        let (l, r) = rms(&mut s, Some(&near_left), 9600);
        assert!(l > 0.01 && l > 2.0 * r, "{l} {r}");
        // Far away: quieter.
        let far = DroneSound {
            left: 0.1,
            right: 0.1,
            ..near_left
        };
        rms(&mut s, Some(&far), 9600);
        let (fl, _) = rms(&mut s, Some(&far), 9600);
        assert!(fl < l * 0.3, "{fl} vs {l}");
        // Gone: it fades to silence.
        rms(&mut s, None, 9600);
        let (gl, gr) = rms(&mut s, None, 480);
        assert!(gl < 1e-3 && gr < 1e-3);
        assert!(
            !DroneSound {
                rotor: 0.0,
                ..near_left
            }
            .audible()
        );
        assert!(near_left.audible());
    }
}
