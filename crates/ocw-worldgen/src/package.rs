//! World package building (spec §4.3 step 4, §4.5, §4.7): terrain
//! conformed to every road of the graph, cut into 1 km tiles with their
//! road strips.

use std::collections::{BTreeMap, HashMap};

use ocw_world::graph::{PROFILE_STEP_M, RoadGraph};
use ocw_world::package::{RoadStrip, Tile, TileKey, WorldPackage};

use crate::profile::sample_along;

/// Flat strip beside the road edge, in metres.
pub const SHOULDER_M: f64 = 4.0;
/// Blend from road level into the natural terrain, in metres.
pub const BLEND_M: f64 = 60.0;
/// Terrain lies this far below the road surface in the corridor. The tile
/// grid (10 m) interpolates across roads, so it is larger than for the
/// 5 m test-route grid; the client's road edge faces cover the step.
pub const ROAD_CLEARANCE_M: f64 = 0.3;

fn smoothstep(t: f64) -> f64 {
    t * t * (3.0 - 2.0 * t)
}

/// Terrain height field conformed to all roads (bridges and tunnels
/// excluded: the terrain stays natural under a bridge and above a tunnel).
pub struct GraphTerrain<'a, F: Fn(f64, f64) -> f64> {
    graph: &'a RoadGraph,
    base: F,
    /// Beside a road (up to half width + this), the terrain is never higher
    /// than the road surface: a cutting on the uphill side. It must exceed
    /// the diagonal of a terrain grid cell, or a mesh triangle spanning the
    /// road edge pokes through the road.
    cut_m: f64,
    /// Cumulative distance at each point of each edge.
    along: Vec<Vec<f64>>,
    buckets: HashMap<(i64, i64), Vec<(u32, u32)>>,
    bucket_m: f64,
    /// Stretches (from, to metres) of untagged edges that are bridges
    /// (spec §4.8.6): treated like bridge edges.
    spans: HashMap<u32, Vec<(f64, f64)>>,
    /// World cleaning (spec §4.8.3): each road acts through its nearest
    /// point only (see [`GraphTerrain::nearest_point_only`]).
    nearest_only: bool,
}

impl<'a, F: Fn(f64, f64) -> f64> GraphTerrain<'a, F> {
    pub fn new(graph: &'a RoadGraph, base: F, cut_m: f64) -> Self {
        let max_width = graph.edges.iter().map(|e| e.width_m).fold(0.0, f64::max);
        let bucket_m = max_width / 2.0 + (SHOULDER_M + BLEND_M).max(cut_m);
        let key = |p: [f64; 2]| {
            (
                (p[0] / bucket_m).floor() as i64,
                (p[1] / bucket_m).floor() as i64,
            )
        };
        let mut buckets: HashMap<(i64, i64), Vec<(u32, u32)>> = HashMap::new();
        let mut along = Vec::with_capacity(graph.edges.len());
        for (i, e) in graph.edges.iter().enumerate() {
            let mut acc = vec![0.0];
            for (s, w) in e.points.windows(2).enumerate() {
                acc.push(
                    acc[s] + ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt(),
                );
                if e.tunnel {
                    continue;
                }
                // Register the segment in every bucket its box touches.
                let (a, b) = (key(w[0]), key(w[1]));
                for bx in a.0.min(b.0)..=a.0.max(b.0) {
                    for bz in a.1.min(b.1)..=a.1.max(b.1) {
                        buckets
                            .entry((bx, bz))
                            .or_default()
                            .push((i as u32, s as u32));
                    }
                }
            }
            along.push(acc);
        }
        Self {
            graph,
            base,
            cut_m,
            along,
            buckets,
            bucket_m,
            spans: HashMap::new(),
            nearest_only: false,
        }
    }

    /// Each road shapes the terrain through its nearest point only (spec
    /// §4.8.3). Without it every segment within the cut zone counts, and the
    /// nearest point of the segment before (or after) the one under a point
    /// is the node between them – up to the cut zone back along the same
    /// road, lower on a climb: the terrain under a 10 % road was cut down to
    /// the road's height 15 m back, 1.5 m below it, and the blend could take
    /// that height too. With it, use a small cut margin and settle the
    /// terrain under the strips afterwards
    /// ([`crate::clean::settle_terrain_under_roads`]).
    pub fn nearest_point_only(mut self) -> Self {
        self.nearest_only = true;
        self
    }

    /// Adds bridge stretches of untagged edges (edge index, from, to m):
    /// the terrain is cut under them but not pulled up, as under bridges.
    pub fn with_bridge_spans(mut self, spans: impl IntoIterator<Item = (usize, f64, f64)>) -> Self {
        for (e, a, b) in spans {
            self.spans.entry(e as u32).or_default().push((a, b));
        }
        self
    }

    fn on_span(&self, edge: u32, s: f64) -> bool {
        self.spans
            .get(&edge)
            .is_some_and(|v| v.iter().any(|&(a, b)| s >= a && s <= b))
    }

    /// Terrain height at local (x, z).
    pub fn height(&self, x: f64, z: f64) -> f64 {
        let base = (self.base)(x, z);
        let (kx, kz) = (
            (x / self.bucket_m).floor() as i64,
            (z / self.bucket_m).floor() as i64,
        );
        // Strongest road influence nearby: (blend weight towards the road, road height).
        let mut best: Option<(f64, f64)> = None;
        // Lowest road surface whose cutting zone covers this point.
        let mut ceiling = f64::INFINITY;
        // Nearest point per edge (cleaned mode): (edge, distance, along).
        let mut nearest: Vec<(u32, f64, f64)> = Vec::new();
        for dx in -1..=1 {
            for dz in -1..=1 {
                let Some(segs) = self.buckets.get(&(kx + dx, kz + dz)) else {
                    continue;
                };
                for &(ei, si) in segs {
                    let e = &self.graph.edges[ei as usize];
                    let (a, b) = (e.points[si as usize], e.points[si as usize + 1]);
                    let ab = [b[0] - a[0], b[1] - a[1]];
                    let len2 = ab[0] * ab[0] + ab[1] * ab[1];
                    let t = if len2 > 0.0 {
                        (((x - a[0]) * ab[0] + (z - a[1]) * ab[1]) / len2).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    let c = [a[0] + ab[0] * t, a[1] + ab[1] * t];
                    let d = ((x - c[0]).powi(2) + (z - c[1]).powi(2)).sqrt();
                    let corridor = e.width_m / 2.0 + SHOULDER_M;
                    let cut = e.width_m / 2.0 + self.cut_m;
                    if d > (corridor + BLEND_M).max(cut) {
                        continue;
                    }
                    let s = self.along[ei as usize][si as usize] + len2.sqrt() * t;
                    if self.nearest_only {
                        match nearest.iter_mut().find(|n| n.0 == ei) {
                            Some(n) if n.1 > d => *n = (ei, d, s),
                            Some(_) => {}
                            None => nearest.push((ei, d, s)),
                        }
                        continue;
                    }
                    let road = e.height_at(s).unwrap_or(base) - ROAD_CLEARANCE_M;
                    if d <= cut {
                        ceiling = ceiling.min(road);
                    }
                    // Bridges cut the terrain under them but do not pull it up.
                    if e.bridge || d > corridor + BLEND_M || self.on_span(ei, s) {
                        continue;
                    }
                    let w = 1.0 - smoothstep(((d - corridor) / BLEND_M).clamp(0.0, 1.0));
                    if best.is_none_or(|(bw, _)| w > bw) {
                        best = Some((w, road));
                    }
                }
            }
        }
        // Cleaned mode: the strongest (then nearest) road wins the blend.
        let mut best_d = f64::INFINITY;
        for &(ei, d, s) in &nearest {
            let e = &self.graph.edges[ei as usize];
            let corridor = e.width_m / 2.0 + SHOULDER_M;
            let road = e.height_at(s).unwrap_or(base) - ROAD_CLEARANCE_M;
            if d <= e.width_m / 2.0 + self.cut_m {
                ceiling = ceiling.min(road);
            }
            if e.bridge || d > corridor + BLEND_M || self.on_span(ei, s) {
                continue;
            }
            let w = 1.0 - smoothstep(((d - corridor) / BLEND_M).clamp(0.0, 1.0));
            if best.is_none_or(|(bw, _)| w > bw + 1e-12 || (w > bw - 1e-12 && d < best_d)) {
                best = Some((w, road));
                best_d = d;
            }
        }
        let h = match best {
            Some((w, road)) => road * w + base * (1.0 - w),
            None => base,
        };
        h.min(ceiling)
    }
}

/// Splits a road polyline into per-tile runs; each run reaches one point
/// into the next tile, so strips of neighbouring tiles overlap (no gaps).
pub fn split_into_tiles(points: &[[f64; 3]], size: f64) -> Vec<(TileKey, Vec<[f64; 3]>)> {
    let mut out: Vec<(TileKey, Vec<[f64; 3]>)> = Vec::new();
    for (i, p) in points.iter().enumerate() {
        let key = TileKey::containing([p[0], p[2]], size);
        match out.last_mut() {
            Some((k, run)) if *k == key => run.push(*p),
            last => {
                if let Some((_, run)) = last {
                    run.push(*p); // overlap into the next tile
                }
                let mut run = Vec::new();
                if i > 0 {
                    run.push(points[i - 1]);
                }
                run.push(*p);
                out.push((key, run));
            }
        }
    }
    out.retain(|(_, run)| run.len() >= 2);
    out
}

/// How far a road strip reaches beyond a cut at the region border (m).
pub const BORDER_OVERLAP_M: f64 = 0.5;

/// Moves the last point `m` further along the last segment's direction.
fn extend_end(points: &mut [[f64; 3]], m: f64) {
    let n = points.len();
    if n < 2 {
        return;
    }
    let (a, b) = (points[n - 2], points[n - 1]);
    let len = (b[0] - a[0]).hypot(b[2] - a[2]);
    if len > 1e-6 {
        points[n - 1] = [
            b[0] + (b[0] - a[0]) / len * m,
            b[1],
            b[2] + (b[2] - a[2]) / len * m,
        ];
    }
}

/// Builds the package: tiles covering all roads plus `margin_tiles`, terrain
/// with `samples` per tile side, road strips (tunnels left out: they run
/// inside the terrain).
pub fn build_package<F: Fn(f64, f64) -> f64 + Sync>(
    name: &str,
    origin: (f64, f64),
    graph: &RoadGraph,
    base: F,
    tile_size: f64,
    samples: u32,
    margin_tiles: i32,
) -> WorldPackage {
    build_package_with_terrain(
        name,
        origin,
        graph,
        graph,
        base,
        tile_size,
        samples,
        margin_tiles,
    )
}

/// [`build_package`] with the terrain conformed to `terrain_graph` instead
/// of `graph` – for a package cut out of a bigger region, the roads around
/// it (spec §2.2.2): the terrain along the border then follows the roads
/// on both sides, as the neighbouring package's does, so the two meet
/// without a step. Road strips and tiles come from `graph`.
#[allow(clippy::too_many_arguments)]
pub fn build_package_with_terrain<F: Fn(f64, f64) -> f64 + Sync>(
    name: &str,
    origin: (f64, f64),
    graph: &RoadGraph,
    terrain_graph: &RoadGraph,
    base: F,
    tile_size: f64,
    samples: u32,
    margin_tiles: i32,
) -> WorldPackage {
    let terrain = GraphTerrain::new(terrain_graph, base, cut_margin_m(tile_size, samples));
    build_package_on(
        name,
        origin,
        graph,
        &terrain,
        tile_size,
        samples,
        margin_tiles,
    )
}

/// Cutting margin beside roads: 1.5 grid cells (more than a cell diagonal).
pub fn cut_margin_m(tile_size: f64, samples: u32) -> f64 {
    1.5 * tile_size / f64::from(samples - 1)
}

/// [`build_package_with_terrain`] on a terrain built by the caller (e.g.
/// with the detected bridge spans, spec §4.8.6).
pub fn build_package_on<F: Fn(f64, f64) -> f64 + Sync>(
    name: &str,
    origin: (f64, f64),
    graph: &RoadGraph,
    terrain: &GraphTerrain<'_, F>,
    tile_size: f64,
    samples: u32,
    margin_tiles: i32,
) -> WorldPackage {
    let (mut min, mut max) = ([i32::MAX; 2], [i32::MIN; 2]);
    for v in &graph.vertices {
        let k = TileKey::containing(*v, tile_size);
        min = [min[0].min(k.x), min[1].min(k.z)];
        max = [max[0].max(k.x), max[1].max(k.z)];
    }
    let mut roads: BTreeMap<TileKey, Vec<RoadStrip>> = BTreeMap::new();
    for e in graph.edges.iter().filter(|e| !e.tunnel) {
        let mut with_h: Vec<[f64; 3]> = sample_along(&e.points, PROFILE_STEP_M, true)
            .into_iter()
            .map(|(p, s)| [p[0], e.height_at(s).unwrap_or(0.0), p[1]])
            .collect();
        // At the region border the strip reaches a little beyond the cut,
        // under the neighbouring package's strip (no hairline gap).
        if graph.boundary_vertices.contains(&e.to) {
            extend_end(&mut with_h, BORDER_OVERLAP_M);
        }
        if graph.boundary_vertices.contains(&e.from) {
            with_h.reverse();
            extend_end(&mut with_h, BORDER_OVERLAP_M);
            with_h.reverse();
        }
        for (key, run) in split_into_tiles(&with_h, tile_size) {
            roads.entry(key).or_default().push(RoadStrip {
                class: e.class,
                width_m: e.width_m as f32,
                points: run
                    .iter()
                    .map(|p| [p[0] as f32, p[1] as f32, p[2] as f32])
                    .collect(),
            });
        }
    }
    let keys: Vec<TileKey> = (min[1] - margin_tiles..=max[1] + margin_tiles)
        .flat_map(|z| {
            (min[0] - margin_tiles..=max[0] + margin_tiles).map(move |x| TileKey { x, z })
        })
        .collect();
    let n = samples as usize;
    let spacing = tile_size / f64::from(samples - 1);
    // Terrain heights in parallel (the expensive part).
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let chunk = keys.len().div_ceil(threads).max(1);
    let heights: Vec<(TileKey, Vec<f32>)> = std::thread::scope(|s| {
        let handles: Vec<_> = keys
            .chunks(chunk)
            .map(|part| {
                let terrain = &terrain;
                s.spawn(move || {
                    part.iter()
                        .map(|&key| {
                            let o = key.origin(tile_size);
                            let mut h = Vec::with_capacity(n * n);
                            for k in 0..n {
                                for i in 0..n {
                                    h.push(terrain.height(
                                        o[0] + i as f64 * spacing,
                                        o[1] + k as f64 * spacing,
                                    ) as f32);
                                }
                            }
                            (key, h)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("tile thread"))
            .collect()
    });
    let tiles = heights
        .into_iter()
        .map(|(key, heights)| {
            let roads = roads.remove(&key).unwrap_or_default();
            (
                key,
                Tile {
                    key,
                    heights,
                    roads,
                },
            )
        })
        .collect();
    WorldPackage {
        name: name.to_string(),
        origin,
        tile_size_m: tile_size as f32,
        samples,
        tiles,
        // The graph rides along for free ride and junction steering (spec §2.2.1).
        graph: Some(graph.clone()),
        // Land use and buildings are added by `landuse::add_landuse` and
        // `buildings::add_buildings`.
        landuse: None,
        buildings: None,
        bridges: None,
        routes: Vec::new(),
        region: None,
        time_zone: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_world::graph::{Edge, EdgeId, RoadClass, VertexId};

    /// One straight 600 m road along x at z = 500, climbing 0 → 30 m, over a
    /// flat base at 100 m; plus a bridge across it.
    fn graph() -> RoadGraph {
        let edge = |id, a: u32, b: u32, pts: Vec<[f64; 2]>, prof: Vec<f64>, bridge| Edge {
            id: EdgeId(id),
            from: VertexId(a),
            to: VertexId(b),
            points: pts,
            class: RoadClass::Secondary,
            name: None,
            width_m: 6.0,
            bridge,
            tunnel: false,
            dead_end_m: None,
            oneway: ocw_world::graph::Oneway::No,
            boundary: false,
            osm_way: id as i64,
            profile: prof,
        };
        let n = 121; // 600 m at 5 m
        RoadGraph {
            vertices: vec![
                [200.0, 500.0],
                [800.0, 500.0],
                [500.0, 200.0],
                [500.0, 800.0],
            ],
            edges: vec![
                edge(
                    1,
                    0,
                    1,
                    vec![[200.0, 500.0], [800.0, 500.0]],
                    (0..n).map(|i| f64::from(i) * 0.25).collect(),
                    false,
                ),
                edge(
                    2,
                    2,
                    3,
                    vec![[500.0, 200.0], [500.0, 800.0]],
                    vec![140.0; n as usize],
                    true,
                ),
            ],
            boundary_vertices: Default::default(),
        }
    }

    #[test]
    fn terrain_follows_roads_and_blends_to_the_base() {
        let g = graph();
        let t = GraphTerrain::new(&g, |_, _| 100.0, 15.0);
        // On the road at 300 m along (x = 500): road height 15 m.
        let on = t.height(500.0, 500.0);
        assert!((on - (15.0 - ROAD_CLEARANCE_M)).abs() < 1e-6, "{on}");
        // Within the corridor (shoulder): still at road level.
        assert!((t.height(500.0, 506.0) - on).abs() < 0.05);
        // Far away: the base.
        assert_eq!(t.height(500.0, 700.0), 100.0);
        // The bridge (x = 500, at 140 m) does not pull the terrain up.
        assert_eq!(t.height(500.0, 300.0), 100.0);
        // Halfway through the blend: between road and base.
        let mid = t.height(300.0, 500.0 + 3.0 + SHOULDER_M + BLEND_M / 2.0);
        assert!(mid > 15.0 && mid < 100.0, "{mid}");
    }

    /// Regression (spec §4.8.3, lint 2026-11-13: a third of Liechtenstein's
    /// roads floated > 0.8 m): a 10 % road with a node every 10 m. The
    /// segment behind the one under a point reaches it at their shared node,
    /// 10 m back and 1 m lower, and its cut zone cut the terrain there.
    #[test]
    fn a_climbing_road_is_not_cut_by_its_own_nodes_behind() {
        let mut g = graph();
        g.edges.truncate(1);
        let e = &mut g.edges[0];
        e.points = (0..=60)
            .map(|i| [200.0 + f64::from(i) * 10.0, 500.0])
            .collect();
        e.profile = (0..121).map(|i| f64::from(i) * 0.5).collect(); // 10 %
        let road_at = |x: f64| (x - 200.0) * 0.1;
        let old = GraphTerrain::new(&g, |_, _| 100.0, 15.0);
        let under = old.height(505.0, 500.0);
        assert!(under < road_at(505.0) - 0.8, "the old cut: {under}");
        let new = GraphTerrain::new(&g, |_, _| 100.0, 0.5).nearest_point_only();
        for x in [305.0, 505.0, 707.0] {
            let h = new.height(x, 500.0);
            assert!(
                (h - (road_at(x) - ROAD_CLEARANCE_M)).abs() < 1e-6,
                "{x}: {h}"
            );
            assert!((new.height(x, 503.0) - h).abs() < 1e-6, "across the road");
        }
    }

    /// A tunnel (spec §4.3): the terrain is neither pulled down to the
    /// road nor cut above it, so it stays the natural hill (the client
    /// finds the road under it and draws a tube there); it gets no road
    /// strip either.
    #[test]
    fn terrain_stays_natural_above_a_tunnel() {
        let mut g = graph();
        g.edges[0].tunnel = true;
        g.edges[1].bridge = false;
        g.edges.truncate(1);
        let t = GraphTerrain::new(&g, |_, _| 100.0, 15.0);
        // Over the road (15 m) and beside it: the natural 100 m.
        assert_eq!(t.height(500.0, 500.0), 100.0);
        assert_eq!(t.height(500.0, 503.0), 100.0);
        let pkg = build_package("t", (47.0, 9.5), &g, |_, _| 100.0, 1000.0, 11, 0);
        assert!(pkg.tiles.values().all(|t| t.roads.is_empty()));
        // The same road outside a tunnel pulls the terrain down to it.
        g.edges[0].tunnel = false;
        let t = GraphTerrain::new(&g, |_, _| 100.0, 15.0);
        assert!(t.height(500.0, 500.0) < 20.0);
    }

    /// A road across a steep slope: no mesh triangle may rise above the
    /// road surface (the artefact: green wedges sticking into the road).
    #[test]
    fn terrain_never_shows_through_a_road_on_a_slope() {
        let edge = Edge {
            id: EdgeId(1),
            from: VertexId(0),
            to: VertexId(1),
            points: vec![[0.0, 500.0], [1000.0, 520.0]],
            class: RoadClass::Secondary,
            name: None,
            width_m: 6.5,
            bridge: false,
            tunnel: false,
            dead_end_m: None,
            oneway: ocw_world::graph::Oneway::No,
            boundary: false,
            osm_way: 1,
            // Flat road at 300 m.
            profile: vec![300.0; 201],
        };
        let g = RoadGraph {
            vertices: vec![[0.0, 500.0], [1000.0, 520.0]],
            edges: vec![edge],
            boundary_vertices: Default::default(),
        };
        // 60 % side slope: the terrain rises steeply towards +z.
        let p = build_package(
            "slope",
            (47.0, 9.0),
            &g,
            |_, z| 300.0 + 0.6 * (z - 500.0),
            1000.0,
            101,
            0,
        );
        let (worst, at, above, total) = p.road_pokes(0.5);
        assert!(total > 10_000);
        assert_eq!(above, 0, "terrain {worst:.2} m above the road at {at:?}");
        // The uphill side is cut down to the road, further out the slope returns.
        let h = |x, z| p.surface_height(x, z).unwrap();
        assert!(h(500.0, 520.0) <= 300.0);
        assert!(h(500.0, 600.0) > 330.0);
    }

    #[test]
    fn road_strips_on_winding_climbs_keep_the_profile_height() {
        // Regression: strip heights were looked up by the chord length of
        // the resampled points, which cuts every bend; on a tight zigzag
        // climbing at 25 % the strip ended up metres below the road profile
        // (and the terrain conformed to it showed through).
        let points: Vec<[f64; 2]> = (0..=200)
            .map(|i| {
                [
                    100.0 + i as f64 * 3.0,
                    500.0 + if i % 2 == 0 { 0.0 } else { 3.0 },
                ]
            })
            .collect();
        let tmp = Edge {
            id: EdgeId(1),
            from: VertexId(0),
            to: VertexId(1),
            points: points.clone(),
            class: RoadClass::Path,
            name: None,
            width_m: 2.0,
            bridge: false,
            tunnel: false,
            dead_end_m: None,
            oneway: ocw_world::graph::Oneway::No,
            boundary: false,
            osm_way: 1,
            profile: vec![],
        };
        let len = tmp.length_m();
        let steps = (len / PROFILE_STEP_M).ceil() as usize;
        let profile = (0..=steps)
            .map(|i| 500.0 + 0.25 * (i as f64 * PROFILE_STEP_M).min(len))
            .collect();
        let edge = Edge { profile, ..tmp };
        let g = RoadGraph {
            vertices: vec![points[0], *points.last().unwrap()],
            edges: vec![edge.clone()],
            boundary_vertices: Default::default(),
        };
        let p = build_package("zigzag", (47.0, 9.0), &g, |_, _| 400.0, 1000.0, 101, 0);
        for s in p.tiles.values().flat_map(|t| &t.roads) {
            for q in &s.points {
                // Every strip point is on the polyline; find its true distance.
                let (mut walked, mut along) = (0.0, None);
                for w in points.windows(2) {
                    let l = ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt();
                    let t = (f64::from(q[0]) - w[0][0]) / (w[1][0] - w[0][0]);
                    if (0.0..=1.0).contains(&t) {
                        along = Some(walked + l * t);
                        break;
                    }
                    walked += l;
                }
                let want = edge.height_at(along.unwrap()).unwrap();
                assert!(
                    (f64::from(q[1]) - want).abs() < 0.05,
                    "{q:?}: want {want:.2}"
                );
            }
        }
        let (worst, at, above, _) = p.road_pokes(0.5);
        assert_eq!(above, 0, "terrain {worst:.2} m above the road at {at:?}");
    }

    #[test]
    fn polylines_are_split_per_tile_with_overlap() {
        let pts: Vec<[f64; 3]> = (0..25).map(|i| [f64::from(i) * 100.0, 0.0, 50.0]).collect();
        let runs = split_into_tiles(&pts, 1000.0);
        let keys: Vec<i32> = runs.iter().map(|(k, _)| k.x).collect();
        assert_eq!(keys, vec![0, 1, 2]);
        // Tile 0: x = 0..900 plus the overlap point 1000.
        assert_eq!(runs[0].1.first().unwrap()[0], 0.0);
        assert_eq!(runs[0].1.last().unwrap()[0], 1000.0);
        // Tile 1 starts with the last point of tile 0 (900).
        assert_eq!(runs[1].1[0][0], 900.0);
    }

    #[test]
    fn package_covers_the_roads_and_matches_the_terrain() {
        let g = graph();
        let p = build_package("test", (47.0, 9.0), &g, |_, _| 100.0, 500.0, 51, 1);
        // Roads span tiles x 0..1 and z 0..1, plus a margin of 1.
        assert_eq!(p.tiles.len(), 16);
        assert_eq!(p.spacing_m(), 10.0);
        let h = p.height(500.0, 500.0).unwrap();
        assert!((h - (15.0 - ROAD_CLEARANCE_M)).abs() < 0.05, "{h}");
        // Road strips exist, the bridge too, with their heights.
        let strips: Vec<&RoadStrip> = p.tiles.values().flat_map(|t| &t.roads).collect();
        assert!(
            strips
                .iter()
                .any(|s| s.points.iter().any(|q| (q[1] - 140.0).abs() < 1e-3))
        );
        assert!(strips.iter().all(|s| s.points.len() >= 2));
        // Neighbouring tiles share their border samples.
        let (a, b) = (
            &p.tiles[&TileKey { x: 0, z: 0 }],
            &p.tiles[&TileKey { x: 1, z: 0 }],
        );
        for k in 0..51 {
            assert_eq!(a.heights[k * 51 + 50], b.heights[k * 51]);
        }
        // Survives the binary round trip.
        assert_eq!(WorldPackage::from_bytes(&p.to_bytes()).unwrap(), p);
    }

    #[test]
    fn road_strips_reach_a_little_beyond_a_border_cut() {
        // The road along x ends at the border at x = 800 (vertex 1); the
        // bridge's ends are ordinary.
        let mut g = graph();
        g.boundary_vertices.insert(VertexId(1));
        let p = build_package("test", (47.0, 9.0), &g, |_, _| 100.0, 500.0, 51, 1);
        let max_x = |p: &WorldPackage| {
            p.tiles
                .values()
                .flat_map(|t| &t.roads)
                .flat_map(|s| &s.points)
                .map(|q| q[0])
                .fold(f32::MIN, f32::max)
        };
        assert!((max_x(&p) - (800.0 + BORDER_OVERLAP_M as f32)).abs() < 1e-3);
        let plain = build_package("test", (47.0, 9.0), &graph(), |_, _| 100.0, 500.0, 51, 1);
        assert!((max_x(&plain) - 800.0).abs() < 1e-3);
    }
}
