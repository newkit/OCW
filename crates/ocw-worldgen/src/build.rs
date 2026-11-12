//! OSM ways → road graph (spec §4.4): rideable ways, split at junctions,
//! clipped to the region, stable edge IDs, then cleanup and dead-end flags.

use std::collections::{BTreeMap, HashMap};

use ocw_world::graph::{Edge, EdgeId, RoadGraph, VertexId};
use ocw_world::region::{PackageRegion, in_ring};

use crate::filter::{rideable, width_m};
use crate::osm::{OsmData, Way};
use crate::projection::LocalProjection;

/// A region: a latitude/longitude box, optionally narrowed to polygons
/// (the extract boundary, e.g. a Geofabrik `.poly` file). Roads are cut
/// where they leave it and marked as boundary (spec §4.4, step 4).
#[derive(Debug, Clone, PartialEq)]
pub struct Region {
    pub min_lat: f64,
    pub min_lon: f64,
    pub max_lat: f64,
    pub max_lon: f64,
    /// Outer rings and holes as (lat, lon); empty = the box only.
    pub polygon: Polygon,
    /// Projection origin of the package's local metres; `None` = the box
    /// centre. Packages meant as neighbours can share one (`world-builder
    /// package --origin`), so their metres differ by nothing at all.
    pub origin: Option<(f64, f64)>,
}

/// Rings of a region boundary, (lat, lon) in degrees.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Polygon {
    pub outer: Vec<Vec<(f64, f64)>>,
    pub holes: Vec<Vec<(f64, f64)>>,
}

impl Polygon {
    /// Parses the Osmosis `.poly` format (name line, sections of "lon lat"
    /// lines closed by END, hole sections starting with '!', final END).
    pub fn parse_poly(text: &str) -> Result<Self, String> {
        let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
        lines.next().ok_or("empty .poly file")?;
        let mut poly = Polygon::default();
        while let Some(section) = lines.next() {
            if section == "END" {
                return Ok(poly);
            }
            let hole = section.starts_with('!');
            let mut ring = Vec::new();
            for l in lines.by_ref() {
                if l == "END" {
                    break;
                }
                let mut it = l.split_whitespace().map(str::parse::<f64>);
                match (it.next(), it.next()) {
                    (Some(Ok(lon)), Some(Ok(lat))) => ring.push((lat, lon)),
                    _ => return Err(format!("bad coordinate line: {l}")),
                }
            }
            if hole {
                poly.holes.push(ring);
            } else {
                poly.outer.push(ring);
            }
        }
        Err("missing final END".into())
    }

    pub fn contains(&self, p: (f64, f64)) -> bool {
        self.outer.iter().any(|r| in_ring(r, p)) && !self.holes.iter().any(|r| in_ring(r, p))
    }
}

impl Region {
    /// A plain latitude/longitude box.
    pub fn bbox(min_lat: f64, min_lon: f64, max_lat: f64, max_lon: f64) -> Self {
        Self {
            min_lat,
            min_lon,
            max_lat,
            max_lon,
            polygon: Polygon::default(),
            origin: None,
        }
    }

    /// The region of a polygon: its bounding box plus the polygon.
    pub fn from_polygon(polygon: Polygon) -> Self {
        let pts = polygon.outer.iter().flatten();
        let (mut r, mut any) = (Self::bbox(90.0, 180.0, -90.0, -180.0), false);
        for &(lat, lon) in pts {
            any = true;
            r.min_lat = r.min_lat.min(lat);
            r.max_lat = r.max_lat.max(lat);
            r.min_lon = r.min_lon.min(lon);
            r.max_lon = r.max_lon.max(lon);
        }
        if !any {
            r = Self::bbox(0.0, 0.0, 0.0, 0.0);
        }
        r.polygon = polygon;
        r
    }

    pub fn contains(&self, (lat, lon): (f64, f64)) -> bool {
        (self.min_lat..=self.max_lat).contains(&lat)
            && (self.min_lon..=self.max_lon).contains(&lon)
            && (self.polygon.outer.is_empty() || self.polygon.contains((lat, lon)))
    }

    pub fn center(&self) -> (f64, f64) {
        (
            (self.min_lat + self.max_lat) / 2.0,
            (self.min_lon + self.max_lon) / 2.0,
        )
    }

    /// The projection origin: [`Region::origin`] or the box centre.
    pub fn origin(&self) -> (f64, f64) {
        self.origin.unwrap_or_else(|| self.center())
    }

    pub fn projection(&self) -> LocalProjection {
        let (lat, lon) = self.origin();
        LocalProjection::new(lat, lon)
    }

    /// Narrowed to a box (`--bbox` together with `--poly`).
    pub fn clipped_to(mut self, min_lat: f64, min_lon: f64, max_lat: f64, max_lon: f64) -> Self {
        self.min_lat = self.min_lat.max(min_lat);
        self.min_lon = self.min_lon.max(min_lon);
        self.max_lat = self.max_lat.min(max_lat);
        self.max_lon = self.max_lon.min(max_lon);
        self
    }

    /// Where the segment from `inside` to `outside` (lat, lon) leaves the
    /// region (bisection on [`Region::contains`], so a neighbouring region
    /// with the same border line finds the same point).
    pub fn crossing(&self, inside: (f64, f64), outside: (f64, f64)) -> (f64, f64) {
        let lerp = |t: f64| {
            (
                inside.0 + (outside.0 - inside.0) * t,
                inside.1 + (outside.1 - inside.1) * t,
            )
        };
        let (mut lo, mut hi) = (0.0f64, 1.0f64);
        for _ in 0..60 {
            let mid = 0.5 * (lo + hi);
            if self.contains(lerp(mid)) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        lerp(0.5 * (lo + hi))
    }

    /// What the package stores of the region (section `REGN`).
    pub fn package_region(&self) -> PackageRegion {
        PackageRegion {
            south: self.min_lat,
            west: self.min_lon,
            north: self.max_lat,
            east: self.max_lon,
            outer: self.polygon.outer.clone(),
            holes: self.polygon.holes.clone(),
        }
    }
}

/// Spurs shorter than this are removed (spec §4.4, step 2).
pub const MIN_SPUR_M: f64 = 50.0;

/// Counts of each build step, for the build log.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BuildStats {
    pub highway_ways: usize,
    pub rideable_ways: usize,
    pub raw_edges: usize,
    pub components: usize,
    pub edges: usize,
    pub vertices: usize,
    pub length_km: f64,
    pub dead_end_edges: usize,
    pub boundary_edges: usize,
}

/// 64-bit FNV-1a: deterministic across builds, platforms and Rust versions.
fn fnv1a(words: impl IntoIterator<Item = u64>) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for w in words {
        for b in w.to_le_bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h
}

/// Stable edge ID from the OSM way, its node range and the node sequence
/// (spec §4.4). Moving a node does not change it; re-routing the road does.
pub fn stable_edge_id(way: i64, nodes: &[i64]) -> EdgeId {
    let first = *nodes.first().unwrap_or(&0);
    let last = *nodes.last().unwrap_or(&0);
    let geometry = fnv1a(nodes.iter().map(|&n| n as u64));
    EdgeId(fnv1a([way as u64, first as u64, last as u64, geometry]))
}

/// Two (lat, lon) points less than 0.5 m apart.
fn close(a: (f64, f64), b: (f64, f64)) -> bool {
    let m = 111_195.0;
    let dx = (a.1 - b.1) * m * a.0.to_radians().cos();
    let dz = (a.0 - b.0) * m;
    dx.hypot(dz) < 0.5
}

/// Builds the raw graph: no cleanup yet.
pub fn raw_graph(osm: &OsmData, region: &Region) -> (RoadGraph, BuildStats) {
    let proj = region.projection();
    let mut stats = BuildStats {
        highway_ways: osm.ways.len(),
        ..BuildStats::default()
    };
    let ways: Vec<(&Way, _)> = osm
        .ways
        .iter()
        .filter_map(|w| rideable(w).map(|c| (w, c)))
        .filter(|(w, _)| w.nodes.len() >= 2 && w.nodes.iter().all(|n| osm.nodes.contains_key(n)))
        .collect();
    stats.rideable_ways = ways.len();

    // Junctions: nodes used more than once among rideable ways.
    let mut uses: HashMap<i64, u32> = HashMap::new();
    for (w, _) in &ways {
        for n in &w.nodes {
            *uses.entry(*n).or_default() += 1;
        }
    }

    let mut graph = RoadGraph::default();
    let mut vertex_of: BTreeMap<i64, VertexId> = BTreeMap::new();
    let mut vertex = |graph: &mut RoadGraph, node: i64| -> VertexId {
        *vertex_of.entry(node).or_insert_with(|| {
            let (lat, lon) = osm.nodes[&node];
            graph.vertices.push(proj.project(lat, lon));
            VertexId(graph.vertices.len() as u32 - 1)
        })
    };
    for (w, class) in ways {
        // Runs of consecutive nodes inside the region. A run cut by the
        // border ends exactly where the road crosses it (a new vertex,
        // marked as boundary), so a neighbouring package cut along the same
        // border line has its boundary vertex at the same point (spec
        // §4.4 step 4, §2.2.2).
        let at = |k: usize| osm.nodes[&w.nodes[k]];
        let inside: Vec<bool> = (0..w.nodes.len()).map(|k| region.contains(at(k))).collect();
        let mut i = 0;
        while i < w.nodes.len() {
            if !inside[i] {
                i += 1;
                continue;
            }
            let start = i;
            while i < w.nodes.len() && inside[i] {
                i += 1;
            }
            // (node or `None` for a border crossing, lat/lon, on the border).
            let mut run: Vec<(Option<i64>, (f64, f64), bool)> = Vec::with_capacity(i - start + 2);
            run.extend((start..i).map(|k| (Some(w.nodes[k]), at(k), false)));
            // A node (almost) on the border line is the cut end itself.
            if start > 0 {
                let x = region.crossing(at(start), at(start - 1));
                if close(x, at(start)) {
                    run[0].2 = true;
                } else {
                    run.insert(0, (None, x, true));
                }
            }
            if i < w.nodes.len() {
                let x = region.crossing(at(i - 1), at(i));
                if close(x, at(i - 1)) {
                    run.last_mut().expect("not empty").2 = true;
                } else {
                    run.push((None, x, true));
                }
            }
            if run.len() < 2 {
                continue;
            }
            // Split the run at junctions.
            let mut piece = vec![run[0]];
            for (k, &pt) in run.iter().enumerate().skip(1) {
                piece.push(pt);
                let last = k == run.len() - 1;
                let junction = pt.0.is_some_and(|n| uses.get(&n).copied().unwrap_or(0) > 1);
                if last || junction {
                    let mut end_vertex =
                        |graph: &mut RoadGraph, (node, (lat, lon), border): (Option<i64>, _, bool)| {
                            let v = match node {
                                Some(n) => vertex(graph, n),
                                None => {
                                    graph.vertices.push(proj.project(lat, lon));
                                    VertexId(graph.vertices.len() as u32 - 1)
                                }
                            };
                            if border {
                                graph.boundary_vertices.insert(v);
                            }
                            v
                        };
                    let (from, to) = (end_vertex(&mut graph, piece[0]), end_vertex(&mut graph, pt));
                    let boundary = piece[0].2 || pt.2;
                    // The ID covers the OSM nodes; a cut end counts as a
                    // marker, so the two halves of a road cut in one
                    // segment differ.
                    let nodes: Vec<i64> = piece.iter().map(|p| p.0.unwrap_or(i64::MIN)).collect();
                    graph.edges.push(Edge {
                        id: stable_edge_id(w.id, &nodes),
                        from,
                        to,
                        points: piece
                            .iter()
                            .map(|&(_, (lat, lon), _)| proj.project(lat, lon))
                            .collect(),
                        class,
                        name: w.tag("name").map(str::to_string),
                        width_m: width_m(w, class),
                        bridge: w.tag("bridge").is_some_and(|v| v != "no"),
                        tunnel: w.tag("tunnel").is_some_and(|v| v != "no"),
                        dead_end_m: None,
                        boundary,
                        oneway: crate::filter::oneway(w),
                        osm_way: w.id,
                        profile: Vec::new(),
                    });
                    piece = vec![pt];
                }
            }
        }
    }
    stats.raw_edges = graph.edges.len();
    (graph, stats)
}

/// Full road graph pipeline (spec §4.4, steps 1–4).
pub fn build_road_graph(osm: &OsmData, region: &Region) -> (RoadGraph, BuildStats) {
    let (mut graph, mut stats) = raw_graph(osm, region);
    stats.components = graph.components().len();
    graph.keep_largest_component();
    graph.prune_spurs(MIN_SPUR_M);
    graph.flag_dead_ends();
    stats.edges = graph.edges.len();
    stats.vertices = graph.vertices.len();
    stats.length_km = graph.total_length_m() / 1000.0;
    stats.dead_end_edges = graph
        .edges
        .iter()
        .filter(|e| e.dead_end_m.is_some())
        .count();
    stats.boundary_edges = graph.edges.iter().filter(|e| e.boundary).count();
    (graph, stats)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::osm::Tags;

    /// Grid of nodes: id = 1000 * row + col, ~100 m apart around 47.15 / 9.55.
    fn grid() -> HashMap<i64, (f64, f64)> {
        let mut nodes = HashMap::new();
        for r in 0..10 {
            for c in 0..10 {
                nodes.insert(
                    1000 * r + c,
                    (47.15 + r as f64 * 0.0009, 9.55 + c as f64 * 0.0013),
                );
            }
        }
        nodes
    }

    fn way(id: i64, nodes: &[i64], tags: &[(&str, &str)]) -> Way {
        let mut t: Tags = tags
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        t.entry("highway".into())
            .or_insert_with(|| "residential".into());
        Way {
            id,
            nodes: nodes.to_vec(),
            tags: t,
        }
    }

    fn region() -> Region {
        Region::bbox(47.0, 9.0, 48.0, 10.0)
    }

    #[test]
    fn ways_are_split_at_shared_nodes_only() {
        // A horizontal road through row 5 and a vertical one through col 5,
        // crossing at node 5005; a third way shares no node.
        let osm = OsmData {
            nodes: grid(),
            ways: vec![
                way(1, &[5000, 5001, 5002, 5003, 5004, 5005, 5006, 5007], &[]),
                way(2, &[1005, 2005, 3005, 4005, 5005, 6005, 7005], &[]),
            ],
        };
        let (g, stats) = raw_graph(&osm, &region());
        assert_eq!(stats.rideable_ways, 2);
        assert_eq!(g.edges.len(), 4);
        assert_eq!(g.vertices.len(), 5);
        // Intermediate shape nodes stay in the geometry.
        assert_eq!(g.edges[0].points.len(), 6);
        assert!(g.edges.iter().all(|e| !e.boundary));
    }

    #[test]
    fn full_pipeline_cleans_up_and_flags() {
        let osm = OsmData {
            nodes: grid(),
            ways: vec![
                // A loop (block) of residential streets.
                way(1, &[1001, 1002, 1003, 1004], &[]),
                way(2, &[1004, 2004, 3004, 4004], &[]),
                way(3, &[4004, 4003, 4002, 4001], &[]),
                way(4, &[4001, 3001, 2001, 1001], &[]),
                // A 500 m valley road leaving the block: a dead end.
                way(
                    5,
                    &[2004, 2005, 2006, 2007, 2008, 2009],
                    &[("name", "Valley Road")],
                ),
                // A driveway (excluded) and a separate fragment (dropped).
                way(
                    6,
                    &[3001, 3000],
                    &[("highway", "service"), ("service", "driveway")],
                ),
                way(7, &[8000, 8001, 8002], &[]),
                // A bridge on the block.
                way(8, &[4001, 5001], &[("bridge", "yes")]),
            ],
        };
        let (g, stats) = build_road_graph(&osm, &region());
        assert_eq!(stats.highway_ways, 8);
        assert_eq!(stats.rideable_ways, 7);
        assert_eq!(stats.components, 2);
        // The ~100 m spur 4001→5001 is longer than 50 m: kept, a dead end.
        let valley = g
            .edges
            .iter()
            .find(|e| e.name.as_deref() == Some("Valley Road"))
            .unwrap();
        let len = valley.length_m();
        assert!((450.0..550.0).contains(&len), "{len}");
        assert_eq!(valley.dead_end_m, Some(len));
        assert!(g.edges.iter().any(|e| e.bridge && e.dead_end_m.is_some()));
        // The block: way 1, way 2 (split at the valley road), ways 3 and 4.
        assert_eq!(g.edges.iter().filter(|e| e.dead_end_m.is_none()).count(), 5);
        assert!(g.edges.iter().all(|e| e.osm_way != 7 && e.osm_way != 6));
    }

    #[test]
    fn roads_leaving_the_region_are_cut_and_marked() {
        let osm = OsmData {
            nodes: grid(),
            ways: vec![way(
                1,
                &[5000, 5001, 5002, 5003, 5004, 5005, 5006, 5007, 5008, 5009],
                &[],
            )],
        };
        // Region ends between column 4 and 5.
        let r = Region::bbox(47.0, 9.0, 48.0, 9.55 + 4.5 * 0.0013);
        let (g, _) = build_road_graph(&osm, &r);
        assert_eq!(g.edges.len(), 1);
        let e = &g.edges[0];
        assert!(e.boundary);
        // Five nodes inside plus the point where the road crosses the border.
        assert_eq!(e.points.len(), 6);
        let v = *g.boundary_vertices.iter().next().unwrap();
        let cut = r.projection().unproject(g.vertices[v.0 as usize]);
        assert!((cut.1 - r.max_lon).abs() < 1e-9, "{cut:?} vs {}", r.max_lon);
        assert!(e.from == v || e.to == v);
        // A road into the neighbouring region is not a dead end at the border.
        assert!(e.dead_end_m.is_some(), "the far (inner) end is a real end");
    }

    #[test]
    fn neighbouring_regions_cut_a_road_at_the_same_point() {
        // Way 1 along row 5, way 2 diagonal; the border between the
        // regions lies between columns 4 and 5, both share the origin.
        let osm = OsmData {
            nodes: grid(),
            ways: vec![
                way(
                    1,
                    &[5000, 5001, 5002, 5003, 5004, 5005, 5006, 5007, 5008, 5009],
                    &[],
                ),
                way(2, &[1002, 2003, 3004, 4005, 5006], &[]),
            ],
        };
        let border = 9.55 + 4.5 * 0.0013;
        let origin = Some((47.15, 9.55));
        let west = Region {
            origin,
            ..Region::bbox(47.0, 9.0, 48.0, border)
        };
        let east = Region {
            origin,
            ..Region::bbox(47.0, border, 48.0, 10.0)
        };
        let (gw, _) = raw_graph(&osm, &west);
        let (ge, _) = raw_graph(&osm, &east);
        let ends = |g: &RoadGraph| -> Vec<[f64; 2]> {
            g.boundary_vertices
                .iter()
                .map(|v| g.vertices[v.0 as usize])
                .collect()
        };
        let (mut a, mut b) = (ends(&gw), ends(&ge));
        assert_eq!((a.len(), b.len()), (2, 2), "both roads cross");
        a.sort_by(|p, q| p[1].total_cmp(&q[1]));
        b.sort_by(|p, q| p[1].total_cmp(&q[1]));
        for (p, q) in a.iter().zip(&b) {
            assert!(
                (p[0] - q[0]).abs() < 1e-6 && (p[1] - q[1]).abs() < 1e-6,
                "{p:?} {q:?}"
            );
        }
        // The two halves are different edges (different stable IDs).
        let ids: BTreeSet<EdgeId> = gw.edges.iter().chain(&ge.edges).map(|e| e.id).collect();
        assert_eq!(ids.len(), gw.edges.len() + ge.edges.len());
        // A road cut in a single segment: still two different IDs.
        let osm = OsmData {
            nodes: grid(),
            ways: vec![way(3, &[7004, 7005], &[])],
        };
        let (gw, _) = raw_graph(&osm, &west);
        let (ge, _) = raw_graph(&osm, &east);
        assert_eq!((gw.edges.len(), ge.edges.len()), (1, 1));
        assert_ne!(gw.edges[0].id, ge.edges[0].id);
        assert_eq!(gw.edges[0].points.len(), 2);
    }

    #[test]
    fn a_node_on_the_border_is_the_cut_end() {
        let osm = OsmData {
            nodes: grid(),
            ways: vec![way(1, &[5000, 5001, 5002, 5003], &[])],
        };
        // Exactly through node 5002 (inclusive on both sides).
        let border = 9.55 + 2.0 * 0.0013;
        let west = Region::bbox(47.0, 9.0, 48.0, border);
        let east = Region::bbox(47.0, border, 48.0, 10.0);
        let (gw, _) = raw_graph(&osm, &west);
        let (ge, _) = raw_graph(&osm, &east);
        assert_eq!(gw.vertices.len(), 2, "no extra vertex next to node 5002");
        assert_eq!(gw.edges[0].points.len(), 3);
        assert_eq!(ge.edges[0].points.len(), 2);
        assert_eq!(gw.boundary_vertices.len(), 1);
        assert_eq!(ge.boundary_vertices.len(), 1);
        assert!(gw.edges[0].boundary && ge.edges[0].boundary);
    }

    #[test]
    fn regions_keep_an_origin_and_export_their_bounds() {
        let r = Region::bbox(47.0, 9.0, 48.0, 10.0);
        assert_eq!(r.origin(), (47.5, 9.5));
        let r = Region {
            origin: Some((47.2, 9.2)),
            ..r
        }
        .clipped_to(47.1, 8.0, 49.0, 9.8);
        assert_eq!(r.origin(), (47.2, 9.2));
        assert_eq!(r.projection().project(47.2, 9.2), [0.0, 0.0]);
        let p = r.package_region();
        assert_eq!(p.bounds(), [47.1, 9.0, 48.0, 9.8]);
        for q in [(47.05, 9.5), (47.5, 9.5), (47.5, 9.9)] {
            assert_eq!(p.contains(q), r.contains(q), "{q:?}");
        }
    }

    #[test]
    fn edge_ids_are_stable_across_unrelated_changes() {
        let base = vec![
            way(1, &[5000, 5001, 5002, 5003, 5004, 5005, 5006], &[]),
            way(2, &[2003, 3003, 4003, 5003, 6003], &[]),
        ];
        let ids = |ways: Vec<Way>| -> Vec<EdgeId> {
            let osm = OsmData {
                nodes: grid(),
                ways,
            };
            let mut ids: Vec<EdgeId> = raw_graph(&osm, &region())
                .0
                .edges
                .iter()
                .map(|e| e.id)
                .collect();
            ids.sort();
            ids
        };
        let before = ids(base.clone());
        assert_eq!(before, ids(base.clone()), "deterministic");
        // An unrelated new road elsewhere: existing IDs unchanged.
        let mut more = base.clone();
        more.push(way(3, &[8000, 8001, 8002], &[]));
        let after = ids(more);
        assert!(before.iter().all(|id| after.contains(id)));
        // A new road joining way 1 at node 5005 splits that edge: only the
        // split edge gets new IDs.
        let mut split = base.clone();
        split.push(way(4, &[5005, 6005, 7005], &[]));
        let after = ids(split);
        let kept = before.iter().filter(|id| after.contains(id)).count();
        assert_eq!(kept, before.len() - 1);
        // Moving a node does not change any ID.
        let mut moved = grid();
        moved.insert(5001, (47.1545, 9.5514));
        let osm = OsmData {
            nodes: moved,
            ways: base,
        };
        let mut ids_moved: Vec<EdgeId> = raw_graph(&osm, &region())
            .0
            .edges
            .iter()
            .map(|e| e.id)
            .collect();
        ids_moved.sort();
        assert_eq!(ids_moved, before);
    }

    #[test]
    fn fnv_reference_values() {
        // FNV-1a of no input is the offset basis.
        assert_eq!(fnv1a([]), 0xcbf2_9ce4_8422_2325);
        assert_ne!(stable_edge_id(1, &[1, 2]), stable_edge_id(1, &[2, 1]));
        assert_ne!(stable_edge_id(1, &[1, 2]), stable_edge_id(2, &[1, 2]));
    }

    const POLY: &str = "test\n1\n  9.0 47.0\n  10.0 47.0\n  10.0 48.0\n  9.0 48.0\nEND\n!2\n  9.4 47.4\n  9.6 47.4\n  9.6 47.6\n  9.4 47.6\nEND\nEND\n";

    #[test]
    fn poly_files_and_point_in_polygon() {
        let p = Polygon::parse_poly(POLY).unwrap();
        assert_eq!((p.outer.len(), p.holes.len()), (1, 1));
        assert_eq!(p.outer[0][1], (47.0, 10.0));
        assert!(p.contains((47.2, 9.2)));
        assert!(!p.contains((47.5, 9.5)), "inside the hole");
        assert!(!p.contains((46.9, 9.5)));
        let r = Region::from_polygon(p);
        assert_eq!(
            (r.min_lat, r.max_lat, r.min_lon, r.max_lon),
            (47.0, 48.0, 9.0, 10.0)
        );
        assert!(!r.contains((47.5, 9.5)));
        assert!(Polygon::parse_poly("x\n1\n 9 47\n").is_err());
        assert!(Polygon::parse_poly("x\n1\n nine 47\nEND\nEND").is_err());
    }

    #[test]
    fn roads_are_cut_at_a_polygon_border() {
        // Border with a slightly slanted east edge between columns 2 and 3
        // (lon 9.5526 … 9.5539) of the row-1 road.
        let tri = Polygon {
            outer: vec![vec![
                (47.0, 9.0),
                (47.0, 9.5535),
                (47.3, 9.5530),
                (47.3, 9.0),
            ]],
            holes: vec![],
        };
        let osm = OsmData {
            nodes: grid(),
            ways: vec![way(
                1,
                &[1000, 1001, 1002, 1003, 1004, 1005, 1006, 1007],
                &[],
            )],
        };
        let (g, _) = build_road_graph(&osm, &Region::from_polygon(tri));
        assert_eq!(g.edges.len(), 1);
        assert!(g.edges[0].boundary);
        // Three nodes and the crossing on the slanted edge.
        assert_eq!(g.edges[0].points.len(), 4);
        assert_eq!(g.boundary_vertices.len(), 1);
    }
}
