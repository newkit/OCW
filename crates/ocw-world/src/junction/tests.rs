use super::fixtures::{self, crossing, fork, split, t_junction};
use super::*;

fn patch_at(g: &RoadGraph, v: usize) -> Patch {
    JunctionSet::new(g)
        .patch(g, v)
        .cloned()
        .unwrap_or_else(|| panic!("a patch at vertex {v}"))
}

/// The outline is closed and simple (star-shaped round the vertex: the
/// polar angles grow once round; J6: else a simple polygon, counter-
/// clockwise), every mouth meets its arm.
fn check_outline(p: &Patch) {
    let c = xz(p.centre);
    let n = p.outline.len();
    let mut total = 0.0;
    if !p.is_star() {
        let pts: Vec<P2> = p.outline.iter().map(|o| xz(o.p)).collect();
        assert!(plane_mesh::simple_ccw(&pts), "the outline is not simple");
        total = std::f64::consts::TAU;
    }
    for k in 0..n {
        if !p.is_star() {
            break;
        }
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
                // Its own profile, leaning as the mouth row (J5: the lean
                // turns flat from the mouth on with a level start).
                height(e, s_out) + (on_row - a.h)
            };
            let slope_out = (out_h - on_row) / d;
            assert!(
                (slope_in - slope_out).abs() < 0.005 + 0.005 * slope_out.abs(),
                "arm {i}: slope {slope_in} inside, {slope_out} outside"
            );
        }
    }
}

/// §4.8.8 Q6 (d): a side road leaving a steep main road at a narrow angle
/// (30°; 10 % main road, the side road falling at 15 %). Its mouth lies
/// far out (the corner arc), but its own surface starts where its centre
/// line leaves the main road's carriageway (3.75 m / sin 30° = 7.5 m), at
/// the main road's surface there; the mouth row lies at its own height; the
/// main road's carriageway is exactly the main road's surface (as J2 has
/// it); the corners slope between the main road's edge and the side road.
#[test]
fn a_narrow_side_road_leaves_at_the_main_road_edge_at_its_own_grade() {
    let g = fixtures::t_junction_at(0.10, -0.15, 30.0);
    let p = patch_at(&g, 0);
    check_outline(&p);
    check_mouths(&g, &p);
    let [a, b] = p.through.expect("a through road");
    let side = (0..3).find(|i| ![a, b].contains(i)).unwrap();
    let s = &p.arms[side];
    assert!(s.has_corner());
    assert!((s.edge_m - 7.5).abs() < 0.05, "{}", s.edge_m);
    assert!(s.mouth_m > s.edge_m + 2.0, "mouth {}", s.mouth_m);
    // The main surface at the edge point: 10 % along x.
    assert!((s.edge_h - 0.10 * s.edge_p[0]).abs() < 1e-6);
    // The mouth row: the side road's own height there (its middle; it
    // leans with the main road's slope across it, J5).
    let mid = 0.5 * (s.row[0][1] + s.row[1][1]);
    assert!((mid + 0.15 * s.mouth_m).abs() < 1e-9, "{:?}", s.row);
    assert!(s.cross.abs() > 0.01, "{}", s.cross);
    // The main road's carriageway: its own plane (10 % along x, flat
    // across), wherever the side road leaves.
    for x in [-6.0, -3.0, 0.0, 3.0, 6.0, 9.0] {
        for z in [-3.7, -1.0, 0.0, 1.0, 3.7] {
            let Some(h) = p.height_at([x, z]) else {
                continue;
            };
            // The mesh interpolates the plane exactly on its triangles
            // that lie on the carriageway; near the edge the triangles
            // reach into the corner.
            if p.main_excess([x, z]) <= 0.0 && z.abs() < 3.0 {
                assert!((h - 0.10 * x).abs() < 0.03, "({x}, {z}): {h}");
            }
            assert!((p.field([x, z]) - 0.10 * x).abs() < 1e-9 || p.main_excess([x, z]) > 0.0);
        }
    }
    // The side road's corner: from the main road's edge (its surface) down
    // to the mouth (the side road's own height), falling steadily, within
    // the two.
    let row_h = 0.5 * (s.row[0][1] + s.row[1][1]);
    let mut prev = f64::INFINITY;
    for k in 0..=40 {
        let m = s.edge_m + (s.mouth_m - s.edge_m) * f64::from(k) / 40.0;
        let q = mul(s.dir, m);
        let h = p.height_at(q).expect("in the patch");
        assert!(h <= prev + 1e-6, "{m}: {h} after {prev}");
        assert!(h <= s.edge_h + 0.02 && h >= row_h - 0.02, "{m}: {h}");
        prev = h;
    }
    // J2 put the mouth row on the main road's surface: 2.2 m above the
    // side road's own height there; now the side road misses only what
    // lies between the edge point and its own profile there.
    let j2_row = 0.10 * s.centre[0];
    assert!(j2_row - row_h > 2.0, "{j2_row} vs {row_h}");
    assert!((s.edge_h - (-0.15 * s.edge_m)).abs() < 1.9);
}

#[test]
fn a_t_junction_keeps_the_main_road_and_the_side_road_starts_at_its_edge() {
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
    // The side arm (§4.8.8 Q6 (d)): its own surface starts at the main
    // road's edge (3.75 m out at a right angle), its mouth row lies at its
    // own height and grade (no blend), and the corner between slopes from
    // the main road's edge to it – steadily, no dip, no hump.
    let side = (0..3).find(|i| ![a, b].contains(i)).unwrap();
    let s = &p.arms[side];
    assert!(s.has_corner());
    assert!((s.edge_m - 3.75).abs() < 0.01, "{}", s.edge_m);
    assert_eq!(s.blend_m, 0.0);
    assert!((s.edge_h - p.field(s.edge_p)).abs() < 1e-9);
    let mid = 0.5 * (s.row[0][1] + s.row[1][1]);
    assert!((mid - 0.20 * s.mouth_m).abs() < 1e-9, "{:?}", s.row);
    let mut prev = f64::NEG_INFINITY;
    for k in 0..=40 {
        let z = s.edge_m + (s.mouth_m - s.edge_m) * f64::from(k) / 40.0;
        let h = p.height_at([0.0, z]).unwrap();
        assert!(h >= prev - 1e-6, "{z}: {h} after {prev}");
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
    // J3: a 10° fork – its arms separate 29 m out – gets a patch too, its
    // mouths beyond 25 m (at most 40 m); a 3° fork keeps the overlapping
    // strips.
    let g = fork(10.0);
    let p = patch_at(&g, 0);
    check_outline(&p);
    check_mouths(&g, &p);
    assert!(
        p.arms.iter().any(|a| a.mouth_m > MAX_MOUTH_M),
        "{:?}",
        p.arms.iter().map(|a| a.mouth_m).collect::<Vec<_>>()
    );
    assert!(p.arms.iter().all(|a| a.mouth_m <= FORK_MAX_MOUTH_M));
    assert!(JunctionSet::new(&fork(3.0)).patch(&fork(3.0), 0).is_none());
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
    let side = p.arms.iter().position(Arm::has_corner).unwrap();
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

/// The junctions without a surface of a real package by reason and shape
/// (`OCW_J3_WORLD=pkg`, skipped without): how short their arms are, how
/// close the next junction vertex lies, the narrowest corner, one-way arms
/// (roundabout rings) – J3's work list.
#[test]
fn junctions_without_a_surface_of_a_real_package() {
    let Some(path) = std::env::var_os("OCW_J3_WORLD") else {
        eprintln!("OCW_J3_WORLD not set: skipped");
        return;
    };
    let pkg = crate::package::WorldPackage::from_bytes(&std::fs::read(path).unwrap()).unwrap();
    let g = pkg.graph.clone().unwrap();
    let js = JunctionSet::new(&g);
    let adj = g.adjacency();
    // One vertex in detail: its arms and outline before the checks.
    for v in std::env::var("OCW_J3_VERTEX")
        .unwrap_or_default()
        .split(',')
        .filter_map(|s| s.parse::<usize>().ok())
    {
        let c = g.vertices[v];
        println!("vertex {v} at {c:?}: {:?}", js.fallback_reason(&g, v));
        for &e in &adj[v] {
            let ed = &g.edges[e];
            println!(
                "  edge {e} {:?} w {:.1} len {:.1} from {} points {:?}",
                ed.class,
                ed.width_m,
                ed.length_m(),
                ed.from.0 as usize == v,
                &ed.points[..ed.points.len().min(4)]
            );
        }
        match patch_outline(&g, &adj[v], v) {
            Ok((p, o)) => {
                for a in &p.arms {
                    println!(
                        "  arm edge {} dir {:?} half {:.1} mouth {:.1}",
                        a.edge, a.dir, a.half, a.mouth_m
                    );
                }
                for (q, m) in &o {
                    let d = sub(*q, c);
                    println!(
                        "    {:8.2} {:8.2}  angle {:7.1}  r {:5.1}  {:?}",
                        d[0],
                        d[1],
                        d[1].atan2(d[0]).to_degrees(),
                        norm(d),
                        m
                    );
                }
            }
            Err(e) => println!("  {e}"),
        }
    }
    let junction_vertex = |v: usize| adj[v].len() >= 3;
    // Vertices of degree >= 3 by 10 m cell, for the neighbour search.
    let mut cells: std::collections::HashMap<(i64, i64), Vec<usize>> = Default::default();
    for v in 0..g.vertices.len() {
        if junction_vertex(v) {
            let p = g.vertices[v];
            cells
                .entry(((p[0] / 10.0).floor() as i64, (p[1] / 10.0).floor() as i64))
                .or_default()
                .push(v);
        }
    }
    let near = |v: usize| -> f64 {
        let p = g.vertices[v];
        let (cx, cz) = ((p[0] / 10.0).floor() as i64, (p[1] / 10.0).floor() as i64);
        let mut best = f64::INFINITY;
        for dx in -2..=2 {
            for dz in -2..=2 {
                for &w in cells.get(&(cx + dx, cz + dz)).into_iter().flatten() {
                    if w != v {
                        let q = g.vertices[w];
                        best = best.min((q[0] - p[0]).hypot(q[1] - p[1]));
                    }
                }
            }
        }
        best
    };
    let mut by: std::collections::BTreeMap<String, usize> = Default::default();
    let mut examples: std::collections::BTreeMap<String, Vec<usize>> = Default::default();
    let mut total = 0;
    #[allow(clippy::needless_range_loop)]
    for v in 0..g.vertices.len() {
        if adj[v].len() < 3 || js.rule(&g, v).is_some() {
            continue;
        }
        let Some(reason) = js.fallback_reason(&g, v) else {
            continue;
        };
        if matches!(
            reason,
            "the region border" | "a bridge or tunnel" | "a loop edge"
        ) {
            continue;
        }
        total += 1;
        let min_len = adj[v]
            .iter()
            .map(|&e| g.edges[e].length_m())
            .fold(f64::INFINITY, f64::min);
        let oneway = adj[v]
            .iter()
            .filter(|&&e| g.edges[e].oneway != crate::graph::Oneway::No)
            .count();
        let nb = near(v);
        let len_b = if min_len < 8.0 {
            "arm<8m"
        } else if min_len < 20.0 {
            "arm<20m"
        } else {
            "arms>=20m"
        };
        let nb_b = if nb < 10.0 {
            "jv<10m"
        } else if nb < 25.0 {
            "jv<25m"
        } else {
            "jv>=25m"
        };
        let key = format!("{reason} | {len_b} | {nb_b} | oneway {}", oneway.min(2));
        *by.entry(key.clone()).or_default() += 1;
        let ex = examples.entry(key).or_default();
        if ex.len() < 3 {
            ex.push(v);
        }
    }
    println!("junctions (degree >= 3, on the ground) without a surface: {total}");
    for (k, n) in &by {
        let ex: Vec<String> = examples[k]
            .iter()
            .map(|&v| format!("{v} ({:.0},{:.0})", g.vertices[v][0], g.vertices[v][1]))
            .collect();
        println!("{n:6}  {k}   e.g. {}", ex.join(", "));
    }
}

/// J3: two junction vertices 7 m apart (a side road meeting a dual
/// carriageway) whose patches alone would meet join into one cluster: one patch
/// for both, round their centroid, with the five other roads as arms; the
/// edge between them lies inside it (no strip, the patch's surface), its
/// ends' heights the patch's.
#[test]
fn junctions_a_few_metres_apart_share_one_patch() {
    let g = fixtures::dual_carriageway_t();
    let adj = g.adjacency();
    let direct = try_cluster(&g, &adj, &[0, 1], &[5]);
    assert!(direct.is_ok(), "{:?}", direct.err());
    let set = JunctionSet::new(&g);
    assert_eq!(set.cluster_of(&g, 0), Some((vec![0, 1], vec![5])));
    let p = set.patch(&g, 0).expect("a cluster patch").clone();
    assert_eq!(set.patch(&g, 1), Some(&p));
    assert!(p.is_at(0) && !p.is_at(1));
    assert!(set.own_patch(&g, 1).is_none());
    assert_eq!((p.members.clone(), p.inner.clone()), (vec![0, 1], vec![5]));
    assert_eq!(p.arms.len(), 5);
    assert!(p.centre[0].abs() < 1e-9 && (p.centre[2] - 3.5).abs() < 1e-9);
    check_outline(&p);
    check_mouths(&g, &p);
    // The edge across: inside, no strip, drawn on the patch – along its
    // centre line its own profile (level here).
    assert!(matches!(
        set.ends(&g, 5),
        [Some(EndRule::Inside(_)), Some(EndRule::Inside(_))]
    ));
    let strips = fixtures::strips(&g);
    let s = &strips[5].1;
    let sh = set.shape_strip(&g, 5, &s.points, s.width_m / 2.0).unwrap();
    assert!(sh.points.is_empty());
    for z in [0.5, 3.5, 6.5] {
        let h = set.drawn_height(&g, 5, z, [0.0, z]).unwrap();
        assert!(h.abs() < 0.02, "{z}: {h}");
    }
    assert_eq!(set.reach(&g, 5), [7.0, 7.0]);
    // The other strips end at the cluster's mouths, not across it.
    for (ei, s) in &strips {
        if *ei == 5 {
            continue;
        }
        let sh = set
            .shape_strip(&g, *ei, &s.points, s.width_m / 2.0)
            .unwrap();
        for q in &sh.points {
            let p2 = [f64::from(q[0]), f64::from(q[2])];
            let on_mouth = p.arms.iter().any(|a| a.mouth_dist(p2) < 1e-3);
            assert!(
                on_mouth || p.height_at(p2).is_none(),
                "edge {ei}: {q:?} inside the patch"
            );
        }
    }
}

/// J3: a small roundabout (outer diameter 20 m, its ring split into 11 m
/// arcs at the four entries, none of which gets a patch alone) is one
/// patch: the four entries its arms, the ring's arcs inside it (no strips;
/// on their centre lines their own profile), the outline once round its
/// centre outside the ring.
#[test]
fn a_small_roundabout_is_one_patch() {
    let g = fixtures::small_roundabout();
    let adj = g.adjacency();
    let alone: Vec<bool> = (0..4).map(|v| try_patch(&g, &adj[v], v).is_ok()).collect();
    let direct = try_cluster(&g, &adj, &[0, 1, 2, 3], &[4, 5, 6, 7]);
    assert!(direct.is_ok(), "alone {alone:?}: {:?}", direct.err());
    let set = JunctionSet::new(&g);
    let p = set.patch(&g, 0).expect("the roundabout's patch").clone();
    assert_eq!(p.members, vec![0, 1, 2, 3]);
    assert_eq!(p.inner, vec![4, 5, 6, 7]);
    assert_eq!(p.arms.len(), 4);
    for v in 1..4 {
        assert_eq!(set.patch(&g, v), Some(&p), "entry {v}");
    }
    check_outline(&p);
    check_mouths(&g, &p);
    // The whole ring lies inside the outline: its outer edge (radius 10 m)
    // every 5°, and the ring's centre line at its own height.
    for k in 0..72 {
        let a = f64::from(k * 5).to_radians();
        let q = [9.8 * a.cos(), 9.8 * a.sin()];
        assert!(p.height_at(q).is_some(), "{q:?} outside");
        let h = p.height_at([7.0 * a.cos(), 7.0 * a.sin()]).unwrap();
        assert!(h.abs() < 0.03, "{k}: {h}");
    }
    let strips = fixtures::strips(&g);
    for (ei, (_, s)) in strips.iter().enumerate().take(8).skip(4) {
        let sh = set.shape_strip(&g, ei, &s.points, s.width_m / 2.0).unwrap();
        assert!(sh.points.is_empty(), "ring arc {ei}");
    }
}

/// §4.8.8 Q7: the side roads' corners never stand above the main road's
/// surface on its carriageway – the patch's triangles are cut at the
/// carriageway's edge, so a steep side road's slope starts at the edge,
/// not inside the main road (before, the star mesh's triangles across the
/// edge lifted the main road by up to 0.4 m at steep side roads). The
/// corner off the carriageway still rises to the side road, and the
/// drawn surface is still the mesh (every mesh vertex at the field).
#[test]
fn side_roads_never_stand_above_the_main_road() {
    let mut measured = 0;
    for g in [
        t_junction(RoadClass::Secondary, 0.08, 0.20),
        fixtures::t_junction_built(RoadClass::Secondary, 0.0, 0.20),
        fixtures::t_junction_at(0.10, -0.15, 30.0),
        fixtures::dual_carriageway_t(),
    ] {
        let js = JunctionSet::new(&g);
        for v in 0..g.vertices.len() {
            let Some(p) = js.own_patch(&g, v) else {
                continue;
            };
            if p.through.is_none() {
                continue;
            }
            check_outline(p);
            let mut worst = f64::NEG_INFINITY;
            let mut x = p.min[0];
            while x <= p.max[0] {
                let mut z = p.min[1];
                while z <= p.max[1] {
                    let q = [x, z];
                    if p.main_excess(q) <= 0.0
                        && let (Some(h), Some(m)) = (p.height_at(q), p.main_own(q))
                    {
                        worst = worst.max(h - m);
                        measured += 1;
                    }
                    z += 0.1;
                }
                x += 0.1;
            }
            assert!(
                worst < 0.005,
                "vertex {v}: {worst:.3} m above the main road"
            );
            // Off the carriageway the corner rises to the side road.
            if let Some(s) = p.arms.iter().find(|a| a.has_corner() && a.edge_m > 0.01) {
                let out = [s.edge_p[0] + s.t[0] * 1.0, s.edge_p[1] + s.t[1] * 1.0];
                assert!(p.height_at(out).is_some());
            }
            for (i, q) in p.vertices.iter().enumerate() {
                if i < p.outline.len() {
                    continue;
                }
                assert!((q[1] - p.surface([q[0], q[2]])).abs() < 1e-9);
            }
        }
    }
    assert!(measured > 1000, "{measured}");
}

/// The worst bend of a patch's sides as the side lint measures it (over
/// pieces of 0.3 m, the strips' edges on at both ends; degrees).
fn worst_side_bend(p: &Patch) -> f64 {
    let mut worst = 0.0f64;
    for ch in &p.chains {
        let (a, b) = (&p.arms[ch.from_arm], &p.arms[ch.to_arm]);
        let n = ch.points.len();
        let mut pts = vec![add(xz(ch.points[0]), a.t)];
        pts.extend(ch.points.iter().map(|q| xz(*q)));
        pts.push(add(xz(ch.points[n - 1]), b.t));
        worst = worst.max(sides::worst_bend(&pts).to_degrees());
    }
    worst
}

/// Spec 4.8.8 Rule (J8): a side road whose OSM way is split 6 m from the
/// junction (a plain way split: same class and width, bending 10°) – its
/// arm used to end there, its mouth at 45 % of the 6 m edge, the corners
/// squeezed into a hook. Now the arm runs on past the split: the patch
/// covers the short edge (inside it, no strip), the arm is the edge
/// beyond with its mouth where the corners' radius needs it, the sides
/// bend no more than the lint allows, and the stored patch is the
/// computed one. J7: the arm on the short edge.
#[test]
fn an_arm_runs_on_past_a_way_split() {
    let g = fixtures::t_junction_split(0.03, 0.06, 6.0, 10.0);
    let js = JunctionSet::new(&g);
    let p = js.patch(&g, 0).expect("a patch").clone();
    assert_eq!(
        (p.members.clone(), p.inner.clone(), p.run_on.clone()),
        (vec![0, 4], vec![2], vec![4])
    );
    assert!(!p.is_cluster());
    assert!(p.arms.iter().all(|a| a.edge != 2));
    let side = p
        .arms
        .iter()
        .find(|a| a.edge == 3)
        .expect("the arm runs on");
    assert_eq!(side.origin, g.vertices[4]);
    // Past the split, where the 6 m corners end (3.75 + 6 m from the
    // vertex): about 3.75 m along the next edge.
    assert!(side.mouth_m > 3.0, "{}", side.mouth_m);
    assert!(matches!(
        js.ends(&g, 2),
        [Some(EndRule::Inside(_)), Some(EndRule::Inside(_))]
    ));
    assert!(matches!(js.ends(&g, 3)[0], Some(EndRule::Patch(q, _)) if q.vertex == 0));
    assert!(matches!(js.rule(&g, 4), Some(VertexRule::Patch(q)) if q.vertex == 0));
    assert!(js.own_patch(&g, 4).is_none());
    check_outline(&p);
    let worst = worst_side_bend(&p);
    assert!(worst < 25.0, "{worst:.1}°");
    // The surface along the edge inside: the patch's.
    let e = &g.edges[2];
    let mid = frame_at(&e.points, &cumulative(&e.points), 3.0).0;
    assert!(js.drawn_height(&g, 2, 3.0, mid).is_some());
    // Stored and read back: the same patch.
    let index = Arc::new(section::JunctionIndex::of_set(&g, &js, 0));
    let mut bytes = Vec::new();
    section::write_junctions(&index, &mut bytes);
    let read = Arc::new(section::read_junctions(&bytes).expect("read"));
    let stored = JunctionSet::with_index(&g, Some(read));
    assert_eq!(stored.patch(&g, 0), Some(&p));
    assert_eq!(stored.patch(&g, 4), Some(&p));
}

/// J8: an arm runs on only past a way split that squeezes it – one far
/// enough out keeps its mitre – and not past a split between two
/// junctions whose arms it squeezes both (it belongs to neither).
#[test]
fn a_way_split_far_out_or_between_two_junctions_stays() {
    let g = fixtures::t_junction_split(0.03, 0.06, 30.0, 10.0);
    let js = JunctionSet::new(&g);
    let p = js.patch(&g, 0).expect("a patch");
    assert!(p.run_on.is_empty() && p.arms.iter().any(|a| a.edge == 2));
    assert!(matches!(js.rule(&g, 4), Some(VertexRule::Mitre(_))));
    // Two T junctions 12 m apart along their side road, split halfway.
    let v = [
        [0.0, 0.0],
        [-100.0, 0.0],
        [100.0, 0.0],
        [0.0, 6.0],
        [0.0, 12.0],
        [-100.0, 12.0],
        [100.0, 12.0],
    ];
    use fixtures::Road;
    let g = fixtures::graph(
        &v,
        vec![
            Road::straight(&v, 1, 0, RoadClass::Secondary, 7.5, |_| 0.0),
            Road::straight(&v, 0, 2, RoadClass::Secondary, 7.5, |_| 0.0),
            Road::straight(&v, 0, 3, RoadClass::Residential, 5.0, |_| 0.0),
            Road::straight(&v, 3, 4, RoadClass::Residential, 5.0, |_| 0.0),
            Road::straight(&v, 5, 4, RoadClass::Secondary, 7.5, |_| 0.0),
            Road::straight(&v, 4, 6, RoadClass::Secondary, 7.5, |_| 0.0),
        ],
    );
    let js = JunctionSet::new(&g);
    assert!(js.runs_on(&g, 0, 2).is_none() && js.runs_on(&g, 4, 3).is_none());
    for v in [0, 4] {
        if let Some(p) = js.patch(&g, v) {
            assert!(p.run_on.is_empty());
        }
    }
}

/// J8: the mouths lie out far enough for each corner's radius along the
/// roads' real edge lines. The corner arcs place them by the straight
/// lines from the vertex to a point 8 m out; a side road that bends 9 m
/// from the vertex meets the main road's edge line elsewhere, and its
/// corner was cut short at the mouth (a hook). Now no corner is cut where
/// the arms' caps leave room, and the sides bend no more than the lint
/// allows. J7: 0.9 m short.
#[test]
fn mouths_lie_out_for_the_corners_along_the_real_edges() {
    for (bend, at) in [(20.0, 9.0), (-15.0, 9.0), (-20.0, 9.0)] {
        let g = fixtures::t_junction_bent(0.02, 0.05, bend, at);
        let p = try_patch(&g, &g.adjacency()[0], 0)
            .unwrap_or_else(|e| panic!("bend {bend} at {at}: {e}"));
        let n = p.arms.len();
        for i in 0..n {
            let (a, b) = (&p.arms[i], &p.arms[(i + 1) % n]);
            let radius = corner_radius(a.class.max(b.class));
            let d = sides::corner_deficit(&g, a, b, radius, None, [false; 2]);
            for (arm, dk) in [(a, d[0]), (b, d[1])] {
                let cap = MAX_MOUTH_M.min(MOUTH_SHARE * arm.len);
                assert!(
                    dk < 0.1 || arm.mouth_m > cap - 0.1,
                    "bend {bend} at {at}: corner {i} cut {dk:.2} m short"
                );
            }
        }
        check_outline(&p);
        let worst = worst_side_bend(&p);
        assert!(worst < 25.0, "bend {bend} at {at}: {worst:.1}°");
    }
}

/// Spec 4.8.8 Rule (J9): an offset T junction – two side roads join the
/// main road 12 m apart on opposite sides, leaning the same way (30° and
/// 240°) – whose patches alone meet on the main road's short edge between
/// them, so they are one cluster, but no star-shaped outline holds both
/// (J8: each kept its own patch, the arms on the short edge squeezed, the
/// second's sides bending 100°). Now the cluster takes a simple outline:
/// one patch for both, the short edge inside it, the sides bending no
/// more than the lint allows, stored = computed. Where the simple outline
/// would bend more (two side roads on the same side leaning together),
/// each keeps its own patch.
#[test]
fn two_junctions_on_a_short_edge_share_one_patch() {
    let g = fixtures::offset_t(8.0, 60.0, 90.0, 0.02);
    let js = JunctionSet::new(&g);
    assert!(!js.patch(&g, 0).is_some_and(Patch::is_cluster));
    assert!(js.cluster_failure(&g, 0).is_some());
    let g = fixtures::offset_t(12.0, 30.0, 240.0, 0.02);
    let js = JunctionSet::new(&g);
    let p = js.patch(&g, 0).expect("a patch").clone();
    assert_eq!((p.members.clone(), p.inner.clone()), (vec![0, 1], vec![1]));
    assert!(p.is_cluster() && !p.is_star());
    assert!(p.arms.iter().all(|a| a.edge != 1));
    assert_eq!(js.patch(&g, 1), Some(&p));
    assert!(js.cluster_failure(&g, 0).is_none());
    check_outline(&p);
    let worst = worst_side_bend(&p);
    assert!(worst < 25.0, "{worst:.1}°");
    // The surface along the edge inside: the patch's.
    let mid = [6.0, 0.0];
    assert!(js.drawn_height(&g, 1, 6.0, mid).is_some());
    let index = Arc::new(section::JunctionIndex::of_set(&g, &js, 0));
    let mut bytes = Vec::new();
    section::write_junctions(&index, &mut bytes);
    let read = Arc::new(section::read_junctions(&bytes).expect("read"));
    let stored = JunctionSet::with_index(&g, Some(read));
    assert_eq!(stored.patch(&g, 0), Some(&p));
    assert_eq!(stored.patch(&g, 1), Some(&p));
}

/// Spec 4.8.8 Rule (J9): a side road only 10 m long up to a bridge – its
/// arm's mouth was held at its share of the edge (4.5 m, J8), the 6 m
/// corners cut short; the bridge end takes nothing of the edge, so the
/// mouth may lie out to 1 m before it, where the corners need it: the
/// sides bend less than with the mouth at its share, stored = computed.
/// On a side road steeper than 6 % the mouth stays at its share.
#[test]
fn an_arm_up_to_a_bridge_takes_the_room_it_leaves() {
    let g = fixtures::t_junction_to_bridge(0.02, 0.04, 10.0);
    let js = JunctionSet::new(&g);
    let p = js.patch(&g, 0).expect("a patch").clone();
    let side = p.arms.iter().find(|a| a.edge == 2).expect("the side arm");
    assert!(side.mouth_m > 5.0 && side.mouth_m <= 9.0 + 1e-9, "{}", side.mouth_m);
    let share = try_patch(&g, js.edges_at(0), 0).expect("its share");
    assert!(
        (share.arms.iter().find(|a| a.edge == 2).expect("arm").mouth_m - 4.5).abs() < 1e-6
    );
    assert!(super::side_bend(&p) < super::side_bend(&share) - 1e-3);
    check_outline(&p);
    let index = Arc::new(section::JunctionIndex::of_set(&g, &js, 0));
    let mut bytes = Vec::new();
    section::write_junctions(&index, &mut bytes);
    let read = Arc::new(section::read_junctions(&bytes).expect("read"));
    let stored = JunctionSet::with_index(&g, Some(read));
    assert_eq!(stored.patch(&g, 0), Some(&p));
    // Steeper than 6 %: at its share.
    let g = fixtures::t_junction_to_bridge(0.02, 0.10, 10.0);
    let js = JunctionSet::new(&g);
    let p = js.patch(&g, 0).expect("a patch");
    let side = p.arms.iter().find(|a| a.edge == 2).expect("the side arm");
    assert!((side.mouth_m - 4.5).abs() < 1e-6, "{}", side.mouth_m);
}
