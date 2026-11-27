//! Plans and plan changes (spec 18F Q3 (a): each vehicle is sent once
//! with its route and speeds, then only its changes).
//!
//! A vehicle's motion is a deterministic function of its [`Plan`] and the
//! [`PlanChange`]s after it: [`PlanFollower`] steps it tick by tick with
//! the shared maths of [`super::kin`]. The server runs the same follower
//! beside each simulated vehicle and sends a change only when the real
//! vehicle leaves what the follower predicts (it brakes behind another,
//! stops at a light, waits to give way, changes lane, gets a longer
//! route) – so a client that applies the events in order sees each
//! vehicle within [`POS_TOLERANCE_M`] of the server's at every tick.

use ocw_world::RoadNetwork;

use super::kin::{self, Curves, DriverParams, RoutePos};
use super::rules::VehicleClass;

/// The server sends a change when the follower is off by more than this (m).
pub const POS_TOLERANCE_M: f64 = 0.5;
/// … or its speed by more than this (m/s).
pub const SPEED_TOLERANCE: f64 = 0.5;

/// The traffic simulation's tick (one fixed step, [`super::TrafficConfig::step_ms`]).
pub type Tick = u64;

/// One edge of a route, driven in one direction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Step {
    /// Edge index in the road network.
    pub edge: u32,
    /// From the edge's `from` to its `to`.
    pub forward: bool,
    /// The driver's desired speed on this edge (m/s): the limit for the
    /// class times the driver's factor.
    pub speed: f32,
    /// The radius of the turn at the end of the step into the next (m).
    pub turn_r: f32,
}

/// How a vehicle moves between changes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Motion {
    /// Free driving: the IDM's free road towards the plan's desired speed
    /// with the curve and turn limits of [`kin::desired_speed`].
    Free,
    /// A constant acceleration `a` (m/s²) until the speed reaches
    /// `target`, then that speed (following, braking for a light or a
    /// queue, pulling away).
    Accel { a: f32, target: f32 },
    /// Standing (a queue, a red light, giving way).
    Stopped,
}

/// The lateral shift of a vehicle from its lane's centre (T4: pulling
/// out to pass a rider, moving back in, swerving away from a rider in its
/// lane): metres towards the road's centreline (negative = towards the
/// kerb), moving towards `to` at `rate` m/s.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Shift {
    pub at: f32,
    pub to: f32,
    pub rate: f32,
}

/// The server sends a change when the follower's shift is off by more
/// than this (m).
pub const SHIFT_TOLERANCE_M: f64 = 0.15;

/// One tick of a shift (`at` towards `to` at `rate`, m and m/s): the same
/// on the server and the clients.
pub fn shift_step(at: f64, to: f64, rate: f64, dt: f64) -> f64 {
    let max = rate.abs() * dt;
    at + (to - at).clamp(-max, max)
}

/// A vehicle as it is spawned: sent once (`TrafficSpawn`, T5).
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub id: u32,
    pub class: VehicleClass,
    /// Seeds the vehicle's look (shape, colour) on the client.
    pub look: u32,
    pub tick: Tick,
    pub driver: DriverParams,
    pub route: Vec<Step>,
    /// The number of the route's first step in the vehicle's numbering
    /// (0 at its spawn; a later plan for a joining client starts at the
    /// step the vehicle is on, and the changes count from the spawn).
    pub base: u32,
    /// Metres along the first step.
    pub s: f32,
    pub v: f32,
    /// Lane, 0 = the kerb-side lane of the driving direction.
    pub lane: u8,
    pub motion: Motion,
    /// Its lateral shift (T4; zero at a spawn).
    pub shift: Shift,
    /// A bus on its line (spec 18B *Buses on their lines*): the line's
    /// number in the session ([`super::bus::LineId`]: the package's bus
    /// route and the piece of it), 0 for every other vehicle.
    pub line: u32,
    /// A bus standing at a stop with its doors open.
    pub doors: bool,
}

/// What changed in a vehicle's plan at `tick` (after that tick's step).
/// Every change carries the vehicle's state, so it is a correction too.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanChange {
    pub id: u32,
    pub tick: Tick,
    /// The route step (an index of the plan's route, extensions counted).
    pub step: u32,
    pub s: f32,
    pub v: f32,
    pub lane: u8,
    pub motion: Motion,
    /// The lateral shift (T4).
    pub shift: Shift,
    /// A bus's doors are open (at a stop; the dwell is sent as changes).
    pub doors: bool,
    /// A longer route (it neared the end of its plan): these steps
    /// replace the route's last step (whose turn now leads into the new
    /// ones) and follow it. Empty: the route is unchanged.
    pub extend: Vec<Step>,
}

/// The traffic's output for the network layer (T5) and the tests.
#[derive(Debug, Clone, PartialEq)]
pub enum TrafficEvent {
    Spawn(Plan),
    Change(PlanChange),
    /// The vehicle left the riders' bubbles or reached its destination.
    Despawn {
        id: u32,
        tick: Tick,
    },
}

impl TrafficEvent {
    pub fn id(&self) -> u32 {
        match self {
            Self::Spawn(p) => p.id,
            Self::Change(c) => c.id,
            Self::Despawn { id, .. } => *id,
        }
    }
}

/// A vehicle extrapolated from its plan and changes: what a client runs
/// (and the server beside each vehicle, to know when to send a change).
#[derive(Debug, Clone, PartialEq)]
pub struct PlanFollower {
    pub id: u32,
    pub class: VehicleClass,
    pub look: u32,
    pub driver: DriverParams,
    pub route: Vec<Step>,
    /// See [`Plan::base`]; `pos.step` indexes `route`.
    pub base: u32,
    pub pos: RoutePos,
    pub v: f64,
    pub lane: u8,
    pub motion: Motion,
    /// The lateral shift now, its target and rate (see [`Shift`]).
    pub shift: f64,
    pub shift_to: f64,
    pub shift_rate: f64,
    /// The tick the state is at.
    pub tick: Tick,
    /// The route ended (the vehicle stands at its end).
    pub ended: bool,
    /// The bus line (0: none) and whether its doors are open.
    pub line: u32,
    pub doors: bool,
}

impl PlanFollower {
    pub fn new(plan: &Plan) -> Self {
        Self {
            id: plan.id,
            class: plan.class,
            look: plan.look,
            driver: plan.driver,
            route: plan.route.clone(),
            base: plan.base,
            pos: RoutePos {
                step: 0,
                s: f64::from(plan.s),
            },
            v: f64::from(plan.v),
            lane: plan.lane,
            motion: plan.motion,
            shift: f64::from(plan.shift.at),
            shift_to: f64::from(plan.shift.to),
            shift_rate: f64::from(plan.shift.rate),
            tick: plan.tick,
            ended: false,
            line: plan.line,
            doors: plan.doors,
        }
    }

    /// Applies a change (its state replaces the extrapolated one).
    pub fn apply(&mut self, c: &PlanChange) {
        if !c.extend.is_empty() {
            self.route.pop();
            self.route.extend_from_slice(&c.extend);
        }
        self.pos = RoutePos {
            step: c.step.saturating_sub(self.base) as usize,
            s: f64::from(c.s),
        };
        self.v = f64::from(c.v);
        self.lane = c.lane;
        self.motion = c.motion;
        self.shift = f64::from(c.shift.at);
        self.shift_to = f64::from(c.shift.to);
        self.shift_rate = f64::from(c.shift.rate);
        self.tick = c.tick;
        self.ended = false;
        self.doors = c.doors;
    }

    /// The acceleration the motion gives now.
    pub fn accel(&self, net: &RoadNetwork, curves: &Curves) -> f64 {
        match self.motion {
            Motion::Free => {
                let v0 = kin::desired_speed(net, curves, &self.route, self.pos, &self.driver);
                kin::idm(&self.driver, self.v, v0, None)
            }
            Motion::Accel { a, target } => {
                let (a, t) = (f64::from(a), f64::from(target));
                if (a > 0.0 && self.v >= t) || (a < 0.0 && self.v <= t) {
                    0.0
                } else {
                    a
                }
            }
            Motion::Stopped => 0.0,
        }
    }

    /// One tick of `dt` seconds.
    pub fn step(&mut self, net: &RoadNetwork, curves: &Curves, dt: f64) {
        self.tick += 1;
        self.shift = shift_step(self.shift, self.shift_to, self.shift_rate, dt);
        if self.ended || self.motion == Motion::Stopped {
            if self.motion == Motion::Stopped {
                self.v = 0.0;
            }
            return;
        }
        let a = self.accel(net, curves);
        let (mut v, ok) = kin::advance(net, &self.route, &mut self.pos, self.v, a, dt);
        // An `Accel` stops exactly at its target, as `accel` holds it there.
        if let Motion::Accel { a, target } = self.motion {
            let t = f64::from(target);
            if (a > 0.0 && v > t) || (a < 0.0 && v < t) {
                v = t;
            }
        }
        self.v = v;
        self.ended = !ok;
    }

    /// Steps until `tick`.
    pub fn advance_to(&mut self, net: &RoadNetwork, curves: &Curves, tick: Tick, dt: f64) {
        while self.tick < tick {
            self.step(net, curves, dt);
        }
    }

    /// The centreline position (x, z).
    pub fn point(&self, net: &RoadNetwork) -> Option<[f64; 2]> {
        let (e, _, s) = self.on_edge(net)?;
        let p = net.geometry(e).point_at(s);
        Some([p[0], p[2]])
    }

    /// The edge, direction and metres from the edge's `from` vertex.
    pub fn on_edge(&self, net: &RoadNetwork) -> Option<(usize, bool, f64)> {
        let st = self.route.get(self.pos.step)?;
        let e = st.edge as usize;
        let s = if st.forward {
            self.pos.s
        } else {
            kin::step_len(net, st) - self.pos.s
        };
        Some((e, st.forward, s))
    }
}

/// Applies events in order to a set of followers (a client's view):
/// spawns add, changes correct, despawns remove. Followers are kept
/// sorted by id.
pub fn apply_events(followers: &mut Vec<PlanFollower>, events: &[TrafficEvent]) {
    for ev in events {
        match ev {
            TrafficEvent::Spawn(p) => {
                let at = followers.partition_point(|f| f.id < p.id);
                followers.insert(at, PlanFollower::new(p));
            }
            TrafficEvent::Change(c) => {
                if let Ok(i) = followers.binary_search_by_key(&c.id, |f| f.id) {
                    followers[i].apply(c);
                }
            }
            TrafficEvent::Despawn { id, .. } => {
                if let Ok(i) = followers.binary_search_by_key(id, |f| f.id) {
                    followers.remove(i);
                }
            }
        }
    }
}
