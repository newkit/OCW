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
            // starts at the main road's height and is shorter).
            if let Some(b) = before.get(&(e, i)) {
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
    assert!((side.profile[1] - 0.25).abs() < 0.03, "{:?}", &side.profile[..4]);
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
    // side road leaves the main road's edge (3.75 m out) at its own grade
    // (§4.8.8 Q6 (d)): 18 % × 1.25 m at its 5 m sample.
    assert_eq!(side.profile[0], g.edges[1].profile[0]);
    assert!((side.profile[1] - 0.225).abs() < 0.03, "{:?}", &side.profile[..4]);
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
        (st.steep_room, st.steep_intervals, st.steep_other, st.steep_source),
        (0, 0, 0, 0),
        "{st:?}"
    );
    let t = &g.edges[4];
    let d: Vec<f64> = (0..t.profile.len()).map(|i| sample_m(i, t.length_m())).collect();
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
