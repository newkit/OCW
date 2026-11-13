//! V (and the console command `video`): record the window to an MP4.
//!
//! A screenshot (Bevy's screenshot API) is requested on every 1/[`VIDEO_FPS`]
//! tick, several in flight at once (the GPU readback takes a few frames),
//! and each lands at the video frame of its *request* time; the frames are
//! streamed as raw pixels into `ffmpeg` (H.264), which must be installed
//! (`make deps` installs it). It is looked up on `PATH`, then in the usual
//! Homebrew/local directories ([`ffmpeg_candidates`]): an app started from
//! Finder gets a `PATH` without them. A tick the game skipped (slower than
//! 25 fps) repeats the previous frame, so the video keeps real time; the
//! game never waits for the encoder (an image it cannot take is dropped and
//! counted). Files go to `~/Videos/OCW/` (or `<data dir>/ocw/videos/`).
//! A recording still running when the app exits is finished first.
//! While recording, the HUD shows a small red "REC m:ss" (top right); after
//! stopping, "Video saved: <file>" for a few seconds.
//!
//! The game sound goes into the video too ([`crate::audio::record`]): it
//! is written to a hidden WAV beside the hidden video while recording, and
//! after the stop ffmpeg muxes both (AAC) into the final MP4. The MP4
//! carries `creation_time` (the start, UTC) and the build id in `comment`
//! (spec §11.4).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command as Process, Stdio};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::thread::JoinHandle;

use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Capturing, Screenshot, ScreenshotCaptured};
use clap::{Arg, Command};
use ocw_render::RideView;
use ocw_render::media;
use ocw_render::stamp::ShotMeta;

use crate::audio::engine::SharedMixer;
use crate::audio::record::{AudioCapture, AudioPending, mux_args};
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, Permission};

/// Frames per second of the videos.
pub const VIDEO_FPS: f64 = 25.0;
/// At most this many copies of one frame (after a long stall, the video
/// skips instead of freezing for seconds).
const MAX_REPEATS: u64 = 12;
/// Screenshots requested but not yet captured at most.
const MAX_IN_FLIGHT: usize = 4;
/// A screenshot not captured after this long (s) is taken as lost.
const IN_FLIGHT_TIMEOUT_S: f64 = 1.0;
/// Frames buffered for the encoder thread (raw: up to 16 MB each).
const ENCODER_QUEUE: usize = 4;
/// Wider windows are scaled down to this width in the video (1440p).
const MAX_VIDEO_WIDTH: u32 = 2560;
/// H.264 quality of the live recording ("90 %", spec §11.4) and the x264
/// preset, as slow as still keeps up with a 1440p window in real time.
pub const VIDEO_CRF: u32 = 18;
pub const LIVE_PRESET: &str = "veryfast";
/// How long a start/stop message stays in the HUD (s).
pub const MESSAGE_S: f64 = 4.0;

pub struct VideoPlugin;

impl Plugin for VideoPlugin {
    fn build(&self, app: &mut App) {
        let dir = app
            .world()
            .get_resource::<crate::Args>()
            .and_then(|a| a.video_dir.clone());
        app.insert_resource(VideoRecorder { dir, ..default() })
            .add_systems(
                Update,
                (
                    video_key.run_if(ocw_render::menu::ride_input),
                    pull_audio,
                    show_in_hud,
                )
                    .chain(),
            )
            // After Update, so screenshots asked for there (console
            // `screenshot`, `--screenshot`) are seen and get the frame.
            .add_systems(PostUpdate, capture)
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
                    let meta = ShotMeta::of(world);
                    let audio = world
                        .get_resource::<crate::audio::GameAudio>()
                        .map(|a| a.recording_source());
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
                        rec.start(now, &meta, audio)
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
    /// The final MP4.
    path: PathBuf,
    /// The video while it is recorded (hidden, beside `path`).
    video_tmp: PathBuf,
    /// The MP4 metadata (spec §11.4).
    meta: Vec<(String, String)>,
    /// The game sound, if the mixer runs.
    audio: Option<AudioCapture>,
    /// The ffmpeg executable found by [`start`](VideoRecorder::start).
    ffmpeg: PathBuf,
    started_s: f64,
    /// Video frame index of the next screenshot request.
    next_request: u64,
    /// Screenshots requested and not yet captured: (frame index, request
    /// time). Bevy skips a screenshot without an event (e.g. two for one
    /// window in one frame), so entries older than [`IN_FLIGHT_TIMEOUT_S`]
    /// are forgotten.
    in_flight: Vec<(u64, f64)>,
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
    /// Game frames (updates) during the recording.
    updates: u64,
    /// Screenshots requested.
    requested: u64,
    /// Captured images written to the video.
    unique: u64,
    /// Extra copies written to fill ticks without an image.
    repeated: u64,
    /// Images dropped because the encoder was behind.
    dropped: u64,
    /// Images that arrived after a later one (skipped).
    late: u64,
    /// Sum and maximum of the time from request to capture (s).
    latency_sum_s: f64,
    latency_max_s: f64,
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

    /// Starts a recording at game time `now_s` (`meta`: its time and the
    /// build; `audio`: the mixer and whether a device plays it); returns
    /// the file it goes to.
    pub fn start(
        &mut self,
        now_s: f64,
        meta: &ShotMeta,
        audio: Option<(SharedMixer, bool)>,
    ) -> Result<PathBuf, String> {
        let r = self.start_inner(now_s, meta, audio);
        match &r {
            Ok(_) => self.message = None,
            Err(e) => self.set_message(format!("Video: {e}"), now_s),
        }
        r
    }

    fn start_inner(
        &mut self,
        now_s: f64,
        meta: &ShotMeta,
        audio: Option<(SharedMixer, bool)>,
    ) -> Result<PathBuf, String> {
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
        let video_tmp = hidden_beside(&path, "video.mp4");
        let audio = audio.and_then(|(mixer, device)| {
            AudioCapture::live(mixer, device, hidden_beside(&path, "wav"), now_s)
                .map_err(|e| warn!("video: no sound ({e})"))
                .ok()
        });
        self.active = Some(Active {
            path: path.clone(),
            video_tmp,
            meta: media::video_metadata(&meta.time, &meta.software),
            audio,
            ffmpeg,
            started_s: now_s,
            next_request: 0,
            in_flight: Vec::new(),
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
            updates = st.updates,
            requested = st.requested,
            unique = st.unique,
            repeated = st.repeated,
            dropped = st.dropped,
            late = st.late,
            latency_mean_ms = (1000.0 * st.latency_sum_s
                / (st.unique + st.dropped + st.late).max(1) as f64)
                .round(),
            latency_max_ms = (1000.0 * st.latency_max_s).round(),
            "video frames"
        );
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into(),
        );
        let audio = a.audio.map(AudioCapture::stop);
        match a.encoder {
            Some(enc) => {
                drop(enc.tx);
                let files = Finish {
                    ffmpeg: a.ffmpeg,
                    video_tmp: a.video_tmp,
                    path: path.clone(),
                    meta: a.meta,
                    audio,
                };
                std::thread::spawn(move || files.run(enc.writer));
                self.set_message(format!("Video saved: {name}"), now_s);
            }
            None => {
                if let Some(p) = audio {
                    let wav = p.path.clone();
                    let _ = p.wait();
                    let _ = std::fs::remove_file(wav);
                }
                warn!("video stopped before the first frame; nothing saved");
                self.set_message("Video stopped before the first frame: nothing saved", now_s);
            }
        }
        Some(path)
    }

    /// Stops and waits until the file is complete (app exit).
    fn stop_and_wait(&mut self) {
        if let Some(a) = self.active.take() {
            let audio = a.audio.map(AudioCapture::stop);
            match a.encoder {
                Some(enc) => {
                    drop(enc.tx);
                    Finish {
                        ffmpeg: a.ffmpeg,
                        video_tmp: a.video_tmp,
                        path: a.path,
                        meta: a.meta,
                        audio,
                    }
                    .run(enc.writer);
                }
                None => {
                    if let Some(p) = audio {
                        let wav = p.path.clone();
                        let _ = p.wait();
                        let _ = std::fs::remove_file(wav);
                    }
                }
            }
        }
    }

    /// Ends a failed recording (no file).
    fn discard(&mut self) {
        if let Some(a) = self.active.take()
            && let Some(audio) = a.audio
        {
            let p = audio.stop();
            std::thread::spawn(move || {
                let wav = p.path.clone();
                let _ = p.wait();
                let _ = std::fs::remove_file(wav);
            });
        }
    }

    /// Live recording without a device: renders the sound up to `now_s`.
    fn pull_audio(&mut self, now_s: f64) {
        if let Some(audio) = self.active.as_mut().and_then(|a| a.audio.as_mut()) {
            audio.pull(now_s);
        }
    }

    /// Video frame index of the screenshot to request at game time
    /// `now_s`, if one is due (one per tick, a limited number in flight).
    fn request(&mut self, now_s: f64) -> Option<u64> {
        let a = self.active.as_mut()?;
        a.stats.updates += 1;
        let index = frame_index(now_s - a.started_s);
        a.in_flight
            .retain(|&(_, t)| now_s - t < IN_FLIGHT_TIMEOUT_S);
        if index < a.next_request || a.in_flight.len() >= MAX_IN_FLIGHT {
            return None;
        }
        a.next_request = index + 1;
        a.in_flight.push((index, now_s));
        a.stats.requested += 1;
        Some(index)
    }

    /// Takes the captured screenshot that was requested for video frame
    /// `index` (`now_s`: game time, for messages).
    fn frame(&mut self, image: &Image, index: u64, now_s: f64) {
        let Some(a) = self.active.as_mut() else {
            return;
        };
        if let Some(&(_, t)) = a.in_flight.iter().find(|&&(i, _)| i == index) {
            let latency = now_s - t;
            a.stats.latency_sum_s += latency;
            a.stats.latency_max_s = a.stats.latency_max_s.max(latency);
        }
        a.in_flight.retain(|&(i, _)| i != index);
        let (w, h) = (image.width(), image.height());
        let Some(data) = image.data.as_ref() else {
            return;
        };
        if a.encoder.is_none() {
            let Some(pix) = pixel_format(image.texture_descriptor.format) else {
                warn!(format = ?image.texture_descriptor.format, "video: unsupported pixel format");
                self.discard();
                self.set_message("Video failed: unsupported pixel format", now_s);
                return;
            };
            match spawn_encoder(&a.ffmpeg, &a.video_tmp, pix, w, h, &a.meta) {
                Ok(enc) => {
                    a.encoder = Some(enc);
                    a.size = Some((w, h));
                }
                Err(e) => {
                    error!("video: cannot start ffmpeg: {e}");
                    self.discard();
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
                self.discard();
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
/// [`MAX_VIDEO_WIDTH`] wide, even size, yuv420p for players, CRF
/// [`VIDEO_CRF`]; [`LIVE_PRESET`] so the encoder keeps up with large
/// windows), with the metadata `meta` (spec §11.4).
pub fn ffmpeg_args(
    pix: &str,
    w: u32,
    h: u32,
    meta: &[(String, String)],
    out: &Path,
) -> Vec<String> {
    let crf = VIDEO_CRF.to_string();
    let head: Vec<String> = [
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
        LIVE_PRESET,
        "-crf",
        &crf,
        "-pix_fmt",
        "yuv420p",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    head.into_iter()
        .chain(media::metadata_args(meta))
        .chain(["-movflags".into(), "+faststart".into()])
        .chain(std::iter::once(out.display().to_string()))
        .collect()
}

/// `dir/.stem.ext` for `dir/stem.mp4`: a working file `*.mp4` globs and
/// file managers do not show.
pub fn hidden_beside(path: &Path, ext: &str) -> PathBuf {
    let stem = path
        .file_stem()
        .map_or_else(|| "video".into(), |s| s.to_string_lossy().into_owned());
    path.with_file_name(format!(".{stem}.{ext}"))
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
    meta: &[(String, String)],
) -> std::io::Result<Encoder> {
    let mut child = Process::new(ffmpeg)
        .args(ffmpeg_args(pix, w, h, meta, path))
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

/// The files of a stopped recording, put together after the encoder ends.
struct Finish {
    ffmpeg: PathBuf,
    video_tmp: PathBuf,
    path: PathBuf,
    meta: Vec<(String, String)>,
    audio: Option<AudioPending>,
}

impl Finish {
    /// Waits for the encoder and the sound, muxes them into the final MP4
    /// (only the picture if there is no sound or muxing fails).
    fn run(self, writer: JoinHandle<std::io::Result<()>>) {
        let video = writer
            .join()
            .unwrap_or_else(|_| Err(std::io::Error::other("video writer panicked")));
        let wav = self.audio.map(|p| {
            let path = p.path.clone();
            (path, p.wait())
        });
        if let Err(e) = video {
            error!(path = %self.path.display(), "video failed: {e}");
            if let Some((w, _)) = wav {
                let _ = std::fs::remove_file(w);
            }
            return;
        }
        let mut with_sound = false;
        match &wav {
            Some((w, Ok(n))) if *n > 0 => {
                let ok = Process::new(&self.ffmpeg)
                    .args(mux_args(&self.video_tmp, w, &self.meta, &self.path))
                    .stdout(Stdio::null())
                    .status()
                    .is_ok_and(|s| s.success());
                if ok {
                    with_sound = true;
                    let _ = std::fs::remove_file(&self.video_tmp);
                } else {
                    error!("video: muxing the sound failed; saved without it");
                }
            }
            Some((_, Err(e))) => error!("video: sound not recorded ({e})"),
            _ => {}
        }
        if !with_sound && let Err(e) = std::fs::rename(&self.video_tmp, &self.path) {
            error!(path = %self.path.display(), "video failed: {e}");
            return;
        }
        if let Some((w, _)) = wav {
            let _ = std::fs::remove_file(w);
        }
        info!(path = %self.path.display(), sound = with_sound, "video saved");
    }
}

fn video_key(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    now: Res<ocw_render::stamp::MediaNow>,
    label: Option<Res<ocw_render::hud::VersionLabel>>,
    audio: Option<Res<crate::audio::GameAudio>>,
    mut rec: ResMut<VideoRecorder>,
) {
    if !keys.just_pressed(KeyCode::KeyV) {
        return;
    }
    let meta = ShotMeta::new(&now, label.as_deref());
    let audio = audio.map(|a| a.recording_source());
    if rec.is_recording() {
        if let Some(p) = rec.stop(time.elapsed_secs_f64()) {
            info!(path = %p.display(), "video recording stopped");
        }
    } else if let Err(e) = rec.start(time.elapsed_secs_f64(), &meta, audio) {
        warn!("{e}");
    }
}

/// Renders the sound of a recording without a device, in step with game
/// time.
fn pull_audio(time: Res<Time>, mut rec: ResMut<VideoRecorder>) {
    rec.pull_audio(time.elapsed_secs_f64());
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

/// Marks the video's own screenshots.
#[derive(Component)]
struct VideoShot;

/// Screenshots of others not yet being captured.
type OtherPendingScreenshot = (With<Screenshot>, Without<Capturing>, Without<VideoShot>);

/// Requests a screenshot on every video frame tick while recording; each
/// remembers the frame it was requested for. Bevy takes one screenshot per
/// window and frame and skips any other, so a frame another screenshot asks
/// for (console `screenshot`, `--screenshot`) is left to it; the next video
/// image fills the gap.
fn capture(
    mut commands: Commands,
    time: Res<Time>,
    mut rec: ResMut<VideoRecorder>,
    others: Query<(), OtherPendingScreenshot>,
) {
    if !others.is_empty() {
        return;
    }
    let Some(index) = rec.request(time.elapsed_secs_f64()) else {
        return;
    };
    commands
        .spawn((Screenshot::primary_window(), VideoShot))
        .observe(
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
        // A 100 fps game; screenshots arrive two ticks after the request.
        let mut requested = Vec::new();
        for f in 0..20 {
            let now = 10.0 + f as f64 * 0.01 + 0.001;
            if let Some(i) = rec.request(now) {
                requested.push(i);
            }
            assert_eq!(rec.request(now), None); // same tick: nothing new
            let a = rec.active.as_mut().unwrap();
            let due = frame_index(now - 10.0).saturating_sub(2);
            a.in_flight.retain(|&(i, _)| i > due);
        }
        // 0.2 s at 25 fps: frames 0..=4, one per tick.
        assert_eq!(requested, vec![0, 1, 2, 3, 4]);
        // The in-flight limit holds while nothing arrives.
        for k in 5..20 {
            let _ = rec.request(10.001 + k as f64 * 0.04);
        }
        assert_eq!(rec.active.as_ref().unwrap().in_flight.len(), MAX_IN_FLIGHT);
        // Regression: a screenshot Bevy skipped (no event) must not block
        // the recording for ever; after the timeout requests go on.
        let later = 10.001 + 20.0 * 0.04 + IN_FLIGHT_TIMEOUT_S;
        assert_eq!(rec.request(later), Some(frame_index(later - 10.0)));
        assert_eq!(rec.active.as_ref().unwrap().in_flight.len(), 1);
        // 5 at first (2 still in flight), up to the limit, one after it.
        assert_eq!(
            rec.active.as_ref().unwrap().stats.requested,
            5 + (MAX_IN_FLIGHT as u64 - 2) + 1
        );
    }

    #[test]
    fn pixel_formats_and_ffmpeg_arguments() {
        assert_eq!(pixel_format(TextureFormat::Bgra8UnormSrgb), Some("bgra"));
        assert_eq!(pixel_format(TextureFormat::Rgba8UnormSrgb), Some("rgba"));
        assert_eq!(pixel_format(TextureFormat::Rgba16Float), None);
        let meta = media::video_metadata(
            &media::MediaTime {
                unix_s: 1_794_518_045,
                offset_s: 3600,
            },
            "Open Cycling World 0.1.0-abc",
        );
        let args = ffmpeg_args("bgra", 1280, 720, &meta, Path::new("/tmp/x.mp4"));
        let s = args.join(" ");
        assert!(s.contains("-crf 18 -pix_fmt yuv420p"), "{s}");
        assert!(s.contains("scale=trunc(min(2560\\,iw)/2)*2:-2"), "{s}");
        // The start in UTC and the build (spec §11.4).
        assert!(
            s.contains("-metadata creation_time=2026-11-12T21:14:05Z -metadata comment=Open Cycling World 0.1.0-abc -movflags +faststart"),
            "{s}"
        );
        assert_eq!(
            hidden_beside(Path::new("/v/2026-11-12_21-14-05.mp4"), "wav"),
            Path::new("/v/.2026-11-12_21-14-05.wav")
        );
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
        assert_eq!(rec.stop(0.0), None);
        assert_eq!(rec.status(), "not recording");
        assert_eq!(rec.message(0.0), None);
    }

    /// A recording without ffmpeg (no frames are captured in the tests).
    fn fake_active(rec: &mut VideoRecorder, path: &str, started_s: f64) {
        rec.active = Some(Active {
            path: PathBuf::from(path),
            video_tmp: hidden_beside(Path::new(path), "video.mp4"),
            meta: Vec::new(),
            audio: None,
            ffmpeg: PathBuf::from("ffmpeg"),
            started_s,
            next_request: 0,
            in_flight: Vec::new(),
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

    /// Regression: a screenshot asked for while recording (`--screenshot`,
    /// console `screenshot`) was skipped by Bevy when the video took the
    /// same frame.
    #[test]
    fn other_screenshots_get_their_frame() {
        let mut app = App::new();
        app.insert_resource(Time::<()>::default())
            .init_resource::<VideoRecorder>()
            .add_systems(Update, capture);
        fake_active(
            &mut app.world_mut().resource_mut::<VideoRecorder>(),
            "/x/v.mp4",
            0.0,
        );
        let other = app.world_mut().spawn(Screenshot::primary_window()).id();
        advance(&mut app, 0.01);
        let requested = |app: &App| {
            app.world()
                .resource::<VideoRecorder>()
                .active
                .as_ref()
                .unwrap()
                .stats
                .requested
        };
        assert_eq!(requested(&app), 0);
        app.world_mut().despawn(other);
        advance(&mut app, 0.01);
        assert_eq!(requested(&app), 1);
        let ours = app
            .world_mut()
            .query_filtered::<(), With<VideoShot>>()
            .iter(app.world())
            .count();
        assert_eq!(ours, 1);
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
