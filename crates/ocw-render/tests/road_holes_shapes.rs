//! The road-hole detector (`ocw_render::road_holes`) on synthetic road
//! shapes (spec 4.4 *Road surface without holes*; bug 2026-11-28, holes in
//! Andorra's streets): hairpins of r = 8 m and tighter with wide roads, a
//! junction on a 12 % slope, a tile seam across a curve, the start of a
//! bridge, a pothole-ridden road. Each is meshed as the renderer meshes it
//! and probed from above: no hole, no culled (down-facing) carriageway.

use std::collections::HashMap;

use bevy::math::{Vec2, Vec3};
use ocw_render::junction_mesh::{PackageJunctions, shape_tile, shaped_meshes};
use ocw_render::package::MeshArrays;
use ocw_render::road_holes::{SurfaceIndex, inverted_area_m2, probe_strips, spots};
use ocw_render::road_mesh::{RoadContext, road_meshes};
use ocw_world::graph::RoadClass;
use ocw_world::junction::fixtures;
use ocw_world::package::{RoadStrip, TileKey};
use ocw_world::road_condition::{RoadCondition, Smoothness};
use ocw_world::road_dress::MarkingStyle;
use ocw_world::{RoadDress, RoadLit, RoadSurface};

const TILE: f64 = 1000.0;

fn strip(points: Vec<[f32; 3]>, width: f32, class: RoadClass) -> RoadStrip {
    let mut dress = RoadDress::of_class(class, width);
    dress.edge_m = points
        .windows(2)
        .map(|w| Vec3::from(w[0]).distance(Vec3::from(w[1])))
        .sum();
    RoadStrip {
        class,
        width_m: width,
        points,
        surface: RoadSurface::of_class(class),
        dress,
        lit: RoadLit::Unknown,
        condition: RoadCondition::UNKNOWN,
    }
}

/// Probes `strips` against `meshes`; panics with the worst spots.
fn assert_no_holes(what: &str, strips: &[RoadStrip], meshes: &[&MeshArrays]) {
    let mut index = SurfaceIndex::new();
    let mut inverted = 0.0;
    for m in meshes {
        index.add(m);
        inverted += inverted_area_m2(m, &[0]);
    }
    let (n, holes) = probe_strips(strips, &index, &|_| true);
    assert!(n > 100, "{what}: only {n} samples");
    let s = spots(&holes);
    assert!(
        holes.is_empty() && inverted < 1e-3,
        "{what}: {} of {n} samples in holes, {inverted:.3} m² culled carriageway; spots: {:?}",
        holes.len(),
        &s[..s.len().min(5)]
    );
}

fn ctx<'a>(
    style: &'a MarkingStyle,
    ground: &'a dyn Fn(f32, f32) -> Option<f32>,
    deck: &'a dyn Fn([f32; 3]) -> bool,
) -> RoadContext<'a> {
    RoadContext {
        colours: None,
        ground,
        deck,
        style,
    }
}

/// A hairpin: in along −x at z = 0, round a half circle of `r` about
/// (0, r)… back out along +x at z = 2r; points every `step` m on the arc
/// (OSM-like), climbing `grade`.
fn hairpin(r: f32, step: f32, grade: f32) -> Vec<[f32; 3]> {
    let mut pts = Vec::new();
    let mut s = 0.0f32;
    let mut push = |p: Vec2, s: f32| pts.push([p.x, s * grade, p.y]);
    for k in 0..=6 {
        let x = 60.0 - 10.0 * k as f32;
        push(Vec2::new(x, 0.0), s);
        s += 10.0;
    }
    s -= 10.0;
    let arc = std::f32::consts::PI * r;
    let n = (arc / step).ceil() as i32;
    for k in 1..=n {
        let a = std::f32::consts::PI * k as f32 / n as f32;
        // Turning right (towards +z) round the centre (0, r).
        let p = Vec2::new(-r * a.sin(), r - r * a.cos());
        push(p, s + arc * k as f32 / n as f32);
    }
    s += arc;
    for k in 1..=6 {
        let x = 10.0 * k as f32;
        push(Vec2::new(x, 2.0 * r), s + 10.0 * k as f32);
    }
    pts
}

#[test]
fn hairpins_with_wide_roads_have_no_holes() {
    let style = MarkingStyle::default();
    let ground = |_: f32, _: f32| Some(-20.0f32);
    let deck = |_: [f32; 3]| false;
    let c = ctx(&style, &ground, &deck);
    for (r, width, step) in [
        (8.0, 7.0, 3.0),
        (8.0, 12.0, 3.0),
        // As wide as the curve: the inner edge meets at the centre (a
        // road still wider would overlap its own other leg).
        (8.0, 16.0, 2.0),
        (5.0, 10.0, 1.5),
        (4.0, 7.0, 1.0),
        // Digitised coarsely: three points round the apex.
        (8.0, 12.0, 9.0),
    ] {
        let s = strip(hairpin(r, step, 0.08), width, RoadClass::Secondary);
        let m = road_meshes(std::slice::from_ref(&s), &c);
        assert_no_holes(
            &format!("hairpin r {r} m, {width} m wide, points {step} m apart"),
            std::slice::from_ref(&s),
            &[&m.roads],
        );
    }
}

/// The fixture's strips per tile, split as the world builder splits them
/// (each run reaches one point into the next tile).
fn tiles(strips: impl IntoIterator<Item = RoadStrip>) -> HashMap<TileKey, Vec<RoadStrip>> {
    let mut out: HashMap<TileKey, Vec<RoadStrip>> = HashMap::new();
    for s in strips {
        let mut runs: Vec<(TileKey, Vec<[f32; 3]>)> = Vec::new();
        for (i, p) in s.points.iter().enumerate() {
            let key = TileKey::containing([f64::from(p[0]), f64::from(p[2])], TILE);
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

#[test]
fn a_junction_on_a_12_percent_slope_has_no_holes() {
    let style = MarkingStyle::default();
    let ground = |_: f32, _: f32| Some(-50.0f32);
    let deck = |_: [f32; 3]| false;
    let c = ctx(&style, &ground, &deck);
    for g in [
        // As the world builder leaves them (the side road on the main
        // road's carriageway up to its edge, J3).
        fixtures::t_junction_built(RoadClass::Secondary, 0.12, 0.12),
        fixtures::t_junction_built(RoadClass::Secondary, 0.12, -0.12),
        fixtures::t_junction_built(RoadClass::Primary, -0.12, 0.10),
    ] {
        let pj = PackageJunctions::of_graph(&g, TILE);
        let strips: Vec<RoadStrip> = fixtures::strips(&g).into_iter().map(|(_, s)| s).collect();
        let mut meshes = Vec::new();
        for (key, ts) in tiles(strips.clone()) {
            let t = shape_tile(&g, &pj, key, TILE, &ts, &style);
            meshes.push(shaped_meshes(&t, &c).roads);
        }
        let refs: Vec<&MeshArrays> = meshes.iter().collect();
        assert_no_holes("junction on a 12 % slope", &strips, &refs);
    }
}

#[test]
fn a_tile_seam_across_a_curve_has_no_holes() {
    let style = MarkingStyle::default();
    let ground = |_: f32, _: f32| Some(-20.0f32);
    let deck = |_: [f32; 3]| false;
    let c = ctx(&style, &ground, &deck);
    // A curve of r = 12 m round (1000, 12) across the seam x = 1000, a
    // point every 4 m, 8 m wide, climbing 10 %.
    let mut pts = Vec::new();
    for k in 0..=30 {
        let a = -1.2 + 2.4 * k as f32 / 30.0;
        let p = Vec2::new(1000.0 + 12.0 * a.sin(), 12.0 - 12.0 * a.cos());
        pts.push([p.x, 0.1 * 12.0 * (a + 1.2), p.y]);
    }
    let s = strip(pts, 8.0, RoadClass::Secondary);
    let split = tiles([s.clone()]);
    assert_eq!(split.len(), 2, "the curve crosses the seam");
    let meshes: Vec<MeshArrays> = split.values().map(|ts| road_meshes(ts, &c).roads).collect();
    let refs: Vec<&MeshArrays> = meshes.iter().collect();
    assert_no_holes("tile seam across a curve", &[s], &refs);
}

#[test]
fn the_start_of_a_bridge_has_no_holes() {
    use ocw_world::bridges::{
        Bridge, BridgeMaterial, BridgeSource, BridgeStructure, Pier, Railing,
    };
    use ocw_world::graph::EdgeId;
    let style = MarkingStyle::default();
    // Ground along the approach, falling away 15 m under the deck (x > 0).
    let ground = |x: f32, _: f32| Some(if x < 0.0 { -0.1 } else { -15.0 });
    let b = Bridge {
        edge: EdgeId(1),
        osm_way: 1,
        structure: BridgeStructure::Beam,
        material: BridgeMaterial::Concrete,
        source: BridgeSource::Tagged,
        railing: Railing::Parapet,
        width_m: 7.0,
        deck_m: 0.8,
        points: vec![[0.0, 0.0, 0.0], [20.0, 0.4, 3.0], [40.0, 0.8, 8.0]],
        abutment_foot_y: [-15.0, -15.0],
        piers: vec![Pier {
            along_m: 20.0,
            foot_y: -15.0,
        }],
        arches: vec![],
    };
    let bridges = [&b];
    let deck = move |p: [f32; 3]| ocw_render::bridges::on_deck(&bridges, p);
    let c = ctx(&style, &ground, &deck);
    // The approach and the deck as two strips meeting at the abutment,
    // bending there, and one strip running through.
    let approach = strip(
        vec![[-40.0, 0.0, -6.0], [-20.0, 0.0, -2.0], [0.0, 0.0, 0.0]],
        7.0,
        RoadClass::Secondary,
    );
    let on = strip(
        vec![[0.0, 0.0, 0.0], [20.0, 0.4, 3.0], [40.0, 0.8, 8.0]],
        7.0,
        RoadClass::Secondary,
    );
    let m = road_meshes(&[approach.clone(), on.clone()], &c);
    assert_no_holes("bridge start", &[approach, on], &[&m.roads]);
}

#[test]
fn potholes_are_painted_not_cut() {
    let style = MarkingStyle::default();
    let ground = |_: f32, _: f32| Some(-0.1f32);
    let deck = |_: [f32; 3]| false;
    let c = ctx(&style, &ground, &deck);
    let mut s = strip(
        (0..=20).map(|k| [5.0 * k as f32, 0.0, 0.0]).collect(),
        6.0,
        RoadClass::Unclassified,
    );
    s.surface = RoadSurface::WornAsphalt;
    s.condition.smoothness = Smoothness::VeryHorrible;
    let m = road_meshes(std::slice::from_ref(&s), &c);
    assert_no_holes("pothole road", std::slice::from_ref(&s), &[&m.roads]);
}
