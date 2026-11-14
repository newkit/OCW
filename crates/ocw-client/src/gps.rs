//! The local rider's GPS position for the HUD (spec §3.2).

use ocw_sim::RiderState;
use ocw_world::Track;
use ocw_world::geo::LocalProjection;

/// (lat °, lon °, altitude m) of a rider at `state` on `track`, whose local
/// metres have the geographic origin `geo_origin`; `None` without one
/// (built-in test routes). The same position as the ride recording's.
pub fn position(
    track: &dyn Track,
    geo_origin: Option<(f64, f64)>,
    state: &RiderState,
) -> Option<(f64, f64, f64)> {
    let (lat0, lon0) = geo_origin?;
    let p = track
        .sample(state.distance)
        .offset_position(state.lateral_offset);
    let (lat, lon) = LocalProjection::new(lat0, lon0).unproject([p[0], p[2]]);
    Some((lat, lon, p[1]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_world::Route;

    fn state(distance: f64, lateral_offset: f64) -> RiderState {
        RiderState {
            distance,
            lateral_offset,
            ..RiderState::default()
        }
    }

    #[test]
    fn none_without_a_geo_origin() {
        let route = Route::test_loop();
        assert_eq!(
            position(&route, route.geo_origin(), &state(100.0, 0.0)),
            None
        );
    }

    #[test]
    fn round_trips_with_the_projection() {
        let origin = (47.15, 9.55);
        let route = Route::test_loop().with_geo_origin(origin);
        let s = state(250.0, 1.5);
        let (lat, lon, alt) = position(&route, route.geo_origin(), &s).unwrap();
        let p = route.sample(250.0).offset_position(1.5);
        let back = LocalProjection::new(origin.0, origin.1).project(lat, lon);
        assert!((back[0] - p[0]).abs() < 1e-6 && (back[1] - p[2]).abs() < 1e-6);
        assert_eq!(alt, p[1]);
        // Plausibly near the origin (the loop is a few hundred metres).
        assert!((lat - origin.0).abs() < 0.05 && (lon - origin.1).abs() < 0.05);
    }
}
