//! Local metric projection around a geographic centre (spec §4.3, §2.8).
//!
//! Spherical transverse-Mercator-like local plane: x east, z south (the
//! crate convention), metres. For pilot-sized regions (tens of km) the scale
//! error is well below 0.1 %; large regions will move to a proper UTM /
//! transverse Mercator in a later step. `world-builder` projects OSM data
//! with it; the client unprojects ride positions for the virtual GPS track of
//! the FIT export.

/// Mean Earth radius in metres.
pub const EARTH_RADIUS_M: f64 = 6_371_008.8;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LocalProjection {
    pub lat0: f64,
    pub lon0: f64,
    cos_lat0: f64,
}

impl LocalProjection {
    pub fn new(lat0: f64, lon0: f64) -> Self {
        Self {
            lat0,
            lon0,
            cos_lat0: lat0.to_radians().cos(),
        }
    }

    /// (lat, lon) in degrees → (x east, z south) in metres.
    pub fn project(&self, lat: f64, lon: f64) -> [f64; 2] {
        let x = (lon - self.lon0).to_radians() * EARTH_RADIUS_M * self.cos_lat0;
        let north = (lat - self.lat0).to_radians() * EARTH_RADIUS_M;
        [x, -north]
    }

    /// Inverse of [`LocalProjection::project`]: (x east, z south) in metres
    /// → (lat, lon) in degrees.
    pub fn unproject(&self, p: [f64; 2]) -> (f64, f64) {
        let lat = self.lat0 + (-p[1] / EARTH_RADIUS_M).to_degrees();
        let lon = self.lon0 + (p[0] / (EARTH_RADIUS_M * self.cos_lat0)).to_degrees();
        (lat, lon)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unproject_inverts_project() {
        let p = LocalProjection::new(47.15, 9.55);
        assert_eq!(p.unproject([0.0, 0.0]), (47.15, 9.55));
        let (lat, lon) = p.unproject(p.project(47.2, 9.6));
        assert!((lat - 47.2).abs() < 1e-12 && (lon - 9.6).abs() < 1e-12);
        // 1 km south-east of the centre.
        let (lat, lon) = p.unproject([1000.0, 1000.0]);
        assert!(lat < 47.15 && lon > 9.55);
        assert!(((47.15 - lat) * 111_195.0 - 1000.0).abs() < 1.0);
    }
}
