//! Client-side prediction of the local rider (spec §5.2, §6.1).

use ocw_core::{RiderProfile, SIM_DT};
use ocw_world::Track;

use ocw_core::RiderId;

use crate::collide::{self, Body, Frame, Mode, Response, Skid, WorldCache};
use crate::physics::{self, RiderState, Steering};
use crate::ride::{self, Controls};

/// Errors above this distance (m) are corrected immediately instead of smoothed.
pub const SNAP_THRESHOLD_M: f64 = 5.0;
/// Time over which smaller errors are blended away, in seconds.
pub const CORRECTION_TIME_S: f64 = 0.5;
/// A predicted contact with another rider pushes the own rider at most
/// this far in one tick (m; spec 14 Phase 12, 12B): a natural contact
/// overlaps by a centimetre or two a tick and is resolved at once, as on
/// the server; a large overlap only comes from news (a rider first seen
/// in a snapshot, a teleport) and is pushed out over a few ticks – the
/// sideways step stays under the 6 cm of the client's test.
pub const PREDICTED_PUSH_MAX_M: f64 = 0.025;
/// Other riders farther than this from the own rider (m, in the plane)
/// are left out of the predicted contacts.
pub const OTHERS_NEAR_M: f64 = 8.0;

/// Predicts the local rider so the bike reacts to pedaling immediately, and
/// converges smoothly to the authoritative server state.
#[derive(Debug, Clone)]
pub struct Predictor {
    profile: RiderProfile,
    state: RiderState,
    /// Remaining distance error still to be blended in.
    pending_correction: f64,
    /// Metres per second at which the pending correction is applied.
    correction_rate: f64,
    /// Distance error measured at the last reconciliation (for diagnostics).
    last_error: f64,
    /// Drafting factor on the drag, predicted by the client (1.0 = none).
    drag_factor: f64,
    /// Steering, wild mode and brake (the same rule as the server, so the
    /// bike reacts at once).
    controls: Controls,
    /// Scraped along the road edge in the last predicted tick.
    scraping: bool,
    /// Remaining lateral error to blend in, and its rate (m/s).
    pending_lateral: f64,
    lateral_rate: f64,
    /// Autodrive avoidance's virtual brake (spec §14 Phase 12D), as on
    /// the server.
    speed_cap: Option<f64>,
    /// The world's static obstacles (spec §14 Phase 12D, K3: the own
    /// rider's contacts with them are predicted exactly; crashes come only
    /// from the server) and the collision model in force.
    world: WorldCache,
    mode: Mode,
    ticks: u64,
    /// The session weather's physics while it is in force (spec §14 Phase
    /// 9, S6; the server's switch), as the server rides it.
    weather: Option<crate::weather::WeatherPhysics>,
    /// The damage's effects the server applies to the rider (spec §14
    /// Phase 12E, K4: from `RiderUpdate.damage`).
    damage: crate::damage::Effects,
    /// The skid of a lean beyond the wet road's grip (S6 → Phase 12: the
    /// speed it costs, as on the server; the fall is only the server's).
    skid: Skid,
    /// The other riders as last known (spec 14 Phase 12, 6.1): their
    /// bodies in the collision frame, moved on with their speed each tick;
    /// the own rider's contacts with them are predicted with the server's
    /// code (`collide::resolve`).
    others: Vec<Body>,
    frame: Frame,
    /// What the last step's predicted contacts did to the others (the
    /// client moves its copies of them alike).
    other_responses: Vec<(RiderId, Response)>,
    /// A ghost to other riders, as on the server after a join (12A,
    /// `session::JOIN_GHOST_S`): for these ticks, then until clear.
    ghost_ticks: u32,
    solid: bool,
    /// Ticks of the rider contacts (the tie's coin; crashes are the
    /// server's anyway).
    rider_ticks: u64,
    /// Wet leaves on the road (spec 4.6.2 Q6): the map and the season as
    /// the server has them, and the sway now in the lateral offset (the
    /// cost and the wobble are predicted; a slide is the server's).
    leaves: Option<crate::leaves::LeafGround>,
    leaf_wobble: f64,
    /// Off the road on the terrain (spec §14 Phase 13A): the track the
    /// rider rides there (as the server's), and the position error still
    /// to blend in (x, z) with its rate (m/s).
    off_road: Option<ocw_world::ground::OffRoadTrack>,
    pending_xz: [f64; 2],
    xz_rate: f64,
}

impl Predictor {
    pub fn new(profile: RiderProfile, initial: RiderState) -> Self {
        Self {
            profile,
            state: initial,
            pending_correction: 0.0,
            correction_rate: 0.0,
            last_error: 0.0,
            drag_factor: 1.0,
            controls: Controls::default(),
            scraping: false,
            pending_lateral: 0.0,
            lateral_rate: 0.0,
            speed_cap: None,
            world: WorldCache::default(),
            mode: Mode::Off,
            ticks: 0,
            weather: None,
            damage: crate::damage::Effects::NONE,
            skid: Skid::default(),
            others: Vec::new(),
            frame: Frame::Path { loop_m: None },
            other_responses: Vec::new(),
            ghost_ticks: Self::GHOST_TICKS,
            solid: false,
            rider_ticks: 0,
            leaves: None,
            leaf_wobble: 0.0,
            off_road: None,
            pending_xz: [0.0; 2],
            xz_rate: 0.0,
        }
    }

    /// Off the road: the terrain's track the rider rides now (spec §14
    /// Phase 13A); `None` on the road.
    pub fn off_road(&self) -> Option<&ocw_world::ground::OffRoadTrack> {
        self.off_road.as_ref()
    }

    /// Takes the server's off-road place as it is (a landing, a snap) or
    /// leaves the terrain (`None`: the server put the rider on a road).
    pub fn set_off_road(&mut self, track: Option<ocw_world::ground::OffRoadTrack>) {
        self.off_road = track;
        self.pending_xz = [0.0; 2];
        self.controls.wild |= self.off_road.is_some();
    }

    /// One predicted tick off the road, as the server's step: the terrain's
    /// physics, the damage's cap, the contacts with the world.
    fn off_road_tick(
        &mut self,
        state: &mut RiderState,
        track: &ocw_world::ground::OffRoadTrack,
        power_w: f64,
        scraping: &mut bool,
    ) -> ocw_world::ground::OffRoadTrack {
        let (profile, power_w, controls) = self.damaged(power_w);
        let (next, _) = crate::offroad::tick(
            state,
            track,
            &profile,
            power_w,
            controls,
            scraping,
            1.0,
            self.weather.as_ref(),
            SIM_DT,
        );
        self.damage_cap(state);
        let wild = self.controls.wild;
        self.controls.wild = true;
        self.world_tick(state, &next, true);
        self.controls.wild = wild;
        crate::offroad::normalize(state, &next)
    }

    /// Reconciles off the road with the server's state and place, `age_s`
    /// old (spec §14 Phase 13A): extrapolated with the same physics, then
    /// compared in the plane; errors above [`SNAP_THRESHOLD_M`] (or the
    /// first state off the road) are taken at once, smaller ones blended
    /// in over [`CORRECTION_TIME_S`].
    pub fn reconcile_off_road(
        &mut self,
        server: RiderState,
        track: ocw_world::ground::OffRoadTrack,
        age_s: f64,
        power_w: f64,
    ) {
        let steps = (age_s.max(0.0) / SIM_DT).round() as usize;
        let mut st = server;
        let mut tr = track;
        let mut scraping = false;
        for _ in 0..steps {
            tr = self.off_road_tick(&mut st, &tr, power_w, &mut scraping);
        }
        let there = tr.at();
        let Some(mine) = &self.off_road else {
            self.last_error = SNAP_THRESHOLD_M * 2.0;
            self.state = st;
            self.off_road = Some(tr);
            self.pending_xz = [0.0; 2];
            self.pending_correction = 0.0;
            self.pending_lateral = 0.0;
            return;
        };
        let here = mine.at();
        let err = [there[0] - here[0], there[1] - here[1]];
        let dist = err[0].hypot(err[1]);
        self.last_error = dist;
        if dist > SNAP_THRESHOLD_M {
            self.state = st;
            self.off_road = Some(tr);
            self.pending_xz = [0.0; 2];
        } else {
            // The odometer and the speed are the server's; the place is
            // blended, the direction follows halfway per snapshot.
            let mut pose = crate::offroad::OffRoadPose::of(mine);
            let target = crate::offroad::OffRoadPose::of(&tr);
            pose.yaw = crate::offroad::OffRoadPose::lerp(pose, target, 0.5).yaw;
            self.state.distance = st.distance;
            self.state.speed = st.speed;
            self.off_road = Some(pose.track(tr.ground().clone(), st.distance));
            self.pending_xz = err;
            self.xz_rate = dist / CORRECTION_TIME_S;
        }
        self.pending_correction = 0.0;
        self.pending_lateral = 0.0;
    }

    /// Where leaves lie and the season (spec 4.6.2 Q6), as the server.
    pub fn set_leaves(&mut self, leaves: Option<crate::leaves::LeafGround>) {
        self.leaves = leaves;
    }

    pub fn leaves(&self) -> Option<&crate::leaves::LeafGround> {
        self.leaves.as_ref()
    }

    /// The physics tick on wet leaves (spec 4.6.2 Q6) as the server's:
    /// the rolling resistance on the leaves, then their sway.
    #[allow(clippy::too_many_arguments)]
    fn leaf_tick(
        &self,
        state: &mut RiderState,
        wobble: &mut f64,
        profile: &RiderProfile,
        power_w: f64,
        controls: Controls,
        scraping: &mut bool,
        route: &dyn Track,
    ) {
        let slick = crate::leaves::slick(self.leaves.as_ref(), self.weather.as_ref(), route, state);
        let weather = self
            .weather
            .as_ref()
            .map(|w| crate::leaves::on_leaves(w, slick));
        ride::tick_in(
            state,
            profile,
            power_w,
            controls,
            scraping,
            route,
            self.drag_factor,
            weather.as_ref(),
            SIM_DT,
        );
        let limit = if controls.wild {
            physics::WILD_LIMIT_M
        } else {
            physics::lateral_limit(route.road_width_at(state.distance))
        };
        crate::leaves::apply_wobble(state, slick, wobble, limit);
    }

    const GHOST_TICKS: u32 = (crate::session::JOIN_GHOST_S / SIM_DT + 0.5) as u32;

    /// The other riders for the next steps (spec 14 Phase 12, 6.1): their
    /// last known bodies in `frame` (a route's path frame, or the world in
    /// free ride; fallen riders as obstacles). The own rider's contacts
    /// with them are predicted; crashes, wobbles and damage stay the
    /// server's.
    pub fn set_others(&mut self, frame: Frame, others: Vec<Body>) {
        self.frame = frame;
        self.others = others;
    }

    /// What the last step's predicted contacts did to each other rider
    /// touched (their push along and beside their road, speed, heading).
    pub fn other_responses(&self) -> &[(RiderId, Response)] {
        &self.other_responses
    }

    /// Joined (again): a ghost to the other riders for
    /// `session::JOIN_GHOST_S` and then until clear of them, as the server
    /// keeps a new client rider.
    pub fn rejoin(&mut self) {
        self.ghost_ticks = Self::GHOST_TICKS;
        self.solid = false;
    }

    /// Takes part in the predicted rider contacts (no longer a ghost).
    pub fn solid(&self) -> bool {
        self.solid
    }

    /// The own rider's body in the collision frame.
    fn own_body(&self, state: &RiderState, route: &dyn Track, controls: Controls) -> Body {
        let (pos, road) = match self.frame {
            Frame::Path { .. } => ([state.distance, state.lateral_offset], [1.0, 0.0]),
            Frame::World => {
                let p = route.sample(state.distance);
                let at = p.offset_position(state.lateral_offset);
                ([at[0], at[2]], p.direction)
            }
        };
        Body {
            id: RiderId(u32::MAX),
            pos,
            road,
            heading: state.heading,
            speed: state.speed,
            lean: collide::lean_of(state.speed, state.turn_curvature),
            mass: self.profile.rider_mass_kg + self.profile.bike_mass_kg,
            fallen: false,
            manual: matches!(controls.steering, Steering::Lean(_)) || controls.wild,
            grip: crate::weather::grip(self.weather.as_ref(), route, state),
        }
    }

    /// One tick of the contacts with the other riders (after the physics
    /// step, before the world's, as on the server): the others moved on a
    /// tick, then `collide::resolve` over the own body and the near ones.
    /// The own push (at most [`PREDICTED_PUSH_MAX_M`]), speed and heading
    /// are applied to `state`, the others' responses to their bodies and
    /// returned.
    fn rider_tick(
        &self,
        state: &mut RiderState,
        route: &dyn Track,
        controls: Controls,
        others: &mut [Body],
        solid: bool,
        tick: u64,
    ) -> Vec<(RiderId, Response)> {
        for b in others.iter_mut() {
            let v = b.velocity();
            b.pos[0] += v[0] * SIM_DT;
            b.pos[1] += v[1] * SIM_DT;
        }
        if !solid || !self.mode.on() || others.is_empty() {
            return Vec::new();
        }
        let own = self.own_body(state, route, controls);
        let near: Vec<usize> = (0..others.len())
            .filter(|&i| self.plane_distance(&own, &others[i]) < OTHERS_NEAR_M)
            .collect();
        if near.is_empty() {
            return Vec::new();
        }
        let mut bodies = Vec::with_capacity(near.len() + 1);
        bodies.push(own);
        bodies.extend(near.iter().map(|&i| others[i]));
        let out = collide::resolve(&bodies, self.frame, self.mode, tick);
        let mine = out[0];
        if mine.touched() {
            let push = mine.ds.hypot(mine.dd);
            let k = if push > PREDICTED_PUSH_MAX_M {
                PREDICTED_PUSH_MAX_M / push
            } else {
                1.0
            };
            let limit = if controls.wild {
                physics::WILD_LIMIT_M
            } else {
                physics::lateral_limit(route.road_width_at(state.distance))
            };
            state.distance += mine.ds * k;
            state.lateral_offset = (state.lateral_offset + mine.dd * k).clamp(-limit, limit);
            if let Some(v) = mine.speed {
                state.speed = v;
            }
            if let Some(h) = mine.heading {
                state.heading = h;
            }
        }
        let mut touched = Vec::new();
        for (j, &i) in near.iter().enumerate() {
            let r = out[j + 1];
            if !r.touched() {
                continue;
            }
            let b = &mut others[i];
            let right = b.road_right();
            b.pos[0] += b.road[0] * r.ds + right[0] * r.dd;
            b.pos[1] += b.road[1] * r.ds + right[1] * r.dd;
            if let Some(v) = r.speed {
                b.speed = v;
            }
            if let Some(h) = r.heading {
                b.heading = h;
            }
            touched.push((b.id, r));
        }
        touched
    }

    /// Distance of two bodies in the plane (a loop's path frame wraps).
    fn plane_distance(&self, a: &Body, b: &Body) -> f64 {
        let mut dx = b.pos[0] - a.pos[0];
        if let Frame::Path { loop_m: Some(l) } = self.frame
            && l > 0.0
        {
            dx -= l * (dx / l).round();
        }
        dx.hypot(b.pos[1] - a.pos[1])
    }

    /// The ghost after a join counts down; then solid once the own body
    /// overlaps nobody (as the server's `joining` rule).
    fn ghost_tick(&mut self, route: &dyn Track) {
        if self.solid {
            return;
        }
        if self.ghost_ticks > 0 {
            self.ghost_ticks -= 1;
            return;
        }
        let own = self.own_body(&self.state, route, self.controls);
        let mut bodies = vec![own];
        bodies.extend(self.others.iter().copied());
        self.solid = !collide::detect(&bodies, self.frame)
            .iter()
            .any(|c| c.a == 0 || c.b == 0);
    }

    /// One tick of the skid after the physics step (as the server's step).
    fn skid_tick(
        &self,
        skid: &mut Skid,
        state: &mut RiderState,
        route: &dyn Track,
        controls: Controls,
    ) {
        let step = crate::collide::skid(
            skid,
            crate::collide::skid_lean(controls.steering, route, state.distance),
            crate::weather::grip(self.weather.as_ref(), route, state),
            state.speed,
            self.profile.rider_mass_kg + self.profile.bike_mass_kg,
            self.mode,
            SIM_DT,
        );
        state.speed -= step.speed_loss;
    }

    /// The world's static obstacles (spec §14 Phase 12 K3).
    pub fn set_world(
        &mut self,
        world: Option<std::sync::Arc<ocw_world::obstacles::WorldObstacles>>,
    ) {
        let woods = self.world.woods.take();
        self.world = WorldCache::new(world).with_woods(woods);
    }

    /// The drawn trees off the road as trunks (spec 14 Phase 13 Q10 (a)),
    /// the same as the server's.
    pub fn set_woods(&mut self, woods: Option<std::sync::Arc<ocw_world::woods::Woods>>) {
        let world = self.world.world.take();
        self.world = WorldCache::new(world).with_woods(woods);
    }

    /// The damage's effects in force (the server's, spec §14 Phase 12E).
    pub fn set_damage(&mut self, effects: crate::damage::Effects) {
        self.damage = effects;
    }

    pub fn damage(&self) -> crate::damage::Effects {
        self.damage
    }

    /// The profile, power and controls the rider rides with under the
    /// damage's effects (as the server's step).
    fn damaged(&self, power_w: f64) -> (RiderProfile, f64, Controls) {
        let fx = self.damage;
        let mut controls = self.controls;
        if let Steering::Lean(l) = controls.steering {
            controls.steering = Steering::Lean(l * fx.lean_factor);
        }
        controls.braking |= fx.held;
        (fx.profile(&self.profile), fx.power(power_w), controls)
    }

    fn damage_cap(&self, state: &mut RiderState) {
        if let Some(c) = self.damage.speed_cap
            && state.speed > c
        {
            state.speed = c;
        }
    }

    /// The slip of a lean beyond the wet road's grip (S6 → Phase 12: 0 =
    /// gripping, 1 = the fall), for the HUD's warning.
    pub fn slip(&self) -> f64 {
        self.skid.slip
    }

    /// The collision model the server says is in force.
    pub fn set_collision_mode(&mut self, mode: Mode) {
        self.mode = mode;
    }

    /// In wild mode: pushed out of the world's solids, the speed into
    /// them lost (as the server does it; a crash is only the server's).
    /// Off the road (`deep`) the drawn trees' trunks too (spec 14 Phase 13
    /// Q10 (a)).
    fn world_tick(&mut self, state: &mut RiderState, route: &dyn Track, deep: bool) {
        if !self.controls.wild || !self.mode.on() || self.world.is_empty() {
            return;
        }
        self.ticks += 1;
        let p = route.sample(state.distance);
        let at = p.offset_position(state.lateral_offset);
        let body = Body {
            id: ocw_core::RiderId(0),
            pos: [at[0], at[2]],
            road: p.direction,
            heading: state.heading,
            speed: state.speed,
            lean: 0.0,
            mass: self.profile.rider_mass_kg + self.profile.bike_mass_kg,
            fallen: false,
            manual: true,
            grip: crate::weather::grip(self.weather.as_ref(), route, state),
        };
        let mut scratch = Vec::new();
        let r = self.world.resolve(
            &body,
            at[1],
            self.ticks,
            self.mode,
            physics::WILD_MAX_HEADING,
            deep,
            &mut scratch,
        );
        if r.touched() {
            state.distance += r.ds;
            state.lateral_offset =
                (state.lateral_offset + r.dd).clamp(-physics::WILD_LIMIT_M, physics::WILD_LIMIT_M);
            if let Some(v) = r.speed {
                state.speed = v;
            }
            if let Some(h) = r.heading {
                state.heading = h;
            }
        }
        if self.ticks.is_multiple_of(500) {
            self.world.sweep(self.ticks);
        }
    }

    /// A new physics profile (the workshop's bike change, spec §14 Phase 4,
    /// 4A): the server applies the same at once.
    pub fn set_profile(&mut self, profile: RiderProfile) {
        self.profile = profile;
    }

    pub fn profile(&self) -> &RiderProfile {
        &self.profile
    }

    /// Autodrive avoidance's speed cap for the next steps (m/s; `None`: none).
    pub fn set_speed_cap(&mut self, cap: Option<f64>) {
        self.speed_cap = cap;
    }

    /// Takes the server's state as it is, dropping pending corrections
    /// (the rider is held, e.g. at an event's start line).
    pub fn set_state(&mut self, state: RiderState) {
        self.last_error = state.distance - self.state.distance;
        self.state = state;
        self.pending_correction = 0.0;
        self.pending_lateral = 0.0;
    }

    pub fn state(&self) -> RiderState {
        self.state
    }

    /// Distance error (server − prediction) at the last reconciliation, in metres.
    pub fn last_error(&self) -> f64 {
        self.last_error
    }

    pub fn set_braking(&mut self, braking: bool) {
        self.controls.braking = braking;
    }

    /// Sets the steering target (already clamped to the road by the caller).
    pub fn set_lateral_target(&mut self, target_m: f64) {
        self.controls.steering = Steering::Auto(target_m);
    }

    /// Sets lean steering (rad, positive = right).
    pub fn set_lean(&mut self, lean_rad: f64) {
        self.controls.steering = Steering::Lean(lean_rad);
    }

    /// Wild mode: not bound to the road.
    pub fn set_wild(&mut self, wild: bool) {
        self.controls.wild = wild;
    }

    /// The weather's physics in force (spec §14 Phase 9, S6; `None`: the
    /// weather does not affect the physics), from the session weather the
    /// server sends.
    pub fn set_weather(&mut self, weather: Option<crate::weather::WeatherPhysics>) {
        self.weather = weather;
    }

    pub fn weather(&self) -> Option<&crate::weather::WeatherPhysics> {
        self.weather.as_ref()
    }

    /// Sets the predicted drafting factor (from the positions of others).
    pub fn set_drag_factor(&mut self, drag_factor: f64) {
        self.drag_factor = drag_factor;
    }

    /// Advances the prediction by one physics tick with the current power
    /// (off the road on the terrain's track instead of `route`).
    pub fn step(&mut self, route: &dyn Track, power_w: f64) {
        if let Some(track) = self.off_road.clone() {
            let (mut state, mut scraping) = (self.state, self.scraping);
            let mut next = self.off_road_tick(&mut state, &track, power_w, &mut scraping);
            if let Some(c) = self.speed_cap
                && state.speed > c
            {
                state.speed = c;
            }
            // The server's place blended in.
            let left = self.pending_xz[0].hypot(self.pending_xz[1]);
            if left > 0.0 {
                let k = ((self.xz_rate * SIM_DT) / left).min(1.0);
                let d = [self.pending_xz[0] * k, self.pending_xz[1] * k];
                let at = next.at();
                next = ocw_world::ground::OffRoadTrack::new(
                    next.ground().clone(),
                    [at[0] + d[0], at[1] + d[1]],
                    next.direction(),
                    next.anchor_m(),
                );
                self.pending_xz = [self.pending_xz[0] - d[0], self.pending_xz[1] - d[1]];
            }
            self.state = state;
            self.scraping = scraping;
            self.off_road = Some(next);
            return;
        }
        let (profile, power_w, controls) = self.damaged(power_w);
        let (mut state, mut wobble, mut scraping) = (self.state, self.leaf_wobble, self.scraping);
        self.leaf_tick(
            &mut state,
            &mut wobble,
            &profile,
            power_w,
            controls,
            &mut scraping,
            route,
        );
        (self.state, self.leaf_wobble, self.scraping) = (state, wobble, scraping);
        let (mut skid, mut state) = (self.skid, self.state);
        self.skid_tick(&mut skid, &mut state, route, controls);
        self.skid = skid;
        self.state = state;
        let mut capped = self.state;
        self.damage_cap(&mut capped);
        self.state = capped;
        self.rider_ticks += 1;
        let mut state = self.state;
        let mut others = std::mem::take(&mut self.others);
        self.other_responses = self.rider_tick(
            &mut state,
            route,
            controls,
            &mut others,
            self.solid,
            self.rider_ticks,
        );
        self.others = others;
        self.state = state;
        self.ghost_tick(route);
        let mut state = self.state;
        self.world_tick(&mut state, route, false);
        self.state = state;
        if let Some(c) = self.speed_cap
            && self.state.speed > c
        {
            self.state.speed = c;
        }
        if self.pending_lateral != 0.0 {
            let max_step = self.lateral_rate.abs() * SIM_DT;
            let applied = self.pending_lateral.clamp(-max_step, max_step);
            self.state.lateral_offset += applied;
            self.pending_lateral -= applied;
        }
        if self.pending_correction != 0.0 {
            // Apply the error linearly over CORRECTION_TIME_S.
            let max_step = self.correction_rate.abs() * SIM_DT;
            let applied = self.pending_correction.clamp(-max_step, max_step);
            self.state.distance += applied;
            self.pending_correction -= applied;
        }
        ride::stop_at_finish(&mut self.state, route);
    }

    /// Reconciles with an authoritative state that is `age_s` seconds old.
    ///
    /// The server state is extrapolated to "now" with the same physics and the
    /// current power, then compared with the prediction.
    pub fn reconcile(&mut self, route: &dyn Track, server: RiderState, age_s: f64, power_w: f64) {
        let mut extrapolated = server;
        let steps = (age_s.max(0.0) / SIM_DT).round() as usize;
        let mut scraping = self.scraping;
        let mut skid = self.skid;
        let (profile, power_w, controls) = self.damaged(power_w);
        // The others as the same snapshot shows them, moved on with the
        // extrapolation (their contacts with the own rider included).
        let mut others = self.others.clone();
        // The server's state carries the leaves' sway of its place.
        let slick =
            crate::leaves::slick(self.leaves.as_ref(), self.weather.as_ref(), route, &server);
        let mut wobble = crate::leaves::wobble_offset(slick, server.speed, server.distance);
        for _ in 0..steps {
            self.leaf_tick(
                &mut extrapolated,
                &mut wobble,
                &profile,
                power_w,
                controls,
                &mut scraping,
                route,
            );
            self.skid_tick(&mut skid, &mut extrapolated, route, controls);
            self.damage_cap(&mut extrapolated);
            self.rider_tick(
                &mut extrapolated,
                route,
                controls,
                &mut others,
                self.solid,
                self.rider_ticks,
            );
            self.world_tick(&mut extrapolated, route, false);
        }
        let error = extrapolated.distance - self.state.distance;
        self.last_error = error;
        if error.abs() > SNAP_THRESHOLD_M {
            self.state = extrapolated;
            self.pending_correction = 0.0;
            self.pending_lateral = 0.0;
        } else {
            self.pending_correction = error;
            self.correction_rate = error / CORRECTION_TIME_S;
            self.state.speed = extrapolated.speed;
            // Lateral errors are blended too (the steering intent reaches
            // the server a little later than the prediction reacts).
            let lateral_error = extrapolated.lateral_offset - self.state.lateral_offset;
            self.pending_lateral = lateral_error;
            self.lateral_rate = lateral_error / CORRECTION_TIME_S;
            // The heading follows the server halfway per snapshot (10 Hz).
            self.state.heading += (extrapolated.heading - self.state.heading) * 0.5;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::{self, Environment};
    use ocw_world::Route;

    fn flat() -> Route {
        Route::from_points(
            "flat",
            "Flat",
            vec![[0.0, 0.0, 0.0], [100_000.0, 0.0, 0.0]],
            false,
            6.0,
        )
        .unwrap()
    }

    #[test]
    fn set_state_drops_pending_corrections() {
        let route = flat();
        let mut p = Predictor::new(RiderProfile::default(), RiderState::default());
        p.reconcile(
            &route,
            RiderState {
                distance: 2.0,
                ..Default::default()
            },
            0.0,
            0.0,
        );
        let held = RiderState {
            distance: 3.0,
            lateral_offset: 1.2,
            ..Default::default()
        };
        p.set_state(held);
        assert_eq!(p.state(), held);
        assert_eq!(p.last_error(), 3.0);
        p.step(&route, 0.0);
        assert_eq!(p.state().distance, 3.0, "no correction left to blend in");
    }

    #[test]
    fn small_errors_are_blended() {
        let route = flat();
        let mut p = Predictor::new(RiderProfile::default(), RiderState::default());
        for _ in 0..100 {
            p.step(&route, 200.0);
        }
        let mut server = p.state();
        server.distance += 1.0;
        p.reconcile(&route, server, 0.0, 200.0);
        assert!((p.last_error() - 1.0).abs() < 1e-9);
        let before = p.state().distance;
        p.step(&route, 200.0);
        // Not snapped: only a fraction of the error is applied per tick.
        assert!(p.state().distance - before < 0.5);
        for _ in 0..100 {
            p.step(&route, 200.0);
        }
        let mut reference = server;
        for _ in 0..101 {
            physics::step(
                &mut reference,
                &RiderProfile::default(),
                200.0,
                &Environment::with_grade(0.0),
                SIM_DT,
            );
        }
        assert!((p.state().distance - reference.distance).abs() < 0.05);
    }

    #[test]
    fn large_errors_snap() {
        let route = flat();
        let mut p = Predictor::new(RiderProfile::default(), RiderState::default());
        let server = RiderState {
            distance: 50.0,
            speed: 8.0,
            lateral_offset: 0.0,
            ..Default::default()
        };
        p.reconcile(&route, server, 0.0, 200.0);
        assert_eq!(p.state(), server);
    }

    #[test]
    fn agreeing_server_causes_no_correction() {
        let route = flat();
        let mut p = Predictor::new(RiderProfile::default(), RiderState::default());
        let mut server = RiderState::default();
        for tick in 1..=500 {
            p.step(&route, 220.0);
            physics::step(
                &mut server,
                &RiderProfile::default(),
                220.0,
                &Environment::with_grade(0.0),
                SIM_DT,
            );
            if tick % 5 == 0 {
                p.reconcile(&route, server, 0.0, 220.0);
                assert!(p.last_error().abs() < 1e-9);
            }
        }
        assert_eq!(p.state(), server);
    }

    #[test]
    fn old_server_state_is_extrapolated_before_comparing() {
        let route = flat();
        let profile = RiderProfile::default();
        let mut server = RiderState {
            distance: 100.0,
            speed: 9.0,
            lateral_offset: 0.0,
            ..Default::default()
        };
        let old = server;
        // The server state is 0.1 s old; meanwhile we rode on.
        for _ in 0..5 {
            physics::step(
                &mut server,
                &profile,
                200.0,
                &Environment::with_grade(0.0),
                SIM_DT,
            );
        }
        let mut p = Predictor::new(profile, server);
        p.reconcile(&route, old, 0.1, 200.0);
        assert!(p.last_error().abs() < 1e-9, "error {}", p.last_error());
    }

    /// Spec §14 Phase 12D / K3: the own rider's contact with a wall in
    /// wild mode is predicted (pushed out, stopped) like the server's.
    #[test]
    fn wild_mode_contacts_with_the_world_are_predicted() {
        use ocw_world::obstacles::{LineObstacle, ObstacleKind, Obstacles, TileObstacles};
        let mut o = Obstacles::new(1000.0);
        o.tiles.insert(
            ocw_world::TileKey { x: 0, z: 0 },
            TileObstacles {
                lines: vec![LineObstacle {
                    kind: ObstacleKind::Wall,
                    flags: 0,
                    height_m: 1.5,
                    points: vec![[300.0, 10.0], [300.0, 30.0]],
                }],
                ..Default::default()
            },
        );
        let pkg = ocw_world::WorldPackage {
            name: "w".into(),
            origin: (47.0, 9.0),
            tile_size_m: 1000.0,
            samples: 2,
            tiles: Default::default(),
            graph: None,
            landuse: None,
            buildings: None,
            bridges: None,
            routes: Vec::new(),
            region: None,
            time_zone: None,
            look: Default::default(),
            obstacles: Some(o),
            places: None,
            junctions: None,
        };
        let world =
            ocw_world::obstacles::WorldObstacles::from_package(&pkg).map(std::sync::Arc::new);
        let route = flat();
        let start = RiderState {
            distance: 290.0,
            lateral_offset: 20.0,
            speed: 5.0,
            ..Default::default()
        };
        let run = |mode: Mode| {
            let mut p = Predictor::new(RiderProfile::default(), start);
            p.set_world(world.clone());
            p.set_collision_mode(mode);
            p.set_wild(true);
            p.set_lean(0.0);
            for _ in 0..150 {
                p.step(&route, 0.0);
            }
            p.state()
        };
        let stopped = run(Mode::Full);
        assert!(
            stopped.distance + 0.875 <= 299.81 && stopped.speed < 0.5,
            "{stopped:?}"
        );
        // Collisions off: through it.
        assert!(run(Mode::Off).distance > 300.0);
    }

    /// A lean beyond the wet road's grip (S6 → Phase 12 skid): the
    /// prediction loses the same speed as the server, tick for tick.
    #[test]
    fn the_skid_is_predicted_as_the_server_rides_it() {
        let w = crate::weather::WeatherPhysics {
            wind: [0.0, 0.0],
            temperature_c: 10.0,
            pressure_hpa: 1013.25,
            pressure_y_m: 0.0,
            humidity: 0.8,
            rolling_factor: 1.0,
            grip: 0.75,
            wetness: 1.0,
            snow_cover: 0.0,
        };
        let route = flat();
        let mut s = crate::session::Session::new(route.clone());
        s.set_collisions(true);
        s.set_weather(Some(w));
        s.set_weather_physics(true);
        let id = s.add_rider("a", RiderProfile::default());
        for _ in 0..=110 {
            s.step();
        }
        let start = RiderState {
            distance: 100.0,
            speed: 10.0,
            ..Default::default()
        };
        s.set_rider_state(id, start);
        s.set_wild(id, true);
        s.set_lean(id, 1, physics::MAX_STEER_LEAN);
        let mut p = Predictor::new(RiderProfile::default(), start);
        p.set_weather(Some(w));
        p.set_collision_mode(Mode::Full);
        p.set_wild(true);
        p.set_lean(physics::MAX_STEER_LEAN);
        let mut free = p.clone();
        free.set_collision_mode(Mode::Off);
        // Before the server's fall (about 0.75 s on a wet road).
        for _ in 0..25 {
            s.step();
            p.step(&route, 0.0);
            free.step(&route, 0.0);
        }
        assert!(s.crash(id).is_none());
        let server = s.rider_state(id).unwrap();
        assert_eq!(p.state(), server);
        assert!(server.speed < free.state().speed - 0.3, "{server:?}");
    }

    /// The bodies of the session's riders but `me`, as the client sees
    /// them in a snapshot (the path frame of a route).
    fn others_of(s: &crate::session::Session, me: RiderId, ids: &[RiderId]) -> Vec<Body> {
        let mass = RiderProfile::default().rider_mass_kg + RiderProfile::default().bike_mass_kg;
        ids.iter()
            .filter(|id| **id != me)
            .map(|id| {
                let st = s.rider_state(*id).unwrap();
                let mut b = Body::on_path(
                    *id,
                    st.distance,
                    st.lateral_offset,
                    st.heading,
                    st.speed,
                    mass,
                );
                b.lean = collide::lean_of(st.speed, st.turn_curvature);
                b
            })
            .collect()
    }

    /// Spec 14 Phase 12 (6.1): the own rider leaning into a rider beside
    /// is pushed off by the prediction as the server pushes it – the same
    /// code (`collide::resolve`) – where a prediction without the others
    /// rides into the other rider and is corrected only by the server.
    #[test]
    fn a_bump_with_another_rider_is_predicted_as_the_server_resolves_it() {
        let route = flat();
        let mut s = crate::session::Session::new(route.clone());
        s.set_collisions(true);
        let me = s.add_rider("Ada Rider", RiderProfile::default());
        let other = s.add_rider("Bea Climber", RiderProfile::default());
        for _ in 0..=110 {
            s.step();
        }
        let start = RiderState {
            distance: 100.0,
            speed: 8.0,
            ..Default::default()
        };
        s.set_rider_state(me, start);
        s.set_rider_state(
            other,
            RiderState {
                distance: 100.3,
                lateral_offset: 0.75,
                speed: 8.0,
                ..Default::default()
            },
        );
        s.set_lean(me, 1, 0.06);
        let ids = [me, other];
        let mut p = Predictor::new(RiderProfile::default(), start);
        p.set_collision_mode(Mode::Full);
        p.set_lean(0.06);
        p.solid = true;
        let mut blind = p.clone();
        let (mut err, mut blind_err, mut pushed) = (0.0f64, 0.0f64, false);
        for _ in 0..60 {
            p.set_others(Frame::Path { loop_m: None }, others_of(&s, me, &ids));
            s.step();
            p.step(&route, 0.0);
            blind.step(&route, 0.0);
            pushed |= !p.other_responses().is_empty();
            let server = s.rider_state(me).unwrap();
            err = err.max((p.state().lateral_offset - server.lateral_offset).abs());
            blind_err = blind_err.max((blind.state().lateral_offset - server.lateral_offset).abs());
        }
        let stats = s.collision_stats();
        assert!(stats.bumps > 0 && stats.crashes == 0, "{stats:?}");
        assert!(pushed, "the other rider was pushed in the prediction too");
        eprintln!("lateral error: predicted {err} m, without the others {blind_err} m");
        assert!(err < 0.01, "predicted like the server: {err}");
        assert!(blind_err > 0.05, "without the others: {blind_err}");
    }

    /// The prediction's contacts are the server's only where the server
    /// would see them: none while a ghost after the join (2 s, then until
    /// clear), none with collisions off; a large overlap (a rider first
    /// seen in a snapshot) is pushed out a little per tick, never at once.
    #[test]
    fn predicted_rider_contacts_wait_for_the_ghost_and_never_jump() {
        let route = flat();
        let start = RiderState {
            distance: 100.0,
            speed: 8.0,
            ..Default::default()
        };
        let mass = RiderProfile::default().rider_mass_kg + RiderProfile::default().bike_mass_kg;
        let path = Frame::Path { loop_m: None };
        // Half a body width into the own rider, on its right.
        let beside = |s: f64, d: f64| vec![Body::on_path(RiderId(7), s, d, 0.0, 8.0, mass)];
        let mut p = Predictor::new(RiderProfile::default(), start);
        p.set_collision_mode(Mode::Full);
        p.set_lateral_target(0.0);
        for _ in 0..90 {
            p.set_others(path, beside(p.state().distance, 0.25));
            p.step(&route, 200.0);
            assert!(p.other_responses().is_empty() && !p.solid(), "a ghost");
        }
        assert!(p.state().lateral_offset.abs() < 1e-9);
        // Past the 2 s, but still overlapping: a ghost until clear.
        for _ in 0..30 {
            p.set_others(path, beside(p.state().distance, 0.25));
            p.step(&route, 200.0);
        }
        assert!(!p.solid());
        p.set_others(path, Vec::new());
        p.step(&route, 200.0);
        assert!(p.solid(), "clear: solid");
        // Now the overlap is pushed out, at most 2.5 cm a tick.
        let mut last = p.state().lateral_offset;
        let mut moved = 0.0;
        for _ in 0..20 {
            p.set_others(path, beside(p.state().distance, 0.25));
            p.step(&route, 200.0);
            let d = p.state().lateral_offset;
            assert!(
                (d - last).abs() <= PREDICTED_PUSH_MAX_M + 1e-9,
                "{}",
                d - last
            );
            moved += last - d;
            last = d;
        }
        assert!(moved > 0.1, "pushed to the left: {moved}");
        // Collisions off: nothing.
        let mut off = p.clone();
        off.set_collision_mode(Mode::Off);
        let before = off.state().lateral_offset;
        off.set_others(path, beside(off.state().distance, before + 0.2));
        off.step(&route, 200.0);
        assert!(off.other_responses().is_empty());
        assert!((off.state().lateral_offset - before).abs() < 0.03);
        // A rejoin makes it a ghost again.
        p.rejoin();
        assert!(!p.solid());
    }
}
