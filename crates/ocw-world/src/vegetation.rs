//! The fields, plantations and mapped trees of a package (spec §4.6.2,
//! §14 Phase 8 T8): what the land use's classes do not tell – which crop
//! grows on a field, which way the rows of a vineyard, an orchard or a
//! banana plantation run, and the single trees and tree rows OSM maps
//! (palms on the promenades, limes in the squares) with their species.
//!
//! Built by the world builder from OSM (`landuse=farmland|orchard|
//! vineyard|greenhouse_horticulture` with `crop=*` / `produce=*` /
//! `trees=*`, `natural=tree` and `natural=tree_row` with `genus` /
//! `species` / `leaf_type`) and, where a package has them, measured crop
//! maps; fields without a crop get one by the region's crop mix, one per
//! field (the OSM outline, larger ones split into parcels) – real outlines
//! instead of the noise patches of the seasons (spec §14 Phase 9, S8).
//!
//! Optional package section `VEGE` (spec §4.7; readers that do not know it
//! skip it, a package without it keeps the noise patches, orchards on the
//! tree grid and no mapped trees):
//!
//! ```text
//! layout version: u32 (1)
//! cells per tile side: u32 (100: 10 m)
//! source count: u8, then per source: id, name, licence, notice, url,
//!   date (each u32 length + UTF-8), resolution (m): f32
//! tile count: u32, then per tile:
//!   x, z: i32
//!   planes length: u32 + bytes (deflated; 0: no fields in the tile):
//!     crops cells² (FieldCrop codes, row-major along +z), then rows cells²
//!     (the rows' direction × π / 256 from +x towards +z, 255: no rows)
//!   tree count: u32, per tree: x, z: f32 (package metres), species: u8,
//!     height: u16 (dm, 0: not mapped), source: u8 (1 tree, 2 tree row)
//! ```
//!
//! Cell `(c, r)` covers local metres `[o.x + c·s, o.x + (c+1)·s] ×
//! [o.z + r·s, o.z + (r+1)·s]` with `s = tile size / cells`, like the land
//! use. The rows are global lines (`dot(p, n) = k · spacing`, `n` the
//! normal of the direction), so a field's rows continue across cells and
//! tiles ([`crate::plantings`]).

use std::collections::BTreeMap;

use crate::flora::Species;
use crate::look::{ImagerySource, check_version, get_str, put_str};
use crate::package::{Bytes, TileKey};

/// Section tag of the vegetation.
pub const VEGETATION_TAG: &[u8; 4] = b"VEGE";
const VEGETATION_VERSION: u32 = 1;
/// Cells per tile side the world builder writes by default (10 m).
pub const DEFAULT_CELLS: u32 = 100;
/// No rows in a cell.
pub const NO_ROWS: u8 = 255;

/// What grows on a field or plantation. The codes are stored in packages:
/// only add new ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
#[repr(u8)]
pub enum FieldCrop {
    /// Not a field (or nothing known).
    #[default]
    None = 0,
    /// Winter cereals: wheat, barley, rye, triticale, oats, spelt.
    Cereal = 1,
    /// Maize (grain and silage).
    Maize = 2,
    /// Rapeseed: yellow in April and May.
    Rapeseed = 3,
    Sunflower = 4,
    Potato = 5,
    /// Sugar and fodder beet.
    Beet = 6,
    /// Temporary grass, clover, lucerne: mown like a meadow.
    Grass = 7,
    /// Vegetables, strawberries, flowers in small plots.
    Vegetables = 8,
    Vineyard = 9,
    /// Fruit trees: apple, pear, cherry, plum, walnut.
    Orchard = 10,
    Banana = 11,
    /// Berries and other bush crops.
    Berries = 12,
    Fallow = 13,
    /// Under glass, plastic or netting.
    Greenhouse = 14,
    /// Avocado, citrus, mango, papaya.
    Subtropical = 15,
}

impl FieldCrop {
    pub const ALL: [FieldCrop; 16] = [
        FieldCrop::None,
        FieldCrop::Cereal,
        FieldCrop::Maize,
        FieldCrop::Rapeseed,
        FieldCrop::Sunflower,
        FieldCrop::Potato,
        FieldCrop::Beet,
        FieldCrop::Grass,
        FieldCrop::Vegetables,
        FieldCrop::Vineyard,
        FieldCrop::Orchard,
        FieldCrop::Banana,
        FieldCrop::Berries,
        FieldCrop::Fallow,
        FieldCrop::Greenhouse,
        FieldCrop::Subtropical,
    ];

    pub fn code(self) -> u8 {
        self as u8
    }

    pub fn from_code(code: u8) -> Option<Self> {
        Self::ALL.get(usize::from(code)).copied()
    }

    pub fn name(self) -> &'static str {
        match self {
            FieldCrop::None => "none",
            FieldCrop::Cereal => "cereal",
            FieldCrop::Maize => "maize",
            FieldCrop::Rapeseed => "rapeseed",
            FieldCrop::Sunflower => "sunflower",
            FieldCrop::Potato => "potato",
            FieldCrop::Beet => "beet",
            FieldCrop::Grass => "grass",
            FieldCrop::Vegetables => "vegetables",
            FieldCrop::Vineyard => "vineyard",
            FieldCrop::Orchard => "orchard",
            FieldCrop::Banana => "banana",
            FieldCrop::Berries => "berries",
            FieldCrop::Fallow => "fallow",
            FieldCrop::Greenhouse => "greenhouse",
            FieldCrop::Subtropical => "avocado / citrus",
        }
    }

    /// Planted in rows ([`crate::plantings`]): vines, fruit trees, bananas,
    /// avocados and citrus, and maize (drawn as a stand).
    pub fn in_rows(self) -> bool {
        matches!(
            self,
            FieldCrop::Vineyard
                | FieldCrop::Orchard
                | FieldCrop::Banana
                | FieldCrop::Subtropical
                | FieldCrop::Maize
                | FieldCrop::Berries
        )
    }

    /// Whether the seasons colour the ground as a summer crop (sown in
    /// spring, the maize calendar) rather than a winter cereal.
    pub fn summer_crop(self) -> bool {
        matches!(
            self,
            FieldCrop::Maize
                | FieldCrop::Sunflower
                | FieldCrop::Potato
                | FieldCrop::Beet
                | FieldCrop::Vegetables
        )
    }

    /// A field crop of the arable land (the ones the seasons' crop calendar
    /// colours), not a plantation.
    pub fn arable(self) -> bool {
        matches!(
            self,
            FieldCrop::Cereal
                | FieldCrop::Maize
                | FieldCrop::Rapeseed
                | FieldCrop::Sunflower
                | FieldCrop::Potato
                | FieldCrop::Beet
                | FieldCrop::Vegetables
                | FieldCrop::Fallow
        )
    }

    /// The crop of an OSM area from its `crop=*`, `produce=*` and
    /// `trees=*` (`;`-separated lists: the first known value), `None` if
    /// they say nothing known.
    pub fn from_osm(crop: Option<&str>, produce: Option<&str>, trees: Option<&str>) -> Option<Self> {
        let values = [trees, produce, crop];
        for v in values.into_iter().flatten() {
            for part in v.split(';') {
                if let Some(c) = Self::of_osm_value(part.trim()) {
                    return Some(c);
                }
            }
        }
        None
    }

    fn of_osm_value(v: &str) -> Option<Self> {
        let v = v.to_ascii_lowercase();
        Some(match v.as_str() {
            "wheat" | "barley" | "rye" | "oats" | "oat" | "triticale" | "spelt" | "cereal"
            | "grain" | "winter_wheat" | "winter_barley" => FieldCrop::Cereal,
            "maize" | "corn" | "sweet_corn" | "silage_maize" => FieldCrop::Maize,
            "rape" | "rapeseed" | "canola" | "oilseed_rape" => FieldCrop::Rapeseed,
            "sunflower" | "sunflowers" => FieldCrop::Sunflower,
            "potato" | "potatoes" => FieldCrop::Potato,
            "sugar_beet" | "beet" | "beets" | "fodder_beet" => FieldCrop::Beet,
            "grass" | "clover" | "lucerne" | "alfalfa" | "hay" | "native_pasture" | "fodder" => {
                FieldCrop::Grass
            }
            "vegetable" | "vegetables" | "strawberry" | "strawberries" | "flowers" | "asparagus"
            | "tomato" | "tomatoes" | "lettuce" | "cabbage" | "onion" | "onions" | "tobacco"
            | "hop" | "hops" | "flower" => FieldCrop::Vegetables,
            "grape" | "grapes" | "wine" | "grape_vines" => FieldCrop::Vineyard,
            "apple" | "apples" | "apple_trees" | "pear" | "pears" | "pear_trees" | "cherry"
            | "cherries" | "cherry_trees" | "plum" | "plums" | "plum_trees" | "walnut"
            | "walnut_trees" | "nut_trees" | "hazel" | "chestnut" | "chestnut_trees"
            | "peach" | "apricot" | "fruit" | "fruit_trees" | "almond_trees" | "olive"
            | "olive_trees" | "fig_trees" => FieldCrop::Orchard,
            "banana" | "bananas" | "banana_plants" | "banana_trees" | "plantain" => {
                FieldCrop::Banana
            }
            "avocado" | "avocados" | "avocado_trees" | "orange" | "oranges" | "orange_trees"
            | "citrus" | "citrus_trees" | "lemon" | "lemon_trees" | "mango" | "mango_trees"
            | "papaya" | "papaya_trees" | "coconut_palms" | "date_palms" => FieldCrop::Subtropical,
            "berries" | "raspberry" | "blueberry" | "blueberries" | "currant" | "kiwi" => {
                FieldCrop::Berries
            }
            "fallow" | "set_aside" => FieldCrop::Fallow,
            _ => return None,
        })
    }
}

/// A mapped tree (OSM `natural=tree`, or a tree of a `natural=tree_row`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlantedTree {
    /// Package metres (x, z).
    pub pos: [f32; 2],
    /// [`Species::Unknown`]: the rules decide by the place (within the
    /// group of `leaf`, if OSM gave a leaf type).
    pub species: Species,
    /// Height mapped in OSM (m; 0: not mapped).
    pub height_m: f32,
    pub source: TreeSource,
}

/// Where a mapped tree comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TreeSource {
    Tree = 1,
    TreeRow = 2,
}

/// The fields and mapped trees of one tile.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct VegetationTile {
    /// `cells²` [`FieldCrop`] codes (empty: no fields in the tile).
    pub crops: Vec<u8>,
    /// `cells²` row directions (`× π / 256`, [`NO_ROWS`]; empty with the
    /// crops).
    pub rows: Vec<u8>,
    pub trees: Vec<PlantedTree>,
}

impl VegetationTile {
    /// The crop and row direction (radians, from +x towards +z) of cell
    /// `index`.
    pub fn cell(&self, index: usize) -> (FieldCrop, Option<f32>) {
        let crop = self
            .crops
            .get(index)
            .and_then(|c| FieldCrop::from_code(*c))
            .unwrap_or_default();
        let rows = self
            .rows
            .get(index)
            .filter(|r| **r != NO_ROWS)
            .map(|r| f32::from(*r) / 256.0 * std::f32::consts::PI);
        (crop, rows)
    }

    pub fn is_empty(&self) -> bool {
        self.crops.is_empty() && self.trees.is_empty()
    }
}

/// Encodes a row direction (radians) as a byte (`× π / 256`, folded into
/// 0 … π: rows have no sense).
pub fn encode_direction(rad: f32) -> u8 {
    let a = rad.rem_euclid(std::f32::consts::PI) / std::f32::consts::PI * 256.0;
    (a.round() as u32 % 256).min(254) as u8
}

/// The vegetation of a package (section `VEGE`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Vegetation {
    /// Cells per tile side.
    pub cells: u32,
    /// Measured sources beyond OSM (crop maps), with their notices.
    pub sources: Vec<ImagerySource>,
    pub tiles: BTreeMap<TileKey, VegetationTile>,
}

impl Vegetation {
    /// The crop and row direction at local (x, z) (package metres).
    pub fn at(&self, tile_size: f64, x: f64, z: f64) -> (FieldCrop, Option<f32>) {
        let key = TileKey::containing([x, z], tile_size);
        let Some(t) = self.tiles.get(&key) else {
            return (FieldCrop::None, None);
        };
        let o = key.origin(tile_size);
        let n = self.cells as usize;
        let s = tile_size / f64::from(self.cells);
        let c = (((x - o[0]) / s).max(0.0) as usize).min(n - 1);
        let r = (((z - o[1]) / s).max(0.0) as usize).min(n - 1);
        t.cell(r * n + c)
    }

    /// Cells per crop and mapped trees per species, for reports.
    pub fn stats(&self) -> (BTreeMap<FieldCrop, usize>, BTreeMap<Species, usize>) {
        let (mut crops, mut trees) = (BTreeMap::new(), BTreeMap::new());
        for t in self.tiles.values() {
            for c in &t.crops {
                if let Some(c) = FieldCrop::from_code(*c).filter(|c| *c != FieldCrop::None) {
                    *crops.entry(c).or_default() += 1;
                }
            }
            for p in &t.trees {
                *trees.entry(p.species).or_default() += 1;
            }
        }
        (crops, trees)
    }
}

fn encode_planes(t: &VegetationTile, cells: u32) -> Vec<u8> {
    if t.crops.is_empty() {
        return Vec::new();
    }
    let n = (cells * cells) as usize;
    assert!(
        t.crops.len() == n && t.rows.len() == n,
        "vegetation planes size"
    );
    let mut raw = t.crops.clone();
    raw.extend(&t.rows);
    miniz_oxide::deflate::compress_to_vec(&raw, 9)
}

fn decode_planes(bytes: &[u8], cells: u32) -> Result<(Vec<u8>, Vec<u8>), String> {
    if bytes.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let n = (cells * cells) as usize;
    let raw = miniz_oxide::inflate::decompress_to_vec_with_limit(bytes, 2 * n)
        .map_err(|e| format!("vegetation tile: {e:?}"))?;
    if raw.len() != 2 * n {
        return Err("vegetation tile: wrong size".into());
    }
    Ok((raw[..n].to_vec(), raw[n..].to_vec()))
}

/// Writes the `VEGE` payload.
pub fn write_vegetation(v: &Vegetation, b: &mut Vec<u8>) {
    b.extend(VEGETATION_VERSION.to_le_bytes());
    b.extend(v.cells.to_le_bytes());
    b.push(v.sources.len() as u8);
    for s in &v.sources {
        for t in [&s.id, &s.name, &s.licence, &s.notice, &s.url, &s.date] {
            put_str(b, t);
        }
        b.extend(s.resolution_m.to_le_bytes());
    }
    b.extend((v.tiles.len() as u32).to_le_bytes());
    for (k, t) in &v.tiles {
        b.extend(k.x.to_le_bytes());
        b.extend(k.z.to_le_bytes());
        let planes = encode_planes(t, v.cells);
        b.extend((planes.len() as u32).to_le_bytes());
        b.extend(planes);
        b.extend((t.trees.len() as u32).to_le_bytes());
        for p in &t.trees {
            b.extend(p.pos[0].to_le_bytes());
            b.extend(p.pos[1].to_le_bytes());
            b.push(p.species.code());
            let dm = (p.height_m * 10.0).round().clamp(0.0, 65535.0) as u16;
            b.extend(dm.to_le_bytes());
            b.push(p.source as u8);
        }
    }
}

/// Reads the `VEGE` payload; an unknown layout is "not supported" (the
/// package reader skips it), unknown species are read as
/// [`Species::Unknown`].
pub fn read_vegetation(bytes: &[u8]) -> Result<Vegetation, String> {
    let mut r = Bytes::new(bytes);
    check_version(&mut r, VEGETATION_VERSION, "vegetation")?;
    let cells = r.u32()?;
    if cells == 0 || cells > 1000 {
        return Err(format!("vegetation: {cells} cells per tile"));
    }
    let mut sources = Vec::new();
    for _ in 0..r.u8()? {
        let mut s = [(); 6].map(|_| String::new());
        for t in &mut s {
            *t = get_str(&mut r)?;
        }
        let [id, name, licence, notice, url, date] = s;
        sources.push(ImagerySource {
            id,
            name,
            licence,
            notice,
            url,
            date,
            resolution_m: r.f32()?,
        });
    }
    let mut tiles = BTreeMap::new();
    for _ in 0..r.u32()? {
        let key = TileKey {
            x: r.i32()?,
            z: r.i32()?,
        };
        let n = r.u32()? as usize;
        let (crops, rows) = decode_planes(r.take(n)?, cells)?;
        let count = r.u32()? as usize;
        let mut trees = Vec::with_capacity(count.min(1 << 20));
        for _ in 0..count {
            let pos = [r.f32()?, r.f32()?];
            let species = Species::from_code(r.u8()?).unwrap_or_default();
            let dm = u16::from_le_bytes(r.arr()?);
            let source = match r.u8()? {
                2 => TreeSource::TreeRow,
                _ => TreeSource::Tree,
            };
            trees.push(PlantedTree {
                pos,
                species,
                height_m: f32::from(dm) / 10.0,
                source,
            });
        }
        tiles.insert(key, VegetationTile { crops, rows, trees });
    }
    r.finish("vegetation")?;
    Ok(Vegetation {
        cells,
        sources,
        tiles,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A tile (0, 0) of 4 × 4 cells: a vineyard in the west column with rows
    /// along +z, maize in the second, the rest no fields; a palm and a tree
    /// of a row of unknown species.
    pub fn sample() -> Vegetation {
        let n = 4usize;
        let mut crops = vec![0u8; n * n];
        let mut rows = vec![NO_ROWS; n * n];
        for r in 0..n {
            crops[r * n] = FieldCrop::Vineyard.code();
            rows[r * n] = encode_direction(std::f32::consts::FRAC_PI_2);
            crops[r * n + 1] = FieldCrop::Maize.code();
            rows[r * n + 1] = 0;
        }
        Vegetation {
            cells: n as u32,
            sources: Vec::new(),
            tiles: BTreeMap::from([
                (
                    TileKey { x: 0, z: 0 },
                    VegetationTile {
                        crops,
                        rows,
                        trees: vec![
                            PlantedTree {
                                pos: [120.5, 340.25],
                                species: Species::DatePalm,
                                height_m: 9.5,
                                source: TreeSource::Tree,
                            },
                            PlantedTree {
                                pos: [800.0, 10.0],
                                species: Species::Unknown,
                                height_m: 0.0,
                                source: TreeSource::TreeRow,
                            },
                        ],
                    },
                ),
                // A tile with trees only.
                (
                    TileKey { x: 1, z: -1 },
                    VegetationTile {
                        crops: Vec::new(),
                        rows: Vec::new(),
                        trees: vec![PlantedTree {
                            pos: [1500.0, -500.0],
                            species: Species::Lime,
                            height_m: 0.0,
                            source: TreeSource::Tree,
                        }],
                    },
                ),
            ]),
        }
    }

    #[test]
    fn section_round_trips_and_rejects_garbage() {
        let v = sample();
        let mut b = Vec::new();
        write_vegetation(&v, &mut b);
        assert_eq!(read_vegetation(&b).unwrap(), v);
        let mut newer = b.clone();
        newer[0] = 9;
        assert!(read_vegetation(&newer).unwrap_err().contains("not supported"));
        assert!(read_vegetation(&b[..b.len() - 2]).is_err());
        assert!(decode_planes(&[1, 2, 3], 4).is_err());
        // Unknown species codes (from a newer builder) read as unknown.
        let mut future = b.clone();
        let at = b.len() - 4; // the lime's species byte: x z species dm dm src
        assert_eq!(b[at], Species::Lime.code());
        future[at] = 250;
        let back = read_vegetation(&future).unwrap();
        assert_eq!(
            back.tiles[&TileKey { x: 1, z: -1 }].trees[0].species,
            Species::Unknown
        );
    }

    #[test]
    fn cells_are_found_by_local_metres() {
        let v = sample();
        let (c, r) = v.at(1000.0, 100.0, 600.0);
        assert_eq!(c, FieldCrop::Vineyard);
        assert!((r.unwrap() - std::f32::consts::FRAC_PI_2).abs() < 0.02);
        assert_eq!(v.at(1000.0, 300.0, 600.0), (FieldCrop::Maize, Some(0.0)));
        assert_eq!(v.at(1000.0, 700.0, 600.0), (FieldCrop::None, None));
        assert_eq!(v.at(1000.0, 1500.0, -500.0), (FieldCrop::None, None));
        assert_eq!(v.at(1000.0, 5000.0, 0.0), (FieldCrop::None, None));
        let (crops, trees) = v.stats();
        assert_eq!(crops[&FieldCrop::Vineyard], 4);
        assert_eq!(trees[&Species::DatePalm], 1);
    }

    #[test]
    fn directions_fold_into_half_a_turn() {
        use std::f32::consts::PI;
        assert_eq!(encode_direction(0.0), 0);
        assert_eq!(encode_direction(PI), 0);
        assert_eq!(encode_direction(-PI / 2.0), encode_direction(PI / 2.0));
        assert_eq!(encode_direction(PI / 2.0), 128);
        assert!(encode_direction(PI - 0.001) <= 254);
    }

    #[test]
    fn osm_crops_are_classified() {
        let f = FieldCrop::from_osm;
        assert_eq!(f(None, None, Some("banana_plants")), Some(FieldCrop::Banana));
        assert_eq!(f(None, Some("banana"), None), Some(FieldCrop::Banana));
        assert_eq!(f(Some("grape"), None, None), Some(FieldCrop::Vineyard));
        assert_eq!(f(None, Some("apple;pear"), None), Some(FieldCrop::Orchard));
        assert_eq!(f(None, Some("mango;orange"), None), Some(FieldCrop::Subtropical));
        assert_eq!(f(None, None, Some("avocado_trees")), Some(FieldCrop::Subtropical));
        assert_eq!(f(Some("maize"), None, None), Some(FieldCrop::Maize));
        assert_eq!(f(Some("Wheat"), None, None), Some(FieldCrop::Cereal));
        assert_eq!(f(Some("native_pasture"), None, None), Some(FieldCrop::Grass));
        assert_eq!(f(Some("live_animal"), None, None), None);
        assert_eq!(f(None, None, None), None);
        // `trees` before `produce` before `crop`.
        assert_eq!(
            f(Some("grass"), None, Some("apple_trees")),
            Some(FieldCrop::Orchard)
        );
        for c in FieldCrop::ALL {
            assert_eq!(FieldCrop::from_code(c.code()), Some(c));
        }
        assert!(FieldCrop::Maize.summer_crop() && !FieldCrop::Cereal.summer_crop());
        assert!(FieldCrop::Vineyard.in_rows() && !FieldCrop::Cereal.in_rows());
    }
}
