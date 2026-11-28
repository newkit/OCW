//! Live aircraft (spec 14 Phase 18, 18C, 18D, 18E T10; Q6 (b)): real
//! positions from an ADS-B network, as the game server shares them
//! (`LiveAirUpdate`), turned into the sky's [`AircraftNow`] and
//! [`Heard`] so that the renderer and the mixer draw and play them with
//! T8's models and sounds.
//!
//! **No identities** (18D *Privacy*): a [`LiveAircraft`] has a number the
//! server gave it for this session, its class ([`AirKind`]), where it was,
//! how fast and which way it flew and climbed – never an ICAO address,
//! callsign, registration, operator or type code.
//!
//! **Extrapolation** between the server's fetches (one a minute at best,
//! 18D): along the great circle of the aircraft's track at its ground
//! speed, and its vertical rate for at most [`CLIMB_S`] (climbs and
//! descents level off); never longer than [`MAX_AGE_S`] after the fix.
//!
//! Pure and wasm-safe; callers pass the instant.

use ocw_world::geo::{EARTH_RADIUS_M, LocalProjection};

use super::relief::Relief;
use super::upper::{Contrail, UpperAir};
use super::{
    AirKind, AircraftNow, HEARD_M, Heard, Mission, SPEED_OF_SOUND_MPS, TRAIL_AGES, Trail,
    TrailPoint, mix,
};

/// A fix is extrapolated for at most this long (s); older ones are not
/// drawn (the client draws the simulated sky instead).
pub const MAX_AGE_S: f64 = 420.0;
/// The vertical rate is followed for this long (s), then the altitude
/// holds.
pub const CLIMB_S: f64 = 60.0;
/// Airborne aircraft stay at least this much above the relief (m): the
/// feed's heights are GNSS or pressure altitudes, our ground a DEM.
pub const CLEARANCE_M: f64 = 30.0;
/// The ids of live aircraft in [`AircraftNow::id`]: apart from the
/// simulated flights'.
pub const ID_BIT: u64 = 1 << 62;

/// One live aircraft as the server shares it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LiveAircraft {
    /// The server's number for it in this session (not an identity).
    pub id: u32,
    pub kind: AirKind,
    /// Where it was (degrees) and how high (m above sea level).
    pub lat: f64,
    pub lon: f64,
    pub alt_m: f64,
    /// Ground speed (m/s), true track (°), vertical rate (m/s).
    pub speed_mps: f64,
    pub track_deg: f64,
    pub climb_mps: f64,
    /// When it was there (Unix ms).
    pub at_ms: i64,
}

/// The point `dist_m` from (lat, lon) along the great circle starting at
/// the true course `course_deg` (degrees in and out).
pub fn destination(lat: f64, lon: f64, course_deg: f64, dist_m: f64) -> (f64, f64) {
    let d = dist_m / EARTH_RADIUS_M;
    let (p1, l1, th) = (lat.to_radians(), lon.to_radians(), course_deg.to_radians());
    let sp2 = p1.sin() * d.cos() + p1.cos() * d.sin() * th.cos();
    let p2 = sp2.clamp(-1.0, 1.0).asin();
    let l2 = l1 + (th.sin() * d.sin() * p1.cos()).atan2(d.cos() - p1.sin() * sp2);
    let lon2 = (l2.to_degrees() + 540.0).rem_euclid(360.0) - 180.0;
    (p2.to_degrees(), lon2)
}

/// The great-circle distance (m) between two points (haversine).
pub fn distance_m(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (p1, p2) = (a.0.to_radians(), b.0.to_radians());
    let dp = p2 - p1;
    let dl = (b.1 - a.1).to_radians();
    let h = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_M * h.sqrt().min(1.0).asin()
}

/// The initial true course (°) of the great circle from `a` to `b`.
pub fn course_deg(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (p1, p2) = (a.0.to_radians(), b.0.to_radians());
    let dl = (b.1 - a.1).to_radians();
    let y = dl.sin() * p2.cos();
    let x = p1.cos() * p2.sin() - p1.sin() * p2.cos() * dl.cos();
    y.atan2(x).to_degrees().rem_euclid(360.0)
}

impl LiveAircraft {
    /// Seconds since the fix at `t_ms` (negative: before it).
    pub fn age_s(&self, t_ms: i64) -> f64 {
        (t_ms - self.at_ms) as f64 / 1000.0
    }

    /// Where it is at `t_ms` (lat, lon, m above sea level), extrapolated;
    /// `None` when the fix is older than [`MAX_AGE_S`] (or from the
    /// future by more than a minute).
    pub fn at(&self, t_ms: i64) -> Option<(f64, f64, f64)> {
        let age = self.age_s(t_ms);
        if !(-60.0..=MAX_AGE_S).contains(&age) {
            return None;
        }
        let (lat, lon) = destination(self.lat, self.lon, self.track_deg, self.speed_mps * age);
        let alt = self.alt_m + self.climb_mps * age.clamp(-CLIMB_S, CLIMB_S);
        Some((lat, lon, alt.max(0.0)))
    }

    /// Its velocity (x east, y up, z south; m/s) at `t_ms`.
    pub fn velocity(&self, t_ms: i64) -> [f64; 3] {
        let age = self.age_s(t_ms);
        // The course changes along a great circle; over our distances the
        // start course is close enough, the climb stops after CLIMB_S.
        let h = self.track_deg.to_radians();
        let climb = if age.abs() <= CLIMB_S {
            self.climb_mps
        } else {
            0.0
        };
        [h.sin() * self.speed_mps, climb, -h.cos() * self.speed_mps]
    }
}

/// The mission an aircraft seems to fly (gear, sounds; the `air` command).
fn mission(kind: AirKind, above_m: f64, climb: f64) -> Mission {
    match kind {
        AirKind::Airliner if above_m < 3000.0 && climb < -1.0 => Mission::Arrival,
        AirKind::Airliner if above_m < 3000.0 && climb > 1.0 => Mission::Departure,
        AirKind::Airliner => Mission::Cruise,
        AirKind::SmallPlane => Mission::CrossCountry,
        AirKind::Helicopter => Mission::Tour,
        AirKind::Glider => Mission::Soaring,
        AirKind::Balloon => Mission::Drift,
    }
}

/// Where the world's projection puts `a` at `t_ms` (x, y, z; m), kept
/// above the relief; `None` when too old.
pub fn position(
    a: &LiveAircraft,
    proj: &LocalProjection,
    relief: &Relief,
    t_ms: i64,
) -> Option<[f64; 3]> {
    let (lat, lon, alt) = a.at(t_ms)?;
    let p = proj.project(lat, lon);
    let floor = relief.max_at(p[0], p[1]) + CLEARANCE_M;
    Some([p[0], alt.max(floor), p[1]])
}

/// The contrail behind a live airliner at `pos` with velocity `vel`: the
/// straight line back along its track (we keep no history, 18D), drifted
/// by the wind aloft; it ends where the air made no trail.
pub fn trail(pos: [f64; 3], vel: [f64; 3], upper: &UpperAir) -> Option<Trail> {
    let kind = upper.contrail(pos[1]);
    if kind == Contrail::None {
        return None;
    }
    let life = kind.lifetime_s();
    let wind = upper.wind_at(pos[1]);
    let mut points = Vec::with_capacity(TRAIL_AGES.len());
    for age in TRAIL_AGES {
        let age = age.min(life);
        let p = [
            pos[0] - vel[0] * age,
            pos[1] - vel[1] * age.min(CLIMB_S),
            pos[2] - vel[2] * age,
        ];
        if upper.contrail(p[1]) == Contrail::None {
            break;
        }
        points.push(TrailPoint {
            pos: [p[0] + wind[0] * age, p[1], p[2] + wind[1] * age],
            age_s: age,
        });
        if age >= life {
            break;
        }
    }
    (points.len() >= 2).then_some(Trail { kind, points })
}

/// What one frame of the live sky needs.
#[derive(Debug, Clone, Copy)]
pub struct LiveFrame<'a> {
    /// The instant (Unix ms; the world clock, which follows the real one
    /// when live aircraft are drawn).
    pub t_ms: i64,
    pub viewer: [f64; 3],
    pub radius_m: f64,
    pub max_aircraft: usize,
    pub contrails: bool,
    pub upper: &'a UpperAir,
    /// The ground under the viewer (m): the field height of the gear rule.
    pub ground_m: f64,
    /// The aerodromes (x, z, elevation; [`super::AirSpace::field_points`]):
    /// the gear rule of [`live_gear`].
    pub fields: &'a [[f64; 3]],
}

/// A live aircraft's gear (18C, v7.08): fixed gear always down; an
/// airliner's down on the ground (under 10 m over it), on final (sinking,
/// under 600 m over the nearest aerodrome within 14 km) and just after
/// take-off or before touchdown (under 40 m over it within 5 km), up
/// otherwise – also with no aerodrome near (cruise, unknown).
pub fn live_gear(
    kind: AirKind,
    pos: [f64; 3],
    climb: f64,
    above_ground: f64,
    fields: &[[f64; 3]],
) -> bool {
    if kind != AirKind::Airliner || above_ground < 10.0 {
        return true;
    }
    let Some((d, e)) = fields
        .iter()
        .map(|f| ((pos[0] - f[0]).hypot(pos[2] - f[1]), f[2]))
        .min_by(|a, b| a.0.total_cmp(&b.0))
    else {
        return false;
    };
    let above = pos[1] - e;
    climb < -0.5 && above < 600.0 && d < 14_000.0 || above < 40.0 && d < 5000.0
}

fn rotor(kind: AirKind) -> f64 {
    match kind {
        AirKind::Glider | AirKind::Balloon => 0.0,
        _ => 1.0,
    }
}

/// The live aircraft at `f.t_ms` within the radius, the nearest within
/// the budget; `offsets` moves each (by its id) a little – the client's
/// blending of corrections after a new fetch.
pub fn now(
    aircraft: &[LiveAircraft],
    proj: &LocalProjection,
    relief: &Relief,
    f: &LiveFrame,
    offset: impl Fn(u32) -> [f64; 3],
) -> Vec<AircraftNow> {
    let v = f.viewer;
    let mut out: Vec<(f64, AircraftNow)> = aircraft
        .iter()
        .filter_map(|a| {
            let mut pos = position(a, proj, relief, f.t_ms)?;
            let o = offset(a.id);
            pos = [pos[0] + o[0], pos[1] + o[1], pos[2] + o[2]];
            let d = (pos[0] - v[0]).hypot(pos[2] - v[2]);
            if d > f.radius_m {
                return None;
            }
            let vel = a.velocity(f.t_ms);
            let above = pos[1] - relief.max_at(pos[0], pos[2]).max(f.ground_m.min(pos[1]));
            let mission = mission(a.kind, above, vel[1]);
            let gear_down = live_gear(a.kind, pos, vel[1], above, f.fields);
            let id = ID_BIT | u64::from(a.id);
            let burner = a.kind == AirKind::Balloon && {
                let slot = (f.t_ms / 6000) as u64;
                mix(id ^ slot).is_multiple_of(4) && (f.t_ms % 6000) < 3000
            };
            let trail = (f.contrails && a.kind == AirKind::Airliner)
                .then(|| trail(pos, vel, f.upper))
                .flatten();
            Some((
                d,
                AircraftNow {
                    id,
                    kind: a.kind,
                    mission,
                    pos,
                    vel,
                    bank: 0.0,
                    on_ground: false,
                    gear_down,
                    rotor: rotor(a.kind),
                    burner,
                    look: mix(id) as u32,
                    model: super::pick_model(a.kind, mission, id),
                    trail,
                },
            ))
        })
        .collect();
    out.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.id.cmp(&b.1.id)));
    out.truncate(f.max_aircraft);
    out.into_iter().map(|(_, a)| a).collect()
}

/// The live aircraft heard at `listener` at `t_ms`, each from where the
/// sound now arriving left it (on its extrapolated straight line).
pub fn heard(
    aircraft: &[LiveAircraft],
    proj: &LocalProjection,
    relief: &Relief,
    t_ms: i64,
    listener: [f64; 3],
) -> Vec<Heard> {
    let dist = |p: [f64; 3]| {
        ((p[0] - listener[0]).powi(2) + (p[1] - listener[1]).powi(2) + (p[2] - listener[2]).powi(2))
            .sqrt()
    };
    let mut out: Vec<Heard> = aircraft
        .iter()
        .filter_map(|a| {
            let here = position(a, proj, relief, t_ms)?;
            if dist(here) > HEARD_M + 2000.0 {
                return None;
            }
            let mut delay = 0.0_f64;
            let mut pos = here;
            for _ in 0..24 {
                let te = t_ms - (delay * 1000.0) as i64;
                pos = position(a, proj, relief, te)?;
                let next = dist(pos) / SPEED_OF_SOUND_MPS;
                let done = (next - delay).abs() < 1e-4;
                delay = next;
                if done {
                    break;
                }
            }
            let d = dist(pos);
            let te = t_ms - (delay * 1000.0) as i64;
            (d <= HEARD_M).then(|| Heard {
                id: ID_BIT | u64::from(a.id),
                kind: a.kind,
                pos,
                vel: a.velocity(te),
                distance_m: d,
                delay_s: delay,
                on_ground: false,
                rotor: rotor(a.kind),
                burner: false,
            })
        })
        .collect();
    out.sort_by(|a, b| a.distance_m.total_cmp(&b.distance_m));
    out
}

/// The class of an aircraft from its ADS-B emitter category (DO-260B:
/// A1 light, A2 small, A3 large, A4 B757, A5 heavy, A6 high performance,
/// A7 rotorcraft; B1 glider, B2 lighter than air, B4 ultralight, …) and,
/// when the category says nothing, its ICAO type designator and how high
/// and fast it flies. `None`: nothing we draw (parachutists, drones,
/// space vehicles, ground vehicles and obstacles).
pub fn classify(
    category: Option<&str>,
    type_code: Option<&str>,
    alt_m: f64,
    speed_mps: f64,
) -> Option<AirKind> {
    let t = type_code.unwrap_or("").trim().to_ascii_uppercase();
    match category.map(str::trim).unwrap_or("") {
        "A1" => return Some(AirKind::SmallPlane),
        "A2" | "A3" | "A4" | "A5" | "A6" => return Some(AirKind::Airliner),
        "A7" => return Some(AirKind::Helicopter),
        "B1" => return Some(AirKind::Glider),
        "B2" => return Some(AirKind::Balloon),
        "B4" => return Some(AirKind::SmallPlane),
        "B3" | "B6" | "B7" | "C0" | "C1" | "C2" | "C3" | "C4" | "C5" | "C6" | "C7" => {
            return None;
        }
        _ => {}
    }
    if t == "BALL" || t == "AIRSHIP" || t == "SHIP" {
        return Some(AirKind::Balloon);
    }
    if t == "GLID" || t.starts_with("DG") || t.starts_with("LS") || t.starts_with("ASK") {
        return Some(AirKind::Glider);
    }
    const ROTOR: [&str; 22] = [
        "EC", "H1", "H2", "H5", "H6", "AS3", "AS5", "AS6", "A10", "A11", "A13", "A16", "A169",
        "AW1", "R22", "R44", "R66", "B06", "B407", "B429", "BK17", "S76",
    ];
    if !t.is_empty() && ROTOR.iter().any(|p| t.starts_with(p)) {
        return Some(AirKind::Helicopter);
    }
    if alt_m > 6000.0 || speed_mps > 100.0 {
        Some(AirKind::Airliner)
    } else {
        Some(AirKind::SmallPlane)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_weather::state::WeatherState;

    fn liner() -> LiveAircraft {
        LiveAircraft {
            id: 7,
            kind: AirKind::Airliner,
            lat: 47.92,
            lon: 7.96,
            alt_m: 11_000.0,
            speed_mps: 230.0,
            track_deg: 90.0,
            climb_mps: 0.0,
            at_ms: 1_000_000,
        }
    }

    #[test]
    fn great_circle_destination_matches_distance_and_course() {
        let a = (47.92, 7.96);
        for course in [0.0, 45.0, 90.0, 200.0, 315.0] {
            let b = destination(a.0, a.1, course, 50_000.0);
            assert!((distance_m(a, b) - 50_000.0).abs() < 0.5, "{course}");
            let c = course_deg(a, b);
            let diff = (c - course + 540.0).rem_euclid(360.0) - 180.0;
            assert!(diff.abs() < 1e-6, "{course} → {c}");
        }
        // Across the antimeridian.
        let (_, lon) = destination(0.0, 179.9, 90.0, 30_000.0);
        assert!(lon < -179.0);
    }

    #[test]
    fn extrapolation_follows_track_speed_and_climb_and_ages_out() {
        let mut a = liner();
        a.climb_mps = 10.0;
        let (lat, lon, alt) = a.at(a.at_ms + 30_000).unwrap();
        assert!((distance_m((a.lat, a.lon), (lat, lon)) - 6900.0).abs() < 1.0);
        assert!((alt - 11_300.0).abs() < 1e-6);
        // The climb levels off after CLIMB_S.
        let (_, _, alt) = a.at(a.at_ms + 300_000).unwrap();
        assert!((alt - 11_600.0).abs() < 1e-6);
        assert!(a.at(a.at_ms + (MAX_AGE_S as i64 + 1) * 1000).is_none());
        let v = a.velocity(a.at_ms);
        assert!((v[0] - 230.0).abs() < 1e-9 && v[2].abs() < 1e-9 && v[1] == 10.0);
    }

    #[test]
    fn categories_and_types_give_our_classes() {
        assert_eq!(
            classify(Some("A3"), Some("A320"), 9000.0, 230.0),
            Some(AirKind::Airliner)
        );
        assert_eq!(
            classify(Some("A2"), Some("AT76"), 5000.0, 140.0),
            Some(AirKind::Airliner)
        );
        assert_eq!(
            classify(Some("A1"), Some("C172"), 900.0, 50.0),
            Some(AirKind::SmallPlane)
        );
        assert_eq!(
            classify(Some("A7"), Some("EC45"), 600.0, 60.0),
            Some(AirKind::Helicopter)
        );
        assert_eq!(
            classify(Some("B1"), None, 1500.0, 28.0),
            Some(AirKind::Glider)
        );
        assert_eq!(
            classify(Some("B2"), None, 600.0, 3.0),
            Some(AirKind::Balloon)
        );
        assert_eq!(classify(Some("B6"), None, 100.0, 10.0), None, "drones");
        assert_eq!(
            classify(Some("C1"), None, 0.0, 10.0),
            None,
            "ground vehicles"
        );
        // No category: by type, else height and speed.
        assert_eq!(
            classify(None, Some("EC35"), 400.0, 50.0),
            Some(AirKind::Helicopter)
        );
        assert_eq!(
            classify(None, Some("GLID"), 900.0, 25.0),
            Some(AirKind::Glider)
        );
        assert_eq!(
            classify(None, None, 10_000.0, 220.0),
            Some(AirKind::Airliner)
        );
        assert_eq!(
            classify(Some(""), None, 800.0, 45.0),
            Some(AirKind::SmallPlane)
        );
    }

    #[test]
    fn frames_keep_the_budget_the_relief_and_the_contrail_rule() {
        let proj = LocalProjection::new(47.92, 7.96);
        let relief = Relief::default();
        let cold = WeatherState {
            temperature_c: -2.0,
            ..WeatherState::default()
        };
        let upper = UpperAir::from_weather(&cold, 300.0);
        let mut low = liner();
        low.id = 8;
        low.kind = AirKind::SmallPlane;
        low.alt_m = -50.0;
        low.speed_mps = 50.0;
        let set = [liner(), low];
        let f = LiveFrame {
            t_ms: 1_010_000,
            viewer: [0.0, 300.0, 0.0],
            radius_m: 110_000.0,
            max_aircraft: 30,
            contrails: true,
            upper: &upper,
            ground_m: 300.0,
            fields: &[],
        };
        let a = now(&set, &proj, &relief, &f, |_| [0.0; 3]);
        assert_eq!(a.len(), 2);
        let plane = a.iter().find(|a| a.kind == AirKind::SmallPlane).unwrap();
        assert!(plane.pos[1] >= CLEARANCE_M, "never under the ground");
        assert!(a.iter().all(|a| a.id & ID_BIT != 0));
        let one = now(
            &set,
            &proj,
            &relief,
            &LiveFrame {
                max_aircraft: 1,
                ..f
            },
            |_| [0.0; 3],
        );
        assert_eq!(one.len(), 1);
        // Too old: nothing.
        let late = now(
            &set,
            &proj,
            &relief,
            &LiveFrame {
                t_ms: 1_000_000 + 500_000,
                ..f
            },
            |_| [0.0; 3],
        );
        assert!(late.is_empty());
        // A hot dry day: no trails.
        let hot = WeatherState {
            temperature_c: 38.0,
            humidity: 0.15,
            ..WeatherState::default()
        };
        let dry = UpperAir::from_weather(&hot, 300.0);
        let a = now(
            &set,
            &proj,
            &relief,
            &LiveFrame { upper: &dry, ..f },
            |_| [0.0; 3],
        );
        assert!(a.iter().all(|a| a.trail.is_none()));
    }

    #[test]
    fn sound_comes_from_behind() {
        let proj = LocalProjection::new(47.92, 7.96);
        let relief = Relief::default();
        let mut a = liner();
        a.alt_m = 3000.0;
        a.lat = 47.92;
        a.lon = 7.96;
        let h = heard(&[a], &proj, &relief, a.at_ms + 10_000, [0.0, 0.0, 0.0]);
        assert_eq!(h.len(), 1);
        let h = h[0];
        assert!((h.delay_s - h.distance_m / SPEED_OF_SOUND_MPS).abs() < 0.01);
        // Flying east: heard west of where it is.
        let now_x = position(&a, &proj, &relief, a.at_ms + 10_000).unwrap()[0];
        assert!(h.pos[0] < now_x);
    }

    /// Live airliners: gear up at cruise and with no aerodrome near, down
    /// on final near one and on the ground; fixed gear always down.
    #[test]
    fn live_gear_follows_the_phase_near_an_aerodrome() {
        let field = [[10_000.0, 0.0, 600.0]];
        let a = AirKind::Airliner;
        // Cruise: up, with or without an aerodrome.
        assert!(!live_gear(a, [0.0, 11_000.0, 0.0], 0.0, 10_000.0, &field));
        assert!(!live_gear(a, [0.0, 1_000.0, 0.0], -4.0, 400.0, &[]));
        // On final 6 km out, 350 m over the field, sinking: down.
        assert!(live_gear(a, [4_000.0, 950.0, 0.0], -4.0, 350.0, &field));
        // The same height 30 km out: up (a low pass, not an approach).
        assert!(!live_gear(a, [-20_000.0, 950.0, 0.0], -4.0, 350.0, &field));
        // Climbing out at 300 m over the field: up.
        assert!(!live_gear(a, [9_000.0, 900.0, 0.0], 8.0, 300.0, &field));
        // On the ground: down.
        assert!(live_gear(a, [10_000.0, 600.0, 0.0], 0.0, 2.0, &field));
        // Fixed gear.
        assert!(live_gear(
            AirKind::SmallPlane,
            [0.0, 3000.0, 0.0],
            0.0,
            2500.0,
            &[]
        ));
    }
}
