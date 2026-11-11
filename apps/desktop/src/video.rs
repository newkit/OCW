//! V (and the console command `video`): record the window to an MP4.
//!
//! A screenshot (Bevy's screenshot API) is requested on every 1/[`VIDEO_FPS`]
//! tick, several in flight at once (the GPU readback takes a few frames),
//! and each lands at the video frame of its *request* time; the frames are
//! streamed as raw pixels into `ffmpeg` (H.264), which must be installed
//! (`make deps` installs it). A tick the game skipped (slower than 25 fps)
//! repeats the previous frame, so the video keeps real time; the game never
//! waits for the encoder (a frame it cannot take is dropped and counted). Files go to `~/Videos/OCW/` (or `<data dir>/ocw/videos/`).
//! A recording still running when the app exits is finished first.
//! While recording, the HUD shows a small red "REC m:ss" (top right); after
//! stopping, "Video saved: <file>" for a few seconds.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command as Process, Stdio};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::thread::JoinHandle;

use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use clap::{Arg, Command};
use ocw_render::RideView;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, ConsoleState, Permission};

/// Frames per second of the videos.
pub const VIDEO_FPS: f64 = 25.0;
/// At most this many copies of one frame (after a long stall, the video
/// skips instead of freezing for seconds).
const MAX_REPEATS: u64 = 12;
/// Screenshots requested but not yet captured at most.
const MAX_IN_FLIGHT: usize = 6;
/// Frames buffered for the encoder thread (raw: up to 16 MB each).
const ENCODER_QUEUE: usize = 4;
/// Wider windows are scaled down to this width in the video.
const MAX_VIDEO_WIDTH: u32 = 1920;
/// How long a start/stop message stays in the HUD (s).
pub const MESSAGE_S: f64 = 4.0;

pub struct VideoPlugin;

impl Plugin for VideoPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<VideoRecorder>()
            .add_systems(Update, (video_key, capture, show_in_hud).chain())
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
                        Ok(match rec.stop(now) {
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
    /// A message for the HUD and the game time until which it is shown.
    message: Option<(String, f64)>,
}

struct Active {
    path: PathBuf,
    started_s: f64,
    /// Video frame index of the next screenshot request.
    next_request: u64,
    /// Screenshots requested and not yet captured.
    in_flight: usize,
    /// Next video frame index to write (all before it are written).
    next_index: u64,
    stats: FrameStats,
    /// Set up with the first frame (its size and pixel format).
    encoder: Option<Encoder>,
    /// Size of the first frame; later frames of another size are skipped.
    size: Option<(u32, u32)>,
}

/// What happened to the frames of a recording (logged when it stops).
#[derive(Debug, Default, Clone, Copy, PartialEq)]
struct FrameStats {
    /// Captured images written to the video.
    unique: u64,
    /// Extra copies written to fill ticks without an image.
    repeated: u64,
    /// Images dropped because the encoder was behind.
    dropped: u64,
    /// Images that arrived after a later one (skipped).
    late: u64,
}

/// One image for the encoder thread and how many video frames it fills.
type EncoderFrame = (Vec<u8>, u64);

struct Encoder {
    tx: SyncSender<EncoderFrame>,
    writer: JoinHandle<std::io::Result<()>>,
}

impl VideoRecorder {
    pub fn is_recording(&self) -> bool {
        self.active.is_some()
    }

    /// Length of the running recording at game time `now_s`.
    pub fn elapsed_s(&self, now_s: f64) -> Option<f64> {
        self.active.as_ref().map(|a| (now_s - a.started_s).max(0.0))
    }

    /// The HUD message (start error, "Video saved: ...") at `now_s`.
    pub fn message(&self, now_s: f64) -> Option<&str> {
        self.message
            .as_ref()
            .filter(|(_, until)| now_s < *until)
            .map(|(m, _)| m.as_str())
    }

    fn set_message(&mut self, msg: impl Into<String>, now_s: f64) {
        self.message = Some((msg.into(), now_s + MESSAGE_S));
    }

    pub fn status(&self) -> String {
        match &self.active {
            Some(a) => format!(
                "recording to {} ({} frames)",
                a.path.display(),
                a.next_index
            ),
            None => "not recording".into(),
        }
    }

    /// Starts a recording at game time `now_s`; returns the file it goes to.
    pub fn start(&mut self, now_s: f64) -> Result<PathBuf, String> {
        let r = self.start_inner(now_s);
        match &r {
            Ok(_) => self.message = None,
            Err(e) => self.set_message(format!("Video: {e}"), now_s),
        }
        r
    }

    fn start_inner(&mut self, now_s: f64) -> Result<PathBuf, String> {
        if !ffmpeg_available() {
            return Err(
                "ffmpeg not found: install ffmpeg (e.g. `make deps`) to record videos".into(),
            );
        }
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
            started_s: now_s,
            next_request: 0,
            in_flight: 0,
            next_index: 0,
            stats: FrameStats::default(),
            encoder: None,
            size: None,
        });
        info!(path = %path.display(), "video recording started");
        Ok(path)
    }

    /// Stops the recording at game time `now_s`; the encoder finishes the
    /// file in the background. Returns the file.
    pub fn stop(&mut self, now_s: f64) -> Option<PathBuf> {
        let a = self.active.take()?;
        let path = a.path.clone();
        let st = a.stats;
        info!(
            unique = st.unique,
            repeated = st.repeated,
            dropped = st.dropped,
            late = st.late,
            "video frames"
        );
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into(),
        );
        match a.encoder {
            Some(enc) => {
                drop(enc.tx);
                let p = path.clone();
                std::thread::spawn(move || finish(enc.writer, &p));
                self.set_message(format!("Video saved: {name}"), now_s);
            }
            None => {
                warn!("video stopped before the first frame; nothing saved");
                self.set_message("Video stopped before the first frame: nothing saved", now_s);
            }
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

    /// Video frame index of the screenshot to request at game time
    /// `now_s`, if one is due (one per tick, a limited number in flight).
    fn request(&mut self, now_s: f64) -> Option<u64> {
        let a = self.active.as_mut()?;
        let index = frame_index(now_s - a.started_s);
        if index < a.next_request || a.in_flight >= MAX_IN_FLIGHT {
            return None;
        }
        a.next_request = index + 1;
        a.in_flight += 1;
        Some(index)
    }

    /// Takes the captured screenshot that was requested for video frame
    /// `index` (`now_s`: game time, for messages).
    fn frame(&mut self, image: &Image, index: u64, now_s: f64) {
        let Some(a) = self.active.as_mut() else {
            return;
        };
        a.in_flight = a.in_flight.saturating_sub(1);
        let (w, h) = (image.width(), image.height());
        let Some(data) = image.data.as_ref() else {
            return;
        };
        if a.encoder.is_none() {
            let Some(pix) = pixel_format(image.texture_descriptor.format) else {
                warn!(format = ?image.texture_descriptor.format, "video: unsupported pixel format");
                self.active = None;
                self.set_message("Video failed: unsupported pixel format", now_s);
                return;
            };
            match spawn_encoder(&a.path, pix, w, h) {
                Ok(enc) => {
                    a.encoder = Some(enc);
                    a.size = Some((w, h));
                }
                Err(e) => {
                    error!("video: cannot start ffmpeg: {e}");
                    self.active = None;
                    self.set_message(format!("Video failed: cannot start ffmpeg: {e}"), now_s);
                    return;
                }
            }
        }
        if a.size != Some((w, h)) {
            return; // window resized: keep the first size
        }
        let copies = frame_copies(a.next_index, index);
        if copies == 0 {
            a.stats.late += 1;
            return;
        }
        let enc = a.encoder.as_ref().expect("set above");
        match enc.tx.try_send((data.clone(), copies)) {
            Ok(()) => {
                a.next_index = index + 1;
                a.stats.unique += 1;
                a.stats.repeated += copies - 1;
            }
            // The encoder is behind: drop this image, never stall the game
            // (the next one fills the gap).
            Err(TrySendError::Full(_)) => a.stats.dropped += 1,
            Err(TrySendError::Disconnected(_)) => {
                error!("video: ffmpeg stopped; recording ended");
                self.active = None;
                self.set_message("Video failed: ffmpeg stopped", now_s);
            }
        }
    }
}

/// The video frame shown at `elapsed_s` into the recording.
pub fn frame_index(elapsed_s: f64) -> u64 {
    (elapsed_s.max(0.0) * VIDEO_FPS).floor() as u64
}

/// How many video frames the image for frame `index` fills when the next
/// frame to write is `next_index`: one, plus a copy for every earlier frame
/// that got no image (at most [`MAX_REPEATS`]: after a long stall the video
/// skips ahead); 0 for an image that came too late (its frame is written).
pub fn frame_copies(next_index: u64, index: u64) -> u64 {
    if index < next_index {
        0
    } else {
        (index - next_index + 1).min(MAX_REPEATS)
    }
}

/// ffmpeg's raw pixel format for a screenshot's texture format.
pub fn pixel_format(format: TextureFormat) -> Option<&'static str> {
    match format {
        TextureFormat::Rgba8Unorm | TextureFormat::Rgba8UnormSrgb => Some("rgba"),
        TextureFormat::Bgra8Unorm | TextureFormat::Bgra8UnormSrgb => Some("bgra"),
        _ => None,
    }
}

/// ffmpeg arguments: raw frames on stdin → H.264 MP4 (at most
/// [`MAX_VIDEO_WIDTH`] wide, even size, yuv420p for players; a fast preset
/// so the encoder keeps up with large windows).
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
        &format!("scale=trunc(min({MAX_VIDEO_WIDTH}\\,iw)/2)*2:-2"),
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-crf",
        "20",
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

fn ffmpeg_available() -> bool {
    Process::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn spawn_encoder(path: &Path, pix: &str, w: u32, h: u32) -> std::io::Result<Encoder> {
    let mut child = Process::new("ffmpeg")
        .args(ffmpeg_args(pix, w, h, path))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()?;
    let stdin = child.stdin.take().expect("piped");
    // A few frames of buffer: the game never waits for the encoder, unless
    // it falls far behind.
    let (tx, rx) = sync_channel::<EncoderFrame>(ENCODER_QUEUE);
    let writer = std::thread::Builder::new()
        .name("ocw-video".into())
        .spawn(move || write_frames(rx, stdin, child))?;
    Ok(Encoder { tx, writer })
}

fn write_frames(
    rx: Receiver<EncoderFrame>,
    mut stdin: ChildStdin,
    mut child: Child,
) -> std::io::Result<()> {
    for (frame, copies) in rx {
        for _ in 0..copies {
            stdin.write_all(&frame)?;
        }
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

fn video_key(
    keys: Res<ButtonInput<KeyCode>>,
    console: Res<ConsoleState>,
    time: Res<Time>,
    mut rec: ResMut<VideoRecorder>,
) {
    if console.is_open() || !keys.just_pressed(KeyCode::KeyV) {
        return;
    }
    if rec.is_recording() {
        if let Some(p) = rec.stop(time.elapsed_secs_f64()) {
            info!(path = %p.display(), "video recording stopped");
        }
    } else if let Err(e) = rec.start(time.elapsed_secs_f64()) {
        warn!("{e}");
    }
}

/// The REC indicator and the message in the HUD.
fn show_in_hud(time: Res<Time>, rec: Res<VideoRecorder>, view: Option<ResMut<RideView>>) {
    let Some(mut view) = view else {
        return;
    };
    let now = time.elapsed_secs_f64();
    let recording = rec.elapsed_s(now);
    let message = rec.message(now).map(str::to_string);
    if view.recording_s != recording {
        view.recording_s = recording;
    }
    if view.app_message != message {
        view.app_message = message;
    }
}

/// Requests a screenshot on every video frame tick while recording; each
/// remembers the frame it was requested for.
fn capture(mut commands: Commands, time: Res<Time>, mut rec: ResMut<VideoRecorder>) {
    let Some(index) = rec.request(time.elapsed_secs_f64()) else {
        return;
    };
    commands.spawn(Screenshot::primary_window()).observe(
        move |ev: On<ScreenshotCaptured>, time: Res<Time>, mut rec: ResMut<VideoRecorder>| {
            rec.frame(&ev.image, index, time.elapsed_secs_f64());
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
    fn frames_land_at_their_request_time() {
        assert_eq!(frame_index(0.0), 0);
        assert_eq!(frame_index(0.039), 0);
        assert_eq!(frame_index(0.041), 1);
        assert_eq!(frame_index(1.001), 25);
        assert_eq!(frame_index(-1.0), 0);
        // In order, one image per tick: one frame each.
        assert_eq!(frame_copies(0, 0), 1);
        assert_eq!(frame_copies(1, 1), 1);
        // Ticks without an image (slow game, dropped image): the next image
        // fills them.
        assert_eq!(frame_copies(2, 4), 3);
        // Late (an image for frame 3 after frame 4 was written): skipped.
        assert_eq!(frame_copies(5, 3), 0);
        // A long stall: capped, the video skips ahead.
        assert_eq!(frame_copies(0, 1000), MAX_REPEATS);
    }

    /// Regression (choppy videos): one screenshot at a time, timed when it
    /// arrived, gave far fewer than 25 images per second. Now one is
    /// requested per tick, several in flight.
    #[test]
    fn a_screenshot_per_tick_with_several_in_flight() {
        let mut rec = VideoRecorder::default();
        fake_active(&mut rec, "/tmp/v.mp4", 10.0);
        // A 100 fps game; nothing arrives yet.
        let mut requested = Vec::new();
        for f in 0..20 {
            let now = 10.0 + f as f64 * 0.01 + 0.001;
            if let Some(i) = rec.request(now) {
                requested.push(i);
            }
            assert_eq!(rec.request(now), None); // same tick: nothing new
        }
        // 0.2 s at 25 fps: frames 0..=4, one per tick.
        assert_eq!(requested, vec![0, 1, 2, 3, 4]);
        // The in-flight limit holds while nothing arrives.
        for k in 5..20 {
            let _ = rec.request(10.001 + k as f64 * 0.04);
        }
        assert_eq!(rec.active.as_ref().unwrap().in_flight, MAX_IN_FLIGHT);
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
    fn stopping_without_frames_saves_nothing() {
        let mut rec = VideoRecorder::default();
        assert!(!rec.is_recording());
        assert_eq!(rec.stop(0.0), None);
        assert_eq!(rec.status(), "not recording");
        assert_eq!(rec.message(0.0), None);
    }

    /// A recording without ffmpeg (no frames are captured in the tests).
    fn fake_active(rec: &mut VideoRecorder, path: &str, started_s: f64) {
        rec.active = Some(Active {
            path: PathBuf::from(path),
            started_s,
            next_request: 0,
            in_flight: 0,
            next_index: 0,
            stats: FrameStats::default(),
            encoder: None,
            size: None,
        });
    }

    #[test]
    fn elapsed_time_and_messages() {
        let mut rec = VideoRecorder::default();
        assert_eq!(rec.elapsed_s(5.0), None);
        fake_active(&mut rec, "/tmp/v.mp4", 2.0);
        assert_eq!(rec.elapsed_s(14.5), Some(12.5));
        assert_eq!(rec.elapsed_s(1.0), Some(0.0));
        assert_eq!(rec.stop(15.0), Some(PathBuf::from("/tmp/v.mp4")));
        assert_eq!(rec.elapsed_s(15.0), None);
        // No frame came: the message says nothing was saved; it expires.
        assert!(rec.message(15.0).unwrap().contains("nothing saved"));
        assert!(rec.message(15.0 + MESSAGE_S - 0.1).is_some());
        assert_eq!(rec.message(15.0 + MESSAGE_S), None);
    }

    fn hud_app() -> App {
        let mut app = App::new();
        app.insert_resource(Time::<()>::default())
            .insert_resource(RideView::new(ocw_world::Route::test_loop()))
            .init_resource::<VideoRecorder>()
            .add_systems(Update, show_in_hud);
        app
    }

    fn advance(app: &mut App, s: f64) {
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_secs_f64(s));
        app.update();
    }

    #[test]
    fn hud_shows_recording_and_then_the_saved_file() {
        let mut app = hud_app();
        advance(&mut app, 1.0);
        let view = app.world().resource::<RideView>();
        assert_eq!((view.recording_s, view.app_message.clone()), (None, None));

        fake_active(
            &mut app.world_mut().resource_mut::<VideoRecorder>(),
            "/x/v.mp4",
            1.0,
        );
        advance(&mut app, 12.0);
        let view = app.world().resource::<RideView>();
        assert_eq!(view.recording_s, Some(12.0));
        assert_eq!(view.app_message, None);

        // A stop with an encoder running says "Video saved: <file name>".
        {
            let mut rec = app.world_mut().resource_mut::<VideoRecorder>();
            let (tx, rx) = sync_channel::<EncoderFrame>(1);
            let writer = std::thread::spawn(move || {
                for _ in rx {}
                Ok(())
            });
            rec.active.as_mut().unwrap().encoder = Some(Encoder { tx, writer });
            assert_eq!(rec.stop(13.0), Some(PathBuf::from("/x/v.mp4")));
        }
        advance(&mut app, 0.5);
        let view = app.world().resource::<RideView>();
        assert_eq!(view.recording_s, None);
        assert_eq!(view.app_message.as_deref(), Some("Video saved: v.mp4"));
        advance(&mut app, MESSAGE_S);
        assert_eq!(app.world().resource::<RideView>().app_message, None);
    }
}
