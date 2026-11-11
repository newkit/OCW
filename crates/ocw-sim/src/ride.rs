//! One physics tick of a rider on a route (spec §5.1, §6.1): surface,
//! road edge, longitudinal physics and steering. The server, the client's
//! prediction and its extrapolation all use it, so they agree.

use ocw_core::RiderProfile;
use ocw_world::{Route, Surface};

use crate::physics::{self, Environment, RiderState, Steering};

/// What a rider does besides pedalling.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Controls {
    pub steering: Steering,
    /// Wild mode: not bound to the road (up to [`physics::WILD_LIMIT_M`]).
    pub wild: bool,
    /// Brake key held.
    pub braking: bool,
}

impl Default for Controls {
    fn default() -> Self {
        Self {
            steering: Steering::Auto(0.0),
            wild: false,
            braking: false,
        }
    }
}

/// The surface under a rider: the road's, or grass beside it.
pub fn surface(route: &Route, state: &RiderState) -> Surface {
    if state.lateral_offset.abs() <= route.road_width_m() / 2.0 {
        route.surface_at(state.distance)
    } else {
        Surface::Grass
    }
}

/// Advances `state` by `dt`. `scraping` is whether the rider scraped
/// along the road edge in the previous tick (it slows them down); it is
/// updated for the next one.
#[allow(clippy::too_many_arguments)]
pub fn tick(
    state: &mut RiderState,
    profile: &RiderProfile,
    power_w: f64,
    controls: Controls,
    scraping: &mut bool,
    route: &Route,
    drag_factor: f64,
    dt: f64,
) {
    let env = Environment::with_grade(route.sample(state.distance).grade)
        .with_drag_factor(drag_factor)
        .with_brake(controls.braking)
        .with_surface(surface(route, state))
        .with_extra_deceleration(if *scraping {
            physics::EDGE_SCRUB_MPS2
        } else {
            0.0
        });
    physics::step(state, profile, power_w, &env, dt);
    let limit = if controls.wild {
        physics::WILD_LIMIT_M
    } else {
        physics::lateral_limit(route.road_width_m())
    };
    *scraping = physics::apply_steering(
        state,
        controls.steering,
        route.curvature(state.distance),
        limit,
        controls.wild,
        dt,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bend() -> Route {
        // Straight 200 m, then a 90° bend to the left (radius 60 m), then straight.
        let mut pts: Vec<[f64; 3]> = (0..=40).map(|i| [f64::from(i) * 5.0, 0.0, 0.0]).collect();
        for i in 1..=19 {
            let a = f64::from(i) / 19.0 * std::f64::consts::FRAC_PI_2;
            pts.push([200.0 + 60.0 * a.sin(), 0.0, -60.0 + 60.0 * a.cos()]);
        }
        for i in 1..=40 {
            pts.push([260.0, 0.0, -60.0 - f64::from(i) * 5.0]);
        }
        Route::from_points("b", "B", pts, false, 7.0).unwrap()
    }

    fn ride(controls: Controls, seconds: f64) -> (RiderState, bool) {
        let route = bend();
        let profile = RiderProfile::default();
        let mut s = RiderState {
            speed: 9.0,
            ..Default::default()
        };
        let (mut scraping, mut ever) = (false, false);
        for _ in 0..(seconds / 0.02) as usize {
            tick(
                &mut s,
                &profile,
                250.0,
                controls,
                &mut scraping,
                &route,
                1.0,
                0.02,
            );
            ever |= scraping;
        }
        (s, ever)
    }

    #[test]
    fn autodrive_takes_the_bend_manual_riders_have_to_steer() {
        let (auto, scraped) = ride(Controls::default(), 40.0);
        assert!(!scraped && auto.lateral_offset.abs() < 0.3, "{auto:?}");
        assert!(auto.distance > 300.0);
        // Hands off (upright, manual): the bend sends the rider to the
        // outside edge (right, for a left bend), where it scrapes.
        let manual = Controls {
            steering: Steering::Lean(0.0),
            ..Default::default()
        };
        let (s, scraped) = ride(manual, 40.0);
        assert!(scraped, "{s:?}");
        assert!(
            (s.lateral_offset - physics::lateral_limit(7.0)).abs() < 1e-9,
            "{s:?}"
        );
        // Wild: off the road onto the grass, slower than on the road.
        let wild = Controls {
            wild: true,
            ..manual
        };
        let (w, scraped) = ride(wild, 40.0);
        assert!(!scraped && w.lateral_offset > 3.5, "{w:?}");
        assert!(
            w.distance < auto.distance - 20.0,
            "{} vs {}",
            w.distance,
            auto.distance
        );
    }

    #[test]
    fn gravel_rolls_slower_than_asphalt() {
        let flat = Route::from_points(
            "f",
            "F",
            vec![[0.0, 0.0, 0.0], [5000.0, 0.0, 0.0]],
            false,
            6.0,
        )
        .unwrap();
        let gravel = flat
            .clone()
            .with_surfaces(vec![(0.0, 5000.0, Surface::Gravel)]);
        let profile = RiderProfile::default();
        let go = |route: &Route| {
            let mut s = RiderState::default();
            let mut scraping = false;
            for _ in 0..3000 {
                tick(
                    &mut s,
                    &profile,
                    200.0,
                    Controls::default(),
                    &mut scraping,
                    route,
                    1.0,
                    0.02,
                );
            }
            s.speed
        };
        let (a, g) = (go(&flat), go(&gravel));
        assert!(a > g + 0.5, "asphalt {a}, gravel {g}");
        assert_eq!(
            surface(
                &flat,
                &RiderState {
                    lateral_offset: 3.5,
                    ..Default::default()
                }
            ),
            Surface::Grass
        );
    }
}
