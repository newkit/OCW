//! One remote bot rider, Bevy-free and without a network: encoded server
//! messages in, encoded client messages out (like the desktop's
//! `ClientCore`, which needs the renderer's types). It predicts its own
//! rider with the same [`Predictor`] and records what the multiplayer test
//! checks (spec §11.4): the Welcome, snapshots and their gaps, the other
//! riders it sees and whether they move, its own progress, whether the
//! server applies its samples, and the prediction error.

use std::collections::BTreeMap;

use ocw_core::{RiderId, RiderLook, RiderProfile, SIM_DT};
use ocw_protocol::v1::{self, client_message::Msg as C, server_message::Msg as S};
use ocw_protocol::{PROTOCOL_VERSION, client_message, decode_server, dequantize, encode};
use ocw_sim::{Predictor, RiderState};
use ocw_world::Route;

use crate::pacing::{Effort, Leader, Pacing};
use crate::report::RiderReport;

/// A trainer reports about four times a second (like the virtual trainer).
pub const SAMPLE_INTERVAL_S: f64 = 0.25;
/// Prediction errors are counted from this long after the first own
/// snapshot (the start: the formation, the first reconcile).
pub const PREDICTION_WARMUP_S: f64 = 3.0;
/// Another rider "moves" once it has covered this distance (m).
pub const MOVING_M: f64 = 5.0;

/// Static settings of one bot rider.
#[derive(Debug, Clone)]
pub struct RiderConfig {
    pub name: String,
    pub weight_kg: f64,
    pub bike: ocw_core::ProfileId,
    pub look: RiderLook,
    pub pacing: Pacing,
}

/// What another rider did while this bot watched.
#[derive(Debug, Clone, PartialEq)]
struct Other {
    name: String,
    first_m: f64,
    last_m: f64,
}

pub struct BotRider {
    config: RiderConfig,
    route: Route,
    hello: v1::Hello,
    profile: RiderProfile,
    predictor: Predictor,
    rider_id: Option<RiderId>,
    /// Latest state of the other riders, advanced every tick (drafting in
    /// the prediction, like the desktop client).
    others_now: Vec<(RiderId, RiderState)>,
    others: BTreeMap<RiderId, Other>,
    leader: Option<Leader>,
    effort: Effort,
    seq: u32,
    /// When the next sample is due (a fixed 4 Hz schedule).
    next_sample_s: Option<f64>,
    snapshot_age_s: f64,
    report: RiderReport,
}

impl BotRider {
    pub fn new(config: RiderConfig, route: Route) -> Self {
        let hello = v1::Hello {
            protocol_version: PROTOCOL_VERSION,
            display_name: config.name.clone(),
            profile: None,
            // A route ride: no road graph (spec §6.5 checks only those).
            world: None,
            rider_weight_kg: Some(config.weight_kg as f32),
            bike_profile: Some(config.bike.id().into()),
            look: Some(config.look.into()),
        };
        // Exactly the profile the server will use for this hello.
        let profile = ocw_server::physics_profile(&hello);
        let report = RiderReport {
            name: config.name.clone(),
            ..Default::default()
        };
        Self {
            predictor: Predictor::new(profile, RiderState::default()),
            config,
            route,
            hello,
            profile,
            rider_id: None,
            others_now: Vec::new(),
            others: BTreeMap::new(),
            leader: None,
            effort: Effort {
                power_w: 0.0,
                brake: false,
            },
            seq: 0,
            next_sample_s: None,
            snapshot_age_s: 0.0,
            report,
        }
    }

    pub fn name(&self) -> &str {
        &self.config.name
    }

    pub fn profile(&self) -> &RiderProfile {
        &self.profile
    }

    /// The first message after connecting.
    pub fn hello(&self) -> Vec<u8> {
        encode(&client_message(C::Hello(self.hello.clone())))
    }

    /// Joined (Welcome accepted) and not closed.
    pub fn riding(&self) -> bool {
        self.rider_id.is_some() && self.report.closed.is_none()
    }

    pub fn closed(&self) -> bool {
        self.report.closed.is_some()
    }

    /// The predicted own state.
    pub fn state(&self) -> RiderState {
        self.predictor.state()
    }

    /// How old snapshots are on arrival (half the round trip).
    pub fn set_snapshot_age(&mut self, age_s: f64) {
        self.snapshot_age_s = age_s.clamp(0.0, 1.0);
        self.report.rtt_ms_max = self.report.rtt_ms_max.max(2000.0 * self.snapshot_age_s);
    }

    /// The link ended (`reason` from the transport) at client time `now_s`.
    pub fn close(&mut self, reason: &str, now_s: f64) {
        if self.report.closed.is_none() {
            self.report.closed = Some(reason.to_owned());
            self.report.closed_at_s = now_s;
        }
    }

    /// Handles one encoded server message received at client time `now_s`.
    pub fn on_message(&mut self, bytes: &[u8], now_s: f64) {
        let Ok(msg) = decode_server(bytes) else {
            self.report.malformed += 1;
            return;
        };
        match msg {
            S::Welcome(w) => self.on_welcome(&w, now_s),
            S::Reject(r) => {
                self.report.rejected = Some(r.reason.clone());
                self.close(&format!("rejected: {}", r.reason), now_s);
            }
            S::Snapshot(s) => self.on_snapshot(&s, now_s),
            // Dev results, events, paths: not used by bots.
            _ => {}
        }
    }

    fn on_welcome(&mut self, w: &v1::Welcome, now_s: f64) {
        self.report.route_id = w.route_id.clone();
        // Riders are distances along the course: the bot must ride the
        // server's (the desktop refuses a mismatch the same way).
        let problem = if w.free_ride {
            Some("the server rides a road graph (free ride / graph route); bots ride route files and test routes only".to_owned())
        } else if w.route_id != self.route.id() {
            Some(format!(
                "course mismatch: the server rides {:?}, this bot {:?}",
                w.route_id,
                self.route.id()
            ))
        } else {
            None
        };
        if let Some(p) = problem {
            self.report.rejected = Some(p.clone());
            self.close(&p, now_s);
            return;
        }
        self.rider_id = Some(RiderId(w.rider_id));
        self.report.joined_at_s = Some(now_s);
    }

    fn on_snapshot(&mut self, s: &v1::Snapshot, now_s: f64) {
        let r = &mut self.report;
        if r.snapshots > 0 && s.server_tick <= r.last_tick {
            r.stale_snapshots += 1;
            return;
        }
        if let Some(prev) = r.last_snapshot_s {
            r.max_snapshot_gap_s = r.max_snapshot_gap_s.max(now_s - prev);
        }
        r.snapshots += 1;
        r.last_tick = s.server_tick;
        r.last_snapshot_s = Some(now_s);
        r.first_snapshot_s.get_or_insert(now_s);
        r.max_riders_seen = r.max_riders_seen.max(s.riders.len() as u32);
        let Some(own) = self.rider_id else { return };
        let mut others = Vec::new();
        self.leader = None;
        for u in &s.riders {
            let (id, state) = dequantize(u);
            if id == own {
                self.on_own(state, u.last_input_seq, now_s);
                continue;
            }
            others.push((id, state));
            let o = self.others.entry(id).or_insert_with(|| Other {
                name: u.display_name.clone(),
                first_m: state.distance,
                last_m: state.distance,
            });
            o.last_m = state.distance;
            if let Some((name, _)) = &self.config.pacing.follow
                && *name == u.display_name
            {
                self.leader = Some(Leader {
                    distance_m: state.distance,
                    speed_mps: state.speed,
                    power_w: f64::from(u.power_w),
                });
            }
        }
        self.others_now = others;
    }

    fn on_own(&mut self, state: RiderState, acked_seq: u32, now_s: f64) {
        let r = &mut self.report;
        if r.start_m.is_none() {
            r.start_m = Some(state.distance);
            // Start the prediction exactly at the server's place, keeping
            // the lane it assigned (the bot sends no steering; autodrive).
            self.predictor.set_state(state);
            self.predictor.set_lateral_target(state.lateral_offset);
            r.own_first_s = Some(now_s);
        } else {
            self.predictor
                .reconcile(&self.route, state, self.snapshot_age_s, self.effort.power_w);
            let warm = r
                .own_first_s
                .is_some_and(|t| now_s - t >= PREDICTION_WARMUP_S);
            if warm {
                let e = self.predictor.last_error().abs();
                r.prediction_samples += 1;
                r.prediction_error_sum_m += e;
                r.prediction_error_max_m = r.prediction_error_max_m.max(e);
            }
        }
        r.end_m = state.distance;
        r.max_speed_mps = r.max_speed_mps.max(state.speed);
        r.acked_seq = r.acked_seq.max(acked_seq);
    }

    /// One 50 Hz tick at client time `now_s`: the pacing, the prediction,
    /// and the messages to send (a sensor sample four times a second, and
    /// at once when the brake changes).
    pub fn tick(&mut self, now_s: f64) -> Vec<Vec<u8>> {
        if !self.riding() {
            return Vec::new();
        }
        let me = self.predictor.state();
        // Like a trainer: the power changes with the samples (4 Hz), so
        // the prediction uses exactly the power the server has.
        let wanted = self.config.pacing.effort(
            now_s,
            me.distance,
            me.speed,
            self.leader.filter(|_| self.report.start_m.is_some()),
        );
        let due = self.next_sample_s.is_none_or(|t| now_s >= t - 1e-9);
        let send = due || wanted.brake != self.effort.brake;
        if send {
            self.effort = wanted;
        }
        let effort = self.effort;
        self.report.following |= self.leader.is_some();
        self.predictor.set_braking(effort.brake);
        let drag = ocw_sim::drafting::drag_factor(
            (me.distance, me.lateral_offset),
            self.others_now
                .iter()
                .map(|(_, o)| (o.distance, o.lateral_offset)),
        );
        self.predictor.set_drag_factor(drag);
        self.predictor.step(&self.route, effort.power_w);
        for (_, o) in &mut self.others_now {
            o.distance += o.speed * SIM_DT;
        }
        if !send {
            return Vec::new();
        }
        let mut next = self.next_sample_s.unwrap_or(now_s);
        while next <= now_s + 1e-9 {
            next += SAMPLE_INTERVAL_S;
        }
        self.next_sample_s = Some(next);
        self.seq += 1;
        self.report.samples_sent += 1;
        let p = effort.power_w;
        vec![encode(&client_message(C::Sample(v1::SensorSample {
            seq: self.seq,
            client_time_us: (now_s * 1e6) as u64,
            power_w: Some(p as f32),
            // Like the virtual trainer: a plausible cadence for the power.
            cadence_rpm: (p > 0.0).then(|| (70.0 + p / 20.0).min(105.0) as f32),
            heart_rate_bpm: None,
            brake: Some(effort.brake),
        })))]
    }

    /// The rider's report (with the other riders it saw).
    pub fn report(&self) -> RiderReport {
        let mut r = self.report.clone();
        r.others_seen = self
            .others
            .values()
            .map(|o| o.name.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        r.others_moving = self
            .others
            .values()
            .filter(|o| o.last_m - o.first_m >= MOVING_M)
            .map(|o| o.name.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        r.samples_acked = self.seq > 0 && r.acked_seq > 0;
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_server::{GameServer, ServerConfig};

    fn config(name: &str, follow: Option<(&str, f64)>) -> RiderConfig {
        RiderConfig {
            name: name.into(),
            weight_kg: 70.0,
            bike: ocw_core::ProfileId::Road,
            look: RiderLook::default(),
            pacing: Pacing {
                base_w: 220.0,
                phase: 0.0,
                follow: follow.map(|(n, g)| (n.into(), g)),
            },
        }
    }

    /// Joins `bot` on `server`, delivering the Welcome.
    fn join(server: &mut GameServer, bot: &mut BotRider) -> ocw_server::ConnectionId {
        let c = server.connect();
        for reply in server.handle(c, &bot.hello()) {
            bot.on_message(&reply, 0.0);
        }
        c
    }

    fn run(
        server: &mut GameServer,
        bots: &mut [(BotRider, ocw_server::ConnectionId)],
        from: f64,
        seconds: f64,
    ) {
        let mut t = from;
        while t < from + seconds {
            for (b, c) in bots.iter_mut() {
                for m in b.tick(t) {
                    server.handle(*c, &m);
                }
            }
            for m in server.tick() {
                for (b, _) in bots.iter_mut() {
                    b.on_message(&m, t);
                }
            }
            t += SIM_DT;
        }
    }

    #[test]
    fn bots_join_ride_and_see_each_other() {
        let mut server = GameServer::new(Route::test_loop(), ServerConfig::default());
        let mut bots: Vec<_> = ["linux-1", "linux-2"]
            .iter()
            .map(|n| {
                let mut b = BotRider::new(config(n, None), Route::test_loop());
                let c = join(&mut server, &mut b);
                (b, c)
            })
            .collect();
        assert!(bots.iter().all(|(b, _)| b.riding()));
        run(&mut server, &mut bots, 0.0, 20.0);
        for (b, _) in &bots {
            let r = b.report();
            assert!(r.joined_at_s.is_some() && r.rejected.is_none());
            assert_eq!(r.route_id, Route::test_loop().id());
            // 10 Hz for 20 s.
            assert!((195..=201).contains(&r.snapshots), "{}", r.snapshots);
            assert!(r.max_snapshot_gap_s < 0.15);
            assert_eq!(r.max_riders_seen, 2);
            assert_eq!(r.others_seen.len(), 1);
            assert_eq!(r.others_moving, r.others_seen);
            // ~220 W on the gentle loop for 20 s: well over 100 m.
            assert!(r.end_m - r.start_m.unwrap() > 100.0, "{r:?}");
            assert!(r.samples_sent >= 79 && r.samples_acked, "{r:?}");
            // In-process, no latency: the prediction matches the server.
            assert!(r.prediction_samples > 100);
            assert!(r.prediction_error_max_m < 0.5, "{r:?}");
        }
        let names: Vec<_> = bots
            .iter()
            .map(|(b, _)| b.report().others_seen[0].clone())
            .collect();
        assert_eq!(names, ["linux-2", "linux-1"]);
    }

    #[test]
    fn a_follower_keeps_near_its_leader() {
        let mut server = GameServer::new(Route::test_loop(), ServerConfig::default());
        let mut bots = Vec::new();
        let mut lead = BotRider::new(config("Kiosk", None), Route::test_loop());
        let c = join(&mut server, &mut lead);
        bots.push((lead, c));
        let mut f = BotRider::new(config("mac-1", Some(("Kiosk", 8.0))), Route::test_loop());
        let c = join(&mut server, &mut f);
        bots.push((f, c));
        run(&mut server, &mut bots, 0.0, 60.0);
        let gap = bots[1].0.state().distance - bots[0].0.state().distance;
        assert!((gap - 8.0).abs() < 4.0, "gap {gap}");
        assert!(bots[1].0.report().following);
        assert!(!bots[0].0.report().following);
    }

    #[test]
    fn a_course_mismatch_or_reject_closes_the_bot() {
        let mut server = GameServer::new(Route::test_hills(), ServerConfig::default());
        let mut b = BotRider::new(config("win-1", None), Route::test_loop());
        join(&mut server, &mut b);
        assert!(!b.riding() && b.closed());
        let r = b.report();
        assert!(
            r.rejected.as_deref().unwrap().contains("course mismatch"),
            "{r:?}"
        );
        assert!(b.tick(1.0).is_empty());

        let mut b = BotRider::new(config("win-2", None), Route::test_loop());
        let reject = encode(&ocw_protocol::server_message(S::Reject(v1::Reject {
            reason: "protocol version 0 not supported".into(),
        })));
        b.on_message(&reject, 0.5);
        assert!(b.report().closed.unwrap().contains("rejected"));
        b.on_message(b"\xff\xff", 0.6);
        assert_eq!(b.report().malformed, 1);
    }

    #[test]
    fn stale_snapshots_are_counted_and_skipped() {
        let mut server = GameServer::new(Route::test_loop(), ServerConfig::default());
        let mut b = BotRider::new(config("linux-1", None), Route::test_loop());
        join(&mut server, &mut b);
        let mut snaps = Vec::new();
        for _ in 0..10 {
            snaps.extend(server.tick());
        }
        assert_eq!(snaps.len(), 2);
        b.on_message(&snaps[1], 0.2);
        b.on_message(&snaps[0], 0.25);
        let r = b.report();
        assert_eq!((r.snapshots, r.stale_snapshots), (1, 1));
        // Closing keeps the first reason.
        b.close("server shutting down", 1.0);
        b.close("connection lost", 2.0);
        assert_eq!(b.report().closed.as_deref(), Some("server shutting down"));
    }
}
