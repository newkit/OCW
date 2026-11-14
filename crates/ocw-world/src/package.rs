//! World package (spec §4.5, §4.7): a region cut into 1 km tiles, each with
//! road-conformed terrain heights and the road strips on it.
//!
//! Binary, little-endian (`OCWPKG01`):
//!
//! ```text
//! magic "OCWPKG01"
//! name: u32 length + UTF-8          origin lat, lon: f64 f64
//! tile size (m): f32                grid samples per side n: u32
//! tile count: u32, then per tile:
//!   tile x, z: i32 i32              n × n heights: f32 (rows along +z)
//!   road count: u32, then per road:
//!     class: u8   width: f32   point count: u32   points: [f32; 3] (x, y, z)
//! ```
//!
//! Version 2 (`OCWPKG02`) is version 1 followed by tagged sections, which
//! readers skip when they do not know the tag (so later sections can be
//! added without a new version):
//!
//! ```text
//! section count: u32, then per section:
//!   tag: [u8; 4]   payload length: u64   payload
//! ```
//!
//! Sections: `GRPH` – the road graph (spec §4.4, §4.7) for free ride and
//! junction steering (§2.2.1), see [`write_graph`]; `ROUT` – routes on the
//! graph by stable edge ID (§2.7), see [`write_routes`]; `LUSE` – land use
//! per tile and water levels (§4.6), see [`crate::landuse`]; `BLDG` –
//! building footprints with heights and roofs, see [`crate::buildings`];
//! `BTAG` – the OSM facts the houses are styled by (roof shape, levels,
//! materials, colours; spec §14 Phase 8 T4, optional), see
//! [`crate::buildings::write_building_tags`];
//! `REGN` – the region the package was cut to (box and polygon in
//! latitude / longitude), for neighbouring packages, see [`crate::region`];
//! `TZID` – the IANA time zone of the package's area ("Europe/Vaduz", UTF-8,
//! spec §14 Phase 9; packages without it fall back to a zone from their
//! origin, see `ocw_sky::zone::zone_for`); `PALT`, `ROOF`, `SEAS` – the
//! imagery sources with the regional palette, the roof colours and the
//! seasonal tint maps (spec §14 Phase 8 T3), see [`crate::look`]; `LITR` –
//! whether each road strip is lit at night (OSM `lit`, spec §14 Phase 9
//! S2), see [`crate::lamps`]; `ORTC` – the orthophoto corridor along the
//! roads (spec §14 Phase 8 T7, optional), see [`crate::ortho`]. A package
//! without sections is written as version 1, byte for byte as before.
//!
//! Tile `(x, z)` covers local metres `[x·size, (x+1)·size] × [z·size, (z+1)·size]`;
//! the grid includes both borders, so neighbouring tiles share their edge
//! samples (no cracks). The manifest with hashes and signatures (§4.7)
//! travels next to the package: see [`crate::manifest`].

use std::collections::BTreeMap;

use crate::bridges::{BRIDGES_TAG, Bridges, read_bridges, write_bridges};
use crate::buildings::{
    BUILDING_TAGS_TAG, BUILDINGS_TAG, Buildings, apply_building_tags, read_building_tags,
    read_buildings, write_building_tags, write_buildings,
};
use crate::graph::{Edge, EdgeId, Oneway, RoadClass, RoadGraph, VertexId};
use crate::graph_route::{GraphRoute, RouteStep};
use crate::lamps::{ROAD_LIT_TAG, RoadLit, read_road_lit, write_road_lit};
use crate::landuse::{LANDUSE_TAG, LandUse, read_landuse, write_landuse};
use crate::look::{
    PALETTE_TAG, ROOFS_TAG, RegionLook, SEASONS_TAG, read_palette, read_roofs, read_seasons,
    write_palette, write_roofs, write_seasons,
};
use crate::ortho::{ORTHO_TAG, read_ortho, write_ortho};
use crate::region::{PackageRegion, REGION_TAG, read_region, write_region};
use crate::road_dress::{ROAD_DRESS_TAG, RoadDress, read_road_dress, write_road_dress};
use crate::road_surface::{ROAD_SURFACE_TAG, RoadSurface, read_road_surfaces, write_road_surfaces};

pub const MAGIC: &[u8; 8] = b"OCWPKG01";
/// Version 2: version 1 plus tagged sections (the road graph).
pub const MAGIC_V2: &[u8; 8] = b"OCWPKG02";
/// Section tag of the road graph.
pub const GRAPH_TAG: &[u8; 4] = b"GRPH";
/// Section tag of the routes on the road graph (spec §2.7).
pub const ROUTES_TAG: &[u8; 4] = b"ROUT";
/// The client draws package road strips this far above their centre line
/// heights (spec §4.3; the same height as the terrain would z-fight).
pub const ROAD_LIFT_M: f64 = 0.02;
/// Section tag of the time zone id (spec §14 Phase 9).
pub const TIME_ZONE_TAG: &[u8; 4] = b"TZID";
/// Layout version inside the graph section.
const GRAPH_VERSION: u32 = 1;

/// Index of a tile in the package grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileKey {
    pub x: i32,
    pub z: i32,
}

impl TileKey {
    pub fn containing(p: [f64; 2], size: f64) -> Self {
        Self {
            x: (p[0] / size).floor() as i32,
            z: (p[1] / size).floor() as i32,
        }
    }

    pub fn origin(&self, size: f64) -> [f64; 2] {
        [f64::from(self.x) * size, f64::from(self.z) * size]
    }

    /// Chebyshev distance in tiles.
    pub fn ring(&self, other: TileKey) -> i32 {
        (self.x - other.x).abs().max((self.z - other.z).abs())
    }
}

/// A road strip on a tile: centreline with heights, local metres.
#[derive(Debug, Clone, PartialEq)]
pub struct RoadStrip {
    pub class: RoadClass,
    pub width_m: f32,
    pub points: Vec<[f32; 3]>,
    /// The look of the road (section `RSRF`; packages without it get the
    /// category's default, [`RoadSurface::of_class`]).
    pub surface: RoadSurface,
    /// Lanes, markings, kerbs, sidewalks and crossings (section `RDRS`,
    /// spec §14 Phase 8 T5; packages without it get the class default,
    /// [`RoadDress::of_class`]).
    pub dress: RoadDress,
    /// Lit at night (section `LITR`, OSM `lit`, spec §14 Phase 9 S2;
    /// packages without it: [`RoadLit::Unknown`], lit in towns).
    pub lit: RoadLit,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tile {
    pub key: TileKey,
    /// `n × n` terrain heights, row-major along +z, covering the tile with
    /// both borders.
    pub heights: Vec<f32>,
    pub roads: Vec<RoadStrip>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorldPackage {
    pub name: String,
    /// Projection centre of the local metres (lat, lon).
    pub origin: (f64, f64),
    pub tile_size_m: f32,
    /// Grid samples per tile side (including both borders).
    pub samples: u32,
    pub tiles: BTreeMap<TileKey, Tile>,
    /// The road graph (version 2 packages; `None` in version 1).
    pub graph: Option<RoadGraph>,
    /// Land use and water levels per tile (section `LUSE`; `None` in older
    /// packages, the renderer then falls back to plain grass).
    pub landuse: Option<LandUse>,
    /// Buildings per tile (section `BLDG`).
    pub buildings: Option<Buildings>,
    /// Bridge structures per tile (section `BRDG`, spec §4.8.6; `None` in
    /// older packages: their bridges are plain road strips).
    pub bridges: Option<Bridges>,
    /// Routes on the road graph (section `ROUT`, spec §2.7).
    pub routes: Vec<GraphRoute>,
    /// The region the roads were cut to (section `REGN`; `None` in older
    /// packages, which then cannot have neighbours, spec §2.2.2).
    pub region: Option<PackageRegion>,
    /// The IANA time zone of the area (section `TZID`, spec §14 Phase 9),
    /// set by the world builder; `None` in older packages.
    pub time_zone: Option<String>,
    /// The region's measured look from open imagery (sections `PALT`,
    /// `ROOF`, `SEAS`, spec §14 Phase 8 T3); empty in older packages,
    /// which then keep the renderer's default colours.
    pub look: RegionLook,
}

impl WorldPackage {
    pub fn spacing_m(&self) -> f64 {
        f64::from(self.tile_size_m) / f64::from(self.samples - 1)
    }

    /// The lowest and highest terrain height of all tiles (m); `None`
    /// without tiles.
    pub fn height_range(&self) -> Option<(f32, f32)> {
        self.tiles
            .values()
            .flat_map(|t| t.heights.iter().copied())
            .filter(|h| h.is_finite())
            .fold(None, |r, h| match r {
                None => Some((h, h)),
                Some((lo, hi)) => Some((lo.min(h), hi.max(h))),
            })
    }

    /// Terrain height at local (x, z), bilinear within its tile.
    pub fn height(&self, x: f64, z: f64) -> Option<f64> {
        let size = f64::from(self.tile_size_m);
        let key = TileKey::containing([x, z], size);
        let tile = self.tiles.get(&key)?;
        let o = key.origin(size);
        let n = self.samples as usize;
        let s = self.spacing_m();
        let (fx, fz) = (
            ((x - o[0]) / s).clamp(0.0, (n - 1) as f64),
            ((z - o[1]) / s).clamp(0.0, (n - 1) as f64),
        );
        let (i, k) = ((fx as usize).min(n - 2), (fz as usize).min(n - 2));
        let (tx, tz) = (fx - i as f64, fz - k as f64);
        let h = |i: usize, k: usize| f64::from(tile.heights[k * n + i]);
        let top = h(i, k) * (1.0 - tx) + h(i + 1, k) * tx;
        let bottom = h(i, k + 1) * (1.0 - tx) + h(i + 1, k + 1) * tx;
        Some(top * (1.0 - tz) + bottom * tz)
    }

    /// Height of the rendered terrain surface: the tile grid triangulated
    /// like the client mesh (cells split along (i+1, k)–(i, k+1)).
    pub fn surface_height(&self, x: f64, z: f64) -> Option<f64> {
        let size = f64::from(self.tile_size_m);
        let key = TileKey::containing([x, z], size);
        let tile = self.tiles.get(&key)?;
        let o = key.origin(size);
        let n = self.samples as usize;
        let s = self.spacing_m();
        let (fx, fz) = (
            ((x - o[0]) / s).clamp(0.0, (n - 1) as f64),
            ((z - o[1]) / s).clamp(0.0, (n - 1) as f64),
        );
        let (i, k) = ((fx as usize).min(n - 2), (fz as usize).min(n - 2));
        let (tx, tz) = (fx - i as f64, fz - k as f64);
        let h = |i: usize, k: usize| f64::from(tile.heights[k * n + i]);
        let (h00, h10, h01, h11) = (h(i, k), h(i + 1, k), h(i, k + 1), h(i + 1, k + 1));
        Some(if tx + tz <= 1.0 {
            h00 + tx * (h10 - h00) + tz * (h01 - h00)
        } else {
            h11 + (1.0 - tx) * (h01 - h11) + (1.0 - tz) * (h10 - h11)
        })
    }

    /// Heights of the road surface centre lines at local (x, z): one per
    /// road strip segment of the tile whose surface covers the point
    /// (within half its width of the centre line, height interpolated
    /// along it); empty off every road. The client draws the strips
    /// [`ROAD_LIFT_M`] above these heights.
    pub fn road_heights(&self, x: f64, z: f64) -> Vec<f64> {
        self.road_heights_along(x, z, None)
    }

    /// [`Self::road_heights`] of the strips running along `direction`
    /// (horizontal unit vector x, z; within about 25°, either way), if
    /// given: a road crossing over or under it, or a side road at a
    /// junction, is left out.
    pub fn road_heights_along(&self, x: f64, z: f64, direction: Option<[f64; 2]>) -> Vec<f64> {
        let size = f64::from(self.tile_size_m);
        let Some(tile) = self.tiles.get(&TileKey::containing([x, z], size)) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for r in &tile.roads {
            let half = f64::from(r.width_m) / 2.0;
            for w in r.points.windows(2) {
                let (a, b) = (w[0].map(f64::from), w[1].map(f64::from));
                let (dx, dz) = (b[0] - a[0], b[2] - a[2]);
                let len2 = dx * dx + dz * dz;
                if len2 < 1e-12 {
                    continue;
                }
                if let Some(d) = direction
                    && (d[0] * dx + d[1] * dz).abs() < 0.9 * len2.sqrt()
                {
                    continue;
                }
                // Only where the point lies beside the segment: beyond its
                // ends the neighbouring segment holds it (the end height
                // would be a second, stale height of the same road).
                let t = ((x - a[0]) * dx + (z - a[2]) * dz) / len2;
                if !(0.0..=1.0).contains(&t) {
                    continue;
                }
                let (cx, cz) = (a[0] + dx * t, a[2] + dz * t);
                if (x - cx).powi(2) + (z - cz).powi(2) <= half * half {
                    out.push(a[1] + (b[1] - a[1]) * t);
                }
            }
        }
        out
    }

    /// Where the terrain surface rises above a road (it would show through
    /// the road surface): sampled every `step` m along each road strip and
    /// every 0.25 m across it. Returns (worst excess in m, its position,
    /// number of samples above the road, number of samples).
    pub fn road_pokes(&self, step: f64) -> (f64, [f64; 2], usize, usize) {
        let (mut worst, mut at, mut above, mut total) = (0.0f64, [0.0, 0.0], 0, 0);
        for tile in self.tiles.values() {
            for r in &tile.roads {
                let half = f64::from(r.width_m) / 2.0;
                for w in r.points.windows(2) {
                    let (a, b) = (w[0].map(f64::from), w[1].map(f64::from));
                    let len = ((b[0] - a[0]).powi(2) + (b[2] - a[2]).powi(2)).sqrt();
                    if len < 1e-6 {
                        continue;
                    }
                    let (dx, dz) = ((b[0] - a[0]) / len, (b[2] - a[2]) / len);
                    let steps = (len / step).ceil() as usize;
                    for j in 0..steps {
                        let t = j as f64 * step / len;
                        let (cx, cy, cz) = (
                            a[0] + (b[0] - a[0]) * t,
                            a[1] + (b[1] - a[1]) * t,
                            a[2] + (b[2] - a[2]) * t,
                        );
                        let mut off = -half;
                        while off <= half + 1e-9 {
                            let (x, z) = (cx - dz * off, cz + dx * off);
                            if let Some(h) = self.surface_height(x, z) {
                                total += 1;
                                let excess = h - cy;
                                if excess > 0.0 {
                                    above += 1;
                                    if excess > worst {
                                        worst = excess;
                                        at = [x, z];
                                    }
                                }
                            }
                            off += 0.25;
                        }
                    }
                }
            }
        }
        (worst, at, above, total)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = Vec::new();
        let mut sections: Vec<(&[u8; 4], Vec<u8>)> = Vec::new();
        if let Some(graph) = &self.graph {
            let mut payload = Vec::new();
            write_graph(graph, &mut payload);
            sections.push((GRAPH_TAG, payload));
        }
        if !self.routes.is_empty() {
            let mut payload = Vec::new();
            write_routes(&self.routes, &mut payload);
            sections.push((ROUTES_TAG, payload));
        }
        if let Some(l) = &self.landuse {
            let mut payload = Vec::new();
            write_landuse(l, &mut payload);
            sections.push((LANDUSE_TAG, payload));
        }
        if let Some(t) = &self.buildings {
            let mut payload = Vec::new();
            write_buildings(t, &mut payload);
            sections.push((BUILDINGS_TAG, payload));
            let mut payload = Vec::new();
            if write_building_tags(t, &mut payload) {
                sections.push((BUILDING_TAGS_TAG, payload));
            }
        }
        if let Some(r) = &self.region {
            let mut payload = Vec::new();
            write_region(r, &mut payload);
            sections.push((REGION_TAG, payload));
        }
        if let Some(t) = &self.bridges {
            let mut payload = Vec::new();
            write_bridges(t, &mut payload);
            sections.push((BRIDGES_TAG, payload));
        }
        if let Some(z) = &self.time_zone {
            sections.push((TIME_ZONE_TAG, z.as_bytes().to_vec()));
        }
        if let Some(p) = &self.look.palette {
            let mut payload = Vec::new();
            write_palette(p, &mut payload);
            sections.push((PALETTE_TAG, payload));
        }
        if let Some(r) = &self.look.roofs {
            let mut payload = Vec::new();
            write_roofs(r, &mut payload);
            sections.push((ROOFS_TAG, payload));
        }
        if let Some(s) = &self.look.seasons {
            let mut payload = Vec::new();
            write_seasons(s, &mut payload);
            sections.push((SEASONS_TAG, payload));
        }
        if let Some(o) = &self.look.ortho {
            let mut payload = Vec::new();
            write_ortho(o, &mut payload);
            sections.push((ORTHO_TAG, payload));
        }
        let mut payload = Vec::new();
        if write_road_surfaces(
            self.tiles.values().map(|t| (t.key, t.roads.as_slice())),
            &mut payload,
        ) {
            sections.push((ROAD_SURFACE_TAG, payload));
        }
        let mut payload = Vec::new();
        if write_road_dress(
            self.tiles.values().map(|t| (t.key, t.roads.as_slice())),
            &mut payload,
        ) {
            sections.push((ROAD_DRESS_TAG, payload));
        }
        let mut payload = Vec::new();
        if write_road_lit(
            self.tiles.values().map(|t| (t.key, t.roads.as_slice())),
            &mut payload,
        ) {
            sections.push((ROAD_LIT_TAG, payload));
        }
        b.extend_from_slice(if sections.is_empty() { MAGIC } else { MAGIC_V2 });
        b.extend((self.name.len() as u32).to_le_bytes());
        b.extend(self.name.as_bytes());
        b.extend(self.origin.0.to_le_bytes());
        b.extend(self.origin.1.to_le_bytes());
        b.extend(self.tile_size_m.to_le_bytes());
        b.extend(self.samples.to_le_bytes());
        b.extend((self.tiles.len() as u32).to_le_bytes());
        for t in self.tiles.values() {
            b.extend(t.key.x.to_le_bytes());
            b.extend(t.key.z.to_le_bytes());
            for h in &t.heights {
                b.extend(h.to_le_bytes());
            }
            b.extend((t.roads.len() as u32).to_le_bytes());
            for r in &t.roads {
                b.push(class_code(r.class));
                b.extend(r.width_m.to_le_bytes());
                b.extend((r.points.len() as u32).to_le_bytes());
                for p in &r.points {
                    for c in p {
                        b.extend(c.to_le_bytes());
                    }
                }
            }
        }
        if !sections.is_empty() {
            b.extend((sections.len() as u32).to_le_bytes());
            for (tag, payload) in sections {
                b.extend_from_slice(tag);
                b.extend((payload.len() as u64).to_le_bytes());
                b.extend(payload);
            }
        }
        b
    }

    /// The route named `name` (case-insensitive), if the package has it.
    pub fn route(&self, name: &str) -> Option<&GraphRoute> {
        self.routes
            .iter()
            .find(|r| r.name.eq_ignore_ascii_case(name))
    }

    /// Adds `route`, replacing one with the same name.
    pub fn put_route(&mut self, route: GraphRoute) {
        self.routes
            .retain(|r| !r.name.eq_ignore_ascii_case(&route.name));
        self.routes.push(route);
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        Self::decode(bytes, true)
    }

    /// The package without its bulk: the tiles are there by key only
    /// (no heights, no road strips), land use and buildings are left out;
    /// the road graph, routes and region are complete. What a neighbouring
    /// package needs until the rider comes near it (spec §2.2.2, §15): a
    /// fraction of the memory of [`WorldPackage::from_bytes`].
    pub fn skeleton_from_bytes(bytes: &[u8]) -> Result<Self, String> {
        Self::decode(bytes, false)
    }

    /// Whether the tiles hold their terrain (not a skeleton).
    pub fn has_bulk(&self) -> bool {
        self.tiles.values().any(|t| !t.heights.is_empty()) || self.tiles.is_empty()
    }

    fn decode(bytes: &[u8], full: bool) -> Result<Self, String> {
        let mut r = Bytes { b: bytes, pos: 0 };
        let head = read_head(&mut r)?;
        let mut tiles = BTreeMap::new();
        for _ in 0..head.tile_count {
            let tile = read_tile(&mut r, head.samples, full)?;
            tiles.insert(tile.key, tile);
        }
        let sections = if head.version >= 2 {
            read_sections(&mut r, head.samples, full, false)?
        } else {
            Sections::default()
        };
        if r.pos != bytes.len() {
            return Err("trailing bytes after the last tile or section".into());
        }
        for (key, surfaces) in &sections.surfaces {
            let tile = tiles
                .get_mut(key)
                .ok_or("road surfaces of a tile the package does not have")?;
            apply_surfaces(tile, surfaces)?;
        }
        for (key, dress) in &sections.dress {
            let tile = tiles
                .get_mut(key)
                .ok_or("road dress of a tile the package does not have")?;
            apply_dress(tile, dress)?;
        }
        for (key, lit) in &sections.lit {
            let tile = tiles
                .get_mut(key)
                .ok_or("road lit state of a tile the package does not have")?;
            apply_lit(tile, lit)?;
        }
        Ok(sections.into_package(head, tiles))
    }

    /// Approximate memory of the tiles' terrain and road strips (bytes):
    /// what a streaming client counts against its memory budget.
    pub fn tile_bytes(&self) -> u64 {
        self.tiles.values().map(Tile::approx_bytes).sum()
    }
}

impl Tile {
    /// Approximate memory of the tile (heights, road points, strip
    /// overhead) in bytes.
    pub fn approx_bytes(&self) -> u64 {
        let points: usize = self.roads.iter().map(|r| r.points.len()).sum();
        (self.heights.len() * 4
            + points * 12
            + self.roads.len() * std::mem::size_of::<RoadStrip>()
            + std::mem::size_of::<Tile>()) as u64
    }
}

/// The fixed fields of a package before its tiles: what the manifest's
/// `header` chunk holds (spec §4.7).
#[derive(Debug, Clone, PartialEq)]
pub struct PackageHead {
    /// 1 (`OCWPKG01`) or 2 (`OCWPKG02`, with sections).
    pub version: u32,
    pub name: String,
    pub origin: (f64, f64),
    pub tile_size_m: f32,
    pub samples: u32,
    pub tile_count: u32,
}

fn read_head(r: &mut Bytes) -> Result<PackageHead, String> {
    let version = match r.take(8)? {
        m if m == MAGIC => 1,
        m if m == MAGIC_V2 => 2,
        m if m.starts_with(b"OCWPKG") => {
            return Err(format!(
                "world package version {} is not supported (this build reads 01 and 02); please update",
                String::from_utf8_lossy(&m[6..])
            ));
        }
        _ => return Err("not an OCW world package (OCWPKG01/02)".into()),
    };
    let name_len = r.u32()? as usize;
    let name = String::from_utf8(r.take(name_len)?.to_vec()).map_err(|e| e.to_string())?;
    let origin = (r.f64()?, r.f64()?);
    let tile_size_m = r.f32()?;
    let samples = r.u32()?;
    if samples < 2 {
        return Err("tile grid needs at least 2 samples".into());
    }
    let tile_count = r.u32()?;
    Ok(PackageHead {
        version,
        name,
        origin,
        tile_size_m,
        samples,
        tile_count,
    })
}

/// One tile; without `full` only its key (the bytes are skipped).
fn read_tile(r: &mut Bytes, samples: u32, full: bool) -> Result<Tile, String> {
    let key = TileKey {
        x: r.i32()?,
        z: r.i32()?,
    };
    let n = samples as usize * samples as usize;
    if !full {
        r.take(n * 4)?;
        for _ in 0..r.u32()? {
            r.take(5)?;
            let pts = r.u32()? as usize;
            r.take(pts * 12)?;
        }
        return Ok(Tile {
            key,
            heights: Vec::new(),
            roads: Vec::new(),
        });
    }
    let heights = (0..n).map(|_| r.f32()).collect::<Result<_, _>>()?;
    let roads_n = r.u32()?;
    let mut roads = Vec::with_capacity(roads_n as usize);
    for _ in 0..roads_n {
        let class = class_from(r.u8()?)?;
        let width_m = r.f32()?;
        let pts = r.u32()?;
        let points = (0..pts)
            .map(|_| Ok([r.f32()?, r.f32()?, r.f32()?]))
            .collect::<Result<_, String>>()?;
        roads.push(RoadStrip {
            class,
            width_m,
            points,
            surface: RoadSurface::of_class(class),
            dress: RoadDress::of_class(class, width_m),
            lit: RoadLit::Unknown,
        });
    }
    Ok(Tile {
        key,
        heights,
        roads,
    })
}

fn apply_surfaces(tile: &mut Tile, surfaces: &[RoadSurface]) -> Result<(), String> {
    if surfaces.len() != tile.roads.len() {
        return Err("road surfaces do not match the tile's road strips".into());
    }
    for (strip, s) in tile.roads.iter_mut().zip(surfaces) {
        strip.surface = *s;
    }
    Ok(())
}

fn apply_lit(tile: &mut Tile, lit: &[RoadLit]) -> Result<(), String> {
    if lit.len() != tile.roads.len() {
        return Err("road lit states do not match the tile's road strips".into());
    }
    for (strip, l) in tile.roads.iter_mut().zip(lit) {
        strip.lit = *l;
    }
    Ok(())
}

fn apply_dress(tile: &mut Tile, dress: &[RoadDress]) -> Result<(), String> {
    if dress.len() != tile.roads.len() {
        return Err("road dress does not match the tile's road strips".into());
    }
    for (strip, d) in tile.roads.iter_mut().zip(dress) {
        strip.dress = d.clone();
    }
    Ok(())
}

/// The tagged sections of a version 2 package, decoded; the road surfaces
/// and dress stay per tile until their tiles are there.
#[derive(Default)]
struct Sections {
    graph: Option<RoadGraph>,
    landuse: Option<LandUse>,
    buildings: Option<Buildings>,
    bridges: Option<Bridges>,
    routes: Vec<GraphRoute>,
    region: Option<PackageRegion>,
    time_zone: Option<String>,
    look: RegionLook,
    surfaces: Vec<(TileKey, Vec<RoadSurface>)>,
    dress: Vec<(TileKey, Vec<RoadDress>)>,
    lit: Vec<(TileKey, Vec<RoadLit>)>,
}

impl Sections {
    fn into_package(self, head: PackageHead, tiles: BTreeMap<TileKey, Tile>) -> WorldPackage {
        WorldPackage {
            name: head.name,
            origin: head.origin,
            tile_size_m: head.tile_size_m,
            samples: head.samples,
            tiles,
            graph: self.graph,
            landuse: self.landuse,
            buildings: self.buildings,
            bridges: self.bridges,
            routes: self.routes,
            region: self.region,
            time_zone: self.time_zone,
            look: self.look,
        }
    }
}

/// The section count and the sections (version 2); without `full` the
/// bulk (land use, buildings, bridges, roofs, seasons, road looks, the
/// orthophoto corridor) is skipped. `streamed`: the sections come as
/// chunks of which a client may leave optional ones out (the orthophoto
/// corridor, [`STREAM_SKIPPED_SECTIONS`]), so fewer than the count may
/// follow.
fn read_sections(
    r: &mut Bytes,
    samples: u32,
    full: bool,
    streamed: bool,
) -> Result<Sections, String> {
    let mut s = Sections::default();
    let mut building_tags = None;
    let count = r.u32()?;
    for _ in 0..count {
        if streamed && r.pos == r.b.len() {
            break;
        }
        let tag: [u8; 4] = r.arr()?;
        let len = usize::try_from(r.u64()?).map_err(|_| "section too large")?;
        let payload = r.take(len)?;
        if &tag == GRAPH_TAG {
            s.graph = Some(read_graph(payload)?);
        } else if &tag == LANDUSE_TAG && full {
            s.landuse = Some(read_landuse(payload, samples)?);
        } else if &tag == BUILDINGS_TAG && full {
            s.buildings = Some(read_buildings(payload)?);
        } else if &tag == BUILDING_TAGS_TAG && full {
            // Optional styling: a layout this reader does not know
            // is skipped (the houses get their regional defaults).
            match read_building_tags(payload) {
                Ok(t) => building_tags = Some(t),
                Err(e) if e.contains("not supported") => {}
                Err(e) => return Err(e),
            }
        } else if &tag == ROUTES_TAG {
            s.routes = read_routes(payload)?;
        } else if &tag == REGION_TAG {
            s.region = Some(read_region(payload)?);
        } else if &tag == BRIDGES_TAG && full {
            s.bridges = Some(read_bridges(payload)?);
        } else if &tag == TIME_ZONE_TAG {
            let z = std::str::from_utf8(payload)
                .map_err(|_| "time zone id is not UTF-8")?
                .to_owned();
            s.time_zone = Some(z);
        } else if &tag == PALETTE_TAG {
            s.look.palette = Some(read_palette(payload)?);
        } else if &tag == ROOFS_TAG && full {
            s.look.roofs = Some(read_roofs(payload)?);
        } else if &tag == SEASONS_TAG && full {
            s.look.seasons = Some(read_seasons(payload)?);
        } else if &tag == ORTHO_TAG && full {
            // Optional imagery (spec §14 Phase 8 T7): a newer layout is
            // skipped (the ground keeps its materials and colour map).
            match read_ortho(payload) {
                Ok(o) => s.look.ortho = Some(o),
                Err(e) if e.contains("not supported") => {}
                Err(e) => return Err(e),
            }
        } else if &tag == ROAD_SURFACE_TAG && full {
            s.surfaces.extend(read_road_surfaces(payload)?);
        } else if &tag == ROAD_LIT_TAG && full {
            s.lit.extend(read_road_lit(payload)?);
        } else if &tag == ROAD_DRESS_TAG && full {
            match read_road_dress(payload) {
                Ok(dress) => s.dress.extend(dress),
                Err(e) if e.contains("not supported") => {}
                Err(e) => return Err(e),
            }
        }
        // Unknown sections (from newer builders) are skipped.
    }
    if let (Some(b), Some(t)) = (s.buildings.as_mut(), building_tags) {
        apply_building_tags(b, t);
    }
    Ok(s)
}

/// Sections a streaming client does not fetch with the base (spec §14
/// Phase 8 T7, Phase 10 B2): the orthophoto corridor (`ORTC`) is tens to
/// hundreds of MB in one chunk, more than the browser should load before
/// the first tile; streamed packages are drawn without it.
pub const STREAM_SKIPPED_SECTIONS: &[&[u8; 4]] = &[ORTHO_TAG];

/// A package streamed in chunks (spec §4.5, §4.7; the browser app, spec
/// §14 Phase 10 B2): the `header` chunk and the sections are decoded once,
/// the tiles one by one as they arrive (each chunk verified by the caller
/// against the manifest first). [`StreamedPackage::package`] puts the
/// tiles that are there into a [`WorldPackage`] equal to the one
/// [`WorldPackage::from_bytes`] gives with those tiles.
#[derive(Debug, Clone)]
pub struct StreamedPackage {
    pub head: PackageHead,
    /// The package without tiles (graph, routes, land use, buildings …).
    base: WorldPackage,
    surfaces: BTreeMap<TileKey, Vec<RoadSurface>>,
    dress: BTreeMap<TileKey, Vec<RoadDress>>,
    lit: BTreeMap<TileKey, Vec<RoadLit>>,
}

impl StreamedPackage {
    /// From the `header` chunk and, for version 2, the bytes from the
    /// `sections` chunk to the end of the file (the `section …` chunks in
    /// file order; optional ones such as [`STREAM_SKIPPED_SECTIONS`] may be
    /// left out).
    pub fn from_parts(header: &[u8], sections: &[u8]) -> Result<Self, String> {
        let mut r = Bytes::new(header);
        let head = read_head(&mut r)?;
        r.finish("package header")?;
        let s = if head.version >= 2 {
            let mut r = Bytes::new(sections);
            let s = read_sections(&mut r, head.samples, true, true)?;
            r.finish("package sections")?;
            s
        } else {
            if !sections.is_empty() {
                return Err("a version 1 package has no sections".into());
            }
            Sections::default()
        };
        let surfaces = s.surfaces.iter().cloned().collect();
        let dress = s.dress.iter().cloned().collect();
        let lit = s.lit.iter().cloned().collect();
        Ok(Self {
            base: s.into_package(head.clone(), BTreeMap::new()),
            head,
            surfaces,
            dress,
            lit,
        })
    }

    /// The package without its tiles.
    pub fn base(&self) -> &WorldPackage {
        &self.base
    }

    /// Decodes one `tile X,Z` chunk, with its road surfaces and dress.
    pub fn read_tile(&self, bytes: &[u8]) -> Result<Tile, String> {
        let mut r = Bytes::new(bytes);
        let mut tile = read_tile(&mut r, self.head.samples, true)?;
        r.finish("tile")?;
        if let Some(s) = self.surfaces.get(&tile.key) {
            apply_surfaces(&mut tile, s)?;
        }
        if let Some(d) = self.dress.get(&tile.key) {
            apply_dress(&mut tile, d)?;
        }
        if let Some(l) = self.lit.get(&tile.key) {
            apply_lit(&mut tile, l)?;
        }
        Ok(tile)
    }

    /// The package with these tiles.
    pub fn package(&self, tiles: impl IntoIterator<Item = Tile>) -> WorldPackage {
        let mut p = self.base.clone();
        p.tiles = tiles.into_iter().map(|t| (t.key, t)).collect();
        p
    }
}

/// Tags and payload sizes of the tagged sections of an encoded package
/// (empty for version 1 or unreadable bytes), for reports.
pub fn section_sizes(bytes: &[u8]) -> Vec<([u8; 4], u64)> {
    fn walk(bytes: &[u8]) -> Result<Vec<([u8; 4], u64)>, String> {
        let mut r = Bytes::new(bytes);
        if r.take(8)? != MAGIC_V2 {
            return Ok(Vec::new());
        }
        let name_len = r.u32()? as usize;
        r.take(name_len + 16 + 4)?;
        let samples = r.u32()? as usize;
        for _ in 0..r.u32()? {
            r.take(8 + samples * samples * 4)?;
            for _ in 0..r.u32()? {
                r.take(5)?;
                let pts = r.u32()? as usize;
                r.take(pts * 12)?;
            }
        }
        let mut out = Vec::new();
        for _ in 0..r.u32()? {
            let tag: [u8; 4] = r.arr()?;
            let len = r.u64()?;
            r.take(usize::try_from(len).map_err(|_| "section too large")?)?;
            out.push((tag, len));
        }
        Ok(out)
    }
    walk(bytes).unwrap_or_default()
}

/// Layout version inside the routes section.
const ROUTES_VERSION: u32 = 1;

/// Writes the routes section payload (spec §2.7: routes reference stable
/// edge IDs, so they survive rebuilds):
///
/// ```text
/// layout version: u32 (1)
/// route count: u32, then per route:
///   name: u16 length + UTF-8   flags: u8 (1 loop)   step count: u32
///   per step: kind u8 (0 edge, 1 U-turn); edges: stable id u64, from, to: f64
/// ```
pub fn write_routes(routes: &[GraphRoute], b: &mut Vec<u8>) {
    b.extend(ROUTES_VERSION.to_le_bytes());
    b.extend((routes.len() as u32).to_le_bytes());
    for r in routes {
        let mut end = r.name.len().min(usize::from(u16::MAX));
        while !r.name.is_char_boundary(end) {
            end -= 1;
        }
        b.extend((end as u16).to_le_bytes());
        b.extend(&r.name.as_bytes()[..end]);
        b.push(u8::from(r.is_loop));
        b.extend((r.steps.len() as u32).to_le_bytes());
        for s in &r.steps {
            match *s {
                RouteStep::Edge { id, from_m, to_m } => {
                    b.push(0);
                    b.extend(id.0.to_le_bytes());
                    b.extend(from_m.to_le_bytes());
                    b.extend(to_m.to_le_bytes());
                }
                RouteStep::Turnaround => b.push(1),
            }
        }
    }
}

/// Reads a routes section payload written by [`write_routes`].
pub fn read_routes(bytes: &[u8]) -> Result<Vec<GraphRoute>, String> {
    let mut r = Bytes { b: bytes, pos: 0 };
    let version = r.u32()?;
    if version != ROUTES_VERSION {
        return Err(format!("routes layout {version} is not supported"));
    }
    let n = r.u32()?;
    let mut out = Vec::new();
    for _ in 0..n {
        let len = u16::from_le_bytes(r.arr()?) as usize;
        let name = String::from_utf8(r.take(len)?.to_vec()).map_err(|e| e.to_string())?;
        let is_loop = r.u8()? & 1 != 0;
        let count = r.u32()?;
        let mut steps = Vec::new();
        for _ in 0..count {
            steps.push(match r.u8()? {
                0 => RouteStep::Edge {
                    id: EdgeId(r.u64()?),
                    from_m: r.f64()?,
                    to_m: r.f64()?,
                },
                1 => RouteStep::Turnaround,
                k => return Err(format!("unknown route step kind {k}")),
            });
        }
        out.push(GraphRoute {
            name,
            steps,
            is_loop,
        });
    }
    if r.pos != bytes.len() {
        return Err("trailing bytes in the routes section".into());
    }
    Ok(out)
}

const FLAG_BRIDGE: u8 = 1;
const FLAG_TUNNEL: u8 = 2;
const FLAG_BOUNDARY: u8 = 4;
const FLAG_DEAD_END: u8 = 8;
const FLAG_NAME: u8 = 16;

/// Writes the graph section payload:
///
/// ```text
/// layout version: u32 (1)
/// vertex count: u32, vertices: [f64; 2] (x, z)
/// boundary vertex count: u32, vertex indices: u32
/// edge count: u32, then per edge:
///   stable id: u64   from, to: u32 u32   class: u8
///   oneway: u8 (0 no, 1 forward, 2 backward)
///   flags: u8 (1 bridge, 2 tunnel, 4 boundary, 8 dead end, 16 name)
///   width: f32   [dead-end distance: f64]   OSM way: i64
///   [name: u16 length + UTF-8]
///   point count: u32, points: [f64; 2]   profile count: u32, heights: f32
/// ```
pub fn write_graph(g: &RoadGraph, b: &mut Vec<u8>) {
    b.extend(GRAPH_VERSION.to_le_bytes());
    b.extend((g.vertices.len() as u32).to_le_bytes());
    for v in &g.vertices {
        b.extend(v[0].to_le_bytes());
        b.extend(v[1].to_le_bytes());
    }
    b.extend((g.boundary_vertices.len() as u32).to_le_bytes());
    for v in &g.boundary_vertices {
        b.extend(v.0.to_le_bytes());
    }
    b.extend((g.edges.len() as u32).to_le_bytes());
    for e in &g.edges {
        b.extend(e.id.0.to_le_bytes());
        b.extend(e.from.0.to_le_bytes());
        b.extend(e.to.0.to_le_bytes());
        b.push(class_code(e.class));
        b.push(match e.oneway {
            Oneway::No => 0,
            Oneway::Forward => 1,
            Oneway::Backward => 2,
        });
        // Names are short; a pathological one is cut at a char boundary.
        let name = e.name.as_deref().map(|n| {
            let mut end = n.len().min(usize::from(u16::MAX));
            while !n.is_char_boundary(end) {
                end -= 1;
            }
            &n.as_bytes()[..end]
        });
        let mut flags = 0;
        for (on, flag) in [
            (e.bridge, FLAG_BRIDGE),
            (e.tunnel, FLAG_TUNNEL),
            (e.boundary, FLAG_BOUNDARY),
            (e.dead_end_m.is_some(), FLAG_DEAD_END),
            (name.is_some(), FLAG_NAME),
        ] {
            if on {
                flags |= flag;
            }
        }
        b.push(flags);
        b.extend((e.width_m as f32).to_le_bytes());
        if let Some(d) = e.dead_end_m {
            b.extend(d.to_le_bytes());
        }
        b.extend(e.osm_way.to_le_bytes());
        if let Some(n) = name {
            b.extend((n.len() as u16).to_le_bytes());
            b.extend(n);
        }
        b.extend((e.points.len() as u32).to_le_bytes());
        for p in &e.points {
            b.extend(p[0].to_le_bytes());
            b.extend(p[1].to_le_bytes());
        }
        b.extend((e.profile.len() as u32).to_le_bytes());
        for h in &e.profile {
            b.extend((*h as f32).to_le_bytes());
        }
    }
}

/// Reads a graph section payload written by [`write_graph`].
pub fn read_graph(bytes: &[u8]) -> Result<RoadGraph, String> {
    let mut r = Bytes { b: bytes, pos: 0 };
    let version = r.u32()?;
    if version != GRAPH_VERSION {
        return Err(format!("road graph layout {version} is not supported"));
    }
    let n = r.u32()? as usize;
    let vertices = (0..n)
        .map(|_| Ok([r.f64()?, r.f64()?]))
        .collect::<Result<Vec<_>, String>>()?;
    let mut boundary_vertices = std::collections::BTreeSet::new();
    for _ in 0..r.u32()? {
        let v = r.u32()?;
        if v as usize >= n {
            return Err(format!("boundary vertex {v} out of range"));
        }
        boundary_vertices.insert(VertexId(v));
    }
    let ne = r.u32()?;
    let mut edges = Vec::with_capacity(ne.min(1 << 20) as usize);
    for _ in 0..ne {
        let id = EdgeId(r.u64()?);
        let (from, to) = (r.u32()?, r.u32()?);
        if from as usize >= n || to as usize >= n {
            return Err(format!("edge {} has a vertex out of range", id.0));
        }
        let class = class_from(r.u8()?)?;
        let oneway = match r.u8()? {
            0 => Oneway::No,
            1 => Oneway::Forward,
            2 => Oneway::Backward,
            x => return Err(format!("unknown oneway code {x}")),
        };
        let flags = r.u8()?;
        let width_m = f64::from(r.f32()?);
        let dead_end_m = if flags & FLAG_DEAD_END != 0 {
            Some(r.f64()?)
        } else {
            None
        };
        let osm_way = r.i64()?;
        let name = if flags & FLAG_NAME != 0 {
            let len = usize::from(u16::from_le_bytes(r.arr()?));
            Some(String::from_utf8(r.take(len)?.to_vec()).map_err(|e| e.to_string())?)
        } else {
            None
        };
        let np = r.u32()?;
        let points = (0..np)
            .map(|_| Ok([r.f64()?, r.f64()?]))
            .collect::<Result<Vec<_>, String>>()?;
        if points.len() < 2 {
            return Err(format!("edge {} has fewer than two points", id.0));
        }
        let nh = r.u32()?;
        let profile = (0..nh)
            .map(|_| r.f32().map(f64::from))
            .collect::<Result<Vec<_>, String>>()?;
        edges.push(Edge {
            id,
            from: VertexId(from),
            to: VertexId(to),
            points,
            class,
            name,
            width_m,
            bridge: flags & FLAG_BRIDGE != 0,
            tunnel: flags & FLAG_TUNNEL != 0,
            dead_end_m,
            boundary: flags & FLAG_BOUNDARY != 0,
            oneway,
            osm_way,
            profile,
        });
    }
    if r.pos != bytes.len() {
        return Err("trailing bytes in the road graph section".into());
    }
    Ok(RoadGraph {
        vertices,
        edges,
        boundary_vertices,
    })
}

/// Every road class, in the order of their codes in the package.
pub const ROAD_CLASSES: [RoadClass; 10] = [
    RoadClass::Trunk,
    RoadClass::Primary,
    RoadClass::Secondary,
    RoadClass::Tertiary,
    RoadClass::Unclassified,
    RoadClass::Residential,
    RoadClass::Service,
    RoadClass::Track,
    RoadClass::Cycleway,
    RoadClass::Path,
];

fn class_code(c: RoadClass) -> u8 {
    ROAD_CLASSES
        .iter()
        .position(|&x| x == c)
        .expect("all classes listed") as u8
}

fn class_from(code: u8) -> Result<RoadClass, String> {
    ROAD_CLASSES
        .get(usize::from(code))
        .copied()
        .ok_or_else(|| format!("unknown road class {code}"))
}

pub(crate) struct Bytes<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Bytes<'a> {
    pub(crate) fn new(b: &'a [u8]) -> Self {
        Self { b, pos: 0 }
    }
    /// Fails unless everything was read.
    pub(crate) fn finish(&self, what: &str) -> Result<(), String> {
        if self.pos == self.b.len() {
            Ok(())
        } else {
            Err(format!("trailing bytes in the {what}"))
        }
    }
    pub(crate) fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let s = self
            .b
            .get(self.pos..self.pos + n)
            .ok_or("package truncated")?;
        self.pos += n;
        Ok(s)
    }
    pub(crate) fn arr<const N: usize>(&mut self) -> Result<[u8; N], String> {
        Ok(self.take(N)?.try_into().expect("length N"))
    }
    pub(crate) fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    pub(crate) fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.arr()?))
    }
    pub(crate) fn i32(&mut self) -> Result<i32, String> {
        Ok(i32::from_le_bytes(self.arr()?))
    }
    pub(crate) fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_le_bytes(self.arr()?))
    }
    pub(crate) fn f64(&mut self) -> Result<f64, String> {
        Ok(f64::from_le_bytes(self.arr()?))
    }
    pub(crate) fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.arr()?))
    }
    pub(crate) fn i64(&mut self) -> Result<i64, String> {
        Ok(i64::from_le_bytes(self.arr()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn height_range_spans_all_tiles() {
        // Heights 10·i + k + 100·x over the tiles x = -1, 0, 1.
        assert_eq!(package().height_range(), Some((-100.0, 122.0)));
        let mut empty = package();
        empty.tiles.clear();
        assert_eq!(empty.height_range(), None);
    }

    fn package() -> WorldPackage {
        let mut tiles = BTreeMap::new();
        for (x, z) in [(0, 0), (1, 0), (-1, 2)] {
            let key = TileKey { x, z };
            tiles.insert(
                key,
                Tile {
                    key,
                    // 3 × 3 grid: height = 10·x_index + z_index + 100·tile x
                    heights: (0..9)
                        .map(|i| (i % 3 * 10 + i / 3 + 100 * x) as f32)
                        .collect(),
                    roads: vec![RoadStrip {
                        class: RoadClass::Secondary,
                        width_m: 6.5,
                        points: vec![[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]],
                        surface: RoadSurface::Asphalt,
                        dress: RoadDress::of_class(RoadClass::Secondary, 6.5),
                        lit: crate::RoadLit::Unknown,
                    }],
                },
            );
        }
        WorldPackage {
            name: "Test".into(),
            origin: (47.15, 9.55),
            tile_size_m: 1000.0,
            samples: 3,
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

    /// Road heights under a point: on the strip (interpolated), not beside
    /// it; along a direction only strips running that way.
    #[test]
    fn road_heights_under_a_point_and_along_a_direction() {
        let p = package();
        // The strip runs from (1, 2, 3) to (4, 5, 6): diagonal in x/z.
        let h = p.road_heights(2.5, 4.5);
        assert_eq!(h.len(), 1, "{h:?}");
        assert!((h[0] - 3.5).abs() < 1e-6, "{h:?}");
        assert!(p.road_heights(20.0, 40.0).is_empty());
        let s = std::f64::consts::FRAC_1_SQRT_2;
        assert_eq!(p.road_heights_along(2.5, 4.5, Some([s, s])).len(), 1);
        assert_eq!(p.road_heights_along(2.5, 4.5, Some([-s, -s])).len(), 1);
        // A road crossing at right angles is another road.
        assert!(p.road_heights_along(2.5, 4.5, Some([s, -s])).is_empty());
    }

    /// The time zone section (spec §14 Phase 9): written, read back, and
    /// absent in older packages (which still read).
    #[test]
    fn time_zone_round_trips_and_is_optional() {
        let p = WorldPackage {
            time_zone: Some("Europe/Vaduz".into()),
            ..package()
        };
        let b = p.to_bytes();
        assert_eq!(&b[..8], MAGIC_V2);
        let back = WorldPackage::from_bytes(&b).unwrap();
        assert_eq!(back.time_zone.as_deref(), Some("Europe/Vaduz"));
        assert_eq!(back, p);
        assert!(
            section_sizes(&b)
                .iter()
                .any(|(t, n)| t == TIME_ZONE_TAG && *n == 12)
        );
        let old = WorldPackage::from_bytes(&package().to_bytes()).unwrap();
        assert_eq!(old.time_zone, None);
    }

    #[test]
    fn routes_section_round_trip() {
        let route = GraphRoute {
            name: "Rund um Vaduz".into(),
            steps: vec![
                RouteStep::Edge {
                    id: EdgeId(u64::MAX - 1),
                    from_m: 12.5,
                    to_m: 200.0,
                },
                RouteStep::Turnaround,
                RouteStep::Edge {
                    id: EdgeId(u64::MAX - 1),
                    from_m: 200.0,
                    to_m: 0.0,
                },
            ],
            is_loop: true,
        };
        let mut p = WorldPackage {
            graph: Some(graph()),
            ..package()
        };
        p.put_route(route.clone());
        p.put_route(GraphRoute {
            name: "other".into(),
            ..route.clone()
        });
        // Same name (any case): replaced.
        p.put_route(GraphRoute {
            name: "RUND UM VADUZ".into(),
            ..route.clone()
        });
        assert_eq!(p.routes.len(), 2);
        let back = WorldPackage::from_bytes(&p.to_bytes()).unwrap();
        assert_eq!(back, p);
        assert_eq!(back.route("rund um vaduz").unwrap().steps, route.steps);
        assert!(back.route("nope").is_none());
        // Routes alone make a version 2 package too.
        let only = WorldPackage {
            routes: vec![route],
            ..package()
        };
        let b = only.to_bytes();
        assert_eq!(&b[..8], MAGIC_V2);
        assert_eq!(WorldPackage::from_bytes(&b).unwrap(), only);
        let mut bad = Vec::new();
        write_routes(&p.routes, &mut bad);
        bad.push(0);
        assert!(read_routes(&bad).is_err());
    }

    /// A small graph with every kind of attribute (heights exact in f32).
    pub(crate) fn graph() -> RoadGraph {
        let edge = |id: u64, from: u32, to: u32| Edge {
            id: EdgeId(id),
            from: VertexId(from),
            to: VertexId(to),
            points: vec![[0.5, -1.25], [100.125, 7.0], [200.0, 0.0]],
            class: RoadClass::Tertiary,
            name: None,
            width_m: 6.0,
            bridge: false,
            tunnel: false,
            dead_end_m: None,
            boundary: false,
            oneway: Oneway::No,
            osm_way: 42,
            profile: vec![450.5, 451.25, 452.0],
        };
        RoadGraph {
            vertices: vec![[0.5, -1.25], [200.0, 0.0], [300.0, 50.0]],
            edges: vec![
                edge(u64::MAX - 1, 0, 1),
                Edge {
                    name: Some("Landstrasse".into()),
                    class: RoadClass::Primary,
                    bridge: true,
                    oneway: Oneway::Backward,
                    ..edge(7, 1, 2)
                },
                Edge {
                    dead_end_m: Some(812.5),
                    boundary: true,
                    tunnel: true,
                    oneway: Oneway::Forward,
                    name: Some("Größe".into()),
                    profile: Vec::new(),
                    ..edge(8, 2, 0)
                },
            ],
            boundary_vertices: [VertexId(2)].into_iter().collect(),
        }
    }

    #[test]
    fn version_2_carries_the_road_graph() {
        let p = WorldPackage {
            graph: Some(graph()),
            ..package()
        };
        let bytes = p.to_bytes();
        assert_eq!(&bytes[..8], MAGIC_V2);
        assert_eq!(WorldPackage::from_bytes(&bytes).unwrap(), p);
        // Without a graph it stays version 1, byte for byte the old format,
        // and version 2 has the same layout up to the sections.
        let v1 = package().to_bytes();
        assert_eq!(&v1[..8], MAGIC);
        assert_eq!(WorldPackage::from_bytes(&v1).unwrap().graph, None);
        assert_eq!(&bytes[8..v1.len()], &v1[8..]);
        // Truncated graph sections and bad references are rejected.
        assert!(WorldPackage::from_bytes(&bytes[..bytes.len() - 3]).is_err());
        let mut bad = graph();
        bad.edges[0].to = VertexId(99);
        let mut b = Vec::new();
        write_graph(&bad, &mut b);
        assert!(read_graph(&b).unwrap_err().contains("out of range"));
        // A newer version is reported as such.
        let mut v3 = bytes.clone();
        v3[7] = b'3';
        assert!(
            WorldPackage::from_bytes(&v3)
                .unwrap_err()
                .contains("please update")
        );
    }

    /// Road surfaces (section `RSRF`): written only where a strip differs
    /// from its category's default, read back onto the strips, skipped by
    /// readers that do not know it (they see the default).
    #[test]
    fn road_surfaces_round_trip_and_default_packages_have_no_section() {
        let plain = package();
        assert_eq!(&plain.to_bytes()[..8], MAGIC, "no section by default");
        let mut p = package();
        p.tiles
            .get_mut(&TileKey { x: 1, z: 0 })
            .unwrap()
            .roads
            .push(RoadStrip {
                class: RoadClass::Track,
                width_m: 3.0,
                points: vec![[1001.0, 2.0, 3.0], [1004.0, 5.0, 6.0]],
                surface: RoadSurface::Dirt,
                dress: RoadDress::of_class(RoadClass::Track, 3.0),
                lit: crate::RoadLit::Unknown,
            });
        p.tiles.get_mut(&TileKey { x: 0, z: 0 }).unwrap().roads[0].surface = RoadSurface::Paving;
        let b = p.to_bytes();
        assert_eq!(&b[..8], MAGIC_V2);
        let back = WorldPackage::from_bytes(&b).unwrap();
        assert_eq!(back, p);
        assert_eq!(
            section_sizes(&b)
                .iter()
                .filter(|(t, _)| t == ROAD_SURFACE_TAG)
                .count(),
            1
        );
        // Only the two changed tiles are in the section.
        let sections = section_sizes(&b);
        let (_, len) = sections
            .iter()
            .find(|(t, _)| t == ROAD_SURFACE_TAG)
            .unwrap();
        assert_eq!(*len, 4 + (12 + 1) + (12 + 2));
        // An old reader skips the section: every strip has its default.
        let mut old = b.clone();
        let at = old.windows(4).rposition(|w| w == ROAD_SURFACE_TAG).unwrap();
        old[at..at + 4].copy_from_slice(b"XXXX");
        let old = WorldPackage::from_bytes(&old).unwrap();
        for t in old.tiles.values() {
            for s in &t.roads {
                assert_eq!(s.surface, RoadSurface::of_class(s.class));
            }
        }
        // A section that does not match the strips is rejected.
        let mut bad = b.clone();
        let n = bad.len();
        bad[n - 1] = 9;
        assert!(WorldPackage::from_bytes(&bad).is_err());
    }

    /// Spec §14 Phase 9 S2: the roads' `lit` state (section `LITR`) is
    /// added without a new version: it round-trips (also streamed), a
    /// package without it has no section, older readers skip it.
    #[test]
    fn road_lit_states_round_trip_as_an_optional_section() {
        let plain = package();
        assert!(
            section_sizes(&plain.to_bytes())
                .iter()
                .all(|(t, _)| t != crate::lamps::ROAD_LIT_TAG)
        );
        let mut p = package();
        p.tiles.get_mut(&TileKey { x: 0, z: 0 }).unwrap().roads[0].lit = crate::RoadLit::Yes;
        let b = p.to_bytes();
        let back = WorldPackage::from_bytes(&b).unwrap();
        assert_eq!(back, p);
        assert_eq!(
            back.tiles[&TileKey { x: 0, z: 0 }].roads[0].lit,
            crate::RoadLit::Yes
        );
        let mut old = b.clone();
        let at = old
            .windows(4)
            .rposition(|w| w == crate::lamps::ROAD_LIT_TAG)
            .unwrap();
        old[at..at + 4].copy_from_slice(b"XXXX");
        let old = WorldPackage::from_bytes(&old).unwrap();
        assert!(
            old.tiles
                .values()
                .flat_map(|t| &t.roads)
                .all(|s| s.lit == crate::RoadLit::Unknown)
        );
    }

    #[test]
    fn unknown_sections_are_skipped() {
        let p = WorldPackage {
            graph: Some(graph()),
            ..package()
        };
        let mut b = p.to_bytes();
        // Patch the section count to 2 and append an unknown section.
        let count_at = package().to_bytes().len();
        b[count_at..count_at + 4].copy_from_slice(&2u32.to_le_bytes());
        b.extend_from_slice(b"XTRA");
        b.extend(3u64.to_le_bytes());
        b.extend([1, 2, 3]);
        assert_eq!(WorldPackage::from_bytes(&b).unwrap(), p);
    }

    #[test]
    fn land_use_and_buildings_sections_round_trip_and_old_readers_skip_them() {
        use crate::buildings::{Building, BuildingKind, Roof};
        let mut classes = vec![crate::landuse::LandClass::Forest; 4];
        classes[3] = crate::landuse::LandClass::Water;
        let landuse = LandUse {
            cells: 2,
            tiles: BTreeMap::from([(
                TileKey { x: 0, z: 0 },
                crate::landuse::LandUseTile {
                    classes,
                    water: vec![(8, 3.5)],
                },
            )]),
        };
        let buildings = Buildings::from([(
            TileKey { x: 1, z: 0 },
            vec![Building {
                kind: BuildingKind::House,
                outline: vec![
                    [1010.0, 10.0],
                    [1020.0, 10.0],
                    [1020.0, 18.0],
                    [1010.0, 18.0],
                ],
                holes: vec![],
                base_y: 99.5,
                eave_y: 106.0,
                roof: Roof::Gabled { height_m: 3.0 },
                tags: Default::default(),
            }],
        )]);
        // Land use alone already makes a version 2 package.
        let only = WorldPackage {
            landuse: Some(landuse.clone()),
            ..package()
        };
        let b = only.to_bytes();
        assert_eq!(&b[..8], MAGIC_V2);
        assert_eq!(WorldPackage::from_bytes(&b).unwrap(), only);
        let full = WorldPackage {
            graph: Some(graph()),
            landuse: Some(landuse),
            buildings: Some(buildings),
            ..package()
        };
        let mut b = full.to_bytes();
        assert_eq!(WorldPackage::from_bytes(&b).unwrap(), full);
        // A reader that knows only the road graph (the builds before these
        // sections) sees them as unknown tags: rename them and read again.
        for tag in [LANDUSE_TAG, crate::buildings::BUILDINGS_TAG] {
            let at = b.windows(4).rposition(|w| w == tag).unwrap();
            b[at] = b'x';
        }
        let tags: Vec<[u8; 4]> = section_sizes(&full.to_bytes())
            .iter()
            .map(|s| s.0)
            .collect();
        assert_eq!(
            tags,
            vec![*GRAPH_TAG, *LANDUSE_TAG, *crate::buildings::BUILDINGS_TAG]
        );
        assert!(section_sizes(&package().to_bytes()).is_empty());
        let old = WorldPackage::from_bytes(&b).unwrap();
        assert_eq!(old.tiles, full.tiles);
        assert_eq!(old.graph, full.graph);
        assert_eq!((old.landuse, old.buildings), (None, None));
    }

    /// Building tags (section `BTAG`, spec §14 Phase 8 T4) come back on
    /// their buildings; a reader that does not know the section sees the
    /// plain `BLDG` buildings; an unknown layout is skipped, not fatal.
    #[test]
    fn building_tags_section_round_trips_and_is_optional() {
        use crate::buildings::{Building, BuildingKind, BuildingTags, Roof, RoofShape};
        let house = |tags| Building {
            kind: BuildingKind::House,
            outline: vec![[10.0, 10.0], [20.0, 10.0], [20.0, 18.0], [10.0, 18.0]],
            holes: vec![],
            base_y: 99.5,
            eave_y: 106.0,
            roof: Roof::Gabled { height_m: 3.0 },
            tags,
        };
        let hipped = BuildingTags {
            roof_shape: Some(RoofShape::Hipped),
            levels: Some(2),
            ..Default::default()
        };
        let p = WorldPackage {
            buildings: Some(Buildings::from([(
                TileKey { x: 0, z: 0 },
                vec![house(BuildingTags::default()), house(hipped)],
            )])),
            ..package()
        };
        let b = p.to_bytes();
        assert_eq!(WorldPackage::from_bytes(&b).unwrap(), p);
        let tags: Vec<[u8; 4]> = section_sizes(&b).iter().map(|s| s.0).collect();
        assert_eq!(tags, vec![*BUILDINGS_TAG, *BUILDING_TAGS_TAG]);
        let plain = |p: &WorldPackage| {
            let mut q = p.clone();
            for l in q.buildings.as_mut().unwrap().values_mut() {
                for b in l {
                    b.tags = BuildingTags::default();
                }
            }
            q
        };
        // An old reader: the section is unknown.
        let mut old = b.clone();
        let at = old
            .windows(4)
            .rposition(|w| w == BUILDING_TAGS_TAG)
            .unwrap();
        old[at] = b'x';
        assert_eq!(WorldPackage::from_bytes(&old).unwrap(), plain(&p));
        // A newer layout: skipped.
        let mut newer = b.clone();
        let at = newer
            .windows(4)
            .rposition(|w| w == BUILDING_TAGS_TAG)
            .unwrap();
        newer[at + 12..at + 16].copy_from_slice(&7u32.to_le_bytes());
        assert_eq!(WorldPackage::from_bytes(&newer).unwrap(), plain(&p));
        // Without tags there is no section.
        let q = plain(&p);
        let tags: Vec<[u8; 4]> = section_sizes(&q.to_bytes()).iter().map(|s| s.0).collect();
        assert_eq!(tags, vec![*BUILDINGS_TAG]);
    }

    #[test]
    fn look_sections_round_trip_and_old_readers_skip_them() {
        use crate::look::{PALETTE_TAG, ROOFS_TAG, SEASONS_TAG};
        let look = crate::look::tests::sample_look();
        let full = WorldPackage {
            graph: Some(graph()),
            look: look.clone(),
            ..package()
        };
        let b = full.to_bytes();
        assert_eq!(WorldPackage::from_bytes(&b).unwrap(), full);
        let tags: Vec<[u8; 4]> = section_sizes(&b).iter().map(|s| s.0).collect();
        assert_eq!(
            tags,
            vec![*GRAPH_TAG, *PALETTE_TAG, *ROOFS_TAG, *SEASONS_TAG]
        );
        // A skeleton keeps the sources and the palette (credits), not the
        // per-tile roof colours and tint maps.
        let skel = WorldPackage::skeleton_from_bytes(&b).unwrap();
        assert_eq!(skel.look.palette, look.palette);
        assert!(skel.look.roofs.is_none() && skel.look.seasons.is_none());
        // A reader from before T3 sees unknown tags: the rest is unchanged.
        let mut old_b = b.clone();
        for tag in [PALETTE_TAG, ROOFS_TAG, SEASONS_TAG] {
            let at = old_b.windows(4).rposition(|w| w == tag).unwrap();
            old_b[at] = b'x';
        }
        let old = WorldPackage::from_bytes(&old_b).unwrap();
        assert!(old.look.is_empty());
        assert_eq!(
            old,
            WorldPackage {
                look: Default::default(),
                ..full.clone()
            }
        );
        // Without a look a package is written exactly as before.
        let plain = WorldPackage {
            graph: Some(graph()),
            ..package()
        };
        assert_eq!(section_sizes(&plain.to_bytes()).len(), 1);
    }

    /// The orthophoto corridor (spec §14 Phase 8 T7): round trip, left out
    /// of a skeleton, skipped by readers from before T7 and when its layout
    /// is newer than this reader.
    #[test]
    fn ortho_section_round_trips_and_old_readers_skip_it() {
        use crate::ortho::ORTHO_TAG;
        let mut look = crate::look::tests::sample_look();
        look.ortho = Some(crate::ortho::tests::sample_corridor());
        let full = WorldPackage {
            graph: Some(graph()),
            look,
            ..package()
        };
        let b = full.to_bytes();
        assert_eq!(WorldPackage::from_bytes(&b).unwrap(), full);
        assert!(section_sizes(&b).iter().any(|s| &s.0 == ORTHO_TAG));
        let skel = WorldPackage::skeleton_from_bytes(&b).unwrap();
        assert!(skel.look.ortho.is_none());
        let without = |p: &WorldPackage| WorldPackage {
            look: crate::look::RegionLook {
                ortho: None,
                ..p.look.clone()
            },
            ..p.clone()
        };
        // An old reader: an unknown tag.
        let mut old_b = b.clone();
        let at = old_b.windows(4).rposition(|w| w == ORTHO_TAG).unwrap();
        old_b[at] = b'x';
        assert_eq!(WorldPackage::from_bytes(&old_b).unwrap(), without(&full));
        // A newer layout of the section: skipped, the rest is read.
        let mut newer = b.clone();
        newer[at + 12] = 9;
        assert_eq!(WorldPackage::from_bytes(&newer).unwrap(), without(&full));
    }

    #[test]
    fn region_section_round_trip_and_old_readers_skip_it() {
        let mut region = PackageRegion::bbox(47.0, 9.4, 47.2, 9.6);
        region
            .outer
            .push(vec![(47.0, 9.4), (47.2, 9.4), (47.1, 9.6)]);
        let p = WorldPackage {
            graph: Some(graph()),
            region: Some(region.clone()),
            ..package()
        };
        let mut b = p.to_bytes();
        assert_eq!(&b[..8], MAGIC_V2);
        let back = WorldPackage::from_bytes(&b).unwrap();
        assert_eq!(back.region, Some(region));
        // A reader from before the section sees an unknown tag.
        let at = b.windows(4).rposition(|w| w == REGION_TAG).unwrap();
        b[at] = b'x';
        let old = WorldPackage::from_bytes(&b).unwrap();
        assert_eq!(old.region, None);
        assert_eq!(old.graph, p.graph);
    }

    /// Spec §4.8.6: the bridges section is additive – older readers skip it
    /// and keep everything else.
    #[test]
    fn bridges_section_round_trip_and_old_readers_skip_it() {
        let mut bridges = crate::bridges::Bridges::new();
        bridges.insert(
            TileKey { x: 0, z: 0 },
            vec![crate::bridges::tests::bridge(
                crate::bridges::BridgeStructure::Beam,
                crate::bridges::Railing::Parapet,
            )],
        );
        let p = WorldPackage {
            graph: Some(graph()),
            bridges: Some(bridges.clone()),
            ..package()
        };
        let mut b = p.to_bytes();
        let back = WorldPackage::from_bytes(&b).unwrap();
        assert_eq!(back.bridges, Some(bridges));
        let at = b.windows(4).rposition(|w| w == BRIDGES_TAG).unwrap();
        b[at] = b'x';
        let old = WorldPackage::from_bytes(&b).unwrap();
        assert_eq!(old.bridges, None);
        assert_eq!(old.graph, p.graph);
        assert_eq!(old.tiles, p.tiles);
    }

    #[test]
    fn binary_round_trip() {
        let p = package();
        let bytes = p.to_bytes();
        assert_eq!(&bytes[..8], MAGIC);
        assert_eq!(WorldPackage::from_bytes(&bytes).unwrap(), p);
    }

    #[test]
    fn broken_packages_are_rejected() {
        let bytes = package().to_bytes();
        assert!(WorldPackage::from_bytes(&bytes[..bytes.len() - 1]).is_err());
        assert!(WorldPackage::from_bytes(b"OCWPKG99").is_err());
        let mut more = bytes.clone();
        more.push(0);
        assert!(WorldPackage::from_bytes(&more).is_err());
        let mut bad_class = package();
        bad_class.tiles.values_mut().next().unwrap().roads[0].class = RoadClass::Path;
        let mut b = bad_class.to_bytes();
        // Corrupt the class byte of the first road.
        let pos = b
            .windows(4)
            .position(|w| w == 6.5f32.to_le_bytes())
            .unwrap()
            - 1;
        b[pos] = 42;
        assert!(WorldPackage::from_bytes(&b).unwrap_err().contains("class"));
    }

    #[test]
    fn heights_are_bilinear_within_tiles() {
        let p = package();
        assert_eq!(p.spacing_m(), 500.0);
        assert_eq!(p.height(0.0, 0.0), Some(0.0));
        assert_eq!(p.height(500.0, 500.0), Some(11.0));
        assert_eq!(p.height(250.0, 0.0), Some(5.0));
        assert_eq!(p.height(1500.0, 0.0), Some(110.0));
        assert_eq!(p.height(5000.0, 0.0), None);
        assert_eq!(
            TileKey::containing([-1.0, 2500.0], 1000.0),
            TileKey { x: -1, z: 2 }
        );
        assert_eq!(TileKey { x: 0, z: 0 }.ring(TileKey { x: -2, z: 1 }), 2);
    }

    /// Streaming (spec §14 Phase 10 B2): the header, the sections and any
    /// subset of the tile chunks give the package with those tiles, road
    /// surfaces included, equal to the whole file's decode.
    #[test]
    fn a_streamed_package_equals_the_decoded_file() {
        let mut p = WorldPackage {
            time_zone: Some("Europe/Vaduz".into()),
            ..package()
        };
        // A surface that differs from the class default: an RSRF section.
        p.tiles.get_mut(&TileKey { x: 1, z: 0 }).unwrap().roads[0].surface = RoadSurface::Gravel;
        let bytes = p.to_bytes();
        let chunks = crate::manifest::package_chunks(&bytes).unwrap();
        let part = |name: &str| {
            let (_, r) = chunks.iter().find(|(n, _)| n == name).unwrap();
            &bytes[r.clone()]
        };
        let sections_start = chunks
            .iter()
            .find(|(n, _)| n == "sections")
            .unwrap()
            .1
            .start;
        let s = StreamedPackage::from_parts(part("header"), &bytes[sections_start..]).unwrap();
        assert_eq!(s.head.tile_count, 3);
        assert_eq!(s.head.name, "Test");
        assert!(s.base().tiles.is_empty());
        assert_eq!(s.base().time_zone.as_deref(), Some("Europe/Vaduz"));
        let tile = s.read_tile(part("tile 1,0")).unwrap();
        assert_eq!(tile.roads[0].surface, RoadSurface::Gravel);
        let other = s.read_tile(part("tile -1,2")).unwrap();
        let streamed = s.package([tile, other]);
        let mut want = WorldPackage::from_bytes(&bytes).unwrap();
        want.tiles.remove(&TileKey { x: 0, z: 0 });
        assert_eq!(streamed, want);
        assert!(streamed.tile_bytes() > 2 * 9 * 4);
        // Damaged parts are errors, not panics.
        assert!(s.read_tile(&part("tile 1,0")[..20]).is_err());
        assert!(StreamedPackage::from_parts(&part("header")[..10], &[]).is_err());
        let cut = &bytes[sections_start..bytes.len() - 1];
        assert!(StreamedPackage::from_parts(part("header"), cut).is_err());
    }

    /// A streaming client leaves the orthophoto corridor out of the base
    /// (spec §14 Phase 8 T7): the sections without the `ORTC` chunk give
    /// the package without the corridor.
    #[test]
    fn a_streamed_package_without_the_ortho_section() {
        let mut p = WorldPackage {
            time_zone: Some("Europe/Vaduz".into()),
            ..package()
        };
        p.look.ortho = Some(crate::ortho::tests::sample_corridor());
        p.tiles.get_mut(&TileKey { x: 1, z: 0 }).unwrap().roads[0].surface = RoadSurface::Gravel;
        // A lit road (section LITR, spec §14 Phase 9 S2): kept by streaming.
        p.tiles.get_mut(&TileKey { x: 1, z: 0 }).unwrap().roads[0].lit = crate::RoadLit::Yes;
        let bytes = p.to_bytes();
        let chunks = crate::manifest::package_chunks(&bytes).unwrap();
        let header = chunks
            .iter()
            .find(|(n, _)| n == "header")
            .unwrap()
            .1
            .clone();
        let mut sections = Vec::new();
        for (name, r) in &chunks {
            if name == "sections"
                || (name.starts_with("section ")
                    && !STREAM_SKIPPED_SECTIONS
                        .iter()
                        .any(|t| name == &format!("section {}", String::from_utf8_lossy(*t))))
            {
                sections.extend_from_slice(&bytes[r.clone()]);
            }
        }
        assert!(chunks.iter().any(|(n, _)| n == "section ORTC"));
        let s = StreamedPackage::from_parts(&bytes[header], &sections).unwrap();
        assert!(s.base().look.ortho.is_none());
        assert_eq!(s.base().time_zone.as_deref(), Some("Europe/Vaduz"));
        // The road surfaces (a section after ORTC) are still there.
        let tile_r = chunks
            .iter()
            .find(|(n, _)| n == "tile 1,0")
            .unwrap()
            .1
            .clone();
        let tile = s.read_tile(&bytes[tile_r]).unwrap();
        assert_eq!(tile.roads[0].surface, RoadSurface::Gravel);
        assert_eq!(tile.roads[0].lit, crate::RoadLit::Yes);
        assert!(chunks.iter().any(|(n, _)| n == "section LITR"));
        // With every chunk the corridor is there.
        let start = chunks
            .iter()
            .find(|(n, _)| n == "sections")
            .unwrap()
            .1
            .start;
        let header = chunks
            .iter()
            .find(|(n, _)| n == "header")
            .unwrap()
            .1
            .clone();
        let all = StreamedPackage::from_parts(&bytes[header], &bytes[start..]).unwrap();
        assert_eq!(all.base().look.ortho, p.look.ortho);
    }
}
