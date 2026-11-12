//! A scripted ride rendered offline through the mixer: a test of the whole
//! chain, and a WAV to listen to.
//!
//! `OCW_AUDIO_DEMO=/path/ride.wav cargo test -p ocw-desktop audio::demo`
//! writes it (48 kHz, 16-bit stereo).

use std::time::Instant;

use ocw_world::Surface;

use super::cues::Cue;
use super::mix::Mix;
use super::mixer::{Mixer, MixerMsg};
use super::ride::{RideInput, ride_params};

const SR: u32 = 48_000;
/// Game frames per second: the ride input changes at this rate, as in the
/// app.
const FPS: u32 = 60;

/// One stretch of the demo ride: from `t0` s, speed (km/h) going from
/// `from` to `to`, cadence (0 = coasting), power, surface.
struct Stretch {
    t0: f64,
    t1: f64,
    from: f64,
    to: f64,
    cadence: f64,
    power: f64,
    surface: Surface,
}

const fn stretch(
    t0: f64,
    t1: f64,
    from: f64,
    to: f64,
    cadence: f64,
    power: f64,
    surface: Surface,
) -> Stretch {
    Stretch {
        t0,
        t1,
        from,
        to,
        cadence,
        power,
        surface,
    }
}

const RIDE: &[Stretch] = &[
    // Start and accelerate on asphalt.
    stretch(0.0, 4.0, 0.0, 34.0, 92.0, 280.0, Surface::Asphalt),
    // Cruise, then coast: the freewheel ticks as the speed falls.
    stretch(4.0, 6.0, 34.0, 34.0, 88.0, 220.0, Surface::Asphalt),
    stretch(6.0, 9.0, 34.0, 22.0, 0.0, 0.0, Surface::Asphalt),
    // Gravel, pedalling.
    stretch(9.0, 12.0, 22.0, 24.0, 85.0, 230.0, Surface::Gravel),
    // Off the road onto the grass, coasting.
    stretch(12.0, 14.0, 20.0, 12.0, 0.0, 0.0, Surface::Grass),
    // Back on asphalt and a fast descent.
    stretch(14.0, 18.0, 25.0, 62.0, 0.0, 0.0, Surface::Asphalt),
    stretch(18.0, 20.0, 62.0, 30.0, 70.0, 120.0, Surface::Asphalt),
];

/// (time s, cue).
const CUES: &[(f64, Cue)] = &[
    (0.4, Cue::UiConfirm),
    (0.7, Cue::UiMove),
    (0.9, Cue::UiMove),
    (1.1, Cue::UiValue),
    (1.4, Cue::UiBack),
    (2.0, Cue::CountdownBeep),
    (3.0, Cue::CountdownBeep),
    (4.0, Cue::CountdownBeep),
    (5.0, Cue::CountdownGo),
    (8.0, Cue::GearShift),
    (10.0, Cue::WorkoutStep),
    (19.0, Cue::Finish),
];

fn input_at(t: f64) -> RideInput {
    let s = RIDE
        .iter()
        .find(|s| t < s.t1)
        .unwrap_or(&RIDE[RIDE.len() - 1]);
    let f = ((t - s.t0) / (s.t1 - s.t0)).clamp(0.0, 1.0);
    RideInput {
        speed_mps: (s.from + (s.to - s.from) * f) / 3.6,
        cadence_rpm: Some(s.cadence),
        power_w: s.power,
        surface: s.surface,
    }
}

/// Renders the demo ride (stereo, interleaved).
pub fn render() -> Vec<f32> {
    let length = RIDE[RIDE.len() - 1].t1;
    let mut m = Mixer::new(SR, Mix::default().gains());
    let per_frame = (SR / FPS) as usize;
    let frames = (length * f64::from(FPS)) as usize;
    let mut out = vec![0.0f32; frames * per_frame * 2];
    let mut next_cue = 0;
    for (i, buf) in out.chunks_mut(per_frame * 2).enumerate() {
        let t = i as f64 / f64::from(FPS);
        m.apply(MixerMsg::Ride(ride_params(&input_at(t))));
        while next_cue < CUES.len() && CUES[next_cue].0 <= t {
            m.apply(MixerMsg::Cue(CUES[next_cue].1, Instant::now()));
            next_cue += 1;
        }
        m.render(buf, 2);
    }
    out
}

/// 16-bit PCM WAV bytes.
pub fn wav(samples: &[f32], channels: u16, sample_rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut b = Vec::with_capacity(44 + data_len as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&channels.to_le_bytes());
    b.extend_from_slice(&sample_rate.to_le_bytes());
    b.extend_from_slice(&(sample_rate * u32::from(channels) * 2).to_le_bytes());
    b.extend_from_slice(&(channels * 2).to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        b.extend_from_slice(&v.to_le_bytes());
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    /// Seconds `a`..`b` of the stereo render.
    fn span(x: &[f32], a: f64, b: f64) -> &[f32] {
        let i = |t: f64| (t * f64::from(SR)) as usize * 2;
        &x[i(a)..i(b)]
    }

    #[test]
    fn demo_ride_renders_and_optionally_writes_a_wav() {
        let out = render();
        assert!(out.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
        // Standing at the start is quieter than riding at speed.
        let start = rms(span(&out, 0.0, 0.3));
        let fast = rms(span(&out, 16.0, 18.0));
        assert!(fast > 3.0 * start, "{fast} vs {start}");
        assert!(rms(span(&out, 9.5, 11.5)) > 0.01, "gravel is heard");
        // Deterministic: the same ride sounds the same.
        assert_eq!(render()[..48_000], out[..48_000]);
        let bytes = wav(&out, 2, SR);
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(bytes.len(), 44 + out.len() * 2);
        if let Ok(path) = std::env::var("OCW_AUDIO_DEMO") {
            std::fs::write(&path, &bytes).expect("write the demo WAV");
            eprintln!("wrote {path}");
        }
    }
}
