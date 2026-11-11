//! What the HUD's workout panel shows, computed without the engine: text
//! lines, the whole workout's profile as blocks, and the progress.

use super::run::{RunState, WorkoutRun};
use super::{Target, Workout, format_duration};

/// Training zone of a target (by fraction of FTP, Coggan-style), for the
/// profile's colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    /// No target (free ride step).
    Free,
    /// < 55 %
    Recovery,
    /// 55–75 %
    Endurance,
    /// 75–90 %
    Tempo,
    /// 90–105 %
    Threshold,
    /// 105–120 %
    Vo2Max,
    /// ≥ 120 %
    Anaerobic,
}

pub fn zone_of(ftp_fraction: f64) -> Zone {
    match ftp_fraction {
        f if f < 0.55 => Zone::Recovery,
        f if f < 0.75 => Zone::Endurance,
        f if f < 0.90 => Zone::Tempo,
        f if f < 1.05 => Zone::Threshold,
        f if f < 1.20 => Zone::Vo2Max,
        _ => Zone::Anaerobic,
    }
}

/// One block of the profile; all values are fractions (0–1) of the
/// profile's width and height.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProfileBlock {
    pub start: f32,
    pub width: f32,
    pub height: f32,
    pub zone: Zone,
}

/// Height of free ride steps in the profile (fraction of the full height).
const FREE_HEIGHT: f64 = 0.2;
/// Ramps are drawn as steps of about this fraction of the whole width.
const RAMP_SLICE: f64 = 1.0 / 100.0;
const MAX_RAMP_SLICES: usize = 16;

/// The whole workout as blocks, for the rider's FTP. The full height is
/// the highest target, at least 120 % FTP.
pub fn profile(workout: &Workout, ftp_w: f64) -> Vec<ProfileBlock> {
    let total = workout.duration_s();
    let fraction = |p: super::Power| p.ftp_fraction(ftp_w);
    let top = workout
        .segments()
        .iter()
        .filter_map(|s| match s.target {
            Target::Power { from, to } => Some(fraction(from).max(fraction(to))),
            Target::Free => None,
        })
        .fold(1.2, f64::max);
    let mut blocks = Vec::new();
    for (i, s) in workout.segments().iter().enumerate() {
        let start = workout.start_of(i) / total;
        let width = s.duration_s / total;
        match s.target {
            Target::Free => blocks.push(ProfileBlock {
                start: start as f32,
                width: width as f32,
                height: FREE_HEIGHT as f32,
                zone: Zone::Free,
            }),
            Target::Power { from, to } => {
                let slices = if from == to {
                    1
                } else {
                    ((width / RAMP_SLICE).ceil() as usize).clamp(2, MAX_RAMP_SLICES)
                };
                for k in 0..slices {
                    let mid = (k as f64 + 0.5) / slices as f64;
                    let f = fraction(from) + (fraction(to) - fraction(from)) * mid;
                    blocks.push(ProfileBlock {
                        start: (start + width * k as f64 / slices as f64) as f32,
                        width: (width / slices as f64) as f32,
                        height: (f / top).clamp(0.02, 1.0) as f32,
                        zone: zone_of(f),
                    });
                }
            }
        }
    }
    blocks
}

/// Everything the HUD shows for a running workout.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkoutPanel {
    /// "Sweet spot 3 × 10  12:30 / 45:00"
    pub title: String,
    /// The big line: "250 W  90 rpm", "Free ride", "Finished".
    pub target: String,
    /// Zone of the current target (colours the target line).
    pub zone: Zone,
    /// Smaller lines: step and time left, next step, cue text, state.
    pub lines: Vec<String>,
    pub blocks: Vec<ProfileBlock>,
    /// Elapsed fraction of the whole workout (0–1).
    pub progress: f32,
}

impl WorkoutPanel {
    pub fn new(run: &WorkoutRun, ftp_w: f64) -> Self {
        let w = run.workout();
        let s = run.status(ftp_w);
        let target = match (s.state, s.target_w) {
            (RunState::Finished, _) => "Workout done".to_string(),
            (_, Some(t)) => match s.cadence_rpm {
                Some(c) => format!("{t:.0} W  {c:.0} rpm"),
                None => format!("{t:.0} W"),
            },
            (_, None) => "Free ride".to_string(),
        };
        let zone = s.target_w.map_or(Zone::Free, |t| {
            zone_of(if ftp_w > 0.0 { t / ftp_w } else { 0.0 })
        });
        let mut lines = Vec::new();
        if s.state != RunState::Finished {
            lines.push(format!(
                "step {}/{}{}: {} left",
                s.index + 1,
                s.count,
                s.label
                    .as_ref()
                    .map(|l| format!(" ({l})"))
                    .unwrap_or_default(),
                format_duration(s.step_left_s)
            ));
            lines.push(match &s.next {
                Some(n) => format!("next: {n}"),
                None => "last step".into(),
            });
        }
        match s.state {
            RunState::Waiting => lines.push("start pedalling to begin".into()),
            RunState::Paused => lines.push("paused: not pedalling".into()),
            RunState::PausedByRider => lines.push("paused (workout resume)".into()),
            RunState::Running | RunState::Finished => {}
        }
        if let Some(t) = s.text.as_ref().filter(|_| s.state != RunState::Finished) {
            lines.push(t.clone());
        }
        Self {
            title: format!(
                "{}  {} / {}",
                w.name,
                format_duration(s.elapsed_s),
                format_duration(s.total_s)
            ),
            target,
            zone,
            lines,
            blocks: profile(w, ftp_w),
            progress: if s.total_s > 0.0 {
                (s.elapsed_s / s.total_s).clamp(0.0, 1.0) as f32
            } else {
                0.0
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::workout::{Power, Segment, parse_toml};

    #[test]
    fn zones() {
        assert_eq!(zone_of(0.5), Zone::Recovery);
        assert_eq!(zone_of(0.55), Zone::Endurance);
        assert_eq!(zone_of(0.88), Zone::Tempo);
        assert_eq!(zone_of(1.0), Zone::Threshold);
        assert_eq!(zone_of(1.1), Zone::Vo2Max);
        assert_eq!(zone_of(1.5), Zone::Anaerobic);
    }

    #[test]
    fn profile_covers_the_workout_and_scales_to_the_top_target() {
        let w = Workout::new(
            "p",
            "",
            vec![
                Segment::new(
                    50.0,
                    Target::Power {
                        from: Power::Ftp(0.4),
                        to: Power::Ftp(0.8),
                    },
                ),
                Segment::new(25.0, Target::steady(Power::Watts(300.0))),
                Segment::new(25.0, Target::Free),
            ],
            vec![],
        )
        .unwrap();
        let b = profile(&w, 200.0);
        // The ramp: 50 slices of 1 % would be too many: 16 steps.
        assert_eq!(b.len(), 16 + 2);
        assert!(b[0].height < b[15].height);
        assert_eq!(b[0].zone, Zone::Recovery);
        assert_eq!(b[15].zone, Zone::Tempo);
        // 300 W at 200 W FTP = 150 %: the top of the profile.
        assert_eq!(b[16].height, 1.0);
        assert_eq!(b[16].zone, Zone::Anaerobic);
        assert_eq!(b[16].start, 0.5);
        assert_eq!(b[17].zone, Zone::Free);
        let end = b.last().map(|x| x.start + x.width).unwrap();
        assert!((end - 1.0).abs() < 1e-6);
        // Blocks follow each other without gaps.
        for p in b.windows(2) {
            assert!((p[0].start + p[0].width - p[1].start).abs() < 1e-5);
        }
        // Low targets are scaled against at least 120 % FTP.
        let easy = Workout::new(
            "e",
            "",
            vec![Segment::new(10.0, Target::steady(Power::Ftp(0.6)))],
            vec![],
        )
        .unwrap();
        assert!((profile(&easy, 250.0)[0].height - 0.5).abs() < 1e-6);
    }

    #[test]
    fn panel_lines_follow_the_run() {
        let w =
            Arc::new(parse_toml(include_str!("../../test-data/workouts/short-demo.toml")).unwrap());
        let mut run = WorkoutRun::new(w);
        let p = WorkoutPanel::new(&run, 200.0);
        assert_eq!(p.title, "Demo: 2-minute intervals  0:00 / 2:00");
        assert_eq!(p.target, "120 W");
        assert_eq!(p.zone, Zone::Endurance);
        assert_eq!(
            p.lines,
            [
                "step 1/6: 0:20 left",
                "next: 240 W for 0:20",
                "start pedalling to begin"
            ]
        );
        assert_eq!(p.progress, 0.0);
        for _ in 0..(25 * 8) {
            run.advance(0.125, 200.0);
        }
        let p = WorkoutPanel::new(&run, 200.0);
        assert_eq!(p.target, "240 W  95 rpm");
        assert_eq!(p.zone, Zone::Anaerobic);
        assert_eq!(p.lines[0], "step 2/6 (1/2 on): 0:15 left");
        assert_eq!(p.lines[1], "next: 110 W for 0:20");
        // The cue at 20 s shows for 10 s.
        assert_eq!(p.lines[2], "First effort: hold 120 %");
        assert!((p.progress - 25.0 / 120.0).abs() < 1e-6);
        while !run.is_finished() {
            run.skip();
        }
        let p = WorkoutPanel::new(&run, 200.0);
        assert_eq!(p.target, "Workout done");
        assert!(p.lines.is_empty());
        assert_eq!(p.progress, 1.0);
    }
}
