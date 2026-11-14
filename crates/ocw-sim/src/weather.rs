//! Weather in the physics (spec §14 Phase 9, 9E, milestone S6): when the
//! session's *weather affects physics* switch is on (Q4: on in solo and
//! free ride, the organiser's choice in events), the session weather's
//! [`WeatherPhysics`] (`ocw_weather::physics`) enters every rider's tick
//! ([`crate::ride::tick_in`]) – on the server for everyone, in the client's
//! prediction for its own rider, with the same inputs:
//!
//! - **Wind** as the air speed along the rider's direction of travel (the
//!   road's bearing turned by the rider's heading): a headwind adds to the
//!   speed in the drag term, a tailwind takes off it. Only the component
//!   along the travel counts (the "tangential" model of Martin et al.
//!   1998): a pure crosswind changes nothing along the road. A yaw-dependent
//!   CdA is left out on purpose (its sign depends on the wheels and the
//!   rider: deep rims lose axial drag at moderate yaw, the body gains it,
//!   and the profiles of spec §2.5 carry no yaw data).
//! - **Drafting in a crosswind:** the wake of a rider ahead drifts
//!   downwind by the crosswind over the air speed along the road per metre
//!   behind ([`wake_drift`]): the shelter moves to the lee side (echelons).
//! - **Air density** at the rider's height from the weather's temperature,
//!   pressure and humidity.
//! - **A wet road** rolls a little harder and grips less: the automatic
//!   slow-down before sharp turns brakes to a lower speed and a leaning
//!   rider can lean less ([`corner_speed`], [`lean_limit`]).
//! - **Tunnels** ([`ocw_world::Track::sheltered`]) shelter from wind and
//!   rain. Other shelter (forests, buildings, cuttings) is open: the
//!   package's land use would allow it, but there is no measured model.

use ocw_world::Track;

pub use ocw_weather::physics::{RelativeWind, WeatherPhysics};

use crate::physics::{MAX_STEER_LEAN, RiderState};

/// Horizontal unit vector of a rider's travel (x, z): the road's direction
/// turned by the rider's heading relative to it (positive = right).
pub fn travel(road_direction: [f64; 2], heading: f64) -> [f64; 2] {
    let [dx, dz] = road_direction;
    // The road's right is (−dz, dx).
    let (s, c) = heading.sin_cos();
    [dx * c - dz * s, dz * c + dx * s]
}

/// The weather at one rider: the session's (dry and calm in a tunnel), the
/// wind relative to its travel and the air density where it is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AtRider {
    pub weather: WeatherPhysics,
    pub wind: RelativeWind,
    pub density: f64,
}

/// The weather at the rider in `state` on `track`.
pub fn at_rider(w: &WeatherPhysics, track: &dyn Track, state: &RiderState) -> AtRider {
    let p = track.sample(state.distance);
    let weather = if track.sheltered(state.distance) {
        w.sheltered()
    } else {
        *w
    };
    AtRider {
        wind: weather.relative(travel(p.direction, state.heading)),
        density: weather.density_at(p.position[1]),
        weather,
    }
}

/// The wake's sideways drift at most (m per metre behind, ≈ 56° yaw).
pub const MAX_WAKE_DRIFT: f64 = 1.5;

/// How far the wake of a rider ahead drifts sideways per metre behind it
/// (positive = to the right) for a rider at `speed` (m/s) in `wind`: the
/// air passes the riders at `speed + head` along the road and `cross`
/// across it.
pub fn wake_drift(wind: RelativeWind, speed: f64) -> f64 {
    (wind.cross_mps / (speed + wind.head_mps).max(1.0)).clamp(-MAX_WAKE_DRIFT, MAX_WAKE_DRIFT)
}

/// The steepest lean a rider can steer with on a road of `grip` (1 = dry):
/// the friction a curve needs is tan(lean), so the dry limit's tangent
/// scales with the grip.
pub fn lean_limit(grip: f64) -> f64 {
    (MAX_STEER_LEAN.tan() * grip.clamp(0.0, 1.0)).atan()
}

/// The highest speed through a curve whose dry limit is `dry_mps` on a
/// road of `grip` (v² = μ·g·r).
pub fn corner_speed(dry_mps: f64, grip: f64) -> f64 {
    dry_mps * grip.clamp(0.0, 1.0).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_weather::Ground;

    #[test]
    fn travel_turns_the_road_by_the_heading() {
        // Riding north (−z), turned 90° right: east (+x).
        let t = travel([0.0, -1.0], std::f64::consts::FRAC_PI_2);
        assert!((t[0] - 1.0).abs() < 1e-12 && t[1].abs() < 1e-12, "{t:?}");
        assert_eq!(travel([0.6, 0.8], 0.0), [0.6, 0.8]);
    }

    #[test]
    fn wake_drifts_downwind() {
        let w = RelativeWind {
            head_mps: 0.0,
            cross_mps: 5.0,
        };
        assert!((wake_drift(w, 10.0) - 0.5).abs() < 1e-12);
        let left = RelativeWind {
            cross_mps: -5.0,
            ..w
        };
        assert!(wake_drift(left, 10.0) < 0.0);
        assert_eq!(wake_drift(w, 0.0), MAX_WAKE_DRIFT);
        assert_eq!(wake_drift(RelativeWind::default(), 10.0), 0.0);
    }

    #[test]
    fn grip_limits_lean_and_corner_speed() {
        assert!((lean_limit(1.0) - MAX_STEER_LEAN).abs() < 1e-12);
        assert!(lean_limit(0.75) < MAX_STEER_LEAN && lean_limit(0.4) < lean_limit(0.75));
        assert_eq!(corner_speed(10.0, 1.0), 10.0);
        assert!((corner_speed(10.0, 0.64) - 8.0).abs() < 1e-12);
    }

    #[test]
    fn the_weather_at_a_rider() {
        let storm = ocw_weather::presets::preset("storm").unwrap();
        let wet = Ground::with_factors(1.0, 1.0, 0.0);
        let w = WeatherPhysics::new(&storm, storm.gust_mps, &wet, 0.0);
        // A straight road north at 500 m above the scene's sea level.
        let route = ocw_world::Route::from_points(
            "r",
            "R",
            vec![[0.0, 500.0, 0.0], [0.0, 500.0, -500.0]],
            false,
            6.0,
        )
        .unwrap();
        let s = RiderState::default();
        let open = at_rider(&w, &route, &s);
        assert!(open.wind.speed() > 10.0 && open.weather.grip < 1.0);
        assert_eq!(open.density, w.density_at(500.0));
        assert_eq!(open.wind, w.relative([0.0, -1.0]));
    }
}
