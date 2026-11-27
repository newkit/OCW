//! Which road vehicles are drawn and how (spec 14 Phase 18, 18A
//! *Performance budgets* and *LOD*, 18E T7): the nearest within the
//! quality level's budget, the near model within [`NEAR_M`], the model's
//! `lod1` within [`LOD1_M`], a painted box with its lamps (the impostor)
//! beyond, nothing past [`FAR_M`] or past the budget. A vehicle keeps its
//! level within a small band ([`HYSTERESIS`]) so it does not flicker at
//! a border.

use crate::quality::QualityLevel;

/// The near model up to here (m; 18A: 150 m).
pub const NEAR_M: f32 = 150.0;
/// The `lod1` model up to here (m; 18A: 600 m), the box beyond.
pub const LOD1_M: f32 = 600.0;
/// Nothing beyond this (m; the 1.5 km of the interest radius).
pub const FAR_M: f32 = 1500.0;
/// The "near" of the budget table: full detail counted up to here (m).
pub const BUDGET_NEAR_M: f32 = 300.0;
/// A level changes only this much past its border (share of the border).
pub const HYSTERESIS: f32 = 0.06;
/// Near vehicles cast shadows up to here (m; 18A: none beyond 100 m).
pub const SHADOW_M: f32 = 100.0;

/// How a vehicle is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Lod {
    /// The near model with its wheels (turning), lamps and moving parts.
    Near,
    /// The model's `lod1`.
    Lod1,
    /// A painted box in the model's size with its lamps on the ends.
    Box,
}

/// The drawing budget of a quality level (18A's table).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    /// Vehicles in full detail (≤ [`BUDGET_NEAR_M`]).
    pub near: usize,
    /// Vehicles in `lod1` or as boxes (≤ [`FAR_M`]).
    pub far: usize,
    /// Real lights (spot lights, no shadows) for the nearest headlights
    /// at night.
    pub spot_lights: usize,
    /// Near vehicles cast shadows.
    pub shadows: bool,
    /// The headlights' cones in the air at night.
    pub cones: bool,
}

/// The budget of `level` (18A: High 60 / 150, Medium 30 / 80, Low 15 / 40,
/// `quest1` 8 / 20).
pub fn budget(level: QualityLevel) -> Budget {
    match level {
        QualityLevel::High => Budget {
            near: 60,
            far: 150,
            spot_lights: 4,
            shadows: true,
            cones: true,
        },
        QualityLevel::Medium | QualityLevel::Vr => Budget {
            near: 30,
            far: 80,
            spot_lights: 2,
            shadows: true,
            cones: true,
        },
        QualityLevel::Low => Budget {
            near: 15,
            far: 40,
            spot_lights: 0,
            shadows: false,
            cones: true,
        },
        QualityLevel::Quest1 => Budget {
            near: 8,
            far: 20,
            spot_lights: 0,
            shadows: false,
            cones: false,
        },
    }
}

/// The parked cars' own budget (spec 18B): as many as the moving
/// vehicles' at each level, without lights, shadows or cones; they are
/// only asked for within 400 m (`ocw_client`'s `PARKED_VIEW_M`).
pub fn parked_budget(level: QualityLevel) -> Budget {
    let b = budget(level);
    Budget {
        near: b.near,
        far: b.far,
        spot_lights: 0,
        shadows: false,
        cones: false,
    }
}

fn level_at(d: f32, before: Option<Lod>) -> Option<Lod> {
    // The border moves away from the current level by the hysteresis.
    let border = |b: f32, inner: Lod| match before {
        Some(l) if l <= inner => b * (1.0 + HYSTERESIS),
        Some(_) => b * (1.0 - HYSTERESIS),
        None => b,
    };
    if d <= border(NEAR_M, Lod::Near) {
        Some(Lod::Near)
    } else if d <= border(LOD1_M, Lod::Lod1) {
        Some(Lod::Lod1)
    } else if d <= border(FAR_M, Lod::Box) {
        Some(Lod::Box)
    } else {
        None
    }
}

/// The level of each vehicle: `vehicles` are (id, distance to the camera
/// in m, the level it had last frame); the result is in the same order,
/// `None` = not drawn. The nearest go first: full detail for at most
/// `budget.near` within [`BUDGET_NEAR_M`] (more near ones get `lod1`),
/// `lod1` or a box for at most `budget.far` more.
pub fn plan(vehicles: &[(u32, f32, Option<Lod>)], budget: Budget) -> Vec<Option<Lod>> {
    let mut order: Vec<usize> = (0..vehicles.len()).collect();
    order.sort_by(|&a, &b| {
        vehicles[a]
            .1
            .total_cmp(&vehicles[b].1)
            .then(vehicles[a].0.cmp(&vehicles[b].0))
    });
    let mut out = vec![None; vehicles.len()];
    let (mut near, mut far) = (0usize, 0usize);
    for i in order {
        let (_, d, before) = vehicles[i];
        let Some(mut lod) = level_at(d, before) else {
            continue;
        };
        if lod == Lod::Near {
            if near < budget.near && d <= BUDGET_NEAR_M {
                near += 1;
                out[i] = Some(lod);
                continue;
            }
            lod = Lod::Lod1;
        }
        if far < budget.far {
            far += 1;
            out[i] = Some(lod);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_by_distance_with_hysteresis() {
        let b = budget(QualityLevel::High);
        let one = |d: f32, before| plan(&[(1, d, before)], b)[0];
        assert_eq!(one(20.0, None), Some(Lod::Near));
        assert_eq!(one(300.0, None), Some(Lod::Lod1));
        assert_eq!(one(900.0, None), Some(Lod::Box));
        assert_eq!(one(1600.0, None), None);
        // Just past the border: keeps its level; well past it: changes.
        assert_eq!(one(155.0, Some(Lod::Near)), Some(Lod::Near));
        assert_eq!(one(170.0, Some(Lod::Near)), Some(Lod::Lod1));
        assert_eq!(one(145.0, Some(Lod::Lod1)), Some(Lod::Lod1));
        assert_eq!(one(130.0, Some(Lod::Lod1)), Some(Lod::Near));
        assert_eq!(one(620.0, Some(Lod::Lod1)), Some(Lod::Lod1));
        assert_eq!(one(580.0, Some(Lod::Box)), Some(Lod::Box));
    }

    #[test]
    fn the_budget_table_is_kept_nearest_first() {
        // 400 vehicles: 200 within 105 m (a jam), 200 from 300 m to 1.3 km.
        let vs: Vec<(u32, f32, Option<Lod>)> = (0..400u32)
            .map(|i| {
                let d = if i < 200 {
                    5.0 + i as f32 * 0.5
                } else {
                    300.0 + (i - 200) as f32 * 5.0
                };
                (i, d, None)
            })
            .collect();
        for (level, near, far) in [
            (QualityLevel::High, 60, 150),
            (QualityLevel::Medium, 30, 80),
            (QualityLevel::Low, 15, 40),
            (QualityLevel::Quest1, 8, 20),
        ] {
            let b = budget(level);
            let p = plan(&vs, b);
            let n = p.iter().filter(|l| **l == Some(Lod::Near)).count();
            let f = p
                .iter()
                .filter(|l| matches!(l, Some(Lod::Lod1 | Lod::Box)))
                .count();
            assert_eq!((n, f), (near, far), "{level:?}");
            // The nearest are the detailed ones; the drawn ones are the
            // nearest.
            assert!(p[..near].iter().all(|l| *l == Some(Lod::Near)));
            let last = p.iter().rposition(Option::is_some).unwrap();
            assert_eq!(last, near + far - 1, "{level:?}");
        }
        // Few vehicles: all drawn.
        let p = plan(&vs[..10], budget(QualityLevel::Low));
        assert!(p.iter().all(|l| *l == Some(Lod::Near)));
    }

    #[test]
    fn spot_lights_and_shadows_only_on_the_higher_levels() {
        assert_eq!(budget(QualityLevel::High).spot_lights, 4);
        assert_eq!(budget(QualityLevel::Low).spot_lights, 0);
        assert!(!budget(QualityLevel::Low).shadows);
        assert!(!budget(QualityLevel::Quest1).cones);
    }
}
