//! The junctions stored in a package (spec §4.7 *Junction sections*,
//! §4.8.8 J2): two optional, additive sections. Older readers skip their
//! tags; a newer layout, or one made for another road graph, is ignored
//! and the junctions are computed from the graph as before (J1), with the
//! same result.
//!
//! **`JNCT` – the junction rules and patch outlines**, so the loader
//! searches nothing (which vertices have a patch or a mitre, the arms'
//! mouths, the outline points):
//!
//! ```text
//! layout version: u8 (2 since J3: the side roads' corners, 4.8.8 Q6 (d),
//!   change the drawn surface and clusters came; layout 1 is ignored)
//! flags: u8 (bit 0: the profiles are reconciled with the patches and the
//!   terrain is settled under them in the package – J2; the renderer then
//!   leaves the terrain as it is)
//! graph fingerprint: u64 (RoadGraph::fingerprint of the GRPH section)
//! vertex count: u32
//! rule count: u32, then per rule (by growing vertex):
//!   record length: u16 (the bytes after it; readers skip unknown bytes
//!     at a record's end)
//!   vertex: u32
//!   kind: u8 (0 patch, 1 mitre, 2 cluster patch, 3 cluster member;
//!     records of unknown kinds are skipped)
//!   patch and cluster patch:
//!     arm count: u8, per arm (by polar angle): edge u32, flags u8 (bit 0:
//!       the edge's `from` end is here), mouth m: f64 (from its vertex),
//!       along: f64 (the strip's along coordinate at the mouth)
//!     outline count: u16, per point: dx, dz f32 (from the patch's centre:
//!       the vertex, a cluster's centroid), mouth arm u8 (255: none)
//!   cluster patch (J3, at its first vertex) also: member count u8,
//!     members u32 (growing, the first this record's), inner edge count
//!     u8, inner edges u32 (inside the patch, no strip)
//!   cluster member: the first member u32 (its patch is that one's)
//! ```
//!
//! Vertices without a record have no junction surface (the overlapping
//! strips of before). Mitres store only their kind: they are cheap.
//!
//! **`RDGE` – the edge of every road strip**, replacing the renderer's
//! geometric matching:
//!
//! ```text
//! layout version: u8 (1)
//! graph fingerprint: u64
//! tile count: u32, then per tile (by key): x, z i32, strip count u32,
//!   per strip in the tile's order: edge index u32 (u32::MAX: none)
//! ```
//!
//! A tile whose strip count differs from the tile's strips is ignored
//! (matched by geometry).

use std::collections::BTreeMap;
use std::sync::Arc;

use super::{JunctionSet, VertexRule};
use crate::graph::RoadGraph;
use crate::package::{Bytes, TileKey};

pub const JUNCTIONS_TAG: &[u8; 4] = b"JNCT";
pub const STRIP_EDGES_TAG: &[u8; 4] = b"RDGE";
/// 2 (J3, 4.8.8 Q6 (d)): the side roads' corners change the drawn surface
/// (a layout 1 section was made for J2's) and clusters (kinds 2 and 3).
const JUNCTIONS_VERSION: u8 = 2;
const STRIP_EDGES_VERSION: u8 = 1;
/// `JNCT` flag: profiles reconciled, terrain settled (J2).
pub const FLAG_SETTLED: u8 = 1;

/// One arm of a stored patch.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredArm {
    pub edge: u32,
    pub at_from: bool,
    pub mouth_m: f64,
    pub along: f64,
}

/// One outline point of a stored patch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StoredPoint {
    /// Offset from the vertex (x, z).
    pub d: [f32; 2],
    /// On the mouth of this arm.
    pub mouth: Option<u8>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StoredPatch {
    pub arms: Vec<StoredArm>,
    pub outline: Vec<StoredPoint>,
    /// A cluster's vertices (its first is the record's) and the edges
    /// inside it (J3); empty for one vertex.
    pub members: Vec<u32>,
    pub inner: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StoredRule {
    Patch(StoredPatch),
    Mitre,
    /// A cluster's other vertex: the patch is that of its first member.
    Member(u32),
}

/// The stored junction rules (section `JNCT`).
#[derive(Debug, Clone, PartialEq)]
pub struct JunctionIndex {
    pub flags: u8,
    pub fingerprint: u64,
    pub vertex_count: u32,
    /// By growing vertex.
    pub rules: Vec<(u32, StoredRule)>,
}

impl JunctionIndex {
    /// The rules of every vertex of `g` as `set` makes them (`set` made for
    /// `g`), with `flags`.
    pub fn of_set(g: &RoadGraph, set: &JunctionSet, flags: u8) -> Self {
        let mut rules = Vec::new();
        for v in 0..g.vertices.len() {
            let Some(rule) = set.rule(g, v) else {
                continue;
            };
            let stored = match rule {
                VertexRule::Mitre(_) => StoredRule::Mitre,
                VertexRule::Patch(p) if !p.is_at(v) => StoredRule::Member(p.vertex),
                VertexRule::Patch(p) => {
                    // Offsets from the patch's centre (a cluster's centroid).
                    let c = [p.centre[0], p.centre[2]];
                    let cluster = p.members.len() > 1;
                    StoredRule::Patch(StoredPatch {
                        members: if cluster { p.members.clone() } else { Vec::new() },
                        inner: p.inner.iter().map(|&e| e as u32).collect(),
                        arms: p
                            .arms
                            .iter()
                            .map(|a| StoredArm {
                                edge: a.edge as u32,
                                at_from: a.at_from,
                                mouth_m: a.mouth_m,
                                along: a.along,
                            })
                            .collect(),
                        outline: p
                            .outline
                            .iter()
                            .map(|o| StoredPoint {
                                d: [(o.p[0] - c[0]) as f32, (o.p[2] - c[1]) as f32],
                                mouth: o.mouth.map(|m| m as u8),
                            })
                            .collect(),
                    })
                }
            };
            rules.push((v as u32, stored));
        }
        Self {
            flags,
            fingerprint: g.fingerprint(),
            vertex_count: g.vertices.len() as u32,
            rules,
        }
    }

    /// Made for this graph.
    pub fn fits(&self, g: &RoadGraph) -> bool {
        self.vertex_count as usize == g.vertices.len() && self.fingerprint == g.fingerprint()
    }

    /// The rule stored for vertex `v` (`None`: no junction surface).
    pub fn get(&self, v: u32) -> Option<&StoredRule> {
        self.rules
            .binary_search_by_key(&v, |r| r.0)
            .ok()
            .map(|i| &self.rules[i].1)
    }

    /// The profiles are reconciled and the terrain settled (J2).
    pub fn settled(&self) -> bool {
        self.flags & FLAG_SETTLED != 0
    }

    pub fn patches(&self) -> usize {
        self.rules
            .iter()
            .filter(|r| matches!(r.1, StoredRule::Patch(_)))
            .count()
    }
}

/// The edge of each road strip, per tile (section `RDGE`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StripEdges {
    pub fingerprint: u64,
    pub tiles: BTreeMap<TileKey, Vec<u32>>,
}

/// No edge.
pub const NO_EDGE: u32 = u32::MAX;

impl StripEdges {
    /// The edges of tile `key`'s strips if stored for `count` strips.
    pub fn tile(&self, key: TileKey, count: usize) -> Option<&[u32]> {
        self.tiles
            .get(&key)
            .filter(|v| v.len() == count)
            .map(Vec::as_slice)
    }
}

/// Both junction sections of a package (`None` fields: not there, or of a
/// layout this reader does not know).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct JunctionData {
    pub index: Option<Arc<JunctionIndex>>,
    pub strip_edges: Option<StripEdges>,
}

impl JunctionData {
    /// The index if it was made for `g`.
    pub fn index_for(&self, g: &RoadGraph) -> Option<Arc<JunctionIndex>> {
        self.index.clone().filter(|i| i.fits(g))
    }

    /// The strip edges if they were made for `g`.
    pub fn strip_edges_for(&self, g: &RoadGraph) -> Option<&StripEdges> {
        self.strip_edges
            .as_ref()
            .filter(|s| s.fingerprint == g.fingerprint())
    }
}

/// Strips are matched to an edge whose polyline passes this close (m).
const MATCH_M: f64 = 0.2;
/// Grid cell for the matching (m).
const CELL_M: f64 = 20.0;
/// A tile's strips reach this far into the next tile (one point, ≤ 5 m).
pub const TILE_REACH_M: f64 = 6.0;

/// The edges that may have strips in each tile of `size` metres (by their
/// bounding box, tunnels left out: they have no strips).
pub fn tile_edges(g: &RoadGraph, size: f64) -> std::collections::HashMap<TileKey, Vec<u32>> {
    let mut out: std::collections::HashMap<TileKey, Vec<u32>> = Default::default();
    for (ei, e) in g.edges.iter().enumerate() {
        if e.tunnel || e.points.is_empty() {
            continue;
        }
        let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        for p in &e.points {
            lo = [lo[0].min(p[0]), lo[1].min(p[1])];
            hi = [hi[0].max(p[0]), hi[1].max(p[1])];
        }
        let a = TileKey::containing([lo[0] - TILE_REACH_M, lo[1] - TILE_REACH_M], size);
        let b = TileKey::containing([hi[0] + TILE_REACH_M, hi[1] + TILE_REACH_M], size);
        for x in a.x..=b.x {
            for z in a.z..=b.z {
                out.entry(TileKey { x, z }).or_default().push(ei as u32);
            }
        }
    }
    out
}

/// The edge of each strip among `edges` (`None`: unmatched): the edge whose
/// polyline passes within [`MATCH_M`] of the strip's middle (its points lie
/// on the edge's polyline).
pub fn match_strips(
    g: &RoadGraph,
    edges: &[u32],
    strips: &[crate::package::RoadStrip],
) -> Vec<Option<usize>> {
    let k = |x: f64| (x / CELL_M).floor() as i32;
    let mut grid: std::collections::HashMap<(i32, i32), Vec<(u32, u32)>> = Default::default();
    for &ei in edges {
        let e = &g.edges[ei as usize];
        for (si, w) in e.points.windows(2).enumerate() {
            let (lo, hi) = (
                [w[0][0].min(w[1][0]), w[0][1].min(w[1][1])],
                [w[0][0].max(w[1][0]), w[0][1].max(w[1][1])],
            );
            for x in k(lo[0])..=k(hi[0]) {
                for z in k(lo[1])..=k(hi[1]) {
                    grid.entry((x, z)).or_default().push((ei, si as u32));
                }
            }
        }
    }
    let dist = |p: [f64; 2], a: [f64; 2], b: [f64; 2]| {
        let ab = [b[0] - a[0], b[1] - a[1]];
        let l2 = ab[0] * ab[0] + ab[1] * ab[1];
        let t = if l2 > 0.0 {
            (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / l2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        (a[0] + ab[0] * t - p[0]).hypot(a[1] + ab[1] * t - p[1])
    };
    strips
        .iter()
        .map(|s| {
            let n = s.points.len();
            if n < 2 {
                return None;
            }
            let m = (n - 1) / 2;
            let mid = [
                f64::from(s.points[m][0] + s.points[m + 1][0]) / 2.0,
                f64::from(s.points[m][2] + s.points[m + 1][2]) / 2.0,
            ];
            let mut best: Option<(usize, f64)> = None;
            for &(ei, si) in grid.get(&(k(mid[0]), k(mid[1]))).into_iter().flatten() {
                let w = &g.edges[ei as usize].points[si as usize..si as usize + 2];
                let d = dist(mid, w[0], w[1]);
                if best.is_none_or(|b| d < b.1) {
                    best = Some((ei as usize, d));
                }
            }
            best.filter(|b| b.1 < MATCH_M).map(|b| b.0)
        })
        .collect()
}

/// The edge of every strip of `pkg`'s tiles, matched by geometry (what the
/// renderer did before `RDGE`), for the graph `g`.
pub fn strip_edges_of(pkg: &crate::package::WorldPackage, g: &RoadGraph) -> StripEdges {
    let by_tile = tile_edges(g, f64::from(pkg.tile_size_m));
    let tiles = pkg
        .tiles
        .iter()
        .map(|(key, t)| {
            let edges = by_tile.get(key).map_or(&[][..], Vec::as_slice);
            let m = match_strips(g, edges, &t.roads)
                .into_iter()
                .map(|e| e.map_or(NO_EDGE, |e| e as u32))
                .collect();
            (*key, m)
        })
        .collect();
    StripEdges {
        fingerprint: g.fingerprint(),
        tiles,
    }
}

pub fn write_junctions(i: &JunctionIndex, b: &mut Vec<u8>) {
    b.push(JUNCTIONS_VERSION);
    b.push(i.flags);
    b.extend(i.fingerprint.to_le_bytes());
    b.extend(i.vertex_count.to_le_bytes());
    b.extend((i.rules.len() as u32).to_le_bytes());
    for (v, rule) in &i.rules {
        let mut r = Vec::new();
        r.extend(v.to_le_bytes());
        match rule {
            StoredRule::Mitre => r.push(1),
            StoredRule::Member(first) => {
                r.push(3);
                r.extend(first.to_le_bytes());
            }
            StoredRule::Patch(p) => {
                r.push(if p.members.len() > 1 { 2 } else { 0 });
                r.push(p.arms.len() as u8);
                for a in &p.arms {
                    r.extend(a.edge.to_le_bytes());
                    r.push(u8::from(a.at_from));
                    r.extend(a.mouth_m.to_le_bytes());
                    r.extend(a.along.to_le_bytes());
                }
                r.extend((p.outline.len() as u16).to_le_bytes());
                for o in &p.outline {
                    r.extend(o.d[0].to_le_bytes());
                    r.extend(o.d[1].to_le_bytes());
                    r.push(o.mouth.unwrap_or(255));
                }
                if p.members.len() > 1 {
                    r.push(p.members.len() as u8);
                    for m in &p.members {
                        r.extend(m.to_le_bytes());
                    }
                    r.push(p.inner.len() as u8);
                    for e in &p.inner {
                        r.extend(e.to_le_bytes());
                    }
                }
            }
        }
        b.extend((r.len() as u16).to_le_bytes());
        b.extend(r);
    }
}

pub fn read_junctions(payload: &[u8]) -> Result<JunctionIndex, String> {
    let mut r = Bytes::new(payload);
    let version = r.u8()?;
    if version != JUNCTIONS_VERSION {
        return Err(format!("junctions layout {version} not supported"));
    }
    let flags = r.u8()?;
    let fingerprint = r.u64()?;
    let vertex_count = r.u32()?;
    let count = r.u32()? as usize;
    let mut rules = Vec::with_capacity(count.min(payload.len() / 6 + 1));
    let mut last: Option<u32> = None;
    for _ in 0..count {
        let len = u16::from_le_bytes(r.arr()?) as usize;
        let mut q = Bytes::new(r.take(len)?);
        let v = q.u32()?;
        if last.is_some_and(|l| v <= l) {
            return Err("junction records out of order".into());
        }
        last = Some(v);
        let rule = match q.u8()? {
            1 => StoredRule::Mitre,
            3 => StoredRule::Member(q.u32()?),
            kind @ (0 | 2) => {
                let n = q.u8()? as usize;
                let mut arms = Vec::with_capacity(n);
                for _ in 0..n {
                    arms.push(StoredArm {
                        edge: q.u32()?,
                        at_from: q.u8()? & 1 != 0,
                        mouth_m: q.f64()?,
                        along: q.f64()?,
                    });
                }
                let m = u16::from_le_bytes(q.arr()?) as usize;
                let mut outline = Vec::with_capacity(m);
                for _ in 0..m {
                    let d = [q.f32()?, q.f32()?];
                    let mouth = match q.u8()? {
                        255 => None,
                        k if (k as usize) < n => Some(k),
                        _ => return Err("junction outline names no arm".into()),
                    };
                    outline.push(StoredPoint { d, mouth });
                }
                let (mut members, mut inner) = (Vec::new(), Vec::new());
                if kind == 2 {
                    for _ in 0..q.u8()? {
                        members.push(q.u32()?);
                    }
                    for _ in 0..q.u8()? {
                        inner.push(q.u32()?);
                    }
                    if members.first() != Some(&v) {
                        return Err("junction cluster without its vertex".into());
                    }
                }
                StoredRule::Patch(StoredPatch {
                    arms,
                    outline,
                    members,
                    inner,
                })
            }
            // A kind from a newer builder: no surface here.
            _ => continue,
        };
        // Fields of newer builders after these are skipped.
        rules.push((v, rule));
    }
    r.finish("junctions section")?;
    Ok(JunctionIndex {
        flags,
        fingerprint,
        vertex_count,
        rules,
    })
}

pub fn write_strip_edges(s: &StripEdges, b: &mut Vec<u8>) {
    b.push(STRIP_EDGES_VERSION);
    b.extend(s.fingerprint.to_le_bytes());
    b.extend((s.tiles.len() as u32).to_le_bytes());
    for (key, edges) in &s.tiles {
        b.extend(key.x.to_le_bytes());
        b.extend(key.z.to_le_bytes());
        b.extend((edges.len() as u32).to_le_bytes());
        for e in edges {
            b.extend(e.to_le_bytes());
        }
    }
}

pub fn read_strip_edges(payload: &[u8]) -> Result<StripEdges, String> {
    let mut r = Bytes::new(payload);
    let version = r.u8()?;
    if version != STRIP_EDGES_VERSION {
        return Err(format!("strip edges layout {version} not supported"));
    }
    let fingerprint = r.u64()?;
    let n = r.u32()? as usize;
    let mut tiles = BTreeMap::new();
    for _ in 0..n {
        let key = TileKey {
            x: r.i32()?,
            z: r.i32()?,
        };
        let m = r.u32()? as usize;
        let mut edges = Vec::with_capacity(m.min(payload.len() / 4));
        for _ in 0..m {
            edges.push(r.u32()?);
        }
        tiles.insert(key, edges);
    }
    r.finish("strip edges section")?;
    Ok(StripEdges { fingerprint, tiles })
}

#[cfg(test)]
mod tests;
