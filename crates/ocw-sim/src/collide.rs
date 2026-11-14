//! Rider collisions (spec §14 Phase 12, 12B / 12C): footprints, contact
//! detection and the response – soft bumps (separation, an inelastic
//! exchange of momentum, a wobble) and crashes (thresholds by contact
//! type, a deterministic choice of who falls, fallen bikes as obstacles).
//!
//! Pure and deterministic (no random numbers; ties are broken by a seed
//! from the tick and the rider ids), so the server, replays and the
//! client agree; builds for wasm32. [`crate::session::Session`] gathers the
//! bodies after the physics step and applies the [`Response`]s.
//!
//! Bodies live in a common plane: the **path frame** (x = distance along
//! the route `s`, y = lateral offset `d`, the road along +x) when all
//! riders share one route, or **world space** (x, z) in free ride, where
//! riders ride their own paths (12B: different edges, junctions). Each
//! body carries its road's direction there, so corrections go back into
//! its own (Δs, Δd).

use std::collections::HashMap;

use ocw_core::RiderId;
use ocw_world::network::splitmix;

/// Footprint of a rider and bike (m): wheelbase plus wheels, handlebar
/// width (12B). The default road bike; every profile uses it for now.
pub const BODY_LENGTH_M: f64 = 1.75;
pub const BODY_WIDTH_M: f64 = 0.5;
/// The wheels inside the footprint: its front and rear 0.35 m.
pub const WHEEL_ZONE_M: f64 = 0.35;
/// Footprint of a fallen rider and bike lying on the road (m, 12B chain
/// crashes).
pub const FALLEN_LENGTH_M: f64 = 1.8;
pub const FALLEN_WIDTH_M: f64 = 1.0;
/// Riders on one route are only tested against neighbours this close
/// along it (m, 12B).
pub const NEIGHBOUR_WINDOW_M: f64 = 3.0;
/// Cell of the spatial hash for world-space checks (m, 12B).
pub const HASH_CELL_M: f64 = 4.0;

/// Crash thresholds (12B), closing speed along the contact normal (m/s):
/// rider against rider (9 km/h) …
pub const CRASH_RIDER_MPS: f64 = 2.5;
/// … against a solid object or a fallen bike (12.6 km/h) …
pub const CRASH_OBJECT_MPS: f64 = 3.5;
/// … a front wheel into another rider's rear wheel with a lateral overlap
/// (the relative speed of the wheels), …
pub const WHEEL_TOUCH_MPS: f64 = 1.0;
/// … or at any speed while leaning more than 10° into the other rider
/// (hooking a wheel, rad) …
pub const HOOK_LEAN_RAD: f64 = 0.1745;
/// … head-on (opposite directions) …
pub const HEAD_ON_MPS: f64 = 1.5;
/// … and the rider in front of a rear-end hit falls too above this.
pub const REAR_END_LEADER_MPS: f64 = 4.0;
/// A hard side hit brings both riders down above this; between
/// [`CRASH_RIDER_MPS`] and it only the rider who was hit falls.
pub const BOTH_FALL_SIDE_MPS: f64 = 3.5;

/// Soft bumps: restitution of the normal impulse (almost fully inelastic)
/// and the friction between shoulders / bars along the contact.
pub const RESTITUTION: f64 = 0.1;
pub const FRICTION: f64 = 0.3;
/// The wobble after a bump: a lean oscillation of ±3–6° over 0.6 s (12B).
pub const WOBBLE_S: f64 = 0.6;
pub const WOBBLE_MIN_RAD: f64 = 0.052;
pub const WOBBLE_MAX_RAD: f64 = 0.105;
/// Its frequency (Hz): about two swings in the 0.6 s.
pub const WOBBLE_HZ: f64 = 3.3;

/// Crash sequence (12C): falling, down by impact energy, remount (s).
pub const FALL_S: f64 = 0.8;
pub const DOWN_MIN_S: f64 = 2.0;
pub const DOWN_MAX_S: f64 = 8.0;
pub const REMOUNT_S: f64 = 1.5;
/// Impact energy that keeps a rider down for [`DOWN_MAX_S`] (J: two
/// 80 kg riders meeting at 8 m/s).
pub const DOWN_REF_ENERGY_J: f64 = 1280.0;

/// The collision model in force (12A, 12G).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Riders pass through each other (only the road edge holds them).
    #[default]
    Off,
    /// Soft bumps only, no crashes (group rides, Phase 12 Q2).
    Soft,
    /// The full model: bumps and crashes (races, free riding).
    Full,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::Off => "off",
            Mode::Soft => "soft bumps only",
            Mode::Full => "on",
        }
    }

    pub fn on(self) -> bool {
        self != Mode::Off
    }
}

/// How two bodies touched (12B).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContactKind {
    /// Side by side (a shoulder rub, bars touching).
    Side,
    /// Wheel beside wheel (both contact points in a wheel zone, from the side).
    WheelToWheel,
    /// A front wheel into another rider's rear wheel with a lateral overlap.
    WheelTouch,
    /// Ridden into from behind (front wheel squarely into the rear wheel).
    RearEnd,
    /// Opposite directions.
    HeadOn,
    /// Against an obstacle: a fallen rider and bike.
    Object,
}

impl ContactKind {
    pub fn label(self) -> &'static str {
        match self {
            ContactKind::Side => "side",
            ContactKind::WheelToWheel => "wheel to wheel",
            ContactKind::WheelTouch => "wheel touch",
            ContactKind::RearEnd => "rear-end",
            ContactKind::HeadOn => "head-on",
            ContactKind::Object => "fallen bike",
        }
    }
}

/// One rider (or a fallen rider and bike) in the common plane.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Body {
    pub id: RiderId,
    /// Centre of the footprint (m).
    pub pos: [f64; 2],
    /// Unit direction of the road at the body (the path frame: +x).
    pub road: [f64; 2],
    /// Direction of travel relative to the road (rad, positive = right).
    pub heading: f64,
    /// Speed along the heading (m/s).
    pub speed: f64,
    /// Lean (rad, positive = right): hooking a wheel.
    pub lean: f64,
    /// Rider plus bike (kg).
    pub mass: f64,
    /// Lying on the road: an immovable obstacle ([`FALLEN_LENGTH_M`] ×
    /// [`FALLEN_WIDTH_M`], along the road).
    pub fallen: bool,
    /// Steered by hand (or a chaotic bot's mistake): may cause crashes.
    /// Two autodrive riders never bring each other down – autodrive never
    /// causes a collision by its own move (12D) – they only bump.
    pub manual: bool,
}

impl Body {
    /// A riding body on a route's path frame.
    pub fn on_path(id: RiderId, s: f64, d: f64, heading: f64, speed: f64, mass: f64) -> Self {
        Self {
            id,
            pos: [s, d],
            road: [1.0, 0.0],
            heading,
            speed,
            lean: 0.0,
            mass,
            fallen: false,
            manual: false,
        }
    }

    /// Unit vector to the road's right.
    pub fn road_right(&self) -> [f64; 2] {
        right_of(self.road)
    }

    /// Direction the footprint points (the road's for a fallen bike).
    pub fn forward(&self) -> [f64; 2] {
        if self.fallen {
            return self.road;
        }
        let (s, c) = self.heading.sin_cos();
        let r = self.road_right();
        [self.road[0] * c + r[0] * s, self.road[1] * c + r[1] * s]
    }

    /// Half length and half width of the footprint.
    pub fn half(&self) -> (f64, f64) {
        if self.fallen {
            (FALLEN_LENGTH_M / 2.0, FALLEN_WIDTH_M / 2.0)
        } else {
            (BODY_LENGTH_M / 2.0, BODY_WIDTH_M / 2.0)
        }
    }

    /// Velocity in the plane.
    pub fn velocity(&self) -> [f64; 2] {
        if self.fallen {
            return [0.0, 0.0];
        }
        scale(self.forward(), self.speed)
    }

    fn inverse_mass(&self) -> f64 {
        if self.fallen || self.mass <= 0.0 {
            0.0
        } else {
            1.0 / self.mass
        }
    }

    /// The four corners of the footprint (for drawing and tests).
    pub fn corners(&self) -> [[f64; 2]; 4] {
        let f = self.forward();
        let b = right_of(f);
        let (hl, hw) = self.half();
        let at = |l: f64, w: f64| {
            [
                self.pos[0] + f[0] * l + b[0] * w,
                self.pos[1] + f[1] * l + b[1] * w,
            ]
        };
        [at(hl, -hw), at(hl, hw), at(-hl, hw), at(-hl, -hw)]
    }
}

/// Where the bodies live.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Frame {
    /// One route: x = `s`, y = `d`; `loop_m` is the length of a loop
    /// (distances wrap there, so riders a lap apart meet).
    Path { loop_m: Option<f64> },
    /// World positions (x, z): free ride, every rider on an own path.
    World,
}

/// A contact between bodies `a` and `b` (indices into the slice given).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Contact {
    pub a: usize,
    pub b: usize,
    pub kind: ContactKind,
    /// Unit normal from `a` to `b`.
    pub normal: [f64; 2],
    /// Overlap along the normal (m).
    pub depth: f64,
    /// Closing speed along the normal (m/s, ≥ 0); for a wheel touch the
    /// relative speed of the wheels.
    pub closing_mps: f64,
    /// `½ · m_red · v_c²` (J; for an obstacle the rider's mass).
    pub energy_j: f64,
    /// The rider whose front wheel hit (wheel touch, rear-end): `a` or `b`.
    pub follower: Option<usize>,
}

/// What a contact does to one body.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    pub kind: ContactKind,
    pub other: RiderId,
    pub closing_mps: f64,
    pub energy_j: f64,
    /// Velocity change at the contact in the body's own frame (m/s: along
    /// its heading, to its right): the ragdoll's push (12C, Q5).
    pub impulse: [f64; 2],
}

/// The response of one body to this tick's contacts.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Response {
    /// Push along the body's road (m) and to its right.
    pub ds: f64,
    pub dd: f64,
    /// New speed (m/s; `None`: unchanged).
    pub speed: Option<f64>,
    /// The heading turned away from the other rider (rad; `None`: unchanged).
    pub heading: Option<f64>,
    /// A soft bump: wobble strength (rad) and side (+1 = pushed right).
    pub bump: Option<(f64, f64)>,
    /// A crash: the hardest hit that brought the body down.
    pub crash: Option<Hit>,
}

impl Response {
    pub fn touched(&self) -> bool {
        self.bump.is_some() || self.crash.is_some() || self.ds != 0.0 || self.dd != 0.0
    }
}

fn dot(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

fn scale(a: [f64; 2], k: f64) -> [f64; 2] {
    [a[0] * k, a[1] * k]
}

fn sub(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

fn add(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] + b[0], a[1] + b[1]]
}

/// The vector 90° to the right of `v` (as `RoutePoint::right`).
pub fn right_of(v: [f64; 2]) -> [f64; 2] {
    [-v[1], v[0]]
}

/// The offset from `a` to `b` in the frame (wrapped on a loop).
fn delta(a: &Body, b: &Body, frame: Frame) -> [f64; 2] {
    let mut d = sub(b.pos, a.pos);
    if let Frame::Path { loop_m: Some(l) } = frame
        && l > 0.0
    {
        d[0] -= l * (d[0] / l).round();
    }
    d
}

/// Overlap of two footprints (separating axes of the two boxes): the
/// unit normal from `a` to `b` and the depth, `None` if apart. `d` is the
/// offset of `b`'s centre from `a`'s.
pub fn overlap(a: &Body, b: &Body, d: [f64; 2]) -> Option<([f64; 2], f64)> {
    let (fa, fb) = (a.forward(), b.forward());
    let (ra, rb) = (right_of(fa), right_of(fb));
    let (hla, hwa) = a.half();
    let (hlb, hwb) = b.half();
    let mut best: Option<([f64; 2], f64)> = None;
    for axis in [fa, ra, fb, rb] {
        let pa = hla * dot(fa, axis).abs() + hwa * dot(ra, axis).abs();
        let pb = hlb * dot(fb, axis).abs() + hwb * dot(rb, axis).abs();
        let dist = dot(d, axis);
        let depth = pa + pb - dist.abs();
        if depth <= 0.0 {
            return None;
        }
        if best.is_none_or(|(_, b)| depth < b - 1e-12) {
            let n = if dist >= 0.0 { axis } else { scale(axis, -1.0) };
            best = Some((n, depth));
        }
    }
    best
}

/// Interval of a body's projection on `axis`, relative to point `origin`.
fn projection(body: &Body, origin: [f64; 2], axis: [f64; 2]) -> (f64, f64) {
    let f = body.forward();
    let r = right_of(f);
    let (hl, hw) = body.half();
    let c = dot(sub(body.pos, origin), axis);
    let e = hl * dot(f, axis).abs() + hw * dot(r, axis).abs();
    (c - e, c + e)
}

/// Where along each body (m from its centre, + = forwards) the contact
/// lies: the middle of the overlap region.
fn contact_along(a: &Body, b: &Body, d: [f64; 2], n: [f64; 2]) -> (f64, f64) {
    // Work with `b` moved next to `a` (the wrap of a loop).
    let b_near = Body {
        pos: add(a.pos, d),
        ..*b
    };
    let t = right_of(n);
    let (a0, a1) = projection(a, a.pos, t);
    let (b0, b1) = projection(&b_near, a.pos, t);
    let mid_t = (a0.max(b0) + a1.min(b1)) / 2.0;
    let (_, a_max) = projection(a, a.pos, n);
    let (b_min, _) = projection(&b_near, a.pos, n);
    let mid_n = (a_max + b_min) / 2.0;
    let p = add(a.pos, add(scale(n, mid_n), scale(t, mid_t)));
    (
        dot(sub(p, a.pos), a.forward()),
        dot(sub(p, b_near.pos), b.forward()),
    )
}

/// Which zone of a body a contact `along` its length is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Zone {
    Front,
    Middle,
    Rear,
}

fn zone(body: &Body, along: f64) -> Zone {
    let (hl, _) = body.half();
    if along > hl - WHEEL_ZONE_M {
        Zone::Front
    } else if along < -hl + WHEEL_ZONE_M {
        Zone::Rear
    } else {
        Zone::Middle
    }
}

/// The contact between bodies `ia` and `ib` (offset `d` from `a` to `b`).
pub fn contact(bodies: &[Body], ia: usize, ib: usize, d: [f64; 2]) -> Option<Contact> {
    let (a, b) = (&bodies[ia], &bodies[ib]);
    if a.fallen && b.fallen {
        return None;
    }
    let (n, depth) = overlap(a, b, d)?;
    let (va, vb) = (a.velocity(), b.velocity());
    let rel = sub(va, vb);
    let closing = dot(rel, n).max(0.0);
    let (fa, fb) = (a.forward(), b.forward());
    let (along_a, along_b) = contact_along(a, b, d, n);
    let (za, zb) = (zone(a, along_a), zone(b, along_b));
    let longitudinal = |f: [f64; 2]| dot(n, f).abs() > 0.7;
    let (kind, follower) = if a.fallen || b.fallen {
        (ContactKind::Object, None)
    } else if dot(fa, fb) < -0.5 {
        (ContactKind::HeadOn, None)
    } else if za == Zone::Front && zb == Zone::Rear {
        if longitudinal(fa) {
            (ContactKind::RearEnd, Some(ia))
        } else {
            (ContactKind::WheelTouch, Some(ia))
        }
    } else if zb == Zone::Front && za == Zone::Rear {
        if longitudinal(fb) {
            (ContactKind::RearEnd, Some(ib))
        } else {
            (ContactKind::WheelTouch, Some(ib))
        }
    } else if za != Zone::Middle && zb != Zone::Middle {
        (ContactKind::WheelToWheel, None)
    } else {
        (ContactKind::Side, None)
    };
    // A wheel touch hurts by how fast the wheels rub, not only by the
    // (often small) sideways closing speed.
    let closing_mps = match kind {
        ContactKind::WheelTouch => closing.max(dot(rel, rel).sqrt()),
        _ => closing,
    };
    let m_red = match (a.fallen, b.fallen) {
        (true, _) => b.mass,
        (_, true) => a.mass,
        _ => a.mass * b.mass / (a.mass + b.mass).max(1e-9),
    };
    Some(Contact {
        a: ia,
        b: ib,
        kind,
        normal: n,
        depth,
        closing_mps,
        energy_j: 0.5 * m_red * closing_mps * closing_mps,
        follower,
    })
}

/// Candidate pairs (i < j by index) with the offset from i to j: in the
/// path frame a sort by `s` and neighbours within [`NEIGHBOUR_WINDOW_M`]
/// (O(n log n)), in world space a [`HASH_CELL_M`] spatial hash.
pub fn candidates(bodies: &[Body], frame: Frame) -> Vec<(usize, usize, [f64; 2])> {
    let mut out = Vec::new();
    let near = |d: [f64; 2]| dot(d, d) < NEIGHBOUR_WINDOW_M * NEIGHBOUR_WINDOW_M;
    match frame {
        Frame::Path { loop_m } => {
            let key = |b: &Body| match loop_m {
                Some(l) if l > 0.0 => b.pos[0].rem_euclid(l),
                _ => b.pos[0],
            };
            let mut order: Vec<(f64, usize)> = bodies
                .iter()
                .enumerate()
                .map(|(i, b)| (key(b), i))
                .collect();
            order.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)));
            let n = order.len();
            let mut push = |i: usize, j: usize| {
                let (i, j) = (i.min(j), i.max(j));
                let d = delta(&bodies[i], &bodies[j], frame);
                if near(d) {
                    out.push((i, j, d));
                }
            };
            for k in 0..n {
                let (s, i) = order[k];
                for &(s2, j) in &order[k + 1..] {
                    if s2 - s > NEIGHBOUR_WINDOW_M {
                        break;
                    }
                    push(i, j);
                }
            }
            // A loop: the last metres meet the first ones.
            if let Some(l) = loop_m.filter(|l| *l > 2.0 * NEIGHBOUR_WINDOW_M) {
                for &(s, i) in order.iter().rev() {
                    if s < l - NEIGHBOUR_WINDOW_M {
                        break;
                    }
                    for &(s2, j) in &order {
                        if s2 + l - s > NEIGHBOUR_WINDOW_M {
                            break;
                        }
                        if i != j {
                            push(i, j);
                        }
                    }
                }
            }
        }
        Frame::World => {
            let cell = |p: [f64; 2]| {
                (
                    (p[0] / HASH_CELL_M).floor() as i64,
                    (p[1] / HASH_CELL_M).floor() as i64,
                )
            };
            let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
            for (i, b) in bodies.iter().enumerate() {
                grid.entry(cell(b.pos)).or_default().push(i);
            }
            for (i, b) in bodies.iter().enumerate() {
                let (cx, cz) = cell(b.pos);
                for dx in -1..=1 {
                    for dz in -1..=1 {
                        let Some(list) = grid.get(&(cx + dx, cz + dz)) else {
                            continue;
                        };
                        for &j in list {
                            if j > i {
                                let d = delta(b, &bodies[j], frame);
                                if near(d) {
                                    out.push((i, j, d));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    out.sort_by_key(|x| (x.0, x.1));
    out.dedup_by(|x, y| x.0 == y.0 && x.1 == y.1);
    out
}

/// Every contact this tick, in a deterministic order.
pub fn detect(bodies: &[Body], frame: Frame) -> Vec<Contact> {
    candidates(bodies, frame)
        .into_iter()
        .filter_map(|(i, j, d)| contact(bodies, i, j, d))
        .collect()
}

/// A deterministic coin for a contact (no random numbers on the server):
/// from the tick and both rider ids.
pub fn coin(tick: u64, a: RiderId, b: RiderId) -> bool {
    let (lo, hi) = (a.0.min(b.0), a.0.max(b.0));
    splitmix(tick ^ (u64::from(lo) << 32 | u64::from(hi))) & 1 == 1
}

/// Who falls in a contact (12B): `(a falls, b falls)`. Only in
/// [`Mode::Full`]; two autodrive riders never bring each other down.
pub fn fallers(c: &Contact, bodies: &[Body], mode: Mode, tick: u64) -> (bool, bool) {
    if mode != Mode::Full {
        return (false, false);
    }
    let (a, b) = (&bodies[c.a], &bodies[c.b]);
    let v = c.closing_mps;
    match c.kind {
        ContactKind::Object => (
            !a.fallen && v >= CRASH_OBJECT_MPS,
            !b.fallen && v >= CRASH_OBJECT_MPS,
        ),
        _ if !(a.manual || b.manual) => (false, false),
        ContactKind::HeadOn => (v > HEAD_ON_MPS, v > HEAD_ON_MPS),
        ContactKind::RearEnd => {
            let follower_a = c.follower == Some(c.a);
            let f = v >= CRASH_RIDER_MPS;
            let l = v >= REAR_END_LEADER_MPS;
            if follower_a { (f, l) } else { (l, f) }
        }
        ContactKind::WheelTouch => {
            let follower_a = c.follower == Some(c.a);
            let (f, l) = if follower_a { (a, b) } else { (b, a) };
            // Leaning into the other rider hooks the wheel at any speed.
            let to_leader = if follower_a {
                c.normal
            } else {
                scale(c.normal, -1.0)
            };
            let side = dot(to_leader, right_of(f.forward())).signum();
            let _ = l;
            let down = v >= WHEEL_TOUCH_MPS || f.lean * side > HOOK_LEAN_RAD;
            // The rider hit in the rear wheel usually stays up.
            if follower_a {
                (down, false)
            } else {
                (false, down)
            }
        }
        ContactKind::Side | ContactKind::WheelToWheel => {
            if v >= BOTH_FALL_SIDE_MPS {
                (true, true)
            } else if v >= CRASH_RIDER_MPS {
                // The rider who was hit falls: the one moving less towards
                // the other; a tie by the deterministic coin.
                // Sideways speed over the road towards the other rider.
                let towards = |body: &Body, n: [f64; 2]| {
                    let side = dot(n, body.road_right()).signum();
                    body.speed * body.heading.sin() * side
                };
                let push_a = towards(a, c.normal);
                let push_b = towards(b, scale(c.normal, -1.0));
                if (push_a - push_b).abs() < 0.05 {
                    let heads = coin(tick, a.id, b.id);
                    (heads, !heads)
                } else {
                    (push_a < push_b, push_b < push_a)
                }
            } else {
                (false, false)
            }
        }
    }
}

/// How long a rider stays down after a hit of `energy_j` (12C: 2–8 s).
pub fn down_s(energy_j: f64) -> f64 {
    DOWN_MIN_S + (DOWN_MAX_S - DOWN_MIN_S) * (energy_j / DOWN_REF_ENERGY_J).clamp(0.0, 1.0)
}

/// Wobble strength of a bump at closing speed `v` (rad, 3–6°).
pub fn wobble_strength(v: f64) -> f64 {
    (WOBBLE_MIN_RAD + v * 0.02).clamp(WOBBLE_MIN_RAD, WOBBLE_MAX_RAD)
}

/// The wobble's lean `t` s after a bump of `strength` (rad, signed by
/// `side`): a damped oscillation that is gone after [`WOBBLE_S`].
pub fn wobble(t: f64, strength: f64, side: f64) -> f64 {
    if !(0.0..WOBBLE_S).contains(&t) {
        return 0.0;
    }
    let fade = 1.0 - t / WOBBLE_S;
    side * strength * fade * fade * (2.0 * std::f64::consts::PI * WOBBLE_HZ * t).sin()
}

/// Resolves this tick's contacts (12B): every body's response, in the
/// order of `bodies`. Off: none. Soft bumps push the bodies apart (no
/// overlap left), exchange momentum inelastically along the normal (with
/// friction along the contact) and wobble; hard hits bring riders down
/// (by [`fallers`]).
pub fn resolve(bodies: &[Body], frame: Frame, mode: Mode, tick: u64) -> Vec<Response> {
    let mut out = vec![Response::default(); bodies.len()];
    if mode == Mode::Off {
        return out;
    }
    let contacts = detect(bodies, frame);
    if contacts.is_empty() {
        return out;
    }
    let mut vel: Vec<[f64; 2]> = bodies.iter().map(Body::velocity).collect();
    let mut push: Vec<[f64; 2]> = vec![[0.0, 0.0]; bodies.len()];
    let mut heading: Vec<Option<f64>> = vec![None; bodies.len()];
    for c in &contacts {
        let (a, b) = (&bodies[c.a], &bodies[c.b]);
        let (wa, wb) = (a.inverse_mass(), b.inverse_mass());
        let w = wa + wb;
        if w <= 0.0 {
            continue;
        }
        let n = c.normal;
        // Separation to touching (a hair more), by inverse mass.
        let sep = c.depth + 0.005;
        push[c.a] = sub(push[c.a], scale(n, sep * wa / w));
        push[c.b] = add(push[c.b], scale(n, sep * wb / w));
        // Inelastic exchange along the normal, friction along the contact.
        let before = (vel[c.a], vel[c.b]);
        let vn = dot(sub(vel[c.a], vel[c.b]), n);
        if vn > 0.0 {
            let j = (1.0 + RESTITUTION) * vn / w;
            vel[c.a] = sub(vel[c.a], scale(n, j * wa));
            vel[c.b] = add(vel[c.b], scale(n, j * wb));
            let t = right_of(n);
            let vt = dot(sub(vel[c.a], vel[c.b]), t);
            let jt = (vt / w).clamp(-FRICTION * j, FRICTION * j);
            vel[c.a] = sub(vel[c.a], scale(t, jt * wa));
            vel[c.b] = add(vel[c.b], scale(t, jt * wb));
        }
        // Steer away: a heading towards the other rider is halved.
        for (i, body, towards) in [(c.a, a, n), (c.b, b, scale(n, -1.0))] {
            if !body.fallen {
                let h = heading[i].unwrap_or(body.heading);
                let side = dot(towards, body.road_right());
                if h * side > 0.0 && side.abs() > 0.3 {
                    heading[i] = Some(h * 0.5);
                }
            }
        }
        let (fall_a, fall_b) = fallers(c, bodies, mode, tick);
        for (i, body, other, falls, dv, away) in [
            (c.a, a, b, fall_a, sub(vel[c.a], before.0), scale(n, -1.0)),
            (c.b, b, a, fall_b, sub(vel[c.b], before.1), n),
        ] {
            if body.fallen {
                continue;
            }
            let f = body.forward();
            if falls {
                let hit = Hit {
                    kind: c.kind,
                    other: other.id,
                    closing_mps: c.closing_mps,
                    energy_j: c.energy_j,
                    // The push: the velocity change, at least away from
                    // the other body by a share of the closing speed.
                    impulse: {
                        let push = add(dv, scale(away, 0.3 * c.closing_mps));
                        [dot(push, f), dot(push, right_of(f))]
                    },
                };
                let hardest = out[i].crash.is_none_or(|h| h.energy_j < hit.energy_j);
                if hardest {
                    out[i].crash = Some(hit);
                }
            } else {
                let strength = wobble_strength(c.closing_mps);
                let side = dot(away, body.road_right()).signum();
                let side = if side == 0.0 { 1.0 } else { side };
                if out[i].bump.is_none_or(|(s, _)| s < strength) {
                    out[i].bump = Some((strength, side));
                }
            }
        }
    }
    for (i, body) in bodies.iter().enumerate() {
        if body.fallen {
            continue;
        }
        let r = &mut out[i];
        let p = push[i];
        r.ds = dot(p, body.road);
        r.dd = dot(p, body.road_right());
        let dv = sub(vel[i], body.velocity());
        if dot(dv, dv) > 1e-18 {
            r.speed = Some(dot(vel[i], body.forward()).max(0.0));
        }
        r.heading = heading[i];
    }
    out
}

/// A soft bump of a rider (12B), for the wobble and the sound.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bump {
    /// The server tick it happened at.
    pub tick: u64,
    /// Wobble strength (rad) and side (+1 = pushed to the right).
    pub strength: f64,
    pub side: f64,
}

/// Phase of a crash (12C), part of the rider state in snapshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrashPhase {
    /// Going down (0.8 s): sliding to a stop, the ragdoll falls.
    Falling,
    /// Lying on the road (2–8 s by impact energy).
    Down,
    /// Picking the bike up and clipping in (1.5 s).
    Remount,
}

/// A crash as the clients see it (from the server, 12C, 12D: a crash is
/// only shown when the server says so).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CrashView {
    pub phase: CrashPhase,
    /// Since the crash began (s).
    pub elapsed_s: f64,
    /// Time down (s), by the impact energy.
    pub down_s: f64,
    pub kind: ContactKind,
    /// Who or what the rider hit.
    pub by: Option<RiderId>,
    /// The push at the contact in the rider's frame (m/s: along, right).
    pub impulse: [f64; 2],
    /// Speed when it happened (m/s).
    pub speed: f64,
}

impl CrashView {
    /// Seconds until the rider rides again.
    pub fn remaining_s(&self) -> f64 {
        (FALL_S + self.down_s + REMOUNT_S - self.elapsed_s).max(0.0)
    }

    /// Lying on the road (falling or down): the avatar is the ragdoll.
    pub fn on_the_ground(&self) -> bool {
        self.phase != CrashPhase::Remount
    }
}

/// The phase `elapsed_s` into a crash with `down_s` down; `None` once the
/// rider rides again.
pub fn crash_phase(elapsed_s: f64, down_s: f64) -> Option<CrashPhase> {
    if elapsed_s < FALL_S {
        Some(CrashPhase::Falling)
    } else if elapsed_s < FALL_S + down_s {
        Some(CrashPhase::Down)
    } else if elapsed_s < FALL_S + down_s + REMOUNT_S {
        Some(CrashPhase::Remount)
    } else {
        None
    }
}

/// A rider's lean in a steady curve (rad, as the avatar's).
pub fn lean_of(speed: f64, curvature: f64) -> f64 {
    (speed * speed * curvature / ocw_core::GRAVITY).atan()
}

#[cfg(test)]
mod tests {
    use super::*;

    const M: f64 = 80.0;

    fn rider(id: u32, s: f64, d: f64, speed: f64) -> Body {
        Body::on_path(RiderId(id), s, d, 0.0, speed, M)
    }

    fn path() -> Frame {
        Frame::Path { loop_m: None }
    }

    #[test]
    fn footprints_overlap_in_the_path_frame() {
        let a = rider(1, 0.0, 0.0, 8.0);
        // Side by side 0.45 m apart: 5 cm overlap of the 0.5 m wide bodies.
        let b = rider(2, 0.0, 0.45, 8.0);
        let (n, depth) = overlap(&a, &b, sub(b.pos, a.pos)).unwrap();
        assert!((depth - 0.05).abs() < 1e-9, "{depth}");
        assert!((n[1] - 1.0).abs() < 1e-9, "normal to the right: {n:?}");
        // 0.55 m apart: no contact.
        let c = rider(3, 0.0, 0.55, 8.0);
        assert!(overlap(&a, &c, sub(c.pos, a.pos)).is_none());
        // Nose to tail 1.7 m apart: 5 cm along.
        let d = rider(4, 1.7, 0.0, 8.0);
        let (n, depth) = overlap(&a, &d, sub(d.pos, a.pos)).unwrap();
        assert!((depth - 0.05).abs() < 1e-9 && n[0] > 0.99, "{n:?} {depth}");
        // A rider turned by the heading reaches further sideways.
        let turned = Body {
            heading: 0.3,
            ..rider(5, 0.0, 0.6, 8.0)
        };
        assert!(overlap(&a, &turned, sub(turned.pos, a.pos)).is_some());
    }

    #[test]
    fn footprints_overlap_in_world_space() {
        // Two riders on crossing roads at a junction: one rides east, one
        // north; the second's nose reaches into the first's side.
        let east = Body {
            id: RiderId(1),
            pos: [100.0, 50.0],
            road: [1.0, 0.0],
            ..rider(1, 0.0, 0.0, 6.0)
        };
        let north = Body {
            id: RiderId(2),
            pos: [100.2, 49.0],
            road: [0.0, 1.0],
            ..rider(2, 0.0, 0.0, 6.0)
        };
        let all = [east, north];
        let found = detect(&all, Frame::World);
        assert_eq!(found.len(), 1, "{found:?}");
        // Far apart in different hash cells: nothing.
        let far = Body {
            pos: [200.0, 49.0],
            ..north
        };
        assert!(detect(&[east, far], Frame::World).is_empty());
    }

    #[test]
    fn contact_types() {
        let kind = |a: Body, b: Body| detect(&[a, b], path())[0];
        // Shoulder to shoulder.
        assert_eq!(
            kind(rider(1, 0.0, 0.0, 8.0), rider(2, 0.0, 0.45, 8.0)).kind,
            ContactKind::Side
        );
        // Front wheel squarely into the rear wheel ahead.
        let c = kind(rider(1, 0.0, 0.0, 9.0), rider(2, 1.7, 0.05, 7.0));
        assert_eq!(c.kind, ContactKind::RearEnd);
        assert_eq!(c.follower, Some(0));
        assert!((c.closing_mps - 2.0).abs() < 0.05, "{}", c.closing_mps);
        // Overlapping wheels from the side: a wheel touch, the rider
        // behind is the follower (in either order).
        let c = kind(rider(1, 1.55, 0.32, 7.0), rider(2, 0.0, 0.0, 8.0));
        assert_eq!(c.kind, ContactKind::WheelTouch);
        assert_eq!(c.follower, Some(1));
        // The front wheel beside the rear wheel ahead, touching: a wheel
        // touch too.
        let c = kind(rider(1, 0.0, 0.0, 8.0), rider(2, 1.3, 0.45, 8.0));
        assert_eq!(c.kind, ContactKind::WheelTouch);
        assert_eq!(c.follower, Some(0));
        // Opposite directions.
        let back = Body {
            heading: std::f64::consts::PI,
            ..rider(2, 1.6, 0.0, 5.0)
        };
        let c = kind(rider(1, 0.0, 0.0, 5.0), back);
        assert_eq!(c.kind, ContactKind::HeadOn);
        assert!((c.closing_mps - 10.0).abs() < 1e-6, "{}", c.closing_mps);
        // A fallen bike lying ahead.
        let fallen = Body {
            fallen: true,
            ..rider(2, 1.7, 0.0, 0.0)
        };
        let c = kind(rider(1, 0.0, 0.0, 6.0), fallen);
        assert_eq!(c.kind, ContactKind::Object);
        assert!((c.energy_j - 0.5 * M * 36.0).abs() < 1e-6);
    }

    #[test]
    fn soft_bump_separates_without_overlap_and_conserves_momentum() {
        // A rides into B from behind at 1.5 m/s more: below the threshold.
        let bodies = [rider(1, 0.0, 0.0, 9.0), rider(2, 1.65, 0.0, 7.5)];
        let r = resolve(&bodies, path(), Mode::Full, 10);
        assert!(r[0].crash.is_none() && r[1].crash.is_none());
        assert!(r[0].bump.is_some() && r[1].bump.is_some());
        // Pushed apart to touching: no overlap left.
        let moved: Vec<Body> = bodies
            .iter()
            .zip(&r)
            .map(|(b, r)| Body {
                pos: [b.pos[0] + r.ds, b.pos[1] + r.dd],
                speed: r.speed.unwrap_or(b.speed),
                ..*b
            })
            .collect();
        assert!(detect(&moved, path()).is_empty(), "no overlap left");
        // The faster rider slowed down, the slower one was pushed; the
        // momentum along the road is kept.
        let (va, vb) = (moved[0].speed, moved[1].speed);
        assert!(va < 9.0 && vb > 7.5, "{va} {vb}");
        let before = M * 9.0 + M * 7.5;
        let after = M * va + M * vb;
        assert!((before - after).abs() < 1e-6, "{before} {after}");
        // Almost fully inelastic: nearly the same speed afterwards.
        assert!((va - vb).abs() < 0.2, "{va} {vb}");
    }

    #[test]
    fn shoulder_rub_exchanges_speed_and_pushes_apart_sideways() {
        // B steers into A from the side while a little faster.
        let a = rider(1, 0.0, 0.0, 8.0);
        let b = Body {
            heading: -0.15,
            ..rider(2, 0.2, 0.46, 9.0)
        };
        let r = resolve(&[a, b], path(), Mode::Full, 1);
        assert!(r[0].crash.is_none() && r[1].crash.is_none(), "{r:?}");
        assert!(r[0].dd < 0.0 && r[1].dd > 0.0, "pushed apart: {r:?}");
        // The wobble is to the side each was pushed.
        assert!(r[0].bump.unwrap().1 < 0.0 && r[1].bump.unwrap().1 > 0.0);
        let (s, _) = r[0].bump.unwrap();
        assert!((WOBBLE_MIN_RAD..=WOBBLE_MAX_RAD).contains(&s));
        // B's heading into A is halved.
        assert!((r[1].heading.unwrap() + 0.075).abs() < 1e-9);
        // Friction: the faster slows, the slower speeds up.
        assert!(
            r[1].speed.unwrap() < 9.0 && r[0].speed.unwrap() > 8.0,
            "{r:?}"
        );
    }

    #[test]
    fn crash_at_and_above_the_threshold() {
        let manual = |b: Body| Body { manual: true, ..b };
        // Rear-end at 2.4 m/s more: a bump; at 2.5: the rider behind falls,
        // the one in front stays up.
        let r = resolve(
            &[manual(rider(1, 0.0, 0.0, 9.4)), rider(2, 1.7, 0.0, 7.0)],
            path(),
            Mode::Full,
            1,
        );
        assert!(r[0].crash.is_none(), "{r:?}");
        let r = resolve(
            &[manual(rider(1, 0.0, 0.0, 9.5)), rider(2, 1.7, 0.0, 7.0)],
            path(),
            Mode::Full,
            1,
        );
        let hit = r[0].crash.expect("the rider behind crashes");
        assert_eq!(hit.kind, ContactKind::RearEnd);
        assert_eq!(hit.other, RiderId(2));
        assert!(r[1].crash.is_none() && r[1].bump.is_some(), "{r:?}");
        // Hard enough, the rider in front goes down too.
        let r = resolve(
            &[manual(rider(1, 0.0, 0.0, 11.5)), rider(2, 1.7, 0.0, 7.0)],
            path(),
            Mode::Full,
            1,
        );
        assert!(r[0].crash.is_some() && r[1].crash.is_some());
        // Soft mode (group rides): never a crash.
        let r = resolve(
            &[manual(rider(1, 0.0, 0.0, 11.5)), rider(2, 1.7, 0.0, 7.0)],
            path(),
            Mode::Soft,
            1,
        );
        assert!(r.iter().all(|r| r.crash.is_none() && r.bump.is_some()));
        // A wheel touch at 1 m/s brings the follower down …
        let r = resolve(
            &[manual(rider(1, 0.0, 0.0, 8.0)), rider(2, 1.55, 0.32, 7.0)],
            path(),
            Mode::Full,
            1,
        );
        assert_eq!(r[0].crash.unwrap().kind, ContactKind::WheelTouch);
        assert!(
            r[1].crash.is_none(),
            "the rider hit in the rear wheel stays up"
        );
        // … and below it only when leaning into the other rider (hooking).
        let slow = manual(rider(1, 0.0, 0.0, 7.5));
        let r = resolve(&[slow, rider(2, 1.55, 0.32, 7.0)], path(), Mode::Full, 1);
        assert!(r[0].crash.is_none(), "{r:?}");
        let hooking = Body { lean: 0.2, ..slow };
        let r = resolve(&[hooking, rider(2, 1.55, 0.32, 7.0)], path(), Mode::Full, 1);
        assert!(r[0].crash.is_some(), "{r:?}");
        // Head-on above 1.5 m/s: both down.
        let back = Body {
            heading: std::f64::consts::PI,
            ..rider(2, 1.7, 0.0, 1.0)
        };
        let r = resolve(
            &[manual(rider(1, 0.0, 0.0, 1.0)), back],
            path(),
            Mode::Full,
            1,
        );
        assert!(r[0].crash.is_some() && r[1].crash.is_some());
        // A fallen bike hit at 3.5 m/s.
        let fallen = Body {
            fallen: true,
            ..rider(2, 1.7, 0.0, 0.0)
        };
        let r = resolve(&[rider(1, 0.0, 0.0, 3.4), fallen], path(), Mode::Full, 1);
        assert!(r[0].crash.is_none());
        let r = resolve(&[rider(1, 0.0, 0.0, 3.6), fallen], path(), Mode::Full, 1);
        assert_eq!(r[0].crash.unwrap().kind, ContactKind::Object);
    }

    #[test]
    fn two_autodrive_riders_only_bump() {
        // The same hard rear-end without a manual rider: bumps only.
        let r = resolve(
            &[rider(1, 0.0, 0.0, 11.5), rider(2, 1.7, 0.0, 7.0)],
            path(),
            Mode::Full,
            1,
        );
        assert!(r.iter().all(|r| r.crash.is_none() && r.bump.is_some()));
    }

    #[test]
    fn the_faller_is_deterministic() {
        // A hard side hit with equal pushes: the coin decides – the same
        // every time for the same tick and riders, and only one falls.
        let a = Body {
            heading: 0.2,
            manual: true,
            ..rider(1, 0.0, 0.0, 8.0)
        };
        let b = Body {
            heading: -0.2,
            ..rider(2, 0.0, 0.48, 8.0)
        };
        let first = resolve(&[a, b], path(), Mode::Full, 77);
        for _ in 0..5 {
            assert_eq!(resolve(&[a, b], path(), Mode::Full, 77), first);
        }
        let down: Vec<bool> = first.iter().map(|r| r.crash.is_some()).collect();
        assert_eq!(down.iter().filter(|d| **d).count(), 1, "{first:?}");
        assert_eq!(coin(77, a.id, b.id), down[0]);
        // The rider who steered into the other stays up when the pushes differ.
        let a_hard = Body {
            heading: 0.3,
            ..rider(1, 0.0, 0.0, 10.0)
        };
        let b_straight = Body {
            heading: 0.0,
            ..rider(2, 0.0, 0.48, 10.0)
        };
        let r = resolve(
            &[
                Body {
                    manual: true,
                    ..a_hard
                },
                b_straight,
            ],
            path(),
            Mode::Full,
            77,
        );
        assert!(r[0].crash.is_none() && r[1].crash.is_some(), "{r:?}");
    }

    #[test]
    fn a_fallen_bike_is_an_immovable_obstacle() {
        let fallen = Body {
            fallen: true,
            ..rider(2, 1.75, 0.3, 0.0)
        };
        // 1.8 × 1 m: reaches 0.8 m to the side – a rider 0.7 m beside its
        // centre still touches it.
        let beside = rider(3, 1.75, 1.0, 5.0);
        assert!(!detect(&[fallen, beside], path()).is_empty());
        let r = resolve(&[rider(1, 0.0, 0.0, 2.0), fallen], path(), Mode::Full, 1);
        assert_eq!(r[1], Response::default(), "the fallen bike does not move");
        assert!(r[0].ds < 0.0, "the rider is pushed back: {r:?}");
        assert!(r[0].speed.unwrap() < 0.5, "and stopped: {r:?}");
    }

    #[test]
    fn collisions_off_means_no_contact() {
        let bodies = [rider(1, 0.0, 0.0, 12.0), rider(2, 0.5, 0.1, 3.0)];
        let r = resolve(&bodies, path(), Mode::Off, 1);
        assert!(r.iter().all(|r| *r == Response::default()));
    }

    #[test]
    fn riders_a_lap_apart_meet_on_a_loop() {
        let bodies = [rider(1, 999.5, 0.0, 9.0), rider(2, 2001.1, 0.0, 7.0)];
        let found = detect(
            &bodies,
            Frame::Path {
                loop_m: Some(1000.0),
            },
        );
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].normal[0] > 0.99, "{found:?}");
    }

    #[test]
    fn crash_phases_and_time_down() {
        assert_eq!(down_s(0.0), DOWN_MIN_S);
        assert_eq!(down_s(1e9), DOWN_MAX_S);
        assert!(down_s(500.0) > down_s(100.0));
        let d = 3.0;
        assert_eq!(crash_phase(0.1, d), Some(CrashPhase::Falling));
        assert_eq!(crash_phase(FALL_S + 1.0, d), Some(CrashPhase::Down));
        assert_eq!(crash_phase(FALL_S + d + 0.1, d), Some(CrashPhase::Remount));
        assert_eq!(crash_phase(FALL_S + d + REMOUNT_S + 0.01, d), None);
    }

    #[test]
    fn the_wobble_fades_out() {
        assert_eq!(wobble(-0.1, 0.1, 1.0), 0.0);
        assert_eq!(wobble(WOBBLE_S, 0.1, 1.0), 0.0);
        let peak = (0..60)
            .map(|i| wobble(f64::from(i) * 0.01, 0.1, 1.0).abs())
            .fold(0.0, f64::max);
        assert!(peak > 0.04 && peak <= 0.1, "{peak}");
    }

    #[test]
    fn a_hundred_riders_are_checked_quickly() {
        // A bunch of 100 on one route: 10 rows of 10 (§15: within 0.3 ms
        // per tick; release builds are much faster than this debug check).
        let bodies: Vec<Body> = (0..100)
            .map(|i| {
                rider(
                    i,
                    f64::from(i / 10) * 2.0,
                    f64::from(i % 10) * 0.55 - 2.5,
                    10.0,
                )
            })
            .collect();
        // And a crush: every rider touching its neighbours (0.45 m apart
        // sideways, 1.7 m along): about 180 contacts each tick.
        let crush: Vec<Body> = (0..100)
            .map(|i| {
                rider(
                    i,
                    f64::from(i / 10) * 1.7,
                    f64::from(i % 10) * 0.45 - 2.0,
                    10.0 - f64::from(i / 10) * 0.1,
                )
            })
            .collect();
        assert!(detect(&crush, path()).len() > 150);
        let budget = if cfg!(debug_assertions) { 3.0 } else { 0.3 };
        for (what, field) in [("a bunch", &bodies), ("a crush", &crush)] {
            let t = std::time::Instant::now();
            let n = 200;
            for tick in 0..n {
                let _ = resolve(field, path(), Mode::Full, tick);
            }
            let per_tick_ms = t.elapsed().as_secs_f64() * 1000.0 / n as f64;
            println!("collisions for 100 riders ({what}): {per_tick_ms:.4} ms per tick");
            assert!(per_tick_ms < budget, "{what}: {per_tick_ms} ms");
        }
    }
}
