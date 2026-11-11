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

/// Dense centreline of an edge with heights, from `from` to `to`.
#[derive(Debug, Clone, PartialEq)]
pub struct EdgeGeometry {
    /// Points (x, y = road height, z), local metres.
    pub points: Vec<[f64; 3]>,
    /// Horizontal distance from the start at each point.
    pub cumulative: Vec<f64>,
}

impl EdgeGeometry {
    fn new(edge: &Edge) -> Self {
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
}

/// The road graph prepared for riding. Shared (in an `Arc`) by the server's
/// riders and by the client's paths.
#[derive(Debug, Clone)]
pub struct RoadNetwork {
    graph: RoadGraph,
    adjacency: Vec<Vec<usize>>,
    geometry: Vec<EdgeGeometry>,
    by_id: HashMap<EdgeId, usize>,
    /// Default rule 4 on equal roads: right (right-hand traffic) or left.
    pub keep_left: bool,
}

impl RoadNetwork {
    pub fn new(graph: RoadGraph) -> Self {
        let adjacency = graph.adjacency();
        let geometry = graph.edges.iter().map(EdgeGeometry::new).collect();
        let by_id = graph
            .edges
            .iter()
            .enumerate()
            .map(|(i, e)| (e.id, i))
            .collect();
        Self {
            graph,
            adjacency,
            geometry,
            by_id,
            keep_left: false,
        }
    }

    pub fn graph(&self) -> &RoadGraph {
        &self.graph
    }

    pub fn edge(&self, i: usize) -> &Edge {
        &self.graph.edges[i]
    }

    pub fn geometry(&self, i: usize) -> &EdgeGeometry {
        &self.geometry[i]
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
        let g = &self.geometry[edge];
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
        let mut out = Vec::new();
        for &e in self.edges_at(v) {
            let edge = &self.graph.edges[e];
            for forward in [true, false] {
                if self.start_vertex(e, forward) != v || !edge.oneway.allows(forward) {
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
        let g = &self.geometry[edge];
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
        let mut best: Option<(f64, usize, f64)> = None;
        for (i, g) in self.geometry.iter().enumerate() {
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

    /// A random start on a real road (no tracks, paths or dead ends, at
    /// least 200 m long) from `seed`: (edge, offset, forward). Falls back
    /// to any edge.
    pub fn random_start(&self, seed: u64) -> Option<(usize, f64, bool)> {
        let good = |e: &Edge| {
            e.dead_end_m.is_none()
                && !matches!(e.class, RoadClass::Track | RoadClass::Path)
                && e.length_m() >= 200.0
        };
        let mut pool: Vec<usize> = (0..self.graph.edges.len())
            .filter(|&i| good(&self.graph.edges[i]))
            .collect();
        if pool.is_empty() {
            pool = (0..self.graph.edges.len()).collect();
        }
        if pool.is_empty() {
            return None;
        }
        let r = splitmix(seed);
        let edge = pool[(r % pool.len() as u64) as usize];
        let e = &self.graph.edges[edge];
        let len = self.geometry[edge].length();
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
}
