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
use ocw_sim::{RiderState, TurnChoice};
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
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use v1::client_message::Msg;

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
                }),
            ],
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
}
