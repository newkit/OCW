//! The browser app's ride session without Bevy (spec §14 Phase 10, B1 /
//! B3): the shared client loop ([`ClientCore`]) and the virtual trainer
//! ([`FrameVirtualTrainer`]) with either the local server on the frame
//! loop ([`FrameServer`]) or a remote game server over WebTransport
//! ([`RemoteLink`]: the wire's [`ClientLink`] state machine with a
//! [`Transport`] that moves its bytes – the browser's WebTransport, or a
//! fake in the tests). Wasm has no threads, so where the desktop runs the
//! server and the trainer on their own threads, [`WebSession::frame`]
//! advances both by the frame time. The Bevy glue (`crate::plugin`) only
//! calls it; the whole loop is unit-tested natively.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use ocw_client::client::{ClientCore, Notice};
use ocw_client::input::{KeyThrottle, ThrottleKeys};
use ocw_core::{RiderLook, RiderProfile};
use ocw_devices::{DeviceCommand, FrameVirtualTrainer};
use ocw_protocol::decode_server;
use ocw_protocol::v1::server_message::Msg as S;
use ocw_server::Course;
use ocw_server::local::FrameServer;
use ocw_server::timing::SnapshotTiming;
use ocw_server::wire::{ClientLink, LinkStatus, server_unreliable};
use ocw_world::Route;

/// Power of the virtual trainer at the start (W).
pub const DEFAULT_POWER_W: f64 = 200.0;
/// The virtual power stays within this range (W).
pub const MAX_POWER_W: f64 = 1500.0;
/// Test riders on the browser's local server.
pub const DEFAULT_BOTS: u32 = 5;

/// A remote game server (spec §14 Phase 10 B3; page options `server`,
/// `cert-hash`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteOptions {
    /// The WebTransport URL (`https://HOST:PORT/ocw/1`).
    pub url: String,
    /// SHA-256 hashes of the certificates to accept without a CA (the
    /// server's self-signed one, `serverCertificateHashes`); empty: the
    /// browser checks the certificate as for any https site.
    pub cert_hashes: Vec<[u8; 32]>,
}

/// What the session needs to start.
#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub route: Route,
    pub name: String,
    pub bots: u32,
    pub start_m: f64,
    pub power_w: f64,
    /// A world package streamed over HTTP (spec §14 Phase 10 B2): the
    /// session starts once its base and first tiles are there.
    pub world: Option<crate::world::WorldOptions>,
    /// Ride on a remote game server instead of the local one (B3).
    pub remote: Option<RemoteOptions>,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            // The desktop's default test route (`--route hills`).
            route: Route::test_hills(),
            name: "Rider".into(),
            bots: DEFAULT_BOTS,
            start_m: 0.0,
            power_w: DEFAULT_POWER_W,
            world: None,
            remote: None,
        }
    }
}

/// Moves a [`ClientLink`]'s bytes: sends what it has to send and hands it
/// what arrived (the browser's WebTransport calls back on its own; the
/// test fake does both in `pump`).
pub trait Transport: Send + Sync {
    /// Sends the link's outgoing bytes (once per frame, and after the
    /// frame's messages).
    fn pump(&mut self, link: &Arc<Mutex<ClientLink>>, now_s: f64);
    /// Closes the connection ("client quit").
    fn close(&mut self) {}
}

/// The link to a remote server and what it measured.
pub struct RemoteLink {
    pub url: String,
    pub link: Arc<Mutex<ClientLink>>,
    pub transport: Box<dyn Transport>,
    /// Snapshot interval, jitter and age (the same measurement as
    /// `ocw-bot`'s, for the comparison with native QUIC).
    pub timing: SnapshotTiming,
    /// Display names of the other riders seen in snapshots.
    pub others_seen: BTreeSet<String>,
    /// Most riders in one snapshot (the own rider included).
    pub max_riders: usize,
    /// The highest sample sequence number the server applied (its own
    /// rider's `last_input_seq`): the server got the inputs.
    pub acked_seq: u32,
}

/// The server the session rides on.
pub enum ServerLink {
    Local(Box<FrameServer>),
    Remote(Box<RemoteLink>),
}

/// Server, client and trainer of one ride in the browser.
pub struct WebSession {
    pub server: ServerLink,
    pub client: ClientCore,
    pub trainer: FrameVirtualTrainer,
    /// Server ticks run so far (for the start-up log and the tests; local
    /// server only).
    pub ticks: u64,
}

impl WebSession {
    /// Starts the session on its test route: the local server (the hello
    /// is sent at once), or – with `remote` – the connection to the remote
    /// server through `transport`.
    pub fn new(config: SessionConfig, transport: Option<Box<dyn Transport>>) -> Self {
        let client = ClientCore::new(
            config.route.clone(),
            config.name.clone(),
            RiderProfile::default(),
        )
        .with_look(RiderLook::anonymous());
        Self::start(config.route.clone().into(), client, &config, transport)
    }

    /// A session on any course (a route, or a ride on a world package's
    /// road graph) with its client: local, or remote when the config has a
    /// server and a transport is given.
    pub fn start(
        course: Course,
        client: ClientCore,
        config: &SessionConfig,
        transport: Option<Box<dyn Transport>>,
    ) -> Self {
        match (&config.remote, transport) {
            (Some(remote), Some(t)) => Self::remote(
                client,
                remote.url.clone(),
                Arc::default(),
                t,
                config.power_w,
            ),
            _ => Self::ride(course, client, config.bots, config.start_m, config.power_w),
        }
    }

    /// A session on the local server.
    pub fn ride(course: Course, client: ClientCore, bots: u32, start_m: f64, power_w: f64) -> Self {
        let mut server = FrameServer::new(course, bots, start_m);
        server.send(&client.hello());
        Self {
            server: ServerLink::Local(Box::new(server)),
            client,
            trainer: FrameVirtualTrainer::new(power_w),
            ticks: 0,
        }
    }

    /// A session on a remote server: the hello waits in `link` until the
    /// transport is up.
    pub fn remote(
        client: ClientCore,
        url: String,
        link: Arc<Mutex<ClientLink>>,
        transport: Box<dyn Transport>,
        power_w: f64,
    ) -> Self {
        link.lock().expect("not poisoned").send(client.hello());
        Self {
            server: ServerLink::Remote(Box::new(RemoteLink {
                url,
                link,
                transport,
                timing: SnapshotTiming::default(),
                others_seen: BTreeSet::new(),
                max_riders: 0,
                acked_seq: 0,
            })),
            client,
            trainer: FrameVirtualTrainer::new(power_w),
            ticks: 0,
        }
    }

    pub fn is_remote(&self) -> bool {
        matches!(self.server, ServerLink::Remote(_))
    }

    pub fn remote_link(&self) -> Option<&RemoteLink> {
        match &self.server {
            ServerLink::Remote(r) => Some(r),
            ServerLink::Local(_) => None,
        }
    }

    /// One frame of `dt_s` seconds, at game time `now_s` (after the
    /// frame's time step): the trainer's sample goes to the server, the
    /// server runs its due ticks (local) or the transport moves the bytes
    /// (remote), and the server's messages go to the client. Returns the
    /// notices for the console.
    pub fn frame(&mut self, dt_s: f64, now_s: f64) -> Vec<Notice> {
        if let Some(event) = self.trainer.advance(dt_s)
            && let Some(sample) = self.client.on_device_event(event)
        {
            self.send(&sample);
        }
        match &mut self.server {
            ServerLink::Local(s) => self.ticks += u64::from(s.advance(dt_s)),
            ServerLink::Remote(r) => r.transport.pump(&r.link, now_s),
        }
        self.receive(now_s)
    }

    /// Sends what is waiting (the frame's steering and commands) without
    /// waiting for the next frame (remote only).
    pub fn flush(&mut self, now_s: f64) {
        if let ServerLink::Remote(r) = &mut self.server {
            r.transport.pump(&r.link, now_s);
        }
    }

    /// Sends a client message (steering, turns, events, dev commands).
    pub fn send(&mut self, bytes: &[u8]) {
        match &mut self.server {
            ServerLink::Local(s) => s.send(bytes),
            ServerLink::Remote(r) => r.link.lock().expect("not poisoned").send(bytes.to_vec()),
        }
    }

    /// Hands the server's messages to the client (also the immediate
    /// replies to [`send`](Self::send)).
    pub fn receive(&mut self, now_s: f64) -> Vec<Notice> {
        let messages = match &mut self.server {
            ServerLink::Local(s) => s.drain(),
            ServerLink::Remote(r) => {
                let (timed, rtt) = {
                    let mut l = r.link.lock().expect("not poisoned");
                    (l.receive_timed(), l.rtt_s())
                };
                if let Some(rtt) = rtt {
                    // The prediction extrapolates snapshots by half the round trip.
                    self.client.set_snapshot_age(rtt / 2.0);
                }
                let own = self.client.rider_id().map(|r| r.0);
                // Timed when the transport got them (the browser's link
                // clock is the wall clock: interval, jitter and age).
                for (at, m) in &timed {
                    r.observe(m, *at, own);
                }
                timed.into_iter().map(|(_, m)| m).collect()
            }
        };
        messages
            .iter()
            .filter_map(|bytes| self.client.on_server_message(bytes, now_s))
            .collect()
    }

    /// The remote link's state (`None`: the local server).
    pub fn link_status(&self) -> Option<LinkStatus> {
        self.remote_link()
            .map(|r| r.link.lock().expect("not poisoned").status().clone())
    }

    /// A line for the HUD while the remote link is not up (as on the
    /// desktop): connecting, or why it ended.
    pub fn link_notice(&self) -> Option<String> {
        match self.link_status()? {
            LinkStatus::Connecting => Some("Connecting to the server …".into()),
            LinkStatus::Connected { .. } => None,
            LinkStatus::Closed(reason) => Some(format!("Disconnected: {reason}")),
        }
    }

    /// Closes the remote connection.
    pub fn close(&mut self) {
        if let ServerLink::Remote(r) = &mut self.server {
            r.transport.close();
            r.link
                .lock()
                .expect("not poisoned")
                .on_closed("connection closed");
        }
    }

    /// The virtual trainer's set power (W).
    pub fn power_w(&self) -> f64 {
        self.trainer.set_power_w()
    }

    /// Sets the virtual trainer's power (W, clamped to 0 … [`MAX_POWER_W`]);
    /// the next sample (≤ 0.25 s) carries it.
    pub fn set_power(&mut self, watts: f64) -> f64 {
        let w = watts.clamp(0.0, MAX_POWER_W);
        self.trainer.command(DeviceCommand::SetVirtualPower(w));
        w
    }

    /// Applies the power keys of a frame ([`KeyThrottle`], the desktop's
    /// rules): ↑ adds keyboard power, ↓ stops (brake, keyboard and
    /// virtual power 0).
    pub fn throttle(&mut self, key: &mut KeyThrottle, keys: ThrottleKeys, dt_s: f64, now_s: f64) {
        let (msgs, stop) = key.update(&mut self.client, keys, dt_s, now_s);
        if stop {
            self.set_power(0.0);
        }
        for m in msgs {
            self.send(&m);
        }
    }

    /// The HUD's status line (device and steering).
    pub fn status_line(&self) -> String {
        let brake = if self.client.braking() {
            " | BRAKE (up: ride on)"
        } else {
            ""
        };
        format!(
            "Virtual trainer {:.0} W | {}{brake}",
            self.power_w(),
            self.client.steering_status()
        )
    }

    /// `net.stats`: the connection and what it measured.
    pub fn stats_text(&self) -> String {
        let st = self.client.stats();
        let Some(r) = self.remote_link() else {
            return format!(
                "local (in the browser's frame loop): {} ticks, {} messages in, {} snapshots, {} samples out",
                self.ticks, st.messages_in, st.snapshots, st.samples_out
            );
        };
        let l = r.link.lock().expect("not poisoned");
        let c = l.counters();
        let status = match l.status() {
            LinkStatus::Connecting => "connecting".to_owned(),
            LinkStatus::Connected { datagrams: true } => "connected (datagrams)".to_owned(),
            LinkStatus::Connected { datagrams: false } => "connected (stream only)".to_owned(),
            LinkStatus::Closed(why) => format!("closed: {why}"),
        };
        let rtt = l
            .rtt_s()
            .map(|r| format!(", rtt {:.1} ms", r * 1000.0))
            .unwrap_or_default();
        format!(
            "server: remote, WebTransport {}: {status}{rtt}\nrider: {}\nin: {} frames, {} datagrams ({} malformed)\nout: {} frames, {} datagrams; {} samples, server applied up to #{}\nsnapshots: {}\nothers seen: {} ({})\nprediction error at last snapshot: {:.3} m",
            r.url,
            self.client
                .rider_id()
                .map(|r| r.to_string())
                .unwrap_or("-".into()),
            c.frames_in,
            c.datagrams_in,
            st.malformed,
            c.frames_out,
            c.datagrams_out,
            st.samples_out,
            r.acked_seq,
            r.timing.summary().line(),
            r.others_seen.len(),
            r.others_seen.iter().cloned().collect::<Vec<_>>().join(", "),
            self.client.prediction_error()
        )
    }

    /// The connection's numbers as JSON (`window.ocw.netStats()`, read by
    /// the browser test).
    pub fn stats_json(&self) -> String {
        let st = self.client.stats();
        let mut v = serde_json::json!({
            "remote": self.is_remote(),
            "rider_id": self.client.rider_id().map(|r| r.0),
            "snapshots": st.snapshots,
            "samples_out": st.samples_out,
            "distance_m": self.client.state().distance,
            "prediction_error_m": self.client.prediction_error(),
        });
        if let Some(r) = self.remote_link() {
            let l = r.link.lock().expect("not poisoned");
            let c = l.counters();
            v["url"] = r.url.clone().into();
            v["status"] = format!("{:?}", l.status()).into();
            v["rtt_ms"] = l.rtt_s().map(|r| r * 1000.0).into();
            v["frames_in"] = c.frames_in.into();
            v["datagrams_in"] = c.datagrams_in.into();
            v["frames_out"] = c.frames_out.into();
            v["datagrams_out"] = c.datagrams_out.into();
            v["acked_seq"] = r.acked_seq.into();
            v["others_seen"] = r.others_seen.iter().cloned().collect::<Vec<_>>().into();
            v["max_riders"] = r.max_riders.into();
            v["timing"] = serde_json::to_value(r.timing.summary()).unwrap_or_default();
        }
        v.to_string()
    }
}

impl RemoteLink {
    /// One received message: the snapshot's timing, the riders in it, and
    /// how far the server applied the own samples.
    /// `at_s`: the arrival on the link's clock (the wall clock in the
    /// browser, so the age means something there).
    fn observe(&mut self, bytes: &[u8], at_s: f64, own: Option<u32>) {
        if !server_unreliable(bytes) {
            return;
        }
        let Ok(S::Snapshot(s)) = decode_server(bytes) else {
            return;
        };
        let age = s
            .clock
            .as_ref()
            .map(|c| at_s * 1000.0 - (c.world_unix_ms - c.offset_ms) as f64);
        self.timing.record(at_s, s.server_tick, age);
        self.max_riders = self.max_riders.max(s.riders.len());
        for r in &s.riders {
            if Some(r.rider_id) == own {
                self.acked_seq = self.acked_seq.max(r.last_input_seq);
            } else if !r.display_name.is_empty() {
                self.others_seen.insert(r.display_name.clone());
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ocw_server::wire::{FrameDecoder, Outgoing, client_unreliable, encode_frame};
    use ocw_server::{ConnectionId, GameServer, ServerConfig};

    /// Runs `seconds` of 60 Hz frames, with the client's prediction at
    /// 50 Hz as Bevy's FixedUpdate does it.
    fn ride(s: &mut WebSession, seconds: f64, now: &mut f64) -> Vec<Notice> {
        let dt = 1.0 / 60.0;
        let mut notices = Vec::new();
        let mut fixed = 0.0;
        let frames = (seconds / dt).round() as u32;
        for _ in 0..frames {
            *now += dt;
            fixed += dt;
            while fixed >= ocw_core::SIM_DT {
                fixed -= ocw_core::SIM_DT;
                s.client.predict_step();
            }
            notices.extend(s.frame(dt, *now));
        }
        notices
    }

    #[test]
    fn the_rider_joins_and_rides_at_the_virtual_power() {
        let mut s = WebSession::new(SessionConfig::default(), None);
        let mut now = 0.0;
        let notices = ride(&mut s, 0.1, &mut now);
        assert!(
            notices.iter().any(|n| matches!(n, Notice::Joined { .. })),
            "{notices:?}"
        );
        assert!(s.client.rider_id().is_some());
        assert!(!s.is_remote() && s.link_notice().is_none());
        ride(&mut s, 10.0, &mut now);
        // 10.1 s at 50 Hz on the server.
        assert!((503..=506).contains(&s.ticks), "{}", s.ticks);
        assert_eq!(s.client.power_w(), DEFAULT_POWER_W);
        let state = s.client.state();
        assert!(state.speed > 1.5, "{state:?}");
        assert!(state.distance > 15.0, "{state:?}");
        // The local rider and the bots are in the view.
        let riders = s.client.view(now, 0.0);
        assert_eq!(riders.len(), 1 + DEFAULT_BOTS as usize);
        assert_eq!(riders.iter().filter(|r| r.is_local).count(), 1);
        // The browser's rider is the anonymous one (Q5).
        let me = riders.iter().find(|r| r.is_local).unwrap();
        assert_eq!(me.look, Some(RiderLook::anonymous()));
        assert!(s.stats_text().starts_with("local"));
    }

    #[test]
    fn the_power_stays_within_bounds() {
        let mut s = WebSession::new(SessionConfig::default(), None);
        assert_eq!(s.set_power(-50.0), 0.0);
        assert_eq!(s.set_power(5000.0), MAX_POWER_W);
        // The next sample brings the new power to the client and server.
        s.set_power(320.0);
        let mut now = 0.0;
        ride(&mut s, 0.5, &mut now);
        assert_eq!(s.client.power_w(), 320.0);
        assert!(s.status_line().starts_with("Virtual trainer 320 W"));
    }

    #[test]
    fn up_adds_keyboard_power_and_down_stops() {
        let mut s = WebSession::new(SessionConfig::default(), None);
        let mut key = KeyThrottle::default();
        let mut now = 0.0;
        ride(&mut s, 0.5, &mut now);
        let up = ThrottleKeys {
            up_pressed: true,
            up_held: true,
            ..ThrottleKeys::default()
        };
        s.throttle(&mut key, up, 0.0, now);
        assert_eq!(s.client.power_w(), 225.0);
        let down = ThrottleKeys {
            down_pressed: true,
            ..ThrottleKeys::default()
        };
        s.throttle(&mut key, down, 0.0, now);
        ride(&mut s, 0.5, &mut now);
        assert_eq!((s.power_w(), s.client.power_w()), (0.0, 0.0));
        assert!(s.client.braking());
        assert!(s.status_line().contains("BRAKE"));
    }

    #[test]
    fn no_power_no_progress() {
        let mut s = WebSession::new(
            SessionConfig {
                power_w: 0.0,
                bots: 0,
                ..SessionConfig::default()
            },
            None,
        );
        let mut now = 0.0;
        ride(&mut s, 5.0, &mut now);
        assert!(s.client.state().distance < 1.0);
        assert_eq!(s.client.view(now, 0.0).len(), 1);
    }

    /// What the fake network does with the link's bytes.
    #[derive(Clone, Copy, PartialEq)]
    pub(crate) enum FakeNet {
        /// Up after `connect_after_s`, datagrams both ways.
        Normal,
        /// Up, but the peer takes no datagrams (everything on the stream).
        NoDatagrams,
        /// Never comes up: the handshake fails.
        Refused,
    }

    /// A fake transport (spec §11.4: no sockets in unit tests): a remote
    /// `GameServer` in memory, ticked by the frame time, with the stream's
    /// framing and datagrams as the real endpoint has them. Datagrams can
    /// be reordered (every `swap_every`th pair).
    pub(crate) struct FakeTransport {
        pub game: Arc<Mutex<GameServer>>,
        pub conn: Option<ConnectionId>,
        pub mode: FakeNet,
        pub connect_after_s: f64,
        pub decoder: FrameDecoder,
        pub swap_every: usize,
        pub held: Option<Vec<u8>>,
        pub datagrams_seen: usize,
        pub last_tick_s: f64,
        /// Samples the server got as datagrams / on the stream.
        pub samples_as_datagrams: Arc<Mutex<(u64, u64)>>,
        pub closed: bool,
    }

    impl FakeTransport {
        pub(crate) fn new(game: Arc<Mutex<GameServer>>, mode: FakeNet) -> Self {
            Self {
                game,
                conn: None,
                mode,
                connect_after_s: 0.05,
                decoder: FrameDecoder::default(),
                swap_every: 0,
                held: None,
                datagrams_seen: 0,
                last_tick_s: 0.0,
                samples_as_datagrams: Arc::default(),
                closed: false,
            }
        }

        fn deliver(&mut self, l: &mut ClientLink, bytes: Vec<u8>, now_s: f64) {
            let datagram = server_unreliable(&bytes) && self.mode == FakeNet::Normal;
            if !datagram {
                l.on_stream_data(now_s, &encode_frame(&bytes).unwrap());
                return;
            }
            self.datagrams_seen += 1;
            if self.swap_every > 0 && self.datagrams_seen.is_multiple_of(self.swap_every) {
                // Held back: arrives after the next one.
                self.held = Some(bytes);
                return;
            }
            l.on_datagram(now_s, bytes);
            if let Some(h) = self.held.take() {
                l.on_datagram(now_s, h);
            }
        }
    }

    impl Transport for FakeTransport {
        fn pump(&mut self, link: &Arc<Mutex<ClientLink>>, now_s: f64) {
            let mut l = link.lock().unwrap();
            if self.closed || l.is_closed() {
                return;
            }
            if self.conn.is_none() {
                if now_s < self.connect_after_s {
                    return;
                }
                if self.mode == FakeNet::Refused {
                    l.on_closed("cannot connect: the certificate hash does not match");
                    return;
                }
                self.conn = Some(self.game.lock().unwrap().connect());
                let max = (self.mode == FakeNet::Normal).then_some(1200);
                l.on_connected(now_s, max);
                self.last_tick_s = now_s;
            }
            let conn = self.conn.expect("connected");
            let mut replies = Vec::new();
            for out in l.take_outgoing() {
                let msgs = match out {
                    Outgoing::Datagram(b) => {
                        assert!(client_unreliable(&b));
                        self.samples_as_datagrams.lock().unwrap().0 += 1;
                        vec![b]
                    }
                    Outgoing::Stream(b) => self.decoder.push(&b).unwrap(),
                };
                for m in msgs {
                    if client_unreliable(&m) && !matches!(self.mode, FakeNet::Normal) {
                        self.samples_as_datagrams.lock().unwrap().1 += 1;
                    }
                    replies.extend(self.game.lock().unwrap().handle(conn, &m));
                }
            }
            // The server's ticks for the time that passed.
            let mut game = self.game.lock().unwrap();
            while now_s - self.last_tick_s >= ocw_core::SIM_DT {
                self.last_tick_s += ocw_core::SIM_DT;
                replies.extend(game.tick());
            }
            drop(game);
            for r in replies {
                self.deliver(&mut l, r, now_s);
            }
        }

        fn close(&mut self) {
            self.closed = true;
            if let Some(c) = self.conn.take() {
                self.game.lock().unwrap().disconnect(c);
            }
        }
    }

    fn remote_game(bots: u32) -> Arc<Mutex<GameServer>> {
        Arc::new(Mutex::new(GameServer::new(
            Route::test_hills(),
            ServerConfig {
                bots,
                ..Default::default()
            },
        )))
    }

    fn remote_config() -> SessionConfig {
        SessionConfig {
            name: "Browser".into(),
            remote: Some(RemoteOptions {
                url: "https://127.0.0.1:47801/ocw/1".into(),
                cert_hashes: vec![[7; 32]],
            }),
            ..SessionConfig::default()
        }
    }

    /// The browser joins a remote session through the link state machine:
    /// "Connecting" until the transport is up, then the Welcome, the
    /// samples as datagrams (after the Welcome), snapshots with the other
    /// riders, and the server applies the browser's inputs.
    #[test]
    fn the_browser_joins_a_remote_session_and_rides() {
        let game = remote_game(2);
        let fake = FakeTransport::new(game.clone(), FakeNet::Normal);
        let counts = fake.samples_as_datagrams.clone();
        let mut s = WebSession::new(remote_config(), Some(Box::new(fake)));
        assert!(s.is_remote());
        assert_eq!(s.link_status(), Some(LinkStatus::Connecting));
        assert_eq!(
            s.link_notice().as_deref(),
            Some("Connecting to the server …")
        );
        let mut now = 0.0;
        let notices = ride(&mut s, 0.5, &mut now);
        assert!(
            notices.iter().any(|n| matches!(n, Notice::Joined { .. })),
            "{notices:?}"
        );
        assert_eq!(
            s.link_status(),
            Some(LinkStatus::Connected { datagrams: true })
        );
        assert!(s.link_notice().is_none());
        ride(&mut s, 10.0, &mut now);
        // No local server ran.
        assert_eq!(s.ticks, 0);
        let state = s.client.state();
        assert!(state.distance > 15.0, "{state:?}");
        // The server's truth: the browser's rider at the virtual power.
        {
            let g = game.lock().unwrap();
            let me = g
                .session()
                .snapshot()
                .into_iter()
                .find(|r| r.name == "Browser")
                .expect("the browser's rider on the server");
            assert_eq!(me.power_w, DEFAULT_POWER_W);
            assert!(me.state.distance > 15.0);
        }
        let r = s.remote_link().unwrap();
        assert_eq!(r.others_seen.len(), 2, "{:?}", r.others_seen);
        assert_eq!(r.max_riders, 3);
        assert!(r.acked_seq > 30, "{}", r.acked_seq);
        // Samples went as datagrams once the server had answered.
        let (dgrams, _) = *counts.lock().unwrap();
        assert!(dgrams > 30, "{dgrams}");
        // About 10 snapshots a second, evenly in this fake.
        let t = r.timing.summary();
        assert!((95..=110).contains(&t.snapshots), "{t:?}");
        assert!((t.interval_mean_ms - 100.0).abs() < 5.0, "{t:?}");
        assert!(t.age_mean_ms.is_some());
        // The others ride in the view next to the own rider.
        let riders = s.client.view(now, 0.0);
        assert_eq!(riders.len(), 3);
        assert!(s.stats_text().contains("WebTransport"));
        let json: serde_json::Value = serde_json::from_str(&s.stats_json()).unwrap();
        assert_eq!(json["remote"], true);
        assert_eq!(json["others_seen"].as_array().unwrap().len(), 2);
        assert!(json["timing"]["snapshots"].as_u64().unwrap() > 90);
        // Closing: the server drops the rider, the HUD says so.
        s.close();
        assert_eq!(game.lock().unwrap().session().rider_count(), 2);
        assert_eq!(
            s.link_notice().as_deref(),
            Some("Disconnected: connection closed")
        );
    }

    /// Without datagrams everything goes on the stream and still rides;
    /// reordered datagrams are dropped by their tick.
    #[test]
    fn the_remote_session_survives_no_datagrams_and_reordering() {
        let game = remote_game(1);
        let fake = FakeTransport::new(game.clone(), FakeNet::NoDatagrams);
        let counts = fake.samples_as_datagrams.clone();
        let mut s = WebSession::new(remote_config(), Some(Box::new(fake)));
        let mut now = 0.0;
        ride(&mut s, 5.0, &mut now);
        assert_eq!(
            s.link_status(),
            Some(LinkStatus::Connected { datagrams: false })
        );
        assert!(s.client.state().distance > 5.0);
        let (dgrams, on_stream) = *counts.lock().unwrap();
        assert_eq!(dgrams, 0);
        assert!(on_stream > 10);
        // Reordered snapshots.
        let mut fake = FakeTransport::new(remote_game(1), FakeNet::Normal);
        fake.swap_every = 3;
        let mut s = WebSession::new(remote_config(), Some(Box::new(fake)));
        let mut now = 0.0;
        ride(&mut s, 5.0, &mut now);
        let r = s.remote_link().unwrap();
        let t = r.timing.summary();
        assert!(t.stale > 5, "{t:?}");
        assert!(s.client.stats().stale_snapshots > 5);
        assert!(s.client.state().distance > 5.0);
    }

    /// A refused connection (a wrong certificate hash) shows in the HUD
    /// and nothing rides.
    #[test]
    fn a_refused_connection_is_shown() {
        let game = remote_game(0);
        let mut s = WebSession::new(
            remote_config(),
            Some(Box::new(FakeTransport::new(game.clone(), FakeNet::Refused))),
        );
        let mut now = 0.0;
        ride(&mut s, 1.0, &mut now);
        assert!(matches!(s.link_status(), Some(LinkStatus::Closed(_))));
        let line = s.link_notice().unwrap();
        assert!(line.starts_with("Disconnected: cannot connect"), "{line}");
        assert!(s.client.rider_id().is_none());
        assert_eq!(game.lock().unwrap().session().rider_count(), 0);
    }

    /// The configuration picks the server: a remote one needs a transport.
    #[test]
    fn remote_needs_a_transport() {
        let s = WebSession::new(remote_config(), None);
        assert!(!s.is_remote(), "no transport: the local server");
    }
}
