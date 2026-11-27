use std::collections::BTreeMap;

use ocw_world::graph::RoadClass;
use ocw_world::junction::fixtures::{self, Road};
use ocw_world::landuse::{LandUse, LandUseTile};
use ocw_world::package::{Tile, TileKey};

use super::*;

const N: u32 = 201;
const SIZE: f32 = 1000.0;
/// The road runs east – west here, south of the hill.
const ROAD_Z: f64 = -450.0;

/// A hill (20 m high, 300 m across) with small bumps (±0.6 m, 8 – 16 m
/// across) on four 1 km tiles, and a straight road beside it at its own
/// heights (the terrain conformed to it within 10 m).
fn hilly(road: bool) -> WorldPackage {
    let g = fixtures::graph(
        &[[-600.0, ROAD_Z], [600.0, ROAD_Z]],
        vec![Road::straight(
            &[[-600.0, ROAD_Z], [600.0, ROAD_Z]],
            0,
            1,
            RoadClass::Secondary,
            7.0,
            |s| 400.0 + s * 0.01,
        )],
    );
    let ground = |x: f64, z: f64| {
        let hill = 20.0 * (-(x * x + z * z) / (2.0 * 150.0f64.powi(2))).exp();
        let bumps =
            0.6 * ((x * 0.5).sin() * (z * 0.45).cos() + 0.5 * (x * 0.7 + z * 0.4).sin()) / 1.5;
        400.0 + hill + bumps
    };
    let mut tiles = BTreeMap::new();
    let sp = f64::from(SIZE) / f64::from(N - 1);
    for (tx, tz) in [(-1, -1), (0, -1), (-1, 0), (0, 0)] {
        let key = TileKey { x: tx, z: tz };
        let o = key.origin(f64::from(SIZE));
        let mut heights = Vec::with_capacity((N * N) as usize);
        for k in 0..N {
            for i in 0..N {
                let (x, z) = (o[0] + f64::from(i) * sp, o[1] + f64::from(k) * sp);
                let h = if road && (z - ROAD_Z).abs() <= 10.0 {
                    400.0 + (x + 600.0) * 0.01 - 0.05
                } else {
                    ground(x, z)
                };
                heights.push(h as f32);
            }
        }
        let roads = if road {
            fixtures::strips(&g)
                .into_iter()
                .map(|(_, s)| s)
                .filter(|s| {
                    let p = s.points[s.points.len() / 2];
                    TileKey::containing([f64::from(p[0]), f64::from(p[2])], f64::from(SIZE)) == key
                })
                .collect()
        } else {
            Vec::new()
        };
        tiles.insert(
            key,
            Tile {
                key,
                heights,
                roads,
            },
        );
    }
    WorldPackage {
        name: "Hill".into(),
        origin: (47.0, 9.0),
        tile_size_m: SIZE,
        samples: N,
        tiles,
        graph: road.then(|| g.into()),
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
        traffic: None,
        aero: None,
    }
}

/// The window's weights sum to 1 and keep planes and quadratic surfaces
/// (hills, ridges and valleys) exactly; a single bump is flattened.
#[test]
fn the_kernel_keeps_quadratic_surfaces() {
    for (r, sigma) in [(1, 0.75), (2, 1.0), (3, 1.5)] {
        let k = quadratic_kernel(r, sigma);
        assert_eq!(k.len(), ((2 * r + 1) * (2 * r + 1)) as usize);
        let sum: f64 = k.iter().map(|q| q.2).sum();
        assert!((sum - 1.0).abs() < 1e-9, "{sum}");
        let surf =
            |x: f64, z: f64| 3.0 + 0.2 * x - 0.1 * z + 0.05 * x * x - 0.02 * x * z + 0.03 * z * z;
        let fit: f64 = k
            .iter()
            .map(|&(i, kk, w)| w * surf(i as f64, kk as f64))
            .sum();
        assert!((fit - surf(0.0, 0.0)).abs() < 1e-9, "r {r}: {fit}");
        // A spike of 1 m on flat ground: lowered.
        let centre = k.iter().find(|q| q.0 == 0 && q.1 == 0).unwrap().2;
        assert!(centre < 0.85, "r {r}: {centre}");
    }
}

/// Every global vertex with its height in each tile holding it.
fn copies(pkg: &WorldPackage) -> BTreeMap<(i64, i64), Vec<f32>> {
    let n = pkg.samples as usize;
    let step = (n - 1) as i64;
    let mut out: BTreeMap<(i64, i64), Vec<f32>> = BTreeMap::new();
    for (key, t) in &pkg.tiles {
        for k in 0..n {
            for i in 0..n {
                let g = (
                    i64::from(key.x) * step + i as i64,
                    i64::from(key.z) * step + k as i64,
                );
                out.entry(g).or_default().push(t.heights[k * n + i]);
            }
        }
    }
    out
}

/// The bumps go (the terrain's roughness falls), no vertex moves more than
/// the limit, the hill keeps its shape (its top within the limit), the road
/// and the terrain conformed to it keep their heights, and every copy of a
/// tile-border vertex gets the same height.
#[test]
fn smoothing_keeps_the_roads_and_its_bounds() {
    // Settled as a built package is.
    let mut before = hilly(true);
    let samples = crate::junctions::surface_samples(
        before.graph.as_ref().unwrap(),
        &ocw_world::junction::JunctionSet::new(before.graph.as_ref().unwrap()),
        0.5,
    );
    crate::clean::settle_terrain_under_surfaces(&mut before, 0.05, &samples);
    let mut pkg = before.clone();
    let (st, settle) = smooth_package_terrain(&mut pkg, RADIUS_M, MAX_CHANGE_M, true);
    assert!(st.changed > 10_000, "{st:?}");
    assert!(st.max_change_m <= MAX_CHANGE_M + 1e-6, "{st:?}");
    assert!(st.mean_change_m > 0.02 && st.mean_change_m < 0.3, "{st:?}");
    assert!(st.mean_signed_m.abs() < 0.05, "{st:?}");
    assert!(st.kept > 0, "{st:?}");
    assert_eq!(settle.vertices_lowered, 0, "{settle:?}");
    // The road: the graph and its strips untouched.
    assert_eq!(pkg.graph, before.graph);
    for (a, b) in pkg.tiles.values().zip(before.tiles.values()) {
        assert_eq!(a.roads, b.roads);
    }
    let (b, a) = (copies(&before), copies(&pkg));
    let sp = pkg.spacing_m();
    let mut rough = [0.0f64; 2];
    let mut peak = [f32::MIN; 2];
    for ((gi, gk), hs) in &a {
        // Shared border vertices: one height.
        assert!(hs.windows(2).all(|w| w[0] == w[1]), "{gi},{gk}: {hs:?}");
        let (x, z) = (*gi as f64 * sp, *gk as f64 * sp);
        let old = b[&(*gi, *gk)][0];
        assert!((hs[0] - old).abs() <= (MAX_CHANGE_M + 1e-4) as f32);
        // Under the road and a grid spacing beyond its edge: kept.
        if (z - ROAD_Z).abs() <= 3.5 + sp && x.abs() < 590.0 {
            assert_eq!(hs[0], old, "the road at {x},{z}");
        }
        peak[0] = peak[0].max(old);
        peak[1] = peak[1].max(hs[0]);
        // Roughness: the step to the next vertex east (away from the road).
        if let (Some(e0), Some(e1)) = (b.get(&(gi + 1, *gk)), a.get(&(gi + 1, *gk)))
            && (z - ROAD_Z).abs() > 40.0
        {
            rough[0] += f64::from((e0[0] - old).abs());
            rough[1] += f64::from((e1[0] - hs[0]).abs());
        }
    }
    assert!(rough[1] < 0.8 * rough[0], "{rough:?}");
    // The hill's top (420 m) lowered by at most the limit.
    assert!(peak[0] - peak[1] <= MAX_CHANGE_M as f32 + 1e-4, "{peak:?}");
    assert!(peak[1] > 419.0, "{peak:?}");
}

/// Water (its levels) and the ground under buildings stay; without them
/// the same vertices would move.
#[test]
fn smoothing_keeps_water_and_building_sites() {
    let mut pkg = hilly(false);
    let n = N as usize;
    let key = TileKey { x: 0, z: 0 };
    // A pond at vertices (10..20, 10..20) of tile (0, 0), a house at
    // (300 m, 300 m).
    let water: Vec<(u32, f32)> = (10..20)
        .flat_map(|k| (10..20).map(move |i| ((k * n + i) as u32, 399.0f32)))
        .collect();
    let mut lu_tiles = BTreeMap::new();
    lu_tiles.insert(
        key,
        LandUseTile {
            classes: Vec::new(),
            water,
        },
    );
    pkg.landuse = Some(LandUse {
        cells: 0,
        tiles: lu_tiles,
    });
    let house = ocw_world::buildings::Building {
        kind: ocw_world::buildings::BuildingKind::House,
        outline: vec![
            [295.0, 295.0],
            [305.0, 295.0],
            [305.0, 305.0],
            [295.0, 305.0],
        ],
        holes: Vec::new(),
        base_y: 399.0,
        eave_y: 405.0,
        roof: ocw_world::buildings::Roof::Flat,
        tags: Default::default(),
        parts: Default::default(),
    };
    let mut b = BTreeMap::new();
    b.insert(key, vec![house]);
    pkg.buildings = Some(b);
    let before = pkg.clone();
    let free = {
        let mut p = hilly(false);
        smooth_terrain(&mut p, &|_| false, &[], RADIUS_M, MAX_CHANGE_M);
        p
    };
    let st = smooth_terrain(&mut pkg, &|_| false, &[], RADIUS_M, MAX_CHANGE_M);
    assert!(st.kept >= 100, "{st:?}");
    let (t0, t1, tf) = (&before.tiles[&key], &pkg.tiles[&key], &free.tiles[&key]);
    let mut moved_free = 0;
    for (i, k) in [(15, 15), (12, 18), (60, 60), (59, 61)] {
        let at = k * n + i;
        assert_eq!(t1.heights[at], t0.heights[at], "vertex {i},{k}");
        if tf.heights[at] != t0.heights[at] {
            moved_free += 1;
        }
    }
    assert!(
        moved_free >= 3,
        "without water and building they would move"
    );
}
