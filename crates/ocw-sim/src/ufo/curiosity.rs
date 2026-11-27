//! The unicorn's curiosity rule (spec 18C *UFOs*, Q7 (a), 18E T9): in god
//! mode, now and then a saucer comes to look at the unicorn – it appears
//! far off, comes closer, circles it, follows it a while at a distance
//! with its rim lights flashing in the mane's colours, and leaves in a
//! sudden dart; flown straight at, it zips away at once.
//!
//! **When:** the world clock is cut into slots ([`slot_s`]: 10 minutes on
//! *rare*, 3 on *often*); a slot has a visit by a hash of the air seed
//! and the slot ([`due`]), at a moment within its first 60 %. A visit
//! starts when that moment has come, the unicorn has flown for
//! [`SETTLE_S`] and no visit is on; a moment more than a slot old is let
//! go (the time machine jumping). The phases: coming closer (at most
//! [`APPROACH_MAX_S`]), circling ([`CIRCLE_S`]), following ([`FOLLOW_S`]),
//! away ([`super::AWAY_S`]).
//!
//! **Never touching:** the saucer keeps [`KEEP_M`] from the unicorn at
//! every step. It moves in fixed steps of [`STEP_S`] whatever the frame
//! rate.

use std::f64::consts::TAU;

use ocw_core::traffic::Ufos;

use super::{Draw, Lights, SightingKind, UfoNow, mix};

/// The unicorn flies this long before a visit starts (s).
pub const SETTLE_S: f64 = 15.0;
/// The saucer's least distance from the unicorn (m).
pub const KEEP_M: f64 = 20.0;
/// Coming closer takes at most this long (s).
pub const APPROACH_MAX_S: f64 = 25.0;
/// Circling and following last (s, from … to by the visit's hash).
pub const CIRCLE_S: (f64, f64) = (25.0, 45.0);
pub const FOLLOW_S: (f64, f64) = (25.0, 45.0);
/// Where it appears (m from the unicorn).
pub const APPEAR_M: f64 = 700.0;
/// The fixed step (s).
pub const STEP_S: f64 = 0.02;
/// Flying straight at it: within this distance (m), this fast toward it
/// (m/s) and within this angle (cos) of its direction, for this long (s).
pub const CHASE_M: f64 = 150.0;
pub const CHASE_MPS: f64 = 8.0;
pub const CHASE_COS: f64 = 0.985;
pub const CHASE_S: f64 = 1.0;
/// Circling: the angular speed round the unicorn (rad/s) – it crosses
/// the unicorn's path ahead faster than a chase is noticed.
pub const CIRCLE_RAD_S: f64 = 0.5;

/// A visit's slot length by the setting (s; `None`: no visits).
pub fn slot_s(level: Ufos) -> Option<f64> {
    match level {
        Ufos::Off => None,
        Ufos::Rare => Some(600.0),
        Ufos::Often => Some(180.0),
    }
}

/// The chance of a slot's visit.
fn chance(level: Ufos) -> f64 {
    match level {
        Ufos::Off => 0.0,
        Ufos::Rare => 0.5,
        Ufos::Often => 0.85,
    }
}

/// When slot `k`'s visit is due (s), if it has one.
pub fn due(seed: u64, level: Ufos, k: i64) -> Option<f64> {
    let len = slot_s(level)?;
    let mut d = Draw::new(mix(seed ^ 0xC0_210 ^ mix(k as u64)));
    (d.f() < chance(level)).then(|| k as f64 * len + d.range(0.0, 0.6) * len)
}

/// The unicorn now.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Unicorn {
    pub pos: [f64; 3],
    pub vel: [f64; 3],
}

/// The phase of a visit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Approach,
    Circle,
    Follow,
    Leave,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Visit {
    id: u64,
    slot: i64,
    phase: Phase,
    since: f64,
    circle_s: f64,
    follow_s: f64,
    radius: f64,
    /// The angle round the unicorn and which way it circles.
    angle: f64,
    turn: f64,
    side: f64,
    pos: [f64; 3],
    vel: [f64; 3],
    away_from: [f64; 3],
    away_dir: [f64; 3],
    chased_s: f64,
    /// Why it left: chased away.
    chased: bool,
}

/// The unicorn's visitor: steps with the clock and the unicorn.
#[derive(Debug, Clone, Default)]
pub struct Curiosity {
    visit: Option<Visit>,
    flying_since: Option<f64>,
    last: Option<f64>,
    /// The slots whose visit came (or was let go).
    done: Vec<i64>,
    /// Visits made, and how many were chased away (the `ufo` command).
    pub visits: u32,
    pub chased: u32,
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn len(a: [f64; 3]) -> f64 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

fn add(a: [f64; 3], b: [f64; 3], s: f64) -> [f64; 3] {
    [a[0] + b[0] * s, a[1] + b[1] * s, a[2] + b[2] * s]
}

impl Curiosity {
    /// The phase of the visit on (the `ufo` command, tests).
    pub fn phase(&self) -> Option<Phase> {
        self.visit.map(|v| v.phase)
    }

    /// Start a visit now (the `ufo.visit` command: scenes and videos).
    pub fn start_now(&mut self, seed: u64, now: f64, unicorn: &Unicorn) {
        self.begin(seed, -1 - i64::from(self.visits), now, unicorn);
    }

    fn begin(&mut self, seed: u64, slot: i64, now: f64, u: &Unicorn) {
        let mut d = Draw::new(mix(seed ^ 0x0007_1517 ^ mix(slot as u64)));
        let a = d.range(0.0, TAU);
        let pos = [
            u.pos[0] + APPEAR_M * a.cos(),
            u.pos[1] + d.range(80.0, 200.0),
            u.pos[2] + APPEAR_M * a.sin(),
        ];
        self.visit = Some(Visit {
            id: mix(seed ^ mix(slot as u64 ^ 0x000F_1517)),
            slot,
            phase: Phase::Approach,
            since: now,
            circle_s: d.range(CIRCLE_S.0, CIRCLE_S.1),
            follow_s: d.range(FOLLOW_S.0, FOLLOW_S.1),
            radius: d.range(32.0, 45.0),
            angle: a,
            turn: if d.f() < 0.5 { 1.0 } else { -1.0 },
            side: if d.f() < 0.5 { 1.0 } else { -1.0 },
            pos,
            vel: [0.0; 3],
            away_from: pos,
            away_dir: [0.0, 1.0, 0.0],
            chased_s: 0.0,
            chased: false,
        });
        self.visits += 1;
        self.done.push(slot);
        if self.done.len() > 64 {
            self.done.remove(0);
        }
    }

    /// One frame at `now`: `unicorn` is the local rider's unicorn (`None`:
    /// riding – no visits, and one on leaves). Returns the visitor.
    pub fn step(
        &mut self,
        seed: u64,
        level: Ufos,
        now: f64,
        unicorn: Option<&Unicorn>,
    ) -> Option<UfoNow> {
        let last = self.last.replace(now);
        let Some(u) = unicorn.filter(|_| level != Ufos::Off) else {
            self.flying_since = None;
            self.visit = None;
            return None;
        };
        let since = *self.flying_since.get_or_insert(now);
        // A clock jump: no catching up over it.
        let dt_all = match last {
            Some(l) if (0.0..5.0).contains(&(now - l)) => now - l,
            _ => 0.0,
        };
        if self.visit.is_none() && now - since >= SETTLE_S {
            let len = slot_s(level).expect("on");
            let k = (now / len).floor() as i64;
            for k in [k - 1, k] {
                if self.done.contains(&k) {
                    continue;
                }
                if let Some(t) = due(seed, level, k)
                    && t <= now
                    && now - t < len
                {
                    self.begin(seed, k, now, u);
                    break;
                }
            }
        }
        let v = self.visit.as_mut()?;
        let steps = (dt_all / STEP_S).round() as usize;
        let h = if steps > 0 {
            dt_all / steps as f64
        } else {
            0.0
        };
        let mut t = now - dt_all;
        for _ in 0..steps {
            t += h;
            Self::advance(v, t, h, u, &mut self.chased);
        }
        if v.phase == Phase::Leave && now - v.since >= super::AWAY_S {
            self.visit = None;
            return None;
        }
        Some(Self::show(v, now))
    }

    fn show(v: &Visit, now: f64) -> UfoNow {
        let presence = match v.phase {
            Phase::Approach => ((now - v.since) / 2.5).clamp(0.0, 1.0),
            Phase::Leave => ((super::AWAY_S - (now - v.since)) / 0.6).clamp(0.0, 1.0),
            _ => 1.0,
        } as f32;
        UfoNow {
            id: v.id,
            kind: SightingKind::Visitor,
            pos: v.pos,
            vel: v.vel,
            spin: (now * if v.phase == Phase::Leave { 6.0 } else { 1.1 }).rem_euclid(TAU),
            presence,
            beam: 0.0,
            beam_ground_m: v.pos[1],
            lights: match v.phase {
                Phase::Circle | Phase::Follow => Lights::Mane,
                _ => Lights::Cycle,
            },
            departing: v.phase == Phase::Leave,
        }
    }

    fn leave(v: &mut Visit, t: f64, u: &Unicorn) {
        let off = sub(v.pos, u.pos);
        let l = len(off).max(1.0);
        // Away from the unicorn and up.
        let mut dir = [off[0] / l * 0.6, 0.8, off[2] / l * 0.6];
        let n = len(dir);
        dir = [dir[0] / n, dir[1] / n, dir[2] / n];
        v.phase = Phase::Leave;
        v.since = t;
        v.away_from = v.pos;
        v.away_dir = dir;
    }

    fn advance(v: &mut Visit, t: f64, h: f64, u: &Unicorn, chased_count: &mut u32) {
        let elapsed = t - v.since;
        let prev = v.pos;
        match v.phase {
            Phase::Leave => {
                let f = (elapsed / super::AWAY_S).clamp(0.0, 1.0);
                v.pos = add(v.away_from, v.away_dir, 4000.0 * f.powi(3));
            }
            _ => {
                let up = 8.0 + 4.0 * (0.5 * v.angle).sin();
                let target = match v.phase {
                    Phase::Approach | Phase::Circle => {
                        if v.phase == Phase::Circle {
                            v.angle += v.turn * CIRCLE_RAD_S * h;
                        }
                        [
                            u.pos[0] + v.radius * v.angle.cos(),
                            u.pos[1] + up,
                            u.pos[2] + v.radius * v.angle.sin(),
                        ]
                    }
                    _ => {
                        // Behind, above and to one side, lagging.
                        let flat = u.vel[0].hypot(u.vel[2]);
                        let back = if flat > 1.0 {
                            [-u.vel[0] / flat, -u.vel[2] / flat]
                        } else {
                            [v.angle.cos(), v.angle.sin()]
                        };
                        let side = [-back[1] * v.side, back[0] * v.side];
                        [
                            u.pos[0] + back[0] * 45.0 + side[0] * 12.0,
                            u.pos[1] + 15.0,
                            u.pos[2] + back[1] * 45.0 + side[1] * 12.0,
                        ]
                    }
                };
                // Chase the target: quick but smooth, at most 120 m/s
                // (60 on the way in).
                let to = sub(target, v.pos);
                let d = len(to);
                let max = if v.phase == Phase::Approach {
                    60.0
                } else {
                    120.0
                };
                let want = (d * 1.5).min(max);
                if d > 1e-6 {
                    v.pos = add(v.pos, to, (want * h / d).min(1.0));
                }
                // Never touching.
                let off = sub(v.pos, u.pos);
                let l = len(off);
                if l < KEEP_M {
                    let dir = if l > 1e-6 {
                        [off[0] / l, off[1] / l, off[2] / l]
                    } else {
                        [0.0, 1.0, 0.0]
                    };
                    v.pos = add(u.pos, dir, KEEP_M);
                }
                match v.phase {
                    Phase::Approach if d < 15.0 || elapsed >= APPROACH_MAX_S => {
                        v.phase = Phase::Circle;
                        v.since = t;
                    }
                    Phase::Circle if elapsed >= v.circle_s => {
                        v.phase = Phase::Follow;
                        v.since = t;
                    }
                    Phase::Follow if elapsed >= v.follow_s => Self::leave(v, t, u),
                    _ => {}
                }
                // Flown straight at: away at once.
                if matches!(v.phase, Phase::Circle | Phase::Follow) {
                    let off = sub(v.pos, u.pos);
                    let l = len(off).max(1e-6);
                    let speed = len(u.vel);
                    let toward = (u.vel[0] * off[0] + u.vel[1] * off[1] + u.vel[2] * off[2]) / l;
                    if l < CHASE_M && toward > CHASE_MPS && toward / speed.max(1e-6) > CHASE_COS {
                        v.chased_s += h;
                    } else {
                        v.chased_s = 0.0;
                    }
                    if v.chased_s >= CHASE_S {
                        v.chased = true;
                        *chased_count += 1;
                        Self::leave(v, t, u);
                    }
                }
            }
        }
        if h > 0.0 {
            v.vel = [
                (v.pos[0] - prev[0]) / h,
                (v.pos[1] - prev[1]) / h,
                (v.pos[2] - prev[2]) / h,
            ];
        }
    }
}
