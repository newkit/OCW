//! What the studio's 2D map draws of a package (spec §14 Phase 16, 16A):
//! the land use as a colour raster (one texel per land-use cell, coarser
//! for large packages), the buildings as triangles, the roads of the road
//! graph as polylines, each with its bounding box for culling. Built once
//! per package.

use ocw_world::graph::RoadClass;
use ocw_world::landuse::LandClass;
use ocw_world::package::{TileKey, WorldPackage};

/// The land-use raster is at most this many texels per side (egui's
/// largest texture side).
pub const MAX_RASTER: usize = 2048;

/// The land use as colours, row-major along +z.
#[derive(Debug, Clone, PartialEq)]
pub struct LandRaster {
    /// The package point of texel (0, 0)'s corner.
    pub origin: [f64; 2],
    pub m_per_texel: f64,
    pub width: usize,
    pub height: usize,
    /// RGBA, unmultiplied; transparent outside the package's tiles.
    pub rgba: Vec<u8>,
}

/// A road of the map.
#[derive(Debug, Clone, PartialEq)]
pub struct MapRoad {
    pub class: RoadClass,
    pub width_m: f32,
    pub points: Vec<[f32; 2]>,
    pub min: [f32; 2],
    pub max: [f32; 2],
}

/// A building of the map: its footprint as triangles.
#[derive(Debug, Clone, PartialEq)]
pub struct MapBuilding {
    pub points: Vec<[f32; 2]>,
    pub triangles: Vec<[u32; 3]>,
    pub min: [f32; 2],
    pub max: [f32; 2],
}

/// What the map draws of a package.
#[derive(Debug, Clone, PartialEq)]
pub struct MapData {
    /// The tiles' extent (x, z).
    pub min: [f64; 2],
    pub max: [f64; 2],
    pub land: Option<LandRaster>,
    pub roads: Vec<MapRoad>,
    pub buildings: Vec<MapBuilding>,
    /// The package's places, most important first (section `PLCE`; empty
    /// for packages without it).
    pub places: Vec<ocw_world::places::Place>,
    /// The package's peaks and passes (section `LMRK`, spec 3.2 Q8; no
    /// viewpoints), peaks first, higher first.
    pub landmarks: Vec<ocw_world::landmarks::Landmark>,
}

/// The box of `points`: (min, max).
pub fn bbox(points: impl Iterator<Item = [f32; 2]>) -> ([f32; 2], [f32; 2]) {
    points.fold(
        ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]),
        |(lo, hi), p| {
            (
                [lo[0].min(p[0]), lo[1].min(p[1])],
                [hi[0].max(p[0]), hi[1].max(p[1])],
            )
        },
    )
}

/// The tiles' extent of `pkg` (x, z): (min, max); zero without tiles.
pub fn tile_extent(pkg: &WorldPackage) -> ([f64; 2], [f64; 2]) {
    let size = f64::from(pkg.tile_size_m);
    let keys: Vec<TileKey> = pkg.tiles.keys().copied().collect();
    if keys.is_empty() {
        return ([0.0; 2], [0.0; 2]);
    }
    let kx0 = keys.iter().map(|k| k.x).min().unwrap_or(0);
    let kx1 = keys.iter().map(|k| k.x).max().unwrap_or(0);
    let kz0 = keys.iter().map(|k| k.z).min().unwrap_or(0);
    let kz1 = keys.iter().map(|k| k.z).max().unwrap_or(0);
    (
        [f64::from(kx0) * size, f64::from(kz0) * size],
        [f64::from(kx1 + 1) * size, f64::from(kz1 + 1) * size],
    )
}

impl MapData {
    /// The map of `pkg` (its own tiles; OCW Studio draws the neighbours
    /// beside it, `apps/studio/src/surround.rs`),
    /// the land use coloured by `land_rgb`.
    pub fn of(pkg: &WorldPackage, land_rgb: impl Fn(LandClass) -> [u8; 3]) -> Self {
        let (min, max) = tile_extent(pkg);
        let land = (!pkg.tiles.is_empty()).then(|| land_raster(pkg, min, max, &land_rgb));
        let roads = match &pkg.graph {
            Some(g) => g
                .edges
                .iter()
                .filter(|e| e.points.len() >= 2)
                .map(|e| {
                    let points: Vec<[f32; 2]> = e
                        .points
                        .iter()
                        .map(|p| [p[0] as f32, p[1] as f32])
                        .collect();
                    let (min, max) = bbox(points.iter().copied());
                    MapRoad {
                        class: e.class,
                        width_m: e.width_m as f32,
                        points,
                        min,
                        max,
                    }
                })
                .collect(),
            // Version 1 packages: the tiles' road strips.
            None => pkg
                .tiles
                .values()
                .flat_map(|t| &t.roads)
                .filter(|s| s.points.len() >= 2)
                .map(|s| {
                    let points: Vec<[f32; 2]> = s.points.iter().map(|p| [p[0], p[2]]).collect();
                    let (min, max) = bbox(points.iter().copied());
                    MapRoad {
                        class: s.class,
                        width_m: s.width_m,
                        points,
                        min,
                        max,
                    }
                })
                .collect(),
        };
        let buildings = pkg
            .buildings
            .iter()
            .flat_map(|b| b.values())
            .flatten()
            .filter(|b| b.outline.len() >= 3)
            .map(|b| {
                let mut points = b.outline.clone();
                for h in &b.holes {
                    points.extend_from_slice(h);
                }
                let triangles = ocw_world::buildings::triangulate(&b.outline, &b.holes);
                let (min, max) = bbox(b.outline.iter().copied());
                MapBuilding {
                    points,
                    triangles,
                    min,
                    max,
                }
            })
            .collect();
        Self {
            min,
            max,
            land,
            roads,
            buildings,
            places: pkg
                .places
                .as_ref()
                .map(|p| p.places.clone())
                .unwrap_or_default(),
            landmarks: pkg
                .places
                .as_ref()
                .map(|p| {
                    p.landmarks
                        .iter()
                        .filter(|l| {
                            l.kind != ocw_world::landmarks::LandmarkKind::Viewpoint
                                && (!l.name.is_empty() || l.ele.is_some())
                        })
                        .cloned()
                        .collect()
                })
                .unwrap_or_default(),
        }
    }
}

/// The land-use raster over the tile range `min`…`max`: one texel per
/// land-use cell (every `stride`-th for large packages), tiles without
/// land use in the default ground colour, no tile transparent.
fn land_raster(
    pkg: &WorldPackage,
    min: [f64; 2],
    max: [f64; 2],
    land_rgb: &dyn Fn(LandClass) -> [u8; 3],
) -> LandRaster {
    let size = f64::from(pkg.tile_size_m);
    let cells = pkg.landuse.as_ref().map_or(1, |l| l.cells.max(1)) as usize;
    let tiles_x = ((max[0] - min[0]) / size).round() as usize;
    let tiles_z = ((max[1] - min[1]) / size).round() as usize;
    let total = tiles_x.max(tiles_z) * cells;
    let stride = total.div_ceil(MAX_RASTER).max(1);
    let width = (tiles_x * cells).div_ceil(stride).max(1);
    let height = (tiles_z * cells).div_ceil(stride).max(1);
    let k0 = TileKey::containing([min[0] + 0.5, min[1] + 0.5], size);
    let mut rgba = vec![0u8; width * height * 4];
    let default = land_rgb(LandClass::Default);
    for v in 0..height {
        for u in 0..width {
            let (gx, gz) = (u * stride, v * stride);
            let key = TileKey {
                x: k0.x + (gx / cells) as i32,
                z: k0.z + (gz / cells) as i32,
            };
            if !pkg.tiles.contains_key(&key) {
                continue;
            }
            let class = pkg
                .landuse
                .as_ref()
                .and_then(|l| l.tiles.get(&key))
                .and_then(|t| t.classes.get((gz % cells) * cells + gx % cells))
                .copied();
            let rgb = class.map_or(default, land_rgb);
            let i = (v * width + u) * 4;
            rgba[i..i + 4].copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
        }
    }
    LandRaster {
        origin: min,
        m_per_texel: size / cells as f64 * stride as f64,
        width,
        height,
        rgba,
    }
}

/// Whether the box `min`…`max` overlaps the area `vis` (min, max).
pub fn overlaps(min: [f32; 2], max: [f32; 2], vis: &([f64; 2], [f64; 2])) -> bool {
    f64::from(max[0]) >= vis.0[0]
        && f64::from(min[0]) <= vis.1[0]
        && f64::from(max[1]) >= vis.0[1]
        && f64::from(min[1]) <= vis.1[1]
}
