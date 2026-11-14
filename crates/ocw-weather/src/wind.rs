//! Gusts (spec §14 Phase 9, 9E): the wind between its mean and its peak
//! gusts, as a smooth deterministic function of session time and the
//! session's seed – the server, every client (trees, rain, sound) and
//! later the physics (S6) see the same gust at the same moment.

use crate::lightning::unit;
use crate::state::WeatherState;

fn value_noise(seed: u64, octave: u64, x: f64) -> f32 {
    let i = x.floor();
    let f = (x - i) as f32;
    let a = unit(seed, i as i64 as u64, 500 + octave) as f32;
    let b = unit(seed, (i as i64 + 1) as u64, 500 + octave) as f32;
    let s = f * f * (3.0 - 2.0 * f);
    a + (b - a) * s
}

/// Gust envelope at session time `t_s` (0 = the mean wind, 1 = a peak
/// gust): slow swells (~15 s) with quicker gusts on top (~5 s and ~2 s),
/// shaped so that peaks are short and the mean wind is common.
pub fn gust(seed: u64, t_s: f64) -> f32 {
    let n = 0.55 * value_noise(seed, 0, t_s / 15.0)
        + 0.3 * value_noise(seed, 1, t_s / 5.0)
        + 0.15 * value_noise(seed, 2, t_s / 2.0);
    // n is about 0.5 ± 0.25: gusts are the upper part.
    ((n - 0.4) / 0.45).clamp(0.0, 1.0).powf(1.8)
}

/// Wind speed now (m/s): the mean plus the gust's share of the peak.
pub fn wind_now(w: &WeatherState, seed: u64, t_s: f64) -> f32 {
    w.wind_mps + (w.gust_mps - w.wind_mps).max(0.0) * gust(seed, t_s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gusts_are_smooth_bounded_and_seeded() {
        let w = WeatherState {
            wind_mps: 8.0,
            gust_mps: 20.0,
            ..Default::default()
        };
        let mut lo = f32::MAX;
        let mut hi = f32::MIN;
        let mut prev = wind_now(&w, 3, 0.0);
        let mut sum = 0.0;
        let n = 60_000;
        for i in 1..=n {
            let t = i as f64 * 0.02;
            let v = wind_now(&w, 3, t);
            assert!((8.0..=20.0).contains(&v));
            // At most a few m/s per 20 ms: smooth for trees and sound.
            assert!((v - prev).abs() < 0.5, "jump at {t}: {prev} -> {v}");
            prev = v;
            lo = lo.min(v);
            hi = hi.max(v);
            sum += v;
        }
        assert!(lo < 8.5 && hi > 15.0, "{lo} {hi}");
        // The mean wind is the common case; gusts are peaks.
        let mean = sum / n as f32;
        assert!(mean < 12.5, "{mean}");
        assert_eq!(gust(3, 123.4), gust(3, 123.4));
        assert_ne!(gust(3, 123.4), gust(4, 123.4));
        let calm = WeatherState::default();
        assert!(wind_now(&calm, 1, 10.0) <= calm.gust_mps);
    }
}
