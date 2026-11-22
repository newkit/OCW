//! The road damage in the ride (spec Phase 8 Q12 (c), v5.35): a bad
//! surface rolls harder, and riding over a pothole, an edge break, an open
//! crack or into an alligator area is a **bump** – a small loss of speed
//! here, and on the client a short resistance kick on the trainer, a small
//! camera shake and a sound (`ocw-client::road_bumps`).
//!
//! Server-authoritative and deterministic: the damage comes from
//! [`ocw_world::road_feel`], the same pure data the renderer draws (the
//! strip's wear grade and the hash-placed features in its frame), at the
//! wheel's place in the world (the track's centre line moved by the
//! rider's lateral offset). [`crate::ride::tick_in`] applies it, so the
//! game server and the client's prediction (and its extrapolation) ride
//! exactly the same; a bump changes only the speed, never the lateral
//! place (no sideways step, nothing to snap).
//!
//! The rolling resistance goes through the shared, additive hook
//! [`crate::physics::Environment::with_rolling_extra`] (other road effects,
//! such as wet leaves, add their own share). Nothing on grades 1 – 2
//! (excellent, good): `road_feel` does not keep those strips.

use ocw_world::road_feel::{Feature, FeatureKind};
use ocw_world::{RoadSurface, Track};

use crate::physics::RiderState;

/// More rolling resistance per wear grade (index 0 … 7; a share on top of
/// the surface's, on paved roads): intermediate +6 %, bad +15 %, very bad
/// +30 %, horrible and worse +50 % – Crr 0.004 → 0.006 on a horrible road,
/// still well below gravel's 2.5 ×. Grades 1 – 2 nothing.
pub const ROLLING_EXTRA: [f64; 8] = [0.0, 0.0, 0.0, 0.02, 0.06, 0.15, 0.30, 0.50];
/// Unpaved roads (gravel, dirt) already roll at gravel's factor: they get
/// this share of their grade's extra.
pub const UNPAVED_SHARE: f64 = 0.5;
/// A bump of strength 1 at speed costs this much speed (m/s) …
pub const BUMP_SPEED_LOSS_MPS: f64 = 0.35;
/// … in full from this speed on (m/s), less below (a slow wheel rolls
/// into a hole instead of hitting its far edge).
pub const BUMP_FULL_SPEED_MPS: f64 = 8.0;
/// A tick's wheel movement above this (m) is a jump (a teleport, a
/// snapped correction), not a ride over the road: no bump.
pub const MAX_STEP_M: f64 = 2.0;

/// What kind of damage a bump came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BumpKind {
    Pothole,
    EdgeBreak,
    Crack,
    Alligator,
}

/// A bump over road damage.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoadBump {
    pub kind: BumpKind,
    /// 0 … 1: a large pothole at speed 0.8, a small one 0.35, an open
    /// crack 0.12, into an alligator area 0.18.
    pub strength: f64,
    /// The feature's id (the same each time it is met).
    pub id: u64,
}

/// Strength of a crack and of riding into an alligator area.
pub const CRACK_STRENGTH: f64 = 0.12;
pub const ALLIGATOR_STRENGTH: f64 = 0.18;
/// A pothole's strength from the smallest (15 cm radius) to the largest
/// (60 cm).
pub const POTHOLE_STRENGTH: (f64, f64) = (0.35, 0.8);
/// An edge break: the wheel meets only a part of it.
pub const EDGE_BREAK_SHARE: f64 = 0.9;
/// Holes on gravel and dirt are softer hollows, on paving sunken spots.
pub const SOFT_HOLE_SHARE: f64 = 0.6;

/// The extra rolling resistance (a share) of `grade` on `surface`.
pub fn rolling_extra(grade: u8, surface: RoadSurface) -> f64 {
    let e = ROLLING_EXTRA[usize::from(grade.min(7))];
    match surface {
        RoadSurface::Gravel | RoadSurface::Dirt => e * UNPAVED_SHARE,
        _ => e,
    }
}

/// The wheel's place (x, z, home metres): the centre line at the rider's
/// distance moved by the lateral offset.
pub fn wheel(track: &dyn Track, state: &RiderState) -> [f64; 2] {
    let p = track
        .sample(state.distance)
        .offset_position(state.lateral_offset);
    [p[0], p[2]]
}

/// The extra rolling resistance under the rider (0 on good roads, off the
/// road, without damage data).
pub fn rolling_extra_at(track: &dyn Track, state: &RiderState) -> f64 {
    let Some(map) = track.road_feel() else {
        return 0.0;
    };
    map.at(wheel(track, state))
        .map_or(0.0, |f| rolling_extra(f.grade, f.surface))
}

/// The bump of a feature.
pub fn bump_of(f: &Feature) -> RoadBump {
    let soft = match f.surface {
        RoadSurface::Asphalt | RoadSurface::WornAsphalt => 1.0,
        _ => SOFT_HOLE_SHARE,
    };
    let hole = |r: f32| {
        let x = ((f64::from(r) - 0.15) / 0.45).clamp(0.0, 1.0);
        (POTHOLE_STRENGTH.0 + (POTHOLE_STRENGTH.1 - POTHOLE_STRENGTH.0) * x) * soft
    };
    let (kind, strength) = match f.kind {
        FeatureKind::Pothole { r } => (BumpKind::Pothole, hole(r)),
        FeatureKind::EdgeBreak { r } => (BumpKind::EdgeBreak, hole(r) * EDGE_BREAK_SHARE),
        FeatureKind::Crack => (BumpKind::Crack, CRACK_STRENGTH),
        FeatureKind::Alligator => (BumpKind::Alligator, ALLIGATOR_STRENGTH),
    };
    RoadBump {
        kind,
        strength,
        id: f.id,
    }
}

/// The speed a bump costs at `speed` (m/s).
pub fn speed_loss(b: &RoadBump, speed: f64) -> f64 {
    BUMP_SPEED_LOSS_MPS * b.strength * (speed.max(0.0) / BUMP_FULL_SPEED_MPS).min(1.0)
}

/// The strongest bump of a wheel moving from `from` to `to` (home metres)
/// on `track`'s damage; `None` on good roads, without damage data or for
/// a jump.
pub fn crossed_between(track: &dyn Track, from: [f64; 2], to: [f64; 2]) -> Option<RoadBump> {
    let map = track.road_feel()?;
    let step = ((to[0] - from[0]).powi(2) + (to[1] - from[1]).powi(2)).sqrt();
    if step > MAX_STEP_M || step == 0.0 {
        return None;
    }
    map.crossed(from, to)
        .iter()
        .map(bump_of)
        .max_by(|a, b| a.strength.total_cmp(&b.strength))
}

/// The strongest bump of a rider going from `before` to `after` (one tick).
pub fn crossed(track: &dyn Track, before: &RiderState, after: &RiderState) -> Option<RoadBump> {
    track.road_feel()?;
    crossed_between(track, wheel(track, before), wheel(track, after))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feature(kind: FeatureKind, surface: RoadSurface) -> Feature {
        Feature {
            kind,
            surface,
            grade: 5,
            id: 7,
        }
    }

    #[test]
    fn the_rolling_resistance_grows_with_the_grade_from_intermediate_on() {
        let a = RoadSurface::Asphalt;
        assert_eq!(rolling_extra(0, a), 0.0);
        assert_eq!(rolling_extra(1, a), 0.0);
        assert_eq!(rolling_extra(2, a), 0.0);
        for g in 3..7 {
            assert!(rolling_extra(g + 1, a) > rolling_extra(g, a), "{g}");
        }
        assert_eq!(rolling_extra(4, a), 0.06);
        assert_eq!(rolling_extra(5, a), 0.15);
        assert_eq!(rolling_extra(7, a), 0.5);
        assert_eq!(rolling_extra(9, a), 0.5);
        // Unpaved: half of it (gravel's own factor already counts).
        assert_eq!(rolling_extra(5, RoadSurface::Gravel), 0.075);
        assert_eq!(rolling_extra(5, RoadSurface::Paving), 0.15);
        // Modest: a horrible road below gravel.
        assert!(1.0 + rolling_extra(7, a) < ocw_world::Surface::Gravel.rolling_factor());
    }

    #[test]
    fn bumps_per_damage_kind() {
        let a = RoadSurface::Asphalt;
        let small = bump_of(&feature(FeatureKind::Pothole { r: 0.15 }, a));
        let large = bump_of(&feature(FeatureKind::Pothole { r: 0.6 }, a));
        let edge = bump_of(&feature(FeatureKind::EdgeBreak { r: 0.6 }, a));
        let crack = bump_of(&feature(FeatureKind::Crack, a));
        let alligator = bump_of(&feature(FeatureKind::Alligator, a));
        let hollow = bump_of(&feature(
            FeatureKind::Pothole { r: 0.6 },
            RoadSurface::Gravel,
        ));
        assert_eq!(small.kind, BumpKind::Pothole);
        assert!((small.strength - 0.35).abs() < 1e-6);
        assert!((large.strength - 0.8).abs() < 1e-12);
        assert_eq!(edge.kind, BumpKind::EdgeBreak);
        assert!(edge.strength < large.strength && edge.strength > small.strength);
        assert_eq!((crack.kind, crack.strength), (BumpKind::Crack, 0.12));
        assert_eq!(alligator.kind, BumpKind::Alligator);
        assert!(crack.strength < alligator.strength && alligator.strength < small.strength);
        assert!(hollow.strength < large.strength);
        assert_eq!(large.id, 7);
        // The speed it costs: at most 0.28 m/s (1 km/h) for the largest
        // hole at speed, less when slow, never more than the speed.
        assert!((speed_loss(&large, 12.0) - 0.28).abs() < 1e-12);
        assert!(speed_loss(&large, 4.0) < speed_loss(&large, 8.0));
        assert_eq!(speed_loss(&large, 0.0), 0.0);
        assert!(speed_loss(&crack, 10.0) < 0.05);
    }

    #[test]
    fn nothing_without_damage_data() {
        let route = ocw_world::Route::from_points(
            "s",
            "S",
            vec![[0.0, 0.0, 0.0], [1000.0, 0.0, 0.0]],
            false,
            6.0,
        )
        .unwrap();
        let a = RiderState {
            distance: 10.0,
            ..Default::default()
        };
        let b = RiderState {
            distance: 10.3,
            ..a
        };
        assert_eq!(rolling_extra_at(&route, &a), 0.0);
        assert_eq!(crossed(&route, &a, &b), None);
    }
}

#[cfg(test)]
mod ride_tests {
    use super::*;
    use crate::physics::{self, Environment};
    use crate::ride::{self, Controls};
    use ocw_core::RiderProfile;
    use ocw_world::RidePath;
    use ocw_world::graph::RoadClass;
    use ocw_world::road_condition::Smoothness;
    use ocw_world::road_feel::fixture;

    /// 60 s at 220 W along a 980 m road in `sm` at `lateral` m from the
    /// middle: the final state and the bumps.
    fn ride(sm: Option<Smoothness>, lateral: f64) -> (RiderState, Vec<RoadBump>) {
        let net = fixture::network(RoadClass::Residential, sm, 6.0, 980.0);
        let path = RidePath::start(net, 0, 5.0, true, 0.0, 0);
        let profile = RiderProfile::default();
        let mut s = RiderState {
            speed: 8.0,
            lateral_offset: lateral,
            ..Default::default()
        };
        let mut scraping = false;
        let mut bumps = Vec::new();
        for _ in 0..(60 * 50) {
            let before = s;
            ride::tick(
                &mut s,
                &profile,
                220.0,
                Controls {
                    steering: physics::Steering::Auto(lateral),
                    ..Default::default()
                },
                &mut scraping,
                &path,
                1.0,
                ocw_core::SIM_DT,
            );
            if let Some(b) = crossed(&path, &before, &s) {
                bumps.push(b);
            }
            // A bump never moves the rider sideways.
            assert!((s.lateral_offset - before.lateral_offset).abs() < 1e-9);
        }
        (s, bumps)
    }

    #[test]
    fn the_rolling_resistance_per_grade_reaches_the_physics() {
        let p = RiderProfile::default();
        let base = physics::resistance(&p, &Environment::with_grade(0.0), 9.0).rolling;
        for g in 0..=7u8 {
            let env = Environment::with_grade(0.0)
                .with_rolling_extra(rolling_extra(g, ocw_world::RoadSurface::Asphalt));
            let r = physics::resistance(&p, &env, 9.0).rolling;
            let want = 1.0 + ROLLING_EXTRA[usize::from(g)];
            assert!((r / base - want).abs() < 1e-9, "{g}");
        }
        // Extras add up (wet leaves and damage both): 1 + 0.15 + 0.1.
        let env = Environment::with_grade(0.0)
            .with_rolling_extra(0.15)
            .with_rolling_extra(0.1);
        let r = physics::resistance(&p, &env, 9.0).rolling;
        assert!((r / base - 1.25).abs() < 1e-9);
        // Added to the weather's factor (a wet road, wet leaves), not
        // multiplied with it: 1.06 + 0.15.
        let wet = Environment::with_grade(0.0)
            .with_surface(ocw_world::Surface::Asphalt)
            .in_weather(0.0, ocw_core::AIR_DENSITY, 1.06)
            .with_rolling_extra(0.15);
        let r = physics::resistance(&p, &wet, 9.0).rolling;
        assert!((r / base - 1.21).abs() < 1e-9, "{}", r / base);
        // On gravel on top of its factor.
        let gravel = Environment::with_grade(0.0)
            .with_surface(ocw_world::Surface::Gravel)
            .with_rolling_extra(0.1);
        assert!(physics::resistance(&p, &gravel, 9.0).rolling > 2.5 * base);
    }

    #[test]
    fn a_bad_road_is_slower_and_bumps_good_roads_ride_as_before() {
        let (clean, none) = ride(None, -1.5);
        assert!(none.is_empty());
        // Good and excellent: exactly the ride without damage data.
        for sm in [Smoothness::Excellent, Smoothness::Good] {
            let (s, bumps) = ride(Some(sm), -1.5);
            assert_eq!(s, clean, "{sm:?}");
            assert!(bumps.is_empty());
        }
        let (bad, bad_bumps) = ride(Some(Smoothness::Bad), -1.5);
        let (horrible, bumps) = ride(Some(Smoothness::Horrible), -1.5);
        assert!(
            bad.distance < clean.distance - 1.0,
            "{} vs {}",
            bad.distance,
            clean.distance
        );
        assert!(horrible.distance < bad.distance);
        assert!(!bumps.is_empty() && bumps.len() > bad_bumps.len());
        // Modest: a horrible road costs well under 10 % of the distance.
        assert!(horrible.distance > 0.9 * clean.distance);
        assert!(bumps.iter().any(|b| b.kind == BumpKind::Crack));
        // Riding the same line again meets the same features.
        let (_, again) = ride(Some(Smoothness::Horrible), -1.5);
        assert_eq!(again, bumps);
    }

    #[test]
    fn untagged_roads_are_felt_little() {
        // An untagged residential street (estimated good): nothing at all;
        // an untagged track (grade 3, gravel) only 1 % more rolling.
        let (clean, _) = ride(None, -1.0);
        let (s, bumps) = ride(Some(Smoothness::Unknown), -1.0);
        assert_eq!(s, clean);
        assert!(bumps.is_empty());
        assert!((rolling_extra(3, ocw_world::RoadSurface::Gravel) - 0.01).abs() < 1e-12);
    }
}
