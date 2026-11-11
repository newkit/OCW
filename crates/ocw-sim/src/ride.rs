//! One physics tick of a rider on a track (spec §5.1, §6.1): surface,
//! road edge, automatic slow-down, longitudinal physics and steering. The
//! server, the client's prediction and its extrapolation all use it, so
//! they agree. The track is a fixed route or a free-ride path.

use ocw_core::RiderProfile;
use ocw_world::{Surface, Track};

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

/// Deceleration of the automatic slow-down before sharp turns and U-turns
/// (m/s², spec §5.2): applied while the rider is faster than the track's
/// speed limit there.
pub const SLOW_DOWN_DECELERATION_MPS2: f64 = 3.0;

/// The surface under a rider: the road's, or grass beside it.
pub fn surface(track: &dyn Track, state: &RiderState) -> Surface {
    if state.lateral_offset.abs() <= track.road_width_at(state.distance) / 2.0 {
        track.surface_at(state.distance)
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
    track: &dyn Track,
    drag_factor: f64,
    dt: f64,
) {
    let d = state.distance;
    let grade = track.sample(d).grade;
    // Downhill the slow-down also holds against gravity.
    let slow_down = match track.speed_limit(d) {
        Some(v) if state.speed > v => {
            SLOW_DOWN_DECELERATION_MPS2 + (-grade).max(0.0) * ocw_core::GRAVITY
        }
        _ => 0.0,
    };
    let scrub = if *scraping {
        physics::EDGE_SCRUB_MPS2
    } else {
        0.0
    };
    let env = Environment::with_grade(grade)
        .with_drag_factor(drag_factor)
        .with_brake(controls.braking)
        .with_surface(surface(track, state))
        .with_extra_deceleration(scrub + slow_down);
    physics::step(state, profile, power_w, &env, dt);
    let limit = if controls.wild {
        physics::WILD_LIMIT_M
    } else {
        physics::lateral_limit(track.road_width_at(state.distance))
    };
    // Through a U-turn loop (and on its run-up) the rider is steered along
    // the centreline, also when leaning: the loop is centred on it and only
    // as wide as the road (spec §2.2.2).
    let steering = if track.assisted(state.distance) {
        Steering::Auto(0.0)
    } else {
        controls.steering
    };
    *scraping = physics::apply_steering(
        state,
        steering,
        track.curvature(state.distance),
        limit,
        controls.wild,
        dt,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_world::Route;

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

    fn straight() -> Route {
        Route::from_points(
            "s",
            "S",
            vec![[0.0, 0.0, 0.0], [5000.0, 0.0, 0.0]],
            false,
            7.0,
        )
        .unwrap()
    }

    /// Runs `ticks` manual ticks with `lean` and `power_w` on `route`.
    fn lean_for(
        s: &mut RiderState,
        scraping: &mut bool,
        route: &Route,
        lean: f64,
        power_w: f64,
        seconds: f64,
    ) {
        let controls = Controls {
            steering: Steering::Lean(lean),
            ..Default::default()
        };
        for _ in 0..(seconds / 0.02) as usize {
            tick(
                s,
                &RiderProfile::default(),
                power_w,
                controls,
                scraping,
                route,
                1.0,
                0.02,
            );
        }
    }

    /// Regression: steered hard into the edge and then upright, the rider
    /// kept scraping along it and was braked almost to a standstill.
    #[test]
    fn upright_along_the_edge_the_rider_keeps_rolling_and_can_steer_away() {
        let route = straight();
        let limit = physics::lateral_limit(7.0);
        let mut s = RiderState {
            speed: 9.0,
            ..Default::default()
        };
        let mut scraping = false;
        lean_for(&mut s, &mut scraping, &route, 0.4, 250.0, 3.0);
        assert_eq!(s.lateral_offset, limit, "{s:?}");
        assert!(scraping);
        let pushed = s.speed;
        // Upright along the edge: no more scraping, back to cruising speed.
        lean_for(&mut s, &mut scraping, &route, 0.0, 250.0, 10.0);
        assert!(!scraping);
        assert!(s.speed > pushed + 1.0 && s.speed > 8.0, "{pushed} {s:?}");
        // Leaning the other way leaves the edge.
        lean_for(&mut s, &mut scraping, &route, -0.3, 250.0, 1.0);
        assert!(s.lateral_offset < limit - 1.0, "{s:?}");
    }

    /// Regression: stopped at the edge, the scraping brake held a rider
    /// pedalling gently at a standstill for ever.
    #[test]
    fn stopped_at_the_edge_the_rider_gets_going_again() {
        let route = straight();
        let limit = physics::lateral_limit(7.0);
        let mut s = RiderState {
            lateral_offset: limit,
            ..Default::default()
        };
        // As after pushing into the edge until stopped.
        let mut scraping = true;
        lean_for(&mut s, &mut scraping, &route, -0.1, 100.0, 3.0);
        assert!(s.speed > 2.0, "{s:?}");
        assert!(s.lateral_offset < limit - 0.5, "{s:?}");
    }

    #[test]
    fn the_slow_down_holds_on_steep_descents() {
        use ocw_world::graph::{Edge, EdgeId, Oneway, RoadClass, RoadGraph, VertexId};
        use ocw_world::{RidePath, RoadNetwork};
        // A 600 m descent at 12 %, then a 90° turn into a flat road, plus a
        // third arm so the corner is a junction.
        let e = |id: u64, a: u32, b: u32, p: Vec<[f64; 2]>, prof: Vec<f64>| Edge {
            id: EdgeId(id),
            from: VertexId(a),
            to: VertexId(b),
            points: p,
            class: RoadClass::Secondary,
            name: None,
            width_m: 6.0,
            bridge: false,
            tunnel: false,
            dead_end_m: None,
            boundary: false,
            oneway: Oneway::No,
            osm_way: id as i64,
            profile: prof,
        };
        let descent: Vec<f64> = (0..=120).map(|i| 72.0 - f64::from(i) * 0.6).collect();
        let graph = RoadGraph {
            vertices: vec![[0.0, 0.0], [600.0, 0.0], [600.0, 400.0], [1000.0, 0.0]],
            edges: vec![
                e(1, 0, 1, vec![[0.0, 0.0], [600.0, 0.0]], descent),
                e(2, 1, 2, vec![[600.0, 0.0], [600.0, 400.0]], vec![0.0; 81]),
                e(3, 1, 3, vec![[600.0, 0.0], [1000.0, 0.0]], vec![0.0; 81]),
            ],
            boundary_vertices: Default::default(),
        };
        let net = std::sync::Arc::new(RoadNetwork::new(graph));
        let mut path = RidePath::start(net, 0, 0.0, true, 0.0, 0);
        let profile = RiderProfile::default();
        let mut s = RiderState::default();
        let mut scraping = false;
        let mut at_corner = None;
        for _ in 0..(120 * 50) {
            // Turn right at the bottom.
            path.extend_at_speed(s.distance, s.speed, &mut |j| {
                j.exits.iter().position(|x| x.angle_deg > 45.0).unwrap()
            });
            tick(
                &mut s,
                &profile,
                250.0,
                Controls::default(),
                &mut scraping,
                &path,
                1.0,
                0.02,
            );
            if at_corner.is_none() && s.distance >= 597.0 {
                at_corner = Some(s.speed);
            }
        }
        // ~60 km/h down the hill, ~22 km/h through the corner.
        let v = at_corner.unwrap();
        assert!(v < 7.0, "{} km/h at the corner", v * 3.6);
        assert!(s.distance > 700.0);
    }

    /// Regression: the U-turn loop (4 m radius, 8 m wide) left narrow roads,
    /// and riders kept their lateral offset through it, so the avatar
    /// dropped off the road surface during the turn.
    #[test]
    fn u_turns_stay_on_the_road() {
        use ocw_world::graph::{Edge, EdgeId, Oneway, RoadClass, RoadGraph, VertexId};
        use ocw_world::{RidePath, RoadNetwork};
        for (width, start_lateral) in [(3.0, 0.9), (5.0, 1.9), (6.5, -2.6), (7.5, 3.1)] {
            let graph = RoadGraph {
                vertices: vec![[0.0, 0.0], [1200.0, 0.0]],
                edges: vec![Edge {
                    id: EdgeId(1),
                    from: VertexId(0),
                    to: VertexId(1),
                    points: vec![[0.0, 0.0], [1200.0, 0.0]],
                    class: RoadClass::Tertiary,
                    name: None,
                    width_m: width,
                    bridge: false,
                    tunnel: false,
                    dead_end_m: None,
                    boundary: false,
                    oneway: Oneway::No,
                    osm_way: 1,
                    profile: Vec::new(),
                }],
                boundary_vertices: Default::default(),
            };
            let net = std::sync::Arc::new(RoadNetwork::new(graph));
            let mut path = RidePath::start(net, 0, 400.0, true, 0.0, 0);
            path.extend(0.0, &mut |j| j.default);
            let profile = RiderProfile::default();
            let mut s = RiderState {
                speed: 9.0,
                lateral_offset: start_lateral,
                ..Default::default()
            };
            let controls = Controls {
                steering: Steering::Auto(start_lateral),
                ..Default::default()
            };
            let mut scraping = false;
            let mut turned_at = None;
            let mut worst: f64 = 0.0;
            for _ in 0..(40 * 50) {
                if turned_at.is_none() && s.distance > 50.0 {
                    turned_at = path.u_turn(s.distance, s.speed);
                    assert!(turned_at.is_some());
                }
                tick(
                    &mut s,
                    &profile,
                    200.0,
                    controls,
                    &mut scraping,
                    &path,
                    1.0,
                    0.02,
                );
                // Where the rider is: sideways from the road's centreline.
                let at = path.sample(s.distance).offset_position(s.lateral_offset);
                worst = worst.max(at[2].abs());
            }
            // Back on the way west, well after the loop.
            assert!(
                path.sample(s.distance).direction[0] < -0.99,
                "{width}: {s:?}"
            );
            let (at, _) = path.turnaround_ahead(0.0).unwrap();
            assert!(s.distance > at + 100.0, "{width}: {s:?}");
            assert!(
                worst <= width / 2.0 + 0.05,
                "{width} m road: {worst:.2} m from the centreline"
            );
        }
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
