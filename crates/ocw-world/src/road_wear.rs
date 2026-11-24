//! Road damage (spec §4.6 step 6, *Road damage*): cracks, crack sealing,
//! alligator areas, repaired patches, worn wheel tracks, potholes, edge
//! breaks and loose stones on the carriageway, drawn by `road.wesl` from
//! the strip's **wear grade** and the strip frame (across, along in metres)
//! – no textures, no geometry: the road surface riders stand on is not
//! changed (`Track::surface_height` and the road meshes' positions are the
//! same for every grade).
//!
//! **The grade (1 … 7, 0: none drawn) comes from OSM `smoothness`** where
//! the road has it ([`smoothness_grade`]; section `RCND`,
//! [`crate::road_condition`]): excellent almost no damage, good a few fine
//! cracks, intermediate cracks, sealing lines, patches and the odd small
//! pothole, bad many cracks, alligator areas, potholes, broken edges and
//! loose stones, very bad and worse heavy damage. **Without the tag it is
//! an estimate** ([`estimate_grade`]): from `tracktype` on tracks, else from
//! the road class and the surface – cleaner and plainer than the tagged
//! roads (spec Phase 8 Q14 (b), v5.35: main roads excellent, the other
//! paved classes good, tracks a little worse). Each grade has its chances
//! ([`wear_density`], scaled by the quality preset's share); main roads
//! keep their potholes within [`MAIN_ROAD_ZONE_M`] of the edge. Where a
//! feature lies is a hash of the cell along the road and of the road's
//! **seed** ([`crate::road_dress::road_seed`], from its edge's length, so
//! every road has its own damage), so it is the same on every machine, in
//! both apps and across tile borders (the along coordinate runs on over
//! the tiles and through a junction patch on the main road).
//!
//! **Variety (v5.68):** each pothole has its own outline (an ellipse of
//! its aspect and angle warped by value noise of its own strength and
//! frequency; a third long along the wheel's way, edge breaks long along
//! the edge), size (10 – 60 cm radius, the largest by grade), depth
//! (1 – 8 cm, deeper on worse roads and for larger holes), may have one or
//! two smaller holes beside it (a cluster, sometimes merged) or be filled
//! with fresh tarmac (more on the better roads; a filled hole is smooth).
//! Cracks come in kinds: transverse ones across the whole width, around
//! the middle or in from one edge, of their own width and wander, some
//! branched; longitudinal ones at the middle joint, in the outer wheel
//! tracks and in the edge zones (some as edge cracks with short cracks off
//! to the edge); block cracking (rectangular blocks, smaller on worse
//! roads); alligator areas of 1 – 13 m in the outer or inner wheel track
//! or between them with their own cell size; sealed bands of 3 – 9 cm.
//!
//! This module is the shader's twin: the placement and outline functions
//! below compute exactly what `road.wesl` draws (same hashes, same
//! constants, the same value noise [`vnoise`]), so the ride feel
//! ([`crate::road_feel`]) meets what is drawn and the tests can check the
//! density per smoothness and class, the kinds per grade and that potholes
//! stay off the middle of main roads.

use crate::RoadSurface;
use crate::graph::RoadClass;
use crate::road_condition::{RoadCondition, Smoothness};
use crate::road_dress::{cell_key, hash2, unit};

/// The highest wear grade.
pub const MAX_GRADE: u8 = 7;
/// Cell length (m) of the transverse cracks.
pub const TRANSVERSE_CELL_M: f32 = 11.0;
/// Cell length (m) of the longitudinal cracks (three slots per cell: the
/// middle joint, the left and the right edge zone).
pub const LONGITUDINAL_CELL_M: f32 = 16.0;
/// Cell length (m) of the alligator areas (one chance per side).
pub const ALLIGATOR_CELL_M: f32 = 20.0;
/// Cell length (m) of the potholes (one chance per side).
pub const POTHOLE_CELL_M: f32 = 7.0;
/// Largest pothole radius (m).
pub const MAX_POTHOLE_R: f32 = 0.6;
/// Half the distance between the wheel ruts of a track (m; the shader's
/// track look).
pub const RUT_M: f32 = 0.8;
/// On main roads (trunk, primary, secondary) a pothole's centre lies at
/// most this far from the edge, whatever the grade (m).
pub const MAIN_ROAD_ZONE_M: f32 = 1.2;
/// Cell size (m, both ways) of the loose stones.
pub const STONE_CELL_M: f32 = 0.1;

/// Trunk, primary and secondary roads.
pub fn main_road(class: RoadClass) -> bool {
    matches!(
        class,
        RoadClass::Trunk | RoadClass::Primary | RoadClass::Secondary
    )
}

/// The grade of an OSM `smoothness` value (`None`: not tagged).
pub fn smoothness_grade(s: Smoothness) -> Option<u8> {
    Some(match s {
        Smoothness::Unknown => return None,
        Smoothness::Excellent => 1,
        Smoothness::Good => 2,
        Smoothness::Intermediate => 4,
        Smoothness::Bad => 5,
        Smoothness::VeryBad => 6,
        Smoothness::Horrible | Smoothness::VeryHorrible | Smoothness::Impassable => 7,
    })
}

/// The estimated grade of a road without `smoothness` (spec Phase 8 Q14
/// (b), v5.35: untagged roads are drawn **cleaner and plainer** than
/// before, so that mostly the tagged roads show damage): from `tracktype`
/// (grade 1 … 5, 0: none) where it is tagged, else from the class – main
/// roads (trunk, primary, secondary) excellent, every other paved class
/// good, tracks between good and intermediate – and the surface (worn
/// asphalt, i.e. a `tracktype=grade1` track, at least 3). Before v5.35:
/// trunk 1, primary / secondary 2, tertiary / residential / cycleway 3,
/// unclassified / service / path 4, track 5, `tracktype` 4 … 7, worn
/// asphalt 5 ([`ESTIMATE_BEFORE_Q14`]).
pub fn estimate_grade(class: RoadClass, surface: RoadSurface, tracktype: u8) -> u8 {
    let base: u8 = match tracktype {
        1 => 2,
        2 | 3 => 3,
        4 | 5 => 4,
        _ => match class {
            RoadClass::Trunk | RoadClass::Primary | RoadClass::Secondary => 1,
            RoadClass::Tertiary
            | RoadClass::Residential
            | RoadClass::Cycleway
            | RoadClass::Unclassified
            | RoadClass::Service
            | RoadClass::Path => 2,
            RoadClass::Track => 3,
        },
    };
    if surface == RoadSurface::WornAsphalt {
        base.max(3)
    } else {
        base
    }
}

/// The estimate before Phase 8 Q14 (b) (v4.75 – v5.34), kept for the
/// comparison in the tests and the spec: (class, grade) without
/// `tracktype`.
pub const ESTIMATE_BEFORE_Q14: [(RoadClass, u8); 10] = [
    (RoadClass::Trunk, 1),
    (RoadClass::Primary, 2),
    (RoadClass::Secondary, 2),
    (RoadClass::Tertiary, 3),
    (RoadClass::Unclassified, 4),
    (RoadClass::Residential, 3),
    (RoadClass::Service, 4),
    (RoadClass::Track, 5),
    (RoadClass::Cycleway, 3),
    (RoadClass::Path, 4),
];

/// The wear of a strip: its grade and whether it is an estimate (no OSM
/// `smoothness`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wear {
    pub grade: u8,
    pub estimated: bool,
}

/// The wear of a strip of `class` and `surface` in `condition`:
/// `smoothness` first, else [`estimate_grade`].
pub fn wear_of(class: RoadClass, surface: RoadSurface, condition: &RoadCondition) -> Wear {
    match smoothness_grade(condition.smoothness) {
        Some(grade) => Wear {
            grade,
            estimated: false,
        },
        None => Wear {
            grade: estimate_grade(class, surface, condition.tracktype),
            estimated: true,
        },
    }
}

/// The grade alone ([`wear_of`]).
pub fn wear_grade(class: RoadClass, surface: RoadSurface, condition: &RoadCondition) -> u8 {
    wear_of(class, surface, condition).grade
}

/// The chances of one grade (`road.wesl`'s `damage`, `repair`, `loose`,
/// `cracks` and `holes` tables).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct WearDensity {
    /// A transverse crack per [`TRANSVERSE_CELL_M`] cell.
    pub transverse: f32,
    /// A longitudinal crack per [`LONGITUDINAL_CELL_M`] cell and slot.
    pub longitudinal: f32,
    /// An alligator (fatigue) area per [`ALLIGATOR_CELL_M`] cell and side.
    pub alligator: f32,
    /// A pothole per [`POTHOLE_CELL_M`] cell and side.
    pub pothole: f32,
    /// A repaired patch per `road_dress::PATCH_CELL_M` cell and side.
    pub patch: f32,
    /// The share of the cracks that are sealed (bitumen lines).
    pub sealed: f32,
    /// How far from the road edge (m) a pothole's centre may lie.
    pub pothole_zone_m: f32,
    /// How much the wheel tracks darken and polish.
    pub tracks: f32,
    /// A loose stone per [`STONE_CELL_M`] cell on a paved road (unpaved
    /// roads add [`UNPAVED_STONES`]).
    pub stones: f32,
    /// How deep the ruts of an unpaved road look (0 … 1).
    pub ruts: f32,
    /// A block-cracked area per [`BLOCK_CELL_M`] cell.
    pub block: f32,
    /// The share of the edge zones' longitudinal cracks that are edge
    /// cracks (short cracks branching off to the edge).
    pub edge_cracks: f32,
    /// The share of the open transverse cracks with a branch.
    pub branch: f32,
    /// The largest pothole radius of the grade (m; from
    /// [`MIN_POTHOLE_R`]).
    pub pothole_r_max: f32,
    /// The share of the potholes filled with fresh tarmac (paved roads).
    pub filled: f32,
    /// The share of the potholes with one or two more next to them.
    pub cluster: f32,
    /// The grade as 0 (excellent) … 1 (horrible): deeper holes, smaller
    /// blocks.
    pub t: f32,
}

/// The loose stones every unpaved road has, on top of its grade's.
pub const UNPAVED_STONES: f32 = 0.06;

/// The chances of `grade` with the quality preset's `share` (0 … 1) of the
/// features (chances only; the sealed share, the pothole zone, the shapes
/// and the wheel tracks are the grade's). Grade 0 draws nothing.
pub fn wear_density(grade: u8, share: f32) -> WearDensity {
    if grade == 0 {
        return WearDensity::default();
    }
    let t = f32::from(grade.min(MAX_GRADE) - 1) / f32::from(MAX_GRADE - 1);
    let s = share.clamp(0.0, 1.0);
    WearDensity {
        transverse: (0.012 + 0.5 * t) * s,
        longitudinal: (0.008 + 0.45 * t) * s,
        alligator: 0.4 * t.powf(1.5) * s,
        pothole: 0.5 * t.powf(2.5) * s,
        patch: (0.01 + 0.3 * t) * s,
        sealed: 0.75 - 0.6 * t,
        pothole_zone_m: if t <= 1.0 / 3.0 {
            0.7
        } else {
            0.7 + 4.0 * (t - 1.0 / 3.0)
        },
        tracks: 0.05 + 0.12 * t,
        stones: 0.5 * (t - 0.5).max(0.0) * s,
        ruts: 0.2 + 0.6 * t,
        block: 0.5 * (t - 0.25).max(0.0) * s,
        edge_cracks: 0.15 + 0.7 * t,
        branch: 0.1 + 0.6 * t,
        pothole_r_max: 0.25 + 0.35 * t,
        filled: 0.5 - 0.3 * t,
        cluster: 0.05 + 0.3 * t,
        t,
    }
}

/// What a surface layer takes of the damage: (cracks, sealing, alligator
/// areas and patches on; pothole factor). Asphalt all of it; paving only
/// sunken spots, rarely; gravel and dirt no cracks but hollows, more often.
pub fn surface_factors(surface: RoadSurface) -> (bool, f32) {
    match surface {
        RoadSurface::Asphalt | RoadSurface::WornAsphalt => (true, 1.0),
        RoadSurface::Paving => (false, 0.3),
        RoadSurface::Gravel | RoadSurface::Dirt => (false, 1.5),
    }
}

/// Whether holes on `surface` can be filled with tarmac (paved roads; a
/// hollow on gravel or dirt stays a hollow).
pub fn fillable(surface: RoadSurface) -> bool {
    !matches!(surface, RoadSurface::Gravel | RoadSurface::Dirt)
}

/// Value noise of the shader (`vnoise`): bilinear with a smoothstep over
/// the corners' [`hash2`] values, 0 … 1.
pub fn vnoise(p: [f32; 2]) -> f32 {
    let (ix, iy) = (p[0].floor(), p[1].floor());
    let (fx, fy) = (p[0] - ix, p[1] - iy);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let (x, y) = (ix as i32, iy as i32);
    let a = unit(hash2(x, y));
    let b = unit(hash2(x.wrapping_add(1), y));
    let c = unit(hash2(x, y.wrapping_add(1)));
    let d = unit(hash2(x.wrapping_add(1), y.wrapping_add(1)));
    let mix = |p: f32, q: f32, t: f32| p + (q - p) * t;
    mix(mix(a, b, sx), mix(c, d, sx), sy)
}

/// What decides a road's potholes: the chance per cell (with the surface
/// factor), the zone from the edge, the track look and the grade's shapes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HoleSpec {
    pub chance: f32,
    pub zone_m: f32,
    /// In the wheel ruts of a track.
    pub track: bool,
    pub r_max: f32,
    /// The grade as 0 … 1 ([`WearDensity::t`]): deeper holes.
    pub t: f32,
    /// Share filled with tarmac (0 on gravel and dirt).
    pub filled: f32,
    /// Share clustered.
    pub cluster: f32,
}

impl HoleSpec {
    /// The spec of a grade's density `d` with the surface's pothole
    /// `factor`, the road's `zone_m` ([`pothole_zone`]), the track look and
    /// whether the holes can be filled ([`fillable`]).
    pub fn of(d: &WearDensity, factor: f32, zone_m: f32, track: bool, fill: bool) -> Self {
        Self {
            chance: d.pothole * factor,
            zone_m,
            track,
            r_max: d.pothole_r_max,
            t: d.t,
            filled: if fill { d.filled } else { 0.0 },
            cluster: d.cluster,
        }
    }
}

/// A smaller hole next to a pothole (a cluster of two or three, sometimes
/// merged into one outline).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lobe {
    pub u: f32,
    pub v: f32,
    pub r: f32,
}

/// A pothole in the strip frame: an irregular outline – an ellipse of
/// `aspect` turned by `angle`, its radius warped by value noise of
/// `warp` and `freq` – with up to two smaller [`Lobe`]s, `depth` deep, or
/// filled with fresh tarmac.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pothole {
    /// Centre across (m from the road's middle, + right).
    pub u: f32,
    /// Centre along (m).
    pub v: f32,
    /// The mean radius (m): the outline's area as a circle's.
    pub r: f32,
    /// An edge break: the hole bites into the road edge.
    pub edge: bool,
    /// Long axis / short axis (1 round … 2.6).
    pub aspect: f32,
    /// The long axis' angle from the along direction (rad).
    pub angle: f32,
    /// Depth (m, 1 … 8 cm).
    pub depth: f32,
    /// How much the noise warps the radius (± half of it).
    pub warp: f32,
    /// The noise's frequency over the outline.
    pub freq: f32,
    /// The noise's offset (its own outline).
    pub noise: f32,
    /// Filled with fresh tarmac: dark, smooth, no recess – not felt.
    pub filled: bool,
    pub lobes: [Option<Lobe>; 2],
}

/// Smallest pothole radius (m).
pub const MIN_POTHOLE_R: f32 = 0.1;
/// The longest half axis of a pothole's outline (m), so it stays in its
/// cell.
pub const MAX_POTHOLE_AXIS_M: f32 = 0.8;
/// How far along a cluster's lobe lies from the hole's centre at most (m).
pub const LOBE_REACH_M: f32 = 0.75;
/// The outline's rim reaches out to this q (1 the outline).
pub const RIM_Q: f32 = 1.25;

impl Pothole {
    /// The number of outlines: the hole and its lobes.
    pub fn shapes(&self) -> usize {
        1 + self.lobes.iter().flatten().count()
    }

    /// Where `(u, v)` lies against outline `j` (0 the hole, 1 – 2 its
    /// lobes): 1 on the outline, below inside. The shader's `hole_q`.
    pub fn shape_q(&self, j: usize, u: f32, v: f32) -> f32 {
        let (n, off) = if j == 0 {
            let (du, dv) = (u - self.u, v - self.v);
            let (s, c) = self.angle.sin_cos();
            let along = du * s + dv * c;
            let across = du * c - dv * s;
            let ra = self.aspect.sqrt();
            ([across * ra / self.r, along / (self.r * ra)], 0.0)
        } else {
            let Some(l) = self.lobes[j - 1] else {
                return f32::INFINITY;
            };
            ([(u - l.u) / l.r, (v - l.v) / l.r], 17.0 * j as f32)
        };
        let w = vnoise([n[0] * self.freq + self.noise, n[1] * self.freq + off]);
        (n[0] * n[0] + n[1] * n[1]).sqrt() / (1.0 - 0.5 * self.warp + self.warp * w)
    }

    /// The smallest [`Self::shape_q`] of all outlines.
    pub fn q(&self, u: f32, v: f32) -> f32 {
        (0..self.shapes())
            .map(|j| self.shape_q(j, u, v))
            .fold(f32::INFINITY, f32::min)
    }

    /// The centre and the largest radius (m) of outline `j` out to `q`.
    pub fn shape_reach(&self, j: usize, q: f32) -> ([f32; 2], f32) {
        let k = q * (1.0 + 0.5 * self.warp);
        if j == 0 {
            ([self.u, self.v], self.r * self.aspect.sqrt() * k)
        } else {
            let l = self.lobes[j - 1].expect("a lobe");
            ([l.u, l.v], l.r * k)
        }
    }

    /// How far along from its centre the hole reaches out to `q` (m).
    pub fn reach_v(&self, q: f32) -> f32 {
        (0..self.shapes())
            .map(|j| {
                let (c, r) = self.shape_reach(j, q);
                (c[1] - self.v).abs() + r
            })
            .fold(0.0, f32::max)
    }
}

/// The pothole of cell `k` on `side` (0 left, 1 right) of a road of half
/// width `half` and damage `seed` ([`crate::road_dress::road_seed`]). The
/// shader's `pothole`.
pub fn pothole_in_cell(k: i32, side: i32, seed: u32, half: f32, s: &HoleSpec) -> Option<Pothole> {
    let kc = cell_key(k, seed);
    let h = |salt: i32| unit(hash2(kc, salt + side));
    if h(211) >= s.chance {
        return None;
    }
    let a = h(223);
    let mut r = MIN_POTHOLE_R + (s.r_max - MIN_POTHOLE_R).max(0.0) * a * a.sqrt();
    if s.track {
        r = r.min(0.35);
    }
    let v = (k as f32 + 0.2 + 0.6 * h(227)) * POTHOLE_CELL_M;
    let sgn = if side == 0 { -1.0 } else { 1.0 };
    let e = h(233);
    let edge = !s.track && h(229) < 0.35;
    // The outline: edge breaks long along the edge; a third of the others
    // long along the wheel's way, the rest roundish at any angle.
    let (aspect, angle) = if edge {
        (1.4 + 1.2 * h(241), 0.3 * (h(247) - 0.5))
    } else if h(251) < 0.3 {
        (1.5 + 0.9 * h(241), 0.5 * (h(247) - 0.5))
    } else {
        (1.0 + 0.45 * h(241), std::f32::consts::PI * h(247))
    };
    let aspect = aspect.min((MAX_POTHOLE_AXIS_M / r).powi(2)).max(1.0);
    let minor = r / aspect.sqrt();
    let u = if s.track {
        sgn * (RUT_M + 0.2 * (e - 0.5))
    } else if edge {
        sgn * (half - 0.3 * minor)
    } else {
        let span = (s.zone_m.min(half - r) - r - 0.1).max(0.0);
        sgn * (half - r - 0.1 - span * e * e)
    };
    let size = ((r - MIN_POTHOLE_R) / (MAX_POTHOLE_R - MIN_POTHOLE_R)).clamp(0.0, 1.0);
    let depth = 0.01 + 0.07 * (0.3 * s.t + 0.7 * h(271) * (0.35 + 0.65 * size)).min(1.0);
    let mut hole = Pothole {
        u,
        v,
        r,
        edge,
        aspect,
        angle,
        depth,
        warp: 0.15 + 0.3 * h(263),
        freq: 1.5 + 2.0 * h(269),
        noise: 50.0 * h(239),
        filled: h(277) < s.filled,
        lobes: [None; 2],
    };
    if !edge && h(257) < s.cluster {
        let n = 1 + usize::from(h(281) < 0.4);
        for j in 0..n {
            let b = 283 + 10 * j as i32;
            let rj = (r * (0.4 + 0.35 * h(b))).clamp(0.07, 0.3);
            let phi = std::f32::consts::TAU * h(b + 4);
            let rho = (r * (0.9 + 0.8 * h(b + 8)) + 0.3 * rj).min(LOBE_REACH_M);
            // Never nearer the middle than the hole (main roads keep their
            // holes near the edge), and on the road.
            let du = (rho * phi.sin()).abs();
            let uj = (u.abs() + du).min(half - 0.6 * rj).max(rj);
            hole.lobes[j] = Some(Lobe {
                u: sgn * uj,
                v: v + rho * phi.cos(),
                r: rj,
            });
        }
    }
    Some(hole)
}

/// An alligator (fatigue cracking) area: an ellipse in the strip frame,
/// its outline warped by noise, cracked into cells of `cell` m.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Alligator {
    pub u: f32,
    /// Start and end along (m).
    pub v: (f32, f32),
    /// Half width across (m).
    pub half_w: f32,
    /// The crack pattern's cell size (m).
    pub cell: f32,
    /// The outline noise's offset.
    pub noise: f32,
}

impl Alligator {
    /// The outline function (the shader's): inside below 0.85, the area
    /// fully drawn below 0.7.
    pub fn e(&self, u: f32, v: f32) -> f32 {
        let a = (u - self.u) / self.half_w;
        let h = 0.5 * (self.v.1 - self.v.0);
        let b = (v - self.v.0 - h) / h;
        a * a + b * b + 0.6 * (vnoise([u * 2.5 + self.noise, v * 2.5 + self.noise]) - 0.5)
    }
}

/// Where an alligator area counts as entered ([`Alligator::e`]): the
/// middle of the drawn edge.
pub const ALLIGATOR_E: f32 = 0.85;

/// The alligator area of cell `k` on `side`: in the outer wheel track
/// mostly, else the inner one or between them (the ruts of a track), of
/// 1 – 14 m and 0.2 – 0.9 m half width. The shader's `alligator`.
pub fn alligator_in_cell(
    k: i32,
    side: i32,
    seed: u32,
    half: f32,
    chance: f32,
    track: bool,
) -> Option<Alligator> {
    let kc = cell_key(k, seed);
    let h = |salt: i32| unit(hash2(kc, salt + side));
    if h(307) >= chance {
        return None;
    }
    let start = (k as f32 + 0.05 + 0.25 * h(311)) * ALLIGATOR_CELL_M;
    let len = 1.0 + 12.0 * h(313).powf(1.5);
    let sgn = if side == 0 { -1.0 } else { 1.0 };
    let outer = (half - 0.9).max(0.3);
    let p = h(319);
    let at = if track {
        RUT_M
    } else if p < 0.6 {
        outer
    } else if p < 0.85 {
        (outer - 1.6).max(0.3)
    } else {
        (0.5 * half).max(0.3)
    };
    Some(Alligator {
        u: sgn * at,
        v: (start, start + len),
        half_w: (0.2 + 0.7 * h(317).powf(1.3)).min(half * 0.5),
        cell: 0.12 + 0.18 * h(323),
        noise: 13.0 * side as f32 + 40.0 * h(329),
    })
}

/// Cell length (m) of the block-cracked areas (one chance per cell).
pub const BLOCK_CELL_M: f32 = 30.0;

/// A block-cracked area: a rounded rectangle of the road (one half or the
/// whole width) cracked into blocks of about `size` m.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlockArea {
    /// Across from … to (m).
    pub u: (f32, f32),
    /// Along from … to (m).
    pub v: (f32, f32),
    pub size: f32,
    pub noise: f32,
}

/// Where a block area counts as entered ([`BlockArea::e`]).
pub const BLOCK_E: f32 = 0.95;

impl BlockArea {
    /// The outline function (the shader's): a superellipse warped by
    /// noise; inside below [`BLOCK_E`].
    pub fn e(&self, u: f32, v: f32) -> f32 {
        let x = (u - 0.5 * (self.u.0 + self.u.1)) / (0.5 * (self.u.1 - self.u.0));
        let y = (v - 0.5 * (self.v.0 + self.v.1)) / (0.5 * (self.v.1 - self.v.0));
        let (x2, y2) = (x * x, y * y);
        (x2 * x2 + y2 * y2).sqrt().sqrt() + 0.25 * (vnoise([u * 0.9 + self.noise, v * 0.9]) - 0.5)
    }
}

/// The block-cracked area of cell `k`: from intermediate on, on worse
/// roads smaller blocks. The shader's `blocks`.
pub fn block_in_cell(k: i32, seed: u32, half: f32, chance: f32, t: f32) -> Option<BlockArea> {
    let kc = cell_key(k, seed);
    let h = |salt: i32| unit(hash2(kc, salt));
    if h(701) >= chance {
        return None;
    }
    let start = (k as f32 + 0.05 + 0.4 * h(703)) * BLOCK_CELL_M;
    let len = 3.0 + 11.0 * h(707);
    let w = h(709);
    let u = if w < 0.4 {
        (-half + 0.15, half - 0.15)
    } else if w < 0.7 {
        (-half + 0.15, -0.1)
    } else {
        (0.1, half - 0.15)
    };
    Some(BlockArea {
        u,
        v: (start, start + len),
        size: 0.35 + 0.9 * h(711) * (1.0 - 0.5 * t),
        noise: 60.0 * h(713),
    })
}

/// A longitudinal crack: its line across and its extent along.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Longitudinal {
    /// Mean across (m; the line wanders ± `amp` around it).
    pub u: f32,
    pub v: (f32, f32),
    pub sealed: bool,
    /// Half width (m): open 3 – 9 mm, sealed 1.5 – 4.5 cm.
    pub width: f32,
    pub amp: f32,
    pub phase: f32,
    /// An edge crack: short cracks branch off it to the road edge.
    pub spurs: bool,
}

/// The longitudinal crack slots: 0 the middle joint, 1 – 2 the left and
/// right edge zone, 3 – 4 the left and right outer wheel track.
pub const LONGITUDINAL_SLOTS: i32 = 5;

impl Longitudinal {
    /// The line's place across at `v`.
    pub fn u_at(&self, v: f32) -> f32 {
        self.u
            + self.amp * (v * 0.9 + self.phase).sin()
            + 0.3 * self.amp * (v * 3.7 + self.phase * 1.3).sin()
    }
}

/// The longitudinal crack of cell `k` in `slot` ([`LONGITUDINAL_SLOTS`]);
/// `edge_share` of the open ones in the edge zones are edge cracks. The
/// shader's `longitudinal`.
pub fn longitudinal_in_cell(
    k: i32,
    slot: i32,
    seed: u32,
    half: f32,
    chance: f32,
    sealed: f32,
    edge_share: f32,
) -> Option<Longitudinal> {
    let kc = cell_key(k, seed);
    let h = |salt: i32| unit(hash2(kc, salt + slot));
    if h(401) >= chance {
        return None;
    }
    let off = 0.5 + 6.0 * h(411);
    let start = k as f32 * LONGITUDINAL_CELL_M + off;
    let len = 2.0 + (13.3 - off) * h(421);
    let o = h(431);
    let wheel = if half >= 2.5 { 0.5 * half + 0.8 } else { 0.8 };
    let wheel = wheel.min(half - 0.3) + 0.2 * (o - 0.5);
    let u = match slot {
        0 => 0.08 + 0.12 * o,
        1 => -(half - 0.15 - 0.6 * o),
        2 => half - 0.15 - 0.6 * o,
        3 => -wheel,
        _ => wheel,
    };
    let is_sealed = h(441) < sealed;
    Some(Longitudinal {
        u,
        v: (start, start + len),
        sealed: is_sealed,
        width: if is_sealed {
            0.015 + 0.03 * h(451)
        } else {
            0.003 + 0.006 * h(451)
        },
        amp: 0.03 + 0.09 * h(461),
        phase: o * 37.0,
        spurs: (slot == 1 || slot == 2) && !is_sealed && h(471) < edge_share,
    })
}

/// A branch off a transverse crack: from `u` on the crack, `len` m at
/// `angle` from the across direction, towards `dir` along (±1) and `to`
/// across (±1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Branch {
    pub u: f32,
    pub dir: f32,
    pub to: f32,
    pub angle: f32,
    pub len: f32,
}

/// A transverse crack across the road (or its sealing line).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transverse {
    /// Where its line lies along at the middle of the road (m; the line
    /// wanders [`transverse_v_at`]).
    pub v: f32,
    /// Across from … to (m): the whole width, around the middle or in
    /// from one edge.
    pub span: (f32, f32),
    pub sealed: bool,
    /// The hash of the cell (the line's wander).
    pub phase: f32,
    pub amp: f32,
    pub freq: f32,
    /// Half width (m): open 3 – 10 mm, sealed 1.5 – 4.5 cm.
    pub width: f32,
    pub branch: Option<Branch>,
}

/// The transverse crack of cell `k` ([`TRANSVERSE_CELL_M`]) of a road of
/// half width `half` and `seed`, for a chance per cell, the sealed share
/// and the share of branched cracks. The shader's `transverse`.
pub fn transverse_in_cell(
    k: i32,
    seed: u32,
    half: f32,
    chance: f32,
    sealed: f32,
    branch: f32,
) -> Option<Transverse> {
    let kc = cell_key(k, seed);
    let h = |salt: i32| unit(hash2(kc, salt));
    let ph = h(101);
    if ph >= chance {
        return None;
    }
    let kind = h(113);
    let span = if kind < 0.35 {
        (-half, half)
    } else if kind < 0.7 {
        let r = half * (0.4 + 0.6 * h(107));
        (-r, r)
    } else {
        let l = half * (0.35 + 0.6 * h(107));
        if h(127) < 0.5 {
            (-half, -half + l)
        } else {
            (half - l, half)
        }
    };
    let is_sealed = h(109) < sealed;
    let branch = (!is_sealed && h(149) < branch).then(|| Branch {
        u: span.0 + (span.1 - span.0) * (0.15 + 0.7 * h(151)),
        dir: if h(157) < 0.5 { -1.0 } else { 1.0 },
        to: if h(163) < 0.5 { -1.0 } else { 1.0 },
        angle: 0.45 + 0.6 * h(167),
        len: 0.3 + 1.1 * h(173),
    });
    Some(Transverse {
        v: (k as f32 + 0.15 + 0.7 * h(103)) * TRANSVERSE_CELL_M,
        span,
        sealed: is_sealed,
        phase: ph,
        amp: 0.06 + 0.2 * h(137),
        freq: 1.2 + 1.2 * h(139),
        width: if is_sealed {
            0.015 + 0.03 * h(131)
        } else {
            0.003 + 0.007 * h(131)
        },
        branch,
    })
}

/// Where a transverse crack lies along at `u` across (m): the shader's
/// wandering line.
pub fn transverse_v_at(t: &Transverse, u: f32) -> f32 {
    t.v + t.amp * (u * t.freq + t.phase * 40.0).sin() + 0.04 * (u * 6.1 + t.phase * 13.0).sin()
}

/// The pothole zone of a road: its grade's, at most
/// [`MAIN_ROAD_ZONE_M`] on main roads (the shader's `zone`).
pub fn pothole_zone(zone_m: f32, main: bool) -> f32 {
    if main {
        zone_m.min(MAIN_ROAD_ZONE_M)
    } else {
        zone_m
    }
}

/// Lanes drawn across a carriageway of half width `half` with `lanes`
/// [forward, backward] (each at most 7; none tagged: one lane, two from
/// 5 m width) – the shader's `tracks`.
pub fn lane_count(half: f32, lanes: [u8; 2]) -> u32 {
    let n = u32::from(lanes[0].min(7)) + u32::from(lanes[1].min(7));
    if n == 0 {
        if half >= 2.5 { 2 } else { 1 }
    } else {
        n
    }
}

/// The middles of the lanes, across (m from the centre line, -half …
/// half).
pub fn lane_middles(half: f32, lanes: [u8; 2]) -> Vec<f32> {
    let n = lane_count(half, lanes);
    let lane = 2.0 * half / n as f32;
    (0..n).map(|i| -half + (i as f32 + 0.5) * lane).collect()
}

/// The tyre tracks either side of a lane's middle (m).
pub const TRACK_OFFSET_M: f32 = 0.8;

/// How much a point `u` m across lies in a wheel track (0 … 1; two per
/// lane, [`TRACK_OFFSET_M`] either side of its middle, 0.3 m wide): the
/// shader's `tracks`, where the worn tracks darken and the loose stones
/// are swept out.
pub fn wheel_tracks(u: f32, half: f32, lanes: [u8; 2]) -> f32 {
    lane_middles(half, lanes)
        .into_iter()
        .map(|c| {
            let d1 = (u - (c - TRACK_OFFSET_M)) / 0.3;
            let d2 = (u - (c + TRACK_OFFSET_M)) / 0.3;
            (-d1 * d1).exp().max((-d2 * d2).exp())
        })
        .fold(0.0, f32::max)
}

/// A loose stone (grit, a broken-off piece of the surface) in the strip
/// frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stone {
    pub u: f32,
    pub v: f32,
    pub r: f32,
}

/// The stone of cell (`i` across, `j` along; [`STONE_CELL_M`]) for a chance
/// per cell. The shader's `stones`: it stays inside its cell.
pub fn stone_in_cell(i: i32, j: i32, chance: f32) -> Option<Stone> {
    if unit(hash2(i * 7 + 601, j)) >= chance {
        return None;
    }
    let r = 0.008 + 0.017 * unit(hash2(i + 613, j * 3));
    let c = STONE_CELL_M;
    let ou = r + (c - 2.0 * r) * unit(hash2(i * 5 + 617, j + 11));
    let ov = r + (c - 2.0 * r) * unit(hash2(i + 619, j * 5 + 13));
    Some(Stone {
        u: i as f32 * c + ou,
        v: j as f32 * c + ov,
        r,
    })
}
#[cfg(test)]
mod tests {
    use super::*;

    const ALL_CLASSES: [RoadClass; 10] = [
        RoadClass::Trunk,
        RoadClass::Primary,
        RoadClass::Secondary,
        RoadClass::Tertiary,
        RoadClass::Unclassified,
        RoadClass::Residential,
        RoadClass::Service,
        RoadClass::Track,
        RoadClass::Cycleway,
        RoadClass::Path,
    ];

    fn spec(grade: u8, zone: Option<f32>, track: bool) -> HoleSpec {
        let d = wear_density(grade, 1.0);
        HoleSpec::of(&d, 1.0, zone.unwrap_or(d.pothole_zone_m), track, true)
    }

    fn potholes_seeded(grade: u8, half: f32, cells: i32, track: bool, seed: u32) -> Vec<Pothole> {
        let sp = spec(grade, None, track);
        (0..cells)
            .flat_map(|k| (0..2).map(move |s| (k, s)))
            .filter_map(|(k, s)| pothole_in_cell(k, s, seed, half, &sp))
            .collect()
    }

    fn potholes(grade: u8, half: f32, cells: i32, track: bool) -> Vec<Pothole> {
        potholes_seeded(grade, half, cells, track, 9)
    }

    fn cond(s: Smoothness) -> RoadCondition {
        RoadCondition {
            smoothness: s,
            ..RoadCondition::UNKNOWN
        }
    }

    #[test]
    fn smoothness_drives_the_grade_and_the_class_only_estimates() {
        // Tagged: the class does not matter.
        for c in ALL_CLASSES {
            for (sm, g) in [
                (Smoothness::Excellent, 1),
                (Smoothness::Good, 2),
                (Smoothness::Intermediate, 4),
                (Smoothness::Bad, 5),
                (Smoothness::VeryBad, 6),
                (Smoothness::Horrible, 7),
                (Smoothness::VeryHorrible, 7),
            ] {
                let w = wear_of(c, RoadSurface::Asphalt, &cond(sm));
                assert_eq!(
                    w,
                    Wear {
                        grade: g,
                        estimated: false
                    },
                    "{c:?} {sm:?}"
                );
            }
        }
        // Worse smoothness, more damage.
        let mut last = 0;
        for sm in &Smoothness::ALL[1..] {
            let g = smoothness_grade(*sm).unwrap();
            assert!(g >= last);
            last = g;
        }
        // Untagged: an estimate from the class …
        let e = |c| wear_of(c, RoadSurface::Asphalt, &RoadCondition::UNKNOWN);
        assert!(e(RoadClass::Primary).estimated);
        let g = |c| e(c).grade;
        assert_eq!(g(RoadClass::Trunk), g(RoadClass::Primary));
        assert!(g(RoadClass::Primary) < g(RoadClass::Residential));
        assert!(g(RoadClass::Residential) <= g(RoadClass::Unclassified));
        assert!(g(RoadClass::Unclassified) < g(RoadClass::Track));
        for c in ALL_CLASSES {
            assert!((1..=MAX_GRADE).contains(&g(c)), "{c:?}");
        }
        // … from `tracktype` where a track has it …
        let tt = |t| {
            let c = RoadCondition {
                tracktype: t,
                ..RoadCondition::UNKNOWN
            };
            wear_of(RoadClass::Track, RoadSurface::Gravel, &c)
        };
        assert!(tt(1).grade < tt(3).grade && tt(3).grade < tt(5).grade);
        assert!(tt(4).estimated);
        // … and worn asphalt at least 5; smoothness wins over tracktype.
        let worn = wear_grade(
            RoadClass::Trunk,
            RoadSurface::WornAsphalt,
            &RoadCondition::UNKNOWN,
        );
        assert!(worn >= 3);
        let both = RoadCondition {
            smoothness: Smoothness::Excellent,
            tracktype: 5,
            ..RoadCondition::UNKNOWN
        };
        assert_eq!(wear_grade(RoadClass::Track, RoadSurface::Asphalt, &both), 1);
        assert_eq!(wear_density(0, 1.0), WearDensity::default());
    }
    #[test]
    fn untagged_roads_are_cleaner_than_before_and_tagged_ones_unchanged() {
        // Phase 8 Q14 (b): every untagged class at most its old grade, the
        // main and residential streets lower, and only tracks above
        // "good"; the tagged grades (Q13 (b)) are the same as before.
        for (c, before) in ESTIMATE_BEFORE_Q14 {
            let now = wear_grade(c, RoadSurface::Asphalt, &RoadCondition::UNKNOWN);
            assert!(now <= before, "{c:?}: {now} > {before}");
            if c != RoadClass::Trunk {
                assert!(now < before, "{c:?}: {now} not below {before}");
            }
            if c != RoadClass::Track {
                assert!(now <= 2, "{c:?}: {now}");
            }
        }
        let unknown = RoadCondition::UNKNOWN;
        assert_eq!(
            wear_grade(RoadClass::Primary, RoadSurface::Asphalt, &unknown),
            1
        );
        assert_eq!(
            wear_grade(RoadClass::Residential, RoadSurface::Asphalt, &unknown),
            2
        );
        for (sm, g) in [
            (Smoothness::Excellent, 1),
            (Smoothness::Good, 2),
            (Smoothness::Intermediate, 4),
            (Smoothness::Bad, 5),
            (Smoothness::VeryBad, 6),
            (Smoothness::Horrible, 7),
        ] {
            let w = wear_grade(RoadClass::Primary, RoadSurface::Asphalt, &cond(sm));
            assert_eq!(w, g);
        }
        // `tracktype` estimates lower than before (4 … 7).
        let tt = |t| {
            let c = RoadCondition {
                tracktype: t,
                ..RoadCondition::UNKNOWN
            };
            wear_grade(RoadClass::Track, RoadSurface::Gravel, &c)
        };
        assert_eq!([tt(1), tt(2), tt(3), tt(4), tt(5)], [2, 3, 3, 4, 4]);
        // Far fewer potholes on 35 km of untagged residential street.
        let half = RoadClass::Residential.default_width_m() as f32 / 2.0;
        let (now, before) = (
            potholes(2, half, 5000, false).len(),
            potholes(3, half, 5000, false).len(),
        );
        assert!(now * 4 < before, "{now} vs {before}");
    }

    #[test]
    fn the_transverse_cracks_stay_in_their_cells() {
        let mut n = 0;
        for k in -500..500 {
            if let Some(t) = transverse_in_cell(k, 4, 3.0, 1.0, 0.3, 1.0) {
                n += 1;
                let c0 = k as f32 * TRANSVERSE_CELL_M;
                for u in [-3.0, -1.4, 0.0, 0.7, 3.0] {
                    let v = transverse_v_at(&t, u);
                    assert!(v >= c0 && v <= c0 + TRANSVERSE_CELL_M, "{t:?}");
                }
                // Its branch too.
                if let Some(b) = t.branch {
                    let v0 = transverse_v_at(&t, b.u);
                    let end = v0 + b.dir * b.len * b.angle.sin();
                    assert!(end >= c0 && end <= c0 + TRANSVERSE_CELL_M, "{t:?}");
                    assert!(b.u >= t.span.0 && b.u <= t.span.1);
                }
                let len = t.span.1 - t.span.0;
                assert!(t.span.0 >= -3.0 && t.span.1 <= 3.0 && len >= 1.05, "{t:?}");
            }
        }
        assert_eq!(n, 1000);
        assert!(transverse_in_cell(3, 4, 3.0, 0.0, 0.0, 0.0).is_none());
    }

    #[test]
    fn density_grows_with_the_grade_and_the_quality_share() {
        for g in 1..MAX_GRADE {
            let (a, b) = (wear_density(g, 1.0), wear_density(g + 1, 1.0));
            assert!(b.transverse > a.transverse && b.longitudinal > a.longitudinal);
            assert!(b.alligator > a.alligator);
            assert!(b.pothole > a.pothole && b.patch > a.patch);
            assert!(b.sealed < a.sealed, "minor roads are sealed less");
            assert!(b.pothole_zone_m >= a.pothole_zone_m);
        }
        // Excellent (grade 1): almost nothing – no potholes, no alligator
        // areas, no loose stones, a crack every kilometre or so.
        let ex = wear_density(1, 1.0);
        assert_eq!((ex.pothole, ex.alligator, ex.stones), (0.0, 0.0, 0.0));
        assert!(ex.transverse < 0.02 && ex.longitudinal < 0.01);
        // Loose stones from bad (5) on, more the worse.
        assert_eq!(wear_density(4, 1.0).stones, 0.0);
        assert!(wear_density(5, 1.0).stones > 0.0);
        assert!(wear_density(7, 1.0).stones > wear_density(6, 1.0).stones);
        // Intermediate: the odd small pothole, far fewer than bad.
        let (im, bad) = (wear_density(4, 1.0), wear_density(5, 1.0));
        assert!(im.pothole > 0.0 && im.pothole < 0.6 * bad.pothole);
        let (hi, lo) = (wear_density(5, 1.0), wear_density(5, 0.5));
        assert!((lo.pothole - 0.5 * hi.pothole).abs() < 1e-6);
        assert_eq!(lo.sealed, hi.sealed);
        assert_eq!(wear_density(5, 0.0).transverse, 0.0);
    }

    #[test]
    fn placement_is_deterministic_and_counts_follow_the_smoothness() {
        // The same cell gives the same hole, whatever else was asked.
        let a = potholes(6, 2.5, 2000, false);
        let b = potholes(6, 2.5, 2000, false);
        assert_eq!(a, b);
        let count = |c: RoadClass, sm: Smoothness| {
            let half = c.default_width_m() as f32 / 2.0;
            let g = wear_grade(c, RoadSurface::Asphalt, &cond(sm));
            potholes(g, half, 5000, false).len()
        };
        // 35 km of road each.
        let n: Vec<usize> = Smoothness::ALL[1..8]
            .iter()
            .map(|&sm| count(RoadClass::Residential, sm))
            .collect();
        assert_eq!(n[0], 0, "excellent: none");
        assert!(n[1] < 120, "good: a few ({})", n[1]);
        assert!(
            n[2] > n[1] && n[3] > n[2] && n[4] > n[3] && n[5] > n[4],
            "{n:?}"
        );
        // A bad primary road has about the potholes of a bad residential
        // road (the class only limits where they lie).
        let bad = count(RoadClass::Primary, Smoothness::Bad);
        assert!(bad * 2 > n[3], "{bad} vs {}", n[3]);
        // Untagged (estimated): a primary road few, a track many.
        let est = |c: RoadClass| {
            let half = c.default_width_m() as f32 / 2.0;
            let g = wear_grade(c, RoadSurface::Asphalt, &RoadCondition::UNKNOWN);
            potholes(g, half, 5000, c == RoadClass::Track).len()
        };
        assert_eq!(est(RoadClass::Trunk), 0);
        assert!(est(RoadClass::Residential) > 3 * est(RoadClass::Primary));
        assert!(est(RoadClass::Track) > est(RoadClass::Residential));
        // Cracks and loose stones too.
        let cracks = |g: u8| {
            let d = wear_density(g, 1.0);
            (0..5000)
                .flat_map(|k| (0..LONGITUDINAL_SLOTS).map(move |s| (k, s)))
                .filter(|&(k, s)| {
                    longitudinal_in_cell(k, s, 2, 3.0, d.longitudinal, d.sealed, d.edge_cracks)
                        .is_some()
                })
                .count()
        };
        assert!(cracks(5) > 2 * cracks(2));
        assert!(cracks(2) > 5 * cracks(1));
        let stones = |g: u8| {
            let d = wear_density(g, 1.0);
            (0..60)
                .flat_map(|i| (0..1000).map(move |j| (i, j)))
                .filter(|&(i, j)| stone_in_cell(i, j, d.stones).is_some())
                .count()
        };
        assert_eq!(stones(4), 0);
        assert!(stones(7) > stones(5) && stones(5) > 0);
    }
    #[test]
    fn potholes_stay_off_the_middle_of_main_roads() {
        // Estimated and every tagged smoothness, even horrible: the holes of
        // a main road stay near its edges.
        for c in [RoadClass::Trunk, RoadClass::Primary, RoadClass::Secondary] {
            let half = c.default_width_m() as f32 / 2.0;
            for sm in Smoothness::ALL {
                let g = wear_grade(c, RoadSurface::Asphalt, &cond(sm));
                let d = wear_density(g, 1.0);
                let zone = pothole_zone(d.pothole_zone_m, main_road(c));
                assert!(zone <= MAIN_ROAD_ZONE_M);
                let sp = spec(g, Some(zone), false);
                let holes: Vec<Pothole> = (0..20_000)
                    .flat_map(|k| (0..2).map(move |s| (k, s)))
                    .filter_map(|(k, s)| pothole_in_cell(k, s, 3, half, &sp))
                    .collect();
                for p in &holes {
                    assert!(p.u.abs() >= half - zone, "{c:?} {sm:?}: {p:?}");
                    assert!(p.u.abs() - p.r >= half - MAIN_ROAD_ZONE_M - MAX_POTHOLE_R);
                    // The cluster's other holes lie no nearer the middle.
                    for l in p.lobes.iter().flatten() {
                        assert!(l.u.abs() >= p.u.abs().min(half - 0.6 * l.r) - 1e-4, "{p:?}");
                        assert!(l.u.signum() == p.u.signum());
                    }
                }
                if sm == Smoothness::Bad {
                    assert!(!holes.is_empty(), "{c:?}");
                }
            }
        }
        // The estimate of a primary road: within 0.7 m of the edge.
        let g = wear_grade(
            RoadClass::Primary,
            RoadSurface::Asphalt,
            &RoadCondition::UNKNOWN,
        );
        assert_eq!(wear_density(g, 1.0).pothole_zone_m, 0.7);
        assert!(!main_road(RoadClass::Tertiary));
        assert_eq!(pothole_zone(3.0, false), 3.0);
        // A minor road's holes may lie anywhere on its half, never across
        // the middle (each side's cell keeps to its side).
        for p in potholes(6, 2.5, 20_000, false) {
            assert!(p.u.abs() >= p.r - 1e-4, "{p:?}");
            assert!(p.u.abs() <= 2.5 + 1e-4);
            for l in p.lobes.iter().flatten() {
                assert!(l.u.abs() <= 2.5 && l.u.signum() == p.u.signum(), "{p:?}");
            }
        }
        // Tracks: in the ruts.
        for p in potholes(5, 1.5, 20_000, true) {
            assert!((p.u.abs() - RUT_M).abs() <= 0.1 + 1e-4, "{p:?}");
        }
    }

    #[test]
    fn features_stay_in_their_cells() {
        // The shader looks only at the pixel's own cell.
        let all = HoleSpec {
            chance: 1.0,
            zone_m: 3.0,
            track: false,
            r_max: MAX_POTHOLE_R,
            t: 1.0,
            filled: 0.3,
            cluster: 1.0,
        };
        for k in -500..500 {
            for s in 0..2 {
                if let Some(p) = pothole_in_cell(k, s, 7, 3.0, &all) {
                    let c0 = k as f32 * POTHOLE_CELL_M;
                    let reach = p.reach_v(RIM_Q);
                    assert!(
                        p.v - reach >= c0 && p.v + reach <= c0 + POTHOLE_CELL_M,
                        "{p:?}"
                    );
                    assert!(reach <= 1.4, "{p:?}");
                }
                if let Some(a) = alligator_in_cell(k, s, 7, 3.0, 1.0, false) {
                    let c0 = k as f32 * ALLIGATOR_CELL_M;
                    let h = 0.5 * (a.v.1 - a.v.0) * 0.15;
                    assert!(
                        a.v.0 - h >= c0 && a.v.1 + h <= c0 + ALLIGATOR_CELL_M,
                        "{a:?}"
                    );
                }
            }
            if let Some(b) = block_in_cell(k, 7, 3.0, 1.0, 1.0) {
                let c0 = k as f32 * BLOCK_CELL_M;
                let h = 0.5 * (b.v.1 - b.v.0) * 0.18;
                assert!(b.v.0 - h >= c0 && b.v.1 + h <= c0 + BLOCK_CELL_M, "{b:?}");
                assert!(b.u.0 >= -3.0 && b.u.1 <= 3.0 && b.size >= 0.35);
            }
            for j in 0..2 {
                if let Some(st) = stone_in_cell(k, j, 1.0) {
                    let (u0, v0) = (k as f32 * STONE_CELL_M, j as f32 * STONE_CELL_M);
                    assert!(st.u - st.r >= u0 - 1e-5);
                    assert!(st.u + st.r <= u0 + STONE_CELL_M + 1e-5);
                    assert!(st.v - st.r >= v0 - 1e-5);
                    assert!(st.v + st.r <= v0 + STONE_CELL_M + 1e-5);
                }
            }
            for slot in 0..LONGITUDINAL_SLOTS {
                if let Some(l) = longitudinal_in_cell(k, slot, 7, 3.0, 1.0, 0.5, 0.5) {
                    let c0 = k as f32 * LONGITUDINAL_CELL_M;
                    assert!(l.v.0 >= c0 && l.v.1 <= c0 + LONGITUDINAL_CELL_M, "{l:?}");
                    assert!(l.u.abs() <= 3.0 - 0.1, "{l:?}");
                }
            }
        }
    }

    #[test]
    fn surfaces_take_their_share() {
        assert_eq!(surface_factors(RoadSurface::Asphalt), (true, 1.0));
        assert!(surface_factors(RoadSurface::WornAsphalt).0);
        let (cracks, holes) = surface_factors(RoadSurface::Paving);
        assert!(!cracks && holes < 1.0);
        assert!(surface_factors(RoadSurface::Gravel).1 > 1.0);
        assert!(!surface_factors(RoadSurface::Dirt).0);
    }

    #[test]
    fn the_wheel_tracks_lie_either_side_of_each_lane_middle() {
        // A 6 m road without lane tags: two lanes, middles at ±1.5 m.
        assert_eq!(lane_count(3.0, [0, 0]), 2);
        assert_eq!(lane_middles(3.0, [0, 0]), [-1.5, 1.5]);
        assert!(wheel_tracks(-2.3, 3.0, [0, 0]) > 0.99);
        assert!(wheel_tracks(0.7, 3.0, [0, 0]) > 0.99);
        assert!(wheel_tracks(-1.5, 3.0, [0, 0]) < 0.01);
        assert!(wheel_tracks(-2.95, 3.0, [0, 0]) < 0.05);
        // A narrow lane: one, its tracks either side of the centre line.
        assert_eq!(lane_middles(2.0, [0, 0]), [0.0]);
        assert!(wheel_tracks(0.8, 2.0, [0, 0]) > 0.99);
        // Tagged lanes win.
        assert_eq!(lane_count(5.0, [2, 2]), 4);
    }

    /// The shape parameters of a hole, for comparing them.
    fn shape_key(p: &Pothole) -> [u32; 7] {
        [p.r, p.aspect, p.angle, p.depth, p.warp, p.freq, p.noise].map(f32::to_bits)
    }

    #[test]
    fn potholes_vary_in_outline_size_depth_and_kind() {
        // A horrible residential street, 1 km (a tile's length), 6 roads
        // of different seeds: no two holes of the same shape.
        let mut keys = std::collections::BTreeSet::new();
        let mut n = 0;
        let mut all = Vec::new();
        for seed in [0, 1, 2, 17, 40, 63] {
            for p in potholes_seeded(7, 3.0, 143, false, seed) {
                n += 1;
                keys.insert(shape_key(&p));
                all.push(p);
            }
        }
        assert!(n > 200, "{n}");
        assert_eq!(keys.len(), n, "two holes of the same shape");
        // Two roads do not have their holes at the same places.
        let at = |seed| -> Vec<(i32, i32)> {
            potholes_seeded(7, 3.0, 143, false, seed)
                .iter()
                .map(|p| ((p.v * 10.0) as i32, (p.u * 10.0) as i32))
                .collect()
        };
        assert_ne!(at(1), at(2));
        let share = |f: &dyn Fn(&Pothole) -> bool| {
            all.iter().filter(|p| f(p)).count() as f32 / all.len() as f32
        };
        // Sizes 0.2 … 1.2 m across, depths 1 … 8 cm, all angles.
        let (rmin, rmax) = all
            .iter()
            .fold((9.0f32, 0.0f32), |a, p| (a.0.min(p.r), a.1.max(p.r)));
        assert!(rmin < 0.13 && rmax > 0.5, "{rmin} {rmax}");
        assert!(all.iter().all(|p| (0.01..=0.0801).contains(&p.depth)));
        let (dmin, dmax) = all.iter().fold((1.0f32, 0.0f32), |a, p| {
            (a.0.min(p.depth), a.1.max(p.depth))
        });
        assert!(dmin < 0.035 && dmax > 0.065, "{dmin} {dmax}");
        // Elongated along the wheel's way, clustered, filled, edge breaks.
        let long = share(&|p| !p.edge && p.aspect > 1.5 && p.angle.abs() < 0.3);
        assert!((0.1..0.4).contains(&long), "{long}");
        let round = share(&|p| p.aspect < 1.5);
        assert!(round > 0.3, "{round}");
        let clustered = share(&|p| p.lobes[0].is_some());
        assert!((0.15..0.5).contains(&clustered), "{clustered}");
        assert!(all.iter().any(|p| p.lobes[1].is_some()));
        let filled = share(&|p| p.filled);
        assert!((0.1..0.35).contains(&filled), "{filled}");
        let edge = share(&|p| p.edge);
        assert!((0.25..0.45).contains(&edge), "{edge}");
        // The outline is irregular: q along a circle around the centre
        // varies by more than the ellipse would.
        let p = all
            .iter()
            .find(|p| p.aspect < 1.1 && p.warp > 0.35)
            .unwrap();
        let qs: Vec<f32> = (0..32)
            .map(|i| {
                let a = i as f32 * std::f32::consts::TAU / 32.0;
                p.shape_q(0, p.u + p.r * a.sin(), p.v + p.r * a.cos())
            })
            .collect();
        let (lo, hi) = qs
            .iter()
            .fold((9.0f32, 0.0f32), |a, &q| (a.0.min(q), a.1.max(q)));
        assert!(hi - lo > 0.12, "{lo} {hi}");
        // Better roads: smaller, shallower holes, more of them filled.
        let mean = |g: u8, f: &dyn Fn(&Pothole) -> f32| {
            let h = potholes(g, 3.0, 20_000, false);
            h.iter().map(f).sum::<f32>() / h.len() as f32
        };
        assert!(mean(4, &|p| p.r) < mean(7, &|p| p.r));
        assert!(mean(4, &|p| p.depth) < mean(7, &|p| p.depth));
        assert!(
            mean(4, &|p| f32::from(u8::from(p.filled)))
                > mean(7, &|p| f32::from(u8::from(p.filled)))
        );
        // Hollows on gravel are never filled.
        let d = wear_density(6, 1.0);
        let gravel = HoleSpec::of(&d, 1.5, 3.0, false, fillable(RoadSurface::Gravel));
        assert!(
            (0..5000).all(|k| pothole_in_cell(k, 0, 3, 3.0, &gravel).is_none_or(|p| !p.filled))
        );
    }

    #[test]
    fn crack_kinds_follow_the_condition() {
        // Per grade over 55 km: the kinds of cracks.
        #[derive(Default, Debug)]
        struct Kinds {
            full: usize,
            middle: usize,
            from_edge: usize,
            branched: usize,
            sealed: usize,
            open: usize,
            edge_cracks: usize,
            wheel_track: usize,
            blocks: usize,
            alligator_small: usize,
            alligator_large: usize,
        }
        let kinds = |g: u8| {
            let d = wear_density(g, 1.0);
            let half = 3.0;
            let mut c = Kinds::default();
            for k in 0..5000 {
                if let Some(t) = transverse_in_cell(k, 5, half, d.transverse, d.sealed, d.branch) {
                    if t.sealed {
                        c.sealed += 1;
                    } else {
                        c.open += 1;
                    }
                    if t.span == (-half, half) {
                        c.full += 1;
                    } else if (t.span.0 + t.span.1).abs() < 1e-4 {
                        c.middle += 1;
                    } else {
                        c.from_edge += 1;
                    }
                    c.branched += usize::from(t.branch.is_some());
                }
                for slot in 0..LONGITUDINAL_SLOTS {
                    if let Some(l) = longitudinal_in_cell(
                        k,
                        slot,
                        5,
                        half,
                        d.longitudinal,
                        d.sealed,
                        d.edge_cracks,
                    ) {
                        c.edge_cracks += usize::from(l.spurs);
                        c.wheel_track += usize::from(slot >= 3);
                    }
                }
                c.blocks += usize::from(block_in_cell(k, 5, half, d.block, d.t).is_some());
                for side in 0..2 {
                    if let Some(a) = alligator_in_cell(k, side, 5, half, d.alligator, false) {
                        if a.half_w * (a.v.1 - a.v.0) < 1.5 {
                            c.alligator_small += 1;
                        } else {
                            c.alligator_large += 1;
                        }
                    }
                }
            }
            c
        };
        let ex = kinds(1);
        let good = kinds(2);
        let im = kinds(4);
        let bad = kinds(5);
        let horrible = kinds(7);
        // Excellent and good: few cracks, most of them sealed, no block
        // cracking, little alligator cracking.
        assert!(ex.open + ex.sealed < 100, "{ex:?}");
        assert!(good.sealed > good.open, "{good:?}");
        assert_eq!(good.blocks, 0, "{good:?}");
        let fatigue = |c: &Kinds| c.alligator_small + c.alligator_large;
        assert!(fatigue(&good) * 5 < fatigue(&bad), "{good:?} {bad:?}");
        // Intermediate: block cracking and edge cracks start.
        assert!(im.blocks > 0 && im.edge_cracks > good.edge_cracks, "{im:?}");
        // Bad and worse: every kind, mostly open, more branched.
        for c in [&bad, &horrible] {
            assert!(c.full > 0 && c.middle > 0 && c.from_edge > 0, "{c:?}");
            assert!(c.wheel_track > 0 && c.alligator_small > 0 && c.alligator_large > 0);
        }
        assert!(horrible.open > horrible.sealed);
        assert!(horrible.branched > bad.branched && bad.branched > im.branched);
        assert!(horrible.blocks > bad.blocks && horrible.edge_cracks > bad.edge_cracks);
        // The three spans in about their shares.
        let n = (horrible.full + horrible.middle + horrible.from_edge) as f32;
        for x in [horrible.full, horrible.middle, horrible.from_edge] {
            assert!((x as f32 / n - 0.33).abs() < 0.08, "{horrible:?}");
        }
        // Widths vary; sealed bands are wide.
        let d = wear_density(6, 1.0);
        let widths: Vec<f32> = (0..2000)
            .filter_map(|k| transverse_in_cell(k, 0, 3.0, d.transverse, d.sealed, d.branch))
            .filter(|t| !t.sealed)
            .map(|t| t.width)
            .collect();
        let (lo, hi) = widths
            .iter()
            .fold((1.0f32, 0.0f32), |a, &w| (a.0.min(w), a.1.max(w)));
        assert!(lo < 0.004 && hi > 0.009, "{lo} {hi}");
        // Blocks are smaller on worse roads.
        let size = |g: u8| {
            let d = wear_density(g, 1.0);
            let b: Vec<f32> = (0..20_000)
                .filter_map(|k| block_in_cell(k, 0, 3.0, d.block, d.t))
                .map(|b| b.size)
                .collect();
            b.iter().sum::<f32>() / b.len() as f32
        };
        assert!(size(7) < size(4));
    }

    #[test]
    fn the_noise_is_the_shaders() {
        // Corners are the hashes, the middle their mean.
        assert_eq!(vnoise([3.0, 5.0]), unit(hash2(3, 5)));
        assert_eq!(vnoise([-2.0, 7.0]), unit(hash2(-2, 7)));
        let m = vnoise([3.5, 5.5]);
        let mean =
            (unit(hash2(3, 5)) + unit(hash2(4, 5)) + unit(hash2(3, 6)) + unit(hash2(4, 6))) / 4.0;
        assert!((m - mean).abs() < 1e-6);
        for i in 0..1000 {
            let x = vnoise([i as f32 * 0.37 - 100.0, i as f32 * 0.11]);
            assert!((0.0..=1.0).contains(&x));
        }
    }
}
