//! Rebuild comparison (spec §4.4): which stable edge IDs survive a rebuild
//! from newer OSM data. Routes and saved positions refer to edges by ID, so
//! everything on a kept edge re-matches directly; the rest must be re-matched
//! by geometry or is flagged.

use std::collections::BTreeMap;

use ocw_world::graph::{EdgeId, RoadGraph};

/// How the edges of two builds relate, by stable ID.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GraphDiff {
    /// Same ID, same geometry (within 1 cm).
    pub unchanged: Vec<EdgeId>,
    /// Same ID, geometry moved (nodes moved, same node sequence), with the
    /// largest point offset in metres.
    pub moved: Vec<(EdgeId, f64)>,
    /// Only in the old build (way removed, re-routed or split differently).
    pub removed: Vec<EdgeId>,
    /// Only in the new build.
    pub added: Vec<EdgeId>,
}

impl GraphDiff {
    pub fn between(old: &RoadGraph, new: &RoadGraph) -> Self {
        let index = |g: &RoadGraph| -> BTreeMap<EdgeId, usize> {
            g.edges.iter().enumerate().map(|(i, e)| (e.id, i)).collect()
        };
        let (a, b) = (index(old), index(new));
        let mut d = Self::default();
        for (&id, &i) in &a {
            let Some(&j) = b.get(&id) else {
                d.removed.push(id);
                continue;
            };
            let (pa, pb) = (&old.edges[i].points, &new.edges[j].points);
            let offset = if pa.len() == pb.len() {
                pa.iter()
                    .zip(pb)
                    .map(|(p, q)| ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt())
                    .fold(0.0, f64::max)
            } else {
                f64::INFINITY
            };
            if offset <= 0.01 {
                d.unchanged.push(id);
            } else {
                d.moved.push((id, offset));
            }
        }
        d.added = b.keys().filter(|id| !a.contains_key(id)).copied().collect();
        d
    }

    /// Edges whose ID survived (unchanged or moved).
    pub fn kept(&self) -> usize {
        self.unchanged.len() + self.moved.len()
    }

    /// Which `route` edges (by ID) no longer exist: the route breaks there.
    pub fn broken<'a>(&self, route: &'a [EdgeId]) -> Vec<&'a EdgeId> {
        route
            .iter()
            .filter(|id| self.removed.contains(id))
            .collect()
    }

    /// One-line summary for the build log.
    pub fn summary(&self) -> String {
        format!(
            "{} edges unchanged, {} moved, {} removed, {} added",
            self.unchanged.len(),
            self.moved.len(),
            self.removed.len(),
            self.added.len()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_world::graph::{Edge, Oneway, RoadClass, VertexId};

    fn edge(id: u64, points: Vec<[f64; 2]>) -> Edge {
        Edge {
            id: EdgeId(id),
            from: VertexId(0),
            to: VertexId(1),
            points,
            class: RoadClass::Residential,
            name: None,
            width_m: 5.0,
            bridge: false,
            tunnel: false,
            dead_end_m: None,
            boundary: false,
            oneway: Oneway::No,
            osm_way: 1,
            profile: Vec::new(),
        }
    }

    fn graph(edges: Vec<Edge>) -> RoadGraph {
        RoadGraph {
            vertices: vec![[0.0, 0.0], [10.0, 0.0]],
            edges,
            ..RoadGraph::default()
        }
    }

    #[test]
    fn diff_by_stable_id() {
        let line = vec![[0.0, 0.0], [10.0, 0.0]];
        let old = graph(vec![
            edge(1, line.clone()),
            edge(2, line.clone()),
            edge(3, line.clone()),
        ]);
        let new = graph(vec![
            edge(1, line.clone()),
            edge(2, vec![[0.0, 0.0], [10.0, 3.0]]),
            edge(4, line),
        ]);
        let d = GraphDiff::between(&old, &new);
        assert_eq!(d.unchanged, [EdgeId(1)]);
        assert_eq!(d.moved, [(EdgeId(2), 3.0)]);
        assert_eq!(d.removed, [EdgeId(3)]);
        assert_eq!(d.added, [EdgeId(4)]);
        assert_eq!(d.kept(), 2);
        let route = [EdgeId(1), EdgeId(3), EdgeId(2)];
        assert_eq!(d.broken(&route), [&EdgeId(3)]);
        assert_eq!(
            d.summary(),
            "1 edges unchanged, 1 moved, 1 removed, 1 added"
        );
        assert_eq!(GraphDiff::between(&old, &old).kept(), 3);
    }
}
