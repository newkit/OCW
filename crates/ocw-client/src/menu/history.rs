//! The training history's pages (spec §14 Phase 7; Navi → Training
//! history), Bevy-free: rows, the description of a ride, the status line
//! and the power-curve chart, from a [`HistoryInfo`] the app fills
//! (`ocw_ui::history`). Each page keeps the ten-foot rule (six rows and
//! Back, longer lists paged).

use std::collections::BTreeSet;

use ocw_render::menu::{ChartBar, MenuChart};

use super::pages::{Activate, Page, Row, info_row, open_row, short_label};
use crate::history::{
    Bests, CURVE_S, History, Period, distance_label, duration_label, height_label, hms,
    start_label, total_summary,
};

/// What the pages show.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HistoryInfo {
    pub history: History,
    /// Now (UTC Unix s) and the local UTC offset (s).
    pub now_unix: i64,
    pub offset_s: i32,
    pub imperial: bool,
    /// FIT files not in the history yet (the desktop's import); `None`:
    /// the app has no import.
    pub importable: Option<usize>,
    /// The rides whose FIT file can be exported (ids).
    pub exportable: BTreeSet<String>,
    /// Why the history is not saved (a newer or broken file).
    pub read_only: Option<String>,
    /// The ride photographer's photos of each ride (spec §2.11; by ride
    /// id), and how many of them lie in a Strava recording (§2.8.1 ST3).
    pub photos: std::collections::BTreeMap<String, RidePhotos>,
    /// The automatic Strava upload through the rider's own API app is set
    /// up (`strava.toml`, the advanced way, spec §2.8.1); without it a
    /// ride goes to Strava by the simple upload in the browser.
    pub strava_api: bool,
    /// The Strava recordings (S … S) within each ride (by ride id; the
    /// desktop): the simple upload sends those instead of the whole ride.
    pub recordings: std::collections::BTreeMap<String, usize>,
    /// The last simple upload: the ride's id and its steps (shown on the
    /// ride's page).
    pub upload: Option<(String, String)>,
}

/// The label of the simple upload's row (spec §2.8.1 *Simple upload*).
pub const UPLOAD_LABEL: &str = "Upload to Strava (in your browser)";
/// The label of the mark's row.
pub const MARK_LABEL: &str = "Marked as uploaded";
/// The hint under the steps.
pub const MARK_HINT: &str = "If you uploaded it, mark it as uploaded (below).";

/// A ride's photos (spec §2.11, §2.8.1 ST3).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RidePhotos {
    /// The photo files (`photo-1.jpg`, …) with their ride seconds.
    pub files: Vec<(String, f64)>,
    /// How many of them were taken during a Strava recording, and the
    /// recordings.
    pub in_strava: usize,
    pub recordings: usize,
}

impl RidePhotos {
    /// The row's value: "3 photos (2 in Strava recordings)".
    pub fn summary(&self) -> String {
        let n = self.files.len();
        let mut s = format!("{n} photo{}", if n == 1 { "" } else { "s" });
        if self.recordings > 0 {
            s.push_str(&format!(
                ", {} in {} Strava recording{}",
                self.in_strava,
                self.recordings,
                if self.recordings == 1 { "" } else { "s" }
            ));
        }
        s
    }
}

/// The durations shown as rows of the bests page (the chart has all; the
/// page with its chart fits 16:9 with four rows).
const BEST_ROWS_S: [u32; 2] = [5, 1200];

fn date_of(info: &HistoryInfo, unix: i64) -> String {
    crate::history::day_label(crate::history::local_day(unix, info.offset_s))
}

/// The rows of a history page.
pub fn rows(page: Page, info: &HistoryInfo) -> Vec<Row> {
    let h = &info.history;
    let imp = info.imperial;
    match page {
        Page::History => {
            let bests = h.bests();
            let mut out = vec![
                if h.rides.is_empty() {
                    info_row("Rides", Some("none yet".into()))
                } else {
                    open_row("Rides", Some(h.rides.len().to_string()), Page::HistoryRides)
                },
                open_row(
                    "This week",
                    Some(total_summary(
                        &h.current(Period::Week, info.now_unix, info.offset_s),
                        imp,
                    )),
                    Page::HistoryWeeks,
                ),
                open_row(
                    "This month",
                    Some(total_summary(
                        &h.current(Period::Month, info.now_unix, info.offset_s),
                        imp,
                    )),
                    Page::HistoryMonths,
                ),
                open_row(
                    "Personal bests",
                    Some(
                        [1200, 300, 60, 5]
                            .iter()
                            .find_map(|&s| {
                                bests
                                    .power(s)
                                    .map(|b| format!("{}: {:.0} W", duration_label(s), b.value))
                            })
                            .unwrap_or_else(|| "none yet".into()),
                    ),
                    Page::HistoryBests,
                ),
            ];
            if let Some(n) = info.importable {
                out.push(Row {
                    label: "Import saved rides".into(),
                    value: Some(if n == 0 {
                        "all in".into()
                    } else {
                        format!("{n} new")
                    }),
                    left: None,
                    right: None,
                    confirm: Some(if n == 0 {
                        Activate::Disabled
                    } else {
                        Activate::Command("history.import".into())
                    }),
                });
            }
            out
        }
        Page::HistoryRides => h
            .rides
            .iter()
            .rev()
            .enumerate()
            .map(|(k, r)| {
                open_row(
                    &start_label(r.start_unix_s, info.offset_s),
                    Some(format!(
                        "{}, {}",
                        distance_label(r.distance_m, imp),
                        short_label(
                            if r.world.is_empty() {
                                &r.route
                            } else {
                                &r.world
                            },
                            18
                        )
                    )),
                    Page::HistoryRide(k),
                )
            })
            .collect(),
        Page::HistoryRide(k) => {
            let Some(r) = h.nth_newest(k) else {
                return Vec::new();
            };
            let mut out = vec![if r.climbs.is_empty() {
                info_row("Climbs", Some("none".into()))
            } else {
                open_row(
                    "Climbs",
                    Some(r.climbs.len().to_string()),
                    Page::HistoryClimbs(k),
                )
            }];
            out.push(if info.exportable.contains(&r.id) {
                Row {
                    label: "Export the FIT file".into(),
                    value: None,
                    left: None,
                    right: None,
                    confirm: Some(Activate::Command(format!("history.export {}", k + 1))),
                }
            } else {
                Row {
                    label: "Export the FIT file".into(),
                    value: Some("not kept".into()),
                    left: None,
                    right: None,
                    confirm: Some(Activate::Disabled),
                }
            });
            // The ride photographer's photos (spec §2.11): the row opens
            // their folder, to add them in Strava by hand (§2.8.1 Q2).
            if let Some(p) = info.photos.get(&r.id).filter(|p| !p.files.is_empty()) {
                out.push(Row {
                    label: "Photos: open the folder".into(),
                    value: Some(p.summary()),
                    left: None,
                    right: None,
                    confirm: Some(Activate::Command(format!("history.photos {}", k + 1))),
                });
            }
            out.extend(strava_rows(k, r, info));
            out
        }
        Page::HistoryClimbs(k) => {
            let Some(r) = h.nth_newest(k) else {
                return Vec::new();
            };
            let rows: Vec<Row> = r
                .climbs
                .iter()
                .map(|c| {
                    info_row(
                        &format!("At {}", distance_label(c.start_m, imp)),
                        Some(format!(
                            "{}, {} at {:.1} %, {}, VAM {:.0}",
                            distance_label(c.length_m, imp),
                            height_label(c.gain_m, imp),
                            c.grade() * 100.0,
                            hms(c.time_s),
                            c.vam()
                        )),
                    )
                })
                .collect();
            if rows.is_empty() {
                vec![info_row("No climbs in this ride", None)]
            } else {
                rows
            }
        }
        Page::HistoryWeeks | Page::HistoryMonths => {
            let period = if page == Page::HistoryWeeks {
                Period::Week
            } else {
                Period::Month
            };
            let rows: Vec<Row> = h
                .totals(period, info.offset_s)
                .iter()
                .map(|t| info_row(&t.label, Some(total_summary(t, imp))))
                .collect();
            if rows.is_empty() {
                vec![info_row("No rides yet", None)]
            } else {
                rows
            }
        }
        Page::HistoryBests => bests_rows(&h.bests(), info),
        _ => Vec::new(),
    }
}

/// A ride's Strava rows (spec §2.8.1 *Simple upload*): with the API set
/// up, the automatic upload's line; without it, the upload in the browser
/// (the ride's Strava recordings if it has any, else the whole ride).
/// Both have the mark, which the rider sets after uploading by hand (the
/// app cannot know) and the API sync respects.
fn strava_rows(k: usize, r: &crate::history::RideEntry, info: &HistoryInfo) -> Vec<Row> {
    let n = k + 1;
    let kept = info.exportable.contains(&r.id);
    let mut out = Vec::new();
    if info.strava_api {
        out.push(info_row(
            "Strava upload",
            Some(if r.strava_marked {
                "skipped (marked as uploaded)".into()
            } else {
                "automatic (your API app)".into()
            }),
        ));
    } else {
        let recordings = info.recordings.get(&r.id).copied().unwrap_or(0);
        out.push(Row {
            label: UPLOAD_LABEL.into(),
            value: if !kept {
                Some("FIT file not kept".into())
            } else {
                match recordings {
                    0 => None,
                    1 => Some("your Strava recording".into()),
                    m => Some(format!("your {m} Strava recordings")),
                }
            },
            left: None,
            right: None,
            confirm: Some(if kept {
                Activate::Command(format!("strava.upload-manual {n}"))
            } else {
                Activate::Disabled
            }),
        });
    }
    out.push(Row {
        label: MARK_LABEL.into(),
        value: Some(if r.strava_marked { "yes" } else { "no" }.into()),
        left: Some(format!("strava.mark-uploaded {n} off")),
        right: Some(format!("strava.mark-uploaded {n} on")),
        confirm: Some(Activate::Command(format!(
            "strava.mark-uploaded {n} toggle"
        ))),
    });
    out
}

fn bests_rows(b: &Bests, info: &HistoryInfo) -> Vec<Row> {
    let imp = info.imperial;
    let mut out: Vec<Row> = BEST_ROWS_S
        .iter()
        .map(|&s| {
            info_row(
                &duration_label(s),
                Some(b.power(s).map_or("-".into(), |p| {
                    format!("{:.0} W, {}", p.value, date_of(info, p.start_unix_s))
                })),
            )
        })
        .collect();
    out.push(info_row(
        "Biggest climb",
        Some(b.biggest_climb.as_ref().map_or("-".into(), |c| {
            format!(
                "{} at {:.1} %, {}",
                height_label(c.value.gain_m, imp),
                c.value.grade() * 100.0,
                date_of(info, c.start_unix_s)
            )
        })),
    ));
    out.push(info_row(
        "Fastest climbing",
        Some(b.fastest_climb.as_ref().map_or("-".into(), |c| {
            format!(
                "VAM {:.0} m/h, {}",
                c.value.vam(),
                date_of(info, c.start_unix_s)
            )
        })),
    ));
    out
}

/// The line under the title.
pub fn status(page: Page, info: &HistoryInfo) -> String {
    let h = &info.history;
    match page {
        Page::History => info.read_only.clone().unwrap_or_default(),
        Page::HistoryRide(k) | Page::HistoryClimbs(k) => {
            h.nth_newest(k).map_or(String::new(), |r| {
                let place = match (r.world.is_empty(), r.route.is_empty()) {
                    (false, false) if r.world != r.route => format!("{} - {}", r.world, r.route),
                    (false, _) => r.world.clone(),
                    (true, _) => r.route.clone(),
                };
                format!("{}   {}", start_label(r.start_unix_s, info.offset_s), place)
            })
        }
        Page::HistoryBests => {
            let b = h.bests();
            match &b.longest_ride {
                Some(l) => format!(
                    "{} rides; longest {} ({})",
                    h.rides.len(),
                    distance_label(l.value, info.imperial),
                    date_of(info, l.start_unix_s)
                ),
                None => "No rides yet.".into(),
            }
        }
        _ => String::new(),
    }
}

/// The description of a ride's page: its numbers.
pub fn text(page: Page, info: &HistoryInfo) -> Vec<String> {
    let Page::HistoryRide(k) = page else {
        return Vec::new();
    };
    let Some(r) = info.history.nth_newest(k) else {
        return Vec::new();
    };
    let imp = info.imperial;
    let mut first = format!(
        "{} in {} (moving {}), {} up, {:.0} kJ",
        distance_label(r.distance_m, imp),
        hms(r.elapsed_s),
        hms(r.moving_s),
        height_label(r.ascent_m, imp),
        r.work_kj
    );
    if let Some(n) = r.hearts {
        first.push_str(&format!(", {n} hearts"));
    }
    first.push('.');
    let mut second = Vec::new();
    if let Some(p) = r.avg_power_w {
        let mut s = format!("Power {p:.0} W");
        if let Some(np) = r.np_w {
            s.push_str(&format!(", NP {np:.0} W"));
        }
        if let Some(m) = r.max_power_w {
            s.push_str(&format!(", max {m:.0} W"));
        }
        second.push(s);
    }
    if let (Some(a), Some(m)) = (r.avg_hr_bpm, r.max_hr_bpm) {
        second.push(format!("heart rate {a:.0} / {m:.0} bpm"));
    }
    match r.crashes {
        0 => {}
        1 => second.push("1 crash".into()),
        n => second.push(format!("{n} crashes")),
    }
    if let Some(d) = r.damage {
        second.push(format!(
            "rider {:.0} %, bike {:.0} %",
            d.rider * 100.0,
            d.bike * 100.0
        ));
    }
    if let Some(w) = &r.workout {
        second.push(format!("workout: {}", short_label(w, 28)));
    }
    // The simple upload's steps first, after it ran for this ride (a page
    // too tall drops its text from the end: the numbers go first).
    let mut out = Vec::new();
    if let Some(steps) = upload_steps_for(info, &r.id) {
        out.push(steps.to_string());
        if !r.strava_marked {
            out.push(MARK_HINT.into());
        }
    }
    out.push(first);
    if !second.is_empty() {
        out.push(format!("{}.", second.join("; ")));
    }
    out
}

/// The simple upload's steps, if it ran for ride `id`.
fn upload_steps_for<'a>(info: &'a HistoryInfo, id: &str) -> Option<&'a str> {
    info.upload
        .as_ref()
        .filter(|(i, _)| i == id)
        .map(|(_, s)| s.as_str())
}

/// "5s", "1m", "20m".
fn bar_label(s: u32) -> String {
    if s < 60 {
        format!("{s}s")
    } else {
        format!("{}m", s / 60)
    }
}

/// The power curve: of a ride (bars) against the bests (lines), or the
/// bests alone.
pub fn chart(page: Page, info: &HistoryInfo) -> Option<MenuChart> {
    let bests = info.history.bests();
    let (ride, caption) = match page {
        Page::HistoryRide(k) => {
            let r = info.history.nth_newest(k)?;
            // The upload's steps take the chart's room.
            if r.power_curve.is_empty() || upload_steps_for(info, &r.id).is_some() {
                return None;
            }
            (
                Some(r),
                "Power curve (W): bars this ride, lines your best".to_string(),
            )
        }
        Page::HistoryBests if !bests.curve.is_empty() => {
            (None, "Your best power curve (W)".to_string())
        }
        _ => return None,
    };
    let best_at = |s| bests.power(s).map(|b| b.value);
    let max = CURVE_S
        .iter()
        .filter_map(|&s| best_at(s))
        .chain(ride.iter().flat_map(|r| r.power_curve.iter().map(|p| p.w)))
        .fold(0.0f64, f64::max);
    if max <= 0.0 {
        return None;
    }
    let bars = CURVE_S
        .iter()
        .filter_map(|&s| {
            let best = best_at(s);
            let v = match ride {
                Some(r) => r.curve_at(s)?,
                None => best?,
            };
            Some(ChartBar {
                label: bar_label(s),
                value: (v / max) as f32,
                text: format!("{v:.0}"),
                mark: ride.and(best).map(|b| (b / max) as f32),
                best: ride.is_some() && best.is_some_and(|b| v >= b - 0.05),
            })
        })
        .collect();
    Some(MenuChart { caption, bars })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{RideContext, RideEntry};
    use crate::menu::model::MAX_ITEMS;

    fn info(rides: usize) -> HistoryInfo {
        let mut h = History::new("Anna");
        for n in 0..rides {
            let w = 200.0 + 20.0 * n as f64;
            let a = ocw_fit::Activity {
                start_unix_s: 1_794_211_200 + n as i64 * 86_400,
                records: (0..1201)
                    .map(|i| ocw_fit::Record {
                        elapsed_s: i as f64,
                        distance_m: 8.0 * i as f64,
                        speed_mps: 8.0,
                        power_w: Some(if i < 30 { w + 300.0 } else { w }),
                        heart_rate_bpm: Some(140.0),
                        altitude_m: Some(100.0 + i as f64 * 0.4),
                        ..Default::default()
                    })
                    .collect(),
                product_name: String::new(),
                software_version: 1,
                laps: Vec::new(),
                markers: Vec::new(),
            };
            h.add(RideEntry::from_activity(
                &a,
                RideContext {
                    id: format!("r{n}"),
                    world: "Black Forest".into(),
                    route: "Free ride".into(),
                    hearts: Some(12),
                    fit: Some(format!("r{n}.fit")),
                    ..Default::default()
                },
            ));
        }
        HistoryInfo {
            exportable: h.rides.iter().map(|r| r.id.clone()).collect(),
            history: h,
            now_unix: 1_794_211_200 + 3 * 86_400,
            importable: Some(2),
            ..Default::default()
        }
    }

    /// A ride's photos (spec §2.11, §2.8.1 ST3): a row with their number
    /// (and those in Strava recordings) that opens their folder; none
    /// without photos.
    #[test]
    fn a_rides_photos_open_their_folder() {
        let mut i = info(2);
        let page = Page::HistoryRide(0);
        assert!(!rows(page, &i).iter().any(|r| r.label.starts_with("Photos")));
        let id = i.history.nth_newest(0).unwrap().id.clone();
        i.photos.insert(
            id,
            RidePhotos {
                files: vec![
                    ("photo-1.jpg".into(), 300.0),
                    ("photo-2.jpg".into(), 1700.0),
                ],
                in_strava: 1,
                recordings: 1,
            },
        );
        let rows = rows(page, &i);
        let row = rows.iter().find(|r| r.label.starts_with("Photos")).unwrap();
        assert_eq!(
            row.value.as_deref(),
            Some("2 photos, 1 in 1 Strava recording")
        );
        assert_eq!(
            row.confirm,
            Some(Activate::Command("history.photos 1".into()))
        );
        assert!(rows.len() < MAX_ITEMS);
    }

    /// The simple Strava upload (spec §2.8.1): without the API a ride's
    /// page has the upload in the browser and the mark; with the API the
    /// automatic upload's line and the mark; the steps show after the
    /// upload, with the hint until the ride is marked.
    #[test]
    fn a_rides_strava_rows_with_and_without_the_api() {
        let mut i = info(2);
        let page = Page::HistoryRide(1);
        let id = i.history.nth_newest(1).unwrap().id.clone();
        let r = rows(page, &i);
        let up = r.iter().find(|r| r.label == UPLOAD_LABEL).unwrap();
        assert_eq!(
            up.confirm,
            Some(Activate::Command("strava.upload-manual 2".into()))
        );
        assert_eq!(up.value, None);
        let mark = r.iter().find(|r| r.label == MARK_LABEL).unwrap();
        assert_eq!(mark.value.as_deref(), Some("no"));
        assert_eq!(
            mark.confirm,
            Some(Activate::Command("strava.mark-uploaded 2 toggle".into()))
        );
        assert_eq!(mark.left.as_deref(), Some("strava.mark-uploaded 2 off"));
        assert_eq!(mark.right.as_deref(), Some("strava.mark-uploaded 2 on"));
        assert!(r.len() < MAX_ITEMS);
        // A ride with Strava recordings uploads those.
        i.recordings.insert(id.clone(), 2);
        let r = rows(page, &i);
        assert_eq!(
            r.iter()
                .find(|r| r.label == UPLOAD_LABEL)
                .unwrap()
                .value
                .as_deref(),
            Some("your 2 Strava recordings")
        );
        // The FIT file not kept: dimmed.
        i.exportable.remove(&id);
        let r = rows(page, &i);
        let up = r.iter().find(|r| r.label == UPLOAD_LABEL).unwrap();
        assert_eq!(up.confirm, Some(Activate::Disabled));
        assert_eq!(up.value.as_deref(), Some("FIT file not kept"));
        // The steps after the upload, the hint until it is marked.
        assert_eq!(text(page, &i).len(), 2);
        i.upload = Some((id.clone(), "Strava opens in your browser: …".into()));
        let t = text(page, &i);
        assert_eq!(t[0], "Strava opens in your browser: …");
        assert_eq!(t[1], MARK_HINT);
        assert_eq!(t.len(), 4);
        assert_eq!(chart(page, &i), None, "the steps take the chart's room");
        assert!(chart(Page::HistoryRide(0), &i).is_some());
        assert_eq!(text(Page::HistoryRide(0), &i).len(), 2, "another ride");
        i.history.ride_mut(&id).unwrap().strava_marked = true;
        assert_eq!(text(page, &i).len(), 3);
        // With the API (the advanced way): no browser row.
        i.strava_api = true;
        let r = rows(page, &i);
        assert!(!r.iter().any(|r| r.label == UPLOAD_LABEL));
        let line = r.iter().find(|r| r.label == "Strava upload").unwrap();
        assert_eq!(line.value.as_deref(), Some("skipped (marked as uploaded)"));
        assert_eq!(
            r.iter()
                .find(|r| r.label == MARK_LABEL)
                .unwrap()
                .value
                .as_deref(),
            Some("yes")
        );
        let other = rows(Page::HistoryRide(0), &i);
        assert_eq!(
            other
                .iter()
                .find(|r| r.label == "Strava upload")
                .unwrap()
                .value
                .as_deref(),
            Some("automatic (your API app)")
        );
    }

    #[test]
    fn the_top_page_sums_up_and_opens_the_lists() {
        let i = info(3);
        let r = rows(Page::History, &i);
        assert_eq!(r.len(), 5);
        assert!(r.len() < MAX_ITEMS);
        assert_eq!(r[0].confirm, Some(Activate::Open(Page::HistoryRides)));
        assert_eq!(r[0].value.as_deref(), Some("3"));
        assert_eq!(
            r[1].value.as_deref(),
            Some("3 rides, 28.8 km, 1:00:00, 1440 m")
        );
        assert_eq!(r[3].value.as_deref(), Some("20 min: 247 W"));
        assert_eq!(
            r[4].confirm,
            Some(Activate::Command("history.import".into()))
        );
        // Nothing yet; no import in the browser.
        let mut empty = info(0);
        empty.importable = None;
        let r = rows(Page::History, &empty);
        assert_eq!(r.len(), 4);
        assert_eq!(r[0].confirm, None);
        assert_eq!(r[3].value.as_deref(), Some("none yet"));
    }

    #[test]
    fn rides_are_listed_newest_first_with_their_page() {
        let i = info(3);
        let r = rows(Page::HistoryRides, &i);
        assert_eq!(r[0].label, "Wed 11 Nov 2026 08:00");
        assert_eq!(r[0].value.as_deref(), Some("9.6 km, Black Forest"));
        assert_eq!(r[0].confirm, Some(Activate::Open(Page::HistoryRide(0))));
        let ride = rows(Page::HistoryRide(0), &i);
        assert_eq!(
            ride[0].confirm,
            Some(Activate::Open(Page::HistoryClimbs(0)))
        );
        assert_eq!(
            ride[1].confirm,
            Some(Activate::Command("history.export 1".into()))
        );
        assert!(
            rows(Page::HistoryRide(3), &i).is_empty(),
            "gone: back to the list"
        );
        // Not kept: dimmed.
        let mut j = info(1);
        j.exportable.clear();
        assert_eq!(
            rows(Page::HistoryRide(0), &j)[1].confirm,
            Some(Activate::Disabled)
        );
        assert!(status(Page::HistoryRide(0), &i).ends_with("Black Forest - Free ride"));
        let t = text(Page::HistoryRide(0), &i);
        assert_eq!(
            t[0],
            "9.6 km in 20:00 (moving 20:00), 480 m up, 297 kJ, 12 hearts."
        );
        assert!(
            t[1].starts_with("Power 24") && t[1].contains(", NP "),
            "{}",
            t[1]
        );
        let climbs = rows(Page::HistoryClimbs(0), &i);
        assert_eq!(climbs.len(), 1);
        assert!(climbs[0].value.as_ref().unwrap().contains("480 m at 5.0 %"));
    }

    #[test]
    fn totals_and_bests_pages() {
        let i = info(3);
        let w = rows(Page::HistoryWeeks, &i);
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].label, "Week of 9 Nov 2026");
        let b = rows(Page::HistoryBests, &i);
        assert_eq!(b.len(), 4);
        assert_eq!(b[0].value.as_deref(), Some("540 W, 11 Nov 2026"));
        assert!(status(Page::HistoryBests, &i).starts_with("3 rides; longest 9.6 km"));
        assert_eq!(rows(Page::HistoryWeeks, &info(0))[0].label, "No rides yet");
    }

    #[test]
    fn the_chart_compares_the_ride_with_the_bests() {
        let i = info(3);
        // The newest is the best: highlighted, its bars reach the top.
        let c = chart(Page::HistoryRide(0), &i).unwrap();
        assert_eq!(c.bars.len(), 8, "5 s to 20 min");
        assert_eq!(c.bars[0].label, "5s");
        assert_eq!(c.bars[7].label, "20m");
        assert_eq!(c.bars[0].value, 1.0);
        assert!(c.bars.iter().all(|b| b.best));
        // An older ride: lower bars, the best as lines above them.
        let c = chart(Page::HistoryRide(2), &i).unwrap();
        assert!(c.bars.iter().all(|b| !b.best && b.mark.unwrap() > b.value));
        let all = chart(Page::HistoryBests, &i).unwrap();
        assert!(all.bars.iter().all(|b| b.mark.is_none()));
        assert_eq!(chart(Page::HistoryBests, &info(0)), None);
        assert_eq!(chart(Page::History, &i), None);
    }
}
