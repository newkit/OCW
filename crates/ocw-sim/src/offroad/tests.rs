//! Riding off the road (spec §14 Phase 13A): the terrain's gradients
//! (Q7 (b): 40 % up and down, a cliff), water and the world's edge, the
//! cover's rolling resistance, landing allowed and refused (Q8 (a)),
//! joining a road again.

use super::test_world::{TREE, height, world};
use super::*;
use ocw_core::{RiderProfile, SIM_DT};

fn ground() -> Arc<GroundMap> {
    world().ground().unwrap().clone()
}

/// Rides `seconds` off the road from (x, z) along `yaw` with `watts`
/// (leaning `lean`); returns the states and poses of every tick.
fn ride(
    from: [f64; 2],
    yaw: f64,
    speed: f64,
    watts: f64,
    lean: f64,
    seconds: f64,
) -> Vec<(RiderState, OffRoadPose)> {
    let g = ground();
    let mut state = RiderState {
        distance: 100.0,
        speed,
        ..Default::default()
    };
    let mut track = OffRoadPose {
        x: from[0],
        z: from[1],
        yaw,
    }
    .track(g, state.distance);
    let controls = Controls {
        steering: Steering::Lean(lean),
        wild: true,
        braking: false,
    };
    let mut out = Vec::new();
    let mut scraping = false;
    for _ in 0..(seconds / SIM_DT).round() as usize {
        let (next, _) = tick(
            &mut state,
            &track,
            &RiderProfile::default(),
            watts,
            controls,
            &mut scraping,
            1.0,
            None,
            SIM_DT,
        );
        track = next;
        out.push((state, OffRoadPose::of(&track)));
    }
    out
}

/// Yaw riding towards −z (north) and +z (south) and +x (east).
const NORTH: f64 = 0.0;
const SOUTH: f64 = std::f64::consts::PI;
const EAST: f64 = -std::f64::consts::FRAC_PI_2;

fn sane(path: &[(RiderState, OffRoadPose)]) {
    let g = ground();
    let mut last: Option<OffRoadPose> = None;
    for (st, p) in path {
        assert!(st.speed.is_finite() && st.distance.is_finite(), "{st:?}");
        assert!(
            p.x.is_finite() && p.z.is_finite() && p.yaw.is_finite(),
            "{p:?}"
        );
        assert!((0.0..=MAX_SPEED_MPS).contains(&st.speed), "{st:?}");
        assert_eq!((st.lateral_offset, st.heading), (0.0, 0.0));
        if let Some(l) = last {
            let step = (p.x - l.x).hypot(p.z - l.z);
            assert!(step <= MAX_SPEED_MPS * SIM_DT + 1e-9, "step {step}");
        }
        // On the ground, never in the air.
        assert!(g.height(p.x, p.z).is_some());
        last = Some(*p);
    }
}

#[test]
fn a_forty_percent_climb_is_ridden_at_its_real_gradient() {
    // From the meadow south up the ramp (towards −z is north: up).
    let up = ride([500.0, 380.0], NORTH, 3.0, 300.0, 0.0, 30.0);
    sane(&up);
    let g = ground();
    let (st, p) = up.last().unwrap();
    // Slow but moving: 300 W on 40 % (~22° of slope) gives ~1 m/s.
    assert!(st.speed > 0.5 && st.speed < 2.0, "{st:?}");
    let climbed = g.height(p.x, p.z).unwrap() - g.height(500.0, 380.0).unwrap();
    assert!(climbed > 8.0, "{climbed} m");
    // The gradient the track gives is the real 40 %.
    let t = p.track(g.clone(), st.distance);
    assert!((t.sample(st.distance).grade - 0.4).abs() < 0.01);
    // Without power the rider stops and never rolls back.
    let stop = ride([500.0, 300.0], NORTH, 6.0, 0.0, 0.0, 10.0);
    sane(&stop);
    assert_eq!(stop.last().unwrap().0.speed, 0.0);
    let z: Vec<f64> = stop.iter().map(|(_, p)| p.z).collect();
    assert!(z.windows(2).all(|w| w[1] <= w[0] + 1e-9), "rolled back");
}

#[test]
fn a_forty_percent_descent_speeds_up_without_a_cap() {
    // Down the ramp towards the road (south, +z).
    let down = ride([500.0, 50.0], SOUTH, 2.0, 0.0, 0.0, 12.0);
    sane(&down);
    let top = down.iter().map(|(s, _)| s.speed).fold(0.0, f64::max);
    // Well past any road speed limit, still finite.
    assert!(top > 20.0, "{top}");
    // Horizontally the step is the slope's length times its cosine.
    let (a, b) = (down[100], down[101]);
    let along = b.0.distance - a.0.distance;
    let run = (b.1.z - a.1.z).abs();
    assert!(
        (run - along * 0.4f64.atan().cos()).abs() < 0.02,
        "{run} vs {along}"
    );
}

#[test]
fn over_a_cliff_the_numbers_stay_finite_and_on_the_ground() {
    // South towards the 200 m drop at z = 300 (x ≥ 1000), riding off the top.
    let off = ride([1500.0, 250.0], SOUTH, 8.0, 200.0, 0.0, 20.0);
    sane(&off);
    let g = ground();
    let (st, p) = off.last().unwrap();
    assert!(p.z > 310.0, "went down: {p:?}");
    assert!(g.height(p.x, p.z).unwrap().abs() < 0.5, "at the foot");
    assert!(st.speed < MAX_SPEED_MPS);
    // Up the cliff: the gradient is bounded, the rider crawls, nothing jumps.
    let up = ride([1500.0, 320.0], NORTH, 2.0, 400.0, 0.0, 20.0);
    sane(&up);
}

#[test]
fn water_and_the_worlds_edge_stop_the_rider_and_leaning_turns_away() {
    // East along z = 700 into the lake at x = 1200.
    let into = ride([1100.0, 700.0], EAST, 8.0, 200.0, 0.0, 30.0);
    sane(&into);
    let (st, p) = into.last().unwrap();
    assert!(p.x < 1200.0 && p.x > 1180.0, "at the shore: {p:?}");
    assert_eq!(st.speed, 0.0);
    // Leaning wheels the bike round on the spot, then it rides away.
    let away = ride([p.x, p.z], p.yaw, 0.0, 200.0, 0.3, 6.0);
    sane(&away);
    let end = away.last().unwrap().1;
    assert!((end.x - p.x).hypot(end.z - p.z) > 3.0, "{end:?}");
    // The world's edge (x = 2000) too.
    let edge = ride([1950.0, 900.0], EAST, 8.0, 200.0, 0.0, 20.0);
    sane(&edge);
    assert!(edge.last().unwrap().1.x < 2000.0);
}

#[test]
fn the_cover_rolls_as_its_surface() {
    // The same power on meadow, rock and forest floor: rock fastest.
    let speed = |x: f64| {
        ride([x, 700.0], NORTH, 0.0, 200.0, 0.0, 40.0)
            .last()
            .unwrap()
            .0
            .speed
    };
    let (meadow, rock, forest) = (speed(1000.0), speed(1700.0), speed(700.0));
    assert!(rock > meadow && meadow > forest, "{rock} {meadow} {forest}");
}

#[test]
fn leaning_rides_a_circle_on_the_terrain() {
    let circle = ride([1000.0, 800.0], NORTH, 6.0, 120.0, 0.25, 30.0);
    sane(&circle);
    let start = circle[0].1;
    // Back near the start after full circles (radius v²/(g tan l) ~ 14 m).
    let closest = circle[100..]
        .iter()
        .map(|(_, p)| (p.x - start.x).hypot(p.z - start.z))
        .fold(f64::INFINITY, f64::min);
    assert!(closest < 6.0, "{closest}");
}

#[test]
fn landing_is_allowed_on_land_and_refused_on_water_buildings_and_obstacles() {
    let net = world();
    let mut cache = WorldCache::new(net.obstacles().cloned());
    let mut at = |x: f64, z: f64| landing(&net, x, z, NORTH, Some(&mut cache), 1);
    // A meadow.
    assert!(matches!(at(900.0, 800.0), Landing::Ground { .. }));
    // Near the road (within its half width + 2 m): on the road.
    match at(500.0, 504.5) {
        Landing::Road { at, .. } => assert!((at[2] - 500.0).abs() < 1e-6, "{at:?}"),
        l => panic!("{l:?}"),
    }
    // Farther: off the road beside it.
    assert!(matches!(at(500.0, 510.0), Landing::Ground { .. }));
    // The lake, the house, the obstacle, outside the world.
    for (x, z, why) in [
        (1300.0, 700.0, "water"),
        (306.0, 705.0, "building"),
        (f64::from(TREE[0]) + 0.3, f64::from(TREE[1]), "obstacle"),
        (2500.0, 700.0, "outside"),
        (500.0, -50.0, "outside"),
    ] {
        match at(x, z) {
            Landing::Refused { why: w, .. } => assert!(w.contains(why), "{x},{z}: {w}"),
            l => panic!("{x},{z}: {l:?}"),
        }
    }
    // On the water the ring floats on its surface.
    let l = at(1300.0, 700.0);
    assert!(
        (l.at()[1] - super::test_world::LAKE_LEVEL).abs() < 1e-6,
        "{l:?}"
    );
    // The ring stands on the ground there.
    let l = at(500.0, 100.0);
    assert!(
        l.allowed() && (l.at()[1] - height(500.0, 100.0)).abs() < 0.05,
        "{l:?}"
    );
}

#[test]
fn a_rider_on_the_road_surface_joins_it_in_the_direction_of_travel() {
    let net = world();
    // On the carriageway, riding east at 30° to it: joins forwards.
    let dir = crate::god::forward(EAST + 0.5);
    let j = road_join(&net, [400.0, 501.0], 0.0, dir).unwrap();
    assert!(j.forward && (j.offset_m - 400.0).abs() < 1e-6, "{j:?}");
    // Heading to the right of the road is positive (+z is right of +x).
    assert!((j.heading.abs() - 0.5).abs() < 1e-9, "{j:?}");
    assert!((j.lateral_m - 1.0).abs() < 1e-6, "{j:?}");
    // Riding west: backwards along the edge, the side flips.
    let j = road_join(&net, [400.0, 501.0], 0.0, crate::god::forward(-EAST)).unwrap();
    assert!(!j.forward && (j.lateral_m + 1.0).abs() < 1e-6, "{j:?}");
    // Crossing at a right angle, beside the road, or high above it: no.
    assert!(road_join(&net, [400.0, 501.0], 0.0, crate::god::forward(NORTH)).is_none());
    assert!(road_join(&net, [400.0, 505.0], 0.0, dir).is_none());
    assert!(road_join(&net, [400.0, 501.0], 8.0, dir).is_none());
}

#[test]
fn normalize_folds_a_push_into_the_track() {
    let g = ground();
    let t = OffRoadPose {
        x: 900.0,
        z: 800.0,
        yaw: NORTH,
    }
    .track(g, 50.0);
    let mut st = RiderState {
        distance: 52.0,
        lateral_offset: 0.5,
        heading: 0.1,
        ..Default::default()
    };
    let n = normalize(&mut st, &t);
    let p = OffRoadPose::of(&n);
    // 2 m north and 0.5 m right (east) of where it was, turned right.
    assert!(
        (p.x - 900.5).abs() < 1e-9 && (p.z - 798.0).abs() < 1e-9,
        "{p:?}"
    );
    assert!((p.yaw - (NORTH - 0.1)).abs() < 1e-9, "{p:?}");
    assert_eq!((st.lateral_offset, st.heading), (0.0, 0.0));
    assert_eq!(n.anchor_m(), 52.0);
    // Interpolation takes the short way round.
    let a = OffRoadPose {
        x: 0.0,
        z: 0.0,
        yaw: 3.1,
    };
    let b = OffRoadPose {
        x: 2.0,
        z: 0.0,
        yaw: -3.1,
    };
    let m = OffRoadPose::lerp(a, b, 0.5);
    assert!((m.x - 1.0).abs() < 1e-12 && (m.yaw.abs() - std::f64::consts::PI).abs() < 1e-9);
}
