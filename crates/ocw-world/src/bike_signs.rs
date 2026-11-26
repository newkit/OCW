//! Warning signs at the ends of a road closed for bikes (spec 14 Phase 16,
//! 16H Q6: "put warning signs, but let it be ridable"): a world patch's
//! `road` operation with `bike = "closed"` keeps the road drawn and in
//! the rideable network; the package gets a "no bicycles" sign
//! ([`ObstacleKind::NoBikeSign`], section `OBST`) at each end of the
//! closed edges – where they meet a road that is not closed, or end.
//!
//! Each sign stands a few metres into the closed road, on the right of a
//! rider who comes onto it, beside the carriageway (its half width plus
//! [`SIDE_M`]), facing that rider – never in a road: a spot inside any
//! road's carriageway (at a junction) moves further along or to the other
//! side. Pure; the same in the world builder and the tests.

use std::collections::BTreeSet;

use crate::graph::{RoadGraph, VertexId};
use crate::obstacles::{ObstacleKind, PointObstacle};

/// The sign's height (pole and plate, m).
pub const HEIGHT_M: f32 = 2.3;
/// How far into the closed road the sign stands (m).
pub const IN_M: f64 = 4.0;
/// How far beyond the carriageway's edge (m).
pub const SIDE_M: f64 = 0.9;
/// The least room between a sign and any road's carriageway (m).
pub const CLEAR_M: f64 = 0.4;

fn seg_dist(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dz * dz;
    let t = if l2 > 0.0 {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dz) / l2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p[0] - a[0] - t * dx).hypot(p[1] - a[1] - t * dz)
}

/// How far `p` is outside the carriageway of the nearest road (negative:
/// inside one).
pub fn clearance(graph: &RoadGraph, p: [f64; 2]) -> f64 {
    graph
        .edges
        .iter()
        .map(|e| {
            let d = e
                .points
                .windows(2)
                .map(|w| seg_dist(p, w[0], w[1]))
                .fold(f64::INFINITY, f64::min);
            d - e.width_m / 2.0
        })
        .fold(f64::INFINITY, f64::min)
}

/// The point `d` metres along `pts` and the direction there.
fn along(pts: &[[f64; 2]], d: f64) -> ([f64; 2], [f64; 2]) {
    let mut left = d;
    for w in pts.windows(2) {
        let (dx, dz) = (w[1][0] - w[0][0], w[1][1] - w[0][1]);
        let l = dx.hypot(dz);
        if l < 1e-9 {
            continue;
        }
        let t = [dx / l, dz / l];
        if left <= l {
            return ([w[0][0] + t[0] * left, w[0][1] + t[1] * left], t);
        }
        left -= l;
    }
    let n = pts.len();
    let (a, b) = (pts[n.saturating_sub(2)], pts[n - 1]);
    let l = (b[0] - a[0]).hypot(b[1] - a[1]).max(1e-9);
    (b, [(b[0] - a[0]) / l, (b[1] - a[1]) / l])
}

/// The signs for the closed edges `closed` (indices into `graph.edges`).
pub fn place(graph: &RoadGraph, closed: &[usize]) -> Vec<PointObstacle> {
    let set: BTreeSet<usize> = closed.iter().copied().collect();
    let degree = |v: VertexId, only_closed: bool| {
        graph
            .edges
            .iter()
            .enumerate()
            .filter(|(i, e)| (e.from == v || e.to == v) && (!only_closed || set.contains(i)))
            .count()
    };
    let mut out = Vec::new();
    for &i in &set {
        let e = &graph.edges[i];
        if e.points.len() < 2 {
            continue;
        }
        for (v, forward) in [(e.from, true), (e.to, false)] {
            let closed_here = degree(v, true);
            // An end: another road meets the closed one here, or it ends.
            if degree(v, false) == closed_here && closed_here != 1 {
                continue;
            }
            let pts: Vec<[f64; 2]> = if forward {
                e.points.clone()
            } else {
                e.points.iter().rev().copied().collect()
            };
            if let Some(p) = spot(graph, &pts, e.width_m) {
                out.push(p);
            }
        }
    }
    out
}

/// A sign beside `pts` (the closed road from its end), facing back to the
/// end: the first spot of a few along it and on either side that is clear
/// of every carriageway.
fn spot(graph: &RoadGraph, pts: &[[f64; 2]], width_m: f64) -> Option<PointObstacle> {
    let len: f64 = pts
        .windows(2)
        .map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]))
        .sum();
    let side = width_m / 2.0 + SIDE_M;
    for k in [1.0, 2.0, 3.0, 0.5] {
        let d = (IN_M * k).min(len * 0.5);
        let (c, t) = along(pts, d);
        // The rider's right going onto the road (x, z with y up), then
        // the left.
        for s in [1.0, -1.0] {
            let p = [c[0] - t[1] * side * s, c[1] + t[0] * side * s];
            if clearance(graph, p) >= CLEAR_M {
                return Some(PointObstacle {
                    kind: ObstacleKind::NoBikeSign,
                    flags: 0,
                    pos: [p[0] as f32, p[1] as f32],
                    // The face looks back along the road, at the rider.
                    angle: (-t[1]).atan2(-t[0]) as f32,
                    width_m: 0.0,
                    height_m: HEIGHT_M,
                });
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{Edge, EdgeId, Oneway, RoadClass};

    fn edge(id: u64, way: i64, from: u32, to: u32, pts: Vec<[f64; 2]>) -> Edge {
        Edge {
            id: EdgeId(id),
            from: VertexId(from),
            to: VertexId(to),
            points: pts,
            class: RoadClass::Residential,
            name: None,
            width_m: 5.0,
            bridge: false,
            tunnel: false,
            dead_end_m: None,
            boundary: false,
            oneway: Oneway::No,
            osm_way: way,
            profile: Vec::new(),
        }
    }

    /// A main road v0 – v1 – v2 and a side road (way 7, two edges)
    /// v1 – v3 – v4 that meets it at v1 and the road v4 – v5 at v4.
    fn graph() -> RoadGraph {
        RoadGraph {
            vertices: vec![
                [-100.0, 0.0],
                [0.0, 0.0],
                [100.0, 0.0],
                [0.0, 60.0],
                [0.0, 120.0],
                [80.0, 120.0],
            ],
            edges: vec![
                edge(1, 1, 0, 1, vec![[-100.0, 0.0], [0.0, 0.0]]),
                edge(2, 1, 1, 2, vec![[0.0, 0.0], [100.0, 0.0]]),
                edge(3, 7, 1, 3, vec![[0.0, 0.0], [0.0, 60.0]]),
                edge(4, 7, 3, 4, vec![[0.0, 60.0], [0.0, 120.0]]),
                edge(5, 9, 4, 5, vec![[0.0, 120.0], [80.0, 120.0]]),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn a_closed_road_gets_a_sign_at_each_end_beside_the_carriageway() {
        let g = graph();
        let signs = place(&g, &[2, 3]);
        assert_eq!(signs.len(), 2, "both ends, none in the middle: {signs:?}");
        for s in &signs {
            assert_eq!(s.kind, ObstacleKind::NoBikeSign);
            let p = [f64::from(s.pos[0]), f64::from(s.pos[1])];
            assert!(clearance(&g, p) >= CLEAR_M, "not in a road: {p:?}");
            // Beside the closed road (x = 0), a few metres in.
            assert!((p[0].abs() - (2.5 + SIDE_M)).abs() < 1e-3, "{p:?}");
            assert!(!s.kind.solid());
        }
        let mut z: Vec<f64> = signs.iter().map(|s| f64::from(s.pos[1])).collect();
        z.sort_by(f64::total_cmp);
        assert!(
            z[0] > 0.0 && z[0] < 20.0 && z[1] > 100.0 && z[1] < 120.0,
            "{z:?}"
        );
        // Each faces the end it stands at: the one near v1 looks to -z.
        let near = signs.iter().find(|s| s.pos[1] < 60.0).unwrap();
        assert!(
            (near.angle - (-std::f32::consts::FRAC_PI_2)).abs() < 1e-4,
            "{}",
            near.angle
        );
        let far = signs.iter().find(|s| s.pos[1] > 60.0).unwrap();
        assert!(
            (far.angle - std::f32::consts::FRAC_PI_2).abs() < 1e-4,
            "{}",
            far.angle
        );
        // A dead-end closed road: a sign at its open end too.
        let one = place(&g, &[4]);
        assert_eq!(one.len(), 2);
        // Nothing closed: no signs.
        assert!(place(&g, &[]).is_empty());
    }

    #[test]
    fn a_sign_in_another_road_moves_along() {
        // A road parallel to the closed one, 5.5 m off: the right side is in
        // it, so the sign goes to the left.
        let mut g = graph();
        g.vertices.push([-5.5, 0.0]);
        g.vertices.push([-5.5, 60.0]);
        g.edges
            .push(edge(6, 11, 6, 7, vec![[-5.5, 0.0], [-5.5, 60.0]]));
        for s in place(&g, &[2]) {
            let p = [f64::from(s.pos[0]), f64::from(s.pos[1])];
            assert!(clearance(&g, p) >= CLEAR_M, "{p:?}");
        }
    }
}
