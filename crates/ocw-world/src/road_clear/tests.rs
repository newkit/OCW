use super::fixture::{FLAT_Y, world};
use super::*;
use crate::vegetation::PlantedTree;
use std::collections::BTreeMap;

fn spruce(scale: f32) -> PlantShape {
    PlantShape::of(Species::NorwaySpruce, scale)
}

#[test]
fn the_crown_shapes_follow_the_generator() {
    let s = spruce(1.0);
    assert!((s.height_m - 24.0).abs() < 1e-9);
    // A spruce's cone is widest at its base (2.4 m up) and narrows up.
    assert_eq!(s.crown_within(0.0, 2.0), 0.0);
    assert!((s.crown_within(0.0, 3.0) - 2.6 * LUMPS).abs() < 1e-6);
    assert!(s.crown_within(18.0, 24.0) < 1.0);
    // A lime's crown starts above a lorry (0.18 × 16 m = 2.9 m at scale
    // 1, 5.8 m at scale 2).
    let lime = PlantShape::of(Species::Lime, 2.0);
    assert_eq!(lime.crown_within(0.0, 4.5), 0.0);
    assert!(lime.crown_within(4.5, 40.0) > 10.0);
    // A palm's fronds only up high.
    let palm = PlantShape::of(Species::DatePalm, 1.0);
    assert_eq!(palm.crown_within(0.0, 6.0), 0.0);
    assert!(palm.crown_within(7.5, 11.0) > 4.0);
}

#[test]
fn the_need_by_height() {
    // A shrub of 1.8 m by a level road: its crown keeps off the edge.
    let broom = PlantShape::of(Species::Broom, 1.0);
    assert!((broom.need(0.0).unwrap() - (1.3 * LUMPS + CROWN_CLEAR_M)).abs() < 1e-6);
    // A large lime level with the road: its trunk keeps clear, its crown
    // (above 4.5 m at scale 2) may hang over by OVERHANG_M.
    let lime = PlantShape::of(Species::Lime, 2.0);
    let n = lime.need(0.0).unwrap();
    assert!((n - (lime.crown_m - OVERHANG_M)).abs() < 1e-9, "{n}");
    assert!(n > lime.trunk_m + TRUNK_CLEAR_M);
    // A spruce's low cone may not hang over the lane.
    let sp = spruce(1.0);
    assert!(sp.need(0.0).unwrap() >= 2.6 * LUMPS);
    // Under a viaduct 30 m up: no conflict; a road 30 m down in a cut:
    // only the overhang of the crown counts.
    assert_eq!(sp.need(30.0), None);
    assert!((sp.need(-30.0).unwrap() - (sp.crown_m - OVERHANG_M)).abs() < 1e-9);
    // A 1 m succulent 3 m below a road surface passes under it.
    assert_eq!(PlantShape::of(Species::Tabaiba, 1.0).need(3.0), None);
    // The need is never beyond the space's reach.
    assert!(PlantShape::of(Species::Oak, 2.0).need(0.0).unwrap() <= MAX_REACH_M);
}

#[test]
fn the_space_sees_every_road_shape() {
    let pkg = world();
    let space = RoadSpace::for_tile(&pkg, TileKey { x: 0, z: 0 });
    let y = f64::from(FLAT_Y);
    let lime = PlantShape::of(Species::Lime, 1.0);
    let at = |x: f64, z: f64| space.conflict([x, y, z], &lime);
    // The primary road's lane, its edge, and 10 m beside it.
    let c = at(250.0, 301.0).expect("on the lane");
    assert_eq!(c.surface, OnSurface::Road(RoadClass::Primary));
    assert_eq!(c.part, Part::Trunk);
    assert!(c.edge_m < 0.0);
    assert!(at(250.0, 300.0 + 3.75 + 0.5).is_some());
    assert!(at(250.0, 320.0).is_none());
    // The junction's corner: off both strips, on the stored surface.
    let c = at(408.0, 308.5).expect("in the junction");
    assert_eq!(c.surface, OnSurface::Junction);
    // The hairpin's apex, the roundabout's ring, the track and the path.
    assert!(at(712.0, 600.0).is_some());
    assert!(at(215.0, 700.0).is_some());
    assert!(at(200.0, 700.0).is_none(), "the island of the roundabout");
    assert!(at(800.5, 500.0).is_some());
    assert!(at(50.0, 450.0).is_some());
    // The road of the next tile, from this tile's space (the graph).
    let c = at(998.5, 600.0).expect("the next tile's road");
    assert_eq!(c.surface, OnSurface::Road(RoadClass::Residential));
    // The bridge: a lime of 16 m under the deck 12 m up hits it, a 1 m
    // succulent passes under; beside the deck the crown is clear.
    assert!(at(300.0, 900.0).is_some());
    let tabaiba = PlantShape::of(Species::Tabaiba, 1.0);
    assert!(space.clear([300.0, y, 900.0], &tabaiba));
}

#[test]
fn the_check_finds_the_plants_on_the_roads_and_the_placement_leaves_them_out() {
    let mut pkg = world();
    let key = TileKey { x: 0, z: 0 };
    // The mapped trees on roads are not drawn at all (the placement's
    // own test): only the one far off the roads stays of the nine.
    let mapped = crate::plantings::mapped_trees(&pkg, key);
    let feet: Vec<[f32; 2]> = mapped.iter().map(|(f, _)| [f[0], f[2]]).collect();
    assert_eq!(feet, vec![[600.0, 450.0]]);
    // The check still finds them where the package stores them (older
    // clients draw what VEGE holds): every source on the road listed.
    let r = check_package(&pkg);
    assert!(r.plants > 1000, "{}", r.plants);
    let stored: Vec<&Violation> = r
        .violations
        .iter()
        .filter(|v| v.plant.source == Source::StoredTree)
        .collect();
    assert_eq!(stored.len(), 8, "{:#?}", r.violations);
    assert_eq!(stored.len(), r.violations.len(), "{:#?}", r.violations);
    let surfaces = r.by_surface();
    assert!(surfaces.contains_key(&OnSurface::Junction), "{surfaces:?}");
    assert!(
        surfaces.contains_key(&OnSurface::Road(RoadClass::Path)),
        "{surfaces:?}"
    );
    // The world builder keeps only the trees the placement draws: then
    // nothing of the package stands on a road.
    let kept: Vec<PlantedTree> = mapped.iter().map(|(_, t)| *t).collect();
    let veg = pkg.look.vegetation.as_mut().unwrap();
    veg.tiles.get_mut(&key).unwrap().trees = kept;
    let r = check_package(&pkg);
    assert!(
        r.violations.is_empty(),
        "{:#?}",
        &r.violations[..r.violations.len().min(5)]
    );
}

#[test]
fn no_grid_tree_and_no_stored_trunk_on_a_road() {
    let mut pkg = world();
    pkg.look.vegetation = None;
    // The trunks OBST stores, as the world builder makes them.
    let mut o = crate::obstacles::Obstacles {
        tree_side: 125,
        tiles: BTreeMap::new(),
    };
    for (key, tile) in &pkg.tiles {
        let cells = crate::trees::tree_cells(&pkg, tile, |_| true).unwrap();
        o.tiles.entry(*key).or_default().set_tree_cells(&cells);
    }
    pkg.obstacles = Some(o);
    let r = check_package(&pkg);
    assert!(r.plants > 10_000, "{}", r.plants);
    assert!(
        r.violations.is_empty(),
        "{:#?}",
        &r.violations[..r.violations.len().min(5)]
    );
    // Forest right up to the roads: the grid keeps trees beside them,
    // only off the surface.
    let space = RoadSpace::for_tile(&pkg, TileKey { x: 0, z: 0 });
    let near = tile_plants(&pkg, TileKey { x: 0, z: 0 })
        .iter()
        .filter(|p| space.near([p.foot[0], p.foot[2]], 8.0))
        .count();
    assert!(near > 50, "{near}");
}

#[test]
fn a_planted_violation_is_found_with_its_place_and_source() {
    let pkg = world();
    let space = RoadSpace::for_tile(&pkg, TileKey { x: 0, z: 0 });
    // A plant the placement would never make, put in by hand.
    let foot = [250.0, f64::from(FLAT_Y), 300.5];
    let c = space
        .conflict(foot, &PlantShape::of(Species::Beech, 1.0))
        .expect("on the road");
    assert_eq!(c.part, Part::Trunk);
    assert!((c.at[1] - 300.0).abs() < 1e-9);
    assert!(c.by_m > 3.0);
}
