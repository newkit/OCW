//! Wet leaves in the ride (spec 4.6.2 Q6; 11.4): the density's
//! determinism and agreement with the drawn amount, the speed cost, the
//! wobble's bounds, the slide only with damage on, never in group rides
//! and never on a dry road, and the prediction riding the same.

use std::collections::BTreeMap;
use std::sync::Arc;

use ocw_core::{RiderProfile, SIM_DT};
use ocw_sky::season::{SeasonModel, bucket_day, season_bucket};
use ocw_world::forest::{ForestCover, ForestTile};
use ocw_world::landuse::{LandClass, LandUse, LandUseTile};
use ocw_world::leaves::LeafMap;
use ocw_world::{Route, TileKey, WorldPackage};

use super::*;
use crate::collide::ContactKind;
use crate::session::{JOIN_GHOST_S, RiderInput, Session};

/// The Black Forest (the package's origin: the region and the season).
const PLACE: (f64, f64) = (48.0, 8.2);

/// A package of 3 × 3 tiles of broadleaf wood (measured 100 %) – or of
/// `class` everywhere.
fn package(class: LandClass) -> WorldPackage {
    package_at(class, PLACE)
}

/// [`package`] at `origin` (its region and season).
fn package_at(class: LandClass, origin: (f64, f64)) -> WorldPackage {
    let cells = 50u32;
    let mut lu = LandUse {
        cells,
        tiles: BTreeMap::new(),
    };
    let mut forest = ForestCover {
        cells,
        ..Default::default()
    };
    for x in -1..=1 {
        for z in -1..=1 {
            let key = TileKey { x, z };
            lu.tiles.insert(
                key,
                LandUseTile {
                    classes: vec![class; (cells * cells) as usize],
                    water: Vec::new(),
                },
            );
            forest.tiles.insert(
                key,
                ForestTile {
                    density: vec![90; (cells * cells) as usize],
                    broadleaf: vec![100; (cells * cells) as usize],
                },
            );
        }
    }
    let mut pkg = WorldPackage {
        name: "Leafy".into(),
        origin,
        tile_size_m: 1000.0,
        samples: 2,
        tiles: BTreeMap::new(),
        graph: None,
        landuse: Some(lu),
        buildings: None,
        bridges: None,
        routes: Vec::new(),
        region: None,
        time_zone: None,
        look: Default::default(),
        obstacles: None,
        places: None,
        junctions: None,
    };
    pkg.look.forest = Some(forest);
    pkg
}

/// The season on a day of the year at the place.
fn day(doy: u32) -> Arc<ocw_sky::season::SeasonDay> {
    Arc::new(SeasonModel::for_place(PLACE.0, PLACE.1).day(bucket_day(season_bucket(doy))))
}

/// 10 November (leaves down) and 1 July (none).
const NOVEMBER: u32 = 314;
const JULY: u32 = 182;

fn ground(class: LandClass, doy: u32) -> LeafGround {
    LeafGround {
        map: Arc::new(LeafMap::from_package(&package(class))),
        day: day(doy),
    }
}

/// Calm air on a road of `wetness` (grip as the weather gives it).
fn weather(wetness: f64) -> WeatherPhysics {
    WeatherPhysics {
        wind: [0.0, 0.0],
        temperature_c: 8.0,
        pressure_hpa: 1013.25,
        pressure_y_m: 0.0,
        humidity: 0.9,
        rolling_factor: 1.0 + 0.06 * wetness,
        grip: 1.0 - 0.25 * wetness,
        wetness,
        snow_cover: 0.0,
    }
}

/// A straight road east through the wood, 6 m wide.
fn straight() -> Route {
    let pts = (0..=150)
        .map(|i| [-700.0 + f64::from(i) * 10.0, 400.0, 0.0])
        .collect();
    Route::from_points("wood", "Wood road", pts, false, 6.0).unwrap()
}

/// A loop of radius `r` m through the wood (a corner all the way).
fn circle(r: f64) -> Route {
    let n = 180;
    let pts = (0..n)
        .map(|i| {
            let a = f64::from(i) / f64::from(n) * std::f64::consts::TAU;
            [r * a.cos(), 400.0, r * a.sin()]
        })
        .collect();
    Route::from_points("bend", "Bend", pts, true, 6.0).unwrap()
}

fn session(route: Route, wet: f64, leaves: Option<LeafGround>) -> Session {
    let mut s = Session::new(route);
    s.set_collisions(true);
    s.set_weather(Some(weather(wet)));
    s.set_weather_physics(true);
    if let Some(g) = leaves {
        s.set_leaf_map(Some(g.map));
        s.set_leaf_season(Some(g.day));
    }
    s
}

fn ride(s: &mut Session, id: ocw_core::RiderId, watts: f64, seconds: f64) {
    s.apply_input(
        id,
        1,
        RiderInput {
            power_w: watts,
            ..Default::default()
        },
    );
    for _ in 0..(seconds / SIM_DT) as usize {
        s.step();
    }
}

#[test]
fn the_density_is_deterministic_and_the_drawn_amount() {
    let g = ground(LandClass::Forest, NOVEMBER);
    let route = straight();
    let state = RiderState {
        distance: 300.0,
        lateral_offset: 1.0,
        speed: 9.0,
        ..Default::default()
    };
    let a = g.per_metre(&route, &state);
    // 0.8 a metre in a pure broadleaf wood with the leaves down.
    assert!(g.day.leaf_fall(400.0) > 0.9, "{}", g.day.leaf_fall(400.0));
    assert!(a > 0.7 && a <= 0.8 + 1e-6, "{a}");
    // The same again, from a map built again.
    let again = ground(LandClass::Forest, NOVEMBER);
    assert_eq!(again.per_metre(&route, &state), a);
    // The renderer's amount at the same verge point (3 m beyond the edge
    // on the rider's side) is the map's.
    let p = route.sample(300.0);
    let (x, z) = (
        p.position[0] + p.right()[0] * 6.0,
        p.position[2] + p.right()[1] * 6.0,
    );
    let drawn = ocw_world::leaves::per_metre(
        ocw_world::leaves::code_at(&package(LandClass::Forest), x, z),
        ocw_world::flora::FloraRegion::at(PLACE.0, PLACE.1),
        400.0,
        g.day.leaf_fall(400.0),
        g.day.snow(400.0),
    );
    assert_eq!(f64::from(drawn), a);
    // Summer: none; meadows: a few; off the road: none.
    assert_eq!(
        ground(LandClass::Forest, JULY).per_metre(&route, &state),
        0.0
    );
    let meadow = ground(LandClass::Meadow, NOVEMBER).per_metre(&route, &state);
    assert!(meadow > 0.0 && meadow < 0.07, "{meadow}");
    let off = RiderState {
        lateral_offset: 4.0,
        ..state
    };
    assert_eq!(g.per_metre(&route, &off), 0.0);
}

#[test]
fn slickness_needs_wet_leaves() {
    let g = ground(LandClass::Forest, NOVEMBER);
    let route = straight();
    let st = RiderState {
        distance: 300.0,
        speed: 9.0,
        ..Default::default()
    };
    let s = |w: f64| slick(Some(&g), Some(&weather(w)), &route, &st);
    assert_eq!(s(0.0), 0.0, "dry");
    assert_eq!(s(0.08), 0.0, "damp");
    assert!(s(0.3) > 0.3 && s(0.3) < s(1.0));
    assert!(s(1.0) > 0.85);
    // Lying snow hides them; no weather physics or no leaf map: nothing.
    let snowy = WeatherPhysics {
        snow_cover: 0.6,
        ..weather(1.0)
    };
    assert_eq!(slick(Some(&g), Some(&snowy), &route, &st), 0.0);
    assert_eq!(slick(Some(&g), None, &route, &st), 0.0);
    assert_eq!(slick(None, Some(&weather(1.0)), &route, &st), 0.0);
    assert_eq!(
        slick(
            Some(&ground(LandClass::Forest, JULY)),
            Some(&weather(1.0)),
            &route,
            &st
        ),
        0.0
    );
    // The rolling resistance adds up with the wet road's.
    let w = weather(1.0);
    assert!((on_leaves(&w, 1.0).rolling_factor - (w.rolling_factor + ROLLING)).abs() < 1e-12);
    assert_eq!(on_leaves(&w, 0.0), w);
}

#[test]
fn wet_leaves_cost_a_little_speed() {
    let speed = |wet: f64, leaves: bool| {
        let mut s = session(
            straight(),
            wet,
            leaves.then(|| ground(LandClass::Forest, NOVEMBER)),
        );
        let a = s.add_rider("Ada Rider", RiderProfile::default());
        ride(&mut s, a, 200.0, 60.0);
        s.rider_state(a).unwrap()
    };
    let wet_leaves = speed(1.0, true);
    let wet = speed(1.0, false);
    let loss_kmh = (wet.speed - wet_leaves.speed) * 3.6;
    println!(
        "200 W, soaked: {:.2} km/h without leaves, {:.2} km/h on a full cover",
        wet.speed * 3.6,
        wet_leaves.speed * 3.6
    );
    // A little: under 2 km/h at 200 W in a pure broadleaf wood, soaked.
    assert!(loss_kmh > 0.3 && loss_kmh < 2.0, "{loss_kmh} km/h");
    // A dry road with leaves rides exactly as without them.
    assert_eq!(speed(0.0, true), speed(0.0, false));
}

#[test]
fn the_wobble_stays_small_and_shows_on_the_lean() {
    // Bounds of the functions: the sway and the lean.
    for i in 0..2000 {
        let d = f64::from(i) * 0.137;
        assert!(wobble_offset(1.0, 15.0, d).abs() <= WOBBLE_M + 1e-12);
        assert!(wobble_lean(1.0, 15.0, d).abs() <= WOBBLE_LEAN_RAD + 1e-12);
    }
    assert_eq!(wobble_offset(1.0, 4.0, 10.3), 0.0, "slow: no wobble");
    assert_eq!(wobble_offset(0.0, 12.0, 10.3), 0.0, "no leaves");
    // In the session: an autodrive rider at speed sways, at most 3 cm off
    // its line and under 2.5 cm a tick.
    let mut s = session(straight(), 1.0, Some(ground(LandClass::Forest, NOVEMBER)));
    let a = s.add_rider("Ada Rider", RiderProfile::default());
    ride(&mut s, a, 400.0, 20.0);
    let mut last = s.rider_state(a).unwrap();
    let base = last.lateral_offset - wobble_offset(1.0, last.speed, last.distance);
    let (mut most, mut step, mut moved) = (0.0f64, 0.0f64, 0.0f64);
    for _ in 0..500 {
        s.step();
        let now = s.rider_state(a).unwrap();
        most = most.max((now.lateral_offset - base).abs());
        step = step.max((now.lateral_offset - last.lateral_offset).abs());
        moved += (now.lateral_offset - last.lateral_offset).abs();
        last = now;
    }
    assert!(last.speed > WOBBLE_FULL_MPS, "{}", last.speed);
    assert!(most <= WOBBLE_M + 0.005, "{most}");
    assert!(step < 0.025, "{step}");
    assert!(moved > 0.1, "it sways: {moved} m");
    let g = s.leaf_ground().unwrap();
    let sl = slick(Some(&g), Some(&weather(1.0)), s.route(), &last);
    assert!(wobble_lean(sl, last.speed, last.distance + 0.9).abs() > 0.0);
}

/// A client rider pushed round a 40 m bend at about 36 km/h for up to
/// `seconds`: the crash's kind, if any.
fn corner(wet: f64, damage: bool, group: bool, seconds: f64) -> (Session, Option<ContactKind>) {
    let mut s = session(circle(40.0), wet, Some(ground(LandClass::Forest, NOVEMBER)));
    s.set_damage(damage);
    let a = s.add_rider("Ada Rider", RiderProfile::default());
    if group {
        s.start_event(crate::EventSpec::GroupRide { target_w: None })
            .unwrap();
        s.set_event_collisions(Some(crate::collide::Mode::Full));
    }
    for _ in 0..=(JOIN_GHOST_S / SIM_DT) as usize {
        s.step();
    }
    s.apply_input(
        a,
        1,
        RiderInput {
            power_w: 600.0,
            ..Default::default()
        },
    );
    for _ in 0..(seconds / SIM_DT) as usize {
        s.step();
        if let Some(c) = s.crash(a) {
            return (s, Some(c.kind));
        }
    }
    (s, None)
}

#[test]
fn a_corner_on_wet_leaves_slides_only_with_damage_on() {
    let (s, kind) = corner(1.0, true, false, 120.0);
    assert_eq!(kind, Some(ContactKind::Leaves));
    // A crash like the others: counted, the damage taken.
    assert_eq!(s.collision_stats().crashes, 1);
    let id = s.snapshot()[0].id;
    let d = s.damage(id).unwrap();
    assert_eq!(d.crashes, 1);
    assert!(d.rider > 0.0, "{d:?}");
    // Deterministic: the same slide at the same tick.
    let (again, _) = corner(1.0, true, false, 120.0);
    assert_eq!(again.tick(), s.tick());
    // Damage off, a dry road or a group ride (even with the full model):
    // no slide.
    assert_eq!(corner(1.0, false, false, 120.0).1, None, "damage off");
    assert_eq!(corner(0.0, true, false, 120.0).1, None, "dry");
    let (g, kind) = corner(1.0, true, true, 120.0);
    assert!(g.group_ride_running() || g.event().is_some());
    assert_eq!(kind, None, "group ride");
}

#[test]
fn the_hazard_needs_speed_and_a_corner() {
    assert_eq!(slide_hazard(1.0, 5.0, 0.05), 0.0, "slow");
    assert_eq!(slide_hazard(1.0, 10.0, 0.0), 0.0, "straight");
    assert_eq!(slide_hazard(0.0, 10.0, 0.05), 0.0, "no leaves");
    // 36 km/h in a 40 m bend: a pull of 0.25 g.
    let h = slide_hazard(1.0, 10.0, 1.0 / 40.0);
    assert!(h > 0.05 && h < 0.1, "{h}");
    assert!(slide_hazard(0.3, 10.0, 1.0 / 40.0) < h);
    assert!(slide_hazard(1.0, 20.0, 0.1) <= SLIDE_MAX_PER_S);
    // The draw: about the hazard's share of ticks.
    let n = (0..200_000u64)
        .filter(|t| slides(ocw_core::RiderId(3), *t, 0.5, SIM_DT))
        .count();
    assert!((1800..2200).contains(&n), "{n}");
}

#[test]
fn the_prediction_rides_the_leaves_as_the_server() {
    let g = ground(LandClass::Forest, NOVEMBER);
    let route = straight();
    let mut s = session(route.clone(), 1.0, Some(g.clone()));
    let a = s.add_rider("Ada Rider", RiderProfile::default());
    ride(&mut s, a, 300.0, 5.0);
    let start = s.rider_state(a).unwrap();
    let mut p = crate::Predictor::new(RiderProfile::default(), start);
    p.set_weather(Some(weather(1.0)));
    p.set_leaves(Some(g));
    p.reconcile(&route, start, 0.0, 300.0);
    let mut worst = (0.0f64, 0.0f64);
    for _ in 0..500 {
        s.step();
        p.step(&route, 300.0);
        let (sv, pr) = (s.rider_state(a).unwrap(), p.state());
        worst.0 = worst.0.max((sv.distance - pr.distance).abs());
        worst.1 = worst.1.max((sv.lateral_offset - pr.lateral_offset).abs());
    }
    // In step with the server: the same cost, the same sway.
    assert!(worst.0 < 0.05, "distance {} m", worst.0);
    assert!(worst.1 < 0.01, "lateral {} m", worst.1);
    // Reconciling with the server's state moves the prediction smoothly:
    // no sideways step over 6 cm.
    let mut last = p.state();
    for _ in 0..50 {
        for _ in 0..5 {
            s.step();
        }
        p.reconcile(&route, s.rider_state(a).unwrap(), 0.06, 300.0);
        for _ in 0..5 {
            p.step(&route, 300.0);
            let now = p.state();
            assert!((now.lateral_offset - last.lateral_offset).abs() < 0.06);
            last = now;
        }
    }
}

#[test]
fn the_season_follows_the_world_clock() {
    let mut ls = LeafSeason::default();
    // 10 November 2026 noon UTC and 1 July 2026.
    let nov = ls.day_at(1_794_312_000_000, PLACE);
    assert!(nov.leaf_fall(400.0) > 0.9);
    let again = ls.day_at(1_794_312_000_000 + 3_600_000, PLACE);
    assert!(Arc::ptr_eq(&nov, &again), "cached within the bucket");
    let jul = ls.day_at(1_782_907_200_000, PLACE);
    assert!(jul.leaf_fall(400.0) < 0.05, "{}", jul.leaf_fall(400.0));
}

/// Spec §14 Phase 9 Q13: on Tenerife a wood measured all broadleaves is
/// laurel forest and keeps its leaves – the wet leaves on its roads are
/// near zero all year (the Black Forest's: up to 0.8 a metre), and they
/// never make the road slick to speak of.
#[test]
fn wet_leaves_are_near_zero_in_tenerifes_evergreen_woods() {
    let puerto = (28.41, -16.55);
    let map = Arc::new(LeafMap::from_package(&package_at(
        LandClass::Forest,
        puerto,
    )));
    let model = SeasonModel::for_place(puerto.0, puerto.1);
    let route = straight();
    let state = RiderState {
        distance: 300.0,
        lateral_offset: 1.0,
        speed: 9.0,
        ..Default::default()
    };
    let mut most = 0.0f64;
    for doy in (1..=365).step_by(4) {
        let g = LeafGround {
            map: map.clone(),
            day: Arc::new(model.day(bucket_day(season_bucket(doy)))),
        };
        let a = g.per_metre(&route, &state);
        most = most.max(a);
        let s = slick(Some(&g), Some(&weather(1.0)), &route, &state);
        assert!(s < 0.02, "day {doy}: {s}");
    }
    assert!(most < 0.01, "{most}");
    let bf = ground(LandClass::Forest, NOVEMBER).per_metre(&route, &state);
    assert!(bf > 50.0 * most.max(1e-3), "{bf} vs {most}");
}
