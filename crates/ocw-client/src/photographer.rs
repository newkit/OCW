//! The ride photographer (spec §2.11), Bevy-free and shared by the apps:
//! when to take a photo, where the photographer (or the drone) is, and
//! what the app draws, renders and hears. The world's side – the view
//! score, the camera poses, who can stand where, the drone's flight – is
//! `ocw_world::photo` / `ocw_world::photo_drone`; the Bevy glue is
//! `ocw_ui::photographer`, the files the app's.
//!
//! - **Limits** ([`Limits`], [`select`]): at most 5 photos per ride, at
//!   least 20 minutes apart, the best view of each 20-minute window, and
//!   only views at or above the minimum score (§2.8.1 Q3); [`select`] is
//!   the rule as a pure function over all views of a ride, the
//!   [`Photographer`] follows it while riding by planning ahead on the
//!   rider's predicted path.
//! - **Planning** ([`Photographer::tick`]): every few seconds the spots
//!   ahead on the known track within the window are scored and the best
//!   becomes the plan; it can still change until the photographer shows
//!   (a person: ~12 s before; a drone: at its launch, 30 s – 2 min before).
//!   Leaving the planned path (a junction, a U-turn, a stop) aborts the
//!   visit without a photo; the window stays open.
//! - **The visit** ([`Visit`]): the person appears out of the rider's
//!   way a few seconds before the rider reaches the spot, takes the photo
//!   when the rider gets there and vanishes once the rider has passed and
//!   it is out of view; the drone flies its [`DroneFlight`] and its pilot
//!   stands beside the road.
//! - **The flash** ([`flash`]): a 0.1 s burst at the camera with the
//!   shutter sound.

use ocw_world::Track;
use ocw_world::photo::{
    self, CameraPose, Carrier, Conditions, MIN_SCORE, PhotoScene, Shot, ShotSpot, ViewScore,
};
use ocw_world::photo_drone::{DroneFlight, DronePhase, DroneState, flight_duration};
use serde::{Deserialize, Serialize};

/// The photo's size (px).
pub const PHOTO_W: u32 = 2560;
pub const PHOTO_H: u32 = 1440;
/// A person shows this long before the rider reaches the spot (s), and at
/// the latest this long before, even in view.
pub const APPEAR_S: f64 = 12.0;
pub const APPEAR_LATEST_S: f64 = 6.0;
/// After the photo the photographer leaves once the rider is this far
/// past (m) and it is out of view, or after [`LEAVE_MAX_S`] in any case.
pub const LEAVE_PASSED_M: f64 = 15.0;
pub const LEAVE_MAX_S: f64 = 25.0;
/// Seconds between two planning ticks (game time).
pub const PLAN_EVERY_S: f64 = 5.0;
/// A plan is replaced by a better one only when this much better.
pub const REPLAN_GAIN: f64 = 1.05;
/// The flash: full for 30 ms, then gone within 0.12 s.
pub const FLASH_S: f64 = 0.12;
/// A spot ahead must be at least this far (s) for a person, the drone's
/// shortest flight plus a little for a drone.
pub const MIN_LEAD_S: f64 = APPEAR_S + 1.0;
/// The planned spot moved more than this (m): the rider left the path.
pub const OFF_PATH_M: f64 = 4.0;

/// The photographer's limits (the user's: ≤ 5 per ride, ≥ 20 min apart).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limits {
    pub max_photos: usize,
    /// Least time between two photos and the windows' length (s).
    pub gap_s: f64,
    pub min_score: f64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_photos: 5,
            gap_s: 20.0 * 60.0,
            min_score: MIN_SCORE,
        }
    }
}

impl Limits {
    /// The limits with time running `scale` times faster (test runs: a
    /// 20-minute window becomes 2 minutes at 10).
    pub fn scaled(scale: f64) -> Self {
        let d = Self::default();
        Self {
            gap_s: d.gap_s / scale.max(1.0),
            ..d
        }
    }

    /// The window of a moment `t` s into the ride.
    pub fn window(&self, t: f64) -> i64 {
        (t / self.gap_s).floor() as i64
    }

    /// Whether a photo at `t` is allowed after the photos at `taken`.
    pub fn allows(&self, taken: &[f64], t: f64, score: f64) -> bool {
        score >= self.min_score
            && taken.len() < self.max_photos
            && taken.iter().all(|&p| (t - p).abs() >= self.gap_s)
            && taken.iter().all(|&p| self.window(p) != self.window(t))
    }
}

/// The photos of a ride from all its views `(t s into the ride, score)`:
/// the best of each window that keeps the limits (the rule the
/// photographer follows; indices into `views`, in time order).
pub fn select(views: &[(f64, f64)], limits: &Limits) -> Vec<usize> {
    let mut order: Vec<usize> = (0..views.len()).collect();
    order.sort_by(|&a, &b| views[a].0.total_cmp(&views[b].0));
    let mut taken: Vec<usize> = Vec::new();
    let mut i = 0;
    while i < order.len() {
        let w = limits.window(views[order[i]].0);
        let mut j = i;
        while j < order.len() && limits.window(views[order[j]].0) == w {
            j += 1;
        }
        let times: Vec<f64> = taken.iter().map(|&k| views[k].0).collect();
        let best = order[i..j]
            .iter()
            .copied()
            .filter(|&k| limits.allows(&times, views[k].0, views[k].1))
            .max_by(|&a, &b| views[a].1.total_cmp(&views[b].1));
        if let Some(b) = best {
            taken.push(b);
        }
        i = j;
    }
    taken
}

/// The flash's brightness `since_s` after the photo (0 … 1).
pub fn flash(since_s: f64) -> f64 {
    if !(0.0..FLASH_S).contains(&since_s) {
        0.0
    } else if since_s < 0.03 {
        1.0
    } else {
        (-(since_s - 0.03) / 0.025).exp()
    }
}

/// The rider's spot at distance `d` on `track` (lateral offset `lateral`).
pub fn spot_at(track: &dyn Track, d: f64, lateral: f64) -> ShotSpot {
    let p = track.sample(d);
    let mut line = Vec::new();
    let mut centre = Vec::new();
    let mut s = -60.0;
    while s <= 60.0 {
        let q = track.sample(d + s);
        let o = q.offset_position(lateral);
        line.push([o[0], o[2]]);
        centre.push([q.position[0], q.position[2]]);
        s += 6.0;
    }
    ShotSpot {
        rider: p.offset_position(lateral),
        heading: p.direction,
        line,
        centre,
        half_width_m: track.road_width_at(d) * 0.5,
    }
}

/// The best shot at `spot`, a person's only when there is no time for a
/// drone's flight (`eta_s`).
pub fn shot_at(
    scene: &dyn PhotoScene,
    spot: &ShotSpot,
    cond: &Conditions,
    eta_s: f64,
    seed: u64,
) -> Option<(Shot, Option<f64>)> {
    shot_with(scene, spot, cond, eta_s, seed, false)
}

/// [`shot_at`]; `drone_only`: a drone's shot only (test runs).
pub fn shot_with(
    scene: &dyn PhotoScene,
    spot: &ShotSpot,
    cond: &Conditions,
    eta_s: f64,
    seed: u64,
    drone_only: bool,
) -> Option<(Shot, Option<f64>)> {
    let shot = photo::choose_with(scene, spot, cond, drone_only)?;
    match shot.pose.carrier {
        Carrier::Person { .. } => Some((shot, None)),
        Carrier::Drone { .. } => flight_duration(seed, eta_s).map(|d| (shot, Some(d))),
    }
}

/// A planned photo.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// The rider's distance at the spot (m) and the spot.
    pub at_m: f64,
    pub spot: ShotSpot,
    pub shot: Shot,
    /// A drone's flight before the photo (s).
    pub flight_s: Option<f64>,
    pub seed: u64,
}

/// What the person does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersonPhase {
    /// Not there yet.
    Hidden,
    /// Standing, waiting for the rider, the camera up.
    Waiting,
    /// The photo was taken.
    Shot,
    Gone,
}

/// The person's phase from the rider's time to the spot (`eta_s`; `None`
/// once at or past it), how far past (m), whether the photographer is in
/// the rider's view and the time since the photo.
pub fn person_phase(
    now: PersonPhase,
    eta_s: Option<f64>,
    passed_m: f64,
    in_view: bool,
    since_shot_s: Option<f64>,
) -> PersonPhase {
    match now {
        PersonPhase::Hidden => match eta_s {
            Some(e) if e <= APPEAR_LATEST_S || (e <= APPEAR_S && !in_view) => PersonPhase::Waiting,
            None => PersonPhase::Waiting,
            _ => PersonPhase::Hidden,
        },
        PersonPhase::Waiting => {
            if eta_s.is_none() {
                PersonPhase::Shot
            } else {
                PersonPhase::Waiting
            }
        }
        PersonPhase::Shot => {
            let s = since_shot_s.unwrap_or(0.0);
            if s >= LEAVE_MAX_S || (s > 1.0 && passed_m >= LEAVE_PASSED_M && !in_view) {
                PersonPhase::Gone
            } else {
                PersonPhase::Shot
            }
        }
        PersonPhase::Gone => PersonPhase::Gone,
    }
}

/// The photographer at work for one photo.
#[derive(Debug, Clone, PartialEq)]
pub enum Visit {
    Person {
        phase: PersonPhase,
        shot_at: Option<f64>,
    },
    Drone {
        flight: Box<DroneFlight>,
        pilot: [f64; 3],
        shot_at: Option<f64>,
        /// The drone has landed and the pilot may go.
        landed_at: Option<f64>,
        gone: bool,
    },
}

/// What the app draws this frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    /// The person (the photographer, or the drone's pilot): feet, the
    /// heading it faces (rad, clockwise from north), the pitch of the
    /// head and the camera (rad), crouching, with the drone's remote.
    pub person: Option<PersonView>,
    pub drone: Option<DroneState>,
    /// The flash: where and how bright (0 … 1).
    pub flash: Option<([f64; 3], f64)>,
    /// The seed of the look (clothes, hair) of this photographer.
    pub seed: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PersonView {
    pub feet: [f64; 3],
    pub yaw: f64,
    pub pitch: f64,
    pub crouch: bool,
    pub remote: bool,
    /// The lens (a person's camera): its front is here.
    pub lens: [f64; 3],
}

/// What happened in a frame.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Events {
    /// Take the photo now from this pose (the app renders it).
    pub photo: Option<Taken>,
    /// The shutter sound at this place.
    pub shutter: Option<[f64; 3]>,
    /// The visit ended without a photo (aborted).
    pub aborted: bool,
}

/// A photo taken: everything the sidecar needs but the files.
#[derive(Debug, Clone, PartialEq)]
pub struct Taken {
    /// 1, 2, … in the ride.
    pub n: usize,
    /// Seconds into the ride.
    pub ride_s: f64,
    pub pose: CameraPose,
    pub score: ViewScore,
}

/// The ride's photographer.
#[derive(Debug, Clone, PartialEq)]
pub struct Photographer {
    pub limits: Limits,
    /// Game time the ride started.
    start: Option<f64>,
    /// Ride seconds of the photos taken.
    taken: Vec<f64>,
    /// Where they were taken (x, z) and their kinds (variety).
    places: Vec<([f64; 2], photo::PoseKind)>,
    plan: Option<Plan>,
    visit: Option<Visit>,
    last_tick: Option<f64>,
    last_distance: Option<f64>,
    /// `photographer now`: the next photo without waiting (tests).
    forced: bool,
    /// `photographer drone`: the next photo by a drone (test runs).
    drone_only: bool,
    seed: u64,
}

impl Photographer {
    pub fn new(limits: Limits, seed: u64) -> Self {
        Self {
            limits,
            start: None,
            taken: Vec::new(),
            places: Vec::new(),
            plan: None,
            visit: None,
            last_tick: None,
            last_distance: None,
            forced: false,
            drone_only: false,
            seed,
        }
    }

    /// A new ride: no photos yet (an active visit is dropped).
    pub fn reset(&mut self) {
        *self = Self::new(self.limits, self.seed.wrapping_add(1));
    }

    /// [`Self::force`] with a drone's shot (`photographer drone`, tests).
    pub fn force_drone(&mut self) {
        self.force();
        self.drone_only = true;
    }

    /// Plans the next photo at once, without the 20 minutes, the window
    /// and the minimum score (`photographer now`, tests).
    pub fn force(&mut self) {
        self.forced = true;
        self.last_tick = None;
    }

    pub fn photos(&self) -> usize {
        self.taken.len()
    }

    pub fn plan(&self) -> Option<&Plan> {
        self.plan.as_ref()
    }

    pub fn visit(&self) -> Option<&Visit> {
        self.visit.as_ref()
    }

    /// The ride seconds at game time `t`.
    fn ride_s(&self, t: f64) -> f64 {
        t - self.start.unwrap_or(t)
    }

    /// Whether the plan may still change (nobody has shown up yet).
    fn soft(&self) -> bool {
        match &self.visit {
            None => true,
            Some(Visit::Person { phase, .. }) => *phase == PersonPhase::Hidden,
            Some(Visit::Drone { flight, .. }) => flight.phase() == DronePhase::Waiting,
        }
    }

    /// The score with the variety factor: a photo near an earlier one
    /// (within 1 km) or of the same kind as the last scores lower.
    fn varied(&self, shot: &Shot) -> f64 {
        let p = [shot.pose.eye[0], shot.pose.eye[2]];
        let near = self
            .places
            .iter()
            .any(|(q, _)| ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt() < 1000.0);
        let same = self
            .places
            .last()
            .is_some_and(|(_, k)| *k == shot.pose.kind);
        shot.score.total * if near { 0.8 } else { 1.0 } * if same { 0.9 } else { 1.0 }
    }

    /// Plans: scores spots ahead on `track` from the rider at `d` (speed
    /// `speed`) within the window and keeps the best (spec §2.11).
    #[allow(clippy::too_many_arguments)]
    fn plan_ahead(
        &mut self,
        t: f64,
        track: &dyn Track,
        d: f64,
        lateral: f64,
        speed: f64,
        scene: &dyn PhotoScene,
        cond: &Conditions,
    ) {
        let ride = self.ride_s(t);
        let speed = speed.max(2.0);
        let window_end = (self.limits.window(ride) + 1) as f64 * self.limits.gap_s;
        let earliest = self
            .taken
            .last()
            .map_or(0.0, |&p| p + self.limits.gap_s)
            .max(ride + MIN_LEAD_S);
        let span = if self.forced {
            240.0
        } else {
            (window_end - ride).min(240.0)
        };
        let mut horizon = span * speed;
        if let Some((_, to)) = track.extent() {
            horizon = horizon.min(to - d - 5.0);
        }
        if let Some(f) = track.finish() {
            horizon = horizon.min(f - d - 5.0);
        }
        let lead = MIN_LEAD_S * speed;
        if horizon < lead {
            return;
        }
        let step = ((horizon - lead) / 12.0).max(40.0);
        let mut ahead = lead;
        // Forced (tests): a fresh plan, whatever was planned.
        let mut best = if self.forced { None } else { self.plan.clone() };
        let mut best_v = best
            .as_ref()
            .map_or(0.0, |p| self.varied(&p.shot) * REPLAN_GAIN);
        while ahead <= horizon {
            let at = d + ahead;
            let eta = ahead / speed;
            let when = ride + eta;
            let in_window = self.forced || self.limits.window(when) == self.limits.window(ride);
            if (self.forced || when >= earliest) && in_window {
                let seed = self.seed ^ (at as u64).wrapping_mul(0x9E37_79B9);
                let spot = spot_at(track, at, lateral);
                let found = shot_with(scene, &spot, cond, eta, seed, self.drone_only);
                if let Some((shot, flight)) = found {
                    let v = self.varied(&shot);
                    let min = if self.forced {
                        0.0
                    } else {
                        self.limits.min_score
                    };
                    if v >= min && v > best_v {
                        best_v = v;
                        best = Some(Plan {
                            at_m: at,
                            spot,
                            shot: Shot {
                                score: ViewScore {
                                    total: v,
                                    ..shot.score
                                },
                                ..shot
                            },
                            flight_s: flight,
                            seed,
                        });
                    }
                }
            }
            ahead += step;
        }
        if best.is_some() {
            self.forced = false;
            self.drone_only = false;
        }
        if best != self.plan {
            self.plan = best;
            self.visit = self.plan.as_ref().map(|p| match p.shot.pose.carrier {
                Carrier::Person { .. } => Visit::Person {
                    phase: PersonPhase::Hidden,
                    shot_at: None,
                },
                Carrier::Drone { pilot } => Visit::Drone {
                    flight: Box::new(DroneFlight::new(
                        p.shot.pose,
                        pilot,
                        p.flight_s.unwrap_or(30.0),
                    )),
                    pilot,
                    shot_at: None,
                    landed_at: None,
                    gone: false,
                },
            });
        }
    }

    /// One frame at game time `t`: the rider at distance `d` (lateral
    /// offset, speed) on `track`; `in_view(p)` says whether a point is in
    /// the rider's camera view. Plans every [`PLAN_EVERY_S`], runs the
    /// visit, and returns what the app draws and what happened.
    #[allow(clippy::too_many_arguments)]
    pub fn frame(
        &mut self,
        t: f64,
        track: &dyn Track,
        d: f64,
        lateral: f64,
        speed: f64,
        scene: &dyn PhotoScene,
        cond: &Conditions,
        in_view: &dyn Fn([f64; 3]) -> bool,
    ) -> (Option<Scene>, Events) {
        self.start.get_or_insert(t);
        let mut ev = Events::default();
        // A U-turn or a jump back: the plan is off.
        let back = self.last_distance.is_some_and(|l| d < l - 20.0);
        self.last_distance = Some(d);
        if self.soft()
            && self.taken.len() < self.limits.max_photos
            && self.last_tick.is_none_or(|l| t - l >= PLAN_EVERY_S)
        {
            self.last_tick = Some(t);
            self.plan_ahead(t, track, d, lateral, speed, scene, cond);
        }
        let Some(plan) = self.plan.clone() else {
            return (None, ev);
        };
        // Left the planned path (a junction, a U-turn, a jump): abort.
        let now_there = track.sample(plan.at_m).offset_position(lateral);
        let moved = ((now_there[0] - plan.spot.rider[0]).powi(2)
            + (now_there[2] - plan.spot.rider[2]).powi(2))
        .sqrt();
        let to_go = plan.at_m - d;
        let eta = (to_go > 0.0).then(|| to_go / speed.max(0.5));
        let passed = (-to_go).max(0.0);
        let off_path = moved > OFF_PATH_M || back;
        let rider = track.sample(d).offset_position(lateral);
        let pose = plan.shot.pose;
        let mut scene_out = Scene {
            person: None,
            drone: None,
            flash: None,
            seed: plan.seed,
        };
        let mut finished = false;
        let ride_s = self.ride_s(t);
        let mut taken_now = false;
        match self.visit.as_mut() {
            Some(Visit::Person { phase, shot_at }) => {
                if off_path && *phase != PersonPhase::Shot {
                    ev.aborted = true;
                    finished = true;
                } else {
                    let since = shot_at.map(|s| t - s);
                    let next = person_phase(*phase, eta, passed, in_view(pose.feet), since);
                    if next == PersonPhase::Shot && *phase == PersonPhase::Waiting {
                        *shot_at = Some(t);
                        taken_now = true;
                    }
                    *phase = next;
                    if *phase == PersonPhase::Gone {
                        finished = true;
                    } else if *phase != PersonPhase::Hidden {
                        let (yaw, pitch) = pose.yaw_pitch();
                        let crouch = matches!(pose.carrier, Carrier::Person { crouch: true });
                        scene_out.person = Some(PersonView {
                            feet: pose.feet,
                            yaw,
                            pitch,
                            crouch,
                            remote: false,
                            lens: pose.eye,
                        });
                    }
                    if let Some(s) = *shot_at {
                        let f = flash(t - s);
                        if f > 0.0 {
                            scene_out.flash = Some((pose.eye, f));
                        }
                    }
                }
            }
            Some(Visit::Drone {
                flight,
                pilot,
                shot_at,
                landed_at,
                gone,
            }) => {
                if off_path && shot_at.is_none() {
                    if !ev.aborted && flight.phase() != DronePhase::Return {
                        ev.aborted = flight.phase() != DronePhase::Landed;
                    }
                    flight.abort(t);
                }
                let state = flight.step(t, rider, eta, scene);
                if state.shot {
                    *shot_at = Some(t);
                    taken_now = true;
                }
                if state.phase == DronePhase::Landed && landed_at.is_none() {
                    *landed_at = Some(t);
                }
                let visible = state.phase != DronePhase::Waiting
                    || eta.is_some_and(|e| e < flight.duration_s + 3.0);
                if let Some(l) = *landed_at {
                    let since = t - l;
                    if since >= LEAVE_MAX_S
                        || (passed >= LEAVE_PASSED_M && !in_view(*pilot))
                        || (off_path && !in_view(*pilot))
                    {
                        *gone = true;
                    }
                }
                if *gone {
                    finished = true;
                } else if visible {
                    // The pilot looks up at the drone.
                    let to = [
                        state.pos[0] - pilot[0],
                        state.pos[1] - (pilot[1] + photo::EYE_STANDING_M),
                        state.pos[2] - pilot[2],
                    ];
                    let h = (to[0] * to[0] + to[2] * to[2]).sqrt().max(1e-6);
                    scene_out.person = Some(PersonView {
                        feet: *pilot,
                        yaw: photo::azimuth_of([to[0], to[2]]),
                        pitch: to[1].atan2(h).clamp(-0.6, 1.2),
                        crouch: false,
                        remote: true,
                        lens: [pilot[0], pilot[1] + photo::EYE_STANDING_M, pilot[2]],
                    });
                    if state.phase != DronePhase::Landed {
                        scene_out.drone = Some(state);
                    }
                }
                if let Some(s) = *shot_at {
                    let f = flash(t - s);
                    if f > 0.0 {
                        scene_out.flash = Some((pose.eye, f));
                    }
                }
            }
            None => {}
        }
        if taken_now {
            self.taken.push(ride_s);
            self.places.push(([pose.eye[0], pose.eye[2]], pose.kind));
            ev.photo = Some(Taken {
                n: self.taken.len(),
                ride_s,
                pose,
                score: plan.shot.score,
            });
            ev.shutter = Some(pose.eye);
        }
        if finished {
            self.plan = None;
            self.visit = None;
            self.last_tick = None;
        }
        let any =
            scene_out.person.is_some() || scene_out.drone.is_some() || scene_out.flash.is_some();
        (any.then_some(scene_out), ev)
    }
}

/// A photo's sidecar (`photo-N.toml`): when, where, the score and its
/// parts, the camera pose.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhotoMeta {
    pub n: usize,
    /// The photo file (`photo-1.jpg`).
    pub file: String,
    /// UTC Unix seconds and the local time with offset (ISO 8601).
    pub time_unix_s: i64,
    pub time: String,
    /// Seconds into the ride.
    pub ride_s: f64,
    /// The ride (its FIT file's stem), the world and the route.
    pub ride: String,
    #[serde(default)]
    pub world: String,
    #[serde(default)]
    pub route: String,
    /// The camera's position (WGS84 °, m) – `None` on routes without a
    /// geographic origin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lat: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lon: Option<f64>,
    pub height_m: f64,
    /// The view: heading (° clockwise from north) and pitch (°, up).
    pub heading_deg: f64,
    pub pitch_deg: f64,
    pub score: ScoreMeta,
    pub camera: CameraMeta,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ScoreMeta {
    pub total: f64,
    pub open: f64,
    pub relief: f64,
    pub water: f64,
    pub landmark: f64,
    pub light: f64,
    pub weather: f64,
    pub framing: f64,
    pub reach_m: f64,
}

impl From<ViewScore> for ScoreMeta {
    fn from(s: ViewScore) -> Self {
        Self {
            total: s.total,
            open: s.open,
            relief: s.relief,
            water: s.water,
            landmark: s.landmark,
            light: s.light,
            weather: s.weather,
            framing: s.framing,
            reach_m: s.reach_m,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CameraMeta {
    /// "ahead, low", "side", … (`ocw_world::photo::PoseKind::label`).
    pub kind: String,
    /// "person, standing", "person, crouching" or "drone".
    pub carrier: String,
    /// Local metres (x, y up, z south).
    pub eye: [f64; 3],
    pub look: [f64; 3],
    /// The person's feet or the drone pilot's.
    pub feet: [f64; 3],
}

impl CameraMeta {
    pub fn of(p: &CameraPose) -> Self {
        let (carrier, feet) = match p.carrier {
            Carrier::Person { crouch: false } => ("person, standing", p.feet),
            Carrier::Person { crouch: true } => ("person, crouching", p.feet),
            Carrier::Drone { pilot } => ("drone", pilot),
        };
        Self {
            kind: p.kind.label().into(),
            carrier: carrier.into(),
            eye: p.eye,
            look: p.look,
            feet,
        }
    }
}

/// What the renderer draws of `scene` (`RideView::photographer`).
pub fn view(scene: &Scene) -> ocw_render::photographer::PhotographerView {
    use ocw_render::photographer::{DroneFigure, PersonFigure, PhotographerView};
    PhotographerView {
        person: scene.person.map(|p| PersonFigure {
            feet: p.feet,
            lens: p.lens,
            yaw: p.yaw,
            pitch: p.pitch,
            crouch: p.crouch,
            remote: p.remote,
            seed: scene.seed,
        }),
        drone: scene.drone.map(|d| DroneFigure {
            pos: d.pos,
            look: d.look,
            rotor: d.rotor as f32,
        }),
        flash: scene.flash.map(|(p, f)| (p, f as f32)),
    }
}

/// `photo-3.jpg`.
pub fn photo_name(n: usize) -> String {
    format!("photo-{n}.jpg")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_world::Route;
    use ocw_world::photo::FnScene;

    #[test]
    fn never_more_than_five_never_two_within_twenty_minutes() {
        let l = Limits::default();
        // A view every 30 s for 4 hours, all nice.
        let views: Vec<(f64, f64)> = (0..480)
            .map(|i| (i as f64 * 30.0, 0.5 + 0.4 * ((i as f64) * 0.37).sin().abs()))
            .collect();
        let s = select(&views, &l);
        assert_eq!(s.len(), 5);
        for w in s.windows(2) {
            assert!(views[w[1]].0 - views[w[0]].0 >= 1200.0);
        }
    }

    #[test]
    fn the_best_of_each_window_and_the_minimum_score() {
        let l = Limits::default();
        let views = [
            (60.0, 0.5),
            (300.0, 0.9), // best of window 0
            (900.0, 0.6),
            (1300.0, 0.2), // too dull
            (1500.0, 0.3), // too dull
            (2600.0, 0.7), // window 2, ≥ 20 min after 300
        ];
        assert_eq!(select(&views, &l), vec![1, 5]);
        // A short ride: its single best view, if nice enough.
        assert_eq!(
            select(&[(10.0, 0.4), (200.0, 0.6), (400.0, 0.5)], &l),
            vec![1]
        );
        assert_eq!(
            select(&[(10.0, 0.2), (200.0, 0.3)], &l),
            Vec::<usize>::new()
        );
        // The best of the next window comes too early after the last
        // photo: the next allowed one in that window is taken.
        let views = [(1100.0, 0.9), (1250.0, 0.95), (2350.0, 0.6)];
        assert_eq!(select(&views, &l), vec![0, 2]);
    }

    #[test]
    fn the_flash_is_a_tenth_of_a_second() {
        assert_eq!(flash(-0.01), 0.0);
        assert_eq!(flash(0.0), 1.0);
        assert_eq!(flash(0.02), 1.0);
        assert!(flash(0.06) < 0.5 && flash(0.06) > 0.0);
        assert_eq!(flash(FLASH_S), 0.0);
        assert_eq!(flash(1.0), 0.0);
    }

    /// The person's timeline: appears out of view before the rider comes
    /// (at the latest 6 s before, even in view), takes the photo at the
    /// spot, vanishes once passed and out of view.
    #[test]
    fn the_person_appears_shoots_and_vanishes() {
        use PersonPhase::*;
        assert_eq!(person_phase(Hidden, Some(20.0), 0.0, false, None), Hidden);
        assert_eq!(
            person_phase(Hidden, Some(10.0), 0.0, true, None),
            Hidden,
            "in view: waits"
        );
        assert_eq!(person_phase(Hidden, Some(10.0), 0.0, false, None), Waiting);
        assert_eq!(person_phase(Hidden, Some(5.0), 0.0, true, None), Waiting);
        assert_eq!(person_phase(Waiting, Some(0.5), 0.0, true, None), Waiting);
        assert_eq!(person_phase(Waiting, None, 0.1, true, None), Shot);
        assert_eq!(
            person_phase(Shot, None, 30.0, true, Some(5.0)),
            Shot,
            "still in view"
        );
        assert_eq!(person_phase(Shot, None, 30.0, false, Some(5.0)), Gone);
        assert_eq!(person_phase(Shot, None, 5.0, true, Some(LEAVE_MAX_S)), Gone);
    }

    /// A ride on the test loop at 8 m/s with time 10× faster: photos come
    /// within the limits, each with its shutter and a flash, the person or
    /// drone in the scene before.
    #[test]
    fn a_ride_gets_its_photos_within_the_limits() {
        let route = Route::test_loop();
        let mut p = Photographer::new(Limits::scaled(10.0), 7);
        let dt = 0.1;
        let speed = 8.0;
        let mut t = 0.0;
        let mut photos = Vec::new();
        let mut seen_before = false;
        let flat = FnScene {
            ground: |_, _| Some(-5.0),
            water: |x: f64, _| x > 400.0,
            canopy: |_, _| 0.0,
            solids: Vec::new(),
            landmarks: Vec::new(),
        };
        while t < 900.0 {
            let d = speed * t;
            let (sc, ev) = p.frame(
                t,
                &route,
                d,
                0.0,
                speed,
                &flat,
                &Conditions::default(),
                &|_| false,
            );
            if let Some(s) = &sc
                && (s.person.is_some() || s.drone.is_some())
                && photos.is_empty()
            {
                seen_before = true;
            }
            if let Some(ph) = ev.photo {
                assert_eq!(ev.shutter, Some(ph.pose.eye));
                photos.push(ph);
            }
            t += dt;
        }
        let gap = Limits::scaled(10.0).gap_s;
        assert!(!photos.is_empty(), "photos on the loop");
        assert!(photos.len() <= 5);
        assert!(seen_before, "the photographer shows before the photo");
        for w in photos.windows(2) {
            assert!(
                w[1].ride_s - w[0].ride_s >= gap - 1e-6,
                "{} {}",
                w[0].ride_s,
                w[1].ride_s
            );
        }
        for ph in &photos {
            assert!(ph.score.total >= MIN_SCORE);
        }
    }

    #[test]
    fn leaving_the_planned_path_aborts_without_a_photo() {
        let route = Route::test_loop();
        let flat = FnScene {
            ground: |_, _| Some(-5.0),
            water: |x: f64, _| x > 400.0,
            canopy: |_, _| 0.0,
            solids: Vec::new(),
            landmarks: Vec::new(),
        };
        let mut p = Photographer::new(Limits::scaled(10.0), 3);
        let mut t = 0.0;
        // Ride until a plan exists.
        while p.plan().is_none() && t < 60.0 {
            p.frame(
                t,
                &route,
                8.0 * t,
                0.0,
                8.0,
                &flat,
                &Conditions::default(),
                &|_| false,
            );
            t += 0.1;
        }
        assert!(p.plan().is_some(), "a plan");
        // A U-turn: back 50 m.
        let d = 8.0 * t - 50.0;
        let (_, ev) = p.frame(
            t,
            &route,
            d,
            0.0,
            8.0,
            &flat,
            &Conditions::default(),
            &|_| false,
        );
        assert!(ev.photo.is_none());
        let mut photos = 0;
        for i in 0..200 {
            let (_, ev) = p.frame(
                t + 0.1 * f64::from(i),
                &route,
                d - f64::from(i),
                0.0,
                8.0,
                &flat,
                &Conditions::default(),
                &|_| false,
            );
            photos += usize::from(ev.photo.is_some());
        }
        assert_eq!(photos, 0);
        assert_eq!(p.photos(), 0, "the window stays open");
    }

    #[test]
    fn sidecars_read_back() {
        let pose = CameraPose {
            kind: photo::PoseKind::Side,
            carrier: Carrier::Person { crouch: false },
            feet: [1.0, 2.0, 3.0],
            eye: [1.0, 3.6, 3.0],
            look: [0.0, 0.0, -1.0],
        };
        let m = PhotoMeta {
            n: 1,
            file: photo_name(1),
            time_unix_s: 1_794_518_045,
            time: "2026-11-16T10:14:05+01:00".into(),
            ride_s: 600.0,
            ride: "2026-11-16_09-04-05".into(),
            world: "Test world".into(),
            route: "Loop".into(),
            lat: Some(28.3),
            lon: Some(-16.6),
            height_m: 1503.6,
            heading_deg: 270.0,
            pitch_deg: -3.0,
            score: ViewScore::default().into(),
            camera: CameraMeta::of(&pose),
        };
        let text = toml::to_string_pretty(&m).unwrap();
        assert!(text.contains("person, standing"));
        assert_eq!(toml::from_str::<PhotoMeta>(&text).unwrap(), m);
        assert_eq!(photo_name(3), "photo-3.jpg");
    }
}
