//! Structured workouts (spec §2.8, §5.3): the model, our own TOML format,
//! importers for `.erg` / `.mrc`, the runner that turns elapsed workout time
//! into a target power, and the display data for the HUD panel.
//!
//! Pure logic without file system or engine access (it builds for
//! `wasm32-unknown-unknown`, so the web companion can show and edit
//! workouts later): callers read the file and pass its text and name to
//! [`Workout::parse_file`].
//!
//! A workout is a flat list of [`Segment`]s (intervals are expanded when
//! parsing) plus timed text [`Cue`]s. Targets are absolute watts or a
//! fraction of the rider's FTP, resolved only when the workout runs.

mod format;
mod import;
mod panel;
mod run;

pub use format::parse_toml;
pub use import::{ImportUnits, parse_erg_mrc};
pub use panel::{ProfileBlock, WorkoutPanel, Zone, profile, zone_of};
pub use run::{AUTO_PAUSE_AFTER_S, Change, RunState, WorkoutRun, WorkoutStatus};

/// A power target: absolute, or relative to FTP.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Power {
    /// Fraction of FTP (0.88 = 88 %).
    Ftp(f64),
    Watts(f64),
}

impl Power {
    /// The target in watts for a rider with this FTP.
    pub fn watts(self, ftp_w: f64) -> f64 {
        match self {
            Self::Ftp(f) => f * ftp_w,
            Self::Watts(w) => w,
        }
    }

    /// The target as a fraction of FTP.
    pub fn ftp_fraction(self, ftp_w: f64) -> f64 {
        match self {
            Self::Ftp(f) => f,
            Self::Watts(w) if ftp_w > 0.0 => w / ftp_w,
            Self::Watts(_) => 0.0,
        }
    }
}

impl std::fmt::Display for Power {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ftp(x) => write!(f, "{:.0} %", x * 100.0),
            Self::Watts(w) => write!(f, "{w:.0} W"),
        }
    }
}

/// What a segment asks of the rider.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Target {
    /// ERG target, linear from `from` to `to` over the segment (equal for a
    /// steady segment).
    Power { from: Power, to: Power },
    /// No target: the trainer simulates the road (SIM mode) meanwhile.
    Free,
}

impl Target {
    pub fn steady(p: Power) -> Self {
        Self::Power { from: p, to: p }
    }
}

/// One stretch of a workout with a single (steady or ramped) target.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub duration_s: f64,
    pub target: Target,
    /// Suggested cadence (rpm), shown only.
    pub cadence_rpm: Option<f64>,
    /// Text shown while the segment runs.
    pub text: Option<String>,
    /// Interval position, e.g. "2/3 on" (set for expanded intervals).
    pub label: Option<String>,
}

impl Segment {
    pub fn new(duration_s: f64, target: Target) -> Self {
        Self {
            duration_s,
            target,
            cadence_rpm: None,
            text: None,
            label: None,
        }
    }

    /// Target in watts `offset_s` into the segment; `None` for free riding.
    pub fn target_w(&self, offset_s: f64, ftp_w: f64) -> Option<f64> {
        match self.target {
            Target::Power { from, to } => {
                let f = if self.duration_s > 0.0 {
                    (offset_s / self.duration_s).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let (a, b) = (from.watts(ftp_w), to.watts(ftp_w));
                Some(a + (b - a) * f)
            }
            Target::Free => None,
        }
    }

    /// Short description: "250 W", "150 → 200 W", "free ride".
    pub fn describe(&self, ftp_w: f64) -> String {
        match self.target {
            Target::Power { from, to } if from == to => {
                format!("{:.0} W", from.watts(ftp_w))
            }
            Target::Power { from, to } => {
                format!("{:.0} → {:.0} W", from.watts(ftp_w), to.watts(ftp_w))
            }
            Target::Free => "free ride".into(),
        }
    }
}

/// A text shown for a while at a point of the workout.
#[derive(Debug, Clone, PartialEq)]
pub struct Cue {
    /// Workout time when it appears (s).
    pub at_s: f64,
    /// How long it stays (s).
    pub duration_s: f64,
    pub text: String,
}

/// Default time a cue stays on screen.
pub const CUE_DURATION_S: f64 = 10.0;

/// A parsed, validated workout.
#[derive(Debug, Clone, PartialEq)]
pub struct Workout {
    pub name: String,
    pub description: String,
    segments: Vec<Segment>,
    /// Start time of each segment (s).
    starts: Vec<f64>,
    cues: Vec<Cue>,
}

/// A problem with a workout file: what and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkoutError(pub String);

impl std::fmt::Display for WorkoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for WorkoutError {}

pub(crate) fn err<T>(msg: impl Into<String>) -> Result<T, WorkoutError> {
    Err(WorkoutError(msg.into()))
}

/// Highest target accepted: 10 × FTP or 3000 W.
const MAX_FTP_FRACTION: f64 = 10.0;
const MAX_WATTS: f64 = 3000.0;

fn check_power(p: Power) -> Result<(), String> {
    match p {
        Power::Ftp(f) if !(0.0..=MAX_FTP_FRACTION).contains(&f) => Err(format!(
            "target {:.0} % FTP is outside 0–{:.0} %",
            f * 100.0,
            MAX_FTP_FRACTION * 100.0
        )),
        Power::Watts(w) if !(0.0..=MAX_WATTS).contains(&w) => {
            Err(format!("target {w} W is outside 0–{MAX_WATTS:.0} W"))
        }
        _ => Ok(()),
    }
}

impl Workout {
    /// Checks the segments (at least one, positive durations, sane
    /// targets) and sorts the cues.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        segments: Vec<Segment>,
        mut cues: Vec<Cue>,
    ) -> Result<Self, WorkoutError> {
        if segments.is_empty() {
            return err("the workout has no steps");
        }
        let mut starts = Vec::with_capacity(segments.len());
        let mut t = 0.0;
        for (i, s) in segments.iter().enumerate() {
            if !(s.duration_s.is_finite() && s.duration_s > 0.0) {
                return err(format!("segment {}: duration must be positive", i + 1));
            }
            if let Target::Power { from, to } = s.target {
                for p in [from, to] {
                    check_power(p).map_err(|e| WorkoutError(format!("segment {}: {e}", i + 1)))?;
                }
            }
            starts.push(t);
            t += s.duration_s;
        }
        for c in &cues {
            if !(c.at_s.is_finite() && c.at_s >= 0.0 && c.duration_s > 0.0) {
                return err(format!("cue {:?}: bad time or duration", c.text));
            }
        }
        cues.sort_by(|a, b| a.at_s.total_cmp(&b.at_s));
        Ok(Self {
            name: name.into(),
            description: description.into(),
            segments,
            starts,
            cues,
        })
    }

    /// Parses a workout file by its extension: `.toml` (our format),
    /// `.erg` (minutes + watts) or `.mrc` (minutes + % FTP). `file_name`
    /// also gives imported workouts without a description their name.
    pub fn parse_file(file_name: &str, text: &str) -> Result<Self, WorkoutError> {
        let lower = file_name.to_ascii_lowercase();
        let stem = file_name
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(file_name)
            .rsplit_once('.')
            .map_or(file_name, |(s, _)| s);
        let result = if lower.ends_with(".toml") {
            parse_toml(text)
        } else if lower.ends_with(".erg") {
            parse_erg_mrc(text, ImportUnits::Watts, stem)
        } else if lower.ends_with(".mrc") {
            parse_erg_mrc(text, ImportUnits::Percent, stem)
        } else {
            err("unknown workout file type (expected .toml, .erg or .mrc)")
        };
        result.map_err(|e| WorkoutError(format!("{file_name}: {e}")))
    }

    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    pub fn cues(&self) -> &[Cue] {
        &self.cues
    }

    /// Start time of segment `i` (s); the total duration for `i == len`.
    pub fn start_of(&self, i: usize) -> f64 {
        self.starts.get(i).copied().unwrap_or(self.duration_s())
    }

    pub fn duration_s(&self) -> f64 {
        self.starts.last().copied().unwrap_or(0.0)
            + self.segments.last().map_or(0.0, |s| s.duration_s)
    }

    /// The segment running at workout time `t` and the time into it;
    /// `None` at or after the end.
    pub fn segment_at(&self, t: f64) -> Option<(usize, f64)> {
        if t.is_nan() || t >= self.duration_s() {
            return None;
        }
        let i = self
            .starts
            .partition_point(|&s| s <= t.max(0.0))
            .saturating_sub(1);
        Some((i, t.max(0.0) - self.starts[i]))
    }

    /// Target in watts at workout time `t`; `None` in free segments and
    /// after the end.
    pub fn target_w_at(&self, t: f64, ftp_w: f64) -> Option<f64> {
        let (i, off) = self.segment_at(t)?;
        self.segments[i].target_w(off, ftp_w)
    }

    /// The cue showing at workout time `t` (the latest started).
    pub fn cue_at(&self, t: f64) -> Option<&Cue> {
        self.cues
            .iter()
            .rev()
            .find(|c| c.at_s <= t && t < c.at_s + c.duration_s)
    }
}

/// Formats seconds as "M:SS" (or "H:MM:SS" from an hour on).
pub fn format_duration(seconds: f64) -> String {
    let s = seconds.max(0.0).round() as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn simple() -> Workout {
        Workout::new(
            "t",
            "",
            vec![
                Segment::new(
                    60.0,
                    Target::Power {
                        from: Power::Ftp(0.5),
                        to: Power::Ftp(1.0),
                    },
                ),
                Segment::new(30.0, Target::steady(Power::Watts(300.0))),
                Segment::new(30.0, Target::Free),
            ],
            vec![Cue {
                at_s: 70.0,
                duration_s: 10.0,
                text: "go".into(),
            }],
        )
        .unwrap()
    }

    #[test]
    fn targets_ramps_and_free_segments() {
        let w = simple();
        assert_eq!(w.duration_s(), 120.0);
        assert_eq!(w.target_w_at(0.0, 200.0), Some(100.0));
        assert_eq!(w.target_w_at(30.0, 200.0), Some(150.0));
        assert_eq!(w.target_w_at(60.0, 200.0), Some(300.0));
        assert_eq!(w.target_w_at(95.0, 200.0), None);
        assert_eq!(w.target_w_at(120.0, 200.0), None);
        assert_eq!(w.segment_at(60.0), Some((1, 0.0)));
        assert_eq!(w.segment_at(119.9).map(|s| s.0), Some(2));
        assert_eq!(w.segment_at(-1.0), Some((0, 0.0)));
        assert_eq!(w.segment_at(f64::NAN), None);
        assert_eq!(w.start_of(2), 90.0);
        assert_eq!(w.start_of(3), 120.0);
        assert_eq!(w.segments()[0].describe(200.0), "100 → 200 W");
        assert_eq!(w.segments()[1].describe(200.0), "300 W");
        assert_eq!(w.cue_at(75.0).map(|c| c.text.as_str()), Some("go"));
        assert_eq!(w.cue_at(80.0), None);
    }

    #[test]
    fn invalid_workouts_are_rejected() {
        assert!(Workout::new("x", "", vec![], vec![]).is_err());
        let bad = |s: Segment| Workout::new("x", "", vec![s], vec![]).unwrap_err().0;
        assert!(bad(Segment::new(0.0, Target::Free)).contains("duration"));
        assert!(bad(Segment::new(10.0, Target::steady(Power::Watts(5000.0)))).contains("5000"));
        assert!(bad(Segment::new(10.0, Target::steady(Power::Ftp(-0.1)))).contains("% FTP"));
    }

    #[test]
    fn power_units() {
        assert_eq!(Power::Ftp(0.9).watts(250.0), 225.0);
        assert_eq!(Power::Watts(150.0).ftp_fraction(300.0), 0.5);
        assert_eq!(Power::Watts(150.0).ftp_fraction(0.0), 0.0);
        assert_eq!(Power::Ftp(0.9).to_string(), "90 %");
        assert_eq!(format_duration(65.4), "1:05");
        assert_eq!(format_duration(3725.0), "1:02:05");
        assert_eq!(format_duration(-3.0), "0:00");
    }

    #[test]
    fn file_type_by_extension() {
        let e = Workout::parse_file("a/b/x.zwo", "").unwrap_err().0;
        assert!(e.starts_with("a/b/x.zwo: unknown workout file type"), "{e}");
        let erg = "[COURSE HEADER]\nMINUTES WATTS\n[END COURSE HEADER]\n[COURSE DATA]\n0 100\n1 100\n[END COURSE DATA]\n";
        let w = Workout::parse_file("dir/My Ride.ERG", erg).unwrap();
        assert_eq!(w.name, "My Ride");
        assert_eq!(w.target_w_at(10.0, 300.0), Some(100.0));
    }
}
