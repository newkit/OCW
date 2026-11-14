//! Weather physics (spec §14 Phase 9, S6; the 9G unit tests): a headwind
//! needs more power for the same speed, a tailwind less, a crosswind none;
//! drafting offset in a crosswind; air density from temperature / altitude;
//! a wet road; the switch off is the physics of before, bit for bit; the
//! client's prediction rides the same weather as the server.

use ocw_core::{RiderId, RiderProfile, SIM_DT};
use ocw_sim::physics::{self, Environment, RiderState, Steering};
use ocw_sim::ride::{self, Controls};
use ocw_sim::weather::{self, WeatherPhysics};
use ocw_sim::{EventSpec, Predictor, RiderInput, Session};
use ocw_weather::{Ground, WeatherState};
use ocw_world::Route;

/// A straight road north (−z), 20 km, flat at scene height `y`.
fn north(y: f64) -> Route {
    let pts = (0..=200).map(|i| [0.0, y, -f64::from(i) * 100.0]).collect();
    Route::from_points("n", "North", pts, false, 8.0).unwrap()
}

/// Calm, 15 °C, 1013.25 hPa at sea level, dry air and road.
fn standard() -> WeatherState {
    WeatherState {
        wind_mps: 0.0,
        gust_mps: 0.0,
        temperature_c: 15.0,
        pressure_hpa: 1013.25,
        humidity: 0.0,
        ..Default::default()
    }
}

/// The standard air with a steady 10 m wind of `mps` from `from_deg`.
fn wind(mps: f32, from_deg: f32) -> WeatherPhysics {
    let w = WeatherState {
        wind_mps: mps,
        gust_mps: mps,
        wind_from_deg: from_deg,
        ..standard()
    };
    WeatherPhysics::new(&w, mps, &Ground::default(), 0.0)
}

/// The steady power (W) to ride `speed` on `route` in `w` (`None`: as
/// before, no weather).
fn power_for(route: &Route, w: Option<&WeatherPhysics>, speed: f64) -> f64 {
    let profile = RiderProfile::default();
    let state = RiderState {
        distance: 1000.0,
        speed,
        ..Default::default()
    };
    let env = match w {
        None => Environment::with_grade(0.0),
        Some(w) => {
            let h = weather::at_rider(w, route, &state);
            Environment::with_grade(0.0).in_weather(
                h.wind.head_mps,
                h.density,
                h.weather.rolling_factor,
            )
        }
    };
    physics::steady_state_power(&profile, &env, speed)
}

/// Speed (m/s) after riding `seconds` at `watts` on `route` in `w`.
fn ride_for(route: &Route, w: Option<&WeatherPhysics>, watts: f64, seconds: f64) -> RiderState {
    let profile = RiderProfile::default();
    let mut s = RiderState {
        distance: 100.0,
        speed: 8.0,
        ..Default::default()
    };
    let mut scraping = false;
    for _ in 0..(seconds / SIM_DT) as usize {
        ride::tick_in(
            &mut s,
            &profile,
            watts,
            Controls::default(),
            &mut scraping,
            route,
            1.0,
            w,
            SIM_DT,
        );
    }
    s
}

#[test]
fn headwind_needs_more_power_tailwind_less_crosswind_none() {
    let road = north(0.0);
    let v = 10.0; // 36 km/h
    let calm = power_for(&road, Some(&wind(0.0, 0.0)), v);
    // Calm standard air is the physics of before.
    assert!((calm - power_for(&road, None, v)).abs() < 0.01, "{calm}");
    let head = power_for(&road, Some(&wind(8.0, 0.0)), v); // from the north
    let tail = power_for(&road, Some(&wind(8.0, 180.0)), v);
    let cross_e = power_for(&road, Some(&wind(8.0, 90.0)), v);
    let cross_w = power_for(&road, Some(&wind(8.0, 270.0)), v);
    assert!(head > calm + 80.0, "head {head} calm {calm}");
    assert!(tail < calm - 50.0, "tail {tail} calm {calm}");
    // A pure crosswind changes nothing along the road.
    assert!((cross_e - calm).abs() < 1e-6, "{cross_e} {calm}");
    assert!((cross_w - calm).abs() < 1e-6, "{cross_w} {calm}");
    // At the same power, a headwind is slower and a tailwind faster.
    let speed = |w: &WeatherPhysics| ride_for(&road, Some(w), 250.0, 120.0).speed;
    let (s_calm, s_head, s_tail, s_cross) = (
        speed(&wind(0.0, 0.0)),
        speed(&wind(8.0, 0.0)),
        speed(&wind(8.0, 180.0)),
        speed(&wind(8.0, 90.0)),
    );
    assert!(
        s_head < s_calm - 1.5 && s_tail > s_calm + 1.5,
        "{s_head} {s_calm} {s_tail}"
    );
    assert!((s_cross - s_calm).abs() < 1e-6);
    // A rider turned across the road gets part of the crosswind as head-
    // or tailwind: the travel's direction counts, not the road's.
    let turned = RiderState {
        heading: 0.3, // to the right (east), into the wind from the east
        ..Default::default()
    };
    let h = weather::at_rider(&wind(8.0, 90.0), &road, &turned);
    assert!(h.wind.head_mps > 1.0, "{h:?}");
}

#[test]
fn air_density_from_temperature_and_altitude_changes_the_speed() {
    let low = north(0.0);
    let high = north(2000.0);
    let air = |t: f32| {
        WeatherPhysics::new(
            &WeatherState {
                temperature_c: t,
                ..standard()
            },
            0.0,
            &Ground::default(),
            0.0,
        )
    };
    let at = |road: &Route, w: &WeatherPhysics| ride_for(road, Some(w), 250.0, 180.0).speed;
    // Thinner air up high: faster at the same power.
    assert!(at(&high, &air(15.0)) > at(&low, &air(15.0)) + 0.3);
    // Hot air is thinner than cold air.
    assert!(at(&low, &air(35.0)) > at(&low, &air(-5.0)) + 0.2);
    let state = RiderState::default();
    let d = |road: &Route| weather::at_rider(&air(15.0), road, &state).density;
    assert!((d(&low) - 1.225).abs() < 0.001);
    assert!((0.95..1.02).contains(&d(&high)), "{}", d(&high));
}

#[test]
fn a_wet_road_rolls_harder_and_grips_less() {
    let road = north(0.0);
    let dry = wind(0.0, 0.0);
    let wet = WeatherPhysics::new(&standard(), 0.0, &Ground::with_factors(1.0, 1.0, 0.0), 0.0);
    let v_dry = ride_for(&road, Some(&dry), 200.0, 120.0).speed;
    let v_wet = ride_for(&road, Some(&wet), 200.0, 120.0).speed;
    assert!(v_wet < v_dry && v_wet > v_dry - 0.5, "{v_wet} {v_dry}");
    // Leaning hard: on a wet road the curve is wider (less lean).
    let lean = |w: &WeatherPhysics| {
        let profile = RiderProfile::default();
        let mut s = RiderState {
            distance: 100.0,
            speed: 8.0,
            ..Default::default()
        };
        let controls = Controls {
            steering: Steering::Lean(physics::MAX_STEER_LEAN),
            wild: true,
            ..Default::default()
        };
        let mut scraping = false;
        ride::tick_in(
            &mut s,
            &profile,
            0.0,
            controls,
            &mut scraping,
            &road,
            1.0,
            Some(w),
            SIM_DT,
        );
        s.turn_curvature
    };
    assert!(
        lean(&wet) < lean(&dry) * 0.9,
        "{} {}",
        lean(&wet),
        lean(&dry)
    );
}

fn pedal(s: &mut Session, id: RiderId, watts: f64) {
    s.apply_input(
        id,
        1,
        RiderInput {
            power_w: watts,
            cadence_rpm: None,
            heart_rate_bpm: None,
            brake: false,
        },
    );
}

/// Two riders 3 m apart on the road north, the follower `lateral` metres
/// to the right; how far the follower got after 20 s at the leader's
/// power, in `w` (or without weather physics).
fn follower_distance(w: Option<WeatherPhysics>, lateral: f64) -> f64 {
    let mut s = Session::new(north(0.0));
    let leader = s.add_rider("leader", RiderProfile::default());
    let follower = s.add_rider("follower", RiderProfile::default());
    s.teleport(leader, 503.0);
    s.teleport(follower, 500.0);
    s.set_lateral_target(leader, 1, 0.0);
    s.set_lateral_target(follower, 1, lateral);
    pedal(&mut s, leader, 250.0);
    pedal(&mut s, follower, 250.0);
    s.set_weather(w);
    s.set_weather_physics(w.is_some());
    // The riders settle into their lines first.
    for _ in 0..(20.0 / SIM_DT) as usize {
        s.step();
    }
    s.rider_state(follower).unwrap().distance
}

#[test]
fn drafting_shelter_moves_downwind_in_a_crosswind() {
    // Still air: straight behind beats one metre to the side.
    assert!(follower_distance(None, 0.0) > follower_distance(None, 1.0) + 0.2);
    // A crosswind from the west (left) blows the wake to the right: the
    // follower on the right gets the shelter now.
    let west = wind(8.0, 270.0);
    let behind = follower_distance(Some(west), 0.0);
    let lee = follower_distance(Some(west), 1.0);
    assert!(lee > behind + 0.2, "lee {lee} behind {behind}");
    // From the east, the left side is the lee.
    let east = wind(8.0, 90.0);
    assert!(follower_distance(Some(east), -1.0) > follower_distance(Some(east), 1.0) + 0.2);
}

/// A session of riders and bots on the test loop for `seconds`, its
/// states every second.
fn run(
    weather: Option<WeatherPhysics>,
    switch: bool,
    event: bool,
) -> Vec<Vec<(RiderId, RiderState)>> {
    let mut s = Session::new(Route::test_loop());
    let me = s.add_rider("me", RiderProfile::default());
    for i in 0..3 {
        s.add_bot(format!("bot {i}"), 180.0 + f64::from(i) * 30.0);
    }
    pedal(&mut s, me, 230.0);
    s.set_weather_physics(switch);
    if event {
        s.start_event(EventSpec::GroupRide { target_w: None })
            .unwrap();
    }
    let mut out = Vec::new();
    for tick in 0..(60.0 / SIM_DT) as usize {
        if weather.is_some() {
            s.set_weather(weather);
        }
        s.step();
        if tick % 50 == 0 {
            out.push(
                s.snapshot()
                    .iter()
                    .map(|r| (r.id, s.rider_state(r.id).unwrap()))
                    .collect(),
            );
        }
    }
    out
}

#[test]
fn switch_off_is_the_physics_of_before_bit_for_bit() {
    let storm = ocw_weather::presets::preset("storm").unwrap();
    let w = WeatherPhysics::new(
        &storm,
        storm.gust_mps,
        &Ground::with_factors(1.0, 1.0, 0.0),
        0.0,
    );
    let before = run(None, false, false);
    // The weather is known but the switch is off: identical.
    assert_eq!(run(Some(w), false, false), before);
    // The switch on but no weather given: identical.
    assert_eq!(run(None, true, false), before);
    // On: the storm changes the ride.
    assert_ne!(run(Some(w), true, false), before);
    // Deterministic.
    assert_eq!(run(Some(w), true, false), run(Some(w), true, false));
}

#[test]
fn events_follow_the_organiser() {
    let mut s = Session::new(Route::test_loop());
    s.add_rider("me", RiderProfile::default());
    s.set_weather(Some(wind(10.0, 0.0)));
    s.set_weather_physics(true);
    assert!(s.weather_physics_on() && s.active_weather().is_some());
    // Q4: off in an event unless the organiser enables it.
    s.start_event(EventSpec::Race {
        laps: None,
        length_m: Some(500.0),
    })
    .unwrap();
    assert!(!s.weather_physics_on() && s.active_weather().is_none());
    assert!(s.set_event_weather_physics(true));
    assert!(s.weather_physics_on());
    s.set_weather_physics(false);
    assert!(s.weather_physics_on(), "the organiser's rule counts");
    s.cancel_event();
    assert!(!s.weather_physics_on() && !s.set_event_weather_physics(true));
    // An event's rule off with the session's on.
    s.set_weather_physics(true);
    assert_eq!(
        run(Some(wind(10.0, 0.0)), true, true),
        run(None, false, true),
        "an event without the organiser's rule rides without weather"
    );
}

#[test]
fn the_prediction_rides_the_servers_weather() {
    let route = north(300.0);
    let storm = ocw_weather::presets::preset("storm").unwrap();
    let w = WeatherPhysics::new(
        &storm,
        storm.wind_mps,
        &Ground::with_factors(1.0, 0.0, 0.0),
        0.0,
    );
    let mut s = Session::new(route.clone());
    let me = s.add_rider("me", RiderProfile::default());
    pedal(&mut s, me, 250.0);
    s.set_weather(Some(w));
    s.set_weather_physics(true);
    let start = s.rider_state(me).unwrap();
    s.set_lateral_target(me, 1, start.lateral_offset);
    let profile = s.rider_profile(me).unwrap();
    let mut with = Predictor::new(profile, start);
    with.set_lateral_target(start.lateral_offset);
    with.set_weather(Some(w));
    let mut without = Predictor::new(profile, start);
    without.set_lateral_target(start.lateral_offset);
    for _ in 0..(30.0 / SIM_DT) as usize {
        s.step();
        with.step(&route, 250.0);
        without.step(&route, 250.0);
    }
    let server = s.rider_state(me).unwrap();
    assert!(
        (with.state().distance - server.distance).abs() < 0.05,
        "{} vs {}",
        with.state().distance,
        server.distance
    );
    assert!((without.state().distance - server.distance).abs() > 1.0);
}
