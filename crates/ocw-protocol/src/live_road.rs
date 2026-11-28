//! Live road events on the wire (spec 14 Phase 18, 18E T13): the rider's
//! `LiveRoadWish` and the server's `LiveRoadUpdate` with its compact
//! `LiveRoadEvent`s – an edge, a direction, what, a limit; never a raw
//! record of the source (18D *What is kept and sent*).

use ocw_sim::traffic::live::{Dir, EdgeEvent, LiveKind};

use crate::{client_message, v1};

/// `LiveRoadWish { on }`.
pub fn wish(on: bool) -> v1::ClientMessage {
    client_message(v1::client_message::Msg::LiveRoad(v1::LiveRoadWish { on }))
}

pub fn kind_to_wire(k: LiveKind) -> v1::LiveRoadKind {
    match k {
        LiveKind::Closed => v1::LiveRoadKind::Closed,
        LiveKind::LaneClosed => v1::LiveRoadKind::LaneClosed,
        LiveKind::Roadworks => v1::LiveRoadKind::Roadworks,
        LiveKind::Obstruction => v1::LiveRoadKind::Obstruction,
        LiveKind::Jam => v1::LiveRoadKind::Jam,
        LiveKind::SpeedLimit => v1::LiveRoadKind::SpeedLimit,
    }
}

/// A received kind (`None`: unknown, from a newer peer – left out).
pub fn kind_from_wire(v: i32) -> Option<LiveKind> {
    Some(match v1::LiveRoadKind::try_from(v).ok()? {
        v1::LiveRoadKind::Closed => LiveKind::Closed,
        v1::LiveRoadKind::LaneClosed => LiveKind::LaneClosed,
        v1::LiveRoadKind::Roadworks => LiveKind::Roadworks,
        v1::LiveRoadKind::Obstruction => LiveKind::Obstruction,
        v1::LiveRoadKind::Jam => LiveKind::Jam,
        v1::LiveRoadKind::SpeedLimit => LiveKind::SpeedLimit,
    })
}

fn dir_to_wire(d: Dir) -> v1::LiveRoadDirection {
    match d {
        Dir::Both => v1::LiveRoadDirection::Both,
        Dir::Forward => v1::LiveRoadDirection::Forward,
        Dir::Backward => v1::LiveRoadDirection::Backward,
    }
}

fn dir_from_wire(v: i32) -> Dir {
    match v1::LiveRoadDirection::try_from(v) {
        Ok(v1::LiveRoadDirection::Forward) => Dir::Forward,
        Ok(v1::LiveRoadDirection::Backward) => Dir::Backward,
        _ => Dir::Both,
    }
}

/// One event for the wire, on road `road`, holding until `until_s`.
pub fn event_to_wire(e: &EdgeEvent, road: &str, until_s: i64) -> v1::LiveRoadEvent {
    v1::LiveRoadEvent {
        edge: e.edge,
        direction: dir_to_wire(e.dir) as i32,
        kind: kind_to_wire(e.kind) as i32,
        speed_kmh: e
            .speed_mps
            .map_or(0, |v| (v * 3.6).round().clamp(1.0, 250.0) as u32),
        road: road.chars().take(24).collect(),
        until_s,
    }
}

/// A received event (`None`: a kind this client does not know).
pub fn event_from_wire(w: &v1::LiveRoadEvent) -> Option<EdgeEvent> {
    Some(EdgeEvent {
        edge: w.edge,
        dir: dir_from_wire(w.direction),
        kind: kind_from_wire(w.kind)?,
        speed_mps: (w.speed_kmh > 0).then(|| w.speed_kmh as f32 / 3.6),
    })
}

/// The update's status (unknown values: down).
pub fn status(u: &v1::LiveRoadUpdate) -> v1::LiveRoadStatus {
    v1::LiveRoadStatus::try_from(u.status).unwrap_or(v1::LiveRoadStatus::Down)
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    #[test]
    fn events_round_trip_compactly() {
        for kind in LiveKind::ALL {
            for dir in [Dir::Both, Dir::Forward, Dir::Backward] {
                let e = EdgeEvent {
                    edge: 4711,
                    dir,
                    kind,
                    speed_mps: Some(40.0 / 3.6),
                };
                let w = event_to_wire(&e, "TF-5", 1_790_000_000);
                let back = event_from_wire(&w).expect("known");
                assert_eq!((back.edge, back.dir, back.kind), (e.edge, e.dir, e.kind));
                assert!((back.speed_mps.expect("speed") - 40.0 / 3.6).abs() < 0.2);
            }
        }
        let w = event_to_wire(
            &EdgeEvent {
                edge: 12,
                dir: Dir::Forward,
                kind: LiveKind::Closed,
                speed_mps: None,
            },
            "TF-445",
            0,
        );
        assert_eq!(event_from_wire(&w).expect("known").speed_mps, None);
        assert!(w.encoded_len() < 20, "compact: {} bytes", w.encoded_len());
        let unknown = v1::LiveRoadEvent { kind: 99, ..w };
        assert!(event_from_wire(&unknown).is_none());
    }
}
