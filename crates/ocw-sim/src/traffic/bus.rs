//! Buses on their lines (spec 14 Phase 18, 18B *Vehicles*: "buses on
//! their OSM lines and stopping at their stops").
//!
//! - **Lines** ([`lines_of`]): each `route=bus` relation of the package's
//!   `TRFC` ([`ocw_world::traffic::BusRoute`]) becomes one or more
//!   [`BusLine`]s – its ways in the relation's order as (edge, forward)
//!   steps on the network, gaps between them (ways off the package, a way
//!   missing from the graph) joined by the quickest legal route for a bus
//!   up to [`MAX_GAP_M`]; where no such route exists the line is cut into
//!   pieces. Each direction of a line is its own relation in OSM (PTv2),
//!   so both directions run where the package has both.
//! - **Stops** ([`LineStop`]): the relation's stops in order, each on the
//!   first step after the previous one that runs on its edge (else the
//!   nearest point of the line within [`STOP_REACH_M`]); a platform and
//!   its stop position mapped both are one stop. The bus stops with its
//!   front [`FRONT_PAST_M`] past the platform.
//! - **Timetable** ([`headway_s`], [`departures`]): a line's base headway
//!   (10, 15, 20 or 30 minutes) and its phase come from the session's
//!   seed and the relation; the hour and the day type change it – the
//!   base on weekdays from 06:00 to 20:00 (two thirds of it in the peaks
//!   07 – 09 and 16 – 19), a third more on Saturdays, double (at least
//!   30 minutes) in the evenings and on Sundays and holidays, hourly at
//!   the edges of the day and none at night (from 23:00, on Sundays
//!   until 07:00). A departure leaves the line's start at its second of
//!   the day and is *scheduled* to be [`SCHEDULE_MPS`] × the time since
//!   along the line: the core spawns it where its scheduled position
//!   enters a rider's bubble (or, at the first fill, where it is
//!   scheduled now), so every rider meets the same buses at the same
//!   places from the seed and the clock alone.
//! - **At a stop** the bus brakes to the platform, pulls to the kerb (a
//!   lateral shift towards the kerb, T4's `Shift` in the plans), stands
//!   with its doors open for its dwell ([`dwell_s`]: 10 – 30 s from the
//!   seed, the departure and the stop), closes them, indicates and pulls
//!   out. Every phase is a plan change (Q3 (a)): the clients see the
//!   stop, the doors and the pull-out at the same ticks as the server.

use ocw_world::RoadNetwork;
use ocw_world::traffic::TrafficData;

use super::demand::{DayType, DemandTime};
use super::routing::{self, key, unit_f};
use super::rules::{EdgeRule, VehicleClass};

/// A line's number on the wire (`TrafficSpawn.line`): the package's bus
/// route (its index in `TRFC`'s routes) + 1, times 16, plus the piece.
pub type LineId = u32;

/// The line id of piece `piece` of route `route`.
pub fn line_id(route: usize, piece: usize) -> LineId {
    ((route as u32 + 1) << 4) | (piece as u32 & 0xF)
}

/// The package's route index of a line id (`None`: no line).
pub fn route_of(id: LineId) -> Option<usize> {
    (id >= 16).then(|| (id >> 4) as usize - 1)
}

/// Gaps between a relation's ways are joined by a route at most this
/// long (m); longer ones cut the line.
pub const MAX_GAP_M: f64 = 1500.0;
/// A stop not on one of the line's edges is put on the nearest point of
/// the line within this (m).
pub const STOP_REACH_M: f64 = 30.0;
/// Two stops of a line closer than this are one (a platform and its stop
/// position, both members of the relation; m).
pub const SAME_STOP_M: f64 = 40.0;
/// The bus's front stands this far past the platform (m): its doors
/// beside it.
pub const FRONT_PAST_M: f64 = 4.0;
/// The scheduled speed along a line, stops included (m/s; 21.6 km/h,
/// a town bus's average).
pub const SCHEDULE_MPS: f64 = 6.0;
/// The bus starts braking for its stop and pulling to the kerb this far
/// before it (m).
pub const PULL_IN_M: f64 = 45.0;
/// The doors close this long before the bus pulls out (s); it indicates
/// meanwhile.
pub const DOORS_CLOSE_S: f64 = 2.0;
/// The bus pulls to the kerb at this rate and back out at half of it (m/s).
pub const PULL_RATE: f32 = 0.5;

/// A stop of a line.
#[derive(Debug, Clone, PartialEq)]
pub struct LineStop {
    /// Index into `TrafficData::stops`.
    pub stop: u32,
    /// Where the bus's front stands: metres along the line.
    pub at: f64,
}

/// One piece of a bus route on the network.
#[derive(Debug, Clone, PartialEq)]
pub struct BusLine {
    pub id: LineId,
    pub relation: i64,
    /// The line's number (`ref`) and where it goes (its last stop, or the
    /// part of its name after "=>").
    pub reference: String,
    pub destination: String,
    /// (edge index, forward) in driving order.
    pub steps: Vec<(usize, bool)>,
    /// Metres along the line at the start of each step, and its length.
    pub start: Vec<f64>,
    pub length: f64,
    pub stops: Vec<LineStop>,
    /// The base headway (minutes) and the phase of the departures (s).
    pub base_min: u32,
    pub phase_s: u32,
    /// Where this piece starts along its whole route (m): the pieces of a
    /// cut route run one timetable, a bus going on from piece to piece.
    pub offset_m: f64,
    /// A night line (`ref` N1, N2, …): only in the nights to Saturday and
    /// Sunday.
    pub night: bool,
}

impl BusLine {
    /// (step index, metres along the step) of line distance `x`.
    pub fn locate(&self, x: f64) -> (usize, f64) {
        let k = self.start.partition_point(|&s| s <= x).saturating_sub(1);
        (k, (x - self.start[k]).max(0.0))
    }

    /// The text a bus of this line shows: "12 Vaduz Post".
    pub fn sign(&self) -> String {
        match (self.reference.is_empty(), self.destination.is_empty()) {
            (false, false) => format!("{} {}", self.reference, self.destination),
            (false, true) => self.reference.clone(),
            (true, _) => self.destination.clone(),
        }
    }
}

/// What [`lines_of`] made of the package's routes (tests, the bench).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LineCounts {
    pub routes: usize,
    pub pieces: usize,
    pub stops: usize,
    /// Stops of the relations that are on none of the line's roads.
    pub stops_missed: usize,
    /// Gaps joined by a route, and cuts.
    pub joined: usize,
    pub cuts: usize,
    /// Routes left out: fewer than two stops in the package (a coach
    /// passing through, a fragment).
    pub idle: usize,
}

/// Where along step `step` of edge `e` driven `forward` the point `p` is
/// nearest: (metres along the driving direction, distance).
fn project(net: &RoadNetwork, e: usize, forward: bool, p: [f64; 2]) -> (f64, f64) {
    let g = net.geometry(e);
    let len = g.length();
    let mut best = (0.0, f64::INFINITY);
    for (k, w) in g.points.windows(2).enumerate() {
        let (a, b) = ([w[0][0], w[0][2]], [w[1][0], w[1][2]]);
        let ab = [b[0] - a[0], b[1] - a[1]];
        let l2 = ab[0] * ab[0] + ab[1] * ab[1];
        let t = if l2 > 0.0 {
            (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / l2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let d = (a[0] + ab[0] * t - p[0]).hypot(a[1] + ab[1] * t - p[1]);
        if d < best.1 {
            let s = g.cumulative[k] + (g.cumulative[k + 1] - g.cumulative[k]) * t;
            best = (if forward { s } else { len - s }, d);
        }
    }
    best
}

/// The bus lines of `net`'s package (`TRFC`): see the module. `rules` are
/// the core's edge rules (a step needs a lane in its direction); `seed`
/// picks each line's headway and phase.
pub fn lines_of(net: &RoadNetwork, rules: &[EdgeRule], seed: u64) -> (Vec<BusLine>, LineCounts) {
    let mut c = LineCounts::default();
    let Some(data) = net.road_rules().data else {
        return (Vec::new(), c);
    };
    // The relations of each line number: a line's variants (branches,
    // short runs) share its headway, each direction's together.
    let mut variants: std::collections::BTreeMap<&str, u32> = Default::default();
    for r in &data.bus_routes {
        *variants.entry(r.reference.trim()).or_default() += 1;
    }
    let mut out = Vec::new();
    for (ri, route) in data.bus_routes.iter().enumerate() {
        c.routes += 1;
        let reference = route.reference.trim();
        let h = key(
            seed,
            ocw_world::graph::fnv1a(reference.as_bytes()),
            if reference.is_empty() {
                route.osm_relation as u64
            } else {
                0
            },
            0xB05,
        );
        let n = if reference.is_empty() {
            1
        } else {
            variants[reference].div_ceil(2).max(1)
        };
        let base_min = ([10, 15, 15, 20, 20, 30, 30, 30][(h % 8) as usize] * n).min(60);
        let phase_s =
            (key(seed, route.osm_relation as u64, 1, 0xB06) % u64::from(base_min * 60)) as u32;
        let night = reference.len() >= 2
            && reference.starts_with('N')
            && reference[1..].starts_with(|ch: char| ch.is_ascii_digit());
        let pieces = pieces_of(net, rules, route, &mut c);
        let mut kept = 0;
        let mut offset = 0.0;
        let mut lines: Vec<BusLine> = Vec::new();
        let mut placed = std::collections::BTreeSet::new();
        for steps in pieces {
            let mut start = Vec::with_capacity(steps.len());
            let mut x = 0.0;
            for &(e, _) in &steps {
                start.push(x);
                x += net.geometry(e).length();
            }
            let piece_at = offset;
            offset += x;
            if x < 200.0 || kept >= 16 {
                continue;
            }
            let mut line = BusLine {
                id: line_id(ri, kept),
                relation: route.osm_relation,
                reference: route.reference.clone(),
                destination: destination(data, route),
                steps,
                start,
                length: x,
                stops: Vec::new(),
                base_min,
                phase_s,
                offset_m: piece_at,
                night,
            };
            line.stops = stops_of(net, data, route, &line, &mut c, &mut placed);
            kept += 1;
            lines.push(line);
        }
        // A stop on none of the pieces (off the roads, out of order).
        c.stops_missed += route.stops.iter().filter(|s| !placed.contains(s)).count();
        if lines.iter().map(|l| l.stops.len()).sum::<usize>() < 2 {
            c.idle += 1;
            continue;
        }
        c.pieces += lines.len();
        out.extend(lines);
    }
    (out, c)
}

/// The route's last stop's name, or what its name says after "=>".
fn destination(data: &TrafficData, route: &ocw_world::traffic::BusRoute) -> String {
    if let Some((_, to)) = route.name.split_once("=>") {
        let to = to.trim();
        if !to.is_empty() {
            return to.to_owned();
        }
    }
    route
        .stops
        .iter()
        .rev()
        .filter_map(|&i| data.stops.get(i as usize))
        .map(|s| s.name.trim())
        .find(|n| !n.is_empty())
        .unwrap_or_default()
        .to_owned()
}

/// A route's ways as pieces of connected steps.
fn pieces_of(
    net: &RoadNetwork,
    rules: &[EdgeRule],
    route: &ocw_world::traffic::BusRoute,
    c: &mut LineCounts,
) -> Vec<Vec<(usize, bool)>> {
    let mut pieces: Vec<Vec<(usize, bool)>> = Vec::new();
    let mut cur: Vec<(usize, bool)> = Vec::new();
    for &(id, forward) in &route.legs {
        let Some(e) = net.edge_index(id) else {
            continue;
        };
        if rules[e].lanes_in(forward) == 0 {
            continue;
        }
        let Some(&last) = cur.last() else {
            cur.push((e, forward));
            continue;
        };
        if last == (e, forward) {
            continue;
        }
        let end = net.end_vertex(last.0, last.1);
        let begin = net.start_vertex(e, forward);
        if end == begin {
            cur.push((e, forward));
            continue;
        }
        // A gap: the quickest legal way from the end of the last step.
        let joined =
            routing::shortest_route(net, rules, VehicleClass::Bus, last, begin).filter(|r| {
                r.iter()
                    .skip(1)
                    .map(|&(x, _)| net.geometry(x).length())
                    .sum::<f64>()
                    <= MAX_GAP_M
            });
        match joined {
            Some(r) => {
                c.joined += 1;
                cur.extend(r.into_iter().skip(1));
                cur.push((e, forward));
            }
            None => {
                c.cuts += 1;
                pieces.push(std::mem::take(&mut cur));
                cur.push((e, forward));
            }
        }
    }
    if !cur.is_empty() {
        pieces.push(cur);
    }
    pieces.retain(|p| p.len() >= 2 || p.iter().any(|&(e, _)| net.geometry(e).length() > 200.0));
    pieces
}

/// A line's stops in order (see the module).
fn stops_of(
    net: &RoadNetwork,
    data: &TrafficData,
    route: &ocw_world::traffic::BusRoute,
    line: &BusLine,
    c: &mut LineCounts,
    placed: &mut std::collections::BTreeSet<u32>,
) -> Vec<LineStop> {
    let mut out: Vec<LineStop> = Vec::new();
    // The line distance reached so far: stops come in order.
    let mut from_x = 0.0f64;
    for &si in &route.stops {
        let Some(stop) = data.stops.get(si as usize) else {
            continue;
        };
        let p = [f64::from(stop.pos[0]), f64::from(stop.pos[1])];
        let edge = stop.edge.and_then(|id| net.edge_index(id));
        let mut found: Option<f64> = None;
        let k0 = line.locate(from_x).0;
        // On its own edge first, else the nearest point within reach.
        if let Some(e) = edge {
            for k in k0..line.steps.len() {
                let (se, fwd) = line.steps[k];
                if se != e {
                    continue;
                }
                let len = net.geometry(e).length();
                let along = f64::from(stop.along_m).clamp(0.0, len);
                let s = if fwd { along } else { len - along };
                let x = line.start[k] + s;
                if x + 1.0 >= from_x {
                    found = Some(x);
                    break;
                }
            }
        }
        if found.is_none() {
            for k in k0..line.steps.len() {
                let (e, fwd) = line.steps[k];
                let (s, d) = project(net, e, fwd, p);
                let x = line.start[k] + s;
                if d <= STOP_REACH_M && x + 1.0 >= from_x {
                    found = Some(x);
                    break;
                }
            }
        }
        let Some(x) = found else {
            continue;
        };
        placed.insert(si);
        let at = (x + FRONT_PAST_M).min(line.length - 1.0);
        if out.last().is_some_and(|l| at - l.at < SAME_STOP_M) {
            continue;
        }
        // Not right at the start: the bus is spawned there.
        if at < 15.0 {
            continue;
        }
        from_x = x;
        c.stops += 1;
        out.push(LineStop { stop: si, at });
    }
    out
}

/// The headway of a line with base `base_min` minutes at `t` (s), `None`
/// when it does not run then (see the module).
pub fn headway_s(base_min: u32, t: &DemandTime) -> Option<u32> {
    let h = t.seconds / 3600;
    let base = base_min.max(5);
    let minutes = match t.day_type() {
        DayType::Weekday => match h {
            0..=4 => return None,
            5 => (2 * base).max(30),
            6 | 9..=15 | 19 => base,
            7 | 8 | 16..=18 => {
                if base >= 15 {
                    (base * 2 / 3).max(10)
                } else {
                    base
                }
            }
            20..=22 => (2 * base).max(30),
            _ => {
                if base <= 15 {
                    60
                } else {
                    return None;
                }
            }
        },
        DayType::Saturday => match h {
            0..=5 | 23 => return None,
            6 | 7 => (2 * base).max(30),
            8..=18 => (base * 4 / 3).div_ceil(5) * 5,
            _ => (2 * base).max(30),
        },
        DayType::Sunday => match h {
            0..=6 | 23 => return None,
            7 | 8 | 20..=22 => 60,
            _ => (2 * base).max(30),
        },
    };
    Some(minutes * 60)
}

/// The departures of `line` on the day of `t` from its start (seconds of
/// the local day, rising): the headway of each departure's hour, the
/// line's phase in the first hour it runs.
pub fn departures(line: &BusLine, t: &DemandTime) -> Vec<u32> {
    let mut out = Vec::new();
    if line.night {
        // The nights to Saturday and Sunday, hourly until 04:00.
        if t.weekday >= 5 {
            out.extend((0..4).map(|h| h * 3600 + line.phase_s % 3600));
        }
        return out;
    }
    let at = |s: u32| DemandTime { seconds: s, ..*t };
    let mut s = line.phase_s % 3600;
    while s < 86_400 {
        match headway_s(line.base_min, &at(s)) {
            Some(h) => {
                out.push(s);
                s += h;
            }
            None => {
                // On at the next hour, in the line's phase.
                s = (s / 3600 + 1) * 3600 + line.phase_s % 600;
            }
        }
    }
    out
}

/// The dwell at a stop (s): 10 – 30 s from the seed, the line, the
/// departure and the stop.
pub fn dwell_s(seed: u64, line: LineId, departure: u32, stop: usize) -> f64 {
    10.0 + 20.0
        * unit_f(key(
            seed,
            u64::from(line),
            u64::from(departure),
            stop as u64 ^ 0xD3E11,
        ))
}

/// Where a departure is scheduled to be at second `now` of its day (m
/// along the line; `None` before it left).
pub fn scheduled_at(departure: u32, now: u32) -> Option<f64> {
    (now >= departure).then(|| f64::from(now - departure) * SCHEDULE_MPS)
}

/// Where along `line` the point `p` (x, z) is (m; the nearest point
/// within 20 m, `None` off the line).
pub fn along(net: &RoadNetwork, line: &BusLine, p: [f64; 2]) -> Option<f64> {
    let mut best: Option<(f64, f64)> = None;
    for (k, &(e, forward)) in line.steps.iter().enumerate() {
        let (s, d) = project(net, e, forward, p);
        if d < 20.0 && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, line.start[k] + s));
        }
    }
    best.map(|(_, x)| x)
}

/// The place (x, z) and driving direction (unit x, z) at `x` metres along
/// `line`.
pub fn place(net: &RoadNetwork, line: &BusLine, x: f64) -> ([f64; 2], [f64; 2]) {
    let (k, s) = line.locate(x.clamp(0.0, line.length));
    let (e, forward) = line.steps[k];
    let g = net.geometry(e);
    let len = g.length();
    let at = if forward { s } else { len - s }.clamp(0.0, len);
    let p = g.point_at(at);
    let (a, b) = (
        g.point_at((at - 1.0).max(0.0)),
        g.point_at((at + 1.0).min(len)),
    );
    let mut h = ocw_world::network::unit([b[0] - a[0], b[2] - a[2]]);
    if !forward {
        h = [-h[0], -h[1]];
    }
    ([p[0], p[2]], h)
}

/// The next stop of a bus of `line` at `p` (x, z): its index and the
/// metres to it.
pub fn next_stop(net: &RoadNetwork, line: &BusLine, p: [f64; 2]) -> Option<(usize, f64)> {
    let x = along(net, line, p)?;
    let k = line.stops.iter().position(|s| s.at > x + 3.0)?;
    Some((k, line.stops[k].at - x))
}

/// The parts of `line` inside the bubbles (line metres, merged over gaps
/// under 50 m).
pub fn spans(net: &RoadNetwork, line: &BusLine, bubbles: &[super::Bubble]) -> Vec<(f64, f64)> {
    let mut out: Vec<(f64, f64)> = Vec::new();
    let mut open: Option<f64> = None;
    for (k, &(e, forward)) in line.steps.iter().enumerate() {
        let g = net.geometry(e);
        let len = g.length();
        let n = g.points.len();
        for j in (0..n).step_by(2).chain(std::iter::once(n - 1)) {
            let idx = if forward { j } else { n - 1 - j };
            let p = g.points[idx];
            let x = line.start[k]
                + if forward {
                    g.cumulative[idx]
                } else {
                    len - g.cumulative[idx]
                };
            let inside = routing::inside(bubbles, [p[0], p[2]], 0.0);
            match (inside, open) {
                (true, None) => open = Some(x),
                (false, Some(a)) => {
                    match out.last_mut() {
                        Some(last) if a - last.1 < 50.0 => last.1 = x,
                        _ => out.push((a, x)),
                    }
                    open = None;
                }
                _ => {}
            }
        }
    }
    if let Some(a) = open {
        match out.last_mut() {
            Some(last) if a - last.1 < 50.0 => last.1 = line.length,
            _ => out.push((a, line.length)),
        }
    }
    out
}

/// A bus's half width (m; the T6 city bus).
const BUS_HALF_W: f64 = 1.275;

/// A spawn waiting for room: (line, departure, line metres, the part's
/// start, tries).
type Pending = (u32, u32, f64, f64, u8);

impl super::TrafficSim {
    /// The line steps from `from` on as route steps (at most 30 steps or
    /// 2 km), for a driver of speed factor `factor`.
    pub(super) fn line_steps(
        &self,
        net: &RoadNetwork,
        line: &BusLine,
        from: usize,
        factor: f64,
    ) -> Vec<super::Step> {
        let mut out: Vec<super::Step> = Vec::new();
        let mut metres = 0.0;
        for &(e, forward) in line.steps.iter().skip(from) {
            if out.len() >= 30 || metres > 2000.0 {
                break;
            }
            let st = routing::make_step(&self.rules, VehicleClass::Bus, e, forward, factor);
            routing::link_turn(net, &mut out, &st);
            metres += net.geometry(e).length();
            out.push(st);
        }
        out
    }

    /// Metres from a bus's front to where it stands at its next stop;
    /// `Some(-1)` while it stands there; `None` for other vehicles and a
    /// bus past its last stop.
    pub(super) fn bus_stop_ahead(&self, v: &super::Vehicle) -> Option<f64> {
        let b = v.bus.as_deref()?;
        if b.dwell_end.is_some() {
            return Some(-1.0);
        }
        let line = &self.lines[b.line as usize];
        let stop = line.stops.get(b.next_stop)?;
        Some(stop.at - (line.start[b.base + v.pos.step] + v.pos.s))
    }

    /// How far a bus pulls to the kerb at its next stop (m, in quarters,
    /// so an `f32` keeps it): the room between its lane and the kerb, at
    /// least 0.25 m and at most 1 m.
    fn kerb_shift(&self, net: &RoadNetwork, line: &BusLine, stop: usize) -> f64 {
        let Some(st) = line.stops.get(stop) else {
            return 0.25;
        };
        let (k, _) = line.locate(st.at);
        let e = line.steps[k].0;
        let w = net.edge(e).width_m;
        let lane = super::lateral_offset(&self.rules[e], w, 0);
        let room = (w / 2.0 - lane - BUS_HALF_W - 0.1).clamp(0.25, 1.0);
        (room * 4.0).round() / 4.0
    }

    /// A bus's lateral shift at its stop: towards the kerb while it pulls
    /// in and stands, back out once its doors close (it indicates).
    pub(super) fn bus_shift(&self, net: &RoadNetwork, v: &mut super::Vehicle) {
        let Some(b) = v.bus.as_deref().copied() else {
            return;
        };
        let line = &self.lines[b.line as usize];
        let rate = f64::from(PULL_RATE);
        if let Some(end) = b.dwell_end {
            let close = (DOORS_CLOSE_S * 1000.0 / f64::from(self.cfg.step_ms)) as u64;
            if self.tick + close >= end {
                v.shift_to = 0.0;
                v.shift_rate = rate / 2.0;
            } else {
                v.shift_to = -self.kerb_shift(net, line, b.next_stop);
                v.shift_rate = rate;
            }
        } else if self
            .bus_stop_ahead(v)
            .is_some_and(|d| d < PULL_IN_M && d > -3.0)
        {
            v.shift_to = -self.kerb_shift(net, line, b.next_stop);
            v.shift_rate = rate;
        } else if v.shift_to < 0.0 && v.pass.is_none() {
            // Past the stop it could not make: back into the lane.
            v.shift_to = 0.0;
            v.shift_rate = rate / 2.0;
        }
    }

    /// After a bus's move: it reaches its stop and opens its doors, closes
    /// them before the dwell ends, and goes on to the next stop.
    pub(super) fn bus_at_stop(&self, v: &mut super::Vehicle) {
        let tick = self.tick;
        let close = (DOORS_CLOSE_S * 1000.0 / f64::from(self.cfg.step_ms)) as u64;
        let ahead = self.bus_stop_ahead(v);
        let speed = v.v;
        let Some(b) = v.bus.as_deref_mut() else {
            return;
        };
        if let Some(end) = b.dwell_end {
            if tick + close >= end {
                b.doors = false;
            }
            if tick >= end {
                b.dwell_end = None;
                b.next_stop += 1;
            }
            return;
        }
        let Some(d) = ahead else {
            return;
        };
        if d < -3.0 {
            // Past it (it could not stop in time): the next one.
            b.next_stop += 1;
        } else if d < 1.5 && speed < 0.5 {
            let line = &self.lines[b.line as usize];
            let s = dwell_s(self.cfg.seed, line.id, b.dep, b.next_stop);
            let ticks = (s * 1000.0 / f64::from(self.cfg.step_ms)).round() as u64;
            b.dwell_end = Some(tick + ticks.max(close + 1));
            b.doors = true;
            // It stands (the last creep to the platform ends here).
            v.v = 0.0;
            v.a = 0.0;
        }
    }

    /// Spawns the buses of the timetable (see the module): once a second,
    /// a departure whose scheduled place enters a part of its line inside
    /// a bubble comes in there; at a fill (new bubbles, the first second,
    /// a jump of the clock) every departure scheduled inside one now.
    pub(super) fn spawn_buses(
        &mut self,
        net: &RoadNetwork,
        bubbles: &[super::Bubble],
        time: &DemandTime,
    ) {
        if self.lines.is_empty() || self.cfg.level == super::TrafficLevel::Off || bubbles.is_empty()
        {
            return;
        }
        let now = (time.weekday, time.seconds);
        let prev = self.bus_clock;
        if prev == Some(now) {
            return;
        }
        self.bus_clock = Some(now);
        let jumped = prev.is_none_or(|p| p.0 != now.0 || p.1 + 1 != now.1);
        if prev.is_some_and(|p| p.0 != now.0 || p.1 > now.1) {
            // A new day: its own departures.
            self.bus_seen.clear();
        }
        let fill = std::mem::take(&mut self.bus_fill) || jumped;
        let first = !self.bus_filled;
        self.bus_filled = true;
        let alive: std::collections::BTreeSet<u32> = self
            .vehicles
            .iter()
            .filter(|v| v.bus.is_some())
            .map(|v| v.f.id)
            .collect();
        let mut fresh: Vec<Pending> = Vec::new();
        for (li, line) in self.lines.iter().enumerate() {
            let spans = &self.bus_spans[li];
            if spans.is_empty() {
                continue;
            }
            for d in departures(line, time) {
                let Some(x1) = scheduled_at(d, now.1) else {
                    break;
                };
                // Along this piece of the route.
                let x1 = x1 - line.offset_m;
                if x1 < 0.0 {
                    continue;
                }
                if x1 > line.length - 30.0 {
                    continue;
                }
                let key = (li as u32, d);
                let seen = self.bus_seen.get(&key).copied();
                if seen.is_some_and(|(_, id)| alive.contains(&id)) {
                    continue;
                }
                for &(a, b) in spans {
                    if seen.is_some_and(|(at, _)| at >= a - 1.0) {
                        continue;
                    }
                    let crossing = x1 >= a && x1 - SCHEDULE_MPS < a && x1 < b;
                    let inside = fill && x1 >= a && x1 < b;
                    let at = if crossing {
                        a.min(line.length - 30.0)
                    } else if inside {
                        let (k, s) = line.locate(x1);
                        let g = net.geometry(line.steps[k].0);
                        let len = g.length();
                        let p = g.point_at(if line.steps[k].1 { s } else { len - s });
                        let near = bubbles.iter().any(|bb| {
                            let (dx, dz) = (p[0] - bb.center[0], p[2] - bb.center[1]);
                            dx * dx + dz * dz < (0.6 * bb.radius).powi(2)
                        });
                        if near && !first {
                            continue;
                        }
                        x1
                    } else {
                        continue;
                    };
                    if !self
                        .bus_pending
                        .iter()
                        .chain(&fresh)
                        .any(|p| (p.0, p.1) == key)
                    {
                        fresh.push((key.0, key.1, at, a, 0));
                    }
                    break;
                }
            }
        }
        self.bus_pending.extend(fresh);
        let pending = std::mem::take(&mut self.bus_pending);
        for (li, d, at, a, tries) in pending {
            match self.spawn_bus(net, li as usize, d, at) {
                Some(id) => {
                    self.bus_seen.insert((li, d), (a, id));
                }
                None if tries < 30 => self.bus_pending.push((li, d, at, a, tries + 1)),
                None => {
                    self.bus_seen.insert((li, d), (a, 0));
                }
            }
        }
        if now.1.is_multiple_of(60) {
            // Forget departures long done.
            let lines = &self.lines;
            self.bus_seen.retain(|&(li, d), (_, id)| {
                alive.contains(id)
                    || scheduled_at(d, now.1).is_none_or(|x| {
                        let l = &lines[li as usize];
                        x - l.offset_m < l.length + 500.0
                    })
            });
        }
    }

    /// Spawns the bus of departure `d` of line `li` at `at` metres along
    /// it, when there is room (no vehicle within 20 m in its lane there,
    /// no rider within 30 m).
    fn spawn_bus(&mut self, net: &RoadNetwork, li: usize, d: u32, at: f64) -> Option<u32> {
        if self.vehicles.len() >= self.cfg.max_vehicles {
            return None;
        }
        let line = &self.lines[li];
        let (k, s) = line.locate(at);
        let (e, forward) = line.steps[k];
        let want = super::lane_key(e as u32, forward, 0);
        let lo = self.slots.partition_point(|x| x.key < want);
        let hi = self.slots.partition_point(|x| x.key <= want);
        if self.slots[lo..hi].iter().any(|x| (x.s - s).abs() < 20.0) {
            return None;
        }
        if self.user_near(net, e, forward, s, 30.0) {
            return None;
        }
        let id = self.next_id;
        let (driver, factor, lunatic) = self.driver(VehicleClass::Bus, id);
        let steps = self.line_steps(net, line, k, factor);
        if steps.is_empty() {
            return None;
        }
        let v0 = f64::from(steps[0].speed) * 0.7;
        let line_id = line.id;
        let next_stop = line.stops.partition_point(|st| st.at < at + 2.0);
        self.push_vehicle(
            net,
            VehicleClass::Bus,
            steps,
            s,
            v0,
            0,
            driver,
            lunatic,
            line_id,
        );
        let v = self.vehicles.last_mut().expect("pushed");
        v.bus = Some(Box::new(super::BusRun {
            line: li as u32,
            dep: d,
            base: k,
            next_stop,
            dwell_end: None,
            doors: false,
        }));
        Some(id)
    }
}
