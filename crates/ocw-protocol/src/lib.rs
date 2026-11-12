//! The Open Cycling World game protocol (spec §6.2).
//!
//! Messages are defined in `proto/ocw/v1/messages.proto` and generated with
//! `prost`. This crate also converts between wire types and simulation types,
//! including the quantization of rider state. It is transport-independent:
//! the in-process local server and the QUIC transport (Phase 5) carry the same
//! encoded bytes.

pub mod v1 {
    #![allow(clippy::all)]
    include!(concat!(env!("OUT_DIR"), "/ocw.v1.rs"));
}

use ocw_core::RiderId;
use ocw_sim::{Event, EventKind, EventPhase, EventSpec, RiderState, Standing, TurnChoice};
use ocw_world::graph::{EdgeId, VertexId};
use ocw_world::{RoadNetwork, Segment, TurnReason};
use prost::Message;

/// Version exchanged in `Hello` / `Welcome`.
pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("malformed message: {0}")]
    Decode(#[from] prost::DecodeError),
    #[error("message has no content")]
    Empty,
}

pub fn encode<M: Message>(msg: &M) -> Vec<u8> {
    msg.encode_to_vec()
}

pub fn decode_client(bytes: &[u8]) -> Result<v1::client_message::Msg, ProtocolError> {
    v1::ClientMessage::decode(bytes)?
        .msg
        .ok_or(ProtocolError::Empty)
}

pub fn decode_server(bytes: &[u8]) -> Result<v1::server_message::Msg, ProtocolError> {
    v1::ServerMessage::decode(bytes)?
        .msg
        .ok_or(ProtocolError::Empty)
}

pub fn client_message(msg: v1::client_message::Msg) -> v1::ClientMessage {
    v1::ClientMessage { msg: Some(msg) }
}

pub fn server_message(msg: v1::server_message::Msg) -> v1::ServerMessage {
    v1::ServerMessage { msg: Some(msg) }
}

impl From<ocw_core::RiderProfile> for v1::RiderProfile {
    fn from(p: ocw_core::RiderProfile) -> Self {
        Self {
            rider_mass_kg: p.rider_mass_kg as f32,
            bike_mass_kg: p.bike_mass_kg as f32,
            cda_m2: p.cda_m2 as f32,
            crr: p.crr as f32,
        }
    }
}

impl From<v1::RiderProfile> for ocw_core::RiderProfile {
    fn from(p: v1::RiderProfile) -> Self {
        Self {
            rider_mass_kg: f64::from(p.rider_mass_kg),
            bike_mass_kg: f64::from(p.bike_mass_kg),
            cda_m2: f64::from(p.cda_m2),
            crr: f64::from(p.crr),
            ..Self::default()
        }
    }
}

impl From<ocw_core::RiderLook> for v1::RiderLook {
    fn from(l: ocw_core::RiderLook) -> Self {
        Self {
            frame_rgb: l.frame.0,
            jersey_rgb: l.jersey.0,
            helmet_rgb: l.helmet.0,
            kit_pattern: l.pattern.wire(),
        }
    }
}

/// A look from the network, validated (spec §2.5): colours are cut to
/// 24-bit sRGB, unknown patterns are plain.
impl From<&v1::RiderLook> for ocw_core::RiderLook {
    fn from(l: &v1::RiderLook) -> Self {
        Self {
            frame: ocw_core::Rgb::from_wire(l.frame_rgb),
            jersey: ocw_core::Rgb::from_wire(l.jersey_rgb),
            helmet: ocw_core::Rgb::from_wire(l.helmet_rgb),
            pattern: ocw_core::KitPattern::from_wire(l.kit_pattern),
        }
    }
}

/// The bike (physics profile, which picks the model) and look of a
/// received rider update; `None` for the look when the server sent none.
pub fn rider_look(u: &v1::RiderUpdate) -> (ocw_core::ProfileId, Option<ocw_core::RiderLook>) {
    (
        ocw_core::ProfileId::from_wire(Some(&u.bike_profile)),
        u.look.as_ref().map(ocw_core::RiderLook::from),
    )
}

/// Quantizes a simulation snapshot entry for the wire.
pub fn quantize(s: &ocw_sim::RiderSnapshot) -> v1::RiderUpdate {
    v1::RiderUpdate {
        rider_id: s.id.0,
        distance_cm: (s.state.distance * 100.0)
            .round()
            .clamp(0.0, u32::MAX as f64) as u32,
        speed_mm_s: (s.state.speed * 1000.0).round().clamp(0.0, u32::MAX as f64) as u32,
        lateral_offset_cm: (s.state.lateral_offset * 100.0).round() as i32,
        animation_state: 0,
        last_input_seq: s.last_input_seq,
        power_w: s.power_w.round().max(0.0) as u32,
        display_name: s.name.clone(),
        bike_profile: s.bike.id().into(),
        look: Some(s.look.into()),
        heading_mrad: (s.state.heading * 1000.0).round() as i32,
        turn_curvature_e4: (s.state.turn_curvature * 1e4).round() as i32,
    }
}

/// A free-ride path change for the wire (edges by stable ID).
pub fn path_update(c: &ocw_sim::PathChange, net: &RoadNetwork) -> v1::PathUpdate {
    v1::PathUpdate {
        rider_id: c.rider.0,
        reset: c.reset,
        first_index: c.first_index,
        start_m: c.start_m,
        steps: c
            .segments
            .iter()
            .map(|s| match *s {
                Segment::Edge { edge, from_m, to_m } => v1::PathStep {
                    edge_id: net.edge(edge).id.0,
                    from_m,
                    to_m,
                    turnaround: 0,
                },
                Segment::Turnaround(r) => v1::PathStep {
                    edge_id: 0,
                    from_m: 0.0,
                    to_m: 0.0,
                    turnaround: match r {
                        TurnReason::DeadEnd => 1,
                        TurnReason::Boundary => 2,
                        TurnReason::Manual => 3,
                    },
                },
            })
            .collect(),
    }
}

/// The path segments of a received update; fails on edges this client's
/// world package does not have.
pub fn path_segments(u: &v1::PathUpdate, net: &RoadNetwork) -> Result<Vec<Segment>, String> {
    u.steps
        .iter()
        .map(|s| {
            Ok(match s.turnaround {
                0 => {
                    let edge = net
                        .edge_index(EdgeId(s.edge_id))
                        .ok_or_else(|| format!("unknown edge {}", s.edge_id))?;
                    let len = net.geometry(edge).length();
                    Segment::Edge {
                        edge,
                        from_m: s.from_m.clamp(0.0, len),
                        to_m: s.to_m.clamp(0.0, len),
                    }
                }
                1 => Segment::Turnaround(TurnReason::DeadEnd),
                2 => Segment::Turnaround(TurnReason::Boundary),
                _ => Segment::Turnaround(TurnReason::Manual),
            })
        })
        .collect()
}

/// An exit choice for the wire (`None` = no choice, the default exit).
pub fn turn_intent(
    seq: u32,
    junction: VertexId,
    choice: Option<&TurnChoice>,
    net: &RoadNetwork,
) -> v1::TurnIntent {
    v1::TurnIntent {
        seq,
        junction_id: junction.0,
        exit_edge_id: choice.map(|c| net.edge(c.edge).id.0),
        exit_forward: choice.is_some_and(|c| c.forward),
    }
}

/// The exit choice of a received intent (`None`: no choice, or an edge
/// this server does not know).
pub fn turn_choice(t: &v1::TurnIntent, net: &RoadNetwork) -> Option<TurnChoice> {
    let edge = net.edge_index(EdgeId(t.exit_edge_id?))?;
    ((t.junction_id as usize) < net.graph().vertices.len()).then_some(TurnChoice {
        junction: VertexId(t.junction_id),
        edge,
        forward: t.exit_forward,
    })
}

/// The world package identity of a road network (Hello / Welcome, spec
/// §6.5): its package name and graph fingerprint.
pub fn world_id(net: &RoadNetwork) -> v1::WorldPackageId {
    v1::WorldPackageId {
        name: net.package.clone(),
        graph_hash: net.fingerprint(),
    }
}

/// Converts a received rider update back to simulation units.
pub fn dequantize(u: &v1::RiderUpdate) -> (RiderId, RiderState) {
    (
        RiderId(u.rider_id),
        RiderState {
            distance: f64::from(u.distance_cm) / 100.0,
            speed: f64::from(u.speed_mm_s) / 1000.0,
            lateral_offset: f64::from(u.lateral_offset_cm) / 100.0,
            heading: f64::from(u.heading_mrad) / 1000.0,
            turn_curvature: f64::from(u.turn_curvature_e4) / 1e4,
        },
    )
}

/// An event request for the wire (`bots`: the field size, if given).
pub fn event_start(spec: &EventSpec, bots: Option<u32>) -> v1::EventStart {
    let len = |l: Option<f64>| l.map(|m| m.round().clamp(0.0, u32::MAX as f64) as u32);
    let (kind, laps, length_m, target_w) = match *spec {
        EventSpec::GroupRide { target_w } => (
            v1::EventKind::GroupRide,
            None,
            None,
            target_w.map(|w| w.round().max(0.0) as u32),
        ),
        EventSpec::Race { laps, length_m } => (v1::EventKind::Race, laps, len(length_m), None),
        EventSpec::TimeTrial { laps, length_m } => {
            (v1::EventKind::TimeTrial, laps, len(length_m), None)
        }
    };
    v1::EventStart {
        kind: kind as i32,
        laps,
        length_m,
        bots,
        target_w,
    }
}

/// The request of a received event start (`None`: unknown kind).
pub fn event_spec(s: &v1::EventStart) -> Option<EventSpec> {
    let length_m = s.length_m.map(f64::from);
    Some(match v1::EventKind::try_from(s.kind).ok()? {
        v1::EventKind::GroupRide => EventSpec::GroupRide {
            target_w: s.target_w.map(f64::from),
        },
        v1::EventKind::Race => EventSpec::Race {
            laps: s.laps,
            length_m,
        },
        v1::EventKind::TimeTrial => EventSpec::TimeTrial {
            laps: s.laps,
            length_m,
        },
        v1::EventKind::Unspecified => return None,
    })
}

/// The event state for the wire at server tick `tick` (`None`: no event).
pub fn event_state(ev: Option<&Event>, standings: &[Standing], tick: u64) -> v1::EventState {
    let Some(ev) = ev else {
        return v1::EventState {
            server_tick: tick,
            ..Default::default()
        };
    };
    let (kind, target_w) = match ev.kind {
        EventKind::GroupRide { target_w } => (v1::EventKind::GroupRide, target_w.round() as u32),
        EventKind::Race { .. } => (v1::EventKind::Race, 0),
        EventKind::TimeTrial { .. } => (v1::EventKind::TimeTrial, 0),
    };
    let ms = |s: f64| (s * 1000.0).round().clamp(0.0, u32::MAX as f64) as u32;
    v1::EventState {
        event_id: ev.id,
        kind: kind as i32,
        phase: match ev.phase {
            EventPhase::Scheduled => v1::EventPhase::Scheduled,
            EventPhase::Countdown => v1::EventPhase::Countdown,
            EventPhase::Running => v1::EventPhase::Running,
            EventPhase::Finished => v1::EventPhase::Finished,
        } as i32,
        countdown_tick: ev.countdown_tick,
        go_tick: ev.go_tick,
        server_tick: tick,
        start_line_m: ev.start_line_m,
        length_m: ev.kind.length_m().unwrap_or(0.0),
        laps: ev.kind.laps().unwrap_or(0),
        target_w,
        standings: standings
            .iter()
            .map(|s| v1::EventStanding {
                rider_id: s.id.0,
                display_name: s.name.clone(),
                position: s.position,
                progress_cm: (s.progress_m * 100.0)
                    .round()
                    .clamp(i32::MIN as f64, i32::MAX as f64) as i32,
                finish_ms: s.finish_s.map(ms),
                dnf: s.dnf,
                gap_ahead_ms: s.gap_ahead_s.map(ms),
            })
            .collect(),
        finished_tick: ev.finished_tick,
    }
}

/// A received event state, in simulation units.
#[derive(Debug, Clone, PartialEq)]
pub struct EventInfo {
    pub id: u32,
    pub kind: EventKind,
    pub phase: EventPhase,
    pub countdown_tick: u64,
    pub go_tick: u64,
    /// The server tick the state is from.
    pub server_tick: u64,
    pub start_line_m: f64,
    pub finished_tick: Option<u64>,
    pub standings: Vec<Standing>,
}

impl EventInfo {
    /// Seconds since GO at server tick `tick` (negative before).
    pub fn race_time_s(&self, tick: f64) -> f64 {
        (tick - self.go_tick as f64) * ocw_core::SIM_DT
    }
}

/// Decodes an event state; `None` = no event (none, or cancelled).
pub fn event_info(e: &v1::EventState) -> Option<EventInfo> {
    let phase = match v1::EventPhase::try_from(e.phase).ok()? {
        v1::EventPhase::None => return None,
        v1::EventPhase::Scheduled => EventPhase::Scheduled,
        v1::EventPhase::Countdown => EventPhase::Countdown,
        v1::EventPhase::Running => EventPhase::Running,
        v1::EventPhase::Finished => EventPhase::Finished,
    };
    let laps = (e.laps > 0).then_some(e.laps);
    let kind = match v1::EventKind::try_from(e.kind).ok()? {
        v1::EventKind::GroupRide => EventKind::GroupRide {
            target_w: f64::from(e.target_w),
        },
        v1::EventKind::Race => EventKind::Race {
            length_m: e.length_m,
            laps,
        },
        v1::EventKind::TimeTrial => EventKind::TimeTrial {
            length_m: e.length_m,
            laps,
        },
        v1::EventKind::Unspecified => return None,
    };
    Some(EventInfo {
        id: e.event_id,
        kind,
        phase,
        countdown_tick: e.countdown_tick,
        go_tick: e.go_tick,
        server_tick: e.server_tick,
        start_line_m: e.start_line_m,
        finished_tick: e.finished_tick,
        standings: e
            .standings
            .iter()
            .map(|s| Standing {
                id: RiderId(s.rider_id),
                name: s.display_name.clone(),
                position: s.position,
                progress_m: f64::from(s.progress_cm) / 100.0,
                finish_s: s.finish_ms.map(|m| f64::from(m) / 1000.0),
                dnf: s.dnf,
                gap_ahead_s: s.gap_ahead_ms.map(|m| f64::from(m) / 1000.0),
            })
            .collect(),
    })
}

/// Voice chat (spec §7): the LiveKit participant identity of a rider.
/// Server and clients derive it from the rider id, so a client maps the
/// voices it hears to riders on the road (spatial audio).
pub fn voice_identity(rider: RiderId) -> String {
    format!("rider-{}", rider.0)
}

/// The rider behind a LiveKit participant identity ([`voice_identity`]);
/// `None` for other participants.
pub fn rider_of_voice_identity(identity: &str) -> Option<RiderId> {
    let id = identity.strip_prefix("rider-")?;
    // No sign, no leading zeros: exactly the form `voice_identity` writes.
    if id.is_empty() || id.starts_with('+') || (id.len() > 1 && id.starts_with('0')) {
        return None;
    }
    id.parse().ok().map(RiderId)
}

/// Longest voice group name ([`v1::VoiceTokenRequest::group`]).
pub const MAX_VOICE_GROUP_LEN: usize = 32;

/// Checks a voice group name: 1 to [`MAX_VOICE_GROUP_LEN`] characters of
/// `a-z`, `0-9` and `-` (it becomes part of the LiveKit room name).
pub fn valid_voice_group(name: &str) -> bool {
    (1..=MAX_VOICE_GROUP_LEN).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// A voice token request (spec §7).
pub fn voice_request(channel: v1::VoiceChannel, group: &str) -> v1::ClientMessage {
    client_message(v1::client_message::Msg::Voice(v1::VoiceTokenRequest {
        channel: channel as i32,
        group: group.to_owned(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use v1::client_message::Msg;

    #[test]
    fn voice_identity_roundtrip() {
        for id in [0, 1, 42, u32::MAX] {
            let s = voice_identity(RiderId(id));
            assert_eq!(rider_of_voice_identity(&s), Some(RiderId(id)), "{s}");
        }
        for bad in [
            "",
            "rider-",
            "rider-x",
            "rider--1",
            "rider-+1",
            "rider-01",
            "bot-1",
            "rider-4294967296",
        ] {
            assert_eq!(rider_of_voice_identity(bad), None, "{bad}");
        }
    }

    #[test]
    fn voice_group_names() {
        assert!(valid_voice_group("team-7"));
        assert!(valid_voice_group(&"a".repeat(MAX_VOICE_GROUP_LEN)));
        for bad in ["", "Team", "a b", "ä", "a_b", "../x"] {
            assert!(!valid_voice_group(bad), "{bad}");
        }
        assert!(!valid_voice_group(&"a".repeat(MAX_VOICE_GROUP_LEN + 1)));
    }

    #[test]
    fn voice_request_roundtrip() {
        let bytes = encode(&voice_request(v1::VoiceChannel::Group, "team-7"));
        match decode_client(&bytes).unwrap() {
            Msg::Voice(r) => {
                assert_eq!(r.channel(), v1::VoiceChannel::Group);
                assert_eq!(r.group, "team-7");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn event_messages_roundtrip() {
        for spec in [
            EventSpec::GroupRide {
                target_w: Some(180.0),
            },
            EventSpec::Race {
                laps: Some(3),
                length_m: None,
            },
            EventSpec::TimeTrial {
                laps: None,
                length_m: Some(4200.0),
            },
            EventSpec::Race {
                laps: None,
                length_m: None,
            },
        ] {
            let start = event_start(&spec, Some(4));
            let bytes = encode(&client_message(Msg::Event(v1::EventRequest {
                req: Some(v1::event_request::Req::Start(start)),
            })));
            let Msg::Event(v1::EventRequest {
                req: Some(v1::event_request::Req::Start(back)),
            }) = decode_client(&bytes).unwrap()
            else {
                panic!()
            };
            assert_eq!(back.bots, Some(4));
            assert_eq!(event_spec(&back), Some(spec));
        }
        assert_eq!(event_spec(&v1::EventStart::default()), None);
        // The state, with standings.
        let mut ev = Event::new(
            7,
            EventKind::Race {
                length_m: 8000.0,
                laps: Some(2),
            },
            100,
            4000.0,
            vec![(RiderId(1), "Ana".into()), (RiderId(2), "Bot 1".into())],
        );
        ev.update(100, &[]);
        let standings = vec![
            Standing {
                id: RiderId(2),
                name: "Bot 1".into(),
                position: 1,
                progress_m: 8000.0,
                finish_s: Some(612.345),
                dnf: false,
                gap_ahead_s: None,
            },
            Standing {
                id: RiderId(1),
                name: "Ana".into(),
                position: 2,
                progress_m: 7990.5,
                finish_s: None,
                dnf: false,
                gap_ahead_s: Some(1.25),
            },
        ];
        let wire = event_state(Some(&ev), &standings, 123);
        let info = event_info(&wire).unwrap();
        assert_eq!(info.id, 7);
        assert_eq!(info.kind, ev.kind);
        assert_eq!(info.phase, EventPhase::Countdown);
        assert_eq!((info.go_tick, info.server_tick), (ev.go_tick, 123));
        assert_eq!(info.start_line_m, 4000.0);
        assert_eq!(info.standings, standings);
        assert_eq!(info.race_time_s(ev.go_tick as f64 + 25.0), 0.5);
        // No event (cancelled).
        assert_eq!(event_info(&event_state(None, &[], 5)), None);
        let group = Event::new(1, EventKind::GroupRide { target_w: 150.0 }, 0, 0.0, vec![]);
        let info = event_info(&event_state(Some(&group), &[], 0)).unwrap();
        assert_eq!(info.kind, EventKind::GroupRide { target_w: 150.0 });
    }

    #[test]
    fn roundtrip_client_message() {
        let sample = v1::SensorSample {
            seq: 7,
            client_time_us: 123,
            power_w: Some(250.0),
            cadence_rpm: Some(90.0),
            heart_rate_bpm: None,
            brake: Some(true),
        };
        let bytes = encode(&client_message(Msg::Sample(sample)));
        match decode_client(&bytes).unwrap() {
            Msg::Sample(s) => assert_eq!(s, sample),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn roundtrip_server_snapshot() {
        use v1::server_message::Msg as S;
        let snapshot = v1::Snapshot {
            server_tick: 4242,
            riders: vec![
                quantize(&ocw_sim::RiderSnapshot {
                    id: RiderId(1),
                    name: "Ana".into(),
                    state: RiderState {
                        distance: 3949.99,
                        speed: 12.5,
                        lateral_offset: -1.2,
                        heading: 0.123,
                        turn_curvature: -0.0345,
                    },
                    power_w: 310.0,
                    last_input_seq: 77,
                    bike: ocw_core::ProfileId::Gravel,
                    look: ocw_core::RiderLook {
                        frame: ocw_core::Rgb(0x123456),
                        pattern: ocw_core::KitPattern::Sash,
                        ..Default::default()
                    },
                }),
                quantize(&ocw_sim::RiderSnapshot {
                    id: RiderId(2),
                    name: "Bot".into(),
                    state: RiderState {
                        distance: 0.0,
                        speed: 0.0,
                        lateral_offset: 1.2,
                        ..Default::default()
                    },
                    power_w: 0.0,
                    last_input_seq: 0,
                    bike: Default::default(),
                    look: Default::default(),
                }),
            ],
            human_riders: 2,
        };
        let bytes = encode(&server_message(S::Snapshot(snapshot.clone())));
        match decode_server(&bytes).unwrap() {
            S::Snapshot(s) => {
                assert_eq!(s, snapshot);
                let (id, st) = dequantize(&s.riders[0]);
                assert_eq!(id, RiderId(1));
                assert!((st.distance - 3949.99).abs() < 1e-9);
                assert!((st.lateral_offset + 1.2).abs() < 1e-9);
                assert_eq!(s.riders[0].last_input_seq, 77);
                let (bike, look) = rider_look(&s.riders[0]);
                assert_eq!(bike, ocw_core::ProfileId::Gravel);
                let look = look.unwrap();
                assert_eq!(look.frame, ocw_core::Rgb(0x123456));
                assert_eq!(look.pattern, ocw_core::KitPattern::Sash);
                assert_eq!(rider_look(&s.riders[1]).0, ocw_core::ProfileId::Road);
            }
            other => panic!("unexpected {other:?}"),
        }
        let welcome = v1::Welcome {
            protocol_version: PROTOCOL_VERSION,
            rider_id: 9,
            tick_rate_hz: 50,
            broadcast_rate_hz: 10,
            route_id: "test-loop".into(),
            free_ride: false,
            world: Some(v1::WorldPackageId {
                name: "Liechtenstein".into(),
                graph_hash: 0x0123_4567_89ab_cdef,
            }),
            graph_route: "Rheintal".into(),
            voice: true,
        };
        let bytes = encode(&server_message(S::Welcome(welcome.clone())));
        assert_eq!(decode_server(&bytes).unwrap(), S::Welcome(welcome));
    }

    #[test]
    fn quantization_clamps_out_of_range_values() {
        let u = quantize(&ocw_sim::RiderSnapshot {
            id: RiderId(1),
            name: String::new(),
            state: RiderState {
                distance: -3.0,
                speed: -1.0,
                lateral_offset: 0.004,
                ..Default::default()
            },
            power_w: -5.0,
            last_input_seq: 0,
            bike: Default::default(),
            look: Default::default(),
        });
        assert_eq!((u.distance_cm, u.speed_mm_s, u.power_w), (0, 0, 0));
        assert_eq!(u.lateral_offset_cm, 0);
    }

    #[test]
    fn empty_message_is_rejected() {
        let bytes = encode(&v1::ClientMessage { msg: None });
        assert!(matches!(decode_client(&bytes), Err(ProtocolError::Empty)));
        assert!(decode_client(&[0xff, 0xff]).is_err());
        let bytes = encode(&v1::ServerMessage { msg: None });
        assert!(matches!(decode_server(&bytes), Err(ProtocolError::Empty)));
    }

    #[test]
    fn free_ride_paths_and_turns_round_trip() {
        use ocw_world::graph::{Edge, Oneway, RoadClass, RoadGraph};
        let edge = |id: u64, a: u32, b: u32, p: [[f64; 2]; 2]| Edge {
            id: EdgeId(id),
            from: VertexId(a),
            to: VertexId(b),
            points: p.to_vec(),
            class: RoadClass::Residential,
            name: None,
            width_m: 5.0,
            bridge: false,
            tunnel: false,
            dead_end_m: None,
            boundary: false,
            oneway: Oneway::No,
            osm_way: 1,
            profile: Vec::new(),
        };
        let net = RoadNetwork::new(RoadGraph {
            vertices: vec![[0.0, 0.0], [100.0, 0.0], [100.0, 80.0]],
            edges: vec![
                edge(u64::MAX - 7, 0, 1, [[0.0, 0.0], [100.0, 0.0]]),
                edge(42, 1, 2, [[100.0, 0.0], [100.0, 80.0]]),
            ],
            boundary_vertices: Default::default(),
        });
        let change = ocw_sim::PathChange {
            rider: RiderId(4),
            reset: true,
            first_index: 17,
            start_m: 1234.5,
            segments: vec![
                Segment::Edge {
                    edge: 0,
                    from_m: 12.25,
                    to_m: 100.0,
                },
                Segment::Edge {
                    edge: 1,
                    from_m: 0.0,
                    to_m: 70.0,
                },
                Segment::Turnaround(TurnReason::Manual),
                Segment::Edge {
                    edge: 1,
                    from_m: 70.0,
                    to_m: 0.0,
                },
            ],
        };
        let u = path_update(&change, &net);
        assert_eq!(u.steps[0].edge_id, u64::MAX - 7);
        let bytes = encode(&server_message(v1::server_message::Msg::Path(u.clone())));
        let v1::server_message::Msg::Path(back) = decode_server(&bytes).unwrap() else {
            panic!()
        };
        assert_eq!(back, u);
        assert_eq!(path_segments(&back, &net).unwrap(), change.segments);
        // Unknown edges are refused.
        let mut bad = back.clone();
        bad.steps[1].edge_id = 999;
        assert!(path_segments(&bad, &net).unwrap_err().contains("999"));
        // Turn intents.
        let c = TurnChoice {
            junction: VertexId(1),
            edge: 1,
            forward: true,
        };
        let t = turn_intent(3, VertexId(1), Some(&c), &net);
        assert_eq!(t.exit_edge_id, Some(42));
        assert_eq!(turn_choice(&t, &net), Some(c));
        let none = turn_intent(4, VertexId(1), None, &net);
        assert_eq!(none.exit_edge_id, None);
        assert_eq!(turn_choice(&none, &net), None);
    }

    #[test]
    fn quantization_keeps_centimetres() {
        let snap = ocw_sim::RiderSnapshot {
            id: RiderId(3),
            name: "x".into(),
            state: RiderState {
                distance: 1234.567,
                speed: 9.8765,
                lateral_offset: -1.234,
                heading: -0.2004,
                turn_curvature: 0.01234,
            },
            power_w: 201.6,
            last_input_seq: 9,
            bike: Default::default(),
            look: Default::default(),
        };
        let u = quantize(&snap);
        assert_eq!(u.power_w, 202);
        let (id, s) = dequantize(&u);
        assert_eq!(id, RiderId(3));
        assert!((s.distance - 1234.57).abs() < 1e-9);
        assert!((s.speed - 9.877).abs() < 1e-9);
        assert!((s.lateral_offset + 1.23).abs() < 1e-9);
        assert!((s.heading + 0.2).abs() < 1e-9);
        assert!((s.turn_curvature - 0.0123).abs() < 1e-9);
    }

    /// Looks from the network are validated: colours to 24 bits, unknown
    /// patterns plain, unknown bikes the default profile.
    #[test]
    fn looks_from_the_wire_are_validated() {
        let wire = v1::RiderLook {
            frame_rgb: 0xFF00_00FF,
            jersey_rgb: 0x00AB_CDEF,
            helmet_rgb: 0,
            kit_pattern: 77,
        };
        let l = ocw_core::RiderLook::from(&wire);
        assert_eq!(l.frame, ocw_core::Rgb(0x0000FF));
        assert_eq!(l.jersey, ocw_core::Rgb(0xABCDEF));
        assert_eq!(l.pattern, ocw_core::KitPattern::Plain);
        let u = v1::RiderUpdate {
            bike_profile: "hovercraft".into(),
            ..Default::default()
        };
        assert_eq!(rider_look(&u), (ocw_core::ProfileId::Road, None));
        let back = v1::RiderLook::from(ocw_core::RiderLook::default());
        assert_eq!(
            ocw_core::RiderLook::from(&back),
            ocw_core::RiderLook::default()
        );
    }
}
