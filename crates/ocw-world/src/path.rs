//! Free-ride path (spec §5.2): the rider's path is a list of edges (with
//! their riding direction) that is extended on the fly – automatically
//! where the road just continues, at the lock point before a junction with
//! the chosen exit (§2.2.1), and with a U-turn loop at dead ends, at the
//! region border and on request (§2.2.2).
//!
//! The rider still moves in 1D: the distance along the path. Corners at
//! junctions are rounded (a fillet of up to [`MAX_FILLET_TRIM_M`] each
//! side), so the lateral offset carries over smoothly instead of following
//! the raw graph corner. Old steps far behind the rider are dropped, so
//! memory stays bounded on long rides; step indices keep counting.
//!
//! The geometry is a pure function of the step list, so the server, the
//! client's prediction and the client's copy of other riders' paths agree.
//!
//! **Routes** (spec §2.7, §2.2.1 route mode): a path may follow a
//! [`RoutePlan`]. It is then extended with the route's next step instead of
//! the default exit (at a junction the route's exit becomes the default, so
//! the overlay shows the route's direction); choosing another exit leaves
//! the route, and [`RidePath::rejoin_route`] finds the way back. Whether a
//! step is on the route is again a pure function of the step list (a step
//! is on the route when it is the route's next step, or – off the route –
//! when it rides a later route edge in the route's direction), so the
//! server and the clients agree on it without sending it.

use std::sync::Arc;

use crate::graph::VertexId;
use crate::graph_route::RoutePlan;
use crate::network::{ARRIVAL_HEADING_M, Junction, RoadNetwork, SHARP_DEG, signed_angle, unit};
use crate::route::{Route, RoutePoint};
use crate::surface::Surface;
use crate::track::Track;

/// The exit choice locks this far before a junction (spec §2.2.1).
pub const LOCK_M: f64 = 20.0;
/// The path is extended through plain road continuations this far ahead.
pub const HORIZON_M: f64 = 2000.0;
/// Most junctions [`RidePath::preview`] decides ahead of the rider.
pub const PREVIEW_MAX_STEPS: usize = 64;
/// Steps that ended more than this far behind the rider are dropped.
pub const KEEP_BEHIND_M: f64 = 600.0;
/// Radius of the rounded corner at junctions (m).
pub const FILLET_RADIUS_M: f64 = 8.0;
/// A rounded corner starts at most this far before the junction (m); less
/// than [`LOCK_M`], so it is never under the rider when the exit locks.
pub const MAX_FILLET_TRIM_M: f64 = 10.0;
/// Largest radius of the U-turn loop (m); on narrower roads it is smaller
/// ([`uturn_radius`]).
pub const UTURN_RADIUS_M: f64 = 4.0;
/// Smallest radius of the U-turn loop (m): on paths narrower than ~3.8 m
/// the loop still overhangs a little, ridden at walking pace.
pub const UTURN_MIN_RADIUS_M: f64 = 1.5;
/// The loop stays this far inside the road edge (m).
pub const UTURN_EDGE_MARGIN_M: f64 = 0.4;
/// Before the loop the rider is steered to the centreline over this
/// distance (m), so the loop – centred on it – stays on the road.
pub const UTURN_RUNUP_M: f64 = 20.0;
/// Highest speed through the U-turn loop (m/s, ~10 km/h, spec §2.2.2).
pub const UTURN_SPEED_MPS: f64 = 2.8;
/// Sideways acceleration through tight U-turn loops (m/s²).
pub const UTURN_LATERAL_MPS2: f64 = 2.5;

/// Radius of the U-turn loop on a road `width_m` wide: the loop is `2 r`
/// wide, centred on the centreline, and stays [`UTURN_EDGE_MARGIN_M`]
/// inside the edges (spec §2.2.2).
pub fn uturn_radius(width_m: f64) -> f64 {
    (width_m / 2.0 - UTURN_EDGE_MARGIN_M).clamp(UTURN_MIN_RADIUS_M, UTURN_RADIUS_M)
}

/// Speed through a U-turn loop of radius `r`.
pub fn uturn_speed(r: f64) -> f64 {
    (UTURN_LATERAL_MPS2 * r).sqrt().min(UTURN_SPEED_MPS)
}

/// How far a loop of radius `r` reaches ahead of where it starts (m).
fn uturn_reach(r: f64) -> f64 {
    (1.0 + 3f64.sqrt()) * r
}
/// Speed through a turn sharper than 120° (m/s, ~15 km/h, spec §5.2).
pub const SHARP_TURN_SPEED_MPS: f64 = 4.2;
/// Deceleration of the automatic slow-down (m/s²).
pub const SLOW_DOWN_MPS2: f64 = 2.0;
/// Junction turns sharper than this (degrees) are taken at a speed with at
/// most [`CORNER_LATERAL_MPS2`] sideways acceleration in the rounded corner.
pub const CORNER_DEG: f64 = 60.0;
/// Sideways acceleration through junction corners (m/s², ~22 km/h at 8 m).
pub const CORNER_LATERAL_MPS2: f64 = 4.5;

/// How far before a junction its exit locks for a rider at `speed`:
/// [`LOCK_M`], or earlier when the slow-down for the sharpest turn needs
/// more room (the turn must be known before the rider brakes for it).
pub fn lock_distance(speed: f64) -> f64 {
    let v0 = SHARP_TURN_SPEED_MPS;
    LOCK_M + ((speed * speed - v0 * v0) / (2.0 * SLOW_DOWN_MPS2)).max(0.0)
}

/// Why a U-turn loop is in the path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TurnReason {
    /// The road ends (spec §2.2.2).
    DeadEnd,
    /// The region ends and no neighbouring package is installed.
    Boundary,
    /// The rider asked for it.
    Manual,
}

/// One piece of a path.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Segment {
    /// Along graph edge `edge` from `from_m` to `to_m`, metres from its
    /// `from` vertex (`from_m > to_m`: against the edge's direction).
    Edge { edge: usize, from_m: f64, to_m: f64 },
    /// A U-turn loop where the previous segment ends.
    Turnaround(TurnReason),
}

impl Segment {
    /// The full edge in a direction.
    pub fn whole(net: &RoadNetwork, edge: usize, forward: bool) -> Self {
        let len = net.geometry(edge).length();
        let (from_m, to_m) = if forward { (0.0, len) } else { (len, 0.0) };
        Self::Edge { edge, from_m, to_m }
    }
}

#[derive(Debug, Clone)]
struct Step {
    seg: Segment,
    /// The edge whose width, surface and name apply (the turnaround's:
    /// the edge it turns on).
    edge: usize,
    /// Path distance where the step starts.
    start_m: f64,
    /// Rounded corner from the previous step (starts at the previous
    /// step's trimmed end, ends on this step's raw line at `trim_start`).
    fillet: Vec<[f64; 3]>,
    fillet_len: f64,
    /// Raw centreline of the segment and its cumulative distances.
    raw: Vec<[f64; 3]>,
    raw_cum: Vec<f64>,
    trim_start: f64,
    /// Ridden centreline: fillet + raw from `trim_start` to its end minus
    /// the next step's `trim_start`.
    points: Vec<[f64; 3]>,
    len_m: f64,
    /// Turn angle into this step (degrees, positive = right).
    entry_angle_deg: f64,
    /// Route progress after this step, and which route step this step
    /// rides (`None`: off the route, or no route).
    route: RouteState,
    plan_index: Option<usize>,
}

/// Where a path is on its route (spec §2.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteState {
    /// On the route: step `next` of lap `lap` comes next.
    On { next: usize, lap: u32 },
    /// Off the route since route step `left_at`; a later step of the path
    /// that rides a route edge (from `left_at` on) in the route's direction
    /// rejoins it.
    Off { left_at: usize, lap: u32 },
    /// The route was ridden to its end (not a loop); free ride from here.
    Finished,
}

/// Progress on the route at a point of the path (HUD, spec §2.7).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RouteProgress {
    pub on_route: bool,
    pub finished: bool,
    /// Lap (0 = the first).
    pub lap: u32,
    /// Route distance ridden in this lap and the lap's length (m).
    pub done_m: f64,
    pub length_m: f64,
}

impl Step {
    fn raw_len(&self) -> f64 {
        *self.raw_cum.last().unwrap_or(&0.0)
    }

    fn end_m(&self) -> f64 {
        self.start_m + self.len_m
    }

    fn forward(&self) -> bool {
        match self.seg {
            Segment::Edge { from_m, to_m, .. } => to_m >= from_m,
            Segment::Turnaround(_) => true,
        }
    }

    /// Recomputes the ridden points for the next step's trim.
    fn rebuild(&mut self, next_trim: f64) {
        let mut pts = self.fillet.clone();
        let end = (self.raw_len() - next_trim).max(self.trim_start);
        let body = slice(&self.raw, &self.raw_cum, self.trim_start, end);
        let skip = usize::from(!pts.is_empty());
        pts.extend(body.into_iter().skip(skip));
        self.len_m = polyline_len(&pts);
        self.points = pts;
    }

    /// Direction of the raw line at its end (over the last metres).
    fn raw_end_dir(&self) -> [f64; 2] {
        let len = self.raw_len();
        let a = point_at(&self.raw, &self.raw_cum, (len - 5.0).max(0.0));
        let b = point_at(&self.raw, &self.raw_cum, len);
        unit([b[0] - a[0], b[2] - a[2]])
    }

    fn raw_start_dir(&self) -> [f64; 2] {
        let len = self.raw_len();
        let a = point_at(&self.raw, &self.raw_cum, 0.0);
        let b = point_at(&self.raw, &self.raw_cum, len.min(5.0));
        unit([b[0] - a[0], b[2] - a[2]])
    }
}

/// A rider's free-ride path through the road graph.
#[derive(Debug, Clone)]
pub struct RidePath {
    net: Arc<RoadNetwork>,
    /// The route this path follows, if any.
    plan: Option<Arc<RoutePlan>>,
    /// Index of `steps[0]` among all steps of this path.
    first_index: u64,
    steps: Vec<Step>,
    /// Geometry of the steps kept (distance 0 = `steps[0].start_m`).
    route: Route,
}

impl RidePath {
    /// A path starting `offset_m` along `edge` (from its `from` vertex),
    /// riding it forwards or backwards; the rider's distance there is
    /// `start_m`, and the first step gets index `first_index`.
    pub fn start(
        net: Arc<RoadNetwork>,
        edge: usize,
        offset_m: f64,
        forward: bool,
        start_m: f64,
        first_index: u64,
    ) -> Self {
        let len = net.geometry(edge).length();
        let offset = if len < 2.0 {
            if forward { 0.0 } else { len }
        } else if forward {
            offset_m.clamp(0.0, len - 1.0)
        } else {
            offset_m.clamp(1.0, len)
        };
        let seg = Segment::Edge {
            edge,
            from_m: offset,
            to_m: if forward { len } else { 0.0 },
        };
        Self::from_segments(net, &[seg], start_m, first_index)
            .expect("an edge segment is a valid start")
    }

    /// A path from a list of segments (e.g. received from the server).
    pub fn from_segments(
        net: Arc<RoadNetwork>,
        segments: &[Segment],
        start_m: f64,
        first_index: u64,
    ) -> Result<Self, String> {
        let first = *segments.first().ok_or("a path needs a segment")?;
        let Segment::Edge { edge, .. } = first else {
            return Err("a path starts on an edge".into());
        };
        if edge >= net.graph().edges.len() {
            return Err(format!("edge {edge} out of range"));
        }
        let mut step = make_step(&net, first, None, 0.0, start_m);
        step.rebuild(0.0);
        let mut path = Self {
            route: placeholder_route(),
            plan: None,
            net,
            first_index,
            steps: vec![step],
        };
        for &s in &segments[1..] {
            path.push(s)?;
        }
        path.rebuild_route();
        Ok(path)
    }

    /// A path starting at the route's start and following it.
    pub fn on_route(
        net: Arc<RoadNetwork>,
        plan: Arc<RoutePlan>,
        start_m: f64,
        first_index: u64,
    ) -> Self {
        let first = plan.steps[0];
        Self::from_segments(net, &[first], start_m, first_index)
            .expect("a route starts on an edge")
            .with_plan(Some(plan))
    }

    /// The same path following `plan` (or none); whether each step is on
    /// the route is worked out from the steps.
    pub fn with_plan(mut self, plan: Option<Arc<RoutePlan>>) -> Self {
        self.plan = plan;
        let mut state = None;
        for i in 0..self.steps.len() {
            let (st, idx) = self.route_after(state, &self.steps[i].seg);
            self.steps[i].route = st;
            self.steps[i].plan_index = idx;
            state = Some(st);
        }
        self
    }

    pub fn plan(&self) -> Option<&Arc<RoutePlan>> {
        self.plan.as_ref()
    }

    /// Route state at the end of the known path (`None` without a route).
    pub fn route_state(&self) -> Option<RouteState> {
        self.plan.as_ref()?;
        Some(self.steps.last().expect("never empty").route)
    }

    /// Route state and progress at path distance `d`.
    pub fn route_progress(&self, d: f64) -> Option<RouteProgress> {
        let plan = self.plan.as_ref()?;
        let k = self.step_index_at(d);
        let s = &self.steps[k];
        let before = if k > 0 {
            self.steps[k - 1].route
        } else {
            s.route
        };
        let lap = match before {
            RouteState::On { lap, .. } | RouteState::Off { lap, .. } => lap,
            RouteState::Finished => 0,
        };
        let mut p = RouteProgress {
            on_route: s.plan_index.is_some(),
            finished: matches!(before, RouteState::Finished),
            lap,
            done_m: 0.0,
            length_m: plan.length_m(),
        };
        if let Some(i) = s.plan_index {
            let from = match plan.step(&self.net, i, lap) {
                Some(Segment::Edge { from_m, .. }) => from_m,
                _ => 0.0,
            };
            let along = match (plan.steps[i], self.locate(d)) {
                (Segment::Edge { .. }, Some((_, _, off))) => (off - from).abs(),
                _ => 0.0,
            };
            let start = if i == 0 { 0.0 } else { plan.start_of(i) };
            p.done_m = (start + along).min(plan.length_m());
        }
        Some(p)
    }

    /// The route step to ride next, if on the route.
    fn planned(&self) -> Option<Segment> {
        match self.steps.last()?.route {
            RouteState::On { next, lap } => self.ridden(next, lap),
            _ => None,
        }
    }

    /// Route step `i` of lap `lap` as the path rides it: the route's last
    /// step runs on to the edge's end, so the path ends at a vertex and
    /// free ride can continue after the finish.
    fn ridden(&self, i: usize, lap: u32) -> Option<Segment> {
        let plan = self.plan.as_ref()?;
        let mut s = plan.step(&self.net, i, lap)?;
        if i + 1 == plan.steps.len()
            && !plan.is_loop
            && let Segment::Edge { edge, from_m, to_m } = s
        {
            let len = self.net.geometry(edge).length();
            s = Segment::Edge {
                edge,
                from_m,
                to_m: if to_m >= from_m { len } else { 0.0 },
            };
        }
        Some(s)
    }

    /// Route state after riding `seg` from `prev` (`None`: the first step),
    /// and which route step `seg` rides.
    fn route_after(&self, prev: Option<RouteState>, seg: &Segment) -> (RouteState, Option<usize>) {
        let Some(plan) = &self.plan else {
            return (RouteState::Finished, None);
        };
        let advance = |i: usize, lap: u32| {
            if i + 1 < plan.steps.len() {
                RouteState::On { next: i + 1, lap }
            } else if plan.is_loop {
                RouteState::On {
                    next: 0,
                    lap: lap + 1,
                }
            } else {
                RouteState::Finished
            }
        };
        let rejoin = |from: usize, lap: u32| {
            if !matches!(seg, Segment::Edge { .. }) {
                return None;
            }
            (from..plan.steps.len())
                .find(|&i| rides(seg, &plan.steps[i]))
                .map(|i| (advance(i, lap), Some(i)))
        };
        match prev {
            None => rejoin(0, 0).unwrap_or((RouteState::Off { left_at: 0, lap: 0 }, None)),
            Some(RouteState::On { next, lap }) => {
                if rides(seg, &plan.steps[next]) {
                    (advance(next, lap), Some(next))
                } else {
                    (RouteState::Off { left_at: next, lap }, None)
                }
            }
            Some(RouteState::Off { left_at, lap }) => {
                rejoin(left_at, lap).unwrap_or((RouteState::Off { left_at, lap }, None))
            }
            Some(RouteState::Finished) => (RouteState::Finished, None),
        }
    }

    pub fn network(&self) -> &Arc<RoadNetwork> {
        &self.net
    }

    /// Index of the oldest step kept.
    pub fn first_index(&self) -> u64 {
        self.first_index
    }

    /// Index the next appended step will get.
    pub fn next_index(&self) -> u64 {
        self.first_index + self.steps.len() as u64
    }

    /// The segments from step `index` on (for sending the path).
    pub fn segments_from(&self, index: u64) -> Vec<Segment> {
        let from = index.saturating_sub(self.first_index) as usize;
        self.steps.iter().skip(from).map(|s| s.seg).collect()
    }

    /// Path distance where step `index` starts.
    pub fn step_start_m(&self, index: u64) -> Option<f64> {
        let i = index.checked_sub(self.first_index)? as usize;
        self.steps.get(i).map(|s| s.start_m)
    }

    /// Path distance where the known path ends.
    pub fn end_m(&self) -> f64 {
        self.steps.last().expect("never empty").end_m()
    }

    /// Path distance where the kept part starts.
    pub fn begin_m(&self) -> f64 {
        self.steps[0].start_m
    }

    /// The geometry of the kept part as a route (distance 0 = [`Self::begin_m`]).
    pub fn route(&self) -> &Route {
        &self.route
    }

    /// The vertex the path ends at, if it ends at one.
    pub fn end_vertex(&self) -> Option<VertexId> {
        let last = self.steps.last()?;
        let Segment::Edge { edge, to_m, .. } = last.seg else {
            return None;
        };
        let len = self.net.geometry(edge).length();
        let e = self.net.edge(edge);
        if to_m <= 0.0 {
            Some(e.from)
        } else if to_m >= len {
            Some(e.to)
        } else {
            None
        }
    }

    /// The edge (and direction) the path ends on.
    pub fn arrival(&self) -> Option<(usize, bool)> {
        let last = self.steps.last()?;
        match last.seg {
            Segment::Edge { edge, .. } => Some((edge, last.forward())),
            Segment::Turnaround(_) => None,
        }
    }

    /// The junction where the path ends (two or more exits) and its path
    /// distance; the next decision (spec §2.2.1: only the next junction is
    /// active).
    pub fn next_junction(&self) -> Option<(Junction, f64)> {
        let v = self.end_vertex()?;
        let (edge, forward) = self.arrival()?;
        let heading = self
            .net
            .arriving_direction(edge, forward, ARRIVAL_HEADING_M);
        let mut j = self.net.junction(v, Some((edge, forward)), heading)?;
        // Route mode: the route's exit is the default (spec §2.2.1).
        if let Some(Segment::Edge { edge, from_m, to_m }) = self.planned()
            && let Some(i) = j
                .exits
                .iter()
                .position(|e| e.edge == edge && e.forward == (to_m >= from_m))
        {
            j.default = i;
            j.route = Some(i);
        }
        Some((j, self.end_m()))
    }

    /// Appends a segment.
    fn push(&mut self, seg: Segment) -> Result<(), String> {
        if let Segment::Edge { edge, .. } = seg
            && edge >= self.net.graph().edges.len()
        {
            return Err(format!("edge {edge} out of range"));
        }
        let prev = self.steps.last().expect("never empty");
        let mut step = make_step(&self.net, seg, Some(prev), 0.0, 0.0);
        // Round the corner when both are edges and they really turn.
        let both_edges =
            matches!(prev.seg, Segment::Edge { .. }) && matches!(seg, Segment::Edge { .. });
        let theta = signed_angle(prev.raw_end_dir(), step.raw_start_dir());
        let t = if both_edges && theta.abs() > 3f64.to_radians() {
            let avail_prev = (prev.raw_len() - prev.trim_start) * 0.45;
            (FILLET_RADIUS_M * (theta.abs() / 2.0).min(1.4).tan())
                .min(MAX_FILLET_TRIM_M)
                .min(avail_prev)
                .min(step.raw_len() * 0.45)
                .max(0.0)
        } else {
            0.0
        };
        if t > 0.05 {
            let p0 = point_at(&prev.raw, &prev.raw_cum, prev.raw_len() - t);
            let corner = *prev.raw.last().expect("raw has points");
            let p2 = point_at(&step.raw, &step.raw_cum, t);
            step.fillet = bezier(p0, corner, p2);
            step.fillet_len = polyline_len(&step.fillet);
            step.trim_start = t;
        }
        let n = self.steps.len();
        self.steps[n - 1].rebuild(step.trim_start);
        step.start_m = self.steps[n - 1].end_m();
        step.rebuild(0.0);
        (step.route, step.plan_index) = self.route_after(Some(self.steps[n - 1].route), &seg);
        self.steps.push(step);
        Ok(())
    }

    fn rebuild_route(&mut self) {
        let mut pts: Vec<[f64; 3]> = Vec::new();
        for s in &self.steps {
            let skip = usize::from(!pts.is_empty());
            pts.extend(s.points.iter().skip(skip));
        }
        let width = self.net.edge(self.steps[0].edge).width_m;
        self.route = Route::from_points("free-ride", "Free ride", pts, false, width)
            .unwrap_or_else(|_| placeholder_route_at(self.steps[0].points[0]));
    }

    /// [`Self::extend_at_speed`] for a rider standing still (lock point
    /// [`LOCK_M`] before the junction).
    pub fn extend(
        &mut self,
        rider_m: f64,
        choose: &mut dyn FnMut(&Junction) -> usize,
    ) -> Option<u64> {
        self.extend_at_speed(rider_m, 0.0, choose)
    }

    /// Extends the path for a rider at `rider_m` riding at `speed`: through
    /// plain road continuations up to [`HORIZON_M`] ahead, with a U-turn at
    /// dead ends and the region border, and at the lock point of the next
    /// junction ([`lock_distance`] before it) with the exit `choose`
    /// returns (an index into its exits). Returns the index of the first
    /// step that changed, if any.
    pub fn extend_at_speed(
        &mut self,
        rider_m: f64,
        speed: f64,
        choose: &mut dyn FnMut(&Junction) -> usize,
    ) -> Option<u64> {
        let lock = lock_distance(speed);
        let mut changed: Option<u64> = None;
        fn mark(i: u64, c: &mut Option<u64>) {
            *c = Some(c.map_or(i, |c| c.min(i)));
        }
        loop {
            // Plain continuations and road ends.
            while self.end_m() - rider_m < HORIZON_M {
                // On a route: its next step, unless that is a junction
                // decision (taken at the lock point below).
                if let Some(seg) = self.planned() {
                    match self.planned_action(seg) {
                        Planned::Push => {
                            mark(self.next_index(), &mut changed);
                            self.push(seg).expect("valid edge");
                            continue;
                        }
                        Planned::Turn => {
                            let first = self.turn_here(rider_m);
                            mark(first, &mut changed);
                            continue;
                        }
                        Planned::Junction => break,
                        // Not reachable from here: free ride (off the route).
                        Planned::Mismatch => {}
                    }
                }
                let (Some(v), Some((edge, forward))) = (self.end_vertex(), self.arrival()) else {
                    break;
                };
                let heading = self
                    .net
                    .arriving_direction(edge, forward, ARRIVAL_HEADING_M);
                let exits = self.net.exits(v, Some((edge, forward)), heading);
                match exits.len() {
                    0 => {
                        let reason = if self.net.graph().boundary_vertices.contains(&v) {
                            TurnReason::Boundary
                        } else {
                            TurnReason::DeadEnd
                        };
                        let first = self.turn_at_end(rider_m, reason);
                        mark(first, &mut changed);
                    }
                    1 => {
                        mark(self.next_index(), &mut changed);
                        let e = &exits[0];
                        self.push(Segment::whole(&self.net, e.edge, e.forward))
                            .expect("valid edge");
                    }
                    _ => break,
                }
            }
            // The lock point of the next junction.
            match self.next_junction() {
                Some((j, at)) if rider_m >= at - lock => {
                    let i = choose(&j).min(j.exits.len() - 1);
                    mark(self.next_index(), &mut changed);
                    let e = &j.exits[i];
                    // The route's step (it may end part-way along the edge).
                    let seg = match self.planned() {
                        Some(p @ Segment::Edge { edge, from_m, to_m })
                            if Some(i) == j.route
                                && edge == e.edge
                                && (to_m >= from_m) == e.forward =>
                        {
                            p
                        }
                        _ => Segment::whole(&self.net, e.edge, e.forward),
                    };
                    self.push(seg).expect("valid edge");
                }
                _ => break,
            }
        }
        if changed.is_some() {
            self.rebuild_route();
        }
        changed
    }

    /// The most likely path ahead of a rider at `rider_m`, reaching at
    /// least `ahead_m` beyond them where the road graph allows – for the
    /// HUD look-ahead (average gradient, elevation profile; spec §3.2). The
    /// real path ends at the next undecided junction, so this extends a
    /// copy as if the rider rode on: at that junction with exit `first`
    /// (the rider's current choice, an index into its exits; `None` = the
    /// default), at later ones with the default exit (route mode: the
    /// route's exit), and with the automatic U-turns at dead ends and the
    /// region border. `None` when the path already reaches far enough (use
    /// it as it is). The copy is display-only: never sent or ridden.
    pub fn preview(&self, rider_m: f64, ahead_m: f64, first: Option<usize>) -> Option<RidePath> {
        let want = rider_m + ahead_m;
        if self.end_m() >= want {
            return None;
        }
        let mut path = self.clone();
        let mut first = first;
        for _ in 0..PREVIEW_MAX_STEPS {
            if path.end_m() >= want {
                break;
            }
            // A rider standing at the junction: it is decided now.
            let at = path.next_junction().map_or(path.end_m(), |(_, at)| at);
            let changed = path.extend(at.max(rider_m), &mut |j| first.take().unwrap_or(j.default));
            if changed.is_none() {
                break;
            }
        }
        Some(path)
    }

    /// U-turn where the path ends at a road end: the last step is cut so
    /// the loop fits before the end (if the rider is not there yet), then
    /// the loop and the way back. Returns the first changed step index.
    fn turn_at_end(&mut self, rider_m: f64, reason: TurnReason) -> u64 {
        let n = self.steps.len();
        let last = &self.steps[n - 1];
        let mut first = self.next_index();
        let r = uturn_radius(self.net.edge(last.edge).width_m);
        let reach = uturn_reach(r) + 1.0;
        if let Segment::Edge { edge, from_m, to_m } = last.seg {
            let usable = (to_m - from_m).abs() - last.trim_start;
            let cut_at = last.end_m() - reach;
            if usable > reach + 2.0 && rider_m < cut_at - 1.0 {
                let to = if to_m > from_m {
                    to_m - reach
                } else {
                    to_m + reach
                };
                self.cut_last(edge, from_m, to);
                first = self.next_index() - 1;
            }
        }
        self.push_turnaround(reason);
        first
    }

    /// Replaces the last step's end (it must be an edge step).
    fn cut_last(&mut self, edge: usize, from_m: f64, to_m: f64) {
        let n = self.steps.len();
        let seg = Segment::Edge { edge, from_m, to_m };
        let old = &self.steps[n - 1];
        let mut step = make_step(&self.net, seg, None, old.entry_angle_deg, old.start_m);
        step.fillet = old.fillet.clone();
        step.fillet_len = old.fillet_len;
        step.trim_start = old.trim_start;
        // The same edge in the same direction: the route state is the same.
        step.route = old.route;
        step.plan_index = old.plan_index;
        step.rebuild(0.0);
        self.steps[n - 1] = step;
    }

    /// Appends a U-turn loop and the way back along the last edge.
    fn push_turnaround(&mut self, reason: TurnReason) {
        let last = self.steps.last().expect("never empty");
        let Segment::Edge { edge, from_m, to_m } = last.seg else {
            return;
        };
        let len = self.net.geometry(edge).length();
        let back_to = if to_m >= from_m { 0.0 } else { len };
        self.push(Segment::Turnaround(reason)).expect("valid");
        self.push(Segment::Edge {
            edge,
            from_m: to_m,
            to_m: back_to,
        })
        .expect("valid edge");
    }

    /// Whether [`Self::extend`] would change anything for a rider at
    /// `rider_m` (so a shared path is only copied when it must change).
    pub fn needs_extend(&self, rider_m: f64, speed: f64) -> bool {
        if let Some((_, at)) = self.next_junction() {
            return rider_m >= at - lock_distance(speed);
        }
        (self.end_vertex().is_some() || self.planned().is_some())
            && self.end_m() - rider_m < HORIZON_M
    }

    /// What to do with the route's next step `seg` where the path ends.
    fn planned_action(&self, seg: Segment) -> Planned {
        let Segment::Edge { edge, from_m, to_m } = seg else {
            return Planned::Turn;
        };
        let forward = to_m >= from_m;
        let (Some(v), Some(arrival)) = (self.end_vertex(), self.arrival()) else {
            return Planned::Mismatch;
        };
        if self.net.start_vertex(edge, forward) != v {
            return Planned::Mismatch;
        }
        let heading = self
            .net
            .arriving_direction(arrival.0, arrival.1, ARRIVAL_HEADING_M);
        let exits = self.net.exits(v, Some(arrival), heading);
        if exits.len() >= 2 && exits.iter().any(|e| e.edge == edge && e.forward == forward) {
            Planned::Junction
        } else {
            Planned::Push
        }
    }

    /// A U-turn of the route where the path ends: at a road end or
    /// junction like at a dead end (the last step is cut so the loop fits),
    /// part-way along an edge right there. Returns the first changed step.
    fn turn_here(&mut self, rider_m: f64) -> u64 {
        match (self.end_vertex(), self.arrival()) {
            (Some(v), Some(arrival)) => {
                let heading = self
                    .net
                    .arriving_direction(arrival.0, arrival.1, ARRIVAL_HEADING_M);
                let reason = if !self.net.exits(v, Some(arrival), heading).is_empty() {
                    TurnReason::Manual
                } else if self.net.graph().boundary_vertices.contains(&v) {
                    TurnReason::Boundary
                } else {
                    TurnReason::DeadEnd
                };
                self.turn_at_end(rider_m, reason)
            }
            _ => {
                let first = self.next_index();
                self.push_turnaround(TurnReason::Manual);
                first
            }
        }
    }

    /// Back to the route after leaving it (spec §2.2.1 "leave route"): the
    /// path is kept up to the first vertex the rider can still turn at
    /// (beyond the lock point at `speed`), then the shortest way to a route
    /// point from where the rider left on (weighing route skipped against
    /// the detour) and that route step are appended. Returns the first
    /// changed step index.
    pub fn rejoin_route(&mut self, rider_m: f64, speed: f64) -> Result<u64, String> {
        let plan = self.plan.clone().ok_or("not riding a route")?;
        let (left_at, lap) = match self.route_state() {
            Some(RouteState::Off { left_at, lap }) => (left_at, lap),
            Some(RouteState::On { .. }) => return Err("already on the route".into()),
            _ => return Err("the route is finished".into()),
        };
        // Keep the path up to a vertex beyond the lock point.
        let keep_to = rider_m + lock_distance(speed);
        let mut k = self.step_index_at(keep_to);
        loop {
            let s = &self.steps[k];
            let at_vertex = match s.seg {
                Segment::Edge { edge, to_m, .. } => {
                    let len = self.net.geometry(edge).length();
                    to_m <= 0.0 || to_m >= len
                }
                Segment::Turnaround(_) => false,
            };
            if at_vertex || k + 1 == self.steps.len() {
                break;
            }
            k += 1;
        }
        let first_changed = self.first_index + k as u64 + 1;
        if k + 1 < self.steps.len() {
            self.steps.truncate(k + 1);
            self.steps[k].rebuild(0.0);
        }
        let (Some(v), Some((arr_edge, arr_fwd))) = (self.end_vertex(), self.arrival()) else {
            self.rebuild_route();
            return Err("no road to turn at ahead".into());
        };
        // Shortest ways from v (not straight back along the arrival).
        let (dist, came) = directed_dijkstra(&self.net, v, Some((arr_edge, !arr_fwd)), 30_000.0);
        let mut best: Option<(f64, usize)> = None;
        for i in left_at..plan.steps.len() {
            let Some(Segment::Edge { edge, from_m, to_m }) = plan.step(&self.net, i, lap) else {
                continue;
            };
            let forward = to_m >= from_m;
            let len = self.net.geometry(edge).length();
            let starts_at_vertex = if forward {
                from_m <= 0.5
            } else {
                from_m >= len - 0.5
            };
            if !starts_at_vertex {
                continue;
            }
            let u = self.net.start_vertex(edge, forward);
            let d = dist[u.0 as usize];
            if !d.is_finite() {
                continue;
            }
            // Not a U-turn into the route step.
            let last_edge = came[u.0 as usize].map_or((arr_edge, arr_fwd), |(e, f, _)| (e, f));
            if last_edge == (edge, !forward) {
                continue;
            }
            let skipped = plan.start_of(i) - plan.start_of(left_at);
            let score = d + 0.3 * skipped.max(0.0);
            if best.is_none_or(|(b, _)| score < b) {
                best = Some((score, i));
            }
        }
        let Some((_, i)) = best else {
            self.rebuild_route();
            return Err("no way back to the route found".into());
        };
        let target = self.ridden(i, lap).expect("checked");
        let Segment::Edge { edge, from_m, to_m } = target else {
            unreachable!("edge steps only");
        };
        let mut detour = Vec::new();
        let mut u = self.net.start_vertex(edge, to_m >= from_m);
        while let Some((e, f, prev)) = came[u.0 as usize] {
            detour.push(Segment::whole(&self.net, e, f));
            u = prev;
        }
        detour.reverse();
        for s in detour {
            self.push(s)?;
        }
        self.push(target)?;
        self.rebuild_route();
        Ok(first_changed)
    }

    /// Whether path distance `d` lies on a step of the route.
    pub fn on_route_at(&self, d: f64) -> bool {
        self.steps[self.step_index_at(d)].plan_index.is_some()
    }

    /// Whether [`Self::trim_behind`] would drop steps.
    pub fn needs_trim(&self, rider_m: f64) -> bool {
        self.steps.len() > 1 && self.steps[0].end_m() < rider_m - KEEP_BEHIND_M
    }

    /// Manual U-turn (spec §2.2.2) for a rider at `rider_m` riding at
    /// `speed`: the loop starts after the slow-down to U-turn speed; the
    /// path beyond is replaced by the way back. Returns the first changed
    /// step index, or `None` if no U-turn fits (in a loop already, or no
    /// room before the next corner).
    pub fn u_turn(&mut self, rider_m: f64, speed: f64) -> Option<u64> {
        if self.in_turnaround(rider_m) {
            return None;
        }
        let v0 = uturn_speed(uturn_radius(self.road_width_at(rider_m)));
        let brake = ((speed * speed - v0 * v0) / (2.0 * SLOW_DOWN_MPS2)).max(0.0);
        // Room to slow down and to steer to the centreline first.
        let at = rider_m + brake.max(UTURN_RUNUP_M);
        let k = self.step_index_at(at);
        for k in k..(k + 3).min(self.steps.len()) {
            let s = &self.steps[k];
            let Segment::Edge { edge, from_m, to_m } = s.seg else {
                return None; // turning already
            };
            let next_trim = self.steps.get(k + 1).map_or(0.0, |n| n.trim_start);
            // Metres into the raw edge line (after the rounded corner).
            let into = at.max(s.start_m) - s.start_m;
            let raw_s = (into - s.fillet_len).max(1.0) + s.trim_start;
            if raw_s > s.raw_len() - next_trim - 1.0 {
                continue;
            }
            let cut = if to_m >= from_m {
                from_m + raw_s
            } else {
                from_m - raw_s
            };
            self.steps.truncate(k + 1);
            self.cut_last(edge, from_m, cut);
            self.push_turnaround(TurnReason::Manual);
            self.rebuild_route();
            return Some(self.first_index + k as u64);
        }
        None
    }

    /// Replaces the steps from `index` on with `segments` (a server
    /// correction or extension). `index` must lie after the first kept
    /// step and at most at [`Self::next_index`].
    pub fn replace_from(&mut self, index: u64, segments: &[Segment]) -> Result<(), String> {
        if index <= self.first_index || index > self.next_index() {
            return Err(format!(
                "step {index} outside the known path ({}..={})",
                self.first_index + 1,
                self.next_index()
            ));
        }
        let keep = (index - self.first_index) as usize;
        // Steps we already have stay (we may also know more than the server).
        let same = segments
            .iter()
            .zip(&self.steps[keep..])
            .take_while(|(a, b)| same_segment(a, &b.seg))
            .count();
        if same == segments.len() {
            return Ok(());
        }
        self.steps.truncate(keep + same);
        let n = self.steps.len();
        self.steps[n - 1].rebuild(0.0);
        for &s in &segments[same..] {
            self.push(s)?;
        }
        self.rebuild_route();
        Ok(())
    }

    /// Drops steps that ended more than [`KEEP_BEHIND_M`] behind `rider_m`.
    pub fn trim_behind(&mut self, rider_m: f64) {
        let mut drop = 0;
        while drop + 1 < self.steps.len() && self.steps[drop].end_m() < rider_m - KEEP_BEHIND_M {
            drop += 1;
        }
        if drop > 0 {
            self.steps.drain(..drop);
            self.first_index += drop as u64;
            self.rebuild_route();
        }
    }

    fn step_index_at(&self, d: f64) -> usize {
        self.steps
            .partition_point(|s| s.start_m <= d)
            .saturating_sub(1)
    }

    fn step_at(&self, d: f64) -> &Step {
        &self.steps[self.step_index_at(d)]
    }

    /// The edge under path distance `d`: (edge, forward, metres from the
    /// edge's `from` vertex); `None` in a U-turn loop.
    pub fn locate(&self, d: f64) -> Option<(usize, bool, f64)> {
        let s = self.step_at(d);
        let Segment::Edge { edge, from_m, .. } = s.seg else {
            return None;
        };
        let along = (d - s.start_m - s.fillet_len).max(0.0) + s.trim_start;
        let fwd = s.forward();
        Some((edge, fwd, if fwd { from_m + along } else { from_m - along }))
    }

    /// The path distance of a point on an edge (as from [`Self::locate`])
    /// ridden in the same direction, near path distance `near_m` (within
    /// 500 m); `None` if this path does not pass there.
    pub fn distance_of(&self, edge: usize, forward: bool, offset: f64, near_m: f64) -> Option<f64> {
        self.steps
            .iter()
            .filter(|s| (s.start_m - near_m).abs() < 500.0 || (s.end_m() - near_m).abs() < 500.0)
            .find_map(|s| {
                let Segment::Edge {
                    edge: e,
                    from_m,
                    to_m,
                } = s.seg
                else {
                    return None;
                };
                if e != edge || s.forward() != forward {
                    return None;
                }
                let along = (offset - from_m).abs();
                let within =
                    (offset - from_m) * (to_m - from_m) >= 0.0 && along <= (to_m - from_m).abs();
                within.then_some(s.start_m + s.fillet_len + (along - s.trim_start))
            })
    }

    /// The next U-turn loop starting after `d`: (path distance, reason).
    pub fn turnaround_ahead(&self, d: f64) -> Option<(f64, TurnReason)> {
        self.steps[self.step_index_at(d)..]
            .iter()
            .find_map(|s| match s.seg {
                Segment::Turnaround(r) if s.end_m() > d => Some((s.start_m, r)),
                _ => None,
            })
    }

    /// Distance to the end of the dead end the rider is riding into
    /// (spec §2.2.2: "Dead end in 800 m"), from the graph's dead-end flags.
    pub fn dead_end_ahead(&self, d: f64) -> Option<f64> {
        if let Some((at, TurnReason::DeadEnd | TurnReason::Boundary)) = self.turnaround_ahead(d) {
            return Some(at - d);
        }
        let (edge, forward, offset) = self.locate(d)?;
        let e = self.net.edge(edge);
        (forward && e.dead_end_m.is_some()).then(|| e.dead_end_m.unwrap_or(0.0) - offset)
    }

    /// Whether `d` lies in a U-turn loop.
    pub fn in_turnaround(&self, d: f64) -> bool {
        let s = self.step_at(d);
        matches!(s.seg, Segment::Turnaround(_)) && d < s.end_m()
    }
}

impl Track for RidePath {
    fn sample(&self, distance: f64) -> RoutePoint {
        self.route.sample(distance - self.begin_m())
    }

    fn curvature(&self, distance: f64) -> f64 {
        self.route.curvature(distance - self.begin_m())
    }

    fn surface_at(&self, distance: f64) -> Surface {
        Surface::of_class(self.net.edge(self.step_at(distance).edge).class)
    }

    fn road_width_at(&self, distance: f64) -> f64 {
        self.net.edge(self.step_at(distance).edge).width_m
    }

    fn speed_limit(&self, distance: f64) -> Option<f64> {
        let k = self.step_index_at(distance);
        let mut limit: Option<f64> = None;
        for s in &self.steps[k..] {
            if s.start_m > distance + 250.0 {
                break;
            }
            let (v, at, span) = match s.seg {
                Segment::Turnaround(_) => (
                    uturn_speed(uturn_radius(self.net.edge(s.edge).width_m)),
                    s.start_m,
                    s.len_m,
                ),
                Segment::Edge { .. } if s.entry_angle_deg.abs() > SHARP_DEG => {
                    (SHARP_TURN_SPEED_MPS, s.start_m, s.fillet_len)
                }
                Segment::Edge { .. }
                    if s.entry_angle_deg.abs() > CORNER_DEG && s.trim_start > 0.0 =>
                {
                    // Radius of the rounded corner, at most CORNER_LATERAL_MPS2.
                    let half = (s.entry_angle_deg.abs().to_radians() / 2.0).tan();
                    let r = (s.trim_start / half.max(1e-3)).max(1.0);
                    let v = (CORNER_LATERAL_MPS2 * r).sqrt().max(SHARP_TURN_SPEED_MPS);
                    (v, s.start_m, s.fillet_len)
                }
                _ => continue,
            };
            let ahead = at - distance;
            if ahead < -span {
                continue; // passed
            }
            let allowed = (v * v + 2.0 * SLOW_DOWN_MPS2 * ahead.max(0.0)).sqrt();
            limit = Some(limit.map_or(allowed, |l: f64| l.min(allowed)));
        }
        limit
    }

    /// In a U-turn loop and on the run-up to it ([`UTURN_RUNUP_M`]): the
    /// rider is steered along the centreline, so the loop stays on the road.
    fn assisted(&self, distance: f64) -> bool {
        self.in_turnaround(distance)
            || self
                .turnaround_ahead(distance)
                .is_some_and(|(at, _)| at - distance <= UTURN_RUNUP_M)
    }

    fn extent(&self) -> Option<(f64, f64)> {
        Some((self.begin_m(), self.end_m()))
    }

    fn name_at(&self, distance: f64) -> String {
        let e = self.net.edge(self.step_at(distance).edge);
        e.name
            .clone()
            .unwrap_or_else(|| format!("{:?}", e.class).to_lowercase())
    }
}

/// What a route's next step means where the path ends.
enum Planned {
    /// Append it (the road just continues, or the route says so).
    Push,
    /// A U-turn.
    Turn,
    /// One of a junction's exits: chosen at the lock point.
    Junction,
    /// It does not start where the path ends.
    Mismatch,
}

/// Whether path step `seg` rides route step `planned`: the same edge in the
/// same direction (any part of it), or both U-turns.
fn rides(seg: &Segment, planned: &Segment) -> bool {
    match (seg, planned) {
        (
            Segment::Edge {
                edge: a,
                from_m: fa,
                to_m: ta,
            },
            Segment::Edge {
                edge: b,
                from_m: fb,
                to_m: tb,
            },
        ) => a == b && (ta >= fa) == (tb >= fb),
        (Segment::Turnaround(_), Segment::Turnaround(_)) => true,
        _ => false,
    }
}

/// How a vertex was reached: (edge, forward, previous vertex).
pub type Reached = (usize, bool, VertexId);

/// Distances (m) from vertex `from` along rideable directions (one-way
/// roads respected) up to `max_m`, and how each vertex was reached: (edge,
/// forward, previous vertex). `forbid` is a first move not to take (back
/// along the arrival).
pub fn directed_dijkstra(
    net: &RoadNetwork,
    from: VertexId,
    forbid: Option<(usize, bool)>,
    max_m: f64,
) -> (Vec<f64>, Vec<Option<Reached>>) {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;
    let n = net.graph().vertices.len();
    let mut dist = vec![f64::INFINITY; n];
    let mut came: Vec<Option<Reached>> = vec![None; n];
    let mut heap = BinaryHeap::new();
    dist[from.0 as usize] = 0.0;
    // Costs as integer millimetres (a total order for the heap).
    heap.push(Reverse((0u64, from.0)));
    while let Some(Reverse((c, v))) = heap.pop() {
        let c = c as f64 / 1000.0;
        if c > dist[v as usize] + 1e-6 || c > max_m {
            continue;
        }
        let vid = VertexId(v);
        for &e in net.edges_at(vid) {
            let edge = net.edge(e);
            for forward in [true, false] {
                if net.start_vertex(e, forward) != vid || !edge.oneway.allows(forward) {
                    continue;
                }
                if vid == from && forbid == Some((e, forward)) {
                    continue;
                }
                let o = net.end_vertex(e, forward);
                let nc = c + net.geometry(e).length();
                if nc < dist[o.0 as usize] {
                    dist[o.0 as usize] = nc;
                    came[o.0 as usize] = Some((e, forward, vid));
                    heap.push(Reverse(((nc * 1000.0) as u64, o.0)));
                }
            }
        }
    }
    (dist, came)
}

fn same_segment(a: &Segment, b: &Segment) -> bool {
    match (a, b) {
        (
            Segment::Edge {
                edge: e1,
                from_m: f1,
                to_m: t1,
            },
            Segment::Edge {
                edge: e2,
                from_m: f2,
                to_m: t2,
            },
        ) => e1 == e2 && (f1 - f2).abs() < 0.02 && (t1 - t2).abs() < 0.02,
        (Segment::Turnaround(a), Segment::Turnaround(b)) => a == b,
        _ => false,
    }
}

/// A step's raw geometry; `prev` gives the start of a turnaround.
fn make_step(
    net: &RoadNetwork,
    seg: Segment,
    prev: Option<&Step>,
    entry_angle_deg: f64,
    start_m: f64,
) -> Step {
    let (raw, edge) = match seg {
        Segment::Edge { edge, from_m, to_m } => (net.geometry(edge).slice(from_m, to_m), edge),
        Segment::Turnaround(_) => {
            let p = prev.expect("a turnaround follows a step");
            (
                bulb(
                    *p.raw.last().expect("raw has points"),
                    p.raw_end_dir(),
                    uturn_radius(net.edge(p.edge).width_m),
                ),
                p.edge,
            )
        }
    };
    let raw_cum = cumulative(&raw);
    let mut step = Step {
        seg,
        edge,
        start_m,
        fillet: Vec::new(),
        fillet_len: 0.0,
        raw,
        raw_cum,
        trim_start: 0.0,
        points: Vec::new(),
        len_m: 0.0,
        entry_angle_deg,
        route: RouteState::Finished,
        plan_index: None,
    };
    if let (Some(p), Segment::Edge { .. }) = (prev, seg)
        && matches!(p.seg, Segment::Edge { .. })
    {
        step.entry_angle_deg = signed_angle(p.raw_end_dir(), step.raw_start_dir()).to_degrees();
    }
    step
}

fn cumulative(pts: &[[f64; 3]]) -> Vec<f64> {
    let mut out = Vec::with_capacity(pts.len());
    let mut s = 0.0;
    for (i, p) in pts.iter().enumerate() {
        if i > 0 {
            s += ((p[0] - pts[i - 1][0]).powi(2) + (p[2] - pts[i - 1][2]).powi(2)).sqrt();
        }
        out.push(s);
    }
    out
}

fn polyline_len(pts: &[[f64; 3]]) -> f64 {
    *cumulative(pts).last().unwrap_or(&0.0)
}

fn point_at(pts: &[[f64; 3]], cum: &[f64], s: f64) -> [f64; 3] {
    let len = *cum.last().unwrap_or(&0.0);
    let s = s.clamp(0.0, len);
    if pts.len() < 2 {
        return pts[0];
    }
    let i = cum.partition_point(|&c| c <= s).clamp(1, pts.len() - 1);
    let seg = cum[i] - cum[i - 1];
    let t = if seg > 0.0 {
        (s - cum[i - 1]) / seg
    } else {
        0.0
    };
    std::array::from_fn(|k| pts[i - 1][k] + (pts[i][k] - pts[i - 1][k]) * t)
}

/// Points of `pts` from `a` to `b` metres (a ≤ b), ends included.
fn slice(pts: &[[f64; 3]], cum: &[f64], a: f64, b: f64) -> Vec<[f64; 3]> {
    let mut out = vec![point_at(pts, cum, a)];
    for (p, &c) in pts.iter().zip(cum) {
        if c > a + 1e-6 && c < b - 1e-6 {
            out.push(*p);
        }
    }
    if b > a + 1e-6 {
        out.push(point_at(pts, cum, b));
    }
    out
}

/// Quadratic Bézier from `p0` via control point `c` to `p2`, ~1 m apart.
fn bezier(p0: [f64; 3], c: [f64; 3], p2: [f64; 3]) -> Vec<[f64; 3]> {
    let span = ((p2[0] - p0[0]).powi(2) + (p2[2] - p0[2]).powi(2)).sqrt();
    let n = (span.ceil() as usize).clamp(4, 40);
    (0..=n)
        .map(|i| {
            let t = i as f64 / n as f64;
            let (a, b, d) = ((1.0 - t) * (1.0 - t), 2.0 * t * (1.0 - t), t * t);
            std::array::from_fn(|k| a * p0[k] + b * c[k] + d * p2[k])
        })
        .collect()
}

/// Rotates a horizontal direction by `a` rad (positive = right turn).
fn rotate(u: [f64; 2], a: f64) -> [f64; 2] {
    let (s, c) = a.sin_cos();
    [u[0] * c - u[1] * s, u[0] * s + u[1] * c]
}

/// The U-turn loop ("bulb") at `p` heading `dir`: 60° right, 300° left,
/// 60° right with radius `r`; it ends back at `p`, heading the other way.
fn bulb(p: [f64; 3], dir: [f64; 2], r: f64) -> Vec<[f64; 3]> {
    let mut pts = vec![p];
    let (mut pos, mut u) = ([p[0], p[2]], dir);
    let third = std::f64::consts::FRAC_PI_3;
    for (turn, sign) in [(third, 1.0), (5.0 * third, -1.0), (third, 1.0)] {
        let n = ((turn * r).ceil() as usize).max(2);
        let d = turn / n as f64 * sign;
        for _ in 0..n {
            let chord = 2.0 * r * (d.abs() / 2.0).sin();
            let m = rotate(u, d / 2.0);
            pos = [pos[0] + m[0] * chord, pos[1] + m[1] * chord];
            u = rotate(u, d);
            pts.push([pos[0], p[1], pos[1]]);
        }
    }
    // Close exactly (rounding).
    let last = pts.len() - 1;
    pts[last] = p;
    pts
}

fn placeholder_route() -> Route {
    placeholder_route_at([0.0; 3])
}

fn placeholder_route_at(p: [f64; 3]) -> Route {
    Route::from_points(
        "free-ride",
        "Free ride",
        vec![p, [p[0] + 1.0, p[1], p[2]]],
        false,
        5.0,
    )
    .expect("two distinct points")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{RoadGraph, VertexId};
    use crate::network::tests::{crossroads, graph};

    fn net(g: RoadGraph) -> Arc<RoadNetwork> {
        Arc::new(RoadNetwork::new(g))
    }

    /// A grid of 3 × 3 crossroads 200 m apart (vertices row by row,
    /// x = 200·col, z = 200·row), every neighbour connected.
    fn grid() -> RoadGraph {
        let mut v = Vec::new();
        for row in 0..3 {
            for col in 0..3 {
                v.push([f64::from(col) * 200.0, f64::from(row) * 200.0]);
            }
        }
        let mut e = Vec::new();
        for row in 0..3u32 {
            for col in 0..3u32 {
                let i = row * 3 + col;
                if col < 2 {
                    e.push((i, i + 1));
                }
                if row < 2 {
                    e.push((i, i + 3));
                }
            }
        }
        graph(&v, &e)
    }

    fn straight(_: &Junction) -> usize {
        usize::MAX
    }

    #[test]
    fn path_follows_an_edge_and_stops_at_the_next_junction() {
        let n = net(crossroads());
        // Start 50 m west of the centre on edge 0 (centre → west), riding
        // east (backwards along the edge).
        let mut p = RidePath::start(n.clone(), 0, 50.0, false, 0.0, 0);
        assert_eq!(p.end_m(), 50.0);
        assert_eq!(p.end_vertex(), Some(VertexId(4)));
        let (j, at) = p.next_junction().unwrap();
        assert_eq!(at, 50.0);
        assert_eq!(j.exits.len(), 3);
        let s = p.sample(10.0);
        assert!((s.position[0] + 40.0).abs() < 1e-9);
        assert_eq!(s.direction, [1.0, 0.0]);
        // Before the lock point nothing is decided.
        assert_eq!(p.extend(29.0, &mut |_| panic!("not yet")), None);
        // At the lock point (20 m before) the choice is taken.
        let mut asked = 0;
        let changed = p.extend(30.0, &mut |j| {
            asked += 1;
            j.exits.iter().position(|e| e.edge == 3).unwrap() // right, south
        });
        assert_eq!(asked, 1);
        assert_eq!(changed, Some(1));
        // The exit, then (a dead end) the U-turn loop and the way back.
        assert_eq!(p.next_index(), 4);
        // The south arm ends: a dead end with a U-turn and the way back.
        assert!(p.end_m() > 50.0 + 2.0 * 200.0 - 40.0, "{}", p.end_m());
        let (at_turn, reason) = p.turnaround_ahead(30.0).unwrap();
        assert_eq!(reason, TurnReason::DeadEnd);
        assert!(at_turn > 200.0 && at_turn < 250.0, "{at_turn}");
        // The rider ends up heading north again after the loop.
        let back = p.sample(at_turn + 40.0);
        assert!(back.direction[1] < -0.99, "{back:?}");
    }

    #[test]
    fn corners_are_rounded_and_continuous() {
        let n = net(crossroads());
        let mut p = RidePath::start(n, 0, 100.0, false, 0.0, 0);
        p.extend(85.0, &mut |j| {
            j.exits.iter().position(|e| e.edge == 2).unwrap() // left, north
        });
        // The path cuts the corner smoothly.
        let mut prev = p.sample(80.0);
        let mut max_turn: f64 = 0.0;
        let mut d = 80.0;
        while d < 125.0 {
            d += 0.25;
            let s = p.sample(d);
            let step = ((s.position[0] - prev.position[0]).powi(2)
                + (s.position[2] - prev.position[2]).powi(2))
            .sqrt();
            assert!(step < 0.3, "jump of {step} m at {d}");
            max_turn = max_turn.max(signed_angle(prev.direction, s.direction).abs());
            prev = s;
        }
        assert!(max_turn.to_degrees() < 6.0, "{}", max_turn.to_degrees());
        // The corner is never closer than ~2 m to the raw vertex.
        let closest = (0..200)
            .map(|i| {
                let s = p.sample(80.0 + f64::from(i) * 0.2).position;
                (s[0].powi(2) + s[2].powi(2)).sqrt()
            })
            .fold(f64::INFINITY, f64::min);
        assert!(closest > 1.5 && closest < 4.0, "{closest}");
        // Turning left: curvature negative in the corner.
        assert!(p.curvature(99.0) < 0.0);
        // A 90° corner (8 m radius) is taken at ~22 km/h.
        let v = p.speed_limit(95.0).unwrap();
        assert!(
            (v - (CORNER_LATERAL_MPS2 * 8.0f64).sqrt()).abs() < 0.3,
            "{v}"
        );
        assert!(p.speed_limit(60.0).unwrap() > v + 3.0);
    }

    #[test]
    fn plain_continuations_are_followed_up_to_the_horizon() {
        // A long line of 100 m edges without junctions.
        let v: Vec<[f64; 2]> = (0..40).map(|i| [f64::from(i) * 100.0, 0.0]).collect();
        let e: Vec<(u32, u32)> = (0..39).map(|i| (i, i + 1)).collect();
        let mut p = RidePath::start(net(graph(&v, &e)), 0, 0.0, true, 0.0, 0);
        assert!(p.needs_extend(0.0, 0.0));
        assert_eq!(p.extend(0.0, &mut straight), Some(1));
        assert!(!p.needs_extend(0.0, 0.0));
        assert!(p.end_m() >= HORIZON_M && p.end_m() < HORIZON_M + 100.1);
        assert!(p.next_junction().is_none());
        // Further along, more is added; old steps are dropped.
        p.extend(2500.0, &mut straight);
        assert!(p.needs_trim(2500.0));
        p.trim_behind(2500.0);
        assert!(!p.needs_trim(2500.0));
        assert!(p.begin_m() >= 2500.0 - KEEP_BEHIND_M - 100.0);
        assert!(p.first_index() > 10);
        let s = p.sample(2600.0);
        assert!((s.position[0] - 2600.0).abs() < 1e-6);
    }

    #[test]
    fn manual_u_turn_slows_turns_and_rides_back() {
        let v: Vec<[f64; 2]> = (0..4).map(|i| [f64::from(i) * 300.0, 0.0]).collect();
        let mut p = RidePath::start(
            net(graph(&v, &[(0, 1), (1, 2), (2, 3)])),
            1,
            0.0,
            true,
            0.0,
            0,
        );
        p.extend(0.0, &mut straight);
        let changed = p.u_turn(100.0, 10.0).unwrap();
        assert_eq!(changed, 0);
        let (at, reason) = p.turnaround_ahead(100.0).unwrap();
        assert_eq!(reason, TurnReason::Manual);
        // Braking from 36 km/h to ~10 km/h at 2 m/s² takes ~23 m.
        assert!((at - 123.0).abs() < 1.5, "{at}");
        // Speed limit: falls towards the loop, 10 km/h in it.
        let far = p.speed_limit(100.0).unwrap();
        assert!((far - 10.0).abs() < 0.2, "{far}");
        assert_eq!(
            p.speed_limit(at + 3.0),
            Some(uturn_speed(uturn_radius(5.0)))
        );
        // Steered on the run-up and in the loop.
        assert!(p.assisted(at + 3.0) && p.assisted(at - 3.0));
        assert!(!p.assisted(at - UTURN_RUNUP_M - 2.0));
        // After the loop: back west, on the other side of the centreline.
        let after = p.sample(at + 35.0);
        assert!(after.direction[0] < -0.99, "{after:?}");
        assert!((after.position[2]).abs() < 1e-6);
        // The way back goes through vertex 1 and on to the west end.
        p.extend(at + 40.0, &mut straight);
        let (t2, r2) = p.turnaround_ahead(at + 40.0).unwrap();
        assert_eq!(r2, TurnReason::DeadEnd);
        assert!(p.sample(t2).position[0] < 20.0);
        // No U-turn inside a loop.
        assert_eq!(p.u_turn(at + 2.0, 2.8), None);
    }

    #[test]
    fn u_turn_loop_closes_and_is_rideable() {
        let b = bulb([10.0, 5.0, 20.0], [1.0, 0.0], UTURN_RADIUS_M);
        assert_eq!(*b.last().unwrap(), [10.0, 5.0, 20.0]);
        let len = polyline_len(&b);
        assert!(
            (len - 7.0 * std::f64::consts::PI / 3.0 * UTURN_RADIUS_M).abs() < 0.3,
            "{len}"
        );
        // It stays within one radius of the road axis sideways.
        assert!(
            b.iter()
                .all(|p| (p[2] - 20.0).abs() <= UTURN_RADIUS_M + 1e-9)
        );
    }

    #[test]
    fn segments_round_trip_and_server_corrections() {
        let n = net(grid());
        // Start at the top-left corner heading east along the top row.
        let mut server = RidePath::start(n.clone(), 0, 0.0, true, 0.0, 0);
        let mut client = server.clone();
        // The server takes the default (straight) at vertex 1; the client
        // predicted a right turn.
        let right = |j: &Junction| {
            j.exits
                .iter()
                .position(|e| e.class == crate::network::TurnClass::Right)
                .unwrap()
        };
        assert_eq!(server.extend(185.0, &mut |j| j.default), Some(1));
        assert_eq!(client.extend(185.0, &mut |j| right(j)), Some(1));
        assert!((server.sample(300.0).position[2] - client.sample(300.0).position[2]).abs() > 50.0);
        // The correction: the server's steps from index 1.
        client.replace_from(1, &server.segments_from(1)).unwrap();
        for d in [150.0, 210.0, 300.0, 390.0] {
            assert_eq!(client.sample(d), server.sample(d));
        }
        // A path rebuilt from its segments is the same.
        let copy = RidePath::from_segments(n, &server.segments_from(0), 0.0, server.first_index())
            .unwrap();
        assert_eq!(copy.end_m(), server.end_m());
        assert_eq!(copy.sample(222.0), server.sample(222.0));
        // Same segments: nothing changes; out of range: refused.
        assert!(client.replace_from(1, &server.segments_from(1)).is_ok());
        assert!(client.replace_from(0, &[]).is_err());
        assert!(client.replace_from(9, &[]).is_err());
    }

    #[test]
    fn locate_and_map_positions_between_paths() {
        let n = net(grid());
        let mut a = RidePath::start(n.clone(), 0, 20.0, true, 0.0, 0);
        a.extend(0.0, &mut |j| j.default);
        let (edge, fwd, off) = a.locate(100.0).unwrap();
        assert_eq!((edge, fwd), (0, true));
        assert!((off - 120.0).abs() < 1e-9);
        // Another rider started earlier on the same edge: 100 m on our path
        // is 220 m on theirs... or the reverse mapping.
        let b = RidePath::start(n, 0, 0.0, true, 1000.0, 0);
        let d = b.distance_of(edge, fwd, off, 1100.0).unwrap();
        assert!((d - 1120.0).abs() < 1e-9, "{d}");
        assert_eq!(b.distance_of(edge, !fwd, off, 1100.0), None);
    }

    #[test]
    fn sharp_turns_slow_the_rider_down() {
        // From the west arm into a road doubling back at 150° to the north-west.
        let g = graph(
            &[[-200.0, 0.0], [0.0, 0.0], [-173.2, -100.0]],
            &[(0, 1), (1, 2), (1, 0)],
        );
        // Vertex 1 has edges 0 (west, in), 1 (sharp back-left) and 2 (a
        // parallel road back west): a junction.
        let mut p = RidePath::start(net(g), 0, 0.0, true, 0.0, 0);
        p.extend(185.0, &mut |j| {
            j.exits
                .iter()
                .position(|e| e.class == crate::network::TurnClass::SharpLeft)
                .unwrap()
        });
        let v = p.speed_limit(170.0).unwrap();
        assert!(v < 10.0 && v > SHARP_TURN_SPEED_MPS, "{v}");
        assert!((p.speed_limit(196.0).unwrap() - SHARP_TURN_SPEED_MPS).abs() < 1.0);
        // Further on only the dead end's U-turn is ahead.
        assert!(p.speed_limit(300.0).unwrap() > 10.0);
        // The lock point moves out with the speed.
        assert_eq!(lock_distance(3.0), LOCK_M);
        assert!((lock_distance(18.0) - LOCK_M - 76.6).abs() < 0.5);
    }

    /// Edge index of the grid edge from vertex `a` to `b` and its direction.
    fn grid_edge(n: &RoadNetwork, a: u32, b: u32) -> (usize, bool) {
        (0..n.graph().edges.len())
            .find_map(|i| {
                let e = n.edge(i);
                match (e.from.0, e.to.0) {
                    (f, t) if (f, t) == (a, b) => Some((i, true)),
                    (f, t) if (f, t) == (b, a) => Some((i, false)),
                    _ => None,
                }
            })
            .unwrap()
    }

    fn whole(n: &RoadNetwork, a: u32, b: u32) -> Segment {
        let (e, f) = grid_edge(n, a, b);
        Segment::whole(n, e, f)
    }

    /// On the grid: 20 m into 0→1, right (south) to 4, left (east) to 5,
    /// right to 8, then west towards 7, ending 100 m along.
    fn zigzag(n: &Arc<RoadNetwork>) -> Arc<RoutePlan> {
        let (e01, _) = grid_edge(n, 0, 1);
        let (e78, _) = grid_edge(n, 7, 8);
        let steps = vec![
            Segment::Edge {
                edge: e01,
                from_m: 20.0,
                to_m: 200.0,
            },
            whole(n, 1, 4),
            whole(n, 4, 5),
            whole(n, 5, 8),
            Segment::Edge {
                edge: e78,
                from_m: 200.0,
                to_m: 100.0,
            },
        ];
        Arc::new(RoutePlan::new(n, "zigzag", steps, false).unwrap())
    }

    /// Rides `p` from `from` to `to` m at standstill lock points, taking
    /// `choose`'s exits.
    fn ride(p: &mut RidePath, from: f64, to: f64, choose: &mut dyn FnMut(&Junction) -> usize) {
        let mut d = from;
        while d <= to {
            p.extend(d, choose);
            d += 5.0;
        }
    }

    #[test]
    fn a_route_is_followed_and_shows_its_exit() {
        let n = net(grid());
        let plan = zigzag(&n);
        assert_eq!(plan.length_m(), 180.0 + 600.0 + 100.0);
        let mut p = RidePath::on_route(n.clone(), plan.clone(), 0.0, 0);
        assert_eq!(p.route_state(), Some(RouteState::On { next: 1, lap: 0 }));
        p.extend(0.0, &mut |j| j.default);
        // Vertex 1: the route's exit (south) is the default.
        let (j, at) = p.next_junction().unwrap();
        assert_eq!(at, 180.0);
        let south = j.route.unwrap();
        assert_eq!(j.default, south);
        assert_eq!(
            (j.exits[south].edge, j.exits[south].forward),
            grid_edge(&n, 1, 4)
        );
        ride(&mut p, 0.0, 1100.0, &mut |j| j.default);
        // All the way: south, east, south, west – then free ride.
        assert!(p.sample(300.0).direction[1] > 0.99, "south");
        assert!(p.sample(500.0).direction[0] > 0.99, "east");
        assert!(p.sample(820.0).direction[0] < -0.99, "west");
        let pr = p.route_progress(100.0).unwrap();
        assert!(pr.on_route && !pr.finished);
        assert!((pr.done_m - 100.0).abs() < 1e-6, "{pr:?}");
        // Rounded corners make the path a little shorter than the route.
        let pr = p.route_progress(500.0).unwrap();
        assert!((pr.done_m - 500.0).abs() < 10.0, "{pr:?}");
        assert_eq!(p.route_state(), Some(RouteState::Finished));
        assert!(p.route_progress(1050.0).unwrap().finished);
        assert!(!p.route_progress(1050.0).unwrap().on_route);
    }

    #[test]
    fn another_exit_leaves_the_route_and_rejoin_finds_it_again() {
        let n = net(grid());
        let plan = zigzag(&n);
        let mut p = RidePath::on_route(n.clone(), plan, 0.0, 0);
        // Straight on at vertex 1 (the route turns right).
        let straight = |j: &Junction| {
            j.exits
                .iter()
                .position(|e| e.class == crate::network::TurnClass::Straight)
                .unwrap()
        };
        p.extend(0.0, &mut |j| j.default);
        p.extend(165.0, &mut |j| straight(j));
        assert_eq!(
            p.route_state(),
            Some(RouteState::Off { left_at: 1, lap: 0 })
        );
        assert!(p.on_route_at(100.0) && !p.on_route_at(250.0));
        assert!(!p.route_progress(250.0).unwrap().on_route);
        // Off the route, junctions show no route exit (vertex 5 here).
        assert_eq!(p.next_junction().unwrap().0.route, None);
        // Back to the route: via 2 → 5, joining it where it goes 5 → 8.
        let changed = p.rejoin_route(170.0, 0.0).unwrap();
        assert_eq!(changed, 2);
        assert_eq!(p.route_state(), Some(RouteState::On { next: 4, lap: 0 }));
        assert!(!p.on_route_at(480.0), "the detour 2 → 5");
        assert!(p.on_route_at(650.0), "on 5 → 8");
        assert!(p.rejoin_route(170.0, 0.0).unwrap_err().contains("already"));
        // The same steps sent to a client give the same route state.
        let copy = RidePath::from_segments(n.clone(), &p.segments_from(0), 0.0, 0)
            .unwrap()
            .with_plan(p.plan().cloned());
        assert_eq!(copy.route_state(), p.route_state());
        // A path without a route cannot rejoin one.
        let mut free = RidePath::start(n, 0, 0.0, true, 0.0, 0);
        assert!(free.rejoin_route(0.0, 0.0).is_err());
        assert_eq!(free.route_state(), None);
    }

    #[test]
    fn route_u_turns_and_laps() {
        // Out and back on a line: 150 m along edge 1, turn, back to 0.
        let v: Vec<[f64; 2]> = (0..4).map(|i| [f64::from(i) * 300.0, 0.0]).collect();
        let n = net(graph(&v, &[(0, 1), (1, 2), (2, 3)]));
        let steps = vec![
            Segment::Edge {
                edge: 1,
                from_m: 0.0,
                to_m: 150.0,
            },
            Segment::Turnaround(TurnReason::Manual),
            Segment::Edge {
                edge: 1,
                from_m: 150.0,
                to_m: 0.0,
            },
            Segment::whole(&n, 0, false),
        ];
        let plan = Arc::new(RoutePlan::new(&n, "out-back", steps, false).unwrap());
        let mut p = RidePath::on_route(n.clone(), plan, 0.0, 0);
        p.extend(0.0, &mut straight);
        let (at, reason) = p.turnaround_ahead(0.0).unwrap();
        assert_eq!(reason, TurnReason::Manual);
        assert!((at - 150.0).abs() < 1e-6, "{at}");
        assert!(p.sample(at + 40.0).direction[0] < -0.99);
        assert_eq!(p.route_state(), Some(RouteState::Finished));
        // A loop round a block of the grid, ridden twice.
        let g = net(grid());
        let steps = vec![
            whole(&g, 0, 1),
            whole(&g, 1, 4),
            whole(&g, 4, 3),
            whole(&g, 3, 0),
        ];
        let plan = RoutePlan::new(&g, "block", steps, true).unwrap();
        assert!(plan.closes(&g));
        let mut p = RidePath::on_route(g.clone(), Arc::new(plan), 0.0, 0);
        ride(&mut p, 0.0, 1300.0, &mut |j| j.default);
        let pr = p.route_progress(900.0).unwrap();
        assert_eq!(pr.lap, 1, "{pr:?}");
        assert!(pr.on_route);
        assert!((pr.done_m - 100.0).abs() < 25.0, "{pr:?}");
        assert!(matches!(p.route_state(), Some(RouteState::On { .. })));
    }

    #[test]
    fn region_border_turns_the_rider_round() {
        let mut g = graph(&[[0.0, 0.0], [300.0, 0.0]], &[(0, 1)]);
        g.boundary_vertices.insert(VertexId(1));
        let mut p = RidePath::start(net(g), 0, 0.0, true, 0.0, 0);
        p.extend(0.0, &mut straight);
        let (_, reason) = p.turnaround_ahead(0.0).unwrap();
        assert_eq!(reason, TurnReason::Boundary);
        assert!(p.dead_end_ahead(0.0).unwrap() > 250.0);
    }

    #[test]
    fn preview_extends_a_copy_through_the_junctions_ahead() {
        let n = net(grid());
        // Riding 0 → 1 (east) from 20 m along; the path ends at vertex 1.
        let (e01, f01) = grid_edge(&n, 0, 1);
        let p = RidePath::start(n.clone(), e01, 20.0, f01, 0.0, 0);
        let (j, at) = p.next_junction().unwrap();
        assert_eq!(at, 180.0);
        // Far enough already: nothing to preview.
        assert!(p.preview(0.0, 150.0, None).is_none());
        // 1 km ahead: through several junctions with the default exits.
        let ahead = p.preview(0.0, 1000.0, None).expect("extended");
        assert!(ahead.end_m() >= 1000.0, "{}", ahead.end_m());
        assert_eq!(ahead.extent(), Some((0.0, ahead.end_m())));
        // The original is unchanged (display only).
        assert_eq!((p.end_m(), p.next_index()), (180.0, 1));
        // The same geometry where both are known.
        assert_eq!(ahead.sample(100.0), p.sample(100.0));
        // The rider's choice at the next junction is taken there.
        let south = j
            .exits
            .iter()
            .position(|e| (e.edge, e.forward) == grid_edge(&n, 1, 4))
            .unwrap();
        let turned = p.preview(0.0, 1000.0, Some(south)).unwrap();
        assert_eq!(turned.segments_from(1)[0], whole(&n, 1, 4));
        let s = turned.sample(300.0);
        assert!(s.direction[1] > 0.99, "heading south: {s:?}");
    }
}
