//! "Ride a day" (spec §14 Phase 9, S7, Q8): memorable days of central
//! European weather for the weather machine. Choosing one sets the time
//! machine to that local time (`time DATE TIME`) and the weather to real
//! (`weather real`): the server then takes the weather of that hour at the
//! place from the historical data (ERA5 via Open-Meteo).

/// One day: id (console), title (menu), local date and time of the place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Day {
    pub id: &'static str,
    pub title: &'static str,
    /// "1999-12-26".
    pub date: &'static str,
    /// "12:00".
    pub time: &'static str,
    pub about: &'static str,
}

/// The days of the menu; "today" first (the real clock and the real
/// weather now).
pub const DAYS: &[Day] = &[
    Day {
        id: "today",
        title: "Today",
        date: "",
        time: "",
        about: "the real time and the weather now",
    },
    Day {
        id: "lothar",
        title: "Storm Lothar",
        date: "1999-12-26",
        time: "12:00",
        about: "hurricane gusts felled whole forests",
    },
    Day {
        id: "kyrill",
        title: "Storm Kyrill",
        date: "2007-01-18",
        time: "19:00",
        about: "a winter gale with rain over Germany",
    },
    Day {
        id: "vivian",
        title: "Storm Vivian",
        date: "1990-02-27",
        time: "06:00",
        about: "a storm over the Alps",
    },
    Day {
        id: "heat-2003",
        title: "Heatwave",
        date: "2003-08-08",
        time: "15:00",
        about: "a hot afternoon of a record summer",
    },
    Day {
        id: "snow-2019",
        title: "Alpine snow",
        date: "2019-01-14",
        time: "11:00",
        about: "heavy snow north of the Alps",
    },
];

/// The day `id`.
pub fn day(id: &str) -> Option<&'static Day> {
    DAYS.iter().find(|d| d.id == id)
}

/// The console lines that ride a day: the time machine, then real
/// weather ("today": the real clock).
pub fn lines(d: &Day) -> Vec<String> {
    if d.date.is_empty() {
        vec!["time real".into(), "weather real".into()]
    } else {
        vec![format!("time {} {}", d.date, d.time), "weather real".into()]
    }
}

/// Checks a "YYYY-MM-DD" date and "HH:MM" time (1900–2100, like the time
/// machine), normalized.
pub fn check(date: &str, time: &str) -> Result<(String, String), String> {
    let bad = || format!("not a date and time: {date} {time} (YYYY-MM-DD HH:MM)");
    let mut p = date.split('-');
    let (y, m, d): (i64, u32, u32) = (
        p.next().and_then(|v| v.parse().ok()).ok_or_else(bad)?,
        p.next().and_then(|v| v.parse().ok()).ok_or_else(bad)?,
        p.next().and_then(|v| v.parse().ok()).ok_or_else(bad)?,
    );
    let (h, mi): (u32, u32) = match time.split_once(':') {
        Some((h, mi)) => (
            h.parse().map_err(|_| bad())?,
            mi.parse().map_err(|_| bad())?,
        ),
        None => return Err(bad()),
    };
    if !(1900..=2100).contains(&y) {
        return Err("the time machine goes from 1900 to 2100".into());
    }
    super::time::utc_ms(y, m, d, h, mi, 0).ok_or_else(bad)?;
    let days = super::time::days_from_civil(y, m, d);
    if super::time::civil_from_days(days) != (y, m, d) {
        return Err(bad());
    }
    Ok((format!("{y:04}-{m:02}-{d:02}"), format!("{h:02}:{mi:02}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_are_valid_and_ride_with_the_time_machine() {
        for d in DAYS {
            assert_eq!(day(d.id), Some(d));
            if !d.date.is_empty() {
                assert_eq!(check(d.date, d.time).unwrap().0, d.date);
            }
            let l = lines(d);
            assert_eq!(l.last().map(String::as_str), Some("weather real"));
        }
        assert_eq!(
            lines(day("lothar").unwrap()),
            ["time 1999-12-26 12:00", "weather real"]
        );
        assert_eq!(lines(day("today").unwrap())[0], "time real");
        assert_eq!(
            check("1999-12-6", "9:05").unwrap(),
            ("1999-12-06".into(), "09:05".into())
        );
        assert!(check("1999-02-30", "12:00").is_err());
        assert!(check("1850-01-01", "12:00").unwrap_err().contains("1900"));
        assert!(check("1999-12-26", "25:00").is_err());
        assert!(check("yesterday", "12:00").is_err());
    }
}
