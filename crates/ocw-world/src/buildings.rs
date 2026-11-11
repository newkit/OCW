//! Buildings (spec §4.1, §4.7): OSM footprints extruded from the terrain,
//! stored per tile in the package section `BLDG`, and the triangulation of
//! their roofs (ear clipping with holes).
//!
//! Footprints are in local metres (x, z). The outer ring winds
//! counter-clockwise in the (x, z) plane (positive [`signed_area`]), holes
//! clockwise; rings are open (the first point is not repeated). With this
//! convention the outward normal of an edge `a → b` is `(b.z − a.z, a.x − b.x)`
//! for the outer ring and the holes alike.

use std::collections::BTreeMap;

use crate::package::{Bytes, TileKey};

/// Section tag of the buildings.
pub const BUILDINGS_TAG: &[u8; 4] = b"BLDG";
/// Layout version inside the buildings section.
const BUILDINGS_VERSION: u32 = 1;

/// What a building is used for (its colours). Codes are stored in
/// packages: only add new kinds at the end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum BuildingKind {
    #[default]
    Other = 0,
    House = 1,
    Apartments = 2,
    Commercial = 3,
    Industrial = 4,
    Farm = 5,
    Religious = 6,
    Public = 7,
    /// Garages, sheds, huts, carports.
    Small = 8,
}

impl BuildingKind {
    pub const ALL: [BuildingKind; 9] = [
        BuildingKind::Other,
        BuildingKind::House,
        BuildingKind::Apartments,
        BuildingKind::Commercial,
        BuildingKind::Industrial,
        BuildingKind::Farm,
        BuildingKind::Religious,
        BuildingKind::Public,
        BuildingKind::Small,
    ];

    pub fn from_code(code: u8) -> Option<Self> {
        Self::ALL.get(usize::from(code)).copied()
    }
}

/// Roof shape.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Roof {
    Flat,
    /// A gable roof over a rectangular footprint (exactly four points):
    /// the ridge runs parallel to the first edge, `height_m` above the eaves.
    Gabled {
        height_m: f32,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Building {
    pub kind: BuildingKind,
    /// Outer ring, counter-clockwise in (x, z).
    pub outline: Vec<[f32; 2]>,
    /// Inner rings (courtyards), clockwise.
    pub holes: Vec<Vec<[f32; 2]>>,
    /// Foot of the walls: a little below the lowest terrain under the footprint.
    pub base_y: f32,
    /// Top of the walls (the eaves, or the flat roof).
    pub eave_y: f32,
    pub roof: Roof,
}

/// Buildings of a package by tile (a building belongs to the tile of its
/// first outline point's centroid, see `world-builder`).
pub type Buildings = BTreeMap<TileKey, Vec<Building>>;

/// Twice the signed area of a ring in the (x, z) plane (positive:
/// counter-clockwise).
pub fn signed_area<T: Copy + Into<f64>>(ring: &[[T; 2]]) -> f64 {
    let n = ring.len();
    (0..n)
        .map(|i| {
            let (a, b) = (ring[i], ring[(i + 1) % n]);
            a[0].into() * b[1].into() - b[0].into() * a[1].into()
        })
        .sum::<f64>()
}

fn cross(o: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
}

fn in_triangle(p: [f64; 2], a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> bool {
    cross(a, b, p) >= 0.0 && cross(b, c, p) >= 0.0 && cross(c, a, p) >= 0.0
}

/// Whether segments `p1 p2` and `q1 q2` cross in their interiors.
fn segments_cross(p1: [f64; 2], p2: [f64; 2], q1: [f64; 2], q2: [f64; 2]) -> bool {
    let (d1, d2) = (cross(q1, q2, p1), cross(q1, q2, p2));
    let (d3, d4) = (cross(p1, p2, q1), cross(p1, p2, q2));
    ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
}

/// Triangulates a polygon with holes by ear clipping (holes are first
/// bridged into the outer ring). Vertex indices refer to the outer ring
/// followed by the holes in order. Triangles wind like the outer ring
/// (counter-clockwise in (x, z)). Rings may wind either way; degenerate
/// input yields fewer triangles, never a panic.
pub fn triangulate(outer: &[[f32; 2]], holes: &[Vec<[f32; 2]>]) -> Vec<[u32; 3]> {
    let mut pts: Vec<[f64; 2]> = outer.iter().map(|p| p.map(f64::from)).collect();
    let mut poly: Vec<u32> = (0..outer.len() as u32).collect();
    if signed_area(outer) < 0.0 {
        poly.reverse();
    }
    // Holes, rightmost first, each wound clockwise and bridged from its
    // rightmost vertex to the nearest visible vertex of the polygon so far.
    let mut hole_rings: Vec<Vec<u32>> = Vec::new();
    for h in holes {
        let start = pts.len() as u32;
        pts.extend(h.iter().map(|p| p.map(f64::from)));
        if h.len() < 3 {
            continue;
        }
        let mut ring: Vec<u32> = (start..start + h.len() as u32).collect();
        if signed_area(h) > 0.0 {
            ring.reverse();
        }
        hole_rings.push(ring);
    }
    let max_x = |r: &Vec<u32>| {
        r.iter()
            .map(|&i| pts[i as usize][0])
            .fold(f64::NEG_INFINITY, f64::max)
    };
    hole_rings.sort_by(|a, b| max_x(b).total_cmp(&max_x(a)));
    for (hi, ring) in hole_rings.iter().enumerate() {
        let m_pos = (0..ring.len())
            .max_by(|&a, &b| pts[ring[a] as usize][0].total_cmp(&pts[ring[b] as usize][0]))
            .expect("hole has points");
        let m = pts[ring[m_pos] as usize];
        // All edges that a bridge must not cross: the polygon and the holes
        // not yet bridged.
        let mut edges: Vec<(u32, u32)> = (0..poly.len())
            .map(|i| (poly[i], poly[(i + 1) % poly.len()]))
            .collect();
        for r in &hole_rings[hi..] {
            edges.extend((0..r.len()).map(|i| (r[i], r[(i + 1) % r.len()])));
        }
        let visible = |v: u32| {
            let p = pts[v as usize];
            edges.iter().all(|&(a, b)| {
                a == v
                    || b == v
                    || a == ring[m_pos]
                    || b == ring[m_pos]
                    || !segments_cross(m, p, pts[a as usize], pts[b as usize])
            })
        };
        let best = (0..poly.len())
            .filter(|&i| visible(poly[i]))
            .min_by(|&a, &b| {
                let d = |i: usize| {
                    let p = pts[poly[i] as usize];
                    (p[0] - m[0]).powi(2) + (p[1] - m[1]).powi(2)
                };
                d(a).total_cmp(&d(b))
            });
        let Some(at) = best else {
            continue; // no visible vertex: the hole is dropped
        };
        // poly[..=at], hole from m around back to m, poly[at], poly[at+1..]
        let mut spliced = Vec::with_capacity(poly.len() + ring.len() + 2);
        spliced.extend_from_slice(&poly[..=at]);
        for j in 0..=ring.len() {
            spliced.push(ring[(m_pos + j) % ring.len()]);
        }
        spliced.push(poly[at]);
        spliced.extend_from_slice(&poly[at + 1..]);
        poly = spliced;
    }
    // Ear clipping.
    let mut tris = Vec::new();
    let mut guard = 0;
    while poly.len() > 3 && guard < poly.len() * poly.len() + 16 {
        guard += 1;
        let n = poly.len();
        let mut clipped = false;
        for i in 0..n {
            let (ia, ib, ic) = (poly[(i + n - 1) % n], poly[i], poly[(i + 1) % n]);
            let (a, b, c) = (pts[ia as usize], pts[ib as usize], pts[ic as usize]);
            let turn = cross(a, b, c);
            if turn < 0.0 {
                continue; // reflex
            }
            if turn == 0.0 {
                // Collinear (or a bridge's doubled vertex): drop it.
                poly.remove(i);
                clipped = true;
                break;
            }
            let blocked = poly.iter().any(|&j| {
                let p = pts[j as usize];
                p != a && p != b && p != c && in_triangle(p, a, b, c)
            });
            if !blocked {
                tris.push([ia, ib, ic]);
                poly.remove(i);
                clipped = true;
                break;
            }
        }
        if !clipped {
            break; // self-intersecting input: give up on the rest
        }
    }
    if poly.len() == 3 {
        let (a, b, c) = (
            pts[poly[0] as usize],
            pts[poly[1] as usize],
            pts[poly[2] as usize],
        );
        if cross(a, b, c) > 0.0 {
            tris.push([poly[0], poly[1], poly[2]]);
        }
    }
    tris
}

/// Writes the buildings section payload:
///
/// ```text
/// layout version: u32 (1)
/// tile count: u32, then per tile:
///   tile x, z: i32 i32   building count: u32, then per building:
///     kind: u8   roof: u8 (0 flat, 1 gabled)   [roof height: f32]
///     base y, eave y: f32 f32
///     ring count: u16 (outline first), per ring: point count: u32, points: [f32; 2]
/// ```
pub fn write_buildings(t: &Buildings, b: &mut Vec<u8>) {
    b.extend(BUILDINGS_VERSION.to_le_bytes());
    b.extend((t.len() as u32).to_le_bytes());
    for (key, list) in t {
        b.extend(key.x.to_le_bytes());
        b.extend(key.z.to_le_bytes());
        b.extend((list.len() as u32).to_le_bytes());
        for bl in list {
            b.push(bl.kind as u8);
            match bl.roof {
                Roof::Flat => b.push(0),
                Roof::Gabled { height_m } => {
                    b.push(1);
                    b.extend(height_m.to_le_bytes());
                }
            }
            b.extend(bl.base_y.to_le_bytes());
            b.extend(bl.eave_y.to_le_bytes());
            let rings = bl.holes.len().min(usize::from(u16::MAX) - 1) + 1;
            b.extend((rings as u16).to_le_bytes());
            for ring in std::iter::once(&bl.outline).chain(&bl.holes).take(rings) {
                b.extend((ring.len() as u32).to_le_bytes());
                for p in ring {
                    b.extend(p[0].to_le_bytes());
                    b.extend(p[1].to_le_bytes());
                }
            }
        }
    }
}

/// Reads a buildings section payload written by [`write_buildings`].
pub fn read_buildings(bytes: &[u8]) -> Result<Buildings, String> {
    let mut r = Bytes::new(bytes);
    let version = r.u32()?;
    if version != BUILDINGS_VERSION {
        return Err(format!("buildings layout {version} is not supported"));
    }
    let mut out = Buildings::new();
    for _ in 0..r.u32()? {
        let key = TileKey {
            x: r.i32()?,
            z: r.i32()?,
        };
        let n = r.u32()?;
        let mut list = Vec::with_capacity(n.min(1 << 16) as usize);
        for _ in 0..n {
            let code = r.u8()?;
            let kind =
                BuildingKind::from_code(code).ok_or(format!("unknown building kind {code}"))?;
            let roof = match r.u8()? {
                0 => Roof::Flat,
                1 => Roof::Gabled { height_m: r.f32()? },
                x => return Err(format!("unknown roof shape {x}")),
            };
            let (base_y, eave_y) = (r.f32()?, r.f32()?);
            let rings = u16::from_le_bytes(r.arr()?);
            if rings == 0 {
                return Err("building without an outline".into());
            }
            let mut ring_list = Vec::with_capacity(usize::from(rings));
            for _ in 0..rings {
                let np = r.u32()?;
                let ring = (0..np)
                    .map(|_| Ok([r.f32()?, r.f32()?]))
                    .collect::<Result<Vec<_>, String>>()?;
                if ring.len() < 3 {
                    return Err("building ring with fewer than three points".into());
                }
                ring_list.push(ring);
            }
            let outline = ring_list.remove(0);
            if matches!(roof, Roof::Gabled { .. }) && outline.len() != 4 {
                return Err("gabled roof needs a rectangular footprint".into());
            }
            list.push(Building {
                kind,
                outline,
                holes: ring_list,
                base_y,
                eave_y,
                roof,
            });
        }
        out.insert(key, list);
    }
    r.finish("buildings section")?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area_of(pts: &[[f64; 2]], tris: &[[u32; 3]]) -> f64 {
        tris.iter()
            .map(|t| cross(pts[t[0] as usize], pts[t[1] as usize], pts[t[2] as usize]) / 2.0)
            .sum()
    }

    fn all_points(outer: &[[f32; 2]], holes: &[Vec<[f32; 2]>]) -> Vec<[f64; 2]> {
        outer
            .iter()
            .chain(holes.iter().flatten())
            .map(|p| p.map(f64::from))
            .collect()
    }

    #[test]
    fn convex_and_concave_polygons_are_triangulated() {
        let square = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let t = triangulate(&square, &[]);
        assert_eq!(t.len(), 2);
        assert!((area_of(&all_points(&square, &[]), &t) - 100.0).abs() < 1e-9);
        // Clockwise input gives counter-clockwise triangles of the same area.
        let mut cw = square;
        cw.reverse();
        let t = triangulate(&cw, &[]);
        assert!((area_of(&all_points(&cw, &[]), &t) - 100.0).abs() < 1e-9);
        // An L shape (concave): 6 points, 4 triangles, area 75.
        let l = [
            [0.0, 0.0],
            [10.0, 0.0],
            [10.0, 5.0],
            [5.0, 5.0],
            [5.0, 10.0],
            [0.0, 10.0],
        ];
        let t = triangulate(&l, &[]);
        assert_eq!(t.len(), 4);
        assert!((area_of(&all_points(&l, &[]), &t) - 75.0).abs() < 1e-9);
        // Collinear points are dropped, not turned into slivers.
        let c = [
            [0.0, 0.0],
            [5.0, 0.0],
            [10.0, 0.0],
            [10.0, 10.0],
            [0.0, 10.0],
        ];
        let t = triangulate(&c, &[]);
        assert!((area_of(&all_points(&c, &[]), &t) - 100.0).abs() < 1e-9);
        // Degenerate input does not panic.
        assert!(triangulate(&[[0.0, 0.0], [1.0, 1.0]], &[]).is_empty());
        assert!(triangulate(&[[0.0, 0.0], [1.0, 1.0], [2.0, 2.0]], &[]).is_empty());
    }

    #[test]
    fn holes_are_left_open() {
        // 20 × 20 courtyard building with two 4 × 4 courtyards: area 368.
        let outer = [[0.0, 0.0], [20.0, 0.0], [20.0, 20.0], [0.0, 20.0]];
        let holes = vec![
            vec![[4.0, 4.0], [8.0, 4.0], [8.0, 8.0], [4.0, 8.0]],
            vec![[12.0, 12.0], [12.0, 16.0], [16.0, 16.0], [16.0, 12.0]],
        ];
        let t = triangulate(&outer, &holes);
        let pts = all_points(&outer, &holes);
        assert!(
            (area_of(&pts, &t) - 368.0).abs() < 1e-9,
            "{}",
            area_of(&pts, &t)
        );
        // No triangle covers the courtyard centres.
        for c in [[6.0, 6.0], [14.0, 14.0]] {
            assert!(!t.iter().any(|tr| in_triangle(
                c,
                pts[tr[0] as usize],
                pts[tr[1] as usize],
                pts[tr[2] as usize]
            )));
        }
        // All triangles wind counter-clockwise.
        assert!(t.iter().all(|tr| cross(
            pts[tr[0] as usize],
            pts[tr[1] as usize],
            pts[tr[2] as usize]
        ) > 0.0));
    }

    #[test]
    fn section_round_trip() {
        let house = Building {
            kind: BuildingKind::House,
            outline: vec![[0.0, 0.0], [12.0, 0.0], [12.0, 8.0], [0.0, 8.0]],
            holes: vec![],
            base_y: 449.5,
            eave_y: 456.0,
            roof: Roof::Gabled { height_m: 3.0 },
        };
        let block = Building {
            kind: BuildingKind::Apartments,
            outline: vec![[0.0, 0.0], [20.0, 0.0], [20.0, 20.0], [0.0, 20.0]],
            holes: vec![vec![[4.0, 4.0], [4.0, 8.0], [8.0, 8.0]]],
            base_y: 450.0,
            eave_y: 465.0,
            roof: Roof::Flat,
        };
        let t = Buildings::from([
            (TileKey { x: 0, z: 0 }, vec![house.clone(), block]),
            (TileKey { x: 3, z: -1 }, vec![]),
        ]);
        let mut b = Vec::new();
        write_buildings(&t, &mut b);
        assert_eq!(read_buildings(&b).unwrap(), t);
        assert!(read_buildings(&b[..b.len() - 2]).is_err());
        // A gabled roof on anything but four points is rejected.
        let bad = Buildings::from([(
            TileKey { x: 0, z: 0 },
            vec![Building {
                outline: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]],
                ..house
            }],
        )]);
        let mut b = Vec::new();
        write_buildings(&bad, &mut b);
        assert!(read_buildings(&b).unwrap_err().contains("rectangular"));
    }
}
