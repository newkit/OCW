//! Two headless riders on an `ocw-server --listen` with its bundled voice
//! server (spec §7.1.5, milestone V1 of §7.1.6). Run by
//! `scripts/test-voice-bundled.sh` (`make test-voice-bundled`), which
//! starts the game server – it starts `livekit-server` by itself, with no
//! `OCW_LIVEKIT_*` variable – and passes its address:
//!
//! - `OCW_VOICE_TEST_SERVER`: the game server's QUIC address,
//! - `OCW_VOICE_TEST_CERT`: its certificate's SHA-256,
//! - `OCW_VOICE_TEST_URL`: the voice URL the grants must name.
//!
//! Without them it passes without doing anything. Each rider joins the
//! game, asks the game server for a voice grant and joins the LiveKit room
//! with it; both must see the other, send a tone and hear the other's.
//! `OCW_VOICE_TEST_HOLD_S` keeps both in the room that long first (the
//! script checks the members through LiveKit's API).
#![cfg(feature = "livekit")]

mod support;

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use ocw_protocol::v1::{self, client_message::Msg as C, server_message::Msg as S};
use ocw_protocol::{PROTOCOL_VERSION, client_message, decode_server, encode};
use ocw_server::net::tls::parse_fingerprint;
use ocw_server::net::{ClientOptions, NetClient, ServerTrust};
use ocw_voice::backend::{BackendCommand, BackendEvent, VoiceBackend};
use ocw_voice::livekit::LiveKitBackend;
use ocw_voice::mix::{FRAME_SAMPLES, VOICE_RATE};
use support::wait_for;

fn hello(name: &str) -> Vec<u8> {
    encode(&client_message(C::Hello(v1::Hello {
        protocol_version: PROTOCOL_VERSION,
        display_name: name.into(),
        profile: Some(ocw_core::RiderProfile::default().into()),
        world: None,
        rider_weight_kg: None,
        bike_profile: None,
        look: None,
        parts: Vec::new(),
        damage: None,
        access_token: None,
    })))
}

/// Joins the game as `name` and asks for the session room's grant.
fn join_and_grant(
    addr: SocketAddr,
    trust: &ServerTrust,
    name: &str,
) -> (NetClient, v1::VoiceGrant) {
    let client = NetClient::connect(addr, "localhost", trust, ClientOptions::default()).unwrap();
    client.send(hello(name));
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut welcomed = false;
    while Instant::now() < deadline {
        for bytes in client.receive() {
            match decode_server(&bytes).unwrap() {
                S::Welcome(w) => {
                    assert!(w.voice, "{name}: the welcome offers voice");
                    welcomed = true;
                    client.send(encode(&ocw_protocol::voice_request(
                        v1::VoiceChannel::Session,
                        "",
                    )));
                }
                S::Voice(g) => {
                    assert!(welcomed);
                    return (client, g);
                }
                S::Reject(r) => panic!("{name} rejected: {}", r.reason),
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("{name}: no voice grant (welcomed: {welcomed})");
}

#[test]
fn riders_get_grants_and_hear_each_other_on_the_bundled_server() {
    let (Ok(server), Ok(cert), Ok(url)) = (
        std::env::var("OCW_VOICE_TEST_SERVER"),
        std::env::var("OCW_VOICE_TEST_CERT"),
        std::env::var("OCW_VOICE_TEST_URL"),
    ) else {
        eprintln!("skipped: run by scripts/test-voice-bundled.sh");
        return;
    };
    let addr: SocketAddr = server.parse().unwrap();
    let trust = ServerTrust::Pinned(parse_fingerprint(&cert).unwrap());
    let (_ga, ada) = join_and_grant(addr, &trust, "Ada Rider");
    let (_gb, bea) = join_and_grant(addr, &trust, "Bea Climber");
    for g in [&ada, &bea] {
        assert_eq!(g.error, "", "{g:?}");
        assert_eq!(
            g.url, url,
            "the grant names the voice server the rider reaches"
        );
        assert!(g.room.starts_with("ocw-") && !g.token.is_empty());
    }
    assert_eq!(ada.room, bea.room);
    assert_ne!(ada.identity, bea.identity);
    println!(
        "grants: {} and {} for room {} at {}",
        ada.identity, bea.identity, ada.room, ada.url
    );

    let mut a = LiveKitBackend::headless();
    let mut b = LiveKitBackend::headless();
    let (mut ea, mut eb) = (Vec::new(), Vec::new());
    let t = Duration::from_secs(20);
    let connected = |e: &BackendEvent| matches!(e, BackendEvent::Connected { .. });
    // One after the other: Bea sees Ada from her join (reported in the
    // same batch as `Connected`, kept by `wait_for`), Ada sees Bea join.
    a.send(BackendCommand::Connect {
        url: ada.url.clone(),
        token: ada.token.clone(),
    });
    assert!(
        wait_for(&mut a, &mut ea, t, connected),
        "Ada did not join the room: {ea:?}"
    );
    b.send(BackendCommand::Connect {
        url: bea.url.clone(),
        token: bea.token.clone(),
    });
    assert!(
        wait_for(&mut b, &mut eb, t, connected),
        "Bea did not join the room: {eb:?}"
    );
    println!("both riders connected to the voice room");
    // The script checks the room's members through LiveKit's server API
    // meanwhile (the launcher's keys).
    let hold = std::env::var("OCW_VOICE_TEST_HOLD_S")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0u64);
    let held = Instant::now();
    while held.elapsed() < Duration::from_secs(hold) {
        ea.extend(a.poll());
        eb.extend(b.poll());
        std::thread::sleep(Duration::from_millis(50));
    }
    let joined = |who: String| move |e: &BackendEvent| matches!(e, BackendEvent::ParticipantJoined { identity, .. } if *identity == who);
    assert!(
        wait_for(&mut b, &mut eb, t, joined(ada.identity.clone())),
        "Bea does not see Ada: {eb:?}"
    );
    assert!(
        wait_for(&mut a, &mut ea, t, joined(bea.identity.clone())),
        "Ada does not see Bea: {ea:?}"
    );
    println!("Ada and Bea see each other");

    // Both speak (tones in real time) and listen to the other at full
    // volume.
    for (backend, other) in [(&mut a, &bea.identity), (&mut b, &ada.identity)] {
        backend.send(BackendCommand::Subscribe {
            identity: other.clone(),
            on: true,
        });
        backend.send(BackendCommand::SetPlacement {
            identity: other.clone(),
            gain: 1.0,
            pan: 0.0,
        });
    }
    let tone = |phase: &mut f32, hz: f32| -> Vec<f32> {
        (0..FRAME_SAMPLES)
            .map(|_| {
                *phase += std::f32::consts::TAU * hz / VOICE_RATE as f32;
                0.3 * phase.sin()
            })
            .collect()
    };
    let energy = |out: Vec<f32>| out.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>();
    let (mut heard_a, mut heard_b) = (0.0f64, 0.0f64);
    let (mut pa, mut pb) = (0.0f32, 0.0f32);
    let start = Instant::now();
    let mut frames = 0u64;
    while start.elapsed() < Duration::from_secs(10) && (heard_a <= 50.0 || heard_b <= 50.0) {
        a.push_microphone(&tone(&mut pa, 440.0));
        b.push_microphone(&tone(&mut pb, 660.0));
        heard_a += energy(a.pull_output(FRAME_SAMPLES));
        heard_b += energy(b.pull_output(FRAME_SAMPLES));
        frames += 1;
        let due = Duration::from_millis(10 * frames);
        if let Some(wait) = due.checked_sub(start.elapsed()) {
            std::thread::sleep(wait);
        }
    }
    assert!(heard_b > 50.0, "Bea did not hear Ada (energy {heard_b:.2})");
    assert!(heard_a > 50.0, "Ada did not hear Bea (energy {heard_a:.2})");
    println!("Ada and Bea hear each other (energy {heard_a:.0} / {heard_b:.0})");
    a.send(BackendCommand::Disconnect);
    b.send(BackendCommand::Disconnect);
}
