//! The motion maths the server and every client share (spec 18A *How the
//! server simulates and sends road traffic*, Q3): the curve and turn
//! speeds ahead, the Intelligent Driver Model, and the ballistic step
//! along a route. Only `+ − × ÷` and `sqrt` (exactly rounded in IEEE 754),
//! never `exp`, `sin` or `powf`, so native and wasm builds give the same
//! bits from the same plan.

use std::sync::OnceLock;

use ocw_world::RoadNetwork;

use super::plan::Step;

/// Radius used for a straight road (m).
pub const STRAIGHT_R: f64 = 10_000.0;
/// Curvature is measured over this many geometry samples to each side
/// (about 20 m: drivers smooth the polyline's corners).
const CURVE_BASE: usize = 4;

/// The horizontal curve radius at each dense geometry point of each edge,
/// made on first use (the same for both directions).
#[derive(Debug, Default)]
pub struct Curves {
    radii: Vec<OnceLock<Vec<f32>>>,
}

impl Curves {
    pub fn new(net: &RoadNetwork) -> Self {
        Self {
            radii: (0..net.graph().edges.len())
                .map(|_| OnceLock::new())
                .collect(),
        }
    }

    /// Radii at the points of `net.geometry(edge)`.
    pub fn radii(&self, net: &RoadNetwork, edge: usize) -> &[f32] {
        self.radii[edge].get_or_init(|| {
            let g = net.geometry(edge);
            let n = g.points.len();
            (0..n)
                .map(|i| {
                    if i < 1 || i + 1 >= n {
                        return STRAIGHT_R as f32;
                    }
                    let k = CURVE_BASE.min(i).min(n - 1 - i);
                    let (a, b, c) = (g.points[i - k], g.points[i], g.points[i + k]);
                    circumradius([a[0], a[2]], [b[0], b[2]], [c[0], c[2]]) as f32
                })
                .collect()
        })
    }
}

/// The radius of the circle through three points (m; [`STRAIGHT_R`] when
/// they are in a line).
pub fn circumradius(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    let d = |p: [f64; 2], q: [f64; 2]| {
        ((p[0] - q[0]) * (p[0] - q[0]) + (p[1] - q[1]) * (p[1] - q[1])).sqrt()
    };
    let cross = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
    let area2 = cross.abs();
    if area2 < 1e-9 {
        return STRAIGHT_R;
    }
    (d(a, b) * d(b, c) * d(a, c) / (2.0 * area2)).min(STRAIGHT_R)
}

/// The radius a vehicle drives through a junction from `arrive` to `leave`
/// (unit headings): 10 m for a right angle, tighter for a U-turn,
/// straight on is no constraint.
pub fn turn_radius(arrive: [f64; 2], leave: [f64; 2]) -> f64 {
    let c = (arrive[0] * leave[0] + arrive[1] * leave[1]).clamp(-1.0, 1.0);
    if c > 0.9999 {
        return STRAIGHT_R;
    }
    (10.0 * (1.0 + c) / (1.0 - c)).clamp(5.0, STRAIGHT_R)
}

/// A driver's own parameters (part of the plan: the client extrapolates
/// with the same values).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DriverParams {
    /// Highest acceleration (m/s²).
    pub accel: f64,
    /// Comfortable deceleration (m/s²).
    pub decel: f64,
    /// Lateral acceleration accepted in curves (m/s²).
    pub lateral: f64,
    /// Time headway (s, IDM `T`).
    pub headway: f64,
    /// Standstill gap (m, IDM `s0`).
    pub gap: f64,
}

/// The highest braking the model applies (m/s²; an emergency stop).
pub const MAX_BRAKE: f64 = 9.0;

/// IDM acceleration: free road towards `v0`, and an interaction with a
/// leader `gap` metres ahead (bumper to bumper) at speed `v_lead`.
pub fn idm(p: &DriverParams, v: f64, v0: f64, leader: Option<(f64, f64)>) -> f64 {
    let v0 = v0.max(0.1);
    let r = v / v0;
    let r2 = r * r;
    let mut a = p.accel * (1.0 - r2 * r2);
    if let Some((gap, v_lead)) = leader {
        let dv = v - v_lead;
        let s_star = p.gap + (v * p.headway + v * dv / (2.0 * (p.accel * p.decel).sqrt())).max(0.0);
        let g = gap.max(0.1);
        let q = s_star / g;
        a -= p.accel * q * q;
    }
    a.clamp(-MAX_BRAKE, p.accel)
}

/// Where a vehicle is on its route: the step and the metres along that
/// step's edge in the driving direction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoutePos {
    pub step: usize,
    pub s: f64,
}

/// The length of a step's edge.
pub fn step_len(net: &RoadNetwork, step: &Step) -> f64 {
    net.geometry(step.edge as usize).length()
}

/// The desired speed at `pos`: the step's speed, lowered ahead of curves,
/// turns and lower limits so the vehicle brakes for them comfortably
/// (`v_c² + 2 b d`, the speed from which it can still slow to `v_c`).
pub fn desired_speed(
    net: &RoadNetwork,
    curves: &Curves,
    route: &[Step],
    pos: RoutePos,
    p: &DriverParams,
) -> f64 {
    let Some(here) = route.get(pos.step) else {
        return 0.0;
    };
    let v0 = f64::from(here.speed);
    let mut best2 = v0 * v0;
    let reach = best2 / (2.0 * p.decel) + 20.0;
    let lat = p.lateral;
    let two_b = 2.0 * p.decel;
    // Metres from the vehicle to the start of the step being looked at.
    let mut base = -pos.s;
    let mut k = pos.step;
    while let Some(step) = route.get(k) {
        if base > reach {
            break;
        }
        let e = step.edge as usize;
        let g = net.geometry(e);
        let len = g.length();
        let radii = curves.radii(net, e);
        let n = radii.len();
        for j in 0..n {
            let idx = if step.forward { j } else { n - 1 - j };
            let along = if step.forward {
                g.cumulative[idx]
            } else {
                len - g.cumulative[idx]
            };
            let d = base + along;
            if d <= 0.0 {
                continue;
            }
            if d > reach {
                break;
            }
            let vc2 = lat * f64::from(radii[idx]);
            let c = vc2 + two_b * d;
            if c < best2 {
                best2 = c;
            }
        }
        let d_end = base + len;
        if d_end > 0.0 {
            // The turn into the next step, and its lower limit.
            let vt2 = lat * f64::from(step.turn_r);
            let c = vt2 + two_b * d_end;
            if c < best2 {
                best2 = c;
            }
            if let Some(next) = route.get(k + 1) {
                let vn = f64::from(next.speed);
                let c = vn * vn + two_b * d_end;
                if c < best2 {
                    best2 = c;
                }
            } else {
                // The route ends: slow down to a stop at its end.
                let c = two_b * d_end + 1.0;
                if c < best2 {
                    best2 = c;
                }
            }
        }
        base = d_end;
        k += 1;
    }
    best2.sqrt()
}

/// Moves `pos` with speed `v` and acceleration `a` for `dt` seconds
/// (ballistic: a vehicle stops instead of reversing), across steps.
/// Returns the new speed; `false` in the flag when the route ended.
pub fn advance(
    net: &RoadNetwork,
    route: &[Step],
    pos: &mut RoutePos,
    v: f64,
    a: f64,
    dt: f64,
) -> (f64, bool) {
    let mut v1 = v + a * dt;
    let ds = if v1 < 0.0 {
        v1 = 0.0;
        if a < 0.0 { -v * v / (2.0 * a) } else { 0.0 }
    } else {
        (v + v1) * 0.5 * dt
    };
    pos.s += ds;
    loop {
        let Some(step) = route.get(pos.step) else {
            return (0.0, false);
        };
        let len = step_len(net, step);
        if pos.s < len {
            return (v1, true);
        }
        if pos.step + 1 >= route.len() {
            pos.s = len;
            return (0.0, false);
        }
        pos.s -= len;
        pos.step += 1;
    }
}

/// The distance from `a` to `b` along the route (negative when `b` is
/// behind `a`).
pub fn route_gap(net: &RoadNetwork, route: &[Step], a: RoutePos, b: RoutePos) -> f64 {
    if a.step == b.step {
        return b.s - a.s;
    }
    let (lo, hi, sign) = if a.step < b.step {
        (a, b, 1.0)
    } else {
        (b, a, -1.0)
    };
    let mut d = -lo.s;
    for k in lo.step..hi.step {
        d += route.get(k).map_or(0.0, |s| step_len(net, s));
    }
    sign * (d + hi.s)
}
