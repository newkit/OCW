//! Weather sounds (spec §3.3, §14 Phase 9, S4): rain by its intensity,
//! drizzle, hail drumming on the helmet, the wind's gusts, the hiss of
//! tyres on a wet road, and thunder – a crack for near strikes, a low
//! rumble rolling for seconds, arriving at the speed of sound after the
//! flash (the delay is the game's: the thunder is sent when it arrives).
//! All synthesised here (our own work, no samples), on the effects bus.
//!
//! [`weather_params`] maps the weather to the sound's targets (pure,
//! tested); [`WeatherSynth`] renders them, stereo-decorrelated.

use super::dsp::{Burst, Noise, Pink, Svf};
use ocw_weather::{Precip, WeatherState};

/// The weather sound's targets.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct WeatherParams {
    /// Rain on the ground and the leaves (0–1).
    pub rain: f32,
    /// Drops hitting the helmet per second.
    pub drop_rate_hz: f32,
    /// Drizzle's soft hiss (0–1).
    pub drizzle: f32,
    /// Hail stones on the helmet per second, and how hard (0–1).
    pub hail_rate_hz: f32,
    pub hail: f32,
    /// The wind's roar (0–1) and its brightness (Hz).
    pub wind: f32,
    pub wind_cutoff_hz: f32,
    /// Tyres on a wet road (0–1).
    pub wet_hiss: f32,
}

impl WeatherParams {
    /// One step of `dt_s` towards `t` (no clicks when the weather changes).
    pub fn approach(&mut self, t: &WeatherParams, dt_s: f32) {
        let a = 1.0 - (-dt_s / 0.4).exp();
        let f = |x: &mut f32, y: f32| *x += (y - *x) * a;
        f(&mut self.rain, t.rain);
        f(&mut self.drop_rate_hz, t.drop_rate_hz);
        f(&mut self.drizzle, t.drizzle);
        f(&mut self.hail_rate_hz, t.hail_rate_hz);
        f(&mut self.hail, t.hail);
        f(&mut self.wind, t.wind);
        f(&mut self.wind_cutoff_hz, t.wind_cutoff_hz);
        f(&mut self.wet_hiss, t.wet_hiss);
    }
}

/// The sound of weather `w` with the wind `wind_mps` now (gusts in), on a
/// road of `wetness` at `speed_mps`; `sheltered` (a tunnel) mutes the
/// rain and the wind.
pub fn weather_params(
    w: &WeatherState,
    wind_mps: f32,
    wetness: f32,
    speed_mps: f32,
    sheltered: bool,
) -> WeatherParams {
    let mm = if w.precipitating() {
        w.precip_mm_h
    } else {
        0.0
    };
    let open = if sheltered { 0.0 } else { 1.0 };
    let liquid = mm * (1.0 - w.precip.frozen_share(w.temperature_c));
    let rain = match w.precip {
        Precip::Rain | Precip::Sleet => ((liquid / 40.0).sqrt()).min(1.0),
        _ => 0.0,
    };
    let drizzle = if w.precip == Precip::Drizzle {
        (mm / 1.5).sqrt().min(1.0)
    } else {
        0.0
    };
    let hail = if w.precip == Precip::Hail {
        (mm / 30.0).sqrt().min(1.0)
    } else {
        0.0
    };
    let wind = ((wind_mps - 2.0).max(0.0) / 25.0).powf(1.2).min(1.0);
    WeatherParams {
        rain: rain * open,
        drop_rate_hz: (4.0 + 1.5 * liquid).min(60.0) * open * f32::from(rain > 0.0),
        drizzle: drizzle * open,
        hail_rate_hz: (6.0 + 1.2 * mm).min(55.0) * open * f32::from(hail > 0.0),
        hail: hail * open,
        wind: wind * open,
        wind_cutoff_hz: 180.0 + 700.0 * wind,
        wet_hiss: (wetness.clamp(0.0, 1.0) * ((speed_mps - 1.0) / 11.0).clamp(0.0, 1.0)).powf(0.8),
    }
}

/// A thunder to play.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThunderSound {
    pub loudness: f32,
    pub crack: bool,
    pub rumble_s: f32,
    pub cutoff_hz: f32,
    pub seed: u32,
}

impl ThunderSound {
    /// The thunder of a strike (`None` beyond hearing).
    pub fn of(strike: &ocw_weather::Strike) -> Option<Self> {
        ocw_weather::lightning::thunder(strike.distance_m, strike.intensity).map(|t| Self {
            loudness: t.loudness,
            crack: t.crack,
            rumble_s: t.rumble_s,
            cutoff_hz: t.cutoff_hz,
            seed: strike.seed,
        })
    }
}

/// One rolling thunder.
#[derive(Debug, Clone)]
struct ThunderVoice {
    t: ThunderSound,
    pos: usize,
    noise: Noise,
    low: Svf,
    low2: Svf,
    crack: Svf,
    /// The rumble's slow swells.
    swell: f32,
    swell_target: f32,
}

impl ThunderVoice {
    fn new(t: ThunderSound, sr: f32) -> Self {
        Self {
            t,
            pos: 0,
            noise: Noise::new(t.seed | 1),
            low: Svf::new(t.cutoff_hz, 0.7, sr),
            low2: Svf::new(t.cutoff_hz * 0.6, 0.7, sr),
            crack: Svf::new(2500.0, 0.6, sr),
            swell: 0.6,
            swell_target: 1.0,
        }
    }

    fn length_s(&self) -> f32 {
        self.t.rumble_s + 1.5
    }

    fn done(&self, sr: f32) -> bool {
        self.pos as f32 > self.length_s() * sr
    }

    fn sample(&mut self, sr: f32) -> f32 {
        let x = self.pos as f32 / sr;
        self.pos += 1;
        let n = self.noise.white();
        // Rolling: the loudness wanders between swells every ~0.2 s.
        if self.pos.is_multiple_of((sr as usize / 5).max(1)) {
            self.swell_target = 0.35 + 0.65 * self.noise.unit();
        }
        self.swell += (self.swell_target - self.swell) * (12.0 / sr);
        let attack = (x / 0.25).min(1.0);
        let decay = (-(x - 0.25).max(0.0) / (self.t.rumble_s * 0.45)).exp();
        let rumble = self.low2.process(self.low.process(n).low).low * 3.2;
        let mut s = rumble * attack * decay * self.swell;
        if self.t.crack && x < 0.4 {
            let hit = (x / 0.002).min(1.0) * (-x / 0.07).exp();
            s += self.crack.process(n).high * hit * 0.9;
        }
        s * self.t.loudness * 1.3
    }
}

/// The weather sounds' synthesis.
pub struct WeatherSynth {
    sr: f32,
    pink: [Pink; 2],
    white: [Noise; 2],
    rain_hp: [Svf; 2],
    rain_lp: [Svf; 2],
    drizzle_bp: [Svf; 2],
    wind_lp: [Svf; 2],
    hiss_bp: Svf,
    clicks: Noise,
    drop: Burst,
    drop_bp: Svf,
    hail_hit: Burst,
    hail_bp: Svf,
    hail_body: Svf,
    thunders: Vec<ThunderVoice>,
}

/// Rain's overall level against the ride sounds.
const RAIN_LEVEL: f32 = 0.38;
const WIND_LEVEL: f32 = 0.35;

impl WeatherSynth {
    pub fn new(sr: f32) -> Self {
        let pair = |f: f32, q: f32| [Svf::new(f, q, sr), Svf::new(f, q, sr)];
        Self {
            sr,
            pink: [Pink::new(0x4A11), Pink::new(0x77E1)],
            white: [Noise::new(0x9A1E), Noise::new(0x3C5D)],
            rain_hp: pair(900.0, 0.6),
            rain_lp: pair(7000.0, 0.6),
            drizzle_bp: pair(5000.0, 0.8),
            wind_lp: pair(400.0, 0.6),
            hiss_bp: Svf::new(4200.0, 1.2, sr),
            clicks: Noise::new(0xD20F),
            drop: Burst::default(),
            drop_bp: Svf::new(3200.0, 3.0, sr),
            hail_hit: Burst::default(),
            hail_bp: Svf::new(2600.0, 5.0, sr),
            hail_body: Svf::new(700.0, 2.0, sr),
            thunders: Vec::new(),
        }
    }

    /// Starts a thunder now.
    pub fn thunder(&mut self, t: ThunderSound) {
        if self.thunders.len() >= 6 {
            self.thunders.remove(0);
        }
        self.thunders.push(ThunderVoice::new(t, self.sr));
    }

    /// Thunders rolling now.
    #[cfg(test)]
    pub fn thunders(&self) -> usize {
        self.thunders.len()
    }

    /// Per block: the filters that follow the parameters.
    pub fn set_filters(&mut self, p: &WeatherParams) {
        for f in &mut self.wind_lp {
            f.set(p.wind_cutoff_hz.max(60.0), 0.6, self.sr);
        }
    }

    /// The next stereo sample.
    pub fn sample(&mut self, p: &WeatherParams) -> (f32, f32) {
        let sr = self.sr;
        let mut out = [0.0f32; 2];
        let quiet = p.rain < 1e-4
            && p.drizzle < 1e-4
            && p.hail < 1e-4
            && p.wind < 1e-4
            && p.wet_hiss < 1e-4;
        if !quiet {
            for (ch, o) in out.iter_mut().enumerate() {
                let w = self.white[ch].white();
                let pk = self.pink[ch].sample();
                // Rain: a broad wash of tiny impacts.
                let wash = self.rain_lp[ch]
                    .process(self.rain_hp[ch].process(0.6 * w + 0.4 * pk).high)
                    .low;
                *o += wash * p.rain * RAIN_LEVEL;
                // Drizzle: a soft, high hiss.
                *o += self.drizzle_bp[ch].process(w).band * p.drizzle * 0.08;
                // Wind: low, rolling.
                *o += self.wind_lp[ch].process(pk).low * p.wind * WIND_LEVEL * 2.0;
            }
            // Drops on the helmet (centre).
            if p.drop_rate_hz > 0.0 && self.clicks.unit() < p.drop_rate_hz / sr {
                let a = 0.3 + 0.7 * self.clicks.unit();
                self.drop.trigger(a, 0.004, sr);
            }
            let d = self.drop.step();
            let n = self.clicks.white();
            let drop = if d > 0.0 {
                self.drop_bp.process(n).band * d * 0.35 * p.rain.sqrt()
            } else {
                0.0
            };
            // Hail: hard knocks on the shell, a hollow body under them.
            if p.hail_rate_hz > 0.0 && self.clicks.unit() < p.hail_rate_hz / sr {
                let a = 0.4 + 0.6 * self.clicks.unit();
                self.hail_hit.trigger(a, 0.006, sr);
            }
            let h = self.hail_hit.step();
            let hail = if h > 0.0 {
                let x = n * h;
                (self.hail_bp.process(x).band * 0.9 + self.hail_body.process(x).band * 0.5) * p.hail
            } else {
                0.0
            };
            // Tyres on water.
            let hiss = self.hiss_bp.process(n).band * p.wet_hiss * 0.06;
            let centre = drop + hail + hiss;
            out[0] += centre;
            out[1] += centre;
        }
        if !self.thunders.is_empty() {
            let mut t = 0.0;
            for v in &mut self.thunders {
                t += v.sample(sr);
            }
            self.thunders.retain(|v| !v.done(sr));
            out[0] += t;
            out[1] += t;
        }
        (out[0], out[1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_weather::presets::preset;

    const SR: f32 = 48_000.0;

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    fn render(params: &WeatherParams, seconds: f32) -> Vec<f32> {
        let mut s = WeatherSynth::new(SR);
        s.set_filters(params);
        (0..(seconds * SR) as usize)
            .map(|_| {
                let (l, r) = s.sample(params);
                0.5 * (l + r)
            })
            .collect()
    }

    #[test]
    fn rain_grows_with_intensity_and_is_quiet_in_a_tunnel() {
        let light = weather_params(&preset("light-rain").unwrap(), 4.0, 1.0, 8.0, false);
        let heavy = weather_params(&preset("heavy-rain").unwrap(), 6.0, 1.0, 8.0, false);
        assert!(heavy.rain > light.rain && heavy.drop_rate_hz > light.drop_rate_hz);
        assert!(rms(&render(&heavy, 0.5)) > rms(&render(&light, 0.5)));
        let tunnel = weather_params(&preset("heavy-rain").unwrap(), 6.0, 1.0, 8.0, true);
        assert_eq!((tunnel.rain, tunnel.wind), (0.0, 0.0));
        let clear = weather_params(&preset("clear").unwrap(), 2.0, 0.0, 8.0, false);
        assert_eq!(
            clear,
            WeatherParams {
                wind_cutoff_hz: 180.0,
                ..Default::default()
            }
        );
        assert!(render(&clear, 0.2).iter().all(|&s| s == 0.0), "silence");
        // Snow falls silently.
        let snow = weather_params(&preset("snow").unwrap(), 3.0, 0.0, 8.0, false);
        assert_eq!(snow.rain + snow.hail + snow.drizzle, 0.0);
    }

    #[test]
    fn hail_drums_and_gusts_roar() {
        let hail = weather_params(&preset("hail").unwrap(), 10.0, 1.0, 8.0, false);
        assert!(hail.hail > 0.5 && hail.hail_rate_hz > 20.0);
        let out = render(&hail, 1.0);
        // Knocks: peaks well above the mean level.
        let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak > 4.0 * rms(&out), "{peak} {}", rms(&out));
        let calm = weather_params(&preset("clear").unwrap(), 2.0, 0.0, 0.0, false);
        let gale = weather_params(&preset("storm").unwrap(), 30.0, 0.0, 0.0, false);
        assert!(gale.wind > 0.8 && calm.wind == 0.0);
        assert!(gale.wind_cutoff_hz > calm.wind_cutoff_hz);
        let wet = weather_params(&preset("clear").unwrap(), 2.0, 1.0, 10.0, false);
        assert!(wet.wet_hiss > 0.5);
        let mut p = WeatherParams::default();
        p.approach(&gale, 10.0);
        assert!((p.wind - gale.wind).abs() < 1e-3);
    }

    /// Thunder: near ones crack and are loud, far ones rumble low; each
    /// rolls for its length and ends.
    #[test]
    fn thunder_cracks_near_rumbles_far_and_ends() {
        let strike = |d: f32| ocw_weather::Strike {
            id: 1,
            at_s: 0.0,
            distance_m: d,
            bearing_deg: 0.0,
            intensity: 1.0,
            seed: 9,
        };
        let near = ThunderSound::of(&strike(500.0)).unwrap();
        let far = ThunderSound::of(&strike(9000.0)).unwrap();
        assert!(near.crack && !far.crack);
        assert!(ThunderSound::of(&strike(30_000.0)).is_none());
        let play = |t: ThunderSound, seconds: f32| {
            let mut s = WeatherSynth::new(SR);
            s.thunder(t);
            let quiet = WeatherParams::default();
            let out: Vec<f32> = (0..(seconds * SR) as usize)
                .map(|_| s.sample(&quiet).0)
                .collect();
            (out, s.thunders())
        };
        let (n, _) = play(near, 1.0);
        let (f, _) = play(far, 1.0);
        assert!(rms(&n) > 2.0 * rms(&f), "{} {}", rms(&n), rms(&f));
        assert!(n.iter().all(|v| v.is_finite() && v.abs() < 1.5));
        let (_, left) = play(far, far.rumble_s + 2.0);
        assert_eq!(left, 0, "over");
    }
}
