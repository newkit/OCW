//! The client's side of god mode (spec §14 Phase 13), Bevy-free: the state
//! machine from riding to flying and back, the flight (client-driven,
//! [`ocw_sim::god::Flight`]), the poses sent to the server at 10 Hz, the
//! glides of leaving, and the other riders' unicorns (interpolated between
//! snapshots, §6.5).
//!
//! The server stays the truth for *who* flies and *where* a rider re-enters
//! the road: the client asks ([`GodCore::request_enter`],
//! [`GodCore::request_leave`]) and follows the answer ([`GodCore::on_result`]);
//! when the server ends god mode by itself (an event's start formation),
//! the client lands where the server put the rider.

use std::collections::BTreeMap;

use ocw_core::RiderId;
use ocw_protocol::v1;
use ocw_protocol::{encode, god_pose_to_wire};
use ocw_sim::god::{self, Bounds, Flight, FlightInput, Glide, GlideKind, GodPose};

/// Where the client is in god mode.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Phase {
    Riding,
    /// Asked the server to enter (at this time); waiting for its answer.
    Asking(f64),
    /// Flying since this time (the transformation runs for the first
    /// [`god::TRANSFORM_S`]).
    Flying {
        since: f64,
    },
    /// Gliding down to the landing spot or back to the take-off spot; the
    /// server has (or will have) put the rider there.
    Leaving {
        glide: Glide,
        since: f64,
        answered: bool,
    },
}

/// How long the client waits for the server's answer (s) before it gives up.
pub const ANSWER_TIMEOUT_S: f64 = 3.0;

/// The client's god mode.
#[derive(Debug, Clone)]
pub struct GodCore {
    phase: Phase,
    flight: Flight,
    /// Where the flight began (the unicorn's start pose).
    takeoff: GodPose,
    first_person: bool,
    seq: u32,
    last_pose_s: Option<f64>,
    /// Where leaving would land now (the glowing ring).
    landing: Option<[f64; 3]>,
    /// The unicorn's speed scale survives flights.
    scale: f64,
    /// Climb rate (m/s) of the last step, for the wings.
    climb_mps: f64,
    /// Height above the ground of the last step (m).
    height_m: f64,
    /// When the own rider's snapshot last said "riding" while flying.
    server_riding_since: Option<f64>,
    /// A pose to send at once (a dev teleport).
    teleport: bool,
}

impl Default for GodCore {
    fn default() -> Self {
        Self {
            phase: Phase::Riding,
            flight: Flight::new([0.0; 3], 0.0),
            takeoff: GodPose::default(),
            first_person: false,
            seq: 0,
            last_pose_s: None,
            landing: None,
            scale: 1.0,
            climb_mps: 0.0,
            height_m: 0.0,
            server_riding_since: None,
            teleport: false,
        }
    }
}

/// What the god mode wants the rest of the client to know after a step or
/// an answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GodEvent {
    /// The rider turned into the unicorn (the chime, the pointer capture).
    Entered,
    /// Back on the road (the rider shows again).
    Landed,
    /// Refused or failed: why.
    Refused(String),
}

impl GodCore {
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// Flying or gliding (the riding keys, the prediction and the chase
    /// camera are off).
    pub fn active(&self) -> bool {
        matches!(self.phase, Phase::Flying { .. } | Phase::Leaving { .. })
    }

    /// Flying (the flight keys move the unicorn).
    pub fn flying(&self) -> bool {
        matches!(self.phase, Phase::Flying { .. })
    }

    pub fn flight(&self) -> &Flight {
        &self.flight
    }

    pub fn first_person(&self) -> bool {
        self.first_person
    }

    pub fn toggle_first_person(&mut self) -> bool {
        self.first_person = !self.first_person;
        self.first_person
    }

    pub fn landing(&self) -> Option<[f64; 3]> {
        self.landing
    }

    pub fn set_landing(&mut self, spot: Option<[f64; 3]>) {
        self.landing = spot;
    }

    pub fn height_m(&self) -> f64 {
        self.height_m
    }

    /// Asks the server to enter god mode; `None` when not riding.
    pub fn request_enter(&mut self, now_s: f64) -> Option<Vec<u8>> {
        if self.phase != Phase::Riding {
            return None;
        }
        self.phase = Phase::Asking(now_s);
        Some(encode(&ocw_protocol::god_enter()))
    }

    /// Leaves god mode: glides down to the landing spot (`fly_back`: back
    /// to the take-off spot) and asks the server to place the rider there.
    /// `None` when not flying.
    pub fn request_leave(&mut self, fly_back: bool, now_s: f64) -> Option<Vec<u8>> {
        if !self.flying() {
            return None;
        }
        let here = self.flight.pos;
        let (kind, to) = if fly_back {
            (GlideKind::FlyBack, self.takeoff.pos)
        } else {
            // Down to the ring; without one (no road known) straight down.
            let spot = self
                .landing
                .unwrap_or([here[0], here[1] - self.height_m, here[2]]);
            (GlideKind::Land, spot)
        };
        self.phase = Phase::Leaving {
            glide: Glide::new(kind, here, to),
            since: now_s,
            answered: false,
        };
        Some(encode(&ocw_protocol::god_leave(fly_back, here[0], here[2])))
    }

    /// The server's answer.
    pub fn on_result(&mut self, r: &v1::GodResult, now_s: f64) -> Option<GodEvent> {
        match (self.phase, r.ok, r.active) {
            (Phase::Asking(_), true, true) => {
                let start = r
                    .start
                    .as_ref()
                    .map(ocw_protocol::god_pose_from_wire)
                    .unwrap_or_default();
                self.takeoff = start;
                self.flight = Flight::new(start.pos, start.yaw);
                self.flight.scale = self.scale;
                self.phase = Phase::Flying { since: now_s };
                self.last_pose_s = None;
                self.server_riding_since = None;
                Some(GodEvent::Entered)
            }
            (Phase::Asking(_), _, _) => {
                self.phase = Phase::Riding;
                Some(GodEvent::Refused(r.message.clone()))
            }
            (
                Phase::Leaving {
                    mut glide, since, ..
                },
                true,
                false,
            ) => {
                // Where the server placed the rider (the road surface).
                glide.retarget([r.x_m, r.y_m, r.z_m]);
                self.phase = Phase::Leaving {
                    glide,
                    since,
                    answered: true,
                };
                None
            }
            (Phase::Leaving { .. }, false, true) => {
                // The server keeps us flying (should not happen): fly on.
                self.phase = Phase::Flying {
                    since: now_s - 10.0,
                };
                Some(GodEvent::Refused(r.message.clone()))
            }
            (Phase::Leaving { glide, since, .. }, false, false) => {
                // Not in god mode on the server any more: just come down.
                self.phase = Phase::Leaving {
                    glide,
                    since,
                    answered: true,
                };
                Some(GodEvent::Refused(r.message.clone()))
            }
            _ => None,
        }
    }

    /// The own rider's mode from a snapshot. When the server ended god
    /// mode by itself (an event's start formation, the session switched)
    /// for more than a second while the client flies, the client follows.
    pub fn on_own_snapshot(&mut self, god_on_server: bool, now_s: f64) -> Option<GodEvent> {
        if !self.flying() || god_on_server {
            self.server_riding_since = None;
            return None;
        }
        let since = *self.server_riding_since.get_or_insert(now_s);
        if now_s - since >= 1.0 {
            self.phase = Phase::Riding;
            self.server_riding_since = None;
            return Some(GodEvent::Landed);
        }
        None
    }

    /// One frame: the flight (flying), the glide (leaving), timeouts.
    /// `look` is the view's turn (yaw, pitch; rad), `wheel` the wheel steps.
    #[allow(clippy::too_many_arguments)]
    pub fn step(
        &mut self,
        input: &FlightInput,
        look: (f64, f64),
        wheel: f64,
        dt: f64,
        now_s: f64,
        surface: &dyn Fn(f64, f64) -> Option<f64>,
        bounds: Option<Bounds>,
    ) -> Option<GodEvent> {
        match &mut self.phase {
            Phase::Riding => None,
            Phase::Asking(at) => {
                if now_s - *at > ANSWER_TIMEOUT_S {
                    self.phase = Phase::Riding;
                    return Some(GodEvent::Refused("the server did not answer".into()));
                }
                None
            }
            Phase::Flying { .. } => {
                self.flight.look(look.0, look.1);
                if wheel != 0.0 {
                    self.flight.scroll(wheel);
                    self.scale = self.flight.scale;
                }
                let y = self.flight.pos[1];
                let info = self.flight.step(input, dt, surface, bounds);
                self.climb_mps = if dt > 0.0 {
                    (self.flight.pos[1] - y) / dt
                } else {
                    0.0
                };
                self.height_m = info.height_m;
                None
            }
            Phase::Leaving {
                glide,
                since,
                answered,
            } => {
                let y = glide.position()[1];
                glide.advance(dt);
                self.climb_mps = if dt > 0.0 {
                    (glide.position()[1] - y) / dt
                } else {
                    0.0
                };
                if glide.done() && (*answered || now_s - *since > ANSWER_TIMEOUT_S) {
                    self.phase = Phase::Riding;
                    self.landing = None;
                    return Some(GodEvent::Landed);
                }
                None
            }
        }
    }

    /// The pose to send now, if one is due (10 Hz while flying).
    pub fn pose_message(&mut self, now_s: f64) -> Option<Vec<u8>> {
        if !self.flying() {
            return None;
        }
        let due = self.teleport
            || self
                .last_pose_s
                .is_none_or(|t| now_s - t >= 1.0 / god::POSE_HZ - 1e-6);
        if !due {
            return None;
        }
        self.last_pose_s = Some(now_s);
        self.seq += 1;
        let teleport = std::mem::take(&mut self.teleport);
        Some(encode(&ocw_protocol::client_message(
            v1::client_message::Msg::GodPose(god_pose_to_wire(
                &self.flight.pose(),
                self.seq,
                teleport,
            )),
        )))
    }

    /// Moves the unicorn (dev command `god pose`, scenes): sent at once as
    /// a jump the server only accepts on a local server.
    pub fn teleport(&mut self, pos: [f64; 3], yaw: Option<f64>, pitch: Option<f64>) -> bool {
        if !self.flying() {
            return false;
        }
        self.flight.pos = pos;
        self.flight.vel = [0.0; 3];
        if let Some(y) = yaw {
            self.flight.yaw = y;
        }
        if let Some(p) = pitch {
            self.flight.pitch = p.clamp(-god::MAX_PITCH, god::MAX_PITCH);
        }
        self.teleport = true;
        true
    }

    /// Where the unicorn is drawn now and how it moves (flying or
    /// gliding), with the seconds since the transformation began.
    pub fn unicorn(&self, now_s: f64) -> Option<LocalUnicorn> {
        match self.phase {
            Phase::Flying { since } => Some(LocalUnicorn {
                pose: self.flight.pose(),
                climb_mps: self.climb_mps,
                turn_rate: self.flight.turn_rate,
                accel: self.flight.accel,
                age_s: now_s - since,
                leaving: false,
            }),
            Phase::Leaving { glide, since, .. } => {
                let pos = glide.position();
                let yaw = glide.direction().map_or(self.flight.yaw, god::yaw_of);
                let speed = if glide.duration_s > 0.0 && !glide.done() {
                    let d: f64 = (0..3).map(|k| (glide.to[k] - glide.from[k]).powi(2)).sum();
                    d.sqrt() / glide.duration_s
                } else {
                    0.0
                };
                Some(LocalUnicorn {
                    pose: GodPose {
                        pos,
                        yaw,
                        pitch: self.flight.pitch.min(0.0),
                        speed,
                    },
                    climb_mps: self.climb_mps,
                    turn_rate: 0.0,
                    accel: 0.0,
                    age_s: (now_s - since) + 10.0,
                    leaving: true,
                })
            }
            _ => None,
        }
    }
}

/// The local unicorn now.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LocalUnicorn {
    pub pose: GodPose,
    pub climb_mps: f64,
    pub turn_rate: f64,
    pub accel: f64,
    pub age_s: f64,
    pub leaving: bool,
}

/// Another rider's unicorn: the last two poses from the snapshots
/// (arrival time, pose) and when it appeared.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OtherGod {
    pub prev: (f64, GodPose),
    pub last: (f64, GodPose),
    pub since: f64,
}

/// The other riders' unicorns, from the snapshots.
#[derive(Debug, Clone, Default)]
pub struct OtherGods(pub BTreeMap<RiderId, OtherGod>);

impl OtherGods {
    /// A snapshot's riders in god mode (`(id, pose)`, the own excluded),
    /// received at `now_s`: the others are forgotten.
    pub fn on_snapshot(&mut self, gods: &[(RiderId, GodPose)], now_s: f64) {
        self.0.retain(|id, _| gods.iter().any(|(g, _)| g == id));
        for &(id, pose) in gods {
            self.0
                .entry(id)
                .and_modify(|o| {
                    o.prev = o.last;
                    o.last = (now_s, pose);
                })
                .or_insert(OtherGod {
                    prev: (now_s, pose),
                    last: (now_s, pose),
                    since: now_s,
                });
        }
    }

    /// Where each is at `now_s` (interpolated between the last two poses,
    /// a little in the past, §6.5), with its climb and turn rates.
    pub fn sample(&self, now_s: f64, delay_s: f64) -> Vec<(RiderId, LocalUnicorn)> {
        self.0
            .iter()
            .map(|(&id, o)| {
                let (t0, a) = o.prev;
                let (t1, b) = o.last;
                let span = (t1 - t0).max(1e-3);
                let u = if t1 > t0 {
                    ((now_s - delay_s - t0) / span).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                let pos: [f64; 3] = std::array::from_fn(|k| a.pos[k] + (b.pos[k] - a.pos[k]) * u);
                let yaw = a.yaw + god::wrap_angle(b.yaw - a.yaw) * u;
                let pose = GodPose {
                    pos,
                    yaw,
                    pitch: a.pitch + (b.pitch - a.pitch) * u,
                    speed: a.speed + (b.speed - a.speed) * u,
                };
                let dt = if t1 > t0 { span } else { 1.0 };
                (
                    id,
                    LocalUnicorn {
                        pose,
                        climb_mps: if t1 > t0 {
                            (b.pos[1] - a.pos[1]) / dt
                        } else {
                            0.0
                        },
                        turn_rate: if t1 > t0 {
                            god::wrap_angle(b.yaw - a.yaw) / dt
                        } else {
                            0.0
                        },
                        accel: if t1 > t0 {
                            (b.speed - a.speed) / dt
                        } else {
                            0.0
                        },
                        age_s: now_s - o.since,
                        leaving: false,
                    },
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_protocol::decode_client;
    use v1::client_message::Msg as C;

    fn entered(at: [f64; 3]) -> GodCore {
        let mut g = GodCore::default();
        assert!(g.request_enter(0.0).is_some());
        assert!(g.request_enter(0.0).is_none(), "once");
        let r = v1::GodResult {
            ok: true,
            active: true,
            start: Some(god_pose_to_wire(
                &GodPose {
                    pos: at,
                    ..Default::default()
                },
                0,
                false,
            )),
            ..Default::default()
        };
        assert_eq!(g.on_result(&r, 0.0), Some(GodEvent::Entered));
        g
    }

    const FLAT: fn(f64, f64) -> Option<f64> = |_, _| Some(0.0);

    #[test]
    fn enter_fly_send_poses_and_land() {
        let mut g = entered([0.0, 5.0, 0.0]);
        assert!(g.flying() && g.active());
        let ahead = FlightInput {
            forward: 1.0,
            ..Default::default()
        };
        let mut sent = 0;
        let mut t = 0.0;
        for _ in 0..60 {
            t += 1.0 / 60.0;
            g.step(&ahead, (0.0, 0.0), 0.0, 1.0 / 60.0, t, &FLAT, None);
            if let Some(m) = g.pose_message(t) {
                let C::GodPose(p) = decode_client(&m).unwrap() else {
                    panic!("a pose")
                };
                assert_eq!(p.seq, sent + 1);
                sent += 1;
            }
        }
        assert!((9..=11).contains(&sent), "10 Hz: {sent}");
        assert!(g.flight().pos[2] < -5.0, "flew north");
        // The unicorn shows with its transformation age.
        assert!((g.unicorn(t).unwrap().age_s - t).abs() < 1e-9);
        // G: glide down to the ring, ask the server.
        g.set_landing(Some([0.0, 0.0, -20.0]));
        let m = g.request_leave(false, t).unwrap();
        let C::God(v1::GodRequest {
            req: Some(v1::god_request::Req::Leave(l)),
        }) = decode_client(&m).unwrap()
        else {
            panic!("a leave")
        };
        assert!(!l.fly_back);
        assert!(g.pose_message(t + 1.0).is_none(), "no poses while leaving");
        assert!(g.unicorn(t).unwrap().leaving);
        // The server put the rider 2 m further.
        g.on_result(
            &v1::GodResult {
                ok: true,
                active: false,
                z_m: -22.0,
                ..Default::default()
            },
            t,
        );
        let mut landed = None;
        for _ in 0..200 {
            t += 1.0 / 60.0;
            if let Some(e) = g.step(&ahead, (0.0, 0.0), 0.0, 1.0 / 60.0, t, &FLAT, None) {
                landed = Some((e, g.phase()));
                break;
            }
            assert!(g.unicorn(t).unwrap().pose.pos[1] >= -1e-9);
        }
        assert_eq!(landed, Some((GodEvent::Landed, Phase::Riding)));
        assert!(!g.active());
    }

    #[test]
    fn shift_g_flies_back_to_the_take_off_spot() {
        let mut g = entered([100.0, 5.0, 100.0]);
        g.teleport([5_000.0, 900.0, -3_000.0], Some(1.0), None);
        let m = g.pose_message(0.01).unwrap();
        let C::GodPose(p) = decode_client(&m).unwrap() else {
            panic!()
        };
        assert!(p.teleport);
        g.request_leave(true, 1.0).unwrap();
        let Phase::Leaving { glide, .. } = g.phase() else {
            panic!()
        };
        assert_eq!(glide.kind, GlideKind::FlyBack);
        assert_eq!(glide.to, [100.0, 5.0, 100.0]);
        assert!(glide.duration_s <= god::MAX_GLIDE_S);
    }

    #[test]
    fn refusals_and_server_side_ends() {
        let mut g = GodCore::default();
        g.request_enter(0.0);
        let e = g.on_result(
            &v1::GodResult {
                ok: false,
                message: "not during a running event".into(),
                ..Default::default()
            },
            0.1,
        );
        assert_eq!(
            e,
            Some(GodEvent::Refused("not during a running event".into()))
        );
        assert_eq!(g.phase(), Phase::Riding);
        // No answer: give up.
        g.request_enter(1.0);
        let e = g.step(
            &FlightInput::default(),
            (0.0, 0.0),
            0.0,
            0.1,
            1.0 + ANSWER_TIMEOUT_S + 0.1,
            &FLAT,
            None,
        );
        assert!(matches!(e, Some(GodEvent::Refused(_))));
        // The server ended it (an event started): the client follows
        // after a second.
        let mut g = entered([0.0, 5.0, 0.0]);
        assert_eq!(g.on_own_snapshot(false, 10.0), None);
        assert_eq!(g.on_own_snapshot(true, 10.5), None);
        assert_eq!(g.on_own_snapshot(false, 11.0), None);
        assert_eq!(g.on_own_snapshot(false, 12.1), Some(GodEvent::Landed));
        assert!(!g.active());
    }

    #[test]
    fn the_wheel_scale_survives_flights() {
        let mut g = entered([0.0, 5.0, 0.0]);
        g.step(
            &FlightInput::default(),
            (0.0, 0.0),
            4.0,
            0.016,
            0.1,
            &FLAT,
            None,
        );
        let s = g.flight().scale;
        assert!(s > 2.0);
        g.request_leave(true, 0.2);
        for k in 0..300 {
            g.step(
                &FlightInput::default(),
                (0.0, 0.0),
                0.0,
                0.02,
                0.2 + k as f64 * 0.02,
                &FLAT,
                None,
            );
        }
        let g2 = {
            let mut g2 = g.clone();
            g2.request_enter(10.0);
            g2.on_result(
                &v1::GodResult {
                    ok: true,
                    active: true,
                    ..Default::default()
                },
                10.0,
            );
            g2
        };
        assert_eq!(g2.flight().scale, s);
    }

    #[test]
    fn other_unicorns_are_interpolated() {
        let mut o = OtherGods::default();
        let at = |x: f64| GodPose {
            pos: [x, 100.0, 0.0],
            ..Default::default()
        };
        o.on_snapshot(&[(RiderId(2), at(0.0))], 0.0);
        o.on_snapshot(&[(RiderId(2), at(10.0))], 0.1);
        let s = o.sample(0.15, 0.1);
        assert_eq!(s.len(), 1);
        assert!((s[0].1.pose.pos[0] - 5.0).abs() < 1e-6);
        assert!((s[0].1.age_s - 0.15).abs() < 1e-9);
        // Landed: gone.
        o.on_snapshot(&[], 0.2);
        assert!(o.sample(0.3, 0.1).is_empty());
    }
}
