use std::collections::BTreeMap;

use ocw_world::graph::RoadClass;
use ocw_world::junction::fixtures::{self, Road, crossing, fork, t_junction};
use ocw_world::package::{Tile, TileKey};

use super::*;

/// The drawn surface along every edge's centre line at its profile
/// samples near the junctions (edge, sample, height).
fn drawn(g: &RoadGraph) -> Vec<(usize, usize, f64)> {
    drawn_samples(g, &JunctionSet::new(g), &|_, _| true)
}

/// A package of one 1 km tile with the fixture's strips (no terrain
/// beyond a flat grid 2 m under the roads).
fn package(g: &RoadGraph) -> WorldPackage {
    let n = 101u32;
    let low = g
        .edges
        .iter()
        .flat_map(|e| e.profile.iter().copied())
        .fold(f64::INFINITY, f64::min);
    let mut tiles = BTreeMap::new();
    for (x, z) in [(-1, -1), (0, -1), (-1, 0), (0, 0)] {
        let key = TileKey { x, z };
        let roads = fixtures::strips(g)
            .into_iter()
            .filter(|(_, s)| {
                let p = s.points[s.points.len() / 2];
                TileKey::containing([f64::from(p[0]), f64::from(p[2])], 1000.0) == key
            })
            .map(|(_, s)| s)
            .collect();
        tiles.insert(
            key,
            Tile {
                key,
                heights: vec![(low - 2.0) as f32; (n * n) as usize],
                roads,
            },
        );
    }
    WorldPackage {
        name: "Fixture".into(),
        origin: (47.0, 9.0),
        tile_size_m: 1000.0,
        samples: n,
        tiles,
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
    }
}

/// After the reconciliation the profiles near a junction are its drawn
/// surface (to the fixed point's residual), the drawn surface itself
/// stays where it was (the main road's), the side road takes the main
/// road's height at the vertex, and the roads far away are unchanged.
#[test]
fn reconciled_profiles_are_the_drawn_surface() {
    for g0 in [
        t_junction(RoadClass::Secondary, 0.08, 0.20),
        t_junction(RoadClass::Residential, -0.05, 0.15),
        crossing([0.05, -0.04, 0.10, 0.0]),
    ] {
        let before: BTreeMap<(usize, usize), f64> = drawn(&g0)
            .into_iter()
            .map(|(e, i, h)| ((e, i), h))
            .collect();
        let mut g = g0.clone();
        let st = reconcile_profiles(&mut g, &|_, _| true);
        assert_eq!(st.patches, 1);
        // The T junctions' side roads change (the crossing's arms keep their
        // heights at the mouths: only what lies inside is drawn).
        if g0.edges.len() == 3 {
            assert!(st.samples > 0 && st.max_change_m > 0.0, "{st:?}");
        }
        assert!(st.residual_m < 0.005, "{st:?}");
        assert!(st.last_round_m < 0.01, "{st:?}");
        for (e, i, h) in drawn(&g) {
            assert!(
                (g.edges[e].profile[i] - h).abs() < 0.005,
                "edge {e} sample {i}"
            );
            // The drawn surface moved little (the side road's blend now
            // starts at the main road's height and is shorter); the
            // crossing's lies on its plane now (Q7 (c)).
            if let Some(b) = before.get(&(e, i))
                && g0.edges.len() == 3
            {
                let a = g0.edges[e].profile[i];
                assert!(
                    (h - b).abs() <= (a - b).abs().max(0.03),
                    "edge {e} sample {i}: drawn {b:.3} -> {h:.3} (profile {a:.3})"
                );
            }
        }
        // One height at the vertex.
        let ends: Vec<f64> = g
            .edges
            .iter()
            .filter_map(|e| {
                if e.from.0 == 0 {
                    e.profile.first().copied()
                } else if e.to.0 == 0 {
                    e.profile.last().copied()
                } else {
                    None
                }
            })
            .collect();
        let (lo, hi) = (
            ends.iter().copied().fold(f64::INFINITY, f64::min),
            ends.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        );
        assert!(hi - lo < 1e-9, "{ends:?}");
        // Far from the junction nothing changed. A side road climbing
        // from the junction (J2b) regains the height its level start did
        // not climb over a longer stretch (at the class grade, its grade
        // changing by at most 1 % per metre: 65 m for 15 %); one already
        // at its class limit (20 % residential) spreads it over the whole
        // road (the least grade possible).
        for (k, (a, b)) in g.edges.iter().zip(&g0.edges).enumerate() {
            let at_limit = (b.profile[b.profile.len() - 1] - b.profile[0]).abs() / b.length_m()
                >= crate::profile::limits(b.class).0 - 1e-9;
            if at_limit {
                assert!(k == 2 && g0.edges.len() == 3, "edge {k}");
                continue;
            }
            let len = a.length_m();
            for i in 0..a.profile.len() {
                let s = sample_m(i, len);
                let m = if a.from.0 == 0 { s } else { len - s };
                let far = if k == 2 && g0.edges.len() == 3 {
                    100.0
                } else {
                    60.0
                };
                if m > far {
                    assert_eq!(a.profile[i], b.profile[i], "edge {k} at {m:.0} m");
                }
            }
        }
    }
}

/// The T junction of the plan: the main road at 8 % keeps its own
/// surface, the side road at 20 % starts on it (no step at the main
/// road's edge) and its profile now follows the blend: the package's
/// heights are the drawn heights, so the renderer's re-heighting changes
/// nothing (within a millimetre at the strip points).
#[test]
fn the_side_road_starts_on_the_main_road() {
    let mut g = t_junction(RoadClass::Secondary, 0.08, 0.20);
    let g0 = g.clone();
    reconcile_profiles(&mut g, &|_, _| true);
    // The main road is a plane along its grade: unchanged.
    for e in [0, 1] {
        for (a, b) in g.edges[e].profile.iter().zip(&g0.edges[e].profile) {
            assert!((a - b).abs() < 0.005);
        }
    }
    // The side road starts on the main road (flat across along +z: the
    // main road's height at x = 0 is 0) and leaves its edge (3.75 m out)
    // at its own grade (§4.8.8 Q6 (d)): 20 % × 1.25 m at its 5 m sample.
    let side = &g.edges[2];
    assert!(side.profile[0].abs() < 1e-6);
    assert!(
        (side.profile[1] - 0.25).abs() < 0.03,
        "{:?}",
        &side.profile[..4]
    );
    // Its strips, shaped for the junction, keep the package's heights.
    let js = JunctionSet::new(&g);
    for (ei, s) in fixtures::strips(&g) {
        let Some(sh) = js.shape_strip(&g, ei, &s.points, s.width_m / 2.0) else {
            continue;
        };
        for p in sh.points.iter().skip(1) {
            let orig = s
                .points
                .iter()
                .find(|q| (q[0] - p[0]).abs() < 1e-3 && (q[2] - p[2]).abs() < 1e-3);
            if let Some(q) = orig {
                assert!((q[1] - p[1]).abs() < 0.005, "edge {ei}: {q:?} vs {p:?}");
            }
        }
    }
}

/// The ease of 4.8.3 is skipped where a patch's blend replaces it.
#[test]
fn patched_vertices_skip_the_mouth_easing() {
    let g = t_junction(RoadClass::Secondary, 0.0, 0.12);
    let patched = patched_vertices(&g);
    assert_eq!(patched, vec![true, false, false, false]);
    let mut eased = g.clone();
    assert_eq!(crate::clean::ease_junction_mouths(&mut eased), 1);
    let mut skipped = g.clone();
    assert_eq!(
        crate::clean::ease_junction_mouths_except(&mut skipped, &patched),
        0
    );
    assert_eq!(skipped, g);
    // A fork the patch cannot hold keeps no patch.
    assert!(!patched_vertices(&fork(3.0))[0]);
}

/// A through road changing its grade at a junction (Valünastrasse: −3 %
/// on one side, +25 % on the other, here 3 % and 25 %): rounded into a
/// vertical curve across the vertex within the class limit, never
/// steeper, the side road taking the new vertex height.
#[test]
fn through_roads_have_no_kink_at_a_junction() {
    let v = [[0.0, 0.0], [-150.0, 0.0], [150.0, 0.0], [0.0, 120.0]];
    let mut g = fixtures::graph(
        &v,
        vec![
            Road::straight(&v, 1, 0, RoadClass::Track, 3.0, |s| (s - 150.0) * 0.03),
            Road::straight(&v, 0, 2, RoadClass::Track, 3.0, |s| s * 0.25),
            Road::straight(&v, 0, 3, RoadClass::Track, 3.0, |s| s * 0.10),
        ],
    );
    let g0 = g.clone();
    assert!(smooth_through_kinks(&mut g) > 0);
    // The joined profile of the through road.
    let (a, b) = (&g.edges[0], &g.edges[1]);
    let mut d = Vec::new();
    let mut h = Vec::new();
    let la = a.length_m();
    for (i, &y) in a.profile.iter().enumerate() {
        d.push(sample_m(i, la) - la);
        h.push(y);
    }
    for (i, &y) in b.profile.iter().enumerate().skip(1) {
        d.push(sample_m(i, b.length_m()));
        h.push(y);
    }
    let (limit, change) = limits(RoadClass::Track);
    for i in 1..h.len() - 1 {
        let (d1, d2) = (d[i] - d[i - 1], d[i + 1] - d[i]);
        let (g1, g2) = ((h[i] - h[i - 1]) / d1, (h[i + 1] - h[i]) / d2);
        assert!(
            (g2 - g1).abs() <= change * (d1 + d2) / 2.0 + 1e-3,
            "kink at {}: {g1:.3} -> {g2:.3}",
            d[i]
        );
        assert!(g2.abs() <= limit + 1e-9);
    }
    // Only near the vertex.
    assert_eq!(a.profile[..20], g0.edges[0].profile[..20]);
    // One height at the vertex, the side road's too.
    let vh = a.profile[a.profile.len() - 1];
    assert_eq!(b.profile[0], vh);
    assert_eq!(g.edges[2].profile[0], vh);
    assert!(vh > 0.05, "lifted into the curve: {vh}");
    // A second pass moves nothing that shows.
    let mut again = g.clone();
    smooth_through_kinks(&mut again);
    for (a, b) in again.edges.iter().zip(&g.edges) {
        for (x, y) in a.profile.iter().zip(&b.profile) {
            assert!((x - y).abs() < 1e-3);
        }
    }
}

/// The settle samples are the drawn junction surface: inside the patch
/// its mesh, along the blend its rows; the settled terrain lies 5 cm
/// under them.
#[test]
fn the_terrain_settles_under_the_junction_surface() {
    let mut g = t_junction(RoadClass::Secondary, 0.08, 0.20);
    reconcile_profiles(&mut g, &|_, _| true);
    let js = JunctionSet::new(&g);
    let samples = surface_samples(&g, &js, 0.5);
    let p = js.patch(&g, 0).unwrap();
    assert!(samples.len() > 200);
    let in_patch = samples
        .iter()
        .filter(|q| p.height_at([q[0], q[2]]).is_some())
        .count();
    assert!(in_patch > 100);
    for q in &samples {
        if let Some(h) = p.height_at([q[0], q[2]]) {
            assert!((h - q[1]).abs() < 1e-6);
        }
    }
    // A terrain 1 m above the junction: settled under it.
    let mut pkg = package(&g);
    for t in pkg.tiles.values_mut() {
        t.heights.iter_mut().for_each(|h| *h = 1.0);
    }
    crate::clean::settle_terrain_under_surfaces(&mut pkg, 0.05, &samples);
    for q in samples.iter().step_by(7) {
        let t = pkg.surface_height(q[0], q[2]).unwrap();
        assert!(t <= q[1] - 0.05 + 2e-3, "terrain {t:.3} under {:.3}", q[1]);
    }
}

/// The post-process (`world-builder junctions`) moves the package's strip
/// points by the profiles' change: afterwards they lie at the new
/// profile heights.
#[test]
fn strips_follow_the_reconciled_profiles() {
    let g0 = t_junction(RoadClass::Secondary, 0.08, 0.20);
    let mut pkg = package(&g0);
    let mut g = g0.clone();
    reconcile_profiles(&mut g, &|_, _| true);
    let edges = strip_edges_of(&pkg, &g0);
    assert!(edges.tiles.values().flatten().all(|&e| e != NO_EDGE));
    assert!(reheight_strips(&mut pkg, &g0, &g, &edges) > 0);
    let fresh = package(&g);
    assert_eq!(pkg.tiles.len(), fresh.tiles.len());
    for (a, b) in pkg.tiles.values().zip(fresh.tiles.values()) {
        for (sa, sb) in a.roads.iter().zip(&b.roads) {
            for (p, q) in sa.points.iter().zip(&sb.points) {
                assert!((p[1] - q[1]).abs() < 1e-4, "{p:?} vs {q:?}");
            }
        }
    }
}

/// The lint measures the drawn carriageways: none of the synthetic
/// junctions has a seam or an overlap (J1's shaping), before and after
/// the reconciliation; a strip lifted by 30 cm shows one; a fork too
/// narrow for a patch is listed without a surface, with its reason.
#[test]
fn the_seam_lint_measures_the_drawn_surfaces() {
    for g0 in [
        t_junction(RoadClass::Secondary, 0.08, 0.20),
        crossing([0.05, -0.04, 0.10, 0.0]),
    ] {
        for reconciled in [false, true] {
            let mut g = g0.clone();
            if reconciled {
                reconcile_profiles(&mut g, &|_, _| true);
            }
            let pkg = package(&g);
            let (s, without) = seams(&pkg, &g);
            let at0 = s.iter().find(|s| s.vertex == 0).expect("the junction");
            assert!(at0.seam_m <= SEAM_M, "{at0:?}");
            assert!(at0.zfight_m2 <= ZFIGHT_M2, "{at0:?}");
            assert!(without.is_empty(), "{without:?}");
        }
    }
    // Two carriageways meeting with a 30 cm step, and two lying on each
    // other (z-fighting).
    let quad = |x0: f64, x1: f64, h: f64| {
        [
            [[x0, h, -5.0], [x1, h, -5.0], [x0, h, 5.0]],
            [[x1, h, -5.0], [x1, h, 5.0], [x0, h, 5.0]],
        ]
    };
    let mut tris = Vec::new();
    for t in quad(-6.0, 0.0, 0.0) {
        tris.push((t, 1));
    }
    for t in quad(0.0, 6.0, 0.3) {
        tris.push((t, 2));
    }
    let m = measure([0.0, 0.0], 4.0, &tris, 7);
    assert!((m.seam_m - 0.3).abs() < 1e-6, "{m:?}");
    assert_eq!(m.zfight_m2, 0.0);
    let mut tris = Vec::new();
    for t in quad(-6.0, 1.0, 0.0) {
        tris.push((t, 1));
    }
    for t in quad(-1.0, 6.0, 0.004) {
        tris.push((t, 2));
    }
    let m = measure([0.0, 0.0], 4.0, &tris, 7);
    assert!(m.seam_m < 0.01, "{m:?}");
    // 2 m × 8 m of the disc (radius 4 m) under both.
    assert!(m.zfight_m2 > 12.0 && m.zfight_m2 < 18.0, "{m:?}");
    let f = fork(3.0);
    let (_, without) = seams(&package(&f), &f);
    assert_eq!(without.len(), 1);
    assert!(without[0].1.contains("fork"), "{without:?}");
}

/// The sections for the final graph: the index gives the computed rules,
/// flagged as settled; every strip has its edge.
#[test]
fn the_junction_data_describe_the_final_graph() {
    let mut g = t_junction(RoadClass::Secondary, 0.08, 0.20);
    reconcile_profiles(&mut g, &|_, _| true);
    let mut pkg = package(&g);
    let data = junction_data(&pkg, true).unwrap();
    let index = data.index.clone().unwrap();
    assert!(index.settled() && index.fits(&g));
    assert_eq!(index.patches(), 1);
    pkg.junctions = Some(Arc::new(data));
    let back = WorldPackage::from_bytes(&pkg.to_bytes()).unwrap();
    assert_eq!(back.junctions, pkg.junctions);
    let set = JunctionSet::with_index(&g, back.junctions.as_ref().unwrap().index_for(&g));
    assert!(set.is_stored());
    let computed = JunctionSet::new(&g);
    for v in 0..g.vertices.len() {
        assert_eq!(set.rule(&g, v), computed.rule(&g, v));
    }
}

/// One junction of a real package in detail (`OCW_J2_WORLD=pkg
/// OCW_J2_VERTEX=v`, skipped without): its arms, the strips of its edges
/// as shaped, and its seam.
#[test]
fn junction_detail_of_a_real_package() {
    let (Some(path), Some(v)) = (
        std::env::var_os("OCW_J2_WORLD"),
        std::env::var("OCW_J2_VERTEX").ok(),
    ) else {
        eprintln!("OCW_J2_WORLD / OCW_J2_VERTEX not set: skipped");
        return;
    };
    let v: usize = v.parse().unwrap();
    let pkg = WorldPackage::from_bytes(&std::fs::read(path).unwrap()).unwrap();
    let g = pkg.graph.clone().unwrap();
    let js = JunctionSet::new(&g);
    let p = js.patch(&g, v).expect("a patch");
    println!("vertex {v} at {:?}, through {:?}", g.vertices[v], p.through);
    for (i, a) in p.arms.iter().enumerate() {
        let e = &g.edges[a.edge];
        println!(
            "arm {i}: edge {} w{} {:?} {:.1} m wide, len {:.1}, from {} mouth {:.2} blend {:.2} h {:.3} grade {:.3} row {:?}",
            a.edge,
            e.osm_way,
            e.class,
            e.width_m,
            a.len,
            a.at_from,
            a.mouth_m,
            a.blend_m,
            a.h,
            a.grade,
            a.row
        );
        println!("   profile {:?}", &e.profile[..e.profile.len().min(8)]);
    }
    let edges = strip_edges_of(&pkg, &g);
    let c = g.vertices[v];
    for (key, t) in &pkg.tiles {
        let Some(ix) = edges.tile(*key, t.roads.len()) else {
            continue;
        };
        for (s, &ei) in t.roads.iter().zip(ix) {
            if !p.arms.iter().any(|a| a.edge as u32 == ei) {
                continue;
            }
            let near = s
                .points
                .iter()
                .any(|q| (f64::from(q[0]) - c[0]).hypot(f64::from(q[2]) - c[1]) < 40.0);
            if !near {
                continue;
            }
            println!(
                "strip of edge {ei} in {key:?}: {} points, first {:?} last {:?}",
                s.points.len(),
                s.points[0],
                s.points[s.points.len() - 1]
            );
            if let Some(sh) = js.shape_strip(&g, ei as usize, &s.points, s.width_m / 2.0) {
                for (q, r) in sh.points.iter().zip(&sh.rows).take(6) {
                    println!("   point {q:?} row {r:?}");
                }
            }
        }
    }
    let (seams, _) = seams(&pkg, &g);
    println!("{:?}", seams.iter().find(|s| s.vertex == v));
}

/// A steep side road too short for the blend it needs (J1 blends over at
/// most half its edge): the reconciliation bakes the longer blend into its
/// profile, so the drawn surface no longer climbs from the main road at
/// once – the seam at the mouth shrinks.
#[test]
fn short_steep_side_roads_get_their_blend_in_the_profile() {
    let v = [[0.0, 0.0], [-120.0, 0.0], [120.0, 0.0], [0.0, 26.0]];
    let g0 = fixtures::graph(
        &v,
        vec![
            Road::straight(&v, 1, 0, RoadClass::Secondary, 7.5, |s| (s - 120.0) * 0.06),
            Road::straight(&v, 0, 2, RoadClass::Secondary, 7.5, |s| s * 0.06),
            Road::straight(&v, 0, 3, RoadClass::Residential, 5.0, |s| s * 0.20),
        ],
    );
    // J3 (Q6 (d)): the side road leaves the main road's edge at its own
    // grade; its blend (from the edge point) lies in its profile.
    let p = JunctionSet::new(&g0).patch(&g0, 0).cloned().unwrap();
    let side = p.arms.iter().find(|a| a.edge == 2).unwrap().clone();
    assert!(side.has_corner() && side.blend_m == 0.0, "{side:?}");
    let mut g = g0.clone();
    let st = reconcile_profiles(&mut g, &|_, _| true);
    assert!(st.longer_blend_samples >= 2, "{st:?}");
    let seam = |g: &RoadGraph| {
        let (s, _) = seams(&package(g), g);
        s.iter().find(|s| s.vertex == 0).unwrap().seam_m
    };
    assert!(seam(&g) <= SEAM_M, "seam {:.3}", seam(&g));
    // The far end keeps its height.
    assert_eq!(g.edges[2].profile.last(), g0.edges[2].profile.last());
}

/// The grade of every interval of a profile at distances `d`.
fn grades(p: &[f64], d: &[f64]) -> Vec<f64> {
    (0..p.len() - 1)
        .map(|i| (p[i + 1] - p[i]) / (d[i + 1] - d[i]))
        .collect()
}

/// J2b: a profile with a 2 m step just past a fixed (junction) stretch:
/// the fixed samples keep their heights, every interval comes within the
/// class grade, the step is spread over a longer stretch (pulled towards
/// the junction, climbing at the limit until it meets the road's own
/// profile), the road far away keeps its profile, and the kink at the
/// mouth is rounded by the free samples.
#[test]
fn relimiting_keeps_the_junction_and_lengthens_the_blend() {
    let d: Vec<f64> = (0..41).map(|i| f64::from(i) * 5.0).collect();
    let mut p: Vec<f64> = d
        .iter()
        .map(|&x| {
            if x <= 5.0 {
                0.0
            } else {
                2.0 + 0.12 * (x - 5.0)
            }
        })
        .collect();
    let fixed: Vec<bool> = (0..41).map(|i| i <= 1 || i == 40).collect();
    let before = p.clone();
    assert!(grades(&p, &d)[1] > 0.4);
    let moved = relimit_profile(&mut p, &d, &fixed, 0.20, 0.01);
    assert!(moved >= 3, "{moved}");
    for i in [0, 1, 40] {
        assert_eq!(p[i], before[i], "fixed sample {i}");
    }
    let g = grades(&p, &d);
    assert!(g.iter().all(|g| g.abs() <= 0.20 + 1e-6), "{g:?}");
    // The far half of the road is its own profile.
    for i in 25..41 {
        assert!(
            (p[i] - before[i]).abs() < 1e-6,
            "sample {i}: {} vs {}",
            p[i],
            before[i]
        );
    }
    // The step's height is reached further on (the blend lengthened).
    let meets = (2..41).find(|&i| (p[i] - before[i]).abs() < 1e-3).unwrap();
    assert!(meets >= 4, "meets the own profile at sample {meets}");
    // The kink at the mouth (the fixed stretch is level) is rounded: the
    // gradient changes by at most 8 points between intervals.
    for w in g.windows(2) {
        assert!((w[1] - w[0]).abs() <= 0.08 + 1e-6, "{g:?}");
    }
}

/// J2b: fixed heights farther apart than the class grade allows (the
/// source data steeper): the stretch between them climbs at the least
/// grade possible, and the steep intervals are `source_steep`.
#[test]
fn relimiting_spreads_what_the_source_makes_too_steep() {
    let d: Vec<f64> = (0..9).map(|i| f64::from(i) * 5.0).collect();
    // 40 m between fixed ends 12 m apart (30 % for a 20 % road), with a
    // wall in the middle.
    let mut p = vec![0.0, 0.0, 0.0, 0.0, 9.0, 12.0, 12.0, 12.0, 12.0];
    let fixed: Vec<bool> = (0..9).map(|i| i == 0 || i == 8).collect();
    relimit_profile(&mut p, &d, &fixed, 0.20, 0.01);
    let g = grades(&p, &d);
    let worst = g.iter().fold(0.0f64, |a, b| a.max(b.abs()));
    assert!(worst <= 0.30 + 1e-6, "{g:?}");
    assert_eq!((p[0], p[8]), (0.0, 12.0));
    // In a graph: a 40 m residential road between two dead ends 12 m
    // apart.
    let v = [[0.0, 0.0], [40.0, 0.0]];
    let g = fixtures::graph(
        &v,
        vec![Road::straight(&v, 0, 1, RoadClass::Residential, 5.0, |s| {
            if s < 18.0 { 0.0 } else { 12.0 }
        })],
    );
    let js = JunctionSet::new(&g);
    let steep = steep_intervals(&g, &js, &|_, _| true);
    assert!(!steep.is_empty());
    assert!(
        steep.iter().all(|s| s.cause == SteepCause::SourceSteep),
        "{steep:?}"
    );
}

/// J2b on a T junction whose side road climbs at 18 % (residential,
/// limit 20 %): the J2 blend alone leaves it steeper than the limit +
/// 1 % beyond the main road's edge; re-limiting with the junction
/// surface fixed brings every interval within the limit, the junction
/// stays level with the main road (the side road's vertex sample and the
/// samples inside the patch on the main road's surface), the profiles
/// are still the drawn surface, and the lint finds no steep interval.
#[test]
fn side_roads_keep_their_class_grade_after_the_blend() {
    let g0 = t_junction(RoadClass::Secondary, 0.06, 0.18);
    // The blend alone (J2): steeper than the limit near the junction.
    let mut blended = g0.clone();
    reconcile_rounds(
        &mut blended,
        &|_, _| true,
        true,
        &mut ReconcileStats::default(),
    );
    let r0 = crate::lint::lint(&package(&blended), None);
    assert!(r0.count("road.vertical.grade_over_limit.junction") > 0);
    let mut g = g0.clone();
    let st = reconcile_profiles(&mut g, &|_, _| true);
    assert!(st.steep_before > 0, "{st:?}");
    assert!(st.relimit.samples > 0, "{st:?}");
    assert_eq!(
        (
            st.steep_intervals,
            st.steep_other,
            st.steep_source,
            st.steep_room
        ),
        (0, 0, 0, 0),
        "{st:?}"
    );
    let side = &g.edges[2];
    let len = side.length_m();
    let d: Vec<f64> = (0..side.profile.len()).map(|i| sample_m(i, len)).collect();
    let gr = grades(&side.profile, &d);
    assert!(gr.iter().all(|x| x.abs() <= 0.20 + 0.01), "{gr:?}");
    // The junction stays on the main road: the vertex at its height; the
    // side road leaves the main road's edge (3.75 m out, §4.8.8 Q6 (d));
    // at its class limit right after the junction the crease has no room
    // to be rounded (Q7 (b) would make it steeper): its own grade from the
    // edge, 18 % × 1.25 m = 0.225 m at its 5 m sample.
    assert_eq!(side.profile[0], g.edges[1].profile[0]);
    assert!(
        side.profile[1] - side.profile[0] < 0.25,
        "{:?}",
        &side.profile[..4]
    );
    // The main road is a plane along its grade: unchanged.
    for e in [0, 1] {
        for (a, b) in g.edges[e].profile.iter().zip(&g0.edges[e].profile) {
            assert!((a - b).abs() < 0.005);
        }
    }
    // Still the drawn surface (the fixed point of the reconciliation).
    assert!(st.residual_m < 0.02, "{st:?}");
    // The far end keeps its height.
    assert_eq!(side.profile.last(), g0.edges[2].profile.last());
    // The lint agrees.
    let r = crate::lint::lint(&package(&g), None);
    assert_eq!(r.count("road.vertical.grade_over_limit"), 0);
}

/// J2b: a 25 m residential road at 20 % between two level main roads –
/// its source heights hold the limit, but with both junction surfaces
/// level with their main roads the rest has too little room:
/// `junction_room`, not a fault of the re-limiting (J3's clusters).
#[test]
fn short_roads_between_junctions_have_too_little_room() {
    let v = [
        [0.0, 0.0],
        [-100.0, 0.0],
        [100.0, 0.0],
        [0.0, 25.0],
        [-100.0, 25.0],
        [100.0, 25.0],
    ];
    let mut g = fixtures::graph(
        &v,
        vec![
            Road::straight(&v, 1, 0, RoadClass::Secondary, 7.5, |_| 0.0),
            Road::straight(&v, 0, 2, RoadClass::Secondary, 7.5, |_| 0.0),
            Road::straight(&v, 4, 3, RoadClass::Secondary, 7.5, |_| 5.0),
            Road::straight(&v, 3, 5, RoadClass::Secondary, 7.5, |_| 5.0),
            Road::straight(&v, 0, 3, RoadClass::Residential, 5.0, |s| s * 0.20),
        ],
    );
    let st = reconcile_profiles(&mut g, &|_, _| true);
    assert_eq!(st.patches, 2, "{st:?}");
    assert_eq!(
        (st.steep_source, st.steep_intervals, st.steep_other),
        (0, 0, 0),
        "{st:?}"
    );
    assert!(st.steep_room > 0, "{st:?}");
}

/// §4.8.8 Q6 (d): the J2b case of `junction_room` – a 33 m track leaving a
/// main road climbing at 12 % at a narrow angle (30°) and falling at 10 %
/// to a level road. J2 put its mouths on the main roads' surfaces 12 – 15 m
/// out, 2.5 m above its own height on the steep side: the 3 – 9 m between
/// them could not hold 25 %. Now the track leaves each main road at its
/// edge (7.5 m from the vertex) at its own grade and the corners slope:
/// within its limit, the main roads unchanged, the drawn surface, the lint
/// clean of grades over the limit.
#[test]
fn narrow_side_roads_leave_steep_roads_at_their_edge() {
    let a = 30f64.to_radians();
    let end = [33.0 * a.cos(), 33.0 * a.sin()];
    let v = [
        [0.0, 0.0],
        [-150.0, 0.0],
        [150.0, 0.0],
        end,
        [-150.0, end[1]],
        [150.0, end[1]],
    ];
    let g0 = fixtures::graph(
        &v,
        vec![
            Road::straight(&v, 1, 0, RoadClass::Secondary, 7.5, |s| (s - 150.0) * 0.12),
            Road::straight(&v, 0, 2, RoadClass::Secondary, 7.5, |s| s * 0.12),
            Road::straight(&v, 4, 3, RoadClass::Secondary, 7.5, |_| -3.3),
            Road::straight(&v, 3, 5, RoadClass::Secondary, 7.5, |_| -3.3),
            Road::straight(&v, 0, 3, RoadClass::Track, 3.0, |s| -0.10 * s),
        ],
    );
    let js = JunctionSet::new(&g0);
    for vx in [0, 3] {
        let p = js.patch(&g0, vx).expect("a patch");
        let side = p.arms.iter().find(|x| x.edge == 4).unwrap();
        assert!(side.has_corner(), "vertex {vx}");
        assert!((side.edge_m - 7.5).abs() < 0.1, "{}", side.edge_m);
        assert!(side.mouth_m > side.edge_m + 2.0, "{}", side.mouth_m);
    }
    drop(js);
    let mut g = g0.clone();
    let st = reconcile_profiles(&mut g, &|_, _| true);
    assert_eq!(st.patches, 2, "{st:?}");
    assert_eq!(
        (
            st.steep_room,
            st.steep_intervals,
            st.steep_other,
            st.steep_source
        ),
        (0, 0, 0, 0),
        "{st:?}"
    );
    let t = &g.edges[4];
    let d: Vec<f64> = (0..t.profile.len())
        .map(|i| sample_m(i, t.length_m()))
        .collect();
    let gr = grades(&t.profile, &d);
    assert!(gr.iter().all(|x| x.abs() <= 0.25 + 0.01), "{gr:?}");
    for e in 0..4 {
        for (x, y) in g.edges[e].profile.iter().zip(&g0.edges[e].profile) {
            assert!((x - y).abs() < 0.005, "main road {e}");
        }
    }
    assert!(st.residual_m < 0.02, "{st:?}");
    let r = crate::lint::lint(&package(&g), None);
    assert_eq!(r.count("road.vertical.grade_over_limit"), 0);
}

/// The kinks (> 8 points between two 5 m intervals) of a real package by
/// where they lie at their junction (`OCW_J3_WORLD=pkg`, skipped without):
/// the kind of arm (main, side with its corner, side blended as in J2, arm
/// of a blended patch, mitre) and the distance from its vertex.
#[test]
fn kinks_of_a_real_package() {
    let Some(path) = std::env::var_os("OCW_J3_WORLD") else {
        eprintln!("OCW_J3_WORLD not set: skipped");
        return;
    };
    let pkg = WorldPackage::from_bytes(&std::fs::read(path).unwrap()).unwrap();
    let g = pkg.graph.clone().unwrap();
    let js = match pkg.junctions.as_ref() {
        Some(j) => JunctionSet::with_index(&g, j.index_for(&g)),
        None => JunctionSet::new(&g),
    };
    let mut by: BTreeMap<String, usize> = BTreeMap::new();
    let mut ex: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (ei, e) in g.edges.iter().enumerate() {
        let p = &e.profile;
        if p.len() < 3 || e.bridge || e.tunnel {
            continue;
        }
        let len = e.length_m();
        let d: Vec<f64> = (0..p.len()).map(|i| sample_m(i, len)).collect();
        let ends = js.ends(&g, ei);
        for i in 1..p.len() - 1 {
            let (d1, d2) = (d[i] - d[i - 1], d[i + 1] - d[i]);
            if d1 < 1.0 || d2 < 1.0 {
                continue;
            }
            let (g1, g2) = ((p[i] - p[i - 1]) / d1, (p[i + 1] - p[i]) / d2);
            if (g2 - g1).abs() <= 0.08 {
                continue;
            }
            // The nearer end.
            let k = usize::from(len - d[i] < d[i]);
            let m = if k == 0 { d[i] } else { len - d[i] };
            let kind = match ends[k] {
                Some(EndRule::Patch(pt, ai)) => {
                    let a = &pt.arms[ai];
                    let side = pt.through.is_some_and(|t| !t.contains(&ai));
                    let what = match (pt.through.is_some(), side, a.has_corner()) {
                        (false, _, _) => "blended patch",
                        (true, false, _) => "main arm",
                        (true, true, true) => "side, corner",
                        (true, true, false) => "side, J2 blend",
                    };
                    let rel = if a.has_corner() && m <= a.edge_m + PROFILE_STEP_M {
                        "at the edge point"
                    } else if m <= a.mouth_m {
                        "inside"
                    } else if m <= a.mouth_m + 30.0 {
                        "< 30 m past the mouth"
                    } else {
                        "farther"
                    };
                    format!("{what}: {rel}")
                }
                Some(EndRule::Mitre(..)) if m <= 30.0 => "mitre < 30 m".to_string(),
                _ if m <= 30.0 => "no surface, < 30 m".to_string(),
                _ => "elsewhere".to_string(),
            };
            *by.entry(kind.clone()).or_default() += 1;
            let x = ex.entry(kind).or_default();
            if x.len() < 4 {
                x.push(format!(
                    "e{ei} {:?} {:.0} m from v{} {:+.1} to {:+.1} %",
                    e.class,
                    m,
                    if k == 0 { e.from.0 } else { e.to.0 },
                    g1 * 100.0,
                    g2 * 100.0
                ));
            }
        }
    }
    for (k, n) in &by {
        println!("{n:6}  {k}   e.g. {}", ex[k].join("; "));
    }
}

/// J3: a cluster (a side road meeting a dual carriageway 5 m apart) is one
/// junction surface in the world builder: counted once, the edge inside it
/// reconciled with its surface all along, the lint measuring it once with
/// no junction left without a surface and no seam.
#[test]
fn clusters_are_one_junction_surface() {
    let g0 = fixtures::dual_carriageway_t();
    let mut g = g0.clone();
    let st = reconcile_profiles(&mut g, &|_, _| true);
    assert_eq!(st.patches, 1, "{st:?}");
    assert!(st.residual_m < 0.02, "{st:?}");
    let js = JunctionSet::new(&g);
    let p = js.patch(&g, 0).expect("the cluster");
    assert_eq!(p.members, vec![0, 1]);
    let inner = &g.edges[5];
    for (i, h) in inner.profile.iter().enumerate() {
        let s = sample_m(i, inner.length_m());
        let d = p.height_at([0.0, s]).unwrap();
        assert!((h - d).abs() < 0.02, "sample {i}: {h} vs {d}");
    }
    let pkg = package(&g);
    let (seams, without) = seams(&pkg, &g);
    assert!(without.is_empty(), "{without:?}");
    let cluster: Vec<&Seam> = seams.iter().filter(|s| s.patch).collect();
    assert_eq!(cluster.len(), 1, "{seams:?}");
    assert_eq!(cluster[0].degree, 5);
    // Within 3 cm where the patch's corner meets a carriageway's strip
    // beside the median (the lint's 2 cm on the real packages).
    assert!(cluster[0].seam_m < 0.03, "{:?}", cluster[0]);
}

/// The kinks of a profile (changes of grade above 8 points between two
/// intervals at least 1 m long), as the lint counts them.
fn kinks(p: &[f64], len: f64) -> Vec<(usize, f64)> {
    let d: Vec<f64> = (0..p.len()).map(|i| sample_m(i, len)).collect();
    let mut out = Vec::new();
    let mut prev: Option<f64> = None;
    for i in 0..p.len() - 1 {
        if d[i + 1] - d[i] < 1.0 {
            continue;
        }
        let g = (p[i + 1] - p[i]) / (d[i + 1] - d[i]);
        if let Some(pg) = prev
            && (g - pg).abs() > 0.08
        {
            out.push((i, (g - pg) * 100.0));
        }
        prev = Some(g);
    }
    out
}

/// Spec §4.8.8 Q7 (b): a side road climbing 12 % off a flat main road
/// leaves its edge with the main road's slope along it (0 %) and turns to
/// its own grade over a vertical curve of 8 m – no kink (J3 left a change
/// of 12 points at the edge) –, the drawn surface still follows the
/// profile, the main road is unchanged, and the road beyond keeps its own
/// profile.
#[test]
fn creases_are_rounded_into_a_vertical_curve() {
    assert_eq!(crease_curve_m(0.03), CREASE_MIN_M);
    assert!((crease_curve_m(0.12) - 8.0).abs() < 1e-9);
    assert_eq!(crease_curve_m(0.40), CREASE_MAX_M);
    let g0 = fixtures::t_junction_built(RoadClass::Secondary, 0.0, 0.12);
    let mut g = g0.clone();
    let st = reconcile_profiles(&mut g, &|_, _| true);
    assert!(st.residual_m < 0.02, "{st:?}");
    let side = &g.edges[2];
    let len = side.length_m();
    assert_eq!(kinks(&side.profile, len), vec![], "{:?}", side.profile);
    // The drawn surface along the side road's centre line: level with the
    // main road at its edge (3.75 m), gentle just past it (the main road's
    // 0 % turning), its own 12 % beyond the curve and the stretch that
    // makes up the height the curve left.
    let js = JunctionSet::new(&g);
    let at = |z: f64| {
        js.drawn_height(&g, 2, z, [0.0, z])
            .unwrap_or_else(|| side.height_at(z).unwrap())
    };
    assert!(at(3.75).abs() < 0.01, "{}", at(3.75));
    let early = at(5.25) - at(4.25);
    assert!(early < 0.05, "the grade just past the edge: {early}");
    let late = (side.height_at(65.0).unwrap() - side.height_at(60.0).unwrap()) / 5.0;
    assert!((late - 0.12).abs() < 0.01, "{late}");
    for e in 0..2 {
        assert_eq!(g.edges[e].profile, g0.edges[e].profile, "main road {e}");
    }
    let tail = g0.edges[2].profile.len() - 6;
    assert_eq!(side.profile[tail..], g0.edges[2].profile[tail..]);
    let r = crate::lint::lint(&package(&g), None);
    assert_eq!(r.count("road.vertical.kink"), 0);
    assert_eq!(r.count("road.vertical.grade_over_limit"), 0);
}

/// [`round_creases`] on a bare profile: the crease point and the far end
/// keep their heights, the road leaves the crease gently (the main road's
/// 0 %) and reaches its 15 % without a kink, the height the curve loses
/// spread within the limit; the same the other way along an edge.
#[test]
fn round_creases_shapes_the_curve_and_keeps_its_ends() {
    // A crease at 2.5 m (main road 0 % before it), then 15 %.
    let mut d = vec![0.0, 2.5];
    d.extend((1..12).map(|i| f64::from(i) * 5.0));
    let p0: Vec<f64> = d.iter().map(|&x| (x - 2.5).max(0.0) * 0.15).collect();
    let mut fixed = vec![false; d.len()];
    fixed[0] = true;
    fixed[1] = true;
    let mut crease = vec![None; d.len()];
    crease[1] = Some((0.0, 1.0));
    fixed[d.len() - 1] = true;
    let mut p = p0.clone();
    assert!(round_creases(&mut p, &d, &fixed, &crease, 0.25));
    assert_eq!(p[..2], p0[..2]);
    // The far end keeps its height; the road gets there a little steeper
    // than its 15 %, within the 25 % limit, without a kink.
    assert_eq!(p.last(), p0.last());
    let gr = grades(&p, &d);
    assert!(gr[1] < 0.06, "{gr:?}");
    assert!(gr.iter().all(|x| *x <= 0.25 + 1e-9), "{gr:?}");
    for w in gr.windows(2) {
        assert!((w[1] - w[0]).abs() < 0.08 + 1e-9, "{gr:?}");
    }
    // Close to a fixed point again (a second pass moves little).
    let again = p.clone();
    round_creases(&mut p, &d, &fixed, &crease, 0.25);
    for (a, b) in p.iter().zip(&again) {
        assert!((a - b).abs() < 0.05, "{p:?} vs {again:?}");
    }
    let p = again;
    // The same edge the other way round: the junction at its `to` end.
    let n = d.len();
    let len = d[n - 1];
    let dr: Vec<f64> = d.iter().rev().map(|x| len - x).collect();
    let mut pr: Vec<f64> = p0.iter().rev().copied().collect();
    let fixed_r: Vec<bool> = fixed.iter().rev().copied().collect();
    let mut crease_r = vec![None; n];
    crease_r[n - 2] = Some((0.0, -1.0));
    assert!(round_creases(&mut pr, &dr, &fixed_r, &crease_r, 0.25));
    let back: Vec<f64> = pr.iter().rev().copied().collect();
    for (a, b) in back.iter().zip(&p) {
        assert!((a - b).abs() < 1e-9, "{back:?} vs {p:?}");
    }
}

/// Spec §4.8.8 Q7: at no T junction the world builder shapes does a side
/// road's surface stand above (or poke through) the main road's drawn
/// surface – steep side roads up and down, at a right angle and at 30° and
/// 45°, and a side road meeting a dual carriageway (a cluster): the lint's
/// `junction.mouth.above_main` is 0.
#[test]
fn side_roads_stay_below_the_main_road_surface() {
    for g0 in [
        t_junction(RoadClass::Secondary, 0.08, 0.20),
        t_junction(RoadClass::Secondary, -0.06, -0.18),
        fixtures::t_junction_at(0.10, -0.15, 30.0),
        fixtures::t_junction_at(0.12, 0.20, 45.0),
        fixtures::dual_carriageway_t(),
    ] {
        let mut g = g0.clone();
        reconcile_profiles(&mut g, &|_, _| true);
        let pkg = package(&g);
        assert_eq!(mouths_above_main(&pkg, &g), vec![]);
        let r = crate::lint::lint(&pkg, None);
        assert_eq!(r.count("junction.mouth.above_main"), 0);
    }
}

/// Spec §4.8.8 Q7 (c): an equal crossing whose four roads climb and fall
/// differently (+8 %, −3 %, +2 %, +6 % outward): its junction surface is
/// one plane (the heights at the mouths and the centre within 2 cm of it),
/// each road turns from the plane's slope to its own grade over 10 – 20 m
/// past the mouth without a kink above 8 points and within its class
/// grade, the roads farther out keep their profiles, and the profiles are
/// the drawn surface.
#[test]
fn blended_junctions_lie_on_one_plane() {
    let g0 = crossing([0.08, -0.03, 0.02, 0.06]);
    let mut g = g0.clone();
    let st = reconcile_profiles(&mut g, &|_, _| true);
    assert_eq!(st.planes, 1, "{st:?}");
    assert!(st.residual_m < 0.02, "{st:?}");
    let js = JunctionSet::new(&g);
    let p = js.patch(&g, 0).expect("a patch");
    assert!(p.through.is_none());
    // The plane through the centre and two mouths holds the others.
    let pts: Vec<([f64; 2], f64)> = std::iter::once(([0.0, 0.0], p.height_at([0.0, 0.0]).unwrap()))
        .chain(
            p.arms
                .iter()
                .map(|a| (a.centre, p.height_at(a.centre).unwrap_or(a.h))),
        )
        .collect();
    let (c0, c1, c2) = (pts[0], pts[1], pts[2]);
    let det = (c1.0[0] - c0.0[0]) * (c2.0[1] - c0.0[1]) - (c2.0[0] - c0.0[0]) * (c1.0[1] - c0.0[1]);
    let gx = ((c1.1 - c0.1) * (c2.0[1] - c0.0[1]) - (c2.1 - c0.1) * (c1.0[1] - c0.0[1])) / det;
    let gz = ((c1.0[0] - c0.0[0]) * (c2.1 - c0.1) - (c2.0[0] - c0.0[0]) * (c1.1 - c0.1)) / det;
    for (q, h) in &pts {
        let on = c0.1 + gx * (q[0] - c0.0[0]) + gz * (q[1] - c0.0[1]);
        assert!((h - on).abs() < 0.02, "{q:?}: {h} vs {on}");
    }
    for (e, e0) in g.edges.iter().zip(&g0.edges) {
        let len = e.length_m();
        assert_eq!(
            kinks(&e.profile, len),
            vec![],
            "edge {}: {:?}",
            e.osm_way,
            e.profile
        );
        let d: Vec<f64> = (0..e.profile.len()).map(|i| sample_m(i, len)).collect();
        assert!(
            grades(&e.profile, &d)
                .iter()
                .all(|x| x.abs() <= 0.20 + 0.01)
        );
        // Past the mouth, the curve and a sample: unchanged.
        let a = p
            .arms
            .iter()
            .find(|a| g.edges[a.edge].osm_way == e.osm_way)
            .unwrap();
        for (i, (x, y)) in e.profile.iter().zip(&e0.profile).enumerate() {
            if sample_m(i, len) > a.mouth_m + PLANE_BLEND_MAX_M + 2.0 * PROFILE_STEP_M {
                assert_eq!(x, y, "edge {} sample {i}", e.osm_way);
            }
        }
    }
    let r = crate::lint::lint(&package(&g), None);
    assert_eq!(r.count("road.vertical.kink"), 0);
}

/// Spec §4.8.8 Q7 follow-up: a path drawn along a main road that curves
/// towards it (the path leaves at 15° and runs 0.3 – 0.5 m above the main
/// road, its strip over the main road's carriageway for 20 m) never
/// stands above the main road's surface: its rows near the carriageway
/// lie under it, also where the main road curves under a long quad –
/// before and after the reconciliation.
#[test]
fn a_path_along_a_curving_main_road_stays_below_it() {
    let v = [[0.0, 0.0], [-100.0, 0.0], [100.0, -20.0], [100.0, -32.0]];
    let main_e = Road {
        from: 0,
        to: 2,
        points: vec![
            [0.0, 0.0],
            [30.0, 0.0],
            [50.0, -2.0],
            [70.0, -6.0],
            [100.0, -20.0],
        ],
        class: RoadClass::Secondary,
        width_m: 7.5,
        height: Box::new(|s| s * 0.02),
    };
    let path = Road {
        from: 0,
        to: 3,
        points: vec![[0.0, 0.0], [25.0, -6.7], [60.0, -12.0], [100.0, -32.0]],
        class: RoadClass::Path,
        width_m: 1.8,
        height: Box::new(|s| s * 0.022 + 0.3),
    };
    let g0 = fixtures::graph(
        &v,
        vec![
            Road::straight(&v, 1, 0, RoadClass::Secondary, 7.5, |s| (s - 100.0) * 0.02),
            main_e,
            path,
        ],
    );
    let js = JunctionSet::new(&g0);
    assert!(
        js.patch(&g0, 0).is_some_and(|p| p.through.is_some()),
        "{:?}",
        js.fallback_reason(&g0, 0)
    );
    drop(js);
    let mut g = g0.clone();
    reconcile_profiles(&mut g, &|_, _| true);
    for gg in [&g0, &g] {
        let pkg = package(gg);
        assert_eq!(mouths_above_main(&pkg, gg), vec![]);
    }
}

/// Spec §4.8.8 Q7 follow-up: a cluster whose second vertex lies just off
/// the main road's carriageway (0.45 m), with paths leaving it 8 cm above
/// the main road: the junction surface's corners there do not lift the
/// main road's surface (the mesh refined and cut along the carriageway's
/// edge), before and after the reconciliation.
#[test]
fn a_cluster_vertex_beside_the_main_road_does_not_lift_it() {
    let v = [
        [0.0, 0.0],
        [-100.0, 0.0],
        [100.0, 0.0],
        [0.0, -4.2],
        [-60.0, -4.6],
        [60.0, -4.6],
    ];
    let g0 = fixtures::graph(
        &v,
        vec![
            Road::straight(&v, 1, 0, RoadClass::Primary, 7.5, |_| 0.0),
            Road::straight(&v, 0, 2, RoadClass::Primary, 7.5, |_| 0.0),
            Road::straight(&v, 0, 3, RoadClass::Path, 1.8, |s| s * 0.02),
            Road::straight(&v, 3, 4, RoadClass::Path, 1.8, |_| 0.08),
            Road::straight(&v, 3, 5, RoadClass::Path, 1.8, |_| 0.08),
        ],
    );
    let mut g = g0.clone();
    reconcile_profiles(&mut g, &|_, _| true);
    for gg in [&g0, &g] {
        let pkg = package(gg);
        assert_eq!(mouths_above_main(&pkg, gg), vec![]);
        let js = JunctionSet::new(gg);
        let p = js.patch(gg, 0).expect("a patch");
        // On the carriageway the drawn surface is the main road's own.
        let mut x = p.min[0];
        while x <= p.max[0] {
            let mut z = p.min[1];
            while z <= p.max[1] {
                let q = [x, z];
                if p.main_excess(q) <= 0.0
                    && let (Some(h), Some(m)) = (p.height_at(q), p.main_own(q))
                {
                    assert!(h - m < 0.02, "{q:?}: {h} vs {m}");
                }
                z += 0.1;
            }
            x += 0.1;
        }
    }
}

/// Spec §4.8.8 Q7 follow-up: a crease with too little room before the
/// next fixed height (a 22 m side road climbing 18 % to a level main road)
/// is rounded over a shorter curve or not at all – never steeper than the
/// class grade or the road was.
#[test]
fn creases_never_make_a_short_road_steeper() {
    let mut d = vec![0.0, 2.5];
    d.extend((1..5).map(|i| f64::from(i) * 5.0));
    d.push(22.0);
    let p0: Vec<f64> = d
        .iter()
        .map(|&x| (x - 2.5).clamp(0.0, 19.5) * 0.18)
        .collect();
    let mut fixed = vec![false; d.len()];
    fixed[0] = true;
    fixed[1] = true;
    fixed[d.len() - 1] = true;
    let mut crease = vec![None; d.len()];
    crease[1] = Some((0.0, 1.0));
    let before = grades(&p0, &d).iter().fold(0.0f64, |a, x| a.max(x.abs()));
    let mut p = p0.clone();
    round_creases(&mut p, &d, &fixed, &crease, 0.20);
    let after = grades(&p, &d).iter().fold(0.0f64, |a, x| a.max(x.abs()));
    assert!(after <= 0.20_f64.max(before) + 1e-6, "{after} {p:?}");
    assert_eq!(p[d.len() - 1], p0[d.len() - 1]);
}

/// The lint's `side.*` counts of the package the world builder makes of
/// `g` (its profiles reconciled with the junction surfaces).
fn side_counts(g0: &RoadGraph) -> [usize; 4] {
    let mut g = g0.clone();
    reconcile_profiles(&mut g, &|_, _| true);
    let pkg = package(&g);
    let r = crate::lint::lint(&pkg, None);
    ["side.gap", "side.step", "side.kink", "side.corner"].map(|k| r.count(k))
}

/// Spec §4.8.8 Rule (J5): a side road that bends by 20° (15° the other
/// way) 9 m out, at its mouth: the outline between the mouths runs along
/// its real edge and round the corner, leaving the mouth along the strip –
/// no bend above 25° in the sides (J4 drew the corner along the side
/// road's first 8 m as a straight line and met the mouth row, square to
/// the road past the bend, at an angle).
#[test]
fn junction_sides_follow_a_bending_side_road() {
    for g in [
        fixtures::t_junction_bent(0.02, 0.03, 20.0, 9.0),
        fixtures::t_junction_bent(0.0, -0.04, -15.0, 9.0),
    ] {
        let js = JunctionSet::new(&g);
        assert!(js.patch(&g, 0).is_some(), "{:?}", js.fallback_reason(&g, 0));
        drop(js);
        assert_eq!(side_counts(&g), [0; 4]);
    }
}

/// Spec §4.8.8 Rule (J5): a side road off a steep main road (15 % and
/// 12 %, at 90° and 60°): its mouth row leans with the main road's slope
/// across it and its strip turns flat beyond, so the corners between the
/// two roads do not twist within a few metres – no kink, no step in the
/// sides (J4 kept the mouth row flat: the corners twisted by the main
/// road's grade times the side road's half width).
#[test]
fn junction_sides_stay_smooth_beside_a_steep_main_road() {
    for g in [
        t_junction(RoadClass::Residential, 0.15, 0.0),
        fixtures::t_junction_at(0.12, 0.02, 60.0),
    ] {
        let [gap, step, kink, _] = side_counts(&g);
        assert_eq!([gap, step, kink], [0, 0, 0]);
    }
}

/// Spec §4.8.8 Rule (J5): a road that narrows at a way split (7.5 m to
/// 4 m, bending 20°): the sides taper along one smooth curve from mouth
/// to mouth.
#[test]
fn a_narrowing_road_tapers_smoothly() {
    let g = fixtures::split(20.0, 7.5, 4.0, 0.03);
    assert_eq!(side_counts(&g), [0; 4]);
}

/// Three junctions of the real packages where J5 left a side road above
/// the main road (`junction.mouth.above_main`), as their road graphs
/// (from the patch's centre: `from to class width x,z;… heights`): a
/// service road starting on a secondary road and climbing along it (Black
/// Forest middle, 4 cm above), a service road leaving a secondary road
/// along its edge (Tenerife, 2 cm), a residential road leaving at 28°
/// where the main road curves towards it (Tenerife, 6 – 13 cm).
const ALONG_MAIN: [&str; 3] = [
    "0 1 Service 3.50 -18.27,-9.94;1.47,-2.42 1.866,1.558,1.500,1.376,1.002,0.777
2 1 Service 3.50 -1.47,2.42;1.47,-2.42 0.028,0.573,0.777
1 3 Service 3.50 1.47,-2.42;17.76,-1.87;24.40,0.01 0.777,0.971,1.192,1.096,1.132,1.285
4 2 Secondary 8.00 -21.66,-5.28;-1.47,2.42 1.165,0.910,0.650,0.384,0.113,0.028
2 5 Secondary 8.00 -1.47,2.42;64.33,27.49;72.67,30.67;97.45,41.71;119.89,51.72;163.67,81.75 0.028,-0.247,-0.536,-0.824,-1.118,-1.416,-1.719,-2.026,-2.338,-2.654,-2.974,-3.299,-3.626,-3.958,-4.292,-4.630,-4.970,-5.313,-5.659,-6.006,-6.356,-6.707,-7.060,-7.414,-7.769,-8.126,-8.483,-8.841,-9.199,-9.558,-9.917,-10.277,-10.636,-10.996,-11.356,-11.715,-12.075,-12.372",
    "0 1 Service 3.50 0.00,0.00;-0.91,4.19;-7.30,4.33 0.000,0.406,1.559,1.697
2 0 Secondary 6.50 -89.95,-74.96;-28.86,-11.89;-16.97,-4.06;-9.58,-0.88;0.00,0.00 -4.486,-4.298,-4.108,-3.916,-3.724,-3.531,-3.337,-3.143,-2.949,-2.755,-2.562,-2.369,-2.178,-1.988,-1.800,-1.614,-1.429,-1.245,-1.064,-0.883,-0.704,-0.525,-0.346,-0.168,0.000
0 3 Secondary 6.50 0.00,0.00;17.88,-3.60 0.000,0.179,0.360,0.541,0.660",
    "0 1 Residential 5.00 3.87,-0.37;8.70,3.37;10.82,6.12;13.60,17.62;15.54,27.39;15.41,33.11;15.04,39.23;14.38,45.16;11.21,56.19;6.27,69.03;-0.88,87.35;-10.23,114.93;-12.27,121.95;-13.25,130.19;-14.36,151.59;-14.39,160.72;-14.41,165.57 0.527,0.204,-0.184,-0.638,-1.154,-1.729,-2.357,-3.031,-3.745,-4.494,-5.271,-6.073,-6.896,-7.735,-8.588,-9.454,-10.330,-11.216,-12.113,-13.021,-13.941,-14.874,-15.828,-16.828,-17.828,-18.828,-19.828,-20.828,-21.828,-22.828,-23.828,-24.828,-25.828,-26.828,-27.828,-28.674
2 3 Unclassified 5.00 -27.58,-46.08;-8.54,-9.54;-6.23,-4.69;-3.87,0.37 4.620,4.112,3.607,3.108,2.617,2.136,1.670,1.222,0.797,0.397,0.029,-0.116
3 4 Unclassified 5.00 -3.87,0.37;-2.47,5.19;-1.44,14.80 -0.116,-0.432,-0.708,-1.033
4 0 Residential 5.00 -1.44,14.80;0.31,10.99;1.56,8.10;2.62,4.94;3.87,-0.37 -1.033,-0.733,-0.254,0.432,0.527
0 5 Residential 5.00 3.87,-0.37;4.75,-9.59;4.07,-16.48;0.20,-38.71;-0.50,-54.60;0.65,-62.28;2.98,-69.19 0.527,0.881,1.305,1.798,2.356,2.975,3.646,4.362,5.117,5.905,6.719,7.556,8.412,9.284,10.111
3 0 Residential 5.00 -3.87,0.37;3.87,-0.37 -0.116,0.267,0.527",
];

/// A road graph from [`ALONG_MAIN`]'s lines (its profiles as given).
fn graph_of(text: &str) -> RoadGraph {
    use ocw_world::graph::{Edge, EdgeId, Oneway, VertexId};
    let mut vertices: Vec<[f64; 2]> = Vec::new();
    let mut edges = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let f: Vec<&str> = line.split_whitespace().collect();
        let (from, to): (u32, u32) = (f[0].parse().unwrap(), f[1].parse().unwrap());
        let class = match f[2] {
            "Secondary" => RoadClass::Secondary,
            "Unclassified" => RoadClass::Unclassified,
            "Residential" => RoadClass::Residential,
            "Service" => RoadClass::Service,
            other => panic!("{other}"),
        };
        let points: Vec<[f64; 2]> = f[4]
            .split(';')
            .map(|q| {
                let v: Vec<f64> = q.split(',').map(|x| x.parse().unwrap()).collect();
                [v[0], v[1]]
            })
            .collect();
        for (v, p) in [(from, points[0]), (to, *points.last().unwrap())] {
            let v = v as usize;
            if vertices.len() <= v {
                vertices.resize(v + 1, [f64::NAN; 2]);
            }
            vertices[v] = p;
        }
        edges.push(Edge {
            id: EdgeId(i as u64 + 1),
            from: VertexId(from),
            to: VertexId(to),
            points,
            class,
            name: None,
            width_m: f[3].parse().unwrap(),
            bridge: false,
            tunnel: false,
            dead_end_m: None,
            boundary: false,
            oneway: Oneway::No,
            osm_way: i as i64 + 1,
            profile: f[5].split(',').map(|x| x.parse().unwrap()).collect(),
        });
    }
    RoadGraph {
        vertices,
        edges,
        boundary_vertices: Default::default(),
    }
}

/// Spec §4.8.8 Rule (J6; the user: a side road never above the main
/// road): the junctions where J5 left a side road's strip above the main
/// road's drawn surface – side roads starting on the main road and
/// climbing along it, one leaving where the main road curves towards it –
/// have none: the mouth row of a side road with a corner on the main road
/// (or, leaving at a narrow angle, within a metre of it) lies on its
/// surface, and the strip's triangles over the main road's real
/// carriageway go under it.
#[test]
fn side_roads_along_the_main_road_never_stand_above_it() {
    for (k, text) in ALONG_MAIN.iter().enumerate() {
        let g = graph_of(text);
        let js = JunctionSet::new(&g);
        let patches = (0..g.vertices.len())
            .filter(|&v| js.own_patch(&g, v).is_some())
            .count();
        assert!(patches >= 1, "junction {k}: a patch");
        drop(js);
        let pkg = package(&g);
        let m = mouths_above_main(&pkg, &g);
        assert!(m.is_empty(), "junction {k}: {m:?}");
    }
}

/// Two junctions of Liechtenstein whose arms curve within the junction
/// (as [`ALONG_MAIN`]): J5's sides along the real edge lines would not
/// keep the outline star-shaped, so they kept J4's straight chain with a
/// `side.corner`; one also a `side.kink`.
const CURVING: [&str; 2] = [
    "0 1 Unclassified 5.00 61.04,314.22;0.00,0.00 0.683,0.680,0.663,0.629,0.593,0.561,0.527,0.491,0.454,0.415,0.374,0.333,0.291,0.249,0.206,0.164,0.123,0.082,0.043,0.005,-0.032,-0.067,-0.100,-0.131,-0.160,-0.186,-0.210,-0.231,-0.250,-0.267,-0.281,-0.293,-0.303,-0.311,-0.317,-0.321,-0.322,-0.322,-0.321,-0.317,-0.312,-0.304,-0.295,-0.285,-0.273,-0.261,-0.248,-0.234,-0.220,-0.205,-0.190,-0.174,-0.159,-0.143,-0.127,-0.112,-0.097,-0.082,-0.068,-0.054,-0.041,-0.029,-0.018,-0.009,-0.000,-0.000
1 2 Unclassified 5.00 0.00,0.00;-3.95,-34.34;-7.35,-54.02 -0.000,0.004,-0.009,-0.034,-0.055,-0.070,-0.085,-0.097,-0.108,-0.117,-0.125,-0.129
1 3 Unclassified 5.00 0.00,0.00;-5.20,-4.56;-9.94,-5.78;-16.78,-6.45;-30.66,-3.09;-48.72,3.04;-71.52,14.00;-77.89,17.89 -0.000,0.007,0.012,0.023,0.035,0.046,0.057,0.070,0.083,0.097,0.111,0.126,0.142,0.158,0.174,0.191,0.208,0.224",
    "0 1 Residential 5.00 59.07,-0.08;55.07,-0.11;0.00,0.00 8.899,8.741,8.274,7.558,6.591,5.636,4.708,3.847,3.106,2.464,1.771,0.852,-0.057
1 2 Residential 5.00 0.00,0.00;-19.40,0.50 -0.057,-0.943,-1.580,-2.020,-2.429
1 3 Residential 5.00 0.00,0.00;1.48,-1.27;2.64,-2.67;3.45,-4.14;4.03,-5.82;4.30,-7.56;4.29,-9.41;-0.96,-79.93;-2.98,-111.58 -0.057,0.472,1.133,1.204,1.002,0.618,0.341,0.164,0.066,0.057,0.074,0.092,0.110,0.128,0.147,0.165,0.184,0.203,0.223,0.242,0.262,0.282,0.301,0.315",
];

/// Spec §4.8.8 Rule (J6): an outline need not be star-shaped from the
/// vertex, only a simple polygon – its surface is then a constrained
/// Delaunay triangulation with points inside at the surface's heights –
/// so the sides follow the arms' real edge lines also where an arm curves
/// within the junction: no `side.corner` or `side.kink` there (J5: one
/// corner each, a kink at the second), and the surface meets the strips.
#[test]
fn curving_arms_get_sides_along_their_edges() {
    for (k, text) in CURVING.iter().enumerate() {
        let g = graph_of(text);
        let js = JunctionSet::new(&g);
        let p = js
            .patch(&g, 1)
            .unwrap_or_else(|| panic!("junction {k}: a patch"));
        assert!(!p.is_star(), "junction {k}: not star-shaped");
        drop(js);
        let pkg = package(&g);
        let r = crate::lint::lint(&pkg, None);
        let counts = ["side.gap", "side.step", "side.kink", "side.corner"].map(|k| r.count(k));
        assert_eq!(counts, [0; 4], "junction {k}: {:?}", side_faults(&pkg, &g));
        let (seams, without) = seams(&pkg, &g);
        assert!(without.is_empty(), "junction {k}: {without:?}");
        for s in seams.iter().filter(|s| s.patch) {
            assert!(s.seam_m < 0.02, "junction {k}: {s:?}");
        }
    }
}
