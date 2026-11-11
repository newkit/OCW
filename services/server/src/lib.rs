//! Game server logic (spec §6.1).
//!
//! [`GameServer`] is transport-independent: it consumes and produces encoded
//! protocol messages. The same type runs
//! - inside the desktop app as the **local server** ([`local::LocalServer`]), and
//! - as the headless **remote server** binary (`ocw-server`), reachable over
//!   QUIC with [`net`] (`--listen`, spec §6.2).

pub mod local;
pub mod net;

use std::collections::BTreeMap;
use std::sync::Arc;

use ocw_core::{BROADCAST_HZ, RiderId, SIM_TICK_HZ, TICKS_PER_BROADCAST};
use ocw_protocol::v1::{self, client_message::Msg as C, server_message::Msg as S};
use ocw_protocol::{PROTOCOL_VERSION, decode_client, encode, server_message};
use ocw_sim::{FreeRideStart, RiderInput, Session};
use ocw_world::{RoadNetwork, Route, RoutePlan};

/// What a session rides: a fixed route, free ride on the road graph, or a
/// route on the road graph (followed with the free-ride machinery, spec
/// §2.7).
#[derive(Debug, Clone)]
pub enum Course {
    Route(Route),
    FreeRide {
        net: Arc<RoadNetwork>,
        start: FreeRideStart,
    },
    RouteRide {
        net: Arc<RoadNetwork>,
        plan: Arc<RoutePlan>,
    },
}

pub use ocw_protocol::world_id;

impl From<Route> for Course {
    fn from(r: Route) -> Self {
        Self::Route(r)
    }
}

/// Identifies one client connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConnectionId(pub u64);

#[derive(Debug, Clone, Default)]
pub struct ServerConfig {
    /// Accept [`v1::DevCommand`]s. Only true for the in-process local server (spec §2.9).
    pub allow_dev_commands: bool,
    /// Number of server-side test riders.
    pub bots: u32,
    /// Where on the route everyone starts, in metres.
    pub start_m: f64,
}

/// Authoritative server for one session.
pub struct GameServer {
    config: ServerConfig,
    session: Session,
    /// Connection → rider, once the client said hello.
    riders: BTreeMap<ConnectionId, RiderId>,
    next_connection: u64,
}

/// Base power of the bots, from easy riders to strong ones, so they spread
/// out on the road instead of riding as one bunch: twelve distinct
/// strengths from 85 to 280 W, easy and strong ones interleaved so even a
/// small field mixes them.
pub const BOT_POWERS_W: [f64; 12] = [
    210.0, 110.0, 160.0, 85.0, 250.0, 135.0, 185.0, 95.0, 280.0, 120.0, 230.0, 145.0,
];

/// Base power of the `i`-th bot.
pub fn bot_power_w(i: u32) -> f64 {
    BOT_POWERS_W[i as usize % BOT_POWERS_W.len()]
}

impl GameServer {
    /// A session on `course` (a [`Route`] converts into one).
    pub fn new(course: impl Into<Course>, config: ServerConfig) -> Self {
        let mut session = match course.into() {
            Course::Route(route) => Session::new(route).with_start(config.start_m),
            Course::FreeRide { net, start } => Session::free_ride(net, start),
            Course::RouteRide { net, plan } => Session::route_ride(net, plan),
        };
        for i in 0..config.bots {
            session.add_bot(format!("Bot {}", i + 1), bot_power_w(i));
        }
        Self {
            config,
            session,
            riders: BTreeMap::new(),
            next_connection: 1,
        }
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    /// Registers a new connection.
    pub fn connect(&mut self) -> ConnectionId {
        let id = ConnectionId(self.next_connection);
        self.next_connection += 1;
        id
    }

    /// Removes a connection and its rider.
    pub fn disconnect(&mut self, conn: ConnectionId) {
        if let Some(rider) = self.riders.remove(&conn) {
            self.session.remove_rider(rider);
            tracing::info!(%rider, "rider left");
        }
    }

    /// Handles one encoded client message. Returns encoded replies for this connection.
    pub fn handle(&mut self, conn: ConnectionId, bytes: &[u8]) -> Vec<Vec<u8>> {
        let msg = match decode_client(bytes) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(?conn, %e, "dropping malformed message");
                return Vec::new();
            }
        };
        let had_rider = self.riders.contains_key(&conn);
        let reply = match (msg, self.riders.get(&conn).copied()) {
            (C::Hello(hello), None) => Some(self.hello(conn, hello)),
            (C::Hello(_), Some(_)) => None,
            (C::Sample(sample), Some(rider)) => {
                let input = RiderInput {
                    power_w: sample.power_w.map(f64::from).unwrap_or(0.0),
                    cadence_rpm: sample.cadence_rpm.map(f64::from),
                    heart_rate_bpm: sample.heart_rate_bpm.map(f64::from),
                    brake: sample.brake.unwrap_or(false),
                };
                self.session.apply_input(rider, sample.seq, input);
                None
            }
            (C::Dev(cmd), Some(rider)) => Some(self.dev_command(rider, cmd)),
            (C::Turn(turn), Some(rider)) => {
                if let Some(net) = self.session.network() {
                    let choice = ocw_protocol::turn_choice(&turn, net);
                    self.session.set_turn(rider, turn.seq, choice);
                }
                None
            }
            (C::UTurn(u), Some(rider)) => {
                let from = (
                    f64::from(u.distance_cm) / 100.0,
                    f64::from(u.speed_mm_s) / 1000.0,
                );
                self.session.u_turn(rider, Some(from));
                None
            }
            (C::RouteRejoin(u), Some(rider)) => {
                let from = (
                    f64::from(u.distance_cm) / 100.0,
                    f64::from(u.speed_mm_s) / 1000.0,
                );
                if let Err(e) = self.session.rejoin_route(rider, Some(from)) {
                    tracing::debug!(%rider, %e, "rejoin refused");
                }
                None
            }
            (C::Steer(steer), Some(rider)) => {
                self.session.set_wild(rider, steer.wild);
                if steer.lean_steering {
                    self.session
                        .set_lean(rider, steer.seq, f64::from(steer.lean_mrad) / 1000.0);
                } else {
                    self.session.set_lateral_target(
                        rider,
                        steer.seq,
                        f64::from(steer.lateral_target_cm) / 100.0,
                    );
                }
                None
            }
            (_, None) => Some(S::Reject(v1::Reject {
                reason: "send Hello first".into(),
            })),
        };
        let mut out: Vec<Vec<u8>> = reply
            .map(|m| vec![encode(&server_message(m))])
            .unwrap_or_default();
        // A client that just joined a free ride gets everyone's path.
        let joined = !had_rider && self.riders.contains_key(&conn);
        if joined && let Some(net) = self.session.network() {
            out.extend(
                self.session
                    .all_paths()
                    .iter()
                    .map(|c| encode(&server_message(S::Path(ocw_protocol::path_update(c, net))))),
            );
        }
        out
    }

    fn hello(&mut self, conn: ConnectionId, hello: v1::Hello) -> S {
        if hello.protocol_version != PROTOCOL_VERSION {
            return S::Reject(v1::Reject {
                reason: format!(
                    "protocol version {} not supported (server speaks {PROTOCOL_VERSION}); please update",
                    hello.protocol_version
                ),
            });
        }
        // Junctions are vertex indices: both sides need the same graph.
        let ours = self.session.network().map(|n| world_id(n));
        if let (Some(ours), Some(theirs)) = (&ours, &hello.world)
            && ours.graph_hash != theirs.graph_hash
        {
            return S::Reject(v1::Reject {
                reason: format!(
                    "world package mismatch: the server rides {} ({:016x}), you have {} ({:016x}); install the same package version",
                    ours.name, ours.graph_hash, theirs.name, theirs.graph_hash
                ),
            });
        }
        let profile = physics_profile(&hello);
        let rider = self.session.add_rider(hello.display_name.clone(), profile);
        self.riders.insert(conn, rider);
        tracing::info!(%rider, name = %hello.display_name, weight_kg = profile.rider_mass_kg, "rider joined");
        S::Welcome(v1::Welcome {
            protocol_version: PROTOCOL_VERSION,
            rider_id: rider.0,
            tick_rate_hz: SIM_TICK_HZ,
            broadcast_rate_hz: BROADCAST_HZ,
            route_id: self.session.route().id().to_owned(),
            free_ride: self.session.is_free_ride(),
            world: ours,
            graph_route: self
                .session
                .plan()
                .map(|p| p.name.clone())
                .unwrap_or_default(),
        })
    }

    fn dev_command(&mut self, rider: RiderId, cmd: v1::DevCommand) -> S {
        let result = |ok: bool, message: String| S::DevResult(v1::DevCommandResult { ok, message });
        if !self.config.allow_dev_commands {
            return result(false, "dev commands are disabled on this server".into());
        }
        match cmd.cmd {
            Some(v1::dev_command::Cmd::TeleportDistanceM(d)) => {
                self.session.teleport(rider, d);
                result(true, format!("teleported to {:.3} km", d / 1000.0))
            }
            Some(v1::dev_command::Cmd::TeleportPoint(p)) => {
                if self.session.teleport_to(rider, p.x_m, p.z_m) {
                    result(
                        true,
                        format!(
                            "teleported to the road nearest x {:.0} z {:.0}",
                            p.x_m, p.z_m
                        ),
                    )
                } else {
                    result(false, "only in free ride".into())
                }
            }
            None => result(false, "empty dev command".into()),
        }
    }

    /// Advances one physics tick. Returns the encoded messages to broadcast
    /// to every connection: free-ride path changes (reliable, at once) and
    /// a snapshot when one is due (every 5th tick at 50/10 Hz).
    pub fn tick(&mut self) -> Vec<Vec<u8>> {
        self.session.step();
        let mut out = Vec::new();
        if let Some(net) = self.session.network().cloned() {
            for c in self.session.take_path_changes() {
                out.push(encode(&server_message(S::Path(ocw_protocol::path_update(
                    &c, &net,
                )))));
            }
        }
        if !self
            .session
            .tick()
            .is_multiple_of(u64::from(TICKS_PER_BROADCAST))
        {
            return out;
        }
        let snapshot = v1::Snapshot {
            server_tick: self.session.tick(),
            riders: self
                .session
                .snapshot()
                .iter()
                .map(ocw_protocol::quantize)
                .collect(),
        };
        out.push(encode(&server_message(S::Snapshot(snapshot))));
        out
    }

    pub fn connections(&self) -> impl Iterator<Item = ConnectionId> + '_ {
        self.riders.keys().copied()
    }
}

/// The physics profile of a joining rider (spec §2.5, §3.2.1): the curated
/// default profile with the rider's weight from the hello (clamped to
/// 30–200 kg). Drag, rolling resistance and bike mass sent by the client
/// are ignored: the server is the truth (§6.1).
pub fn physics_profile(hello: &v1::Hello) -> ocw_core::RiderProfile {
    hello
        .rider_weight_kg
        .or(hello.profile.as_ref().map(|p| p.rider_mass_kg))
        .map_or_else(ocw_core::RiderProfile::default, |kg| {
            ocw_core::RiderProfile::for_rider_weight(f64::from(kg))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_protocol::{client_message, decode_server};

    fn hello(version: u32) -> Vec<u8> {
        hello_with(version, None)
    }

    fn hello_with(version: u32, world: Option<v1::WorldPackageId>) -> Vec<u8> {
        encode(&client_message(C::Hello(v1::Hello {
            protocol_version: version,
            display_name: "Tester".into(),
            profile: Some(ocw_core::RiderProfile::default().into()),
            world,
            rider_weight_kg: None,
        })))
    }

    fn join_with_weight(server: &mut GameServer, weight: Option<f32>) -> RiderId {
        let conn = server.connect();
        let reply = server.handle(
            conn,
            &encode(&client_message(C::Hello(v1::Hello {
                protocol_version: PROTOCOL_VERSION,
                display_name: "Weighed".into(),
                profile: None,
                world: None,
                rider_weight_kg: weight,
            }))),
        );
        match decode_server(&reply[0]).unwrap() {
            S::Welcome(w) => RiderId(w.rider_id),
            other => panic!("expected a welcome, got {other:?}"),
        }
    }

    /// The rider weight from the hello is the rider's mass in the server's
    /// physics (spec §3.2.1), clamped; the rest is the curated profile.
    #[test]
    fn the_hello_weight_reaches_the_physics_profile() {
        let mut server = GameServer::new(Route::test_hills(), ServerConfig::default());
        let light = join_with_weight(&mut server, Some(58.0));
        let p = server.session().rider_profile(light).unwrap();
        assert_eq!(p.rider_mass_kg, 58.0);
        assert_eq!(p.total_mass_kg(), 66.0);
        let heavy = join_with_weight(&mut server, Some(900.0));
        assert_eq!(
            server.session().rider_profile(heavy).unwrap().rider_mass_kg,
            ocw_core::MAX_RIDER_WEIGHT_KG
        );
        let none = join_with_weight(&mut server, None);
        assert_eq!(
            server.session().rider_profile(none).unwrap(),
            ocw_core::RiderProfile::default()
        );
        // The weight changes the ride: on the climb the same power carries
        // the light rider faster than the heavy one.
        for rider in [light, heavy] {
            let conn = server
                .riders
                .iter()
                .find(|(_, r)| **r == rider)
                .map(|(c, _)| *c)
                .unwrap();
            let sample = v1::SensorSample {
                seq: 1,
                power_w: Some(250.0),
                ..Default::default()
            };
            server.handle(conn, &encode(&client_message(C::Sample(sample))));
        }
        for _ in 0..(50 * 60) {
            server.tick();
        }
        let distance = |r| server.session().rider_state(r).unwrap().distance;
        assert!(
            distance(light) > distance(heavy) + 10.0,
            "{} vs {}",
            distance(light),
            distance(heavy)
        );
    }

    /// A client cannot choose its own drag or rolling resistance (§2.5):
    /// from the deprecated profile field only the rider mass counts.
    #[test]
    fn client_physics_parameters_are_ignored() {
        let hello = v1::Hello {
            profile: Some(v1::RiderProfile {
                rider_mass_kg: 70.0,
                bike_mass_kg: 1.0,
                cda_m2: 0.01,
                crr: 0.0,
            }),
            ..Default::default()
        };
        assert_eq!(
            physics_profile(&hello),
            ocw_core::RiderProfile::for_rider_weight(70.0)
        );
        let hello = v1::Hello {
            rider_weight_kg: Some(64.0),
            ..hello
        };
        assert_eq!(physics_profile(&hello).rider_mass_kg, 64.0);
        let hello = v1::Hello {
            rider_weight_kg: Some(f32::NAN),
            ..hello
        };
        assert_eq!(physics_profile(&hello).rider_mass_kg, 75.0);
    }

    fn line_network(name: &str, length: f64) -> Arc<RoadNetwork> {
        use ocw_world::graph::{Edge, EdgeId, Oneway, RoadClass, RoadGraph, VertexId};
        let edge = Edge {
            id: EdgeId(9),
            from: VertexId(0),
            to: VertexId(1),
            points: vec![[0.0, 0.0], [length, 0.0]],
            class: RoadClass::Tertiary,
            name: None,
            width_m: 6.0,
            bridge: false,
            tunnel: false,
            dead_end_m: None,
            boundary: false,
            oneway: Oneway::No,
            osm_way: 1,
            profile: Vec::new(),
        };
        Arc::new(
            RoadNetwork::new(RoadGraph {
                vertices: vec![[0.0, 0.0], [length, 0.0]],
                edges: vec![edge],
                boundary_vertices: Default::default(),
            })
            .with_package_name(name),
        )
    }

    #[test]
    fn clients_with_another_world_package_are_rejected() {
        let net = line_network("Testland", 500.0);
        let course = || Course::FreeRide {
            net: net.clone(),
            start: FreeRideStart {
                edge: 0,
                offset_m: 10.0,
                forward: true,
            },
        };
        let mut server = GameServer::new(course(), ServerConfig::default());
        // Same graph: welcome, with the server's package identity.
        let conn = server.connect();
        let S::Welcome(w) = decode_server(
            &server.handle(conn, &hello_with(PROTOCOL_VERSION, Some(world_id(&net))))[0],
        )
        .unwrap() else {
            panic!("expected welcome")
        };
        assert_eq!(w.world, Some(world_id(&net)));
        assert!(w.free_ride);
        assert_eq!(w.graph_route, "");
        // Another build of the package (a longer road): rejected.
        let other = line_network("Testland", 501.0);
        assert_ne!(other.fingerprint(), net.fingerprint());
        let conn = server.connect();
        let S::Reject(r) = decode_server(
            &server.handle(conn, &hello_with(PROTOCOL_VERSION, Some(world_id(&other))))[0],
        )
        .unwrap() else {
            panic!("expected reject")
        };
        assert!(r.reason.contains("world package mismatch"), "{}", r.reason);
        assert_eq!(server.session().rider_count(), 1);
        // A route session (no graph): no package check, no package sent.
        let mut plain = GameServer::new(Route::test_loop(), ServerConfig::default());
        let conn = plain.connect();
        let S::Welcome(w) = decode_server(
            &plain.handle(conn, &hello_with(PROTOCOL_VERSION, Some(world_id(&other))))[0],
        )
        .unwrap() else {
            panic!("expected welcome")
        };
        assert_eq!(w.world, None);
    }

    #[test]
    fn route_rides_name_the_route_and_accept_rejoins() {
        let net = line_network("Testland", 800.0);
        let plan = ocw_world::RoutePlan::new(
            &net,
            "Line",
            vec![ocw_world::Segment::Edge {
                edge: 0,
                from_m: 0.0,
                to_m: 800.0,
            }],
            false,
        )
        .unwrap();
        let mut server = GameServer::new(
            Course::RouteRide {
                net: net.clone(),
                plan: Arc::new(plan),
            },
            ServerConfig::default(),
        );
        let conn = server.connect();
        let replies = server.handle(conn, &hello(PROTOCOL_VERSION));
        let S::Welcome(w) = decode_server(&replies[0]).unwrap() else {
            panic!("expected welcome")
        };
        assert_eq!(w.graph_route, "Line");
        assert_eq!(w.route_id, "Line");
        // The joining client gets the paths.
        assert!(replies.len() > 1);
        // On the route already: a rejoin is refused quietly.
        let rejoin = encode(&client_message(C::RouteRejoin(v1::RouteRejoin {
            distance_cm: 0,
            speed_mm_s: 0,
        })));
        assert!(server.handle(conn, &rejoin).is_empty());
    }

    fn sample(seq: u32, power: f32) -> Vec<u8> {
        encode(&client_message(C::Sample(v1::SensorSample {
            seq,
            client_time_us: 0,
            power_w: Some(power),
            cadence_rpm: None,
            heart_rate_bpm: None,
            brake: None,
        })))
    }

    #[test]
    fn hello_then_ride() {
        let mut server = GameServer::new(Route::test_loop(), ServerConfig::default());
        let conn = server.connect();
        let reply = server.handle(conn, &hello(PROTOCOL_VERSION));
        let S::Welcome(w) = decode_server(&reply[0]).unwrap() else {
            panic!("expected welcome")
        };
        assert_eq!(w.tick_rate_hz, 50);
        server.handle(conn, &sample(1, 250.0));
        let mut snapshots = 0;
        for _ in 0..250 {
            for bytes in server.tick() {
                snapshots += 1;
                let S::Snapshot(s) = decode_server(&bytes).unwrap() else {
                    panic!()
                };
                assert_eq!(s.riders.len(), 1);
            }
        }
        assert_eq!(snapshots, 50, "10 Hz over 5 s");
        let state = server.session().rider_state(RiderId(w.rider_id)).unwrap();
        // 250 W from standstill for 5 s: power-limited acceleration covers ~15 m.
        assert!(state.distance > 10.0, "distance {}", state.distance);
    }

    #[test]
    fn rejects_wrong_version_and_missing_hello() {
        let mut server = GameServer::new(Route::test_loop(), ServerConfig::default());
        let conn = server.connect();
        assert!(matches!(
            decode_server(&server.handle(conn, &sample(1, 100.0))[0]).unwrap(),
            S::Reject(_)
        ));
        assert!(matches!(
            decode_server(&server.handle(conn, &hello(99))[0]).unwrap(),
            S::Reject(_)
        ));
        assert_eq!(server.session().rider_count(), 0);
    }

    #[test]
    fn dev_commands_only_on_local_server() {
        let teleport = encode(&client_message(C::Dev(v1::DevCommand {
            cmd: Some(v1::dev_command::Cmd::TeleportDistanceM(1000.0)),
        })));
        for allow in [false, true] {
            let mut server = GameServer::new(
                Route::test_loop(),
                ServerConfig {
                    allow_dev_commands: allow,
                    bots: 0,
                    start_m: 0.0,
                },
            );
            let conn = server.connect();
            server.handle(conn, &hello(PROTOCOL_VERSION));
            let S::DevResult(r) = decode_server(&server.handle(conn, &teleport)[0]).unwrap() else {
                panic!()
            };
            assert_eq!(r.ok, allow);
        }
    }

    #[test]
    fn bots_ride_at_different_strengths() {
        let n = BOT_POWERS_W.len() as u32;
        assert!(n >= 10);
        let powers: Vec<f64> = (0..n).map(bot_power_w).collect();
        // Some are clearly slower, some faster, and they are all different.
        assert!(
            powers.iter().filter(|&&p| p < 140.0).count() >= 4,
            "{powers:?}"
        );
        assert!(powers.iter().filter(|&&p| p > 200.0).count() >= 3);
        assert!(powers.iter().all(|&p| (85.0..=280.0).contains(&p)));
        let mut sorted = powers.clone();
        sorted.sort_by(f64::total_cmp);
        sorted.dedup();
        assert_eq!(sorted.len(), n as usize);
        // The first bots (a small server) already mix slow and fast.
        assert!(bot_power_w(1) < bot_power_w(0) - 50.0);
        let first: Vec<f64> = (0..4).map(bot_power_w).collect();
        assert!(first.iter().any(|&p| p < 100.0) && first.iter().any(|&p| p > 200.0));
        // Deterministic, and it repeats for big fields.
        assert_eq!(bot_power_w(n + 3), bot_power_w(3));
    }

    #[test]
    fn many_bots_start_in_formation_on_the_road() {
        let route = Route::test_loop();
        let limit = ocw_sim::physics::lateral_limit(route.road_width_m());
        let server = GameServer::new(
            route,
            ServerConfig {
                allow_dev_commands: false,
                bots: 20,
                start_m: 200.0,
            },
        );
        let snap = server.session().snapshot();
        assert_eq!(snap.len(), 20);
        let mut spots: Vec<(i64, i64)> = snap
            .iter()
            .map(|r| {
                assert!(r.state.lateral_offset.abs() < limit, "{r:?}");
                assert!(r.state.distance <= 200.0 && r.state.distance >= 180.0);
                (
                    (r.state.distance * 10.0).round() as i64,
                    (r.state.lateral_offset * 10.0).round() as i64,
                )
            })
            .collect();
        spots.sort();
        spots.dedup();
        assert_eq!(spots.len(), 20);
    }

    #[test]
    fn disconnect_removes_rider_and_bots_stay() {
        let mut server = GameServer::new(
            Route::test_loop(),
            ServerConfig {
                allow_dev_commands: false,
                bots: 3,
                start_m: 0.0,
            },
        );
        let conn = server.connect();
        server.handle(conn, &hello(PROTOCOL_VERSION));
        assert_eq!(server.session().rider_count(), 4);
        server.disconnect(conn);
        assert_eq!(server.session().rider_count(), 3);
    }
}
