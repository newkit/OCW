//! Automated runs (CI smoke test, agent driver) on a virtual display:
//! - `--screenshot`: ride for a while, save a screenshot, exit.
//! - `--capture`: save every frame with a fixed game-time step per frame, exit.

use std::path::PathBuf;
use std::time::Duration;

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk};
use bevy::time::TimeUpdateStrategy;

use crate::Args;

pub struct AutomationPlugin;

impl Plugin for AutomationPlugin {
    fn build(&self, app: &mut App) {
        let args = app.world().resource::<Args>().clone();
        if args.screenshot.is_some() {
            app.init_resource::<ScreenshotRun>()
                .add_systems(Update, take_screenshot_and_exit);
        }
        if let Some(dir) = args.capture {
            std::fs::create_dir_all(&dir).expect("cannot create capture directory");
            let plan =
                CapturePlan::new(args.capture_fps, args.capture_warmup, args.capture_seconds);
            // Game time advances exactly one step per rendered frame, however
            // long the frame took to render.
            app.insert_resource(TimeUpdateStrategy::ManualDuration(plan.step))
                .insert_resource(CaptureRun {
                    plan,
                    dir,
                    frame: 0,
                    captured: 0,
                    frames_after_done: 0,
                })
                .add_systems(Update, capture_frames);
        }
    }
}

#[derive(Resource, Default)]
struct ScreenshotRun {
    requested: bool,
    /// Frames since the screenshot was captured.
    frames_since_capture: Option<u32>,
}

fn take_screenshot_and_exit(
    mut commands: Commands,
    args: Res<Args>,
    time: Res<Time<Real>>,
    mut run: ResMut<ScreenshotRun>,
    mut exit: MessageWriter<AppExit>,
) {
    let path = args
        .screenshot
        .clone()
        .expect("plugin only added with --screenshot");
    if !run.requested && time.elapsed_secs_f64() >= args.screenshot_delay {
        run.requested = true;
        info!(path = %path.display(), "taking screenshot");
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path))
            .observe(
                |_: On<ScreenshotCaptured>, mut run: ResMut<ScreenshotRun>| {
                    run.frames_since_capture = Some(0);
                },
            );
    }
    if let Some(frames) = run.frames_since_capture.as_mut() {
        // Give the save observer a few frames to write the file.
        *frames += 1;
        if *frames > 5 {
            info!("screenshot saved, exiting");
            exit.write(AppExit::Success);
        }
    }
}

/// Which frames to capture: `warmup_frames` first, then `frames` frames.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CapturePlan {
    pub step: Duration,
    pub warmup_frames: u32,
    pub frames: u32,
}

/// What to do in a given frame of a capture run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureAction {
    Warmup,
    Capture(u32),
    Finish,
}

impl CapturePlan {
    pub fn new(fps: u32, warmup_s: f64, seconds: f64) -> Self {
        let fps_f = f64::from(fps);
        Self {
            step: Duration::from_secs_f64(1.0 / fps_f),
            warmup_frames: (warmup_s * fps_f).round().max(0.0) as u32,
            frames: (seconds * fps_f).round().max(1.0) as u32,
        }
    }

    pub fn action(&self, frame: u32) -> CaptureAction {
        if frame < self.warmup_frames {
            CaptureAction::Warmup
        } else if frame - self.warmup_frames < self.frames {
            CaptureAction::Capture(frame - self.warmup_frames)
        } else {
            CaptureAction::Finish
        }
    }
}

#[derive(Resource)]
struct CaptureRun {
    plan: CapturePlan,
    dir: PathBuf,
    frame: u32,
    captured: u32,
    frames_after_done: u32,
}

fn capture_frames(
    mut commands: Commands,
    mut run: ResMut<CaptureRun>,
    mut exit: MessageWriter<AppExit>,
) {
    match run.plan.action(run.frame) {
        CaptureAction::Warmup => {}
        CaptureAction::Capture(n) => {
            let path = run.dir.join(format!("frame_{n:05}.png"));
            commands
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(path))
                .observe(|_: On<ScreenshotCaptured>, mut run: ResMut<CaptureRun>| {
                    run.captured += 1;
                });
        }
        CaptureAction::Finish => {
            // Wait until every requested frame was captured and written.
            if run.captured >= run.plan.frames {
                run.frames_after_done += 1;
                if run.frames_after_done > 3 {
                    info!(frames = run.captured, dir = %run.dir.display(), "capture finished, exiting");
                    exit.write(AppExit::Success);
                }
            }
        }
    }
    run.frame += 1;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_plan_warms_up_then_captures_numbered_frames() {
        let plan = CapturePlan::new(25, 2.0, 4.0);
        assert_eq!(plan.step, Duration::from_millis(40));
        assert_eq!((plan.warmup_frames, plan.frames), (50, 100));
        assert_eq!(plan.action(0), CaptureAction::Warmup);
        assert_eq!(plan.action(49), CaptureAction::Warmup);
        assert_eq!(plan.action(50), CaptureAction::Capture(0));
        assert_eq!(plan.action(149), CaptureAction::Capture(99));
        assert_eq!(plan.action(150), CaptureAction::Finish);
    }

    #[test]
    fn capture_plan_always_captures_at_least_one_frame() {
        let plan = CapturePlan::new(30, 0.0, 0.0);
        assert_eq!(plan.action(0), CaptureAction::Capture(0));
        assert_eq!(plan.action(1), CaptureAction::Finish);
    }
}
