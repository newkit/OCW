//! A flight's path (spec 18C *Flight paths*): waypoints with their speeds,
//! the corners rounded by the aircraft's turn radius, kept clear of the
//! ground ([`super::relief::Relief`]), and the position at any time –
//! the speed changes linearly along each piece (so the time on a piece is
//! exact: `L / (v1 - v0) · ln(v1 / v0)`), holds at a vertex (a helicopter
//! on its pad, a balloon before it lifts) stand still.

use super::relief::{CELL_M, Relief};

/// Gravity (m/s²) for the bank in a turn.
const G: f64 = 9.81;
/// The least speed of a vertex (m/s): a standing start still has a time.
const MIN_SPEED: f64 = 1.5;

/// A point the flight passes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Waypoint {
    /// x east, y up (m above the sea), z south (package metres).
    pub pos: [f64; 3],
    /// Speed here (m/s).
    pub speed: f64,
    /// Standing still here (s) before going on.
    pub hold_s: f64,
    /// On the ground (runway, pad, field): never lifted, no clearance.
    pub ground: bool,
    /// Kept as planned (a glide slope, a pad's last metres): never lifted.
    pub fixed: bool,
}

impl Waypoint {
    pub fn air(pos: [f64; 3], speed: f64) -> Self {
        Self {
            pos,
            speed,
            hold_s: 0.0,
            ground: false,
            fixed: false,
        }
    }

    pub fn fixed(pos: [f64; 3], speed: f64) -> Self {
        Self {
            fixed: true,
            ..Self::air(pos, speed)
        }
    }

    pub fn ground(pos: [f64; 3], speed: f64) -> Self {
        Self {
            ground: true,
            fixed: true,
            ..Self::air(pos, speed)
        }
    }

    pub fn hold(mut self, s: f64) -> Self {
        self.hold_s = s;
        self
    }
}

/// A vertex of the built path.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Vertex {
    pos: [f64; 3],
    speed: f64,
    hold_s: f64,
    ground: bool,
    fixed: bool,
    /// Arrival time from the start (s).
    t: f64,
}

/// Where a flight is at a time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PathState {
    pub pos: [f64; 3],
    /// m/s, x y z.
    pub vel: [f64; 3],
    /// Bank (rad, positive: right wing down, turning right).
    pub bank: f64,
    pub ground: bool,
}

/// See the module docs.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FlightPath {
    v: Vec<Vertex>,
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn lerp3(a: [f64; 3], b: [f64; 3], t: f64) -> [f64; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

fn len3(a: [f64; 3]) -> f64 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

fn len2(a: [f64; 3]) -> f64 {
    a[0].hypot(a[2])
}

/// Signed turn (rad) from horizontal direction `a` to `b` (positive:
/// clockwise seen from above, a right turn – x east, z south).
fn turn(a: [f64; 3], b: [f64; 3]) -> f64 {
    let cross = a[0] * b[2] - a[2] * b[0];
    let dot = a[0] * b[0] + a[2] * b[2];
    cross.atan2(dot)
}

/// Builds a path.
#[derive(Debug, Clone)]
pub struct PathBuilder {
    points: Vec<Waypoint>,
    radius_m: f64,
}

impl PathBuilder {
    /// Corners rounded with about `radius_m`.
    pub fn new(radius_m: f64) -> Self {
        Self {
            points: Vec::new(),
            radius_m,
        }
    }

    pub fn to(&mut self, w: Waypoint) -> &mut Self {
        self.points.push(w);
        self
    }

    pub fn last(&self) -> Option<&Waypoint> {
        self.points.last()
    }

    /// The path, at least `clearance_m` over `relief` where it is not
    /// fixed, climbing and descending at most `grade` (rise per metre)
    /// to get there.
    pub fn build(&self, relief: Option<&Relief>, clearance_m: f64, grade: f64) -> FlightPath {
        let mut v = self.rounded();
        if let Some(r) = relief.filter(|r| !r.is_empty()) {
            v = densify(v, CELL_M);
            clear(&mut v, r, clearance_m, grade);
        }
        timing(&mut v);
        FlightPath { v }
    }

    /// The waypoints with their corners rounded (quadratic arcs).
    fn rounded(&self) -> Vec<Vertex> {
        let p = &self.points;
        let vx = |w: &Waypoint| Vertex {
            pos: w.pos,
            speed: w.speed.max(MIN_SPEED),
            hold_s: w.hold_s,
            ground: w.ground,
            fixed: w.fixed,
            t: 0.0,
        };
        let mut out = Vec::with_capacity(p.len() * 4);
        for i in 0..p.len() {
            let w = &p[i];
            if i == 0 || i + 1 == p.len() || w.hold_s > 0.0 || w.ground {
                out.push(vx(w));
                continue;
            }
            let (a, b) = (sub(w.pos, p[i - 1].pos), sub(p[i + 1].pos, w.pos));
            let (la, lb) = (len2(a), len2(b));
            let th = if la > 1.0 && lb > 1.0 {
                turn(a, b).abs()
            } else {
                0.0
            };
            if th < 0.02 {
                out.push(vx(w));
                continue;
            }
            let r = self
                .radius_m
                .min(w.speed * w.speed / (G * 0.35f64.tan()) * 1.2);
            let d = (r * (th / 2.0).tan()).min(0.45 * la).min(0.45 * lb);
            let start = lerp3(w.pos, p[i - 1].pos, d / la);
            let end = lerp3(w.pos, p[i + 1].pos, d / lb);
            let s0 = w.speed + (p[i - 1].speed - w.speed) * d / la;
            let s1 = w.speed + (p[i + 1].speed - w.speed) * d / lb;
            let n = ((th / 0.2).ceil() as usize).clamp(2, 16);
            for k in 0..=n {
                let t = k as f64 / n as f64;
                let q0 = lerp3(start, w.pos, t);
                let q1 = lerp3(w.pos, end, t);
                out.push(Vertex {
                    pos: lerp3(q0, q1, t),
                    speed: (s0 + (s1 - s0) * t).max(MIN_SPEED),
                    hold_s: 0.0,
                    ground: false,
                    fixed: w.fixed && p[i - 1].fixed && p[i + 1].fixed,
                    t: 0.0,
                });
            }
        }
        out
    }
}

/// Pieces longer than `step` (not on the ground) split up.
fn densify(v: Vec<Vertex>, step: f64) -> Vec<Vertex> {
    let mut out = Vec::with_capacity(v.len() * 2);
    for i in 0..v.len() {
        if i > 0 {
            let (a, b) = (v[i - 1], v[i]);
            let l = len2(sub(b.pos, a.pos));
            let on_ground = a.ground && b.ground;
            if l > step && !on_ground {
                let n = (l / step).ceil() as usize;
                for k in 1..n {
                    let t = k as f64 / n as f64;
                    out.push(Vertex {
                        pos: lerp3(a.pos, b.pos, t),
                        speed: a.speed + (b.speed - a.speed) * t,
                        hold_s: 0.0,
                        ground: false,
                        fixed: a.fixed && b.fixed,
                        t: 0.0,
                    });
                }
            }
        }
        out.push(v[i]);
    }
    out
}

/// Lifts the free vertices to `clearance` over the relief, with ramps of
/// at most `grade` before and after.
fn clear(v: &mut [Vertex], relief: &Relief, clearance: f64, grade: f64) {
    for x in v.iter_mut() {
        if !x.fixed && !x.ground {
            let floor = relief.max_at(x.pos[0], x.pos[2]) + clearance;
            if x.pos[1] < floor {
                x.pos[1] = floor;
            }
        }
    }
    // Climb ahead of the high ground (backwards), descend after it.
    for i in (0..v.len().saturating_sub(1)).rev() {
        if v[i].fixed || v[i].ground {
            continue;
        }
        let d = len2(sub(v[i + 1].pos, v[i].pos));
        let need = v[i + 1].pos[1] - grade * d;
        if !v[i + 1].fixed && v[i].pos[1] < need {
            v[i].pos[1] = need;
        }
    }
    for i in 1..v.len() {
        if v[i].fixed || v[i].ground {
            continue;
        }
        let d = len2(sub(v[i].pos, v[i - 1].pos));
        let need = v[i - 1].pos[1] - grade * d;
        if !v[i - 1].fixed && v[i].pos[1] < need {
            v[i].pos[1] = need;
        }
    }
}

/// Time on a piece of length `l` from speed `a` to `b` (linear in distance).
fn piece_time(l: f64, a: f64, b: f64) -> f64 {
    if (b - a).abs() < 1e-6 {
        l / a
    } else {
        l / (b - a) * (b / a).ln()
    }
}

fn timing(v: &mut [Vertex]) {
    let mut t = 0.0;
    for i in 0..v.len() {
        if i > 0 {
            let l = len3(sub(v[i].pos, v[i - 1].pos));
            t += v[i - 1].hold_s + piece_time(l, v[i - 1].speed, v[i].speed);
        }
        v[i].t = t;
    }
}

impl FlightPath {
    /// How long the flight takes (s), holds included.
    pub fn duration_s(&self) -> f64 {
        self.v.last().map_or(0.0, |l| l.t + l.hold_s)
    }

    pub fn is_empty(&self) -> bool {
        self.v.len() < 2
    }

    /// The first and last point.
    pub fn ends(&self) -> Option<([f64; 3], [f64; 3])> {
        Some((self.v.first()?.pos, self.v.last()?.pos))
    }

    /// The vertices' places (tests, the map).
    pub fn points(&self) -> impl Iterator<Item = [f64; 3]> + '_ {
        self.v.iter().map(|v| v.pos)
    }

    /// Where the flight is `t` seconds after its start (clamped).
    pub fn at(&self, t: f64) -> PathState {
        let v = &self.v;
        if v.is_empty() {
            return PathState {
                pos: [0.0; 3],
                vel: [0.0; 3],
                bank: 0.0,
                ground: true,
            };
        }
        let t = t.clamp(0.0, self.duration_s());
        // The last vertex reached by `t`.
        let i = v.partition_point(|x| x.t <= t).saturating_sub(1);
        let a = v[i];
        if i + 1 >= v.len() || t < a.t + a.hold_s {
            let dir = if i + 1 < v.len() {
                sub(v[i + 1].pos, a.pos)
            } else if i > 0 {
                sub(a.pos, v[i - 1].pos)
            } else {
                [0.0, 0.0, -1.0]
            };
            let l = len3(dir).max(1e-9);
            let hold = t < a.t + a.hold_s || i + 1 >= v.len();
            let speed = if hold { 0.0 } else { a.speed };
            return PathState {
                pos: a.pos,
                vel: [dir[0] / l * speed, dir[1] / l * speed, dir[2] / l * speed],
                bank: 0.0,
                ground: a.ground,
            };
        }
        let b = v[i + 1];
        let tau = t - a.t - a.hold_s;
        let d = sub(b.pos, a.pos);
        let l = len3(d).max(1e-9);
        let k = (b.speed - a.speed) / l;
        let s = if k.abs() < 1e-9 {
            a.speed * tau
        } else {
            a.speed * ((k * tau).exp() - 1.0) / k
        }
        .clamp(0.0, l);
        let speed = a.speed + k * s;
        let u = [d[0] / l, d[1] / l, d[2] / l];
        // Bank from the turn rate between the neighbouring pieces.
        let bank = {
            let prev = (i > 0).then(|| sub(a.pos, v[i - 1].pos));
            let next = (i + 2 < v.len()).then(|| sub(v[i + 2].pos, b.pos));
            let f = s / l;
            let rate = |x: [f64; 3], y: [f64; 3], len: f64| {
                let dt = len / speed.max(MIN_SPEED);
                turn(x, y) / dt.max(0.1)
            };
            let w0 = prev
                .filter(|p| len2(*p) > 0.5)
                .map_or(0.0, |p| rate(p, d, 0.5 * (len3(p) + l)));
            let w1 = next
                .filter(|n| len2(*n) > 0.5)
                .map_or(0.0, |n| rate(d, n, 0.5 * (len3(n) + l)));
            let w = w0 * (1.0 - f) + w1 * f;
            if a.ground || b.ground {
                0.0
            } else {
                (speed * w / G).atan().clamp(-1.0, 1.0)
            }
        };
        PathState {
            pos: [
                a.pos[0] + u[0] * s,
                a.pos[1] + u[1] * s,
                a.pos[2] + u[2] * s,
            ],
            vel: [u[0] * speed, u[1] * speed, u[2] * speed],
            bank,
            ground: a.ground && b.ground,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_and_speeds_follow_the_waypoints() {
        let mut b = PathBuilder::new(500.0);
        b.to(Waypoint::air([0.0, 1000.0, 0.0], 50.0))
            .to(Waypoint::air([0.0, 1000.0, -10_000.0], 50.0));
        let p = b.build(None, 0.0, 0.1);
        assert!((p.duration_s() - 200.0).abs() < 1e-6);
        let s = p.at(100.0);
        assert!((s.pos[2] + 5000.0).abs() < 1e-6, "{s:?}");
        assert!((s.vel[2] + 50.0).abs() < 1e-6);
        // Accelerating: the exact time of a linear speed change.
        let mut b = PathBuilder::new(500.0);
        b.to(Waypoint::ground([0.0, 0.0, 0.0], 0.0))
            .to(Waypoint::ground([1000.0, 0.0, 0.0], 80.0));
        let p = b.build(None, 0.0, 0.1);
        let end = p.at(p.duration_s());
        assert!((end.pos[0] - 1000.0).abs() < 1e-6 && end.ground);
        let mid = p.at(p.duration_s() * 0.5);
        assert!(
            mid.pos[0] < 500.0,
            "accelerating: less than half way at half time"
        );
    }

    #[test]
    fn corners_are_rounded_and_banked_into_the_turn() {
        let mut b = PathBuilder::new(2000.0);
        b.to(Waypoint::air([0.0, 3000.0, 0.0], 100.0))
            .to(Waypoint::air([0.0, 3000.0, -10_000.0], 100.0))
            .to(Waypoint::air([10_000.0, 3000.0, -10_000.0], 100.0));
        let p = b.build(None, 0.0, 0.1);
        // Never through the corner itself.
        let near = (0..400)
            .map(|k| p.at(k as f64 * p.duration_s() / 400.0).pos)
            .map(|q| q[0].hypot(q[2] + 10_000.0))
            .fold(f64::INFINITY, f64::min);
        assert!(near > 200.0, "{near}");
        // North, then east: a right turn, right wing down.
        let t_corner = p.duration_s() * 0.5;
        assert!(p.at(t_corner).bank > 0.2, "{}", p.at(t_corner).bank);
    }

    #[test]
    fn holds_stand_still() {
        let mut b = PathBuilder::new(100.0);
        b.to(Waypoint::ground([0.0, 0.0, 0.0], 0.0).hold(60.0))
            .to(Waypoint::air([0.0, 300.0, 0.0], 5.0));
        let p = b.build(None, 0.0, 0.1);
        assert_eq!(p.at(30.0).pos, [0.0, 0.0, 0.0]);
        assert!(p.at(30.0).ground);
        assert!(p.at(65.0).pos[1] > 0.0);
    }
}
