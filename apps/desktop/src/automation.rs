//! `--screenshot`: ride for a while, save a screenshot, exit.
//!
//! Used by automated runs (CI smoke test, agent driver) on a virtual display.

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk};

use crate::Args;

pub struct AutomationPlugin;

impl Plugin for AutomationPlugin {
    fn build(&self, app: &mut App) {
        if app.world().resource::<Args>().screenshot.is_some() {
            app.init_resource::<ScreenshotRun>()
                .add_systems(Update, take_screenshot_and_exit);
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
