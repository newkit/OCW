//! Bridges (spec §4.8.6): which stretches of road are bridges, their planned
//! deck profile, and the structure built for them.
//!
//! 1. [`detect_spans`] finds the bridges OSM does not tag: an untagged road
//!    crossing water (a water area or a river / canal line at least
//!    [`MIN_BRIDGE_WATER_M`] wide, with water on both sides of the road), a
//!    road inside a `man_made=bridge` outline, or a road running more than
//!    [`GAP_M`] above the natural ground for [`GAP_MIN_M`]–[`GAP_MAX_M`]
//!    (a gap or a narrow valley the elevation filter bridged). Each span
//!    reaches [`ABUTMENT_M`] beyond the obstacle on both sides.
//! 2. [`plan_span_profiles`] makes the deck of each detected span a straight
//!    line between its abutments (tagged bridges are planned so by the
//!    elevation filter already, spec §4.3).
//! 3. [`make_bridges`] chooses the structure from the OSM tags
//!    (`bridge=covered|viaduct|…`, `bridge:structure`, `bridge:material`,
//!    else by road class and span) and builds deck, railings, piers,
//!    abutments and arches on the finished terrain (section `BRDG`).

use std::collections::HashMap;

use ocw_world::bridges::{
    Arch, Bridge, BridgeMaterial, BridgeSource, BridgeStructure, Bridges, DECK_TOP_BELOW_M, Pier,
    Railing,
};
use ocw_world::graph::{PROFILE_STEP_M, RoadClass, RoadGraph};
use ocw_world::landuse::LandClass;
use ocw_world::package::{TileKey, WorldPackage};

use crate::areas::inside_rings;
use crate::clean::point_at;
use crate::landuse::{Area, WaterLine};
use crate::osm::Tags;
use crate::profile::sample_along;

/// Rivers and canals at least this wide get a bridge; narrower streams
/// pass under the road in a culvert.
pub const MIN_BRIDGE_WATER_M: f64 = 3.0;
/// A detected bridge reaches this far beyond the water or gap (m).
pub const ABUTMENT_M: f64 = 3.0;
/// A road this far above the natural ground is over a gap (m) …
pub const GAP_M: f64 = 5.0;
/// … somewhere at least this high (m) …
pub const GAP_PEAK_M: f64 = 10.0;
/// … over a stretch this long (m); longer stretches are embankments.
pub const GAP_MIN_M: f64 = 8.0;
pub const GAP_MAX_M: f64 = 150.0;
/// Water stretches longer than this along a road are mapping overlaps
/// (a road along a shore), not crossings.
pub const WATER_MAX_M: f64 = 250.0;

/// A stretch of one edge that is a bridge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    pub edge: usize,
    pub from_m: f64,
    pub to_m: f64,
    pub source: BridgeSource,
}

/// Water for the crossing test: areas and lines in bucket grids.
pub struct WaterTest<'a> {
    areas: Vec<&'a Area>,
    lines: Vec<&'a WaterLine>,
    outlines: &'a [Vec<Vec<[f64; 2]>>],
    area_buckets: HashMap<(i64, i64), Vec<u32>>,
    line_buckets: HashMap<(i64, i64), Vec<(u32, u32)>>,
    outline_buckets: HashMap<(i64, i64), Vec<u32>>,
}

const BUCKET_M: f64 = 50.0;

fn bucket(p: [f64; 2]) -> (i64, i64) {
    (
        (p[0] / BUCKET_M).floor() as i64,
        (p[1] / BUCKET_M).floor() as i64,
    )
}

fn fill_box(b: [f64; 4], mut f: impl FnMut((i64, i64))) {
    let (lo, hi) = (bucket([b[0], b[1]]), bucket([b[2], b[3]]));
    for x in lo.0..=hi.0 {
        for z in lo.1..=hi.1 {
            f((x, z));
        }
    }
}

fn ring_box<'a>(pts: impl Iterator<Item = &'a [f64; 2]>) -> [f64; 4] {
    let mut b = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for p in pts {
        b = [
            b[0].min(p[0]),
            b[1].min(p[1]),
            b[2].max(p[0]),
            b[3].max(p[1]),
        ];
    }
    b
}

impl<'a> WaterTest<'a> {
    /// Water areas, waterway lines and `man_made=bridge` outlines (rings).
    pub fn new(
        areas: &'a [Area],
        lines: &'a [WaterLine],
        outlines: &'a [Vec<Vec<[f64; 2]>>],
    ) -> Self {
        let areas: Vec<&Area> = areas
            .iter()
            .filter(|a| a.class == LandClass::Water)
            .collect();
        let lines: Vec<&WaterLine> = lines
            .iter()
            .filter(|l| l.width_m >= MIN_BRIDGE_WATER_M)
            .collect();
        let mut area_buckets: HashMap<(i64, i64), Vec<u32>> = HashMap::new();
        for (i, a) in areas.iter().enumerate() {
            fill_box(a.bbox, |k| {
                area_buckets.entry(k).or_default().push(i as u32)
            });
        }
        let mut line_buckets: HashMap<(i64, i64), Vec<(u32, u32)>> = HashMap::new();
        for (i, l) in lines.iter().enumerate() {
            let h = l.width_m / 2.0;
            for (s, w) in l.points.windows(2).enumerate() {
                let b = ring_box(w.iter());
                fill_box([b[0] - h, b[1] - h, b[2] + h, b[3] + h], |k| {
                    line_buckets
                        .entry(k)
                        .or_default()
                        .push((i as u32, s as u32));
                });
            }
        }
        let mut outline_buckets: HashMap<(i64, i64), Vec<u32>> = HashMap::new();
        for (i, o) in outlines.iter().enumerate() {
            fill_box(ring_box(o.iter().flatten()), |k| {
                outline_buckets.entry(k).or_default().push(i as u32);
            });
        }
        Self {
            areas,
            lines,
            outlines,
            area_buckets,
            line_buckets,
            outline_buckets,
        }
    }

    pub fn is_water(&self, p: [f64; 2]) -> bool {
        let k = bucket(p);
        if let Some(v) = self.line_buckets.get(&k) {
            for &(i, s) in v {
                let l = self.lines[i as usize];
                let (a, b) = (l.points[s as usize], l.points[s as usize + 1]);
                if crate::spatial::foot_on_segment(p, a, b).0 <= l.width_m / 2.0 {
                    return true;
                }
            }
        }
        self.area_buckets.get(&k).is_some_and(|v| {
            v.iter().any(|&i| {
                let a = self.areas[i as usize];
                p[0] >= a.bbox[0]
                    && p[0] <= a.bbox[2]
                    && p[1] >= a.bbox[1]
                    && p[1] <= a.bbox[3]
                    && inside_rings(p, &a.rings)
            })
        })
    }

    pub fn in_outline(&self, p: [f64; 2]) -> bool {
        self.outline_buckets.get(&bucket(p)).is_some_and(|v| {
            v.iter()
                .any(|&i| inside_rings(p, &self.outlines[i as usize]))
        })
    }
}

/// The `man_made=bridge` outlines of an extract (rings in local metres).
pub fn bridge_outlines(
    data: &crate::areas::AreaData,
    proj: &crate::projection::LocalProjection,
) -> Vec<Vec<Vec<[f64; 2]>>> {
    data.areas()
        .into_iter()
        .filter(|a| a.tags.get("man_made").map(String::as_str) == Some("bridge"))
        .map(|a| {
            a.outer
                .iter()
                .chain(&a.inner)
                .map(|r| r.iter().map(|&(lat, lon)| proj.project(lat, lon)).collect())
                .collect()
        })
        .collect()
}

/// Runs of `true` in samples at `step` m: (from, to) metres.
fn runs(flags: &[bool], step: f64) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    let mut k = 0;
    while k < flags.len() {
        if !flags[k] {
            k += 1;
            continue;
        }
        let a = k;
        while k < flags.len() && flags[k] {
            k += 1;
        }
        out.push((a as f64 * step, (k - 1) as f64 * step));
    }
    out
}

/// Finds the untagged bridges of `graph` (see the module docs). `ground` is
/// the natural DEM; the graph must have its profiles.
pub fn detect_spans(
    graph: &RoadGraph,
    water: &WaterTest,
    ground: &impl Fn([f64; 2]) -> Option<f64>,
) -> Vec<Span> {
    let adj = graph.adjacency();
    let mut out = Vec::new();
    for (ei, e) in graph.edges.iter().enumerate() {
        if e.bridge || e.tunnel || e.profile.is_empty() {
            continue;
        }
        let len = e.length_m();
        let half = e.width_m / 2.0;
        let step = 1.0;
        let n = (len / step).floor() as usize + 1;
        let mut found: Vec<(f64, f64, BridgeSource)> = Vec::new();
        // Water and outlines.
        let (mut wet, mut outl) = (vec![false; n], vec![false; n]);
        for k in 0..n {
            let s = k as f64 * step;
            let p = point_at(&e.points, s);
            wet[k] = water.is_water(p);
            outl[k] = water.in_outline(p);
        }
        for (a, b) in runs(&outl, step) {
            found.push((a, b, BridgeSource::Outline));
        }
        for (a, b) in runs(&wet, step) {
            if b - a > WATER_MAX_M {
                continue;
            }
            // A crossing has water on both sides of the road.
            let m = (a + b) / 2.0;
            let c = point_at(&e.points, m);
            let d = {
                let (p, q) = (
                    point_at(&e.points, (m - 1.0).max(0.0)),
                    point_at(&e.points, (m + 1.0).min(len)),
                );
                let l = (q[0] - p[0]).hypot(q[1] - p[1]).max(1e-9);
                [(q[0] - p[0]) / l, (q[1] - p[1]) / l]
            };
            let off = half + 2.0;
            let sides =
                [-1.0, 1.0].map(|s| water.is_water([c[0] - d[1] * off * s, c[1] + d[0] * off * s]));
            if sides[0] && sides[1] && !found.iter().any(|f| f.0 <= b && a <= f.1) {
                found.push((a, b, BridgeSource::Water));
            }
        }
        // Deep gaps under the road.
        let gstep = 2.0;
        let gn = (len / gstep).floor() as usize + 1;
        let gaps: Vec<f64> = (0..gn)
            .map(|k| {
                let s = k as f64 * gstep;
                let y = e.height_at(s).unwrap_or(0.0);
                ground(point_at(&e.points, s)).map_or(0.0, |g| y - g)
            })
            .collect();
        let high: Vec<bool> = gaps.iter().map(|&g| g > GAP_M).collect();
        for (a, b) in runs(&high, gstep) {
            let peak = gaps[(a / gstep) as usize..=(b / gstep) as usize]
                .iter()
                .copied()
                .fold(0.0, f64::max);
            let l = b - a;
            if peak >= GAP_PEAK_M
                && (GAP_MIN_M..=GAP_MAX_M).contains(&l)
                && !found.iter().any(|f| f.0 <= b && a <= f.1)
            {
                found.push((a, b, BridgeSource::Gap));
            }
        }
        // Abutments; skip ends where a tagged bridge already starts (OSM
        // bridges are often drawn a little short).
        let bridge_at = |v: u32| {
            adj[v as usize]
                .iter()
                .any(|&o| o != ei && graph.edges[o].bridge)
        };
        found.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut spans: Vec<Span> = Vec::new();
        for (a, b, source) in found {
            let (from, to) = ((a - ABUTMENT_M).max(0.0), (b + ABUTMENT_M).min(len));
            if (from <= 0.5 && bridge_at(e.from.0)) || (to >= len - 0.5 && bridge_at(e.to.0)) {
                continue;
            }
            if to - from < 2.0 {
                continue;
            }
            match spans.last_mut() {
                Some(last) if from - last.to_m < 8.0 => last.to_m = last.to_m.max(to),
                _ => spans.push(Span {
                    edge: ei,
                    from_m: from,
                    to_m: to,
                    source,
                }),
            }
        }
        out.extend(spans);
    }
    out
}

/// Makes the deck of each detected span straight between its abutments
/// (the profile samples strictly inside it are replaced by the line between
/// the heights at its ends; the class gradient holds, as the line is the
/// mean gradient of a limited profile). Returns the samples changed.
pub fn plan_span_profiles(graph: &mut RoadGraph, spans: &[Span]) -> usize {
    let mut changed = 0;
    for s in spans {
        let e = &mut graph.edges[s.edge];
        let (Some(ha), Some(hb)) = (e.height_at(s.from_m), e.height_at(s.to_m)) else {
            continue;
        };
        let len = e.length_m();
        for (i, h) in e.profile.iter_mut().enumerate() {
            let d = (i as f64 * PROFILE_STEP_M).min(len);
            if d > s.from_m + 1e-6 && d < s.to_m - 1e-6 {
                *h = ha + (hb - ha) * (d - s.from_m) / (s.to_m - s.from_m).max(1e-9);
                changed += 1;
            }
        }
    }
    changed
}

/// The bridge spans of a graph: every tagged bridge edge whole, plus the
/// detected spans.
pub fn all_spans(graph: &RoadGraph, detected: &[Span]) -> Vec<Span> {
    let mut v: Vec<Span> = graph
        .edges
        .iter()
        .enumerate()
        .filter(|(_, e)| e.bridge && !e.tunnel)
        .map(|(i, e)| Span {
            edge: i,
            from_m: 0.0,
            to_m: e.length_m(),
            source: BridgeSource::Tagged,
        })
        .collect();
    v.extend_from_slice(detected);
    v
}

/// The structure of a bridge from its OSM tags, road class and length.
pub fn choose_structure(
    tags: Option<&Tags>,
    class: RoadClass,
    length_m: f64,
) -> (BridgeStructure, BridgeMaterial, Railing) {
    let t = |k: &str| tags.and_then(|t| t.get(k)).map(String::as_str);
    let mut structure = match t("bridge:structure") {
        Some("arch" | "humpback") => Some(BridgeStructure::Arch),
        Some("truss") => Some(BridgeStructure::Truss),
        Some("suspension" | "simple-suspension" | "cable-stayed") => {
            Some(BridgeStructure::Suspension)
        }
        Some(_) => Some(BridgeStructure::Beam),
        None => None,
    };
    // `covered=yes` only on bridge ways: on others it means a passage
    // under a building (a road inside a bridge outline, e.g.).
    let on_bridge_way = t("bridge").is_some_and(|b| b != "no");
    if t("bridge") == Some("covered") || (on_bridge_way && t("covered") == Some("yes")) {
        structure = Some(BridgeStructure::Covered);
    }
    let material = match t("bridge:material").or(t("material")) {
        Some("stone" | "masonry" | "brick" | "granite" | "sandstone") => {
            Some(BridgeMaterial::Stone)
        }
        Some("wood" | "timber") => Some(BridgeMaterial::Wood),
        Some("steel" | "metal" | "iron" | "aluminium") => Some(BridgeMaterial::Steel),
        Some(m) if m.contains("concrete") => Some(BridgeMaterial::Concrete),
        _ => None,
    };
    let path = matches!(class, RoadClass::Path | RoadClass::Cycleway);
    // Short stone bridges are arches unless tagged otherwise.
    if structure.is_none() && material == Some(BridgeMaterial::Stone) && length_m <= 40.0 {
        structure = Some(BridgeStructure::Arch);
    }
    let structure = structure.unwrap_or(BridgeStructure::Beam);
    let material = material.unwrap_or(match structure {
        BridgeStructure::Covered => BridgeMaterial::Wood,
        BridgeStructure::Truss | BridgeStructure::Suspension => BridgeMaterial::Steel,
        BridgeStructure::Arch => BridgeMaterial::Stone,
        BridgeStructure::Beam if class == RoadClass::Path && length_m <= 25.0 => {
            BridgeMaterial::Wood
        }
        BridgeStructure::Beam if path => BridgeMaterial::Steel,
        BridgeStructure::Beam => BridgeMaterial::Concrete,
    });
    let railing = match material {
        BridgeMaterial::Stone => Railing::Parapet,
        BridgeMaterial::Concrete if !path => Railing::Parapet,
        _ => Railing::Rail,
    };
    (structure, material, railing)
}

/// Longest span between two supports (m) for a structure and material.
pub fn max_span_m(structure: BridgeStructure, material: BridgeMaterial) -> f64 {
    match (structure, material) {
        (BridgeStructure::Suspension, _) => f64::INFINITY,
        (BridgeStructure::Truss, _) => 60.0,
        (BridgeStructure::Arch, BridgeMaterial::Stone) => 18.0,
        (BridgeStructure::Arch, _) => 50.0,
        (BridgeStructure::Covered, _) => 30.0,
        (_, BridgeMaterial::Wood) => 12.0,
        (_, BridgeMaterial::Stone) => 15.0,
        (_, BridgeMaterial::Steel) => 45.0,
        (_, BridgeMaterial::Concrete) => 35.0,
    }
}

/// Deck thickness (m) for a structure and the span between supports.
pub fn deck_thickness_m(structure: BridgeStructure, material: BridgeMaterial, span: f64) -> f64 {
    match structure {
        BridgeStructure::Arch => 0.45,
        BridgeStructure::Truss => 0.6,
        BridgeStructure::Suspension => 0.8,
        BridgeStructure::Covered => 0.5,
        BridgeStructure::Beam if material == BridgeMaterial::Wood => (span / 25.0).clamp(0.3, 0.8),
        BridgeStructure::Beam => (span / 18.0).clamp(0.45, 2.5),
    }
}

/// What [`make_bridges`] built.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BridgeStats {
    pub by_source: [usize; 4],
    pub by_structure: [usize; 5],
    pub piers: usize,
    pub arches: usize,
    pub length_m: f64,
}

/// Builds the bridge structures of `spans` on the finished terrain of
/// `pkg` (its graph is the one the spans index). `tags` gives a way's OSM
/// tags. Tagged bridge edges that meet only other bridge edges at a vertex
/// (a bridge split into several ways) share their piers' rhythm and get an
/// abutment only at the real ends.
pub fn make_bridges(
    pkg: &mut WorldPackage,
    spans: &[Span],
    tags: &impl Fn(i64) -> Option<Tags>,
) -> BridgeStats {
    let mut stats = BridgeStats::default();
    let Some(graph) = pkg.graph.as_ref() else {
        return stats;
    };
    let adj = graph.adjacency();
    let size = f64::from(pkg.tile_size_m);
    let ground = |p: [f64; 2]| pkg.surface_height(p[0], p[1]);
    let mut out = Bridges::new();
    for s in spans {
        let e = &graph.edges[s.edge];
        let len = s.to_m - s.from_m;
        if len < 1.0 || e.profile.is_empty() {
            continue;
        }
        // Points of the deck: the same as the road strip's (5 m and every
        // vertex), within the span.
        let mut pts: Vec<[f32; 3]> = Vec::new();
        let mut push = |d: f64| {
            let p = point_at(&e.points, d);
            let y = e.height_at(d).unwrap_or(0.0);
            pts.push([p[0] as f32, y as f32, p[1] as f32]);
        };
        push(s.from_m);
        for (_, d) in sample_along(&e.points, PROFILE_STEP_M, true) {
            if d > s.from_m + 0.05 && d < s.to_m - 0.05 {
                push(d);
            }
        }
        push(s.to_m);
        let way_tags = tags(e.osm_way);
        let (structure, material, railing) = choose_structure(way_tags.as_ref(), e.class, len);
        // An end continues on another bridge edge: no abutment there.
        let continues = |v: u32, at_end: bool| {
            s.source == BridgeSource::Tagged
                && ((at_end && s.to_m >= e.length_m() - 0.01) || (!at_end && s.from_m <= 0.01))
                && adj[v as usize]
                    .iter()
                    .any(|&o| o != s.edge && graph.edges[o].bridge)
        };
        let deck_top = |d: f64| e.height_at(d).unwrap_or(0.0);
        let max_span = max_span_m(structure, material);
        let bays = (len / max_span).ceil().max(1.0) as usize;
        let bay = len / bays as f64;
        let deck_m = deck_thickness_m(structure, material, bay);
        let bottom = |d: f64| deck_top(d) - f64::from(DECK_TOP_BELOW_M) - deck_m;
        let mut piers = Vec::new();
        for k in 1..bays {
            let d = bay * k as f64;
            let p = point_at(&e.points, s.from_m + d);
            if let Some(g) = ground(p)
                && bottom(s.from_m + d) - g > 1.0
            {
                piers.push(Pier {
                    along_m: d as f32,
                    foot_y: g as f32,
                });
            }
        }
        // Abutment feet: the ground just outside each end (none where the
        // bridge continues on the next edge: the foot is the deck then).
        let end_foot = |d: f64, outward: f64, cont: bool| -> f32 {
            if cont {
                return (deck_top(d) + 1.0) as f32;
            }
            let p = point_at(&e.points, (d + outward).clamp(0.0, e.length_m()));
            ground(p).map_or(deck_top(d) - 2.0, |g| g.min(bottom(d))) as f32
        };
        let abutment_foot_y = [
            end_foot(s.from_m, -1.0, continues(e.from.0, false)),
            end_foot(s.to_m, 1.0, continues(e.to.0, true)),
        ];
        // Arches between the supports.
        let mut arches = Vec::new();
        let mut structure = structure;
        if structure == BridgeStructure::Arch {
            let mut supports: Vec<f64> = vec![0.0];
            supports.extend(piers.iter().map(|p| f64::from(p.along_m)));
            supports.push(len);
            for w in supports.windows(2) {
                let (a, b) = (w[0], w[1]);
                let span = b - a;
                let foot = |d: f64| ground(point_at(&e.points, s.from_m + d));
                let low = match (foot(a), foot(b)) {
                    (Some(x), Some(y)) => x.min(y),
                    _ => continue,
                };
                let crown = bottom(s.from_m + (a + b) / 2.0) - 0.35;
                let spring = low.max(crown - span / 2.0);
                if crown - spring >= 0.8 {
                    arches.push(Arch {
                        from_m: a as f32,
                        to_m: b as f32,
                        spring_y: spring as f32,
                        crown_y: crown as f32,
                    });
                }
            }
            if arches.is_empty() {
                // Too low for an arch: a slab.
                structure = BridgeStructure::Beam;
            }
        }
        stats.by_source[usize::from(s.source.code())] += 1;
        stats.by_structure[usize::from(structure.code())] += 1;
        stats.piers += piers.len();
        stats.arches += arches.len();
        stats.length_m += len;
        let mid = point_at(&e.points, (s.from_m + s.to_m) / 2.0);
        out.entry(TileKey::containing(mid, size))
            .or_default()
            .push(Bridge {
                edge: e.id,
                osm_way: e.osm_way,
                structure,
                material,
                source: s.source,
                railing,
                width_m: e.width_m as f32,
                deck_m: deck_m as f32,
                points: pts,
                abutment_foot_y,
                piers,
                arches,
            });
    }
    pkg.bridges = Some(out);
    stats
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_world::graph::{Edge, EdgeId, Oneway, VertexId};

    fn road(points: Vec<[f64; 2]>, profile: Vec<f64>) -> Edge {
        Edge {
            id: EdgeId(1),
            from: VertexId(0),
            to: VertexId(1),
            points,
            class: RoadClass::Tertiary,
            name: None,
            width_m: 6.0,
            bridge: false,
            tunnel: false,
            dead_end_m: None,
            boundary: false,
            oneway: Oneway::No,
            osm_way: 99,
            profile,
        }
    }

    fn graph(e: Edge) -> RoadGraph {
        RoadGraph {
            vertices: vec![e.points[0], *e.points.last().unwrap()],
            edges: vec![e],
            boundary_vertices: Default::default(),
        }
    }

    /// A 200 m road along x, over a 20 m wide river crossing at x = 100.
    #[test]
    fn untagged_river_crossings_become_bridges() {
        let g = graph(road(vec![[0.0, 0.0], [200.0, 0.0]], vec![400.0; 41]));
        let lines = vec![WaterLine {
            points: vec![[100.0, -500.0], [100.0, 500.0]],
            width_m: 20.0,
        }];
        let w = WaterTest::new(&[], &lines, &[]);
        let spans = detect_spans(&g, &w, &|_| Some(399.0));
        assert_eq!(spans.len(), 1, "{spans:?}");
        let s = spans[0];
        assert_eq!(s.source, BridgeSource::Water);
        assert!((s.from_m - (90.0 - ABUTMENT_M)).abs() < 1.01, "{s:?}");
        assert!((s.to_m - (110.0 + ABUTMENT_M)).abs() < 1.01, "{s:?}");
        // A stream (2 m, a culvert) is not a bridge.
        let lines = vec![WaterLine {
            points: vec![[100.0, -500.0], [100.0, 500.0]],
            width_m: 2.0,
        }];
        let w = WaterTest::new(&[], &lines, &[]);
        assert!(detect_spans(&g, &w, &|_| Some(399.0)).is_empty());
        // A road along the shore (water on one side only) is not a bridge.
        let lake = Area::new(
            LandClass::Water,
            vec![vec![[0.0, 1.0], [200.0, 1.0], [200.0, 300.0], [0.0, 300.0]]],
        );
        let areas = [lake];
        let w = WaterTest::new(&areas, &[], &[]);
        let g2 = graph(road(vec![[50.0, 1.5], [150.0, 1.5]], vec![400.0; 21]));
        assert!(detect_spans(&g2, &w, &|_| Some(399.0)).is_empty());
    }

    #[test]
    fn deep_gaps_become_bridges_and_long_embankments_do_not() {
        // The road stays at 400 m; the ground drops to 390 m for 40 m.
        let g = graph(road(vec![[0.0, 0.0], [200.0, 0.0]], vec![400.0; 41]));
        let w = WaterTest::new(&[], &[], &[]);
        let valley = |p: [f64; 2]| {
            Some(if (80.0..120.0).contains(&p[0]) {
                390.0
            } else {
                399.7
            })
        };
        let spans = detect_spans(&g, &w, &valley);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].source, BridgeSource::Gap);
        // 180 m lower: an embankment, no bridge.
        let wide = |p: [f64; 2]| {
            Some(if (10.0..190.0).contains(&p[0]) {
                390.0
            } else {
                399.7
            })
        };
        assert!(detect_spans(&g, &w, &wide).is_empty());
    }

    #[test]
    fn detected_decks_are_straight_between_their_abutments() {
        let profile: Vec<f64> = (0..41)
            .map(|i| 400.0 - if (16..=24).contains(&i) { 3.0 } else { 0.0 })
            .collect();
        let mut g = graph(road(vec![[0.0, 0.0], [200.0, 0.0]], profile));
        let span = Span {
            edge: 0,
            from_m: 75.0,
            to_m: 125.0,
            source: BridgeSource::Water,
        };
        assert_eq!(plan_span_profiles(&mut g, &[span]), 9);
        for i in 15..=25 {
            assert!((g.edges[0].profile[i] - 400.0).abs() < 1e-9, "{i}");
        }
    }

    #[test]
    fn structures_follow_the_tags() {
        let tags = |pairs: &[(&str, &str)]| -> Tags {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        };
        let c = |t: Tags, class, len| choose_structure(Some(&t), class, len);
        // The usual tagging of the Alte Rheinbrücke Vaduz – Sevelen.
        assert_eq!(
            c(
                tags(&[("bridge", "yes"), ("covered", "yes")]),
                RoadClass::Cycleway,
                135.0
            )
            .0,
            BridgeStructure::Covered
        );
        assert_eq!(
            c(tags(&[("covered", "yes")]), RoadClass::Path, 20.0).0,
            BridgeStructure::Beam,
            "a passage under a building, not a covered bridge"
        );
        assert_eq!(
            c(tags(&[("bridge", "covered")]), RoadClass::Path, 60.0),
            (
                BridgeStructure::Covered,
                BridgeMaterial::Wood,
                Railing::Rail
            )
        );
        assert_eq!(
            c(
                tags(&[("bridge:material", "stone")]),
                RoadClass::Track,
                12.0
            ),
            (
                BridgeStructure::Arch,
                BridgeMaterial::Stone,
                Railing::Parapet
            )
        );
        assert_eq!(
            c(
                tags(&[("bridge:material", "stone")]),
                RoadClass::Track,
                80.0
            )
            .0,
            BridgeStructure::Beam
        );
        assert_eq!(
            c(
                tags(&[("bridge:structure", "truss")]),
                RoadClass::Primary,
                100.0
            ),
            (BridgeStructure::Truss, BridgeMaterial::Steel, Railing::Rail)
        );
        assert_eq!(
            choose_structure(None, RoadClass::Primary, 120.0),
            (
                BridgeStructure::Beam,
                BridgeMaterial::Concrete,
                Railing::Parapet
            )
        );
        assert_eq!(
            choose_structure(None, RoadClass::Path, 10.0).1,
            BridgeMaterial::Wood
        );
        assert_eq!(
            deck_thickness_m(BridgeStructure::Beam, BridgeMaterial::Concrete, 35.0),
            35.0 / 18.0
        );
    }

    /// A tagged 60 m bridge 10 m over a valley: piers every ≤ 35 m,
    /// abutments on the ground, the deck on the road profile.
    #[test]
    fn bridges_are_built_on_the_terrain() {
        use ocw_world::package::Tile;
        let mut e = road(vec![[200.0, 500.0], [260.0, 500.0]], vec![110.0; 13]);
        e.bridge = true;
        let g = graph(e);
        let n = 101usize;
        let mut tiles = std::collections::BTreeMap::new();
        let key = TileKey { x: 0, z: 0 };
        tiles.insert(
            key,
            Tile {
                key,
                heights: vec![100.0; n * n],
                roads: vec![],
            },
        );
        let mut pkg = WorldPackage {
            name: "t".into(),
            origin: (47.0, 9.0),
            tile_size_m: 1000.0,
            samples: n as u32,
            tiles,
            graph: Some(g.clone()),
            landuse: None,
            buildings: None,
            bridges: None,
            routes: vec![],
            region: None,
        };
        let stats = make_bridges(&mut pkg, &all_spans(&g, &[]), &|_| None);
        assert_eq!(stats.by_source[0], 1);
        let b = &pkg.bridges.as_ref().unwrap()[&key][0];
        assert_eq!(b.structure, BridgeStructure::Beam);
        assert_eq!(b.piers.len(), 1);
        assert!((b.piers[0].along_m - 30.0).abs() < 1e-4);
        assert_eq!(b.piers[0].foot_y, 100.0);
        assert_eq!(b.abutment_foot_y, [100.0, 100.0]);
        assert!(b.points.iter().all(|p| p[1] == 110.0));
        assert!((b.length_m() - 60.0).abs() < 1e-3);
    }
}
