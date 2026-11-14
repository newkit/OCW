//! The orthophoto corridor (spec §14 Phase 8 T7, §4.6 step 4): the open
//! orthophoto of the region (the same sources as the palette, `PALT`) cut
//! along the package's roads, about ±50 m at 0.5 m per pixel, so the
//! renderer can colour the roadside like the real place (fields, gardens,
//! driveways) between its tiling materials and the land-use colour map.
//!
//! One additive, optional package section (spec §4.7); readers that do not
//! know it skip it, and a package without it looks as before:
//!
//! ```text
//! ORTC  layout version: u32 (1)
//!       pixel size (m): f32          corridor half width (m): f32
//!       chunks per tile side: u16    weight cells per tile side: u16
//!       encoding: u8 (1 = baseline JPEG, sRGB, rows along +z)
//!       source count: u8, then the sources' indices into PALT: u8 …
//!       tile count: u32, then per tile:
//!         x, z: i32
//!         weights: u32 length + bytes ([`encode_weights`])
//!         chunk count: u16, then per chunk:
//!           column, row: u8 u8   u32 length + JPEG bytes
//! ```
//!
//! A tile is cut into `chunks × chunks` square chunks (8 × 8 of 125 m at
//! 1 km tiles); only the chunks the corridor touches are stored. Chunk
//! `(c, r)` covers local metres `[o.x + c·m, o.x + (c+1)·m] × [o.z + r·m,
//! o.z + (r+1)·m]` with `m = tile size / chunks`, its first image row at
//! the low `z` edge (like the land use: row-major along +z). Pixels outside
//! the corridor are filled with the chunk's mean colour (they compress to
//! almost nothing).
//!
//! The **corridor weight** (0 … 1) says how much of the photo belongs at a
//! point: 1 near the road, fading out towards the corridor's edge
//! ([`corridor_weight`]), none in forests (the photo shows the canopy, not
//! the floor: [`FOREST_SHARE`]) and none on the footprints of buildings
//! (their roofs, leaning in the photo, would be painted on the ground
//! beside the 3D houses: [`BUILDING_CLEAR_M`]). It is stored per tile at
//! the corners of `weight cells` (4 m), sampled bilinearly by the renderer.
//! [`corridor_weights`] computes it from the package; the world builder
//! and the tests use it, the renderer only reads the stored values.

use std::collections::BTreeMap;

use crate::landuse::LandClass;
use crate::package::{Bytes, TileKey, WorldPackage};

/// Section tag of the orthophoto corridor.
pub const ORTHO_TAG: &[u8; 4] = b"ORTC";
const ORTHO_VERSION: u32 = 1;

/// Corridor half width beside the road edge (spec §14 Phase 8B.3, Q4).
pub const HALF_WIDTH_M: f32 = 50.0;
/// Finest pixel size (m) of the corridor (the source's own size if coarser).
pub const PIXEL_M: f32 = 0.5;
/// Chunks per tile side.
pub const CHUNKS: u16 = 8;
/// Size (m) of the weight cells (4 m: 2 m cost 40–70 MB in the large
/// packages for little visible difference).
pub const WEIGHT_CELL_M: f32 = 4.0;
/// The weight is full up to this share of the half width, then fades out.
pub const FULL_SHARE: f32 = 0.6;
/// Weight of the photo in forests: none – the photo shows the canopy from
/// above, not the floor under our 3D trees, and leaving the forests out
/// halves the corridor of the wooded regions (Black Forest south: 552 →
/// 275 MB, spec §14 Phase 8 T7).
pub const FOREST_SHARE: f32 = 0.0;
/// No photo within the first of these distances (m) around a building's
/// footprint, full photo beyond the second.
pub const BUILDING_CLEAR_M: (f32, f32) = (1.5, 4.0);

/// How the chunks are encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OrthoEncoding {
    /// Baseline JPEG, 8-bit sRGB.
    #[default]
    Jpeg = 1,
}

/// The orthophoto corridor of a package (section `ORTC`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct OrthoCorridor {
    /// Ground size of a pixel (m).
    pub pixel_m: f32,
    /// Half width of the corridor beside the road edges (m).
    pub half_width_m: f32,
    /// Chunks per tile side.
    pub chunks: u16,
    /// Weight cells per tile side (corners: one more).
    pub weight_cells: u16,
    pub encoding: OrthoEncoding,
    /// The imagery sources (indices into the palette's sources, `PALT`).
    pub sources: Vec<u8>,
    pub tiles: BTreeMap<TileKey, OrthoTile>,
}

/// One tile of the corridor.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct OrthoTile {
    /// The corridor weights at the cell corners, encoded
    /// ([`encode_weights`]; kept compressed, decoded when the tile is drawn).
    pub weights: Vec<u8>,
    /// Encoded images by chunk (column, row).
    pub chunks: BTreeMap<(u8, u8), Vec<u8>>,
}

impl OrthoCorridor {
    /// Pixels per chunk side.
    pub fn chunk_px(&self, tile_size: f32) -> u32 {
        (tile_size / f32::from(self.chunks.max(1)) / self.pixel_m).round() as u32
    }

    /// Pixels per tile side.
    pub fn tile_px(&self, tile_size: f32) -> u32 {
        self.chunk_px(tile_size) * u32::from(self.chunks)
    }

    /// The decoded weights of tile `key` (`(cells + 1)²` corners, row-major
    /// along +z, 0 … 255).
    pub fn weights(&self, key: TileKey) -> Option<Result<Vec<u8>, String>> {
        let t = self.tiles.get(&key)?;
        Some(decode_weights(&t.weights, u32::from(self.weight_cells) + 1))
    }

    /// Encoded bytes of all chunks.
    pub fn image_bytes(&self) -> usize {
        self.tiles
            .values()
            .flat_map(|t| t.chunks.values())
            .map(Vec::len)
            .sum()
    }

    /// Number of stored chunks.
    pub fn chunk_count(&self) -> usize {
        self.tiles.values().map(|t| t.chunks.len()).sum()
    }
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The corridor weight at `edge_m` metres from the nearest road edge
/// (0 on the road): 1 up to [`FULL_SHARE`] of the half width, fading
/// smoothly to 0 at the half width.
pub fn corridor_weight(edge_m: f32, half_width_m: f32) -> f32 {
    if half_width_m <= 0.0 {
        return 0.0;
    }
    1.0 - smoothstep(FULL_SHARE * half_width_m, half_width_m, edge_m.max(0.0))
}

/// The share of the photo a land class keeps.
pub fn class_share(class: LandClass) -> f32 {
    match class {
        LandClass::Forest => FOREST_SHARE,
        _ => 1.0,
    }
}

/// The share of the photo at `d` metres from a building's footprint
/// (0 inside).
pub fn building_share(d: f32) -> f32 {
    smoothstep(BUILDING_CLEAR_M.0, BUILDING_CLEAR_M.1, d)
}

fn seg_dist(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dz * dz;
    let t = if l2 > 0.0 {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dz) / l2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p[0] - a[0] - t * dx).hypot(p[1] - a[1] - t * dz)
}

/// Distance from `p` to the outline of the closed `ring`.
pub fn ring_distance(p: [f32; 2], ring: &[[f32; 2]]) -> f32 {
    (0..ring.len())
        .map(|j| seg_dist(p, ring[j], ring[(j + 1) % ring.len()]))
        .fold(f32::MAX, f32::min)
}

/// Whether `p` lies inside the closed `ring` (even-odd).
pub fn point_in_ring(p: [f32; 2], ring: &[[f32; 2]]) -> bool {
    let mut odd = false;
    let n = ring.len();
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        if (a[1] > p[1]) != (b[1] > p[1]) {
            let x = a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
            if p[0] < x {
                odd = !odd;
            }
        }
    }
    odd
}

/// Distance (m) from every corner of a `(cells + 1)²` lattice of `step`
/// metres at `o` to the nearest of `segments` (centre line pieces with
/// their half widths), measured from the road edge; `f32::MAX` beyond
/// `reach`.
fn edge_distances(
    o: [f32; 2],
    step: f32,
    corners: usize,
    segments: &[([f32; 2], [f32; 2], f32)],
    reach: f32,
) -> Vec<f32> {
    let mut d = vec![f32::MAX; corners * corners];
    let idx = |v: f32| ((v / step).floor() as i64).clamp(0, corners as i64 - 1) as usize;
    for &(a, b, half) in segments {
        let r = reach + half;
        let (x0, x1) = (a[0].min(b[0]) - r - o[0], a[0].max(b[0]) + r - o[0]);
        let (z0, z1) = (a[1].min(b[1]) - r - o[1], a[1].max(b[1]) + r - o[1]);
        if x1 < 0.0 || z1 < 0.0 || x0 > step * corners as f32 || z0 > step * corners as f32 {
            continue;
        }
        for k in idx(z0)..=idx(z1 + step) {
            for i in idx(x0)..=idx(x1 + step) {
                let p = [o[0] + i as f32 * step, o[1] + k as f32 * step];
                let e = (seg_dist(p, a, b) - half).max(0.0);
                let c = &mut d[k * corners + i];
                if e < *c {
                    *c = e;
                }
            }
        }
    }
    d
}

/// The corridor weights of tile `key` at the corners of `cells × cells`
/// cells (0 … 1, row-major along +z): [`corridor_weight`] of the distance
/// to the nearest road edge of this tile and its neighbours, times the
/// land class's share ([`class_share`], the class of the cell the corner
/// lies in) and the buildings' ([`building_share`]). `keep` chooses the
/// road strips that count (all of them for the whole network; the routes'
/// for a smaller package).
pub fn corridor_weights(
    pkg: &WorldPackage,
    key: TileKey,
    cells: u32,
    half_width_m: f32,
    keep: &dyn Fn(&crate::package::RoadStrip) -> bool,
) -> Vec<f32> {
    let size = pkg.tile_size_m;
    let o = key.origin(f64::from(size));
    let o = [o[0] as f32, o[1] as f32];
    let step = size / cells as f32;
    let corners = cells as usize + 1;
    let mut segments = Vec::new();
    let mut houses: Vec<&[[f32; 2]]> = Vec::new();
    for dz in -1..=1 {
        for dx in -1..=1 {
            let k = TileKey {
                x: key.x + dx,
                z: key.z + dz,
            };
            if let Some(t) = pkg.tiles.get(&k) {
                for s in t.roads.iter().filter(|s| keep(s)) {
                    for w in s.points.windows(2) {
                        segments.push(([w[0][0], w[0][2]], [w[1][0], w[1][2]], s.width_m / 2.0));
                    }
                }
            }
            if let Some(b) = pkg.buildings.as_ref().and_then(|b| b.get(&k)) {
                houses.extend(b.iter().map(|b| b.outline.as_slice()));
            }
        }
    }
    let road = edge_distances(o, step, corners, &segments, half_width_m);
    let mut w: Vec<f32> = road
        .iter()
        .map(|&d| {
            if d == f32::MAX {
                0.0
            } else {
                corridor_weight(d, half_width_m)
            }
        })
        .collect();
    // Land classes: the cell each corner lies in (the last row and column
    // take the cell before them).
    if let Some((lu, lc)) = pkg
        .landuse
        .as_ref()
        .and_then(|l| l.tiles.get(&key).map(|t| (t, l.cells as usize)))
        && lc * lc == lu.classes.len()
        && lc > 0
    {
        for k in 0..corners {
            for i in 0..corners {
                let c = (i * lc / (corners - 1)).min(lc - 1);
                let r = (k * lc / (corners - 1)).min(lc - 1);
                w[k * corners + i] *= class_share(lu.classes[r * lc + c]);
            }
        }
    }
    // Buildings: distance to the outline (0 inside).
    let far = BUILDING_CLEAR_M.1;
    for ring in houses {
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for p in ring {
            lo = [lo[0].min(p[0]), lo[1].min(p[1])];
            hi = [hi[0].max(p[0]), hi[1].max(p[1])];
        }
        let i0 = ((lo[0] - far - o[0]) / step).floor().max(0.0) as usize;
        let i1 = ((hi[0] + far - o[0]) / step).ceil();
        let k0 = ((lo[1] - far - o[1]) / step).floor().max(0.0) as usize;
        let k1 = ((hi[1] + far - o[1]) / step).ceil();
        if i1 < 0.0 || k1 < 0.0 {
            continue;
        }
        let (i1, k1) = (
            (i1 as usize).min(corners - 1),
            (k1 as usize).min(corners - 1),
        );
        for k in k0..=k1 {
            for i in i0..=i1 {
                let p = [o[0] + i as f32 * step, o[1] + k as f32 * step];
                let d = if point_in_ring(p, ring) {
                    0.0
                } else {
                    ring_distance(p, ring)
                };
                w[k * corners + i] *= building_share(d);
            }
        }
    }
    w
}

/// The weights quantised to bytes.
pub fn quantise_weights(w: &[f32]) -> Vec<u8> {
    w.iter()
        .map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u8)
        .collect()
}

/// Encodes `corners × corners` weight bytes: each predicted from its
/// neighbours (LOCO-I, as the tint maps) and deflated. The weights are 255
/// along the roads and 0 between them, so a tile shrinks to a few KB.
pub fn encode_weights(w: &[u8], corners: u32) -> Vec<u8> {
    let n = corners as usize;
    assert_eq!(w.len(), n * n, "weight plane size");
    miniz_oxide::deflate::compress_to_vec(&crate::look::predict(w, n), 9)
}

/// Decodes [`encode_weights`]'s bytes.
pub fn decode_weights(bytes: &[u8], corners: u32) -> Result<Vec<u8>, String> {
    let n = corners as usize;
    let raw = miniz_oxide::inflate::decompress_to_vec_with_limit(bytes, n * n)
        .map_err(|e| format!("corridor weights: {e:?}"))?;
    if raw.len() != n * n {
        return Err("corridor weights: wrong size".into());
    }
    Ok(crate::look::unpredict(&raw, n))
}

/// Writes the `ORTC` payload.
pub fn write_ortho(c: &OrthoCorridor, b: &mut Vec<u8>) {
    b.extend(ORTHO_VERSION.to_le_bytes());
    b.extend(c.pixel_m.to_le_bytes());
    b.extend(c.half_width_m.to_le_bytes());
    b.extend(c.chunks.to_le_bytes());
    b.extend(c.weight_cells.to_le_bytes());
    b.push(c.encoding as u8);
    b.push(c.sources.len() as u8);
    b.extend(&c.sources);
    b.extend((c.tiles.len() as u32).to_le_bytes());
    for (k, t) in &c.tiles {
        b.extend(k.x.to_le_bytes());
        b.extend(k.z.to_le_bytes());
        b.extend((t.weights.len() as u32).to_le_bytes());
        b.extend(&t.weights);
        b.extend((t.chunks.len() as u16).to_le_bytes());
        for ((col, row), img) in &t.chunks {
            b.push(*col);
            b.push(*row);
            b.extend((img.len() as u32).to_le_bytes());
            b.extend(img);
        }
    }
}

/// Reads the `ORTC` payload. A newer layout or encoding is an error with
/// "not supported" in it (the package reader then skips the section).
pub fn read_ortho(bytes: &[u8]) -> Result<OrthoCorridor, String> {
    let mut r = Bytes::new(bytes);
    match r.u32()? {
        ORTHO_VERSION => {}
        v => return Err(format!("orthophoto corridor layout {v} is not supported")),
    }
    let pixel_m = r.f32()?;
    let half_width_m = r.f32()?;
    let chunks = u16::from_le_bytes(r.arr()?);
    let weight_cells = u16::from_le_bytes(r.arr()?);
    let encoding = match r.u8()? {
        1 => OrthoEncoding::Jpeg,
        e => return Err(format!("orthophoto encoding {e} is not supported")),
    };
    if pixel_m.is_nan() || pixel_m <= 0.0 || chunks == 0 || chunks > 255 || weight_cells == 0 {
        return Err("orthophoto corridor: bad header".into());
    }
    let n = r.u8()? as usize;
    let sources = r.take(n)?.to_vec();
    let count = r.u32()?;
    let mut tiles = BTreeMap::new();
    for _ in 0..count {
        let key = TileKey {
            x: r.i32()?,
            z: r.i32()?,
        };
        let wl = r.u32()? as usize;
        let weights = r.take(wl)?.to_vec();
        let cn = u16::from_le_bytes(r.arr()?);
        let mut ch = BTreeMap::new();
        for _ in 0..cn {
            let (col, row) = (r.u8()?, r.u8()?);
            if u16::from(col) >= chunks || u16::from(row) >= chunks {
                return Err("orthophoto chunk outside its tile".into());
            }
            let len = r.u32()? as usize;
            ch.insert((col, row), r.take(len)?.to_vec());
        }
        tiles.insert(
            key,
            OrthoTile {
                weights,
                chunks: ch,
            },
        );
    }
    r.finish("orthophoto corridor")?;
    Ok(OrthoCorridor {
        pixel_m,
        half_width_m,
        chunks,
        weight_cells,
        encoding,
        sources,
        tiles,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::graph::RoadClass;
    use crate::package::{RoadStrip, Tile};
    use crate::road_dress::RoadDress;
    use crate::road_surface::RoadSurface;

    pub fn sample_corridor() -> OrthoCorridor {
        let corners = 5u32;
        let w: Vec<u8> = (0..25).map(|i| (i * 10) as u8).collect();
        let mut tiles = BTreeMap::new();
        let mut chunks = BTreeMap::new();
        chunks.insert((0u8, 1u8), vec![0xff, 0xd8, 1, 2, 3]);
        chunks.insert((3, 3), vec![9; 40]);
        tiles.insert(
            TileKey { x: 0, z: 0 },
            OrthoTile {
                weights: encode_weights(&w, corners),
                chunks,
            },
        );
        OrthoCorridor {
            pixel_m: 0.5,
            half_width_m: 50.0,
            chunks: 4,
            weight_cells: 4,
            encoding: OrthoEncoding::Jpeg,
            sources: vec![0, 1],
            tiles,
        }
    }

    #[test]
    fn the_section_round_trips_and_newer_layouts_are_refused() {
        let c = sample_corridor();
        let mut b = Vec::new();
        write_ortho(&c, &mut b);
        assert_eq!(read_ortho(&b).unwrap(), c);
        let w = c.weights(TileKey { x: 0, z: 0 }).unwrap().unwrap();
        assert_eq!(w, (0..25).map(|i| (i * 10) as u8).collect::<Vec<_>>());
        assert!(c.weights(TileKey { x: 1, z: 0 }).is_none());
        assert_eq!(c.chunk_count(), 2);
        assert_eq!(c.image_bytes(), 45);
        assert_eq!(c.chunk_px(1000.0), 500);
        assert_eq!(c.tile_px(1000.0), 2000);
        let mut newer = b.clone();
        newer[0] = 2;
        assert!(read_ortho(&newer).unwrap_err().contains("not supported"));
        // A chunk outside its tile, trailing bytes.
        let mut bad = b.clone();
        bad.push(0);
        assert!(read_ortho(&bad).is_err());
    }

    #[test]
    fn weights_compress_and_round_trip() {
        let n = 501u32;
        let w: Vec<u8> = (0..n * n)
            .map(|i| if (i % n) < 100 { 255 } else { 0 })
            .collect();
        let e = encode_weights(&w, n);
        assert!(e.len() < 2000, "{}", e.len());
        assert_eq!(decode_weights(&e, n).unwrap(), w);
        assert!(decode_weights(&e, n + 1).is_err());
    }

    #[test]
    fn the_weight_is_full_near_the_road_and_fades_out() {
        assert_eq!(corridor_weight(0.0, 50.0), 1.0);
        assert_eq!(corridor_weight(29.9, 50.0), 1.0);
        let mid = corridor_weight(40.0, 50.0);
        assert!(mid > 0.3 && mid < 0.7, "{mid}");
        assert_eq!(corridor_weight(50.0, 50.0), 0.0);
        assert_eq!(corridor_weight(80.0, 50.0), 0.0);
        assert_eq!(corridor_weight(0.0, 0.0), 0.0);
        let mut last = 1.0;
        for i in 0..60 {
            let w = corridor_weight(i as f32, 50.0);
            assert!(w <= last);
            last = w;
        }
        assert_eq!(class_share(LandClass::Meadow), 1.0);
        assert_eq!(class_share(LandClass::Forest), FOREST_SHARE);
        assert_eq!(building_share(0.0), 0.0);
        assert_eq!(building_share(10.0), 1.0);
    }

    fn strip(points: &[[f32; 2]], width: f32) -> RoadStrip {
        RoadStrip {
            class: RoadClass::Residential,
            width_m: width,
            points: points.iter().map(|p| [p[0], 0.0, p[1]]).collect(),
            surface: RoadSurface::of_class(RoadClass::Residential),
            dress: RoadDress::of_class(RoadClass::Residential, width),
        }
    }

    fn package(roads: Vec<(TileKey, RoadStrip)>) -> WorldPackage {
        let mut tiles = BTreeMap::new();
        for x in -1..=1 {
            for z in -1..=1 {
                let key = TileKey { x, z };
                tiles.insert(
                    key,
                    Tile {
                        key,
                        heights: vec![0.0; 4],
                        roads: Vec::new(),
                    },
                );
            }
        }
        for (k, s) in roads {
            tiles.get_mut(&k).unwrap().roads.push(s);
        }
        WorldPackage {
            name: "t".into(),
            origin: (47.0, 9.5),
            tile_size_m: 1000.0,
            samples: 2,
            tiles,
            graph: None,
            landuse: None,
            buildings: None,
            bridges: None,
            routes: Vec::new(),
            region: None,
            time_zone: None,
            look: Default::default(),
        }
    }

    /// The corridor geometry: a road along x = 500 m (10 m wide) in tile
    /// (0, 0); the weight depends only on the distance from its edge, is
    /// symmetric, and a road in the neighbour tile reaches across the edge.
    #[test]
    fn the_corridor_follows_the_road_edges_across_tiles() {
        let road = strip(&[[500.0, -10.0], [500.0, 1010.0]], 10.0);
        let pkg = package(vec![(TileKey { x: 0, z: 0 }, road)]);
        let cells = 500;
        let w = corridor_weights(&pkg, TileKey { x: 0, z: 0 }, cells, 50.0, &|_| true);
        let n = cells as usize + 1;
        assert_eq!(w.len(), n * n);
        let at = |x: f32, z: f32| w[(z / 2.0) as usize * n + (x / 2.0) as usize];
        // On the road and up to 30 m beside its edge (35 m from the centre).
        assert_eq!(at(500.0, 400.0), 1.0);
        assert_eq!(at(530.0, 400.0), 1.0);
        assert_eq!(at(470.0, 400.0), 1.0);
        // Fading, symmetric.
        let (e, west) = (at(546.0, 400.0), at(454.0, 400.0));
        assert!(e > 0.0 && e < 1.0 && (e - west).abs() < 1e-6, "{e} {west}");
        // Outside the corridor (> 55 m from the centre).
        assert_eq!(at(560.0, 400.0), 0.0);
        assert_eq!(at(100.0, 400.0), 0.0);
        // The neighbour tile (1, 0) sees nothing; tile (-1, 0) neither.
        let east = corridor_weights(&pkg, TileKey { x: 1, z: 0 }, cells, 50.0, &|_| true);
        assert!(east.iter().all(|v| *v == 0.0));
        // A road just west of tile (1, 0)'s edge reaches into it.
        let edge = strip(&[[990.0, 0.0], [990.0, 1000.0]], 4.0);
        let pkg = package(vec![(TileKey { x: 0, z: 0 }, edge)]);
        let east = corridor_weights(&pkg, TileKey { x: 1, z: 0 }, cells, 50.0, &|_| true);
        assert_eq!(east[200 * n], 1.0);
        assert_eq!(east[200 * n + 30], 0.0);
        // The shared edge agrees.
        let west = corridor_weights(&pkg, TileKey { x: 0, z: 0 }, cells, 50.0, &|_| true);
        for k in 0..n {
            assert_eq!(west[k * n + n - 1], east[k * n]);
        }
        // Without strips that count: nothing.
        let none = corridor_weights(&pkg, TileKey { x: 0, z: 0 }, cells, 50.0, &|_| false);
        assert!(none.iter().all(|v| *v == 0.0));
    }

    /// Forests keep a share of the photo; building footprints none.
    #[test]
    fn forests_and_buildings_lower_the_weight() {
        use crate::buildings::{Building, BuildingKind, Roof};
        use crate::landuse::{LandUse, LandUseTile};
        let road = strip(&[[0.0, 500.0], [1000.0, 500.0]], 6.0);
        let mut pkg = package(vec![(TileKey { x: 0, z: 0 }, road)]);
        // Forest in the western half of tile (0, 0).
        let lc = 200usize;
        let classes = (0..lc * lc)
            .map(|i| {
                if i % lc < 100 {
                    LandClass::Forest
                } else {
                    LandClass::Meadow
                }
            })
            .collect();
        let mut lu = LandUse {
            cells: lc as u32,
            ..Default::default()
        };
        lu.tiles.insert(
            TileKey { x: 0, z: 0 },
            LandUseTile {
                classes,
                water: Vec::new(),
            },
        );
        pkg.landuse = Some(lu);
        let mut b = BTreeMap::new();
        b.insert(
            TileKey { x: 0, z: 0 },
            vec![Building {
                kind: BuildingKind::House,
                outline: vec![
                    [700.0, 510.0],
                    [710.0, 510.0],
                    [710.0, 520.0],
                    [700.0, 520.0],
                ],
                holes: Vec::new(),
                base_y: 0.0,
                eave_y: 6.0,
                roof: Roof::Flat,
                tags: Default::default(),
            }],
        );
        pkg.buildings = Some(b);
        let cells = 500;
        let n = cells as usize + 1;
        let w = corridor_weights(&pkg, TileKey { x: 0, z: 0 }, cells, 50.0, &|_| true);
        let at = |x: f32, z: f32| w[(z / 2.0) as usize * n + (x / 2.0) as usize];
        assert_eq!(FOREST_SHARE, 0.0);
        assert!((at(200.0, 510.0) - FOREST_SHARE).abs() < 1e-6);
        assert_eq!(at(800.0, 510.0), 1.0);
        // Inside the house and right beside it: no photo; 6 m away: all.
        assert_eq!(at(706.0, 516.0), 0.0);
        assert_eq!(at(711.0, 516.0), 0.0);
        assert!(at(712.0, 516.0) > 0.0 && at(712.0, 516.0) < 0.5);
        assert_eq!(at(716.0, 516.0), 1.0);
    }
}
