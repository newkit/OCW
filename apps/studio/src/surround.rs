//! What lies around the open package on the studio's 2D map (spec 14
//! Phase 16, 16H Q15 (a)): **the world's own border** as a thin line and
//! **the neighbouring packages the game joins on**, dimmed.
//!
//! The neighbours are found and loaded exactly as the game finds them
//! (`ocw_worldstore::neighbourhood`, spec 2.2.2: installed packages whose
//! region box touches the home package's, verified with the same trust
//! policy, joined where a road crosses, as **skeletons** – graph, routes,
//! region and tile keys, no tiles). The studio already loads them for its
//! 3D view; the map takes only their roads (from the joined graph, in the
//! home package's metres) and their region outline from these skeletons,
//! so nothing more is read for the map. A neighbour **signed by a user key
//! not trusted here** is not joined and not opened (16H Q14): the map
//! draws only the box its manifest states, dashed, marked "unknown key"
//! ([`ocw_worldstore::neighbours::unknown_key_neighbours`]).
//!
//! The neighbours are only drawn: picking on the map reads the open
//! package's own graph, so edits stay in the open package.

use ocw_map::data::{MapRoad, bbox};
use ocw_world::WorldSet;
use ocw_world::geo::LocalProjection;
use ocw_world::package::WorldPackage;
use ocw_world::region::PackageRegion;
use ocw_worldstore::neighbours::UnknownKeyNeighbour;

/// A closed ring in package metres (x, z), the first point not repeated.
pub type Ring = Vec<[f64; 2]>;

/// A joined neighbouring package as the map draws it.
#[derive(Debug, Clone, PartialEq)]
pub struct NeighbourMap {
    pub id: String,
    pub name: String,
    /// Its region's outline (home metres).
    pub outline: Vec<Ring>,
    /// Its region filled (triangles over `fill_points`), dim ground.
    pub fill_points: Vec<[f32; 2]>,
    pub fill: Vec<[u32; 3]>,
    /// Its roads (home metres).
    pub roads: Vec<MapRoad>,
    /// Where its name goes: the middle of its region.
    pub label_at: [f64; 2],
}

/// A package next to the world by a user key not trusted here: only its
/// stated box.
#[derive(Debug, Clone, PartialEq)]
pub struct UnknownOutline {
    pub name: String,
    pub claimed_signer: String,
    pub outline: Ring,
    pub label_at: [f64; 2],
}

/// The border and the neighbours of the open package.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Surround {
    /// The world's own border (home metres).
    pub border: Vec<Ring>,
    pub neighbours: Vec<NeighbourMap>,
    pub unknown: Vec<UnknownOutline>,
}

impl Surround {
    /// The map's text for the World workspace: what joins and what not.
    pub fn summary(&self) -> String {
        let mut parts: Vec<String> = self.neighbours.iter().map(|n| n.name.clone()).collect();
        parts.extend(
            self.unknown
                .iter()
                .map(|u| format!("{} (unknown key)", u.name)),
        );
        if parts.is_empty() {
            "none: the world ends at its border".into()
        } else {
            parts.join(", ")
        }
    }

    /// The box of the home extent `min`…`max` with the neighbours' and the
    /// unknown packages' outlines (View → Fit the world and its
    /// neighbours).
    pub fn extent(&self, min: [f64; 2], max: [f64; 2]) -> ([f64; 2], [f64; 2]) {
        self.neighbours
            .iter()
            .flat_map(|n| n.outline.iter().flatten())
            .chain(self.unknown.iter().flat_map(|u| &u.outline))
            .fold((min, max), |(lo, hi), p| {
                (
                    [lo[0].min(p[0]), lo[1].min(p[1])],
                    [hi[0].max(p[0]), hi[1].max(p[1])],
                )
            })
    }

    /// Whether `p` (home metres) lies inside a joined neighbour's region
    /// and outside the world's own border.
    pub fn in_neighbour(&self, p: [f64; 2]) -> bool {
        !inside(&self.border, p) && self.neighbours.iter().any(|n| inside(&n.outline, p))
    }
}

/// Even-odd test over all rings (outer rings and holes alike).
pub fn inside(rings: &[Ring], p: [f64; 2]) -> bool {
    let mut is_in = false;
    for r in rings {
        let n = r.len();
        for i in 0..n {
            let (a, b) = (r[i], r[(i + n - 1) % n]);
            if (a[1] > p[1]) != (b[1] > p[1])
                && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
            {
                is_in = !is_in;
            }
        }
    }
    is_in
}

/// `ring` (lat, lon) cut to the box of `r` (Sutherland-Hodgman; the box
/// is convex): the part of the polygon a package cut with `--bbox` covers
/// (2.2.2; its `REGN` keeps the whole `.poly` and the box).
pub fn clip_to_box(ring: &[(f64, f64)], r: &PackageRegion) -> Vec<(f64, f64)> {
    // The box's sides: latitude (true) or longitude, the limit, and
    // whether the inside is above it.
    let sides = [
        (true, r.south, true),
        (true, r.north, false),
        (false, r.west, true),
        (false, r.east, false),
    ];
    let mut pts: Vec<(f64, f64)> = ring.to_vec();
    if pts.len() > 1 && pts.first() == pts.last() {
        pts.pop();
    }
    for (lat, v, above) in sides {
        let n = pts.len();
        if n == 0 {
            break;
        }
        let coord = |p: (f64, f64)| if lat { p.0 } else { p.1 };
        let inside = |p: (f64, f64)| if above { coord(p) >= v } else { coord(p) <= v };
        let cross = |a: (f64, f64), b: (f64, f64)| {
            let t = (v - coord(a)) / (coord(b) - coord(a));
            if lat {
                (v, a.1 + t * (b.1 - a.1))
            } else {
                (a.0 + t * (b.0 - a.0), v)
            }
        };
        let mut out = Vec::with_capacity(n + 4);
        for i in 0..n {
            let (a, b) = (pts[(i + n - 1) % n], pts[i]);
            match (inside(a), inside(b)) {
                (true, true) => out.push(b),
                (true, false) => out.push(cross(a, b)),
                (false, true) => {
                    out.push(cross(a, b));
                    out.push(b);
                }
                (false, false) => {}
            }
        }
        pts = out;
    }
    pts
}

/// The rings of `region` in the metres of `proj`: its polygon's outer
/// rings and holes cut to its box, else the latitude / longitude box
/// (straight in the local projection, 4.5).
pub fn region_rings(region: &PackageRegion, proj: &LocalProjection) -> Vec<Ring> {
    let ring = |r: &Vec<(f64, f64)>| -> Ring {
        clip_to_box(r, region)
            .into_iter()
            .map(|(lat, lon)| proj.project(lat, lon))
            .collect()
    };
    let rings: Vec<Ring> = region
        .outer
        .iter()
        .chain(&region.holes)
        .map(ring)
        .filter(|r| r.len() >= 3)
        .collect();
    if !rings.is_empty() {
        return rings;
    }
    vec![vec![
        proj.project(region.north, region.west),
        proj.project(region.north, region.east),
        proj.project(region.south, region.east),
        proj.project(region.south, region.west),
    ]]
}

/// The world's own border (home metres): its region (section `REGN`),
/// else the tiles' extent `min`…`max` (a package without a region).
pub fn border(pkg: &WorldPackage, min: [f64; 2], max: [f64; 2]) -> Vec<Ring> {
    match &pkg.region {
        Some(r) => region_rings(r, &LocalProjection::new(pkg.origin.0, pkg.origin.1)),
        None if max[0] > min[0] && max[1] > min[1] => vec![vec![
            [min[0], min[1]],
            [max[0], min[1]],
            [max[0], max[1]],
            [min[0], max[1]],
        ]],
        None => Vec::new(),
    }
}

fn middle(rings: &[Ring]) -> [f64; 2] {
    let (lo, hi) = rings.iter().flatten().fold(
        ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]),
        |(lo, hi), p| {
            (
                [lo[0].min(p[0]), lo[1].min(p[1])],
                [hi[0].max(p[0]), hi[1].max(p[1])],
            )
        },
    );
    [(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5]
}

/// What surrounds `pkg` (the home package of `set`): its border, the
/// joined neighbours of `set` – `regions[i]` is the region of member
/// `i + 1` (the skeletons' `REGN`) – and the packages by an unknown key.
pub fn surround(
    pkg: &WorldPackage,
    extent: ([f64; 2], [f64; 2]),
    set: &WorldSet,
    regions: &[Option<PackageRegion>],
    unknown: &[UnknownKeyNeighbour],
) -> Surround {
    let proj = LocalProjection::new(pkg.origin.0, pkg.origin.1);
    let members = set.members();
    let mut neighbours: Vec<NeighbourMap> = members
        .iter()
        .enumerate()
        .skip(1)
        .map(|(i, m)| {
            let outline = match regions.get(i - 1).and_then(Option::as_ref) {
                Some(r) => region_rings(r, &proj),
                None => {
                    let (lo, hi) = m.tile_bounds();
                    vec![vec![lo, [hi[0], lo[1]], hi, [lo[0], hi[1]]]]
                }
            };
            let (fill_points, fill) = fill(&outline);
            NeighbourMap {
                id: m.id.clone(),
                name: m.name.clone(),
                label_at: middle(&outline),
                outline,
                fill_points,
                fill,
                roads: Vec::new(),
            }
        })
        .collect();
    // The joined graph holds the home package's edges first, then each
    // neighbour's (spec 2.2.2, `join_graphs`); a road goes to the
    // neighbour whose region holds its middle, else to the nearest.
    if let (Some(g), false) = (set.graph(), neighbours.is_empty()) {
        let own = pkg.graph.as_ref().map_or(0, |g| g.edges.len());
        for e in g.edges.iter().skip(own).filter(|e| e.points.len() >= 2) {
            let mid = e.points[e.points.len() / 2];
            let k = members
                .iter()
                .enumerate()
                .skip(1)
                .find(|(_, m)| m.region_contains(mid))
                .or_else(|| {
                    members
                        .iter()
                        .enumerate()
                        .skip(1)
                        .min_by(|a, b| a.1.distance(mid).total_cmp(&b.1.distance(mid)))
                })
                .map_or(1, |(i, _)| i);
            let points: Vec<[f32; 2]> = e
                .points
                .iter()
                .map(|p| [p[0] as f32, p[1] as f32])
                .collect();
            let (min, max) = bbox(points.iter().copied());
            neighbours[k - 1].roads.push(MapRoad {
                class: e.class,
                width_m: e.width_m as f32,
                points,
                min,
                max,
            });
        }
    }
    let unknown = unknown
        .iter()
        .map(|u| {
            // The box the manifest states (its polygon is not read).
            let b = &u.bounds;
            let outline = region_rings(
                &PackageRegion::bbox(b.south, b.west, b.north, b.east),
                &proj,
            )
            .remove(0);
            UnknownOutline {
                name: u.name.clone(),
                claimed_signer: u.claimed_signer.clone(),
                label_at: middle(std::slice::from_ref(&outline)),
                outline,
            }
        })
        .collect();
    Surround {
        border: border(pkg, extent.0, extent.1),
        neighbours,
        unknown,
    }
}

/// The outer rings of `rings` as triangles (for the dim fill).
fn fill(rings: &[Ring]) -> (Vec<[f32; 2]>, Vec<[u32; 3]>) {
    let mut points = Vec::new();
    let mut tris = Vec::new();
    for r in rings {
        let pts: Vec<[f32; 2]> = r.iter().map(|p| [p[0] as f32, p[1] as f32]).collect();
        let base = points.len() as u32;
        for t in ocw_world::buildings::triangulate(&pts, &[]) {
            tris.push([base + t[0], base + t[1], base + t[2]]);
        }
        points.extend(pts);
    }
    (points, tris)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use ocw_world::graph::{Edge, EdgeId, Oneway, RoadClass, RoadGraph, VertexId};
    use ocw_world::package::{Tile, TileKey};

    const ORIGIN: (f64, f64) = (48.0, 8.1);

    fn edge(id: u64, from: u32, to: u32, pts: Vec<[f64; 2]>, boundary: bool) -> Edge {
        Edge {
            id: EdgeId(id),
            from: VertexId(from),
            to: VertexId(to),
            points: pts,
            class: RoadClass::Secondary,
            name: None,
            width_m: 6.0,
            bridge: false,
            tunnel: false,
            dead_end_m: None,
            boundary,
            oneway: Oneway::No,
            osm_way: 7,
            profile: vec![],
        }
    }

    /// Package `k` of a column of three (south 0 – middle 1 – north 2, as
    /// the Black Forest's): lat 47.98 + 0.02·k … + 0.02, lon 8.08 … 8.12,
    /// a road along x = 0 from its south to its north border (made up).
    fn part(k: u32) -> WorldPackage {
        let proj = LocalProjection::new(ORIGIN.0, ORIGIN.1);
        let (s, n) = (47.98 + 0.02 * f64::from(k), 48.0 + 0.02 * f64::from(k));
        let (a, b) = (proj.project(s, 8.1), proj.project(n, 8.1));
        let id = 100 + u64::from(k);
        let g = RoadGraph {
            vertices: vec![a, b],
            edges: vec![edge(id, 0, 1, vec![a, [0.0, (a[1] + b[1]) / 2.0], b], true)],
            boundary_vertices: [VertexId(0), VertexId(1)].into(),
        };
        let mut tiles = std::collections::BTreeMap::new();
        for z in (b[1] / 1000.0).floor() as i32..=(a[1] / 1000.0).floor() as i32 {
            for x in -2..2 {
                let key = TileKey { x, z };
                tiles.insert(
                    key,
                    Tile {
                        key,
                        heights: vec![500.0; 4],
                        roads: vec![],
                    },
                );
            }
        }
        WorldPackage {
            name: ["South", "Middle", "North"][k as usize].into(),
            origin: ORIGIN,
            tile_size_m: 1000.0,
            samples: 2,
            tiles,
            graph: Some(g.into()),
            landuse: None,
            buildings: None,
            bridges: None,
            routes: vec![],
            region: Some(PackageRegion::bbox(s, 8.08, n, 8.12)),
            time_zone: None,
            look: Default::default(),
            obstacles: None,
            places: None,
            junctions: None,
            traffic: None,
            aero: None,
        }
    }

    fn set_of(home: &WorldPackage, others: &[WorldPackage]) -> WorldSet {
        let mut set = WorldSet::new("bf/middle", Arc::new(home.clone()));
        for (i, o) in others.iter().enumerate() {
            set.add_neighbour(&format!("bf/{i}"), Arc::new(o.clone()))
                .unwrap();
        }
        set
    }

    /// Spec 16H Q15: the middle of three packages has both neighbours,
    /// dimmed beside its own border, each with its own road.
    #[test]
    fn the_middle_package_has_both_neighbours_with_their_roads() {
        let (south, middle, north) = (part(0), part(1), part(2));
        let set = set_of(&middle, &[south.clone(), north.clone()]);
        assert_eq!(set.len(), 3);
        let extent = ocw_map::data::tile_extent(&middle);
        let s = surround(
            &middle,
            extent,
            &set,
            &[south.region.clone(), north.region.clone()],
            &[],
        );
        assert_eq!(s.neighbours.len(), 2);
        assert_eq!(s.neighbours[0].name, "South");
        assert_eq!(s.neighbours[1].name, "North");
        // Each neighbour's road, none of the home package's.
        for n in &s.neighbours {
            assert_eq!(n.roads.len(), 1, "{}", n.name);
            assert!(!n.fill.is_empty(), "filled dim");
        }
        // South is south (larger z), north is north.
        assert!(s.neighbours[0].label_at[1] > 0.0);
        assert!(s.neighbours[1].label_at[1] < 0.0);
        assert!(s.neighbours[0].roads[0].max[1] > 2000.0);
        // A point in the north package is a neighbour's, one in the middle
        // package is not, one far away neither.
        let north_pt = s.neighbours[1].label_at;
        assert!(s.in_neighbour(north_pt));
        assert!(!s.in_neighbour([0.0, -1112.0]));
        assert!(!s.in_neighbour([0.0, -50_000.0]));
        assert_eq!(s.summary(), "South, North");
        // Fitting the world with its neighbours: three packages high.
        let (lo, hi) = s.extent([-100.0, -2000.0], [100.0, -100.0]);
        assert!(
            (hi[1] - lo[1] - 3.0 * 2224.0).abs() < 120.0,
            "{lo:?} {hi:?}"
        );
        assert!(hi[0] - lo[0] > 2900.0);
    }

    /// Spec 16H Q15: the border comes from the package's region box (4
    /// corners in the local metres), else from its tiles; a package alone
    /// has no neighbours.
    #[test]
    fn the_border_outline_follows_the_package_bounds() {
        let middle = part(1);
        let b = border(&middle, [0.0; 2], [0.0; 2]);
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].len(), 4);
        let proj = LocalProjection::new(ORIGIN.0, ORIGIN.1);
        let nw = proj.project(48.02, 8.08);
        let se = proj.project(48.0, 8.12);
        assert!((b[0][0][0] - nw[0]).abs() < 1e-6 && (b[0][0][1] - nw[1]).abs() < 1e-6);
        assert!((b[0][2][0] - se[0]).abs() < 1e-6 && (b[0][2][1] - se[1]).abs() < 1e-6);
        // About 3 km wide (0.04° of longitude at 48° N) and 2.2 km high.
        assert!((se[0] - nw[0] - 2975.0).abs() < 40.0, "{}", se[0] - nw[0]);
        assert!((se[1] - nw[1] - 2224.0).abs() < 40.0, "{}", se[1] - nw[1]);
        assert!(inside(&b, [0.0, -1112.0]));
        assert!(!inside(&b, [0.0, 1000.0]));
        // A polygon region: its rings (closing point dropped), holes too.
        let mut poly = middle.clone();
        let r = poly.region.as_mut().unwrap();
        r.outer = vec![vec![
            (48.02, 8.08),
            (48.02, 8.12),
            (48.0, 8.1),
            (48.02, 8.08),
        ]];
        r.holes = vec![vec![(48.016, 8.099), (48.016, 8.101), (48.014, 8.1)]];
        let b = border(&poly, [0.0; 2], [0.0; 2]);
        assert_eq!(b.len(), 2);
        assert_eq!(b[0].len(), 3);
        assert!(!inside(&b, proj.project(48.0155, 8.1)), "in the hole");
        // A polygon larger than the box (a `--bbox` cut of a `.poly`):
        // cut to the box.
        let r = poly.region.as_mut().unwrap();
        r.outer = vec![vec![(47.9, 8.0), (48.1, 8.0), (48.1, 8.2), (47.9, 8.2)]];
        r.holes.clear();
        let b = border(&poly, [0.0; 2], [0.0; 2]);
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].len(), 4, "{:?}", b[0]);
        for p in &b[0] {
            assert!(
                (p[0] - nw[0]).abs() < 1e-6 || (p[0] - se[0]).abs() < 1e-6,
                "{p:?}"
            );
        }
        // A ring crossing one side keeps its inner part.
        let half = clip_to_box(
            &[(48.01, 8.0), (48.01, 8.1), (48.015, 8.1), (48.015, 8.0)],
            poly.region.as_ref().unwrap(),
        );
        assert_eq!(half.len(), 4);
        assert!(half.iter().all(|p| p.1 >= 8.08 - 1e-12), "{half:?}");
        // No region: the tiles' extent.
        let mut bare = middle.clone();
        bare.region = None;
        let b = border(&bare, [-2000.0, -1000.0], [2000.0, 1000.0]);
        assert_eq!(
            b[0],
            vec![
                [-2000.0, -1000.0],
                [2000.0, -1000.0],
                [2000.0, 1000.0],
                [-2000.0, 1000.0]
            ]
        );
        // Alone (as Liechtenstein, Tenerife): the border only.
        let set = WorldSet::new("li", Arc::new(middle.clone()));
        let s = surround(&middle, ([0.0; 2], [0.0; 2]), &set, &[], &[]);
        assert!(s.neighbours.is_empty() && s.unknown.is_empty());
        assert_eq!(s.border.len(), 1);
        assert_eq!(s.summary(), "none: the world ends at its border");
    }

    /// A neighbour by an unknown key: its stated box only, marked.
    #[test]
    fn an_unknown_key_neighbour_is_an_outline() {
        let middle = part(1);
        let set = WorldSet::new("bf/middle", Arc::new(middle.clone()));
        let u = UnknownKeyNeighbour {
            spec: "north.ocwpkg".into(),
            id: "user/ada/north".into(),
            name: "North".into(),
            claimed_signer: "Ada Rider".into(),
            bounds: PackageRegion::bbox(48.02, 8.08, 48.04, 8.12),
        };
        let s = surround(&middle, ([0.0; 2], [0.0; 2]), &set, &[], &[u]);
        assert!(s.neighbours.is_empty());
        assert_eq!(s.unknown.len(), 1);
        assert_eq!(s.unknown[0].outline.len(), 4);
        assert!(s.unknown[0].label_at[1] < 0.0, "north of the middle");
        assert_eq!(s.summary(), "North (unknown key)");
    }
}
