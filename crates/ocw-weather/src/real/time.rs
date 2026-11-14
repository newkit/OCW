//! Dates and times of the providers' data (UTC), without a time library:
//! the civil calendar ↔ days since 1970 (Howard Hinnant's algorithm), ISO
//! 8601 instants ("2026-11-14T11:00:00Z") and the stations' "14.11.2026
//! 11:30" / "14.11.26" + "11:00".

/// Milliseconds per day.
pub const DAY_MS: i64 = 86_400_000;
/// Milliseconds per hour.
pub const HOUR_MS: i64 = 3_600_000;

/// Days since 1970-01-01 of a civil date (proleptic Gregorian).
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(m);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The civil date (year, month, day) of a day since 1970-01-01.
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// The UTC day (days since 1970) of an instant (Unix ms).
pub fn day_of(ms: i64) -> i64 {
    ms.div_euclid(DAY_MS)
}

/// "1999-12-26" for a day since 1970.
pub fn date_label(day: i64) -> String {
    let (y, m, d) = civil_from_days(day);
    format!("{y:04}-{m:02}-{d:02}")
}

/// "2026-11-14 11:30 UTC".
pub fn utc_label(ms: i64) -> String {
    let day = day_of(ms);
    let min = (ms - day * DAY_MS) / 60_000;
    format!("{} {:02}:{:02} UTC", date_label(day), min / 60, min % 60)
}

/// "11:30" (UTC).
pub fn hh_mm_utc(ms: i64) -> String {
    let min = ms.rem_euclid(DAY_MS) / 60_000;
    format!("{:02}:{:02}", min / 60, min % 60)
}

fn num<T: std::str::FromStr>(s: &str) -> Option<T> {
    s.trim().parse().ok()
}

/// The instant of a UTC date and time.
pub fn utc_ms(y: i64, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> Option<i64> {
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || s > 60 {
        return None;
    }
    Some(
        days_from_civil(y, mo, d) * DAY_MS
            + i64::from(h) * HOUR_MS
            + i64::from(mi) * 60_000
            + i64::from(s) * 1000,
    )
}

/// "2026-11-14T11:00:00Z" (also without seconds, with "+00:00").
pub fn parse_iso(s: &str) -> Option<i64> {
    let s = s.trim();
    let (date, rest) = s.split_once('T')?;
    let mut dp = date.splitn(3, '-');
    let (y, mo, d) = (num(dp.next()?)?, num(dp.next()?)?, num(dp.next()?)?);
    let time = rest
        .trim_end_matches('Z')
        .trim_end_matches("+00:00")
        .split('.')
        .next()?;
    let mut tp = time.split(':');
    let h = num(tp.next()?)?;
    let mi = num(tp.next()?)?;
    let sec = tp.next().map_or(Some(0), num)?;
    utc_ms(y, mo, d, h, mi, sec)
}

/// "14.11.2026 11:30" (MeteoSwiss) or "14.11.26" + "11:00" (DWD), UTC.
pub fn parse_dotted(date: &str, time: &str) -> Option<i64> {
    let mut dp = date.trim().splitn(3, '.');
    let d = num(dp.next()?)?;
    let mo = num(dp.next()?)?;
    let ys = dp.next()?.trim();
    let y: i64 = num(ys)?;
    let y = if ys.len() == 2 { 2000 + y } else { y };
    let mut tp = time.trim().split(':');
    let h = num(tp.next()?)?;
    let mi = num(tp.next()?)?;
    utc_ms(y, mo, d, h, mi, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_days_round_trip_and_known_dates() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        assert_eq!(
            civil_from_days(days_from_civil(1999, 12, 26)),
            (1999, 12, 26)
        );
        for z in (-800_000..800_000).step_by(997) {
            let (y, m, d) = civil_from_days(z);
            assert_eq!(days_from_civil(y, m, d), z);
        }
        assert_eq!(date_label(days_from_civil(1940, 1, 1)), "1940-01-01");
    }

    #[test]
    fn instants_from_the_providers_formats() {
        // 2026-11-14T11:00:00Z = 1794654000 (the recorded MET Norway sample).
        let t = parse_iso("2026-11-14T11:00:00Z").unwrap();
        assert_eq!(t, 1_794_654_000_000);
        assert_eq!(parse_iso("2026-11-14T11:00Z"), Some(t));
        assert_eq!(parse_iso("2026-11-14T11:00:00+00:00"), Some(t));
        assert_eq!(parse_dotted("14.11.2026", "11:00"), Some(t));
        assert_eq!(parse_dotted("14.11.26", "11:00"), Some(t));
        assert_eq!(parse_iso("2026-13-14T11:00:00Z"), None);
        assert_eq!(parse_iso("garbage"), None);
        assert_eq!(utc_label(t + 30 * 60_000), "2026-11-14 11:30 UTC");
        assert_eq!(hh_mm_utc(t), "11:00");
        assert_eq!(day_of(t - 1), day_of(t));
        assert_eq!(day_of(-1), -1);
    }
}
