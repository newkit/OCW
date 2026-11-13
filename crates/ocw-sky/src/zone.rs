//! Local time in the virtual world (spec §14 Phase 9, 9A / 9B, decision
//! Q1): IANA time zones from the tzdb compiled into the binary (crate
//! `jiff`), local time and DST at an instant, local date and time back to
//! an instant (with the skipped and the repeated hour of DST changes), and
//! the default zone of a place for world packages without one.

use jiff::civil::DateTime;
use jiff::tz::{AmbiguousOffset, TimeZone};

use crate::time::{MS_PER_DAY, MS_PER_HOUR, UnixMs, civil_from_days, days_from_civil};

/// The zone of rides without a geographic origin (the built-in test
/// routes): Liechtenstein, like [`DEFAULT_PLACE`].
pub const DEFAULT_ZONE: &str = "Europe/Vaduz";
/// Where rides without a geographic origin are (lat °, lon °): Vaduz.
pub const DEFAULT_PLACE: (f64, f64) = (47.141, 9.521);

/// An IANA time zone.
#[derive(Clone, Debug)]
pub struct Zone {
    id: String,
    tz: TimeZone,
}

impl PartialEq for Zone {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

/// A local date and time (wall clock) in a zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalTime {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    /// Offset from UTC in seconds (east positive), DST included.
    pub offset_s: i32,
    pub dst: bool,
}

impl LocalTime {
    /// "18:42".
    pub fn hh_mm(&self) -> String {
        format!("{:02}:{:02}", self.hour, self.minute)
    }

    /// "2026-06-21".
    pub fn date(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    /// Seconds since local midnight.
    pub fn seconds_of_day(&self) -> u32 {
        self.hour * 3600 + self.minute * 60 + self.second
    }

    /// Day of the week, 0 = Monday.
    pub fn weekday(&self) -> u32 {
        // 1970-01-01 was a Thursday (3).
        (days_from_civil(self.year, self.month, self.day) + 3).rem_euclid(7) as u32
    }
}

/// How a local wall-clock time maps to instants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalKind {
    /// Exactly one instant.
    Normal,
    /// Skipped (the clocks went forward over it, spring): the instant is
    /// taken with the offset before the change, i.e. it lands after the gap.
    Gap,
    /// Repeated (the clocks went back, autumn): the earlier instant.
    Fold,
}

impl Zone {
    /// The zone `id` ("Europe/Vaduz", "Etc/GMT-1", "UTC").
    pub fn get(id: &str) -> Result<Self, String> {
        let tz = TimeZone::get(id).map_err(|e| format!("unknown time zone {id:?}: {e}"))?;
        Ok(Self {
            id: id.to_owned(),
            tz,
        })
    }

    /// UTC.
    pub fn utc() -> Self {
        Self {
            id: "UTC".into(),
            tz: TimeZone::UTC,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    fn timestamp(ms: UnixMs) -> jiff::Timestamp {
        jiff::Timestamp::from_millisecond(ms).unwrap_or(jiff::Timestamp::UNIX_EPOCH)
    }

    /// Offset from UTC at `ms` in seconds (east positive).
    pub fn offset_s(&self, ms: UnixMs) -> i32 {
        self.tz.to_offset(Self::timestamp(ms)).seconds()
    }

    /// Daylight saving time at `ms`.
    pub fn is_dst(&self, ms: UnixMs) -> bool {
        self.tz.to_offset_info(Self::timestamp(ms)).dst().is_dst()
    }

    /// The zone's abbreviation at `ms` ("CET", "CEST").
    pub fn abbreviation(&self, ms: UnixMs) -> String {
        self.tz
            .to_offset_info(Self::timestamp(ms))
            .abbreviation()
            .to_owned()
    }

    /// The local wall-clock time at `ms`.
    pub fn local(&self, ms: UnixMs) -> LocalTime {
        let offset_s = self.offset_s(ms);
        let local_ms = ms + i64::from(offset_s) * 1000;
        let days = local_ms.div_euclid(MS_PER_DAY);
        let (year, month, day) = civil_from_days(days);
        let in_day = local_ms.rem_euclid(MS_PER_DAY) / 1000;
        LocalTime {
            year,
            month,
            day,
            hour: (in_day / 3600) as u32,
            minute: (in_day / 60 % 60) as u32,
            second: (in_day % 60) as u32,
            offset_s,
            dst: self.is_dst(ms),
        }
    }

    /// The instant of a local date and time, and how it maps (DST gaps and
    /// folds, see [`LocalKind`]).
    pub fn instant(
        &self,
        (y, mo, d): (i32, u32, u32),
        (h, mi, s): (u32, u32, u32),
    ) -> Result<(UnixMs, LocalKind), String> {
        let bad = || format!("no such local time {y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}");
        let dt = DateTime::new(
            i16::try_from(y).map_err(|_| bad())?,
            i8::try_from(mo).map_err(|_| bad())?,
            i8::try_from(d).map_err(|_| bad())?,
            i8::try_from(h).map_err(|_| bad())?,
            i8::try_from(mi).map_err(|_| bad())?,
            i8::try_from(s).map_err(|_| bad())?,
            0,
        )
        .map_err(|_| bad())?;
        let local_ms =
            (days_from_civil(y, mo, d) * 86_400 + i64::from(h * 3600 + mi * 60 + s)) * 1000;
        let at = |offset: jiff::tz::Offset| local_ms - i64::from(offset.seconds()) * 1000;
        Ok(match self.tz.to_ambiguous_timestamp(dt).offset() {
            AmbiguousOffset::Unambiguous { offset } => (at(offset), LocalKind::Normal),
            AmbiguousOffset::Gap { before, .. } => (at(before), LocalKind::Gap),
            AmbiguousOffset::Fold { before, .. } => (at(before), LocalKind::Fold),
        })
    }

    /// The instant of local `hh:mm:ss` on the local date of `ms`.
    pub fn at_time_of_day(
        &self,
        ms: UnixMs,
        (h, mi, s): (u32, u32, u32),
    ) -> Result<UnixMs, String> {
        let l = self.local(ms);
        self.instant((l.year, l.month, l.day), (h, mi, s))
            .map(|r| r.0)
    }

    /// The instant of local midnight starting the day after `ms`'s local date.
    pub fn next_midnight(&self, ms: UnixMs) -> UnixMs {
        let l = self.local(ms);
        let (y, m, d) = civil_from_days(days_from_civil(l.year, l.month, l.day) + 1);
        self.instant((y, m, d), (0, 0, 0))
            .map_or(ms + 12 * MS_PER_HOUR, |r| r.0)
    }
}

/// Regions with their zone, first match wins (rough boxes of lat / lon;
/// the world builder takes the zone of the package origin, `world-builder
/// package --time-zone` overrides it).
const REGIONS: &[(&str, [f64; 4], &str)] = &[
    // [lat min, lat max, lon min, lon max]
    (
        "Liechtenstein",
        [47.048, 47.271, 9.471, 9.636],
        "Europe/Vaduz",
    ),
    // Southern Baden-Württemberg north of the Swiss border (Black Forest).
    ("Black Forest", [47.75, 49.8, 7.5, 10.2], "Europe/Berlin"),
    ("Switzerland", [45.82, 47.81, 5.95, 10.49], "Europe/Zurich"),
    ("Austria", [46.37, 49.02, 9.53, 17.16], "Europe/Vienna"),
    ("Germany", [47.27, 55.06, 5.87, 15.04], "Europe/Berlin"),
    ("France", [41.3, 51.1, -5.2, 9.6], "Europe/Paris"),
    ("Italy", [36.6, 47.1, 6.6, 18.5], "Europe/Rome"),
];

/// The time zone of a place (lat °, lon °): a region's IANA zone
/// ([`REGIONS`]), else the nautical zone of the longitude
/// (`Etc/GMT-1` for 7.5–22.5° E; no DST).
pub fn zone_for(lat: f64, lon: f64) -> String {
    for (_, [la0, la1, lo0, lo1], zone) in REGIONS {
        if (*la0..=*la1).contains(&lat) && (*lo0..=*lo1).contains(&lon) {
            return (*zone).to_owned();
        }
    }
    longitude_zone(lon)
}

/// The nautical time zone of a longitude: `Etc/GMT-N` is UTC+N (POSIX
/// signs), `Etc/UTC` around Greenwich.
pub fn longitude_zone(lon: f64) -> String {
    let h = (lon / 15.0).round().clamp(-12.0, 12.0) as i32;
    match h {
        0 => "Etc/UTC".into(),
        h if h > 0 => format!("Etc/GMT-{h}"),
        h => format!("Etc/GMT+{}", -h),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::utc_midnight;

    fn utc(y: i32, m: u32, d: u32, h: i64, mi: i64) -> UnixMs {
        utc_midnight(y, m, d) + h * MS_PER_HOUR + mi * 60_000
    }

    /// DST in Liechtenstein and Germany 2026: from 29 March 02:00 CET to
    /// 03:00 CEST (01:00 UTC), back on 25 October 03:00 CEST to 02:00 CET.
    #[test]
    fn dst_transitions_in_vaduz_and_berlin() {
        for id in ["Europe/Vaduz", "Europe/Berlin"] {
            let z = Zone::get(id).unwrap();
            // Just before and after the spring change.
            let before = z.local(utc(2026, 3, 29, 0, 59));
            assert_eq!(
                (before.hour, before.minute, before.offset_s, before.dst),
                (1, 59, 3600, false),
                "{id}"
            );
            let after = z.local(utc(2026, 3, 29, 1, 0));
            assert_eq!(
                (after.hour, after.minute, after.offset_s, after.dst),
                (3, 0, 7200, true),
                "{id}"
            );
            assert_eq!(z.abbreviation(utc(2026, 3, 29, 1, 0)), "CEST");
            assert_eq!(z.abbreviation(utc(2026, 3, 28, 12, 0)), "CET");
            // The skipped hour: 02:30 does not exist; it lands after the gap
            // (taken with the old offset: 01:30 UTC = 03:30 CEST).
            let (at, kind) = z.instant((2026, 3, 29), (2, 30, 0)).unwrap();
            assert_eq!(kind, LocalKind::Gap);
            assert_eq!(at, utc(2026, 3, 29, 1, 30));
            assert_eq!(z.local(at).hh_mm(), "03:30");
            // Autumn: 02:00-03:00 happens twice; 02:30 is the earlier one
            // (CEST, 00:30 UTC), the second one is an hour later.
            let (at, kind) = z.instant((2026, 10, 25), (2, 30, 0)).unwrap();
            assert_eq!(kind, LocalKind::Fold);
            assert_eq!(at, utc(2026, 10, 25, 0, 30));
            assert_eq!(z.local(at).hh_mm(), "02:30");
            assert_eq!(z.local(at + MS_PER_HOUR).hh_mm(), "02:30");
            assert!(z.local(at).dst && !z.local(at + MS_PER_HOUR).dst);
            // Ordinary times map one to one.
            let (at, kind) = z.instant((2026, 6, 21), (21, 15, 0)).unwrap();
            assert_eq!((at, kind), (utc(2026, 6, 21, 19, 15), LocalKind::Normal));
            let (at, _) = z.instant((2026, 12, 21), (12, 0, 0)).unwrap();
            assert_eq!(at, utc(2026, 12, 21, 11, 0));
        }
    }

    #[test]
    fn local_time_fields() {
        let z = Zone::get("Europe/Vaduz").unwrap();
        let l = z.local(utc(2026, 11, 13, 23, 30));
        assert_eq!((l.year, l.month, l.day), (2026, 11, 14));
        assert_eq!(l.hh_mm(), "00:30");
        assert_eq!(l.date(), "2026-11-14");
        assert_eq!(l.weekday(), 5, "a Saturday");
        assert_eq!(l.seconds_of_day(), 1800);
        // Historic dates work (1900: no DST, LMT is before 1894 in Zurich).
        let old = z.local(utc(1900, 7, 1, 12, 0));
        assert_eq!(old.offset_s, 3600);
        assert_eq!(
            z.next_midnight(utc(2026, 6, 21, 10, 0)),
            utc(2026, 6, 21, 22, 0)
        );
        assert!(Zone::get("Mars/Olympus_Mons").is_err());
        assert_eq!(Zone::utc().offset_s(0), 0);
    }

    #[test]
    fn default_zones_by_region_and_longitude() {
        assert_eq!(zone_for(47.14, 9.52), "Europe/Vaduz");
        assert_eq!(
            zone_for(47.92, 7.96),
            "Europe/Berlin",
            "Black Forest origin"
        );
        assert_eq!(zone_for(47.37, 8.54), "Europe/Zurich");
        assert_eq!(zone_for(48.21, 16.37), "Europe/Vienna");
        assert_eq!(zone_for(52.52, 13.40), "Europe/Berlin");
        assert_eq!(zone_for(35.68, 139.69), "Etc/GMT-9", "Tokyo: by longitude");
        assert_eq!(zone_for(40.0, -105.0), "Etc/GMT+7");
        assert_eq!(longitude_zone(3.0), "Etc/UTC");
        // Every default is a zone the bundled tzdb knows.
        for lon in [-180.0, -97.0, -7.4, 0.0, 7.6, 179.9] {
            let id = longitude_zone(lon);
            let z = Zone::get(&id).unwrap();
            assert_eq!(
                z.offset_s(0),
                (lon / 15.0_f64).round() as i32 * 3600,
                "{id}"
            );
        }
        for (_, _, id) in REGIONS {
            Zone::get(id).unwrap();
        }
        Zone::get(DEFAULT_ZONE).unwrap();
    }

    /// Q1 of Phase 9 (spec 9A): the time of the place in the world counts.
    /// 14:00 in Tokyo is 06:00 in Liechtenstein in winter, 07:00 in summer.
    #[test]
    fn tokyo_rider_in_liechtenstein_sees_vaduz_time() {
        let tokyo = Zone::get("Asia/Tokyo").unwrap();
        let vaduz = Zone::get("Europe/Vaduz").unwrap();
        let (winter, _) = tokyo.instant((2026, 12, 21), (14, 0, 0)).unwrap();
        assert_eq!(vaduz.local(winter).hh_mm(), "06:00");
        let (summer, _) = tokyo.instant((2026, 6, 21), (14, 0, 0)).unwrap();
        assert_eq!(vaduz.local(summer).hh_mm(), "07:00");
    }
}
