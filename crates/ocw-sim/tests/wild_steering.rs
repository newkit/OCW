//! Wild mode steering at any speed and on any slope (spec 5.2.1 *Wild
//! mode*; the bug of 2026-11-28: "in wild mode, if i ride downhill, i
//! can't control freely where to go, steering seems to be blocked").
//!
//! The band beside the road is a frame along the road: lateral offset and
//! heading to it. Before the fix every bend of the road turned a wild
//! rider at the full rate `v · κ` – also riding across the road or far
//! beside it – and the heading to the road was capped at ~70° (fading
//! from ~49°). Down a winding descent the road's turning grows with the
//! speed while a lean's turn shrinks with it (`g·tan(lean)/v`), so at
//! 50 – 70 km/h the bends took the rider along with them. Now the road
//! turns under the rider only as fast as they ride along it,
//! `cos(heading) / (1 − κ·y)`, the gradient is felt along the way ridden,
//! and the road's own rules (the slow-down before sharp turns, the U-turn
//! loop) are not applied beside it.

use ocw_core::{GRAVITY, RiderProfile, SIM_DT};
use ocw_sim::physics::{self, RiderState, Steering};
use ocw_sim::ride::{self, Controls};
use ocw_sim::{Predictor, RiderInput, Session};
use ocw_world::{Route, Track};

const KMH: [f64; 4] = [10.0, 30.0, 50.0, 70.0];
const GRADES: [f64; 3] = [0.0, -0.08, -0.15];

/// A straight road north (−z), 10 km at `grade`.
fn straight(grade: f64) -> Route {
    let pts = (0..=400)
        .map(|i| {
            let s = f64::from(i) * 25.0;
            [0.0, 2000.0 + grade * s, -s]
        })
        .collect();
    Route::from_points("straight", "Straight", pts, false, 7.0).unwrap()
}

/// A road winding down a valley northwards like a mountain descent: its
/// direction swings ±40° with a 400 m wavelength (bends down to ~90 m
/// radius, the road ~40 m either side of the valley's axis), 10 km at
/// `grade`.
fn winding(grade: f64) -> Route {
    let amp = 40f64.to_radians();
    let mut pts = vec![[0.0, 2000.0, 0.0]];
    let (mut x, mut z, mut y) = (0.0f64, 0.0f64, 2000.0f64);
    for i in 0..4000 {
        let s = f64::from(i) * 2.5;
        let th = amp * (std::f64::consts::TAU * s / 400.0).sin();
        x += 2.5 * th.sin();
        z -= 2.5 * th.cos();
        y += grade * 2.5;
        pts.push([x, y, z]);
    }
    Route::from_points("winding", "Winding", pts, false, 7.0).unwrap()
}

/// The rider's direction of travel in the world (rad, 0 = north,
/// positive = right / east).
fn yaw(track: &dyn Track, st: &RiderState) -> f64 {
    let d = track.sample(st.distance).direction;
    let (s, c) = st.heading.sin_cos();
    let v = [d[0] * c - d[1] * s, d[0] * s + d[1] * c];
    v[0].atan2(-v[1])
}

fn wrap(a: f64) -> f64 {
    (a + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI
}

fn wild(lean: f64) -> Controls {
    Controls {
        steering: Steering::Lean(lean),
        wild: true,
        braking: false,
    }
}

/// Rides `seconds` in wild mode at `kmh` (held: the steering, not the
/// speed, is tested) with the lean `lean(time, state, yaw)`; `each` sees
/// every tick.
fn ride_wild(
    track: &dyn Track,
    kmh: f64,
    seconds: f64,
    mut lean: impl FnMut(f64, &RiderState, f64) -> f64,
    mut each: impl FnMut(f64, &RiderState, f64),
) -> RiderState {
    let v = kmh / 3.6;
    let profile = RiderProfile::default();
    let mut st = RiderState {
        distance: 500.0,
        speed: v,
        ..Default::default()
    };
    let mut scraping = false;
    for i in 0..(seconds / SIM_DT).round() as usize {
        let t = i as f64 * SIM_DT;
        let l = lean(t, &st, yaw(track, &st));
        ride::tick(
            &mut st,
            &profile,
            0.0,
            wild(l),
            &mut scraping,
            track,
            1.0,
            SIM_DT,
        );
        st.speed = v;
        each(t + SIM_DT, &st, yaw(track, &st));
    }
    st
}

/// A rider's lean that turns them to the world direction `want`.
fn towards(want: f64, y: f64) -> f64 {
    (6.0 * wrap(want - y)).clamp(-physics::MAX_STEER_LEAN, physics::MAX_STEER_LEAN)
}

/// A full lean turns the rider as a bike turns, r = v² / (g·tan(lean))
/// (at least 5 m): on the flat and downhill, at 10 – 70 km/h, to both
/// sides, the direction changes by the expected amount (straight road, up
/// to 75° to it; before the fix the turn faded from ~49° and stopped at
/// ~69°: at 70 km/h a full lean turned 50° in 3 s, now 72°).
#[test]
fn a_full_lean_turns_as_a_bike_at_every_speed_and_slope() {
    let max = physics::MAX_STEER_LEAN;
    for g in GRADES {
        let road = straight(g);
        for kmh in KMH {
            let v = kmh / 3.6;
            let k = (GRAVITY * max.tan() / (v * v)).min(physics::MAX_TURN_CURVATURE);
            let rate = v * k; // rad/s
            let t = (75f64.to_radians() / rate).min(3.0);
            for side in [1.0, -1.0] {
                let st = ride_wild(&road, kmh, t, |_, _, _| side * max, |_, _, _| {});
                let turned = side * yaw(&road, &st);
                assert!(
                    (turned - rate * t).abs() < 0.03 * rate * t + 0.01,
                    "{g} {kmh} km/h: turned {:.1}° in {t:.2} s, expected {:.1}°",
                    turned.to_degrees(),
                    (rate * t).to_degrees()
                );
            }
        }
    }
}

/// The direction a test rider steers to at `t` s (rad, 0 = north, down the
/// valley): north, 30° right, 30° left, north – 6 s each.
fn wanted(t: f64) -> f64 {
    [0.0, 30f64.to_radians(), -30f64.to_radians(), 0.0][((t / 6.0) as usize).min(3)]
}

/// The worst error of the direction ridden against [`wanted`] down the
/// winding road at `kmh` (rad): from 3 s after each change of direction
/// and 2 s after the rider last touched the band's edge (along it the
/// rider rides along the road).
fn worst_heading_error(road: &dyn Track, kmh: f64) -> f64 {
    let mut worst = 0f64;
    let mut edge_at = -10.0;
    ride_wild(
        road,
        kmh,
        24.0,
        |t, _, y| towards(wanted(t), y),
        |t, st, y| {
            if st.lateral_offset.abs() >= physics::WILD_LIMIT_M - 1e-6 {
                edge_at = t;
            }
            if t % 6.0 > 3.0 && t - edge_at > 2.0 {
                worst = worst.max(wrap(wanted(t) - y).abs());
            }
        },
    );
    worst
}

/// The regression: down a winding descent the rider holds the direction
/// they steer to – down the valley, 30° right, 30° left – at 30 – 70
/// km/h, while the road bends under them. Before the fix the bends took
/// a wild rider with them.
#[test]
fn down_a_winding_descent_the_rider_goes_where_they_steer() {
    let mut bad = Vec::new();
    for g in GRADES {
        let road = winding(g);
        for kmh in [30.0, 50.0, 70.0] {
            let worst = worst_heading_error(&road, kmh).to_degrees();
            println!("{g} {kmh} km/h: worst error {worst:.1}°");
            if worst >= 3.0 {
                bad.push(format!("{g} {kmh} km/h: {worst:.1}°"));
            }
        }
    }
    assert!(bad.is_empty(), "worst errors: {bad:?}");
}

/// Down the winding descent at 50 km/h the rider reaches both sides of
/// wild mode's 60 m band (and rides along its edge there).
#[test]
fn the_band_is_reached_on_both_sides_downhill() {
    for g in [-0.08, -0.15] {
        let road = winding(g);
        let (mut lo, mut hi) = (0f64, 0f64);
        ride_wild(
            &road,
            50.0,
            40.0,
            |t, _, y| towards(if t < 15.0 { 1.2 } else { -1.2 }, y),
            |_, st, _| {
                lo = lo.min(st.lateral_offset);
                hi = hi.max(st.lateral_offset);
            },
        );
        assert_eq!(
            (lo, hi),
            (-physics::WILD_LIMIT_M, physics::WILD_LIMIT_M),
            "{g}"
        );
    }
}

/// Riding across a descent in wild mode is flat, not downhill: the road's
/// gradient is felt along the way ridden (`cos(heading)` of it).
#[test]
fn across_a_descent_the_gradient_is_the_way_ridden() {
    let road = straight(-0.15);
    let profile = RiderProfile::default();
    let coast = |heading: f64| {
        let mut st = RiderState {
            distance: 500.0,
            speed: 8.0,
            heading,
            ..Default::default()
        };
        let mut scraping = false;
        for _ in 0..100 {
            ride::tick(
                &mut st,
                &profile,
                0.0,
                wild(0.0),
                &mut scraping,
                &road,
                1.0,
                SIM_DT,
            );
        }
        st.speed
    };
    // Down the road: gravity pulls (15 % ≈ 1.45 m/s² for 2 s).
    assert!(coast(0.0) > 10.0, "{}", coast(0.0));
    // Nearly across: hardly any pull, the rolling resistance of the meadow
    // and the air slow the rider down.
    let across = coast(physics::WILD_MAX_HEADING);
    assert!(across < 8.1, "{across}");
}

/// A route with a sharp turn's slow-down (10 km/h) and a U-turn loop's
/// steering everywhere, as a free-ride path has them before its turns.
struct Hairpin(Route);

impl Track for Hairpin {
    fn sample(&self, d: f64) -> ocw_world::RoutePoint {
        self.0.sample(d)
    }
    fn curvature(&self, d: f64) -> f64 {
        self.0.curvature(d)
    }
    fn surface_at(&self, d: f64) -> ocw_world::Surface {
        self.0.surface_at(d)
    }
    fn road_width_at(&self, _d: f64) -> f64 {
        self.0.road_width_m()
    }
    fn speed_limit(&self, _d: f64) -> Option<f64> {
        Some(10.0 / 3.6)
    }
    fn assisted(&self, _d: f64) -> bool {
        true
    }
}

/// The road's own rules are the road's: a wild rider 30 m beside it is
/// neither braked for its hairpin nor steered through its U-turn loop; on
/// the road (wild or not) they still are.
#[test]
fn beside_the_road_its_rules_do_not_apply() {
    let road = Hairpin(straight(0.0));
    let profile = RiderProfile::default();
    let ride = |lateral: f64, controls: Controls| {
        let mut st = RiderState {
            distance: 500.0,
            speed: 12.0,
            lateral_offset: lateral,
            ..Default::default()
        };
        let mut scraping = false;
        for _ in 0..100 {
            ride::tick(
                &mut st,
                &profile,
                300.0,
                controls,
                &mut scraping,
                &road,
                1.0,
                SIM_DT,
            );
        }
        st
    };
    let lean = physics::MAX_STEER_LEAN;
    let beside = ride(30.0, wild(lean));
    assert!(beside.speed > 10.0, "not braked: {beside:?}");
    assert!(beside.heading > 0.5, "steered by the lean: {beside:?}");
    let on = ride(0.0, wild(lean));
    assert!(on.speed < 8.0, "braked on the road: {on:?}");
    assert!(on.heading.abs() < 0.05, "steered through the loop: {on:?}");
}

/// The server and the client's prediction agree on a wild ride down the
/// winding descent, carving left and right (one code path, spec 6.1).
#[test]
fn the_prediction_rides_the_servers_wild_descent() {
    let road = winding(-0.08);
    let mut s = Session::new(road.clone());
    let me = s.add_rider("Ada Rider", RiderProfile::default());
    s.apply_input(
        me,
        1,
        RiderInput {
            power_w: 150.0,
            ..Default::default()
        },
    );
    for _ in 0..(5.0 / SIM_DT) as usize {
        s.step();
    }
    assert!(s.set_wild(me, true));
    let start = s.rider_state(me).unwrap();
    let mut p = Predictor::new(s.rider_profile(me).unwrap(), start);
    p.set_wild(true);
    let mut seq = 10;
    for i in 0..(30.0 / SIM_DT) as usize {
        if i % 250 == 0 {
            // Carve: right, left, right, … at the full lean.
            let lean = if (i / 250) % 2 == 0 { 1.0 } else { -1.0 } * physics::MAX_STEER_LEAN;
            seq += 1;
            assert!(s.set_lean(me, seq, lean));
            p.set_lean(lean);
        }
        s.step();
        p.step(&road, 150.0);
    }
    let (a, b) = (s.rider_state(me).unwrap(), p.state());
    assert!(a.lateral_offset.abs() > 5.0, "it left the road: {a:?}");
    assert!(
        (a.distance - b.distance).abs() < 0.05
            && (a.lateral_offset - b.lateral_offset).abs() < 0.05
            && (a.heading - b.heading).abs() < 1e-3,
        "server {a:?} vs prediction {b:?}"
    );
}
