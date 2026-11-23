use super::fixture::{KEY, wood_package};
use super::*;

/// Every cell of the tile.
fn all_cells() -> Vec<WoodCell> {
    WoodCell::covering([0.0, 0.0], [999.9, 999.9], 1000.0)
}

#[test]
fn the_cells_cover_the_tile_once() {
    assert_eq!(cells_per_tile(1000.0), 25);
    let cells = all_cells();
    assert_eq!(cells.len(), 625);
    let mut seen = vec![false; 125 * 125];
    for c in &cells {
        let (cols, rows) = c.candidates(1000.0);
        for k in rows {
            for i in cols.clone() {
                assert!(!seen[k * 125 + i], "candidate {i},{k} twice");
                seen[k * 125 + i] = true;
            }
        }
    }
    assert!(seen.iter().all(|s| *s));
    // A place is in the cell its 40 m square says, also across tiles.
    let c = WoodCell::containing([85.0, 1039.0], 1000.0);
    assert_eq!(
        c,
        WoodCell {
            tile: TileKey { x: 0, z: 1 },
            cx: 2,
            cz: 0
        }
    );
    let c = WoodCell::containing([-1.0, 5.0], 1000.0);
    assert_eq!((c.tile, c.cx, c.cz), (TileKey { x: -1, z: 0 }, 24, 0));
    assert_eq!(
        WoodCell::covering([990.0, 10.0], [1010.0, 20.0], 1000.0).len(),
        2
    );
}

#[test]
fn a_window_places_the_same_trees_as_the_tile() {
    // Each cell's trees are the whole tile's there, the road's margin
    // and the orchard's rows included.
    let pkg = wood_package();
    let tile = &pkg.tiles[&KEY];
    let (lu, cells) = tile_landuse(&pkg, KEY).unwrap();
    let whole = crate::trees::landuse_trees_in(&pkg, tile, lu, cells, 0..125, 0..125);
    assert!(whole.len() > 10_000, "{}", whole.len());
    let mut parts = Vec::new();
    for c in all_cells() {
        let (cols, rows) = c.candidates(1000.0);
        parts.extend(crate::trees::landuse_trees_in(
            &pkg, tile, lu, cells, cols, rows,
        ));
    }
    parts.sort_by_key(|t| t.0);
    let mut whole = whole;
    whole.sort_by_key(|t| t.0);
    assert_eq!(parts, whole);
    // No tree in the road's margin (half width + 3 m).
    assert!(
        whole
            .iter()
            .all(|(_, _, p, _)| (p[0] - 503.0).abs() > 1.5 + 3.0 - 1e-3)
    );
}

#[test]
fn the_trunks_are_the_drawn_trees() {
    // Each grid tree's trunk: at its foot, the drawn species' trunk at the
    // drawn scale; shrubs none.
    let pkg = Arc::new(wood_package());
    let woods = Woods::new(pkg.clone());
    let gt = GridTrees::of_tile(&pkg, KEY);
    let drawn: Vec<([f64; 2], f64)> = crate::trees::classed_tree_sites(&pkg, &pkg.tiles[&KEY])
        .into_iter()
        .filter_map(|((p, s), class)| {
            let (sp, scale) = gt.tree(&pkg, class, p, s);
            // Whole millimetres.
            let r = f64::from((sp.trunk_radius_m() * scale * 1000.0).round()) / 1000.0;
            (r > 0.0).then(|| ([f64::from(p[0]), f64::from(p[2])], r))
        })
        .collect();
    let mut made = Vec::new();
    for c in all_cells() {
        made.extend(woods.grid_trunks(c, &Stored::default()).unwrap());
    }
    assert_eq!(made.len(), drawn.len());
    let mut a: Vec<_> = made.iter().map(|t| (t.c, t.r)).collect();
    let mut b = drawn.clone();
    let key = |v: &([f64; 2], f64)| (v.0[0].to_bits(), v.0[1].to_bits());
    a.sort_by_key(key);
    b.sort_by_key(key);
    assert_eq!(a, b);
    // Forest trunks: a spruce of 24 m at scale 1 has 0.38 m.
    assert!(made.iter().all(|t| (0.1..1.0).contains(&t.r)), "radii");
    // Ids are unique.
    let mut ids: Vec<u64> = made.iter().map(|t| t.id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), made.len());
    // The planted trees: the orchard's rows and the lime.
    let planted = woods.planted_trunks(KEY, &Stored::default()).unwrap();
    assert!(planted.len() > 20, "{}", planted.len());
    assert!(
        planted
            .iter()
            .any(|t| (t.c[0] - 900.0).abs() < 1e-3 && (t.c[1] - 900.0).abs() < 1e-3)
    );
    // Deterministic.
    let again = Woods::new(Arc::new(wood_package()));
    let c = WoodCell::containing([300.0, 300.0], 1000.0);
    assert_eq!(
        again.grid_trunks(c, &Stored::default()),
        woods.grid_trunks(c, &Stored::default())
    );
}

#[test]
fn the_stored_road_side_trees_are_left_out() {
    // OBST keeps the trees within 60 m of the road; the cells add the
    // others – together every drawn trunk once.
    let pkg = Arc::new(wood_package());
    let woods = Woods::new(pkg.clone());
    let tile = &pkg.tiles[&KEY];
    let near = |p: [f64; 2]| (p[0] - 503.0).abs() <= 62.0;
    let cells = crate::trees::tree_cells(&pkg, tile, near).unwrap();
    let planted_near: Vec<[f32; 2]> = crate::plantings::planted_points(&pkg, KEY, near)
        .iter()
        .map(|p| p.pos)
        .collect();
    let stored = Stored {
        cells: Some((125, cells.clone())),
        planted: planted_near.clone(),
    };
    let obst = crate::trees::trunks_from_cells(KEY, 1000.0, &cells);
    assert!(obst.len() > 1000, "{}", obst.len());
    let mut deep = Vec::new();
    let mut all = Vec::new();
    for c in all_cells() {
        deep.extend(woods.grid_trunks(c, &stored).unwrap());
        all.extend(woods.grid_trunks(c, &Stored::default()).unwrap());
    }
    // None of the deep ones is a stored one.
    for t in &deep {
        assert!(!near(t.c), "{:?} is stored", t.c);
        assert!(
            obst.iter()
                .all(|(p, _)| (p[0] - t.c[0]).hypot(p[1] - t.c[1]) > 1e-3)
        );
    }
    // Every drawn trunk is one or the other.
    let drawn_near = all.iter().filter(|t| near(t.c)).count();
    assert_eq!(deep.len() + drawn_near, all.len());
    assert!(drawn_near <= obst.len());
    // The planted trees near the road are not made again.
    let p_all = woods.planted_trunks(KEY, &Stored::default()).unwrap();
    let p_deep = woods.planted_trunks(KEY, &stored).unwrap();
    assert_eq!(p_deep.len() + planted_near.len(), p_all.len());
}

#[test]
fn a_tile_not_loaded_gives_nothing_yet() {
    let woods = Woods::empty(1000.0);
    let c = WoodCell::containing([300.0, 300.0], 1000.0);
    assert!(woods.grid_trunks(c, &Stored::default()).is_none());
    assert!(!woods.has_tile(KEY));
    woods.set_package(Arc::new(wood_package()));
    assert!(woods.has_tile(KEY));
    assert!(!woods.grid_trunks(c, &Stored::default()).unwrap().is_empty());
    let far = WoodCell::containing([1300.0, 300.0], 1000.0);
    assert!(woods.grid_trunks(far, &Stored::default()).is_none());
}

#[test]
fn a_servers_baked_tiles_give_the_same_trunks() {
    let pkg = wood_package();
    let woods = Woods::new(Arc::new(wood_package()));
    let baked = Woods::bake(&pkg);
    assert!(baked.package().is_none());
    // A forested tile: well under its 31 KB of radii.
    assert!(
        baked.baked_bytes() < 25_000,
        "{} bytes",
        baked.baked_bytes()
    );
    let near = |p: [f64; 2]| (p[0] - 503.0).abs() <= 62.0;
    let stored = Stored {
        cells: Some((
            125,
            crate::trees::tree_cells(&pkg, &pkg.tiles[&KEY], near).unwrap(),
        )),
        planted: crate::plantings::planted_points(&pkg, KEY, near)
            .iter()
            .map(|p| p.pos)
            .collect(),
    };
    for c in all_cells() {
        for s in [&Stored::default(), &stored] {
            assert_eq!(baked.grid_trunks(c, s), woods.grid_trunks(c, s));
        }
    }
    for s in [&Stored::default(), &stored] {
        assert_eq!(baked.planted_trunks(KEY, s), woods.planted_trunks(KEY, s));
    }
    let far = WoodCell::containing([1300.0, 300.0], 1000.0);
    assert!(!baked.has_tile(far.tile));
    assert!(baked.grid_trunks(far, &Stored::default()).is_none());
}
