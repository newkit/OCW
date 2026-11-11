//! Routes through the road graph: a path of edges → route points with the
//! smoothed road elevation (spec §2.7, §4.3).

use ocw_world::Surface;
use ocw_world::graph::{Edge, PROFILE_STEP_M, RoadClass, RoadGraph};

use crate::profile::sample_along;

/// Riding cost of an edge for automatic routes: its length, weighted so
/// main roads are preferred over tracks and paths, and stretches steeper
/// than 12 % are avoided where a gentler road exists.
pub fn road_cost(e: &Edge) -> f64 {
    let steep = (crate::profile::steepest(&e.profile, 1) - 0.12).max(0.0);
    class_cost(e) * (1.0 + 20.0 * steep)
}

fn class_cost(e: &Edge) -> f64 {
    let factor = match e.class {
        RoadClass::Trunk | RoadClass::Primary | RoadClass::Secondary => 1.0,
        RoadClass::Tertiary => 1.1,
        RoadClass::Unclassified | RoadClass::Cycleway => 1.3,
        RoadClass::Residential => 1.5,
        RoadClass::Service => 2.5,
        RoadClass::Track => 4.0,
        RoadClass::Path => 6.0,
    };
    e.length_m() * factor
}

/// Stretches of `path` (from, to metres) whose surface is not asphalt,
/// from the road classes (tracks and paths are gravel).
pub fn path_surfaces(graph: &RoadGraph, path: &[(usize, bool)]) -> Vec<(f64, f64, Surface)> {
    let mut out: Vec<(f64, f64, Surface)> = Vec::new();
    let mut at = 0.0;
    for &(i, _) in path {
        let e = &graph.edges[i];
        let (a, b) = (at, at + e.length_m());
        at = b;
        let s = Surface::of_class(e.class);
        if s == Surface::Asphalt {
            continue;
        }
        match out.last_mut() {
            Some(last) if last.2 == s && (last.1 - a).abs() < 1e-6 => last.1 = b,
            _ => out.push((a, b, s)),
        }
    }
    out
}

/// Points every 5 m along `path` (edges with direction), with the height
/// from each edge's profile: x (east), y (up), z (south).
pub fn path_points(graph: &RoadGraph, path: &[(usize, bool)]) -> Vec<[f64; 3]> {
    let mut out: Vec<[f64; 3]> = Vec::new();
    for &(i, forward) in path {
        let e = &graph.edges[i];
        let mut pts = sample_along(&e.points, PROFILE_STEP_M, false);
        if !forward {
            pts.reverse();
        }
        for (k, &(p, s)) in pts.iter().enumerate() {
            if k == 0 && !out.is_empty() {
                continue; // the junction is already the last point
            }
            let y = e.height_at(s).unwrap_or(0.0);
            out.push([p[0], y, p[1]]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_world::graph::{EdgeId, VertexId};

    fn edge(id: u64, from: u32, to: u32, points: Vec<[f64; 2]>, profile: Vec<f64>) -> Edge {
        Edge {
            id: EdgeId(id),
            from: VertexId(from),
            to: VertexId(to),
            points,
            class: RoadClass::Secondary,
            name: None,
            width_m: 6.5,
            bridge: false,
            tunnel: false,
            dead_end_m: None,
            boundary: false,
            osm_way: 1,
            profile,
        }
    }

    #[test]
    fn points_follow_the_profiles_in_both_directions() {
        // 0 → 1: 10 m east, climbing 0 → 2 m. 2 → 1 (stored backwards): 10 m
        // further east, 4 m at vertex 2, 2 m at vertex 1.
        let g = RoadGraph {
            vertices: vec![[0.0, 0.0], [10.0, 0.0], [20.0, 0.0]],
            edges: vec![
                edge(1, 0, 1, vec![[0.0, 0.0], [10.0, 0.0]], vec![0.0, 1.0, 2.0]),
                edge(2, 2, 1, vec![[20.0, 0.0], [10.0, 0.0]], vec![4.0, 3.0, 2.0]),
            ],
            boundary_vertices: Default::default(),
        };
        let pts = path_points(&g, &[(0, true), (1, false)]);
        let xs: Vec<f64> = pts.iter().map(|p| p[0]).collect();
        let ys: Vec<f64> = pts.iter().map(|p| p[1]).collect();
        assert_eq!(xs, vec![0.0, 5.0, 10.0, 15.0, 20.0]);
        assert_eq!(ys, vec![0.0, 1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn gravel_stretches_follow_the_road_classes() {
        let mut edges: Vec<Edge> = (0..4)
            .map(|i| {
                let x = f64::from(i) * 10.0;
                edge(
                    u64::from(i),
                    i,
                    i + 1,
                    vec![[x, 0.0], [x + 10.0, 0.0]],
                    vec![0.0; 3],
                )
            })
            .collect();
        edges[1].class = RoadClass::Track;
        edges[2].class = RoadClass::Path;
        let g = RoadGraph {
            vertices: (0..5).map(|i| [f64::from(i) * 10.0, 0.0]).collect(),
            edges,
            boundary_vertices: Default::default(),
        };
        let path: Vec<(usize, bool)> = (0..4).map(|i| (i, true)).collect();
        assert_eq!(
            path_surfaces(&g, &path),
            vec![(10.0, 30.0, Surface::Gravel)]
        );
    }

    #[test]
    fn main_roads_are_cheaper() {
        let mut e = edge(1, 0, 1, vec![[0.0, 0.0], [100.0, 0.0]], vec![]);
        let main = road_cost(&e);
        e.class = RoadClass::Track;
        assert!(road_cost(&e) > 3.0 * main);
        assert_eq!(main, 100.0);
        // A 30 % stretch costs extra: 1 + 20 × 0.18.
        e.class = RoadClass::Secondary;
        e.profile = (0..21).map(|i| f64::from(i) * 1.5).collect();
        assert!((road_cost(&e) - 460.0).abs() < 1e-6, "{}", road_cost(&e));
    }
}
