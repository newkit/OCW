//! The expected riding time of a route at a constant power (spec 14
//! Phase 16, 16D *Views*): the physics of spec 5 ([`physics::step`] at the
//! game's 50 Hz) run along the route's elevation profile and surfaces,
//! with the rider's profile (weight and curated bike, spec 2.5).
//!
//! What it assumes: the same power at the pedals all the way (also
//! downhill), a start from standstill, still air at sea level, no
//! drafting, no braking for corners or junctions. So on long descents it
//! is faster than a rider who coasts or brakes; on the flat and uphill it
//! is what the game gives at that power.

use ocw_core::RiderProfile;
use ocw_world::Surface;

use crate::physics::{self, Environment, RiderState};

/// The time step (s): the game's physics tick (spec 5.2).
pub const STEP_S: f64 = 1.0 / 50.0;
/// The longest ride simulated (s); a route that takes longer is not
/// estimated (48 h).
pub const MAX_TIME_S: f64 = 48.0 * 3600.0;

/// A point of the route's profile: from here to the next point the road
/// climbs (or falls) straight and has this surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProfilePoint {
    /// Distance along the route (m, increasing).
    pub distance_m: f64,
    /// Road height (m).
    pub height_m: f64,
    pub surface: Surface,
}

/// What the estimate gives.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RideEstimate {
    /// The expected riding time (s).
    pub time_s: f64,
    pub length_m: f64,
    /// The power and the rider's profile it was made with.
    pub power_w: f64,
    pub profile: RiderProfile,
    /// Average and top speed (m/s).
    pub average_mps: f64,
    pub top_mps: f64,
    /// Time spent on parts steeper than 3 % up (s).
    pub climbing_s: f64,
}

/// The expected time to ride `points` at `power_w` watts with `profile`;
/// `None` without power, without two points with distance between them,
/// or when it would take longer than [`MAX_TIME_S`].
pub fn ride_time(
    profile: &RiderProfile,
    power_w: f64,
    points: &[ProfilePoint],
) -> Option<RideEstimate> {
    if power_w.is_nan() || power_w <= 0.0 || points.len() < 2 {
        return None;
    }
    let start = points[0].distance_m;
    let length = points[points.len() - 1].distance_m - start;
    if length.is_nan() || length <= 0.0 {
        return None;
    }
    let mut state = RiderState::default();
    let (mut t, mut top, mut climbing) = (0.0f64, 0.0f64, 0.0f64);
    let mut i = 0;
    let mut env = Environment::with_grade(0.0);
    let mut env_for = usize::MAX;
    while state.distance < length {
        while i + 2 < points.len() && points[i + 1].distance_m - start <= state.distance {
            i += 1;
        }
        if env_for != i {
            let (a, b) = (points[i], points[i + 1]);
            let run = b.distance_m - a.distance_m;
            let grade = if run > 1e-6 {
                (b.height_m - a.height_m) / run
            } else {
                0.0
            };
            env = Environment::with_grade(grade).with_surface(a.surface);
            env_for = i;
        }
        let before = state.distance;
        physics::step(&mut state, profile, power_w, &env, STEP_S);
        let moved = state.distance - before;
        let mut dt = STEP_S;
        if state.distance > length && moved > 0.0 {
            // The finish within this step.
            dt *= (length - before) / moved;
        }
        t += dt;
        top = top.max(state.speed);
        if env.grade > 0.03 {
            climbing += dt;
        }
        if t > MAX_TIME_S {
            return None;
        }
    }
    Some(RideEstimate {
        time_s: t,
        length_m: length,
        power_w,
        profile: *profile,
        average_mps: length / t,
        top_mps: top,
        climbing_s: climbing,
    })
}

/// "1 h 05 min", "42 min", "50 s".
pub fn duration_text(s: f64) -> String {
    let s = s.max(0.0).round() as u64;
    if s < 60 {
        format!("{s} s")
    } else if s < 3600 {
        format!("{} min", (s + 30) / 60)
    } else {
        let m = (s + 30) / 60;
        format!("{} h {:02} min", m / 60, m % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_core::ProfileId;

    /// A straight road of `length` m at `grade`, a point every 10 m.
    fn road(length: f64, grade: f64, surface: Surface) -> Vec<ProfilePoint> {
        let n = (length / 10.0) as usize;
        (0..=n)
            .map(|k| {
                let d = k as f64 * 10.0;
                ProfilePoint {
                    distance_m: d,
                    height_m: 400.0 + d * grade,
                    surface,
                }
            })
            .collect()
    }

    /// The time of the steady speed alone (no start).
    fn steady_time(p: &RiderProfile, length: f64, grade: f64, power: f64) -> f64 {
        length / physics::steady_speed(p, &Environment::with_grade(grade), power)
    }

    #[test]
    fn on_the_flat_it_is_the_steady_speed_plus_the_start() {
        let p = RiderProfile::default();
        let e = ride_time(&p, 200.0, &road(20_000.0, 0.0, Surface::Asphalt)).unwrap();
        let steady = steady_time(&p, 20_000.0, 0.0, 200.0);
        // 75 kg on the road bike at 200 W: about 33 km/h.
        assert!(
            (32.0..35.0).contains(&(e.average_mps * 3.6)),
            "{} km/h",
            e.average_mps * 3.6
        );
        // The start from standstill costs a few seconds, no more.
        assert!(
            e.time_s > steady && e.time_s < steady + 15.0,
            "{} {steady}",
            e.time_s
        );
        assert!((e.top_mps * 3.6 - 3.6 * 20_000.0 / steady).abs() < 0.1);
        assert_eq!(e.climbing_s, 0.0);
        assert_eq!(e.length_m, 20_000.0);
    }

    #[test]
    fn a_climb_takes_its_steady_time_and_weight_counts() {
        // 5 km at 7 %, 300 W, 75 kg: about 4.6 m/s, 18 minutes.
        let p = RiderProfile::default();
        let climb = road(5000.0, 0.07, Surface::Asphalt);
        let e = ride_time(&p, 300.0, &climb).unwrap();
        let steady = steady_time(&p, 5000.0, 0.07, 300.0);
        assert!(
            (e.time_s - steady).abs() / steady < 0.01,
            "{} {steady}",
            e.time_s
        );
        assert!(
            (17.0 * 60.0..19.5 * 60.0).contains(&e.time_s),
            "{}",
            e.time_s
        );
        assert!((e.climbing_s - e.time_s).abs() < 1e-6);
        // 10 kg more: slower uphill, by about the extra weight's share.
        let heavy = RiderProfile::for_rider_weight(85.0);
        let h = ride_time(&heavy, 300.0, &climb).unwrap();
        assert!(
            h.time_s > e.time_s * 1.08 && h.time_s < e.time_s * 1.14,
            "{} {}",
            h.time_s,
            e.time_s
        );
        // Twice the power: much faster, but not twice (air drag).
        let fast = ride_time(&p, 600.0, &climb).unwrap();
        assert!(fast.time_s < e.time_s * 0.6 && fast.time_s > e.time_s * 0.45);
    }

    #[test]
    fn up_and_down_and_rough_surfaces() {
        let p = RiderProfile::default();
        // Up 2 km at 6 % and down again: slower than the flat at the same
        // power (the descent gives back less than the climb takes).
        let mut hill = road(2000.0, 0.06, Surface::Asphalt);
        let top = hill.last().unwrap().height_m;
        hill.extend((1..=200).map(|k| ProfilePoint {
            distance_m: 2000.0 + f64::from(k) * 10.0,
            height_m: top - f64::from(k) * 0.6,
            surface: Surface::Asphalt,
        }));
        let flat = ride_time(&p, 200.0, &road(4000.0, 0.0, Surface::Asphalt)).unwrap();
        let h = ride_time(&p, 200.0, &hill).unwrap();
        assert!(h.time_s > flat.time_s * 1.3, "{} {}", h.time_s, flat.time_s);
        assert!(h.top_mps > flat.top_mps);
        // Gravel rolls slower than asphalt, and the gravel bike minds it less.
        let asphalt = ride_time(&p, 200.0, &road(5000.0, 0.0, Surface::Asphalt)).unwrap();
        let gravel = ride_time(&p, 200.0, &road(5000.0, 0.0, Surface::Gravel)).unwrap();
        assert!(gravel.time_s > asphalt.time_s * 1.05);
        let gp = RiderProfile::curated(ProfileId::Gravel, 75.0);
        let on_gravel_bike = ride_time(&gp, 200.0, &road(5000.0, 0.0, Surface::Gravel)).unwrap();
        let on_asphalt_bike = ride_time(&gp, 200.0, &road(5000.0, 0.0, Surface::Asphalt)).unwrap();
        assert!(
            on_gravel_bike.time_s / on_asphalt_bike.time_s < gravel.time_s / asphalt.time_s,
            "the gravel bike feels less of the gravel"
        );
    }

    #[test]
    fn nothing_to_estimate() {
        let p = RiderProfile::default();
        let r = road(1000.0, 0.0, Surface::Asphalt);
        assert!(ride_time(&p, 0.0, &r).is_none());
        assert!(ride_time(&p, f64::NAN, &r).is_none());
        assert!(ride_time(&p, 200.0, &r[..1]).is_none());
        let same = [r[0], r[0]];
        assert!(ride_time(&p, 200.0, &same).is_none());
        assert_eq!(duration_text(50.0), "50 s");
        assert_eq!(duration_text(42.0 * 60.0 + 10.0), "42 min");
        assert_eq!(duration_text(3600.0 + 5.0 * 60.0 + 40.0), "1 h 06 min");
    }
}
