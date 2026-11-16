//! The tests' event wait keeps every event (regression of 2026-11-16, spec
//! §7.1.8): a backend reports `Connected` and the riders already in the
//! room in one `poll` batch; the old wait returned at `Connected` and
//! dropped the rest, so `livekit_two_participants` saw nobody.

mod support;

use std::time::Duration;

use ocw_voice::backend::{BackendEvent, MockBackend};
use support::wait_for;

fn joined(who: &'static str) -> impl Fn(&BackendEvent) -> bool {
    move |e| matches!(e, BackendEvent::ParticipantJoined { identity, .. } if identity == who)
}

#[test]
fn events_after_the_match_in_one_batch_are_kept() {
    let mut b = MockBackend {
        events: vec![
            BackendEvent::Connected {
                room: "ocw-test".into(),
            },
            BackendEvent::ParticipantJoined {
                identity: "rider-1".into(),
                name: "Ada Rider".into(),
            },
            BackendEvent::ParticipantMuted {
                identity: "rider-1".into(),
                muted: true,
            },
        ],
        ..Default::default()
    };
    let mut seen = Vec::new();
    let t = Duration::from_millis(50);
    assert!(wait_for(&mut b, &mut seen, t, |e| matches!(
        e,
        BackendEvent::Connected { .. }
    )));
    assert_eq!(seen.len(), 3, "the whole batch is kept: {seen:?}");
    // The join came with `Connected`: found without polling again.
    assert!(wait_for(&mut b, &mut seen, t, joined("rider-1")));
    assert!(!wait_for(&mut b, &mut seen, t, joined("rider-2")));
}

#[test]
fn events_of_later_polls_are_found() {
    let mut b = MockBackend::default();
    let mut seen = Vec::new();
    assert!(!wait_for(
        &mut b,
        &mut seen,
        Duration::from_millis(30),
        joined("rider-2")
    ));
    b.events.push(BackendEvent::ParticipantJoined {
        identity: "rider-2".into(),
        name: "Bea Climber".into(),
    });
    assert!(wait_for(
        &mut b,
        &mut seen,
        Duration::from_millis(30),
        joined("rider-2")
    ));
}
