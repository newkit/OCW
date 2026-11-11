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
use ocw_sim::RiderState;
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
        };
        let bytes = encode(&client_message(Msg::Sample(sample)));
        match decode_client(&bytes).unwrap() {
            Msg::Sample(s) => assert_eq!(s, sample),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn empty_message_is_rejected() {
        let bytes = encode(&v1::ClientMessage { msg: None });
        assert!(matches!(decode_client(&bytes), Err(ProtocolError::Empty)));
        assert!(decode_client(&[0xff, 0xff]).is_err());
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
    }
}
