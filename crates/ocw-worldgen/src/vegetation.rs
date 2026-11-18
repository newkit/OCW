//! The section `VEGE` (spec §4.6.1, §14 Phase 8 T8): fields with their
//! crop, plantations with their rows and the mapped trees, from OSM.
//!
//! - **Fields and plantations:** every `landuse=farmland|orchard|vineyard|
//!   greenhouse_horticulture|plant_nursery` area is rasterised onto the
//!   section's grid (10 m). Its crop is what OSM says (`trees=*`,
//!   `produce=*`, `crop=*`, [`FieldCrop::from_osm`]), else – for fields – a
//!   measured crop map where the package has one, else the region's crop
//!   mix ([`crop_mix`]) drawn once per parcel: the OSM outline, larger
//!   ones cut into parcels along their main axis ([`parcel_size`]), so the
//!   fields have real outlines. Plantations without a tag get the
//!   region's rule (Tenerife: bananas on the coast, avocados and citrus
//!   higher up, [`plantation_default`]).
//! - **Rows:** each plantation's rows run along its main axis (the
//!   direction of its longest edges, [`main_axis`]); vineyards on slopes
//!   steeper than [`FALL_LINE_SLOPE`] run down the fall line, as on the
//!   steep vineyards of Vaduz and the Ortenau.
//! - **Trees:** `natural=tree` nodes and the trees of `natural=tree_row`
//!   ways (every [`TREE_ROW_SPACING_M`]) with their species (`species` /
//!   `taxon` / `genus`, else `leaf_type`, [`Species::from_osm`]) and height.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use ocw_world::flora::{FloraRegion, OsmLeaf, Species};
use ocw_world::geo::LocalProjection;
use ocw_world::trees::hash;
use ocw_world::vegetation::{
    DEFAULT_CELLS, FieldCrop, NO_ROWS, PlantedTree, TreeSource, Vegetation, VegetationTile,
    encode_direction,
};
use ocw_world::{TileKey, WorldPackage};

use crate::areas::AreaData;
use crate::osm::{OsmError, Tags, par_elements, way_box_overlaps};

/// Vineyards steeper than this (rise over run) have their rows down the
/// fall line.
pub const FALL_LINE_SLOPE: f64 = 0.12;
/// The spacing of the trees of a tree row without its own (m).
pub const TREE_ROW_SPACING_M: f64 = 9.0;

/// Keys kept on tree nodes and tree-row ways.
fn kept(k: &str) -> bool {
    matches!(
        k,
        "natural" | "genus" | "species" | "taxon" | "leaf_type" | "height" | "denotation"
    )
}

/// OSM trees and tree rows.
#[derive(Debug, Clone, Default)]
pub struct TreeData {
    /// Node id → (lat, lon) (tree-row nodes).
    pub nodes: HashMap<i64, (f64, f64)>,
    /// `natural=tree` nodes: position and kept tags.
    pub trees: Vec<(i64, (f64, f64), Tags)>,
    /// `natural=tree_row` ways: node ids and kept tags.
    pub rows: Vec<(i64, Vec<i64>, Tags)>,
}

fn tags_of<'a>(it: impl Iterator<Item = (&'a str, &'a str)>) -> Tags {
    it.filter(|(k, _)| kept(k))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

impl TreeData {
    /// Two passes: the tree-row ways, then the nodes (theirs and the trees).
    pub fn read_pbf(path: &Path) -> Result<Self, OsmError> {
        let rows = par_elements(path, |el, out| {
            if let osmpbf::Element::Way(w) = el
                && w.tags().any(|(k, v)| k == "natural" && v == "tree_row")
            {
                out.push((w.id(), w.refs().collect::<Vec<_>>(), tags_of(w.tags())));
            }
        })?;
        let needed: HashSet<i64> = rows.iter().flat_map(|w| w.1.iter().copied()).collect();
        let found = par_elements(path, |el, out| {
            let (id, lat, lon, tags): (i64, f64, f64, Vec<(&str, &str)>) = match el {
                osmpbf::Element::Node(n) => (n.id(), n.lat(), n.lon(), n.tags().collect()),
                osmpbf::Element::DenseNode(n) => (n.id(), n.lat(), n.lon(), n.tags().collect()),
                _ => return,
            };
            let tree = tags.iter().any(|(k, v)| *k == "natural" && *v == "tree");
            if tree || needed.contains(&id) {
                out.push((id, (lat, lon), tree.then(|| tags_of(tags.into_iter()))));
            }
        })?;
        let mut d = Self {
            rows,
            ..Default::default()
        };
        for (id, p, tags) in found {
            match tags {
                Some(t) => d.trees.push((id, p, t)),
                None => {
                    d.nodes.insert(id, p);
                }
            }
        }
        Ok(d)
    }

    /// Reads several extracts and merges them (by id).
    pub fn read_pbfs(paths: &[PathBuf]) -> Result<Self, String> {
        let mut out = Self::default();
        let (mut rows, mut trees) = (HashSet::new(), HashSet::new());
        for p in paths {
            let d = Self::read_pbf(p).map_err(|e| format!("{}: {e}", p.display()))?;
            out.nodes.extend(d.nodes);
            out.rows
                .extend(d.rows.into_iter().filter(|w| rows.insert(w.0)));
            out.trees
                .extend(d.trees.into_iter().filter(|t| trees.insert(t.0)));
        }
        Ok(out)
    }

    /// Keeps what lies in (south, west, north, east).
    pub fn retain_near(&mut self, bounds: (f64, f64, f64, f64)) {
        let nodes = &self.nodes;
        self.rows
            .retain(|w| way_box_overlaps(w.1.iter().filter_map(|n| nodes.get(n)), bounds));
        self.trees
            .retain(|t| way_box_overlaps(std::iter::once(&t.1), bounds));
        let used: HashSet<i64> = self.rows.iter().flat_map(|w| w.1.iter().copied()).collect();
        self.nodes.retain(|id, _| used.contains(id));
    }
}

/// The species of an OSM tree: by its scientific name, else its leaf type
/// in the region (palms: date palms on Tenerife – most mapped palms there
/// are *Phoenix canariensis* – slender palms elsewhere; needle trees: the
/// region's common conifer), else unknown (the renderer's rules decide).
pub fn tree_species(tags: &Tags, region: FloraRegion, pick: f32) -> Species {
    let t = |k: &str| tags.get(k).map(String::as_str);
    if let Some(s) = Species::from_osm(t("genus"), t("species"), t("taxon")) {
        return s;
    }
    match (t("leaf_type").and_then(OsmLeaf::parse), region) {
        (Some(OsmLeaf::Palm), FloraRegion::Tenerife) if pick < 0.7 => Species::DatePalm,
        (Some(OsmLeaf::Palm), _) => Species::SlenderPalm,
        (Some(OsmLeaf::Needleleaved), FloraRegion::Tenerife) => Species::CanaryPine,
        (Some(OsmLeaf::Needleleaved), _) if pick < 0.7 => Species::NorwaySpruce,
        (Some(OsmLeaf::Needleleaved), _) => Species::ScotsPine,
        _ => Species::Unknown,
    }
}

/// A height tag in metres ("12", "12 m", "8,5").
fn height_tag(tags: &Tags) -> f32 {
    tags.get("height")
        .and_then(|v| {
            v.trim()
                .trim_end_matches('m')
                .trim()
                .replace(',', ".")
                .parse::<f32>()
                .ok()
        })
        .filter(|h| (1.0..80.0).contains(h))
        .unwrap_or(0.0)
}

/// The share of each arable crop on fields without a crop tag, by region
/// and altitude (spec §4.6.1 table 4; proposals from the agricultural
/// statistics).
pub fn crop_mix(region: FloraRegion, alt_m: f64) -> &'static [(FieldCrop, f32)] {
    use FieldCrop::*;
    match region {
        FloraRegion::Tenerife if alt_m < 300.0 => &[
            (Vegetables, 0.55),
            (Grass, 0.15),
            (Fallow, 0.2),
            (Potato, 0.1),
        ],
        FloraRegion::Tenerife => &[
            (Potato, 0.42),
            (Vegetables, 0.2),
            (Fallow, 0.2),
            (Grass, 0.1),
            (Maize, 0.08),
        ],
        _ if alt_m > 900.0 => &[(Grass, 0.7), (Cereal, 0.2), (Potato, 0.1)],
        FloraRegion::AlpineRhine => &[
            (Maize, 0.38),
            (Cereal, 0.3),
            (Grass, 0.18),
            (Vegetables, 0.06),
            (Potato, 0.04),
            (Rapeseed, 0.04),
        ],
        FloraRegion::BlackForest | FloraRegion::Other => &[
            (Maize, 0.4),
            (Cereal, 0.32),
            (Grass, 0.1),
            (Rapeseed, 0.04),
            (Sunflower, 0.03),
            (Vegetables, 0.05),
            (Potato, 0.02),
            (Beet, 0.01),
            (Fallow, 0.03),
        ],
    }
}

/// The parcels big fields are cut into: (width across the main axis,
/// length along it) in metres (spec §4.6.1: Central European fields of
/// one to three hectares, Tenerife's terraces much smaller).
pub fn parcel_size(region: FloraRegion) -> (f64, f64) {
    match region {
        FloraRegion::Tenerife => (30.0, 70.0),
        _ => (55.0, 220.0),
    }
}

/// The crop of a plantation OSM does not tag, by `landuse` and region at
/// `alt_m`: Tenerife's orchards are bananas on the coast (below 350 m),
/// avocados and citrus up to 900 m, fruit trees above; its greenhouses on
/// the coast are banana houses (netting); elsewhere orchards are fruit
/// trees, greenhouses vegetables. `None` for fields (the crop mix decides).
pub fn plantation_default(landuse: &str, region: FloraRegion, alt_m: f64) -> Option<FieldCrop> {
    let tf = region == FloraRegion::Tenerife;
    Some(match landuse {
        "vineyard" => FieldCrop::Vineyard,
        "orchard" if tf && alt_m < 350.0 => FieldCrop::Banana,
        "orchard" if tf && alt_m < 900.0 => FieldCrop::Subtropical,
        "orchard" => FieldCrop::Orchard,
        "greenhouse_horticulture" if tf && alt_m < 400.0 => FieldCrop::Banana,
        "greenhouse_horticulture" => FieldCrop::Greenhouse,
        "plant_nursery" => FieldCrop::Vegetables,
        _ => return None,
    })
}

fn choose(mix: &[(FieldCrop, f32)], pick: f32) -> FieldCrop {
    let sum: f32 = mix.iter().map(|m| m.1).sum();
    let mut at = pick.clamp(0.0, 0.999_999) * sum;
    for &(c, w) in mix {
        if at < w {
            return c;
        }
        at -= w;
    }
    mix.last().map_or(FieldCrop::None, |m| m.0)
}

/// The main axis of a ring (radians from +x towards +z, 0 … π): the
/// direction of its edges weighted by their length (angles doubled, so
/// opposite edges agree) – a field's long sides for a rectangle.
pub fn main_axis(ring: &[[f64; 2]]) -> f64 {
    let (mut c, mut s) = (0.0, 0.0);
    let n = ring.len();
    for j in 0..n {
        let (a, b) = (ring[j], ring[(j + 1) % n]);
        let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
        let len2 = dx * dx + dz * dz;
        if len2 <= 0.0 {
            continue;
        }
        let th = dz.atan2(dx);
        // Weighted by the length squared: the long sides win.
        c += len2 * (2.0 * th).cos();
        s += len2 * (2.0 * th).sin();
    }
    (s.atan2(c) / 2.0).rem_euclid(std::f64::consts::PI)
}

/// The fall line at `p` (radians, 0 … π) if the ground there is steeper
/// than [`FALL_LINE_SLOPE`] (mean over a 40 m cross).
pub fn fall_line(pkg: &WorldPackage, p: [f64; 2]) -> Option<f64> {
    let h = |x: f64, z: f64| pkg.height(x, z);
    let d = 20.0;
    let (gx, gz) = (
        (h(p[0] + d, p[1])? - h(p[0] - d, p[1])?) / (2.0 * d),
        (h(p[0], p[1] + d)? - h(p[0], p[1] - d)?) / (2.0 * d),
    );
    ((gx * gx + gz * gz).sqrt() > FALL_LINE_SLOPE)
        .then(|| gz.atan2(gx).rem_euclid(std::f64::consts::PI))
}

#[derive(Debug, Clone, serde::Deserialize)]
struct CropIndex {
    kind: String,
    crs: String,
    pixel_m: f64,
    west: f64,
    north: f64,
    width: usize,
    height: usize,
    crops: String,
    sources: Vec<CropSource>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct CropSource {
    id: String,
    name: String,
    licence: String,
    notice: String,
    url: String,
    date: String,
}

/// A measured crop map of `scripts/fetch-crops.sh` (`scripts/crops.py`):
/// OCW crop codes ([`FieldCrop`]) on a raster in a CRS, rows from the
/// north (0 none, 255 no data).
#[derive(Debug, Clone)]
pub struct CropRaster {
    pub sources: Vec<ocw_world::look::ImagerySource>,
    pub crs: crate::crs::Crs,
    pub pixel_m: f64,
    pub west: f64,
    pub north: f64,
    pub width: usize,
    pub height: usize,
    pub codes: Vec<u8>,
}

impl CropRaster {
    /// Reads `dir/index.json` and its raster.
    pub fn load(dir: &Path) -> Result<Self, String> {
        let p = dir.join("index.json");
        let text = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
        let ix: CropIndex =
            serde_json::from_str(&text).map_err(|e| format!("{}: {e}", p.display()))?;
        if ix.kind != "crops" {
            return Err(format!("{}: kind {} is not crops", p.display(), ix.kind));
        }
        let path = dir.join(&ix.crops);
        let codes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if codes.len() != ix.width * ix.height {
            return Err(format!(
                "{}: {} bytes, expected {} × {}",
                path.display(),
                codes.len(),
                ix.width,
                ix.height
            ));
        }
        Ok(Self {
            sources: ix
                .sources
                .iter()
                .map(|s| ocw_world::look::ImagerySource {
                    id: s.id.clone(),
                    name: s.name.clone(),
                    licence: s.licence.clone(),
                    notice: s.notice.clone(),
                    url: s.url.clone(),
                    date: s.date.clone(),
                    resolution_m: ix.pixel_m as f32,
                })
                .collect(),
            crs: crate::crs::Crs::parse(&ix.crs)?,
            pixel_m: ix.pixel_m,
            west: ix.west,
            north: ix.north,
            width: ix.width,
            height: ix.height,
            codes,
        })
    }

    /// The crop at CRS `(e, n)` (`None`: no crop, no data or outside).
    pub fn crop(&self, e: f64, n: f64) -> Option<FieldCrop> {
        let fx = (e - self.west) / self.pixel_m;
        let fy = (self.north - n) / self.pixel_m;
        if !(fx >= 0.0 && fy >= 0.0) {
            return None;
        }
        let (x, y) = (fx as usize, fy as usize);
        if x >= self.width || y >= self.height {
            return None;
        }
        FieldCrop::from_code(self.codes[y * self.width + x]).filter(|c| *c != FieldCrop::None)
    }

    /// The crop at local metres `p` of a package projected by `proj`.
    pub fn at(&self, proj: &LocalProjection, p: [f64; 2]) -> Option<FieldCrop> {
        let (lat, lon) = proj.unproject(p);
        let [e, n] = self.crs.forward(lat, lon);
        self.crop(e, n)
    }
}

/// The land classes a measured crop may lie on outside OSM's fields (the
/// crop map knows plantations and fields OSM does not map): open land, and
/// for plantations also scrub and bare ground (abandoned terraces).
fn takes_measured(class: Option<ocw_world::landuse::LandClass>, crop: FieldCrop) -> bool {
    use ocw_world::landuse::LandClass::*;
    match class {
        Some(Default | Meadow | Farmland) | None => true,
        Some(Orchard | Vineyard | Scrub | Bare) => !crop.arable(),
        _ => false,
    }
}

/// The second pass of a measured crop map: its crops on the cells no OSM
/// field covers (open land only, [`takes_measured`]), rows across the
/// slope (terraces) or, for vineyards, down the fall line; directions per
/// 50 m block so neighbouring rows agree. Returns the cells set.
fn fill_measured(
    pkg: &WorldPackage,
    tiles: &mut BTreeMap<TileKey, VegetationTile>,
    raster: &CropRaster,
    proj: &LocalProjection,
) -> usize {
    let size = f64::from(pkg.tile_size_m);
    let n = DEFAULT_CELLS as usize;
    let cs = size / n as f64;
    let keys: Vec<TileKey> = pkg.tiles.keys().copied().collect();
    let filled = crate::par::map(&keys, crate::par::threads(), |key| {
        let aff = crate::imagery::TileAffine::new(proj, raster.crs, *key, size);
        let o = key.origin(size);
        let have = tiles.get(key);
        let mut out: Vec<(usize, u8, u8)> = Vec::new();
        for r in 0..n {
            for c in 0..n {
                let i = r * n + c;
                if have.is_some_and(|t| !t.crops.is_empty() && t.crops[i] != 0) {
                    continue;
                }
                let p = [o[0] + (c as f64 + 0.5) * cs, o[1] + (r as f64 + 0.5) * cs];
                let [e, nn] = aff.apply(p);
                let Some(crop) = raster.crop(e, nn) else {
                    continue;
                };
                if !takes_measured(class_at(pkg, p[0], p[1]), crop) {
                    continue;
                }
                let block = [
                    (p[0] / 50.0).floor() * 50.0 + 25.0,
                    (p[1] / 50.0).floor() * 50.0 + 25.0,
                ];
                let dir = if !crop.in_rows() {
                    NO_ROWS
                } else {
                    let fall = fall_line(pkg, block);
                    let a = match (crop, fall) {
                        (FieldCrop::Vineyard, Some(f)) => f,
                        // Terraces: rows along the contour.
                        (_, Some(f)) => f + std::f64::consts::FRAC_PI_2,
                        (_, None) => 0.0,
                    };
                    encode_direction(a as f32)
                };
                out.push((i, crop.code(), dir));
            }
        }
        out
    });
    let mut count = 0;
    for (key, cells) in keys.iter().zip(filled) {
        if cells.is_empty() {
            continue;
        }
        let t = tiles.entry(*key).or_default();
        if t.crops.is_empty() {
            t.crops = vec![0; n * n];
            t.rows = vec![NO_ROWS; n * n];
        }
        for (i, c, d) in cells {
            t.crops[i] = c;
            t.rows[i] = d;
            count += 1;
        }
    }
    count
}

/// A field or plantation, projected.
struct Field {
    id: i64,
    rings: Vec<Vec<[f64; 2]>>,
    landuse: String,
    /// The crop OSM tags.
    tagged: Option<FieldCrop>,
    /// Drawn over fields (plantations win where they overlap).
    priority: u8,
}

/// What [`add_vegetation`] did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VegetationStats {
    pub fields: usize,
    pub tagged: usize,
    pub measured_cells: usize,
    pub cells: BTreeMap<FieldCrop, usize>,
    pub trees: BTreeMap<Species, usize>,
    pub tree_rows: usize,
}

impl VegetationStats {
    pub fn summary(&self) -> String {
        let ha = |n: usize| n as f64 * 0.01; // 10 m cells: 100 m² each
        let crops: Vec<String> = self
            .cells
            .iter()
            .map(|(c, n)| format!("{} {:.0} ha", c.name(), ha(*n)))
            .collect();
        let mut trees: Vec<(&Species, &usize)> = self.trees.iter().collect();
        trees.sort_by_key(|t| std::cmp::Reverse(*t.1));
        let trees: Vec<String> = trees
            .iter()
            .take(8)
            .map(|(s, n)| format!("{n} {}", s.name()))
            .collect();
        format!(
            "{} fields and plantations ({} with an OSM crop, {} cells from a crop map): {}; {} mapped trees ({} tree rows): {}",
            self.fields,
            self.tagged,
            self.measured_cells,
            crops.join(", "),
            self.trees.values().sum::<usize>(),
            self.tree_rows,
            trees.join(", ")
        )
    }
}

/// Builds the section from the OSM areas and trees (and a measured crop
/// map for fields without a crop tag).
pub fn build_vegetation(
    pkg: &WorldPackage,
    areas: &AreaData,
    trees: &TreeData,
    proj: &LocalProjection,
    crops: Option<&CropRaster>,
) -> (Vegetation, VegetationStats) {
    let region = FloraRegion::at(pkg.origin.0, pkg.origin.1);
    let mut st = VegetationStats::default();
    let project = |ring: &Vec<(f64, f64)>| -> Vec<[f64; 2]> {
        ring.iter()
            .map(|&(lat, lon)| proj.project(lat, lon))
            .collect()
    };
    let mut fields: Vec<Field> = areas
        .areas()
        .into_iter()
        .filter_map(|a| {
            let t = |k: &str| a.tags.get(k).map(String::as_str);
            let landuse = t("landuse")?;
            let priority = match landuse {
                "farmland" => 1,
                "orchard" | "vineyard" | "greenhouse_horticulture" | "plant_nursery" => 2,
                _ => return None,
            };
            Some(Field {
                id: a.id,
                rings: a.outer.iter().chain(&a.inner).map(project).collect(),
                landuse: landuse.to_string(),
                tagged: FieldCrop::from_osm(t("crop"), t("produce"), t("trees")),
                priority,
            })
        })
        .collect();
    fields.sort_by_key(|f| (f.priority, f.id));
    let size = f64::from(pkg.tile_size_m);
    let cells = DEFAULT_CELLS;
    let n = cells as usize;
    let cs = size / f64::from(cells);
    let mut tiles: BTreeMap<TileKey, VegetationTile> = BTreeMap::new();
    for f in &fields {
        let outer = &f.rings[0];
        let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for p in outer {
            lo = [lo[0].min(p[0]), lo[1].min(p[1])];
            hi = [hi[0].max(p[0]), hi[1].max(p[1])];
        }
        let centre = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0];
        let Some(alt) = pkg.height(centre[0], centre[1]) else {
            continue;
        };
        st.fields += 1;
        st.tagged += usize::from(f.tagged.is_some());
        let axis = main_axis(outer);
        let fall = fall_line(pkg, centre);
        let plantation = plantation_default(&f.landuse, region, alt);
        // The rows of a crop on this field: vineyards down the fall line on
        // slopes, the rest along the field's main axis.
        let rows_of = |crop: FieldCrop| match crop {
            FieldCrop::Vineyard => encode_direction(fall.unwrap_or(axis) as f32),
            c if c.in_rows() => encode_direction(axis as f32),
            _ => NO_ROWS,
        };
        let (d, nrm) = ([axis.cos(), axis.sin()], [-axis.sin(), axis.cos()]);
        let (pw, pl) = parcel_size(region);
        let mix = crop_mix(region, alt);
        let crop_at = |p: [f64; 2]| -> FieldCrop {
            if let Some(c) = plantation {
                return c;
            }
            // A parcel of the field: its crop by the region's mix.
            let a = ((p[0] * nrm[0] + p[1] * nrm[1]) / pw).floor() as i64;
            let b = ((p[0] * d[0] + p[1] * d[1]) / pl).floor() as i64;
            choose(mix, hash(f.id.wrapping_mul(31).wrapping_add(a), b, 41))
        };
        // Scanlines over the cell centres in the field's box.
        let first = |v: f64| ((v / cs) - 0.5).ceil() as i64;
        let last = |v: f64| ((v / cs) - 0.5).floor() as i64;
        for gz in first(lo[1])..=last(hi[1]) {
            let z = (gz as f64 + 0.5) * cs;
            let mut xs: Vec<f64> = Vec::new();
            for ring in &f.rings {
                let m = ring.len();
                for j in 0..m {
                    let (a, b) = (ring[j], ring[(j + 1) % m]);
                    if (a[1] <= z) != (b[1] <= z) {
                        xs.push(a[0] + (z - a[1]) / (b[1] - a[1]) * (b[0] - a[0]));
                    }
                }
            }
            xs.sort_by(f64::total_cmp);
            for span in xs.as_chunks::<2>().0 {
                for gx in first(span[0])..=last(span[1]) {
                    let x = (gx as f64 + 0.5) * cs;
                    if x < span[0] || x >= span[1] {
                        continue;
                    }
                    let key = TileKey::containing([x, z], size);
                    if !pkg.tiles.contains_key(&key) {
                        continue;
                    }
                    // OSM's tag, else the measured map, else the region's
                    // rule (plantations) or its crop mix (fields).
                    let crop = match (f.tagged, crops.and_then(|c| c.at(proj, [x, z]))) {
                        (Some(c), _) => c,
                        (None, Some(m)) => {
                            st.measured_cells += 1;
                            m
                        }
                        (None, None) => crop_at([x, z]),
                    };
                    let t = tiles.entry(key).or_default();
                    if t.crops.is_empty() {
                        t.crops = vec![0; n * n];
                        t.rows = vec![NO_ROWS; n * n];
                    }
                    let o = key.origin(size);
                    let c = (((x - o[0]) / cs) as usize).min(n - 1);
                    let r = (((z - o[1]) / cs) as usize).min(n - 1);
                    let i = r * n + c;
                    t.crops[i] = crop.code();
                    t.rows[i] = rows_of(crop);
                }
            }
        }
    }
    // The measured crops beyond OSM's fields.
    if let Some(r) = crops {
        st.measured_cells += fill_measured(pkg, &mut tiles, r, proj);
    }
    for t in tiles.values() {
        for c in &t.crops {
            if let Some(c) = FieldCrop::from_code(*c).filter(|c| *c != FieldCrop::None) {
                *st.cells.entry(c).or_default() += 1;
            }
        }
    }
    // The mapped trees.
    let mut put = |pos: [f64; 2], tags: &Tags, source: TreeSource, pick: f32| {
        let key = TileKey::containing(pos, size);
        if !pkg.tiles.contains_key(&key) {
            return;
        }
        let species = tree_species(tags, region, pick);
        *st.trees.entry(species).or_default() += 1;
        tiles.entry(key).or_default().trees.push(PlantedTree {
            pos: [pos[0] as f32, pos[1] as f32],
            species,
            height_m: height_tag(tags),
            source,
        });
    };
    for (id, (lat, lon), tags) in &trees.trees {
        put(
            proj.project(*lat, *lon),
            tags,
            TreeSource::Tree,
            hash(*id, 0, 43),
        );
    }
    for (id, nodes, tags) in &trees.rows {
        let pts: Vec<[f64; 2]> = nodes
            .iter()
            .filter_map(|n| trees.nodes.get(n))
            .map(|&(lat, lon)| proj.project(lat, lon))
            .collect();
        st.tree_rows += 1;
        let mut along = TREE_ROW_SPACING_M / 2.0;
        let mut k = 0i64;
        for w in pts.windows(2) {
            let (a, b) = (w[0], w[1]);
            let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
            while along <= len {
                let t = along / len;
                put(
                    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t],
                    tags,
                    TreeSource::TreeRow,
                    hash(*id, k, 43),
                );
                along += TREE_ROW_SPACING_M;
                k += 1;
            }
            along -= len;
        }
    }
    for t in tiles.values_mut() {
        t.trees.sort_by(|a, b| {
            a.pos[0]
                .total_cmp(&b.pos[0])
                .then(a.pos[1].total_cmp(&b.pos[1]))
        });
    }
    tiles.retain(|_, t| !t.is_empty());
    (
        Vegetation {
            cells,
            sources: crops.map(|r| r.sources.clone()).unwrap_or_default(),
            tiles,
        },
        st,
    )
}

/// Adds the section to a package (replacing one it has) and renews the
/// trees of `OBST` (the grid leaves plantations and mapped trees' cells,
/// the solid planted trees become point obstacles).
pub fn add_vegetation(
    pkg: &mut WorldPackage,
    areas: &AreaData,
    trees: &TreeData,
    proj: &LocalProjection,
    crops: Option<&CropRaster>,
) -> VegetationStats {
    let (v, st) = build_vegetation(pkg, areas, trees, proj, crops);
    pkg.look.vegetation = Some(v);
    crate::obstacles::refresh_trees(pkg);
    st
}

/// A place to look at the vegetation (scenes, checks): a name, a point
/// beside a road (package metres), its altitude and how much of what the
/// spot looks for lies within 80 m (cells of 20 m, at most 81).
#[derive(Debug, Clone, PartialEq)]
pub struct Spot {
    pub name: &'static str,
    pub at: [f64; 2],
    pub alt_m: f64,
    pub score: u32,
}

type SpotTest<'a> = Box<dyn Fn(f64, f64, f64) -> bool + Sync + 'a>;

fn class_at(pkg: &WorldPackage, x: f64, z: f64) -> Option<ocw_world::landuse::LandClass> {
    let size = f64::from(pkg.tile_size_m);
    pkg.landuse.as_ref().and_then(|l| l.class_at(size, x, z))
}

fn crop_test(pkg: &WorldPackage, c: FieldCrop) -> SpotTest<'_> {
    let size = f64::from(pkg.tile_size_m);
    Box::new(move |x, z, _| {
        pkg.look
            .vegetation
            .as_ref()
            .is_some_and(|v| v.at(size, x, z).0 == c)
    })
}

fn forest_test(
    pkg: &WorldPackage,
    lo: f64,
    hi: f64,
    b: std::ops::RangeInclusive<u8>,
) -> SpotTest<'_> {
    let size = f64::from(pkg.tile_size_m);
    Box::new(move |x, z, alt| {
        (lo..hi).contains(&alt)
            && class_at(pkg, x, z) == Some(ocw_world::landuse::LandClass::Forest)
            && pkg
                .look
                .forest
                .as_ref()
                .and_then(|f| f.cell(size, x, z))
                .is_some_and(|c| c.density >= 50 && c.broadleaf.is_some_and(|s| b.contains(&s)))
    })
}

/// The best places beside the roads to see each kind of vegetation of the
/// package: plantations by crop, fields, palms in towns, forests by leaf
/// type and altitude, the tree line, the Teide's broom.
pub fn spots(pkg: &WorldPackage) -> Vec<Spot> {
    use ocw_world::landuse::LandClass;
    let region = FloraRegion::at(pkg.origin.0, pkg.origin.1);
    let size = f64::from(pkg.tile_size_m);
    let crop = |c: FieldCrop| crop_test(pkg, c);
    let class = move |x: f64, z: f64| class_at(pkg, x, z);
    let forest = |lo: f64, hi: f64, b: std::ops::RangeInclusive<u8>| forest_test(pkg, lo, hi, b);
    let palms: SpotTest<'_> = Box::new(move |x, z, _| {
        let key = TileKey::containing([x, z], size);
        pkg.look
            .vegetation
            .as_ref()
            .and_then(|v| v.tiles.get(&key))
            .is_some_and(|t| {
                t.trees.iter().any(|p| {
                    matches!(p.species, Species::DatePalm | Species::SlenderPalm)
                        && (f64::from(p.pos[0]) - x).abs() < 10.0
                        && (f64::from(p.pos[1]) - z).abs() < 10.0
                })
            })
    });
    let line = region.tree_line_m();
    let mut tests: Vec<(&'static str, SpotTest<'_>)> = vec![
        ("vineyard", crop(FieldCrop::Vineyard)),
        ("orchard", crop(FieldCrop::Orchard)),
        ("banana", crop(FieldCrop::Banana)),
        ("avocado-citrus", crop(FieldCrop::Subtropical)),
        ("maize", crop(FieldCrop::Maize)),
        ("cereal", crop(FieldCrop::Cereal)),
        ("palms", palms),
        ("forest-conifer-low", forest(f64::MIN, 900.0, 0..=20)),
        ("forest-conifer-high", forest(900.0, 2400.0, 0..=20)),
        ("forest-mixed", forest(600.0, 1400.0, 35..=65)),
        ("forest-broadleaf", forest(f64::MIN, 1300.0, 75..=100)),
        (
            "tree-line",
            Box::new(move |x, z, alt| {
                (line - 250.0..line + 50.0).contains(&alt) && class(x, z) == Some(LandClass::Forest)
            }),
        ),
    ];
    if region == FloraRegion::Tenerife {
        tests.push((
            "broom",
            Box::new(move |x, z, alt| {
                (1900.0..2600.0).contains(&alt) && class(x, z) == Some(LandClass::Scrub)
            }),
        ));
        tests.push((
            "succulent-scrub",
            Box::new(move |x, z, alt| alt < 600.0 && class(x, z) == Some(LandClass::Scrub)),
        ));
    }
    let roads = crate::obstacles::StripIndex::new(pkg);
    // Sample points every 40 m within 25 m of a road.
    let mut samples: Vec<([f64; 2], f64)> = Vec::new();
    for key in pkg.tiles.keys() {
        let o = key.origin(size);
        let n = (size / 40.0) as i64;
        for k in 0..n {
            for i in 0..n {
                let p = [
                    o[0] + (i as f64 + 0.5) * 40.0,
                    o[1] + (k as f64 + 0.5) * 40.0,
                ];
                if roads.nearest(p, 25.0).is_some()
                    && let Some(y) = pkg.height(p[0], p[1])
                {
                    samples.push((p, y));
                }
            }
        }
    }
    let mut out = Vec::new();
    for (name, test) in &tests {
        let scored = crate::par::map(&samples, crate::par::threads(), |(p, _)| {
            let mut n = 0u32;
            for dz in -4..=4 {
                for dx in -4..=4 {
                    let (x, z) = (p[0] + f64::from(dx) * 20.0, p[1] + f64::from(dz) * 20.0);
                    if let Some(y) = pkg.height(x, z)
                        && test(x, z, y)
                    {
                        n += 1;
                    }
                }
            }
            n
        });
        let mut best: Vec<(u32, usize)> = scored
            .iter()
            .enumerate()
            .filter(|(_, n)| **n >= 12)
            .map(|(i, n)| (*n, i))
            .collect();
        best.sort_by_key(|b| (std::cmp::Reverse(b.0), b.1));
        let mut picked: Vec<[f64; 2]> = Vec::new();
        for (score, i) in best {
            let (p, y) = samples[i];
            // Spots at least 1.5 km apart.
            if picked
                .iter()
                .any(|q| (q[0] - p[0]).hypot(q[1] - p[1]) < 1500.0)
            {
                continue;
            }
            picked.push(p);
            out.push(Spot {
                name,
                at: p,
                alt_m: y,
                score,
            });
            if picked.len() == 3 {
                break;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::areas::{Relation, Role};
    use crate::osm::Way;
    use std::collections::BTreeMap;

    /// A flat 2 × 1 km package near Vaduz (`origin`), heights 450 m with a
    /// slope rising 30 % towards +x in x 1 200 … 1 800.
    fn package(origin: (f64, f64)) -> WorldPackage {
        let n = 21u32;
        let mut tiles = BTreeMap::new();
        for x in 0..2 {
            let key = TileKey { x, z: 0 };
            let mut heights = Vec::new();
            for _k in 0..n {
                for i in 0..n {
                    let gx = f64::from(x) * 1000.0 + f64::from(i) * 50.0;
                    let h = 450.0 + 0.3 * (gx - 1200.0).clamp(0.0, 600.0);
                    heights.push(h as f32);
                }
            }
            tiles.insert(
                key,
                ocw_world::Tile {
                    key,
                    heights,
                    roads: Vec::new(),
                },
            );
        }
        WorldPackage {
            name: "Fields".into(),
            origin,
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
        }
    }

    fn tags(pairs: &[(&str, &str)]) -> Tags {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// Areas: a big untagged field (x 0 … 600, z 100 … 400), a vineyard on
    /// the slope (x 1 300 … 1 400, z 100 … 700, long along z), an orchard
    /// tagged with apples (x 700 … 900, z 600 … 700), a banana field.
    fn areas(proj: &LocalProjection) -> AreaData {
        let mut d = AreaData::default();
        let mut nid = 1i64;
        let mut ring = |d: &mut AreaData, id: i64, pts: &[[f64; 2]], t: Tags| {
            let mut ids = Vec::new();
            for p in pts {
                let (lat, lon) = proj.unproject(*p);
                d.nodes.insert(nid, (lat, lon));
                ids.push(nid);
                nid += 1;
            }
            ids.push(ids[0]);
            d.ways.insert(
                id,
                Way {
                    id,
                    nodes: ids,
                    tags: t,
                },
            );
        };
        let rect = |x0: f64, z0: f64, x1: f64, z1: f64| [[x0, z0], [x1, z0], [x1, z1], [x0, z1]];
        ring(
            &mut d,
            10,
            &rect(0.0, 100.0, 600.0, 400.0),
            tags(&[("landuse", "farmland")]),
        );
        ring(
            &mut d,
            11,
            &rect(1300.0, 100.0, 1400.0, 700.0),
            tags(&[("landuse", "vineyard")]),
        );
        ring(
            &mut d,
            12,
            &rect(700.0, 600.0, 900.0, 700.0),
            tags(&[("landuse", "orchard"), ("trees", "apple_trees")]),
        );
        ring(
            &mut d,
            13,
            &rect(100.0, 700.0, 300.0, 900.0),
            tags(&[("landuse", "farmland"), ("crop", "maize")]),
        );
        // A relation without members is skipped.
        d.relations.push(Relation {
            id: 99,
            tags: tags(&[("type", "multipolygon"), ("landuse", "farmland")]),
            members: vec![(12345, Role::Outer)],
        });
        d
    }

    fn trees(proj: &LocalProjection) -> TreeData {
        let mut t = TreeData::default();
        let at = |p: [f64; 2]| proj.unproject(p);
        t.trees.push((
            1,
            at([500.0, 50.0]),
            tags(&[
                ("natural", "tree"),
                ("species", "Tilia cordata"),
                ("height", "14 m"),
            ]),
        ));
        t.trees.push((
            2,
            at([520.0, 50.0]),
            tags(&[("natural", "tree"), ("leaf_type", "palm")]),
        ));
        t.trees
            .push((3, at([5000.0, 50.0]), tags(&[("natural", "tree")]))); // outside
        for (i, x) in [100.0, 190.0].iter().enumerate() {
            t.nodes.insert(100 + i as i64, at([*x, 950.0]));
        }
        t.rows.push((
            7,
            vec![100, 101],
            tags(&[("natural", "tree_row"), ("genus", "Populus")]),
        ));
        t
    }

    #[test]
    fn fields_get_crops_rows_and_trees() {
        let pkg = package((47.14, 9.52));
        let proj = LocalProjection::new(47.14, 9.52);
        let (v, st) = build_vegetation(&pkg, &areas(&proj), &trees(&proj), &proj, None);
        assert_eq!(st.fields, 4);
        assert_eq!(st.tagged, 2);
        let at = |x: f64, z: f64| v.at(1000.0, x, z);
        // The tagged orchard: rows along its long side (+x).
        let (c, r) = at(800.0, 650.0);
        assert_eq!(c, FieldCrop::Orchard);
        assert!(r.unwrap().abs() < 0.02 || (r.unwrap() - std::f32::consts::PI).abs() < 0.02);
        // The vineyard on the 30 % slope: rows down the fall line (+x).
        let (c, r) = at(1350.0, 400.0);
        assert_eq!(c, FieldCrop::Vineyard);
        assert!(r.unwrap().abs() < 0.02, "{r:?}");
        // Maize tagged: maize with rows along the axis.
        assert_eq!(at(200.0, 800.0).0, FieldCrop::Maize);
        // The big untagged field: several parcels with the region's crops,
        // each parcel one crop.
        let mut seen = std::collections::BTreeSet::new();
        for i in 0..60 {
            for k in 0..30 {
                seen.insert(at(5.0 + f64::from(i) * 10.0, 105.0 + f64::from(k) * 10.0).0);
            }
        }
        assert!(seen.len() >= 2, "{seen:?}");
        assert!(seen.iter().all(|c| c.arable() || *c == FieldCrop::Grass));
        // Outside the fields: none.
        assert_eq!(at(650.0, 50.0).0, FieldCrop::None);
        // Trees: the lime with its height, a palm, the poplar row (every
        // 9 m on 90 m: 10 trees), none outside the package.
        let all: Vec<&PlantedTree> = v.tiles.values().flat_map(|t| &t.trees).collect();
        assert_eq!(all.len(), 2 + 10);
        let lime = all.iter().find(|t| t.species == Species::Lime).unwrap();
        assert!((lime.height_m - 14.0).abs() < 1e-6);
        assert!(all.iter().any(|t| t.species == Species::SlenderPalm));
        assert_eq!(
            all.iter().filter(|t| t.species == Species::Willow).count(),
            10
        );
        assert_eq!(st.tree_rows, 1);
        // Deterministic.
        let (again, _) = build_vegetation(&pkg, &areas(&proj), &trees(&proj), &proj, None);
        assert_eq!(again, v);
    }

    /// A crop raster in LAEA around `proj`'s origin: sunflowers west of
    /// local x 300, bananas in the box x 1 500 … 1 600, z 800 … 900 (no OSM
    /// field there), no data elsewhere.
    fn raster(proj: &LocalProjection) -> CropRaster {
        let crs = crate::crs::Crs::Laea;
        let corners = [
            [-500.0, -500.0],
            [2500.0, -500.0],
            [-500.0, 1500.0],
            [2500.0, 1500.0],
        ];
        let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for c in corners {
            let (lat, lon) = proj.unproject(c);
            let p = crs.forward(lat, lon);
            lo = [lo[0].min(p[0]), lo[1].min(p[1])];
            hi = [hi[0].max(p[0]), hi[1].max(p[1])];
        }
        let (west, north) = ((lo[0] / 10.0).floor() * 10.0, (hi[1] / 10.0).ceil() * 10.0);
        let width = ((hi[0] - west) / 10.0).ceil() as usize + 1;
        let height = ((north - lo[1]) / 10.0).ceil() as usize + 1;
        let mut codes = vec![255u8; width * height];
        for y in 0..height {
            for x in 0..width {
                let (e, n) = (
                    west + (x as f64 + 0.5) * 10.0,
                    north - (y as f64 + 0.5) * 10.0,
                );
                let (lat, lon) = crs.inverse(e, n);
                let p = proj.project(lat, lon);
                if p[0] < 300.0 {
                    codes[y * width + x] = FieldCrop::Sunflower.code();
                } else if (1500.0..1600.0).contains(&p[0]) && (800.0..900.0).contains(&p[1]) {
                    codes[y * width + x] = FieldCrop::Banana.code();
                }
            }
        }
        CropRaster {
            sources: Vec::new(),
            crs,
            pixel_m: 10.0,
            west,
            north,
            width,
            height,
            codes,
        }
    }

    #[test]
    fn a_crop_map_overrides_the_mix_but_not_the_tags() {
        let pkg = package((47.99, 7.85));
        let proj = LocalProjection::new(47.99, 7.85);
        let r = raster(&proj);
        let (v, st) = build_vegetation(&pkg, &areas(&proj), &TreeData::default(), &proj, Some(&r));
        assert_eq!(v.at(1000.0, 105.0, 205.0).0, FieldCrop::Sunflower);
        // The tagged maize stays maize.
        assert_eq!(v.at(1000.0, 205.0, 805.0).0, FieldCrop::Maize);
        // Beyond OSM's fields the map's plantations come in, with rows.
        let (c, rows) = v.at(1000.0, 1555.0, 855.0);
        assert_eq!(c, FieldCrop::Banana);
        assert!(rows.is_some());
        // And on open land west of x 300 (no OSM field at z 50) the
        // sunflowers too.
        assert_eq!(v.at(1000.0, 105.0, 55.0).0, FieldCrop::Sunflower);
        assert!(st.measured_cells > 100);
    }

    #[test]
    fn tenerife_plantations_by_altitude_and_palms() {
        let r = FloraRegion::Tenerife;
        assert_eq!(
            plantation_default("orchard", r, 100.0),
            Some(FieldCrop::Banana)
        );
        assert_eq!(
            plantation_default("orchard", r, 600.0),
            Some(FieldCrop::Subtropical)
        );
        assert_eq!(
            plantation_default("orchard", r, 1200.0),
            Some(FieldCrop::Orchard)
        );
        assert_eq!(
            plantation_default("greenhouse_horticulture", r, 50.0),
            Some(FieldCrop::Banana)
        );
        assert_eq!(
            plantation_default("orchard", FloraRegion::AlpineRhine, 100.0),
            Some(FieldCrop::Orchard)
        );
        assert_eq!(plantation_default("farmland", r, 100.0), None);
        let palm = tags(&[("leaf_type", "palm")]);
        assert_eq!(tree_species(&palm, r, 0.1), Species::DatePalm);
        assert_eq!(
            tree_species(&palm, FloraRegion::BlackForest, 0.1),
            Species::SlenderPalm
        );
        let pine = tags(&[("leaf_type", "needleleaved")]);
        assert_eq!(tree_species(&pine, r, 0.9), Species::CanaryPine);
        assert_eq!(tree_species(&tags(&[]), r, 0.9), Species::Unknown);
        // Mixes sum to about 1 and hold only arable crops or grass.
        for reg in [FloraRegion::AlpineRhine, FloraRegion::BlackForest, r] {
            for alt in [200.0, 600.0, 1200.0] {
                let m = crop_mix(reg, alt);
                let s: f32 = m.iter().map(|x| x.1).sum();
                assert!((s - 1.0).abs() < 0.02, "{reg:?} {alt}: {s}");
            }
        }
    }

    #[test]
    fn main_axes_of_rings() {
        let long_x = [[0.0, 0.0], [100.0, 0.0], [100.0, 20.0], [0.0, 20.0]];
        assert!(
            main_axis(&long_x).abs() < 1e-9
                || (main_axis(&long_x) - std::f64::consts::PI).abs() < 1e-9
        );
        let long_z = [[0.0, 0.0], [20.0, 0.0], [20.0, 100.0], [0.0, 100.0]];
        assert!((main_axis(&long_z) - std::f64::consts::FRAC_PI_2).abs() < 1e-9);
        // A rotated rectangle at 30°.
        let (c, s) = (30f64.to_radians().cos(), 30f64.to_radians().sin());
        let rot = |p: [f64; 2]| [p[0] * c - p[1] * s, p[0] * s + p[1] * c];
        let r: Vec<[f64; 2]> = long_x.iter().map(|p| rot(*p)).collect();
        assert!((main_axis(&r) - 30f64.to_radians()).abs() < 1e-9);
    }
}
