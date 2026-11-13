//! Solar position and the day's sun events (spec §14 Phase 9, 9B): the
//! NOAA solar calculator's algorithm (after Meeus, *Astronomical
//! Algorithms*): declination and equation of time from the instant, then
//! azimuth and elevation (with the NOAA atmospheric refraction) at a place,
//! and sunrise / sunset / solar noon and the civil, nautical and
//! astronomical twilights of a date. Valid for 1900–2100; the error is
//! well under 0.05° in position and under a minute in the event times at
//! mid latitudes.

use crate::time::{MS_PER_DAY, UnixMs, julian_century, julian_day, utc_midnight};

/// Where the sun is, seen from a place.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SunPosition {
    /// Clockwise from true north, 0–360°.
    pub azimuth_deg: f64,
    /// Above the horizon (negative below), with atmospheric refraction:
    /// where the sun is seen.
    pub elevation_deg: f64,
    /// Geometric elevation without refraction.
    pub true_elevation_deg: f64,
}

impl SunPosition {
    /// Unit vector towards the sun in east / up / north components.
    pub fn direction_enu(&self) -> [f64; 3] {
        let (az, el) = (self.azimuth_deg.to_radians(), self.elevation_deg.to_radians());
        [el.cos() * az.sin(), el.sin(), el.cos() * az.cos()]
    }
}

/// Declination and equation of time at a Julian day.
#[derive(Debug, Clone, Copy)]
struct Ephemeris {
    declination_rad: f64,
    /// Apparent minus mean solar time, minutes.
    equation_of_time_min: f64,
}

fn ephemeris(jd: f64) -> Ephemeris {
    let t = julian_century(jd);
    let l0 = (280.46646 + t * (36_000.76983 + t * 0.000_303_2)).rem_euclid(360.0);
    let m = 357.52911 + t * (35_999.05029 - 0.000_153_7 * t);
    let e = 0.016_708_634 - t * (0.000_042_037 + 0.000_000_126_7 * t);
    let mr = m.to_radians();
    let c = mr.sin() * (1.914_602 - t * (0.004_817 + 0.000_014 * t))
        + (2.0 * mr).sin() * (0.019_993 - 0.000_101 * t)
        + (3.0 * mr).sin() * 0.000_289;
    let true_long = l0 + c;
    let omega = (125.04 - 1934.136 * t).to_radians();
    let app_long = true_long - 0.005_69 - 0.004_78 * omega.sin();
    let mean_obliq =
        23.0 + (26.0 + (21.448 - t * (46.815 + t * (0.000_59 - t * 0.001_813))) / 60.0) / 60.0;
    let obliq = (mean_obliq + 0.002_56 * omega.cos()).to_radians();
    let declination_rad = (obliq.sin() * app_long.to_radians().sin()).asin();
    let y = (obliq / 2.0).tan().powi(2);
    let l0r = l0.to_radians();
    let eq = y * (2.0 * l0r).sin() - 2.0 * e * mr.sin() + 4.0 * e * y * mr.sin() * (2.0 * l0r).cos()
        - 0.5 * y * y * (4.0 * l0r).sin()
        - 1.25 * e * e * (2.0 * mr).sin();
    Ephemeris {
        declination_rad,
        equation_of_time_min: 4.0 * eq.to_degrees(),
    }
}

/// Atmospheric refraction (degrees) at a geometric elevation, NOAA's
/// approximation for standard pressure and temperature.
pub fn refraction_deg(elevation_deg: f64) -> f64 {
    let e = elevation_deg;
    let arcsec = if e > 85.0 {
        0.0
    } else if e > 5.0 {
        let t = e.to_radians().tan();
        58.1 / t - 0.07 / t.powi(3) + 0.000_086 / t.powi(5)
    } else if e > -0.575 {
        1735.0 + e * (-518.2 + e * (103.4 + e * (-12.79 + e * 0.711)))
    } else {
        -20.772 / e.to_radians().tan()
    };
    arcsec / 3600.0
}

/// The sun at instant `ms` from latitude `lat` / longitude `lon` (°,
/// north and east positive).
pub fn position(ms: UnixMs, lat: f64, lon: f64) -> SunPosition {
    let eph = ephemeris(julian_day(ms));
    let minutes_utc = ms.rem_euclid(MS_PER_DAY) as f64 / 60_000.0;
    let true_solar_min = (minutes_utc + eph.equation_of_time_min + 4.0 * lon).rem_euclid(1440.0);
    let mut hour_angle = true_solar_min / 4.0 - 180.0;
    if hour_angle < -180.0 {
        hour_angle += 360.0;
    }
    let (latr, ha, dec) = (lat.to_radians(), hour_angle.to_radians(), eph.declination_rad);
    let cos_zenith = (latr.sin() * dec.sin() + latr.cos() * dec.cos() * ha.cos()).clamp(-1.0, 1.0);
    let zenith = cos_zenith.acos();
    let true_elevation_deg = 90.0 - zenith.to_degrees();
    let denom = latr.cos() * zenith.sin();
    let azimuth = if denom.abs() > 0.001 {
        let a = ((latr.sin() * cos_zenith - dec.sin()) / denom).clamp(-1.0, 1.0);
        let az = 180.0 - a.acos().to_degrees();
        if hour_angle > 0.0 { -az } else { az }
    } else if lat > 0.0 {
        180.0
    } else {
        0.0
    };
    SunPosition {
        azimuth_deg: azimuth.rem_euclid(360.0),
        elevation_deg: true_elevation_deg + refraction_deg(true_elevation_deg),
        true_elevation_deg,
    }
}

/// Zenith angles of the sun events (°).
pub mod zenith {
    /// Sunrise / sunset: the upper limb on the horizon with refraction.
    pub const SUNRISE: f64 = 90.833;
    pub const CIVIL: f64 = 96.0;
    pub const NAUTICAL: f64 = 102.0;
    pub const ASTRONOMICAL: f64 = 108.0;
}

/// The instant (minutes after 00:00 UTC of the date, may be outside
/// 0–1440) when the sun crosses `zenith_deg`, rising or setting, for the
/// ephemeris at `jd`; `None` when it never does (polar day / night).
fn crossing_min(jd: f64, lat: f64, lon: f64, zenith_deg: f64, rising: bool) -> Option<f64> {
    let eph = ephemeris(jd);
    let (latr, dec) = (lat.to_radians(), eph.declination_rad);
    let arg = zenith_deg.to_radians().cos() / (latr.cos() * dec.cos()) - latr.tan() * dec.tan();
    if !(-1.0..=1.0).contains(&arg) {
        return None;
    }
    let ha = arg.acos().to_degrees();
    let ha = if rising { ha } else { -ha };
    Some(720.0 - 4.0 * (lon + ha) - eph.equation_of_time_min)
}

/// When on the UTC date `(y, m, d)` the sun crosses `zenith_deg` (see
/// [`zenith`]), rising (morning) or setting (evening).
pub fn crossing(
    (y, m, d): (i32, u32, u32),
    lat: f64,
    lon: f64,
    zenith_deg: f64,
    rising: bool,
) -> Option<UnixMs> {
    let midnight = utc_midnight(y, m, d);
    let jd0 = julian_day(midnight);
    // Iterate with the ephemeris at the event itself (NOAA does one step).
    let mut t = crossing_min(jd0 + 0.5, lat, lon, zenith_deg, rising)?;
    for _ in 0..3 {
        t = crossing_min(jd0 + t / 1440.0, lat, lon, zenith_deg, rising)?;
    }
    Some(midnight + (t * 60_000.0).round() as UnixMs)
}

/// Solar noon (the sun on the meridian) of the UTC date `(y, m, d)`.
pub fn solar_noon((y, m, d): (i32, u32, u32), lon: f64) -> UnixMs {
    let midnight = utc_midnight(y, m, d);
    let jd0 = julian_day(midnight);
    let mut t = 720.0 - 4.0 * lon;
    for _ in 0..3 {
        t = 720.0 - 4.0 * lon - ephemeris(jd0 + t / 1440.0).equation_of_time_min;
    }
    midnight + (t * 60_000.0).round() as UnixMs
}

/// A pair of morning / evening events; `None` where the sun does not
/// reach that depth (e.g. no astronomical night in a northern June).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pair {
    pub morning: Option<UnixMs>,
    pub evening: Option<UnixMs>,
}

/// The sun's day at a place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SunDay {
    pub solar_noon: UnixMs,
    /// Sunrise and sunset.
    pub sun: Pair,
    pub civil: Pair,
    pub nautical: Pair,
    pub astronomical: Pair,
}

/// The sun events of a calendar date at a place (the date as the place's
/// local date: the NOAA calculator also computes on the given date).
pub fn day(date: (i32, u32, u32), lat: f64, lon: f64) -> SunDay {
    let pair = |z| Pair {
        morning: crossing(date, lat, lon, z, true),
        evening: crossing(date, lat, lon, z, false),
    };
    SunDay {
        solar_noon: solar_noon(date, lon),
        sun: pair(zenith::SUNRISE),
        civil: pair(zenith::CIVIL),
        nautical: pair(zenith::NAUTICAL),
        astronomical: pair(zenith::ASTRONOMICAL),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::MS_PER_HOUR;

    const VADUZ: (f64, f64) = (47.141, 9.521);
    const FREIBURG: (f64, f64) = (47.995, 7.85);

    fn hms(date: (i32, u32, u32), h: i64, m: i64, s: i64) -> UnixMs {
        utc_midnight(date.0, date.1, date.2) + h * MS_PER_HOUR + m * 60_000 + s * 1000
    }

    fn close(got: Option<UnixMs>, want: UnixMs, tol_s: i64, what: &str) {
        let got = got.unwrap_or_else(|| panic!("{what}: no event"));
        let d = (got - want).abs();
        assert!(d <= tol_s * 1000, "{what}: off by {:.1} s", d as f64 / 1000.0);
    }

    /// Reference values (UTC, to the second): the NOAA-algorithm tables of
    /// sunrise-sunset.org for these coordinates, fetched 2026-11-13; ± 1
    /// minute (astronomical twilight on the June solstice ± 3 minutes: the
    /// sun only just reaches -18°, so the time is very sensitive).
    #[test]
    fn sun_events_in_vaduz_and_freiburg_match_the_noaa_tables() {
        // (place, date, noon, rise, set, civil, nautical, astronomical)
        type Hms = (i64, i64, i64);
        type Case = ((f64, f64), (i32, u32, u32), Hms, [(Hms, Hms); 4]);
        let cases: [Case; 5] = [
            (VADUZ, (2026, 6, 21), (11, 23, 44), [
                ((3, 26, 9), (19, 21, 18)),
                ((2, 46, 9), (20, 1, 18)),
                ((1, 51, 6), (20, 56, 22)),
                ((0, 26, 52), (22, 20, 36)),
            ]),
            (VADUZ, (2026, 12, 21), (11, 19, 58), [
                ((7, 5, 12), (15, 34, 43)),
                ((6, 29, 42), (16, 10, 13)),
                ((5, 50, 46), (16, 49, 10)),
                ((5, 13, 32), (17, 26, 23)),
            ]),
            (VADUZ, (2026, 10, 25), (11, 5, 59), [
                ((5, 54, 34), (16, 17, 23)),
                ((5, 23, 19), (16, 48, 39)),
                ((4, 47, 39), (17, 24, 19)),
                ((4, 12, 19), (17, 59, 38)),
            ]),
            (FREIBURG, (2026, 6, 21), (11, 30, 25), [
                ((3, 28, 59), (19, 31, 51)),
                ((2, 47, 46), (20, 13, 4)),
                ((1, 50, 5), (21, 10, 45)),
                ((0, 10, 38), (22, 50, 11)),
            ]),
            (FREIBURG, (2026, 3, 29), (11, 33, 21), [
                ((5, 12, 44), (17, 53, 58)),
                ((4, 41, 33), (18, 25, 10)),
                ((4, 4, 17), (19, 2, 25)),
                ((3, 25, 15), (19, 41, 27)),
            ]),
        ];
        let freiburg_dec = ((2026, 12, 21), (11, 26, 39), ((7, 15, 26), (15, 37, 52)));
        for ((lat, lon), date, noon, events) in cases {
            let d = day(date, lat, lon);
            let at = |(h, m, s): Hms| hms(date, h, m, s);
            let what = |w: &str| format!("{w} {lat},{lon} {date:?}");
            close(Some(d.solar_noon), at(noon), 60, &what("noon"));
            let pairs = [d.sun, d.civil, d.nautical, d.astronomical];
            for (i, (pair, (rise, set))) in pairs.iter().zip(events).enumerate() {
                let tol = if i == 3 && date.1 == 6 { 180 } else { 60 };
                close(pair.morning, at(rise), tol, &what(&format!("morning {i}")));
                close(pair.evening, at(set), tol, &what(&format!("evening {i}")));
            }
        }
        let (date, noon, (rise, set)) = freiburg_dec;
        let d = day(date, FREIBURG.0, FREIBURG.1);
        close(Some(d.solar_noon), hms(date, noon.0, noon.1, noon.2), 60, "noon Freiburg Dec");
        close(d.sun.morning, hms(date, rise.0, rise.1, rise.2), 60, "rise Freiburg Dec");
        close(d.sun.evening, hms(date, set.0, set.1, set.2), 60, "set Freiburg Dec");
    }

    /// The NREL SPA paper's reference instant (Reda & Andreas 2004):
    /// 2003-10-17 12:30:30 MST (19:30:30 UTC) at 39.742476 N, 105.1786 W:
    /// zenith 50.11162°, azimuth 194.34024°. The NOAA algorithm agrees
    /// within 0.05° (its refraction is for standard air).
    #[test]
    fn position_matches_the_spa_reference_point() {
        let ms = hms((2003, 10, 17), 19, 30, 30);
        let p = position(ms, 39.742476, -105.1786);
        assert!((90.0 - p.elevation_deg - 50.11162).abs() < 0.05, "{p:?}");
        assert!((p.azimuth_deg - 194.34024).abs() < 0.05, "{p:?}");
    }

    #[test]
    fn position_is_consistent_with_the_events() {
        let date = (2026, 6, 21);
        let d = day(date, VADUZ.0, VADUZ.1);
        // At sunrise the true centre is 0.833° below the horizon (radius
        // and refraction: the upper limb appears on the horizon).
        let rise = position(d.sun.morning.unwrap(), VADUZ.0, VADUZ.1);
        assert!((rise.true_elevation_deg + 0.833).abs() < 0.02, "{rise:?}");
        assert!((45.0..60.0).contains(&rise.azimuth_deg), "north-east in June: {rise:?}");
        let set = position(d.sun.evening.unwrap(), VADUZ.0, VADUZ.1);
        assert!((300.0..315.0).contains(&set.azimuth_deg), "north-west: {set:?}");
        // At noon the sun is due south, at 90 - lat + declination (23.44).
        let noon = position(d.solar_noon, VADUZ.0, VADUZ.1);
        assert!((noon.azimuth_deg - 180.0).abs() < 0.3, "{noon:?}");
        assert!((noon.true_elevation_deg - (90.0 - VADUZ.0 + 23.44)).abs() < 0.05);
        let civil = position(d.civil.evening.unwrap(), VADUZ.0, VADUZ.1);
        assert!((civil.true_elevation_deg + 6.0).abs() < 0.02);
        // Midnight: below the horizon, roughly north.
        let night = position(hms(date, 23, 23, 0), VADUZ.0, VADUZ.1);
        assert!(night.elevation_deg < -15.0 && (night.azimuth_deg - 360.0).abs().min(night.azimuth_deg) < 5.0, "{night:?}");
        let v = noon.direction_enu();
        assert!(v[2] < 0.0 && v[1] > 0.9, "south and high: {v:?}");
    }

    #[test]
    fn polar_day_and_night_have_no_sunrise() {
        let summer = day((2026, 6, 21), 78.22, 15.65); // Longyearbyen
        assert_eq!(summer.sun, Pair { morning: None, evening: None });
        let winter = day((2026, 12, 21), 78.22, 15.65);
        assert_eq!(winter.sun.morning, None);
        // No astronomical night at 52.5° N in June (the sun stays above -18°).
        assert_eq!(day((2026, 6, 21), 52.5, 13.4).astronomical.morning, None);
    }

    #[test]
    fn refraction_near_the_horizon() {
        assert!((refraction_deg(0.0) - 0.482).abs() < 0.01);
        assert!(refraction_deg(45.0) < 0.02);
        assert_eq!(refraction_deg(89.0), 0.0);
    }
}
