//! Region borders between neighbouring packages (spec §2.2.2, §4.4 step 4).
//!
//! A road cut by a package's boundary ends at a *boundary vertex*; the
//! neighbouring package has the same road cut at the same place. Matching
//! the two ends links the packages: [`match_border`] pairs boundary
//! vertices of two graphs that lie at the same geographic point (within
//! [`LINK_TOLERANCE_M`]) on the same road (same OSM way, or the same stable
//! edge ID). [`join_graphs`] then builds one road graph in the first
//! package's local metres from both, with the linked vertices merged and
//! no longer marked as boundary – a free-ride path simply continues across
//! the border instead of turning round ("End of region").
//!
//! Each package keeps its own projection (spec §4.5); points of the second
//! package are converted through latitude / longitude.

use std::collections::BTreeMap;

use crate::geo::LocalProjection;
use crate::graph::{EdgeId, RoadGraph, VertexId};

/// Boundary vertices of two packages closer than this are the same point.
pub const LINK_TOLERANCE_M: f64 = 8.0;

/// One road crossing the border: a boundary vertex in each package.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BorderLink {
    /// Boundary vertex in the first package and its edge.
    pub here: VertexId,
    pub here_edge: usize,
    /// The same point in the neighbouring package and its edge.
    pub there: VertexId,
    pub there_edge: usize,
    /// Distance between the two ends (m, in the first package's metres).
    pub gap_m: f64,
}

/// A package's road graph with the projection centre of its local metres.
#[derive(Debug, Clone, Copy)]
pub struct Placed<'a> {
    pub graph: &'a RoadGraph,
    /// (lat, lon) of the local origin.
    pub origin: (f64, f64),
}

impl Placed<'_> {
    /// A local point of this package in the local metres of `other`.
    fn in_frame_of(&self, p: [f64; 2], other: (f64, f64)) -> [f64; 2] {
        if self.origin == other {
            return p;
        }
        let (lat, lon) = LocalProjection::new(self.origin.0, self.origin.1).unproject(p);
        LocalProjection::new(other.0, other.1).project(lat, lon)
    }

    /// The edge ending at each boundary vertex (the cut road): the first
    /// boundary edge there, else the first edge. One pass over the edges
    /// (a scan per vertex pair made joining two 100,000-edge packages take
    /// a minute).
    fn boundary_edges(&self) -> BTreeMap<VertexId, usize> {
        let mut out: BTreeMap<VertexId, (usize, bool)> = BTreeMap::new();
        for (i, e) in self.graph.edges.iter().enumerate() {
            for v in [e.from, e.to] {
                if !self.graph.boundary_vertices.contains(&v) {
                    continue;
                }
                match out.get(&v) {
                    Some(&(_, true)) => {}
                    Some(&(_, false)) if !e.boundary => {}
                    _ => {
                        out.insert(v, (i, e.boundary));
                    }
                }
            }
        }
        out.into_iter().map(|(v, (i, _))| (v, i)).collect()
    }
}

/// Pairs the boundary vertices of `a` and `b` that are the same road at the
/// same point. Each vertex is linked at most once (the nearest match).
pub fn match_border(a: Placed, b: Placed) -> Vec<BorderLink> {
    let mut candidates: Vec<BorderLink> = Vec::new();
    let (edges_a, edges_b) = (a.boundary_edges(), b.boundary_edges());
    for &va in &a.graph.boundary_vertices {
        let Some(&ea) = edges_a.get(&va) else {
            continue;
        };
        let pa = a.graph.vertices[va.0 as usize];
        let way_a = &a.graph.edges[ea];
        for &vb in &b.graph.boundary_vertices {
            let Some(&eb) = edges_b.get(&vb) else {
                continue;
            };
            let way_b = &b.graph.edges[eb];
            if way_a.osm_way != way_b.osm_way && way_a.id != way_b.id {
                continue;
            }
            let pb = b.in_frame_of(b.graph.vertices[vb.0 as usize], a.origin);
            let gap_m = ((pa[0] - pb[0]).powi(2) + (pa[1] - pb[1]).powi(2)).sqrt();
            if gap_m <= LINK_TOLERANCE_M {
                candidates.push(BorderLink {
                    here: va,
                    here_edge: ea,
                    there: vb,
                    there_edge: eb,
                    gap_m,
                });
            }
        }
    }
    candidates.sort_by(|x, y| x.gap_m.total_cmp(&y.gap_m));
    let mut out: Vec<BorderLink> = Vec::new();
    for c in candidates {
        if out.iter().all(|l| l.here != c.here && l.there != c.there) {
            out.push(c);
        }
    }
    out.sort_by_key(|l| (l.here, l.there));
    out
}

/// One graph from `a` and its neighbour `b`, in `a`'s local metres: `b`'s
/// vertices are converted, linked vertices merged (they stop being
/// boundary vertices; the cut edges stay as two edges meeting there).
/// Edges of `b` with a stable ID that `a` already has are left out (a road
/// both packages contain whole).
pub fn join_graphs(a: Placed, b: Placed, links: &[BorderLink]) -> RoadGraph {
    let mut g = a.graph.clone();
    let linked: BTreeMap<VertexId, VertexId> = links.iter().map(|l| (l.there, l.here)).collect();
    let mut map: Vec<VertexId> = Vec::with_capacity(b.graph.vertices.len());
    for (i, v) in b.graph.vertices.iter().enumerate() {
        let vb = VertexId(i as u32);
        match linked.get(&vb) {
            Some(&va) => map.push(va),
            None => {
                g.vertices.push(b.in_frame_of(*v, a.origin));
                map.push(VertexId(g.vertices.len() as u32 - 1));
            }
        }
    }
    let known: std::collections::BTreeSet<EdgeId> = g.edges.iter().map(|e| e.id).collect();
    for e in &b.graph.edges {
        if known.contains(&e.id) {
            continue;
        }
        let mut e = e.clone();
        let ends = (linked.contains_key(&e.from), linked.contains_key(&e.to));
        e.from = map[e.from.0 as usize];
        e.to = map[e.to.0 as usize];
        for p in &mut e.points {
            *p = b.in_frame_of(*p, a.origin);
        }
        // The road meets the linked vertex exactly (the two cut ends may be
        // up to LINK_TOLERANCE_M apart).
        if let (true, Some(p)) = (ends.0, e.points.first_mut()) {
            *p = g.vertices[e.from.0 as usize];
        }
        if let (true, Some(p)) = (ends.1, e.points.last_mut()) {
            *p = g.vertices[e.to.0 as usize];
        }
        g.edges.push(e);
    }
    for l in links {
        g.boundary_vertices.remove(&l.here);
        g.edges[l.here_edge].boundary = false;
    }
    // The neighbour's own outer border stays a border.
    for &vb in &b.graph.boundary_vertices {
        if !linked.contains_key(&vb) {
            g.boundary_vertices.insert(map[vb.0 as usize]);
        }
    }
    // Edges of b that were cut at a linked vertex are whole now.
    let joined: std::collections::BTreeSet<VertexId> = links.iter().map(|l| l.here).collect();
    let base = a.graph.edges.len();
    for e in g.edges.iter_mut().skip(base) {
        if joined.contains(&e.from) || joined.contains(&e.to) {
            e.boundary = false;
        }
    }
    g
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::graph::Edge;
    use crate::network::RoadNetwork;
    use crate::network::tests::graph;
    use crate::path::{RidePath, TurnReason};
    use crate::track::Track;

    /// A road from x = -600 to +600 along z = 0 (one OSM way, 77) with side
    /// roads at x = ±300, cut at the border x = 0 into two packages.
    /// Package B has its own origin 0.01° further east (its local x is
    /// shifted by ~760 m).
    fn split() -> (RoadGraph, (f64, f64), RoadGraph, (f64, f64)) {
        let oa = (47.1, 9.5);
        let ob = (47.1, 9.51);
        // A: vertices west of the border (A's metres).
        let mut a = graph(
            &[[-600.0, 0.0], [-300.0, 0.0], [0.0, 0.0], [-300.0, 300.0]],
            &[(0, 1), (1, 2), (1, 3)],
        );
        a.edges[0].osm_way = 77;
        a.edges[1].osm_way = 77;
        a.edges[1].boundary = true;
        a.edges[1].id = EdgeId(5001);
        a.boundary_vertices.insert(VertexId(2));
        // B: east of the border, in B's metres.
        let to_b = |x: f64, z: f64| {
            let (lat, lon) = LocalProjection::new(oa.0, oa.1).unproject([x, z]);
            LocalProjection::new(ob.0, ob.1).project(lat, lon)
        };
        let vb = [
            to_b(0.0, 0.0),
            to_b(300.0, 0.0),
            to_b(600.0, 0.0),
            to_b(300.0, 300.0),
        ];
        let mut b: RoadGraph = graph(&vb, &[(0, 1), (1, 2), (1, 3)]);
        for (i, e) in b.edges.iter_mut().enumerate() {
            e.id = EdgeId(6000 + i as u64);
        }
        b.edges[0].osm_way = 77;
        b.edges[1].osm_way = 77;
        b.edges[0].boundary = true;
        b.boundary_vertices.insert(VertexId(0));
        // Its east end is B's own border (to a third package).
        b.boundary_vertices.insert(VertexId(2));
        (a, oa, b, ob)
    }

    #[test]
    fn the_edge_at_a_boundary_vertex_prefers_the_cut_road() {
        // Regression (spec v2.34): the edge was found by a scan per vertex
        // pair; the one-pass index must give the same answer: the first
        // boundary edge at the vertex, else the first edge.
        let (a, oa, b, _) = split();
        let pa = Placed {
            graph: &a,
            origin: oa,
        };
        let idx = pa.boundary_edges();
        assert_eq!(idx.len(), 1);
        assert_eq!(idx[&VertexId(2)], 1, "edge 1 is the cut road");
        let mut c = a.clone();
        c.edges[1].boundary = false;
        // A copy of the road (index 3) that is the cut one.
        c.edges.push(c.edges[1].clone());
        c.edges[3].boundary = true;
        let pc = Placed {
            graph: &c,
            origin: oa,
        };
        assert_eq!(
            pc.boundary_edges()[&VertexId(2)],
            3,
            "the boundary edge wins"
        );
        c.edges[3].boundary = false;
        assert_eq!(pc_first(&c, oa), 1, "else the first edge");
        let pb = Placed {
            graph: &b,
            origin: oa,
        };
        assert_eq!(pb.boundary_edges().len(), 2);
    }

    fn pc_first(g: &RoadGraph, origin: (f64, f64)) -> usize {
        Placed { graph: g, origin }.boundary_edges()[&VertexId(2)]
    }

    #[test]
    fn the_cut_road_is_linked_across_the_border() {
        let (a, oa, b, ob) = split();
        let pa = Placed {
            graph: &a,
            origin: oa,
        };
        let pb = Placed {
            graph: &b,
            origin: ob,
        };
        let links = match_border(pa, pb);
        assert_eq!(links.len(), 1, "{links:?}");
        assert_eq!((links[0].here, links[0].there), (VertexId(2), VertexId(0)));
        assert!(links[0].gap_m < 0.01, "{}", links[0].gap_m);
        // Another road (different way) at the same place is not linked.
        let mut other = b.clone();
        for e in &mut other.edges {
            e.osm_way = 99;
        }
        assert!(
            match_border(
                pa,
                Placed {
                    graph: &other,
                    origin: ob
                }
            )
            .is_empty()
        );
        // Too far apart: not the same point.
        let far = Placed {
            graph: &b,
            origin: (47.1, 9.5102),
        };
        assert!(match_border(pa, far).is_empty());
    }

    #[test]
    fn a_free_ride_path_continues_into_the_neighbour() {
        let (a, oa, b, ob) = split();
        let pa = Placed {
            graph: &a,
            origin: oa,
        };
        let pb = Placed {
            graph: &b,
            origin: ob,
        };
        let links = match_border(pa, pb);
        let joined = join_graphs(pa, pb, &links);
        assert_eq!(joined.vertices.len(), 4 + 3, "the border vertex is shared");
        assert_eq!(joined.edges.len(), 6);
        assert!(!joined.boundary_vertices.contains(&VertexId(2)));
        // B's far end stays a border.
        assert_eq!(joined.boundary_vertices.len(), 1);
        let east_end = *joined.boundary_vertices.iter().next().unwrap();
        assert!((joined.vertices[east_end.0 as usize][0] - 600.0).abs() < 0.5);
        // Riding east from A: no "End of region" U-turn at the border any
        // more; the path goes on into B and turns only at B's far border.
        let net = Arc::new(RoadNetwork::new(joined));
        let mut p = RidePath::start(net.clone(), 1, 100.0, true, 0.0, 0);
        p.extend(0.0, &mut |j| j.default);
        // Past the lock point of the junction at x = 300 (path 500 m).
        p.extend(490.0, &mut |j| j.default);
        let (at, reason) = p.turnaround_ahead(0.0).unwrap();
        assert_eq!(reason, TurnReason::Boundary);
        assert!(at > 700.0, "turns at B's far border, not at x = 0: {at}");
        let s = p.sample(400.0);
        assert!(s.position[0] > 150.0, "{s:?}");
        // Without the neighbour the rider turns at the border.
        let alone = Arc::new(RoadNetwork::new(a));
        let mut q = RidePath::start(alone, 1, 100.0, true, 0.0, 0);
        q.extend(0.0, &mut |j| j.default);
        let (at, reason) = q.turnaround_ahead(0.0).unwrap();
        assert_eq!(reason, TurnReason::Boundary);
        assert!(at < 200.0, "{at}");
    }

    #[test]
    fn the_neighbour_road_starts_exactly_at_the_linked_vertex() {
        let (a, oa, mut b, ob) = split();
        // B's cut end lies 3 m off A's (still the same point).
        b.vertices[0][0] += 3.0;
        b.edges[0].points[0][0] += 3.0;
        let pa = Placed {
            graph: &a,
            origin: oa,
        };
        let pb = Placed {
            graph: &b,
            origin: ob,
        };
        let links = match_border(pa, pb);
        assert_eq!(links.len(), 1);
        let joined = join_graphs(pa, pb, &links);
        let e = joined.edges.iter().find(|e| e.id == b.edges[0].id).unwrap();
        assert_eq!(e.points[0], joined.vertices[2]);
    }

    #[test]
    fn edges_both_packages_contain_are_not_doubled() {
        let (a, oa, mut b, ob) = split();
        // B also contains A's side road (same stable ID).
        let mut dup: Edge = a.edges[2].clone();
        dup.from = VertexId(1);
        dup.to = VertexId(3);
        b.edges.push(dup);
        let pa = Placed {
            graph: &a,
            origin: oa,
        };
        let pb = Placed {
            graph: &b,
            origin: ob,
        };
        let joined = join_graphs(pa, pb, &match_border(pa, pb));
        assert_eq!(joined.edges.len(), 6);
    }
}
