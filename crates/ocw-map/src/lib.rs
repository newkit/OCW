//! The 2D map of a world package (spec §3.2 *Navigation map*, §14 Phase
//! 16, 16A), shared by OCW Studio's map and the game HUD's navigation
//! map. Bevy-free and wasm-safe: the projections ([`view`]), the studio
//! map's data ([`data`]), the styles and palettes ([`style`]), the
//! navigation map's source with its road index ([`source`]), its tile
//! cache ([`ring`]), the CPU rasteriser of its tiles and icons
//! ([`raster`]), its zoom, banner and route rules ([`navi`]) and the place
//! names' hook ([`labels`]) and the training history's ride map
//! ([`ride`]). Only data from the package itself – never
//! online map tiles (spec §4).

pub mod data;
pub mod labels;
pub mod navi;
pub mod raster;
pub mod ride;
pub mod ring;
pub mod source;
pub mod style;
pub mod view;

pub use data::{LandRaster, MapBuilding, MapData, MapRoad, bbox, overlaps};
pub use style::{format_length, nice_length, road_style};
pub use view::{MapView, NaviView};

/// A made-up package for the tests (here and in the apps): two tiles of
/// 100 m side by side, land use in the first (forest left, water right),
/// one building, a road graph with a named main road along z = 50 and a
/// side road north from it.
pub mod fixture {
    use std::collections::BTreeMap;

    use ocw_world::buildings::{Building, BuildingKind, Roof};
    use ocw_world::graph::{Edge, EdgeId, Oneway, RoadClass, RoadGraph, VertexId};
    use ocw_world::landuse::{LandClass, LandUse, LandUseTile};
    use ocw_world::package::{RoadStrip, Tile, TileKey, WorldPackage};

    fn edge(
        id: u64,
        from: u32,
        to: u32,
        points: Vec<[f64; 2]>,
        class: RoadClass,
        name: &str,
    ) -> Edge {
        Edge {
            id: EdgeId(id),
            from: VertexId(from),
            to: VertexId(to),
            points,
            class,
            name: (!name.is_empty()).then(|| name.to_string()),
            width_m: class.default_width_m(),
            bridge: false,
            tunnel: false,
            dead_end_m: None,
            boundary: false,
            oneway: Oneway::No,
            osm_way: id as i64,
            profile: Vec::new(),
        }
    }

    pub fn package() -> WorldPackage {
        let n = 3;
        let tile = |x| Tile {
            key: TileKey { x, z: 0 },
            heights: vec![400.0; n * n],
            roads: vec![RoadStrip {
                class: RoadClass::Secondary,
                width_m: 6.0,
                points: vec![
                    [x as f32 * 100.0, 400.0, 50.0],
                    [x as f32 * 100.0 + 100.0, 400.0, 50.0],
                ],
                surface: ocw_world::RoadSurface::Asphalt,
                dress: ocw_world::RoadDress::of_class(RoadClass::Secondary, 6.0),
                lit: ocw_world::RoadLit::Unknown,
                condition: Default::default(),
            }],
        };
        let classes = vec![
            LandClass::Forest,
            LandClass::Water,
            LandClass::Forest,
            LandClass::Water,
        ];
        let graph = RoadGraph {
            vertices: vec![[0.0, 50.0], [100.0, 50.0], [200.0, 50.0], [100.0, 0.0]],
            edges: vec![
                edge(
                    1,
                    0,
                    1,
                    vec![[0.0, 50.0], [100.0, 50.0]],
                    RoadClass::Secondary,
                    "Landstrasse",
                ),
                edge(
                    2,
                    1,
                    2,
                    vec![[100.0, 50.0], [200.0, 50.0]],
                    RoadClass::Secondary,
                    "Landstrasse",
                ),
                edge(
                    3,
                    1,
                    3,
                    vec![[100.0, 50.0], [100.0, 0.0]],
                    RoadClass::Residential,
                    "Kirchweg",
                ),
            ],
            boundary_vertices: Default::default(),
        };
        WorldPackage {
            name: "testland".into(),
            origin: (47.0, 9.0),
            tile_size_m: 100.0,
            samples: n as u32,
            tiles: BTreeMap::from([
                (TileKey { x: 0, z: 0 }, tile(0)),
                (TileKey { x: 1, z: 0 }, tile(1)),
            ]),
            graph: Some(graph),
            landuse: Some(LandUse {
                cells: 2,
                tiles: BTreeMap::from([(
                    TileKey { x: 0, z: 0 },
                    LandUseTile {
                        classes,
                        water: Vec::new(),
                    },
                )]),
            }),
            buildings: Some(BTreeMap::from([(
                TileKey { x: 0, z: 0 },
                vec![Building {
                    kind: BuildingKind::House,
                    outline: vec![[10.0, 10.0], [20.0, 10.0], [20.0, 20.0], [10.0, 20.0]],
                    holes: Vec::new(),
                    base_y: 399.0,
                    eave_y: 406.0,
                    roof: Roof::Flat,
                    tags: Default::default(),
                    parts: Default::default(),
                }],
            )])),
            bridges: None,
            routes: Vec::new(),
            region: None,
            time_zone: None,
            look: Default::default(),
            obstacles: None,
            places: None,
            junctions: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::raster::{
        MapLine, TILE_PX, TileSpec, compass, render_base_tile, render_line_tile, rider_arrow,
        turn_arrow,
    };
    use crate::source::MapSource;
    use crate::style::{Palette, Shade};
    use ocw_world::landuse::LandClass;

    fn px(v: &[u8], x: u32, y: u32) -> [u8; 4] {
        let i = ((y * TILE_PX + x) * 4) as usize;
        [v[i], v[i + 1], v[i + 2], v[i + 3]]
    }

    fn spec() -> TileSpec {
        // 1 m per texel: tile (0, 0) covers 0 … 128 m.
        TileSpec {
            ix: 0,
            iz: 0,
            m_per_texel: 1.0,
            density: 1.0,
        }
    }

    #[test]
    fn the_studio_map_holds_land_use_buildings_and_roads_of_the_package() {
        let rgb = |c: LandClass| [c as u8, 0, 0];
        let m = MapData::of(&fixture::package(), rgb);
        assert_eq!((m.min, m.max), ([0.0, 0.0], [200.0, 100.0]));
        let r = m.land.as_ref().unwrap();
        assert_eq!((r.width, r.height, r.m_per_texel), (4, 2, 50.0));
        assert_eq!(r.rgba[0], LandClass::Forest as u8);
        assert_eq!(m.roads.len(), 3, "the graph's edges");
        assert_eq!(m.buildings[0].triangles.len(), 2);
    }

    /// Deterministic rendering for a fixed position: the same tile twice
    /// gives the same bytes, and its pixels are the package's: forest,
    /// water, the building, the main road with its casing, the world's
    /// edge outside.
    #[test]
    fn a_map_tile_draws_the_package_the_same_every_time() {
        let src = MapSource::single(Arc::new(fixture::package()));
        let pal = Palette::of(Shade::Day);
        let a = render_base_tile(&src, &pal, &spec());
        let b = render_base_tile(&src, &pal, &spec());
        assert_eq!(a.len(), (TILE_PX * TILE_PX * 4) as usize);
        assert!(a == b, "deterministic");
        let rgb = |c: [u8; 4]| [c[0], c[1], c[2], 255];
        // Forest in the first land-use cell (0 … 50 m), water beside it.
        assert_eq!(px(&a, 30, 40), rgb(pal.forest));
        assert_eq!(px(&a, 70, 20), rgb(pal.water));
        // The building at 10 … 20 m (1 m per texel: drawn at this zoom).
        assert_eq!(px(&a, 15, 15), rgb(pal.building));
        // The main road along z = 50 (fill in the middle).
        assert_eq!(px(&a, 30, 50), rgb(pal.road_main));
        // The side road north from x = 100.
        assert_eq!(px(&a, 100, 20), rgb(pal.road_minor));
        // Beyond the package (z > 100 m): outside.
        assert_eq!(px(&a, 64, 120), rgb(pal.outside));
        // Opaque everywhere (the cache uses alpha for "drawn").
        assert!(a.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
        // Night: another look of the same tile.
        let n = render_base_tile(&src, &Palette::of(Shade::Night), &spec());
        assert_eq!(px(&n, 30, 40), rgb(Palette::of(Shade::Night).forest));
        // Far out: no buildings, no residential road.
        let far = TileSpec {
            m_per_texel: 8.0,
            ..spec()
        };
        let f = render_base_tile(&src, &pal, &far);
        assert_ne!(px(&f, 2, 2), rgb(pal.building));
        assert_ne!(px(&f, 12, 3), rgb(pal.road_minor));
    }

    #[test]
    fn the_route_layer_is_transparent_beside_the_route() {
        let pal = Palette::of(Shade::Day);
        let lines = vec![MapLine {
            points: vec![[0.0, 64.0], [128.0, 64.0]],
            width: 5.0,
            colour: pal.route,
            casing: Some((2.0, pal.route_casing)),
            layer: 2,
        }];
        let t = render_line_tile(&lines, &spec()).unwrap();
        assert_eq!(px(&t, 64, 64), pal.route);
        assert_eq!(px(&t, 64, 10)[3], 0, "transparent off the route");
        // A tile far away: nothing to draw.
        let far = TileSpec { ix: 5, ..spec() };
        assert!(render_line_tile(&lines, &far).is_none());
        assert!(render_line_tile(&lines, &spec()).unwrap() == t);
    }

    #[test]
    fn icons_have_their_shapes() {
        let a = rider_arrow(32, [30, 140, 230, 255]);
        assert_eq!(a.len(), 32 * 32 * 4);
        let at = |v: &[u8], s: u32, x: u32, y: u32| v[((y * s + x) * 4 + 3) as usize];
        assert!(at(&a, 32, 16, 12) > 200, "the arrow's body");
        assert_eq!(at(&a, 32, 1, 1), 0, "clear around it");
        // A right turn reaches the right, a left turn the left.
        let r = turn_arrow(90.0, 40, [255; 4]);
        let l = turn_arrow(-90.0, 40, [255; 4]);
        assert!(at(&r, 40, 32, 22) > 0 && at(&r, 40, 8, 22) == 0);
        assert!(at(&l, 40, 8, 22) > 0 && at(&l, 40, 32, 22) == 0);
        let u = turn_arrow(180.0, 40, [255; 4]);
        assert!(u.as_chunks::<4>().0.iter().any(|p| p[3] > 0));
        let c = compass(24);
        let rgba = |v: &[u8], x: u32, y: u32| {
            let i = ((y * 24 + x) * 4) as usize;
            [v[i], v[i + 1], v[i + 2]]
        };
        assert!(
            rgba(&c, 12, 6)[0] > 200 && rgba(&c, 12, 6)[1] < 100,
            "north red"
        );
        assert!(rgba(&c, 12, 18)[1] > 200, "south light");
    }

    #[test]
    fn the_road_index_finds_the_roads_of_a_tile() {
        let src = MapSource::single(Arc::new(fixture::package()));
        assert_eq!(src.roads().len(), 3);
        let near = src.roads_in([90.0, 0.0], [110.0, 10.0]);
        assert_eq!(near.len(), 1);
        assert_eq!(near[0].name.as_deref(), Some("Kirchweg"));
        assert!(src.roads_in([500.0, 500.0], [600.0, 600.0]).is_empty());
        assert!(src.has_tile_at([150.0, 50.0]) && !src.has_tile_at([250.0, 50.0]));
        assert_eq!(raster::tile_keys(&src, &spec()).len(), 2);
    }
}
