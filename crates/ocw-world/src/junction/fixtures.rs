//! Synthetic junctions for tests (here, in `ocw-render` and its analysis
//! test): small road graphs with profiles, and their road strips as the
//! world builder writes them (a point every 5 m at the profile height,
//! the polyline's vertices kept, `ocw_worldgen::package::road_strips`).
//! Coordinates are local metres (x east, z south).

use std::collections::BTreeSet;

use crate::graph::{Edge, EdgeId, Oneway, PROFILE_STEP_M, RoadClass, RoadGraph, VertexId};
use crate::package::RoadStrip;
use crate::{RoadDress, RoadLit, RoadSurface};

/// One road of a fixture: vertices, polyline (from `from` to `to`, the
/// vertices' positions at its ends), class, width and its height as a
/// function of the distance from `from`.
pub struct Road {
    pub from: u32,
    pub to: u32,
    pub points: Vec<[f64; 2]>,
    pub class: RoadClass,
    pub width_m: f64,
    pub height: Box<dyn Fn(f64) -> f64>,
}

impl Road {
    /// A straight road between two vertices.
    pub fn straight(
        v: &[[f64; 2]],
        from: u32,
        to: u32,
        class: RoadClass,
        width_m: f64,
        height: impl Fn(f64) -> f64 + 'static,
    ) -> Self {
        Self {
            from,
            to,
            points: vec![v[from as usize], v[to as usize]],
            class,
            width_m,
            height: Box::new(height),
        }
    }
}

/// The graph of `roads` between `vertices`, with profiles every 5 m.
pub fn graph(vertices: &[[f64; 2]], roads: Vec<Road>) -> RoadGraph {
    let edges = roads
        .into_iter()
        .enumerate()
        .map(|(i, r)| {
            let mut e = Edge {
                id: EdgeId(i as u64 + 1),
                from: VertexId(r.from),
                to: VertexId(r.to),
                points: r.points,
                class: r.class,
                name: None,
                width_m: r.width_m,
                bridge: false,
                tunnel: false,
                dead_end_m: None,
                boundary: false,
                oneway: Oneway::No,
                osm_way: i as i64 + 1,
                profile: Vec::new(),
            };
            let len = e.length_m();
            let n = (len / PROFILE_STEP_M).ceil() as usize;
            e.profile = (0..=n)
                .map(|k| (r.height)((k as f64 * PROFILE_STEP_M).min(len)))
                .collect();
            e
        })
        .collect();
    RoadGraph {
        vertices: vertices.to_vec(),
        edges,
        boundary_vertices: BTreeSet::new(),
    }
}

/// The road strip of every edge (one per edge, no tiles), as the world
/// builder writes them.
pub fn strips(g: &RoadGraph) -> Vec<(usize, RoadStrip)> {
    g.edges
        .iter()
        .enumerate()
        .map(|(ei, e)| {
            let pts: Vec<[f32; 3]> = super::sample_along(&e.points)
                .into_iter()
                .map(|(p, s)| {
                    [
                        p[0] as f32,
                        e.height_at(s).unwrap_or(0.0) as f32,
                        p[1] as f32,
                    ]
                })
                .collect();
            let mut dress = RoadDress::of_class(e.class, e.width_m as f32);
            let mut cum = 0.0f32;
            for w in pts.windows(2) {
                cum += ((w[1][0] - w[0][0]).powi(2)
                    + (w[1][1] - w[0][1]).powi(2)
                    + (w[1][2] - w[0][2]).powi(2))
                .sqrt();
            }
            dress.edge_m = cum;
            (
                ei,
                RoadStrip {
                    class: e.class,
                    width_m: e.width_m as f32,
                    points: pts,
                    surface: RoadSurface::of_class(e.class),
                    dress,
                    lit: RoadLit::Unknown,
                    condition: Default::default(),
                },
            )
        })
        .collect()
}

/// A T junction at (0, 0): a main road along x (`main_class`, 7.5 m,
/// climbing `main_grade` towards +x, split into two edges at the vertex)
/// and a side road (5 m) to +z climbing `side_grade` away from it.
pub fn t_junction(main_class: RoadClass, main_grade: f64, side_grade: f64) -> RoadGraph {
    let v = [[0.0, 0.0], [-120.0, 0.0], [120.0, 0.0], [0.0, 120.0]];
    graph(
        &v,
        vec![
            Road::straight(&v, 1, 0, main_class, 7.5, move |s| (s - 120.0) * main_grade),
            Road::straight(&v, 0, 2, main_class, 7.5, move |s| s * main_grade),
            Road::straight(&v, 0, 3, RoadClass::Residential, 5.0, move |s| {
                s * side_grade
            }),
        ],
    )
}

/// A T junction as [`t_junction`] (secondary main road, 7.5 m), with the
/// side road (residential, 5 m) leaving at `angle_deg` from the main road's
/// +x arm (towards +z): a narrow angle puts its mouth far out.
pub fn t_junction_at(main_grade: f64, side_grade: f64, angle_deg: f64) -> RoadGraph {
    let a = angle_deg.to_radians();
    let v = [
        [0.0, 0.0],
        [-120.0, 0.0],
        [120.0, 0.0],
        [120.0 * a.cos(), 120.0 * a.sin()],
    ];
    graph(
        &v,
        vec![
            Road::straight(&v, 1, 0, RoadClass::Secondary, 7.5, move |s| {
                (s - 120.0) * main_grade
            }),
            Road::straight(&v, 0, 2, RoadClass::Secondary, 7.5, move |s| s * main_grade),
            Road::straight(&v, 0, 3, RoadClass::Residential, 5.0, move |s| {
                s * side_grade
            }),
        ],
    )
}

/// A side road meeting a dual carriageway (J3 clusters): two carriageways
/// along x (secondary, 5 m, climbing 3 % towards +x) 7 m apart (a 2 m
/// median), at z = 0 (vertex 0) and z = 7 (vertex 1), joined by a 7 m edge
/// across (edge 5); the side road (residential, 5 m, climbing 6 % away)
/// comes from −z to vertex 0. Their patches alone would meet on the edge
/// across.
pub fn dual_carriageway_t() -> RoadGraph {
    let v = [
        [0.0, 0.0],
        [0.0, 7.0],
        [-100.0, 0.0],
        [100.0, 0.0],
        [-100.0, 7.0],
        [100.0, 7.0],
        [0.0, -100.0],
    ];
    graph(
        &v,
        vec![
            Road::straight(&v, 2, 0, RoadClass::Secondary, 5.0, |s| (s - 100.0) * 0.03),
            Road::straight(&v, 0, 3, RoadClass::Secondary, 5.0, |s| s * 0.03),
            Road::straight(&v, 4, 1, RoadClass::Secondary, 5.0, |s| (s - 100.0) * 0.03),
            Road::straight(&v, 1, 5, RoadClass::Secondary, 5.0, |s| s * 0.03),
            Road::straight(&v, 6, 0, RoadClass::Residential, 5.0, |s| {
                (100.0 - s) * 0.06
            }),
            Road::straight(&v, 0, 1, RoadClass::Secondary, 5.0, |_| 0.0),
        ],
    )
}

/// A small roundabout (J3): a one-way ring of radius 7 m (tertiary, 6 m
/// wide; outer diameter 20 m) round (0, 0), split at its four entries
/// (vertices 0 – 3 at 0°, 90°, 180°, 270°) into four 11 m arcs (edges
/// 4 – 7), the entries (residential, 5 m) leaving outward, climbing 4 %.
pub fn small_roundabout() -> RoadGraph {
    let r = 7.0;
    let at = |deg: f64| [r * deg.to_radians().cos(), r * deg.to_radians().sin()];
    let mut v: Vec<[f64; 2]> = (0..4).map(|k| at(90.0 * f64::from(k))).collect();
    for k in 0..4 {
        let d = (90.0 * f64::from(k)).to_radians();
        v.push([100.0 * d.cos(), 100.0 * d.sin()]);
    }
    let mut roads: Vec<Road> = (0..4u32)
        .map(|k| Road::straight(&v, k, k + 4, RoadClass::Residential, 5.0, |s| s * 0.04))
        .collect();
    for k in 0..4u32 {
        let points = (0..=9)
            .map(|i| at(90.0 * f64::from(k) + 10.0 * f64::from(i)))
            .collect();
        roads.push(Road {
            from: k,
            to: (k + 1) % 4,
            points,
            class: RoadClass::Tertiary,
            width_m: 6.0,
            height: Box::new(|_| 0.0),
        });
    }
    let mut g = graph(&v, roads);
    for e in &mut g.edges[4..] {
        e.oneway = Oneway::Forward;
    }
    g
}

/// An equal crossing of four residential roads at (0, 0), each climbing or
/// falling at its own grade away from the vertex.
pub fn crossing(grades: [f64; 4]) -> RoadGraph {
    let v = [
        [0.0, 0.0],
        [100.0, 0.0],
        [0.0, 100.0],
        [-100.0, 0.0],
        [0.0, -100.0],
    ];
    let roads = (0..4)
        .map(|k| {
            let g = grades[k];
            Road::straight(&v, 0, k as u32 + 1, RoadClass::Residential, 5.0, move |s| {
                s * g
            })
        })
        .collect();
    graph(&v, roads)
}

/// A fork: a road from −x splits at (0, 0) into two arms `spread_deg`
/// apart (symmetric about +x).
pub fn fork(spread_deg: f64) -> RoadGraph {
    let h = (spread_deg / 2.0).to_radians();
    let v = [
        [0.0, 0.0],
        [-100.0, 0.0],
        [100.0 * h.cos(), -100.0 * h.sin()],
        [100.0 * h.cos(), 100.0 * h.sin()],
    ];
    graph(
        &v,
        vec![
            Road::straight(&v, 1, 0, RoadClass::Residential, 5.0, |s| {
                (s - 100.0) * 0.03
            }),
            Road::straight(&v, 0, 2, RoadClass::Residential, 5.0, |s| s * 0.03),
            Road::straight(&v, 0, 3, RoadClass::Residential, 5.0, |s| s * 0.05),
        ],
    )
}

/// A way split at (0, 0) bending by `bend_deg`, the first edge `w1` wide,
/// the second `w2`, climbing at `grade`.
pub fn split(bend_deg: f64, w1: f64, w2: f64, grade: f64) -> RoadGraph {
    let b = bend_deg.to_radians();
    let v = [
        [0.0, 0.0],
        [-100.0, 0.0],
        [100.0 * b.cos(), 100.0 * b.sin()],
    ];
    graph(
        &v,
        vec![
            Road::straight(&v, 1, 0, RoadClass::Secondary, w1, move |s| {
                (s - 100.0) * grade
            }),
            Road::straight(&v, 0, 2, RoadClass::Secondary, w2, move |s| s * grade),
        ],
    )
}
