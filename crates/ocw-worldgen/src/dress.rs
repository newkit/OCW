//! The road dress from OSM (spec §14 Phase 8 T5): lanes, markings, cycle
//! lanes, sidewalks and kerbs per way ([`ocw_world::RoadDress::from_osm`]),
//! sidewalks mapped as separate ways beside a road (`footway=sidewalk`),
//! and the crossings, stop and give-way lines at way nodes, put on the road
//! strips of a package ([`apply_road_dress`], section `RDRS`).

use std::collections::HashMap;

use ocw_world::geo::LocalProjection;
use ocw_world::graph::{RoadClass, RoadGraph};
use ocw_world::package::WorldPackage;
use ocw_world::road_dress::{
    Crossing, CrossingKind, DEFAULT_SIDEWALK_M, Half, RoadDress, SidewalkTag, sidewalk_tags,
};

use crate::osm::{OsmData, Way};
use crate::spatial::foot_on_segment;

/// The dress of one OSM way: what its tags say, and its sidewalk tags (the
/// untagged sides may still get a separately mapped sidewalk).
#[derive(Debug, Clone, PartialEq)]
pub struct WayDress {
    pub dress: RoadDress,
    pub sidewalks: [SidewalkTag; 2],
}

/// Roads with a carriageway for motor traffic (they get crossings,
/// kerbs and sidewalks).
fn carriageway(class: RoadClass) -> bool {
    !matches!(
        class,
        RoadClass::Track | RoadClass::Cycleway | RoadClass::Path
    )
}

/// The dress of a rideable way from its tags.
pub fn way_dress(way: &Way, class: RoadClass) -> WayDress {
    let width = crate::filter::width_m(way, class) as f32;
    let oneway = crate::filter::oneway(way);
    let tag = |k: &str| way.tag(k);
    WayDress {
        dress: RoadDress::from_osm(class, width, oneway, tag),
        sidewalks: if carriageway(class) {
            sidewalk_tags(tag)
        } else {
            [SidewalkTag::No; 2]
        },
    }
}

/// The dress of every rideable way, by OSM way id.
pub fn way_dresses(osm: &OsmData) -> HashMap<i64, WayDress> {
    osm.ways
        .iter()
        .filter_map(|w| Some((w.id, way_dress(w, crate::filter::rideable(w)?))))
        .collect()
}

/// Sidewalks mapped as separate ways (`footway=sidewalk`, not rideable)
/// in local metres.
pub fn sidewalk_ways(osm: &OsmData, proj: &LocalProjection) -> Vec<Vec<[f64; 2]>> {
    osm.ways
        .iter()
        .filter(|w| {
            w.tag("footway") == Some("sidewalk")
                && matches!(w.tag("highway"), Some("footway" | "path" | "pedestrian"))
                && crate::filter::rideable(w).is_none()
        })
        .map(|w| {
            w.nodes
                .iter()
                .filter_map(|n| osm.nodes.get(n))
                .map(|&(lat, lon)| proj.project(lat, lon))
                .collect::<Vec<_>>()
        })
        .filter(|p| p.len() >= 2)
        .collect()
}

/// Farthest a separate sidewalk's centreline lies beyond the road edge (m).
pub const SEPARATE_REACH_M: f64 = 6.0;
/// Share of the samples along a road that must see the separate sidewalk.
const SEPARATE_SHARE: f64 = 0.4;
/// Samples along a road for the separate sidewalks (m).
const SEPARATE_STEP_M: f64 = 8.0;

/// A line segment (two points, local metres).
type Segment = ([f64; 2], [f64; 2]);

/// A grid of line segments for "which sidewalk ways are near" queries.
struct Segments {
    bucket: f64,
    cells: HashMap<(i64, i64), Vec<Segment>>,
}

impl Segments {
    fn new(lines: &[Vec<[f64; 2]>], bucket: f64) -> Self {
        let mut cells: HashMap<(i64, i64), Vec<Segment>> = HashMap::new();
        let k = |v: f64| (v / bucket).floor() as i64;
        for l in lines {
            for w in l.windows(2) {
                let (a, b) = (w[0], w[1]);
                for x in k(a[0].min(b[0]))..=k(a[0].max(b[0])) {
                    for z in k(a[1].min(b[1]))..=k(a[1].max(b[1])) {
                        cells.entry((x, z)).or_default().push((a, b));
                    }
                }
            }
        }
        Self { bucket, cells }
    }

    fn near(&self, p: [f64; 2], r: f64) -> impl Iterator<Item = &Segment> {
        let k = |v: f64| (v / self.bucket).floor() as i64;
        let (x0, x1, z0, z1) = (k(p[0] - r), k(p[0] + r), k(p[1] - r), k(p[1] + r));
        (x0..=x1)
            .flat_map(move |x| (z0..=z1).map(move |z| (x, z)))
            .filter_map(|c| self.cells.get(&c))
            .flatten()
    }
}

/// The width of a sidewalk mapped as a separate way on each side
/// ([left, right]) of `points` (a road `width` metres wide), if one runs
/// beside it for most of its length: the sidewalk reaches from the kerb
/// to just beyond the separate way's centreline.
fn separate_beside(points: &[[f64; 2]], width: f64, sidewalks: &Segments) -> [Option<f32>; 2] {
    let half = width / 2.0;
    let samples = crate::profile::sample_along(points, SEPARATE_STEP_M, false);
    let mut found: [Vec<f64>; 2] = [Vec::new(), Vec::new()];
    let n = samples.len();
    for (i, (p, _)) in samples.iter().enumerate() {
        let (a, b) = (
            samples[i.saturating_sub(1)].0,
            samples[(i + 1).min(n - 1)].0,
        );
        let d = [b[0] - a[0], b[1] - a[1]];
        let len = d[0].hypot(d[1]);
        if len < 1e-6 {
            continue;
        }
        let dir = [d[0] / len, d[1] / len];
        // Right of the direction (x east, z south): (-dz, dx).
        let right = [-dir[1], dir[0]];
        let mut best = [f64::INFINITY; 2];
        for &(s0, s1) in sidewalks.near(*p, half + SEPARATE_REACH_M) {
            let (dist, _, foot) = foot_on_segment(*p, s0, s1);
            let off = dist - half;
            if !(0.3..=SEPARATE_REACH_M).contains(&off) {
                continue;
            }
            let sd = [s1[0] - s0[0], s1[1] - s0[1]];
            let sl = sd[0].hypot(sd[1]);
            if sl < 1e-6 || (sd[0] * dir[0] + sd[1] * dir[1]).abs() / sl < 0.85 {
                continue;
            }
            let side = usize::from((foot[0] - p[0]) * right[0] + (foot[1] - p[1]) * right[1] > 0.0);
            best[side] = best[side].min(dist);
        }
        for s in 0..2 {
            if best[s].is_finite() {
                found[s].push(best[s]);
            }
        }
    }
    let need = (n as f64 * SEPARATE_SHARE).max(1.0);
    found.map(|mut d| {
        if (d.len() as f64) < need {
            return None;
        }
        d.sort_by(f64::total_cmp);
        let median = d[d.len() / 2];
        Some(((median - half + 0.8) as f32).clamp(1.2, 5.0))
    })
}

/// A transverse marking of an edge: at `point` (local metres, on or near
/// the edge), moved `shift_m` along the direction of travel of its half.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EdgeCrossing {
    pub edge: usize,
    pub point: [f64; 2],
    pub kind: CrossingKind,
    pub half: Half,
    pub shift_m: f64,
}

/// The crossings, stop and give-way lines at the way nodes of `osm`, on the
/// edges of `graph` (roads with a carriageway only: a zebra where a
/// footway crosses a road is painted on the road).
pub fn edge_crossings(
    osm: &OsmData,
    graph: &RoadGraph,
    proj: &LocalProjection,
) -> Vec<EdgeCrossing> {
    let mut by_way: HashMap<i64, Vec<usize>> = HashMap::new();
    for (i, e) in graph.edges.iter().enumerate() {
        if !e.tunnel && carriageway(e.class) {
            by_way.entry(e.osm_way).or_default().push(i);
        }
    }
    // The ways through each dress node.
    let mut ways_at: HashMap<i64, Vec<&Way>> = HashMap::new();
    for w in &osm.ways {
        if !by_way.contains_key(&w.id) {
            continue;
        }
        for n in &w.nodes {
            if osm.node_tags.contains_key(n) {
                ways_at.entry(*n).or_default().push(w);
            }
        }
    }
    let mut nodes: Vec<(&i64, &Vec<&Way>)> = ways_at.iter().collect();
    nodes.sort_by_key(|(n, _)| **n);
    let mut out = Vec::new();
    for (node, ways) in nodes {
        let tags = &osm.node_tags[node];
        let tag = |k: &str| tags.get(k).map(String::as_str);
        let Some(kind) = CrossingKind::from_osm(tag) else {
            continue;
        };
        let Some(&(lat, lon)) = osm.nodes.get(node) else {
            continue;
        };
        let p = proj.project(lat, lon);
        // The nearest edge of each way through the node.
        let mut edges: Vec<(usize, f64, f64, f64)> = Vec::new(); // edge, dist, along, length
        for w in ways {
            let mut best: Option<(usize, f64, f64, f64)> = None;
            for &ei in &by_way[&w.id] {
                let e = &graph.edges[ei];
                let (mut walked, mut hit) = (0.0, (f64::INFINITY, 0.0));
                for s in e.points.windows(2) {
                    let (d, t, _) = foot_on_segment(p, s[0], s[1]);
                    let l = (s[1][0] - s[0][0]).hypot(s[1][1] - s[0][1]);
                    if d < hit.0 {
                        hit = (d, walked + t * l);
                    }
                    walked += l;
                }
                if hit.0 <= e.width_m / 2.0 + 2.0 && best.is_none_or(|b| hit.0 < b.1) {
                    best = Some((ei, hit.0, hit.1, walked));
                }
            }
            edges.extend(best);
        }
        if edges.is_empty() {
            continue;
        }
        let stop_like = matches!(kind, CrossingKind::Stop | CrossingKind::GiveWay);
        // A stop or give-way node at a junction belongs to the minor roads.
        if stop_like {
            let top = edges
                .iter()
                .map(|e| graph.edges[e.0].class)
                .min()
                .expect("not empty");
            if edges.iter().any(|e| graph.edges[e.0].class != top) {
                edges.retain(|e| graph.edges[e.0].class != top);
            }
        }
        for (ei, _, along, len) in edges {
            let (half, shift_m) = if !stop_like {
                (Half::Both, 0.0)
            } else {
                match tag("direction").or_else(|| tag("traffic_signals:direction")) {
                    Some("forward") => (Half::Forward, 0.0),
                    Some("backward") => (Half::Backward, 0.0),
                    // Traffic meets the line on its way to the nearer end
                    // (the junction); at the end itself, the line stands
                    // before the crossing road.
                    _ => {
                        let to_end = len - along;
                        let at_end = to_end.min(along) < 1.0;
                        let back = if at_end { 3.0 } else { 0.0 };
                        if to_end <= along {
                            (Half::Forward, -back)
                        } else {
                            (Half::Backward, -back)
                        }
                    }
                }
            };
            out.push(EdgeCrossing {
                edge: ei,
                point: p,
                kind,
                half,
                shift_m,
            });
        }
    }
    out
}

/// What [`apply_road_dress`] put on the strips.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DressStats {
    pub strips: usize,
    pub marked: usize,
    pub with_sidewalks: usize,
    pub cycle_lanes: usize,
    pub crossings: usize,
    pub crossings_missed: usize,
}

/// The dress of each edge of `graph` from its OSM way (`ways`, by OSM way
/// id; edges without one keep their class default) with the separately
/// mapped `sidewalks` on untagged sides.
pub fn edge_dresses(
    graph: &RoadGraph,
    ways: &HashMap<i64, WayDress>,
    sidewalks: &[Vec<[f64; 2]>],
) -> (Vec<Option<RoadDress>>, usize) {
    let index = Segments::new(sidewalks, 50.0);
    let mut separate = 0;
    let out = graph
        .edges
        .iter()
        .map(|e| {
            let w = ways.get(&e.osm_way)?;
            let mut d = w.dress.clone();
            if carriageway(e.class) && !sidewalks.is_empty() {
                let wants = w
                    .sidewalks
                    .map(|t| matches!(t, SidewalkTag::Unknown | SidewalkTag::Separate));
                if wants.iter().any(|x| *x) {
                    let found = separate_beside(&e.points, e.width_m, &index);
                    for s in 0..2 {
                        if let (true, Some(width)) = (wants[s], found[s]) {
                            d.sidewalk_m[s] = (width * 10.0).round() / 10.0;
                            separate += 1;
                        } else if w.sidewalks[s] == SidewalkTag::Separate {
                            d.sidewalk_m[s] = DEFAULT_SIDEWALK_M;
                        }
                    }
                }
            }
            Some(d)
        })
        .collect();
    (out, separate)
}

/// Puts the dress on the road strips of `pkg` (built from `graph` with
/// [`crate::package::build_package`]): the tagged parts of each edge's
/// dress (`edges`, by edge index; `None` keeps the class default), keeping
/// the strips' geometry (arc length, junction zones), and the crossings on
/// the strip of their edge nearest to them.
pub fn apply_road_dress(
    pkg: &mut WorldPackage,
    graph: &RoadGraph,
    edges: &[Option<RoadDress>],
    crossings: &[EdgeCrossing],
) -> DressStats {
    let mut stats = DressStats::default();
    let mut strips_of: HashMap<usize, Vec<(ocw_world::TileKey, usize)>> = HashMap::new();
    for (key, strips) in crate::package::road_strips(graph, f64::from(pkg.tile_size_m)) {
        let Some(tile) = pkg.tiles.get_mut(&key) else {
            continue;
        };
        assert_eq!(
            tile.roads.len(),
            strips.len(),
            "the package was not built from this graph"
        );
        for (i, (road, (ei, _))) in tile.roads.iter_mut().zip(strips).enumerate() {
            strips_of.entry(ei).or_default().push((key, i));
            stats.strips += 1;
            if let Some(d) = edges.get(ei).and_then(Option::as_ref) {
                let geometry = (road.dress.start_m, road.dress.edge_m, road.dress.junction_m);
                road.dress = RoadDress {
                    start_m: geometry.0,
                    edge_m: geometry.1,
                    junction_m: geometry.2,
                    crossings: Vec::new(),
                    ..d.clone()
                };
            }
            let d = &road.dress;
            stats.marked += usize::from(d.centre != Default::default() || d.edge_lines);
            stats.with_sidewalks += usize::from(d.town());
            stats.cycle_lanes += usize::from(d.cycle.iter().any(|c| *c != Default::default()));
        }
    }
    for c in crossings {
        // The strip of the edge whose foot point is farthest inside it.
        let mut best: Option<(ocw_world::TileKey, usize, f32, f32)> = None;
        for &(key, i) in strips_of.get(&c.edge).map_or(&[][..], Vec::as_slice) {
            let strip = &pkg.tiles[&key].roads[i];
            let Some((along, len, dist)) = foot_along(&strip.points, c.point) else {
                continue;
            };
            if dist > strip.width_m / 2.0 + 2.0 {
                continue;
            }
            // The overlay is centred on the line; a stop line before the
            // node in the direction of travel of its half.
            let dir = if c.half == Half::Backward { -1.0 } else { 1.0 };
            let at = along + dir * c.shift_m as f32;
            let inside = at.min(len - at);
            if inside >= 0.0 && best.is_none_or(|b| inside > b.3) {
                best = Some((key, i, at, inside));
            }
        }
        match best {
            Some((key, i, at, _)) => {
                let strip = &mut pkg.tiles.get_mut(&key).expect("tile").roads[i];
                strip.dress.crossings.push(Crossing {
                    along_m: at,
                    kind: c.kind,
                    half: c.half,
                });
                stats.crossings += 1;
            }
            None => stats.crossings_missed += 1,
        }
    }
    stats
}

/// The foot of `p` on a strip's points: (arc length along the drawn
/// points in 3D, the strip's length, the horizontal distance).
fn foot_along(points: &[[f32; 3]], p: [f64; 2]) -> Option<(f32, f32, f32)> {
    let mut walked = 0.0f64;
    let mut best: Option<(f64, f64)> = None;
    for w in points.windows(2) {
        let (a, b) = (w[0].map(f64::from), w[1].map(f64::from));
        let (d, t, _) = foot_on_segment(p, [a[0], a[2]], [b[0], b[2]]);
        let l = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2) + (b[2] - a[2]).powi(2)).sqrt();
        if best.is_none_or(|x| d < x.0) {
            best = Some((d, walked + t * l));
        }
        walked += l;
    }
    best.map(|(d, a)| (a as f32, walked as f32, d as f32))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::osm::Tags;
    use ocw_world::graph::{Edge, EdgeId, Oneway, VertexId};
    use ocw_world::road_dress::CentreLine;

    fn way(id: i64, nodes: &[i64], tags: &[(&str, &str)]) -> Way {
        Way {
            id,
            nodes: nodes.to_vec(),
            tags: tags
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<Tags>(),
        }
    }

    fn edge(id: u64, way: i64, pts: Vec<[f64; 2]>, class: RoadClass, from: u32, to: u32) -> Edge {
        Edge {
            id: EdgeId(id),
            from: VertexId(from),
            to: VertexId(to),
            points: pts,
            class,
            name: None,
            width_m: class.default_width_m(),
            bridge: false,
            tunnel: false,
            dead_end_m: None,
            boundary: false,
            oneway: Oneway::No,
            osm_way: way,
            profile: Vec::new(),
        }
    }

    #[test]
    fn way_tags_become_the_dress() {
        let w = way(
            1,
            &[1, 2],
            &[
                ("highway", "secondary"),
                ("lanes", "2"),
                ("overtaking", "no"),
                ("sidewalk", "right"),
                ("cycleway:left", "lane"),
            ],
        );
        let d = way_dress(&w, RoadClass::Secondary);
        assert_eq!(d.dress.centre, CentreLine::Solid);
        assert_eq!(d.dress.sidewalk_m, [0.0, DEFAULT_SIDEWALK_M]);
        assert_eq!(d.sidewalks[0], SidewalkTag::No);
        assert_eq!(d.dress.cycle[0], ocw_world::road_dress::CycleLane::Lane);
        // Tracks never get sidewalks.
        let t = way_dress(
            &way(2, &[1, 2], &[("highway", "track"), ("sidewalk", "both")]),
            RoadClass::Track,
        );
        assert_eq!(t.sidewalks, [SidewalkTag::No; 2]);
        assert_eq!(t.dress.sidewalk_m, [0.0; 2]);
    }

    #[test]
    fn separate_sidewalks_beside_a_road_are_found_on_their_side() {
        // A residential road along +x (z = 0); a sidewalk way 4 m to the
        // south (+z, the right side going east), parallel, most of the way;
        // a crossing footway at right angles is no sidewalk.
        let road = vec![[0.0, 0.0], [100.0, 0.0]];
        let sw = vec![
            vec![[5.0, 4.0], [95.0, 4.0]],
            vec![[50.0, -30.0], [50.0, 30.0]],
        ];
        let idx = Segments::new(&sw, 50.0);
        let found = separate_beside(&road, 5.0, &idx);
        assert_eq!(found[0], None);
        let w = found[1].expect("right sidewalk");
        assert!((w - (4.0 - 2.5 + 0.8)).abs() < 1e-5, "{w}");
        // Too far away: a footway 20 m off is not this road's sidewalk.
        let far = Segments::new(&[vec![[0.0, 20.0], [100.0, 20.0]]], 50.0);
        assert_eq!(separate_beside(&road, 5.0, &far), [None, None]);
        // Through edge_dresses: only untagged or `separate` sides take it.
        let g = RoadGraph {
            vertices: vec![[0.0, 0.0], [100.0, 0.0]],
            edges: vec![edge(1, 7, road.clone(), RoadClass::Residential, 0, 1)],
            ..Default::default()
        };
        let mut ways = HashMap::new();
        ways.insert(
            7,
            way_dress(
                &way(
                    7,
                    &[1, 2],
                    &[("highway", "residential"), ("sidewalk:right", "no")],
                ),
                RoadClass::Residential,
            ),
        );
        let (d, n) = edge_dresses(&g, &ways, &sw);
        assert_eq!((d[0].as_ref().unwrap().sidewalk_m, n), ([0.0, 0.0], 0));
        ways.insert(
            7,
            way_dress(
                &way(7, &[1, 2], &[("highway", "residential")]),
                RoadClass::Residential,
            ),
        );
        let (d, n) = edge_dresses(&g, &ways, &sw);
        assert_eq!(n, 1);
        assert!(d[0].as_ref().unwrap().sidewalk_m[1] > 2.0);
    }

    /// Two crossing roads; a zebra mid-block on the main road, a stop sign
    /// at the junction node and a give-way node on the side road near it.
    fn crossing_world() -> (OsmData, RoadGraph, LocalProjection) {
        let proj = LocalProjection::new(47.0, 9.0);
        let ll = |x: f64, z: f64| proj.unproject([x, z]);
        let mut osm = OsmData::default();
        let pts = [
            (1, 0.0, 0.0),
            (2, 50.0, 0.0),
            (3, 100.0, 0.0),
            (4, 50.0, -60.0),
            (5, 20.0, 0.0),
            (6, 50.0, -10.0),
        ];
        for (id, x, z) in pts {
            osm.nodes.insert(id, ll(x, z));
        }
        osm.ways
            .push(way(10, &[1, 5, 2, 3], &[("highway", "primary")]));
        osm.ways
            .push(way(11, &[4, 6, 2], &[("highway", "residential")]));
        let t = |p: &[(&str, &str)]| -> Tags {
            p.iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        };
        osm.node_tags
            .insert(5, t(&[("highway", "crossing"), ("crossing", "zebra")]));
        osm.node_tags.insert(2, t(&[("highway", "stop")]));
        osm.node_tags.insert(6, t(&[("highway", "give_way")]));
        let g = RoadGraph {
            vertices: vec![[0.0, 0.0], [50.0, 0.0], [100.0, 0.0], [50.0, -60.0]],
            edges: vec![
                edge(
                    1,
                    10,
                    vec![[0.0, 0.0], [20.0, 0.0], [50.0, 0.0]],
                    RoadClass::Primary,
                    0,
                    1,
                ),
                edge(
                    2,
                    10,
                    vec![[50.0, 0.0], [100.0, 0.0]],
                    RoadClass::Primary,
                    1,
                    2,
                ),
                edge(
                    3,
                    11,
                    vec![[50.0, -60.0], [50.0, -10.0], [50.0, 0.0]],
                    RoadClass::Residential,
                    3,
                    1,
                ),
            ],
            ..Default::default()
        };
        (osm, g, proj)
    }

    #[test]
    fn crossings_are_placed_on_the_roads_through_their_nodes() {
        let (osm, g, proj) = crossing_world();
        let c = edge_crossings(&osm, &g, &proj);
        let zebra: Vec<_> = c.iter().filter(|c| c.kind == CrossingKind::Zebra).collect();
        assert_eq!(zebra.len(), 1);
        assert_eq!((zebra[0].edge, zebra[0].half), (0, Half::Both));
        assert!((zebra[0].point[0] - 20.0).abs() < 0.01);
        // The stop at the junction node: only on the minor road, before it.
        let stop: Vec<_> = c.iter().filter(|c| c.kind == CrossingKind::Stop).collect();
        assert_eq!(stop.len(), 1);
        assert_eq!((stop[0].edge, stop[0].half), (2, Half::Forward));
        assert!(stop[0].shift_m < 0.0);
        // Give way mid-edge, nearer its `to` end: the forward half.
        let gw: Vec<_> = c
            .iter()
            .filter(|c| c.kind == CrossingKind::GiveWay)
            .collect();
        assert_eq!(
            (gw[0].edge, gw[0].half, gw[0].shift_m),
            (2, Half::Forward, 0.0)
        );
    }

    #[test]
    fn dress_lands_on_the_strips_with_the_crossings() {
        let (osm, g, proj) = crossing_world();
        let mut pkg =
            crate::package::build_package("t", (47.0, 9.0), &g, |_, _| 100.0, 1000.0, 11, 0);
        assert!(
            pkg.tiles
                .values()
                .flat_map(|t| &t.roads)
                .all(|r| r.dress.edge_m > 0.0)
        );
        let ways = way_dresses(&osm);
        let (edges, _) = edge_dresses(&g, &ways, &[]);
        let crossings = edge_crossings(&osm, &g, &proj);
        let s = apply_road_dress(&mut pkg, &g, &edges, &crossings);
        assert_eq!((s.crossings, s.crossings_missed), (3, 0));
        let strips: Vec<&ocw_world::RoadStrip> =
            pkg.tiles.values().flat_map(|t| &t.roads).collect();
        // The zebra 20 m along the first edge.
        let z = strips
            .iter()
            .flat_map(|s| &s.dress.crossings)
            .find(|c| c.kind == CrossingKind::Zebra)
            .unwrap();
        assert!((z.along_m - 20.0).abs() < 0.1, "{}", z.along_m);
        // The stop line 3 m before the junction on the side road (50 m).
        let st = strips
            .iter()
            .flat_map(|s| &s.dress.crossings)
            .find(|c| c.kind == CrossingKind::Stop)
            .unwrap();
        assert!((st.along_m - 57.0).abs() < 0.2, "{}", st.along_m);
        // The geometry stays: edge lengths and the junction at vertex 1.
        let side = strips
            .iter()
            .find(|s| s.class == RoadClass::Residential)
            .unwrap();
        assert!((side.dress.edge_m - 60.0).abs() < 0.01);
        assert_eq!(side.dress.junction_m[0], 0.0);
        let j = RoadClass::Primary.default_width_m() as f32 / 2.0 + ocw_world::road_dress::CORNER_M;
        assert!((side.dress.junction_m[1] - j).abs() < 0.051);
        // The package round trip keeps it all.
        let back = WorldPackage::from_bytes(&pkg.to_bytes()).unwrap();
        assert_eq!(back.tiles, pkg.tiles);
    }
}
