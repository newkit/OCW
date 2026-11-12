//! Two simulated participants against a real LiveKit server (spec §14
//! Phase 6 integration test). Network-bound, so it only runs when a dev
//! LiveKit server is configured; otherwise it passes without doing
//! anything (and says so):
//!
//! ```sh
//! livekit-server --dev            # API key "devkey", secret "secret"
//! OCW_LIVEKIT_URL=ws://127.0.0.1:7880 OCW_LIVEKIT_API_KEY=devkey \
//! OCW_LIVEKIT_API_SECRET=secret \
//!   cargo test -p ocw-voice --features livekit --test livekit_two_participants -- --nocapture
//! ```
//!
//! The tokens are minted like the game server does (`ocw_server::voice`);
//! the backends run headless (no sound card): rider 1 "speaks" a tone,
//! rider 2 must hear it, then rider 1 mutes and rider 2 sees the mute.
#![cfg(feature = "livekit")]

use std::time::{Duration, Instant};

use ocw_core::RiderId;
use ocw_server::voice::VoiceConfig;
use ocw_voice::backend::{BackendCommand, BackendEvent, VoiceBackend};
use ocw_voice::livekit::LiveKitBackend;
use ocw_voice::mix::{FRAME_SAMPLES, VOICE_RATE};

fn config() -> Option<VoiceConfig> {
    VoiceConfig::from_parts(None, None, None, |k| std::env::var(k).ok())
        .expect("complete LiveKit configuration")
}

/// Polls `b` until an event matches (collecting all events seen).
fn wait_for(
    b: &mut LiveKitBackend,
    seen: &mut Vec<BackendEvent>,
    timeout: Duration,
    what: impl Fn(&BackendEvent) -> bool,
) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        for e in b.poll() {
            let hit = what(&e);
            seen.push(e);
            if hit {
                return true;
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
fn two_participants_hear_each_other() {
    let Some(mut cfg) = config() else {
        eprintln!("skipped: set OCW_LIVEKIT_URL, OCW_LIVEKIT_API_KEY, OCW_LIVEKIT_API_SECRET");
        return;
    };
    cfg.room_prefix = "ocw-test".into();
    let room = cfg.session_room();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let id = |n| ocw_protocol::voice_identity(RiderId(n));
    let token = |n: u32| cfg.access_token(&room, &id(n), &format!("Rider {n}"), now);

    let mut a = LiveKitBackend::headless();
    let mut b = LiveKitBackend::headless();
    let (mut ea, mut eb) = (Vec::new(), Vec::new());
    for (backend, n) in [(&mut a, 1), (&mut b, 2)] {
        backend.send(BackendCommand::Connect {
            url: cfg.url.clone(),
            token: token(n),
        });
    }
    let t = Duration::from_secs(20);
    assert!(
        wait_for(&mut a, &mut ea, t, |e| matches!(
            e,
            BackendEvent::Connected { .. }
        )),
        "rider 1 did not connect: {ea:?}"
    );
    assert!(
        wait_for(&mut b, &mut eb, t, |e| matches!(
            e,
            BackendEvent::Connected { .. }
        )),
        "rider 2 did not connect: {eb:?}"
    );
    // Each sees the other (joined before or after).
    let joined = |who: String| move |e: &BackendEvent| matches!(e, BackendEvent::ParticipantJoined { identity, .. } if *identity == who);
    assert!(eb.iter().any(joined(id(1))) || wait_for(&mut b, &mut eb, t, joined(id(1))));
    assert!(ea.iter().any(joined(id(2))) || wait_for(&mut a, &mut ea, t, joined(id(2))));

    // Rider 2 listens to rider 1 at full volume.
    b.send(BackendCommand::Subscribe {
        identity: id(1),
        on: true,
    });
    b.send(BackendCommand::SetPlacement {
        identity: id(1),
        gain: 1.0,
        pan: 0.0,
    });
    // Rider 1 speaks: a 440 Hz tone in real time, while rider 2's output
    // is read at the same pace.
    let mut energy = 0.0f64;
    let mut phase = 0.0f32;
    let start = Instant::now();
    let mut frames = 0u64;
    while start.elapsed() < Duration::from_secs(8) {
        let block: Vec<f32> = (0..FRAME_SAMPLES)
            .map(|_| {
                phase += std::f32::consts::TAU * 440.0 / VOICE_RATE as f32;
                0.3 * phase.sin()
            })
            .collect();
        a.push_microphone(&block);
        let out = b.pull_output(FRAME_SAMPLES);
        energy += out.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>();
        frames += 1;
        // Pace at 10 ms per frame.
        let due = Duration::from_millis(10 * frames);
        if let Some(wait) = due.checked_sub(start.elapsed()) {
            std::thread::sleep(wait);
        }
        if energy > 50.0 {
            break;
        }
    }
    assert!(energy > 50.0, "rider 2 heard nothing (energy {energy:.2})");

    // Rider 1 mutes: rider 2 sees it (the track stays published).
    a.send(BackendCommand::SetMicMuted(true));
    let muted = |e: &BackendEvent| matches!(e, BackendEvent::ParticipantMuted { identity, muted: true } if *identity == id(1));
    assert!(wait_for(&mut b, &mut eb, t, muted), "no mute seen: {eb:?}");

    a.send(BackendCommand::Disconnect);
    let left = |e: &BackendEvent| matches!(e, BackendEvent::ParticipantLeft { identity } if *identity == id(1));
    assert!(
        wait_for(&mut b, &mut eb, t, left),
        "rider 1 did not leave: {eb:?}"
    );
    b.send(BackendCommand::Disconnect);
}
