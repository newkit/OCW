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
        graph: Some(g.clone()),
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
        let before: BTreeMap<(usize, usize), f64> =
            drawn(&g0).into_iter().map(|(e, i, h)| ((e, i), h)).collect();
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
            assert!((g.edges[e].profile[i] - h).abs() < 0.005, "edge {e} sample {i}");
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
        // Far from the junction nothing changed.
        for (a, b) in g.edges.iter().zip(&g0.edges) {
            let len = a.length_m();
            for i in 0..a.profile.len() {
                let s = sample_m(i, len);
                let m = if a.from.0 == 0 { s } else { len - s };
                if m > 60.0 {
                    assert_eq!(a.profile[i], b.profile[i]);
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
    // The side road's first samples lie on the main road (flat across
    // along +z: the main road's height at x = 0 is 0).
    let side = &g.edges[2];
    assert!(side.profile[0].abs() < 1e-6);
    assert!(side.profile[1].abs() < 0.01, "{:?}", &side.profile[..4]);
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
    assert!(!patched_vertices(&fork(10.0))[0]);
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
    let f = fork(10.0);
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
            Road::straight(&v, 1, 0, RoadClass::Secondary, 7.5, |s| {
                (s - 120.0) * 0.06
            }),
            Road::straight(&v, 0, 2, RoadClass::Secondary, 7.5, |s| s * 0.06),
            Road::straight(&v, 0, 3, RoadClass::Residential, 5.0, |s| s * 0.20),
        ],
    );
    let p = JunctionSet::new(&g0).patch(&g0, 0).cloned().unwrap();
    let side = p.arms.iter().find(|a| a.edge == 2).unwrap().clone();
    assert!(side.blend_m < 5.0, "cut short: {side:?}");
    let mut g = g0.clone();
    let st = reconcile_profiles(&mut g, &|_, _| true);
    assert!(st.longer_blend_samples >= 2, "{st:?}");
    let seam = |g: &RoadGraph| {
        let (s, _) = seams(&package(g), g);
        s.iter().find(|s| s.vertex == 0).unwrap().seam_m
    };
    let (before, after) = (seam(&g0), seam(&g));
    assert!(after < 0.5 * before, "seam {before:.3} -> {after:.3}");
    // The far end keeps its height.
    assert_eq!(g.edges[2].profile.last(), g0.edges[2].profile.last());
}
