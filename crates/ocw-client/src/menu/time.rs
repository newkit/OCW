//! The time machine's menu (spec §14 Phase 9, S1 / S3; §3.2.1 World →
//! Time), Bevy-free and shared by the desktop and browser apps: what the
//! page knows of the world clock ([`TimeMachineInfo`], refreshed every
//! frame by [`refresh`]), today's sun at the rider ([`SunToday`]), the
//! date and time texts and the page's rows (each a `time …` console line;
//! the server decides who may change the clock).

use ocw_render::RideView;
use ocw_render::sky::SkyView;
use ocw_sky::machine::{self, ClockChange, Preset};
use ocw_sky::{ClockMode, Sky, Zone};

use super::pages::{Activate, Row};
use crate::client::ClientCore;
use crate::sky::hud_label;

/// What the Time machine page shows (updated every frame).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TimeMachineInfo {
    /// "21:15 CEST" and "Sun 21 Jun 2026" (two rows: one line each at 16:9).
    pub time: Option<String>,
    pub date: Option<String>,
    pub mode: Option<ClockMode>,
    /// "offset +2.0 h".
    pub mode_label: String,
    pub speed: f64,
    pub paused: bool,
    /// Who changes the time: "you", "the host" or "the organizer".
    pub control: String,
    pub may_change: bool,
    /// The last preset chosen (the page's ← / → step from it).
    pub preset: Option<Preset>,
    /// Today's sun at the rider (World → About → Sun and time).
    pub sun: Option<SunToday>,
}

/// The sun's day at the rider's place on the world's local date (spec
/// §3.2.1 World → About this world), as text in the place's zone.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SunToday {
    /// "Europe/Vaduz, UTC+01:00".
    pub zone: String,
    /// "Fri 13 Nov 2026".
    pub date: String,
    /// Sunrise, sunset ("-" where the sun does not rise or set).
    pub sun: (String, String),
    pub noon: String,
    pub civil: (String, String),
    pub nautical: (String, String),
    pub astronomical: (String, String),
    /// Where (lat, lon), rounded for the cache.
    key: (i64, i64, String),
}

/// "UTC+01:00" / "UTC-03:30".
pub fn utc_offset(offset_s: i32) -> String {
    let sign = if offset_s < 0 { '-' } else { '+' };
    let m = offset_s.unsigned_abs() / 60;
    format!("UTC{sign}{:02}:{:02}", m / 60, m % 60)
}

impl SunToday {
    /// The sun events of the local date of `ms` at `(lat, lon)` in `zone`.
    pub fn at(ms: i64, zone: &Zone, (lat, lon): (f64, f64)) -> Self {
        let l = zone.local(ms);
        let day = ocw_sky::sun::day((l.year, l.month, l.day), lat, lon);
        let hm = |t: Option<i64>| t.map_or_else(|| "-".to_owned(), |t| zone.local(t).hh_mm());
        let pair = |p: ocw_sky::sun::Pair| (hm(p.morning), hm(p.evening));
        Self {
            zone: format!("{}, {}", zone.id(), utc_offset(zone.offset_s(ms))),
            date: long_date(zone, ms),
            sun: pair(day.sun),
            noon: hm(Some(day.solar_noon)),
            civil: pair(day.civil),
            nautical: pair(day.nautical),
            astronomical: pair(day.astronomical),
            key: Self::key(ms, zone, (lat, lon)),
        }
    }

    /// Recomputed when the local date, the zone or the place (to about
    /// 1 km) changes.
    fn key(ms: i64, zone: &Zone, (lat, lon): (f64, f64)) -> (i64, i64, String) {
        (
            (lat * 100.0).round() as i64 * 100_000 + (lon * 100.0).round() as i64,
            0,
            format!("{} {}", zone.id(), zone.local(ms).date()),
        )
    }

    fn is_for(&self, ms: i64, zone: &Zone, place: (f64, f64)) -> bool {
        self.key == Self::key(ms, zone, place)
    }
}

const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// "Sun 21 Jun 2026".
pub fn long_date(zone: &Zone, ms: i64) -> String {
    let l = zone.local(ms);
    format!(
        "{} {} {} {}",
        WEEKDAYS[l.weekday() as usize],
        l.day,
        MONTHS[(l.month - 1) as usize],
        l.year
    )
}

/// "21:15 CEST".
pub fn short_time(zone: &Zone, ms: i64) -> String {
    format!("{} {}", zone.local(ms).hh_mm(), zone.abbreviation(ms))
}

/// "Sun 21 Jun 2026 21:15 CEST".
pub fn long_time(zone: &Zone, ms: i64) -> String {
    format!("{} {}", long_date(zone, ms), short_time(zone, ms))
}

/// Where the sun is computed: the rider's GPS position, the ride's origin,
/// or the default place.
pub fn place_latlon(view: &RideView, client: &ClientCore) -> (f64, f64) {
    view.position
        .map(|p| (p.0, p.1))
        .or(client.geo_origin())
        .unwrap_or(ocw_sky::zone::DEFAULT_PLACE)
}

/// The sky for the renderer and the page's info for this frame: `None`
/// before the server's clock arrived. `prev` keeps the chosen preset and
/// today's sun (recomputed only for a new date or place).
pub fn refresh(
    client: &mut ClientCore,
    now: f64,
    zone: &Zone,
    latlon: (f64, f64),
    night_floor: f32,
    prev: &TimeMachineInfo,
) -> Option<(SkyView, TimeMachineInfo)> {
    let ms = client.world_ms(now)?;
    let sky = Sky::at(ms, zone, latlon, night_floor);
    let clock = client.clock_info().copied();
    let hud = hud_label(&sky, clock.as_ref());
    let sun = match &prev.sun {
        Some(s) if s.is_for(ms, zone, latlon) => s.clone(),
        _ => SunToday::at(ms, zone, latlon),
    };
    let info = TimeMachineInfo {
        time: Some(short_time(zone, ms)),
        date: Some(long_date(zone, ms)),
        mode: clock.map(|c| c.mode),
        mode_label: clock.map_or("real".into(), |c| match c.mode {
            ClockMode::Offset => format!("offset {:+.1} h", c.offset_ms as f64 / 3_600_000.0),
            m => m.name().into(),
        }),
        speed: clock.map_or(1.0, |c| c.speed),
        paused: clock.is_some_and(|c| c.paused),
        control: control_label(clock.as_ref(), client.may_change_clock()).into(),
        may_change: client.may_change_clock(),
        preset: prev.preset,
        sun: Some(sun),
    };
    Some((SkyView { sky, hud }, info))
}

/// Who sets the time, for the page.
pub fn control_label(clock: Option<&ocw_protocol::ClockInfo>, mine: bool) -> &'static str {
    match clock.map(|c| c.access) {
        _ if mine => "you",
        Some(ocw_protocol::ClockAccess::Nobody) => "the organizer",
        _ => "the host",
    }
}

/// The rows of World → Time, the time machine (six rows and Back, ten-foot rule).
pub fn rows(info: &TimeMachineInfo) -> Vec<Row> {
    let line = |l: &str| Some(l.to_owned());
    let speed_line = |dir| match machine::speed_step(info.speed, info.paused, dir) {
        ClockChange::Paused(true) => "time pause".to_owned(),
        ClockChange::Paused(false) => "time play".to_owned(),
        ClockChange::Speed(s) => format!("time speed {s}"),
        _ => unreachable!("speed_step gives speeds and pauses"),
    };
    let preset_line = |dir: isize| {
        let all = Preset::ALL;
        let at = info
            .preset
            .and_then(|p| all.iter().position(|&q| q == p))
            .map_or(if dir > 0 { -1 } else { 0 }, |i| i as isize);
        let p = all[(at + dir).rem_euclid(all.len() as isize) as usize];
        format!("time preset {}", p.id())
    };
    vec![
        Row {
            label: "World time".into(),
            value: Some(info.time.clone().unwrap_or_else(|| "-".into())),
            left: line("time -1h"),
            right: line("time +1h"),
            confirm: None,
        },
        Row {
            label: "Day".into(),
            value: Some(info.date.clone().unwrap_or_else(|| "-".into())),
            left: line("time -1d"),
            right: line("time +1d"),
            confirm: None,
        },
        Row {
            label: "Speed".into(),
            value: Some(ocw_sky::clock::speed_label(info.speed, info.paused)),
            left: Some(speed_line(-1)),
            right: Some(speed_line(1)),
            confirm: Some(Activate::Command(if info.paused {
                "time play".into()
            } else {
                "time pause".into()
            })),
        },
        Row {
            label: "Preset".into(),
            value: Some(info.preset.map_or("-".into(), |p| p.label().into())),
            left: Some(preset_line(-1)),
            right: Some(preset_line(1)),
            confirm: Some(Activate::Command(preset_line(if info.preset.is_some() {
                0
            } else {
                1
            }))),
        },
        Row {
            label: "Mode (Enter: real)".into(),
            value: Some(info.mode_label.clone()),
            left: None,
            right: None,
            confirm: Some(Activate::Command("time real".into())),
        },
        Row {
            label: "Who sets the time".into(),
            value: Some(info.control.clone()),
            left: None,
            right: None,
            confirm: None,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sky::place_zone;
    use ocw_protocol::{ClockAccess, ClockInfo};

    fn info(mode: ClockMode, speed: f64, paused: bool, offset_ms: i64) -> ClockInfo {
        ClockInfo {
            world_ms: 0,
            rate: if paused { 0.0 } else { speed },
            mode,
            speed,
            paused,
            offset_ms,
            access: ClockAccess::Anyone,
            host: None,
        }
    }

    fn vaduz_sky() -> Sky {
        let zone = Zone::get("Europe/Vaduz").unwrap();
        let (ms, _) = zone.instant((2026, 6, 21), (18, 42, 0)).unwrap();
        Sky::at(ms, &zone, ocw_sky::zone::DEFAULT_PLACE, 0.1)
    }

    #[test]
    fn hud_clock_shows_time_and_mode() {
        let sky = vaduz_sky();
        assert_eq!(hud_label(&sky, None), "☼ 18:42");
        assert_eq!(
            hud_label(&sky, Some(&info(ClockMode::Real, 1.0, false, 0))),
            "☼ 18:42"
        );
        assert_eq!(
            hud_label(&sky, Some(&info(ClockMode::Fixed, 600.0, false, 0))),
            "☼ 18:42 x600"
        );
        assert_eq!(
            hud_label(&sky, Some(&info(ClockMode::Fixed, 600.0, true, 0))),
            "☼ 18:42 paused"
        );
        assert_eq!(
            hud_label(&sky, Some(&info(ClockMode::Fixed, 1.0, false, 0))),
            "☼ 18:42 fixed"
        );
        assert_eq!(
            hud_label(&sky, Some(&info(ClockMode::Offset, 1.0, false, 7_200_000))),
            "☼ 18:42 +2 h"
        );
        assert!(ocw_render::font::covers(&hud_label(&sky, None)));
        assert!(ocw_render::font::covers("◐ ●"));
    }

    #[test]
    fn places_get_their_zone() {
        assert_eq!(
            place_zone(Some("Europe/Berlin"), Some((47.1, 9.5))).0.id(),
            "Europe/Berlin"
        );
        let (z, src) = place_zone(Some("Nowhere/Land"), Some((47.92, 7.96)));
        assert_eq!((z.id(), src), ("Europe/Berlin", "origin"));
        let (z, src) = place_zone(None, None);
        assert_eq!((z.id(), src), ("Europe/Vaduz", "default"));
    }

    #[test]
    fn long_times_name_the_zone() {
        let zone = Zone::get("Europe/Vaduz").unwrap();
        let (ms, _) = zone.instant((2026, 6, 21), (21, 15, 0)).unwrap();
        assert_eq!(long_time(&zone, ms), "Sun 21 Jun 2026 21:15 CEST");
        let (ms, _) = zone.instant((2026, 12, 24), (7, 5, 0)).unwrap();
        assert_eq!(long_time(&zone, ms), "Thu 24 Dec 2026 07:05 CET");
    }

    /// The page: six rows (plus Back), each a `time` command line.
    #[test]
    fn page_rows_are_time_commands() {
        let mut i = TimeMachineInfo {
            time: Some("21:15 CEST".into()),
            date: Some("Sun 21 Jun 2026".into()),
            mode: Some(ClockMode::Real),
            mode_label: "real".into(),
            speed: 1.0,
            control: "you".into(),
            may_change: true,
            ..Default::default()
        };
        let r = rows(&i);
        assert_eq!(r.len(), 6);
        assert!(r.len() < super::super::model::MAX_ITEMS);
        assert_eq!(r[0].left.as_deref(), Some("time -1h"));
        assert_eq!(r[1].right.as_deref(), Some("time +1d"));
        assert_eq!(r[2].right.as_deref(), Some("time speed 10"));
        assert_eq!(r[2].left.as_deref(), Some("time pause"));
        assert_eq!(r[3].right.as_deref(), Some("time preset sunrise"));
        assert_eq!(r[3].left.as_deref(), Some("time preset new-moon"));
        assert_eq!(r[4].confirm, Some(Activate::Command("time real".into())));
        i.preset = Some(Preset::Sunset);
        i.paused = true;
        let r = rows(&i);
        assert_eq!(r[3].right.as_deref(), Some("time preset blue-hour"));
        assert_eq!(r[3].left.as_deref(), Some("time preset noon"));
        assert_eq!(r[3].value.as_deref(), Some("Sunset"));
        assert_eq!(r[2].right.as_deref(), Some("time play"));
        assert_eq!(r[2].value.as_deref(), Some("paused"));
        // Every line parses as a time command.
        for row in rows(&i) {
            for l in [row.left, row.right].into_iter().flatten() {
                let args: Vec<&str> = l.split_whitespace().skip(1).collect();
                machine::parse(&args).unwrap_or_else(|e| panic!("{l}: {e}"));
            }
        }
    }

    /// World → About this world → Sun and time: the NOAA events of the
    /// local date in the place's zone (the times of `ocw_sky::sun` tests).
    #[test]
    fn sun_today_in_the_local_zone() {
        let zone = Zone::get("Europe/Vaduz").unwrap();
        let (ms, _) = zone.instant((2026, 6, 21), (14, 0, 0)).unwrap();
        let s = SunToday::at(ms, &zone, (47.141, 9.521));
        assert_eq!(s.zone, "Europe/Vaduz, UTC+02:00");
        assert_eq!(s.date, "Sun 21 Jun 2026");
        // 03:26 / 19:21 UTC: 05:26 / 21:21 CEST.
        assert_eq!(s.sun, ("05:26".into(), "21:21".into()));
        assert_eq!(s.noon, "13:23");
        assert_eq!(s.civil, ("04:46".into(), "22:01".into()));
        assert_eq!(s.astronomical.0, "02:26");
        assert!(s.is_for(ms + 3_600_000, &zone, (47.141, 9.521)), "same day");
        let (next, _) = zone.instant((2026, 6, 22), (0, 30, 0)).unwrap();
        assert!(!s.is_for(next, &zone, (47.141, 9.521)), "the next day");
        assert!(!s.is_for(ms, &zone, (47.995, 7.85)), "another place");
        // Far north in June: no astronomical night.
        let n = SunToday::at(ms, &zone, (64.0, 9.5));
        assert_eq!(n.astronomical, ("-".into(), "-".into()));
        assert_eq!(utc_offset(-12_600), "UTC-03:30");
        assert_eq!(utc_offset(0), "UTC+00:00");
    }

    #[test]
    fn control_labels() {
        let mut c = info(ClockMode::Real, 1.0, false, 0);
        assert_eq!(control_label(Some(&c), true), "you");
        c.access = ClockAccess::Host;
        assert_eq!(control_label(Some(&c), false), "the host");
        c.access = ClockAccess::Nobody;
        assert_eq!(control_label(Some(&c), false), "the organizer");
    }
}
