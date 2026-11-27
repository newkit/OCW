//! The ground away from the roads (spec §14 Phase 13, 13A *Free 2D riding
//! off the road*, v5.52): the terrain's height and what covers it, for a
//! rider who landed from the unicorn in a meadow and rides on the terrain.
//! Shared (in an `Arc`, through the road network) by the game server's
//! physics and the client's prediction, so both ride the same slopes; the
//! renderer draws the same heights (the tile grid triangulated like the
//! terrain mesh, [`crate::WorldPackage::surface_height`]).
//!
//! **Per tile** the heights (the package's grid, `samples²`, stored as
//! 16-bit steps between the tile's lowest and highest point: at most 2 cm
//! off for a tile spanning 1,300 m, typically a few millimetres) and the
//! **cover** at the land use's cells (5 m for 1 km tiles; run-length coded
//! per row, a tile of one class keeps one run a row): [`Cover`] from the
//! `LUSE` classes, buildings rasterised on top (a cell whose centre or a
//! corner of a footprint lies in it). Tiles without land use are grass.
//! Black Forest south's 2,800 tiles take ~60 MB (mostly heights); the
//! browser app adds tiles as they are streamed in ([`GroundMap::add_tile`]).
//!
//! **The off-road track** ([`OffRoadTrack`]): the physics rides a [`Track`]
//! everywhere, so a rider off the road rides a short straight track along
//! their heading through where they are, its heights and gradient from the
//! ground, its surface from the cover. The physics re-anchors it every tick
//! (`ocw-sim::offroad`); the renderer, the HUD's gradient, the trainer, the
//! GPS track and the FIT recording read it like any road.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use crate::buildings::Building;
use crate::landuse::{LandClass, LandUseTile};
use crate::package::{Tile, TileKey, WorldPackage};
use crate::route::RoutePoint;
use crate::surface::Surface;
use crate::track::Track;

/// Half the base of the gradient's central difference along the heading (m).
pub const SLOPE_HALF_BASE_M: f64 = 2.0;
/// Gradients beyond this (rise over run) are taken as this: a cliff in the
/// 10 m grid is at most a few hundred percent, so this only keeps the
/// numbers finite (spec Phase 13 Q7 (b): every slope at its real gradient).
pub const MAX_GRADE: f64 = 20.0;
/// Width the off-road track reports (m): the rider is always on its
/// "road" (the surface is the cover's), and nothing beside it.
pub const OFF_ROAD_WIDTH_M: f64 = 2.0;

/// What covers the ground in a cell (off the roads).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
#[repr(u8)]
pub enum Cover {
    /// Meadows, lawns, parks, fields, orchards, vineyards, gardens in towns.
    #[default]
    Grass = 0,
    /// Woods and scrub: the forest floor.
    Forest = 1,
    /// Bare rock.
    Rock = 2,
    /// Sand, scree, bare soil, wetland, glacier snow: soft ground.
    Sand = 3,
    /// Water: impassable, no landing.
    Water = 4,
    /// A building's footprint: no landing (its walls are obstacles).
    Building = 5,
}

impl Cover {
    pub const ALL: [Cover; 6] = [
        Cover::Grass,
        Cover::Forest,
        Cover::Rock,
        Cover::Sand,
        Cover::Water,
        Cover::Building,
    ];

    /// The cover of a land-use class.
    pub fn of_class(c: LandClass) -> Self {
        match c {
            LandClass::Forest | LandClass::Scrub => Cover::Forest,
            LandClass::Rock => Cover::Rock,
            LandClass::Scree | LandClass::Bare | LandClass::Wetland | LandClass::Glacier => {
                Cover::Sand
            }
            LandClass::Water => Cover::Water,
            LandClass::Default
            | LandClass::Meadow
            | LandClass::Farmland
            | LandClass::Orchard
            | LandClass::Vineyard
            | LandClass::Residential
            | LandClass::Commercial
            | LandClass::Industrial
            | LandClass::Park => Cover::Grass,
        }
    }

    /// The riding surface (its rolling resistance, spec 5.2.1).
    pub fn surface(self) -> Surface {
        match self {
            Cover::Grass | Cover::Building | Cover::Water => Surface::Grass,
            Cover::Forest => Surface::ForestFloor,
            Cover::Rock => Surface::Rock,
            Cover::Sand => Surface::Sand,
        }
    }

    /// Whether a rider may ride and land here.
    pub fn rideable(self) -> bool {
        !matches!(self, Cover::Water | Cover::Building)
    }

    pub fn name(self) -> &'static str {
        match self {
            Cover::Grass => "meadow",
            Cover::Forest => "forest floor",
            Cover::Rock => "rock",
            Cover::Sand => "soft ground",
            Cover::Water => "water",
            Cover::Building => "building",
        }
    }

    fn code(self) -> u8 {
        self as u8
    }

    fn from_code(c: u8) -> Self {
        Self::ALL.get(usize::from(c)).copied().unwrap_or_default()
    }
}

/// One tile of the ground.
#[derive(Debug, Clone, PartialEq)]
struct GroundTile {
    /// Height of step 0 and the metres per step.
    base: f32,
    step: f32,
    heights: Vec<u16>,
    /// Cells per side of the cover and, per row, the index of its first
    /// run in `runs` (row `r`'s runs are `row_start[r] .. row_start[r+1]`).
    cells: u32,
    row_start: Vec<u32>,
    /// (last column of the run, cover code).
    runs: Vec<(u16, u8)>,
    /// Water surface levels at the terrain vertices in and around water
    /// (the land use's, sorted by vertex index).
    water: Vec<(u32, f32)>,
}

impl GroundTile {
    fn of(
        tile: &Tile,
        samples: u32,
        tile_size: f64,
        landuse: Option<(&LandUseTile, u32)>,
        buildings: &[&Building],
    ) -> Option<Self> {
        let n = samples as usize;
        if n < 2 || tile.heights.len() != n * n {
            return None;
        }
        let (lo, hi) = tile
            .heights
            .iter()
            .filter(|h| h.is_finite())
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), &h| {
                (lo.min(h), hi.max(h))
            });
        if !lo.is_finite() {
            return None;
        }
        let step = ((hi - lo) / f32::from(u16::MAX)).max(1e-4);
        let heights = tile
            .heights
            .iter()
            .map(|&h| {
                let h = if h.is_finite() { h } else { lo };
                ((h - lo) / step).round().clamp(0.0, f32::from(u16::MAX)) as u16
            })
            .collect();
        // The cover: the land use's classes (grass without), then the
        // buildings on top.
        let cells = landuse.map_or(100, |(_, c)| c.max(1));
        let c = cells as usize;
        let mut grid = vec![Cover::Grass.code(); c * c];
        if let Some((lu, _)) = landuse
            && lu.classes.len() == c * c
        {
            for (g, class) in grid.iter_mut().zip(&lu.classes) {
                *g = Cover::of_class(*class).code();
            }
        }
        let o = tile.key.origin(tile_size);
        let s = tile_size / cells as f64;
        let cell_of = |p: [f64; 2]| -> Option<usize> {
            let (cx, cz) = ((p[0] - o[0]) / s, (p[1] - o[1]) / s);
            (cx >= 0.0 && cz >= 0.0 && cx < cells as f64 && cz < cells as f64)
                .then(|| cz as usize * c + cx as usize)
        };
        for b in buildings.iter().copied() {
            if b.outline.len() < 3 {
                continue;
            }
            let (mut blo, mut bhi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
            for p in &b.outline {
                for k in 0..2 {
                    blo[k] = blo[k].min(f64::from(p[k]));
                    bhi[k] = bhi[k].max(f64::from(p[k]));
                }
                if let Some(i) = cell_of([f64::from(p[0]), f64::from(p[1])]) {
                    grid[i] = Cover::Building.code();
                }
            }
            let (c0, c1) = (
                (((blo[0] - o[0]) / s).floor().max(0.0) as usize).min(c - 1),
                (((bhi[0] - o[0]) / s).ceil().max(0.0) as usize).min(c - 1),
            );
            let (r0, r1) = (
                (((blo[1] - o[1]) / s).floor().max(0.0) as usize).min(c - 1),
                (((bhi[1] - o[1]) / s).ceil().max(0.0) as usize).min(c - 1),
            );
            for r in r0..=r1 {
                for col in c0..=c1 {
                    let centre = [
                        (o[0] + (col as f64 + 0.5) * s) as f32,
                        (o[1] + (r as f64 + 0.5) * s) as f32,
                    ];
                    let inside = crate::ortho::point_in_ring(centre, &b.outline)
                        && !b
                            .holes
                            .iter()
                            .any(|h| crate::ortho::point_in_ring(centre, h));
                    if inside {
                        grid[r * c + col] = Cover::Building.code();
                    }
                }
            }
        }
        let mut row_start = Vec::with_capacity(c + 1);
        let mut runs = Vec::new();
        for r in 0..c {
            row_start.push(runs.len() as u32);
            let row = &grid[r * c..(r + 1) * c];
            let mut col = 0;
            while col < c {
                let code = row[col];
                let mut end = col;
                while end + 1 < c && row[end + 1] == code {
                    end += 1;
                }
                runs.push((end as u16, code));
                col = end + 1;
            }
        }
        row_start.push(runs.len() as u32);
        Some(Self {
            base: lo,
            step,
            heights,
            cells,
            row_start,
            runs,
            water: landuse.map_or_else(Vec::new, |(lu, _)| lu.water.clone()),
        })
    }

    fn h(&self, n: usize, i: usize, k: usize) -> f64 {
        f64::from(self.base) + f64::from(self.step) * f64::from(self.heights[k * n + i])
    }

    fn cover(&self, col: usize, row: usize) -> Cover {
        let (a, b) = (
            self.row_start[row] as usize,
            self.row_start[row + 1] as usize,
        );
        let runs = &self.runs[a..b];
        let i = runs.partition_point(|r| usize::from(r.0) < col);
        Cover::from_code(runs.get(i).map_or(0, |r| r.1))
    }

    fn bytes(&self) -> usize {
        self.heights.len() * 2
            + self.runs.len() * 4
            + self.row_start.len() * 4
            + self.water.len() * 8
            + 48
    }
}

/// The ground of a world: heights and cover per tile (see the module).
pub struct GroundMap {
    tile_size_m: f64,
    samples: u32,
    tiles: RwLock<BTreeMap<TileKey, Arc<GroundTile>>>,
}

impl std::fmt::Debug for GroundMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GroundMap")
            .field("tile_size_m", &self.tile_size_m)
            .field("tiles", &self.tile_count())
            .finish()
    }
}

impl GroundMap {
    /// An empty map for tiles of `tile_size_m` with `samples` heights a side.
    pub fn new(tile_size_m: f32, samples: u32) -> Self {
        Self {
            tile_size_m: f64::from(tile_size_m),
            samples,
            tiles: RwLock::new(BTreeMap::new()),
        }
    }

    /// The ground of a package's tiles (those it holds: a streamed base
    /// has none yet).
    pub fn from_package(pkg: &WorldPackage) -> Self {
        let map = Self::new(pkg.tile_size_m, pkg.samples);
        map.add_package_tiles(pkg);
        map
    }

    /// Adds the tiles of `pkg` the map does not have yet (the browser's
    /// streamed versions: their land use and buildings come with them).
    pub fn add_package_tiles(&self, pkg: &WorldPackage) {
        if pkg.samples != self.samples || f64::from(pkg.tile_size_m) != self.tile_size_m {
            return;
        }
        for (key, tile) in &pkg.tiles {
            if self.has_tile(*key) {
                continue;
            }
            let lu = pkg
                .landuse
                .as_ref()
                .and_then(|l| l.tiles.get(key).map(|t| (t, l.cells)));
            // Buildings belong to one tile but may reach into the next.
            let b: Vec<&Building> = pkg.buildings.as_ref().map_or_else(Vec::new, |b| {
                (-1..=1)
                    .flat_map(|dx| (-1..=1).map(move |dz| (dx, dz)))
                    .filter_map(|(dx, dz)| {
                        b.get(&TileKey {
                            x: key.x + dx,
                            z: key.z + dz,
                        })
                    })
                    .flatten()
                    .collect()
            });
            self.add_tile(tile, lu, &b);
        }
    }

    /// Adds (or replaces) one tile with its land use (`(tile, cells per
    /// side)`) and the buildings standing on it (or reaching into it).
    pub fn add_tile(
        &self,
        tile: &Tile,
        landuse: Option<(&LandUseTile, u32)>,
        buildings: &[&Building],
    ) {
        if let Some(t) = GroundTile::of(tile, self.samples, self.tile_size_m, landuse, buildings)
            && let Ok(mut tiles) = self.tiles.write()
        {
            tiles.insert(tile.key, Arc::new(t));
        }
    }

    pub fn has_tile(&self, key: TileKey) -> bool {
        self.tiles.read().is_ok_and(|t| t.contains_key(&key))
    }

    pub fn tile_count(&self) -> usize {
        self.tiles.read().map_or(0, |t| t.len())
    }

    /// Memory held (approximately, bytes).
    pub fn approx_bytes(&self) -> usize {
        self.tiles
            .read()
            .map_or(0, |t| t.values().map(|t| t.bytes()).sum())
    }

    fn tile(&self, key: TileKey) -> Option<Arc<GroundTile>> {
        self.tiles.read().ok()?.get(&key).cloned()
    }

    /// Whether (x, z) lies on a tile of the map (inside the world).
    pub fn covers(&self, x: f64, z: f64) -> bool {
        x.is_finite()
            && z.is_finite()
            && self.has_tile(TileKey::containing([x, z], self.tile_size_m))
    }

    /// Height of the drawn terrain at (x, z) (the grid triangulated like
    /// the terrain mesh); `None` off the map.
    pub fn height(&self, x: f64, z: f64) -> Option<f64> {
        if !(x.is_finite() && z.is_finite()) {
            return None;
        }
        let size = self.tile_size_m;
        let key = TileKey::containing([x, z], size);
        let t = self.tile(key)?;
        let o = key.origin(size);
        let n = self.samples as usize;
        let s = size / f64::from(self.samples - 1);
        let (fx, fz) = (
            ((x - o[0]) / s).clamp(0.0, (n - 1) as f64),
            ((z - o[1]) / s).clamp(0.0, (n - 1) as f64),
        );
        let (i, k) = ((fx as usize).min(n - 2), (fz as usize).min(n - 2));
        let (tx, tz) = (fx - i as f64, fz - k as f64);
        let (h00, h10, h01, h11) = (
            t.h(n, i, k),
            t.h(n, i + 1, k),
            t.h(n, i, k + 1),
            t.h(n, i + 1, k + 1),
        );
        Some(if tx + tz <= 1.0 {
            h00 + tx * (h10 - h00) + tz * (h01 - h00)
        } else {
            h11 + (1.0 - tx) * (h01 - h11) + (1.0 - tz) * (h10 - h11)
        })
    }

    /// The water surface's level at (x, z) (the nearest terrain vertex's),
    /// where the land use has one; `None` elsewhere.
    pub fn water_level(&self, x: f64, z: f64) -> Option<f64> {
        if !(x.is_finite() && z.is_finite()) {
            return None;
        }
        let size = self.tile_size_m;
        let key = TileKey::containing([x, z], size);
        let t = self.tile(key)?;
        let o = key.origin(size);
        let n = self.samples as usize;
        let s = size / f64::from(self.samples - 1);
        let i = (((x - o[0]) / s).round().max(0.0) as usize).min(n - 1);
        let k = (((z - o[1]) / s).round().max(0.0) as usize).min(n - 1);
        let index = (k * n + i) as u32;
        let at = t.water.binary_search_by_key(&index, |w| w.0).ok()?;
        Some(f64::from(t.water[at].1))
    }

    /// The cover at (x, z); `None` off the map.
    pub fn cover(&self, x: f64, z: f64) -> Option<Cover> {
        if !(x.is_finite() && z.is_finite()) {
            return None;
        }
        let size = self.tile_size_m;
        let key = TileKey::containing([x, z], size);
        let t = self.tile(key)?;
        let o = key.origin(size);
        let c = t.cells as usize;
        let s = size / f64::from(t.cells);
        let col = (((x - o[0]) / s).max(0.0) as usize).min(c - 1);
        let row = (((z - o[1]) / s).max(0.0) as usize).min(c - 1);
        Some(t.cover(col, row))
    }

    /// The gradient (rise over run) at (x, z) along the horizontal unit
    /// direction `dir`: a central difference over ±[`SLOPE_HALF_BASE_M`],
    /// one-sided at the map's edge, within ±[`MAX_GRADE`]; `None` off the
    /// map.
    pub fn slope(&self, x: f64, z: f64, dir: [f64; 2]) -> Option<f64> {
        let here = self.height(x, z)?;
        let h = SLOPE_HALF_BASE_M;
        let ahead = self.height(x + dir[0] * h, z + dir[1] * h);
        let behind = self.height(x - dir[0] * h, z - dir[1] * h);
        let g = match (behind, ahead) {
            (Some(b), Some(a)) => (a - b) / (2.0 * h),
            (None, Some(a)) => (a - here) / h,
            (Some(b), None) => (here - b) / h,
            (None, None) => 0.0,
        };
        Some(if g.is_finite() {
            g.clamp(-MAX_GRADE, MAX_GRADE)
        } else {
            0.0
        })
    }
}

/// The straight track a rider off the road rides for one physics tick
/// (see the module): through `at` along `dir`, the track's distance
/// `anchor_m` at `at`; the horizontal distance along it is the track's
/// distance times the cosine of the slope at the anchor (a rider rides
/// the slope's length, not its map length), heights and gradients from
/// the ground, the surface from the cover.
#[derive(Debug, Clone)]
pub struct OffRoadTrack {
    ground: Arc<GroundMap>,
    at: [f64; 2],
    dir: [f64; 2],
    anchor_m: f64,
    /// The ground's height at `at` (also where the map has none).
    y: f64,
    /// Horizontal metres per metre of the track.
    run: f64,
}

impl OffRoadTrack {
    /// The track through `at` along the horizontal direction `dir` (made a
    /// unit vector), its distance `anchor_m` there.
    pub fn new(ground: Arc<GroundMap>, at: [f64; 2], dir: [f64; 2], anchor_m: f64) -> Self {
        let len = dir[0].hypot(dir[1]);
        let dir = if len > 1e-12 && len.is_finite() {
            [dir[0] / len, dir[1] / len]
        } else {
            [0.0, -1.0]
        };
        let y = ground.height(at[0], at[1]).unwrap_or(0.0);
        let grade = ground.slope(at[0], at[1], dir).unwrap_or(0.0);
        Self {
            ground,
            at,
            dir,
            anchor_m,
            y,
            run: grade.atan().cos(),
        }
    }

    pub fn ground(&self) -> &Arc<GroundMap> {
        &self.ground
    }

    /// Where the rider is (x, z) and the direction of travel.
    pub fn at(&self) -> [f64; 2] {
        self.at
    }

    pub fn direction(&self) -> [f64; 2] {
        self.dir
    }

    pub fn anchor_m(&self) -> f64 {
        self.anchor_m
    }

    /// The horizontal point at track distance `distance`.
    pub fn point(&self, distance: f64) -> [f64; 2] {
        let s = (distance - self.anchor_m) * self.run;
        [self.at[0] + self.dir[0] * s, self.at[1] + self.dir[1] * s]
    }

    /// The cover under track distance `distance`.
    pub fn cover_at(&self, distance: f64) -> Option<Cover> {
        let p = self.point(distance);
        self.ground.cover(p[0], p[1])
    }
}

impl Track for OffRoadTrack {
    fn sample(&self, distance: f64) -> RoutePoint {
        let p = self.point(distance);
        RoutePoint {
            position: [p[0], self.ground.height(p[0], p[1]).unwrap_or(self.y), p[1]],
            direction: self.dir,
            grade: self.ground.slope(p[0], p[1], self.dir).unwrap_or(0.0),
        }
    }
    fn curvature(&self, _distance: f64) -> f64 {
        0.0
    }
    fn surface_at(&self, distance: f64) -> Surface {
        self.cover_at(distance).unwrap_or_default().surface()
    }
    fn road_width_at(&self, _distance: f64) -> f64 {
        OFF_ROAD_WIDTH_M
    }
    fn name_at(&self, distance: f64) -> String {
        format!(
            "off the road ({})",
            self.cover_at(distance).unwrap_or_default().name()
        )
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::landuse::LandUse;

    /// A package of `tiles_x` × 1 tiles of 1 km, 101 samples (10 m), the
    /// heights `h(x, z)`.
    pub fn package(tiles_x: i32, h: impl Fn(f64, f64) -> f64) -> WorldPackage {
        let n = 101u32;
        let mut tiles = BTreeMap::new();
        for tx in 0..tiles_x {
            let key = TileKey { x: tx, z: 0 };
            let o = key.origin(1000.0);
            let mut heights = Vec::new();
            for k in 0..n {
                for i in 0..n {
                    heights.push(h(o[0] + f64::from(i) * 10.0, o[1] + f64::from(k) * 10.0) as f32);
                }
            }
            tiles.insert(
                key,
                Tile {
                    key,
                    heights,
                    roads: Vec::new(),
                },
            );
        }
        WorldPackage {
            name: "ground-test".into(),
            origin: (47.14, 9.52),
            tile_size_m: 1000.0,
            samples: n,
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

    /// A land use of 200 cells (5 m) a tile, `class(x, z)` per cell centre.
    pub fn with_landuse(
        mut pkg: WorldPackage,
        class: impl Fn(f64, f64) -> LandClass,
    ) -> WorldPackage {
        let cells = 200u32;
        let mut tiles = BTreeMap::new();
        for key in pkg.tiles.keys() {
            let o = key.origin(1000.0);
            let mut classes = Vec::new();
            for r in 0..cells {
                for c in 0..cells {
                    classes.push(class(
                        o[0] + (f64::from(c) + 0.5) * 5.0,
                        o[1] + (f64::from(r) + 0.5) * 5.0,
                    ));
                }
            }
            tiles.insert(
                *key,
                LandUseTile {
                    classes,
                    water: Vec::new(),
                },
            );
        }
        pkg.landuse = Some(LandUse { cells, tiles });
        pkg
    }

    #[test]
    fn heights_match_the_drawn_terrain_within_two_centimetres() {
        let pkg = package(2, |x, z| 400.0 + 0.3 * x + 30.0 * (z / 70.0).sin());
        let g = GroundMap::from_package(&pkg);
        assert_eq!(g.tile_count(), 2);
        for (x, z) in [
            (12.3, 456.7),
            (999.9, 3.0),
            (1500.5, 500.5),
            (1990.0, 990.0),
        ] {
            let a = g.height(x, z).unwrap();
            let b = pkg.surface_height(x, z).unwrap();
            assert!((a - b).abs() < 0.02, "{x},{z}: {a} vs {b}");
        }
        assert_eq!(g.height(-5.0, 10.0), None, "outside the world");
        assert_eq!(g.height(f64::NAN, 10.0), None);
        assert!(g.covers(10.0, 10.0) && !g.covers(10.0, -10.0));
        assert!(g.approx_bytes() < 2 * 25_000, "{}", g.approx_bytes());
    }

    #[test]
    fn the_slope_along_a_heading_is_its_real_gradient() {
        // 40 % up towards +x, flat across.
        let pkg = package(1, |x, _| 0.4 * x);
        let g = GroundMap::from_package(&pkg);
        let up = g.slope(500.0, 500.0, [1.0, 0.0]).unwrap();
        assert!((up - 0.4).abs() < 1e-3, "{up}");
        let down = g.slope(500.0, 500.0, [-1.0, 0.0]).unwrap();
        assert!((down + 0.4).abs() < 1e-3, "{down}");
        let across = g.slope(500.0, 500.0, [0.0, 1.0]).unwrap();
        assert!(across.abs() < 1e-3, "{across}");
        // At the world's edge one-sided, never off.
        assert!((g.slope(0.5, 500.0, [-1.0, 0.0]).unwrap() + 0.4).abs() < 1e-3);
        // A cliff: 300 m in one 10 m cell, finite and within the bound.
        let cliff = package(1, |x, _| if x > 500.0 { 0.0 } else { 300.0 });
        let g = GroundMap::from_package(&cliff);
        let s = g.slope(500.0, 300.0, [1.0, 0.0]).unwrap();
        assert!(s.is_finite() && (-MAX_GRADE..-1.0).contains(&s), "{s}");
    }

    #[test]
    fn the_cover_comes_from_the_land_use_and_the_buildings() {
        let pkg = with_landuse(package(1, |_, _| 500.0), |x, _| {
            if x < 300.0 {
                LandClass::Meadow
            } else if x < 500.0 {
                LandClass::Forest
            } else if x < 700.0 {
                LandClass::Water
            } else {
                LandClass::Bare
            }
        });
        let mut pkg = pkg;
        let house = Building {
            kind: crate::buildings::BuildingKind::House,
            outline: vec![
                [100.0, 100.0],
                [112.0, 100.0],
                [112.0, 110.0],
                [100.0, 110.0],
            ],
            holes: vec![],
            base_y: 499.5,
            eave_y: 506.0,
            roof: crate::buildings::Roof::Flat,
            tags: Default::default(),
            parts: Default::default(),
        };
        pkg.buildings = Some(
            [(TileKey { x: 0, z: 0 }, vec![house])]
                .into_iter()
                .collect(),
        );
        let g = GroundMap::from_package(&pkg);
        assert_eq!(g.cover(50.0, 50.0), Some(Cover::Grass));
        assert_eq!(g.cover(106.0, 105.0), Some(Cover::Building));
        assert_eq!(g.cover(400.0, 900.0), Some(Cover::Forest));
        assert_eq!(g.cover(600.0, 500.0), Some(Cover::Water));
        assert_eq!(g.cover(900.0, 10.0), Some(Cover::Sand));
        assert_eq!(g.cover(-1.0, 10.0), None);
        assert!(!Cover::Water.rideable() && !Cover::Building.rideable());
        assert!(Cover::Rock.rideable());
        // Rougher ground rolls harder.
        assert!(Cover::Rock.surface().rolling_factor() < Cover::Grass.surface().rolling_factor());
        assert!(Cover::Grass.surface().rolling_factor() < Cover::Forest.surface().rolling_factor());
        assert!(Cover::Forest.surface().rolling_factor() < Cover::Sand.surface().rolling_factor());
        // A package without land use: grass.
        let g = GroundMap::from_package(&package(1, |_, _| 0.0));
        assert_eq!(g.cover(600.0, 500.0), Some(Cover::Grass));
    }

    #[test]
    fn the_off_road_track_follows_the_ground_along_the_heading() {
        let pkg = with_landuse(package(1, |x, _| 0.1 * x), |_, _| LandClass::Forest);
        let g = Arc::new(GroundMap::from_package(&pkg));
        let t = OffRoadTrack::new(g, [500.0, 500.0], [3.0, 0.0], 1234.0);
        let p = t.sample(1234.0);
        assert!((p.position[0] - 500.0).abs() < 1e-9 && (p.position[2] - 500.0).abs() < 1e-9);
        assert!((p.position[1] - 50.0).abs() < 0.02, "{p:?}");
        assert!((p.grade - 0.1).abs() < 1e-3);
        assert_eq!(p.direction, [1.0, 0.0]);
        // 10 m along the slope is 10 × cos(atan 0.1) on the map.
        let q = t.sample(1244.0);
        let run = 10.0 * 0.1f64.atan().cos();
        assert!((q.position[0] - 500.0 - run).abs() < 1e-3, "{q:?}");
        assert_eq!(t.surface_at(1240.0), Surface::ForestFloor);
        assert_eq!(t.road_width_at(0.0), OFF_ROAD_WIDTH_M);
        assert!(t.name_at(1234.0).contains("forest"));
        // Off the map: the anchor's height, flat.
        let far = t.sample(1234.0 + 2000.0);
        assert!(far.position[1].is_finite() && far.grade == 0.0);
    }
}
