//! The plausibility checks of spec 6.4 for an uploaded ride summary
//! (Q19 (a), Q20 (a)). Deterministic; the backend runs them before it
//! credits a summary's hearts, the app may run them first to say early
//! that a ride will be held.
//!
//! - **Refused** (no hearts, nothing to review): a malformed summary
//!   (no samples, lengths that do not match, a start in the future or
//!   older than [`MAX_AGE_S`], non-finite numbers), virtual power only, a
//!   power source missing.
//! - **Flagged** (kept, hearts held until reviewed, 6.4.5): the power-
//!   duration curve above absolute W/kg limits ([`CURVE_LIMITS`]), single
//!   samples above [`MAX_POWER_W`], jumps faster than [`MAX_STEP_W`] a
//!   second, power without cadence (or no cadence at all), perfectly
//!   constant power for [`CONSTANT_S`], a heart rate that does not follow
//!   the load, work that does not match the samples, an impossible speed
//!   or climb.
//! - Otherwise **credited**: [`ocw_core::hearts::earned`] of the summary's
//!   work and climbing.

use crate::rides::{DeviceKind, RideSummary};

/// The power-duration limits (window in s, W/kg): above world-class
/// values, so a strong rider is never flagged by them.
pub const CURVE_LIMITS: [(usize, f64); 5] =
    [(5, 25.0), (60, 12.0), (300, 7.5), (1200, 6.6), (3600, 6.2)];
/// No sample above this (W).
pub const MAX_POWER_W: u16 = 2500;
/// No jump larger than this from one second to the next (W).
pub const MAX_STEP_W: u16 = 1500;
/// Power without cadence: power ≥ this (W) …
pub const NO_CADENCE_POWER_W: u16 = 150;
/// … with cadence 0 for more than this many seconds.
pub const NO_CADENCE_S: usize = 60;
/// The same non-zero power for this long (s) is not a real sensor.
pub const CONSTANT_S: usize = 600;
/// The longest ride (s).
pub const MAX_DURATION_S: u32 = 12 * 3600;
/// The oldest ride uploaded (s, 30 days: a long time offline).
pub const MAX_AGE_S: i64 = 30 * 86_400;
/// The fastest average speed (m/s, 90 km/h).
pub const MAX_SPEED_MS: f64 = 25.0;

/// The checks' result.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Verdict {
    /// Reasons the summary is refused.
    pub refused: Vec<String>,
    /// Reasons it is held for review.
    pub flags: Vec<String>,
    /// The hearts it is worth if it passes.
    pub hearts: u32,
}

impl Verdict {
    pub fn passed(&self) -> bool {
        self.refused.is_empty() && self.flags.is_empty()
    }
}

/// The best mean power over `window` seconds.
pub fn best_mean(power: &[u16], window: usize) -> Option<f64> {
    if window == 0 || power.len() < window {
        return None;
    }
    let mut sum: u64 = power[..window].iter().map(|p| u64::from(*p)).sum();
    let mut best = sum;
    for i in window..power.len() {
        sum = sum + u64::from(power[i]) - u64::from(power[i - window]);
        best = best.max(sum);
    }
    Some(best as f64 / window as f64)
}

fn mean_sd(v: &[f64]) -> (f64, f64) {
    if v.is_empty() {
        return (0.0, 0.0);
    }
    let n = v.len() as f64;
    let m = v.iter().sum::<f64>() / n;
    let var = v.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / n;
    (m, var.sqrt())
}

/// Checks a summary at `now_unix_s`.
pub fn check(s: &RideSummary, now_unix_s: i64) -> Verdict {
    let mut v = Verdict::default();
    let n = s.power_w.len();
    // --- Refused: malformed or not eligible.
    if s.ride_id.trim().is_empty() || s.ride_id.len() > 128 {
        v.refused
            .push("the ride needs an id (at most 128 characters)".into());
    }
    if ![s.distance_m, s.climb_m, s.work_j, s.weight_kg]
        .iter()
        .all(|x| x.is_finite() && *x >= 0.0)
    {
        v.refused.push("a number is negative or not finite".into());
    }
    if s.duration_s == 0 || s.duration_s > MAX_DURATION_S {
        v.refused.push(format!(
            "duration {} s is outside 1 s – {} h",
            s.duration_s,
            MAX_DURATION_S / 3600
        ));
    }
    if n == 0 {
        v.refused.push("no power samples".into());
    } else if n.abs_diff(s.duration_s as usize) > 2 {
        v.refused.push(format!(
            "{n} power samples for a ride of {} s",
            s.duration_s
        ));
    }
    for (name, stream) in [("cadence", &s.cadence_rpm), ("heart rate", &s.heart_bpm)] {
        if !stream.is_empty() && stream.len() != n {
            v.refused.push(format!(
                "{} {name} samples for {n} power samples",
                stream.len()
            ));
        }
    }
    if s.started_unix_s > now_unix_s + 300 {
        v.refused.push("the ride starts in the future".into());
    }
    if s.started_unix_s < now_unix_s - MAX_AGE_S {
        v.refused.push(format!(
            "the ride is older than {} days",
            MAX_AGE_S / 86_400
        ));
    }
    let real_power = s
        .devices
        .iter()
        .any(|d| matches!(d.kind, DeviceKind::Trainer | DeviceKind::PowerMeter));
    if s.devices.iter().any(|d| d.kind == DeviceKind::Virtual) {
        v.refused
            .push("virtual power earns no hearts (spec 6.4.6)".into());
    } else if !real_power {
        v.refused
            .push("no power source: a trainer or a power meter is needed".into());
    }
    if !(30.0..=250.0).contains(&s.weight_kg) {
        v.refused.push(format!(
            "weight {:.0} kg is outside 30 – 250 kg",
            s.weight_kg
        ));
    }
    if !v.refused.is_empty() {
        return v;
    }

    // --- Flagged: implausible, held for review.
    let p = &s.power_w;
    for (window, limit) in CURVE_LIMITS {
        if let Some(best) = best_mean(p, window) {
            let wkg = best / s.weight_kg;
            if wkg > limit {
                v.flags.push(format!(
                    "best {} power {best:.0} W = {wkg:.1} W/kg (limit {limit} W/kg)",
                    if window < 60 {
                        format!("{window} s")
                    } else {
                        format!("{} min", window / 60)
                    }
                ));
            }
        }
    }
    if let Some(m) = p.iter().max().filter(|m| **m > MAX_POWER_W) {
        v.flags
            .push(format!("a power sample of {m} W (limit {MAX_POWER_W} W)"));
    }
    if let Some(step) = p
        .windows(2)
        .map(|w| w[0].abs_diff(w[1]))
        .max()
        .filter(|d| *d > MAX_STEP_W)
    {
        v.flags.push(format!(
            "power jumps {step} W within a second (limit {MAX_STEP_W} W)"
        ));
    }
    if s.cadence_rpm.is_empty() {
        v.flags.push("no cadence recorded".into());
    } else {
        let without = p
            .iter()
            .zip(&s.cadence_rpm)
            .filter(|(w, c)| **w >= NO_CADENCE_POWER_W && **c == 0)
            .count();
        if without > NO_CADENCE_S {
            v.flags.push(format!(
                "{without} s of power ≥ {NO_CADENCE_POWER_W} W without cadence"
            ));
        }
    }
    let mut run = 1usize;
    let mut longest = 0usize;
    for w in p.windows(2) {
        if w[0] == w[1] && w[0] > 0 {
            run += 1;
        } else {
            run = 1;
        }
        longest = longest.max(run);
    }
    if longest >= CONSTANT_S {
        v.flags
            .push(format!("exactly the same power for {longest} s"));
    }
    if !s.heart_bpm.is_empty() && n >= 1200 {
        let pf: Vec<f64> = p.iter().map(|x| f64::from(*x)).collect();
        let hf: Vec<f64> = s.heart_bpm.iter().map(|x| f64::from(*x)).collect();
        let (_, psd) = mean_sd(&pf);
        let (_, hsd) = mean_sd(&hf);
        if psd > 40.0 && hsd < 1.0 {
            v.flags
                .push("the heart rate does not follow the load".into());
        }
    }
    let samples_j: f64 = p.iter().map(|x| f64::from(*x)).sum();
    if (s.work_j - samples_j).abs() > samples_j * 0.03 + 1000.0 {
        v.flags.push(format!(
            "work {:.0} kJ does not match the samples ({:.0} kJ)",
            s.work_j / 1000.0,
            samples_j / 1000.0
        ));
    }
    if s.distance_m / f64::from(s.duration_s) > MAX_SPEED_MS {
        v.flags.push(format!(
            "average speed {:.0} km/h",
            s.distance_m / f64::from(s.duration_s) * 3.6
        ));
    }
    if s.climb_m > s.distance_m * 0.2 + 50.0 {
        v.flags.push(format!(
            "{:.0} m of climbing on {:.1} km",
            s.climb_m,
            s.distance_m / 1000.0
        ));
    }
    v.hearts = ocw_core::hearts::earned(s.work_j.min(samples_j * 1.03 + 1000.0), s.climb_m);
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rides::Device;

    /// A plausible hour: 200 W ± 70 W, 85–95 rpm, heart rate following.
    pub fn hour_ride(id: &str, start: i64) -> RideSummary {
        let n = 3600;
        let power: Vec<u16> = (0..n)
            .map(|i| (200.0 + 70.0 * ((i as f64) / 90.0).sin()) as u16)
            .collect();
        let cadence: Vec<u16> = (0..n).map(|i| 85 + (i % 11) as u16).collect();
        let heart: Vec<u16> = power.iter().map(|p| 90 + p / 4).collect();
        RideSummary {
            ride_id: id.into(),
            started_unix_s: start,
            duration_s: n as u32,
            distance_m: 30_000.0,
            climb_m: 300.0,
            work_j: power.iter().map(|p| f64::from(*p)).sum(),
            weight_kg: 72.0,
            bike: None,
            devices: vec![Device {
                kind: DeviceKind::Trainer,
                name: "Test trainer".into(),
            }],
            power_w: power,
            cadence_rpm: cadence,
            heart_bpm: heart,
        }
    }

    const NOW: i64 = 1_800_000_000;

    #[test]
    fn a_plausible_hour_passes_and_earns() {
        let s = hour_ride("r1", NOW - 4000);
        let v = check(&s, NOW);
        assert!(v.passed(), "{v:?}");
        // ≈ 720 kJ → 72 ♥, 300 m → 15 ♥.
        assert_eq!(v.hearts, ocw_core::hearts::earned(s.work_j, 300.0));
        assert!((80..=90).contains(&v.hearts), "{}", v.hearts);
    }

    #[test]
    fn best_mean_windows() {
        assert_eq!(best_mean(&[1, 2, 3, 10], 2), Some(6.5));
        assert_eq!(best_mean(&[1], 2), None);
    }

    #[test]
    fn malformed_and_virtual_are_refused() {
        let mut s = hour_ride("r", NOW - 4000);
        s.devices = vec![Device {
            kind: DeviceKind::Virtual,
            name: String::new(),
        }];
        assert!(!check(&s, NOW).refused.is_empty());
        let mut s = hour_ride("r", NOW - 4000);
        s.devices.clear();
        assert!(!check(&s, NOW).refused.is_empty());
        let mut s = hour_ride("r", NOW - 4000);
        s.power_w.truncate(100);
        assert!(!check(&s, NOW).refused.is_empty());
        let s = hour_ride("r", NOW + 3600);
        assert!(!check(&s, NOW).refused.is_empty());
        let s = hour_ride("r", NOW - MAX_AGE_S - 4000);
        assert!(!check(&s, NOW).refused.is_empty());
        let mut s = hour_ride("", NOW - 4000);
        s.work_j = f64::NAN;
        assert_eq!(check(&s, NOW).refused.len(), 2);
    }

    #[test]
    fn implausible_rides_are_flagged() {
        let flagged = |f: &dyn Fn(&mut RideSummary)| {
            let mut s = hour_ride("r", NOW - 4000);
            f(&mut s);
            let v = check(&s, NOW);
            assert!(v.refused.is_empty(), "{v:?}");
            assert!(!v.flags.is_empty(), "not flagged");
            v.flags
        };
        // 20 minutes at 7 W/kg (504 W).
        let f = flagged(&|s| {
            s.power_w[..1200].fill(504);
            s.work_j = s.power_w.iter().map(|p| f64::from(*p)).sum();
        });
        assert!(f.iter().any(|r| r.contains("20 min")), "{f:?}");
        // A spike.
        flagged(&|s| {
            s.power_w[100] = 2000;
            s.work_j = s.power_w.iter().map(|p| f64::from(*p)).sum();
        });
        // Power without cadence.
        flagged(&|s| {
            s.cadence_rpm[..200].fill(0);
        });
        flagged(&|s| s.cadence_rpm.clear());
        // Perfectly constant power.
        flagged(&|s| {
            s.power_w[..700].fill(230);
            s.work_j = s.power_w.iter().map(|p| f64::from(*p)).sum();
        });
        // A flat heart rate under a changing load.
        flagged(&|s| {
            s.heart_bpm.fill(120);
        });
        // Work claimed beyond the samples.
        flagged(&|s| s.work_j *= 2.0);
        // Too fast, too much climbing.
        flagged(&|s| s.distance_m = 200_000.0);
        flagged(&|s| s.climb_m = 9000.0);
    }

    #[test]
    fn claimed_work_cannot_raise_the_hearts_much() {
        let mut s = hour_ride("r", NOW - 4000);
        let honest = check(&s, NOW).hearts;
        s.work_j *= 10.0;
        let v = check(&s, NOW);
        assert!(v.hearts <= honest + 3, "{} vs {honest}", v.hearts);
    }
}
