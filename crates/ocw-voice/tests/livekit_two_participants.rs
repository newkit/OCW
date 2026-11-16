//! Two simulated participants against a real LiveKit server (spec §14
//! Phase 6 integration test, §7.1.8). Network-bound: it starts a
//! `livekit-server` of its own on free loopback ports when it finds the
//! binary (`OCW_LIVEKIT_SERVER`, `~/.local/bin/livekit-server` or `PATH`)
//! or uses the server in `OCW_LIVEKIT_URL` / `OCW_LIVEKIT_API_KEY` /
//! `OCW_LIVEKIT_API_SECRET`; with neither it passes without doing
//! anything (and says so):
//!
//! ```sh
//! CC=clang-21 CXX=clang++-21 \
//!   cargo test -p ocw-voice --features livekit --test livekit_two_participants -- --nocapture
//! ```
//!
//! `RUST_LOG=livekit=debug` shows the SDK's log. The tokens are minted
//! like the game server does (`ocw_server::voice`); the backends run
//! headless (no sound card): both riders see each other, each "speaks" a
//! tone and hears the other's, then rider 1 mutes and rider 2 sees the
//! mute, rider 1 leaves and rider 2 sees it. A second test does the same
//! through two `VoiceController`s, the desktop app's voice logic.
#![cfg(feature = "livekit")]

mod support;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ocw_core::RiderId;
use ocw_server::voice::VoiceConfig;
use ocw_voice::backend::{BackendCommand, BackendEvent, VoiceBackend};
use ocw_voice::livekit::LiveKitBackend;
use ocw_voice::mix::{FRAME_SAMPLES, VOICE_RATE};
use ocw_voice::spatial::Peer;
use ocw_voice::{MuteState, VoiceController, VoiceSettings};
use support::{LiveKitServer, wait_for};

/// The server from the environment, else one of our own.
fn server() -> Option<(VoiceConfig, Option<LiveKitServer>)> {
    let env = VoiceConfig::from_parts(None, None, None, |k| std::env::var(k).ok())
        .expect("complete LiveKit configuration");
    if let Some(cfg) = env {
        return Some((cfg, None));
    }
    let own = LiveKitServer::start()?;
    let cfg = VoiceConfig::new(own.url.clone(), own.api_key.clone(), own.api_secret.clone());
    Some((cfg, Some(own)))
}

fn tone(phase: &mut f32, hz: f32) -> Vec<f32> {
    (0..FRAME_SAMPLES)
        .map(|_| {
            *phase += std::f32::consts::TAU * hz / VOICE_RATE as f32;
            0.3 * phase.sin()
        })
        .collect()
}

#[test]
fn two_participants_hear_each_other() {
    let Some((mut cfg, own)) = server() else {
        eprintln!(
            "skipped: no livekit-server binary (OCW_LIVEKIT_SERVER, ~/.local/bin, PATH) \
             and no OCW_LIVEKIT_URL / OCW_LIVEKIT_API_KEY / OCW_LIVEKIT_API_SECRET"
        );
        return;
    };
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
    let log = own
        .as_ref()
        .map(|s| format!(" (server log {})", s.log.display()))
        .unwrap_or_default();
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
    let connected = |e: &BackendEvent| matches!(e, BackendEvent::Connected { .. });
    assert!(
        wait_for(&mut a, &mut ea, t, connected),
        "rider 1 did not connect: {ea:?}{log}"
    );
    assert!(
        wait_for(&mut b, &mut eb, t, connected),
        "rider 2 did not connect: {eb:?}{log}"
    );
    // Each sees the other (joined before or after).
    let joined = |who: String| move |e: &BackendEvent| matches!(e, BackendEvent::ParticipantJoined { identity, .. } if *identity == who);
    assert!(
        wait_for(&mut b, &mut eb, t, joined(id(1))),
        "rider 2 does not see rider 1: {eb:?}{log}"
    );
    assert!(
        wait_for(&mut a, &mut ea, t, joined(id(2))),
        "rider 1 does not see rider 2: {ea:?}{log}"
    );

    // Each listens to the other at full volume.
    for (backend, other) in [(&mut a, id(2)), (&mut b, id(1))] {
        backend.send(BackendCommand::Subscribe {
            identity: other.clone(),
            on: true,
        });
        backend.send(BackendCommand::SetPlacement {
            identity: other,
            gain: 1.0,
            pan: 0.0,
        });
    }
    // Both speak (440 / 660 Hz tones in real time) while their outputs are
    // read at the same pace.
    let (mut heard_a, mut heard_b) = (0.0f64, 0.0f64);
    let (mut pa, mut pb) = (0.0f32, 0.0f32);
    let energy = |out: Vec<f32>| out.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>();
    let start = Instant::now();
    let mut frames = 0u64;
    while start.elapsed() < Duration::from_secs(10) && (heard_a <= 50.0 || heard_b <= 50.0) {
        a.push_microphone(&tone(&mut pa, 440.0));
        b.push_microphone(&tone(&mut pb, 660.0));
        heard_a += energy(a.pull_output(FRAME_SAMPLES));
        heard_b += energy(b.pull_output(FRAME_SAMPLES));
        frames += 1;
        // Pace at 10 ms per frame.
        let due = Duration::from_millis(10 * frames);
        if let Some(wait) = due.checked_sub(start.elapsed()) {
            std::thread::sleep(wait);
        }
    }
    assert!(
        heard_b > 50.0,
        "rider 2 did not hear rider 1 (energy {heard_b:.2}){log}"
    );
    assert!(
        heard_a > 50.0,
        "rider 1 did not hear rider 2 (energy {heard_a:.2}){log}"
    );

    // Rider 1 mutes: rider 2 sees it (the track stays published).
    a.send(BackendCommand::SetMicMuted(true));
    let muted = |e: &BackendEvent| matches!(e, BackendEvent::ParticipantMuted { identity, muted: true } if *identity == id(1));
    assert!(
        wait_for(&mut b, &mut eb, t, muted),
        "no mute seen: {eb:?}{log}"
    );

    a.send(BackendCommand::Disconnect);
    let left = |e: &BackendEvent| matches!(e, BackendEvent::ParticipantLeft { identity } if *identity == id(1));
    assert!(
        wait_for(&mut b, &mut eb, t, left),
        "rider 1 did not leave: {eb:?}{log}"
    );
    b.send(BackendCommand::Disconnect);
}

/// A LiveKit backend shared between the controller that owns it and the
/// test that feeds its microphone and reads its output.
#[derive(Clone)]
struct Shared(Arc<Mutex<LiveKitBackend>>);

impl VoiceBackend for Shared {
    fn available(&self) -> Result<(), String> {
        self.0.lock().unwrap().available()
    }
    fn input_devices(&mut self) -> Vec<String> {
        Vec::new()
    }
    fn output_devices(&mut self) -> Vec<String> {
        Vec::new()
    }
    fn send(&mut self, cmd: BackendCommand) {
        self.0.lock().unwrap().send(cmd);
    }
    fn poll(&mut self) -> Vec<BackendEvent> {
        self.0.lock().unwrap().poll()
    }
}

/// The desktop app's path (spec §7.1.8): two `VoiceController`s – the
/// app's voice logic – ask for a token, join, list each other, subscribe
/// by their spatial placement and hear each other's tone.
#[test]
fn two_desktop_controllers_hear_each_other() {
    let Some((mut cfg, own)) = server() else {
        eprintln!("skipped: no LiveKit server (see two_participants_hear_each_other)");
        return;
    };
    let log = own
        .as_ref()
        .map(|s| format!(" (server log {})", s.log.display()))
        .unwrap_or_default();
    cfg.room_prefix = "ocw-test".into();
    let room = cfg.session_room();
    let riders = [RiderId(1), RiderId(2)];
    let mut sides: Vec<(VoiceController, Shared)> = riders
        .iter()
        .map(|_| {
            let b = Shared(Arc::new(Mutex::new(LiveKitBackend::headless())));
            let mut c = VoiceController::new(
                Box::new(b.clone()),
                VoiceSettings::default(),
                MuteState::default(),
            );
            c.new_session(true);
            c.set_human_riders(2);
            (c, b)
        })
        .collect();
    let mut heard = [0.0f64; 2];
    let mut phases = [0.0f32; 2];
    let start = Instant::now();
    let mut frames = 0u64;
    let sees = |c: &VoiceController, other: RiderId| {
        let who = ocw_protocol::voice_identity(other);
        c.participants().any(|p| p.identity == who)
    };
    while start.elapsed() < Duration::from_secs(30) {
        let t = start.elapsed().as_secs_f64();
        for (i, (c, b)) in sides.iter_mut().enumerate() {
            let other = riders[1 - i];
            // The other rider 5 m ahead / behind on the road.
            let peers = [Peer {
                rider: other,
                gap_m: if i == 0 { 5.0 } else { -5.0 },
                lateral_m: 0.0,
            }];
            if c.update(t, &peers).is_some() {
                // The game server's answer (`ocw_server::voice`).
                let identity = ocw_protocol::voice_identity(riders[i]);
                let unix = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs();
                let name = format!("Rider {}", i + 1);
                let grant = ocw_protocol::v1::VoiceGrant {
                    url: cfg.url.clone(),
                    token: cfg.access_token(&room, &identity, &name, unix),
                    room: room.clone(),
                    identity,
                    expires_unix_s: unix + cfg.token_ttl_s,
                    ..Default::default()
                };
                c.on_grant(&grant, t);
            }
            let hz = if i == 0 { 440.0 } else { 660.0 };
            let backend = b.0.lock().unwrap();
            backend.push_microphone(&tone(&mut phases[i], hz));
            heard[i] += backend
                .pull_output(FRAME_SAMPLES)
                .iter()
                .map(|s| f64::from(*s).powi(2))
                .sum::<f64>();
        }
        let both_see = sees(&sides[0].0, riders[1]) && sees(&sides[1].0, riders[0]);
        if both_see && heard.iter().all(|e| *e > 50.0) {
            break;
        }
        frames += 1;
        let due = Duration::from_millis(10 * frames);
        if let Some(wait) = due.checked_sub(start.elapsed()) {
            std::thread::sleep(wait);
        }
    }
    for (i, (c, _)) in sides.iter_mut().enumerate() {
        let status = c.status().text();
        let messages = c.take_messages();
        assert!(
            sees(c, riders[1 - i]),
            "rider {} does not list rider {} ({status}; {messages:?}){log}",
            i + 1,
            2 - i
        );
        assert!(
            heard[i] > 50.0,
            "rider {} did not hear rider {} (energy {:.2}; {status}; {messages:?}){log}",
            i + 1,
            2 - i,
            heard[i]
        );
    }
    for (c, _) in &mut sides {
        c.new_session(false);
    }
}
