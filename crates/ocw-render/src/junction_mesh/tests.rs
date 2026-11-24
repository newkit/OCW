use super::*;
use crate::road_mesh::{decode_b, road_meshes_rows};
use ocw_world::graph::RoadClass;
use ocw_world::junction::fixtures::{self, crossing, split, t_junction};
use ocw_world::road_dress::{Crossing, CrossingKind, Half, MarkingStyle};

const SIZE: f64 = 1000.0;

/// The fixture's strips per tile, split as the world builder splits them
/// (`split_into_tiles_indexed`: each run reaches one point into the next
/// tile).
fn tiles(g: &RoadGraph) -> HashMap<TileKey, Vec<RoadStrip>> {
    let mut out: HashMap<TileKey, Vec<RoadStrip>> = HashMap::new();
    for (_, s) in fixtures::strips(g) {
        let mut runs: Vec<(TileKey, Vec<[f32; 3]>)> = Vec::new();
        for (i, p) in s.points.iter().enumerate() {
            let key = TileKey::containing([f64::from(p[0]), f64::from(p[2])], SIZE);
            match runs.last_mut() {
                Some((k, run)) if *k == key => run.push(*p),
                last => {
                    if let Some((_, run)) = last {
                        run.push(*p);
                    }
                    let mut run = Vec::new();
                    if i > 0 {
                        run.push(s.points[i - 1]);
                    }
                    run.push(*p);
                    runs.push((key, run));
                }
            }
        }
        for (key, run) in runs {
            if run.len() >= 2 {
                out.entry(key).or_default().push(RoadStrip {
                    points: run,
                    ..s.clone()
                });
            }
        }
    }
    out
}

fn ctx_with<'a>(
    style: &'a MarkingStyle,
    ground: &'a dyn Fn(f32, f32) -> Option<f32>,
) -> RoadContext<'a> {
    RoadContext {
        colours: None,
        ground,
        deck: &|_| false,
        style,
    }
}

/// Every part of the drawn roads by owner (strip of an edge, patch).
fn parts(g: &RoadGraph) -> Vec<(Owner, RoadMeshes)> {
    let pj = PackageJunctions::of_graph(g, SIZE);
    let style = MarkingStyle::default();
    let ground = |_: f32, _: f32| Some(-50.0f32);
    let ctx = ctx_with(&style, &ground);
    let mut out = Vec::new();
    for (key, strips) in tiles(g) {
        let t = shape_tile(g, &pj, key, SIZE, &strips, &style);
        for i in 0..t.strips.len() {
            let rows = [t.rows[i].as_deref()];
            let bands = [t.bands[i].as_deref()];
            out.push((
                t.owners[i],
                crate::road_mesh::road_meshes_bands(
                    std::slice::from_ref(&t.strips[i]),
                    &rows,
                    &bands,
                    &ctx,
                ),
            ));
        }
        for (p, looks) in &t.patches {
            let mut m = RoadMeshes::default();
            patch_mesh(&mut m.roads, p, looks, &ctx);
            out.push((Owner::Patch(p.vertex), m));
        }
    }
    out
}

/// Up-facing carriageway triangles by owner.
fn surface_tris(parts: &[(Owner, RoadMeshes)], kinds: &[Kind]) -> Vec<(Owner, [Vec3; 3])> {
    let mut out = Vec::new();
    for (o, m) in parts {
        for t in m.roads.indices.as_chunks::<3>().0 {
            let k = decode_b(m.roads.uvs_b[t[0] as usize][0]).1;
            if !kinds.iter().any(|x| *x as u8 == k) {
                continue;
            }
            let p = t.map(|i| Vec3::from(m.roads.positions[i as usize]));
            if (p[1] - p[0]).cross(p[2] - p[0]).y.abs() < 1e-6 {
                continue;
            }
            out.push((*o, p));
        }
    }
    out
}

fn tri_height(t: &[Vec3; 3], x: f32, z: f32, tol: f32) -> Option<f32> {
    let p = Vec2::new(x, z);
    let (a, b, c) = (t[0].xz(), t[1].xz(), t[2].xz());
    let d = (b - a).perp_dot(c - a);
    let w1 = (p - a).perp_dot(c - a) / d;
    let w2 = (b - a).perp_dot(p - a) / d;
    let w0 = 1.0 - w1 - w2;
    (w0 >= -tol && w1 >= -tol && w2 >= -tol).then(|| w0 * t[0].y + w1 * t[1].y + w2 * t[2].y)
}

/// On the synthetic junctions the drawn carriageways neither overlap nor
/// step: at every sample of the junction, the surfaces of different owners
/// under it agree within 2 mm (only on their shared mouth lines do two
/// meet), and no shoulder, kerb or sidewalk lies above another owner's
/// carriageway; the roads are covered (no gap).
#[test]
fn junction_surfaces_meet_the_strips_without_steps_or_overlaps() {
    for (label, g) in [
        ("T 8 % / 20 %", t_junction(RoadClass::Secondary, 0.08, 0.20)),
        ("crossing", crossing([0.06, -0.04, 0.10, 0.0])),
        ("hairpin", split(150.0, 5.0, 5.0, 0.1)),
        ("width change", split(4.0, 12.0, 5.0, 0.03)),
    ] {
        let p = parts(&g);
        assert!(
            p.iter().any(|(o, _)| matches!(o, Owner::Patch(0))),
            "{label}: a patch"
        );
        let surf = surface_tris(&p, &[Kind::Surface]);
        let verge = surface_tris(&p, &[Kind::Shoulder, Kind::Kerb, Kind::Sidewalk]);
        let (mut worst, mut overlap_m2, mut verge_m2, mut gap_m2) = (0.0f32, 0.0, 0.0, 0.0);
        let a = 0.1 * 0.1;
        for ix in -200..=200 {
            for iz in -200..=200 {
                let (x, z) = (ix as f32 * 0.1 + 0.0137, iz as f32 * 0.1 + 0.0071);
                let hits: Vec<(Owner, f32)> = surf
                    .iter()
                    .filter_map(|(o, t)| tri_height(t, x, z, 1e-4).map(|h| (*o, h)))
                    .collect();
                for (oa, ha) in &hits {
                    for (ob, hb) in &hits {
                        if oa != ob {
                            worst = worst.max((ha - hb).abs());
                        }
                    }
                }
                let strict: Vec<Owner> = surf
                    .iter()
                    .filter(|(_, t)| tri_height(t, x, z, -1e-3).is_some())
                    .map(|(o, _)| *o)
                    .collect();
                if strict.iter().any(|o| strict.iter().any(|q| q != o)) {
                    overlap_m2 += a;
                }
                if let Some(top) = hits.iter().map(|h| h.1).reduce(f32::max) {
                    let owner = hits.iter().find(|h| h.1 == top).unwrap().0;
                    if verge.iter().any(|(o, t)| {
                        *o != owner && tri_height(t, x, z, -1e-3).is_some_and(|h| h > top + 0.01)
                    }) {
                        verge_m2 += a;
                    }
                }
                // Inside a road (centreline within half width − 0.1 m):
                // covered.
                let inside = g.edges.iter().any(|e| {
                    let d = e
                        .points
                        .windows(2)
                        .map(|w| {
                            let (pa, pb) = (
                                Vec2::new(w[0][0] as f32, w[0][1] as f32),
                                Vec2::new(w[1][0] as f32, w[1][1] as f32),
                            );
                            let ab = pb - pa;
                            // Beside the segment only (not the round caps
                            // at the vertex: no road there at a width
                            // change).
                            let t = (Vec2::new(x, z) - pa).dot(ab) / ab.length_squared();
                            if !(0.0..=1.0).contains(&t) {
                                return f32::MAX;
                            }
                            (pa + ab * t).distance(Vec2::new(x, z))
                        })
                        .fold(f32::MAX, f32::min);
                    d < e.width_m as f32 / 2.0 - 0.1
                });
                if inside && hits.is_empty() {
                    gap_m2 += a;
                }
            }
        }
        println!(
            "{label}: worst seam {worst:.4} m, overlap {overlap_m2:.3} m2, verge on road {verge_m2:.3} m2, gap {gap_m2:.3} m2"
        );
        assert!(worst < 0.002, "{label}: seam {worst}");
        assert!(overlap_m2 < 0.05, "{label}: overlap {overlap_m2}");
        assert!(verge_m2 < 0.05, "{label}: verge {verge_m2}");
        assert!(gap_m2 < 0.05, "{label}: gap {gap_m2}");
    }
}

/// The patch outline's shoulder starts where the arms' shoulders end: at
/// every mouth corner both have a shoulder vertex (row 0, the edge top).
#[test]
fn the_outline_dress_meets_the_arms_dress() {
    let g = t_junction(RoadClass::Secondary, 0.08, 0.20);
    let pj = PackageJunctions::of_graph(&g, SIZE);
    let p = pj.set.patch(&g, 0).unwrap();
    let parts = parts(&g);
    let walls = |want: fn(&Owner) -> bool| -> Vec<Vec3> {
        parts
            .iter()
            .filter(|(o, _)| want(o))
            .flat_map(|(_, m)| {
                m.roads
                    .positions
                    .iter()
                    .zip(&m.roads.uvs_b)
                    .filter(|(_, b)| decode_b(b[0]).1 == Kind::Wall as u8)
                    .map(|(q, _)| Vec3::from(*q))
                    .collect::<Vec<_>>()
            })
            .collect()
    };
    let (patch, arms) = (
        walls(|o| matches!(o, Owner::Patch(_))),
        walls(|o| matches!(o, Owner::Edge(_))),
    );
    for arm in &p.arms {
        for c in arm.row {
            let q = Vec3::new(
                c[0] as f32,
                c[1] as f32 + crate::ROAD_SURFACE_LIFT_M,
                c[2] as f32,
            );
            let near = |v: &[Vec3]| v.iter().any(|w| w.distance(q) < 1e-3);
            assert!(near(&patch) && near(&arms), "dress meets at {q:?}");
        }
    }
}

/// §4.8.8 Q7 follow-up ("that little hole"): with the ground a little
/// under the roads (a verge, not a wall), the verge round a junction's
/// corner meets the arms' verges at every mouth corner – the corner's
/// verge ends on the arm's verge's outer point, so no step and no hole –
/// and both are as wide as the ground's drop asks (J5: no wall).
#[test]
fn the_corner_verge_meets_the_arms_verge_without_a_hole() {
    for g in [
        fixtures::t_junction_built(RoadClass::Secondary, 0.0, 0.12),
        t_junction(RoadClass::Secondary, 0.04, 0.10),
    ] {
        let pj = PackageJunctions::of_graph(&g, SIZE);
        let p = pj.set.patch(&g, 0).unwrap().clone();
        let style = MarkingStyle::default();
        // The ground 0.5 m under the main road's level: a verge of 1.15 m
        // where it would follow the ground's drop.
        let ground = |_: f32, _: f32| Some(-0.5f32);
        let ctx = ctx_with(&style, &ground);
        let mut parts = Vec::new();
        for (key, strips) in tiles(&g) {
            let t = shape_tile(&g, &pj, key, SIZE, &strips, &style);
            for i in 0..t.strips.len() {
                let rows = [t.rows[i].as_deref()];
                parts.push((
                    t.owners[i],
                    road_meshes_rows(std::slice::from_ref(&t.strips[i]), &rows, &ctx),
                ));
            }
            for (q, looks) in &t.patches {
                let mut m = RoadMeshes::default();
                patch_mesh(&mut m.roads, q, looks, &ctx);
                parts.push((Owner::Patch(q.vertex), m));
            }
        }
        let shoulders = |want: fn(&Owner) -> bool| -> Vec<Vec3> {
            parts
                .iter()
                .filter(|(o, _)| want(o))
                .flat_map(|(_, m)| {
                    m.roads
                        .positions
                        .iter()
                        .zip(&m.roads.uvs_b)
                        .filter(|(_, b)| decode_b(b[0]).1 == Kind::Shoulder as u8)
                        .map(|(q, _)| Vec3::from(*q))
                        .collect::<Vec<_>>()
                })
                .collect()
        };
        let (patch, arms) = (
            shoulders(|o| matches!(o, Owner::Patch(_))),
            shoulders(|o| matches!(o, Owner::Edge(_))),
        );
        assert!(!patch.is_empty());
        let mut checked = 0;
        for arm in &p.arms {
            let (r0, r1) = (
                Vec2::new(arm.row[0][0] as f32, arm.row[0][2] as f32),
                Vec2::new(arm.row[1][0] as f32, arm.row[1][2] as f32),
            );
            let along = (r1 - r0).normalize();
            for (c, sign) in [(r0, -1.0f32), (r1, 1.0)] {
                // The arm's verge at the mouth corner: its points on the
                // mouth row's line, out from the corner – the outer one
                // where the ground's drop puts it (wider than 0.5 m: the ground
                // lies 0.18 m or more below), J4 kept the corner's at 0.4 m, a
                // wall where the ground lies low (§4.8.8 Rule, J5).
                let on_row: Vec<Vec3> = arms
                    .iter()
                    .filter(|q| {
                        let v = q.xz() - c;
                        let out = v.dot(along) * sign;
                        out > 0.05 && out < 4.0 && (v - along * v.dot(along)).length() < 0.02
                    })
                    .copied()
                    .collect();
                // A retaining wall where the ground lies more than 1.5 m
                // below (the side road climbing away): no verge to meet.
                if on_row.is_empty() {
                    continue;
                }
                checked += 1;
                let far = on_row
                    .iter()
                    .map(|q| q.xz().distance(c))
                    .fold(0.0f32, f32::max);
                assert!(far >= 0.5, "the arm's verge at {c:?}: {far} m");
                // The corner's verge has the same points there (no step,
                // no hole).
                for w in &on_row {
                    assert!(
                        patch.iter().any(|q| q.distance(*w) < 0.01),
                        "a hole at the mouth corner {c:?}: {w:?}"
                    );
                }
            }
        }
        assert!(checked >= 4, "{checked} mouth corners with a verge");
    }
}

/// §4.8.8 Rule, J6 (the V at Letzanaweg): where the ground lies well
/// below a junction whose corner has a small radius (a path or a track
/// leaving a road, a side road at an acute angle), the outer edge of the
/// corner's verge runs as one smooth curve from the main road's verge to
/// the side road's: no bend above the side lint's 25° over 0.3 m, also
/// where it leaves each arm's verge's outer edge; where the two arms'
/// verges cross beyond short mouths it lies within them (J5 cut the verge
/// to 0.8 of the corner's radius there – a V-shaped notch of grass between
/// the two wide verges).
#[test]
fn the_corner_verge_s_outer_edge_runs_on_smoothly() {
    use ocw_world::junction::fixtures::{Road, graph};
    let ground_y = -1.2f32;
    let mut checked = 0;
    for (label, side, angle) in [
        ("path at 90°", RoadClass::Path, 90.0f64),
        ("track at 90°", RoadClass::Track, 90.0),
        ("path at 55°", RoadClass::Path, 55.0),
        ("residential at 50°", RoadClass::Residential, 50.0),
    ] {
        let a = angle.to_radians();
        let v = [
            [0.0, 0.0],
            [-120.0, 0.0],
            [120.0, 0.0],
            [120.0 * a.cos(), 120.0 * a.sin()],
        ];
        let width = if side == RoadClass::Residential {
            5.0
        } else {
            2.5
        };
        let g = graph(
            &v,
            vec![
                Road::straight(&v, 1, 0, RoadClass::Residential, 6.0, |_| 0.0),
                Road::straight(&v, 0, 2, RoadClass::Residential, 6.0, |_| 0.0),
                Road::straight(&v, 0, 3, side, width, |_| 0.0),
            ],
        );
        let pj = PackageJunctions::of_graph(&g, SIZE);
        assert!(pj.set.patch(&g, 0).is_some(), "{label}: a patch");
        let style = MarkingStyle::default();
        let ground = move |_: f32, _: f32| Some(ground_y);
        let ctx = ctx_with(&style, &ground);
        let mut parts = Vec::new();
        for (key, strips) in tiles(&g) {
            let t = shape_tile(&g, &pj, key, SIZE, &strips, &style);
            for i in 0..t.strips.len() {
                let rows = [t.rows[i].as_deref()];
                parts.push((
                    t.owners[i],
                    road_meshes_rows(std::slice::from_ref(&t.strips[i]), &rows, &ctx),
                ));
            }
            for (q, looks) in &t.patches {
                let mut m = RoadMeshes::default();
                patch_mesh(&mut m.roads, q, looks, &ctx);
                parts.push((Owner::Patch(q.vertex), m));
            }
        }
        // The verges' outer edges: their shoulder vertices down at the
        // ground, in mesh order (a chain's in order along it).
        let outer = |want: fn(&Owner) -> bool, gap: f32| -> Vec<Vec<Vec2>> {
            let mut runs: Vec<Vec<Vec2>> = Vec::new();
            for (_, m) in parts.iter().filter(|(o, _)| want(o)) {
                let mut run: Vec<Vec2> = Vec::new();
                for (q, b) in m.roads.positions.iter().zip(&m.roads.uvs_b) {
                    if decode_b(b[0]).1 != Kind::Shoulder as u8 || q[1] > ground_y + 0.2 {
                        continue;
                    }
                    let p = Vec2::new(q[0], q[2]);
                    // (A strip's rows lie up to 6 m apart, a corner's points close.)
                    if run.last().is_some_and(|l| l.distance(p) > gap) {
                        runs.push(std::mem::take(&mut run));
                    }
                    run.push(p);
                }
                runs.push(run);
            }
            runs.retain(|r| r.len() >= 2);
            runs
        };
        let (corners, arms) = (
            outer(|o| matches!(o, Owner::Patch(_)), 2.5),
            outer(|o| matches!(o, Owner::Edge(_)), 6.5),
        );
        let arm_pts: Vec<Vec2> = arms.iter().flatten().copied().collect();
        for c in &corners {
            let mut cum = vec![0.0f32];
            for w in c.windows(2) {
                cum.push(cum.last().unwrap() + w[0].distance(w[1]));
            }
            let total = *cum.last().unwrap();
            let at = |s: f32| -> Vec2 {
                let k = cum.partition_point(|&x| x <= s).clamp(1, c.len() - 1);
                let l = (cum[k] - cum[k - 1]).max(1e-6);
                c[k - 1].lerp(c[k], ((s - cum[k - 1]) / l).clamp(0.0, 1.0))
            };
            // Each end: on an arm's verge's outer point; the direction of
            // that verge's outer edge arriving there (from away from the
            // junction, the vertex at 0, 0).
            let mut arrive = Vec::new();
            for end in [c[0], c[c.len() - 1]] {
                assert!(
                    arm_pts.iter().any(|q| q.distance(end) < 0.01),
                    "{label}: the corner's verge ends at {end:?} off the arm's verge ({:?})",
                    arm_pts
                        .iter()
                        .filter(|q| q.distance(end) < 2.0)
                        .collect::<Vec<_>>()
                );
                let run = arms
                    .iter()
                    .find(|r| r.iter().any(|q| q.distance(end) < 0.01))
                    .unwrap();
                let k = run.iter().position(|q| q.distance(end) < 0.01).unwrap();
                let beyond = [k.wrapping_sub(1), k + 1]
                    .into_iter()
                    .filter_map(|j| run.get(j))
                    .copied()
                    .max_by(|a, b| a.length().total_cmp(&b.length()))
                    .unwrap();
                arrive.push((end - beyond).normalize());
            }
            // Where the two arms' verges cross beyond the mouths (a verge
            // wider than the corner leaves room for) they cover the
            // corner's outside: its verge lies within them (between its
            // ends and their outer edges' crossing).
            let (e0, e1) = (c[0], c[c.len() - 1]);
            let (o0, o1) = (-arrive[0], -arrive[1]);
            let den = o0.perp_dot(o1);
            let s0 = (e1 - e0).perp_dot(o1) / den;
            let s1 = (e1 - e0).perp_dot(o0) / den;
            if den.abs() > 1e-4 && s0 > 0.0 && s1 > 0.0 {
                let x = e0 + o0 * s0;
                let area = |a: Vec2, b: Vec2, p: Vec2| (b - a).perp_dot(p - a);
                let sign = area(e0, x, e1).signum();
                for p in c {
                    let inside = [area(e0, x, *p), area(x, e1, *p), area(e1, e0, *p)]
                        .iter()
                        .all(|v| v * sign >= -0.01);
                    assert!(
                        inside,
                        "{label}: the corner's verge at {p:?} reaches out of the arms' verges"
                    );
                }
                checked += 2;
                continue;
            }
            // Along the corner: no bend above the side lint's 25° over
            // pieces of 0.3 m (a curve tighter than 0.69 m).
            let h = 0.3f32;
            let mut s = h;
            while s + h <= total {
                let (p0, p1, p2) = (at(s - h), at(s), at(s + h));
                let turn = (p1 - p0).angle_to(p2 - p1).abs().to_degrees();
                assert!(
                    turn <= 25.0,
                    "{label}: the corner's verge bends by {turn:.0}° at {p1:?} (its outer edge {c:?})"
                );
                s += 0.1;
            }
            // At each end it leaves along the arm's verge's outer edge: no bend
            // above 25° there either (over 0.3 m, as the side lint measures).
            for (k, (end, next)) in [(c[0], at(0.3)), (c[c.len() - 1], at(total - 0.3))]
                .into_iter()
                .enumerate()
            {
                let deg = arrive[k]
                    .angle_to((next - end).normalize())
                    .abs()
                    .to_degrees();
                assert!(
                    deg <= 25.0,
                    "{label}: the verges' outer edges meet at {end:?} at {deg:.1}°"
                );
                checked += 1;
            }
        }
    }
    assert!(checked >= 16, "{checked} joints checked");
}

/// Markings at the mouths (§4.8.8 Q2 fallback): the patch of a through
/// road carries its centre line; its edge line only on the side without a
/// side road. Without a through road no lines.
#[test]
fn a_through_road_s_centre_line_runs_on_and_its_edge_line_breaks_at_the_mouth() {
    let g = t_junction(RoadClass::Secondary, 0.0, 0.0);
    let parts = parts(&g);
    let patch = &parts
        .iter()
        .find(|(o, _)| matches!(o, Owner::Patch(_)))
        .unwrap()
        .1
        .roads;
    let mut with_edges = (0, 0);
    for t in patch.indices.as_chunks::<3>().0 {
        let b = patch.uvs_b[t[0] as usize];
        if decode_b(b[0]).1 != Kind::Surface as u8 {
            continue;
        }
        let code = MarkCode::decode(b[1] as u32, decode_b(b[0]).2);
        assert_ne!(
            code.centre,
            ocw_world::road_dress::CentreLine::None,
            "centre line"
        );
        let z = t
            .iter()
            .map(|&i| patch.positions[i as usize][2])
            .fold(f32::MIN, f32::max);
        if code.edge_lines {
            with_edges.0 += 1;
            // The side road leaves to +z: no edge line reaches it.
            assert!(z < 3.75 - 1.0 + 1e-3, "edge line towards the mouth: {z}");
        } else {
            with_edges.1 += 1;
        }
    }
    assert!(with_edges.0 > 0 && with_edges.1 > 0, "{with_edges:?}");
    let parts = parts_of_crossing();
    let patch = &parts
        .iter()
        .find(|(o, _)| matches!(o, Owner::Patch(_)))
        .unwrap()
        .1
        .roads;
    assert!(
        patch
            .uvs_b
            .iter()
            .filter(|b| decode_b(b[0]).1 == Kind::Surface as u8)
            .all(|b| b[1] == 0.0)
    );
}

fn parts_of_crossing() -> Vec<(Owner, RoadMeshes)> {
    parts(&crossing([0.0; 4]))
}

/// A give-way line tagged near the vertex (inside the patch now) moves to
/// the side road's mouth (§4.8.8 Q3: only where OSM tags it).
#[test]
fn give_way_lines_move_to_the_mouth() {
    let g = t_junction(RoadClass::Secondary, 0.0, 0.05);
    let pj = PackageJunctions::of_graph(&g, SIZE);
    let p = pj.set.patch(&g, 0).unwrap();
    let side = p
        .arms
        .iter()
        .find(|a| a.class == RoadClass::Residential)
        .unwrap();
    let mut strips = fixtures::strips(&g);
    let s = &mut strips[side.edge].1;
    s.dress.crossings = vec![Crossing {
        along_m: 1.5,
        kind: CrossingKind::GiveWay,
        half: Half::Both,
    }];
    let style = MarkingStyle::default();
    let ground = |_: f32, _: f32| Some(-50.0f32);
    let ctx = ctx_with(&style, &ground);
    let all: Vec<RoadStrip> = strips.into_iter().map(|x| x.1).collect();
    let t = shape_tile(&g, &pj, TileKey { x: 0, z: 0 }, SIZE, &all, &style);
    let m = shaped_meshes(&t, &ctx);
    assert!(!m.overlays.positions.is_empty(), "the line is drawn");
    // All of it beyond the mouth (z > mouth − the overlay margin).
    let mouth = side.mouth_m as f32;
    for q in &m.overlays.positions {
        assert!(
            q[2] > mouth - 0.3,
            "{q:?} inside the patch (mouth at {mouth})"
        );
    }
}

/// The terrain under a patch lies under its surface, and where the side
/// road's blend lowers the road below its profile (here the 20 % side road
/// starts at the main road's height) the ground beside it goes down by as
/// much; far away nothing changes.
#[test]
fn the_terrain_is_lowered_under_a_patch_and_a_lowered_side_road() {
    let g = t_junction(RoadClass::Secondary, 0.0, 0.2);
    let pj = PackageJunctions::of_graph(&g, SIZE);
    let p = pj.set.patch(&g, 0).unwrap();
    // A 5 m grid from (−50, −50), the ground 5 cm under each road's profile
    // (as the world builder settles it): the side road climbs to +z.
    let grid = TerrainGrid {
        origin: [-50.0, -50.0],
        spacing: 5.0,
        n: 21,
    };
    let ground = |_x: f64, z: f64| if z > 3.75 { 0.2 * z - 0.05 } else { -0.05 };
    let mut terrain = MeshArrays::default();
    for k in 0..grid.n {
        for i in 0..grid.n {
            let (x, z) = (-50.0 + 5.0 * i as f64, -50.0 + 5.0 * k as f64);
            terrain
                .positions
                .push([x as f32, ground(x, z) as f32, z as f32]);
        }
    }
    let before = terrain.positions.clone();
    lower_terrain(&mut terrain, &g, &[p], grid);
    let at = |x: f64, z: f64| {
        let i = ((x + 50.0) / 5.0) as usize + 21 * ((z + 50.0) / 5.0) as usize;
        (before[i][1], terrain.positions[i][1])
    };
    // Under the patch.
    let (_, y0) = at(0.0, 0.0);
    let h0 = p.height_at([0.0, 0.0]).unwrap() as f32;
    assert!(y0 <= h0 + crate::ROAD_SURFACE_LIFT_M - TERRAIN_UNDER_M + 1e-4);
    // Beside the side road: in its corner (J3, §4.8.8 Q6 (d)) its drawn
    // surface lies below its profile (it leaves the main road's edge at
    // the main road's height); the ground goes down with it, staying under
    // the road.
    let side = p.arms.iter().position(|a| a.has_corner()).unwrap();
    let sunk = JunctionSet::sunk(&g, p, side);
    assert!(!sunk.is_empty());
    let (c, delta) = sunk[sunk.len() / 2];
    let (b, a) = at((c[0] / 5.0).round() * 5.0, (c[1] / 5.0).round() * 5.0);
    assert!(a < b - 0.5 * (-delta) as f32, "{b} -> {a} (road {delta})");
    // Far from the junction: unchanged.
    assert_eq!(at(-45.0, -45.0).0, at(-45.0, -45.0).1);
    assert_eq!(at(45.0, 45.0).0, at(45.0, 45.0).1);
}

/// A J2 package's stored junctions (`JNCT`) and strip edges (`RDGE`)
/// shape every tile exactly as the computed junctions and the geometric
/// matching do, and mark the terrain as settled (no lowering at load).
#[test]
fn stored_junctions_and_strip_edges_shape_the_same_tiles() {
    use ocw_world::junction::section::{FLAG_SETTLED, JunctionData, JunctionIndex, strip_edges_of};
    for g in [
        t_junction(RoadClass::Secondary, 0.08, 0.20),
        crossing([0.05, -0.04, 0.10, 0.0]),
    ] {
        let mut pkg = WorldPackage {
            name: "Fixture".into(),
            origin: (47.0, 9.0),
            tile_size_m: SIZE as f32,
            samples: 2,
            tiles: tiles(&g)
                .into_iter()
                .map(|(key, roads)| {
                    (
                        key,
                        ocw_world::package::Tile {
                            key,
                            heights: vec![0.0; 4],
                            roads,
                        },
                    )
                })
                .collect(),
            graph: Some(g.clone().into()),
            landuse: None,
            buildings: None,
            bridges: None,
            routes: vec![],
            region: None,
            time_zone: None,
            look: Default::default(),
            obstacles: None,
            places: None,
            junctions: None,
        };
        let plain = PackageJunctions::new(&pkg).unwrap();
        assert!(!plain.settled && !plain.set.is_stored());
        let index = JunctionIndex::of_set(&g, &JunctionSet::new(&g), FLAG_SETTLED);
        pkg.junctions = Some(Arc::new(JunctionData {
            index: Some(Arc::new(index)),
            strip_edges: Some(strip_edges_of(&pkg, &g)),
            mini_roundabouts: None,
        }));
        let stored = PackageJunctions::new(&pkg).unwrap();
        assert!(stored.settled && stored.set.is_stored());
        let style = MarkingStyle::default();
        for (key, t) in &pkg.tiles {
            assert_eq!(
                stored.strip_edges(&g, *key, &t.roads),
                plain.strip_edges(&g, *key, &t.roads)
            );
            let a = shape_tile(&g, &plain, *key, SIZE, &t.roads, &style);
            let b = shape_tile(&g, &stored, *key, SIZE, &t.roads, &style);
            assert_eq!(a.strips, b.strips);
            assert_eq!(a.rows, b.rows);
            assert_eq!(a.owners, b.owners);
            let pa: Vec<&Patch> = a.patches.iter().map(|p| p.0).collect();
            let pb: Vec<&Patch> = b.patches.iter().map(|p| p.0).collect();
            assert_eq!(pa, pb);
        }
        // A tile whose strips changed since (another count): matched by
        // geometry again.
        let key = *pkg.tiles.keys().next().unwrap();
        let mut fewer = pkg.tiles[&key].roads.clone();
        fewer.pop();
        assert_eq!(
            stored.strip_edges(&g, key, &fewer),
            plain.strip_edges(&g, key, &fewer)
        );
    }
}

/// J3: a mini-roundabout (section `JMRB`) gets a disc on its junction
/// surface: an overlay of the disc code round the vertex, at the patch's
/// drawn heights (lifted as the other overlays), its radius from the
/// narrowest road; a junction without one gets none.
#[test]
fn mini_roundabouts_get_a_painted_disc() {
    let g = crossing([0.04, -0.02, 0.03, 0.0]);
    let style = MarkingStyle::default();
    let ground = |_: f32, _: f32| None;
    let ctx = ctx_with(&style, &ground);
    let key = TileKey::containing([0.0, 0.0], SIZE);
    let strips = tiles(&g).remove(&key).unwrap();
    let pj_plain = PackageJunctions::of_graph(&g, SIZE);
    let plain = shape_tile(&g, &pj_plain, key, SIZE, &strips, &style);
    assert!(plain.discs.is_empty());
    let mut pj = PackageJunctions::of_graph(&g, SIZE);
    pj.mini_roundabouts = vec![0];
    let t = shape_tile(&g, &pj, key, SIZE, &strips, &style);
    assert_eq!(t.discs.len(), 1);
    let (p, c) = t.discs[0];
    assert_eq!(c, [0.0, 0.0]);
    let r = mini_disc_radius(p);
    assert!((r - 1.5).abs() < 1e-9, "{r}");
    let meshes = shaped_meshes(&t, &ctx);
    let lift = crate::ROAD_SURFACE_LIFT_M + crate::road_mesh::OVERLAY_LIFT_M;
    let disc: Vec<usize> = (0..meshes.overlays.positions.len())
        .filter(|&i| meshes.overlays.uvs_b[i][1] == MINI_DISC_CODE)
        .collect();
    assert!(disc.len() > 50, "{}", disc.len());
    for &i in &disc {
        let q = meshes.overlays.positions[i];
        let uv = meshes.overlays.uvs[i];
        assert!((f64::from(uv[0]) - f64::from(q[0])).abs() < 1e-4);
        assert!((meshes.overlays.colors[i][3] - r as f32).abs() < 1e-6);
        let h = p.height_at([f64::from(q[0]), f64::from(q[2])]).unwrap();
        assert!((q[1] - (h as f32 + lift)).abs() < 1e-4);
    }
    assert!(
        shaped_meshes(&plain, &ctx)
            .overlays
            .uvs_b
            .iter()
            .all(|b| b[1] != MINI_DISC_CODE)
    );
}

/// Spec §4.8.8 Rule (J7): sidewalks OSM draws as ways of their own along
/// both streets of a crossing are drawn as a band of their street's side –
/// no strip of their own, the street's sidewalk reaching over them – and
/// the band goes on round the junction's corner, one band from the one
/// street to the other (J6: the sidewalks as roads with their own mouths
/// at the junction, the outline turning back at them).
#[test]
fn sidewalk_ways_are_drawn_as_a_band_of_their_street_s_side() {
    let g = fixtures::crossing_with_sidewalks();
    let side = PackageJunctions::of_graph(&g, SIZE)
        .set
        .side_paths()
        .to_vec();
    assert_eq!(side.iter().filter(|&&x| x).count(), 6);
    let parts = parts(&g);
    for (o, _) in &parts {
        assert!(!matches!(o, Owner::Edge(e) if side[*e]), "{o:?} drawn");
    }
    let walk = surface_tris(&parts, &[Kind::Sidewalk]);
    let on = |x: f32, z: f32| {
        walk.iter()
            .any(|(_, t)| tri_height(t, x, z, 1e-4).is_some())
    };
    // Along both streets over the sidewalks' centre lines.
    for k in 0..30 {
        let d = 12.0 + k as f32;
        assert!(on(d, 5.0), "along x at {d}");
        assert!(on(5.0, d), "along z at {d}");
    }
    // Round the corner (radius 6 m from the streets' edges at z 3.5 and
    // x 3): the middle of the band, 1.25 m beyond the kerb.
    let c = Vec2::new(9.0, 9.5);
    for k in 0..=18 {
        let a = std::f32::consts::PI * (1.0 + 0.5 * k as f32 / 18.0);
        let q = c + Vec2::new(a.cos(), a.sin()) * (6.0 - 1.25);
        assert!(on(q.x, q.y), "round the corner at {q}");
    }
}
