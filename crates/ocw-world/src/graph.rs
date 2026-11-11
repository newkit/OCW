//! Road graph (spec §4.4): the basis for routes (§2.7), junction steering
//! (§2.2.1) and navigation.
//!
//! Vertices are junctions and road ends; edges are road segments between
//! them with their centreline in local metres (x east, z south, see the
//! crate docs). Pure data and algorithms; built by `ocw-worldgen`.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Stable edge identifier: derived from the OSM way, its node range and a
/// geometry hash, so it survives rebuilds from newer OSM data (spec §4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EdgeId(pub u64);

/// Index of a vertex in a [`RoadGraph`] (not stable across builds).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VertexId(pub u32);

/// Road category, ordered roughly by size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RoadClass {
    Trunk,
    Primary,
    Secondary,
    Tertiary,
    Unclassified,
    Residential,
    Service,
    Track,
    Cycleway,
    Path,
}

impl RoadClass {
    /// Typical carriageway width in metres (used when OSM has no width).
    pub fn default_width_m(self) -> f64 {
        match self {
            Self::Trunk | Self::Primary => 7.5,
            Self::Secondary => 6.5,
            Self::Tertiary => 6.0,
            Self::Unclassified | Self::Residential => 5.0,
            Self::Service => 3.5,
            Self::Track => 3.0,
            Self::Cycleway => 2.5,
            Self::Path => 1.8,
        }
    }
}

/// One-way restriction of an edge for bicycles (OSM `oneway`, unless
/// `oneway:bicycle=no`; roundabouts are one-way).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Oneway {
    /// Rideable in both directions.
    #[default]
    No,
    /// Only from `from` to `to`.
    Forward,
    /// Only from `to` to `from`.
    Backward,
}

impl Oneway {
    /// The same restriction for the edge with `from` and `to` swapped.
    pub fn reversed(self) -> Self {
        match self {
            Self::No => Self::No,
            Self::Forward => Self::Backward,
            Self::Backward => Self::Forward,
        }
    }

    /// Whether the edge may be ridden in this direction (`forward` = from
    /// `from` to `to`).
    pub fn allows(self, forward: bool) -> bool {
        match self {
            Self::No => true,
            Self::Forward => forward,
            Self::Backward => !forward,
        }
    }
}

/// A road segment between two vertices.
#[derive(Debug, Clone, PartialEq)]
pub struct Edge {
    pub id: EdgeId,
    pub from: VertexId,
    pub to: VertexId,
    /// Centreline from `from` to `to`, local metres (x, z).
    pub points: Vec<[f64; 2]>,
    pub class: RoadClass,
    pub name: Option<String>,
    pub width_m: f64,
    pub bridge: bool,
    pub tunnel: bool,
    /// Distance to the end of the dead-end subtree this edge leads into,
    /// measured from its `from` vertex towards the end (spec §4.4, step 3).
    pub dead_end_m: Option<f64>,
    /// Cut by the region boundary (spec §4.4, step 4).
    pub boundary: bool,
    /// One-way for bicycles (spec §2.2.1: no exits against it).
    pub oneway: Oneway,
    /// OSM way this edge comes from (for diagnostics and re-matching).
    pub osm_way: i64,
    /// Smoothed road elevation every [`PROFILE_STEP_M`] from `from` to `to`
    /// (last sample at the end); empty until the elevation step ran.
    pub profile: Vec<f64>,
}

/// FNV-1a, 64 bit: a small, stable content hash (not cryptographic; the
/// package signature, spec §4.7, covers tampering).
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Spacing of [`Edge::profile`] samples (spec §4.3: every 5 m).
pub const PROFILE_STEP_M: f64 = 5.0;

impl Edge {
    pub fn length_m(&self) -> f64 {
        self.points
            .windows(2)
            .map(|w| ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt())
            .sum()
    }

    /// Road elevation at `distance` metres from `from` (linear between
    /// profile samples); `None` without a profile.
    pub fn height_at(&self, distance: f64) -> Option<f64> {
        let p = &self.profile;
        if p.is_empty() {
            return None;
        }
        let len = self.length_m();
        let d = distance.clamp(0.0, len);
        let f = d / PROFILE_STEP_M;
        let i = (f.floor() as usize).min(p.len() - 1);
        if i + 1 >= p.len() {
            return p.last().copied();
        }
        // The last interval may be shorter than PROFILE_STEP_M.
        let seg = (len - i as f64 * PROFILE_STEP_M).clamp(1e-9, PROFILE_STEP_M);
        let t = ((d - i as f64 * PROFILE_STEP_M) / seg).clamp(0.0, 1.0);
        Some(p[i] + (p[i + 1] - p[i]) * t)
    }

    /// The other end of the edge.
    pub fn other(&self, v: VertexId) -> VertexId {
        if v == self.from { self.to } else { self.from }
    }
}

/// Road graph of one region.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RoadGraph {
    /// Vertex positions, local metres (x, z).
    pub vertices: Vec<[f64; 2]>,
    pub edges: Vec<Edge>,
    /// Vertices where a road was cut by the region boundary: the road
    /// continues in the neighbouring region there.
    pub boundary_vertices: BTreeSet<VertexId>,
}

impl RoadGraph {
    /// Edge indices touching each vertex.
    pub fn adjacency(&self) -> Vec<Vec<usize>> {
        let mut adj = vec![Vec::new(); self.vertices.len()];
        for (i, e) in self.edges.iter().enumerate() {
            adj[e.from.0 as usize].push(i);
            if e.to != e.from {
                adj[e.to.0 as usize].push(i);
            }
        }
        adj
    }

    /// Fingerprint of the graph's content (FNV-1a 64 of its package
    /// encoding): client and server must ride the same graph, since
    /// junctions are vertex indices (spec §6.5); it is sent in the Welcome.
    pub fn fingerprint(&self) -> u64 {
        let mut bytes = Vec::new();
        crate::package::write_graph(self, &mut bytes);
        fnv1a(&bytes)
    }

    pub fn total_length_m(&self) -> f64 {
        self.edges.iter().map(Edge::length_m).sum()
    }

    /// Connected components as sets of edge indices, largest first.
    pub fn components(&self) -> Vec<Vec<usize>> {
        let adj = self.adjacency();
        let mut seen = vec![false; self.vertices.len()];
        let mut out = Vec::new();
        for start in 0..self.vertices.len() {
            if seen[start] || adj[start].is_empty() {
                continue;
            }
            let mut edges = BTreeSet::new();
            let mut queue = VecDeque::from([start]);
            seen[start] = true;
            while let Some(v) = queue.pop_front() {
                for &e in &adj[v] {
                    edges.insert(e);
                    let o = self.edges[e].other(VertexId(v as u32)).0 as usize;
                    if !seen[o] {
                        seen[o] = true;
                        queue.push_back(o);
                    }
                }
            }
            out.push(edges.into_iter().collect::<Vec<_>>());
        }
        out.sort_by(|a: &Vec<usize>, b: &Vec<usize>| {
            let len = |c: &Vec<usize>| c.iter().map(|&e| self.edges[e].length_m()).sum::<f64>();
            len(b).total_cmp(&len(a))
        });
        out
    }

    /// Keeps only the given edges and the vertices they use (renumbered).
    pub fn retain_edges(&mut self, keep: &BTreeSet<usize>) {
        let mut map = BTreeMap::new();
        let mut vertices = Vec::new();
        let mut edges = Vec::new();
        for (i, e) in self.edges.iter().enumerate() {
            if !keep.contains(&i) {
                continue;
            }
            let mut remap = |v: VertexId| {
                *map.entry(v).or_insert_with(|| {
                    vertices.push(self.vertices[v.0 as usize]);
                    VertexId(vertices.len() as u32 - 1)
                })
            };
            let mut e = e.clone();
            e.from = remap(e.from);
            e.to = remap(e.to);
            edges.push(e);
        }
        self.boundary_vertices = self
            .boundary_vertices
            .iter()
            .filter_map(|v| map.get(v).copied())
            .collect();
        self.vertices = vertices;
        self.edges = edges;
    }

    /// Vertex closest to `p` (local metres).
    pub fn nearest_vertex(&self, p: [f64; 2]) -> Option<VertexId> {
        self.vertices
            .iter()
            .enumerate()
            .min_by(|a, b| {
                let d = |v: &[f64; 2]| (v[0] - p[0]).powi(2) + (v[1] - p[1]).powi(2);
                d(a.1).total_cmp(&d(b.1))
            })
            .map(|(i, _)| VertexId(i as u32))
    }

    /// Cheapest path from `from` to `to` (Dijkstra) as edges with their
    /// direction (`true` = from `edge.from` to `edge.to`). `cost` gives the
    /// cost of riding an edge (e.g. its length, weighted by road class).
    pub fn shortest_path(
        &self,
        from: VertexId,
        to: VertexId,
        cost: impl Fn(&Edge) -> f64,
    ) -> Option<Vec<(usize, bool)>> {
        use std::cmp::Reverse;
        use std::collections::BinaryHeap;
        #[derive(PartialEq)]
        struct Cost(f64);
        impl Eq for Cost {}
        impl PartialOrd for Cost {
            fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
                Some(self.cmp(other))
            }
        }
        impl Ord for Cost {
            fn cmp(&self, other: &Self) -> std::cmp::Ordering {
                self.0.total_cmp(&other.0)
            }
        }
        let adj = self.adjacency();
        let n = self.vertices.len();
        let mut best = vec![f64::INFINITY; n];
        let mut came: Vec<Option<(usize, usize)>> = vec![None; n]; // (edge, previous vertex)
        let mut heap = BinaryHeap::new();
        best[from.0 as usize] = 0.0;
        heap.push(Reverse((Cost(0.0), from.0 as usize)));
        while let Some(Reverse((Cost(c), v))) = heap.pop() {
            if v == to.0 as usize {
                break;
            }
            if c > best[v] {
                continue;
            }
            for &e in &adj[v] {
                let o = self.edges[e].other(VertexId(v as u32)).0 as usize;
                let nc = c + cost(&self.edges[e]).max(0.0);
                if nc < best[o] {
                    best[o] = nc;
                    came[o] = Some((e, v));
                    heap.push(Reverse((Cost(nc), o)));
                }
            }
        }
        if !best[to.0 as usize].is_finite() {
            return None;
        }
        let mut path = Vec::new();
        let mut v = to.0 as usize;
        while let Some((e, prev)) = came[v] {
            path.push((e, self.edges[e].from.0 as usize == prev));
            v = prev;
        }
        path.reverse();
        Some(path)
    }

    /// Keeps only the largest connected component (spec §4.4, step 1).
    pub fn keep_largest_component(&mut self) {
        if let Some(largest) = self.components().into_iter().next() {
            self.retain_edges(&largest.into_iter().collect());
        }
    }

    /// Repeatedly removes dead-end edges shorter than `min_m` (short spurs,
    /// spec §4.4, step 2). Boundary edges are kept: they continue in the
    /// neighbouring region.
    pub fn prune_spurs(&mut self, min_m: f64) {
        loop {
            let adj = self.adjacency();
            let spurs: BTreeSet<usize> = self
                .edges
                .iter()
                .enumerate()
                .filter(|(_, e)| {
                    !e.boundary
                        && e.length_m() < min_m
                        && (adj[e.from.0 as usize].len() == 1) != (adj[e.to.0 as usize].len() == 1)
                })
                .map(|(i, _)| i)
                .collect();
            if spurs.is_empty() {
                return;
            }
            let keep = (0..self.edges.len())
                .filter(|i| !spurs.contains(i))
                .collect();
            self.retain_edges(&keep);
        }
    }

    /// Flags edges that lead only into dead-end subtrees (spec §4.4,
    /// step 3) with the distance to the farthest end. Leaves are peeled off
    /// repeatedly; boundary edges count as continuing roads, not dead ends.
    pub fn flag_dead_ends(&mut self) {
        let adj = self.adjacency();
        let n = self.vertices.len();
        let mut degree: Vec<usize> = adj.iter().map(Vec::len).collect();
        let mut removed = vec![false; self.edges.len()];
        // Distance from a vertex to the farthest end of the subtree peeled
        // off behind it.
        let mut depth = vec![0.0f64; n];
        let boundary_vertex: Vec<bool> = (0..n)
            .map(|v| self.boundary_vertices.contains(&VertexId(v as u32)))
            .collect();
        let mut queue: VecDeque<usize> = (0..n)
            .filter(|&v| degree[v] == 1 && !boundary_vertex[v])
            .collect();
        while let Some(leaf) = queue.pop_front() {
            let Some(&e) = adj[leaf].iter().find(|&&e| !removed[e]) else {
                continue;
            };
            removed[e] = true;
            let edge = &mut self.edges[e];
            let len = edge.length_m();
            let parent = edge.other(VertexId(leaf as u32)).0 as usize;
            // Oriented from the parent towards the dead end.
            let dist = depth[leaf] + len;
            edge.dead_end_m = Some(dist);
            if edge.from.0 as usize != parent {
                std::mem::swap(&mut edge.from, &mut edge.to);
                edge.points.reverse();
                edge.profile.reverse();
                edge.oneway = edge.oneway.reversed();
            }
            depth[parent] = depth[parent].max(dist);
            degree[parent] -= 1;
            if degree[parent] == 1 && !boundary_vertex[parent] {
                queue.push_back(parent);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a graph from straight edges between named vertex positions.
    pub(crate) fn graph(vertices: &[[f64; 2]], edges: &[(u32, u32)]) -> RoadGraph {
        RoadGraph {
            vertices: vertices.to_vec(),
            edges: edges
                .iter()
                .enumerate()
                .map(|(i, &(a, b))| Edge {
                    id: EdgeId(i as u64),
                    from: VertexId(a),
                    to: VertexId(b),
                    points: vec![vertices[a as usize], vertices[b as usize]],
                    class: RoadClass::Residential,
                    name: None,
                    width_m: 5.0,
                    bridge: false,
                    tunnel: false,
                    dead_end_m: None,
                    oneway: Oneway::No,
                    boundary: false,
                    osm_way: i as i64,
                    profile: Vec::new(),
                })
                .collect(),
            boundary_vertices: BTreeSet::new(),
        }
    }

    #[test]
    fn largest_component_is_kept() {
        // A 300 m triangle and a separate 10 m stub.
        let mut g = graph(
            &[
                [0.0, 0.0],
                [100.0, 0.0],
                [50.0, 86.6],
                [500.0, 0.0],
                [510.0, 0.0],
            ],
            &[(0, 1), (1, 2), (2, 0), (3, 4)],
        );
        assert_eq!(g.components().len(), 2);
        g.keep_largest_component();
        assert_eq!(g.edges.len(), 3);
        assert_eq!(g.vertices.len(), 3);
        assert!(
            g.edges
                .iter()
                .all(|e| (e.from.0 as usize) < 3 && (e.to.0 as usize) < 3)
        );
    }

    #[test]
    fn short_spurs_are_pruned_repeatedly_but_boundary_edges_stay() {
        // Square loop with a 20 m spur that itself has a 20 m spur, a 200 m
        // dead-end road, and a short boundary edge.
        let mut g = graph(
            &[
                [0.0, 0.0],
                [100.0, 0.0],
                [100.0, 100.0],
                [0.0, 100.0],
                [120.0, 0.0],
                [140.0, 0.0],
                [0.0, -200.0],
                [-10.0, 100.0],
            ],
            &[
                (0, 1),
                (1, 2),
                (2, 3),
                (3, 0),
                (1, 4),
                (4, 5),
                (0, 6),
                (3, 7),
            ],
        );
        g.edges[7].boundary = true;
        g.prune_spurs(50.0);
        assert_eq!(
            g.edges.len(),
            6,
            "{:?}",
            g.edges.iter().map(|e| e.length_m()).collect::<Vec<_>>()
        );
        assert!(g.edges.iter().any(|e| e.boundary));
        assert!(g.edges.iter().any(|e| (e.length_m() - 200.0).abs() < 1e-9));
    }

    #[test]
    fn dead_end_subtrees_are_flagged_with_their_depth() {
        // Loop 0-1-2-3 with a valley road 1→4 (300 m) that forks into 4→5
        // (120 m) and 4→6 (250 m).
        let mut g = graph(
            &[
                [0.0, 0.0],
                [100.0, 0.0],
                [100.0, 100.0],
                [0.0, 100.0],
                [400.0, 0.0],
                [520.0, 0.0],
                [400.0, 250.0],
            ],
            &[(0, 1), (1, 2), (2, 3), (3, 0), (4, 1), (4, 5), (4, 6)],
        );
        g.flag_dead_ends();
        let by_len = |len: f64| {
            g.edges
                .iter()
                .find(|e| (e.length_m() - len).abs() < 1e-9)
                .unwrap()
        };
        // The valley road: 300 m plus the longer branch (250 m).
        let valley = by_len(300.0);
        assert_eq!(valley.dead_end_m, Some(550.0));
        // Oriented away from the loop.
        assert_eq!(valley.from, VertexId(1));
        assert_eq!(by_len(250.0).dead_end_m, Some(250.0));
        assert_eq!(by_len(120.0).dead_end_m, Some(120.0));
        let loop_edges = g.edges.iter().filter(|e| e.dead_end_m.is_none()).count();
        assert_eq!(loop_edges, 4);
    }

    #[test]
    fn boundary_vertices_stop_the_peeling() {
        // A road from the border (vertex 2) into the region, ending at 0:
        // seen from the border it is a 200 m dead end.
        let mut g = graph(&[[0.0, 0.0], [100.0, 0.0], [200.0, 0.0]], &[(0, 1), (1, 2)]);
        g.edges[1].boundary = true;
        g.boundary_vertices.insert(VertexId(2));
        g.flag_dead_ends();
        assert_eq!(g.edges[0].dead_end_m, Some(100.0));
        assert_eq!(g.edges[1].dead_end_m, Some(200.0));
        assert_eq!(
            g.edges[1].from,
            VertexId(2),
            "oriented from the border inwards"
        );
        // A road crossing the region (both ends at the border) with a side
        // road: the through road is not a dead end, the side road is.
        let mut g = graph(
            &[[0.0, 0.0], [100.0, 0.0], [200.0, 0.0], [100.0, 80.0]],
            &[(0, 1), (1, 2), (1, 3)],
        );
        g.boundary_vertices.extend([VertexId(0), VertexId(2)]);
        g.flag_dead_ends();
        assert_eq!(g.edges[0].dead_end_m, None);
        assert_eq!(g.edges[1].dead_end_m, None);
        assert_eq!(g.edges[2].dead_end_m, Some(80.0));
    }

    #[test]
    fn height_along_an_edge() {
        let mut g = graph(&[[0.0, 0.0], [12.0, 0.0]], &[(0, 1)]);
        let e = &mut g.edges[0];
        assert_eq!(e.height_at(3.0), None);
        // 12 m: samples at 0, 5, 10 and the end (12 m).
        e.profile = vec![100.0, 101.0, 102.0, 103.0];
        assert_eq!(e.height_at(0.0), Some(100.0));
        assert_eq!(e.height_at(7.5), Some(101.5));
        assert_eq!(e.height_at(11.0), Some(102.5));
        assert_eq!(e.height_at(12.0), Some(103.0));
        assert_eq!(e.height_at(99.0), Some(103.0));
    }

    #[test]
    fn shortest_path_prefers_cheap_edges() {
        // Square 0-1-2-3 plus a diagonal 0-2 that is shorter but expensive.
        let mut g = graph(
            &[[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]],
            &[(0, 1), (1, 2), (2, 3), (3, 0), (2, 0)],
        );
        g.edges[4].class = RoadClass::Path;
        let by_length = g
            .shortest_path(VertexId(0), VertexId(2), Edge::length_m)
            .unwrap();
        assert_eq!(
            by_length,
            vec![(4, false)],
            "diagonal, ridden from 0 to 2 (against its direction)"
        );
        let avoid_paths =
            |e: &Edge| e.length_m() * if e.class == RoadClass::Path { 5.0 } else { 1.0 };
        let path = g
            .shortest_path(VertexId(0), VertexId(2), avoid_paths)
            .unwrap();
        assert_eq!(path.len(), 2);
        assert!(path.iter().all(|(e, _)| *e != 4));
        assert_eq!(
            g.shortest_path(VertexId(1), VertexId(1), Edge::length_m),
            Some(vec![])
        );
        assert_eq!(g.nearest_vertex([90.0, 95.0]), Some(VertexId(2)));
        // Unreachable.
        let mut g2 = graph(&[[0.0, 0.0], [1.0, 0.0], [5.0, 0.0]], &[(0, 1)]);
        g2.vertices.push([9.0, 9.0]);
        assert_eq!(
            g2.shortest_path(VertexId(0), VertexId(2), Edge::length_m),
            None
        );
    }
}
