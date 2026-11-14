//! Bridges (spec §4.8.6): the structures the world builder plans for bridge
//! stretches of the road graph – OSM `bridge=*` ways, roads inside
//! `man_made=bridge` outlines and detected crossings of water or deep gaps
//! – and their geometry (deck, parapets or railings, piers, abutments,
//! arches, truss sides, a roof on covered bridges). Pure data and
//! geometry; the client colours the parts by material (`ocw-render`).
//!
//! Section `BRDG` (optional, readers that do not know it skip it):
//!
//! ```text
//! layout version: u32 (1)
//! tile count: u32, then per tile:
//!   tile x, z: i32 i32   bridge count: u32, then per bridge:
//!     edge id: u64   osm way: i64
//!     structure, material, source, railing: u8 × 4
//!     width, deck thickness: f32 f32
//!     point count: u32, points (deck top centreline): [f32; 3]
//!     abutment foot at start and end: f32 f32
//!     pier count: u32, per pier: along (m), foot y: f32 f32
//!     arch count: u32, per arch: from, to (m), spring y, crown y: f32 × 4
//! ```

use std::collections::BTreeMap;

use crate::graph::EdgeId;
use crate::package::{Bytes, TileKey};

pub const BRIDGES_TAG: &[u8; 4] = b"BRDG";
const BRIDGES_VERSION: u32 = 1;

/// How a bridge carries its deck.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BridgeStructure {
    /// Beam or slab on piers (most concrete and steel road bridges).
    Beam,
    /// Masonry or concrete arches under the deck.
    Arch,
    /// A steel or wooden truss on both sides.
    Truss,
    /// A suspension or cable-stayed bridge: towers and cables.
    Suspension,
    /// A covered (roofed) wooden bridge.
    Covered,
}

/// What a bridge is built of (OSM `bridge:material`, else by structure and
/// road class).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BridgeMaterial {
    Concrete,
    Stone,
    Steel,
    Wood,
}

/// Why the builder made a bridge here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BridgeSource {
    /// OSM `bridge=*` on the way.
    Tagged,
    /// Inside a `man_made=bridge` outline.
    Outline,
    /// Detected: an untagged road over water.
    Water,
    /// Detected: an untagged road high over a gap or valley.
    Gap,
}

/// The edges of the deck.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Railing {
    /// Solid parapet walls (stone, concrete).
    Parapet,
    /// Posts and rails (steel, wood; paths and cycle bridges).
    Rail,
}

macro_rules! codes {
    ($t:ty, [$($v:ident),*]) => {
        impl $t {
            pub const ALL: &'static [$t] = &[$(<$t>::$v),*];
            pub fn code(self) -> u8 {
                Self::ALL.iter().position(|v| *v == self).expect("listed") as u8
            }
            pub fn from_code(c: u8) -> Option<Self> {
                Self::ALL.get(usize::from(c)).copied()
            }
        }
    };
}
codes!(BridgeStructure, [Beam, Arch, Truss, Suspension, Covered]);
codes!(BridgeMaterial, [Concrete, Stone, Steel, Wood]);
codes!(BridgeSource, [Tagged, Outline, Water, Gap]);
codes!(Railing, [Parapet, Rail]);

/// A pier under the deck.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pier {
    /// Metres along the deck from its start.
    pub along_m: f32,
    /// Height of its foot (the ground or river bed).
    pub foot_y: f32,
}

/// One arch under the deck, between two supports.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Arch {
    pub from_m: f32,
    pub to_m: f32,
    /// Height of the arch at its ends (springing) and its top (crown).
    pub spring_y: f32,
    pub crown_y: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Bridge {
    /// The (first) road edge the bridge carries.
    pub edge: EdgeId,
    pub osm_way: i64,
    pub structure: BridgeStructure,
    pub material: BridgeMaterial,
    pub source: BridgeSource,
    pub railing: Railing,
    /// Width of the carriageway on it (the deck is wider by its kerbs).
    pub width_m: f32,
    /// Deck thickness under the road surface.
    pub deck_m: f32,
    /// Deck top centreline (the road surface), package metres.
    pub points: Vec<[f32; 3]>,
    /// Foot heights of the abutments at the start and the end.
    pub abutment_foot_y: [f32; 2],
    pub piers: Vec<Pier>,
    pub arches: Vec<Arch>,
}

impl Bridge {
    /// Length along the deck (horizontal).
    pub fn length_m(&self) -> f32 {
        self.points
            .windows(2)
            .map(|w| (w[1][0] - w[0][0]).hypot(w[1][2] - w[0][2]))
            .sum()
    }

    /// Deck top centre at `along` metres (clamped), and the horizontal
    /// direction there.
    pub fn at(&self, along: f32) -> ([f32; 3], [f32; 2]) {
        let mut walked = 0.0;
        for w in self.points.windows(2) {
            let (a, b) = (w[0], w[1]);
            let len = (b[0] - a[0]).hypot(b[2] - a[2]);
            if len < 1e-6 {
                continue;
            }
            if walked + len >= along {
                let t = ((along - walked) / len).clamp(0.0, 1.0);
                let p = [
                    a[0] + (b[0] - a[0]) * t,
                    a[1] + (b[1] - a[1]) * t,
                    a[2] + (b[2] - a[2]) * t,
                ];
                return (p, [(b[0] - a[0]) / len, (b[2] - a[2]) / len]);
            }
            walked += len;
        }
        let n = self.points.len();
        let (a, b) = (self.points[n.saturating_sub(2)], self.points[n - 1]);
        let len = (b[0] - a[0]).hypot(b[2] - a[2]).max(1e-6);
        (b, [(b[0] - a[0]) / len, (b[2] - a[2]) / len])
    }

    /// Distance of `(x, z)` from the deck centreline in the horizontal and
    /// the deck height at its foot point.
    pub fn deck_near(&self, x: f32, z: f32) -> Option<(f32, f32)> {
        let mut best: Option<(f32, f32)> = None;
        for w in self.points.windows(2) {
            let (a, b) = (w[0], w[1]);
            let (dx, dz) = (b[0] - a[0], b[2] - a[2]);
            let l2 = dx * dx + dz * dz;
            let t = if l2 > 0.0 {
                (((x - a[0]) * dx + (z - a[2]) * dz) / l2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let d = (x - a[0] - dx * t).hypot(z - a[2] - dz * t);
            if best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, a[1] + (b[1] - a[1]) * t));
            }
        }
        best
    }
}

/// Bridges by tile (the tile of the deck's middle point).
pub type Bridges = BTreeMap<TileKey, Vec<Bridge>>;

pub fn write_bridges(t: &Bridges, b: &mut Vec<u8>) {
    b.extend(BRIDGES_VERSION.to_le_bytes());
    b.extend((t.len() as u32).to_le_bytes());
    for (key, list) in t {
        b.extend(key.x.to_le_bytes());
        b.extend(key.z.to_le_bytes());
        b.extend((list.len() as u32).to_le_bytes());
        for br in list {
            b.extend(br.edge.0.to_le_bytes());
            b.extend(br.osm_way.to_le_bytes());
            b.extend([
                br.structure.code(),
                br.material.code(),
                br.source.code(),
                br.railing.code(),
            ]);
            b.extend(br.width_m.to_le_bytes());
            b.extend(br.deck_m.to_le_bytes());
            b.extend((br.points.len() as u32).to_le_bytes());
            for p in &br.points {
                for c in p {
                    b.extend(c.to_le_bytes());
                }
            }
            for y in br.abutment_foot_y {
                b.extend(y.to_le_bytes());
            }
            b.extend((br.piers.len() as u32).to_le_bytes());
            for p in &br.piers {
                b.extend(p.along_m.to_le_bytes());
                b.extend(p.foot_y.to_le_bytes());
            }
            b.extend((br.arches.len() as u32).to_le_bytes());
            for a in &br.arches {
                for v in [a.from_m, a.to_m, a.spring_y, a.crown_y] {
                    b.extend(v.to_le_bytes());
                }
            }
        }
    }
}

pub fn read_bridges(bytes: &[u8]) -> Result<Bridges, String> {
    let mut r = Bytes::new(bytes);
    let version = r.u32()?;
    if version != BRIDGES_VERSION {
        return Err(format!("bridges layout {version} is not supported"));
    }
    let mut out = Bridges::new();
    for _ in 0..r.u32()? {
        let key = TileKey {
            x: r.i32()?,
            z: r.i32()?,
        };
        let n = r.u32()?;
        let mut list = Vec::with_capacity(n.min(1 << 12) as usize);
        for _ in 0..n {
            let edge = EdgeId(r.u64()?);
            let osm_way = r.i64()?;
            let codes: [u8; 4] = r.arr()?;
            let bad = |what: &str, c: u8| format!("unknown bridge {what} {c}");
            let structure =
                BridgeStructure::from_code(codes[0]).ok_or_else(|| bad("structure", codes[0]))?;
            let material =
                BridgeMaterial::from_code(codes[1]).ok_or_else(|| bad("material", codes[1]))?;
            let source =
                BridgeSource::from_code(codes[2]).ok_or_else(|| bad("source", codes[2]))?;
            let railing = Railing::from_code(codes[3]).ok_or_else(|| bad("railing", codes[3]))?;
            let (width_m, deck_m) = (r.f32()?, r.f32()?);
            let np = r.u32()?;
            let points = (0..np)
                .map(|_| Ok([r.f32()?, r.f32()?, r.f32()?]))
                .collect::<Result<Vec<_>, String>>()?;
            if points.len() < 2 {
                return Err("bridge with fewer than two points".into());
            }
            let abutment_foot_y = [r.f32()?, r.f32()?];
            let piers = (0..r.u32()?)
                .map(|_| {
                    Ok(Pier {
                        along_m: r.f32()?,
                        foot_y: r.f32()?,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            let arches = (0..r.u32()?)
                .map(|_| {
                    Ok(Arch {
                        from_m: r.f32()?,
                        to_m: r.f32()?,
                        spring_y: r.f32()?,
                        crown_y: r.f32()?,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            list.push(Bridge {
                edge,
                osm_way,
                structure,
                material,
                source,
                railing,
                width_m,
                deck_m,
                points,
                abutment_foot_y,
                piers,
                arches,
            });
        }
        out.insert(key, list);
    }
    r.finish("bridges section")?;
    Ok(out)
}

// --- Geometry ---------------------------------------------------------------

/// The parts of a bridge, for colours and materials (`ocw-render`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BridgePart {
    /// Deck top beside the road, kerbs, sides and underside.
    Deck,
    /// Parapet walls, rails and posts.
    Railing,
    Pier,
    Abutment,
    /// Arch spandrels and the arch underside.
    Arch,
    /// Truss members, towers and cables.
    Frame,
    Roof,
}

/// Triangles of a bridge: per-vertex position, normal and part.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BridgeMesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub parts: Vec<BridgePart>,
    pub indices: Vec<u32>,
}

/// Kerb beyond the carriageway on each side of the deck (m).
pub const KERB_M: f32 = 0.6;
/// The deck top lies this far below the road strip (which is drawn 2 cm
/// above the road height), so the strip stays visible on it.
pub const DECK_TOP_BELOW_M: f32 = 0.01;
pub const PARAPET_H_M: f32 = 0.9;
pub const PARAPET_T_M: f32 = 0.35;
pub const RAIL_H_M: f32 = 1.1;
/// Piers and abutments reach this far into the ground.
pub const FOUNDATION_M: f32 = 0.6;

type V3 = [f32; 3];

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn scale(a: V3, s: f32) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn norm(a: V3) -> V3 {
    let l = dot(a, a).sqrt();
    if l < 1e-9 {
        [0.0, 1.0, 0.0]
    } else {
        scale(a, 1.0 / l)
    }
}

impl BridgeMesh {
    /// A quad (a, b, c, d around its edge) facing `normal` (flat shaded).
    fn quad(&mut self, q: [V3; 4], normal: V3, part: BridgePart) {
        let base = self.positions.len() as u32;
        let facing = dot(cross(sub(q[1], q[0]), sub(q[2], q[0])), normal) >= 0.0;
        for p in q {
            self.positions.push(p);
            self.normals.push(normal);
            self.parts.push(part);
        }
        if facing {
            self.indices
                .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        } else {
            self.indices
                .extend([base, base + 2, base + 1, base, base + 3, base + 2]);
        }
    }

    /// A box between two points: a beam of `w` × `h` (across × up) whose
    /// axis runs from `a` to `b` (both on its centreline).
    pub fn beam(&mut self, a: V3, b: V3, w: f32, h: f32, part: BridgePart) {
        let axis = norm(sub(b, a));
        let up0 = if axis[1].abs() > 0.95 {
            [1.0, 0.0, 0.0]
        } else {
            [0.0, 1.0, 0.0]
        };
        let side = norm(cross(axis, up0));
        let up = norm(cross(side, axis));
        let (s, u) = (scale(side, w / 2.0), scale(up, h / 2.0));
        let corner = |p: V3, i: f32, j: f32| add(add(p, scale(s, i)), scale(u, j));
        let faces: [(f32, f32, f32, f32, V3); 4] = [
            (1.0, -1.0, 1.0, 1.0, side),
            (-1.0, -1.0, -1.0, 1.0, scale(side, -1.0)),
            (-1.0, 1.0, 1.0, 1.0, up),
            (-1.0, -1.0, 1.0, -1.0, scale(up, -1.0)),
        ];
        for (i0, j0, i1, j1, n) in faces {
            self.quad(
                [
                    corner(a, i0, j0),
                    corner(b, i0, j0),
                    corner(b, i1, j1),
                    corner(a, i1, j1),
                ],
                n,
                part,
            );
        }
        for (p, n) in [(a, scale(axis, -1.0)), (b, axis)] {
            self.quad(
                [
                    corner(p, -1.0, -1.0),
                    corner(p, 1.0, -1.0),
                    corner(p, 1.0, 1.0),
                    corner(p, -1.0, 1.0),
                ],
                n,
                part,
            );
        }
    }

    /// An upright box: footprint corners (x, z) from `bottom` to `top`.
    fn block(&mut self, corners: [[f32; 2]; 4], bottom: f32, top: f32, part: BridgePart) {
        let c: Vec<[f32; 2]> = corners.to_vec();
        let mid = [
            c.iter().map(|p| p[0]).sum::<f32>() / 4.0,
            c.iter().map(|p| p[1]).sum::<f32>() / 4.0,
        ];
        for i in 0..4 {
            let (p, q) = (c[i], c[(i + 1) % 4]);
            let e = [q[0] - p[0], 0.0, q[1] - p[1]];
            let mut n = norm([e[2], 0.0, -e[0]]);
            let out = [
                (p[0] + q[0]) / 2.0 - mid[0],
                0.0,
                (p[1] + q[1]) / 2.0 - mid[1],
            ];
            if dot(n, out) < 0.0 {
                n = scale(n, -1.0);
            }
            self.quad(
                [
                    [p[0], bottom, p[1]],
                    [q[0], bottom, q[1]],
                    [q[0], top, q[1]],
                    [p[0], top, p[1]],
                ],
                n,
                part,
            );
        }
        let ring = |y: f32| c.iter().map(|p| [p[0], y, p[1]]).collect::<Vec<_>>();
        let (t, b) = (ring(top), ring(bottom));
        self.quad([t[0], t[1], t[2], t[3]], [0.0, 1.0, 0.0], part);
        self.quad([b[0], b[1], b[2], b[3]], [0.0, -1.0, 0.0], part);
    }
}

/// Cross-section frame at a deck point: centre, right (horizontal, unit).
pub(crate) fn frame(points: &[[f32; 3]], i: usize) -> (V3, V3) {
    let n = points.len();
    let (a, b) = (points[i.saturating_sub(1)], points[(i + 1).min(n - 1)]);
    let d = norm([b[0] - a[0], 0.0, b[2] - a[2]]);
    (points[i], [-d[2], 0.0, d[0]])
}

/// Deck heights of the bottom of the deck along the bridge.
fn deck_bottom(b: &Bridge, along: f32) -> f32 {
    b.at(along).0[1] - DECK_TOP_BELOW_M - b.deck_m
}

/// The geometry of a bridge (see [`BridgePart`] for its pieces).
pub fn bridge_mesh(b: &Bridge) -> BridgeMesh {
    let mut m = BridgeMesh::default();
    let n = b.points.len();
    if n < 2 {
        return m;
    }
    let half = b.width_m / 2.0 + KERB_M;
    let up = [0.0, 1.0, 0.0];
    // Deck: top (kerbs and under the road), sides, underside, end caps.
    let mut along = vec![0.0f32; n];
    for i in 1..n {
        let (p, q) = (b.points[i - 1], b.points[i]);
        along[i] = along[i - 1] + (q[0] - p[0]).hypot(q[2] - p[2]);
    }
    let edge = |i: usize, side: f32, dy: f32| {
        let (c, r) = frame(&b.points, i);
        add(
            add(c, scale(r, side * half)),
            [0.0, dy - DECK_TOP_BELOW_M, 0.0],
        )
    };
    for i in 0..n - 1 {
        let (top, bot) = (0.0, -b.deck_m);
        m.quad(
            [
                edge(i, -1.0, top),
                edge(i, 1.0, top),
                edge(i + 1, 1.0, top),
                edge(i + 1, -1.0, top),
            ],
            up,
            BridgePart::Deck,
        );
        m.quad(
            [
                edge(i, -1.0, bot),
                edge(i, 1.0, bot),
                edge(i + 1, 1.0, bot),
                edge(i + 1, -1.0, bot),
            ],
            [0.0, -1.0, 0.0],
            BridgePart::Deck,
        );
        for side in [-1.0f32, 1.0] {
            let (_, r) = frame(&b.points, i);
            m.quad(
                [
                    edge(i, side, top),
                    edge(i + 1, side, top),
                    edge(i + 1, side, bot),
                    edge(i, side, bot),
                ],
                scale(r, side),
                BridgePart::Deck,
            );
        }
    }
    for (i, s) in [(0usize, -1.0f32), (n - 1, 1.0)] {
        let (c0, c1) = (
            b.points[i.saturating_sub(1).min(n - 2)],
            b.points[(i + 1).min(n - 1).max(1)],
        );
        let dir = norm([c1[0] - c0[0], 0.0, c1[2] - c0[2]]);
        m.quad(
            [
                edge(i, -1.0, 0.0),
                edge(i, 1.0, 0.0),
                edge(i, 1.0, -b.deck_m),
                edge(i, -1.0, -b.deck_m),
            ],
            scale(dir, s),
            BridgePart::Deck,
        );
    }
    // Parapets or railings on both edges.
    for side in [-1.0f32, 1.0] {
        let at = |i: usize, inset: f32, dy: f32| {
            let (c, r) = frame(&b.points, i);
            add(
                add(c, scale(r, side * (half - inset))),
                [0.0, dy - DECK_TOP_BELOW_M, 0.0],
            )
        };
        match b.railing {
            Railing::Parapet => {
                for i in 0..n - 1 {
                    let (a, c) = (
                        at(i, PARAPET_T_M / 2.0, PARAPET_H_M / 2.0),
                        at(i + 1, PARAPET_T_M / 2.0, PARAPET_H_M / 2.0),
                    );
                    m.beam(a, c, PARAPET_T_M, PARAPET_H_M, BridgePart::Railing);
                }
            }
            Railing::Rail => {
                for i in 0..n - 1 {
                    for h in [RAIL_H_M, RAIL_H_M * 0.5] {
                        m.beam(
                            at(i, 0.1, h),
                            at(i + 1, 0.1, h),
                            0.07,
                            0.07,
                            BridgePart::Railing,
                        );
                    }
                }
                let len = along[n - 1];
                let posts = (len / 2.0).ceil().max(1.0) as usize;
                for k in 0..=posts {
                    let s = len * k as f32 / posts as f32;
                    let (c, d) = b.at(s);
                    let r = [-d[1], 0.0, d[0]];
                    let foot = add(
                        add(c, scale(r, side * (half - 0.1))),
                        [0.0, -DECK_TOP_BELOW_M, 0.0],
                    );
                    m.beam(
                        foot,
                        add(foot, [0.0, RAIL_H_M, 0.0]),
                        0.08,
                        0.08,
                        BridgePart::Railing,
                    );
                }
            }
        }
    }
    // Abutments: blocks at both ends, from the deck bottom into the ground.
    let len = along[n - 1];
    for (k, s) in [(0usize, 0.0f32), (1, len)] {
        let (c, d) = b.at(s);
        let r = [-d[1], d[0]];
        let into = if k == 0 { -1.0 } else { 1.0 };
        let (l0, l1) = (s - into * 1.2, s + into * 1.0);
        let p = |along_off: f32, side: f32| {
            let t = along_off - s;
            [
                c[0] + d[0] * t + r[0] * side * (half + 0.3),
                c[2] + d[1] * t + r[1] * side * (half + 0.3),
            ]
        };
        let top = c[1] - DECK_TOP_BELOW_M - b.deck_m;
        let foot = b.abutment_foot_y[k] - FOUNDATION_M;
        if top - foot > 0.1 {
            m.block(
                [p(l0, -1.0), p(l1, -1.0), p(l1, 1.0), p(l0, 1.0)],
                foot,
                top,
                BridgePart::Abutment,
            );
        }
    }
    // Piers.
    for pier in &b.piers {
        let (c, d) = b.at(pier.along_m);
        let r = [-d[1], d[0]];
        let (hl, hw) = (0.7f32, (half * 0.75).max(1.0));
        let p = |a: f32, w: f32| [c[0] + d[0] * a + r[0] * w, c[2] + d[1] * a + r[1] * w];
        let top = deck_bottom(b, pier.along_m);
        let foot = pier.foot_y - FOUNDATION_M;
        if top - foot > 0.1 {
            m.block(
                [p(-hl, -hw), p(hl, -hw), p(hl, hw), p(-hl, hw)],
                foot,
                top,
                BridgePart::Pier,
            );
        }
    }
    // Arches: spandrel walls on both faces and the arch underside.
    for a in &b.arches {
        let span = (a.to_m - a.from_m).max(0.1);
        let steps = ((span / 1.0).ceil() as usize).clamp(8, 48);
        let curve = |u: f32| {
            a.spring_y + (a.crown_y - a.spring_y) * (1.0 - (2.0 * u - 1.0).powi(2)).max(0.0).sqrt()
        };
        for j in 0..steps {
            let (u0, u1) = (j as f32 / steps as f32, (j + 1) as f32 / steps as f32);
            let (s0, s1) = (a.from_m + span * u0, a.from_m + span * u1);
            let ((c0, d0), (c1, d1)) = (b.at(s0), b.at(s1));
            let (r0, r1) = ([-d0[1], 0.0, d0[0]], [-d1[1], 0.0, d1[0]]);
            let (y0, y1) = (curve(u0), curve(u1));
            let (t0, t1) = (deck_bottom(b, s0), deck_bottom(b, s1));
            for side in [-1.0f32, 1.0] {
                let q = |c: V3, r: V3, y: f32| {
                    let h = add(c, scale(r, side * half));
                    [h[0], y, h[2]]
                };
                if t0 > y0 + 0.01 || t1 > y1 + 0.01 {
                    m.quad(
                        [q(c0, r0, y0), q(c1, r1, y1), q(c1, r1, t1), q(c0, r0, t0)],
                        scale(r0, side),
                        BridgePart::Arch,
                    );
                }
            }
            // Underside of the arch, facing down and in.
            let w = |c: V3, r: V3, side: f32, y: f32| {
                let h = add(c, scale(r, side * half));
                [h[0], y, h[2]]
            };
            let tangent = norm([c1[0] - c0[0], y1 - y0, c1[2] - c0[2]]);
            let mut nrm = norm(cross(r0, tangent));
            if nrm[1] > 0.0 {
                nrm = scale(nrm, -1.0);
            }
            m.quad(
                [
                    w(c0, r0, -1.0, y0),
                    w(c0, r0, 1.0, y0),
                    w(c1, r1, 1.0, y1),
                    w(c1, r1, -1.0, y1),
                ],
                nrm,
                BridgePart::Arch,
            );
        }
    }
    // Structure above the deck.
    match b.structure {
        BridgeStructure::Truss => {
            let h = (len / 8.0).clamp(2.5, 7.0);
            let bays = (len / h).round().max(1.0) as usize;
            for side in [-1.0f32, 1.0] {
                let node = |k: usize, top: bool| {
                    let (c, d) = b.at(len * k as f32 / bays as f32);
                    let r = [-d[1], 0.0, d[0]];
                    add(
                        add(c, scale(r, side * (half - 0.15))),
                        [0.0, if top { h } else { 0.0 }, 0.0],
                    )
                };
                for k in 0..bays {
                    m.beam(
                        node(k, true),
                        node(k + 1, true),
                        0.3,
                        0.3,
                        BridgePart::Frame,
                    );
                    m.beam(
                        node(k, false),
                        node(k + 1, false),
                        0.3,
                        0.3,
                        BridgePart::Frame,
                    );
                    m.beam(node(k, false), node(k, true), 0.25, 0.25, BridgePart::Frame);
                    let (a, c) = if k % 2 == 0 {
                        (node(k, false), node(k + 1, true))
                    } else {
                        (node(k, true), node(k + 1, false))
                    };
                    m.beam(a, c, 0.2, 0.2, BridgePart::Frame);
                }
                m.beam(
                    node(bays, false),
                    node(bays, true),
                    0.25,
                    0.25,
                    BridgePart::Frame,
                );
            }
        }
        BridgeStructure::Suspension => {
            let tower = (len / 6.0).clamp(6.0, 30.0);
            let ends = [len * 0.08, len * 0.92];
            for side in [-1.0f32, 1.0] {
                let top_at = |s: f32| {
                    let (c, d) = b.at(s);
                    let r = [-d[1], 0.0, d[0]];
                    add(add(c, scale(r, side * (half + 0.2))), [0.0, tower, 0.0])
                };
                for &s in &ends {
                    let t = top_at(s);
                    m.beam(
                        [t[0], b.at(s).0[1] - b.deck_m, t[2]],
                        t,
                        0.6,
                        0.6,
                        BridgePart::Frame,
                    );
                }
                // The main cable sags to just above the deck in the middle.
                let segs = 16;
                let cable = |u: f32| {
                    let s = ends[0] + (ends[1] - ends[0]) * u;
                    let (c, d) = b.at(s);
                    let r = [-d[1], 0.0, d[0]];
                    let sag = tower * (1.0 - 4.0 * u * (1.0 - u)) + 1.0;
                    add(
                        add(c, scale(r, side * (half + 0.2))),
                        [0.0, sag.min(tower), 0.0],
                    )
                };
                for j in 0..segs {
                    let (u0, u1) = (j as f32 / segs as f32, (j + 1) as f32 / segs as f32);
                    m.beam(cable(u0), cable(u1), 0.12, 0.12, BridgePart::Frame);
                    let p = cable(u0);
                    let deck = b.at(ends[0] + (ends[1] - ends[0]) * u0).0[1];
                    if p[1] - deck > 0.5 {
                        m.beam([p[0], deck, p[2]], p, 0.04, 0.04, BridgePart::Frame);
                    }
                }
            }
        }
        BridgeStructure::Covered => {
            let wall = 2.6f32;
            let roof = 1.4f32;
            let posts = (len / 3.0).ceil().max(1.0) as usize;
            for side in [-1.0f32, 1.0] {
                for k in 0..=posts {
                    let (c, d) = b.at(len * k as f32 / posts as f32);
                    let r = [-d[1], 0.0, d[0]];
                    let foot = add(c, scale(r, side * (half - 0.15)));
                    m.beam(
                        foot,
                        add(foot, [0.0, wall, 0.0]),
                        0.2,
                        0.2,
                        BridgePart::Frame,
                    );
                }
                // Board walls from the deck to 1.2 m (open above).
                for i in 0..n - 1 {
                    let at = |i: usize| {
                        let (c, r) = frame(&b.points, i);
                        add(c, scale(r, side * (half - 0.15)))
                    };
                    let (p, q) = (at(i), at(i + 1));
                    m.beam(
                        add(p, [0.0, 0.6, 0.0]),
                        add(q, [0.0, 0.6, 0.0]),
                        0.08,
                        1.2,
                        BridgePart::Railing,
                    );
                }
            }
            // Gable roof.
            for i in 0..n - 1 {
                let (c0, r0) = frame(&b.points, i);
                let (c1, r1) = frame(&b.points, i + 1);
                let eave = |c: V3, r: V3, s: f32| {
                    add(add(c, scale(r, s * (half + 0.5))), [0.0, wall, 0.0])
                };
                let ridge = |c: V3| add(c, [0.0, wall + roof, 0.0]);
                for s in [-1.0f32, 1.0] {
                    let nrm = norm(add(scale(r0, s * roof), [0.0, half + 0.5, 0.0]));
                    m.quad(
                        [eave(c0, r0, s), eave(c1, r1, s), ridge(c1), ridge(c0)],
                        nrm,
                        BridgePart::Roof,
                    );
                    // Underside, so the roof is solid from below too.
                    m.quad(
                        [eave(c0, r0, s), eave(c1, r1, s), ridge(c1), ridge(c0)],
                        scale(nrm, -1.0),
                        BridgePart::Roof,
                    );
                }
            }
        }
        BridgeStructure::Beam | BridgeStructure::Arch => {}
    }
    m
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn bridge(structure: BridgeStructure, railing: Railing) -> Bridge {
        Bridge {
            edge: EdgeId(7),
            osm_way: 42,
            structure,
            material: BridgeMaterial::Stone,
            source: BridgeSource::Tagged,
            railing,
            width_m: 5.0,
            deck_m: 0.6,
            // 40 m along +x, rising 1 m, 8 m above ground at 100 m.
            points: (0..=8)
                .map(|i| [i as f32 * 5.0, 108.0 + i as f32 / 8.0, 0.0])
                .collect(),
            abutment_foot_y: [104.0, 104.5],
            piers: vec![Pier {
                along_m: 20.0,
                foot_y: 100.0,
            }],
            arches: vec![],
        }
    }

    #[test]
    fn section_round_trip_and_codes() {
        let mut t = Bridges::new();
        let mut a = bridge(BridgeStructure::Arch, Railing::Parapet);
        a.arches = vec![Arch {
            from_m: 0.0,
            to_m: 20.0,
            spring_y: 101.0,
            crown_y: 107.0,
        }];
        t.insert(
            TileKey { x: -1, z: 2 },
            vec![a, bridge(BridgeStructure::Covered, Railing::Rail)],
        );
        let mut b = Vec::new();
        write_bridges(&t, &mut b);
        assert_eq!(read_bridges(&b).unwrap(), t);
        assert!(read_bridges(&b[..b.len() - 1]).is_err());
        for s in BridgeStructure::ALL {
            assert_eq!(BridgeStructure::from_code(s.code()), Some(*s));
        }
        assert_eq!(BridgeMaterial::from_code(9), None);
    }

    fn bounds(m: &BridgeMesh, part: BridgePart) -> ([f32; 3], [f32; 3]) {
        let mut lo = [f32::INFINITY; 3];
        let mut hi = [f32::NEG_INFINITY; 3];
        for (p, q) in m.positions.iter().zip(&m.parts) {
            if *q == part {
                for k in 0..3 {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
            }
        }
        (lo, hi)
    }

    #[test]
    fn beam_bridge_has_deck_parapets_piers_and_abutments() {
        let b = bridge(BridgeStructure::Beam, Railing::Parapet);
        let m = bridge_mesh(&b);
        assert_eq!(m.positions.len(), m.normals.len());
        assert!(m.indices.iter().all(|&i| (i as usize) < m.positions.len()));
        assert_eq!(m.indices.len() % 3, 0);
        // The deck top lies just under the road surface, its bottom a deck
        // thickness lower, as wide as the road plus its kerbs.
        let (lo, hi) = bounds(&m, BridgePart::Deck);
        assert!((hi[1] - (109.0 - DECK_TOP_BELOW_M)).abs() < 1e-3, "{hi:?}");
        assert!(
            (lo[1] - (108.0 - DECK_TOP_BELOW_M - 0.6)).abs() < 1e-3,
            "{lo:?}"
        );
        assert!((hi[2] - (2.5 + KERB_M)).abs() < 1e-3 && (lo[2] + 2.5 + KERB_M).abs() < 1e-3);
        // Parapets stand on the deck edges.
        let (lo, hi) = bounds(&m, BridgePart::Railing);
        assert!(hi[1] > 108.8 + PARAPET_H_M && lo[1] < 108.2);
        assert!(hi[2] > 2.5 + KERB_M - 0.01);
        // The pier reaches from the deck into the ground, abutments too.
        let (lo, hi) = bounds(&m, BridgePart::Pier);
        assert!((lo[1] - (100.0 - FOUNDATION_M)).abs() < 1e-3);
        assert!((hi[1] - deck_bottom(&b, 20.0)).abs() < 1e-3);
        assert!((lo[0] - 19.3).abs() < 1e-3 && (hi[0] - 20.7).abs() < 1e-3);
        let (lo, _) = bounds(&m, BridgePart::Abutment);
        assert!((lo[1] - (104.0 - FOUNDATION_M)).abs() < 1e-3);
        // Flat-shaded quads: unit normals.
        for n in &m.normals {
            assert!((dot(*n, *n) - 1.0).abs() < 1e-3);
        }
    }

    #[test]
    fn faces_point_outwards() {
        // Every triangle's winding agrees with its stored normal.
        for s in BridgeStructure::ALL {
            for r in Railing::ALL {
                let mut b = bridge(*s, *r);
                if *s == BridgeStructure::Arch {
                    b.arches = vec![Arch {
                        from_m: 0.0,
                        to_m: 40.0,
                        spring_y: 102.0,
                        crown_y: 107.2,
                    }];
                }
                let m = bridge_mesh(&b);
                assert!(!m.indices.is_empty());
                for t in m.indices.chunks(3) {
                    let p = |i: u32| m.positions[i as usize];
                    let n = cross(sub(p(t[1]), p(t[0])), sub(p(t[2]), p(t[0])));
                    if dot(n, n) < 1e-10 {
                        continue;
                    }
                    assert!(dot(n, m.normals[t[0] as usize]) >= -1e-4, "{s:?} {r:?}");
                }
            }
        }
    }

    #[test]
    fn arches_span_between_springing_and_deck() {
        let mut b = bridge(BridgeStructure::Arch, Railing::Parapet);
        b.piers.clear();
        b.arches = vec![
            Arch {
                from_m: 0.0,
                to_m: 20.0,
                spring_y: 101.0,
                crown_y: 107.0,
            },
            Arch {
                from_m: 20.0,
                to_m: 40.0,
                spring_y: 101.0,
                crown_y: 107.0,
            },
        ];
        let m = bridge_mesh(&b);
        let (lo, hi) = bounds(&m, BridgePart::Arch);
        assert!((lo[1] - 101.0).abs() < 1e-3, "{lo:?}");
        assert!(hi[1] <= deck_bottom(&b, 40.0) + 1e-3);
        // The crown (middle of the first arch) is at its crown height.
        let crown = m
            .positions
            .iter()
            .zip(&m.parts)
            .filter(|(p, q)| **q == BridgePart::Arch && (p[0] - 10.0).abs() < 0.01)
            .map(|(p, _)| p[1])
            .fold(f32::INFINITY, f32::min);
        assert!((crown - 107.0).abs() < 1e-3, "{crown}");
    }

    #[test]
    fn deck_lookup_and_length() {
        let b = bridge(BridgeStructure::Beam, Railing::Rail);
        assert!((b.length_m() - 40.0).abs() < 1e-4);
        let (d, y) = b.deck_near(10.0, 1.5).unwrap();
        assert!((d - 1.5).abs() < 1e-5 && (y - 108.25).abs() < 1e-4);
        let (p, dir) = b.at(25.0);
        assert!((p[0] - 25.0).abs() < 1e-4 && (dir[0] - 1.0).abs() < 1e-5);
        assert_eq!(b.at(99.0).0, b.points[8]);
    }
}
