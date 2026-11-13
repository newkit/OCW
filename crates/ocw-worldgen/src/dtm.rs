//! High-resolution DTM grids (spec §4.3, "Regional high-resolution open
//! elevation models"): tiles of a regular grid in a projected CRS, read
//! from GeoTIFF (swissALTI3D, our own 2 m imports) or XYZ text (LGL DGM1),
//! block-averaged to a coarser cell, and written as GeoTIFF. Pure Rust
//! (`tiff` crate, LZW / Deflate).
//!
//! **Layout:** a tile holds `nx × ny` heights (`f32`, NaN = no data), rows
//! from the north; the sample (i, j) is the cell centre at
//! `x0 + (i + ½)·cell`, `y0 − (j + ½)·cell` (GeoTIFF "pixel is area").
//! Tiles of a [`GridLayer`] share the CRS, cell and tile size and sit on
//! one lattice, so a sample has a global index and bilinear sampling
//! crosses tile edges seamlessly.

use std::collections::{HashMap, VecDeque};
use std::path::Path;

use crate::crs::Crs;
use crate::dem::DemError;

/// No-data value written into GeoTIFFs (`GDAL_NODATA`).
pub const NODATA: f32 = -9999.0;

/// Coarse coverage cells for the feathering (m).
const COVERAGE_CELL_M: f64 = 50.0;

/// One tile of a projected elevation grid (see the module docs).
#[derive(Debug, Clone, PartialEq)]
pub struct GridTile {
    pub crs: Crs,
    pub cell: f64,
    /// West edge of the first column, north edge of the first row.
    pub x0: f64,
    pub y0: f64,
    pub nx: usize,
    pub ny: usize,
    pub data: Vec<f32>,
}

fn format(msg: impl Into<String>) -> DemError {
    DemError::Format(msg.into())
}

impl GridTile {
    pub fn get(&self, i: usize, j: usize) -> Option<f32> {
        let v = self.data[j * self.nx + i];
        (!v.is_nan()).then_some(v)
    }

    /// (west, south, east, north) of the tile.
    pub fn bounds(&self) -> [f64; 4] {
        [
            self.x0,
            self.y0 - self.ny as f64 * self.cell,
            self.x0 + self.nx as f64 * self.cell,
            self.y0,
        ]
    }

    /// Share of samples with data.
    pub fn valid_fraction(&self) -> f64 {
        let n = self.data.iter().filter(|v| !v.is_nan()).count();
        n as f64 / self.data.len().max(1) as f64
    }

    /// Averaged over `factor × factor` blocks (cell × factor); a block's
    /// voids are left out, a block without data is a void. The tile must
    /// divide evenly.
    pub fn resampled(&self, factor: usize) -> Result<Self, DemError> {
        if factor <= 1 {
            return Ok(self.clone());
        }
        if !self.nx.is_multiple_of(factor) || !self.ny.is_multiple_of(factor) {
            return Err(format(format!(
                "{} × {} samples do not divide into {factor} × {factor} blocks",
                self.nx, self.ny
            )));
        }
        let (nx, ny) = (self.nx / factor, self.ny / factor);
        let mut data = Vec::with_capacity(nx * ny);
        for bj in 0..ny {
            for bi in 0..nx {
                let (mut sum, mut n) = (0.0f64, 0u32);
                for j in bj * factor..(bj + 1) * factor {
                    for v in &self.data[j * self.nx + bi * factor..j * self.nx + (bi + 1) * factor]
                    {
                        if !v.is_nan() {
                            sum += f64::from(*v);
                            n += 1;
                        }
                    }
                }
                data.push(if n > 0 {
                    (sum / f64::from(n)) as f32
                } else {
                    f32::NAN
                });
            }
        }
        Ok(Self {
            cell: self.cell * factor as f64,
            nx,
            ny,
            data,
            ..*self
        })
    }

    /// Parses an XYZ text grid (`x y z` per line, cell centres, any order;
    /// e.g. LGL DGM1 `dgm1_32_E_N_1_bw_YEAR.xyz`). The cell size is the
    /// smallest spacing of the x values; missing points are voids.
    pub fn from_xyz(text: &str, crs: Crs) -> Result<Self, DemError> {
        let mut pts: Vec<[f64; 3]> = Vec::new();
        for (k, line) in text.lines().enumerate() {
            let mut it = line.split_ascii_whitespace();
            let (Some(x), Some(y), Some(z)) = (it.next(), it.next(), it.next()) else {
                if line.trim().is_empty() {
                    continue;
                }
                return Err(format(format!("line {}: expected x y z", k + 1)));
            };
            let num = |s: &str| {
                s.parse::<f64>()
                    .map_err(|e| format(format!("line {}: {s}: {e}", k + 1)))
            };
            pts.push([num(x)?, num(y)?, num(z)?]);
        }
        if pts.len() < 4 {
            return Err(format("an XYZ grid needs at least 4 points"));
        }
        let mut xs: Vec<f64> = pts.iter().map(|p| p[0]).collect();
        xs.sort_by(f64::total_cmp);
        xs.dedup();
        let cell = xs
            .windows(2)
            .map(|w| w[1] - w[0])
            .fold(f64::INFINITY, f64::min);
        if !cell.is_finite() || cell <= 0.0 {
            return Err(format("cannot tell the cell size (one column only)"));
        }
        let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        for p in &pts {
            lo = [lo[0].min(p[0]), lo[1].min(p[1])];
            hi = [hi[0].max(p[0]), hi[1].max(p[1])];
        }
        let nx = ((hi[0] - lo[0]) / cell).round() as usize + 1;
        let ny = ((hi[1] - lo[1]) / cell).round() as usize + 1;
        let (x0, y0) = (lo[0] - cell / 2.0, hi[1] + cell / 2.0);
        let mut data = vec![f32::NAN; nx * ny];
        for p in &pts {
            let i = ((p[0] - lo[0]) / cell).round() as usize;
            let j = ((hi[1] - p[1]) / cell).round() as usize;
            data[j * nx + i] = p[2] as f32;
        }
        Ok(Self {
            crs,
            cell,
            x0,
            y0,
            nx,
            ny,
            data,
        })
    }

    pub fn read_xyz(path: &Path, crs: Crs) -> Result<Self, DemError> {
        Self::from_xyz(&std::fs::read_to_string(path)?, crs)
    }

    /// Reads a single-band GeoTIFF (striped or tiled, uncompressed, LZW
    /// or Deflate; integer or float samples) with its georeference:
    /// `ModelPixelScale`, `ModelTiepoint`, the EPSG code of
    /// `ProjectedCSTypeGeoKey` and `GDAL_NODATA`. `crs` overrides a
    /// missing or unknown EPSG code.
    ///
    /// Tiled float images whose strict decoding fails (cloud-optimised
    /// GeoTIFFs with empty "sparse" tiles or LZW tiles without an end code,
    /// e.g. some swissALTI3D 2026 tiles) are decoded tile by tile instead,
    /// missing data as voids, as GDAL does.
    pub fn read_geotiff(path: &Path, crs: Option<Crs>) -> Result<Self, DemError> {
        let bytes = std::fs::read(path)?;
        let mut header = GeoHeader::read(std::io::Cursor::new(&bytes[..]), crs)?;
        match header.read_data() {
            Ok(t) => Ok(t),
            Err(strict) => header
                .read_tiles_lenient(&bytes)
                .map_err(|e| format(format!("{strict}; tile by tile: {e}"))),
        }
    }

    /// Only the georeference of a GeoTIFF (no pixel data): to skip tiles
    /// outside the area being built.
    pub fn peek_geotiff(path: &Path, crs: Option<Crs>) -> Result<GridTile, DemError> {
        let file = std::io::BufReader::new(std::fs::File::open(path)?);
        Ok(GeoHeader::read(file, crs)?.tile)
    }

    /// Writes the tile as a Deflate-compressed float GeoTIFF (pixel is
    /// area, `GDAL_NODATA` −9999), readable by GDAL / QGIS.
    pub fn write_geotiff(&self, path: &Path) -> Result<(), DemError> {
        use tiff::encoder::{Compression, DeflateLevel, TiffEncoder, colortype::Gray32Float};
        use tiff::tags::Tag;
        let err = |e: tiff::TiffError| format(format!("{}: {e}", path.display()));
        let tmp = path.with_extension("tif.part");
        let file = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
        let mut enc = TiffEncoder::new(file)
            .map_err(err)?
            .with_compression(Compression::Deflate(DeflateLevel::Balanced));
        let mut img = enc
            .new_image::<Gray32Float>(self.nx as u32, self.ny as u32)
            .map_err(err)?;
        let epsg = u16::try_from(self.crs.epsg()).map_err(|_| format("EPSG code > 65535"))?;
        let e = img.encoder();
        e.write_tag(Tag::ModelPixelScaleTag, &[self.cell, self.cell, 0.0][..])
            .map_err(err)?;
        e.write_tag(
            Tag::ModelTiepointTag,
            &[0.0, 0.0, 0.0, self.x0, self.y0, 0.0][..],
        )
        .map_err(err)?;
        // Version 1.1.0, 3 keys: model type projected, raster type pixel is
        // area, projected CRS = EPSG code.
        e.write_tag(
            Tag::GeoKeyDirectoryTag,
            &[
                1u16, 1, 0, 3, 1024, 0, 1, 1, 1025, 0, 1, 1, 3072, 0, 1, epsg,
            ][..],
        )
        .map_err(err)?;
        e.write_tag(Tag::GdalNodata, "-9999").map_err(err)?;
        let data: Vec<f32> = self
            .data
            .iter()
            .map(|v| if v.is_nan() { NODATA } else { *v })
            .collect();
        img.write_data(&data).map_err(err)?;
        drop(enc);
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

/// A GeoTIFF's georeference, read before the pixel data.
struct GeoHeader<R: std::io::Read + std::io::Seek> {
    decoder: tiff::decoder::Decoder<R>,
    tile: GridTile,
    nodata: Option<f64>,
}

impl<R: std::io::Read + std::io::Seek> GeoHeader<R> {
    fn read(reader: R, crs: Option<Crs>) -> Result<Self, DemError> {
        use tiff::tags::Tag;
        let err = |e: tiff::TiffError| format(format!("GeoTIFF: {e}"));
        let mut d = tiff::decoder::Decoder::new(reader)
            .map_err(err)?
            .with_limits(tiff::decoder::Limits::unlimited());
        let (w, h) = d.dimensions().map_err(err)?;
        let scale = d
            .get_tag_f64_vec(Tag::ModelPixelScaleTag)
            .map_err(|_| format("GeoTIFF without ModelPixelScale"))?;
        let tie = d
            .get_tag_f64_vec(Tag::ModelTiepointTag)
            .map_err(|_| format("GeoTIFF without ModelTiepoint"))?;
        if scale.len() < 2 || tie.len() < 6 {
            return Err(format("GeoTIFF: short ModelPixelScale / ModelTiepoint"));
        }
        if (scale[0] - scale[1]).abs() > 1e-9 * scale[0] {
            return Err(format(format!(
                "GeoTIFF: cells are not square ({} × {})",
                scale[0], scale[1]
            )));
        }
        let keys = d
            .get_tag_u16_vec(Tag::GeoKeyDirectoryTag)
            .unwrap_or_default();
        let key = |id: u16| {
            keys.get(4..)?
                .as_chunks::<4>()
                .0
                .iter()
                .find(|k| k[0] == id && k[1] == 0)
                .map(|k| k[3])
        };
        let crs = match key(3072).and_then(|c| Crs::from_epsg(u32::from(c))) {
            Some(c) => crs.unwrap_or(c),
            None => crs.ok_or_else(|| {
                format(format!(
                    "GeoTIFF: unsupported or missing projected CRS (EPSG {:?}); give the CRS",
                    key(3072)
                ))
            })?,
        };
        let point = key(1025) == Some(2);
        let cell = scale[0];
        let mut x0 = tie[3] - tie[0] * cell;
        let mut y0 = tie[4] + tie[1] * cell;
        if point {
            x0 -= cell / 2.0;
            y0 += cell / 2.0;
        }
        let nodata = d
            .get_tag_ascii_string(Tag::GdalNodata)
            .ok()
            .and_then(|s| s.trim_matches(char::from(0)).trim().parse::<f64>().ok());
        Ok(Self {
            decoder: d,
            tile: GridTile {
                crs,
                cell,
                x0,
                y0,
                nx: w as usize,
                ny: h as usize,
                data: Vec::new(),
            },
            nodata,
        })
    }

    /// Decodes a tiled single-band `f32` image tile by tile (uncompressed
    /// or LZW, no predictor): empty tiles and short tiles are voids.
    fn read_tiles_lenient(&mut self, bytes: &[u8]) -> Result<GridTile, DemError> {
        use tiff::tags::Tag;
        let d = &mut self.decoder;
        let tag = |d: &mut tiff::decoder::Decoder<R>, t: Tag| d.get_tag_u64_vec(t).ok();
        let one = |v: Option<Vec<u64>>| v.and_then(|v| v.first().copied());
        let compression = one(tag(d, Tag::Compression)).unwrap_or(1);
        let predictor = one(tag(d, Tag::Predictor)).unwrap_or(1);
        let bits = one(tag(d, Tag::BitsPerSample)).unwrap_or(0);
        let format_ = one(tag(d, Tag::SampleFormat)).unwrap_or(1);
        let (Some(tw), Some(th)) = (one(tag(d, Tag::TileWidth)), one(tag(d, Tag::TileLength)))
        else {
            return Err(format("not a tiled image"));
        };
        let offsets = tag(d, Tag::TileOffsets).ok_or_else(|| format("no TileOffsets"))?;
        let counts = tag(d, Tag::TileByteCounts).ok_or_else(|| format("no TileByteCounts"))?;
        if bits != 32 || format_ != 3 || predictor != 1 || !matches!(compression, 1 | 5) {
            return Err(format(format!(
                "only f32 tiles, uncompressed or LZW without predictor (have {bits} bit, format {format_}, compression {compression}, predictor {predictor})"
            )));
        }
        let little = bytes.starts_with(b"II");
        let (tw, th) = (tw as usize, th as usize);
        let mut t = self.tile.clone();
        let across = t.nx.div_ceil(tw);
        t.data = vec![f32::NAN; t.nx * t.ny];
        for (k, (&off, &n)) in offsets.iter().zip(&counts).enumerate() {
            let (ox, oy) = ((k % across) * tw, (k / across) * th);
            if n == 0 || oy >= t.ny {
                continue; // sparse tile: no data
            }
            let raw = bytes
                .get(off as usize..(off + n) as usize)
                .ok_or_else(|| format("tile beyond the end of the file"))?;
            let plain = if compression == 5 {
                lzw_lenient(raw)
            } else {
                raw.to_vec()
            };
            for (i, b) in plain.as_chunks::<4>().0.iter().take(tw * th).enumerate() {
                let (x, y) = (ox + i % tw, oy + i / tw);
                if x < t.nx && y < t.ny {
                    t.data[y * t.nx + x] = if little {
                        f32::from_le_bytes(*b)
                    } else {
                        f32::from_be_bytes(*b)
                    };
                }
            }
        }
        let nodata = self.nodata;
        for v in &mut t.data {
            if nodata.is_some_and(|n| (f64::from(*v) - n).abs() < 1e-6) {
                *v = f32::NAN;
            }
        }
        Ok(t)
    }

    fn read_data(&mut self) -> Result<GridTile, DemError> {
        use tiff::decoder::DecodingResult as D;
        let img = self
            .decoder
            .read_image()
            .map_err(|e| format(format!("GeoTIFF: {e}")))?;
        let raw: Vec<f64> = match img {
            D::F32(v) => v.into_iter().map(f64::from).collect(),
            D::F64(v) => v,
            D::I16(v) => v.into_iter().map(f64::from).collect(),
            D::U16(v) => v.into_iter().map(f64::from).collect(),
            D::I32(v) => v.into_iter().map(f64::from).collect(),
            D::U32(v) => v.into_iter().map(f64::from).collect(),
            _ => return Err(format("GeoTIFF: unsupported sample type")),
        };
        let mut t = self.tile.clone();
        if raw.len() != t.nx * t.ny {
            return Err(format("GeoTIFF: more than one band"));
        }
        let nodata = self.nodata;
        t.data = raw
            .into_iter()
            .map(|v| {
                if v.is_nan() || nodata.is_some_and(|n| (v - n).abs() < 1e-6) {
                    f32::NAN
                } else {
                    v as f32
                }
            })
            .collect();
        Ok(t)
    }
}

/// TIFF LZW (MSB first, early change) that keeps what it decoded when the
/// stream ends without an end code or breaks off.
fn lzw_lenient(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut d = weezl::decode::Decoder::with_tiff_size_switch(weezl::BitOrder::Msb, 8);
    let _ = d.into_vec(&mut out).decode_all(data);
    out
}

/// Tiles of one grid on one lattice (see the module docs), with their
/// coverage for the feathering.
#[derive(Debug, Clone, Default)]
pub struct GridLayer {
    pub crs: Option<Crs>,
    pub cell: f64,
    pub nx: usize,
    pub ny: usize,
    tiles: HashMap<(i64, i64), GridTile>,
    /// Coarse cells per coverage cell side, and the feather width in them.
    block: i64,
    feather: u8,
    /// Distance (coarse cells) to the edge of the coverage, for coarse
    /// cells closer than `feather`; others in a tile count as `feather`.
    edge: HashMap<(i64, i64), u8>,
    finished: bool,
}

impl GridLayer {
    pub fn new(like: &GridTile) -> Self {
        Self {
            crs: Some(like.crs),
            cell: like.cell,
            nx: like.nx,
            ny: like.ny,
            ..Self::default()
        }
    }

    pub fn crs(&self) -> Crs {
        self.crs.expect("layer made from a tile")
    }

    pub fn len(&self) -> usize {
        self.tiles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    /// Same CRS, cell and tile size.
    pub fn accepts(&self, t: &GridTile) -> bool {
        self.crs == Some(t.crs)
            && (self.cell - t.cell).abs() < 1e-9
            && self.nx == t.nx
            && self.ny == t.ny
    }

    /// Global index of a tile's first sample.
    fn origin_index(&self, t: &GridTile) -> Option<(i64, i64)> {
        let gi = t.x0 / self.cell;
        let gj = -t.y0 / self.cell;
        ((gi - gi.round()).abs() < 1e-6 && (gj - gj.round()).abs() < 1e-6)
            .then(|| (gi.round() as i64, gj.round() as i64))
    }

    /// Adds a tile (a later tile at the same place replaces the earlier).
    pub fn insert(&mut self, t: GridTile) -> Result<(), DemError> {
        if !self.accepts(&t) {
            return Err(format("tile does not belong to this layer"));
        }
        let (gi, gj) = self.origin_index(&t).ok_or_else(|| {
            format(format!(
                "tile at {},{} is not on the {} m lattice",
                t.x0, t.y0, self.cell
            ))
        })?;
        let (nx, ny) = (self.nx as i64, self.ny as i64);
        if gi.rem_euclid(nx) != 0 || gj.rem_euclid(ny) != 0 {
            return Err(format(format!(
                "tile at {},{} is not aligned to the {} × {} tile grid",
                t.x0, t.y0, self.nx, self.ny
            )));
        }
        self.tiles.insert((gi / nx, gj.div_euclid(ny)), t);
        self.finished = false;
        Ok(())
    }

    /// The sample with global index (gi, gj).
    pub fn value(&self, gi: i64, gj: i64) -> Option<f32> {
        let (nx, ny) = (self.nx as i64, self.ny as i64);
        let t = self.tiles.get(&(gi.div_euclid(nx), gj.div_euclid(ny)))?;
        t.get(gi.rem_euclid(nx) as usize, gj.rem_euclid(ny) as usize)
    }

    /// Bilinear height at (x, y); voids are left out (their weight goes to
    /// the valid corners). `None` without any valid corner.
    pub fn sample(&self, x: f64, y: f64) -> Option<f64> {
        let fx = x / self.cell - 0.5;
        let fy = -y / self.cell - 0.5;
        let (i, j) = (fx.floor() as i64, fy.floor() as i64);
        let (tx, ty) = (fx - i as f64, fy - j as f64);
        let mut sum = 0.0;
        let mut weight = 0.0;
        let mut any = None;
        for (di, dj, w) in [
            (0, 0, (1.0 - tx) * (1.0 - ty)),
            (1, 0, tx * (1.0 - ty)),
            (0, 1, (1.0 - tx) * ty),
            (1, 1, tx * ty),
        ] {
            if let Some(v) = self.value(i + di, j + dj) {
                let v = f64::from(v);
                sum += v * w;
                weight += w;
                any.get_or_insert(v);
            }
        }
        if weight > 1e-12 {
            Some(sum / weight)
        } else {
            any
        }
    }

    fn coarse_of(&self, gi: i64, gj: i64) -> (i64, i64) {
        (gi.div_euclid(self.block), gj.div_euclid(self.block))
    }

    fn has_tile_for_coarse(&self, c: (i64, i64)) -> bool {
        let (gi, gj) = (c.0 * self.block, c.1 * self.block);
        let (nx, ny) = (self.nx as i64, self.ny as i64);
        self.tiles
            .contains_key(&(gi.div_euclid(nx), gj.div_euclid(ny)))
    }

    /// Computes the coverage distances: coarse cells (about 50 m) with a
    /// void, or next to a missing tile, are the edge.
    pub fn finish(&mut self, feather_m: f64) {
        self.block = ((COVERAGE_CELL_M / self.cell).round() as i64).max(1);
        self.feather = ((feather_m / (self.block as f64 * self.cell)).ceil() as u8).max(1);
        self.edge.clear();
        let b = self.block;
        let mut queue = VecDeque::new();
        // Coarse cells with a void.
        for (&(ti, tj), t) in &self.tiles {
            let (gi0, gj0) = (ti * self.nx as i64, tj * self.ny as i64);
            let (ci0, cj0) = self.coarse_of(gi0, gj0);
            let (ci1, cj1) = self.coarse_of(gi0 + self.nx as i64 - 1, gj0 + self.ny as i64 - 1);
            for cj in cj0..=cj1 {
                for ci in ci0..=ci1 {
                    let void = (cj * b..(cj + 1) * b).any(|gj| {
                        (ci * b..(ci + 1) * b).any(|gi| {
                            let (li, lj) = (gi - gi0, gj - gj0);
                            if li < 0 || lj < 0 || li >= t.nx as i64 || lj >= t.ny as i64 {
                                // Belongs to the neighbouring tile.
                                return self.value(gi, gj).is_none();
                            }
                            t.get(li as usize, lj as usize).is_none()
                        })
                    });
                    if void && self.edge.insert((ci, cj), 0).is_none() {
                        queue.push_back((ci, cj));
                    }
                }
            }
        }
        // Coarse cells next to a missing tile: distance 1.
        let keys: Vec<(i64, i64)> = self.tiles.keys().copied().collect();
        for (ti, tj) in keys {
            let (gi0, gj0) = (ti * self.nx as i64, tj * self.ny as i64);
            let (ci0, cj0) = self.coarse_of(gi0, gj0);
            let (ci1, cj1) = self.coarse_of(gi0 + self.nx as i64 - 1, gj0 + self.ny as i64 - 1);
            for cj in cj0..=cj1 {
                for ci in ci0..=ci1 {
                    if self.edge.contains_key(&(ci, cj)) {
                        continue;
                    }
                    let outside = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                        .iter()
                        .any(|(di, dj)| !self.has_tile_for_coarse((ci + di, cj + dj)));
                    if outside {
                        self.edge.insert((ci, cj), 1);
                        queue.push_back((ci, cj));
                    }
                }
            }
        }
        // Breadth-first inwards up to the feather width.
        while let Some(c) = queue.pop_front() {
            let d = self.edge[&c];
            if d + 1 >= self.feather {
                continue;
            }
            for (di, dj) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let n = (c.0 + di, c.1 + dj);
                if self.has_tile_for_coarse(n) && !self.edge.contains_key(&n) {
                    self.edge.insert(n, d + 1);
                    queue.push_back(n);
                }
            }
        }
        self.finished = true;
    }

    fn coarse_distance(&self, c: (i64, i64)) -> f64 {
        if !self.has_tile_for_coarse(c) {
            return 0.0;
        }
        f64::from(*self.edge.get(&c).unwrap_or(&self.feather))
    }

    /// How much this layer counts at (x, y): 1 inside its coverage, down
    /// to 0 at its edge over the feather width (smoothstep); without
    /// [`GridLayer::finish`] 1 wherever there is a tile.
    pub fn weight(&self, x: f64, y: f64) -> f64 {
        let gi = (x / self.cell).floor() as i64;
        let gj = (-y / self.cell).floor() as i64;
        if !self.finished {
            let (nx, ny) = (self.nx as i64, self.ny as i64);
            return f64::from(u8::from(
                self.tiles
                    .contains_key(&(gi.div_euclid(nx), gj.div_euclid(ny))),
            ));
        }
        // Bilinear over the coarse cell centres.
        let size = self.block as f64 * self.cell;
        let fx = x / size - 0.5;
        let fy = -y / size - 0.5;
        let (i, j) = (fx.floor() as i64, fy.floor() as i64);
        let (tx, ty) = (fx - i as f64, fy - j as f64);
        let d = self.coarse_distance((i, j)) * (1.0 - tx) * (1.0 - ty)
            + self.coarse_distance((i + 1, j)) * tx * (1.0 - ty)
            + self.coarse_distance((i, j + 1)) * (1.0 - tx) * ty
            + self.coarse_distance((i + 1, j + 1)) * tx * ty;
        let t = (d / f64::from(self.feather)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UTM: Crs = Crs::UtmNorth(32);

    /// A tile of the plane z = 0.01·x + 0.02·y − c (slope in both axes).
    fn plane(x0: f64, y0: f64, cell: f64, n: usize, c: f64) -> GridTile {
        let mut data = Vec::new();
        for j in 0..n {
            for i in 0..n {
                let x = x0 + (i as f64 + 0.5) * cell;
                let y = y0 - (j as f64 + 0.5) * cell;
                data.push((0.01 * x + 0.02 * y - c) as f32);
            }
        }
        GridTile {
            crs: UTM,
            cell,
            x0,
            y0,
            nx: n,
            ny: n,
            data,
        }
    }

    #[test]
    fn xyz_grids_are_read_like_lgl_dgm1() {
        // Rows from the north, x fastest, cell centres at .5, CRLF.
        let mut text = String::new();
        for y in [1001.5, 1000.5] {
            for x in [2000.5, 2001.5, 2002.5] {
                text += &format!("{x:.2} {y:.2} {:.2}\r\n", x - 2000.0 + 10.0 * (y - 1000.0));
            }
        }
        let t = GridTile::from_xyz(&text, UTM).unwrap();
        assert_eq!(
            (t.nx, t.ny, t.cell, t.x0, t.y0),
            (3, 2, 1.0, 2000.0, 1002.0)
        );
        assert_eq!(t.get(0, 0), Some(15.5));
        assert_eq!(t.get(2, 1), Some(7.5));
        assert!(GridTile::from_xyz("1 2\n", UTM).is_err());
        // A missing point is a void.
        let gap: String = text.lines().skip(1).map(|l| format!("{l}\n")).collect();
        let t = GridTile::from_xyz(&gap, UTM).unwrap();
        assert_eq!(t.get(0, 0), None);
        assert!((t.valid_fraction() - 5.0 / 6.0).abs() < 1e-9);
    }

    #[test]
    fn block_means_keep_a_plane_and_skip_voids() {
        let t = plane(425_000.0, 5_303_000.0, 1.0, 8, 110_000.0);
        let r = t.resampled(2).unwrap();
        assert_eq!((r.nx, r.ny, r.cell), (4, 4, 2.0));
        // The mean of a plane is the plane at the block centre.
        let want = 0.01 * (425_000.0 + 1.0) + 0.02 * (5_303_000.0 - 1.0) - 110_000.0;
        assert!((f64::from(r.get(0, 0).unwrap()) - want).abs() < 1e-2);
        let mut v = t.clone();
        v.data[0] = f32::NAN;
        v.data[1] = f32::NAN;
        v.data[8] = f32::NAN;
        v.data[9] = f32::NAN;
        assert_eq!(v.resampled(2).unwrap().get(0, 0), None);
        assert!(t.resampled(3).is_err());
    }

    #[test]
    fn geotiffs_round_trip_with_their_georeference() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.tif");
        let mut t = plane(2_764_000.0, 1_218_000.0, 2.0, 16, 52_000.0);
        t.crs = Crs::Lv95;
        t.data[5] = f32::NAN;
        t.write_geotiff(&path).unwrap();
        let back = GridTile::read_geotiff(&path, None).unwrap();
        assert_eq!(
            (back.crs, back.cell, back.x0, back.y0, back.nx, back.ny),
            (Crs::Lv95, 2.0, 2_764_000.0, 1_218_000.0, 16, 16)
        );
        assert_eq!(back.get(5, 0), None);
        assert_eq!(back.data[6], t.data[6]);
        let peek = GridTile::peek_geotiff(&path, None).unwrap();
        assert!(peek.data.is_empty() && peek.nx == 16);
        assert_eq!(
            peek.bounds(),
            [2_764_000.0, 1_217_968.0, 2_764_032.0, 1_218_000.0]
        );
        // Not a TIFF.
        std::fs::write(&path, b"nope").unwrap();
        assert!(GridTile::read_geotiff(&path, None).is_err());
    }

    #[test]
    fn lzw_without_an_end_code_keeps_its_data() {
        // Regression: some swissALTI3D 2026 COG tiles ended without the LZW
        // end code ("no lzw end code found") and were skipped.
        let plain: Vec<u8> = (0..4000u32).map(|i| (i * 7 % 251) as u8).collect();
        let mut enc = weezl::encode::Encoder::with_tiff_size_switch(weezl::BitOrder::Msb, 8);
        let full = enc.encode(&plain).unwrap();
        assert_eq!(lzw_lenient(&full), plain);
        // Cut off the end code (and the last code or two).
        let cut = lzw_lenient(&full[..full.len() - 3]);
        assert!(
            cut.len() > plain.len() - 40 && plain.starts_with(&cut),
            "{}",
            cut.len()
        );
    }

    #[test]
    fn layers_sample_bilinearly_across_tile_edges() {
        let c = 106_000.0;
        let mut l = GridLayer::new(&plane(0.0, 0.0, 2.0, 10, c));
        for (x0, y0) in [
            (1000.0, 5_300_020.0),
            (1020.0, 5_300_020.0),
            (1000.0, 5_300_000.0),
        ] {
            l.insert(plane(x0, y0, 2.0, 10, c)).unwrap();
        }
        assert_eq!(l.len(), 3);
        let f = |x: f64, y: f64| 0.01 * x + 0.02 * y - c;
        // Inside a tile, across the vertical and the horizontal tile edge.
        for (x, y) in [
            (1005.3, 5_300_013.7),
            (1020.4, 5_300_011.0),
            (1007.0, 5_300_000.2),
        ] {
            let v = l.sample(x, y).unwrap();
            assert!((v - f(x, y)).abs() < 0.05, "{x},{y}: {v} vs {}", f(x, y));
        }
        assert_eq!(l.sample(5000.0, 5_300_010.0), None);
        // Off the lattice or the tile grid: refused.
        assert!(l.insert(plane(1001.0, 5_300_020.0, 2.0, 10, c)).is_err());
        assert!(l.insert(plane(1010.0, 5_300_020.0, 2.0, 10, c)).is_err());
        assert!(l.insert(plane(1040.0, 5_300_020.0, 1.0, 20, c)).is_err());
    }

    #[test]
    fn the_weight_fades_out_towards_the_edge_of_the_coverage() {
        // 10 × 10 tiles of 100 m (cell 10 m), one hole in the middle tile.
        let mut l = GridLayer::new(&plane(0.0, 0.0, 10.0, 10, 0.0));
        for ti in 0..10 {
            for tj in 0..10 {
                let mut t = plane(f64::from(ti) * 100.0, -f64::from(tj) * 100.0, 10.0, 10, 0.0);
                if (ti, tj) == (5, 5) {
                    t.data[55] = f32::NAN;
                }
                l.insert(t).unwrap();
            }
        }
        l.finish(200.0);
        // Deep inside: full weight; at the outer edge: none; next to the
        // void: none; in between rising.
        assert_eq!(l.weight(250.0, -250.0), 1.0);
        assert!(l.weight(1.0, -500.0) < 0.1);
        assert!(l.weight(555.0, -555.0) < 0.15);
        let (a, b) = (l.weight(30.0, -500.0), l.weight(130.0, -500.0));
        assert!(0.0 < a && a < b && b <= 1.0, "{a} {b}");
        assert_eq!(l.weight(-50.0, -500.0), 0.0);
    }
}
