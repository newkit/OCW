//! The render distance (spec §14 Phase 10, 10G): how far the world is
//! drawn, fixed or following the frame rate. Bevy-free and pure; both apps
//! drive it through `ocw_ui::view_distance`, the renderer turns the
//! distance into the far plane and fog, the trees' range, the houses'
//! levels of detail and the tile ring (`ocw_render::quality::
//! QualityPreset::with_view_distance`, `stream_radius_for`).
//!
//! Three settings (Settings → Display → Graphics → Render distance):
//!
//! - `display.view_distance` ([`DistanceSetting`]): `preset` (the quality
//!   preset's own: High unlimited, Medium 30 km, Low 8 km), 1 – 60 km, or
//!   `unlimited` (the world's reach). It is the **upper limit** of the
//!   dynamic mode.
//! - `display.view_distance_auto`: the dynamic mode, **on by default
//!   everywhere** (Q8): [`Controller`] steps the distance down when the
//!   frame rate stays below the target and up when there is room.
//! - `display.target_fps` ([`TargetSetting`]): `auto` = the display's
//!   refresh on the desktop and 60 fps in the browser (Q9), `display`, or
//!   a rate; `--max-fps` caps it ([`target_fps`]).
//!
//! **With the quality presets** (`display.quality`): the preset sets the
//! default upper limit (`preset`) and the most the trees and houses get;
//! a distance below the preset's draws fewer trees and coarser houses, a
//! distance above it moves only the far plane and the fog out (the
//! preset's trees, houses, shadows, MSAA and particles stay).

use crate::menu::pages::{Activate, Row, setting_row};
use crate::settings::Settings;

/// The slider's range in km.
pub const MIN_KM: u32 = 1;
pub const MAX_KM: u32 = 60;
/// The slider's top below unlimited (m).
pub const MAX_M: f32 = MAX_KM as f32 * 1000.0;
/// The dynamic mode never goes below this distance (m; a slider set lower
/// is kept).
pub const FLOOR_M: f32 = 1_500.0;

/// `display.view_distance`'s words: `preset`, 1 – 60 (km), `unlimited`.
pub const DISTANCE_CHOICES: &[&str] = &[
    "preset",
    "1",
    "2",
    "3",
    "4",
    "5",
    "6",
    "7",
    "8",
    "9",
    "10",
    "11",
    "12",
    "13",
    "14",
    "15",
    "16",
    "17",
    "18",
    "19",
    "20",
    "21",
    "22",
    "23",
    "24",
    "25",
    "26",
    "27",
    "28",
    "29",
    "30",
    "31",
    "32",
    "33",
    "34",
    "35",
    "36",
    "37",
    "38",
    "39",
    "40",
    "41",
    "42",
    "43",
    "44",
    "45",
    "46",
    "47",
    "48",
    "49",
    "50",
    "51",
    "52",
    "53",
    "54",
    "55",
    "56",
    "57",
    "58",
    "59",
    "60",
    "unlimited",
];

/// `display.target_fps`'s words.
pub const TARGET_CHOICES: &[&str] = &["auto", "display", "30", "45", "60", "72", "90", "120"];

/// The setting `display.view_distance`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DistanceSetting {
    /// The quality preset's own distance.
    #[default]
    Preset,
    /// 1 – 60 km.
    Km(u32),
    /// As far as the world reaches.
    Unlimited,
}

impl DistanceSetting {
    /// `preset`, `unlimited`, `12` or `12km`.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase();
        match s.as_str() {
            "preset" => Some(Self::Preset),
            "unlimited" => Some(Self::Unlimited),
            _ => {
                let n = s.strip_suffix("km").unwrap_or(&s).trim();
                let k: u32 = n.parse().ok()?;
                (MIN_KM..=MAX_KM).contains(&k).then_some(Self::Km(k))
            }
        }
    }

    /// The setting's word.
    pub fn name(self) -> String {
        match self {
            Self::Preset => "preset".into(),
            Self::Km(k) => k.to_string(),
            Self::Unlimited => "unlimited".into(),
        }
    }

    /// The upper limit (m; `None`: unlimited) with a preset whose view
    /// distance is `preset_m`.
    pub fn ceiling(self, preset_m: Option<f32>) -> Option<f32> {
        match self {
            Self::Preset => preset_m,
            Self::Km(k) => Some(k as f32 * 1000.0),
            Self::Unlimited => None,
        }
    }

    /// The slider one step left (`dir` < 0) or right: from the distance it
    /// stands for (`preset`: the preset's), by 1 km, 60 km ↔ unlimited.
    pub fn step(self, preset_m: Option<f32>, dir: i32) -> Self {
        let km = self
            .ceiling(preset_m)
            .map(|m| ((m / 1000.0).round() as u32).clamp(MIN_KM, MAX_KM));
        match (km, dir.signum()) {
            (None, -1) => Self::Km(MAX_KM),
            (None, _) => Self::Unlimited,
            (Some(k), -1) => Self::Km(k.saturating_sub(1).max(MIN_KM)),
            (Some(k), 1) if k >= MAX_KM => Self::Unlimited,
            (Some(k), 1) => Self::Km(k + 1),
            (Some(k), _) => Self::Km(k),
        }
    }

    /// For the menu: "preset (30 km)", "12 km", "unlimited".
    pub fn label(self, preset_m: Option<f32>) -> String {
        match self {
            Self::Preset => format!("preset ({})", km_text(preset_m)),
            other => km_text(other.ceiling(preset_m)),
        }
    }
}

/// The setting `display.target_fps`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TargetSetting {
    /// The display's refresh on the desktop, 60 in the browser (Q9).
    #[default]
    Auto,
    /// The display's refresh (60 where it is not known).
    Display,
    /// This many frames per second.
    Fps(u32),
}

impl TargetSetting {
    /// `auto`, `display` or a rate (20 – 360).
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "display" => Some(Self::Display),
            n => n
                .parse::<u32>()
                .ok()
                .filter(|f| (20..=360).contains(f))
                .map(Self::Fps),
        }
    }

    pub fn name(self) -> String {
        match self {
            Self::Auto => "auto".into(),
            Self::Display => "display".into(),
            Self::Fps(f) => f.to_string(),
        }
    }
}

/// Where the app runs, for the target's `auto`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AppKind {
    #[default]
    Desktop,
    /// The browser app, on computers and phones alike.
    Browser,
}

/// The target in the browser and where the refresh is not known.
pub const FALLBACK_FPS: f64 = 60.0;

/// A refresh rate a display can have (24 – 500 Hz); others (0, a broken
/// driver value, a headless output) are "not known".
pub fn sane_refresh(hz: Option<f64>) -> Option<f64> {
    hz.filter(|h| h.is_finite() && (24.0..=500.0).contains(h))
}

/// The target frame rate and why: `auto` is the display's refresh on the
/// desktop and 60 in the browser (Q9), `display` the refresh (60 where it
/// is not known); a frame cap below it (`--max-fps`) wins.
pub fn target_fps(
    setting: TargetSetting,
    app: AppKind,
    refresh_hz: Option<f64>,
    max_fps: Option<f64>,
) -> (f64, &'static str) {
    let (fps, why) = match (setting, app) {
        (TargetSetting::Fps(n), _) => (f64::from(n), "chosen"),
        (TargetSetting::Auto, AppKind::Browser) => (FALLBACK_FPS, "auto: 60 in the browser"),
        (TargetSetting::Auto | TargetSetting::Display, _) => match sane_refresh(refresh_hz) {
            Some(hz) => (hz, "the display's refresh"),
            None => (FALLBACK_FPS, "the display's refresh is not known: 60"),
        },
    };
    match max_fps.filter(|m| m.is_finite() && *m > 0.0) {
        Some(m) if m < fps => (m, "the frame cap (--max-fps)"),
        _ => (fps, why),
    }
}

/// "8.5 km", "12 km", "unlimited".
pub fn km_text(m: Option<f32>) -> String {
    match m {
        None => "unlimited".into(),
        Some(m) if m < 9_950.0 => {
            let km = (m / 100.0).round() / 10.0;
            if km.fract() == 0.0 {
                format!("{km:.0} km")
            } else {
                format!("{km:.1} km")
            }
        }
        Some(m) => format!("{:.0} km", m / 1000.0),
    }
}

/// The controller's thresholds and times (spec §14 Phase 10, 10G).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Params {
    /// Below this share of the target …
    pub slow_ratio: f64,
    /// … for this long: one step down.
    pub slow_after_s: f64,
    /// Below this share (a frame over 3 × the target's frame time on
    /// average): a step down as soon as the meter's window shows it.
    pub very_slow_ratio: f64,
    /// Above this share of the target (no vsync) …
    pub fast_ratio: f64,
    /// … or at the target (vsync holds the rate at the refresh) with no
    /// frame over this many target frame times …
    pub at_target_ratio: f64,
    pub smooth_worst: f64,
    /// … for this long: one step up.
    pub fast_after_s: f64,
    /// Down by this factor, up by that one.
    pub down: f32,
    pub up: f32,
    /// At least this long between two steps.
    pub spacing_s: f64,
    /// The frame-rate meter's window: what it shows after a change.
    pub window_s: f64,
    /// Nothing changes this long after a world loads, a teleport or a
    /// jump of the time machine.
    pub settle_s: f64,
    /// A frame longer than this (a background tab, a hang) is no sample.
    pub max_frame_s: f64,
    /// After a step down, no step up to near the distance that was too
    /// slow (`below_blocked` of it) for this long. A step up that fails
    /// within `failed_up_s` goes back to the distance before it and blocks
    /// there for twice the last time (up to the max), so a GPU at its limit
    /// is probed ever more rarely.
    pub backoff_s: f64,
    pub backoff_max_s: f64,
    pub failed_up_s: f64,
    pub below_blocked: f32,
    /// Never below this distance (m).
    pub floor_m: f32,
    /// A step down whose frame rate is above this share of the one where
    /// the cutting began counts as a gain …
    pub gain_ratio: f64,
    /// … and with none down to this share of that distance the distance is
    /// not what limits: back there, no steps down for this long …
    pub no_gain_share: f32,
    pub no_gain_hold_s: f64,
    /// … unless the frame rate falls below this share of what it was.
    pub worse_ratio: f64,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            slow_ratio: 0.9,
            slow_after_s: 2.0,
            very_slow_ratio: 1.0 / 3.0,
            fast_ratio: 1.1,
            at_target_ratio: 0.97,
            smooth_worst: 1.5,
            fast_after_s: 5.0,
            down: 0.85,
            up: 1.1,
            spacing_s: 2.0,
            window_s: 1.0,
            settle_s: 5.0,
            max_frame_s: 1.0,
            backoff_s: 30.0,
            backoff_max_s: 240.0,
            failed_up_s: 10.0,
            below_blocked: 0.95,
            floor_m: FLOOR_M,
            gain_ratio: 1.1,
            no_gain_share: 0.25,
            no_gain_hold_s: 300.0,
            worse_ratio: 0.8,
        }
    }
}

/// Why the distance changed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Why {
    /// Below 90 % of the target for 2 s.
    Slow { fps: f64 },
    /// Below a third of the target.
    VerySlow { fps: f64 },
    /// Room to spare for 5 s.
    Headroom { fps: f64 },
    /// The upper limit came down (the slider, the preset).
    Limit,
    /// Shorter distances brought no frames: back to where the cutting
    /// began (the CPU or another program limits the frame rate).
    NoGain { fps: f64 },
}

/// One change of the distance (`None`: unlimited).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Step {
    pub from: Option<f32>,
    pub to: Option<f32>,
    pub why: Why,
}

impl Step {
    /// "14 → 12 km: 47 fps below 54 (90 % of 60) for 2 s".
    pub fn describe(&self, target: f64) -> String {
        let why = match self.why {
            Why::Slow { fps } => format!(
                "{fps:.0} fps below {:.0} (90 % of {target:.0}) for 2 s",
                0.9 * target
            ),
            Why::VerySlow { fps } => format!("{fps:.0} fps, below a third of {target:.0}"),
            Why::Headroom { fps } => format!("{fps:.0} fps, room to spare for 5 s"),
            Why::Limit => "the upper limit".into(),
            Why::NoGain { fps } => format!(
                "still {fps:.0} fps with a quarter of the distance: the distance is not the limit"
            ),
        };
        format!("{} → {}: {why}", km_text(self.from), km_text(self.to))
    }
}

/// One frame for [`Controller::update`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Input {
    /// This frame's length (s).
    pub dt_s: f64,
    /// The meter's average over its window, and its longest frame.
    pub fps: Option<f64>,
    pub worst_ms: Option<f64>,
    pub target_fps: f64,
    /// The upper limit (m; `None`: unlimited).
    pub ceiling: Option<f32>,
    /// The frame rate says nothing now: the menu, the loading screen.
    pub hold: bool,
}

/// The dynamic mode: steps the distance down when the frame rate stays
/// below the target and up when there is room, with a dead band between,
/// spacing, ignored windows and a backoff after a failed step up.
#[derive(Debug, Clone, PartialEq)]
pub struct Controller {
    pub params: Params,
    /// The distance (m; infinite: unlimited).
    distance: f32,
    slow_s: f64,
    fast_s: f64,
    since_change_s: f64,
    since_up_s: f64,
    grace_s: f64,
    /// No step up past this distance (m) for this many seconds.
    blocked: Option<(f32, f64)>,
    backoff_s: f64,
    /// The distance before the last step up (it was fine there).
    before_up: Option<f32>,
    /// Where the cutting began or the frame rate last rose: (m, fps).
    anchor: Option<(f32, f64)>,
    /// No steps down for this many seconds, unless below this share of
    /// that frame rate (the distance is not the limit).
    no_gain: Option<(f64, f64)>,
    last: Option<Step>,
}

fn finite(m: Option<f32>) -> f32 {
    m.unwrap_or(f32::INFINITY)
}

fn opt(m: f32) -> Option<f32> {
    m.is_finite().then_some(m)
}

impl Controller {
    /// Starts at the upper limit, after the settle window.
    pub fn new(ceiling: Option<f32>) -> Self {
        Self::with_params(ceiling, Params::default())
    }

    pub fn with_params(ceiling: Option<f32>, params: Params) -> Self {
        Self {
            params,
            distance: finite(ceiling),
            slow_s: 0.0,
            fast_s: 0.0,
            since_change_s: 0.0,
            since_up_s: f64::INFINITY,
            grace_s: params.settle_s,
            blocked: None,
            backoff_s: params.backoff_s,
            before_up: None,
            anchor: None,
            no_gain: None,
            last: None,
        }
    }

    /// The distance now (m; `None`: unlimited).
    pub fn distance(&self) -> Option<f32> {
        opt(self.distance)
    }

    /// The last change.
    pub fn last(&self) -> Option<Step> {
        self.last
    }

    /// Back to the upper limit (the slider moved, the mode was switched
    /// on): what the rider asked for, then down if it is too slow.
    pub fn reset(&mut self, ceiling: Option<f32>) {
        let params = self.params;
        let last = self.last;
        *self = Self::with_params(ceiling, params);
        self.last = last;
    }

    /// A world loaded, a teleport or a time-machine jump: the frame rate
    /// says nothing for the settle window.
    pub fn settle(&mut self) {
        self.grace_s = self.grace_s.max(self.params.settle_s);
        self.slow_s = 0.0;
        self.fast_s = 0.0;
        // A new place: what limits the frame rate may differ.
        self.anchor = None;
        self.no_gain = None;
    }

    fn changed(&mut self, from: f32, why: Why) -> Option<Step> {
        self.slow_s = 0.0;
        self.fast_s = 0.0;
        self.since_change_s = 0.0;
        let step = Step {
            from: opt(from),
            to: opt(self.distance),
            why,
        };
        self.last = Some(step);
        Some(step)
    }

    /// One frame; the change it makes, if any.
    pub fn update(&mut self, input: Input) -> Option<Step> {
        let p = self.params;
        let dt = input.dt_s.max(0.0);
        let ceiling = finite(input.ceiling);
        let floor = p.floor_m.min(ceiling);
        self.since_change_s += dt;
        self.since_up_s += dt;
        if let Some((_, left)) = &mut self.blocked {
            *left -= dt;
            if *left <= 0.0 {
                self.blocked = None;
            }
        }
        // The upper limit came down: at once.
        if self.distance > ceiling {
            let from = self.distance;
            self.distance = ceiling;
            return self.changed(from, Why::Limit);
        }
        if self.distance < floor {
            let from = self.distance;
            self.distance = floor;
            return self.changed(from, Why::Limit);
        }
        if input.hold || dt > p.max_frame_s {
            // Afterwards the meter's window holds frames of the pause.
            self.slow_s = 0.0;
            self.fast_s = 0.0;
            self.grace_s = self.grace_s.max(p.window_s);
            return None;
        }
        if self.grace_s > 0.0 {
            self.grace_s -= dt;
            self.slow_s = 0.0;
            self.fast_s = 0.0;
            return None;
        }
        let fps = input.fps?;
        let target = input.target_fps.max(1.0);
        let frame_ms = 1000.0 / target;
        if fps < p.slow_ratio * target {
            self.slow_s += dt;
        } else {
            self.slow_s = 0.0;
        }
        let headroom = fps >= p.fast_ratio * target
            || (fps >= p.at_target_ratio * target
                && input
                    .worst_ms
                    .is_some_and(|w| w <= p.smooth_worst * frame_ms));
        if headroom {
            self.fast_s += dt;
        } else {
            self.fast_s = 0.0;
        }
        let very_slow = fps < p.very_slow_ratio * target;
        let mut down = (self.slow_s >= p.slow_after_s && self.since_change_s >= p.spacing_s)
            || (very_slow && self.slow_s >= p.window_s && self.since_change_s >= p.window_s);
        // The distance was found not to be what limits the frame rate: no
        // steps down unless it gets clearly worse than it was.
        if let Some((left, anchor_fps)) = &mut self.no_gain {
            *left -= dt;
            if *left <= 0.0 || fps < p.worse_ratio * *anchor_fps {
                self.no_gain = None;
            } else {
                down = false;
                self.slow_s = 0.0;
            }
        }
        if down && self.distance > floor {
            let from = self.distance;
            // Where the frame rate last rose: shorter distances should help.
            let reach = |m: f32| if m.is_infinite() { 2.0 * MAX_M } else { m };
            let anchor = match self.anchor {
                Some((d, f)) if fps <= p.gain_ratio * f => (d, f),
                _ => (from, fps),
            };
            self.anchor = Some(anchor);
            let next = if from.is_infinite() {
                MAX_M
            } else {
                from * p.down
            };
            let failed_probe = self.before_up.is_some() && self.since_up_s <= p.failed_up_s;
            if !failed_probe && next < p.no_gain_share * reach(anchor.0) {
                // A quarter of the distance and no more frames: something
                // else limits them (the CPU, another program). Back to where
                // the cutting began, and leave it there.
                self.distance = anchor.0.min(ceiling);
                self.anchor = None;
                self.no_gain = Some((p.no_gain_hold_s, anchor.1));
                return self.changed(from, Why::NoGain { fps });
            }
            match self.before_up.filter(|_| self.since_up_s <= p.failed_up_s) {
                // A step up that failed: back to where it was fine, and no
                // higher for twice as long as last time.
                Some(good) => {
                    self.distance = good.max(floor).min(from);
                    self.backoff_s = (self.backoff_s * 2.0).min(p.backoff_max_s);
                    self.blocked = Some((self.distance, self.backoff_s));
                }
                // Too slow: -15 %, and not back up to near the slow distance
                // for a while.
                None => {
                    self.distance = if from.is_infinite() {
                        MAX_M.min(ceiling)
                    } else {
                        from * p.down
                    }
                    .max(floor);
                    self.backoff_s = p.backoff_s;
                    let cap = if from.is_infinite() {
                        MAX_M
                    } else {
                        from * p.below_blocked
                    };
                    self.blocked = Some((cap, self.backoff_s));
                }
            }
            self.before_up = None;
            let why = if very_slow {
                Why::VerySlow { fps }
            } else {
                Why::Slow { fps }
            };
            return self.changed(from, why);
        }
        if self.distance < ceiling
            && self.fast_s >= p.fast_after_s
            && self.since_change_s >= p.spacing_s
        {
            let from = self.distance;
            let mut to = from * p.up;
            if to > MAX_M {
                // Past the slider's top: unlimited, if the limit is.
                to = ceiling;
            }
            to = to.min(ceiling);
            if let Some((cap, _)) = self.blocked {
                to = to.min(cap);
            }
            if to > from * 1.001 {
                self.distance = to;
                self.since_up_s = 0.0;
                self.before_up = Some(from);
                self.anchor = None;
                return self.changed(from, Why::Headroom { fps });
            }
            // Blocked: wait for the block to end.
            self.fast_s = 0.0;
        }
        None
    }
}

/// What the menu's Render distance page shows besides the settings.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ViewInfo {
    /// The quality preset's view distance (m; `None`: unlimited).
    pub preset_m: Option<f32>,
    /// "12 km, auto, target 60 fps (the display's refresh)".
    pub status: String,
}

/// The settings' values (defaults where broken).
pub fn read_settings(settings: &Settings) -> (DistanceSetting, bool, TargetSetting) {
    let text = |k: &str| settings.get(k).map(|v| v.to_string()).unwrap_or_default();
    (
        DistanceSetting::parse(&text("display.view_distance")).unwrap_or_default(),
        settings
            .get("display.view_distance_auto")
            .ok()
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        TargetSetting::parse(&text("display.target_fps")).unwrap_or_default(),
    )
}

/// The Graphics page's Render distance row value: "preset, auto",
/// "12 km", "unlimited, auto".
pub fn summary(settings: &Settings) -> String {
    let (d, auto, _) = read_settings(settings);
    let d = match d {
        DistanceSetting::Preset => "preset".to_string(),
        other => km_text(other.ceiling(None)),
    };
    if auto { format!("{d}, auto") } else { d }
}

/// The rows of Display → Graphics → Render distance: the slider (← / →
/// by 1 km from the distance it stands for; Confirm: back to the preset's),
/// the dynamic mode, the target, and what runs now.
pub fn rows(settings: &Settings, info: &ViewInfo) -> Vec<Row> {
    let (d, _, _) = read_settings(settings);
    let set = |s: DistanceSetting| format!("set display.view_distance {}", s.name());
    vec![
        Row {
            label: "Render distance".into(),
            value: Some(d.label(info.preset_m)),
            left: Some(set(d.step(info.preset_m, -1))),
            right: Some(set(d.step(info.preset_m, 1))),
            confirm: Some(Activate::Command(set(DistanceSetting::Preset))),
        },
        setting_row(
            settings,
            "display.view_distance_auto",
            "Adjust to the frame rate",
        ),
        setting_row(settings, "display.target_fps", "Target frame rate"),
        Row {
            label: "Now".into(),
            value: Some(info.status.clone()),
            left: None,
            right: None,
            confirm: None,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_render::frame_rate::FrameRateMeter;

    const KM: f32 = 1000.0;

    /// A simulated GPU: frame time from the distance, with or without vsync
    /// (a frame waits for the next refresh).
    struct Gpu {
        ms: Box<dyn Fn(Option<f32>) -> f64>,
        vsync_hz: Option<f64>,
    }

    impl Gpu {
        fn frame_s(&self, d: Option<f32>) -> f64 {
            let ms = (self.ms)(d);
            match self.vsync_hz {
                Some(hz) => {
                    let period = 1000.0 / hz;
                    ((ms / period - 1e-9).ceil().max(1.0) * period) / 1000.0
                }
                None => ms / 1000.0,
            }
        }
    }

    /// Frame time ∝ the drawn distance (unlimited: 120 km), 8 ms base.
    fn linear(base: f64, ms_per_km: f64) -> Box<dyn Fn(Option<f32>) -> f64> {
        Box::new(move |d| base + ms_per_km * f64::from(d.unwrap_or(120.0 * KM) / KM))
    }

    struct Run {
        /// (time, distance, fps) per frame.
        trace: Vec<(f64, Option<f32>, f64)>,
        steps: Vec<(f64, Step)>,
    }

    impl Run {
        fn changes_between(&self, a: f64, b: f64) -> usize {
            self.steps.iter().filter(|(t, _)| (a..b).contains(t)).count()
        }
        fn at(&self, t: f64) -> (Option<f32>, f64) {
            let i = self
                .trace
                .iter()
                .position(|(s, _, _)| *s >= t)
                .unwrap_or(self.trace.len() - 1);
            (self.trace[i].1, self.trace[i].2)
        }
        /// The share of frames from `a` on with at least `fps`.
        fn share_at_least(&self, a: f64, fps: f64) -> f64 {
            let after: Vec<_> = self.trace.iter().filter(|(t, _, _)| *t >= a).collect();
            after.iter().filter(|(_, _, f)| *f >= fps).count() as f64 / after.len() as f64
        }
    }

    /// Closes the loop: the controller's distance → the GPU's frame time →
    /// the real frame-rate meter → the controller.
    fn simulate(
        c: &mut Controller,
        gpu: &dyn Fn(f64) -> Gpu,
        target: f64,
        ceiling: Option<f32>,
        seconds: f64,
        hold: &dyn Fn(f64) -> bool,
    ) -> Run {
        let mut meter = FrameRateMeter::default();
        let mut t = 0.0;
        let mut run = Run {
            trace: Vec::new(),
            steps: Vec::new(),
        };
        while t < seconds {
            let dt = gpu(t).frame_s(c.distance());
            t += dt;
            meter.record(dt);
            let fps = meter.fps();
            if let Some(step) = c.update(Input {
                dt_s: dt,
                fps,
                worst_ms: meter.worst_ms(),
                target_fps: target,
                ceiling,
                hold: hold(t),
            }) {
                run.steps.push((t, step));
            }
            run.trace.push((t, c.distance(), fps.unwrap_or(0.0)));
        }
        run
    }

    fn never(_: f64) -> bool {
        false
    }

    /// A slow GPU (60 fps up to about 12 km) converges below the target
    /// within 23 s after the settle window (from unlimited) and stays without oscillating –
    /// with vsync (the rate drops to 30 past the limit) and without.
    #[test]
    fn a_slow_gpu_converges_and_stays() {
        for vsync in [None, Some(60.0)] {
            let gpu = |_: f64| Gpu {
                ms: linear(8.0, 0.72),
                vsync_hz: vsync,
            };
            let mut c = Controller::new(None);
            let run = simulate(&mut c, &gpu, 60.0, None, 400.0, &never);
            let (d, fps) = run.at(28.0);
            let d = d.expect("came down from unlimited");
            assert!((FLOOR_M..=14.5 * KM).contains(&d), "{vsync:?}: {d}");
            assert!(fps >= 54.0, "{vsync:?}: {fps}");
            // After that: at the target nearly all the time, few changes
            // (a failed probe now and then, ever rarer).
            assert!(
                run.share_at_least(35.0, 54.0) >= 0.9,
                "{vsync:?}: {}",
                run.share_at_least(35.0, 54.0)
            );
            let late = run.changes_between(100.0, 400.0);
            assert!(late <= 4, "{vsync:?}: {late} changes: {:?}", run.steps);
            // Steps at least 1 s apart (2 s unless very slow).
            for w in run.steps.windows(2) {
                let gap = w[1].0 - w[0].0;
                let very = matches!(w[1].1.why, Why::VerySlow { .. });
                assert!(gap >= if very { 1.0 } else { 2.0 } - 1e-6, "{gap}");
            }
        }
    }

    /// Without vsync a GPU in the dead band (between 90 % and 97 % of the
    /// target) is left alone.
    #[test]
    fn the_dead_band_changes_nothing() {
        // 57 fps at 30 km.
        let gpu = |_: f64| Gpu {
            ms: Box::new(|_| 1000.0 / 57.0),
            vsync_hz: None,
        };
        let mut c = Controller::new(Some(30.0 * KM));
        let run = simulate(&mut c, &gpu, 60.0, Some(30.0 * KM), 120.0, &never);
        assert!(run.steps.is_empty(), "{:?}", run.steps);
        assert_eq!(c.distance(), Some(30.0 * KM));
    }

    /// A heavy load that goes away: the distance comes down, then grows
    /// back to the slider (and no further).
    #[test]
    fn a_fast_gpu_grows_to_the_slider() {
        let gpu = |t: f64| Gpu {
            ms: if t < 40.0 {
                linear(8.0, 1.5)
            } else {
                linear(3.0, 0.05)
            },
            vsync_hz: Some(60.0),
        };
        let ceiling = Some(30.0 * KM);
        let mut c = Controller::new(ceiling);
        let run = simulate(&mut c, &gpu, 60.0, ceiling, 400.0, &never);
        let low = run.at(39.0).0.unwrap();
        assert!(low < 8.0 * KM, "{low}");
        assert_eq!(c.distance(), ceiling, "back at the slider");
        assert!(run.trace.iter().all(|(_, d, _)| d.unwrap() <= 30.0 * KM));
        // Unlimited as the limit: past 60 km to unlimited.
        let mut c = Controller::new(None);
        let mut slow = Controller::new(None);
        let gpu_slow = |_: f64| Gpu {
            ms: linear(8.0, 1.5),
            vsync_hz: Some(60.0),
        };
        simulate(&mut slow, &gpu_slow, 60.0, None, 30.0, &never);
        assert!(slow.distance().is_some());
        c.distance = slow.distance;
        let fast = |_: f64| Gpu {
            ms: linear(3.0, 0.01),
            vsync_hz: Some(60.0),
        };
        simulate(&mut c, &fast, 60.0, None, 400.0, &never);
        assert_eq!(c.distance(), None, "unlimited again");
    }

    /// Single slow frames (a tile upload, 300 ms every 3 s) and the ignored
    /// windows (the settle window, the menu) change nothing.
    #[test]
    fn spikes_and_ignored_windows_do_nothing() {
        let gpu = |t: f64| Gpu {
            ms: Box::new(move |_| {
                if (t % 3.0) < 0.017 {
                    300.0
                } else {
                    10.0
                }
            }),
            vsync_hz: Some(60.0),
        };
        let mut c = Controller::new(Some(20.0 * KM));
        let run = simulate(&mut c, &gpu, 60.0, Some(20.0 * KM), 60.0, &never);
        assert!(run.steps.is_empty(), "{:?}", run.steps);

        // Very slow, but only in the first 5 s (the world loads).
        let gpu = |t: f64| Gpu {
            ms: Box::new(move |_| if t < 4.5 { 100.0 } else { 10.0 }),
            vsync_hz: Some(60.0),
        };
        let mut c = Controller::new(Some(20.0 * KM));
        let run = simulate(&mut c, &gpu, 60.0, Some(20.0 * KM), 30.0, &never);
        assert!(run.steps.is_empty(), "{:?}", run.steps);

        // Slow while the menu is open (10 – 40 s): nothing; slow after it
        // closes: a step only after the meter's window and 2 s.
        let gpu = |t: f64| Gpu {
            ms: Box::new(move |_| if t < 45.0 { 25.0 } else { 10.0 }),
            vsync_hz: None,
        };
        let menu = |t: f64| (6.0..40.0).contains(&t);
        let mut c = Controller::new(Some(20.0 * KM));
        let run = simulate(&mut c, &gpu, 60.0, Some(20.0 * KM), 60.0, &menu);
        // Before the menu (5 – 6 s): too short for a step.
        assert!(run.changes_between(0.0, 40.0) == 0, "{:?}", run.steps);
        let first = run.steps.first().expect("a step after the menu").0;
        assert!(first >= 43.0 - 1e-6, "{first}");

        // A frame over a second (a background tab) is no sample.
        let mut c = Controller::new(Some(20.0 * KM));
        c.grace_s = 0.0;
        let quiet = Input {
            dt_s: 5.0,
            fps: Some(0.2),
            worst_ms: Some(5000.0),
            target_fps: 60.0,
            ceiling: Some(20.0 * KM),
            hold: false,
        };
        assert_eq!(c.update(quiet), None);
        assert_eq!(c.update(Input { dt_s: 0.5, ..quiet }), None, "grace");
    }

    /// A frame rate the distance does not change (the CPU or another
    /// program limits it, as on a busy build machine): after a quarter of
    /// the distance without a gain, back to where the cutting began, and
    /// left there – unless it gets clearly worse.
    #[test]
    fn a_frame_rate_the_distance_does_not_change() {
        // 80 fps whatever the distance, the target 144.
        let cpu = |_: f64| Gpu {
            ms: Box::new(|_| 12.5),
            vsync_hz: None,
        };
        let mut c = Controller::new(None);
        let run = simulate(&mut c, &cpu, 144.0, None, 300.0, &never);
        let last = run.steps.last().expect("it tried").1;
        assert!(matches!(last.why, Why::NoGain { .. }), "{:?}", run.steps);
        assert_eq!(last.to, None, "back to unlimited");
        assert!(run.steps.len() <= 8, "{:?}", run.steps);
        assert_eq!(c.distance(), None);
        assert_eq!(run.changes_between(40.0, 300.0), 0, "{:?}", run.steps);
        // Much worse later (25 fps, below 80 % of the 80): it tries again.
        let worse = |_: f64| Gpu {
            ms: Box::new(|_| 40.0),
            vsync_hz: None,
        };
        let run = simulate(&mut c, &worse, 144.0, None, 20.0, &never);
        assert!(!run.steps.is_empty(), "steps again");
    }

    /// Bounds: never below the floor, never above the slider; a lower
    /// slider at once; a slider below the floor is kept.
    #[test]
    fn floor_and_ceiling() {
        // Hopeless: below 20 fps even at the floor.
        let gpu = |_: f64| Gpu {
            ms: linear(50.0, 5.0),
            vsync_hz: None,
        };
        let mut c = Controller::new(None);
        let run = simulate(&mut c, &gpu, 60.0, None, 120.0, &never);
        assert_eq!(c.distance(), Some(FLOOR_M));
        assert!(run.trace.iter().all(|(_, d, _)| d.unwrap_or(f32::MAX) >= FLOOR_M));
        // The way down from unlimited is 60 km, then -15 % steps.
        assert_eq!(run.steps[0].1.from, None);
        assert_eq!(run.steps[0].1.to, Some(MAX_M));
        assert!(matches!(run.steps[0].1.why, Why::VerySlow { .. }));

        let mut c = Controller::new(Some(30.0 * KM));
        let input = Input {
            dt_s: 0.016,
            fps: Some(60.0),
            worst_ms: Some(17.0),
            target_fps: 60.0,
            ceiling: Some(10.0 * KM),
            hold: true,
        };
        let step = c.update(input).expect("the limit came down");
        assert_eq!(step.why, Why::Limit);
        assert_eq!(c.distance(), Some(10.0 * KM));
        // A slider at 1 km (below the floor): 1 km, also when slow.
        let mut c = Controller::new(Some(1.0 * KM));
        let run = simulate(&mut c, &gpu, 60.0, Some(1.0 * KM), 30.0, &never);
        assert!(run.steps.is_empty());
        assert_eq!(c.distance(), Some(1.0 * KM));
        // Reset: back to the limit.
        let mut c = Controller::new(None);
        simulate(&mut c, &gpu, 60.0, None, 30.0, &never);
        c.reset(Some(25.0 * KM));
        assert_eq!(c.distance(), Some(25.0 * KM));
        assert!(c.last().is_some(), "the last step stays for the console");
    }

    /// The target: the display's refresh on the desktop, 60 in the browser,
    /// sane fallbacks, a chosen rate, the frame cap.
    #[test]
    fn the_target_frame_rate() {
        use AppKind::*;
        use TargetSetting::*;
        assert_eq!(target_fps(Auto, Desktop, Some(144.0), None).0, 144.0);
        assert_eq!(target_fps(Auto, Desktop, Some(59.94), None).0, 59.94);
        assert_eq!(target_fps(Auto, Browser, Some(144.0), None).0, 60.0);
        assert_eq!(target_fps(Auto, Browser, None, None).0, 60.0);
        for broken in [None, Some(0.0), Some(1.0), Some(f64::NAN), Some(10_000.0)] {
            let (fps, why) = target_fps(Auto, Desktop, broken, None);
            assert_eq!(fps, 60.0, "{broken:?}");
            assert!(why.contains("not known"));
        }
        assert_eq!(target_fps(Display, Browser, Some(120.0), None).0, 120.0);
        assert_eq!(target_fps(Display, Browser, None, None).0, 60.0);
        assert_eq!(target_fps(Fps(45), Desktop, Some(144.0), None).0, 45.0);
        let (fps, why) = target_fps(Auto, Desktop, Some(144.0), Some(25.0));
        assert_eq!((fps, why), (25.0, "the frame cap (--max-fps)"));
        assert_eq!(target_fps(Fps(30), Desktop, None, Some(60.0)).0, 30.0);
        assert_eq!(target_fps(Auto, Desktop, None, Some(0.0)).0, 60.0);
        for w in TARGET_CHOICES {
            assert!(TargetSetting::parse(w).is_some(), "{w}");
        }
        assert_eq!(TargetSetting::parse("5"), None);
        assert_eq!(TargetSetting::parse("75"), Some(Fps(75)));
    }

    #[test]
    fn distance_words_and_the_slider() {
        use DistanceSetting::*;
        for w in DISTANCE_CHOICES {
            let d = DistanceSetting::parse(w).unwrap();
            assert_eq!(d.name(), *w);
        }
        assert_eq!(DISTANCE_CHOICES.len(), 62);
        assert_eq!(DistanceSetting::parse("12km"), Some(Km(12)));
        assert_eq!(DistanceSetting::parse(" 12 KM"), Some(Km(12)));
        assert_eq!(DistanceSetting::parse("0"), None);
        assert_eq!(DistanceSetting::parse("61"), None);
        let medium = Some(30.0 * KM);
        assert_eq!(Preset.ceiling(medium), medium);
        assert_eq!(Km(12).ceiling(medium), Some(12.0 * KM));
        assert_eq!(Unlimited.ceiling(medium), None);
        // The slider moves from what the preset stands for.
        assert_eq!(Preset.step(medium, -1), Km(29));
        assert_eq!(Preset.step(medium, 1), Km(31));
        assert_eq!(Preset.step(None, -1), Km(60));
        assert_eq!(Preset.step(None, 1), Unlimited);
        assert_eq!(Km(60).step(None, 1), Unlimited);
        assert_eq!(Unlimited.step(None, -1), Km(60));
        assert_eq!(Km(1).step(None, -1), Km(1));
        assert_eq!(Preset.label(medium), "preset (30 km)");
        assert_eq!(Km(8).label(None), "8 km");
        assert_eq!(Unlimited.label(medium), "unlimited");
        assert_eq!(km_text(Some(8_500.0)), "8.5 km");
        assert_eq!(km_text(Some(12_345.0)), "12 km");
        assert_eq!(km_text(Some(1_500.0)), "1.5 km");
        assert_eq!(km_text(None), "unlimited");
        let s = Step {
            from: Some(14_100.0),
            to: Some(12_000.0),
            why: Why::Slow { fps: 47.0 },
        };
        assert_eq!(
            s.describe(60.0),
            "14 km → 12 km: 47 fps below 54 (90 % of 60) for 2 s"
        );
    }

    /// The settings round trip (registry, file text, menu rows).
    #[test]
    fn settings_round_trip_and_rows() {
        let mut s = Settings::default();
        assert_eq!(
            read_settings(&s),
            (DistanceSetting::Preset, true, TargetSetting::Auto),
            "Q8: dynamic on by default; Q9: auto target"
        );
        assert_eq!(summary(&s), "preset, auto");
        s.set("display.view_distance", "12").unwrap();
        s.set("display.view_distance_auto", "off").unwrap();
        s.set("display.target_fps", "90").unwrap();
        assert!(s.set("display.view_distance", "61").is_err());
        assert!(s.set("display.target_fps", "5").is_err());
        let text = s.to_toml();
        let (again, warnings) = Settings::parse(&text).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            read_settings(&again),
            (DistanceSetting::Km(12), false, TargetSetting::Fps(90))
        );
        assert_eq!(summary(&again), "12 km");
        let info = ViewInfo {
            preset_m: None,
            status: "12 km, fixed".into(),
        };
        let r = rows(&again, &info);
        assert!(r.len() <= 6, "the ten-foot rule");
        assert_eq!(r[0].value.as_deref(), Some("12 km"));
        assert_eq!(r[0].left.as_deref(), Some("set display.view_distance 11"));
        assert_eq!(r[0].right.as_deref(), Some("set display.view_distance 13"));
        assert_eq!(
            r[0].confirm,
            Some(Activate::Command("set display.view_distance preset".into()))
        );
        assert_eq!(
            r[1].confirm,
            Some(Activate::Command("set display.view_distance_auto on".into()))
        );
        assert_eq!(r[3].value.as_deref(), Some("12 km, fixed"));
        // Unlimited and preset on the slider.
        s.set("display.view_distance", "unlimited").unwrap();
        let r = rows(&s, &info);
        assert_eq!(r[0].right.as_deref(), Some("set display.view_distance unlimited"));
        assert_eq!(r[0].left.as_deref(), Some("set display.view_distance 60"));
    }
}
