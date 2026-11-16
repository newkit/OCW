//! The photographer's drone (spec §2.8.1, the user's request of
//! 2026-11-16: where nobody can stand, the photographer stands near and
//! flies a drone, visible and audible to the rider for 30 s – 2 min
//! before the photo, exactly at the photo's pose at the moment of the
//! photo). Pure: the flight is a function of game time and its
//! [`FlightPlan`] (the pose, the pilot, the take-off, the rider's road to
//! the spot and the timings of the rider's arrival), so every client that
//! has the plan draws the same flight (spec §2.11, v4.38: other riders see
//! the drone too). [`DroneFlight::step`] is the photographed rider's side:
//! it takes off at the rider's ETA, records the road and revises the
//! timing as the rider's speed changes (each new timing blends in over
//! [`BLEND_S`], never past half the time to the photo).
//!
//! Phases: waiting on the pilot's hands → climbing 6 m → flying to the
//! rider and following it (~14 m ahead and ~6 m up, sweeping from side to
//! side across the rider's view, at most
//! [`MAX_SPEED_MPS`]) → the approach (the last [`APPROACH_S`] of the ETA:
//! from where it is to the pose, over the obstacles, ending *exactly* at
//! the pose when the rider reaches the spot) → the photo → back above the
//! pilot → landing. On the way it stays [`DRONE_CLEARANCE_M`] above the
//! ground, tree crowns and buildings under it and [`DRONE_SAFE_M`] away
//! from the rider. [`DroneFlight::abort`] sends it back without a photo
//! (the rider left the planned path).

use crate::photo::{CameraPose, DRONE_CLEARANCE_M, DRONE_SAFE_M, PhotoScene, top_at};

/// Fastest flight (m/s).
pub const MAX_SPEED_MPS: f64 = 18.0;
/// The approach to the pose takes the last this many seconds of the ETA.
pub const APPROACH_S: f64 = 8.0;
/// Climb after take-off (m, s).
pub const CLIMB_M: f64 = 6.0;
pub const CLIMB_S: f64 = 4.0;
/// Landing (s).
pub const LAND_S: f64 = 3.0;
/// Flight before the photo (s): the user's 30 s – 2 min.
pub const MIN_FLIGHT_S: f64 = 30.0;
pub const MAX_FLIGHT_S: f64 = 120.0;
/// Following the rider: distance out and height above the rider (m), and
/// the rate of its sweep across the rider's view (rad/s).
pub const FOLLOW_OUT_M: f64 = 14.0;
pub const FOLLOW_UP_M: f64 = 6.0;
pub const ORBIT_RATE: f64 = 0.15;
/// The photo is held this long at the pose (s).
pub const HOLD_S: f64 = 0.6;

/// The flight's duration before the photo for a shot with `seed`, within
/// 30 s – 2 min and no longer than `eta_s − 2` (the rider's time to the
/// spot when the shot is planned); `None` when there is no time for 30 s.
pub fn flight_duration(seed: u64, eta_s: f64) -> Option<f64> {
    let wish = MIN_FLIGHT_S + (seed % 91) as f64;
    let d = wish.min(eta_s - 2.0);
    (d >= MIN_FLIGHT_S).then_some(d.min(MAX_FLIGHT_S))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DronePhase {
    /// On the pilot's hands, rotors still.
    Waiting,
    Climb,
    /// Flying to the rider and following it.
    Follow,
    Approach,
    /// At the pose: the photo now.
    Photo,
    /// Back to the pilot (also after an abort).
    Return,
    Landing,
    Landed,
}

/// What the app draws and hears of the drone.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DroneState {
    pub pos: [f64; 3],
    /// Unit view direction of its camera.
    pub look: [f64; 3],
    pub phase: DronePhase,
    /// Rotor speed 0 (still) … 1 (flying): the buzz and the spin.
    pub rotor: f64,
    /// The photo is taken in this step.
    pub shot: bool,
    /// Velocity (m/s): the buzz's Doppler.
    pub velocity: [f64; 3],
}

fn lerp3(a: [f64; 3], b: [f64; 3], t: f64) -> [f64; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn len(v: [f64; 3]) -> f64 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

fn norm(v: [f64; 3]) -> [f64; 3] {
    let l = len(v).max(1e-9);
    [v[0] / l, v[1] / l, v[2] / l]
}

/// Smooth 0 → 1 (zero slope at both ends).
pub fn smoothstep(u: f64) -> f64 {
    let u = u.clamp(0.0, 1.0);
    u * u * (3.0 - 2.0 * u)
}

/// The rider's timing for the flight from game time `at`: the rider was
/// `to_go_m` from the spot and is expected there at `shot_at` (spec §2.11,
/// v4.38: the flight is a function of these, the same on every client).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Revision {
    pub at: f64,
    pub to_go_m: f64,
    pub shot_at: f64,
}

impl Revision {
    /// The rider's expected distance to the spot at `t` (negative: past).
    pub fn to_go(&self, t: f64) -> f64 {
        let span = self.shot_at - self.at;
        if span <= 1e-6 {
            return 0.0;
        }
        self.to_go_m * (1.0 - (t - self.at) / span)
    }

    /// How long this revision takes to blend in (s): never past half the
    /// time to its photo, so the drone is exactly at the pose for it.
    pub fn blend_s(&self) -> f64 {
        BLEND_S.min(((self.shot_at - self.at) * 0.5).max(0.0))
    }
}

/// A new timing blends in over this long (s).
pub const BLEND_S: f64 = 1.5;
/// At most this many timings per flight, and the rider's path in at most
/// this many points.
pub const MAX_REVISIONS: usize = 64;
pub const MAX_PATH_POINTS: usize = 200;
/// The owner revises the timing at most once a second, when the expected
/// photo moved by more than [`REVISE_DRIFT_S`], and not in the last
/// [`REVISE_UNTIL_S`] before the photo.
pub const REVISE_EVERY_S: f64 = 1.0;
pub const REVISE_DRIFT_S: f64 = 0.25;
pub const REVISE_UNTIL_S: f64 = 0.5;
/// The flight from the climb to the rider (m/s) and back (m/s).
pub const FLY_IN_MPS: f64 = 11.0;
pub const RETURN_MPS: f64 = MAX_SPEED_MPS * 0.6;

/// Everything a client needs to draw the drone's whole flight (spec
/// §2.11, v4.38): the photo's pose, the pilot, the take-off, the rider's
/// road from the take-off to the spot and the timings. The drone's place
/// at any game time is a pure function of this and the world
/// ([`FlightPlan::state`]), so the photographed rider's client and every
/// other rider's client draw the same flight, frame rate or not.
#[derive(Debug, Clone, PartialEq)]
pub struct FlightPlan {
    pub pose: CameraPose,
    /// The pilot's feet.
    pub pilot: [f64; 3],
    /// Flight before the photo (s).
    pub duration_s: f64,
    /// Game time of the take-off (`None`: still on the pilot's hands).
    pub launch_at: Option<f64>,
    /// `path[i]`: the rider's road position `i × path_step_m` before the
    /// spot (`path[0]` is the spot).
    pub path_step_m: f64,
    pub path: Vec<[f64; 3]>,
    /// The timings, the first at the take-off.
    pub revisions: Vec<Revision>,
    /// Game time of an abort (the rider left the path): back, no photo.
    pub abort_at: Option<f64>,
}

impl FlightPlan {
    pub fn new(pose: CameraPose, pilot: [f64; 3], duration_s: f64) -> Self {
        Self {
            pose,
            pilot,
            duration_s: duration_s.clamp(MIN_FLIGHT_S, MAX_FLIGHT_S),
            launch_at: None,
            path_step_m: 10.0,
            path: Vec::new(),
            revisions: Vec::new(),
            abort_at: None,
        }
    }

    /// Where it takes off and lands: the pilot holds it at chest height.
    pub fn home(&self) -> [f64; 3] {
        [self.pilot[0], self.pilot[1] + 1.25, self.pilot[2]]
    }

    fn top(&self) -> [f64; 3] {
        let h = self.home();
        [h[0], h[1] + CLIMB_M, h[2]]
    }

    /// The photo's game time (the last timing's), unless aborted before.
    pub fn shot_at(&self) -> Option<f64> {
        let s = self.revisions.last()?.shot_at;
        match self.abort_at {
            Some(a) if a < s => None,
            _ => Some(s),
        }
    }

    /// The rider's road position `to_go` m before the spot (past the spot
    /// and beyond the path: along its ends).
    pub fn rider_at(&self, to_go: f64) -> [f64; 3] {
        let n = self.path.len();
        if n == 0 {
            return self.pose.feet;
        }
        if n == 1 {
            return self.path[0];
        }
        let step = self.path_step_m.max(1e-3);
        let f = to_go / step;
        let (i, u) = if f < 0.0 {
            (0, f)
        } else if f >= (n - 1) as f64 {
            (n - 2, f - (n - 2) as f64)
        } else {
            (f.floor() as usize, f - f.floor())
        };
        lerp3(self.path[i], self.path[i + 1], u)
    }

    /// The rider's travel direction (x, z) `to_go` m before the spot.
    fn heading_at(&self, to_go: f64) -> [f64; 2] {
        let a = self.rider_at(to_go + 8.0);
        let b = self.rider_at(to_go - 8.0);
        let d = [b[0] - a[0], b[2] - a[2]];
        let l = (d[0] * d[0] + d[1] * d[1]).sqrt();
        if l > 1e-6 {
            [d[0] / l, d[1] / l]
        } else {
            [0.0, -1.0]
        }
    }

    /// The flight under one timing at `t`, without an abort.
    fn raw(
        &self,
        r: &Revision,
        t: f64,
        scene: &dyn PhotoScene,
    ) -> (DronePhase, [f64; 3], [f64; 3]) {
        let launch = self.launch_at.unwrap_or(f64::INFINITY);
        let home = self.home();
        let top = self.top();
        let tc = launch + CLIMB_S;
        let ta = (r.shot_at - APPROACH_S).max(tc);
        let rider = |t: f64| self.rider_at(r.to_go(t));
        let to_rider = |p: [f64; 3], t: f64| {
            let q = rider(t);
            norm(sub([q[0], q[1] + 1.0, q[2]], p))
        };
        let target = |t: f64| {
            // Ahead of the rider, sweeping from side to side across the
            // rider's view (±50°): seen the whole time.
            let a = 0.9 * (ORBIT_RATE * (t - tc)).sin();
            let (c, s) = (a.cos(), a.sin());
            let h = self.heading_at(r.to_go(t));
            let dir = [h[0] * c - h[1] * s, h[0] * s + h[1] * c];
            let q = rider(t);
            [
                q[0] + FOLLOW_OUT_M * dir[0],
                q[1] + FOLLOW_UP_M,
                q[2] + FOLLOW_OUT_M * dir[1],
            ]
        };
        let ease_s = (len(sub(target(tc), top)) / FLY_IN_MPS).max(2.0);
        let follow = |t: f64| {
            let w = smoothstep((t - tc) / ease_s);
            Self::clear(scene, lerp3(top, target(t), w), rider(t), 1.0)
        };
        if t < launch {
            return (DronePhase::Waiting, home, self.pose.look);
        }
        if t < tc {
            let p = lerp3(home, top, smoothstep((t - launch) / CLIMB_S));
            return (DronePhase::Climb, p, to_rider(p, t));
        }
        if t < ta {
            let p = follow(t);
            return (DronePhase::Follow, p, to_rider(p, t));
        }
        if t < r.shot_at {
            let from = follow(ta);
            let from_look = to_rider(from, ta);
            let u = ((t - ta) / (r.shot_at - ta).max(1e-6)).clamp(0.0, 1.0);
            let s = smoothstep(u);
            let mut p = lerp3(from, self.pose.eye, s);
            // An arc over what is between.
            p[1] += 3.0 * (std::f64::consts::PI * s).sin();
            let weight = 1.0 - smoothstep((u - 0.8) / 0.2);
            let p = Self::clear(scene, p, rider(t), weight);
            let l = lerp3(from_look, self.pose.look, smoothstep(u * 1.25));
            return (DronePhase::Approach, p, norm(lerp3(to_rider(p, t), l, s)));
        }
        if t < r.shot_at + HOLD_S {
            return (DronePhase::Photo, self.pose.eye, self.pose.look);
        }
        self.returning(
            self.pose.eye,
            self.pose.look,
            r.shot_at + HOLD_S,
            t,
            rider(t),
            scene,
        )
    }

    /// Back from `from` (at `since`) to above the pilot and down.
    fn returning(
        &self,
        from: [f64; 3],
        look: [f64; 3],
        since: f64,
        t: f64,
        rider: [f64; 3],
        scene: &dyn PhotoScene,
    ) -> (DronePhase, [f64; 3], [f64; 3]) {
        let top = self.top();
        let dur = (len(sub(top, from)) / RETURN_MPS).max(1.0);
        let u = (t - since) / dur;
        if u < 1.0 {
            let p = Self::clear(scene, lerp3(from, top, smoothstep(u)), rider, 1.0);
            return (DronePhase::Return, p, look);
        }
        let v = (t - since - dur) / LAND_S;
        if v < 1.0 {
            return (
                DronePhase::Landing,
                lerp3(top, self.home(), smoothstep(v)),
                look,
            );
        }
        (DronePhase::Landed, self.home(), look)
    }

    /// The flight with every timing known at `t`, each new one blended in.
    fn blended(&self, t: f64, scene: &dyn PhotoScene) -> (DronePhase, [f64; 3], [f64; 3]) {
        let known: Vec<&Revision> = self.revisions.iter().filter(|r| r.at <= t).collect();
        let Some(first) = known.first() else {
            return (DronePhase::Waiting, self.home(), self.pose.look);
        };
        // From the last timing fully blended in at `t`.
        let from = known
            .iter()
            .rposition(|r| r.at + r.blend_s() <= t)
            .unwrap_or(0);
        let (mut phase, mut pos, mut look) = self.raw(known.get(from).unwrap_or(first), t, scene);
        for r in &known[from + 1..] {
            let w = smoothstep((t - r.at) / r.blend_s().max(1e-6));
            let (ph, p, l) = self.raw(r, t, scene);
            pos = lerp3(pos, p, w);
            look = norm(lerp3(look, l, w));
            phase = ph;
        }
        (phase, pos, look)
    }

    fn place(&self, t: f64, scene: &dyn PhotoScene) -> (DronePhase, [f64; 3], [f64; 3]) {
        if self.launch_at.is_none() {
            let phase = if self.abort_at.is_some() {
                DronePhase::Landed
            } else {
                DronePhase::Waiting
            };
            return (phase, self.home(), self.pose.look);
        }
        match self.abort_at {
            Some(a) if t >= a && self.shot_at().is_none() => {
                let (_, p, l) = self.blended(a, scene);
                let r = self.revisions.last().copied();
                let rider = r.map_or(self.pose.feet, |r| self.rider_at(r.to_go(t)));
                self.returning(p, l, a, t, rider, scene)
            }
            _ => self.blended(t, scene),
        }
    }

    /// The drone at game time `t` (pure: the same on every client).
    pub fn state(&self, t: f64, scene: &dyn PhotoScene) -> DroneState {
        let (phase, pos, look) = self.place(t, scene);
        let dt = 0.05;
        let (_, before, _) = self.place(t - dt, scene);
        let launch = self.launch_at.unwrap_or(f64::INFINITY);
        let rotor = match phase {
            DronePhase::Waiting | DronePhase::Landed => 0.0,
            DronePhase::Climb => (t - launch).clamp(0.3, 1.0),
            DronePhase::Landing => {
                // Its start: the return's end, from where it began.
                let since = self.landing_since(t, scene);
                1.0 - 0.6 * ((t - since) / LAND_S).clamp(0.0, 1.0)
            }
            _ => 1.0,
        };
        let velocity = [
            (pos[0] - before[0]) / dt,
            (pos[1] - before[1]) / dt,
            (pos[2] - before[2]) / dt,
        ];
        DroneState {
            pos,
            look,
            phase,
            rotor,
            shot: false,
            velocity,
        }
    }

    /// When the landing began (for the rotor's run-down).
    fn landing_since(&self, t: f64, scene: &dyn PhotoScene) -> f64 {
        let (from, since) = match (self.abort_at, self.shot_at()) {
            (Some(a), None) => (self.blended(a, scene).1, a),
            (_, Some(s)) => (self.pose.eye, s + HOLD_S),
            _ => return t,
        };
        since + (len(sub(self.top(), from)) / RETURN_MPS).max(1.0)
    }

    /// Keeps `p` clear of what is under it and away from the rider.
    fn clear(scene: &dyn PhotoScene, p: [f64; 3], rider: [f64; 3], weight: f64) -> [f64; 3] {
        let mut p = p;
        if weight > 0.0
            && let Some(need) = clearance(scene, p[0], p[2])
            && p[1] < need
        {
            p[1] += (need - p[1]) * weight;
        }
        let head = [rider[0], rider[1] + 1.2, rider[2]];
        let d = sub(p, head);
        let dist = len(d);
        if dist < DRONE_SAFE_M {
            let h = (d[0] * d[0] + d[2] * d[2]).sqrt();
            let dir = if h > 1e-6 {
                [d[0] / h, 0.0, d[2] / h]
            } else {
                [1.0, 0.0, 0.0]
            };
            let need = (DRONE_SAFE_M * DRONE_SAFE_M - d[1] * d[1]).max(0.0).sqrt();
            p = [head[0] + dir[0] * need, p[1], head[2] + dir[2] * need];
        }
        p
    }
}

/// Around a clearance's place: a world-aligned grid this fine (m), this
/// far (m), the height needed falling off with this slope (m per m).
const CLEAR_GRID_M: f64 = 5.0;
const CLEAR_REACH_M: f64 = 25.0;
const CLEAR_SLOPE: f64 = 0.9;

/// The height the drone needs over (x, z): [`DRONE_CLEARANCE_M`] above
/// what is under it, and rising early before what is near (tree crowns,
/// a house: a cone of [`CLEAR_SLOPE`] over a grid fixed to the world), so
/// it climbs over a tree stand smoothly instead of jumping up at its edge.
pub fn clearance(scene: &dyn PhotoScene, x: f64, z: f64) -> Option<f64> {
    let mut need = top_at(scene, x, z)? + DRONE_CLEARANCE_M;
    let n = (CLEAR_REACH_M / CLEAR_GRID_M).ceil() as i64;
    let (gx, gz) = (
        (x / CLEAR_GRID_M).round() as i64,
        (z / CLEAR_GRID_M).round() as i64,
    );
    for i in -n..=n {
        for j in -n..=n {
            let (qx, qz) = (
                (gx + i) as f64 * CLEAR_GRID_M,
                (gz + j) as f64 * CLEAR_GRID_M,
            );
            let d = ((qx - x).powi(2) + (qz - z).powi(2)).sqrt();
            if d > CLEAR_REACH_M {
                continue;
            }
            if let Some(top) = top_at(scene, qx, qz) {
                need = need.max(top + DRONE_CLEARANCE_M - CLEAR_SLOPE * d);
            }
        }
    }
    Some(need)
}

/// A drone's flight for one photo, as the photographed rider's client
/// runs it: it takes off at the rider's ETA, records the rider's road and
/// revises the timing as the rider's speed changes ([`DroneFlight::step`]);
/// what it draws is always [`FlightPlan::state`] of its plan, which the
/// other clients get through the server (spec §2.11, v4.38).
#[derive(Debug, Clone, PartialEq)]
pub struct DroneFlight {
    pub plan: FlightPlan,
    phase: DronePhase,
    last_t: Option<f64>,
    shot_taken: bool,
}

impl DroneFlight {
    /// A flight from the pilot (`pilot` feet) to `pose`, taking off
    /// `duration_s` before the rider reaches the spot.
    pub fn new(pose: CameraPose, pilot: [f64; 3], duration_s: f64) -> Self {
        Self::from_plan(FlightPlan::new(pose, pilot, duration_s))
    }

    /// Another rider's flight from its plan.
    pub fn from_plan(plan: FlightPlan) -> Self {
        Self {
            plan,
            phase: DronePhase::Waiting,
            last_t: None,
            shot_taken: false,
        }
    }

    pub fn phase(&self) -> DronePhase {
        self.phase
    }

    pub fn duration_s(&self) -> f64 {
        self.plan.duration_s
    }

    /// Whether it has landed (or never took off after an abort).
    pub fn done(&self) -> bool {
        self.phase == DronePhase::Landed
    }

    /// The rider left the planned path: back to the pilot, no photo.
    pub fn abort(&mut self, t: f64) {
        if self.plan.abort_at.is_some() || self.shot_taken {
            return;
        }
        self.plan.abort_at = Some(t);
        if self.plan.launch_at.is_none() {
            self.phase = DronePhase::Landed;
        }
    }

    /// One step at game time `t`: the rider `to_go_m` before the spot,
    /// `eta_s` seconds from it (`None`: passed it or stopped); `path(s)`
    /// is the rider's road position `s` m before the spot (sampled once,
    /// at the take-off).
    pub fn step(
        &mut self,
        t: f64,
        to_go_m: f64,
        eta_s: Option<f64>,
        path: &dyn Fn(f64) -> [f64; 3],
        scene: &dyn PhotoScene,
    ) -> DroneState {
        let p = &mut self.plan;
        if p.abort_at.is_none() {
            match (p.launch_at, eta_s) {
                (None, Some(e)) if e <= p.duration_s => {
                    p.launch_at = Some(t);
                    let length = to_go_m.max(0.0);
                    let step = (length / (MAX_PATH_POINTS - 1) as f64).max(5.0);
                    let n = (length / step).ceil() as usize + 1;
                    p.path_step_m = step;
                    p.path = (0..=n).map(|i| path(i as f64 * step)).collect();
                    p.revisions = vec![Revision {
                        at: t,
                        to_go_m,
                        shot_at: t + e,
                    }];
                }
                (Some(_), Some(e)) => {
                    let last = *p.revisions.last().expect("launched");
                    let new = t + e;
                    if t < last.shot_at - REVISE_UNTIL_S
                        && t - last.at >= REVISE_EVERY_S
                        && (new - last.shot_at).abs() > REVISE_DRIFT_S
                        && p.revisions.len() < MAX_REVISIONS
                    {
                        p.revisions.push(Revision {
                            at: t,
                            to_go_m,
                            shot_at: new,
                        });
                    }
                }
                _ => {}
            }
        }
        let mut s = self.plan.state(t, scene);
        if let Some(at) = self.plan.shot_at()
            && !self.shot_taken
            && self.plan.launch_at.is_some()
            && t >= at
            && self.last_t.is_none_or(|l| l < at + HOLD_S)
        {
            self.shot_taken = true;
            s.shot = true;
        }
        self.last_t = Some(t);
        self.phase = s.phase;
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::photo::{Carrier, FnScene, PoseKind};

    fn pose() -> CameraPose {
        CameraPose {
            kind: PoseKind::AerialWide,
            carrier: Carrier::Drone {
                pilot: [8.0, 100.0, -20.0],
            },
            feet: [30.0, 100.0, 5.0],
            eye: [30.0, 125.0, 5.0],
            look: norm([-0.8, -0.6, -0.1]),
        }
    }

    /// Hills, a tree stand and a house along the way.
    #[allow(clippy::type_complexity)]
    fn scene() -> FnScene<
        impl Fn(f64, f64) -> Option<f64>,
        impl Fn(f64, f64) -> bool,
        impl Fn(f64, f64) -> f64,
    > {
        FnScene {
            ground: |x: f64, z: f64| Some(100.0 + 6.0 * (x / 40.0).sin() + 4.0 * (z / 55.0).cos()),
            water: |_, _| false,
            canopy: |x: f64, z: f64| {
                if (10.0..30.0).contains(&x) && (100.0..200.0).contains(&z) {
                    22.0
                } else {
                    0.0
                }
            },
            solids: vec![([-30.0, 300.0], [-12.0, 330.0], 115.0)],
            landmarks: Vec::new(),
        }
    }

    #[test]
    fn flights_last_half_a_minute_to_two_minutes() {
        for seed in 0..200 {
            let d = flight_duration(seed, 500.0).unwrap();
            assert!((MIN_FLIGHT_S..=MAX_FLIGHT_S).contains(&d), "{d}");
        }
        assert_eq!(
            flight_duration(90, 60.0),
            Some(58.0),
            "no longer than the ETA"
        );
        assert_eq!(flight_duration(3, 25.0), None, "no time for 30 s");
    }

    /// A rider from 900 m south at 8 m/s towards the spot (the origin),
    /// the drone launched at its ETA, flying around, exactly at the pose
    /// at the photo, clear of the ground, the trees, the house and the
    /// rider all the way, back on the pilot's hands at the end.
    #[test]
    fn the_drone_flies_around_and_is_exactly_at_the_pose_for_the_photo() {
        let scene = scene();
        let p = pose();
        let pilot = [8.0, scene.ground(8.0, -20.0).unwrap(), -20.0];
        let mut f = DroneFlight::new(p, pilot, 75.0);
        let speed = 8.0;
        let start_z = 900.0;
        let dt = 1.0 / 30.0;
        let (mut launched, mut shot_at) = (None, None);
        let mut t = 0.0;
        while t < 300.0 && !f.done() {
            let z = start_z - speed * t;
            let rider = [0.6, scene.ground(0.6, z).unwrap(), z];
            let eta = (z > 0.0).then_some(z / speed);
            let road = |s: f64| [0.6, scene.ground(0.6, s).unwrap(), s];
            let s = f.step(t, z, eta, &road, &scene);
            if s.phase != DronePhase::Waiting && launched.is_none() {
                launched = Some(t);
            }
            if s.shot {
                assert!(shot_at.is_none(), "one photo");
                shot_at = Some(t);
                let d = len(sub(s.pos, p.eye));
                assert!(d < 0.02, "at the pose: {d} m");
                let angle = (s.look[0] * p.look[0] + s.look[1] * p.look[1] + s.look[2] * p.look[2])
                    .clamp(-1.0, 1.0)
                    .acos()
                    .to_degrees();
                assert!(angle < 0.5, "looking along the pose: {angle}°");
            }
            let flying = !matches!(
                s.phase,
                DronePhase::Waiting | DronePhase::Landed | DronePhase::Landing
            ) && !(s.phase == DronePhase::Climb && t - launched.unwrap() < 1.0);
            if flying && s.phase != DronePhase::Photo {
                let top = top_at(&scene, s.pos[0], s.pos[2]).unwrap();
                assert!(
                    s.pos[1] > top + 0.5,
                    "t {t:.1} {:?}: {:?} over {top}",
                    s.phase,
                    s.pos
                );
            }
            if s.phase != DronePhase::Waiting {
                let head = [rider[0], rider[1] + 1.2, rider[2]];
                assert!(
                    len(sub(s.pos, head)) >= DRONE_SAFE_M - 0.05,
                    "t {t:.1}: too close"
                );
            }
            t += dt;
        }
        let (l, s) = (launched.unwrap(), shot_at.expect("the photo"));
        let flight = s - l;
        assert!((MIN_FLIGHT_S..=MAX_FLIGHT_S).contains(&flight), "{flight}");
        assert!((flight - 75.0).abs() < 0.5, "{flight}");
        assert!(f.done(), "landed again");
    }

    #[test]
    fn an_abort_brings_it_back_without_a_photo() {
        let scene = scene();
        let p = pose();
        let mut f = DroneFlight::new(p, [8.0, 100.0, -20.0], 40.0);
        let mut t = 0.0;
        let mut photos = 0;
        let mut aborted = false;
        while t < 200.0 && !f.done() {
            let z = 400.0 - 8.0 * t;
            let rider = [0.6, 105.0, z];
            if t >= 30.0 && !aborted {
                assert_eq!(f.phase(), DronePhase::Follow);
                f.abort(t);
                aborted = true;
            }
            let _ = rider;
            let road = |s: f64| [0.6, 105.0, s];
            let s = f.step(t, z, Some(z / 8.0), &road, &scene);
            photos += usize::from(s.shot);
            t += 1.0 / 25.0;
        }
        assert_eq!(photos, 0);
        assert!(f.done());
        // Aborted before take-off: it never flies.
        let mut f = DroneFlight::new(p, [8.0, 100.0, -20.0], 40.0);
        f.abort(0.0);
        assert!(f.done());
    }

    /// A rider that slows down and speeds up again (spec §2.11, v4.38): the
    /// owner revises the timing, the drone moves on without a jump and is
    /// exactly at the pose when the rider reaches the spot.
    #[test]
    fn a_changing_speed_revises_the_timing_without_a_jump() {
        let scene = scene();
        let p = pose();
        let pilot = [8.0, scene.ground(8.0, -20.0).unwrap(), -20.0];
        let mut f = DroneFlight::new(p, pilot, 60.0);
        let road = |s: f64| [0.6, scene.ground(0.6, s).unwrap(), s];
        let dt = 1.0 / 30.0;
        let mut z: f64 = 700.0;
        let mut t = 0.0;
        let mut last: Option<[f64; 3]> = None;
        let mut shot_to_go = None;
        while t < 300.0 && !f.done() {
            // 9 m/s, 5 m/s for a while, then 8 m/s.
            let speed = if t < 25.0 {
                9.0
            } else if t < 45.0 {
                5.0
            } else {
                8.0
            };
            let eta = (z > 0.0).then_some(z / speed);
            let s = f.step(t, z, eta, &road, &scene);
            if s.shot {
                shot_to_go = Some(z);
                assert!(len(sub(s.pos, p.eye)) < 0.02, "at the pose");
            }
            if let Some(l) = last {
                let jump = len(sub(s.pos, l));
                assert!(jump < 1.6, "t {t:.2} {:?}: {jump} m in one frame", s.phase);
            }
            last = Some(s.pos);
            z -= speed * dt;
            t += dt;
        }
        assert!(
            f.plan.revisions.len() > 2,
            "revised: {:?}",
            f.plan.revisions
        );
        let to_go = shot_to_go.expect("the photo");
        assert!(to_go.abs() < 1.5, "the rider at the spot: {to_go} m");
        assert!(f.done());
    }

    /// Every client draws the same flight from the plan (spec §2.11,
    /// v4.38): the owner's frames at 30 Hz and another client's at 47 Hz,
    /// from the plan as it was known at each moment, agree exactly.
    #[test]
    fn every_client_draws_the_same_flight_from_the_plan() {
        let scene = scene();
        let p = pose();
        let pilot = [8.0, scene.ground(8.0, -20.0).unwrap(), -20.0];
        let mut f = DroneFlight::new(p, pilot, 45.0);
        let road = |s: f64| [0.6, scene.ground(0.6, s).unwrap(), s];
        let mut z: f64 = 600.0;
        let mut t: f64 = 0.0;
        let mut owner = Vec::new();
        let mut plans = Vec::new();
        while t < 200.0 && !f.done() {
            let speed = 7.0 + 2.0 * (t / 9.0).sin();
            let eta = (z > 0.0).then_some(z / speed);
            let s = f.step(t, z, eta, &road, &scene);
            owner.push((t, s));
            plans.push((t, f.plan.clone()));
            z -= speed / 30.0;
            t += 1.0 / 30.0;
        }
        assert!(f.plan.revisions.len() > 1, "revised");
        // The other client: the final plan at the owner's moments …
        let other = DroneFlight::from_plan(f.plan.clone());
        for (t, s) in &owner {
            let o = other.plan.state(*t, &scene);
            assert_eq!(o.pos, s.pos, "t {t}");
            assert_eq!(o.look, s.look, "t {t}");
            assert_eq!(o.phase, s.phase, "t {t}");
        }
        // … and the plan as known then, at its own frame times.
        let mut tt = 0.0;
        while tt < t {
            let known = plans
                .iter()
                .rev()
                .find(|(at, _)| *at <= tt)
                .map(|(_, pl)| pl)
                .unwrap();
            let a = known.state(tt, &scene);
            let b = f.plan.state(tt, &scene);
            assert_eq!(a.pos, b.pos, "t {tt}");
            tt += 1.0 / 47.0;
        }
        let shot = f.plan.shot_at().expect("a photo");
        let at = other.plan.state(shot, &scene);
        assert_eq!(at.phase, DronePhase::Photo);
        assert!(len(sub(at.pos, p.eye)) < 1e-9);
    }
}
