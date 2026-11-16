//! Picking in the views (spec §14 Phase 16, 16C, E5 step 2), pure and
//! tested: a click on the 2D map or in the 3D view becomes a package point
//! (x, z), and the point the nearest road edge – the edge the click
//! selects. The 3D view's click is a ray from the camera, followed to the
//! ground ([`ray_ground`]).

use ocw_world::graph::{Edge, RoadGraph};

/// How far from a road centreline a click in the 3D view still picks it
/// (m), plus 1 % of the distance to the camera.
pub const PICK_3D_M: f64 = 12.0;
/// The same on the 2D map, in screen points (at least 3 m).
pub const PICK_MAP_PX: f64 = 7.0;

/// The pick tolerance on the 2D map at a zoom (metres per point).
pub fn map_tolerance_m(m_per_px: f64) -> f64 {
    (PICK_MAP_PX * m_per_px).max(3.0)
}

/// The pick tolerance in the 3D view for a hit `distance_m` from the
/// camera.
pub fn view_tolerance_m(distance_m: f64) -> f64 {
    PICK_3D_M + 0.01 * distance_m.max(0.0)
}

/// The nearest point of a polyline to `p`: (distance, arc position along
/// the line, the point on it).
pub fn nearest_on_polyline(points: &[[f64; 2]], p: [f64; 2]) -> (f64, f64, [f64; 2]) {
    let mut best = (f64::INFINITY, 0.0, points.first().copied().unwrap_or(p));
    if points.len() == 1 {
        let q = points[0];
        return ((q[0] - p[0]).hypot(q[1] - p[1]), 0.0, q);
    }
    let mut arc = 0.0;
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        let d = [b[0] - a[0], b[1] - a[1]];
        let l2 = d[0] * d[0] + d[1] * d[1];
        let l = l2.sqrt();
        let t = if l2 > 0.0 {
            (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / l2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let q = [a[0] + d[0] * t, a[1] + d[1] * t];
        let dist = (q[0] - p[0]).hypot(q[1] - p[1]);
        if dist < best.0 {
            best = (dist, arc + l * t, q);
        }
        arc += l;
    }
    best
}

/// A road picked at a point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pick {
    /// Index of the edge in the graph.
    pub edge: usize,
    /// Distance of the click from the centreline (m).
    pub dist: f64,
    /// Arc position of the nearest centreline point from the edge's start.
    pub arc: f64,
    /// That point (x, z).
    pub foot: [f64; 2],
}

/// The edge nearest to `p` within `max_m` of its centreline – or of its
/// edge, for wide roads (a click on the carriageway always picks it).
pub fn pick_edge(graph: &RoadGraph, p: [f64; 2], max_m: f64) -> Option<Pick> {
    let mut best: Option<Pick> = None;
    for (i, e) in graph.edges.iter().enumerate() {
        if e.points.is_empty() {
            continue;
        }
        let reach = max_m + e.width_m * 0.5;
        if !near_box(&e.points, p, reach) {
            continue;
        }
        let (dist, arc, foot) = nearest_on_polyline(&e.points, p);
        // Compared by the distance beyond the carriageway's half width,
        // so a click on a wide road beside a narrow one picks the wide one;
        // on both carriageways (a crossing) the wider road wins.
        let score = (dist - e.width_m * 0.5).max(0.0);
        if dist <= reach
            && best.is_none_or(|b| {
                let bs = (b.dist - graph.edges[b.edge].width_m * 0.5).max(0.0);
                let bw = graph.edges[b.edge].width_m;
                score < bs
                    || (score == bs && (e.width_m > bw || (e.width_m == bw && dist < b.dist)))
            })
        {
            best = Some(Pick {
                edge: i,
                dist,
                arc,
                foot,
            });
        }
    }
    best
}

fn near_box(points: &[[f64; 2]], p: [f64; 2], reach: f64) -> bool {
    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for q in points {
        lo = [lo[0].min(q[0]), lo[1].min(q[1])];
        hi = [hi[0].max(q[0]), hi[1].max(q[1])];
    }
    p[0] >= lo[0] - reach && p[0] <= hi[0] + reach && p[1] >= lo[1] - reach && p[1] <= hi[1] + reach
}

/// Where a ray from `origin` along `dir` (need not be normalised) first
/// meets the ground (`ground(x, z)`: its height, `None` outside the
/// world), within `max_m`. Steps that grow with the distance, then a
/// bisection to a few centimetres.
pub fn ray_ground(
    origin: [f64; 3],
    dir: [f64; 3],
    ground: impl Fn(f64, f64) -> Option<f64>,
    max_m: f64,
) -> Option<[f64; 3]> {
    let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
    if len <= 0.0 || !len.is_finite() {
        return None;
    }
    let d = [dir[0] / len, dir[1] / len, dir[2] / len];
    let at = |t: f64| {
        [
            origin[0] + d[0] * t,
            origin[1] + d[1] * t,
            origin[2] + d[2] * t,
        ]
    };
    let above = |t: f64| {
        let p = at(t);
        ground(p[0], p[2]).map(|h| p[1] - h)
    };
    let mut t0 = 0.0;
    let mut t = 0.0;
    while t < max_m {
        let step = (t * 0.01).clamp(0.5, 25.0);
        t = (t + step).min(max_m);
        match above(t) {
            Some(h) if h <= 0.0 => {
                // The ground is between t0 and t: bisect.
                let (mut a, mut b) = (t0, t);
                for _ in 0..40 {
                    let m = 0.5 * (a + b);
                    match above(m) {
                        Some(h) if h <= 0.0 => b = m,
                        _ => a = m,
                    }
                    if b - a < 0.02 {
                        break;
                    }
                }
                return Some(at(b));
            }
            _ => t0 = t,
        }
    }
    None
}

/// An edge's centreline in 3D: its points at the road profile's height
/// (else the ground's) plus `lift` metres, so a line drawn over the road
/// is not hidden in it.
pub fn edge_line(
    edge: &Edge,
    profile: Option<&[f64]>,
    lift: f64,
    ground: impl Fn(f64, f64) -> Option<f64>,
) -> Vec<[f64; 3]> {
    let mut e = None;
    if let Some(p) = profile {
        let mut c = edge.clone();
        c.profile = p.to_vec();
        e = Some(c);
    }
    let edge = e.as_ref().unwrap_or(edge);
    let mut arc = 0.0;
    let mut out = Vec::with_capacity(edge.points.len());
    for (i, q) in edge.points.iter().enumerate() {
        if i > 0 {
            let a = edge.points[i - 1];
            arc += (q[0] - a[0]).hypot(q[1] - a[1]);
        }
        let h = edge
            .height_at(arc)
            .or_else(|| ground(q[0], q[1]))
            .unwrap_or(0.0);
        out.push([q[0], h + lift, q[1]]);
    }
    out
}

/// Where along a polyline the arc position `arc` is (x, z).
pub fn point_at(points: &[[f64; 2]], arc: f64) -> Option<[f64; 2]> {
    let first = *points.first()?;
    let mut left = arc.max(0.0);
    for w in points.windows(2) {
        let l = (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
        if left <= l && l > 0.0 {
            let t = left / l;
            return Some([
                w[0][0] + (w[1][0] - w[0][0]) * t,
                w[0][1] + (w[1][1] - w[0][1]) * t,
            ]);
        }
        left -= l;
    }
    Some(points.last().copied().unwrap_or(first))
}

#[cfg(test)]
pub mod tests {
    use ocw_world::graph::{EdgeId, Oneway, PROFILE_STEP_M, RoadClass, VertexId};

    use super::*;

    pub fn edge(id: u64, way: i64, pts: Vec<[f64; 2]>, class: RoadClass, height: f64) -> Edge {
        let mut e = Edge {
            id: EdgeId(id),
            from: VertexId(0),
            to: VertexId(1),
            points: pts,
            class,
            name: Some("Hauptstrasse".into()),
            width_m: class.default_width_m(),
            bridge: false,
            tunnel: false,
            dead_end_m: None,
            boundary: false,
            oneway: Oneway::No,
            osm_way: way,
            profile: Vec::new(),
        };
        let n = (e.length_m() / PROFILE_STEP_M).ceil() as usize + 1;
        e.profile = vec![height; n];
        e
    }

    /// A made-up graph: a primary road along x at z = 0 (edge 100, way
    /// 7), a path crossing it along z at x = 50 (edge 200, way 8), and a
    /// residential road at z = 40 (edge 300, way 7).
    pub fn graph() -> RoadGraph {
        RoadGraph {
            vertices: vec![[0.0, 0.0], [100.0, 0.0]],
            edges: vec![
                edge(
                    100,
                    7,
                    vec![[0.0, 0.0], [100.0, 0.0]],
                    RoadClass::Primary,
                    400.0,
                ),
                edge(
                    200,
                    8,
                    vec![[50.0, -50.0], [50.0, 50.0]],
                    RoadClass::Path,
                    401.0,
                ),
                edge(
                    300,
                    7,
                    vec![[0.0, 40.0], [100.0, 40.0]],
                    RoadClass::Residential,
                    402.0,
                ),
            ],
            boundary_vertices: Default::default(),
        }
    }

    #[test]
    fn a_click_picks_the_nearest_road_within_the_tolerance() {
        let g = graph();
        let p = pick_edge(&g, [20.0, 3.0], 5.0).unwrap();
        assert_eq!(p.edge, 0);
        assert!((p.dist - 3.0).abs() < 1e-9 && (p.arc - 20.0).abs() < 1e-9);
        assert_eq!(p.foot, [20.0, 0.0]);
        assert_eq!(pick_edge(&g, [20.0, 39.0], 5.0).unwrap().edge, 2);
        assert_eq!(pick_edge(&g, [52.0, 25.0], 5.0).unwrap().edge, 1);
        assert_eq!(pick_edge(&g, [20.0, 20.0], 5.0), None, "between the roads");
        assert!(pick_edge(&g, [20.0, 20.0], 19.0).is_some());
        assert_eq!(pick_edge(&g, [500.0, 0.0], 5.0), None, "far away");
        // On the crossing, the wide primary road wins over the path
        // although the click is nearer to the path's centreline.
        assert_eq!(pick_edge(&g, [50.5, 2.0], 5.0).unwrap().edge, 0);
    }

    #[test]
    fn tolerances_follow_the_zoom_and_the_distance() {
        assert_eq!(map_tolerance_m(0.1), 3.0);
        assert_eq!(map_tolerance_m(10.0), 70.0);
        assert_eq!(view_tolerance_m(100.0), PICK_3D_M + 1.0);
    }

    #[test]
    fn a_ray_from_the_camera_meets_the_ground() {
        // Flat ground at 400 m; from 500 m, 45° down towards +x.
        let flat = |_: f64, _: f64| Some(400.0);
        let hit = ray_ground([0.0, 500.0, 0.0], [1.0, -1.0, 0.0], flat, 5000.0).unwrap();
        assert!((hit[0] - 100.0).abs() < 0.05 && (hit[1] - 400.0).abs() < 0.05);
        assert_eq!(hit[2], 0.0);
        // Looking up, or off the world: nothing.
        assert_eq!(
            ray_ground([0.0, 500.0, 0.0], [1.0, 0.2, 0.0], flat, 5000.0),
            None
        );
        let island = |x: f64, _: f64| (x < 50.0).then_some(400.0);
        assert_eq!(
            ray_ground([0.0, 500.0, 0.0], [1.0, -1.0, 0.0], island, 5000.0),
            None
        );
        // A hill in front stops the ray before the flat part behind it.
        let hill = |x: f64, _: f64| {
            Some(if (40.0..60.0).contains(&x) {
                475.0
            } else {
                0.0
            })
        };
        let hit = ray_ground([0.0, 500.0, 0.0], [1.0, -0.5, 0.0], hill, 5000.0).unwrap();
        assert!((hit[0] - 50.0).abs() < 0.1, "{hit:?}");
        assert_eq!(ray_ground([0.0; 3], [0.0; 3], flat, 10.0), None);
    }

    #[test]
    fn an_edge_line_follows_the_profile_or_a_new_one() {
        let g = graph();
        let line = edge_line(&g.edges[0], None, 0.5, |_, _| None);
        assert_eq!(line, vec![[0.0, 400.5, 0.0], [100.0, 400.5, 0.0]]);
        let raised: Vec<f64> = (0..21).map(|i| 400.0 + i as f64).collect();
        let line = edge_line(&g.edges[0], Some(&raised), 0.0, |_, _| None);
        assert_eq!(line[1], [100.0, 420.0, 0.0]);
        let mut flat = g.edges[0].clone();
        flat.profile.clear();
        let line = edge_line(&flat, None, 1.0, |_, _| Some(10.0));
        assert_eq!(line[0][1], 11.0, "the ground without a profile");
    }

    #[test]
    fn points_along_a_line() {
        let pts = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]];
        assert_eq!(point_at(&pts, 5.0), Some([5.0, 0.0]));
        assert_eq!(point_at(&pts, 15.0), Some([10.0, 5.0]));
        assert_eq!(point_at(&pts, 99.0), Some([10.0, 10.0]));
        assert_eq!(point_at(&[], 1.0), None);
        let (d, arc, foot) = nearest_on_polyline(&pts, [12.0, 4.0]);
        assert!((d - 2.0).abs() < 1e-9 && (arc - 14.0).abs() < 1e-9);
        assert_eq!(foot, [10.0, 4.0]);
    }
}
