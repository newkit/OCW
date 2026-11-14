//! The section `OBST` (spec §14 Phase 12 K3, 12B): static obstacles near
//! the roads from OSM – `barrier=*` nodes and ways (gates, bollards, lift
//! gates, blocks, cycle barriers, fences, walls, hedges, guard rails,
//! kerbs), posts and pylons (`power=pole|tower`, `man_made=utility_pole`,
//! guide posts) – and the tree cells of the package's tree placement
//! ([`ocw_world::trees`]). The 2 m corridor rule: only what lies within
//! wild mode's reach of a road ([`WILD_REACH_M`] beside the centre line)
//! plus [`CORRIDOR_MARGIN_M`] is kept; point barriers within 2 m of a
//! carriageway are the road's ([`ROAD_BARRIER`]: drawn with
//! `display.road_barriers`, never solid, Q7), on it also [`ON_ROAD`]; the
//! parts of barrier lines on a carriageway are cut out.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use ocw_world::geo::LocalProjection;
use ocw_world::obstacles::{
    CORRIDOR_MARGIN_M, LineObstacle, ON_ROAD, ObstacleKind, Obstacles, PointObstacle, ROAD_BARRIER,
    WILD_REACH_M,
};
use ocw_world::{TileKey, WorldPackage};

use crate::osm::{OsmError, Tags, par_elements, way_box_overlaps};

/// Keys kept on obstacle nodes and ways.
fn kept(k: &str) -> bool {
    matches!(
        k,
        "barrier" | "power" | "man_made" | "information" | "height" | "highway" | "material"
    )
}

/// Whether a node is an obstacle (its kind, [`ObstacleKind::of_point`]).
fn point_kind(tags: &Tags) -> Option<ObstacleKind> {
    ObstacleKind::of_point(&|k: &str| tags.get(k).cloned())
}

/// OSM barrier ways, obstacle nodes and the nodes the ways use.
#[derive(Debug, Clone, Default)]
pub struct BarrierData {
    /// Node id → (lat, lon).
    pub nodes: HashMap<i64, (f64, f64)>,
    /// Obstacle nodes (barriers, posts) with their kept tags.
    pub points: Vec<(i64, Tags)>,
    /// Barrier ways: node ids and kept tags.
    pub ways: Vec<(i64, Vec<i64>, Tags)>,
}

impl BarrierData {
    /// Two passes: barrier ways, then the nodes (theirs and the obstacle
    /// nodes).
    pub fn read_pbf(path: &Path) -> Result<Self, OsmError> {
        let ways = par_elements(path, |el, out| {
            if let osmpbf::Element::Way(w) = el
                && w.tags().any(|(k, _)| k == "barrier")
            {
                let tags: Tags = w
                    .tags()
                    .filter(|(k, _)| kept(k))
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect();
                out.push((w.id(), w.refs().collect::<Vec<_>>(), tags));
            }
        })?;
        let needed: HashSet<i64> = ways.iter().flat_map(|w| w.1.iter().copied()).collect();
        let found = par_elements(path, |el, out| {
            let (id, lat, lon, tags): (i64, f64, f64, Vec<(&str, &str)>) = match el {
                osmpbf::Element::Node(n) => (n.id(), n.lat(), n.lon(), n.tags().collect()),
                osmpbf::Element::DenseNode(n) => (n.id(), n.lat(), n.lon(), n.tags().collect()),
                _ => return,
            };
            let tags: Tags = tags
                .into_iter()
                .filter(|(k, _)| kept(k))
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            let obstacle = point_kind(&tags).is_some();
            if obstacle || needed.contains(&id) {
                out.push((id, (lat, lon), obstacle.then_some(tags)));
            }
        })?;
        let mut d = Self {
            ways,
            ..Default::default()
        };
        for (id, p, tags) in found {
            d.nodes.insert(id, p);
            if let Some(t) = tags {
                d.points.push((id, t));
            }
        }
        Ok(d)
    }

    /// Reads several extracts and merges them (by id).
    pub fn read_pbfs(paths: &[PathBuf]) -> Result<Self, String> {
        let mut out = Self::default();
        let (mut ways, mut points) = (HashSet::new(), HashSet::new());
        for p in paths {
            let d = Self::read_pbf(p).map_err(|e| format!("{}: {e}", p.display()))?;
            out.nodes.extend(d.nodes);
            for w in d.ways {
                if ways.insert(w.0) {
                    out.ways.push(w);
                }
            }
            for n in d.points {
                if points.insert(n.0) {
                    out.points.push(n);
                }
            }
        }
        Ok(out)
    }

    /// Keeps what overlaps (south, west, north, east).
    pub fn retain_near(&mut self, bounds: (f64, f64, f64, f64)) {
        let nodes = &self.nodes;
        self.ways
            .retain(|w| way_box_overlaps(w.1.iter().filter_map(|n| nodes.get(n)), bounds));
        self.points.retain(|(id, _)| {
            nodes
                .get(id)
                .is_some_and(|p| way_box_overlaps(std::iter::once(p), bounds))
        });
        let used: HashSet<i64> = self
            .ways
            .iter()
            .flat_map(|w| w.1.iter().copied())
            .chain(self.points.iter().map(|p| p.0))
            .collect();
        self.nodes.retain(|id, _| used.contains(id));
    }
}

/// A road strip segment: centre line ends and half width.
#[derive(Debug, Clone, Copy)]
struct StripSeg {
    a: [f64; 2],
    b: [f64; 2],
    half: f64,
}

/// The road strips of a package in a grid, for distances to the roads.
pub struct StripIndex {
    cell: f64,
    segs: Vec<StripSeg>,
    grid: HashMap<(i64, i64), Vec<u32>>,
}

/// What [`StripIndex::nearest`] finds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoadNear {
    /// Distance to the nearest centre line (m).
    pub centre_m: f64,
    /// Distance beyond the nearest carriageway's edge (m, < 0 on it).
    pub edge_m: f64,
    /// Direction of the road with the nearest edge (rad, 0 = +x).
    pub angle: f64,
    /// Its width (m).
    pub width_m: f64,
}

impl StripIndex {
    pub fn new(pkg: &WorldPackage) -> Self {
        let cell = 32.0;
        let mut segs = Vec::new();
        let mut grid: HashMap<(i64, i64), Vec<u32>> = HashMap::new();
        for t in pkg.tiles.values() {
            for r in &t.roads {
                let half = f64::from(r.width_m) / 2.0;
                for w in r.points.windows(2) {
                    let (a, b) = (
                        [f64::from(w[0][0]), f64::from(w[0][2])],
                        [f64::from(w[1][0]), f64::from(w[1][2])],
                    );
                    let i = segs.len() as u32;
                    segs.push(StripSeg { a, b, half });
                    let (x0, x1) = (
                        (a[0].min(b[0]) / cell).floor() as i64,
                        (a[0].max(b[0]) / cell).floor() as i64,
                    );
                    let (z0, z1) = (
                        (a[1].min(b[1]) / cell).floor() as i64,
                        (a[1].max(b[1]) / cell).floor() as i64,
                    );
                    for x in x0..=x1 {
                        for z in z0..=z1 {
                            grid.entry((x, z)).or_default().push(i);
                        }
                    }
                }
            }
        }
        Self { cell, segs, grid }
    }

    /// The nearest road within `max` m of the centre line (by the edge
    /// distance), `None` if none.
    pub fn nearest(&self, p: [f64; 2], max: f64) -> Option<RoadNear> {
        let r = (max / self.cell).ceil() as i64 + 1;
        let (cx, cz) = (
            (p[0] / self.cell).floor() as i64,
            (p[1] / self.cell).floor() as i64,
        );
        let mut best: Option<RoadNear> = None;
        let mut centre = f64::INFINITY;
        for x in cx - r..=cx + r {
            for z in cz - r..=cz + r {
                for &i in self.grid.get(&(x, z)).into_iter().flatten() {
                    let s = self.segs[i as usize];
                    let d = ocw_world::obstacles::segment_distance(p, s.a, s.b);
                    centre = centre.min(d);
                    if d <= max && best.is_none_or(|b| d - s.half < b.edge_m) {
                        best = Some(RoadNear {
                            centre_m: d,
                            edge_m: d - s.half,
                            angle: (s.b[1] - s.a[1]).atan2(s.b[0] - s.a[0]),
                            width_m: 2.0 * s.half,
                        });
                    }
                }
            }
        }
        best.map(|b| RoadNear {
            centre_m: centre,
            ..b
        })
    }
}

/// The reach of the corridor rule: wild mode's plus the 2 m.
pub const CORRIDOR_M: f64 = WILD_REACH_M + CORRIDOR_MARGIN_M;

/// How many of each were written, and what was left out.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ObstacleStats {
    pub by_kind: std::collections::BTreeMap<ObstacleKind, usize>,
    pub road_barriers: usize,
    pub on_road: usize,
    pub outside_corridor: usize,
    pub line_pieces_cut: usize,
    pub trees: usize,
    pub tree_tiles: usize,
}

/// A height tag in metres ("2", "2.5 m", "1,8").
fn height_m(tags: &Tags) -> Option<f32> {
    let v = tags.get("height")?;
    let t = v.trim().trim_end_matches('m').trim().replace(',', ".");
    t.parse::<f32>().ok().filter(|h| (0.1..100.0).contains(h))
}

/// Cuts a polyline where `outside` turns false (sampled every 0.5 m):
/// the pieces that stay outside, each at least 0.3 m long.
pub fn cut_line(points: &[[f64; 2]], outside: impl Fn([f64; 2]) -> bool) -> Vec<Vec<[f64; 2]>> {
    let mut out = Vec::new();
    let mut cur: Vec<[f64; 2]> = Vec::new();
    let flush = |cur: &mut Vec<[f64; 2]>, out: &mut Vec<Vec<[f64; 2]>>| {
        let len: f64 = cur
            .windows(2)
            .map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]))
            .sum();
        if cur.len() >= 2 && len >= 0.3 {
            out.push(std::mem::take(cur));
        } else {
            cur.clear();
        }
    };
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len = (b[0] - a[0]).hypot(b[1] - a[1]);
        let steps = (len / 0.5).ceil().max(1.0) as usize;
        let at = |t: f64| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
        for k in 0..steps {
            let (t0, t1) = (k as f64 / steps as f64, (k + 1) as f64 / steps as f64);
            let mid = at((t0 + t1) / 2.0);
            if outside(mid) {
                if cur.is_empty() {
                    cur.push(at(t0));
                }
                // Original vertices and cut points only (collinear steps
                // merge).
                if k + 1 == steps {
                    cur.push(at(t1));
                }
            } else {
                if !cur.is_empty() {
                    cur.push(at(t0));
                }
                flush(&mut cur, &mut out);
            }
        }
    }
    flush(&mut cur, &mut out);
    out
}

/// Builds the section from the barrier data and the package (its strips,
/// land use, buildings and terrain for the trees).
pub fn build_obstacles(
    pkg: &WorldPackage,
    data: &BarrierData,
    proj: &LocalProjection,
) -> (Obstacles, ObstacleStats) {
    let roads = StripIndex::new(pkg);
    let size = f64::from(pkg.tile_size_m);
    let mut o = Obstacles::new(pkg.tile_size_m);
    let mut st = ObstacleStats::default();
    let tile_of = |p: [f64; 2]| TileKey::containing(p, size);
    // Points.
    let mut points: Vec<(i64, &Tags)> = data.points.iter().map(|(i, t)| (*i, t)).collect();
    points.sort_by_key(|p| p.0);
    for (id, tags) in points {
        let (Some(kind), Some(&(lat, lon))) = (point_kind(tags), data.nodes.get(&id)) else {
            continue;
        };
        let p = proj.project(lat, lon);
        if !pkg.tiles.contains_key(&tile_of(p)) {
            continue;
        }
        let Some(near) = roads.nearest(p, CORRIDOR_M) else {
            st.outside_corridor += 1;
            continue;
        };
        let mut flags = 0;
        if near.edge_m <= CORRIDOR_MARGIN_M
            && kind != ObstacleKind::Pylon
            && kind != ObstacleKind::Post
        {
            flags |= ROAD_BARRIER;
            st.road_barriers += 1;
        }
        if near.edge_m <= 0.0 && flags & ROAD_BARRIER != 0 {
            flags |= ON_ROAD;
            st.on_road += 1;
        }
        // A gate or lift gate on the road spans it, across its direction.
        let spans =
            matches!(kind, ObstacleKind::Gate | ObstacleKind::LiftGate) && flags & ON_ROAD != 0;
        let point = PointObstacle {
            kind,
            flags,
            pos: [p[0] as f32, p[1] as f32],
            angle: near.angle as f32,
            width_m: if spans { near.width_m as f32 } else { 0.0 },
            height_m: height_m(tags).unwrap_or(kind.default_height_m() as f32),
        };
        *st.by_kind.entry(kind).or_default() += 1;
        o.tiles.entry(tile_of(p)).or_default().points.push(point);
    }
    // Lines: cut out of the carriageways and to the corridor.
    let mut ways: Vec<&(i64, Vec<i64>, Tags)> = data.ways.iter().collect();
    ways.sort_by_key(|w| w.0);
    for (_, nodes, tags) in ways {
        let Some(kind) = tags.get("barrier").and_then(|b| ObstacleKind::of_line(b)) else {
            continue;
        };
        let pts: Option<Vec<[f64; 2]>> = nodes
            .iter()
            .map(|n| data.nodes.get(n).map(|&(lat, lon)| proj.project(lat, lon)))
            .collect();
        let Some(pts) = pts.filter(|p| p.len() >= 2) else {
            continue;
        };
        let pieces = cut_line(&pts, |p| {
            roads
                .nearest(p, CORRIDOR_M)
                .is_some_and(|n| n.edge_m > 0.0 || kind == ObstacleKind::Kerb)
        });
        if pieces.is_empty() {
            st.outside_corridor += 1;
            continue;
        }
        st.line_pieces_cut += pieces.len() - 1;
        let h = height_m(tags).unwrap_or(kind.default_height_m() as f32);
        for piece in pieces {
            let key = tile_of(piece[0]);
            if !pkg.tiles.contains_key(&key) {
                continue;
            }
            *st.by_kind.entry(kind).or_default() += 1;
            o.tiles.entry(key).or_default().lines.push(LineObstacle {
                kind,
                flags: 0,
                height_m: h,
                points: piece.iter().map(|p| [p[0] as f32, p[1] as f32]).collect(),
            });
        }
    }
    // Trees within the corridor.
    let keys: Vec<TileKey> = pkg.tiles.keys().copied().collect();
    let trees = crate::par::map(&keys, crate::par::threads(), |key| {
        let tile = &pkg.tiles[key];
        ocw_world::trees::tree_cells(pkg, tile, |p| roads.nearest(p, CORRIDOR_M).is_some())
    });
    for (key, cells) in keys.iter().zip(trees) {
        let Some(cells) = cells else { continue };
        let n = cells.iter().filter(|c| **c != 0).count();
        if n == 0 {
            continue;
        }
        st.trees += n;
        st.tree_tiles += 1;
        o.tiles.entry(*key).or_default().set_tree_cells(&cells);
    }
    o.tiles.retain(|_, t| !t.is_empty());
    (o, st)
}

/// Adds the section to a package (replacing one it has).
pub fn add_obstacles(
    pkg: &mut WorldPackage,
    data: &BarrierData,
    proj: &LocalProjection,
) -> ObstacleStats {
    let (o, st) = build_obstacles(pkg, data, proj);
    pkg.obstacles = Some(o);
    st
}

impl ObstacleStats {
    /// One line for the build log.
    pub fn summary(&self) -> String {
        let kinds: Vec<String> = self
            .by_kind
            .iter()
            .map(|(k, n)| match k {
                ObstacleKind::GateLine => format!("{n} gate ways"),
                _ => format!("{n} {}", k.label()),
            })
            .collect();
        format!(
            "{}; {} road barriers ({} on a carriageway), {} trees in {} tiles; {} outside the corridor, {} line cuts at roads",
            if kinds.is_empty() {
                "no OSM obstacles".to_string()
            } else {
                kinds.join(", ")
            },
            self.road_barriers,
            self.on_road,
            self.trees,
            self.tree_tiles,
            self.outside_corridor,
            self.line_pieces_cut
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_world::graph::RoadClass;
    use ocw_world::{RoadDress, RoadLit, RoadStrip, RoadSurface, Tile};
    use std::collections::BTreeMap;

    /// A package with one 6 m road along +x at z = 500 through tile (0, 0).
    fn package() -> WorldPackage {
        let key = TileKey { x: 0, z: 0 };
        let strip = RoadStrip {
            class: RoadClass::Residential,
            width_m: 6.0,
            points: vec![[0.0, 100.0, 500.0], [1000.0, 100.0, 500.0]],
            surface: RoadSurface::of_class(RoadClass::Residential),
            dress: RoadDress::of_class(RoadClass::Residential, 6.0),
            lit: RoadLit::Unknown,
        };
        WorldPackage {
            name: "T".into(),
            origin: (47.0, 9.0),
            tile_size_m: 1000.0,
            samples: 11,
            tiles: BTreeMap::from([(
                key,
                Tile {
                    key,
                    heights: vec![99.0; 121],
                    roads: vec![strip],
                },
            )]),
            graph: None,
            landuse: Some(ocw_world::landuse::LandUse {
                cells: 100,
                tiles: BTreeMap::from([(
                    key,
                    ocw_world::landuse::LandUseTile {
                        classes: vec![ocw_world::landuse::LandClass::Forest; 100 * 100],
                        water: Vec::new(),
                    },
                )]),
            }),
            buildings: None,
            bridges: None,
            routes: Vec::new(),
            region: None,
            time_zone: None,
            look: Default::default(),
            obstacles: None,
        }
    }

    fn tags(pairs: &[(&str, &str)]) -> Tags {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    type Nodes<'a> = [(i64, [f64; 2], &'a [(&'a str, &'a str)])];
    type Ways<'a> = [(i64, &'a [i64], &'a [(&'a str, &'a str)])];

    /// Barrier nodes and ways at package metres (x, z).
    fn data(proj: &LocalProjection, nodes: &Nodes<'_>, ways: &Ways<'_>) -> BarrierData {
        let mut d = BarrierData::default();
        for (id, p, t) in nodes {
            d.nodes.insert(*id, proj.unproject(*p));
            if !t.is_empty() {
                d.points.push((*id, tags(t)));
            }
        }
        for (id, ns, t) in ways {
            d.ways.push((*id, ns.to_vec(), tags(t)));
        }
        d
    }

    #[test]
    fn obstacles_by_barrier_type_with_the_corridor_rule() {
        let pkg = package();
        let proj = LocalProjection::new(pkg.origin.0, pkg.origin.1);
        let d = data(
            &proj,
            &[
                // On the road: a bollard (a road barrier, on the carriageway).
                (1, [100.0, 500.0], &[("barrier", "bollard")]),
                // 1.5 m beside the edge: a road barrier, not on it.
                (2, [110.0, 504.5], &[("barrier", "bollard")]),
                // A gate across the road: spans its width.
                (3, [200.0, 500.0], &[("barrier", "gate")]),
                (4, [300.0, 520.0], &[("barrier", "lift_gate")]),
                (5, [310.0, 530.0], &[("barrier", "block")]),
                (6, [320.0, 540.0], &[("barrier", "cycle_barrier")]),
                (7, [330.0, 510.0], &[("power", "pole")]),
                (8, [340.0, 510.0], &[("barrier", "entrance")]),
                // Beyond the corridor (100 m from the road).
                (9, [350.0, 600.0], &[("barrier", "bollard")]),
                // A fence crossing the road.
                (10, [500.0, 450.0], &[]),
                (11, [500.0, 550.0], &[]),
                // A wall beside it.
                (12, [600.0, 510.0], &[]),
                (13, [700.0, 510.0], &[]),
                (14, [600.0, 490.0], &[]),
                (15, [700.0, 490.0], &[]),
                (16, [800.0, 495.0], &[]),
                (17, [900.0, 495.0], &[]),
            ],
            &[
                (100, &[10, 11], &[("barrier", "fence")]),
                (101, &[12, 13], &[("barrier", "wall"), ("height", "2.5")]),
                (102, &[14, 15], &[("barrier", "hedge")]),
                (103, &[16, 17], &[("barrier", "kerb")]),
            ],
        );
        let (o, st) = build_obstacles(&pkg, &d, &proj);
        let t = &o.tiles[&TileKey { x: 0, z: 0 }];
        let kind = |k: ObstacleKind| t.points.iter().filter(|p| p.kind == k).count();
        assert_eq!(kind(ObstacleKind::Bollard), 2, "the far one is left out");
        assert_eq!(kind(ObstacleKind::Gate), 1);
        assert_eq!(kind(ObstacleKind::LiftGate), 1);
        assert_eq!(kind(ObstacleKind::Block), 1);
        assert_eq!(kind(ObstacleKind::CycleBarrier), 1);
        assert_eq!(kind(ObstacleKind::Post), 1);
        assert_eq!(st.outside_corridor, 1);
        let b1 = t.points.iter().find(|p| p.pos[0] == 100.0).unwrap();
        assert_eq!(b1.flags, ROAD_BARRIER | ON_ROAD);
        assert!(!b1.solid(), "never solid on the road (Q7)");
        let b2 = t.points.iter().find(|p| p.pos[0] == 110.0).unwrap();
        assert_eq!(b2.flags, ROAD_BARRIER);
        let gate = t
            .points
            .iter()
            .find(|p| p.kind == ObstacleKind::Gate)
            .unwrap();
        assert!((gate.width_m - 6.0).abs() < 0.05 && gate.angle.abs() < 0.01);
        // Off the road: solid in wild mode.
        let block = t
            .points
            .iter()
            .find(|p| p.kind == ObstacleKind::Block)
            .unwrap();
        assert!(block.flags == 0 && block.solid());
        let pole = t
            .points
            .iter()
            .find(|p| p.kind == ObstacleKind::Post)
            .unwrap();
        assert_eq!(
            pole.flags, 0,
            "a pole beside the road is not a road barrier"
        );
        // The fence is cut where it crosses the road: two pieces, none on it.
        let fences: Vec<&LineObstacle> = t
            .lines
            .iter()
            .filter(|l| l.kind == ObstacleKind::Fence)
            .collect();
        assert_eq!(fences.len(), 2, "{fences:?}");
        for f in &fences {
            for p in &f.points {
                assert!((f64::from(p[1]) - 500.0).abs() >= 2.9, "{p:?}");
            }
        }
        let wall = t
            .lines
            .iter()
            .find(|l| l.kind == ObstacleKind::Wall)
            .unwrap();
        assert!((wall.height_m - 2.5).abs() < 0.05 && wall.points.len() == 2);
        assert_eq!(
            t.lines
                .iter()
                .filter(|l| l.kind == ObstacleKind::Hedge)
                .count(),
            1
        );
        // The kerb is kept (not solid; for the bike damage of K4).
        assert_eq!(
            t.lines
                .iter()
                .filter(|l| l.kind == ObstacleKind::Kerb)
                .count(),
            1
        );
        // Trees: only within the corridor, none on or beside the road.
        let cells = t.tree_cells(125).unwrap();
        let trunks = ocw_world::trees::trunks_from_cells(TileKey { x: 0, z: 0 }, 1000.0, &cells);
        assert!(!trunks.is_empty());
        for (p, _) in &trunks {
            let d = (p[1] - 500.0).abs();
            assert!(d > 3.0 + 3.0 && d < CORRIDOR_M + 8.0, "{p:?}");
        }
        assert!(st.summary().contains("bollard"));
    }

    #[test]
    fn lines_are_cut_where_they_leave_the_outside() {
        let pieces = cut_line(&[[0.0, 0.0], [10.0, 0.0]], |p| !(4.0..6.0).contains(&p[0]));
        assert_eq!(pieces.len(), 2);
        assert!((pieces[0].last().unwrap()[0] - 4.0).abs() < 0.51);
        assert!((pieces[1][0][0] - 6.0).abs() < 0.51);
        assert!(cut_line(&[[0.0, 0.0], [10.0, 0.0]], |_| false).is_empty());
        assert_eq!(
            cut_line(&[[0.0, 0.0], [5.0, 0.0], [5.0, 5.0]], |_| true),
            vec![vec![[0.0, 0.0], [5.0, 0.0], [5.0, 5.0]]]
        );
    }
}
