//! Settings → Traffic (spec 14 Phase 18, 18A, T1): one shared page for
//! both apps. The master switch, the road traffic and the driver manners
//! (the session's: solo the rider's, in a shared ride the host's, in an
//! event the organiser's, Q2 – dimmed with who decides when it is not the
//! rider's), aircraft and UFOs (the rider's own view), and a second page
//! with the sounds and the live rows: *Aircraft: live* (T10: on / off,
//! with whether the live aircraft are drawn now or why the simulated sky
//! is) and *Live traffic data* (T13: switchable where the world has a
//! live road source – Tenerife: the DGT's closures, roadworks and reported
//! jams, with the source's credit –, dimmed with why elsewhere, Q5). Road
//! traffic is in the desktop app only (Q9 (b)): its rows are left out on
//! [`Platform::Browser`] (the browser, phones, the Quest).

use ocw_protocol::traffic::TrafficInfo;

use crate::live_air::LiveState;
use crate::live_road::LiveRoadRow;

use super::model::Platform;
use super::pages::{Activate, Page, Row, open_row, setting_row};
use crate::settings::Settings;

/// The page note of Settings → Traffic.
pub const NOTE: &str = "Road traffic is the ride's: solo yours, in a shared ride the host's, in events the organiser's (races: none). Aircraft and UFOs are for you only.";
/// … of its second page (with the live aircraft's credit, 18D).
pub const MORE_NOTE: &str = "Live aircraft come from the game server (Aircraft data: adsb.lol (ODbL)), with simulated gliders, balloons and local flights.";
/// The live aircraft's source, as Settings names it.
pub const AIR_LIVE_SOURCE: &str = "adsb.lol (ODbL)";
/// … and what flies with the live aircraft (spec 14 Phase 18 Q13 (c)).
pub const AIR_LIVE_MIX: &str = "+ simulated gliders, balloons";
/// The *Live traffic data* row's value (Q5: greyed until a world has a
/// source that suits us).
pub const LIVE_ROAD_NONE: &str = "no live data for this world yet";
/// … while the road traffic is off (18D: it only matters with it on).
pub const LIVE_ROAD_ROAD_OFF: &str = "turn on road traffic first";

/// What the page needs besides the settings.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
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
    /// With *Aircraft: live*: drawn now, or why not (`None`: not chosen).
    pub live_air: Option<LiveState>,
    /// The world's live road data as the server told (T13).
    pub live_road: LiveRoadRow,
}

impl TrafficMenuInfo {
    /// Outside a ride (the rider decides for the next one).
    pub fn alone(platform: Platform) -> Self {
        Self {
            platform,
            session: None,
            may_change: true,
            decided_by: "you",
            live_air: None,
            live_road: LiveRoadRow::Unknown,
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
            out.push(air_row(settings, info.live_air));
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
                live_row(settings, info.live_air),
            ];
            if desktop {
                out.push(live_road_row(settings, info));
            }
            out
        }
        _ => Vec::new(),
    }
}

/// *Live traffic data* (T13, Q5): switchable where the world has a source
/// (with what it gives and its credit), dimmed with why elsewhere and
/// while the road traffic is off.
fn live_road_row(settings: &Settings, info: &TrafficMenuInfo) -> Row {
    const LABEL: &str = "Live traffic data";
    let road_on = text(settings, "traffic.enabled") != "off"
        && match info.session {
            Some(s) => s.rules.level.is_on(),
            None => text(settings, "traffic.road") != "off",
        };
    let on = text(settings, "traffic.live_road") == "on";
    let value = match &info.live_road {
        LiveRoadRow::Unknown => return disabled(LABEL, LIVE_ROAD_NONE.into()),
        LiveRoadRow::NoSource(why) | LiveRoadRow::Unavailable(why) => {
            let why = if why.is_empty() { LIVE_ROAD_NONE } else { why };
            return disabled(LABEL, why.into());
        }
        _ if !road_on => return disabled(LABEL, LIVE_ROAD_ROAD_OFF.into()),
        LiveRoadRow::Available { attribution, what } => {
            let live = format!("Live: {what} – {attribution}");
            if on {
                format!("on – {live}")
            } else {
                format!("off ({live})")
            }
        }
        LiveRoadRow::Live { attribution, .. } if on => format!("on – {attribution}"),
        LiveRoadRow::Waiting { note, .. } if on => format!("on – {note}"),
        LiveRoadRow::Live { attribution, .. } | LiveRoadRow::Waiting { attribution, .. } => {
            format!("off ({attribution})")
        }
    };
    let line = if on {
        "set traffic.live_road off"
    } else {
        "set traffic.live_road on"
    };
    Row {
        label: LABEL.into(),
        value: Some(value),
        left: Some(line.into()),
        right: Some(line.into()),
        confirm: Some(Activate::Command(line.into())),
    }
}

/// The aircraft row: off / simulated / live (live is switched on its own
/// row, *Aircraft: live*; here it shows whether it is drawn now).
fn air_row(settings: &Settings, live: Option<LiveState>) -> Row {
    let v = text(settings, "traffic.air");
    let (value, other) = match v.as_str() {
        "off" => ("off".to_owned(), "simulated"),
        "live" => (
            match live {
                Some(LiveState::Live) | None => "live".to_owned(),
                Some(_) => "live (simulated now)".to_owned(),
            },
            "off",
        ),
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

/// *Aircraft: live* (T10): on / off, and whether the live aircraft are
/// drawn now or why the simulated sky is.
fn live_row(settings: &Settings, live: Option<LiveState>) -> Row {
    let on = text(settings, "traffic.air") == "live";
    let value = match (on, live) {
        (false, _) => "off".to_owned(),
        (true, Some(LiveState::Live) | None) => format!("on – {AIR_LIVE_SOURCE} {AIR_LIVE_MIX}"),
        (true, Some(s)) => format!("on – {}", s.label()),
    };
    let line = if on {
        "set traffic.air simulated"
    } else {
        "set traffic.air live"
    };
    Row {
        label: "Aircraft: live".into(),
        value: Some(value),
        left: Some(line.into()),
        right: Some(line.into()),
        confirm: Some(Activate::Command(line.into())),
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
        // T10: *Aircraft: live* switches; Q5: dimmed with why.
        assert_eq!(row(&dm, "Aircraft: live").value.as_deref(), Some("off"));
        assert_eq!(
            row(&dm, "Aircraft: live").confirm,
            Some(Activate::Command("set traffic.air live".into()))
        );
        assert_eq!(
            row(&dm, "Live traffic data").confirm,
            Some(Activate::Disabled)
        );
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
        // Live (T10): drawn, or the simulated sky now with why.
        let mut live = Settings::default();
        live.set("traffic.air", "live").unwrap();
        let drawn = TrafficMenuInfo {
            live_air: Some(LiveState::Live),
            ..TrafficMenuInfo::alone(Platform::Browser)
        };
        let r = rows(Page::Traffic, &live, &drawn);
        assert_eq!(row(&r, "Aircraft").value.as_deref(), Some("live"));
        let m = rows(Page::TrafficMore, &live, &drawn);
        assert_eq!(
            row(&m, "Aircraft: live").value.as_deref(),
            Some("on – adsb.lol (ODbL) + simulated gliders, balloons")
        );
        assert!(
            MORE_NOTE.contains("with simulated gliders, balloons and local flights"),
            "what is live and what is simulated (Q13 (c))"
        );
        assert_eq!(
            row(&m, "Aircraft: live").confirm,
            Some(Activate::Command("set traffic.air simulated".into()))
        );
        let down = TrafficMenuInfo {
            live_air: Some(LiveState::Down),
            ..drawn
        };
        let r = rows(Page::Traffic, &live, &down);
        assert_eq!(
            row(&r, "Aircraft").value.as_deref(),
            Some("live (simulated now)")
        );
        let m = rows(Page::TrafficMore, &live, &down);
        assert_eq!(
            row(&m, "Aircraft: live").value.as_deref(),
            Some("on – simulated: the live feed is down")
        );
        assert!(MORE_NOTE.contains("adsb.lol (ODbL)"), "the credit (18D)");
    }

    /// T13: *Live traffic data* switchable on Tenerife with the credit,
    /// dimmed elsewhere, without road traffic and on a server that fetches
    /// none.
    #[test]
    fn live_traffic_data_where_the_world_has_a_source() {
        let mut s = Settings::default();
        s.set("traffic.road", "normal").unwrap();
        let with = |row: LiveRoadRow| TrafficMenuInfo {
            live_road: row,
            ..TrafficMenuInfo::alone(Platform::Desktop)
        };
        let dgt = LiveRoadRow::Available {
            attribution: "Fuente: DGT".into(),
            what: "closures, roadworks and reported jams".into(),
        };
        let r = rows(Page::TrafficMore, &s, &with(dgt.clone()));
        let row = row(&r, "Live traffic data");
        assert_eq!(
            row.value.as_deref(),
            Some("off (Live: closures, roadworks and reported jams – Fuente: DGT)")
        );
        assert_eq!(
            row.confirm,
            Some(Activate::Command("set traffic.live_road on".into()))
        );
        s.set("traffic.live_road", "on").unwrap();
        let live = with(LiveRoadRow::Live {
            attribution: "Fuente: DGT".into(),
            events: 12,
        });
        let r = rows(Page::TrafficMore, &s, &live);
        let on = r.iter().find(|r| r.label == "Live traffic data").unwrap();
        assert_eq!(on.value.as_deref(), Some("on – Fuente: DGT"));
        assert_eq!(
            on.confirm,
            Some(Activate::Command("set traffic.live_road off".into()))
        );
        let down = with(LiveRoadRow::Waiting {
            attribution: "Fuente: DGT".into(),
            note: "live data not reachable – using statistics".into(),
        });
        let r = rows(Page::TrafficMore, &s, &down);
        assert_eq!(
            r.iter()
                .find(|r| r.label == "Live traffic data")
                .unwrap()
                .value
                .as_deref(),
            Some("on – live data not reachable – using statistics")
        );
        // Elsewhere: dimmed with the reason.
        for (row_state, why) in [
            (LiveRoadRow::NoSource(LIVE_ROAD_NONE.into()), LIVE_ROAD_NONE),
            (
                LiveRoadRow::Unavailable("this server does not fetch live road data".into()),
                "this server does not fetch live road data",
            ),
        ] {
            let r = rows(Page::TrafficMore, &s, &with(row_state));
            let x = r.iter().find(|r| r.label == "Live traffic data").unwrap();
            assert_eq!(x.confirm, Some(Activate::Disabled));
            assert_eq!(x.value.as_deref(), Some(why));
        }
        // Without road traffic.
        s.set("traffic.road", "off").unwrap();
        let r = rows(Page::TrafficMore, &s, &with(dgt));
        let x = r.iter().find(|r| r.label == "Live traffic data").unwrap();
        assert_eq!(x.value.as_deref(), Some(LIVE_ROAD_ROAD_OFF));
        assert_eq!(x.confirm, Some(Activate::Disabled));
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
            live_air: None,
            live_road: LiveRoadRow::Unknown,
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
