//! The planets seen by eye (spec §14 Phase 9, S2): Mercury, Venus, Mars,
//! Jupiter and Saturn from Keplerian elements with their linear rates
//! (E. M. Standish, JPL, *Approximate Positions of the Planets*, table 1:
//! J2000 ecliptic elements fitted to DE405 for 1800–2050, a few arc
//! minutes there and slowly worse to 2100 – far inside the degree a rider
//! can judge), light time, precession to the date, and the visual
//! magnitudes of the *Astronomical Almanac* (Meeus ch. 41). Published
//! orbital constants, licence-free.

use crate::astro::{self, Equatorial};
use crate::time::UnixMs;

/// A planet visible to the eye.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Planet {
    Mercury,
    Venus,
    Mars,
    Jupiter,
    Saturn,
}

/// Orbital elements at J2000 and their rate per Julian century:
/// a (AU), e, I (°), L (°), long. of perihelion ϖ (°), long. of node Ω (°).
type Elements = [(f64, f64); 6];

#[rustfmt::skip]
const MERCURY: Elements = [
    (0.387_099_27, 0.000_000_37), (0.205_635_93, 0.000_019_06), (7.004_979_02, -0.005_947_49),
    (252.250_323_50, 149_472.674_111_75), (77.457_796_28, 0.160_476_89), (48.330_765_93, -0.125_340_81),
];
#[rustfmt::skip]
const VENUS: Elements = [
    (0.723_335_66, 0.000_003_90), (0.006_776_72, -0.000_041_07), (3.394_676_05, -0.000_788_90),
    (181.979_099_50, 58_517.815_387_29), (131.602_467_18, 0.002_683_29), (76.679_842_55, -0.277_694_18),
];
#[rustfmt::skip]
const EARTH: Elements = [
    (1.000_002_61, 0.000_005_62), (0.016_711_23, -0.000_043_92), (-0.000_015_31, -0.012_946_68),
    (100.464_571_66, 35_999.372_449_81), (102.937_681_93, 0.323_273_64), (0.0, 0.0),
];
#[rustfmt::skip]
const MARS: Elements = [
    (1.523_710_34, 0.000_018_47), (0.093_394_10, 0.000_078_82), (1.849_691_42, -0.008_131_31),
    (-4.553_432_05, 19_140.302_684_99), (-23.943_629_59, 0.444_410_88), (49.559_538_91, -0.292_573_43),
];
#[rustfmt::skip]
const JUPITER: Elements = [
    (5.202_887_00, -0.000_116_07), (0.048_386_24, -0.000_132_53), (1.304_396_95, -0.001_837_14),
    (34.396_440_51, 3_034.746_127_75), (14.728_479_83, 0.212_526_68), (100.473_909_09, 0.204_691_06),
];
#[rustfmt::skip]
const SATURN: Elements = [
    (9.536_675_94, -0.001_250_60), (0.053_861_79, -0.000_509_91), (2.485_991_87, 0.001_936_09),
    (49.954_244_23, 1_222.493_622_01), (92.598_878_31, -0.418_972_16), (113.662_424_48, -0.288_677_94),
];

impl Planet {
    pub const ALL: [Planet; 5] = [
        Planet::Mercury,
        Planet::Venus,
        Planet::Mars,
        Planet::Jupiter,
        Planet::Saturn,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Mercury => "Mercury",
            Self::Venus => "Venus",
            Self::Mars => "Mars",
            Self::Jupiter => "Jupiter",
            Self::Saturn => "Saturn",
        }
    }

    fn elements(self) -> &'static Elements {
        match self {
            Self::Mercury => &MERCURY,
            Self::Venus => &VENUS,
            Self::Mars => &MARS,
            Self::Jupiter => &JUPITER,
            Self::Saturn => &SATURN,
        }
    }

    /// Colour as seen by eye (linear RGB, max 1): Mars ruddy, Saturn and
    /// Jupiter cream, Venus and Mercury white.
    pub fn colour(self) -> [f32; 3] {
        match self {
            Self::Mercury => [1.0, 0.93, 0.85],
            Self::Venus => [1.0, 0.98, 0.92],
            Self::Mars => [1.0, 0.55, 0.32],
            Self::Jupiter => [1.0, 0.92, 0.78],
            Self::Saturn => [1.0, 0.86, 0.62],
        }
    }

    /// Visual magnitude for distances r (sun), Δ (earth) in AU and the
    /// phase angle `i` (°): the Astronomical Almanac formulas (Meeus 41);
    /// Saturn with the rings' average contribution (-0.6 mag at a typical
    /// tilt; it varies by ±0.6 over the 29-year ring cycle).
    pub fn magnitude(self, r: f64, delta: f64, i: f64) -> f64 {
        let d = 5.0 * (r * delta).log10();
        match self {
            Self::Mercury => -0.42 + d + 0.0380 * i - 0.000_273 * i * i + 0.000_002 * i.powi(3),
            Self::Venus => -4.40 + d + 0.0009 * i + 0.000_239 * i * i - 0.000_000_65 * i.powi(3),
            Self::Mars => -1.52 + d + 0.016 * i,
            Self::Jupiter => -9.40 + d + 0.005 * i,
            Self::Saturn => -8.88 + d - 0.6,
        }
    }
}

/// Heliocentric ecliptic position (J2000, AU) from elements at `t`
/// centuries since J2000 (TT).
fn heliocentric(el: &Elements, t: f64) -> [f64; 3] {
    let v = |k: usize| el[k].0 + el[k].1 * t;
    let (a, e, inc, l, peri, node) = (v(0), v(1), v(2), v(3), v(4), v(5));
    let m = astro::wrap180(l - peri).to_radians();
    let w = (peri - node).to_radians();
    // Kepler's equation by Newton's method.
    let mut big_e = m + e * m.sin();
    for _ in 0..12 {
        let d = (big_e - e * big_e.sin() - m) / (1.0 - e * big_e.cos());
        big_e -= d;
        if d.abs() < 1e-12 {
            break;
        }
    }
    let xp = a * (big_e.cos() - e);
    let yp = a * (1.0 - e * e).sqrt() * big_e.sin();
    let (sw, cw) = w.sin_cos();
    let (so, co) = node.to_radians().sin_cos();
    let (si, ci) = inc.to_radians().sin_cos();
    [
        (cw * co - sw * so * ci) * xp + (-sw * co - cw * so * ci) * yp,
        (cw * so + sw * co * ci) * xp + (-sw * so + cw * co * ci) * yp,
        (sw * si) * xp + (cw * si) * yp,
    ]
}

fn norm(v: [f64; 3]) -> f64 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

/// A planet seen from the earth's centre.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geocentric {
    /// Ecliptic longitude / latitude of the date (°).
    pub longitude_deg: f64,
    pub latitude_deg: f64,
    pub equatorial: Equatorial,
    /// Distances to the sun and the earth (AU).
    pub r_au: f64,
    pub delta_au: f64,
    /// Sun – planet – earth angle (°).
    pub phase_angle_deg: f64,
    pub magnitude: f64,
}

/// Light time per AU (days).
const LIGHT_DAYS_PER_AU: f64 = 0.005_775_518_3;

/// The planet at `t` centuries since J2000 (TT).
pub fn geocentric_t(planet: Planet, t: f64) -> Geocentric {
    let earth = heliocentric(&EARTH, t);
    let mut p = heliocentric(planet.elements(), t);
    let mut g = [p[0] - earth[0], p[1] - earth[1], p[2] - earth[2]];
    // One step of light time.
    let tau = norm(g) * LIGHT_DAYS_PER_AU / 36_525.0;
    p = heliocentric(planet.elements(), t - tau);
    g = [p[0] - earth[0], p[1] - earth[1], p[2] - earth[2]];
    let (r, delta, big_r) = (norm(p), norm(g), norm(earth));
    let lon = g[1].atan2(g[0]).to_degrees();
    let lat = (g[2] / delta).asin().to_degrees();
    let (dpsi, _) = astro::nutation(t);
    let lon = (lon + astro::precession_lon(t) + dpsi).rem_euclid(360.0);
    let i = ((r * r + delta * delta - big_r * big_r) / (2.0 * r * delta))
        .clamp(-1.0, 1.0)
        .acos()
        .to_degrees();
    Geocentric {
        longitude_deg: lon,
        latitude_deg: lat,
        equatorial: astro::ecliptic_to_equatorial(lon, lat, astro::true_obliquity(t)),
        r_au: r,
        delta_au: delta,
        phase_angle_deg: i,
        magnitude: planet.magnitude(r, delta, i),
    }
}

/// A planet at an instant (UT), seen from a place.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlanetPosition {
    pub planet: Planet,
    pub azimuth_deg: f64,
    /// With atmospheric refraction.
    pub elevation_deg: f64,
    pub geocentric: Geocentric,
}

impl PlanetPosition {
    pub fn magnitude(&self) -> f64 {
        self.geocentric.magnitude
    }
}

pub fn position(planet: Planet, ms: UnixMs, lat: f64, lon: f64) -> PlanetPosition {
    let g = geocentric_t(planet, astro::centuries_tt(ms));
    let (az, el) = astro::horizontal(g.equatorial, ms, lat, lon);
    PlanetPosition {
        planet,
        azimuth_deg: az,
        elevation_deg: el + crate::sun::refraction_deg(el),
        geocentric: g,
    }
}

/// All five planets.
pub fn all(ms: UnixMs, lat: f64, lon: f64) -> [PlanetPosition; 5] {
    Planet::ALL.map(|p| position(p, ms, lat, lon))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::{MS_PER_HOUR, MS_PER_MINUTE, julian_day, utc_midnight};

    fn utc(y: i32, mo: u32, d: u32, h: i64, mi: i64) -> UnixMs {
        utc_midnight(y, mo, d) + h * MS_PER_HOUR + mi * MS_PER_MINUTE
    }

    /// Meeus example 33.a: Venus on 1992 December 20, 0h TD: apparent
    /// α 21h04m41.454s, δ -18°53'16.84"; distance 0.910947 AU.
    #[test]
    fn venus_matches_meeus_example() {
        let t = (2_448_976.5 - 2_451_545.0) / 36_525.0;
        let g = geocentric_t(Planet::Venus, t);
        let ra = (21.0 + 4.0 / 60.0 + 41.454 / 3600.0) * 15.0;
        let dec = -(18.0 + 53.0 / 60.0 + 16.84 / 3600.0);
        let sep = astro::separation(
            g.equatorial,
            Equatorial {
                ra_deg: ra,
                dec_deg: dec,
            },
        );
        eprintln!("Venus 1992-12-20: {:.4}° from Meeus", sep);
        assert!(sep < 0.05, "{sep}° {g:?}");
        assert!((g.delta_au - 0.910_947).abs() < 0.001, "{g:?}");
        // Bright evening star (example 41.a: about -3.8 to -4.4).
        assert!((-4.6..-3.7).contains(&g.magnitude), "{}", g.magnitude);
    }

    /// The great conjunction of 2020-12-21 (18:22 UTC): Jupiter and Saturn
    /// 0.1° apart.
    #[test]
    fn jupiter_and_saturn_meet_in_december_2020() {
        let at = utc(2020, 12, 21, 18, 22);
        let t = astro::centuries_tt(at);
        let (j, s) = (
            geocentric_t(Planet::Jupiter, t),
            geocentric_t(Planet::Saturn, t),
        );
        let sep = astro::separation(j.equatorial, s.equatorial);
        eprintln!("great conjunction 2020-12-21: {sep:.3}° (true 0.10°)");
        assert!(sep < 0.25, "{sep}");
        assert!((-2.3..-1.7).contains(&j.magnitude), "{}", j.magnitude);
        assert!((0.2..1.0).contains(&s.magnitude), "{}", s.magnitude);
    }

    /// Oppositions and conjunctions: the planet's longitude 180° from the
    /// sun (Mars 2020-10-13 23:20 UTC, Jupiter 2026-01-10 08 UTC) or on it
    /// (Venus inferior conjunction 2022-01-09 01:47 UTC, Mercury superior
    /// conjunction 2026-02-21).
    #[test]
    fn oppositions_and_conjunctions_within_a_degree() {
        let cases = [
            (Planet::Mars, utc(2020, 10, 13, 23, 20), 180.0),
            (Planet::Jupiter, utc(2026, 1, 10, 8, 0), 180.0),
            (Planet::Venus, utc(2022, 1, 9, 1, 47), 0.0),
        ];
        for (p, at, want) in cases {
            let g = geocentric_t(p, astro::centuries_tt(at));
            let sun = crate::sun::geocentric(julian_day(at)).longitude_deg;
            let d = astro::wrap180(g.longitude_deg - sun - want);
            eprintln!("{p:?} opposition / conjunction: {d:+.3}°");
            assert!(d.abs() < 1.0, "{p:?}: {d}°");
        }
        let mars = geocentric_t(Planet::Mars, astro::centuries_tt(cases[0].1));
        assert!((-2.9..-2.3).contains(&mars.magnitude), "{}", mars.magnitude);
        assert!(mars.phase_angle_deg < 3.0);
    }

    #[test]
    fn evening_venus_in_summer_2026() {
        // Venus is the evening star in July 2026: above the western
        // horizon after sunset in Vaduz.
        let at = utc(2026, 7, 16, 19, 45);
        let v = position(Planet::Venus, at, 47.141, 9.521);
        assert!(v.elevation_deg > 3.0, "{v:?}");
        assert!((250.0..310.0).contains(&v.azimuth_deg), "{v:?}");
        assert!(v.magnitude() < -3.9);
        for p in all(at, 47.141, 9.521) {
            assert!(p.geocentric.delta_au > 0.2 && p.geocentric.delta_au < 11.5);
        }
    }
}
