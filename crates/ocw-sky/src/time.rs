//! Instants and calendar arithmetic (UTC): Unix milliseconds, Julian days,
//! civil dates, and the supported range 1900–2100 (spec §14 Phase 9, 9D).

/// An instant as milliseconds since 1970-01-01T00:00:00Z.
pub type UnixMs = i64;

pub const MS_PER_DAY: i64 = 86_400_000;
pub const MS_PER_HOUR: i64 = 3_600_000;
pub const MS_PER_MINUTE: i64 = 60_000;

/// Days since 1970-01-01 of a proleptic Gregorian date (H. Hinnant's
/// algorithm).
pub const fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y } as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = m as i64;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The date (year, month, day) of a day number since 1970-01-01.
pub const fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    ((if m <= 2 { y + 1 } else { y }) as i32, m, d)
}

/// Earliest instant the time machine accepts: 1900-01-01T00:00:00Z.
pub const MIN_MS: UnixMs = days_from_civil(1900, 1, 1) * MS_PER_DAY;
/// Latest: the last millisecond of 2100 (UTC).
pub const MAX_MS: UnixMs = days_from_civil(2101, 1, 1) * MS_PER_DAY - 1;

/// `ms` limited to the supported range (years 1900–2100).
pub fn clamp_range(ms: UnixMs) -> UnixMs {
    ms.clamp(MIN_MS, MAX_MS)
}

/// Whether `ms` lies in the supported range.
pub fn in_range(ms: UnixMs) -> bool {
    (MIN_MS..=MAX_MS).contains(&ms)
}

/// Julian day (UT) of an instant.
pub fn julian_day(ms: UnixMs) -> f64 {
    ms as f64 / MS_PER_DAY as f64 + 2_440_587.5
}

/// The instant of a Julian day.
pub fn from_julian_day(jd: f64) -> UnixMs {
    ((jd - 2_440_587.5) * MS_PER_DAY as f64).round() as UnixMs
}

/// Julian centuries since J2000.0.
pub fn julian_century(jd: f64) -> f64 {
    (jd - 2_451_545.0) / 36_525.0
}

/// The UTC date of an instant.
pub fn utc_date(ms: UnixMs) -> (i32, u32, u32) {
    civil_from_days(ms.div_euclid(MS_PER_DAY))
}

/// Instant of 00:00 UTC on a date.
pub fn utc_midnight(y: i32, m: u32, d: u32) -> UnixMs {
    days_from_civil(y, m, d) * MS_PER_DAY
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates_round_trip() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        for z in [-80_000, -25_567, -1, 0, 1, 11_016, 20_000, 47_482, 49_000] {
            let (y, m, d) = civil_from_days(z);
            assert_eq!(days_from_civil(y, m, d), z, "{y}-{m}-{d}");
        }
        assert_eq!(civil_from_days(days_from_civil(2024, 2, 29)), (2024, 2, 29));
        assert_eq!(civil_from_days(days_from_civil(1900, 3, 1) - 1), (1900, 2, 28));
    }

    #[test]
    fn julian_days() {
        // J2000.0 is 2000-01-01 12:00 TT, ~12:00 UT.
        let j2000 = utc_midnight(2000, 1, 1) + 12 * MS_PER_HOUR;
        assert!((julian_day(j2000) - 2_451_545.0).abs() < 1e-9);
        assert_eq!(from_julian_day(2_451_545.0), j2000);
        assert!(julian_century(2_451_545.0).abs() < 1e-12);
    }

    #[test]
    fn the_range_is_1900_to_2100() {
        assert_eq!(utc_date(MIN_MS), (1900, 1, 1));
        assert_eq!(utc_date(MAX_MS), (2100, 12, 31));
        assert_eq!(utc_date(MAX_MS + 1), (2101, 1, 1));
        assert_eq!(clamp_range(MIN_MS - 1), MIN_MS);
        assert_eq!(clamp_range(i64::MAX), MAX_MS);
        assert!(in_range(0) && !in_range(MAX_MS + 1));
    }
}
