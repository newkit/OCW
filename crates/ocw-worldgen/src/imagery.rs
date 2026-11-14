//! Open imagery for region calibration (spec §14 Phase 8 T3, 8B): the
//! orthophotos and the seasonal Sentinel-2 composites that
//! `scripts/fetch-imagery.sh` downloads into `data/<region>/imagery/`, read
//! at local package metres.
//!
//! Each source is a folder with an `index.json` (written by
//! `scripts/imagery.py`):
//!
//! - **Orthophoto** (`"kind": "ortho"`): square cells of `cell_m` metres in
//!   a projected CRS, one image per cell named `E_N.jpg` (or `.rgb`, raw
//!   8-bit RGB rows from the north) after its lower-left corner in km, at
//!   `pixel_m` metres per pixel; colours listed in `nodata` (within
//!   `nodata_tolerance`) are no data (outside the service's coverage).
//! - **Seasonal composites** (`"kind": "seasonal"`): one raster per season
//!   (raw 8-bit RGB, `width × height` pixels of `pixel_m` from the
//!   north-west corner `west`, `north`), black (0, 0, 0) where no clear
//!   scene was found.
//!
//! Both carry the source's attribution (`name`, `licence`, `notice`,
//! `url`, `date`), which goes into the package (`PALT`) and its manifest.
//!
//! Local metres map to the CRS through the package's [`LocalProjection`];
//! within a 1 km tile that map is affine to well under a centimetre, so
//! [`OrthoReader`] projects three corners per tile instead of every pixel.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ocw_world::TileKey;
use ocw_world::geo::LocalProjection;
use ocw_world::look::{ImagerySource, Season, SeasonInfo};
use serde::Deserialize;

use crate::crs::Crs;

/// `index.json` of an imagery folder.
#[derive(Debug, Clone, Deserialize)]
pub struct IndexFile {
    pub kind: String,
    pub id: String,
    pub name: String,
    pub licence: String,
    pub notice: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub date: String,
    pub crs: String,
    pub pixel_m: f64,
    // Orthophoto cells.
    #[serde(default)]
    pub cell_m: Option<f64>,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub nodata: Vec<[u8; 3]>,
    #[serde(default)]
    pub nodata_tolerance: u8,
    // Seasonal rasters.
    #[serde(default)]
    pub west: Option<f64>,
    #[serde(default)]
    pub north: Option<f64>,
    #[serde(default)]
    pub width: Option<usize>,
    #[serde(default)]
    pub height: Option<usize>,
    #[serde(default)]
    pub seasons: Vec<SeasonEntry>,
}

/// One seasonal composite in `index.json`.
#[derive(Debug, Clone, Deserialize)]
pub struct SeasonEntry {
    pub season: String,
    pub centre_doy: u16,
    pub first_doy: u16,
    pub last_doy: u16,
    #[serde(default)]
    pub scenes: u16,
    pub file: String,
}

impl IndexFile {
    pub fn source(&self) -> ImagerySource {
        ImagerySource {
            id: self.id.clone(),
            name: self.name.clone(),
            licence: self.licence.clone(),
            notice: self.notice.clone(),
            url: self.url.clone(),
            date: self.date.clone(),
            resolution_m: self.pixel_m as f32,
        }
    }
}

/// Image format of orthophoto cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellFormat {
    Jpeg,
    /// Raw 8-bit RGB, rows from the north (test fixtures).
    Rgb,
}

impl CellFormat {
    fn ext(self) -> &'static str {
        match self {
            CellFormat::Jpeg => "jpg",
            CellFormat::Rgb => "rgb",
        }
    }
}

/// An orthophoto in square cells.
#[derive(Debug, Clone)]
pub struct OrthoSource {
    pub meta: ImagerySource,
    pub crs: Crs,
    pub cell_m: f64,
    pub pixel_m: f64,
    pub format: CellFormat,
    pub dir: PathBuf,
    /// Cells present, by lower-left corner in cell units.
    pub cells: HashSet<(i64, i64)>,
    pub nodata: Vec<[u8; 3]>,
    pub nodata_tolerance: u8,
}

/// Seasonal composites on one raster grid.
#[derive(Debug, Clone)]
pub struct SeasonalSource {
    pub meta: ImagerySource,
    pub crs: Crs,
    pub pixel_m: f64,
    pub west: f64,
    pub north: f64,
    pub width: usize,
    pub height: usize,
    /// Per season its info and `width × height` RGB.
    pub composites: Vec<(SeasonInfo, Vec<u8>)>,
}

/// All imagery of a region, orthophotos best first.
#[derive(Debug, Clone, Default)]
pub struct Imagery {
    pub ortho: Vec<OrthoSource>,
    pub seasonal: Option<SeasonalSource>,
}

fn read_index(dir: &Path) -> Result<IndexFile, String> {
    let p = dir.join("index.json");
    let text = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", p.display()))
}

impl OrthoSource {
    pub fn load(dir: &Path) -> Result<Self, String> {
        let ix = read_index(dir)?;
        Self::from_index(dir, &ix)
    }

    fn from_index(dir: &Path, ix: &IndexFile) -> Result<Self, String> {
        let crs = Crs::parse(&ix.crs)?;
        let cell_m = ix.cell_m.ok_or("ortho index.json: cell_m missing")?;
        let format = match ix.format.as_deref().unwrap_or("jpeg") {
            "jpeg" | "jpg" => CellFormat::Jpeg,
            "rgb" => CellFormat::Rgb,
            f => return Err(format!("ortho index.json: unknown format {f}")),
        };
        let mut cells = HashSet::new();
        let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for e in entries.flatten() {
            let name = e.file_name();
            let Some(stem) = name
                .to_str()
                .and_then(|n| n.strip_suffix(&format!(".{}", format.ext())))
            else {
                continue;
            };
            if let Some((a, b)) = stem.split_once('_')
                && let (Ok(e_km), Ok(n_km)) = (a.parse::<f64>(), b.parse::<f64>())
            {
                cells.insert((
                    (e_km * 1000.0 / cell_m).round() as i64,
                    (n_km * 1000.0 / cell_m).round() as i64,
                ));
            }
        }
        Ok(Self {
            meta: ix.source(),
            crs,
            cell_m,
            pixel_m: ix.pixel_m,
            format,
            dir: dir.to_path_buf(),
            cells,
            nodata: ix.nodata.clone(),
            nodata_tolerance: ix.nodata_tolerance,
        })
    }

    /// Pixels per cell side.
    pub fn cell_px(&self) -> usize {
        (self.cell_m / self.pixel_m).round() as usize
    }

    fn cell_path(&self, (i, j): (i64, i64)) -> PathBuf {
        let km = |v: i64| {
            let m = v as f64 * self.cell_m / 1000.0;
            if m.fract() == 0.0 {
                format!("{}", m as i64)
            } else {
                format!("{m}")
            }
        };
        self.dir
            .join(format!("{}_{}.{}", km(i), km(j), self.format.ext()))
    }

    /// Decodes a cell into `cell_px²` RGB (rows from the north).
    pub fn decode_cell(&self, cell: (i64, i64)) -> Result<Vec<u8>, String> {
        let path = self.cell_path(cell);
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let n = self.cell_px();
        let rgb = match self.format {
            CellFormat::Rgb => bytes,
            CellFormat::Jpeg => {
                let opts = zune_jpeg::zune_core::options::DecoderOptions::default()
                    .jpeg_set_out_colorspace(zune_jpeg::zune_core::colorspace::ColorSpace::RGB);
                let mut d = zune_jpeg::JpegDecoder::new_with_options(
                    zune_jpeg::zune_core::bytestream::ZCursor::new(bytes),
                    opts,
                );
                d.decode()
                    .map_err(|e| format!("{}: {e:?}", path.display()))?
            }
        };
        if rgb.len() != n * n * 3 {
            return Err(format!(
                "{}: {} bytes, expected {n} × {n} RGB",
                path.display(),
                rgb.len()
            ));
        }
        Ok(rgb)
    }

    pub(crate) fn is_nodata(&self, px: [u8; 3]) -> bool {
        let t = i16::from(self.nodata_tolerance);
        self.nodata
            .iter()
            .any(|nd| (0..3).all(|k| (i16::from(px[k]) - i16::from(nd[k])).abs() <= t))
    }
}

impl SeasonalSource {
    pub fn load(dir: &Path) -> Result<Self, String> {
        let ix = read_index(dir)?;
        let crs = Crs::parse(&ix.crs)?;
        let need =
            |v: Option<f64>, what: &str| v.ok_or(format!("seasonal index.json: {what} missing"));
        let (west, north) = (need(ix.west, "west")?, need(ix.north, "north")?);
        let width = ix.width.ok_or("seasonal index.json: width missing")?;
        let height = ix.height.ok_or("seasonal index.json: height missing")?;
        let mut composites = Vec::new();
        for s in &ix.seasons {
            let season = Season::parse(&s.season).ok_or(format!("unknown season {}", s.season))?;
            let path = dir.join(&s.file);
            let rgb = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            if rgb.len() != width * height * 3 {
                return Err(format!(
                    "{}: {} bytes, expected {width} × {height} RGB",
                    path.display(),
                    rgb.len()
                ));
            }
            composites.push((
                SeasonInfo {
                    season,
                    centre_doy: s.centre_doy,
                    first_doy: s.first_doy,
                    last_doy: s.last_doy,
                    scenes: s.scenes,
                },
                rgb,
            ));
        }
        composites.sort_by_key(|c| c.0.season);
        Ok(Self {
            meta: ix.source(),
            crs,
            pixel_m: ix.pixel_m,
            west,
            north,
            width,
            height,
            composites,
        })
    }

    /// The composite of `season`, if built.
    pub fn index_of(&self, season: Season) -> Option<usize> {
        self.composites.iter().position(|c| c.0.season == season)
    }

    /// Bilinear colour of composite `i` at CRS `(e, n)`, `None` outside the
    /// raster or where a neighbour pixel has no data.
    pub fn sample(&self, i: usize, e: f64, n: f64) -> Option<[u8; 3]> {
        let rgb = &self.composites.get(i)?.1;
        let fx = (e - self.west) / self.pixel_m - 0.5;
        let fy = (self.north - n) / self.pixel_m - 0.5;
        if !(fx >= 0.0 && fy >= 0.0) {
            return None;
        }
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
        if x0 >= self.width || y0 >= self.height {
            return None;
        }
        let (tx, ty) = (fx - x0 as f64, fy - y0 as f64);
        let px = |x: usize, y: usize| {
            let k = (y * self.width + x) * 3;
            [rgb[k], rgb[k + 1], rgb[k + 2]]
        };
        let corners = [px(x0, y0), px(x1, y0), px(x0, y1), px(x1, y1)];
        if corners.contains(&[0, 0, 0]) {
            return None;
        }
        let w = [
            (1.0 - tx) * (1.0 - ty),
            tx * (1.0 - ty),
            (1.0 - tx) * ty,
            tx * ty,
        ];
        let mut out = [0u8; 3];
        for k in 0..3 {
            let v: f64 = corners
                .iter()
                .zip(w)
                .map(|(c, w)| f64::from(c[k]) * w)
                .sum();
            out[k] = v.round().clamp(0.0, 255.0) as u8;
        }
        Some(out)
    }
}

impl Imagery {
    /// Reads every source folder under `root` (folders with an
    /// `index.json`); orthophotos in the order of their folder names
    /// (prefix them `1-`, `2-` … to rank them), at most one seasonal source.
    pub fn load(root: &Path) -> Result<Self, String> {
        let mut dirs: Vec<PathBuf> = std::fs::read_dir(root)
            .map_err(|e| format!("{}: {e}", root.display()))?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.join("index.json").is_file())
            .collect();
        dirs.sort();
        let mut out = Imagery::default();
        for d in dirs {
            let ix = read_index(&d)?;
            match ix.kind.as_str() {
                "ortho" => out.ortho.push(OrthoSource::from_index(&d, &ix)?),
                "seasonal" if out.seasonal.is_none() => {
                    out.seasonal = Some(SeasonalSource::load(&d)?);
                }
                "seasonal" => return Err(format!("{}: a second seasonal source", d.display())),
                k => return Err(format!("{}: unknown kind {k}", d.display())),
            }
        }
        Ok(out)
    }

    pub fn is_empty(&self) -> bool {
        self.ortho.is_empty() && self.seasonal.is_none()
    }
}

/// Local metres of one tile → a CRS, as an affine map fitted at three
/// corners of the tile.
#[derive(Debug, Clone, Copy)]
pub struct TileAffine {
    origin: [f64; 2],
    o: [f64; 2],
    ex: [f64; 2],
    ez: [f64; 2],
}

impl TileAffine {
    pub fn new(proj: &LocalProjection, crs: Crs, key: TileKey, size: f64) -> Self {
        let origin = key.origin(size);
        let at = |dx: f64, dz: f64| {
            let (lat, lon) = proj.unproject([origin[0] + dx, origin[1] + dz]);
            crs.forward(lat, lon)
        };
        let (o, x, z) = (at(0.0, 0.0), at(size, 0.0), at(0.0, size));
        Self {
            origin,
            o,
            ex: [(x[0] - o[0]) / size, (x[1] - o[1]) / size],
            ez: [(z[0] - o[0]) / size, (z[1] - o[1]) / size],
        }
    }

    /// The CRS point of local `p`.
    pub fn apply(&self, p: [f64; 2]) -> [f64; 2] {
        let (dx, dz) = (p[0] - self.origin[0], p[1] - self.origin[1]);
        [
            self.o[0] + dx * self.ex[0] + dz * self.ez[0],
            self.o[1] + dx * self.ex[1] + dz * self.ez[1],
        ]
    }
}

/// Colour at local metres, for the calibration passes (orthophotos or a
/// test image).
pub trait ColourSampler {
    /// The colour at local `p` and the index of the source it came from.
    fn sample(&mut self, p: [f64; 2]) -> Option<([u8; 3], u8)>;
    /// Ground size of a pixel (m): the sampling step inside footprints.
    fn pixel_m(&self) -> f64;
}

/// A decoded orthophoto cell in the reader's cache: (source, cell) and its
/// pixels (`None`: it failed to decode).
type CachedCell = ((usize, (i64, i64)), Option<Arc<Vec<u8>>>);

/// Reads orthophoto pixels at local metres, best source first, with a
/// small cache of decoded cells (one reader per thread).
pub struct OrthoReader<'a> {
    sources: &'a [OrthoSource],
    proj: LocalProjection,
    tile_size: f64,
    affine: Option<(TileKey, Vec<TileAffine>)>,
    cache: Vec<CachedCell>,
    cache_cap: usize,
    /// Cells that failed to decode (reported once).
    pub errors: Vec<String>,
}

impl<'a> OrthoReader<'a> {
    pub fn new(sources: &'a [OrthoSource], proj: LocalProjection, tile_size: f64) -> Self {
        Self {
            sources,
            proj,
            tile_size,
            affine: None,
            cache: Vec::new(),
            cache_cap: 12,
            errors: Vec::new(),
        }
    }

    fn cell(&mut self, s: usize, cell: (i64, i64)) -> Option<Arc<Vec<u8>>> {
        if let Some(i) = self.cache.iter().position(|c| c.0 == (s, cell)) {
            let hit = self.cache.remove(i);
            let v = hit.1.clone();
            self.cache.push(hit);
            return v;
        }
        let v = match self.sources[s].decode_cell(cell) {
            Ok(v) => Some(Arc::new(v)),
            Err(e) => {
                self.errors.push(e);
                None
            }
        };
        if self.cache.len() >= self.cache_cap {
            self.cache.remove(0);
        }
        self.cache.push(((s, cell), v.clone()));
        v
    }
}

impl ColourSampler for OrthoReader<'_> {
    fn sample(&mut self, p: [f64; 2]) -> Option<([u8; 3], u8)> {
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
            let src = &self.sources[s];
            let [e, n] = self.affine.as_ref().expect("set above").1[s].apply(p);
            let cell = (
                (e / src.cell_m).floor() as i64,
                (n / src.cell_m).floor() as i64,
            );
            if !src.cells.contains(&cell) {
                continue;
            }
            let px_n = src.cell_px();
            let col = ((e - cell.0 as f64 * src.cell_m) / src.pixel_m) as usize;
            let row = (((cell.1 + 1) as f64 * src.cell_m - n) / src.pixel_m) as usize;
            let (col, row) = (col.min(px_n - 1), row.min(px_n - 1));
            let Some(img) = self.cell(s, cell) else {
                continue;
            };
            let k = (row * px_n + col) * 3;
            let px = [img[k], img[k + 1], img[k + 2]];
            if !self.sources[s].is_nodata(px) {
                return Some((px, s as u8));
            }
        }
        None
    }

    fn pixel_m(&self) -> f64 {
        self.sources
            .iter()
            .map(|s| s.pixel_m)
            .fold(f64::INFINITY, f64::min)
    }
}

/// The imagery cells (lower-left corner in km, `E_N`) of `cell_m` metres
/// in `crs` that cover the package's tiles plus `margin_m`, for scripted
/// downloads of only what a package needs.
pub fn package_cells(
    tiles: impl IntoIterator<Item = TileKey>,
    proj: &LocalProjection,
    crs: Crs,
    tile_size: f64,
    cell_m: f64,
    margin_m: f64,
) -> std::collections::BTreeSet<(i64, i64)> {
    let mut out = std::collections::BTreeSet::new();
    for key in tiles {
        let o = key.origin(tile_size);
        let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        for i in 0..=4 {
            for j in 0..=4 {
                let p = [
                    o[0] - margin_m + (tile_size + 2.0 * margin_m) * f64::from(i) / 4.0,
                    o[1] - margin_m + (tile_size + 2.0 * margin_m) * f64::from(j) / 4.0,
                ];
                let (lat, lon) = proj.unproject(p);
                let q = crs.forward(lat, lon);
                for k in 0..2 {
                    lo[k] = lo[k].min(q[k]);
                    hi[k] = hi[k].max(q[k]);
                }
            }
        }
        let (i0, i1) = (
            (lo[0] / cell_m).floor() as i64,
            (hi[0] / cell_m).floor() as i64,
        );
        let (j0, j1) = (
            (lo[1] / cell_m).floor() as i64,
            (hi[1] / cell_m).floor() as i64,
        );
        for i in i0..=i1 {
            for j in j0..=j1 {
                out.insert((i, j));
            }
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Writes an orthophoto fixture: `cells` (E_N in km) of `cell_m` at
    /// `pixel_m` in LV95, coloured by `colour(e, n)` at each pixel centre.
    pub fn write_ortho(
        dir: &Path,
        cells: &[(i64, i64)],
        cell_m: f64,
        pixel_m: f64,
        colour: impl Fn(f64, f64) -> [u8; 3],
    ) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join("index.json"),
            format!(
                r#"{{"kind":"ortho","id":"test-ortho","name":"Test orthophoto","licence":"CC0",
                "notice":"test","crs":"lv95","cell_m":{cell_m},"pixel_m":{pixel_m},"format":"rgb",
                "nodata":[[255,255,255]],"nodata_tolerance":2}}"#
            ),
        )
        .unwrap();
        let n = (cell_m / pixel_m).round() as usize;
        for &(i, j) in cells {
            let mut rgb = Vec::with_capacity(n * n * 3);
            for row in 0..n {
                for col in 0..n {
                    let e = i as f64 * cell_m + (col as f64 + 0.5) * pixel_m;
                    let nn = (j + 1) as f64 * cell_m - (row as f64 + 0.5) * pixel_m;
                    rgb.extend(colour(e, nn));
                }
            }
            let km = |v: i64| (v as f64 * cell_m / 1000.0).to_string();
            std::fs::write(dir.join(format!("{}_{}.rgb", km(i), km(j))), rgb).unwrap();
        }
    }

    #[test]
    fn ortho_cells_are_read_at_local_metres() {
        let dir = tempfile::tempdir().unwrap();
        // Vaduz: the package origin and its LV95 point.
        let proj = LocalProjection::new(47.14, 9.52);
        let at0 = Crs::Lv95.forward(47.14, 9.52);
        let cell_m = 100.0;
        let ci = (
            (at0[0] / cell_m).floor() as i64,
            (at0[1] / cell_m).floor() as i64,
        );
        // Red east of the cell's middle, green west of it; white (no data)
        // in the northern quarter.
        let mid = (ci.0 as f64 + 0.5) * cell_m;
        let north = (ci.1 as f64 + 0.75) * cell_m;
        write_ortho(dir.path(), &[ci], cell_m, 1.0, |e, n| {
            if n > north {
                [255, 255, 255]
            } else if e > mid {
                [200, 30, 30]
            } else {
                [30, 200, 30]
            }
        });
        let src = OrthoSource::load(dir.path()).unwrap();
        assert_eq!(src.cells.len(), 1);
        assert_eq!(src.meta.licence, "CC0");
        let sources = vec![src];
        let mut r = OrthoReader::new(&sources, proj, 1000.0);
        // The local point of a CRS point, by inverting through lat/lon.
        let local = |e: f64, n: f64| {
            let (lat, lon) = Crs::Lv95.inverse(e, n);
            proj.project(lat, lon)
        };
        let base_n = ci.1 as f64 * cell_m;
        assert_eq!(
            r.sample(local(mid + 10.0, base_n + 30.0)).unwrap().0,
            [200, 30, 30]
        );
        assert_eq!(
            r.sample(local(mid - 10.0, base_n + 30.0)).unwrap().0,
            [30, 200, 30]
        );
        // No data, and outside every cell.
        assert!(r.sample(local(mid + 10.0, base_n + 90.0)).is_none());
        assert!(r.sample(local(mid + 10.0, base_n - 30.0)).is_none());
        assert!(r.errors.is_empty());
    }

    #[test]
    fn package_cells_cover_the_tiles() {
        let proj = LocalProjection::new(47.14, 9.52);
        let tiles = [TileKey { x: 0, z: 0 }, TileKey { x: 3, z: -2 }];
        let cells = package_cells(tiles, &proj, Crs::Lv95, 1000.0, 1000.0, 50.0);
        // Every corner of the tiles lies in a listed cell.
        for k in tiles {
            let o = k.origin(1000.0);
            for p in [o, [o[0] + 999.0, o[1] + 999.0], [o[0], o[1] + 999.0]] {
                let (lat, lon) = proj.unproject(p);
                let q = Crs::Lv95.forward(lat, lon);
                let c = (
                    (q[0] / 1000.0).floor() as i64,
                    (q[1] / 1000.0).floor() as i64,
                );
                assert!(cells.contains(&c), "{c:?} for {p:?}");
            }
        }
        // Two tiles, each at most 2 × 2 cells (+ margin): no box fill between them.
        assert!(cells.len() <= 18, "{}", cells.len());
    }

    #[test]
    fn seasonal_rasters_sample_bilinear_and_skip_no_data() {
        let dir = tempfile::tempdir().unwrap();
        let (w, h) = (4usize, 3usize);
        let mut rgb = vec![100u8; w * h * 3];
        // Pixel (3, 2) has no data.
        let k = (2 * w + 3) * 3;
        rgb[k..k + 3].copy_from_slice(&[0, 0, 0]);
        // Pixel (1, 0) is brighter.
        rgb[3..6].copy_from_slice(&[200, 200, 200]);
        std::fs::write(dir.path().join("summer.rgb"), &rgb).unwrap();
        std::fs::write(
            dir.path().join("index.json"),
            r#"{"kind":"seasonal","id":"s2","name":"Sentinel-2","licence":"Copernicus",
            "notice":"Contains modified Copernicus Sentinel data","crs":"utm32","pixel_m":20,
            "west":1000,"north":5000,"width":4,"height":3,
            "seasons":[{"season":"summer","centre_doy":196,"first_doy":166,"last_doy":243,"scenes":9,"file":"summer.rgb"}]}"#,
        )
        .unwrap();
        let s = SeasonalSource::load(dir.path()).unwrap();
        assert_eq!(s.index_of(Season::Summer), Some(0));
        assert_eq!(s.index_of(Season::Winter), None);
        // Centre of pixel (0, 0): exact.
        assert_eq!(s.sample(0, 1010.0, 4990.0), Some([100; 3]));
        // Half-way between pixels (0, 0) and (1, 0).
        assert_eq!(s.sample(0, 1020.0, 4990.0), Some([150; 3]));
        // Next to the no-data pixel, and outside.
        assert_eq!(s.sample(0, 1065.0, 4945.0), None);
        assert_eq!(s.sample(0, 990.0, 4990.0), None);
    }
}
