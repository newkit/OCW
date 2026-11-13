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
//! `REGN` – the region the package was cut to (box and polygon in
//! latitude / longitude), for neighbouring packages, see [`crate::region`];
//! `TZID` – the IANA time zone of the package's area ("Europe/Vaduz", UTF-8,
//! spec §14 Phase 9; packages without it fall back to a zone from their
//! origin, see `ocw_sky::zone::zone_for`). A
//! package without sections is written as version 1, byte for byte as before.
//!
//! Tile `(x, z)` covers local metres `[x·size, (x+1)·size] × [z·size, (z+1)·size]`;
//! the grid includes both borders, so neighbouring tiles share their edge
//! samples (no cracks). The manifest with hashes and signatures (§4.7)
//! travels next to the package: see [`crate::manifest`].

use std::collections::BTreeMap;

use crate::buildings::{BUILDINGS_TAG, Buildings, read_buildings, write_buildings};
use crate::graph::{Edge, EdgeId, Oneway, RoadClass, RoadGraph, VertexId};
use crate::graph_route::{GraphRoute, RouteStep};
use crate::landuse::{LANDUSE_TAG, LandUse, read_landuse, write_landuse};
use crate::region::{PackageRegion, REGION_TAG, read_region, write_region};

pub const MAGIC: &[u8; 8] = b"OCWPKG01";
/// Version 2: version 1 plus tagged sections (the road graph).
pub const MAGIC_V2: &[u8; 8] = b"OCWPKG02";
/// Section tag of the road graph.
pub const GRAPH_TAG: &[u8; 4] = b"GRPH";
/// Section tag of the routes on the road graph (spec §2.7).
pub const ROUTES_TAG: &[u8; 4] = b"ROUT";
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
    /// Routes on the road graph (section `ROUT`, spec §2.7).
    pub routes: Vec<GraphRoute>,
    /// The region the roads were cut to (section `REGN`; `None` in older
    /// packages, which then cannot have neighbours, spec §2.2.2).
    pub region: Option<PackageRegion>,
    /// The IANA time zone of the area (section `TZID`, spec §14 Phase 9),
    /// set by the world builder; `None` in older packages.
    pub time_zone: Option<String>,
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
        }
        if let Some(r) = &self.region {
            let mut payload = Vec::new();
            write_region(r, &mut payload);
            sections.push((REGION_TAG, payload));
        }
        if let Some(z) = &self.time_zone {
            sections.push((TIME_ZONE_TAG, z.as_bytes().to_vec()));
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
        let mut r = Bytes { b: bytes, pos: 0 };
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
        let count = r.u32()?;
        let mut tiles = BTreeMap::new();
        for _ in 0..count {
            let key = TileKey {
                x: r.i32()?,
                z: r.i32()?,
            };
            let n = samples as usize * samples as usize;
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
                });
            }
            tiles.insert(
                key,
                Tile {
                    key,
                    heights,
                    roads,
                },
            );
        }
        let mut graph = None;
        let (mut landuse, mut buildings) = (None, None);
        let mut routes = Vec::new();
        let mut region = None;
        let mut time_zone = None;
        if version >= 2 {
            let sections = r.u32()?;
            for _ in 0..sections {
                let tag: [u8; 4] = r.arr()?;
                let len = usize::try_from(r.u64()?).map_err(|_| "section too large")?;
                let payload = r.take(len)?;
                if &tag == GRAPH_TAG {
                    graph = Some(read_graph(payload)?);
                } else if &tag == LANDUSE_TAG {
                    landuse = Some(read_landuse(payload, samples)?);
                } else if &tag == BUILDINGS_TAG {
                    buildings = Some(read_buildings(payload)?);
                } else if &tag == ROUTES_TAG {
                    routes = read_routes(payload)?;
                } else if &tag == REGION_TAG {
                    region = Some(read_region(payload)?);
                } else if &tag == TIME_ZONE_TAG {
                    let z = std::str::from_utf8(payload)
                        .map_err(|_| "time zone id is not UTF-8")?
                        .to_owned();
                    time_zone = Some(z);
                }
                // Unknown sections (from newer builders) are skipped.
            }
        }
        if r.pos != bytes.len() {
            return Err("trailing bytes after the last tile or section".into());
        }
        Ok(Self {
            name,
            origin,
            tile_size_m,
            samples,
            tiles,
            graph,
            landuse,
            buildings,
            routes,
            region,
            time_zone,
        })
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

const CLASSES: [RoadClass; 10] = [
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
    CLASSES
        .iter()
        .position(|&x| x == c)
        .expect("all classes listed") as u8
}

fn class_from(code: u8) -> Result<RoadClass, String> {
    CLASSES
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
            routes: Vec::new(),
            region: None,
            time_zone: None,
        }
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
}
