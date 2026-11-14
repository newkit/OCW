//! The orthophoto corridor (spec §14 Phase 8 T7): the region's open
//! orthophotos (`scripts/fetch-imagery.sh`, the sources of `PALT`) cut
//! along the package's roads and stored per tile chunk as JPEG in the
//! optional section `ORTC` ([`ocw_world::ortho`]).
//!
//! Per tile: the corridor weights at 4 m corners
//! ([`ocw_world::ortho::corridor_weights`]: ±50 m beside the road edges,
//! none in forests or on buildings); the 125 m chunks the corridor
//! touches are resampled bilinearly from the orthophoto cells at the
//! corridor's pixel size (0.5 m, or the source's own if coarser), the best
//! source first; **shadows and bright spots are toned down** relative to the
//! local mean brightness ([`tone`]: dark pixels – cast shadows of houses and
//! trees, which our own 3D objects cast again – are lifted, highlights such
//! as parked cars and glare pulled in); the pixels outside the corridor are
//! filled with the chunk's mean colour, and the chunk is JPEG encoded.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use ocw_world::TileKey;
use ocw_world::geo::LocalProjection;
use ocw_world::ortho::{
    self, CHUNKS, OrthoCorridor, OrthoEncoding, OrthoTile, WEIGHT_CELL_M, encode_weights,
    quantise_weights,
};
use ocw_world::package::WorldPackage;

use crate::imagery::{Imagery, OrthoSource, TileAffine};

/// Options of [`cut_corridor`].
#[derive(Debug, Clone, Copy)]
pub struct CorridorOptions {
    /// Half width beside the road edges (m).
    pub half_width_m: f32,
    /// Finest pixel size (m); the sources' own size if coarser.
    pub pixel_m: f32,
    /// JPEG quality (1 … 100).
    pub quality: u8,
    /// Worker threads (0 = all cores).
    pub threads: usize,
}

impl Default for CorridorOptions {
    fn default() -> Self {
        Self {
            half_width_m: ortho::HALF_WIDTH_M,
            pixel_m: ortho::PIXEL_M,
            quality: 80,
            threads: 0,
        }
    }
}

/// What [`cut_corridor`] did.
#[derive(Debug, Clone, Default)]
pub struct CorridorStats {
    pub tiles: usize,
    pub chunks: usize,
    /// Corridor pixels (weight > 0, with the margin) sampled.
    pub pixels: u64,
    /// JPEG bytes of all chunks.
    pub image_bytes: usize,
    /// Encoded weight bytes of all tiles.
    pub weight_bytes: usize,
    pub pixel_m: f32,
    pub errors: Vec<String>,
}

/// How much darker-than-local pixels are lifted: the ratio to the local
/// mean brightness is raised to this power (0.55: a shadow at 40 % of its
/// surroundings comes out at 60 %).
pub const SHADOW_LIFT: f32 = 0.55;
/// Brighter-than-local pixels: the ratio to this power (cars, glare).
pub const HIGHLIGHT_PULL: f32 = 0.7;
/// Size of the blocks (m) the local mean brightness is measured on (then
/// blurred over 3 × 3 blocks).
pub const TONE_BLOCK_M: f32 = 8.0;

/// How far a shadow's colour moves towards the local ground colour, per
/// unit of darkness (shadows are lit by the blue sky: brightened alone they
/// stayed blue-green patches in the first Black Forest renders).
pub const SHADOW_DECAST: f32 = 0.8;

fn luma(c: [f32; 3]) -> f32 {
    0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]
}

/// The toned colour of `rgb` (sRGB 0 … 255) where the local mean colour is
/// `mean` (same scale): the brightness ratio to the local mean is
/// compressed ([`SHADOW_LIFT`], [`HIGHLIGHT_PULL`]); highlights keep their
/// hue, shadows also take on the local colour ([`SHADOW_DECAST`]).
pub fn tone(rgb: [f32; 3], mean: [f32; 3]) -> [f32; 3] {
    let (y, ym) = (luma(rgb), luma(mean));
    if y <= 0.5 || ym <= 0.5 {
        return rgb;
    }
    let t = y / ym;
    let k = if t < 1.0 { SHADOW_LIFT } else { HIGHLIGHT_PULL };
    let s = t.powf(k) / t;
    let lifted = rgb.map(|c| c * s);
    let decast = SHADOW_DECAST * (1.0 - t).clamp(0.0, 1.0);
    // The local colour at the lifted brightness.
    let local = mean.map(|c| c * y * s / ym);
    std::array::from_fn(|i| (lifted[i] + (local[i] - lifted[i]) * decast).clamp(0.0, 255.0))
}

/// Reads orthophoto pixels with bilinear filtering, best source first, with
/// a small cache of decoded cells.
struct Sampler<'a> {
    sources: &'a [OrthoSource],
    proj: LocalProjection,
    tile_size: f64,
    affine: Option<(TileKey, Vec<TileAffine>)>,
    cache: HashMap<CellKey, Option<Arc<Vec<u8>>>>,
    order: Vec<CellKey>,
    errors: Vec<String>,
}

const CACHE_CELLS: usize = 10;

/// A decoded cell in the cache: (source, cell).
type CellKey = (usize, (i64, i64));

impl<'a> Sampler<'a> {
    fn new(sources: &'a [OrthoSource], proj: LocalProjection, tile_size: f64) -> Self {
        Self {
            sources,
            proj,
            tile_size,
            affine: None,
            cache: HashMap::new(),
            order: Vec::new(),
            errors: Vec::new(),
        }
    }

    fn cell(&mut self, s: usize, cell: (i64, i64)) -> Option<Arc<Vec<u8>>> {
        if let Some(v) = self.cache.get(&(s, cell)) {
            return v.clone();
        }
        let v = match self.sources[s].decode_cell(cell) {
            Ok(v) => Some(Arc::new(v)),
            Err(e) => {
                self.errors.push(e);
                None
            }
        };
        if self.order.len() >= CACHE_CELLS {
            let old = self.order.remove(0);
            self.cache.remove(&old);
        }
        self.order.push((s, cell));
        self.cache.insert((s, cell), v.clone());
        v
    }

    /// Pixel (column `ci` east, `ni` north, in pixels from the CRS origin)
    /// of source `s`, `None` outside its cells or where it has no data.
    fn pixel(&mut self, s: usize, ci: i64, ni: i64) -> Option<[u8; 3]> {
        let src = &self.sources[s];
        let n = src.cell_px() as i64;
        let cell = (ci.div_euclid(n), ni.div_euclid(n));
        if !src.cells.contains(&cell) {
            return None;
        }
        let col = ci.rem_euclid(n);
        let row = n - 1 - ni.rem_euclid(n);
        let img = self.cell(s, cell)?;
        let k = ((row * n + col) * 3) as usize;
        let px = [img[k], img[k + 1], img[k + 2]];
        (!self.sources[s].is_nodata(px)).then_some(px)
    }

    /// The colour at local `p`, bilinear between the four nearest pixels
    /// of the first source with data there.
    fn sample(&mut self, p: [f64; 2]) -> Option<[f32; 3]> {
        let key = TileKey::containing(p, self.tile_size);
        if self.affine.as_ref().is_none_or(|a| a.0 != key) {
            let v = self
                .sources
                .iter()
                .map(|s| TileAffine::new(&self.proj, s.crs, key, self.tile_size))
                .collect();
            self.affine = Some((key, v));
        }
        for s in 0..self.sources.len() {
            let [e, n] = self.affine.as_ref().expect("set above").1[s].apply(p);
            let px = self.sources[s].pixel_m;
            let (fx, fy) = (e / px - 0.5, n / px - 0.5);
            let (c0, n0) = (fx.floor(), fy.floor());
            let (tx, ty) = ((fx - c0) as f32, (fy - n0) as f32);
            let (c0, n0) = (c0 as i64, n0 as i64);
            let mut acc = [0.0f32; 3];
            let mut wsum = 0.0;
            for (dc, dn, w) in [
                (0, 0, (1.0 - tx) * (1.0 - ty)),
                (1, 0, tx * (1.0 - ty)),
                (0, 1, (1.0 - tx) * ty),
                (1, 1, tx * ty),
            ] {
                if let Some(c) = self.pixel(s, c0 + dc, n0 + dn) {
                    for k in 0..3 {
                        acc[k] += w * f32::from(c[k]);
                    }
                    wsum += w;
                }
            }
            if wsum > 1e-4 {
                return Some(acc.map(|v| v / wsum));
            }
        }
        None
    }
}

/// One tile's cut: its corridor tile, sampled pixels, errors.
fn cut_tile(
    pkg: &WorldPackage,
    key: TileKey,
    sampler: &mut Sampler<'_>,
    pixel_m: f32,
    opts: &CorridorOptions,
) -> Option<(OrthoTile, u64)> {
    let size = pkg.tile_size_m;
    let cells = (size / WEIGHT_CELL_M).round() as u32;
    let corners = cells as usize + 1;
    let mut w = ortho::corridor_weights(pkg, key, cells, opts.half_width_m, &|_| true);
    if w.iter().all(|v| *v <= 0.0) {
        return None;
    }
    // Keep a margin of one cell (4 m) around the corridor (filtering, mips).
    let mut keep = vec![false; corners * corners];
    for k in 0..corners {
        for i in 0..corners {
            if w[k * corners + i] > 0.0 {
                for kk in k.saturating_sub(1)..=(k + 1).min(corners - 1) {
                    for ii in i.saturating_sub(1)..=(i + 1).min(corners - 1) {
                        keep[kk * corners + ii] = true;
                    }
                }
            }
        }
    }
    let chunks = usize::from(CHUNKS);
    let chunk_m = size / chunks as f32;
    let cpx = (chunk_m / pixel_m).round() as usize;
    let tpx = cpx * chunks;
    let cells_per_chunk = cells as usize / chunks;
    let o = key.origin(f64::from(size));
    // Pixel (x, z) of the tile is kept if a corner of its weight cell is.
    let cell_of = |px: usize| ((px as f32 + 0.5) * pixel_m / WEIGHT_CELL_M) as usize;
    let kept = |x: usize, z: usize| {
        let (c, r) = (
            cell_of(x).min(cells as usize - 1),
            cell_of(z).min(cells as usize - 1),
        );
        keep[r * corners + c]
            || keep[r * corners + c + 1]
            || keep[(r + 1) * corners + c]
            || keep[(r + 1) * corners + c + 1]
    };
    let mut img = vec![[0.0f32; 3]; tpx * tpx];
    let mut valid = vec![false; tpx * tpx];
    let mut used: Vec<(usize, usize)> = Vec::new();
    let mut pixels = 0u64;
    for cz in 0..chunks {
        for cx in 0..chunks {
            let touched = (cz * cells_per_chunk..=(cz + 1) * cells_per_chunk).any(|r| {
                (cx * cells_per_chunk..=(cx + 1) * cells_per_chunk).any(|c| keep[r * corners + c])
            });
            if !touched {
                continue;
            }
            let mut any = false;
            for z in cz * cpx..(cz + 1) * cpx {
                for x in cx * cpx..(cx + 1) * cpx {
                    if !kept(x, z) {
                        continue;
                    }
                    let p = [
                        o[0] + (x as f64 + 0.5) * f64::from(pixel_m),
                        o[1] + (z as f64 + 0.5) * f64::from(pixel_m),
                    ];
                    if let Some(c) = sampler.sample(p) {
                        img[z * tpx + x] = c;
                        valid[z * tpx + x] = true;
                        any = true;
                        pixels += 1;
                    }
                }
            }
            if any {
                used.push((cx, cz));
            }
        }
    }
    if used.is_empty() {
        return None;
    }
    // No photo where the source has none (corners whose pixel is invalid).
    for k in 0..corners {
        for i in 0..corners {
            let x = ((i as f32 * WEIGHT_CELL_M / pixel_m) as usize).min(tpx - 1);
            let z = ((k as f32 * WEIGHT_CELL_M / pixel_m) as usize).min(tpx - 1);
            if !valid[z * tpx + x] {
                w[k * corners + i] = 0.0;
            }
        }
    }
    // Roofs: the photo's roofs lean beyond the footprints (the photos are
    // not true orthophotos); blurred into the ground they made red patches
    // beside the 3D houses. Their pixels count as no data and are filled
    // with the ground around them.
    mask_buildings(pkg, key, pixel_m, tpx, &mut valid);
    // Tone: local mean brightness on blocks, blurred, bilinear; the blocks'
    // mean colour also fills the pixels without data.
    let bpx = ((TONE_BLOCK_M / pixel_m).round() as usize).max(1);
    let nb = tpx.div_ceil(bpx);
    let mut sum = vec![[0.0f32; 3]; nb * nb];
    let mut cnt = vec![0u32; nb * nb];
    for z in 0..tpx {
        for x in 0..tpx {
            if valid[z * tpx + x] {
                let c = img[z * tpx + x];
                let b = (z / bpx) * nb + x / bpx;
                for k in 0..3 {
                    sum[b][k] += c[k];
                }
                cnt[b] += 1;
            }
        }
    }
    // Per block the mean colour of 3 × 3 blocks (`None`: no data near).
    let mut blur: Vec<Option<[f32; 3]>> = vec![None; nb * nb];
    for bz in 0..nb {
        for bx in 0..nb {
            let (mut s, mut n) = ([0.0f32; 3], 0u32);
            for zz in bz.saturating_sub(1)..=(bz + 1).min(nb - 1) {
                for xx in bx.saturating_sub(1)..=(bx + 1).min(nb - 1) {
                    for k in 0..3 {
                        s[k] += sum[zz * nb + xx][k];
                    }
                    n += cnt[zz * nb + xx];
                }
            }
            if n > 0 {
                blur[bz * nb + bx] = Some(s.map(|v| v / n as f32));
            }
        }
    }
    // The local mean colour at a pixel, bilinear between the blocks that
    // have one.
    let mean_at = |x: usize, z: usize| -> Option<[f32; 3]> {
        let fx = ((x as f32 + 0.5) / bpx as f32 - 0.5).clamp(0.0, (nb - 1) as f32);
        let fz = ((z as f32 + 0.5) / bpx as f32 - 0.5).clamp(0.0, (nb - 1) as f32);
        let (x0, z0) = (fx as usize, fz as usize);
        let (x1, z1) = ((x0 + 1).min(nb - 1), (z0 + 1).min(nb - 1));
        let (tx, tz) = (fx - x0 as f32, fz - z0 as f32);
        let mut acc = [0.0f32; 3];
        let mut wsum = 0.0;
        for (bx, bz, w) in [
            (x0, z0, (1.0 - tx) * (1.0 - tz)),
            (x1, z0, tx * (1.0 - tz)),
            (x0, z1, (1.0 - tx) * tz),
            (x1, z1, tx * tz),
        ] {
            if let Some(c) = blur[bz * nb + bx] {
                for k in 0..3 {
                    acc[k] += w * c[k];
                }
                wsum += w;
            }
        }
        (wsum > 1e-4).then(|| acc.map(|v| v / wsum))
    };
    let mut tile = OrthoTile {
        weights: encode_weights(&quantise_weights(&w), corners as u32),
        chunks: BTreeMap::new(),
    };
    for (cx, cz) in used {
        let mut rgb = vec![0u8; cpx * cpx * 3];
        let mut out: Vec<Option<[f32; 3]>> = vec![None; cpx * cpx];
        let (mut mean, mut n) = ([0.0f64; 3], 0u32);
        for z in 0..cpx {
            for x in 0..cpx {
                let (gx, gz) = (cx * cpx + x, cz * cpx + z);
                let local = mean_at(gx, gz);
                let c = if valid[gz * tpx + gx] {
                    Some(tone(img[gz * tpx + gx], local.unwrap_or([0.0; 3])))
                } else {
                    // Roofs and gaps: the ground around them.
                    local
                };
                if let Some(c) = c {
                    for k in 0..3 {
                        mean[k] += f64::from(c[k]);
                    }
                    n += 1;
                }
                out[z * cpx + x] = c;
            }
        }
        let fill = mean.map(|m| (m / f64::from(n.max(1))) as f32);
        for (i, c) in out.iter().enumerate() {
            let c = c.unwrap_or(fill);
            for k in 0..3 {
                rgb[i * 3 + k] = c[k].round().clamp(0.0, 255.0) as u8;
            }
        }
        match encode_jpeg(&rgb, cpx as u32, opts.quality) {
            Ok(b) => {
                tile.chunks.insert((cx as u8, cz as u8), b);
            }
            Err(e) => sampler.errors.push(e),
        }
    }
    Some((tile, pixels))
}

/// Grown around a building's footprint before its pixels are masked (m):
/// the roofs lean outwards in the photo.
pub const ROOF_PAD_M: f32 = 2.0;

/// Marks the pixels of the buildings of tile `key` (and those of its
/// neighbours reaching into it), grown by [`ROOF_PAD_M`], as no data.
fn mask_buildings(pkg: &WorldPackage, key: TileKey, pixel_m: f32, tpx: usize, valid: &mut [bool]) {
    let Some(all) = pkg.buildings.as_ref() else {
        return;
    };
    let o = key.origin(f64::from(pkg.tile_size_m));
    let o = [o[0] as f32, o[1] as f32];
    for dz in -1..=1 {
        for dx in -1..=1 {
            let k = TileKey {
                x: key.x + dx,
                z: key.z + dz,
            };
            for b in all.get(&k).into_iter().flatten() {
                let ring = &b.outline;
                let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
                for p in ring {
                    lo = [lo[0].min(p[0]), lo[1].min(p[1])];
                    hi = [hi[0].max(p[0]), hi[1].max(p[1])];
                }
                let px = |v: f32, o: f32| ((v - o) / pixel_m).floor();
                let (x0, x1) = (px(lo[0] - ROOF_PAD_M, o[0]), px(hi[0] + ROOF_PAD_M, o[0]));
                let (z0, z1) = (px(lo[1] - ROOF_PAD_M, o[1]), px(hi[1] + ROOF_PAD_M, o[1]));
                if x1 < 0.0 || z1 < 0.0 || x0 >= tpx as f32 || z0 >= tpx as f32 {
                    continue;
                }
                let clamp = |v: f32| (v.max(0.0) as usize).min(tpx - 1);
                for z in clamp(z0)..=clamp(z1) {
                    for x in clamp(x0)..=clamp(x1) {
                        let p = [
                            o[0] + (x as f32 + 0.5) * pixel_m,
                            o[1] + (z as f32 + 0.5) * pixel_m,
                        ];
                        if ortho::point_in_ring(p, ring)
                            || ortho::ring_distance(p, ring) < ROOF_PAD_M
                        {
                            valid[z * tpx + x] = false;
                        }
                    }
                }
            }
        }
    }
}

/// Baseline JPEG of a square RGB image (rows along +z, as stored).
pub fn encode_jpeg(rgb: &[u8], size: u32, quality: u8) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality.clamp(1, 100))
        .encode(rgb, size, size, image::ExtendedColorType::Rgb8)
        .map_err(|e| format!("JPEG: {e}"))?;
    Ok(out)
}

/// Cuts the orthophoto corridor of `pkg` from `imagery` into
/// `pkg.look.ortho` (section `ORTC`). The orthophoto sources must be listed
/// in the package's palette (`PALT`, written by [`crate::calibrate`]): the
/// corridor refers to them for its credits. Without orthophotos or a
/// palette nothing is cut (`None` in the stats' tiles).
pub fn cut_corridor(
    pkg: &mut WorldPackage,
    imagery: &Imagery,
    opts: CorridorOptions,
) -> CorridorStats {
    let mut stats = CorridorStats::default();
    pkg.look.ortho = None;
    if imagery.ortho.is_empty() || opts.half_width_m <= 0.0 {
        return stats;
    }
    let Some(palette) = pkg.look.palette.as_ref() else {
        stats
            .errors
            .push("no palette (PALT): calibrate the package first".into());
        return stats;
    };
    let sources: Vec<u8> = imagery
        .ortho
        .iter()
        .filter_map(|o| {
            palette
                .sources
                .iter()
                .position(|s| s.id == o.meta.id)
                .map(|i| i as u8)
        })
        .collect();
    let finest = imagery
        .ortho
        .iter()
        .map(|o| o.pixel_m as f32)
        .fold(f32::INFINITY, f32::min);
    // A pixel size that divides the chunks evenly.
    let chunk_m = pkg.tile_size_m / f32::from(CHUNKS);
    let pixel_m = chunk_m / (chunk_m / opts.pixel_m.max(finest)).floor().max(1.0);
    stats.pixel_m = pixel_m;
    let proj = LocalProjection::new(pkg.origin.0, pkg.origin.1);
    let size = f64::from(pkg.tile_size_m);
    let keys: Vec<TileKey> = pkg.tiles.keys().copied().collect();
    let threads = if opts.threads == 0 {
        std::thread::available_parallelism().map_or(4, |n| n.get())
    } else {
        opts.threads
    }
    .max(1);
    let pkg_ref = &*pkg;
    let next = std::sync::atomic::AtomicUsize::new(0);
    type Out = (Vec<(TileKey, OrthoTile, u64)>, Vec<String>);
    let parts: Vec<Out> = std::thread::scope(|sc| {
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                let (keys, next, opts) = (&keys, &next, &opts);
                sc.spawn(move || {
                    let mut s = Sampler::new(&imagery.ortho, proj, size);
                    let mut out = Vec::new();
                    // Rows of neighbouring tiles share orthophoto cells:
                    // take them in order, a few at a time.
                    loop {
                        let i = next.fetch_add(4, std::sync::atomic::Ordering::Relaxed);
                        if i >= keys.len() {
                            break;
                        }
                        for &k in &keys[i..(i + 4).min(keys.len())] {
                            if let Some((t, px)) = cut_tile(pkg_ref, k, &mut s, pixel_m, opts) {
                                out.push((k, t, px));
                            }
                        }
                    }
                    (out, s.errors)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("corridor thread"))
            .collect()
    });
    let mut tiles = BTreeMap::new();
    for (out, errors) in parts {
        for (k, t, px) in out {
            stats.pixels += px;
            tiles.insert(k, t);
        }
        stats.errors.extend(errors);
    }
    stats.errors.sort();
    stats.errors.dedup();
    let c = OrthoCorridor {
        pixel_m,
        half_width_m: opts.half_width_m,
        chunks: CHUNKS,
        weight_cells: (pkg.tile_size_m / WEIGHT_CELL_M).round() as u16,
        encoding: OrthoEncoding::Jpeg,
        sources,
        tiles,
    };
    stats.tiles = c.tiles.len();
    stats.chunks = c.chunk_count();
    stats.image_bytes = c.image_bytes();
    stats.weight_bytes = c.tiles.values().map(|t| t.weights.len()).sum();
    if !c.tiles.is_empty() {
        pkg.look.ortho = Some(c);
    }
    stats
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crs::Crs;
    use crate::imagery::tests::write_ortho;
    use ocw_world::graph::RoadClass;
    use ocw_world::look::{ImagerySource, Palette};
    use ocw_world::package::{RoadStrip, Tile};
    use ocw_world::road_dress::RoadDress;
    use ocw_world::road_surface::RoadSurface;

    #[test]
    fn toning_lifts_shadows_and_pulls_highlights() {
        let y = |c: [f32; 3]| 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
        let grey = [110.0; 3];
        let shadow = tone([40.0, 50.0, 30.0], grey);
        assert!(y(shadow) > y([40.0, 50.0, 30.0]) * 1.3);
        assert!(y(shadow) < 110.0);
        // A blue sky-lit shadow on a green meadow takes on the meadow's
        // colour.
        let meadow = [90.0, 130.0, 50.0];
        let blue = [35.0, 45.0, 60.0];
        let s = tone(blue, meadow);
        assert!(s[2] / s[1] < blue[2] / blue[1] * 0.7, "{s:?}");
        assert!(y(s) > y(blue) * 1.3);
        // Highlights (a white car) are pulled in, their hue kept.
        let car = tone([250.0, 240.0, 230.0], [100.0; 3]);
        assert!(y(car) < 235.0 && y(car) > 100.0);
        assert!((car[0] / car[2] - 250.0 / 230.0).abs() < 1e-4);
        // At the local mean nothing changes.
        let m = tone([100.0, 100.0, 100.0], [100.0; 3]);
        assert!((m[0] - 100.0).abs() < 1e-3);
        assert_eq!(tone([0.0; 3], [100.0; 3]), [0.0; 3]);
    }

    #[test]
    fn jpeg_chunks_decode_to_their_colours() {
        let n = 64u32;
        let mut rgb = Vec::new();
        for z in 0..n {
            for _ in 0..n {
                rgb.extend(if z < 32 { [200, 40, 40] } else { [40, 160, 40] });
            }
        }
        let j = encode_jpeg(&rgb, n, 85).unwrap();
        assert!(j.len() < rgb.len() / 10, "{}", j.len());
        let d = image::load_from_memory(&j).unwrap().to_rgb8();
        assert_eq!(d.dimensions(), (n, n));
        let p = d.get_pixel(10, 5).0;
        assert!((i32::from(p[0]) - 200).abs() < 12 && (i32::from(p[1]) - 40).abs() < 12);
        let q = d.get_pixel(10, 60).0;
        assert!((i32::from(q[1]) - 160).abs() < 12, "{q:?}");
    }

    /// A small package in Vaduz with one road through tile (0, 0) and a
    /// synthetic orthophoto: the corridor holds only chunks near the road,
    /// the photo's colours at the right place (rows along +z), its weights
    /// and its source.
    #[test]
    fn the_corridor_is_cut_along_the_road_from_the_orthophoto() {
        let dir = tempfile::tempdir().unwrap();
        let proj = LocalProjection::new(47.14, 9.52);
        let tile = 1000.0;
        // The tile's cells (1 km in LV95) and an image: red north of the
        // local z = 500 line, green south of it... in CRS terms: by the
        // local z of each pixel's centre.
        let tiles = [TileKey { x: 0, z: 0 }];
        let cells: Vec<(i64, i64)> =
            crate::imagery::package_cells(tiles, &proj, Crs::Lv95, tile, 1000.0, 60.0)
                .into_iter()
                .collect();
        write_ortho(dir.path(), &cells, 1000.0, 2.5, |e, n| {
            let (lat, lon) = Crs::Lv95.inverse(e, n);
            let p = proj.project(lat, lon);
            // A red roof (house at x 400 … 412, z 320 … 332), leaning 1 m
            // beyond its footprint in the photo.
            let roof = (399.0..413.0).contains(&p[0]) && (319.0..333.0).contains(&p[1]);
            if p[1] > 500.0 || roof {
                [200, 40, 40]
            } else {
                [40, 160, 40]
            }
        });
        let src = OrthoSource::load(dir.path()).unwrap();
        let imagery = Imagery {
            ortho: vec![src],
            seasonal: None,
        };
        let road = RoadStrip {
            class: RoadClass::Residential,
            width_m: 6.0,
            points: vec![[-20.0, 0.0, 312.0], [1020.0, 0.0, 312.0]],
            surface: RoadSurface::of_class(RoadClass::Residential),
            dress: RoadDress::of_class(RoadClass::Residential, 6.0),
        };
        let mut t = BTreeMap::new();
        t.insert(
            tiles[0],
            Tile {
                key: tiles[0],
                heights: vec![0.0; 4],
                roads: vec![road],
            },
        );
        let mut pkg = WorldPackage {
            name: "t".into(),
            origin: (47.14, 9.52),
            tile_size_m: 1000.0,
            samples: 2,
            tiles: t,
            graph: None,
            landuse: None,
            buildings: Some(BTreeMap::from([(
                tiles[0],
                vec![ocw_world::buildings::Building {
                    kind: ocw_world::buildings::BuildingKind::House,
                    outline: vec![
                        [400.0, 320.0],
                        [412.0, 320.0],
                        [412.0, 332.0],
                        [400.0, 332.0],
                    ],
                    holes: vec![],
                    base_y: 0.0,
                    eave_y: 6.0,
                    roof: ocw_world::buildings::Roof::Flat,
                    tags: Default::default(),
                }],
            )])),
            bridges: None,
            routes: vec![],
            region: None,
            time_zone: None,
            look: Default::default(),
        };
        // Without a palette: nothing, with a reason.
        let s = cut_corridor(&mut pkg, &imagery, CorridorOptions::default());
        assert_eq!(s.tiles, 0);
        assert!(!s.errors.is_empty());
        pkg.look.palette = Some(Palette {
            sources: vec![ImagerySource {
                id: "test-ortho".into(),
                ..Default::default()
            }],
            entries: vec![],
        });
        let s = cut_corridor(
            &mut pkg,
            &imagery,
            CorridorOptions {
                threads: 2,
                ..Default::default()
            },
        );
        assert!(s.errors.is_empty(), "{:?}", s.errors);
        assert_eq!(s.tiles, 1);
        // The source is 2.5 m per pixel: the corridor takes 2.5 m too.
        assert_eq!(s.pixel_m, 2.5);
        let c = pkg.look.ortho.as_ref().unwrap();
        assert_eq!(c.sources, vec![0]);
        let ot = &c.tiles[&tiles[0]];
        // Road at z = 312 ± 3, corridor ±50 m + 4 m margin: chunk row 2
        // (250 … 375 m) only, all eight columns.
        let rows: std::collections::BTreeSet<u8> = ot.chunks.keys().map(|k| k.1).collect();
        assert_eq!(rows.into_iter().collect::<Vec<_>>(), vec![2]);
        assert_eq!(ot.chunks.len(), 8);
        // The chunk decodes to the photo: green (south of z = 500).
        let d = image::load_from_memory(&ot.chunks[&(3, 2)])
            .unwrap()
            .to_rgb8();
        let px = c.chunk_px(1000.0);
        assert_eq!(d.dimensions(), (px, px));
        // Row 25 is z = 250 + 62.5 m: on the road.
        let p = d.get_pixel(10, 25).0;
        assert!(p[1] > p[0] + 60, "{p:?}");
        // The roof (x 406, z 326: column 12, row 30) and its leaning edge
        // (x 399.5) are filled with the ground around them: no red.
        for (col, row) in [(12, 30), (9, 30)] {
            let q = d.get_pixel(col, row).0;
            assert!(q[1] > q[0] + 40, "{q:?} at {col} {row}");
        }
        // Weights: full at the road, none 100 m away.
        let w = c.weights(tiles[0]).unwrap().unwrap();
        let n = usize::from(c.weight_cells) + 1;
        assert_eq!(w[78 * n + 100], 255);
        assert_eq!(w[103 * n + 100], 0);
        // The package round-trips with it.
        let back = WorldPackage::from_bytes(&pkg.to_bytes()).unwrap();
        assert_eq!(back.look.ortho, pkg.look.ortho);
    }
}
