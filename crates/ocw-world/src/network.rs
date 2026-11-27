//! The road graph prepared for riding (spec §2.2.1, §4.4): dense 3D edge
//! geometry, adjacency, stable-ID lookup, and the junction rules – which
//! exits a junction has, their turn angles and classes, and the default
//! exit when the rider chooses none.

use std::collections::HashMap;

use crate::graph::{Edge, EdgeId, RoadClass, RoadGraph, VertexId};
use crate::surface::Surface;

/// Spacing of the dense edge geometry (spec §4.3: road points every 5 m).
pub const GEOMETRY_STEP_M: f64 = 5.0;
/// Exit angles are measured over this much of the exit (spec §2.2.1).
pub const EXIT_ANGLE_M: f64 = 30.0;
/// The arrival heading is measured over the last metres of the arrival.
pub const ARRIVAL_HEADING_M: f64 = 20.0;
/// Exits within ± this angle are "straight" (degrees).
pub const STRAIGHT_DEG: f64 = 30.0;
/// Beyond ± this angle an exit is "sharp" (degrees).
pub const SHARP_DEG: f64 = 120.0;
/// "Same road" continues within ± this angle (default rule 1, degrees).
pub const SAME_ROAD_DEG: f64 = 60.0;
/// Steep-climb icon: steeper than this over 100 m in the first 500 m.
pub const STEEP_GRADE: f64 = 0.08;

/// The squared horizontal distance from `p` to a polyline (x, z).
fn polyline_d2(points: &[[f64; 2]], p: [f64; 2]) -> f64 {
    let d2 = |a: [f64; 2]| (a[0] - p[0]).powi(2) + (a[1] - p[1]).powi(2);
    if points.len() == 1 {
        return d2(points[0]);
    }
    points
        .windows(2)
        .map(|w| {
            let (a, b) = (w[0], w[1]);
            let ab = [b[0] - a[0], b[1] - a[1]];
            let len2 = ab[0] * ab[0] + ab[1] * ab[1];
            let t = if len2 > 0.0 {
                (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / len2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            d2([a[0] + ab[0] * t, a[1] + ab[1] * t])
        })
        .fold(f64::INFINITY, f64::min)
}

/// Dense centreline of an edge with heights, from `from` to `to`.
#[derive(Debug, Clone, PartialEq)]
pub struct EdgeGeometry {
    /// Points (x, y = road height, z), local metres.
    pub points: Vec<[f64; 3]>,
    /// Horizontal distance from the start at each point.
    pub cumulative: Vec<f64>,
}

impl EdgeGeometry {
    pub fn new(edge: &Edge) -> Self {
        let mut points = Vec::new();
        let mut cumulative = Vec::new();
        let mut s = 0.0;
        for (i, w) in edge.points.windows(2).enumerate() {
            let len = ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt();
            if i == 0 {
                points.push([w[0][0], height(edge, 0.0), w[0][1]]);
                cumulative.push(0.0);
            }
            let n = (len / GEOMETRY_STEP_M).ceil().max(1.0) as usize;
            for k in 1..=n {
                let t = k as f64 / n as f64;
                let d = s + len * t;
                points.push([
                    w[0][0] + (w[1][0] - w[0][0]) * t,
                    height(edge, d),
                    w[0][1] + (w[1][1] - w[0][1]) * t,
                ]);
                cumulative.push(d);
            }
            s += len;
        }
        Self { points, cumulative }
    }

    pub fn length(&self) -> f64 {
        *self.cumulative.last().unwrap_or(&0.0)
    }

    /// Point at `s` metres from the start (clamped), interpolated.
    pub fn point_at(&self, s: f64) -> [f64; 3] {
        let s = s.clamp(0.0, self.length());
        let i = self
            .cumulative
            .partition_point(|&c| c <= s)
            .clamp(1, self.points.len() - 1);
        let (a, b) = (self.points[i - 1], self.points[i]);
        let seg = self.cumulative[i] - self.cumulative[i - 1];
        let t = if seg > 0.0 {
            (s - self.cumulative[i - 1]) / seg
        } else {
            0.0
        };
        std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t)
    }

    /// Points from `s0` to `s1` (either direction), both ends included.
    pub fn slice(&self, s0: f64, s1: f64) -> Vec<[f64; 3]> {
        let (lo, hi) = (s0.min(s1), s0.max(s1));
        let mut out = vec![self.point_at(lo)];
        for (p, &c) in self.points.iter().zip(&self.cumulative) {
            if c > lo + 1e-6 && c < hi - 1e-6 {
                out.push(*p);
            }
        }
        out.push(self.point_at(hi));
        if s0 > s1 {
            out.reverse();
        }
        out
    }
}

fn height(edge: &Edge, s: f64) -> f64 {
    edge.height_at(s).unwrap_or(0.0)
}

/// Turn class of an exit (spec §2.2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TurnClass {
    SharpLeft,
    Left,
    Straight,
    Right,
    SharpRight,
}

impl TurnClass {
    /// Class of a turn angle in degrees (positive = right).
    pub fn of(angle_deg: f64) -> Self {
        if angle_deg < -SHARP_DEG {
            Self::SharpLeft
        } else if angle_deg < -STRAIGHT_DEG {
            Self::Left
        } else if angle_deg <= STRAIGHT_DEG {
            Self::Straight
        } else if angle_deg <= SHARP_DEG {
            Self::Right
        } else {
            Self::SharpRight
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::SharpLeft => "sharp left",
            Self::Left => "left",
            Self::Straight => "straight",
            Self::Right => "right",
            Self::SharpRight => "sharp right",
        }
    }
}

/// A way out of a junction.
#[derive(Debug, Clone, PartialEq)]
pub struct Exit {
    /// Edge index in the graph.
    pub edge: usize,
    /// Ridden from `edge.from` to `edge.to`.
    pub forward: bool,
    /// Turn angle relative to the arrival heading (degrees, positive = right).
    pub angle_deg: f64,
    pub class: TurnClass,
    /// Leads only into a dead end (spec §2.2.2).
    pub dead_end: bool,
    /// Unpaved (a warning for road bikes, spec §2.2.1).
    pub unpaved: bool,
    /// Climbs steeper than [`STEEP_GRADE`] within the first 500 m.
    pub steep: bool,
}

/// A junction ahead: its exits from leftmost to rightmost, and the default.
#[derive(Debug, Clone, PartialEq)]
pub struct Junction {
    pub vertex: VertexId,
    pub exits: Vec<Exit>,
    /// Index into `exits` taken when the rider chooses none.
    pub default: usize,
    /// Route mode: the route's exit (then also the default, spec §2.2.1).
    pub route: Option<usize>,
}

/// The road graph prepared for riding. Shared (in an `Arc`) by the server's
/// riders and by the client's paths.
#[derive(Debug, Clone)]
pub struct RoadNetwork {
    /// Shared with the package it comes from (spec §14 Phase 10 B2).
    graph: std::sync::Arc<RoadGraph>,
    adjacency: Vec<Vec<usize>>,
    /// Each edge's dense geometry, made when first asked for (spec §14
    /// Phase 10 B2: made for every edge at once it took ~100 MB on
    /// Tenerife's 9,000 km of roads; a ride needs a few hundred edges).
    geometry: Vec<std::sync::OnceLock<EdgeGeometry>>,
    by_id: HashMap<EdgeId, usize>,
    /// Default rule 4 on equal roads: right (right-hand traffic) or left.
    pub keep_left: bool,
    /// Name of the world package the graph comes from (empty if none).
    pub package: String,
    /// [`RoadGraph::fingerprint`] of the graph.
    fingerprint: u64,
    /// The static obstacles of the world the graph belongs to (spec §14
    /// Phase 12 K3: trees, buildings, walls, barriers, railings, tunnel
    /// walls), if its package has any.
    obstacles: Option<std::sync::Arc<crate::obstacles::WorldObstacles>>,
    /// The damaged roads under the wheels (spec Phase 8 Q12, v5.35), if
    /// its package has any.
    road_feel: Option<std::sync::Arc<crate::road_feel::RoadFeelMap>>,
    /// Where leaves lie on the roads (spec 4.6.2 Q6: wet leaves in the
    /// ride), if the world has land use.
    leaves: Option<std::sync::Arc<crate::leaves::LeafMap>>,
    /// The junction surfaces (spec §4.8.8, J1): riders ride the heights
    /// the renderer draws there.
    junctions: std::sync::Arc<crate::junction::JunctionSet>,
    /// The ground away from the roads (spec §14 Phase 13A: riding off the
    /// road after a unicorn landing), if the world has tiles.
    ground: Option<std::sync::Arc<crate::ground::GroundMap>>,
    /// The drawn trees away from the roads, as trunks made on demand
    /// (spec 14 Phase 13 Q10 (a), [`crate::woods`]), if the world has
    /// tiles with land use.
    woods: Option<std::sync::Arc<crate::woods::Woods>>,
    /// The edges by 100 m cell, made when first needed ([`Self::nearest_within`]).
    edge_grid: std::sync::Arc<std::sync::OnceLock<EdgeGrid>>,
    /// The motor-traffic rules of the package (section `TRFC`, spec 18B;
    /// `None`: an old package, the rules implied by class) and the country
    /// for edges without data (spec 18E T5: the traffic core reads them).
    traffic: Option<std::sync::Arc<crate::traffic::TrafficData>>,
    traffic_country: crate::traffic::Country,
}

/// Cell size of the edge index (m).
const EDGE_CELL_M: f64 = 100.0;

/// The edges whose polylines pass each cell.
#[derive(Debug, Default)]
struct EdgeGrid {
    cells: HashMap<(i32, i32), Vec<u32>>,
}

impl EdgeGrid {
    fn of(graph: &RoadGraph) -> Self {
        let mut cells: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        let cell = |v: f64| (v / EDGE_CELL_M).floor() as i32;
        for (i, e) in graph.edges.iter().enumerate() {
            let mut mine: Vec<(i32, i32)> = Vec::new();
            let pts = &e.points;
            let segs: Vec<([f64; 2], [f64; 2])> = if pts.len() == 1 {
                vec![(pts[0], pts[0])]
            } else {
                pts.windows(2).map(|w| (w[0], w[1])).collect()
            };
            for (a, b) in segs {
                for cx in cell(a[0].min(b[0]))..=cell(a[0].max(b[0])) {
                    for cz in cell(a[1].min(b[1]))..=cell(a[1].max(b[1])) {
                        if !mine.contains(&(cx, cz)) {
                            mine.push((cx, cz));
                        }
                    }
                }
            }
            for c in mine {
                cells.entry(c).or_default().push(i as u32);
            }
        }
        Self { cells }
    }
}

impl RoadNetwork {
    /// On a graph, owned or shared (a package's: no copy).
    pub fn new(graph: impl Into<std::sync::Arc<RoadGraph>>) -> Self {
        let graph = graph.into();
        let adjacency = graph.adjacency();
        let geometry = (0..graph.edges.len())
            .map(|_| std::sync::OnceLock::new())
            .collect();
        let by_id = graph
            .edges
            .iter()
            .enumerate()
            .map(|(i, e)| (e.id, i))
            .collect();
        let fingerprint = graph.fingerprint();
        let junctions = std::sync::Arc::new(crate::junction::JunctionSet::new(&graph));
        Self {
            junctions,
            graph,
            adjacency,
            geometry,
            by_id,
            keep_left: false,
            package: String::new(),
            fingerprint,
            obstacles: None,
            road_feel: None,
            leaves: None,
            ground: None,
            woods: None,
            edge_grid: Default::default(),
            traffic: None,
            traffic_country: crate::traffic::Country::Other,
        }
    }

    /// The same network with the traffic rules of its package (spec 18E
    /// T5): its `TRFC` section, else the rules implied by class and the
    /// country of its time zone ([`crate::traffic::RoadRules::of`]).
    pub fn with_traffic_of(self, package: &crate::WorldPackage) -> Self {
        let country = crate::traffic::RoadRules::of(package).country;
        self.with_traffic(package.traffic.clone(), country)
    }

    /// The same network with these traffic rules (`None`: implied by class
    /// in `country`).
    pub fn with_traffic(
        mut self,
        data: Option<std::sync::Arc<crate::traffic::TrafficData>>,
        country: crate::traffic::Country,
    ) -> Self {
        self.traffic = data;
        self.traffic_country = country;
        self
    }

    /// The motor-traffic rules of the edges (spec 18B): the package's
    /// `TRFC` where it has it, else the implied ones.
    pub fn road_rules(&self) -> crate::traffic::RoadRules<'_> {
        crate::traffic::RoadRules {
            data: self.traffic.as_deref(),
            country: self.traffic_country,
        }
    }

    /// The same network with the ground of its world (spec §14 Phase 13A).
    pub fn with_ground(mut self, ground: Option<std::sync::Arc<crate::ground::GroundMap>>) -> Self {
        self.ground = ground;
        self
    }

    /// The ground away from the roads (heights and cover), if known.
    pub fn ground(&self) -> Option<&std::sync::Arc<crate::ground::GroundMap>> {
        self.ground.as_ref()
    }

    /// The same network with the trees off the road as obstacles (spec 14
    /// Phase 13 Q10 (a)).
    pub fn with_woods(mut self, woods: Option<std::sync::Arc<crate::woods::Woods>>) -> Self {
        self.woods = woods;
        self
    }

    /// The source of the trunks off the road, if any.
    pub fn woods(&self) -> Option<&std::sync::Arc<crate::woods::Woods>> {
        self.woods.as_ref()
    }

    /// The same network with the leaf map of its world (spec 4.6.2 Q6).
    pub fn with_leaves(mut self, leaves: Option<std::sync::Arc<crate::leaves::LeafMap>>) -> Self {
        self.leaves = leaves;
        self
    }

    /// Where leaves lie on the roads (spec 4.6.2 Q6).
    pub fn leaves(&self) -> Option<&std::sync::Arc<crate::leaves::LeafMap>> {
        self.leaves.as_ref()
    }

    /// The same network with the static obstacles of its world.
    pub fn with_obstacles(
        mut self,
        obstacles: Option<std::sync::Arc<crate::obstacles::WorldObstacles>>,
    ) -> Self {
        self.obstacles = obstacles;
        self
    }

    /// The same network with the damaged roads of its world (spec Phase 8
    /// Q12: bumps and rolling resistance, [`crate::road_feel`]).
    pub fn with_road_feel(
        mut self,
        feel: Option<std::sync::Arc<crate::road_feel::RoadFeelMap>>,
    ) -> Self {
        self.road_feel = feel;
        self
    }

    /// The damaged roads of the world, if known.
    pub fn road_feel(&self) -> Option<&crate::road_feel::RoadFeelMap> {
        self.road_feel.as_deref()
    }

    /// The same, shared (the browser app adds streamed tiles to it).
    pub fn road_feel_map(&self) -> Option<&std::sync::Arc<crate::road_feel::RoadFeelMap>> {
        self.road_feel.as_ref()
    }

    /// The static obstacles of the world (spec §14 Phase 12 K3).
    pub fn obstacles(&self) -> Option<&std::sync::Arc<crate::obstacles::WorldObstacles>> {
        self.obstacles.as_ref()
    }

    /// The same network, named after its world package.
    pub fn with_package_name(mut self, name: impl Into<String>) -> Self {
        self.package = name.into();
        self
    }

    /// Fingerprint of the road graph (spec §6.5: client and server must use
    /// the same package).
    pub fn fingerprint(&self) -> u64 {
        self.fingerprint
    }

    pub fn graph(&self) -> &RoadGraph {
        &self.graph
    }

    /// The junction surfaces of the graph (spec §4.8.8).
    pub fn junctions(&self) -> &crate::junction::JunctionSet {
        &self.junctions
    }

    pub fn edge(&self, i: usize) -> &Edge {
        &self.graph.edges[i]
    }

    pub fn geometry(&self, i: usize) -> &EdgeGeometry {
        self.geometry[i].get_or_init(|| EdgeGeometry::new(&self.graph.edges[i]))
    }

    /// How many edges' dense geometry was made so far.
    pub fn geometry_made(&self) -> usize {
        self.geometry.iter().filter(|g| g.get().is_some()).count()
    }

    /// Edge index of a stable edge ID.
    pub fn edge_index(&self, id: EdgeId) -> Option<usize> {
        self.by_id.get(&id).copied()
    }

    /// Edges touching vertex `v`.
    pub fn edges_at(&self, v: VertexId) -> &[usize] {
        &self.adjacency[v.0 as usize]
    }

    /// The vertex an edge is left from / arrived at.
    pub fn start_vertex(&self, edge: usize, forward: bool) -> VertexId {
        let e = &self.graph.edges[edge];
        if forward { e.from } else { e.to }
    }

    pub fn end_vertex(&self, edge: usize, forward: bool) -> VertexId {
        self.start_vertex(edge, !forward)
    }

    /// Horizontal direction leaving along `edge` (in the riding direction),
    /// measured over its first `over_m` metres.
    pub fn leaving_direction(&self, edge: usize, forward: bool, over_m: f64) -> [f64; 2] {
        let g = self.geometry(edge);
        let len = g.length();
        let d = over_m.min(len);
        let (a, b) = if forward {
            (g.point_at(0.0), g.point_at(d))
        } else {
            (g.point_at(len), g.point_at(len - d))
        };
        unit([b[0] - a[0], b[2] - a[2]])
    }

    /// Horizontal direction arriving along `edge` at its end, over the last
    /// `over_m` metres.
    pub fn arriving_direction(&self, edge: usize, forward: bool, over_m: f64) -> [f64; 2] {
        let d = self.leaving_direction(edge, !forward, over_m);
        [-d[0], -d[1]]
    }

    /// Rideable ways out of `v` for a rider arriving along `arrival` (edge,
    /// forward) with horizontal `heading`, from leftmost to rightmost
    /// (spec §2.2.1): not back along the arrival edge, not against a
    /// one-way road. Bicycle access was already filtered by the builder.
    pub fn exits(
        &self,
        v: VertexId,
        arrival: Option<(usize, bool)>,
        heading: [f64; 2],
    ) -> Vec<Exit> {
        let out = self.exits_with(v, arrival, heading, false);
        // A rider who arrives against a one-way road (only after an
        // automatic U-turn at a dead end or the region border on one) and
        // finds no legal way on rides on against it rather than turning
        // again, which would send them back into the dead end for ever.
        let contraflow = arrival.is_some_and(|(e, f)| !self.graph.edges[e].oneway.allows(f));
        if out.is_empty() && contraflow {
            return self.exits_with(v, arrival, heading, true);
        }
        out
    }

    fn exits_with(
        &self,
        v: VertexId,
        arrival: Option<(usize, bool)>,
        heading: [f64; 2],
        ignore_oneway: bool,
    ) -> Vec<Exit> {
        let mut out = Vec::new();
        for &e in self.edges_at(v) {
            let edge = &self.graph.edges[e];
            for forward in [true, false] {
                if self.start_vertex(e, forward) != v
                    || !(ignore_oneway || edge.oneway.allows(forward))
                {
                    continue;
                }
                if arrival == Some((e, !forward)) {
                    continue; // back where we came from
                }
                let dir = self.leaving_direction(e, forward, EXIT_ANGLE_M);
                let angle_deg = signed_angle(heading, dir).to_degrees();
                out.push(Exit {
                    edge: e,
                    forward,
                    angle_deg,
                    class: TurnClass::of(angle_deg),
                    dead_end: edge.dead_end_m.is_some() && forward,
                    unpaved: Surface::of_class(edge.class) != Surface::Asphalt,
                    steep: self.climbs_steeply(e, forward),
                });
            }
        }
        out.sort_by(|a, b| a.angle_deg.total_cmp(&b.angle_deg));
        out
    }

    /// The junction at `v` (two or more exits), or `None` when the road
    /// just continues (one exit) or ends (none).
    pub fn junction(
        &self,
        v: VertexId,
        arrival: Option<(usize, bool)>,
        heading: [f64; 2],
    ) -> Option<Junction> {
        let exits = self.exits(v, arrival, heading);
        if exits.len() < 2 {
            return None;
        }
        let default = self.default_exit(&exits, arrival.map(|a| a.0))?;
        Some(Junction {
            vertex: v,
            exits,
            default,
            route: None,
        })
    }

    /// The exit taken when the rider presses nothing ("follow the road",
    /// spec §2.2.1); the first matching rule wins:
    /// 1. the same road (OSM way or name) within ±60°, the straightest;
    /// 2. the straightest exit within ±30°;
    /// 3. the higher road category;
    /// 4. on equal roads, the rightmost (leftmost with `keep_left`);
    /// 5. never a dead end while another exit exists.
    pub fn default_exit(&self, exits: &[Exit], arrival_edge: Option<usize>) -> Option<usize> {
        let live: Vec<usize> = (0..exits.len()).filter(|&i| !exits[i].dead_end).collect();
        let candidates = if live.is_empty() {
            (0..exits.len()).collect()
        } else {
            live
        };
        let straightest = |pool: &mut dyn Iterator<Item = usize>| {
            pool.min_by(|&a, &b| {
                exits[a]
                    .angle_deg
                    .abs()
                    .total_cmp(&exits[b].angle_deg.abs())
            })
        };
        if let Some(arr) = arrival_edge.map(|a| &self.graph.edges[a]) {
            let same_road =
                |e: &Edge| e.osm_way == arr.osm_way || (e.name.is_some() && e.name == arr.name);
            let mut pool = candidates.iter().copied().filter(|&i| {
                exits[i].angle_deg.abs() <= SAME_ROAD_DEG
                    && same_road(&self.graph.edges[exits[i].edge])
            });
            if let Some(i) = straightest(&mut pool) {
                return Some(i);
            }
        }
        let mut pool = candidates
            .iter()
            .copied()
            .filter(|&i| exits[i].angle_deg.abs() <= STRAIGHT_DEG);
        if let Some(i) = straightest(&mut pool) {
            return Some(i);
        }
        let best: RoadClass = candidates
            .iter()
            .map(|&i| self.graph.edges[exits[i].edge].class)
            .min()?;
        let top = candidates
            .iter()
            .copied()
            .filter(|&i| self.graph.edges[exits[i].edge].class == best);
        if self.keep_left {
            top.min_by(|&a, &b| exits[a].angle_deg.total_cmp(&exits[b].angle_deg))
        } else {
            top.max_by(|&a, &b| exits[a].angle_deg.total_cmp(&exits[b].angle_deg))
        }
    }

    /// Whether riding `edge` this way climbs more than [`STEEP_GRADE`] over
    /// any 100 m of its first 500 m.
    fn climbs_steeply(&self, edge: usize, forward: bool) -> bool {
        let g = self.geometry(edge);
        let len = g.length();
        let h = |s: f64| {
            let s = if forward { s } else { len - s };
            g.point_at(s)[1]
        };
        let window = 100.0f64.min(len);
        if window < 20.0 {
            return false;
        }
        let mut s = 0.0;
        while s + window <= len.min(500.0) + 1e-9 {
            if (h(s + window) - h(s)) / window > STEEP_GRADE {
                return true;
            }
            s += 25.0;
        }
        false
    }

    /// The point of the road graph nearest to `p` (x, z): (edge, metres
    /// from its `from` vertex).
    pub fn nearest(&self, p: [f64; 2]) -> Option<(usize, f64)> {
        // The dense geometry subdivides the edge's own polyline, so the
        // polyline's distance picks the candidates (equal within rounding)
        // without making every edge's geometry.
        let coarse: Vec<f64> = self
            .graph
            .edges
            .iter()
            .map(|e| polyline_d2(&e.points, p))
            .collect();
        let least = coarse.iter().copied().fold(f64::INFINITY, f64::min);
        if !least.is_finite() {
            return None;
        }
        let near = least + 1e-6 * (1.0 + least);
        let mut best: Option<(f64, usize, f64)> = None;
        for i in (0..coarse.len()).filter(|&i| coarse[i] <= near) {
            let g = self.geometry(i);
            for (k, w) in g.points.windows(2).enumerate() {
                let (a, b) = ([w[0][0], w[0][2]], [w[1][0], w[1][2]]);
                let ab = [b[0] - a[0], b[1] - a[1]];
                let len2 = ab[0] * ab[0] + ab[1] * ab[1];
                let t = if len2 > 0.0 {
                    (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / len2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let d2 = (a[0] + ab[0] * t - p[0]).powi(2) + (a[1] + ab[1] * t - p[1]).powi(2);
                if best.is_none_or(|(bd, _, _)| d2 < bd) {
                    let s = g.cumulative[k] + (g.cumulative[k + 1] - g.cumulative[k]) * t;
                    best = Some((d2, i, s));
                }
            }
        }
        best.map(|(_, i, s)| (i, s))
    }

    /// The edges whose polylines pass the 100 m cells within `radius` of
    /// `p` (candidates, sorted; the parked cars of spec 18B).
    pub fn edges_near(&self, p: [f64; 2], radius: f64) -> Vec<usize> {
        if !(p[0].is_finite() && p[1].is_finite()) {
            return Vec::new();
        }
        let grid = self.edge_grid.get_or_init(|| EdgeGrid::of(&self.graph));
        let cell = |v: f64| (v / EDGE_CELL_M).floor() as i32;
        let mut cand: Vec<usize> = Vec::new();
        for cx in cell(p[0] - radius)..=cell(p[0] + radius) {
            for cz in cell(p[1] - radius)..=cell(p[1] + radius) {
                if let Some(list) = grid.cells.get(&(cx, cz)) {
                    cand.extend(list.iter().map(|&i| i as usize));
                }
            }
        }
        cand.sort_unstable();
        cand.dedup();
        cand
    }

    /// The point of the road graph nearest to `p` within `radius` m (an
    /// index of 100 m cells: cheap enough for every physics tick, spec
    /// §14 Phase 13A): (edge, metres from its `from` vertex, the distance
    /// from `p`, the road's height there); `None` when no road is that near.
    pub fn nearest_within(&self, p: [f64; 2], radius: f64) -> Option<(usize, f64, f64, f64)> {
        if !(p[0].is_finite() && p[1].is_finite()) {
            return None;
        }
        let grid = self.edge_grid.get_or_init(|| EdgeGrid::of(&self.graph));
        let cell = |v: f64| (v / EDGE_CELL_M).floor() as i32;
        let mut cand: Vec<u32> = Vec::new();
        for cx in cell(p[0] - radius)..=cell(p[0] + radius) {
            for cz in cell(p[1] - radius)..=cell(p[1] + radius) {
                if let Some(list) = grid.cells.get(&(cx, cz)) {
                    for &i in list {
                        if !cand.contains(&i) {
                            cand.push(i);
                        }
                    }
                }
            }
        }
        cand.sort_unstable();
        let r2 = radius * radius;
        let mut best: Option<(f64, usize, f64, f64)> = None;
        for i in cand {
            let i = i as usize;
            // The polyline first: most candidates are farther.
            if polyline_d2(&self.graph.edges[i].points, p) > r2 + 1.0 {
                continue;
            }
            let g = self.geometry(i);
            for (k, w) in g.points.windows(2).enumerate() {
                let (a, b) = ([w[0][0], w[0][2]], [w[1][0], w[1][2]]);
                let ab = [b[0] - a[0], b[1] - a[1]];
                let len2 = ab[0] * ab[0] + ab[1] * ab[1];
                let t = if len2 > 0.0 {
                    (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / len2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let d2 = (a[0] + ab[0] * t - p[0]).powi(2) + (a[1] + ab[1] * t - p[1]).powi(2);
                if d2 <= r2 && best.is_none_or(|(bd, _, _, _)| d2 < bd) {
                    let s = g.cumulative[k] + (g.cumulative[k + 1] - g.cumulative[k]) * t;
                    let y = w[0][1] + (w[1][1] - w[0][1]) * t;
                    best = Some((d2, i, s, y));
                }
            }
        }
        best.map(|(d2, i, s, y)| (i, s, d2.sqrt(), y))
    }

    /// A random start on a real road (no tracks, paths or dead ends, at
    /// least 200 m long) from `seed`: (edge, offset, forward). Falls back
    /// to any edge.
    pub fn random_start(&self, seed: u64) -> Option<(usize, f64, bool)> {
        Self::random_start_in(&self.graph, seed)
    }

    /// [`RoadNetwork::random_start`] on a bare graph (the same start;
    /// without building a network: the browser chooses a phone's tiles
    /// from it while the base is decoded, 14 Phase 10, 10G).
    pub fn random_start_in(graph: &RoadGraph, seed: u64) -> Option<(usize, f64, bool)> {
        let good = |e: &Edge| {
            e.dead_end_m.is_none()
                && !matches!(e.class, RoadClass::Track | RoadClass::Path)
                && e.length_m() >= 200.0
        };
        let mut pool: Vec<usize> = (0..graph.edges.len())
            .filter(|&i| good(&graph.edges[i]))
            .collect();
        if pool.is_empty() {
            pool = (0..graph.edges.len()).collect();
        }
        if pool.is_empty() {
            return None;
        }
        let r = splitmix(seed);
        let edge = pool[(r % pool.len() as u64) as usize];
        let e = &graph.edges[edge];
        let len = EdgeGeometry::new(e).length();
        let frac = (splitmix(r) >> 11) as f64 / (1u64 << 53) as f64;
        let offset = len * (0.1 + 0.4 * frac);
        let forward = match e.oneway {
            crate::graph::Oneway::No => splitmix(r ^ 0x5555) & 1 == 0,
            o => o.allows(true),
        };
        Some((edge, offset, forward))
    }
}

/// SplitMix64: a small, good mixing function for seeds.
pub fn splitmix(seed: u64) -> u64 {
    let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Unit vector (x, z); +x if degenerate.
pub fn unit(v: [f64; 2]) -> [f64; 2] {
    let len = (v[0] * v[0] + v[1] * v[1]).sqrt();
    if len > 1e-12 {
        [v[0] / len, v[1] / len]
    } else {
        [1.0, 0.0]
    }
}

/// Signed angle from `a` to `b` (rad, positive = right turn; x east, z south).
pub fn signed_angle(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] * b[1] - a[1] * b[0]).atan2(a[0] * b[0] + a[1] * b[1])
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::graph::Oneway;

    /// A graph of straight edges between vertex positions (x, z).
    pub(crate) fn graph(vertices: &[[f64; 2]], edges: &[(u32, u32)]) -> RoadGraph {
        RoadGraph {
            vertices: vertices.to_vec(),
            edges: edges
                .iter()
                .enumerate()
                .map(|(i, &(a, b))| Edge {
                    id: EdgeId(1000 + i as u64),
                    from: VertexId(a),
                    to: VertexId(b),
                    points: vec![vertices[a as usize], vertices[b as usize]],
                    class: RoadClass::Residential,
                    name: None,
                    width_m: 5.0,
                    bridge: false,
                    tunnel: false,
                    dead_end_m: None,
                    boundary: false,
                    oneway: Oneway::No,
                    osm_way: i as i64,
                    profile: Vec::new(),
                })
                .collect(),
            boundary_vertices: Default::default(),
        }
    }

    /// A crossroads at the origin: arms 200 m west (0), east (1), north
    /// (2, -z) and south (3, +z). Edges run from the centre (vertex 4) out.
    pub(crate) fn crossroads() -> RoadGraph {
        graph(
            &[
                [-200.0, 0.0],
                [200.0, 0.0],
                [0.0, -200.0],
                [0.0, 200.0],
                [0.0, 0.0],
            ],
            &[(4, 0), (4, 1), (4, 2), (4, 3)],
        )
    }

    const EAST: [f64; 2] = [1.0, 0.0];

    #[test]
    fn edge_geometry_is_dense_and_follows_the_profile() {
        let mut g = graph(&[[0.0, 0.0], [12.0, 0.0]], &[(0, 1)]);
        g.edges[0].profile = vec![100.0, 101.0, 102.0, 103.0];
        let n = RoadNetwork::new(g);
        let geo = n.geometry(0);
        assert_eq!(geo.points.len(), 4, "12 m in 5 m steps");
        assert_eq!(geo.length(), 12.0);
        assert_eq!(geo.point_at(7.5), [7.5, 101.5, 0.0]);
        let back = geo.slice(10.0, 2.0);
        assert_eq!(back.first().unwrap()[0], 10.0);
        assert_eq!(back.last().unwrap()[0], 2.0);
        assert!(back.windows(2).all(|w| w[1][0] < w[0][0]));
        assert_eq!(n.edge_index(EdgeId(1000)), Some(0));
        assert_eq!(n.edge_index(EdgeId(7)), None);
    }

    #[test]
    fn junction_detection_and_exit_classification() {
        let n = RoadNetwork::new(crossroads());
        // Arriving from the west, heading east, on edge 0 (centre → west,
        // ridden backwards).
        let j = n.junction(VertexId(4), Some((0, false)), EAST).unwrap();
        // Three exits (not back west), leftmost first: north, east, south.
        let order: Vec<(usize, TurnClass)> = j.exits.iter().map(|e| (e.edge, e.class)).collect();
        assert_eq!(
            order,
            vec![
                (2, TurnClass::Left),
                (1, TurnClass::Straight),
                (3, TurnClass::Right)
            ]
        );
        assert!((j.exits[0].angle_deg + 90.0).abs() < 1e-9);
        assert!(j.exits[1].angle_deg.abs() < 1e-9);
        assert!((j.exits[2].angle_deg - 90.0).abs() < 1e-9);
        assert_eq!(j.default, 1, "straight on");
        // A road end and a plain continuation are not junctions.
        assert!(n.junction(VertexId(1), Some((1, true)), EAST).is_none());
        let line = RoadNetwork::new(graph(
            &[[0.0, 0.0], [100.0, 0.0], [200.0, 0.0]],
            &[(0, 1), (1, 2)],
        ));
        assert!(line.junction(VertexId(1), Some((0, true)), EAST).is_none());
        assert_eq!(line.exits(VertexId(1), Some((0, true)), EAST).len(), 1);
        // Classes by angle.
        assert_eq!(TurnClass::of(-150.0), TurnClass::SharpLeft);
        assert_eq!(TurnClass::of(-31.0), TurnClass::Left);
        assert_eq!(TurnClass::of(30.0), TurnClass::Straight);
        assert_eq!(TurnClass::of(121.0), TurnClass::SharpRight);
    }

    #[test]
    fn exit_angles_are_measured_over_30_m_not_the_first_segment() {
        // The exit east starts with a 2 m stub to the south-east, then runs
        // straight east: measured over 30 m it is nearly straight.
        let mut g = crossroads();
        g.edges[1].points = vec![[0.0, 0.0], [2.0, 2.0], [200.0, 2.0]];
        let n = RoadNetwork::new(g);
        let j = n.junction(VertexId(4), Some((0, false)), EAST).unwrap();
        let east = j.exits.iter().find(|e| e.edge == 1).unwrap();
        assert!(
            east.angle_deg > 0.0 && east.angle_deg < 10.0,
            "{}",
            east.angle_deg
        );
        assert_eq!(east.class, TurnClass::Straight);
    }

    #[test]
    fn one_way_roads_are_no_exit_against_their_direction() {
        let mut g = crossroads();
        // North arm only rideable towards the centre; south arm only away.
        g.edges[2].oneway = Oneway::Backward;
        g.edges[3].oneway = Oneway::Forward;
        let n = RoadNetwork::new(g);
        let j = n.junction(VertexId(4), Some((0, false)), EAST).unwrap();
        let edges: Vec<usize> = j.exits.iter().map(|e| e.edge).collect();
        assert_eq!(edges, vec![1, 3]);
    }

    /// Regression: a rider arriving against a one-way road (after an
    /// automatic U-turn on it) with no legal way on rides on against the
    /// one-way roads instead of finding no exit (another U-turn, back and
    /// forth for ever). With a legal way on, only that one counts.
    #[test]
    fn against_a_one_way_road_without_a_legal_exit_ride_on() {
        // A one-way chain west → centre → east: edge 0 (centre → west) only
        // towards the centre, edge 1 (centre → east) only away from it.
        let mut g = graph(
            &[[-200.0, 0.0], [200.0, 0.0], [0.0, 0.0]],
            &[(2, 0), (2, 1)],
        );
        g.edges[0].oneway = Oneway::Backward;
        g.edges[1].oneway = Oneway::Forward;
        let n = RoadNetwork::new(g.clone());
        // Along the one-way roads: on east.
        let along = n.exits(VertexId(2), Some((0, false)), EAST);
        assert_eq!(along.len(), 1);
        assert_eq!((along[0].edge, along[0].forward), (1, true));
        // Against edge 1 (arriving westwards): no legal exit, so on west
        // against edge 0, never straight back.
        let against = n.exits(VertexId(2), Some((1, false)), [-1.0, 0.0]);
        assert_eq!(against.len(), 1);
        assert_eq!((against[0].edge, against[0].forward), (0, true));
        // A legal way on (a two-way side road) wins over riding against.
        let mut g = graph(
            &[[-200.0, 0.0], [200.0, 0.0], [0.0, 200.0], [0.0, 0.0]],
            &[(3, 0), (3, 1), (3, 2)],
        );
        g.edges[0].oneway = Oneway::Backward;
        g.edges[1].oneway = Oneway::Forward;
        let n = RoadNetwork::new(g);
        let against = n.exits(VertexId(3), Some((1, false)), [-1.0, 0.0]);
        assert_eq!(against.len(), 1);
        assert_eq!(against[0].edge, 2);
    }

    #[test]
    fn default_exit_rules() {
        // Rule 1: the same road wins over a straighter side road. West and
        // north arms are one OSM way, bending left by 90° (beyond ±60°:
        // not the same-road rule) – make it 50° instead.
        let mut g = graph(
            &[
                [-200.0, 0.0],
                [200.0, 0.0],
                [128.6, -153.2], // 50° left
                [0.0, 200.0],
                [0.0, 0.0],
            ],
            &[(4, 0), (4, 1), (4, 2), (4, 3)],
        );
        g.edges[0].osm_way = 77;
        g.edges[2].osm_way = 77;
        let n = RoadNetwork::new(g.clone());
        let j = n.junction(VertexId(4), Some((0, false)), EAST).unwrap();
        assert_eq!(j.exits[j.default].edge, 2, "same road, 50° left");
        // By name instead of the way.
        let mut by_name = g.clone();
        by_name.edges[2].osm_way = 5;
        by_name.edges[0].name = Some("Landstrasse".into());
        by_name.edges[2].name = Some("Landstrasse".into());
        let n = RoadNetwork::new(by_name);
        let j = n.junction(VertexId(4), Some((0, false)), EAST).unwrap();
        assert_eq!(j.exits[j.default].edge, 2);
        // Rule 2: otherwise the straightest within ±30°.
        g.edges[2].osm_way = 5;
        let n = RoadNetwork::new(g.clone());
        let j = n.junction(VertexId(4), Some((0, false)), EAST).unwrap();
        assert_eq!(j.exits[j.default].edge, 1);
        // Rule 3: T-junction (no straight exit): the bigger road.
        let mut t = crossroads();
        t.edges.remove(1); // no east arm
        t.edges[1].class = RoadClass::Secondary; // north
        let n = RoadNetwork::new(t.clone());
        let j = n.junction(VertexId(4), Some((0, false)), EAST).unwrap();
        assert_eq!(j.exits.len(), 2);
        assert_eq!(n.edge(j.exits[j.default].edge).class, RoadClass::Secondary);
        // Rule 4: equal roads: right, or left with keep_left.
        t.edges[1].class = RoadClass::Residential;
        let mut n = RoadNetwork::new(t);
        let j = n.junction(VertexId(4), Some((0, false)), EAST).unwrap();
        assert_eq!(j.exits[j.default].class, TurnClass::Right);
        n.keep_left = true;
        let j = n.junction(VertexId(4), Some((0, false)), EAST).unwrap();
        assert_eq!(j.exits[j.default].class, TurnClass::Left);
        // Rule 5: never into a dead end while another exit exists, even
        // straight on.
        let mut d = crossroads();
        d.edges[1].dead_end_m = Some(200.0);
        let n = RoadNetwork::new(d);
        let j = n.junction(VertexId(4), Some((0, false)), EAST).unwrap();
        let east = j.exits.iter().find(|e| e.edge == 1).unwrap();
        assert!(east.dead_end);
        assert_ne!(j.exits[j.default].edge, 1);
        assert_eq!(j.exits[j.default].class, TurnClass::Right);
    }

    #[test]
    fn exit_icons_unpaved_and_steep() {
        let mut g = crossroads();
        g.edges[2].class = RoadClass::Track;
        // South arm climbs 10 m per 100 m.
        g.edges[3].profile = (0..=40).map(|i| f64::from(i) * 0.5).collect();
        let n = RoadNetwork::new(g);
        let j = n.junction(VertexId(4), Some((0, false)), EAST).unwrap();
        let by_edge = |e: usize| j.exits.iter().find(|x| x.edge == e).unwrap();
        assert!(by_edge(2).unpaved && !by_edge(2).steep);
        assert!(by_edge(3).steep && !by_edge(3).unpaved);
        assert!(!by_edge(1).steep);
    }

    #[test]
    fn nearest_point_and_random_starts() {
        let n = RoadNetwork::new(crossroads());
        let (e, s) = n.nearest([50.0, 3.0]).unwrap();
        assert_eq!(e, 1);
        assert!((s - 50.0).abs() < 1e-9);
        for seed in 0..20 {
            let (e, s, _) = n.random_start(seed).unwrap();
            assert!(e < 4 && s > 0.0 && s < 200.0);
        }
        let starts: std::collections::BTreeSet<usize> =
            (0..50).map(|s| n.random_start(s).unwrap().0).collect();
        assert!(starts.len() > 2, "{starts:?}");
    }

    /// The memory per resident tile (spec §14 Phase 10 B2): the dense
    /// geometry is made per edge when first asked for, the same as made
    /// at once; the nearest point is found without making every edge's
    /// and is the one the dense points give.
    #[test]
    fn edge_geometry_is_made_when_asked_for() {
        let g = crossroads();
        let n = RoadNetwork::new(g.clone());
        assert_eq!(n.geometry_made(), 0);
        let (e, s) = n.nearest([50.0, 3.0]).unwrap();
        assert!(n.geometry_made() < g.edges.len(), "{}", n.geometry_made());
        assert_eq!(*n.geometry(2), EdgeGeometry::new(&g.edges[2]));
        // Brute force over every edge's dense points.
        let mut best = (f64::INFINITY, 0, 0.0);
        for (i, edge) in g.edges.iter().enumerate() {
            let geo = EdgeGeometry::new(edge);
            for (k, w) in geo.points.windows(2).enumerate() {
                let (a, b) = ([w[0][0], w[0][2]], [w[1][0], w[1][2]]);
                let ab = [b[0] - a[0], b[1] - a[1]];
                let len2 = ab[0] * ab[0] + ab[1] * ab[1];
                let t = (((50.0 - a[0]) * ab[0] + (3.0 - a[1]) * ab[1]) / len2).clamp(0.0, 1.0);
                let d2 = (a[0] + ab[0] * t - 50.0).powi(2) + (a[1] + ab[1] * t - 3.0).powi(2);
                if d2 < best.0 {
                    let s = geo.cumulative[k] + (geo.cumulative[k + 1] - geo.cumulative[k]) * t;
                    best = (d2, i, s);
                }
            }
        }
        assert_eq!((e, s), (best.1, best.2));
        // The graph is shared with whoever passed it in.
        let shared = std::sync::Arc::new(g);
        let n = RoadNetwork::new(shared.clone());
        assert!(std::ptr::eq(n.graph(), &*shared));
    }
}
