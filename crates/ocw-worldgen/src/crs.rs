//! Map projections of the official elevation models (spec §4.3): the
//! grids of high-resolution DTMs are in national projected coordinates,
//! the world builder works in latitude / longitude (WGS 84 / ETRS89, which
//! differ by under a metre in Europe and are treated as the same here).
//!
//! - **UTM** (ETRS89 / UTM zone 32N = EPSG:25832: LGL DGM1 Baden-Württemberg
//!   and most German state DTMs): transverse Mercator with Krüger's series
//!   to the fourth order in n (Karney 2011), millimetre accuracy within a
//!   zone.
//! - **Swiss LV95** (CH1903+ / LV95 = EPSG:2056: swissALTI3D, Switzerland
//!   and Liechtenstein): swisstopo's approximate formulas including the
//!   datum shift, accurate to about 1 m (well within the accuracy of OSM
//!   positions and of a 2 m grid).

/// A projected coordinate reference system of a DTM grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Crs {
    /// ETRS89 / WGS 84 UTM zone `zone`, northern hemisphere.
    UtmNorth(u8),
    /// CH1903+ / LV95.
    Lv95,
}

// GRS 80 (ETRS89); WGS 84 differs in the inverse flattening by 1e-7.
const A: f64 = 6_378_137.0;
const F: f64 = 1.0 / 298.257_222_101;
const K0: f64 = 0.9996;
const FALSE_EASTING: f64 = 500_000.0;

/// Krüger series coefficients (n = third flattening).
struct Kruger {
    a_rect: f64,
    alpha: [f64; 4],
    beta: [f64; 4],
    delta: [f64; 4],
    e: f64,
}

fn kruger() -> Kruger {
    let n = F / (2.0 - F);
    let (n2, n3, n4) = (n * n, n * n * n, n * n * n * n);
    Kruger {
        a_rect: A / (1.0 + n) * (1.0 + n2 / 4.0 + n4 / 64.0),
        alpha: [
            n / 2.0 - 2.0 * n2 / 3.0 + 5.0 * n3 / 16.0 + 41.0 * n4 / 180.0,
            13.0 * n2 / 48.0 - 3.0 * n3 / 5.0 + 557.0 * n4 / 1440.0,
            61.0 * n3 / 240.0 - 103.0 * n4 / 140.0,
            49561.0 * n4 / 161_280.0,
        ],
        beta: [
            n / 2.0 - 2.0 * n2 / 3.0 + 37.0 * n3 / 96.0 - n4 / 360.0,
            n2 / 48.0 + n3 / 15.0 - 437.0 * n4 / 1440.0,
            17.0 * n3 / 480.0 - 37.0 * n4 / 840.0,
            4397.0 * n4 / 161_280.0,
        ],
        delta: [
            2.0 * n - 2.0 * n2 / 3.0 - 2.0 * n3 + 116.0 * n4 / 45.0,
            7.0 * n2 / 3.0 - 8.0 * n3 / 5.0 - 227.0 * n4 / 45.0,
            56.0 * n3 / 15.0 - 136.0 * n4 / 35.0,
            4279.0 * n4 / 630.0,
        ],
        e: (F * (2.0 - F)).sqrt(),
    }
}

fn utm_forward(zone: u8, lat: f64, lon: f64) -> [f64; 2] {
    let k = kruger();
    let lon0 = (f64::from(zone) * 6.0 - 183.0).to_radians();
    let (phi, dl) = (lat.to_radians(), lon.to_radians() - lon0);
    let s = phi.sin();
    let t = (s.atanh() - k.e * (k.e * s).atanh()).sinh();
    let xi_p = t.atan2(dl.cos());
    let eta_p = (dl.sin() / (1.0 + t * t).sqrt()).atanh();
    let (mut xi, mut eta) = (xi_p, eta_p);
    for (j, a) in k.alpha.iter().enumerate() {
        let m = 2.0 * (j as f64 + 1.0);
        xi += a * (m * xi_p).sin() * (m * eta_p).cosh();
        eta += a * (m * xi_p).cos() * (m * eta_p).sinh();
    }
    [FALSE_EASTING + K0 * k.a_rect * eta, K0 * k.a_rect * xi]
}

fn utm_inverse(zone: u8, x: f64, y: f64) -> (f64, f64) {
    let k = kruger();
    let lon0 = (f64::from(zone) * 6.0 - 183.0).to_radians();
    let xi = y / (K0 * k.a_rect);
    let eta = (x - FALSE_EASTING) / (K0 * k.a_rect);
    let (mut xi_p, mut eta_p) = (xi, eta);
    for (j, b) in k.beta.iter().enumerate() {
        let m = 2.0 * (j as f64 + 1.0);
        xi_p -= b * (m * xi).sin() * (m * eta).cosh();
        eta_p -= b * (m * xi).cos() * (m * eta).sinh();
    }
    let chi = (xi_p.sin() / eta_p.cosh()).asin();
    let mut phi = chi;
    for (j, d) in k.delta.iter().enumerate() {
        phi += d * (2.0 * (j as f64 + 1.0) * chi).sin();
    }
    let lon = lon0 + eta_p.sinh().atan2(xi_p.cos());
    (phi.to_degrees(), lon.to_degrees())
}

fn lv95_forward(lat: f64, lon: f64) -> [f64; 2] {
    let p = (lat * 3600.0 - 169_028.66) / 10_000.0;
    let l = (lon * 3600.0 - 26_782.5) / 10_000.0;
    let e =
        2_600_072.37 + 211_455.93 * l - 10_938.51 * l * p - 0.36 * l * p * p - 44.54 * l * l * l;
    let n = 1_200_147.07 + 308_807.95 * p + 3_745.25 * l * l + 76.63 * p * p - 194.56 * l * l * p
        + 119.79 * p * p * p;
    [e, n]
}

fn lv95_inverse(e: f64, n: f64) -> (f64, f64) {
    let y = (e - 2_600_000.0) / 1e6;
    let x = (n - 1_200_000.0) / 1e6;
    let l =
        2.677_909_4 + 4.728_982 * y + 0.791_484 * y * x + 0.1306 * y * x * x - 0.0436 * y * y * y;
    let p = 16.902_389_2 + 3.238_272 * x
        - 0.270_978 * y * y
        - 0.002_528 * x * x
        - 0.0447 * y * y * x
        - 0.0140 * x * x * x;
    (p * 100.0 / 36.0, l * 100.0 / 36.0)
}

impl Crs {
    /// From an EPSG code (GeoTIFF `ProjectedCSTypeGeoKey`): 258zz (ETRS89
    /// UTM), 326zz (WGS 84 UTM north), 2056 (LV95).
    pub fn from_epsg(code: u32) -> Option<Self> {
        match code {
            2056 => Some(Self::Lv95),
            25801..=25860 => Some(Self::UtmNorth((code - 25800) as u8)),
            32601..=32660 => Some(Self::UtmNorth((code - 32600) as u8)),
            _ => None,
        }
    }

    /// The EPSG code written into GeoTIFFs (ETRS89 for UTM).
    pub fn epsg(&self) -> u32 {
        match self {
            Self::Lv95 => 2056,
            Self::UtmNorth(z) => 25800 + u32::from(*z),
        }
    }

    /// Parses `utm32`, `epsg:25832`, `lv95`, `epsg:2056`.
    pub fn parse(s: &str) -> Result<Self, String> {
        let s = s.trim().to_ascii_lowercase();
        if s == "lv95" {
            return Ok(Self::Lv95);
        }
        if let Some(z) = s.strip_prefix("utm") {
            let z: u8 = z.parse().map_err(|_| format!("bad UTM zone in {s}"))?;
            if (1..=60).contains(&z) {
                return Ok(Self::UtmNorth(z));
            }
        }
        if let Some(c) = s.strip_prefix("epsg:")
            && let Some(crs) = c.parse().ok().and_then(Self::from_epsg)
        {
            return Ok(crs);
        }
        Err(format!(
            "unknown CRS {s} (utm32, lv95, epsg:25832, epsg:2056)"
        ))
    }

    /// (lat, lon) in degrees → (easting, northing) in metres.
    pub fn forward(&self, lat: f64, lon: f64) -> [f64; 2] {
        match *self {
            Self::UtmNorth(z) => utm_forward(z, lat, lon),
            Self::Lv95 => lv95_forward(lat, lon),
        }
    }

    /// (easting, northing) → (lat, lon).
    pub fn inverse(&self, x: f64, y: f64) -> (f64, f64) {
        match *self {
            Self::UtmNorth(z) => utm_inverse(z, x, y),
            Self::Lv95 => lv95_inverse(x, y),
        }
    }

    /// The (easting, northing) box covering a latitude / longitude box
    /// (its edges sampled, so the curved image of the box is inside).
    pub fn bounds_of(&self, south: f64, west: f64, north: f64, east: f64) -> [f64; 4] {
        let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        for i in 0..=32 {
            let t = f64::from(i) / 32.0;
            let lat = south + (north - south) * t;
            let lon = west + (east - west) * t;
            for p in [
                self.forward(lat, west),
                self.forward(lat, east),
                self.forward(south, lon),
                self.forward(north, lon),
            ] {
                lo = [lo[0].min(p[0]), lo[1].min(p[1])];
                hi = [hi[0].max(p[0]), hi[1].max(p[1])];
            }
        }
        [lo[0], lo[1], hi[0], hi[1]]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Metres per degree of latitude, for error tolerances.
    const M_PER_DEG: f64 = 111_200.0;

    fn close_m(a: (f64, f64), b: (f64, f64), tol_m: f64) {
        let dlat = (a.0 - b.0) * M_PER_DEG;
        let dlon = (a.1 - b.1) * M_PER_DEG * a.0.to_radians().cos();
        assert!(dlat.hypot(dlon) < tol_m, "{a:?} vs {b:?}");
    }

    // Reference values from GDAL/PROJ (EPSG:25832 and EPSG:2056 → 4326).
    #[test]
    fn utm32_matches_proj() {
        let utm = Crs::UtmNorth(32);
        for (xy, ll) in [
            ([425_000.0, 5_302_000.0], (47.866_957_03, 7.997_148_14)),
            ([500_000.0, 5_300_000.0], (47.853_341_95, 9.0)),
            ([450_000.0, 5_400_000.0], (48.751_005_51, 8.319_785_30)),
        ] {
            close_m(utm.inverse(xy[0], xy[1]), ll, 0.01);
            let p = utm.forward(ll.0, ll.1);
            assert!(
                (p[0] - xy[0]).abs() < 0.01 && (p[1] - xy[1]).abs() < 0.01,
                "{p:?}"
            );
        }
    }

    #[test]
    fn lv95_matches_proj_within_a_metre() {
        for (xy, ll) in [
            ([2_764_000.0, 1_217_000.0], (47.083_696_44, 9.598_560_98)),
            ([2_600_000.0, 1_200_000.0], (46.951_082_77, 7.438_632_42)),
        ] {
            close_m(Crs::Lv95.inverse(xy[0], xy[1]), ll, 1.5);
            let p = Crs::Lv95.forward(ll.0, ll.1);
            assert!(
                (p[0] - xy[0]).abs() < 1.5 && (p[1] - xy[1]).abs() < 1.5,
                "{p:?}"
            );
        }
    }

    #[test]
    fn round_trips() {
        // UTM is exact; swisstopo's approximate formulas hold within
        // Switzerland and Liechtenstein.
        for (crs, pts, tol) in [
            (
                Crs::UtmNorth(32),
                [(47.15, 9.55), (48.0, 8.0), (48.76, 8.24)],
                1e-4,
            ),
            (Crs::Lv95, [(47.15, 9.55), (46.95, 7.44), (46.5, 8.0)], 1.0),
        ] {
            for ll in pts {
                let p = crs.forward(ll.0, ll.1);
                close_m(crs.inverse(p[0], p[1]), ll, tol);
            }
        }
    }

    #[test]
    fn codes_and_names() {
        assert_eq!(Crs::from_epsg(25832), Some(Crs::UtmNorth(32)));
        assert_eq!(Crs::from_epsg(32632), Some(Crs::UtmNorth(32)));
        assert_eq!(Crs::from_epsg(2056), Some(Crs::Lv95));
        assert_eq!(Crs::from_epsg(4326), None);
        assert_eq!(Crs::UtmNorth(32).epsg(), 25832);
        assert_eq!(Crs::parse("UTM32"), Ok(Crs::UtmNorth(32)));
        assert_eq!(Crs::parse("epsg:2056"), Ok(Crs::Lv95));
        assert!(Crs::parse("utm99").is_err());
        let b = Crs::UtmNorth(32).bounds_of(47.8, 7.9, 48.0, 8.1);
        assert!(b[0] < b[2] && b[1] < b[3]);
        let sw = Crs::UtmNorth(32).forward(47.8, 7.9);
        assert!(b[0] <= sw[0] && b[1] <= sw[1]);
    }
}
