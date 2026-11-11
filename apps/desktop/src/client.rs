//! Engine-independent client logic: device samples → server, server messages →
//! prediction (own rider) and interpolation (others) → render view.
//!
//! The Bevy systems in `connection.rs` only move bytes in and out of this type,
//! so the whole client loop is unit-testable without a window or GPU.
//!
//! In free ride (spec §2.2.1) the client also predicts its own path through
//! the road graph: it locks the chosen exit at the lock point like the
//! server does, sends the choice as a `TurnIntent`, and takes the server's
//! `PathUpdate`s as corrections; other riders' paths come only from the
//! server.

use std::collections::BTreeMap;
use std::sync::Arc;

use ocw_core::{INTERPOLATION_DELAY_S, RiderId, RiderProfile};
use ocw_devices::DeviceEvent;
use ocw_protocol::v1::{self, client_message::Msg as C, server_message::Msg as S};
use ocw_protocol::{PROTOCOL_VERSION, client_message, decode_server, dequantize, encode};
use ocw_render::{ExitView, JunctionView, RiderView};
use ocw_sim::freeride::{self, TurnChoice, UTURN_INTERVAL_S};
use ocw_sim::{Predictor, RiderState, SnapshotBuffer};
use ocw_world::path::lock_distance;
use ocw_world::{Junction, RidePath, RoadNetwork, Route, RoutePlan, RouteState, Track, TurnReason};

/// Minimum time between steering messages while the target changes.
pub const STEER_SEND_INTERVAL_S: f64 = 0.05;
/// The junction overlay appears within this distance, or within
/// [`JUNCTION_LOOKAHEAD_S`] of riding, whichever is larger (spec §2.2.1).
pub const JUNCTION_LOOKAHEAD_M: f64 = 150.0;
pub const JUNCTION_LOOKAHEAD_S: f64 = 10.0;
/// Dead-end warnings start this far before the end (spec §2.2.2).
pub const DEAD_END_WARNING_M: f64 = 1000.0;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NetStats {
    pub messages_in: u64,
    pub malformed: u64,
    pub snapshots: u64,
    pub samples_out: u64,
    pub last_snapshot_tick: u64,
    /// Free ride: path updates received, and own-path corrections among them.
    pub path_updates: u64,
}

/// Something the user should be told about (shown in the console).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    Joined { rider_id: RiderId, route_id: String },
    Rejected(String),
    CommandResult(String),
}

/// Free-ride state of the client.
#[derive(Debug, Clone)]
struct FreeRide {
    net: Arc<RoadNetwork>,
    /// Route ride: the route the own path follows (spec §2.7).
    plan: Option<Arc<RoutePlan>>,
    /// The own path, predicted (shared with the render view).
    own: Option<Arc<RidePath>>,
    /// The other riders' paths, from the server.
    others: BTreeMap<RiderId, Arc<RidePath>>,
    /// Exit chosen for the next junction (`None`: the default).
    choice: Option<TurnChoice>,
    turn_seq: u32,
    /// Last choice sent: for which junction, and what.
    turn_sent: Option<(ocw_world::graph::VertexId, Option<TurnChoice>)>,
    last_u_turn_s: Option<f64>,
    /// The junction locked last, until passed: (junction, its path
    /// distance, the exit taken).
    locked: Option<(Junction, f64, usize)>,
}

pub struct ClientCore {
    route: Route,
    free: Option<FreeRide>,
    profile: RiderProfile,
    name: String,
    rider_id: Option<RiderId>,
    predictor: Predictor,
    /// Predicted state before the last physics step: rendering interpolates
    /// between it and the current one, so motion is smooth at any frame rate.
    prev_state: RiderState,
    /// Steering target (lateral offset); set from the first own snapshot.
    lateral_target: Option<f64>,
    /// Lean steering (rad, positive = right), the arrow keys; replaces the
    /// target while set.
    lean: Option<f64>,
    /// Wild mode: not bound to the road.
    wild: bool,
    steer_seq: u32,
    /// Last steering intent sent ((lean mode, mrad or cm, wild)) and when.
    steer_sent: Option<((bool, i32, bool), f64)>,
    others: SnapshotBuffer,
    /// Latest known state of the other riders, advanced every physics tick
    /// (for predicting the local rider's drafting).
    others_now: Vec<(RiderId, RiderState)>,
    names: BTreeMap<RiderId, (String, f64)>,
    power_w: f64,
    cadence_rpm: Option<f64>,
    heart_rate_bpm: Option<f64>,
    /// Extra power from the keyboard, on top of the device power.
    key_power_w: f64,
    braking: bool,
    last_sample_us: u64,
    seq: u32,
    stats: NetStats,
    /// The steering mode saved while the menu holds autodrive.
    menu_hold: Option<HeldSteering>,
}

/// Steering mode before the menu opened (spec §3.2.1), restored on closing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HeldSteering {
    autodrive: bool,
    wild: bool,
}

impl ClientCore {
    pub fn new(route: Route, name: impl Into<String>, profile: RiderProfile) -> Self {
        Self {
            route,
            free: None,
            profile,
            name: name.into(),
            rider_id: None,
            predictor: Predictor::new(profile, RiderState::default()),
            prev_state: RiderState::default(),
            lateral_target: None,
            lean: None,
            wild: false,
            steer_seq: 0,
            steer_sent: None,
            others: SnapshotBuffer::default(),
            others_now: Vec::new(),
            names: BTreeMap::new(),
            power_w: 0.0,
            cadence_rpm: None,
            heart_rate_bpm: None,
            key_power_w: 0.0,
            braking: false,
            last_sample_us: 0,
            seq: 0,
            stats: NetStats::default(),
            menu_hold: None,
        }
    }

    /// A client for a free ride on `net` (the server sends the paths).
    pub fn free_ride(
        net: Arc<RoadNetwork>,
        name: impl Into<String>,
        profile: RiderProfile,
    ) -> Self {
        let v = net.graph().vertices.first().copied().unwrap_or([0.0, 0.0]);
        let placeholder = Route::from_points(
            "free-ride",
            "Free ride",
            vec![[v[0], 0.0, v[1]], [v[0] + 1.0, 0.0, v[1]]],
            false,
            5.0,
        )
        .expect("two distinct points");
        Self {
            free: Some(FreeRide {
                net,
                plan: None,
                own: None,
                others: BTreeMap::new(),
                choice: None,
                turn_seq: 0,
                turn_sent: None,
                last_u_turn_s: None,
                locked: None,
            }),
            ..Self::new(placeholder, name, profile)
        }
    }

    /// A route ride on the road graph (spec §2.7): the own path follows
    /// `plan` (the server's session rides the same route).
    pub fn with_route_plan(mut self, plan: Arc<RoutePlan>) -> Self {
        if let Some(fr) = &mut self.free {
            let v = self.route.points()[0];
            self.route = Route::from_points(
                plan.name.clone(),
                plan.name.clone(),
                vec![v, [v[0] + 1.0, v[1], v[2]]],
                false,
                5.0,
            )
            .expect("two distinct points");
            fr.plan = Some(plan);
        }
        self
    }

    /// Name of what is ridden (the route's, or "Free ride").
    pub fn course_name(&self) -> &str {
        self.route.name()
    }

    /// (lat, lon) of the local metres, if known (route files, packages).
    pub fn geo_origin(&self) -> Option<(f64, f64)> {
        self.route.geo_origin()
    }

    /// The same client with the geographic origin of its local metres
    /// (free ride: the world package's), for the GPS track of rides.
    pub fn with_geo_origin(mut self, origin: (f64, f64)) -> Self {
        self.route = self.route.clone().with_geo_origin(origin);
        self
    }

    pub fn is_free_ride(&self) -> bool {
        self.free.is_some()
    }

    /// The first message to send after connecting.
    pub fn hello(&self) -> Vec<u8> {
        encode(&client_message(C::Hello(v1::Hello {
            protocol_version: PROTOCOL_VERSION,
            display_name: self.name.clone(),
            profile: Some(self.profile.into()),
            world: self.free.as_ref().map(|f| ocw_protocol::world_id(&f.net)),
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
            power_w: Some(self.power_w() as f32),
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
                // Junctions are vertex indices: riding on another graph
                // than the server's would go wrong at every junction.
                if let (Some(fr), Some(theirs)) = (&self.free, &w.world)
                    && theirs.graph_hash != fr.net.fingerprint()
                {
                    return Some(Notice::Rejected(format!(
                        "world package mismatch: the server rides {} ({:016x}), this client has {} ({:016x})",
                        theirs.name,
                        theirs.graph_hash,
                        fr.net.package,
                        fr.net.fingerprint()
                    )));
                }
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
            S::Path(p) => {
                self.on_path(&p);
                None
            }
        }
    }

    /// A rider's path from the server: a new one, an extension, or a
    /// correction of the own prediction (spec §6.5).
    fn on_path(&mut self, u: &v1::PathUpdate) {
        let own_id = self.rider_id;
        let Some(fr) = &mut self.free else { return };
        let Ok(segments) = ocw_protocol::path_segments(u, &fr.net) else {
            self.stats.malformed += 1;
            return;
        };
        self.stats.path_updates += 1;
        let rider = RiderId(u.rider_id);
        let current = if Some(rider) == own_id {
            fr.own.take()
        } else {
            fr.others.remove(&rider)
        };
        let net = fr.net.clone();
        // The own path follows the route (others' paths only come from the
        // server and need no route).
        let plan = if Some(rider) == own_id {
            fr.plan.clone()
        } else {
            None
        };
        let fresh = || {
            RidePath::from_segments(net.clone(), &segments, u.start_m, u.first_index)
                .ok()
                .map(|p| Arc::new(p.with_plan(plan.clone())))
        };
        let updated = match current {
            Some(mut p) if !u.reset && u.first_index > p.first_index() => {
                match Arc::make_mut(&mut p).replace_from(u.first_index, &segments) {
                    Ok(()) => Some(p),
                    Err(_) => fresh(),
                }
            }
            _ => fresh(),
        };
        if let Some(p) = updated {
            if Some(rider) == own_id {
                fr.own = Some(p);
            } else {
                fr.others.insert(rider, p);
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
                let power = self.power_w();
                let track: &dyn Track = match self.free.as_ref().and_then(|f| f.own.as_deref()) {
                    Some(p) => p,
                    None => &self.route,
                };
                self.predictor.reconcile(track, state, 0.0, power);
                // A snap (teleport, large error) must not be interpolated.
                if self.predictor.last_error().abs() > ocw_sim::prediction::SNAP_THRESHOLD_M {
                    self.prev_state = self.predictor.state();
                }
            } else {
                others.push((id, state));
            }
        }
        if let Some(fr) = &mut self.free {
            // Forget riders who left; drop what lies far behind the others.
            fr.others
                .retain(|id, _| others.iter().any(|(o, _)| o == id));
            for (id, st) in &others {
                if let Some(p) = fr.others.get_mut(id)
                    && p.needs_trim(st.distance - 50.0)
                {
                    Arc::make_mut(p).trim_behind(st.distance - 50.0);
                }
            }
        }
        self.others_now = others.clone();
        self.others.push(now_s, others);
    }

    /// What the local rider rides along: the own free-ride path (once
    /// known), or the route.
    pub fn track(&self) -> &dyn Track {
        match self.free.as_ref().and_then(|f| f.own.as_deref()) {
            Some(p) => p,
            None => &self.route,
        }
    }

    /// The lateral limit at the rider (road edge, or wild mode's).
    fn lateral_limit(&self) -> f64 {
        if self.wild {
            ocw_sim::physics::WILD_LIMIT_M
        } else {
            let d = self.predictor.state().distance;
            ocw_sim::physics::lateral_limit(self.track().road_width_at(d))
        }
    }

    /// Sets the steering target (lateral offset in metres), within the road.
    pub fn steer_to(&mut self, target_m: f64) {
        if self.rider_id.is_none() {
            return;
        }
        let limit = self.lateral_limit();
        let t = target_m.clamp(-limit, limit);
        self.lateral_target = Some(t);
        self.lean = None;
        self.predictor.set_lateral_target(t);
    }

    /// Lean steering (the arrow keys): the rider leans by `lean_rad`
    /// (positive = right) and rides a curve until the lean is 0 again.
    pub fn set_lean(&mut self, lean_rad: f64) {
        if self.rider_id.is_none() {
            return;
        }
        let max = ocw_sim::physics::MAX_STEER_LEAN;
        let l = lean_rad.clamp(-max, max);
        self.lean = Some(l);
        self.predictor.set_lean(l);
    }

    /// The current lean steering, if the rider steers by leaning.
    pub fn lean(&self) -> Option<f64> {
        self.lean
    }

    /// Autodrive: the rider keeps to its line (the middle of the road
    /// unless moved) by itself; otherwise the rider steers by leaning.
    pub fn autodrive(&self) -> bool {
        self.lean.is_none()
    }

    /// Switches autodrive on (to the middle of the road) or off (upright,
    /// steering by leaning from now on).
    pub fn set_autodrive(&mut self, on: bool) {
        if on {
            self.steer_to(0.0);
        } else {
            self.set_lean(0.0);
        }
    }

    /// Moves the autodrive line sideways by `delta_m` (positive = right).
    pub fn steer_by(&mut self, delta_m: f64) {
        let t = self.lateral_target.unwrap_or(0.0);
        self.steer_to(t + delta_m);
    }

    pub fn wild(&self) -> bool {
        self.wild
    }

    /// Steering mode and surface for the HUD, e.g. "manual | WILD | grass".
    pub fn steering_status(&self) -> String {
        let mut s = String::from(if self.autodrive() {
            "autodrive"
        } else {
            "manual"
        });
        if self.wild {
            s.push_str(" | WILD");
        }
        if self.menu_hold.is_some() {
            s.push_str(" (menu)");
        }
        let surface = ocw_sim::ride::surface(self.track(), &self.predictor.state());
        if surface != ocw_world::Surface::Asphalt {
            s.push_str(" | ");
            s.push_str(surface.name());
        }
        s
    }

    /// Wild mode: not bound to the road, ride anywhere (spec §2.2).
    /// Switching it on also switches autodrive off (upright, lean
    /// steering), so the rider can leave the road; switching it off keeps
    /// the steering mode (spec §5.2.1).
    pub fn set_wild(&mut self, wild: bool) {
        self.wild = wild;
        self.predictor.set_wild(wild);
        if wild {
            self.set_autodrive(false);
        } else {
            // Back within the road for the autodrive line.
            let limit = self.lateral_limit();
            if let Some(t) = self.lateral_target {
                self.lateral_target = Some(t.clamp(-limit, limit));
            }
        }
    }

    /// The menu opened (spec §3.2.1): the ride goes on, so the rider stays
    /// on the road by itself: autodrive on (wild mode off; from manual
    /// steering at the rider's current line, so it does not swerve) and the
    /// next junction back to its default exit. [`Self::release_steering`]
    /// restores the previous mode. Holding twice keeps the first mode.
    pub fn hold_steering(&mut self) {
        if self.menu_hold.is_some() {
            return;
        }
        self.menu_hold = Some(HeldSteering {
            autodrive: self.autodrive(),
            wild: self.wild,
        });
        if self.wild {
            self.set_wild(false);
        }
        if !self.autodrive() {
            let line = self.predictor.state().lateral_offset;
            self.steer_to(line);
        }
        self.clear_exit();
    }

    /// The menu closed: back to the steering mode before
    /// [`Self::hold_steering`]: manual steering upright (no lean carried
    /// over), wild mode again; autodrive keeps the line it rode.
    pub fn release_steering(&mut self) {
        let Some(h) = self.menu_hold.take() else {
            return;
        };
        if h.wild {
            // Directly: `set_wild` would also switch autodrive off.
            self.wild = true;
            self.predictor.set_wild(true);
        }
        if !h.autodrive {
            self.set_lean(0.0);
        }
    }

    /// Whether the menu holds the steering in autodrive.
    #[cfg(test)]
    pub fn steering_held(&self) -> bool {
        self.menu_hold.is_some()
    }

    pub fn lateral_target(&self) -> Option<f64> {
        self.lateral_target
    }

    /// The steering intent to send now, if the target changed (at most
    /// every `STEER_SEND_INTERVAL_S` while it keeps changing).
    pub fn steer_message(&mut self, now_s: f64) -> Option<Vec<u8>> {
        let key = match self.lean {
            Some(l) => (true, (l * 1000.0).round() as i32, self.wild),
            None => (
                false,
                (self.lateral_target? * 100.0).round() as i32,
                self.wild,
            ),
        };
        if let Some((sent, at)) = self.steer_sent
            && (sent == key || now_s - at < STEER_SEND_INTERVAL_S)
        {
            return None;
        }
        self.steer_seq += 1;
        self.steer_sent = Some((key, now_s));
        let (lean_steering, value, wild) = key;
        Some(encode(&client_message(C::Steer(v1::SteerIntent {
            seq: self.steer_seq,
            lateral_target_cm: if lean_steering { 0 } else { value },
            lean_steering,
            lean_mrad: if lean_steering { value } else { 0 },
            wild,
        }))))
    }

    /// The junction the own path ends at (the next decision) and its path
    /// distance.
    fn next_junction(&self) -> Option<(Junction, f64)> {
        self.free.as_ref()?.own.as_ref()?.next_junction()
    }

    /// Whether a junction is ahead within the overlay's look-ahead and not
    /// locked yet (the arrow keys then choose the exit, spec §2.2.1).
    pub fn junction_active(&self) -> bool {
        let st = self.predictor.state();
        self.next_junction().is_some_and(|(_, at)| {
            let left = at - st.distance;
            left >= lock_distance(st.speed)
                && left <= JUNCTION_LOOKAHEAD_M.max(JUNCTION_LOOKAHEAD_S * st.speed)
        })
    }

    /// Moves the exit selection of the next junction `steps` exits to the
    /// right (negative: left), spec §2.2.1. Back at the default it is "no
    /// choice". Returns the chosen exit's description.
    pub fn turn(&mut self, steps: i32) -> Option<String> {
        let (j, _) = self.next_junction()?;
        let fr = self.free.as_mut()?;
        let current = freeride::exit_index(&j, fr.choice.as_ref()) as i32;
        let i = (current + steps).clamp(0, j.exits.len() as i32 - 1) as usize;
        self.select_exit(&j, i)
    }

    /// Selects exit `i` (leftmost = 0) of the next junction.
    pub fn choose_exit(&mut self, i: usize) -> Option<String> {
        let (j, _) = self.next_junction()?;
        (i < j.exits.len()).then_some(())?;
        self.select_exit(&j, i)
    }

    /// Back to the default exit ("no choice").
    pub fn clear_exit(&mut self) -> Option<String> {
        let (j, _) = self.next_junction()?;
        self.select_exit(&j, j.default)
    }

    /// Selects the leftmost / straightest / rightmost-like exit by name:
    /// "left", "straight" or "right" (the nearest of that class).
    pub fn choose_direction(&mut self, dir: &str) -> Option<String> {
        let (j, _) = self.next_junction()?;
        let i = match dir {
            "left" => j
                .exits
                .iter()
                .rposition(|e| e.angle_deg < -ocw_world::network::STRAIGHT_DEG)?,
            "right" => j
                .exits
                .iter()
                .position(|e| e.angle_deg > ocw_world::network::STRAIGHT_DEG)?,
            "straight" => (0..j.exits.len()).min_by(|&a, &b| {
                j.exits[a]
                    .angle_deg
                    .abs()
                    .total_cmp(&j.exits[b].angle_deg.abs())
            })?,
            _ => return None,
        };
        self.select_exit(&j, i)
    }

    fn select_exit(&mut self, j: &Junction, i: usize) -> Option<String> {
        let fr = self.free.as_mut()?;
        let e = &j.exits[i];
        fr.choice = (i != j.default).then_some(TurnChoice {
            junction: j.vertex,
            edge: e.edge,
            forward: e.forward,
        });
        let note = match j.route {
            Some(r) if r == i => ", the route",
            Some(_) => ", leaves the route (ride.rejoin returns to it)",
            None if i == j.default => ", the default",
            None => "",
        };
        Some(format!(
            "next junction: {} ({}){note}",
            e.class.name(),
            exit_name(&fr.net, e.edge),
        ))
    }

    /// The exit choice to send, when it changed (spec §6.5: reliable,
    /// whenever the choice changes).
    pub fn turn_message(&mut self) -> Option<Vec<u8>> {
        let (j, _) = self.next_junction()?;
        let fr = self.free.as_mut()?;
        let choice = fr.choice.filter(|c| c.junction == j.vertex);
        let key = (j.vertex, choice);
        if fr.turn_sent == Some(key) {
            return None;
        }
        // "No choice" only needs sending to take back a choice.
        let server_has_choice = matches!(fr.turn_sent, Some((_, Some(_))));
        fr.turn_sent = Some(key);
        if choice.is_none() && !server_has_choice {
            return None;
        }
        fr.turn_seq += 1;
        Some(encode(&client_message(C::Turn(ocw_protocol::turn_intent(
            fr.turn_seq,
            j.vertex,
            choice.as_ref(),
            &fr.net,
        )))))
    }

    /// Manual U-turn (spec §2.2.2): predicted at once; returns the message
    /// for the server. At most every [`UTURN_INTERVAL_S`].
    pub fn u_turn(&mut self, now_s: f64) -> Option<Vec<u8>> {
        let st = self.predictor.state();
        let fr = self.free.as_mut()?;
        if fr
            .last_u_turn_s
            .is_some_and(|t| now_s - t < UTURN_INTERVAL_S)
        {
            return None;
        }
        // Quantized like the message, so the server builds the same loop.
        let (d_cm, v_mm) = (
            (st.distance * 100.0).round().max(0.0) as u32,
            (st.speed * 1000.0).round().max(0.0) as u32,
        );
        let own = fr.own.as_mut()?;
        Arc::make_mut(own).u_turn(f64::from(d_cm) / 100.0, f64::from(v_mm) / 1000.0)?;
        fr.last_u_turn_s = Some(now_s);
        fr.choice = None;
        Some(encode(&client_message(C::UTurn(v1::UTurn {
            distance_cm: d_cm,
            speed_mm_s: v_mm,
        }))))
    }

    /// The junction overlay (spec §2.2.1): the next junction within the
    /// look-ahead, or the one just locked until it is passed.
    pub fn junction_view(&self) -> Option<JunctionView> {
        let fr = self.free.as_ref()?;
        let st = self.predictor.state();
        let d = st.distance;
        if let Some((j, at, chosen)) = &fr.locked
            && d < at + 8.0
        {
            return Some(junction_view(&fr.net, j, *chosen, at - d, true));
        }
        let (j, at) = fr.own.as_ref()?.next_junction()?;
        if at - d > JUNCTION_LOOKAHEAD_M.max(JUNCTION_LOOKAHEAD_S * st.speed) {
            return None;
        }
        let i = freeride::exit_index(&j, fr.choice.as_ref());
        Some(junction_view(&fr.net, &j, i, at - d, false))
    }

    /// Route ride: back to the route after leaving it (console
    /// `ride.rejoin`, key J). Predicted at once; returns the message for the
    /// server, or why it is not possible.
    pub fn rejoin_route(&mut self) -> Result<Vec<u8>, String> {
        let st = self.predictor.state();
        let fr = self.free.as_mut().ok_or("not riding a route")?;
        let own = fr.own.as_mut().ok_or("not riding yet")?;
        // Quantized like the message, so the server finds the same way.
        let (d_cm, v_mm) = (
            (st.distance * 100.0).round().max(0.0) as u32,
            (st.speed * 1000.0).round().max(0.0) as u32,
        );
        Arc::make_mut(own).rejoin_route(f64::from(d_cm) / 100.0, f64::from(v_mm) / 1000.0)?;
        fr.choice = None;
        Ok(encode(&client_message(C::RouteRejoin(v1::RouteRejoin {
            distance_cm: d_cm,
            speed_mm_s: v_mm,
        }))))
    }

    /// Route ride: whether the rider is on the route right now.
    pub fn on_route(&self) -> Option<bool> {
        let own = self.free.as_ref()?.own.as_ref()?;
        own.plan()?;
        Some(own.on_route_at(self.predictor.state().distance))
    }

    /// Route ride: the route's name and progress for the HUD, e.g.
    /// "Rheintal 3.2 / 12.4 km", "Rheintal lap 2 0.4 / 3.1 km", or
    /// "Rheintal (off the route)".
    pub fn route_status(&self) -> Option<String> {
        let fr = self.free.as_ref()?;
        let plan = fr.plan.as_ref()?;
        let Some(own) = fr.own.as_ref() else {
            return Some(plan.name.clone());
        };
        let p = own.route_progress(self.predictor.state().distance)?;
        Some(if p.finished {
            format!("{} (finished)", plan.name)
        } else if !p.on_route {
            format!("{} (off the route)", plan.name)
        } else if plan.is_loop {
            format!(
                "{} lap {} {:.1} / {:.1} km",
                plan.name,
                p.lap + 1,
                p.done_m / 1000.0,
                p.length_m / 1000.0
            )
        } else {
            format!(
                "{} {:.1} / {:.1} km",
                plan.name,
                p.done_m / 1000.0,
                p.length_m / 1000.0
            )
        })
    }

    /// A message for the rider: "Dead end in 800 m", "End of region",
    /// "U-turn" (spec §2.2.2); on a route ride also that the rider left it.
    pub fn notice(&self) -> Option<String> {
        let own = self.free.as_ref()?.own.as_ref()?;
        let d = self.predictor.state().distance;
        if own.in_turnaround(d) {
            return Some("U-turn".into());
        }
        if own.plan().is_some() && !own.on_route_at(d) {
            let progress = own.route_progress(d);
            let back = matches!(own.route_state(), Some(RouteState::On { .. }));
            if !progress.is_some_and(|p| p.finished) {
                return Some(if back {
                    "Off the route - heading back to it".into()
                } else {
                    "Off the route - press J (ride.rejoin) to return".into()
                });
            }
        }
        let round = |m: f64| (m / 10.0).round() * 10.0;
        match own.turnaround_ahead(d) {
            Some((at, TurnReason::Boundary)) if at - d < DEAD_END_WARNING_M => Some(format!(
                "End of region in {:.0} m - install the neighbouring package to ride on",
                round(at - d)
            )),
            Some((at, TurnReason::Manual)) => Some(format!("U-turn in {:.0} m", round(at - d))),
            _ => own
                .dead_end_ahead(d)
                .filter(|m| *m < DEAD_END_WARNING_M)
                .map(|m| format!("Dead end in {:.0} m", round(m.max(0.0)))),
        }
    }

    /// Every rider's free-ride path (for rendering).
    pub fn paths(&self) -> BTreeMap<RiderId, Arc<RidePath>> {
        let Some(fr) = &self.free else {
            return BTreeMap::new();
        };
        let mut out = fr.others.clone();
        if let (Some(id), Some(own)) = (self.rider_id, &fr.own) {
            out.insert(id, own.clone());
        }
        out
    }

    /// One fixed physics tick of the local prediction.
    pub fn predict_step(&mut self) {
        if self.rider_id.is_none() {
            return;
        }
        let me = self.predictor.state();
        // Free ride: extend the own path like the server (lock point).
        if let Some(fr) = &mut self.free
            && let Some(own) = &mut fr.own
        {
            let d = me.distance;
            if own.needs_extend(d, me.speed) {
                let pending = own.next_junction();
                let (changed, used) =
                    freeride::advance(Arc::make_mut(own), d, me.speed, fr.choice.as_ref());
                if let (Some((j, at)), Some(_)) = (pending, changed)
                    && d >= at - lock_distance(me.speed)
                {
                    let chosen = freeride::exit_index(&j, fr.choice.as_ref());
                    fr.locked = Some((j, at, chosen));
                }
                if used {
                    fr.choice = None;
                }
            }
            if own.needs_trim(d) {
                Arc::make_mut(own).trim_behind(d);
            }
        }
        let others: Vec<(f64, f64)> = match self
            .free
            .as_ref()
            .and_then(|f| f.own.as_ref().map(|o| (f, o)))
        {
            Some((fr, own)) => freeride::others_on_path(
                own,
                me.distance,
                self.others_now.iter().filter_map(|(id, s)| {
                    fr.others
                        .get(id)
                        .map(|p| (p.as_ref(), s.distance, s.lateral_offset))
                }),
            ),
            None => self
                .others_now
                .iter()
                .map(|(_, o)| (o.distance, o.lateral_offset))
                .collect(),
        };
        let drag = ocw_sim::drafting::drag_factor((me.distance, me.lateral_offset), others);
        self.predictor.set_drag_factor(drag);
        self.prev_state = self.predictor.state();
        let power = self.power_w();
        let track: &dyn Track = match self.free.as_ref().and_then(|f| f.own.as_deref()) {
            Some(p) => p,
            None => &self.route,
        };
        self.predictor.step(track, power);
        for (_, o) in &mut self.others_now {
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
            power_w: self.power_w(),
            cadence_rpm: self.cadence_rpm,
            heart_rate_bpm: self.heart_rate_bpm,
            is_local: true,
        }];
        for (id, state) in self.others.sample(now_s - INTERPOLATION_DELAY_S) {
            // Free ride: only riders whose path is known can be placed.
            if let Some(fr) = &self.free
                && !fr.others.contains_key(&id)
            {
                continue;
            }
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

    #[cfg(test)]
    pub fn route(&self) -> &Route {
        &self.route
    }
    pub fn rider_id(&self) -> Option<RiderId> {
        self.rider_id
    }
    pub fn state(&self) -> RiderState {
        self.predictor.state()
    }
    /// The rider's power: the device's plus the keyboard's (↑).
    pub fn power_w(&self) -> f64 {
        self.power_w + self.key_power_w
    }

    /// Extra power from the keyboard (↑ / ↓), added to the device's.
    pub fn key_power_w(&self) -> f64 {
        self.key_power_w
    }

    /// Sets the keyboard power; returns a sample to send when it changed
    /// (the next device sample may be 250 ms away).
    pub fn set_key_power(&mut self, watts: f64) -> Option<Vec<u8>> {
        let watts = watts.max(0.0);
        if (watts - self.key_power_w).abs() < 1e-9 {
            return None;
        }
        self.key_power_w = watts;
        Some(self.sample())
    }
    pub fn cadence_rpm(&self) -> Option<f64> {
        self.cadence_rpm
    }
    pub fn heart_rate_bpm(&self) -> Option<f64> {
        self.heart_rate_bpm
    }
    pub fn prediction_error(&self) -> f64 {
        self.predictor.last_error()
    }
    pub fn stats(&self) -> &NetStats {
        &self.stats
    }
}

/// Street name or ref of an edge, or its road class.
fn exit_name(net: &RoadNetwork, edge: usize) -> String {
    let e = net.edge(edge);
    e.name
        .clone()
        .unwrap_or_else(|| format!("{:?}", e.class).to_lowercase())
}

fn junction_view(
    net: &RoadNetwork,
    j: &Junction,
    selected: usize,
    distance_m: f64,
    locked: bool,
) -> JunctionView {
    JunctionView {
        distance_m,
        exits: j
            .exits
            .iter()
            .map(|e| ExitView {
                angle_deg: e.angle_deg,
                name: exit_name(net, e.edge),
                dead_end: e.dead_end,
                unpaved: e.unpaved,
                steep: e.steep,
            })
            .collect(),
        selected,
        default: j.default,
        locked,
        route: j.route,
    }
}

/// Linear interpolation of a rider state (`t` clamped to 0–1).
fn lerp_state(a: RiderState, b: RiderState, t: f64) -> RiderState {
    let t = t.clamp(0.0, 1.0);
    RiderState {
        distance: a.distance + (b.distance - a.distance) * t,
        speed: a.speed + (b.speed - a.speed) * t,
        lateral_offset: a.lateral_offset + (b.lateral_offset - a.lateral_offset) * t,
        heading: a.heading + (b.heading - a.heading) * t,
        turn_curvature: a.turn_curvature + (b.turn_curvature - a.turn_curvature) * t,
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
            let server = GameServer::new(
                route(),
                ServerConfig {
                    allow_dev_commands: allow_dev,
                    bots,
                    start_m: 0.0,
                },
            );
            let client = ClientCore::new(route(), "Tester", RiderProfile::default());
            Self::with(server, client)
        }

        /// Free ride on the grid (see [`grid`]), starting 50 m east of
        /// vertex 1 on the top row, riding east.
        fn free(bots: u32) -> Self {
            let net = Arc::new(RoadNetwork::new(grid()));
            let edge = (0..net.graph().edges.len())
                .find(|&e| net.edge(e).from.0 == 1 && net.edge(e).to.0 == 2)
                .unwrap();
            let server = GameServer::new(
                ocw_server::Course::FreeRide {
                    net: net.clone(),
                    start: ocw_sim::FreeRideStart {
                        edge,
                        offset_m: 50.0,
                        forward: true,
                    },
                },
                ServerConfig {
                    allow_dev_commands: true,
                    bots,
                    start_m: 0.0,
                },
            );
            Self::with(
                server,
                ClientCore::free_ride(net, "Free", RiderProfile::default()),
            )
        }

        fn with(mut server: GameServer, client: ClientCore) -> Self {
            let conn = server.connect();
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
                for msg in self.server.tick() {
                    self.receive(&msg);
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

        let server = ocw_server::local::LocalServer::spawn(route(), 2, 0.0).unwrap();
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
        h.client.steer_to(start + 1.5);
        let msg = h.client.steer_message(h.time_s + 0.1).expect("changed");
        match ocw_protocol::decode_client(&msg).unwrap() {
            C::Steer(s) => assert_eq!(s.lateral_target_cm, ((start + 1.5) * 100.0).round() as i32),
            other => panic!("unexpected {other:?}"),
        }
        // Rate limited: a further change right away waits.
        h.client.steer_to(start + 1.6);
        assert!(h.client.steer_message(h.time_s + 0.11).is_none());
        let msg = h
            .client
            .steer_message(h.time_s + 0.16)
            .expect("after the interval");
        h.send(&msg);
        h.ride(200.0, 8.0);
        let own = h.client.rider_id().unwrap();
        let server = h.server.session().rider_state(own).unwrap().lateral_offset;
        // Autodrive steers there smoothly (it leans over, it does not slide).
        assert!((server - (start + 1.6)).abs() < 0.1, "server {server}");
        assert!((h.client.state().lateral_offset - server).abs() < 0.05);
        // Never off the road.
        h.client.steer_to(100.0);
        assert_eq!(
            h.client.lateral_target(),
            Some(ocw_sim::physics::lateral_limit(7.0))
        );
    }

    /// Lean steering: the rider turns into a curve and keeps it until
    /// straightened, the same on the client and the server.
    #[test]
    fn lean_steering_rides_a_curve_until_straightened() {
        let mut h = Harness::new(0, false);
        h.ride(250.0, 4.0);
        let start = h.client.state().lateral_offset;
        let own = h.client.rider_id().unwrap();
        h.client.set_lean(-0.15);
        let msg = h.client.steer_message(h.time_s).expect("lean intent");
        match ocw_protocol::decode_client(&msg).unwrap() {
            C::Steer(s) => assert!(s.lean_steering && s.lean_mrad == -150, "{s:?}"),
            other => panic!("unexpected {other:?}"),
        }
        h.send(&msg);
        h.ride(250.0, 0.6);
        // Turning left: heading and offset to the left, on both sides.
        let server = h.server.session().rider_state(own).unwrap();
        let client = h.client.state();
        assert!(
            server.heading < -0.05 && server.turn_curvature < 0.0,
            "{server:?}"
        );
        assert!(server.lateral_offset < start - 0.2, "{server:?}");
        assert!(
            (client.heading - server.heading).abs() < 0.05,
            "{client:?} vs {server:?}"
        );
        assert!((client.lateral_offset - server.lateral_offset).abs() < 0.3);
        // Upright on the curving loop: nobody steers the bends, so the
        // rider ends up at an edge; client and server agree.
        h.client.set_lean(0.0);
        let msg = h.client.steer_message(h.time_s + 1.0).expect("straighten");
        h.send(&msg);
        h.ride(250.0, 20.0);
        let server = h.server.session().rider_state(own).unwrap();
        let limit = ocw_sim::physics::lateral_limit(7.0);
        assert!(
            (server.lateral_offset.abs() - limit).abs() < 1e-6,
            "{server:?}"
        );
        assert!((h.client.state().lateral_offset - server.lateral_offset).abs() < 0.3);
        // Autodrive brings it back to the middle.
        h.client.set_autodrive(true);
        assert_eq!(h.client.lean(), None);
        let msg = h.client.steer_message(h.time_s + 2.0).expect("autodrive");
        h.send(&msg);
        h.ride(250.0, 10.0);
        let server = h.server.session().rider_state(own).unwrap();
        assert!(server.lateral_offset.abs() < 0.3, "{server:?}");
    }

    /// Regression: leaning hard into the road edge, then holding the other
    /// arrow key stopped the lean at upright, and the rider scraped along
    /// the edge until it nearly stopped. Now the lean goes on to the other
    /// side and the rider leaves the edge, on the client and the server.
    #[test]
    fn steered_into_the_edge_the_rider_steers_back_out() {
        let mut h = Harness::new(0, false);
        h.ride(250.0, 8.0);
        let own = h.client.rider_id().unwrap();
        let limit = ocw_sim::physics::lateral_limit(7.0);
        let mut lean = -ocw_sim::physics::MAX_STEER_LEAN;
        h.client.set_lean(lean);
        let msg = h.client.steer_message(h.time_s).expect("lean intent");
        h.send(&msg);
        h.ride(250.0, 3.0);
        let server = h.server.session().rider_state(own).unwrap();
        assert!((server.lateral_offset + limit).abs() < 1e-6, "{server:?}");
        let pushed = server.speed;
        // Hold → (frame = tick), as the input system does.
        let mut pause = 0.0;
        for _ in 0..(4 * ocw_core::SIM_TICK_HZ) {
            lean = crate::input::lean_ramp(lean, 1.0, &mut pause, ocw_core::SIM_DT);
            h.client.set_lean(lean);
            if let Some(msg) = h.client.steer_message(h.time_s) {
                h.send(&msg);
            }
            h.ride(250.0, ocw_core::SIM_DT);
        }
        assert!(lean > 0.2, "{lean}");
        let server = h.server.session().rider_state(own).unwrap();
        let client = h.client.state();
        assert!(server.lateral_offset > -limit + 1.0, "{server:?}");
        // Still rolling (scraping on held it near 2 m/s).
        assert!(server.speed > 0.8 * pushed, "{pushed} {server:?}");
        assert!(
            (client.lateral_offset - server.lateral_offset).abs() < 0.3,
            "{client:?} vs {server:?}"
        );
        assert!((client.speed - server.speed).abs() < 0.2);
    }

    /// Wild mode on switches autodrive off (one intent carries both), so
    /// the rider leans off the road; wild off keeps the steering mode.
    #[test]
    fn wild_mode_switches_autodrive_off_and_leaves_the_road() {
        let mut h = Harness::new(0, false);
        h.ride(250.0, 4.0);
        let own = h.client.rider_id().unwrap();
        let first = h.client.steer_message(h.time_s).expect("initial intent");
        h.send(&first);
        assert!(h.client.autodrive());
        h.client.set_wild(true);
        assert!(!h.client.autodrive());
        assert_eq!(h.client.lean(), Some(0.0));
        assert!(h.client.steering_status().starts_with("manual | WILD"));
        let msg = h.client.steer_message(h.time_s + 1.0).expect("wild intent");
        match ocw_protocol::decode_client(&msg).unwrap() {
            C::Steer(s) => assert!(s.wild && s.lean_steering && s.lean_mrad == 0, "{s:?}"),
            other => panic!("unexpected {other:?}"),
        }
        h.send(&msg);
        assert!(h.client.steer_message(h.time_s + 2.0).is_none());
        h.client.set_lean(-0.15);
        let msg = h.client.steer_message(h.time_s + 2.0).expect("lean intent");
        h.send(&msg);
        h.ride(250.0, 3.0);
        let limit = ocw_sim::physics::lateral_limit(7.0);
        let server = h.server.session().rider_state(own).unwrap();
        assert!(server.lateral_offset < -limit - 1.0, "{server:?}");
        assert!((h.client.state().lateral_offset - server.lateral_offset).abs() < 0.3);
        // Autodrive switched on in wild mode stays on when wild goes off.
        h.client.set_autodrive(true);
        h.client.set_wild(false);
        assert!(h.client.autodrive());
        assert!(!h.client.steering_status().contains("WILD"));
        // Manual steering also stays manual.
        h.client.set_wild(true);
        h.client.set_autodrive(true);
        h.client.set_lean(0.1);
        h.client.set_wild(false);
        assert_eq!(h.client.lean(), Some(0.1));
    }

    /// The menu holds autodrive (spec §3.2.1): manual steering with a lean
    /// would leave the road while the rider looks at the menu.
    #[test]
    fn menu_holds_autodrive_and_restores_manual_steering() {
        let mut h = Harness::new(0, false);
        h.ride(250.0, 2.0);
        let own = h.client.rider_id().unwrap();
        h.client.set_lean(0.2);
        let msg = h.client.steer_message(h.time_s).expect("lean intent");
        h.send(&msg);
        h.ride(250.0, 0.5);
        h.client.hold_steering();
        assert!(h.client.autodrive() && h.client.steering_held());
        assert!(h.client.steering_status().starts_with("autodrive (menu)"));
        let msg = h
            .client
            .steer_message(h.time_s + 1.0)
            .expect("autodrive intent");
        match ocw_protocol::decode_client(&msg).unwrap() {
            C::Steer(s) => assert!(!s.lean_steering && !s.wild, "{s:?}"),
            other => panic!("unexpected {other:?}"),
        }
        h.send(&msg);
        // Held twice (menu reopened from a sub page): the first mode stays.
        h.client.hold_steering();
        // The ride goes on: power samples still reach the server, the rider
        // rides on and stays on the road.
        let d0 = h.server.session().rider_state(own).unwrap().distance;
        assert!(
            h.client
                .on_device_event(power(250.0, h.time_s + 0.1))
                .is_some()
        );
        h.ride(250.0, 10.0);
        let st = h.server.session().rider_state(own).unwrap();
        assert!(st.distance - d0 > 50.0, "{st:?}");
        let limit = ocw_sim::physics::lateral_limit(7.0);
        assert!(st.lateral_offset.abs() <= limit + 0.05, "{st:?}");
        assert!((h.client.state().lateral_offset - st.lateral_offset).abs() < 0.1);
        // Closed: manual again, upright.
        h.client.release_steering();
        assert!(!h.client.steering_held());
        assert_eq!(h.client.lean(), Some(0.0));
        assert!(h.client.steering_status().starts_with("manual"));
        // Released twice: nothing more happens.
        h.client.set_lean(0.1);
        h.client.release_steering();
        assert_eq!(h.client.lean(), Some(0.1));
    }

    #[test]
    fn menu_keeps_the_autodrive_line_and_restores_wild_mode() {
        let mut h = Harness::new(0, false);
        h.ride(250.0, 2.0);
        // Autodrive on its own line: unchanged by the menu.
        h.client.steer_to(1.0);
        h.client.hold_steering();
        assert_eq!(h.client.lateral_target(), Some(1.0));
        h.client.release_steering();
        assert!(h.client.autodrive());
        assert_eq!(h.client.lateral_target(), Some(1.0));
        // Wild mode: off while the menu is open (back within the road).
        h.client.set_wild(true);
        h.client.hold_steering();
        assert!(!h.client.wild() && h.client.autodrive());
        let limit = ocw_sim::physics::lateral_limit(7.0);
        assert!(h.client.lateral_target().unwrap().abs() <= limit);
        h.client.release_steering();
        assert!(h.client.wild() && !h.client.autodrive());
        // Wild with autodrive: both come back.
        h.client.set_autodrive(true);
        h.client.hold_steering();
        h.client.release_steering();
        assert!(h.client.wild() && h.client.autodrive());
    }

    /// While the menu is open the next junction takes its default exit.
    #[test]
    fn menu_resets_the_junction_choice_to_the_default_exit() {
        let mut h = Harness::free(0);
        h.ride(250.0, 1.0);
        h.client.turn(1).expect("a junction ahead");
        let msg = h.client.turn_message().expect("turn intent");
        h.send(&msg);
        h.client.hold_steering();
        // Taking back the choice is sent to the server.
        let msg = h.client.turn_message().expect("choice taken back");
        match ocw_protocol::decode_client(&msg).unwrap() {
            C::Turn(t) => assert!(t.exit_edge_id.is_none(), "{t:?}"),
            other => panic!("unexpected {other:?}"),
        }
        h.send(&msg);
        let mut seen = false;
        for _ in 0..500 {
            h.ride(250.0, 0.1);
            if let Some(j) = h.client.junction_view() {
                assert_eq!(j.selected, j.default, "{j:?}");
                seen = true;
            }
        }
        assert!(seen);
        // Straight on through x = 600 m: still on the top row (z = 0).
        let (c, s) = h.positions();
        assert!(c.position[0] > 620.0 && c.position[2].abs() < 6.0, "{c:?}");
        assert!(close(c.position, s.position, 1.0), "{c:?} vs {s:?}");
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

    /// ↑ adds power on top of the device (e.g. a real trainer nobody pedals).
    #[test]
    fn keyboard_power_adds_to_the_device_power() {
        let mut h = Harness::new(0, false);
        h.ride(0.0, 1.0); // device reports 0 W
        assert_eq!(h.client.power_w(), 0.0);
        let msg = h.client.set_key_power(250.0).expect("sent at once");
        match ocw_protocol::decode_client(&msg).unwrap() {
            C::Sample(s) => assert_eq!(s.power_w, Some(250.0)),
            other => panic!("unexpected {other:?}"),
        }
        assert!(h.client.set_key_power(250.0).is_none(), "unchanged");
        h.send(&msg);
        // Device samples keep 0 W, the keyboard's 250 W stay on top.
        h.ride(0.0, 10.0);
        assert_eq!(h.client.power_w(), 250.0);
        let own = h.client.rider_id().unwrap();
        assert!(h.server.session().rider_state(own).unwrap().speed > 5.0);
        assert!(h.client.set_key_power(-5.0).is_some());
        assert_eq!(h.client.key_power_w(), 0.0);
    }

    /// A 4 × 4 grid of crossroads 300 m apart (vertex 4·row + col at
    /// x = 300·col, z = 300·row; +z is south).
    fn grid() -> ocw_world::graph::RoadGraph {
        use ocw_world::graph::{Edge, EdgeId, Oneway, RoadClass, RoadGraph, VertexId};
        let n = 4u32;
        let mut vertices = Vec::new();
        for row in 0..n {
            for col in 0..n {
                vertices.push([f64::from(col) * 300.0, f64::from(row) * 300.0]);
            }
        }
        let mut pairs = Vec::new();
        for row in 0..n {
            for col in 0..n {
                let i = row * n + col;
                if col + 1 < n {
                    pairs.push((i, i + 1));
                }
                if row + 1 < n {
                    pairs.push((i, i + n));
                }
            }
        }
        RoadGraph {
            edges: pairs
                .iter()
                .enumerate()
                .map(|(k, &(a, b))| Edge {
                    id: EdgeId(9000 + k as u64),
                    from: VertexId(a),
                    to: VertexId(b),
                    points: vec![vertices[a as usize], vertices[b as usize]],
                    class: RoadClass::Tertiary,
                    name: Some(format!("Road {k}")),
                    width_m: 6.0,
                    bridge: false,
                    tunnel: false,
                    dead_end_m: None,
                    boundary: false,
                    oneway: Oneway::No,
                    osm_way: k as i64,
                    profile: Vec::new(),
                })
                .collect(),
            vertices,
            boundary_vertices: Default::default(),
        }
    }

    impl Harness {
        /// Where the local rider is on the client and on the server.
        fn positions(&self) -> (ocw_world::RoutePoint, ocw_world::RoutePoint) {
            let own = self.client.rider_id().unwrap();
            let server = self.server.session().rider_state(own).unwrap();
            (
                self.client.track().sample(self.client.state().distance),
                self.server.session().track(own).sample(server.distance),
            )
        }
    }

    fn close(a: [f64; 3], b: [f64; 3], tol: f64) -> bool {
        (a[0] - b[0]).hypot(a[2] - b[2]) < tol
    }

    /// Free ride through two junctions (spec §14, Phase 3): a chosen exit
    /// (right) and then the default (straight on). The client predicts its
    /// path, the server takes the same exits, and both agree.
    #[test]
    fn free_ride_through_junctions_with_a_chosen_and_a_default_exit() {
        let mut h = Harness::free(2);
        h.ride(250.0, 1.0);
        assert!(h.client.is_free_ride());
        let own = h.client.rider_id().unwrap();
        assert!(h.client.paths().contains_key(&own));
        // The junction at x = 600 m is 250 m ahead: no overlay yet, but a
        // pre-selection is allowed.
        assert!(h.client.junction_view().is_none());
        assert!(!h.client.junction_active());
        let said = h.client.turn(1).expect("a junction ahead");
        assert!(said.contains("right"), "{said}");
        let msg = h.client.turn_message().expect("turn intent");
        match ocw_protocol::decode_client(&msg).unwrap() {
            C::Turn(t) => assert!(t.exit_edge_id.is_some() && t.junction_id == 2, "{t:?}"),
            other => panic!("unexpected {other:?}"),
        }
        h.send(&msg);
        assert!(h.client.turn_message().is_none(), "unchanged");
        let (mut active, mut locked) = (false, false);
        for _ in 0..400 {
            h.ride(250.0, 0.1);
            if let Some(j) = h.client.junction_view() {
                assert!(j.exits[j.selected].angle_deg > 60.0, "{j:?}");
                assert_ne!(j.selected, j.default);
                if j.locked {
                    locked = true;
                    assert!(j.distance_m < lock_distance(12.0));
                } else {
                    active = true;
                    assert!(j.distance_m <= 150.0 && j.distance_m >= ocw_world::path::LOCK_M - 1.0);
                }
            }
        }
        assert!(active && locked);
        // Turned right: heading south on the x = 600 m road.
        let (c, s) = h.positions();
        assert!(
            (c.position[0] - 600.0).abs() < 4.0 && c.position[2] > 30.0,
            "{c:?}"
        );
        assert!(c.direction[1] > 0.99);
        assert!(close(c.position, s.position, 1.0), "{c:?} vs {s:?}");
        // The next junction (x 600, z 300): nothing chosen, straight on.
        h.ride(250.0, 45.0);
        let (c, s) = h.positions();
        assert!(
            (c.position[0] - 600.0).abs() < 4.0 && c.position[2] > 320.0,
            "{c:?}"
        );
        assert!(c.direction[1] > 0.99, "{c:?}");
        assert!(close(c.position, s.position, 1.0), "{c:?} vs {s:?}");
        assert!(h.client.prediction_error().abs() < 0.5);
        // Same path on both sides.
        let d = h.client.state().distance;
        let server_path = h.server.session().path(own).unwrap();
        for k in [-150.0, -60.0, 0.0, 20.0] {
            let a = h.client.track().sample(d + k).position;
            let b = server_path.sample(d + k).position;
            assert!(close(a, b, 1e-6), "{k}: {a:?} vs {b:?}");
        }
        // The bots ride their own ways and are drawn.
        assert_eq!(h.client.view(h.time_s, 1.0).len(), 3);
        assert_eq!(h.client.paths().len(), 3);
        assert_eq!(h.client.stats().malformed, 0);
    }

    /// The choice did not reach the server in time: the server's default
    /// applies and corrects the client's predicted path (spec §6.5).
    #[test]
    fn free_ride_late_choice_is_corrected_by_the_server() {
        let mut h = Harness::free(0);
        h.ride(250.0, 1.0);
        h.client.turn(1).unwrap(); // right, but never sent
        h.ride(250.0, 45.0);
        let (c, s) = h.positions();
        // Straight on (east) along the top row, like the server.
        assert!(c.position[0] > 650.0 && c.position[2].abs() < 4.0, "{c:?}");
        assert!(c.direction[0] > 0.99);
        assert!(close(c.position, s.position, 1.0), "{c:?} vs {s:?}");
    }

    /// U-turn (spec §2.2.2): predicted at once, the server builds the same
    /// loop from the client's position; rate limited.
    #[test]
    fn free_ride_u_turn_on_client_and_server() {
        let mut h = Harness::free(0);
        h.ride(250.0, 8.0);
        let msg = h.client.u_turn(h.time_s).expect("u-turn");
        assert!(h.client.u_turn(h.time_s + 1.0).is_none(), "rate limited");
        assert_eq!(
            h.client
                .notice()
                .as_deref()
                .map(|n| n.starts_with("U-turn")),
            Some(true)
        );
        h.send(&msg);
        h.ride(250.0, 25.0);
        let (c, s) = h.positions();
        assert!(c.direction[0] < -0.99, "{c:?}");
        assert!(close(c.position, s.position, 0.5), "{c:?} vs {s:?}");
        assert!(h.client.prediction_error().abs() < 0.5);
    }

    /// A route on the grid: from vertex 1 east to 2, south to 6 and 10,
    /// east to 11.
    fn grid_route(net: &RoadNetwork) -> Arc<RoutePlan> {
        let e = |a: u32, b: u32| {
            (0..net.graph().edges.len())
                .find_map(|i| {
                    let x = net.edge(i);
                    if (x.from.0, x.to.0) == (a, b) {
                        Some(ocw_world::Segment::whole(net, i, true))
                    } else if (x.from.0, x.to.0) == (b, a) {
                        Some(ocw_world::Segment::whole(net, i, false))
                    } else {
                        None
                    }
                })
                .unwrap()
        };
        let steps = vec![e(1, 2), e(2, 6), e(6, 10), e(10, 11)];
        Arc::new(RoutePlan::new(net, "Grid route", steps, false).unwrap())
    }

    impl Harness {
        /// A route ride on the grid with `bots` bots.
        fn route_ride(bots: u32) -> Self {
            let net = Arc::new(RoadNetwork::new(grid()).with_package_name("Grid"));
            let plan = grid_route(&net);
            let server = GameServer::new(
                ocw_server::Course::RouteRide {
                    net: net.clone(),
                    plan: plan.clone(),
                },
                ServerConfig {
                    allow_dev_commands: true,
                    bots,
                    start_m: 0.0,
                },
            );
            Self::with(
                server,
                ClientCore::free_ride(net, "Routed", RiderProfile::default()).with_route_plan(plan),
            )
        }
    }

    /// Route ride (spec §2.7, §2.2.1): the overlay shows the route's exit
    /// as the default, the client and the server follow it; another exit
    /// leaves the route, `ride.rejoin` brings the rider back; bots stay on
    /// it.
    #[test]
    fn route_ride_follow_leave_and_rejoin() {
        let mut h = Harness::route_ride(1);
        h.ride(250.0, 1.0);
        assert_eq!(h.client.course_name(), "Grid route");
        assert!(
            matches!(&h.notices[0], Notice::Joined { route_id, .. } if route_id == "Grid route")
        );
        assert_eq!(h.client.on_route(), Some(true));
        assert!(
            h.client
                .route_status()
                .unwrap()
                .starts_with("Grid route 0.0 / 1.2 km"),
            "{:?}",
            h.client.route_status()
        );
        // At vertex 2 the route turns right (south): that is the default.
        let mut saw_route_arrow = false;
        for _ in 0..450 {
            h.ride(250.0, 0.1);
            if let Some(j) = h.client.junction_view() {
                assert_eq!(j.route, Some(j.default), "{j:?}");
                assert_eq!(j.selected, j.default);
                assert!(j.exits[j.default].angle_deg > 60.0, "right: {j:?}");
                saw_route_arrow = true;
            }
        }
        assert!(saw_route_arrow);
        let (c, s) = h.positions();
        assert!(c.direction[1] > 0.99, "south on the route: {c:?}");
        assert!(close(c.position, s.position, 1.0), "{c:?} vs {s:?}");
        assert_eq!(h.client.on_route(), Some(true));
        // Vertex 6: leave the route (straight on is the route; turn left).
        h.ride(250.0, 5.0);
        let said = h.client.choose_direction("left").expect("junction at 6");
        assert!(said.contains("leaves the route"), "{said}");
        let msg = h.client.turn_message().unwrap();
        h.send(&msg);
        h.ride(250.0, 40.0);
        let (c, s) = h.positions();
        assert!(c.direction[0] > 0.99, "left = east, off the route: {c:?}");
        assert!(close(c.position, s.position, 1.0), "{c:?} vs {s:?}");
        assert_eq!(h.client.on_route(), Some(false));
        assert!(h.client.notice().unwrap().contains("ride.rejoin"));
        assert!(h.client.route_status().unwrap().contains("off the route"));
        // Back to the route: predicted, and the server agrees.
        let msg = h.client.rejoin_route().expect("a way back");
        assert!(h.client.rejoin_route().unwrap_err().contains("already"));
        h.send(&msg);
        h.ride(250.0, 150.0);
        let status = h.client.route_status().unwrap();
        assert!(!status.contains("off the route"), "{status}");
        let (c, s) = h.positions();
        assert!(close(c.position, s.position, 1.0), "{c:?} vs {s:?}");
        // The bot never left the route.
        let bot = h
            .server
            .session()
            .snapshot()
            .iter()
            .map(|r| r.id)
            .find(|&id| Some(id) != h.client.rider_id())
            .unwrap();
        let bd = h.server.session().rider_state(bot).unwrap().distance;
        let bp = h.server.session().path(bot).unwrap();
        assert!(bp.on_route_at(bd) || bp.route_progress(bd).unwrap().finished);
        assert_eq!(h.client.stats().malformed, 0);
    }

    /// Package version check (spec §6.5): a client with another build of the
    /// package is refused, by the server or (for servers that do not check)
    /// by the client itself.
    #[test]
    fn another_world_package_is_refused() {
        let net = Arc::new(RoadNetwork::new(grid()).with_package_name("Grid"));
        let mut other = grid();
        other.edges[0].points[1][0] += 1.0;
        let other = Arc::new(RoadNetwork::new(other).with_package_name("Grid"));
        let server = GameServer::new(
            ocw_server::Course::FreeRide {
                net,
                start: ocw_sim::FreeRideStart {
                    edge: 1,
                    offset_m: 10.0,
                    forward: true,
                },
            },
            ServerConfig::default(),
        );
        let mut h = Harness::with(
            server,
            ClientCore::free_ride(other.clone(), "Old", RiderProfile::default()),
        );
        assert!(
            matches!(&h.notices[..], [Notice::Rejected(r)] if r.contains("world package mismatch")),
            "{:?}",
            h.notices
        );
        assert_eq!(h.client.rider_id(), None);
        // A Welcome naming another graph is refused by the client too.
        let welcome = encode(&ocw_protocol::server_message(S::Welcome(v1::Welcome {
            protocol_version: PROTOCOL_VERSION,
            rider_id: 7,
            tick_rate_hz: 50,
            broadcast_rate_hz: 10,
            route_id: String::new(),
            free_ride: true,
            world: Some(v1::WorldPackageId {
                name: "Grid".into(),
                graph_hash: other.fingerprint() ^ 1,
            }),
            graph_route: String::new(),
        })));
        h.receive(&welcome);
        assert!(matches!(h.notices.last(), Some(Notice::Rejected(_))));
        assert_eq!(h.client.rider_id(), None);
    }

    #[test]
    fn exit_selection_steps_through_the_exits() {
        let mut h = Harness::free(0);
        h.ride(250.0, 1.0);
        // The top row's junction: straight on (the default) or right.
        // Nothing to the left of straight.
        assert!(h.client.turn(-1).unwrap().contains("default"));
        assert!(h.client.turn(1).unwrap().contains("right"));
        assert!(h.client.turn(1).unwrap().contains("right"));
        // Back to straight: the default, "no choice".
        assert!(h.client.turn(-1).unwrap().contains("default"));
        assert!(h.client.choose_direction("left").is_none());
        assert!(
            h.client
                .choose_direction("right")
                .unwrap()
                .contains("right")
        );
        assert!(h.client.clear_exit().unwrap().contains("default"));
        assert!(h.client.choose_exit(9).is_none());
        // Route rides have no junctions.
        let mut r = Harness::new(0, false);
        r.ride(200.0, 1.0);
        assert!(r.client.turn(1).is_none());
        assert!(r.client.u_turn(1.0).is_none());
        assert!(r.client.junction_view().is_none());
    }
}
