//! The summary of a ride the app uploads to the account (spec 14 Phase 4,
//! 4O Q19 (a) / Q20 (a)): made from the ride's recording – its records
//! once a second – when the ride ends, for every ride a certified server
//! did not credit itself. Seconds of virtual power (keyboard power,
//! `sim.power`, the virtual trainer) count no power: they earn nothing, as
//! on the server.

use ocw_account_api::rides::{Device, DeviceKind, RideSummary};
use ocw_fit::Activity;

/// Whether a ride's summary goes to the account: signed in, a server
/// that does not credit the hearts itself, not a capture or the kiosk's
/// show (`--no-hearts`), not Training mode.
pub fn uploads(
    signed_in: bool,
    server_credits: bool,
    capture: bool,
    no_hearts: bool,
    training: bool,
) -> bool {
    signed_in && !server_credits && !capture && !no_hearts && !training
}

/// The summary of `activity` (`virtual_s[i]`: record `i` had virtual
/// power; shorter than the records: the rest real). `None` when no second
/// had real power (nothing to credit).
pub fn summary(
    activity: &Activity,
    virtual_s: &[bool],
    ride_id: &str,
    weight_kg: f64,
    bike: Option<String>,
) -> Option<RideSummary> {
    let recs = &activity.records;
    if recs.is_empty() {
        return None;
    }
    let is_virtual = |i: usize| virtual_s.get(i).copied().unwrap_or(false);
    let power_w: Vec<u16> = recs
        .iter()
        .enumerate()
        .map(|(i, r)| {
            if is_virtual(i) {
                0
            } else {
                r.power_w.unwrap_or(0.0).clamp(0.0, 65_535.0).round() as u16
            }
        })
        .collect();
    let real = (0..recs.len()).any(|i| !is_virtual(i) && recs[i].power_w.is_some_and(|p| p > 0.0));
    if !real {
        return None;
    }
    let cadence_rpm: Vec<u16> = if recs.iter().any(|r| r.cadence_rpm.is_some()) {
        recs.iter()
            .map(|r| r.cadence_rpm.unwrap_or(0.0).clamp(0.0, 400.0).round() as u16)
            .collect()
    } else {
        Vec::new()
    };
    let heart_bpm: Vec<u16> = if recs.iter().any(|r| r.heart_rate_bpm.is_some()) {
        recs.iter()
            .map(|r| r.heart_rate_bpm.unwrap_or(0.0).clamp(0.0, 300.0).round() as u16)
            .collect()
    } else {
        Vec::new()
    };
    // Climbing from the altitude, the rises only, at most 2 m a second
    // (as the server counts it; a jump is no climb).
    let climb_m: f64 = recs
        .windows(2)
        .filter_map(|w| Some((w[1].altitude_m? - w[0].altitude_m?).clamp(0.0, 2.0)))
        .sum();
    let mut devices = vec![Device {
        kind: DeviceKind::Trainer,
        name: String::new(),
    }];
    if !heart_bpm.is_empty() {
        devices.push(Device {
            kind: DeviceKind::HeartRate,
            name: String::new(),
        });
    }
    let distance_m = recs.last().map_or(0.0, |r| r.distance_m) - recs[0].distance_m;
    Some(RideSummary {
        ride_id: ride_id.into(),
        started_unix_s: activity.start_unix_s,
        duration_s: power_w.len() as u32,
        distance_m: distance_m.max(0.0),
        climb_m,
        work_j: power_w.iter().map(|p| f64::from(*p)).sum(),
        weight_kg,
        bike,
        devices,
        power_w,
        cadence_rpm,
        heart_bpm,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_fit::Record;

    fn activity(n: usize) -> Activity {
        Activity {
            start_unix_s: 1_800_000_000,
            records: (0..n)
                .map(|i| Record {
                    elapsed_s: i as f64,
                    distance_m: i as f64 * 8.0,
                    speed_mps: 8.0,
                    power_w: Some(200.0 + (i % 7) as f64),
                    cadence_rpm: Some(88.0),
                    heart_rate_bpm: None,
                    altitude_m: Some(100.0 + i as f64 * 0.1),
                    position: None,
                })
                .collect(),
            ..empty()
        }
    }

    fn empty() -> Activity {
        Activity {
            start_unix_s: 0,
            records: Vec::new(),
            product_name: String::new(),
            software_version: 0,
            laps: Vec::new(),
            markers: Vec::new(),
        }
    }

    #[test]
    fn a_recording_becomes_a_summary_that_passes_the_checks() {
        let a = activity(1200);
        let s = summary(&a, &[], "r", 72.0, None).unwrap();
        assert_eq!(
            (s.duration_s, s.power_w.len(), s.cadence_rpm.len()),
            (1200, 1200, 1200)
        );
        assert!(s.heart_bpm.is_empty());
        assert!((s.climb_m - 119.9).abs() < 0.2, "{}", s.climb_m);
        assert!((s.distance_m - 1199.0 * 8.0).abs() < 1e-6);
        let v = ocw_account_api::plausibility::check(&s, a.start_unix_s + 1300);
        assert!(v.passed(), "{v:?}");
        assert!(v.hearts > 0);
    }

    #[test]
    fn virtual_seconds_earn_nothing() {
        let a = activity(100);
        assert!(summary(&a, &[true; 100], "r", 72.0, None).is_none());
        let s = summary(&a, &[true; 50], "r", 72.0, None).unwrap();
        assert!(s.power_w[..50].iter().all(|p| *p == 0));
        assert!(s.power_w[50..].iter().all(|p| *p >= 200));
        assert!(summary(&empty(), &[], "r", 72.0, None).is_none());
    }

    #[test]
    fn who_uploads() {
        assert!(uploads(true, false, false, false, false));
        assert!(!uploads(false, false, false, false, false));
        assert!(
            !uploads(true, true, false, false, false),
            "the server credits"
        );
        assert!(!uploads(true, false, true, false, false));
        assert!(!uploads(true, false, false, true, false));
        assert!(!uploads(true, false, false, false, true));
    }
}
