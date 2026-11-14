//! Shared astronomy for the night sky (spec §14 Phase 9, S2): dynamical
//! time, sidereal time, obliquity and nutation, and the conversions between
//! ecliptic, equatorial and horizontal coordinates (after Meeus,
//! *Astronomical Algorithms*, 2nd ed., chapters 10, 12, 13 and 22; the
//! formulas are facts, no data is copied). Angles in degrees unless the
//! name says radians.

use crate::time::{UnixMs, julian_day, utc_date};

/// TT − UT (seconds) for a decimal year: the polynomial fits of Espenak &
/// Meeus (NASA, 2006) for 1900–2150; good to a few seconds (the Moon moves
/// 0.5" per second, far below what is visible).
pub fn delta_t_s(year: f64) -> f64 {
    let y = year;
    if y < 1920.0 {
        let t = y - 1900.0;
        -2.79 + t * (1.494_119 + t * (-0.059_893_9 + t * (0.006_196_6 - 0.000_197 * t)))
    } else if y < 1941.0 {
        let t = y - 1920.0;
        21.20 + t * (0.844_93 + t * (-0.076_100 + 0.002_093_6 * t))
    } else if y < 1961.0 {
        let t = y - 1950.0;
        29.07 + 0.407 * t - t * t / 233.0 + t.powi(3) / 2547.0
    } else if y < 1986.0 {
        let t = y - 1975.0;
        45.45 + 1.067 * t - t * t / 260.0 - t.powi(3) / 718.0
    } else if y < 2005.0 {
        let t = y - 2000.0;
        63.86
            + t * (0.3345
                + t * (-0.060_374 + t * (0.001_727_5 + t * (0.000_651_814 + 0.000_023_735_99 * t))))
    } else if y < 2050.0 {
        let t = y - 2000.0;
        62.92 + 0.322_17 * t + 0.005_589 * t * t
    } else {
        let u = (y - 1820.0) / 100.0;
        -20.0 + 32.0 * u * u - 0.5628 * (2150.0 - y)
    }
}

/// Julian Ephemeris Day (TT) of an instant (UT).
pub fn jde(ms: UnixMs) -> f64 {
    let (y, m, _) = utc_date(ms);
    let year = f64::from(y) + (f64::from(m) - 0.5) / 12.0;
    julian_day(ms) + delta_t_s(year) / 86_400.0
}

/// Julian centuries (TT) since J2000.0.
pub fn centuries_tt(ms: UnixMs) -> f64 {
    (jde(ms) - 2_451_545.0) / 36_525.0
}

/// Greenwich mean sidereal time (°, 0–360) at an instant (Meeus 12.4).
pub fn gmst_deg(ms: UnixMs) -> f64 {
    let jd = julian_day(ms);
    let t = (jd - 2_451_545.0) / 36_525.0;
    (280.460_618_37 + 360.985_647_366_29 * (jd - 2_451_545.0) + 0.000_387_933 * t * t
        - t.powi(3) / 38_710_000.0)
        .rem_euclid(360.0)
}

/// Local mean sidereal time (°) at longitude `lon` (east positive).
pub fn lst_deg(ms: UnixMs, lon: f64) -> f64 {
    (gmst_deg(ms) + lon).rem_euclid(360.0)
}

/// Nutation in longitude and obliquity (°), the main terms (Meeus 22,
/// accurate to 0.5" and 0.1").
pub fn nutation(t: f64) -> (f64, f64) {
    let omega = (125.044_52 - 1_934.136_261 * t).to_radians();
    let l_sun = (280.4665 + 36_000.769_8 * t).to_radians();
    let l_moon = (218.3165 + 481_267.881_3 * t).to_radians();
    let dpsi = -17.20 * omega.sin() - 1.32 * (2.0 * l_sun).sin() - 0.23 * (2.0 * l_moon).sin()
        + 0.21 * (2.0 * omega).sin();
    let deps = 9.20 * omega.cos() + 0.57 * (2.0 * l_sun).cos() + 0.10 * (2.0 * l_moon).cos()
        - 0.09 * (2.0 * omega).cos();
    (dpsi / 3600.0, deps / 3600.0)
}

/// Mean obliquity of the ecliptic (°, Meeus 22.2).
pub fn mean_obliquity(t: f64) -> f64 {
    23.0 + 26.0 / 60.0 + (21.448 - t * (46.8150 + t * (0.000_59 - 0.001_813 * t))) / 3600.0
}

/// True obliquity (mean plus nutation, °).
pub fn true_obliquity(t: f64) -> f64 {
    mean_obliquity(t) + nutation(t).1
}

/// A direction on the celestial sphere (right ascension, declination; °).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Equatorial {
    pub ra_deg: f64,
    pub dec_deg: f64,
}

/// Ecliptic longitude / latitude (°) to equatorial coordinates for the
/// obliquity `eps` (°, Meeus 13.3 and 13.4).
pub fn ecliptic_to_equatorial(lon: f64, lat: f64, eps: f64) -> Equatorial {
    let (l, b, e) = (lon.to_radians(), lat.to_radians(), eps.to_radians());
    let ra = (l.sin() * e.cos() - b.tan() * e.sin()).atan2(l.cos());
    let dec = (b.sin() * e.cos() + b.cos() * e.sin() * l.sin())
        .clamp(-1.0, 1.0)
        .asin();
    Equatorial {
        ra_deg: ra.to_degrees().rem_euclid(360.0),
        dec_deg: dec.to_degrees(),
    }
}

/// East / north / up unit vector of a direction (hour angle `ha`,
/// declination `dec`) seen from latitude `lat` (all °).
pub fn hour_angle_to_enu(ha: f64, dec: f64, lat: f64) -> [f64; 3] {
    let (h, d, p) = (ha.to_radians(), dec.to_radians(), lat.to_radians());
    [
        -d.cos() * h.sin(),
        p.cos() * d.sin() - p.sin() * d.cos() * h.cos(),
        p.sin() * d.sin() + p.cos() * d.cos() * h.cos(),
    ]
}

/// Azimuth (clockwise from north, 0–360°) and elevation (°, geometric) of
/// an east / north / up vector.
pub fn enu_to_az_el(v: [f64; 3]) -> (f64, f64) {
    let az = v[0].atan2(v[1]).to_degrees().rem_euclid(360.0);
    let el = v[2].clamp(-1.0, 1.0).asin().to_degrees();
    (az, el)
}

/// Geometric azimuth and elevation of `eq` at an instant and place.
pub fn horizontal(eq: Equatorial, ms: UnixMs, lat: f64, lon: f64) -> (f64, f64) {
    let ha = lst_deg(ms, lon) - eq.ra_deg;
    enu_to_az_el(hour_angle_to_enu(ha, eq.dec_deg, lat))
}

/// The rotation from equatorial unit vectors (x towards the equinox, z
/// towards the north celestial pole) to east / north / up at local
/// sidereal time `lst` and latitude `lat` (°): row-major, `enu = M · eq`.
pub fn equatorial_to_enu(lst: f64, lat: f64) -> [[f64; 3]; 3] {
    let (s, c) = lst.to_radians().sin_cos();
    let (sp, cp) = lat.to_radians().sin_cos();
    [[-s, c, 0.0], [-sp * c, -sp * s, cp], [cp * c, cp * s, sp]]
}

/// Unit vector of an equatorial direction (x equinox, z pole).
pub fn equatorial_vector(eq: Equatorial) -> [f64; 3] {
    let (a, d) = (eq.ra_deg.to_radians(), eq.dec_deg.to_radians());
    [d.cos() * a.cos(), d.cos() * a.sin(), d.sin()]
}

pub fn mat_vec(m: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2])
}

/// Angular separation (°) of two directions.
pub fn separation(a: Equatorial, b: Equatorial) -> f64 {
    let (va, vb) = (equatorial_vector(a), equatorial_vector(b));
    let dot = va[0] * vb[0] + va[1] * vb[1] + va[2] * vb[2];
    let cross = [
        va[1] * vb[2] - va[2] * vb[1],
        va[2] * vb[0] - va[0] * vb[2],
        va[0] * vb[1] - va[1] * vb[0],
    ];
    let c = (cross[0].powi(2) + cross[1].powi(2) + cross[2].powi(2)).sqrt();
    c.atan2(dot).to_degrees()
}

/// General precession in longitude from J2000.0 to `t` centuries (°): the
/// ecliptic J2000 longitudes of the planets' orbital elements become
/// longitudes of the date (good to 0.01° over 1900–2100).
pub fn precession_lon(t: f64) -> f64 {
    (5029.0966 * t + 1.111_13 * t * t) / 3600.0
}

/// Normalises an angle to -180 … 180°.
pub fn wrap180(a: f64) -> f64 {
    (a + 180.0).rem_euclid(360.0) - 180.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::{MS_PER_HOUR, from_julian_day, utc_midnight};

    #[test]
    fn sidereal_time_matches_meeus() {
        // Meeus example 12.a: 1987 April 10, 0h UT: 13h10m46.3668s.
        let ms = utc_midnight(1987, 4, 10);
        let want = (13.0 + 10.0 / 60.0 + 46.3668 / 3600.0) * 15.0;
        assert!((gmst_deg(ms) - want).abs() < 1e-4, "{}", gmst_deg(ms));
        // Example 12.b: 19h21m00s UT the same day: 8h34m57.0896s.
        let ms = ms + 19 * MS_PER_HOUR + 21 * 60_000;
        let want = (8.0 + 34.0 / 60.0 + 57.0896 / 3600.0) * 15.0;
        assert!((gmst_deg(ms) - want).abs() < 1e-3, "{}", gmst_deg(ms));
        assert!((lst_deg(ms, 10.0) - (want + 10.0)).abs() < 1e-3);
    }

    #[test]
    fn nutation_and_obliquity_match_meeus() {
        // Example 22.a: 1987 April 10, 0h TD: Δψ = -3.788", Δε = +9.443",
        // ε0 = 23°26'27.407", ε = 23°26'36.850".
        let t = (2_446_895.5 - 2_451_545.0) / 36_525.0;
        let (dpsi, deps) = nutation(t);
        assert!((dpsi * 3600.0 + 3.788).abs() < 0.5, "{}", dpsi * 3600.0);
        assert!((deps * 3600.0 - 9.443).abs() < 0.2, "{}", deps * 3600.0);
        let e0 = 23.0 + 26.0 / 60.0 + 27.407 / 3600.0;
        assert!((mean_obliquity(t) - e0).abs() < 1e-5);
        assert!((true_obliquity(t) - (e0 + 9.443 / 3600.0)).abs() < 1e-4);
    }

    #[test]
    fn delta_t_is_about_a_minute_today() {
        assert!((delta_t_s(2026.0) - 70.0).abs() < 7.0);
        assert!((delta_t_s(2000.0) - 63.9).abs() < 0.5);
        assert!((delta_t_s(1950.0) - 29.1).abs() < 0.5);
        assert!(delta_t_s(1900.0).abs() < 3.0);
        assert!((60.0..250.0).contains(&delta_t_s(2100.0)));
        let ms = from_julian_day(2_451_545.0);
        assert!((jde(ms) - 2_451_545.0) * 86_400.0 > 60.0);
    }

    #[test]
    fn coordinate_rotations_agree() {
        // Meeus example 13.a (Pollux): λ 113.215630°, β 6.684170°,
        // ε 23.4392911°: α 116.328942°, δ 28.026183°.
        let eq = ecliptic_to_equatorial(113.215_630, 6.684_170, 23.439_291_1);
        assert!((eq.ra_deg - 116.328_942).abs() < 1e-5, "{eq:?}");
        assert!((eq.dec_deg - 28.026_183).abs() < 1e-5, "{eq:?}");
        // The matrix and the hour-angle formula give the same direction.
        let (lst, lat) = (123.4, 47.1);
        let m = equatorial_to_enu(lst, lat);
        for (ra, dec) in [(10.0, 20.0), (200.0, -40.0), (300.0, 80.0)] {
            let a = mat_vec(
                &m,
                equatorial_vector(Equatorial {
                    ra_deg: ra,
                    dec_deg: dec,
                }),
            );
            let b = hour_angle_to_enu(lst - ra, dec, lat);
            for i in 0..3 {
                assert!((a[i] - b[i]).abs() < 1e-12);
            }
        }
        // The pole stands at the latitude's height in the north.
        let (az, el) = enu_to_az_el(mat_vec(&m, [0.0, 0.0, 1.0]));
        assert!((el - lat).abs() < 1e-9 && az.abs() < 1e-9);
        let s = separation(
            Equatorial {
                ra_deg: 0.0,
                dec_deg: 0.0,
            },
            Equatorial {
                ra_deg: 90.0,
                dec_deg: 0.0,
            },
        );
        assert!((s - 90.0).abs() < 1e-9);
        assert_eq!(wrap180(350.0), -10.0);
    }
}
