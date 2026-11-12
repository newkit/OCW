//! The real output: cpal (Apache-2.0; the same version Bevy's own audio
//! links, so no second copy) on ALSA (Linux; PipeWire / PulseAudio
//! through their ALSA plugins), WASAPI (Windows) and CoreAudio (macOS).
//! Devices are listed and chosen by their name (cpal's device
//! description); the mixer renders straight into the device's buffer in
//! its native sample rate, channel count and sample format.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};

use super::engine::{OutputBackend, SharedMixer};

pub struct CpalBackend {
    host: cpal::Host,
    stream: Option<cpal::Stream>,
    failed: Arc<AtomicBool>,
}

impl Default for CpalBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl CpalBackend {
    pub fn new() -> Self {
        Self {
            host: cpal::default_host(),
            stream: None,
            failed: Arc::new(AtomicBool::new(false)),
        }
    }

    fn start(&mut self, device: &cpal::Device, mixer: &SharedMixer) -> Result<(), String> {
        let config = device.default_output_config().map_err(|e| e.to_string())?;
        let channels = usize::from(config.channels());
        if let Ok(mut m) = mixer.lock() {
            m.set_sample_rate(config.sample_rate());
        }
        let failed = Arc::new(AtomicBool::new(false));
        self.failed = failed.clone();
        let on_error = move |e: cpal::StreamError| match e {
            // A glitch, not a lost device.
            cpal::StreamError::BufferUnderrun => {}
            other => {
                tracing::warn!("audio: stream error: {other}");
                failed.store(true, Ordering::Relaxed);
            }
        };
        let sc = config.config();
        let m = mixer.clone();
        let stream = match config.sample_format() {
            SampleFormat::F32 => build::<f32>(device, &sc, m, channels, on_error),
            SampleFormat::I16 => build::<i16>(device, &sc, m, channels, on_error),
            SampleFormat::U16 => build::<u16>(device, &sc, m, channels, on_error),
            SampleFormat::I32 => build::<i32>(device, &sc, m, channels, on_error),
            other => Err(format!("unsupported sample format {other:?}")),
        }?;
        stream.play().map_err(|e| e.to_string())?;
        tracing::debug!(
            "audio: {} Hz, {channels} channels, {:?}",
            config.sample_rate(),
            config.sample_format()
        );
        self.stream = Some(stream);
        Ok(())
    }
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mixer: SharedMixer,
    channels: usize,
    on_error: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let mut scratch: Vec<f32> = Vec::new();
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                scratch.resize(data.len(), 0.0);
                // Never block the device: the engine holds the lock only
                // for a moment when it changes the sample rate.
                match mixer.try_lock() {
                    Ok(mut m) => m.render(&mut scratch, channels),
                    Err(_) => scratch.fill(0.0),
                }
                for (out, s) in data.iter_mut().zip(&scratch) {
                    *out = T::from_sample(*s);
                }
            },
            on_error,
            None,
        )
        .map_err(|e| e.to_string())
}

/// A device's name as the user sees and saves it.
fn device_name(d: &cpal::Device) -> Option<String> {
    d.description()
        .ok()
        .map(|d| d.name().trim().to_string())
        .filter(|n| !n.is_empty())
}

impl OutputBackend for CpalBackend {
    fn devices(&mut self) -> Result<Vec<String>, String> {
        let mut out: Vec<String> = Vec::new();
        for d in self.host.output_devices().map_err(|e| e.to_string())? {
            if let Some(n) = device_name(&d)
                && !out.contains(&n)
            {
                out.push(n);
            }
        }
        Ok(out)
    }

    fn default_device(&mut self) -> Option<String> {
        self.host
            .default_output_device()
            .as_ref()
            .and_then(device_name)
    }

    fn open(&mut self, name: Option<&str>, mixer: SharedMixer) -> Result<String, String> {
        self.close();
        // Several devices can share a name (ALSA lists a card as hw:,
        // plughw:, sysdefault: …): the first that opens wins.
        let candidates: Vec<cpal::Device> = match name {
            None => self.host.default_output_device().into_iter().collect(),
            Some(n) => self
                .host
                .output_devices()
                .map_err(|e| e.to_string())?
                .filter(|d| device_name(d).as_deref() == Some(n))
                .collect(),
        };
        if candidates.is_empty() {
            return Err(match name {
                Some(n) => format!("no output device {n:?}"),
                None => "no default output device".into(),
            });
        }
        let mut error = String::new();
        for d in &candidates {
            match self.start(d, &mixer) {
                Ok(()) => {
                    return Ok(device_name(d).unwrap_or_else(|| "default device".into()));
                }
                Err(e) => error = e,
            }
        }
        Err(error)
    }

    fn close(&mut self) {
        self.stream = None;
        self.failed.store(false, Ordering::Relaxed);
    }

    fn failed(&mut self) -> bool {
        self.stream.is_some() && self.failed.load(Ordering::Relaxed)
    }
}
