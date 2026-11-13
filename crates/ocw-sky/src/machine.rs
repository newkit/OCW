//! The time machine's requests (spec §14 Phase 9, 9D): parsing the
//! console command `time` and resolving presets and local times at the
//! rider's place into a [`ClockChange`] for the server, which owns the
//! clock (§6.1) and checks who may change it.

use crate::clock::{MAX_SPEED, SPEEDS};
use crate::sun;
use crate::time::{MS_PER_DAY, MS_PER_HOUR, MS_PER_MINUTE, UnixMs};
use crate::zone::Zone;

/// A change of the session clock, as sent to the server.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClockChange {
    Real,
    Fixed(UnixMs),
    /// Real time plus this offset.
    Offset(i64),
    /// Move the world instant, keeping the mode.
    Step(i64),
    Speed(f64),
    Paused(bool),
}

/// The presets (sun positions at the rider's place on the current world
/// date; the solstices keep the time of day).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    Sunrise,
    /// Evening golden hour: the sun 5° high.
    GoldenHour,
    /// Solar noon.
    Noon,
    Sunset,
    /// Evening blue hour: the sun 5° below the horizon.
    BlueHour,
    /// The coming local midnight.
    Midnight,
    /// 21 June (21 December south of the equator) at the current time of day.
    SummerSolstice,
    WinterSolstice,
}

impl Preset {
    pub const ALL: [Preset; 8] = [
        Self::Sunrise,
        Self::GoldenHour,
        Self::Noon,
        Self::Sunset,
        Self::BlueHour,
        Self::Midnight,
        Self::SummerSolstice,
        Self::WinterSolstice,
    ];

    /// The console name ("golden-hour").
    pub fn id(self) -> &'static str {
        match self {
            Self::Sunrise => "sunrise",
            Self::GoldenHour => "golden-hour",
            Self::Noon => "noon",
            Self::Sunset => "sunset",
            Self::BlueHour => "blue-hour",
            Self::Midnight => "midnight",
            Self::SummerSolstice => "summer-solstice",
            Self::WinterSolstice => "winter-solstice",
        }
    }

    /// The menu label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Sunrise => "Sunrise",
            Self::GoldenHour => "Golden hour",
            Self::Noon => "Noon",
            Self::Sunset => "Sunset",
            Self::BlueHour => "Blue hour",
            Self::Midnight => "Midnight",
            Self::SummerSolstice => "Summer solstice",
            Self::WinterSolstice => "Winter solstice",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let s = s.to_ascii_lowercase().replace('_', "-");
        Self::ALL.into_iter().find(|p| p.id() == s)
    }

    /// The instant of this preset for the world instant `now` at (lat, lon)
    /// in `zone`.
    pub fn instant(self, now: UnixMs, zone: &Zone, lat: f64, lon: f64) -> Result<UnixMs, String> {
        let l = zone.local(now);
        let date = (l.year, l.month, l.day);
        let none = || format!("no {} on {} here (polar day or night)", self.label().to_lowercase(), l.date());
        // Elevation h: zenith 90 - h.
        let evening = |h: f64| sun::crossing(date, lat, lon, 90.0 - h, false).ok_or_else(none);
        match self {
            Self::Sunrise => sun::crossing(date, lat, lon, sun::zenith::SUNRISE, true).ok_or_else(none),
            Self::GoldenHour => evening(5.0),
            Self::Noon => Ok(sun::solar_noon(date, lon)),
            Self::Sunset => sun::crossing(date, lat, lon, sun::zenith::SUNRISE, false).ok_or_else(none),
            Self::BlueHour => evening(-5.0),
            Self::Midnight => Ok(zone.next_midnight(now)),
            Self::SummerSolstice | Self::WinterSolstice => {
                let june = (self == Self::SummerSolstice) == (lat >= 0.0);
                let (m, d) = if june { (6, 21) } else { (12, 21) };
                zone.instant((l.year, m, d), (l.hour, l.minute, l.second))
                    .map(|r| r.0)
            }
        }
    }
}

/// A parsed `time` command.
#[derive(Debug, Clone, PartialEq)]
pub enum TimeCommand {
    /// Show the clock.
    Show,
    Real,
    /// Local wall-clock time on the current world date.
    TimeOfDay(u32, u32),
    /// A local date, keeping the time of day.
    Date(i32, u32, u32),
    /// A local date and time.
    DateTime((i32, u32, u32), (u32, u32)),
    /// Real time plus an offset (ms).
    Offset(i64),
    /// Move the world time by this much (ms).
    Step(i64),
    Speed(f64),
    Paused(bool),
    Preset(Preset),
}

/// Help text of the console command.
pub const USAGE: &str = "time [real | HH:MM | YYYY-MM-DD [HH:MM] | +2h | -30m | offset +3h | speed 1|10|60|600 | pause | play | PRESET]; presets: sunrise, golden-hour, noon, sunset, blue-hour, midnight, summer-solstice, winter-solstice";

fn parse_hm(s: &str) -> Option<(u32, u32)> {
    let (h, m) = s.split_once(':')?;
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some((h, m))
}

fn parse_date(s: &str) -> Option<(i32, u32, u32)> {
    let mut it = s.splitn(3, '-');
    let y = it.next()?.parse().ok()?;
    let m = it.next()?.parse().ok()?;
    let d = it.next()?.parse().ok()?;
    ((1..=12).contains(&m) && (1..=31).contains(&d)).then_some((y, m, d))
}

/// "+2h", "-30m", "+1d", "-90s", "+1.5h" → milliseconds.
pub fn parse_duration(s: &str) -> Option<i64> {
    let (sign, rest) = match s.as_bytes().first()? {
        b'+' => (1.0, &s[1..]),
        b'-' => (-1.0, &s[1..]),
        _ => (1.0, s),
    };
    let unit = rest.chars().last()?;
    let n: f64 = rest[..rest.len() - unit.len_utf8()].parse().ok()?;
    let ms = match unit {
        'd' => MS_PER_DAY as f64,
        'h' => MS_PER_HOUR as f64,
        'm' => MS_PER_MINUTE as f64,
        's' => 1000.0,
        _ => return None,
    };
    (n.is_finite()).then(|| (sign * n * ms).round() as i64)
}

/// Parses the arguments of `time`.
pub fn parse(args: &[&str]) -> Result<TimeCommand, String> {
    let bad = || format!("usage: {USAGE}");
    let lower: Vec<String> = args.iter().map(|a| a.to_ascii_lowercase()).collect();
    let a: Vec<&str> = lower.iter().map(String::as_str).collect();
    Ok(match a.as_slice() {
        [] | ["show"] | ["status"] => TimeCommand::Show,
        ["real"] | ["now"] => TimeCommand::Real,
        ["pause"] => TimeCommand::Paused(true),
        ["play"] | ["resume"] => TimeCommand::Paused(false),
        ["speed", "pause"] | ["speed", "paused"] | ["speed", "0"] => TimeCommand::Paused(true),
        ["speed", s] => {
            let s = s.trim_start_matches(['x', '×']);
            let v: f64 = s.parse().map_err(|_| bad())?;
            if !(v > 0.0 && v <= MAX_SPEED) {
                return Err(format!(
                    "time speed above 0 and at most x{MAX_SPEED:.0} (offered: {})",
                    SPEEDS.map(|s| format!("x{s}")).join(", ")
                ));
            }
            TimeCommand::Speed(v)
        }
        ["offset", d] => TimeCommand::Offset(parse_duration(d).ok_or_else(bad)?),
        ["preset", p] => TimeCommand::Preset(Preset::parse(p).ok_or_else(bad)?),
        [one] => {
            if let Some(p) = Preset::parse(one) {
                TimeCommand::Preset(p)
            } else if let Some((h, m)) = parse_hm(one) {
                TimeCommand::TimeOfDay(h, m)
            } else if let Some((date, time)) = one.split_once('t') {
                let d = parse_date(date).ok_or_else(bad)?;
                let t = parse_hm(time).ok_or_else(bad)?;
                TimeCommand::DateTime(d, t)
            } else if let Some((y, m, d)) = parse_date(one) {
                TimeCommand::Date(y, m, d)
            } else if one.starts_with(['+', '-']) {
                TimeCommand::Step(parse_duration(one).ok_or_else(bad)?)
            } else {
                return Err(bad());
            }
        }
        [date, time] => {
            let d = parse_date(date).ok_or_else(bad)?;
            let t = parse_hm(time).ok_or_else(bad)?;
            TimeCommand::DateTime(d, t)
        }
        _ => return Err(bad()),
    })
}

/// The clock change of `cmd` for the world instant `now` at (lat, lon) in
/// `zone`; `None` for [`TimeCommand::Show`].
pub fn resolve(
    cmd: &TimeCommand,
    now: UnixMs,
    zone: &Zone,
    lat: f64,
    lon: f64,
) -> Result<Option<ClockChange>, String> {
    let fixed = |r: Result<(UnixMs, crate::zone::LocalKind), String>| r.map(|x| ClockChange::Fixed(x.0));
    Ok(Some(match *cmd {
        TimeCommand::Show => return Ok(None),
        TimeCommand::Real => ClockChange::Real,
        TimeCommand::TimeOfDay(h, m) => {
            let l = zone.local(now);
            fixed(zone.instant((l.year, l.month, l.day), (h, m, 0)))?
        }
        TimeCommand::Date(y, m, d) => {
            let l = zone.local(now);
            fixed(zone.instant((y, m, d), (l.hour, l.minute, l.second)))?
        }
        TimeCommand::DateTime(date, (h, m)) => fixed(zone.instant(date, (h, m, 0)))?,
        TimeCommand::Offset(ms) => ClockChange::Offset(ms),
        TimeCommand::Step(ms) => ClockChange::Step(ms),
        TimeCommand::Speed(s) => ClockChange::Speed(s),
        TimeCommand::Paused(p) => ClockChange::Paused(p),
        TimeCommand::Preset(p) => ClockChange::Fixed(p.instant(now, zone, lat, lon)?),
    }))
}

/// The next / previous speed of the time machine's menu row (paused
/// below x1).
pub fn speed_step(speed: f64, paused: bool, dir: i32) -> ClockChange {
    let at = if paused {
        -1
    } else {
        SPEEDS.iter().position(|&s| s >= speed).unwrap_or(SPEEDS.len() - 1) as i32
    };
    let to = (at + dir).clamp(-1, SPEEDS.len() as i32 - 1);
    if to < 0 {
        ClockChange::Paused(true)
    } else if paused && to == 0 {
        ClockChange::Paused(false)
    } else {
        ClockChange::Speed(SPEEDS[to as usize])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::utc_midnight;

    const VADUZ: (f64, f64) = (47.141, 9.521);

    fn zone() -> Zone {
        Zone::get("Europe/Vaduz").unwrap()
    }

    #[test]
    fn parses_the_console_forms() {
        use TimeCommand::*;
        assert_eq!(parse(&[]), Ok(Show));
        assert_eq!(parse(&["real"]), Ok(Real));
        assert_eq!(parse(&["18:30"]), Ok(TimeOfDay(18, 30)));
        assert_eq!(parse(&["2026-06-21"]), Ok(Date(2026, 6, 21)));
        assert_eq!(parse(&["2026-06-21", "21:15"]), Ok(DateTime((2026, 6, 21), (21, 15))));
        assert_eq!(parse(&["2026-06-21T21:15"]), Ok(DateTime((2026, 6, 21), (21, 15))));
        assert_eq!(parse(&["+2h"]), Ok(Step(2 * MS_PER_HOUR)));
        assert_eq!(parse(&["-30m"]), Ok(Step(-30 * MS_PER_MINUTE)));
        assert_eq!(parse(&["offset", "+1.5h"]), Ok(Offset(90 * MS_PER_MINUTE)));
        assert_eq!(parse(&["speed", "x600"]), Ok(Speed(600.0)));
        assert_eq!(parse(&["speed", "60"]), Ok(Speed(60.0)));
        assert_eq!(parse(&["pause"]), Ok(Paused(true)));
        assert_eq!(parse(&["speed", "paused"]), Ok(Paused(true)));
        assert_eq!(parse(&["play"]), Ok(Paused(false)));
        assert_eq!(parse(&["Sunset"]), Ok(Preset(super::Preset::Sunset)));
        assert_eq!(parse(&["preset", "golden_hour"]), Ok(Preset(super::Preset::GoldenHour)));
        for bad in [&["25:00"][..], &["speed", "9000"], &["tomorrow"], &["2026-13-01"], &["a", "b", "c"]] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn local_times_resolve_in_the_world_zone() {
        let z = zone();
        let now = utc_midnight(2026, 6, 21) + 8 * MS_PER_HOUR; // 10:00 CEST
        let r = |c| resolve(&c, now, &z, VADUZ.0, VADUZ.1).unwrap();
        assert_eq!(
            r(TimeCommand::TimeOfDay(18, 30)),
            Some(ClockChange::Fixed(utc_midnight(2026, 6, 21) + 16 * MS_PER_HOUR + 30 * MS_PER_MINUTE))
        );
        // A winter date keeps the wall-clock time (CET instead of CEST).
        assert_eq!(
            r(TimeCommand::Date(2026, 12, 24)),
            Some(ClockChange::Fixed(utc_midnight(2026, 12, 24) + 9 * MS_PER_HOUR))
        );
        assert_eq!(r(TimeCommand::Show), None);
        assert_eq!(r(TimeCommand::Step(5)), Some(ClockChange::Step(5)));
    }

    #[test]
    fn presets_at_the_riders_place() {
        let z = zone();
        let (lat, lon) = VADUZ;
        let now = utc_midnight(2026, 6, 21) + 8 * MS_PER_HOUR;
        let at = |p: Preset| p.instant(now, &z, lat, lon).unwrap();
        let elevation = |ms| sun::position(ms, lat, lon).elevation_deg;
        let day = sun::day((2026, 6, 21), lat, lon);
        assert_eq!(at(Preset::Sunrise), day.sun.morning.unwrap());
        assert_eq!(at(Preset::Sunset), day.sun.evening.unwrap());
        assert_eq!(at(Preset::Noon), day.solar_noon);
        assert!((sun::position(at(Preset::GoldenHour), lat, lon).true_elevation_deg - 5.0).abs() < 0.05);
        assert!((sun::position(at(Preset::BlueHour), lat, lon).true_elevation_deg + 5.0).abs() < 0.05);
        // Order through the evening.
        assert!(at(Preset::Noon) < at(Preset::GoldenHour));
        assert!(at(Preset::GoldenHour) < at(Preset::Sunset));
        assert!(at(Preset::Sunset) < at(Preset::BlueHour));
        assert!(at(Preset::BlueHour) < at(Preset::Midnight));
        assert_eq!(z.local(at(Preset::Midnight)).hh_mm(), "00:00");
        assert_eq!(z.local(at(Preset::Midnight)).day, 22);
        assert!(elevation(at(Preset::Midnight)) < -15.0);
        // Sunset in Vaduz on 21 June: 21:21 CEST.
        assert_eq!(z.local(at(Preset::Sunset)).hh_mm(), "21:21");
        // Solstices keep the time of day (10:00), in CEST / CET.
        let w = at(Preset::WinterSolstice);
        assert_eq!((z.local(w).month, z.local(w).day, z.local(w).hh_mm()), (12, 21, "10:00".into()));
        let s = Preset::SummerSolstice.instant(w, &z, lat, lon).unwrap();
        assert_eq!((z.local(s).month, z.local(s).hh_mm()), (6, "10:00".into()));
        // South of the equator summer is in December.
        let south = Preset::SummerSolstice.instant(now, &z, -33.9, 18.4).unwrap();
        assert_eq!(z.local(south).month, 12);
        // Polar night: no sunrise.
        let arctic = Preset::Sunrise.instant(utc_midnight(2026, 12, 21), &Zone::utc(), 78.2, 15.6);
        assert!(arctic.is_err());
        for p in Preset::ALL {
            assert_eq!(Preset::parse(p.id()), Some(p));
        }
    }

    #[test]
    fn speed_steps_through_pause_and_the_speeds() {
        assert_eq!(speed_step(1.0, false, 1), ClockChange::Speed(10.0));
        assert_eq!(speed_step(600.0, false, 1), ClockChange::Speed(600.0));
        assert_eq!(speed_step(10.0, false, -1), ClockChange::Speed(1.0));
        assert_eq!(speed_step(1.0, false, -1), ClockChange::Paused(true));
        assert_eq!(speed_step(60.0, true, 1), ClockChange::Paused(false));
        assert_eq!(speed_step(1.0, true, -1), ClockChange::Paused(true));
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("+1d"), Some(MS_PER_DAY));
        assert_eq!(parse_duration("90s"), Some(90_000));
        assert_eq!(parse_duration("-0.5h"), Some(-30 * MS_PER_MINUTE));
        assert_eq!(parse_duration("5x"), None);
        assert_eq!(parse_duration(""), None);
    }
}
