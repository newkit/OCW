//! The in-process local server for single-player / offline rides (spec §6.1).
//!
//! Runs [`GameServer`] on its own thread at the fixed 50 Hz tick, so the
//! simulation never depends on the client's frame rate. The client talks to it
//! with the same encoded protocol messages a remote server would receive.

use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use ocw_core::SIM_DT;
use ocw_world::Route;

use crate::{GameServer, ServerConfig};

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
    pub fn spawn(route: Route, bots: u32) -> std::io::Result<LocalConnection> {
        let (to_server, server_rx) = mpsc::channel::<Vec<u8>>();
        let (server_tx, from_server) = mpsc::channel::<Vec<u8>>();
        let mut server = GameServer::new(
            route,
            ServerConfig {
                allow_dev_commands: true,
                bots,
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
                    if let Some(snapshot) = server.tick()
                        && server_tx.send(snapshot).is_err()
                    {
                        break;
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

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_protocol::v1::{self, client_message::Msg as C, server_message::Msg as S};
    use ocw_protocol::{PROTOCOL_VERSION, client_message, decode_server, encode};

    #[test]
    fn local_server_welcomes_and_broadcasts() {
        let conn = LocalServer::spawn(Route::test_loop(), 2).unwrap();
        conn.to_server
            .send(encode(&client_message(C::Hello(v1::Hello {
                protocol_version: PROTOCOL_VERSION,
                display_name: "Local".into(),
                profile: None,
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
}
