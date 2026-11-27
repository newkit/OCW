//! The traffic fields on the wire (spec 14 Phase 18, 18A, T1): the
//! session's road traffic in `Welcome` / `Snapshot` (`SessionTraffic`),
//! the rider's `TrafficRequest`, the organiser's rule in `EventStart` /
//! `EventState` and the air seed in `Welcome`.

use ocw_core::traffic::{DriverManners, RoadRules, RoadTraffic};

use crate::{ClockAccess, access_from_wire, access_to_wire, client_message, v1};

pub fn road_to_wire(l: RoadTraffic) -> i32 {
    (match l {
        RoadTraffic::Off => v1::RoadTrafficLevel::Off,
        RoadTraffic::Light => v1::RoadTrafficLevel::Light,
        RoadTraffic::Normal => v1::RoadTrafficLevel::Normal,
        RoadTraffic::Busy => v1::RoadTrafficLevel::Busy,
        RoadTraffic::Lunatic => v1::RoadTrafficLevel::Lunatic,
    }) as i32
}

/// A received level (unknown values from a newer peer: off).
pub fn road_from_wire(v: i32) -> RoadTraffic {
    match v1::RoadTrafficLevel::try_from(v) {
        Ok(v1::RoadTrafficLevel::Light) => RoadTraffic::Light,
        Ok(v1::RoadTrafficLevel::Normal) => RoadTraffic::Normal,
        Ok(v1::RoadTrafficLevel::Busy) => RoadTraffic::Busy,
        Ok(v1::RoadTrafficLevel::Lunatic) => RoadTraffic::Lunatic,
        _ => RoadTraffic::Off,
    }
}

pub fn manners_to_wire(m: DriverManners) -> i32 {
    (match m {
        DriverManners::Polite => v1::DriverManners::Polite,
        DriverManners::MostlyPolite => v1::DriverManners::MostlyPolite,
        DriverManners::SomeRealism => v1::DriverManners::SomeRealism,
    }) as i32
}

/// Received manners (unknown values: polite).
pub fn manners_from_wire(v: i32) -> DriverManners {
    match v1::DriverManners::try_from(v) {
        Ok(v1::DriverManners::MostlyPolite) => DriverManners::MostlyPolite,
        Ok(v1::DriverManners::SomeRealism) => DriverManners::SomeRealism,
        _ => DriverManners::Polite,
    }
}

/// The session's road traffic as the clients learn it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TrafficInfo {
    pub rules: RoadRules,
    /// Who may change it (the running event's lock already applied).
    pub access: ClockAccess,
    /// Set by the running event's organiser.
    pub event: bool,
}

pub fn session_traffic(info: &TrafficInfo) -> v1::SessionTraffic {
    v1::SessionTraffic {
        road: road_to_wire(info.rules.level),
        manners: manners_to_wire(info.rules.manners),
        access: access_to_wire(info.access),
        event: info.event,
    }
}

pub fn traffic_info(m: &v1::SessionTraffic) -> TrafficInfo {
    TrafficInfo {
        rules: RoadRules {
            level: road_from_wire(m.road),
            manners: manners_from_wire(m.manners),
        },
        access: access_from_wire(m.access),
        event: m.event,
    }
}

/// `traffic road LEVEL` / `traffic manners M` for the server.
pub fn traffic_request(
    road: Option<RoadTraffic>,
    manners: Option<DriverManners>,
) -> v1::ClientMessage {
    client_message(v1::client_message::Msg::Traffic(v1::TrafficRequest {
        road: road.map(road_to_wire),
        manners: manners.map(manners_to_wire),
    }))
}

/// What a request asks for.
pub fn traffic_change(r: &v1::TrafficRequest) -> (Option<RoadTraffic>, Option<DriverManners>) {
    (r.road.map(road_from_wire), r.manners.map(manners_from_wire))
}

/// The organiser's road traffic of an event start (Q2: absent = none, in
/// races and in every other event).
pub fn event_traffic(s: &v1::EventStart) -> RoadRules {
    RoadRules {
        level: s
            .road_traffic
            .map_or(ocw_core::traffic::EVENT_DEFAULT, road_from_wire),
        manners: s.driver_manners.map(manners_from_wire).unwrap_or_default(),
    }
}

/// `start` with the organiser's road traffic.
pub fn with_event_traffic(mut start: v1::EventStart, rules: Option<RoadRules>) -> v1::EventStart {
    if let Some(r) = rules {
        start.road_traffic = Some(road_to_wire(r.level));
        start.driver_manners = Some(manners_to_wire(r.manners));
    }
    start
}

/// The event's road traffic in its state (the event's info).
pub fn event_state_traffic(e: &v1::EventState) -> RoadRules {
    RoadRules {
        level: road_from_wire(e.road_traffic),
        manners: manners_from_wire(e.driver_manners),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{decode_client, decode_server, encode, server_message};
    use prost::Message;

    #[test]
    fn levels_and_manners_round_trip() {
        for l in RoadTraffic::ALL {
            assert_eq!(road_from_wire(road_to_wire(l)), l);
        }
        for m in DriverManners::ALL {
            assert_eq!(manners_from_wire(manners_to_wire(m)), m);
        }
        assert_eq!(road_from_wire(99), RoadTraffic::Off, "unknown: off");
        assert_eq!(manners_from_wire(99), DriverManners::Polite);
    }

    #[test]
    fn the_request_round_trips() {
        let bytes = encode(&traffic_request(
            Some(RoadTraffic::Lunatic),
            Some(DriverManners::SomeRealism),
        ));
        let Ok(v1::client_message::Msg::Traffic(r)) = decode_client(&bytes) else {
            panic!("a traffic request");
        };
        assert_eq!(
            traffic_change(&r),
            (Some(RoadTraffic::Lunatic), Some(DriverManners::SomeRealism))
        );
        // Only the manners: the level stays.
        let bytes = encode(&traffic_request(None, Some(DriverManners::Polite)));
        let Ok(v1::client_message::Msg::Traffic(r)) = decode_client(&bytes) else {
            panic!("a traffic request");
        };
        assert_eq!(traffic_change(&r), (None, Some(DriverManners::Polite)));
    }

    #[test]
    fn welcome_snapshot_and_event_fields_round_trip() {
        let info = TrafficInfo {
            rules: RoadRules {
                level: RoadTraffic::Busy,
                manners: DriverManners::MostlyPolite,
            },
            access: ClockAccess::Host,
            event: false,
        };
        let w = v1::Welcome {
            traffic: Some(session_traffic(&info)),
            air_seed: 0xC0FF_EE00_1234_5678,
            ..Default::default()
        };
        let Ok(v1::server_message::Msg::Welcome(back)) = decode_server(&encode(&server_message(
            v1::server_message::Msg::Welcome(w),
        ))) else {
            panic!("a welcome");
        };
        assert_eq!(traffic_info(back.traffic.as_ref().unwrap()), info);
        assert_eq!(back.air_seed, 0xC0FF_EE00_1234_5678);
        let s = v1::Snapshot {
            traffic: Some(session_traffic(&TrafficInfo {
                access: ClockAccess::Nobody,
                event: true,
                ..info
            })),
            ..Default::default()
        };
        let back = v1::Snapshot::decode(encode(&s).as_slice()).unwrap();
        let t = traffic_info(back.traffic.as_ref().unwrap());
        assert!(t.event && t.access == ClockAccess::Nobody);
        // An older server: no field, no traffic.
        assert_eq!(
            traffic_info(&v1::SessionTraffic::default()).rules,
            RoadRules::default()
        );
        let e = v1::EventState {
            road_traffic: road_to_wire(RoadTraffic::Light),
            driver_manners: manners_to_wire(DriverManners::SomeRealism),
            ..Default::default()
        };
        let back = v1::EventState::decode(encode(&e).as_slice()).unwrap();
        assert_eq!(
            event_state_traffic(&back),
            RoadRules {
                level: RoadTraffic::Light,
                manners: DriverManners::SomeRealism
            }
        );
    }

    #[test]
    fn events_and_races_default_to_no_road_traffic() {
        for kind in [
            v1::EventKind::GroupRide,
            v1::EventKind::Race,
            v1::EventKind::TimeTrial,
        ] {
            let s = v1::EventStart {
                kind: kind as i32,
                ..Default::default()
            };
            assert_eq!(event_traffic(&s).level, RoadTraffic::Off, "{kind:?}");
        }
        let busy = with_event_traffic(
            v1::EventStart {
                kind: v1::EventKind::Race as i32,
                ..Default::default()
            },
            Some(RoadRules {
                level: RoadTraffic::Busy,
                manners: DriverManners::Polite,
            }),
        );
        let back = v1::EventStart::decode(encode(&busy).as_slice()).unwrap();
        assert_eq!(event_traffic(&back).level, RoadTraffic::Busy);
    }
}
