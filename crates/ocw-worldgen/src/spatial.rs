//! A bucket grid over the segments of the road graph, for "which roads are
//! near this point" queries of the world lint (spec §4.8) and the cleaning
//! passes. Pure and deterministic (buckets and hits in insertion order).

use std::collections::HashMap;

use ocw_world::graph::RoadGraph;

/// The nearest point of a road edge to a query point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    pub edge: usize,
    /// Distance from the query point to the centreline (m).
    pub dist: f64,
    /// Arc length of the foot point along the edge (m).
    pub along: f64,
    /// The foot point.
    pub foot: [f64; 2],
}

/// Segments of a road graph in square buckets.
pub struct SegmentIndex {
    bucket_m: f64,
    buckets: HashMap<(i64, i64), Vec<(u32, u32)>>,
    /// Cumulative arc length at each point of each edge.
    along: Vec<Vec<f64>>,
}

fn key(p: [f64; 2], bucket_m: f64) -> (i64, i64) {
    (
        (p[0] / bucket_m).floor() as i64,
        (p[1] / bucket_m).floor() as i64,
    )
}

/// Foot point of `p` on segment `a`-`b`: (distance, t in 0..=1, foot).
pub fn foot_on_segment(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> (f64, f64, [f64; 2]) {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let len2 = ab[0] * ab[0] + ab[1] * ab[1];
    let t = if len2 > 0.0 {
        (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let f = [a[0] + ab[0] * t, a[1] + ab[1] * t];
    ((p[0] - f[0]).hypot(p[1] - f[1]), t, f)
}

impl SegmentIndex {
    /// Indexes every edge accepted by `keep` (e.g. no tunnels).
    pub fn new(graph: &RoadGraph, bucket_m: f64, keep: impl Fn(usize) -> bool) -> Self {
        let mut buckets: HashMap<(i64, i64), Vec<(u32, u32)>> = HashMap::new();
        let mut along = Vec::with_capacity(graph.edges.len());
        for (i, e) in graph.edges.iter().enumerate() {
            let mut acc = vec![0.0];
            for (s, w) in e.points.windows(2).enumerate() {
                acc.push(acc[s] + (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]));
                if !keep(i) {
                    continue;
                }
                let (a, b) = (key(w[0], bucket_m), key(w[1], bucket_m));
                for bx in a.0.min(b.0)..=a.0.max(b.0) {
                    for bz in a.1.min(b.1)..=a.1.max(b.1) {
                        buckets
                            .entry((bx, bz))
                            .or_default()
                            .push((i as u32, s as u32));
                    }
                }
            }
            along.push(acc);
        }
        Self {
            bucket_m,
            buckets,
            along,
        }
    }

    /// Arc length of point `k` of edge `e`.
    pub fn along(&self, e: usize, k: usize) -> f64 {
        self.along[e][k]
    }

    /// The nearest point of every indexed edge within `radius` of `p`
    /// (one hit per edge), nearest first.
    pub fn near(&self, graph: &RoadGraph, p: [f64; 2], radius: f64) -> Vec<Hit> {
        let r = (radius / self.bucket_m).ceil() as i64;
        let (kx, kz) = key(p, self.bucket_m);
        let mut best: Vec<Hit> = Vec::new();
        for dx in -r..=r {
            for dz in -r..=r {
                let Some(segs) = self.buckets.get(&(kx + dx, kz + dz)) else {
                    continue;
                };
                for &(ei, si) in segs {
                    let e = &graph.edges[ei as usize];
                    let (a, b) = (e.points[si as usize], e.points[si as usize + 1]);
                    let (d, t, foot) = foot_on_segment(p, a, b);
                    if d > radius {
                        continue;
                    }
                    let along =
                        self.along[ei as usize][si as usize] + t * (b[0] - a[0]).hypot(b[1] - a[1]);
                    let hit = Hit {
                        edge: ei as usize,
                        dist: d,
                        along,
                        foot,
                    };
                    match best.iter_mut().find(|h| h.edge == hit.edge) {
                        Some(h) if h.dist > d => *h = hit,
                        Some(_) => {}
                        None => best.push(hit),
                    }
                }
            }
        }
        best.sort_by(|a, b| a.dist.total_cmp(&b.dist).then(a.edge.cmp(&b.edge)));
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_world::graph::{Edge, EdgeId, Oneway, RoadClass, VertexId};

    pub(crate) fn edge(points: Vec<[f64; 2]>, from: u32, to: u32) -> Edge {
        Edge {
            id: EdgeId(u64::from(from) * 1000 + u64::from(to)),
            from: VertexId(from),
            to: VertexId(to),
            points,
            class: RoadClass::Residential,
            name: None,
            width_m: 5.0,
            bridge: false,
            tunnel: false,
            dead_end_m: None,
            boundary: false,
            oneway: Oneway::No,
            osm_way: i64::from(from),
            profile: Vec::new(),
        }
    }

    #[test]
    fn nearest_points_per_edge() {
        let g = RoadGraph {
            vertices: vec![[0.0, 0.0], [100.0, 0.0], [50.0, 10.0], [50.0, 100.0]],
            edges: vec![
                edge(vec![[0.0, 0.0], [40.0, 0.0], [100.0, 0.0]], 0, 1),
                edge(vec![[50.0, 10.0], [50.0, 100.0]], 2, 3),
            ],
            boundary_vertices: Default::default(),
        };
        let idx = SegmentIndex::new(&g, 10.0, |_| true);
        let hits = idx.near(&g, [50.0, 4.0], 8.0);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].edge, 0);
        assert!((hits[0].dist - 4.0).abs() < 1e-9);
        assert!((hits[0].along - 50.0).abs() < 1e-9);
        assert!((hits[1].dist - 6.0).abs() < 1e-9);
        assert!(idx.near(&g, [50.0, 4.0], 3.0).is_empty());
        let only_first = SegmentIndex::new(&g, 10.0, |e| e == 0);
        assert_eq!(only_first.near(&g, [50.0, 9.0], 8.0).len(), 0);
        assert_eq!(only_first.near(&g, [50.0, 7.0], 8.0).len(), 1);
    }
}
