//! The in-process local server for single-player / offline rides (spec §6.1).
//!
//! Runs [`GameServer`] on its own thread at the fixed 50 Hz tick, so the
//! simulation never depends on the client's frame rate. The client talks to it
//! with the same encoded protocol messages a remote server would receive.

use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::{Course, GameServer, ServerConfig};
use ocw_core::SIM_DT;

/// Client side of a local server connection.
pub struct LocalConnection {
    /// Encoded `ClientMessage`s to the server.
    pub to_server: Sender<Vec<u8>>,
    /// Encoded `ServerMessage`s from the server.
    pub from_server: Receiver<Vec<u8>>,
    thread: Option<JoinHandle<()>>,
}

impl LocalConnection {
    /// Stops the server thread and waits for it.
    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        // Dropping the sender makes the server loop exit.
        let (dead, _) = mpsc::channel();
        drop(std::mem::replace(&mut self.to_server, dead));
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for LocalConnection {
    fn drop(&mut self) {
        self.stop();
    }
}

pub struct LocalServer;

impl LocalServer {
    /// Starts a local server thread with dev commands enabled.
    /// Everyone starts `start_m` metres along the route.
    pub fn spawn(
        course: impl Into<Course>,
        bots: u32,
        start_m: f64,
    ) -> std::io::Result<LocalConnection> {
        let (to_server, server_rx) = mpsc::channel::<Vec<u8>>();
        let (server_tx, from_server) = mpsc::channel::<Vec<u8>>();
        let mut server = GameServer::new(
            course,
            ServerConfig {
                allow_dev_commands: true,
                bots,
                start_m,
                allow_local_events: true,
                voice: None,
                // The single player is the host (spec §14 Phase 9, 9D).
                clock_access: crate::ClockAccess::Anyone,
                // The app switches its local world itself (spec §3.2.1).
                world_access: crate::ClockAccess::Anyone,
                real_epoch_ms: None,
            },
        );
        let thread = thread::Builder::new()
            .name("ocw-local-server".into())
            .spawn(move || {
                let conn = server.connect();
                let tick = Duration::from_secs_f64(SIM_DT);
                let mut next = Instant::now();
                'run: loop {
                    loop {
                        match server_rx.try_recv() {
                            Ok(bytes) => {
                                for reply in server.handle(conn, &bytes) {
                                    if server_tx.send(reply).is_err() {
                                        break 'run;
                                    }
                                }
                            }
                            Err(TryRecvError::Empty) => break,
                            Err(TryRecvError::Disconnected) => break 'run,
                        }
                    }
                    for msg in server.tick() {
                        if server_tx.send(msg).is_err() {
                            break 'run;
                        }
                    }
                    next += tick;
                    let now = Instant::now();
                    if next > now {
                        thread::sleep(next - now);
                    } else if now - next > Duration::from_millis(250) {
                        // Far behind (e.g. machine suspended): don't try to catch up.
                        next = now;
                    }
                }
                server.disconnect(conn);
            })?;
        Ok(LocalConnection {
            to_server,
            from_server,
            thread: Some(thread),
        })
    }
}

/// A local server that does not run on its own clock: the caller ticks it.
///
/// Used for deterministic frame capture (`ocw-desktop --capture`), where game
/// time advances by a fixed step per rendered frame instead of with the wall
/// clock, so client and server must share that time.
pub struct SteppedServer {
    server: GameServer,
    conn: crate::ConnectionId,
    outbox: std::collections::VecDeque<Vec<u8>>,
}

impl SteppedServer {
    /// A stepped server with dev commands enabled and one connected client.
    pub fn new(course: impl Into<Course>, bots: u32, start_m: f64) -> Self {
        let mut server = GameServer::new(
            course,
            ServerConfig {
                allow_dev_commands: true,
                bots,
                start_m,
                allow_local_events: true,
                voice: None,
                // The single player is the host (spec §14 Phase 9, 9D).
                clock_access: crate::ClockAccess::Anyone,
                // The app switches its local world itself (spec §3.2.1).
                world_access: crate::ClockAccess::Anyone,
                real_epoch_ms: None,
            },
        );
        let conn = server.connect();
        Self {
            server,
            conn,
            outbox: Default::default(),
        }
    }

    /// Handles an encoded client message immediately.
    pub fn send(&mut self, bytes: &[u8]) {
        let replies = self.server.handle(self.conn, bytes);
        self.outbox.extend(replies);
    }

    /// Advances the simulation by one 50 Hz tick.
    pub fn tick(&mut self) {
        let messages = self.server.tick();
        self.outbox.extend(messages);
    }

    /// Takes all messages sent to the client since the last call.
    pub fn drain(&mut self) -> Vec<Vec<u8>> {
        self.outbox.drain(..).collect()
    }

    pub fn server(&self) -> &GameServer {
        &self.server
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_protocol::v1::{self, client_message::Msg as C, server_message::Msg as S};
    use ocw_protocol::{PROTOCOL_VERSION, client_message, decode_server, encode};
    use ocw_world::Route;

    #[test]
    fn local_server_welcomes_and_broadcasts() {
        let conn = LocalServer::spawn(Route::test_loop(), 2, 0.0).unwrap();
        conn.to_server
            .send(encode(&client_message(C::Hello(v1::Hello {
                protocol_version: PROTOCOL_VERSION,
                display_name: "Local".into(),
                profile: None,
                world: None,
                rider_weight_kg: None,
                bike_profile: None,
                look: None,
            }))))
            .unwrap();
        let mut welcomed = false;
        let mut riders = 0;
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline && !(welcomed && riders == 3) {
            let bytes = conn
                .from_server
                .recv_timeout(Duration::from_secs(1))
                .unwrap();
            match decode_server(&bytes).unwrap() {
                S::Welcome(_) => welcomed = true,
                S::Snapshot(s) => riders = s.riders.len(),
                _ => {}
            }
        }
        assert!(welcomed);
        assert_eq!(riders, 3);
        conn.shutdown();
    }

    #[test]
    fn stepped_server_advances_only_when_ticked() {
        let mut s = SteppedServer::new(Route::test_loop(), 1, 0.0);
        s.send(&encode(&client_message(C::Hello(v1::Hello {
            protocol_version: PROTOCOL_VERSION,
            display_name: "Cap".into(),
            profile: None,
            world: None,
            rider_weight_kg: None,
            bike_profile: None,
            look: None,
        }))));
        let welcome = s.drain();
        assert_eq!(welcome.len(), 1);
        assert!(matches!(decode_server(&welcome[0]).unwrap(), S::Welcome(_)));
        // No wall-clock progress: nothing happens without ticks.
        std::thread::sleep(Duration::from_millis(50));
        assert!(s.drain().is_empty());
        assert_eq!(s.server().session().tick(), 0);
        for _ in 0..50 {
            s.tick();
        }
        assert_eq!(s.server().session().tick(), 50);
        // One snapshot per 5 ticks (10 Hz).
        assert_eq!(s.drain().len(), 10);
    }
}
