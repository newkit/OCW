//! What a rider rides along (spec §5.2): a fixed [`Route`] (route files,
//! test loops) or a free-ride [`RidePath`](crate::RidePath) through the
//! road graph that grows at every junction. Physics and rendering only use
//! this trait, so both work the same on either.

use crate::route::{Route, RoutePoint};
use crate::surface::Surface;

/// A 1D track: everything is a function of the distance along it.
pub trait Track {
    /// Centreline position, direction and gradient at `distance`.
    fn sample(&self, distance: f64) -> RoutePoint;
    /// Signed horizontal curvature at `distance` (1/m, positive = right).
    fn curvature(&self, distance: f64) -> f64;
    /// Road surface at `distance` (on the road; beside it is grass).
    fn surface_at(&self, distance: f64) -> Surface;
    /// Road width at `distance`, in metres.
    fn road_width_at(&self, distance: f64) -> f64;
    /// Highest speed (m/s) a rider should have at `distance`: the automatic
    /// slow-down before sharp turns and U-turns (spec §5.2, §2.2.2), since
    /// a rider on a trainer cannot brake. `None` = no limit.
    fn speed_limit(&self, _distance: f64) -> Option<f64> {
        None
    }
    /// Whether steering is automatic at `distance` (the U-turn loop):
    /// riders steering by leaning are steered through it.
    fn assisted(&self, _distance: f64) -> bool {
        false
    }
    /// Display name (route name, or the street the rider is on).
    fn name_at(&self, _distance: f64) -> String {
        String::new()
    }
    /// The distances the track is known for, `(from, to)`; `None` = all
    /// (a loop). Outside it [`Track::sample`] repeats the nearest end, so
    /// look-ahead (gradient, elevation profile) stops at `to`.
    fn extent(&self) -> Option<(f64, f64)> {
        None
    }
}

impl Track for Route {
    fn sample(&self, distance: f64) -> RoutePoint {
        Route::sample(self, distance)
    }
    fn curvature(&self, distance: f64) -> f64 {
        Route::curvature(self, distance)
    }
    fn surface_at(&self, distance: f64) -> Surface {
        Route::surface_at(self, distance)
    }
    fn road_width_at(&self, _distance: f64) -> f64 {
        self.road_width_m()
    }
    fn name_at(&self, _distance: f64) -> String {
        self.name().to_string()
    }
    fn extent(&self) -> Option<(f64, f64)> {
        (!self.is_loop()).then(|| (0.0, self.length()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_are_tracks() {
        let r = Route::test_hills();
        let t: &dyn Track = &r;
        assert_eq!(t.sample(100.0), r.sample(100.0));
        assert_eq!(t.curvature(100.0), r.curvature(100.0));
        assert_eq!(t.surface_at(1600.0), Surface::Gravel);
        assert_eq!(t.road_width_at(5.0), 7.0);
        assert_eq!(t.speed_limit(5.0), None);
        assert!(!t.assisted(5.0));
        assert_eq!(t.name_at(0.0), "Test Hills");
        assert_eq!(t.extent(), None, "a loop never ends");
        let line =
            Route::from_points("l", "L", vec![[0.0; 3], [100.0, 0.0, 0.0]], false, 5.0).unwrap();
        assert_eq!((&line as &dyn Track).extent(), Some((0.0, 100.0)));
    }
}
