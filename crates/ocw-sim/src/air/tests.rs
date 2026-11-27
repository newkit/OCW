//! The tests of 18E T8: the same seed and clock give the same flights on
//! two clients; contrails only in cold, humid upper air; no aircraft
//! inside the terrain; approaches and departures on the runway's line;
//! the density by the hour; the delay of sound.

use std::collections::BTreeMap;

use ocw_weather::state::{CloudType, Precip, WeatherState};
use ocw_world::aero::{Aerodrome, AerodromeKind, Helipad, Runway};

use super::*;

/// 2026-07-15 00:00 UTC.
const DAY0: f64 = 1_784_073_600.0;

/// A plain at 400 m with a 2,400 m mountain 9 km east-north-east of the
/// airport and a ridge to the south.
fn ground(x: f64, z: f64) -> f64 {
    let m =
        2000.0 * (-((x - 9000.0).powi(2) + (z + 4000.0).powi(2)) / (2.0 * 2500.0f64.powi(2))).exp();
    let ridge = 900.0 * (-((z - 14_000.0).powi(2)) / (2.0 * 1500.0f64.powi(2))).exp();
    400.0 + m + ridge
}

const HALF: f64 = 30_000.0;

fn inside(x: f64, z: f64) -> bool {
    x.abs() < HALF && z.abs() < HALF
}

fn aero() -> Aero {
    let ad = |name: &str, kind, pos: [f32; 2], ele| Aerodrome {
        osm_id: 1,
        name: name.into(),
        icao: String::new(),
        iata: String::new(),
        kind,
        lat_lon: (0.0, 0.0),
        pos,
        ele_m: ele,
        outside: false,
    };
    let mut a = Aero {
        aerodromes: vec![
            ad(
                "Plain International",
                AerodromeKind::International,
                [0.0, 0.0],
                Some(400.0),
            ),
            ad(
                "Meadow Field",
                AerodromeKind::Airfield,
                [-12_000.0, 5000.0],
                None,
            ),
            ad(
                "Soaring Club",
                AerodromeKind::Gliding,
                [-6000.0, -12_000.0],
                None,
            ),
        ],
        runways: Vec::new(),
        helipads: vec![
            Helipad {
                osm_id: 2,
                name: "Valley Hospital".into(),
                pos: [4000.0, -2500.0],
                ele_m: None,
                hospital: true,
                heliport: false,
            },
            Helipad {
                osm_id: 3,
                name: "Heliport".into(),
                pos: [-5000.0, 9000.0],
                ele_m: None,
                hospital: false,
                heliport: true,
            },
        ],
    };
    let h = 118.0f32.to_radians();
    let (dx, dz) = (h.sin() * 1700.0, -h.cos() * 1700.0);
    a.runways.push(Runway {
        aerodrome: Some(0),
        reference: "12/30".into(),
        ends: [[-dx, -dz], [dx, dz]],
        heading_deg: 118.0,
        length_m: 3400.0,
        width_m: 45.0,
        surface: "asphalt".into(),
    });
    a
}

fn space() -> AirSpace {
    make_space()
}

fn make_space() -> AirSpace {
    let samples = (-300..=300).flat_map(|k| {
        (-300..=300).map(move |i| {
            let (x, z) = (f64::from(i) * 100.0, f64::from(k) * 100.0);
            (x, z, ground(x, z) as f32)
        })
    });
    let relief = Relief::from_samples([-HALF, -HALF], [HALF, HALF], samples);
    // Near Puerto de la Cruz: Tenerife's airways.
    AirSpace::new((28.41, -16.55), Some(&aero()), relief, |x, z| {
        inside(x, z).then(|| ground(x, z))
    })
}

/// A fine summer day: fair-weather heaps, light wind from the north-east.
fn summer() -> WeatherState {
    WeatherState {
        cloud_cover: 0.3,
        cloud_type: CloudType::Cumulus,
        cloud_base_m: 1600.0,
        wind_mps: 2.5,
        wind_from_deg: 40.0,
        temperature_c: 24.0,
        humidity: 0.5,
        ..Default::default()
    }
}

fn inputs(w: &WeatherState, seed: u64, now_s: f64) -> AirInputs<'_> {
    AirInputs {
        seed,
        now_s,
        utc_offset_s: 3600,
        weather: w,
        viewer: [0.0, 500.0, 0.0],
        radius_m: 100_000.0,
        max_aircraft: 30,
        contrails: true,
        ground_m: 400.0,
    }
}

/// Every flight of a day, by id.
fn day_flights(
    space: &AirSpace,
    w: &WeatherState,
    seed: u64,
    day: f64,
) -> BTreeMap<u64, Arc<Flight>> {
    let mut sky = AirSky::default();
    let mut all = BTreeMap::new();
    let mut t = day;
    while t < day + 86_400.0 {
        for f in sky.flights(space, &inputs(w, seed, t)) {
            all.entry(f.id).or_insert(f);
        }
        t += 240.0;
    }
    all
}

#[test]
fn the_same_seed_and_clock_give_the_same_flights_on_two_clients() {
    let space = space();
    let w = summer();
    let (mut a, mut b) = (AirSky::default(), AirSky::default());
    // The first client rides all morning; the second joins at noon.
    let mut t = DAY0 + 6.0 * 3600.0;
    while t < DAY0 + 12.0 * 3600.0 {
        a.now(&space, &inputs(&w, 42, t));
        t += 60.0;
    }
    for k in 0..30 {
        let t = DAY0 + 12.0 * 3600.0 + f64::from(k) * 37.0;
        let (x, y) = (
            a.now(&space, &inputs(&w, 42, t)),
            b.now(&space, &inputs(&w, 42, t)),
        );
        assert!(!x.is_empty(), "aircraft at noon");
        assert_eq!(x, y, "the same sky at {t}");
    }
    // The same space built twice (another client's package load).
    let mut c = AirSky::default();
    let t = DAY0 + 12.5 * 3600.0;
    assert_eq!(
        c.now(&make_space(), &inputs(&w, 42, t)),
        b.now(&space, &inputs(&w, 42, t))
    );
    // Another session's seed: another sky.
    let mut d = AirSky::default();
    assert_ne!(
        d.now(&space, &inputs(&w, 43, t)),
        b.now(&space, &inputs(&w, 42, t))
    );
}

#[test]
fn every_kind_flies_on_its_day() {
    let space = space();
    let w = WeatherState {
        wind_mps: 1.5,
        ..summer()
    };
    let all = day_flights(&space, &w, 7, DAY0);
    let count = |m: Mission| all.values().filter(|f| f.mission == m).count();
    for m in [
        Mission::Cruise,
        Mission::Arrival,
        Mission::Departure,
        Mission::Circuit,
        Mission::CrossCountry,
        Mission::Rescue,
        Mission::Tour,
        Mission::Soaring,
        Mission::Drift,
    ] {
        assert!(count(m) > 0, "{m:?}: none in a day ({} flights)", all.len());
    }
    // A rainy, windy day: no small planes, gliders or balloons.
    let rain = WeatherState {
        precip: Precip::Rain,
        precip_mm_h: 4.0,
        wind_mps: 9.0,
        cloud_cover: 0.9,
        cloud_type: CloudType::Stratus,
        cloud_base_m: 350.0,
        ..summer()
    };
    let wet = day_flights(&space, &rain, 7, DAY0);
    assert!(
        wet.values()
            .all(|f| matches!(f.kind, AirKind::Airliner | AirKind::Helicopter))
    );
    assert!(wet.values().any(|f| f.kind == AirKind::Airliner));
}

#[test]
fn contrails_appear_only_in_cold_humid_upper_air() {
    let space = space();
    let cold = WeatherState {
        temperature_c: 3.0,
        humidity: 0.85,
        cloud_cover: 0.35,
        cloud_type: CloudType::Cirrus,
        ..Default::default()
    };
    let hot = WeatherState {
        temperature_c: 34.0,
        humidity: 0.2,
        cloud_cover: 0.0,
        ..Default::default()
    };
    let mut trails = [0usize; 2];
    for (i, w) in [&cold, &hot].into_iter().enumerate() {
        let mut sky = AirSky::default();
        for k in 0..40 {
            let t = DAY0 + 9.0 * 3600.0 + f64::from(k) * 180.0;
            for a in sky.now(&space, &inputs(w, 5, t)) {
                if let Some(tr) = &a.trail {
                    assert_eq!(a.kind, AirKind::Airliner);
                    assert!(a.pos[1] > 6000.0, "a trail low down: {}", a.pos[1]);
                    assert!(tr.points.len() >= 2);
                    trails[i] += 1;
                }
            }
        }
    }
    assert!(
        trails[0] > 10,
        "cold and humid: long trails ({})",
        trails[0]
    );
    assert_eq!(trails[1], 0, "hot and dry: none");
    // Persistent trails drift with the wind.
    let mut sky = AirSky::default();
    let windy = WeatherState {
        wind_mps: 8.0,
        wind_from_deg: 270.0,
        ..cold
    };
    let t = DAY0 + 10.0 * 3600.0;
    let a = sky
        .now(&space, &inputs(&windy, 5, t))
        .into_iter()
        .find(|a| {
            a.trail
                .as_ref()
                .is_some_and(|t| t.kind == Contrail::Persistent && t.points.len() > 8)
        })
        .expect("a long trail");
    let tr = a.trail.unwrap();
    let old = tr.points.last().unwrap();
    let f = sky
        .flights(&space, &inputs(&windy, 5, t))
        .into_iter()
        .find(|f| f.id == a.id)
        .unwrap();
    let was = f.state(t - old.age_s).pos;
    assert!(old.pos[0] - was[0] > 10.0 * old.age_s, "drifted east");
}

#[test]
fn no_aircraft_inside_the_terrain() {
    let space = space();
    for (seed, w) in [
        (11, summer()),
        (
            12,
            WeatherState {
                wind_mps: 1.0,
                ..summer()
            },
        ),
    ] {
        let all = day_flights(&space, &w, seed, DAY0);
        assert!(all.len() > 50, "{}", all.len());
        let mut checked = 0;
        for f in all.values() {
            let mut t = f.start_s;
            while t <= f.end_s() {
                let s = f.state(t);
                let [x, y, z] = s.pos;
                if !s.ground && inside(x, z) {
                    assert!(
                        y >= ground(x, z) - 0.5,
                        "{:?} {:?} {:.0} s in: {y:.0} m over ground {:.0} m at {x:.0}, {z:.0}",
                        f.kind,
                        f.mission,
                        t - f.start_s,
                        ground(x, z)
                    );
                    checked += 1;
                }
                if s.ground && inside(x, z) {
                    assert!(
                        (y - ground(x, z)).abs() < 25.0,
                        "{:?} on the ground at {y}",
                        f.mission
                    );
                }
                t += 2.0;
            }
        }
        assert!(checked > 10_000);
    }
}

#[test]
fn approaches_and_departures_follow_the_runway() {
    let space = space();
    for wind_from in [118.0f32, 298.0] {
        let w = WeatherState {
            wind_mps: 6.0,
            wind_from_deg: wind_from,
            ..summer()
        };
        let all = day_flights(&space, &w, 3, DAY0);
        let mut n = 0;
        for f in all
            .values()
            .filter(|f| matches!(f.mission, Mission::Arrival | Mission::Departure))
        {
            let rw = f.runway.expect("a runway");
            // Into the wind.
            assert!(
                (rw.heading_deg - f64::from(wind_from)).abs() < 1.0,
                "{rw:?}"
            );
            let u = rw.dir();
            let mut t = f.start_s;
            while t <= f.end_s() {
                let s = f.state(t);
                let rel = [s.pos[0] - rw.threshold[0], s.pos[2] - rw.threshold[1]];
                let along = rel[0] * u[0] + rel[1] * u[1];
                let across = rel[0] * u[1] - rel[1] * u[0];
                let on_final = f.mission == Mission::Arrival
                    && (-17_000.0..0.0).contains(&along)
                    && across.abs() < 2000.0
                    && s.pos[1] < rw.elevation_m + 1000.0;
                let on_climb = f.mission == Mission::Departure
                    && (100.0..9000.0).contains(&along)
                    && s.pos[1] < rw.elevation_m + 700.0
                    && t - f.start_s < 400.0;
                if on_final || on_climb || s.ground {
                    assert!(
                        across.abs() < 5.0,
                        "{:?} {across:.1} m off the line",
                        f.mission
                    );
                    let speed = s.vel[0].hypot(s.vel[2]);
                    if speed > 5.0 {
                        let h = heading_of([s.vel[0], s.vel[2]]);
                        let diff = (h - rw.heading_deg + 540.0).rem_euclid(360.0) - 180.0;
                        assert!(
                            diff.abs() < 0.5,
                            "heading {h:.1} on runway {:.1}",
                            rw.heading_deg
                        );
                    }
                }
                if on_final {
                    // The 3° glide slope.
                    let slope = (s.pos[1] - rw.elevation_m - 15.0) / -along;
                    assert!((slope - 3f64.to_radians().tan()).abs() < 0.003, "{slope}");
                }
                t += 3.0;
            }
            // Touchdown on the runway, and the end on it.
            let (_, end) = f.path.ends().unwrap();
            if f.mission == Mission::Arrival {
                let rel = [end[0] - rw.threshold[0], end[2] - rw.threshold[1]];
                let along = rel[0] * u[0] + rel[1] * u[1];
                assert!((0.0..rw.length_m).contains(&along));
            }
            n += 1;
        }
        assert!(n > 20, "{n}");
    }
}

#[test]
fn density_follows_the_hour() {
    let space = space();
    let w = summer();
    let mut by_hour = [[0usize; 24]; 2];
    let mut t = DAY0;
    let mut sky = AirSky::default();
    let mut seen = std::collections::BTreeSet::new();
    while t < DAY0 + 6.0 * 86_400.0 {
        for f in sky.flights(&space, &inputs(&w, 9, t)) {
            if seen.insert(f.id) {
                let (h, _) = local_hour_month(f.start_s, 3600);
                let i = usize::from(f.kind != AirKind::Airliner);
                by_hour[i][h as usize % 24] += 1;
            }
        }
        t += 300.0;
    }
    let sum = |i: usize, r: std::ops::Range<usize>| by_hour[i][r].iter().sum::<usize>() as f64;
    let night = sum(0, 1..5);
    let day = sum(0, 9..13);
    assert!(day > 50.0, "{by_hour:?}");
    assert!(night < 0.2 * day, "airliners: night {night} vs day {day}");
    // Small planes, gliders: not before 8 or after 20; helicopters some.
    let small_night = sum(1, 0..5) + sum(1, 22..24);
    let small_day = sum(1, 10..17);
    assert!(
        small_day > 20.0 && small_night < 0.25 * small_day,
        "{by_hour:?}"
    );
}

#[test]
fn the_sound_arrives_late_by_the_distance() {
    let space = space();
    let w = summer();
    let t = DAY0 + 11.0 * 3600.0;
    let mut sky = AirSky::default();
    let listener = [0.0, 450.0, 0.0];
    let heard = sky.heard(&space, &inputs(&w, 42, t), listener);
    assert!(!heard.is_empty());
    let flights = sky.flights(&space, &inputs(&w, 42, t));
    for h in heard {
        assert!(h.distance_m <= HEARD_M);
        assert!(
            (h.delay_s - h.distance_m / SPEED_OF_SOUND_MPS).abs() < 1e-3,
            "{h:?}"
        );
        // Behind where it is seen now (unless standing).
        let f = flights.iter().find(|f| f.id == h.id).unwrap();
        let now = f.state(t);
        let speed = now.vel[0].hypot(now.vel[2]);
        let moved = ((now.pos[0] - h.pos[0]).powi(2) + (now.pos[2] - h.pos[2]).powi(2)).sqrt();
        if speed > 30.0 && !h.on_ground {
            assert!(
                moved > 0.5 * speed * h.delay_s,
                "{h:?} now {now:?} {:?} {}",
                f.mission,
                t - f.start_s
            );
        }
    }
}

#[test]
fn the_budget_keeps_the_nearest() {
    let space = space();
    let w = summer();
    let t = DAY0 + 11.0 * 3600.0;
    let mut sky = AirSky::default();
    let all = sky.now(
        &space,
        &AirInputs {
            max_aircraft: 1000,
            ..inputs(&w, 1, t)
        },
    );
    let few = sky.now(
        &space,
        &AirInputs {
            max_aircraft: 6,
            ..inputs(&w, 1, t)
        },
    );
    assert!(all.len() > 6, "{}", all.len());
    assert_eq!(few.len(), 6);
    let d = |a: &AircraftNow| a.pos[0].hypot(a.pos[2]);
    let cut = few.iter().map(d).fold(0.0, f64::max);
    assert!(all.iter().filter(|a| d(a) < cut).count() <= 6);
    // No contrails asked: none drawn.
    let none = sky.now(
        &space,
        &AirInputs {
            contrails: false,
            ..inputs(&w, 1, t)
        },
    );
    assert!(none.iter().all(|a| a.trail.is_none()));
}

#[test]
fn local_hours_and_months() {
    assert_eq!(local_hour_month(DAY0, 0), (0.0, 7));
    assert_eq!(local_hour_month(DAY0 + 3600.0 * 13.5, 3600).0, 14.5);
    // 2026-12-31 23:30 UTC: still December at UTC, January an hour east.
    let (h, m) = local_hour_month(1_798_759_800.0, 0);
    assert_eq!((h, m), (23.5, 12));
    assert_eq!(local_hour_month(1_798_759_800.0, 3600).1, 1);
}

#[test]
fn a_world_without_aerodromes_has_the_nearby_airports() {
    // Liechtenstein's packages have no AERO of their own here: Zurich,
    // Altenrhein, Friedrichshafen from the table.
    let s = AirSpace::new((47.14, 9.52), None, Relief::default(), |_, _| None);
    let names: Vec<&str> = s.fields().map(|f| f.0).collect();
    assert!(
        names.contains(&"ZRH") && names.contains(&"FDH"),
        "{names:?}"
    );
    assert!(s.generator_count() > 6);
}
