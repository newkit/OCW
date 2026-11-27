//! Live aircraft on the wire (spec 14 Phase 18, 18E T10): the rider's
//! `LiveAirWish` and the server's `LiveAirUpdate` with its
//! `LiveAircraft` – a session number, a class, the position and motion;
//! no identities (18D *Privacy*).

use ocw_sim::air::AirKind;
use ocw_sim::air::live::LiveAircraft;

use crate::{client_message, v1};

/// `LiveAirWish { on }`.
pub fn wish(on: bool) -> v1::ClientMessage {
    client_message(v1::client_message::Msg::LiveAir(v1::LiveAirWish { on }))
}

pub fn class_to_wire(k: AirKind) -> i32 {
    (match k {
        AirKind::Airliner => v1::LiveAirClass::Airliner,
        AirKind::SmallPlane => v1::LiveAirClass::SmallPlane,
        AirKind::Helicopter => v1::LiveAirClass::Helicopter,
        AirKind::Glider => v1::LiveAirClass::Glider,
        AirKind::Balloon => v1::LiveAirClass::Balloon,
    }) as i32
}

/// A received class (unknown values from a newer peer: a small plane).
pub fn class_from_wire(v: i32) -> AirKind {
    match v1::LiveAirClass::try_from(v) {
        Ok(v1::LiveAirClass::Airliner) => AirKind::Airliner,
        Ok(v1::LiveAirClass::Helicopter) => AirKind::Helicopter,
        Ok(v1::LiveAirClass::Glider) => AirKind::Glider,
        Ok(v1::LiveAirClass::Balloon) => AirKind::Balloon,
        _ => AirKind::SmallPlane,
    }
}

/// One aircraft for the wire, its fix `fetched_ms` – `age` earlier.
pub fn aircraft_to_wire(a: &LiveAircraft, fetched_ms: i64) -> v1::LiveAircraft {
    v1::LiveAircraft {
        id: a.id,
        class: class_to_wire(a.kind),
        lat_e6: (a.lat * 1e6).round() as i32,
        lon_e6: (a.lon * 1e6).round() as i32,
        alt_m: a.alt_m.round() as i32,
        speed_dm_s: (a.speed_mps * 10.0).round().clamp(0.0, 1e6) as u32,
        track_cdeg: (a.track_deg.rem_euclid(360.0) * 100.0).round() as u32 % 36_000,
        climb_cm_s: (a.climb_mps * 100.0).round() as i32,
        age_ms: (fetched_ms - a.at_ms).clamp(0, i64::from(u32::MAX)) as u32,
    }
}

/// A received aircraft of an update fetched at `fetched_ms`.
pub fn aircraft_from_wire(w: &v1::LiveAircraft, fetched_ms: i64) -> LiveAircraft {
    LiveAircraft {
        id: w.id,
        kind: class_from_wire(w.class),
        lat: f64::from(w.lat_e6) / 1e6,
        lon: f64::from(w.lon_e6) / 1e6,
        alt_m: f64::from(w.alt_m),
        speed_mps: f64::from(w.speed_dm_s) / 10.0,
        track_deg: f64::from(w.track_cdeg) / 100.0,
        climb_mps: f64::from(w.climb_cm_s) / 100.0,
        at_ms: fetched_ms - i64::from(w.age_ms),
    }
}

/// The status of an update.
pub fn status(u: &v1::LiveAirUpdate) -> v1::LiveAirStatus {
    v1::LiveAirStatus::try_from(u.status).unwrap_or(v1::LiveAirStatus::Down)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aircraft_round_trip_within_the_wire_precision() {
        let a = LiveAircraft {
            id: 3,
            kind: AirKind::Helicopter,
            lat: 28.412_345_6,
            lon: -16.551_234_5,
            alt_m: 612.4,
            speed_mps: 61.27,
            track_deg: 359.996,
            climb_mps: -2.345,
            at_ms: 1_000_000 - 1500,
        };
        let w = aircraft_to_wire(&a, 1_000_000);
        assert_eq!(w.age_ms, 1500);
        let b = aircraft_from_wire(&w, 1_000_000);
        assert_eq!(b.id, 3);
        assert_eq!(b.kind, AirKind::Helicopter);
        assert!((b.lat - a.lat).abs() < 1e-6 && (b.lon - a.lon).abs() < 1e-6);
        assert!((b.speed_mps - a.speed_mps).abs() < 0.05);
        assert!(b.track_deg < 360.0);
        assert!((b.climb_mps - a.climb_mps).abs() < 0.006);
        assert_eq!(b.at_ms, a.at_ms);
        for k in AirKind::ALL {
            assert_eq!(class_from_wire(class_to_wire(k)), k);
        }
    }
}
