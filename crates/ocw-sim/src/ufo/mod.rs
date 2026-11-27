//! UFOs (spec 14 Phase 18, 18C *UFOs*, 18E T9; Q7 (a) – (c)): a fun
//! extra that fits the unicorn of Phase 13 – flying saucers seen now and
//! then on clear nights, on *often* also at dusk and by day high in the
//! sky, scenes in the fields (a crop circle that stays, a cow lifted by a
//! beam and set down again a field away, unharmed) and, in god mode, a
//! curious saucer that comes to look at the unicorn ([`curiosity`]).
//!
//! **The same for every rider who has them on, without network traffic**
//! (18A, Q2: a rider's own view setting): like the aircraft of
//! [`crate::air`], the sightings come from the session's air seed and the
//! world clock alone. The world is cut into cells of [`CELL_M`] and time
//! into slots of [`SLOT_S`]; whether a cell's slot has a sighting is an
//! integer hash of the seed, the cell and the slot against the setting's
//! rate ([`rates`]), the light of the hour (the sun at the world's origin)
//! and the weather (clear nights, [`clear`]); where it is and what it does
//! comes from the same hash and the ground ([`UfoGround`]). So riders at
//! the same clock see the same saucers at the same places.
//!
//! **Never touching a rider:** riders ride the roads, so a hovering saucer
//! stays [`HOVER_ROAD_M`] from every road and [`HOVER_MIN_M`] over the
//! ground; the scenes keep [`SCENE_ROAD_M`] from the roads (the cow's
//! whole way too), the saucer [`SCENE_HEIGHT_M`] over the field; arrivals
//! come down from straight above and departures climb steeply away.
//! Nothing here reaches the server, a race, an event or a recording (the
//! client draws them only outside events: `ViewChoice::ufos_shown`).
//!
//! Pure and wasm-safe: no clocks, threads or files; callers pass the
//! instant.

pub mod curiosity;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashMap};
use std::f64::consts::TAU;
use std::sync::Arc;

use ocw_core::traffic::Ufos;
use ocw_weather::state::WeatherState;

use crate::air::mix;

/// The side of a cell (m).
pub const CELL_M: f64 = 3000.0;
/// One sighting chance per cell and slot (s).
pub const SLOT_S: f64 = 1800.0;
/// Cells whose centre is this close to the viewer are looked at (m).
pub const VIEW_M: f64 = 6000.0;
/// The longest sighting (s).
pub const LONGEST_S: f64 = 240.0;
/// A hovering saucer's distance from every road (m).
pub const HOVER_ROAD_M: f64 = 150.0;
/// … and its least height over the ground (m).
pub const HOVER_MIN_M: f64 = 60.0;
/// The scenes' distance from every road (m): the crop circle's edge, the
/// cow's way.
pub const SCENE_ROAD_M: f64 = 40.0;
/// The farthest a scene's field is from a road (m): seen from the route.
pub const SCENE_NEAR_M: f64 = 400.0;
/// The saucer's height over the field in a scene (m).
pub const SCENE_HEIGHT_M: f64 = 30.0;
/// How far below the saucer's centre the lifted cow hangs (m).
pub const COW_BELOW_M: f64 = 9.0;
/// The cow grazes where it is set down this long after (s).
pub const COW_STAYS_S: f64 = 3600.0;
/// The cow grazes where it will be lifted this long before (s).
pub const COW_BEFORE_S: f64 = 600.0;
/// Crop circles made this long ago are still found (s); the client keeps
/// every circle it has seen for the session besides.
pub const CIRCLES_KEPT_S: f64 = 12.0 * 3600.0;

/// The light of a sighting's hour by the sun's elevation (°).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Light {
    /// Under −8°.
    Night,
    /// −8° … 2°.
    Dusk,
    Day,
}

impl Light {
    pub fn of_sun(elevation_deg: f64) -> Self {
        if elevation_deg < -8.0 {
            Self::Night
        } else if elevation_deg <= 2.0 {
            Self::Dusk
        } else {
            Self::Day
        }
    }
}

/// The chances of a setting (per cell and slot) and its scenes' shares.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rates {
    pub night: f64,
    pub dusk: f64,
    pub day: f64,
    /// Of the sightings low over the fields: crop circles and cows.
    pub circle: f64,
    pub cow: f64,
}

/// The rates of `traffic.ufo` (18C): *rare* only on clear nights – about
/// one sighting an hour within [`VIEW_M`]; *often* about five times as
/// many at night, also at dusk and by day high in the sky; *off* none.
pub fn rates(level: Ufos) -> Rates {
    match level {
        Ufos::Off => Rates {
            night: 0.0,
            dusk: 0.0,
            day: 0.0,
            circle: 0.0,
            cow: 0.0,
        },
        Ufos::Rare => Rates {
            night: 0.025,
            dusk: 0.0,
            day: 0.0,
            circle: 0.2,
            cow: 0.15,
        },
        Ufos::Often => Rates {
            night: 0.12,
            dusk: 0.08,
            day: 0.05,
            circle: 0.3,
            cow: 0.25,
        },
    }
}

/// Whether the sky is clear enough to see a saucer: at night and dusk few
/// clouds, no rain, no fog; by day (high up) not overcast.
pub fn clear(w: &WeatherState, light: Light) -> bool {
    let dry = w.precip_mm_h < 0.1 && w.lightning_per_min == 0.0 && w.fog_visibility_m > 3000.0;
    match light {
        Light::Night | Light::Dusk => dry && w.cloud_cover < 0.45,
        Light::Day => dry && w.cloud_cover < 0.6,
    }
}

/// What the ground is at a point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Land {
    /// Farmland or a meadow.
    Field,
    Water,
    /// Woods, buildings, rock …
    Other,
    /// No land use known (a package without it): taken as open land.
    Unknown,
}

impl Land {
    fn open(self) -> bool {
        matches!(self, Self::Field | Self::Water | Self::Unknown)
    }
    fn field(self) -> bool {
        matches!(self, Self::Field | Self::Unknown)
    }
}

/// The world as the UFOs need it (the client answers it from the package).
pub trait UfoGround {
    /// The ground (m); `None` outside the world.
    fn height(&self, x: f64, z: f64) -> Option<f64>;
    fn land(&self, x: f64, z: f64) -> Land;
    /// The distance to the nearest road (m), or `within` when none is
    /// nearer.
    fn road_m(&self, x: f64, z: f64, within: f64) -> f64;
}

/// What a sighting is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SightingKind {
    /// Hovering low over the fields or a lake far from the road, jumping,
    /// gone (night and dusk).
    Night,
    /// High in the sky by day (*often*).
    Day,
    /// Hovering over a field near the route; a crop circle is left.
    CropCircle,
    /// A cow lifted by the beam, carried a field away and set down.
    Cow,
    /// The unicorn's visitor ([`curiosity`]).
    Visitor,
}

impl SightingKind {
    pub fn key(self) -> &'static str {
        match self {
            Self::Night => "night",
            Self::Day => "day",
            Self::CropCircle => "circle",
            Self::Cow => "cow",
            Self::Visitor => "visitor",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        [Self::Night, Self::Day, Self::CropCircle, Self::Cow]
            .into_iter()
            .find(|k| k.key().starts_with(s) || s.starts_with(k.key()))
    }
}

/// How a piece of the path moves.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Ease {
    /// Hovering at `from` (bobbing).
    Hold,
    /// Gliding (smooth start and stop).
    Smooth,
    /// A sudden jump.
    Dart,
    /// Down from high above (fast, then braking).
    Arrive,
    /// Away: slow at first, then gone at a huge speed.
    Away,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Seg {
    t0: f64,
    t1: f64,
    from: [f64; 3],
    to: [f64; 3],
    ease: Ease,
}

/// A crop circle: rings and discs of flattened crop on a field.
#[derive(Debug, Clone, PartialEq)]
pub struct CropCircle {
    pub id: u64,
    /// The centre (x, z) and the ground there.
    pub centre: [f64; 2],
    pub ground_m: f64,
    /// The pattern: (offset from the centre (x, z), inner radius, outer
    /// radius); a disc has an inner radius of 0.
    pub shapes: Vec<([f64; 2], f64, f64)>,
    /// Made from … to (s): the rings appear while the saucer hovers.
    pub from_s: f64,
    pub to_s: f64,
}

impl CropCircle {
    /// The radius of the whole pattern (m).
    pub fn extent_m(&self) -> f64 {
        self.shapes
            .iter()
            .map(|(o, _, r)| o[0].hypot(o[1]) + r)
            .fold(0.0, f64::max)
    }

    /// How far it is made at `t` (0 … 1).
    pub fn made(&self, t: f64) -> f64 {
        ((t - self.from_s) / (self.to_s - self.from_s).max(1e-6)).clamp(0.0, 1.0)
    }
}

/// The cow of a beam scene.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CowScene {
    /// Where it grazes first and where it is set down (on the ground).
    pub a: [f64; 3],
    pub b: [f64; 3],
    pub yaw: f64,
    /// The lift (up into the beam), the carry, the lowering (s).
    pub lift: (f64, f64),
    pub carry: (f64, f64),
    pub lower: (f64, f64),
}

/// A cow now.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CowNow {
    pub id: u64,
    /// Its feet (m) and heading (rad, about +Y).
    pub pos: [f64; 3],
    pub yaw: f64,
    /// In the beam (off the ground).
    pub lifted: bool,
}

/// One sighting: its whole path, a function of the clock.
#[derive(Debug, Clone, PartialEq)]
pub struct Sighting {
    pub id: u64,
    pub kind: SightingKind,
    pub start_s: f64,
    pub end_s: f64,
    segs: Vec<Seg>,
    /// When the beam is on (s).
    beam: Option<(f64, f64)>,
    pub circle: Option<CropCircle>,
    pub cow: Option<CowScene>,
    /// The bobbing's phase.
    phase: f64,
}

/// How the rim lights shine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lights {
    /// Running round the rim in many colours.
    Cycle,
    /// Flashing in the unicorn's mane colours (the visitor near it).
    Mane,
    /// Off (by day: a silver glint high up).
    Off,
}

/// A saucer now.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UfoNow {
    pub id: u64,
    pub kind: SightingKind,
    /// The centre under the hull (the model's origin) and the velocity (m, m/s).
    pub pos: [f64; 3],
    pub vel: [f64; 3],
    /// The hull's turn (rad).
    pub spin: f64,
    /// Materialising and vanishing: 0 … 1 (drawn smaller when lower).
    pub presence: f32,
    /// The beam's strength 0 … 1 and the ground it reaches (m).
    pub beam: f32,
    pub beam_ground_m: f64,
    pub lights: Lights,
    /// Darting away (the whoosh).
    pub departing: bool,
}

fn smooth(f: f64) -> f64 {
    let f = f.clamp(0.0, 1.0);
    f * f * (3.0 - 2.0 * f)
}

fn lerp3(a: [f64; 3], b: [f64; 3], f: f64) -> [f64; 3] {
    [
        a[0] + (b[0] - a[0]) * f,
        a[1] + (b[1] - a[1]) * f,
        a[2] + (b[2] - a[2]) * f,
    ]
}

/// The time an away-dart takes (s).
pub const AWAY_S: f64 = 3.0;
/// … and how far it goes (m).
const AWAY_M: f64 = 4000.0;

impl Sighting {
    pub fn alive(&self, t: f64) -> bool {
        (self.start_s..self.end_s).contains(&t)
    }

    fn seg(&self, t: f64) -> &Seg {
        self.segs
            .iter()
            .find(|s| t < s.t1)
            .unwrap_or_else(|| self.segs.last().expect("a sighting has a path"))
    }

    /// The saucer's centre at `t`.
    pub fn pos(&self, t: f64) -> [f64; 3] {
        let s = self.seg(t);
        let f = ((t - s.t0) / (s.t1 - s.t0).max(1e-6)).clamp(0.0, 1.0);
        let g = match s.ease {
            Ease::Hold => 0.0,
            Ease::Smooth | Ease::Dart => smooth(f),
            Ease::Arrive => 1.0 - (1.0 - f).powi(3),
            Ease::Away => f.powi(3),
        };
        let mut p = lerp3(s.from, s.to, g);
        if matches!(s.ease, Ease::Hold | Ease::Smooth) {
            // Hovering: a slow bob and sway.
            let w = TAU * 0.3 * t + self.phase;
            p[1] += 1.2 * w.sin();
            p[0] += 0.8 * (0.7 * w).cos();
        }
        p
    }

    /// The saucer at `t` (`None` when it is not there).
    pub fn now(&self, t: f64) -> Option<UfoNow> {
        if !self.alive(t) {
            return None;
        }
        let pos = self.pos(t);
        let (a, b) = (self.pos(t - 0.05), self.pos(t + 0.05));
        let vel = [
            (b[0] - a[0]) * 10.0,
            (b[1] - a[1]) * 10.0,
            (b[2] - a[2]) * 10.0,
        ];
        let seg = self.seg(t);
        let departing = seg.ease == Ease::Away;
        // Materialising over 2.5 s (an arrival from above over 1 s) and
        // fading in the last 0.6 s of the away-dart.
        let fade_in = match self.segs[0].ease {
            Ease::Arrive => 1.0,
            _ => 2.5,
        };
        let presence = ((t - self.start_s) / fade_in)
            .min((self.end_s - t) / 0.6)
            .clamp(0.0, 1.0) as f32;
        let beam = self.beam.map_or(0.0, |(on, off)| {
            ((t - on) / 1.2).min((off - t) / 1.2).clamp(0.0, 1.0)
        }) as f32;
        let spin_rate = if seg.ease == Ease::Dart { 6.0 } else { 0.8 };
        let spin = (t * spin_rate + self.phase).rem_euclid(TAU);
        let beam_ground_m = self
            .cow
            .map(|c| {
                let f = ((t - c.lift.0) / (c.lower.1 - c.lift.0)).clamp(0.0, 1.0);
                c.a[1] + (c.b[1] - c.a[1]) * f
            })
            .or_else(|| self.circle.as_ref().map(|c| c.ground_m))
            .unwrap_or(pos[1] - SCENE_HEIGHT_M);
        Some(UfoNow {
            id: self.id,
            kind: self.kind,
            pos,
            vel,
            spin,
            presence,
            beam,
            beam_ground_m,
            lights: if self.kind == SightingKind::Day {
                Lights::Off
            } else {
                Lights::Cycle
            },
            departing,
        })
    }

    /// The cow of a beam scene at `t` (grazing before and after).
    pub fn cow_now(&self, t: f64) -> Option<CowNow> {
        let c = self.cow?;
        if t < self.start_s - COW_BEFORE_S || t >= self.end_s + COW_STAYS_S {
            return None;
        }
        let (pos, lifted) = if t < c.lift.0 {
            (c.a, false)
        } else if t < c.lift.1 {
            let f = smooth((t - c.lift.0) / (c.lift.1 - c.lift.0));
            let u = self.pos(t);
            (lerp3(c.a, [c.a[0], u[1] - COW_BELOW_M, c.a[2]], f), true)
        } else if t < c.carry.1 {
            let u = self.pos(t);
            ([u[0], u[1] - COW_BELOW_M, u[2]], true)
        } else if t < c.lower.1 {
            let f = smooth((t - c.lower.0) / (c.lower.1 - c.lower.0));
            let u = self.pos(t);
            (lerp3([u[0], u[1] - COW_BELOW_M, u[2]], c.b, f), f < 1.0)
        } else {
            (c.b, false)
        };
        // Turning slowly in the beam, set down facing a new way.
        let yaw = if t < c.lift.0 {
            c.yaw
        } else if t < c.lower.1 {
            c.yaw + 0.35 * (t - c.lift.0)
        } else {
            c.yaw + 0.35 * (c.lower.1 - c.lift.0)
        };
        Some(CowNow {
            id: self.id,
            pos,
            yaw,
            lifted,
        })
    }

    /// Every point the saucer, its beam and its cow take, every `dt` s
    /// (the tests' check that nothing comes near a road).
    pub fn samples(&self, dt: f64) -> Vec<[f64; 3]> {
        let mut out = Vec::new();
        let mut t = self.start_s;
        while t < self.end_s {
            out.push(self.pos(t));
            if let Some(c) = self.cow_now(t) {
                out.push(c.pos);
            }
            t += dt;
        }
        out
    }
}

/// A stream of uniform numbers from one hash.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Draw(u64);

impl Draw {
    pub(crate) fn new(h: u64) -> Self {
        Self(h)
    }
    pub(crate) fn f(&mut self) -> f64 {
        self.0 = mix(self.0);
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
    pub(crate) fn range(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.f()
    }
}

/// The hash of a cell's slot.
fn slot_hash(seed: u64, cell: (i32, i32), slot: i64) -> u64 {
    let c = (u64::from(cell.0 as u32) << 32) | u64::from(cell.1 as u32);
    mix(seed ^ mix(c ^ 0x00F0_5A0C) ^ mix(slot as u64 ^ 0x0005_1067))
}

/// What a sighting needs to be built.
#[derive(Debug, Clone, Copy)]
pub struct UfoInputs<'a> {
    pub seed: u64,
    pub now_s: f64,
    pub level: Ufos,
    pub weather: &'a WeatherState,
    /// The world's origin (lat, lon): the sun of the slots.
    pub origin: (f64, f64),
    /// The camera (m).
    pub viewer: [f64; 3],
}

/// The sun's elevation at `t` (s) over `origin`.
pub fn sun_deg(t: f64, origin: (f64, f64)) -> f64 {
    ocw_sky::sun::position((t * 1000.0) as i64, origin.0, origin.1).elevation_deg
}

/// The sighting of `cell`'s slot `slot`, if it has one.
pub fn sighting(
    ground: &dyn UfoGround,
    inp: &UfoInputs,
    cell: (i32, i32),
    slot: i64,
) -> Option<Sighting> {
    let rates = rates(inp.level);
    let h = slot_hash(inp.seed, cell, slot);
    let mut d = Draw::new(h);
    let start = slot as f64 * SLOT_S + d.range(0.0, SLOT_S - LONGEST_S);
    let light = Light::of_sun(sun_deg(start, inp.origin));
    let p = match light {
        Light::Night => rates.night,
        Light::Dusk => rates.dusk,
        Light::Day => rates.day,
    };
    // One number for all settings: *often*'s sightings include *rare*'s.
    if d.f() >= p || !clear(inp.weather, light) {
        return None;
    }
    let roll = d.f();
    let kind = match light {
        Light::Day => SightingKind::Day,
        _ if roll < rates.circle => SightingKind::CropCircle,
        _ if roll < rates.circle + rates.cow => SightingKind::Cow,
        _ => SightingKind::Night,
    };
    let base = [f64::from(cell.0) * CELL_M, f64::from(cell.1) * CELL_M];
    let b = Builder {
        ground,
        id: h,
        start,
        base,
        phase: d.range(0.0, TAU),
    };
    match kind {
        SightingKind::CropCircle => b.crop_circle(&mut d).or_else(|| b.night(&mut d)),
        SightingKind::Cow => b.cow(&mut d).or_else(|| b.night(&mut d)),
        SightingKind::Day => b.day(&mut d),
        _ => b.night(&mut d),
    }
}

struct Builder<'a> {
    ground: &'a dyn UfoGround,
    id: u64,
    start: f64,
    base: [f64; 2],
    phase: f64,
}

impl Builder<'_> {
    /// A point of the cell.
    fn point(&self, d: &mut Draw) -> [f64; 2] {
        [
            self.base[0] + d.range(0.05, 0.95) * CELL_M,
            self.base[1] + d.range(0.05, 0.95) * CELL_M,
        ]
    }

    /// Somewhere to hover low: open land or water far from the roads.
    fn hover_site(&self, d: &mut Draw, near: Option<([f64; 2], f64, f64)>) -> Option<[f64; 3]> {
        for _ in 0..12 {
            let p = match near {
                Some((c, r0, r1)) => {
                    let a = d.range(0.0, TAU);
                    let r = d.range(r0, r1);
                    [c[0] + r * a.cos(), c[1] + r * a.sin()]
                }
                None => self.point(d),
            };
            let Some(g) = self.ground.height(p[0], p[1]) else {
                continue;
            };
            if self.ground.land(p[0], p[1]).open()
                && self.ground.road_m(p[0], p[1], HOVER_ROAD_M) >= HOVER_ROAD_M
            {
                return Some([p[0], g + d.range(HOVER_MIN_M, 180.0), p[1]]);
            }
        }
        None
    }

    fn away(&self, d: &mut Draw, from: [f64; 3], t: f64) -> Seg {
        let a = d.range(0.0, TAU);
        // Steeply up and away: over everything on the ground.
        let dir = [a.cos() * 0.55, 0.835, a.sin() * 0.55];
        Seg {
            t0: t,
            t1: t + AWAY_S,
            from,
            to: [
                from[0] + dir[0] * AWAY_M,
                from[1] + dir[1] * AWAY_M,
                from[2] + dir[2] * AWAY_M,
            ],
            ease: Ease::Away,
        }
    }

    fn finish(&self, kind: SightingKind, segs: Vec<Seg>) -> Sighting {
        Sighting {
            id: self.id,
            kind,
            start_s: self.start,
            end_s: segs.last().map_or(self.start, |s| s.t1),
            segs,
            beam: None,
            circle: None,
            cow: None,
            phase: self.phase,
        }
    }

    /// Hovering, a few sudden jumps, gone.
    fn night(&self, d: &mut Draw) -> Option<Sighting> {
        let mut at = self.hover_site(d, None)?;
        let mut t = self.start;
        let mut segs = Vec::new();
        let jumps = 1 + (d.f() * 3.0) as usize;
        for k in 0..=jumps {
            let hold = d.range(8.0, 25.0) + if k == 0 { 6.0 } else { 0.0 };
            segs.push(Seg {
                t0: t,
                t1: t + hold,
                from: at,
                to: at,
                ease: Ease::Hold,
            });
            t += hold;
            if k == jumps {
                break;
            }
            if let Some(next) = self.hover_site(d, Some(([at[0], at[2]], 150.0, 500.0))) {
                segs.push(Seg {
                    t0: t,
                    t1: t + 0.5,
                    from: at,
                    to: next,
                    ease: Ease::Dart,
                });
                t += 0.5;
                at = next;
            }
        }
        segs.push(self.away(d, at, t));
        Some(self.finish(SightingKind::Night, segs))
    }

    /// High in the sky by day: a silver glint that jumps and is gone.
    fn day(&self, d: &mut Draw) -> Option<Sighting> {
        let p = self.point(d);
        let g = self.ground.height(p[0], p[1])?;
        let mut at = [p[0], g + d.range(1500.0, 3500.0), p[1]];
        let mut t = self.start;
        let mut segs = Vec::new();
        let jumps = 1 + (d.f() * 3.0) as usize;
        for k in 0..=jumps {
            let hold = d.range(15.0, 40.0);
            segs.push(Seg {
                t0: t,
                t1: t + hold,
                from: at,
                to: at,
                ease: Ease::Hold,
            });
            t += hold;
            if k == jumps {
                break;
            }
            let a = d.range(0.0, TAU);
            let r = d.range(800.0, 2000.0);
            let next = [
                at[0] + r * a.cos(),
                (at[1] + d.range(-400.0, 400.0)).max(g + 1200.0),
                at[2] + r * a.sin(),
            ];
            segs.push(Seg {
                t0: t,
                t1: t + 0.8,
                from: at,
                to: next,
                ease: Ease::Dart,
            });
            t += 0.8;
            at = next;
        }
        segs.push(self.away(d, at, t));
        Some(self.finish(SightingKind::Day, segs))
    }

    /// A field near a road: its ground and the road's distance.
    fn field_site(&self, d: &mut Draw, edge_m: f64) -> Option<[f64; 3]> {
        for _ in 0..16 {
            let p = self.point(d);
            let Some(g) = self.ground.height(p[0], p[1]) else {
                continue;
            };
            if !self.ground.land(p[0], p[1]).field() {
                continue;
            }
            let road = self.ground.road_m(p[0], p[1], SCENE_NEAR_M);
            if road >= edge_m + SCENE_ROAD_M && road < SCENE_NEAR_M {
                return Some([p[0], g, p[1]]);
            }
        }
        None
    }

    /// Down from above, hovering low over a field while the rings form,
    /// away: the circle stays.
    fn crop_circle(&self, d: &mut Draw) -> Option<Sighting> {
        let r0 = d.range(7.0, 13.0);
        let design = (d.f() * 4.0) as usize;
        let mut shapes = vec![([0.0, 0.0], 0.0, r0)];
        let rot = d.range(0.0, TAU);
        let at = |a: f64, r: f64| [r * (a + rot).cos(), r * (a + rot).sin()];
        match design {
            0 => shapes.push(([0.0, 0.0], r0 + 3.0, r0 + 4.2)),
            1 => {
                shapes.push(([0.0, 0.0], r0 * 1.9, r0 * 1.9 + 1.0));
                for k in 0..3 {
                    shapes.push((at(k as f64 * TAU / 3.0, r0 * 1.9), 0.0, r0 * 0.35));
                }
            }
            2 => {
                for (k, w) in [(1.5, 1.0), (2.2, 0.8)] {
                    shapes.push(([0.0, 0.0], r0 * k, r0 * k + w));
                }
            }
            _ => {
                for k in 0..6 {
                    shapes.push((at(k as f64 * TAU / 6.0, r0 * 1.7), 0.0, r0 * 0.3));
                }
            }
        }
        let mut circle = CropCircle {
            id: self.id,
            centre: [0.0, 0.0],
            ground_m: 0.0,
            shapes,
            from_s: 0.0,
            to_s: 0.0,
        };
        let site = self.field_site(d, circle.extent_m())?;
        let over = [site[0], site[1] + SCENE_HEIGHT_M, site[2]];
        let high = [site[0], site[1] + 900.0, site[2]];
        let t = self.start;
        let hold = d.range(45.0, 70.0);
        let segs = vec![
            Seg {
                t0: t,
                t1: t + 3.0,
                from: high,
                to: over,
                ease: Ease::Arrive,
            },
            Seg {
                t0: t + 3.0,
                t1: t + 3.0 + hold,
                from: over,
                to: over,
                ease: Ease::Hold,
            },
        ];
        circle.centre = [site[0], site[2]];
        circle.ground_m = site[1];
        circle.from_s = t + 8.0;
        circle.to_s = t + hold;
        let mut segs = segs;
        segs.push(self.away(d, over, t + 3.0 + hold));
        let mut s = self.finish(SightingKind::CropCircle, segs);
        s.beam = Some((t + 6.0, t + hold + 1.0));
        s.circle = Some(circle);
        Some(s)
    }

    /// The cow: lifted in the beam, carried a field away, set down.
    fn cow(&self, d: &mut Draw) -> Option<Sighting> {
        let a = self.field_site(d, 20.0)?;
        // The next field: 250 – 400 m on, and the whole way off the roads.
        let mut b = None;
        for _ in 0..10 {
            let ang = d.range(0.0, TAU);
            let r = d.range(250.0, 400.0);
            let p = [a[0] + r * ang.cos(), a[2] + r * ang.sin()];
            let Some(g) = self.ground.height(p[0], p[1]) else {
                continue;
            };
            let clear_way = (0..=10).all(|k| {
                let f = f64::from(k) / 10.0;
                let q = [a[0] + (p[0] - a[0]) * f, a[2] + (p[1] - a[2]) * f];
                self.ground.road_m(q[0], q[1], SCENE_ROAD_M + 20.0) >= SCENE_ROAD_M + 20.0
            });
            if clear_way && self.ground.land(p[0], p[1]).field() {
                b = Some([p[0], g, p[1]]);
                break;
            }
        }
        let b = b?;
        let t = self.start;
        let over_a = [a[0], a[1] + SCENE_HEIGHT_M, a[2]];
        let over_b = [b[0], b[1] + SCENE_HEIGHT_M, b[2]];
        let high = [a[0], a[1] + 900.0, a[2]];
        let lift = (t + 9.0, t + 23.0);
        let carry = (lift.1 + 1.0, lift.1 + 19.0);
        let lower = (carry.1 + 1.0, carry.1 + 13.0);
        let gone = lower.1 + 4.0;
        let segs = vec![
            Seg {
                t0: t,
                t1: t + 3.0,
                from: high,
                to: over_a,
                ease: Ease::Arrive,
            },
            Seg {
                t0: t + 3.0,
                t1: carry.0,
                from: over_a,
                to: over_a,
                ease: Ease::Hold,
            },
            Seg {
                t0: carry.0,
                t1: carry.1,
                from: over_a,
                to: over_b,
                ease: Ease::Smooth,
            },
            Seg {
                t0: carry.1,
                t1: gone,
                from: over_b,
                to: over_b,
                ease: Ease::Hold,
            },
        ];
        let mut segs = segs;
        segs.push(self.away(d, over_b, gone));
        let mut s = self.finish(SightingKind::Cow, segs);
        s.beam = Some((lift.0 - 2.0, lower.1 + 1.5));
        s.cow = Some(CowScene {
            a,
            b,
            yaw: d.range(0.0, TAU),
            lift,
            carry,
            lower,
        });
        Some(s)
    }
}

/// The cells around `viewer` (within [`VIEW_M`] of their nearest point).
pub fn cells_near(viewer: [f64; 3]) -> Vec<(i32, i32)> {
    let r = VIEW_M;
    let (x0, x1) = (
        ((viewer[0] - r) / CELL_M).floor() as i32,
        ((viewer[0] + r) / CELL_M).floor() as i32,
    );
    let (z0, z1) = (
        ((viewer[2] - r) / CELL_M).floor() as i32,
        ((viewer[2] + r) / CELL_M).floor() as i32,
    );
    let mut out = Vec::new();
    for cx in x0..=x1 {
        for cz in z0..=z1 {
            let nx = viewer[0].clamp(f64::from(cx) * CELL_M, f64::from(cx + 1) * CELL_M);
            let nz = viewer[2].clamp(f64::from(cz) * CELL_M, f64::from(cz + 1) * CELL_M);
            if (nx - viewer[0]).hypot(nz - viewer[2]) <= r {
                out.push((cx, cz));
            }
        }
    }
    out
}

/// What the UFOs are now.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UfoScene {
    pub ufos: Vec<UfoNow>,
    pub cows: Vec<CowNow>,
    pub circles: Vec<CropCircle>,
}

/// A cell and a slot.
type SlotKey = ((i32, i32), i64);

/// The UFOs of a session on one client: the sightings of the slots around
/// now (built once each, from its hash and the weather when first looked
/// at), and every crop circle seen – they stay for the session.
#[derive(Debug, Default)]
pub struct UfoSky {
    cache: HashMap<SlotKey, Option<Arc<Sighting>>>,
    key: Option<(u64, Ufos)>,
    circles: BTreeMap<u64, CropCircle>,
    calls: u64,
}

impl UfoSky {
    /// The sightings of the cells near the viewer that may be seen now
    /// (`back_s`: also those that ended up to this long ago).
    pub fn sightings(
        &mut self,
        ground: &dyn UfoGround,
        inp: &UfoInputs,
        back_s: f64,
    ) -> Vec<Arc<Sighting>> {
        if self.key != Some((inp.seed, inp.level)) {
            self.cache.clear();
            self.circles.clear();
            self.key = Some((inp.seed, inp.level));
        }
        if inp.level == Ufos::Off {
            return Vec::new();
        }
        let now = inp.now_s;
        let k0 = ((now - back_s - SLOT_S) / SLOT_S).floor() as i64;
        let k1 = (now / SLOT_S).floor() as i64;
        let mut out = Vec::new();
        for cell in cells_near(inp.viewer) {
            for k in k0..=k1 {
                let s = self
                    .cache
                    .entry((cell, k))
                    .or_insert_with(|| sighting(ground, inp, cell, k).map(Arc::new));
                if let Some(s) = s
                    && s.start_s <= now + COW_BEFORE_S
                    && s.end_s + back_s > now
                {
                    out.push(s.clone());
                }
            }
        }
        self.calls += 1;
        if self.calls.is_multiple_of(300) || self.cache.len() > 20_000 {
            let keep = ((now - CIRCLES_KEPT_S - 2.0 * SLOT_S) / SLOT_S).floor() as i64;
            self.cache.retain(|(_, k), _| *k >= keep);
        }
        out.sort_by(|a, b| a.start_s.total_cmp(&b.start_s).then(a.id.cmp(&b.id)));
        out
    }

    /// The saucers, the cows and the crop circles now.
    pub fn now(&mut self, ground: &dyn UfoGround, inp: &UfoInputs) -> UfoScene {
        let now = inp.now_s;
        let mut scene = UfoScene::default();
        for s in self.sightings(ground, inp, CIRCLES_KEPT_S) {
            if let Some(u) = s.now(now) {
                scene.ufos.push(u);
            }
            if let Some(c) = s.cow_now(now) {
                scene.cows.push(c);
            }
            if let Some(c) = &s.circle
                && c.from_s <= now
            {
                self.circles.entry(c.id).or_insert_with(|| c.clone());
            }
        }
        // Every circle seen this session, near enough to draw.
        let v = inp.viewer;
        scene.circles = self
            .circles
            .values()
            .filter(|c| (c.centre[0] - v[0]).hypot(c.centre[1] - v[2]) < VIEW_M + CELL_M)
            .cloned()
            .collect();
        scene
    }

    /// The crop circles seen so far this session.
    pub fn circles_seen(&self) -> usize {
        self.circles.len()
    }
}
