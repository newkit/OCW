//! The training history's pages (spec §14 Phase 7; Navi → Training
//! history), Bevy-free: rows, the description of a ride, the status line
//! and the power-curve chart, from a [`HistoryInfo`] the app fills
//! (`ocw_ui::history`). Each page keeps the ten-foot rule (six rows and
//! Back, longer lists paged).

use std::collections::BTreeSet;

use ocw_render::menu::{ChartBar, MenuChart, MenuMap};

use super::pages::{Activate, Page, Row, info_row, open_row, short_label};
use crate::history::{
    Bests, CURVE_S, History, Period, RideEntry, distance_label, duration_label, height_label, hms,
    start_label, total_summary,
};

/// The ride page's row of the export and the delete (spec 14 Phase 7,
/// 7A), and its value.
pub const FILE_LABEL: &str = "Export or delete";
const FILE_VALUE: &str = "the FIT file, the ride";
/// The ride page's delete row.
pub const DELETE_LABEL: &str = "Delete this ride";
/// The map's caption over the picture.
pub const MAP_CAPTION: &str = "Track";

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
    /// Where the FIT files are kept, if that needs saying (the browser
    /// without OPFS: only while the page is open; spec 14 Phase 7, 7A).
    pub kept_note: Option<String>,
    /// The ride photographer's photos of each ride (spec §2.11; by ride
    /// id), and how many of them lie in a Strava recording (§2.8.1 ST3).
    pub photos: std::collections::BTreeMap<String, RidePhotos>,
    /// The automatic Strava upload through the rider's own API app is set
    /// up (`strava.toml`, the advanced way, spec §2.8.1); without it a
    /// ride goes to Strava by the simple upload in the browser.
    pub strava_api: bool,
    /// The automatic upload of the Strava recordings is off (setting
    /// `strava.auto_upload`, spec 2.8.1 Q6): they wait for the rider's
    /// *Upload now* on the ride's page.
    pub strava_auto_off: bool,
    /// The Strava recordings (S … S) within each ride (by ride id): only
    /// those go to Strava, never the whole ride (spec 2.8.1 Q6); a ride
    /// without one has no upload.
    pub recordings: std::collections::BTreeMap<String, usize>,
    /// The last simple upload: the ride's id and its steps (shown on the
    /// ride's page).
    pub upload: Option<(String, String)>,
    /// Each ride's Strava state as the API sync knows it (by ride id;
    /// the desktop: its Strava recordings', else its own file's; spec
    /// §2.8.1 *Recording without Strava set up, uploaded later*). A ride
    /// without one: automatic with the API, else in the browser.
    pub strava: std::collections::BTreeMap<String, StravaState>,
    /// The open question: upload this many waiting recordings?
    pub strava_question: Option<usize>,
}

/// A ride's Strava state on its page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StravaState {
    /// Recorded while Strava was not set up: uploaded once it is.
    Waiting,
    Uploading,
    /// On Strava (the activity, if Strava said which).
    OnStrava {
        activity_id: Option<u64>,
    },
    /// Failed or rejected: retried later, or now from the page.
    Failed {
        error: String,
    },
}

impl StravaState {
    /// The ride's state from its files' states: a failure first, then an
    /// upload running, a waiting one, all on Strava (the activity when
    /// there is one); `None` when no file has a state.
    pub fn of_files(states: &[Option<StravaState>]) -> Option<StravaState> {
        let known: Vec<&StravaState> = states.iter().flatten().collect();
        if known.is_empty() {
            return None;
        }
        let find = |f: fn(&StravaState) -> bool| known.iter().find(|s| f(s)).copied().cloned();
        find(|s| matches!(s, StravaState::Failed { .. }))
            .or_else(|| find(|s| matches!(s, StravaState::Uploading)))
            .or_else(|| find(|s| matches!(s, StravaState::Waiting)))
            .or_else(|| {
                // On Strava only when every file is.
                (known.len() == states.len()).then(|| {
                    let ids: Vec<u64> = known
                        .iter()
                        .filter_map(|s| match s {
                            StravaState::OnStrava { activity_id } => *activity_id,
                            _ => None,
                        })
                        .collect();
                    StravaState::OnStrava {
                        activity_id: ids.first().copied().filter(|_| ids.len() == 1),
                    }
                })
            })
    }
}

/// The ride page's Strava line without the API set-up for a waiting
/// recording.
pub const WAITING_LABEL: &str = "waiting for Strava set-up";
/// … in the browser (no API upload there) and on the desktop without it.
pub const IN_BROWSER_LABEL: &str = "upload in your browser";
/// … with the API and the automatic upload off (spec 2.8.1 Q6): the
/// row uploads the ride's recordings now.
pub const UPLOAD_NOW_LABEL: &str = "not uploaded – upload now";
/// The label of a ride page's Strava state row.
pub const STRAVA_ROW: &str = "Strava upload";

/// A Strava activity's page.
pub fn activity_url(id: u64) -> String {
    format!("https://www.strava.com/activities/{id}")
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
            let mut out = vec![laps_and_climbs_row(k, r)];
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
            // Export and delete (spec 14 Phase 7, 7A): one row to their
            // page, so the ride page keeps its rows (and its numbers, chart
            // and map fit 16:9).
            out.insert(
                1,
                open_row(
                    FILE_LABEL,
                    Some(if info.exportable.contains(&r.id) {
                        FILE_VALUE.into()
                    } else {
                        "FIT file not kept".into()
                    }),
                    Page::HistoryRideFile(k),
                ),
            );
            out
        }
        Page::HistoryRideFile(k) => match h.nth_newest(k) {
            Some(r) => vec![export_row(k, r, info), delete_row(k)],
            None => Vec::new(),
        },
        Page::HistoryClimbs(k) => {
            let Some(r) = h.nth_newest(k) else {
                return Vec::new();
            };
            // The laps first (spec 14 Phase 7, 7A), then the climbs.
            let laps = r.laps.iter().enumerate().map(|(n, l)| {
                // One line at 16:9 (no speed: distance and time say it).
                let mut v = format!(
                    "{}, {}",
                    distance_label(l.distance_m, imp),
                    hms(l.elapsed_s)
                );
                if let Some(p) = l.avg_power_w {
                    v.push_str(&format!(", {p:.0} W"));
                }
                if let Some(b) = l.avg_hr_bpm {
                    v.push_str(&format!(", {b:.0} bpm"));
                }
                info_row(&format!("Lap {}", n + 1), Some(v))
            });
            let rows: Vec<Row> = laps
                .chain(r.climbs.iter().map(|c| {
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
                }))
                .collect();
            if rows.is_empty() {
                vec![info_row("No laps or climbs in this ride", None)]
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
        Page::HistoryWaiting => {
            let rows: Vec<Row> = h
                .rides
                .iter()
                .rev()
                .enumerate()
                .filter(|(_, r)| {
                    !r.strava_marked && info.strava.get(&r.id) == Some(&StravaState::Waiting)
                })
                .map(|(k, r)| {
                    let m = info.recordings.get(&r.id).copied().unwrap_or(1);
                    open_row(
                        &start_label(r.start_unix_s, info.offset_s),
                        Some(format!(
                            "{m} recording{}, {}",
                            if m == 1 { "" } else { "s" },
                            short_label(
                                if r.world.is_empty() {
                                    &r.route
                                } else {
                                    &r.world
                                },
                                16
                            )
                        )),
                        Page::HistoryRide(k),
                    )
                })
                .collect();
            if rows.is_empty() {
                vec![info_row("No recording is waiting", None)]
            } else {
                rows
            }
        }
        Page::StravaWaiting => match info.strava_question {
            Some(n) => vec![
                Row {
                    label: format!("Yes, upload all {n}"),
                    value: None,
                    left: None,
                    right: None,
                    confirm: Some(Activate::Run("strava.waiting yes".into())),
                },
                Row {
                    label: "Pick which".into(),
                    value: None,
                    left: None,
                    right: None,
                    confirm: Some(Activate::RunOpen(
                        "strava.waiting pick".into(),
                        Page::HistoryWaiting,
                    )),
                },
                Row {
                    label: "No, leave them waiting".into(),
                    value: None,
                    left: None,
                    right: None,
                    confirm: Some(Activate::Run("strava.waiting no".into())),
                },
            ],
            None => vec![info_row("Nothing to decide", None)],
        },
        _ => Vec::new(),
    }
}

/// The ride page's first row: its climbs, with its laps when it has some
/// (spec 14 Phase 7, 7A); both on one page.
fn laps_and_climbs_row(k: usize, r: &RideEntry) -> Row {
    let plural = |n: usize, one: &str| format!("{n} {one}{}", if n == 1 { "" } else { "s" });
    match (r.laps.len(), r.climbs.len()) {
        (0, 0) => info_row("Climbs", Some("none".into())),
        (0, c) => open_row("Climbs", Some(c.to_string()), Page::HistoryClimbs(k)),
        (l, 0) => open_row(
            "Laps and climbs",
            Some(format!("{}, no climbs", plural(l, "lap"))),
            Page::HistoryClimbs(k),
        ),
        (l, c) => open_row(
            "Laps and climbs",
            Some(format!("{}, {}", plural(l, "lap"), plural(c, "climb"))),
            Page::HistoryClimbs(k),
        ),
    }
}

/// The export of the ride's FIT file (dimmed: not kept).
fn export_row(k: usize, r: &RideEntry, info: &HistoryInfo) -> Row {
    if info.exportable.contains(&r.id) {
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
    }
}

/// Deleting the ride (spec 14 Phase 7, 7A): asks first.
fn delete_row(k: usize) -> Row {
    Row {
        label: DELETE_LABEL.into(),
        value: None,
        left: None,
        right: None,
        confirm: Some(Activate::AskDelete(format!("history.delete {}", k + 1))),
    }
}

/// The ride a `history.delete N` line asks about (0 = the newest).
pub fn delete_index(line: &str) -> Option<usize> {
    line.strip_prefix("history.delete ")?
        .split_whitespace()
        .next()?
        .parse::<usize>()
        .ok()?
        .checked_sub(1)
}

/// The confirm page's text for ride `k` (spec 14 Phase 7, 7A): which
/// ride; what goes – its entry and its FIT file –, and what stays: its
/// Strava recordings (S … S) and photos in the activities folder, a
/// recording waiting for Strava still uploaded (nothing is lost without
/// a word).
pub fn delete_text(k: usize, info: &HistoryInfo) -> Vec<String> {
    let Some(r) = info.history.nth_newest(k) else {
        return Vec::new();
    };
    let place = if r.world.is_empty() {
        &r.route
    } else {
        &r.world
    };
    // The date and place are the page's status line.
    let mut out = vec![format!(
        "The ride ({}{}) leaves the training history and its FIT file is deleted. This cannot be undone.",
        distance_label(r.distance_m, info.imperial),
        if place.is_empty() {
            String::new()
        } else {
            format!(" in {}", short_label(place, 30))
        }
    )];
    let recordings = info.recordings.get(&r.id).copied().unwrap_or(0);
    let photos = info.photos.get(&r.id).map_or(0, |p| p.files.len());
    let mut kept = Vec::new();
    if recordings > 0 {
        kept.push(match recordings {
            1 => "its Strava recording".to_string(),
            n => format!("its {n} Strava recordings"),
        });
    }
    if photos > 0 {
        kept.push(match photos {
            1 => "its photo".to_string(),
            n => format!("its {n} photos"),
        });
    }
    if !kept.is_empty() {
        let mut s = format!("Kept: {} in your activities folder.", kept.join(" and "));
        let waiting = !r.strava_marked
            && matches!(
                info.strava.get(&r.id),
                Some(StravaState::Waiting | StravaState::Failed { .. })
            );
        let (it, verb) = if recordings == 1 {
            ("it", "is")
        } else {
            ("they", "are")
        };
        if recordings > 0 && waiting {
            s.push_str(&if info.strava_api {
                format!(" Not on Strava yet: {it} {verb} still uploaded.")
            } else {
                format!(
                    " Not on Strava yet: {it} still wait{} there and {verb} uploaded once Strava is set up.",
                    if recordings == 1 { "s" } else { "" }
                )
            });
        } else if recordings > 0 && !r.strava_marked && !info.strava.contains_key(&r.id) {
            s.push_str(&format!(
                " Upload {} from there if you have not yet.",
                if recordings == 1 { "it" } else { "them" }
            ));
        }
        out.push(s);
    }
    out
}

/// The ride's track as a small map beside the power curve (spec 14
/// Phase 7, 7A; drawn with `ocw_map::ride`); none without a track, or
/// while the upload's steps take the chart's room.
pub fn map(page: Page, info: &HistoryInfo) -> Option<MenuMap> {
    let Page::HistoryRide(k) = page else {
        return None;
    };
    let r = info.history.nth_newest(k)?;
    if r.track.len() < 2 || upload_steps_for(info, &r.id).is_some() {
        return None;
    }
    Some(MenuMap {
        caption: MAP_CAPTION.into(),
        track: r.track.clone(),
        imperial: info.imperial,
    })
}

/// A ride's Strava rows (spec 2.8.1 *Simple upload*, *Recording
/// without Strava set up, uploaded later*, Q6). Only the ride's Strava
/// recordings (S … S) go to Strava, never the whole ride: a ride without
/// one has the single line *no Strava recording – press S while riding*
/// (or, for a whole ride uploaded before v5.05, *on Strava*). A ride with
/// recordings shows their state – *waiting for Strava set-up* /
/// *uploading* / *on Strava* (opens the activity) / *failed – retry*, or
/// the automatic upload's line, or *upload now* with the automatic upload
/// off – then without the API the upload in the browser, and the mark,
/// which the rider sets after uploading by hand (the app cannot know) and
/// the API sync respects.
fn strava_rows(k: usize, r: &crate::history::RideEntry, info: &HistoryInfo) -> Vec<Row> {
    let n = k + 1;
    let recordings = info.recordings.get(&r.id).copied().unwrap_or(0);
    let state = info.strava.get(&r.id);
    if recordings == 0 {
        // Whole rides never go to Strava (Q6); one uploaded earlier
        // keeps its line.
        return vec![match state {
            Some(StravaState::OnStrava {
                activity_id: Some(id),
            }) => Row {
                label: STRAVA_ROW.into(),
                value: Some(format!("on Strava: strava.com/activities/{id}")),
                left: None,
                right: None,
                confirm: Some(Activate::Command(format!("strava.open {n}"))),
            },
            Some(StravaState::OnStrava { activity_id: None }) => {
                info_row(STRAVA_ROW, Some("on Strava".into()))
            }
            _ => info_row(STRAVA_ROW, Some(crate::strava::NO_RECORDING.into())),
        }];
    }
    let mut out = Vec::new();
    let api = info.strava_api;
    let auto = !info.strava_auto_off;
    let (value, confirm) = match (r.strava_marked, state) {
        (true, _) => ("skipped (marked as uploaded)".to_string(), None),
        (false, Some(StravaState::Waiting)) if api => (
            "waiting – upload now".to_string(),
            Some(Activate::Command(format!(
                "strava.sync --waiting --ride {n}"
            ))),
        ),
        // The Strava settings page says how to set it up.
        (false, Some(StravaState::Waiting)) => (
            if auto {
                WAITING_LABEL.to_string()
            } else {
                IN_BROWSER_LABEL.to_string()
            },
            Some(Activate::Open(Page::Strava)),
        ),
        (false, Some(StravaState::Uploading)) => ("uploading".to_string(), None),
        (false, Some(StravaState::OnStrava { activity_id })) => match activity_id {
            Some(id) => (
                format!("on Strava: strava.com/activities/{id}"),
                Some(Activate::Command(format!("strava.open {n}"))),
            ),
            None => ("on Strava".to_string(), None),
        },
        (false, Some(StravaState::Failed { .. })) => (
            "failed – retry".to_string(),
            Some(if api {
                Activate::Command("strava.sync --retry".into())
            } else {
                Activate::Disabled
            }),
        ),
        (false, None) if api && auto => ("automatic (your API app)".to_string(), None),
        // The automatic upload is off: the rider uploads it (Q6).
        (false, None) if api => (
            UPLOAD_NOW_LABEL.to_string(),
            Some(Activate::Command(format!("strava.sync --ride {n}"))),
        ),
        (false, None) => (IN_BROWSER_LABEL.to_string(), None),
    };
    out.push(Row {
        label: STRAVA_ROW.into(),
        value: Some(value),
        left: None,
        right: None,
        confirm,
    });
    if !api {
        out.push(Row {
            label: UPLOAD_LABEL.into(),
            value: Some(match recordings {
                1 => "your Strava recording".into(),
                m => format!("your {m} Strava recordings"),
            }),
            left: None,
            right: None,
            confirm: Some(Activate::Command(format!("strava.upload-manual {n}"))),
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
        Page::History => info
            .read_only
            .clone()
            .or_else(|| info.kept_note.clone())
            .unwrap_or_default(),
        Page::StravaWaiting => info
            .strava_question
            .map(|n| format!("Upload {n} waiting recordings to Strava?"))
            .unwrap_or_default(),
        Page::HistoryRide(k) | Page::HistoryClimbs(k) | Page::HistoryRideFile(k) => {
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
    if page == Page::StravaWaiting {
        return match info.strava_question {
            Some(n) => vec![format!(
                "{n} Strava recordings made before Strava was set up are waiting in your activities folder. Yes uploads them now, oldest first; Pick shows them to choose; No leaves them waiting (later: strava.sync --waiting)."
            )],
            None => Vec::new(),
        };
    }
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
    // Why the Strava upload failed (spec §2.8.1).
    if let (false, Some(StravaState::Failed { error })) = (r.strava_marked, info.strava.get(&r.id))
    {
        second.push(format!("Strava upload failed: {}", short_label(error, 40)));
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
            // Beside the map (7A) the caption is shorter: one line.
            (
                Some(r),
                if r.track.len() >= 2 {
                    "Power curve (W), lines: your best".to_string()
                } else {
                    "Power curve (W): bars this ride, lines your best".to_string()
                },
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
                devices: Vec::new(),
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

    /// The simple Strava upload (spec 2.8.1, Q6): a ride without Strava
    /// recordings has no upload, only the line saying how to make one
    /// (an older whole ride on Strava keeps its line); with recordings,
    /// without the API its page has the upload in the browser (the
    /// recordings, never the whole ride) and the mark; with the API the
    /// automatic upload's line – or *upload now* with it off – and the
    /// mark; the steps show after the upload, with the hint until the
    /// ride is marked.
    #[test]
    fn a_rides_strava_rows_with_and_without_the_api() {
        let mut i = info(2);
        let page = Page::HistoryRide(1);
        let id = i.history.nth_newest(1).unwrap().id.clone();
        // No Strava recording: one line, no upload, no mark.
        for api in [false, true] {
            i.strava_api = api;
            let r = rows(page, &i);
            assert!(
                !r.iter()
                    .any(|r| r.label == UPLOAD_LABEL || r.label == MARK_LABEL)
            );
            let line = r.iter().find(|r| r.label == STRAVA_ROW).unwrap();
            assert_eq!(
                line.value.as_deref(),
                Some("no Strava recording – press S while riding")
            );
            assert_eq!(line.confirm, None);
        }
        // A whole ride uploaded before (v5.05) stays on Strava.
        i.strava.insert(
            id.clone(),
            StravaState::OnStrava {
                activity_id: Some(5),
            },
        );
        assert_eq!(
            strava_value(page, &i),
            (
                Some("on Strava: strava.com/activities/5".into()),
                Some(Activate::Command("strava.open 2".into()))
            )
        );
        i.strava.clear();
        i.strava_api = false;
        // One Strava recording: the browser's upload sends it.
        i.recordings.insert(id.clone(), 1);
        let r = rows(page, &i);
        let up = r.iter().find(|r| r.label == UPLOAD_LABEL).unwrap();
        assert_eq!(
            up.confirm,
            Some(Activate::Command("strava.upload-manual 2".into()))
        );
        assert_eq!(up.value.as_deref(), Some("your Strava recording"));
        let mark = r.iter().find(|r| r.label == MARK_LABEL).unwrap();
        assert_eq!(mark.value.as_deref(), Some("no"));
        assert_eq!(
            mark.confirm,
            Some(Activate::Command("strava.mark-uploaded 2 toggle".into()))
        );
        assert_eq!(mark.left.as_deref(), Some("strava.mark-uploaded 2 off"));
        assert_eq!(mark.right.as_deref(), Some("strava.mark-uploaded 2 on"));
        assert!(r.len() < MAX_ITEMS);
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
        // The ride's own FIT file does not matter: the recordings go.
        i.exportable.remove(&id);
        let up = rows(page, &i)
            .into_iter()
            .find(|r| r.label == UPLOAD_LABEL)
            .unwrap();
        assert_eq!(
            up.confirm,
            Some(Activate::Command("strava.upload-manual 2".into()))
        );
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
        let newest = i.history.nth_newest(0).unwrap().id.clone();
        i.recordings.insert(newest, 1);
        assert_eq!(
            strava_value(Page::HistoryRide(0), &i),
            (Some("automatic (your API app)".into()), None)
        );
        // The automatic upload off (setting strava.auto_upload): upload now.
        i.strava_auto_off = true;
        assert_eq!(
            strava_value(Page::HistoryRide(0), &i),
            (
                Some(UPLOAD_NOW_LABEL.into()),
                Some(Activate::Command("strava.sync --ride 1".into()))
            )
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
            Some(Activate::Open(Page::HistoryRideFile(0)))
        );
        assert_eq!(
            rows(Page::HistoryRideFile(0), &i)[0].confirm,
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
            rows(Page::HistoryRide(0), &j)[1].value.as_deref(),
            Some("FIT file not kept")
        );
        assert_eq!(
            rows(Page::HistoryRideFile(0), &j)[0].confirm,
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

    fn strava_value(page: Page, i: &HistoryInfo) -> (Option<String>, Option<Activate>) {
        let r = rows(page, i);
        let row = r.iter().find(|r| r.label == STRAVA_ROW).expect("the row");
        (row.value.clone(), row.confirm.clone())
    }

    /// A ride's Strava state (spec §2.8.1 *Recording without Strava set
    /// up, uploaded later*): waiting for the set-up (opens the Strava
    /// page), uploading, on Strava with the activity's link, failed –
    /// retry; with the API a waiting one uploads from its page; the
    /// browser (no state, no API) uploads in the browser; a marked ride
    /// is skipped whatever its state.
    #[test]
    fn a_rides_strava_state_on_its_page() {
        let mut i = info(2);
        let page = Page::HistoryRide(0);
        let id = i.history.nth_newest(0).unwrap().id.clone();
        i.recordings.insert(id.clone(), 1);
        // The browser, or the desktop without the API.
        assert_eq!(
            strava_value(page, &i),
            (Some(IN_BROWSER_LABEL.into()), None)
        );
        i.strava.insert(id.clone(), StravaState::Waiting);
        assert_eq!(
            strava_value(page, &i),
            (
                Some("waiting for Strava set-up".into()),
                Some(Activate::Open(Page::Strava))
            )
        );
        // The browser's upload stays beside it.
        assert!(rows(page, &i).iter().any(|r| r.label == UPLOAD_LABEL));
        // The automatic upload off: it waits for the rider, not the set-up.
        i.strava_auto_off = true;
        assert_eq!(strava_value(page, &i).0.as_deref(), Some(IN_BROWSER_LABEL));
        i.strava_auto_off = false;
        i.strava_api = true;
        assert_eq!(
            strava_value(page, &i),
            (
                Some("waiting – upload now".into()),
                Some(Activate::Command("strava.sync --waiting --ride 1".into()))
            )
        );
        assert!(!rows(page, &i).iter().any(|r| r.label == UPLOAD_LABEL));
        i.strava.insert(id.clone(), StravaState::Uploading);
        assert_eq!(strava_value(page, &i), (Some("uploading".into()), None));
        i.strava.insert(
            id.clone(),
            StravaState::OnStrava {
                activity_id: Some(1001),
            },
        );
        assert_eq!(
            strava_value(page, &i),
            (
                Some("on Strava: strava.com/activities/1001".into()),
                Some(Activate::Command("strava.open 1".into()))
            )
        );
        assert_eq!(activity_url(1001), "https://www.strava.com/activities/1001");
        i.strava
            .insert(id.clone(), StravaState::OnStrava { activity_id: None });
        assert_eq!(strava_value(page, &i), (Some("on Strava".into()), None));
        i.strava.insert(
            id.clone(),
            StravaState::Failed {
                error: "Strava rate limit reached".into(),
            },
        );
        assert_eq!(
            strava_value(page, &i),
            (
                Some("failed – retry".into()),
                Some(Activate::Command("strava.sync --retry".into()))
            )
        );
        assert!(
            text(page, &i)
                .last()
                .unwrap()
                .contains("Strava upload failed: Strava rate limit reached"),
            "{:?}",
            text(page, &i)
        );
        i.history.ride_mut(&id).unwrap().strava_marked = true;
        assert_eq!(
            strava_value(page, &i).0.as_deref(),
            Some("skipped (marked as uploaded)")
        );
        assert!(rows(page, &i).len() < MAX_ITEMS);
    }

    #[test]
    fn a_rides_state_from_its_recordings() {
        use StravaState::*;
        let on = |id| OnStrava {
            activity_id: Some(id),
        };
        assert_eq!(StravaState::of_files(&[None, None]), None);
        assert_eq!(StravaState::of_files(&[Some(on(1))]), Some(on(1)));
        assert_eq!(
            StravaState::of_files(&[Some(on(1)), Some(on(2))]),
            Some(OnStrava { activity_id: None }),
            "two activities: no single link"
        );
        assert_eq!(StravaState::of_files(&[Some(on(1)), None]), None);
        assert_eq!(
            StravaState::of_files(&[Some(on(1)), Some(Waiting)]),
            Some(Waiting)
        );
        assert_eq!(
            StravaState::of_files(&[Some(Waiting), Some(Uploading)]),
            Some(Uploading)
        );
        let failed = Failed { error: "x".into() };
        assert_eq!(
            StravaState::of_files(&[Some(Uploading), Some(failed.clone())]),
            Some(failed)
        );
    }

    /// The question before uploading many waiting recordings: Yes / Pick
    /// / No with its text; Pick lists the rides with waiting recordings,
    /// each opening its page.
    #[test]
    fn the_question_and_the_waiting_list() {
        let mut i = info(3);
        let q = Page::StravaWaiting;
        assert_eq!(rows(q, &i)[0].label, "Nothing to decide");
        i.strava_question = Some(12);
        let r = rows(q, &i);
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].label, "Yes, upload all 12");
        assert_eq!(
            r[0].confirm,
            Some(Activate::Run("strava.waiting yes".into()))
        );
        assert_eq!(
            r[1].confirm,
            Some(Activate::RunOpen(
                "strava.waiting pick".into(),
                Page::HistoryWaiting
            ))
        );
        assert_eq!(
            r[2].confirm,
            Some(Activate::Run("strava.waiting no".into()))
        );
        assert_eq!(status(q, &i), "Upload 12 waiting recordings to Strava?");
        assert!(text(q, &i)[0].starts_with("12 Strava recordings made before Strava was set up"));
        // Pick: the waiting rides only, newest first, each opens its page.
        assert_eq!(
            rows(Page::HistoryWaiting, &i)[0].label,
            "No recording is waiting"
        );
        let newest = i.history.nth_newest(0).unwrap().id.clone();
        let oldest = i.history.nth_newest(2).unwrap().id.clone();
        i.strava.insert(newest.clone(), StravaState::Waiting);
        i.strava.insert(oldest.clone(), StravaState::Waiting);
        i.strava.insert(
            i.history.nth_newest(1).unwrap().id.clone(),
            StravaState::OnStrava { activity_id: None },
        );
        i.recordings.insert(newest, 2);
        let w = rows(Page::HistoryWaiting, &i);
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].confirm, Some(Activate::Open(Page::HistoryRide(0))));
        assert_eq!(w[0].value.as_deref(), Some("2 recordings, Black Forest"));
        assert_eq!(w[1].confirm, Some(Activate::Open(Page::HistoryRide(2))));
        assert!(Page::HistoryWaiting.is_history_page() && q.is_history_page());
    }

    /// Laps (spec 14 Phase 7, 7A): the ride's first row names them with
    /// its climbs; their page lists the laps, then the climbs.
    #[test]
    fn a_rides_laps_and_climbs() {
        let mut i = info(2);
        let laps = vec![
            crate::history::Lap {
                start_s: 0.0,
                start_m: 0.0,
                elapsed_s: 600.0,
                distance_m: 4800.0,
                ascent_m: 240.0,
                avg_power_w: Some(231.0),
                avg_hr_bpm: Some(141.0),
            },
            crate::history::Lap {
                start_s: 600.0,
                start_m: 4800.0,
                elapsed_s: 600.0,
                distance_m: 4800.0,
                ascent_m: 240.0,
                avg_power_w: None,
                avg_hr_bpm: None,
            },
        ];
        let id = i.history.nth_newest(0).unwrap().id.clone();
        i.history.ride_mut(&id).unwrap().laps = laps;
        let r = rows(Page::HistoryRide(0), &i);
        assert_eq!(r[0].label, "Laps and climbs");
        assert_eq!(r[0].value.as_deref(), Some("2 laps, 1 climb"));
        assert_eq!(r[0].confirm, Some(Activate::Open(Page::HistoryClimbs(0))));
        let page = rows(Page::HistoryClimbs(0), &i);
        assert_eq!(page.len(), 3);
        assert_eq!(page[0].label, "Lap 1");
        assert_eq!(
            page[0].value.as_deref(),
            Some("4.8 km, 10:00, 231 W, 141 bpm")
        );
        assert_eq!(page[1].value.as_deref(), Some("4.8 km, 10:00"));
        assert!(page[2].label.starts_with("At "));
        i.imperial = true;
        assert!(
            rows(Page::HistoryClimbs(0), &i)[0]
                .value
                .as_ref()
                .unwrap()
                .contains(" mi,")
        );
        // Laps without climbs; neither.
        i.history.ride_mut(&id).unwrap().climbs.clear();
        assert_eq!(
            rows(Page::HistoryRide(0), &i)[0].value.as_deref(),
            Some("2 laps, no climbs")
        );
        i.history.ride_mut(&id).unwrap().laps.clear();
        assert_eq!(rows(Page::HistoryRide(0), &i)[0].confirm, None);
        assert_eq!(
            rows(Page::HistoryClimbs(0), &i)[0].label,
            "No laps or climbs in this ride"
        );
    }

    /// Deleting (7A): the ride page's *Export or delete* opens the
    /// ride's file page with both rows; Delete asks first. The ride page
    /// keeps its rows (within six with photos and the Strava rows).
    #[test]
    fn a_ride_can_be_deleted_from_its_page() {
        let mut i = info(2);
        let r = rows(Page::HistoryRide(1), &i);
        assert_eq!(r[1].label, FILE_LABEL);
        assert_eq!(r[1].value.as_deref(), Some(FILE_VALUE));
        assert!(!r.iter().any(|r| r.label == DELETE_LABEL));
        let file = rows(Page::HistoryRideFile(1), &i);
        assert_eq!(file.len(), 2);
        assert_eq!(
            file[0].confirm,
            Some(Activate::Command("history.export 2".into()))
        );
        assert_eq!(file[1].label, DELETE_LABEL);
        assert_eq!(
            file[1].confirm,
            Some(Activate::AskDelete("history.delete 2".into()))
        );
        assert!(rows(Page::HistoryRideFile(5), &i).is_empty());
        assert!(status(Page::HistoryRideFile(1), &i).contains("Black Forest"));
        assert_eq!(delete_index("history.delete 2"), Some(1));
        assert_eq!(delete_index("history.delete 0"), None);
        assert_eq!(delete_index("history.export 2"), None);
        // Photos and a recording without the API: still six rows at most.
        let id = i.history.nth_newest(1).unwrap().id.clone();
        i.recordings.insert(id.clone(), 1);
        i.photos.insert(
            id,
            RidePhotos {
                files: vec![("photo-1.jpg".into(), 30.0)],
                in_strava: 1,
                recordings: 1,
            },
        );
        for api in [false, true] {
            i.strava_api = api;
            for k in 0..2 {
                assert!(rows(Page::HistoryRide(k), &i).len() < MAX_ITEMS);
            }
        }
        assert!(
            rows(Page::HistoryRide(1), &i)
                .iter()
                .any(|r| r.label == FILE_LABEL)
        );
    }

    /// The confirm page says which ride, that it cannot be undone, and
    /// what stays: the Strava recordings and photos in the activities
    /// folder, a recording waiting for Strava still uploaded.
    #[test]
    fn the_delete_question_says_what_goes_and_what_stays() {
        let mut i = info(2);
        let t = delete_text(0, &i);
        assert_eq!(t.len(), 1);
        assert!(
            t[0].starts_with("The ride (9.6 km in Black Forest) leaves")
                && t[0].contains("cannot be undone"),
            "{}",
            t[0]
        );
        assert!(delete_text(9, &i).is_empty());
        let id = i.history.nth_newest(0).unwrap().id.clone();
        i.recordings.insert(id.clone(), 2);
        i.photos.insert(
            id.clone(),
            RidePhotos {
                files: vec![("a.jpg".into(), 1.0), ("b.jpg".into(), 2.0)],
                in_strava: 2,
                recordings: 2,
            },
        );
        let t = delete_text(0, &i);
        assert_eq!(
            t[1],
            "Kept: its 2 Strava recordings and its 2 photos in your activities folder. Upload them from there if you have not yet."
        );
        i.strava.insert(id.clone(), StravaState::Waiting);
        assert!(delete_text(0, &i)[1].ends_with(
            "Not on Strava yet: they still wait there and are uploaded once Strava is set up."
        ));
        i.strava_api = true;
        assert!(delete_text(0, &i)[1].ends_with("Not on Strava yet: they are still uploaded."));
        i.strava
            .insert(id.clone(), StravaState::OnStrava { activity_id: None });
        assert!(delete_text(0, &i)[1].ends_with("activities folder."));
        i.recordings.clear();
        assert_eq!(
            delete_text(0, &i)[1],
            "Kept: its 2 photos in your activities folder."
        );
    }

    /// The map (7A): the ride's track beside the chart; none without a
    /// track or while the upload's steps take the room.
    #[test]
    fn a_rides_map_comes_from_its_track() {
        let mut i = info(2);
        assert_eq!(map(Page::HistoryRide(0), &i), None);
        let id = i.history.nth_newest(0).unwrap().id.clone();
        let track = vec![[47.1, 9.5], [47.11, 9.51], [47.12, 9.5]];
        i.history.ride_mut(&id).unwrap().track = track.clone();
        let m = map(Page::HistoryRide(0), &i).expect("a map");
        assert_eq!(m.track, track);
        assert_eq!(m.caption, MAP_CAPTION);
        assert!(!m.imperial);
        assert_eq!(map(Page::HistoryClimbs(0), &i), None);
        assert_eq!(map(Page::HistoryRide(1), &i), None);
        i.upload = Some((id, "steps".into()));
        assert_eq!(map(Page::HistoryRide(0), &i), None);
    }

    /// The menu's flow (7A): Delete asks with Cancel focused; Cancel goes
    /// back to the ride; Delete runs the line and goes back past the
    /// ride's pages to the list.
    #[test]
    fn deleting_asks_first_and_returns_to_the_list() {
        use crate::menu::model::{Action, Effect, Menu, Page as MenuPage};
        let i = info(2);
        let rows_of = |p| rows(p, &i);
        let mut m = Menu::default();
        m.apply_with(Action::OpenSub(Page::HistoryRides), rows_of);
        m.apply_with(Action::Click(1), rows_of);
        assert_eq!(m.page(), Some(MenuPage::Sub(Page::HistoryRide(1))));
        m.apply_with(Action::Click(1), rows_of);
        assert_eq!(m.page(), Some(MenuPage::Sub(Page::HistoryRideFile(1))));
        let at = m.labels().iter().position(|l| l == DELETE_LABEL).unwrap();
        assert_eq!(m.apply_with(Action::Click(at), rows_of), Effect::None);
        assert_eq!(m.page(), Some(MenuPage::ConfirmDelete));
        assert_eq!(m.focus(), 1, "Cancel is focused");
        assert_eq!(m.pending(), Some("history.delete 2"));
        // Cancel: back on the ride's file page, nothing run.
        assert_eq!(m.apply_with(Action::Confirm, rows_of), Effect::None);
        assert_eq!(m.page(), Some(MenuPage::Sub(Page::HistoryRideFile(1))));
        assert_eq!(m.pending(), None);
        m.apply_with(Action::Click(at), rows_of);
        let e = m.apply_with(Action::Click(0), rows_of);
        assert_eq!(e, Effect::Command("history.delete 2".into()));
        assert_eq!(m.page(), Some(MenuPage::Sub(Page::HistoryRides)));
    }
}
