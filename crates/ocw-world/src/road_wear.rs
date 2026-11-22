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
//! roads (spec Phase 8 Q14 (b), v5.33: main roads excellent, the other
//! paved classes good, tracks a little worse). Each grade has its chances
//! ([`wear_density`], scaled by the quality preset's share); main roads
//! keep their potholes within [`MAIN_ROAD_ZONE_M`] of the edge. Where a
//! feature lies is a hash of the cell along the road, so it is the same on
//! every machine, in both apps and across tile borders (the along
//! coordinate runs on over the tiles and through a junction patch on the
//! main road).
//!
//! This module is the shader's twin: the placement functions below compute
//! exactly what `road.wesl` draws (same hashes, same constants), so the
//! tests can check the density per smoothness and class and that potholes
//! stay off the middle of main roads.

use crate::RoadSurface;
use crate::graph::RoadClass;
use crate::road_condition::{RoadCondition, Smoothness};
use crate::road_dress::{hash2, unit};

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
/// (b), v5.33: untagged roads are drawn **cleaner and plainer** than
/// before, so that mostly the tagged roads show damage): from `tracktype`
/// (grade 1 … 5, 0: none) where it is tagged, else from the class – main
/// roads (trunk, primary, secondary) excellent, every other paved class
/// good, tracks between good and intermediate – and the surface (worn
/// asphalt, i.e. a `tracktype=grade1` track, at least 3). Before v5.33:
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

/// The estimate before Phase 8 Q14 (b) (v4.75 – v5.32), kept for the
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

/// The chances of one grade (`road.wesl`'s `damage` and `repair` tables).
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
}

/// The loose stones every unpaved road has, on top of its grade's.
pub const UNPAVED_STONES: f32 = 0.06;

/// The chances of `grade` with the quality preset's `share` (0 … 1) of the
/// features (chances only; the sealed share, the pothole zone and the
/// wheel tracks are the grade's). Grade 0 draws nothing.
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

/// A pothole in the strip frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pothole {
    /// Centre across (m from the road's middle, + right).
    pub u: f32,
    /// Centre along (m).
    pub v: f32,
    /// Radius (m).
    pub r: f32,
    /// An edge break: the hole bites into the road edge.
    pub edge: bool,
}

/// The pothole of cell `k` on `side` (0 left, 1 right) of a road of half
/// width `half`, for a chance per cell (already with the surface factor);
/// `track`: in the wheel ruts. The shader's `pothole_in`.
pub fn pothole_in_cell(
    k: i32,
    side: i32,
    half: f32,
    chance: f32,
    zone_m: f32,
    track: bool,
) -> Option<Pothole> {
    if unit(hash2(k, 211 + side)) >= chance {
        return None;
    }
    let a = unit(hash2(k, 223 + side));
    let r = 0.15 + (MAX_POTHOLE_R - 0.15) * a * a.sqrt();
    let v = (k as f32 + 0.2 + 0.6 * unit(hash2(k, 227 + side))) * POTHOLE_CELL_M;
    let sgn = if side == 0 { -1.0 } else { 1.0 };
    let e = unit(hash2(k, 233 + side));
    if track {
        let u = sgn * (RUT_M + 0.2 * (e - 0.5));
        return Some(Pothole {
            u,
            v,
            r: r.min(0.35),
            edge: false,
        });
    }
    if unit(hash2(k, 229 + side)) < 0.35 {
        return Some(Pothole {
            u: sgn * (half - 0.25 * r),
            v,
            r,
            edge: true,
        });
    }
    let span = (zone_m.min(half - r) - r - 0.1).max(0.0);
    Some(Pothole {
        u: sgn * (half - r - 0.1 - span * e * e),
        v,
        r,
        edge: false,
    })
}

/// An alligator (fatigue cracking) area: an ellipse in the strip frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Alligator {
    pub u: f32,
    /// Start and end along (m).
    pub v: (f32, f32),
    /// Half width across (m).
    pub half_w: f32,
}

/// The alligator area of cell `k` on `side`: in the outer wheel track (the
/// ruts of a track). The shader's `alligator_in`.
pub fn alligator_in_cell(
    k: i32,
    side: i32,
    half: f32,
    chance: f32,
    track: bool,
) -> Option<Alligator> {
    if unit(hash2(k, 307 + side)) >= chance {
        return None;
    }
    let start = (k as f32 + 0.1 + 0.5 * unit(hash2(k, 311 + side))) * ALLIGATOR_CELL_M;
    let len = 2.0 + 5.0 * unit(hash2(k, 313 + side));
    let sgn = if side == 0 { -1.0 } else { 1.0 };
    let at = if track { RUT_M } else { (half - 0.9).max(0.3) };
    Some(Alligator {
        u: sgn * at,
        v: (start, start + len),
        half_w: (0.25 + 0.35 * unit(hash2(k, 317 + side))).min(half * 0.5),
    })
}

/// A longitudinal crack: its line across and its extent along.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Longitudinal {
    /// Mean across (m; the line wanders ± 8 cm around it).
    pub u: f32,
    pub v: (f32, f32),
    pub sealed: bool,
}

/// The longitudinal crack of cell `k` in `slot` (0 the middle joint, 1 the
/// left edge zone, 2 the right). The shader's `longitudinal_in`.
pub fn longitudinal_in_cell(
    k: i32,
    slot: i32,
    half: f32,
    chance: f32,
    sealed: f32,
) -> Option<Longitudinal> {
    if unit(hash2(k, 401 + slot)) >= chance {
        return None;
    }
    let start = k as f32 * LONGITUDINAL_CELL_M + 0.5 + 6.0 * unit(hash2(k, 409 + slot));
    let len = 2.0 + 7.0 * unit(hash2(k, 419 + slot));
    let o = unit(hash2(k, 421 + slot));
    let u = match slot {
        0 => 0.08 + 0.12 * o,
        1 => -(half - 0.3 - 0.5 * o),
        _ => half - 0.3 - 0.5 * o,
    };
    Some(Longitudinal {
        u,
        v: (start, start + len),
        sealed: unit(hash2(k, 431 + slot)) < sealed,
    })
}

/// A transverse crack across the road (or its sealing line).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transverse {
    /// Where it crosses the middle of the road, along (m; the line wanders
    /// [`transverse_v_at`] across).
    pub v: f32,
    /// How far either side of the middle it reaches (m).
    pub reach: f32,
    pub sealed: bool,
    /// The hash of the cell (the line's wander).
    pub phase: f32,
}

/// The transverse crack of cell `k` ([`TRANSVERSE_CELL_M`]) of a road of
/// half width `half`, for a chance per cell. The shader's `transverse`.
pub fn transverse_in_cell(k: i32, half: f32, chance: f32, sealed: f32) -> Option<Transverse> {
    let h = unit(hash2(k, 101));
    if h >= chance {
        return None;
    }
    Some(Transverse {
        v: (k as f32 + 0.15 + 0.7 * unit(hash2(k, 103))) * TRANSVERSE_CELL_M,
        reach: half * (0.4 + 0.6 * unit(hash2(k, 107))),
        sealed: unit(hash2(k, 109)) < sealed,
        phase: h,
    })
}

/// Where a transverse crack lies along at `u` across (m): the shader's
/// wandering line.
pub fn transverse_v_at(t: &Transverse, u: f32) -> f32 {
    t.v + 0.15 * (u * 1.7 + t.phase * 40.0).sin() + 0.04 * (u * 6.1).sin()
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

    fn potholes(grade: u8, half: f32, cells: i32, track: bool) -> Vec<Pothole> {
        let d = wear_density(grade, 1.0);
        (0..cells)
            .flat_map(|k| (0..2).map(move |s| (k, s)))
            .filter_map(|(k, s)| pothole_in_cell(k, s, half, d.pothole, d.pothole_zone_m, track))
            .collect()
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
            if let Some(t) = transverse_in_cell(k, 3.0, 1.0, 0.3) {
                n += 1;
                let c0 = k as f32 * TRANSVERSE_CELL_M;
                for u in [-3.0, 0.0, 3.0] {
                    let v = transverse_v_at(&t, u);
                    assert!(v >= c0 && v <= c0 + TRANSVERSE_CELL_M, "{t:?}");
                }
                assert!(t.reach <= 3.0 && t.reach >= 1.2);
            }
        }
        assert_eq!(n, 1000);
        assert!(transverse_in_cell(3, 3.0, 0.0, 0.0).is_none());
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
                .flat_map(|k| (0..3).map(move |s| (k, s)))
                .filter(|&(k, s)| {
                    longitudinal_in_cell(k, s, 3.0, d.longitudinal, d.sealed).is_some()
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
                let holes: Vec<Pothole> = (0..20_000)
                    .flat_map(|k| (0..2).map(move |s| (k, s)))
                    .filter_map(|(k, s)| pothole_in_cell(k, s, half, d.pothole, zone, false))
                    .collect();
                for p in &holes {
                    assert!(p.u.abs() >= half - zone, "{c:?} {sm:?}: {p:?}");
                    assert!(p.u.abs() - p.r >= half - MAIN_ROAD_ZONE_M - MAX_POTHOLE_R);
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
        }
        // Tracks: in the ruts.
        for p in potholes(5, 1.5, 20_000, true) {
            assert!((p.u.abs() - RUT_M).abs() <= 0.1 + 1e-4, "{p:?}");
        }
    }

    #[test]
    fn features_stay_in_their_cells() {
        // The shader looks only at the pixel's own cell.
        for k in -500..500 {
            for s in 0..2 {
                if let Some(p) = pothole_in_cell(k, s, 3.0, 1.0, 3.0, false) {
                    let c0 = k as f32 * POTHOLE_CELL_M;
                    assert!(p.v - p.r >= c0 && p.v + p.r <= c0 + POTHOLE_CELL_M, "{p:?}");
                }
                if let Some(a) = alligator_in_cell(k, s, 3.0, 1.0, false) {
                    let c0 = k as f32 * ALLIGATOR_CELL_M;
                    assert!(a.v.0 >= c0 && a.v.1 <= c0 + ALLIGATOR_CELL_M, "{a:?}");
                }
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
            for slot in 0..3 {
                if let Some(l) = longitudinal_in_cell(k, slot, 3.0, 1.0, 0.5) {
                    let c0 = k as f32 * LONGITUDINAL_CELL_M;
                    assert!(l.v.0 >= c0 && l.v.1 <= c0 + LONGITUDINAL_CELL_M, "{l:?}");
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
}
