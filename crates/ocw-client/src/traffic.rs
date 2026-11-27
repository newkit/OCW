//! The client's side of traffic (spec 14 Phase 18, 18A, T1): the
//! session's road traffic as the server decided it (Welcome, Snapshot), the
//! running event's rule (EventState), the session's air seed, and whether
//! this rider may change the road traffic. Aircraft and UFOs are the
//! rider's own view settings (`traffic.air`, `traffic.ufo`): they never go
//! to the server. Nothing draws vehicles yet; the next milestones read
//! this state.

use ocw_core::RiderId;
use ocw_core::traffic::RoadRules;
use ocw_protocol::traffic::{TrafficInfo, traffic_info};
use ocw_protocol::{ClockAccess, v1};

/// Traffic state of the client.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TrafficClient {
    /// The server's word (`None`: not joined yet, or an older server).
    info: Option<TrafficInfo>,
    air_seed: u64,
}

impl TrafficClient {
    pub fn on_welcome(&mut self, w: &v1::Welcome) {
        self.info = w.traffic.as_ref().map(traffic_info);
        self.air_seed = w.air_seed;
    }

    pub fn on_snapshot(&mut self, s: &v1::Snapshot) {
        if let Some(t) = &s.traffic {
            self.info = Some(traffic_info(t));
        }
    }

    /// The session's road traffic in force (none before the welcome).
    pub fn road(&self) -> RoadRules {
        self.info.map(|i| i.rules).unwrap_or_default()
    }

    /// What the server said, if anything.
    pub fn info(&self) -> Option<TrafficInfo> {
        self.info
    }

    /// The session's air seed (`Welcome.air_seed`).
    pub fn air_seed(&self) -> u64 {
        self.air_seed
    }

    /// Set by the running event's organiser.
    pub fn by_event(&self) -> bool {
        self.info.is_some_and(|i| i.event)
    }

    /// Whether `rider` may change the road traffic (`host`: the session's
    /// host; the server checks again). Before the welcome: yes (it applies
    /// to the next ride).
    pub fn may_change(&self, rider: Option<RiderId>, host: Option<RiderId>) -> bool {
        self.info
            .is_none_or(|i| i.access.allows(rider, host) && !i.event)
    }

    /// Who decides, for the menu and the console: "you", "the host",
    /// "the organiser".
    pub fn decided_by(&self, rider: Option<RiderId>, host: Option<RiderId>) -> &'static str {
        match self.info {
            None => "you",
            Some(i) if i.event => "the event's organiser",
            Some(i) => match i.access {
                ClockAccess::Nobody => "the organiser",
                a if a.allows(rider, host) => "you",
                _ => "the host",
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_core::traffic::{DriverManners, RoadTraffic};
    use ocw_protocol::traffic::session_traffic;

    fn info(level: RoadTraffic, access: ClockAccess, event: bool) -> v1::SessionTraffic {
        session_traffic(&TrafficInfo {
            rules: RoadRules {
                level,
                manners: DriverManners::MostlyPolite,
            },
            access,
            event,
        })
    }

    #[test]
    fn follows_the_welcome_and_the_snapshots() {
        let mut t = TrafficClient::default();
        assert_eq!(t.road().level, RoadTraffic::Off);
        assert!(t.may_change(None, None), "before a ride: the next one");
        t.on_welcome(&v1::Welcome {
            traffic: Some(info(RoadTraffic::Normal, ClockAccess::Host, false)),
            air_seed: 77,
            ..Default::default()
        });
        assert_eq!(t.road().level, RoadTraffic::Normal);
        assert_eq!(t.air_seed(), 77);
        let (me, host) = (Some(RiderId(2)), Some(RiderId(1)));
        assert!(!t.may_change(me, host));
        assert_eq!(t.decided_by(me, host), "the host");
        assert!(t.may_change(host, host));
        assert_eq!(t.decided_by(host, host), "you");
        // A snapshot without the field (an older server) keeps it.
        t.on_snapshot(&v1::Snapshot::default());
        assert_eq!(t.road().level, RoadTraffic::Normal);
        t.on_snapshot(&v1::Snapshot {
            traffic: Some(info(RoadTraffic::Off, ClockAccess::Nobody, true)),
            ..Default::default()
        });
        assert!(t.by_event() && !t.may_change(host, host));
        assert_eq!(t.decided_by(host, host), "the event's organiser");
        // An older server's welcome: no traffic, the rider decides.
        t.on_welcome(&v1::Welcome::default());
        assert_eq!(t.road(), RoadRules::default());
        assert_eq!(t.air_seed(), 0);
    }
}
