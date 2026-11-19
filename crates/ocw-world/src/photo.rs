//! The virtual photographer's eye (spec §2.8.1, ST2): how nice a view is
//! and where a person with a camera would stand to take it. Pure and
//! engine-free (wasm-safe); the world comes in through [`PhotoScene`], so
//! the tests use fixed scenes and the app the loaded world package.
//!
//! - **The view score** ([`view_score`]): an *open view* from horizon rays
//!   over the ground in the camera's direction (how far the eye reaches,
//!   tree crowns raising the ground they stand on), the *relief* in view
//!   (mountains, a valley below), *water* in view (lakes, rivers, the
//!   sea), *landmarks* (a named peak or a viewpoint in the frame and in
//!   sight, or the camera at a viewpoint looking its way – from the
//!   package's section `LMRK`), times *the light* (golden hour best, night worst) and *the
//!   weather* (fog cuts the reach, rain dulls). A covered road (a tunnel)
//!   scores zero.
//! - **The camera poses** ([`candidates`], [`choose`]): a photographer is
//!   a person (the user's request of 2026-11-16: a real photographer
//!   appears in the game), so every pose stands on the ground beside the
//!   road – eye 1.6 m above the feet standing, 0.9 m crouching – never in
//!   water, inside a building, tree trunk or obstacle, on the carriageway
//!   or within 1.5 m of the riding line, 4–40 m from the rider, on ground
//!   a person can stand on. Kinds: ahead and low (the rider coming up),
//!   from the side (the valley behind the rider), from behind (the road
//!   ahead into the landscape) and wide (a person farther away on the
//!   hillside, the rider small). No aerial poses.
//! - **Occlusion** ([`occluded`]): the lines from the eye to the rider's
//!   head and body must clear the ground, buildings and tree crowns.
//! - **Framing** ([`aim`]): the rider on a third of the frame (the side
//!   with the more open view) and in the lower part, never cut by the
//!   edge; too small or too large scores lower.

/// Eye height of a standing photographer above the feet (m).
pub const EYE_STANDING_M: f64 = 1.6;
/// … and of a crouching one.
pub const EYE_CROUCHING_M: f64 = 0.9;
/// The photographer stands at least this far from the riding line (m).
pub const MIN_LINE_GAP_M: f64 = 1.5;
/// … at least this far beyond the road's edge.
pub const MIN_ROAD_GAP_M: f64 = 0.5;
/// Distance from the rider (m).
pub const MIN_DISTANCE_M: f64 = 4.0;
pub const MAX_DISTANCE_M: f64 = 40.0;
/// Steepest ground a person stands on (rise over run).
pub const MAX_STAND_SLOPE: f64 = 0.7;
/// The photo's vertical field of view (rad, ~40°) and aspect (2560 × 1440).
pub const PHOTO_FOV_Y: f64 = 0.70;
pub const PHOTO_ASPECT: f64 = 16.0 / 9.0;
/// The rider's height in the frame (m, rider on the bike).
pub const RIDER_HEIGHT_M: f64 = 1.75;
/// Horizon rays reach this far (m).
pub const MAX_REACH_M: f64 = 12_000.0;
/// The lowest score a photo is taken at (spec §2.8.1 Q3, decided: only
/// nice enough views; a flat meadow at noon stays under it, a view with
/// relief or water passes).
pub const MIN_SCORE: f64 = 0.35;
/// A road this far (m) below the ground above it is covered (a tunnel).
pub const COVERED_M: f64 = 2.5;

/// A peak in the frame counts this far (m).
pub const PEAK_REACH_M: f64 = 15_000.0;
/// A viewpoint in the frame counts this far (m).
pub const VIEWPOINT_REACH_M: f64 = 3_000.0;
/// A camera this close to a viewpoint (m) stands at it: the view counts
/// as a landmark when it looks the viewpoint's way (spec 2.11 Q4).
pub const VIEWPOINT_NEAR_M: f64 = 120.0;
/// The landmark part of a viewpoint seen in the frame (a peak's is 1).
pub const VIEWPOINT_IN_FRAME: f64 = 0.6;

pub use crate::landmarks::{LandmarkKind, ViewDirection};

/// A place worth having in the picture (a named peak, a viewpoint), from
/// the package's section `LMRK`.
#[derive(Debug, Clone, PartialEq)]
pub struct Landmark {
    pub name: String,
    /// Local metres (x, y up, z); y the elevation (OSM `ele` or the
    /// terrain's).
    pub pos: [f64; 3],
    pub kind: LandmarkKind,
    /// A viewpoint's direction of view.
    pub direction: Option<ViewDirection>,
}

impl Landmark {
    /// A named peak at `pos`.
    pub fn peak(name: &str, pos: [f64; 3]) -> Self {
        Self {
            name: name.into(),
            pos,
            kind: LandmarkKind::Peak,
            direction: None,
        }
    }

    /// The peaks and viewpoints of the loaded packages of `set` (section
    /// `LMRK`) in the home package's metres, each once; y their elevation
    /// (a landmark without one is left out). Passes are not landmarks of
    /// the view score and are left out too.
    pub fn of_set(set: &crate::WorldSet) -> Vec<Self> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for m in set.members() {
            let Some(places) = m.package().and_then(|p| p.places.as_ref()) else {
                continue;
            };
            for l in &places.landmarks {
                if l.kind == LandmarkKind::Pass || !seen.insert((l.element, l.osm_id)) {
                    continue;
                }
                let Some(ele) = l.ele else { continue };
                let p = m.frame.apply([f64::from(l.pos[0]), f64::from(l.pos[1])]);
                out.push(Self {
                    name: l.name.clone(),
                    pos: [p[0], f64::from(ele), p[1]],
                    kind: l.kind,
                    direction: l.direction,
                });
            }
        }
        out
    }

    /// A viewpoint at `pos` looking `direction` (all round: `None`).
    pub fn viewpoint(name: &str, pos: [f64; 3], direction: Option<ViewDirection>) -> Self {
        Self {
            name: name.into(),
            pos,
            kind: LandmarkKind::Viewpoint,
            direction,
        }
    }
}

/// What the photographer sees of the world.
pub trait PhotoScene {
    /// The ground (terrain surface) at local (x, z); `None` outside the
    /// known world.
    fn ground(&self, x: f64, z: f64) -> Option<f64>;
    /// Whether (x, z) is water (a lake, a river, the sea).
    fn water(&self, _x: f64, _z: f64) -> bool {
        false
    }
    /// Height of the tree crowns above the ground at (x, z) (m; 0: none).
    fn canopy_m(&self, _x: f64, _z: f64) -> f64 {
        0.0
    }
    /// Top of a building or another solid object at (x, z) (absolute m).
    fn solid_top(&self, _x: f64, _z: f64) -> Option<f64> {
        None
    }
    /// Rock, scree or ice: nobody stands there for a photo.
    fn rocky(&self, _x: f64, _z: f64) -> bool {
        false
    }
    /// A tree trunk, post or other obstacle a person cannot stand in.
    fn blocked(&self, _x: f64, _z: f64) -> bool {
        false
    }
    /// The named peaks and viewpoints (none where the world has none).
    fn landmarks(&self) -> &[Landmark] {
        &[]
    }
}

/// The light and the weather at the moment of the photo.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Conditions {
    /// The sun (° above the horizon; azimuth ° clockwise from north).
    pub sun_elevation_deg: f64,
    pub sun_azimuth_deg: f64,
    /// Visibility in fog, haze and precipitation (m).
    pub visibility_m: f64,
    /// Precipitation (mm/h).
    pub precip_mm_h: f64,
}

impl Default for Conditions {
    /// A clear afternoon.
    fn default() -> Self {
        Self {
            sun_elevation_deg: 30.0,
            sun_azimuth_deg: 200.0,
            visibility_m: 40_000.0,
            precip_mm_h: 0.0,
        }
    }
}

/// The score of a view and its parts (each 0–1; the sidecar keeps them).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ViewScore {
    /// The total (0–1).
    pub total: f64,
    pub open: f64,
    pub relief: f64,
    pub water: f64,
    pub landmark: f64,
    /// The factors.
    pub light: f64,
    pub weather: f64,
    pub framing: f64,
    /// Mean reach of the horizon rays (m).
    pub reach_m: f64,
}

fn len2(v: [f64; 2]) -> f64 {
    (v[0] * v[0] + v[1] * v[1]).sqrt()
}

fn norm3(v: [f64; 3]) -> [f64; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-9);
    [v[0] / l, v[1] / l, v[2] / l]
}

fn sub3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// The horizontal unit vector at `azimuth` (rad, clockwise from north;
/// x east, z south).
fn heading(azimuth: f64) -> [f64; 2] {
    [azimuth.sin(), -azimuth.cos()]
}

/// The azimuth (rad, clockwise from north) of a horizontal vector.
pub fn azimuth_of(d: [f64; 2]) -> f64 {
    d[0].atan2(-d[1])
}

/// The light factor: golden hour best, midday a little flat, the blue
/// hour lower, night very low (moon and stars postponed, spec §14 Phase 9
/// Q7); looking straight into a low sun flares.
pub fn light_factor(c: &Conditions, look_azimuth: f64) -> f64 {
    let e = c.sun_elevation_deg;
    let base = if e < -6.0 {
        0.05
    } else if e < 0.0 {
        0.6
    } else if e < 12.0 {
        1.0
    } else if e < 35.0 {
        1.0 - 0.1 * (e - 12.0) / 23.0
    } else {
        0.8
    };
    let into = (look_azimuth - c.sun_azimuth_deg.to_radians()).cos();
    let flare = if (0.0..20.0).contains(&e) && into > 0.97 {
        0.85
    } else {
        1.0
    };
    base * flare
}

/// The weather factor: rain and snow dull the picture (fog is in the
/// reach).
pub fn weather_factor(c: &Conditions) -> f64 {
    let p = c.precip_mm_h.max(0.0);
    let rain = 1.0 - (p / 8.0).min(0.6);
    let fog = if c.visibility_m < 300.0 { 0.4 } else { 1.0 };
    rain * fog
}

/// What one horizon ray saw.
#[derive(Debug, Clone, Copy, Default)]
struct Ray {
    reach_m: f64,
    water: f64,
    lowest: f64,
    highest: f64,
}

/// A horizon ray from `eye` along `dir` (horizontal unit): the ground
/// (with the tree crowns on it) is marched outwards at growing steps; a
/// sample is seen when it rises above every nearer one in elevation
/// angle. Stops at the visibility, or where the world ends (seen so far).
fn ray(scene: &dyn PhotoScene, eye: [f64; 3], dir: [f64; 2], visibility_m: f64) -> Ray {
    let mut r = 12.0;
    let mut max_angle = f64::NEG_INFINITY;
    let mut out = Ray {
        reach_m: 0.0,
        lowest: f64::INFINITY,
        highest: f64::NEG_INFINITY,
        ..Ray::default()
    };
    let mut wet = false;
    let limit = visibility_m.clamp(20.0, MAX_REACH_M);
    while r <= limit {
        let (x, z) = (eye[0] + dir[0] * r, eye[2] + dir[1] * r);
        let Some(g) = scene.ground(x, z) else {
            out.reach_m = r;
            break;
        };
        let top = g + scene.canopy_m(x, z);
        // The earth's curvature (with the usual refraction): far flat
        // land sinks below the horizon.
        let top = scene.solid_top(x, z).map_or(top, |s| s.max(top)) - r * r * 0.87 / 12.742e6;
        let angle = (top - eye[1]) / r;
        if angle >= max_angle {
            max_angle = angle;
            out.reach_m = r;
            wet |= scene.water(x, z);
            if r > 250.0 {
                out.lowest = out.lowest.min(g);
                out.highest = out.highest.max(g);
            }
        }
        // A wall right in front: the rest is hidden.
        if max_angle > 1.5 && r > 40.0 {
            break;
        }
        r *= 1.3;
    }
    out.water = if wet { 1.0 } else { 0.0 };
    out
}

/// The view score from `eye` looking along `look` (unit vector), under
/// `cond`; `framing` is the rider's framing factor (1 when no rider).
pub fn view_score(
    scene: &dyn PhotoScene,
    eye: [f64; 3],
    look: [f64; 3],
    cond: &Conditions,
    framing: f64,
) -> ViewScore {
    // Under the ground (a tunnel): nothing to see.
    if scene.ground(eye[0], eye[2]).is_some_and(|g| g > eye[1]) {
        return ViewScore::default();
    }
    let az = azimuth_of([look[0], look[2]]);
    let hfov = 2.0 * ((PHOTO_FOV_Y / 2.0).tan() * PHOTO_ASPECT).atan();
    const RAYS: usize = 7;
    let mut open = 0.0;
    let mut reach = 0.0;
    let mut water = 0.0;
    let (mut lowest, mut highest) = (f64::INFINITY, f64::NEG_INFINITY);
    for i in 0..RAYS {
        let a = az + hfov * (i as f64 / (RAYS - 1) as f64 - 0.5);
        let r = ray(scene, eye, heading(a), cond.visibility_m);
        open += ((r.reach_m.max(30.0) / 30.0).ln() / (MAX_REACH_M / 30.0).ln()).clamp(0.0, 1.0);
        reach += r.reach_m;
        water += r.water;
        lowest = lowest.min(r.lowest);
        highest = highest.max(r.highest);
    }
    let n = RAYS as f64;
    let open = open / n;
    let water = (water / n * 2.0).min(1.0);
    let relief = if highest.is_finite() {
        let span = ((highest - lowest) / 600.0).clamp(0.0, 1.0);
        let below = ((eye[1] - lowest) / 400.0).clamp(0.0, 1.0);
        span.max(below)
    } else {
        0.0
    };
    let landmark = landmark_in_view(scene, eye, az, hfov);
    let light = light_factor(cond, az);
    let weather = weather_factor(cond);
    let base = 0.4 * open + 0.3 * relief + 0.2 * water + 0.1 * landmark;
    ViewScore {
        total: (base * light * weather * framing).clamp(0.0, 1.0),
        open,
        relief,
        water,
        landmark,
        light,
        weather,
        framing,
        reach_m: reach / n,
    }
}

/// The landmark part of the view score (spec 2.11 Q4): 1 when a named
/// peak within [`PEAK_REACH_M`] is in the frame and in sight, or when the
/// camera stands at a viewpoint (within [`VIEWPOINT_NEAR_M`]) looking its
/// way (its `direction`, 30° more each side; any way when it has none);
/// [`VIEWPOINT_IN_FRAME`] when a viewpoint within [`VIEWPOINT_REACH_M`]
/// is in the frame and in sight; else 0. Passes do not count.
fn landmark_in_view(scene: &dyn PhotoScene, eye: [f64; 3], az: f64, hfov: f64) -> f64 {
    let mut best: f64 = 0.0;
    for l in scene.landmarks() {
        let d = [l.pos[0] - eye[0], l.pos[2] - eye[2]];
        let dist = len2(d);
        let (reach, part) = match l.kind {
            LandmarkKind::Peak => (PEAK_REACH_M, 1.0),
            LandmarkKind::Viewpoint => (VIEWPOINT_REACH_M, VIEWPOINT_IN_FRAME),
            LandmarkKind::Pass => continue,
        };
        if l.kind == LandmarkKind::Viewpoint && dist < VIEWPOINT_NEAR_M {
            let looks = l
                .direction
                .is_none_or(|v| v.contains(az.to_degrees().rem_euclid(360.0), 30.0));
            if looks {
                return 1.0;
            }
            continue;
        }
        if part <= best || !(50.0..reach).contains(&dist) {
            continue;
        }
        let off = (azimuth_of(d) - az + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU)
            - std::f64::consts::PI;
        if off.abs() > hfov / 2.0 {
            continue;
        }
        // Aimed a little above the summit (or the platform), the last
        // stretch not tested: the drawn ground there may lie a few metres
        // above the tagged elevation.
        let (lift, skip) = match l.kind {
            LandmarkKind::Peak => (5.0, 60.0_f64.min(dist * 0.1)),
            _ => (2.0, 20.0),
        };
        let target = [l.pos[0], l.pos[1] + lift, l.pos[2]];
        if !line_blocked(scene, eye, target, skip) {
            best = best.max(part);
            if best >= 1.0 {
                return 1.0;
            }
        }
    }
    best
}

/// The kinds of poses (spec §2.8.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PoseKind {
    /// Ahead of the rider and low (crouching): the rider coming up.
    AheadLow,
    /// From the side: the valley behind the rider.
    Side,
    /// From behind: the road ahead into the landscape.
    Behind,
    /// Farther away at eye height (a person on the hillside, or a drone
    /// low over ground nobody stands on): the rider small.
    Wide,
    /// Behind and above (a drone): the road ahead into the landscape.
    BehindAbove,
    /// High and far out (a drone): the rider small in the landscape.
    AerialWide,
}

impl PoseKind {
    pub fn label(self) -> &'static str {
        match self {
            PoseKind::AheadLow => "ahead, low",
            PoseKind::Side => "side",
            PoseKind::Behind => "behind",
            PoseKind::Wide => "wide",
            PoseKind::BehindAbove => "behind, above",
            PoseKind::AerialWide => "aerial, wide",
        }
    }

    /// Only a drone gets there.
    pub fn aerial(self) -> bool {
        matches!(self, PoseKind::BehindAbove | PoseKind::AerialWide)
    }
}

/// Who holds the camera (the user's requests of 2026-11-16: a real
/// photographer where a person can stand, else a drone the photographer
/// flies from somewhere near).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Carrier {
    /// The photographer stands (or crouches) at the pose.
    Person { crouch: bool },
    /// A drone is at the pose; the pilot stands at `pilot` (feet).
    Drone { pilot: [f64; 3] },
}

/// A camera pose: the eye (the lens) and where it looks, and who holds
/// the camera there.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraPose {
    pub kind: PoseKind,
    pub carrier: Carrier,
    /// The photographer's feet (a person) or the ground under the drone.
    pub feet: [f64; 3],
    pub eye: [f64; 3],
    /// Unit view direction.
    pub look: [f64; 3],
}

impl CameraPose {
    /// Heading (rad, clockwise from north) and pitch (rad, up) of the view.
    pub fn yaw_pitch(&self) -> (f64, f64) {
        (
            azimuth_of([self.look[0], self.look[2]]),
            self.look[1].clamp(-1.0, 1.0).asin(),
        )
    }

    pub fn is_drone(&self) -> bool {
        matches!(self.carrier, Carrier::Drone { .. })
    }
}

/// Where the rider will be at the moment of the photo.
#[derive(Debug, Clone, PartialEq)]
pub struct ShotSpot {
    /// The rider's road position (local metres; y the road).
    pub rider: [f64; 3],
    /// Travel direction (horizontal unit).
    pub heading: [f64; 2],
    /// The riding line around the spot (x, z) and the road's centre line
    /// with its half width: the photographer stays off both.
    pub line: Vec<[f64; 2]>,
    pub centre: Vec<[f64; 2]>,
    pub half_width_m: f64,
}

/// Why a person cannot stand somewhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unreachable {
    OutsideWorld,
    Water,
    /// Rock, scree, glacier.
    Rocky,
    InsideSolid,
    Obstacle,
    OnRoad,
    OnRidingLine,
    TooSteep,
    Distance,
}

/// Distance from p to the polyline (∞ when empty).
pub fn polyline_distance(p: [f64; 2], line: &[[f64; 2]]) -> f64 {
    if line.len() == 1 {
        return len2([p[0] - line[0][0], p[1] - line[0][1]]);
    }
    line.windows(2)
        .map(|w| crate::obstacles::segment_distance(p, w[0], w[1]))
        .fold(f64::INFINITY, f64::min)
}

/// Whether a person can stand at (x, z) near the rider at `spot`: on the
/// ground (not water, rock, scree or ice), not inside a building, tree
/// trunk or obstacle, off the road and at least 1.5 m from the riding
/// line, on ground not steeper than [`MAX_STAND_SLOPE`], and within
/// `range` (m) of the rider. `Ok` gives the feet's height.
pub fn standable(
    scene: &dyn PhotoScene,
    spot: &ShotSpot,
    x: f64,
    z: f64,
    range: std::ops::RangeInclusive<f64>,
) -> Result<f64, Unreachable> {
    let d = len2([x - spot.rider[0], z - spot.rider[2]]);
    if !range.contains(&d) {
        return Err(Unreachable::Distance);
    }
    let g = scene.ground(x, z).ok_or(Unreachable::OutsideWorld)?;
    if polyline_distance([x, z], &spot.centre) < spot.half_width_m + MIN_ROAD_GAP_M {
        return Err(Unreachable::OnRoad);
    }
    if polyline_distance([x, z], &spot.line) < MIN_LINE_GAP_M {
        return Err(Unreachable::OnRidingLine);
    }
    // The body's footprint (±0.4 m).
    for (dx, dz) in [(0.0, 0.0), (0.4, 0.0), (-0.4, 0.0), (0.0, 0.4), (0.0, -0.4)] {
        let (px, pz) = (x + dx, z + dz);
        if scene.water(px, pz) {
            return Err(Unreachable::Water);
        }
        if scene.blocked(px, pz) {
            return Err(Unreachable::Obstacle);
        }
        if scene.solid_top(px, pz).is_some_and(|t| t > g + 0.3) {
            return Err(Unreachable::InsideSolid);
        }
    }
    if scene.rocky(x, z) {
        return Err(Unreachable::Rocky);
    }
    let slope = [(0.5, 0.0), (0.0, 0.5)]
        .iter()
        .filter_map(|(dx, dz)| {
            let a = scene.ground(x + dx, z + dz)?;
            let b = scene.ground(x - dx, z - dz)?;
            Some((a - b).abs())
        })
        .fold(0.0f64, f64::max);
    if slope > MAX_STAND_SLOPE {
        return Err(Unreachable::TooSteep);
    }
    Ok(g)
}

/// Whether a person can stand at (x, z) to take the photo (at
/// [`MIN_DISTANCE_M`] … [`MAX_DISTANCE_M`] from the rider).
pub fn reachable(
    scene: &dyn PhotoScene,
    spot: &ShotSpot,
    x: f64,
    z: f64,
) -> Result<f64, Unreachable> {
    standable(scene, spot, x, z, MIN_DISTANCE_M..=MAX_DISTANCE_M)
}

/// The highest thing at (x, z): the ground, the tree crowns, a solid.
pub fn top_at(scene: &dyn PhotoScene, x: f64, z: f64) -> Option<f64> {
    let g = scene.ground(x, z)?;
    let t = g + scene.canopy_m(x, z);
    Some(scene.solid_top(x, z).map_or(t, |s| s.max(t)))
}

/// Whether the line from `a` to `b` is blocked by the ground, a solid or
/// tree crowns (the last `skip_end` of it, at the target, is not tested).
fn line_blocked(scene: &dyn PhotoScene, a: [f64; 3], b: [f64; 3], skip_end: f64) -> bool {
    let d = sub3(b, a);
    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    if len < 0.5 {
        return false;
    }
    let step = (len / 400.0).max(0.4);
    let mut s = 0.3;
    while s < len - skip_end {
        let t = s / len;
        let p = [a[0] + d[0] * t, a[1] + d[1] * t, a[2] + d[2] * t];
        if let Some(g) = scene.ground(p[0], p[2]) {
            if g > p[1] - 0.05 {
                return true;
            }
            let crown = scene.canopy_m(p[0], p[2]);
            if crown > 3.0 && p[1] > g + 2.5 && p[1] < g + crown {
                return true;
            }
        }
        if scene.solid_top(p[0], p[2]).is_some_and(|top| top > p[1]) {
            return true;
        }
        s += step;
    }
    false
}

/// Whether the rider at `rider` is hidden from `eye`: the head or the
/// body behind the ground, a building or tree crowns.
pub fn occluded(scene: &dyn PhotoScene, eye: [f64; 3], rider: [f64; 3]) -> bool {
    [1.55, 1.0]
        .iter()
        .any(|h| line_blocked(scene, eye, [rider[0], rider[1] + h, rider[2]], 0.6))
}

/// Whether the line between `a` and `b` is clear (the drone's path, the
/// pilot's view of it).
pub fn clear_line(scene: &dyn PhotoScene, a: [f64; 3], b: [f64; 3]) -> bool {
    !line_blocked(scene, a, b, 0.3)
}

/// The view direction for a pose at `eye`: the rider on a third of the
/// frame (`side`: −1 left third, +1 right third) and a little low; and
/// the framing factor (the rider's size in the frame; 0 when cut by the
/// edge).
pub fn aim(eye: [f64; 3], rider: [f64; 3], side: f64) -> ([f64; 3], f64) {
    let centre = [rider[0], rider[1] + RIDER_HEIGHT_M * 0.5, rider[2]];
    let d = sub3(centre, eye);
    let horiz = len2([d[0], d[2]]).max(1e-6);
    let hfov = 2.0 * ((PHOTO_FOV_Y / 2.0).tan() * PHOTO_ASPECT).atan();
    // The rider a third off centre: the view turns the other way.
    let az = azimuth_of([d[0], d[2]]) - side * hfov / 6.0;
    let pitch_to = d[1].atan2(horiz);
    let pitch = (pitch_to + PHOTO_FOV_Y / 8.0).clamp(-1.2, 0.35);
    let h = heading(az);
    let look = norm3([h[0] * pitch.cos(), pitch.sin(), h[1] * pitch.cos()]);
    // The rider's extent in the frame (vertical fraction, by the
    // distance).
    let dist = (horiz * horiz + d[1] * d[1]).sqrt();
    let frac = RIDER_HEIGHT_M / (2.0 * dist * (PHOTO_FOV_Y / 2.0).tan());
    let top = ((rider[1] + RIDER_HEIGHT_M - eye[1]).atan2(horiz) - pitch).abs();
    let bottom = ((rider[1] - eye[1]).atan2(horiz) - pitch).abs();
    let inside = top < PHOTO_FOV_Y / 2.0 * 0.95 && bottom < PHOTO_FOV_Y / 2.0 * 0.95;
    let framing = if !inside || frac > 0.8 {
        0.0
    } else if (0.15..=0.55).contains(&frac) {
        1.0
    } else if frac > 0.55 {
        1.0 - (frac - 0.55) / 0.25 * 0.6
    } else if frac < 0.03 {
        // A speck in the landscape.
        0.2
    } else {
        // Small in the landscape: a wide shot is fine.
        0.55 + 0.45 * (frac - 0.03) / 0.12
    };
    (look, framing)
}

/// A candidate pose before the tests: its kind, the eye's (x, z), the eye
/// height (above the ground for the ground poses, above the road for the
/// aerial ones) and crouching.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Candidate {
    pub kind: PoseKind,
    pub x: f64,
    pub z: f64,
    pub height_m: f64,
    pub crouch: bool,
}

/// The candidate poses around `spot`: on the ground (a person, or a drone
/// at eye height where nobody can stand) and in the air (a drone).
pub fn candidates(spot: &ShotSpot) -> Vec<Candidate> {
    let h = spot.heading;
    let r = [-h[1], h[0]];
    let p = spot.rider;
    let at = |ahead: f64, side: f64| {
        (
            p[0] + h[0] * ahead + r[0] * side,
            p[2] + h[1] * ahead + r[1] * side,
        )
    };
    let edge = spot.half_width_m + MIN_ROAD_GAP_M;
    let mut out = Vec::new();
    let mut push = |kind, (x, z): (f64, f64), height_m, crouch| {
        out.push(Candidate {
            kind,
            x,
            z,
            height_m,
            crouch,
        })
    };
    for s in [-1.0, 1.0] {
        for (a, o) in [(12.0, 2.0), (18.0, 4.0)] {
            push(
                PoseKind::AheadLow,
                at(a, s * (edge + o)),
                EYE_CROUCHING_M,
                true,
            );
        }
        for (a, o) in [(2.0, 5.0), (-3.0, 9.0)] {
            push(PoseKind::Side, at(a, s * (edge + o)), EYE_STANDING_M, false);
        }
        for (a, o) in [(-8.0, 1.5), (-12.0, 3.0)] {
            push(
                PoseKind::Behind,
                at(a, s * (edge + o)),
                EYE_STANDING_M,
                false,
            );
        }
        for (a, o) in [(-10.0, 26.0), (10.0, 34.0)] {
            push(PoseKind::Wide, at(a, s * (edge + o)), EYE_STANDING_M, false);
        }
        push(PoseKind::BehindAbove, at(-12.0, s * 4.0), 9.0, false);
        for (a, o, up) in [(-15.0, 40.0, 20.0), (15.0, 50.0, 28.0)] {
            push(PoseKind::AerialWide, at(a, s * o), up, false);
        }
    }
    out
}

/// A drone flies no closer than this to the rider (m).
pub const DRONE_SAFE_M: f64 = 3.5;
/// A drone keeps this much above everything under it on its way (m).
pub const DRONE_CLEARANCE_M: f64 = 2.0;
/// A drone pose is at most this far from the rider (m).
pub const DRONE_MAX_DISTANCE_M: f64 = 60.0;
/// A person's photo is preferred when a drone's is barely better.
pub const DRONE_FACTOR: f64 = 0.92;

/// Where the drone's pilot stands for a shot of the rider at `spot` from
/// `eye`: beside the road near the spot, where a person can stand, the
/// rider sees the pilot on the approach and the pilot sees the drone.
pub fn pilot_spot(scene: &dyn PhotoScene, spot: &ShotSpot, eye: [f64; 3]) -> Option<[f64; 3]> {
    let h = spot.heading;
    let r = [-h[1], h[0]];
    let p = spot.rider;
    let edge = spot.half_width_m + MIN_ROAD_GAP_M;
    // The drone's side of the road first.
    let side = (eye[0] - p[0]) * r[0] + (eye[2] - p[2]) * r[1];
    let first = if side < 0.0 { -1.0 } else { 1.0 };
    // Seen by the rider on the approach: from the riding line's first
    // point (about 60 m before the spot), at eye height over the ground
    // there (the road follows the ground; a straight line back from the
    // spot may cut through a bend's hillside).
    let back = spot
        .line
        .first()
        .copied()
        .unwrap_or([p[0] - h[0] * 60.0, p[2] - h[1] * 60.0]);
    let base = scene.ground(back[0], back[1]).map_or(p[1], |g| g.max(p[1]));
    let before = [back[0], base + 1.5, back[1]];
    for s in [first, -first] {
        for ahead in [6.0, 14.0, 0.0, 22.0, -6.0] {
            for o in [1.5, 3.0, 5.0] {
                let (x, z) = (
                    p[0] + h[0] * ahead + r[0] * s * (edge + o),
                    p[2] + h[1] * ahead + r[1] * s * (edge + o),
                );
                let Ok(g) = standable(scene, spot, x, z, MIN_DISTANCE_M..=30.0) else {
                    continue;
                };
                let head = [x, g + EYE_STANDING_M, z];
                if clear_line(scene, before, head) && clear_line(scene, head, eye) {
                    return Some([x, g, z]);
                }
            }
        }
    }
    None
}

/// A chosen photo: the pose and its score.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shot {
    pub pose: CameraPose,
    pub score: ViewScore,
}

/// The eye and the ground under it for candidate `c`: a person where one
/// can stand (`Ok`), else a drone at the same place (`Err`: the eye kept
/// clear of what is under it).
fn place(scene: &dyn PhotoScene, spot: &ShotSpot, c: &Candidate) -> Option<(bool, [f64; 3], f64)> {
    let g = scene.ground(c.x, c.z)?;
    if !c.kind.aerial()
        && let Ok(g) = reachable(scene, spot, c.x, c.z)
    {
        return Some((true, [c.x, g + c.height_m, c.z], g));
    }
    let base = if c.kind.aerial() { spot.rider[1] } else { g };
    let top = top_at(scene, c.x, c.z).unwrap_or(g);
    let y = (base + c.height_m).max(top + 1.0);
    Some((false, [c.x, y, c.z], g))
}

/// The best pose for a photo of the rider at `spot`: every candidate
/// whose view of the rider is clear, aimed on the better third, scored;
/// held by a person where one can stand there, else by a drone with its
/// pilot near. `None` when no pose works or the road is covered (a
/// tunnel).
pub fn choose(scene: &dyn PhotoScene, spot: &ShotSpot, cond: &Conditions) -> Option<Shot> {
    choose_with(scene, spot, cond, false)
}

/// [`choose`]; `drone_only`: only the drone's poses (test runs:
/// `photographer drone`).
pub fn choose_with(
    scene: &dyn PhotoScene,
    spot: &ShotSpot,
    cond: &Conditions,
    drone_only: bool,
) -> Option<Shot> {
    if covered(scene, spot.rider) {
        return None;
    }
    let mut best: Option<Shot> = None;
    for c in candidates(spot) {
        let Some((person, eye, g)) = place(scene, spot, &c) else {
            continue;
        };
        if drone_only && person {
            continue;
        }
        let d = len2([eye[0] - spot.rider[0], eye[2] - spot.rider[2]]);
        let max = if person {
            MAX_DISTANCE_M
        } else {
            DRONE_MAX_DISTANCE_M
        };
        if !(MIN_DISTANCE_M..=max).contains(&d) || occluded(scene, eye, spot.rider) {
            continue;
        }
        let carrier = if person {
            Carrier::Person { crouch: c.crouch }
        } else {
            match pilot_spot(scene, spot, eye) {
                Some(pilot) => Carrier::Drone { pilot },
                None => continue,
            }
        };
        let factor = if person { 1.0 } else { DRONE_FACTOR };
        for side in [-1.0, 1.0] {
            let (look, framing) = aim(eye, spot.rider, side);
            if framing <= 0.0 {
                continue;
            }
            let mut score = view_score(scene, eye, look, cond, framing);
            score.total *= factor;
            if best.is_none_or(|b| score.total > b.score.total) {
                best = Some(Shot {
                    pose: CameraPose {
                        kind: c.kind,
                        carrier,
                        feet: [c.x, g, c.z],
                        eye,
                        look,
                    },
                    score,
                });
            }
        }
    }
    best
}

/// Whether the road at `rider` runs under the ground (a tunnel).
pub fn covered(scene: &dyn PhotoScene, rider: [f64; 3]) -> bool {
    scene
        .ground(rider[0], rider[2])
        .is_some_and(|g| g > rider[1] + COVERED_M)
}

/// A scene from closures (tests, tools).
pub struct FnScene<G, W, C>
where
    G: Fn(f64, f64) -> Option<f64>,
    W: Fn(f64, f64) -> bool,
    C: Fn(f64, f64) -> f64,
{
    pub ground: G,
    pub water: W,
    pub canopy: C,
    pub solids: Vec<([f64; 2], [f64; 2], f64)>,
    pub landmarks: Vec<Landmark>,
}

impl<G, W, C> PhotoScene for FnScene<G, W, C>
where
    G: Fn(f64, f64) -> Option<f64>,
    W: Fn(f64, f64) -> bool,
    C: Fn(f64, f64) -> f64,
{
    fn ground(&self, x: f64, z: f64) -> Option<f64> {
        (self.ground)(x, z)
    }
    fn water(&self, x: f64, z: f64) -> bool {
        (self.water)(x, z)
    }
    fn canopy_m(&self, x: f64, z: f64) -> f64 {
        (self.canopy)(x, z)
    }
    fn solid_top(&self, x: f64, z: f64) -> Option<f64> {
        self.solids
            .iter()
            .filter(|(lo, hi, _)| (lo[0]..=hi[0]).contains(&x) && (lo[1]..=hi[1]).contains(&z))
            .map(|s| s.2)
            .reduce(f64::max)
    }
    fn landmarks(&self) -> &[Landmark] {
        &self.landmarks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A straight road along −z (north) through the origin at height `y`.
    fn spot(y: f64) -> ShotSpot {
        ShotSpot {
            rider: [0.6, y, 0.0],
            heading: [0.0, -1.0],
            line: vec![[0.6, 60.0], [0.6, -60.0]],
            centre: vec![[0.0, 60.0], [0.0, -60.0]],
            half_width_m: 3.0,
        }
    }

    /// A Tenerife ridge: the road on a ridge at 1500 m running north, the
    /// slope falling to the sea in the west within 10 km, a valley and
    /// the sea in the east too.
    #[allow(clippy::type_complexity)]
    fn tenerife() -> FnScene<
        impl Fn(f64, f64) -> Option<f64>,
        impl Fn(f64, f64) -> bool,
        impl Fn(f64, f64) -> f64,
    > {
        let h = |x: f64, _z: f64| 1700.0 * (-x.abs() / 4000.0).exp() - 200.0;
        FnScene {
            ground: move |x, z| Some(h(x, z).max(-30.0)),
            water: move |x, z| h(x, z) <= 0.0,
            canopy: |_, _| 0.0,
            solids: Vec::new(),
            landmarks: Vec::new(),
        }
    }

    /// A Black Forest track in the trees: rolling hills at 800 m, forest
    /// (25 m crowns) everywhere but on the track.
    #[allow(clippy::type_complexity)]
    fn black_forest() -> FnScene<
        impl Fn(f64, f64) -> Option<f64>,
        impl Fn(f64, f64) -> bool,
        impl Fn(f64, f64) -> f64,
    > {
        FnScene {
            ground: |x: f64, z: f64| Some(800.0 + 40.0 * (x / 700.0).sin() * (z / 900.0).cos()),
            water: |_, _| false,
            canopy: |x: f64, _| if x.abs() < 4.0 { 0.0 } else { 25.0 },
            solids: Vec::new(),
            landmarks: Vec::new(),
        }
    }

    #[test]
    fn a_ridge_with_the_sea_scores_higher_than_a_track_in_the_trees() {
        let cond = Conditions::default();
        let t = choose(&tenerife(), &spot(1500.0), &cond).expect("a pose on the ridge");
        let bf_scene = black_forest();
        let bf_spot = spot(bf_scene.ground(0.6, 0.0).unwrap());
        let bf = choose(&bf_scene, &bf_spot, &cond).map_or(0.0, |s| s.score.total);
        assert!(t.score.total > MIN_SCORE + 0.05, "ridge: {:?}", t.score);
        assert!(t.score.water > 0.0 && t.score.relief > 0.5, "{:?}", t.score);
        assert!(bf < MIN_SCORE * 0.6, "forest track: {bf}");
        assert!(t.score.total > bf + 0.2);
    }

    #[test]
    fn a_tunnel_scores_zero() {
        // The road runs 30 m under a hill.
        let scene = FnScene {
            ground: |_, _| Some(530.0),
            water: |_, _| false,
            canopy: |_, _| 0.0,
            solids: Vec::new(),
            landmarks: Vec::new(),
        };
        let s = spot(500.0);
        assert!(covered(&scene, s.rider));
        assert_eq!(choose(&scene, &s, &Conditions::default()), None);
        let eye = [5.0, 501.6, 0.0];
        assert_eq!(
            view_score(&scene, eye, [0.0, 0.0, -1.0], &Conditions::default(), 1.0).total,
            0.0
        );
    }

    #[test]
    fn night_fog_and_rain_lower_the_score() {
        let scene = tenerife();
        let s = spot(1500.0);
        let day = choose(&scene, &s, &Conditions::default())
            .unwrap()
            .score
            .total;
        let night = Conditions {
            sun_elevation_deg: -20.0,
            ..Conditions::default()
        };
        let fog = Conditions {
            visibility_m: 150.0,
            ..Conditions::default()
        };
        let rain = Conditions {
            precip_mm_h: 6.0,
            ..Conditions::default()
        };
        let golden = Conditions {
            sun_elevation_deg: 6.0,
            ..Conditions::default()
        };
        for (c, what) in [(night, "night"), (fog, "fog"), (rain, "rain")] {
            let v = choose(&scene, &s, &c).unwrap().score.total;
            assert!(v < day * 0.7, "{what}: {v} vs {day}");
        }
        let g = choose(&scene, &s, &golden).unwrap().score.total;
        assert!(g >= day, "golden hour {g} vs {day}");
    }

    /// Where a person can stand (spec §2.8.1): a meadow or verge beside
    /// the road yes; water, rock, a steep slope, the road, the riding
    /// line, a building no – there a drone takes the photo.
    #[test]
    fn standable_ground_is_classified() {
        // A lake east of x = 12, a building west of x = -6, rock in the
        // north (z < -30), a steep slope in the south (z > 30).
        let scene = Classified;
        let s = spot(100.0);
        let at = |x, z| standable(&scene, &s, x, z, 0.0..=100.0);
        assert_eq!(at(8.0, 0.0), Ok(100.0), "the verge");
        assert_eq!(at(1.0, -10.0), Err(Unreachable::OnRoad));
        assert_eq!(at(20.0, -10.0), Err(Unreachable::Water));
        assert_eq!(at(-10.0, -10.0), Err(Unreachable::InsideSolid));
        assert_eq!(at(8.0, -40.0), Err(Unreachable::Rocky));
        assert_eq!(at(8.0, 40.0), Err(Unreachable::TooSteep));
        assert_eq!(
            reachable(&scene, &s, 0.0, -100.0),
            Err(Unreachable::Distance)
        );
        let mut narrow = spot(100.0);
        narrow.half_width_m = 0.5;
        assert_eq!(
            standable(&scene, &narrow, 1.5, -10.0, 0.0..=100.0),
            Err(Unreachable::OnRidingLine)
        );
        // The ground candidates over the lake are drone poses at eye
        // height, the aerial ones always.
        for c in candidates(&s) {
            let Some((person, eye, g)) = place(&scene, &s, &c) else {
                continue;
            };
            if c.kind.aerial() || (c.x > 12.0 && c.z.abs() < 25.0) {
                assert!(!person, "{c:?}");
            }
            if person {
                assert!(standable(&scene, &s, c.x, c.z, 0.0..=100.0).is_ok());
                assert!((eye[1] - g - c.height_m).abs() < 1e-9);
            } else {
                assert!(eye[1] >= top_at(&scene, c.x, c.z).unwrap() + 1.0 - 1e-9);
            }
        }
    }

    struct Classified;

    impl PhotoScene for Classified {
        fn ground(&self, x: f64, z: f64) -> Option<f64> {
            Some(if z > 30.0 {
                100.0 + (z - 30.0) * 2.0 + x * 0.0
            } else {
                100.0
            })
        }
        fn water(&self, x: f64, _z: f64) -> bool {
            x > 12.0
        }
        fn rocky(&self, _x: f64, z: f64) -> bool {
            z < -30.0
        }
        fn solid_top(&self, x: f64, z: f64) -> Option<f64> {
            ((-30.0..=-6.0).contains(&x) && (-25.0..=25.0).contains(&z)).then_some(112.0)
        }
    }

    /// Every chosen pose: a person stands where a person can stand, at
    /// standing or crouching eye height; a drone has its pilot on
    /// standable ground with a clear view of it.
    #[test]
    fn chosen_poses_are_held_by_a_person_or_a_drone() {
        let scene = Classified;
        for z0 in [-20.0, 0.0, 20.0] {
            let mut s = spot(100.0);
            s.rider[2] = z0;
            let shot = choose(&scene, &s, &Conditions::default()).expect("a pose");
            let p = shot.pose;
            match p.carrier {
                Carrier::Person { crouch } => {
                    assert!(reachable(&scene, &s, p.feet[0], p.feet[2]).is_ok(), "{p:?}");
                    let h = if crouch {
                        EYE_CROUCHING_M
                    } else {
                        EYE_STANDING_M
                    };
                    assert!((p.eye[1] - p.feet[1] - h).abs() < 1e-9);
                }
                Carrier::Drone { pilot } => {
                    assert!(standable(&scene, &s, pilot[0], pilot[2], 0.0..=100.0).is_ok());
                    assert!(clear_line(
                        &scene,
                        [pilot[0], pilot[1] + 1.6, pilot[2]],
                        p.eye
                    ));
                }
            }
            assert!(!occluded(&scene, p.eye, s.rider));
            // Drones only (test runs): a drone with its pilot.
            let d = choose_with(&scene, &s, &Conditions::default(), true).expect("a drone pose");
            assert!(d.pose.is_drone(), "{:?}", d.pose);
        }
    }

    #[test]
    fn an_occluded_pose_is_never_chosen() {
        // A high wall on both sides of the road: only poses between the
        // walls see the rider.
        let scene = FnScene {
            ground: |_, _| Some(100.0),
            water: |_, _| false,
            canopy: |_, _| 0.0,
            solids: vec![
                ([-7.0, -100.0], [-6.0, 100.0], 160.0),
                ([7.0, -100.0], [8.0, 100.0], 160.0),
            ],
            landmarks: Vec::new(),
        };
        let s = spot(100.0);
        let shot = choose(&scene, &s, &Conditions::default()).expect("a clear pose");
        assert!(!occluded(&scene, shot.pose.eye, s.rider));
        assert!(shot.pose.eye[0].abs() < 7.0, "{:?}", shot.pose);
        for c in candidates(&s) {
            if let Some((_, eye, _)) = place(&scene, &s, &c)
                && (c.x < -6.0 || c.x > 7.0)
                && eye[1] < 160.0
            {
                assert!(occluded(&scene, eye, s.rider), "{c:?}");
            }
        }
        // Tree crowns between (the line through the crowns): hidden.
        let trees = FnScene {
            ground: |_, _| Some(100.0),
            water: |_, _| false,
            canopy: |x: f64, _| {
                if x.abs() > 4.0 && x.abs() < 30.0 {
                    20.0
                } else {
                    0.0
                }
            },
            solids: Vec::new(),
            landmarks: Vec::new(),
        };
        assert!(occluded(&trees, [40.0, 112.0, 0.0], s.rider));
    }

    #[test]
    fn the_rider_is_framed_on_a_third_and_never_cut() {
        let eye = [8.0, 101.6, -2.0];
        let rider = [0.6, 100.0, 0.0];
        for side in [-1.0, 1.0] {
            let (look, framing) = aim(eye, rider, side);
            assert!(framing > 0.9, "{framing}");
            // The rider is off centre towards `side`.
            let to = norm3(sub3([rider[0], rider[1] + 0.9, rider[2]], eye));
            let right = [-look[2], 0.0, look[0]];
            let x = to[0] * right[0] + to[2] * right[2];
            assert!(x * side > 0.1, "side {side}: {x}");
        }
        // Too close: cut by the frame.
        assert_eq!(aim([0.6, 101.6, -0.8], rider, 1.0).1, 0.0);
        // A speck far away scores low.
        assert!(aim([200.0, 101.6, 0.0], rider, 1.0).1 < 0.4);
    }

    #[test]
    fn landmarks_in_view_add_to_the_score() {
        let flat = |marks: Vec<Landmark>| FnScene {
            ground: |_, _| Some(100.0),
            water: |_, _| false,
            canopy: |_, _| 0.0,
            solids: Vec::new(),
            landmarks: marks,
        };
        let peak = Landmark::peak("Pico Uno", [0.0, 900.0, -5000.0]);
        let eye = [0.0, 101.6, 0.0];
        let a = view_score(
            &flat(vec![]),
            eye,
            [0.0, 0.0, -1.0],
            &Conditions::default(),
            1.0,
        );
        let b = view_score(
            &flat(vec![peak.clone()]),
            eye,
            [0.0, 0.0, -1.0],
            &Conditions::default(),
            1.0,
        );
        let behind = view_score(
            &flat(vec![peak]),
            eye,
            [0.0, 0.0, 1.0],
            &Conditions::default(),
            1.0,
        );
        assert_eq!(b.landmark, 1.0);
        assert_eq!(behind.landmark, 0.0);
        assert!(b.total > a.total);
    }

    #[allow(clippy::type_complexity)]
    fn plain(
        marks: Vec<Landmark>,
        ground: fn(f64, f64) -> Option<f64>,
    ) -> FnScene<
        impl Fn(f64, f64) -> Option<f64>,
        impl Fn(f64, f64) -> bool,
        impl Fn(f64, f64) -> f64,
    > {
        FnScene {
            ground,
            water: |_, _| false,
            canopy: |_, _| 0.0,
            solids: Vec::new(),
            landmarks: marks,
        }
    }

    #[test]
    fn a_peak_behind_a_ridge_or_too_far_and_a_pass_do_not_count() {
        let eye = [0.0, 101.6, 0.0];
        let north = [0.0, 0.0, -1.0];
        let cond = Conditions::default();
        // A ridge 300 m high 1 km north hides a peak of 250 m 5 km out.
        let ridge = |_x: f64, z: f64| {
            Some(if (-1100.0..-900.0).contains(&z) {
                400.0
            } else {
                100.0
            })
        };
        let hidden = Landmark::peak("Pico Dos", [0.0, 250.0, -5000.0]);
        let s = view_score(&plain(vec![hidden.clone()], ridge), eye, north, &cond, 1.0);
        assert_eq!(s.landmark, 0.0);
        let flat = |_: f64, _: f64| Some(100.0);
        assert_eq!(
            view_score(&plain(vec![hidden], flat), eye, north, &cond, 1.0).landmark,
            1.0
        );
        let far = Landmark::peak("Pico Tres", [0.0, 3000.0, -20_000.0]);
        assert_eq!(
            view_score(&plain(vec![far], flat), eye, north, &cond, 1.0).landmark,
            0.0
        );
        let pass = Landmark {
            kind: LandmarkKind::Pass,
            ..Landmark::peak("Paso Uno", [0.0, 300.0, -3000.0])
        };
        assert_eq!(
            view_score(&plain(vec![pass], flat), eye, north, &cond, 1.0).landmark,
            0.0
        );
    }

    #[test]
    fn a_viewpoint_counts_at_it_looking_its_way_or_in_the_frame() {
        let eye = [0.0, 101.6, 0.0];
        let cond = Conditions::default();
        let flat = |_: f64, _: f64| Some(100.0);
        let west = crate::landmarks::parse_direction("W");
        // The camera 40 m from a viewpoint that looks west.
        let at = Landmark::viewpoint("", [30.0, 100.0, 25.0], west);
        let score = |m: &Landmark, look: [f64; 3]| {
            view_score(&plain(vec![m.clone()], flat), eye, look, &cond, 1.0)
        };
        assert_eq!(score(&at, [-1.0, 0.0, 0.0]).landmark, 1.0);
        assert_eq!(score(&at, [1.0, 0.0, 0.0]).landmark, 0.0, "looking east");
        // Without a direction: any way.
        let round = Landmark::viewpoint("Mirador Uno", [30.0, 100.0, 25.0], None);
        assert_eq!(score(&round, [1.0, 0.0, 0.0]).landmark, 1.0);
        // A viewpoint 1 km ahead in the frame: part of a peak's.
        let ahead = Landmark::viewpoint("Mirador Dos", [0.0, 100.0, -1000.0], None);
        let s = score(&ahead, [0.0, 0.0, -1.0]);
        assert_eq!(s.landmark, VIEWPOINT_IN_FRAME);
        assert_eq!(score(&ahead, [0.0, 0.0, 1.0]).landmark, 0.0);
        let none = view_score(&plain(vec![], flat), eye, [0.0, 0.0, -1.0], &cond, 1.0);
        assert!(s.total > none.total);
        // A peak beside it wins (1).
        let both = view_score(
            &plain(
                vec![ahead, Landmark::peak("Pico Uno", [200.0, 900.0, -6000.0])],
                flat,
            ),
            eye,
            [0.0, 0.0, -1.0],
            &cond,
            1.0,
        );
        assert_eq!(both.landmark, 1.0);
    }

    #[test]
    fn the_photographer_frames_a_peak_when_there_is_one() {
        // A flat plain, the road north; a peak 6 km east. Without it the
        // poses score alike; with it the chosen pose looks towards it.
        let cond = Conditions::default();
        let flat = |_: f64, _: f64| Some(100.0);
        let peak = Landmark::peak("Pico Cuatro", [6000.0, 1400.0, 0.0]);
        let with = choose(&plain(vec![peak], flat), &spot(100.0), &cond).unwrap();
        let without = choose(&plain(vec![], flat), &spot(100.0), &cond).unwrap();
        assert_eq!(with.score.landmark, 1.0, "{with:?}");
        assert!(with.score.total > without.score.total);
        assert!(with.pose.look[0] > 0.3, "looks east: {:?}", with.pose.look);
    }
}
