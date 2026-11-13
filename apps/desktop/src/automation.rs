//! Automated runs (CI smoke test, agent driver) on a virtual display:
//! - `--screenshot`: ride for a while, save a screenshot, exit.
//! - `--capture`: save every frame with a fixed game-time step per frame, exit.
//!
//! Every picture carries its time and the build (spec §11.4): screenshots
//! as JPEG with EXIF (PNG with text chunks for a `.png` path), the capture
//! frames as lossless PNG with text chunks (they are the input of the
//! video). Frame capture also writes `audio.wav`, the game sound rendered
//! exactly one frame's worth per frame ([`crate::audio::record`]),
//! `metadata.txt` (ffmpeg metadata for the video: `creation_time`,
//! `comment`) and `still.jpg` (the last frame); the HUD stamp follows the
//! stepped clock ([`MediaClockSetting::stepped_from_now`]).

use std::path::PathBuf;
use std::time::Duration;

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::time::TimeUpdateStrategy;
use ocw_render::hud::VersionLabel;
use ocw_render::media;
use ocw_render::stamp::{MediaClockSetting, MediaNow, ShotMeta, save_shot};

use crate::Args;
use crate::audio::GameAudio;
use crate::audio::record::{AudioCapture, samples_for_frame};

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
                .insert_resource(MediaClockSetting::stepped_from_now())
                .insert_resource(CaptureRun {
                    plan,
                    fps: args.capture_fps,
                    dir,
                    frame: 0,
                    captured: 0,
                    frames_after_done: 0,
                    audio: None,
                    audio_tried: false,
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
    now: Res<MediaNow>,
    label: Option<Res<VersionLabel>>,
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
            .observe(save_shot(path, ShotMeta::new(&now, label.as_deref())))
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

    /// The last captured frame (also saved as `still.jpg`).
    pub fn is_last(&self, n: u32) -> bool {
        n + 1 == self.frames
    }
}

#[derive(Resource)]
struct CaptureRun {
    plan: CapturePlan,
    fps: u32,
    dir: PathBuf,
    frame: u32,
    captured: u32,
    frames_after_done: u32,
    /// The game sound, rendered one frame's worth per frame.
    audio: Option<AudioCapture>,
    audio_tried: bool,
}

fn capture_frames(
    mut commands: Commands,
    mut run: ResMut<CaptureRun>,
    now: Res<MediaNow>,
    label: Option<Res<VersionLabel>>,
    game_audio: Option<Res<GameAudio>>,
    mut exit: MessageWriter<AppExit>,
) {
    let action = run.plan.action(run.frame);
    if !run.audio_tried
        && let Some(a) = game_audio
    {
        run.audio_tried = true;
        let path = run.dir.join("audio.wav");
        match AudioCapture::exact(a.recording_source().0, path) {
            Ok(c) => run.audio = Some(c),
            Err(e) => warn!("capture: no sound ({e})"),
        }
    }
    if let Some(audio) = &run.audio
        && action != CaptureAction::Finish
    {
        let n = samples_for_frame(u64::from(run.frame), audio.rate, run.fps);
        audio.render_frame(n, matches!(action, CaptureAction::Capture(_)));
    }
    let meta = ShotMeta::new(&now, label.as_deref());
    match action {
        CaptureAction::Warmup => {}
        CaptureAction::Capture(n) => {
            if n == 0 {
                let file = run.dir.join("metadata.txt");
                let text = media::ffmetadata(&media::video_metadata(&meta.time, &meta.software));
                if let Err(e) = std::fs::write(&file, text) {
                    warn!("capture: {}: {e}", file.display());
                }
            }
            let path = run.dir.join(format!("frame_{n:05}.png"));
            let mut shot = commands.spawn(Screenshot::primary_window());
            if run.plan.is_last(n) {
                shot.observe(save_shot(run.dir.join("still.jpg"), meta.clone()));
            }
            shot.observe(save_shot(path, meta)).observe(
                |_: On<ScreenshotCaptured>, mut run: ResMut<CaptureRun>| {
                    run.captured += 1;
                },
            );
        }
        CaptureAction::Finish => {
            // Wait until every requested frame was captured and written.
            if run.captured >= run.plan.frames {
                if let Some(audio) = run.audio.take() {
                    match audio.stop().wait() {
                        Ok(n) => info!(samples = n, "capture sound saved"),
                        Err(e) => warn!("capture sound failed: {e}"),
                    }
                }
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
        assert!(plan.is_last(99) && !plan.is_last(98));
    }

    #[test]
    fn capture_plan_always_captures_at_least_one_frame() {
        let plan = CapturePlan::new(30, 0.0, 0.0);
        assert_eq!(plan.action(0), CaptureAction::Capture(0));
        assert_eq!(plan.action(1), CaptureAction::Finish);
    }
}
