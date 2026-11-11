//! Longitudinal bicycle physics along a 1D path (spec §5.1, §5.2).

use ocw_core::{AIR_DENSITY, GRAVITY, RiderProfile};

/// Fraction of pedal power that reaches the rear wheel.
pub const DRIVETRAIN_EFFICIENCY: f64 = 0.976;
/// Extra mass equivalent of the rotating wheels, in kg.
pub const WHEEL_INERTIA_MASS_KG: f64 = 0.9;
/// Below this speed the propulsive force is computed as if riding at this
/// speed, which avoids an infinite force when starting from standstill.
pub const MIN_PROPULSION_SPEED: f64 = 1.0;
/// Fastest sideways movement when steering across the road, in m/s.
pub const MAX_LATERAL_SPEED: f64 = 1.5;
/// Distance kept from the road edge when steering, in metres.
pub const EDGE_MARGIN_M: f64 = 0.6;

/// Largest lateral offset (either side) a rider may steer to on a road.
pub fn lateral_limit(road_width_m: f64) -> f64 {
    (road_width_m / 2.0 - EDGE_MARGIN_M).max(0.0)
}

/// Moves the lateral offset towards `target` by at most
/// `MAX_LATERAL_SPEED · dt` (steering is smooth, never a jump).
pub fn steer(current: f64, target: f64, dt: f64) -> f64 {
    let max = MAX_LATERAL_SPEED * dt;
    current + (target - current).clamp(-max, max)
}

/// How a rider steers across the road (spec §6.1: intents, applied by the
/// server and predicted by the client with the same rule).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Steering {
    /// Move to this lateral offset (m, positive = right) at up to
    /// `MAX_LATERAL_SPEED`, keeping parallel to the road.
    Target(f64),
    /// Lean by this angle (rad, positive = right) and ride the curve it
    /// gives, κ = g·tan(lean)/v², relative to the road. Leaning 0 rides
    /// straight on and lines up with the road again.
    Lean(f64),
}

/// Largest lean a rider can steer with (rad, 30°).
pub const MAX_STEER_LEAN: f64 = 0.52;
/// Tightest curve a rider rides relative to the road (1/m, radius 8 m).
pub const MAX_TURN_CURVATURE: f64 = 0.125;
/// Largest angle between the rider and the road (rad, ~17°).
pub const MAX_HEADING: f64 = 0.3;
/// Time constant with which a rider riding straight on lines up with the
/// road again, in seconds.
pub const ALIGN_TIME_S: f64 = 2.0;

/// Applies `steering` for one tick: lateral offset, heading and the
/// curvature ridden relative to the road. `limit` is the largest offset
/// either side ([`lateral_limit`]).
pub fn apply_steering(state: &mut RiderState, steering: Steering, limit: f64, dt: f64) {
    match steering {
        Steering::Target(target) => {
            state.lateral_offset = steer(state.lateral_offset, target.clamp(-limit, limit), dt);
            state.heading = 0.0;
            state.turn_curvature = 0.0;
        }
        Steering::Lean(lean) => {
            let v = state.speed;
            let lean = lean.clamp(-MAX_STEER_LEAN, MAX_STEER_LEAN);
            let mut k = if v > 0.1 {
                (GRAVITY * lean.tan() / (v * v)).clamp(-MAX_TURN_CURVATURE, MAX_TURN_CURVATURE)
            } else {
                0.0
            };
            if lean.abs() < 1e-6 {
                state.heading *= (-dt / ALIGN_TIME_S).exp();
            } else {
                let h = state.heading + v * k * dt;
                if h.abs() > MAX_HEADING {
                    k = 0.0; // turned as far as it goes: straight at that angle
                }
                state.heading = h.clamp(-MAX_HEADING, MAX_HEADING);
            }
            state.lateral_offset += v * state.heading.sin() * dt;
            if state.lateral_offset.abs() >= limit {
                // At the road edge: along it, no further turning outwards.
                state.lateral_offset = state.lateral_offset.clamp(-limit, limit);
                if state.heading * state.lateral_offset > 0.0 {
                    state.heading = 0.0;
                }
                if k * state.lateral_offset > 0.0 {
                    k = 0.0;
                }
            }
            state.turn_curvature = k;
        }
    }
}

/// State of one rider along their path.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RiderState {
    /// Total distance ridden in metres (keeps growing over laps).
    pub distance: f64,
    /// Speed in m/s (never negative).
    pub speed: f64,
    /// Sideways offset from the road centerline in metres (positive = right).
    pub lateral_offset: f64,
    /// Direction of travel relative to the road in rad (positive = right).
    pub heading: f64,
    /// Curvature ridden relative to the road in 1/m (positive = right).
    pub turn_curvature: f64,
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
    /// Factor on the aerodynamic drag from drafting (1.0 = none, see `drafting`).
    pub drag_factor: f64,
    /// Braking deceleration in m/s² (0 = not braking).
    pub brake_mps2: f64,
}

/// Deceleration of the brake key: a firm stop (~3.5 s from 36 km/h).
pub const BRAKE_DECELERATION_MPS2: f64 = 3.0;

impl Environment {
    /// Still air at sea level on the given gradient.
    pub fn with_grade(grade: f64) -> Self {
        Self {
            grade,
            headwind: 0.0,
            air_density: AIR_DENSITY,
            drag_factor: 1.0,
            brake_mps2: 0.0,
        }
    }

    /// The same environment, braking (`true`) or not.
    pub fn with_brake(self, braking: bool) -> Self {
        Self {
            brake_mps2: if braking {
                BRAKE_DECELERATION_MPS2
            } else {
                0.0
            },
            ..self
        }
    }

    /// The same environment with a drafting drag factor.
    pub fn with_drag_factor(self, drag_factor: f64) -> Self {
        Self {
            drag_factor,
            ..self
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
        aero: 0.5 * env.air_density * profile.cda_m2 * env.drag_factor * air * air.abs(),
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
    state.speed = (state.speed + (accel - env.brake_mps2) * dt).max(0.0);
    state.distance += state.speed * dt;
}

/// Power in watts needed to hold `speed` steadily under `env`.
pub fn steady_state_power(profile: &RiderProfile, env: &Environment, speed: f64) -> f64 {
    resistance(profile, env, speed).total() * speed / DRIVETRAIN_EFFICIENCY
}

#[cfg(test)]
mod tests {
    use super::*;

    fn riding(speed: f64) -> RiderState {
        RiderState {
            speed,
            ..Default::default()
        }
    }

    #[test]
    fn leaning_rides_a_curve_until_straightened() {
        // 10 m/s, leaning 10° right: κ = g·tan(10°)/v² ≈ 0.0173 / m.
        let mut s = riding(10.0);
        let lean = 10f64.to_radians();
        apply_steering(&mut s, Steering::Lean(lean), 3.0, 0.02);
        assert!((s.turn_curvature - GRAVITY * lean.tan() / 100.0).abs() < 1e-9);
        for _ in 0..25 {
            apply_steering(&mut s, Steering::Lean(lean), 3.0, 0.02);
        }
        // Half a second later: turned right and moving right.
        assert!(s.heading > 0.05 && s.lateral_offset > 0.0, "{s:?}");
        // Upright again (the opposite key): no more turning, lines up with
        // the road and stays on its new line.
        let at = s.lateral_offset;
        for _ in 0..500 {
            apply_steering(&mut s, Steering::Lean(0.0), 3.0, 0.02);
        }
        assert_eq!(s.turn_curvature, 0.0);
        assert!(s.heading.abs() < 0.001, "{}", s.heading);
        assert!(s.lateral_offset > at && s.lateral_offset < 3.0, "{s:?}");
    }

    #[test]
    fn leaning_stops_at_the_road_edge_and_the_heading_limit() {
        let mut s = riding(12.0);
        for _ in 0..1000 {
            apply_steering(&mut s, Steering::Lean(-0.4), 2.5, 0.02);
            assert!(s.heading.abs() <= MAX_HEADING + 1e-12);
        }
        assert_eq!(s.lateral_offset, -2.5);
        assert_eq!((s.heading, s.turn_curvature), (0.0, 0.0));
        // Standing still: no curve, no movement.
        let mut s = riding(0.0);
        apply_steering(&mut s, Steering::Lean(0.4), 2.5, 0.02);
        assert_eq!(s, riding(0.0));
        // Slow: the curvature is capped (no turning on the spot).
        let mut s = riding(1.0);
        apply_steering(&mut s, Steering::Lean(0.5), 2.5, 0.02);
        assert_eq!(s.turn_curvature, MAX_TURN_CURVATURE);
    }

    #[test]
    fn target_steering_keeps_parallel_to_the_road() {
        let mut s = riding(10.0);
        s.heading = 0.2;
        apply_steering(&mut s, Steering::Target(5.0), 2.0, 0.02);
        assert_eq!(s.heading, 0.0);
        assert!((s.lateral_offset - MAX_LATERAL_SPEED * 0.02).abs() < 1e-12);
    }
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

    #[test]
    fn drafting_is_faster_at_the_same_power() {
        let profile = RiderProfile::default();
        let speed_after = |factor: f64| {
            let mut s = RiderState::default();
            let env = Environment::with_grade(0.0).with_drag_factor(factor);
            for _ in 0..(120 * 50) {
                step(&mut s, &profile, 200.0, &env, SIM_DT);
            }
            s.speed
        };
        let (alone, drafting) = (speed_after(1.0), speed_after(0.7));
        assert!(drafting > alone * 1.08, "{alone} vs {drafting}");
    }

    #[test]
    fn steering_is_rate_limited_and_reaches_the_target() {
        assert_eq!(lateral_limit(7.0), 2.9);
        assert_eq!(lateral_limit(1.0), 0.0);
        let mut x = 0.0;
        for _ in 0..50 {
            x = steer(x, 2.0, SIM_DT); // 1 s
        }
        assert!((x - 1.5).abs() < 1e-9, "{x}");
        for _ in 0..50 {
            x = steer(x, 2.0, SIM_DT);
        }
        assert_eq!(x, 2.0);
        assert_eq!(steer(2.0, -1.0, 0.0), 2.0);
    }

    #[test]
    fn braking_stops_the_rider() {
        let profile = RiderProfile::default();
        let mut s = RiderState {
            speed: 10.0,
            ..Default::default()
        };
        let env = Environment::with_grade(0.0).with_brake(true);
        let mut t = 0.0;
        while s.speed > 0.0 && t < 10.0 {
            step(&mut s, &profile, 0.0, &env, SIM_DT);
            t += SIM_DT;
        }
        assert!((2.5..3.5).contains(&t), "stopped after {t} s");
        // Stays stopped, never rolls backwards.
        step(&mut s, &profile, 0.0, &env, SIM_DT);
        assert_eq!(s.speed, 0.0);
        assert_eq!(
            Environment::with_grade(0.0).with_brake(false).brake_mps2,
            0.0
        );
    }
}
