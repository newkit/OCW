//! Camera that follows the local rider.
//!
//! In chase mode the camera can be orbited around the rider: drag with the
//! left mouse button to turn and tilt, mouse wheel to zoom, `C` to reset (`R`
//! restarts the whole app).
//!
//! # What moves the camera (spec §3.2, "Camera and avatar")
//!
//! Every world object is in one of two classes, [`CameraInteraction`]:
//!
//! - **Solid** – terrain, the road and (later) buildings. The camera avoids
//!   them: it stays above them and lifts until the line of sight to the
//!   rider clears them. The terrain comes in as [`crate::Ground`]; other
//!   solid objects register the height of their top per area in
//!   [`CameraObstacles`].
//! - **See-through** – trees (later bushes, signs, other riders). They
//!   never change the camera position. Where one would block the view (in
//!   front of the lens, or between the camera and the rider) it fades out
//!   instead ([`crate::see_through`]).
//!
//! # Tilting with the road (spec §3.2, "Gradient feel")
//!
//! A level chase camera makes climbs look flat. The chase orbit is
//! therefore tilted with the road ([`CameraTilt`]): around the rider's
//! right axis by a fraction of the slope angle (smoothed over about a
//! second, capped), so uphill the eye drops behind the rider and looks up
//! the road, downhill it rises and looks down. The tilt is applied to the
//! orbit before the solid-object code ([`place_eye`]) sees the eye, so the
//! camera still never ends up in a slope or behind a building; the mouse
//! orbit and `C` (reset) stay relative to the tilted frame. Side and top
//! views are not tilted.
//!
//! # Tunnels (spec §3.2)
//!
//! Where the road runs under the terrain ([`crate::tunnel`]) the solid
//! terrain above would push the camera over the hill and hide the rider.
//! There the camera (in every mode) blends into a tunnel camera close
//! behind the rider inside the tube, below the terrain; terrain and
//! buildings above do not move it, and the mouse orbit is ignored.

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;

use std::collections::HashMap;
use std::sync::Arc;

use crate::menu::InputContext;
use crate::riders::avatar_transform;
use crate::{RideView, world};

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CameraMode>()
            .init_resource::<CameraOrbit>()
            .init_resource::<CameraObstacles>()
            .init_resource::<CameraFocus>()
            .init_resource::<CameraTilt>()
            .add_systems(Startup, spawn_camera)
            .add_systems(Update, orbit_input)
            .add_systems(PostUpdate, follow.before(TransformSystems::Propagate));
    }
}

/// How the camera treats a world object (see the module docs).
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraInteraction {
    /// The camera avoids it: stays above it and keeps the line of sight to
    /// the rider clear of it (terrain, road, buildings). Its shape reaches
    /// the camera as [`crate::Ground`] or a [`CameraObstacles`] layer.
    Solid,
    /// Never moves the camera; faded out where it would block the view
    /// (trees, bushes, signs, other riders).
    SeeThrough,
}

impl CameraInteraction {
    /// Whether objects of this class can change the camera position.
    pub fn moves_camera(self) -> bool {
        self == Self::Solid
    }
}

/// Height of the top of a solid object at world (x, z), `None` where it
/// has none (keep it cheap: reject by bounding box first).
pub type SolidTop = Arc<dyn Fn(f64, f64) -> Option<f64> + Send + Sync>;

/// One set of solid obstacles, e.g. the buildings of one tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ObstacleLayer {
    /// What they are ("buildings", …).
    pub kind: &'static str,
    /// The world package of the tile (index in the world set: 0 = the
    /// home package, neighbours from 1, spec §2.2.2).
    pub package: u32,
    /// The tile they belong to (route mode: (0, 0)).
    pub tile: (i32, i32),
}

/// Solid obstacles besides the terrain ([`CameraInteraction::Solid`]): the
/// camera stays above them and sees the rider over them like over terrain.
/// Streaming code inserts a layer per tile when it loads the tile and
/// removes it with the tile. See-through objects are never registered.
#[derive(Resource, Default, Clone)]
pub struct CameraObstacles {
    layers: HashMap<ObstacleLayer, SolidTop>,
}

impl CameraObstacles {
    /// Adds (or replaces) a layer.
    pub fn insert(
        &mut self,
        layer: ObstacleLayer,
        top: impl Fn(f64, f64) -> Option<f64> + Send + Sync + 'static,
    ) {
        self.layers.insert(layer, Arc::new(top));
    }

    /// Removes all layers (a ride switch).
    pub fn clear(&mut self) {
        self.layers.clear();
    }

    pub fn remove(&mut self, layer: ObstacleLayer) {
        self.layers.remove(&layer);
    }

    /// Removes every layer of a tile of a package (when the tile is
    /// unloaded).
    pub fn remove_tile(&mut self, package: u32, tile: (i32, i32)) {
        self.layers
            .retain(|l, _| (l.package, l.tile) != (package, tile));
    }

    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }

    /// Highest solid top at (x, z) over all layers.
    pub fn top_at(&self, x: f64, z: f64) -> Option<f64> {
        self.layers
            .values()
            .filter_map(|top| top(x, z))
            .reduce(f64::max)
    }
}

/// Distance kept between the camera and the walls of a solid prism (m):
/// points this close to a footprint count as inside it.
pub const WALL_CLEARANCE_M: f32 = 0.5;

/// Solid prisms – a footprint polygon from the ground up to a flat top –
/// with a grid index, as a [`CameraObstacles`] layer (e.g. the buildings
/// of one tile; a gable roof counts with its ridge height, courtyards as
/// solid). Built off the main thread with the tile's meshes.
#[derive(Debug, Clone, Default)]
pub struct SolidFootprints {
    prisms: Vec<Prism>,
    origin: [f32; 2],
    cell: f32,
    cols: usize,
    rows: usize,
    grid: Vec<Vec<u32>>,
}

#[derive(Debug, Clone)]
struct Prism {
    outline: Vec<[f32; 2]>,
    top: f32,
    min: [f32; 2],
    max: [f32; 2],
}

impl Prism {
    /// Inside the footprint or within [`WALL_CLEARANCE_M`] of it.
    fn covers(&self, x: f32, z: f32) -> bool {
        let m = WALL_CLEARANCE_M;
        if x < self.min[0] - m || x > self.max[0] + m || z < self.min[1] - m || z > self.max[1] + m
        {
            return false;
        }
        let ring = &self.outline;
        let mut inside = false;
        let mut near = false;
        for i in 0..ring.len() {
            let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
            if (a[1] > z) != (b[1] > z) && x < a[0] + (z - a[1]) / (b[1] - a[1]) * (b[0] - a[0]) {
                inside = !inside;
            }
            let (ab, ap) = (
                Vec2::new(b[0] - a[0], b[1] - a[1]),
                Vec2::new(x - a[0], z - a[1]),
            );
            let t = (ap.dot(ab) / ab.length_squared().max(1e-9)).clamp(0.0, 1.0);
            near |= (ap - ab * t).length_squared() < m * m;
        }
        inside || near
    }
}

/// Grid cell of the [`SolidFootprints`] index (m).
const FOOTPRINT_CELL_M: f32 = 16.0;

impl SolidFootprints {
    /// From (outline in world (x, z), top height) pairs; outlines with
    /// fewer than three points are skipped.
    pub fn new(prisms: impl IntoIterator<Item = (Vec<[f32; 2]>, f32)>) -> Self {
        let prisms: Vec<Prism> = prisms
            .into_iter()
            .filter(|(o, _)| o.len() >= 3)
            .map(|(outline, top)| {
                let (min, max) = outline.iter().fold(
                    ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]),
                    |(lo, hi), p| {
                        (
                            [lo[0].min(p[0]), lo[1].min(p[1])],
                            [hi[0].max(p[0]), hi[1].max(p[1])],
                        )
                    },
                );
                Prism {
                    outline,
                    top,
                    min,
                    max,
                }
            })
            .collect();
        if prisms.is_empty() {
            return Self::default();
        }
        let m = WALL_CLEARANCE_M;
        let (lo, hi) = prisms.iter().fold(
            ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]),
            |(lo, hi), p| {
                (
                    [lo[0].min(p.min[0] - m), lo[1].min(p.min[1] - m)],
                    [hi[0].max(p.max[0] + m), hi[1].max(p.max[1] + m)],
                )
            },
        );
        // At most 256 cells a side (a tile is 1 km: 63 cells of 16 m).
        let cell = FOOTPRINT_CELL_M.max((hi[0] - lo[0]).max(hi[1] - lo[1]) / 256.0);
        let cols = (((hi[0] - lo[0]) / cell).ceil() as usize).max(1);
        let rows = (((hi[1] - lo[1]) / cell).ceil() as usize).max(1);
        let mut grid = vec![Vec::new(); cols * rows];
        let at = |v: f32, o: f32, n: usize| (((v - o) / cell) as usize).min(n - 1);
        for (i, p) in prisms.iter().enumerate() {
            for r in at(p.min[1] - m, lo[1], rows)..=at(p.max[1] + m, lo[1], rows) {
                for c in at(p.min[0] - m, lo[0], cols)..=at(p.max[0] + m, lo[0], cols) {
                    grid[r * cols + c].push(i as u32);
                }
            }
        }
        Self {
            prisms,
            origin: lo,
            cell,
            cols,
            rows,
            grid,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.prisms.is_empty()
    }

    pub fn len(&self) -> usize {
        self.prisms.len()
    }

    /// Highest top of the prisms covering (x, z) (see [`Prism::covers`]).
    pub fn top_at(&self, x: f64, z: f64) -> Option<f64> {
        if self.prisms.is_empty() {
            return None;
        }
        let (x, z) = (x as f32, z as f32);
        let (c, r) = (
            ((x - self.origin[0]) / self.cell).floor(),
            ((z - self.origin[1]) / self.cell).floor(),
        );
        if c < 0.0 || r < 0.0 || c as usize >= self.cols || r as usize >= self.rows {
            return None;
        }
        self.grid[r as usize * self.cols + c as usize]
            .iter()
            .map(|&i| &self.prisms[i as usize])
            .filter(|p| p.covers(x, z))
            .map(|p| f64::from(p.top))
            .reduce(f64::max)
    }
}

/// Height of everything solid at (x, z): the terrain and the obstacles.
pub fn solid_height(
    ground: Option<&crate::Ground>,
    obstacles: &CameraObstacles,
    x: f64,
    z: f64,
) -> Option<f64> {
    let terrain = ground.and_then(|g| (g.0)(x, z));
    match (terrain, obstacles.top_at(x, z)) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    }
}

/// The point the camera keeps in view (the local rider's head), for fading
/// see-through objects between it and the camera. `None` without a rider.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq)]
pub struct CameraFocus(pub Option<Vec3>);

/// Camera perspective, switchable with the console command `camera.mode`.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CameraMode {
    /// Behind and above the rider.
    #[default]
    Chase,
    /// From the side of the road.
    Side,
    /// High above, looking down.
    Top,
}

impl std::str::FromStr for CameraMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "chase" => Ok(Self::Chase),
            "side" => Ok(Self::Side),
            "top" => Ok(Self::Top),
            other => Err(format!("unknown camera mode '{other}' (chase, side, top)")),
        }
    }
}

#[derive(Component)]
pub struct FollowCamera;

/// Orbit of the chase camera around the rider, relative to the riding
/// direction. The default reproduces the classic chase view.
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct CameraOrbit {
    /// Rotation around the rider (radians, 0 = behind, π = in front).
    pub yaw: f32,
    /// Elevation above the horizon (radians).
    pub pitch: f32,
    /// Distance from the rider in metres.
    pub distance: f32,
}

impl Default for CameraOrbit {
    fn default() -> Self {
        Self {
            yaw: 0.0,
            pitch: (2.6f32).atan2(6.5),
            distance: (6.5f32 * 6.5 + 2.6 * 2.6).sqrt(),
        }
    }
}

pub const MIN_PITCH: f32 = -0.05;
pub const MAX_PITCH: f32 = 1.45;
pub const MIN_DISTANCE: f32 = 2.5;
pub const MAX_DISTANCE: f32 = 80.0;
/// Radians per pixel of mouse movement.
const DRAG_SENSITIVITY: f32 = 0.005;

impl CameraOrbit {
    /// Applies a mouse drag (pixels) and wheel steps; keeps the limits.
    pub fn apply(&mut self, drag: Vec2, wheel_steps: f32) {
        self.yaw = (self.yaw - drag.x * DRAG_SENSITIVITY).rem_euclid(std::f32::consts::TAU);
        self.pitch = (self.pitch + drag.y * DRAG_SENSITIVITY).clamp(MIN_PITCH, MAX_PITCH);
        self.distance =
            (self.distance * (-wheel_steps * 0.12).exp()).clamp(MIN_DISTANCE, MAX_DISTANCE);
    }

    /// Eye and look-at target for a rider at `rider` riding along `forward`.
    pub fn eye_and_target(&self, rider: Vec3, forward: Vec3) -> (Vec3, Vec3) {
        let back = Quat::from_rotation_y(self.yaw) * -forward;
        let eye = rider + (back * self.pitch.cos() + Vec3::Y * self.pitch.sin()) * self.distance;
        // Behind the rider, look a bit ahead of them; when orbited to the
        // side or front, look at the rider.
        let ahead = self.yaw.cos().max(0.0) * 6.0;
        (eye, rider + Vec3::Y * 1.0 + forward * ahead)
    }
}

impl CameraOrbit {
    /// [`Self::eye_and_target`] in a frame tilted by `tilt` radians around
    /// the rider's right axis (positive = nose up, on climbs): the eye and
    /// the look-ahead point turn around the rider; the rider stays put.
    pub fn eye_and_target_tilted(&self, rider: Vec3, forward: Vec3, tilt: f32) -> (Vec3, Vec3) {
        let (eye, target) = self.eye_and_target(rider, forward);
        let right = Vec3::new(-forward.z, 0.0, forward.x).normalize_or_zero();
        if tilt == 0.0 || right == Vec3::ZERO {
            return (eye, target);
        }
        let q = Quat::from_axis_angle(right, tilt);
        let head = rider + Vec3::Y;
        (rider + q * (eye - rider), head + q * (target - head))
    }
}

/// How the chase camera tilts with the road (see the module docs; console
/// command `camera.tilt`).
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct CameraTilt {
    pub enabled: bool,
    /// Fraction of the road's slope angle the camera follows (0 … 1).
    pub fraction: f32,
    /// Largest tilt either way (radians).
    pub max: f32,
    /// Time constant of the smoothing (s).
    pub smoothing_s: f32,
}

impl Default for CameraTilt {
    /// 70 % of the slope angle, at most 8°, smoothed over 1.2 s (tuned by
    /// eye: a 10 % climb tilts the view by 4°).
    fn default() -> Self {
        Self {
            enabled: true,
            fraction: 0.7,
            max: 8f32.to_radians(),
            smoothing_s: 1.2,
        }
    }
}

/// The road gradient the tilt follows is averaged from this far behind
/// the rider (m) …
pub const TILT_BEHIND_M: f64 = 5.0;
/// … to this far ahead (m): about the camera's view of the road near the
/// rider, so single bumps do not rock the view.
pub const TILT_AHEAD_M: f64 = 20.0;

impl CameraTilt {
    /// The tilt (radians) the camera heads for on road gradient `grade`
    /// (rise over run).
    pub fn target(&self, grade: f64) -> f32 {
        if !self.enabled {
            return 0.0;
        }
        ((grade.atan() as f32) * self.fraction.clamp(0.0, 1.0)).clamp(-self.max, self.max)
    }

    /// One smoothing step from `current` towards `target` over `dt` seconds.
    pub fn smooth(&self, current: f32, target: f32, dt: f32) -> f32 {
        if self.smoothing_s <= 0.0 {
            return target;
        }
        current + (target - current) * (1.0 - (-dt.max(0.0) / self.smoothing_s).exp())
    }
}

/// Smoothed tilt of the follow camera and where the rider was (a jump
/// snaps it).
#[derive(Debug, Default)]
pub(crate) struct TiltState {
    tilt: f32,
    rider: Option<Vec3>,
    /// 0 = chase (or side, top) camera … 1 = tunnel camera ([`crate::tunnel`]).
    tunnel: f32,
    /// The smoothed eye of the chase camera while the tunnel camera blends in.
    outside_eye: Option<Vec3>,
    /// The smoothed eye of the tunnel camera.
    tunnel_eye: Option<Vec3>,
}

/// Mouse orbit (left button drag), zoom (wheel) and reset (`C`), only in
/// the ride's input context (not while the menu or the console is open).
fn orbit_input(
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    context: Option<Res<InputContext>>,
    view: Option<Res<RideView>>,
    mut orbit: ResMut<CameraOrbit>,
) {
    if !crate::menu::ride_input(context) {
        return;
    }
    // In god mode the mouse looks, the wheel sets the speed and C is the
    // first-person view (the app's god module).
    if view.is_some_and(|v| v.flight.is_some()) {
        return;
    }
    if keys.just_pressed(KeyCode::KeyC) {
        *orbit = CameraOrbit::default();
        return;
    }
    let drag = if buttons.pressed(MouseButton::Left) {
        motion.delta
    } else {
        Vec2::ZERO
    };
    let steps = match scroll.unit {
        MouseScrollUnit::Line => scroll.delta.y,
        MouseScrollUnit::Pixel => scroll.delta.y / 40.0,
    };
    if drag != Vec2::ZERO || steps != 0.0 {
        orbit.apply(drag, steps);
    }
}

fn spawn_camera(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 10.0, 20.0).looking_at(Vec3::ZERO, Vec3::Y),
        world::fog(),
        // Soft shadow edges (9-tap filter); less flicker than hardware 2×2.
        bevy::light::ShadowFilteringMethod::Gaussian,
        FollowCamera,
    ));
}

/// Lowest camera height above the ground (no looking through road or terrain).
pub const MIN_CAMERA_CLEARANCE_M: f32 = 0.6;

/// Lifts `eye` so it stays at least `MIN_CAMERA_CLEARANCE_M` above `ground`.
pub fn above_ground(eye: Vec3, ground: Option<f64>) -> Vec3 {
    match ground {
        Some(h) => eye.with_y(eye.y.max(h as f32 + MIN_CAMERA_CLEARANCE_M)),
        None => eye,
    }
}

/// Clearance kept between the line of sight and the terrain.
pub const SIGHT_CLEARANCE_M: f32 = 0.5;
/// Highest lift of the eye to see over terrain (m).
const MAX_SIGHT_LIFT_M: f32 = 40.0;

/// Lifts `eye` until the line of sight to `target` clears `ground` (with
/// [`SIGHT_CLEARANCE_M`]): a hill or embankment between the camera and the
/// rider would otherwise cut through the view. Raising the eye by Δ raises
/// the line at fraction t of the way by Δ·(1 − t), so the lift needed is the
/// largest (ground + clearance − line) / (1 − t) along the line; the last
/// part next to the rider is skipped (the rider stands on that ground).
pub fn clear_line_of_sight(
    eye: Vec3,
    target: Vec3,
    ground: &dyn Fn(f64, f64) -> Option<f64>,
) -> Vec3 {
    const SAMPLES: usize = 24;
    let mut lift = 0.0f32;
    for i in 1..SAMPLES {
        let t = i as f32 / SAMPLES as f32;
        if t > 0.85 {
            break;
        }
        let p = eye.lerp(target, t);
        if let Some(h) = ground(f64::from(p.x), f64::from(p.z)) {
            let need = (h as f32 + SIGHT_CLEARANCE_M - p.y) / (1.0 - t);
            lift = lift.max(need);
        }
    }
    eye + Vec3::Y * lift.clamp(0.0, MAX_SIGHT_LIFT_M)
}

/// Keeps `eye` clear of solid objects (`solid` = their height at (x, z)):
/// above them, and lifted until the line of sight to `rider_head` clears
/// them. See-through objects are not part of `solid`.
pub fn place_eye(eye: Vec3, rider_head: Vec3, solid: &dyn Fn(f64, f64) -> Option<f64>) -> Vec3 {
    let eye = above_ground(eye, solid(f64::from(eye.x), f64::from(eye.z)));
    clear_line_of_sight(eye, rider_head, solid)
}

/// Bevy's default vertical field of view (rad).
pub const DEFAULT_FOV: f32 = std::f32::consts::FRAC_PI_4;
/// The flight camera's distance behind the unicorn (m) and height above
/// its line of view.
pub const FLIGHT_BACK_M: f32 = 8.0;
pub const FLIGHT_UP_M: f32 = 2.0;
/// The flight camera keeps at least this much above the ground (m).
pub const FLIGHT_CLEARANCE_M: f32 = 1.0;

/// The field of view in flight: widened by up to 10° at 400 m/s.
pub fn flight_fov(speed_mps: f64) -> f32 {
    DEFAULT_FOV + 10f32.to_radians() * (speed_mps as f32 / 400.0).clamp(0.0, 1.0)
}

/// The direction the flight view looks along (yaw counter-clockwise from
/// −z, pitch up).
pub fn view_direction(yaw: f64, pitch: f64) -> Vec3 {
    let (y, p) = (yaw as f32, pitch as f32);
    Vec3::new(-y.sin() * p.cos(), p.sin(), -y.cos() * p.cos())
}

/// Eye and look-at point of the flight camera: behind and a little above
/// the unicorn, looking along the view; with C from the unicorn's head
/// (the horn in view). Never under the ground (`solid`).
pub fn flight_eye(f: &crate::FlightView, solid: &dyn Fn(f64, f64) -> Option<f64>) -> (Vec3, Vec3) {
    let body = crate::to_vec3(f.pos);
    let dir = view_direction(f.yaw, f.pitch);
    let eye = if f.first_person {
        body + Quat::from_rotation_y(f.yaw as f32) * Vec3::new(0.0, 1.05, -1.5)
    } else {
        body - dir * FLIGHT_BACK_M + Vec3::Y * FLIGHT_UP_M
    };
    let eye = match solid(f64::from(eye.x), f64::from(eye.z)) {
        Some(h) => eye.with_y(eye.y.max(h as f32 + FLIGHT_CLEARANCE_M)),
        None => eye,
    };
    (eye, eye + dir * 50.0)
}

/// The flight camera orbited by the console's `camera.orbit` (scenes and
/// videos: the unicorn from the side, its wings beating): around the
/// unicorn at the orbit's yaw from behind its view, pitch and distance,
/// looking at it, never under the ground. `None` while the orbit is the
/// default (the normal flight camera) or in the first-person view.
pub fn flight_orbit_eye(
    f: &crate::FlightView,
    orbit: &CameraOrbit,
    solid: &dyn Fn(f64, f64) -> Option<f64>,
) -> Option<(Vec3, Vec3)> {
    if f.first_person || *orbit == CameraOrbit::default() {
        return None;
    }
    let body = crate::to_vec3(f.pos);
    let forward = view_direction(f.yaw, 0.0);
    let back = Quat::from_rotation_y(orbit.yaw) * -forward;
    let eye = body + (back * orbit.pitch.cos() + Vec3::Y * orbit.pitch.sin()) * orbit.distance;
    let eye = match solid(f64::from(eye.x), f64::from(eye.z)) {
        Some(h) => eye.with_y(eye.y.max(h as f32 + FLIGHT_CLEARANCE_M)),
        None => eye,
    };
    Some((eye, body + Vec3::Y * 0.3))
}

/// The flight camera's up: the world's up rolled about the view by the
/// flight's roll (a right bank rolls the camera to the right).
pub fn flight_up(f: &crate::FlightView) -> Vec3 {
    let dir = view_direction(f.yaw, f.pitch).normalize();
    Quat::from_axis_angle(dir, f.roll as f32) * Vec3::Y
}

#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
pub(crate) fn follow(
    view: Res<RideView>,
    mode: Res<CameraMode>,
    orbit: Res<CameraOrbit>,
    ground: Option<Res<crate::Ground>>,
    obstacles: Res<CameraObstacles>,
    tilt_cfg: Res<CameraTilt>,
    tunnels: Option<Res<crate::tunnel::Tunnels>>,
    time: Res<Time>,
    mut camera: Query<&mut Transform, With<FollowCamera>>,
    mut projections: Query<&mut Projection, With<FollowCamera>>,
    mut focus: ResMut<CameraFocus>,
    mut smoothed_target: Local<Option<Vec3>>,
    mut tilt: Local<TiltState>,
) {
    // God mode (spec §14 Phase 13D): the flight camera.
    let fov = view
        .flight
        .as_ref()
        .map_or(DEFAULT_FOV, |f| flight_fov(f.speed_mps));
    for mut p in &mut projections {
        if let Projection::Perspective(pp) = &mut *p
            && (pp.fov - fov).abs() > 1e-4
        {
            pp.fov = fov;
        }
    }
    if let Some(f) = &view.flight {
        let Ok(mut cam) = camera.single_mut() else {
            return;
        };
        let solid = |x: f64, z: f64| solid_height(ground.as_deref(), &obstacles, x, z);
        if let Some((eye, target)) = flight_orbit_eye(f, &orbit, &solid) {
            cam.translation = eye;
            cam.look_at(target, Vec3::Y);
        } else {
            let (eye, target) = flight_eye(f, &solid);
            cam.translation = eye;
            cam.look_at(target, flight_up(f));
        }
        focus.set_if_neq(CameraFocus(Some(crate::to_vec3(f.pos))));
        // Back on the road the chase camera starts afresh.
        tilt.rider = None;
        *smoothed_target = None;
        return;
    }
    let Some(local) = view.local() else {
        focus.set_if_neq(CameraFocus(None));
        return;
    };
    let Ok(mut cam) = camera.single_mut() else {
        return;
    };
    // Follow the avatar itself: on the terrain off the road (wild mode) and
    // along the rider's own heading, not the road's.
    let track = view.track(local.id);
    let rider = avatar_transform(
        track,
        &local.state,
        ground.as_deref(),
        &ocw_assets::shipped::bike_rig(local.bike),
    );
    let p = track.sample(local.state.distance);
    let road = Vec3::new(p.direction[0] as f32, 0.0, p.direction[1] as f32);
    // Positive heading = to the right; Bevy turns left for a positive Y rotation.
    let forward = Quat::from_rotation_y(-local.state.heading as f32) * road;
    let right = Vec3::new(-forward.z, 0.0, forward.x);
    // Tilt with the road (chase view only); snap after a jump (teleport).
    let d = local.state.distance;
    let grade = ocw_world::average_grade(track, d - TILT_BEHIND_M, TILT_BEHIND_M + TILT_AHEAD_M);
    let goal = tilt_cfg.target(grade);
    let jumped = tilt
        .rider
        .is_none_or(|prev| prev.distance(rider.translation) >= 50.0);
    tilt.tilt = if jumped {
        goal
    } else {
        tilt_cfg.smooth(tilt.tilt, goal, time.delta_secs())
    };
    tilt.rider = Some(rider.translation);
    // In a tunnel the camera follows close behind the rider inside the
    // tube; blended in before the portal and out after the exit.
    let in_tunnel = tunnels
        .as_deref()
        .is_some_and(|t| crate::tunnel::tunnel_camera_wanted(&t.stretches, local.state.distance));
    let was_in_tunnel = tilt.tunnel > 0.0;
    tilt.tunnel = if jumped {
        if in_tunnel { 1.0 } else { 0.0 }
    } else {
        crate::tunnel::smooth_blend(tilt.tunnel, in_tunnel, time.delta_secs())
    };
    let (eye, target) = match *mode {
        CameraMode::Chase => orbit.eye_and_target_tilted(rider.translation, forward, tilt.tilt),
        CameraMode::Side => (
            rider.translation + right * 7.0 + Vec3::Y * 1.5,
            rider.translation + Vec3::Y * 0.9,
        ),
        CameraMode::Top => (
            rider.translation - forward * 30.0 + Vec3::Y * 60.0,
            rider.translation,
        ),
    };
    // Smooth the eye position; snap if far away (teleport, mode switch at start).
    // While the tunnel camera is (partly) on, the chase eye is kept apart.
    let k = 1.0 - (-8.0 * time.delta_secs()).exp();
    let prev = match tilt.outside_eye {
        Some(e) if was_in_tunnel => e,
        _ => cam.translation,
    };
    cam.translation = if prev.distance(eye) > 50.0 {
        eye
    } else {
        prev.lerp(eye, k)
    };
    let rider_head = rider.translation + Vec3::Y * 1.2;
    focus.set_if_neq(CameraFocus(Some(rider_head)));
    // Only solid objects move the camera: never below them (also while
    // smoothing towards a low eye), and none between the eye and the rider
    // (the rider, not the look-ahead point: uphill that is in the slope).
    // Trees are see-through: they fade out instead (see_through.rs).
    let solid = |x: f64, z: f64| solid_height(ground.as_deref(), &obstacles, x, z);
    cam.translation = place_eye(cam.translation, rider_head, &solid);
    let mut target = target;
    if let Some(g) = ground.as_deref() {
        // The look-ahead point stays above the ground too, but not so far
        // above the rider that the rider leaves the picture.
        if let Some(h) = (g.0)(f64::from(target.x), f64::from(target.z)) {
            let rider_y = rider.translation.y;
            target.y = target.y.max(h as f32 + 1.0).min(rider_y + 3.0);
        }
    }
    // Smooth the look-at point like the eye (bumpy ground ahead would make
    // the view jerk); snap when far (teleport).
    let target = match *smoothed_target {
        Some(t) if t.distance(target) < 50.0 => t.lerp(target, k),
        _ => target,
    };
    *smoothed_target = Some(target);
    let target = if tilt.tunnel > 0.0 {
        tilt.outside_eye = Some(cam.translation);
        let (eye, inside) =
            crate::tunnel::tunnel_eye(track, local.state.distance, local.state.lateral_offset);
        // A very quick glide (smooths the eye's steps along the road).
        let eye = match tilt.tunnel_eye {
            Some(prev) if !jumped && prev.distance(eye) < 20.0 => {
                prev.lerp(eye, 1.0 - (-time.delta_secs() / 0.05).exp())
            }
            _ => eye,
        };
        tilt.tunnel_eye = Some(eye);
        // Smoothstep: no jerk where the blend starts and ends.
        let u = tilt.tunnel * tilt.tunnel * (3.0 - 2.0 * tilt.tunnel);
        cam.translation = cam.translation.lerp(eye, u);
        target.lerp(inside, u)
    } else {
        tilt.outside_eye = None;
        tilt.tunnel_eye = None;
        target
    };
    cam.look_at(target, Vec3::Y);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-4
    }

    #[test]
    fn camera_stays_above_the_ground() {
        let eye = Vec3::new(1.0, 100.2, 2.0);
        assert_eq!(
            above_ground(eye, Some(100.0)).y,
            100.0 + MIN_CAMERA_CLEARANCE_M
        );
        assert_eq!(above_ground(eye, Some(90.0)), eye);
        assert_eq!(above_ground(eye, None), eye);
    }

    #[test]
    fn camera_sees_over_a_hill_between_it_and_the_rider() {
        // Rider at x = 0 on flat ground (0 m); a 5 m ridge at x 3 … 5 m
        // between the rider and the eye at x = 8, 2 m high.
        let ridge = |x: f64, _z: f64| Some(if (3.0..=5.0).contains(&x) { 5.0 } else { 0.0 });
        let target = Vec3::new(0.0, 1.0, 0.0);
        let eye = clear_line_of_sight(Vec3::new(8.0, 2.0, 0.0), target, &ridge);
        assert!(eye.y > 2.0 && eye.x == 8.0 && eye.z == 0.0, "{eye}");
        // Now the whole line is above the ridge (+ clearance).
        for i in 1..20 {
            let p = eye.lerp(target, i as f32 / 20.0);
            if (3.0..=5.0).contains(&p.x) {
                assert!(p.y >= 5.0 + SIGHT_CLEARANCE_M - 0.3, "{p}");
            }
        }
        // Nothing in the way: unchanged.
        let flat = |_: f64, _: f64| Some(0.0);
        assert_eq!(
            clear_line_of_sight(Vec3::new(8.0, 2.0, 0.0), target, &flat),
            Vec3::new(8.0, 2.0, 0.0)
        );
        // Outside the terrain data: unchanged.
        let none = |_: f64, _: f64| None;
        assert_eq!(
            clear_line_of_sight(Vec3::new(8.0, 2.0, 0.0), target, &none),
            Vec3::new(8.0, 2.0, 0.0)
        );
    }

    #[test]
    fn only_solid_objects_move_the_camera() {
        assert!(CameraInteraction::Solid.moves_camera());
        assert!(!CameraInteraction::SeeThrough.moves_camera());
    }

    #[test]
    fn obstacle_layers_add_to_the_terrain_and_go_with_their_tile() {
        let mut obstacles = CameraObstacles::default();
        assert!(obstacles.is_empty());
        let ground = crate::Ground(Arc::new(|_, _| Some(100.0)));
        assert_eq!(
            solid_height(Some(&ground), &obstacles, 5.0, 5.0),
            Some(100.0)
        );
        // A 12 m house at x 0 … 10 (any z) on tile (0, 0).
        let house = ObstacleLayer {
            kind: "buildings",
            package: 0,
            tile: (0, 0),
        };
        obstacles.insert(house, |x, _| (0.0..=10.0).contains(&x).then_some(112.0));
        assert_eq!(
            solid_height(Some(&ground), &obstacles, 5.0, 5.0),
            Some(112.0)
        );
        assert_eq!(
            solid_height(Some(&ground), &obstacles, 20.0, 5.0),
            Some(100.0)
        );
        assert_eq!(solid_height(None, &obstacles, 5.0, 0.0), Some(112.0));
        assert_eq!(solid_height(None, &obstacles, 20.0, 0.0), None);
        // The highest layer wins.
        let tower = ObstacleLayer {
            kind: "towers",
            package: 0,
            tile: (0, 0),
        };
        obstacles.insert(tower, |x, _| (x < 6.0).then_some(130.0));
        assert_eq!(obstacles.top_at(5.0, 0.0), Some(130.0));
        obstacles.remove(tower);
        assert_eq!(obstacles.top_at(5.0, 0.0), Some(112.0));
        obstacles.remove_tile(0, (1, 0));
        assert!(!obstacles.is_empty());
        // The same tile key of a neighbouring package is another tile.
        obstacles.remove_tile(1, (0, 0));
        assert!(!obstacles.is_empty());
        obstacles.remove_tile(0, (0, 0));
        assert!(obstacles.is_empty());
    }

    #[test]
    fn footprints_cover_their_outline_and_a_wall_clearance() {
        // An L-shaped 8 m house and a 20 m tower inside its bounding box gap.
        let l = vec![
            [0.0, 0.0],
            [10.0, 0.0],
            [10.0, 4.0],
            [4.0, 4.0],
            [4.0, 10.0],
            [0.0, 10.0],
        ];
        let tower = vec![[30.0, 30.0], [32.0, 30.0], [32.0, 32.0], [30.0, 32.0]];
        let f = SolidFootprints::new([(l, 8.0), (tower, 20.0), (vec![[0.0, 0.0]], 99.0)]);
        assert_eq!(f.len(), 2);
        assert_eq!(f.top_at(2.0, 2.0), Some(8.0));
        assert_eq!(f.top_at(8.0, 2.0), Some(8.0));
        // The notch of the L is free, except within the wall clearance.
        assert_eq!(f.top_at(8.0, 8.0), None);
        assert_eq!(f.top_at(4.3, 8.0), Some(8.0));
        assert_eq!(f.top_at(-0.4, 5.0), Some(8.0));
        assert_eq!(f.top_at(-0.6, 5.0), None);
        assert_eq!(f.top_at(31.0, 31.0), Some(20.0));
        assert_eq!(f.top_at(500.0, -500.0), None);
        assert!(SolidFootprints::new([]).top_at(0.0, 0.0).is_none());
    }

    #[test]
    fn a_solid_obstacle_lifts_the_eye_like_terrain() {
        let flat = |_: f64, _: f64| Some(0.0);
        let head = Vec3::new(0.0, 1.2, 0.0);
        let eye = Vec3::new(8.0, 2.0, 0.0);
        assert_eq!(place_eye(eye, head, &flat), eye);
        // A 6 m wall at x 3 … 5 between the eye and the rider.
        let mut obstacles = CameraObstacles::default();
        obstacles.insert(
            ObstacleLayer {
                kind: "buildings",
                package: 0,
                tile: (0, 0),
            },
            |x, _| (3.0..=5.0).contains(&x).then_some(6.0),
        );
        let ground = crate::Ground(Arc::new(|_, _| Some(0.0)));
        let solid = |x: f64, z: f64| solid_height(Some(&ground), &obstacles, x, z);
        let lifted = place_eye(eye, head, &solid);
        assert!(lifted.y > 6.0 && lifted.xz() == eye.xz(), "{lifted}");
        // An eye inside the wall goes on top of it.
        let on_roof = place_eye(Vec3::new(4.0, 3.0, 0.0), Vec3::new(4.0, 7.2, 0.0), &solid);
        assert!(on_roof.y >= 6.0 + MIN_CAMERA_CLEARANCE_M, "{on_roof}");
    }

    /// The real `follow` system with see-through trees at the eye and in
    /// the line of sight: the camera stays exactly where the orbit puts it;
    /// a solid block in the same place lifts it.
    #[test]
    fn trees_never_move_the_camera_but_solid_obstacles_do() {
        use ocw_core::RiderId;
        use ocw_world::Route;

        let route = Route::test_loop();
        let mut view = RideView::new(route.clone());
        view.riders.push(crate::RiderView {
            id: RiderId(1),
            name: "me".into(),
            state: ocw_sim::RiderState {
                distance: 100.0,
                ..default()
            },
            power_w: 0.0,
            cadence_rpm: None,
            heart_rate_bpm: None,
            is_local: true,
            bike: Default::default(),
            look: None,
            crash: None,
            wobble: 0.0,
        });
        let road_y = route.sample(100.0).position[1];
        let ground = crate::Ground(Arc::new(move |_, _| Some(road_y - 0.1)));
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(view)
            .insert_resource(ground)
            .init_resource::<CameraMode>()
            .init_resource::<CameraOrbit>()
            .init_resource::<CameraObstacles>()
            .init_resource::<CameraFocus>()
            .init_resource::<CameraTilt>()
            .add_systems(Update, follow);
        let cam = app
            .world_mut()
            .spawn((Transform::from_xyz(1e4, 0.0, 1e4), FollowCamera))
            .id();
        app.update();
        let free = app.world().get::<Transform>(cam).unwrap().translation;
        let focus = app.world().resource::<CameraFocus>().0.expect("rider");
        let mid = free.lerp(focus, 0.5);
        // Trees at the eye and half way to the rider.
        for p in [free, mid] {
            app.world_mut().spawn((
                Transform::from_translation(p.with_y(road_y as f32)),
                CameraInteraction::SeeThrough,
            ));
        }
        app.update();
        let with_trees = app.world().get::<Transform>(cam).unwrap().translation;
        assert!((with_trees - free).length() < 1e-3, "{free} → {with_trees}");
        // A solid 8 m block around the half-way point lifts the eye.
        app.world_mut().resource_mut::<CameraObstacles>().insert(
            ObstacleLayer {
                kind: "buildings",
                package: 0,
                tile: (0, 0),
            },
            move |x, z| {
                ((x - f64::from(mid.x)).abs() < 1.5 && (z - f64::from(mid.z)).abs() < 1.5)
                    .then_some(road_y + 8.0)
            },
        );
        app.update();
        let lifted = app.world().get::<Transform>(cam).unwrap().translation;
        assert!(lifted.y > free.y + 2.0, "{free} → {lifted}");
    }

    #[test]
    fn default_orbit_is_the_classic_chase_view() {
        let (eye, target) = CameraOrbit::default().eye_and_target(Vec3::ZERO, Vec3::NEG_Z);
        assert!(close(eye, Vec3::new(0.0, 2.6, 6.5)), "{eye}");
        assert!(close(target, Vec3::new(0.0, 1.0, -6.0)), "{target}");
    }

    #[test]
    fn tilt_follows_a_fraction_of_the_slope_and_is_capped() {
        let t = CameraTilt::default();
        // 10 %: 5.7° slope, 70 % of it.
        assert!((t.target(0.10).to_degrees() - 0.7 * 5.711).abs() < 0.01);
        assert!((t.target(-0.10) + t.target(0.10)).abs() < 1e-6);
        assert_eq!(t.target(0.0), 0.0);
        // A 30 % wall: capped at 8°.
        assert!((t.target(0.30) - 8f32.to_radians()).abs() < 1e-6);
        assert!((t.target(-0.30) + 8f32.to_radians()).abs() < 1e-6);
        let off = CameraTilt {
            enabled: false,
            ..t
        };
        assert_eq!(off.target(0.2), 0.0);
        let full = CameraTilt { fraction: 1.0, ..t };
        assert!((full.target(0.05) - 0.05f32.atan()).abs() < 1e-6);
    }

    #[test]
    fn tilt_is_smoothed_over_about_a_second() {
        let t = CameraTilt::default();
        let goal = 0.1;
        // After one time constant ~63 %, after five ~99 %; independent of
        // the frame rate.
        let mut a = 0.0;
        for _ in 0..12 {
            a = t.smooth(a, goal, 0.1);
        }
        let b = t.smooth(0.0, goal, 1.2);
        assert!(
            (a - b).abs() < 1e-5 && (b / goal - 0.632).abs() < 0.01,
            "{a} {b}"
        );
        assert!((t.smooth(0.0, goal, 6.0) / goal) > 0.99);
        assert_eq!(t.smooth(0.05, goal, 0.0), 0.05);
        let instant = CameraTilt {
            smoothing_s: 0.0,
            ..t
        };
        assert_eq!(instant.smooth(0.0, goal, 0.01), goal);
    }

    #[test]
    fn tilted_orbit_looks_up_the_climb() {
        let o = CameraOrbit::default();
        let rider = Vec3::new(3.0, 50.0, 7.0);
        let (eye0, target0) = o.eye_and_target(rider, Vec3::NEG_Z);
        assert_eq!(
            o.eye_and_target_tilted(rider, Vec3::NEG_Z, 0.0),
            (eye0, target0)
        );
        let a = 5f32.to_radians();
        let (eye, target) = o.eye_and_target_tilted(rider, Vec3::NEG_Z, a);
        // Same distance, the eye lower behind the rider, the look-ahead
        // point higher; nothing moves sideways.
        assert!((eye.distance(rider) - o.distance).abs() < 1e-4);
        assert!(eye.y < eye0.y && target.y > target0.y, "{eye} {target}");
        assert!((eye.x - rider.x).abs() < 1e-5 && (target.x - rider.x).abs() < 1e-5);
        // The elevation above the rider drops by exactly the tilt.
        let elev = ((eye.y - rider.y) / o.distance).asin();
        assert!((elev - (o.pitch - a)).abs() < 1e-4, "{elev}");
        // Downhill the other way.
        let (eye_down, _) = o.eye_and_target_tilted(rider, Vec3::NEG_Z, -a);
        assert!(eye_down.y > eye0.y);
        // Orbited to the side, the eye is on the tilt axis: it stays.
        let side = CameraOrbit {
            yaw: std::f32::consts::FRAC_PI_2,
            pitch: 0.0,
            ..o
        };
        let (e0, _) = side.eye_and_target(rider, Vec3::NEG_Z);
        let (e1, _) = side.eye_and_target_tilted(rider, Vec3::NEG_Z, a);
        assert!(close(e0, e1), "{e0} {e1}");
    }

    /// On a climb the real `follow` system tilts the chase camera (eye
    /// lower than with a level orbit, looking further up) and still keeps
    /// it above the slope.
    #[test]
    fn follow_tilts_on_a_climb() {
        use ocw_core::RiderId;
        use ocw_world::Route;

        // A straight 10 % ramp along +x.
        let pts: Vec<[f64; 3]> = (0..=100)
            .map(|i| {
                let x = f64::from(i) * 5.0;
                [x, x * 0.1, 0.0]
            })
            .collect();
        let route = Route::from_points("ramp", "Ramp", pts, false, 6.0).unwrap();
        let run = |tilt: CameraTilt| {
            let mut view = RideView::new(route.clone());
            view.riders.push(crate::RiderView {
                id: RiderId(1),
                name: "me".into(),
                state: ocw_sim::RiderState {
                    distance: 200.0,
                    ..default()
                },
                power_w: 0.0,
                cadence_rpm: None,
                heart_rate_bpm: None,
                is_local: true,
                bike: Default::default(),
                look: None,
                crash: None,
                wobble: 0.0,
            });
            let mut app = App::new();
            app.add_plugins(MinimalPlugins)
                .insert_resource(view)
                .insert_resource(crate::Ground(Arc::new(|x: f64, _: f64| Some(x * 0.1))))
                .insert_resource(tilt)
                .init_resource::<CameraMode>()
                .init_resource::<CameraOrbit>()
                .init_resource::<CameraObstacles>()
                .init_resource::<CameraFocus>()
                .add_systems(Update, follow);
            let cam = app
                .world_mut()
                .spawn((Transform::from_xyz(1e4, 0.0, 1e4), FollowCamera))
                .id();
            app.update();
            *app.world().get::<Transform>(cam).unwrap()
        };
        let level = run(CameraTilt {
            enabled: false,
            ..default()
        });
        let tilted = run(CameraTilt::default());
        assert!(
            tilted.translation.y < level.translation.y - 0.2,
            "{level:?} {tilted:?}"
        );
        // Above the slope behind the rider.
        let ground = tilted.translation.x * 0.1;
        assert!(tilted.translation.y >= ground + MIN_CAMERA_CLEARANCE_M - 1e-3);
        // Looking further up.
        assert!(tilted.forward().y > level.forward().y);
    }

    /// In a tunnel (the road under a hill) the real `follow` system puts
    /// the camera close behind the rider inside the tube, below the
    /// terrain, instead of lifting it over the hill; outside it is the
    /// chase camera above the ground again.
    #[test]
    fn follow_stays_in_the_tunnel_with_the_rider() {
        use crate::tunnel::{TUBE_WALL_M, Tunnels, covered_stretches};
        use ocw_core::RiderId;
        use ocw_world::Route;

        // A straight road at 100 m along +x, under a 40 m hill at x 800 … 1200.
        let pts: Vec<[f64; 3]> = (0..=400)
            .map(|i| [f64::from(i) * 5.0, 100.0, 0.0])
            .collect();
        let route = Route::from_points("tunnel", "Tunnel", pts, false, 6.0).unwrap();
        let hill = |x: f64, _: f64| {
            Some(if (800.0..=1200.0).contains(&x) {
                140.0
            } else {
                99.7
            })
        };
        let run = |distance: f64| {
            let mut view = RideView::new(route.clone());
            view.riders.push(crate::RiderView {
                id: RiderId(1),
                name: "me".into(),
                state: ocw_sim::RiderState {
                    distance,
                    ..default()
                },
                power_w: 0.0,
                cadence_rpm: None,
                heart_rate_bpm: None,
                is_local: true,
                bike: Default::default(),
                look: None,
                crash: None,
                wobble: 0.0,
            });
            let mut tunnels = Tunnels::default();
            tunnels.stretches = covered_stretches(&route, &hill, 0.0, 2000.0);
            let mut app = App::new();
            app.add_plugins(MinimalPlugins)
                .insert_resource(view)
                .insert_resource(crate::Ground(Arc::new(hill)))
                .insert_resource(tunnels)
                .init_resource::<CameraTilt>()
                .init_resource::<CameraMode>()
                .init_resource::<CameraOrbit>()
                .init_resource::<CameraObstacles>()
                .init_resource::<CameraFocus>()
                .add_systems(Update, follow);
            let cam = app
                .world_mut()
                .spawn((Transform::from_xyz(1e4, 0.0, 1e4), FollowCamera))
                .id();
            app.update();
            *app.world().get::<Transform>(cam).unwrap()
        };
        let inside = run(1000.0);
        let p = inside.translation;
        assert!(
            p.y > 100.0 + MIN_CAMERA_CLEARANCE_M && p.y < 100.0 + TUBE_WALL_M as f32,
            "{p}"
        );
        assert!(p.x < 1000.0 && p.x > 1000.0 - 6.0, "close behind: {p}");
        assert!(p.z.abs() < 3.0, "on the road: {p}");
        assert!(inside.forward().y.abs() < 0.2, "looking along the tube");
        // Just before the portal the tunnel camera is already on …
        assert!(run(790.0).translation.y < 100.0 + TUBE_WALL_M as f32);
        // … and well outside it is the chase camera again, above the ground.
        let outside = run(500.0).translation;
        let chase = CameraOrbit::default();
        assert!(outside.x < 500.0 - chase.distance * 0.5, "{outside}");
        assert!(outside.y >= 99.7 + MIN_CAMERA_CLEARANCE_M, "{outside}");
    }

    #[test]
    fn dragging_orbits_and_limits_apply() {
        let mut o = CameraOrbit::default();
        // Drag far to the left/right: half a turn puts the camera in front.
        o.apply(
            Vec2::new(-std::f32::consts::PI / DRAG_SENSITIVITY, 0.0),
            0.0,
        );
        let (eye, target) = o.eye_and_target(Vec3::ZERO, Vec3::NEG_Z);
        assert!(eye.z < -6.0, "{eye}");
        assert!(close(target, Vec3::Y), "looks at the rider: {target}");
        // Pitch and zoom are clamped.
        o.apply(Vec2::new(0.0, 1e6), 0.0);
        assert_eq!(o.pitch, MAX_PITCH);
        o.apply(Vec2::new(0.0, -1e6), 100.0);
        assert_eq!((o.pitch, o.distance), (MIN_PITCH, MIN_DISTANCE));
        o.apply(Vec2::ZERO, -100.0);
        assert_eq!(o.distance, MAX_DISTANCE);
    }

    #[test]
    fn c_resets_the_orbit() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<AccumulatedMouseMotion>()
            .init_resource::<AccumulatedMouseScroll>()
            .insert_resource(CameraOrbit {
                yaw: 2.0,
                pitch: 1.0,
                distance: 30.0,
            })
            .add_systems(Update, orbit_input);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyC);
        app.update();
        assert_eq!(
            *app.world().resource::<CameraOrbit>(),
            CameraOrbit::default()
        );
    }

    #[test]
    fn the_mouse_and_c_do_not_move_the_camera_while_the_menu_is_open() {
        let mut app = App::new();
        let orbit = CameraOrbit {
            yaw: 2.0,
            pitch: 1.0,
            distance: 30.0,
        };
        app.add_plugins(MinimalPlugins)
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<ButtonInput<KeyCode>>()
            .insert_resource(AccumulatedMouseMotion {
                delta: Vec2::new(40.0, 10.0),
            })
            .init_resource::<AccumulatedMouseScroll>()
            .insert_resource(InputContext::Menu)
            .insert_resource(orbit)
            .add_systems(Update, orbit_input);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyC);
        app.update();
        assert_eq!(*app.world().resource::<CameraOrbit>(), orbit);
        // Back in the ride: C resets.
        *app.world_mut().resource_mut::<InputContext>() = InputContext::Ride;
        app.update();
        assert_eq!(
            *app.world().resource::<CameraOrbit>(),
            CameraOrbit::default()
        );
    }

    /// God mode (spec §14 Phase 13D): behind and above the unicorn,
    /// looking along the view; C: from the head; never under the ground;
    /// a wider field of view at speed.
    #[test]
    fn the_flight_camera_follows_the_unicorn_and_stays_above_the_ground() {
        let f = crate::FlightView {
            pos: [100.0, 50.0, 200.0],
            yaw: 0.0,
            pitch: 0.0,
            ..Default::default()
        };
        let none = |_: f64, _: f64| None;
        let (eye, target) = flight_eye(&f, &none);
        assert!(close(eye, Vec3::new(100.0, 52.0, 208.0)), "{eye}");
        assert!((target - eye).normalize().dot(Vec3::NEG_Z) > 0.999);
        // First person: at the head, in front of the body.
        let fp = crate::FlightView {
            first_person: true,
            ..f.clone()
        };
        let (eye, _) = flight_eye(&fp, &none);
        assert!(eye.z < 200.0 && eye.y > 50.0);
        // Looking steeply up from low over a hill (the eye behind and
        // below): lifted above the hill.
        let low = crate::FlightView {
            pos: [0.0, 12.0, 0.0],
            pitch: 1.2,
            ..f.clone()
        };
        let hill = |_: f64, _: f64| Some(10.0);
        let (eye, _) = flight_eye(&low, &hill);
        assert!(eye.y >= 10.0 + FLIGHT_CLEARANCE_M - 1e-4, "{eye}");
        assert_eq!(flight_fov(0.0), DEFAULT_FOV);
        assert!((flight_fov(800.0) - DEFAULT_FOV - 10f32.to_radians()).abs() < 1e-6);
        // The direction: yaw turns left (counter-clockwise from above).
        assert!(close(
            view_direction(std::f64::consts::FRAC_PI_2, 0.0),
            Vec3::NEG_X
        ));
        // Level: the world's up; a right bank rolls the camera to the
        // right (looking north, its up leans east), a left one to the left.
        assert!(close(flight_up(&f), Vec3::Y));
        let banked = crate::FlightView {
            roll: 0.4,
            ..f.clone()
        };
        let up = flight_up(&banked);
        assert!(
            up.x > 0.3 && up.y > 0.9 && up.dot(Vec3::NEG_Z).abs() < 1e-5,
            "{up}"
        );
        let left = crate::FlightView {
            roll: -0.4,
            ..f.clone()
        };
        assert!(flight_up(&left).x < -0.3);
        let mut cam = Transform::from_translation(Vec3::ZERO);
        cam.look_at(Vec3::NEG_Z, up);
        assert!(
            cam.right().y < -0.3,
            "the horizon tilts: the right side dips"
        );
        // camera.orbit in flight (scenes): off by default; 90° looks at the
        // unicorn from its side, at the orbit's distance.
        assert_eq!(flight_orbit_eye(&f, &CameraOrbit::default(), &none), None);
        let side = CameraOrbit {
            yaw: 90f32.to_radians(),
            pitch: 0.1,
            distance: 7.0,
        };
        let (eye, target) = flight_orbit_eye(&f, &side, &none).unwrap();
        let body = crate::to_vec3(f.pos);
        assert!((eye.distance(body) - 7.0).abs() < 1e-3);
        assert!((eye.x - body.x).abs() > 6.5, "to the side: {eye}");
        assert!(target.distance(body) < 0.5);
        let fp = crate::FlightView {
            first_person: true,
            ..f.clone()
        };
        assert_eq!(flight_orbit_eye(&fp, &side, &none), None);
    }

    #[test]
    fn camera_mode_parses() {
        assert_eq!("chase".parse::<CameraMode>(), Ok(CameraMode::Chase));
        assert_eq!("side".parse::<CameraMode>(), Ok(CameraMode::Side));
        assert_eq!("top".parse::<CameraMode>(), Ok(CameraMode::Top));
        assert!("Top".parse::<CameraMode>().is_err());
        assert!("".parse::<CameraMode>().is_err());
        assert_eq!(CameraMode::default(), CameraMode::Chase);
    }
}
