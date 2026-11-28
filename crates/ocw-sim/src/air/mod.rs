//! Simulated aircraft (spec 14 Phase 18, 18C, 18E T8): airliners on the
//! airways at cruise and on approach and departure at the airports near the
//! world, small planes on circuits and cross-country flights from small
//! fields, helicopters to hospital pads and from heliports, gliders
//! thermalling on soaring days, hot-air balloons on calm mornings – and the
//! contrails of the airliners where the upper air is cold and humid enough.
//!
//! **The same sky for every rider, without network traffic** (18A, Q2):
//! aircraft never touch a rider, so each client draws them for itself
//! from the session's air seed (`Welcome.air_seed`), the world clock, the
//! package's aerodromes (section `AERO`, else the airports of
//! [`ocw_world::aero::NEARBY`] around its origin) and the session's
//! weather. Time is cut into slots per *generator* (an airway direction,
//! an airport's arrivals, a field's circuits, a hospital's pad …); a
//! slot's flight exists or not by a hash of the seed, the generator and
//! the slot and by the density of its local hour, and its path is built
//! from that hash alone – so two clients at the same clock see the same
//! flights at the same places (bit for bit on the same platform; the
//! existence decisions are integer hashes everywhere).
//!
//! Pure and wasm-safe: no clocks, threads or files; callers pass the
//! instant ([`AirInputs::now_s`]).

pub mod airways;
pub mod live;
pub mod mixed;
pub mod path;
pub mod relief;
#[cfg(test)]
mod tests;
pub mod upper;

use std::collections::HashMap;
use std::f64::consts::TAU;
use std::sync::Arc;

use ocw_weather::state::WeatherState;
use ocw_world::aero::{Aero, AerodromeKind};
use ocw_world::geo::LocalProjection;
use ocw_world::package::WorldPackage;

use path::{FlightPath, PathBuilder, PathState, Waypoint};
use relief::Relief;
use upper::{Contrail, UpperAir};

/// The kinds of aircraft (one model file each, spec 18G).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AirKind {
    Airliner,
    SmallPlane,
    Helicopter,
    Glider,
    Balloon,
}

impl AirKind {
    pub const ALL: [Self; 5] = [
        Self::Airliner,
        Self::SmallPlane,
        Self::Helicopter,
        Self::Glider,
        Self::Balloon,
    ];

    /// The model's id in `artwork/3dmodels/` (T6).
    pub fn model_id(self) -> &'static str {
        match self {
            Self::Airliner => "vehicles/airliner-twinjet",
            Self::SmallPlane => "vehicles/plane-high-wing",
            Self::Helicopter => "vehicles/helicopter-light",
            Self::Glider => "vehicles/glider-sailplane",
            Self::Balloon => "vehicles/balloon-hot-air",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Airliner => "airliner",
            Self::SmallPlane => "small plane",
            Self::Helicopter => "helicopter",
            Self::Glider => "glider",
            Self::Balloon => "balloon",
        }
    }
}

/// What a flight does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mission {
    /// An airliner crossing the region on an airway.
    Cruise,
    /// Landing at an airport.
    Arrival,
    /// Taking off from an airport.
    Departure,
    /// Circuits (touch and go) at a small field.
    Circuit,
    /// A small plane flying across the region on a fine day.
    CrossCountry,
    /// A rescue helicopter to a hospital's pad and away.
    Rescue,
    /// A helicopter's round trip from a heliport.
    Tour,
    /// A glider climbing in thermals.
    Soaring,
    /// A balloon drifting with the wind.
    Drift,
}

impl Mission {
    pub fn key(self) -> &'static str {
        match self {
            Self::Cruise => "cruise",
            Self::Arrival => "arrival",
            Self::Departure => "departure",
            Self::Circuit => "circuit",
            Self::CrossCountry => "cross-country",
            Self::Rescue => "rescue",
            Self::Tour => "tour",
            Self::Soaring => "soaring",
            Self::Drift => "drift",
        }
    }
}

/// A runway direction in use: where the wheels touch first and which way.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RunwayUse {
    /// The threshold (x, z).
    pub threshold: [f64; 2],
    /// The direction of landing and take-off (true heading, °).
    pub heading_deg: f64,
    pub length_m: f64,
    /// The ground at the threshold and at the far end (m).
    pub elevation_m: f64,
    pub end_elevation_m: f64,
}

impl RunwayUse {
    /// The runway's surface `along` metres past the threshold.
    pub fn ground_at(&self, along: f64) -> f64 {
        let f = (along / self.length_m.max(1.0)).clamp(0.0, 1.0);
        self.elevation_m + (self.end_elevation_m - self.elevation_m) * f
    }

    /// The unit vector along the heading (x east, z south).
    pub fn dir(&self) -> [f64; 2] {
        heading_vec(self.heading_deg)
    }
}

/// One flight.
#[derive(Debug, Clone, PartialEq)]
pub struct Flight {
    pub id: u64,
    pub kind: AirKind,
    pub mission: Mission,
    /// When it starts (s since the Unix epoch, world clock).
    pub start_s: f64,
    pub path: FlightPath,
    /// The look (paint) seed.
    pub look: u32,
    /// The runway it lands on or takes off from (arrivals, departures,
    /// circuits).
    pub runway: Option<RunwayUse>,
    /// The ground it starts from or lands on (m; gear and lights).
    pub field_m: f64,
}

impl Flight {
    pub fn end_s(&self) -> f64 {
        self.start_s + self.path.duration_s()
    }

    pub fn alive(&self, t: f64) -> bool {
        t >= self.start_s && t <= self.end_s()
    }

    pub fn state(&self, t: f64) -> PathState {
        self.path.at(t - self.start_s)
    }
}

/// An aircraft now, for the renderer.
#[derive(Debug, Clone, PartialEq)]
pub struct AircraftNow {
    pub id: u64,
    pub kind: AirKind,
    pub mission: Mission,
    pub pos: [f64; 3],
    pub vel: [f64; 3],
    /// Bank (rad, positive: right wing down).
    pub bank: f64,
    pub on_ground: bool,
    pub gear_down: bool,
    /// Propeller / rotor speed (0 still … 1 flying).
    pub rotor: f64,
    /// A balloon's burner firing.
    pub burner: bool,
    pub look: u32,
    pub trail: Option<Trail>,
}

/// A contrail: points along where the aircraft was, drifted by the wind.
#[derive(Debug, Clone, PartialEq)]
pub struct Trail {
    pub kind: Contrail,
    /// From the aircraft back; age 0 at the engines.
    pub points: Vec<TrailPoint>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrailPoint {
    pub pos: [f64; 3],
    pub age_s: f64,
}

/// An aircraft as it is heard: from where it was when the sound now
/// arriving left it (sound travels at 343 m/s).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Heard {
    pub id: u64,
    pub kind: AirKind,
    /// Where the sound left it, and its velocity then.
    pub pos: [f64; 3],
    pub vel: [f64; 3],
    pub distance_m: f64,
    /// How long ago that was (s).
    pub delay_s: f64,
    pub on_ground: bool,
    pub rotor: f64,
    pub burner: bool,
}

/// The speed of sound (m/s).
pub const SPEED_OF_SOUND_MPS: f64 = 343.0;
/// Aircraft are heard up to here (m).
pub const HEARD_M: f64 = 25_000.0;

/// What a frame of the sky needs.
#[derive(Debug, Clone, Copy)]
pub struct AirInputs<'a> {
    /// The session's air seed.
    pub seed: u64,
    /// The world clock (s since the Unix epoch).
    pub now_s: f64,
    /// The place's offset from UTC (s; the hour of each slot).
    pub utc_offset_s: i32,
    pub weather: &'a WeatherState,
    /// Where the rider (or the unicorn, the camera) is.
    pub viewer: [f64; 3],
    /// Aircraft farther than this (horizontally, m) are left out.
    pub radius_m: f64,
    /// At most this many (the nearest; the quality level's budget).
    pub max_aircraft: usize,
    /// Contrails drawn (the Quest 1 has none).
    pub contrails: bool,
    /// The ground at the rider (m above the sea): where the weather's
    /// temperature is.
    pub ground_m: f64,
}

/// A runway of an aerodrome (both directions).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Strip {
    ends: [[f64; 2]; 2],
    /// The ground at each end (m).
    ele: [f64; 2],
    heading_deg: f64,
    length_m: f64,
}

/// An aerodrome as the sky uses it.
#[derive(Debug, Clone, PartialEq)]
struct Field {
    name: String,
    pos: [f64; 2],
    elevation_m: f64,
    strips: Vec<Strip>,
    kind: AerodromeKind,
    outside: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Pad {
    pos: [f64; 2],
    elevation_m: f64,
    hospital: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum GenKind {
    Airway { way: usize, reverse: bool },
    Arrival { field: usize },
    Departure { field: usize },
    Circuit { field: usize },
    Cross { field: usize },
    Rescue { pad: usize },
    Tour { pad: usize },
    Soaring { field: usize },
    Balloon { site: usize },
}

/// A source of flights: one slot every `period_s`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Gen {
    id: u32,
    what: GenKind,
    period_s: f64,
    /// The longest flight it makes (s).
    max_s: f64,
    /// Where its flights are, roughly (x, z) and how far around (m).
    centre: [f64; 2],
    reach_m: f64,
}

/// The airspace of a world: its aerodromes, pads, airways and the relief
/// under them – built once per world.
#[derive(Debug, Clone, PartialEq)]
pub struct AirSpace {
    /// The package's origin (lat, lon): the sun of the slots.
    pub origin: (f64, f64),
    relief: Relief,
    fields: Vec<Field>,
    pads: Vec<Pad>,
    ways: Vec<(Vec<[f64; 2]>, f64)>,
    balloon_sites: Vec<[f64; 3]>,
    /// The exact ground every kilometre (where a balloon comes down).
    landings: Vec<[f64; 3]>,
    gens: Vec<Gen>,
}

/// Landing places for balloons: every [`LANDING_GRID_M`], the exact ground.
pub const LANDING_GRID_M: f64 = 1000.0;

/// SplitMix64.
pub fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A stream of uniform numbers from one hash.
#[derive(Debug, Clone, Copy)]
struct Draw(u64);

impl Draw {
    fn new(h: u64) -> Self {
        Self(h)
    }
    /// The next number in 0 … 1.
    fn f(&mut self) -> f64 {
        self.0 = mix(self.0);
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.f()
    }
    fn pick(&mut self, n: usize) -> usize {
        ((self.f() * n as f64) as usize).min(n.saturating_sub(1))
    }
}

/// The unit vector of a true heading (x east, z south).
pub fn heading_vec(deg: f64) -> [f64; 2] {
    let h = deg.to_radians();
    [h.sin(), -h.cos()]
}

/// The true heading (°) of a direction (x, z).
pub fn heading_of(d: [f64; 2]) -> f64 {
    d[0].atan2(-d[1]).to_degrees().rem_euclid(360.0)
}

fn add(p: [f64; 2], d: [f64; 2], s: f64) -> [f64; 2] {
    [p[0] + d[0] * s, p[1] + d[1] * s]
}

fn at(p: [f64; 2], y: f64) -> [f64; 3] {
    [p[0], y, p[1]]
}

fn dist2(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// The right-hand normal of a direction.
fn right(d: [f64; 2]) -> [f64; 2] {
    [-d[1], d[0]]
}

/// The local hour (0 … 24) and month (1 … 12) at `t`.
pub fn local_hour_month(t: f64, offset_s: i32) -> (f64, u32) {
    let local = t + f64::from(offset_s);
    let days = (local / 86_400.0).floor();
    let hour = (local - days * 86_400.0) / 3600.0;
    // Civil date from the days since 1970 (H. Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (hour, month as u32)
}

/// Airliners per hour by the local hour (share of the peak): few at
/// night, the morning wave from six, busy all day, thinning after ten.
pub const AIRLINER_HOURS: [f64; 24] = [
    0.15, 0.1, 0.08, 0.08, 0.1, 0.25, 0.6, 0.95, 1.0, 0.95, 0.9, 0.9, 0.9, 0.9, 0.9, 0.95, 1.0,
    1.0, 0.95, 0.9, 0.8, 0.6, 0.4, 0.25,
];

/// Small planes by the local hour: daylight, most in the afternoon.
pub const LIGHT_HOURS: [f64; 24] = [
    0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.3, 0.6, 0.8, 0.9, 0.9, 1.0, 1.0, 1.0, 0.9, 0.8, 0.6,
    0.3, 0.0, 0.0, 0.0, 0.0,
];

/// Rescue helicopters: by day, fewer at night.
pub const RESCUE_HOURS: [f64; 24] = [
    0.3, 0.25, 0.2, 0.2, 0.2, 0.3, 0.5, 0.8, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0,
    0.9, 0.7, 0.5, 0.4, 0.35,
];

fn by_hour(table: &[f64; 24], hour: f64) -> f64 {
    let h = hour.rem_euclid(24.0);
    let i = h.floor() as usize % 24;
    let f = h - h.floor();
    table[i] * (1.0 - f) + table[(i + 1) % 24] * f
}

impl AirSpace {
    /// The airspace of a package: its `AERO` (else the airports of
    /// `NEARBY` around its origin), the relief of its tiles.
    pub fn from_package(pkg: &WorldPackage) -> Self {
        let relief = Relief::of_package(pkg);
        Self::new(pkg.origin, pkg.aero.as_ref(), relief, |x, z| {
            pkg.height(x, z)
        })
    }

    /// The airspace around `origin` (lat, lon) with `aero` (`None`: the
    /// `NEARBY` table within 170 km), `relief` and the exact ground
    /// `height` (aerodromes without an elevation, the pads).
    pub fn new(
        origin: (f64, f64),
        aero: Option<&Aero>,
        relief: Relief,
        height: impl Fn(f64, f64) -> Option<f64>,
    ) -> Self {
        let proj = LocalProjection::new(origin.0, origin.1);
        let mut built;
        let aero = match aero {
            Some(a) if !a.is_empty() => a,
            _ => {
                built = Aero::default();
                built.add_nearby(origin, 170.0, &proj);
                &built
            }
        };
        let elevation = |pos: [f32; 2], ele: Option<f32>| -> f64 {
            height(f64::from(pos[0]), f64::from(pos[1]))
                .or(ele.map(f64::from))
                .filter(|h| h.is_finite())
                .unwrap_or(0.0)
        };
        let fields: Vec<Field> = aero
            .aerodromes
            .iter()
            .enumerate()
            .map(|(i, a)| Field {
                name: if a.iata.is_empty() {
                    a.name.clone()
                } else {
                    a.iata.clone()
                },
                pos: [f64::from(a.pos[0]), f64::from(a.pos[1])],
                elevation_m: elevation(a.pos, a.ele_m),
                strips: aero
                    .runways_of(i)
                    .filter(|r| r.length_m > 150.0)
                    .map(|r| {
                        let field = elevation(a.pos, a.ele_m);
                        let ends = [
                            [f64::from(r.ends[0][0]), f64::from(r.ends[0][1])],
                            [f64::from(r.ends[1][0]), f64::from(r.ends[1][1])],
                        ];
                        let span = dist2(ends[0], ends[1]);
                        Strip {
                            ends,
                            ele: ends.map(|e| {
                                height(e[0], e[1])
                                    .filter(|h| h.is_finite())
                                    .unwrap_or(field)
                            }),
                            heading_deg: f64::from(r.heading_deg),
                            length_m: if span > 100.0 {
                                span
                            } else {
                                f64::from(r.length_m)
                            },
                        }
                    })
                    .collect(),
                kind: a.kind,
                outside: a.outside,
            })
            .collect();
        let pads: Vec<Pad> = aero
            .helipads
            .iter()
            .map(|p| Pad {
                pos: [f64::from(p.pos[0]), f64::from(p.pos[1])],
                elevation_m: elevation(p.pos, p.ele_m),
                hospital: p.hospital,
            })
            .filter(|p| p.pos[0].is_finite())
            .collect();
        let mut ways: Vec<(Vec<[f64; 2]>, f64)> = airways::near(origin, 150.0)
            .into_iter()
            .map(|a| {
                (
                    a.points
                        .iter()
                        .map(|&(lat, lon)| proj.project(lat, lon))
                        .collect(),
                    a.per_hour,
                )
            })
            .collect();
        if ways.is_empty() {
            // Elsewhere: two airways crossing over the world.
            for h in [35.0f64, 125.0] {
                let d = heading_vec(h);
                ways.push((
                    vec![add([0.0; 2], d, -200_000.0), add([0.0; 2], d, 200_000.0)],
                    4.0,
                ));
            }
        }
        // Balloons start from the fields, else from low open ground.
        let mut balloon_sites: Vec<[f64; 3]> = fields
            .iter()
            .filter(|f| {
                !f.outside
                    && matches!(
                        f.kind,
                        AerodromeKind::Airfield
                            | AerodromeKind::Gliding
                            | AerodromeKind::Ultralight
                            | AerodromeKind::Private
                    )
            })
            .map(|f| at(f.pos, f.elevation_m))
            .collect();
        if balloon_sites.is_empty()
            && let Some(p) = relief.lowest_near(0.0, 0.0, 15_000.0)
        {
            balloon_sites.push(at(p, height(p[0], p[1]).unwrap_or(0.0)));
        }
        let mut landings = Vec::new();
        if let Some((lo, hi)) = relief.bounds() {
            let mut z = (lo[1] / LANDING_GRID_M).ceil() * LANDING_GRID_M;
            while z < hi[1] {
                let mut x = (lo[0] / LANDING_GRID_M).ceil() * LANDING_GRID_M;
                while x < hi[0] {
                    if let Some(h) = height(x, z).filter(|h| h.is_finite() && *h > 1.0) {
                        landings.push([x, h, z]);
                    }
                    x += LANDING_GRID_M;
                }
                z += LANDING_GRID_M;
            }
        }
        let mut s = Self {
            origin,
            relief,
            fields,
            pads,
            ways,
            balloon_sites,
            landings,
            gens: Vec::new(),
        };
        s.gens = s.generators();
        s
    }

    /// The sources of flights.
    fn generators(&self) -> Vec<Gen> {
        let mut g = Vec::new();
        let mut push =
            |what: GenKind, period_s: f64, max_s: f64, centre: [f64; 2], reach_m: f64| {
                let id = g.len() as u32;
                g.push(Gen {
                    id,
                    what,
                    period_s,
                    max_s,
                    centre,
                    reach_m,
                });
            };
        for (i, (pts, per_hour)) in self.ways.iter().enumerate() {
            let len: f64 = pts.windows(2).map(|w| dist2(w[0], w[1])).sum();
            for reverse in [false, true] {
                push(
                    GenKind::Airway { way: i, reverse },
                    3600.0 / per_hour,
                    len / 210.0,
                    [0.0; 2],
                    f64::INFINITY,
                );
            }
        }
        for (i, f) in self.fields.iter().enumerate() {
            let airline = matches!(
                f.kind,
                AerodromeKind::International | AerodromeKind::Regional
            );
            if airline && !f.strips.is_empty() {
                let per_hour = match (f.kind, f.outside) {
                    (AerodromeKind::International, _) => 9.0,
                    _ => 2.5,
                };
                for dep in [false, true] {
                    let what = if dep {
                        GenKind::Departure { field: i }
                    } else {
                        GenKind::Arrival { field: i }
                    };
                    push(what, 3600.0 / per_hour, 1500.0, f.pos, 90_000.0);
                }
            }
            if f.outside {
                continue;
            }
            let light = matches!(
                f.kind,
                AerodromeKind::Airfield
                    | AerodromeKind::Ultralight
                    | AerodromeKind::Private
                    | AerodromeKind::Regional
            );
            if light {
                push(GenKind::Circuit { field: i }, 1500.0, 1500.0, f.pos, 6000.0);
                push(GenKind::Cross { field: i }, 2400.0, 1200.0, f.pos, 45_000.0);
            }
            if matches!(
                f.kind,
                AerodromeKind::Gliding | AerodromeKind::Airfield | AerodromeKind::Ultralight
            ) {
                let period = if f.kind == AerodromeKind::Gliding {
                    900.0
                } else {
                    2700.0
                };
                push(
                    GenKind::Soaring { field: i },
                    period,
                    7200.0,
                    f.pos,
                    16_000.0,
                );
            }
        }
        for (i, p) in self.pads.iter().enumerate() {
            if p.hospital {
                push(
                    GenKind::Rescue { pad: i },
                    4.0 * 3600.0,
                    1800.0,
                    p.pos,
                    32_000.0,
                );
            } else {
                push(GenKind::Tour { pad: i }, 5400.0, 1800.0, p.pos, 20_000.0);
            }
        }
        for (i, s) in self.balloon_sites.iter().enumerate() {
            push(
                GenKind::Balloon { site: i },
                1200.0,
                6000.0,
                [s[0], s[2]],
                16_000.0,
            );
        }
        g
    }

    /// The aerodromes (name, place, elevation): the `air` command.
    pub fn fields(&self) -> impl Iterator<Item = (&str, [f64; 2], f64)> {
        self.fields
            .iter()
            .map(|f| (f.name.as_str(), f.pos, f.elevation_m))
    }

    /// How many sources of flights (the `air` command, tests).
    pub fn generator_count(&self) -> usize {
        self.gens.len()
    }

    pub fn relief(&self) -> &Relief {
        &self.relief
    }

    /// The runway direction a field uses now: into the wind (in a calm
    /// the day's choice from the seed); a field without a runway gets a
    /// strip into the wind.
    fn runway_in_use(&self, field: usize, w: &WeatherState, salt: u64) -> RunwayUse {
        let f = &self.fields[field];
        let from = f64::from(w.wind_from_deg);
        let speed = f64::from(w.wind_mps);
        let mut best: Option<(f64, RunwayUse)> = None;
        let strips: Vec<Strip> = if f.strips.is_empty() {
            let h = if speed > 2.0 {
                (from / 10.0).round() * 10.0
            } else {
                (salt % 36) as f64 * 10.0
            };
            let d = heading_vec(h);
            vec![Strip {
                ends: [add(f.pos, d, -350.0), add(f.pos, d, 350.0)],
                ele: [f.elevation_m; 2],
                heading_deg: h,
                length_m: 700.0,
            }]
        } else {
            f.strips.clone()
        };
        let calm = speed < 2.5;
        for (k, s) in strips.iter().enumerate() {
            for (end, h) in [(0, s.heading_deg), (1, (s.heading_deg + 180.0) % 360.0)] {
                let head = ((from - h).to_radians()).cos() * speed;
                let score = if calm {
                    // A calm: the same choice all day, the longest runway.
                    s.length_m * 1e-3
                        + if (salt >> (k + end)) & 1 == 0 {
                            1.0
                        } else {
                            0.0
                        }
                } else {
                    head + s.length_m * 1e-4
                };
                let u = RunwayUse {
                    threshold: s.ends[end],
                    heading_deg: h,
                    length_m: s.length_m,
                    elevation_m: s.ele[end],
                    end_elevation_m: s.ele[1 - end],
                };
                if best.is_none_or(|b| score > b.0) {
                    best = Some((score, u));
                }
            }
        }
        best.expect("a strip").1
    }

    /// Whether a generator's flights can be near `viewer`.
    fn near(&self, g: &Gen, viewer: [f64; 2], radius: f64) -> bool {
        match g.what {
            GenKind::Airway { way, .. } => {
                let pts = &self.ways[way].0;
                pts.windows(2)
                    .any(|w| segment_dist(viewer, w[0], w[1]) < radius + 10_000.0)
            }
            _ => dist2(g.centre, viewer) < radius + g.reach_m,
        }
    }

    /// The flight of slot `k` of generator `g`, if it flies.
    fn flight(&self, g: &Gen, k: i64, inp: &AirInputs) -> Option<Flight> {
        let h = mix(inp.seed ^ mix(u64::from(g.id) << 40 ^ k as u64));
        let mut d = Draw::new(h);
        // Airliners at an airport keep their slots (separated as by the
        // tower: at least 70 % of the period apart); the rest spread out.
        let airport = matches!(g.what, GenKind::Arrival { .. } | GenKind::Departure { .. });
        let jitter = if airport { 0.3 } else { 0.9 };
        // An arrival's slot is its landing; it starts a flight earlier.
        let slot_s = (k as f64 + jitter * d.f()) * g.period_s;
        let start_s = slot_s;
        let keep = d.f();
        let (hour, month) = local_hour_month(start_s, inp.utc_offset_s);
        let w = inp.weather;
        let day_salt =
            mix(inp.seed ^ ((start_s + f64::from(inp.utc_offset_s)) / 86_400.0).floor() as u64);
        let id = h;
        let look = (mix(h ^ 0x10C) >> 32) as u32;
        let f = |kind, mission, path: FlightPath, runway: Option<RunwayUse>, field_m| {
            (!path.is_empty()).then_some(Flight {
                id,
                kind,
                mission,
                start_s,
                path,
                look,
                runway,
                field_m,
            })
        };
        match g.what {
            GenKind::Airway { way, reverse } => {
                if keep > by_hour(&AIRLINER_HOURS, hour) {
                    return None;
                }
                let mut pts = self.ways[way].0.clone();
                if reverse {
                    pts.reverse();
                }
                let dir = [
                    pts[pts.len() - 1][0] - pts[0][0],
                    pts[pts.len() - 1][1] - pts[0][1],
                ];
                let east = heading_of(dir) < 180.0;
                // Flight levels: odd thousands of feet eastbound, even westbound.
                let fl =
                    [29.0, 31.0, 33.0, 35.0, 37.0, 39.0][d.pick(6)] + if east { 0.0 } else { 1.0 };
                let alt = fl * 1000.0 * 0.3048;
                let speed = d.range(225.0, 250.0);
                let side = d.range(-3000.0, 3000.0);
                let mut b = PathBuilder::new(20_000.0);
                for (i, p) in pts.iter().enumerate() {
                    let a = if i + 1 < pts.len() {
                        pts[i + 1]
                    } else {
                        pts[i]
                    };
                    let z = if i > 0 { pts[i - 1] } else { pts[i] };
                    let n = right(norm([a[0] - z[0], a[1] - z[1]]));
                    b.to(Waypoint::air(at(add(*p, n, side), alt), speed));
                }
                f(
                    AirKind::Airliner,
                    Mission::Cruise,
                    b.build(None, 0.0, 0.05),
                    None,
                    0.0,
                )
            }
            GenKind::Arrival { field } => {
                if keep > by_hour(&AIRLINER_HOURS, hour) {
                    return None;
                }
                let rw = self.runway_in_use(field, w, day_salt);
                let path = self.arrival(rw, &mut d);
                // Down on the runway at the slot's time.
                let touchdown = path.time_of_touchdown();
                f(
                    AirKind::Airliner,
                    Mission::Arrival,
                    path,
                    Some(rw),
                    rw.elevation_m,
                )
                .map(|mut fl| {
                    fl.start_s -= touchdown;
                    fl
                })
            }
            GenKind::Departure { field } => {
                if keep > by_hour(&AIRLINER_HOURS, hour) {
                    return None;
                }
                let rw = self.runway_in_use(field, w, day_salt);
                f(
                    AirKind::Airliner,
                    Mission::Departure,
                    self.departure(rw, &mut d),
                    Some(rw),
                    rw.elevation_m,
                )
            }
            GenKind::Circuit { field } => {
                if keep > by_hour(&LIGHT_HOURS, hour) || !upper::fine(w) {
                    return None;
                }
                let rw = self.runway_in_use(field, w, day_salt);
                f(
                    AirKind::SmallPlane,
                    Mission::Circuit,
                    self.circuit(rw, &mut d),
                    Some(rw),
                    rw.elevation_m,
                )
            }
            GenKind::Cross { field } => {
                if keep > 0.6 * by_hour(&LIGHT_HOURS, hour) || !upper::fine(w) {
                    return None;
                }
                let rw = self.runway_in_use(field, w, day_salt);
                f(
                    AirKind::SmallPlane,
                    Mission::CrossCountry,
                    self.cross(rw, &mut d),
                    Some(rw),
                    rw.elevation_m,
                )
            }
            GenKind::Rescue { pad } => {
                if keep > 0.7 * by_hour(&RESCUE_HOURS, hour) || w.fog_visibility_m < 800.0 {
                    return None;
                }
                let p = self.pads[pad];
                f(
                    AirKind::Helicopter,
                    Mission::Rescue,
                    self.rescue(p, &mut d),
                    None,
                    p.elevation_m,
                )
            }
            GenKind::Tour { pad } => {
                if keep > 0.6 * by_hour(&LIGHT_HOURS, hour) || !upper::fine(w) {
                    return None;
                }
                let p = self.pads[pad];
                f(
                    AirKind::Helicopter,
                    Mission::Tour,
                    self.tour(p, &mut d),
                    None,
                    p.elevation_m,
                )
            }
            GenKind::Soaring { field } => {
                let ms = (start_s * 1000.0) as i64 + 3_600_000;
                let sun = ocw_sky::sun::position(ms, self.origin.0, self.origin.1);
                let s = upper::thermals(w, sun.elevation_deg, month, self.origin.0);
                if s < 0.3 || keep > s || !(10.5..16.5).contains(&hour) {
                    return None;
                }
                let rw = self.runway_in_use(field, w, day_salt);
                f(
                    AirKind::Glider,
                    Mission::Soaring,
                    self.soaring(rw, s, w, &mut d),
                    Some(rw),
                    rw.elevation_m,
                )
            }
            GenKind::Balloon { site } => {
                let sun =
                    ocw_sky::sun::position((start_s * 1000.0) as i64, self.origin.0, self.origin.1);
                let morning = hour < 11.0 && (-1.0..22.0).contains(&sun.elevation_deg);
                if !morning || !upper::calm(w) || keep > 0.5 {
                    return None;
                }
                let s = self.balloon_sites[site];
                f(
                    AirKind::Balloon,
                    Mission::Drift,
                    self.drift(s, w, &mut d),
                    None,
                    s[1],
                )
            }
        }
    }

    /// An airliner's approach: from 75 km out, a base point beside the
    /// extended centreline, the intercept at 24 km, the 3° glide slope
    /// from 18 km to the threshold (as planned, never lifted), touchdown
    /// 400 m beyond it and the roll-out.
    fn arrival(&self, rw: RunwayUse, d: &mut Draw) -> FlightPath {
        let u = rw.dir();
        let n = right(u);
        let e = rw.elevation_m;
        let thr = rw.threshold;
        let bearing = d.range(0.0, 360.0);
        let entry = add(rw.threshold, heading_vec(bearing), 75_000.0);
        let side = if (entry[0] - thr[0]) * n[0] + (entry[1] - thr[1]) * n[1] >= 0.0 {
            1.0
        } else {
            -1.0
        };
        let glide = 3f64.to_radians().tan();
        let mut b = PathBuilder::new(3000.0);
        b.to(Waypoint::air(at(entry, e + d.range(4800.0, 6200.0)), 165.0))
            .to(Waypoint::air(
                at(add(add(thr, u, -32_000.0), n, side * 8000.0), e + 2700.0),
                115.0,
            ))
            .to(Waypoint::air(
                at(add(thr, u, -24_000.0), e + 24_000.0 * glide),
                85.0,
            ))
            .to(Waypoint::fixed(
                at(add(thr, u, -18_000.0), e + 18_000.0 * glide),
                78.0,
            ))
            .to(Waypoint::fixed(at(thr, e + 15.0), 72.0))
            .to(Waypoint::ground(
                at(add(thr, u, 400.0), rw.ground_at(400.0)),
                68.0,
            ));
        let stop = (rw.length_m * 0.7)
            .clamp(900.0, 2000.0)
            .min(rw.length_m - 50.0);
        b.to(Waypoint::ground(
            at(add(thr, u, stop), rw.ground_at(stop)),
            14.0,
        ));
        b.build(Some(&self.relief), 300.0, 0.06)
    }

    /// Lined up on the runway, the take-off run, the first climb straight
    /// ahead (as planned), then a turn out to 90 km.
    fn departure(&self, rw: RunwayUse, d: &mut Draw) -> FlightPath {
        let u = rw.dir();
        let e = rw.elevation_m;
        let thr = rw.threshold;
        let run = (rw.length_m * 0.55).clamp(900.0, 1900.0);
        let out = heading_vec(d.range(0.0, 360.0));
        let mut b = PathBuilder::new(4000.0);
        b.to(Waypoint::ground(at(thr, e), 0.0).hold(d.range(10.0, 40.0)))
            .to(Waypoint::ground(
                at(add(thr, u, run), rw.ground_at(run)),
                80.0,
            ))
            .to(Waypoint::fixed(
                at(add(thr, u, run + 9000.0), e + 750.0),
                115.0,
            ))
            .to(Waypoint::air(
                at(add(thr, u, run + 20_000.0), e + 1800.0),
                140.0,
            ))
            .to(Waypoint::air(
                at(
                    add(add(thr, u, run + 20_000.0), out, 70_000.0),
                    e + d.range(7500.0, 9000.0),
                ),
                215.0,
            ));
        b.build(Some(&self.relief), 300.0, 0.06)
    }

    /// Left-hand circuits 1,000 ft over the field, one to three touch and
    /// goes, then a full stop.
    fn circuit(&self, rw: RunwayUse, d: &mut Draw) -> FlightPath {
        let u = rw.dir();
        let l = [-right(u)[0], -right(u)[1]];
        let e = rw.elevation_m;
        let thr = rw.threshold;
        let p = |along: f64, side: f64| add(add(thr, u, along), l, side);
        let mut b = PathBuilder::new(350.0);
        b.to(Waypoint::ground(at(thr, e), 0.0).hold(d.range(20.0, 90.0)))
            .to(Waypoint::ground(
                at(p(320.0, 0.0), rw.ground_at(320.0)),
                30.0,
            ));
        let laps = 1 + d.pick(3);
        for lap in 0..laps {
            b.to(Waypoint::air(at(p(1800.0, 0.0), e + 170.0), 38.0))
                .to(Waypoint::air(at(p(2400.0, 1100.0), e + 300.0), 42.0))
                .to(Waypoint::air(at(p(0.0, 1100.0), e + 300.0), 44.0))
                .to(Waypoint::air(at(p(-1500.0, 1100.0), e + 280.0), 40.0))
                .to(Waypoint::air(at(p(-1900.0, 300.0), e + 200.0), 36.0))
                .to(Waypoint::fixed(at(p(-1200.0, 0.0), e + 110.0), 33.0))
                .to(Waypoint::fixed(at(thr, e + 8.0), 30.0))
                .to(Waypoint::ground(
                    at(p(200.0, 0.0), rw.ground_at(200.0)),
                    28.0,
                ));
            if lap + 1 < laps {
                b.to(Waypoint::ground(
                    at(p(550.0, 0.0), rw.ground_at(550.0)),
                    31.0,
                ));
            } else {
                let stop = (rw.length_m * 0.6).max(450.0);
                b.to(Waypoint::ground(at(p(stop, 0.0), rw.ground_at(stop)), 6.0));
            }
        }
        b.build(Some(&self.relief), 150.0, 0.08)
    }

    /// Out of a field and away 45 km at 1 – 2.5 km (or in and landing).
    fn cross(&self, rw: RunwayUse, d: &mut Draw) -> FlightPath {
        let u = rw.dir();
        let e = rw.elevation_m;
        let thr = rw.threshold;
        let far = add(thr, heading_vec(d.range(0.0, 360.0)), 45_000.0);
        let alt = (e + 900.0).max(d.range(1000.0, 2500.0));
        let mut b = PathBuilder::new(600.0);
        if d.f() < 0.5 {
            b.to(Waypoint::ground(at(thr, e), 0.0).hold(d.range(10.0, 60.0)))
                .to(Waypoint::ground(
                    at(add(thr, u, 350.0), rw.ground_at(350.0)),
                    32.0,
                ))
                .to(Waypoint::air(at(add(thr, u, 3000.0), e + 300.0), 45.0))
                .to(Waypoint::air(at(far, alt), 52.0));
        } else {
            b.to(Waypoint::air(at(far, alt), 52.0))
                .to(Waypoint::air(at(add(thr, u, -5000.0), e + 350.0), 42.0))
                .to(Waypoint::fixed(at(add(thr, u, -1500.0), e + 130.0), 34.0))
                .to(Waypoint::fixed(at(thr, e + 8.0), 30.0))
                .to(Waypoint::ground(
                    at(add(thr, u, 200.0), rw.ground_at(200.0)),
                    28.0,
                ))
                .to(Waypoint::ground(
                    at(add(thr, u, 500.0), rw.ground_at(500.0)),
                    6.0,
                ));
        }
        b.build(Some(&self.relief), 250.0, 0.08)
    }

    /// A rescue helicopter: in from 30 km, a slow approach, hover, down
    /// on the pad, eight to fifteen minutes there, and away.
    fn rescue(&self, p: Pad, d: &mut Draw) -> FlightPath {
        let e = p.elevation_m;
        let (bin, bout) = (
            heading_vec(d.range(0.0, 360.0)),
            heading_vec(d.range(0.0, 360.0)),
        );
        let mut b = PathBuilder::new(400.0);
        b.to(Waypoint::air(
            at(add(p.pos, bin, 30_000.0), e + 600.0),
            60.0,
        ))
        .to(Waypoint::air(at(add(p.pos, bin, 1500.0), e + 180.0), 32.0))
        .to(Waypoint::fixed(at(p.pos, e + 30.0), 3.0))
        .to(Waypoint::ground(at(p.pos, e), 1.5).hold(d.range(480.0, 900.0)))
        .to(Waypoint::fixed(at(p.pos, e + 40.0), 4.0))
        .to(Waypoint::air(at(add(p.pos, bout, 1500.0), e + 250.0), 40.0))
        .to(Waypoint::air(
            at(add(p.pos, bout, 30_000.0), e + 600.0),
            62.0,
        ));
        b.build(Some(&self.relief), 200.0, 0.12)
    }

    /// A round trip from a heliport over three points 6 – 15 km out.
    fn tour(&self, p: Pad, d: &mut Draw) -> FlightPath {
        let e = p.elevation_m;
        let mut b = PathBuilder::new(500.0);
        b.to(Waypoint::ground(at(p.pos, e), 1.5).hold(d.range(30.0, 120.0)))
            .to(Waypoint::fixed(at(p.pos, e + 40.0), 4.0));
        let mut bearing = d.range(0.0, 360.0);
        for _ in 0..3 {
            bearing += d.range(60.0, 140.0);
            b.to(Waypoint::air(
                at(
                    add(p.pos, heading_vec(bearing), d.range(6000.0, 15_000.0)),
                    e + d.range(350.0, 800.0),
                ),
                55.0,
            ));
        }
        let back = heading_vec(bearing + 180.0);
        b.to(Waypoint::air(
            at(add(p.pos, back, -1200.0), e + 150.0),
            30.0,
        ))
        .to(Waypoint::fixed(at(p.pos, e + 30.0), 3.0))
        .to(Waypoint::ground(at(p.pos, e), 1.5));
        b.build(Some(&self.relief), 200.0, 0.12)
    }

    /// A winch launch, three to six thermals (circling up toward the cloud
    /// base, drifting with the wind), glides between them, and back to
    /// land on the field.
    fn soaring(&self, rw: RunwayUse, strength: f64, w: &WeatherState, d: &mut Draw) -> FlightPath {
        let u = rw.dir();
        let e = rw.elevation_m;
        let thr = rw.threshold;
        let top = e + f64::from(w.cloud_base_m).clamp(800.0, 2500.0) - 150.0;
        let climb = 1.0 + 2.0 * strength;
        let up = UpperAir::from_weather(w, e);
        let mut b = PathBuilder::new(110.0);
        b.to(Waypoint::ground(at(thr, e), 0.0).hold(d.range(20.0, 120.0)))
            .to(Waypoint::ground(
                at(add(thr, u, 150.0), rw.ground_at(150.0)),
                22.0,
            ))
            .to(Waypoint::fixed(at(add(thr, u, 700.0), e + 380.0), 27.0));
        let mut pos = add(thr, u, 700.0);
        let mut alt = e + 380.0;
        let thermals = 3 + d.pick(4);
        for _ in 0..thermals {
            let c = add(
                rw.threshold,
                heading_vec(d.range(0.0, 360.0)),
                d.range(1500.0, 11_000.0),
            );
            let glide = dist2(c, pos);
            alt = (alt - glide / 32.0).max(e + 250.0);
            b.to(Waypoint::air(at(c, alt), 30.0));
            let r = d.range(90.0, 130.0);
            let turn = if d.f() < 0.5 { 1.0 } else { -1.0 };
            let lap_s = TAU * r / 26.0;
            let mut a0 = d.range(0.0, TAU);
            let mut centre = c;
            let wind = up.wind_at(alt);
            let leave = top.min(alt + d.range(400.0, 800.0));
            while alt < leave {
                for _ in 0..12 {
                    a0 += turn * TAU / 12.0;
                    alt = (alt + climb * lap_s / 12.0).min(leave);
                    centre = add(centre, wind, lap_s / 12.0);
                    b.to(Waypoint::air(
                        at([centre[0] + r * a0.cos(), centre[1] + r * a0.sin()], alt),
                        26.0,
                    ));
                }
            }
            pos = centre;
        }
        // Home: a straight glide, then the final.
        b.to(Waypoint::air(at(add(thr, u, -1800.0), e + 180.0), 30.0))
            .to(Waypoint::fixed(at(add(thr, u, -600.0), e + 60.0), 27.0))
            .to(Waypoint::fixed(at(thr, e + 4.0), 25.0))
            .to(Waypoint::ground(
                at(add(thr, u, 220.0), rw.ground_at(220.0)),
                4.0,
            ));
        b.build(Some(&self.relief), 120.0, 0.1)
    }

    /// A balloon: inflated and standing, lifting off, drifting with the
    /// wind for an hour or so between 300 and 900 m over the ground, and
    /// down again.
    fn drift(&self, site: [f64; 3], w: &WeatherState, d: &mut Draw) -> FlightPath {
        let e = site[1];
        let up = UpperAir::from_weather(w, e);
        let mut b = PathBuilder::new(50.0);
        let mut p = [site[0], site[2]];
        b.to(Waypoint::ground(at(p, e), 0.4).hold(d.range(300.0, 600.0)));
        let cruise = e + d.range(300.0, 900.0);
        let legs = 10 + d.pick(10);
        let mut alt = e + 60.0;
        b.to(Waypoint::air(at(p, alt), 1.5));
        for i in 0..legs {
            let a = if i + 2 >= legs {
                e + 120.0
            } else {
                cruise + d.range(-80.0, 80.0)
            };
            let wind = up.wind_at(alt);
            let v = (wind[0].hypot(wind[1]) * 1.4 + 0.6).max(1.0);
            let dir = if wind[0].hypot(wind[1]) > 0.2 {
                norm(wind)
            } else {
                heading_vec(d.range(0.0, 360.0))
            };
            p = add(p, dir, v * 240.0);
            alt = a;
            b.to(Waypoint::air(at(p, alt), v));
        }
        // Down on the nearest open ground (the landing grid), else back
        // where it rose.
        let land = self
            .landings
            .iter()
            .min_by(|a, b| dist2([a[0], a[2]], p).total_cmp(&dist2([b[0], b[2]], p)))
            .copied()
            .unwrap_or(site);
        b.to(Waypoint::fixed([land[0], land[1] + 40.0, land[2]], 1.0))
            .to(Waypoint::ground(land, 0.6).hold(d.range(60.0, 180.0)));
        b.build(Some(&self.relief), 120.0, 0.05)
    }
}

fn norm(d: [f64; 2]) -> [f64; 2] {
    let l = d[0].hypot(d[1]);
    if l < 1e-9 {
        [0.0, -1.0]
    } else {
        [d[0] / l, d[1] / l]
    }
}

fn segment_dist(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let d = [b[0] - a[0], b[1] - a[1]];
    let l2 = d[0] * d[0] + d[1] * d[1];
    let t = if l2 > 0.0 {
        (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / l2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    dist2(p, add(a, d, t))
}

/// The sky of a session on one client: the flights of the slots around
/// now, kept while they fly (a flight is built once, from its slot's
/// hash and the weather when it is first seen).
#[derive(Debug, Default)]
pub struct AirSky {
    cache: HashMap<(u32, i64), Option<Arc<Flight>>>,
    seed: Option<u64>,
    calls: u64,
}

impl AirSky {
    /// Every flight alive at `inp.now_s` near the viewer (not limited by
    /// the budget).
    pub fn flights(&mut self, space: &AirSpace, inp: &AirInputs) -> Vec<Arc<Flight>> {
        if self.seed != Some(inp.seed) {
            self.cache.clear();
            self.seed = Some(inp.seed);
        }
        let now = inp.now_s;
        let viewer = [inp.viewer[0], inp.viewer[2]];
        let mut out = Vec::new();
        for g in &space.gens {
            if !space.near(g, viewer, inp.radius_m) {
                continue;
            }
            let k0 = ((now - g.max_s - g.period_s) / g.period_s).floor() as i64;
            // Arrivals start up to a flight before their slot.
            let lead = if matches!(g.what, GenKind::Arrival { .. }) {
                g.max_s
            } else {
                0.0
            };
            let k1 = ((now + lead) / g.period_s).floor() as i64;
            for k in k0..=k1 {
                let f = self
                    .cache
                    .entry((g.id, k))
                    .or_insert_with(|| space.flight(g, k, inp).map(Arc::new));
                if let Some(f) = f
                    && f.alive(now)
                {
                    out.push(f.clone());
                }
            }
        }
        // Forget the slots that are over.
        self.calls += 1;
        if self.cache.len() > 4096 || self.calls.is_multiple_of(120) {
            let gens = &space.gens;
            self.cache.retain(|(id, k), f| {
                let g = gens.get(*id as usize);
                g.is_some_and(|g| {
                    (*k as f64 + 1.0) * g.period_s + g.max_s + g.period_s > now
                        && f.as_ref().is_none_or(|f| f.end_s() + g.period_s > now)
                })
            });
        }
        out
    }

    /// The aircraft now: the nearest within the budget, with their
    /// contrails.
    pub fn now(&mut self, space: &AirSpace, inp: &AirInputs) -> Vec<AircraftNow> {
        let now = inp.now_s;
        let v = inp.viewer;
        let mut fl: Vec<(f64, Arc<Flight>)> = self
            .flights(space, inp)
            .into_iter()
            .map(|f| {
                let p = f.state(now).pos;
                ((p[0] - v[0]).hypot(p[2] - v[2]), f)
            })
            .filter(|(d, _)| *d <= inp.radius_m)
            .collect();
        fl.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.id.cmp(&b.1.id)));
        fl.truncate(inp.max_aircraft);
        let upper = UpperAir::from_weather(inp.weather, inp.ground_m);
        fl.into_iter()
            .map(|(_, f)| aircraft_now(&f, now, &upper, inp.contrails))
            .collect()
    }

    /// The aircraft heard at `listener` now (within [`HEARD_M`]), each from
    /// where the sound now arriving left it.
    pub fn heard(&mut self, space: &AirSpace, inp: &AirInputs, listener: [f64; 3]) -> Vec<Heard> {
        let inp = AirInputs {
            viewer: listener,
            radius_m: HEARD_M + 2000.0,
            ..*inp
        };
        let mut out: Vec<Heard> = self
            .flights(space, &inp)
            .iter()
            .filter_map(|f| heard(f, inp.now_s, listener))
            .collect();
        out.sort_by(|a, b| a.distance_m.total_cmp(&b.distance_m));
        out
    }
}

/// The sound of `f` heard at `listener` at `t`: the emission time `te`
/// with `|p(te) − listener| = c (t − te)` (a few fixed-point steps; the
/// aircraft is far slower than sound).
pub fn heard(f: &Flight, t: f64, listener: [f64; 3]) -> Option<Heard> {
    let dist = |p: [f64; 3]| {
        ((p[0] - listener[0]).powi(2) + (p[1] - listener[1]).powi(2) + (p[2] - listener[2]).powi(2))
            .sqrt()
    };
    let mut te = t;
    for _ in 0..24 {
        let next = t - dist(f.state(te).pos) / SPEED_OF_SOUND_MPS;
        let done = (next - te).abs() < 1e-5;
        te = next;
        if done {
            break;
        }
    }
    if !f.alive(te) {
        return None;
    }
    let s = f.state(te);
    let d = dist(s.pos);
    (d <= HEARD_M).then(|| {
        let a = aircraft_motion(f, te, &s);
        Heard {
            id: f.id,
            kind: f.kind,
            pos: s.pos,
            vel: s.vel,
            distance_m: d,
            delay_s: t - te,
            on_ground: s.ground,
            rotor: a.0,
            burner: a.1,
        }
    })
}

/// The propeller / rotor speed and the burner at `t`.
fn aircraft_motion(f: &Flight, t: f64, s: &PathState) -> (f64, bool) {
    let speed = (s.vel[0].powi(2) + s.vel[1].powi(2) + s.vel[2].powi(2)).sqrt();
    let rotor = match f.kind {
        AirKind::Glider | AirKind::Balloon => 0.0,
        // Idling on the pad.
        AirKind::Helicopter if s.ground && speed < 0.5 => 0.3,
        AirKind::SmallPlane if s.ground && speed < 0.5 => 0.4,
        _ => 1.0,
    };
    let burner = f.kind == AirKind::Balloon && {
        let slot = (t / 6.0).floor() as i64;
        let climbing = s.vel[1] > 0.3;
        mix(f.id ^ slot as u64).is_multiple_of(if climbing { 2 } else { 4 })
            && (t / 6.0).fract() < 0.5
    };
    (rotor, burner)
}

fn aircraft_now(f: &Flight, now: f64, upper: &UpperAir, contrails: bool) -> AircraftNow {
    let s = f.state(now);
    let (rotor, burner) = aircraft_motion(f, now, &s);
    let above = s.pos[1] - f.field_m;
    let gear_down = match (f.kind, f.mission) {
        (AirKind::Airliner, Mission::Arrival) => above < 1100.0,
        (AirKind::Airliner, Mission::Departure) => above < 350.0,
        (AirKind::Airliner, _) => false,
        _ => true,
    } || s.ground;
    let trail = (contrails && f.kind == AirKind::Airliner)
        .then(|| trail(f, now, upper))
        .flatten();
    AircraftNow {
        id: f.id,
        kind: f.kind,
        mission: f.mission,
        pos: s.pos,
        vel: s.vel,
        bank: s.bank,
        on_ground: s.ground,
        gear_down,
        rotor,
        burner,
        look: f.look,
        trail,
    }
}

/// Ages of the trail's points (s): dense at the engines, sparse far back.
const TRAIL_AGES: [f64; 18] = [
    0.3, 1.0, 2.0, 4.0, 7.0, 11.0, 16.0, 22.0, 30.0, 45.0, 65.0, 90.0, 130.0, 180.0, 250.0, 340.0,
    450.0, 600.0,
];

/// The contrail behind `f` at `now`: where it was, drifted by the wind
/// aloft since; it ends where the air did not make a trail.
pub fn trail(f: &Flight, now: f64, upper: &UpperAir) -> Option<Trail> {
    let here = f.state(now).pos;
    let kind = upper.contrail(here[1]);
    if kind == Contrail::None {
        return None;
    }
    let life = kind.lifetime_s();
    let wind = upper.wind_at(here[1]);
    let mut points = Vec::with_capacity(TRAIL_AGES.len());
    for age in TRAIL_AGES {
        let age = age.min(life);
        let t = now - age;
        if t < f.start_s {
            break;
        }
        let p = f.state(t).pos;
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
