use std::collections::{BTreeMap, HashSet};

use ocw_world::landuse::LandClass;
use ocw_world::package::{Tile, TileKey, WorldPackage};

use super::*;
use crate::landuse::{Area, WaterLine, add_landuse, add_landuse_with_sea};

/// 4 × 4 tiles of 100 m (11 × 11 vertices, 10 m), a gentle slope with
/// some noise (the DEM's: the ground beside the lake partly below the
/// lake's level).
fn package() -> WorldPackage {
    let n = 11;
    let mut tiles = BTreeMap::new();
    for x in 0..4 {
        for z in 0..4 {
            let key = TileKey { x, z };
            let heights = (0..n * n)
                .map(|j| {
                    let (gi, gk) = (x as usize * 10 + j % n, z as usize * 10 + j / n);
                    let noise = ((gi * 7 + gk * 13) % 5) as f64 * 0.35 - 0.7;
                    (500.0 + gi as f64 * 0.08 + gk as f64 * 0.05 + noise) as f32
                })
                .collect();
            tiles.insert(
                key,
                Tile {
                    key,
                    heights,
                    roads: vec![],
                },
            );
        }
    }
    WorldPackage {
        name: "shore".into(),
        origin: (47.0, 9.0),
        tile_size_m: 100.0,
        samples: n as u32,
        tiles,
        graph: None,
        landuse: None,
        buildings: None,
        bridges: None,
        routes: Vec::new(),
        region: None,
        time_zone: None,
        look: Default::default(),
        obstacles: None,
        places: None,
        junctions: None,
        traffic: None,
        aero: None,
    }
}

/// A round lake (radius `r`, 72 points) around `c`.
fn round_lake(c: [f64; 2], r: f64) -> Area {
    let ring = (0..72)
        .map(|j| {
            let a = j as f64 / 72.0 * std::f64::consts::TAU;
            [c[0] + r * a.cos(), c[1] + 0.8 * r * a.sin()]
        })
        .collect();
    Area::new(LandClass::Water, vec![ring])
}

fn river() -> WaterLine {
    WaterLine {
        points: vec![[5.0, 330.0], [120.0, 300.0], [230.0, 370.0], [395.0, 340.0]],
        width_m: 12.0,
    }
}

fn no_road(_: [f64; 2]) -> bool {
    false
}

/// The drawn shoreline of a lake follows the OSM outline: before the fit
/// it followed the 10 m grid (offsets of metres, the zig-zag), after it
/// every sampled point of the outline lies within 0.6 m of where the drawn
/// terrain crosses the water.
#[test]
fn the_drawn_shoreline_follows_the_outline_not_the_grid() {
    let mut p = package();
    let lake = round_lake([150.0, 150.0], 72.0);
    add_landuse(&mut p, std::slice::from_ref(&lake), &[], 20);
    let outlines = outlines(std::slice::from_ref(&lake), None);
    assert_eq!(outlines.len(), 1);
    let before = shore_offsets(&p, &outlines, 2.0, 15.0, &no_road);
    let st = fit_shores(&mut p, &outlines, &HashSet::new(), &no_road);
    assert!(st.band > 100 && st.no_level == 0, "{st:?}");
    assert!(
        st.raised > 0 && st.lowered > 0 && st.new_levels > 0,
        "{st:?}"
    );
    let after = shore_offsets(&p, &outlines, 2.0, 15.0, &no_road);
    assert!(after.len() > 150, "{} samples", after.len());
    let worst = |v: &[f64]| v.last().copied().unwrap_or(0.0);
    let p90 = |v: &[f64]| v[(v.len() - 1) * 9 / 10];
    assert!(
        worst(&before) > 2.5,
        "the old shore followed the grid: worst {:.2} m",
        worst(&before)
    );
    assert!(
        worst(&after) < 0.6 && p90(&after) < 0.3,
        "worst {:.2} m, p90 {:.2} m",
        worst(&after),
        p90(&after)
    );
}

/// A river 12 m wide and a stream 3 m wide become ribbons along their
/// course: the river at levels that never rise downstream over a carved
/// channel, its grid water (the staircase) gone, ending at the lake it
/// flows into; the stream draped on the ground (its level below it).
#[test]
fn rivers_and_streams_are_ribbons_along_their_course() {
    let mut p = package();
    let lake = round_lake([150.0, 150.0], 72.0);
    let stream = WaterLine {
        points: vec![[300.0, 20.0], [330.0, 120.0], [360.0, 250.0]],
        width_m: 3.0,
    };
    // The river flows from the west into the lake.
    let inflow = WaterLine {
        points: vec![[5.0, 330.0], [120.0, 300.0], [150.0, 190.0]],
        width_m: 12.0,
    };
    let lines = vec![river(), stream, inflow];
    add_landuse(&mut p, std::slice::from_ref(&lake), &lines, 20);
    let had_levels: usize = p
        .landuse
        .as_ref()
        .unwrap()
        .tiles
        .values()
        .map(|t| t.water.len())
        .sum();
    let outlines = outlines(std::slice::from_ref(&lake), None);
    fit_shores(&mut p, &outlines, &HashSet::new(), &no_road);
    let st = rivers(&mut p, &lines, &outlines, &HashSet::new(), &no_road);
    assert_eq!((st.wide, st.draped), (2, 1), "{st:?}");
    assert!(
        st.carved > 10 && st.levels_dropped > 10 && st.in_areas > 0,
        "{st:?}"
    );
    let w = p.look.water_lines.as_ref().expect("WATR");
    let all: Vec<&ocw_world::water_lines::WaterLine> = w.tiles.values().flatten().collect();
    // The river: levels never rise along the flow; the terrain under its
    // centre line lies below its surface.
    for l in all.iter().filter(|l| l.width_m == 12.0) {
        assert!(l.points.windows(2).all(|q| q[1][1] <= q[0][1] + 1e-4));
        for q in &l.points {
            let g = p.surface_height(f64::from(q[0]), f64::from(q[2])).unwrap();
            assert!(
                g < f64::from(q[1]) - 0.3,
                "ground {g:.2} under the river at {q:?}"
            );
        }
    }
    // The inflow ends at the lake's shore (one point in the water).
    let inflow_end = all
        .iter()
        .filter(|l| l.width_m == 12.0)
        .flat_map(|l| l.points.iter())
        .map(|q| outlines[0].signed_distance([f64::from(q[0]), f64::from(q[2])]))
        .fold(f64::INFINITY, f64::min);
    assert!(inflow_end > -RIBBON_STEP_M - 0.1, "{inflow_end}");
    // The stream lies on the ground: its level is below it.
    for l in all.iter().filter(|l| l.width_m == 3.0) {
        for q in &l.points {
            let g = p.surface_height(f64::from(q[0]), f64::from(q[2])).unwrap();
            assert!(f64::from(q[1]) < g - 1.0);
        }
    }
    // No grid water along the river any more (the lake keeps its own).
    let left: usize = p
        .landuse
        .as_ref()
        .unwrap()
        .tiles
        .values()
        .map(|t| t.water.len())
        .sum();
    assert!(left < had_levels + st.levels_dropped);
    let lu = p.landuse.as_ref().unwrap();
    let key = TileKey { x: 0, z: 3 };
    let at_river = (2 * 11 + 3) as u32; // (30, 320): beside the river's centre
    assert_eq!(lu.tiles[&key].water_level(at_river), None);
    // Survives the package round trip.
    assert_eq!(WorldPackage::from_bytes(&p.to_bytes()).unwrap(), p);
}

/// Where the DEM beside a lake lies below the lake's level, the land of
/// the band is raised out of the water: no water stands over the land
/// (the slab with stepped walls); and the band's cells all have water
/// levels, so the surface is drawn up to the outline.
#[test]
fn the_land_beside_the_water_stands_above_it() {
    let mut p = package();
    let lake = round_lake([150.0, 150.0], 72.0);
    add_landuse(&mut p, std::slice::from_ref(&lake), &[], 20);
    let outlines = outlines(std::slice::from_ref(&lake), None);
    fit_shores(&mut p, &outlines, &HashSet::new(), &no_road);
    let sp = p.spacing_m();
    for (v, (d, _)) in band_distances(&p, &outlines, BAND_CELLS * sp) {
        let (level, h) = vertex_state(&p, v).unwrap();
        let level = level.expect("every band vertex has a level");
        if d > 0.5 {
            assert!(
                h > level + 0.05,
                "{v:?} {d:.1} m out: ground {h:.2} under the water {level:.2}"
            );
        }
        if d < -0.5 {
            assert!(h < level - 0.05, "{v:?} {d:.1} m in: {h:.2} vs {level:.2}");
        }
    }
}

/// Roads, junction surfaces and building sites keep their heights;
/// shared tile borders get the same height and level in both tiles.
#[test]
fn kept_vertices_stay_and_tile_borders_agree() {
    let mut p = package();
    let lake = round_lake([200.0, 150.0], 60.0);
    add_landuse(&mut p, std::slice::from_ref(&lake), &[], 20);
    let outlines = outlines(std::slice::from_ref(&lake), None);
    let before = p.clone();
    // A road along x = 140 … 150 (the lake's west shore).
    let road = |q: [f64; 2]| (138.0..=152.0).contains(&q[0]);
    let kept: HashSet<(i64, i64)> = [(20, 15)].into_iter().collect();
    let st = fit_shores(&mut p, &outlines, &kept, &road);
    assert!(st.kept > 0);
    for k in 0..=40i64 {
        for i in [14i64, 15] {
            let a = vertex_state(&before, (i, k)).unwrap().1;
            let b = vertex_state(&p, (i, k)).unwrap().1;
            assert_eq!(a, b, "road vertex {i},{k}");
        }
    }
    assert_eq!(
        vertex_state(&before, (20, 15)).unwrap().1,
        vertex_state(&p, (20, 15)).unwrap().1
    );
    // x = 200 is the border of tiles 1 and 2.
    let lu = p.landuse.as_ref().unwrap();
    for z in 0..4 {
        let (a, b) = (TileKey { x: 1, z }, TileKey { x: 2, z });
        for k in 0..11 {
            assert_eq!(
                p.tiles[&a].heights[k * 11 + 10],
                p.tiles[&b].heights[k * 11],
                "tile z {z} row {k}"
            );
            assert_eq!(
                lu.tiles[&a].water_level((k * 11 + 10) as u32),
                lu.tiles[&b].water_level((k * 11) as u32)
            );
        }
    }
}

/// The sea keeps its level of 0 m and its shore rises out of it along
/// the coastline.
#[test]
fn the_sea_keeps_level_zero_along_its_fitted_coast() {
    let mut p = package();
    for t in p.tiles.values_mut() {
        for h in &mut t.heights {
            *h -= 500.0;
        }
    }
    // An island: the frame minus a round coast.
    let coast = round_lake([200.0, 200.0], 150.0).rings[0].clone();
    let frame = vec![
        [-1000.0, -1000.0],
        [1400.0, -1000.0],
        [1400.0, 1400.0],
        [-1000.0, 1400.0],
    ];
    let sea = Area::sea(vec![frame, coast]);
    add_landuse_with_sea(&mut p, &[], Some(&sea), &[], 20);
    let outlines = outlines(&[], Some(&sea));
    let st = fit_shores(&mut p, &outlines, &HashSet::new(), &no_road);
    assert!(st.band > 50 && st.no_level == 0, "{st:?}");
    let lu = p.landuse.as_ref().unwrap();
    assert!(
        lu.tiles
            .values()
            .flat_map(|t| &t.water)
            .all(|&(_, level)| level == 0.0)
    );
    let at = shore_offsets_at(&p, &outlines, 3.0, 15.0, &no_road);
    let worst: Vec<_> = at.iter().rev().take(5).collect();
    assert!(
        at.len() > 100 && at.last().unwrap().0 < 0.6,
        "{} points, worst {worst:?}, {st:?}",
        at.len()
    );
}
