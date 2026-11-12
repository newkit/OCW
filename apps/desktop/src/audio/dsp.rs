//! Small DSP building blocks for the procedurally synthesised sounds:
//! deterministic noise, a state-variable filter, a smoother and a short
//! decaying burst. Pure; the same seed gives the same samples (tests and
//! the demo WAV repeat exactly).

use std::f32::consts::PI;

/// White noise from a xorshift32 generator.
#[derive(Debug, Clone)]
pub struct Noise(u32);

impl Noise {
    pub fn new(seed: u32) -> Self {
        Self(seed.max(1))
    }

    /// Uniform in [-1, 1].
    pub fn white(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    /// Uniform in [0, 1].
    pub fn unit(&mut self) -> f32 {
        (self.white() + 1.0) * 0.5
    }
}

/// Pink-ish noise (-3 dB/octave): Paul Kellet's economy filter over white
/// noise. Wind and air sound natural with it, white noise hisses.
#[derive(Debug, Clone)]
pub struct Pink {
    noise: Noise,
    b: [f32; 3],
}

impl Pink {
    pub fn new(seed: u32) -> Self {
        Self {
            noise: Noise::new(seed),
            b: [0.0; 3],
        }
    }

    /// Roughly in [-1, 1].
    pub fn sample(&mut self) -> f32 {
        let w = self.noise.white();
        self.b[0] = 0.99765 * self.b[0] + w * 0.099_046;
        self.b[1] = 0.96300 * self.b[1] + w * 0.296_516_4;
        self.b[2] = 0.57000 * self.b[2] + w * 1.052_691_3;
        (self.b[0] + self.b[1] + self.b[2] + w * 0.1848) * 0.2
    }
}

/// The three outputs of [`Svf`].
#[derive(Debug, Clone, Copy, Default)]
pub struct SvfOut {
    pub low: f32,
    pub band: f32,
    pub high: f32,
}

/// A state-variable filter (topology-preserving transform, Zavalishin):
/// stays stable while its cutoff moves, which the speed-dependent sounds
/// do all the time.
#[derive(Debug, Clone, Default)]
pub struct Svf {
    ic1: f32,
    ic2: f32,
    a1: f32,
    a2: f32,
    a3: f32,
    k: f32,
}

impl Svf {
    pub fn new(cutoff_hz: f32, q: f32, sample_rate: f32) -> Self {
        let mut f = Self::default();
        f.set(cutoff_hz, q, sample_rate);
        f
    }

    /// Sets cutoff (clamped below Nyquist) and resonance `q` (0.5 = soft,
    /// higher = narrower band).
    pub fn set(&mut self, cutoff_hz: f32, q: f32, sample_rate: f32) {
        let f = (cutoff_hz / sample_rate).clamp(1e-5, 0.49);
        let g = (PI * f).tan();
        self.k = 1.0 / q.max(0.05);
        self.a1 = 1.0 / (1.0 + g * (g + self.k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
    }

    pub fn process(&mut self, x: f32) -> SvfOut {
        let v3 = x - self.ic2;
        let v1 = self.a1 * self.ic1 + self.a2 * v3;
        let v2 = self.ic2 + self.a2 * self.ic1 + self.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        SvfOut {
            low: v2,
            band: v1,
            high: x - self.k * v1 - v2,
        }
    }
}

/// The per-sample coefficient of a one-pole smoother with time constant
/// `tau_s`, called every `every` samples.
pub fn smoothing(tau_s: f32, sample_rate: f32, every: usize) -> f32 {
    1.0 - (-(every as f32) / (tau_s * sample_rate)).exp()
}

/// An exponentially decaying burst (a click, a gravel stone): starts at
/// `amp` and falls by 1/e every `tau_s`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Burst {
    amp: f32,
    decay: f32,
}

impl Burst {
    pub fn trigger(&mut self, amp: f32, tau_s: f32, sample_rate: f32) {
        self.amp = self.amp.max(amp);
        self.decay = (-1.0 / (tau_s * sample_rate)).exp();
    }

    /// The envelope now; then one sample on.
    pub fn step(&mut self) -> f32 {
        let a = self.amp;
        self.amp *= self.decay;
        if self.amp < 1e-5 {
            self.amp = 0.0;
        }
        a
    }
}

/// A sine oscillator's next sample at `freq_hz`; `phase` in turns (0–1).
pub fn sine(phase: &mut f32, freq_hz: f32, sample_rate: f32) -> f32 {
    let s = (2.0 * PI * *phase).sin();
    *phase = (*phase + freq_hz / sample_rate).fract();
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    #[test]
    fn noise_is_deterministic_and_bounded() {
        let a: Vec<f32> = {
            let mut n = Noise::new(7);
            (0..1000).map(|_| n.white()).collect()
        };
        let mut n = Noise::new(7);
        assert!(a.iter().all(|&v| v == n.white()));
        assert!(a.iter().all(|v| (-1.0..=1.0).contains(v)));
        let mean = a.iter().sum::<f32>() / a.len() as f32;
        assert!(mean.abs() < 0.1, "{mean}");
        assert!(Noise::new(0).white() != 0.0, "seed 0 is not stuck");
        let mut p = Pink::new(3);
        let pink: Vec<f32> = (0..48_000).map(|_| p.sample()).collect();
        assert!(pink.iter().all(|v| v.abs() < 1.5));
        assert!(rms(&pink) > 0.05);
    }

    /// A lowpass passes a low tone and damps a high one; a highpass the
    /// other way round.
    #[test]
    fn svf_filters_by_frequency() {
        let tone = |f: f32| -> Vec<f32> {
            let mut ph = 0.0;
            (0..9600).map(|_| sine(&mut ph, f, SR)).collect()
        };
        let through = |x: &[f32], pick: fn(SvfOut) -> f32| -> f32 {
            let mut svf = Svf::new(1000.0, 0.707, SR);
            let y: Vec<f32> = x.iter().map(|&v| pick(svf.process(v))).collect();
            rms(&y[2000..]) / rms(&x[2000..])
        };
        let (low, high) = (tone(100.0), tone(10_000.0));
        assert!(through(&low, |o| o.low) > 0.95);
        assert!(through(&high, |o| o.low) < 0.02);
        assert!(through(&high, |o| o.high) > 0.95);
        assert!(through(&low, |o| o.high) < 0.02);
        // Band: the cutoff passes, far away is damped.
        assert!(through(&tone(1000.0), |o| o.band) > 0.6);
        assert!(through(&tone(20.0), |o| o.band) < 0.05);
        // A cutoff above Nyquist is clamped, not unstable.
        let mut svf = Svf::new(40_000.0, 10.0, SR);
        let mut n = Noise::new(1);
        assert!((0..48_000).all(|_| svf.process(n.white()).low.is_finite()));
    }

    #[test]
    fn bursts_decay_and_smoothing_is_in_range() {
        let mut b = Burst::default();
        assert_eq!(b.step(), 0.0);
        b.trigger(1.0, 0.001, SR);
        assert_eq!(b.step(), 1.0);
        for _ in 0..47 {
            b.step();
        }
        // One time constant (48 samples) later: about 1/e.
        assert!((b.step() - (-1.0f32).exp()).abs() < 0.01);
        for _ in 0..2000 {
            b.step();
        }
        assert_eq!(b.step(), 0.0);
        let a = smoothing(0.05, SR, 32);
        assert!(a > 0.0 && a < 0.05, "{a}");
    }
}
