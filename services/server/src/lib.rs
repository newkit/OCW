//! Game server logic (spec §6.1).
//!
//! [`GameServer`] is transport-independent: it consumes and produces encoded
//! protocol messages. The same type runs
//! - inside the desktop app as the **local server** ([`local::LocalServer`]), and
//! - as the headless **remote server** binary (`ocw-server`; QUIC transport in Phase 5).

pub mod local;

use std::collections::BTreeMap;

use ocw_core::{BROADCAST_HZ, RiderId, SIM_TICK_HZ, TICKS_PER_BROADCAST};
use ocw_protocol::v1::{self, client_message::Msg as C, server_message::Msg as S};
use ocw_protocol::{PROTOCOL_VERSION, decode_client, encode, server_message};
use ocw_sim::{RiderInput, Session};
use ocw_world::Route;

/// Identifies one client connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConnectionId(pub u64);

#[derive(Debug, Clone, Default)]
pub struct ServerConfig {
    /// Accept [`v1::DevCommand`]s. Only true for the in-process local server (spec §2.9).
    pub allow_dev_commands: bool,
    /// Number of server-side test riders.
    pub bots: u32,
}

/// Authoritative server for one session.
pub struct GameServer {
    config: ServerConfig,
    session: Session,
    /// Connection → rider, once the client said hello.
    riders: BTreeMap<ConnectionId, RiderId>,
    next_connection: u64,
}

impl GameServer {
    pub fn new(route: Route, config: ServerConfig) -> Self {
        let mut session = Session::new(route);
        for i in 0..config.bots {
            session.add_bot(format!("Bot {}", i + 1), 160.0 + 25.0 * f64::from(i % 5));
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
            (C::Steer(steer), Some(rider)) => {
                self.session.set_lateral_target(
                    rider,
                    steer.seq,
                    f64::from(steer.lateral_target_cm) / 100.0,
                );
                None
            }
            (_, None) => Some(S::Reject(v1::Reject {
                reason: "send Hello first".into(),
            })),
        };
        reply
            .map(|m| vec![encode(&server_message(m))])
            .unwrap_or_default()
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
        let profile = hello.profile.map(Into::into).unwrap_or_default();
        let rider = self.session.add_rider(hello.display_name.clone(), profile);
        self.riders.insert(conn, rider);
        tracing::info!(%rider, name = %hello.display_name, "rider joined");
        S::Welcome(v1::Welcome {
            protocol_version: PROTOCOL_VERSION,
            rider_id: rider.0,
            tick_rate_hz: SIM_TICK_HZ,
            broadcast_rate_hz: BROADCAST_HZ,
            route_id: self.session.route().id().to_owned(),
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
            None => result(false, "empty dev command".into()),
        }
    }

    /// Advances one physics tick. Returns an encoded snapshot to broadcast to
    /// every connection when a broadcast is due (every 5th tick at 50/10 Hz).
    pub fn tick(&mut self) -> Option<Vec<u8>> {
        self.session.step();
        if !self
            .session
            .tick()
            .is_multiple_of(u64::from(TICKS_PER_BROADCAST))
        {
            return None;
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
        Some(encode(&server_message(S::Snapshot(snapshot))))
    }

    pub fn connections(&self) -> impl Iterator<Item = ConnectionId> + '_ {
        self.riders.keys().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_protocol::{client_message, decode_server};

    fn hello(version: u32) -> Vec<u8> {
        encode(&client_message(C::Hello(v1::Hello {
            protocol_version: version,
            display_name: "Tester".into(),
            profile: Some(ocw_core::RiderProfile::default().into()),
        })))
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
            if let Some(bytes) = server.tick() {
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
    fn disconnect_removes_rider_and_bots_stay() {
        let mut server = GameServer::new(
            Route::test_loop(),
            ServerConfig {
                allow_dev_commands: false,
                bots: 3,
            },
        );
        let conn = server.connect();
        server.handle(conn, &hello(PROTOCOL_VERSION));
        assert_eq!(server.session().rider_count(), 4);
        server.disconnect(conn);
        assert_eq!(server.session().rider_count(), 3);
    }
}
