//! Riding off the road (spec §14 Phase 13, 13A *Free 2D riding off the
//! road*, v5.52; Q7 (b), Q8 (a), Q9 (a)): a rider who landed from the
//! unicorn on a meadow – or rode out of wild mode's 60 m band in free ride
//! – moves in 2D on the terrain. Pure and deterministic; the game server
//! and the client's prediction both ride it, so they agree.
//!
//! **The model.** The rider's position (x, z) and direction of travel are
//! an [`OffRoadTrack`]: a straight track along the heading through the
//! rider, its heights and gradient from the ground ([`GroundMap`]: the
//! terrain the renderer draws), its surface from the land cover. Each
//! physics tick ([`tick`]) rides one ordinary [`ride::tick_in`] step on
//! it – the gradient along the heading, the cover's rolling resistance,
//! the weather, lean steering (wild mode's; autodrive rides straight) –
//! and re-anchors the track where the rider got to, turned by the heading
//! ridden ([`normalize`]). So everything that reads a track (rendering,
//! the HUD's gradient, the trainer, the GPS track, the FIT file, the
//! collisions in the world's plane) works off the road too. The distance
//! goes on counting (the odometer: what the ride records, Q9 (a)).
//!
//! **Every slope at its real gradient (Q7 (b)).** No "too steep" stop and
//! no speed cap: 40 % up is ridden at the speed the power gives (the
//! physics never rolls back), 40 % down accelerates as gravity says. Safe
//! on cliffs: the gradient is finite (the ground's grid, at most
//! [`ocw_world::ground::MAX_GRADE`]), the rider follows the terrain (never
//! in the air), the horizontal step is the slope's length times its
//! cosine, and the speed is bounded by [`MAX_SPEED_MPS`] – 2 m a tick – so
//! nothing can jump (air drag stops a rider well below it on any real
//! slope).
//!
//! **Water and the world's edge** stop the rider (the step is not taken,
//! the speed is lost); leaning turns the bike on the spot there
//! ([`TURN_ON_SPOT_RAD_S`]) so the rider can ride away. **Back on a road**
//! ([`road_join`]): on a road's surface (within half its width, at the
//! road's height – not under a bridge, not over a tunnel) and at most
//! [`JOIN_MAX_ANGLE`] to it the rider joins it: a new path along the road
//! in the direction of travel, the lateral offset and heading where the
//! rider is. Crossing a road more steeply rides over it.

use std::sync::Arc;

use ocw_core::{RiderId, RiderProfile};
use ocw_world::ground::{Cover, GroundMap, OffRoadTrack};
use ocw_world::{RoadNetwork, Track};

use crate::collide::{self, Body, WorldCache};
use crate::physics::{self, RiderState, Steering};
use crate::ride::{self, Controls};
use crate::weather::WeatherPhysics;

/// The fastest a rider moves off the road (m/s; 2 m a tick): only a
/// numerical bound, far above what air drag allows on any slope (~60 m/s
/// down a cliff), never a speed cap felt in riding.
pub const MAX_SPEED_MPS: f64 = 100.0;
/// Turning on the spot while stopped by water or the world's edge (rad/s,
/// in the direction of the lean): the rider wheels the bike round.
pub const TURN_ON_SPOT_RAD_S: f64 = 1.0;
/// Largest angle to a road at which a rider on it joins it (rad, ~57°);
/// more steeply the rider rides across.
pub const JOIN_MAX_ANGLE: f64 = 1.0;
/// How far the rider's height may be from the road's to join it (m): not a
/// bridge overhead or a tunnel below.
pub const JOIN_MAX_DY_M: f64 = 1.5;
/// A landing within this much beyond a road's half width lands on the road
/// (spec 13A *Landing anywhere*: the road's width + 2 m).
pub const ROAD_LANDING_MARGIN_M: f64 = 2.0;

/// Where the rider is off the road: position (x, z, local metres) and the
/// direction of travel as a yaw (rad, the unicorn's convention:
/// counter-clockwise from north seen from above, [`crate::god::yaw_of`]).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct OffRoadPose {
    pub x: f64,
    pub z: f64,
    pub yaw: f64,
}

impl OffRoadPose {
    /// The pose of a track (where the rider is and the travel direction).
    pub fn of(track: &OffRoadTrack) -> Self {
        let at = track.at();
        Self {
            x: at[0],
            z: at[1],
            yaw: crate::god::yaw_of(track.direction()),
        }
    }

    /// The track through the pose, its distance `distance` there.
    pub fn track(&self, ground: Arc<GroundMap>, distance: f64) -> OffRoadTrack {
        OffRoadTrack::new(
            ground,
            [self.x, self.z],
            crate::god::forward(self.yaw),
            distance,
        )
    }

    /// Linear between two poses (the yaw the short way round).
    pub fn lerp(a: Self, b: Self, t: f64) -> Self {
        let mut dy = b.yaw - a.yaw;
        dy -= std::f64::consts::TAU * (dy / std::f64::consts::TAU).round();
        Self {
            x: a.x + (b.x - a.x) * t,
            z: a.z + (b.z - a.z) * t,
            yaw: a.yaw + dy * t,
        }
    }
}

/// Turns a horizontal direction by `heading` (rad, positive = right).
fn turned(dir: [f64; 2], heading: f64) -> [f64; 2] {
    let (s, c) = heading.sin_cos();
    [dir[0] * c - dir[1] * s, dir[0] * s + dir[1] * c]
}

/// Folds the rider's lateral offset and heading on `track` into a new
/// track through where the rider is, along where they ride: afterwards
/// `lateral_offset` and `heading` are 0. Anything that moved the rider on
/// the track (a collision's push, a crash's slide) is kept this way.
pub fn normalize(state: &mut RiderState, track: &OffRoadTrack) -> OffRoadTrack {
    let p = track.sample(state.distance);
    let at = p.offset_position(state.lateral_offset);
    let dir = turned(track.direction(), state.heading);
    state.lateral_offset = 0.0;
    state.heading = 0.0;
    OffRoadTrack::new(track.ground().clone(), [at[0], at[2]], dir, state.distance)
}

/// Whether the rider may be at (x, z): on the world's tiles and on
/// rideable ground (not water, not inside a building).
pub fn passable(ground: &GroundMap, x: f64, z: f64) -> bool {
    ground.cover(x, z).is_some_and(Cover::rideable)
}

/// One physics tick off the road (see the module). Returns the track for
/// the next tick (through where the rider is now) and the height climbed
/// (m, for the hearts: the real rise, at most 2 m).
#[allow(clippy::too_many_arguments)]
pub fn tick(
    state: &mut RiderState,
    track: &OffRoadTrack,
    profile: &RiderProfile,
    power_w: f64,
    controls: Controls,
    scraping: &mut bool,
    drag_factor: f64,
    weather: Option<&WeatherPhysics>,
    dt: f64,
) -> (OffRoadTrack, f64) {
    let tr = normalize(state, track);
    let y0 = tr.sample(state.distance).position[1];
    // Off the road there is no line to hold: autodrive rides straight,
    // leaning steers (wild mode's limits never bind: the track turns
    // with the rider every tick).
    let steering = match controls.steering {
        Steering::Auto(_) => Steering::Auto(0.0),
        s => s,
    };
    let controls = Controls {
        steering,
        wild: true,
        ..controls
    };
    let before = *state;
    ride::tick_in(
        state,
        profile,
        power_w,
        controls,
        scraping,
        &tr,
        drag_factor,
        weather,
        dt,
    );
    *scraping = false;
    if !state.speed.is_finite() || !state.distance.is_finite() {
        *state = RiderState {
            speed: 0.0,
            ..before
        };
    }
    state.speed = state.speed.clamp(0.0, MAX_SPEED_MPS);
    // At most the bound's step along the track.
    let max_step = MAX_SPEED_MPS * dt;
    if state.distance - before.distance > max_step {
        state.distance = before.distance + max_step;
    }
    let to = tr
        .sample(state.distance)
        .offset_position(state.lateral_offset);
    let ground = tr.ground();
    if !passable(ground, to[0], to[2]) {
        // Water or the world's edge: the step is not taken; leaning
        // wheels the bike round on the spot.
        let turn = match controls.steering {
            Steering::Lean(l) if l.abs() > 0.02 => l.signum() * TURN_ON_SPOT_RAD_S * dt,
            _ => 0.0,
        };
        *state = RiderState {
            speed: 0.0,
            turn_curvature: 0.0,
            heading: turn,
            ..before
        };
    }
    let next = normalize(state, &tr);
    let y1 = next.sample(state.distance).position[1];
    let rise = (y1 - y0).clamp(0.0, 2.0);
    (next, if rise.is_finite() { rise } else { 0.0 })
}

/// A rider off the road reached a road (see the module): where on the
/// graph they join it and how they ride on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Join {
    pub edge: usize,
    /// Metres from the edge's `from` vertex.
    pub offset_m: f64,
    /// Riding the edge from `from` to `to`.
    pub forward: bool,
    /// Lateral offset from the centre line (m, positive = right) and the
    /// heading to the road (rad, positive = right).
    pub lateral_m: f64,
    pub heading: f64,
}

/// The road the rider at (x, z, height `y`) riding along `dir` is on, if
/// they join it (see the module); `None` off every road, under a bridge,
/// over a tunnel, too steep an angle or against a one-way street.
pub fn road_join(net: &RoadNetwork, at: [f64; 2], y: f64, dir: [f64; 2]) -> Option<Join> {
    let (edge, s, dist, road_y) = net.nearest_within(at, 15.0)?;
    let e = net.edge(edge);
    if dist > e.width_m / 2.0 || (road_y - y).abs() > JOIN_MAX_DY_M {
        return None;
    }
    let g = net.geometry(edge);
    let len = g.length();
    let (a, b) = (
        g.point_at((s - 1.0).max(0.0)),
        g.point_at((s + 1.0).min(len)),
    );
    let along = [b[0] - a[0], b[2] - a[2]];
    let n = along[0].hypot(along[1]);
    if n < 1e-9 {
        return None;
    }
    let road = [along[0] / n, along[1] / n];
    let forward = road[0] * dir[0] + road[1] * dir[1] >= 0.0;
    if !e.oneway.allows(forward) {
        return None;
    }
    let travel = if forward { road } else { [-road[0], -road[1]] };
    // Heading relative to the road (positive = right, as `turned`).
    let cross = travel[0] * dir[1] - travel[1] * dir[0];
    let dot = travel[0] * dir[0] + travel[1] * dir[1];
    let heading = cross.atan2(dot);
    if heading.abs() > JOIN_MAX_ANGLE {
        return None;
    }
    let p = g.point_at(s);
    let right = [-travel[1], travel[0]];
    let lateral = (at[0] - p[0]) * right[0] + (at[1] - p[2]) * right[1];
    Some(Join {
        edge,
        offset_m: s,
        forward,
        lateral_m: lateral,
        heading,
    })
}

/// Where the unicorn lands (spec 13A, Q8 (a)): on a road, off it on the
/// ground, or nowhere (the ring red).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Landing {
    /// On or near a road (within its half width + [`ROAD_LANDING_MARGIN_M`]):
    /// the nearest point of the graph (edge, offset), standing there.
    Road {
        edge: usize,
        offset_m: f64,
        at: [f64; 3],
    },
    /// Off the road, on the ground at `at`.
    Ground { at: [f64; 3] },
    /// Not here (water, a building, an obstacle, outside the world's tiles).
    Refused { at: [f64; 3], why: &'static str },
}

impl Landing {
    /// The spot the landing ring marks.
    pub fn at(&self) -> [f64; 3] {
        match *self {
            Landing::Road { at, .. } | Landing::Ground { at } | Landing::Refused { at, .. } => at,
        }
    }

    pub fn allowed(&self) -> bool {
        !matches!(self, Landing::Refused { .. })
    }
}

/// A standing rider's footprint at (x, z) facing `yaw`, for the landing
/// check against the world's solids.
fn standing_body(x: f64, z: f64, yaw: f64) -> Body {
    Body {
        id: RiderId(0),
        pos: [x, z],
        road: crate::god::forward(yaw),
        heading: 0.0,
        speed: 0.0,
        lean: 0.0,
        mass: 80.0,
        fallen: false,
        manual: true,
        grip: 1.0,
    }
}

/// Whether a rider standing at (x, z, height `y`) facing `yaw` overlaps a
/// solid of the world (a tree, a wall, a fence, a pole, a building's wall).
pub fn blocked(world: &mut WorldCache, x: f64, z: f64, y: f64, yaw: f64, tick: u64) -> bool {
    if world.world.is_none() {
        return false;
    }
    let body = standing_body(x, z, yaw);
    let reach = collide::BODY_LENGTH_M / 2.0 + 0.5;
    let mut solids = Vec::new();
    world.solids_near(
        [x - reach, z - reach],
        [x + reach, z + reach],
        tick,
        &mut solids,
    );
    solids
        .iter()
        .filter(|s| s.reaches(y))
        .any(|s| collide::static_contact(&body, body.pos, [0.0, 0.0], s).is_some())
}

/// Where a unicorn over (x, z) facing `yaw` lands in a free ride (Q8 (a);
/// fixed routes and events land on the route, the session decides): on
/// the road when one is that near, else on the ground if the rider may
/// stand there. `world` checks the obstacles (`None`: not checked).
pub fn landing(
    net: &RoadNetwork,
    x: f64,
    z: f64,
    yaw: f64,
    world: Option<&mut WorldCache>,
    tick: u64,
) -> Landing {
    let lift = ocw_world::package::ROAD_LIFT_M;
    if let Some((edge, s, dist, y)) = net.nearest_within([x, z], 40.0)
        && dist <= net.edge(edge).width_m / 2.0 + ROAD_LANDING_MARGIN_M
    {
        let p = net.geometry(edge).point_at(s);
        return Landing::Road {
            edge,
            offset_m: s,
            at: [p[0], y + lift, p[2]],
        };
    }
    let Some(ground) = net.ground() else {
        // A world without tiles: only its roads.
        return match net.nearest([x, z]) {
            Some((edge, s)) => {
                let p = net.geometry(edge).point_at(s);
                Landing::Road {
                    edge,
                    offset_m: s,
                    at: [p[0], p[1] + lift, p[2]],
                }
            }
            None => Landing::Refused {
                at: [x, 0.0, z],
                why: "no ground here",
            },
        };
    };
    let Some(y) = ground.height(x, z) else {
        return Landing::Refused {
            at: [x, 0.0, z],
            why: "outside the world",
        };
    };
    let at = [x, y, z];
    match ground.cover(x, z) {
        // The ring on the water's surface.
        Some(Cover::Water) => Landing::Refused {
            at: [x, ground.water_level(x, z).unwrap_or(y).max(y), z],
            why: "not on water",
        },
        Some(Cover::Building) => Landing::Refused {
            at,
            why: "not on a building",
        },
        None => Landing::Refused {
            at,
            why: "outside the world",
        },
        Some(_) => {
            if world.is_some_and(|w| blocked(w, x, z, y, yaw, tick)) {
                Landing::Refused {
                    at,
                    why: "not on an obstacle",
                }
            } else {
                Landing::Ground { at }
            }
        }
    }
}

/// Whether a wild rider at the edge of wild mode's band (`lateral` m from
/// the road, heading `heading` to it) leaves it for the terrain (free ride
/// only; the session decides where).
pub fn leaves_band(lateral: f64, heading: f64) -> bool {
    lateral.abs() >= physics::WILD_LIMIT_M - 1e-6 && heading * lateral >= 0.0
}

/// The off-road track of a rider leaving wild mode's band: at their
/// position on `track`, along their direction of travel.
pub fn from_band(ground: Arc<GroundMap>, track: &dyn Track, state: &RiderState) -> OffRoadTrack {
    let p = track.sample(state.distance);
    let at = p.offset_position(state.lateral_offset);
    OffRoadTrack::new(
        ground,
        [at[0], at[2]],
        turned(p.direction, state.heading),
        state.distance,
    )
}

/// The other riders' places off the road between snapshots (like
/// [`crate::SnapshotBuffer`] for their states, with the same times).
#[derive(Debug, Clone, Default)]
pub struct PoseBuffer {
    snapshots: std::collections::VecDeque<(f64, std::collections::BTreeMap<RiderId, OffRoadPose>)>,
}

impl PoseBuffer {
    pub fn push(&mut self, time_s: f64, poses: impl IntoIterator<Item = (RiderId, OffRoadPose)>) {
        self.snapshots
            .push_back((time_s, poses.into_iter().collect()));
        while self.snapshots.len() > 32 {
            self.snapshots.pop_front();
        }
    }

    /// The places at `render_time_s`: interpolated where a rider is off
    /// the road in both surrounding snapshots, else the newer one's.
    pub fn sample(&self, render_time_s: f64) -> std::collections::BTreeMap<RiderId, OffRoadPose> {
        let Some((_, newest)) = self.snapshots.back() else {
            return Default::default();
        };
        let later = self.snapshots.iter().position(|(t, _)| *t >= render_time_s);
        let (a, b) = match later {
            Some(0) | None => return newest.clone(),
            Some(i) => (&self.snapshots[i - 1], &self.snapshots[i]),
        };
        let span = b.0 - a.0;
        let t = if span > 0.0 {
            ((render_time_s - a.0) / span).clamp(0.0, 1.0)
        } else {
            1.0
        };
        b.1.iter()
            .map(|(id, pb)| {
                let p = a.1.get(id).map_or(*pb, |pa| OffRoadPose::lerp(*pa, *pb, t));
                (*id, p)
            })
            .collect()
    }
}

#[doc(hidden)]
pub mod test_world;

#[cfg(test)]
mod tests;
