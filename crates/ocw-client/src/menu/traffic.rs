//! Settings → Traffic (spec 14 Phase 18, 18A, T1): one shared page for
//! both apps. The master switch, the road traffic and the driver manners
//! (the session's: solo the rider's, in a shared ride the host's, in an
//! event the organiser's, Q2 – dimmed with who decides when it is not the
//! rider's), aircraft and UFOs (the rider's own view), and a second page
//! with the sounds and the live rows (*Aircraft: live*, *Live traffic
//! data*: present, dimmed with why until their sources are built, Q5 /
//! Q6). Road traffic is in the desktop app only (Q9 (b)): its rows are left
//! out on [`Platform::Browser`] (the browser, phones, the Quest).

use ocw_protocol::traffic::TrafficInfo;

use super::model::Platform;
use super::pages::{Activate, Page, Row, open_row, setting_row};
use crate::settings::Settings;

/// The page note of Settings → Traffic.
pub const NOTE: &str = "Road traffic is the ride's: solo yours, in a shared ride the host's, in events the organiser's (races: none). Aircraft and UFOs are for you only.";
/// … of its second page.
pub const MORE_NOTE: &str = "Live aircraft and live road data come once their sources are built; until then the simulated ones.";
/// The *Aircraft: live* row's value (Q6, T10).
pub const AIR_LIVE_SOON: &str = "not available yet";
/// The *Live traffic data* row's value (Q5: greyed until a world has a
/// source that suits us).
pub const LIVE_ROAD_NONE: &str = "no live data for this world yet";

/// What the page needs besides the settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TrafficMenuInfo {
    pub platform: Platform,
    /// The session's road traffic as the server sent it (`None`: no ride,
    /// or an older server).
    pub session: Option<TrafficInfo>,
    /// The rider may change the road traffic now.
    pub may_change: bool,
    /// Who decides: "you", "the host", "the organiser", "the event's
    /// organiser".
    pub decided_by: &'static str,
}

impl TrafficMenuInfo {
    /// Outside a ride (the rider decides for the next one).
    pub fn alone(platform: Platform) -> Self {
        Self {
            platform,
            session: None,
            may_change: true,
            decided_by: "you",
        }
    }
}

/// Whether `page` is one of the traffic pages.
pub fn is_traffic_page(page: Page) -> bool {
    matches!(page, Page::Traffic | Page::TrafficMore)
}

fn disabled(label: &str, value: String) -> Row {
    Row {
        label: label.into(),
        value: Some(value),
        left: None,
        right: None,
        confirm: Some(Activate::Disabled),
    }
}

fn text(settings: &Settings, key: &str) -> String {
    settings.get(key).map(|v| v.to_string()).unwrap_or_default()
}

/// The rows of a traffic page.
pub fn rows(page: Page, settings: &Settings, info: &TrafficMenuInfo) -> Vec<Row> {
    let desktop = info.platform == Platform::Desktop;
    match page {
        Page::Traffic => {
            let mut out = vec![setting_row(settings, "traffic.enabled", "Traffic")];
            if desktop {
                if info.may_change {
                    out.push(setting_row(settings, "traffic.road", "Road traffic"));
                    let mut m = setting_row(settings, "traffic.manners", "Driver manners");
                    m.value = Some(text(settings, "traffic.manners").replace('_', " "));
                    out.push(m);
                } else {
                    // Not the rider's (Q2): what the session has, and who
                    // decides.
                    let r = info.session.map(|s| s.rules).unwrap_or_default();
                    out.push(disabled(
                        "Road traffic",
                        format!("{} ({})", r.level.key(), info.decided_by),
                    ));
                    out.push(disabled(
                        "Driver manners",
                        format!("{} ({})", r.manners.label(), info.decided_by),
                    ));
                }
            }
            out.push(air_row(settings));
            out.push(setting_row(settings, "traffic.ufo", "UFOs"));
            out.push(open_row(
                "Sound and live data",
                Some(format!("sound {}", text(settings, "traffic.sound"))),
                Page::TrafficMore,
            ));
            out
        }
        Page::TrafficMore => {
            let mut out = vec![
                setting_row(settings, "traffic.sound", "Sound"),
                // Q6: live aircraft come with T10.
                disabled("Aircraft: live", AIR_LIVE_SOON.into()),
            ];
            if desktop {
                // Q5: greyed until a world has a source (18D).
                out.push(disabled("Live traffic data", LIVE_ROAD_NONE.into()));
            }
            out
        }
        _ => Vec::new(),
    }
}

/// The aircraft row: off / simulated (live is its own dimmed row until
/// the feed is built; a `live` set in the console shows as simulated).
fn air_row(settings: &Settings) -> Row {
    let v = text(settings, "traffic.air");
    let (value, other) = match v.as_str() {
        "off" => ("off".to_owned(), "simulated"),
        "live" => ("live (simulated)".to_owned(), "off"),
        _ => ("simulated".to_owned(), "off"),
    };
    let line = format!("set traffic.air {other}");
    Row {
        label: "Aircraft".into(),
        value: Some(value),
        left: Some(line.clone()),
        right: Some(line.clone()),
        confirm: Some(Activate::Command(line)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menu::model::MAX_ITEMS;
    use ocw_core::traffic::{DriverManners, RoadRules, RoadTraffic};
    use ocw_protocol::ClockAccess;

    fn labels(r: &[Row]) -> Vec<&str> {
        r.iter().map(|r| r.label.as_str()).collect()
    }

    fn row<'a>(r: &'a [Row], label: &str) -> &'a Row {
        r.iter().find(|r| r.label == label).expect(label)
    }

    /// Q9: the road rows on the desktop only; the sky everywhere; every
    /// page fits the ten-foot rule.
    #[test]
    fn the_rows_on_every_platform() {
        let s = Settings::default();
        let d = rows(
            Page::Traffic,
            &s,
            &TrafficMenuInfo::alone(Platform::Desktop),
        );
        assert_eq!(
            labels(&d),
            [
                "Traffic",
                "Road traffic",
                "Driver manners",
                "Aircraft",
                "UFOs",
                "Sound and live data"
            ]
        );
        let b = rows(
            Page::Traffic,
            &s,
            &TrafficMenuInfo::alone(Platform::Browser),
        );
        assert_eq!(
            labels(&b),
            ["Traffic", "Aircraft", "UFOs", "Sound and live data"]
        );
        let dm = rows(
            Page::TrafficMore,
            &s,
            &TrafficMenuInfo::alone(Platform::Desktop),
        );
        assert_eq!(
            labels(&dm),
            ["Sound", "Aircraft: live", "Live traffic data"]
        );
        let bm = rows(
            Page::TrafficMore,
            &s,
            &TrafficMenuInfo::alone(Platform::Browser),
        );
        assert_eq!(labels(&bm), ["Sound", "Aircraft: live"]);
        for r in [&d, &b, &dm, &bm] {
            assert!(r.len() < MAX_ITEMS, "with Back on one page");
        }
        // Q5 / Q6: present, dimmed, with why.
        assert_eq!(row(&dm, "Aircraft: live").confirm, Some(Activate::Disabled));
        assert_eq!(
            row(&dm, "Live traffic data").value.as_deref(),
            Some(LIVE_ROAD_NONE)
        );
    }

    /// Q1 on a first start, and what the rows' keys run.
    #[test]
    fn first_start_values_and_commands() {
        let s = Settings::default();
        let r = rows(
            Page::Traffic,
            &s,
            &TrafficMenuInfo::alone(Platform::Desktop),
        );
        assert_eq!(row(&r, "Traffic").value.as_deref(), Some("on"));
        assert_eq!(row(&r, "Road traffic").value.as_deref(), Some("off"));
        assert_eq!(
            row(&r, "Road traffic").right.as_deref(),
            Some("set traffic.road light")
        );
        assert_eq!(
            row(&r, "Road traffic").left.as_deref(),
            Some("set traffic.road lunatic")
        );
        assert_eq!(row(&r, "Driver manners").value.as_deref(), Some("polite"));
        assert_eq!(
            row(&r, "Driver manners").right.as_deref(),
            Some("set traffic.manners mostly_polite")
        );
        assert_eq!(row(&r, "Aircraft").value.as_deref(), Some("simulated"));
        assert_eq!(
            row(&r, "Aircraft").confirm,
            Some(Activate::Command("set traffic.air off".into()))
        );
        assert_eq!(row(&r, "UFOs").value.as_deref(), Some("off"));
        assert_eq!(
            row(&r, "Sound and live data").confirm,
            Some(Activate::Open(Page::TrafficMore))
        );
        // Live set in the console: shown as simulated until the feed is
        // built.
        let mut live = Settings::default();
        live.set("traffic.air", "live").unwrap();
        let r = rows(
            Page::Traffic,
            &live,
            &TrafficMenuInfo::alone(Platform::Desktop),
        );
        assert_eq!(
            row(&r, "Aircraft").value.as_deref(),
            Some("live (simulated)")
        );
    }

    /// Q2: in a shared ride the host's road traffic shows dimmed with who
    /// decides; the sky stays the rider's.
    #[test]
    fn a_guest_sees_the_hosts_road_traffic_dimmed() {
        let s = Settings::default();
        let info = TrafficMenuInfo {
            platform: Platform::Desktop,
            session: Some(TrafficInfo {
                rules: RoadRules {
                    level: RoadTraffic::Busy,
                    manners: DriverManners::MostlyPolite,
                },
                access: ClockAccess::Host,
                event: false,
            }),
            may_change: false,
            decided_by: "the host",
        };
        let r = rows(Page::Traffic, &s, &info);
        let road = row(&r, "Road traffic");
        assert_eq!(road.value.as_deref(), Some("busy (the host)"));
        assert_eq!(road.confirm, Some(Activate::Disabled));
        assert!(road.left.is_none() && road.right.is_none());
        assert_eq!(
            row(&r, "Driver manners").value.as_deref(),
            Some("mostly polite (the host)")
        );
        assert!(row(&r, "UFOs").right.is_some(), "the rider's own");
        assert!(is_traffic_page(Page::Traffic) && is_traffic_page(Page::TrafficMore));
        assert!(!is_traffic_page(Page::Display));
    }
}
