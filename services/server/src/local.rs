//! The in-process local server for single-player / offline rides (spec §6.1).
//!
//! [`LocalServer`] runs [`GameServer`] on its own thread at the fixed 50 Hz
//! tick, so the simulation never depends on the client's frame rate (native
//! only). [`SteppedServer`] is ticked by the caller (frame capture), and
//! [`FrameServer`] keeps the 50 Hz clock from the frame times it is given:
//! the browser app's local server (spec §14 Phase 10, B1), where there are
//! no threads. The client talks to all of them with the same encoded
//! protocol messages a remote server would receive.

#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
#[cfg(not(target_arch = "wasm32"))]
use std::thread::{self, JoinHandle};
#[cfg(not(target_arch = "wasm32"))]
use std::time::{Duration, Instant};

use crate::{Course, GameServer, ServerConfig};
use ocw_core::SIM_DT;

/// The configuration of every local server: dev commands and local events
/// on, the single player is the host (clock, world, god mode).
pub fn local_config(bots: u32, start_m: f64) -> ServerConfig {
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
        // God mode for the single player (spec §14 Phase 13A).
        god_access: crate::ClockAccess::Anyone,
        real_epoch_ms: None,
        // The echo test for the single player (spec §7).
        allow_echo_bot: true,
        echo_bot_at_start: false,
        // Collisions on, damage off (spec §14 Phase 12 Q1); the rider's
        // settings change them (RulesRequest), the single player decides.
        collisions: true,
        chaotic_bots: false,
        collision_access: crate::ClockAccess::Anyone,
    }
}

/// A local server that has fallen this far behind (s; e.g. the machine
/// was suspended, or the browser tab was in the background) does not try
/// to catch up: the backlog is dropped.
pub const MAX_BACKLOG_S: f64 = 0.25;

/// Client side of a local server connection.
#[cfg(not(target_arch = "wasm32"))]
pub struct LocalConnection {
    /// Encoded `ClientMessage`s to the server.
    pub to_server: Sender<Vec<u8>>,
    /// Encoded `ServerMessage`s from the server.
    pub from_server: Receiver<Vec<u8>>,
    thread: Option<JoinHandle<()>>,
}

#[cfg(not(target_arch = "wasm32"))]
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

#[cfg(not(target_arch = "wasm32"))]
impl Drop for LocalConnection {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The threaded local server (native only).
#[cfg(not(target_arch = "wasm32"))]
pub struct LocalServer;

#[cfg(not(target_arch = "wasm32"))]
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
        let mut server = GameServer::new(course, local_config(bots, start_m));
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
                    } else if now - next > Duration::from_secs_f64(MAX_BACKLOG_S) {
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
        let mut server = GameServer::new(course, local_config(bots, start_m));
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

/// A local server on the caller's frame loop (the browser app, spec §14
/// Phase 10, B1: wasm has no threads): [`FrameServer::advance`] gets the
/// time since the last frame and runs as many fixed 50 Hz ticks as fit,
/// like [`LocalServer`]'s thread does with the wall clock. Ticks are
/// counted in whole microseconds, so frame times never drift the clock.
pub struct FrameServer {
    stepped: SteppedServer,
    /// Time not yet simulated (µs).
    backlog_us: u64,
}

/// One 50 Hz tick in microseconds.
const TICK_US: u64 = (SIM_DT * 1e6) as u64;

impl FrameServer {
    /// A frame-driven server with dev commands enabled and one connected
    /// client.
    pub fn new(course: impl Into<Course>, bots: u32, start_m: f64) -> Self {
        Self {
            stepped: SteppedServer::new(course, bots, start_m),
            backlog_us: 0,
        }
    }

    /// Handles an encoded client message immediately.
    pub fn send(&mut self, bytes: &[u8]) {
        self.stepped.send(bytes);
    }

    /// Advances the server's clock by `dt_s` (the frame time): runs the
    /// ticks that became due and returns how many. A backlog beyond
    /// [`MAX_BACKLOG_S`] is dropped (a tab in the background).
    pub fn advance(&mut self, dt_s: f64) -> u32 {
        let dt_us = (dt_s.max(0.0) * 1e6).round() as u64;
        self.backlog_us += dt_us;
        let max_us = (MAX_BACKLOG_S * 1e6) as u64;
        if self.backlog_us > max_us + TICK_US {
            self.backlog_us = max_us;
        }
        let mut ticks = 0;
        while self.backlog_us >= TICK_US {
            self.backlog_us -= TICK_US;
            self.stepped.tick();
            ticks += 1;
        }
        ticks
    }

    /// Takes all messages sent to the client since the last call.
    pub fn drain(&mut self) -> Vec<Vec<u8>> {
        self.stepped.drain()
    }

    pub fn server(&self) -> &GameServer {
        self.stepped.server()
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

    fn hello(name: &str) -> Vec<u8> {
        encode(&client_message(C::Hello(v1::Hello {
            protocol_version: PROTOCOL_VERSION,
            display_name: name.into(),
            profile: None,
            world: None,
            rider_weight_kg: None,
            bike_profile: None,
            look: None,
        })))
    }

    /// (server tick, riders) of each snapshot among `messages`.
    fn snapshots(messages: &[Vec<u8>]) -> Vec<(u64, usize)> {
        messages
            .iter()
            .filter_map(|b| match decode_server(b).unwrap() {
                S::Snapshot(s) => Some((s.server_tick, s.riders.len())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn frame_server_runs_the_ticks_that_fit_into_the_frames() {
        let mut f = FrameServer::new(Route::test_loop(), 2, 0.0);
        f.send(&hello("Web"));
        assert!(matches!(
            decode_server(&f.drain()[0]).unwrap(),
            S::Welcome(_)
        ));
        // Uneven frames (60 Hz with jitter, a slow frame): 2 s in total.
        let frames: [f64; 5] = [0.016, 0.021, 0.012, 0.05, 0.007];
        let mut total = 0.0;
        let mut ticks = 0;
        let mut frame = 0;
        while total < 2.0 - 1e-9 {
            let dt = frames[frame % frames.len()].min(2.0 - total);
            frame += 1;
            total += dt;
            ticks += f.advance(dt);
        }
        // Exactly the 100 ticks of 2 s, whatever the frame times.
        assert_eq!(ticks, 100);
        assert_eq!(f.server().session().tick(), 100);
        let snaps = snapshots(&f.drain());
        assert_eq!(snaps.len(), 20);
        assert!(snaps.iter().all(|&(_, riders)| riders == 3));
        // A frame shorter than a tick runs none; the rest is kept.
        assert_eq!(f.advance(0.015), 0);
        assert_eq!(f.advance(0.005), 1);
    }

    #[test]
    fn frame_server_matches_the_stepped_server_tick_for_tick() {
        let mut f = FrameServer::new(Route::test_loop(), 3, 100.0);
        let mut s = SteppedServer::new(Route::test_loop(), 3, 100.0);
        f.send(&hello("A"));
        s.send(&hello("A"));
        let power = encode(&client_message(C::Sample(v1::SensorSample {
            seq: 1,
            client_time_us: 0,
            power_w: Some(250.0),
            cadence_rpm: Some(90.0),
            heart_rate_bpm: None,
            brake: None,
            virtual_power: None,
        })));
        f.send(&power);
        s.send(&power);
        // 3 s of 60 Hz frames against 150 explicit ticks.
        for _ in 0..180 {
            f.advance(1.0 / 60.0);
        }
        for _ in 0..150 {
            s.tick();
        }
        assert_eq!(f.server().session().tick(), 150);
        assert_eq!(snapshots(&f.drain()), snapshots(&s.drain()));
        // The same simulation: everyone at the same distance.
        let at = |g: &GameServer| {
            g.session()
                .snapshot()
                .iter()
                .map(|r| r.state.distance)
                .collect::<Vec<_>>()
        };
        assert_eq!(at(f.server()), at(s.server()));
    }

    #[test]
    fn frame_server_drops_a_long_backlog() {
        let mut f = FrameServer::new(Route::test_loop(), 0, 0.0);
        // A tab in the background for 10 s: at most MAX_BACKLOG_S is
        // simulated, then it runs at the normal rate again.
        let ticks = f.advance(10.0);
        assert_eq!(ticks, (MAX_BACKLOG_S / SIM_DT) as u32);
        assert_eq!(f.advance(0.02), 1);
        // Negative or zero frame times do nothing.
        assert_eq!(f.advance(-1.0), 0);
        assert_eq!(f.advance(0.0), 0);
    }

    /// The frame-driven server keeps the same 50 Hz clock as the threaded
    /// one: over the same wall time both broadcast the same number of
    /// snapshots (within scheduling jitter).
    #[test]
    fn frame_server_keeps_the_threaded_servers_rate() {
        let conn = LocalServer::spawn(Route::test_loop(), 1, 0.0).unwrap();
        let mut f = FrameServer::new(Route::test_loop(), 1, 0.0);
        conn.to_server.send(hello("T")).unwrap();
        f.send(&hello("T"));
        let start = Instant::now();
        let mut last = start;
        let mut threaded = Vec::new();
        while start.elapsed() < Duration::from_millis(1500) {
            std::thread::sleep(Duration::from_millis(16));
            let now = Instant::now();
            f.advance((now - last).as_secs_f64());
            last = now;
            threaded.extend(conn.from_server.try_iter());
        }
        let (t, fr) = (snapshots(&threaded).len(), snapshots(&f.drain()).len());
        // 1.5 s at 10 Hz: 15 each.
        assert!((13..=17).contains(&fr), "frame server: {fr}");
        assert!(t.abs_diff(fr) <= 3, "threaded {t} vs frame {fr}");
        conn.shutdown();
    }
}
