//! What a world package is, read without loading it (spec §3.2.1 World
//! menu, §4.7): its name, origin, tiles, routes, region and time zone.
//!
//! [`summarize`] walks the package layout (see [`crate::package`]) and
//! *skips* the bulky parts – the terrain heights, the road strips' points,
//! the road graph, land use and buildings – with seeks, reading only the
//! header, the counts and the small sections `ROUT`, `REGN` and `TZID`.
//! A 260 MB package is summarised in milliseconds, so the world list can
//! show every installed package without loading (or verifying) each one;
//! the full check (hashes, signature, every section) happens when a package
//! is ridden.

use std::io::{BufReader, Read, Seek, SeekFrom};

use crate::graph_route::GraphRoute;
use crate::package::{GRAPH_TAG, MAGIC, MAGIC_V2, ROUTES_TAG, TIME_ZONE_TAG, read_routes};
use crate::region::{PackageRegion, REGION_TAG, read_region};

/// Small sections larger than this are not read (a damaged length).
const MAX_SMALL_SECTION: u64 = 64 << 20;

/// A package as the world list shows it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PackageSummary {
    pub name: String,
    /// Projection centre (lat, lon).
    pub origin: (f64, f64),
    /// Format version (1 or 2).
    pub format: u8,
    pub tile_size_m: f32,
    pub tiles: usize,
    /// It has a road graph (section `GRPH`: free ride, junctions).
    pub has_graph: bool,
    /// The routes (section `ROUT`), in package order.
    pub routes: Vec<GraphRoute>,
    pub region: Option<PackageRegion>,
    pub time_zone: Option<String>,
    /// The file's size in bytes.
    pub size_bytes: u64,
}

impl PackageSummary {
    /// The area of terrain the tiles cover, km².
    pub fn terrain_km2(&self) -> f64 {
        self.tiles as f64 * f64::from(self.tile_size_m).powi(2) / 1e6
    }

    /// The names of the routes.
    pub fn route_names(&self) -> Vec<&str> {
        self.routes.iter().map(|r| r.name.as_str()).collect()
    }
}

struct Reader<R> {
    r: BufReader<R>,
    pos: u64,
    len: u64,
}

impl<R: Read + Seek> Reader<R> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], String> {
        if self.pos + N as u64 > self.len {
            return Err("package truncated".into());
        }
        let mut b = [0u8; N];
        self.r
            .read_exact(&mut b)
            .map_err(|_| "package truncated".to_string())?;
        self.pos += N as u64;
        Ok(b)
    }
    fn bytes(&mut self, n: u64) -> Result<Vec<u8>, String> {
        if self.pos + n > self.len {
            return Err("package truncated".into());
        }
        let mut b = vec![0u8; usize::try_from(n).map_err(|_| "section too large")?];
        self.r
            .read_exact(&mut b)
            .map_err(|_| "package truncated".to_string())?;
        self.pos += n;
        Ok(b)
    }
    fn skip(&mut self, n: u64) -> Result<(), String> {
        if self.pos + n > self.len {
            return Err("package truncated".into());
        }
        let n = i64::try_from(n).map_err(|_| "package truncated")?;
        self.r.seek_relative(n).map_err(|e| e.to_string())?;
        self.pos += n as u64;
        Ok(())
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take()?))
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.take()?))
    }
    fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_le_bytes(self.take()?))
    }
    fn f64(&mut self) -> Result<f64, String> {
        Ok(f64::from_le_bytes(self.take()?))
    }
}

/// Summarises an encoded package from a seekable source (a file).
pub fn summarize<R: Read + Seek>(mut source: R) -> Result<PackageSummary, String> {
    let len = source.seek(SeekFrom::End(0)).map_err(|e| e.to_string())?;
    source.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut r = Reader {
        r: BufReader::with_capacity(64 << 10, source),
        pos: 0,
        len,
    };
    let magic: [u8; 8] = r
        .take()
        .map_err(|_| "not an OCW world package (too short)".to_string())?;
    let format = match &magic {
        m if m == MAGIC => 1,
        m if m == MAGIC_V2 => 2,
        m if m.starts_with(b"OCWPKG") => {
            return Err(format!(
                "world package version {} is not supported (this build reads 01 and 02)",
                String::from_utf8_lossy(&m[6..])
            ));
        }
        _ => return Err("not an OCW world package (OCWPKG01/02)".into()),
    };
    let name_len = r.u32()?;
    if u64::from(name_len) > 4096 {
        return Err("package header damaged (name length)".into());
    }
    let name = String::from_utf8(r.bytes(u64::from(name_len))?)
        .map_err(|_| "package name is not UTF-8".to_string())?;
    let origin = (r.f64()?, r.f64()?);
    let tile_size_m = r.f32()?;
    let samples = u64::from(r.u32()?);
    if samples < 2 {
        return Err("tile grid needs at least 2 samples".into());
    }
    let tiles = r.u32()? as usize;
    for _ in 0..tiles {
        r.skip(8 + samples * samples * 4)?;
        let roads = r.u32()?;
        for _ in 0..roads {
            r.skip(5)?;
            let pts = u64::from(r.u32()?);
            r.skip(pts * 12)?;
        }
    }
    let mut s = PackageSummary {
        name,
        origin,
        format,
        tile_size_m,
        tiles,
        size_bytes: len,
        ..PackageSummary::default()
    };
    if format >= 2 {
        let sections = r.u32()?;
        for _ in 0..sections {
            let tag: [u8; 4] = r.take()?;
            let n = r.u64()?;
            let small = [ROUTES_TAG, REGION_TAG, TIME_ZONE_TAG].contains(&&tag);
            if !small || n > MAX_SMALL_SECTION {
                if &tag == GRAPH_TAG {
                    s.has_graph = true;
                }
                r.skip(n)?;
                continue;
            }
            let payload = r.bytes(n)?;
            if &tag == ROUTES_TAG {
                s.routes = read_routes(&payload)?;
            } else if &tag == REGION_TAG {
                s.region = Some(read_region(&payload)?);
            } else {
                s.time_zone =
                    Some(String::from_utf8(payload).map_err(|_| "time zone id is not UTF-8")?);
            }
        }
    }
    if r.pos != len {
        return Err("trailing bytes after the last tile or section".into());
    }
    Ok(s)
}

/// Summarises the package file at `path`.
pub fn summarize_file(path: &std::path::Path) -> Result<PackageSummary, String> {
    let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    summarize(f)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{Edge, EdgeId, Oneway, RoadClass, RoadGraph, VertexId};
    use crate::graph_route::RouteStep;
    use crate::package::{RoadStrip, Tile, TileKey, WorldPackage};
    use std::collections::BTreeMap;
    use std::io::Cursor;

    fn package(sections: bool) -> WorldPackage {
        let mut tiles = BTreeMap::new();
        for (x, z) in [(0, 0), (1, 0), (-1, 2)] {
            let key = TileKey { x, z };
            tiles.insert(
                key,
                Tile {
                    key,
                    heights: (0..16).map(|i| i as f32).collect(),
                    roads: vec![RoadStrip {
                        class: RoadClass::Secondary,
                        width_m: 6.5,
                        points: vec![[1.0, 2.0, 3.0], [4.0, 5.0, 6.0], [7.0, 8.0, 9.0]],
                    }],
                },
            );
        }
        let graph = RoadGraph {
            vertices: vec![[0.0, 0.0], [100.0, 0.0]],
            edges: vec![Edge {
                id: EdgeId(7),
                from: VertexId(0),
                to: VertexId(1),
                points: vec![[0.0, 0.0], [100.0, 0.0]],
                class: RoadClass::Residential,
                name: None,
                width_m: 5.0,
                bridge: false,
                tunnel: false,
                dead_end_m: None,
                boundary: false,
                oneway: Oneway::No,
                osm_way: 1,
                profile: Vec::new(),
            }],
            boundary_vertices: Default::default(),
        };
        let route = |name: &str| GraphRoute {
            name: name.into(),
            steps: vec![RouteStep::Edge {
                id: EdgeId(7),
                from_m: 0.0,
                to_m: 100.0,
            }],
            is_loop: false,
        };
        WorldPackage {
            name: "Example Land".into(),
            origin: (47.15, 9.55),
            tile_size_m: 1000.0,
            samples: 4,
            tiles,
            graph: sections.then_some(graph),
            landuse: None,
            buildings: None,
            routes: if sections {
                vec![route("Valley"), route("Pass")]
            } else {
                Vec::new()
            },
            region: sections.then(|| PackageRegion::bbox(47.0, 9.4, 47.3, 9.7)),
            time_zone: sections.then(|| "Europe/Vaduz".into()),
            bridges: None,
            look: Default::default(),
        }
    }

    #[test]
    fn summaries_read_the_header_and_the_small_sections() {
        let bytes = package(true).to_bytes();
        let s = summarize(Cursor::new(&bytes)).unwrap();
        assert_eq!(s.name, "Example Land");
        assert_eq!((s.format, s.tiles, s.tile_size_m), (2, 3, 1000.0));
        assert_eq!(s.origin, (47.15, 9.55));
        assert!(s.has_graph);
        assert_eq!(s.route_names(), ["Valley", "Pass"]);
        assert_eq!(s.region.as_ref().unwrap().bounds(), [47.0, 9.4, 47.3, 9.7]);
        assert_eq!(s.time_zone.as_deref(), Some("Europe/Vaduz"));
        assert_eq!(s.size_bytes, bytes.len() as u64);
        assert_eq!(s.terrain_km2(), 3.0);
        // The same as a full load says.
        let full = WorldPackage::from_bytes(&bytes).unwrap();
        assert_eq!(full.routes, s.routes);
        // Version 1: no sections.
        let s = summarize(Cursor::new(package(false).to_bytes())).unwrap();
        assert_eq!((s.format, s.has_graph, s.routes.len()), (1, false, 0));
        assert!(s.region.is_none() && s.time_zone.is_none());
    }

    #[test]
    fn broken_packages_say_why() {
        let bytes = package(true).to_bytes();
        let err = |b: &[u8]| summarize(Cursor::new(b.to_vec())).unwrap_err();
        assert!(err(b"hello").contains("not an OCW world package"));
        assert!(err(b"PK\x03\x04 a zip file, not a package").contains("OCWPKG01/02"));
        assert!(err(b"OCWPKG09rest").contains("version 09 is not supported"));
        assert!(err(&bytes[..bytes.len() - 3]).contains("truncated"));
        assert!(err(&bytes[..200]).contains("truncated"));
        let mut more = bytes.clone();
        more.extend_from_slice(b"xx");
        assert!(err(&more).contains("trailing bytes"));
        assert!(err(b"").contains("too short"));
    }
}
