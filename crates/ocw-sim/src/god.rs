//! God mode (spec §14 Phase 13): the rider leaves the road and flies freely
//! through the world as a unicorn.
//!
//! Everything here is pure and engine-free (it builds for wasm32):
//!
//! - the **flight model** ([`Flight`]): the speed law
//!   `v = clamp(20 m/s + 0.5 × height above ground, 20, 400 m/s)`, Shift
//!   ×4, Ctrl ×0.25, the wheel scale ×0.25 … ×8, climbing at half the
//!   horizontal speed, smoothed movements (0.15 s to speed up, 0.25 s to
//!   stop), moving relative to the view's yaw in the horizontal plane (the
//!   pitch never dives), at least [`CLEARANCE_M`] above the ground (terrain,
//!   water, buildings – the caller's surface), at most [`CEILING_M`] above
//!   sea level, softly stopped [`LIMIT_MARGIN_M`] outside the world;
//! - the **glides** of leaving ([`Glide`]): down to the landing spot, or
//!   back to the take-off spot (Shift + G), at most [`MAX_GLIDE_S`];
//! - the **landing spot** on a polyline ([`nearest_on_polyline`]) and on a
//!   route of the road graph ([`nearest_on_plan`]);
//! - the server's **plausibility check** of the poses a client sends
//!   ([`check_pose`]);
//! - the wing beat and gallop rates the unicorn's animation and sounds
//!   share ([`wing_beat_hz`], [`gallop_hz`]).
//!
//! Coordinates are local metres: x east, y up, z south (the world's).
//! The yaw turns to the left (counter-clockwise seen from above, as a
//! positive rotation about +y): yaw 0 looks along −z (north).

use ocw_world::{RoadNetwork, RoutePlan, Segment};

/// Slowest base speed (m/s, 72 km/h): close to the ground.
pub const MIN_SPEED_MPS: f64 = 20.0;
/// Fastest base speed (m/s, 1,440 km/h): from 760 m above the ground.
pub const MAX_BASE_SPEED_MPS: f64 = 400.0;
/// Base speed gained per metre above the ground (1/s).
pub const SPEED_PER_HEIGHT: f64 = 0.5;
/// Shift: boost factor.
pub const BOOST: f64 = 4.0;
/// Ctrl: precise factor.
pub const PRECISE: f64 = 0.25;
/// The mouse wheel's scale of the base speed.
pub const MIN_SCALE: f64 = 0.25;
pub const MAX_SCALE: f64 = 8.0;
/// One wheel step multiplies the scale by this (16 steps from ×0.25 to ×8).
pub const SCALE_STEP: f64 = 1.241_857_812_073_484; // 32^(1/16)
/// Climb rate as a fraction of the horizontal speed.
pub const CLIMB_FRACTION: f64 = 0.5;
/// Time constant of speeding up (s) …
pub const ACCEL_TAU_S: f64 = 0.15;
/// … and of stopping (s).
pub const STOP_TAU_S: f64 = 0.25;
/// Least height above the ground (terrain, water, buildings).
pub const CLEARANCE_M: f64 = 2.0;
/// Highest altitude above sea level.
pub const CEILING_M: f64 = 8_000.0;
/// The unicorn is softly stopped this far outside the world set.
pub const LIMIT_MARGIN_M: f64 = 200.0;
/// Fastest the unicorn ever flies (m/s): the fastest base speed with the
/// boost. The wheel scale (×8) cannot go beyond it, so the speed stays in
/// what the tile streaming and the server's check can follow (a design
/// decision of G1: 400 × 4 × 8 = 12.8 km/s would cross a 1 km tile per
/// frame).
pub const MAX_SPEED_MPS: f64 = MAX_BASE_SPEED_MPS * BOOST;
/// Pitch limit of the view (rad, 89°).
pub const MAX_PITCH: f64 = 89.0 * std::f64::consts::PI / 180.0;
/// How far ahead (s of flight) the clearance looks, so the unicorn glides
/// up over a hill instead of being pushed up at its foot.
pub const LOOK_AHEAD_S: f64 = 0.6;
/// The time constant of gliding up to what lies ahead (s).
pub const RISE_TAU_S: f64 = 0.25;
/// The unicorn rises this much when the rider turns into it (m).
pub const TAKE_OFF_RISE_M: f64 = 3.0;
/// The transformation takes this long (s).
pub const TRANSFORM_S: f64 = 0.6;
/// Longest glide when leaving (s), however far.
pub const MAX_GLIDE_S: f64 = 2.0;
/// Rate at which god poses are sent (Hz).
pub const POSE_HZ: f64 = 10.0;

/// The base speed at `height` metres above the ground (m/s).
pub fn base_speed(height_m: f64) -> f64 {
    (MIN_SPEED_MPS + SPEED_PER_HEIGHT * height_m.max(0.0)).clamp(MIN_SPEED_MPS, MAX_BASE_SPEED_MPS)
}

/// The speed factor of the modifier keys: Shift ×4, Ctrl ×0.25 (both: ×1).
pub fn modifier_factor(boost: bool, precise: bool) -> f64 {
    (if boost { BOOST } else { 1.0 }) * (if precise { PRECISE } else { 1.0 })
}

/// The flight speed for this height, modifiers and wheel scale (m/s),
/// capped at [`MAX_SPEED_MPS`].
pub fn flight_speed(height_m: f64, boost: bool, precise: bool, scale: f64) -> f64 {
    (base_speed(height_m) * modifier_factor(boost, precise) * scale.clamp(MIN_SCALE, MAX_SCALE))
        .min(MAX_SPEED_MPS)
}

/// Horizontal unit vector the view looks along at `yaw` (x, z).
pub fn forward(yaw: f64) -> [f64; 2] {
    [-yaw.sin(), -yaw.cos()]
}

/// Horizontal unit vector to the right of the view at `yaw` (x, z).
pub fn right(yaw: f64) -> [f64; 2] {
    [yaw.cos(), -yaw.sin()]
}

/// The yaw that looks along the horizontal direction `d` (x, z).
pub fn yaw_of(d: [f64; 2]) -> f64 {
    (-d[0]).atan2(-d[1])
}

/// Compass heading of `yaw` (degrees, 0 = north, 90 = east).
pub fn heading_deg(yaw: f64) -> f64 {
    let f = forward(yaw);
    // North is −z, east +x.
    f[0].atan2(-f[1]).to_degrees().rem_euclid(360.0)
}

/// What the rider asks of the flight this frame. The movement values are
/// −1 … 1 (keys give ±1, sticks and triggers in between).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FlightInput {
    /// Forward (+) / back (−), along the view's yaw.
    pub forward: f64,
    /// Right (+) / left (−).
    pub strafe: f64,
    /// Climb (+) / descend (−).
    pub climb: f64,
    /// Shift (×4).
    pub boost: bool,
    /// Ctrl (×0.25).
    pub precise: bool,
}

/// The horizontal area the unicorn may fly over (home metres): the world
/// set's tiles; it is softly stopped [`LIMIT_MARGIN_M`] outside.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub min: [f64; 2],
    pub max: [f64; 2],
}

impl Bounds {
    /// The box grown by `m` metres on every side.
    pub fn grown(self, m: f64) -> Self {
        Self {
            min: [self.min[0] - m, self.min[1] - m],
            max: [self.max[0] + m, self.max[1] + m],
        }
    }

    pub fn contains(&self, p: [f64; 2]) -> bool {
        (self.min[0]..=self.max[0]).contains(&p[0]) && (self.min[1]..=self.max[1]).contains(&p[1])
    }

    /// The box around `points` (x, z of 3D points), `None` without any.
    pub fn around(points: impl IntoIterator<Item = [f64; 3]>) -> Option<Self> {
        let mut b: Option<Self> = None;
        for p in points {
            let q = [p[0], p[2]];
            b = Some(match b {
                None => Self { min: q, max: q },
                Some(b) => Self {
                    min: [b.min[0].min(q[0]), b.min[1].min(q[1])],
                    max: [b.max[0].max(q[0]), b.max[1].max(q[1])],
                },
            });
        }
        b
    }
}

/// The unicorn's pose: where it is and where the rider looks.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct GodPose {
    /// Local metres (x, y up, z).
    pub pos: [f64; 3],
    pub yaw: f64,
    pub pitch: f64,
    /// Flight speed (m/s, 3D).
    pub speed: f64,
}

/// What a flight step found below the unicorn.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct StepInfo {
    /// Height of the surface below (m above sea level), if known.
    pub ground: Option<f64>,
    /// Height above that surface (m).
    pub height_m: f64,
    /// The speed the input asked for (m/s).
    pub target_speed: f64,
    /// Whether the clearance lifted the unicorn this step.
    pub lifted: bool,
}

/// The flight state of the unicorn (client side: the flight is driven by
/// the client, spec §14 Phase 13E).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Flight {
    pub pos: [f64; 3],
    pub yaw: f64,
    pub pitch: f64,
    /// Smoothed velocity (m/s; x, y, z).
    pub vel: [f64; 3],
    /// The wheel's scale of the base speed.
    pub scale: f64,
    /// Turn rate of the yaw (rad/s, smoothed): the wings bank with it.
    pub turn_rate: f64,
    /// Rate of change of the speed (m/s², smoothed): the wings beat with it.
    pub accel: f64,
}

impl Flight {
    /// A flight starting at `pos`, looking along `yaw` (level).
    pub fn new(pos: [f64; 3], yaw: f64) -> Self {
        Self {
            pos,
            yaw,
            pitch: -0.12,
            vel: [0.0; 3],
            scale: 1.0,
            turn_rate: 0.0,
            accel: 0.0,
        }
    }

    pub fn pose(&self) -> GodPose {
        GodPose {
            pos: self.pos,
            yaw: self.yaw,
            pitch: self.pitch,
            speed: self.speed(),
        }
    }

    /// Speed (m/s, 3D).
    pub fn speed(&self) -> f64 {
        (self.vel[0].powi(2) + self.vel[1].powi(2) + self.vel[2].powi(2)).sqrt()
    }

    /// Horizontal speed (m/s).
    pub fn horizontal_speed(&self) -> f64 {
        self.vel[0].hypot(self.vel[2])
    }

    /// Turns the view by `d_yaw` (unlimited) and `d_pitch` (rad; the pitch
    /// stays within ±[`MAX_PITCH`]).
    pub fn look(&mut self, d_yaw: f64, d_pitch: f64) {
        self.yaw = (self.yaw + d_yaw).rem_euclid(std::f64::consts::TAU);
        self.pitch = (self.pitch + d_pitch).clamp(-MAX_PITCH, MAX_PITCH);
    }

    /// Mouse wheel steps (positive = faster): the base-speed scale.
    pub fn scroll(&mut self, steps: f64) {
        self.scale = (self.scale * SCALE_STEP.powf(steps)).clamp(MIN_SCALE, MAX_SCALE);
    }

    /// The velocity the input asks for at `height_m` above the ground,
    /// inside `bounds` (the soft stop): horizontal relative to the yaw only,
    /// the pitch never dives.
    pub fn target_velocity(
        &self,
        input: &FlightInput,
        height_m: f64,
        bounds: Option<Bounds>,
    ) -> [f64; 3] {
        let v = flight_speed(height_m, input.boost, input.precise, self.scale);
        let (f, r) = (forward(self.yaw), right(self.yaw));
        let fw = input.forward.clamp(-1.0, 1.0);
        let st = input.strafe.clamp(-1.0, 1.0);
        let mut h = [f[0] * fw + r[0] * st, f[1] * fw + r[1] * st];
        let n = h[0].hypot(h[1]);
        if n > 1.0 {
            h = [h[0] / n, h[1] / n];
        }
        let mut t = [
            h[0] * v,
            input.climb.clamp(-1.0, 1.0) * v * CLIMB_FRACTION,
            h[1] * v,
        ];
        if let Some(b) = bounds {
            // Softly stopped outside the world: the outward speed fades to
            // nothing over the margin.
            let p = [self.pos[0], self.pos[2]];
            for (axis, k) in [(0usize, 0usize), (1, 2)] {
                let over_max = p[axis] - b.max[axis];
                let over_min = b.min[axis] - p[axis];
                if over_max > 0.0 && t[k] > 0.0 {
                    t[k] *= (1.0 - over_max / LIMIT_MARGIN_M).clamp(0.0, 1.0);
                }
                if over_min > 0.0 && t[k] < 0.0 {
                    t[k] *= (1.0 - over_min / LIMIT_MARGIN_M).clamp(0.0, 1.0);
                }
            }
        }
        t
    }

    /// Advances the flight by `dt` seconds. `surface` is the height of
    /// what the unicorn must stay above (terrain, water, buildings) at
    /// (x, z); `bounds` the world set's area (`None`: unlimited).
    pub fn step(
        &mut self,
        input: &FlightInput,
        dt: f64,
        surface: &dyn Fn(f64, f64) -> Option<f64>,
        bounds: Option<Bounds>,
    ) -> StepInfo {
        let dt = dt.clamp(0.0, 0.25);
        let ground = surface(self.pos[0], self.pos[2]);
        let height = self.pos[1] - ground.unwrap_or(0.0);
        let target = self.target_velocity(input, height, bounds);
        let target_speed = norm(target);
        let before = self.speed();
        let yaw_before = self.yaw;
        let tau = if target_speed >= before {
            ACCEL_TAU_S
        } else {
            STOP_TAU_S
        };
        let a = 1.0 - (-dt / tau).exp();
        for (v, t) in self.vel.iter_mut().zip(target) {
            *v += (t - *v) * a;
        }
        let from = self.pos;
        for (p, v) in self.pos.iter_mut().zip(self.vel) {
            *p += v * dt;
        }
        if let Some(b) = bounds {
            let b = b.grown(LIMIT_MARGIN_M);
            self.pos[0] = self.pos[0].clamp(b.min[0], b.max[0]);
            self.pos[2] = self.pos[2].clamp(b.min[1], b.max[1]);
        }
        // Clearance: never below the surface along this step and the next
        // one (at this velocity; sampled every metre), so the straight
        // line between two frames clears it too …
        let highest = |a: [f64; 3], b: [f64; 3]| {
            let n = ((b[0] - a[0]).hypot(b[2] - a[2]).ceil() as usize).clamp(1, 128);
            (0..=n)
                .filter_map(|k| {
                    let t = k as f64 / n as f64;
                    surface(a[0] + (b[0] - a[0]) * t, a[2] + (b[2] - a[2]) * t)
                })
                .fold(None, |m: Option<f64>, h| Some(m.map_or(h, |m| m.max(h))))
        };
        let next: [f64; 3] = std::array::from_fn(|k| self.pos[k] + self.vel[k] * dt);
        let floor = match (highest(from, self.pos), highest(self.pos, next)) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        }
        .map(|h| h + CLEARANCE_M);
        // … and gliding up to what lies ahead.
        let ahead = (1..=12)
            .filter_map(|k| {
                let t = LOOK_AHEAD_S * f64::from(k) / 12.0;
                surface(self.pos[0] + self.vel[0] * t, self.pos[2] + self.vel[2] * t)
            })
            .fold(None, |m: Option<f64>, h| Some(m.map_or(h, |m| m.max(h))))
            .map(|h| h + CLEARANCE_M);
        let mut lifted = false;
        if let Some(h) = ahead
            && self.pos[1] < h
        {
            self.pos[1] += (h - self.pos[1]) * (1.0 - (-dt / RISE_TAU_S).exp());
            lifted = true;
        }
        if let Some(h) = floor
            && self.pos[1] < h
        {
            self.pos[1] = h;
            self.vel[1] = self.vel[1].max(0.0);
            lifted = true;
        }
        if self.pos[1] > CEILING_M {
            self.pos[1] = CEILING_M;
            self.vel[1] = self.vel[1].min(0.0);
        }
        if dt > 0.0 {
            let k = 1.0 - (-dt / 0.2).exp();
            let turn = wrap_angle(self.yaw - yaw_before) / dt;
            self.turn_rate += (turn - self.turn_rate) * k;
            self.accel += ((self.speed() - before) / dt - self.accel) * k;
        }
        let ground = surface(self.pos[0], self.pos[2]);
        StepInfo {
            ground,
            height_m: self.pos[1] - ground.unwrap_or(0.0),
            target_speed,
            lifted,
        }
    }
}

fn norm(v: [f64; 3]) -> f64 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

/// `a` wrapped into −π … π.
pub fn wrap_angle(a: f64) -> f64 {
    let t = std::f64::consts::TAU;
    (a + std::f64::consts::PI).rem_euclid(t) - std::f64::consts::PI
}

/// Where a glide goes: down to the landing spot, or back to where the
/// rider took off (Shift + G).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlideKind {
    Land,
    FlyBack,
}

/// An eased flight from `from` to `to` (spec §14 Phase 13A: at most 2 s
/// however far; flying back arcs over the land between).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glide {
    pub kind: GlideKind,
    pub from: [f64; 3],
    pub to: [f64; 3],
    pub duration_s: f64,
    /// Time since the glide began (s).
    pub t: f64,
    /// Height of the arc over the straight line at its middle (m).
    pub arc_m: f64,
}

/// How long a glide over `distance_m` takes (s): quick for short ones, at
/// most [`MAX_GLIDE_S`].
pub fn glide_duration(distance_m: f64) -> f64 {
    (0.8 + distance_m.max(0.0) / 300.0).min(MAX_GLIDE_S)
}

impl Glide {
    pub fn new(kind: GlideKind, from: [f64; 3], to: [f64; 3]) -> Self {
        let d = norm([to[0] - from[0], to[1] - from[1], to[2] - from[2]]);
        let arc_m = match kind {
            GlideKind::Land => 0.0,
            GlideKind::FlyBack => (d * 0.15).min(400.0),
        };
        Self {
            kind,
            from,
            to,
            duration_s: glide_duration(d),
            t: 0.0,
            arc_m,
        }
    }

    /// 0 … 1, eased (smoothstep).
    pub fn progress(&self) -> f64 {
        let u = if self.duration_s > 0.0 {
            (self.t / self.duration_s).clamp(0.0, 1.0)
        } else {
            1.0
        };
        u * u * (3.0 - 2.0 * u)
    }

    pub fn done(&self) -> bool {
        self.t >= self.duration_s
    }

    pub fn advance(&mut self, dt: f64) {
        self.t += dt.max(0.0);
    }

    /// The position now.
    pub fn position(&self) -> [f64; 3] {
        let e = self.progress();
        if e >= 1.0 {
            return self.to;
        }
        let mut p: [f64; 3] =
            std::array::from_fn(|k| self.from[k] + (self.to[k] - self.from[k]) * e);
        p[1] += self.arc_m * (std::f64::consts::PI * e).sin();
        p
    }

    /// The horizontal direction of the glide (x, z), if it moves.
    pub fn direction(&self) -> Option<[f64; 2]> {
        let d = [self.to[0] - self.from[0], self.to[2] - self.from[2]];
        let n = d[0].hypot(d[1]);
        (n > 0.5).then(|| [d[0] / n, d[1] / n])
    }

    /// Moves the end (the server placed the rider elsewhere than the
    /// client guessed): the glide continues from where it is now.
    pub fn retarget(&mut self, to: [f64; 3]) {
        let here = self.position();
        let left = (self.duration_s - self.t).max(0.3);
        *self = Self {
            from: here,
            to,
            duration_s: left,
            t: 0.0,
            arc_m: 0.0,
            ..*self
        };
    }
}

/// The point of the polyline `points` nearest to `p` (x, z): its distance
/// along the polyline (horizontal, as routes measure) and the point.
pub fn nearest_on_polyline(points: &[[f64; 3]], p: [f64; 2]) -> Option<(f64, [f64; 3])> {
    let mut best: Option<(f64, f64, [f64; 3])> = None;
    let mut along = 0.0;
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        let ab = [b[0] - a[0], b[2] - a[2]];
        let len2 = ab[0] * ab[0] + ab[1] * ab[1];
        let len = len2.sqrt();
        let t = if len2 > 0.0 {
            (((p[0] - a[0]) * ab[0] + (p[1] - a[2]) * ab[1]) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let q: [f64; 3] = std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t);
        let d2 = (q[0] - p[0]).powi(2) + (q[2] - p[1]).powi(2);
        if best.is_none_or(|(bd, ..)| d2 < bd) {
            best = Some((d2, along + len * t, q));
        }
        along += len;
    }
    if points.len() == 1 {
        return Some((0.0, points[0]));
    }
    best.map(|(_, s, q)| (s, q))
}

/// The point of a route on the road graph nearest to `p` (x, z): metres
/// along the route (one lap) and the point on the road.
pub fn nearest_on_plan(
    net: &RoadNetwork,
    plan: &RoutePlan,
    p: [f64; 2],
) -> Option<(f64, [f64; 3])> {
    let mut best: Option<(f64, f64, [f64; 3])> = None;
    let mut base = 0.0;
    for step in &plan.steps {
        let Segment::Edge { edge, from_m, to_m } = *step else {
            continue;
        };
        let pts = net.geometry(edge).slice(from_m, to_m);
        if let Some((s, q)) = nearest_on_polyline(&pts, p) {
            let d2 = (q[0] - p[0]).powi(2) + (q[2] - p[1]).powi(2);
            if best.is_none_or(|(bd, ..)| d2 < bd) {
                best = Some((d2, base + s, q));
            }
        }
        base += (to_m - from_m).abs();
    }
    best.map(|(_, s, q)| (s, q))
}

/// The distance to place a rider at on a course of `length` (a loop: the
/// same lap as `current`, else the one after if that is nearer ahead) for
/// the point `along` metres into a lap.
pub fn lap_distance(current: f64, along: f64, length: f64, is_loop: bool) -> f64 {
    if !is_loop || length <= 0.0 {
        return along.max(0.0);
    }
    let lap = (current / length).floor().max(0.0);
    (lap * length + along).max(0.0)
}

/// Why the server refused a pose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoseError {
    NotFinite,
    TooHigh,
    OutsideWorld,
    TooFast,
}

impl PoseError {
    pub fn describe(self) -> &'static str {
        match self {
            Self::NotFinite => "not a number",
            Self::TooHigh => "above the ceiling",
            Self::OutsideWorld => "outside the world",
            Self::TooFast => "faster than a unicorn flies",
        }
    }
}

/// Slack of the server's check: the poses travel over the network (jitter,
/// bunching) and the server measures time by its own ticks.
pub const POSE_SLACK_M: f64 = 30.0;
pub const POSE_SLACK_S: f64 = 0.15;

/// The server's plausibility check of a pose (spec §14 Phase 13E): finite,
/// at most the ceiling, inside the world (`area`, with the soft stop's
/// margin; `None`: not checked), and not farther from the last accepted
/// pose `prev` than the fastest flight in the time `dt_s` between them.
pub fn check_pose(
    prev: Option<[f64; 3]>,
    dt_s: f64,
    pose: &GodPose,
    area: Option<Bounds>,
) -> Result<(), PoseError> {
    let finite = pose.pos.iter().all(|v| v.is_finite())
        && pose.yaw.is_finite()
        && pose.pitch.is_finite()
        && pose.speed.is_finite();
    if !finite {
        return Err(PoseError::NotFinite);
    }
    if pose.pos[1] > CEILING_M + POSE_SLACK_M {
        return Err(PoseError::TooHigh);
    }
    if let Some(a) = area
        && !a
            .grown(LIMIT_MARGIN_M + POSE_SLACK_M)
            .contains([pose.pos[0], pose.pos[2]])
    {
        return Err(PoseError::OutsideWorld);
    }
    if let Some(p) = prev {
        let d = norm([pose.pos[0] - p[0], pose.pos[1] - p[1], pose.pos[2] - p[2]]);
        let allowed = MAX_SPEED_MPS * 1.15 * (dt_s.max(0.0) + POSE_SLACK_S) + POSE_SLACK_M;
        if d > allowed {
            return Err(PoseError::TooFast);
        }
    }
    Ok(())
}

/// Wing beats per second (the animation and the wing sound): fast when
/// climbing or speeding up (up to ~2 Hz), slow while cruising, none while
/// gliding down.
pub fn wing_beat_hz(climb_mps: f64, accel_mps2: f64, speed_mps: f64) -> f64 {
    let climb = (climb_mps / 15.0).clamp(0.0, 1.0);
    let accel = (accel_mps2 / 40.0).clamp(0.0, 1.0);
    let cruise = if speed_mps > 2.0 { 0.35 } else { 0.0 };
    let gliding_down = climb_mps < -3.0;
    if gliding_down {
        return 0.0;
    }
    (cruise + 1.65 * climb.max(accel)).min(2.0)
}

/// Gallop strides per second: following the speed (legs tucked in at
/// high speed, see [`legs_tucked`]), a slow paddle while hovering.
pub fn gallop_hz(speed_mps: f64) -> f64 {
    (0.6 + speed_mps / 25.0).min(2.4)
}

/// How far the legs are tucked in (0 … 1) at `speed_mps`.
pub fn legs_tucked(speed_mps: f64) -> f64 {
    ((speed_mps - 60.0) / 140.0).clamp(0.0, 1.0)
}

/// How far the wings are spread (0 folded … 1 wide): folded while
/// hovering (and landing, by the caller), spread in flight and turns.
pub fn wing_spread(speed_mps: f64, turn_rate: f64) -> f64 {
    let flying = (speed_mps / 12.0).clamp(0.0, 1.0);
    (flying * 0.85 + (turn_rate.abs() / 0.8).clamp(0.0, 1.0) * 0.15).clamp(0.0, 1.0)
}

/// The bank of the body in a turn (rad, positive = right wing down):
/// turning left (positive turn rate) banks to the left.
pub fn bank(turn_rate: f64, speed_mps: f64) -> f64 {
    let k = (speed_mps / 30.0).clamp(0.0, 1.0);
    (-turn_rate * 0.45 * k).clamp(-0.6, 0.6)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(h: f64) -> impl Fn(f64, f64) -> Option<f64> {
        move |_, _| Some(h)
    }

    /// Runs `seconds` of flight at 60 fps.
    fn fly(
        f: &mut Flight,
        input: FlightInput,
        seconds: f64,
        surface: &dyn Fn(f64, f64) -> Option<f64>,
        bounds: Option<Bounds>,
    ) -> Vec<StepInfo> {
        let dt = 1.0 / 60.0;
        (0..(seconds / dt).round() as usize)
            .map(|_| f.step(&input, dt, surface, bounds))
            .collect()
    }

    const AHEAD: FlightInput = FlightInput {
        forward: 1.0,
        strafe: 0.0,
        climb: 0.0,
        boost: false,
        precise: false,
    };

    #[test]
    fn speed_grows_with_height_and_is_clamped() {
        assert_eq!(base_speed(0.0), 20.0);
        assert_eq!(base_speed(-5.0), 20.0);
        // 252 km/h at 100 m, 792 km/h at 400 m.
        assert!((base_speed(100.0) - 70.0).abs() < 1e-9);
        assert!((base_speed(400.0) - 220.0).abs() < 1e-9);
        assert_eq!(base_speed(760.0), 400.0);
        assert_eq!(base_speed(5_000.0), 400.0);
    }

    #[test]
    fn boost_precise_and_wheel_factors() {
        assert_eq!(flight_speed(0.0, true, false, 1.0), 80.0);
        assert_eq!(flight_speed(0.0, false, true, 1.0), 5.0);
        assert_eq!(flight_speed(0.0, true, true, 1.0), 20.0);
        assert_eq!(flight_speed(0.0, false, false, 8.0), 160.0);
        assert_eq!(flight_speed(0.0, false, false, 0.25), 5.0);
        // Never faster than the cap, whatever the wheel says.
        assert_eq!(flight_speed(2_000.0, true, false, 8.0), MAX_SPEED_MPS);
        // The wheel: 16 steps from 1 to 8 … and it stops there.
        let mut f = Flight::new([0.0; 3], 0.0);
        f.scroll(9.0);
        assert!((f.scale - SCALE_STEP.powi(9)).abs() < 1e-9);
        f.scroll(100.0);
        assert_eq!(f.scale, MAX_SCALE);
        f.scroll(-100.0);
        assert_eq!(f.scale, MIN_SCALE);
    }

    #[test]
    fn accelerates_in_015_s_and_stops_in_025_s() {
        let ground = flat(0.0);
        let mut f = Flight::new([0.0, 2.0, 0.0], 0.0);
        let full = base_speed(2.0);
        // One time constant: 63 % of the speed.
        fly(&mut f, AHEAD, ACCEL_TAU_S, &ground, None);
        let v = f.horizontal_speed();
        assert!((v / full - 0.632).abs() < 0.03, "{v}");
        fly(&mut f, AHEAD, 1.5, &ground, None);
        assert!((f.horizontal_speed() - full).abs() < 0.05);
        fly(&mut f, FlightInput::default(), STOP_TAU_S, &ground, None);
        let v = f.horizontal_speed();
        assert!((v / full - 0.368).abs() < 0.03, "{v}");
        fly(&mut f, FlightInput::default(), 3.0, &ground, None);
        assert!(f.horizontal_speed() < 0.01);
    }

    #[test]
    fn moves_relative_to_the_yaw_in_the_horizontal_plane() {
        let ground = flat(0.0);
        for yaw in [0.0, 0.7, -2.0, 3.0] {
            for pitch in [-1.5, 0.0, 1.2] {
                let mut f = Flight::new([0.0, 100.0, 0.0], yaw);
                f.pitch = pitch;
                fly(&mut f, AHEAD, 2.0, &ground, None);
                let d = [f.pos[0], f.pos[2]];
                let n = d[0].hypot(d[1]);
                let fw = forward(yaw);
                assert!((d[0] / n - fw[0]).abs() < 1e-6 && (d[1] / n - fw[1]).abs() < 1e-6);
                // Looking down does not dive, looking up does not climb.
                assert!(
                    (f.pos[1] - 100.0).abs() < 1e-6,
                    "pitch {pitch}: {}",
                    f.pos[1]
                );
            }
        }
        // Strafing right is to the right of the view; yaw 0 looks north (−z).
        let mut f = Flight::new([0.0, 100.0, 0.0], 0.0);
        let input = FlightInput {
            strafe: 1.0,
            ..Default::default()
        };
        fly(&mut f, input, 1.0, &ground, None);
        assert!(f.pos[0] > 10.0 && f.pos[2].abs() < 1e-6);
        assert_eq!(forward(0.0), [-0.0, -1.0]);
        assert!((heading_deg(0.0) - 0.0).abs() < 1e-9);
        assert!(
            (heading_deg(yaw_of([1.0, 0.0])) - 90.0).abs() < 1e-9,
            "east"
        );
        // Diagonal is not faster than straight.
        let mut f = Flight::new([0.0, 100.0, 0.0], 0.0);
        let both = FlightInput {
            forward: 1.0,
            strafe: 1.0,
            ..Default::default()
        };
        fly(&mut f, both, 3.0, &ground, None);
        let v = f.horizontal_speed();
        assert!((v - base_speed(100.0)).abs() < 0.5, "{v}");
    }

    #[test]
    fn climbs_at_half_the_horizontal_speed() {
        let ground = flat(0.0);
        let mut f = Flight::new([0.0, 2.0, 0.0], 0.0);
        let up = FlightInput {
            climb: 1.0,
            ..Default::default()
        };
        fly(&mut f, up, 0.5, &ground, None);
        // At height h: v = 20 + 0.5 h; the climb rate is half of it.
        let h = f.pos[1];
        let infos = fly(&mut f, up, 1.0 / 60.0, &ground, None);
        let expected = base_speed(infos[0].height_m) * CLIMB_FRACTION;
        assert!(
            f.vel[1] > 0.0 && f.vel[1] <= expected + 1e-6,
            "{} vs {expected}",
            f.vel[1]
        );
        assert!(f.pos[1] > h);
        // Descending stops 2 m above the ground.
        let down = FlightInput {
            climb: -1.0,
            ..Default::default()
        };
        fly(&mut f, down, 30.0, &ground, None);
        assert!((f.pos[1] - CLEARANCE_M).abs() < 1e-6, "{}", f.pos[1]);
    }

    #[test]
    fn keeps_clear_of_a_ridge_at_full_speed() {
        // A sharp ridge 300 m high at x = 3 km, the unicorn flying east 5 m
        // above the plain with boost and the widest scale.
        let ridge = |x: f64, _z: f64| Some(300.0 * (1.0 - ((x - 3_000.0).abs() / 150.0)).max(0.0));
        let mut f = Flight::new([0.0, 5.0, 0.0], yaw_of([1.0, 0.0]));
        f.scale = MAX_SCALE;
        let input = FlightInput {
            forward: 1.0,
            boost: true,
            ..Default::default()
        };
        let dt = 1.0 / 60.0;
        let mut crossed = false;
        for _ in 0..600 {
            let from = f.pos;
            f.step(&input, dt, &ridge, None);
            // Never into the ridge, also between the frames (sampled
            // every metre along a step, 2 m slope: within the clearance).
            for k in 0..=8 {
                let t = f64::from(k) / 8.0;
                let x = from[0] + (f.pos[0] - from[0]) * t;
                let y = from[1] + (f.pos[1] - from[1]) * t;
                let h = ridge(x, 0.0).unwrap();
                assert!(y >= h + CLEARANCE_M - 2.0, "x {x:.0}: {y:.1} over {h:.1}");
            }
            assert!(f.pos[1] >= ridge(f.pos[0], 0.0).unwrap() + CLEARANCE_M - 1e-9);
            crossed |= f.pos[0] > 3_200.0;
        }
        assert!(crossed, "it flew over the ridge ({:.0})", f.pos[0]);
    }

    #[test]
    fn stays_under_the_ceiling() {
        let mut f = Flight::new([0.0, 7_990.0, 0.0], 0.0);
        let up = FlightInput {
            climb: 1.0,
            boost: true,
            ..Default::default()
        };
        fly(&mut f, up, 5.0, &flat(0.0), None);
        assert_eq!(f.pos[1], CEILING_M);
    }

    #[test]
    fn softly_stopped_outside_the_world() {
        let b = Bounds {
            min: [0.0, 0.0],
            max: [1_000.0, 1_000.0],
        };
        let mut f = Flight::new([500.0, 50.0, 500.0], yaw_of([1.0, 0.0]));
        let input = FlightInput {
            forward: 1.0,
            boost: true,
            ..Default::default()
        };
        fly(&mut f, input, 30.0, &flat(0.0), Some(b));
        assert!(f.pos[0] > 1_000.0, "beyond the edge a bit");
        assert!(f.pos[0] <= 1_000.0 + LIMIT_MARGIN_M + 1e-9, "{}", f.pos[0]);
        assert!(
            f.horizontal_speed() < 5.0,
            "stopped: {}",
            f.horizontal_speed()
        );
        // Turning back is at full speed.
        f.yaw = yaw_of([-1.0, 0.0]);
        fly(&mut f, input, 2.0, &flat(0.0), Some(b));
        assert!(f.pos[0] < 1_000.0, "{}", f.pos[0]);
    }

    #[test]
    fn look_limits_pitch_and_wraps_yaw() {
        let mut f = Flight::new([0.0; 3], 0.0);
        f.look(10.0, 5.0);
        assert_eq!(f.pitch, MAX_PITCH);
        assert!((0.0..std::f64::consts::TAU).contains(&f.yaw));
        f.look(0.0, -10.0);
        assert_eq!(f.pitch, -MAX_PITCH);
    }

    #[test]
    fn glides_take_at_most_two_seconds_and_end_on_the_spot() {
        let from = [0.0, 300.0, 0.0];
        let to = [50_000.0, 500.0, 0.0];
        let mut g = Glide::new(GlideKind::FlyBack, from, to);
        assert_eq!(g.duration_s, MAX_GLIDE_S);
        assert_eq!(g.position(), from);
        g.advance(1.0);
        let mid = g.position();
        assert!(mid[1] > 400.0 + 100.0, "arcs over the land: {}", mid[1]);
        g.advance(1.5);
        assert!(g.done());
        assert_eq!(g.position(), to);
        // A short landing is quick and straight down.
        let g = Glide::new(GlideKind::Land, [0.0, 20.0, 0.0], [3.0, 0.0, 0.0]);
        assert!(g.duration_s < 1.0);
        assert_eq!(g.arc_m, 0.0);
        let mut g = Glide::new(GlideKind::Land, [0.0, 20.0, 0.0], [3.0, 0.0, 0.0]);
        g.advance(0.2);
        g.retarget([10.0, 1.0, 0.0]);
        g.advance(5.0);
        assert_eq!(g.position(), [10.0, 1.0, 0.0]);
    }

    #[test]
    fn nearest_point_of_a_polyline() {
        let pts = [[0.0, 0.0, 0.0], [100.0, 10.0, 0.0], [100.0, 10.0, 100.0]];
        let (s, q) = nearest_on_polyline(&pts, [40.0, 30.0]).unwrap();
        assert!((s - 40.0).abs() < 1e-9 && (q[1] - 4.0).abs() < 1e-9);
        let (s, q) = nearest_on_polyline(&pts, [130.0, 60.0]).unwrap();
        assert!((s - 160.0).abs() < 1e-9 && q[0] == 100.0);
        assert!(nearest_on_polyline(&[], [0.0, 0.0]).is_none());
        // Loops keep the lap.
        assert_eq!(lap_distance(2_500.0, 300.0, 1_000.0, true), 2_300.0);
        assert_eq!(lap_distance(2_500.0, 300.0, 1_000.0, false), 300.0);
    }

    #[test]
    fn the_server_refuses_implausible_poses() {
        let area = Some(Bounds {
            min: [0.0, 0.0],
            max: [10_000.0, 10_000.0],
        });
        let at = |x: f64, y: f64| GodPose {
            pos: [x, y, 5_000.0],
            ..Default::default()
        };
        assert_eq!(check_pose(None, 0.0, &at(10.0, 100.0), area), Ok(()));
        // 100 ms at the fastest flight: fine; 2 km in 100 ms: too fast.
        assert_eq!(
            check_pose(Some([0.0, 100.0, 5_000.0]), 0.1, &at(160.0, 100.0), area),
            Ok(())
        );
        assert_eq!(
            check_pose(Some([0.0, 100.0, 5_000.0]), 0.1, &at(2_000.0, 100.0), area),
            Err(PoseError::TooFast)
        );
        assert_eq!(
            check_pose(None, 0.0, &at(10.0, 9_000.0), area),
            Err(PoseError::TooHigh)
        );
        assert_eq!(
            check_pose(None, 0.0, &at(-1_000.0, 100.0), area),
            Err(PoseError::OutsideWorld)
        );
        // Within the soft stop's margin is fine.
        assert_eq!(check_pose(None, 0.0, &at(-150.0, 100.0), area), Ok(()));
        assert_eq!(
            check_pose(None, 0.0, &at(f64::NAN, 100.0), area),
            Err(PoseError::NotFinite)
        );
    }

    #[test]
    fn wings_and_gallop_follow_the_flight() {
        assert_eq!(wing_beat_hz(0.0, 0.0, 0.0), 0.0);
        assert!(wing_beat_hz(20.0, 0.0, 30.0) <= 2.0 && wing_beat_hz(20.0, 0.0, 30.0) > 1.5);
        assert!(wing_beat_hz(0.0, 50.0, 30.0) > 1.5);
        assert_eq!(wing_beat_hz(-10.0, 0.0, 30.0), 0.0, "glides down");
        assert!(gallop_hz(100.0) > gallop_hz(10.0));
        assert_eq!(legs_tucked(20.0), 0.0);
        assert_eq!(legs_tucked(400.0), 1.0);
        assert_eq!(wing_spread(0.0, 0.0), 0.0, "folded while hovering");
        assert!(wing_spread(50.0, 1.0) > 0.95);
        assert!(bank(0.5, 50.0) < 0.0, "turning left banks left");
    }
}
