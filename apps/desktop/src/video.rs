//! V (and the console command `video`): record the window to an MP4.
//!
//! Frames are taken with Bevy's screenshot API at up to [`VIDEO_FPS`] and
//! streamed as raw pixels into `ffmpeg` (H.264), which must be installed
//! (`make deps` installs it). It is looked up on `PATH`, then in the usual
//! Homebrew/local directories ([`ffmpeg_candidates`]): an app started from
//! Finder gets a `PATH` without them. Slow frames are repeated, so the video
//! keeps real time. Files go to `~/Videos/OCW/` (or `<data dir>/ocw/videos/`).
//! A recording still running when the app exits is finished first.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command as Process, Stdio};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::thread::JoinHandle;

use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use clap::{Arg, Command};
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, Permission};

/// Frames per second of the videos.
pub const VIDEO_FPS: f64 = 25.0;
/// At most this many copies of one frame (after a long stall, the video
/// skips instead of freezing for seconds).
const MAX_REPEATS: u64 = 12;

pub struct VideoPlugin;

impl Plugin for VideoPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<VideoRecorder>()
            .add_systems(
                Update,
                (video_key.run_if(ocw_render::menu::ride_input), capture),
            )
            .add_systems(Last, finish_on_exit)
            .console_command(ConsoleCommand::new(
                Permission::User,
                Command::new("video")
                    .about("Record the window as an MP4 video (key V): start, stop, or toggle")
                    .arg(
                        Arg::new("action")
                            .value_parser(["start", "stop", "toggle"])
                            .default_value("toggle"),
                    ),
                |world, m| {
                    let action = m.get_one::<String>("action").expect("default").clone();
                    let now = world.resource::<Time>().elapsed_secs_f64();
                    let mut rec = world.resource_mut::<VideoRecorder>();
                    let start = match action.as_str() {
                        "start" => true,
                        "stop" => false,
                        _ => !rec.is_recording(),
                    };
                    if start == rec.is_recording() {
                        return Ok(rec.status());
                    }
                    if start {
                        rec.start(now)
                            .map(|p| format!("recording to {}", p.display()))
                    } else {
                        Ok(match rec.stop() {
                            Some(p) => format!("video saved to {}", p.display()),
                            None => "not recording".into(),
                        })
                    }
                },
            ));
    }
}

/// The video recording state.
#[derive(Resource, Default)]
pub struct VideoRecorder {
    active: Option<Active>,
    /// Directory for videos (tests); default: [`default_video_dir`].
    pub dir: Option<PathBuf>,
}

struct Active {
    path: PathBuf,
    /// The ffmpeg executable found by [`start`](VideoRecorder::start).
    ffmpeg: PathBuf,
    started_s: f64,
    /// Frames written so far (including repeats).
    frames: u64,
    /// A screenshot is on its way (one at a time).
    pending: bool,
    /// Set up with the first frame (its size and pixel format).
    encoder: Option<Encoder>,
    /// Size of the first frame; later frames of another size are skipped.
    size: Option<(u32, u32)>,
}

struct Encoder {
    tx: SyncSender<Vec<u8>>,
    writer: JoinHandle<std::io::Result<()>>,
}

impl VideoRecorder {
    pub fn is_recording(&self) -> bool {
        self.active.is_some()
    }

    pub fn status(&self) -> String {
        match &self.active {
            Some(a) => format!("recording to {} ({} frames)", a.path.display(), a.frames),
            None => "not recording".into(),
        }
    }

    /// Starts a recording at game time `now_s`; returns the file it goes to.
    pub fn start(&mut self, now_s: f64) -> Result<PathBuf, String> {
        let Some(ffmpeg) = find_ffmpeg() else {
            return Err(
                "ffmpeg not found: install ffmpeg (e.g. `make deps`) to record videos".into(),
            );
        };
        let dir = self
            .dir
            .clone()
            .or_else(default_video_dir)
            .ok_or("no directory for videos")?;
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = unique_path(
            &dir,
            &crate::recording::file_stem(crate::recording::unix_now()),
        );
        self.active = Some(Active {
            path: path.clone(),
            ffmpeg,
            started_s: now_s,
            frames: 0,
            pending: false,
            encoder: None,
            size: None,
        });
        info!(path = %path.display(), "video recording started");
        Ok(path)
    }

    /// Stops the recording; the encoder finishes the file in the background.
    /// Returns the file.
    pub fn stop(&mut self) -> Option<PathBuf> {
        let a = self.active.take()?;
        let path = a.path.clone();
        match a.encoder {
            Some(enc) => {
                drop(enc.tx);
                let p = path.clone();
                std::thread::spawn(move || finish(enc.writer, &p));
            }
            None => warn!("video stopped before the first frame; nothing saved"),
        }
        Some(path)
    }

    /// Stops and waits until the file is complete (app exit).
    fn stop_and_wait(&mut self) {
        if let Some(a) = self.active.take()
            && let Some(enc) = a.encoder
        {
            drop(enc.tx);
            finish(enc.writer, &a.path);
        }
    }

    /// Takes a captured frame (`now_s`: game time).
    fn frame(&mut self, image: &Image, now_s: f64) {
        let Some(a) = self.active.as_mut() else {
            return;
        };
        a.pending = false;
        let (w, h) = (image.width(), image.height());
        let Some(data) = image.data.as_ref() else {
            return;
        };
        if a.encoder.is_none() {
            let Some(pix) = pixel_format(image.texture_descriptor.format) else {
                warn!(format = ?image.texture_descriptor.format, "video: unsupported pixel format");
                self.active = None;
                return;
            };
            match spawn_encoder(&a.ffmpeg, &a.path, pix, w, h) {
                Ok(enc) => {
                    a.encoder = Some(enc);
                    a.size = Some((w, h));
                }
                Err(e) => {
                    error!("video: cannot start ffmpeg: {e}");
                    self.active = None;
                    return;
                }
            }
        }
        if a.size != Some((w, h)) {
            return; // window resized: keep the first size
        }
        let copies = frames_due(now_s - a.started_s, a.frames).min(MAX_REPEATS);
        let enc = a.encoder.as_ref().expect("set above");
        for _ in 0..copies {
            if enc.tx.send(data.clone()).is_err() {
                error!("video: ffmpeg stopped; recording ended");
                self.active = None;
                return;
            }
            a.frames += 1;
        }
    }
}

/// How many frames to write now so that `written` frames cover
/// `elapsed_s` seconds at [`VIDEO_FPS`] (at least one for a new image).
pub fn frames_due(elapsed_s: f64, written: u64) -> u64 {
    let should = (elapsed_s.max(0.0) * VIDEO_FPS).floor() as u64 + 1;
    should.saturating_sub(written).max(1)
}

/// ffmpeg's raw pixel format for a screenshot's texture format.
pub fn pixel_format(format: TextureFormat) -> Option<&'static str> {
    match format {
        TextureFormat::Rgba8Unorm | TextureFormat::Rgba8UnormSrgb => Some("rgba"),
        TextureFormat::Bgra8Unorm | TextureFormat::Bgra8UnormSrgb => Some("bgra"),
        _ => None,
    }
}

/// ffmpeg arguments: raw frames on stdin → H.264 MP4 (even size, yuv420p
/// for players).
pub fn ffmpeg_args(pix: &str, w: u32, h: u32, out: &Path) -> Vec<String> {
    [
        "-loglevel",
        "error",
        "-y",
        "-f",
        "rawvideo",
        "-pix_fmt",
        pix,
        "-s",
        &format!("{w}x{h}"),
        "-r",
        &VIDEO_FPS.to_string(),
        "-i",
        "-",
        "-vf",
        "scale=trunc(iw/2)*2:trunc(ih/2)*2",
        "-c:v",
        "libx264",
        "-preset",
        "veryfast",
        "-crf",
        "22",
        "-pix_fmt",
        "yuv420p",
        "-movflags",
        "+faststart",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain(std::iter::once(out.display().to_string()))
    .collect()
}

pub fn default_video_dir() -> Option<PathBuf> {
    dirs::video_dir()
        .map(|d| d.join("OCW"))
        .or_else(|| dirs::data_dir().map(|d| d.join("ocw").join("videos")))
}

fn unique_path(dir: &Path, stem: &str) -> PathBuf {
    let mut p = dir.join(format!("{stem}.mp4"));
    let mut n = 2;
    while p.exists() {
        p = dir.join(format!("{stem}-{n}.mp4"));
        n += 1;
    }
    p
}

/// Where to look for ffmpeg, in order: each `PATH` directory, then
/// Homebrew (Apple Silicon, Intel) and `~/.local/bin`.
pub fn ffmpeg_candidates(path_var: Option<&std::ffi::OsStr>, home: Option<&Path>) -> Vec<PathBuf> {
    let exe = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    let mut search: Vec<PathBuf> = path_var
        .map(|p| std::env::split_paths(p).collect())
        .unwrap_or_default();
    search.push("/opt/homebrew/bin".into());
    search.push("/usr/local/bin".into());
    if let Some(home) = home {
        search.push(home.join(".local").join("bin"));
    }
    search
        .into_iter()
        .filter(|d| !d.as_os_str().is_empty())
        .map(|d| d.join(exe))
        .collect()
}

/// The first candidate that is an executable file.
pub fn first_executable(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|p| is_executable(p)).cloned()
}

#[cfg(unix)]
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(p: &Path) -> bool {
    p.is_file()
}

/// The ffmpeg executable that runs (`ffmpeg -version` succeeds). Falls back
/// to the bare name, so the system's own lookup still gets its turn.
fn find_ffmpeg() -> Option<PathBuf> {
    let candidates = ffmpeg_candidates(
        std::env::var_os("PATH").as_deref(),
        dirs::home_dir().as_deref(),
    );
    let exe = first_executable(&candidates).unwrap_or_else(|| "ffmpeg".into());
    Process::new(&exe)
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
        .then_some(exe)
}

fn spawn_encoder(
    ffmpeg: &Path,
    path: &Path,
    pix: &str,
    w: u32,
    h: u32,
) -> std::io::Result<Encoder> {
    let mut child = Process::new(ffmpeg)
        .args(ffmpeg_args(pix, w, h, path))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()?;
    let stdin = child.stdin.take().expect("piped");
    // A few frames of buffer: the game never waits for the encoder, unless
    // it falls far behind.
    let (tx, rx) = sync_channel::<Vec<u8>>(8);
    let writer = std::thread::Builder::new()
        .name("ocw-video".into())
        .spawn(move || write_frames(rx, stdin, child))?;
    Ok(Encoder { tx, writer })
}

fn write_frames(
    rx: Receiver<Vec<u8>>,
    mut stdin: ChildStdin,
    mut child: Child,
) -> std::io::Result<()> {
    for frame in rx {
        stdin.write_all(&frame)?;
    }
    drop(stdin);
    let status = child.wait()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "ffmpeg exited with {status}"
        )))
    }
}

fn finish(writer: JoinHandle<std::io::Result<()>>, path: &Path) {
    match writer.join() {
        Ok(Ok(())) => info!(path = %path.display(), "video saved"),
        Ok(Err(e)) => error!(path = %path.display(), "video failed: {e}"),
        Err(_) => error!("video writer panicked"),
    }
}

fn video_key(keys: Res<ButtonInput<KeyCode>>, time: Res<Time>, mut rec: ResMut<VideoRecorder>) {
    if !keys.just_pressed(KeyCode::KeyV) {
        return;
    }
    if rec.is_recording() {
        if let Some(p) = rec.stop() {
            info!(path = %p.display(), "video recording stopped");
        }
    } else if let Err(e) = rec.start(time.elapsed_secs_f64()) {
        warn!("{e}");
    }
}

/// Asks for the next frame while recording (one screenshot at a time).
fn capture(mut commands: Commands, mut rec: ResMut<VideoRecorder>) {
    let Some(a) = rec.active.as_mut() else {
        return;
    };
    if a.pending {
        return;
    }
    a.pending = true;
    commands.spawn(Screenshot::primary_window()).observe(
        |ev: On<ScreenshotCaptured>, time: Res<Time>, mut rec: ResMut<VideoRecorder>| {
            rec.frame(&ev.image, time.elapsed_secs_f64());
        },
    );
}

fn finish_on_exit(mut exit: MessageReader<AppExit>, mut rec: ResMut<VideoRecorder>) {
    if exit.read().next().is_some() {
        rec.stop_and_wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_keep_real_time() {
        // First frame at once; a fast game does not write more than due.
        assert_eq!(frames_due(0.0, 0), 1);
        assert_eq!(frames_due(0.01, 1), 1); // a new image still counts once
        // A slow game repeats frames to keep 25 fps: 1 s in, 10 written.
        assert_eq!(frames_due(1.0, 10), 26 - 10);
        assert_eq!(frames_due(-1.0, 0), 1);
    }

    #[test]
    fn pixel_formats_and_ffmpeg_arguments() {
        assert_eq!(pixel_format(TextureFormat::Bgra8UnormSrgb), Some("bgra"));
        assert_eq!(pixel_format(TextureFormat::Rgba8UnormSrgb), Some("rgba"));
        assert_eq!(pixel_format(TextureFormat::Rgba16Float), None);
        let args = ffmpeg_args("bgra", 1280, 720, Path::new("/tmp/x.mp4"));
        let s = args.join(" ");
        assert!(
            s.contains("-f rawvideo -pix_fmt bgra -s 1280x720 -r 25 -i -"),
            "{s}"
        );
        assert!(
            s.contains("-c:v libx264") && s.ends_with("/tmp/x.mp4"),
            "{s}"
        );
    }

    #[test]
    fn files_are_never_overwritten() {
        let dir = std::env::temp_dir().join(format!("ocw-video-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = unique_path(&dir, "2026-11-11_12-00-00");
        std::fs::write(&a, b"x").unwrap();
        let b = unique_path(&dir, "2026-11-11_12-00-00");
        assert_ne!(a, b);
        assert!(b.to_string_lossy().ends_with("-2.mp4"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn ffmpeg_is_found_on_path_first_then_in_fallback_directories() {
        let root = std::env::temp_dir().join(format!("ocw-ffmpeg-test-{}", std::process::id()));
        let (a, b, home) = (root.join("a"), root.join("b"), root.join("home"));
        for d in [&a, &b, &home.join(".local/bin")] {
            std::fs::create_dir_all(d).unwrap();
        }
        let path_var = std::env::join_paths([&a, &b]).unwrap();
        let candidates = ffmpeg_candidates(Some(&path_var), Some(&home));
        let exe = if cfg!(windows) {
            "ffmpeg.exe"
        } else {
            "ffmpeg"
        };
        assert_eq!(candidates[0], a.join(exe));
        assert_eq!(candidates[1], b.join(exe));
        assert_eq!(candidates[2], Path::new("/opt/homebrew/bin").join(exe));
        assert_eq!(candidates[3], Path::new("/usr/local/bin").join(exe));
        assert_eq!(candidates[4], home.join(".local").join("bin").join(exe));
        assert_eq!(candidates.len(), 5);

        // Only the test's own directories, so the machine's ffmpeg is not seen.
        let mine = [a.join(exe), b.join(exe), home.join(".local/bin").join(exe)];
        assert_eq!(first_executable(&mine), None);
        let make = |p: &Path, mode: u32| {
            std::fs::write(p, b"#!/bin/sh\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode)).unwrap();
            }
            let _ = mode;
        };
        make(&mine[2], 0o755);
        assert_eq!(first_executable(&mine), Some(mine[2].clone()), "fallback");
        make(&mine[1], 0o755);
        assert_eq!(first_executable(&mine), Some(mine[1].clone()), "PATH wins");
        #[cfg(unix)]
        {
            make(&mine[0], 0o644); // not executable: skipped
            assert_eq!(first_executable(&mine), Some(mine[1].clone()));
        }
        assert_eq!(
            first_executable(std::slice::from_ref(&a)),
            None,
            "a directory is no executable"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn stopping_without_frames_saves_nothing() {
        let mut rec = VideoRecorder::default();
        assert!(!rec.is_recording());
        assert_eq!(rec.stop(), None);
        assert_eq!(rec.status(), "not recording");
    }
}
