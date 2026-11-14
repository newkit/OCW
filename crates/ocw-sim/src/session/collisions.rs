//! Collisions in the session (spec §14 Phase 12, K1 / K2): the switches
//! (`ride.collisions`, `ride.chaotic_bots`, the event's rules), the
//! bodies handed to [`crate::collide`] after the physics step, the
//! response (pushes, speeds, wobbles) and the crash sequence of a rider
//! (Falling → Down → Remount → Riding), autodrive avoidance
//! ([`crate::avoid`]) for every autodrive rider and bot, chaotic bots'
//! mistakes, and the staged contacts of `dev.bump` / `dev.crash`.

use super::*;
use crate::avoid::{self, AvoidState};
use crate::collide::WorldCache;
use crate::collide::{self, Body, Bump, CrashPhase, CrashView, Frame, Hit, Mode, Response};
use ocw_world::obstacles::WorldObstacles;

/// A new (or reconnected) client rider rides as a ghost this long (s),
/// until its client has the snapshots (12A: a lag spike after joining
/// cannot cause a crash).
pub const JOIN_GHOST_S: f64 = 2.0;
// After it the rider stays a ghost until clear of everyone, so it never
// appears inside another rider.

/// Chaotic bots (Phase 12 Q3): every [`MISTAKE_WINDOW_S`] each bot makes a
/// mistake with this chance – a late reaction (no avoidance) drifting
/// [`MISTAKE_DRIFT_M`] towards the rider ahead – for [`MISTAKE_S`].
pub const MISTAKE_CHANCE: f64 = 0.08;
pub const MISTAKE_WINDOW_S: f64 = 20.0;
pub const MISTAKE_S: f64 = 3.0;
pub const MISTAKE_DRIFT_M: f64 = 0.45;

/// A rider's crash on the server (12C).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Crash {
    pub start_tick: u64,
    pub down_s: f64,
    pub hit: Hit,
    /// Speed and sideways slide when it happened (m/s).
    pub speed: f64,
    pub slide: f64,
}

impl Crash {
    pub fn elapsed_s(&self, tick: u64) -> f64 {
        tick.saturating_sub(self.start_tick) as f64 * SIM_DT
    }

    pub fn view(&self, tick: u64) -> Option<CrashView> {
        let elapsed_s = self.elapsed_s(tick);
        Some(CrashView {
            phase: collide::crash_phase(elapsed_s, self.down_s)?,
            elapsed_s,
            down_s: self.down_s,
            kind: self.hit.kind,
            by: Some(self.hit.other).filter(|o| o.0 != 0),
            impulse: self.hit.impulse,
            speed: self.speed,
        })
    }
}

/// Counters of the collision model (tests, `ocw-server` statistics).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CollisionStats {
    pub contacts: u64,
    pub bumps: u64,
    pub crashes: u64,
}

/// What `dev.bump` / `dev.crash` / `dev.tree` stage (12H, K3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Bump,
    Crash,
    /// A rider in wild mode put 8 m before the nearest tree off the road
    /// ahead, riding straight at it.
    Tree,
}

/// `dev.tree` puts the rider this far before the tree (m).
pub const STAGE_TREE_M: f64 = 8.0;

/// The collision switches of a session.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(super) struct Rules {
    /// `ride.collisions` (12A): the host's (on the local server the
    /// rider's) setting.
    pub on: bool,
    /// `ride.chaotic_bots` (Q3).
    pub chaotic_bots: bool,
    pub stats: CollisionStats,
}

impl Session {
    /// `ride.collisions` of the session (12A; off for a new session, the
    /// embedding switches it on).
    pub fn set_collisions(&mut self, on: bool) {
        self.rules.on = on;
    }

    pub fn collisions_setting(&self) -> bool {
        self.rules.on
    }

    /// `ride.chaotic_bots` (Phase 12 Q3).
    pub fn set_chaotic_bots(&mut self, on: bool) {
        self.rules.chaotic_bots = on;
    }

    pub fn chaotic_bots(&self) -> bool {
        self.rules.chaotic_bots
    }

    /// The collision model in force now (12A, 12G): the organiser's rule
    /// for the current event if one was set; else off when the session's
    /// switch is off; else by the event: group rides soft bumps only
    /// (Q2), races and free riding the full model.
    pub fn collision_mode(&self) -> Mode {
        let event = self
            .event
            .as_ref()
            .filter(|e| e.phase != event::Phase::Finished);
        if let Some(mode) = event.and_then(|e| e.collisions) {
            return mode;
        }
        if !self.rules.on {
            return Mode::Off;
        }
        match event.map(|e| e.kind) {
            Some(EventKind::GroupRide { .. }) => Mode::Soft,
            _ => Mode::Full,
        }
    }

    /// The organiser's collision rule for the current event (`None`: the
    /// default by its kind).
    pub fn set_event_collisions(&mut self, mode: Option<Mode>) -> bool {
        match &mut self.event {
            Some(e) => {
                e.collisions = mode;
                true
            }
            None => false,
        }
    }

    pub fn collision_stats(&self) -> CollisionStats {
        self.rules.stats
    }

    /// The world's static obstacles (spec §14 Phase 12 K3): set from the
    /// road network for free rides and route rides; a fixed route (no
    /// package) has none unless given here.
    pub fn set_obstacles(&mut self, world: Option<Arc<WorldObstacles>>) {
        self.world = WorldCache::new(world);
    }

    pub fn obstacles(&self) -> Option<&Arc<WorldObstacles>> {
        self.world.world.as_ref()
    }

    /// Tiles of the obstacle index in memory (tests, statistics).
    pub fn obstacle_tiles_loaded(&self) -> usize {
        self.world.loaded()
    }

    /// A rider's crash as the clients see it.
    pub fn crash(&self, id: RiderId) -> Option<CrashView> {
        self.riders.get(&id)?.crash?.view(self.tick)
    }

    /// Sets a rider's road state (tests, staged contacts).
    pub fn set_rider_state(&mut self, id: RiderId, state: RiderState) -> bool {
        match self.riders.get_mut(&id) {
            Some(r) => {
                r.state = state;
                true
            }
            None => false,
        }
    }

    /// Whether a rider takes part in collisions now (12A exemptions:
    /// god mode, the Echo rider, a new or reconnected client's first
    /// [`JOIN_GHOST_S`]; the countdown formation holds everyone).
    pub(super) fn collides(&self, r: &Rider) -> bool {
        if r.god.is_some() || r.control == Control::Echo {
            return false;
        }
        r.solid
    }

    /// A client rider past [`JOIN_GHOST_S`] who is still a ghost.
    fn joining(&self, r: &Rider) -> bool {
        !r.solid
            && r.god.is_none()
            && r.control == Control::Client
            && self.tick.saturating_sub(r.joined_tick) as f64 * SIM_DT >= JOIN_GHOST_S
    }

    /// The frame the bodies live in: one route's path frame, or the world.
    fn frame(&self) -> Frame {
        if self.free.is_some() {
            Frame::World
        } else {
            Frame::Path {
                loop_m: self.route.is_loop().then(|| self.route.length()),
            }
        }
    }

    /// Overtaking side (12D): from the region's traffic side.
    pub(super) fn overtake_left(&self) -> bool {
        avoid::overtake_left(self.free.as_ref().is_some_and(|(n, _)| n.keep_left))
    }

    /// The body of a rider in the frame.
    fn body(&self, id: RiderId, r: &Rider, frame: Frame) -> Body {
        let st = &r.state;
        let (pos, road) = match frame {
            Frame::Path { .. } => ([st.distance, st.lateral_offset], [1.0, 0.0]),
            Frame::World => {
                let p = self.track(id).sample(st.distance);
                let at = p.offset_position(st.lateral_offset);
                ([at[0], at[2]], p.direction)
            }
        };
        let mistaking = r.mistake_until > self.tick;
        Body {
            id,
            pos,
            road,
            heading: st.heading,
            speed: st.speed,
            lean: collide::lean_of(st.speed, st.turn_curvature),
            mass: r.effective.rider_mass_kg + r.effective.bike_mass_kg,
            fallen: r.crash.is_some(),
            manual: matches!(r.steering, Steering::Lean(_)) || r.wild || mistaking,
        }
    }

    /// Contacts after the physics step (12B: before the broadcast).
    pub(super) fn resolve_collisions(&mut self, mode: Mode) {
        if mode == Mode::Off {
            return;
        }
        let frame = self.frame();
        // Joining riders become solid once clear of everyone.
        let joining: Vec<RiderId> = self
            .riders
            .iter()
            .filter(|(_, r)| self.joining(r))
            .map(|(id, _)| *id)
            .collect();
        if !joining.is_empty() {
            let all: Vec<RiderId> = self
                .riders
                .iter()
                .filter(|(id, r)| self.collides(r) || joining.contains(id))
                .map(|(id, _)| *id)
                .collect();
            let bodies: Vec<Body> = all
                .iter()
                .map(|id| self.body(*id, &self.riders[id], frame))
                .collect();
            let touching: Vec<usize> = collide::detect(&bodies, frame)
                .iter()
                .flat_map(|c| [c.a, c.b])
                .collect();
            for (i, id) in all.iter().enumerate() {
                if joining.contains(id) && !touching.contains(&i) {
                    self.riders.get_mut(id).expect("listed").solid = true;
                }
            }
        }
        let ids: Vec<RiderId> = self
            .riders
            .iter()
            .filter(|(_, r)| self.collides(r))
            .map(|(id, _)| *id)
            .collect();
        if ids.len() >= 2 {
            let bodies: Vec<Body> = ids
                .iter()
                .map(|id| self.body(*id, &self.riders[id], frame))
                .collect();
            let out = collide::resolve(&bodies, frame, mode, self.tick);
            for (id, resp) in ids.iter().zip(&out) {
                if resp.touched() {
                    self.apply_response(*id, resp);
                }
            }
        }
        self.resolve_world(mode);
    }

    /// Applies a contact's response to rider `id`: the push (within the
    /// road edge, or wild mode's reach), speed, heading, a bump or a crash.
    pub(super) fn apply_response(&mut self, id: RiderId, resp: &Response) {
        let tick = self.tick;
        self.rules.stats.contacts += 1;
        let width = self
            .track(id)
            .road_width_at(body_distance(&self.riders[&id]));
        let r = self.riders.get_mut(&id).expect("listed");
        let limit = if r.wild {
            physics::WILD_LIMIT_M
        } else {
            physics::lateral_limit(width)
        };
        r.state.distance += resp.ds;
        r.state.lateral_offset = (r.state.lateral_offset + resp.dd).clamp(-limit, limit);
        if let Some(v) = resp.speed {
            r.state.speed = v;
        }
        if let Some(h) = resp.heading {
            r.state.heading = h;
        }
        if let Some((strength, side)) = resp.bump {
            r.bump = Some(Bump {
                tick,
                strength,
                side,
            });
            self.rules.stats.bumps += 1;
        }
        if let Some(hit) = resp.crash {
            start_crash(r, hit, tick);
            self.rules.stats.crashes += 1;
        }
    }

    /// Riders in wild mode against the world (K3): trees, buildings,
    /// walls, fences, barriers, bridge railings and tunnel walls. Riders
    /// on the road are held by its edge and never reach them (the routed
    /// road is never blocked, Q7).
    fn resolve_world(&mut self, mode: Mode) {
        if self.world.world.is_none() {
            return;
        }
        let tick = self.tick;
        let wild: Vec<RiderId> = self
            .riders
            .iter()
            .filter(|(_, r)| r.wild && r.crash.is_none() && self.collides(r))
            .map(|(id, _)| *id)
            .collect();
        let mut solids = Vec::new();
        for id in wild {
            let r = &self.riders[&id];
            let st = r.state;
            let mass = r.effective.rider_mass_kg + r.effective.bike_mass_kg;
            let p = self.track(id).sample(st.distance);
            let at = p.offset_position(st.lateral_offset);
            let body = Body {
                id,
                pos: [at[0], at[2]],
                road: p.direction,
                heading: st.heading,
                speed: st.speed,
                lean: collide::lean_of(st.speed, st.turn_curvature),
                mass,
                fallen: false,
                manual: true,
            };
            let resp = self.world.resolve(
                &body,
                at[1],
                tick,
                mode,
                physics::WILD_MAX_HEADING,
                &mut solids,
            );
            if resp.touched() {
                self.apply_response(id, &resp);
            }
        }
        if tick.is_multiple_of(250) {
            self.world.sweep(tick);
        }
    }

    /// The road edge hit hard (12B): rider `id` was stopped by the edge
    /// this tick while riding into it at `heading` with `speed` (before
    /// the tick).
    pub(super) fn edge_hit(&mut self, id: RiderId, speed: f64, heading: f64, mode: Mode) {
        let Some(r) = self.riders.get(&id) else {
            return;
        };
        let mass = r.effective.rider_mass_kg + r.effective.bike_mass_kg;
        if let Some(resp) = collide::edge_hit(speed, heading, mass, mode) {
            self.apply_response(id, &resp);
        }
    }

    /// Stages a contact for the local rider `id` (`dev.bump` / `dev.crash`,
    /// local server only): the nearest other rider is put beside (a bump
    /// from the side) or just ahead with its rear wheel overlapping the
    /// rider's front wheel and slower (a wheel touch: the rider falls). The
    /// other rider counts as making a mistake (it may cause a crash).
    pub fn stage(&mut self, id: RiderId, what: Stage) -> Result<String, String> {
        let mode = self.collision_mode();
        if what == Stage::Tree {
            return self.stage_tree(id, mode);
        }
        match (mode, what) {
            (Mode::Off, _) => return Err("collisions are off (set ride.collisions on)".into()),
            (Mode::Soft, Stage::Crash) => {
                return Err("only soft bumps in this event (group ride): no crashes".into());
            }
            _ => {}
        }
        let me = self.riders.get(&id).ok_or("no such rider")?;
        if me.crash.is_some() {
            return Err("already down".into());
        }
        let st = me.state;
        let my_path = me.path.clone();
        let width = self.track(id).road_width_at(st.distance);
        let limit = physics::lateral_limit(width);
        // The nearest other rider who can take part (bots first).
        let other = self
            .riders
            .iter()
            .filter(|(o, r)| **o != id && self.collides(r) && r.crash.is_none())
            .min_by(|(a, ra), (b, rb)| {
                let bot = |r: &Rider| !matches!(r.control, Control::Bot { .. });
                (bot(ra), (ra.state.distance - st.distance).abs(), a.0)
                    .partial_cmp(&(bot(rb), (rb.state.distance - st.distance).abs(), b.0))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(o, _)| *o)
            .ok_or("no other rider to bump into (add bots: --bots N)")?;
        // Room on the right or the left.
        let side = if st.lateral_offset + 0.5 <= limit {
            1.0
        } else {
            -1.0
        };
        let (distance, lateral, speed, heading) = match what {
            Stage::Bump => (
                st.distance,
                st.lateral_offset + side * 0.46,
                st.speed.max(4.0),
                -side * 0.12,
            ),
            Stage::Crash | Stage::Tree => (
                st.distance + 1.55,
                st.lateral_offset + side * 0.3,
                (st.speed - 2.5).max(0.0),
                0.0,
            ),
        };
        let tick = self.tick;
        let r = self.riders.get_mut(&other).expect("found");
        if let Some(p) = my_path {
            r.path = Some(p);
            r.path_reset = true;
            r.path_changed = None;
        }
        r.state = RiderState {
            distance,
            speed,
            lateral_offset: lateral,
            heading,
            turn_curvature: 0.0,
        };
        r.steering = Steering::Auto(lateral);
        r.avoid = AvoidState::default();
        r.mistake_until = tick + (MISTAKE_S / SIM_DT) as u64;
        r.mistake_drift = 0.0;
        let name = r.name.clone();
        if what == Stage::Crash && st.speed < 3.0 {
            // Too slow for a wheel touch to bite: give the rider some speed.
            let me = self.riders.get_mut(&id).expect("checked");
            me.state.speed = me.state.speed.max(5.0);
            if let Some(o) = self.riders.get_mut(&other) {
                o.state.speed = 2.5;
            }
        }
        Ok(match what {
            Stage::Bump => format!("{name} bumps into you from the side"),
            Stage::Crash | Stage::Tree => format!("{name} cuts in front of you: wheel touch"),
        })
    }
}

impl Session {
    /// `dev.tree` (K3, local server only): the rider (in wild mode) is put
    /// [`STAGE_TREE_M`] before the nearest tree off the road ahead (within
    /// 120 m along and 55 m beside), riding straight at it at least 7 m/s.
    fn stage_tree(&mut self, id: RiderId, mode: Mode) -> Result<String, String> {
        if mode == Mode::Off {
            return Err("collisions are off (set ride.collisions on)".into());
        }
        let me = self.riders.get(&id).ok_or("no such rider")?;
        if !me.wild {
            return Err("wild mode first (rider.wild on)".into());
        }
        if me.crash.is_some() {
            return Err("already down".into());
        }
        let st = me.state;
        let track = self.track(id);
        let p = track.sample(st.distance);
        let width = track.road_width_at(st.distance);
        let at = p.offset_position(0.0);
        let (lo, hi) = (
            [at[0] - 120.0, at[2] - 120.0],
            [at[0] + 120.0, at[2] + 120.0],
        );
        let mut solids = Vec::new();
        self.world.solids_near(lo, hi, self.tick, &mut solids);
        let track = self.track(id);
        // Each tree in the rider's frame (distance along, offset beside).
        let frame = |c: [f64; 2]| {
            let mut s =
                st.distance + (c[0] - at[0]) * p.direction[0] + (c[1] - at[2]) * p.direction[1];
            let mut d = 0.0;
            for _ in 0..3 {
                let q = track.sample(s);
                let r = q.right();
                d = (c[0] - q.position[0]) * r[0] + (c[1] - q.position[2]) * r[1];
                s += (c[0] - q.position[0]) * q.direction[0]
                    + (c[1] - q.position[2]) * q.direction[1];
            }
            (s, d)
        };
        let target = solids
            .iter()
            .filter(|s| s.kind == collide::ObstacleKind::Tree)
            .filter_map(|s| match s.shape {
                ocw_world::obstacles::Shape::Circle { c, .. } => Some(frame(c)),
                ocw_world::obstacles::Shape::Segment { .. } => None,
            })
            .filter(|(s, d)| {
                *s > st.distance + STAGE_TREE_M
                    && *s < st.distance + 120.0
                    && d.abs() > width / 2.0 + 2.0
                    && d.abs() < 55.0
            })
            .min_by(|a, b| (a.0 + a.1.abs()).total_cmp(&(b.0 + b.1.abs())))
            .ok_or("no tree off the road ahead")?;
        let r = self.riders.get_mut(&id).expect("checked");
        r.state = RiderState {
            distance: target.0 - STAGE_TREE_M,
            lateral_offset: target.1,
            heading: 0.0,
            speed: st.speed.max(7.0),
            turn_curvature: 0.0,
        };
        r.steering = Steering::Lean(0.0);
        r.avoid = AvoidState::default();
        Ok(format!(
            "riding at a tree {STAGE_TREE_M:.0} m ahead, {:.1} m beside the road",
            target.1
        ))
    }
}

fn body_distance(r: &Rider) -> f64 {
    r.state.distance
}

/// Starts a rider's crash (12C): sliding to a stop in the fall, power
/// without effect, autodrive's line reset to where the rider lies.
fn start_crash(r: &mut Rider, hit: Hit, tick: u64) {
    r.crash = Some(Crash {
        start_tick: tick,
        down_s: collide::down_s(hit.energy_j),
        hit,
        speed: r.state.speed,
        slide: (hit.impulse[1] * 0.5).clamp(-2.0, 2.0),
    });
    r.bump = None;
    r.avoid = AvoidState::default();
    r.mistake_until = 0;
    if let Steering::Auto(_) = r.steering {
        r.steering = Steering::Auto(r.state.lateral_offset);
    }
}

/// One tick of a crashed rider (12C): sliding to a stop while falling,
/// lying still while down and remounting, then riding again from
/// standstill. Returns whether the rider is still crashed.
pub(super) fn crash_tick(r: &mut Rider, tick: u64, limit: f64) -> bool {
    let Some(c) = r.crash else {
        return false;
    };
    let elapsed = c.elapsed_s(tick);
    match collide::crash_phase(elapsed, c.down_s) {
        Some(CrashPhase::Falling) => {
            let fade = (1.0 - elapsed / collide::FALL_S).max(0.0);
            r.state.speed = c.speed * fade;
            r.state.distance += r.state.speed * SIM_DT;
            r.state.lateral_offset =
                (r.state.lateral_offset + c.slide * fade * SIM_DT).clamp(-limit, limit);
            r.state.heading *= 0.9;
            r.state.turn_curvature = 0.0;
            true
        }
        Some(CrashPhase::Down | CrashPhase::Remount) => {
            r.state.speed = 0.0;
            r.state.heading = 0.0;
            r.state.turn_curvature = 0.0;
            true
        }
        None => {
            r.crash = None;
            r.state.speed = 0.0;
            if let Steering::Auto(_) = r.steering {
                r.steering = Steering::Auto(r.state.lateral_offset);
            }
            false
        }
    }
}

/// Chaotic bots (Q3): whether bot `id` starts a mistake at `tick` (one
/// deterministic draw per window).
pub(super) fn mistake_starts(id: RiderId, tick: u64) -> bool {
    let window = (MISTAKE_WINDOW_S / SIM_DT) as u64;
    if tick == 0 || !tick.is_multiple_of(window) {
        return false;
    }
    let draw = splitmix((tick / window) << 20 ^ u64::from(id.0) ^ 0x0C4A_0715);
    (draw % 10_000) as f64 / 10_000.0 < MISTAKE_CHANCE
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collide::ContactKind;

    /// A straight 2 km route, `width_m` wide, that ends.
    fn straight(width_m: f64) -> Route {
        let pts = (0..=200).map(|i| [f64::from(i) * 10.0, 0.0, 0.0]).collect();
        Route::from_points("straight", "Straight", pts, false, width_m).unwrap()
    }

    fn input(watts: f64) -> RiderInput {
        RiderInput {
            power_w: watts,
            ..Default::default()
        }
    }

    /// A session with collisions on.
    fn on(route: Route) -> Session {
        let mut s = Session::new(route);
        s.set_collisions(true);
        s
    }

    /// Past the joining ghost time.
    fn settle(s: &mut Session) {
        for _ in 0..=(JOIN_GHOST_S / SIM_DT) as usize {
            s.step();
        }
    }

    fn put(s: &mut Session, id: RiderId, distance: f64, lateral: f64, speed: f64) {
        let r = s.riders.get_mut(&id).unwrap();
        r.state = RiderState {
            distance,
            lateral_offset: lateral,
            speed,
            ..Default::default()
        };
        if let Steering::Auto(_) = r.steering {
            r.steering = Steering::Auto(lateral);
        }
        r.avoid = AvoidState::default();
    }

    fn manual(s: &mut Session, id: RiderId) {
        let seq = s.riders[&id].last_steer_seq + 1;
        s.set_lean(id, seq, 0.0);
    }

    #[test]
    fn collisions_off_riders_pass_through() {
        let mut s = Session::new(straight(6.0));
        let a = s.add_rider("a", RiderProfile::default());
        let b = s.add_rider("b", RiderProfile::default());
        settle(&mut s);
        put(&mut s, a, 100.0, 0.0, 10.0);
        put(&mut s, b, 100.5, 0.0, 6.0);
        s.step();
        assert_eq!(s.collision_mode(), Mode::Off);
        assert_eq!(s.collision_stats(), CollisionStats::default());
        assert!(s.crash(a).is_none());
    }

    #[test]
    fn a_side_bump_pushes_apart_and_wobbles() {
        let mut s = on(straight(6.0));
        let a = s.add_rider("a", RiderProfile::default());
        let b = s.add_rider("b", RiderProfile::default());
        settle(&mut s);
        manual(&mut s, b);
        put(&mut s, a, 100.0, 0.0, 8.0);
        put(&mut s, b, 100.0, 0.47, 8.0);
        s.riders.get_mut(&b).unwrap().state.heading = -0.1;
        s.step();
        let st = s.collision_stats();
        assert!(st.bumps >= 2 && st.crashes == 0, "{st:?}");
        let (sa, sb) = (s.rider_state(a).unwrap(), s.rider_state(b).unwrap());
        assert!(
            sb.lateral_offset - sa.lateral_offset >= crate::collide::BODY_WIDTH_M - 0.01,
            "{sa:?} {sb:?}"
        );
        let snap = s.snapshot();
        assert!(
            snap.iter()
                .all(|r| r.bump.is_some_and(|b| b.tick == s.tick()))
        );
    }

    #[test]
    fn the_crash_sequence_and_time_down_by_energy() {
        let crash = |closing: f64| {
            let mut s = on(straight(6.0));
            let a = s.add_rider("a", RiderProfile::default());
            let b = s.add_rider("b", RiderProfile::default());
            settle(&mut s);
            manual(&mut s, a);
            s.apply_input(a, 1, input(400.0));
            put(&mut s, a, 100.0, 0.0, 6.0 + closing);
            put(&mut s, b, 101.7, 0.0, 6.0);
            s.step();
            (s, a, b)
        };
        let (mut s, a, b) = crash(3.0);
        let c = s.crash(a).expect("the rider behind went down");
        assert_eq!(c.phase, CrashPhase::Falling);
        assert_eq!(c.kind, ContactKind::RearEnd);
        assert_eq!(c.by, Some(b));
        assert!(s.crash(b).is_none(), "the rider in front stays up");
        let down = c.down_s;
        assert!((collide::DOWN_MIN_S..=collide::DOWN_MAX_S).contains(&down));
        // A harder hit keeps the rider down longer.
        let (s2, a2, _) = crash(6.0);
        assert!(s2.crash(a2).unwrap().down_s > down);
        // Falling: sliding to a stop within the fall.
        let mut speeds = vec![s.rider_state(a).unwrap().speed];
        while s.crash(a).is_some_and(|c| c.phase == CrashPhase::Falling) {
            s.step();
            speeds.push(s.rider_state(a).unwrap().speed);
        }
        assert!(speeds.windows(2).all(|w| w[1] <= w[0] + 1e-9), "{speeds:?}");
        assert_eq!(s.rider_state(a).unwrap().speed, 0.0);
        // Down: 400 W have no effect.
        let at = s.rider_state(a).unwrap().distance;
        assert_eq!(s.crash(a).unwrap().phase, CrashPhase::Down);
        for _ in 0..((down / SIM_DT) as usize - 2) {
            s.step();
        }
        assert_eq!(s.crash(a).unwrap().phase, CrashPhase::Down);
        assert_eq!(s.rider_state(a).unwrap().distance, at);
        for _ in 0..4 {
            s.step();
        }
        assert_eq!(s.crash(a).unwrap().phase, CrashPhase::Remount);
        let mut remount = 0;
        while s.crash(a).is_some() {
            s.step();
            remount += 1;
        }
        assert!(
            remount <= (collide::REMOUNT_S / SIM_DT) as usize + 1,
            "{remount}"
        );
        // Riding again, from standstill with the own power.
        let v0 = s.rider_state(a).unwrap().speed;
        assert!(v0 < 0.3, "{v0}");
        for _ in 0..50 {
            s.step();
        }
        assert!(s.rider_state(a).unwrap().speed > v0 + 1.0);
        assert_eq!(s.collision_stats().crashes, 1);
    }

    #[test]
    fn a_fallen_bike_brings_down_the_next_rider() {
        let mut s = on(straight(6.0));
        let a = s.add_rider("a", RiderProfile::default());
        let b = s.add_rider("b", RiderProfile::default());
        let c = s.add_rider("c", RiderProfile::default());
        settle(&mut s);
        for id in [a, c] {
            manual(&mut s, id);
        }
        s.apply_input(c, 1, input(300.0));
        put(&mut s, a, 100.0, 0.0, 9.5);
        put(&mut s, b, 101.7, 0.0, 6.0);
        put(&mut s, c, 70.0, 0.0, 9.0);
        s.step();
        assert!(s.crash(a).is_some());
        // C rides straight on into A lying on the road (no avoidance:
        // steered by hand) and goes down too: a chain crash.
        for _ in 0..200 {
            s.step();
        }
        let cc = s.crash(c).expect("chain crash");
        assert_eq!(cc.kind, ContactKind::Object);
        assert_eq!(cc.by, Some(a));
    }

    #[test]
    fn exempt_riders_never_collide() {
        let mut s = on(straight(6.0));
        let a = s.add_rider("a", RiderProfile::default());
        let b = s.add_rider("b", RiderProfile::default());
        // Joining clients ride as ghosts for their first seconds.
        manual(&mut s, a);
        put(&mut s, a, 100.0, 0.0, 10.0);
        put(&mut s, b, 101.0, 0.0, 5.0);
        s.step();
        assert_eq!(s.collision_stats().contacts, 0, "ghosts while joining");
        settle(&mut s);
        // God mode: out of the physics.
        put(&mut s, a, 300.0, 0.0, 10.0);
        put(&mut s, b, 300.5, 0.0, 5.0);
        s.enter_god(b).unwrap();
        s.step();
        assert_eq!(s.collision_stats().contacts, 0, "unicorns fly");
        // The Echo rider stands apart, out of the physics.
        let mut s = on(straight(6.0));
        let a = s.add_rider("a", RiderProfile::default());
        settle(&mut s);
        let echo = s.place_echo("Echo", Some(a));
        let at = s.rider_state(echo).unwrap();
        manual(&mut s, a);
        put(&mut s, a, at.distance - 1.0, at.lateral_offset, 8.0);
        s.step();
        assert_eq!(s.collision_stats().contacts, 0, "the Echo rider");
    }

    #[test]
    fn the_countdown_formation_waits_without_collisions() {
        let mut s = on(Route::test_loop());
        for i in 0..6 {
            s.add_bot(format!("b{i}"), 200.0);
        }
        let a = s.add_rider("a", RiderProfile::default());
        settle(&mut s);
        s.start_event(EventSpec::Race {
            laps: Some(1),
            length_m: None,
        })
        .unwrap();
        // Overlapping the next rider in the formation does nothing while
        // the countdown holds the field.
        let next = s.riders.keys().copied().find(|id| *id != a).unwrap();
        let at = s.rider_state(next).unwrap();
        put(&mut s, a, at.distance - 1.0, at.lateral_offset, 0.0);
        for _ in 0..20 {
            s.step();
        }
        assert!(s.event().unwrap().holds());
        assert_eq!(s.collision_stats().contacts, 0);
        assert_eq!(s.collision_mode(), Mode::Full, "races: the full model");
    }

    #[test]
    fn group_rides_are_soft_and_organisers_decide() {
        let mut s = on(Route::test_loop());
        s.add_bot("b", 200.0);
        s.start_event(EventSpec::GroupRide { target_w: None })
            .unwrap();
        assert_eq!(s.collision_mode(), Mode::Soft);
        s.set_event_collisions(Some(Mode::Full));
        assert_eq!(s.collision_mode(), Mode::Full);
        s.set_collisions(false);
        assert_eq!(s.collision_mode(), Mode::Full, "the organiser's rule wins");
        s.set_event_collisions(None);
        assert_eq!(s.collision_mode(), Mode::Off);
        s.cancel_event();
        s.set_collisions(true);
        assert_eq!(s.collision_mode(), Mode::Full);
    }

    #[test]
    fn a_manual_rider_steered_into_another_crashes() {
        // Rear-ending a bot at 3.5 m/s more brings the manual rider down.
        let mut s = on(straight(6.0));
        let a = s.add_rider("a", RiderProfile::default());
        let bot = s.add_bot("bot", 100.0);
        settle(&mut s);
        manual(&mut s, a);
        s.apply_input(a, 1, input(500.0));
        put(&mut s, bot, 200.0, 0.0, 6.0);
        put(&mut s, a, 196.0, 0.0, 9.5);
        for _ in 0..50 {
            s.step();
        }
        let c = s.crash(a).expect("down");
        assert_eq!(c.kind, ContactKind::RearEnd);
        assert!(s.crash(bot).is_none());
    }

    /// Bots at mixed powers, `n` of them, on the test loop.
    fn field(n: u32) -> Session {
        let mut s = on(Route::test_loop());
        for i in 0..n {
            s.add_bot(format!("b{i}"), 110.0 + f64::from(i * 37 % 230));
        }
        s
    }

    #[test]
    fn fifty_autodrive_bots_ride_thirty_minutes_without_a_contact() {
        let mut s = field(50);
        let start: Vec<f64> = s.riders.values().map(|r| r.state.distance).collect();
        let mut passes = 0u32;
        let mut order: Vec<RiderId> = Vec::new();
        for tick in 0..(30.0 * 60.0 / SIM_DT) as u64 {
            s.step();
            assert_eq!(
                s.collision_stats().contacts,
                0,
                "a contact after {:.1} s",
                tick as f64 * SIM_DT
            );
            if tick % 500 == 0 {
                let mut now: Vec<(f64, RiderId)> = s
                    .riders
                    .iter()
                    .map(|(id, r)| (r.state.distance, *id))
                    .collect();
                now.sort_by(|a, b| a.0.total_cmp(&b.0));
                let ids: Vec<RiderId> = now.iter().map(|x| x.1).collect();
                if !order.is_empty() {
                    passes += ids.iter().zip(&order).filter(|(a, b)| a != b).count() as u32;
                }
                order = ids;
            }
        }
        let end: Vec<f64> = s.riders.values().map(|r| r.state.distance).collect();
        assert!(end.iter().zip(&start).all(|(e, s)| e - s > 5_000.0));
        assert!(passes > 50, "the field mixed: {passes}");
    }

    /// A fast bot 20 m behind a slow one in the same line: the widest
    /// lateral offset of the fast one while it passes.
    fn pass(s: &mut Session) -> f64 {
        let ids: Vec<RiderId> = s.riders.keys().copied().collect();
        let (fast, slow) = (ids[0], ids[1]);
        let d = s.rider_state(fast).unwrap().distance.max(30.0);
        put(s, slow, d + 20.0, 0.0, 4.0);
        put(s, fast, d, 0.0, 9.0);
        let mut widest: f64 = 0.0;
        for _ in 0..(20.0 / SIM_DT) as usize {
            s.step();
            let l = s.rider_state(fast).unwrap().lateral_offset;
            if l.abs() > widest.abs() {
                widest = l;
            }
        }
        assert_eq!(s.collision_stats().contacts, 0);
        assert!(
            s.rider_state(fast).unwrap().distance > s.rider_state(slow).unwrap().distance,
            "passed"
        );
        // Back in the own line afterwards.
        assert!(
            s.rider_state(fast).unwrap().lateral_offset.abs() < 0.2,
            "{:?}",
            s.rider_state(fast)
        );
        widest
    }

    #[test]
    fn autodrive_overtakes_on_the_regions_side() {
        // A route (right-hand traffic): on the left.
        let mut s = on(straight(7.0));
        s.add_bot("fast", 400.0);
        s.add_bot("slow", 60.0);
        let w = pass(&mut s);
        assert!(w < -0.6, "{w}");
        // A left-hand traffic region: on the right.
        let mut net = ocw_world::RoadNetwork::new(crate::freeride::tests::crossroads());
        net.keep_left = true;
        let mut s = Session::free_ride(
            Arc::new(net),
            FreeRideStart {
                edge: 1,
                offset_m: 10.0,
                forward: true,
            },
        );
        s.set_collisions(true);
        s.add_bot("fast", 400.0);
        s.add_bot("slow", 60.0);
        let w = pass(&mut s);
        assert!(w > 0.6, "{w}");
    }

    #[test]
    fn a_blocked_autodrive_rider_holds_the_wheel_gap() {
        // 2.5 m wide: no room to pass.
        let mut s = on(straight(2.5));
        let fast = s.add_bot("fast", 400.0);
        let slow = s.add_bot("slow", 80.0);
        put(&mut s, slow, 300.0, 0.0, 5.0);
        put(&mut s, fast, 280.0, 0.0, 9.0);
        for _ in 0..(30.0 / SIM_DT) as usize {
            s.step();
        }
        let (f, sl) = (s.rider_state(fast).unwrap(), s.rider_state(slow).unwrap());
        let gap = sl.distance - f.distance - crate::collide::BODY_LENGTH_M;
        assert!((gap - avoid::HOLD_GAP_M).abs() < 0.2, "{gap}");
        assert!((f.speed - sl.speed).abs() < 0.3, "{} {}", f.speed, sl.speed);
        assert_eq!(s.collision_stats().contacts, 0);
    }

    #[test]
    fn staged_bump_and_crash() {
        let mut s = on(Route::test_loop());
        for i in 0..4 {
            s.add_bot(format!("b{i}"), 200.0);
        }
        let me = s.add_rider("me", RiderProfile::default());
        s.apply_input(me, 1, input(250.0));
        for _ in 0..(10.0 / SIM_DT) as usize {
            s.step();
        }
        let said = s.stage(me, Stage::Bump).unwrap();
        assert!(said.contains("bumps"), "{said}");
        for _ in 0..5 {
            s.step();
        }
        assert!(s.collision_stats().bumps > 0, "{:?}", s.collision_stats());
        assert!(s.crash(me).is_none());
        for _ in 0..(5.0 / SIM_DT) as usize {
            s.step();
        }
        s.stage(me, Stage::Crash).unwrap();
        for _ in 0..5 {
            s.step();
        }
        let c = s.crash(me).expect("down after the staged wheel touch");
        assert_eq!(c.kind, ContactKind::WheelTouch);
        // Collisions off: nothing to stage.
        s.set_collisions(false);
        assert!(s.stage(me, Stage::Bump).is_err());
    }

    #[test]
    fn chaotic_bots_make_mistakes() {
        let contacts = |chaotic: bool| {
            let mut s = field(30);
            s.set_chaotic_bots(chaotic);
            for _ in 0..(10.0 * 60.0 / SIM_DT) as usize {
                s.step();
            }
            s.collision_stats()
        };
        assert_eq!(contacts(false).contacts, 0);
        let chaos = contacts(true);
        assert!(chaos.contacts > 0, "{chaos:?}");
        // The same every time (no random numbers).
        assert_eq!(contacts(true), chaos);
    }
}

/// Riders against the world (spec §14 Phase 12 K3).
#[cfg(test)]
mod world_tests {
    use super::*;
    use crate::collide::{ContactKind, ObstacleKind};
    use ocw_world::obstacles::{
        LineObstacle, ON_ROAD, Obstacles, PointObstacle, ROAD_BARRIER, TileObstacles,
    };
    use ocw_world::{TileKey, WorldPackage};

    /// A straight 2 km route along +x at z = 0, 6 m wide (lateral = +z).
    fn straight() -> Route {
        let pts = (0..=200).map(|i| [f64::from(i) * 10.0, 0.0, 0.0]).collect();
        Route::from_points("straight", "Straight", pts, false, 6.0).unwrap()
    }

    fn empty_package() -> WorldPackage {
        WorldPackage {
            name: "World".into(),
            origin: (47.0, 9.0),
            tile_size_m: 1000.0,
            samples: 2,
            tiles: BTreeMap::new(),
            graph: None,
            landuse: None,
            buildings: None,
            bridges: None,
            routes: Vec::new(),
            region: None,
            time_zone: None,
            look: Default::default(),
            obstacles: None,
        }
    }

    fn point(kind: ObstacleKind, x: f32, z: f32, flags: u8) -> PointObstacle {
        PointObstacle {
            kind,
            flags,
            pos: [x, z],
            angle: 0.0,
            width_m: 0.0,
            height_m: 1.0,
        }
    }

    /// A session on the straight route with these obstacles in tile (0, 0).
    fn world(tile: TileObstacles, edit: impl FnOnce(&mut WorldPackage)) -> Session {
        let mut pkg = empty_package();
        let mut o = Obstacles::new(1000.0);
        o.tiles.insert(TileKey { x: 0, z: 0 }, tile);
        pkg.obstacles = Some(o);
        edit(&mut pkg);
        let mut s = Session::new(straight());
        s.set_collisions(true);
        s.set_obstacles(WorldObstacles::from_package(&pkg).map(Arc::new));
        s
    }

    /// A client rider in wild mode at (distance, lateral), riding at
    /// `heading` with `speed`, past its joining time.
    fn wild_rider(
        s: &mut Session,
        distance: f64,
        lateral: f64,
        heading: f64,
        speed: f64,
    ) -> RiderId {
        let id = s.add_rider("wild", RiderProfile::default());
        let r = s.riders.get_mut(&id).unwrap();
        r.solid = true;
        r.wild = true;
        r.steering = Steering::Lean(0.0);
        r.state = RiderState {
            distance,
            lateral_offset: lateral,
            heading,
            speed,
            turn_curvature: 0.0,
        };
        id
    }

    fn ride(s: &mut Session, seconds: f64) {
        for _ in 0..(seconds / SIM_DT) as usize {
            s.step();
        }
    }

    /// Rides into whatever stands 6 m ahead at 8 m/s: the crash's kind.
    fn crash_into(s: &mut Session, at: [f64; 2]) -> Option<ContactKind> {
        let id = wild_rider(s, at[0] - 6.0, at[1], 0.0, 8.0);
        ride(s, 1.5);
        s.crash(id).map(|c| c.kind)
    }

    fn wall(kind: ObstacleKind, a: [f32; 2], b: [f32; 2]) -> LineObstacle {
        LineObstacle {
            kind,
            flags: 0,
            height_m: 1.5,
            points: vec![a, b],
        }
    }

    #[test]
    fn wild_riders_crash_into_every_kind_of_world_object() {
        // Points: blocks, posts, bollards and gates off the road, pylons.
        for kind in [
            ObstacleKind::Block,
            ObstacleKind::Post,
            ObstacleKind::Bollard,
            ObstacleKind::Gate,
            ObstacleKind::LiftGate,
            ObstacleKind::CycleBarrier,
            ObstacleKind::Barrier,
            ObstacleKind::Pylon,
        ] {
            let mut s = world(
                TileObstacles {
                    points: vec![point(kind, 300.0, 20.0, 0)],
                    ..Default::default()
                },
                |_| {},
            );
            assert_eq!(
                crash_into(&mut s, [300.0, 20.0]),
                Some(ContactKind::World(kind)),
                "{kind:?}"
            );
        }
        // Lines across the rider's way.
        for kind in [
            ObstacleKind::Wall,
            ObstacleKind::Fence,
            ObstacleKind::Hedge,
            ObstacleKind::GuardRail,
            ObstacleKind::GateLine,
        ] {
            let mut s = world(
                TileObstacles {
                    lines: vec![wall(kind, [300.0, 10.0], [300.0, 30.0])],
                    ..Default::default()
                },
                |_| {},
            );
            assert_eq!(
                crash_into(&mut s, [300.0, 20.0]),
                Some(ContactKind::World(kind)),
                "{kind:?}"
            );
        }
        // Kerbs are ridden over.
        let mut s = world(
            TileObstacles {
                lines: vec![wall(ObstacleKind::Kerb, [300.0, 10.0], [300.0, 30.0])],
                ..Default::default()
            },
            |_| {},
        );
        assert_eq!(crash_into(&mut s, [300.0, 20.0]), None);
        // A tree of the package's tree placement (section OBST).
        let mut cells = vec![0u8; 125 * 125];
        let (i, k) = (37usize, 2usize);
        cells[k * 125 + i] = ocw_world::landuse::LandClass::Forest.code() + 1;
        let mut tile = TileObstacles::default();
        tile.set_tree_cells(&cells);
        let (trunk, r) =
            ocw_world::trees::trunks_from_cells(TileKey { x: 0, z: 0 }, 1000.0, &cells)[0];
        assert!(r > 0.15 && trunk[1] > 15.0 && trunk[1] < 25.0, "{trunk:?}");
        let mut s = world(tile, |_| {});
        assert_eq!(
            crash_into(&mut s, trunk),
            Some(ContactKind::World(ObstacleKind::Tree))
        );
        // A building (section BLDG).
        let mut s = world(TileObstacles::default(), |p| {
            p.buildings = Some(BTreeMap::from([(
                TileKey { x: 0, z: 0 },
                vec![ocw_world::buildings::Building {
                    kind: ocw_world::buildings::BuildingKind::House,
                    outline: vec![[300.0, 15.0], [310.0, 15.0], [310.0, 25.0], [300.0, 25.0]],
                    holes: Vec::new(),
                    base_y: 0.0,
                    eave_y: 6.0,
                    roof: ocw_world::buildings::Roof::Flat,
                    tags: Default::default(),
                }],
            )]))
        });
        assert_eq!(
            crash_into(&mut s, [300.0, 20.0]),
            Some(ContactKind::World(ObstacleKind::Building))
        );
    }

    #[test]
    fn a_slow_hit_stops_the_rider_without_a_crash_or_overlap() {
        let mut s = world(
            TileObstacles {
                lines: vec![wall(ObstacleKind::Wall, [300.0, 10.0], [300.0, 30.0])],
                ..Default::default()
            },
            |_| {},
        );
        let id = wild_rider(&mut s, 297.0, 20.0, 0.0, 2.0);
        ride(&mut s, 2.0);
        assert!(s.crash(id).is_none());
        let st = s.rider_state(id).unwrap();
        // Stopped at the wall (0.2 m thick), the front of the bike at it.
        assert!(st.speed < 0.5, "{st:?}");
        let front = st.distance + collide::BODY_LENGTH_M / 2.0;
        assert!(front <= 300.0 - 0.2 + 0.01 && front > 299.0, "{front}");
        assert!(s.riders[&id].bump.is_some());
    }

    #[test]
    fn glancing_a_wall_slides_along_it() {
        // A wall along the road 20 m beside it; the rider steers into it at
        // 10°: the speed into it (1.4 m/s) is lost, the rider rides on.
        let mut s = world(
            TileObstacles {
                lines: vec![wall(ObstacleKind::Wall, [100.0, 20.0], [900.0, 20.0])],
                ..Default::default()
            },
            |_| {},
        );
        let id = wild_rider(&mut s, 300.0, 18.5, 0.17, 8.0);
        ride(&mut s, 1.0);
        assert!(s.crash(id).is_none());
        let st = s.rider_state(id).unwrap();
        assert!(st.speed > 6.0, "rides on: {st:?}");
        assert!(st.lateral_offset < 20.0 - 0.2, "{st:?}");
        // Steeply (35° at 8 m/s, 4.6 m/s into it): a crash.
        let id = wild_rider(&mut s, 600.0, 18.5, 0.61, 8.0);
        ride(&mut s, 0.5);
        assert_eq!(
            s.crash(id).map(|c| c.kind),
            Some(ContactKind::World(ObstacleKind::Wall))
        );
        // Down where it fell, beside the wall (not back on the road).
        ride(&mut s, 2.0);
        assert!(s.rider_state(id).unwrap().lateral_offset > 17.0);
    }

    #[test]
    fn road_barriers_never_block_and_riders_on_the_road_meet_nothing() {
        // A bollard and a gate on the road (road barriers, Q7) and a block
        // just beyond its edge: a rider on the road rides through, a rider
        // hugging the edge passes the block, a wild rider on the road rides
        // through the barriers too.
        let mut s = world(
            TileObstacles {
                points: vec![
                    point(ObstacleKind::Bollard, 300.0, 0.0, ROAD_BARRIER | ON_ROAD),
                    point(ObstacleKind::Gate, 400.0, 0.0, ROAD_BARRIER | ON_ROAD),
                    point(ObstacleKind::Block, 500.0, 3.6, 0),
                ],
                ..Default::default()
            },
            |_| {},
        );
        let limit = physics::lateral_limit(6.0);
        let road = wild_rider(&mut s, 290.0, 0.0, 0.0, 10.0);
        let edge = wild_rider(&mut s, 480.0, limit, 0.0, 10.0);
        for id in [road, edge] {
            let r = s.riders.get_mut(&id).unwrap();
            r.wild = false;
            r.steering = Steering::Auto(r.state.lateral_offset);
        }
        let through = wild_rider(&mut s, 280.0, 0.0, 0.0, 10.0);
        ride(&mut s, 3.0);
        for id in [road, edge, through] {
            assert!(s.crash(id).is_none(), "{id:?}");
            assert!(s.rider_state(id).unwrap().speed > 5.0, "{id:?}");
        }
        assert_eq!(s.collision_stats().contacts, 0);
    }

    #[test]
    fn dev_tree_stages_a_wild_crash_into_a_tree() {
        let mut cells = vec![0u8; 125 * 125];
        cells[3 * 125 + 40] = ocw_world::landuse::LandClass::Forest.code() + 1;
        let mut tile = TileObstacles::default();
        tile.set_tree_cells(&cells);
        let mut s = world(tile, |_| {});
        let id = wild_rider(&mut s, 250.0, 0.0, 0.0, 5.0);
        let road = s.add_rider("road", RiderProfile::default());
        assert!(s.stage(road, Stage::Tree).is_err(), "wild mode first");
        let msg = s.stage(id, Stage::Tree).unwrap();
        assert!(msg.contains("tree"), "{msg}");
        ride(&mut s, 2.0);
        assert_eq!(
            s.crash(id).map(|c| c.kind),
            Some(ContactKind::World(ObstacleKind::Tree))
        );
    }

    #[test]
    fn collisions_off_rides_through_the_world() {
        let mut s = world(
            TileObstacles {
                points: vec![point(ObstacleKind::Block, 300.0, 20.0, 0)],
                ..Default::default()
            },
            |_| {},
        );
        s.set_collisions(false);
        assert_eq!(crash_into(&mut s, [300.0, 20.0]), None);
        assert_eq!(s.collision_stats().contacts, 0);
    }

    #[test]
    fn railings_and_tunnel_walls_bound_riders_on_their_road_only() {
        use ocw_world::bridges::{Bridge, BridgeMaterial, BridgeSource, BridgeStructure, Railing};
        use ocw_world::graph::{Edge, EdgeId, Oneway, RoadClass, RoadGraph, VertexId};
        let deck = |y: f32| Bridge {
            edge: EdgeId(1),
            osm_way: 1,
            structure: BridgeStructure::Beam,
            material: BridgeMaterial::Concrete,
            source: BridgeSource::Tagged,
            railing: Railing::Parapet,
            width_m: 6.0,
            deck_m: 1.0,
            points: vec![[200.0, y, 0.0], [400.0, y, 0.0]],
            abutment_foot_y: [y - 8.0; 2],
            piers: Vec::new(),
            arches: Vec::new(),
        };
        let tunnel = |y: f64| RoadGraph {
            vertices: vec![[600.0, 0.0], [800.0, 0.0]],
            edges: vec![Edge {
                id: EdgeId(7),
                from: VertexId(0),
                to: VertexId(1),
                points: vec![[600.0, 0.0], [800.0, 0.0]],
                class: RoadClass::Tertiary,
                name: None,
                width_m: 6.0,
                bridge: false,
                tunnel: true,
                dead_end_m: None,
                boundary: false,
                oneway: Oneway::No,
                osm_way: 7,
                profile: vec![y; 41],
            }],
            boundary_vertices: Default::default(),
        };
        let at = |y: f32, ty: f64| {
            world(TileObstacles::default(), |p| {
                p.bridges = Some(BTreeMap::from([(TileKey { x: 0, z: 0 }, vec![deck(y)])]));
                p.graph = Some(tunnel(ty));
            })
        };
        // On the bridge (its deck at the road's height): wild mode cannot
        // leave it sideways; steeply into the parapet is a crash.
        let mut s = at(0.0, 0.0);
        let id = wild_rider(&mut s, 300.0, 0.0, 0.9, 8.0);
        ride(&mut s, 1.0);
        assert_eq!(
            s.crash(id).map(|c| c.kind),
            Some(ContactKind::World(ObstacleKind::Railing))
        );
        assert!(s.rider_state(id).unwrap().lateral_offset < 3.6 - 0.175);
        // Gently: bounded, riding on along it.
        let id = wild_rider(&mut s, 250.0, 2.0, 0.15, 6.0);
        ride(&mut s, 2.0);
        let st = s.rider_state(id).unwrap();
        assert!(
            s.crash(id).is_none() && st.lateral_offset < 3.5 && st.speed > 4.0,
            "{st:?}"
        );
        // In the tunnel: the walls 1 m beside the road edge.
        let id = wild_rider(&mut s, 700.0, 0.0, -0.9, 8.0);
        ride(&mut s, 1.0);
        assert_eq!(
            s.crash(id).map(|c| c.kind),
            Some(ContactKind::World(ObstacleKind::TunnelWall))
        );
        assert!(s.rider_state(id).unwrap().lateral_offset > -4.0);
        // A road 8 m under the bridge and 20 m under the tunnel's road
        // (another level): neither bounds its riders.
        let mut s = at(8.0, 20.0);
        for d in [300.0, 700.0] {
            let id = wild_rider(&mut s, d, 0.0, 0.9, 8.0);
            ride(&mut s, 1.0);
            assert!(s.crash(id).is_none());
            assert!(s.rider_state(id).unwrap().lateral_offset > 5.0);
        }
    }

    #[test]
    fn hitting_the_road_edge_hard_is_a_wall_hit() {
        // No world objects needed: the road edge of §5.2.1.
        let mut s = Session::new(straight());
        s.set_collisions(true);
        // Wild mode just switched off while riding across the road at 50°:
        // stopped by the edge at 10 m/s × sin 50° = 7.7 m/s into it.
        let id = wild_rider(&mut s, 300.0, 2.39, 0.87, 10.0);
        s.riders.get_mut(&id).unwrap().wild = false;
        s.step();
        let c = s.crash(id).expect("a crash into the road edge");
        assert_eq!(c.kind, ContactKind::RoadEdge);
        // At 40° but slowly (4 m/s: 2.6 m/s into it): a bump, no crash.
        let slow = wild_rider(&mut s, 600.0, 2.39, 0.7, 4.0);
        s.riders.get_mut(&slow).unwrap().wild = false;
        s.step();
        assert!(s.crash(slow).is_none());
        assert!(s.riders[&slow].bump.is_some());
        assert!(s.rider_state(slow).unwrap().speed < 4.0 * 0.7f64.cos() + 0.01);
        // Shallow (17°, the road's largest angle): the scrape only.
        let scrape = wild_rider(&mut s, 900.0, 2.3, 0.3, 10.0);
        s.riders.get_mut(&scrape).unwrap().wild = false;
        ride(&mut s, 0.5);
        assert!(s.crash(scrape).is_none() && s.riders[&scrape].bump.is_none());
    }

    #[test]
    fn a_hundred_wild_riders_in_a_forest_are_checked_quickly() {
        // A forest tile (a tree in every 8 m cell) with fences; 100 riders
        // in wild mode spread over it (spec §15: the collisions of 100
        // riders within 0.3 ms per tick).
        let cells = vec![ocw_world::landuse::LandClass::Forest.code() + 1; 125 * 125];
        let mut tile = TileObstacles {
            lines: (0..20)
                .map(|i| {
                    let z = 3.0 + i as f32 * 2.9;
                    wall(ObstacleKind::Fence, [0.0, z], [1000.0, z])
                })
                .collect(),
            ..Default::default()
        };
        tile.set_tree_cells(&cells);
        let mut s = world(tile, |_| {});
        for i in 0..100 {
            wild_rider(
                &mut s,
                100.0 + f64::from(i) * 7.3,
                -50.0 + f64::from(i % 20) * 5.1,
                0.3,
                8.0,
            );
        }
        s.resolve_collisions(Mode::Full);
        assert!(s.obstacle_tiles_loaded() >= 1);
        let budget = if cfg!(debug_assertions) { 6.0 } else { 0.3 };
        let n = 100;
        let t = std::time::Instant::now();
        for _ in 0..n {
            s.resolve_collisions(Mode::Full);
        }
        let per_tick_ms = t.elapsed().as_secs_f64() * 1000.0 / f64::from(n);
        println!("world collisions for 100 wild riders in a forest: {per_tick_ms:.4} ms per tick");
        assert!(per_tick_ms < budget, "{per_tick_ms} ms");
    }
}
