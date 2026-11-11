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

/// How a rider steers (spec §6.1: intents, applied by the server and
/// predicted by the client with the same rule). Riders have a heading
/// relative to the road; the road's curves turn the road under them, so
/// keeping on it needs steering.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Steering {
    /// Autodrive: an autopilot leans the rider to hold this lateral offset
    /// (m, positive = right; 0 = the middle of the road), through curves.
    Auto(f64),
    /// Manual: lean by this angle (rad, positive = right) and ride the
    /// curve it gives, κ = g·tan(lean)/v². The road's own curves have to be
    /// steered, or the rider drifts to the edge.
    Lean(f64),
}

/// Largest lean a rider can steer with (rad, 30°).
pub const MAX_STEER_LEAN: f64 = 0.52;
/// Tightest curve a rider steers by leaning, and the largest correction
/// the autopilot steers on top of the road's own curve (1/m, radius 5 m).
pub const MAX_TURN_CURVATURE: f64 = 0.2;
/// Largest angle between the rider and the road (rad, ~17°); in wild mode
/// riders may ride across the road ([`WILD_MAX_HEADING`]).
pub const MAX_HEADING: f64 = 0.3;
/// Largest angle to the road in wild mode (rad, ~70°).
pub const WILD_MAX_HEADING: f64 = 1.2;
/// How far from the road riders in wild mode may go (m, either side).
pub const WILD_LIMIT_M: f64 = 60.0;
/// Up to this share of the largest angle to the road, turning is not
/// limited ([`apply_steering`]); above it, turning away from the road fades
/// out (autodrive stays below it).
const HEADING_KNEE: f64 = 0.7;
/// Deceleration while pushing into the road edge (m/s²).
pub const EDGE_SCRUB_MPS2: f64 = 1.5;
/// Autodrive: largest angle it steers to the road (rad) and how quickly it
/// turns the rider to the heading it wants (s).
const AUTO_MAX_HEADING: f64 = 0.2;
const AUTO_TIME_S: f64 = 0.6;
/// Autodrive aims at the target this far ahead (s of riding, at least 4 m).
const AUTO_LOOKAHEAD_S: f64 = 1.5;

/// Applies `steering` for one tick: heading, lateral offset and progress
/// along the road, and the curvature ridden (`turn_curvature`, absolute).
/// `road_curvature` is the road's at the rider (1/m, positive = right),
/// `limit` the largest offset either side ([`lateral_limit`], or
/// [`WILD_LIMIT_M`] in wild mode). Returns whether the rider scrapes along
/// the road edge (never in wild mode).
pub fn apply_steering(
    state: &mut RiderState,
    steering: Steering,
    road_curvature: f64,
    limit: f64,
    wild: bool,
    dt: f64,
) -> bool {
    let v = state.speed;
    let max_heading = if wild { WILD_MAX_HEADING } else { MAX_HEADING };
    let k = match steering {
        Steering::Auto(target) => {
            let error = target.clamp(-limit, limit) - state.lateral_offset;
            let lookahead = (v * AUTO_LOOKAHEAD_S).max(4.0);
            let wanted = (error / lookahead)
                .atan()
                .clamp(-AUTO_MAX_HEADING, AUTO_MAX_HEADING);
            // The correction is limited, not the road's own curve: the
            // autopilot follows hairpins and the U-turn loop (§2.2.2).
            let correction = ((wanted - state.heading) / (AUTO_TIME_S * v.max(1.0)))
                .clamp(-MAX_TURN_CURVATURE, MAX_TURN_CURVATURE);
            road_curvature + correction
        }
        Steering::Lean(lean) if v > 0.1 => {
            (GRAVITY * lean.clamp(-MAX_STEER_LEAN, MAX_STEER_LEAN).tan() / (v * v))
                .clamp(-MAX_TURN_CURVATURE, MAX_TURN_CURVATURE)
        }
        Steering::Lean(_) => road_curvature,
    };
    let before = state.heading;
    let mut turn = v * (k - road_curvature) * dt;
    if turn * before > 0.0 {
        // Turning further away from the road: the turn fades out towards
        // the largest angle, so the rider (and the avatar's lean) settles
        // there smoothly instead of stopping to turn from one tick to the
        // next.
        turn *= heading_room(before / max_heading);
    }
    state.heading = (before + turn).clamp(-max_heading, max_heading);
    // The curvature actually ridden (for the lean of the avatar).
    state.turn_curvature = if v > 0.1 {
        road_curvature + (state.heading - before) / (v * dt)
    } else {
        0.0
    };
    state.lateral_offset += v * state.heading.sin() * dt;
    // Riding at an angle to the road covers less of it.
    state.distance -= v * (1.0 - state.heading.cos()) * dt;
    let mut scraping = false;
    if state.lateral_offset.abs() >= limit {
        // At the edge: along it, no further outwards.
        state.lateral_offset = state.lateral_offset.clamp(-limit, limit);
        if state.heading * state.lateral_offset > 0.0 {
            state.heading = 0.0;
            // Along the edge, not leaning away from it (the turn was stopped).
            if v > 0.1 {
                state.turn_curvature = road_curvature;
            }
            // Only pushing into the edge scrapes; rolling along it or
            // steering away from it does not.
            scraping = !wild;
        }
    }
    scraping
}

/// Share of a turn away from the road that still applies at `x` = heading
/// / largest heading: all of it up to [`HEADING_KNEE`], then less and less,
/// none at the largest angle.
fn heading_room(x: f64) -> f64 {
    let over = ((x.abs() - HEADING_KNEE) / (1.0 - HEADING_KNEE)).max(0.0);
    (1.0 - over * over).max(0.0)
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
    /// Curvature of the rider's own path in 1/m (positive = right): the
    /// road's plus the rider's steering; the avatar leans for it.
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
    /// Factor on the rolling resistance from the surface (asphalt = 1).
    pub rolling_factor: f64,
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
            rolling_factor: 1.0,
        }
    }

    /// The same environment on another surface.
    pub fn with_surface(self, surface: ocw_world::Surface) -> Self {
        Self {
            rolling_factor: surface.rolling_factor(),
            ..self
        }
    }

    /// The same environment with an extra deceleration (m/s², e.g. scraping
    /// along the road edge) on top of the brake.
    pub fn with_extra_deceleration(self, mps2: f64) -> Self {
        Self {
            brake_mps2: self.brake_mps2 + mps2,
            ..self
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
            profile.crr * env.rolling_factor * m * GRAVITY * angle.cos()
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
    fn leaning_rides_a_curve_and_the_road_does_not_steer() {
        // Straight road, 10 m/s, leaning 10° right: κ = g·tan(10°)/v².
        let mut s = riding(10.0);
        let lean = 10f64.to_radians();
        apply_steering(&mut s, Steering::Lean(lean), 0.0, 3.0, false, 0.02);
        assert!((s.turn_curvature - GRAVITY * lean.tan() / 100.0).abs() < 1e-6);
        for _ in 0..25 {
            apply_steering(&mut s, Steering::Lean(lean), 0.0, 3.0, false, 0.02);
        }
        assert!(s.heading > 0.05 && s.lateral_offset > 0.0, "{s:?}");
        // Upright on a road curving left: the rider keeps going straight,
        // so relative to the road they drift right until the edge.
        let mut s = riding(10.0);
        let mut scraped = false;
        for _ in 0..500 {
            scraped |= apply_steering(&mut s, Steering::Lean(0.0), -0.02, 3.0, false, 0.02);
        }
        assert_eq!(s.lateral_offset, 3.0);
        assert!(scraped);
        // Leaning just right for the curve keeps the line.
        let mut s = riding(10.0);
        let follow = (100.0 * -0.02 / GRAVITY).atan();
        for _ in 0..500 {
            assert!(!apply_steering(
                &mut s,
                Steering::Lean(follow),
                -0.02,
                3.0,
                false,
                0.02
            ));
        }
        assert!(
            s.lateral_offset.abs() < 1e-6 && s.heading.abs() < 1e-9,
            "{s:?}"
        );
        assert!((s.turn_curvature + 0.02).abs() < 1e-9);
    }

    /// Regression: once at the edge, the rider scraped (and was braked) on
    /// every tick, even riding straight along it or steering away.
    #[test]
    fn only_pushing_into_the_edge_scrapes() {
        let at_edge = || RiderState {
            speed: 8.0,
            lateral_offset: 3.0,
            ..Default::default()
        };
        // Rolling along the edge, upright on a straight road.
        let mut s = at_edge();
        for _ in 0..100 {
            assert!(!apply_steering(
                &mut s,
                Steering::Lean(0.0),
                0.0,
                3.0,
                false,
                0.02
            ));
        }
        assert_eq!(s.lateral_offset, 3.0);
        // Autodrive with its line at the edge.
        let mut s = at_edge();
        for _ in 0..100 {
            assert!(!apply_steering(
                &mut s,
                Steering::Auto(3.0),
                0.0,
                3.0,
                false,
                0.02
            ));
        }
        // Leaning into it scrapes; the other edge too.
        let mut s = at_edge();
        assert!(apply_steering(
            &mut s,
            Steering::Lean(0.3),
            0.0,
            3.0,
            false,
            0.02
        ));
        assert_eq!((s.lateral_offset, s.heading), (3.0, 0.0));
        let mut s = RiderState {
            lateral_offset: -3.0,
            ..at_edge()
        };
        assert!(apply_steering(
            &mut s,
            Steering::Lean(-0.3),
            0.0,
            3.0,
            false,
            0.02
        ));
        // A bend that turns the road away under an upright rider pushes too.
        let mut s = at_edge();
        assert!(apply_steering(
            &mut s,
            Steering::Lean(0.0),
            -0.02,
            3.0,
            false,
            0.02
        ));
        // Steering away leaves the edge without scraping.
        let mut s = at_edge();
        for _ in 0..50 {
            assert!(!apply_steering(
                &mut s,
                Steering::Lean(-0.3),
                0.0,
                3.0,
                false,
                0.02
            ));
        }
        assert!(s.lateral_offset < 2.5, "{s:?}");
    }

    /// Regression: the ridden curvature (the avatar's lean) jumped to the
    /// road's when the heading hit its limit, and to the other side when
    /// the edge stopped the rider.
    #[test]
    fn the_lean_settles_smoothly_at_the_largest_angle_and_the_edge() {
        for (limit, wild) in [(1000.0, false), (2.9, false), (WILD_LIMIT_M, true)] {
            let max_heading = if wild { WILD_MAX_HEADING } else { MAX_HEADING };
            let mut s = riding(10.0);
            let lean = Steering::Lean(MAX_STEER_LEAN);
            apply_steering(&mut s, lean, 0.0, limit, wild, 0.02);
            let full = s.turn_curvature;
            assert!(full > 0.05, "{s:?}");
            let mut last = full;
            for _ in 0..1000 {
                apply_steering(&mut s, lean, 0.0, limit, wild, 0.02);
                assert!(s.heading <= max_heading, "{s:?}");
                assert!(s.turn_curvature >= 0.0, "{limit}: {s:?}");
                assert!(
                    (s.turn_curvature - last).abs() < 0.15 * full,
                    "{limit}: jumps from {last} to {s:?}"
                );
                last = s.turn_curvature;
            }
            // It turned as far as it may (or got stopped by the edge).
            if limit > 100.0 {
                assert!(s.heading > 0.97 * max_heading, "{s:?}");
            } else {
                assert_eq!(s.lateral_offset, limit, "{s:?}");
            }
        }
    }

    #[test]
    fn autodrive_holds_its_line_through_curves() {
        let mut s = riding(12.0);
        s.lateral_offset = 2.0;
        let mut worst: f64 = 0.0;
        for i in 0..1500 {
            // Left, straight, then right curves.
            let k = [-0.03, 0.0, 0.025][i / 500];
            apply_steering(&mut s, Steering::Auto(0.0), k, 3.0, false, 0.02);
            if i > 300 {
                worst = worst.max(s.lateral_offset.abs());
            }
        }
        // It got to the middle and stays there through the curves.
        assert!(worst < 0.2, "worst {worst}");
        assert!((s.turn_curvature - 0.025).abs() < 0.002, "{s:?}");
    }

    #[test]
    fn wild_riders_leave_the_road() {
        let mut s = riding(8.0);
        let mut scraped = false;
        for _ in 0..1500 {
            scraped |= apply_steering(&mut s, Steering::Lean(0.3), 0.0, WILD_LIMIT_M, true, 0.02);
            assert!(s.heading <= WILD_MAX_HEADING);
        }
        assert!(!scraped);
        assert_eq!(s.lateral_offset, WILD_LIMIT_M);
        // Riding at an angle covers less road.
        assert!(s.distance < 0.0);
    }

    #[test]
    fn surfaces_and_the_edge_slow_the_rider() {
        let profile = RiderProfile::default();
        let speed_after = |env: Environment| {
            let mut s = riding(10.0);
            for _ in 0..500 {
                step(&mut s, &profile, 150.0, &env, 0.02);
            }
            s.speed
        };
        let flat = Environment::with_grade(0.0);
        let asphalt = speed_after(flat);
        let gravel = speed_after(flat.with_surface(ocw_world::Surface::Gravel));
        let grass = speed_after(flat.with_surface(ocw_world::Surface::Grass));
        let edge = speed_after(flat.with_extra_deceleration(EDGE_SCRUB_MPS2));
        assert!(
            asphalt > gravel + 0.3 && gravel > grass + 0.3,
            "{asphalt} {gravel} {grass}"
        );
        assert!(edge < asphalt - 2.0, "{edge}");
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
