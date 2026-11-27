//! The tests of 18E T9: off is nothing, the rates by setting, the same
//! for every rider, never near a rider's road, the curiosity rule's
//! timing (events: `ocw_client::ufo`).

use super::curiosity::{self, Curiosity, Phase, Unicorn};
use super::*;

/// A flat world at 400 m: fields everywhere but woods in every fifth
/// kilometre square, a grid of roads every kilometre.
struct Grid;

const ROADS_M: f64 = 1000.0;

fn grid_road_m(x: f64, z: f64) -> f64 {
    let d = |v: f64| {
        let r = v.rem_euclid(ROADS_M);
        r.min(ROADS_M - r)
    };
    d(x).min(d(z))
}

impl UfoGround for Grid {
    fn height(&self, x: f64, z: f64) -> Option<f64> {
        (x.abs() < 30_000.0 && z.abs() < 30_000.0).then_some(400.0)
    }
    fn land(&self, x: f64, z: f64) -> Land {
        let (i, k) = ((x / 1000.0).floor() as i64, (z / 1000.0).floor() as i64);
        if (i + 2 * k).rem_euclid(5) == 0 {
            Land::Other
        } else {
            Land::Field
        }
    }
    fn road_m(&self, x: f64, z: f64, within: f64) -> f64 {
        grid_road_m(x, z).min(within)
    }
}

/// 2026-07-15 00:00 UTC; the place near Vaduz (UTC+2 in summer).
const DAY0: f64 = 1_784_073_600.0;
const ORIGIN: (f64, f64) = (47.14, 9.52);

fn inputs(level: Ufos, now_s: f64, w: &WeatherState, viewer: [f64; 3]) -> UfoInputs<'_> {
    UfoInputs {
        seed: 42,
        now_s,
        level,
        weather: w,
        origin: ORIGIN,
        viewer,
    }
}

/// Every sighting of `days` days in the 5 × 5 cells round the origin.
fn all(level: Ufos, w: &WeatherState, days: i64) -> Vec<Sighting> {
    let inp = inputs(level, DAY0, w, [0.0; 3]);
    let k0 = (DAY0 / SLOT_S) as i64;
    let mut out = Vec::new();
    for k in k0..k0 + days * 48 {
        for cx in -2..=2 {
            for cz in -2..=2 {
                out.extend(sighting(&Grid, &inp, (cx, cz), k));
            }
        }
    }
    out
}

#[test]
fn off_is_nothing() {
    let w = WeatherState::default();
    assert!(all(Ufos::Off, &w, 10).is_empty());
    let mut sky = UfoSky::default();
    let night = DAY0 + 23.0 * 3600.0;
    let s = sky.now(&Grid, &inputs(Ufos::Off, night, &w, [0.0; 3]));
    assert_eq!(s, UfoScene::default());
    let mut c = Curiosity::default();
    let u = Unicorn {
        pos: [0.0, 600.0, 0.0],
        vel: [10.0, 0.0, 0.0],
    };
    for i in 0..100_000 {
        assert!(
            c.step(42, Ufos::Off, night + f64::from(i) * 0.05, Some(&u))
                .is_none()
        );
    }
}

#[test]
fn the_rates_follow_the_setting_the_light_and_the_weather() {
    let clear = WeatherState::default();
    let rare = all(Ufos::Rare, &clear, 30);
    let often = all(Ufos::Often, &clear, 30);
    let light = |s: &Sighting| Light::of_sun(sun_deg(s.start_s, ORIGIN));
    // Rare: only at night, about 0.025 per cell and night slot.
    assert!(!rare.is_empty());
    assert!(
        rare.iter().all(|s| light(s) == Light::Night),
        "rare only at night"
    );
    // Often: about five times as many, at dusk and by day too.
    assert!(
        often.len() > 4 * rare.len(),
        "{} / {}",
        often.len(),
        rare.len()
    );
    assert!(often.iter().any(|s| light(s) == Light::Dusk));
    let day: Vec<_> = often.iter().filter(|s| light(s) == Light::Day).collect();
    assert!(!day.is_empty());
    assert!(day.iter().all(|s| s.kind == SightingKind::Day));
    // By day high in the sky (over 1,200 m), by night low.
    for s in &day {
        assert!(s.samples(1.0).iter().all(|p| p[1] - 400.0 > 1100.0));
    }
    // The scenes: crop circles and cows on often.
    for k in [
        SightingKind::CropCircle,
        SightingKind::Cow,
        SightingKind::Night,
    ] {
        assert!(often.iter().any(|s| s.kind == k), "{k:?}");
    }
    // Rare's sightings are among often's (one number decides both).
    for s in &rare {
        assert!(often.iter().any(|o| o.id == s.id), "{}", s.id);
    }
    // Night slots per cell: the expected count, within a wide margin.
    let night_slots = (0..30 * 48)
        .filter(|k| {
            let t = DAY0 + f64::from(*k) * SLOT_S + SLOT_S / 2.0;
            Light::of_sun(sun_deg(t, ORIGIN)) == Light::Night
        })
        .count() as f64
        * 25.0;
    let expect = night_slots * rates(Ufos::Rare).night;
    let got = rare.len() as f64;
    assert!(
        got > expect * 0.5 && got < expect * 1.6,
        "{got} vs {expect}"
    );
    // Overcast, rain or fog: none.
    for w in [
        WeatherState {
            cloud_cover: 0.9,
            ..clear
        },
        WeatherState {
            precip_mm_h: 2.0,
            ..clear
        },
        WeatherState {
            fog_visibility_m: 300.0,
            ..clear
        },
    ] {
        assert!(all(Ufos::Often, &w, 10).is_empty());
    }
}

#[test]
fn every_rider_sees_the_same_ufos() {
    let w = WeatherState::default();
    // Two riders 2 km apart, a third joining at half time.
    let (mut a, mut b, mut late) = (UfoSky::default(), UfoSky::default(), UfoSky::default());
    let (va, vb) = ([0.0, 420.0, 0.0], [2000.0, 420.0, -800.0]);
    let start = DAY0 + 20.0 * 3600.0;
    let mut shared = 0;
    for i in 0..(10 * 3600 / 4) {
        let t = start + f64::from(i) * 4.0;
        let sa = a.now(&Grid, &inputs(Ufos::Often, t, &w, va));
        let sb = b.now(&Grid, &inputs(Ufos::Often, t, &w, vb));
        for u in &sa.ufos {
            if let Some(v) = sb.ufos.iter().find(|v| v.id == u.id) {
                assert_eq!(u, v);
                shared += 1;
            }
        }
        for c in &sa.cows {
            if let Some(d) = sb.cows.iter().find(|d| d.id == c.id) {
                assert_eq!(c, d);
            }
        }
        if t >= start + 5.0 * 3600.0 {
            let sl = late.now(&Grid, &inputs(Ufos::Often, t, &w, va));
            assert_eq!(sl.ufos, sa.ufos);
            assert_eq!(sl.cows, sa.cows);
            // The circles made in the last 12 h are found by the late one.
            for c in &sl.circles {
                assert!(sa.circles.contains(c));
            }
        }
    }
    assert!(shared > 50, "{shared}");
    assert!(a.circles_seen() > 0);
    // Another seed: another sky.
    let other = UfoInputs {
        seed: 43,
        ..inputs(Ufos::Often, start, &w, va)
    };
    let mine: Vec<u64> = all(Ufos::Often, &w, 3).iter().map(|s| s.id).collect();
    let k0 = (DAY0 / SLOT_S) as i64;
    let theirs: Vec<u64> = (k0..k0 + 3 * 48)
        .flat_map(|k| sighting(&Grid, &other, (0, 0), k).map(|s| s.id))
        .collect();
    assert!(theirs.iter().all(|id| !mine.contains(id)));
}

#[test]
fn ufos_never_come_near_a_riders_road() {
    let w = WeatherState::default();
    let often = all(Ufos::Often, &w, 20);
    assert!(often.len() > 100);
    for s in &often {
        let dt = 0.25;
        let mut t = s.start_s;
        while t < s.end_s {
            let p = s.pos(t);
            let road = grid_road_m(p[0], p[2]);
            // A rider's head on the nearest road, 1.5 m up.
            let d = road.hypot(p[1] - 401.5);
            assert!(
                d >= 30.0,
                "{:?} {} at {t}: {d:.1} m from a road",
                s.kind,
                s.id
            );
            if s.kind == SightingKind::Night && p[1] - 400.0 < HOVER_MIN_M - 2.0 {
                panic!("{} hovers too low at {t}", s.id);
            }
            if let Some(c) = s.cow_now(t) {
                assert!(grid_road_m(c.pos[0], c.pos[2]) >= SCENE_ROAD_M);
                assert!(c.pos[1] >= 400.0 - 1e-6 && c.pos[1] < p[1]);
            }
            t += dt;
        }
        if let Some(c) = &s.circle {
            let road = grid_road_m(c.centre[0], c.centre[1]);
            assert!(road >= c.extent_m() + SCENE_ROAD_M && road < SCENE_NEAR_M);
            assert_eq!(Grid.land(c.centre[0], c.centre[1]), Land::Field);
        }
        if let Some(c) = &s.cow {
            // Set down unharmed a field away, on the ground.
            let moved = (c.b[0] - c.a[0]).hypot(c.b[2] - c.a[2]);
            assert!((250.0..=400.0).contains(&moved));
            let end = s.cow_now(s.end_s + 1.0).expect("grazing after");
            assert_eq!(end.pos, c.b);
            assert!(!end.lifted);
            assert_eq!(s.cow_now(s.start_s - 10.0).map(|c| c.pos), Some(c.a));
        }
    }
}

#[test]
fn a_crop_circle_stays_for_the_session() {
    let w = WeatherState::default();
    let often = all(Ufos::Often, &w, 10);
    let s = often
        .iter()
        .find(|s| s.kind == SightingKind::CropCircle)
        .expect("a circle");
    let c = s.circle.as_ref().unwrap();
    let v = [c.centre[0] + 300.0, 420.0, c.centre[1]];
    let mut sky = UfoSky::default();
    assert!(
        !sky.now(&Grid, &inputs(Ufos::Often, c.from_s - 1.0, &w, v))
            .circles
            .contains(c)
    );
    let made = sky.now(&Grid, &inputs(Ufos::Often, c.to_s, &w, v));
    assert!(made.circles.contains(c));
    assert_eq!(c.made(c.to_s), 1.0);
    // Two days later it is still there for this rider.
    let later = sky.now(&Grid, &inputs(Ufos::Often, c.to_s + 2.0 * 86_400.0, &w, v));
    assert!(later.circles.contains(c));
}

/// The unicorn flying a wide circle at 600 m.
fn unicorn(t: f64) -> Unicorn {
    let a = t * 0.02;
    Unicorn {
        pos: [800.0 * a.cos(), 600.0, 800.0 * a.sin()],
        vel: [-16.0 * a.sin(), 0.0, 16.0 * a.cos()],
    }
}

/// The visits of `minutes` minutes of flight from `t0`: (start, phases with
/// their start times, the least distance to the unicorn).
/// A visit: its start, its phases with their start times, the least
/// distance to the unicorn.
type Visit = (f64, Vec<(Phase, f64)>, f64);

fn fly(level: Ufos, t0: f64, minutes: f64) -> Vec<Visit> {
    let mut c = Curiosity::default();
    let mut out: Vec<Visit> = Vec::new();
    let mut on = false;
    let steps = (minutes * 60.0 * 30.0) as usize;
    for i in 0..steps {
        let t = t0 + i as f64 / 30.0;
        let u = unicorn(t);
        let now = c.step(42, level, t, Some(&u));
        match (now, on) {
            (Some(n), _) => {
                if !on {
                    out.push((t, Vec::new(), f64::MAX));
                }
                let v = out.last_mut().unwrap();
                let p = c.phase().unwrap_or(Phase::Leave);
                if v.1.last().is_none_or(|l| l.0 != p) {
                    v.1.push((p, t));
                }
                if !n.departing {
                    let d = len3(n.pos, u.pos);
                    v.2 = v.2.min(d);
                }
                on = true;
            }
            (None, _) => on = false,
        }
    }
    out
}

fn len3(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

#[test]
fn the_curiosity_rule_comes_on_time_circles_follows_and_leaves() {
    let t0 = DAY0 + 18.0 * 3600.0;
    let visits = fly(Ufos::Often, t0, 60.0);
    let len = curiosity::slot_s(Ufos::Often).unwrap();
    // Every due slot after the unicorn settled has its visit, at its time
    // (or right after the one before ended).
    let due: Vec<f64> = (((t0 / len) as i64)..((t0 + 3600.0) / len) as i64)
        .filter_map(|k| curiosity::due(42, Ufos::Often, k))
        .filter(|t| *t >= t0 + curiosity::SETTLE_S && *t < t0 + 3500.0)
        .collect();
    assert!(
        visits.len() >= due.len() * 2 / 3,
        "{} visits, {} due",
        visits.len(),
        due.len()
    );
    assert!(visits.len() <= due.len() + 1);
    for (start, _, _) in &visits {
        assert!(
            due.iter().any(|d| *start >= *d - 0.1 && *start < *d + len),
            "a visit at {start} without its slot"
        );
    }
    for (start, phases, closest) in &visits {
        let kinds: Vec<Phase> = phases.iter().map(|p| p.0).collect();
        if *start + 150.0 > t0 + 3600.0 {
            continue;
        }
        assert_eq!(
            kinds,
            [Phase::Approach, Phase::Circle, Phase::Follow, Phase::Leave],
            "{start}"
        );
        let at = |p: Phase| phases.iter().find(|x| x.0 == p).unwrap().1;
        assert!(at(Phase::Circle) - at(Phase::Approach) <= curiosity::APPROACH_MAX_S + 0.1);
        let circling = at(Phase::Follow) - at(Phase::Circle);
        assert!((curiosity::CIRCLE_S.0 - 0.1..=curiosity::CIRCLE_S.1 + 0.1).contains(&circling));
        let following = at(Phase::Leave) - at(Phase::Follow);
        assert!((curiosity::FOLLOW_S.0 - 0.1..=curiosity::FOLLOW_S.1 + 0.1).contains(&following));
        // Never touching the unicorn.
        assert!(*closest >= curiosity::KEEP_M - 0.01, "{closest}");
        assert!(*closest < 80.0, "it comes close: {closest}");
    }
    // Rare: fewer visits.
    let rare = fly(Ufos::Rare, t0, 60.0);
    assert!(
        !rare.is_empty() && rare.len() * 2 < visits.len(),
        "{} / {}",
        rare.len(),
        visits.len()
    );
    // Not before the unicorn has flown a while.
    assert!(visits[0].0 >= t0 + curiosity::SETTLE_S - 0.1);
}

#[test]
fn flown_straight_at_it_zips_away() {
    let mut c = Curiosity::default();
    let t0 = DAY0;
    let mut u = Unicorn {
        pos: [0.0, 500.0, 0.0],
        vel: [0.0; 3],
    };
    c.start_now(7, t0, &u);
    let mut t = t0;
    // Hover until it circles.
    while c.phase() != Some(Phase::Circle) {
        t += 1.0 / 30.0;
        c.step(7, Ufos::Rare, t, Some(&u)).expect("visiting");
        assert!(t - t0 < 30.0);
    }
    for _ in 0..60 {
        t += 1.0 / 30.0;
        c.step(7, Ufos::Rare, t, Some(&u));
    }
    // Fly straight at it at 25 m/s.
    let chase_from = t;
    loop {
        t += 1.0 / 30.0;
        let ufo = c.step(7, Ufos::Rare, t, Some(&u)).expect("still there");
        if ufo.departing {
            break;
        }
        let d = [
            ufo.pos[0] - u.pos[0],
            ufo.pos[1] - u.pos[1],
            ufo.pos[2] - u.pos[2],
        ];
        let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        u.vel = [d[0] / l * 25.0, d[1] / l * 25.0, d[2] / l * 25.0];
        for k in 0..3 {
            u.pos[k] += u.vel[k] / 30.0;
        }
        assert!(t - chase_from < 3.0, "it should flee");
    }
    assert_eq!(c.chased, 1);
    // Gone after the dart.
    for _ in 0..(30.0 * AWAY_S) as usize + 5 {
        t += 1.0 / 30.0;
        c.step(7, Ufos::Rare, t, Some(&u));
    }
    assert!(c.phase().is_none());
    // Riding again (no unicorn): no visitor.
    assert!(c.step(7, Ufos::Often, t + 1.0, None).is_none());
}
