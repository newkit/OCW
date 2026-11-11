//! Local metric projection around a region centre. The projection itself
//! lives in `ocw_world::geo` (the client unprojects ride positions with it,
//! spec §2.8); re-exported here.

pub use ocw_world::geo::{EARTH_RADIUS_M, LocalProjection};

/// Great-circle distance in metres (haversine), for checking the projection.
pub fn haversine_m(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (la1, lo1, la2, lo2) = (
        a.0.to_radians(),
        a.1.to_radians(),
        b.0.to_radians(),
        b.1.to_radians(),
    );
    let h = ((la2 - la1) / 2.0).sin().powi(2)
        + la1.cos() * la2.cos() * ((lo2 - lo1) / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_M * h.sqrt().asin()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axes_and_round_trip() {
        let p = LocalProjection::new(47.15, 9.55);
        assert_eq!(p.project(47.15, 9.55), [0.0, 0.0]);
        let east = p.project(47.15, 9.56);
        let north = p.project(47.16, 9.55);
        assert!(east[0] > 0.0 && east[1].abs() < 1e-9);
        // z points south, so north is negative z.
        assert!(north[1] < 0.0 && north[0].abs() < 1e-9);
        let (lat, lon) = p.unproject(p.project(47.2, 9.6));
        assert!((lat - 47.2).abs() < 1e-12 && (lon - 9.6).abs() < 1e-12);
    }

    #[test]
    fn distances_match_great_circle_within_a_pilot_region() {
        let p = LocalProjection::new(47.15, 9.55);
        // Corners of a ~25 × 30 km box around Liechtenstein.
        for (a, b) in [
            ((47.05, 9.47), (47.27, 9.64)),
            ((47.05, 9.64), (47.27, 9.47)),
            ((47.15, 9.40), (47.15, 9.70)),
        ] {
            let (pa, pb) = (p.project(a.0, a.1), p.project(b.0, b.1));
            let planar = ((pa[0] - pb[0]).powi(2) + (pa[1] - pb[1]).powi(2)).sqrt();
            let true_m = haversine_m(a, b);
            assert!(
                ((planar - true_m) / true_m).abs() < 1e-3,
                "{planar} vs {true_m}"
            );
        }
    }
}
