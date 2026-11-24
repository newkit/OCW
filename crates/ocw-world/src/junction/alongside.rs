//! Paths along a road (spec §4.8.8 *Rule: the sides of a road are as
//! smooth as the road*, J7): OSM often draws a road's sidewalk or cycle
//! track as a way of its own a few metres beside the road
//! (`footway=sidewalk`, `cycleway=track` as separate ways). Drawn as roads
//! of their own they met every crossing street with mouths of their own:
//! at a crossing of two such roads the junction surface's outline turned
//! back by 130 – 140° between a path's mouth and its road's (J6's cause
//! (1)). Such a path is now drawn as a band of its road's side – the
//! road's sidewalk widened to the path's far edge, round the junction's
//! corner with the road's own side – and takes no part in the junctions'
//! surfaces. It stays in the road graph: routable and rideable as before
//! (its own profile, its ends plain).
//!
//! The paths are found from the graph's geometry alone (the package has no
//! OSM tags per edge), the same in the world builder, the renderer, the
//! server and the prediction: a path or cycleway whose every point – its
//! first and last [`ALONG_END_M`] left out, where it turns to a crossing –
//! lies within [`ALONG_GAP_M`] of a road's edge, alongside it within
//! [`ALONG_DEG`].

use std::collections::HashMap;

use super::{P2, add, cross, cumulative, dot, frame_at, mul, norm, side_pos, sub, unit};
use crate::graph::{RoadClass, RoadGraph};

/// The path's near edge lies at most this far beyond the road's edge (m)
/// …
pub const ALONG_GAP_M: f64 = 3.0;
/// … or over it by at most this much (m).
pub const ALONG_OVER_M: f64 = 0.5;
/// … and runs alongside it within this angle (degrees).
pub const ALONG_DEG: f64 = 20.0;
/// … and its height differs from the road's by at most this (m): a path
/// on an embankment above the road or a terrace below is its own.
pub const ALONG_LEVEL_M: f64 = 0.5;
/// The first and last metres of a path (turning to its crossing) are not
/// checked.
pub const ALONG_END_M: f64 = 5.0;
/// A path shorter than this (m) is a link, not a side path.
pub const ALONG_MIN_M: f64 = 4.0;
/// Points along a path are checked this far apart (m).
const STEP_M: f64 = 2.0;
/// Grid cells of the segment index (m).
const CELL_M: f64 = 16.0;

/// Whether a road of this class can be drawn as a band of another's side.
pub fn is_path(class: RoadClass) -> bool {
    matches!(class, RoadClass::Cycleway | RoadClass::Path)
}

/// Segments of some edges by grid cell: (edge, index of its first point).
struct Segments {
    cells: HashMap<(i64, i64), Vec<(u32, u32)>>,
}

impl Segments {
    fn new(g: &RoadGraph, keep: impl Fn(usize) -> bool) -> Self {
        let mut cells: HashMap<(i64, i64), Vec<(u32, u32)>> = HashMap::new();
        for (ei, e) in g.edges.iter().enumerate() {
            if !keep(ei) {
                continue;
            }
            for (k, w) in e.points.windows(2).enumerate() {
                let (x0, x1) = (w[0][0].min(w[1][0]), w[0][0].max(w[1][0]));
                let (z0, z1) = (w[0][1].min(w[1][1]), w[0][1].max(w[1][1]));
                let c = |v: f64| (v / CELL_M).floor() as i64;
                for cx in c(x0)..=c(x1) {
                    for cz in c(z0)..=c(z1) {
                        cells
                            .entry((cx, cz))
                            .or_default()
                            .push((ei as u32, k as u32));
                    }
                }
            }
        }
        for v in cells.values_mut() {
            v.sort_unstable();
            v.dedup();
        }
        Self { cells }
    }

    /// The segments in the cells within `r` of `p` (each once).
    fn near(&self, p: P2, r: f64) -> Vec<(u32, u32)> {
        let c = |v: f64| (v / CELL_M).floor() as i64;
        let mut out = Vec::new();
        for cx in c(p[0] - r)..=c(p[0] + r) {
            for cz in c(p[1] - r)..=c(p[1] + r) {
                if let Some(v) = self.cells.get(&(cx, cz)) {
                    out.extend_from_slice(v);
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// The closest point of segment `a`–`b` to `p` and its parameter.
fn closest(p: P2, a: P2, b: P2) -> (P2, f64) {
    let ab = sub(b, a);
    let l2 = dot(ab, ab);
    if l2 < 1e-12 {
        return (a, 0.0);
    }
    let t = (dot(sub(p, a), ab) / l2).clamp(0.0, 1.0);
    (add(a, mul(ab, t)), t)
}

/// Per edge of `g`: a path drawn as a band of a road's side.
pub fn side_paths(g: &RoadGraph) -> Vec<bool> {
    let road = |ei: usize| {
        let e = &g.edges[ei];
        !is_path(e.class) && e.from != e.to && e.points.len() >= 2
    };
    let roads = Segments::new(g, road);
    let (sin_max, cos_max) = ALONG_DEG.to_radians().sin_cos();
    g.edges
        .iter()
        .map(|e| {
            if !is_path(e.class) || e.bridge || e.tunnel || e.from == e.to || e.points.len() < 2 {
                return false;
            }
            let cum = cumulative(&e.points);
            let len = cum[cum.len() - 1];
            if len < ALONG_MIN_M {
                return false;
            }
            let half = e.width_m / 2.0;
            // The points checked: every STEP_M between the ends' metres,
            // the middle of a short path.
            let mut ss: Vec<f64> = Vec::new();
            if len <= 2.0 * ALONG_END_M + STEP_M {
                ss.push(len / 2.0);
            } else {
                let mut s = ALONG_END_M;
                while s <= len - ALONG_END_M + 1e-9 {
                    ss.push(s);
                    s += STEP_M;
                }
            }
            ss.iter().all(|&s| {
                let (p, d) = frame_at(&e.points, &cum, s);
                let h = e.height_at(s);
                roads
                    .near(p, 12.0 + half + ALONG_GAP_M)
                    .into_iter()
                    .any(|(ri, k)| {
                        let r = &g.edges[ri as usize];
                        let (a, b) = (r.points[k as usize], r.points[k as usize + 1]);
                        let u = unit(sub(b, a));
                        if cross(u, d).abs() > sin_max || dot(u, d).abs() < cos_max {
                            return false;
                        }
                        let (q, t) = closest(p, a, b);
                        let gap = norm(sub(p, q)) - r.width_m / 2.0 - half;
                        // At the road's level (not on an embankment above
                        // it or a terrace below).
                        let start: f64 = r.points[..=k as usize]
                            .windows(2)
                            .map(|w| norm(sub(w[1], w[0])))
                            .sum();
                        let rh = r.height_at(start + t * norm(sub(b, a)));
                        let level = match (h, rh) {
                            (Some(h), Some(rh)) => (h - rh).abs() <= ALONG_LEVEL_M,
                            _ => true,
                        };
                        level && (-ALONG_OVER_M..=ALONG_GAP_M).contains(&gap)
                    })
            })
        })
        .collect()
}

/// The height of the band drawn for a side path at point `p` of its centre
/// line: the nearest road's (not a path) height there, its kerb of
/// [`BAND_KERB_M`] and the sidewalk's fall of [`BAND_FALL`] per metre out
/// from its edge; `None` without a road within reach. `roads`: from
/// [`road_index`].
pub fn band_height(g: &RoadGraph, roads: &RoadIndex, p: P2) -> Option<f64> {
    let mut best: Option<(f64, f64, f64)> = None;
    for (ri, k) in roads.0.near(p, 12.0 + ALONG_GAP_M) {
        let r = &g.edges[ri as usize];
        let (a, b) = (r.points[k as usize], r.points[k as usize + 1]);
        let (q, t) = closest(p, a, b);
        let d = norm(sub(p, q));
        if best.is_none_or(|x| d < x.0) {
            let start: f64 = r.points[..=k as usize]
                .windows(2)
                .map(|w| norm(sub(w[1], w[0])))
                .sum();
            if let Some(h) = r.height_at(start + t * norm(sub(b, a))) {
                best = Some((d, h, r.width_m / 2.0));
            }
        }
    }
    let (d, h, half) = best?;
    Some(h + BAND_KERB_M + (d - half).max(0.0) * BAND_FALL)
}

/// The kerb (m) and the fall per metre of a band drawn for a side path
/// where its road has no sidewalk of its own (the renderer's
/// `BAND_KERB_M`, the sidewalk's `SIDEWALK_FALL`).
pub const BAND_KERB_M: f64 = 0.08;
pub const BAND_FALL: f64 = 0.02;

/// The roads (not paths) of a graph by place.
pub struct RoadIndex(Segments);

pub fn road_index(g: &RoadGraph) -> RoadIndex {
    RoadIndex(Segments::new(g, |ei| {
        let e = &g.edges[ei];
        !is_path(e.class) && e.from != e.to && e.points.len() >= 2
    }))
}

/// The side paths of a graph by place, for drawing their bands.
pub struct SideBands {
    paths: Segments,
}

impl SideBands {
    pub fn new(g: &RoadGraph, side: &[bool]) -> Self {
        Self {
            paths: Segments::new(g, |ei| side.get(ei).copied().unwrap_or(false)),
        }
    }

    /// Whether there are no side paths at all.
    pub fn is_empty(&self) -> bool {
        self.paths.cells.is_empty()
    }

    /// How far beyond a road's edges the side paths along it reach at
    /// point `p` of its centre line going along `d` (half width `half`):
    /// [the side `-side_pos(d)`, the side `side_pos(d)`] (m, 0: none) – to
    /// the path's far edge, where the path runs alongside there (not past
    /// its ends).
    pub fn reach(&self, g: &RoadGraph, p: P2, d: P2, half: f64) -> [f64; 2] {
        let mut out = [0.0f64; 2];
        let (sin_max, cos_max) = ALONG_DEG.to_radians().sin_cos();
        let d = unit(d);
        for (pi, k) in self.paths.near(p, half + ALONG_GAP_M + 6.0) {
            let e = &g.edges[pi as usize];
            let (k, n) = (k as usize, e.points.len());
            let (a, b) = (e.points[k], e.points[k + 1]);
            let u = unit(sub(b, a));
            if cross(u, d).abs() > sin_max || dot(u, d).abs() < cos_max {
                continue;
            }
            let (q, t) = closest(p, a, b);
            // Past the path's ends: none.
            if (k == 0 && t <= 0.0 && dot(sub(p, a), u) < 0.0)
                || (k + 2 == n && t >= 1.0 && dot(sub(p, b), u) > 0.0)
            {
                continue;
            }
            let v = sub(q, p);
            let dist = norm(v);
            let ph = e.width_m / 2.0;
            let gap = dist - half - ph;
            if !(-ALONG_OVER_M..=ALONG_GAP_M + 0.5).contains(&gap) {
                continue;
            }
            let side = usize::from(dot(v, side_pos(d)) > 0.0);
            out[side] = out[side].max(dist + ph - half);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::junction::fixtures::{Road, graph};

    fn flat(_: f64) -> f64 {
        0.0
    }

    /// A street along x with a sidewalk way 4.5 m off its centre (z +),
    /// crossing a side street at x = 0 and x = 60, and a footway leaving
    /// it into a park.
    fn street_with_sidewalk() -> RoadGraph {
        let v = [
            [-60.0, 0.0],
            [0.0, 0.0],
            [60.0, 0.0],
            [0.0, 4.5],
            [60.0, 4.5],
            [0.0, 40.0],
            [60.0, 40.0],
            [30.0, 4.5],
            [30.0, 60.0],
        ];
        graph(
            &v,
            vec![
                Road::straight(&v, 0, 1, RoadClass::Residential, 6.0, flat),
                Road::straight(&v, 1, 2, RoadClass::Residential, 6.0, flat),
                // The side streets.
                Road::straight(&v, 1, 3, RoadClass::Residential, 5.0, flat),
                Road::straight(&v, 3, 5, RoadClass::Residential, 5.0, flat),
                Road::straight(&v, 2, 4, RoadClass::Residential, 5.0, flat),
                Road::straight(&v, 4, 6, RoadClass::Residential, 5.0, flat),
                // The sidewalk, in two pieces, and a footway into a park.
                Road::straight(&v, 3, 7, RoadClass::Path, 2.0, flat),
                Road::straight(&v, 7, 4, RoadClass::Path, 2.0, flat),
                Road::straight(&v, 7, 8, RoadClass::Path, 2.0, flat),
            ],
        )
    }

    #[test]
    fn a_sidewalk_way_beside_a_street_is_its_side() {
        let g = street_with_sidewalk();
        let side = side_paths(&g);
        assert_eq!(
            side,
            [false, false, false, false, false, false, true, true, false]
        );
        let bands = SideBands::new(&g, &side);
        // Along the street: to the sidewalk's far edge, 4.5 + 1 − 3 m.
        let r = bands.reach(&g, [20.0, 0.0], [1.0, 0.0], 3.0);
        assert!(r[0] == 0.0 && (r[1] - 2.5).abs() < 1e-9, "{r:?}");
        // The other way round: the other side.
        let r = bands.reach(&g, [20.0, 0.0], [-1.0, 0.0], 3.0);
        assert!((r[0] - 2.5).abs() < 1e-9 && r[1] == 0.0, "{r:?}");
        // Beyond the sidewalk's ends: none.
        let r = bands.reach(&g, [-20.0, 0.0], [1.0, 0.0], 3.0);
        assert_eq!(r, [0.0, 0.0]);
        // The side street is not along it.
        let r = bands.reach(&g, [0.0, 20.0], [0.0, 1.0], 2.5);
        assert_eq!(r, [0.0, 0.0]);
    }

    /// Spec §4.8.8 Rule (J7): sidewalks drawn as ways of their own along
    /// both streets of a crossing take no part in its junction surface –
    /// one patch of the two streets, whose arms go on from where the
    /// sidewalks cross them (J6: the sidewalks' mouths in the outline, and
    /// the corner (5, 5) of the two sidewalks a junction of its own).
    #[test]
    fn sidewalk_ways_take_no_part_in_the_junction() {
        let g = crate::junction::fixtures::crossing_with_sidewalks();
        let side = side_paths(&g);
        assert_eq!(
            side,
            [
                false, false, false, false, false, false, true, true, true, true, true, true
            ]
        );
        let js = crate::junction::JunctionSet::new(&g);
        let p = js
            .patch(&g, 0)
            .unwrap_or_else(|| panic!("{:?}", js.fallback_reason(&g, 0)));
        let mut arms: Vec<usize> = p.arms.iter().map(|a| a.edge).collect();
        arms.sort_unstable();
        assert_eq!(arms, [0, 2, 3, 5]);
        assert_eq!(p.members, [0, 2, 5]);
        // The sidewalks' corner has no surface of its own.
        assert!(js.patch(&g, 7).is_none());
    }

    #[test]
    fn a_path_leaving_the_road_or_far_from_it_is_its_own() {
        let v = [
            [0.0, 0.0],
            [80.0, 0.0],
            [0.0, 4.0],
            [80.0, 44.0],
            [0.0, 12.0],
            [80.0, 12.0],
        ];
        let g = graph(
            &v,
            vec![
                Road::straight(&v, 0, 1, RoadClass::Secondary, 7.0, flat),
                // Leaves the road at 27°.
                Road::straight(&v, 2, 3, RoadClass::Cycleway, 2.0, flat),
                // Parallel but 12 m off (8 m beyond the edge).
                Road::straight(&v, 4, 5, RoadClass::Path, 2.0, flat),
            ],
        );
        assert_eq!(side_paths(&g), [false, false, false]);
    }
}
