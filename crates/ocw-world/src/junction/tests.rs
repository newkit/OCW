use super::fixtures::{self, crossing, fork, split, t_junction};
use super::*;

fn patch_at(g: &RoadGraph, v: usize) -> Patch {
    JunctionSet::new(g)
        .patch(g, v)
        .cloned()
        .unwrap_or_else(|| panic!("a patch at vertex {v}"))
}

/// The outline is closed and simple (star-shaped round the vertex: the
/// polar angles grow once round), every mouth meets its arm.
fn check_outline(p: &Patch) {
    let c = xz(p.centre);
    let n = p.outline.len();
    let mut total = 0.0;
    for k in 0..n {
        let (a, b) = (xz(p.outline[k].p), xz(p.outline[(k + 1) % n].p));
        let mut d = angle(c, b) - angle(c, a);
        while d <= 0.0 {
            d += std::f64::consts::TAU;
        }
        assert!(d < std::f64::consts::PI, "outline turns back at {k}");
        total += d;
    }
    assert!(
        (total - std::f64::consts::TAU).abs() < 1e-6,
        "once round: {total}"
    );
    for (i, a) in p.arms.iter().enumerate() {
        let on: Vec<&OutlinePoint> = p.outline.iter().filter(|o| o.mouth == Some(i)).collect();
        assert!(on.len() >= 2);
        // The mouth points lie on the mouth row (their offsets from the
        // vertex rounded to `f32`, as section `JNCT` stores them: µm).
        for o in on {
            assert!(seg_dist(xz(o.p), xz(a.row[0]), xz(a.row[1])) < 1e-5);
        }
    }
}

/// Heights at the mouth (on the patch mesh, a little inside) and slopes
/// across it match the arm (on the strip side: its profile or blend).
fn check_mouths(g: &RoadGraph, p: &Patch) {
    for (i, a) in p.arms.iter().enumerate() {
        for f in [0.25, 0.5, 0.75] {
            let q = add(xz(a.row[0]), mul(sub(xz(a.row[1]), xz(a.row[0])), f));
            let on_row = a.row[0][1] + (a.row[1][1] - a.row[0][1]) * f;
            // Just inside the patch: the mesh height.
            let inside = sub(q, mul(a.t, 1e-4));
            let h = p.height_at(inside).expect("inside the patch");
            assert!((h - on_row).abs() < 1e-3, "arm {i} height {h} vs {on_row}");
            // The field's slope along the arm at the mouth = the arm's
            // (outward of the mouth: its blend or profile).
            let d = 0.02;
            let slope_in = (p.field(q) - p.field(sub(q, mul(a.t, d)))) / d;
            let e = &g.edges[a.edge];
            let s_out = a.s_edge(a.mouth_m + d);
            let out_h = if a.blend_m > 0.0 {
                // The blend itself (the drawn rows are its chords, 1 m apart).
                let r = JunctionSet::blend_row(g, p, i, a.mouth_m + d);
                r[0][1] + (r[1][1] - r[0][1]) * f
            } else {
                height(e, s_out)
            };
            let slope_out = (out_h - on_row) / d;
            assert!(
                (slope_in - slope_out).abs() < 0.005 + 0.005 * slope_out.abs(),
                "arm {i}: slope {slope_in} inside, {slope_out} outside"
            );
        }
    }
}

#[test]
fn a_t_junction_keeps_the_main_road_and_blends_the_side_road() {
    let g = t_junction(RoadClass::Secondary, 0.08, 0.20);
    let p = patch_at(&g, 0);
    assert_eq!(p.arms.len(), 3);
    let [a, b] = p.through.expect("a through road");
    assert_eq!(
        [g.edges[p.arms[a].edge].class, g.edges[p.arms[b].edge].class],
        [RoadClass::Secondary; 2]
    );
    check_outline(&p);
    check_mouths(&g, &p);
    // The main road keeps its own surface: along its centre line the patch
    // follows its grade (8 %), whatever the side road does.
    for x in [-4.0, -2.0, 0.0, 2.0, 4.0] {
        let h = p.height_at([x, 0.0]).unwrap();
        assert!((h - 0.08 * x).abs() < 0.02, "x {x}: {h}");
    }
    // The side arm: its mouth row lies on the main road's surface, the
    // blend reaches its own profile, and its length follows the rule.
    let side = (0..3).find(|i| ![a, b].contains(i)).unwrap();
    let s = &p.arms[side];
    assert!(
        s.blend_m >= BLEND_MIN_M && s.blend_m <= BLEND_MAX_M,
        "{}",
        s.blend_m
    );
    for q in s.row {
        assert!((q[1] - p.field(xz(q))).abs() < 1e-9);
    }
    let end = JunctionSet::blend_row(&g, &p, side, s.mouth_m + s.blend_m);
    let e = &g.edges[s.edge];
    let h_end = height(e, s.s_edge(s.mouth_m + s.blend_m));
    assert!(end.iter().all(|q| (q[1] - h_end).abs() < 1e-9));
    // The blend climbs steadily (no dip, no hump).
    let mut prev = f64::NEG_INFINITY;
    for st in JunctionSet::blend_steps(s) {
        let r = JunctionSet::blend_row(&g, &p, side, st);
        let h = 0.5 * (r[0][1] + r[1][1]);
        assert!(h >= prev - 1e-9, "{st}: {h} after {prev}");
        prev = h;
    }
    // A corner arc between the main road and the side road: radius 6 m
    // (residential, the smaller road), the outline reaches beyond the
    // square corner at (±2.5, 3.75).
    let corner = p
        .outline
        .iter()
        .filter(|o| o.mouth.is_none())
        .any(|o| o.p[2] > 3.8 && o.p[0].abs() > 2.6);
    assert!(corner, "rounded corners");
}

#[test]
fn an_equal_crossing_blends_all_arms_within_their_heights() {
    let g = crossing([0.06, -0.04, 0.10, 0.0]);
    let p = patch_at(&g, 0);
    assert!(
        p.through.is_none(),
        "four equal arms: no clear through road"
    );
    check_outline(&p);
    check_mouths(&g, &p);
    let (lo, hi) = p
        .arms
        .iter()
        .map(|a| a.h)
        .fold((f64::MAX, f64::MIN), |(l, h), x| (l.min(x), h.max(x)));
    for v in &p.vertices {
        assert!(
            v[1] >= lo - 0.15 && v[1] <= hi + 0.15,
            "{v:?} outside {lo}..{hi}"
        );
    }
}

#[test]
fn forks_get_a_patch_unless_too_narrow() {
    let p = patch_at(&fork(40.0), 0);
    check_outline(&p);
    check_mouths(&fork(40.0), &p);
    // A 10° fork: the arms overlap far out, no patch (overlapping strips).
    assert!(
        JunctionSet::new(&fork(10.0))
            .patch(&fork(10.0), 0)
            .is_none()
    );
}

#[test]
fn way_splits_mitre_small_bends_and_patch_large_ones() {
    // 12°: mitred, both strips end on the same two points.
    let g = split(12.0, 6.0, 6.0, 0.05);
    let set = JunctionSet::new(&g);
    let Some(VertexRule::Mitre(m)) = set.rule(&g, 0) else {
        panic!("a mitre")
    };
    let pts = |r: [[f64; 3]; 2]| {
        let mut v = vec![r[0], r[1]];
        v.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[2].total_cmp(&b[2])));
        v
    };
    assert_eq!(pts(m.ends[0].2), pts(m.ends[1].2));
    // The mitre lies on the bisector, a half width (over cos) from the
    // vertex.
    for q in m.ends[0].2 {
        let r = norm(xz(q));
        assert!((r - 3.0 / (6f64.to_radians()).cos()).abs() < 1e-6, "{r}");
    }
    // A hairpin (150°) and a width change get patches.
    for (label, g) in [
        ("hairpin", split(150.0, 5.0, 5.0, 0.1)),
        ("width change", split(5.0, 12.0, 5.0, 0.02)),
    ] {
        let set = JunctionSet::new(&g);
        let p = set
            .patch(&g, 0)
            .unwrap_or_else(|| panic!("{label}: a patch"));
        check_outline(p);
        check_mouths(&g, p);
    }
    // Nearly straight: nothing.
    let g = split(0.2, 6.0, 6.0, 0.0);
    assert!(JunctionSet::new(&g).rule(&g, 0).is_none());
}

#[test]
fn the_result_does_not_depend_on_the_edge_order() {
    let g = t_junction(RoadClass::Primary, 0.04, -0.12);
    let mut r = g.clone();
    r.edges.reverse();
    let (p, q) = (patch_at(&g, 0), patch_at(&r, 0));
    assert_eq!(p.outline.len(), q.outline.len());
    // The same outline, starting at the same arm (sorted by angle).
    for (a, b) in p.outline.iter().zip(&q.outline) {
        for k in 0..3 {
            assert!((a.p[k] - b.p[k]).abs() < 1e-9);
        }
    }
    for (a, b) in p.vertices.iter().zip(&q.vertices) {
        assert!((a[1] - b[1]).abs() < 1e-9);
    }
}

/// The trimmed strips and the patch do not overlap, and between them they
/// cover the roads (sampled every 0.1 m).
#[test]
fn trimmed_strips_meet_the_patch_without_overlap() {
    for g in [
        t_junction(RoadClass::Secondary, 0.08, 0.20),
        crossing([0.02, 0.05, -0.03, 0.08]),
        split(150.0, 5.0, 5.0, 0.1),
    ] {
        let set = JunctionSet::new(&g);
        let p = set.patch(&g, 0).unwrap();
        let shaped: Vec<Shaped> = fixtures::strips(&g)
            .iter()
            .map(|(ei, s)| {
                set.shape_strip(&g, *ei, &s.points, s.width_m / 2.0)
                    .expect("shaped at the junction")
            })
            .collect();
        let in_strip = |q: P2| {
            shaped.iter().any(|sh| {
                sh.rows.windows(2).any(|w| {
                    let [l0, r0] = w[0].map(|x| x.map(f64::from));
                    let [l1, r1] = w[1].map(|x| x.map(f64::from));
                    tri_height([l0, r0, l1], q, -1e-6).is_some()
                        || tri_height([r0, r1, l1], q, -1e-6).is_some()
                })
            })
        };
        let mut overlap = 0.0;
        let a = 0.01;
        for ix in -150..=150 {
            for iz in -150..=150 {
                let q = [ix as f64 * 0.1 + 0.013, iz as f64 * 0.1 + 0.007];
                let inside = p.height_at(q).is_some() && {
                    // Strictly inside (not on the mouth line).
                    p.arms.iter().all(|arm| arm.mouth_dist(q) > 1e-4)
                };
                if inside && in_strip(q) {
                    overlap += a;
                }
            }
        }
        assert!(overlap < 0.01, "overlap {overlap} m2");
        // Mouth rows of the strips are the patch's mouth rows.
        for arm in &p.arms {
            let sh = &shaped[arm.edge];
            let row = if arm.at_from {
                sh.rows[0]
            } else {
                *sh.rows.last().unwrap()
            };
            for (got, want) in row.iter().zip(&arm.row) {
                for (a, b) in got.iter().zip(want) {
                    assert!((f64::from(*a) - b).abs() < 1e-4);
                }
            }
        }
    }
}

/// What a rider rides is what is drawn: inside the patch the patch mesh,
/// along a blended side arm the strip's rows.
#[test]
fn drawn_heights_are_the_meshes() {
    let g = t_junction(RoadClass::Secondary, 0.08, 0.20);
    let set = JunctionSet::new(&g);
    let p = set.patch(&g, 0).unwrap();
    let side = p.arms.iter().position(|a| a.blend_m > 0.0).unwrap();
    let a = &p.arms[side];
    let (strips, ei) = (fixtures::strips(&g), a.edge);
    let s = &strips[ei].1;
    let sh = set.shape_strip(&g, ei, &s.points, s.width_m / 2.0).unwrap();
    // Every point of the shaped strip's quads: drawn_height equals the
    // triangle's height.
    for w in sh.rows.windows(2) {
        let [l0, r0] = w[0].map(|x| x.map(f64::from));
        let [l1, r1] = w[1].map(|x| x.map(f64::from));
        for (u, t) in [(0.3, 0.2), (0.7, 0.6), (0.5, 0.5), (0.1, 0.9)] {
            let left = lerp3(l0, l1, t);
            let right = lerp3(r0, r1, t);
            let q = lerp3(left, right, u);
            let tri = |tri| tri_height(tri, xz(q), 1e-9);
            let want = tri([l0, r0, l1]).or_else(|| tri([r0, r1, l1])).unwrap();
            let cen = lerp3(lerp3(l0, r0, 0.5), lerp3(l1, r1, 0.5), t);
            let s_edge = strip_distances(
                &g.edges[ei],
                &cumulative(&g.edges[ei].points),
                &[[cen[0] as f32, 0.0, cen[2] as f32]],
            )[0];
            let got = set
                .drawn_height(&g, ei, s_edge, xz(q))
                .unwrap_or_else(|| height(&g.edges[ei], s_edge));
            assert!((got - want).abs() < 2e-3, "{q:?}: {got} vs {want}");
        }
    }
    // Inside the patch: the mesh.
    for tri in &p.triangles {
        let t3 = tri.map(|i| p.vertices[i as usize]);
        let c = [
            (t3[0][0] + t3[1][0] + t3[2][0]) / 3.0,
            (t3[0][2] + t3[1][2] + t3[2][2]) / 3.0,
        ];
        let want = tri_height(t3, c, 1e-9);
        if let Some(want) = want {
            let got = set.patch_height(&g, 0, c).unwrap();
            assert!((got - want).abs() < 1e-6);
        }
    }
}

#[test]
fn strips_far_from_a_junction_stay_as_they_are() {
    let g = t_junction(RoadClass::Secondary, 0.08, 0.2);
    let set = JunctionSet::new(&g);
    let strips = fixtures::strips(&g);
    let (ei, s) = &strips[1];
    // The part of the strip beyond 40 m from the vertex.
    let far: Vec<[f32; 3]> = s.points.iter().copied().filter(|p| p[0] > 40.0).collect();
    assert!(set.shape_strip(&g, *ei, &far, 3.75).is_none(), "unchanged");
    // From 8 m on (beyond the 7.5 m mouth, within reach): kept as it is.
    let near: Vec<[f32; 3]> = s.points.iter().copied().filter(|p| p[0] > 7.9).collect();
    let mouth = set
        .patch(&g, 0)
        .unwrap()
        .arms
        .iter()
        .find(|a| a.edge == *ei)
        .unwrap()
        .mouth_m;
    if f64::from(near[0][0]) > mouth {
        assert!(
            set.shape_strip(&g, *ei, &near, 3.75)
                .is_none_or(|sh| sh.points == near)
        );
    }
    // Its far end is a dead end: no rule there.
    assert!(set.ends(&g, *ei)[1].is_none());
}

/// Some packages store a strip against its edge's direction: it is shaped
/// the same (turned back, its left and right swapped), not dropped.
#[test]
fn strips_against_their_edge_are_shaped_the_same() {
    let g = t_junction(RoadClass::Secondary, 0.08, 0.2);
    let set = JunctionSet::new(&g);
    for (ei, s) in fixtures::strips(&g) {
        let fwd = set.shape_strip(&g, ei, &s.points, 2.5).unwrap();
        let back: Vec<[f32; 3]> = s.points.iter().rev().copied().collect();
        assert!(strip_reversed(&g.edges[ei], &back));
        let rev = set.shape_strip(&g, ei, &back, 2.5).unwrap();
        assert_eq!(rev.points.len(), fwd.points.len(), "edge {ei}");
        let n = fwd.points.len();
        for i in 0..n {
            assert_eq!(rev.points[n - 1 - i], fwd.points[i]);
            assert_eq!(rev.rows[n - 1 - i], [fwd.rows[i][1], fwd.rows[i][0]]);
        }
        assert_eq!(rev.junction_end, [fwd.junction_end[1], fwd.junction_end[0]]);
    }
}
