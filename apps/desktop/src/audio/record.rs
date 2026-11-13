//! The game sound of video recordings (spec §3.3, §11.4): the mixer's
//! output written to a WAV file next to the video, muxed into the MP4 as
//! AAC (192 kbit/s, 48 kHz stereo) when the recording stops.
//!
//! Where the samples come from:
//! - **Tap** (live recording, V, with an output device playing): a copy of
//!   every buffer the device pulls from the mixer – what the rider hears.
//! - **Pull** (live recording without a device: `--no-audio`, headless):
//!   the game thread renders the mixer itself, as many samples as game
//!   time has passed ([`samples_due_s`]).
//! - **Exact** (frame capture `--capture`): the game thread renders exactly
//!   one frame's worth per captured frame ([`samples_for_frame`]), so the
//!   sound matches the picture however fast or slowly the frames render,
//!   with no audio device at all.
//!
//! The accounting, the WAV writer and the mux arguments are pure and
//! tested; only [`AudioCapture`] touches threads and files.

use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;

use super::engine::SharedMixer;

/// AAC bit rate, sample rate and channels of the videos' sound.
pub const AAC_BITRATE: &str = "192k";
pub const AUDIO_RATE: u32 = 48_000;
pub const AUDIO_CHANNELS: u16 = 2;

/// Samples (per channel) due before captured frame `frame` at `fps`
/// frames per second: `frame × rate / fps`, exact in integers, so no
/// rounding drift however long the capture runs.
pub fn samples_due(frame: u64, rate: u32, fps: u32) -> u64 {
    (u128::from(frame) * u128::from(rate) / u128::from(fps.max(1))) as u64
}

/// Samples of captured frame `frame` (1920 at 48 kHz and 25 fps; at rates
/// that do not divide, the frames alternate so the total stays exact).
pub fn samples_for_frame(frame: u64, rate: u32, fps: u32) -> u64 {
    samples_due(frame + 1, rate, fps) - samples_due(frame, rate, fps)
}

/// Samples due after `elapsed_s` of game time (live recording without a
/// device).
pub fn samples_due_s(elapsed_s: f64, rate: u32) -> u64 {
    (elapsed_s.max(0.0) * f64::from(rate)).floor() as u64
}

/// A 16-bit PCM WAV file written as it goes; the sizes are filled in by
/// [`finish`](Self::finish). Mono input goes to every channel, stereo
/// input to the first two.
pub struct WavWriter<W: Write + Seek> {
    out: W,
    channels: u16,
    frames: u64,
}

impl<W: Write + Seek> WavWriter<W> {
    pub fn new(mut out: W, rate: u32, channels: u16) -> std::io::Result<Self> {
        let block = channels * 2;
        let mut h = Vec::with_capacity(44);
        h.extend_from_slice(b"RIFF");
        h.extend_from_slice(&0u32.to_le_bytes());
        h.extend_from_slice(b"WAVEfmt ");
        h.extend_from_slice(&16u32.to_le_bytes());
        h.extend_from_slice(&1u16.to_le_bytes()); // PCM
        h.extend_from_slice(&channels.to_le_bytes());
        h.extend_from_slice(&rate.to_le_bytes());
        h.extend_from_slice(&(rate * u32::from(block)).to_le_bytes());
        h.extend_from_slice(&block.to_le_bytes());
        h.extend_from_slice(&16u16.to_le_bytes());
        h.extend_from_slice(b"data");
        h.extend_from_slice(&0u32.to_le_bytes());
        out.write_all(&h)?;
        Ok(Self {
            out,
            channels,
            frames: 0,
        })
    }

    /// Appends mono samples (clipped to ±1).
    #[cfg(test)]
    pub fn write_mono(&mut self, samples: &[f32]) -> std::io::Result<()> {
        let mut buf = Vec::with_capacity(samples.len() * usize::from(self.channels) * 2);
        for &s in samples {
            let v = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
            for _ in 0..self.channels {
                buf.extend_from_slice(&v.to_le_bytes());
            }
        }
        self.frames += samples.len() as u64;
        self.out.write_all(&buf)
    }

    /// Appends interleaved stereo samples (clipped to ±1); a file with
    /// more channels gets the average on the others, a mono file only the
    /// average.
    pub fn write_stereo(&mut self, samples: &[f32]) -> std::io::Result<()> {
        let ch = usize::from(self.channels);
        let pcm = |s: f32| (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        let mut buf = Vec::with_capacity(samples.len() / 2 * ch * 2);
        for f in samples.as_chunks::<2>().0 {
            let mid = pcm(0.5 * (f[0] + f[1]));
            if ch == 1 {
                buf.extend_from_slice(&mid.to_le_bytes());
                continue;
            }
            buf.extend_from_slice(&pcm(f[0]).to_le_bytes());
            buf.extend_from_slice(&pcm(f[1]).to_le_bytes());
            for _ in 2..ch {
                buf.extend_from_slice(&mid.to_le_bytes());
            }
        }
        self.frames += (samples.len() / 2) as u64;
        self.out.write_all(&buf)
    }

    /// Fills in the sizes; returns the samples per channel and the output.
    pub fn finish(mut self) -> std::io::Result<(u64, W)> {
        let data = self.frames * u64::from(self.channels) * 2;
        let data = u32::try_from(data).unwrap_or(u32::MAX - 36);
        self.out.seek(SeekFrom::Start(4))?;
        self.out.write_all(&(36 + data).to_le_bytes())?;
        self.out.seek(SeekFrom::Start(40))?;
        self.out.write_all(&data.to_le_bytes())?;
        self.out.seek(SeekFrom::End(0))?;
        self.out.flush()?;
        Ok((self.frames, self.out))
    }
}

/// ffmpeg arguments that put the recording's `video` (H.264, copied) and
/// `audio` (WAV, encoded as AAC) into `out`, with the metadata `meta`.
pub fn mux_args(video: &Path, audio: &Path, meta: &[(String, String)], out: &Path) -> Vec<String> {
    let mut a: Vec<String> = [
        "-loglevel",
        "error",
        "-y",
        "-i",
        &video.display().to_string(),
        "-i",
        &audio.display().to_string(),
        "-map",
        "0:v:0",
        "-map",
        "1:a:0",
        "-c:v",
        "copy",
        "-c:a",
        "aac",
        "-b:a",
        AAC_BITRATE,
        "-ar",
        &AUDIO_RATE.to_string(),
        "-ac",
        &AUDIO_CHANNELS.to_string(),
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    a.extend(ocw_render::media::metadata_args(meta));
    a.extend(["-movflags".into(), "+faststart".into()]);
    a.push(out.display().to_string());
    a
}

/// Where the samples come from (module docs).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Source {
    Tap,
    /// Game time at the start and the samples rendered so far.
    Pull {
        start_s: f64,
        rendered: u64,
    },
    Exact,
}

/// A running recording of the mixer's output into a WAV file.
pub struct AudioCapture {
    mixer: SharedMixer,
    tx: Sender<Vec<f32>>,
    writer: Writer,
    pub path: PathBuf,
    pub rate: u32,
    source: Source,
}

/// A stopped recording whose file is still being finished.
pub struct AudioPending {
    writer: Writer,
    pub path: PathBuf,
}

impl AudioPending {
    /// Waits for the file; returns the samples per channel in it.
    pub fn wait(self) -> std::io::Result<u64> {
        self.writer
            .join()
            .unwrap_or_else(|_| Err(std::io::Error::other("audio writer panicked")))
    }
}

/// The writer thread: samples per channel written, or the error.
type Writer = JoinHandle<std::io::Result<u64>>;

fn spawn_writer(path: &Path, rate: u32) -> std::io::Result<(Sender<Vec<f32>>, Writer)> {
    let wav = WavWriter::new(BufWriter::new(File::create(path)?), rate, AUDIO_CHANNELS)?;
    let (tx, rx) = channel::<Vec<f32>>();
    let writer = std::thread::Builder::new()
        .name("ocw-video-audio".into())
        .spawn(move || write_all(wav, rx))?;
    Ok((tx, writer))
}

fn write_all(mut wav: WavWriter<BufWriter<File>>, rx: Receiver<Vec<f32>>) -> std::io::Result<u64> {
    for chunk in rx {
        wav.write_stereo(&chunk)?;
    }
    Ok(wav.finish()?.0)
}

impl AudioCapture {
    /// Live recording from game time `now_s`: taps the device's buffers
    /// while one plays (`device_playing`), else renders with game time.
    pub fn live(
        mixer: SharedMixer,
        device_playing: bool,
        path: PathBuf,
        now_s: f64,
    ) -> std::io::Result<Self> {
        let source = if device_playing {
            Source::Tap
        } else {
            Source::Pull {
                start_s: now_s,
                rendered: 0,
            }
        };
        Self::start(mixer, path, source)
    }

    /// Frame capture: [`render_frame`](Self::render_frame) per frame.
    pub fn exact(mixer: SharedMixer, path: PathBuf) -> std::io::Result<Self> {
        Self::start(mixer, path, Source::Exact)
    }

    fn start(mixer: SharedMixer, path: PathBuf, source: Source) -> std::io::Result<Self> {
        let rate = mixer.lock().map_or(AUDIO_RATE, |m| m.sample_rate());
        let (tx, writer) = spawn_writer(&path, rate)?;
        if source == Source::Tap
            && let Ok(mut m) = mixer.lock()
        {
            m.set_tap(Some(tx.clone()));
        }
        Ok(Self {
            mixer,
            tx,
            writer,
            path,
            rate,
            source,
        })
    }

    /// `n` stereo frames from the mixer (interleaved).
    fn render(&self, n: u64) -> Vec<f32> {
        let mut buf = vec![0.0; 2 * n as usize];
        if let Ok(mut m) = self.mixer.lock() {
            m.render(&mut buf, 2);
        }
        buf
    }

    /// Live recording without a device: renders what game time `now_s`
    /// makes due (nothing for a tap).
    pub fn pull(&mut self, now_s: f64) {
        if let Source::Pull { start_s, rendered } = self.source {
            let due = samples_due_s(now_s - start_s, self.rate);
            if due > rendered {
                let _ = self.tx.send(self.render(due - rendered));
                self.source = Source::Pull {
                    start_s,
                    rendered: due,
                };
            }
        }
    }

    /// Frame capture: renders `n` samples; written when `keep` (the
    /// warm-up frames are rendered and dropped, so the sound is settled
    /// when the video starts).
    pub fn render_frame(&self, n: u64, keep: bool) {
        let s = self.render(n);
        if keep {
            let _ = self.tx.send(s);
        }
    }

    /// Stops listening; the file is finished in the background.
    pub fn stop(self) -> AudioPending {
        if self.source == Source::Tap
            && let Ok(mut m) = self.mixer.lock()
        {
            m.set_tap(None);
        }
        drop(self.tx);
        AudioPending {
            writer: self.writer,
            path: self.path,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::mix::BusGains;
    use super::super::mixer::Mixer;
    use super::*;
    use std::io::Cursor;
    use std::sync::{Arc, Mutex};

    #[test]
    fn samples_per_captured_frame() {
        assert_eq!(samples_for_frame(0, 48_000, 25), 1920);
        assert_eq!(samples_for_frame(99, 48_000, 25), 1920);
        assert_eq!(samples_for_frame(0, 48_000, 30), 1600);
        // 44.1 kHz at 30 fps: 1470 each; at 25 fps 1764.
        assert_eq!(samples_for_frame(7, 44_100, 30), 1470);
        // A rate that does not divide: frames alternate, the sum is exact.
        let total: u64 = (0..60).map(|f| samples_for_frame(f, 44_100, 60 * 7)).sum();
        assert_eq!(total, samples_due(60, 44_100, 420));
        assert_eq!(samples_due(60, 44_100, 420), 6300);
        let sizes: std::collections::BTreeSet<u64> =
            (0..100).map(|f| samples_for_frame(f, 1000, 3)).collect();
        assert_eq!(sizes, [333, 334].into_iter().collect());
        // An hour at 25 fps: exactly an hour of samples.
        assert_eq!(samples_due(90_000, 48_000, 25), 3600 * 48_000);
        assert_eq!(samples_due_s(1.5, 48_000), 72_000);
        assert_eq!(samples_due_s(-1.0, 48_000), 0);
    }

    #[test]
    fn wav_header_and_samples() {
        let mut w = WavWriter::new(Cursor::new(Vec::new()), 48_000, 2).unwrap();
        w.write_mono(&[0.0, 1.0, -1.0, 2.0]).unwrap();
        w.write_mono(&[0.5]).unwrap();
        let (frames, out) = w.finish().unwrap();
        let b = out.into_inner();
        assert_eq!(frames, 5);
        assert_eq!(b.len(), 44 + 5 * 4);
        let u32_at = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        let u16_at = |o: usize| u16::from_le_bytes(b[o..o + 2].try_into().unwrap());
        assert_eq!(&b[0..4], b"RIFF");
        assert_eq!(u32_at(4), 36 + 20);
        assert_eq!(&b[8..16], b"WAVEfmt ");
        assert_eq!((u16_at(20), u16_at(22)), (1, 2)); // PCM, stereo
        assert_eq!(u32_at(24), 48_000);
        assert_eq!(u32_at(28), 48_000 * 4);
        assert_eq!((u16_at(32), u16_at(34)), (4, 16));
        assert_eq!(&b[36..40], b"data");
        assert_eq!(u32_at(40), 20);
        let s = |i: usize| i16::from_le_bytes([b[44 + 2 * i], b[45 + 2 * i]]);
        // Mono on both channels, clipped.
        assert_eq!((s(0), s(1)), (0, 0));
        assert_eq!((s(2), s(3)), (32767, 32767));
        assert_eq!((s(4), s(5)), (-32767, -32767));
        assert_eq!(s(6), 32767);
        assert_eq!(s(8), 16384);
        // Stereo input keeps its sides.
        let mut w = WavWriter::new(Cursor::new(Vec::new()), 48_000, 2).unwrap();
        w.write_stereo(&[0.5, -0.5, 1.0, 0.0]).unwrap();
        let (frames, out) = w.finish().unwrap();
        let b = out.into_inner();
        assert_eq!((frames, b.len()), (2, 44 + 8));
        let s = |i: usize| i16::from_le_bytes([b[44 + 2 * i], b[45 + 2 * i]]);
        assert_eq!((s(0), s(1), s(2), s(3)), (16384, -16384, 32767, 0));
        // Into a mono file: the average.
        let mut w = WavWriter::new(Cursor::new(Vec::new()), 48_000, 1).unwrap();
        w.write_stereo(&[0.5, -0.5, 1.0, 0.0]).unwrap();
        let b = w.finish().unwrap().1.into_inner();
        assert_eq!(b.len(), 44 + 4);
        assert_eq!(i16::from_le_bytes([b[46], b[47]]), 16384);
    }

    #[test]
    fn mux_arguments() {
        let meta = vec![(
            "creation_time".to_string(),
            "2026-11-12T21:14:05Z".to_string(),
        )];
        let a = mux_args(
            Path::new("/v/a.video.mp4"),
            Path::new("/v/a.wav"),
            &meta,
            Path::new("/v/a.mp4"),
        );
        let s = a.join(" ");
        assert!(
            s.contains("-i /v/a.video.mp4 -i /v/a.wav -map 0:v:0 -map 1:a:0 -c:v copy"),
            "{s}"
        );
        assert!(s.contains("-c:a aac -b:a 192k -ar 48000 -ac 2"), "{s}");
        assert!(
            s.contains("-metadata creation_time=2026-11-12T21:14:05Z"),
            "{s}"
        );
        assert!(s.ends_with("-movflags +faststart /v/a.mp4"), "{s}");
    }

    fn mixer() -> SharedMixer {
        Arc::new(Mutex::new(Mixer::new(48_000, BusGains::default())))
    }

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("ocw-record-{}-{name}.wav", std::process::id()))
    }

    /// Frame capture: exactly one frame's worth per captured frame; the
    /// warm-up is rendered but not written.
    #[test]
    fn exact_capture_writes_the_frames_samples() {
        let path = temp("exact");
        let cap = AudioCapture::exact(mixer(), path.clone()).unwrap();
        for f in 0..10 {
            cap.render_frame(samples_for_frame(f, 48_000, 25), f >= 4);
        }
        let n = cap.stop().wait().unwrap();
        assert_eq!(n, 6 * 1920);
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 44 + 6 * 1920 * 4);
        std::fs::remove_file(&path).unwrap();
    }

    /// Without a device the recording renders as much as game time passed.
    #[test]
    fn pull_follows_game_time() {
        let path = temp("pull");
        let mut cap = AudioCapture::live(mixer(), false, path.clone(), 10.0).unwrap();
        for f in 1..=50 {
            cap.pull(10.0 + f64::from(f) * 0.02);
        }
        assert_eq!(cap.stop().wait().unwrap(), 48_000);
        std::fs::remove_file(&path).unwrap();
    }

    /// With a device, the buffers it pulls are copied, until the stop.
    #[test]
    fn tap_copies_the_device_buffers() {
        let path = temp("tap");
        let m = mixer();
        let cap = AudioCapture::live(m.clone(), true, path.clone(), 0.0).unwrap();
        let mut buf = vec![0.0; 512 * 2];
        for _ in 0..3 {
            m.lock().unwrap().render(&mut buf, 2);
        }
        let pending = cap.stop();
        m.lock().unwrap().render(&mut buf, 2); // after the stop: not recorded
        assert_eq!(pending.wait().unwrap(), 3 * 512);
        std::fs::remove_file(&path).unwrap();
    }
}
