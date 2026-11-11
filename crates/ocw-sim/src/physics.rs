//! Longitudinal bicycle physics along a 1D path (spec §5.1, §5.2).

use ocw_core::{AIR_DENSITY, GRAVITY, RiderProfile};

/// Fraction of pedal power that reaches the rear wheel.
pub const DRIVETRAIN_EFFICIENCY: f64 = 0.976;
/// Extra mass equivalent of the rotating wheels, in kg.
pub const WHEEL_INERTIA_MASS_KG: f64 = 0.9;
/// Below this speed the propulsive force is computed as if riding at this
/// speed, which avoids an infinite force when starting from standstill.
pub const MIN_PROPULSION_SPEED: f64 = 1.0;

/// State of one rider along their path.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RiderState {
    /// Total distance ridden in metres (keeps growing over laps).
    pub distance: f64,
    /// Speed in m/s (never negative).
    pub speed: f64,
    /// Sideways offset from the road centerline in metres (positive = right).
    pub lateral_offset: f64,
}

/// Conditions at the rider's position.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Environment {
    /// Gradient as rise over run.
    pub grade: f64,
    /// Headwind speed in m/s (negative = tailwind).
    pub headwind: f64,
    /// Air density in kg/m³.
    pub air_density: f64,
}

impl Environment {
    /// Still air at sea level on the given gradient.
    pub fn with_grade(grade: f64) -> Self {
        Self {
            grade,
            headwind: 0.0,
            air_density: AIR_DENSITY,
        }
    }
}

/// Resistive forces in newtons at a given speed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Resistance {
    pub gravity: f64,
    pub rolling: f64,
    pub aero: f64,
}

impl Resistance {
    pub fn total(&self) -> f64 {
        self.gravity + self.rolling + self.aero
    }
}

/// Computes the resistive forces acting against the rider.
pub fn resistance(profile: &RiderProfile, env: &Environment, speed: f64) -> Resistance {
    let m = profile.total_mass_kg();
    let angle = env.grade.atan();
    let air = speed + env.headwind;
    Resistance {
        gravity: m * GRAVITY * angle.sin(),
        rolling: if speed > 0.0 {
            profile.crr * m * GRAVITY * angle.cos()
        } else {
            0.0
        },
        aero: 0.5 * env.air_density * profile.cda_m2 * air * air.abs(),
    }
}

/// Advances `state` by `dt` seconds with `power_w` watts at the pedals.
///
/// Semi-implicit Euler; stable at the 50 Hz tick rate.
pub fn step(
    state: &mut RiderState,
    profile: &RiderProfile,
    power_w: f64,
    env: &Environment,
    dt: f64,
) {
    let propulsion =
        DRIVETRAIN_EFFICIENCY * power_w.max(0.0) / state.speed.max(MIN_PROPULSION_SPEED);
    let net = propulsion - resistance(profile, env, state.speed).total();
    let accel = net / (profile.total_mass_kg() + WHEEL_INERTIA_MASS_KG);
    state.speed = (state.speed + accel * dt).max(0.0);
    state.distance += state.speed * dt;
}

/// Power in watts needed to hold `speed` steadily under `env`.
pub fn steady_state_power(profile: &RiderProfile, env: &Environment, speed: f64) -> f64 {
    resistance(profile, env, speed).total() * speed / DRIVETRAIN_EFFICIENCY
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_core::SIM_DT;

    fn ride(power: f64, grade: f64, seconds: f64) -> RiderState {
        let profile = RiderProfile::default();
        let env = Environment::with_grade(grade);
        let mut s = RiderState::default();
        for _ in 0..(seconds / SIM_DT) as usize {
            step(&mut s, &profile, power, &env, SIM_DT);
        }
        s
    }

    #[test]
    fn flat_200w_reaches_realistic_speed() {
        let s = ride(200.0, 0.0, 120.0);
        let kmh = s.speed * 3.6;
        assert!((32.0..36.0).contains(&kmh), "{kmh} km/h");
        // Steady state: power balance holds.
        let p = steady_state_power(
            &RiderProfile::default(),
            &Environment::with_grade(0.0),
            s.speed,
        );
        assert!((p - 200.0).abs() < 1.0, "{p} W");
    }

    #[test]
    fn climbing_is_slower() {
        let flat = ride(250.0, 0.0, 120.0).speed;
        let climb = ride(250.0, 0.06, 120.0).speed;
        assert!(climb < flat * 0.6, "flat {flat}, climb {climb}");
        // 250 W on 6 % for an 83 kg system is about 13–15 km/h.
        assert!(
            (12.0..16.0).contains(&(climb * 3.6)),
            "{} km/h",
            climb * 3.6
        );
    }

    #[test]
    fn coasting_downhill_accelerates_and_uphill_stops() {
        assert!(ride(0.0, -0.05, 30.0).speed > 10.0);
        let mut s = RiderState {
            speed: 5.0,
            ..Default::default()
        };
        let profile = RiderProfile::default();
        let env = Environment::with_grade(0.08);
        for _ in 0..500 {
            step(&mut s, &profile, 0.0, &env, SIM_DT);
        }
        assert_eq!(s.speed, 0.0, "never rolls backwards");
    }

    #[test]
    fn starts_from_standstill() {
        assert!(ride(150.0, 0.0, 5.0).speed > 3.0);
    }
}
