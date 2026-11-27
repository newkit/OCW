//! Where leaves lie on the road (spec 4.6.2, Q5 and Q6): a small,
//! deterministic map of the package, shared by the renderer (the leaves it
//! draws on the road, `ocw-render::ground_detail`) and the physics (wet
//! leaves cost a little speed, wobble the bike and, with damage on, may
//! make it slide in a corner: `ocw-sim::leaves`). Both read the same
//! [`LeafMap`] cell codes and the same [`per_metre`], so what is drawn on
//! the road is what the server rides over.
//!
//! **The map:** [`LEAF_CELLS`]² cells a tile (20 m for 1 km tiles, the
//! forest section's grid), each one code of four bits ([`LeafCode`]): no
//! leaves (water, wetland, glacier), open land, a town, a wood whose leaf
//! type is not measured or a wood with its measured broadleaf share (12
//! steps). The code comes from the land use at 3 × 3 points of the cell
//! (the class most of them have) and the forest section's broadleaf share
//! at its centre. A tile whose cells all have the same code keeps one
//! byte; tiles without land use are open land. Black Forest south (2,800
//! tiles) takes at most 3.6 MB.
//!
//! **The amount** ([`per_metre`], leaves per metre of one side's half of
//! the carriageway, as drawn): 0.8 × the broadleaf share (at least
//! [`LEAF_WOOD_MIN`]) × the leaf fall in a wood, 0.06 × the leaf fall in
//! open land and half that in towns (swept streets), thinned by the
//! climatological snow cover ((1 − snow)²). The leaf fall (0 in summer, 1
//! when the leaves are down, `ocw_sky::season::SeasonDay::leaf_fall`) and
//! the snow come from the season at the road's altitude. Tenerife's woods
//! are evergreen (laurel, fayal-brezal, Canary pine): they count as open
//! land ([`evergreen_woods`]), and the season's fall there is that of its
//! few deciduous lone trees, a tenth (spec §14 Phase 9 Q13).

use std::collections::BTreeMap;

use crate::flora::FloraRegion;
use crate::forest::ForestCover;
use crate::landuse::{LandClass, LandUse, LandUseIndex};
use crate::{TileKey, WorldPackage};

/// Cells per tile side of the map (20 m for 1 km tiles).
pub const LEAF_CELLS: u32 = 50;
/// Leaves per metre of one half of the carriageway in a pure broadleaf
/// wood when all leaves are down (spec 4.6.2 Q5, v5.08).
pub const WOOD_PER_M: f32 = 0.8;
/// … in open land (blown out of the hedges and roadside trees).
pub const OPEN_PER_M: f32 = 0.06;
/// Towns sweep their streets: this share of the open land's.
pub const SWEPT: f32 = 0.5;
/// The broadleaf share the leaves count with at least in a wood (mixed
/// woods measured as mostly conifers still have beeches and maples by the
/// roads; spec 4.6.2 Q5).
pub const LEAF_WOOD_MIN: f32 = 0.2;
/// Steps of a measured broadleaf share in a [`LeafCode`].
pub const BROADLEAF_STEPS: u8 = 11;

/// What a cell of the map says about the leaves (four bits).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LeafCode {
    /// Water, wetland, glacier: none.
    None,
    /// Open land: meadows, fields, rock, scrub …
    Open,
    /// A town (residential, commercial, industrial).
    Town,
    /// A wood without a measured leaf type (the region's share).
    Wood,
    /// A wood with its measured broadleaf share in steps of
    /// 1 / [`BROADLEAF_STEPS`] (0 … 11).
    WoodMeasured(u8),
}

impl LeafCode {
    pub fn nibble(self) -> u8 {
        match self {
            LeafCode::None => 0,
            LeafCode::Open => 1,
            LeafCode::Town => 2,
            LeafCode::Wood => 3,
            LeafCode::WoodMeasured(b) => 4 + b.min(BROADLEAF_STEPS),
        }
    }

    pub fn from_nibble(n: u8) -> Self {
        match n & 15 {
            0 => LeafCode::None,
            1 => LeafCode::Open,
            2 => LeafCode::Town,
            3 => LeafCode::Wood,
            b => LeafCode::WoodMeasured(b - 4),
        }
    }

    /// The leaves' class of a land-use class.
    fn of_class(c: LandClass) -> Self {
        match c {
            LandClass::Water | LandClass::Wetland | LandClass::Glacier => LeafCode::None,
            LandClass::Forest => LeafCode::Wood,
            LandClass::Residential | LandClass::Commercial | LandClass::Industrial => {
                LeafCode::Town
            }
            _ => LeafCode::Open,
        }
    }

    /// The broadleaf share of a wood (`None` elsewhere): measured, else the
    /// region's at `alt_m`.
    pub fn broadleaf(self, region: FloraRegion, alt_m: f64) -> Option<f32> {
        match self {
            LeafCode::Wood => Some(region_broadleaf(region, alt_m)),
            LeafCode::WoodMeasured(b) => Some(f32::from(b) / f32::from(BROADLEAF_STEPS)),
            _ => None,
        }
    }
}

/// The region's broadleaf share where the package has no forest type.
pub fn region_broadleaf(region: FloraRegion, alt_m: f64) -> f32 {
    match region {
        FloraRegion::BlackForest => 0.3,
        FloraRegion::AlpineRhine if alt_m > 1200.0 => 0.15,
        FloraRegion::AlpineRhine => 0.4,
        // Laurel forest in the cloud belt, Canary pine above.
        FloraRegion::Tenerife if alt_m < 1100.0 => 0.7,
        FloraRegion::Tenerife => 0.0,
        // Andorra's pine forests: oaks, birches and hazels low down, a few
        // birches among the pines higher up (spec 4.6.1, table 6).
        FloraRegion::Pyrenees if alt_m < 1300.0 => 0.3,
        FloraRegion::Pyrenees => 0.06,
        FloraRegion::Other => 0.5,
    }
}

/// Whether the region's woods are evergreen (their broadleaves too): no
/// leaf fall from the woods themselves.
pub fn evergreen_woods(region: FloraRegion) -> bool {
    region == FloraRegion::Tenerife
}

/// Leaves per metre of one side's half of the carriageway at a place of
/// `code` (see the module docs): `fall` the season's leaf fall (0 – 1),
/// `snow` its climatological snow cover (0 – 1), at `alt_m`.
pub fn per_metre(code: LeafCode, region: FloraRegion, alt_m: f64, fall: f32, snow: f32) -> f32 {
    let fall = fall.clamp(0.0, 1.0);
    let base = match code {
        LeafCode::None => 0.0,
        LeafCode::Open => OPEN_PER_M,
        LeafCode::Town => OPEN_PER_M * SWEPT,
        // Laurel forest, fayal-brezal and Canary pines keep their leaves:
        // only the lone deciduous trees by the road shed, as in open land
        // (spec §14 Phase 9 Q13; the season's fall is already scaled to
        // those few trees, `SeasonDay::leaf_fall`).
        LeafCode::Wood | LeafCode::WoodMeasured(_) if evergreen_woods(region) => OPEN_PER_M,
        LeafCode::Wood | LeafCode::WoodMeasured(_) => {
            WOOD_PER_M
                * code
                    .broadleaf(region, alt_m)
                    .unwrap_or(0.0)
                    .max(LEAF_WOOD_MIN)
        }
    };
    base * fall * (1.0 - snow.clamp(0.0, 1.0)).powi(2)
}

/// The cell of the map holding (x, z) (package metres): its tile and its
/// column and row.
pub fn cell_of(tile_size_m: f64, x: f64, z: f64) -> (TileKey, u32, u32) {
    let key = TileKey::containing([x, z], tile_size_m);
    let o = key.origin(tile_size_m);
    let s = tile_size_m / f64::from(LEAF_CELLS);
    let c = (((x - o[0]) / s).floor().max(0.0) as u32).min(LEAF_CELLS - 1);
    let r = (((z - o[1]) / s).floor().max(0.0) as u32).min(LEAF_CELLS - 1);
    (key, c, r)
}

/// The code of one cell from the land use (`class_at`, package metres) and
/// the forest section: the leaf class most of 3 × 3 points of the cell
/// have (ties: wood, open land, town, none), a wood's measured broadleaf
/// share at its centre.
pub fn cell_code(
    tile_size_m: f64,
    key: TileKey,
    col: u32,
    row: u32,
    class_at: &dyn Fn(f64, f64) -> LandClass,
    forest: Option<&ForestCover>,
) -> LeafCode {
    let o = key.origin(tile_size_m);
    let s = tile_size_m / f64::from(LEAF_CELLS);
    let (cx, cz) = (
        o[0] + (f64::from(col) + 0.5) * s,
        o[1] + (f64::from(row) + 0.5) * s,
    );
    // none, open, town, wood
    let mut n = [0u8; 4];
    for dz in [-1.0, 0.0, 1.0] {
        for dx in [-1.0, 0.0, 1.0] {
            let k = match LeafCode::of_class(class_at(cx + dx * s / 3.0, cz + dz * s / 3.0)) {
                LeafCode::None => 0,
                LeafCode::Open => 1,
                LeafCode::Town => 2,
                _ => 3,
            };
            n[k] += 1;
        }
    }
    let best = [3usize, 1, 2, 0]
        .into_iter()
        .max_by_key(|&k| (n[k], [0u8, 2, 1, 3][k]))
        .unwrap_or(1);
    match best {
        0 => LeafCode::None,
        1 => LeafCode::Open,
        2 => LeafCode::Town,
        _ => match forest
            .and_then(|f| f.cell(tile_size_m, cx, cz))
            .and_then(|c| c.broadleaf)
        {
            Some(b) => LeafCode::WoodMeasured(
                ((f32::from(b.min(100)) / 100.0 * f32::from(BROADLEAF_STEPS)).round()) as u8,
            ),
            None => LeafCode::Wood,
        },
    }
}

/// The code at (x, z) straight from a package's land use and forest (the
/// renderer's way: its resident tiles hold their land use); the same as
/// [`LeafMap::code_at`] of a map built from that package.
pub fn code_at(pkg: &WorldPackage, x: f64, z: f64) -> LeafCode {
    let size = f64::from(pkg.tile_size_m);
    let (key, c, r) = cell_of(size, x, z);
    let Some(lu) = &pkg.landuse else {
        return LeafCode::Open;
    };
    if !lu.tiles.contains_key(&key) {
        return LeafCode::Open;
    }
    let class_at = |x: f64, z: f64| lu.class_at(size, x, z).unwrap_or_default();
    cell_code(size, key, c, r, &class_at, pkg.look.forest.as_ref())
}

/// One tile of the map.
#[derive(Debug, Clone, PartialEq, Eq)]
enum LeafTile {
    /// Every cell the same.
    Uniform(u8),
    /// Two cells a byte (low nibble first), row-major along +z.
    Grid(Box<[u8]>),
}

/// Where leaves lie on the roads of a package (see the module docs).
#[derive(Debug, Clone, PartialEq)]
pub struct LeafMap {
    tile_size_m: f64,
    region: FloraRegion,
    tiles: BTreeMap<TileKey, LeafTile>,
}

impl LeafMap {
    /// From a package with its land use (a whole file, or a resident
    /// version: only its tiles with land use are mapped, the others are
    /// open land).
    pub fn from_package(pkg: &WorldPackage) -> Self {
        let mut map = Self::empty(pkg);
        if let Some(lu) = &pkg.landuse {
            for key in lu.tiles.keys() {
                map.add_tile(*key, lu, pkg.look.forest.as_ref());
            }
        }
        map
    }

    /// From a streamed package's base and its encoded land use (spec §14
    /// Phase 10 B2: the tiles decoded one at a time).
    pub fn from_streamed(base: &WorldPackage, landuse: Option<&LandUseIndex>) -> Self {
        let mut map = Self::empty(base);
        if let Some(index) = landuse {
            for key in index.keys() {
                let one = index.land_use([key]);
                map.add_tile(key, &one, base.look.forest.as_ref());
            }
        }
        map
    }

    fn empty(pkg: &WorldPackage) -> Self {
        Self {
            tile_size_m: f64::from(pkg.tile_size_m),
            region: FloraRegion::at(pkg.origin.0, pkg.origin.1),
            tiles: BTreeMap::new(),
        }
    }

    fn add_tile(&mut self, key: TileKey, lu: &LandUse, forest: Option<&ForestCover>) {
        let size = self.tile_size_m;
        let class_at = |x: f64, z: f64| lu.class_at(size, x, z).unwrap_or_default();
        let n = LEAF_CELLS as usize;
        let mut nibbles = Vec::with_capacity(n * n);
        for r in 0..LEAF_CELLS {
            for c in 0..LEAF_CELLS {
                nibbles.push(cell_code(size, key, c, r, &class_at, forest).nibble());
            }
        }
        let tile = if nibbles.iter().all(|&v| v == nibbles[0]) {
            LeafTile::Uniform(nibbles[0])
        } else {
            LeafTile::Grid(
                nibbles
                    .chunks(2)
                    .map(|p| p[0] | (p.get(1).copied().unwrap_or(0) << 4))
                    .collect(),
            )
        };
        self.tiles.insert(key, tile);
    }

    /// The region (by the package's origin).
    pub fn region(&self) -> FloraRegion {
        self.region
    }

    /// The code at (x, z) (package metres).
    pub fn code_at(&self, x: f64, z: f64) -> LeafCode {
        let (key, c, r) = cell_of(self.tile_size_m, x, z);
        match self.tiles.get(&key) {
            None => LeafCode::Open,
            Some(LeafTile::Uniform(v)) => LeafCode::from_nibble(*v),
            Some(LeafTile::Grid(g)) => {
                let i = (r * LEAF_CELLS + c) as usize;
                LeafCode::from_nibble(g[i / 2] >> (4 * (i % 2)))
            }
        }
    }

    /// Leaves per metre at (x, z) and `alt_m` with the season's `fall` and
    /// `snow` ([`per_metre`]).
    pub fn per_metre_at(&self, x: f64, z: f64, alt_m: f64, fall: f32, snow: f32) -> f32 {
        per_metre(self.code_at(x, z), self.region, alt_m, fall, snow)
    }

    /// Tiles mapped.
    pub fn len(&self) -> usize {
        self.tiles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    /// The bytes it keeps (about).
    pub fn approx_bytes(&self) -> usize {
        self.tiles
            .values()
            .map(|t| match t {
                LeafTile::Uniform(_) => 32,
                LeafTile::Grid(g) => 32 + g.len(),
            })
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forest::ForestTile;
    use crate::landuse::LandUseTile;

    /// A package of one 1 km tile at the Black Forest's origin: forest in
    /// the west half (measured 60 % broadleaf in its north-west quarter),
    /// a town in the south-east, meadow in the north-east, a lake strip.
    pub fn package() -> WorldPackage {
        let cells = 100u32;
        let mut classes = Vec::new();
        for r in 0..cells {
            for c in 0..cells {
                classes.push(if c < 50 {
                    LandClass::Forest
                } else if r >= 50 {
                    LandClass::Residential
                } else if (60..62).contains(&c) {
                    LandClass::Water
                } else {
                    LandClass::Meadow
                });
            }
        }
        let key = TileKey { x: 0, z: 0 };
        let mut lu = LandUse {
            cells,
            tiles: BTreeMap::new(),
        };
        lu.tiles.insert(
            key,
            LandUseTile {
                classes,
                water: Vec::new(),
            },
        );
        let fc = 50u32;
        let mut broadleaf = vec![crate::forest::NO_DATA; (fc * fc) as usize];
        let density = vec![80u8; (fc * fc) as usize];
        for r in 0..25 {
            for c in 0..25 {
                broadleaf[(r * fc + c) as usize] = 60;
            }
        }
        let mut forest = ForestCover {
            cells: fc,
            ..Default::default()
        };
        forest.tiles.insert(key, ForestTile { density, broadleaf });
        let mut pkg = WorldPackage {
            name: "Leaves".into(),
            origin: (48.0, 8.2),
            tile_size_m: 1000.0,
            samples: 2,
            tiles: BTreeMap::new(),
            graph: None,
            landuse: Some(lu),
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
        };
        pkg.look.forest = Some(forest);
        pkg
    }

    #[test]
    fn codes_and_amounts() {
        for n in 0..16u8 {
            assert_eq!(LeafCode::from_nibble(n).nibble(), n);
        }
        let pkg = package();
        let map = LeafMap::from_package(&pkg);
        assert_eq!(map.region(), FloraRegion::BlackForest);
        assert_eq!(map.code_at(100.0, 100.0), LeafCode::WoodMeasured(7));
        assert_eq!(map.code_at(100.0, 700.0), LeafCode::Wood);
        assert_eq!(map.code_at(800.0, 800.0), LeafCode::Town);
        assert_eq!(map.code_at(800.0, 200.0), LeafCode::Open);
        assert_eq!(map.code_at(610.0, 200.0), LeafCode::None, "the lake");
        // Outside the mapped tiles: open land.
        assert_eq!(map.code_at(-5.0, 100.0), LeafCode::Open);
        // The amounts (spec 4.6.2 Q5): a measured 7/11 broadleaf wood, the
        // region's 0.3 in the Black Forest, open land, towns, nothing in
        // summer, thinned by snow.
        let a = |x, z, fall, snow| map.per_metre_at(x, z, 500.0, fall, snow);
        assert!((a(100.0, 100.0, 1.0, 0.0) - 0.8 * 7.0 / 11.0).abs() < 1e-6);
        assert!((a(100.0, 700.0, 1.0, 0.0) - 0.24).abs() < 1e-6);
        assert!((a(800.0, 200.0, 1.0, 0.0) - 0.06).abs() < 1e-6);
        assert!((a(800.0, 800.0, 1.0, 0.0) - 0.03).abs() < 1e-6);
        assert_eq!(a(610.0, 200.0, 1.0, 0.0), 0.0);
        assert_eq!(a(100.0, 100.0, 0.0, 0.0), 0.0);
        assert!((a(100.0, 700.0, 0.5, 0.5) - 0.24 * 0.5 * 0.25).abs() < 1e-6);
        // A wood measured all conifers still has a few leaves.
        assert!(
            (per_metre(LeafCode::WoodMeasured(0), map.region(), 0.0, 1.0, 0.0) - 0.16).abs() < 1e-6
        );
    }

    /// Spec §14 Phase 9 Q13: Tenerife's woods are evergreen – a laurel
    /// wood (measured broadleaves) or a pine wood sheds no more than open
    /// land; the Black Forest's woods as before.
    #[test]
    fn evergreen_woods_shed_like_open_land() {
        let tf = FloraRegion::Tenerife;
        assert!(evergreen_woods(tf));
        for code in [
            LeafCode::Wood,
            LeafCode::WoodMeasured(0),
            LeafCode::WoodMeasured(BROADLEAF_STEPS),
        ] {
            for alt in [50.0, 800.0, 1600.0] {
                assert_eq!(
                    per_metre(code, tf, alt, 1.0, 0.0),
                    per_metre(LeafCode::Open, tf, alt, 1.0, 0.0)
                );
            }
        }
        assert!(!evergreen_woods(FloraRegion::BlackForest));
        assert!(
            (per_metre(LeafCode::Wood, FloraRegion::BlackForest, 500.0, 1.0, 0.0) - 0.24).abs()
                < 1e-6
        );
    }

    #[test]
    fn the_map_is_the_package_read_directly_and_compact() {
        let pkg = package();
        let map = LeafMap::from_package(&pkg);
        for i in 0..400 {
            let (x, z) = (
                f64::from(i * 37 % 1000) + 0.3,
                f64::from(i * 53 % 1000) + 0.7,
            );
            assert_eq!(map.code_at(x, z), code_at(&pkg, x, z), "{x}, {z}");
        }
        // Deterministic.
        assert_eq!(map, LeafMap::from_package(&pkg));
        // Two cells a byte.
        assert_eq!(map.approx_bytes(), 32 + 1250);
        // The streamed way gives the same map.
        let mut bytes = Vec::new();
        crate::landuse::write_landuse(pkg.landuse.as_ref().unwrap(), &mut bytes);
        let index = LandUseIndex::new(&bytes, pkg.samples).unwrap();
        let mut base = pkg.clone();
        base.landuse = None;
        assert_eq!(LeafMap::from_streamed(&base, Some(&index)), map);
        // A tile all of one kind keeps one byte.
        let mut meadow = pkg.clone();
        let tile = meadow
            .landuse
            .as_mut()
            .unwrap()
            .tiles
            .values_mut()
            .next()
            .unwrap();
        tile.classes.fill(LandClass::Meadow);
        assert_eq!(LeafMap::from_package(&meadow).approx_bytes(), 32);
    }
}
