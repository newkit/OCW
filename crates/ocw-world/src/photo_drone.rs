//! The photographer's drone (spec §2.8.1, the user's request of
//! 2026-11-16: where nobody can stand, the photographer stands near and
//! flies a drone, visible and audible to the rider for 30 s – 2 min
//! before the photo, exactly at the photo's pose at the moment of the
//! photo). Pure and stepped by the app's frames: [`DroneFlight::step`]
//! takes the game time, the rider's position and the rider's time to the
//! shot spot (its ETA), and gives the drone's position, view and phase.
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

/// A drone's flight for one photo.
#[derive(Debug, Clone, PartialEq)]
pub struct DroneFlight {
    /// The photo's pose (eye and view).
    pub pose: CameraPose,
    /// Where it takes off and lands: the pilot's hands.
    pub home: [f64; 3],
    /// Flight before the photo (s): it takes off when the rider's ETA to
    /// the spot is this.
    pub duration_s: f64,
    phase: DronePhase,
    pos: [f64; 3],
    look: [f64; 3],
    /// Game time the phase began.
    since: f64,
    /// Where the approach (or the return) began.
    from: [f64; 3],
    from_look: [f64; 3],
    /// The approach's progress (never goes back).
    u: f64,
    /// The swing's phase (rad): the drone sweeps across the rider's view.
    theta: f64,
    /// The rider's travel direction (from its movement) and last place.
    heading: [f64; 2],
    last_rider: Option<[f64; 3]>,
    last_t: Option<f64>,
}

impl DroneFlight {
    /// A flight from the pilot (`pilot` feet) to `pose`, taking off
    /// `duration_s` before the rider reaches the spot.
    pub fn new(pose: CameraPose, pilot: [f64; 3], duration_s: f64) -> Self {
        // The pilot holds it at chest height, a little in front.
        let home = [pilot[0], pilot[1] + 1.25, pilot[2]];
        let theta = 0.0;
        Self {
            pose,
            home,
            duration_s: duration_s.clamp(MIN_FLIGHT_S, MAX_FLIGHT_S),
            phase: DronePhase::Waiting,
            pos: home,
            look: pose.look,
            since: 0.0,
            from: home,
            from_look: pose.look,
            u: 0.0,
            theta,
            heading: [0.0, -1.0],
            last_rider: None,
            last_t: None,
        }
    }

    pub fn phase(&self) -> DronePhase {
        self.phase
    }

    /// Whether it is in the air or will still take the photo.
    pub fn done(&self) -> bool {
        self.phase == DronePhase::Landed
    }

    /// The rider left the planned path: back to the pilot, no photo.
    pub fn abort(&mut self, t: f64) {
        match self.phase {
            DronePhase::Waiting => self.phase = DronePhase::Landed,
            DronePhase::Return | DronePhase::Landing | DronePhase::Landed => {}
            _ => self.start(DronePhase::Return, t),
        }
    }

    fn start(&mut self, phase: DronePhase, t: f64) {
        self.phase = phase;
        self.since = t;
        self.from = self.pos;
        self.from_look = self.look;
    }

    /// Keeps `p` clear of what is under it and away from the rider.
    fn clear(scene: &dyn PhotoScene, p: [f64; 3], rider: [f64; 3], weight: f64) -> [f64; 3] {
        let mut p = p;
        if weight > 0.0
            && let Some(top) = top_at(scene, p[0], p[2])
        {
            let need = top + DRONE_CLEARANCE_M;
            if p[1] < need {
                p[1] += (need - p[1]) * weight;
            }
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

    /// One step at game time `t`: the rider at `rider` (road position),
    /// `eta_s` seconds from the spot (`None`: passed it or stopped).
    pub fn step(
        &mut self,
        t: f64,
        rider: [f64; 3],
        eta_s: Option<f64>,
        scene: &dyn PhotoScene,
    ) -> DroneState {
        let dt = self.last_t.map_or(0.0, |l| (t - l).max(0.0));
        self.last_t = Some(t);
        if let Some(l) = self.last_rider {
            let m = [rider[0] - l[0], rider[2] - l[2]];
            let ml = (m[0] * m[0] + m[1] * m[1]).sqrt();
            if ml > 0.05 {
                let k = 0.2;
                let h = [
                    self.heading[0] * (1.0 - k) + m[0] / ml * k,
                    self.heading[1] * (1.0 - k) + m[1] / ml * k,
                ];
                let hl = (h[0] * h[0] + h[1] * h[1]).sqrt().max(1e-9);
                self.heading = [h[0] / hl, h[1] / hl];
            }
        }
        self.last_rider = Some(rider);
        let before = self.pos;
        let mut shot = false;
        let to_rider = |p: [f64; 3]| norm(sub([rider[0], rider[1] + 1.0, rider[2]], p));
        match self.phase {
            DronePhase::Waiting => {
                if eta_s.is_some_and(|e| e <= self.duration_s) {
                    self.start(DronePhase::Climb, t);
                }
            }
            DronePhase::Climb => {
                let u = (t - self.since) / CLIMB_S;
                self.pos = lerp3(
                    self.home,
                    [self.home[0], self.home[1] + CLIMB_M, self.home[2]],
                    smoothstep(u),
                );
                self.look = to_rider(self.pos);
                if u >= 1.0 {
                    self.start(DronePhase::Follow, t);
                }
            }
            DronePhase::Follow => {
                // Ahead of the rider, sweeping from side to side across
                // the rider's view (±50°): seen the whole time.
                self.theta += ORBIT_RATE * dt;
                let a = 0.9 * self.theta.sin();
                let (c, s) = (a.cos(), a.sin());
                let h = self.heading;
                let dir = [h[0] * c - h[1] * s, h[0] * s + h[1] * c];
                let target = [
                    rider[0] + FOLLOW_OUT_M * dir[0],
                    rider[1] + FOLLOW_UP_M,
                    rider[2] + FOLLOW_OUT_M * dir[1],
                ];
                let d = sub(target, self.pos);
                let dist = len(d);
                // Eases in near the target, at most the top speed.
                let speed = (dist * 0.8).min(MAX_SPEED_MPS);
                let step = (speed * dt).min(dist);
                let dir = norm(d);
                let p = [
                    self.pos[0] + dir[0] * step,
                    self.pos[1] + dir[1] * step,
                    self.pos[2] + dir[2] * step,
                ];
                self.pos = Self::clear(scene, p, rider, 1.0);
                self.look = to_rider(self.pos);
                if eta_s.is_some_and(|e| e <= APPROACH_S) || eta_s.is_none() {
                    self.start(DronePhase::Approach, t);
                }
            }
            DronePhase::Approach => {
                let u = match eta_s {
                    Some(e) => (1.0 - e / APPROACH_S).clamp(0.0, 1.0),
                    None => 1.0,
                };
                self.u = self.u.max(u);
                if self.u >= 1.0 {
                    self.pos = self.pose.eye;
                    self.look = self.pose.look;
                    self.start(DronePhase::Photo, t);
                    shot = true;
                } else {
                    let s = smoothstep(self.u);
                    let mut p = lerp3(self.from, self.pose.eye, s);
                    // An arc over what is between.
                    p[1] += 3.0 * (std::f64::consts::PI * s).sin();
                    let weight = 1.0 - smoothstep((self.u - 0.8) / 0.2);
                    self.pos = Self::clear(scene, p, rider, weight);
                    let l = lerp3(self.from_look, self.pose.look, smoothstep(self.u * 1.25));
                    self.look = norm(lerp3(to_rider(self.pos), l, s));
                }
            }
            DronePhase::Photo => {
                self.pos = self.pose.eye;
                self.look = self.pose.look;
                if t - self.since >= HOLD_S {
                    self.start(DronePhase::Return, t);
                }
            }
            DronePhase::Return => {
                let above = [self.home[0], self.home[1] + CLIMB_M, self.home[2]];
                let d = sub(above, self.pos);
                let dist = len(d);
                let step = (MAX_SPEED_MPS * 0.6 * dt).min(dist);
                let dir = norm(d);
                let p = [
                    self.pos[0] + dir[0] * step,
                    self.pos[1] + dir[1] * step,
                    self.pos[2] + dir[2] * step,
                ];
                self.pos = Self::clear(scene, p, rider, 1.0);
                if dist < 0.3 {
                    self.pos = above;
                    self.start(DronePhase::Landing, t);
                }
            }
            DronePhase::Landing => {
                let u = (t - self.since) / LAND_S;
                self.pos = lerp3(self.from, self.home, smoothstep(u));
                if u >= 1.0 {
                    self.pos = self.home;
                    self.start(DronePhase::Landed, t);
                }
            }
            DronePhase::Landed => {}
        }
        let rotor = match self.phase {
            DronePhase::Waiting | DronePhase::Landed => 0.0,
            DronePhase::Climb => ((t - self.since) / 1.0).clamp(0.3, 1.0),
            DronePhase::Landing => 1.0 - 0.6 * ((t - self.since) / LAND_S).clamp(0.0, 1.0),
            _ => 1.0,
        };
        let velocity = if dt > 0.0 {
            [
                (self.pos[0] - before[0]) / dt,
                (self.pos[1] - before[1]) / dt,
                (self.pos[2] - before[2]) / dt,
            ]
        } else {
            [0.0; 3]
        };
        DroneState {
            pos: self.pos,
            look: self.look,
            phase: self.phase,
            rotor,
            shot,
            velocity,
        }
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
            let s = f.step(t, rider, eta, &scene);
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
            let s = f.step(t, rider, Some(z / 8.0), &scene);
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
}
