//! `--screenshot` of the automated runs (CI smoke test, the agent driver
//! `drive.sh`; spec §11.4): run for a while, save a screenshot of the
//! window, exit. Shared by the desktop app and OCW Studio. The picture
//! carries its time and the build ([`crate::stamp::save_shot`]: JPEG with
//! EXIF, PNG with text chunks).

use std::path::PathBuf;

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};

use crate::hud::VersionLabel;
use crate::stamp::{MediaNow, ShotMeta, save_shot};

/// Where and when to take the screenshot.
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct AutoScreenshot {
    pub path: PathBuf,
    /// Seconds of real time after the start.
    pub delay_s: f64,
}

/// Takes the [`AutoScreenshot`] and exits (add it only with one).
pub struct AutoScreenshotPlugin;

impl Plugin for AutoScreenshotPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ScreenshotRun>()
            .add_systems(Update, take_screenshot_and_exit);
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
    shot: Option<Res<AutoScreenshot>>,
    time: Res<Time<Real>>,
    now: Res<MediaNow>,
    label: Option<Res<VersionLabel>>,
    mut run: ResMut<ScreenshotRun>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(shot) = shot else {
        return;
    };
    if !run.requested && time.elapsed_secs_f64() >= shot.delay_s {
        run.requested = true;
        info!(path = %shot.path.display(), "taking screenshot");
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_shot(
                shot.path.clone(),
                ShotMeta::new(&now, label.as_deref()),
            ))
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
