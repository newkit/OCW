//! The measured look of a region (spec §14 Phase 8 T3, §4.6): what the
//! world builder takes from open aerial and satellite imagery, so the
//! renderer tints its tiling materials with the real colours of the place.
//!
//! Three additive package sections (spec §4.7), all optional; readers that
//! do not know them skip them, and a package without them looks as before:
//!
//! - `PALT` – the **imagery sources** used (with their notices) and the
//!   **regional palette**: the median colour of each land class (meadow,
//!   farmland, forest, rock, …), of the paved roads and of the roofs,
//!   measured in the orthophoto (or the Sentinel-2 summer composite where
//!   there is no orthophoto). Layout:
//!
//!   ```text
//!   layout version: u32 (1)
//!   source count: u32, then per source: id, name, licence, notice, url,
//!     date (each u32 length + UTF-8), resolution (m): f32
//!   entry count: u32, then per entry:
//!     key: u8 (land class code 0–16, 200 paved road, 201 roofs, 202 unpaved)
//!     colour: [u8; 3] (sRGB)   samples: u32   source index: u8
//!   ```
//!
//! - `ROOF` – the **roof colour of every building**, sampled in the
//!   orthophoto inside its footprint eroded by 1 m, or taken from OSM
//!   `roof:colour`. Per tile the buildings in the order of the `BLDG`
//!   section:
//!
//!   ```text
//!   layout version: u32 (1)
//!   tile count: u32, then per tile: x, z: i32, building count: u32,
//!     per building [r, g, b, origin]: u8 × 4 (origin 0 = none, 1 = orthophoto, 2 = OSM)
//!   ```
//!
//! - `SEAS` – **seasonal tint maps**: per tile a coarse colour map per
//!   season (spring, summer, autumn, winter) from our own Sentinel-2
//!   median composites, which the renderer blends by the world date
//!   ([`SeasonalTints::weights`]). Each map is stored compactly
//!   ([`encode_tint`]: YCoCg with half-resolution chroma, a LOCO-I
//!   predictor and deflate):
//!
//!   ```text
//!   layout version: u32 (1)
//!   cells per tile side: u32   source index: u8   season count: u8
//!   per season: season u8 (0 spring, 1 summer, 2 autumn, 3 winter),
//!     centre day of year u16, first u16, last u16 (1-based), scenes u16,
//!     class count u8, then per land class: code u8, median colour [u8; 3]
//!   tile count: u32, then per tile: x, z: i32, per season: length u32 + bytes
//!   ```
//!
//! Cell `(c, r)` of a tint map covers local metres
//! `[o.x + c·s, o.x + (c+1)·s] × [o.z + r·s, o.z + (r+1)·s]` with
//! `s = tile size / cells`, row-major along +z like the land use.

use std::collections::BTreeMap;

use crate::landuse::LandClass;
use crate::package::{Bytes, TileKey};

/// Section tag of the imagery sources and the palette.
pub const PALETTE_TAG: &[u8; 4] = b"PALT";
/// Section tag of the roof colours.
pub const ROOFS_TAG: &[u8; 4] = b"ROOF";
/// Section tag of the seasonal tint maps.
pub const SEASONS_TAG: &[u8; 4] = b"SEAS";
const PALETTE_VERSION: u32 = 1;
const ROOFS_VERSION: u32 = 1;
const SEASONS_VERSION: u32 = 1;

/// Everything a package knows about its measured look.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RegionLook {
    /// Sources and palette (section `PALT`).
    pub palette: Option<Palette>,
    /// Roof colours per building (section `ROOF`).
    pub roofs: Option<RoofColours>,
    /// Seasonal tint maps (section `SEAS`).
    pub seasons: Option<SeasonalTints>,
    /// The orthophoto corridor along the roads (section `ORTC`, spec §14
    /// Phase 8 T7; optional).
    pub ortho: Option<crate::ortho::OrthoCorridor>,
}

impl RegionLook {
    pub fn is_empty(&self) -> bool {
        self.palette.is_none()
            && self.roofs.is_none()
            && self.seasons.is_none()
            && self.ortho.is_none()
    }

    /// The imagery sources (empty without a palette section).
    pub fn sources(&self) -> &[ImagerySource] {
        self.palette.as_ref().map_or(&[], |p| p.sources.as_slice())
    }

    /// The roof colour of building `index` of tile `key`.
    pub fn roof(&self, key: TileKey, index: usize) -> Option<RoofColour> {
        self.roofs
            .as_ref()?
            .get(&key)?
            .get(index)
            .copied()
            .flatten()
    }
}

/// An imagery source and the notice it requires (spec §13, Phase 8A).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ImagerySource {
    /// Short id, e.g. `li-orthophoto-2025`.
    pub id: String,
    /// What it is, e.g. "Orthophoto Liechtenstein 2025".
    pub name: String,
    /// Licence, e.g. "CC BY 4.0", "dl-de/by-2-0".
    pub licence: String,
    /// Text for the credits screen and LICENSE-DATA.md.
    pub notice: String,
    /// Where the licence or the data is described.
    pub url: String,
    /// When the imagery was taken ("2025", "2023-2025, 15 Jun - 31 Aug").
    pub date: String,
    /// Ground resolution used by the builder (m per pixel).
    pub resolution_m: f32,
}

/// What a palette colour is the colour of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PaletteKey {
    Land(LandClass),
    /// Asphalt and other paved roads (road strips of car roads).
    PavedRoad,
    /// All roofs together (the default for buildings without a sample).
    Roofs,
    /// Tracks and paths (gravel, dirt).
    UnpavedRoad,
}

impl PaletteKey {
    pub fn code(self) -> u8 {
        match self {
            PaletteKey::Land(c) => c.code(),
            PaletteKey::PavedRoad => 200,
            PaletteKey::Roofs => 201,
            PaletteKey::UnpavedRoad => 202,
        }
    }

    pub fn from_code(code: u8) -> Option<Self> {
        match code {
            200 => Some(PaletteKey::PavedRoad),
            201 => Some(PaletteKey::Roofs),
            202 => Some(PaletteKey::UnpavedRoad),
            c => LandClass::from_code(c).map(PaletteKey::Land),
        }
    }
}

/// One measured colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaletteEntry {
    pub key: PaletteKey,
    /// Median colour (sRGB).
    pub rgb: [u8; 3],
    /// Pixels it was measured from.
    pub samples: u32,
    /// Index into [`Palette::sources`].
    pub source: u8,
}

/// The imagery sources of a package and its regional palette.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Palette {
    pub sources: Vec<ImagerySource>,
    pub entries: Vec<PaletteEntry>,
}

impl Palette {
    /// The measured colour of `key`, if any.
    pub fn colour(&self, key: PaletteKey) -> Option<[u8; 3]> {
        self.entries.iter().find(|e| e.key == key).map(|e| e.rgb)
    }
}

/// Where a roof colour comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoofOrigin {
    /// Sampled in the orthophoto.
    Orthophoto = 1,
    /// OSM `roof:colour` (wins over the orthophoto).
    Osm = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoofColour {
    pub rgb: [u8; 3],
    pub origin: RoofOrigin,
}

/// Roof colours per tile, in the order of the tile's buildings (`BLDG`).
pub type RoofColours = BTreeMap<TileKey, Vec<Option<RoofColour>>>;

/// The four seasons of the tint maps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Season {
    Spring = 0,
    Summer = 1,
    Autumn = 2,
    Winter = 3,
}

impl Season {
    pub const ALL: [Season; 4] = [
        Season::Spring,
        Season::Summer,
        Season::Autumn,
        Season::Winter,
    ];

    pub fn from_code(c: u8) -> Option<Self> {
        Self::ALL.get(usize::from(c)).copied()
    }

    pub fn name(self) -> &'static str {
        match self {
            Season::Spring => "spring",
            Season::Summer => "summer",
            Season::Autumn => "autumn",
            Season::Winter => "winter",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|x| x.name() == s)
    }
}

/// One seasonal composite: its season and the days of the year it was
/// built from (1-based; a winter window wraps, e.g. 335 … 59).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeasonInfo {
    pub season: Season,
    /// The day the composite stands for (blend weight 1 there).
    pub centre_doy: u16,
    pub first_doy: u16,
    pub last_doy: u16,
    /// Scenes that went into the median.
    pub scenes: u16,
}

/// Seasonal tint maps of all tiles (section `SEAS`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SeasonalTints {
    /// Cells per tile side.
    pub cells: u32,
    /// Index into the palette's sources.
    pub source: u8,
    pub seasons: Vec<SeasonInfo>,
    /// Per season (in the order of `seasons`) the median colour of each
    /// land class over the whole package: how a class changes through the
    /// year, for the near materials' seasonal tint.
    pub class_means: Vec<Vec<(LandClass, [u8; 3])>>,
    /// Per tile, per season (in the order of `seasons`) the encoded map
    /// ([`encode_tint`]).
    pub tiles: BTreeMap<TileKey, Vec<Vec<u8>>>,
}

/// Days of a (mean) year, for the cyclic blend.
const YEAR_DAYS: f64 = 365.25;

impl SeasonalTints {
    /// The map of season `index` of tile `key` (`cells × cells` sRGB,
    /// row-major along +z), decoded.
    pub fn map(&self, key: TileKey, index: usize) -> Option<Vec<[u8; 3]>> {
        let bytes = self.tiles.get(&key)?.get(index)?;
        decode_tint(bytes, self.cells).ok()
    }

    /// Blend weights of the seasons at `day_of_year` (1 … 366): linear
    /// between the two composites whose centre days enclose it, cyclic
    /// over the year (late December blends autumn into winter, January is
    /// winter). The weights sum to 1; one composite alone gets weight 1.
    pub fn weights(&self, day_of_year: f64) -> Vec<(usize, f32)> {
        season_weights(&self.seasons, day_of_year)
    }

    /// The median colour of `class` in season `index`, if measured.
    pub fn class_mean(&self, index: usize, class: LandClass) -> Option<[u8; 3]> {
        self.class_means
            .get(index)?
            .iter()
            .find(|c| c.0 == class)
            .map(|c| c.1)
    }

    /// The index of `season`'s composite.
    pub fn index_of(&self, season: Season) -> Option<usize> {
        self.seasons.iter().position(|s| s.season == season)
    }

    /// The tint at local `(x, z)` for `day_of_year`, blended between the
    /// seasons (decodes the maps: for tests and probes, the renderer keeps
    /// decoded maps).
    pub fn tint_at(&self, tile_size: f64, x: f64, z: f64, day_of_year: f64) -> Option<[u8; 3]> {
        let key = TileKey::containing([x, z], tile_size);
        let o = key.origin(tile_size);
        let s = tile_size / f64::from(self.cells);
        let c = (((x - o[0]) / s) as u32).min(self.cells - 1) as usize;
        let r = (((z - o[1]) / s) as u32).min(self.cells - 1) as usize;
        let mut acc = [0.0f32; 3];
        for (i, w) in self.weights(day_of_year) {
            let px = self.map(key, i)?[r * self.cells as usize + c];
            for k in 0..3 {
                acc[k] += w * f32::from(px[k]);
            }
        }
        Some(acc.map(|v| v.round().clamp(0.0, 255.0) as u8))
    }
}

/// Blend weights of composites centred on `seasons[i].centre_doy` at
/// `day_of_year` (see [`SeasonalTints::weights`]).
pub fn season_weights(seasons: &[SeasonInfo], day_of_year: f64) -> Vec<(usize, f32)> {
    match seasons.len() {
        0 => return Vec::new(),
        1 => return vec![(0, 1.0)],
        _ => {}
    }
    let mut order: Vec<usize> = (0..seasons.len()).collect();
    order.sort_by_key(|&i| seasons[i].centre_doy);
    let d = day_of_year.rem_euclid(YEAR_DAYS);
    let centre = |i: usize| f64::from(seasons[i].centre_doy);
    // The last centre at or before d (cyclically) and the next one.
    let k = order
        .iter()
        .rposition(|&i| centre(i) <= d)
        .unwrap_or(order.len() - 1);
    let (a, b) = (order[k], order[(k + 1) % order.len()]);
    let span = (centre(b) - centre(a)).rem_euclid(YEAR_DAYS);
    let into = (d - centre(a)).rem_euclid(YEAR_DAYS);
    let t = if span > 0.0 {
        (into / span).clamp(0.0, 1.0)
    } else {
        0.0
    } as f32;
    if t <= 0.0 {
        vec![(a, 1.0)]
    } else if t >= 1.0 {
        vec![(b, 1.0)]
    } else {
        vec![(a, 1.0 - t), (b, t)]
    }
}

/// Day of the year (1 … 366) of a civil date.
pub fn day_of_year(year: i32, month: u32, day: u32) -> u32 {
    const START: [u32; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let m = month.clamp(1, 12) as usize;
    START[m - 1] + day + u32::from(leap && m > 2)
}

// --- Tint map codec -------------------------------------------------------

/// Encodes a `cells × cells` sRGB map compactly (lossy, a few levels per
/// channel): YCoCg with chroma averaged over 2 × 2 cells, each plane
/// predicted from its left, upper and upper-left neighbours (LOCO-I / MED,
/// as in JPEG-LS) and the residuals deflated. Smooth satellite colours
/// shrink to a few bytes per cell.
pub fn encode_tint(rgb: &[[u8; 3]], cells: u32) -> Vec<u8> {
    let n = cells as usize;
    assert_eq!(rgb.len(), n * n, "tint map size");
    let h = n.div_ceil(2);
    let mut y = vec![0u8; n * n];
    let (mut co_acc, mut cg_acc, mut cnt) =
        (vec![0i32; h * h], vec![0i32; h * h], vec![0i32; h * h]);
    for r in 0..n {
        for c in 0..n {
            let [rr, gg, bb] = rgb[r * n + c].map(i32::from);
            y[r * n + c] = ((rr + 2 * gg + bb + 2) >> 2).clamp(0, 255) as u8;
            let k = (r / 2) * h + c / 2;
            co_acc[k] += rr - bb; // Co·2
            cg_acc[k] += 2 * gg - rr - bb; // Cg·4
            cnt[k] += 1;
        }
    }
    let avg = |acc: &[i32], div: i32| -> Vec<u8> {
        acc.iter()
            .zip(&cnt)
            .map(|(&a, &c)| {
                let d = div * c;
                (128 + (a + a.signum() * d / 2) / d).clamp(0, 255) as u8
            })
            .collect()
    };
    let co = avg(&co_acc, 2);
    let cg = avg(&cg_acc, 4);
    let mut raw = Vec::with_capacity(n * n + 2 * h * h);
    raw.extend(predict(&y, n));
    raw.extend(predict(&co, h));
    raw.extend(predict(&cg, h));
    miniz_oxide::deflate::compress_to_vec(&raw, 9)
}

/// Decodes [`encode_tint`]'s bytes into `cells × cells` sRGB.
pub fn decode_tint(bytes: &[u8], cells: u32) -> Result<Vec<[u8; 3]>, String> {
    let n = cells as usize;
    let h = n.div_ceil(2);
    let raw = miniz_oxide::inflate::decompress_to_vec_with_limit(bytes, n * n + 2 * h * h)
        .map_err(|e| format!("tint map: {e:?}"))?;
    if raw.len() != n * n + 2 * h * h {
        return Err("tint map: wrong size".into());
    }
    let y = unpredict(&raw[..n * n], n);
    let co = unpredict(&raw[n * n..n * n + h * h], h);
    let cg = unpredict(&raw[n * n + h * h..], h);
    let mut out = Vec::with_capacity(n * n);
    for r in 0..n {
        for c in 0..n {
            let k = (r / 2) * h + c / 2;
            let yy = i32::from(y[r * n + c]);
            let (o, g) = (i32::from(co[k]) - 128, i32::from(cg[k]) - 128);
            let t = yy - g;
            out.push([t + o, yy + g, t - o].map(|v| v.clamp(0, 255) as u8));
        }
    }
    Ok(out)
}

/// LOCO-I median edge detector prediction of a square plane.
fn med(p: &[u8], n: usize, r: usize, c: usize) -> u8 {
    let a = if c > 0 {
        i32::from(p[r * n + c - 1])
    } else {
        0
    };
    let b = if r > 0 {
        i32::from(p[(r - 1) * n + c])
    } else {
        a
    };
    let a = if c > 0 { a } else { b };
    let cc = if r > 0 && c > 0 {
        i32::from(p[(r - 1) * n + c - 1])
    } else {
        a
    };
    let v = if cc >= a.max(b) {
        a.min(b)
    } else if cc <= a.min(b) {
        a.max(b)
    } else {
        a + b - cc
    };
    v as u8
}

pub(crate) fn predict(p: &[u8], n: usize) -> Vec<u8> {
    (0..n * n)
        .map(|i| p[i].wrapping_sub(med(p, n, i / n, i % n)))
        .collect()
}

pub(crate) fn unpredict(res: &[u8], n: usize) -> Vec<u8> {
    let mut p = vec![0u8; n * n];
    for i in 0..n * n {
        p[i] = res[i].wrapping_add(med(&p, n, i / n, i % n));
    }
    p
}

// --- Section payloads -----------------------------------------------------

fn put_str(b: &mut Vec<u8>, s: &str) {
    b.extend((s.len() as u32).to_le_bytes());
    b.extend(s.as_bytes());
}

fn get_str(r: &mut Bytes<'_>) -> Result<String, String> {
    let n = r.u32()? as usize;
    String::from_utf8(r.take(n)?.to_vec()).map_err(|_| "string is not UTF-8".into())
}

fn check_version(r: &mut Bytes<'_>, want: u32, what: &str) -> Result<(), String> {
    match r.u32()? {
        v if v == want => Ok(()),
        v => Err(format!(
            "{what} layout {v} not supported (this build reads {want})"
        )),
    }
}

/// Writes the `PALT` payload.
pub fn write_palette(p: &Palette, b: &mut Vec<u8>) {
    b.extend(PALETTE_VERSION.to_le_bytes());
    b.extend((p.sources.len() as u32).to_le_bytes());
    for s in &p.sources {
        for t in [&s.id, &s.name, &s.licence, &s.notice, &s.url, &s.date] {
            put_str(b, t);
        }
        b.extend(s.resolution_m.to_le_bytes());
    }
    b.extend((p.entries.len() as u32).to_le_bytes());
    for e in &p.entries {
        b.push(e.key.code());
        b.extend(e.rgb);
        b.extend(e.samples.to_le_bytes());
        b.push(e.source);
    }
}

/// Reads the `PALT` payload.
pub fn read_palette(bytes: &[u8]) -> Result<Palette, String> {
    let mut r = Bytes::new(bytes);
    check_version(&mut r, PALETTE_VERSION, "palette")?;
    let mut p = Palette::default();
    for _ in 0..r.u32()? {
        let mut s = [(); 6].map(|_| String::new());
        for t in &mut s {
            *t = get_str(&mut r)?;
        }
        let [id, name, licence, notice, url, date] = s;
        p.sources.push(ImagerySource {
            id,
            name,
            licence,
            notice,
            url,
            date,
            resolution_m: r.f32()?,
        });
    }
    for _ in 0..r.u32()? {
        let code = r.u8()?;
        let rgb = r.arr()?;
        let samples = r.u32()?;
        let source = r.u8()?;
        // Keys from newer builders are skipped.
        if let Some(key) = PaletteKey::from_code(code) {
            p.entries.push(PaletteEntry {
                key,
                rgb,
                samples,
                source,
            });
        }
    }
    r.finish("palette")?;
    Ok(p)
}

/// Writes the `ROOF` payload.
pub fn write_roofs(roofs: &RoofColours, b: &mut Vec<u8>) {
    b.extend(ROOFS_VERSION.to_le_bytes());
    b.extend((roofs.len() as u32).to_le_bytes());
    for (k, v) in roofs {
        b.extend(k.x.to_le_bytes());
        b.extend(k.z.to_le_bytes());
        b.extend((v.len() as u32).to_le_bytes());
        for c in v {
            match c {
                Some(c) => {
                    b.extend(c.rgb);
                    b.push(c.origin as u8);
                }
                None => b.extend([0; 4]),
            }
        }
    }
}

/// Reads the `ROOF` payload.
pub fn read_roofs(bytes: &[u8]) -> Result<RoofColours, String> {
    let mut r = Bytes::new(bytes);
    check_version(&mut r, ROOFS_VERSION, "roof colours")?;
    let mut out = RoofColours::new();
    for _ in 0..r.u32()? {
        let key = TileKey {
            x: r.i32()?,
            z: r.i32()?,
        };
        let n = r.u32()? as usize;
        let mut v = Vec::with_capacity(n.min(bytes.len() / 4));
        for _ in 0..n {
            let [cr, cg, cb, o] = r.arr()?;
            let origin = match o {
                1 => Some(RoofOrigin::Orthophoto),
                2 => Some(RoofOrigin::Osm),
                _ => None, // none, or an origin of a newer builder
            };
            v.push(origin.map(|origin| RoofColour {
                rgb: [cr, cg, cb],
                origin,
            }));
        }
        out.insert(key, v);
    }
    r.finish("roof colours")?;
    Ok(out)
}

/// Writes the `SEAS` payload.
pub fn write_seasons(s: &SeasonalTints, b: &mut Vec<u8>) {
    b.extend(SEASONS_VERSION.to_le_bytes());
    b.extend(s.cells.to_le_bytes());
    b.push(s.source);
    b.push(s.seasons.len() as u8);
    for (k, i) in s.seasons.iter().enumerate() {
        b.push(i.season as u8);
        for d in [i.centre_doy, i.first_doy, i.last_doy, i.scenes] {
            b.extend(d.to_le_bytes());
        }
        let means = s.class_means.get(k).map_or(&[][..], Vec::as_slice);
        b.push(means.len() as u8);
        for (c, rgb) in means {
            b.push(c.code());
            b.extend(rgb);
        }
    }
    b.extend((s.tiles.len() as u32).to_le_bytes());
    for (k, maps) in &s.tiles {
        b.extend(k.x.to_le_bytes());
        b.extend(k.z.to_le_bytes());
        for m in maps {
            b.extend((m.len() as u32).to_le_bytes());
            b.extend(m);
        }
    }
}

/// Reads the `SEAS` payload.
pub fn read_seasons(bytes: &[u8]) -> Result<SeasonalTints, String> {
    let mut r = Bytes::new(bytes);
    check_version(&mut r, SEASONS_VERSION, "seasonal tints")?;
    let cells = r.u32()?;
    if cells == 0 {
        return Err("seasonal tints: zero cells".into());
    }
    let source = r.u8()?;
    let count = r.u8()?;
    let u16 = |r: &mut Bytes<'_>| -> Result<u16, String> { Ok(u16::from_le_bytes(r.arr()?)) };
    let mut seasons = Vec::new();
    let mut class_means = Vec::new();
    for _ in 0..count {
        let season = Season::from_code(r.u8()?).ok_or("unknown season")?;
        seasons.push(SeasonInfo {
            season,
            centre_doy: u16(&mut r)?,
            first_doy: u16(&mut r)?,
            last_doy: u16(&mut r)?,
            scenes: u16(&mut r)?,
        });
        let mut means = Vec::new();
        for _ in 0..r.u8()? {
            let code = r.u8()?;
            let rgb = r.arr()?;
            if let Some(c) = LandClass::from_code(code) {
                means.push((c, rgb));
            }
        }
        class_means.push(means);
    }
    let mut tiles = BTreeMap::new();
    for _ in 0..r.u32()? {
        let key = TileKey {
            x: r.i32()?,
            z: r.i32()?,
        };
        let mut maps = Vec::with_capacity(seasons.len());
        for _ in 0..seasons.len() {
            let n = r.u32()? as usize;
            maps.push(r.take(n)?.to_vec());
        }
        tiles.insert(key, maps);
    }
    r.finish("seasonal tints")?;
    Ok(SeasonalTints {
        cells,
        source,
        seasons,
        class_means,
        tiles,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn info(season: Season, centre: u16) -> SeasonInfo {
        SeasonInfo {
            season,
            centre_doy: centre,
            first_doy: centre.saturating_sub(30).max(1),
            last_doy: centre + 30,
            scenes: 12,
        }
    }

    fn four() -> Vec<SeasonInfo> {
        vec![
            info(Season::Spring, 120),
            info(Season::Summer, 200),
            info(Season::Autumn, 290),
            info(Season::Winter, 20),
        ]
    }

    fn weight_of(w: &[(usize, f32)], i: usize) -> f32 {
        w.iter().find(|x| x.0 == i).map_or(0.0, |x| x.1)
    }

    #[test]
    fn season_weights_pick_and_blend_by_day_of_year() {
        let s = four();
        // On a centre day: that composite alone.
        assert_eq!(season_weights(&s, 200.0), vec![(1, 1.0)]);
        assert_eq!(season_weights(&s, 20.0), vec![(3, 1.0)]);
        // Half-way between summer (200) and autumn (290).
        let w = season_weights(&s, 245.0);
        assert!((weight_of(&w, 1) - 0.5).abs() < 1e-6, "{w:?}");
        assert!((weight_of(&w, 2) - 0.5).abs() < 1e-6, "{w:?}");
        // A quarter from spring (120) to summer.
        let w = season_weights(&s, 140.0);
        assert!((weight_of(&w, 0) - 0.75).abs() < 1e-6, "{w:?}");
        // Cyclic: late December is between autumn (290) and winter (20 + 365).
        let w = season_weights(&s, 355.0);
        let t = (355.0 - 290.0) / (20.0 + 365.25 - 290.0);
        assert!((weight_of(&w, 3) - t as f32).abs() < 1e-5, "{w:?}");
        assert!((weight_of(&w, 2) - (1.0 - t) as f32).abs() < 1e-5);
        // Early January: winter's centre is day 20, autumn still blends in.
        let w = season_weights(&s, 5.0);
        assert!(weight_of(&w, 3) > 0.8 && weight_of(&w, 2) > 0.0, "{w:?}");
        // Weights always sum to 1.
        for d in 1..=366 {
            let sum: f32 = season_weights(&s, f64::from(d)).iter().map(|x| x.1).sum();
            assert!((sum - 1.0).abs() < 1e-5, "day {d}: {sum}");
        }
        // One composite: always it; none: nothing.
        assert_eq!(season_weights(&s[1..2], 17.0), vec![(0, 1.0)]);
        assert!(season_weights(&[], 17.0).is_empty());
    }

    #[test]
    fn day_of_year_counts_leap_days() {
        assert_eq!(day_of_year(2026, 1, 1), 1);
        assert_eq!(day_of_year(2026, 3, 1), 60);
        assert_eq!(day_of_year(2024, 3, 1), 61);
        assert_eq!(day_of_year(2024, 12, 31), 366);
        assert_eq!(day_of_year(2026, 7, 15), 196);
    }

    fn gradient(n: usize) -> Vec<[u8; 3]> {
        (0..n * n)
            .map(|i| {
                let (r, c) = (i / n, i % n);
                [(40 + c * 2) as u8, (90 + r) as u8, (30 + (r + c) / 2) as u8]
            })
            .collect()
    }

    #[test]
    fn tint_maps_round_trip_within_a_few_levels() {
        for n in [100usize, 50, 7] {
            let img = gradient(n);
            let enc = encode_tint(&img, n as u32);
            let dec = decode_tint(&enc, n as u32).unwrap();
            let worst = img
                .iter()
                .zip(&dec)
                .flat_map(|(a, b)| (0..3).map(move |k| (i32::from(a[k]) - i32::from(b[k])).abs()))
                .max()
                .unwrap();
            assert!(worst <= 4, "n {n}: worst {worst}");
            if n == 100 {
                // A smooth map compresses far below 3 bytes per cell.
                assert!(enc.len() < 3 * n * n / 10, "{} bytes", enc.len());
            }
        }
        // A flat colour is exact.
        let flat = vec![[120, 140, 60]; 16];
        assert_eq!(decode_tint(&encode_tint(&flat, 4), 4).unwrap(), flat);
        // Garbage is an error, not a panic.
        assert!(decode_tint(&[1, 2, 3], 4).is_err());
    }

    pub fn sample_look() -> RegionLook {
        let palette = Palette {
            sources: vec![ImagerySource {
                id: "test-ortho".into(),
                name: "Test orthophoto".into(),
                licence: "CC BY 4.0".into(),
                notice: "© Test".into(),
                url: "https://example.org".into(),
                date: "2025".into(),
                resolution_m: 0.5,
            }],
            entries: vec![
                PaletteEntry {
                    key: PaletteKey::Land(LandClass::Meadow),
                    rgb: [90, 130, 60],
                    samples: 1000,
                    source: 0,
                },
                PaletteEntry {
                    key: PaletteKey::PavedRoad,
                    rgb: [100, 100, 104],
                    samples: 50,
                    source: 0,
                },
            ],
        };
        let roofs = RoofColours::from([(
            TileKey { x: 0, z: 0 },
            vec![
                Some(RoofColour {
                    rgb: [160, 60, 40],
                    origin: RoofOrigin::Orthophoto,
                }),
                None,
                Some(RoofColour {
                    rgb: [50, 50, 50],
                    origin: RoofOrigin::Osm,
                }),
            ],
        )]);
        let n = 4;
        let maps = Season::ALL
            .iter()
            .map(|&s| encode_tint(&vec![[s as u8 * 40, 100, 50]; n * n], n as u32))
            .collect();
        let seasons = SeasonalTints {
            cells: n as u32,
            source: 0,
            seasons: four(),
            class_means: Season::ALL
                .iter()
                .map(|&s| vec![(LandClass::Meadow, [s as u8 * 40, 100, 50])])
                .collect(),
            tiles: BTreeMap::from([(TileKey { x: 0, z: 0 }, maps)]),
        };
        RegionLook {
            palette: Some(palette),
            roofs: Some(roofs),
            seasons: Some(seasons),
            ortho: None,
        }
    }

    #[test]
    fn sections_round_trip() {
        let look = sample_look();
        let mut b = Vec::new();
        write_palette(look.palette.as_ref().unwrap(), &mut b);
        assert_eq!(&read_palette(&b).unwrap(), look.palette.as_ref().unwrap());
        let mut b = Vec::new();
        write_roofs(look.roofs.as_ref().unwrap(), &mut b);
        assert_eq!(&read_roofs(&b).unwrap(), look.roofs.as_ref().unwrap());
        let mut b = Vec::new();
        write_seasons(look.seasons.as_ref().unwrap(), &mut b);
        let s = read_seasons(&b).unwrap();
        assert_eq!(&s, look.seasons.as_ref().unwrap());
        // Autumn's map decodes to its colour; summer → autumn blends.
        let k = TileKey { x: 0, z: 0 };
        assert_eq!(s.class_mean(2, LandClass::Meadow), Some([80, 100, 50]));
        assert_eq!(s.class_mean(2, LandClass::Forest), None);
        assert_eq!(s.index_of(Season::Winter), Some(3));
        let px = s.map(k, 2).unwrap()[5];
        for (a, b) in px.iter().zip([80, 100, 50]) {
            assert!((i32::from(*a) - b).abs() <= 1, "{px:?}");
        }
        let mid = s.tint_at(1000.0, 10.0, 10.0, 245.0).unwrap();
        assert!((mid[0] as i32 - 60).abs() <= 2, "{mid:?}");
        assert_eq!(look.roof(k, 0).unwrap().origin, RoofOrigin::Orthophoto);
        assert_eq!(look.roof(k, 1), None);
        assert_eq!(
            look.palette.as_ref().unwrap().colour(PaletteKey::PavedRoad),
            Some([100, 100, 104])
        );
        // Truncated or newer layouts are errors.
        assert!(read_palette(&b[..3]).is_err());
        let mut newer = b.clone();
        newer[0] = 9;
        assert!(read_seasons(&newer).unwrap_err().contains("not supported"));
    }

    #[test]
    fn unknown_palette_keys_and_roof_origins_are_skipped() {
        let mut b = Vec::new();
        write_palette(
            &Palette {
                sources: Vec::new(),
                entries: vec![PaletteEntry {
                    key: PaletteKey::Roofs,
                    rgb: [1, 2, 3],
                    samples: 1,
                    source: 0,
                }],
            },
            &mut b,
        );
        let at = b.len() - 9; // the entry's key byte
        b[at] = 150; // a key of a newer builder
        assert!(read_palette(&b).unwrap().entries.is_empty());
        let mut b = Vec::new();
        write_roofs(
            &RoofColours::from([(
                TileKey { x: 1, z: 2 },
                vec![Some(RoofColour {
                    rgb: [1, 2, 3],
                    origin: RoofOrigin::Osm,
                })],
            )]),
            &mut b,
        );
        *b.last_mut().unwrap() = 7;
        assert_eq!(read_roofs(&b).unwrap()[&TileKey { x: 1, z: 2 }], vec![None]);
    }
}
