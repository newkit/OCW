//! The echo test over a real LiveKit server (spec §7 *Echo bot*): a
//! headless participant that joins a voice room *as the Echo rider*
//! (identity `rider-<id>` of the game server's Echo rider), hears everyone
//! in the room and sends it back after a delay. The clients then hear the
//! echo through the whole real path – their microphone processing, Opus,
//! the SFU, Opus again, their spatial mix at the Echo rider's place – and
//! switch their own local loopback off while it is in the room.
//!
//! ```sh
//! ocw-server --listen --echo-bot          # logs the voice room and the Echo rider id
//! OCW_LIVEKIT_URL=… OCW_LIVEKIT_API_KEY=… OCW_LIVEKIT_API_SECRET=… \
//!   cargo run -p ocw-voice --features livekit --example echo_participant -- ROOM RIDER_ID [DELAY_MS]
//! ```
//!
//! The token is minted like the game server's (`ocw_server::voice`), so the
//! participant needs the same API key pair. Runs until killed.

use std::time::{Duration, Instant};

use ocw_core::RiderId;
use ocw_server::voice::VoiceConfig;
use ocw_voice::backend::{BackendCommand, BackendEvent, VoiceBackend};
use ocw_voice::echo::{DEFAULT_DELAY_MS, DelayLine, ECHO_NAME, clamp_delay, delay_samples};
use ocw_voice::livekit::LiveKitBackend;
use ocw_voice::mix::{FRAME_SAMPLES, VOICE_RATE, to_mono};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(room), Some(rider)) = (
        args.first(),
        args.get(1).and_then(|r| r.parse::<u32>().ok()),
    ) else {
        eprintln!("usage: echo_participant ROOM RIDER_ID [DELAY_MS]");
        std::process::exit(2);
    };
    let delay_ms = clamp_delay(
        args.get(2)
            .and_then(|d| d.parse().ok())
            .unwrap_or(DEFAULT_DELAY_MS),
    );
    let cfg = match VoiceConfig::from_parts(None, None, None, |k| std::env::var(k).ok()) {
        Ok(Some(c)) => c,
        Ok(None) => {
            eprintln!("set OCW_LIVEKIT_URL, OCW_LIVEKIT_API_KEY, OCW_LIVEKIT_API_SECRET");
            std::process::exit(2);
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let identity = ocw_protocol::voice_identity(RiderId(rider));
    let now = || {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
    };
    let mut backend = LiveKitBackend::headless();
    backend.send(BackendCommand::Connect {
        url: cfg.url.clone(),
        token: cfg.access_token(room, &identity, ECHO_NAME, now()),
    });
    let mut line = DelayLine::new(delay_samples(delay_ms));
    let start = Instant::now();
    let mut produced: u64 = 0;
    let mut delayed = Vec::new();
    eprintln!("echo participant {identity} in {room}, {delay_ms} ms");
    loop {
        for e in backend.poll() {
            match e {
                BackendEvent::ParticipantJoined { identity, name } => {
                    eprintln!("hearing {name} ({identity})");
                    backend.send(BackendCommand::Subscribe {
                        identity: identity.clone(),
                        on: true,
                    });
                    backend.send(BackendCommand::SetPlacement {
                        identity,
                        gain: 1.0,
                        pan: 0.0,
                    });
                }
                BackendEvent::ConnectFailed(e) | BackendEvent::Disconnected { reason: e } => {
                    eprintln!("voice: {e}; joining again");
                    std::thread::sleep(Duration::from_secs(2));
                    backend.send(BackendCommand::Connect {
                        url: cfg.url.clone(),
                        token: cfg.access_token(room, &identity, ECHO_NAME, now()),
                    });
                }
                other => log_event(&other),
            }
        }
        // Real time, in whole 10 ms frames: as many out as came in.
        let due = (start.elapsed().as_secs_f64() * f64::from(VOICE_RATE)) as u64;
        while produced + FRAME_SAMPLES as u64 <= due {
            // The centred stereo mix back to mono at its level (pan 0 puts
            // 1/√2 on each side).
            let heard: Vec<f32> = to_mono(&backend.pull_output(FRAME_SAMPLES), 2)
                .into_iter()
                .map(|s| s * std::f32::consts::SQRT_2)
                .collect();
            delayed.clear();
            line.process(&heard, &mut delayed);
            backend.push_microphone(&delayed);
            produced += FRAME_SAMPLES as u64;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn log_event(e: &BackendEvent) {
    if let BackendEvent::Connected { room } = e {
        eprintln!("connected to {room}");
    }
}
