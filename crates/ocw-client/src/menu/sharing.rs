//! The desktop's *Sharing* screen (spec 14 Phase 10, 10D.2 *Built (local
//! bridge)*): what it shows while the trainer is shared with a browser,
//! Bevy-free. The app gathers a [`SharingInfo`] from its bridge, [`view`]
//! turns it into the `ocw_render::sharing::SharingView` the screen draws.

use ocw_devices::DeviceCommand;
use ocw_render::sharing::{SharingTile, SharingView};

/// A reading older than this is not shown as live (s): the trainer or the
/// strap went quiet.
pub const STALE_S: f64 = 3.0;
/// The browser counts as gone quiet when not heard for this long (s; its
/// pings come every second, the desktop replaces it after 5 s).
pub const QUIET_S: f64 = 3.0;

/// What the browser last asked of the trainer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SetLoad {
    /// SIM: the road's grade (rise over run), the headwind (m/s).
    Sim { grade: f64, wind_mps: f64 },
    /// ERG: the target power (W).
    Erg { watts: f64 },
}

impl SetLoad {
    /// The load a control command sets (`None`: not one of the browser's).
    pub fn of(cmd: &DeviceCommand) -> Option<Self> {
        match cmd {
            DeviceCommand::SetSimulation(p) => Some(Self::Sim {
                grade: p.grade,
                wind_mps: p.wind_speed_mps,
            }),
            DeviceCommand::SetTargetPower(w) => Some(Self::Erg { watts: *w }),
            _ => None,
        }
    }

    fn text(self) -> String {
        match self {
            // Calm air below 0.5 m/s (no "-0 m/s").
            Self::Sim { grade, wind_mps } if wind_mps.abs() >= 0.5 => {
                format!("{:+.1} % (SIM, wind {wind_mps:+.1} m/s)", grade * 100.0)
            }
            Self::Sim { grade, .. } => format!("{:+.1} % (SIM)", grade * 100.0),
            Self::Erg { watts } => format!("{watts:.0} W (ERG)"),
        }
    }
}

/// The headset's (or the browser's) last report over the bridge (spec 14
/// Phase 14, 14I W4: the beacon).
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    /// Its kind (`xr`: a headset run).
    pub kind: String,
    pub fps: Option<f64>,
    /// The frame time (ms): the headset's work per frame (p50), else the
    /// page's frame interval.
    pub frame_ms: Option<f64>,
}

impl Report {
    /// The numbers of a report's JSON: `xr.fps` and `xr.work_ms_p50` of a
    /// headset run (`web/ocw-xr.js`), else `fps` and `frame_ms_p50` at the
    /// top (the flat page).
    pub fn of(kind: &str, data: &serde_json::Value) -> Self {
        let num = |k: &str| data.pointer(k).and_then(serde_json::Value::as_f64);
        Self {
            kind: kind.to_owned(),
            fps: num("/xr/fps").or_else(|| num("/fps")),
            frame_ms: num("/xr/work_ms_p50").or_else(|| num("/frame_ms_p50")),
        }
    }
}

/// Everything the Sharing screen shows.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SharingInfo {
    /// The paired browser ("Quest Browser", "Safari on iPhone").
    pub browser: String,
    pub power_w: Option<f64>,
    pub cadence_rpm: Option<f64>,
    pub heart_rate_bpm: Option<f64>,
    /// Since the last power reading (s; `None`: none yet).
    pub power_age_s: Option<f64>,
    /// Since the last heart-rate reading (s).
    pub heart_age_s: Option<f64>,
    /// What the browser set, and since when (s).
    pub load: Option<(SetLoad, f64)>,
    /// The connection's round trip (ms).
    pub rtt_ms: Option<f64>,
    /// Since the browser was last heard (s).
    pub heard_s_ago: f64,
    /// Since it paired (s).
    pub paired_s: f64,
    /// The readings relayed to it so far.
    pub relayed: u64,
    /// The last report, and since when (s).
    pub report: Option<(Report, f64)>,
    /// The pairing code (re-pairing).
    pub code: String,
    /// The pads' Confirm is East (setting `controls.pad_confirm east`).
    pub pad_confirm_east: bool,
}

/// "45 s", "2 min 05 s", "1 h 02 min".
pub fn duration(s: f64) -> String {
    let s = s.max(0.0).round() as u64;
    match s {
        0..60 => format!("{s} s"),
        60..3600 => format!("{} min {:02} s", s / 60, s % 60),
        _ => format!("{} h {:02} min", s / 3600, (s % 3600) / 60),
    }
}

fn live(v: Option<f64>, age: Option<f64>) -> String {
    match (v, age) {
        (Some(v), Some(a)) if a <= STALE_S => format!("{v:.0}"),
        _ => "–".into(),
    }
}

/// The screen for `info`.
pub fn view(info: &SharingInfo) -> SharingView {
    let tile = |label: &str, value: String, unit: &str| SharingTile {
        label: label.into(),
        value,
        unit: unit.into(),
    };
    let browser = if info.browser.is_empty() {
        "a browser"
    } else {
        info.browser.as_str()
    };
    let state = if info.heard_s_ago > QUIET_S {
        format!(
            "waiting for the browser (last heard {} ago)",
            duration(info.heard_s_ago)
        )
    } else {
        format!(
            "connected for {}, {} readings sent",
            duration(info.paired_s),
            info.relayed
        )
    };
    let mut rows = vec![
        (
            "Grade set by the browser".to_owned(),
            match info.load {
                Some((l, _)) => l.text(),
                None => "not yet".into(),
            },
        ),
        ("Bridge".to_owned(), state),
        (
            "Round trip".to_owned(),
            match info.rtt_ms {
                Some(ms) if ms.is_finite() => format!("{ms:.1} ms"),
                _ => "–".into(),
            },
        ),
    ];
    if let Some((r, age)) = &info.report {
        let label = if r.kind == "xr" {
            "Last headset report"
        } else {
            "Last browser report"
        };
        let mut parts = Vec::new();
        if let Some(f) = r.fps {
            parts.push(format!("{f:.1} fps"));
        }
        if let Some(ms) = r.frame_ms {
            parts.push(format!("frame time {ms:.1} ms"));
        }
        if parts.is_empty() {
            parts.push(r.kind.clone());
        }
        rows.push((
            label.to_owned(),
            format!("{}, {} ago", parts.join(", "), duration(*age)),
        ));
    }
    rows.push(("Pairing code".to_owned(), info.code.clone()));
    let pad = if info.pad_confirm_east { "B" } else { "A" };
    SharingView {
        open: true,
        title: "Sharing".into(),
        subtitle: format!("Your trainer rides in {browser}"),
        tiles: vec![
            tile("Power", live(info.power_w, info.power_age_s), "W"),
            tile("Cadence", live(info.cadence_rpm, info.power_age_s), "rpm"),
            tile(
                "Heart rate",
                live(info.heart_rate_bpm, info.heart_age_s),
                "bpm",
            ),
        ],
        rows,
        note: format!(
            "Your ride here is paused – no distance, no recording, no hearts: the ride in {browser} is the one that counts. It goes on when the browser leaves or you stop sharing."
        ),
        button: "Stop sharing".into(),
        hint: format!("Enter / {pad} / click: stop sharing   ·   Esc: menu"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> SharingInfo {
        SharingInfo {
            browser: "Quest Browser".into(),
            power_w: Some(182.4),
            cadence_rpm: Some(88.6),
            heart_rate_bpm: Some(131.0),
            power_age_s: Some(0.2),
            heart_age_s: Some(0.9),
            load: Some((
                SetLoad::Sim {
                    grade: 0.045,
                    wind_mps: 0.0,
                },
                0.4,
            )),
            rtt_ms: Some(1.84),
            heard_s_ago: 0.3,
            paired_s: 125.0,
            relayed: 500,
            report: None,
            code: "7K2M-Q9XP-4DA".into(),
            pad_confirm_east: false,
        }
    }

    fn row<'a>(v: &'a SharingView, label: &str) -> Option<&'a str> {
        v.rows
            .iter()
            .find(|(l, _)| l == label)
            .map(|(_, v)| v.as_str())
    }

    /// The content: the browser, the live numbers, the grade, the bridge's
    /// state and round trip, the code, Stop sharing, the paused ride.
    #[test]
    fn the_screen_shows_the_browser_the_numbers_and_the_bridge() {
        let v = view(&info());
        assert!(v.open);
        assert_eq!(v.title, "Sharing");
        assert_eq!(v.subtitle, "Your trainer rides in Quest Browser");
        let tiles: Vec<_> = v
            .tiles
            .iter()
            .map(|t| (t.label.as_str(), t.value.as_str(), t.unit.as_str()))
            .collect();
        assert_eq!(
            tiles,
            [
                ("Power", "182", "W"),
                ("Cadence", "89", "rpm"),
                ("Heart rate", "131", "bpm")
            ]
        );
        assert_eq!(row(&v, "Grade set by the browser"), Some("+4.5 % (SIM)"));
        assert_eq!(
            row(&v, "Bridge"),
            Some("connected for 2 min 05 s, 500 readings sent")
        );
        assert_eq!(row(&v, "Round trip"), Some("1.8 ms"));
        assert_eq!(row(&v, "Pairing code"), Some("7K2M-Q9XP-4DA"));
        assert_eq!(row(&v, "Last headset report"), None, "none yet");
        assert_eq!(v.button, "Stop sharing");
        assert!(v.hint.contains("Enter / A"), "{}", v.hint);
        assert!(v.note.contains("paused") && v.note.contains("no hearts"));
    }

    /// Stale readings show "–"; a quiet browser says so; ERG and the
    /// headset's report; the pads' swapped Confirm.
    #[test]
    fn stale_readings_a_quiet_browser_erg_and_the_report() {
        let mut i = info();
        i.power_age_s = Some(STALE_S + 1.0);
        i.heart_rate_bpm = None;
        i.heard_s_ago = 7.0;
        i.load = Some((SetLoad::Erg { watts: 215.0 }, 1.0));
        i.report = Some((
            Report::of(
                "xr",
                &serde_json::json!({"xr": {"fps": 71.94, "work_ms_p50": 9.84}}),
            ),
            12.0,
        ));
        i.pad_confirm_east = true;
        i.browser.clear();
        let v = view(&i);
        assert!(v.tiles.iter().all(|t| t.value == "–"), "{:?}", v.tiles);
        assert_eq!(
            row(&v, "Bridge"),
            Some("waiting for the browser (last heard 7 s ago)")
        );
        assert_eq!(row(&v, "Grade set by the browser"), Some("215 W (ERG)"));
        assert_eq!(
            row(&v, "Last headset report"),
            Some("71.9 fps, frame time 9.8 ms, 12 s ago")
        );
        assert!(v.hint.contains("Enter / B"));
        assert_eq!(v.subtitle, "Your trainer rides in a browser");
        // The flat page's report: its own fields.
        let r = Report::of(
            "page",
            &serde_json::json!({"fps": 60.0, "frame_ms_p50": 16.7}),
        );
        assert_eq!((r.fps, r.frame_ms), (Some(60.0), Some(16.7)));
        i.report = Some((r, 3.0));
        assert_eq!(
            row(&view(&i), "Last browser report"),
            Some("60.0 fps, frame time 16.7 ms, 3 s ago")
        );
    }

    #[test]
    fn loads_and_durations() {
        use ocw_devices::SimulationParameters;
        let sim = DeviceCommand::SetSimulation(SimulationParameters {
            grade: -0.021,
            wind_speed_mps: 3.0,
            crr: 0.004,
            cw_kg_per_m: 0.3,
            mass_kg: 85.0,
            wind_grade: -0.01,
        });
        let l = SetLoad::of(&sim).unwrap();
        assert_eq!(l.text(), "-2.1 % (SIM, wind +3.0 m/s)");
        let calm = SetLoad::Sim {
            grade: 0.003,
            wind_mps: -0.4,
        };
        assert_eq!(calm.text(), "+0.3 % (SIM)");
        assert_eq!(SetLoad::of(&DeviceCommand::SetVirtualPower(100.0)), None);
        assert_eq!(duration(0.4), "0 s");
        assert_eq!(duration(59.4), "59 s");
        assert_eq!(duration(3725.0), "1 h 02 min");
    }
}
