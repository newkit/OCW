//! The session's traffic on the game server (spec 14 Phase 18, 18A, T1):
//! the server decides the road traffic of the session and sends it to
//! everyone (`Welcome.traffic`, `Snapshot.traffic`); the air seed in
//! `Welcome` lets every client simulate the same aircraft and UFOs.
//!
//! Who decides (Q2): the local servers follow their single player's
//! settings (`traffic_access` anyone); a shared session's host decides
//! road traffic for everyone (host, the default); `nobody` keeps it as the
//! server was started; while an organiser's event runs, the event's rule
//! applies (absent: no road traffic – races and every other event) and
//! nobody can change it until the event is over. Aircraft and UFOs are each
//! rider's own view setting and never reach the server.
//!
//! Nothing simulates vehicles yet (T3 / T5 read this state).

use ocw_core::RiderId;
use ocw_core::traffic::{DriverManners, RoadRules, RoadTraffic};
use ocw_protocol::traffic::TrafficInfo;
use ocw_protocol::v1;
use ocw_sim::EventPhase;

use crate::{ClockAccess, GameServer};

type S = v1::server_message::Msg;

/// The server's traffic state.
#[derive(Debug, Clone, Default)]
pub(crate) struct ServerTraffic {
    /// The session's own rule (outside events).
    pub(crate) session: RoadRules,
    /// The organiser's rule of event `id` (spec 14 Phase 18, Q2).
    pub(crate) event: Option<(u32, RoadRules)>,
    /// The session's air seed (18A).
    pub(crate) air_seed: u64,
}

/// SplitMix64: the air seed from the session's start instant.
fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl ServerTraffic {
    /// From the config; `epoch_ms`: the session's start instant (the air
    /// seed when the config has none).
    pub(crate) fn new(config: &crate::ServerConfig, epoch_ms: i64) -> Self {
        Self {
            session: RoadRules {
                level: config.road_traffic,
                manners: config.driver_manners,
            },
            event: None,
            air_seed: config
                .air_seed
                .unwrap_or_else(|| mix(epoch_ms as u64 ^ 0x0A1F_5EED)),
        }
    }
}

impl GameServer {
    /// The organiser's road traffic while its event runs (from the start
    /// until it is finished or cancelled).
    pub fn event_traffic(&self) -> Option<RoadRules> {
        let (id, r) = self.traffic.event?;
        self.session
            .event()
            .is_some_and(|e| e.id == id && e.phase != EventPhase::Finished)
            .then_some(r)
    }

    /// The road traffic in force: the running event's, else the session's.
    pub fn road_traffic(&self) -> RoadRules {
        self.event_traffic().unwrap_or(self.traffic.session)
    }

    /// Who may change the road traffic now: nobody during an organiser's
    /// event, else `ServerConfig::traffic_access`.
    pub fn traffic_access(&self) -> ClockAccess {
        if self.event_traffic().is_some() {
            ClockAccess::Nobody
        } else {
            self.config.traffic_access
        }
    }

    /// The session's air seed (`Welcome.air_seed`).
    pub fn air_seed(&self) -> u64 {
        self.traffic.air_seed
    }

    pub(crate) fn traffic_message(&self) -> v1::SessionTraffic {
        ocw_protocol::traffic::session_traffic(&TrafficInfo {
            rules: self.road_traffic(),
            access: self.traffic_access(),
            event: self.event_traffic().is_some(),
        })
    }

    /// Puts the organiser's road traffic of event `id` in force (Q2).
    pub(crate) fn set_event_traffic(&mut self, id: u32, rules: RoadRules) {
        tracing::info!(event = id, traffic = %rules.summary(), "event road traffic");
        self.traffic.event = Some((id, rules));
    }

    /// A rider's [`v1::TrafficRequest`]: checked here, on the server (6.1),
    /// like the weather's.
    pub(crate) fn traffic_request(&mut self, rider: RiderId, req: &v1::TrafficRequest) -> S {
        let result = |ok: bool, message: String| S::DevResult(v1::DevCommandResult { ok, message });
        if let Some(r) = self.event_traffic() {
            return result(
                false,
                format!(
                    "the organizer set the road traffic of this event ({}); it can be changed when the event is over",
                    r.summary()
                ),
            );
        }
        if !self.config.traffic_access.allows(Some(rider), self.host()) {
            return result(
                false,
                match self.config.traffic_access {
                    ClockAccess::Nobody => {
                        "the road traffic of this session is set by the organizer".into()
                    }
                    _ => "only the host of this shared ride can change the road traffic".into(),
                },
            );
        }
        let (road, manners) = ocw_protocol::traffic::traffic_change(req);
        if road.is_none() && manners.is_none() {
            return result(false, "empty traffic request".into());
        }
        let mut rules = self.traffic.session;
        if let Some(l) = road {
            rules.level = l;
        }
        if let Some(m) = manners {
            rules.manners = m;
        }
        self.apply_road_traffic(rules);
        tracing::info!(%rider, traffic = %rules.summary(), "road traffic changed");
        result(true, format!("road traffic: {}", describe(rules)))
    }

    /// Sets the session's road traffic (the embedding's own switch; a
    /// request is checked first, [`GameServer::traffic_request`]).
    pub fn apply_road_traffic(&mut self, rules: RoadRules) {
        self.traffic.session = rules;
    }

    /// The server's configured road traffic (`ocw-server --traffic`,
    /// `--driver-manners`; off on the public server).
    pub fn default_road_traffic(&self) -> RoadRules {
        RoadRules {
            level: self.config.road_traffic,
            manners: self.config.driver_manners,
        }
    }

    /// The last human rider has left (spec 14 Phase 18, Q2): the session's
    /// road traffic goes back to the server's configured defaults, so the
    /// next visitor does not find the host's busy roads of before. Bots and
    /// the Echo rider do not count; an organiser's event keeps its own rule.
    pub(crate) fn reset_traffic_without_humans(&mut self) {
        let humans = self
            .riders
            .keys()
            .filter(|c| !self.bot_conns.contains(c))
            .count();
        let defaults = self.default_road_traffic();
        if humans > 0 || self.traffic.session == defaults {
            return;
        }
        tracing::info!(
            was = %self.traffic.session.summary(),
            traffic = %defaults.summary(),
            "no rider left: road traffic back to the server's defaults"
        );
        self.traffic.session = defaults;
    }
}

/// "busy, drivers mostly polite" for the answer.
pub fn describe(r: RoadRules) -> String {
    match r.level {
        RoadTraffic::Off => "off".into(),
        RoadTraffic::Lunatic => "lunatic (drivers speed and overtake dangerously)".into(),
        l => format!("{}, drivers {}", l.key(), r.manners.label()),
    }
}

/// `ocw-server --traffic` (off, light, normal, busy, lunatic).
pub fn parse_level(s: &str) -> Result<RoadTraffic, String> {
    RoadTraffic::parse(s).ok_or_else(|| "off, light, normal, busy or lunatic".into())
}

/// `ocw-server --driver-manners` (polite, mostly_polite, some_realism).
pub fn parse_manners(s: &str) -> Result<DriverManners, String> {
    DriverManners::parse(s).ok_or_else(|| "polite, mostly_polite or some_realism".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ConnectionId, ServerConfig};
    use ocw_protocol::{client_message, decode_server, encode};
    use ocw_sim::EventSpec;
    use ocw_world::Route;

    fn route() -> Route {
        Route::test_loop()
    }

    fn server(access: ClockAccess) -> GameServer {
        GameServer::new(
            route(),
            ServerConfig {
                traffic_access: access,
                allow_local_events: true,
                real_epoch_ms: Some(1_764_000_000_000),
                ..Default::default()
            },
        )
    }

    fn hello(name: &str) -> Vec<u8> {
        encode(&client_message(v1::client_message::Msg::Hello(v1::Hello {
            protocol_version: ocw_protocol::PROTOCOL_VERSION,
            display_name: name.into(),
            profile: Some(ocw_core::RiderProfile::default().into()),
            ..Default::default()
        })))
    }

    /// Joins `name`; the welcome.
    fn join(s: &mut GameServer, name: &str) -> (ConnectionId, v1::Welcome) {
        let c = s.connect();
        let out = s.handle(c, &hello(name));
        let Ok(S::Welcome(w)) = decode_server(&out[0]) else {
            panic!("a welcome for {name}");
        };
        (c, w)
    }

    fn ask(s: &mut GameServer, c: ConnectionId, road: RoadTraffic) -> (bool, String) {
        let out = s.handle(
            c,
            &encode(&ocw_protocol::traffic::traffic_request(Some(road), None)),
        );
        let Ok(S::DevResult(r)) = decode_server(&out[0]) else {
            panic!("an answer");
        };
        (r.ok, r.message)
    }

    #[test]
    fn the_welcome_carries_the_traffic_and_the_air_seed() {
        let mut s = GameServer::new(
            route(),
            ServerConfig {
                road_traffic: RoadTraffic::Normal,
                driver_manners: DriverManners::MostlyPolite,
                air_seed: Some(42),
                ..Default::default()
            },
        );
        let (_, w) = join(&mut s, "Ada Rider");
        let t = ocw_protocol::traffic::traffic_info(w.traffic.as_ref().expect("sent"));
        assert_eq!(t.rules.level, RoadTraffic::Normal);
        assert_eq!(t.rules.manners, DriverManners::MostlyPolite);
        assert_eq!(t.access, ClockAccess::Host, "the default: the host");
        assert_eq!(w.air_seed, 42);
        // Without a seed: one from the start instant, the same for every
        // rider of the session, another in another session.
        let mut a = server(ClockAccess::Host);
        let (_, w1) = join(&mut a, "Ada Rider");
        let (_, w2) = join(&mut a, "Bea Climber");
        assert_eq!(w1.air_seed, w2.air_seed);
        assert_ne!(w1.air_seed, 0);
        let b = GameServer::new(
            route(),
            ServerConfig {
                real_epoch_ms: Some(1_764_000_060_000),
                ..Default::default()
            },
        );
        assert_ne!(b.air_seed(), a.air_seed());
    }

    #[test]
    fn a_shared_sessions_host_decides_for_everyone() {
        let mut s = server(ClockAccess::Host);
        let (host, _) = join(&mut s, "Ada Rider");
        let (guest, _) = join(&mut s, "Bea Climber");
        let (ok, m) = ask(&mut s, guest, RoadTraffic::Busy);
        assert!(!ok && m.contains("only the host"), "{m}");
        assert_eq!(s.road_traffic().level, RoadTraffic::Off);
        let (ok, m) = ask(&mut s, host, RoadTraffic::Busy);
        assert!(ok, "{m}");
        assert_eq!(s.road_traffic().level, RoadTraffic::Busy);
        // Everyone sees it in the next snapshot.
        let snap = (0..20)
            .flat_map(|_| s.tick())
            .find_map(|b| match decode_server(&b) {
                Ok(S::Snapshot(s)) => Some(s),
                _ => None,
            })
            .expect("a snapshot");
        let t = ocw_protocol::traffic::traffic_info(snap.traffic.as_ref().expect("sent"));
        assert_eq!(t.rules.level, RoadTraffic::Busy);
        // The manners alone: the level stays.
        let out = s.handle(
            host,
            &encode(&ocw_protocol::traffic::traffic_request(
                None,
                Some(DriverManners::SomeRealism),
            )),
        );
        assert!(matches!(decode_server(&out[0]), Ok(S::DevResult(r)) if r.ok));
        assert_eq!(
            s.road_traffic(),
            RoadRules {
                level: RoadTraffic::Busy,
                manners: DriverManners::SomeRealism
            }
        );
    }

    fn leave(s: &mut GameServer, c: ConnectionId) {
        s.disconnect(c);
    }

    fn bot_hello(name: &str) -> Vec<u8> {
        encode(&client_message(v1::client_message::Msg::Hello(v1::Hello {
            protocol_version: ocw_protocol::PROTOCOL_VERSION,
            display_name: name.into(),
            profile: Some(ocw_core::RiderProfile::default().into()),
            bot: Some(true),
            ..Default::default()
        })))
    }

    /// The host's road traffic does not outlive the people (Q2): when the
    /// last human leaves, the next visitor finds the server's defaults;
    /// while another human stays, it stays (and they become the host).
    #[test]
    fn the_hosts_traffic_ends_when_the_last_human_leaves() {
        let mut s = server(ClockAccess::Host);
        let (host, _) = join(&mut s, "Ada Rider");
        // A client bot rides along; it does not keep the session.
        let bot = s.connect();
        s.handle(bot, &bot_hello("Bot Cyclo"));
        assert!(ask(&mut s, host, RoadTraffic::Busy).0);
        leave(&mut s, host);
        assert_eq!(s.road_traffic(), s.default_road_traffic());
        let (next, w) = join(&mut s, "Bea Climber");
        let t = ocw_protocol::traffic::traffic_info(w.traffic.as_ref().unwrap());
        assert_eq!(t.rules.level, RoadTraffic::Off, "the next visitor: off");
        // The bot connected longest is the host now (the existing rule);
        // it leaves too, and the human is the host.
        assert_eq!(
            s.host(),
            s.riders.get(&bot).copied(),
            "the longest connected"
        );
        leave(&mut s, bot);
        assert_eq!(s.host(), s.riders.get(&next).copied());
        // Two humans: the one who stays keeps it and becomes the host.
        let (guest, _) = join(&mut s, "Cleo Sprinter");
        assert!(ask(&mut s, next, RoadTraffic::Busy).0);
        leave(&mut s, next);
        assert_eq!(s.road_traffic().level, RoadTraffic::Busy, "a human stays");
        assert_eq!(s.host(), s.riders.get(&guest).copied());
        assert!(ask(&mut s, guest, RoadTraffic::Light).0, "the new host");
        // A server started with traffic goes back to its own setting.
        let mut s = GameServer::new(
            route(),
            ServerConfig {
                road_traffic: RoadTraffic::Normal,
                driver_manners: DriverManners::Polite,
                ..Default::default()
            },
        );
        let (c, _) = join(&mut s, "Dora Descender");
        assert!(ask(&mut s, c, RoadTraffic::Lunatic).0);
        leave(&mut s, c);
        assert_eq!(
            s.road_traffic(),
            RoadRules {
                level: RoadTraffic::Normal,
                manners: DriverManners::Polite
            }
        );
    }

    #[test]
    fn anyone_and_nobody() {
        let mut s = server(ClockAccess::Anyone);
        join(&mut s, "Ada Rider");
        let (guest, _) = join(&mut s, "Bea Climber");
        assert!(ask(&mut s, guest, RoadTraffic::Lunatic).0);
        assert_eq!(s.road_traffic().level, RoadTraffic::Lunatic);
        let mut s = server(ClockAccess::Nobody);
        let (host, w) = join(&mut s, "Ada Rider");
        assert_eq!(
            ocw_protocol::traffic::traffic_info(w.traffic.as_ref().unwrap()).access,
            ClockAccess::Nobody
        );
        let (ok, m) = ask(&mut s, host, RoadTraffic::Light);
        assert!(!ok && m.contains("organizer"), "{m}");
        assert_eq!(s.road_traffic().level, RoadTraffic::Off);
    }

    fn start_event(s: &mut GameServer, c: ConnectionId, start: v1::EventStart) {
        let out = s.handle(
            c,
            &encode(&client_message(v1::client_message::Msg::Event(
                v1::EventRequest {
                    req: Some(v1::event_request::Req::Start(start)),
                },
            ))),
        );
        let Ok(S::DevResult(r)) = decode_server(&out[0]) else {
            panic!("an answer");
        };
        assert!(r.ok, "{}", r.message);
    }

    #[test]
    fn events_and_races_default_to_no_road_traffic_and_lock_it() {
        for kind in [
            EventSpec::Race {
                laps: None,
                length_m: Some(1000.0),
            },
            EventSpec::GroupRide { target_w: None },
            EventSpec::TimeTrial {
                laps: None,
                length_m: Some(1000.0),
            },
        ] {
            let mut s = server(ClockAccess::Anyone);
            let (c, _) = join(&mut s, "Ada Rider");
            assert!(ask(&mut s, c, RoadTraffic::Busy).0);
            start_event(&mut s, c, ocw_protocol::event_start(&kind, Some(0)));
            assert_eq!(s.road_traffic().level, RoadTraffic::Off, "{kind:?}");
            assert_eq!(s.traffic_access(), ClockAccess::Nobody);
            let (ok, m) = ask(&mut s, c, RoadTraffic::Normal);
            assert!(!ok && m.contains("organizer set the road traffic"), "{m}");
            // The event's state says so too.
            let e = s.event_state();
            assert_eq!(
                ocw_protocol::traffic::event_state_traffic(&e).level,
                RoadTraffic::Off
            );
            // Cancelled: the session's busy roads again, changeable.
            start_cancel(&mut s, c);
            assert_eq!(s.road_traffic().level, RoadTraffic::Busy);
            assert_eq!(s.traffic_access(), ClockAccess::Anyone);
        }
    }

    fn start_cancel(s: &mut GameServer, c: ConnectionId) {
        s.handle(
            c,
            &encode(&client_message(v1::client_message::Msg::Event(
                v1::EventRequest {
                    req: Some(v1::event_request::Req::Cancel(true)),
                },
            ))),
        );
    }

    #[test]
    fn the_organiser_may_give_an_event_road_traffic() {
        let mut s = server(ClockAccess::Host);
        let (c, _) = join(&mut s, "Ada Rider");
        start_event(
            &mut s,
            c,
            ocw_protocol::traffic::with_event_traffic(
                ocw_protocol::event_start(&EventSpec::GroupRide { target_w: None }, Some(0)),
                Some(RoadRules {
                    level: RoadTraffic::Light,
                    manners: DriverManners::MostlyPolite,
                }),
            ),
        );
        assert_eq!(
            s.road_traffic(),
            RoadRules {
                level: RoadTraffic::Light,
                manners: DriverManners::MostlyPolite
            }
        );
        let t = ocw_protocol::traffic::traffic_info(&s.traffic_message());
        assert!(t.event && t.access == ClockAccess::Nobody);
        let e = s.event_state();
        assert_eq!(
            ocw_protocol::traffic::event_state_traffic(&e).level,
            RoadTraffic::Light
        );
    }

    #[test]
    fn the_cli_words_parse() {
        assert_eq!(parse_level("lunatic"), Ok(RoadTraffic::Lunatic));
        assert!(parse_level("jam").is_err());
        assert_eq!(
            parse_manners("mostly_polite"),
            Ok(DriverManners::MostlyPolite)
        );
        assert_eq!(
            describe(RoadRules {
                level: RoadTraffic::Busy,
                manners: DriverManners::Polite
            }),
            "busy, drivers polite"
        );
    }
}
