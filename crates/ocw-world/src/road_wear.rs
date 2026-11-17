//! Road damage (spec §4.6 step 6, *Road look*): cracks, crack sealing,
//! alligator areas, repaired patches, worn wheel tracks and potholes on the
//! carriageway, drawn by `road.wesl` from the strip's **wear grade** and
//! the strip frame (across, along in metres) – no textures, no geometry:
//! the road surface riders stand on is not changed (`Track::surface_height`
//! and the road meshes' positions are the same for every grade).
//!
//! The grade (1 … 7, 0: none drawn) comes from the road class and the
//! surface ([`wear_grade`]): main roads are kept up (few cracks, mostly
//! sealed, no potholes but at the very edge), minor roads, service roads and
//! tracks are worn, and OSM `smoothness` (intermediate or worse, which makes
//! the surface [`RoadSurface::WornAsphalt`]) adds two grades. Each grade has
//! its chances ([`wear_density`], scaled by the quality preset's share);
//! where a feature lies is a hash of the cell along the road, so it is the
//! same on every machine, in both apps and across tile borders (the along
//! coordinate runs on over the tiles and through a junction patch on the
//! main road).
//!
//! This module is the shader's twin: the placement functions below compute
//! exactly what `road.wesl` draws (same hashes, same constants), so the
//! tests can check the density per class and that potholes stay off the
//! middle of main roads.

use crate::RoadSurface;
use crate::graph::RoadClass;
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
pub const MAX_POTHOLE_R: f32 = 0.45;
/// Half the distance between the wheel ruts of a track (m; the shader's
/// track look).
pub const RUT_M: f32 = 0.8;

/// How worn a strip draws: 1 (motorway-like, kept up) … 7 (broken), 0 for
/// parts without a strip frame (the corner fans of `road_corners`).
pub fn wear_grade(class: RoadClass, surface: RoadSurface) -> u8 {
    let base: u8 = match class {
        RoadClass::Trunk => 1,
        RoadClass::Primary | RoadClass::Secondary => 2,
        RoadClass::Tertiary | RoadClass::Residential | RoadClass::Cycleway => 3,
        RoadClass::Unclassified | RoadClass::Service | RoadClass::Path => 4,
        RoadClass::Track => 5,
    };
    if surface == RoadSurface::WornAsphalt {
        (base + 2).max(5).min(MAX_GRADE)
    } else {
        base
    }
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
}

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
        transverse: (0.04 + 0.5 * t) * s,
        longitudinal: (0.03 + 0.45 * t) * s,
        alligator: 0.4 * t.powf(1.5) * s,
        pothole: 0.25 * t * t * s,
        patch: (0.04 + 0.3 * t) * s,
        sealed: 0.75 - 0.6 * t,
        pothole_zone_m: if t <= 1.0 / 3.0 {
            0.7
        } else {
            0.7 + 4.0 * (t - 1.0 / 3.0)
        },
        tracks: 0.05 + 0.12 * t,
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
    let r = 0.12 + (MAX_POTHOLE_R - 0.12) * a * a;
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

    #[test]
    fn grades_follow_class_and_smoothness() {
        for c in ALL_CLASSES {
            let g = wear_grade(c, RoadSurface::Asphalt);
            assert!((1..=MAX_GRADE).contains(&g), "{c:?}");
            // OSM smoothness intermediate or worse: more worn.
            assert!(wear_grade(c, RoadSurface::WornAsphalt) > g, "{c:?}");
            assert!(wear_grade(c, RoadSurface::WornAsphalt) >= 5);
        }
        let g = |c| wear_grade(c, RoadSurface::Asphalt);
        assert!(g(RoadClass::Trunk) < g(RoadClass::Primary));
        assert!(g(RoadClass::Primary) < g(RoadClass::Residential));
        assert!(g(RoadClass::Residential) < g(RoadClass::Unclassified));
        assert!(g(RoadClass::Unclassified) < g(RoadClass::Track));
        assert_eq!(
            wear_grade(RoadClass::Track, RoadSurface::WornAsphalt),
            MAX_GRADE
        );
        assert_eq!(wear_density(0, 1.0), WearDensity::default());
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
        // Main roads: no potholes on a trunk road, no alligator areas.
        assert_eq!(wear_density(1, 1.0).pothole, 0.0);
        assert_eq!(wear_density(1, 1.0).alligator, 0.0);
        let (hi, lo) = (wear_density(5, 1.0), wear_density(5, 0.5));
        assert!((lo.pothole - 0.5 * hi.pothole).abs() < 1e-6);
        assert_eq!(lo.sealed, hi.sealed);
        assert_eq!(wear_density(5, 0.0).transverse, 0.0);
    }

    #[test]
    fn placement_is_deterministic_and_counts_follow_the_class() {
        // The same cell gives the same hole, whatever else was asked.
        let a = potholes(6, 2.5, 2000, false);
        let b = potholes(6, 2.5, 2000, false);
        assert_eq!(a, b);
        let count = |c: RoadClass, s: RoadSurface| {
            let half = c.default_width_m() as f32 / 2.0;
            potholes(wear_grade(c, s), half, 5000, c == RoadClass::Track).len()
        };
        let primary = count(RoadClass::Primary, RoadSurface::Asphalt);
        let residential = count(RoadClass::Residential, RoadSurface::Asphalt);
        let worn = count(RoadClass::Residential, RoadSurface::WornAsphalt);
        let track = count(RoadClass::Track, RoadSurface::Asphalt);
        assert_eq!(count(RoadClass::Trunk, RoadSurface::Asphalt), 0);
        // 35 km of road: a few on a primary road, many on a track.
        assert!(primary > 0 && primary < 120, "{primary}");
        assert!(residential > 3 * primary, "{residential} vs {primary}");
        assert!(worn > residential, "{worn} vs {residential}");
        assert!(track > residential, "{track} vs {residential}");
        // Cracks too: a residential road has more than a primary one.
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
    }

    #[test]
    fn potholes_stay_off_the_middle_of_main_roads() {
        for c in [RoadClass::Primary, RoadClass::Secondary] {
            let half = c.default_width_m() as f32 / 2.0;
            let g = wear_grade(c, RoadSurface::Asphalt);
            let zone = wear_density(g, 1.0).pothole_zone_m;
            let holes = potholes(g, half, 50_000, false);
            assert!(!holes.is_empty(), "{c:?}");
            for p in holes {
                assert!(p.u.abs() >= half - zone, "{c:?}: {p:?}");
                // The whole hole lies in the outer metre.
                assert!(p.u.abs() - p.r >= half - 1.0 - MAX_POTHOLE_R, "{p:?}");
            }
        }
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
}
