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

use std::sync::Arc;

use crate::graph::VertexId;
use crate::network::{ARRIVAL_HEADING_M, Junction, RoadNetwork, SHARP_DEG, signed_angle, unit};
use crate::route::{Route, RoutePoint};
use crate::surface::Surface;
use crate::track::Track;

/// The exit choice locks this far before a junction (spec §2.2.1).
pub const LOCK_M: f64 = 20.0;
/// The path is extended through plain road continuations this far ahead.
pub const HORIZON_M: f64 = 2000.0;
/// Steps that ended more than this far behind the rider are dropped.
pub const KEEP_BEHIND_M: f64 = 600.0;
/// Radius of the rounded corner at junctions (m).
pub const FILLET_RADIUS_M: f64 = 8.0;
/// A rounded corner starts at most this far before the junction (m); less
/// than [`LOCK_M`], so it is never under the rider when the exit locks.
pub const MAX_FILLET_TRIM_M: f64 = 10.0;
/// Radius of the U-turn loop (m).
pub const UTURN_RADIUS_M: f64 = 4.0;
/// Speed through the U-turn loop (m/s, ~10 km/h, spec §2.2.2).
pub const UTURN_SPEED_MPS: f64 = 2.8;
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

    fn arrival(&self) -> Option<(usize, bool)> {
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
        self.net
            .junction(v, Some((edge, forward)), heading)
            .map(|j| (j, self.end_m()))
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
                    self.push(Segment::whole(&self.net, e.edge, e.forward))
                        .expect("valid edge");
                }
                _ => break,
            }
        }
        if changed.is_some() {
            self.rebuild_route();
        }
        changed
    }

    /// U-turn where the path ends at a road end: the last step is cut so
    /// the loop fits before the end (if the rider is not there yet), then
    /// the loop and the way back. Returns the first changed step index.
    fn turn_at_end(&mut self, rider_m: f64, reason: TurnReason) -> u64 {
        let n = self.steps.len();
        let last = &self.steps[n - 1];
        let mut first = self.next_index();
        let reach = (1.0 + 3f64.sqrt()) * UTURN_RADIUS_M + 1.0;
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
        self.end_vertex().is_some() && self.end_m() - rider_m < HORIZON_M
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
        let brake =
            ((speed * speed - UTURN_SPEED_MPS * UTURN_SPEED_MPS) / (2.0 * SLOW_DOWN_MPS2)).max(0.0);
        let at = rider_m + brake.max(6.0);
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
                Segment::Turnaround(_) => (UTURN_SPEED_MPS, s.start_m, s.len_m),
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

    fn assisted(&self, distance: f64) -> bool {
        self.in_turnaround(distance)
    }

    fn name_at(&self, distance: f64) -> String {
        let e = self.net.edge(self.step_at(distance).edge);
        e.name
            .clone()
            .unwrap_or_else(|| format!("{:?}", e.class).to_lowercase())
    }
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
                    UTURN_RADIUS_M,
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
        assert_eq!(p.speed_limit(at + 3.0), Some(UTURN_SPEED_MPS));
        assert!(p.assisted(at + 3.0) && !p.assisted(at - 3.0));
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
}
