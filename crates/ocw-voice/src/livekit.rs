//! The LiveKit voice backend (spec §7; cargo feature `livekit`).
//!
//! - **Signalling and media:** the LiveKit Rust SDK (on LiveKit's prebuilt
//!   libwebrtc) on its own thread with a small Tokio runtime. Remote voices
//!   are subscribed explicitly (auto-subscribe off: nearest-N, §7).
//! - **Audio devices:** cpal on a separate audio thread (cpal streams must
//!   stay on the thread that made them on some platforms). The microphone
//!   and the speaker are opened only while in a call.
//! - **Processing:** the received voices are mixed by [`Mixer`] (per-voice
//!   gain and pan: spatial audio) instead of the SDK's platform playout,
//!   which has no per-participant volume. Echo cancellation, noise
//!   suppression and gain control come from WebRTC's audio processing
//!   module, fed with the microphone and, as the echo reference, the mix
//!   that is played.
//! - **Mute:** the microphone track stays published and is muted (others
//!   see it muted; unmuting is instant); while muted no samples are sent.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use futures::StreamExt;
use livekit::options::TrackPublishOptions;
use livekit::prelude::*;
use livekit::webrtc::audio_source::native::NativeAudioSource;
use livekit::webrtc::audio_stream::native::NativeAudioStream;
use livekit::webrtc::native::apm::AudioProcessingModule;
use livekit::webrtc::prelude::{AudioFrame, AudioSourceOptions, RtcAudioSource};
use tokio::sync::mpsc as tmpsc;

use crate::backend::{BackendCommand, BackendEvent, VoiceBackend};
use crate::mix::{FRAME_SAMPLES, Mixer, Resampler, VOICE_RATE, to_channels, to_i16, to_mono};

/// How long a join may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Device lists are read at most this often (the menu asks every frame).
const DEVICE_CACHE: Duration = Duration::from_secs(2);

/// State shared by the network thread and the audio callbacks.
struct Shared {
    mixer: Mutex<Mixer>,
    apm: Mutex<AudioProcessingModule>,
    /// Microphone samples are sent (in a call, not muted).
    mic_live: AtomicBool,
    /// The microphone path of the current call (48 kHz mono blocks).
    capture: Mutex<Option<tmpsc::Sender<Vec<f32>>>>,
    /// No audio devices: samples come from [`LiveKitBackend::push_microphone`]
    /// and go to [`LiveKitBackend::pull_output`] (tests, servers without
    /// sound cards).
    headless: bool,
}

pub struct LiveKitBackend {
    commands: tmpsc::UnboundedSender<BackendCommand>,
    events: mpsc::Receiver<BackendEvent>,
    inputs: (Option<Instant>, Vec<String>),
    outputs: (Option<Instant>, Vec<String>),
    shared: Arc<Shared>,
}

impl Default for LiveKitBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl LiveKitBackend {
    /// The backend with the audio devices (cpal).
    pub fn new() -> Self {
        Self::build(false)
    }

    /// Without audio devices: feed the microphone with
    /// [`Self::push_microphone`], read the mix with [`Self::pull_output`].
    pub fn headless() -> Self {
        Self::build(true)
    }

    /// Headless: microphone samples (48 kHz mono, −1 … 1), sent while in a
    /// call and not muted. Returns whether a call takes them.
    pub fn push_microphone(&self, samples: &[f32]) -> bool {
        let tx = self.shared.capture.lock().ok().and_then(|c| c.clone());
        tx.is_some_and(|tx| tx.try_send(samples.to_vec()).is_ok())
    }

    /// Headless: the next `frames` of the voice mix (48 kHz, interleaved
    /// stereo).
    pub fn pull_output(&self, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0; frames * 2];
        if let Ok(mut m) = self.shared.mixer.lock() {
            m.pull_stereo(&mut out, None);
        }
        out
    }

    fn build(headless: bool) -> Self {
        let (commands, rx) = tmpsc::unbounded_channel();
        let (ev_tx, events) = mpsc::channel();
        let shared = Arc::new(Shared {
            mixer: Mutex::new(Mixer::default()),
            // Echo canceller, gain control, high-pass filter, noise suppression.
            apm: Mutex::new(AudioProcessingModule::new(true, true, true, true)),
            mic_live: AtomicBool::new(false),
            capture: Mutex::new(None),
            headless,
        });
        let worker_shared = shared.clone();
        let spawned = std::thread::Builder::new()
            .name("ocw-voice".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .thread_name("ocw-voice-rt")
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ =
                            ev_tx.send(BackendEvent::ConnectFailed(format!("voice runtime: {e}")));
                        return;
                    }
                };
                rt.block_on(Worker::new(worker_shared, ev_tx).run(rx));
            });
        if let Err(e) = spawned {
            tracing::error!(%e, "voice thread did not start");
        }
        Self {
            commands,
            events,
            inputs: (None, Vec::new()),
            outputs: (None, Vec::new()),
            shared,
        }
    }
}

fn device_names(input: bool) -> Vec<String> {
    let host = cpal::default_host();
    let devices = if input {
        host.input_devices()
    } else {
        host.output_devices()
    };
    match devices {
        Ok(d) => d
            .filter_map(|d| d.description().ok().map(|d| d.name().to_owned()))
            .collect(),
        Err(e) => {
            tracing::warn!(%e, "audio devices");
            Vec::new()
        }
    }
}

fn cached(cache: &mut (Option<Instant>, Vec<String>), input: bool) -> Vec<String> {
    if cache.0.is_none_or(|t| t.elapsed() > DEVICE_CACHE) {
        *cache = (Some(Instant::now()), device_names(input));
    }
    cache.1.clone()
}

impl VoiceBackend for LiveKitBackend {
    fn available(&self) -> Result<(), String> {
        Ok(())
    }

    fn input_devices(&mut self) -> Vec<String> {
        cached(&mut self.inputs, true)
    }

    fn output_devices(&mut self) -> Vec<String> {
        cached(&mut self.outputs, false)
    }

    fn send(&mut self, cmd: BackendCommand) {
        if self.commands.send(cmd).is_err() {
            tracing::warn!("voice thread gone");
        }
    }

    fn poll(&mut self) -> Vec<BackendEvent> {
        self.events.try_iter().collect()
    }
}

// ------------------------------------------------------------------ audio

enum AudioCmd {
    Start {
        input: Option<String>,
        output: Option<String>,
        capture: tmpsc::Sender<Vec<f32>>,
    },
    Stop,
}

/// The audio thread: owns the cpal streams.
fn audio_thread(
    shared: Arc<Shared>,
    rx: mpsc::Receiver<AudioCmd>,
    events: mpsc::Sender<BackendEvent>,
) {
    let mut streams: Vec<cpal::Stream> = Vec::new();
    for cmd in rx {
        streams.clear();
        if let AudioCmd::Start {
            input,
            output,
            capture,
        } = cmd
        {
            let host = cpal::default_host();
            match open_output(&host, output.as_deref(), shared.clone()) {
                Ok(s) => streams.push(s),
                Err(e) => {
                    let _ = events.send(BackendEvent::DeviceError(format!("speaker: {e}")));
                }
            }
            match open_input(&host, input.as_deref(), capture) {
                Ok(s) => streams.push(s),
                Err(e) => {
                    let _ = events.send(BackendEvent::DeviceError(format!("microphone: {e}")));
                }
            }
        }
    }
}

fn find_device(host: &cpal::Host, name: Option<&str>, input: bool) -> Result<cpal::Device, String> {
    let default = || {
        if input {
            host.default_input_device()
        } else {
            host.default_output_device()
        }
        .ok_or_else(|| "no default device".to_owned())
    };
    let Some(name) = name else { return default() };
    let devices = if input {
        host.input_devices()
    } else {
        host.output_devices()
    }
    .map_err(|e| e.to_string())?;
    let found = devices
        .into_iter()
        .find(|d| d.description().is_ok_and(|desc| desc.name() == name));
    match found {
        Some(d) => Ok(d),
        None => default(),
    }
}

/// A 48 kHz config if the device has one, else its default.
fn pick_config(
    supported: Vec<cpal::SupportedStreamConfigRange>,
    default: Result<cpal::SupportedStreamConfig, cpal::DefaultStreamConfigError>,
) -> Result<cpal::StreamConfig, String> {
    let f32_48k = supported.iter().find(|r| {
        r.sample_format() == cpal::SampleFormat::F32
            && r.min_sample_rate() <= VOICE_RATE
            && r.max_sample_rate() >= VOICE_RATE
    });
    if let Some(r) = f32_48k {
        return Ok(r.with_sample_rate(VOICE_RATE).config());
    }
    let d = default.map_err(|e| e.to_string())?;
    if d.sample_format() != cpal::SampleFormat::F32 {
        return Err(format!("unsupported sample format {:?}", d.sample_format()));
    }
    Ok(d.config())
}

fn open_output(
    host: &cpal::Host,
    name: Option<&str>,
    shared: Arc<Shared>,
) -> Result<cpal::Stream, String> {
    let device = find_device(host, name, false)?;
    let supported = device
        .supported_output_configs()
        .map(|c| c.collect())
        .unwrap_or_default();
    let config = pick_config(supported, device.default_output_config())?;
    let channels = usize::from(config.channels);
    let rate = config.sample_rate;
    let mut left = Resampler::new(VOICE_RATE, rate);
    let mut right = Resampler::new(VOICE_RATE, rate);
    // Stereo at the device rate, waiting to be played.
    let mut fifo: VecDeque<f32> = VecDeque::new();
    let mut block = vec![0.0f32; FRAME_SAMPLES * 2];
    let mut reference = Vec::new();
    // 10 ms chunks of the echo reference (i16, 48 kHz mono).
    let mut reverse: Vec<i16> = Vec::new();
    let (mut l_out, mut r_out) = (Vec::new(), Vec::new());
    let stream = device
        .build_output_stream(
            &config,
            move |out: &mut [f32], _| {
                let frames = out.len() / channels.max(1);
                while fifo.len() < frames * 2 {
                    if let Ok(mut m) = shared.mixer.lock() {
                        m.pull_stereo(&mut block, Some(&mut reference));
                    }
                    reverse.extend(reference.iter().map(|&s| to_i16(s)));
                    if reverse.len() >= FRAME_SAMPLES {
                        let n = reverse.len() / FRAME_SAMPLES * FRAME_SAMPLES;
                        if let Ok(mut apm) = shared.apm.lock() {
                            let _ =
                                apm.process_reverse_stream(&mut reverse[..n], VOICE_RATE as i32, 1);
                        }
                        reverse.drain(..n);
                    }
                    let l: Vec<f32> = block.iter().step_by(2).copied().collect();
                    let r: Vec<f32> = block.iter().skip(1).step_by(2).copied().collect();
                    l_out.clear();
                    r_out.clear();
                    left.process(&l, &mut l_out);
                    right.process(&r, &mut r_out);
                    for (a, b) in l_out.iter().zip(&r_out) {
                        fifo.push_back(*a);
                        fifo.push_back(*b);
                    }
                }
                let stereo: Vec<f32> = fifo.drain(..frames * 2).collect();
                to_channels(&stereo, channels, out);
            },
            |e| tracing::warn!(%e, "voice output"),
            None,
        )
        .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}

fn open_input(
    host: &cpal::Host,
    name: Option<&str>,
    capture: tmpsc::Sender<Vec<f32>>,
) -> Result<cpal::Stream, String> {
    let device = find_device(host, name, true)?;
    let supported = device
        .supported_input_configs()
        .map(|c| c.collect())
        .unwrap_or_default();
    let config = pick_config(supported, device.default_input_config())?;
    let channels = usize::from(config.channels);
    let mut resample = Resampler::new(config.sample_rate, VOICE_RATE);
    let stream = device
        .build_input_stream(
            &config,
            move |data: &[f32], _| {
                let mono = to_mono(data, channels);
                let mut out = Vec::with_capacity(mono.len() + 16);
                resample.process(&mono, &mut out);
                // Full: the sender is behind; dropping is better than delay.
                let _ = capture.try_send(out);
            },
            |e| tracing::warn!(%e, "voice input"),
            None,
        )
        .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}

// ---------------------------------------------------------------- network

struct Worker {
    shared: Arc<Shared>,
    events: mpsc::Sender<BackendEvent>,
    audio: mpsc::Sender<AudioCmd>,
    room: Option<Room>,
    track: Option<LocalAudioTrack>,
    source: Option<NativeAudioSource>,
    mic_muted: bool,
    input: Option<String>,
    output: Option<String>,
    /// Identities whose voice is wanted.
    wanted: BTreeSet<String>,
    /// Running receive tasks per identity.
    streams: BTreeMap<String, tokio::task::JoinHandle<()>>,
    capture_task: Option<tokio::task::JoinHandle<()>>,
}

impl Worker {
    fn new(shared: Arc<Shared>, events: mpsc::Sender<BackendEvent>) -> Self {
        let (audio, audio_rx) = mpsc::channel();
        let (s, e) = (shared.clone(), events.clone());
        if let Err(err) = std::thread::Builder::new()
            .name("ocw-voice-audio".into())
            .spawn(move || audio_thread(s, audio_rx, e))
        {
            tracing::error!(%err, "voice audio thread did not start");
        }
        Self {
            shared,
            events,
            audio,
            room: None,
            track: None,
            source: None,
            mic_muted: false,
            input: None,
            output: None,
            wanted: BTreeSet::new(),
            streams: BTreeMap::new(),
            capture_task: None,
        }
    }

    fn emit(&self, e: BackendEvent) {
        let _ = self.events.send(e);
    }

    async fn run(mut self, mut commands: tmpsc::UnboundedReceiver<BackendCommand>) {
        let mut room_events: Option<tmpsc::UnboundedReceiver<RoomEvent>> = None;
        loop {
            tokio::select! {
                cmd = commands.recv() => match cmd {
                    Some(cmd) => {
                        if let Some(rx) = self.command(cmd).await {
                            room_events = Some(rx);
                        }
                        if self.room.is_none() {
                            room_events = None;
                        }
                    }
                    None => break,
                },
                ev = async { room_events.as_mut()?.recv().await }, if room_events.is_some() => match ev {
                    Some(ev) => self.room_event(ev),
                    None => {
                        room_events = None;
                        if self.room.is_some() {
                            self.cleanup();
                            self.emit(BackendEvent::Disconnected { reason: "connection closed".into() });
                        }
                    }
                },
            }
        }
        self.leave().await;
    }

    async fn command(
        &mut self,
        cmd: BackendCommand,
    ) -> Option<tmpsc::UnboundedReceiver<RoomEvent>> {
        match cmd {
            BackendCommand::Connect { url, token } => return self.connect(&url, &token).await,
            BackendCommand::Disconnect => {
                self.leave().await;
                self.emit(BackendEvent::Disconnected {
                    reason: String::new(),
                });
            }
            BackendCommand::SetMicMuted(m) => {
                self.mic_muted = m;
                if let Some(t) = &self.track {
                    if m { t.mute() } else { t.unmute() }
                }
                self.shared
                    .mic_live
                    .store(self.room.is_some() && !m, Ordering::SeqCst);
            }
            BackendCommand::SetSpeakerMuted(m) => {
                if let Ok(mut mx) = self.shared.mixer.lock() {
                    mx.muted = m;
                }
            }
            BackendCommand::SetInputDevice(d) => {
                self.input = d;
                self.restart_audio();
            }
            BackendCommand::SetOutputDevice(d) => {
                self.output = d;
                self.restart_audio();
            }
            BackendCommand::Subscribe { identity, on } => {
                if on {
                    self.wanted.insert(identity.clone());
                } else {
                    self.wanted.remove(&identity);
                    if let Some(h) = self.streams.remove(&identity) {
                        h.abort();
                    }
                    if let Ok(mut mx) = self.shared.mixer.lock() {
                        mx.remove(&identity);
                    }
                }
                if let Some(room) = &self.room {
                    for p in room.remote_participants().values() {
                        if p.identity().as_str() == identity {
                            for publication in p.track_publications().values() {
                                if publication.kind() == TrackKind::Audio {
                                    publication.set_subscribed(on);
                                }
                            }
                        }
                    }
                }
            }
            BackendCommand::SetPlacement {
                identity,
                gain,
                pan,
            } => {
                if let Ok(mut mx) = self.shared.mixer.lock() {
                    mx.set_placement(&identity, gain, pan);
                }
            }
            BackendCommand::PlayCue(c) => {
                if let Ok(mut mx) = self.shared.mixer.lock() {
                    mx.play_cue(c);
                }
            }
        }
        None
    }

    async fn connect(
        &mut self,
        url: &str,
        token: &str,
    ) -> Option<tmpsc::UnboundedReceiver<RoomEvent>> {
        self.leave().await;
        let mut options = RoomOptions::default();
        options.auto_subscribe = false;
        let (room, rx) =
            match tokio::time::timeout(CONNECT_TIMEOUT, Room::connect(url, token, options)).await {
                Ok(Ok(r)) => r,
                Ok(Err(e)) => {
                    self.emit(BackendEvent::ConnectFailed(e.to_string()));
                    return None;
                }
                Err(_) => {
                    self.emit(BackendEvent::ConnectFailed("timed out".into()));
                    return None;
                }
            };
        // Processing is ours (the APM above), not the source's.
        let source = NativeAudioSource::new(
            AudioSourceOptions {
                echo_cancellation: false,
                noise_suppression: false,
                auto_gain_control: false,
            },
            VOICE_RATE,
            1,
            0,
        );
        let track = LocalAudioTrack::create_audio_track(
            "microphone",
            RtcAudioSource::Native(source.clone()),
        );
        let options = TrackPublishOptions {
            source: TrackSource::Microphone,
            dtx: true,
            ..Default::default()
        };
        if let Err(e) = room
            .local_participant()
            .publish_track(LocalTrack::Audio(track.clone()), options)
            .await
        {
            let _ = room.close().await;
            self.emit(BackendEvent::ConnectFailed(format!(
                "publishing the microphone: {e}"
            )));
            return None;
        }
        if self.mic_muted {
            track.mute();
        }
        self.emit(BackendEvent::Connected { room: room.name() });
        for p in room.remote_participants().values() {
            self.emit(BackendEvent::ParticipantJoined {
                identity: p.identity().as_str().to_owned(),
                name: p.name(),
            });
        }
        self.room = Some(room);
        self.track = Some(track);
        self.source = Some(source.clone());
        self.shared
            .mic_live
            .store(!self.mic_muted, Ordering::SeqCst);
        self.start_capture(source);
        Some(rx)
    }

    /// The microphone path: 48 kHz mono blocks from the audio thread →
    /// 10 ms frames → audio processing → the published source.
    fn start_capture(&mut self, source: NativeAudioSource) {
        let (tx, mut rx) = tmpsc::channel::<Vec<f32>>(64);
        let shared = self.shared.clone();
        self.capture_task = Some(tokio::spawn(async move {
            let mut pending: Vec<i16> = Vec::new();
            while let Some(block) = rx.recv().await {
                pending.extend(block.iter().map(|&s| to_i16(s)));
                while pending.len() >= FRAME_SAMPLES {
                    let mut frame: Vec<i16> = pending.drain(..FRAME_SAMPLES).collect();
                    if !shared.mic_live.load(Ordering::SeqCst) {
                        continue;
                    }
                    if let Ok(mut apm) = shared.apm.lock() {
                        let _ = apm.process_stream(&mut frame, VOICE_RATE as i32, 1);
                    }
                    let f = AudioFrame {
                        data: Cow::Owned(frame),
                        sample_rate: VOICE_RATE,
                        num_channels: 1,
                        samples_per_channel: FRAME_SAMPLES as u32,
                    };
                    if let Err(e) = source.capture_frame(&f).await {
                        tracing::debug!(%e, "voice frame");
                    }
                }
            }
        }));
        if let Ok(mut c) = self.shared.capture.lock() {
            *c = Some(tx.clone());
        }
        if self.shared.headless {
            return;
        }
        let _ = self.audio.send(AudioCmd::Start {
            input: self.input.clone(),
            output: self.output.clone(),
            capture: tx,
        });
    }

    fn restart_audio(&mut self) {
        if let Some(source) = self.source.clone() {
            if let Some(t) = self.capture_task.take() {
                t.abort();
            }
            self.start_capture(source);
        }
    }

    fn room_event(&mut self, ev: RoomEvent) {
        match ev {
            RoomEvent::ParticipantConnected(p) => self.emit(BackendEvent::ParticipantJoined {
                identity: p.identity().as_str().to_owned(),
                name: p.name(),
            }),
            RoomEvent::ParticipantDisconnected(p) => {
                let identity = p.identity().as_str().to_owned();
                if let Some(h) = self.streams.remove(&identity) {
                    h.abort();
                }
                if let Ok(mut mx) = self.shared.mixer.lock() {
                    mx.remove(&identity);
                }
                self.emit(BackendEvent::ParticipantLeft { identity });
            }
            RoomEvent::TrackPublished {
                publication,
                participant,
            } => {
                if publication.kind() == TrackKind::Audio
                    && self.wanted.contains(participant.identity().as_str())
                {
                    publication.set_subscribed(true);
                }
            }
            RoomEvent::TrackSubscribed {
                track: RemoteTrack::Audio(audio),
                participant,
                ..
            } => {
                let identity = participant.identity().as_str().to_owned();
                let shared = self.shared.clone();
                let id = identity.clone();
                let handle = tokio::spawn(async move {
                    let mut stream =
                        NativeAudioStream::new(audio.rtc_track(), VOICE_RATE as i32, 1);
                    while let Some(frame) = stream.next().await {
                        if let Ok(mut mx) = shared.mixer.lock() {
                            mx.push(&id, &frame.data);
                        }
                    }
                });
                if let Some(old) = self.streams.insert(identity, handle) {
                    old.abort();
                }
            }
            RoomEvent::TrackUnsubscribed { participant, .. } => {
                let identity = participant.identity().as_str().to_owned();
                if let Some(h) = self.streams.remove(&identity) {
                    h.abort();
                }
            }
            RoomEvent::TrackMuted { participant, .. } => {
                self.emit(BackendEvent::ParticipantMuted {
                    identity: participant.identity().as_str().to_owned(),
                    muted: true,
                })
            }
            RoomEvent::TrackUnmuted { participant, .. } => {
                self.emit(BackendEvent::ParticipantMuted {
                    identity: participant.identity().as_str().to_owned(),
                    muted: false,
                })
            }
            RoomEvent::ActiveSpeakersChanged { speakers } => self.emit(BackendEvent::Speaking(
                speakers
                    .iter()
                    .map(|p| p.identity().as_str().to_owned())
                    .collect(),
            )),
            RoomEvent::Reconnecting => self.emit(BackendEvent::Reconnecting),
            RoomEvent::Reconnected => self.emit(BackendEvent::Reconnected),
            RoomEvent::Disconnected { reason } => {
                self.cleanup();
                self.emit(BackendEvent::Disconnected {
                    reason: format!("{reason:?}"),
                });
            }
            _ => {}
        }
    }

    /// Drops the call's state (the room is gone or going).
    fn cleanup(&mut self) {
        self.room = None;
        self.track = None;
        self.source = None;
        self.shared.mic_live.store(false, Ordering::SeqCst);
        for (_, h) in std::mem::take(&mut self.streams) {
            h.abort();
        }
        if let Some(t) = self.capture_task.take() {
            t.abort();
        }
        self.wanted.clear();
        if let Ok(mut c) = self.shared.capture.lock() {
            *c = None;
        }
        if let Ok(mut mx) = self.shared.mixer.lock() {
            mx.clear();
        }
        let _ = self.audio.send(AudioCmd::Stop);
    }

    async fn leave(&mut self) {
        if let Some(room) = self.room.take() {
            let _ = room.close().await;
        }
        self.cleanup();
    }
}
