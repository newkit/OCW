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

/// The primary window, when it has a surface the renderer can draw to.
type ShownWindow = (
    With<bevy::window::PrimaryWindow>,
    With<bevy::window::RawHandleWrapper>,
);

/// What a screenshot needs: its time stamp and build label, and where to
/// take it from: the left eye in a headset session (v4.52: the first run on
/// the Quest 1 saved the window, the monitor's chase mirror), or when the
/// window has no surface; the window otherwise (`--vr-sim` shows both
/// eyes there).
#[derive(bevy::ecs::system::SystemParam)]
struct ShotSource<'w, 's> {
    now: Res<'w, MediaNow>,
    label: Option<Res<'w, VersionLabel>>,
    eyes: Option<Res<'w, crate::vr::EyeImages>>,
    vr: Option<Res<'w, crate::vr::VrConfig>>,
    shown: Query<'w, 's, (), ShownWindow>,
}

/// Where the screenshot comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShotFrom {
    Window,
    LeftEye,
}

/// The rule: a headset's session shoots the left eye; so does VR on a
/// window without a surface (else the capture never comes).
pub fn shot_from(eyes: bool, headset: bool, window_shown: bool) -> ShotFrom {
    if eyes && (headset || !window_shown) {
        ShotFrom::LeftEye
    } else {
        ShotFrom::Window
    }
}

fn take_screenshot_and_exit(
    mut commands: Commands,
    shot: Option<Res<AutoScreenshot>>,
    time: Res<Time<Real>>,
    src: ShotSource,
    mut run: ResMut<ScreenshotRun>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(shot) = shot else {
        return;
    };
    let ShotSource {
        now,
        label,
        eyes,
        vr,
        shown,
    } = src;
    if !run.requested && time.elapsed_secs_f64() >= shot.delay_s {
        run.requested = true;
        info!(path = %shot.path.display(), "taking screenshot");
        // VR on a GPU that cannot present to the window (the desktop's
        // surface guard, `Verdict::Headless`): the window has no surface, so
        // the left eye is the picture (else the capture never comes and the
        // app waits).
        let headset = vr.is_some_and(|c| c.mode == crate::vr::VrMode::Headset);
        let source = match (&eyes, shot_from(eyes.is_some(), headset, !shown.is_empty())) {
            (Some(eyes), ShotFrom::LeftEye) => {
                info!("screenshot of the left eye");
                Screenshot::image(eyes.eyes[0].clone())
            }
            _ => Screenshot::primary_window(),
        };
        commands
            .spawn(source)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression (2026-11-17, Weela): a headset session's screenshot is
    /// the left eye, not the window's mirror.
    #[test]
    fn a_headset_session_shoots_the_left_eye() {
        assert_eq!(shot_from(true, true, true), ShotFrom::LeftEye);
        assert_eq!(shot_from(true, true, false), ShotFrom::LeftEye);
        // `--vr-sim`: the window (both eyes), unless it has no surface.
        assert_eq!(shot_from(true, false, true), ShotFrom::Window);
        assert_eq!(shot_from(true, false, false), ShotFrom::LeftEye);
        // No VR: the window.
        assert_eq!(shot_from(false, false, true), ShotFrom::Window);
        assert_eq!(shot_from(false, true, false), ShotFrom::Window);
    }
}
