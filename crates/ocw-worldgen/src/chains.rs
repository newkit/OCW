//! Through-roads (spec §4.3, step 3): OSM splits one road into many ways
//! (at every change of a tag, at bridges, at side roads). For the
//! elevation filter the pieces are joined back into chains wherever the
//! road continues straight on, so gradients run smoothly across the
//! splits instead of being filtered piece by piece.

use std::collections::HashMap;

use ocw_world::graph::{Edge, RoadGraph};

/// Largest turn (degrees from straight on) at which a road continues.
const MAX_TURN_DEG: f64 = 50.0;
/// Same name: the road also continues through sharper bends.
const MAX_TURN_SAME_NAME_DEG: f64 = 75.0;
/// How far along the edge the leaving direction is measured.
const DIRECTION_M: f64 = 15.0;

/// One end of an edge: (edge index, `true` = the `to` end).
type End = (usize, bool);

/// Direction in which `e` leaves the vertex at its `at_to` end.
fn leaving(e: &Edge, at_to: bool) -> [f64; 2] {
    let pts: Vec<[f64; 2]> = if at_to {
        e.points.iter().rev().copied().collect()
    } else {
        e.points.clone()
    };
    let o = pts[0];
    let mut walked = 0.0;
    let mut p = pts[pts.len() - 1];
    for w in pts.windows(2) {
        walked += ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt();
        if walked >= DIRECTION_M {
            p = w[1];
            break;
        }
    }
    let d = [p[0] - o[0], p[1] - o[1]];
    let l = (d[0] * d[0] + d[1] * d[1]).sqrt().max(1e-9);
    [d[0] / l, d[1] / l]
}

/// Chains of edges `(edge index, forward)` that form continuous roads.
/// Every edge accepted by `include` is in exactly one chain; bridges and
/// tunnels can be left out by the caller. Chains are sorted with the
/// largest road class first (then longest), the order in which their
/// heights should be fixed.
pub fn through_chains(
    graph: &RoadGraph,
    include: impl Fn(&Edge) -> bool,
) -> Vec<Vec<(usize, bool)>> {
    // Ends at each vertex.
    let mut ends: HashMap<u32, Vec<End>> = HashMap::new();
    for (i, e) in graph.edges.iter().enumerate() {
        if !include(e) {
            continue;
        }
        ends.entry(e.from.0).or_default().push((i, false));
        ends.entry(e.to.0).or_default().push((i, true));
    }
    // Pair the ends at each vertex: straightest continuation first.
    let mut partner: HashMap<End, End> = HashMap::new();
    let mut vertices: Vec<_> = ends.keys().copied().collect();
    vertices.sort_unstable();
    for v in vertices {
        let list = &ends[&v];
        let mut pairs = Vec::new();
        for (x, &a) in list.iter().enumerate() {
            for &b in &list[x + 1..] {
                if a.0 == b.0 {
                    continue; // both ends of a loop
                }
                let (ea, eb) = (&graph.edges[a.0], &graph.edges[b.0]);
                let (da, db) = (leaving(ea, a.1), leaving(eb, b.1));
                // Straight on: the two leave in opposite directions.
                let cos = (-(da[0] * db[0] + da[1] * db[1])).clamp(-1.0, 1.0);
                let turn = cos.acos().to_degrees();
                let same_name = ea.name.is_some() && ea.name == eb.name;
                let limit = if same_name {
                    MAX_TURN_SAME_NAME_DEG
                } else {
                    MAX_TURN_DEG
                };
                if turn > limit {
                    continue;
                }
                let class_gap = (ea.class as i32 - eb.class as i32).abs() as f64;
                let score = turn + 10.0 * class_gap - if same_name { 30.0 } else { 0.0 };
                pairs.push((score, a, b));
            }
        }
        pairs.sort_by(|x, y| x.0.total_cmp(&y.0).then((x.1, x.2).cmp(&(y.1, y.2))));
        for (_, a, b) in pairs {
            if !partner.contains_key(&a) && !partner.contains_key(&b) {
                partner.insert(a, b);
                partner.insert(b, a);
            }
        }
    }
    // Walk the chains.
    let mut done = vec![false; graph.edges.len()];
    let mut chains = Vec::new();
    for start in 0..graph.edges.len() {
        if done[start] || !include(&graph.edges[start]) {
            continue;
        }
        // Back to the first edge (or once around a closed loop).
        let mut first = (start, true);
        loop {
            let entry = (first.0, !first.1);
            match partner.get(&entry) {
                Some(&(e, at_to)) if e != start => first = (e, at_to),
                _ => break,
            }
        }
        let mut chain = vec![first];
        done[first.0] = true;
        loop {
            let &(e, fwd) = chain.last().expect("non-empty");
            match partner.get(&(e, fwd)) {
                Some(&(n, at_to)) if !done[n] => {
                    done[n] = true;
                    chain.push((n, !at_to));
                }
                _ => break,
            }
        }
        chains.push(chain);
    }
    let key = |c: &Vec<(usize, bool)>| {
        let class = c.iter().map(|&(e, _)| graph.edges[e].class).min();
        let len: f64 = c.iter().map(|&(e, _)| graph.edges[e].length_m()).sum();
        (class, -len)
    };
    chains.sort_by(|a, b| {
        let (ka, kb) = (key(a), key(b));
        ka.0.cmp(&kb.0).then(ka.1.total_cmp(&kb.1))
    });
    chains
}

/// The vertices along a chain, from its start to its end.
pub fn chain_vertices(graph: &RoadGraph, chain: &[(usize, bool)]) -> Vec<u32> {
    let mut out = Vec::with_capacity(chain.len() + 1);
    for (k, &(e, fwd)) in chain.iter().enumerate() {
        let e = &graph.edges[e];
        let (a, b) = if fwd { (e.from, e.to) } else { (e.to, e.from) };
        if k == 0 {
            out.push(a.0);
        }
        out.push(b.0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_world::graph::{EdgeId, RoadClass, VertexId};

    pub(crate) fn edge(id: u64, from: u32, to: u32, points: Vec<[f64; 2]>, name: &str) -> Edge {
        Edge {
            id: EdgeId(id),
            from: VertexId(from),
            to: VertexId(to),
            points,
            class: RoadClass::Secondary,
            name: (!name.is_empty()).then(|| name.to_string()),
            width_m: 6.5,
            bridge: false,
            tunnel: false,
            dead_end_m: None,
            oneway: ocw_world::graph::Oneway::No,
            boundary: false,
            osm_way: id as i64,
            profile: vec![],
        }
    }

    #[test]
    fn straight_pieces_join_and_side_roads_stay_apart() {
        // Main road west-east through v1 and v2 (split in three, middle
        // piece reversed); a side road leaves v1 to the north.
        let v = [
            [0.0, 0.0],
            [100.0, 0.0],
            [200.0, 5.0],
            [300.0, 0.0],
            [100.0, -100.0],
        ];
        let g = RoadGraph {
            vertices: v.to_vec(),
            edges: vec![
                edge(1, 0, 1, vec![v[0], v[1]], "Main"),
                edge(2, 2, 1, vec![v[2], v[1]], "Main"),
                edge(3, 2, 3, vec![v[2], v[3]], ""),
                edge(4, 1, 4, vec![v[1], v[4]], "Side"),
            ],
            boundary_vertices: Default::default(),
        };
        let chains = through_chains(&g, |_| true);
        assert_eq!(chains.len(), 2);
        let main = chains.iter().find(|c| c.len() == 3).expect("main road");
        let order: Vec<usize> = main.iter().map(|c| c.0).collect();
        assert!(order == [0, 1, 2] || order == [2, 1, 0], "{order:?}");
        let vs = chain_vertices(&g, main);
        assert!(vs == [0, 1, 2, 3] || vs == [3, 2, 1, 0], "{vs:?}");
        assert!(
            chains
                .iter()
                .any(|c| c == &vec![(3, true)] || c == &vec![(3, false)])
        );
    }

    #[test]
    fn closed_loops_and_excluded_edges() {
        let v = [[0.0, 0.0], [100.0, 0.0], [50.0, 80.0]];
        let mut g = RoadGraph {
            vertices: v.to_vec(),
            edges: vec![
                edge(1, 0, 1, vec![v[0], v[1]], "Ring"),
                edge(2, 1, 2, vec![v[1], v[2]], "Ring"),
                edge(3, 2, 0, vec![v[2], v[0]], "Ring"),
            ],
            boundary_vertices: Default::default(),
        };
        let chains = through_chains(&g, |_| true);
        assert_eq!(chains.iter().map(Vec::len).sum::<usize>(), 3);
        g.edges[1].bridge = true;
        let chains = through_chains(&g, |e| !e.bridge);
        assert!(chains.iter().all(|c| c.iter().all(|&(e, _)| e != 1)));
        assert_eq!(chains.iter().map(Vec::len).sum::<usize>(), 2);
    }
}
