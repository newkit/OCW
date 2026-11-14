//! The moon (spec §14 Phase 9, S2): position from the truncated ELP-2000/82
//! series of Meeus (*Astronomical Algorithms*, ch. 47: the 60 + 60 main
//! periodic terms, about 10" in longitude and 4" in latitude), phase and
//! illuminated fraction, the position angle of the bright limb (ch. 48),
//! the instants of the main phases (solved from the elongation), and moon
//! rise / set at a place (the upper limb on the horizon with parallax and
//! refraction).
//!
//! The series coefficients are published constants of the lunar theory
//! (Chapront-Touzé & Chapront 1983, as truncated by Meeus); they are facts,
//! typed in here, not a copied data file.

use crate::astro::{self, Equatorial};
use crate::sun;
use crate::time::{MS_PER_DAY, MS_PER_MINUTE, UnixMs};

/// Mean length of a lunation (synodic month), days.
pub const SYNODIC_MONTH_D: f64 = 29.530_588_861;
/// Equatorial radius of the earth and the moon's radius (km).
const EARTH_RADIUS_KM: f64 = 6378.14;
pub const MOON_RADIUS_KM: f64 = 1737.4;
/// Astronomical unit (km).
const AU_KM: f64 = 149_597_870.7;

/// Periodic terms for longitude (1e-6 °) and distance (1e-3 km):
/// multiples of D, M, M', F (Meeus table 47.A).
#[rustfmt::skip]
const LR: [(i8, i8, i8, i8, i32, i32); 60] = [
    (0, 0, 1, 0, 6_288_774, -20_905_355), (2, 0, -1, 0, 1_274_027, -3_699_111),
    (2, 0, 0, 0, 658_314, -2_955_968), (0, 0, 2, 0, 213_618, -569_925),
    (0, 1, 0, 0, -185_116, 48_888), (0, 0, 0, 2, -114_332, -3_149),
    (2, 0, -2, 0, 58_793, 246_158), (2, -1, -1, 0, 57_066, -152_138),
    (2, 0, 1, 0, 53_322, -170_733), (2, -1, 0, 0, 45_758, -204_586),
    (0, 1, -1, 0, -40_923, -129_620), (1, 0, 0, 0, -34_720, 108_743),
    (0, 1, 1, 0, -30_383, 104_755), (2, 0, 0, -2, 15_327, 10_321),
    (0, 0, 1, 2, -12_528, 0), (0, 0, 1, -2, 10_980, 79_661),
    (4, 0, -1, 0, 10_675, -34_782), (0, 0, 3, 0, 10_034, -23_210),
    (4, 0, -2, 0, 8_548, -21_636), (2, 1, -1, 0, -7_888, 24_208),
    (2, 1, 0, 0, -6_766, 30_824), (1, 0, -1, 0, -5_163, -8_379),
    (1, 1, 0, 0, 4_987, -16_675), (2, -1, 1, 0, 4_036, -12_831),
    (2, 0, 2, 0, 3_994, -10_445), (4, 0, 0, 0, 3_861, -11_650),
    (2, 0, -3, 0, 3_665, 14_403), (0, 1, -2, 0, -2_689, -7_003),
    (2, 0, -1, 2, -2_602, 0), (2, -1, -2, 0, 2_390, 10_056),
    (1, 0, 1, 0, -2_348, 6_322), (2, -2, 0, 0, 2_236, -9_884),
    (0, 1, 2, 0, -2_120, 5_751), (0, 2, 0, 0, -2_069, 0),
    (2, -2, -1, 0, 2_048, -4_950), (2, 0, 1, -2, -1_773, 4_130),
    (2, 0, 0, 2, -1_595, 0), (4, -1, -1, 0, 1_215, -3_958),
    (0, 0, 2, 2, -1_110, 0), (3, 0, -1, 0, -892, 3_258),
    (2, 1, 1, 0, -810, 2_616), (4, -1, -2, 0, 759, -1_897),
    (0, 2, -1, 0, -713, -2_117), (2, 2, -1, 0, -700, 2_354),
    (2, 1, -2, 0, 691, 0), (2, -1, 0, -2, 596, 0),
    (4, 0, 1, 0, 549, -1_423), (0, 0, 4, 0, 537, -1_117),
    (4, -1, 0, 0, 520, -1_571), (1, 0, -2, 0, -487, -1_739),
    (2, 1, 0, -2, -399, 0), (0, 0, 2, -2, -381, -4_421),
    (1, 1, 1, 0, 351, 0), (3, 0, -2, 0, -340, 0),
    (4, 0, -3, 0, 330, 0), (2, -1, 2, 0, 327, 0),
    (0, 2, 1, 0, -323, 1_165), (1, 1, -1, 0, 299, 0),
    (2, 0, 3, 0, 294, 0), (2, 0, -1, -2, 0, 8_752),
];

/// Periodic terms for latitude (1e-6 °; Meeus table 47.B).
#[rustfmt::skip]
const B: [(i8, i8, i8, i8, i32); 60] = [
    (0, 0, 0, 1, 5_128_122), (0, 0, 1, 1, 280_602), (0, 0, 1, -1, 277_693),
    (2, 0, 0, -1, 173_237), (2, 0, -1, 1, 55_413), (2, 0, -1, -1, 46_271),
    (2, 0, 0, 1, 32_573), (0, 0, 2, 1, 17_198), (2, 0, 1, -1, 9_266),
    (0, 0, 2, -1, 8_822), (2, -1, 0, -1, 8_216), (2, 0, -2, -1, 4_324),
    (2, 0, 1, 1, 4_200), (2, 1, 0, -1, -3_359), (2, -1, -1, 1, 2_463),
    (2, -1, 0, 1, 2_211), (2, -1, -1, -1, 2_065), (0, 1, -1, -1, -1_870),
    (4, 0, -1, -1, 1_828), (0, 1, 0, 1, -1_794), (0, 0, 0, 3, -1_749),
    (0, 1, -1, 1, -1_565), (1, 0, 0, 1, -1_491), (0, 1, 1, 1, -1_475),
    (0, 1, 1, -1, -1_410), (0, 1, 0, -1, -1_344), (1, 0, 0, -1, -1_335),
    (0, 0, 3, 1, 1_107), (4, 0, 0, -1, 1_021), (4, 0, -1, 1, 833),
    (0, 0, 1, -3, 777), (4, 0, -2, 1, 671), (2, 0, 0, -3, 607),
    (2, 0, 2, -1, 596), (2, -1, 1, -1, 491), (2, 0, -2, 1, -451),
    (0, 0, 3, -1, 439), (2, 0, 2, 1, 422), (2, 0, -3, -1, 421),
    (2, 1, -1, 1, -366), (2, 1, 0, 1, -351), (4, 0, 0, 1, 331),
    (2, -1, 1, 1, 315), (2, -2, 0, -1, 302), (0, 0, 1, 3, -283),
    (2, 1, 1, -1, -229), (1, 1, 0, -1, 223), (1, 1, 0, 1, 223),
    (0, 1, -2, -1, -220), (2, 1, -1, -1, -220), (1, 0, 1, 1, -185),
    (2, -1, -2, -1, 181), (0, 1, 2, 1, -177), (4, 0, -2, -1, 176),
    (4, -1, -1, -1, 166), (1, 0, 1, -1, -164), (4, 0, 1, -1, 132),
    (1, 0, -1, -1, -119), (4, -1, 0, -1, 115), (2, -2, 0, 1, 107),
];

/// The moon seen from the earth's centre.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geocentric {
    /// Apparent ecliptic longitude and latitude of the date (°).
    pub longitude_deg: f64,
    pub latitude_deg: f64,
    pub distance_km: f64,
    pub equatorial: Equatorial,
}

/// Geocentric moon at Julian Ephemeris Day `jde` (Meeus 47).
pub fn geocentric_jde(jde: f64) -> Geocentric {
    let t = (jde - 2_451_545.0) / 36_525.0;
    let lp = 218.316_447_7 + 481_267.881_234_21 * t - 0.001_578_6 * t * t + t.powi(3) / 538_841.0
        - t.powi(4) / 65_194_000.0;
    let d = 297.850_192_1 + 445_267.111_403_4 * t - 0.001_881_9 * t * t + t.powi(3) / 545_868.0
        - t.powi(4) / 113_065_000.0;
    let m = 357.529_109_2 + 35_999.050_290_9 * t - 0.000_153_6 * t * t + t.powi(3) / 24_490_000.0;
    let mp = 134.963_396_4 + 477_198.867_505_5 * t + 0.008_741_4 * t * t + t.powi(3) / 69_699.0
        - t.powi(4) / 14_712_000.0;
    let f = 93.272_095_0 + 483_202.017_523_3 * t - 0.003_653_9 * t * t - t.powi(3) / 3_526_000.0
        + t.powi(4) / 863_310_000.0;
    let a1 = (119.75 + 131.849 * t).to_radians();
    let a2 = (53.09 + 479_264.290 * t).to_radians();
    let a3 = (313.45 + 481_266.484 * t).to_radians();
    let e = 1.0 - 0.002_516 * t - 0.000_007_4 * t * t;
    let ecc = |mm: i8| match mm.abs() {
        1 => e,
        2 => e * e,
        _ => 1.0,
    };
    let arg = |dd: i8, mm: i8, mpp: i8, ff: i8| {
        (f64::from(dd) * d + f64::from(mm) * m + f64::from(mpp) * mp + f64::from(ff) * f)
            .to_radians()
    };
    let (mut sl, mut sr) = (0.0, 0.0);
    for &(dd, mm, mpp, ff, l, r) in &LR {
        let a = arg(dd, mm, mpp, ff);
        sl += f64::from(l) * ecc(mm) * a.sin();
        sr += f64::from(r) * ecc(mm) * a.cos();
    }
    let mut sb = 0.0;
    for &(dd, mm, mpp, ff, b) in &B {
        sb += f64::from(b) * ecc(mm) * arg(dd, mm, mpp, ff).sin();
    }
    let (lpr, fr, mpr) = (lp.to_radians(), f.to_radians(), mp.to_radians());
    sl += 3958.0 * a1.sin() + 1962.0 * (lpr - fr).sin() + 318.0 * a2.sin();
    sb += -2235.0 * lpr.sin()
        + 382.0 * a3.sin()
        + 175.0 * (a1 - fr).sin()
        + 175.0 * (a1 + fr).sin()
        + 127.0 * (lpr - mpr).sin()
        - 115.0 * (lpr + mpr).sin();
    let (dpsi, deps) = astro::nutation(t);
    let longitude_deg = (lp + sl / 1e6 + dpsi).rem_euclid(360.0);
    let latitude_deg = sb / 1e6;
    let eps = astro::mean_obliquity(t) + deps;
    Geocentric {
        longitude_deg,
        latitude_deg,
        distance_km: 385_000.56 + sr / 1000.0,
        equatorial: astro::ecliptic_to_equatorial(longitude_deg, latitude_deg, eps),
    }
}

/// Geocentric moon at an instant (UT).
pub fn geocentric(ms: UnixMs) -> Geocentric {
    geocentric_jde(astro::jde(ms))
}

/// The moon seen from a place.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MoonPosition {
    /// Clockwise from true north, 0–360°.
    pub azimuth_deg: f64,
    /// Where it is seen: topocentric (parallax, up to ~1°) with
    /// atmospheric refraction.
    pub elevation_deg: f64,
    /// Geocentric, without parallax and refraction.
    pub geocentric: Geocentric,
    /// Share of the disc that is lit (0 new … 1 full).
    pub illuminated: f64,
    /// Sun – moon – earth angle (0 full … 180° new).
    pub phase_angle_deg: f64,
    /// Position angle of the bright limb's midpoint, from the celestial
    /// north through east (Meeus 48.5).
    pub bright_limb_deg: f64,
    /// Waxing (the lit side towards the evening sun).
    pub waxing: bool,
    /// Apparent diameter (°).
    pub diameter_deg: f64,
}

impl MoonPosition {
    /// Unit vector towards the moon in east / up / north components.
    pub fn direction_enu(&self) -> [f64; 3] {
        let (az, el) = (
            self.azimuth_deg.to_radians(),
            self.elevation_deg.to_radians(),
        );
        [el.cos() * az.sin(), el.sin(), el.cos() * az.cos()]
    }

    /// "waxing crescent", "full moon", …
    pub fn phase_name(&self) -> &'static str {
        let k = self.illuminated;
        match (k, self.waxing) {
            (k, _) if k < 0.03 => "new moon",
            (k, _) if k > 0.97 => "full moon",
            (k, true) if k < 0.47 => "waxing crescent",
            (k, true) if k <= 0.53 => "first quarter",
            (_, true) => "waxing gibbous",
            (k, false) if k < 0.47 => "waning crescent",
            (k, false) if k <= 0.53 => "last quarter",
            (_, false) => "waning gibbous",
        }
    }
}

/// Phase angle (°) and the bright limb's position angle (°) of the moon
/// `m` with the sun `s` (Meeus 48.2–48.5), and whether it is waxing.
fn phase(m: &Geocentric, s: &sun::Geocentric) -> (f64, f64, bool) {
    let (b, dl) = (
        m.latitude_deg.to_radians(),
        (m.longitude_deg - s.longitude_deg).to_radians(),
    );
    let psi = (b.cos() * dl.cos()).clamp(-1.0, 1.0).acos();
    let r = s.distance_au * AU_KM;
    let i = (r * psi.sin()).atan2(m.distance_km - r * psi.cos());
    let (a0, d0) = (
        s.equatorial.ra_deg.to_radians(),
        s.equatorial.dec_deg.to_radians(),
    );
    let (a, d) = (
        m.equatorial.ra_deg.to_radians(),
        m.equatorial.dec_deg.to_radians(),
    );
    let chi =
        (d0.cos() * (a0 - a).sin()).atan2(d0.sin() * d.cos() - d0.cos() * d.sin() * (a0 - a).cos());
    let waxing = astro::wrap180(m.longitude_deg - s.longitude_deg) > 0.0;
    (i.to_degrees(), chi.to_degrees().rem_euclid(360.0), waxing)
}

/// The moon at instant `ms` from latitude `lat` / longitude `lon` (°).
pub fn position(ms: UnixMs, lat: f64, lon: f64) -> MoonPosition {
    let g = geocentric(ms);
    let s = sun::geocentric(crate::time::julian_day(ms));
    let (az, el_geo) = astro::horizontal(g.equatorial, ms, lat, lon);
    let parallax = (EARTH_RADIUS_KM / g.distance_km).asin().to_degrees();
    let el_topo = el_geo - parallax * el_geo.to_radians().cos();
    let (i, chi, waxing) = phase(&g, &s);
    MoonPosition {
        azimuth_deg: az,
        elevation_deg: el_topo + sun::refraction_deg(el_topo),
        geocentric: g,
        illuminated: (1.0 + i.to_radians().cos()) / 2.0,
        phase_angle_deg: i,
        bright_limb_deg: chi,
        waxing,
        diameter_deg: 2.0 * (MOON_RADIUS_KM / g.distance_km).asin().to_degrees(),
    }
}

/// The main phases, by the moon's elongation in longitude from the sun.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    New,
    FirstQuarter,
    Full,
    LastQuarter,
}

impl Phase {
    fn elongation(self) -> f64 {
        match self {
            Self::New => 0.0,
            Self::FirstQuarter => 90.0,
            Self::Full => 180.0,
            Self::LastQuarter => 270.0,
        }
    }
}

/// Moon minus sun in apparent longitude (0–360°) at an instant.
pub fn elongation_lon(ms: UnixMs) -> f64 {
    let m = geocentric(ms);
    let s = sun::geocentric(crate::time::julian_day(ms));
    (m.longitude_deg - s.longitude_deg).rem_euclid(360.0)
}

/// The first instant after `ms` when the moon reaches `phase` (to a few
/// seconds: Newton steps on the elongation, ~12.2° per day).
pub fn next_phase(ms: UnixMs, phase: Phase) -> UnixMs {
    let rate = 360.0 / SYNODIC_MONTH_D; // ° per day
    let ahead = (phase.elongation() - elongation_lon(ms)).rem_euclid(360.0);
    // Never the instant itself: a phase just past is a month away.
    let ahead = if ahead < 1e-6 { 360.0 } else { ahead };
    let mut t = ms + (ahead / rate * MS_PER_DAY as f64) as i64;
    for _ in 0..8 {
        let off = astro::wrap180(phase.elongation() - elongation_lon(t));
        t += (off / rate * MS_PER_DAY as f64) as i64;
        if off.abs() < 1e-5 {
            break;
        }
    }
    t
}

/// The upper limb's apparent height above the horizon (°).
fn limb_height(ms: UnixMs, lat: f64, lon: f64) -> f64 {
    let p = position(ms, lat, lon);
    p.elevation_deg + p.diameter_deg / 2.0
}

/// Moon rise and set within `[from, from + 1 day)` at a place (the upper
/// limb crossing the horizon, parallax and refraction included), to the
/// minute; `None` when it does not rise or set in that day (it happens
/// about once a month: the moon is ~50 min later each day).
pub fn rise_set(from: UnixMs, lat: f64, lon: f64) -> (Option<UnixMs>, Option<UnixMs>) {
    let step = 10 * MS_PER_MINUTE;
    let (mut rise, mut set) = (None, None);
    let mut t0 = from;
    let mut h0 = limb_height(t0, lat, lon);
    while t0 < from + MS_PER_DAY {
        let t1 = (t0 + step).min(from + MS_PER_DAY);
        let h1 = limb_height(t1, lat, lon);
        if (h0 < 0.0) != (h1 < 0.0) {
            let (mut a, mut b) = (t0, t1);
            while b - a > 30_000 {
                let mid = (a + b) / 2;
                if (limb_height(mid, lat, lon) < 0.0) == (h0 < 0.0) {
                    a = mid;
                } else {
                    b = mid;
                }
            }
            let at = (a + b) / 2;
            if h0 < 0.0 {
                rise.get_or_insert(at);
            } else {
                set.get_or_insert(at);
            }
        }
        t0 = t1;
        h0 = h1;
    }
    (rise, set)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::{MS_PER_HOUR, julian_day, utc_midnight};

    fn utc(y: i32, mo: u32, d: u32, h: i64, mi: i64) -> UnixMs {
        utc_midnight(y, mo, d) + h * MS_PER_HOUR + mi * MS_PER_MINUTE
    }

    /// Meeus example 47.a: 1992 April 12, 0h TD.
    #[test]
    fn position_matches_meeus_example() {
        let g = geocentric_jde(2_448_724.5);
        // λ 133.162655 (apparent: + Δψ 0.004610 = 133.167265), β -3.229126,
        // Δ 368409.7 km, α 134.688470, δ 13.768368.
        assert!((g.longitude_deg - 133.167_265).abs() < 0.001, "{g:?}");
        assert!((g.latitude_deg + 3.229_126).abs() < 0.001, "{g:?}");
        assert!((g.distance_km - 368_409.7).abs() < 1.0, "{g:?}");
        assert!((g.equatorial.ra_deg - 134.688_470).abs() < 0.002, "{g:?}");
        assert!((g.equatorial.dec_deg - 13.768_368).abs() < 0.002, "{g:?}");
    }

    /// Meeus example 48.a: the same instant, illuminated fraction 0.6786,
    /// phase angle 69.0756°, bright limb at 285.0°.
    #[test]
    fn phase_matches_meeus_example() {
        let jde = 2_448_724.5;
        let g = geocentric_jde(jde);
        let s = sun::geocentric(jde - 59.0 / 86_400.0);
        let (i, chi, waxing) = phase(&g, &s);
        assert!((i - 69.0756).abs() < 0.05, "{i}");
        assert!(((1.0 + i.to_radians().cos()) / 2.0 - 0.6786).abs() < 0.001);
        assert!((chi - 285.0).abs() < 0.5, "{chi}");
        assert!(waxing);
    }

    /// The full and new moons of 2026 (UTC, published tables; the
    /// eclipses of 17 Feb, 3 Mar, 12 Aug and 28 Aug pin them).
    #[test]
    fn the_phases_of_2026_within_minutes() {
        let full = [
            utc(2026, 1, 3, 10, 3),
            utc(2026, 2, 1, 22, 9),
            utc(2026, 3, 3, 11, 38),
            utc(2026, 4, 2, 2, 12),
            utc(2026, 5, 1, 17, 23),
            utc(2026, 5, 31, 8, 45),
            utc(2026, 6, 29, 23, 57),
            utc(2026, 7, 29, 14, 36),
            utc(2026, 8, 28, 4, 18),
            utc(2026, 9, 26, 16, 49),
            utc(2026, 10, 26, 4, 12),
            utc(2026, 11, 24, 14, 53),
            utc(2026, 12, 24, 1, 28),
        ];
        let new = [
            utc(2026, 1, 18, 19, 52),
            utc(2026, 2, 17, 12, 1),
            utc(2026, 3, 19, 1, 23),
            utc(2026, 4, 17, 11, 52),
            utc(2026, 5, 16, 20, 1),
            utc(2026, 6, 15, 2, 54),
            utc(2026, 7, 14, 9, 44),
            utc(2026, 8, 12, 17, 37),
            utc(2026, 9, 11, 3, 27),
            utc(2026, 10, 10, 15, 50),
            utc(2026, 11, 9, 7, 2),
            utc(2026, 12, 9, 0, 52),
        ];
        let mut worst: f64 = 0.0;
        for (phase, list) in [(Phase::Full, &full[..]), (Phase::New, &new[..])] {
            for &want in list {
                let got = next_phase(want - 5 * MS_PER_DAY, phase);
                let off_min = (got - want) as f64 / 60_000.0;
                worst = worst.max(off_min.abs());
                assert!(
                    off_min.abs() < 10.0,
                    "{phase:?} {want}: off {off_min:.1} min"
                );
                let k = position(got, 47.1, 9.5).illuminated;
                match phase {
                    Phase::Full => assert!(k > 0.995, "{k}"),
                    _ => assert!(k < 0.005, "{k}"),
                }
            }
        }
        eprintln!("phases of 2026: worst offset {worst:.1} min");
        assert!(worst < 10.0);
        // From a full moon itself: the next one, a month later.
        let f = full[0];
        let next = next_phase(f + 60_000, Phase::Full);
        assert!((next - full[1]).abs() < 10 * MS_PER_MINUTE);
        let q = next_phase(full[0], Phase::LastQuarter);
        let p = position(q, 47.1, 9.5);
        assert!((p.illuminated - 0.5).abs() < 0.02 && !p.waxing, "{p:?}");
        assert_eq!(p.phase_name(), "last quarter");
    }

    /// Positions against the instant of the eclipses: at the total solar
    /// eclipse of 2026-08-12 (greatest 17:46 UTC) the moon stands on the
    /// sun; at the total lunar eclipse of 2026-03-03 (11:33 UTC) opposite.
    #[test]
    fn eclipses_put_the_moon_on_and_opposite_the_sun() {
        let at = utc(2026, 8, 12, 17, 46);
        let m = geocentric(at);
        let s = sun::geocentric(julian_day(at));
        let sep = astro::separation(m.equatorial, s.equatorial);
        // Geocentric: within the sum of the radii plus the parallax.
        assert!(sep < 1.5, "{sep}");
        // Seen from the eclipse's path (Spain, Burgos area at 18:28 UTC):
        // within the sun's radius.
        let at = utc(2026, 8, 12, 18, 28);
        let (lat, lon) = (42.34, -3.70);
        let mp = position(at, lat, lon);
        let sp = sun::position(at, lat, lon);
        let d_az =
            astro::wrap180(mp.azimuth_deg - sp.azimuth_deg) * sp.elevation_deg.to_radians().cos();
        let d_el = mp.elevation_deg - sp.elevation_deg;
        let off = (d_az * d_az + d_el * d_el).sqrt();
        eprintln!("eclipse 2026-08-12 Burgos: moon {off:.3}° from the sun");
        assert!(off < 0.5, "moon {mp:?} sun {sp:?} off {off}");
        let at = utc(2026, 3, 3, 11, 33);
        let m = geocentric(at);
        let s = sun::geocentric(julian_day(at));
        let sep = astro::separation(m.equatorial, s.equatorial);
        assert!((sep - 180.0).abs() < 1.5, "{sep}");
    }

    #[test]
    fn rise_and_set_bracket_the_moon_above_the_horizon() {
        // Vaduz around the full moon of 24 Nov 2026: it rises near sunset
        // and sets near sunrise.
        let (lat, lon) = (47.141, 9.521);
        let day = utc_midnight(2026, 11, 24);
        let (rise, set) = rise_set(day, lat, lon);
        let rise = rise.expect("rises");
        let set = set.expect("sets");
        let sunset = sun::crossing((2026, 11, 24), lat, lon, sun::zenith::SUNRISE, false).unwrap();
        assert!(
            (rise - sunset).abs() < 2 * MS_PER_HOUR,
            "rise {} sunset {}",
            rise / 60_000,
            sunset / 60_000
        );
        assert!(position(rise + 30 * MS_PER_MINUTE, lat, lon).elevation_deg > 0.0);
        assert!(position(rise - 30 * MS_PER_MINUTE, lat, lon).elevation_deg < 0.0);
        assert!(position(set - 30 * MS_PER_MINUTE, lat, lon).elevation_deg > 0.0);
        // The next day it rises later, by roughly 30-80 minutes.
        let (next, _) = rise_set(day + MS_PER_DAY, lat, lon);
        let late = (next.unwrap() - rise) as f64 / 60_000.0 - 1440.0;
        assert!((20.0..90.0).contains(&late), "{late}");
        // Topocentric: the parallax lowers it by up to a degree.
        let p = position(rise + 4 * MS_PER_HOUR, lat, lon);
        assert!((0.48..0.58).contains(&p.diameter_deg), "{p:?}");
    }

    #[test]
    fn crescents_face_the_sun() {
        // An evening crescent (2 days after new, 2026-07-16 19:30 UTC in
        // Vaduz): lit towards the sun, which is below the western horizon.
        let at = utc(2026, 7, 16, 19, 30);
        let (lat, lon) = (47.141, 9.521);
        let m = position(at, lat, lon);
        assert!(m.waxing && m.illuminated < 0.15, "{m:?}");
        assert_eq!(m.phase_name(), "waxing crescent");
        assert!(m.azimuth_deg > 240.0 && m.azimuth_deg < 320.0, "{m:?}");
        // The bright limb points at the sun as projected on the sky: the
        // position angle from the celestial north through east.
        let g = m.geocentric.equatorial;
        let sun = sun::geocentric(julian_day(at)).equatorial;
        let lst = astro::lst_deg(at, lon);
        let enu = |e: Equatorial| astro::hour_angle_to_enu(lst - e.ra_deg, e.dec_deg, lat);
        let v = enu(g);
        let s = enu(sun);
        let plane = |x: [f64; 3]| {
            let d = x[0] * v[0] + x[1] * v[1] + x[2] * v[2];
            [x[0] - d * v[0], x[1] - d * v[1], x[2] - d * v[2]]
        };
        let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        // North: towards the pole; east: towards growing right ascension.
        let north = plane([0.0, lat.to_radians().cos(), lat.to_radians().sin()]);
        let ahead = enu(Equatorial {
            ra_deg: g.ra_deg + 0.01,
            dec_deg: g.dec_deg,
        });
        let east = plane([ahead[0] - v[0], ahead[1] - v[1], ahead[2] - v[2]]);
        let st = plane(s);
        let pa = (dot(st, east) / dot(east, east).sqrt())
            .atan2(dot(st, north) / dot(north, north).sqrt())
            .to_degrees()
            .rem_euclid(360.0);
        assert!(
            astro::wrap180(pa - m.bright_limb_deg).abs() < 1.0,
            "{pa} vs {m:?}"
        );
    }
}
