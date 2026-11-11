//! Land use for world packages (spec §4.6 step 2): OSM areas classified
//! into [`LandClass`]es, rasterised per tile into the splat map (a class per
//! cell, highest priority wins), and water surfaces: a level at the terrain
//! vertices in water, with the terrain carved below it.

use std::collections::BTreeMap;

use ocw_world::landuse::{LandClass, LandUse, LandUseTile};
use ocw_world::package::{TileKey, WorldPackage};

use crate::areas::{AreaData, OsmArea};
use crate::osm::Tags;
use crate::projection::LocalProjection;

/// Default land-use cells per 1 km tile side (5 m cells).
pub const CELLS: u32 = 200;
/// Rivers at least this wide get a water surface; narrower water lines
/// (streams, ditches) only colour the ground.
pub const SURFACE_MIN_WIDTH_M: f64 = 8.0;
/// The water level at a vertex is the mean terrain height of the water
/// vertices within this many grid steps.
pub const LEVEL_RADIUS: i64 = 5;
/// The terrain under water lies this far below the water surface.
pub const WATER_DEPTH_M: f64 = 1.0;

/// An area in local metres with its class; even-odd fill over all rings
/// (holes are inner rings).
#[derive(Debug, Clone, PartialEq)]
pub struct Area {
    pub class: LandClass,
    pub rings: Vec<Vec<[f64; 2]>>,
    /// min x, min z, max x, max z.
    pub bbox: [f64; 4],
}

impl Area {
    pub fn new(class: LandClass, rings: Vec<Vec<[f64; 2]>>) -> Self {
        let mut bbox = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for p in rings.iter().flatten() {
            bbox = [
                bbox[0].min(p[0]),
                bbox[1].min(p[1]),
                bbox[2].max(p[0]),
                bbox[3].max(p[1]),
            ];
        }
        Self { class, rings, bbox }
    }
}

/// A river or stream: a centreline with a width.
#[derive(Debug, Clone, PartialEq)]
pub struct WaterLine {
    pub points: Vec<[f64; 2]>,
    pub width_m: f64,
}

/// Land class of an OSM area from its tags, `None` for areas that are not
/// land use (buildings, admin areas, …).
pub fn classify(tags: &Tags) -> Option<LandClass> {
    let t = |k: &str| tags.get(k).map(String::as_str);
    if t("natural") == Some("water")
        || matches!(t("waterway"), Some("riverbank" | "dock"))
        || matches!(t("landuse"), Some("reservoir" | "basin"))
        || t("leisure") == Some("swimming_area")
    {
        return Some(LandClass::Water);
    }
    if let Some(n) = t("natural") {
        let c = match n {
            "wood" => Some(LandClass::Forest),
            "grassland" | "heath" | "fell" => Some(LandClass::Meadow),
            "scrub" => Some(LandClass::Scrub),
            "wetland" => Some(LandClass::Wetland),
            "bare_rock" | "rock" => Some(LandClass::Rock),
            "scree" | "shingle" => Some(LandClass::Scree),
            "glacier" => Some(LandClass::Glacier),
            "sand" | "beach" | "mud" => Some(LandClass::Bare),
            _ => None,
        };
        if c.is_some() {
            return c;
        }
    }
    if let Some(l) = t("landuse") {
        let c = match l {
            "forest" => Some(LandClass::Forest),
            "meadow" | "grass" | "village_green" | "recreation_ground" => Some(LandClass::Meadow),
            "farmland" | "plant_nursery" | "greenhouse_horticulture" => Some(LandClass::Farmland),
            "orchard" => Some(LandClass::Orchard),
            "vineyard" => Some(LandClass::Vineyard),
            "residential" | "farmyard" => Some(LandClass::Residential),
            "commercial" | "retail" => Some(LandClass::Commercial),
            "industrial" | "railway" | "garages" | "military" => Some(LandClass::Industrial),
            "cemetery" | "allotments" => Some(LandClass::Park),
            "quarry" | "landfill" | "construction" | "brownfield" => Some(LandClass::Bare),
            _ => None,
        };
        if c.is_some() {
            return c;
        }
    }
    match t("leisure") {
        Some(
            "park" | "garden" | "golf_course" | "pitch" | "playground" | "recreation_ground"
            | "dog_park" | "miniature_golf",
        ) => Some(LandClass::Park),
        _ => None,
    }
}

/// Width of a waterway line in metres (`width` tag or a default by type);
/// `None` for lines that are not drawn (dams, weirs, underground).
pub fn waterway_width(tags: &Tags) -> Option<f64> {
    if tags.get("tunnel").is_some_and(|v| v != "no")
        || tags.get("location").is_some_and(|v| v == "underground")
    {
        return None;
    }
    let default = match tags.get("waterway")?.as_str() {
        "river" => 12.0,
        "canal" => 6.0,
        "stream" => 2.0,
        "ditch" | "drain" => 1.5,
        _ => return None,
    };
    Some(
        tags.get("width")
            .and_then(|w| crate::buildings::parse_length(w))
            .filter(|w| (0.3..=500.0).contains(w))
            .unwrap_or(default),
    )
}

/// The classified areas and water lines of an extract, in local metres.
pub fn extract(data: &AreaData, proj: &LocalProjection) -> (Vec<Area>, Vec<WaterLine>) {
    let project = |ring: &Vec<(f64, f64)>| -> Vec<[f64; 2]> {
        ring.iter()
            .map(|&(lat, lon)| proj.project(lat, lon))
            .collect()
    };
    let areas = data
        .areas()
        .into_iter()
        .filter_map(|a: OsmArea| {
            let class = classify(a.tags)?;
            let rings = a.outer.iter().chain(&a.inner).map(project).collect();
            Some(Area::new(class, rings))
        })
        .collect();
    let lines = data
        .waterways()
        .into_iter()
        .filter_map(|(w, pts)| {
            Some(WaterLine {
                width_m: waterway_width(&w.tags)?,
                points: pts
                    .iter()
                    .map(|&(lat, lon)| proj.project(lat, lon))
                    .collect(),
            })
        })
        .collect();
    (areas, lines)
}

/// A regular grid of sample points: `(i, k)` at `origin + (i, k)·spacing`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grid {
    pub origin: [f64; 2],
    pub spacing: f64,
    pub nx: usize,
    pub nz: usize,
}

impl Grid {
    fn bbox(&self) -> [f64; 4] {
        [
            self.origin[0],
            self.origin[1],
            self.origin[0] + (self.nx - 1) as f64 * self.spacing,
            self.origin[1] + (self.nz - 1) as f64 * self.spacing,
        ]
    }
}

fn overlaps(a: [f64; 4], b: [f64; 4], margin: f64) -> bool {
    a[0] <= b[2] + margin && b[0] <= a[2] + margin && a[1] <= b[3] + margin && b[1] <= a[3] + margin
}

fn set(out: &mut [LandClass], i: usize, c: LandClass) {
    if c.priority() > out[i].priority() {
        out[i] = c;
    }
}

/// Fills an area into the grid by scanlines (even-odd over all rings, a
/// sample on the left edge of a span is inside, on the right edge outside).
fn fill_area(out: &mut [LandClass], g: &Grid, area: &Area) {
    let mut rows: Vec<Vec<f64>> = vec![Vec::new(); g.nz];
    for ring in &area.rings {
        let n = ring.len();
        for j in 0..n {
            let (a, b) = (ring[j], ring[(j + 1) % n]);
            if a[1] == b[1] {
                continue;
            }
            let (lo, hi) = (a[1].min(b[1]), a[1].max(b[1]));
            let k0 = ((lo - g.origin[1]) / g.spacing).ceil().max(0.0) as usize;
            let k1 = ((hi - g.origin[1]) / g.spacing).ceil().min(g.nz as f64);
            if k1 <= 0.0 {
                continue;
            }
            for (k, row) in rows.iter_mut().enumerate().take(k1 as usize).skip(k0) {
                let z = g.origin[1] + k as f64 * g.spacing;
                let t = (z - a[1]) / (b[1] - a[1]);
                row.push(a[0] + t * (b[0] - a[0]));
            }
        }
    }
    for (k, row) in rows.iter_mut().enumerate() {
        row.sort_by(f64::total_cmp);
        for span in row.as_chunks::<2>().0 {
            let i0 = ((span[0] - g.origin[0]) / g.spacing).ceil().max(0.0);
            let i1 = ((span[1] - g.origin[0]) / g.spacing)
                .ceil()
                .min(g.nx as f64);
            for i in i0 as usize..i1.max(0.0) as usize {
                set(out, k * g.nx + i, area.class);
            }
        }
    }
}

/// Marks the samples within half the width of a water line (at least
/// `min_half` m) as water.
fn fill_line(out: &mut [LandClass], g: &Grid, line: &WaterLine, min_half: f64) {
    let half = (line.width_m / 2.0).max(min_half);
    for w in line.points.windows(2) {
        let (a, b) = (w[0], w[1]);
        let ab = [b[0] - a[0], b[1] - a[1]];
        let len2 = ab[0] * ab[0] + ab[1] * ab[1];
        let range = |lo: f64, hi: f64, o: f64, n: usize| {
            let i0 = ((lo - half - o) / g.spacing).ceil().max(0.0) as usize;
            let i1 = (((hi + half - o) / g.spacing).floor() + 1.0).clamp(0.0, n as f64) as usize;
            i0..i1
        };
        for k in range(a[1].min(b[1]), a[1].max(b[1]), g.origin[1], g.nz) {
            for i in range(a[0].min(b[0]), a[0].max(b[0]), g.origin[0], g.nx) {
                let p = [
                    g.origin[0] + i as f64 * g.spacing,
                    g.origin[1] + k as f64 * g.spacing,
                ];
                let t = if len2 > 0.0 {
                    (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / len2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let d2 = (p[0] - a[0] - ab[0] * t).powi(2) + (p[1] - a[1] - ab[1] * t).powi(2);
                if d2 <= half * half {
                    set(out, k * g.nx + i, LandClass::Water);
                }
            }
        }
    }
}

/// Classes at the grid samples: every area and water line, the highest
/// priority wins.
pub fn rasterise(g: &Grid, areas: &[&Area], lines: &[&WaterLine]) -> Vec<LandClass> {
    let mut out = vec![LandClass::Default; g.nx * g.nz];
    let bbox = g.bbox();
    for a in areas.iter().filter(|a| overlaps(a.bbox, bbox, 0.0)) {
        fill_area(&mut out, g, a);
    }
    for l in lines {
        // At least half a cell diagonal, so thin lines stay connected.
        fill_line(&mut out, g, l, g.spacing * 0.71);
    }
    out
}

/// One tile's land use and its carved heights (vertex index, height).
type TileResult = (TileKey, LandUseTile, Vec<(u32, f32)>);

/// What [`add_landuse`] did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LandUseStats {
    pub areas: usize,
    pub water_lines: usize,
    /// Cells per class over all tiles (indexed by class code).
    pub cells: Vec<usize>,
    /// Terrain vertices under water (carved) and with a water level.
    pub water_vertices: usize,
    pub level_vertices: usize,
}

/// Adds the land use (section `LUSE`) to a package and carves the terrain
/// under water surfaces. `cells` per tile side.
pub fn add_landuse(
    pkg: &mut WorldPackage,
    areas: &[Area],
    lines: &[WaterLine],
    cells: u32,
) -> LandUseStats {
    let size = f64::from(pkg.tile_size_m);
    let n = pkg.samples as usize;
    let sp = pkg.spacing_m();
    let margin = (LEVEL_RADIUS + 1) as usize;
    let ext = n + 2 * margin;
    let cell = size / f64::from(cells);
    let line_bbox: Vec<[f64; 4]> = lines
        .iter()
        .map(|l| {
            let a = Area::new(LandClass::Water, vec![l.points.clone()]);
            let h = l.width_m / 2.0 + sp;
            [a.bbox[0] - h, a.bbox[1] - h, a.bbox[2] + h, a.bbox[3] + h]
        })
        .collect();
    let keys: Vec<TileKey> = pkg.tiles.keys().copied().collect();
    let heights = |gi: i64, gk: i64| -> Option<f64> {
        let step = (n - 1) as i64;
        let key = TileKey {
            x: gi.div_euclid(step) as i32,
            z: gk.div_euclid(step) as i32,
        };
        let t = pkg.tiles.get(&key)?;
        let (i, k) = (gi.rem_euclid(step) as usize, gk.rem_euclid(step) as usize);
        Some(f64::from(t.heights[k * n + i]))
    };
    let tile_work = |key: TileKey| {
        let o = key.origin(size);
        let tb = [o[0], o[1], o[0] + size, o[1] + size];
        let m = margin as f64 * sp;
        let here: Vec<&Area> = areas.iter().filter(|a| overlaps(a.bbox, tb, m)).collect();
        let lines_here: Vec<&WaterLine> = lines
            .iter()
            .zip(&line_bbox)
            .filter(|(_, b)| overlaps(**b, tb, m))
            .map(|(l, _)| l)
            .collect();
        let classes = rasterise(
            &Grid {
                origin: [o[0] + cell / 2.0, o[1] + cell / 2.0],
                spacing: cell,
                nx: cells as usize,
                nz: cells as usize,
            },
            &here,
            &lines_here,
        );
        // Water surfaces: water polygons and wide rivers on the vertex grid
        // with a margin (levels are means over a neighbourhood).
        let water_areas: Vec<&Area> = here
            .iter()
            .filter(|a| a.class == LandClass::Water)
            .copied()
            .collect();
        let wide: Vec<&WaterLine> = lines_here
            .iter()
            .filter(|l| l.width_m >= SURFACE_MIN_WIDTH_M)
            .copied()
            .collect();
        let mut water = Vec::new();
        let mut carved = Vec::new();
        if !water_areas.is_empty() || !wide.is_empty() {
            let g = Grid {
                origin: [o[0] - m, o[1] - m],
                spacing: sp,
                nx: ext,
                nz: ext,
            };
            let mut mask = vec![LandClass::Default; ext * ext];
            for a in &water_areas {
                fill_area(&mut mask, &g, a);
            }
            for l in &wide {
                fill_line(&mut mask, &g, l, 0.0);
            }
            let (gi0, gk0) = (
                i64::from(key.x) * (n as i64 - 1) - margin as i64,
                i64::from(key.z) * (n as i64 - 1) - margin as i64,
            );
            let h: Vec<Option<f64>> = (0..ext * ext)
                .map(|j| heights(gi0 + (j % ext) as i64, gk0 + (j / ext) as i64))
                .collect();
            let is_water = |i: usize, k: usize| {
                mask[k * ext + i] == LandClass::Water && h[k * ext + i].is_some()
            };
            let r = LEVEL_RADIUS;
            let level = |i: usize, k: usize| -> Option<f64> {
                if !is_water(i, k) {
                    return None;
                }
                let (mut sum, mut count) = (0.0, 0);
                for dk in -r..=r {
                    for di in -r..=r {
                        if di * di + dk * dk > r * r {
                            continue;
                        }
                        let (ii, kk) = (i as i64 + di, k as i64 + dk);
                        if ii < 0 || kk < 0 || ii >= ext as i64 || kk >= ext as i64 {
                            continue;
                        }
                        let (ii, kk) = (ii as usize, kk as usize);
                        if is_water(ii, kk) {
                            sum += h[kk * ext + ii].expect("water has a height");
                            count += 1;
                        }
                    }
                }
                Some(sum / f64::from(count))
            };
            for k in 0..n {
                for i in 0..n {
                    let (ei, ek) = (i + margin, k + margin);
                    let idx = (k * n + i) as u32;
                    if let Some(l) = level(ei, ek) {
                        water.push((idx, l as f32));
                        let hv = h[ek * ext + ei].expect("water has a height");
                        carved.push((idx, hv.min(l - WATER_DEPTH_M) as f32));
                        continue;
                    }
                    let near: Vec<f64> = [
                        (-1, -1),
                        (0, -1),
                        (1, -1),
                        (-1, 0),
                        (1, 0),
                        (-1, 1),
                        (0, 1),
                        (1, 1),
                    ]
                    .iter()
                    .filter_map(|&(di, dk): &(i64, i64)| {
                        level((ei as i64 + di) as usize, (ek as i64 + dk) as usize)
                    })
                    .collect();
                    if !near.is_empty() {
                        water.push((idx, (near.iter().sum::<f64>() / near.len() as f64) as f32));
                    }
                }
            }
        }
        (key, LandUseTile { classes, water }, carved)
    };
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let chunk = keys.len().div_ceil(threads).max(1);
    let results: Vec<TileResult> = std::thread::scope(|s| {
        let handles: Vec<_> = keys
            .chunks(chunk)
            .map(|part| {
                let work = &tile_work;
                s.spawn(move || part.iter().map(|&k| work(k)).collect::<Vec<_>>())
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("land-use thread"))
            .collect()
    });
    let mut stats = LandUseStats {
        areas: areas.len(),
        water_lines: lines.len(),
        cells: vec![0; LandClass::ALL.len()],
        ..Default::default()
    };
    let mut tiles = BTreeMap::new();
    for (key, lt, carved) in results {
        stats.water_vertices += carved.len();
        stats.level_vertices += lt.water.len();
        for c in &lt.classes {
            stats.cells[usize::from(c.code())] += 1;
        }
        let t = pkg.tiles.get_mut(&key).expect("tile exists");
        for (i, h) in carved {
            t.heights[i as usize] = h;
        }
        tiles.insert(key, lt);
    }
    pkg.landuse = Some(LandUse { cells, tiles });
    stats
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_world::package::Tile;

    fn tags(pairs: &[(&str, &str)]) -> Tags {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn square(x0: f64, z0: f64, x1: f64, z1: f64) -> Vec<[f64; 2]> {
        vec![[x0, z0], [x1, z0], [x1, z1], [x0, z1]]
    }

    #[test]
    fn osm_tags_are_classified() {
        for (t, want) in [
            (vec![("landuse", "forest")], Some(LandClass::Forest)),
            (vec![("natural", "wood")], Some(LandClass::Forest)),
            (vec![("landuse", "meadow")], Some(LandClass::Meadow)),
            (vec![("natural", "grassland")], Some(LandClass::Meadow)),
            (vec![("landuse", "farmland")], Some(LandClass::Farmland)),
            (vec![("landuse", "orchard")], Some(LandClass::Orchard)),
            (vec![("landuse", "vineyard")], Some(LandClass::Vineyard)),
            (
                vec![("landuse", "residential")],
                Some(LandClass::Residential),
            ),
            (vec![("landuse", "retail")], Some(LandClass::Commercial)),
            (vec![("landuse", "industrial")], Some(LandClass::Industrial)),
            (
                vec![("natural", "water"), ("water", "lake")],
                Some(LandClass::Water),
            ),
            (vec![("waterway", "riverbank")], Some(LandClass::Water)),
            (vec![("landuse", "reservoir")], Some(LandClass::Water)),
            (vec![("natural", "bare_rock")], Some(LandClass::Rock)),
            (vec![("natural", "scree")], Some(LandClass::Scree)),
            (vec![("natural", "glacier")], Some(LandClass::Glacier)),
            (vec![("natural", "wetland")], Some(LandClass::Wetland)),
            (vec![("leisure", "park")], Some(LandClass::Park)),
            (vec![("leisure", "pitch")], Some(LandClass::Park)),
            (vec![("natural", "scrub")], Some(LandClass::Scrub)),
            (vec![("leisure", "nature_reserve")], None),
            (vec![("building", "yes")], None),
            (vec![("natural", "tree_row")], None),
        ] {
            assert_eq!(classify(&tags(&t)), want, "{t:?}");
        }
        assert_eq!(waterway_width(&tags(&[("waterway", "river")])), Some(12.0));
        assert_eq!(
            waterway_width(&tags(&[("waterway", "river"), ("width", "40 m")])),
            Some(40.0)
        );
        assert_eq!(
            waterway_width(&tags(&[("waterway", "stream"), ("tunnel", "culvert")])),
            None
        );
        assert_eq!(waterway_width(&tags(&[("waterway", "dam")])), None);
    }

    fn grid10() -> Grid {
        // 10 × 10 cells of 10 m, sampled at the cell centres.
        Grid {
            origin: [5.0, 5.0],
            spacing: 10.0,
            nx: 10,
            nz: 10,
        }
    }

    #[test]
    fn polygons_are_rasterised_with_holes() {
        let g = grid10();
        // Forest 0..60 × 0..60 with a hole 20..40 × 20..40.
        let forest = Area::new(
            LandClass::Forest,
            vec![square(0.0, 0.0, 60.0, 60.0), square(20.0, 20.0, 40.0, 40.0)],
        );
        let out = rasterise(&g, &[&forest], &[]);
        let at = |i: usize, k: usize| out[k * 10 + i];
        assert_eq!(at(0, 0), LandClass::Forest);
        assert_eq!(at(5, 5), LandClass::Forest);
        assert_eq!(at(6, 0), LandClass::Default);
        assert_eq!(at(2, 2), LandClass::Default); // the hole
        assert_eq!(at(3, 3), LandClass::Default);
        assert_eq!(at(1, 2), LandClass::Forest);
        let count = out.iter().filter(|&&c| c == LandClass::Forest).count();
        assert_eq!(count, 36 - 4);
        // A triangle: cells whose centre lies inside (x + z < 50).
        let tri = Area::new(
            LandClass::Meadow,
            vec![vec![[0.0, 0.0], [50.0, 0.0], [0.0, 50.0]]],
        );
        let out = rasterise(&g, &[&tri], &[]);
        for k in 0..10 {
            for i in 0..10 {
                let (x, z) = (5.0 + 10.0 * i as f64, 5.0 + 10.0 * k as f64);
                assert_eq!(
                    out[k * 10 + i] == LandClass::Meadow,
                    x + z < 50.0,
                    "({i}, {k})"
                );
            }
        }
    }

    #[test]
    fn priorities_decide_overlaps_regardless_of_order() {
        let g = grid10();
        let residential = Area::new(LandClass::Residential, vec![square(0.0, 0.0, 100.0, 100.0)]);
        let park = Area::new(LandClass::Park, vec![square(0.0, 0.0, 30.0, 30.0)]);
        let lake = Area::new(LandClass::Water, vec![square(20.0, 20.0, 50.0, 50.0)]);
        for order in [
            vec![&residential, &park, &lake],
            vec![&lake, &park, &residential],
        ] {
            let out = rasterise(&g, &order, &[]);
            assert_eq!(out[0], LandClass::Park);
            assert_eq!(out[2 * 10 + 2], LandClass::Water); // in both park and lake
            assert_eq!(out[4 * 10 + 4], LandClass::Water);
            assert_eq!(out[9 * 10 + 9], LandClass::Residential);
        }
        // A river line across the residential area.
        let river = WaterLine {
            points: vec![[0.0, 75.0], [100.0, 75.0]],
            width_m: 12.0,
        };
        let out = rasterise(&g, &[&residential], &[&river]);
        assert_eq!(out[7 * 10 + 3], LandClass::Water); // z 75
        assert_eq!(out[6 * 10 + 3], LandClass::Residential); // z 65: 10 m away
        // Thin streams still cover the cells they pass through.
        let stream = WaterLine {
            points: vec![[0.0, 52.0], [100.0, 52.0]],
            width_m: 1.0,
        };
        let out = rasterise(&g, &[], &[&stream]);
        assert!((0..10).all(|i| out[5 * 10 + i] == LandClass::Water));
    }

    #[test]
    fn grids_agree_across_tile_edges() {
        // The same polygon over two adjacent grids gives the same classes
        // at the shared samples (global positions, not tile-relative).
        let a = Area::new(
            LandClass::Forest,
            vec![vec![[13.0, 7.0], [187.0, 33.0], [91.0, 171.0]]],
        );
        let left = Grid {
            origin: [0.0, 0.0],
            spacing: 10.0,
            nx: 11,
            nz: 21,
        };
        let right = Grid {
            origin: [100.0, 0.0],
            spacing: 10.0,
            nx: 11,
            nz: 21,
        };
        let (l, r) = (rasterise(&left, &[&a], &[]), rasterise(&right, &[&a], &[]));
        for k in 0..21 {
            assert_eq!(l[k * 11 + 10], r[k * 11], "row {k}");
        }
    }

    /// Two 100 m tiles (11 × 11 vertices, 10 m) on a slope, a lake
    /// across their shared edge.
    fn lake_package() -> WorldPackage {
        let n = 11;
        let mut tiles = BTreeMap::new();
        for x in 0..2 {
            let key = TileKey { x, z: 0 };
            let heights = (0..n * n)
                .map(|j| {
                    let gi = x as usize * 10 + j % n;
                    (500.0 + gi as f64 * 0.5 + if (j / n) % 2 == 0 { 0.4 } else { -0.4 }) as f32
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
        WorldPackage {
            name: "lake".into(),
            origin: (47.0, 9.0),
            tile_size_m: 100.0,
            samples: n as u32,
            tiles,
            graph: None,
            landuse: None,
            buildings: None,
        }
    }

    #[test]
    fn water_gets_a_level_and_the_terrain_is_carved_below_it() {
        let mut p = lake_package();
        let before = p.clone();
        let lake = Area::new(LandClass::Water, vec![square(62.0, 22.0, 138.0, 78.0)]);
        let forest = Area::new(LandClass::Forest, vec![square(0.0, 0.0, 40.0, 100.0)]);
        let stats = add_landuse(&mut p, &[lake, forest], &[], 20);
        assert!(stats.water_vertices > 0 && stats.level_vertices > stats.water_vertices);
        let l = p.landuse.as_ref().unwrap();
        assert_eq!(l.cells, 20);
        assert_eq!(l.class_at(100.0, 10.0, 50.0), Some(LandClass::Forest));
        assert_eq!(l.class_at(100.0, 70.0, 50.0), Some(LandClass::Water));
        assert_eq!(l.class_at(100.0, 120.0, 50.0), Some(LandClass::Water));
        assert_eq!(l.class_at(100.0, 190.0, 90.0), Some(LandClass::Default));
        let (a, b) = (
            &l.tiles[&TileKey { x: 0, z: 0 }],
            &l.tiles[&TileKey { x: 1, z: 0 }],
        );
        let (ta, tb) = (
            &p.tiles[&TileKey { x: 0, z: 0 }],
            &p.tiles[&TileKey { x: 1, z: 0 }],
        );
        for k in 0..11u32 {
            // Shared edge (x = 100): same level and same carved height in both tiles.
            assert_eq!(a.water_level(k * 11 + 10), b.water_level(k * 11), "row {k}");
            assert_eq!(
                ta.heights[k as usize * 11 + 10],
                tb.heights[k as usize * 11]
            );
        }
        // Inside the lake: water vertex at (100, 50), carved ≥ 1 m below the level,
        // the level close to the mean ground height there (≈ 525 m), not tilted much.
        let lv = a.water_level(5 * 11 + 10).unwrap();
        assert!((f64::from(lv) - 505.0).abs() < 1.0, "{lv}");
        assert!(f64::from(ta.heights[5 * 11 + 10]) <= f64::from(lv) - WATER_DEPTH_M + 1e-4);
        // A neighbour of the lake (x 60) has a level but keeps its height.
        assert!(a.water_level(5 * 11 + 6).is_some());
        assert_eq!(
            ta.heights[5 * 11 + 6],
            before.tiles[&TileKey { x: 0, z: 0 }].heights[5 * 11 + 6]
        );
        // Far from water: no level, unchanged.
        assert_eq!(a.water_level(5 * 11), None);
        // Every grid cell with a water vertex corner has levels at all
        // four corners (the renderer draws the surface over those cells).
        for (x, t) in [(0u32, a), (1, b)] {
            for k in 0..10u32 {
                for i in 0..10u32 {
                    let c = [
                        k * 11 + i,
                        k * 11 + i + 1,
                        (k + 1) * 11 + i,
                        (k + 1) * 11 + i + 1,
                    ];
                    let wet = c.iter().any(|&v| {
                        let (gx, gz) = (f64::from(x * 100 + (v % 11) * 10), f64::from(v / 11 * 10));
                        (62.0..138.0).contains(&gx) && (22.0..78.0).contains(&gz)
                    });
                    if wet {
                        assert!(
                            c.iter().all(|&v| t.water_level(v).is_some()),
                            "cell {i} {k}"
                        );
                    }
                }
            }
        }
        // Survives the package round trip.
        assert_eq!(WorldPackage::from_bytes(&p.to_bytes()).unwrap(), p);
    }
}
