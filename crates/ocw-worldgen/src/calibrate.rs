//! Region calibration (spec §14 Phase 8 T3, 8B.2): the world builder
//! *measures* the region's look in open imagery instead of guessing it.
//!
//! - [`palette`]: the median colour of every land class (interior cells of
//!   the splat map, away from roads and buildings), of the paved and the
//!   unpaved roads (along the road graph, not in tunnels or on bridges)
//!   and of the roofs, from the orthophoto; land classes the orthophoto
//!   does not cover come from the Sentinel-2 summer composite.
//! - [`roof_colour`]: one colour per building, the median of the
//!   orthophoto inside the footprint eroded by 1 m (fewer than
//!   [`MIN_ROOF_PIXELS`] there: the whole footprint); OSM `roof:colour`
//!   wins ([`parse_colour`]).
//! - [`seasonal_tints`]: per tile and season a coarse colour map from the
//!   Sentinel-2 composites; gaps (persistent cloud) are filled from the
//!   other seasons, then from the tile's mean.
//!
//! Medians are per channel (exact, from 256-bin histograms), so shadows,
//! cars and single bright pixels do not move them.

use std::collections::BTreeMap;

use ocw_world::buildings::Building;
use ocw_world::geo::LocalProjection;
use ocw_world::graph::{RoadClass, RoadGraph};
use ocw_world::landuse::{LandClass, LandUse};
use ocw_world::look::{
    ImagerySource, Palette, PaletteEntry, PaletteKey, RegionLook, RoofColour, RoofColours,
    RoofOrigin, SeasonalTints, encode_tint,
};
use ocw_world::{TileKey, WorldPackage};

use crate::imagery::{ColourSampler, Imagery, OrthoReader, SeasonalSource, TileAffine};

/// Erosion of the footprints before the roof is sampled (m): keeps walls,
/// eaves' shadows and the ground next to the house out (spec 8B.2).
pub const ROOF_EROSION_M: f64 = 1.0;
/// Fewer pixels than this inside the eroded footprint: sample the whole
/// footprint (small sheds).
pub const MIN_ROOF_PIXELS: usize = 6;
/// A palette colour needs at least this many pixels.
pub const MIN_PALETTE_SAMPLES: u32 = 50;
/// Sampling step along the roads (m).
const ROAD_STEP_M: f64 = 2.0;

/// Per-channel 256-bin histograms: exact medians in constant memory.
#[derive(Debug, Clone)]
pub struct Histogram {
    bins: Box<[[u32; 256]; 3]>,
    pub count: u32,
}

impl Default for Histogram {
    fn default() -> Self {
        Self {
            bins: Box::new([[0; 256]; 3]),
            count: 0,
        }
    }
}

impl Histogram {
    pub fn add(&mut self, rgb: [u8; 3]) {
        for (k, v) in rgb.iter().enumerate() {
            self.bins[k][usize::from(*v)] += 1;
        }
        self.count += 1;
    }

    pub fn merge(&mut self, o: &Histogram) {
        for k in 0..3 {
            for i in 0..256 {
                self.bins[k][i] += o.bins[k][i];
            }
        }
        self.count += o.count;
    }

    /// Per-channel median (lower median), `None` when empty.
    pub fn median(&self) -> Option<[u8; 3]> {
        if self.count == 0 {
            return None;
        }
        let half = self.count.div_ceil(2);
        let mut out = [0u8; 3];
        for (k, o) in out.iter_mut().enumerate() {
            let mut acc = 0;
            for (i, n) in self.bins[k].iter().enumerate() {
                acc += n;
                if acc >= half {
                    *o = i as u8;
                    break;
                }
            }
        }
        Some(out)
    }
}

/// Median of a list of colours (per channel).
pub fn median(colours: &[[u8; 3]]) -> Option<[u8; 3]> {
    let mut h = Histogram::default();
    colours.iter().for_each(|c| h.add(*c));
    h.median()
}

// --- OSM colours ----------------------------------------------------------

/// Parses an OSM colour value (`roof:colour`, `building:colour`): `#rgb`,
/// `#rrggbb` or a CSS / common OSM colour name (case-insensitive).
pub fn parse_colour(s: &str) -> Option<[u8; 3]> {
    let s = s.trim().to_ascii_lowercase();
    if let Some(hex) = s.strip_prefix('#') {
        let v = |a: &str| u8::from_str_radix(a, 16).ok();
        return match hex.len() {
            6 => Some([v(&hex[0..2])?, v(&hex[2..4])?, v(&hex[4..6])?]),
            3 => {
                let d = |i: usize| v(&hex[i..=i]).map(|x| x * 17);
                Some([d(0)?, d(1)?, d(2)?])
            }
            _ => None,
        };
    }
    let rgb = match s.replace([' ', '_', '-'], "").as_str() {
        "red" => [255, 0, 0],
        "darkred" => [139, 0, 0],
        "maroon" => [128, 0, 0],
        "brown" => [165, 42, 42],
        "saddlebrown" => [139, 69, 19],
        "sienna" => [160, 82, 45],
        "chocolate" => [210, 105, 30],
        "terracotta" => [204, 78, 52],
        "orange" => [255, 165, 0],
        "darkorange" => [255, 140, 0],
        "orangered" => [255, 69, 0],
        "firebrick" => [178, 34, 34],
        "indianred" => [205, 92, 92],
        "salmon" => [250, 128, 114],
        "tan" => [210, 180, 140],
        "beige" => [245, 245, 220],
        "wheat" => [245, 222, 179],
        "yellow" => [255, 255, 0],
        "gold" => [255, 215, 0],
        "khaki" => [240, 230, 140],
        "olive" => [128, 128, 0],
        "green" => [0, 128, 0],
        "darkgreen" => [0, 100, 0],
        "lightgreen" => [144, 238, 144],
        "teal" => [0, 128, 128],
        "blue" => [0, 0, 255],
        "darkblue" => [0, 0, 139],
        "navy" => [0, 0, 128],
        "lightblue" => [173, 216, 230],
        "steelblue" => [70, 130, 180],
        "slategray" | "slategrey" => [112, 128, 144],
        "darkslategray" | "darkslategrey" => [47, 79, 79],
        "black" => [0, 0, 0],
        "dimgray" | "dimgrey" => [105, 105, 105],
        "darkgray" | "darkgrey" => [169, 169, 169],
        "gray" | "grey" => [128, 128, 128],
        "silver" => [192, 192, 192],
        "lightgray" | "lightgrey" => [211, 211, 211],
        "gainsboro" => [220, 220, 220],
        "white" => [255, 255, 255],
        "ivory" => [255, 255, 240],
        "purple" => [128, 0, 128],
        "pink" => [255, 192, 203],
        _ => return None,
    };
    Some(rgb)
}

// --- Geometry -------------------------------------------------------------

fn inside(p: [f64; 2], ring: &[[f64; 2]]) -> bool {
    let mut c = false;
    let n = ring.len();
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + n - 1) % n]);
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
        {
            c = !c;
        }
    }
    c
}

fn seg_dist(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dz * dz;
    let t = if l2 > 0.0 {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dz) / l2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    ((p[0] - a[0] - t * dx).powi(2) + (p[1] - a[1] - t * dz).powi(2)).sqrt()
}

fn ring_dist(p: [f64; 2], ring: &[[f64; 2]]) -> f64 {
    let n = ring.len();
    (0..n)
        .map(|i| seg_dist(p, ring[i], ring[(i + 1) % n]))
        .fold(f64::INFINITY, f64::min)
}

/// The sample points of a footprint on a `step` lattice: inside the outer
/// ring, outside the holes, at least `erosion` from every ring.
pub fn footprint_points(
    outer: &[[f64; 2]],
    holes: &[Vec<[f64; 2]>],
    step: f64,
    erosion: f64,
) -> Vec<[f64; 2]> {
    if outer.len() < 3 || step <= 0.0 {
        return Vec::new();
    }
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for p in outer {
        lo = [lo[0].min(p[0]), lo[1].min(p[1])];
        hi = [hi[0].max(p[0]), hi[1].max(p[1])];
    }
    let mut out = Vec::new();
    let mut z = (lo[1] / step).floor() * step + step / 2.0;
    while z < hi[1] {
        let mut x = (lo[0] / step).floor() * step + step / 2.0;
        while x < hi[0] {
            let p = [x, z];
            if inside(p, outer)
                && !holes.iter().any(|h| inside(p, h))
                && (erosion <= 0.0
                    || (ring_dist(p, outer) >= erosion
                        && holes.iter().all(|h| ring_dist(p, h) >= erosion)))
            {
                out.push(p);
            }
            x += step;
        }
        z += step;
    }
    out
}

/// The roof colour of a footprint: OSM `roof:colour` if given, else the
/// median of the imagery inside the footprint eroded by [`ROOF_EROSION_M`]
/// (the whole footprint when that leaves fewer than [`MIN_ROOF_PIXELS`]
/// pixels with data); `None` without enough data.
pub fn roof_colour(
    outer: &[[f64; 2]],
    holes: &[Vec<[f64; 2]>],
    osm: Option<[u8; 3]>,
    sampler: &mut dyn ColourSampler,
) -> Option<RoofColour> {
    if let Some(rgb) = osm {
        return Some(RoofColour {
            rgb,
            origin: RoofOrigin::Osm,
        });
    }
    let step = sampler.pixel_m().max(0.1);
    for erosion in [ROOF_EROSION_M, 0.0] {
        let px: Vec<[u8; 3]> = footprint_points(outer, holes, step, erosion)
            .into_iter()
            .filter_map(|p| sampler.sample(p).map(|s| s.0))
            .collect();
        if px.len() >= MIN_ROOF_PIXELS || (erosion == 0.0 && px.len() >= 3) {
            return median(&px).map(|rgb| RoofColour {
                rgb,
                origin: RoofOrigin::Orthophoto,
            });
        }
    }
    None
}

fn building_rings(b: &Building) -> (Vec<[f64; 2]>, Vec<Vec<[f64; 2]>>) {
    let f = |r: &[[f32; 2]]| r.iter().map(|p| p.map(f64::from)).collect::<Vec<_>>();
    (f(&b.outline), b.holes.iter().map(|h| f(h)).collect())
}

/// The centroid of a building's outline (local metres).
pub fn building_centre(b: &Building) -> [f64; 2] {
    let n = b.outline.len().max(1) as f64;
    [
        b.outline.iter().map(|p| f64::from(p[0])).sum::<f64>() / n,
        b.outline.iter().map(|p| f64::from(p[1])).sum::<f64>() / n,
    ]
}

// --- Palette --------------------------------------------------------------

/// Cells of a tile that are not plain ground: under or next to a building
/// or a road strip (`true` = masked).
fn tile_mask(pkg: &WorldPackage, key: TileKey, cells: u32) -> Vec<bool> {
    let size = f64::from(pkg.tile_size_m);
    let n = cells as usize;
    let s = size / f64::from(cells);
    let o = key.origin(size);
    let mut mask = vec![false; n * n];
    let mut mark = |lo: [f64; 2], hi: [f64; 2], test: &dyn Fn([f64; 2]) -> bool| {
        let c0 = (((lo[0] - o[0]) / s).floor().max(0.0)) as usize;
        let r0 = (((lo[1] - o[1]) / s).floor().max(0.0)) as usize;
        let c1 = (((hi[0] - o[0]) / s).ceil().max(0.0) as usize).min(n);
        let r1 = (((hi[1] - o[1]) / s).ceil().max(0.0) as usize).min(n);
        for r in r0..r1 {
            for c in c0..c1 {
                let p = [o[0] + (c as f64 + 0.5) * s, o[1] + (r as f64 + 0.5) * s];
                if !mask[r * n + c] && test(p) {
                    mask[r * n + c] = true;
                }
            }
        }
    };
    if let Some(bs) = pkg.buildings.as_ref().and_then(|b| b.get(&key)) {
        for b in bs {
            let (outer, _) = building_rings(b);
            let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
            for p in &outer {
                lo = [lo[0].min(p[0]), lo[1].min(p[1])];
                hi = [hi[0].max(p[0]), hi[1].max(p[1])];
            }
            let pad = s;
            mark([lo[0] - pad, lo[1] - pad], [hi[0] + pad, hi[1] + pad], &|p| {
                inside(p, &outer) || ring_dist(p, &outer) < s
            });
        }
    }
    if let Some(t) = pkg.tiles.get(&key) {
        for road in &t.roads {
            let reach = f64::from(road.width_m) / 2.0 + s;
            for w in road.points.windows(2) {
                let a = [f64::from(w[0][0]), f64::from(w[0][2])];
                let b = [f64::from(w[1][0]), f64::from(w[1][2])];
                let lo = [a[0].min(b[0]) - reach, a[1].min(b[1]) - reach];
                let hi = [a[0].max(b[0]) + reach, a[1].max(b[1]) + reach];
                mark(lo, hi, &|p| seg_dist(p, a, b) < reach);
            }
        }
    }
    mask
}

/// Histograms of the land classes of one tile: interior cells (all four
/// neighbours of the same class), not masked; orthophoto first, the
/// fallback sampler (Sentinel-2 summer) where the orthophoto has no data.
fn land_histograms(
    pkg: &WorldPackage,
    landuse: &LandUse,
    key: TileKey,
    ortho: &mut dyn ColourSampler,
    mut fallback: Option<&mut dyn FnMut([f64; 2]) -> Option<[u8; 3]>>,
    ortho_h: &mut BTreeMap<PaletteKey, (Histogram, Vec<u32>)>,
    fallback_h: &mut BTreeMap<PaletteKey, Histogram>,
) {
    let Some(tile) = landuse.tiles.get(&key) else {
        return;
    };
    let n = landuse.cells as usize;
    let size = f64::from(pkg.tile_size_m);
    let s = size / f64::from(landuse.cells);
    let o = key.origin(size);
    let mask = tile_mask(pkg, key, landuse.cells);
    for r in 1..n.saturating_sub(1) {
        for c in 1..n.saturating_sub(1) {
            let i = r * n + c;
            let class = tile.classes[i];
            if mask[i]
                || class == LandClass::Water
                || [i - 1, i + 1, i - n, i + n]
                    .iter()
                    .any(|&j| tile.classes[j] != class)
            {
                continue;
            }
            let p = [o[0] + (c as f64 + 0.5) * s, o[1] + (r as f64 + 0.5) * s];
            let k = PaletteKey::Land(class);
            if let Some((rgb, src)) = ortho.sample(p) {
                let e = ortho_h.entry(k).or_default();
                e.0.add(rgb);
                bump(&mut e.1, src);
            } else if let Some(f) = fallback.as_mut()
                && let Some(rgb) = f(p)
            {
                fallback_h.entry(k).or_default().add(rgb);
            }
        }
    }
}

fn bump(v: &mut Vec<u32>, src: u8) {
    let i = usize::from(src);
    if v.len() <= i {
        v.resize(i + 1, 0);
    }
    v[i] += 1;
}

/// Whether a road class is paved (car roads and cycleways) or not
/// (tracks, paths).
pub fn paved(class: RoadClass) -> bool {
    !matches!(class, RoadClass::Track | RoadClass::Path)
}

/// Samples along the graph's edges every [`ROAD_STEP_M`] (centre line; no
/// tunnels, no bridges) into the paved / unpaved histograms.
fn road_histograms(
    graph: &RoadGraph,
    ortho: &mut dyn ColourSampler,
    tile_size: f64,
    tiles: &dyn Fn(TileKey) -> bool,
    out: &mut BTreeMap<PaletteKey, (Histogram, Vec<u32>)>,
) {
    for e in &graph.edges {
        if e.tunnel || e.bridge {
            continue;
        }
        let key = if paved(e.class) {
            PaletteKey::PavedRoad
        } else {
            PaletteKey::UnpavedRoad
        };
        let mut carry = 0.0;
        for w in e.points.windows(2) {
            let (a, b) = (w[0], w[1]);
            let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
            let mut t = carry;
            while t < len {
                let p = [
                    a[0] + (b[0] - a[0]) * t / len,
                    a[1] + (b[1] - a[1]) * t / len,
                ];
                if tiles(TileKey::containing(p, tile_size))
                    && let Some((rgb, src)) = ortho.sample(p)
                {
                    let h = out.entry(key).or_default();
                    h.0.add(rgb);
                    bump(&mut h.1, src);
                }
                t += ROAD_STEP_M;
            }
            carry = t - len;
        }
    }
}

/// Picks each key's colour: from the orthophoto with enough samples (the
/// source that gave most of them), else from the fallback (`fallback_src`).
fn choose(
    ortho: &BTreeMap<PaletteKey, (Histogram, Vec<u32>)>,
    fallback: &BTreeMap<PaletteKey, Histogram>,
    fallback_src: Option<u8>,
) -> Vec<PaletteEntry> {
    let mut keys: Vec<PaletteKey> = ortho.keys().chain(fallback.keys()).copied().collect();
    keys.sort();
    keys.dedup();
    let mut out = Vec::new();
    for key in keys {
        if let Some((h, by)) = ortho.get(&key)
            && h.count >= MIN_PALETTE_SAMPLES
        {
            let source = (0..by.len()).max_by_key(|&i| by[i]).unwrap_or(0) as u8;
            out.push(PaletteEntry {
                key,
                rgb: h.median().expect("non-empty"),
                samples: h.count,
                source,
            });
        } else if let (Some(h), Some(src)) = (fallback.get(&key), fallback_src)
            && h.count >= MIN_PALETTE_SAMPLES
        {
            out.push(PaletteEntry {
                key,
                rgb: h.median().expect("non-empty"),
                samples: h.count,
                source: src,
            });
        }
    }
    out
}

// --- The passes -----------------------------------------------------------

/// What [`calibrate`] measured.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CalibrationStats {
    pub buildings: usize,
    pub roofs_ortho: usize,
    pub roofs_osm: usize,
    pub roofs_none: usize,
    pub palette: Vec<(PaletteKey, [u8; 3], u32)>,
    pub season_tiles: usize,
    /// Tiles left without tint maps: no composite reaches them (the
    /// renderer keeps the class colours there).
    pub season_tiles_outside: usize,
    pub season_bytes: usize,
    /// Tint cells filled from another season or the tile mean.
    pub season_filled: usize,
    pub errors: Vec<String>,
}

/// Options of [`calibrate`].
#[derive(Debug, Clone, Copy)]
pub struct CalibrateOptions {
    /// Cells per tile side of the seasonal tint maps (50 = 20 m).
    pub season_cells: u32,
    /// Worker threads (0 = all cores).
    pub threads: usize,
}

impl Default for CalibrateOptions {
    fn default() -> Self {
        Self {
            season_cells: 50,
            threads: 0,
        }
    }
}

/// Per-thread results of the tile pass.
#[derive(Default)]
struct Part {
    ortho_h: BTreeMap<PaletteKey, (Histogram, Vec<u32>)>,
    fallback_h: BTreeMap<PaletteKey, Histogram>,
    roofs: Vec<(TileKey, Vec<Option<RoofColour>>)>,
    seasons: Vec<(TileKey, Vec<Vec<u8>>, usize)>,
    /// Per season the histograms of the tint cells by land class.
    season_classes: Vec<BTreeMap<LandClass, Histogram>>,
    /// Tiles the seasonal composites do not reach at all.
    seasons_outside: usize,
    errors: Vec<String>,
}

/// Measures the package's look in `imagery` and stores it in `pkg.look`
/// (sections `PALT`, `ROOF`, `SEAS`). `osm_roof` gives the OSM
/// `roof:colour` of the building whose footprint contains a point (it
/// wins over the orthophoto).
pub fn calibrate(
    pkg: &mut WorldPackage,
    imagery: &Imagery,
    osm_roof: &(dyn Fn([f64; 2]) -> Option<[u8; 3]> + Sync),
    opts: CalibrateOptions,
) -> CalibrationStats {
    let proj = LocalProjection::new(pkg.origin.0, pkg.origin.1);
    let size = f64::from(pkg.tile_size_m);
    let mut sources: Vec<ImagerySource> = imagery.ortho.iter().map(|o| o.meta.clone()).collect();
    let s2_src = imagery.seasonal.as_ref().map(|s| {
        sources.push(s.meta.clone());
        (sources.len() - 1) as u8
    });
    let keys: Vec<TileKey> = pkg.tiles.keys().copied().collect();
    let threads = if opts.threads == 0 {
        std::thread::available_parallelism().map_or(4, |n| n.get())
    } else {
        opts.threads
    }
    .max(1);
    let chunk = keys.len().div_ceil(threads).max(1);
    let pkg_ref = &*pkg;
    let parts: Vec<Part> = std::thread::scope(|sc| {
        let handles: Vec<_> = keys
            .chunks(chunk)
            .map(|ks| {
                let proj = proj;
                sc.spawn(move || tile_pass(pkg_ref, ks, imagery, &proj, size, osm_roof, opts))
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("calibration thread"))
            .collect()
    });
    let mut ortho_h: BTreeMap<PaletteKey, (Histogram, Vec<u32>)> = BTreeMap::new();
    let mut fallback_h: BTreeMap<PaletteKey, Histogram> = BTreeMap::new();
    let mut roofs = RoofColours::new();
    let mut season_tiles = BTreeMap::new();
    let mut season_classes: Vec<BTreeMap<LandClass, Histogram>> = Vec::new();
    let mut stats = CalibrationStats::default();
    for p in parts {
        for (k, (h, by)) in p.ortho_h {
            let e = ortho_h.entry(k).or_default();
            e.0.merge(&h);
            if e.1.len() < by.len() {
                e.1.resize(by.len(), 0);
            }
            for (i, n) in by.iter().enumerate() {
                e.1[i] += n;
            }
        }
        for (k, h) in p.fallback_h {
            fallback_h.entry(k).or_default().merge(&h);
        }
        for (k, v) in p.roofs {
            roofs.insert(k, v);
        }
        stats.season_tiles_outside += p.seasons_outside;
        if season_classes.len() < p.season_classes.len() {
            season_classes.resize_with(p.season_classes.len(), BTreeMap::new);
        }
        for (i, m) in p.season_classes.iter().enumerate() {
            for (c, h) in m {
                season_classes[i].entry(*c).or_default().merge(h);
            }
        }
        for (k, maps, filled) in p.seasons {
            stats.season_filled += filled;
            stats.season_bytes += maps.iter().map(Vec::len).sum::<usize>();
            season_tiles.insert(k, maps);
        }
        stats.errors.extend(p.errors);
    }
    stats.errors.sort();
    stats.errors.dedup();
    // Roads: one pass over the graph (cheap next to the tiles).
    if let Some(g) = pkg.graph.as_ref()
        && !imagery.ortho.is_empty()
    {
        let mut r = OrthoReader::new(&imagery.ortho, proj, size);
        let has = |k: TileKey| pkg.tiles.contains_key(&k);
        road_histograms(g, &mut r, size, &has, &mut ortho_h);
        stats.errors.extend(r.errors);
    }
    // Roofs: the median of the sampled roofs.
    let mut roof_h = Histogram::default();
    let mut roof_by = Vec::new();
    for v in roofs.values() {
        for c in v.iter().flatten() {
            stats.buildings += 1;
            match c.origin {
                RoofOrigin::Orthophoto => {
                    stats.roofs_ortho += 1;
                    roof_h.add(c.rgb);
                    bump(&mut roof_by, 0);
                }
                RoofOrigin::Osm => stats.roofs_osm += 1,
            }
        }
        stats.roofs_none += v.iter().filter(|c| c.is_none()).count();
        stats.buildings += v.iter().filter(|c| c.is_none()).count();
    }
    if roof_h.count > 0 {
        ortho_h.insert(PaletteKey::Roofs, (roof_h, roof_by));
    }
    let entries = choose(&ortho_h, &fallback_h, s2_src);
    stats.palette = entries.iter().map(|e| (e.key, e.rgb, e.samples)).collect();
    let seasons = imagery.seasonal.as_ref().map(|s| SeasonalTints {
        cells: opts.season_cells,
        source: s2_src.expect("seasonal source listed"),
        seasons: s.composites.iter().map(|c| c.0).collect(),
        class_means: (0..s.composites.len())
            .map(|i| {
                season_classes
                    .get(i)
                    .into_iter()
                    .flatten()
                    .filter(|(_, h)| h.count >= MIN_PALETTE_SAMPLES)
                    .filter_map(|(c, h)| h.median().map(|m| (*c, m)))
                    .collect()
            })
            .collect(),
        tiles: season_tiles,
    });
    stats.season_tiles = seasons.as_ref().map_or(0, |s| s.tiles.len());
    pkg.look = RegionLook {
        palette: (!sources.is_empty()).then(|| Palette { sources, entries }),
        roofs: (!imagery.ortho.is_empty() && pkg.buildings.is_some()).then_some(roofs),
        seasons,
    };
    stats
}

#[allow(clippy::too_many_arguments)]
fn tile_pass(
    pkg: &WorldPackage,
    keys: &[TileKey],
    imagery: &Imagery,
    proj: &LocalProjection,
    size: f64,
    osm_roof: &(dyn Fn([f64; 2]) -> Option<[u8; 3]> + Sync),
    opts: CalibrateOptions,
) -> Part {
    let mut part = Part::default();
    let mut reader = OrthoReader::new(&imagery.ortho, *proj, size);
    let summer = imagery
        .seasonal
        .as_ref()
        .and_then(|s| s.index_of(ocw_world::look::Season::Summer).map(|i| (s, i)));
    let mut affine: Option<(TileKey, TileAffine)> = None;
    let mut fallback = |p: [f64; 2]| -> Option<[u8; 3]> {
        let (s, i) = summer?;
        let key = TileKey::containing(p, size);
        if affine.is_none_or(|a| a.0 != key) {
            affine = Some((key, TileAffine::new(proj, s.crs, key, size)));
        }
        let [e, n] = affine.expect("set").1.apply(p);
        s.sample(i, e, n)
    };
    for &key in keys {
        if let Some(lu) = pkg.landuse.as_ref() {
            land_histograms(
                pkg,
                lu,
                key,
                &mut reader,
                summer.is_some().then_some(&mut fallback as &mut dyn FnMut(_) -> _),
                &mut part.ortho_h,
                &mut part.fallback_h,
            );
        }
        if !imagery.ortho.is_empty()
            && let Some(bs) = pkg.buildings.as_ref().and_then(|b| b.get(&key))
        {
            let v = bs
                .iter()
                .map(|b| {
                    let (outer, holes) = building_rings(b);
                    let osm = osm_roof(building_centre(b));
                    roof_colour(&outer, &holes, osm, &mut reader)
                })
                .collect();
            part.roofs.push((key, v));
        }
        if let Some(s) = imagery.seasonal.as_ref() {
            match tile_tints(s, proj, key, size, opts.season_cells) {
                Some((maps, filled, raw)) => {
                    if let Some(lu) = pkg.landuse.as_ref() {
                        class_histograms(lu, key, &raw, opts.season_cells, &mut part.season_classes);
                    }
                    part.seasons.push((key, maps, filled));
                }
                None => part.seasons_outside += 1,
            }
        }
    }
    part.errors.append(&mut reader.errors);
    part
}

/// Raw (unfilled) tint cells of one tile, per composite.
pub type RawTints = Vec<Vec<Option<[u8; 3]>>>;

/// The encoded tint maps of one tile (one per composite), how many cells
/// had to be filled and the raw cells; `None` when no composite has any
/// data on the tile.
pub fn tile_tints(
    s: &SeasonalSource,
    proj: &LocalProjection,
    key: TileKey,
    size: f64,
    cells: u32,
) -> Option<(Vec<Vec<u8>>, usize, RawTints)> {
    let aff = TileAffine::new(proj, s.crs, key, size);
    let n = cells as usize;
    let step = size / f64::from(cells);
    let o = key.origin(size);
    let raw: Vec<Vec<Option<[u8; 3]>>> = (0..s.composites.len())
        .map(|i| {
            (0..n * n)
                .map(|j| {
                    let p = [
                        o[0] + ((j % n) as f64 + 0.5) * step,
                        o[1] + ((j / n) as f64 + 0.5) * step,
                    ];
                    let [e, nn] = aff.apply(p);
                    s.sample(i, e, nn)
                })
                .collect()
        })
        .collect();
    if raw.iter().all(|m| m.iter().all(Option::is_none)) {
        return None;
    }
    let (maps, filled) = fill_tints(&raw, s.index_of(ocw_world::look::Season::Summer));
    Some((maps.iter().map(|m| encode_tint(m, cells)).collect(), filled, raw))
}

/// Adds the tint cells with data to per-season histograms by the land class
/// at each cell's centre (water left out).
pub fn class_histograms(
    lu: &LandUse,
    key: TileKey,
    raw: &RawTints,
    cells: u32,
    out: &mut Vec<BTreeMap<LandClass, Histogram>>,
) {
    let Some(tile) = lu.tiles.get(&key) else {
        return;
    };
    if out.len() < raw.len() {
        out.resize_with(raw.len(), BTreeMap::new);
    }
    let (n, m) = (cells as usize, lu.cells as usize);
    for (si, season) in raw.iter().enumerate() {
        for (j, c) in season.iter().enumerate() {
            let Some(rgb) = c else { continue };
            // The land-use cell under the tint cell's centre.
            let (tc, tr) = ((j % n) * 2 + 1, (j / n) * 2 + 1);
            let (lc, lr) = (tc * m / (2 * n), tr * m / (2 * n));
            let class = tile.classes[lr.min(m - 1) * m + lc.min(m - 1)];
            if class != LandClass::Water {
                out[si].entry(class).or_default().add(*rgb);
            }
        }
    }
}

/// Fills the gaps of per-season cell colours: from the same cell in the
/// other seasons (`prefer` – the summer composite, the most complete one –
/// first, then in order), else the season's mean over the tile, else mid
/// grey-green. Returns the filled maps and the number of cells filled.
pub fn fill_tints(
    raw: &[Vec<Option<[u8; 3]>>],
    prefer: Option<usize>,
) -> (Vec<Vec<[u8; 3]>>, usize) {
    let mean = |v: &[Option<[u8; 3]>]| -> Option<[u8; 3]> {
        let (mut acc, mut k) = ([0u64; 3], 0u64);
        for c in v.iter().flatten() {
            for i in 0..3 {
                acc[i] += u64::from(c[i]);
            }
            k += 1;
        }
        (k > 0).then(|| acc.map(|a| (a / k) as u8))
    };
    let means: Vec<Option<[u8; 3]>> = raw.iter().map(|v| mean(v)).collect();
    let any_mean = means.iter().flatten().next().copied();
    let mut filled = 0;
    let maps = raw
        .iter()
        .enumerate()
        .map(|(si, v)| {
            v.iter()
                .enumerate()
                .map(|(j, c)| {
                    c.unwrap_or_else(|| {
                        filled += 1;
                        let order = prefer
                            .into_iter()
                            .chain(0..raw.len())
                            .filter(|&o| o != si);
                        order
                            .filter_map(|o| raw[o][j])
                            .next()
                            .or(means[si])
                            .or(any_mean)
                            .unwrap_or([110, 125, 90])
                    })
                })
                .collect()
        })
        .collect();
    (maps, filled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_world::buildings::{BuildingKind, Roof};
    use ocw_world::landuse::LandUseTile;
    use ocw_world::look::{Season, decode_tint};

    /// A synthetic "orthophoto" at local metres: a function of the point.
    struct Synthetic<F: FnMut([f64; 2]) -> Option<[u8; 3]>> {
        f: F,
        px: f64,
    }

    impl<F: FnMut([f64; 2]) -> Option<[u8; 3]>> ColourSampler for Synthetic<F> {
        fn sample(&mut self, p: [f64; 2]) -> Option<([u8; 3], u8)> {
            (self.f)(p).map(|c| (c, 0))
        }
        fn pixel_m(&self) -> f64 {
            self.px
        }
    }

    fn square(x0: f64, z0: f64, w: f64) -> Vec<[f64; 2]> {
        vec![[x0, z0], [x0 + w, z0], [x0 + w, z0 + w], [x0, z0 + w]]
    }

    #[test]
    fn histogram_medians_ignore_outliers() {
        let mut v = vec![[100, 150, 50]; 7];
        v.push([255, 255, 255]);
        v.push([0, 0, 0]);
        v.push([0, 0, 0]);
        assert_eq!(median(&v), Some([100, 150, 50]));
        assert_eq!(median(&[]), None);
        // Per channel, lower median.
        assert_eq!(median(&[[1, 9, 5], [3, 7, 5]]), Some([1, 7, 5]));
    }

    #[test]
    fn osm_colours_parse() {
        assert_eq!(parse_colour("#a52a2a"), Some([165, 42, 42]));
        assert_eq!(parse_colour(" #F00 "), Some([255, 0, 0]));
        assert_eq!(parse_colour("Dark Red"), Some([139, 0, 0]));
        assert_eq!(parse_colour("grey"), parse_colour("gray"));
        assert_eq!(parse_colour("terracotta"), Some([204, 78, 52]));
        assert_eq!(parse_colour("#12"), None);
        assert_eq!(parse_colour("plaid"), None);
    }

    #[test]
    fn footprints_are_eroded() {
        // 10 × 10 m at 0.5 m: eroded by 1 m → 8 × 8 m = 256 points.
        let pts = footprint_points(&square(0.0, 0.0, 10.0), &[], 0.5, 1.0);
        assert_eq!(pts.len(), 256);
        assert!(pts.iter().all(|p| p[0] > 1.0 && p[0] < 9.0 && p[1] > 1.0 && p[1] < 9.0));
        // A courtyard (4 × 4 m in the middle) and 1 m around it are left out.
        let hole = square(3.0, 3.0, 4.0);
        let with_hole = footprint_points(&square(0.0, 0.0, 10.0), &[hole], 0.5, 1.0);
        assert!(with_hole.iter().all(|p| ring_dist(*p, &square(3.0, 3.0, 4.0)) >= 1.0 && !inside(*p, &square(3.0, 3.0, 4.0))));
        // The 6 × 6 m buffer around the courtyard, but its corners are round:
        // the four lattice points diagonally off its corners stay.
        assert_eq!(with_hole.len(), 256 - 144 + 4);
        // A 1.5 m shed has nothing left after the erosion.
        assert!(footprint_points(&square(0.0, 0.0, 1.5), &[], 0.5, 1.0).is_empty());
    }

    #[test]
    fn roofs_are_sampled_inside_the_eroded_footprint() {
        // Red roof in the 10 × 10 m footprint, but the outer 0.8 m (eaves'
        // shadow, walls, misregistration) is dark and the ground green.
        let mut img = Synthetic {
            f: |p: [f64; 2]| {
                let inner = p[0] > 0.8 && p[0] < 9.2 && p[1] > 0.8 && p[1] < 9.2;
                let roof = p[0] > 0.0 && p[0] < 10.0 && p[1] > 0.0 && p[1] < 10.0;
                Some(if inner {
                    [170, 70, 50]
                } else if roof {
                    [20, 20, 20]
                } else {
                    [60, 140, 50]
                })
            },
            px: 0.5,
        };
        let c = roof_colour(&square(0.0, 0.0, 10.0), &[], None, &mut img).unwrap();
        assert_eq!(c.rgb, [170, 70, 50]);
        assert_eq!(c.origin, RoofOrigin::Orthophoto);
        // Without erosion the dark rim would have pulled the median.
        let rim = footprint_points(&square(0.0, 0.0, 10.0), &[], 0.5, 0.0).len();
        let eroded = footprint_points(&square(0.0, 0.0, 10.0), &[], 0.5, 1.0).len();
        assert!(rim > eroded);
    }

    #[test]
    fn roof_sampling_skips_no_data_and_osm_wins() {
        // Half of the roof is outside the orthophoto's coverage.
        let mut img = Synthetic {
            f: |p: [f64; 2]| (p[0] < 5.0).then_some([90, 90, 95]),
            px: 0.5,
        };
        let c = roof_colour(&square(0.0, 0.0, 10.0), &[], None, &mut img).unwrap();
        assert_eq!(c.rgb, [90, 90, 95]);
        // No data at all: no colour.
        let mut none = Synthetic {
            f: |_: [f64; 2]| None,
            px: 0.5,
        };
        assert_eq!(roof_colour(&square(0.0, 0.0, 10.0), &[], None, &mut none), None);
        // OSM roof:colour wins over the orthophoto.
        let c = roof_colour(&square(0.0, 0.0, 10.0), &[], Some([139, 0, 0]), &mut img).unwrap();
        assert_eq!((c.rgb, c.origin), ([139, 0, 0], RoofOrigin::Osm));
        // A small shed: the whole footprint is used.
        let mut flat = Synthetic {
            f: |_: [f64; 2]| Some([120, 120, 120]),
            px: 0.5,
        };
        let c = roof_colour(&square(0.0, 0.0, 1.5), &[], None, &mut flat).unwrap();
        assert_eq!(c.rgb, [120, 120, 120]);
    }

    /// A 100 m tile with 20 × 20 land-use cells: forest west of x = 50,
    /// meadow east; a house at (70, 70); a road along z = 20.
    fn test_pkg() -> WorldPackage {
        let key = TileKey { x: 0, z: 0 };
        let classes = (0..400)
            .map(|j| {
                if j % 20 < 10 {
                    LandClass::Forest
                } else {
                    LandClass::Meadow
                }
            })
            .collect();
        let mut tiles = BTreeMap::new();
        tiles.insert(
            key,
            ocw_world::Tile {
                key,
                heights: vec![0.0; 4],
                roads: vec![ocw_world::RoadStrip {
                    class: RoadClass::Residential,
                    width_m: 5.0,
                    surface: ocw_world::RoadSurface::Asphalt,
                    points: vec![[0.0, 0.0, 20.0], [100.0, 0.0, 20.0]],
                }],
            },
        );
        let house = Building {
            kind: BuildingKind::House,
            outline: vec![[65.0, 65.0], [75.0, 65.0], [75.0, 75.0], [65.0, 75.0]],
            holes: vec![],
            base_y: 0.0,
            eave_y: 6.0,
            roof: Roof::Flat,
        };
        let mut graph = RoadGraph::default();
        graph.edges.push(ocw_world::graph::Edge {
            id: ocw_world::graph::EdgeId(1),
            from: ocw_world::graph::VertexId(0),
            to: ocw_world::graph::VertexId(1),
            points: vec![[0.0, 20.0], [100.0, 20.0]],
            class: RoadClass::Residential,
            name: None,
            width_m: 5.0,
            bridge: false,
            tunnel: false,
            dead_end_m: None,
            boundary: false,
            oneway: ocw_world::graph::Oneway::No,
            osm_way: 1,
            profile: vec![],
        });
        WorldPackage {
            name: "test".into(),
            origin: (47.14, 9.52),
            tile_size_m: 100.0,
            samples: 2,
            tiles,
            graph: Some(graph),
            landuse: Some(LandUse {
                cells: 20,
                tiles: BTreeMap::from([(
                    key,
                    LandUseTile {
                        classes,
                        water: vec![],
                    },
                )]),
            }),
            buildings: Some(BTreeMap::from([(key, vec![house])])),
            bridges: None,
            routes: vec![],
            region: None,
            time_zone: None,
            look: Default::default(),
        }
    }

    /// The synthetic orthophoto of [`test_pkg`]: dark green forest, light
    /// green meadow, grey road, red roof (with some noise pixels).
    fn colour_at(p: [f64; 2]) -> Option<[u8; 3]> {
        let noise = ((p[0] * 7.3 + p[1] * 3.1) as i64) % 17 == 0;
        Some(if noise {
            [250, 250, 250]
        } else if (p[1] - 20.0).abs() < 2.5 {
            [95, 95, 100]
        } else if (65.0..75.0).contains(&p[0]) && (65.0..75.0).contains(&p[1]) {
            [160, 65, 45]
        } else if p[0] < 50.0 {
            [40, 70, 35]
        } else {
            [110, 150, 70]
        })
    }

    #[test]
    fn palettes_come_from_interior_cells_away_from_roads_and_houses() {
        let pkg = test_pkg();
        let key = TileKey { x: 0, z: 0 };
        let mut img = Synthetic { f: colour_at, px: 0.5 };
        let (mut ortho_h, mut fb) = (BTreeMap::new(), BTreeMap::new());
        land_histograms(
            &pkg,
            pkg.landuse.as_ref().unwrap(),
            key,
            &mut img,
            None,
            &mut ortho_h,
            &mut fb,
        );
        road_histograms(
            pkg.graph.as_ref().unwrap(),
            &mut img,
            100.0,
            &|_| true,
            &mut ortho_h,
        );
        let entries = choose(&ortho_h, &fb, None);
        let get = |k| entries.iter().find(|e| e.key == k).map(|e| e.rgb);
        assert_eq!(get(PaletteKey::Land(LandClass::Forest)), Some([40, 70, 35]));
        assert_eq!(get(PaletteKey::Land(LandClass::Meadow)), Some([110, 150, 70]));
        // 50 road samples (every 2 m): enough for the asphalt colour.
        assert_eq!(get(PaletteKey::PavedRoad), Some([95, 95, 100]));
        assert_eq!(get(PaletteKey::UnpavedRoad), None);
        // No road or roof pixel got into the meadow's histogram.
        let meadow = &ortho_h[&PaletteKey::Land(LandClass::Meadow)].0;
        assert!(meadow.count > 50 && meadow.count < 180, "{}", meadow.count);
    }

    #[test]
    fn the_fallback_fills_classes_the_orthophoto_misses() {
        let pkg = test_pkg();
        let key = TileKey { x: 0, z: 0 };
        // The orthophoto covers only the forest half.
        let mut img = Synthetic {
            f: |p: [f64; 2]| (p[0] < 50.0).then(|| colour_at(p)).flatten(),
            px: 0.5,
        };
        let mut s2 = |_: [f64; 2]| Some([100, 140, 60]);
        let (mut ortho_h, mut fb) = (BTreeMap::new(), BTreeMap::new());
        land_histograms(
            &pkg,
            pkg.landuse.as_ref().unwrap(),
            key,
            &mut img,
            Some(&mut s2),
            &mut ortho_h,
            &mut fb,
        );
        let entries = choose(&ortho_h, &fb, Some(3));
        let meadow = entries
            .iter()
            .find(|e| e.key == PaletteKey::Land(LandClass::Meadow))
            .unwrap();
        assert_eq!((meadow.rgb, meadow.source), ([100, 140, 60], 3));
        let forest = entries
            .iter()
            .find(|e| e.key == PaletteKey::Land(LandClass::Forest))
            .unwrap();
        assert_eq!(forest.source, 0);
    }

    #[test]
    fn calibrate_fills_all_sections_from_fixture_imagery() {
        use crate::crs::Crs;
        let dir = tempfile::tempdir().unwrap();
        let mut pkg = test_pkg();
        let proj = LocalProjection::new(pkg.origin.0, pkg.origin.1);
        // Orthophoto fixture in LV95 around the tile, coloured like
        // colour_at at the local point of each pixel.
        let crs_cells = crate::imagery::package_cells(
            [TileKey { x: 0, z: 0 }],
            &proj,
            Crs::Lv95,
            100.0,
            100.0,
            10.0,
        );
        let cells: Vec<(i64, i64)> = crs_cells.into_iter().collect();
        let local = |e: f64, n: f64| {
            let (lat, lon) = Crs::Lv95.inverse(e, n);
            proj.project(lat, lon)
        };
        crate::imagery::tests::write_ortho(&dir.path().join("1-ortho"), &cells, 100.0, 0.5, |e, n| {
            colour_at(local(e, n)).unwrap()
        });
        // Seasonal composites in UTM 32: summer green, winter white, and
        // autumn with a hole of no data.
        let s2 = dir.path().join("s2");
        std::fs::create_dir_all(&s2).unwrap();
        let o = Crs::UtmNorth(32).forward(47.14, 9.52);
        let (west, north) = ((o[0] / 10.0).floor() * 10.0 - 200.0, (o[1] / 10.0).floor() * 10.0 + 200.0);
        let (w, h) = (40usize, 40usize);
        let mut seasons = Vec::new();
        for (name, rgb, doy) in [
            ("summer", [80u8, 130, 60], 196),
            ("autumn", [150, 110, 50], 288),
            ("winter", [235, 235, 240], 15),
        ] {
            let mut px: Vec<u8> = (0..w * h).flat_map(|_| rgb).collect();
            if name == "autumn" {
                px.iter_mut().for_each(|v| *v = 0);
            }
            std::fs::write(s2.join(format!("{name}.rgb")), px).unwrap();
            seasons.push(format!(
                r#"{{"season":"{name}","centre_doy":{doy},"first_doy":{},"last_doy":{},"scenes":5,"file":"{name}.rgb"}}"#,
                (doy - 30).max(1),
                doy + 30
            ));
        }
        std::fs::write(
            s2.join("index.json"),
            format!(
                r#"{{"kind":"seasonal","id":"s2","name":"Sentinel-2","licence":"Copernicus",
                "notice":"Contains modified Copernicus Sentinel data 2025","crs":"utm32","pixel_m":10,
                "west":{west},"north":{north},"width":{w},"height":{h},"seasons":[{}]}}"#,
                seasons.join(",")
            ),
        )
        .unwrap();
        let imagery = Imagery::load(dir.path()).unwrap();
        assert_eq!(imagery.ortho.len(), 1);
        assert_eq!(imagery.seasonal.as_ref().unwrap().composites.len(), 3);
        let osm = |_: [f64; 2]| None;
        let stats = calibrate(
            &mut pkg,
            &imagery,
            &osm,
            CalibrateOptions {
                season_cells: 10,
                threads: 2,
            },
        );
        assert!(stats.errors.is_empty(), "{:?}", stats.errors);
        let look = &pkg.look;
        let pal = look.palette.as_ref().unwrap();
        assert_eq!(pal.sources.len(), 2);
        assert_eq!(pal.sources[1].notice, "Contains modified Copernicus Sentinel data 2025");
        assert_eq!(pal.colour(PaletteKey::Land(LandClass::Forest)), Some([40, 70, 35]));
        assert_eq!(pal.colour(PaletteKey::PavedRoad), Some([95, 95, 100]));
        let roof = look.roof(TileKey { x: 0, z: 0 }, 0).unwrap();
        assert_eq!(roof.rgb, [160, 65, 45]);
        // Seasons: summer and winter as given, autumn filled from summer.
        let s = look.seasons.as_ref().unwrap();
        assert_eq!(s.seasons.iter().map(|i| i.season).collect::<Vec<_>>(), [
            Season::Summer,
            Season::Autumn,
            Season::Winter
        ]);
        let key = TileKey { x: 0, z: 0 };
        let near = |a: [u8; 3], b: [u8; 3]| (0..3).all(|k| (i32::from(a[k]) - i32::from(b[k])).abs() <= 2);
        assert!(near(s.map(key, 0).unwrap()[0], [80, 130, 60]));
        assert!(near(s.map(key, 1).unwrap()[55], [80, 130, 60]));
        assert!(near(decode_tint(&s.tiles[&key][2], 10).unwrap()[99], [235, 235, 240]));
        assert_eq!(stats.season_filled, 100);
        // Class means per season: summer meadow and forest green, winter white;
        // autumn had no data.
        assert_eq!(s.class_mean(0, LandClass::Meadow), Some([80, 130, 60]));
        assert_eq!(s.class_mean(2, LandClass::Forest), Some([235, 235, 240]));
        assert_eq!(s.class_mean(1, LandClass::Meadow), None);
        // A tile the composites do not reach gets no maps (class colours).
        let far = TileKey { x: 50, z: 50 };
        assert!(tile_tints(imagery.seasonal.as_ref().unwrap(), &proj, far, 100.0, 10).is_none());
        // And the package round-trips with its look (the test graph has no
        // vertices, so it is left out).
        pkg.graph = None;
        let back = WorldPackage::from_bytes(&pkg.to_bytes()).unwrap();
        assert_eq!(back.look, pkg.look);
    }

    #[test]
    fn tint_gaps_fill_from_other_seasons_then_the_mean() {
        let raw = vec![
            vec![Some([10, 10, 10]), None, None],         // spring
            vec![Some([50, 60, 70]), Some([52, 62, 72]), None], // summer
        ];
        let (maps, filled) = fill_tints(&raw, Some(1));
        assert_eq!(filled, 3);
        // Spring's gap at 1 takes summer's cell; at 2 neither has data:
        // spring's own mean.
        assert_eq!(maps[0], vec![[10, 10, 10], [52, 62, 72], [10, 10, 10]]);
        // Summer's gap at 2: spring has nothing there either → summer mean.
        assert_eq!(maps[1][2], [51, 61, 71]);
    }
}
