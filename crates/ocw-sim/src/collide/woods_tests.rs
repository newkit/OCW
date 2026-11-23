//! The trunks off the road in the world cache (spec 14 Phase 13 Q10 (a)):
//! no doubles with the road-side trees of `OBST`, cells made and dropped
//! as a rider moves, the memory bounded.

use super::*;
use ocw_world::obstacles::{Obstacles, TileObstacles};
use ocw_world::woods::fixture::{KEY, wood_package};

/// The test wood with `OBST` holding the trees within 62 m of its road
/// (x = 503), as the world builder stores them, and the cache over it.
fn cache() -> (WorldCache, Arc<WorldPackage>) {
    let mut pkg = wood_package();
    let near = |p: [f64; 2]| (p[0] - 503.0).abs() <= 62.0;
    let tile = &pkg.tiles[&KEY];
    let cells = ocw_world::trees::tree_cells(&pkg, tile, near).unwrap();
    let mut t = TileObstacles {
        points: ocw_world::plantings::planted_points(&pkg, KEY, near),
        ..Default::default()
    };
    t.set_tree_cells(&cells);
    let mut o = Obstacles::new(1000.0);
    o.tiles.insert(KEY, t);
    pkg.obstacles = Some(o);
    let world = WorldObstacles::from_package(&pkg).map(Arc::new);
    let pkg = Arc::new(pkg);
    let woods = Arc::new(Woods::new(pkg.clone()));
    (WorldCache::new(world).with_woods(Some(woods)), pkg)
}

use ocw_world::WorldPackage;

fn centre(s: &Solid) -> [f64; 2] {
    match s.shape {
        Shape::Circle { c, .. } => c,
        Shape::Segment { a, .. } => a,
    }
}

#[test]
fn the_road_side_trees_and_the_trunks_off_the_road_do_not_double() {
    let (mut cache, pkg) = cache();
    // Across the edge of the stored band (x = 565).
    let (lo, hi) = ([520.0, 100.0], [610.0, 900.0]);
    let inside = |c: [f64; 2]| c[0] >= 530.0 && c[0] < 600.0 && c[1] >= 110.0 && c[1] < 890.0;
    let mut road_side = Vec::new();
    cache.solids_near(lo, hi, 1, &mut road_side);
    let mut deep = Vec::new();
    cache.trunks_near(lo, hi, 1, &mut deep);
    let road_side: Vec<[f64; 2]> = road_side
        .iter()
        .filter(|s| s.kind == ObstacleKind::Tree)
        .map(centre)
        .filter(|c| inside(*c))
        .collect();
    let deep: Vec<[f64; 2]> = deep.iter().map(centre).filter(|c| inside(*c)).collect();
    assert!(
        road_side.len() > 200 && deep.len() > 200,
        "{} / {}",
        road_side.len(),
        deep.len()
    );
    // No tree twice.
    let mut all: Vec<[f64; 2]> = road_side.iter().chain(&deep).copied().collect();
    all.sort_by(|a, b| a.partial_cmp(b).unwrap());
    for w in all.windows(2) {
        assert!(
            (w[0][0] - w[1][0]).hypot(w[0][1] - w[1][1]) > 0.01,
            "a double at {:?}",
            w[0]
        );
    }
    // Every drawn tree there is one of them (shrubs are soft off the road).
    let gt = ocw_world::flora::GridTrees::of_tile(&pkg, KEY);
    let drawn = ocw_world::trees::classed_tree_sites(&pkg, &pkg.tiles[&KEY])
        .into_iter()
        .filter(|((p, s), class)| {
            let c = [f64::from(p[0]), f64::from(p[2])];
            let (sp, _) = gt.tree(&pkg, *class, *p, *s);
            inside(c) && ((c[0] - 503.0).abs() <= 62.0 || sp.solid())
        })
        .count();
    assert_eq!(all.len(), drawn);
}

#[test]
fn the_cells_are_made_and_dropped_as_a_rider_moves() {
    let (mut cache, _) = cache();
    // A rider deep in the wood (x = 810) riding north at 10 m/s.
    let mut most = 0;
    let mut tick = 0u64;
    for k in 0..4500 {
        tick = k + 1;
        let z = 950.0 - k as f64 * 0.2;
        let mut out = Vec::new();
        let r = BODY_LENGTH_M / 2.0 + 0.5;
        cache.trunks_near([810.0 - r, z - r], [810.0 + r, z + r], tick, &mut out);
        if tick.is_multiple_of(250) {
            cache.sweep(tick);
        }
        most = most.max(cache.wood_cells());
        // The cell under the rider is there.
        let here = WoodCell::containing([810.0, z], 1000.0);
        assert!(cache.wood_cells.contains_key(&here));
    }
    // Kept 10 s, swept every 5 s: at most 150 m of riding at 10 m/s – a
    // few cells, never the wood.
    assert!(most <= 6, "{most} cells at once");
    assert!(cache.wood_cells_made() >= 22, "{}", cache.wood_cells_made());
    assert!(cache.wood_bytes() < 100_000, "{} bytes", cache.wood_bytes());
    // Gone 10 s after the rider left.
    cache.sweep(tick + (WOOD_CELL_KEEP_S / SIM_DT) as u64 + 1);
    assert_eq!(cache.wood_cells(), 0);
    assert_eq!(cache.wood_tiles.len(), 0);
}

#[test]
fn the_cells_are_bounded() {
    let (mut cache, _) = cache();
    let mut out = Vec::new();
    // A box over the whole tile at once would be 625 cells; many riders
    // far apart are held to the cap.
    for (i, cell) in WoodCell::covering([0.0, 0.0], [999.0, 999.0], 1000.0)
        .into_iter()
        .enumerate()
    {
        out.clear();
        let o = [
            f64::from(cell.cx) * 40.0 + 20.0,
            f64::from(cell.cz) * 40.0 + 20.0,
        ];
        cache.trunks_near(o, o, i as u64, &mut out);
    }
    assert_eq!(cache.wood_cells(), 625);
    let trunks = cache.wood_trunks();
    assert!(trunks > 10_000, "{trunks}");
    // A whole square kilometre of dense wood: well under 2 MB.
    assert!(
        cache.wood_bytes() < 2_000_000,
        "{} bytes",
        cache.wood_bytes()
    );
}
