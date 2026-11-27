//! The airways over our worlds (spec 18C *Airliners at altitude*: "a
//! handful of airways across the region, from a small hand-made table per
//! region"): the main flows of upper-airspace traffic, approximate – good
//! for the sky seen from a bike, not for navigation. A world elsewhere
//! gets two crossing airways through its centre.

/// An airway: its points (lat, lon) and the peak flights per hour in each
/// direction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Airway {
    pub name: &'static str,
    pub points: &'static [(f64, f64)],
    pub per_hour: f64,
}

/// The table (checked against the picture of the region's live traffic,
/// 18D, November 2026: busy over the Rhine valley and the Black Forest,
/// fewer over Tenerife's west).
pub const AIRWAYS: &[Airway] = &[
    // Tenerife: Europe – the Canaries east of the island, the islands'
    // links and the Atlantic routes south-west.
    Airway {
        name: "Europe – Gran Canaria",
        points: &[(31.6, -14.6), (29.2, -15.2), (27.6, -15.5)],
        per_hour: 6.0,
    },
    Airway {
        name: "Canaries – Atlantic",
        points: &[(29.4, -15.6), (28.2, -17.0), (26.8, -18.6)],
        per_hour: 2.0,
    },
    Airway {
        name: "Madeira – Tenerife South",
        points: &[(32.6, -16.8), (30.2, -16.7), (28.3, -16.3)],
        per_hour: 2.5,
    },
    // Liechtenstein and the Alps: Germany – Italy over the Rhine valley,
    // Basel – Innsbruck along the Alps, Munich – Milan.
    Airway {
        name: "Stuttgart – Milan",
        points: &[(48.6, 9.0), (47.3, 9.5), (45.8, 9.8)],
        per_hour: 8.0,
    },
    Airway {
        name: "Basel – Innsbruck",
        points: &[(47.6, 7.5), (47.25, 9.6), (47.25, 11.4)],
        per_hour: 7.0,
    },
    Airway {
        name: "Munich – Geneva",
        points: &[(48.3, 11.6), (47.4, 9.2), (46.3, 6.3)],
        per_hour: 6.0,
    },
    // The Black Forest: the Rhine valley north – south, Paris – Munich,
    // Stuttgart – Lyon.
    Airway {
        name: "Frankfurt – Basel",
        points: &[(50.0, 8.6), (48.6, 8.0), (47.4, 7.5)],
        per_hour: 12.0,
    },
    Airway {
        name: "Paris – Munich",
        points: &[(48.8, 6.4), (48.4, 8.3), (48.2, 11.0)],
        per_hour: 9.0,
    },
    Airway {
        name: "Stuttgart – Lyon",
        points: &[(48.9, 9.5), (48.0, 8.0), (46.5, 5.4)],
        per_hour: 6.0,
    },
    // Andorra: Toulouse – Barcelona over the Pyrenees, Bordeaux – the
    // Mediterranean, Spain's north – southern France.
    Airway {
        name: "Toulouse – Barcelona",
        points: &[(43.8, 1.3), (42.5, 1.6), (41.2, 2.2)],
        per_hour: 5.0,
    },
    Airway {
        name: "Bordeaux – Mediterranean",
        points: &[(44.6, -0.8), (42.7, 1.4), (41.0, 3.8)],
        per_hour: 7.0,
    },
    Airway {
        name: "Madrid – Marseille",
        points: &[(41.6, -1.6), (42.3, 1.2), (43.3, 5.2)],
        per_hour: 6.0,
    },
];

/// Airways whose nearest point lies within `km` of `centre` (lat, lon).
pub fn near(centre: (f64, f64), km: f64) -> Vec<&'static Airway> {
    AIRWAYS
        .iter()
        .filter(|a| {
            a.points
                .windows(2)
                .any(|w| segment_km(centre, w[0], w[1]) <= km)
        })
        .collect()
}

/// Distance (km, flat-earth near `p`) from `p` to the segment `a`–`b`.
fn segment_km(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let k = p.0.to_radians().cos();
    let xy = |q: (f64, f64)| ((q.1 - p.1) * k * 111.2, (q.0 - p.0) * 111.2);
    let (a, b) = (xy(a), xy(b));
    let d = (b.0 - a.0, b.1 - a.1);
    let l2 = d.0 * d.0 + d.1 * d.1;
    let t = if l2 > 0.0 {
        (-(a.0 * d.0 + a.1 * d.1) / l2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (a.0 + d.0 * t).hypot(a.1 + d.1 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_world_has_its_airways() {
        // Puerto de la Cruz, Vaduz, Freudenstadt, Andorra la Vella.
        for (place, n) in [
            ((28.41, -16.55), 2),
            ((47.14, 9.52), 3),
            ((48.46, 8.41), 3),
            ((42.51, 1.52), 3),
        ] {
            assert!(near(place, 120.0).len() >= n, "{place:?}");
        }
        assert!(near((60.0, 25.0), 120.0).is_empty());
    }
}
