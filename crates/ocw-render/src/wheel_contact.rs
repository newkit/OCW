//! Wheel/road contact, measured (bug 2026-11-13: "the wheels of the bike
//! always seem to be like 1 cm inside the road").
//!
//! Every shipped bike model is placed on a known road exactly as the riders
//! plugin places it (avatar transform, sway node, spinning wheels or the far
//! LOD); the tyre's lowest vertex is compared with the road surface **as
//! drawn** (the route ribbon or the package road strips, triangle by
//! triangle) straight under it. Positive gap = the tyre floats, negative =
//! it sinks into the road.

use bevy::math::Affine3A;
use bevy::mesh::{Indices, VertexAttributeValues};
use bevy::prelude::*;
use ocw_assets::model::Model;
use ocw_assets::pose::bike_nodes;
use ocw_core::ProfileId;
use ocw_world::graph::RoadClass;
use ocw_world::{RoadStrip, Route, Track};

use crate::riders::{avatar_transform, sway_transform};

/// Allowed |gap| between the tyre and the road (m).
const TOLERANCE_M: f32 = 0.002;

/// A drawn road surface: its upward triangles, bucketed on a 4 m grid.
struct Surface {
    tris: Vec<[Vec3; 3]>,
    buckets: std::collections::HashMap<(i32, i32), Vec<u32>>,
}

const BUCKET_M: f32 = 4.0;

impl Surface {
    fn new(positions: &[[f32; 3]], indices: &[u32]) -> Self {
        let mut tris = Vec::new();
        let mut buckets: std::collections::HashMap<(i32, i32), Vec<u32>> = Default::default();
        for t in indices.as_chunks::<3>().0 {
            let [a, b, c] = t.map(|i| Vec3::from(positions[i as usize]));
            // Only (roughly) horizontal faces: not the strips' edge faces.
            let area = (b - a).cross(c - a);
            if area.y.abs() < 1e-4 {
                continue;
            }
            let i = tris.len() as u32;
            let (lo, hi) = (a.min(b).min(c), a.max(b).max(c));
            let k = |v: f32| (v / BUCKET_M).floor() as i32;
            for bx in k(lo.x)..=k(hi.x) {
                for bz in k(lo.z)..=k(hi.z) {
                    buckets.entry((bx, bz)).or_default().push(i);
                }
            }
            tris.push([a, b, c]);
        }
        Self { tris, buckets }
    }

    fn of_mesh(mesh: &Mesh) -> Self {
        let Some(VertexAttributeValues::Float32x3(p)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("positions")
        };
        let idx: Vec<u32> = match mesh.indices() {
            Some(Indices::U32(i)) => i.clone(),
            Some(Indices::U16(i)) => i.iter().map(|&i| u32::from(i)).collect(),
            None => panic!("indices"),
        };
        Self::new(p, &idx)
    }

    fn of_strips(strips: &[RoadStrip]) -> Self {
        let m = crate::package::road_arrays(strips);
        Self::new(&m.positions, &m.indices)
    }

    /// The highest drawn surface straight under/over (x, z).
    fn height(&self, x: f32, z: f32) -> Option<f32> {
        self.heights(x, z).into_iter().reduce(f32::max)
    }

    /// The drawn surface straight under/over (x, z) nearest to `y`: where
    /// strips overlap (a junction, a road crossing above or below), the
    /// one the tyre is on.
    fn height_near(&self, x: f32, z: f32, y: f32) -> Option<f32> {
        self.heights(x, z)
            .into_iter()
            .min_by(|a, b| (a - y).abs().total_cmp(&(b - y).abs()))
    }

    /// Every drawn surface straight under/over (x, z).
    fn heights(&self, x: f32, z: f32) -> Vec<f32> {
        let key = ((x / BUCKET_M).floor() as i32, (z / BUCKET_M).floor() as i32);
        let mut all = Vec::new();
        for &i in self.buckets.get(&key).map_or(&[][..], Vec::as_slice) {
            let [a, b, c] = self.tris[i as usize];
            let (v0, v1, p) = (
                Vec2::new(b.x - a.x, b.z - a.z),
                Vec2::new(c.x - a.x, c.z - a.z),
                Vec2::new(x - a.x, z - a.z),
            );
            let d = v0.perp_dot(v1);
            let (s, t) = (p.perp_dot(v1) / d, v0.perp_dot(p) / d);
            if s >= -1e-5 && t >= -1e-5 && s + t <= 1.0 + 1e-5 {
                all.push(a.y + s * (b.y - a.y) + t * (c.y - a.y));
            }
        }
        all
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Lod {
    /// The near model: both wheel nodes, turned by the wheel angle.
    Near,
    /// The far model (`lod1`): one node, wheels not turning.
    Far,
}

/// Bike-root-space transform of node `i`, the wheel nodes turned by `spin`
/// (as `riders::animate` sets them).
fn node_world(m: &Model, i: usize, spin: f32) -> Affine3A {
    let n = &m.nodes[i];
    let wheel = n.name == bike_nodes::WHEEL_FRONT || n.name == bike_nodes::WHEEL_REAR;
    let rotation = if wheel {
        Quat::from_rotation_x(-spin)
    } else {
        n.rotation
    };
    let local = Affine3A::from_scale_rotation_translation(n.scale, rotation, n.translation);
    match n.parent {
        Some(p) => node_world(m, p, spin) * local,
        None => local,
    }
}

/// The tyre (wheel) vertices of a bike model in its root's frame.
fn wheel_points(m: &Model, lod: Lod, spin: f32) -> Vec<Vec3> {
    let roots: Vec<usize> = match lod {
        Lod::Near => [bike_nodes::WHEEL_FRONT, bike_nodes::WHEEL_REAR]
            .iter()
            .map(|n| m.node(n).expect("wheel node"))
            .collect(),
        Lod::Far => vec![m.node(bike_nodes::LOD1).expect("lod1")],
    };
    let mut out = Vec::new();
    for (i, node) in m.nodes.iter().enumerate() {
        if !roots.iter().any(|&r| m.is_below(i, r)) {
            continue;
        }
        let w = node_world(m, i, spin);
        for prim in &node.primitives {
            out.extend(
                prim.positions
                    .iter()
                    .map(|p| w.transform_point3(Vec3::from(*p))),
            );
        }
    }
    out
}

/// Where a rider is and how the bike moves (lean, rock, wheel angle).
#[derive(Debug, Clone, Copy)]
struct Case {
    state: ocw_sim::RiderState,
    /// The sway node's roll (standing riders rock the bike), rad.
    rock: f32,
}

/// The worst gap (m) between `bike`'s tyres and `road` over a full wheel
/// turn: the minimum over all tyre vertices of height above the surface.
fn gap(track: &dyn Track, road: &Surface, bike: ProfileId, lod: Lod, case: Case) -> f32 {
    let m = ocw_assets::shipped::bike_model(bike);
    let rig = ocw_assets::shipped::bike_rig(bike);
    let root = avatar_transform(track, &case.state, None, &rig);
    let sway = sway_transform(case.rock, &rig);
    let to_world = root.to_matrix() * sway.to_matrix();
    let spins: Vec<f32> = match lod {
        Lod::Near => (0..48)
            .map(|k| k as f32 / 48.0 * std::f32::consts::TAU)
            .collect(),
        Lod::Far => vec![0.0],
    };
    let mut worst = f32::INFINITY;
    for spin in spins {
        for p in wheel_points(m, lod, spin) {
            let w = to_world.transform_point3(p);
            if let Some(h) = road.height_near(w.x, w.z, w.y) {
                worst = worst.min(w.y - h);
            }
        }
    }
    assert!(worst.is_finite(), "no road under the tyres");
    worst
}

fn straight(grade: f64) -> Route {
    let n = 101;
    let pts = (0..n)
        .map(|i| {
            let x = i as f64 * 5.0;
            [x, x * grade, 0.0]
        })
        .collect();
    Route::from_points("t", "T", pts, false, 6.0).unwrap()
}

/// Road strips like the world builder writes them: the road's profile
/// points (here the route's).
fn strips_of(route: &Route, width: f32) -> Vec<RoadStrip> {
    vec![RoadStrip {
        class: RoadClass::Secondary,
        width_m: width,
        points: route
            .points()
            .iter()
            .map(|p| [p[0] as f32, p[1] as f32, p[2] as f32])
            .collect(),
    }]
}

fn ribbon_of(route: &Route) -> Surface {
    let view = crate::RideView::new(route.clone());
    Surface::of_mesh(&crate::world::ribbon_mesh(
        &view,
        route.road_width_m() as f32,
    ))
}

/// The cases at `distance`: beside the centre means 1.6 m to the right,
/// or less on a narrow road (still on its surface).
fn cases(track: &dyn Track, distance: f64) -> Vec<(&'static str, Case)> {
    let beside = (track.road_width_at(distance) / 2.0 - 0.6).clamp(0.0, 1.6);
    let base = ocw_sim::RiderState {
        distance,
        speed: 10.0,
        ..Default::default()
    };
    let lean = |k: f64, speed: f64| ocw_sim::RiderState {
        turn_curvature: k,
        speed,
        ..base
    };
    vec![
        (
            "straight",
            Case {
                state: base,
                rock: 0.0,
            },
        ),
        (
            "beside centre",
            Case {
                state: ocw_sim::RiderState {
                    lateral_offset: beside,
                    ..base
                },
                rock: 0.0,
            },
        ),
        (
            "lean 20°",
            Case {
                state: lean(0.0364, 10.0),
                rock: 0.0,
            },
        ),
        (
            "lean max left",
            Case {
                state: lean(-0.2, 15.0),
                rock: 0.0,
            },
        ),
        (
            "rocking",
            Case {
                state: base,
                rock: 0.16,
            },
        ),
        (
            "lean max + rock",
            Case {
                state: lean(0.2, 15.0),
                rock: -0.16,
            },
        ),
    ]
}

/// Measures every bike, LOD and case on `road` at the distances `at`;
/// prints the range of the gap (mm) per case, bike and LOD and returns
/// the worst |gap|.
fn measure(label: &str, track: &dyn Track, road: &Surface, at: &[f64]) -> f32 {
    let mut worst = 0.0f32;
    for (ci, (name, _)) in cases(track, 0.0).into_iter().enumerate() {
        let mut row = format!("{label:>14} {name:>16}:");
        for bike in ProfileId::ALL {
            for lod in [Lod::Near, Lod::Far] {
                let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
                for &d in at {
                    let case = cases(track, d)[ci].1;
                    let g = gap(track, road, bike, lod, case);
                    lo = lo.min(g);
                    hi = hi.max(g);
                }
                worst = worst.max(lo.abs()).max(hi.abs());
                let tag = if lod == Lod::Near { "" } else { "/far" };
                row += &format!(
                    " {}{tag} {:+.1}..{:+.1}",
                    bike.id(),
                    lo * 1000.0,
                    hi * 1000.0
                );
            }
        }
        println!("{row}");
    }
    println!("{label}: worst |gap| {:.1} mm", worst * 1000.0);
    worst
}

#[test]
fn tyres_touch_the_route_ribbon() {
    for grade in [0.0, 0.12, -0.2] {
        let route = straight(grade);
        let worst = measure(
            &format!("ribbon {grade}"),
            &route,
            &ribbon_of(&route),
            &[100.0, 252.5],
        );
        assert!(worst <= TOLERANCE_M, "grade {grade}: {worst}");
    }
}

#[test]
fn tyres_touch_package_road_strips() {
    for grade in [0.0, 0.12, -0.2] {
        let route = straight(grade);
        let road = Surface::of_strips(&strips_of(&route, 6.0));
        let worst = measure(&format!("strips {grade}"), &route, &road, &[100.0, 252.5]);
        assert!(worst <= TOLERANCE_M, "grade {grade}: {worst}");
    }
}

/// A crest and a dip (grade changes of 8–9 % per point), where the bike's
/// two wheels stand on different road segments. The points are 3.7 m
/// apart: not on the ribbon's 2 m steps.
#[test]
fn tyres_touch_the_road_over_crests_and_dips() {
    let mut pts = Vec::new();
    let mut y = 0.0;
    for i in 0..120 {
        let x = i as f64 * 3.7;
        let g = match i {
            0..40 => 0.0,
            40..42 => 0.08,
            42..80 => 0.15,
            80..82 => 0.06,
            _ => -0.02,
        };
        pts.push([x, y, 0.0]);
        y += g * 3.7;
    }
    let route = Route::from_points("t", "T", pts, false, 6.0).unwrap();
    let at: Vec<f64> = (0..40).map(|k| 143.0 + k as f64 * 0.5).collect();
    let mut at2: Vec<f64> = (0..40).map(|k| 291.0 + k as f64 * 0.5).collect();
    let mut all = at;
    all.append(&mut at2);
    for (label, road) in [
        ("crest ribbon", ribbon_of(&route)),
        ("crest strips", Surface::of_strips(&strips_of(&route, 6.0))),
    ] {
        let worst = measure(label, &route, &road, &all);
        assert!(worst <= TOLERANCE_M, "{label}: {worst}");
    }
}

/// The real road of the Vaduz–Malbun climb in the Liechtenstein package:
/// at 11 spots along the route (km 0.5 … 13, flat Vaduz to the 15 % ramps)
/// a rider rides the package's road network (a free-ride path, as the
/// app builds it: its heights are the road graph's, like the road strips')
/// and is measured against the package's road strips 20 … 240 m on
/// (`OCW_CONTACT_WORLD=…/liechtenstein.ocwpkg`; skipped without).
#[test]
fn tyres_touch_the_road_on_vaduz_malbun() {
    let Some(pkg_path) = std::env::var_os("OCW_CONTACT_WORLD") else {
        eprintln!("OCW_CONTACT_WORLD not set: skipped");
        return;
    };
    let pkg = ocw_world::WorldPackage::from_bytes(&std::fs::read(pkg_path).unwrap()).unwrap();
    let net = std::sync::Arc::new(ocw_world::RoadNetwork::new(
        pkg.graph.clone().expect("road graph"),
    ));
    let strips: Vec<RoadStrip> = pkg
        .tiles
        .values()
        .flat_map(|t| t.roads.iter().cloned())
        .collect();
    let road = Surface::of_strips(&strips);
    // Points of the route (x, z) at these km marks.
    let spots = [
        (0.5, [-2418.0, 1841.0]),
        (1.0, [-2286.0, 1681.0]),
        (2.0, [-2093.0, 2356.0]),
        (3.0, [-1767.0, 3272.0]),
        (3.8, [-1457.0, 3955.0]),
        (4.5, [-1097.0, 4372.0]),
        (6.0, [-342.0, 5072.0]),
        (7.5, [-285.0, 4098.0]),
        (10.0, [1503.0, 5259.0]),
        (11.5, [2953.0, 5406.0]),
        (13.0, [4059.0, 6244.0]),
    ];
    // The road bike riding straight, per mark: the model and placement are
    // exact on synthetic roads (the tests above); here the gap is how far
    // the path's profile (road graph) and the strips' (resampled every
    // 5 m) differ.
    let mut gaps: Vec<(f64, f64, f32)> = Vec::new();
    for (km, p) in spots {
        let (edge, offset) = net.nearest(p).expect("a road");
        let mut path = ocw_world::RidePath::start(net.clone(), edge, offset, true, 0.0, 0);
        path.extend(0.0, &mut |_| 0);
        // Where a road strip is under the rider (not in a tunnel).
        let at: Vec<f64> = (0..12)
            .map(|k| 20.0 + f64::from(k) * 20.3)
            .filter(|&d| d < path.end_m() - 5.0)
            .filter(|&d| {
                let q = path.sample(d).position;
                road.height(q[0] as f32, q[2] as f32).is_some()
            })
            .collect();
        if at.is_empty() {
            continue;
        }
        for &d in &at {
            let straight = cases(&path, d)[0].1;
            gaps.push((
                km,
                d,
                gap(&path, &road, ProfileId::Road, Lod::Near, straight),
            ));
        }
        let grades: Vec<String> = at
            .iter()
            .map(|&d| format!("{:+.0}", path.sample(d).grade * 100.0))
            .collect();
        println!("km {km}: grades (%) {grades:?}");
        measure(&format!("km {km}"), &path, &road, &at);
    }
    assert!(gaps.len() >= 60, "{} marks on the road", gaps.len());
    for (km, d, g) in gaps.iter().filter(|g| g.2.abs() > TOLERANCE_M) {
        println!(
            "outlier: {d:.0} m on from km {km}: gap {:+.1} mm",
            g * 1000.0
        );
    }
    // No systematic sinking (the bug: −20 mm everywhere) or floating.
    let mut sorted: Vec<f32> = gaps.iter().map(|g| g.2).collect();
    sorted.sort_by(f32::total_cmp);
    let median = sorted[sorted.len() / 2];
    let within = sorted.iter().filter(|g| g.abs() <= 0.004).count();
    println!(
        "vaduz-malbun: {} marks, median gap {:+.1} mm, {within} within 4 mm",
        sorted.len(),
        median * 1000.0
    );
    assert!(median.abs() <= TOLERANCE_M, "median {median}");
    assert!(
        within * 4 >= sorted.len() * 3,
        "{within} of {}",
        sorted.len()
    );
}
