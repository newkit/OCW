//! Engine-independent client logic: device samples → server, server messages →
//! prediction (own rider) and interpolation (others) → render view.
//!
//! The Bevy systems in `connection.rs` only move bytes in and out of this type,
//! so the whole client loop is unit-testable without a window or GPU.

use std::collections::BTreeMap;

use ocw_core::{INTERPOLATION_DELAY_S, RiderId, RiderProfile};
use ocw_devices::DeviceEvent;
use ocw_protocol::v1::{self, client_message::Msg as C, server_message::Msg as S};
use ocw_protocol::{PROTOCOL_VERSION, client_message, decode_server, dequantize, encode};
use ocw_render::RiderView;
use ocw_sim::{Predictor, RiderState, SnapshotBuffer};
use ocw_world::Route;

/// Minimum time between steering messages while the target changes.
pub const STEER_SEND_INTERVAL_S: f64 = 0.05;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NetStats {
    pub messages_in: u64,
    pub malformed: u64,
    pub snapshots: u64,
    pub samples_out: u64,
    pub last_snapshot_tick: u64,
}

/// Something the user should be told about (shown in the console).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    Joined { rider_id: RiderId, route_id: String },
    Rejected(String),
    CommandResult(String),
}

pub struct ClientCore {
    route: Route,
    profile: RiderProfile,
    name: String,
    rider_id: Option<RiderId>,
    predictor: Predictor,
    /// Predicted state before the last physics step: rendering interpolates
    /// between it and the current one, so motion is smooth at any frame rate.
    prev_state: RiderState,
    /// Steering target (lateral offset); set from the first own snapshot.
    lateral_target: Option<f64>,
    steer_seq: u32,
    /// Last steering target sent (cm) and when.
    steer_sent: Option<(i32, f64)>,
    others: SnapshotBuffer,
    /// Latest known state of the other riders, advanced every physics tick
    /// (for predicting the local rider's drafting).
    others_now: Vec<RiderState>,
    names: BTreeMap<RiderId, (String, f64)>,
    power_w: f64,
    cadence_rpm: Option<f64>,
    heart_rate_bpm: Option<f64>,
    braking: bool,
    last_sample_us: u64,
    seq: u32,
    stats: NetStats,
}

impl ClientCore {
    pub fn new(route: Route, name: impl Into<String>, profile: RiderProfile) -> Self {
        Self {
            route,
            profile,
            name: name.into(),
            rider_id: None,
            predictor: Predictor::new(profile, RiderState::default()),
            prev_state: RiderState::default(),
            lateral_target: None,
            steer_seq: 0,
            steer_sent: None,
            others: SnapshotBuffer::default(),
            others_now: Vec::new(),
            names: BTreeMap::new(),
            power_w: 0.0,
            cadence_rpm: None,
            heart_rate_bpm: None,
            braking: false,
            last_sample_us: 0,
            seq: 0,
            stats: NetStats::default(),
        }
    }

    /// The first message to send after connecting.
    pub fn hello(&self) -> Vec<u8> {
        encode(&client_message(C::Hello(v1::Hello {
            protocol_version: PROTOCOL_VERSION,
            display_name: self.name.clone(),
            profile: Some(self.profile.into()),
        })))
    }

    /// Handles a device event; returns the encoded sample to send, if any.
    /// Heart rate is stored and sent with the next power sample.
    pub fn on_device_event(&mut self, event: DeviceEvent) -> Option<Vec<u8>> {
        let (watts, cadence_rpm, timestamp) = match event {
            DeviceEvent::Power {
                watts,
                cadence_rpm,
                timestamp,
            } => (watts, cadence_rpm, timestamp),
            DeviceEvent::HeartRate { bpm, .. } => {
                self.heart_rate_bpm = Some(bpm);
                return None;
            }
            DeviceEvent::Disconnected => return None,
        };
        self.power_w = watts;
        self.cadence_rpm = cadence_rpm;
        self.last_sample_us = timestamp.as_micros() as u64;
        Some(self.sample())
    }

    /// Encodes the current inputs as the next sensor sample.
    fn sample(&mut self) -> Vec<u8> {
        self.seq += 1;
        self.stats.samples_out += 1;
        encode(&client_message(C::Sample(v1::SensorSample {
            seq: self.seq,
            client_time_us: self.last_sample_us,
            power_w: Some(self.power_w as f32),
            cadence_rpm: self.cadence_rpm.map(|c| c as f32),
            heart_rate_bpm: self.heart_rate_bpm.map(|h| h as f32),
            brake: Some(self.braking),
        })))
    }

    /// Brake key pressed or released; returns a sample to send at once when
    /// the state changed (the next trainer sample may be 250 ms away).
    pub fn set_brake(&mut self, braking: bool) -> Option<Vec<u8>> {
        if braking == self.braking {
            return None;
        }
        self.braking = braking;
        self.predictor.set_braking(braking);
        Some(self.sample())
    }

    pub fn braking(&self) -> bool {
        self.braking
    }

    /// Handles one encoded server message received at client time `now_s`.
    pub fn on_server_message(&mut self, bytes: &[u8], now_s: f64) -> Option<Notice> {
        self.stats.messages_in += 1;
        let Ok(msg) = decode_server(bytes) else {
            self.stats.malformed += 1;
            return None;
        };
        match msg {
            S::Welcome(w) => {
                let id = RiderId(w.rider_id);
                self.rider_id = Some(id);
                Some(Notice::Joined {
                    rider_id: id,
                    route_id: w.route_id,
                })
            }
            S::Reject(r) => Some(Notice::Rejected(r.reason)),
            S::DevResult(r) => Some(Notice::CommandResult(if r.ok {
                r.message
            } else {
                format!("error: {}", r.message)
            })),
            S::Snapshot(s) => {
                self.on_snapshot(&s, now_s);
                None
            }
        }
    }

    fn on_snapshot(&mut self, s: &v1::Snapshot, now_s: f64) {
        self.stats.snapshots += 1;
        self.stats.last_snapshot_tick = s.server_tick;
        let mut others = Vec::new();
        for u in &s.riders {
            let (id, state) = dequantize(u);
            self.names
                .insert(id, (u.display_name.clone(), f64::from(u.power_w)));
            if Some(id) == self.rider_id {
                // In-process server: the snapshot is fresh (age ≈ 0).
                if self.lateral_target.is_none() {
                    // Start steering from the lane the server assigned.
                    self.lateral_target = Some(state.lateral_offset);
                    self.predictor.set_lateral_target(state.lateral_offset);
                }
                self.predictor
                    .reconcile(&self.route, state, 0.0, self.power_w);
                // A snap (teleport, large error) must not be interpolated.
                if self.predictor.last_error().abs() > ocw_sim::prediction::SNAP_THRESHOLD_M {
                    self.prev_state = self.predictor.state();
                }
            } else {
                others.push((id, state));
            }
        }
        self.others_now = others.iter().map(|(_, s)| *s).collect();
        self.others.push(now_s, others);
    }

    /// Moves the steering target sideways by `delta_m` (positive = right),
    /// within the road.
    pub fn steer_by(&mut self, delta_m: f64) {
        if let Some(t) = self.lateral_target {
            self.steer_to(t + delta_m);
        }
    }

    /// Sets the steering target (lateral offset in metres), within the road.
    pub fn steer_to(&mut self, target_m: f64) {
        if self.rider_id.is_none() {
            return;
        }
        let limit = ocw_sim::physics::lateral_limit(self.route.road_width_m());
        let t = target_m.clamp(-limit, limit);
        self.lateral_target = Some(t);
        self.predictor.set_lateral_target(t);
    }

    pub fn lateral_target(&self) -> Option<f64> {
        self.lateral_target
    }

    /// The steering intent to send now, if the target changed (at most
    /// every `STEER_SEND_INTERVAL_S` while it keeps changing).
    pub fn steer_message(&mut self, now_s: f64) -> Option<Vec<u8>> {
        let cm = (self.lateral_target? * 100.0).round() as i32;
        if let Some((sent_cm, at)) = self.steer_sent
            && (sent_cm == cm || now_s - at < STEER_SEND_INTERVAL_S)
        {
            return None;
        }
        self.steer_seq += 1;
        self.steer_sent = Some((cm, now_s));
        Some(encode(&client_message(C::Steer(v1::SteerIntent {
            seq: self.steer_seq,
            lateral_target_cm: cm,
        }))))
    }

    /// One fixed physics tick of the local prediction.
    pub fn predict_step(&mut self) {
        if self.rider_id.is_none() {
            return;
        }
        let me = self.predictor.state();
        let drag = ocw_sim::drafting::drag_factor(
            (me.distance, me.lateral_offset),
            self.others_now
                .iter()
                .map(|o| (o.distance, o.lateral_offset)),
        );
        self.predictor.set_drag_factor(drag);
        self.prev_state = self.predictor.state();
        self.predictor.step(&self.route, self.power_w);
        for o in &mut self.others_now {
            o.distance += o.speed * ocw_core::SIM_DT;
        }
    }

    /// Riders to render at client time `now_s`: the local rider first
    /// (predicted), then the others (interpolated, slightly in the past).
    /// `alpha` (0–1) is how far the frame lies between the last two physics
    /// steps (Bevy's `Time<Fixed>::overstep_fraction`).
    pub fn view(&self, now_s: f64, alpha: f64) -> Vec<RiderView> {
        let Some(own) = self.rider_id else {
            return Vec::new();
        };
        let mut riders = vec![RiderView {
            id: own,
            name: self
                .names
                .get(&own)
                .map(|n| n.0.clone())
                .unwrap_or_default(),
            state: lerp_state(self.prev_state, self.predictor.state(), alpha),
            power_w: self.power_w,
            cadence_rpm: self.cadence_rpm,
            heart_rate_bpm: self.heart_rate_bpm,
            is_local: true,
        }];
        for (id, state) in self.others.sample(now_s - INTERPOLATION_DELAY_S) {
            let (name, power_w) = self.names.get(&id).cloned().unwrap_or_default();
            riders.push(RiderView {
                id,
                name,
                state,
                power_w,
                cadence_rpm: None,
                heart_rate_bpm: None,
                is_local: false,
            });
        }
        riders
    }

    pub fn route(&self) -> &Route {
        &self.route
    }
    pub fn rider_id(&self) -> Option<RiderId> {
        self.rider_id
    }
    pub fn state(&self) -> RiderState {
        self.predictor.state()
    }
    pub fn power_w(&self) -> f64 {
        self.power_w
    }
    pub fn cadence_rpm(&self) -> Option<f64> {
        self.cadence_rpm
    }
    pub fn prediction_error(&self) -> f64 {
        self.predictor.last_error()
    }
    pub fn stats(&self) -> &NetStats {
        &self.stats
    }
}

/// Linear interpolation of a rider state (`t` clamped to 0–1).
fn lerp_state(a: RiderState, b: RiderState, t: f64) -> RiderState {
    let t = t.clamp(0.0, 1.0);
    RiderState {
        distance: a.distance + (b.distance - a.distance) * t,
        speed: a.speed + (b.speed - a.speed) * t,
        lateral_offset: a.lateral_offset + (b.lateral_offset - a.lateral_offset) * t,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use ocw_server::{GameServer, ServerConfig};

    use super::*;

    fn route() -> Route {
        Route::test_loop()
    }

    fn power(watts: f64, t: f64) -> DeviceEvent {
        DeviceEvent::Power {
            watts,
            cadence_rpm: Some(90.0),
            timestamp: Duration::from_secs_f64(t),
        }
    }

    /// A client connected to an in-memory `GameServer`, driven tick by tick.
    struct Harness {
        server: GameServer,
        conn: ocw_server::ConnectionId,
        client: ClientCore,
        notices: Vec<Notice>,
        time_s: f64,
    }

    impl Harness {
        fn new(bots: u32, allow_dev: bool) -> Self {
            let mut server = GameServer::new(
                route(),
                ServerConfig {
                    allow_dev_commands: allow_dev,
                    bots,
                },
            );
            let conn = server.connect();
            let client = ClientCore::new(route(), "Tester", RiderProfile::default());
            let mut h = Self {
                server,
                conn,
                client,
                notices: Vec::new(),
                time_s: 0.0,
            };
            let hello = h.client.hello();
            h.send(&hello);
            h
        }

        fn send(&mut self, bytes: &[u8]) {
            for reply in self.server.handle(self.conn, bytes) {
                self.receive(&reply);
            }
        }

        fn receive(&mut self, bytes: &[u8]) {
            if let Some(n) = self.client.on_server_message(bytes, self.time_s) {
                self.notices.push(n);
            }
        }

        /// Runs `seconds` of simulated time at the real rates: 50 Hz server and
        /// prediction, 4 Hz trainer samples.
        fn ride(&mut self, watts: f64, seconds: f64) {
            let ticks = (seconds * f64::from(ocw_core::SIM_TICK_HZ)).round() as u32;
            for _ in 0..ticks {
                if (self.time_s * 4.0).fract() < 1e-9
                    && let Some(sample) = self.client.on_device_event(power(watts, self.time_s))
                {
                    self.send(&sample);
                }
                if let Some(snapshot) = self.server.tick() {
                    self.receive(&snapshot);
                }
                self.client.predict_step();
                self.time_s += ocw_core::SIM_DT;
                // Avoid drift so the 4 Hz check stays exact.
                self.time_s = (self.time_s * 1000.0).round() / 1000.0;
            }
        }
    }

    #[test]
    fn joins_and_shows_nothing_before_welcome() {
        let client = ClientCore::new(route(), "A", RiderProfile::default());
        assert!(client.view(0.0, 1.0).is_empty());
        let mut h = Harness::new(0, false);
        assert!(matches!(h.notices[0], Notice::Joined { .. }));
        assert!(h.client.rider_id().is_some());
        h.ride(200.0, 0.0);
        assert_eq!(h.client.view(0.0, 1.0).len(), 1);
    }

    #[test]
    fn device_samples_have_increasing_sequence_numbers() {
        let mut c = ClientCore::new(route(), "A", RiderProfile::default());
        for i in 1..=3u32 {
            let bytes = c.on_device_event(power(150.0, f64::from(i))).unwrap();
            match ocw_protocol::decode_client(&bytes).unwrap() {
                C::Sample(s) => {
                    assert_eq!(s.seq, i);
                    assert_eq!(s.power_w, Some(150.0));
                }
                other => panic!("unexpected {other:?}"),
            }
        }
        assert_eq!(c.stats().samples_out, 3);
        assert!(
            c.on_device_event(DeviceEvent::HeartRate {
                bpm: 120.0,
                timestamp: Duration::ZERO,
            })
            .is_none()
        );
        // Heart rate is sent along with the next power sample.
        let bytes = c.on_device_event(power(150.0, 4.0)).unwrap();
        match ocw_protocol::decode_client(&bytes).unwrap() {
            C::Sample(s) => assert_eq!(s.heart_rate_bpm, Some(120.0)),
            other => panic!("unexpected {other:?}"),
        }
        assert!(c.on_device_event(DeviceEvent::Disconnected).is_none());
    }

    /// Desktop client ↔ server through the real protocol (spec §14, Phase 1).
    #[test]
    fn prediction_tracks_server_over_a_long_ride() {
        let mut h = Harness::new(3, false);
        h.ride(250.0, 60.0);
        let own = h.client.rider_id().unwrap();
        let server = h.server.session().rider_state(own).unwrap();
        let predicted = h.client.state();
        assert!(
            predicted.distance > 300.0,
            "rode {:.0} m",
            predicted.distance
        );
        // Same physics on both sides: only quantization and the sub-snapshot
        // tick offset remain.
        assert!(
            (server.distance - predicted.distance).abs() < 0.5,
            "server {:.2} m, predicted {:.2} m",
            server.distance,
            predicted.distance
        );
        assert!(h.client.prediction_error().abs() < 0.5);
        assert!(h.client.stats().snapshots >= 590);
        assert_eq!(h.client.stats().malformed, 0);
    }

    #[test]
    fn view_contains_local_rider_first_and_interpolated_bots() {
        let mut h = Harness::new(3, false);
        h.ride(200.0, 5.0);
        let view = h.client.view(h.time_s, 1.0);
        assert_eq!(view.len(), 4);
        assert!(view[0].is_local && view[0].name == "Tester");
        assert_eq!(view.iter().filter(|r| r.is_local).count(), 1);
        for bot in &view[1..] {
            assert!(!bot.name.is_empty());
            assert!(bot.state.distance > 0.0);
            assert!(bot.power_w > 0.0);
        }
    }

    #[test]
    fn teleport_snaps_prediction_on_next_snapshot() {
        let mut h = Harness::new(0, true);
        h.ride(200.0, 1.0);
        let cmd = encode(&client_message(C::Dev(v1::DevCommand {
            cmd: Some(v1::dev_command::Cmd::TeleportDistanceM(2000.0)),
        })));
        h.send(&cmd);
        assert!(
            matches!(h.notices.last(), Some(Notice::CommandResult(m)) if !m.starts_with("error"))
        );
        h.ride(200.0, 0.2);
        assert!((h.client.state().distance - 2000.0).abs() < 5.0);
    }

    #[test]
    fn dev_commands_are_refused_by_a_public_server() {
        let mut h = Harness::new(0, false);
        let cmd = encode(&client_message(C::Dev(v1::DevCommand {
            cmd: Some(v1::dev_command::Cmd::TeleportDistanceM(2000.0)),
        })));
        h.send(&cmd);
        assert!(
            matches!(h.notices.last(), Some(Notice::CommandResult(m)) if m.starts_with("error"))
        );
    }

    #[test]
    fn prediction_is_continuous_across_the_loop_seam() {
        let mut h = Harness::new(0, true);
        let len = h.client.route().length();
        let cmd = encode(&client_message(C::Dev(v1::DevCommand {
            cmd: Some(v1::dev_command::Cmd::TeleportDistanceM(len - 20.0)),
        })));
        h.send(&cmd);
        h.ride(300.0, 0.2);
        let mut last = h.client.state().distance;
        for _ in 0..30 {
            h.ride(300.0, 0.5);
            let d = h.client.state().distance;
            // Never jumps backwards or by more than a few metres per step.
            assert!(d >= last && d - last < 10.0, "{last:.2} -> {d:.2}");
            last = d;
        }
        assert!(last > len, "crossed the seam: {last:.1} of {len:.1} m");
        // Rendering wraps the cumulative distance onto the loop.
        let p = h.client.route().sample(last).position;
        let q = h.client.route().sample(last - len).position;
        assert!((p[0] - q[0]).abs() < 1e-6 && (p[2] - q[2]).abs() < 1e-6);
    }

    #[test]
    fn rejection_and_malformed_messages_are_reported() {
        let mut c = ClientCore::new(route(), "A", RiderProfile::default());
        assert_eq!(c.on_server_message(&[0xff, 0x01], 0.0), None);
        assert_eq!(c.stats().malformed, 1);
        let reject = encode(&ocw_protocol::server_message(S::Reject(v1::Reject {
            reason: "full".into(),
        })));
        assert_eq!(
            c.on_server_message(&reject, 0.0),
            Some(Notice::Rejected("full".into()))
        );
        assert!(c.rider_id().is_none());
    }

    /// The real wiring: client core ↔ `LocalServer` thread ↔ virtual trainer thread.
    #[test]
    fn rides_against_threaded_local_server_and_virtual_trainer() {
        use ocw_devices::{DeviceBackend, VirtualTrainer};
        use std::time::Instant;

        let server = ocw_server::local::LocalServer::spawn(route(), 2).unwrap();
        let trainer = VirtualTrainer::new(300.0).start().unwrap();
        let mut client = ClientCore::new(route(), "Live", RiderProfile::default());
        server.to_server.send(client.hello()).unwrap();
        let start = Instant::now();
        let mut next_tick = 0.0;
        while start.elapsed() < Duration::from_millis(1500) {
            let now = start.elapsed().as_secs_f64();
            for e in trainer.events.try_iter() {
                if let Some(sample) = client.on_device_event(e) {
                    server.to_server.send(sample).unwrap();
                }
            }
            for msg in server.from_server.try_iter() {
                client.on_server_message(&msg, now);
            }
            while next_tick <= now {
                client.predict_step();
                next_tick += ocw_core::SIM_DT;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let _ = trainer.commands.send(ocw_devices::DeviceCommand::Shutdown);
        assert!(client.rider_id().is_some());
        assert!(client.power_w() > 250.0);
        assert!(client.stats().snapshots >= 10);
        assert!(client.state().distance > 0.5);
        assert_eq!(client.view(start.elapsed().as_secs_f64(), 1.0).len(), 3);
    }

    /// Drafting a bot: the client predicts the server's drafting, so the
    /// prediction stays close (spec §5.2).
    #[test]
    fn prediction_includes_drafting() {
        let mut h = Harness::new(1, true);
        let bot = h
            .server
            .session()
            .snapshot()
            .into_iter()
            .find(|r| Some(r.id) != h.client.rider_id())
            .unwrap();
        // Put the client rider 3 m behind the bot.
        let cmd = encode(&client_message(C::Dev(v1::DevCommand {
            cmd: Some(v1::dev_command::Cmd::TeleportDistanceM(
                bot.state.distance - 3.0,
            )),
        })));
        h.send(&cmd);
        h.ride(220.0, 30.0);
        assert!(
            h.client.prediction_error().abs() < 0.3,
            "error {}",
            h.client.prediction_error()
        );
    }

    /// Renders at 144 Hz with 50 Hz physics steps: the rider must move by
    /// (nearly) the same amount every frame, not in 0-or-1-step jumps
    /// (regression test for the bouncing rider and scenery).
    #[test]
    fn rendered_motion_is_smooth_between_physics_steps() {
        let mut h = Harness::new(0, false);
        h.ride(250.0, 8.0); // get up to speed
        let dt = 1.0 / 144.0;
        let mut acc = 0.0;
        let mut last = None;
        let mut moves = Vec::new();
        for frame in 0..288 {
            acc += dt;
            while acc >= ocw_core::SIM_DT {
                h.client.predict_step();
                acc -= ocw_core::SIM_DT;
            }
            let alpha = acc / ocw_core::SIM_DT;
            let d = h.client.view(h.time_s + f64::from(frame) * dt, alpha)[0]
                .state
                .distance;
            if let Some(prev) = last {
                moves.push(d - prev);
            }
            last = Some(d);
        }
        let mean = moves.iter().sum::<f64>() / moves.len() as f64;
        let (min, max) = moves.iter().fold((f64::INFINITY, 0.0f64), |(lo, hi), m| {
            (lo.min(*m), hi.max(*m))
        });
        assert!(mean > 0.03, "rider should move: {mean}");
        assert!(
            min > mean * 0.8 && max < mean * 1.2,
            "uneven motion per frame: min {min:.4}, mean {mean:.4}, max {max:.4} m"
        );
    }

    /// Steering: the client reacts at once, sends the intent (rate
    /// limited), and the server ends up at the same lateral offset.
    #[test]
    fn steering_is_predicted_and_applied_by_the_server() {
        let mut h = Harness::new(0, false);
        h.ride(200.0, 1.0);
        let start = h.client.state().lateral_offset;
        assert_eq!(h.client.lateral_target(), Some(start));
        // The first intent is sent once; unchanged targets are not resent.
        let first = h.client.steer_message(h.time_s).expect("initial intent");
        h.send(&first);
        assert!(h.client.steer_message(h.time_s + 1.0).is_none());
        h.client.steer_by(1.5);
        let msg = h.client.steer_message(h.time_s + 0.1).expect("changed");
        match ocw_protocol::decode_client(&msg).unwrap() {
            C::Steer(s) => assert_eq!(s.lateral_target_cm, ((start + 1.5) * 100.0).round() as i32),
            other => panic!("unexpected {other:?}"),
        }
        // Rate limited: a further change right away waits.
        h.client.steer_by(0.1);
        assert!(h.client.steer_message(h.time_s + 0.11).is_none());
        let msg = h
            .client
            .steer_message(h.time_s + 0.16)
            .expect("after the interval");
        h.send(&msg);
        h.ride(200.0, 3.0);
        let own = h.client.rider_id().unwrap();
        let server = h.server.session().rider_state(own).unwrap().lateral_offset;
        assert!((server - (start + 1.6)).abs() < 1e-6, "server {server}");
        assert!((h.client.state().lateral_offset - server).abs() < 0.02);
        // Never off the road.
        h.client.steer_by(100.0);
        assert_eq!(
            h.client.lateral_target(),
            Some(ocw_sim::physics::lateral_limit(7.0))
        );
    }

    /// Down arrow: the brake reaches the server at once and stops the
    /// rider; the prediction brakes too.
    #[test]
    fn braking_stops_the_rider_on_server_and_client() {
        let mut h = Harness::new(0, false);
        h.ride(250.0, 15.0);
        assert!(h.client.state().speed > 5.0);
        let msg = h.client.set_brake(true).expect("brake sent at once");
        match ocw_protocol::decode_client(&msg).unwrap() {
            C::Sample(s) => assert_eq!(s.brake, Some(true)),
            other => panic!("unexpected {other:?}"),
        }
        assert!(h.client.set_brake(true).is_none(), "unchanged");
        h.send(&msg);
        h.ride(0.0, 5.0);
        let own = h.client.rider_id().unwrap();
        assert_eq!(h.server.session().rider_state(own).unwrap().speed, 0.0);
        assert!(h.client.state().speed < 0.05, "{}", h.client.state().speed);
        // Released: riding again.
        let msg = h.client.set_brake(false).unwrap();
        h.send(&msg);
        h.ride(250.0, 3.0);
        assert!(h.client.state().speed > 1.0);
    }
}
